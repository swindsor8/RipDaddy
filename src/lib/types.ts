/**
 * Shared TypeScript interfaces matching RipDaddy's Rust serde payloads (`camelCase`).
 */

// ---------------------------------------------------------------------------
// External Binary Detection & OS Setup (src-tauri/src/detect.rs)
// ---------------------------------------------------------------------------

export type BinaryKind = "makeMkv" | "handBrake";

export type BinarySource =
  | "customPath"
  | "systemPath"
  | "standardInstallDir"
  | "flatpak"
  | "notFound";

export interface ResolvedCommand {
  program: string;
  prefixArgs: string[];
}

export type PackageManagerKind =
  | "winget"
  | "brew"
  | "dnf"
  | "apt"
  | "pacman"
  | "flatpak";

export interface InstallHint {
  packageManager: PackageManagerKind | null;
  command: string;
  officialUrl: string;
  canAutoInstall: boolean;
  note: string;
}

export interface DefaultPaths {
  tempDir: string;
  outputDir: string;
}

export interface BinaryStatus {
  name: string;
  found: boolean;
  path: string | null;
  version: string | null;
  source: BinarySource;
  error: string | null;
  command: ResolvedCommand | null;
  installHint: InstallHint;
}

export interface BinaryPathsConfig {
  makemkvPath?: string | null;
  handbrakePath?: string | null;
}

export interface BinariesCheckResult {
  makemkv: BinaryStatus;
  handbrake: BinaryStatus;
  platform: string;
  availablePackageManagers: PackageManagerKind[];
  defaultPaths: DefaultPaths;
}

export interface BinaryInstallOutcome {
  binary: BinaryKind;
  attempted: boolean;
  succeeded: boolean;
  message: string;
}

export interface AutoSetupResult {
  outcomes: BinaryInstallOutcome[];
  binaries: BinariesCheckResult;
}

// ---------------------------------------------------------------------------
// MakeMKV Drives, Titles & Tracks (src-tauri/src/makemkv.rs)
// ---------------------------------------------------------------------------

export interface OpticalDrive {
  index: number;
  visible: number;
  enabled: number;
  flags: number;
  driveName: string;
  discName: string;
  devicePath: string | null;
}

export type TrackType = "video" | "audio" | "subtitle" | "unknown";

export interface StreamTrack {
  streamId: number;
  trackType: TrackType;
  codecId: string | null;
  codecShort: string | null;
  codecLong: string | null;
  languageCode: string | null;
  languageName: string | null;
  channels: number | null;
  channelLayout: string | null;
  videoSize: string | null;
  frameRate: string | null;
  name: string | null;
}

export interface DiscTitle {
  titleId: number;
  name: string | null;
  chapterCount: number | null;
  duration: string | null;
  durationSeconds: number | null;
  sizeHuman: string | null;
  sizeBytes: number | null;
  sourceFileName: string | null;
  outputFileName: string | null;
  streams: StreamTrack[];
}

export interface DiscScanResult {
  driveIndex: number;
  discName: string;
  titles: DiscTitle[];
}

// ---------------------------------------------------------------------------
// HandBrake Presets & Progress (src-tauri/src/handbrake.rs)
// ---------------------------------------------------------------------------

export type HandBrakeState =
  | "scanning"
  | "working"
  | "muxing"
  | "paused"
  | "workDone"
  | "unknown";

export type ResolutionTier = "p480" | "p720" | "p1080" | "uhd4k";

export type DeliveryTarget = "local" | "remote";

export type VideoEncoder =
  | "x26510Bit"
  | "x265"
  | "x264"
  | "svtAv110Bit"
  | "nvencH265"
  | "vaapiH265"
  | "qsvH265"
  | "vtH265";

export type AudioMode = "passthrough" | "aac51" | "aacStereo";

export type SubtitleMode = "all" | "foreignScan" | "none";

export interface QualityProfile {
  id: string;
  label: string;
  resolution: ResolutionTier;
  delivery: DeliveryTarget;
  encoder: VideoEncoder;
  encoderPreset: string;
  rfQuality: number;
  audioMode: AudioMode;
  audioBitrateKbps: number;
  container: string;
  decomb: boolean;
  webOptimize: boolean;
  subtitleMode: SubtitleMode;
  estimatedBitrateLabel: string;
  extraArgs: string[];
}

