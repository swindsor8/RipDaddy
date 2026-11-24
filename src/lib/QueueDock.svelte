<script lang="ts">
  import type { JobStage, QueueJob } from "./types";

  interface Props {
    expanded: boolean;
    jobs: QueueJob[];
    jobLogs: Record<string, string[]>;
    onToggleExpanded: () => void;
    onCancelJob: (jobId: string) => void;
    onRetryJob: (jobId: string) => void;
    onRevealPath: (path: string) => void;
    onClearFinished: () => void;
  }

  let {
    expanded,
    jobs,
    jobLogs,
    onToggleExpanded,
    onCancelJob,
    onRetryJob,
    onRevealPath,
    onClearFinished,
  }: Props = $props();

  let expandedLogJobIds = $state<string[]>([]);
  let copiedLogJobId = $state<string | null>(null);

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
        return "border-[#1FA868] bg-[#E6FBF1] text-[#0E4D31]";
      case "failed":
        return "border-red-200 bg-red-50 text-red-700";
      case "cancelled":
        return "border-[#E6E0D6] bg-[#FAF8F4] text-[#7D716A]";
      case "ripping":
        return "border-[#F8B4C0] bg-[#FFF0F2] text-[#751A2E]";
      default:
        return "border-[#9CD9F5] bg-[#E6F6FD] text-[#0E5678]";
    }
  }

  function formatEta(seconds: number | null): string {
    if (seconds === null || seconds < 0) return "--";
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}m ${String(secs).padStart(2, "0")}s`;
  }
</script>

<section
  aria-label="Active pipeline queue"
  class="shrink-0 border-t border-[#E6E0D6] bg-white"
>
  <!-- Always-Visible 40px Live Status Ticker -->
  <div class="flex h-10 items-center justify-between gap-4 px-5 text-xs">
    <div class="flex min-w-0 items-center gap-4">
      <!-- Stage 1 Ticker -->
      <div class="flex min-w-0 items-center gap-2">
        <span
          class={`h-2 w-2 shrink-0 rounded-full ${
            activeRipJob ? "bg-[#FA637C]" : "bg-[#CFC8BC]"
          }`}
        ></span>
        <span class="font-semibold text-[#1A1513]">Stage 1 · Rip:</span>
        {#if activeRipJob}
          <span class="truncate font-mono text-[11px] text-[#544943]">
            {activeRipJob.titleName} ({activeRipJob.progress.percent.toFixed(1)}%)
          </span>
        {:else}
          <span class="text-[#7D716A]">Idle</span>
        {/if}
      </div>

      <div class="h-3.5 w-px shrink-0 bg-[#E6E0D6]"></div>

      <!-- Stage 2 Ticker -->
      <div class="flex min-w-0 items-center gap-2">
        <span
          class={`h-2 w-2 shrink-0 rounded-full ${
            activeEncodeJob ? "bg-[#1C8EC0]" : "bg-[#CFC8BC]"
          }`}
        ></span>
        <span class="font-semibold text-[#1A1513]">Stage 2 · Encode:</span>
        {#if activeEncodeJob}
          <span class="truncate font-mono text-[11px] text-[#544943]">
            {activeEncodeJob.titleName} ·
            {activeEncodeJob.progress.percent.toFixed(1)}%
            {#if activeEncodeJob.progress.fps !== null}
              · {activeEncodeJob.progress.fps.toFixed(1)} fps
            {/if}
            · ETA {formatEta(activeEncodeJob.progress.etaSeconds)}
          </span>
        {:else}
          <span class="text-[#7D716A]">Idle</span>
        {/if}
      </div>
    </div>

    <div class="flex shrink-0 items-center gap-2">
      {#if hasFinishedJobs}
        <button
          type="button"
          onclick={onClearFinished}
          class="rounded-md border border-[#E6E0D6] bg-white px-2 py-1 text-[10px] font-semibold text-[#61554F] transition hover:bg-[#FAF8F4] hover:text-[#1A1513]"
        >
          Clear Finished
        </button>
      {/if}

      <span
        class="rounded-full border border-[#E6E0D6] bg-[#FAF8F4] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#544943]"
      >
        {activeJobs.length} active · {jobs.length} total
      </span>

      <button
        type="button"
        onclick={onToggleExpanded}
        aria-expanded={expanded}
        class="rounded-md border border-[#E6E0D6] bg-[#FAF8F4] px-2.5 py-1 text-[11px] font-semibold text-[#1A1513] transition hover:bg-[#EDE7DD]"
      >
        {expanded ? "Hide Queue ▾" : "Show Queue ▴"}
      </button>
    </div>
  </div>

  <!-- Expanded Full-Width Queue Table -->
  {#if expanded}
    <div class="max-h-72 min-h-[14rem] overflow-y-auto border-t border-[#EDE7DE] bg-[#F7F4EE]">
      {#if jobs.length === 0}
        <div
          class="flex h-56 flex-col items-center justify-center px-6 text-center"
        >
          <p class="text-xs font-semibold text-[#1A1513]">
            No jobs in the queue yet
          </p>
          <p class="mt-1 max-w-md text-[11px] text-[#7D716A]">
            Queued titles move independently through Stage 1 (MakeMKV), Stage 2
            (HandBrakeCLI), and automatic temp MKV cleanup.
          </p>
        </div>
      {:else}
        <div
          class="grid grid-cols-12 border-b border-[#E6E0D6] bg-[#FAF8F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#7D716A]"
        >
          <div class="col-span-3">Job</div>
          <div class="col-span-2">Stage</div>
          <div class="col-span-3">Progress</div>
          <div class="col-span-2">Speed / ETA</div>
          <div class="col-span-2 text-right">Actions</div>
        </div>

        <div class="divide-y divide-[#EDE7DE] bg-white">
          {#each jobs as job (job.id)}
            {@const logs = jobLogs[job.id] ?? []}
            {@const logsOpen = expandedLogJobIds.includes(job.id)}
            <div class="space-y-1.5 px-5 py-2.5 text-xs">
              <div class="grid grid-cols-12 items-center gap-3">
                <div class="col-span-3 min-w-0">
                  <p class="truncate font-semibold text-[#1A1513]">
                    {job.titleName}
                  </p>
                  <p
                    class="truncate font-mono text-[10px] text-[#7D716A]"
                    title={job.outputPath}
                  >
                    {job.presetName ?? "Remux MKV"} · {job.outputPath}
                  </p>
                </div>

                <div class="col-span-2">
                  <span
                    class={`inline-flex rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold ${stageBadgeClass(job.stage)}`}
                  >
                    {formatStageLabel(job.stage)}
                  </span>
                </div>

                <div class="col-span-3 flex items-center gap-2.5">
                  <div
                    class="h-1.5 flex-1 overflow-hidden rounded-full bg-[#E6E0D6]"
                  >
                    <div
                      class={`h-full transition-all ${
                        job.stage === "completed"
                          ? "bg-[#4CDE99]"
                          : job.stage === "failed"
                            ? "bg-red-500"
                            : job.stage === "ripping"
                              ? "bg-[#FA637C]"
                              : "bg-[#1C8EC0]"
                      }`}
                      style={`width: ${Math.min(100, Math.max(0, job.progress.percent))}%`}
                    ></div>
                  </div>
                  <span class="w-10 text-right font-mono text-[11px] text-[#1A1513]">
                    {job.progress.percent.toFixed(0)}%
                  </span>
                </div>

                <div class="col-span-2 font-mono text-[11px] text-[#544943]">
                  {#if job.progress.message}
                    {job.progress.message}
                  {:else}
                    {#if job.progress.fps !== null}
                      {job.progress.fps.toFixed(1)} fps ·
                    {/if}
                    {formatEta(job.progress.etaSeconds)}
                  {/if}
                </div>

                <div class="col-span-2 flex items-center justify-end gap-1.5">
                  <button
                    type="button"
                    onclick={() => toggleJobLogs(job.id)}
                    aria-expanded={logsOpen}
                    class={`rounded border px-1.5 py-0.5 font-mono text-[10px] font-semibold transition ${
                      logsOpen
                        ? "border-[#1A1513] bg-[#1A1513] text-white"
                        : "border-[#DAD3C7] bg-[#FAF8F4] text-[#544943] hover:bg-[#EDE7DD] hover:text-[#1A1513]"
                    }`}
                  >
                    Logs{logs.length > 0 ? ` (${logs.length})` : ""}
                  </button>

                  {#if job.stage === "completed"}
                    <button
                      type="button"
                      onclick={() => onRevealPath(job.outputPath)}
                      title={`Reveal ${job.outputPath} in file manager`}
                      class="rounded border border-[#1FA868] bg-[#E6FBF1] px-2 py-0.5 text-[10px] font-semibold text-[#0E4D31] transition hover:bg-[#CEF6E3]"
                    >
                      Open
                    </button>
                  {:else if job.stage === "failed" || job.stage === "cancelled"}
                    <button
                      type="button"
                      onclick={() => onRetryJob(job.id)}
                      class="rounded border border-[#D68D1C] bg-[#FFF7E6] px-2 py-0.5 text-[10px] font-semibold text-[#5E3906] transition hover:bg-[#FDEBC4]"
                    >
                      Retry
                    </button>
                  {:else}
                    <button
                      type="button"
                      onclick={() => onCancelJob(job.id)}
                      class="rounded border border-red-200 bg-red-50 px-2 py-0.5 text-[10px] font-semibold text-red-700 transition hover:bg-red-100"
                    >
                      Cancel
                    </button>
                  {/if}
                </div>
              </div>

              {#if job.error}
                <p
                  class="rounded border border-red-200 bg-red-50 px-2.5 py-1 font-mono text-[10px] text-red-700"
                >
                  {job.error}
                </p>
              {/if}

              {#if logsOpen}
                <div
                  class="overflow-hidden rounded-lg border border-[#332925] bg-[#181311] text-[#EAE2D8]"
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
