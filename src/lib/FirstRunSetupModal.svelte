<script lang="ts">
  import RipDaddyLogo from "./RipDaddyLogo.svelte";
  import type {
    BinariesCheckResult,
    BinaryInstallOutcome,
    BinaryKind,
    BinarySource,
  } from "./types";

  interface Props {
    open: boolean;
    binaries: BinariesCheckResult | null;
    checkingBinaries: boolean;
    installingBinary: BinaryKind | "all" | null;
    checkError: string | null;
    setupOutcomes: BinaryInstallOutcome[];
    onAutoSetupAll: () => void;
    onInstallSingle: (kind: BinaryKind) => void;
    onBrowseBinary: (target: "makemkv" | "handbrake") => void;
    onRecheck: () => void;
    onFinish: () => void;
  }

  let {
    open,
    binaries,
    checkingBinaries,
    installingBinary,
    checkError,
    setupOutcomes,
    onAutoSetupAll,
    onInstallSingle,
    onBrowseBinary,
    onRecheck,
    onFinish,
  }: Props = $props();

  let copiedCommand = $state<string | null>(null);

  const bothReady = $derived(
    Boolean(binaries?.makemkv.found && binaries?.handbrake.found),
  );

  const autoInstallLabel = $derived.by(() => {
    if (!binaries) return null;
    const pms = new Set<string>();
    if (!binaries.makemkv.found && binaries.makemkv.installHint.canAutoInstall) {
      if (binaries.makemkv.installHint.packageManager) {
        pms.add(binaries.makemkv.installHint.packageManager);
      }
    }
    if (
      !binaries.handbrake.found &&
      binaries.handbrake.installHint.canAutoInstall
    ) {
      if (binaries.handbrake.installHint.packageManager) {
        pms.add(binaries.handbrake.installHint.packageManager);
      }
    }
    if (pms.size === 0) return null;
    return Array.from(pms).join(" / ");
  });

  function formatSource(source: BinarySource): string {
    switch (source) {
      case "customPath":
        return "Custom";
      case "systemPath":
        return "PATH";
      case "standardInstallDir":
        return "Auto";
      case "flatpak":
        return "Flatpak";
      case "notFound":
        return "Not Found";
    }
  }

  async function copyCommand(command: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(command);
      copiedCommand = command;
      setTimeout(() => {
        if (copiedCommand === command) {
          copiedCommand = null;
        }
      }, 2000);
    } catch {
      copiedCommand = null;
    }
  }
</script>

