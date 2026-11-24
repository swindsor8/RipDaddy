<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import RipDaddyLogo from "./RipDaddyLogo.svelte";
  import {
    AUDIO_BITRATES,
    ENCODER_PRESETS,
    RESOLUTION_TIERS,
    VIDEO_ENCODERS,
    isValidAudioMode,
    isValidEncoder,
    isValidSubtitleMode,
    qualityProfileKey,
    rfQualityTierLabel,
    shortEncoderLabel,
  } from "./qualityDefaults";
  import type {
    AppSettings,
    BinariesCheckResult,
    BinaryInstallOutcome,
    BinaryKind,
    BinarySource,
    BinaryStatus,
    DeliveryTarget,
    QualityProfile,
    ResolutionTier,
    SettingsCategory,
  } from "./types";

  interface Props {
    open: boolean;
    activeCategory: SettingsCategory;
    binaries: BinariesCheckResult | null;
    checkingBinaries: boolean;
    installingBinary: BinaryKind | "all" | null;
    checkError: string | null;
    setupOutcomes: BinaryInstallOutcome[];
    settings: AppSettings;
    onClose: () => void;
    onSelectCategory: (category: SettingsCategory) => void;
    onUpdateSetting: <K extends keyof AppSettings>(
      key: K,
      value: AppSettings[K],
    ) => void;
    onUpdateQualityProfile: (profile: QualityProfile) => void;
    onResetQualityProfiles: (onlyKey?: string) => void;
    onOpenSetupWizard: () => void;
    onRecheckBinaries: () => void;
    onAutoSetupAll: () => void;
    onInstallSingle: (kind: BinaryKind) => void;
    onBrowseBinary: (target: "makemkv" | "handbrake") => void;
    onResetBinaryPath: (target: "makemkv" | "handbrake") => void;
    onBrowseDirectory: (target: "tempDir" | "outputDir") => void;
    onResetDirectory: (target: "tempDir" | "outputDir") => void;
    onBrowseCustomPreset: () => void;
  }

  let {
    open,
    activeCategory,
    binaries,
    checkingBinaries,
    installingBinary,
    checkError,
    setupOutcomes,
    settings,
    onClose,
    onSelectCategory,
    onUpdateSetting,
    onUpdateQualityProfile,
    onResetQualityProfiles,
    onOpenSetupWizard,
    onRecheckBinaries,
    onAutoSetupAll,
    onInstallSingle,
    onBrowseBinary,
    onResetBinaryPath,
    onBrowseDirectory,
    onResetDirectory,
    onBrowseCustomPreset,
  }: Props = $props();

  let copiedCommand = $state<string | null>(null);
  let showApiKeys = $state(false);
  let editResolution = $state<ResolutionTier>("p1080");
  let editDelivery = $state<DeliveryTarget>("local");
  let cliFlagsPreview = $state<string>("");

  const categories: {
    id: SettingsCategory;
    label: string;
    desc: string;
    dot: string;
  }[] = [
    {
      id: "general",
      label: "General",
      desc: "Workflow & behavior",
      dot: "bg-[#111111]",
    },
    {
      id: "binaries",
      label: "External CLIs",
      desc: "MakeMKV & HandBrake",
      dot: "bg-[#3AD27E]",
    },
    {
      id: "storage",
      label: "Storage & Paths",
      desc: "Output & temp dirs",
      dot: "bg-[#F6BE4F]",
    },
    {
      id: "encoding",
      label: "Encoding & Ripping",
      desc: "Presets & filters",
      dot: "bg-[#FF6AD5]",
    },
    {
      id: "metadata",
      label: "Metadata & Naming",
      desc: "IMDb / TVDB / TMDB",
      dot: "bg-[#58B0F9]",
    },
  ];

  const canAutoSetupAnyMissing = $derived.by(() => {
    if (!binaries) return false;
    return (
      (!binaries.makemkv.found && binaries.makemkv.installHint.canAutoInstall) ||
      (!binaries.handbrake.found && binaries.handbrake.installHint.canAutoInstall)
    );
  });

  const missingCliCount = $derived.by(() => {
    if (!binaries) return 0;
    return (binaries.makemkv.found ? 0 : 1) + (binaries.handbrake.found ? 0 : 1);
  });

  const editingProfile = $derived.by(() => {
    const key = qualityProfileKey(editResolution, editDelivery);
    return settings.qualityProfiles[key] ?? null;
  });

  // Sync editor selection to the user's active toolbar tier when opening Encoding tab
  $effect(() => {
    if (open && activeCategory === "encoding") {
      editResolution = settings.activeResolution;
      editDelivery = settings.activeDelivery;
    }
  });

  // Live HandBrakeCLI flag preview from Rust's handbrake::build_quality_flags
  $effect(() => {
    const profile = editingProfile;
    if (!open || activeCategory !== "encoding" || !profile) return;
    let cancelled = false;
    void invoke<string[]>("preview_quality_cli_args", { profile })
      .then((flags) => {
        if (!cancelled) {
          cliFlagsPreview = `HandBrakeCLI --json -i <in> -o <out> ${flags.join(" ")}`;
        }
      })
      .catch(() => {
        if (!cancelled) {
          cliFlagsPreview = `HandBrakeCLI --encoder ${profile.encoder} --quality ${profile.rfQuality.toFixed(1)}`;
        }
      });
    return () => {
      cancelled = true;
    };
  });

  function patchEditingProfile(partial: Partial<QualityProfile>): void {
    if (!editingProfile) return;
    onUpdateQualityProfile({ ...editingProfile, ...partial });
  }

  function handleEncoderChange(event: Event): void {
    const val = (event.currentTarget as HTMLSelectElement).value;
    if (isValidEncoder(val)) {
      patchEditingProfile({ encoder: val });
    }
  }

  function handleAudioModeChange(event: Event): void {
    const val = (event.currentTarget as HTMLSelectElement).value;
    if (isValidAudioMode(val)) {
      patchEditingProfile({ audioMode: val });
    }
  }

  function handleSubtitleModeChange(event: Event): void {
    const val = (event.currentTarget as HTMLSelectElement).value;
    if (isValidSubtitleMode(val)) {
      patchEditingProfile({ subtitleMode: val });
    }
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (open && event.key === "Escape") {
      onClose();
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

  function formatSourceLabel(source: BinarySource): string {
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
        return "Missing";
    }
  }

  function statusDotClass(status: BinaryStatus | undefined): string {
    if (!status) return "bg-[#B8B6AF]";
    return status.found ? "bg-[#22B567]" : "bg-[#E59819]";
  }

  function handleDefaultModeChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (
      value === "ripOnly" ||
      value === "encodeOnly" ||
      value === "ripAndEncode"
    ) {
      onUpdateSetting("defaultMode", value);
    }
  }

  function handleNamingChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (value === "plex" || value === "jellyfin" || value === "plain") {
      onUpdateSetting("namingConvention", value);
    }
  }

  function handleProviderChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (
      value === "imdb" ||
      value === "tvdb" ||
      value === "tmdb" ||
      value === "tvmaze"
    ) {
      onUpdateSetting("defaultMetadataProvider", value);
    }
  }

  function handleFolderOrgChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (
      value === "ask" ||
      value === "showAndSeason" ||
      value === "seasonOnly" ||
      value === "flat"
    ) {
      onUpdateSetting("tvFolderOrganization", value);
    }
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />

