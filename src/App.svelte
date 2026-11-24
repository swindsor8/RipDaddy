<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { Store } from "@tauri-apps/plugin-store";
  import FirstRunSetupModal from "./lib/FirstRunSetupModal.svelte";
  import FolderOrganizeModal from "./lib/FolderOrganizeModal.svelte";
  import PreviewDrawer from "./lib/PreviewDrawer.svelte";
  import QueueDock from "./lib/QueueDock.svelte";
  import RipDaddyLogo from "./lib/RipDaddyLogo.svelte";
  import SettingsDropdown from "./lib/SettingsDropdown.svelte";
  import {
    RESOLUTION_TIERS,
    defaultQualityProfileMap,
    qualityProfileKey,
    shortEncoderLabel,
    shortResolutionLabel,
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
    DiscScanResult,
    DiscTitle,
    EnqueueJobRequest,
    EpisodeInfo,
    EpisodePreviewSample,
    ExternalIds,
    FolderContentKind,
    FolderIdentifyResult,
    FolderOrganization,
    FolderOrganizationPref,
    IdentifiedVideoFile,
    MediaKind,
    MediaMetadata,
    MetadataProvider,
    MetadataProviderConfig,
    MetadataSearchQuery,
    NamingConvention,
    OpticalDrive,
    PipelineEvent,
    PipelineMode,
    PreviewFrame,
    QualityProfile,
    QueueJob,
    ResolutionTier,
    SettingsCategory,
    StreamTrack,
    TrickplayPreview,
    VideoMediaInfo,
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
    tvFolderOrganization: "ask",
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

  // Source selection, scanning & preview sample state
  let drives = $state<OpticalDrive[]>([]);
  let selectedDriveIndex = $state<number | null>(null);
  let activeDiscName = $state<string>("");
  let scanningDrives = $state(false);
  let scanningDisc = $state(false);
  let scanStatusMessage = $state<string | null>(null);
  let titles = $state<DiscTitle[]>([]);
  let checkedTitleIds = $state<number[]>([]);
  let inspectedTitleId = $state<number | null>(null);

  let localVideoItems = $state<IdentifiedVideoFile[]>([]);
  let checkedLocalFiles = $state<string[]>([]);
  let inspectedLocalFile = $state<string | null>(null);
  let sourceFolderPath = $state<string | null>(null);
  let detectedFolderKind = $state<FolderContentKind | null>(null);
  let sharedTvShow = $state<MediaMetadata | null>(null);
  let identifyingFolder = $state(false);
  let folderIdentifyStatus = $state<string | null>(null);
  let folderShowCandidates = $state<MediaMetadata[]>([]);
  let seriesEpisodeCatalog = $state<EpisodeInfo[]>([]);
  let organizeModalOpen = $state(false);

  // Auto-generated trickplay frame preview cache & state
  let trickplayCache = $state<Record<string, TrickplayPreview>>({});
  let loadingTrickplayFor = $state<string | null>(null);
  let trickplayError = $state<string | null>(null);
  let trickplayRequestSeq = 0;

  // Queue enqueue state
  let queueingJobs = $state(false);
  let queueActionError = $state<string | null>(null);

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

  const inspectedLocalItem = $derived(
    localVideoItems.find((item) => item.path === inspectedLocalFile) ?? null,
  );

  const totalLocalSizeBytes = $derived(
    localVideoItems.reduce((sum, item) => sum + (item.fileSizeBytes ?? 0), 0),
  );

  const activeTrickplay = $derived(
    inspectedLocalFile ? (trickplayCache[inspectedLocalFile] ?? null) : null,
  );

  const loadingActiveTrickplay = $derived(
    Boolean(inspectedLocalFile && loadingTrickplayFor === inspectedLocalFile),
  );

  const selectedCount = $derived(
    workflowMode === "encodeOnly"
      ? checkedLocalFiles.length
      : checkedTitleIds.length,
  );

  const allRowsChecked = $derived.by(() => {
    if (workflowMode === "encodeOnly") {
      return (
        localVideoItems.length > 0 &&
        checkedLocalFiles.length === localVideoItems.length
      );
    }
    return titles.length > 0 && checkedTitleIds.length === titles.length;
  });

  const resolvedOutputDir = $derived(
    settings.outputDir || binaries?.defaultPaths.outputDir || "Auto",
  );

  const resolvedTempDir = $derived(
    settings.tempDir || binaries?.defaultPaths.tempDir || "",
  );

  const activeQualityProfile = $derived.by(() => {
    const key = qualityProfileKey(
      settings.activeResolution,
      settings.activeDelivery,
    );
    return settings.qualityProfiles[key] ?? null;
  });

  function formatProviderTagFor(
    kind: MediaKind,
    ids: ExternalIds,
    convention: NamingConvention,
  ): string | null {
    if (convention === "plain") return null;
    const tvdb = ids.tvdbId?.trim() || null;
    const imdb = ids.imdbId?.trim() || null;
    const tmdb = ids.tmdbId?.trim() || null;
    if (kind === "movie") {
      if (convention === "plex") {
        if (imdb) return `{imdb-${imdb}}`;
        if (tmdb) return `{tmdb-${tmdb}}`;
        if (tvdb) return `{tvdb-${tvdb}}`;
      } else {
        if (imdb) return `[imdbid-${imdb}]`;
        if (tmdb) return `[tmdbid-${tmdb}]`;
        if (tvdb) return `[tvdbid-${tvdb}]`;
      }
      return null;
    }
    if (convention === "plex") {
      if (tvdb) return `{tvdb-${tvdb}}`;
      if (imdb) return `{imdb-${imdb}}`;
      if (tmdb) return `{tmdb-${tmdb}}`;
    } else {
      if (tvdb) return `[tvdbid-${tvdb}]`;
      if (imdb) return `[imdbid-${imdb}]`;
      if (tmdb) return `[tmdbid-${tmdb}]`;
    }
    return null;
  }

  const activeBatchKind = $derived.by<FolderContentKind>(() => {
    if (workflowMode === "encodeOnly" && localVideoItems.length > 0) {
      const selected = localVideoItems.filter((item) =>
        checkedLocalFiles.includes(item.path),
      );
      const list = selected.length > 0 ? selected : localVideoItems;
      const hasMovie = list.some((i) => i.mediaKind === "movie");
      const hasTv = list.some((i) => i.mediaKind === "tvEpisode");
      if (hasMovie && hasTv) return "mixed";
      if (hasMovie) return "movies";
      return "tvSeries";
    }
    return mediaKind === "tvEpisode" ? "tvSeries" : "movies";
  });

  const detectedBatchSeasons = $derived.by<number[]>(() => {
    const set = new Set<number>();
    if (workflowMode === "encodeOnly") {
      const selected = localVideoItems.filter((item) =>
        checkedLocalFiles.includes(item.path),
      );
      const source = selected.length > 0 ? selected : localVideoItems;
      for (const item of source) {
        if (item.mediaKind === "tvEpisode") {
          set.add(item.season ?? parsedSeason);
        }
      }
    } else if (mediaKind === "tvEpisode") {
      set.add(parsedSeason);
    }
    return Array.from(set).sort((a, b) => a - b);
  });

  const recommendedFolderOrg = $derived.by<FolderOrganization>(() => {
    const outDir = (
      settings.outputDir ||
      binaries?.defaultPaths.outputDir ||
      ""
    )
      .replace(/[\\/]+$/, "")
      .trim();
    const lastSeg = outDir.split(/[\\/]/).pop() ?? "";
    if (/^(?:season|series)\s*\d+$|^s\d{1,2}$/i.test(lastSeg.trim())) {
      return "flat";
    }
    if (activeBatchKind === "movies") {
      return namingConvention === "plain" ? "flat" : "showAndSeason";
    }
    const tvTitle = (sharedTvShow?.title || mediaTitle)
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, " ")
      .trim();
    const cleanSeg = lastSeg
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, " ")
      .trim();
    if (tvTitle.length >= 3 && cleanSeg.includes(tvTitle)) {
      return "seasonOnly";
    }
    if (namingConvention === "plain" && detectedBatchSeasons.length <= 1) {
      return "flat";
    }
    return "showAndSeason";
  });

  const effectivePreviewFolderOrg = $derived<FolderOrganization>(
    settings.tvFolderOrganization === "ask"
      ? recommendedFolderOrg
      : settings.tvFolderOrganization,
  );

  const modalSampleItems = $derived.by<EpisodePreviewSample[]>(() => {
    const samples: EpisodePreviewSample[] = [];
    const seenSeasons = new Set<number>();
    const tvIds: ExternalIds = sharedTvShow?.ids ?? {
      imdbId: imdbId.trim() || null,
      tvdbId: tvdbId.trim() || null,
      tmdbId: tmdbId.trim() || null,
    };
    const tvTag = formatProviderTagFor("tvEpisode", tvIds, namingConvention);
    const tvShowTitle = sharedTvShow?.title || mediaTitle.trim() || "Untitled Show";
    const tvShowYear = sharedTvShow?.year ?? parsedYear;

    if (workflowMode === "encodeOnly") {
      const ordered = localVideoItems.filter((item) =>
        checkedLocalFiles.includes(item.path),
      );
      const movies = ordered.filter((i) => i.mediaKind === "movie");
      const tvEps = ordered.filter((i) => i.mediaKind === "tvEpisode");

      // Add up to 2 movie samples (or 1 when mixed with TV)
      const maxMovieSamples = tvEps.length > 0 ? 1 : 3;
      for (const m of movies.slice(0, maxMovieSamples)) {
        const meta = m.movieMetadata;
        const mIds = meta?.ids ?? { imdbId: null, tvdbId: null, tmdbId: null };
        samples.push({
          kind: "movie",
          title: meta?.title || m.fileName.replace(/\.[^.]+$/, ""),
          year: meta?.year ?? null,
          providerTag: formatProviderTagFor("movie", mIds, namingConvention),
          season: null,
          episode: null,
          episodeTitle: null,
        });
      }

      // Add one sample per distinct TV season (up to 3 total)
      for (let i = 0; i < tvEps.length && samples.length < 3; i++) {
        const item = tvEps[i];
        const s = item.season ?? parsedSeason;
        const e = item.episode ?? parsedStartEpisode + i;
        if (!seenSeasons.has(s)) {
          seenSeasons.add(s);
          samples.push({
            kind: "tvEpisode",
            title: tvShowTitle,
            year: tvShowYear,
            providerTag: tvTag,
            season: s,
            episode: e,
            episodeTitle: item.episodeTitle,
          });
        }
      }
      if (tvEps.length > 1 && samples.length < 2) {
        const second = tvEps[1];
        samples.push({
          kind: "tvEpisode",
          title: tvShowTitle,
          year: tvShowYear,
          providerTag: tvTag,
          season: second.season ?? parsedSeason,
          episode: second.episode ?? parsedStartEpisode + 1,
          episodeTitle: second.episodeTitle,
        });
      }
    } else {
      samples.push({
        kind: mediaKind,
        title: mediaTitle.trim() || "Untitled",
        year: parsedYear,
        providerTag: formatProviderTagFor(
          mediaKind,
          {
            imdbId: imdbId.trim() || null,
            tvdbId: tvdbId.trim() || null,
            tmdbId: tmdbId.trim() || null,
          },
          namingConvention,
        ),
        season: mediaKind === "tvEpisode" ? parsedSeason : null,
        episode: mediaKind === "tvEpisode" ? parsedStartEpisode : null,
        episodeTitle: null,
      });
    }

    if (samples.length === 0) {
      samples.push({
        kind: mediaKind,
        title: mediaTitle.trim() || "Untitled",
        year: parsedYear,
        providerTag: tvTag,
        season: mediaKind === "tvEpisode" ? parsedSeason : null,
        episode: mediaKind === "tvEpisode" ? parsedStartEpisode : null,
        episodeTitle: null,
      });
    }
    return samples;
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
    const activeItem =
      workflowMode === "encodeOnly" ? inspectedLocalItem : null;
    const previewSeason =
      mediaKind === "tvEpisode"
        ? (activeItem?.season ?? parsedSeason)
        : null;
    const previewEpisode =
      mediaKind === "tvEpisode"
        ? (activeItem?.episode ?? parsedStartEpisode)
        : null;
    const previewEpisodeTitle =
      mediaKind === "tvEpisode" ? (activeItem?.episodeTitle ?? null) : null;

    const payload: MediaMetadata = {
      kind: mediaKind,
      title: mediaTitle,
      year: parsedYear,
      season: previewSeason,
      episode: previewEpisode,
      episodeTitle: previewEpisodeTitle,
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
    const folderOrg = effectivePreviewFolderOrg;

    let cancelled = false;
    void invoke<string>("preview_output_filename", {
      media: payload,
      convention,
      extension: ext,
      folderOrganization: folderOrg,
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
      settings.tvFolderOrganization =
        (await readStoreValue<FolderOrganizationPref>(
          "tvFolderOrganization",
        )) ?? "ask";
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

  function currentMetadataConfig(): MetadataProviderConfig {
    return {
      omdbApiKey: settings.omdbApiKey,
      tvdbApiKey: settings.tvdbApiKey,
      tmdbApiKey: settings.tmdbApiKey,
    };
  }

  function formatBytesHuman(bytes: number | null): string {
    if (bytes === null || bytes <= 0) return "--";
    const gb = bytes / (1024 * 1024 * 1024);
    if (gb >= 1) return `${gb.toFixed(gb >= 10 ? 1 : 2)} GB`;
    const mb = bytes / (1024 * 1024);
    return `${mb.toFixed(0)} MB`;
  }

  function formatDurationShort(seconds: number | null): string {
    if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) {
      return "";
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

  function sanitizeClientFilename(input: string): string {
    return input
      .replace(/[<>:"/\\|?*]/g, " ")
      .replace(/\s+/g, " ")
      .replace(/\.+$/, "")
      .trim();
  }

  function formatItemTargetFilename(
    item: IdentifiedVideoFile,
    rowIndex: number,
  ): string {
    const ext = outputContainer || "mkv";

    if (item.mediaKind === "movie") {
      const meta = item.movieMetadata;
      const rawTitle =
        meta?.title ||
        (inspectedLocalFile === item.path && mediaTitle.trim()
          ? mediaTitle.trim()
          : item.fileName.replace(/\.[^.]+$/, ""));
      const clean = sanitizeClientFilename(rawTitle) || "Untitled";
      const yr =
        meta?.year ?? (inspectedLocalFile === item.path ? parsedYear : null);
      const base = yr ? `${clean} (${yr})` : clean;
      const ids: ExternalIds = meta?.ids ?? {
        imdbId: inspectedLocalFile === item.path ? imdbId.trim() || null : null,
        tvdbId: null,
        tmdbId: inspectedLocalFile === item.path ? tmdbId.trim() || null : null,
      };
      const tag = formatProviderTagFor("movie", ids, namingConvention);
      const tagged = tag ? `${base} ${tag}` : base;
      if (effectivePreviewFolderOrg === "flat") {
        return `${tagged}.${ext}`;
      }
      return `${tagged}/${tagged}.${ext}`;
    }

    const tvTitle =
      sharedTvShow?.title ||
      (mediaKind === "tvEpisode" ? mediaTitle.trim() : "") ||
      item.fileName.replace(/\.[^.]+$/, "");
    const tvYear =
      sharedTvShow?.year ?? (mediaKind === "tvEpisode" ? parsedYear : null);
    const baseTitle = sanitizeClientFilename(tvTitle) || "Untitled";
    const titleWithYear = tvYear ? `${baseTitle} (${tvYear})` : baseTitle;
    const s = item.season ?? parsedSeason;
    const e = item.episode ?? parsedStartEpisode + rowIndex;
    const seasonPad = String(s).padStart(2, "0");
    const code = `S${seasonPad}E${String(e).padStart(2, "0")}`;
    const epTitle = item.episodeTitle
      ? sanitizeClientFilename(item.episodeTitle)
      : "";
    const file = epTitle
      ? `${titleWithYear} - ${code} - ${epTitle}.${ext}`
      : `${titleWithYear} - ${code}.${ext}`;
    if (effectivePreviewFolderOrg === "flat") {
      return file;
    }
    return `Season ${seasonPad}/${file}`;
  }

  function syncMetadataStripToItem(item: IdentifiedVideoFile): void {
    if (item.mediaKind === "movie") {
      mediaKind = "movie";
      if (item.movieMetadata) {
        mediaTitle = item.movieMetadata.title;
        mediaYearInput = item.movieMetadata.year
          ? String(item.movieMetadata.year)
          : "";
        imdbId = item.movieMetadata.ids.imdbId ?? "";
        tvdbId = item.movieMetadata.ids.tvdbId ?? "";
        tmdbId = item.movieMetadata.ids.tmdbId ?? "";
        selectedPosterUrl = item.movieMetadata.posterUrl;
        selectedOverview = item.movieMetadata.overview;
      } else {
        mediaTitle = item.needsTitle
          ? ""
          : item.fileName.replace(/\.[^.]+$/, "");
        mediaYearInput = "";
        imdbId = "";
        tvdbId = "";
        tmdbId = "";
        selectedPosterUrl = null;
        selectedOverview = null;
      }
    } else if (sharedTvShow) {
      mediaKind = "tvEpisode";
      mediaTitle = sharedTvShow.title;
      mediaYearInput = sharedTvShow.year ? String(sharedTvShow.year) : "";
      imdbId = sharedTvShow.ids.imdbId ?? "";
      tvdbId = sharedTvShow.ids.tvdbId ?? "";
      tmdbId = sharedTvShow.ids.tmdbId ?? "";
      selectedPosterUrl = sharedTvShow.posterUrl;
      selectedOverview = sharedTvShow.overview;
    } else {
      mediaKind = "tvEpisode";
    }
  }

  function toggleItemMediaKind(path: string): void {
    localVideoItems = localVideoItems.map((item) => {
      if (item.path !== path) return item;
      const nextKind: MediaKind =
        item.mediaKind === "movie" ? "tvEpisode" : "movie";
      return {
        ...item,
        mediaKind: nextKind,
        season: nextKind === "tvEpisode" ? (item.season ?? parsedSeason) : null,
        episode:
          nextKind === "tvEpisode"
            ? (item.episode ?? parsedStartEpisode)
            : null,
        needsTitle: nextKind === "movie" && !item.movieMetadata,
      };
    });
    const updated = localVideoItems.find((i) => i.path === path);
    if (updated && inspectedLocalFile === path) {
      syncMetadataStripToItem(updated);
    }
  }

  function applyEpisodeCatalogToItems(catalog: EpisodeInfo[]): void {
    seriesEpisodeCatalog = catalog;
    if (catalog.length === 0 || localVideoItems.length === 0) return;

    const byKey = new Map<string, EpisodeInfo>();
    for (const ep of catalog) {
      byKey.set(`${ep.season}:${ep.episode}`, ep);
    }

    localVideoItems = localVideoItems.map((item, idx) => {
      if (item.mediaKind !== "tvEpisode") return item;
      const s = item.season ?? parsedSeason;
      const e = item.episode ?? parsedStartEpisode + idx;
      const official = byKey.get(`${s}:${e}`);
      if (official) {
        return {
          ...item,
          season: s,
          episode: e,
          episodeTitle: official.title,
          matchedFromCatalog: true,
        };
      }
      return {
        ...item,
        matchedFromCatalog: false,
      };
    });
  }

  function applyFolderIdentifyResult(
    res: FolderIdentifyResult,
    replaceList: boolean,
  ): void {
    if (replaceList) {
      localVideoItems = res.files;
      checkedLocalFiles = res.files.map((f) => f.path);
    } else {
      const byPath = new Map(res.files.map((f) => [f.path, f]));
      const merged = localVideoItems.map((old) => byPath.get(old.path) ?? old);
      for (const f of res.files) {
        if (!merged.some((m) => m.path === f.path)) {
          merged.push(f);
        }
      }
      localVideoItems = merged;
      const newPaths = res.files
        .map((f) => f.path)
        .filter((p) => !checkedLocalFiles.includes(p));
      if (newPaths.length > 0) {
        checkedLocalFiles = [...checkedLocalFiles, ...newPaths];
      }
    }

    detectedFolderKind = res.detectedKind;
    sharedTvShow = res.matchedShow;
    folderShowCandidates = res.showCandidates;
    seriesEpisodeCatalog = res.episodesCatalog;
    folderIdentifyStatus = res.statusMessage;
    if (res.folderPath) {
      sourceFolderPath = res.folderPath;
    }

    if (res.providerUsed) {
      metadataProvider = res.providerUsed;
    }

    const firstSeason = res.files.find(
      (f) => f.mediaKind === "tvEpisode" && f.season !== null,
    )?.season;
    if (firstSeason !== undefined && firstSeason !== null) {
      seasonInput = String(firstSeason);
    }
    const firstEp = res.files.find(
      (f) => f.mediaKind === "tvEpisode" && f.episode !== null,
    )?.episode;
    if (firstEp !== undefined && firstEp !== null) {
      startEpisodeInput = String(firstEp);
    }

    if (res.files.length > 0) {
      const first = res.files[0];
      inspectedLocalFile = first.path;
      previewOpen = true;
      syncMetadataStripToItem(first);
      if (!res.matchedShow && res.inferredShowQuery && first.mediaKind === "tvEpisode") {
        mediaTitle = res.inferredShowQuery;
      }
    }
  }

  async function pickLocalVideoFolder(): Promise<void> {
    try {
      const selected = await open({
        title: "Select a TV season or video folder to identify & batch encode",
        directory: true,
        multiple: false,
      });
      if (typeof selected !== "string" || selected.trim().length === 0) return;

      identifyingFolder = true;
      queueActionError = null;
      folderIdentifyStatus = "Scanning folder and identifying show & episodes...";
      try {
        const res = await invoke<FolderIdentifyResult>("identify_video_folder", {
          folderPath: selected.trim(),
          config: currentMetadataConfig(),
        });
        applyFolderIdentifyResult(res, true);
      } catch (err) {
        folderIdentifyStatus = null;
        queueActionError = err instanceof Error ? err.message : String(err);
      } finally {
        identifyingFolder = false;
      }
    } catch (err) {
      checkError =
        err instanceof Error ? err.message : "Failed to open folder dialog.";
    }
  }

  async function handleAutoIdentifyLoadedFiles(): Promise<void> {
    if (localVideoItems.length === 0) return;
    identifyingFolder = true;
    queueActionError = null;
    folderIdentifyStatus = "Identifying show & episode titles...";
    try {
      const res = await invoke<FolderIdentifyResult>("identify_video_files", {
        filePaths: localVideoItems.map((item) => item.path),
        config: currentMetadataConfig(),
      });
      applyFolderIdentifyResult(res, true);
    } catch (err) {
      folderIdentifyStatus = null;
      queueActionError = err instanceof Error ? err.message : String(err);
    } finally {
      identifyingFolder = false;
    }
  }

  function updateItemSeasonOrEpisode(
    path: string,
    field: "season" | "episode",
    raw: string,
  ): void {
    const parsed = Number.parseInt(raw.trim(), 10);
    const num = Number.isFinite(parsed) && parsed >= 0 ? parsed : null;

    localVideoItems = localVideoItems.map((item) => {
      if (item.path !== path) return item;
      const nextSeason = field === "season" ? num : item.season;
      const nextEpisode = field === "episode" ? num : item.episode;
      let nextTitle = item.episodeTitle;
      let matched = false;
      if (nextSeason !== null && nextEpisode !== null) {
        const official = seriesEpisodeCatalog.find(
          (ep) => ep.season === nextSeason && ep.episode === nextEpisode,
        );
        if (official) {
          nextTitle = official.title;
          matched = true;
        }
      }
      return {
        ...item,
        season: nextSeason,
        episode: nextEpisode,
        episodeTitle: nextTitle,
        matchedFromCatalog: matched,
      };
    });
  }

  function clearLocalVideoItems(): void {
    localVideoItems = [];
    checkedLocalFiles = [];
    inspectedLocalFile = null;
    sourceFolderPath = null;
    detectedFolderKind = null;
    sharedTvShow = null;
    folderIdentifyStatus = null;
    folderShowCandidates = [];
    seriesEpisodeCatalog = [];
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
      const addedPaths: string[] = [];
      if (selected) {
        for (const item of selected) {
          if (
            item.trim().length > 0 &&
            !localVideoItems.some((existing) => existing.path === item)
          ) {
            addedPaths.push(item);
          }
        }
      }

      if (addedPaths.length > 0) {
        const newItems: IdentifiedVideoFile[] = addedPaths.map((p) => ({
          path: p,
          fileName: fileBaseName(p),
          fileSizeBytes: null,
          durationSeconds: null,
          mediaKind: mediaKind,
          season: null,
          episode: null,
          episodeTitle: null,
          movieMetadata: null,
          movieCandidates: [],
          matchedFromCatalog: false,
          needsTitle: false,
        }));
        localVideoItems = [...localVideoItems, ...newItems];
        checkedLocalFiles = [...checkedLocalFiles, ...addedPaths];
        inspectedLocalFile = addedPaths[0];
        previewOpen = true;
      }
    } catch (err) {
      checkError =
        err instanceof Error ? err.message : "Failed to pick video files.";
    }
  }

  function removeLocalSourceFile(path: string): void {
    localVideoItems = localVideoItems.filter((item) => item.path !== path);
    checkedLocalFiles = checkedLocalFiles.filter((item) => item !== path);
    if (path in trickplayCache) {
      const next = { ...trickplayCache };
      delete next[path];
      trickplayCache = next;
    }
    if (inspectedLocalFile === path) {
      inspectedLocalFile = localVideoItems[0]?.path ?? null;
    }
    if (localVideoItems.length === 0) {
      folderIdentifyStatus = null;
      folderShowCandidates = [];
    }
  }

  function toggleAllRows(): void {
    if (workflowMode === "encodeOnly") {
      checkedLocalFiles = allRowsChecked
        ? []
        : localVideoItems.map((item) => item.path);
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
    const found = localVideoItems.find((i) => i.path === path);
    if (found && detectedFolderKind !== null) {
      syncMetadataStripToItem(found);
    }
  }

  function inspectTitle(titleId: number): void {
    inspectedTitleId = titleId;
    previewOpen = true;
  }

  async function handleScanDisc(driveIndex: number): Promise<void> {
    scanningDisc = true;
    scanStatusMessage = `Scanning disc in drive #${driveIndex}...`;
    titles = [];
    checkedTitleIds = [];
    inspectedTitleId = null;

    try {
      const scan = await invoke<DiscScanResult>("scan_disc", {
        driveIndex,
        minLengthSeconds: settings.minTitleLengthSeconds,
        config: currentBinaryConfig(),
      });
      titles = scan.titles;
      activeDiscName = scan.discName;

      if (scan.titles.length > 0) {
        if (mediaKind === "tvEpisode") {
          checkedTitleIds = scan.titles.map((t) => t.titleId);
        } else {
          // Auto-select the longest title (typically the main feature)
          let longest = scan.titles[0];
          for (const t of scan.titles) {
            if ((t.durationSeconds ?? 0) > (longest.durationSeconds ?? 0)) {
              longest = t;
            }
          }
          checkedTitleIds = [longest.titleId];
        }
        inspectedTitleId = checkedTitleIds[0] ?? scan.titles[0].titleId;
        scanStatusMessage = null;
      } else {
        scanStatusMessage = `No titles >= ${settings.minTitleLengthSeconds}s found on drive #${driveIndex}.`;
      }

      // Auto-populate the Metadata Strip from the disc label if blank
      if (scan.discName && !mediaTitle.trim()) {
        try {
          const guess = await invoke<DiscLabelGuess>("clean_disc_label", {
            rawLabel: scan.discName,
          });
          if (guess.cleanedTitle) mediaTitle = guess.cleanedTitle;
          if (guess.guessedYear) mediaYearInput = String(guess.guessedYear);
          if (guess.guessedSeason) {
            mediaKind = "tvEpisode";
            seasonInput = String(guess.guessedSeason);
            checkedTitleIds = scan.titles.map((t) => t.titleId);
          }
        } catch {
          mediaTitle = scan.discName;
        }
      }
    } catch (err) {
      scanStatusMessage = err instanceof Error ? err.message : String(err);
    } finally {
      scanningDisc = false;
    }
  }

  async function handleScanDrives(): Promise<void> {
    scanningDrives = true;
    scanStatusMessage = "Scanning optical drives...";
    try {
      const found = await invoke<OpticalDrive[]>("scan_drives", {
        config: currentBinaryConfig(),
      });
      drives = found;

      if (found.length === 0) {
        selectedDriveIndex = null;
        titles = [];
        checkedTitleIds = [];
        scanStatusMessage =
          "No usable optical drives detected. Insert a disc and try again.";
        return;
      }

      const withDisc =
        found.find((d) => d.discName.trim().length > 0 || d.visible === 2) ??
        found[0];
      selectedDriveIndex = withDisc.index;
      activeDiscName = withDisc.discName;

      if (withDisc.discName.trim().length > 0 || withDisc.visible === 2) {
        await handleScanDisc(withDisc.index);
      } else {
        titles = [];
        checkedTitleIds = [];
        scanStatusMessage = `Detected ${found.length} drive(s), but no disc is inserted.`;
      }
    } catch (err) {
      scanStatusMessage = err instanceof Error ? err.message : String(err);
    } finally {
      scanningDrives = false;
    }
  }

  function formatTimestampLabel(seconds: number): string {
    const total = Math.max(0, Math.round(seconds));
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    if (h > 0) {
      return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    }
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function computeAspectLabel(w: number, h: number): string {
    const ratio = w / h;
    const named: [number, string][] = [
      [4 / 3, "4:3"],
      [3 / 2, "3:2"],
      [16 / 10, "16:10"],
      [16 / 9, "16:9"],
      [1.85, "1.85:1"],
      [2.35, "2.35:1"],
      [2.39, "2.39:1"],
    ];
    for (const [target, label] of named) {
      if (Math.abs(ratio - target) <= 0.025) return label;
    }
    return `${ratio.toFixed(2)}:1`;
  }

  async function extractBrowserFallbackFrames(
    filePath: string,
    baseInfo: VideoMediaInfo,
    frameCount = 8,
  ): Promise<TrickplayPreview | null> {
    let videoUrl: string;
    try {
      videoUrl = convertFileSrc(filePath);
    } catch {
      return null;
    }

    const video = document.createElement("video");
    video.muted = true;
    video.preload = "auto";
    video.crossOrigin = "anonymous";
    video.src = videoUrl;

    try {
      await new Promise<void>((resolve, reject) => {
        const timer = window.setTimeout(
          () => reject(new Error("Timed out loading video metadata")),
          6000,
        );
        video.onloadedmetadata = () => {
          window.clearTimeout(timer);
          resolve();
        };
        video.onerror = () => {
          window.clearTimeout(timer);
          reject(new Error("Browser codec unsupported"));
        };
      });

      const w = video.videoWidth;
      const h = video.videoHeight;
      const dur = Number.isFinite(video.duration) ? video.duration : 0;
      if (w <= 0 || h <= 0) return null;

      const timestamps: number[] = [];
      if (dur <= 1) {
        timestamps.push(0);
      } else {
        const start = dur * 0.08;
        const end = dur * 0.92;
        const span = Math.max(0.5, end - start);
        for (let i = 0; i < frameCount; i++) {
          timestamps.push(start + (span * i) / Math.max(1, frameCount - 1));
        }
      }

      const scale = Math.min(1, 1280 / w);
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(2, Math.round(w * scale));
      canvas.height = Math.max(2, Math.round(h * scale));
      const ctx = canvas.getContext("2d");
      if (!ctx) return null;

      const frames: PreviewFrame[] = [];
      for (let i = 0; i < timestamps.length; i++) {
        const ts = timestamps[i];
        await new Promise<void>((resolve) => {
          const timer = window.setTimeout(() => resolve(), 2500);
          video.onseeked = () => {
            window.clearTimeout(timer);
            resolve();
          };
          video.currentTime = ts;
        });
        ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
        const dataUrl = canvas.toDataURL("image/jpeg", 0.85);
        frames.push({
          index: i,
          timestampSeconds: ts,
          timestampLabel: formatTimestampLabel(ts),
          path: dataUrl,
        });
      }

      const mergedInfo: VideoMediaInfo = {
        ...baseInfo,
        width: baseInfo.width ?? w,
        height: baseInfo.height ?? h,
        displayWidth: baseInfo.displayWidth ?? w,
        displayHeight: baseInfo.displayHeight ?? h,
        displayAspectRatio:
          baseInfo.displayAspectRatio ?? computeAspectLabel(w, h),
        durationSeconds: baseInfo.durationSeconds ?? (dur > 0 ? dur : null),
      };

      return {
        sourcePath: filePath,
        mediaInfo: mergedInfo,
        frames,
        extractor: "browserCanvas",
        cached: false,
        note: null,
      };
    } catch {
      return null;
    } finally {
      video.removeAttribute("src");
      video.load();
    }
  }

  async function loadTrickplayPreview(
    filePath: string,
    forceRegenerate = false,
  ): Promise<void> {
    if (!forceRegenerate && trickplayCache[filePath]) {
      trickplayError = null;
      return;
    }

    const reqId = ++trickplayRequestSeq;
    loadingTrickplayFor = filePath;
    trickplayError = null;

    try {
      const effectiveTemp = resolvedTempDir || null;
      let result = await invoke<TrickplayPreview>(
        "generate_trickplay_preview",
        {
          inputPath: filePath,
          tempDir: effectiveTemp,
          frameCount: 8,
          forceRegenerate,
          config: currentBinaryConfig(),
        },
      );

      if (reqId !== trickplayRequestSeq) return;

      if (result.frames.length === 0 && result.extractor === "browserFallback") {
        const fallback = await extractBrowserFallbackFrames(
          filePath,
          result.mediaInfo,
          8,
        );
        if (reqId !== trickplayRequestSeq) return;
        if (fallback && fallback.frames.length > 0) {
          result = fallback;
        }
      }

      trickplayCache = {
        ...trickplayCache,
        [filePath]: result,
      };
    } catch (err) {
      if (reqId === trickplayRequestSeq) {
        trickplayError = err instanceof Error ? err.message : String(err);
      }
    } finally {
      if (reqId === trickplayRequestSeq) {
        loadingTrickplayFor = null;
      }
    }
  }

  // Automatically generate trickplay preview frames whenever a local video file is selected
  $effect(() => {
    const file = inspectedLocalFile;
    const mode = workflowMode;
    if (mode === "encodeOnly" && file) {
      void loadTrickplayPreview(file, false);
    }
  });

  function buildJobMetadata(
    rowIndex: number,
    fallbackTitle: string,
    totalSelected: number,
    rowOverride?: {
      season: number | null;
      episode: number | null;
      episodeTitle: string | null;
    },
  ): MediaMetadata {
    const trimmed = mediaTitle.trim();
    let chosenTitle: string;
    if (mediaKind === "tvEpisode") {
      chosenTitle = trimmed || fallbackTitle;
    } else if (!trimmed) {
      chosenTitle = fallbackTitle;
    } else if (totalSelected > 1 && rowIndex > 0) {
      chosenTitle = `${trimmed} - ${fallbackTitle}`;
    } else {
      chosenTitle = trimmed;
    }
    return {
      kind: mediaKind,
      title: chosenTitle,
      year: parsedYear,
      season:
        mediaKind === "tvEpisode"
          ? (rowOverride?.season ?? parsedSeason)
          : null,
      episode:
        mediaKind === "tvEpisode"
          ? (rowOverride?.episode ?? parsedStartEpisode + rowIndex)
          : null,
      episodeTitle:
        mediaKind === "tvEpisode" ? (rowOverride?.episodeTitle ?? null) : null,
      ids: {
        imdbId: imdbId.trim() || null,
        tvdbId: tvdbId.trim() || null,
        tmdbId: tmdbId.trim() || null,
      },
      overview: selectedOverview,
      posterUrl: selectedPosterUrl,
    };
  }

  function handleRequestEnqueue(): void {
    if (!modeReady || selectedCount === 0 || queueingJobs) return;
    const shouldPrompt =
      settings.tvFolderOrganization === "ask" &&
      (activeBatchKind === "tvSeries" ||
        activeBatchKind === "mixed" ||
        (activeBatchKind === "movies" &&
          (selectedCount > 1 || workflowMode === "encodeOnly")));
    if (shouldPrompt) {
      organizeModalOpen = true;
      return;
    }
    const org: FolderOrganization | null =
      settings.tvFolderOrganization !== "ask"
        ? settings.tvFolderOrganization
        : null;
    void executeEnqueueSelected(org);
  }

  function handleConfirmFolderOrganization(
    org: FolderOrganization,
    saveAsDefault: boolean,
  ): void {
    organizeModalOpen = false;
    if (saveAsDefault) {
      updateSetting("tvFolderOrganization", org);
    }
    void executeEnqueueSelected(org);
  }

  async function executeEnqueueSelected(
    folderOrg: FolderOrganization | null,
  ): Promise<void> {
    if (!modeReady || selectedCount === 0) return;
    queueingJobs = true;
    queueActionError = null;

    try {
      const requests: EnqueueJobRequest[] = [];
      const activeDrive = drives.find((d) => d.index === selectedDriveIndex);
      const effectiveOutputDir =
        settings.outputDir || binaries?.defaultPaths.outputDir || "";
      const effectiveTempDir =
        settings.tempDir || binaries?.defaultPaths.tempDir || "";

      if (workflowMode === "encodeOnly") {
        const ordered = localVideoItems.filter((item) =>
          checkedLocalFiles.includes(item.path),
        );
        ordered.forEach((item, idx) => {
          const base = item.fileName.replace(/\.[^.]+$/, "");

          if (item.mediaKind === "movie") {
            const meta: MediaMetadata = item.movieMetadata
              ? { ...item.movieMetadata, kind: "movie" }
              : {
                  kind: "movie",
                  title:
                    inspectedLocalFile === item.path && mediaTitle.trim()
                      ? mediaTitle.trim()
                      : ordered.length === 1 && mediaTitle.trim()
                        ? mediaTitle.trim()
                        : base,
                  year:
                    inspectedLocalFile === item.path || ordered.length === 1
                      ? parsedYear
                      : null,
                  season: null,
                  episode: null,
                  episodeTitle: null,
                  ids:
                    inspectedLocalFile === item.path || ordered.length === 1
                      ? {
                          imdbId: imdbId.trim() || null,
                          tvdbId: tvdbId.trim() || null,
                          tmdbId: tmdbId.trim() || null,
                        }
                      : { imdbId: null, tvdbId: null, tmdbId: null },
                  overview: null,
                  posterUrl: null,
                };
            const titleLabel = meta.year
              ? `${meta.title} (${meta.year})`
              : meta.title;
            requests.push({
              mode: "encodeOnly",
              discName: "Local Movie",
              driveIndex: null,
              driveDevicePath: null,
              titleId: null,
              sourceFilePath: item.path,
              titleName: titleLabel,
              qualityProfile: activeQualityProfile,
              customPresetFile: settings.customPresetFile,
              customPresetName: settings.defaultPreset,
              tempDir: effectiveTempDir,
              outputDir: effectiveOutputDir,
              keepTempMkv: settings.keepTempMkv,
              autoEjectDisc: false,
              namingConvention,
              folderOrganization: folderOrg,
              mediaMetadata: meta,
            });
          } else {
            const tvTitle =
              sharedTvShow?.title || mediaTitle.trim() || base;
            const tvYear = sharedTvShow?.year ?? parsedYear;
            const tvIds: ExternalIds = sharedTvShow?.ids ?? {
              imdbId: imdbId.trim() || null,
              tvdbId: tvdbId.trim() || null,
              tmdbId: tmdbId.trim() || null,
            };
            const epSeason = item.season ?? parsedSeason;
            const epNum = item.episode ?? parsedStartEpisode + idx;
            const epCode = `S${String(epSeason).padStart(2, "0")}E${String(epNum).padStart(2, "0")}`;
            const titleLabel = item.episodeTitle
              ? `${tvTitle} ${epCode} - ${item.episodeTitle}`
              : `${tvTitle} ${epCode}`;
            requests.push({
              mode: "encodeOnly",
              discName: "Local File",
              driveIndex: null,
              driveDevicePath: null,
              titleId: null,
              sourceFilePath: item.path,
              titleName: titleLabel,
              qualityProfile: activeQualityProfile,
              customPresetFile: settings.customPresetFile,
              customPresetName: settings.defaultPreset,
              tempDir: effectiveTempDir,
              outputDir: effectiveOutputDir,
              keepTempMkv: settings.keepTempMkv,
              autoEjectDisc: false,
              namingConvention,
              folderOrganization: folderOrg,
              mediaMetadata: {
                kind: "tvEpisode",
                title: tvTitle,
                year: tvYear,
                season: epSeason,
                episode: epNum,
                episodeTitle: item.episodeTitle,
                ids: tvIds,
                overview: sharedTvShow?.overview ?? selectedOverview,
                posterUrl: sharedTvShow?.posterUrl ?? selectedPosterUrl,
              },
            });
          }
        });
      } else {
        const ordered = titles.filter((t) =>
          checkedTitleIds.includes(t.titleId),
        );
        ordered.forEach((title, idx) => {
          const fallback = title.name ?? `Title_${title.titleId}`;
          const titleLabel =
            mediaTitle.trim() && mediaKind === "tvEpisode"
              ? `${mediaTitle.trim()} ${episodeCodeForRow(idx)}`
              : ordered.length === 1 && mediaTitle.trim()
                ? mediaTitle.trim()
                : fallback;
          requests.push({
            mode: workflowMode,
            discName: activeDiscName || activeDrive?.discName || "Optical Disc",
            driveIndex: selectedDriveIndex,
            driveDevicePath: activeDrive?.devicePath ?? null,
            titleId: title.titleId,
            sourceFilePath: null,
            titleName: titleLabel,
            qualityProfile:
              workflowMode === "ripOnly" ? null : activeQualityProfile,
            customPresetFile:
              workflowMode === "ripOnly" ? null : settings.customPresetFile,
            customPresetName:
              workflowMode === "ripOnly" ? null : settings.defaultPreset,
            tempDir: effectiveTempDir,
            outputDir: effectiveOutputDir,
            keepTempMkv: settings.keepTempMkv,
            autoEjectDisc: settings.autoEjectDisc && idx === ordered.length - 1,
            namingConvention,
            folderOrganization: folderOrg,
            mediaMetadata: buildJobMetadata(idx, fallback, ordered.length),
          });
        });
      }

      const added = await invoke<QueueJob[]>("enqueue_jobs", {
        requests,
        config: currentBinaryConfig(),
      });

      const merged = [...queueJobs];
      for (const job of added) {
        const existingIdx = merged.findIndex((j) => j.id === job.id);
        if (existingIdx >= 0) {
          merged[existingIdx] = job;
        } else {
          merged.push(job);
        }
      }
      queueJobs = merged;
      queueExpanded = true;
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
    } finally {
      queueingJobs = false;
    }
  }

  async function handleClearFinished(): Promise<void> {
    try {
      queueJobs = await invoke<QueueJob[]>("clear_finished_jobs");
    } catch {
      // Ignore outside Tauri runtime.
    }
  }

  function applyPipelineEvent(event: PipelineEvent): void {
    switch (event.type) {
      case "jobUpdated": {
        const idx = queueJobs.findIndex((j) => j.id === event.job.id);
        if (idx >= 0) {
          queueJobs[idx] = event.job;
        } else {
          queueJobs = [...queueJobs, event.job];
        }
        break;
      }
      case "progress": {
        const idx = queueJobs.findIndex((j) => j.id === event.jobId);
        if (idx >= 0) {
          queueJobs[idx] = {
            ...queueJobs[idx],
            progress: event.progress,
          };
        }
        break;
      }
      case "stageChanged": {
        const idx = queueJobs.findIndex((j) => j.id === event.jobId);
        if (idx >= 0) {
          queueJobs[idx] = {
            ...queueJobs[idx],
            stage: event.to,
            progress: {
              ...queueJobs[idx].progress,
              stage: event.to,
            },
          };
        }
        break;
      }
      case "logLine":
      case "tempCleaned":
        break;
    }
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
      const results = await invoke<MediaMetadata[]>("search_metadata", {
        query,
        config: currentMetadataConfig(),
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
        const ext = await invoke<ExternalIds>("fetch_tmdb_external_ids", {
          kind: item.kind,
          tmdbId: item.ids.tmdbId,
          config: currentMetadataConfig(),
        });
        if (ext.imdbId) imdbId = ext.imdbId;
        if (ext.tvdbId) tvdbId = ext.tvdbId;
      } catch {
        // Keep the TMDB ID even if secondary external_ids call fails.
      }
    }

    // When in Encode Only with a specific Movie row selected, assign this match to THAT row
    if (
      workflowMode === "encodeOnly" &&
      inspectedLocalFile &&
      item.kind === "movie" &&
      localVideoItems.some((f) => f.path === inspectedLocalFile)
    ) {
      const hydrated: MediaMetadata = {
        ...item,
        ids: {
          imdbId: imdbId.trim() || null,
          tvdbId: tvdbId.trim() || null,
          tmdbId: tmdbId.trim() || null,
        },
      };
      localVideoItems = localVideoItems.map((row) =>
        row.path === inspectedLocalFile
          ? {
              ...row,
              mediaKind: "movie",
              movieMetadata: hydrated,
              matchedFromCatalog: true,
              needsTitle: false,
            }
          : row,
      );
      return;
    }

    // When picking a TV show with local files loaded, update shared TV identity,
    // fetch official episode titles, and re-apply uniform episode naming across TV rows.
    if (item.kind === "tvEpisode") {
      sharedTvShow = {
        ...item,
        ids: {
          imdbId: imdbId.trim() || null,
          tvdbId: tvdbId.trim() || null,
          tmdbId: tmdbId.trim() || null,
        },
      };
      const tvRows = localVideoItems.filter((f) => f.mediaKind === "tvEpisode");
      if (tvRows.length > 0) {
        try {
          const seasonsHint = Array.from(
            new Set(
              tvRows
                .map((f) => f.season)
                .filter((s): s is number => s !== null),
            ),
          );
          const [prov, episodes] = await invoke<
            [MetadataProvider, EpisodeInfo[]]
          >("fetch_series_episodes", {
            title: item.title,
            ids: sharedTvShow.ids,
            seasonsHint: seasonsHint.length > 0 ? seasonsHint : [parsedSeason],
            config: currentMetadataConfig(),
          });
          applyEpisodeCatalogToItems(episodes);
          const matchedCount = localVideoItems.filter(
            (f) => f.mediaKind === "tvEpisode" && f.matchedFromCatalog,
          ).length;
          folderIdentifyStatus = `Matched "${item.title}" via ${prov.toUpperCase()} · ${matchedCount}/${tvRows.length} episodes matched`;
        } catch {
          // Keep user's show selection even if episode catalog lookup fails.
        }
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
    let unlisten: UnlistenFn | null = null;
    void listen<PipelineEvent>("pipeline-event", (evt) => {
      applyPipelineEvent(evt.payload);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        // Ignore outside Tauri runtime.
      });

    void initialize();

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  });
</script>

<div class="flex h-screen w-screen flex-col overflow-hidden bg-[#F8F7F4] text-[#111111]">
  <!-- Row 1: Single Lean Clay App Bar -->
  <header
    class="flex h-12 shrink-0 items-center justify-between gap-4 border-b border-[#E5E3DD] bg-white px-5"
  >
    <!-- Brand -->
    <div class="flex items-center gap-2.5">
      <RipDaddyLogo class="h-8 w-8" alt="" />
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
            onchange={() => {
              if (selectedDriveIndex !== null) {
                void handleScanDisc(selectedDriveIndex);
              }
            }}
            disabled={!modeReady || drives.length === 0 || scanningDisc}
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
            onclick={() => void handleScanDrives()}
            disabled={!modeReady || scanningDrives || scanningDisc}
            title={modeReady
              ? "Scan connected optical drives and disc titles via makemkvcon"
              : "Configure makemkvcon in Settings"}
            class="rounded-md border border-[#DFDDD6] bg-white px-2.5 py-1 text-xs font-semibold text-[#111111] transition hover:bg-[#EFECE6] disabled:cursor-not-allowed disabled:opacity-50"
          >
            {scanningDrives || scanningDisc ? "Scanning..." : "Scan"}
          </button>
        </div>
      {:else}
        <div class="flex items-center gap-1.5">
          <button
            type="button"
            onclick={() => void pickLocalVideoFolder()}
            disabled={!modeReady || identifyingFolder}
            title="Pick a TV season or video folder to auto-identify show + episodes and batch encode"
            class="rounded-md bg-[#1D8BF8] px-3 py-1 text-xs font-semibold text-white transition hover:bg-[#137CE5] disabled:cursor-not-allowed disabled:bg-[#D8D6CF] disabled:text-[#787772]"
          >
            {identifyingFolder ? "Identifying..." : "+ Add Folder"}
          </button>
          <button
            type="button"
            onclick={() => void pickLocalVideoFiles()}
            disabled={!modeReady || identifyingFolder}
            class="rounded-md border border-[#DFDDD6] bg-white px-2.5 py-1 text-xs font-semibold text-[#111111] transition hover:bg-[#EFECE6] disabled:cursor-not-allowed disabled:opacity-50"
          >
            + Add Files
          </button>
          {#if localVideoItems.length > 0}
            <button
              type="button"
              onclick={() => void handleAutoIdentifyLoadedFiles()}
              disabled={identifyingFolder}
              title="Auto-identify show and episode numbers via TVDB, TMDB, or free TVMaze"
              class="rounded-md border border-[#B5DAFA] bg-[#DCEEFF] px-2.5 py-1 text-xs font-semibold text-[#155CA2] transition hover:bg-[#CBE5FE] disabled:opacity-50"
            >
              Auto-Identify
            </button>
          {/if}
        </div>
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
        onclick={handleRequestEnqueue}
        disabled={!modeReady || selectedCount === 0 || queueingJobs}
        class="rounded-md bg-[#111111] px-3.5 py-1.5 text-xs font-semibold text-white transition hover:bg-[#2A2A2A] disabled:cursor-not-allowed disabled:bg-[#C9C7C0] disabled:text-[#6B6A65]"
      >
        {#if queueingJobs}
          Queueing...
        {:else if selectedCount > 0 && workflowMode === "encodeOnly" && activeQualityProfile}
          Encode {selectedCount} ({shortResolutionLabel(activeQualityProfile.resolution)} {activeQualityProfile.delivery === "local" ? "Local" : "Remote"}) →
        {:else if selectedCount > 0}
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
      {#if queueActionError}
        <div
          class="flex shrink-0 items-center justify-between gap-3 border-b border-red-200 bg-red-50 px-5 py-2 text-xs text-red-700"
        >
          <span class="truncate font-mono text-[11px]">{queueActionError}</span>
          <button
            type="button"
            onclick={() => (queueActionError = null)}
            class="shrink-0 font-semibold text-red-700 hover:underline"
          >
            Dismiss
          </button>
        </div>
      {/if}
      {#if workflowMode === "encodeOnly" && (folderIdentifyStatus || localVideoItems.length > 0)}
        <div
          class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-[#E5E3DD] bg-[#FAF9F6] px-5 py-1.5 text-[11px]"
        >
          <div class="flex min-w-0 flex-wrap items-center gap-2">
            {#if localVideoItems.length > 0}
              <span
                class="rounded-full border border-[#E5E3DD] bg-white px-2 py-0.5 font-mono text-[10px] font-semibold text-[#111111]"
              >
                {localVideoItems.length} file{localVideoItems.length === 1 ? "" : "s"}
                {#if totalLocalSizeBytes > 0}
                  · {formatBytesHuman(totalLocalSizeBytes)}
                {/if}
              </span>
            {/if}
            {#if folderIdentifyStatus}
              <span class="truncate font-medium text-[#17462B]">
                {folderIdentifyStatus}
              </span>
            {:else if activeQualityProfile}
              <span class="text-[#5C5B57]">
                All checked files encode uniformly to
                <span class="font-mono font-semibold text-[#111111]">
                  {shortResolutionLabel(activeQualityProfile.resolution)} {activeQualityProfile.delivery} · {shortEncoderLabel(activeQualityProfile.encoder)} · .{outputContainer}
                </span>
              </span>
            {/if}
          </div>

          <div class="flex shrink-0 items-center gap-1.5">
            {#if mediaKind === "tvEpisode" && detectedBatchSeasons.length > 1}
              <span
                class="rounded-full border border-[#B5DAFA] bg-[#DCEEFF] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#155CA2]"
              >
                {detectedBatchSeasons.length} seasons ({detectedBatchSeasons
                  .map((s) => `S${String(s).padStart(2, "0")}`)
                  .join(", ")})
              </span>
            {/if}
            {#if folderShowCandidates.length > 1}
              <button
                type="button"
                onclick={() => (searchResults = folderShowCandidates)}
                class="rounded border border-[#B5DAFA] bg-[#DCEEFF] px-2 py-0.5 text-[10px] font-semibold text-[#155CA2] hover:bg-[#CBE5FE]"
              >
                Switch Show ({folderShowCandidates.length})
              </button>
            {/if}
            {#if localVideoItems.length > 0}
              <button
                type="button"
                onclick={clearLocalVideoItems}
                class="rounded border border-[#E5E3DD] bg-white px-2 py-0.5 text-[10px] font-medium text-[#6B6A65] hover:text-red-600"
              >
                Clear List
              </button>
            {/if}
          </div>
        </div>
      {/if}
      <!-- Table Header -->
      {#if workflowMode === "encodeOnly"}
        <div
          class="grid shrink-0 grid-cols-12 items-center border-b border-[#E5E3DD] bg-[#F8F7F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#787772]"
        >
          <div class="col-span-1 flex items-center">
            <input
              type="checkbox"
              checked={allRowsChecked}
              disabled={localVideoItems.length === 0}
              onchange={toggleAllRows}
              aria-label="Select all video files"
              class="h-3.5 w-3.5 accent-[#111111]"
            />
          </div>
          <div class="col-span-2">Type / Episode</div>
          <div class="col-span-5">Source File → Uniform Output Name</div>
          <div class="col-span-2 text-right">Length / Size</div>
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
          {#if localVideoItems.length === 0}
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
                Add a TV season folder or video files to batch encode
              </h2>
              <p class="mt-1 max-w-md text-xs leading-relaxed text-[#787772]">
                Drop in an entire folder of oversized <span class="font-mono text-[#111111]">.mkv</span>
                or <span class="font-mono text-[#111111]">.mp4</span> episodes. RipDaddy
                automatically identifies the show and episodes via TVDB/TMDB/TVMaze,
                uniforms all filenames, generates trickplay frames, and encodes the whole
                folder with one quality profile.
              </p>
              <div class="mt-4 flex flex-wrap items-center justify-center gap-2">
                <button
                  type="button"
                  onclick={() => void pickLocalVideoFolder()}
                  disabled={!modeReady || identifyingFolder}
                  class="rounded-md bg-[#1D8BF8] px-4 py-2 text-xs font-semibold text-white transition hover:bg-[#137CE5] disabled:cursor-not-allowed disabled:bg-[#D8D6CF] disabled:text-[#787772]"
                >
                  {identifyingFolder ? "Scanning Folder..." : "+ Add TV / Video Folder"}
                </button>
                <button
                  type="button"
                  onclick={() => void pickLocalVideoFiles()}
                  disabled={!modeReady || identifyingFolder}
                  class="rounded-md border border-[#DFDDD6] bg-white px-4 py-2 text-xs font-semibold text-[#111111] transition hover:bg-[#F8F7F4] disabled:cursor-not-allowed disabled:opacity-50"
                >
                  + Add Video Files
                </button>
              </div>
            </div>
          {:else}
            <div class="divide-y divide-[#EFECE6]">
              {#each localVideoItems as item, idx (item.path)}
                <div
                  class={`grid grid-cols-12 items-center gap-2 px-5 py-2.5 text-xs transition ${
                    inspectedLocalFile === item.path
                      ? "bg-[#F0F7FF]"
                      : "hover:bg-[#FAF9F6]"
                  }`}
                >
                  <div class="col-span-1 flex items-center">
                    <input
                      type="checkbox"
                      checked={checkedLocalFiles.includes(item.path)}
                      onchange={() => toggleLocalFileChecked(item.path)}
                      aria-label={`Select ${item.fileName}`}
                      class="h-3.5 w-3.5 accent-[#111111]"
                    />
                  </div>

                  <div class="col-span-2 flex items-center gap-1">
                    {#if item.mediaKind === "tvEpisode"}
                      <button
                        type="button"
                        onclick={() => toggleItemMediaKind(item.path)}
                        title="Detected as TV episode (click to switch to Movie)"
                        class="rounded border border-[#B5DAFA] bg-[#DCEEFF] px-1 py-0.5 font-mono text-[9px] font-bold uppercase text-[#155CA2] hover:bg-[#CBE5FE]"
                      >
                        TV
                      </button>
                      <input
                        type="text"
                        inputmode="numeric"
                        aria-label={`Season for ${item.fileName}`}
                        value={item.season ?? parsedSeason}
                        onchange={(e) =>
                          updateItemSeasonOrEpisode(
                            item.path,
                            "season",
                            e.currentTarget.value,
                          )}
                        class="w-7 rounded border border-[#DFDDD6] bg-white px-0.5 py-0.5 text-center font-mono text-[10px] font-semibold text-[#111111] outline-none focus:border-[#1D8BF8]"
                      />
                      <span class="font-mono text-[9px] text-[#787772]">×</span>
                      <input
                        type="text"
                        inputmode="numeric"
                        aria-label={`Episode for ${item.fileName}`}
                        value={item.episode ?? parsedStartEpisode + idx}
                        onchange={(e) =>
                          updateItemSeasonOrEpisode(
                            item.path,
                            "episode",
                            e.currentTarget.value,
                          )}
                        class="w-7 rounded border border-[#DFDDD6] bg-white px-0.5 py-0.5 text-center font-mono text-[10px] font-semibold text-[#111111] outline-none focus:border-[#1D8BF8]"
                      />
                    {:else}
                      <button
                        type="button"
                        onclick={() => toggleItemMediaKind(item.path)}
                        title="Detected as Movie (click to switch to TV episode)"
                        class="rounded border border-[#B3E8CB] bg-[#E3F9EC] px-1.5 py-0.5 font-mono text-[9px] font-bold uppercase text-[#17462B] hover:bg-[#CEF5DF]"
                      >
                        Movie
                      </button>
                      {#if item.needsTitle}
                        <button
                          type="button"
                          onclick={() => inspectLocalFile(item.path)}
                          class="rounded border border-[#E6C575] bg-[#FEF7E4] px-1.5 py-0.5 text-[9px] font-semibold text-[#5C4107]"
                        >
                          Needs title
                        </button>
                      {:else if item.movieCandidates.length > 1}
                        <button
                          type="button"
                          onclick={() => {
                            inspectLocalFile(item.path);
                            searchResults = item.movieCandidates;
                          }}
                          class="rounded border border-[#E5E3DD] bg-white px-1.5 py-0.5 text-[9px] font-semibold text-[#5C5B57] hover:text-[#111111]"
                        >
                          Switch
                        </button>
                      {/if}
                    {/if}
                  </div>

                  <button
                    type="button"
                    onclick={() => inspectLocalFile(item.path)}
                    class="col-span-5 min-w-0 text-left"
                    title={item.path}
                  >
                    <div class="flex items-center gap-1.5">
                      <span class="truncate font-semibold text-[#111111] hover:text-[#1D8BF8]">
                        {item.fileName}
                      </span>
                      {#if item.matchedFromCatalog}
                        <span
                          title="Matched from online catalog"
                          class="h-1.5 w-1.5 shrink-0 rounded-full bg-[#3AD27E]"
                        ></span>
                      {/if}
                    </div>
                    <div
                      class="truncate font-mono text-[10px] text-[#155CA2]"
                      title={formatItemTargetFilename(item, idx)}
                    >
                      → {formatItemTargetFilename(item, idx)}
                    </div>
                  </button>

                  <div class="col-span-2 text-right font-mono text-[11px]">
                    {#if item.durationSeconds}
                      <span class="text-[#787772]">
                        {formatDurationShort(item.durationSeconds)} ·
                      </span>
                    {/if}
                    <span
                      class={(item.fileSizeBytes ?? 0) >= 8_000_000_000
                        ? "font-semibold text-[#B4690E]"
                        : "text-[#52524E]"}
                    >
                      {formatBytesHuman(item.fileSizeBytes)}
                    </span>
                  </div>

                  <div class="col-span-2 flex items-center justify-end gap-2">
                    <button
                      type="button"
                      onclick={() => inspectLocalFile(item.path)}
                      class={`rounded border px-2 py-0.5 text-[10px] font-semibold transition ${
                        inspectedLocalFile === item.path
                          ? "border-[#3894E2] bg-[#DCEEFF] text-[#155CA2]"
                          : "border-[#DFDDD6] bg-white text-[#111111] hover:bg-[#F8F7F4]"
                      }`}
                    >
                      {inspectedLocalFile === item.path ? "Viewing" : "Frames"}
                    </button>
                    <button
                      type="button"
                      onclick={() => removeLocalSourceFile(item.path)}
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
              {scanningDrives || scanningDisc
                ? "Scanning optical disc..."
                : "No disc titles loaded"}
            </h2>
            <p class="mt-1 max-w-md text-xs leading-relaxed text-[#787772]">
              {#if scanStatusMessage}
                {scanStatusMessage}
              {:else}
                Insert a DVD or Blu-ray and click
                <span class="font-semibold text-[#111111]">Scan</span> to populate
                titles, durations, chapters, and selectable video/audio/subtitle
                track pills (filtering titles shorter than
                <span class="font-mono text-[#111111]">{settings.minTitleLengthSeconds}s</span>).
              {/if}
            </p>
            {#if !modeReady}
              <button
                type="button"
                onclick={() => openSettingsAt("binaries")}
                class="mt-4 rounded-md border border-[#DDA12D] bg-[#FEF7E4] px-3.5 py-1.5 text-xs font-semibold text-[#5C4107] hover:bg-[#FCEEC8]"
              >
                Open Settings → External CLIs
              </button>
            {:else if !scanningDrives && !scanningDisc}
              <button
                type="button"
                onclick={() => void handleScanDrives()}
                class="mt-4 rounded-md bg-[#1D8BF8] px-4 py-2 text-xs font-semibold text-white transition hover:bg-[#137CE5]"
              >
                Scan Optical Drives
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
        trickplay={activeTrickplay}
        loadingTrickplay={loadingActiveTrickplay}
        {trickplayError}
        onRegenerateFrames={() => {
          if (inspectedLocalFile) {
            void loadTrickplayPreview(inspectedLocalFile, true);
          }
        }}
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
    onClearFinished={() => void handleClearFinished()}
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

  <!-- Folder Organization Prompt (when encoding Movie, TV, or Mixed batches) -->
  <FolderOrganizeModal
    open={organizeModalOpen}
    batchKind={activeBatchKind}
    showTitle={sharedTvShow?.title || mediaTitle}
    {namingConvention}
    outputExtension={workflowMode === "ripOnly" ? "mkv" : outputContainer}
    outputDir={resolvedOutputDir}
    {sourceFolderPath}
    selectedFileCount={selectedCount}
    detectedSeasons={detectedBatchSeasons}
    sampleItems={modalSampleItems}
    recommendedOrg={recommendedFolderOrg}
    onBrowseOutputDir={() => void browseDirectory("outputDir")}
    onUseSourceFolderAsOutput={() => {
      if (sourceFolderPath) {
        updateSetting("outputDir", sourceFolderPath);
      }
    }}
    onConfirm={handleConfirmFolderOrganization}
    onCancel={() => (organizeModalOpen = false)}
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