export interface HandBrakePreset {
  name: string;
  category: string;
  description: string | null;
  isDefault: boolean;
  isCustom: boolean;
}

export interface EncodeOptions {
  presetName: string;
  customPresetFile: string | null;
  container: string;
  extraArgs: string[];
}

export interface TrackSelectionOptions {
  selectedAudioTracks?: number[] | null;
  selectedSubtitleTracks?: number[] | null;
  audioLanguages: string[];
  subtitleLanguages: string[];
}

export interface EncodeProgress {
  state: HandBrakeState;
  percent: number;
  fps: number | null;
  avgFps: number | null;
  etaSeconds: number | null;
  pass: number | null;
  passCount: number | null;
}

export interface EncoderProbeInfo {
  encoder: VideoEncoder;
  cliName: string;
  isHardware: boolean;
  supported: boolean;
  cliListed: boolean;
  hardwareDetected: boolean;
  reason: string;
}

export interface EncoderCapabilities {
  handbrakeAvailable: boolean;
  handbrakeVersion: string | null;
  encoders: EncoderProbeInfo[];
  recommendedGpuEncoder: VideoEncoder | null;
  summary: string;
}

// ---------------------------------------------------------------------------
// Free Disk Space & OS Notifications (src-tauri/src/system.rs)
// ---------------------------------------------------------------------------

export type DiskSpaceSeverity = "ok" | "low" | "critical";

export interface DiskSpaceInfo {
  path: string;
  resolvedMount: string;
  availableBytes: number;
  totalBytes: number;
  usedBytes: number;
  isTmpfs: boolean;
  severity: DiskSpaceSeverity;
}

export interface StorageSpaceReport {
  temp: DiskSpaceInfo | null;
  output: DiskSpaceInfo | null;
  sameVolume: boolean;
}

// ---------------------------------------------------------------------------
// Auto-Generated Trickplay Preview Frames & Media Info (src-tauri/src/preview.rs)
// ---------------------------------------------------------------------------

export interface VideoMediaInfo {
  width: number | null;
  height: number | null;
  displayWidth: number | null;
  displayHeight: number | null;
  displayAspectRatio: string | null;
  sampleAspectRatio: string | null;
  videoCodec: string | null;
  pixelFormat: string | null;
  fps: number | null;
  durationSeconds: number | null;
  bitrateKbps: number | null;
  fileSizeBytes: number | null;
  audioSummary: string | null;
  subtitleCount: number;
}

export interface PreviewFrame {
  index: number;
  timestampSeconds: number;
  timestampLabel: string;
  path: string;
}

export interface TrickplayPreview {
  sourcePath: string;
  mediaInfo: VideoMediaInfo;
  frames: PreviewFrame[];
  extractor: string;
  cached: boolean;
  note: string | null;
}

// ---------------------------------------------------------------------------
// IMDb / TVDB / TMDB Titling & Naming (src-tauri/src/metadata.rs)
// ---------------------------------------------------------------------------

export type MediaKind = "movie" | "tvEpisode";

export type MetadataProvider = "imdb" | "tvdb" | "tmdb" | "tvmaze";

export type NamingConvention = "plex" | "jellyfin" | "plain";

export type FolderOrganization = "showAndSeason" | "seasonOnly" | "flat";

export type FolderOrganizationPref = "ask" | FolderOrganization;

export type MediaExtraType =
  | "behindTheScenes"
  | "deleted"
  | "featurette"
  | "interview"
  | "scene"
  | "short"
  | "trailer"
  | "other";

export interface EpisodePreviewSample {
  kind: MediaKind;
  title: string;
  year: number | null;
  providerTag: string | null;
  season: number | null;
  episode: number | null;
  episodeEnd?: number | null;
  episodeTitle: string | null;
  extraType?: MediaExtraType | null;
}

export type FolderContentKind = "tvSeries" | "movies" | "mixed";

export interface ExternalIds {
  imdbId: string | null;
  tvdbId: string | null;
  tmdbId: string | null;
}

export interface EpisodeInfo {
  season: number;
  episode: number;
  title: string;
}

