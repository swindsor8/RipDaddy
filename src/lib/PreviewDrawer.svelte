<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import FrameLightbox from "./FrameLightbox.svelte";
  import {
    shortAudioLabel,
    shortEncoderLabel,
    shortResolutionLabel,
  } from "./qualityDefaults";
  import type {
    DiscTitle,
    NamingConvention,
    PipelineMode,
    PreviewFrame,
    QualityProfile,
    ResolutionTier,
    StreamTrack,
    TrickplayPreview,
  } from "./types";

  interface Props {
    workflowMode: PipelineMode;
    selectedTitle: DiscTitle | null;
    selectedLocalFile: string | null;
    outputFilenamePreview: string;
    namingConvention: NamingConvention;
    outputContainer: string;
    activeQualityProfile: QualityProfile | null;
    posterUrl: string | null;
    mediaTitle: string;
    mediaYear: number | null;
    mediaOverview: string | null;
    trickplay: TrickplayPreview | null;
    loadingTrickplay: boolean;
    trickplayError: string | null;
    onRegenerateFrames: () => void;
    onClose: () => void;
    onEditQuality: () => void;
  }

  let {
    workflowMode,
    selectedTitle,
    selectedLocalFile,
    outputFilenamePreview,
    namingConvention,
    outputContainer,
    activeQualityProfile,
    posterUrl,
    mediaTitle,
    mediaYear,
    mediaOverview,
    trickplay,
    loadingTrickplay,
    trickplayError,
    onRegenerateFrames,
    onClose,
    onEditQuality,
  }: Props = $props();

  let activeFrameIndex = $state(0);
  let lightboxOpen = $state(false);

  const frames = $derived(trickplay?.frames ?? []);
  const activeFrame = $derived<PreviewFrame | null>(
    frames[activeFrameIndex] ?? frames[0] ?? null,
  );

  // Reset active frame index whenever the selected file or frame count changes
  $effect(() => {
    const _file = selectedLocalFile;
    void _file;
    activeFrameIndex = 0;
  });

  $effect(() => {
    if (frames.length > 0 && activeFrameIndex >= frames.length) {
      activeFrameIndex = 0;
    }
  });

  function frameImageSrc(frame: PreviewFrame): string {
    if (frame.path.startsWith("data:") || frame.path.startsWith("http")) {
      return frame.path;
    }
    try {
      return convertFileSrc(frame.path);
    } catch {
      return frame.path;
    }
  }

  function stepFrame(delta: number): void {
    if (frames.length === 0) return;
    activeFrameIndex = (activeFrameIndex + delta + frames.length) % frames.length;
  }

  function handleScrubInput(event: Event): void {
    const val = Number.parseInt(
      (event.currentTarget as HTMLInputElement).value,
      10,
    );
    if (Number.isFinite(val) && val >= 0 && val < frames.length) {
      activeFrameIndex = val;
    }
  }

  function fileBaseName(path: string): string {
    const parts = path.split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function formatDuration(seconds: number | null): string {
    if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) {
      return "--:--";
    }
    const total = Math.round(seconds);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    if (h > 0) {
      return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    }
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function formatFileSize(bytes: number | null): string {
    if (bytes === null || bytes <= 0) return "--";
    const gb = bytes / (1024 * 1024 * 1024);
    if (gb >= 1) return `${gb.toFixed(2)} GB`;
    const mb = bytes / (1024 * 1024);
    return `${mb.toFixed(1)} MB`;
  }

  function formatBitrate(kbps: number | null): string {
    if (kbps === null || kbps <= 0) return "--";
    if (kbps >= 1000) return `${(kbps / 1000).toFixed(1)} Mbps`;
    return `${kbps} kbps`;
  }

  function tierMaxBounds(tier: ResolutionTier): [number, number] {
    switch (tier) {
      case "p480":
        return [720, 480];
      case "p720":
        return [1280, 720];
      case "p1080":
        return [1920, 1080];
      case "uhd4k":
        return [3840, 2160];
    }
  }

  const scalingNote = $derived.by(() => {
    if (!trickplay?.mediaInfo.width || !trickplay.mediaInfo.height || !activeQualityProfile) {
      return null;
    }
    const srcW = trickplay.mediaInfo.displayWidth ?? trickplay.mediaInfo.width;
    const srcH = trickplay.mediaInfo.displayHeight ?? trickplay.mediaInfo.height;
    const [maxW, maxH] = tierMaxBounds(activeQualityProfile.resolution);
    if (srcW > maxW || srcH > maxH) {
      return `Downscales to max ${maxW}×${maxH} (${shortResolutionLabel(activeQualityProfile.resolution)})`;
    }
    return `Preserves source ${srcW}×${srcH} (no upscale)`;
  });

  function trackBadgeClass(track: StreamTrack): string {
    switch (track.trackType) {
      case "video":
        return "border-[#B5DAFA] bg-[#DCEEFF] text-[#155CA2]";
      case "audio":
        return "border-[#B3E8CB] bg-[#E3F9EC] text-[#17462B]";
      case "subtitle":
        return "border-[#F1D89F] bg-[#FDF4DE] text-[#533B0B]";
      case "unknown":
        return "border-[#E5E3DD] bg-[#F8F7F4] text-[#5C5B57]";
    }
  }

  function formatTrackSummary(track: StreamTrack): string {
    const parts: string[] = [];
    if (track.codecShort) parts.push(track.codecShort);
    if (track.videoSize) parts.push(track.videoSize);
    if (track.channelLayout) parts.push(track.channelLayout);
    if (track.languageName) parts.push(track.languageName);
    else if (track.languageCode) parts.push(track.languageCode.toUpperCase());
    return parts.join(" · ") || `Stream #${track.streamId}`;
  }
</script>

<aside
  aria-label="Preview and inspector"
  class="flex w-[360px] shrink-0 flex-col border-l border-[#E5E3DD] bg-white"
>
  <!-- Drawer Header -->
  <header
    class="flex items-center justify-between border-b border-[#ECEAE4] px-4 py-2.5"
  >
    <div class="flex items-center gap-2">
      <h2 class="text-xs font-bold tracking-[-0.01em] text-[#111111]">
        Preview
      </h2>
      <span
        class="rounded-full border border-[#E5E3DD] bg-[#F8F7F4] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#5C5B57]"
      >
        {#if workflowMode === "encodeOnly"}
          {#if loadingTrickplay}
            Extracting...
          {:else if frames.length > 0}
            Trickplay ({frames.length})
          {:else}
            Local Video
          {/if}
        {:else}
          Disc Inspector
        {/if}
      </span>
    </div>

    <div class="flex items-center gap-1">
      {#if workflowMode === "encodeOnly" && activeFrame}
        <button
          type="button"
          onclick={() => (lightboxOpen = true)}
          title="Open fullscreen frame inspector"
          class="rounded border border-[#E5E3DD] bg-[#F8F7F4] px-2 py-0.5 text-[10px] font-semibold text-[#111111] transition hover:bg-[#EFECE6]"
        >
          ⤢ Fullscreen
        </button>
      {/if}
      <button
        type="button"
        onclick={onClose}
        aria-label="Close preview panel"
        class="rounded px-1.5 py-0.5 text-xs font-medium text-[#787772] transition hover:bg-[#F8F7F4] hover:text-[#111111]"
      >
        ✕
      </button>
    </div>
  </header>

  <div class="flex-1 space-y-3.5 overflow-y-auto p-4">
    <!-- 16:9 Trickplay Frame Stage -->
    <div
      class="group relative aspect-video w-full overflow-hidden rounded-lg border border-[#DFDDD6] bg-[#141413]"
    >
      {#if workflowMode === "encodeOnly" && selectedLocalFile}
        {#if loadingTrickplay}
          <div
            class="flex h-full flex-col items-center justify-center gap-2 px-5 text-center"
          >
            <div
              class="h-6 w-6 animate-spin rounded-full border-2 border-white/20 border-t-[#58B0F9]"
            ></div>
            <p class="max-w-[260px] truncate text-xs font-semibold text-white">
              {fileBaseName(selectedLocalFile)}
            </p>
            <p class="font-mono text-[10px] text-[#9A9891]">
              Auto-generating trickplay frames...
            </p>
          </div>
        {:else if activeFrame}
          <button
            type="button"
            onclick={() => (lightboxOpen = true)}
            aria-label="Open frame in fullscreen"
            class="block h-full w-full cursor-zoom-in"
          >
            <img
              src={frameImageSrc(activeFrame)}
              alt={`Trickplay frame at ${activeFrame.timestampLabel}`}
              class="h-full w-full object-contain"
            />
          </button>

          <!-- Top-left Aspect / Resolution Pill -->
          {#if trickplay?.mediaInfo.width && trickplay.mediaInfo.height}
            <div
              class="pointer-events-none absolute top-2 left-2 flex items-center gap-1 rounded bg-black/70 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-white backdrop-blur-[2px]"
            >
              <span>{trickplay.mediaInfo.width}×{trickplay.mediaInfo.height}</span>
              {#if trickplay.mediaInfo.displayAspectRatio}
                <span class="text-white/50">·</span>
                <span class="text-[#8CCBFF]">
                  {trickplay.mediaInfo.displayAspectRatio}
                </span>
              {/if}
            </div>
          {/if}

          <!-- Hover Prev / Next Controls -->
          {#if frames.length > 1}
            <button
              type="button"
              onclick={() => stepFrame(-1)}
              aria-label="Previous frame"
              class="absolute top-1/2 left-2 flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded-full bg-black/65 text-sm font-bold text-white opacity-0 transition hover:bg-black/90 group-hover:opacity-100"
            >
              ‹
            </button>
            <button
              type="button"
              onclick={() => stepFrame(1)}
              aria-label="Next frame"
              class="absolute top-1/2 right-2 flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded-full bg-black/65 text-sm font-bold text-white opacity-0 transition hover:bg-black/90 group-hover:opacity-100"
            >
              ›
            </button>
          {/if}

          <!-- Bottom Timestamp + Fullscreen Trigger -->
          <div
            class="absolute right-2 bottom-2 left-2 flex items-center justify-between gap-2"
          >
            <span
              class="rounded bg-black/75 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-white"
            >
              {activeFrame.timestampLabel} · {activeFrameIndex + 1}/{frames.length}
            </span>
            <button
              type="button"
              onclick={() => (lightboxOpen = true)}
              class="rounded bg-black/75 px-2 py-0.5 font-mono text-[10px] font-semibold text-white transition hover:bg-[#1D8BF8]"
            >
              ⤢ Fullscreen
            </button>
          </div>
        {:else}
          <div
            class="flex h-full flex-col items-center justify-center px-5 text-center"
          >
            <span
              class="mb-2 rounded-full border border-[#F6BE4F]/40 bg-[#F6BE4F]/15 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-[#F6BE4F]"
            >
              FRAME PREVIEW UNAVAILABLE
            </span>
            <p class="max-w-[260px] truncate text-xs font-semibold text-white">
              {fileBaseName(selectedLocalFile)}
            </p>
            <p class="mt-1 text-[11px] leading-relaxed text-[#A4A29B]">
              {trickplayError ??
                trickplay?.note ??
                "Install ffmpeg on PATH to extract trickplay frames for all MKV/HEVC codecs."}
            </p>
          </div>
        {/if}
      {:else if selectedTitle}
        <div
          class="flex h-full flex-col items-center justify-center px-5 text-center"
        >
          <span
            class="mb-2 rounded-full border border-[#58B0F9]/40 bg-[#58B0F9]/15 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-[#58B0F9]"
          >
            TITLE #{selectedTitle.titleId} · {selectedTitle.duration ?? "--:--"}
          </span>
          <p class="max-w-[260px] truncate text-xs font-semibold text-white">
            {selectedTitle.name ?? `Title #${selectedTitle.titleId}`}
          </p>
          <p class="mt-1 text-[11px] leading-relaxed text-[#A4A29B]">
            Pre-rip optical disc metadata. Trickplay frames unlock automatically
            for local video files.
          </p>
        </div>
      {:else if posterUrl}
        <div class="flex h-full items-center gap-3.5 bg-[#161615] p-3.5">
          <img
            src={posterUrl}
            alt={mediaTitle || "Selected media poster"}
            class="h-full w-auto shrink-0 rounded border border-white/15 object-cover shadow"
          />
          <div class="min-w-0 flex-1 space-y-1 text-left">
            <span
              class="inline-block rounded-full border border-[#3AD27E]/40 bg-[#3AD27E]/15 px-2 py-0.5 font-mono text-[9px] font-semibold text-[#3AD27E]"
            >
              MATCHED METADATA
            </span>
            <p class="truncate text-xs font-bold text-white">
              {mediaTitle || "Untitled"}{mediaYear ? ` (${mediaYear})` : ""}
            </p>
            {#if mediaOverview}
              <p class="line-clamp-4 text-[11px] leading-relaxed text-[#B8B6AF]">
                {mediaOverview}
              </p>
            {/if}
          </div>
        </div>
      {:else}
        <div
          class="flex h-full flex-col items-center justify-center px-6 text-center"
        >
          <div
            class="mb-2.5 flex h-9 w-9 items-center justify-center rounded-full border border-white/15 bg-white/5 text-[#A4A29B]"
          >
            <svg
              class="h-4 w-4"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              aria-hidden="true"
            >
              <rect x="3" y="5" width="18" height="14" rx="2" />
              <circle cx="9" cy="10" r="1.5" />
              <path d="m21 15-5-4-4 3-3-2-6 5" />
            </svg>
          </div>
          <p class="text-xs font-semibold text-white/90">No source selected</p>
          <p class="mt-1 text-[11px] leading-relaxed text-[#9A9891]">
            {workflowMode === "encodeOnly"
              ? "Click any added video file in the table to auto-generate trickplay preview frames."
              : "Scan a disc and click any title row to inspect its streams and output path."}
          </p>
        </div>
      {/if}
    </div>

    <!-- Interactive Trickplay Scrubber & 4x2 Thumbnail Filmstrip -->
    {#if workflowMode === "encodeOnly" && selectedLocalFile && frames.length > 0}
      <div class="space-y-2">
        {#if frames.length > 1}
          <div class="flex items-center gap-2">
            <span class="font-mono text-[10px] text-[#787772]">
              {frames[0].timestampLabel}
            </span>
            <label for="trickplay-scrubber" class="sr-only">
              Scrub preview frames
            </label>
            <input
              id="trickplay-scrubber"
              type="range"
              min={0}
              max={frames.length - 1}
              step={1}
              value={activeFrameIndex}
              oninput={handleScrubInput}
              class="h-1.5 flex-1 cursor-pointer accent-[#1D8BF8]"
            />
            <span class="font-mono text-[10px] text-[#787772]">
              {frames[frames.length - 1].timestampLabel}
            </span>
          </div>
        {/if}

        <div class="grid grid-cols-4 gap-1.5">
          {#each frames as frame, idx (frame.index)}
            <button
              type="button"
              onclick={() => (activeFrameIndex = idx)}
              ondblclick={() => {
                activeFrameIndex = idx;
                lightboxOpen = true;
              }}
              title={`Jump to ${frame.timestampLabel} (double-click for fullscreen)`}
              class={`relative overflow-hidden rounded border bg-[#141413] transition ${
                idx === activeFrameIndex
                  ? "border-[#1D8BF8] ring-2 ring-[#1D8BF8]/35"
                  : "border-[#DFDDD6] opacity-80 hover:border-[#111111] hover:opacity-100"
              }`}
            >
              <img
                src={frameImageSrc(frame)}
                alt={`Frame ${idx + 1} at ${frame.timestampLabel}`}
                class="aspect-video w-full object-contain"
              />
              <span
                class="absolute right-0.5 bottom-0.5 rounded bg-black/75 px-1 py-0.2 font-mono text-[8px] font-semibold text-white"
              >
                {frame.timestampLabel}
              </span>
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Source Video Quality & Aspect Ratio Specs (Encode Only) -->
    {#if workflowMode === "encodeOnly" && selectedLocalFile && trickplay}
      <div
        class="divide-y divide-[#EFECE6] overflow-hidden rounded-lg border border-[#E5E3DD] text-xs"
      >
        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#787772]">File</span>
          <span
            class="col-span-8 truncate font-semibold text-[#111111]"
            title={selectedLocalFile}
          >
            {fileBaseName(selectedLocalFile)}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#787772]">Resolution</span>
          <span class="col-span-8 font-mono text-[11px] font-semibold text-[#111111]">
            {#if trickplay.mediaInfo.width && trickplay.mediaInfo.height}
              {trickplay.mediaInfo.width}×{trickplay.mediaInfo.height}
              {#if trickplay.mediaInfo.displayWidth && trickplay.mediaInfo.displayWidth !== trickplay.mediaInfo.width}
                <span class="text-[#787772]">
                  → {trickplay.mediaInfo.displayWidth}×{trickplay.mediaInfo.displayHeight}
                </span>
              {/if}
            {:else}
              --
            {/if}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#787772]">Aspect Ratio</span>
          <span class="col-span-8 font-mono text-[11px] text-[#111111]">
            {trickplay.mediaInfo.displayAspectRatio ?? "--"}
            {#if trickplay.mediaInfo.sampleAspectRatio && trickplay.mediaInfo.sampleAspectRatio !== "1:1"}
              <span class="text-[#787772]">
                (SAR {trickplay.mediaInfo.sampleAspectRatio})
              </span>
            {/if}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#787772]">Video</span>
          <span class="col-span-8 truncate font-mono text-[11px] text-[#111111]">
            {trickplay.mediaInfo.videoCodec ?? "--"}
            {#if trickplay.mediaInfo.fps}
              · {trickplay.mediaInfo.fps.toFixed(2)} fps
            {/if}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#787772]">Duration</span>
          <span class="col-span-8 font-mono text-[11px] text-[#111111]">
            {formatDuration(trickplay.mediaInfo.durationSeconds)}
            {#if trickplay.mediaInfo.bitrateKbps}
              · {formatBitrate(trickplay.mediaInfo.bitrateKbps)}
            {/if}
            {#if trickplay.mediaInfo.fileSizeBytes}
              · {formatFileSize(trickplay.mediaInfo.fileSizeBytes)}
            {/if}
          </span>
        </div>

        {#if trickplay.mediaInfo.audioSummary}
          <div class="grid grid-cols-12 px-3 py-1.5">
            <span class="col-span-4 font-medium text-[#787772]">Audio / Sub</span>
            <span class="col-span-8 truncate font-mono text-[11px] text-[#111111]">
              {trickplay.mediaInfo.audioSummary}
              {#if trickplay.mediaInfo.subtitleCount > 0}
                · {trickplay.mediaInfo.subtitleCount} sub{trickplay.mediaInfo.subtitleCount === 1 ? "" : "s"}
              {/if}
            </span>
          </div>
        {/if}
      </div>
    {/if}

    <!-- Output Target Card -->
    <div class="space-y-1.5 rounded-lg border border-[#E5E3DD] bg-[#FAF9F6] p-3">
      <div class="flex items-center justify-between">
        <span
          class="text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
        >
          Target Output ({namingConvention})
        </span>
        <span
          class="rounded border border-[#E5E3DD] bg-white px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#111111]"
        >
          .{workflowMode === "ripOnly" ? "mkv" : outputContainer}
        </span>
      </div>
      <p
        class="break-all font-mono text-[11px] leading-relaxed text-[#111111]"
        title={outputFilenamePreview}
      >
        {outputFilenamePreview}
      </p>
    </div>

    <!-- Active Quality Profile Summary -->
    {#if workflowMode !== "ripOnly" && activeQualityProfile}
      <div
        class="space-y-1.5 rounded-lg border border-[#E5E3DD] bg-[#FAF9F6] p-3"
      >
        <div class="flex items-center justify-between gap-2">
          <div class="flex items-center gap-1.5">
            <span
              class={`rounded-full border px-2 py-0.5 text-[10px] font-bold uppercase ${
                activeQualityProfile.delivery === "local"
                  ? "border-[#24B465] bg-[#3AD27E] text-[#111111]"
                  : "border-[#3894E2] bg-[#58B0F9] text-[#111111]"
              }`}
            >
              {activeQualityProfile.delivery}
            </span>
            <span class="font-mono text-xs font-bold text-[#111111]">
              {shortResolutionLabel(activeQualityProfile.resolution)}
            </span>
          </div>
          <button
            type="button"
            onclick={onEditQuality}
            class="text-[11px] font-semibold text-[#1D8BF8] hover:underline"
          >
            Tune Quality
          </button>
        </div>

        <div
          class="flex flex-wrap items-center gap-1.5 font-mono text-[10px] text-[#52524E]"
        >
          <span>{shortEncoderLabel(activeQualityProfile.encoder)}</span>
          <span>·</span>
          <span>RF {activeQualityProfile.rfQuality.toFixed(1)}</span>
          <span>·</span>
          <span>{shortAudioLabel(activeQualityProfile.audioMode)}</span>
          <span>·</span>
          <span>{activeQualityProfile.estimatedBitrateLabel}</span>
        </div>

        {#if scalingNote}
          <p class="font-mono text-[10px] font-medium text-[#155CA2]">
            {scalingNote}
          </p>
        {/if}
      </div>
    {/if}

    <!-- Disc Title Specs (Rip Only / Rip + Encode) -->
    {#if workflowMode !== "encodeOnly" && selectedTitle}
      <div
        class="divide-y divide-[#EFECE6] overflow-hidden rounded-lg border border-[#E5E3DD] text-xs"
      >
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#787772]">Title ID</span>
          <span class="col-span-8 font-mono font-semibold text-[#111111]">
            #{selectedTitle.titleId}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#787772]">Duration</span>
          <span class="col-span-8 font-mono text-[#111111]">
            {selectedTitle.duration ?? "--:--:--"}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#787772]">Size</span>
          <span class="col-span-8 font-mono text-[#111111]">
            {selectedTitle.sizeHuman ?? "--"}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#787772]">Chapters</span>
          <span class="col-span-8 font-mono text-[#111111]">
            {selectedTitle.chapterCount ?? "--"}
          </span>
        </div>
      </div>

      <!-- Stream Pills -->
      <div class="space-y-2">
        <div
          class="text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
        >
          Streams ({selectedTitle.streams.length})
        </div>
        {#if selectedTitle.streams.length === 0}
          <p class="text-[11px] text-[#787772]">No streams reported.</p>
        {:else}
          <div class="flex flex-wrap gap-1.5">
            {#each selectedTitle.streams as track (track.streamId)}
              <span
                class={`inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-semibold ${trackBadgeClass(track)}`}
              >
                {formatTrackSummary(track)}
              </span>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <!-- Clean Trickplay Footer (Encode Only) -->
  {#if workflowMode === "encodeOnly" && selectedLocalFile}
    <footer
      class="flex items-center gap-2 border-t border-[#ECEAE4] bg-[#FAF9F6] p-3"
    >
      <button
        type="button"
        onclick={onRegenerateFrames}
        disabled={loadingTrickplay}
        class="rounded-md border border-[#DFDDD6] bg-white px-2.5 py-1.5 text-[11px] font-semibold text-[#111111] transition hover:bg-[#EFECE6] disabled:opacity-50"
      >
        {loadingTrickplay ? "Extracting..." : "Refresh Frames"}
      </button>

      <button
        type="button"
        onclick={() => (lightboxOpen = true)}
        disabled={!activeFrame || loadingTrickplay}
        class="flex-1 rounded-md bg-[#111111] px-3 py-1.5 text-xs font-semibold text-white transition hover:bg-[#2A2A2A] disabled:cursor-not-allowed disabled:bg-[#C9C7C0] disabled:text-[#6B6A65]"
      >
        ⤢ Inspect Fullscreen
      </button>
    </footer>
  {/if}
</aside>

<FrameLightbox
  open={lightboxOpen}
  fileName={selectedLocalFile ? fileBaseName(selectedLocalFile) : ""}
  preview={trickplay}
  activeIndex={activeFrameIndex}
  onSelectIndex={(idx) => (activeFrameIndex = idx)}
  onClose={() => (lightboxOpen = false)}
/>
