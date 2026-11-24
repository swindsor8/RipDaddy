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
    PreviewFrame,
    QualityProfile,
    ResolutionTier,
    StreamTrack,
    TrickplayPreview,
    WorkspaceMode,
  } from "./types";

  interface Props {
    workflowMode: WorkspaceMode;
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
    excludedStreamIds?: number[];
    onToggleTrack?: (streamId: number) => void;
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
    excludedStreamIds = [],
    onToggleTrack,
    onRegenerateFrames,
    onClose,
    onEditQuality,
  }: Props = $props();

  let activeFrameIndex = $state(0);
  let lightboxOpen = $state(false);

  const isLocalMode = $derived(
    workflowMode === "encodeOnly" || workflowMode === "organizeOnly",
  );

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
        return "border-[#9CD9F5] bg-[#E6F6FD] text-[#0E5678]";
      case "audio":
        return "border-[#9BE5C1] bg-[#E6FBF1] text-[#0E4D31]";
      case "subtitle":
        return "border-[#F0CB82] bg-[#FDF3DC] text-[#563406]";
      case "unknown":
        return "border-[#E6E0D6] bg-[#FAF8F4] text-[#61554F]";
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
  class="flex min-w-0 flex-[1_1_0%] flex-col border-l border-[#E6E0D6] bg-white"
