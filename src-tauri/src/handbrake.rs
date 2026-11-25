//! HandBrakeCLI Resolution × Delivery quality matrix, multiline `--json` progress parser,
//! `--preset-list` parser, and 10-second sample preview clip generator.

use crate::detect::{self, ResolvedCommand};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::timeout;

const PRESET_LIST_TIMEOUT: Duration = Duration::from_secs(15);
const ENCODER_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const VAINFO_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const SAMPLE_CLIP_TIMEOUT: Duration = Duration::from_secs(180);

/// Errors produced while parsing HandBrakeCLI output or invoking the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandBrakeError {
    InvalidJson(String),
    Io(String),
    Timeout(String),
}

impl fmt::Display for HandBrakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandBrakeError::InvalidJson(msg) => {
                write!(f, "Invalid HandBrakeCLI JSON payload: {msg}")
            }
            HandBrakeError::Io(msg) => write!(f, "HandBrakeCLI execution error: {msg}"),
            HandBrakeError::Timeout(msg) => write!(f, "HandBrakeCLI timed out: {msg}"),
        }
    }
}

impl std::error::Error for HandBrakeError {}

/// Target maximum pixel resolution tier (never upscales smaller sources).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolutionTier {
    P480,
    P720,
    P1080,
    Uhd4k,
}

impl ResolutionTier {
    /// Maximum `(width, height)` bounds passed to `HandBrakeCLI --maxWidth / --maxHeight`.
    pub fn max_dimensions(self) -> (u32, u32) {
        match self {
            ResolutionTier::P480 => (720, 480),
            ResolutionTier::P720 => (1280, 720),
            ResolutionTier::P1080 => (1920, 1080),
            ResolutionTier::Uhd4k => (3840, 2160),
        }
    }
}

/// Playback environment optimizing fidelity vs. WAN streaming bandwidth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryTarget {
    Local,
    Remote,
}

/// Video codec / hardware accelerator passed to `HandBrakeCLI --encoder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoEncoder {
    X26510Bit,
    X265,
    X264,
    SvtAv110Bit,
    NvencH265,
    VaapiH265,
    QsvH265,
    VtH265,
}

impl VideoEncoder {
    /// All supported video encoders exposed in RipDaddy's Quality Matrix.
    pub const ALL: [VideoEncoder; 8] = [
        VideoEncoder::X26510Bit,
        VideoEncoder::X265,
        VideoEncoder::X264,
        VideoEncoder::SvtAv110Bit,
        VideoEncoder::NvencH265,
        VideoEncoder::VaapiH265,
        VideoEncoder::QsvH265,
        VideoEncoder::VtH265,
    ];

    /// Exact `HandBrakeCLI -e` encoder identifier.
    pub fn cli_name(self) -> &'static str {
        match self {
            VideoEncoder::X26510Bit => "x265_10bit",
            VideoEncoder::X265 => "x265",
            VideoEncoder::X264 => "x264",
            VideoEncoder::SvtAv110Bit => "svt_av1_10bit",
            VideoEncoder::NvencH265 => "nvenc_h265",
            VideoEncoder::VaapiH265 => "vaapi_h265",
            VideoEncoder::QsvH265 => "qsv_h265",
            VideoEncoder::VtH265 => "vt_h265",
        }
    }

    /// Friendly display name for badges and summaries.
    pub fn display_name(self) -> &'static str {
        match self {
            VideoEncoder::X26510Bit => "x265 10-bit",
            VideoEncoder::X265 => "x265",
            VideoEncoder::X264 => "x264",
            VideoEncoder::SvtAv110Bit => "SVT-AV1 10-bit",
            VideoEncoder::NvencH265 => "NVENC H.265",
            VideoEncoder::VaapiH265 => "VAAPI H.265",
            VideoEncoder::QsvH265 => "QSV H.265",
            VideoEncoder::VtH265 => "VT H.265",
        }
    }

    /// Returns `true` for hardware GPU encoders (`nvenc_h265`, `vaapi_h265`, `qsv_h265`, `vt_h265`).
    pub fn is_hardware(self) -> bool {
        matches!(
            self,
            VideoEncoder::NvencH265
                | VideoEncoder::VaapiH265
                | VideoEncoder::QsvH265
                | VideoEncoder::VtH265
        )
    }
}

/// Availability and hardware probe details for a single RipDaddy `VideoEncoder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderProbeInfo {
    pub encoder: VideoEncoder,
    pub cli_name: String,
    pub is_hardware: bool,
    pub supported: bool,
    pub cli_listed: bool,
    pub hardware_detected: bool,
    pub reason: String,
}

/// Complete capability report returned by `probe_video_encoders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCapabilities {
    pub handbrake_available: bool,
    pub handbrake_version: Option<String>,
    pub encoders: Vec<EncoderProbeInfo>,
    pub recommended_gpu_encoder: Option<VideoEncoder>,
    pub summary: String,
}

/// Audio encoding / passthrough strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioMode {
    Passthrough,
    Aac51,
    AacStereo,
}

/// Subtitle inclusion strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleMode {
    All,
    ForeignScan,
    None,
}

/// Complete customizable HandBrake quality profile for one `(ResolutionTier, DeliveryTarget)` slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityProfile {
    pub id: String,
    pub label: String,
    pub resolution: ResolutionTier,
    pub delivery: DeliveryTarget,
    pub encoder: VideoEncoder,
    pub encoder_preset: String,
    pub rf_quality: f32,
    pub audio_mode: AudioMode,
    pub audio_bitrate_kbps: u32,
    pub container: String,
    pub decomb: bool,
    pub web_optimize: bool,
    pub subtitle_mode: SubtitleMode,
    pub estimated_bitrate_label: String,
    pub extra_args: Vec<String>,
}

