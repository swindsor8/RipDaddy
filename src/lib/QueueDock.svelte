<script lang="ts">
  import type { JobStage, QueueJob, ReorderAction } from "./types";

  interface Props {
    expanded: boolean;
    jobs: QueueJob[];
    paused: boolean;
    restoredPendingCount: number;
    jobLogs: Record<string, string[]>;
    onToggleExpanded: () => void;
    onTogglePaused: () => void;
    onCancelJob: (jobId: string) => void;
    onRetryJob: (jobId: string) => void;
    onMoveJob: (jobId: string, action: ReorderAction) => void;
    onReorderJobToIndex: (jobId: string, targetIndex: number) => void;
    onRemoveJob: (jobId: string) => void;
    onRevealPath: (path: string) => void;
    onClearFinished: () => void;
  }

  let {
    expanded,
    jobs,
    paused,
    restoredPendingCount,
    jobLogs,
    onToggleExpanded,
    onTogglePaused,
    onCancelJob,
    onRetryJob,
    onMoveJob,
    onReorderJobToIndex,
    onRemoveJob,
    onRevealPath,
    onClearFinished,
  }: Props = $props();

  let expandedLogJobIds = $state<string[]>([]);
  let copiedLogJobId = $state<string | null>(null);
  let draggedJobId = $state<string | null>(null);
  let dragOverJobId = $state<string | null>(null);

  function isPendingStage(stage: JobStage): boolean {
    return stage === "queuedForRip" || stage === "queuedForEncode";
  }

  function toggleJobLogs(jobId: string): void {
    if (expandedLogJobIds.includes(jobId)) {
      expandedLogJobIds = expandedLogJobIds.filter((id) => id !== jobId);
    } else {
      expandedLogJobIds = [...expandedLogJobIds, jobId];
    }
  }

  async function copyJobLogs(jobId: string): Promise<void> {
    const lines = jobLogs[jobId] ?? [];
    if (lines.length === 0) return;
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      copiedLogJobId = jobId;
      setTimeout(() => {
        if (copiedLogJobId === jobId) {
          copiedLogJobId = null;
        }
      }, 2000);
    } catch {
      copiedLogJobId = null;
    }
  }

  function logLineClass(line: string): string {
    if (line.includes("] $ ")) return "text-[#4CDE99]";
    if (line.includes("[stderr]")) return "text-[#F8B142]";
    if (line.includes("--- Retrying")) return "text-[#5BC0EB]";
    return "text-[#EAE2D8]";
  }

  const activeJobs = $derived(
    jobs.filter(
      (job) =>
        job.stage !== "completed" &&
        job.stage !== "cancelled" &&
        job.stage !== "failed",
    ),
  );

  const pendingIndices = $derived(
    jobs
      .map((job, idx) => (isPendingStage(job.stage) ? idx : -1))
      .filter((idx) => idx >= 0),
  );

  const firstPendingIndex = $derived(
    pendingIndices.length > 0 ? pendingIndices[0] : -1,
  );
  const lastPendingIndex = $derived(
    pendingIndices.length > 0 ? pendingIndices[pendingIndices.length - 1] : -1,
  );

  const hasFinishedJobs = $derived(
    jobs.some(
      (job) =>
        job.stage === "completed" ||
        job.stage === "cancelled" ||
        job.stage === "failed",
    ),
  );

  const activeRipJob = $derived(
    jobs.find((job) => job.stage === "ripping") ?? null,
  );

  const activeEncodeJob = $derived(
    jobs.find(
      (job) => job.stage === "encoding" || job.stage === "cleaningTemp",
    ) ?? null,
  );

  function formatStageLabel(stage: JobStage): string {
    switch (stage) {
      case "queuedForRip":
        return "Queued (Rip)";
      case "ripping":
        return "Ripping";
      case "queuedForEncode":
        return "Queued (Encode)";
      case "encoding":
        return "Encoding";
      case "cleaningTemp":
        return "Cleaning Temp";
      case "completed":
        return "Completed";
      case "cancelled":
        return "Cancelled";
      case "failed":
        return "Failed";
    }
  }

  function stageBadgeClass(stage: JobStage): string {
    switch (stage) {
      case "completed":
        return "border-[#1FA868] bg-[#E6FBF1] text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]";
      case "failed":
        return "border-red-200 bg-red-50 text-red-700 dark:border-red-500/40 dark:bg-red-500/15 dark:text-red-300";
      case "cancelled":
        return "border-[#E6E0D6] bg-[#FAF8F4] text-[#7D716A] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#A69990]";
      case "ripping":
        return "border-[#F8B4C0] bg-[#FFF0F2] text-[#751A2E] dark:border-[#FA637C]/45 dark:bg-[#FA637C]/15 dark:text-[#FF99AA]";
      default:
        return "border-[#9CD9F5] bg-[#E6F6FD] text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]";
    }
  }

  function formatEta(seconds: number | null): string {
    if (seconds === null || seconds < 0) return "--";
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}m ${String(secs).padStart(2, "0")}s`;
  }

  function handleDragStart(event: DragEvent, job: QueueJob): void {
    if (!isPendingStage(job.stage)) {
      event.preventDefault();
      return;
    }
    draggedJobId = job.id;
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", job.id);
    }
  }

  function handleDragOver(event: DragEvent, job: QueueJob): void {
    if (!draggedJobId || draggedJobId === job.id || !isPendingStage(job.stage)) {
      return;
    }
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "move";
    }
    dragOverJobId = job.id;
  }

  function handleDrop(event: DragEvent, targetIndex: number, targetJob: QueueJob): void {
    event.preventDefault();
    const sourceId = draggedJobId;
    draggedJobId = null;
    dragOverJobId = null;
    if (!sourceId || sourceId === targetJob.id || !isPendingStage(targetJob.stage)) {
      return;
    }
    onReorderJobToIndex(sourceId, targetIndex);
  }

  function handleDragEnd(): void {
    draggedJobId = null;
    dragOverJobId = null;
  }
</script>

<section
  aria-label="Active pipeline queue"
  class="shrink-0 border-t border-[#E6E0D6] bg-white dark:border-[#332925] dark:bg-[#1A1513]"
>
  <!-- Always-Visible 40px Live Status Ticker -->
  <div class="flex h-10 items-center justify-between gap-4 px-5 text-xs">
    <div class="flex min-w-0 items-center gap-4">
      <!-- Stage 1 Ticker -->
      <div class="flex min-w-0 items-center gap-2">
        <span
          class={`h-2 w-2 shrink-0 rounded-full ${
            activeRipJob ? "bg-[#FA637C]" : "bg-[#CFC8BC] dark:bg-[#4A3D37]"
          }`}
        ></span>
        <span class="font-semibold text-[#1A1513] dark:text-[#F5EFE8]">
          Stage 1 · Rip:
        </span>
        {#if activeRipJob}
          <span class="truncate font-mono text-[11px] text-[#544943] dark:text-[#CFC5BB]">
            {activeRipJob.titleName} ({activeRipJob.progress.percent.toFixed(1)}%)
          </span>
        {:else if paused && pendingIndices.length > 0}
          <span class="font-medium text-[#AF6C0B] dark:text-[#F8B142]">Paused</span>
        {:else}
          <span class="text-[#7D716A] dark:text-[#9E9187]">Idle</span>
        {/if}
      </div>

      <div class="h-3.5 w-px shrink-0 bg-[#E6E0D6] dark:bg-[#332925]"></div>

      <!-- Stage 2 Ticker -->
      <div class="flex min-w-0 items-center gap-2">
        <span
          class={`h-2 w-2 shrink-0 rounded-full ${
            activeEncodeJob ? "bg-[#1C8EC0] dark:bg-[#5BC0EB]" : "bg-[#CFC8BC] dark:bg-[#4A3D37]"
          }`}
        ></span>
        <span class="font-semibold text-[#1A1513] dark:text-[#F5EFE8]">
          Stage 2 · Encode:
        </span>
        {#if activeEncodeJob}
          <span class="truncate font-mono text-[11px] text-[#544943] dark:text-[#CFC5BB]">
            {activeEncodeJob.titleName} ·
            {activeEncodeJob.progress.percent.toFixed(1)}%
            {#if activeEncodeJob.progress.fps !== null}
              · {activeEncodeJob.progress.fps.toFixed(1)} fps
            {/if}
            · ETA {formatEta(activeEncodeJob.progress.etaSeconds)}
          </span>
        {:else if paused && pendingIndices.length > 0}
          <span class="font-medium text-[#AF6C0B] dark:text-[#F8B142]">
            {restoredPendingCount > 0
              ? `Restored ${restoredPendingCount} saved job${restoredPendingCount === 1 ? "" : "s"}`
              : `${pendingIndices.length} queued (paused)`}
          </span>
        {:else}
          <span class="text-[#7D716A] dark:text-[#9E9187]">Idle</span>
        {/if}
      </div>
    </div>

    <div class="flex shrink-0 items-center gap-2">
      {#if paused && pendingIndices.length > 0}
        <button
          type="button"
          onclick={onTogglePaused}
          title="Resume processing saved/queued jobs"
          class="rounded-md border border-[#1FA868] bg-[#4CDE99] px-2.5 py-1 text-[10px] font-bold text-[#1A1513] transition hover:brightness-95"
        >
          ▶ Resume Queue ({pendingIndices.length})
        </button>
      {:else if !paused && pendingIndices.length > 0}
        <button
          type="button"
          onclick={onTogglePaused}
          title="Pause starting next queued job so you can reorder priorities"
          class="rounded-md border border-[#E6E0D6] bg-white px-2 py-1 text-[10px] font-semibold text-[#61554F] transition hover:bg-[#FAF8F4] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB] dark:hover:bg-[#2B231F] dark:hover:text-[#F5EFE8]"
        >
          ⏸ Pause Queue
        </button>
      {/if}

      {#if hasFinishedJobs}
        <button
          type="button"
          onclick={onClearFinished}
          class="rounded-md border border-[#E6E0D6] bg-white px-2 py-1 text-[10px] font-semibold text-[#61554F] transition hover:bg-[#FAF8F4] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB] dark:hover:bg-[#2B231F] dark:hover:text-[#F5EFE8]"
        >
          Clear Finished
        </button>
      {/if}

      <span
        class="rounded-full border border-[#E6E0D6] bg-[#FAF8F4] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#544943] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB]"
      >
        {activeJobs.length} active · {jobs.length} total
      </span>

      <button
        type="button"
        onclick={onToggleExpanded}
        aria-expanded={expanded}
        class="rounded-md border border-[#E6E0D6] bg-[#FAF8F4] px-2.5 py-1 text-[11px] font-semibold text-[#1A1513] transition hover:bg-[#EDE7DD] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2B231F]"
      >
        {expanded ? "Hide Queue ▾" : "Show Queue ▴"}
      </button>
    </div>
  </div>

  <!-- Expanded Full-Width Queue Table -->
  {#if expanded}
    <div
      class="max-h-72 min-h-[14rem] overflow-y-auto border-t border-[#EDE7DE] bg-[#F7F4EE] dark:border-[#2B221E] dark:bg-[#120E0D]"
    >
      {#if paused && pendingIndices.length > 0}
        <div
          class="flex flex-wrap items-center justify-between gap-2 border-b border-[#E8BF6E] bg-[#FFF7E6] px-5 py-1.5 text-[11px] text-[#5E3906] dark:border-[#F8B142]/35 dark:bg-[#F8B142]/12 dark:text-[#FCD283]"
        >
          <span>
            {#if restoredPendingCount > 0}
              Restored <strong>{restoredPendingCount}</strong> queued job{restoredPendingCount === 1 ? "" : "s"} from your previous session. Drag rows or click <strong>Top</strong> to prioritize before resuming.
            {:else}
              Queue dispatch is paused. Reorder queued jobs freely, then click <strong>Resume Queue</strong>.
            {/if}
          </span>
          <button
            type="button"
            onclick={onTogglePaused}
            class="rounded border border-[#1FA868] bg-[#4CDE99] px-2 py-0.5 text-[10px] font-bold text-[#1A1513] hover:brightness-95"
          >
            ▶ Resume Now
          </button>
        </div>
      {/if}

      {#if jobs.length === 0}
        <div
          class="flex h-56 flex-col items-center justify-center px-6 text-center"
        >
          <p class="text-xs font-semibold text-[#1A1513] dark:text-[#F5EFE8]">
            No jobs in the queue yet
          </p>
          <p class="mt-1 max-w-md text-[11px] text-[#7D716A] dark:text-[#9E9187]">
            Queued titles are saved automatically across restarts and move
            independently through Stage 1 (MakeMKV), Stage 2 (HandBrakeCLI), and
            automatic temp MKV cleanup.
          </p>
        </div>
      {:else}
        <div
          class="grid grid-cols-12 border-b border-[#E6E0D6] bg-[#FAF8F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#7D716A] dark:border-[#332925] dark:bg-[#1A1513] dark:text-[#9E9187]"
        >
          <div class="col-span-3">Job</div>
          <div class="col-span-2">Stage</div>
          <div class="col-span-2">Progress</div>
          <div class="col-span-2">Speed / ETA</div>
          <div class="col-span-3 text-right">Priority &amp; Actions</div>
        </div>

        <div class="divide-y divide-[#EDE7DE] bg-white dark:divide-[#2B221E] dark:bg-[#1A1513]">
          {#each jobs as job, idx (job.id)}
            {@const logs = jobLogs[job.id] ?? []}
            {@const logsOpen = expandedLogJobIds.includes(job.id)}
            {@const isPending = isPendingStage(job.stage)}
            {@const canMoveUp = isPending && firstPendingIndex >= 0 && idx > firstPendingIndex}
            {@const canMoveDown = isPending && lastPendingIndex >= 0 && idx < lastPendingIndex}
            <div
              role="listitem"
              draggable={isPending}
              ondragstart={(e) => handleDragStart(e, job)}
              ondragover={(e) => handleDragOver(e, job)}
              ondrop={(e) => handleDrop(e, idx, job)}
              ondragend={handleDragEnd}
              class={`space-y-1.5 px-5 py-2.5 text-xs transition ${
                dragOverJobId === job.id
                  ? "bg-[#EEFCF5] ring-1 ring-inset ring-[#4CDE99] dark:bg-[#4CDE99]/10"
                  : ""
              } ${draggedJobId === job.id ? "opacity-50" : ""}`}
            >
              <div class="grid grid-cols-12 items-center gap-3">
                <div class="col-span-3 flex min-w-0 items-center gap-2">
                  {#if isPending}
                    <span
                      title="Drag to reorder queued job"
                      class="cursor-grab select-none font-mono text-xs text-[#A89C95] hover:text-[#1A1513] active:cursor-grabbing dark:text-[#6E6058] dark:hover:text-[#F5EFE8]"
                    >
                      ⋮⋮
                    </span>
                  {/if}
                  <div class="min-w-0 flex-1">
                    <p class="truncate font-semibold text-[#1A1513] dark:text-[#F5EFE8]">
                      {job.titleName}
                    </p>
                    <p
                      class="truncate font-mono text-[10px] text-[#7D716A] dark:text-[#9E9187]"
                      title={job.outputPath}
                    >
                      {job.presetName ?? "Remux MKV"} · {job.outputPath}
                    </p>
                  </div>
                </div>

                <div class="col-span-2">
                  <span
                    class={`inline-flex rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold ${stageBadgeClass(job.stage)}`}
                  >
                    {formatStageLabel(job.stage)}
                  </span>
                </div>

                <div class="col-span-2 flex items-center gap-2">
                  <div
                    class="h-1.5 flex-1 overflow-hidden rounded-full bg-[#E6E0D6] dark:bg-[#332925]"
                  >
                    <div
                      class={`h-full transition-all ${
                        job.stage === "completed"
                          ? "bg-[#4CDE99]"
                          : job.stage === "failed"
                            ? "bg-red-500"
                            : job.stage === "ripping"
                              ? "bg-[#FA637C]"
                              : "bg-[#1C8EC0] dark:bg-[#5BC0EB]"
                      }`}
                      style={`width: ${Math.min(100, Math.max(0, job.progress.percent))}%`}
                    ></div>
                  </div>
                  <span class="w-9 text-right font-mono text-[11px] text-[#1A1513] dark:text-[#F5EFE8]">
                    {job.progress.percent.toFixed(0)}%
                  </span>
                </div>

                <div class="col-span-2 truncate font-mono text-[11px] text-[#544943] dark:text-[#CFC5BB]">
                  {#if job.progress.message}
                    {job.progress.message}
                  {:else}
                    {#if job.progress.fps !== null}
                      {job.progress.fps.toFixed(1)} fps ·
                    {/if}
                    {formatEta(job.progress.etaSeconds)}
                  {/if}
                </div>

                <div class="col-span-3 flex items-center justify-end gap-1">
                  {#if isPending && pendingIndices.length > 1}
                    <div
                      class="mr-1 inline-flex items-center overflow-hidden rounded border border-[#DAD3C7] bg-[#FAF8F4] dark:border-[#3A2F2A] dark:bg-[#221C19]"
                      role="group"
                      aria-label={`Reorder ${job.titleName}`}
                    >
                      <button
                        type="button"
                        onclick={() => onMoveJob(job.id, "top")}
                        disabled={!canMoveUp}
                        title="Move to top of queue (encode next)"
                        class="px-1.5 py-0.5 font-mono text-[10px] font-bold text-[#11824F] transition hover:bg-[#E6FBF1] disabled:cursor-not-allowed disabled:opacity-30 dark:text-[#4CDE99] dark:hover:bg-[#4CDE99]/15"
                      >
                        Top
                      </button>
                      <button
                        type="button"
                        onclick={() => onMoveJob(job.id, "up")}
                        disabled={!canMoveUp}
                        title="Move up one position"
                        aria-label="Move job up"
                        class="border-l border-[#DAD3C7] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#544943] transition hover:bg-[#EDE7DD] hover:text-[#1A1513] disabled:cursor-not-allowed disabled:opacity-30 dark:border-[#3A2F2A] dark:text-[#CFC5BB] dark:hover:bg-[#2E2622] dark:hover:text-[#F5EFE8]"
                      >
                        ↑
                      </button>
                      <button
                        type="button"
                        onclick={() => onMoveJob(job.id, "down")}
                        disabled={!canMoveDown}
                        title="Move down one position"
                        aria-label="Move job down"
                        class="border-l border-[#DAD3C7] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#544943] transition hover:bg-[#EDE7DD] hover:text-[#1A1513] disabled:cursor-not-allowed disabled:opacity-30 dark:border-[#3A2F2A] dark:text-[#CFC5BB] dark:hover:bg-[#2E2622] dark:hover:text-[#F5EFE8]"
                      >
                        ↓
                      </button>
                    </div>
                  {/if}

                  <button
                    type="button"
                    onclick={() => toggleJobLogs(job.id)}
                    aria-expanded={logsOpen}
                    class={`rounded border px-1.5 py-0.5 font-mono text-[10px] font-semibold transition ${
                      logsOpen
                        ? "border-[#1A1513] bg-[#1A1513] text-white dark:border-[#4CDE99] dark:bg-[#4CDE99] dark:text-[#120E0D]"
                        : "border-[#DAD3C7] bg-[#FAF8F4] text-[#544943] hover:bg-[#EDE7DD] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB] dark:hover:bg-[#2E2622] dark:hover:text-[#F5EFE8]"
                    }`}
                  >
                    Logs{logs.length > 0 ? ` (${logs.length})` : ""}
                  </button>

                  {#if job.stage === "completed"}
                    <button
                      type="button"
                      onclick={() => onRevealPath(job.outputPath)}
                      title={`Reveal ${job.outputPath} in file manager`}
                      class="rounded border border-[#1FA868] bg-[#E6FBF1] px-2 py-0.5 text-[10px] font-semibold text-[#0E4D31] transition hover:bg-[#CEF6E3] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8] dark:hover:bg-[#4CDE99]/25"
                    >
                      Open
                    </button>
                  {:else if job.stage === "failed" || job.stage === "cancelled"}
                    <button
                      type="button"
                      onclick={() => onRetryJob(job.id)}
                      class="rounded border border-[#D68D1C] bg-[#FFF7E6] px-2 py-0.5 text-[10px] font-semibold text-[#5E3906] transition hover:bg-[#FDEBC4] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283] dark:hover:bg-[#F8B142]/25"
                    >
                      Retry
                    </button>
                  {:else}
                    <button
                      type="button"
                      onclick={() => onCancelJob(job.id)}
                      class="rounded border border-red-200 bg-red-50 px-2 py-0.5 text-[10px] font-semibold text-red-700 transition hover:bg-red-100 dark:border-red-500/40 dark:bg-red-500/15 dark:text-red-300 dark:hover:bg-red-500/25"
                    >
                      Cancel
                    </button>
                  {/if}

                  {#if isPending || job.stage === "completed" || job.stage === "failed" || job.stage === "cancelled"}
                    <button
                      type="button"
                      onclick={() => onRemoveJob(job.id)}
                      title="Remove from queue"
                      aria-label={`Remove ${job.titleName} from queue`}
                      class="rounded px-1 py-0.5 text-[10px] font-semibold text-[#9E9187] transition hover:text-red-600 dark:hover:text-red-400"
                    >
                      ✕
                    </button>
                  {/if}
                </div>
              </div>

              {#if job.error}
                <p
                  class="rounded border border-red-200 bg-red-50 px-2.5 py-1 font-mono text-[10px] text-red-700 dark:border-red-500/40 dark:bg-red-500/15 dark:text-red-300"
                >
                  {job.error}
                </p>
              {/if}

              {#if logsOpen}
                <div
                  class="overflow-hidden rounded-lg border border-[#332925] bg-[#120E0D] text-[#EAE2D8]"
                >
                  <div
                    class="flex items-center justify-between border-b border-[#332925] bg-[#1E1816] px-3 py-1 text-[10px]"
                  >
                    <span class="font-mono font-semibold text-[#B6AEA2]">
                      CLI Output · {logs.length} line{logs.length === 1 ? "" : "s"}
                    </span>
                    <div class="flex items-center gap-2">
                      {#if logs.length > 0}
                        <button
                          type="button"
                          onclick={() => void copyJobLogs(job.id)}
                          class="rounded border border-[#473A35] bg-[#29211E] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#EAE2D8] transition hover:bg-[#3A2F2B]"
                        >
                          {copiedLogJobId === job.id ? "Copied ✓" : "Copy Log"}
                        </button>
                      {/if}
                      <button
                        type="button"
                        onclick={() => toggleJobLogs(job.id)}
                        class="font-semibold text-[#9E928B] hover:text-white"
                      >
                        Hide
                      </button>
                    </div>
                  </div>
                  {#if logs.length === 0}
                    <p class="px-3 py-2 font-mono text-[10px] text-[#9E928B]">
                      Waiting for MakeMKV / HandBrakeCLI log output...
                    </p>
                  {:else}
                    <pre
                      class="max-h-36 overflow-y-auto px-3 py-2 font-mono text-[10px] leading-relaxed whitespace-pre-wrap break-all"
                    >{#each logs as line, lineIdx (`${job.id}-${lineIdx}`)}<span class={`block ${logLineClass(line)}`}>{line}</span>{/each}</pre>
                  {/if}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</section>
