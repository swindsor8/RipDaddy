<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { PreviewFrame, TrickplayPreview } from "./types";

  interface Props {
    open: boolean;
    fileName: string;
    preview: TrickplayPreview | null;
    activeIndex: number;
    onSelectIndex: (index: number) => void;
    onClose: () => void;
  }

  let {
    open,
    fileName,
    preview,
    activeIndex,
    onSelectIndex,
    onClose,
  }: Props = $props();

  let zoomActual = $state(false);
  let isNativeFullscreen = $state(false);
  let containerEl = $state<HTMLDivElement | null>(null);

  const frames = $derived(preview?.frames ?? []);
  const activeFrame = $derived<PreviewFrame | null>(
    frames[activeIndex] ?? frames[0] ?? null,
  );

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
    const next = (activeIndex + delta + frames.length) % frames.length;
    onSelectIndex(next);
  }

  async function toggleNativeFullscreen(): Promise<void> {
    if (!containerEl) return;
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen();
      } else if (containerEl.requestFullscreen) {
        await containerEl.requestFullscreen();
      }
    } catch {
      // Fullscreen API may be restricted in some WebViews; overlay remains full-window.
    }
  }

  function handleFullscreenChange(): void {
    isNativeFullscreen = Boolean(document.fullscreenElement);
  }

  function handleClose(): void {
    if (document.fullscreenElement) {
      void document.exitFullscreen().catch(() => {});
    }
    zoomActual = false;
    onClose();
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (!open) return;
    if (event.key === "Escape") {
      event.preventDefault();
      handleClose();
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      stepFrame(-1);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      stepFrame(1);
    } else if (event.key === "z" || event.key === "Z") {
      event.preventDefault();
      zoomActual = !zoomActual;
    } else if (event.key === "f" || event.key === "F") {
      event.preventDefault();
      void toggleNativeFullscreen();
    }
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />
<svelte:document onfullscreenchange={handleFullscreenChange} />

{#if open && preview && activeFrame}
  <div
    bind:this={containerEl}
    role="dialog"
    aria-modal="true"
    aria-label={`Fullscreen frame preview: ${fileName}`}
    class="fixed inset-0 z-[60] flex flex-col bg-[#120E0D]/95 text-white backdrop-blur-sm"
  >
    <!-- Top HUD -->
    <header
      class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-white/10 bg-black/50 px-5 py-3"
    >
      <div class="flex min-w-0 items-center gap-2.5">
        <span class="truncate text-sm font-bold text-white" title={fileName}>
          {fileName}
        </span>

        {#if preview.mediaInfo.width && preview.mediaInfo.height}
          <span
            class="rounded-full border border-white/15 bg-white/10 px-2 py-0.5 font-mono text-[11px] font-semibold text-white/90"
          >
            {preview.mediaInfo.width}×{preview.mediaInfo.height}
          </span>
        {/if}

        {#if preview.mediaInfo.displayAspectRatio}
          <span
            class="rounded-full border border-[#5BC0EB]/40 bg-[#5BC0EB]/15 px-2 py-0.5 font-mono text-[11px] font-semibold text-[#82D6FA]"
          >
            {preview.mediaInfo.displayAspectRatio}
          </span>
        {/if}

        {#if preview.mediaInfo.videoCodec}
          <span
            class="rounded-full border border-[#4CDE99]/40 bg-[#4CDE99]/15 px-2 py-0.5 font-mono text-[11px] font-semibold text-[#7AF0B8]"
          >
            {preview.mediaInfo.videoCodec}
          </span>
        {/if}

        <span class="font-mono text-xs text-white/60">
          Frame {activeIndex + 1} / {frames.length} · {activeFrame.timestampLabel}
        </span>
      </div>

      <div class="flex shrink-0 items-center gap-2">
        <button
          type="button"
          onclick={() => (zoomActual = !zoomActual)}
          title="Toggle Fit vs 1:1 Actual Pixels (Z)"
          class={`rounded-md border px-2.5 py-1 text-xs font-semibold transition ${
            zoomActual
              ? "border-[#11824F] bg-[#11824F] text-white"
              : "border-white/15 bg-white/10 text-white/90 hover:bg-white/20"
          }`}
        >
          {zoomActual ? "1:1 Actual Pixels" : "Fit Screen"}
        </button>

        <button
          type="button"
          onclick={() => void toggleNativeFullscreen()}
          title="Toggle monitor fullscreen (F)"
          class="rounded-md border border-white/15 bg-white/10 px-2.5 py-1 text-xs font-semibold text-white/90 transition hover:bg-white/20"
        >
          {isNativeFullscreen ? "Exit Monitor Fullscreen" : "Monitor Fullscreen"}
        </button>

        <button
          type="button"
          onclick={handleClose}
          title="Close preview (Esc)"
          class="rounded-md border border-white/15 bg-white/10 px-2.5 py-1 text-xs font-semibold text-white transition hover:bg-white/20"
        >
          Close ✕
        </button>
      </div>
    </header>

    <!-- Main Image Stage -->
    <div
      class={`relative flex min-h-0 flex-1 ${
        zoomActual
          ? "overflow-auto p-6"
          : "items-center justify-center overflow-hidden p-4"
      }`}
    >
      {#if frames.length > 1}
        <button
          type="button"
          onclick={() => stepFrame(-1)}
          aria-label="Previous frame"
          class="fixed top-1/2 left-4 z-10 flex h-11 w-11 -translate-y-1/2 items-center justify-center rounded-full border border-white/15 bg-black/60 text-xl font-bold text-white transition hover:bg-black/85"
        >
          ‹
        </button>
        <button
          type="button"
          onclick={() => stepFrame(1)}
          aria-label="Next frame"
          class="fixed top-1/2 right-4 z-10 flex h-11 w-11 -translate-y-1/2 items-center justify-center rounded-full border border-white/15 bg-black/60 text-xl font-bold text-white transition hover:bg-black/85"
        >
          ›
        </button>
      {/if}

      <img
        src={frameImageSrc(activeFrame)}
        alt={`Preview frame at ${activeFrame.timestampLabel}`}
        class={zoomActual
          ? "m-auto block max-w-none rounded shadow-2xl"
          : "max-h-full max-w-full rounded object-contain shadow-2xl"}
      />
    </div>

    <!-- Bottom Trickplay Filmstrip -->
    <footer
      class="shrink-0 border-t border-white/10 bg-black/60 px-5 py-3"
    >
      <div class="mx-auto flex max-w-5xl items-center justify-center gap-2 overflow-x-auto pb-0.5">
        {#each frames as frame, idx (frame.index)}
          <button
            type="button"
            onclick={() => onSelectIndex(idx)}
            class={`group relative w-28 shrink-0 overflow-hidden rounded-md border-2 bg-[#1E1816] transition ${
              idx === activeIndex
                ? "border-[#11824F] ring-2 ring-[#11824F]/40"
                : "border-white/15 opacity-70 hover:border-white/50 hover:opacity-100"
            }`}
          >
            <img
              src={frameImageSrc(frame)}
              alt={`Thumbnail at ${frame.timestampLabel}`}
              class="aspect-video w-full object-contain"
            />
            <span
              class="absolute right-1 bottom-1 rounded bg-black/75 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-white"
            >
              {frame.timestampLabel}
            </span>
          </button>
        {/each}
      </div>
      <p class="mt-2 text-center font-mono text-[10px] text-white/45">
        ← / → switch frames · Z toggle 1:1 actual pixels · F monitor fullscreen · Esc close
      </p>
    </footer>
  </div>
{/if}