/// Returns the 8 tuned default profiles spanning `480p / 720p / 1080p / 4K` × `Local / Remote`.
pub fn recommended_quality_profiles() -> Vec<QualityProfile> {
    vec![
        QualityProfile {
            id: "p480:local".to_string(),
            label: "480p Local (DVD Archive)".to_string(),
            resolution: ResolutionTier::P480,
            delivery: DeliveryTarget::Local,
            encoder: VideoEncoder::X26510Bit,
            encoder_preset: "slow".to_string(),
            rf_quality: 19.0,
            audio_mode: AudioMode::Passthrough,
            audio_bitrate_kbps: 192,
            container: "mkv".to_string(),
            decomb: true,
            web_optimize: false,
            subtitle_mode: SubtitleMode::All,
            estimated_bitrate_label: "~1.2–2.2 Mbps".to_string(),
            extra_args: Vec::new(),
        },
        QualityProfile {
            id: "p480:remote".to_string(),
            label: "480p Remote (Low Bandwidth)".to_string(),
            resolution: ResolutionTier::P480,
            delivery: DeliveryTarget::Remote,
            encoder: VideoEncoder::X264,
            encoder_preset: "medium".to_string(),
            rf_quality: 22.0,
            audio_mode: AudioMode::AacStereo,
            audio_bitrate_kbps: 128,
            container: "mp4".to_string(),
            decomb: true,
            web_optimize: true,
            subtitle_mode: SubtitleMode::ForeignScan,
            estimated_bitrate_label: "~0.8–1.5 Mbps".to_string(),
            extra_args: Vec::new(),
        },
        QualityProfile {
            id: "p720:local".to_string(),
            label: "720p Local (HD Direct Play)".to_string(),
            resolution: ResolutionTier::P720,
            delivery: DeliveryTarget::Local,
            encoder: VideoEncoder::X26510Bit,
            encoder_preset: "slow".to_string(),
            rf_quality: 19.0,
            audio_mode: AudioMode::Passthrough,
            audio_bitrate_kbps: 256,
            container: "mkv".to_string(),
            decomb: false,
            web_optimize: false,
            subtitle_mode: SubtitleMode::All,
            estimated_bitrate_label: "~2.5–4.5 Mbps".to_string(),
            extra_args: Vec::new(),
        },
        QualityProfile {
            id: "p720:remote".to_string(),
            label: "720p Remote (Mobile / WAN)".to_string(),
            resolution: ResolutionTier::P720,
            delivery: DeliveryTarget::Remote,
            encoder: VideoEncoder::X264,
            encoder_preset: "medium".to_string(),
            rf_quality: 23.0,
            audio_mode: AudioMode::AacStereo,
            audio_bitrate_kbps: 160,
            container: "mp4".to_string(),
            decomb: false,
            web_optimize: true,
            subtitle_mode: SubtitleMode::ForeignScan,
            estimated_bitrate_label: "~1.5–2.8 Mbps".to_string(),
            extra_args: Vec::new(),
        },
        QualityProfile {
            id: "p1080:local".to_string(),
            label: "1080p Local (Blu-ray Quality)".to_string(),
            resolution: ResolutionTier::P1080,
            delivery: DeliveryTarget::Local,
            encoder: VideoEncoder::X26510Bit,
            encoder_preset: "slow".to_string(),
            rf_quality: 18.0,
            audio_mode: AudioMode::Passthrough,
            audio_bitrate_kbps: 320,
            container: "mkv".to_string(),
            decomb: false,
            web_optimize: false,
            subtitle_mode: SubtitleMode::All,
            estimated_bitrate_label: "~6–12 Mbps".to_string(),
            extra_args: Vec::new(),
        },
        QualityProfile {
            id: "p1080:remote".to_string(),
            label: "1080p Remote (Direct Stream)".to_string(),
            resolution: ResolutionTier::P1080,
            delivery: DeliveryTarget::Remote,
            encoder: VideoEncoder::X26510Bit,
            encoder_preset: "medium".to_string(),
            rf_quality: 23.0,
            audio_mode: AudioMode::Aac51,
            audio_bitrate_kbps: 256,
            container: "mp4".to_string(),
            decomb: false,
            web_optimize: true,
            subtitle_mode: SubtitleMode::ForeignScan,
            estimated_bitrate_label: "~3–5 Mbps".to_string(),
            extra_args: Vec::new(),
        },
        QualityProfile {
            id: "uhd4k:local".to_string(),
            label: "4K Local (UHD 10-bit HDR)".to_string(),
            resolution: ResolutionTier::Uhd4k,
            delivery: DeliveryTarget::Local,
            encoder: VideoEncoder::X26510Bit,
            encoder_preset: "slow".to_string(),
            rf_quality: 18.0,
            audio_mode: AudioMode::Passthrough,
            audio_bitrate_kbps: 384,
            container: "mkv".to_string(),
            decomb: false,
            web_optimize: false,
            subtitle_mode: SubtitleMode::All,
            estimated_bitrate_label: "~16–28 Mbps".to_string(),
            extra_args: Vec::new(),
        },
        QualityProfile {
            id: "uhd4k:remote".to_string(),
            label: "4K Remote (Efficient UHD)".to_string(),
            resolution: ResolutionTier::Uhd4k,
            delivery: DeliveryTarget::Remote,
            encoder: VideoEncoder::X26510Bit,
            encoder_preset: "medium".to_string(),
            rf_quality: 24.0,
            audio_mode: AudioMode::Aac51,
            audio_bitrate_kbps: 320,
            container: "mp4".to_string(),
            decomb: false,
            web_optimize: true,
            subtitle_mode: SubtitleMode::ForeignScan,
            estimated_bitrate_label: "~8–14 Mbps".to_string(),
            extra_args: Vec::new(),
        },
    ]
}

