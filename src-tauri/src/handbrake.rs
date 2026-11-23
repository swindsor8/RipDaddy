//! Data structures, Resolution × Delivery quality profiles (480p / 720p / 1080p / 4K × Local / Remote),
//! and parser stubs for `HandBrakeCLI --json` progress and preset listing.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Errors produced while parsing HandBrakeCLI output or invoking the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandBrakeError {
    InvalidJson(String),
    NotImplemented(&'static str),
    Io(String),
}

impl fmt::Display for HandBrakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandBrakeError::InvalidJson(msg) => {
                write!(f, "Invalid HandBrakeCLI JSON payload: {msg}")
            }
            HandBrakeError::NotImplemented(feature) => {
                write!(
                    f,
                    "HandBrakeCLI integration stub not implemented yet: {feature}"
                )
            }
            HandBrakeError::Io(msg) => write!(f, "HandBrakeCLI I/O error: {msg}"),
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
    /// Home LAN / direct play archival (higher RF quality, lossless/surround passthrough).
    Local,
    /// Remote WAN / mobile streaming (tighter RF, AAC audio, fast-start MP4).
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
}

/// Audio encoding / passthrough strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioMode {
    /// Pass through TrueHD/DTS-HD/DTS/EAC3/AC3 with AAC fallback (ideal for Local LAN).
    Passthrough,
    /// Transcode to 5.1 Surround AAC (good balance for Remote streaming).
    Aac51,
    /// Downmix to 2.0 Stereo AAC (lowest bandwidth for Remote/SD).
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
        // 480p (SD / DVD)
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
        // 720p (HD)
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
        // 1080p (Full HD / Blu-ray)
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
        // 4K (2160p UHD / HDR10)
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
/// for a given `QualityProfile` (excluding `-i` and `-o`).
pub fn build_quality_flags(profile: &QualityProfile) -> Vec<String> {
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

    match profile.audio_mode {
        AudioMode::Passthrough => {
            args.push("--all-audio".to_string());
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

    args.extend(profile.extra_args.iter().cloned());
    args
}

/// Builds a complete `HandBrakeCLI --json` invocation from a `QualityProfile`.
pub fn build_quality_profile_encode_args(
    input_path: &Path,
    output_path: &Path,
    profile: &QualityProfile,
) -> Vec<String> {
    let mut args = vec![
        "--json".to_string(),
        "-i".to_string(),
        input_path.to_string_lossy().into_owned(),
        "-o".to_string(),
        output_path.to_string_lossy().into_owned(),
    ];
    args.extend(build_quality_flags(profile));
    args
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

/// Stateful line-by-line accumulator stub for HandBrakeCLI's multiline `Progress: { ... }`
/// stdout stream.
#[derive(Debug, Default)]
pub struct ProgressStreamParser {
    buffer: String,
    capturing: bool,
}

impl ProgressStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one line of `HandBrakeCLI --json` stdout into the accumulator.
    pub fn push_line(&mut self, _line: &str) -> Result<Option<EncodeProgress>, HandBrakeError> {
        let _ = (&self.buffer, self.capturing);
        Err(HandBrakeError::NotImplemented(
            "HandBrakeCLI streaming JSON progress parser",
        ))
    }
}

/// Parser stub for a standalone HandBrakeCLI `Progress` JSON object.
pub fn parse_progress_payload(_json_block: &str) -> Result<EncodeProgress, HandBrakeError> {
    Err(HandBrakeError::NotImplemented("parse_progress_payload"))
}

/// Parser stub for `HandBrakeCLI --preset-list` console output.
pub fn parse_preset_list(_output: &str) -> Result<Vec<HandBrakePreset>, HandBrakeError> {
    Err(HandBrakeError::NotImplemented("parse_preset_list"))
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

/// Async stub for enumerating built-in presets from `HandBrakeCLI --preset-list`.
pub async fn list_presets(_binary_path: &Path) -> Result<Vec<HandBrakePreset>, HandBrakeError> {
    Err(HandBrakeError::NotImplemented("list_presets"))
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
        assert!(!flags.contains(&"--optimize".to_string()));

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
    fn deserializes_handbrake_raw_progress_schema() {
        let sample = r#"{
            "State": "WORKING",
            "Working": {
                "Progress": 0.425,
                "Rate": 61.5,
                "RateAvg": 58.2,
                "ETASeconds": 145,
                "Hours": 0,
                "Minutes": 2,
                "Seconds": 25,
                "Pass": 1,
                "PassCount": 1,
                "PassID": 0
            }
        }"#;

        let parsed: RawProgressPayload = serde_json::from_str(sample).expect("valid raw JSON");
        assert_eq!(HandBrakeState::from_raw(&parsed.state), HandBrakeState::Working);
        let working = parsed.working.expect("working block");
        assert!((working.progress - 0.425).abs() < 1e-6);
        assert_eq!(working.eta_seconds, 145);
    }
}
