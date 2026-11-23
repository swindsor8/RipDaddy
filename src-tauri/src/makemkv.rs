//! Data structures and parser stubs for MakeMKV CLI robot mode (`makemkvcon -r`).
//!
//! MakeMKV outputs strictly line-oriented, comma-delimited records with quoted strings:
//! - `DRV:index,visible,enabled,flags,drive_name,disc_name[,device_path]`
//! - `TINFO:title_id,attr_id,code,value`
//! - `SINFO:title_id,stream_id,attr_id,code,value`
//! - `PRGV:current,total,max`

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Subset of MakeMKV `AP_ItemAttributeId` numeric attribute codes used by `TINFO` and `SINFO`.
pub mod attr_id {
    pub const STREAM_TYPE: u32 = 1;
    pub const NAME: u32 = 2;
    pub const LANG_CODE: u32 = 3;
    pub const LANG_NAME: u32 = 4;
    pub const CODEC_ID: u32 = 5;
    pub const CODEC_SHORT: u32 = 6;
    pub const CODEC_LONG: u32 = 7;
    pub const CHAPTER_COUNT: u32 = 8;
    pub const DURATION: u32 = 9;
    pub const DISK_SIZE: u32 = 10;
    pub const DISK_SIZE_BYTES: u32 = 11;
    pub const SOURCE_FILE_NAME: u32 = 16;
    pub const VIDEO_SIZE: u32 = 19;
    pub const VIDEO_FRAME_RATE: u32 = 21;
    pub const AUDIO_CHANNELS: u32 = 14;
    pub const OUTPUT_FILE_NAME: u32 = 27;
}

/// Errors produced while parsing `makemkvcon -r` output or invoking the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MakeMkvError {
    InvalidFormat(String),
    NotImplemented(&'static str),
    Io(String),
}

impl fmt::Display for MakeMkvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MakeMkvError::InvalidFormat(msg) => write!(f, "Invalid MakeMKV robot record: {msg}"),
            MakeMkvError::NotImplemented(feature) => {
                write!(f, "MakeMKV integration stub not implemented yet: {feature}")
            }
            MakeMkvError::Io(msg) => write!(f, "MakeMKV I/O error: {msg}"),
        }
    }
}

impl std::error::Error for MakeMkvError {}

/// Optical drive reported by a `DRV:` robot-mode line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpticalDrive {
    pub index: u32,
    pub visible: u32,
    pub enabled: u32,
    pub flags: u32,
    pub drive_name: String,
    pub disc_name: String,
    pub device_path: Option<String>,
}

/// Normalized track category derived from `SINFO` stream attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackType {
    Video,
    Audio,
    Subtitle,
    Unknown,
}

/// Aggregated stream metadata assembled from `SINFO` records belonging to a title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub stream_id: u32,
    pub track_type: TrackType,
    pub codec_id: Option<String>,
    pub codec_short: Option<String>,
    pub codec_long: Option<String>,
    pub language_code: Option<String>,
    pub language_name: Option<String>,
    pub channels: Option<u8>,
    pub channel_layout: Option<String>,
    pub video_size: Option<String>,
    pub frame_rate: Option<String>,
    pub name: Option<String>,
}

/// Aggregated title metadata assembled from `TINFO` and child `SINFO` records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleInfo {
    pub title_id: u32,
    pub name: Option<String>,
    pub chapter_count: Option<u32>,
    pub duration: Option<String>,
    pub duration_seconds: Option<u64>,
    pub size_human: Option<String>,
    pub size_bytes: Option<u64>,
    pub source_file_name: Option<String>,
    pub output_file_name: Option<String>,
    pub streams: Vec<StreamInfo>,
}

/// Raw parsed `DRV:` line payload.
pub type DrvRecord = OpticalDrive;

/// Raw parsed `TINFO:title_id,attr_id,code,value` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TinfoRecord {
    pub title_id: u32,
    pub attr_id: u32,
    pub code: u32,
    pub value: String,
}

/// Raw parsed `SINFO:title_id,stream_id,attr_id,code,value` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SinfoRecord {
    pub title_id: u32,
    pub stream_id: u32,
    pub attr_id: u32,
    pub code: u32,
    pub value: String,
}

/// Raw parsed `PRGV:current,total,max` progress record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrgvRecord {
    pub current: u32,
    pub total: u32,
    pub max: u32,
}

impl PrgvRecord {
    /// Current step completion percentage in `0.0..=100.0`.
    pub fn current_percent(self) -> f64 {
        if self.max == 0 {
            return 0.0;
        }
        ((self.current as f64 / self.max as f64) * 100.0).clamp(0.0, 100.0)
    }

    /// Overall operation completion percentage in `0.0..=100.0`.
    pub fn total_percent(self) -> f64 {
        if self.max == 0 {
            return 0.0;
        }
        ((self.total as f64 / self.max as f64) * 100.0).clamp(0.0, 100.0)
    }
}

/// Typed representation of a single `makemkvcon -r` stdout line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RobotMessage {
    Drv(DrvRecord),
    Tinfo(TinfoRecord),
    Sinfo(SinfoRecord),
    Prgv(PrgvRecord),
    Other { prefix: String, payload: String },
}