/// Builds the HandBrakeCLI quality, resolution, audio, container, and subtitle flags
/// Optional per-job audio/subtitle track indices (1-based) and preferred ISO-639-2 language
/// lists passed to `HandBrakeCLI` (`--audio`, `--audio-lang-list`, `--subtitle`, `--subtitle-lang-list`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackSelectionOptions {
    #[serde(default)]
    pub selected_audio_tracks: Option<Vec<u32>>,
    #[serde(default)]
    pub selected_subtitle_tracks: Option<Vec<u32>>,
    #[serde(default)]
    pub audio_languages: Vec<String>,
    #[serde(default)]
    pub subtitle_languages: Vec<String>,
}

fn clean_lang_codes(langs: &[String]) -> Vec<String> {
    langs
        .iter()
        .map(|s| {
            s.trim()
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|s| (2..=3).contains(&s.len()))
        .collect()
}

/// Builds the full HandBrakeCLI flag vector (excluding `-i` and `-o`) with optional explicit
/// track selection and preferred ISO-639-2 audio/subtitle language filters.
pub fn build_quality_flags_with_tracks(
    profile: &QualityProfile,
    tracks: &TrackSelectionOptions,
) -> Vec<String> {
    let (max_w, max_h) = profile.resolution.max_dimensions();
    let format_flag = if profile.container.eq_ignore_ascii_case("mp4") {
        "av_mp4"
    } else {
        "av_mkv"
    };

    let mut args = vec![
        "--format".to_string(),
        format_flag.to_string(),
        "--encoder".to_string(),
        profile.encoder.cli_name().to_string(),
        "--quality".to_string(),
        format!("{:.1}", profile.rf_quality),
        "--encoder-preset".to_string(),
        profile.encoder_preset.clone(),
        "--maxWidth".to_string(),
        max_w.to_string(),
        "--maxHeight".to_string(),
        max_h.to_string(),
        "--pfr".to_string(),
        "--markers".to_string(),
    ];

    if profile.decomb {
        args.push("--comb-detect".to_string());
        args.push("--decomb".to_string());
    }

    if profile.web_optimize && format_flag == "av_mp4" {
        args.push("--optimize".to_string());
    }

    // Audio track selection / language preference
    let explicit_audio: Vec<u32> = tracks
        .selected_audio_tracks
        .as_ref()
        .map(|list| list.iter().copied().filter(|&n| n > 0).collect())
        .unwrap_or_default();
    let audio_langs = clean_lang_codes(&tracks.audio_languages);

    if !explicit_audio.is_empty() {
        args.push("--audio".to_string());
        args.push(
            explicit_audio
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
    } else if !audio_langs.is_empty() {
        args.push("--audio-lang-list".to_string());
        args.push(audio_langs.join(","));
        if profile.audio_mode == AudioMode::Passthrough {
            args.push("--all-audio".to_string());
        } else {
            args.push("--first-audio".to_string());
        }
    } else if profile.audio_mode == AudioMode::Passthrough {
        args.push("--all-audio".to_string());
    }

    match profile.audio_mode {
        AudioMode::Passthrough => {
            args.push("--aencoder".to_string());
            args.push("copy:truehd,copy:dtshd,copy:dts,copy:eac3,copy:ac3,av_aac".to_string());
            args.push("--audio-fallback".to_string());
            args.push("av_aac".to_string());
            args.push("--ab".to_string());
            args.push(profile.audio_bitrate_kbps.to_string());
        }
        AudioMode::Aac51 => {
            args.push("--aencoder".to_string());
            args.push("av_aac".to_string());
            args.push("--mixdown".to_string());
            args.push("5point1".to_string());
            args.push("--ab".to_string());
            args.push(profile.audio_bitrate_kbps.to_string());
        }
        AudioMode::AacStereo => {
            args.push("--aencoder".to_string());
            args.push("av_aac".to_string());
            args.push("--mixdown".to_string());
            args.push("stereo".to_string());
            args.push("--ab".to_string());
            args.push(profile.audio_bitrate_kbps.to_string());
        }
    }

    // Subtitle track selection / language preference
    let sub_langs = clean_lang_codes(&tracks.subtitle_languages);
    if let Some(ref explicit_subs) = tracks.selected_subtitle_tracks {
        let valid: Vec<u32> = explicit_subs.iter().copied().filter(|&n| n > 0).collect();
        if !valid.is_empty() {
            args.push("--subtitle".to_string());
            args.push(
                valid
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
    } else {
        if !sub_langs.is_empty() && profile.subtitle_mode != SubtitleMode::None {
            args.push("--subtitle-lang-list".to_string());
            args.push(sub_langs.join(","));
        }
        match profile.subtitle_mode {
            SubtitleMode::All => {
                args.push("--all-subtitles".to_string());
            }
            SubtitleMode::ForeignScan => {
                args.push("--subtitle".to_string());
                args.push("scan".to_string());
                args.push("--subtitle-forced".to_string());
            }
            SubtitleMode::None => {}
        }
    }

    args.extend(profile.extra_args.iter().cloned());
    args
}

/// for a given `QualityProfile` (excluding `-i` and `-o`).
pub fn build_quality_flags(profile: &QualityProfile) -> Vec<String> {
    build_quality_flags_with_tracks(profile, &TrackSelectionOptions::default())
}

/// Builds a complete `HandBrakeCLI --json` invocation from a `QualityProfile` and track options.
pub fn build_quality_profile_encode_args_with_tracks(
    input_path: &Path,
    output_path: &Path,
    profile: &QualityProfile,
    tracks: &TrackSelectionOptions,
) -> Vec<String> {
    let mut args = vec![
        "--json".to_string(),
        "-i".to_string(),
        input_path.to_string_lossy().into_owned(),
        "-o".to_string(),
        output_path.to_string_lossy().into_owned(),
    ];
    args.extend(build_quality_flags_with_tracks(profile, tracks));
    args
}

/// Builds a complete `HandBrakeCLI --json` invocation from a `QualityProfile`.
pub fn build_quality_profile_encode_args(
    input_path: &Path,
    output_path: &Path,
    profile: &QualityProfile,
) -> Vec<String> {
    build_quality_profile_encode_args_with_tracks(
        input_path,
        output_path,
        profile,
        &TrackSelectionOptions::default(),
    )
}

/// High-level encoding state reported by HandBrakeCLI `--json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HandBrakeState {
    Scanning,
    Working,
    Muxing,
    Paused,
    WorkDone,
    Unknown,
}

impl HandBrakeState {
    /// Converts HandBrake's uppercase state token into a typed enum variant.
    pub fn from_raw(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "SCANNING" => HandBrakeState::Scanning,
            "WORKING" => HandBrakeState::Working,
            "MUXING" => HandBrakeState::Muxing,
            "PAUSED" => HandBrakeState::Paused,
            "WORKDONE" => HandBrakeState::WorkDone,
            _ => HandBrakeState::Unknown,
        }
    }
}

/// Raw `Working` sub-object emitted inside HandBrakeCLI's `Progress: { ... }` JSON block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawWorkingBlock {
    #[serde(rename = "Progress", default)]
    pub progress: f64,
    #[serde(rename = "Rate", default)]
    pub rate: f64,
    #[serde(rename = "RateAvg", default)]
    pub rate_avg: f64,
    #[serde(rename = "ETASeconds", default)]
    pub eta_seconds: i64,
    #[serde(rename = "Hours", default)]
    pub hours: u32,
    #[serde(rename = "Minutes", default)]
    pub minutes: u32,
    #[serde(rename = "Seconds", default)]
    pub seconds: u32,
    #[serde(rename = "Pass", default)]
    pub pass: u32,
    #[serde(rename = "PassCount", default)]
    pub pass_count: u32,
    #[serde(rename = "PassID", default)]
    pub pass_id: i32,
}

/// Raw `Muxing` sub-object emitted while HandBrake finalizes the output container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMuxingBlock {
    #[serde(rename = "Progress", default)]
    pub progress: f64,
}

