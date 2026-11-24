<script lang="ts">
  import type { JobStage, QueueJob } from "./types";

  interface Props {
    expanded: boolean;
    jobs: QueueJob[];
    onToggleExpanded: () => void;
    onCancelJob: (jobId: string) => void;
    onClearFinished: () => void;
  }

  let {
    expanded,
    jobs,
    onToggleExpanded,
    onCancelJob,
    onClearFinished,
  }: Props = $props();

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
        return "border-[#24B465] bg-[#E3F9EC] text-[#17462B]";
      case "failed":
        return "border-red-200 bg-red-50 text-red-700";
      case "cancelled":
        return "border-[#E5E3DD] bg-[#F8F7F4] text-[#787772]";
      case "ripping":
        return "border-[#F1B6E2] bg-[#FFE8F8] text-[#4A2040]";
      default:
        return "border-[#B5DAFA] bg-[#DCEEFF] text-[#155CA2]";
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
  class="shrink-0 border-t border-[#E5E3DD] bg-white"
>
  <!-- Always-Visible 40px Live Status Ticker -->
  <div class="flex h-10 items-center justify-between gap-4 px-5 text-xs">
    <div class="flex min-w-0 items-center gap-4">
      <!-- Stage 1 Ticker -->
      <div class="flex min-w-0 items-center gap-2">
        <span
          class={`h-2 w-2 shrink-0 rounded-full ${
            activeRipJob ? "bg-[#3AD27E]" : "bg-[#D1CFC8]"
          }`}
        ></span>
        <span class="font-semibold text-[#111111]">Stage 1 · Rip:</span>
        {#if activeRipJob}
          <span class="truncate font-mono text-[11px] text-[#52524E]">
            {activeRipJob.titleName} ({activeRipJob.progress.percent.toFixed(1)}%)
          </span>
        {:else}
          <span class="text-[#787772]">Idle</span>
        {/if}
      </div>

      <div class="h-3.5 w-px shrink-0 bg-[#E5E3DD]"></div>

      <!-- Stage 2 Ticker -->
      <div class="flex min-w-0 items-center gap-2">
        <span
          class={`h-2 w-2 shrink-0 rounded-full ${
            activeEncodeJob ? "bg-[#1D8BF8]" : "bg-[#D1CFC8]"
          }`}
        ></span>
        <span class="font-semibold text-[#111111]">Stage 2 · Encode:</span>
        {#if activeEncodeJob}
          <span class="truncate font-mono text-[11px] text-[#52524E]">
            {activeEncodeJob.titleName} ·
            {activeEncodeJob.progress.percent.toFixed(1)}%
            {#if activeEncodeJob.progress.fps !== null}
              · {activeEncodeJob.progress.fps.toFixed(1)} fps
            {/if}
            · ETA {formatEta(activeEncodeJob.progress.etaSeconds)}
          </span>
        {:else}
          <span class="text-[#787772]">Idle</span>
        {/if}
      </div>
    </div>

    <div class="flex shrink-0 items-center gap-2">
      {#if hasFinishedJobs}
        <button
          type="button"
          onclick={onClearFinished}
          class="rounded-md border border-[#E5E3DD] bg-white px-2 py-1 text-[10px] font-semibold text-[#5C5B57] transition hover:bg-[#F8F7F4] hover:text-[#111111]"
        >
          Clear Finished
        </button>
      {/if}

      <span
        class="rounded-full border border-[#E5E3DD] bg-[#F8F7F4] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#52524E]"
      >
        {activeJobs.length} active · {jobs.length} total
      </span>

      <button
        type="button"
        onclick={onToggleExpanded}
        aria-expanded={expanded}
        class="rounded-md border border-[#E5E3DD] bg-[#F8F7F4] px-2.5 py-1 text-[11px] font-semibold text-[#111111] transition hover:bg-[#EFECE6]"
      >
        {expanded ? "Hide Queue ▾" : "Show Queue ▴"}
      </button>
    </div>
  </div>

  <!-- Expanded Full-Width Queue Table -->
  {#if expanded}
    <div class="h-56 overflow-y-auto border-t border-[#ECEAE4] bg-[#FAF9F6]">
      {#if jobs.length === 0}
        <div
          class="flex h-full flex-col items-center justify-center px-6 text-center"
        >
          <p class="text-xs font-semibold text-[#111111]">
            No jobs in the queue yet
          </p>
          <p class="mt-1 max-w-md text-[11px] text-[#787772]">
            Queued titles move independently through Stage 1 (MakeMKV), Stage 2
            (HandBrakeCLI), and automatic temp MKV cleanup.
          </p>
        </div>
      {:else}
        <div
          class="grid grid-cols-12 border-b border-[#E5E3DD] bg-[#F8F7F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
        >
          <div class="col-span-3">Job</div>
          <div class="col-span-2">Stage</div>
          <div class="col-span-4">Progress</div>
          <div class="col-span-2">Speed / ETA</div>
          <div class="col-span-1 text-right">Action</div>
        </div>

        <div class="divide-y divide-[#ECEAE4] bg-white">
          {#each jobs as job (job.id)}
            <div class="space-y-1 px-5 py-2.5 text-xs">
              <div class="grid grid-cols-12 items-center gap-3">
                <div class="col-span-3 min-w-0">
                  <p class="truncate font-semibold text-[#111111]">
                    {job.titleName}
                  </p>
                  <p
                    class="truncate font-mono text-[10px] text-[#787772]"
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

                <div class="col-span-4 flex items-center gap-2.5">
                  <div
                    class="h-1.5 flex-1 overflow-hidden rounded-full bg-[#E5E3DD]"
                  >
                    <div
                      class={`h-full transition-all ${
                        job.stage === "completed"
                          ? "bg-[#3AD27E]"
                          : job.stage === "failed"
                            ? "bg-red-500"
                            : "bg-[#1D8BF8]"
                      }`}
                      style={`width: ${Math.min(100, Math.max(0, job.progress.percent))}%`}
                    ></div>
                  </div>
                  <span class="w-11 text-right font-mono text-[11px] text-[#111111]">
                    {job.progress.percent.toFixed(0)}%
                  </span>
                </div>

                <div class="col-span-2 font-mono text-[11px] text-[#52524E]">
                  {#if job.progress.message}
                    {job.progress.message}
                  {:else}
                    {#if job.progress.fps !== null}
                      {job.progress.fps.toFixed(1)} fps ·
                    {/if}
                    {formatEta(job.progress.etaSeconds)}
                  {/if}
                </div>

                <div class="col-span-1 text-right">
                  {#if job.stage !== "completed" && job.stage !== "cancelled" && job.stage !== "failed"}
                    <button
                      type="button"
                      onclick={() => onCancelJob(job.id)}
                      class="text-[11px] font-semibold text-red-600 hover:text-red-700"
                    >
                      Cancel
                    </button>
                  {:else}
                    <span class="text-[10px] text-[#9A9891]">—</span>
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
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</section>
