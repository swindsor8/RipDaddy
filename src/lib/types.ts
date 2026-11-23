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

export interface EncodeProgress {
  state: HandBrakeState;
  percent: number;
  fps: number | null;
  avgFps: number | null;
  etaSeconds: number | null;
  pass: number | null;
  passCount: number | null;
}

// ---------------------------------------------------------------------------
// IMDb / TVDB / TMDB Titling & Naming (src-tauri/src/metadata.rs)
// ---------------------------------------------------------------------------

export type MediaKind = "movie" | "tvEpisode";

export type MetadataProvider = "imdb" | "tvdb" | "tmdb" | "tvmaze";

export type NamingConvention = "plex" | "jellyfin" | "plain";

export interface ExternalIds {
  imdbId: string | null;
  tvdbId: string | null;
  tmdbId: string | null;
}

export interface MediaMetadata {
  kind: MediaKind;
  title: string;
  year: number | null;
  season: number | null;
  episode: number | null;
  episodeTitle: string | null;
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

export type PipelineEvent =
  | { type: "jobUpdated"; job: QueueJob }
  | { type: "progress"; jobId: string; progress: JobProgress }
  | { type: "stageChanged"; jobId: string; from: JobStage; to: JobStage }
  | {
      type: "logLine";
      jobId: string;
      stage: JobStage;
      stream: string;
      line: string;
    }
  | { type: "tempCleaned"; jobId: string; removedPath: string };

// ---------------------------------------------------------------------------
// Persistent Local Settings (tauri-plugin-store)
// ---------------------------------------------------------------------------

export type SettingsCategory =
  | "general"
  | "binaries"
  | "storage"
  | "encoding"
  | "metadata";

export interface AppSettings {
  // General
  defaultMode: PipelineMode;
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
  // Metadata & Naming
  namingConvention: NamingConvention;
  defaultMetadataProvider: MetadataProvider;
  omdbApiKey: string | null;
  tvdbApiKey: string | null;
  tmdbApiKey: string | null;
}