/// Raw `WorkDone` sub-object emitted once an encode job terminates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawWorkDoneBlock {
    #[serde(rename = "Error", default)]
    pub error: i32,
}

/// Raw deserialization target matching HandBrakeCLI's `Progress: { ... }` JSON schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawProgressPayload {
    #[serde(rename = "State")]
    pub state: String,
    #[serde(rename = "Working")]
    pub working: Option<RawWorkingBlock>,
    #[serde(rename = "Muxing")]
    pub muxing: Option<RawMuxingBlock>,
    #[serde(rename = "WorkDone")]
    pub work_done: Option<RawWorkDoneBlock>,
}

/// Normalized encoding progress payload forwarded to the frontend via Tauri IPC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeProgress {
    pub state: HandBrakeState,
    pub percent: f64,
    pub fps: Option<f64>,
    pub avg_fps: Option<f64>,
    pub eta_seconds: Option<u64>,
    pub pass: Option<u32>,
    pub pass_count: Option<u32>,
}

/// Encoding preset discovered via `HandBrakeCLI --preset-list` or imported JSON file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandBrakePreset {
    pub name: String,
    pub category: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub is_custom: bool,
}

/// Configuration for invoking `HandBrakeCLI` using a named HandBrake preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeOptions {
    pub preset_name: String,
    pub custom_preset_file: Option<String>,
    pub container: String,
    pub extra_args: Vec<String>,
}

/// Stateful line-by-line parser for HandBrakeCLI's multiline `Progress: { ... }` stdout stream.
#[derive(Debug, Default)]
pub struct ProgressStreamParser {
    buffer: String,
    capturing: bool,
    brace_depth: i32,
    saw_open_brace: bool,
    in_string: bool,
    escaped: bool,
}

impl ProgressStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.capturing = false;
        self.brace_depth = 0;
        self.saw_open_brace = false;
        self.in_string = false;
        self.escaped = false;
    }

    /// Feeds one line of `HandBrakeCLI --json` stdout into the brace-depth accumulator.
    /// Returns `Ok(Some(EncodeProgress))` once a complete `Progress: { ... }` block closes.
    pub fn push_line(&mut self, line: &str) -> Result<Option<EncodeProgress>, HandBrakeError> {
        let fragment = if !self.capturing {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("Progress:") else {
                return Ok(None);
            };
            self.reset();
            self.capturing = true;
            rest
        } else {
            line
        };

        for ch in fragment.chars() {
            if self.saw_open_brace || ch == '{' {
                self.buffer.push(ch);
            }
            if self.escaped {
                self.escaped = false;
                continue;
            }
            if self.in_string {
                match ch {
                    '\\' => self.escaped = true,
                    '"' => self.in_string = false,
                    _ => {}
                }
            } else {
                match ch {
                    '"' => self.in_string = true,
                    '{' => {
                        self.saw_open_brace = true;
                        self.brace_depth += 1;
                    }
                    '}' if self.saw_open_brace => {
                        self.brace_depth -= 1;
                        if self.brace_depth == 0 {
                            let completed = std::mem::take(&mut self.buffer);
                            self.reset();
                            return parse_progress_payload(&completed).map(Some);
                        }
                    }
                    _ => {}
                }
            }
        }

        if self.capturing {
            self.buffer.push('\n');
        }
        Ok(None)
    }
}