/// Dispatches a single line of `makemkvcon -r` output to the corresponding parser stub.
pub fn parse_robot_line(line: &str) -> Result<Option<RobotMessage>, MakeMkvError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let Some((prefix, payload)) = trimmed.split_once(':') else {
        return Err(MakeMkvError::InvalidFormat(trimmed.to_string()));
    };

    match prefix {
        "DRV" => parse_drv(payload).map(|rec| Some(RobotMessage::Drv(rec))),
        "TINFO" => parse_tinfo(payload).map(|rec| Some(RobotMessage::Tinfo(rec))),
        "SINFO" => parse_sinfo(payload).map(|rec| Some(RobotMessage::Sinfo(rec))),
        "PRGV" => parse_prgv(payload).map(|rec| Some(RobotMessage::Prgv(rec))),
        _ => Ok(Some(RobotMessage::Other {
            prefix: prefix.to_string(),
            payload: payload.to_string(),
        })),
    }
}

/// Parser stub for `DRV:index,visible,enabled,flags,drive_name,disc_name[,device_path]`.
pub fn parse_drv(_payload: &str) -> Result<DrvRecord, MakeMkvError> {
    Err(MakeMkvError::NotImplemented("DRV record parser"))
}

/// Parser stub for `TINFO:title_id,attr_id,code,value`.
pub fn parse_tinfo(_payload: &str) -> Result<TinfoRecord, MakeMkvError> {
    Err(MakeMkvError::NotImplemented("TINFO record parser"))
}

/// Parser stub for `SINFO:title_id,stream_id,attr_id,code,value`.
pub fn parse_sinfo(_payload: &str) -> Result<SinfoRecord, MakeMkvError> {
    Err(MakeMkvError::NotImplemented("SINFO record parser"))
}

/// Parser stub for `PRGV:current,total,max`.
pub fn parse_prgv(_payload: &str) -> Result<PrgvRecord, MakeMkvError> {
    Err(MakeMkvError::NotImplemented("PRGV record parser"))
}

/// Builds CLI arguments to enumerate optical drives without scanning a disc's titles.
pub fn build_scan_drives_args() -> Vec<String> {
    vec![
        "-r".to_string(),
        "--cache=1".to_string(),
        "info".to_string(),
        "disc:9999".to_string(),
    ]
}

/// Builds CLI arguments to inspect all titles and streams on a given optical drive index.
pub fn build_scan_disc_args(drive_index: u32) -> Vec<String> {
    vec![
        "-r".to_string(),
        "--progress=-same".to_string(),
        "info".to_string(),
        format!("disc:{drive_index}"),
    ]
}

/// Builds CLI arguments to rip a specific title to an MKV file inside `output_dir`.
pub fn build_rip_title_args(drive_index: u32, title_id: u32, output_dir: &Path) -> Vec<String> {
    vec![
        "-r".to_string(),
        "--progress=-same".to_string(),
        "mkv".to_string(),
        format!("disc:{drive_index}"),
        title_id.to_string(),
        output_dir.to_string_lossy().into_owned(),
    ]
}

/// Async stub for enumerating connected optical drives via `makemkvcon`.
pub async fn scan_drives(_binary_path: &Path) -> Result<Vec<OpticalDrive>, MakeMkvError> {
    Err(MakeMkvError::NotImplemented("scan_drives"))
}

/// Async stub for inspecting titles and tracks on a disc via `makemkvcon`.
pub async fn scan_disc(
    _binary_path: &Path,
    _drive_index: u32,
) -> Result<Vec<TitleInfo>, MakeMkvError> {
    Err(MakeMkvError::NotImplemented("scan_disc"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prgv_percent_handles_zero_max_safely() {
        let zero = PrgvRecord {
            current: 10,
            total: 20,
            max: 0,
        };
        assert_eq!(zero.current_percent(), 0.0);
        assert_eq!(zero.total_percent(), 0.0);

        let half = PrgvRecord {
            current: 32768,
            total: 65536,
            max: 65536,
        };
        assert!((half.current_percent() - 50.0).abs() < 0.01);
        assert!((half.total_percent() - 100.0).abs() < 0.01);
    }

    #[test]
    fn robot_line_dispatcher_routes_prefixes() {
        assert_eq!(parse_robot_line("   ").unwrap(), None);
        assert!(matches!(
            parse_robot_line("DRV:0,2,999,1,\"BD\",\"MOVIE\""),
            Err(MakeMkvError::NotImplemented("DRV record parser"))
        ));
        assert!(matches!(
            parse_robot_line("TINFO:0,2,0,\"Title\""),
            Err(MakeMkvError::NotImplemented("TINFO record parser"))
        ));
        assert!(matches!(
            parse_robot_line("SINFO:0,0,1,6201,\"Video\""),
            Err(MakeMkvError::NotImplemented("SINFO record parser"))
        ));
        assert!(matches!(
            parse_robot_line("PRGV:100,200,65536"),
            Err(MakeMkvError::NotImplemented("PRGV record parser"))
        ));
        assert!(matches!(
            parse_robot_line("MSG:1005,0,1,\"started\""),
            Ok(Some(RobotMessage::Other { prefix, .. })) if prefix == "MSG"
        ));
    }

    #[test]
    fn rip_args_match_makemkv_robot_cli() {
        let args = build_rip_title_args(0, 3, Path::new("/tmp/ripdaddy"));
        assert_eq!(
            args,
            vec!["-r", "--progress=-same", "mkv", "disc:0", "3", "/tmp/ripdaddy"]
        );
    }
}
