<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { Store } from "@tauri-apps/plugin-store";
  import FirstRunSetupModal from "./lib/FirstRunSetupModal.svelte";
  import PreviewDrawer from "./lib/PreviewDrawer.svelte";
  import QueueDock from "./lib/QueueDock.svelte";
  import SettingsDropdown from "./lib/SettingsDropdown.svelte";
  import {
    RESOLUTION_TIERS,
    defaultQualityProfileMap,
    qualityProfileKey,
    shortEncoderLabel,
  } from "./lib/qualityDefaults";
  import type {
    AppSettings,
    AutoSetupResult,
    BinariesCheckResult,
    BinaryInstallOutcome,
    BinaryKind,
    BinaryPathsConfig,
    DeliveryTarget,
    DiscLabelGuess,
    DiscTitle,
    ExternalIds,
    MediaKind,
    MediaMetadata,
    MetadataProvider,
    MetadataProviderConfig,
    MetadataSearchQuery,
    NamingConvention,
    OpticalDrive,
    PipelineMode,
    QualityProfile,
    QueueJob,
    ResolutionTier,
    SettingsCategory,
    StreamTrack,
  } from "./lib/types";

  const SETTINGS_STORE_FILE = "settings.json";

  // Binary detection & OS auto-setup state
  let binaries = $state<BinariesCheckResult | null>(null);
  let checkingBinaries = $state(false);
  let installingBinary = $state<BinaryKind | "all" | null>(null);
  let checkError = $state<string | null>(null);
  let setupOutcomes = $state<BinaryInstallOutcome[]>([]);

  // Categorized settings dropdown & first-run onboarding state
  let settingsOpen = $state(false);
  let settingsCategory = $state<SettingsCategory>("general");
  let firstRunModalOpen = $state(false);

  let settings = $state<AppSettings>({
    defaultMode: "ripAndEncode",
    autoSetupOnFirstRun: true,
    firstRunSetupDone: false,
    keepTempMkv: false,
    autoEjectDisc: false,
    makemkvPath: null,
    handbrakePath: null,
    tempDir: null,
    outputDir: null,
    activeResolution: "p1080",
    activeDelivery: "local",
    qualityProfiles: defaultQualityProfileMap(),
    defaultPreset: null,
    defaultContainer: "mkv",
    customPresetFile: null,
    minTitleLengthSeconds: 120,
    namingConvention: "plex",
    defaultMetadataProvider: "imdb",
    omdbApiKey: null,
    tvdbApiKey: null,
    tmdbApiKey: null,
  });

  // Active workspace mode
  let workflowMode = $state<PipelineMode>("ripAndEncode");

  // Layout panels
  let previewOpen = $state(true);
  let queueExpanded = $state(false);

  // Source selection & inspection state
  let drives = $state<OpticalDrive[]>([]);
  let selectedDriveIndex = $state<number | null>(null);
  let titles = $state<DiscTitle[]>([]);
  let checkedTitleIds = $state<number[]>([]);
  let inspectedTitleId = $state<number | null>(null);

  let localSourceFiles = $state<string[]>([]);
  let checkedLocalFiles = $state<string[]>([]);
  let inspectedLocalFile = $state<string | null>(null);

  // HandBrake active quality profile & container state
  let outputContainer = $state<string>("mkv");

  // Compact IMDb / TVDB / TMDB metadata strip state
  let mediaKind = $state<MediaKind>("movie");
  let namingConvention = $state<NamingConvention>("plex");
  let mediaTitle = $state<string>("");
  let mediaYearInput = $state<string>("");
  let seasonInput = $state<string>("1");
  let startEpisodeInput = $state<string>("1");
  let imdbId = $state<string>("");
  let tvdbId = $state<string>("");
  let tmdbId = $state<string>("");
  let metadataProvider = $state<MetadataProvider>("imdb");
  let metadataLookupStatus = $state<string | null>(null);
  let searchingMetadata = $state(false);
  let searchResults = $state<MediaMetadata[]>([]);
  let selectedPosterUrl = $state<string | null>(null);
  let selectedOverview = $state<string | null>(null);
  let outputFilenamePreview = $state<string>("Untitled.mkv");

  // Queue state
  let queueJobs = $state<QueueJob[]>([]);

  let settingsStore: Store | null = null;

  const parsedYear = $derived.by(() => {
    const n = Number.parseInt(mediaYearInput.trim(), 10);
    return Number.isFinite(n) && n >= 1900 && n <= 2099 ? n : null;
  });

  const parsedSeason = $derived.by(() => {
    const n = Number.parseInt(seasonInput.trim(), 10);
    return Number.isFinite(n) && n >= 0 ? n : 1;
  });

  const parsedStartEpisode = $derived.by(() => {
    const n = Number.parseInt(startEpisodeInput.trim(), 10);
    return Number.isFinite(n) && n >= 0 ? n : 1;
  });

  const requiresMakeMkv = $derived(
    workflowMode === "ripOnly" || workflowMode === "ripAndEncode",
  );

  const requiresHandBrake = $derived(
    workflowMode === "encodeOnly" || workflowMode === "ripAndEncode",
  );

  const modeReady = $derived.by(() => {
    if (!binaries) return false;
    const mkvOk = !requiresMakeMkv || binaries.makemkv.found;
    const hbOk = !requiresHandBrake || binaries.handbrake.found;
    return mkvOk && hbOk;
  });

  const missingCliCount = $derived.by(() => {
    if (!binaries) return 0;
    return (binaries.makemkv.found ? 0 : 1) + (binaries.handbrake.found ? 0 : 1);
  });

  const inspectedTitle = $derived(
    titles.find((t) => t.titleId === inspectedTitleId) ?? null,
  );

  const selectedCount = $derived(
    workflowMode === "encodeOnly"
      ? checkedLocalFiles.length
      : checkedTitleIds.length,
  );

  const allRowsChecked = $derived.by(() => {
    if (workflowMode === "encodeOnly") {
      return (
        localSourceFiles.length > 0 &&
        checkedLocalFiles.length === localSourceFiles.length
      );
    }
    return titles.length > 0 && checkedTitleIds.length === titles.length;
  });

  const resolvedOutputDir = $derived(
    settings.outputDir || binaries?.defaultPaths.outputDir || "Auto",
  );

  const activeQualityProfile = $derived.by(() => {
    const key = qualityProfileKey(
      settings.activeResolution,
      settings.activeDelivery,
    );
    return settings.qualityProfiles[key] ?? null;
  });

  const activeTokenChips = $derived.by(() => {
    const chips: string[] = [];
    chips.push(mediaTitle.trim() || "Title");
    if (parsedYear) chips.push(`(${parsedYear})`);
    if (mediaKind === "tvEpisode") {
      chips.push(
        `S${String(parsedSeason).padStart(2, "0")}E${String(parsedStartEpisode).padStart(2, "0")}`,
      );
    }
    if (imdbId.trim()) chips.push(`imdb:${imdbId.trim()}`);
    if (tvdbId.trim()) chips.push(`tvdb:${tvdbId.trim()}`);
    if (tmdbId.trim()) chips.push(`tmdb:${tmdbId.trim()}`);
    return chips;
  });

  // Live output path preview backed by Rust's metadata::format_output_relative_path
  $effect(() => {
    const payload: MediaMetadata = {
      kind: mediaKind,
      title: mediaTitle,
      year: parsedYear,
      season: mediaKind === "tvEpisode" ? parsedSeason : null,
      episode: mediaKind === "tvEpisode" ? parsedStartEpisode : null,
      episodeTitle: null,
      ids: {
        imdbId: imdbId.trim() || null,
        tvdbId: tvdbId.trim() || null,
        tmdbId: tmdbId.trim() || null,
      },
      overview: null,
      posterUrl: null,
    };
    const convention = namingConvention;
    const ext = workflowMode === "ripOnly" ? "mkv" : outputContainer;

    let cancelled = false;
    void invoke<string>("preview_output_filename", {
      media: payload,
      convention,
      extension: ext,
    })
      .then((preview) => {
        if (!cancelled) {
          outputFilenamePreview = preview;
        }
      })
      .catch(() => {
        if (!cancelled) {
          const safeTitle = mediaTitle.trim() || "Untitled";
          outputFilenamePreview = `${safeTitle}.${ext}`;
        }
      });

    return () => {
      cancelled = true;
    };
  });

  async function persistSetting(key: string, value: unknown): Promise<void> {
    if (!settingsStore) return;
    try {
      if (value === null || value === undefined || value === "") {
        await settingsStore.delete(key);
      } else {
        await settingsStore.set(key, value);
      }
      await settingsStore.save();
    } catch {
      // Ignore persistence errors outside Tauri runtime.
    }
  }

  function updateSetting<K extends keyof AppSettings>(
    key: K,
    value: AppSettings[K],
  ): void {
    settings[key] = value;

    if (key === "defaultMode") {
      workflowMode = value as PipelineMode;
    } else if (key === "defaultContainer") {
      outputContainer = value as string;
    } else if (key === "namingConvention") {
      namingConvention = value as NamingConvention;
    } else if (key === "defaultMetadataProvider") {
      metadataProvider = value as MetadataProvider;
    }

    void persistSetting(key, value);
  }

  async function readStoreValue<T>(key: string): Promise<T | null> {
    if (!settingsStore) return null;
    return (await settingsStore.get<T>(key)) ?? null;
  }

  async function loadPersistedSettings(): Promise<void> {
    try {
      settingsStore = await Store.load(SETTINGS_STORE_FILE);

      settings.defaultMode =
        (await readStoreValue<PipelineMode>("defaultMode")) ?? "ripAndEncode";
      settings.autoSetupOnFirstRun =
        (await readStoreValue<boolean>("autoSetupOnFirstRun")) ?? true;
      settings.firstRunSetupDone =
        (await readStoreValue<boolean>("firstRunSetupDone")) ?? false;
      settings.keepTempMkv =
        (await readStoreValue<boolean>("keepTempMkv")) ?? false;
      settings.autoEjectDisc =
        (await readStoreValue<boolean>("autoEjectDisc")) ?? false;
      settings.makemkvPath = await readStoreValue<string>("makemkvPath");
      settings.handbrakePath = await readStoreValue<string>("handbrakePath");
      settings.tempDir = await readStoreValue<string>("tempDir");
      settings.outputDir = await readStoreValue<string>("outputDir");
      settings.activeResolution =
        (await readStoreValue<ResolutionTier>("activeResolution")) ?? "p1080";
      settings.activeDelivery =
        (await readStoreValue<DeliveryTarget>("activeDelivery")) ?? "local";
      const savedProfiles =
        await readStoreValue<Record<string, QualityProfile>>("qualityProfiles");
      if (savedProfiles) {
        settings.qualityProfiles = {
          ...defaultQualityProfileMap(),
          ...savedProfiles,
        };
      }
      settings.defaultPreset = await readStoreValue<string>("defaultPreset");
      settings.defaultContainer =
        (await readStoreValue<string>("defaultContainer")) ?? "mkv";
      settings.customPresetFile =
        await readStoreValue<string>("customPresetFile");
      settings.minTitleLengthSeconds =
        (await readStoreValue<number>("minTitleLengthSeconds")) ?? 120;
      settings.namingConvention =
        (await readStoreValue<NamingConvention>("namingConvention")) ?? "plex";
      settings.defaultMetadataProvider =
        (await readStoreValue<MetadataProvider>("defaultMetadataProvider")) ??
        "imdb";
      settings.omdbApiKey = await readStoreValue<string>("omdbApiKey");
      settings.tvdbApiKey = await readStoreValue<string>("tvdbApiKey");
      settings.tmdbApiKey = await readStoreValue<string>("tmdbApiKey");

      workflowMode = settings.defaultMode;
      const activeSlot =
        settings.qualityProfiles[
          qualityProfileKey(settings.activeResolution, settings.activeDelivery)
        ];
      outputContainer = activeSlot?.container ?? settings.defaultContainer;
      namingConvention = settings.namingConvention;
      metadataProvider = settings.defaultMetadataProvider;
    } catch {
      settingsStore = null;
    }
  }

  function selectQualityTier(
    resolution: ResolutionTier,
    delivery: DeliveryTarget,
  ): void {
    updateSetting("activeResolution", resolution);
    updateSetting("activeDelivery", delivery);
    const slot =
      settings.qualityProfiles[qualityProfileKey(resolution, delivery)];
    if (slot) {
      outputContainer = slot.container;
    }
  }

  function handleUpdateQualityProfile(updated: QualityProfile): void {
    const nextProfiles = {
      ...settings.qualityProfiles,
      [updated.id]: updated,
    };
    updateSetting("qualityProfiles", nextProfiles);
    if (
      updated.resolution === settings.activeResolution &&
      updated.delivery === settings.activeDelivery
    ) {
      outputContainer = updated.container;
    }
  }

  async function handleResetQualityProfiles(onlyKey?: string): Promise<void> {
    let defaults = defaultQualityProfileMap();
    try {
      const fromRust = await invoke<QualityProfile[]>(
        "get_recommended_quality_profiles",
      );
      const map: Record<string, QualityProfile> = {};
      for (const item of fromRust) {
        map[item.id] = item;
      }
      defaults = map;
    } catch {
      // Use frontend mirror defaults if outside Tauri runtime.
    }

    if (onlyKey && defaults[onlyKey]) {
      handleUpdateQualityProfile(defaults[onlyKey]);
    } else {
      updateSetting("qualityProfiles", defaults);
      const current =
        defaults[
          qualityProfileKey(settings.activeResolution, settings.activeDelivery)
        ];
      if (current) {
        outputContainer = current.container;
      }
    }
  }

  function currentBinaryConfig(): BinaryPathsConfig {
    return {
      makemkvPath: settings.makemkvPath,
      handbrakePath: settings.handbrakePath,
    };
  }

  async function runCheckBinaries(): Promise<BinariesCheckResult | null> {
    checkingBinaries = true;
    checkError = null;
    try {
      const result = await invoke<BinariesCheckResult>("check_binaries", {
        config: currentBinaryConfig(),
      });
      binaries = result;
      return result;
    } catch (err) {
      checkError =
        err instanceof Error
          ? err.message
          : typeof err === "string"
            ? err
            : "Unable to invoke check_binaries. Launch RipDaddy via `npm run tauri dev`.";
      return null;
    } finally {
      checkingBinaries = false;
    }
  }

  async function runAutoSetupAll(): Promise<void> {
    installingBinary = "all";
    checkError = null;
    try {
      const res = await invoke<AutoSetupResult>("auto_setup_dependencies", {
        config: currentBinaryConfig(),
      });
      binaries = res.binaries;
      setupOutcomes = res.outcomes;
      updateSetting("firstRunSetupDone", true);
    } catch (err) {
      checkError = err instanceof Error ? err.message : String(err);
    } finally {
      installingBinary = null;
    }
  }

  async function runInstallSingle(kind: BinaryKind): Promise<void> {
    installingBinary = kind;
    checkError = null;
    try {
      binaries = await invoke<BinariesCheckResult>("install_binary_cli", {
        kind,
        config: currentBinaryConfig(),
      });
    } catch (err) {
      checkError = err instanceof Error ? err.message : String(err);
    } finally {
      installingBinary = null;
    }
  }

  async function refreshQueue(): Promise<void> {
    try {
      queueJobs = await invoke<QueueJob[]>("get_queue");
    } catch {
      queueJobs = [];
    }
  }

  async function setCustomBinaryPath(
    target: "makemkv" | "handbrake",
    path: string | null,
  ): Promise<void> {
    const key = target === "makemkv" ? "makemkvPath" : "handbrakePath";
    updateSetting(key, path);
    await runCheckBinaries();
  }

  async function browseBinary(target: "makemkv" | "handbrake"): Promise<void> {
    try {
      const title =
        target === "makemkv"
          ? "Select makemkvcon executable"
          : "Select HandBrakeCLI executable";
      const selected = await open({
        title,
        multiple: false,
        directory: false,
      });

      if (typeof selected === "string" && selected.trim().length > 0) {
        await setCustomBinaryPath(target, selected.trim());
      }
    } catch (err) {
      checkError =
        err instanceof Error ? err.message : "Failed to open file dialog.";
    }
  }

  async function browseDirectory(target: "tempDir" | "outputDir"): Promise<void> {
    try {
      const selected = await open({
        title:
          target === "tempDir"
            ? "Select temporary MKV staging folder"
            : "Select final output folder",
        directory: true,
        multiple: false,
      });
      if (typeof selected === "string" && selected.trim().length > 0) {
        updateSetting(target, selected.trim());
      }
    } catch (err) {
      checkError =
        err instanceof Error ? err.message : "Failed to open folder dialog.";
    }
  }

  async function browseCustomPresetFile(): Promise<void> {
    try {
      const selected = await open({
        title: "Select HandBrake Preset JSON",
        multiple: false,
        directory: false,
        filters: [{ name: "HandBrake Preset JSON", extensions: ["json"] }],
      });
      if (typeof selected === "string" && selected.trim().length > 0) {
        updateSetting("customPresetFile", selected.trim());
      }
    } catch (err) {
      checkError =
        err instanceof Error ? err.message : "Failed to open file dialog.";
    }
  }

  async function pickLocalVideoFiles(): Promise<void> {
    try {
      const selected = await open({
        title: "Select video files to encode",
        multiple: true,
        directory: false,
        filters: [
          {
            name: "Video Files",
            extensions: ["mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts"],
          },
        ],
      });
      const added: string[] = [];
      if (selected) {
        for (const item of selected) {
          if (item.trim().length > 0 && !localSourceFiles.includes(item)) {
            added.push(item);
          }
        }
      }

      if (added.length > 0) {
        localSourceFiles = [...localSourceFiles, ...added];
        checkedLocalFiles = [...checkedLocalFiles, ...added];
        if (!inspectedLocalFile) {
          inspectedLocalFile = added[0];
        }
      }
    } catch (err) {
      checkError =
        err instanceof Error ? err.message : "Failed to pick video files.";
    }
  }

  function removeLocalSourceFile(path: string): void {
    localSourceFiles = localSourceFiles.filter((item) => item !== path);
    checkedLocalFiles = checkedLocalFiles.filter((item) => item !== path);
    if (inspectedLocalFile === path) {
      inspectedLocalFile = localSourceFiles[0] ?? null;
    }
  }

  function toggleAllRows(): void {
    if (workflowMode === "encodeOnly") {
      checkedLocalFiles = allRowsChecked ? [] : [...localSourceFiles];
    } else {
      checkedTitleIds = allRowsChecked ? [] : titles.map((t) => t.titleId);
    }
  }

  function toggleLocalFileChecked(path: string): void {
    if (checkedLocalFiles.includes(path)) {
      checkedLocalFiles = checkedLocalFiles.filter((p) => p !== path);
    } else {
      checkedLocalFiles = [...checkedLocalFiles, path];
    }
  }

  function toggleTitleChecked(titleId: number): void {
    if (checkedTitleIds.includes(titleId)) {
      checkedTitleIds = checkedTitleIds.filter((id) => id !== titleId);
    } else {
      checkedTitleIds = [...checkedTitleIds, titleId];
    }
  }

  function inspectLocalFile(path: string): void {
    inspectedLocalFile = path;
    previewOpen = true;
  }

  function inspectTitle(titleId: number): void {
    inspectedTitleId = titleId;
    previewOpen = true;
  }

  async function handleCleanCurrentTitle(): Promise<void> {
    if (!mediaTitle.trim()) return;
    try {
      const guess = await invoke<DiscLabelGuess>("clean_disc_label", {
        rawLabel: mediaTitle,
      });
      if (guess.cleanedTitle) {
        mediaTitle = guess.cleanedTitle;
      }
      if (guess.guessedYear) {
        mediaYearInput = String(guess.guessedYear);
      }
      if (guess.guessedSeason) {
        mediaKind = "tvEpisode";
        seasonInput = String(guess.guessedSeason);
      }
    } catch {
      // Ignore outside Tauri runtime.
    }
  }

  async function handleMetadataSearch(): Promise<void> {
    if (!mediaTitle.trim()) {
      metadataLookupStatus = "Enter a movie or show title first.";
      searchResults = [];
      return;
    }
    searchingMetadata = true;
    metadataLookupStatus = null;
    searchResults = [];
    try {
      const query: MetadataSearchQuery = {
        query: mediaTitle.trim(),
        year: parsedYear,
        kind: mediaKind,
        provider: metadataProvider,
      };
      const config: MetadataProviderConfig = {
        omdbApiKey: settings.omdbApiKey,
        tvdbApiKey: settings.tvdbApiKey,
        tmdbApiKey: settings.tmdbApiKey,
      };
      const results = await invoke<MediaMetadata[]>("search_metadata", {
        query,
        config,
      });
      if (results.length === 0) {
        metadataLookupStatus = "No matches found. Try a shorter title or another provider.";
      } else {
        searchResults = results;
      }
    } catch (err) {
      metadataLookupStatus = err instanceof Error ? err.message : String(err);
    } finally {
      searchingMetadata = false;
    }
  }

  async function applyMetadataResult(item: MediaMetadata): Promise<void> {
    mediaKind = item.kind;
    mediaTitle = item.title;
    mediaYearInput = item.year ? String(item.year) : "";
    imdbId = item.ids.imdbId ?? "";
    tvdbId = item.ids.tvdbId ?? "";
    tmdbId = item.ids.tmdbId ?? "";
    selectedPosterUrl = item.posterUrl;
    selectedOverview = item.overview;
    searchResults = [];
    metadataLookupStatus = null;

    if (
      item.ids.tmdbId &&
      (!item.ids.imdbId || !item.ids.tvdbId) &&
      settings.tmdbApiKey
    ) {
      try {
        const config: MetadataProviderConfig = {
          omdbApiKey: settings.omdbApiKey,
          tvdbApiKey: settings.tvdbApiKey,
          tmdbApiKey: settings.tmdbApiKey,
        };
        const ext = await invoke<ExternalIds>("fetch_tmdb_external_ids", {
          kind: item.kind,
          tmdbId: item.ids.tmdbId,
          config,
        });
        if (ext.imdbId) imdbId = ext.imdbId;
        if (ext.tvdbId) tvdbId = ext.tvdbId;
      } catch {
        // Keep the TMDB ID even if secondary external_ids call fails.
      }
    }
  }

  function handleTitleInputKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      void handleMetadataSearch();
    } else if (event.key === "Escape" && searchResults.length > 0) {
      searchResults = [];
    }
  }

  async function handleCancelJob(jobId: string): Promise<void> {
    try {
      await invoke<boolean>("cancel_job", { jobId });
      await refreshQueue();
    } catch {
      // Surface through pipeline events once workers are wired.
    }
  }

  function handleNamingConventionChange(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (value === "plex" || value === "jellyfin" || value === "plain") {
      updateSetting("namingConvention", value);
    }
  }

  function openSettingsAt(category: SettingsCategory): void {
    settingsCategory = category;
    settingsOpen = true;
  }

  function episodeCodeForRow(rowIndex: number): string {
    const ep = parsedStartEpisode + rowIndex;
    return `S${String(parsedSeason).padStart(2, "0")}E${String(ep).padStart(2, "0")}`;
  }

  function fileBaseName(path: string): string {
    const parts = path.split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function trackPillClass(track: StreamTrack): string {
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

  function dismissFirstRunModal(): void {
    firstRunModalOpen = false;
    updateSetting("firstRunSetupDone", true);
  }

  async function initialize(): Promise<void> {
    await loadPersistedSettings();
    const [detected] = await Promise.all([runCheckBinaries(), refreshQueue()]);

    if (detected) {
      const bothFound = detected.makemkv.found && detected.handbrake.found;
      if (bothFound) {
        if (!settings.firstRunSetupDone) {
          updateSetting("firstRunSetupDone", true);
        }
      } else if (!settings.firstRunSetupDone && settings.autoSetupOnFirstRun) {
        // Show the clean 1-step onboarding modal only when a CLI is missing
        firstRunModalOpen = true;
      }
    }
  }

  onMount(() => {
    void initialize();
  });
</script>

<div class="flex h-screen w-screen flex-col overflow-hidden bg-[#F8F7F4] text-[#111111]">
  <!-- Row 1: Single Lean Clay App Bar -->
  <header
    class="flex h-12 shrink-0 items-center justify-between gap-4 border-b border-[#E5E3DD] bg-white px-5"
  >
    <!-- Brand -->
    <div class="flex items-center gap-2.5">
      <svg class="h-6 w-6" viewBox="0 0 28 28" fill="none" aria-hidden="true">
        <path
          d="M4 19C4 11.268 8.477 6 14 6C19.523 6 24 11.268 24 19V22H4V19Z"
          fill="#58B0F9"
        />
        <path
          d="M7.5 19.5C7.5 13.701 10.41 9.5 14 9.5C17.59 9.5 20.5 13.701 20.5 19.5V22H7.5V19.5Z"
          fill="#3AD27E"
        />
        <path
          d="M10.5 20C10.5 16.41 12.067 13.5 14 13.5C15.933 13.5 17.5 16.41 17.5 20V22H10.5V20Z"
          fill="#F6BE4F"
        />
        <circle cx="14" cy="19.5" r="2.2" fill="#FF6AD5" />
      </svg>
      <span class="text-[16px] font-extrabold tracking-[-0.03em] text-[#111111]">
        RipDaddy
      </span>
    </div>

    <!-- Center: Clay Colored Workflow Mode Pills -->
    <div
      class="flex items-center gap-1.5"
      role="group"
      aria-label="Workflow mode"
    >
      <button
        type="button"
        onclick={() => (workflowMode = "ripOnly")}
        class={`rounded-full border px-3 py-1 text-xs font-semibold transition ${
          workflowMode === "ripOnly"
            ? "border-[#D84CB2] bg-[#FF6AD5] text-[#111111] shadow-[0_1px_2px_rgba(0,0,0,0.08)]"
            : "border-[#F1B6E2] bg-[#FFE8F8] text-[#4A2040] hover:bg-[#FFD6F3]"
        }`}
      >
        Rip Only
      </button>

      <button
        type="button"
        onclick={() => (workflowMode = "ripAndEncode")}
        class={`rounded-full border px-3 py-1 text-xs font-semibold transition ${
          workflowMode === "ripAndEncode"
            ? "border-[#24B465] bg-[#3AD27E] text-[#111111] shadow-[0_1px_2px_rgba(0,0,0,0.08)]"
            : "border-[#B3E8CB] bg-[#E3F9EC] text-[#17462B] hover:bg-[#CEF5DF]"
        }`}
      >
        Rip + Encode
      </button>

      <button
        type="button"
        onclick={() => (workflowMode = "encodeOnly")}
        class={`rounded-full border px-3 py-1 text-xs font-semibold transition ${
          workflowMode === "encodeOnly"
            ? "border-[#DDA12D] bg-[#F6BE4F] text-[#111111] shadow-[0_1px_2px_rgba(0,0,0,0.08)]"
            : "border-[#F1D89F] bg-[#FDF4DE] text-[#533B0B] hover:bg-[#FBEAC2]"
        }`}
      >
        Encode Only
      </button>
    </div>

    <!-- Right: Preview Toggle + Unified Categorized Settings Dropdown -->
    <div class="flex items-center gap-2">
      <button
        type="button"
        onclick={() => (previewOpen = !previewOpen)}
        aria-pressed={previewOpen}
        class={`flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-semibold transition ${
          previewOpen
            ? "border-[#3894E2] bg-[#DCEEFF] text-[#155CA2]"
            : "border-[#E5E3DD] bg-[#F8F7F4] text-[#52524E] hover:text-[#111111]"
        }`}
      >
        <svg
          class="h-3.5 w-3.5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          aria-hidden="true"
        >
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <line x1="15" y1="4" x2="15" y2="20" />
        </svg>
        <span>Preview</span>
      </button>

      <button
        type="button"
        onclick={() => {
          settingsCategory = !modeReady ? "binaries" : "general";
          settingsOpen = !settingsOpen;
        }}
        aria-expanded={settingsOpen}
        aria-haspopup="dialog"
        class={`flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-xs font-semibold transition ${
          settingsOpen
            ? "border-[#111111] bg-[#111111] text-white"
            : "border-[#DFDDD6] bg-white text-[#111111] hover:bg-[#F8F7F4]"
        }`}
      >
        <span
          class={`h-2 w-2 rounded-full ${
            checkingBinaries || installingBinary !== null
              ? "bg-[#B8B6AF]"
              : missingCliCount === 0
                ? "bg-[#3AD27E]"
                : modeReady
                  ? "bg-[#58B0F9]"
                  : "bg-[#F6BE4F]"
          }`}
        ></span>
        <span>Settings</span>
        <span class="text-[10px] opacity-70">▾</span>
      </button>
    </div>
  </header>

  <!-- Row 2: Compact Source, Preset & Primary Action Toolbar -->
  <div
    class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-[#E5E3DD] bg-[#F8F7F4] px-5 py-2"
  >
    <div class="flex flex-wrap items-center gap-3">
      {#if requiresMakeMkv}
        <div class="flex items-center gap-2">
          <label
            for="drive-select"
            class="text-[11px] font-semibold text-[#5C5B57]"
          >
            Drive
          </label>
          <select
            id="drive-select"
            bind:value={selectedDriveIndex}
            disabled={!modeReady || drives.length === 0}
            class="rounded-md border border-[#DFDDD6] bg-white px-2.5 py-1 text-xs font-medium text-[#111111] outline-none focus:border-[#111111] disabled:cursor-not-allowed disabled:bg-[#F2F0EA] disabled:text-[#8E8C85]"
          >
            {#if drives.length === 0}
              <option value={null}>No drives scanned</option>
            {:else}
              {#each drives as drive (drive.index)}
                <option value={drive.index}>
                  [{drive.index}] {drive.discName || drive.driveName}
                </option>
              {/each}
            {/if}
          </select>

          <button
            type="button"
            disabled={!modeReady}
            title={modeReady
              ? "Scan optical drives once MakeMKV parser is wired"
              : "Configure makemkvcon in Settings"}
            class="rounded-md border border-[#DFDDD6] bg-white px-2.5 py-1 text-xs font-semibold text-[#111111] transition hover:bg-[#EFECE6] disabled:cursor-not-allowed disabled:opacity-50"
          >
            Scan
          </button>
        </div>
      {:else}
        <button
          type="button"
          onclick={() => void pickLocalVideoFiles()}
          disabled={!modeReady}
          class="rounded-md bg-[#1D8BF8] px-3 py-1 text-xs font-semibold text-white transition hover:bg-[#137CE5] disabled:cursor-not-allowed disabled:bg-[#D8D6CF] disabled:text-[#787772]"
        >
          + Add Video Files
        </button>
      {/if}

      {#if requiresHandBrake}
        <div class="h-4 w-px bg-[#E5E3DD]"></div>
        <div class="flex flex-wrap items-center gap-2">
          <!-- Local vs Remote Delivery Pill Toggle -->
          <div
            class="inline-flex rounded-full border border-[#DFDDD6] bg-white p-0.5 text-[11px]"
            role="group"
            aria-label="Delivery target"
          >
            <button
              type="button"
              onclick={() =>
                selectQualityTier(settings.activeResolution, "local")}
              class={`rounded-full px-2.5 py-0.5 font-semibold transition ${
                settings.activeDelivery === "local"
                  ? "bg-[#3AD27E] text-[#111111]"
                  : "text-[#6B6A65] hover:text-[#111111]"
              }`}
            >
              Local
            </button>
            <button
              type="button"
              onclick={() =>
                selectQualityTier(settings.activeResolution, "remote")}
              class={`rounded-full px-2.5 py-0.5 font-semibold transition ${
                settings.activeDelivery === "remote"
                  ? "bg-[#58B0F9] text-[#111111]"
                  : "text-[#6B6A65] hover:text-[#111111]"
              }`}
            >
              Remote
            </button>
          </div>

          <!-- 480p / 720p / 1080p / 4K Resolution Selector -->
          <div
            class="inline-flex rounded-md border border-[#DFDDD6] bg-white p-0.5 font-mono text-[11px]"
            role="group"
            aria-label="Target resolution"
          >
            {#each RESOLUTION_TIERS as tier (tier.id)}
              <button
                type="button"
                onclick={() =>
                  selectQualityTier(tier.id, settings.activeDelivery)}
                class={`rounded px-2 py-0.5 font-semibold transition ${
                  settings.activeResolution === tier.id
                    ? "bg-[#111111] text-white"
                    : "text-[#52524E] hover:bg-[#F8F7F4] hover:text-[#111111]"
                }`}
              >
                {tier.shortLabel}
              </button>
            {/each}
          </div>

          <!-- Active Quality Summary Pill -> Jumps straight into Quality Matrix Editor -->
          {#if activeQualityProfile}
            <button
              type="button"
              onclick={() => openSettingsAt("encoding")}
              title="Customize this resolution/delivery profile in Settings"
              class="rounded-md border border-[#DFDDD6] bg-white px-2 py-1 font-mono text-[10px] font-semibold text-[#52524E] transition hover:border-[#111111] hover:text-[#111111]"
            >
              {shortEncoderLabel(activeQualityProfile.encoder)} · RF
              {activeQualityProfile.rfQuality.toFixed(0)} · .{outputContainer}
            </button>
          {/if}
        </div>
      {/if}

      <div class="h-4 w-px bg-[#E5E3DD]"></div>

      <button
        type="button"
        onclick={() => openSettingsAt("storage")}
        class="flex max-w-[260px] items-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-[11px] text-[#5C5B57] transition hover:border-[#E5E3DD] hover:bg-white hover:text-[#111111]"
        title={`Output folder: ${resolvedOutputDir}`}
      >
        <span class="font-semibold text-[#787772]">Output:</span>
        <span class="truncate font-mono text-[#111111]">{resolvedOutputDir}</span>
      </button>
    </div>

    <!-- Single Contextual Primary CTA + Missing CLI Shortcut -->
    <div class="flex items-center gap-2">
      {#if !modeReady && !checkingBinaries}
        <button
          type="button"
          onclick={() => openSettingsAt("binaries")}
          class="flex items-center gap-1.5 rounded-full border border-[#DDA12D] bg-[#FEF7E4] px-2.5 py-1 text-[11px] font-semibold text-[#5C4107] hover:bg-[#FCEEC8]"
        >
          <span class="h-1.5 w-1.5 rounded-full bg-[#E59819]"></span>
          <span>Setup CLIs</span>
        </button>
      {/if}

      <button
        type="button"
        disabled={!modeReady || selectedCount === 0}
        class="rounded-md bg-[#111111] px-3.5 py-1.5 text-xs font-semibold text-white transition hover:bg-[#2A2A2A] disabled:cursor-not-allowed disabled:bg-[#C9C7C0] disabled:text-[#6B6A65]"
      >
        {#if selectedCount > 0}
          Queue {selectedCount} Selected →
        {:else if workflowMode === "ripOnly"}
          Start Rip Queue →
        {:else if workflowMode === "encodeOnly"}
          Start Encode Queue →
        {:else}
          Start Pipeline →
        {/if}
      </button>
    </div>
  </div>

  <!-- Row 3: Compact IMDb / TVDB / TMDB Metadata Strip -->
  <section
    aria-label="Media titling and naming"
    class="relative shrink-0 space-y-2 border-b border-[#E5E3DD] bg-white px-5 py-2.5"
  >
    <div class="flex flex-wrap items-center gap-2">
      <!-- Movie / TV Toggle -->
      <div
        class="inline-flex rounded-md border border-[#E5E3DD] bg-[#F8F7F4] p-0.5 text-[11px]"
        role="group"
        aria-label="Media type"
      >
        <button
          type="button"
          onclick={() => (mediaKind = "movie")}
          class={`rounded px-2 py-0.5 font-semibold transition ${
            mediaKind === "movie"
              ? "bg-white text-[#111111] shadow-[0_1px_1px_rgba(0,0,0,0.06)]"
              : "text-[#6B6A65] hover:text-[#111111]"
          }`}
        >
          Movie
        </button>
        <button
          type="button"
          onclick={() => (mediaKind = "tvEpisode")}
          class={`rounded px-2 py-0.5 font-semibold transition ${
            mediaKind === "tvEpisode"
              ? "bg-white text-[#111111] shadow-[0_1px_1px_rgba(0,0,0,0.06)]"
              : "text-[#6B6A65] hover:text-[#111111]"
          }`}
        >
          TV
        </button>
      </div>

      <!-- Title + Clean -->
      <div class="flex min-w-[200px] flex-1 items-center gap-1">
        <label for="media-title" class="sr-only">Title</label>
        <input
          id="media-title"
          type="text"
          bind:value={mediaTitle}
          onkeydown={handleTitleInputKeydown}
          placeholder={mediaKind === "movie"
            ? "Movie title or disc label (e.g. THE_MATRIX_1999_DISC_1)"
            : "TV series title (e.g. Breaking Bad)"}
          class="min-w-0 flex-1 rounded-md border border-[#DFDDD6] bg-[#FAF9F6] px-2.5 py-1 text-xs text-[#111111] placeholder:text-[#9D9B94] outline-none focus:border-[#111111] focus:bg-white"
        />
        <button
          type="button"
          onclick={() => void handleCleanCurrentTitle()}
          title="Clean raw optical disc volume label"
          class="rounded-md border border-[#DFDDD6] bg-white px-2 py-1 text-[11px] font-semibold text-[#52524E] hover:bg-[#F8F7F4] hover:text-[#111111]"
        >
          Clean
        </button>
      </div>

      <!-- Year -->
      <label for="media-year" class="sr-only">Year</label>
      <input
        id="media-year"
        type="text"
        inputmode="numeric"
        bind:value={mediaYearInput}
        placeholder="Year"
        class="w-16 rounded-md border border-[#DFDDD6] bg-[#FAF9F6] px-2 py-1 font-mono text-xs text-[#111111] placeholder:text-[#9D9B94] outline-none focus:border-[#111111] focus:bg-white"
      />

      <!-- TV Season & Start Episode (inline auto-numbering for TV rows) -->
      {#if mediaKind === "tvEpisode"}
        <div class="flex items-center gap-1">
          <label for="tv-season" class="text-[11px] font-semibold text-[#787772]">
            S
          </label>
          <input
            id="tv-season"
            type="text"
            inputmode="numeric"
            bind:value={seasonInput}
            class="w-10 rounded-md border border-[#DFDDD6] bg-[#FAF9F6] px-1.5 py-1 text-center font-mono text-xs text-[#111111] outline-none focus:border-[#111111] focus:bg-white"
          />
          <label
            for="tv-start-ep"
            class="text-[11px] font-semibold text-[#787772]"
          >
            E
          </label>
          <input
            id="tv-start-ep"
            type="text"
            inputmode="numeric"
            bind:value={startEpisodeInput}
            title="Starting episode number (rows auto-increment)"
            class="w-10 rounded-md border border-[#DFDDD6] bg-[#FAF9F6] px-1.5 py-1 text-center font-mono text-xs text-[#111111] outline-none focus:border-[#111111] focus:bg-white"
          />
        </div>
      {/if}

      <!-- IMDb / TVDB / TMDB Compact ID Inputs -->
      <label for="imdb-id" class="sr-only">IMDb ID or URL</label>
      <input
        id="imdb-id"
        type="text"
        bind:value={imdbId}
        placeholder="IMDb tt..."
        class="w-28 rounded-md border border-[#DFDDD6] bg-[#FAF9F6] px-2 py-1 font-mono text-[11px] text-[#111111] placeholder:text-[#9D9B94] outline-none focus:border-[#111111] focus:bg-white"
      />

      <label for="tvdb-id" class="sr-only">TVDB ID or URL</label>
      <input
        id="tvdb-id"
        type="text"
        bind:value={tvdbId}
        placeholder="TVDB ID"
        class="w-24 rounded-md border border-[#DFDDD6] bg-[#FAF9F6] px-2 py-1 font-mono text-[11px] text-[#111111] placeholder:text-[#9D9B94] outline-none focus:border-[#111111] focus:bg-white"
      />

      <label for="tmdb-id" class="sr-only">TMDB ID or URL</label>
      <input
        id="tmdb-id"
        type="text"
        bind:value={tmdbId}
        placeholder="TMDB ID"
        class="w-24 rounded-md border border-[#DFDDD6] bg-[#FAF9F6] px-2 py-1 font-mono text-[11px] text-[#111111] placeholder:text-[#9D9B94] outline-none focus:border-[#111111] focus:bg-white"
      />

      <!-- Provider Lookup -->
      <div class="flex items-center gap-1">
        <label for="metadata-provider" class="sr-only">Metadata provider</label>
        <select
          id="metadata-provider"
          bind:value={metadataProvider}
          class="rounded-md border border-[#DFDDD6] bg-[#F8F7F4] px-2 py-1 text-[11px] font-semibold text-[#111111] outline-none focus:border-[#111111]"
        >
          <option value="imdb">IMDb</option>
          <option value="tvdb">TVDB</option>
          <option value="tmdb">TMDB</option>
          <option value="tvmaze">TVMaze (Free)</option>
        </select>
        <button
          type="button"
          onclick={() => void handleMetadataSearch()}
          disabled={searchingMetadata}
          class="rounded-md bg-[#1D8BF8] px-2.5 py-1 text-[11px] font-semibold text-white transition hover:bg-[#137CE5] disabled:opacity-50"
        >
          {searchingMetadata ? "..." : "Lookup"}
        </button>
      </div>
    </div>

    <!-- Live Clay Token Chips + Output Path Preview Line -->
    <div class="flex flex-wrap items-center justify-between gap-2 text-[11px]">
      <div class="flex min-w-0 flex-wrap items-center gap-1.5">
        <span
          class="text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
        >
          Naming:
        </span>
        {#each activeTokenChips as chip, i (`${chip}-${i}`)}
          <span
            class="inline-flex items-center rounded border border-[#B5DAFA] bg-[#DCEEFF] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#155CA2]"
          >
            {chip}
          </span>
        {/each}
        <span class="text-[#B8B6AF]">→</span>
        <code
          class="truncate font-mono text-[11px] text-[#3F3E3B]"
          title={outputFilenamePreview}
        >
          {outputFilenamePreview}
        </code>
      </div>

      <div class="flex shrink-0 items-center gap-2">
        {#if metadataLookupStatus}
          <button
            type="button"
            onclick={() => openSettingsAt("metadata")}
            class="rounded border border-[#E6C575] bg-[#FEF7E4] px-2 py-0.5 text-[10px] font-semibold text-[#5C4107] hover:bg-[#FCEEC8]"
          >
            {metadataLookupStatus}
          </button>
        {/if}
        <label for="naming-convention" class="sr-only">Naming convention</label>
        <select
          id="naming-convention"
          value={namingConvention}
          onchange={handleNamingConventionChange}
          class="rounded border border-[#DFDDD6] bg-[#F8F7F4] px-2 py-0.5 text-[10px] font-semibold text-[#111111]"
        >
          <option value="plex">Plex {"{imdb/tvdb}"}</option>
          <option value="jellyfin">Jellyfin [imdbid/tvdbid]</option>
          <option value="plain">Plain</option>
        </select>
      </div>
    </div>

    <!-- Live Search Results Popover -->
    {#if searchResults.length > 0}
      <div
        class="absolute top-full right-5 left-5 z-30 mt-1 max-h-80 overflow-y-auto rounded-xl border border-[#DFDDD6] bg-white shadow-[0_18px_45px_rgba(17,17,17,0.14)]"
      >
        <div
          class="flex items-center justify-between border-b border-[#ECEAE4] bg-[#F8F7F4] px-3.5 py-2 text-[11px]"
        >
          <span class="font-semibold text-[#111111]">
            Select a match ({searchResults.length})
          </span>
          <button
            type="button"
            onclick={() => (searchResults = [])}
            class="font-medium text-[#787772] hover:text-[#111111]"
          >
            Dismiss
          </button>
        </div>
        <div class="divide-y divide-[#EFECE6]">
          {#each searchResults as item, idx (`${item.title}-${item.ids.imdbId ?? item.ids.tvdbId ?? item.ids.tmdbId ?? idx}`)}
            <button
              type="button"
              onclick={() => void applyMetadataResult(item)}
              class="flex w-full items-start gap-3 px-3.5 py-2.5 text-left transition hover:bg-[#F0F7FF]"
            >
              {#if item.posterUrl}
                <img
                  src={item.posterUrl}
                  alt={item.title}
                  class="h-14 w-10 shrink-0 rounded border border-[#E5E3DD] object-cover"
                />
              {:else}
                <div
                  class="flex h-14 w-10 shrink-0 items-center justify-center rounded border border-[#E5E3DD] bg-[#F8F7F4] font-mono text-[10px] text-[#9A9891]"
                >
                  {item.kind === "movie" ? "MOV" : "TV"}
                </div>
              {/if}

              <div class="min-w-0 flex-1 space-y-1">
                <div class="flex flex-wrap items-center gap-1.5">
                  <span class="text-xs font-bold text-[#111111]">
                    {item.title}
                  </span>
                  {#if item.year}
                    <span class="font-mono text-[11px] text-[#5C5B57]">
                      ({item.year})
                    </span>
                  {/if}
                  <span
                    class="rounded-full border border-[#E5E3DD] bg-[#F8F7F4] px-1.5 py-0.5 text-[9px] font-bold uppercase text-[#5C5B57]"
                  >
                    {item.kind === "movie" ? "Movie" : "TV"}
                  </span>
                  {#if item.ids.imdbId}
                    <span
                      class="rounded border border-[#B5DAFA] bg-[#DCEEFF] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#155CA2]"
                    >
                      imdb:{item.ids.imdbId}
                    </span>
                  {/if}
                  {#if item.ids.tvdbId}
                    <span
                      class="rounded border border-[#B3E8CB] bg-[#E3F9EC] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#17462B]"
                    >
                      tvdb:{item.ids.tvdbId}
                    </span>
                  {/if}
                  {#if item.ids.tmdbId}
                    <span
                      class="rounded border border-[#F1D89F] bg-[#FDF4DE] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#533B0B]"
                    >
                      tmdb:{item.ids.tmdbId}
                    </span>
                  {/if}
                </div>
                {#if item.overview}
                  <p class="line-clamp-2 text-[11px] leading-snug text-[#6B6A65]">
                    {item.overview}
                  </p>
                {/if}
              </div>
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </section>

  <!-- Row 4: Full-Width Clay Data Table + Toggleable Right Preview Drawer -->
  <div class="flex min-h-0 flex-1 overflow-hidden">
    <main class="flex min-w-0 flex-1 flex-col overflow-hidden bg-white">
      <!-- Table Header -->
      {#if workflowMode === "encodeOnly"}
        <div
          class="grid shrink-0 grid-cols-12 items-center border-b border-[#E5E3DD] bg-[#F8F7F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
        >
          <div class="col-span-1 flex items-center">
            <input
              type="checkbox"
              checked={allRowsChecked}
              disabled={localSourceFiles.length === 0}
              onchange={toggleAllRows}
              aria-label="Select all video files"
              class="h-3.5 w-3.5 accent-[#111111]"
            />
          </div>
          <div class="col-span-5">Video File</div>
          <div class="col-span-4">Source Path</div>
          <div class="col-span-2 text-right">Actions</div>
        </div>
      {:else}
        <div
          class="grid shrink-0 grid-cols-12 items-center border-b border-[#E5E3DD] bg-[#F8F7F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
        >
          <div class="col-span-1 flex items-center">
            <input
              type="checkbox"
              checked={allRowsChecked}
              disabled={titles.length === 0}
              onchange={toggleAllRows}
              aria-label="Select all disc titles"
              class="h-3.5 w-3.5 accent-[#111111]"
            />
          </div>
          <div class="col-span-4">Title / Episode</div>
          <div class="col-span-1">Length</div>
          <div class="col-span-1">Size</div>
          <div class="col-span-4">Tracks (Video / Audio / Subtitles)</div>
          <div class="col-span-1 text-right">Inspect</div>
        </div>
      {/if}

      <!-- Scrollable Table Body -->
      <div class="flex-1 overflow-y-auto">
        {#if workflowMode === "encodeOnly"}
          {#if localSourceFiles.length === 0}
            <div
              class="flex h-full flex-col items-center justify-center px-6 py-16 text-center"
            >
              <div
                class="mb-3 flex h-11 w-11 items-center justify-center rounded-full border border-[#E5E3DD] bg-[#F8F7F4] text-[#5C5B57]"
              >
                <svg
                  class="h-5 w-5"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.6"
                  aria-hidden="true"
                >
                  <path d="M4 7a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7Z" />
                  <path d="m10 9 5 3-5 3V9Z" />
                </svg>
              </div>
              <h2 class="text-sm font-bold tracking-[-0.01em] text-[#111111]">
                Drop or add local video files to encode
              </h2>
              <p class="mt-1 max-w-md text-xs leading-relaxed text-[#787772]">
                Select existing <span class="font-mono text-[#111111]">.mkv</span>,
                <span class="font-mono text-[#111111]">.mp4</span>, or
                <span class="font-mono text-[#111111]">.m2ts</span> files to preview
                playback and transcode with
                <span class="font-mono text-[#111111]">HandBrakeCLI</span>.
              </p>
              <button
                type="button"
                onclick={() => void pickLocalVideoFiles()}
                disabled={!modeReady}
                class="mt-4 rounded-md bg-[#1D8BF8] px-4 py-2 text-xs font-semibold text-white transition hover:bg-[#137CE5] disabled:cursor-not-allowed disabled:bg-[#D8D6CF] disabled:text-[#787772]"
              >
                + Add Video Files
              </button>
            </div>
          {:else}
            <div class="divide-y divide-[#EFECE6]">
              {#each localSourceFiles as filePath, idx (filePath)}
                <div
                  class={`grid grid-cols-12 items-center gap-2 px-5 py-2.5 text-xs transition ${
                    inspectedLocalFile === filePath
                      ? "bg-[#F0F7FF]"
                      : "hover:bg-[#FAF9F6]"
                  }`}
                >
                  <div class="col-span-1 flex items-center">
                    <input
                      type="checkbox"
                      checked={checkedLocalFiles.includes(filePath)}
                      onchange={() => toggleLocalFileChecked(filePath)}
                      aria-label={`Select ${fileBaseName(filePath)}`}
                      class="h-3.5 w-3.5 accent-[#111111]"
                    />
                  </div>
                  <div class="col-span-5 flex min-w-0 items-center gap-2">
                    {#if mediaKind === "tvEpisode"}
                      <span
                        class="shrink-0 rounded border border-[#B5DAFA] bg-[#DCEEFF] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#155CA2]"
                      >
                        {episodeCodeForRow(idx)}
                      </span>
                    {/if}
                    <button
                      type="button"
                      onclick={() => inspectLocalFile(filePath)}
                      class="truncate text-left font-semibold text-[#111111] hover:text-[#1D8BF8]"
                    >
                      {fileBaseName(filePath)}
                    </button>
                  </div>
                  <div
                    class="col-span-4 truncate font-mono text-[11px] text-[#787772]"
                    title={filePath}
                  >
                    {filePath}
                  </div>
                  <div class="col-span-2 flex items-center justify-end gap-2">
                    <button
                      type="button"
                      onclick={() => inspectLocalFile(filePath)}
                      class="rounded border border-[#DFDDD6] bg-white px-2 py-0.5 text-[10px] font-semibold text-[#111111] hover:bg-[#F8F7F4]"
                    >
                      Preview
                    </button>
                    <button
                      type="button"
                      onclick={() => removeLocalSourceFile(filePath)}
                      class="text-[11px] font-medium text-[#787772] hover:text-red-600"
                    >
                      Remove
                    </button>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        {:else if titles.length === 0}
          <div
            class="flex h-full flex-col items-center justify-center px-6 py-16 text-center"
          >
            <div
              class="mb-3 flex h-11 w-11 items-center justify-center rounded-full border border-[#E5E3DD] bg-[#F8F7F4] text-[#5C5B57]"
            >
              <svg
                class="h-5 w-5"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.6"
                aria-hidden="true"
              >
                <circle cx="12" cy="12" r="9" />
                <circle cx="12" cy="12" r="2.5" />
              </svg>
            </div>
            <h2 class="text-sm font-bold tracking-[-0.01em] text-[#111111]">
              No disc titles loaded
            </h2>
            <p class="mt-1 max-w-md text-xs leading-relaxed text-[#787772]">
              Insert a DVD or Blu-ray and click
              <span class="font-semibold text-[#111111]">Scan</span> to populate
              titles, durations, chapters, and selectable video/audio/subtitle
              track pills (filtering titles shorter than
              <span class="font-mono text-[#111111]">{settings.minTitleLengthSeconds}s</span>).
            </p>
            {#if !modeReady}
              <button
                type="button"
                onclick={() => openSettingsAt("binaries")}
                class="mt-4 rounded-md border border-[#DDA12D] bg-[#FEF7E4] px-3.5 py-1.5 text-xs font-semibold text-[#5C4107] hover:bg-[#FCEEC8]"
              >
                Open Settings → External CLIs
              </button>
            {/if}
          </div>
        {:else}
          <div class="divide-y divide-[#EFECE6]">
            {#each titles as title, idx (title.titleId)}
              <div
                class={`grid grid-cols-12 items-center gap-2 px-5 py-2.5 text-xs transition ${
                  inspectedTitleId === title.titleId
                    ? "bg-[#F0F7FF]"
                    : "hover:bg-[#FAF9F6]"
                }`}
              >
                <div class="col-span-1 flex items-center">
                  <input
                    type="checkbox"
                    checked={checkedTitleIds.includes(title.titleId)}
                    onchange={() => toggleTitleChecked(title.titleId)}
                    aria-label={`Select title ${title.titleId}`}
                    class="h-3.5 w-3.5 accent-[#111111]"
                  />
                </div>
                <div class="col-span-4 flex min-w-0 items-center gap-2">
                  {#if mediaKind === "tvEpisode"}
                    <span
                      class="shrink-0 rounded border border-[#B5DAFA] bg-[#DCEEFF] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#155CA2]"
                    >
                      {episodeCodeForRow(idx)}
                    </span>
                  {/if}
                  <button
                    type="button"
                    onclick={() => inspectTitle(title.titleId)}
                    class="truncate text-left font-semibold text-[#111111] hover:text-[#1D8BF8]"
                  >
                    {title.name ?? `Title #${title.titleId}`}
                  </button>
                </div>
                <div class="col-span-1 font-mono text-[11px] text-[#52524E]">
                  {title.duration ?? "--:--"}
                </div>
                <div class="col-span-1 font-mono text-[11px] text-[#52524E]">
                  {title.sizeHuman ?? "--"}
                </div>
                <div class="col-span-4 flex flex-wrap items-center gap-1">
                  {#each title.streams.slice(0, 4) as track (track.streamId)}
                    <span
                      class={`rounded-full border px-2 py-0.5 text-[10px] font-semibold ${trackPillClass(track)}`}
                    >
                      {track.codecShort ?? track.trackType}
                    </span>
                  {/each}
                  {#if title.streams.length > 4}
                    <span class="font-mono text-[10px] text-[#787772]">
                      +{title.streams.length - 4}
                    </span>
                  {/if}
                </div>
                <div class="col-span-1 text-right">
                  <button
                    type="button"
                    onclick={() => inspectTitle(title.titleId)}
                    class="rounded border border-[#DFDDD6] bg-white px-2 py-0.5 text-[10px] font-semibold text-[#111111] hover:bg-[#F8F7F4]"
                  >
                    Inspect
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </main>

    <!-- Toggleable Right Preview & Inspector Drawer -->
    {#if previewOpen}
      <PreviewDrawer
        {workflowMode}
        selectedTitle={inspectedTitle}
        selectedLocalFile={inspectedLocalFile}
        {outputFilenamePreview}
        {namingConvention}
        {outputContainer}
        {activeQualityProfile}
        posterUrl={selectedPosterUrl}
        {mediaTitle}
        mediaYear={parsedYear}
        mediaOverview={selectedOverview}
        onClose={() => (previewOpen = false)}
        onEditQuality={() => openSettingsAt("encoding")}
      />
    {/if}
  </div>

  <!-- Row 5: Collapsible Bottom Queue Dock -->
  <QueueDock
    expanded={queueExpanded}
    jobs={queueJobs}
    onToggleExpanded={() => (queueExpanded = !queueExpanded)}
    onCancelJob={(jobId) => void handleCancelJob(jobId)}
  />

  <!-- Categorized Settings Dropdown -->
  <SettingsDropdown
    open={settingsOpen}
    activeCategory={settingsCategory}
    {binaries}
    {checkingBinaries}
    {installingBinary}
    {checkError}
    {setupOutcomes}
    {settings}
    onClose={() => (settingsOpen = false)}
    onSelectCategory={(cat) => (settingsCategory = cat)}
    onUpdateSetting={updateSetting}
    onUpdateQualityProfile={handleUpdateQualityProfile}
    onResetQualityProfiles={(key) => void handleResetQualityProfiles(key)}
    onOpenSetupWizard={() => (firstRunModalOpen = true)}
    onRecheckBinaries={() => void runCheckBinaries()}
    onAutoSetupAll={() => void runAutoSetupAll()}
    onInstallSingle={(kind) => void runInstallSingle(kind)}
    onBrowseBinary={(target) => void browseBinary(target)}
    onResetBinaryPath={(target) => void setCustomBinaryPath(target, null)}
    onBrowseDirectory={(target) => void browseDirectory(target)}
    onResetDirectory={(target) => updateSetting(target, null)}
    onBrowseCustomPreset={() => void browseCustomPresetFile()}
  />

  <!-- First-Run Onboarding Modal (auto-skipped when both CLIs are already installed) -->
  <FirstRunSetupModal
    open={firstRunModalOpen}
    {binaries}
    {checkingBinaries}
    {installingBinary}
    {checkError}
    {setupOutcomes}
    onAutoSetupAll={() => void runAutoSetupAll()}
    onInstallSingle={(kind) => void runInstallSingle(kind)}
    onBrowseBinary={(target) => void browseBinary(target)}
    onRecheck={() => void runCheckBinaries()}
    onFinish={dismissFirstRunModal}
  />
</div>