/// Parses a standalone HandBrakeCLI `Progress` JSON object into a normalized `EncodeProgress`.
pub fn parse_progress_payload(json_block: &str) -> Result<EncodeProgress, HandBrakeError> {
    let raw: RawProgressPayload = serde_json::from_str(json_block)
        .map_err(|err| HandBrakeError::InvalidJson(err.to_string()))?;

    let state = HandBrakeState::from_raw(&raw.state);
    match state {
        HandBrakeState::Working => {
            let w = raw.working.unwrap_or(RawWorkingBlock {
                progress: 0.0,
                rate: 0.0,
                rate_avg: 0.0,
                eta_seconds: -1,
                hours: 0,
                minutes: 0,
                seconds: 0,
                pass: 1,
                pass_count: 1,
                pass_id: 0,
            });
            Ok(EncodeProgress {
                state,
                percent: (w.progress * 100.0).clamp(0.0, 100.0),
                fps: (w.rate > 0.0).then_some(w.rate),
                avg_fps: (w.rate_avg > 0.0).then_some(w.rate_avg),
                eta_seconds: (w.eta_seconds >= 0).then_some(w.eta_seconds as u64),
                pass: (w.pass > 0).then_some(w.pass),
                pass_count: (w.pass_count > 0).then_some(w.pass_count),
            })
        }
        HandBrakeState::Muxing => {
            let progress = raw.muxing.map(|m| m.progress).unwrap_or(0.0);
            Ok(EncodeProgress {
                state,
                percent: (progress * 100.0).clamp(0.0, 100.0),
                fps: None,
                avg_fps: None,
                eta_seconds: Some(0),
                pass: None,
                pass_count: None,
            })
        }
        HandBrakeState::WorkDone => {
            let err_code = raw.work_done.map(|w| w.error).unwrap_or(0);
            Ok(EncodeProgress {
                state,
                percent: if err_code == 0 { 100.0 } else { 0.0 },
                fps: None,
                avg_fps: None,
                eta_seconds: Some(0),
                pass: None,
                pass_count: None,
            })
        }
        _ => Ok(EncodeProgress {
            state,
            percent: 0.0,
            fps: None,
            avg_fps: None,
            eta_seconds: None,
            pass: None,
            pass_count: None,
        }),
    }
}

/// Parses `HandBrakeCLI --preset-list` console output into structured `HandBrakePreset` items.
pub fn parse_preset_list(output: &str) -> Result<Vec<HandBrakePreset>, HandBrakeError> {
    let mut presets = Vec::new();
    let mut current_category = "General".to_string();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('[')
            || trimmed.starts_with("HandBrake ")
            || trimmed.starts_with("Preset ")
        {
            continue;
        }

        // Top-level category lines look like `General/` or `Matroska/` (0–2 leading spaces)
        let indent = line.len() - line.trim_start().len();
        if indent <= 2 && trimmed.ends_with('/') {
            current_category = trimmed.trim_end_matches('/').to_string();
            continue;
        }

        // Preset name lines are indented (typically 4 spaces) and do not start with `+`
        if indent >= 4 && !trimmed.starts_with('+') && !trimmed.starts_with('-') {
            let clean_name = trimmed.trim_start_matches("+ ").trim().to_string();
            if !clean_name.is_empty() {
                presets.push(HandBrakePreset {
                    is_default: clean_name == "Fast 1080p30",
                    name: clean_name,
                    category: current_category.clone(),
                    description: None,
                    is_custom: false,
                });
            }
        }
    }

    Ok(presets)
}

/// Builds CLI arguments for a named-preset `HandBrakeCLI --json` encode run.
pub fn build_encode_args(
    input_path: &Path,
    output_path: &Path,
    options: &EncodeOptions,
) -> Vec<String> {
    let mut args = vec![
        "--json".to_string(),
        "-i".to_string(),
        input_path.to_string_lossy().into_owned(),
        "-o".to_string(),
        output_path.to_string_lossy().into_owned(),
    ];

    if let Some(custom_file) = options
        .custom_preset_file
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        args.push("--preset-import-file".to_string());
        args.push(custom_file.to_string());
    }

    args.push("-Z".to_string());
    args.push(options.preset_name.clone());
    args.extend(options.extra_args.iter().cloned());
    args
}

#[cfg(windows)]
fn configure_platform_command(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_platform_command(_cmd: &mut Command) {}

/// Lists built-in HandBrake presets via `HandBrakeCLI --preset-list`.
pub async fn list_presets(resolved: &ResolvedCommand) -> Result<Vec<HandBrakePreset>, HandBrakeError> {
    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.prefix_args)
        .arg("--preset-list")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|err| HandBrakeError::Io(format!("Failed to launch HandBrakeCLI: {err}")))?;

    let output = timeout(PRESET_LIST_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| HandBrakeError::Timeout("Listing HandBrake presets timed out.".to_string()))?
        .map_err(|err| HandBrakeError::Io(err.to_string()))?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_preset_list(&combined)
}

/// Parses the `-e, --encoder` section from `HandBrakeCLI --help` output into a set of
/// lowercase encoder identifiers (e.g. `x265_10bit`, `nvenc_h265`, `vaapi_h265`, `qsv_h265`, `vt_h265`).
pub fn parse_handbrake_help_encoders(help_output: &str) -> HashSet<String> {
    let mut found = HashSet::new();
    let mut in_encoder_section = false;

    for raw_line in help_output.lines() {
        let trimmed = raw_line.trim();

        if !in_encoder_section {
            if trimmed.contains("--encoder")
                && (trimmed.contains("Select video encoder")
                    || trimmed.contains("video encoder:")
                    || trimmed.starts_with("-e,"))
            {
                in_encoder_section = true;
                // Handle any inline token after `:` on the same line if present
                if let Some((_, after)) = trimmed.split_once("Select video encoder:") {
                    collect_encoder_tokens(after, &mut found);
                }
            }
            continue;
        }

        // Stop once we reach the next CLI flag or a new help section header
        if trimmed.is_empty() {
            if !found.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('-') || (trimmed.ends_with(':') && !trimmed.contains(' ')) {
            break;
        }

        // Only consume indented encoder listing lines
        let indent = raw_line.len() - raw_line.trim_start().len();
        if indent < 4 {
            break;
        }

        collect_encoder_tokens(trimmed, &mut found);
    }

    found
}

fn collect_encoder_tokens(segment: &str, out: &mut HashSet<String>) {
    for raw_token in segment.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let token = raw_token.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ':'));
        if token.is_empty() || token.eq_ignore_ascii_case("default") {
            continue;
        }
        let valid = token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && token.chars().any(|c| c.is_ascii_alphabetic());
        if valid {
            out.insert(token.to_ascii_lowercase());
        }
    }
}