{#if open && binaries}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4 backdrop-blur-[2px] dark:bg-black/60"
  >
    <div
      role="dialog"
      aria-modal="true"
      aria-label="RipDaddy First-Run Setup"
      class="flex w-full max-w-[640px] flex-col overflow-hidden rounded-2xl border border-[#DAD3C7] bg-white shadow-[0_28px_70px_rgba(26,21,19,0.18)] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:shadow-[0_28px_70px_rgba(0,0,0,0.55)]"
    >
      <div
        class="h-[3px] w-full shrink-0 bg-gradient-to-r from-[#4CDE99] via-[#F8B142] to-[#FA637C]"
        aria-hidden="true"
      ></div>
      <!-- Header -->
      <header
        class="flex items-start justify-between gap-4 border-b border-[#E6E0D6] bg-[#FAF8F4] px-6 py-5 dark:border-[#332925] dark:bg-[#120E0D]"
      >
        <div class="flex items-start gap-3">
          <RipDaddyLogo class="mt-0.5 h-11 w-11" alt="" />
          <div>
            <div class="flex items-center gap-2">
              <h1
                class="text-base font-extrabold tracking-[-0.025em] text-[#1A1513] dark:text-[#F5EFE8]"
              >
                Welcome to RipDaddy
              </h1>
              <span
                class="rounded-full border border-[#DAD3C7] bg-white px-2 py-0.5 font-mono text-[10px] font-semibold uppercase text-[#61554F] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB]"
              >
                {binaries.platform}
              </span>
            </div>
            <p class="mt-1 text-xs leading-relaxed text-[#61554F] dark:text-[#B8ABA0]">
              RipDaddy wraps your local MakeMKV and HandBrake CLIs without
              bundling third-party binaries. Let's make sure the tools you need
              are ready.
            </p>
          </div>
        </div>
      </header>

      <!-- Body -->
      <div class="space-y-4 p-6">
        {#if checkError}
          <div
            class="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-500/40 dark:bg-red-500/15 dark:text-red-300"
          >
            {checkError}
          </div>
        {/if}

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <!-- MakeMKV Card -->
          <div
            class="flex flex-col justify-between space-y-3 rounded-xl border border-[#E6E0D6] bg-[#F7F4EE] p-4 dark:border-[#332925] dark:bg-[#221C19]"
          >
            <div class="space-y-2">
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <span
                    class={`h-2.5 w-2.5 rounded-full ${
                      binaries.makemkv.found ? "bg-[#4CDE99]" : "bg-[#F8B142]"
                    }`}
                  ></span>
                  <span class="font-mono text-xs font-bold text-[#1A1513] dark:text-[#F5EFE8]">
                    makemkvcon
                  </span>
                </div>
                <span
                  class={`rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold ${
                    binaries.makemkv.found
                      ? "border-[#1FA868] bg-[#4CDE99] text-[#1A1513]"
                      : "border-[#E6E0D6] bg-white text-[#61554F] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#CFC5BB]"
                  }`}
                >
                  {binaries.makemkv.found
                    ? (binaries.makemkv.version ?? "Ready")
                    : formatSource(binaries.makemkv.source)}
                </span>
              </div>

              <p class="text-[11px] text-[#70645D] dark:text-[#A69990]">
                Used for Stage 1 optical disc ripping (<span class="font-mono">Rip Only</span> &amp; <span class="font-mono">Rip + Encode</span>).
              </p>

              {#if binaries.makemkv.found && binaries.makemkv.path}
                <p
                  class="truncate rounded border border-[#E6E0D6] bg-white px-2 py-1 font-mono text-[10px] text-[#544943] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#CFC5BB]"
                  title={binaries.makemkv.path}
                >
                  {binaries.makemkv.path}
                </p>
              {:else if binaries.makemkv.path && binaries.makemkv.error}
                <p
                  class="rounded border border-red-200 bg-red-50 px-2 py-1 font-mono text-[10px] text-red-700 dark:border-red-500/40 dark:bg-red-500/15 dark:text-red-300"
                >
                  {binaries.makemkv.error}
                </p>
              {:else}
                <div class="flex items-center gap-1">
                  <code
                    class="flex-1 truncate rounded border border-[#E6E0D6] bg-white px-2 py-1 font-mono text-[10px] text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8]"
                    title={binaries.makemkv.installHint.command}
                  >
                    {binaries.makemkv.installHint.command}
                  </code>
                  <button
                    type="button"
                    onclick={() => {
                      if (binaries) {
                        void copyCommand(binaries.makemkv.installHint.command);
                      }
                    }}
                    class="rounded border border-[#DAD3C7] bg-white px-2 py-1 text-[10px] font-semibold text-[#1A1513] hover:bg-[#F2EDE5] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8] dark:hover:bg-[#2B231F]"
                  >
                    {copiedCommand === binaries.makemkv.installHint.command
                      ? "Copied"
                      : "Copy"}
                  </button>
                </div>
              {/if}
            </div>

            {#if !binaries.makemkv.found}
              <div class="flex items-center gap-1.5">
                {#if binaries.makemkv.installHint.canAutoInstall}
                  <button
                    type="button"
                    onclick={() => onInstallSingle("makeMkv")}
                    disabled={installingBinary !== null}
                    class="flex-1 rounded-md bg-[#11824F] px-2.5 py-1.5 text-[11px] font-semibold text-white hover:bg-[#0D6E42] disabled:opacity-50"
                  >
                    {installingBinary === "makeMkv"
                      ? "Installing..."
                      : `Install (${binaries.makemkv.installHint.packageManager})`}
                  </button>
                {/if}
                <button
                  type="button"
                  onclick={() => onBrowseBinary("makemkv")}
                  class="flex-1 rounded-md border border-[#DAD3C7] bg-white px-2.5 py-1.5 text-[11px] font-semibold text-[#1A1513] hover:bg-[#F2EDE5] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8] dark:hover:bg-[#2B231F]"
                >
                  Locate...
                </button>
              </div>
            {/if}
          </div>

          <!-- HandBrakeCLI Card -->
          <div
            class="flex flex-col justify-between space-y-3 rounded-xl border border-[#E6E0D6] bg-[#F7F4EE] p-4 dark:border-[#332925] dark:bg-[#221C19]"
          >
            <div class="space-y-2">
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <span
                    class={`h-2.5 w-2.5 rounded-full ${
                      binaries.handbrake.found ? "bg-[#4CDE99]" : "bg-[#F8B142]"
                    }`}
                  ></span>
                  <span class="font-mono text-xs font-bold text-[#1A1513] dark:text-[#F5EFE8]">
                    HandBrakeCLI
                  </span>
                </div>
                <span
                  class={`rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold ${
                    binaries.handbrake.found
                      ? "border-[#1FA868] bg-[#4CDE99] text-[#1A1513]"
                      : "border-[#E6E0D6] bg-white text-[#61554F] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#CFC5BB]"
                  }`}
                >
                  {binaries.handbrake.found
                    ? (binaries.handbrake.version ?? "Ready")
                    : formatSource(binaries.handbrake.source)}
                </span>
              </div>

              <p class="text-[11px] text-[#70645D] dark:text-[#A69990]">
                Used for Stage 2 encoding (<span class="font-mono">Encode Only</span> &amp; <span class="font-mono">Rip + Encode</span>).
              </p>

              {#if binaries.handbrake.found && binaries.handbrake.path}
                <p
                  class="truncate rounded border border-[#E6E0D6] bg-white px-2 py-1 font-mono text-[10px] text-[#544943] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#CFC5BB]"
                  title={binaries.handbrake.path}
                >
                  {binaries.handbrake.path}
                </p>
              {:else if binaries.handbrake.path && binaries.handbrake.error}
                <p
                  class="rounded border border-red-200 bg-red-50 px-2 py-1 font-mono text-[10px] text-red-700 dark:border-red-500/40 dark:bg-red-500/15 dark:text-red-300"
                >
                  {binaries.handbrake.error}
                </p>
              {:else}
                <div class="flex items-center gap-1">
                  <code
                    class="flex-1 truncate rounded border border-[#E6E0D6] bg-white px-2 py-1 font-mono text-[10px] text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8]"
                    title={binaries.handbrake.installHint.command}
                  >
                    {binaries.handbrake.installHint.command}
                  </code>
                  <button
                    type="button"
                    onclick={() => {
                      if (binaries) {
                        void copyCommand(binaries.handbrake.installHint.command);
                      }
                    }}
                    class="rounded border border-[#DAD3C7] bg-white px-2 py-1 text-[10px] font-semibold text-[#1A1513] hover:bg-[#F2EDE5] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8] dark:hover:bg-[#2B231F]"
                  >
                    {copiedCommand === binaries.handbrake.installHint.command
                      ? "Copied"
                      : "Copy"}
                  </button>
                </div>
              {/if}
            </div>

            {#if !binaries.handbrake.found}
              <div class="flex items-center gap-1.5">
                {#if binaries.handbrake.installHint.canAutoInstall}
                  <button
                    type="button"
                    onclick={() => onInstallSingle("handBrake")}
                    disabled={installingBinary !== null}
                    class="flex-1 rounded-md bg-[#11824F] px-2.5 py-1.5 text-[11px] font-semibold text-white hover:bg-[#0D6E42] disabled:opacity-50"
                  >
                    {installingBinary === "handBrake"
                      ? "Installing..."
                      : `Install (${binaries.handbrake.installHint.packageManager})`}
                  </button>
                {/if}
                <button
                  type="button"
                  onclick={() => onBrowseBinary("handbrake")}
                  class="flex-1 rounded-md border border-[#DAD3C7] bg-white px-2.5 py-1.5 text-[11px] font-semibold text-[#1A1513] hover:bg-[#F2EDE5] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8] dark:hover:bg-[#2B231F]"
                >
                  Locate...
                </button>
              </div>
            {/if}
          </div>
        </div>

        {#if setupOutcomes.length > 0}
          <div
            class="space-y-1 rounded-lg border border-[#E6E0D6] bg-[#FAF8F4] p-3 text-[11px] dark:border-[#332925] dark:bg-[#221C19]"
          >
            {#each setupOutcomes as outcome (outcome.binary)}
              <p
                class={outcome.succeeded
                  ? "font-medium text-emerald-700 dark:text-[#7AF0B8]"
                  : "text-amber-800 dark:text-[#FCD283]"}
              >
                {outcome.message}
              </p>
            {/each}
          </div>
        {/if}

        <div
          class="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[#E6E0D6] bg-[#FAF8F4] px-3.5 py-2.5 text-[11px] text-[#61554F] dark:border-[#332925] dark:bg-[#221C19] dark:text-[#B8ABA0]"
        >
          <span>
            Output folder auto-configured to
            <span class="font-mono font-semibold text-[#1A1513] dark:text-[#F5EFE8]">
              {binaries.defaultPaths.outputDir}
            </span>
          </span>
          <span class="font-mono text-[10px] text-[#7D716A] dark:text-[#9E9187]">
            Change anytime in Settings
          </span>
        </div>
      </div>

      <!-- Footer Actions -->
      <footer
        class="flex flex-wrap items-center justify-between gap-3 border-t border-[#E6E0D6] bg-[#FAF8F4] px-6 py-4 dark:border-[#332925] dark:bg-[#120E0D]"
      >
        <div class="flex items-center gap-2">
          <button
            type="button"
            onclick={onRecheck}
            disabled={checkingBinaries || installingBinary !== null}
            class="rounded-md border border-[#DAD3C7] bg-white px-3 py-1.5 text-xs font-semibold text-[#1A1513] hover:bg-[#EDE7DD] disabled:opacity-50 dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2E2622]"
          >
            {checkingBinaries ? "Checking..." : "Re-check"}
          </button>

          {#if !bothReady}
            <button
              type="button"
              onclick={onFinish}
              disabled={installingBinary !== null}
              class="px-2 py-1.5 text-xs font-medium text-[#70645D] hover:text-[#1A1513] disabled:opacity-50 dark:text-[#9E9187] dark:hover:text-[#F5EFE8]"
            >
              Skip for now
            </button>
          {/if}
        </div>

        {#if bothReady}
          <button
            type="button"
            onclick={onFinish}
            class="rounded-md border border-[#1FA868] bg-[#4CDE99] px-4 py-2 text-xs font-bold text-[#1A1513] shadow-[0_1px_2px_rgba(0,0,0,0.06)] transition hover:brightness-95"
          >
            All Set — Open RipDaddy →
          </button>
        {:else if autoInstallLabel}
          <button
            type="button"
            onclick={onAutoSetupAll}
            disabled={installingBinary !== null}
            class="rounded-md bg-[#11824F] px-4 py-2 text-xs font-semibold text-white shadow-[0_1px_2px_rgba(17,130,79,0.22)] transition hover:bg-[#0D6E42] disabled:opacity-50"
          >
            {installingBinary === "all"
              ? `Installing via ${autoInstallLabel}...`
              : `Auto-Install Missing CLIs (${autoInstallLabel}) →`}
          </button>
        {:else}
          <button
            type="button"
            onclick={onFinish}
            class="rounded-md bg-[#11824F] px-4 py-2 text-xs font-semibold text-white shadow-[0_1px_2px_rgba(17,130,79,0.22)] transition hover:bg-[#0D6E42]"
          >
            Continue to RipDaddy →
          </button>
        {/if}
      </footer>
    </div>
  </div>
{/if}
