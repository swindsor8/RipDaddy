<script lang="ts">
  import { onMount } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open } from "@tauri-apps/plugin-dialog";
  import { Store } from "@tauri-apps/plugin-store";
  import FirstRunSetupModal from "./lib/FirstRunSetupModal.svelte";
  import FolderOrganizeModal from "./lib/FolderOrganizeModal.svelte";
  import OrganizeReviewModal from "./lib/OrganizeReviewModal.svelte";
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
  import {
    analyzeDiscTitles,
    dedupeTitleSelection,
  } from "./lib/discTitleFilters";
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
    DiskSpaceInfo,
    EncoderCapabilities,
    EnqueueJobRequest,
    EpisodeInfo,
    EpisodePreviewSample,
    ExternalIds,
    FolderContentKind,
    FolderIdentifyResult,
    FolderOrganization,
    FolderOrganizationPref,
    IdentifiedVideoFile,
    MediaExtraType,
    MediaKind,
    MediaMetadata,
    MetadataProvider,
    MetadataProviderConfig,
    MetadataSearchQuery,
    NamingConvention,
    OpticalDrive,
    OrganizeExecutionResult,
    OrganizeFileRequest,
    PipelineEvent,
    PipelineMode,
    PreviewFrame,
    QualityProfile,
    QueueJob,
    QueueStateSnapshot,
    ReorderAction,
    ResolutionTier,
    SettingsCategory,
    StorageSpaceReport,
    StreamTrack,
    ThemeMode,
    TrackSelectionOptions,
    TrickplayPreview,
    UndoOrganizeResult,
    VideoMediaInfo,
    WorkspaceMode,
  } from "./lib/types";

  const SETTINGS_STORE_FILE = "settings.json";

  // Binary detection, GPU probing, disk space & OS notifications state
  let binaries = $state<BinariesCheckResult | null>(null);
  let checkingBinaries = $state(false);
  let installingBinary = $state<BinaryKind | "all" | null>(null);
  let checkError = $state<string | null>(null);
  let setupOutcomes = $state<BinaryInstallOutcome[]>([]);
  let encoderCaps = $state<EncoderCapabilities | null>(null);
  let probingEncoders = $state(false);
  let storageSpace = $state<StorageSpaceReport | null>(null);
  let loadingDiskSpace = $state(false);
  let testingNotification = $state(false);
  let notificationTestStatus = $state<string | null>(null);
  let notificationBanner = $state<{ title: string; body: string; kind: "info" | "success" | "warn" } | null>(null);
  let systemPrefersDark = $state(false);

  // Categorized settings dropdown & first-run onboarding state
  let settingsOpen = $state(false);
  let settingsCategory = $state<SettingsCategory>("general");
  let firstRunModalOpen = $state(false);

  let settings = $state<AppSettings>({
    theme: "light",
    desktopNotifications: true,
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
    preferredAudioLanguages: [],
    preferredSubtitleLanguages: [],
    namingConvention: "plex",
    tvFolderOrganization: "ask",
    defaultMetadataProvider: "imdb",
    omdbApiKey: null,
    tvdbApiKey: null,
    tmdbApiKey: null,
  });

  // Active workspace mode
  let workflowMode = $state<WorkspaceMode>("ripAndEncode");

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
  let excludedStreamIdsByTitle = $state<Record<number, number[]>>({});
  let titleExtraById = $state<Record<number, MediaExtraType | null>>({});
  let titleEpisodeEndById = $state<Record<number, number | null>>({});

  let localVideoItems = $state<IdentifiedVideoFile[]>([]);
  let checkedLocalFiles = $state<string[]>([]);
  let inspectedLocalFile = $state<string | null>(null);
  let sourceFolderPath = $state<string | null>(null);
  let detectedFolderKind = $state<FolderContentKind | null>(null);
  let sharedTvShow = $state<MediaMetadata | null>(null);
  let detectedShows = $state<MediaMetadata[]>([]);
  let identifyingFolder = $state(false);
  let folderIdentifyStatus = $state<string | null>(null);
  let folderShowCandidates = $state<MediaMetadata[]>([]);
  let seriesEpisodeCatalog = $state<EpisodeInfo[]>([]);
  let seriesEpisodesByShow = $state<Record<string, EpisodeInfo[]>>({});
  let draggingOver = $state(false);
  let organizeModalOpen = $state(false);
  let organizeReviewOpen = $state(false);
  let lastOrganizeResult = $state<OrganizeExecutionResult | null>(null);
  let undoingOrganize = $state(false);
  let organizeUndoBanner = $state<string | null>(null);

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

  // Queue state & live CLI log lines per job
  let queueJobs = $state<QueueJob[]>([]);
  let queuePaused = $state(false);
  let restoredPendingCount = $state(0);
  let jobLogs = $state<Record<string, string[]>>({});

  const MAX_LOG_LINES_PER_JOB = 250;

  const MEDIA_EXTRA_OPTIONS: {
    value: MediaExtraType;
    label: string;
    suffix: string;
  }[] = [
    { value: "behindTheScenes", label: "Behind the Scenes", suffix: "-behindthescenes" },
    { value: "deleted", label: "Deleted Scene", suffix: "-deleted" },
    { value: "featurette", label: "Featurette", suffix: "-featurette" },
    { value: "interview", label: "Interview", suffix: "-interview" },
    { value: "scene", label: "Scene", suffix: "-scene" },
    { value: "short", label: "Short", suffix: "-short" },
    { value: "trailer", label: "Trailer", suffix: "-trailer" },
    { value: "other", label: "Other Extra", suffix: "-other" },
  ];

  function extraTypeSuffix(extra: MediaExtraType): string {
    return (
      MEDIA_EXTRA_OPTIONS.find((opt) => opt.value === extra)?.suffix ?? "-other"
    );
  }

  function isValidExtraType(val: string): val is MediaExtraType {
    return MEDIA_EXTRA_OPTIONS.some((opt) => opt.value === val);
  }

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

  const isLocalMode = $derived(
    workflowMode === "encodeOnly" || workflowMode === "organizeOnly",
  );

  const isOrganizeMode = $derived(workflowMode === "organizeOnly");

  const modeReady = $derived.by(() => {
    if (!requiresMakeMkv && !requiresHandBrake) return true;
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
    isLocalMode ? checkedLocalFiles.length : checkedTitleIds.length,
  );

  const allRowsChecked = $derived.by(() => {
    if (isLocalMode) {
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

  const resolvedDark = $derived(
    settings.theme === "dark" ||
      (settings.theme === "system" && systemPrefersDark),
  );

  // Apply Warm Espresso dark mode class to <html> and mirror preference to localStorage
  $effect(() => {
    const isDark = resolvedDark;
    const pref = settings.theme;
    if (typeof document !== "undefined") {
      document.documentElement.classList.toggle("dark", isDark);
    }
    try {
      localStorage.setItem("ripdaddy-theme", pref);
    } catch {
      // Ignore storage failures in restricted contexts.
    }
  });

  // Smart disc title analysis (Main Feature, Episodes 20–65m, duplicate-duration obfuscation)
  const discAnalysis = $derived.by(() => analyzeDiscTitles(titles));

  const inspectedTitleDuplicateGroup = $derived(
    inspectedTitleId !== null
      ? (discAnalysis.duplicateByTitleId[inspectedTitleId] ?? null)
      : null,
  );

  const selectedDiscBytes = $derived.by(() => {
    if (isLocalMode) return 0;
    return titles
      .filter((t) => checkedTitleIds.includes(t.titleId))
      .reduce((sum, t) => sum + (t.sizeBytes ?? 0), 0);
  });

  const hasCheckedDuplicates = $derived.by(() => {
    if (discAnalysis.duplicateGroups.length === 0) return false;
    const selectedSet = new Set(checkedTitleIds);
    return discAnalysis.duplicateGroups.some(
      (g) => g.titleIds.filter((id) => selectedSet.has(id)).length > 1,
    );
  });

  const activeDiskSpaceWarning = $derived.by<{
    level: "low" | "critical";
    message: string;
  } | null>(() => {
    if (isOrganizeMode || !storageSpace) return null;

    const targetInfo: DiskSpaceInfo | null =
      workflowMode === "ripOnly" || workflowMode === "encodeOnly"
        ? storageSpace.output
        : storageSpace.temp;
    const targetLabel =
      workflowMode === "ripOnly" || workflowMode === "encodeOnly"
        ? "Output folder"
        : "Temp rip folder";

    if (!targetInfo) return null;

    if (targetInfo.isTmpfs && workflowMode !== "encodeOnly") {
      return {
        level: "critical",
        message: `${targetLabel} is mounted on RAM-backed tmpfs (${formatDiskBytes(targetInfo.availableBytes)}). Switch it to a physical drive in Settings → Storage before ripping.`,
      };
    }

    if (!isLocalMode && selectedDiscBytes > 0) {
      if (selectedDiscBytes > targetInfo.availableBytes) {
        return {
          level: "critical",
          message: `Insufficient space in ${targetLabel}: selected titles total ~${formatBytesHuman(selectedDiscBytes)}, but only ${formatDiskBytes(targetInfo.availableBytes)} is available.`,
        };
      }
      if (
        workflowMode === "ripAndEncode" &&
        storageSpace.sameVolume &&
        selectedDiscBytes * 1.35 > targetInfo.availableBytes
      ) {
        return {
          level: "low",
          message: `Tight headroom on shared volume: selected titles need ~${formatBytesHuman(selectedDiscBytes)} temp staging + encoded output (${formatDiskBytes(targetInfo.availableBytes)} free).`,
        };
      }
    }

    if (targetInfo.severity === "critical") {
      return {
        level: "critical",
        message: `${targetLabel} is critically low (${formatDiskBytes(targetInfo.availableBytes)} free). Blu-ray remuxes require 40–80 GB.`,
      };
    }

    return null;
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
    if (isLocalMode && localVideoItems.length > 0) {
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
    if (isLocalMode) {
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
    if (isLocalMode && detectedShows.length > 1) {
      return "showAndSeason";
    }
    const outDir = (
      (isOrganizeMode && sourceFolderPath ? sourceFolderPath : null) ||
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

  function episodesCatalogForItem(
    item: IdentifiedVideoFile | null,
  ): EpisodeInfo[] {
    if (item?.seriesMetadata) {
      const byShow =
        seriesEpisodesByShow[item.seriesMetadata.title.toLowerCase()];
      if (byShow && byShow.length > 0) {
        return byShow;
      }
    }
    return seriesEpisodeCatalog;
  }

  const activeEpisodeCatalog = $derived.by<EpisodeInfo[]>(() =>
    episodesCatalogForItem(inspectedLocalItem),
  );

  const activeInspectedExtraType = $derived.by<MediaExtraType | null>(() => {
    if (isLocalMode) {
      return inspectedLocalItem?.extraType ?? null;
    }
    if (inspectedTitleId !== null) {
      return titleExtraById[inspectedTitleId] ?? null;
    }
    return null;
  });

  const activeInspectedEpisodeEnd = $derived.by<number | null>(() => {
    if (isLocalMode) {
      return inspectedLocalItem?.episodeEnd ?? null;
    }
    if (inspectedTitleId !== null) {
      return titleEpisodeEndById[inspectedTitleId] ?? null;
    }
    return null;
  });

  const modalSampleItems = $derived.by<EpisodePreviewSample[]>(() => {
    const samples: EpisodePreviewSample[] = [];
    const seenTvKeys = new Set<string>();
    const defaultTvIds: ExternalIds = sharedTvShow?.ids ?? {
      imdbId: imdbId.trim() || null,
      tvdbId: tvdbId.trim() || null,
      tmdbId: tmdbId.trim() || null,
    };
    const defaultTvTag = formatProviderTagFor(
      "tvEpisode",
      defaultTvIds,
      namingConvention,
    );
    const defaultShowTitle =
      sharedTvShow?.title || mediaTitle.trim() || "Untitled Show";
    const defaultShowYear = sharedTvShow?.year ?? parsedYear;

    if (isLocalMode) {
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
          episodeEnd: null,
          episodeTitle: m.extraType
            ? m.episodeTitle || m.fileName.replace(/\.[^.]+$/, "")
            : null,
          extraType: m.extraType,
        });
      }

      // Add one sample per distinct (show, season) (up to 3 total)
      for (let i = 0; i < tvEps.length && samples.length < 3; i++) {
        const item = tvEps[i];
        const showMeta = item.seriesMetadata ?? sharedTvShow;
        const showTitle = showMeta?.title || defaultShowTitle;
        const showYear = showMeta?.year ?? defaultShowYear;
        const showTag = showMeta
          ? formatProviderTagFor("tvEpisode", showMeta.ids, namingConvention)
          : defaultTvTag;
        const s = item.season ?? parsedSeason;
        const e = item.episode ?? parsedStartEpisode + i;
        const key = `${showTitle.toLowerCase()}:${s}:${item.extraType ?? ""}`;
        if (!seenTvKeys.has(key)) {
          seenTvKeys.add(key);
          samples.push({
            kind: "tvEpisode",
            title: showTitle,
            year: showYear,
            providerTag: showTag,
            season: s,
            episode: e,
            episodeEnd: item.episodeEnd,
            episodeTitle: item.episodeTitle,
            extraType: item.extraType,
          });
        }
      }
      if (tvEps.length > 1 && samples.length < 2) {
        const second = tvEps[1];
        const showMeta = second.seriesMetadata ?? sharedTvShow;
        samples.push({
          kind: "tvEpisode",
          title: showMeta?.title || defaultShowTitle,
          year: showMeta?.year ?? defaultShowYear,
          providerTag: showMeta
            ? formatProviderTagFor("tvEpisode", showMeta.ids, namingConvention)
            : defaultTvTag,
          season: second.season ?? parsedSeason,
          episode: second.episode ?? parsedStartEpisode + 1,
          episodeEnd: second.episodeEnd,
          episodeTitle: second.episodeTitle,
          extraType: second.extraType,
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
        episodeEnd:
          mediaKind === "tvEpisode" ? activeInspectedEpisodeEnd : null,
        episodeTitle: null,
        extraType: activeInspectedExtraType,
      });
    }

    if (samples.length === 0) {
      samples.push({
        kind: mediaKind,
        title: mediaTitle.trim() || "Untitled",
        year: parsedYear,
        providerTag: defaultTvTag,
        season: mediaKind === "tvEpisode" ? parsedSeason : null,
        episode: mediaKind === "tvEpisode" ? parsedStartEpisode : null,
        episodeEnd: null,
        episodeTitle: null,
        extraType: null,
      });
    }
    return samples;
  });

  const activeTokenChips = $derived.by(() => {
    const chips: string[] = [];
    chips.push(mediaTitle.trim() || "Title");
    if (parsedYear) chips.push(`(${parsedYear})`);
    if (mediaKind === "tvEpisode") {
      const activeSeason = isLocalMode
        ? (inspectedLocalItem?.season ?? parsedSeason)
        : parsedSeason;
      const activeEp = isLocalMode
        ? (inspectedLocalItem?.episode ?? parsedStartEpisode)
        : parsedStartEpisode;
      const activeEnd = activeInspectedEpisodeEnd;
      if (activeInspectedExtraType) {
        chips.push(`Specials${extraTypeSuffix(activeInspectedExtraType)}`);
      } else {
        const baseCode = `S${String(activeSeason).padStart(2, "0")}E${String(activeEp).padStart(2, "0")}`;
        chips.push(
          activeEnd && activeEnd > activeEp
            ? `${baseCode}-E${String(activeEnd).padStart(2, "0")}`
            : baseCode,
        );
      }
    } else if (activeInspectedExtraType) {
      chips.push(extraTypeSuffix(activeInspectedExtraType));
    }
    if (imdbId.trim()) chips.push(`imdb:${imdbId.trim()}`);
    if (tvdbId.trim()) chips.push(`tvdb:${tvdbId.trim()}`);
    if (tmdbId.trim()) chips.push(`tmdb:${tmdbId.trim()}`);
    return chips;
  });

  // Live output path preview backed by Rust's metadata::format_output_relative_path
  $effect(() => {
    const activeItem = isLocalMode ? inspectedLocalItem : null;
    const activeDiscTitle = !isLocalMode ? inspectedTitle : null;
    const previewSeason =
      mediaKind === "tvEpisode"
        ? (activeItem?.season ?? parsedSeason)
        : null;
    const previewEpisode =
      mediaKind === "tvEpisode"
        ? (activeItem?.episode ?? parsedStartEpisode)
        : null;
    const previewEpisodeEnd =
      mediaKind === "tvEpisode"
        ? (activeItem?.episodeEnd ??
          (activeDiscTitle ? (titleEpisodeEndById[activeDiscTitle.titleId] ?? null) : null))
        : null;
    const previewExtraType = activeItem
      ? activeItem.extraType
      : activeDiscTitle
        ? (titleExtraById[activeDiscTitle.titleId] ?? null)
        : null;
    const previewEpisodeTitle =
      mediaKind === "tvEpisode"
        ? (activeItem?.episodeTitle ??
          (previewExtraType ? (activeDiscTitle?.name ?? null) : null))
        : previewExtraType
          ? (activeItem?.episodeTitle ??
            activeItem?.fileName.replace(/\.[^.]+$/, "") ??
            activeDiscTitle?.name ??
            null)
          : null;

    const payload: MediaMetadata = {
      kind: mediaKind,
      title: mediaTitle,
      year: parsedYear,
      season: previewSeason,
      episode: previewEpisode,
      episodeEnd: previewEpisodeEnd,
      episodeTitle: previewEpisodeTitle,
      extraType: previewExtraType,
      ids: {
        imdbId: imdbId.trim() || null,
        tvdbId: tvdbId.trim() || null,
        tmdbId: tmdbId.trim() || null,
      },
      overview: null,
      posterUrl: null,
    };
    const convention = namingConvention;
    const ext =
      workflowMode === "ripOnly"
        ? "mkv"
        : workflowMode === "organizeOnly" && activeItem
          ? fileExtensionOf(activeItem.fileName)
          : outputContainer;
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
      workflowMode = value as WorkspaceMode;
    } else if (key === "defaultContainer") {
      outputContainer = value as string;
    } else if (key === "namingConvention") {
      namingConvention = value as NamingConvention;
    } else if (key === "defaultMetadataProvider") {
      metadataProvider = value as MetadataProvider;
    } else if (key === "desktopNotifications") {
      void invoke("set_desktop_notifications", { enabled: Boolean(value) }).catch(
        () => {},
      );
    } else if (key === "tempDir" || key === "outputDir") {
      void refreshDiskSpace();
    }

    void persistSetting(key, value);
  }

  function toggleDarkMode(): void {
    updateSetting("theme", resolvedDark ? "light" : "dark");
  }

  function formatDiskBytes(bytes: number): string {
    const tb = bytes / 1024 ** 4;
    if (tb >= 1) return `${tb.toFixed(tb >= 10 ? 1 : 2)} TB`;
    const gb = bytes / 1024 ** 3;
    if (gb >= 1) return `${gb.toFixed(gb >= 100 ? 0 : 1)} GB`;
    const mb = bytes / 1024 ** 2;
    return `${mb.toFixed(0)} MB`;
  }

  async function refreshDiskSpace(): Promise<void> {
    loadingDiskSpace = true;
    try {
      storageSpace = await invoke<StorageSpaceReport>("get_disk_space", {
        tempDir: settings.tempDir,
        outputDir: settings.outputDir,
      });
    } catch {
      // Ignore outside Tauri runtime.
    } finally {
      loadingDiskSpace = false;
    }
  }

  async function runProbeEncoders(): Promise<void> {
    probingEncoders = true;
    try {
      encoderCaps = await invoke<EncoderCapabilities>("probe_video_encoders", {
        config: currentBinaryConfig(),
      });
    } catch {
      encoderCaps = null;
    } finally {
      probingEncoders = false;
    }
  }

  async function handleTestNotification(): Promise<void> {
    testingNotification = true;
    notificationTestStatus = null;
    try {
      await invoke("send_os_notification", {
        title: "RipDaddy · Test Notification",
        body: "Native OS completion & disc-eject alerts are working!",
      });
      notificationTestStatus = "Notification sent ✓";
    } catch (err) {
      notificationTestStatus =
        err instanceof Error ? err.message : String(err);
    } finally {
      testingNotification = false;
    }
  }

  function showTransientBanner(
    title: string,
    body: string,
    kind: "info" | "success" | "warn",
  ): void {
    notificationBanner = { title, body, kind };
    setTimeout(() => {
      if (notificationBanner?.title === title && notificationBanner?.body === body) {
        notificationBanner = null;
      }
    }, 12000);
  }

  async function readStoreValue<T>(key: string): Promise<T | null> {
    if (!settingsStore) return null;
    return (await settingsStore.get<T>(key)) ?? null;
  }

  async function loadPersistedSettings(): Promise<void> {
    try {
      settingsStore = await Store.load(SETTINGS_STORE_FILE);

      const savedTheme = await readStoreValue<ThemeMode>("theme");
      if (
        savedTheme === "light" ||
        savedTheme === "dark" ||
        savedTheme === "system"
      ) {
        settings.theme = savedTheme;
      }
      settings.desktopNotifications =
        (await readStoreValue<boolean>("desktopNotifications")) ?? true;
      settings.defaultMode =
        (await readStoreValue<WorkspaceMode>("defaultMode")) ?? "ripAndEncode";
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
      settings.preferredAudioLanguages =
        (await readStoreValue<string[]>("preferredAudioLanguages")) ?? [];
      settings.preferredSubtitleLanguages =
        (await readStoreValue<string[]>("preferredSubtitleLanguages")) ?? [];
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
      void invoke("set_desktop_notifications", {
        enabled: settings.desktopNotifications,
      }).catch(() => {});
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
      void runProbeEncoders();
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

  async function refreshQueue(expandIfRestored = false): Promise<void> {
    try {
      const snap = await invoke<QueueStateSnapshot>("get_queue_state");
      queueJobs = snap.jobs;
      queuePaused = snap.paused;
      restoredPendingCount = snap.restoredPendingCount;
      if (expandIfRestored && snap.restoredPendingCount > 0) {
        queueExpanded = true;
      }
    } catch {
      try {
        queueJobs = await invoke<QueueJob[]>("get_queue");
      } catch {
        queueJobs = [];
      }
    }
  }

  async function handleToggleQueuePaused(): Promise<void> {
    queueActionError = null;
    try {
      const snap = await invoke<QueueStateSnapshot>("set_queue_paused", {
        paused: !queuePaused,
        config: currentBinaryConfig(),
      });
      queueJobs = snap.jobs;
      queuePaused = snap.paused;
      restoredPendingCount = snap.restoredPendingCount;
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
    }
  }

  async function handleMoveJob(
    jobId: string,
    action: ReorderAction,
  ): Promise<void> {
    queueActionError = null;
    try {
      queueJobs = await invoke<QueueJob[]>("move_queue_job", {
        jobId,
        action,
      });
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
    }
  }

  async function handleReorderJobToIndex(
    jobId: string,
    targetIndex: number,
  ): Promise<void> {
    queueActionError = null;
    try {
      queueJobs = await invoke<QueueJob[]>("reorder_queue_job", {
        jobId,
        targetIndex,
      });
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
    }
  }

  async function handleRemoveJob(jobId: string): Promise<void> {
    queueActionError = null;
    try {
      queueJobs = await invoke<QueueJob[]>("remove_queue_job", { jobId });
      if (jobId in jobLogs) {
        const next = { ...jobLogs };
        delete next[jobId];
        jobLogs = next;
      }
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
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

  function fileExtensionOf(fileName: string): string {
    const idx = fileName.lastIndexOf(".");
    if (idx <= 0 || idx === fileName.length - 1) return "mkv";
    return fileName.slice(idx + 1).toLowerCase();
  }

  function formatItemTargetFilename(
    item: IdentifiedVideoFile,
    rowIndex: number,
  ): string {
    const ext = isOrganizeMode
      ? fileExtensionOf(item.fileName)
      : outputContainer || "mkv";

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
      if (item.extraType) {
        const extraStem =
          sanitizeClientFilename(
            item.episodeTitle || item.fileName.replace(/\.[^.]+$/, ""),
          ) || clean;
        const extraFile = `${extraStem}${extraTypeSuffix(item.extraType)}.${ext}`;
        if (effectivePreviewFolderOrg === "flat") {
          return extraFile;
        }
        return `${tagged}/${extraFile}`;
      }
      if (effectivePreviewFolderOrg === "flat") {
        return `${tagged}.${ext}`;
      }
      return `${tagged}/${tagged}.${ext}`;
    }

    const showMeta = item.seriesMetadata ?? sharedTvShow;
    const tvTitle =
      showMeta?.title ||
      (mediaKind === "tvEpisode" ? mediaTitle.trim() : "") ||
      item.fileName.replace(/\.[^.]+$/, "");
    const tvYear =
      showMeta?.year ?? (mediaKind === "tvEpisode" ? parsedYear : null);
    const tvIds: ExternalIds = showMeta?.ids ?? {
      imdbId: imdbId.trim() || null,
      tvdbId: tvdbId.trim() || null,
      tmdbId: tmdbId.trim() || null,
    };
    const baseTitle = sanitizeClientFilename(tvTitle) || "Untitled";
    const titleWithYear = tvYear ? `${baseTitle} (${tvYear})` : baseTitle;
    const tvTag = formatProviderTagFor("tvEpisode", tvIds, namingConvention);
    const taggedShowDir = tvTag ? `${titleWithYear} ${tvTag}` : titleWithYear;

    if (item.extraType) {
      const extraStem =
        sanitizeClientFilename(
          item.episodeTitle || item.fileName.replace(/\.[^.]+$/, ""),
        ) || baseTitle;
      const extraFile = `${extraStem}${extraTypeSuffix(item.extraType)}.${ext}`;
      if (effectivePreviewFolderOrg === "flat") {
        return extraFile;
      }
      if (effectivePreviewFolderOrg === "seasonOnly") {
        return `Specials/${extraFile}`;
      }
      return `${taggedShowDir}/Specials/${extraFile}`;
    }

    const s = item.season ?? parsedSeason;
    const e = item.episode ?? parsedStartEpisode + rowIndex;
    const seasonPad = String(s).padStart(2, "0");
    const epPad = String(e).padStart(2, "0");
    const code =
      item.episodeEnd && item.episodeEnd > e
        ? `S${seasonPad}E${epPad}-E${String(item.episodeEnd).padStart(2, "0")}`
        : `S${seasonPad}E${epPad}`;
    const epTitle = item.episodeTitle
      ? sanitizeClientFilename(item.episodeTitle)
      : "";
    const file = epTitle
      ? `${titleWithYear} - ${code} - ${epTitle}.${ext}`
      : `${titleWithYear} - ${code}.${ext}`;
    const seasonDir = s === 0 ? "Specials" : `Season ${seasonPad}`;
    if (effectivePreviewFolderOrg === "flat") {
      return file;
    }
    if (effectivePreviewFolderOrg === "seasonOnly") {
      return `${seasonDir}/${file}`;
    }
    return `${taggedShowDir}/${seasonDir}/${file}`;
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
    } else {
      const showMeta = item.seriesMetadata ?? sharedTvShow;
      mediaKind = "tvEpisode";
      if (showMeta) {
        mediaTitle = showMeta.title;
        mediaYearInput = showMeta.year ? String(showMeta.year) : "";
        imdbId = showMeta.ids.imdbId ?? "";
        tvdbId = showMeta.ids.tvdbId ?? "";
        tmdbId = showMeta.ids.tmdbId ?? "";
        selectedPosterUrl = showMeta.posterUrl;
        selectedOverview = showMeta.overview;
      }
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
        episodeEnd: nextKind === "tvEpisode" ? item.episodeEnd : null,
        needsTitle: nextKind === "movie" && !item.movieMetadata,
      };
    });
    const updated = localVideoItems.find((i) => i.path === path);
    if (updated && inspectedLocalFile === path) {
      syncMetadataStripToItem(updated);
    }
  }

  function applyEpisodeCatalogToItems(
    catalog: EpisodeInfo[],
    showTitleFilter?: string,
  ): void {
    if (!showTitleFilter) {
      seriesEpisodeCatalog = catalog;
    }
    if (catalog.length === 0 || localVideoItems.length === 0) return;

    const byKey = new Map<string, EpisodeInfo>();
    for (const ep of catalog) {
      byKey.set(`${ep.season}:${ep.episode}`, ep);
    }

    localVideoItems = localVideoItems.map((item, idx) => {
      if (item.mediaKind !== "tvEpisode") return item;
      if (
        showTitleFilter &&
        item.seriesMetadata &&
        item.seriesMetadata.title.toLowerCase() !==
          showTitleFilter.toLowerCase()
      ) {
        return item;
      }
      const s = item.season ?? parsedSeason;
      const e = item.episode ?? parsedStartEpisode + idx;
      const official = byKey.get(`${s}:${e}`);
      if (official) {
        let combinedTitle = official.title;
        if (item.episodeEnd && item.episodeEnd > e) {
          const endOfficial = byKey.get(`${s}:${item.episodeEnd}`);
          if (endOfficial && endOfficial.title !== official.title) {
            combinedTitle = `${official.title} & ${endOfficial.title}`;
          }
        }
        return {
          ...item,
          season: s,
          episode: e,
          episodeTitle: combinedTitle,
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
    detectedShows = res.detectedShows ?? (res.matchedShow ? [res.matchedShow] : []);
    folderShowCandidates = res.showCandidates;
    seriesEpisodeCatalog = res.episodesCatalog;
    seriesEpisodesByShow = res.seriesEpisodesByShow ?? {};
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
      if (
        !res.matchedShow &&
        !first.seriesMetadata &&
        res.inferredShowQuery &&
        first.mediaKind === "tvEpisode"
      ) {
        mediaTitle = res.inferredShowQuery;
      }
    }
  }

  async function handleDroppedPaths(rawPaths: string[]): Promise<void> {
    const cleaned = rawPaths.map((p) => p.trim()).filter((p) => p.length > 0);
    if (cleaned.length === 0 || identifyingFolder) return;

    if (!isLocalMode) {
      workflowMode = "organizeOnly";
    }
    lastOrganizeResult = null;
    organizeUndoBanner = null;
    identifyingFolder = true;
    queueActionError = null;
    folderIdentifyStatus = `Scanning ${cleaned.length} dropped item${cleaned.length === 1 ? "" : "s"} and identifying metadata...`;
    try {
      const res = await invoke<FolderIdentifyResult>("identify_dropped_paths", {
        paths: cleaned,
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

  async function pickLocalVideoFolder(): Promise<void> {
    try {
      const selected = await open({
        title: isOrganizeMode
          ? "Connect a movie or TV folder to look up, rename & reorganize"
          : "Select a TV season or video folder to identify & batch encode",
        directory: true,
        multiple: false,
      });
      if (typeof selected !== "string" || selected.trim().length === 0) return;

      lastOrganizeResult = null;
      organizeUndoBanner = null;
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

  async function rescanConnectedFolder(): Promise<void> {
    if (!sourceFolderPath || identifyingFolder) return;
    identifyingFolder = true;
    queueActionError = null;
    folderIdentifyStatus = "Rescanning connected folder...";
    try {
      const res = await invoke<FolderIdentifyResult>("identify_video_folder", {
        folderPath: sourceFolderPath,
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

  function assignOfficialEpisodeToItem(
    path: string,
    encodedSe: string,
  ): void {
    const [sStr, eStr] = encodedSe.split(":");
    const s = Number.parseInt(sStr, 10);
    const e = Number.parseInt(eStr, 10);
    if (!Number.isFinite(s) || !Number.isFinite(e)) return;
    const targetItem = localVideoItems.find((i) => i.path === path) ?? null;
    const catalog = episodesCatalogForItem(targetItem);
    const official = catalog.find(
      (ep) => ep.season === s && ep.episode === e,
    );
    if (!official) return;
    localVideoItems = localVideoItems.map((item) => {
      if (item.path !== path) return item;
      let combinedTitle = official.title;
      const validEnd =
        item.episodeEnd && item.episodeEnd > official.episode
          ? item.episodeEnd
          : null;
      if (validEnd) {
        const endEp = catalog.find(
          (ep) => ep.season === official.season && ep.episode === validEnd,
        );
        if (endEp && endEp.title !== official.title) {
          combinedTitle = `${official.title} & ${endEp.title}`;
        }
      }
      return {
        ...item,
        mediaKind: "tvEpisode",
        season: official.season,
        episode: official.episode,
        episodeEnd: validEnd,
        episodeTitle: combinedTitle,
        matchedFromCatalog: true,
      };
    });
  }

  function updateItemEpisodeTitle(path: string, rawTitle: string): void {
    const cleaned = rawTitle.trim();
    localVideoItems = localVideoItems.map((item) =>
      item.path === path
        ? {
            ...item,
            episodeTitle: cleaned.length > 0 ? cleaned : null,
          }
        : item,
    );
  }

  function updateItemSeasonOrEpisode(
    path: string,
    field: "season" | "episode" | "episodeEnd",
    raw: string,
  ): void {
    const trimmed = raw.trim();
    const parsed = trimmed.length > 0 ? Number.parseInt(trimmed, 10) : Number.NaN;
    const num = Number.isFinite(parsed) && parsed >= 0 ? parsed : null;

    localVideoItems = localVideoItems.map((item) => {
      if (item.path !== path) return item;
      const catalog = episodesCatalogForItem(item);
      const nextSeason = field === "season" ? num : item.season;
      const nextEpisode = field === "episode" ? num : item.episode;
      let nextEpisodeEnd = field === "episodeEnd" ? num : item.episodeEnd;
      if (
        nextEpisodeEnd !== null &&
        nextEpisode !== null &&
        nextEpisodeEnd <= nextEpisode
      ) {
        nextEpisodeEnd = null;
      }
      let nextTitle = item.episodeTitle;
      let matched = false;
      if (nextSeason !== null && nextEpisode !== null) {
        const official = catalog.find(
          (ep) => ep.season === nextSeason && ep.episode === nextEpisode,
        );
        if (official) {
          nextTitle = official.title;
          if (nextEpisodeEnd && nextEpisodeEnd > nextEpisode) {
            const endEp = catalog.find(
              (ep) =>
                ep.season === nextSeason && ep.episode === nextEpisodeEnd,
            );
            if (endEp && endEp.title !== official.title) {
              nextTitle = `${official.title} & ${endEp.title}`;
            }
          }
          matched = true;
        }
      }
      return {
        ...item,
        season: nextSeason,
        episode: nextEpisode,
        episodeEnd: nextEpisodeEnd,
        episodeTitle: nextTitle,
        matchedFromCatalog: matched,
      };
    });
  }

  function updateInspectedExtraType(raw: string): void {
    const nextExtra: MediaExtraType | null = isValidExtraType(raw) ? raw : null;
    if (isLocalMode && inspectedLocalFile) {
      localVideoItems = localVideoItems.map((item) =>
        item.path === inspectedLocalFile
          ? {
              ...item,
              extraType: nextExtra,
              season:
                nextExtra && item.mediaKind === "tvEpisode"
                  ? 0
                  : item.season,
            }
          : item,
      );
    } else if (!isLocalMode && inspectedTitleId !== null) {
      titleExtraById = {
        ...titleExtraById,
        [inspectedTitleId]: nextExtra,
      };
    }
  }

  function updateInspectedDiscEpisodeEnd(raw: string): void {
    if (isLocalMode || inspectedTitleId === null) return;
    const parsed = Number.parseInt(raw.trim(), 10);
    const val = Number.isFinite(parsed) && parsed > 0 ? parsed : null;
    titleEpisodeEndById = {
      ...titleEpisodeEndById,
      [inspectedTitleId]: val,
    };
  }

  function toggleTitleStream(titleId: number, streamId: number): void {
    const current = excludedStreamIdsByTitle[titleId] ?? [];
    const next = current.includes(streamId)
      ? current.filter((id) => id !== streamId)
      : [...current, streamId];
    excludedStreamIdsByTitle = {
      ...excludedStreamIdsByTitle,
      [titleId]: next,
    };
  }

  function buildTrackSelectionForTitle(
    title: DiscTitle | null,
  ): TrackSelectionOptions {
    const base: TrackSelectionOptions = {
      selectedAudioTracks: null,
      selectedSubtitleTracks: null,
      audioLanguages: [...settings.preferredAudioLanguages],
      subtitleLanguages: [...settings.preferredSubtitleLanguages],
    };
    if (!title) return base;
    const excluded = excludedStreamIdsByTitle[title.titleId] ?? [];
    if (excluded.length === 0) return base;

    const audioStreams = title.streams.filter((s) => s.trackType === "audio");
    const subStreams = title.streams.filter((s) => s.trackType === "subtitle");

    if (audioStreams.some((s) => excluded.includes(s.streamId))) {
      const keptAudio = audioStreams
        .map((s, idx) => (excluded.includes(s.streamId) ? null : idx + 1))
        .filter((n): n is number => n !== null);
      if (keptAudio.length > 0) {
        base.selectedAudioTracks = keptAudio;
      }
    }

    if (subStreams.some((s) => excluded.includes(s.streamId))) {
      const keptSubs = subStreams
        .map((s, idx) => (excluded.includes(s.streamId) ? null : idx + 1))
        .filter((n): n is number => n !== null);
      base.selectedSubtitleTracks = keptSubs;
    }

    return base;
  }

  function clearLocalVideoItems(): void {
    localVideoItems = [];
    checkedLocalFiles = [];
    inspectedLocalFile = null;
    sourceFolderPath = null;
    detectedFolderKind = null;
    sharedTvShow = null;
    detectedShows = [];
    folderIdentifyStatus = null;
    folderShowCandidates = [];
    seriesEpisodeCatalog = [];
    seriesEpisodesByShow = {};
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
          episodeEnd: null,
          episodeTitle: null,
          extraType: null,
          seriesMetadata: null,
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
    if (isLocalMode) {
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

  function selectMainFeatureTitle(): void {
    const mainId = discAnalysis.mainFeatureTitleId;
    if (mainId === null) return;
    checkedTitleIds = [mainId];
    inspectedTitleId = mainId;
    previewOpen = true;
  }

  function selectDiscEpisodeTitles(): void {
    const epIds = discAnalysis.episodeTitleIds;
    if (epIds.length === 0) return;
    checkedTitleIds = [...epIds];
    mediaKind = "tvEpisode";
    inspectedTitleId = epIds[0];
    previewOpen = true;
  }

  function selectNoDiscTitles(): void {
    checkedTitleIds = [];
  }

  function dedupeCheckedDiscTitles(): void {
    checkedTitleIds = dedupeTitleSelection(
      titles,
      checkedTitleIds,
      discAnalysis,
    );
  }

  async function handleScanDisc(driveIndex: number): Promise<void> {
    scanningDisc = true;
    scanStatusMessage = `Scanning disc in drive #${driveIndex}...`;
    titles = [];
    checkedTitleIds = [];
    inspectedTitleId = null;
    excludedStreamIdsByTitle = {};
    titleExtraById = {};
    titleEpisodeEndById = {};

    try {
      const scan = await invoke<DiscScanResult>("scan_disc", {
        driveIndex,
        minLengthSeconds: settings.minTitleLengthSeconds,
        config: currentBinaryConfig(),
      });
      titles = scan.titles;
      activeDiscName = scan.discName;

      if (scan.titles.length > 0) {
        const analysis = analyzeDiscTitles(scan.titles);
        if (mediaKind === "tvEpisode") {
          checkedTitleIds =
            analysis.episodeTitleIds.length > 0
              ? [...analysis.episodeTitleIds]
              : scan.titles.map((t) => t.titleId);
        } else {
          // Auto-select the main feature (longest title with smart tie-breaking)
          checkedTitleIds =
            analysis.mainFeatureTitleId !== null
              ? [analysis.mainFeatureTitleId]
              : [scan.titles[0].titleId];
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
    if ((mode === "encodeOnly" || mode === "organizeOnly") && file) {
      void loadTrickplayPreview(file, false);
    }
  });

  function buildJobMetadata(
    rowIndex: number,
    fallbackTitle: string,
    totalSelected: number,
    rowOverride?: {
      season?: number | null;
      episode?: number | null;
      episodeEnd?: number | null;
      episodeTitle?: string | null;
      extraType?: MediaExtraType | null;
    },
  ): MediaMetadata {
    const trimmed = mediaTitle.trim();
    const extraType = rowOverride?.extraType ?? null;
    let chosenTitle: string;
    if (mediaKind === "tvEpisode") {
      chosenTitle = trimmed || fallbackTitle;
    } else if (!trimmed) {
      chosenTitle = fallbackTitle;
    } else if (!extraType && totalSelected > 1 && rowIndex > 0) {
      chosenTitle = `${trimmed} - ${fallbackTitle}`;
    } else {
      chosenTitle = trimmed;
    }
    const epStart = rowOverride?.episode ?? parsedStartEpisode + rowIndex;
    const epEnd =
      rowOverride?.episodeEnd && rowOverride.episodeEnd > epStart
        ? rowOverride.episodeEnd
        : null;
    return {
      kind: mediaKind,
      title: chosenTitle,
      year: parsedYear,
      season:
        mediaKind === "tvEpisode"
          ? (rowOverride?.season ?? parsedSeason)
          : null,
      episode: mediaKind === "tvEpisode" ? epStart : null,
      episodeEnd: mediaKind === "tvEpisode" ? epEnd : null,
      episodeTitle:
        rowOverride?.episodeTitle ?? (extraType ? fallbackTitle : null),
      extraType,
      ids: {
        imdbId: imdbId.trim() || null,
        tvdbId: tvdbId.trim() || null,
        tmdbId: tmdbId.trim() || null,
      },
      overview: selectedOverview,
      posterUrl: selectedPosterUrl,
    };
  }

  function buildLocalItemMetadata(
    item: IdentifiedVideoFile,
    idx: number,
    totalSelected: number,
  ): MediaMetadata {
    const base = item.fileName.replace(/\.[^.]+$/, "");
    if (item.mediaKind === "movie") {
      if (item.movieMetadata) {
        return {
          ...item.movieMetadata,
          kind: "movie",
          episodeEnd: null,
          episodeTitle: item.extraType
            ? item.episodeTitle || base
            : item.movieMetadata.episodeTitle,
          extraType: item.extraType,
        };
      }
      return {
        kind: "movie",
        title:
          inspectedLocalFile === item.path && mediaTitle.trim()
            ? mediaTitle.trim()
            : totalSelected === 1 && mediaTitle.trim()
              ? mediaTitle.trim()
              : base,
        year:
          inspectedLocalFile === item.path || totalSelected === 1
            ? parsedYear
            : null,
        season: null,
        episode: null,
        episodeEnd: null,
        episodeTitle: item.extraType ? item.episodeTitle || base : null,
        extraType: item.extraType,
        ids:
          inspectedLocalFile === item.path || totalSelected === 1
            ? {
                imdbId: imdbId.trim() || null,
                tvdbId: tvdbId.trim() || null,
                tmdbId: tmdbId.trim() || null,
              }
            : { imdbId: null, tvdbId: null, tmdbId: null },
        overview: null,
        posterUrl: null,
      };
    }

    const showMeta = item.seriesMetadata ?? sharedTvShow;
    const tvTitle = showMeta?.title || mediaTitle.trim() || base;
    const tvYear = showMeta?.year ?? parsedYear;
    const tvIds: ExternalIds = showMeta?.ids ?? {
      imdbId: imdbId.trim() || null,
      tvdbId: tvdbId.trim() || null,
      tmdbId: tmdbId.trim() || null,
    };
    const epSeason = item.season ?? parsedSeason;
    const epNum = item.episode ?? parsedStartEpisode + idx;
    const epEnd =
      item.episodeEnd && item.episodeEnd > epNum ? item.episodeEnd : null;
    return {
      kind: "tvEpisode",
      title: tvTitle,
      year: tvYear,
      season: epSeason,
      episode: epNum,
      episodeEnd: epEnd,
      episodeTitle: item.extraType
        ? item.episodeTitle || base
        : item.episodeTitle,
      extraType: item.extraType,
      ids: tvIds,
      overview: showMeta?.overview ?? selectedOverview,
      posterUrl: showMeta?.posterUrl ?? selectedPosterUrl,
    };
  }

  const organizeFileRequests = $derived.by<OrganizeFileRequest[]>(() => {
    if (!isOrganizeMode) return [];
    const ordered = localVideoItems.filter((item) =>
      checkedLocalFiles.includes(item.path),
    );
    return ordered.map((item, idx) => ({
      sourcePath: item.path,
      mediaMetadata: buildLocalItemMetadata(item, idx, ordered.length),
    }));
  });

  function handleOrganizeCompleted(result: OrganizeExecutionResult): void {
    organizeReviewOpen = false;
    lastOrganizeResult = result;
    organizeUndoBanner = null;
    queueActionError =
      result.errors.length > 0 ? result.errors.join(" · ") : null;

    if (result.transferMode === "move" && result.operations.length > 0) {
      const movedMap = new Map<string, string>();
      for (const op of result.operations) {
        if (!op.isSidecar) {
          movedMap.set(op.sourcePath, op.targetPath);
        }
      }
      if (movedMap.size > 0) {
        localVideoItems = localVideoItems.map((item) => {
          const nextPath = movedMap.get(item.path);
          if (!nextPath) return item;
          return {
            ...item,
            path: nextPath,
            fileName: fileBaseName(nextPath),
          };
        });
        checkedLocalFiles = checkedLocalFiles.map(
          (p) => movedMap.get(p) ?? p,
        );
        if (inspectedLocalFile && movedMap.has(inspectedLocalFile)) {
          inspectedLocalFile = movedMap.get(inspectedLocalFile) ?? null;
        }
      }
    }
  }

  async function handleUndoLastOrganize(): Promise<void> {
    if (undoingOrganize) return;
    undoingOrganize = true;
    queueActionError = null;
    try {
      const res = await invoke<UndoOrganizeResult>("undo_last_organize");
      lastOrganizeResult = null;
      organizeUndoBanner = res.statusMessage;
      if (res.errors.length > 0) {
        queueActionError = res.errors.join(" · ");
      }
      if (sourceFolderPath) {
        await rescanConnectedFolder();
      }
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
    } finally {
      undoingOrganize = false;
    }
  }

  function handleRequestEnqueue(): void {
    if (!modeReady || selectedCount === 0 || queueingJobs) return;
    if (workflowMode === "organizeOnly") {
      organizeReviewOpen = true;
      return;
    }
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
    if (!modeReady || selectedCount === 0 || workflowMode === "organizeOnly") {
      return;
    }
    const pipelineMode: PipelineMode = workflowMode;
    queueingJobs = true;
    queueActionError = null;

    try {
      const requests: EnqueueJobRequest[] = [];
      const activeDrive = drives.find((d) => d.index === selectedDriveIndex);
      const effectiveOutputDir =
        settings.outputDir || binaries?.defaultPaths.outputDir || "";
      const effectiveTempDir =
        settings.tempDir || binaries?.defaultPaths.tempDir || "";

      if (pipelineMode === "encodeOnly") {
        const ordered = localVideoItems.filter((item) =>
          checkedLocalFiles.includes(item.path),
        );
        const localTrackSelection = buildTrackSelectionForTitle(null);
        ordered.forEach((item, idx) => {
          const meta = buildLocalItemMetadata(item, idx, ordered.length);
          if (item.mediaKind === "movie") {
            const titleLabel = meta.extraType
              ? `${meta.title} (${extraTypeSuffix(meta.extraType).slice(1)})`
              : meta.year
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
              notifyOnComplete: settings.desktopNotifications,
              namingConvention,
              folderOrganization: folderOrg,
              mediaMetadata: meta,
              trackSelection: localTrackSelection,
            });
          } else {
            const epSeason = meta.season ?? parsedSeason;
            const epNum = meta.episode ?? parsedStartEpisode + idx;
            const baseCode = `S${String(epSeason).padStart(2, "0")}E${String(epNum).padStart(2, "0")}`;
            const epCode =
              meta.episodeEnd && meta.episodeEnd > epNum
                ? `${baseCode}-E${String(meta.episodeEnd).padStart(2, "0")}`
                : baseCode;
            const titleLabel = meta.extraType
              ? `${meta.title} · ${meta.episodeTitle ?? item.fileName} (${extraTypeSuffix(meta.extraType).slice(1)})`
              : meta.episodeTitle
                ? `${meta.title} ${epCode} - ${meta.episodeTitle}`
                : `${meta.title} ${epCode}`;
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
              notifyOnComplete: settings.desktopNotifications,
              namingConvention,
              folderOrganization: folderOrg,
              mediaMetadata: meta,
              trackSelection: localTrackSelection,
            });
          }
        });
      } else {
        const ordered = titles.filter((t) =>
          checkedTitleIds.includes(t.titleId),
        );
        ordered.forEach((title, idx) => {
          const fallback = title.name ?? `Title_${title.titleId}`;
          const rowExtra = titleExtraById[title.titleId] ?? null;
          const rowEpEnd = titleEpisodeEndById[title.titleId] ?? null;
          const epStart = parsedStartEpisode + idx;
          const baseCode = episodeCodeForRow(idx);
          const rangeCode =
            rowEpEnd && rowEpEnd > epStart
              ? `${baseCode}-E${String(rowEpEnd).padStart(2, "0")}`
              : baseCode;
          const titleLabel = rowExtra
            ? `${mediaTitle.trim() || fallback} (${extraTypeSuffix(rowExtra).slice(1)})`
            : mediaTitle.trim() && mediaKind === "tvEpisode"
              ? `${mediaTitle.trim()} ${rangeCode}`
              : ordered.length === 1 && mediaTitle.trim()
                ? mediaTitle.trim()
                : fallback;
          requests.push({
            mode: pipelineMode,
            discName: activeDiscName || activeDrive?.discName || "Optical Disc",
            driveIndex: selectedDriveIndex,
            driveDevicePath: activeDrive?.devicePath ?? null,
            titleId: title.titleId,
            sourceFilePath: null,
            titleName: titleLabel,
            qualityProfile:
              pipelineMode === "ripOnly" ? null : activeQualityProfile,
            customPresetFile:
              pipelineMode === "ripOnly" ? null : settings.customPresetFile,
            customPresetName:
              pipelineMode === "ripOnly" ? null : settings.defaultPreset,
            tempDir: effectiveTempDir,
            outputDir: effectiveOutputDir,
            keepTempMkv: settings.keepTempMkv,
            autoEjectDisc: settings.autoEjectDisc && idx === ordered.length - 1,
            notifyOnComplete: settings.desktopNotifications,
            namingConvention,
            folderOrganization: folderOrg,
            mediaMetadata: buildJobMetadata(idx, fallback, ordered.length, {
              episodeEnd: rowEpEnd,
              extraType: rowExtra,
            }),
            trackSelection: buildTrackSelectionForTitle(title),
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
      const remaining = await invoke<QueueJob[]>("clear_finished_jobs");
      queueJobs = remaining;
      const keepIds = new Set(remaining.map((j) => j.id));
      const prunedLogs: Record<string, string[]> = {};
      for (const [id, lines] of Object.entries(jobLogs)) {
        if (keepIds.has(id)) {
          prunedLogs[id] = lines;
        }
      }
      jobLogs = prunedLogs;
    } catch {
      // Ignore outside Tauri runtime.
    }
  }

  async function handleRetryJob(jobId: string): Promise<void> {
    queueActionError = null;
    try {
      const retried = await invoke<QueueJob>("retry_job", {
        jobId,
        config: currentBinaryConfig(),
      });
      const idx = queueJobs.findIndex((j) => j.id === retried.id);
      if (idx >= 0) {
        queueJobs[idx] = retried;
      } else {
        queueJobs = [...queueJobs, retried];
      }
      queueExpanded = true;
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
    }
  }

  async function handleRevealPath(path: string): Promise<void> {
    if (!path.trim()) return;
    try {
      await invoke("reveal_in_file_manager", { path });
    } catch (err) {
      queueActionError = err instanceof Error ? err.message : String(err);
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
        if (event.job.stage === "completed") {
          void refreshDiskSpace();
        }
        break;
      }
      case "queueSnapshot": {
        queueJobs = event.jobs;
        queuePaused = event.paused;
        if (!event.paused) {
          restoredPendingCount = 0;
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
      case "logLine": {
        const prefix =
          event.stream === "system"
            ? `[${event.stage}]`
            : `[${event.stage}][${event.stream}]`;
        const formatted = `${prefix} ${event.line}`;
        const existing = jobLogs[event.jobId] ?? [];
        const nextLines =
          existing.length >= MAX_LOG_LINES_PER_JOB
            ? [...existing.slice(-(MAX_LOG_LINES_PER_JOB - 1)), formatted]
            : [...existing, formatted];
        jobLogs = {
          ...jobLogs,
          [event.jobId]: nextLines,
        };
        break;
      }
      case "tempCleaned": {
        const existing = jobLogs[event.jobId] ?? [];
        jobLogs = {
          ...jobLogs,
          [event.jobId]: [
            ...existing.slice(-(MAX_LOG_LINES_PER_JOB - 1)),
            `[cleaningTemp] Removed temp MKV: ${event.removedPath}`,
          ],
        };
        void refreshDiskSpace();
        break;
      }
      case "discEjected": {
        showTransientBanner(
          "Disc Ready to Swap",
          `${event.discName} finished ripping and ejected. Insert the next disc while encoding continues.`,
          "info",
        );
        break;
      }
      case "batchCompleted": {
        showTransientBanner(
          event.title.replace(/^RipDaddy · /, ""),
          event.body,
          event.failed > 0 ? "warn" : "success",
        );
        void refreshDiskSpace();
        break;
      }
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

    // When in Encode Only or Organize mode with a specific Movie row selected, assign this match to THAT row
    if (
      isLocalMode &&
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
    // convert misclassified rows if needed, fetch official episodes, and fuzzy-rematch.
    if (item.kind === "tvEpisode") {
      await applySeriesSelectionToLocalFiles({
        ...item,
        ids: {
          imdbId: imdbId.trim() || null,
          tvdbId: tvdbId.trim() || null,
          tmdbId: tmdbId.trim() || null,
        },
      });
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

  async function setAllLocalMediaKind(targetKind: MediaKind): Promise<void> {
    if (localVideoItems.length === 0) return;
    mediaKind = targetKind;
    detectedFolderKind = targetKind === "tvEpisode" ? "tvSeries" : "movies";

    if (targetKind === "tvEpisode") {
      // If a TV show & episode catalog are already matched, immediately run Rust fuzzy rematch!
      if (sharedTvShow && seriesEpisodeCatalog.length > 0) {
        try {
          const rematched = await invoke<IdentifiedVideoFile[]>(
            "rematch_tv_files",
            {
              filePaths: localVideoItems.map((f) => f.path),
              showTitle: sharedTvShow.title,
              seriesMetadata: sharedTvShow,
              episodesCatalog: seriesEpisodeCatalog,
              defaultSeason: parsedSeason,
            },
          );
          if (rematched.length > 0) {
            localVideoItems = rematched;
            const matchedCount = rematched.filter(
              (f) => f.matchedFromCatalog,
            ).length;
            folderIdentifyStatus = `Matched "${sharedTvShow.title}" · ${matchedCount}/${rematched.length} episodes matched`;
            return;
          }
        } catch {
          // Fall back to local kind conversion below
        }
      }

      localVideoItems = localVideoItems.map((item, idx) => ({
        ...item,
        mediaKind: "tvEpisode",
        season: item.season ?? parsedSeason,
        episode: item.episode ?? parsedStartEpisode + idx,
        needsTitle: false,
      }));
      folderIdentifyStatus = `Switched all ${localVideoItems.length} files to TV episodes — search a show above to auto-match episode titles.`;
    } else {
      localVideoItems = localVideoItems.map((item) => ({
        ...item,
        mediaKind: "movie",
        season: null,
        episode: null,
        episodeEnd: null,
        needsTitle: !item.movieMetadata,
      }));
      folderIdentifyStatus = `Switched all ${localVideoItems.length} files to Movies.`;
    }

    const active = localVideoItems.find((i) => i.path === inspectedLocalFile);
    if (active) {
      syncMetadataStripToItem(active);
    }
  }

  // When picking a TV show with local files loaded, update shared/per-show TV identity,
  // convert misclassified Movie rows to TV if needed, fetch official episode titles,
  // and run Rust's fuzzy multi-season rematch across the matching rows!
  async function applySeriesSelectionToLocalFiles(
    show: MediaMetadata,
  ): Promise<void> {
    const inspectedShowTitle =
      detectedShows.length > 1
        ? inspectedLocalItem?.seriesMetadata?.title ?? null
        : null;
    if (!inspectedShowTitle) {
      sharedTvShow = show;
    }
    if (localVideoItems.length === 0) return;

    const existingTv = localVideoItems.filter(
      (f) => f.mediaKind === "tvEpisode",
    );
    const convertAll = existingTv.length === 0;
    const targetRows = convertAll
      ? localVideoItems
      : inspectedShowTitle
        ? existingTv.filter(
            (f) =>
              f.seriesMetadata?.title.toLowerCase() ===
              inspectedShowTitle.toLowerCase(),
          )
        : existingTv;
    if (convertAll) {
      detectedFolderKind = "tvSeries";
    }

    try {
      const seasonsHint = Array.from(
        new Set(
          targetRows
            .map((f) => f.season)
            .filter((s): s is number => s !== null),
        ),
      );
      const [prov, episodes] = await invoke<[MetadataProvider, EpisodeInfo[]]>(
        "fetch_series_episodes",
        {
          title: show.title,
          ids: show.ids,
          seasonsHint: seasonsHint.length > 0 ? seasonsHint : [parsedSeason],
          config: currentMetadataConfig(),
        },
      );
      if (!inspectedShowTitle) {
        seriesEpisodeCatalog = episodes;
      }
      seriesEpisodesByShow = {
        ...seriesEpisodesByShow,
        [show.title.toLowerCase()]: episodes,
      };
      if (inspectedShowTitle) {
        detectedShows = detectedShows.map((s) =>
          s.title.toLowerCase() === inspectedShowTitle.toLowerCase() ? show : s,
        );
      } else {
        detectedShows = [show];
      }

      // Use Rust's full container-tag + sibling + fuzzy Levenshtein multi-season matcher
      const rematched = await invoke<IdentifiedVideoFile[]>("rematch_tv_files", {
        filePaths: targetRows.map((f) => f.path),
        showTitle: show.title,
        seriesMetadata: show,
        episodesCatalog: episodes,
        defaultSeason: parsedSeason,
      });

      if (rematched.length > 0) {
        if (convertAll) {
          localVideoItems = rematched;
        } else {
          const byPath = new Map(rematched.map((f) => [f.path, f]));
          localVideoItems = localVideoItems.map(
            (row) => byPath.get(row.path) ?? row,
          );
        }
      } else {
        if (convertAll) {
          localVideoItems = localVideoItems.map((row, idx) => ({
            ...row,
            mediaKind: "tvEpisode",
            seriesMetadata: show,
            season: row.season ?? parsedSeason,
            episode: row.episode ?? parsedStartEpisode + idx,
            needsTitle: false,
          }));
        }
        applyEpisodeCatalogToItems(episodes, inspectedShowTitle ?? undefined);
      }

      const tvAfter = localVideoItems.filter(
        (f) => f.mediaKind === "tvEpisode",
      );
      const matchedCount = tvAfter.filter((f) => f.matchedFromCatalog).length;
      folderIdentifyStatus = `Matched "${show.title}" via ${prov.toUpperCase()} · ${matchedCount}/${tvAfter.length} episodes matched`;
    } catch {
      if (convertAll) {
        localVideoItems = localVideoItems.map((row, idx) => ({
          ...row,
          mediaKind: "tvEpisode",
          seriesMetadata: show,
          season: row.season ?? parsedSeason,
          episode: row.episode ?? parsedStartEpisode + idx,
          needsTitle: false,
        }));
      }
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
    if (category === "storage") {
      void refreshDiskSpace();
    } else if (category === "encoding" && !encoderCaps && !probingEncoders) {
      void runProbeEncoders();
    }
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
        return "border-[#9CD9F5] bg-[#E6F6FD] text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]";
      case "audio":
        return "border-[#9BE5C1] bg-[#E6FBF1] text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]";
      case "subtitle":
        return "border-[#F0CB82] bg-[#FDF3DC] text-[#563406] dark:border-[#F8B142]/40 dark:bg-[#F8B142]/15 dark:text-[#FCD283]";
      case "unknown":
        return "border-[#E6E0D6] bg-[#FAF8F4] text-[#61554F] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#B8ABA0]";
    }
  }

  function dismissFirstRunModal(): void {
    firstRunModalOpen = false;
    updateSetting("firstRunSetupDone", true);
  }

  async function initialize(): Promise<void> {
    await loadPersistedSettings();
    const [detected] = await Promise.all([
      runCheckBinaries(),
      refreshQueue(true),
      refreshDiskSpace(),
    ]);

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
    let unlistenDragDrop: UnlistenFn | null = null;
    let mediaQuery: MediaQueryList | null = null;
    const handleMediaChange = (e: MediaQueryListEvent) => {
      systemPrefersDark = e.matches;
    };

    if (typeof window !== "undefined" && window.matchMedia) {
      mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
      systemPrefersDark = mediaQuery.matches;
      mediaQuery.addEventListener("change", handleMediaChange);
    }

    void listen<PipelineEvent>("pipeline-event", (evt) => {
      applyPipelineEvent(evt.payload);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        // Ignore outside Tauri runtime.
      });

    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            draggingOver = true;
          } else if (event.payload.type === "leave") {
            draggingOver = false;
          } else if (event.payload.type === "drop") {
            draggingOver = false;
            void handleDroppedPaths(event.payload.paths);
          }
        })
        .then((fn) => {
          unlistenDragDrop = fn;
        })
        .catch(() => {
          // Ignore outside Tauri runtime.
        });
    } catch {
      // Ignore outside Tauri runtime.
    }

    void initialize();

    return () => {
      if (unlisten) {
        unlisten();
      }
      if (unlistenDragDrop) {
        unlistenDragDrop();
      }
      if (mediaQuery) {
        mediaQuery.removeEventListener("change", handleMediaChange);
      }
    };
  });
</script>

<div
  class="relative flex h-screen w-screen flex-col overflow-hidden bg-[#FAF8F4] text-[#1A1513] dark:bg-[#120E0D] dark:text-[#F5EFE8]"
>
  <!-- Native Drag-and-Drop Overlay -->
  {#if draggingOver}
    <div
      class="pointer-events-none fixed inset-0 z-50 flex items-center justify-center bg-[#FAF8F4]/90 p-6 backdrop-blur-[3px] dark:bg-[#120E0D]/90"
    >
      <div
        class="flex max-w-md flex-col items-center rounded-2xl border-2 border-dashed border-[#1FA868] bg-white px-10 py-8 text-center shadow-[0_24px_60px_rgba(26,21,19,0.16)] dark:border-[#4CDE99] dark:bg-[#1A1513] dark:shadow-[0_24px_60px_rgba(0,0,0,0.55)]"
      >
        <RipDaddyLogo class="mb-3 h-14 w-14" alt="" />
        <p class="text-base font-extrabold tracking-[-0.02em] text-[#1A1513] dark:text-[#F5EFE8]">
          Drop folders or video files
        </p>
        <p class="mt-1.5 text-xs leading-relaxed text-[#61554F] dark:text-[#B8ABA0]">
          Drop a single show, a multi-show <span class="font-mono text-[#1A1513] dark:text-[#F5EFE8]">/TV Shows/</span>
          root library, a movie folder, or loose video files to auto-identify and organize or batch encode.
        </p>
      </div>
    </div>
  {/if}

  <!-- RipDaddy Mascot Brand Ribbon (Mint Tile -> Cap Gold -> Coral Shirt) -->
  <div
    class="h-[3px] w-full shrink-0 bg-gradient-to-r from-[#4CDE99] via-[#F8B142] to-[#FA637C]"
    aria-hidden="true"
  ></div>

  <!-- Row 1: Single Lean RipDaddy App Bar -->
  <header
    class="flex h-12 shrink-0 items-center justify-between gap-4 border-b border-[#E6E0D6] bg-white px-5 dark:border-[#332925] dark:bg-[#1A1513]"
  >
    <!-- Brand -->
    <div class="flex items-center gap-2.5">
      <RipDaddyLogo class="h-8 w-8" alt="" />
      <span class="text-[16px] font-extrabold tracking-[-0.03em] text-[#1A1513] dark:text-[#F5EFE8]">
        Rip<span class="text-[#11824F] dark:text-[#4CDE99]">Daddy</span>
      </span>
    </div>

    <!-- Center: RipDaddy Mascot Workflow Mode Pills -->
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
            ? "border-[#D9415D] bg-[#FA637C] text-[#1A1513] shadow-[0_1px_2px_rgba(0,0,0,0.08)]"
            : "border-[#F8B4C0] bg-[#FFF0F2] text-[#751A2E] hover:bg-[#FFE0E5] dark:border-[#FA637C]/40 dark:bg-[#FA637C]/12 dark:text-[#FF99AA] dark:hover:bg-[#FA637C]/22"
        }`}
      >
        Rip Only
      </button>

      <button
        type="button"
        onclick={() => (workflowMode = "ripAndEncode")}
        class={`rounded-full border px-3 py-1 text-xs font-semibold transition ${
          workflowMode === "ripAndEncode"
            ? "border-[#1FA868] bg-[#4CDE99] text-[#1A1513] shadow-[0_1px_2px_rgba(0,0,0,0.08)]"
            : "border-[#9BE5C1] bg-[#E6FBF1] text-[#0E4D31] hover:bg-[#CEF6E3] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/12 dark:text-[#7AF0B8] dark:hover:bg-[#4CDE99]/22"
        }`}
      >
        Rip + Encode
      </button>

      <button
        type="button"
        onclick={() => (workflowMode = "encodeOnly")}
        class={`rounded-full border px-3 py-1 text-xs font-semibold transition ${
          workflowMode === "encodeOnly"
            ? "border-[#D68D1C] bg-[#F8B142] text-[#1A1513] shadow-[0_1px_2px_rgba(0,0,0,0.08)]"
            : "border-[#F0CB82] bg-[#FDF3DC] text-[#563406] hover:bg-[#FBE4B4] dark:border-[#F8B142]/40 dark:bg-[#F8B142]/12 dark:text-[#FCD283] dark:hover:bg-[#F8B142]/22"
        }`}
      >
        Encode Only
      </button>

      <button
        type="button"
        onclick={() => (workflowMode = "organizeOnly")}
        class={`rounded-full border px-3 py-1 text-xs font-semibold transition ${
          workflowMode === "organizeOnly"
            ? "border-[#1C8EC0] bg-[#5BC0EB] text-[#1A1513] shadow-[0_1px_2px_rgba(0,0,0,0.08)]"
            : "border-[#9CD9F5] bg-[#E6F6FD] text-[#0E5678] hover:bg-[#D1EFFB] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/12 dark:text-[#8BD7F8] dark:hover:bg-[#5BC0EB]/22"
        }`}
      >
        Organize
      </button>
    </div>

    <!-- Right: Dark Mode Toggle + Preview Toggle + Categorized Settings Dropdown -->
    <div class="flex items-center gap-2">
      <button
        type="button"
        onclick={toggleDarkMode}
        aria-pressed={resolvedDark}
        aria-label={resolvedDark
          ? "Switch to Light Mode"
          : "Switch to Warm Espresso Dark Mode"}
        title={resolvedDark
          ? "Warm Espresso Dark Mode active — click for Light Mode"
          : "Switch to Warm Espresso Dark Mode"}
        class="flex items-center gap-1.5 rounded-md border border-[#E6E0D6] bg-[#FAF8F4] px-2 py-1.5 text-xs font-semibold text-[#544943] transition hover:border-[#DAD3C7] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F8B142] dark:hover:border-[#F8B142]/50 dark:hover:bg-[#2B231F]"
      >
        {#if resolvedDark}
          <svg
            class="h-3.5 w-3.5"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <circle cx="12" cy="12" r="4" />
            <path d="M12 2v2" />
            <path d="M12 20v2" />
            <path d="m4.93 4.93 1.41 1.41" />
            <path d="m17.66 17.66 1.41 1.41" />
            <path d="M2 12h2" />
            <path d="M20 12h2" />
            <path d="m6.34 17.66-1.41 1.41" />
            <path d="m19.07 4.93-1.41 1.41" />
          </svg>
        {:else}
          <svg
            class="h-3.5 w-3.5"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
          </svg>
        {/if}
      </button>

      <button
        type="button"
        onclick={() => (previewOpen = !previewOpen)}
        aria-pressed={previewOpen}
        class={`flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-semibold transition ${
          previewOpen
            ? "border-[#1C8EC0] bg-[#E6F6FD] text-[#0E5678] dark:border-[#5BC0EB]/50 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
            : "border-[#E6E0D6] bg-[#FAF8F4] text-[#544943] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB] dark:hover:text-[#F5EFE8]"
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
          if (settingsOpen) {
            void refreshDiskSpace();
          }
        }}
        aria-expanded={settingsOpen}
        aria-haspopup="dialog"
        class={`flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-xs font-semibold transition ${
          settingsOpen
            ? "border-[#1A1513] bg-[#1A1513] text-white dark:border-[#4CDE99] dark:bg-[#4CDE99] dark:text-[#120E0D]"
            : "border-[#DAD3C7] bg-white text-[#1A1513] hover:bg-[#FAF8F4] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2B231F]"
        }`}
      >
        <span
          class={`h-2 w-2 rounded-full ${
            checkingBinaries || installingBinary !== null
              ? "bg-[#B6AEA2]"
              : missingCliCount === 0
                ? "bg-[#4CDE99]"
                : modeReady
                  ? "bg-[#5BC0EB]"
                  : "bg-[#F8B142]"
          }`}
        ></span>
        <span>Settings</span>
        <span class="text-[10px] opacity-70">▾</span>
      </button>
    </div>
  </header>

  <!-- Row 2: Compact Source, Preset, Disk Space & Primary Action Toolbar -->
  <div
    class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-[#E6E0D6] bg-[#FAF8F4] px-5 py-2 dark:border-[#332925] dark:bg-[#120E0D]"
  >
    <div class="flex flex-wrap items-center gap-3">
      {#if requiresMakeMkv}
        <div class="flex items-center gap-2">
          <label
            for="drive-select"
            class="text-[11px] font-semibold text-[#61554F] dark:text-[#B8ABA0]"
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
            class="rounded-md border border-[#DAD3C7] bg-white px-2.5 py-1 text-xs font-medium text-[#1A1513] outline-none focus:border-[#1A1513] disabled:cursor-not-allowed disabled:bg-[#F2EDE5] disabled:text-[#92867F] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8] dark:focus:border-[#4CDE99] dark:disabled:bg-[#221C19] dark:disabled:text-[#6E6058]"
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
            class="rounded-md border border-[#DAD3C7] bg-white px-2.5 py-1 text-xs font-semibold text-[#1A1513] transition hover:bg-[#EDE7DD] disabled:cursor-not-allowed disabled:opacity-50 dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2E2622]"
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
            title={isOrganizeMode
              ? "Connect a movie or TV directory to look up metadata, rename files, and sort into folders"
              : "Pick a TV season or video folder to auto-identify show + episodes and batch encode"}
            class="rounded-md bg-[#11824F] px-3 py-1 text-xs font-semibold text-white transition hover:bg-[#0D6E42] disabled:cursor-not-allowed disabled:bg-[#D6CFC3] disabled:text-[#7D716A] dark:disabled:bg-[#2C2420] dark:disabled:text-[#6E6058]"
          >
            {#if identifyingFolder}
              Identifying...
            {:else if isOrganizeMode}
              {sourceFolderPath ? "Switch Folder" : "+ Connect Folder"}
            {:else}
              + Add Folder
            {/if}
          </button>
          <button
            type="button"
            onclick={() => void pickLocalVideoFiles()}
            disabled={!modeReady || identifyingFolder}
            class="rounded-md border border-[#DAD3C7] bg-white px-2.5 py-1 text-xs font-semibold text-[#1A1513] transition hover:bg-[#EDE7DD] disabled:cursor-not-allowed disabled:opacity-50 dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2E2622]"
          >
            + Add Files
          </button>
          {#if localVideoItems.length > 0}
            <button
              type="button"
              onclick={() => void handleAutoIdentifyLoadedFiles()}
              disabled={identifyingFolder}
              title="Auto-identify movies, show, and episode numbers via IMDb, TVDB, TMDB, or free TVMaze"
              class="rounded-md border border-[#9CD9F5] bg-[#E6F6FD] px-2.5 py-1 text-xs font-semibold text-[#0E5678] transition hover:bg-[#D1EFFB] disabled:opacity-50 dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8] dark:hover:bg-[#5BC0EB]/25"
            >
              {isOrganizeMode ? "Re-Lookup" : "Auto-Identify"}
            </button>
          {/if}
        </div>
      {/if}

      {#if requiresHandBrake}
        <div class="h-4 w-px bg-[#E6E0D6] dark:bg-[#332925]"></div>
        <div class="flex flex-wrap items-center gap-2">
          <!-- Local vs Remote Delivery Pill Toggle -->
          <div
            class="inline-flex rounded-full border border-[#DAD3C7] bg-white p-0.5 text-[11px] dark:border-[#3A2F2A] dark:bg-[#1A1513]"
            role="group"
            aria-label="Delivery target"
          >
            <button
              type="button"
              onclick={() =>
                selectQualityTier(settings.activeResolution, "local")}
              class={`rounded-full px-2.5 py-0.5 font-semibold transition ${
                settings.activeDelivery === "local"
                  ? "bg-[#4CDE99] text-[#1A1513]"
                  : "text-[#70645D] hover:text-[#1A1513] dark:text-[#A69990] dark:hover:text-[#F5EFE8]"
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
                  ? "bg-[#5BC0EB] text-[#1A1513]"
                  : "text-[#70645D] hover:text-[#1A1513] dark:text-[#A69990] dark:hover:text-[#F5EFE8]"
              }`}
            >
              Remote
            </button>
          </div>

          <!-- 480p / 720p / 1080p / 4K Resolution Selector -->
          <div
            class="inline-flex rounded-md border border-[#DAD3C7] bg-white p-0.5 font-mono text-[11px] dark:border-[#3A2F2A] dark:bg-[#1A1513]"
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
                    ? "bg-[#1A1513] text-white dark:bg-[#F5EFE8] dark:text-[#120E0D]"
                    : "text-[#544943] hover:bg-[#FAF8F4] hover:text-[#1A1513] dark:text-[#B8ABA0] dark:hover:bg-[#221C19] dark:hover:text-[#F5EFE8]"
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
              title="Customize this resolution/delivery profile and GPU encoder in Settings"
              class="rounded-md border border-[#DAD3C7] bg-white px-2 py-1 font-mono text-[10px] font-semibold text-[#544943] transition hover:border-[#1A1513] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#CFC5BB] dark:hover:border-[#4CDE99] dark:hover:text-[#F5EFE8]"
            >
              {shortEncoderLabel(activeQualityProfile.encoder)} · RF
              {activeQualityProfile.rfQuality.toFixed(0)} · .{outputContainer}
            </button>
          {/if}
        </div>
      {/if}

      <div class="h-4 w-px bg-[#E6E0D6] dark:bg-[#332925]"></div>

      {#if isOrganizeMode}
        <button
          type="button"
          onclick={() => void pickLocalVideoFolder()}
          class="flex max-w-[340px] items-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-[11px] text-[#61554F] transition hover:border-[#E6E0D6] hover:bg-white hover:text-[#1A1513] dark:text-[#B8ABA0] dark:hover:border-[#3A2F2A] dark:hover:bg-[#1A1513] dark:hover:text-[#F5EFE8]"
          title={sourceFolderPath || "Connect a folder to organize in place"}
        >
          <span class="font-semibold text-[#0E5678] dark:text-[#8BD7F8]">Connected:</span>
          <span class="truncate font-mono text-[#1A1513] dark:text-[#F5EFE8]">
            {sourceFolderPath || "No folder connected"}
          </span>
        </button>
      {:else}
        <button
          type="button"
          onclick={() => openSettingsAt("storage")}
          class="flex max-w-[230px] items-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-[11px] text-[#61554F] transition hover:border-[#E6E0D6] hover:bg-white hover:text-[#1A1513] dark:text-[#B8ABA0] dark:hover:border-[#3A2F2A] dark:hover:bg-[#1A1513] dark:hover:text-[#F5EFE8]"
          title={`Output folder: ${resolvedOutputDir}`}
        >
          <span class="font-semibold text-[#7D716A] dark:text-[#9E9187]">Output:</span>
          <span class="truncate font-mono text-[#1A1513] dark:text-[#F5EFE8]">{resolvedOutputDir}</span>
        </button>

        <!-- Temp / Output Free Disk Space Indicator Pill -->
        {#if storageSpace && (storageSpace.temp || storageSpace.output)}
          {@const tempInfo = storageSpace.temp}
          {@const outInfo = storageSpace.output}
          {@const worstSeverity =
            activeDiskSpaceWarning?.level === "critical" ||
            tempInfo?.severity === "critical" ||
            outInfo?.severity === "critical"
              ? "critical"
              : activeDiskSpaceWarning?.level === "low" ||
                  tempInfo?.severity === "low" ||
                  outInfo?.severity === "low"
                ? "low"
                : "ok"}
          <button
            type="button"
            onclick={() => openSettingsAt("storage")}
            title={storageSpace.sameVolume && outInfo
              ? `Temp & Output share volume ${outInfo.resolvedMount}: ${formatDiskBytes(outInfo.availableBytes)} free of ${formatDiskBytes(outInfo.totalBytes)}`
              : `Temp (${tempInfo?.resolvedMount ?? "?"}): ${tempInfo ? formatDiskBytes(tempInfo.availableBytes) : "?"} free · Output (${outInfo?.resolvedMount ?? "?"}): ${outInfo ? formatDiskBytes(outInfo.availableBytes) : "?"} free`}
            class={`flex items-center gap-1.5 rounded-full border px-2 py-0.5 font-mono text-[10px] font-semibold transition ${
              worstSeverity === "critical"
                ? "border-[#D9415D] bg-[#FFF0F2] text-[#751A2E] hover:bg-[#FFE0E5] dark:border-[#FA637C]/50 dark:bg-[#FA637C]/15 dark:text-[#FF99AA]"
                : worstSeverity === "low"
                  ? "border-[#D68D1C] bg-[#FFF7E6] text-[#5E3906] hover:bg-[#FDEBC4] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
                  : "border-[#DAD3C7] bg-white text-[#544943] hover:border-[#1FA868] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#CFC5BB] dark:hover:border-[#4CDE99]"
            }`}
          >
            <span
              class={`h-1.5 w-1.5 rounded-full ${
                worstSeverity === "critical"
                  ? "bg-[#FA637C]"
                  : worstSeverity === "low"
                    ? "bg-[#F8B142]"
                    : "bg-[#4CDE99]"
              }`}
            ></span>
            {#if storageSpace.sameVolume && outInfo}
              <span>{formatDiskBytes(outInfo.availableBytes)} free</span>
            {:else}
              <span>
                {#if tempInfo && workflowMode !== "encodeOnly"}
                  Temp {formatDiskBytes(tempInfo.availableBytes)}
                {/if}
                {#if tempInfo && outInfo && workflowMode !== "encodeOnly"}
                  ·
                {/if}
                {#if outInfo}
                  Out {formatDiskBytes(outInfo.availableBytes)}
                {/if}
              </span>
            {/if}
          </button>
        {/if}
      {/if}
    </div>

    <!-- Single Contextual Primary CTA + Missing CLI Shortcut -->
    <div class="flex items-center gap-2">
      {#if !modeReady && !checkingBinaries}
        <button
          type="button"
          onclick={() => openSettingsAt("binaries")}
          class="flex items-center gap-1.5 rounded-full border border-[#D68D1C] bg-[#FFF7E6] px-2.5 py-1 text-[11px] font-semibold text-[#5E3906] hover:bg-[#FDEBC4] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283] dark:hover:bg-[#F8B142]/25"
        >
          <span class="h-1.5 w-1.5 rounded-full bg-[#DE941E] dark:bg-[#F8B142]"></span>
          <span>Setup CLIs</span>
        </button>
      {/if}

      <button
        type="button"
        onclick={handleRequestEnqueue}
        disabled={!modeReady || selectedCount === 0 || queueingJobs}
        class="rounded-md bg-[#11824F] px-3.5 py-1.5 text-xs font-semibold text-white shadow-[0_1px_2px_rgba(17,130,79,0.22)] transition hover:bg-[#0D6E42] disabled:cursor-not-allowed disabled:bg-[#C6BEB1] disabled:text-[#70645D] disabled:shadow-none dark:disabled:bg-[#2C2420] dark:disabled:text-[#6E6058]"
      >
        {#if queueingJobs}
          Queueing...
        {:else if isOrganizeMode && selectedCount > 0}
          Preview &amp; Organize {selectedCount} File{selectedCount === 1 ? "" : "s"} →
        {:else if isOrganizeMode}
          Organize Folder →
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
    class="relative shrink-0 space-y-2 border-b border-[#E6E0D6] bg-white px-5 py-2.5 dark:border-[#332925] dark:bg-[#1A1513]"
  >
    <div class="flex flex-wrap items-center gap-2">
      <!-- Movie / TV Toggle -->
      <div
        class="inline-flex rounded-md border border-[#E6E0D6] bg-[#FAF8F4] p-0.5 text-[11px] dark:border-[#3A2F2A] dark:bg-[#120E0D]"
        role="group"
        aria-label="Media type"
      >
        <button
          type="button"
          onclick={() => (mediaKind = "movie")}
          class={`rounded px-2 py-0.5 font-semibold transition ${
            mediaKind === "movie"
              ? "bg-white text-[#1A1513] shadow-[0_1px_1px_rgba(0,0,0,0.06)] dark:bg-[#2A221F] dark:text-[#F5EFE8]"
              : "text-[#70645D] hover:text-[#1A1513] dark:text-[#9E9187] dark:hover:text-[#F5EFE8]"
          }`}
        >
          Movie
        </button>
        <button
          type="button"
          onclick={() => (mediaKind = "tvEpisode")}
          class={`rounded px-2 py-0.5 font-semibold transition ${
            mediaKind === "tvEpisode"
              ? "bg-white text-[#1A1513] shadow-[0_1px_1px_rgba(0,0,0,0.06)] dark:bg-[#2A221F] dark:text-[#F5EFE8]"
              : "text-[#70645D] hover:text-[#1A1513] dark:text-[#9E9187] dark:hover:text-[#F5EFE8]"
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
          class="min-w-0 flex-1 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-2.5 py-1 text-xs text-[#1A1513] placeholder:text-[#A1958E] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#7D716A] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
        />
        <button
          type="button"
          onclick={() => void handleCleanCurrentTitle()}
          title="Clean raw optical disc volume label"
          class="rounded-md border border-[#DAD3C7] bg-white px-2 py-1 text-[11px] font-semibold text-[#544943] hover:bg-[#FAF8F4] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB] dark:hover:bg-[#2E2622] dark:hover:text-[#F5EFE8]"
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
        class="w-16 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-2 py-1 font-mono text-xs text-[#1A1513] placeholder:text-[#A1958E] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#7D716A] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
      />

      <!-- TV Season & Start Episode (inline auto-numbering for TV rows) -->
      {#if mediaKind === "tvEpisode"}
        <div class="flex items-center gap-1">
          <label for="tv-season" class="text-[11px] font-semibold text-[#7D716A] dark:text-[#9E9187]">
            S
          </label>
          <input
            id="tv-season"
            type="text"
            inputmode="numeric"
            bind:value={seasonInput}
            class="w-10 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-1.5 py-1 text-center font-mono text-xs text-[#1A1513] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
          />
          <label
            for="tv-start-ep"
            class="text-[11px] font-semibold text-[#7D716A] dark:text-[#9E9187]"
          >
            E
          </label>
          <input
            id="tv-start-ep"
            type="text"
            inputmode="numeric"
            bind:value={startEpisodeInput}
            title="Starting episode number (rows auto-increment)"
            class="w-10 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-1.5 py-1 text-center font-mono text-xs text-[#1A1513] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
          />
          {#if !isLocalMode && inspectedTitleId !== null}
            <span class="font-mono text-[10px] text-[#7D716A] dark:text-[#9E9187]">-E</span>
            <input
              type="text"
              inputmode="numeric"
              aria-label="Multi-episode end number for selected disc title"
              value={activeInspectedEpisodeEnd ?? ""}
              placeholder="–"
              title="Optional end episode for multi-part disc tracks (e.g. S01E01-E02)"
              onchange={(e) =>
                updateInspectedDiscEpisodeEnd(e.currentTarget.value)}
              class="w-9 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-1 py-1 text-center font-mono text-xs text-[#1A1513] placeholder:text-[#A1958E] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#7D716A] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
            />
          {/if}
          {#if isLocalMode && inspectedLocalItem && inspectedLocalItem.mediaKind === "tvEpisode"}
            {#if activeEpisodeCatalog.length > 0}
              <label for="selected-ep-picker" class="sr-only">
                Pick official episode for selected file
              </label>
              <select
                id="selected-ep-picker"
                value={inspectedLocalItem.season !== null &&
                inspectedLocalItem.episode !== null
                  ? `${inspectedLocalItem.season}:${inspectedLocalItem.episode}`
                  : ""}
                onchange={(e) => {
                  if (inspectedLocalItem) {
                    assignOfficialEpisodeToItem(
                      inspectedLocalItem.path,
                      e.currentTarget.value,
                    );
                  }
                }}
                title="Pick the exact official episode from the show catalog for the selected row"
                class="max-w-[220px] truncate rounded-md border border-[#9CD9F5] bg-[#E6F6FD] px-2 py-1 text-[11px] font-semibold text-[#0E5678] outline-none focus:border-[#11824F] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8] dark:focus:border-[#4CDE99]"
              >
                <option value="">Pick official episode...</option>
                {#each activeEpisodeCatalog as ep (`${ep.season}:${ep.episode}`)}
                  <option value={`${ep.season}:${ep.episode}`}>
                    S{String(ep.season).padStart(2, "0")}E{String(ep.episode).padStart(2, "0")} · {ep.title}
                  </option>
                {/each}
              </select>
            {:else}
              <label for="selected-ep-title" class="sr-only">
                Episode title for selected file
              </label>
              <input
                id="selected-ep-title"
                type="text"
                value={inspectedLocalItem.episodeTitle ?? ""}
                onchange={(e) => {
                  if (inspectedLocalItem) {
                    updateItemEpisodeTitle(
                      inspectedLocalItem.path,
                      e.currentTarget.value,
                    );
                  }
                }}
                placeholder="Episode name"
                class="w-36 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-2 py-1 text-xs text-[#1A1513] placeholder:text-[#A1958E] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#7D716A] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
              />
            {/if}
          {/if}
        </div>
      {/if}

      <!-- Optional Plex/Jellyfin Bonus Extra Tag (for inspected row / disc track) -->
      {#if (isLocalMode && inspectedLocalItem) || (!isLocalMode && inspectedTitle)}
        <label for="media-extra-tag" class="sr-only">Extra or bonus type</label>
        <select
          id="media-extra-tag"
          value={activeInspectedExtraType ?? ""}
          onchange={(e) => updateInspectedExtraType(e.currentTarget.value)}
          title="Tag selected track/file as a Plex/Jellyfin bonus extra (-featurette, -deleted, Specials)"
          class={`rounded-md border px-2 py-1 text-[11px] font-semibold outline-none transition ${
            activeInspectedExtraType
              ? "border-[#F8B4C0] bg-[#FFF0F2] text-[#751A2E] dark:border-[#FA637C]/45 dark:bg-[#FA637C]/15 dark:text-[#FF99AA]"
              : "border-[#DAD3C7] bg-[#FAF8F4] text-[#544943] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB] dark:hover:text-[#F5EFE8]"
          }`}
        >
          <option value="">Main Feature / Ep</option>
          {#each MEDIA_EXTRA_OPTIONS as opt (opt.value)}
            <option value={opt.value}>{opt.label} ({opt.suffix})</option>
          {/each}
        </select>
      {/if}

      <!-- IMDb / TVDB / TMDB Compact ID Inputs -->
      <label for="imdb-id" class="sr-only">IMDb ID or URL</label>
      <input
        id="imdb-id"
        type="text"
        bind:value={imdbId}
        placeholder="IMDb tt..."
        class="w-28 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-2 py-1 font-mono text-[11px] text-[#1A1513] placeholder:text-[#A1958E] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#7D716A] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
      />

      <label for="tvdb-id" class="sr-only">TVDB ID or URL</label>
      <input
        id="tvdb-id"
        type="text"
        bind:value={tvdbId}
        placeholder="TVDB ID"
        class="w-24 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-2 py-1 font-mono text-[11px] text-[#1A1513] placeholder:text-[#A1958E] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#7D716A] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
      />

      <label for="tmdb-id" class="sr-only">TMDB ID or URL</label>
      <input
        id="tmdb-id"
        type="text"
        bind:value={tmdbId}
        placeholder="TMDB ID"
        class="w-24 rounded-md border border-[#DAD3C7] bg-[#F7F4EE] px-2 py-1 font-mono text-[11px] text-[#1A1513] placeholder:text-[#A1958E] outline-none focus:border-[#1A1513] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#7D716A] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
      />

      <!-- Provider Lookup -->
      <div class="flex items-center gap-1">
        <label for="metadata-provider" class="sr-only">Metadata provider</label>
        <select
          id="metadata-provider"
          bind:value={metadataProvider}
          class="rounded-md border border-[#DAD3C7] bg-[#FAF8F4] px-2 py-1 text-[11px] font-semibold text-[#1A1513] outline-none focus:border-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:focus:border-[#4CDE99]"
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
          class="rounded-md bg-[#11824F] px-2.5 py-1 text-[11px] font-semibold text-white transition hover:bg-[#0D6E42] disabled:opacity-50"
        >
          {searchingMetadata ? "..." : "Lookup"}
        </button>
      </div>
    </div>

    <!-- Live Token Chips + Output Path Preview Line -->
    <div class="flex flex-wrap items-center justify-between gap-2 text-[11px]">
      <div class="flex min-w-0 flex-wrap items-center gap-1.5">
        <span
          class="text-[10px] font-bold uppercase tracking-[0.08em] text-[#7D716A] dark:text-[#9E9187]"
        >
          Naming:
        </span>
        {#each activeTokenChips as chip, i (`${chip}-${i}`)}
          <span
            class="inline-flex items-center rounded border border-[#9CD9F5] bg-[#E6F6FD] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
          >
            {chip}
          </span>
        {/each}
        <span class="text-[#B6AEA2] dark:text-[#6E6058]">→</span>
        <code
          class="truncate font-mono text-[11px] text-[#3F3530] dark:text-[#CFC5BB]"
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
            class="rounded border border-[#E8BF6E] bg-[#FFF7E6] px-2 py-0.5 text-[10px] font-semibold text-[#5E3906] hover:bg-[#FDEBC4] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
          >
            {metadataLookupStatus}
          </button>
        {/if}
        <label for="naming-convention" class="sr-only">Naming convention</label>
        <select
          id="naming-convention"
          value={namingConvention}
          onchange={handleNamingConventionChange}
          class="rounded border border-[#DAD3C7] bg-[#FAF8F4] px-2 py-0.5 text-[10px] font-semibold text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8]"
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
        class="absolute top-full right-5 left-5 z-30 mt-1 max-h-80 overflow-y-auto rounded-xl border border-[#DAD3C7] bg-white shadow-[0_18px_45px_rgba(26,21,19,0.14)] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:shadow-[0_18px_45px_rgba(0,0,0,0.55)]"
      >
        <div
          class="flex items-center justify-between border-b border-[#EDE7DE] bg-[#FAF8F4] px-3.5 py-2 text-[11px] dark:border-[#2B221E] dark:bg-[#120E0D]"
        >
          <span class="font-semibold text-[#1A1513] dark:text-[#F5EFE8]">
            Select a match ({searchResults.length})
          </span>
          <button
            type="button"
            onclick={() => (searchResults = [])}
            class="font-medium text-[#7D716A] hover:text-[#1A1513] dark:text-[#9E9187] dark:hover:text-[#F5EFE8]"
          >
            Dismiss
          </button>
        </div>
        <div class="divide-y divide-[#EDE7DD] dark:divide-[#2B221E]">
          {#each searchResults as item, idx (`${item.title}-${item.ids.imdbId ?? item.ids.tvdbId ?? item.ids.tmdbId ?? idx}`)}
            <button
              type="button"
              onclick={() => void applyMetadataResult(item)}
              class="flex w-full items-start gap-3 px-3.5 py-2.5 text-left transition hover:bg-[#EEFCF5] dark:hover:bg-[#4CDE99]/10"
            >
              {#if item.posterUrl}
                <img
                  src={item.posterUrl}
                  alt={item.title}
                  class="h-14 w-10 shrink-0 rounded border border-[#E6E0D6] object-cover dark:border-[#332925]"
                />
              {:else}
                <div
                  class="flex h-14 w-10 shrink-0 items-center justify-center rounded border border-[#E6E0D6] bg-[#FAF8F4] font-mono text-[10px] text-[#9E928B] dark:border-[#332925] dark:bg-[#221C19] dark:text-[#8E8178]"
                >
                  {item.kind === "movie" ? "MOV" : "TV"}
                </div>
              {/if}

              <div class="min-w-0 flex-1 space-y-1">
                <div class="flex flex-wrap items-center gap-1.5">
                  <span class="text-xs font-bold text-[#1A1513] dark:text-[#F5EFE8]">
                    {item.title}
                  </span>
                  {#if item.year}
                    <span class="font-mono text-[11px] text-[#61554F] dark:text-[#B8ABA0]">
                      ({item.year})
                    </span>
                  {/if}
                  <span
                    class="rounded-full border border-[#E6E0D6] bg-[#FAF8F4] px-1.5 py-0.5 text-[9px] font-bold uppercase text-[#61554F] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB]"
                  >
                    {item.kind === "movie" ? "Movie" : "TV"}
                  </span>
                  {#if item.ids.imdbId}
                    <span
                      class="rounded border border-[#9CD9F5] bg-[#E6F6FD] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
                    >
                      imdb:{item.ids.imdbId}
                    </span>
                  {/if}
                  {#if item.ids.tvdbId}
                    <span
                      class="rounded border border-[#9BE5C1] bg-[#E6FBF1] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]"
                    >
                      tvdb:{item.ids.tvdbId}
                    </span>
                  {/if}
                  {#if item.ids.tmdbId}
                    <span
                      class="rounded border border-[#F0CB82] bg-[#FDF3DC] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[#563406] dark:border-[#F8B142]/40 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
                    >
                      tmdb:{item.ids.tmdbId}
                    </span>
                  {/if}
                </div>
                {#if item.overview}
                  <p class="line-clamp-2 text-[11px] leading-snug text-[#70645D] dark:text-[#A69990]">
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

  <!-- Row 4: 2/3 Title Table + Toggleable 1/3 Right Preview Drawer -->
  <div class="flex min-h-0 flex-1 overflow-hidden">
    <main
      class="flex min-w-0 flex-[2_1_0%] flex-col overflow-hidden bg-white dark:bg-[#1A1513]"
    >
      {#if queueActionError}
        <div
          class="flex shrink-0 items-center justify-between gap-3 border-b border-red-200 bg-red-50 px-5 py-2 text-xs text-red-700 dark:border-red-500/40 dark:bg-red-500/15 dark:text-red-300"
        >
          <span class="truncate font-mono text-[11px]">{queueActionError}</span>
          <button
            type="button"
            onclick={() => (queueActionError = null)}
            class="shrink-0 font-semibold text-red-700 hover:underline dark:text-red-300"
          >
            Dismiss
          </button>
        </div>
      {/if}

      {#if notificationBanner}
        <div
          class={`flex shrink-0 items-center justify-between gap-3 border-b px-5 py-1.5 text-xs ${
            notificationBanner.kind === "success"
              ? "border-[#9BE5C1] bg-[#E6FBF1] text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]"
              : notificationBanner.kind === "warn"
                ? "border-[#E8BF6E] bg-[#FFF7E6] text-[#5E3906] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
                : "border-[#9CD9F5] bg-[#E6F6FD] text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
          }`}
        >
          <span class="truncate">
            <strong>{notificationBanner.title}:</strong> {notificationBanner.body}
          </span>
          <button
            type="button"
            onclick={() => (notificationBanner = null)}
            class="shrink-0 text-[11px] font-semibold hover:underline"
          >
            Dismiss
          </button>
        </div>
      {/if}

      {#if activeDiskSpaceWarning}
        <div
          class={`flex shrink-0 items-center justify-between gap-3 border-b px-5 py-1.5 text-xs ${
            activeDiskSpaceWarning.level === "critical"
              ? "border-[#F8B4C0] bg-[#FFF0F2] text-[#751A2E] dark:border-[#FA637C]/45 dark:bg-[#FA637C]/15 dark:text-[#FF99AA]"
              : "border-[#E8BF6E] bg-[#FFF7E6] text-[#5E3906] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
          }`}
        >
          <span class="truncate font-medium">
            ⚠ {activeDiskSpaceWarning.message}
          </span>
          <button
            type="button"
            onclick={() => openSettingsAt("storage")}
            class="shrink-0 rounded border border-current/30 bg-white/60 px-2 py-0.5 text-[10px] font-semibold transition hover:bg-white dark:bg-black/20 dark:hover:bg-black/40"
          >
            Storage Settings
          </button>
        </div>
      {/if}

      {#if lastOrganizeResult && isOrganizeMode}
        <div
          class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-[#9BE5C1] bg-[#E6FBF1] px-5 py-2 text-xs text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]"
        >
          <div class="flex min-w-0 items-center gap-2">
            <span class="h-2 w-2 shrink-0 rounded-full bg-[#1FA868] dark:bg-[#4CDE99]"></span>
            <span class="font-semibold">{lastOrganizeResult.statusMessage}</span>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            {#if sourceFolderPath}
              <button
                type="button"
                onclick={() => {
                  if (sourceFolderPath) {
                    void handleRevealPath(sourceFolderPath);
                  }
                }}
                class="rounded border border-[#1FA868] bg-white px-2 py-0.5 text-[11px] font-semibold text-[#0E4D31] hover:bg-[#CEF6E3] dark:border-[#4CDE99]/40 dark:bg-[#1A1513] dark:text-[#7AF0B8] dark:hover:bg-[#4CDE99]/20"
              >
                Open Folder
              </button>
              <button
                type="button"
                onclick={() => void rescanConnectedFolder()}
                disabled={identifyingFolder}
                class="rounded border border-[#9BE5C1] bg-white px-2 py-0.5 text-[11px] font-semibold text-[#0E4D31] hover:bg-[#CEF6E3] disabled:opacity-50 dark:border-[#4CDE99]/40 dark:bg-[#1A1513] dark:text-[#7AF0B8] dark:hover:bg-[#4CDE99]/20"
              >
                Rescan Folder
              </button>
            {/if}
            {#if lastOrganizeResult.canUndo}
              <button
                type="button"
                onclick={() => void handleUndoLastOrganize()}
                disabled={undoingOrganize}
                class="rounded border border-[#D68D1C] bg-[#FFF7E6] px-2.5 py-0.5 text-[11px] font-semibold text-[#5E3906] hover:bg-[#FDEBC4] disabled:opacity-50 dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
              >
                {undoingOrganize ? "Undoing..." : "↩ Undo Last Organize"}
              </button>
            {/if}
            <button
              type="button"
              onclick={() => (lastOrganizeResult = null)}
              class="text-[11px] font-semibold opacity-80 hover:opacity-100"
            >
              Dismiss
            </button>
          </div>
        </div>
      {:else if organizeUndoBanner && isOrganizeMode}
        <div
          class="flex shrink-0 items-center justify-between gap-2 border-b border-[#9CD9F5] bg-[#E6F6FD] px-5 py-2 text-xs text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
        >
          <span class="font-semibold">↩ {organizeUndoBanner}</span>
          <button
            type="button"
            onclick={() => (organizeUndoBanner = null)}
            class="text-[11px] font-semibold hover:underline"
          >
            Dismiss
          </button>
        </div>
      {/if}

      <!-- Local Mode Batch Summary & Bulk Actions Bar -->
      {#if isLocalMode && (folderIdentifyStatus || localVideoItems.length > 0)}
        <div
          class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-[#E6E0D6] bg-[#F7F4EE] px-5 py-1.5 text-[11px] dark:border-[#332925] dark:bg-[#221C19]"
        >
          <div class="flex min-w-0 flex-wrap items-center gap-2">
            {#if localVideoItems.length > 0}
              <span
                class="rounded-full border border-[#E6E0D6] bg-white px-2 py-0.5 font-mono text-[10px] font-semibold text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8]"
              >
                {localVideoItems.length} file{localVideoItems.length === 1 ? "" : "s"}
                {#if totalLocalSizeBytes > 0}
                  · {formatBytesHuman(totalLocalSizeBytes)}
                {/if}
              </span>
            {/if}
            {#if folderIdentifyStatus}
              <span class="truncate font-medium text-[#0E4D31] dark:text-[#7AF0B8]">
                {folderIdentifyStatus}
              </span>
            {:else if isOrganizeMode}
              <span class="text-[#61554F] dark:text-[#B8ABA0]">
                Lookup &amp; reorganize in place · preserves original extensions &amp; companion <span class="font-mono text-[#1A1513] dark:text-[#F5EFE8]">.srt/.nfo</span> sidecars
              </span>
            {:else if activeQualityProfile}
              <span class="text-[#61554F] dark:text-[#B8ABA0]">
                All checked files encode uniformly to
                <span class="font-mono font-semibold text-[#1A1513] dark:text-[#F5EFE8]">
                  {shortResolutionLabel(activeQualityProfile.resolution)} {activeQualityProfile.delivery} · {shortEncoderLabel(activeQualityProfile.encoder)} · .{outputContainer}
                </span>
              </span>
            {/if}
          </div>

          <div class="flex shrink-0 items-center gap-1.5">
            {#if localVideoItems.length > 0}
              <div
                class="inline-flex rounded border border-[#DAD3C7] bg-white p-0.5 text-[10px] dark:border-[#3A2F2A] dark:bg-[#120E0D]"
                role="group"
                aria-label="Set media type for all loaded files"
              >
                <button
                  type="button"
                  onclick={() => void setAllLocalMediaKind("tvEpisode")}
                  title="Mark all loaded files as TV episodes"
                  class={`rounded px-1.5 py-0.5 font-semibold transition ${
                    activeBatchKind === "tvSeries"
                      ? "bg-[#5BC0EB] text-[#1A1513]"
                      : "text-[#61554F] hover:text-[#1A1513] dark:text-[#A69990] dark:hover:text-[#F5EFE8]"
                  }`}
                >
                  All TV
                </button>
                <button
                  type="button"
                  onclick={() => void setAllLocalMediaKind("movie")}
                  title="Mark all loaded files as Movies"
                  class={`rounded px-1.5 py-0.5 font-semibold transition ${
                    activeBatchKind === "movies"
                      ? "bg-[#4CDE99] text-[#1A1513]"
                      : "text-[#61554F] hover:text-[#1A1513] dark:text-[#A69990] dark:hover:text-[#F5EFE8]"
                  }`}
                >
                  All Movies
                </button>
              </div>
            {/if}
            {#if detectedShows.length > 1}
              <span
                class="rounded-full border border-[#9BE5C1] bg-[#E6FBF1] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]"
              >
                {detectedShows.length} shows
              </span>
            {/if}
            {#if mediaKind === "tvEpisode" && detectedBatchSeasons.length > 1}
              <span
                class="rounded-full border border-[#9CD9F5] bg-[#E6F6FD] px-2 py-0.5 font-mono text-[10px] font-semibold text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
              >
                {detectedBatchSeasons.length} seasons ({detectedBatchSeasons
                  .map((s) => (s === 0 ? "Specials" : `S${String(s).padStart(2, "0")}`))
                  .join(", ")})
              </span>
            {/if}
            {#if folderShowCandidates.length > 1}
              <button
                type="button"
                onclick={() => (searchResults = folderShowCandidates)}
                class="rounded border border-[#9CD9F5] bg-[#E6F6FD] px-2 py-0.5 text-[10px] font-semibold text-[#0E5678] hover:bg-[#D1EFFB] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
              >
                Switch Show ({folderShowCandidates.length})
              </button>
            {/if}
            {#if localVideoItems.length > 0}
              <button
                type="button"
                onclick={clearLocalVideoItems}
                class="rounded border border-[#E6E0D6] bg-white px-2 py-0.5 text-[10px] font-medium text-[#70645D] hover:text-red-600 dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#A69990] dark:hover:text-red-400"
              >
                Clear List
              </button>
            {/if}
          </div>
        </div>
      {/if}

      <!-- Smart Disc Title Filters Bar (Rip Only / Rip + Encode) -->
      {#if !isLocalMode && titles.length > 0}
        <div
          class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-[#E6E0D6] bg-[#F7F4EE] px-5 py-1.5 text-[11px] dark:border-[#332925] dark:bg-[#221C19]"
        >
          <div class="flex flex-wrap items-center gap-1.5">
            <span
              class="rounded-full border border-[#E6E0D6] bg-white px-2 py-0.5 font-mono text-[10px] font-semibold text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#F5EFE8]"
            >
              {titles.length} title{titles.length === 1 ? "" : "s"} · {checkedTitleIds.length} selected
              {#if selectedDiscBytes > 0}
                · {formatBytesHuman(selectedDiscBytes)}
              {/if}
            </span>

            <button
              type="button"
              onclick={selectMainFeatureTitle}
              disabled={discAnalysis.mainFeatureTitleId === null}
              title="Select only the longest title on the disc (Main Feature)"
              class={`rounded-md border px-2 py-0.5 text-[10px] font-semibold transition disabled:opacity-40 ${
                checkedTitleIds.length === 1 &&
                checkedTitleIds[0] === discAnalysis.mainFeatureTitleId
                  ? "border-[#1FA868] bg-[#4CDE99] text-[#1A1513]"
                  : "border-[#9BE5C1] bg-[#E6FBF1] text-[#0E4D31] hover:bg-[#CEF6E3] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8] dark:hover:bg-[#4CDE99]/25"
              }`}
            >
              ★ Select Main Feature
            </button>

            <button
              type="button"
              onclick={selectDiscEpisodeTitles}
              disabled={discAnalysis.episodeTitleIds.length === 0}
              title="Select all titles between 20 and 65 minutes (ideal for TV box-set discs)"
              class={`rounded-md border px-2 py-0.5 text-[10px] font-semibold transition disabled:cursor-not-allowed disabled:opacity-40 ${
                discAnalysis.episodeTitleIds.length > 0 &&
                checkedTitleIds.length === discAnalysis.episodeTitleIds.length &&
                discAnalysis.episodeTitleIds.every((id) =>
                  checkedTitleIds.includes(id),
                )
                  ? "border-[#1C8EC0] bg-[#5BC0EB] text-[#1A1513]"
                  : "border-[#9CD9F5] bg-[#E6F6FD] text-[#0E5678] hover:bg-[#D1EFFB] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8] dark:hover:bg-[#5BC0EB]/25"
              }`}
            >
              Select Episodes (20–65m) · {discAnalysis.episodeTitleIds.length}
            </button>

            {#if checkedTitleIds.length > 0}
              <button
                type="button"
                onclick={selectNoDiscTitles}
                class="rounded border border-[#DAD3C7] bg-white px-2 py-0.5 text-[10px] font-medium text-[#61554F] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#1A1513] dark:text-[#A69990] dark:hover:text-[#F5EFE8]"
              >
                None
              </button>
            {/if}
          </div>

          {#if discAnalysis.duplicateTitleCount > 0}
            <div class="flex flex-wrap items-center gap-1.5">
              <span
                title={discAnalysis.hasObfuscatedPattern
                  ? "Multiple feature-length titles share identical durations — common on obfuscated ScreenPass Blu-rays. Inspect playlist (.mpls) filenames before ripping."
                  : "Multiple titles on this disc share nearly identical runtimes."}
                class="inline-flex items-center gap-1 rounded-full border border-[#D68D1C] bg-[#FFF7E6] px-2 py-0.5 font-mono text-[10px] font-bold text-[#5E3906] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
              >
                ⚠ {discAnalysis.duplicateTitleCount} dup-duration{discAnalysis.hasObfuscatedPattern ? " (obfuscated Blu-ray?)" : ""}
              </span>
              {#if hasCheckedDuplicates}
                <button
                  type="button"
                  onclick={dedupeCheckedDiscTitles}
                  title="Keep only the highest-priority title for each duplicate duration"
                  class="rounded border border-[#D68D1C] bg-white px-2 py-0.5 text-[10px] font-semibold text-[#5E3906] hover:bg-[#FFF7E6] dark:border-[#F8B142]/45 dark:bg-[#1A1513] dark:text-[#FCD283] dark:hover:bg-[#F8B142]/20"
                >
                  Keep 1 / Duration
                </button>
              {/if}
            </div>
          {/if}
        </div>
      {/if}

      <!-- Table Header -->
      {#if isLocalMode}
        <div
          class="grid shrink-0 grid-cols-12 items-center border-b border-[#E6E0D6] bg-[#FAF8F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#7D716A] dark:border-[#332925] dark:bg-[#120E0D] dark:text-[#9E9187]"
        >
          <div class="col-span-1 flex items-center">
            <input
              type="checkbox"
              checked={allRowsChecked}
              disabled={localVideoItems.length === 0}
              onchange={toggleAllRows}
              aria-label="Select all video files"
              class="h-3.5 w-3.5 accent-[#11824F] dark:accent-[#4CDE99]"
            />
          </div>
          <div class="col-span-2">Type / Episode</div>
          <div class="col-span-5">
            {isOrganizeMode
              ? "Current File → Organized Target Path"
              : "Source File → Uniform Output Name"}
          </div>
          <div class="col-span-2 text-right">Length / Size</div>
          <div class="col-span-2 text-right">Actions</div>
        </div>
      {:else}
        <div
          class="grid shrink-0 grid-cols-12 items-center border-b border-[#E6E0D6] bg-[#FAF8F4] px-5 py-2 text-[10px] font-bold uppercase tracking-[0.08em] text-[#7D716A] dark:border-[#332925] dark:bg-[#120E0D] dark:text-[#9E9187]"
        >
          <div class="col-span-1 flex items-center">
            <input
              type="checkbox"
              checked={allRowsChecked}
              disabled={titles.length === 0}
              onchange={toggleAllRows}
              aria-label="Select all disc titles"
              class="h-3.5 w-3.5 accent-[#11824F] dark:accent-[#4CDE99]"
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
        {#if isLocalMode}
          {#if localVideoItems.length === 0}
            <div
              class="flex h-full flex-col items-center justify-center px-6 py-16 text-center"
            >
              <div
                class="mb-3 flex h-11 w-11 items-center justify-center rounded-full border border-[#E6E0D6] bg-[#FAF8F4] text-[#61554F] dark:border-[#332925] dark:bg-[#221C19] dark:text-[#B8ABA0]"
              >
                {#if isOrganizeMode}
                  <svg
                    class="h-5 w-5 text-[#0E5678] dark:text-[#5BC0EB]"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    aria-hidden="true"
                  >
                    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z" />
                    <path d="m9 14 2 2 4-4" />
                  </svg>
                {:else}
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
                {/if}
              </div>
              <h2 class="text-sm font-bold tracking-[-0.01em] text-[#1A1513] dark:text-[#F5EFE8]">
                {isOrganizeMode
                  ? "Connect a folder to look up, rename & reorganize"
                  : "Add a TV season folder or video files to batch encode"}
              </h2>
              <p class="mt-1 max-w-md text-xs leading-relaxed text-[#7D716A] dark:text-[#9E9187]">
                {#if isOrganizeMode}
                  Already have the encodes you want? Connect any messy movie or TV
                  directory — RipDaddy runs IMDb / TVDB / TMDB / TVMaze lookups,
                  previews every rename and <span class="font-mono text-[#1A1513] dark:text-[#F5EFE8]">.srt/.nfo</span>
                  sidecar, and reorganizes in place with one-click Undo. No MakeMKV
                  or HandBrake required.
                {:else}
                  Drop in an entire folder of oversized <span class="font-mono text-[#1A1513] dark:text-[#F5EFE8]">.mkv</span>
                  or <span class="font-mono text-[#1A1513] dark:text-[#F5EFE8]">.mp4</span> episodes. RipDaddy
                  automatically identifies the show and episodes via TVDB/TMDB/TVMaze,
                  uniforms all filenames, generates trickplay frames, and encodes the whole
                  folder with one quality profile.
                {/if}
              </p>
              <div class="mt-4 flex flex-wrap items-center justify-center gap-2">
                <button
                  type="button"
                  onclick={() => void pickLocalVideoFolder()}
                  disabled={!modeReady || identifyingFolder}
                  class="rounded-md bg-[#11824F] px-4 py-2 text-xs font-semibold text-white transition hover:bg-[#0D6E42] disabled:cursor-not-allowed disabled:bg-[#D6CFC3] disabled:text-[#7D716A] dark:disabled:bg-[#2C2420] dark:disabled:text-[#6E6058]"
                >
                  {#if identifyingFolder}
                    Scanning Folder...
                  {:else if isOrganizeMode}
                    + Connect Folder to Organize
                  {:else}
                    + Add TV / Video Folder
                  {/if}
                </button>
                <button
                  type="button"
                  onclick={() => void pickLocalVideoFiles()}
                  disabled={!modeReady || identifyingFolder}
                  class="rounded-md border border-[#DAD3C7] bg-white px-4 py-2 text-xs font-semibold text-[#1A1513] transition hover:bg-[#FAF8F4] disabled:cursor-not-allowed disabled:opacity-50 dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2E2622]"
                >
                  + Add Video Files
                </button>
              </div>
            </div>
          {:else}
            <div class="divide-y divide-[#EDE7DD] dark:divide-[#2B221E]">
              {#each localVideoItems as item, idx (item.path)}
                <div
                  class={`grid grid-cols-12 items-center gap-2 px-5 py-2.5 text-xs transition ${
                    inspectedLocalFile === item.path
                      ? "bg-[#EEFCF5] dark:bg-[#4CDE99]/10"
                      : "hover:bg-[#F7F4EE] dark:hover:bg-[#221C19]"
                  }`}
                >
                  <div class="col-span-1 flex items-center">
                    <input
                      type="checkbox"
                      checked={checkedLocalFiles.includes(item.path)}
                      onchange={() => toggleLocalFileChecked(item.path)}
                      aria-label={`Select ${item.fileName}`}
                      class="h-3.5 w-3.5 accent-[#11824F] dark:accent-[#4CDE99]"
                    />
                  </div>

                  <div class="col-span-2 flex items-center gap-1">
                    {#if item.mediaKind === "tvEpisode"}
                      <button
                        type="button"
                        onclick={() => toggleItemMediaKind(item.path)}
                        title="Detected as TV episode (click to switch to Movie)"
                        class="rounded border border-[#9CD9F5] bg-[#E6F6FD] px-1 py-0.5 font-mono text-[9px] font-bold uppercase text-[#0E5678] hover:bg-[#D1EFFB] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
                      >
                        TV
                      </button>
                      <input
                        type="text"
                        inputmode="numeric"
                        aria-label={`Season for ${item.fileName}`}
                        title="Season (0 = Specials)"
                        value={item.season ?? parsedSeason}
                        onchange={(e) =>
                          updateItemSeasonOrEpisode(
                            item.path,
                            "season",
                            e.currentTarget.value,
                          )}
                        class="w-6 rounded border border-[#DAD3C7] bg-white px-0.5 py-0.5 text-center font-mono text-[10px] font-semibold text-[#1A1513] outline-none focus:border-[#11824F] dark:border-[#3A2F2A] dark:bg-[#120E0D] dark:text-[#F5EFE8] dark:focus:border-[#4CDE99]"
                      />
                      <span class="font-mono text-[9px] text-[#7D716A] dark:text-[#9E9187]">×</span>
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
                        class="w-6 rounded border border-[#DAD3C7] bg-white px-0.5 py-0.5 text-center font-mono text-[10px] font-semibold text-[#1A1513] outline-none focus:border-[#11824F] dark:border-[#3A2F2A] dark:bg-[#120E0D] dark:text-[#F5EFE8] dark:focus:border-[#4CDE99]"
                      />
                      <span class="font-mono text-[9px] text-[#7D716A] dark:text-[#9E9187]">-</span>
                      <input
                        type="text"
                        inputmode="numeric"
                        aria-label={`End episode for ${item.fileName}`}
                        placeholder="–"
                        title="Optional end episode for multi-part files (e.g. S01E01-E02)"
                        value={item.episodeEnd ?? ""}
                        onchange={(e) =>
                          updateItemSeasonOrEpisode(
                            item.path,
                            "episodeEnd",
                            e.currentTarget.value,
                          )}
                        class="w-6 rounded border border-[#E6E0D6] bg-[#FAF8F4] px-0.5 py-0.5 text-center font-mono text-[10px] font-semibold text-[#1A1513] placeholder:text-[#B6AEA2] outline-none focus:border-[#11824F] focus:bg-white dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:placeholder:text-[#6E6058] dark:focus:border-[#4CDE99] dark:focus:bg-[#120E0D]"
                      />
                    {:else}
                      <button
                        type="button"
                        onclick={() => toggleItemMediaKind(item.path)}
                        title="Detected as Movie (click to switch to TV episode)"
                        class="rounded border border-[#9BE5C1] bg-[#E6FBF1] px-1.5 py-0.5 font-mono text-[9px] font-bold uppercase text-[#0E4D31] hover:bg-[#CEF6E3] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]"
                      >
                        Movie
                      </button>
                      {#if item.needsTitle}
                        <button
                          type="button"
                          onclick={() => inspectLocalFile(item.path)}
                          class="rounded border border-[#E8BF6E] bg-[#FFF7E6] px-1.5 py-0.5 text-[9px] font-semibold text-[#5E3906] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
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
                          class="rounded border border-[#E6E0D6] bg-white px-1.5 py-0.5 text-[9px] font-semibold text-[#61554F] hover:text-[#1A1513] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#CFC5BB] dark:hover:text-[#F5EFE8]"
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
                      {#if detectedShows.length > 1 && item.mediaKind === "tvEpisode" && item.seriesMetadata}
                        <span
                          class="max-w-[110px] shrink-0 truncate rounded border border-[#9BE5C1] bg-[#E6FBF1] px-1.5 py-0.2 text-[9px] font-bold text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]"
                          title={item.seriesMetadata.title}
                        >
                          {item.seriesMetadata.title}
                        </span>
                      {/if}
                      <span class="truncate font-semibold text-[#1A1513] hover:text-[#11824F] dark:text-[#F5EFE8] dark:hover:text-[#4CDE99]">
                        {item.fileName}
                      </span>
                      {#if item.extraType}
                        <span
                          class="shrink-0 rounded border border-[#F8B4C0] bg-[#FFF0F2] px-1.5 py-0.2 font-mono text-[9px] font-semibold text-[#751A2E] dark:border-[#FA637C]/45 dark:bg-[#FA637C]/15 dark:text-[#FF99AA]"
                        >
                          {extraTypeSuffix(item.extraType)}
                        </span>
                      {/if}
                      {#if item.matchedFromCatalog}
                        <span
                          title="Matched from online catalog"
                          class="h-1.5 w-1.5 shrink-0 rounded-full bg-[#4CDE99]"
                        ></span>
                      {/if}
                    </div>
                    <div
                      class="truncate font-mono text-[10px] text-[#0E5678] dark:text-[#8BD7F8]"
                      title={formatItemTargetFilename(item, idx)}
                    >
                      → {formatItemTargetFilename(item, idx)}
                    </div>
                  </button>

                  <div class="col-span-2 text-right font-mono text-[11px]">
                    {#if item.durationSeconds}
                      <span class="text-[#7D716A] dark:text-[#9E9187]">
                        {formatDurationShort(item.durationSeconds)} ·
                      </span>
                    {/if}
                    <span
                      class={(item.fileSizeBytes ?? 0) >= 8_000_000_000
                        ? "font-semibold text-[#AF6C0B] dark:text-[#F8B142]"
                        : "text-[#544943] dark:text-[#CFC5BB]"}
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
                          ? "border-[#1C8EC0] bg-[#E6F6FD] text-[#0E5678] dark:border-[#5BC0EB]/50 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
                          : "border-[#DAD3C7] bg-white text-[#1A1513] hover:bg-[#FAF8F4] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2E2622]"
                      }`}
                    >
                      {inspectedLocalFile === item.path ? "Viewing" : "Frames"}
                    </button>
                    <button
                      type="button"
                      onclick={() => removeLocalSourceFile(item.path)}
                      class="text-[11px] font-medium text-[#7D716A] hover:text-red-600 dark:text-[#9E9187] dark:hover:text-red-400"
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
              class="mb-3 flex h-11 w-11 items-center justify-center rounded-full border border-[#E6E0D6] bg-[#FAF8F4] text-[#61554F] dark:border-[#332925] dark:bg-[#221C19] dark:text-[#B8ABA0]"
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
            <h2 class="text-sm font-bold tracking-[-0.01em] text-[#1A1513] dark:text-[#F5EFE8]">
              {scanningDrives || scanningDisc
                ? "Scanning optical disc..."
                : "No disc titles loaded"}
            </h2>
            <p class="mt-1 max-w-md text-xs leading-relaxed text-[#7D716A] dark:text-[#9E9187]">
              {#if scanStatusMessage}
                {scanStatusMessage}
              {:else}
                Insert a DVD or Blu-ray and click
                <span class="font-semibold text-[#1A1513] dark:text-[#F5EFE8]">Scan</span> to populate
                titles, durations, chapters, and selectable video/audio/subtitle
                track pills (filtering titles shorter than
                <span class="font-mono text-[#1A1513] dark:text-[#F5EFE8]">{settings.minTitleLengthSeconds}s</span>).
              {/if}
            </p>
            {#if !modeReady}
              <button
                type="button"
                onclick={() => openSettingsAt("binaries")}
                class="mt-4 rounded-md border border-[#D68D1C] bg-[#FFF7E6] px-3.5 py-1.5 text-xs font-semibold text-[#5E3906] hover:bg-[#FDEBC4] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
              >
                Open Settings → External CLIs
              </button>
            {:else if !scanningDrives && !scanningDisc}
              <button
                type="button"
                onclick={() => void handleScanDrives()}
                class="mt-4 rounded-md bg-[#11824F] px-4 py-2 text-xs font-semibold text-white transition hover:bg-[#0D6E42]"
              >
                Scan Optical Drives
              </button>
            {/if}
          </div>
        {:else}
          <div class="divide-y divide-[#EDE7DD] dark:divide-[#2B221E]">
            {#each titles as title, idx (title.titleId)}
              {@const excludedStreams = excludedStreamIdsByTitle[title.titleId] ?? []}
              {@const rowExtra = titleExtraById[title.titleId] ?? null}
              {@const rowEpEnd = titleEpisodeEndById[title.titleId] ?? null}
              {@const isMainFeature = discAnalysis.mainFeatureTitleId === title.titleId}
              {@const dupGroup = discAnalysis.duplicateByTitleId[title.titleId] ?? null}
              <div
                class={`grid grid-cols-12 items-center gap-2 px-5 py-2.5 text-xs transition ${
                  inspectedTitleId === title.titleId
                    ? "bg-[#EEFCF5] dark:bg-[#4CDE99]/10"
                    : "hover:bg-[#F7F4EE] dark:hover:bg-[#221C19]"
                }`}
              >
                <div class="col-span-1 flex items-center">
                  <input
                    type="checkbox"
                    checked={checkedTitleIds.includes(title.titleId)}
                    onchange={() => toggleTitleChecked(title.titleId)}
                    aria-label={`Select title ${title.titleId}`}
                    class="h-3.5 w-3.5 accent-[#11824F] dark:accent-[#4CDE99]"
                  />
                </div>
                <div class="col-span-4 flex min-w-0 flex-wrap items-center gap-1.5">
                  {#if mediaKind === "tvEpisode" && !rowExtra}
                    <span
                      class="shrink-0 rounded border border-[#9CD9F5] bg-[#E6F6FD] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#0E5678] dark:border-[#5BC0EB]/40 dark:bg-[#5BC0EB]/15 dark:text-[#8BD7F8]"
                    >
                      {rowEpEnd && rowEpEnd > parsedStartEpisode + idx
                        ? `${episodeCodeForRow(idx)}-E${String(rowEpEnd).padStart(2, "0")}`
                        : episodeCodeForRow(idx)}
                    </span>
                  {/if}
                  {#if isMainFeature && titles.length > 1 && mediaKind === "movie"}
                    <span
                      title="Longest title on disc (Main Feature)"
                      class="shrink-0 rounded border border-[#1FA868] bg-[#E6FBF1] px-1.5 py-0.2 font-mono text-[9px] font-bold uppercase text-[#0E4D31] dark:border-[#4CDE99]/40 dark:bg-[#4CDE99]/15 dark:text-[#7AF0B8]"
                    >
                      Main
                    </span>
                  {/if}
                  {#if dupGroup}
                    <span
                      title={`Duplicate duration (${title.duration ?? ""}) shared by ${dupGroup.count} titles — verify playlist (${title.sourceFileName ?? "unknown"}) on obfuscated Blu-rays`}
                      class="shrink-0 rounded border border-[#D68D1C] bg-[#FFF7E6] px-1.5 py-0.2 font-mono text-[9px] font-bold text-[#5E3906] dark:border-[#F8B142]/45 dark:bg-[#F8B142]/15 dark:text-[#FCD283]"
                    >
                      ⚠ Dup ×{dupGroup.count}
                    </span>
                  {/if}
                  {#if rowExtra}
                    <span
                      class="shrink-0 rounded border border-[#F8B4C0] bg-[#FFF0F2] px-1.5 py-0.5 font-mono text-[10px] font-semibold text-[#751A2E] dark:border-[#FA637C]/45 dark:bg-[#FA637C]/15 dark:text-[#FF99AA]"
                    >
                      {extraTypeSuffix(rowExtra)}
                    </span>
                  {/if}
                  <button
                    type="button"
                    onclick={() => inspectTitle(title.titleId)}
                    class="truncate text-left font-semibold text-[#1A1513] hover:text-[#11824F] dark:text-[#F5EFE8] dark:hover:text-[#4CDE99]"
                  >
                    {title.name ?? `Title #${title.titleId}`}
                  </button>
                  {#if dupGroup && title.sourceFileName}
                    <span class="shrink-0 font-mono text-[9px] text-[#7D716A] dark:text-[#9E9187]">
                      {title.sourceFileName}
                    </span>
                  {/if}
                </div>
                <div class="col-span-1 font-mono text-[11px] text-[#544943] dark:text-[#CFC5BB]">
                  {title.duration ?? "--:--"}
                </div>
                <div class="col-span-1 font-mono text-[11px] text-[#544943] dark:text-[#CFC5BB]">
                  {title.sizeHuman ?? "--"}
                </div>
                <div class="col-span-4 flex flex-wrap items-center gap-1">
                  {#each title.streams.slice(0, 6) as track (track.streamId)}
                    {@const toggleable =
                      track.trackType === "audio" ||
                      track.trackType === "subtitle"}
                    {@const excluded = excludedStreams.includes(track.streamId)}
                    {#if toggleable}
                      <button
                        type="button"
                        onclick={() =>
                          toggleTitleStream(title.titleId, track.streamId)}
                        aria-pressed={!excluded}
                        title={`${excluded ? "Excluded" : "Included"} ${track.trackType} (${track.languageName ?? track.languageCode ?? "und"}) — click to toggle`}
                        class={`inline-flex items-center gap-0.5 rounded-full border px-2 py-0.5 text-[10px] font-semibold transition ${
                          excluded
                            ? "border-dashed border-[#D6CFC3] bg-[#FAF8F4] text-[#9E928B] line-through opacity-75 hover:border-[#1A1513] dark:border-[#443731] dark:bg-[#221C19] dark:text-[#8E8178] dark:hover:border-[#F5EFE8]"
                            : `${trackPillClass(track)} hover:brightness-95`
                        }`}
                      >
                        <span>{excluded ? "✕" : "✓"}</span>
                        <span>
                          {track.codecShort ?? track.trackType}{#if track.languageCode}&nbsp;{track.languageCode.toUpperCase()}{/if}
                        </span>
                      </button>
                    {:else}
                      <span
                        class={`rounded-full border px-2 py-0.5 text-[10px] font-semibold ${trackPillClass(track)}`}
                      >
                        {track.codecShort ?? track.trackType}
                      </span>
                    {/if}
                  {/each}
                  {#if title.streams.length > 6}
                    <button
                      type="button"
                      onclick={() => inspectTitle(title.titleId)}
                      class="font-mono text-[10px] font-semibold text-[#0E5678] hover:underline dark:text-[#8BD7F8]"
                    >
                      +{title.streams.length - 6}
                    </button>
                  {/if}
                </div>
                <div class="col-span-1 text-right">
                  <button
                    type="button"
                    onclick={() => inspectTitle(title.titleId)}
                    class="rounded border border-[#DAD3C7] bg-white px-2 py-0.5 text-[10px] font-semibold text-[#1A1513] hover:bg-[#FAF8F4] dark:border-[#3A2F2A] dark:bg-[#221C19] dark:text-[#F5EFE8] dark:hover:bg-[#2E2622]"
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
        selectedTitleDuplicateGroup={inspectedTitleDuplicateGroup}
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
        excludedStreamIds={inspectedTitle
          ? (excludedStreamIdsByTitle[inspectedTitle.titleId] ?? [])
          : []}
        onToggleTrack={(streamId) => {
          if (inspectedTitle) {
            toggleTitleStream(inspectedTitle.titleId, streamId);
          }
        }}
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
    paused={queuePaused}
    {restoredPendingCount}
    {jobLogs}
    onToggleExpanded={() => (queueExpanded = !queueExpanded)}
    onTogglePaused={() => void handleToggleQueuePaused()}
    onCancelJob={(jobId) => void handleCancelJob(jobId)}
    onRetryJob={(jobId) => void handleRetryJob(jobId)}
    onMoveJob={(jobId, action) => void handleMoveJob(jobId, action)}
    onReorderJobToIndex={(jobId, targetIndex) =>
      void handleReorderJobToIndex(jobId, targetIndex)}
    onRemoveJob={(jobId) => void handleRemoveJob(jobId)}
    onRevealPath={(path) => void handleRevealPath(path)}
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
    {encoderCaps}
    {probingEncoders}
    {storageSpace}
    {loadingDiskSpace}
    {testingNotification}
    {notificationTestStatus}
    {settings}
    onClose={() => (settingsOpen = false)}
    onSelectCategory={(cat) => {
      settingsCategory = cat;
      if (cat === "storage") {
        void refreshDiskSpace();
      } else if (cat === "encoding" && !encoderCaps && !probingEncoders) {
        void runProbeEncoders();
      }
    }}
    onUpdateSetting={updateSetting}
    onUpdateQualityProfile={handleUpdateQualityProfile}
    onResetQualityProfiles={(key) => void handleResetQualityProfiles(key)}
    onOpenSetupWizard={() => (firstRunModalOpen = true)}
    onRecheckBinaries={() => void runCheckBinaries()}
    onProbeEncoders={() => void runProbeEncoders()}
    onRefreshDiskSpace={() => void refreshDiskSpace()}
    onTestNotification={() => void handleTestNotification()}
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

  <!-- Direct Folder Lookup, Dry-Run Review & Reorganize Modal (Organize mode) -->
  <OrganizeReviewModal
    open={organizeReviewOpen}
    items={organizeFileRequests}
    batchKind={activeBatchKind}
    {namingConvention}
    {sourceFolderPath}
    defaultOutputDir={resolvedOutputDir === "Auto" ? "" : resolvedOutputDir}
    recommendedOrg={recommendedFolderOrg}
    onCompleted={handleOrganizeCompleted}
    onClose={() => (organizeReviewOpen = false)}
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
