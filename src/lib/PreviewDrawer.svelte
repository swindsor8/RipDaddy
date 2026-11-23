<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import {
    shortAudioLabel,
    shortEncoderLabel,
    shortResolutionLabel,
  } from "./qualityDefaults";
  import type {
    DiscTitle,
    NamingConvention,
    PipelineMode,
    QualityProfile,
    StreamTrack,
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
    onClose,
    onEditQuality,
  }: Props = $props();

  let videoLoadError = $state(false);

  const localVideoSrc = $derived.by(() => {
    if (!selectedLocalFile) return null;
    try {
      return convertFileSrc(selectedLocalFile);
    } catch {
      return null;
    }
  });

  // Reset error flag whenever the selected local file changes
  $effect(() => {
    const _current = selectedLocalFile;
    void _current;
    videoLoadError = false;
  });

  function fileBaseName(path: string): string {
    const parts = path.split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

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
        {workflowMode === "encodeOnly" ? "Local Video" : "Disc Inspector"}
      </span>
    </div>
    <button
      type="button"
      onclick={onClose}
      aria-label="Close preview panel"
      class="rounded px-1.5 py-0.5 text-xs font-medium text-[#787772] transition hover:bg-[#F8F7F4] hover:text-[#111111]"
    >
      ✕
    </button>
  </header>

  <div class="flex-1 space-y-4 overflow-y-auto p-4">
    <!-- 16:9 Visual Preview Canvas -->
    <div
      class="relative aspect-video w-full overflow-hidden rounded-lg border border-[#DFDDD6] bg-[#141413]"
    >
      {#if workflowMode === "encodeOnly" && selectedLocalFile && localVideoSrc && !videoLoadError}
        <video
          src={localVideoSrc}
          controls
          preload="metadata"
          onerror={() => (videoLoadError = true)}
          class="h-full w-full object-contain"
        >
          <track kind="captions" />
        </video>
      {:else if workflowMode === "encodeOnly" && selectedLocalFile}
        <div
          class="flex h-full flex-col items-center justify-center px-5 text-center"
        >
          <span
            class="mb-2 rounded-full border border-[#F6BE4F]/40 bg-[#F6BE4F]/15 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-[#F6BE4F]"
          >
            CODEC PREVIEW UNAVAILABLE
          </span>
          <p class="max-w-[260px] truncate text-xs font-semibold text-white">
            {fileBaseName(selectedLocalFile)}
          </p>
          <p class="mt-1 text-[11px] leading-relaxed text-[#A4A29B]">
            WebView cannot decode this container/codec natively. Generate a 10s
            HandBrake sample clip to preview crop and quality.
          </p>
        </div>
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
            Pre-rip optical disc metadata. Frame preview unlocks once ripped to
            local MKV.
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
              <polygon points="6 4 20 12 6 20 6 4" />
            </svg>
          </div>
          <p class="text-xs font-semibold text-white/90">No source selected</p>
          <p class="mt-1 text-[11px] leading-relaxed text-[#9A9891]">
            {workflowMode === "encodeOnly"
              ? "Click any local video row in the table to play and inspect it here."
              : "Scan a disc and click any title row to inspect its streams and output path."}
          </p>
        </div>
      {/if}
    </div>

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

    <!-- Active Quality Profile Summary (when HandBrake encoding is active) -->
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

        <div class="flex flex-wrap items-center gap-1.5 font-mono text-[10px] text-[#52524E]">
          <span>{shortEncoderLabel(activeQualityProfile.encoder)}</span>
          <span>·</span>
          <span>RF {activeQualityProfile.rfQuality.toFixed(1)}</span>
          <span>·</span>
          <span>{shortAudioLabel(activeQualityProfile.audioMode)}</span>
          <span>·</span>
          <span>{activeQualityProfile.estimatedBitrateLabel}</span>
        </div>
      </div>
    {/if}

    <!-- Specs / Key-Value Table -->
    {#if workflowMode === "encodeOnly" && selectedLocalFile}
      <div
        class="divide-y divide-[#EFECE6] overflow-hidden rounded-lg border border-[#E5E3DD] text-xs"
      >
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#787772]">File</span>
          <span class="col-span-8 truncate font-semibold text-[#111111]">
            {fileBaseName(selectedLocalFile)}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#787772]">Source</span>
          <span
            class="col-span-8 truncate font-mono text-[11px] text-[#52524E]"
            title={selectedLocalFile}
          >
            {selectedLocalFile}
          </span>
        </div>
        <div class="grid grid-cols-12 px-3 py-2">
          <span class="col-span-4 font-medium text-[#787772]">Pipeline</span>
          <span class="col-span-8 font-medium text-[#111111]">
            HandBrakeCLI (--json)
          </span>
        </div>
      </div>
    {:else if selectedTitle}
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

  <!-- Preview Footer Action -->
  {#if workflowMode !== "ripOnly"}
    <footer class="border-t border-[#ECEAE4] bg-[#FAF9F6] p-3">
      <button
        type="button"
        disabled={!selectedLocalFile && !selectedTitle}
        title="10-second HandBrakeCLI sample encode preview will activate once the encode worker is wired"
        class="w-full rounded-md border border-[#DFDDD6] bg-white px-3 py-1.5 text-xs font-semibold text-[#111111] transition hover:bg-[#F2F0EA] disabled:cursor-not-allowed disabled:opacity-50"
      >
        Generate 10s Sample Clip
      </button>
    </footer>
  {/if}
</aside>