>
  <!-- Drawer Header -->
  <header
    class="flex items-center justify-between border-b border-[#EDE7DE] px-4 py-2.5"
  >
    <div class="flex items-center gap-2">
      <h2 class="text-xs font-bold tracking-[-0.01em] text-[#1A1513]">
        Preview
      </h2>
      <span
        class="rounded-full border border-[#E6E0D6] bg-[#FAF8F4] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#61554F]"
      >
        {#if isLocalMode}
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
      {#if isLocalMode && activeFrame}
        <button
          type="button"
          onclick={() => (lightboxOpen = true)}
          title="Open fullscreen frame inspector"
          class="rounded border border-[#E6E0D6] bg-[#FAF8F4] px-2 py-0.5 text-[10px] font-semibold text-[#1A1513] transition hover:bg-[#EDE7DD]"
        >
          ⤢ Fullscreen
        </button>
      {/if}
      <button
        type="button"
        onclick={onClose}
        aria-label="Close preview panel"
        class="rounded px-1.5 py-0.5 text-xs font-medium text-[#7D716A] transition hover:bg-[#FAF8F4] hover:text-[#1A1513]"
      >
        ✕
      </button>
    </div>
  </header>

  <div class="flex-1 space-y-3.5 overflow-y-auto p-4">
    <!-- 16:9 Trickplay Frame Stage -->
    <div
      class="group relative aspect-video w-full overflow-hidden rounded-lg border border-[#DAD3C7] bg-[#181311]"
    >
      {#if isLocalMode && selectedLocalFile}
        {#if loadingTrickplay}
          <div
            class="flex h-full flex-col items-center justify-center gap-2 px-5 text-center"
          >
            <div
              class="h-6 w-6 animate-spin rounded-full border-2 border-white/20 border-t-[#4CDE99]"
            ></div>
            <p class="max-w-full truncate text-xs font-semibold text-white">
              {fileBaseName(selectedLocalFile)}
            </p>
            <p class="font-mono text-[10px] text-[#9E928B]">
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
                <span class="text-[#82D6FA]">
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
              class="rounded bg-black/75 px-2 py-0.5 font-mono text-[10px] font-semibold text-white transition hover:bg-[#11824F]"
            >
              ⤢ Fullscreen
            </button>
          </div>
        {:else}
          <div
            class="flex h-full flex-col items-center justify-center px-5 text-center"
          >
            <span
              class="mb-2 rounded-full border border-[#F8B142]/40 bg-[#F8B142]/15 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-[#F8B142]"
            >
              FRAME PREVIEW UNAVAILABLE
            </span>
            <p class="max-w-full truncate text-xs font-semibold text-white">
              {fileBaseName(selectedLocalFile)}
            </p>
            <p class="mt-1 text-[11px] leading-relaxed text-[#A89C95]">
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
            class="mb-2 rounded-full border border-[#5BC0EB]/40 bg-[#5BC0EB]/15 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-[#5BC0EB]"
          >
            TITLE #{selectedTitle.titleId} · {selectedTitle.duration ?? "--:--"}
          </span>
          <p class="max-w-full truncate text-xs font-semibold text-white">
            {selectedTitle.name ?? `Title #${selectedTitle.titleId}`}
          </p>
          <p class="mt-1 text-[11px] leading-relaxed text-[#A89C95]">
            Pre-rip optical disc metadata. Trickplay frames unlock automatically
            for local video files.
          </p>
        </div>
      {:else if posterUrl}
        <div class="flex h-full items-center gap-3.5 bg-[#1E1816] p-3.5">
          <img
            src={posterUrl}
            alt={mediaTitle || "Selected media poster"}
            class="h-full w-auto shrink-0 rounded border border-white/15 object-cover shadow"
          />
          <div class="min-w-0 flex-1 space-y-1 text-left">
            <span
              class="inline-block rounded-full border border-[#4CDE99]/40 bg-[#4CDE99]/15 px-2 py-0.5 font-mono text-[9px] font-semibold text-[#4CDE99]"
            >
              MATCHED METADATA
            </span>
            <p class="truncate text-xs font-bold text-white">
              {mediaTitle || "Untitled"}{mediaYear ? ` (${mediaYear})` : ""}
            </p>
            {#if mediaOverview}
              <p class="line-clamp-4 text-[11px] leading-relaxed text-[#B6AEA2]">
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
            class="mb-2.5 flex h-9 w-9 items-center justify-center rounded-full border border-white/15 bg-white/5 text-[#A89C95]"
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
          <p class="mt-1 text-[11px] leading-relaxed text-[#9E928B]">
            {isLocalMode
              ? "Click any added video file in the table to auto-generate trickplay preview frames."
              : "Scan a disc and click any title row to inspect its streams and output path."}
          </p>
        </div>
      {/if}
    </div>

    <!-- Interactive Trickplay Scrubber & 4x2 Thumbnail Filmstrip -->
    {#if isLocalMode && selectedLocalFile && frames.length > 0}
      <div class="space-y-2">
        {#if frames.length > 1}
          <div class="flex items-center gap-2">
            <span class="font-mono text-[10px] text-[#7D716A]">
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
              class="h-1.5 flex-1 cursor-pointer accent-[#11824F]"
            />
            <span class="font-mono text-[10px] text-[#7D716A]">
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
              class={`relative overflow-hidden rounded border bg-[#181311] transition ${
                idx === activeFrameIndex
                  ? "border-[#11824F] ring-2 ring-[#11824F]/35"
                  : "border-[#DAD3C7] opacity-80 hover:border-[#1A1513] hover:opacity-100"
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

    <!-- Source Video Quality & Aspect Ratio Specs (Encode / Organize) -->
    {#if isLocalMode && selectedLocalFile && trickplay}
      <div
        class="divide-y divide-[#EDE7DD] overflow-hidden rounded-lg border border-[#E6E0D6] text-xs"
      >
        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#7D716A]">File</span>
          <span
            class="col-span-8 truncate font-semibold text-[#1A1513]"
            title={selectedLocalFile}
          >
            {fileBaseName(selectedLocalFile)}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#7D716A]">Resolution</span>
          <span class="col-span-8 font-mono text-[11px] font-semibold text-[#1A1513]">
            {#if trickplay.mediaInfo.width && trickplay.mediaInfo.height}
              {trickplay.mediaInfo.width}×{trickplay.mediaInfo.height}
              {#if trickplay.mediaInfo.displayWidth && trickplay.mediaInfo.displayWidth !== trickplay.mediaInfo.width}
                <span class="text-[#7D716A]">
                  → {trickplay.mediaInfo.displayWidth}×{trickplay.mediaInfo.displayHeight}
                </span>
              {/if}
            {:else}
              --
            {/if}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#7D716A]">Aspect Ratio</span>
          <span class="col-span-8 font-mono text-[11px] text-[#1A1513]">
            {trickplay.mediaInfo.displayAspectRatio ?? "--"}
            {#if trickplay.mediaInfo.sampleAspectRatio && trickplay.mediaInfo.sampleAspectRatio !== "1:1"}
              <span class="text-[#7D716A]">
                (SAR {trickplay.mediaInfo.sampleAspectRatio})
              </span>
            {/if}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#7D716A]">Video</span>
          <span class="col-span-8 truncate font-mono text-[11px] text-[#1A1513]">
            {trickplay.mediaInfo.videoCodec ?? "--"}
            {#if trickplay.mediaInfo.fps}
              · {trickplay.mediaInfo.fps.toFixed(2)} fps
            {/if}
          </span>
        </div>

        <div class="grid grid-cols-12 px-3 py-1.5">
          <span class="col-span-4 font-medium text-[#7D716A]">Duration</span>
          <span class="col-span-8 font-mono text-[11px] text-[#1A1513]">
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
            <span class="col-span-4 font-medium text-[#7D716A]">Audio / Sub</span>
            <span class="col-span-8 truncate font-mono text-[11px] text-[#1A1513]">
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
    <div class="space-y-1.5 rounded-lg border border-[#E6E0D6] bg-[#F7F4EE] p-3">
      <div class="flex items-center justify-between">
        <span
          class="text-[10px] font-bold uppercase tracking-[0.08em] text-[#7D716A]"
        >
          Target Output ({namingConvention})
        </span>
        <span
          class="rounded border border-[#E6E0D6] bg-white px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#1A1513]"
        >
          .{workflowMode === "ripOnly"
            ? "mkv"
            : workflowMode === "organizeOnly" && selectedLocalFile
              ? (selectedLocalFile.split(".").pop()?.toLowerCase() || "mkv")
              : outputContainer}
        </span>
      </div>
      <p
        class="break-all font-mono text-[11px] leading-relaxed text-[#1A1513]"
        title={outputFilenamePreview}
      >
        {outputFilenamePreview}
      </p>
    </div>

    <!-- Active Quality Profile Summary -->
    {#if workflowMode !== "ripOnly" && workflowMode !== "organizeOnly" && activeQualityProfile}
      <div
        class="space-y-1.5 rounded-lg border border-[#E6E0D6] bg-[#F7F4EE] p-3"
      >
        <div class="flex items-center justify-between gap-2">
          <div class="flex items-center gap-1.5">
            <span
              class={`rounded-full border px-2 py-0.5 text-[10px] font-bold uppercase ${
                activeQualityProfile.delivery === "local"
                  ? "border-[#1FA868] bg-[#4CDE99] text-[#1A1513]"
                  : "border-[#1C8EC0] bg-[#5BC0EB] text-[#1A1513]"
              }`}
            >
              {activeQualityProfile.delivery}
            </span>
            <span class="font-mono text-xs font-bold text-[#1A1513]">
              {shortResolutionLabel(activeQualityProfile.resolution)}
            </span>
          </div>
          <button
            type="button"
            onclick={onEditQuality}
            class="text-[11px] font-semibold text-[#11824F] hover:underline"
          >
            Tune Quality
          </button>
        </div>

        <div
          class="flex flex-wrap items-center gap-1.5 font-mono text-[10px] text-[#544943]"
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
          <p class="font-mono text-[10px] font-medium text-[#0E5678]">
            {scalingNote}
          </p>
        {/if}
      </div>
    {/if}

    <!-- Disc Title Specs (Rip Only / Rip + Encode) -->
    {#if !isLocalMode && selectedTitle}
      <div
        class="divide-y divide-[#EDE7DD] overflow-hidden rounded-lg border border-[#E6E0D6] text-xs"
      >
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#7D716A]">Title ID</span>
          <span class="col-span-8 font-mono font-semibold text-[#1A1513]">
            #{selectedTitle.titleId}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#7D716A]">Duration</span>
          <span class="col-span-8 font-mono text-[#1A1513]">
            {selectedTitle.duration ?? "--:--:--"}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#7D716A]">Size</span>
          <span class="col-span-8 font-mono text-[#1A1513]">
            {selectedTitle.sizeHuman ?? "--"}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#7D716A]">Chapters</span>
          <span class="col-span-8 font-mono text-[#1A1513]">
            {selectedTitle.chapterCount ?? "--"}
          </span>
        </div>
      </div>

      <!-- Interactive Stream Pills -->
      <div class="space-y-2">
        <div class="flex items-center justify-between gap-2">
          <span
            class="text-[10px] font-bold uppercase tracking-[0.08em] text-[#7D716A]"
          >
            Streams ({selectedTitle.streams.length})
          </span>
          {#if onToggleTrack && selectedTitle.streams.some((t) => t.trackType === "audio" || t.trackType === "subtitle")}
            <span class="text-[10px] text-[#7D716A]">
              Click audio/subs to toggle
            </span>
          {/if}
        </div>
        {#if selectedTitle.streams.length === 0}
          <p class="text-[11px] text-[#7D716A]">No streams reported.</p>
        {:else}
          <div class="flex flex-wrap gap-1.5">
            {#each selectedTitle.streams as track (track.streamId)}
              {@const toggleable =
                Boolean(onToggleTrack) &&
                (track.trackType === "audio" || track.trackType === "subtitle")}
              {@const excluded = excludedStreamIds.includes(track.streamId)}
              {#if toggleable}
                <button
                  type="button"
                  onclick={() => onToggleTrack?.(track.streamId)}
                  aria-pressed={!excluded}
                  title={excluded
                    ? `Excluded ${track.trackType} track — click to include in encode`
                    : `Included ${track.trackType} track — click to exclude from encode`}
                  class={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-semibold transition ${
                    excluded
                      ? "border-dashed border-[#D6CFC3] bg-[#FAF8F4] text-[#9E928B] line-through opacity-75 hover:border-[#1A1513] hover:text-[#1A1513]"
                      : `${trackBadgeClass(track)} hover:brightness-95`
                  }`}
                >
                  <span>{excluded ? "✕" : "✓"}</span>
                  <span>{formatTrackSummary(track)}</span>
                </button>
              {:else}
                <span
                  class={`inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-semibold ${trackBadgeClass(track)}`}
                >
                  {formatTrackSummary(track)}
                </span>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <!-- Clean Trickplay Footer (Encode / Organize) -->
  {#if isLocalMode && selectedLocalFile}
    <footer
      class="flex items-center gap-2 border-t border-[#EDE7DE] bg-[#F7F4EE] p-3"
    >
      <button
        type="button"
        onclick={onRegenerateFrames}
        disabled={loadingTrickplay}
        class="rounded-md border border-[#DAD3C7] bg-white px-2.5 py-1.5 text-[11px] font-semibold text-[#1A1513] transition hover:bg-[#EDE7DD] disabled:opacity-50"
      >
        {loadingTrickplay ? "Extracting..." : "Refresh Frames"}
      </button>

      <button
        type="button"
        onclick={() => (lightboxOpen = true)}
        disabled={!activeFrame || loadingTrickplay}
        class="flex-1 rounded-md bg-[#1A1513] px-3 py-1.5 text-xs font-semibold text-white transition hover:bg-[#332925] disabled:cursor-not-allowed disabled:bg-[#C6BEB1] disabled:text-[#70645D]"
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