{#if open}
  <!-- Soft translucent backdrop for click-outside dismissal -->
  <button
    type="button"
    onclick={onClose}
    aria-label="Close settings menu"
    class="fixed inset-0 z-40 cursor-default bg-black/15 backdrop-blur-[1px]"
  ></button>

  <!-- Categorized Settings Dropdown (Clay Editorial Style) -->
  <div
    role="dialog"
    aria-label="RipDaddy Settings"
    class="fixed top-16 left-1/2 z-50 flex h-[548px] max-h-[calc(100vh-5rem)] w-[700px] max-w-[calc(100vw-2rem)] -translate-x-1/2 overflow-hidden rounded-xl border border-[#DFDDD6] bg-white shadow-[0_24px_60px_rgba(17,17,17,0.14)]"
  >
    <!-- Category Sidebar -->
    <nav
      aria-label="Settings categories"
      class="flex w-52 shrink-0 flex-col border-r border-[#E5E3DD] bg-[#F8F7F4] p-3"
    >
      <div class="mb-2.5 flex items-center justify-between px-2 py-1">
        <span
          class="text-[10px] font-bold uppercase tracking-[0.1em] text-[#787772]"
        >
          Settings
        </span>
        {#if binaries?.platform}
          <span
            class="rounded-full border border-[#E5E3DD] bg-white px-2 py-0.5 font-mono text-[10px] font-medium uppercase text-[#5C5B57]"
          >
            {binaries.platform}
          </span>
        {/if}
      </div>

      <div class="flex-1 space-y-1">
        {#each categories as cat (cat.id)}
          <button
            type="button"
            onclick={() => onSelectCategory(cat.id)}
            class={`flex w-full items-center justify-between rounded-lg px-2.5 py-2 text-left transition ${
              activeCategory === cat.id
                ? "border border-[#E5E3DD] bg-white text-[#111111] shadow-[0_1px_2px_rgba(0,0,0,0.04)]"
                : "border border-transparent text-[#52524E] hover:bg-white/70 hover:text-[#111111]"
            }`}
          >
            <div class="flex min-w-0 items-center gap-2.5">
              <span class={`h-2 w-2 shrink-0 rounded-full ${cat.dot}`}></span>
              <div class="min-w-0">
                <div class="truncate text-xs font-semibold">{cat.label}</div>
                <div class="truncate text-[10px] text-[#787772]">{cat.desc}</div>
              </div>
            </div>
            {#if cat.id === "binaries" && missingCliCount > 0}
              <span
                class="ml-1.5 shrink-0 rounded-full border border-[#DDA12D] bg-[#F6BE4F] px-1.5 py-0.5 font-mono text-[10px] font-bold text-[#111111]"
              >
                {missingCliCount}
              </span>
            {/if}
          </button>
        {/each}
      </div>

      <div
        class="mt-2 border-t border-[#E5E3DD] px-2 pt-2.5 text-[10px] leading-relaxed text-[#787772]"
      >
        Auto-saved locally to <span class="font-mono text-[#52524E]">settings.json</span>.
      </div>
    </nav>

    <!-- Category Content Pane -->
    <div class="flex min-w-0 flex-1 flex-col bg-white">
      <header
        class="flex items-center justify-between border-b border-[#E5E3DD] px-5 py-3.5"
      >
        <div>
          <h2 class="text-sm font-bold tracking-[-0.02em] text-[#111111]">
            {categories.find((c) => c.id === activeCategory)?.label}
          </h2>
          <p class="text-[11px] text-[#787772]">
            {categories.find((c) => c.id === activeCategory)?.desc}
          </p>
        </div>
        <button
          type="button"
          onclick={onClose}
          class="rounded-md bg-[#111111] px-3 py-1.5 text-xs font-medium text-white transition hover:bg-[#2A2A2A]"
        >
          Done
        </button>
      </header>

      <div class="flex-1 space-y-4 overflow-y-auto p-5">
        {#if activeCategory === "general"}
          <div class="space-y-4">
            <div class="space-y-1.5">
              <label
                for="setting-default-mode"
                class="block text-xs font-semibold text-[#111111]"
              >
                Default Workflow Mode
              </label>
              <select
                id="setting-default-mode"
                value={settings.defaultMode}
                onchange={handleDefaultModeChange}
                class="w-full rounded-lg border border-[#E5E3DD] bg-[#F8F7F4] px-3 py-2 text-xs font-medium text-[#111111] outline-none focus:border-[#111111] focus:bg-white"
              >
                <option value="ripAndEncode">Rip + Encode (Two-Stage Pipeline)</option>
                <option value="ripOnly">Rip Only (MakeMKV Remux)</option>
                <option value="encodeOnly">Encode Only (Local Video Files)</option>
              </select>
            </div>

            <div class="divide-y divide-[#ECEAE4] rounded-xl border border-[#E5E3DD] bg-[#FAF9F6]">
              <label
                for="setting-auto-setup"
                class="flex cursor-pointer items-start justify-between gap-3 p-3.5"
              >
                <div>
                  <div class="text-xs font-semibold text-[#111111]">
                    Auto-setup missing CLIs on first run
                  </div>
                  <div class="mt-0.5 text-[11px] text-[#6B6A65]">
                    Delegates missing CLI installs to your OS package manager
                    (winget/brew) on first launch.
                  </div>
                </div>
                <input
                  id="setting-auto-setup"
                  type="checkbox"
                  checked={settings.autoSetupOnFirstRun}
                  onchange={(e) =>
                    onUpdateSetting(
                      "autoSetupOnFirstRun",
                      e.currentTarget.checked,
                    )}
                  class="mt-0.5 h-4 w-4 accent-[#111111]"
                />
              </label>

              <label
                for="setting-keep-temp"
                class="flex cursor-pointer items-start justify-between gap-3 p-3.5"
              >
                <div>
                  <div class="text-xs font-semibold text-[#111111]">
                    Keep intermediate temp MKV files
                  </div>
                  <div class="mt-0.5 text-[11px] text-[#6B6A65]">
                    Skip Stage 3 temp cleanup after HandBrakeCLI finishes encoding.
                  </div>
                </div>
                <input
                  id="setting-keep-temp"
                  type="checkbox"
                  checked={settings.keepTempMkv}
                  onchange={(e) =>
                    onUpdateSetting("keepTempMkv", e.currentTarget.checked)}
                  class="mt-0.5 h-4 w-4 accent-[#111111]"
                />
              </label>

              <label
                for="setting-auto-eject"
                class="flex cursor-pointer items-start justify-between gap-3 p-3.5"
              >
                <div>
                  <div class="text-xs font-semibold text-[#111111]">
                    Auto-eject optical disc when ripping completes
                  </div>
                  <div class="mt-0.5 text-[11px] text-[#6B6A65]">
                    Ejects the drive after Stage 1 so you can load the next disc
                    while Stage 2 encodes.
                  </div>
                </div>
                <input
                  id="setting-auto-eject"
                  type="checkbox"
                  checked={settings.autoEjectDisc}
                  onchange={(e) =>
                    onUpdateSetting("autoEjectDisc", e.currentTarget.checked)}
                  class="mt-0.5 h-4 w-4 accent-[#111111]"
                />
              </label>
            </div>

            <div
              class="flex items-center justify-between gap-3 rounded-xl border border-[#E5E3DD] bg-[#F8F7F4] p-3.5 text-[11px] leading-relaxed text-[#5C5B57]"
            >
              <div class="flex items-center gap-3">
                <RipDaddyLogo class="h-8 w-8" alt="" />
                <span>
                  RipDaddy bundles zero third-party binaries or decryption code.
                  All spawned child processes terminate cleanly on cancel or exit.
                </span>
              </div>
              <button
                type="button"
                onclick={() => {
                  onClose();
                  onOpenSetupWizard();
                }}
                class="shrink-0 rounded-md border border-[#DFDDD6] bg-white px-2.5 py-1.5 text-[11px] font-semibold text-[#111111] hover:bg-[#EFECE6]"
              >
                Setup Wizard
              </button>
            </div>
          </div>

        {:else if activeCategory === "binaries"}
          <div class="space-y-3">
            <div class="flex items-center justify-between gap-2">
              <div class="text-[11px] text-[#6B6A65]">
                {#if binaries?.availablePackageManagers.length}
                  Package managers:
                  <span class="font-mono font-medium text-[#111111]">
                    {binaries.availablePackageManagers.join(", ")}
                  </span>
                {:else}
                  Searches PATH and standard OS installation locations.
                {/if}
              </div>

              <button
                type="button"
                onclick={onRecheckBinaries}
                disabled={checkingBinaries || installingBinary !== null}
                class="shrink-0 rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1 text-[11px] font-semibold text-[#111111] transition hover:bg-[#F8F7F4] disabled:opacity-50"
              >
                {checkingBinaries ? "Checking..." : "Re-check"}
              </button>
            </div>

            {#if canAutoSetupAnyMissing}
              <button
                type="button"
                onclick={onAutoSetupAll}
                disabled={installingBinary !== null}
                class="w-full rounded-md bg-[#1D8BF8] px-3 py-2 text-xs font-semibold text-white transition hover:bg-[#137CE5] disabled:opacity-50"
              >
                {installingBinary === "all"
                  ? "Installing missing CLIs via OS package manager..."
                  : `Auto-Setup Missing CLIs (${binaries?.platform ?? "OS"}) →`}
              </button>
            {/if}

            {#if checkError}
              <div
                class="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700"
              >
                {checkError}
              </div>
            {/if}

            {#if setupOutcomes.length > 0}
              <div
                class="space-y-1 rounded-lg border border-[#E5E3DD] bg-[#F8F7F4] p-2.5 text-[11px]"
              >
                {#each setupOutcomes as outcome (outcome.binary)}
                  <p
                    class={outcome.succeeded
                      ? "font-medium text-emerald-700"
                      : "text-amber-800"}
                  >
                    {outcome.message}
                  </p>
                {/each}
              </div>
            {/if}

            <!-- makemkvcon card -->
            <div
              class="space-y-2.5 rounded-xl border border-[#E5E3DD] bg-[#FAF9F6] p-3.5"
            >
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <span
                    class={`h-2.5 w-2.5 rounded-full ${statusDotClass(binaries?.makemkv)}`}
                  ></span>
                  <span class="font-mono text-xs font-bold text-[#111111]">
                    makemkvcon
                  </span>
                </div>
                {#if binaries}
                  <span
                    class={`rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold ${
                      binaries.makemkv.found
                        ? "border-[#24B465] bg-[#3AD27E] text-[#111111]"
                        : "border-[#E5E3DD] bg-white text-[#5C5B57]"
                    }`}
                  >
                    {binaries.makemkv.found
                      ? (binaries.makemkv.version ?? "Ready")
                      : formatSourceLabel(binaries.makemkv.source)}
                  </span>
                {/if}
              </div>

              {#if binaries?.makemkv.path}
                <p
                  class="truncate rounded border border-[#E5E3DD] bg-white px-2 py-1 font-mono text-[11px] text-[#52524E]"
                  title={binaries.makemkv.path}
                >
                  {binaries.makemkv.path}
                </p>
              {/if}

              {#if binaries && !binaries.makemkv.found}
                {#if binaries.makemkv.path && binaries.makemkv.error}
                  <p
                    class="rounded-md border border-red-200 bg-red-50 px-2.5 py-1.5 font-mono text-[11px] text-red-700"
                  >
                    {binaries.makemkv.error}
                  </p>
                {:else}
                  <p class="text-[11px] leading-relaxed text-[#6B6A65]">
                    {binaries.makemkv.installHint.note}
                  </p>
                  <div class="flex items-center gap-1.5">
                    <code
                      class="flex-1 truncate rounded border border-[#E5E3DD] bg-white px-2 py-1 font-mono text-[10px] text-[#111111]"
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
                      class="rounded border border-[#E5E3DD] bg-white px-2 py-1 text-[10px] font-semibold text-[#111111] hover:bg-[#F2F0EA]"
                    >
                      {copiedCommand === binaries.makemkv.installHint.command
                        ? "Copied"
                        : "Copy"}
                    </button>
                  </div>
                {/if}
              {/if}

              <div class="flex items-center gap-2 pt-0.5">
                {#if binaries && !binaries.makemkv.found && binaries.makemkv.installHint.canAutoInstall}
                  <button
                    type="button"
                    onclick={() => onInstallSingle("makeMkv")}
                    disabled={installingBinary !== null}
                    class="flex-1 rounded-md bg-[#1D8BF8] px-2.5 py-1.5 text-[11px] font-semibold text-white hover:bg-[#137CE5] disabled:opacity-50"
                  >
                    {installingBinary === "makeMkv"
                      ? "Installing..."
                      : `Install (${binaries.makemkv.installHint.packageManager})`}
                  </button>
                {/if}
                <button
                  type="button"
                  onclick={() => onBrowseBinary("makemkv")}
                  class="flex-1 rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1.5 text-[11px] font-semibold text-[#111111] transition hover:bg-[#F2F0EA]"
                >
                  Select Path...
                </button>
                {#if settings.makemkvPath}
                  <button
                    type="button"
                    onclick={() => onResetBinaryPath("makemkv")}
                    class="rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1.5 text-[11px] font-medium text-[#5C5B57] hover:text-[#111111]"
                  >
                    Auto
                  </button>
                {/if}
              </div>
            </div>

            <!-- HandBrakeCLI card -->
            <div
              class="space-y-2.5 rounded-xl border border-[#E5E3DD] bg-[#FAF9F6] p-3.5"
            >
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <span
                    class={`h-2.5 w-2.5 rounded-full ${statusDotClass(binaries?.handbrake)}`}
                  ></span>
                  <span class="font-mono text-xs font-bold text-[#111111]">
                    HandBrakeCLI
                  </span>
                </div>
                {#if binaries}
                  <span
                    class={`rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold ${
                      binaries.handbrake.found
                        ? "border-[#24B465] bg-[#3AD27E] text-[#111111]"
                        : "border-[#E5E3DD] bg-white text-[#5C5B57]"
                    }`}
                  >
                    {binaries.handbrake.found
                      ? (binaries.handbrake.version ?? "Ready")
                      : formatSourceLabel(binaries.handbrake.source)}
                  </span>
                {/if}
              </div>

              {#if binaries?.handbrake.path}
                <p
                  class="truncate rounded border border-[#E5E3DD] bg-white px-2 py-1 font-mono text-[11px] text-[#52524E]"
                  title={binaries.handbrake.path}
                >
                  {binaries.handbrake.path}
                </p>
              {/if}

              {#if binaries && !binaries.handbrake.found}
                {#if binaries.handbrake.path && binaries.handbrake.error}
                  <p
                    class="rounded-md border border-red-200 bg-red-50 px-2.5 py-1.5 font-mono text-[11px] text-red-700"
                  >
                    {binaries.handbrake.error}
                  </p>
                {:else}
                  <p class="text-[11px] leading-relaxed text-[#6B6A65]">
                    {binaries.handbrake.installHint.note}
                  </p>
                  <div class="flex items-center gap-1.5">
                    <code
                      class="flex-1 truncate rounded border border-[#E5E3DD] bg-white px-2 py-1 font-mono text-[10px] text-[#111111]"
                      title={binaries.handbrake.installHint.command}
                    >
                      {binaries.handbrake.installHint.command}
                    </code>
                    <button
                      type="button"
                      onclick={() => {
                        if (binaries) {
                          void copyCommand(
                            binaries.handbrake.installHint.command,
                          );
                        }
                      }}
                      class="rounded border border-[#E5E3DD] bg-white px-2 py-1 text-[10px] font-semibold text-[#111111] hover:bg-[#F2F0EA]"
                    >
                      {copiedCommand === binaries.handbrake.installHint.command
                        ? "Copied"
                        : "Copy"}
                    </button>
                  </div>
                {/if}
              {/if}

              <div class="flex items-center gap-2 pt-0.5">
                {#if binaries && !binaries.handbrake.found && binaries.handbrake.installHint.canAutoInstall}
                  <button
                    type="button"
                    onclick={() => onInstallSingle("handBrake")}
                    disabled={installingBinary !== null}
                    class="flex-1 rounded-md bg-[#1D8BF8] px-2.5 py-1.5 text-[11px] font-semibold text-white hover:bg-[#137CE5] disabled:opacity-50"
                  >
                    {installingBinary === "handBrake"
                      ? "Installing..."
                      : `Install (${binaries.handbrake.installHint.packageManager})`}
                  </button>
                {/if}
                <button
                  type="button"
                  onclick={() => onBrowseBinary("handbrake")}
                  class="flex-1 rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1.5 text-[11px] font-semibold text-[#111111] transition hover:bg-[#F2F0EA]"
                >
                  Select Path...
                </button>
                {#if settings.handbrakePath}
                  <button
                    type="button"
                    onclick={() => onResetBinaryPath("handbrake")}
                    class="rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1.5 text-[11px] font-medium text-[#5C5B57] hover:text-[#111111]"
                  >
                    Auto
                  </button>
                {/if}
              </div>
            </div>
          </div>

        {:else if activeCategory === "storage"}
          <div class="space-y-3">
            <div
              class="space-y-2.5 rounded-xl border border-[#E5E3DD] bg-[#FAF9F6] p-3.5"
            >
              <div class="flex items-center justify-between gap-2">
                <div>
                  <div class="text-xs font-semibold text-[#111111]">
                    Output Directory
                  </div>
                  <div class="text-[11px] text-[#6B6A65]">
                    Destination for final movies and TV show folders.
                  </div>
                </div>
                <div class="flex items-center gap-1.5">
                  <button
                    type="button"
                    onclick={() => onBrowseDirectory("outputDir")}
                    class="rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1 text-[11px] font-semibold text-[#111111] hover:bg-[#F2F0EA]"
                  >
                    Change
                  </button>
                  {#if settings.outputDir}
                    <button
                      type="button"
                      onclick={() => onResetDirectory("outputDir")}
                      class="rounded-md border border-[#E5E3DD] bg-white px-2 py-1 text-[11px] text-[#6B6A65] hover:text-[#111111]"
                    >
                      Default
                    </button>
                  {/if}
                </div>
              </div>
              <p
                class="truncate rounded border border-[#E5E3DD] bg-white px-2.5 py-1.5 font-mono text-[11px] text-[#111111]"
                title={settings.outputDir ||
                  binaries?.defaultPaths.outputDir ||
                  ""}
              >
                {settings.outputDir ||
                  binaries?.defaultPaths.outputDir ||
                  "Auto-detecting..."}
              </p>
            </div>

            <div
              class="space-y-2.5 rounded-xl border border-[#E5E3DD] bg-[#FAF9F6] p-3.5"
            >
              <div class="flex items-center justify-between gap-2">
                <div>
                  <div class="text-xs font-semibold text-[#111111]">
                    Temp Rip Folder
                  </div>
                  <div class="text-[11px] text-[#6B6A65]">
                    Intermediate MKV staging directory between Rip and Encode.
                  </div>
                </div>
                <div class="flex items-center gap-1.5">
                  <button
                    type="button"
                    onclick={() => onBrowseDirectory("tempDir")}
                    class="rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1 text-[11px] font-semibold text-[#111111] hover:bg-[#F2F0EA]"
                  >
                    Change
                  </button>
                  {#if settings.tempDir}
                    <button
                      type="button"
                      onclick={() => onResetDirectory("tempDir")}
                      class="rounded-md border border-[#E5E3DD] bg-white px-2 py-1 text-[11px] text-[#6B6A65] hover:text-[#111111]"
                    >
                      Default
                    </button>
                  {/if}
                </div>
              </div>
              <p
                class="truncate rounded border border-[#E5E3DD] bg-white px-2.5 py-1.5 font-mono text-[11px] text-[#111111]"
                title={settings.tempDir || binaries?.defaultPaths.tempDir || ""}
              >
                {settings.tempDir ||
                  binaries?.defaultPaths.tempDir ||
                  "Auto-detecting..."}
              </p>
            </div>
          </div>

        {:else if activeCategory === "encoding"}
          <div class="space-y-4">
            <!-- Quality Profile Matrix Header: Local vs Remote + 480p / 720p / 1080p / 4K -->
            <div class="space-y-2.5 rounded-xl border border-[#E5E3DD] bg-[#FAF9F6] p-3.5">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <div class="text-xs font-bold text-[#111111]">
                    Resolution × Delivery Quality Matrix
                  </div>
                  <div class="text-[11px] text-[#6B6A65]">
                    Customize HandBrake settings per pixel tier (up to 4K) for
                    Local LAN vs Remote streaming.
                  </div>
                </div>

                <div
                  class="inline-flex rounded-full border border-[#DFDDD6] bg-white p-0.5 text-[11px]"
                  role="group"
                  aria-label="Edit delivery target"
                >
                  <button
                    type="button"
                    onclick={() => (editDelivery = "local")}
                    class={`rounded-full px-2.5 py-0.5 font-semibold transition ${
                      editDelivery === "local"
                        ? "bg-[#3AD27E] text-[#111111]"
                        : "text-[#6B6A65] hover:text-[#111111]"
                    }`}
                  >
                    Local
                  </button>
                  <button
                    type="button"
                    onclick={() => (editDelivery = "remote")}
                    class={`rounded-full px-2.5 py-0.5 font-semibold transition ${
                      editDelivery === "remote"
                        ? "bg-[#58B0F9] text-[#111111]"
                        : "text-[#6B6A65] hover:text-[#111111]"
                    }`}
                  >
                    Remote
                  </button>
                </div>
              </div>

              <!-- 4 Pixel Tier Selector Cards -->
              <div class="grid grid-cols-4 gap-2">
                {#each RESOLUTION_TIERS as tier (tier.id)}
                  {@const slot =
                    settings.qualityProfiles[
                      qualityProfileKey(tier.id, editDelivery)
                    ]}
                  <button
                    type="button"
                    onclick={() => (editResolution = tier.id)}
                    class={`rounded-lg border p-2 text-left transition ${
                      editResolution === tier.id
                        ? "border-[#111111] bg-white shadow-[0_1px_2px_rgba(0,0,0,0.06)]"
                        : "border-[#E5E3DD] bg-white/60 hover:bg-white"
                    }`}
                  >
                    <div class="flex items-center justify-between">
                      <span class="font-mono text-xs font-bold text-[#111111]">
                        {tier.shortLabel}
                      </span>
                      <span class="font-mono text-[9px] text-[#787772]">
                        {slot ? `RF ${slot.rfQuality.toFixed(0)}` : ""}
                      </span>
                    </div>
                    <div class="mt-0.5 truncate font-mono text-[10px] text-[#6B6A65]">
                      {slot ? shortEncoderLabel(slot.encoder) : tier.dims}
                    </div>
                  </button>
                {/each}
              </div>

              {#if editingProfile}
                <div class="space-y-3 border-t border-[#E5E3DD] pt-3">
                  <div class="flex items-center justify-between">
                    <span class="text-xs font-bold text-[#111111]">
                      {editingProfile.label}
                    </span>
                    <div class="flex items-center gap-2">
                      <span class="font-mono text-[10px] text-[#6B6A65]">
                        {editingProfile.estimatedBitrateLabel}
                      </span>
                      <button
                        type="button"
                        onclick={() => onResetQualityProfiles(editingProfile.id)}
                        class="rounded border border-[#DFDDD6] bg-white px-2 py-0.5 text-[10px] font-semibold text-[#52524E] hover:text-[#111111]"
                      >
                        Reset Slot
                      </button>
                    </div>
                  </div>

                  <!-- RF Quality Slider -->
                  <div class="space-y-1">
                    <div class="flex items-center justify-between text-[11px]">
                      <label
                        for="setting-rf-slider"
                        class="font-semibold text-[#111111]"
                      >
                        Constant Quality (RF {editingProfile.rfQuality.toFixed(1)})
                      </label>
                      <span
                        class="rounded border border-[#B5DAFA] bg-[#DCEEFF] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#155CA2]"
                      >
                        {rfQualityTierLabel(editingProfile.rfQuality)}
                      </span>
                    </div>
                    <input
                      id="setting-rf-slider"
                      type="range"
                      min="14"
                      max="28"
                      step="0.5"
                      value={editingProfile.rfQuality}
                      oninput={(e) =>
                        patchEditingProfile({
                          rfQuality: Number.parseFloat(e.currentTarget.value),
                        })}
                      class="w-full accent-[#111111]"
                    />
                    <div
                      class="flex justify-between font-mono text-[9px] text-[#787772]"
                    >
                      <span>14 (Archival / Larger)</span>
                      <span>20 (Balanced)</span>
                      <span>28 (Compact / Remote)</span>
                    </div>
                  </div>

                  <div class="grid grid-cols-2 gap-2.5">
                    <div class="space-y-1">
                      <label
                        for="setting-slot-encoder"
                        class="block text-[11px] font-semibold text-[#52524E]"
                      >
                        Video Encoder
                      </label>
                      <select
                        id="setting-slot-encoder"
                        value={editingProfile.encoder}
                        onchange={handleEncoderChange}
                        class="w-full rounded-md border border-[#DFDDD6] bg-white px-2 py-1.5 text-xs text-[#111111] outline-none focus:border-[#111111]"
                      >
                        {#each VIDEO_ENCODERS as enc (enc.id)}
                          <option value={enc.id}>{enc.label}</option>
                        {/each}
                      </select>
                    </div>

                    <div class="space-y-1">
                      <label
                        for="setting-slot-speed"
                        class="block text-[11px] font-semibold text-[#52524E]"
                      >
                        Encoder Speed Preset
                      </label>
                      <select
                        id="setting-slot-speed"
                        value={editingProfile.encoderPreset}
                        onchange={(e) =>
                          patchEditingProfile({
                            encoderPreset: e.currentTarget.value,
                          })}
                        class="w-full rounded-md border border-[#DFDDD6] bg-white px-2 py-1.5 font-mono text-xs text-[#111111] outline-none focus:border-[#111111]"
                      >
                        {#each ENCODER_PRESETS as speed (speed)}
                          <option value={speed}>{speed}</option>
                        {/each}
                      </select>
                    </div>

                    <div class="space-y-1">
                      <label
                        for="setting-slot-audio"
                        class="block text-[11px] font-semibold text-[#52524E]"
                      >
                        Audio Strategy
                      </label>
                      <select
                        id="setting-slot-audio"
                        value={editingProfile.audioMode}
                        onchange={handleAudioModeChange}
                        class="w-full rounded-md border border-[#DFDDD6] bg-white px-2 py-1.5 text-xs text-[#111111] outline-none focus:border-[#111111]"
                      >
                        <option value="passthrough">Passthrough + AAC Fallback</option>
                        <option value="aac51">5.1 Surround AAC</option>
                        <option value="aacStereo">2.0 Stereo AAC</option>
                      </select>
                    </div>

                    <div class="grid grid-cols-2 gap-2">
                      <div class="space-y-1">
                        <label
                          for="setting-slot-bitrate"
                          class="block text-[11px] font-semibold text-[#52524E]"
                        >
                          Audio kbps
                        </label>
                        <select
                          id="setting-slot-bitrate"
                          value={editingProfile.audioBitrateKbps}
                          onchange={(e) =>
                            patchEditingProfile({
                              audioBitrateKbps: Number.parseInt(
                                e.currentTarget.value,
                                10,
                              ),
                            })}
                          class="w-full rounded-md border border-[#DFDDD6] bg-white px-2 py-1.5 font-mono text-xs text-[#111111] outline-none focus:border-[#111111]"
                        >
                          {#each AUDIO_BITRATES as kbps (kbps)}
                            <option value={kbps}>{kbps}k</option>
                          {/each}
                        </select>
                      </div>

                      <div class="space-y-1">
                        <label
                          for="setting-slot-container"
                          class="block text-[11px] font-semibold text-[#52524E]"
                        >
                          Container
                        </label>
                        <select
                          id="setting-slot-container"
                          value={editingProfile.container}
                          onchange={(e) =>
                            patchEditingProfile({
                              container: e.currentTarget.value,
                            })}
                          class="w-full rounded-md border border-[#DFDDD6] bg-white px-2 py-1.5 font-mono text-xs text-[#111111] outline-none focus:border-[#111111]"
                        >
                          <option value="mkv">.mkv</option>
                          <option value="mp4">.mp4</option>
                        </select>
                      </div>
                    </div>
                  </div>

                  <div class="grid grid-cols-2 items-center gap-2.5">
                    <div class="space-y-1">
                      <label
                        for="setting-slot-subtitles"
                        class="block text-[11px] font-semibold text-[#52524E]"
                      >
                        Subtitles
                      </label>
                      <select
                        id="setting-slot-subtitles"
                        value={editingProfile.subtitleMode}
                        onchange={handleSubtitleModeChange}
                        class="w-full rounded-md border border-[#DFDDD6] bg-white px-2 py-1.5 text-xs text-[#111111] outline-none focus:border-[#111111]"
                      >
                        <option value="all">Keep All Subtitles</option>
                        <option value="foreignScan">Foreign Audio Scan (Forced)</option>
                        <option value="none">None</option>
                      </select>
                    </div>

                    <div class="space-y-1.5 pt-3 text-[11px]">
                      <label
                        for="setting-slot-decomb"
                        class="flex cursor-pointer items-center gap-2 font-medium text-[#111111]"
                      >
                        <input
                          id="setting-slot-decomb"
                          type="checkbox"
                          checked={editingProfile.decomb}
                          onchange={(e) =>
                            patchEditingProfile({
                              decomb: e.currentTarget.checked,
                            })}
                          class="h-3.5 w-3.5 accent-[#111111]"
                        />
                        <span>Decomb / Deinterlace</span>
                      </label>

                      <label
                        for="setting-slot-webopt"
                        class="flex cursor-pointer items-center gap-2 font-medium text-[#111111]"
                      >
                        <input
                          id="setting-slot-webopt"
                          type="checkbox"
                          checked={editingProfile.webOptimize}
                          disabled={editingProfile.container !== "mp4"}
                          onchange={(e) =>
                            patchEditingProfile({
                              webOptimize: e.currentTarget.checked,
                            })}
                          class="h-3.5 w-3.5 accent-[#111111] disabled:opacity-40"
                        />
                        <span>MP4 Web Optimize (Fast Start)</span>
                      </label>
                    </div>
                  </div>

                  <!-- Live HandBrakeCLI Command Preview -->
                  <div class="space-y-1">
                    <div
                      class="text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
                    >
                      Generated HandBrakeCLI Flags
                    </div>
                    <code
                      class="block max-h-16 overflow-y-auto break-all rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1.5 font-mono text-[10px] leading-relaxed text-[#3F3E3B]"
                    >
                      {cliFlagsPreview}
                    </code>
                  </div>
                </div>
              {/if}
            </div>

            <!-- Ripping Filter + Optional Custom Preset JSON -->
            <div class="grid grid-cols-2 gap-3">
              <div class="space-y-1">
                <label
                  for="setting-min-length"
                  class="block text-xs font-semibold text-[#111111]"
                >
                  Min Disc Title Length (sec)
                </label>
                <input
                  id="setting-min-length"
                  type="number"
                  min="0"
                  max="7200"
                  value={settings.minTitleLengthSeconds}
                  onchange={(e) => {
                    const parsed = Number.parseInt(e.currentTarget.value, 10);
                    onUpdateSetting(
                      "minTitleLengthSeconds",
                      Number.isFinite(parsed) && parsed >= 0 ? parsed : 120,
                    );
                  }}
                  class="w-full rounded-lg border border-[#E5E3DD] bg-[#F8F7F4] px-3 py-1.5 font-mono text-xs text-[#111111] outline-none focus:border-[#111111] focus:bg-white"
                />
              </div>

              <div class="flex items-end justify-end">
                <button
                  type="button"
                  onclick={() => onResetQualityProfiles()}
                  class="rounded-md border border-[#DFDDD6] bg-white px-3 py-1.5 text-xs font-semibold text-[#52524E] hover:bg-[#F8F7F4] hover:text-[#111111]"
                >
                  Reset All 8 Profiles
                </button>
              </div>
            </div>

            <div
              class="space-y-2 rounded-xl border border-[#E5E3DD] bg-[#FAF9F6] p-3"
            >
              <div class="flex items-center justify-between gap-2">
                <div>
                  <div class="text-xs font-semibold text-[#111111]">
                    Optional Custom HandBrake Preset JSON
                  </div>
                  <div class="text-[11px] text-[#6B6A65]">
                    Override matrix with an exported GUI preset via
                    <span class="font-mono">--preset-import-file</span>.
                  </div>
                </div>
                <div class="flex items-center gap-1.5">
                  <button
                    type="button"
                    onclick={onBrowseCustomPreset}
                    class="rounded-md border border-[#E5E3DD] bg-white px-2.5 py-1 text-[11px] font-semibold text-[#111111] hover:bg-[#F2F0EA]"
                  >
                    Browse...
                  </button>
                  {#if settings.customPresetFile}
                    <button
                      type="button"
                      onclick={() => onUpdateSetting("customPresetFile", null)}
                      class="rounded-md border border-[#E5E3DD] bg-white px-2 py-1 text-[11px] text-[#6B6A65] hover:text-[#111111]"
                    >
                      Clear
                    </button>
                  {/if}
                </div>
              </div>
              <p class="truncate font-mono text-[11px] text-[#52524E]">
                {settings.customPresetFile ?? "None (using Quality Matrix above)"}
              </p>
            </div>
          </div>

        {:else if activeCategory === "metadata"}
          <div class="space-y-4">
            <div class="grid grid-cols-2 gap-3">
              <div class="space-y-1.5">
                <label
                  for="setting-naming-convention"
                  class="block text-xs font-semibold text-[#111111]"
                >
                  Naming Convention
                </label>
                <select
                  id="setting-naming-convention"
                  value={settings.namingConvention}
                  onchange={handleNamingChange}
                  class="w-full rounded-lg border border-[#E5E3DD] bg-[#F8F7F4] px-3 py-2 text-xs text-[#111111] outline-none focus:border-[#111111] focus:bg-white"
                >
                  <option value="plex">Plex {"{imdb-...} / {tvdb-...}"}</option>
                  <option value="jellyfin">Jellyfin [imdbid-... / tvdbid-...]</option>
                  <option value="plain">Plain Title (Year)</option>
                </select>
              </div>

              <div class="space-y-1.5">
                <label
                  for="setting-metadata-provider"
                  class="block text-xs font-semibold text-[#111111]"
                >
                  Default Lookup Provider
                </label>
                <select
                  id="setting-metadata-provider"
                  value={settings.defaultMetadataProvider}
                  onchange={handleProviderChange}
                  class="w-full rounded-lg border border-[#E5E3DD] bg-[#F8F7F4] px-3 py-2 text-xs text-[#111111] outline-none focus:border-[#111111] focus:bg-white"
                >
                  <option value="imdb">IMDb (OMDb API)</option>
                  <option value="tvdb">TVDB v4</option>
                  <option value="tmdb">TMDB</option>
                  <option value="tvmaze">TVMaze (Free TV · No Key)</option>
                </select>
              </div>
            </div>

            <div class="space-y-1.5">
                <label
                  for="setting-tv-folder-org"
                  class="block text-xs font-semibold text-[#111111]"
                >
                  Batch Output Folder Organization
                </label>
                <select
                  id="setting-tv-folder-org"
                  value={settings.tvFolderOrganization}
                  onchange={handleFolderOrgChange}
                  class="w-full rounded-lg border border-[#E5E3DD] bg-[#F8F7F4] px-3 py-2 text-xs text-[#111111] outline-none focus:border-[#111111] focus:bg-white"
                >
                  <option value="ask">Ask before encoding batches (Recommended)</option>
                  <option value="showAndSeason">Create Movie / Show &amp; Season XX folders</option>
                  <option value="seasonOnly">Create Season XX folders directly in Output dir</option>
                  <option value="flat">Keep flat (no extra subfolders)</option>
                </select>
                <p class="text-[11px] text-[#6B6A65]">
                  Controls whether Movie and multi-season TV batches automatically create organized folders as they encode.
                </p>
            </div>

            <div class="space-y-3 rounded-xl border border-[#E5E3DD] bg-[#FAF9F6] p-3.5">
              <div class="flex items-center justify-between">
                <div>
                  <div class="text-xs font-semibold text-[#111111]">
                    Metadata API Keys
                  </div>
                  <div class="text-[11px] text-[#6B6A65]">
                    Stored locally for online IMDb / TVDB / TMDB searches.
                  </div>
                </div>
                <button
                  type="button"
                  onclick={() => (showApiKeys = !showApiKeys)}
                  class="rounded border border-[#E5E3DD] bg-white px-2 py-1 text-[10px] font-semibold text-[#52524E] hover:text-[#111111]"
                >
                  {showApiKeys ? "Hide" : "Show"}
                </button>
              </div>

              <div class="space-y-1">
                <label for="setting-omdb-key" class="block text-[11px] font-medium text-[#52524E]">
                  OMDb API Key (IMDb)
                </label>
                <input
                  id="setting-omdb-key"
                  type={showApiKeys ? "text" : "password"}
                  value={settings.omdbApiKey ?? ""}
                  placeholder="Optional OMDb key"
                  onchange={(e) =>
                    onUpdateSetting(
                      "omdbApiKey",
                      e.currentTarget.value.trim() || null,
                    )}
                  class="w-full rounded-lg border border-[#E5E3DD] bg-white px-2.5 py-1.5 font-mono text-xs text-[#111111] placeholder:text-[#A4A29B] outline-none focus:border-[#111111]"
                />
              </div>

              <div class="space-y-1">
                <label for="setting-tvdb-key" class="block text-[11px] font-medium text-[#52524E]">
                  TVDB API Key
                </label>
                <input
                  id="setting-tvdb-key"
                  type={showApiKeys ? "text" : "password"}
                  value={settings.tvdbApiKey ?? ""}
                  placeholder="Optional TVDB v4 key"
                  onchange={(e) =>
                    onUpdateSetting(
                      "tvdbApiKey",
                      e.currentTarget.value.trim() || null,
                    )}
                  class="w-full rounded-lg border border-[#E5E3DD] bg-white px-2.5 py-1.5 font-mono text-xs text-[#111111] placeholder:text-[#A4A29B] outline-none focus:border-[#111111]"
                />
              </div>

              <div class="space-y-1">
                <label for="setting-tmdb-key" class="block text-[11px] font-medium text-[#52524E]">
                  TMDB API Key / Read Access Token
                </label>
                <input
                  id="setting-tmdb-key"
                  type={showApiKeys ? "text" : "password"}
                  value={settings.tmdbApiKey ?? ""}
                  placeholder="Optional TMDB key"
                  onchange={(e) =>
                    onUpdateSetting(
                      "tmdbApiKey",
                      e.currentTarget.value.trim() || null,
                    )}
                  class="w-full rounded-lg border border-[#E5E3DD] bg-white px-2.5 py-1.5 font-mono text-xs text-[#111111] placeholder:text-[#A4A29B] outline-none focus:border-[#111111]"
                />
              </div>
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}