/// Hardware probe snapshot for the current host machine.
#[derive(Debug, Clone, Copy, Default)]
struct HostGpuSnapshot {
    nvidia_present: bool,
    dri_render_present: bool,
    intel_gpu_present: bool,
    vaapi_hevc_verified: Option<bool>,
}

async fn probe_host_gpu_snapshot() -> HostGpuSnapshot {
    #[cfg(target_os = "linux")]
    {
        let nvidia_present = Path::new("/dev/nvidiactl").exists()
            || Path::new("/dev/nvidia0").exists()
            || detect_linux_pci_vendor("0x10de");

        let dri_render_present = std::fs::read_dir("/dev/dri")
            .ok()
            .map(|entries| {
                entries
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().starts_with("renderD"))
            })
            .unwrap_or(false);

        let intel_gpu_present = detect_linux_drm_vendor("0x8086")
            || (dri_render_present && detect_linux_pci_vendor("0x8086"));

        let vaapi_hevc_verified = if dri_render_present {
            probe_linux_vainfo_hevc_encode().await
        } else {
            Some(false)
        };

        HostGpuSnapshot {
            nvidia_present,
            dri_render_present,
            intel_gpu_present,
            vaapi_hevc_verified,
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        HostGpuSnapshot::default()
    }
}

#[cfg(target_os = "linux")]
fn detect_linux_pci_vendor(target_vendor: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return false;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(class_raw) = std::fs::read_to_string(dir.join("class")) else {
            continue;
        };
        // PCI class 0x03xxxx = Display controller (VGA / 3D / Display)
        if !class_raw.trim().starts_with("0x03") {
            continue;
        }
        if let Ok(vendor_raw) = std::fs::read_to_string(dir.join("vendor")) {
            if vendor_raw.trim().eq_ignore_ascii_case(target_vendor) {
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn detect_linux_drm_vendor(target_vendor: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("renderD") {
            continue;
        }
        if let Ok(vendor_raw) = std::fs::read_to_string(entry.path().join("device/vendor")) {
            if vendor_raw.trim().eq_ignore_ascii_case(target_vendor) {
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "linux")]
async fn probe_linux_vainfo_hevc_encode() -> Option<bool> {
    let resolved = detect::locate_optional_tool("vainfo")?;
    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.prefix_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().ok()?;
    let out = timeout(VAINFO_PROBE_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let has_hevc_enc = text.lines().any(|line| {
        (line.contains("VAProfileHEVCMain") || line.contains("VAProfileHEVCMain10"))
            && line.contains("VAEntrypointEncSlice")
    });
    Some(has_hevc_enc)
}

/// Evaluates `EncoderCapabilities` given a parsed set of `HandBrakeCLI --help` encoders,
/// host OS identifier, and hardware snapshot. Pure logic separated for deterministic unit tests.
fn evaluate_encoder_capabilities(
    handbrake_available: bool,
    handbrake_version: Option<String>,
    cli_encoders: &HashSet<String>,
    os: &str,
    gpu: HostGpuSnapshot,
) -> EncoderCapabilities {
    let mut encoders = Vec::with_capacity(VideoEncoder::ALL.len());

    for enc in VideoEncoder::ALL {
        let cli_name = enc.cli_name().to_string();
        let is_hardware = enc.is_hardware();
        let cli_listed = handbrake_available && cli_encoders.contains(&cli_name);

        let (hardware_detected, supported, reason) = if !handbrake_available {
            (
                false,
                false,
                "HandBrakeCLI is not installed or unavailable.".to_string(),
            )
        } else if !is_hardware {
            if cli_listed {
                (
                    true,
                    true,
                    "Software CPU encoder supported by HandBrakeCLI.".to_string(),
                )
            } else {
                (
                    false,
                    false,
                    format!("{cli_name} is not enabled in this HandBrakeCLI build."),
                )
            }
        } else {
            match enc {
                VideoEncoder::NvencH265 => {
                    if os == "macos" {
                        (false, false, "NVIDIA NVENC is not supported on macOS.".to_string())
                    } else if !cli_listed {
                        (
                            gpu.nvidia_present,
                            false,
                            "Not available in HandBrakeCLI (no compatible NVIDIA NVENC runtime detected).".to_string(),
                        )
                    } else if os == "linux" && !gpu.nvidia_present {
                        (
                            false,
                            false,
                            "Listed by HandBrakeCLI, but no NVIDIA GPU device was detected.".to_string(),
                        )
                    } else {
                        (
                            true,
                            true,
                            "NVIDIA NVENC H.265 hardware encoding ready.".to_string(),
                        )
                    }
                }
                VideoEncoder::VaapiH265 => {
                    if os != "linux" {
                        (false, false, "VAAPI hardware encoding is Linux-only.".to_string())
                    } else if !cli_listed {
                        (
                            gpu.dri_render_present,
                            false,
                            "vaapi_h265 is not enabled in this HandBrakeCLI build.".to_string(),
                        )
                    } else if !gpu.dri_render_present {
                        (
                            false,
                            false,
                            "No /dev/dri/renderD* GPU render node detected.".to_string(),
                        )
                    } else if gpu.vaapi_hevc_verified == Some(false) {
                        (
                            false,
                            false,
                            "DRI device found, but vainfo reports no HEVC encode entrypoint.".to_string(),
                        )
                    } else {
                        (
                            true,
                            true,
                            "Linux VAAPI H.265 hardware encoding ready.".to_string(),
                        )
                    }
                }
                VideoEncoder::QsvH265 => {
                    if os == "macos" {
                        (
                            false,
                            false,
                            "Intel QuickSync is unavailable on macOS (use Apple VideoToolbox).".to_string(),
                        )
                    } else if !cli_listed {
                        (
                            gpu.intel_gpu_present,
                            false,
                            "Not available in HandBrakeCLI (Intel QSV / oneVPL runtime not active).".to_string(),
                        )
                    } else if os == "linux" && !gpu.intel_gpu_present {
                        (
                            false,
                            false,
                            "Listed by HandBrakeCLI, but no Intel GPU render device was detected.".to_string(),
                        )
                    } else {
                        (
                            true,
                            true,
                            "Intel QuickSync H.265 hardware encoding ready.".to_string(),
                        )
                    }
                }
                VideoEncoder::VtH265 => {
                    if os != "macos" {
                        (
                            false,
                            false,
                            "Apple VideoToolbox is macOS-only.".to_string(),
                        )
                    } else if !cli_listed {
                        (
                            false,
                            false,
                            "vt_h265 is not enabled in this HandBrakeCLI build.".to_string(),
                        )
                    } else {
                        (
                            true,
                            true,
                            "Apple VideoToolbox H.265 hardware encoding ready.".to_string(),
                        )
                    }
                }
                _ => (cli_listed, cli_listed, String::new()),
            }
        };

        encoders.push(EncoderProbeInfo {
            encoder: enc,
            cli_name,
            is_hardware,
            supported,
            cli_listed,
            hardware_detected,
            reason,
        });
    }

    let gpu_priority = if os == "macos" {
        [
            VideoEncoder::VtH265,
            VideoEncoder::NvencH265,
            VideoEncoder::QsvH265,
            VideoEncoder::VaapiH265,
        ]
    } else {
        [
            VideoEncoder::NvencH265,
            VideoEncoder::QsvH265,
            VideoEncoder::VaapiH265,
            VideoEncoder::VtH265,
        ]
    };

    let recommended_gpu_encoder = gpu_priority.into_iter().find(|candidate| {
        encoders
            .iter()
            .any(|info| info.encoder == *candidate && info.supported)
    });

    let supported_gpu_names: Vec<&'static str> = encoders
        .iter()
        .filter(|e| e.is_hardware && e.supported)
        .map(|e| e.encoder.display_name())
        .collect();

    let summary = if !handbrake_available {
        "Install HandBrakeCLI to probe GPU & CPU encoders.".to_string()
    } else if supported_gpu_names.is_empty() {
        "CPU encoding only (no supported GPU H.265 encoder detected).".to_string()
    } else {
        format!("GPU ready: {}", supported_gpu_names.join(", "))
    };

    EncoderCapabilities {
        handbrake_available,
        handbrake_version,
        encoders,
        recommended_gpu_encoder,
        summary,
    }
}

/// Probes `HandBrakeCLI --help` and local GPU hardware to determine which software
/// and hardware encoders (`nvenc_h265`, `vaapi_h265`, `qsv_h265`, `vt_h265`) are usable.
pub async fn probe_encoders(
    resolved: Option<&ResolvedCommand>,
    version: Option<String>,
) -> EncoderCapabilities {
    let Some(resolved_cmd) = resolved else {
        return evaluate_encoder_capabilities(
            false,
            version,
            &HashSet::new(),
            std::env::consts::OS,
            HostGpuSnapshot::default(),
        );
    };

    let help_future = async {
        let mut cmd = Command::new(&resolved_cmd.program);
        cmd.args(&resolved_cmd.prefix_args)
            .arg("--help")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        configure_platform_command(&mut cmd);

        let child = cmd.spawn().ok()?;
        let output = timeout(ENCODER_PROBE_TIMEOUT, child.wait_with_output())
            .await
            .ok()?
            .ok()?;

        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Some(parse_handbrake_help_encoders(&combined))
    };

    let (cli_opt, gpu_snapshot) = tokio::join!(help_future, probe_host_gpu_snapshot());
    let cli_encoders = cli_opt.unwrap_or_default();

    evaluate_encoder_capabilities(
        true,
        version,
        &cli_encoders,
        std::env::consts::OS,
        gpu_snapshot,
    )
}

/// Generates a 10-second WebView-playable MP4 sample clip using the user's selected
/// `QualityProfile` (resolution, RF quality, decomb) so they can inspect quality & crop
/// directly inside the right-hand Preview drawer before queuing a full encode.
pub async fn generate_sample_clip(
    resolved: &ResolvedCommand,
    input_path: &Path,
    temp_dir: &Path,
    profile: &QualityProfile,
    start_seconds: u64,
    duration_seconds: u64,
) -> Result<PathBuf, HandBrakeError> {
    let preview_dir = temp_dir.join("previews");
    tokio::fs::create_dir_all(&preview_dir)
        .await
        .map_err(|err| HandBrakeError::Io(format!("Failed creating preview dir: {err}")))?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let output_path = preview_dir.join(format!("sample_{stamp}.mp4"));

    // Use H.264 + AAC inside fast-start MP4 so WebKitGTK / WebView2 / WKWebView plays it reliably
    // while honoring the user's chosen resolution cap, RF quality, and decomb settings.
    let (max_w, max_h) = profile.resolution.max_dimensions();
    let clip_len = duration_seconds.clamp(3, 30);

    let mut args = vec![
        "--json".to_string(),
        "-i".to_string(),
        input_path.to_string_lossy().into_owned(),
        "-o".to_string(),
        output_path.to_string_lossy().into_owned(),
        "--start-at".to_string(),
        format!("seconds:{start_seconds}"),
        "--stop-at".to_string(),
        format!("seconds:{clip_len}"),
        "--format".to_string(),
        "av_mp4".to_string(),
        "--optimize".to_string(),
        "--encoder".to_string(),
        "x264".to_string(),
        "--encoder-preset".to_string(),
        "veryfast".to_string(),
        "--quality".to_string(),
        format!("{:.1}", profile.rf_quality),
        "--maxWidth".to_string(),
        max_w.to_string(),
        "--maxHeight".to_string(),
        max_h.to_string(),
        "--aencoder".to_string(),
        "av_aac".to_string(),
        "--mixdown".to_string(),
        "stereo".to_string(),
        "--ab".to_string(),
        "160".to_string(),
    ];
    if profile.decomb {
        args.push("--comb-detect".to_string());
        args.push("--decomb".to_string());
    }

    let mut mount_dirs: Vec<&Path> = vec![preview_dir.as_path()];
    if let Some(in_parent) = input_path.parent() {
        mount_dirs.push(in_parent);
    }
    let mounted_cmd = resolved.with_flatpak_paths(&mount_dirs);

    let mut cmd = Command::new(&mounted_cmd.program);
    cmd.args(&mounted_cmd.prefix_args)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|err| HandBrakeError::Io(format!("Failed starting sample encode: {err}")))?;

    let output = timeout(SAMPLE_CLIP_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| HandBrakeError::Timeout("Sample clip generation timed out.".to_string()))?
        .map_err(|err| HandBrakeError::Io(err.to_string()))?;

    if output.status.success() && tokio::fs::try_exists(&output_path).await.unwrap_or(false) {
        Ok(output_path)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let snippet = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("HandBrakeCLI exited with an error");
        Err(HandBrakeError::Io(format!(
            "Sample clip failed (exit {:?}): {}",
            output.status.code(),
            snippet.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommended_matrix_covers_all_eight_resolution_delivery_slots() {
        let profiles = recommended_quality_profiles();
        assert_eq!(profiles.len(), 8);

        let uhd_local = profiles
            .iter()
            .find(|p| p.id == "uhd4k:local")
            .expect("4K local profile");
        let flags = build_quality_flags(uhd_local);
        assert!(flags.contains(&"x265_10bit".to_string()));
        assert!(flags.contains(&"3840".to_string()));
        assert!(flags.contains(&"2160".to_string()));
        assert!(flags.contains(&"--all-audio".to_string()));

        let dvd_remote = profiles
            .iter()
            .find(|p| p.id == "p480:remote")
            .expect("480p remote profile");
        let remote_flags = build_quality_flags(dvd_remote);
        assert!(remote_flags.contains(&"av_mp4".to_string()));
        assert!(remote_flags.contains(&"--decomb".to_string()));
        assert!(remote_flags.contains(&"--optimize".to_string()));
        assert!(remote_flags.contains(&"stereo".to_string()));
    }

    #[test]
    fn streams_multiline_handbrake_progress_json() {
        let lines = [
            "Version: {",
            "    \"Name\": \"HandBrake\"",
            "}",
            "Progress: {",
            "    \"State\": \"WORKING\",",
            "    \"Working\": {",
            "        \"Progress\": 0.425,",
            "        \"Rate\": 61.5,",
            "        \"RateAvg\": 58.2,",
            "        \"ETASeconds\": 145,",
            "        \"Pass\": 1,",
            "        \"PassCount\": 1",
            "    }",
            "}",
            "Progress: {\"State\":\"MUXING\",\"Muxing\":{\"Progress\":0.8}}",
        ];

        let mut parser = ProgressStreamParser::new();
        let mut updates = Vec::new();
        for line in lines {
            if let Some(update) = parser.push_line(line).expect("parse line") {
                updates.push(update);
            }
        }

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].state, HandBrakeState::Working);
        assert!((updates[0].percent - 42.5).abs() < 0.01);
        assert_eq!(updates[0].fps, Some(61.5));
        assert_eq!(updates[0].eta_seconds, Some(145));

        assert_eq!(updates[1].state, HandBrakeState::Muxing);
        assert!((updates[1].percent - 80.0).abs() < 0.01);
    }

    #[test]
    fn parses_handbrake_preset_list() {
        let sample = "\
General/
    Very Fast 1080p30
    Fast 1080p30
Matroska/
    H.265 MKV 1080p30
";
        let presets = parse_preset_list(sample).expect("parse presets");
        assert_eq!(presets.len(), 3);
        assert_eq!(presets[1].name, "Fast 1080p30");
        assert!(presets[1].is_default);
        assert_eq!(presets[2].category, "Matroska");
    }

    #[test]
    fn parses_handbrake_help_video_encoders_and_evaluates_gpu_support() {
        let sample_help = "\
Video Options -----------------------------------------------------------------

    -e, --encoder <string>  Select video encoder:
                                x264
                                x264_10bit
                                nvenc_h264
                                nvenc_h265
                                vaapi_h265
                                x265
                                x265_10bit
                                svt_av1_10bit
        --encoder-preset <string>
                            Adjust video encoding settings for a particular
";
        let parsed = parse_handbrake_help_encoders(sample_help);
        assert!(parsed.contains("x265_10bit"));
        assert!(parsed.contains("nvenc_h265"));
        assert!(parsed.contains("vaapi_h265"));
        assert!(!parsed.contains("vt_h265"));
        assert!(!parsed.contains("qsv_h265"));

        // Linux machine with AMD/Intel DRI render node (VAAPI HEVC ok), no NVIDIA GPU
        let caps = evaluate_encoder_capabilities(
            true,
            Some("1.11.2".to_string()),
            &parsed,
            "linux",
            HostGpuSnapshot {
                nvidia_present: false,
                dri_render_present: true,
                intel_gpu_present: false,
                vaapi_hevc_verified: Some(true),
            },
        );
        let find_enc = |e: VideoEncoder| {
            caps.encoders
                .iter()
                .find(|info| info.encoder == e)
                .expect("encoder entry")
        };
        assert!(find_enc(VideoEncoder::X26510Bit).supported);
        assert!(find_enc(VideoEncoder::VaapiH265).supported);
        assert!(!find_enc(VideoEncoder::NvencH265).supported);
        assert!(!find_enc(VideoEncoder::VtH265).supported);
        assert_eq!(caps.recommended_gpu_encoder, Some(VideoEncoder::VaapiH265));
    }
}