export interface IdentifiedVideoFile {
  path: string;
  fileName: string;
  fileSizeBytes: number | null;
  durationSeconds: number | null;
  mediaKind: MediaKind;
  season: number | null;
  episode: number | null;
  episodeEnd: number | null;
  episodeTitle: string | null;
  extraType: MediaExtraType | null;
  seriesMetadata: MediaMetadata | null;
  movieMetadata: MediaMetadata | null;
  movieCandidates: MediaMetadata[];
  matchedFromCatalog: boolean;
  needsTitle: boolean;
}

export interface FolderIdentifyResult {
  folderPath: string | null;
  detectedKind: FolderContentKind;
  inferredShowQuery: string;
  providerUsed: MetadataProvider | null;
  matchedShow: MediaMetadata | null;
  detectedShows: MediaMetadata[];
  showCandidates: MediaMetadata[];
  episodesCatalog: EpisodeInfo[];
  seriesEpisodesByShow: Record<string, EpisodeInfo[]>;
  files: IdentifiedVideoFile[];
  totalSizeBytes: number;
  statusMessage: string;
}

export interface MediaMetadata {
  kind: MediaKind;
  title: string;
  year: number | null;
  season: number | null;
  episode: number | null;
  episodeEnd?: number | null;
  episodeTitle: string | null;
  extraType?: MediaExtraType | null;
  ids: ExternalIds;
  overview: string | null;
  posterUrl: string | null;
}

export interface DiscLabelGuess {
  cleanedTitle: string;
  guessedYear: number | null;
  guessedSeason: number | null;
}

export interface MetadataSearchQuery {
  query: string;
  year: number | null;
  kind: MediaKind;
  provider: MetadataProvider;
}

export interface MetadataProviderConfig {
  omdbApiKey?: string | null;
  tvdbApiKey?: string | null;
  tmdbApiKey?: string | null;
}

// ---------------------------------------------------------------------------
// Decoupled Two-Stage Pipeline & Queue (src-tauri/src/pipeline.rs)
// ---------------------------------------------------------------------------

export type PipelineMode = "ripOnly" | "encodeOnly" | "ripAndEncode";

export type WorkspaceMode = PipelineMode | "organizeOnly";

// ---------------------------------------------------------------------------
// Direct Folder Lookup, Rename & Reorganize (src-tauri/src/organize.rs)
// ---------------------------------------------------------------------------

export type OrganizeTransferMode = "move" | "copy";

export interface OrganizeFileRequest {
  sourcePath: string;
  mediaMetadata: MediaMetadata;
}

export interface OrganizePlanRequest {
  items: OrganizeFileRequest[];
  destinationDir: string;
  namingConvention: NamingConvention;
  folderOrganization: FolderOrganization;
  transferMode: OrganizeTransferMode;
  includeSidecars: boolean;
  pruneEmptyDirs: boolean;
  sourceRootDir: string | null;
}

export type OrganizeItemStatus =
  | "ready"
  | "unchanged"
  | "conflict"
  | "missingSource";

export interface PlannedSidecarOp {
  sourcePath: string;
  targetPath: string;
  suffix: string;
}

export interface PlannedOrganizeItem {
  sourcePath: string;
  targetPath: string;
  relativeTargetPath: string;
  status: OrganizeItemStatus;
  reason: string | null;
  sidecars: PlannedSidecarOp[];
}

export interface OrganizePlan {
  destinationDir: string;
  transferMode: OrganizeTransferMode;
  includeSidecars: boolean;
  pruneEmptyDirs: boolean;
  sourceRootDir: string | null;
  items: PlannedOrganizeItem[];
  readyCount: number;
  unchangedCount: number;
  conflictCount: number;
  sidecarCount: number;
}

export interface CompletedFileOp {
  sourcePath: string;
  targetPath: string;
  transferMode: OrganizeTransferMode;
  isSidecar: boolean;
}

export interface OrganizeExecutionResult {
  batchId: string;
  transferMode: OrganizeTransferMode;
  movedCount: number;
  sidecarsMovedCount: number;
  skippedCount: number;
  failedCount: number;
  prunedDirsCount: number;
  canUndo: boolean;
  operations: CompletedFileOp[];
  errors: string[];
  statusMessage: string;
}

export interface UndoOrganizeResult {
  restoredCount: number;
  failedCount: number;
  errors: string[];
  statusMessage: string;
}

export type JobStage =
  | "queuedForRip"
  | "ripping"
  | "queuedForEncode"
  | "encoding"
  | "cleaningTemp"
  | "completed"
  | "cancelled"
  | "failed";

export interface JobProgress {
  stage: JobStage;
  percent: number;
  etaSeconds: number | null;
  fps: number | null;
  message: string | null;
}

export interface QueueJob {
  id: string;
  mode: PipelineMode;
  discName: string;
  driveIndex: number | null;
  titleId: number | null;
  sourceFilePath: string | null;
  titleName: string;
  presetName: string | null;
  tempDir: string;
  tempMkvPath: string | null;
  outputPath: string;
  keepTempMkv: boolean;
  namingConvention: NamingConvention;
  mediaMetadata: MediaMetadata | null;
  stage: JobStage;
  progress: JobProgress;
  error: string | null;
}

export type ReorderAction = "top" | "up" | "down" | "bottom";

export interface QueueStateSnapshot {
  jobs: QueueJob[];
  paused: boolean;
  restoredPendingCount: number;
}

export interface EnqueueJobRequest {
  mode: PipelineMode;
  discName: string;
  driveIndex: number | null;
  driveDevicePath: string | null;
  titleId: number | null;
  sourceFilePath: string | null;
  titleName: string;
  qualityProfile: QualityProfile | null;
  customPresetFile: string | null;
  customPresetName: string | null;
  tempDir: string;
  outputDir: string;
  keepTempMkv: boolean;
  autoEjectDisc: boolean;
  notifyOnComplete?: boolean;
  namingConvention: NamingConvention;
  folderOrganization: FolderOrganization | null;
  mediaMetadata: MediaMetadata | null;
  trackSelection?: TrackSelectionOptions;
}

export type PipelineEvent =
  | { type: "jobUpdated"; job: QueueJob }
  | { type: "queueSnapshot"; jobs: QueueJob[]; paused: boolean }
  | { type: "progress"; jobId: string; progress: JobProgress }
  | { type: "stageChanged"; jobId: string; from: JobStage; to: JobStage }
  | {
      type: "logLine";
      jobId: string;
      stage: JobStage;
      stream: string;
      line: string;
    }
  | { type: "tempCleaned"; jobId: string; removedPath: string }
  | { type: "discEjected"; jobId: string; discName: string }
  | {
      type: "batchCompleted";
      completed: number;
      failed: number;
      title: string;
      body: string;
    };

// ---------------------------------------------------------------------------
// Persistent Local Settings (tauri-plugin-store)
// ---------------------------------------------------------------------------

export type ThemeMode = "light" | "dark" | "system";

export type AppUpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "upToDate"
  | "error";

export interface AppUpdateState {
  status: AppUpdateStatus;
  currentVersion: string;
  availableVersion: string | null;
  notes: string | null;
  progressPercent: number;
  downloadedBytes: number;
  totalBytes: number | null;
  error: string | null;
  releaseUrl: string;
}

export type SettingsCategory =
  | "general"
  | "binaries"
  | "storage"
  | "encoding"
  | "metadata";

export interface AppSettings {
  // General
  theme: ThemeMode;
  desktopNotifications: boolean;
  autoCheckUpdates: boolean;
  defaultMode: WorkspaceMode;
  autoSetupOnFirstRun: boolean;
  firstRunSetupDone: boolean;
  keepTempMkv: boolean;
  autoEjectDisc: boolean;
  // External CLIs
  makemkvPath: string | null;
  handbrakePath: string | null;
  // Storage & Paths
  tempDir: string | null;
  outputDir: string | null;
  // Encoding & Ripping
  activeResolution: ResolutionTier;
  activeDelivery: DeliveryTarget;
  qualityProfiles: Record<string, QualityProfile>;
  defaultPreset: string | null;
  defaultContainer: string;
  customPresetFile: string | null;
  minTitleLengthSeconds: number;
  preferredAudioLanguages: string[];
  preferredSubtitleLanguages: string[];
  // Metadata & Naming
  namingConvention: NamingConvention;
  tvFolderOrganization: FolderOrganizationPref;
  defaultMetadataProvider: MetadataProvider;
  omdbApiKey: string | null;
  tvdbApiKey: string | null;
  tmdbApiKey: string | null;
}
