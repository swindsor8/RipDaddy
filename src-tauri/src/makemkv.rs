//! MakeMKV CLI robot-mode (`makemkvcon -r`) data structures, full quoted-CSV parsers
//! (`DRV`, `CINFO`, `TINFO`, `SINFO`, `PRGV`), and async drive/disc scanners.

use crate::detect::ResolvedCommand;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const DRIVE_SCAN_TIMEOUT: Duration = Duration::from_secs(30);
const DISC_SCAN_TIMEOUT: Duration = Duration::from_secs(180);

/// Subset of MakeMKV `AP_ItemAttributeId` numeric attribute codes used by `CINFO`, `TINFO`, and `SINFO`.
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
    pub const AUDIO_CHANNELS: u32 = 14;
    pub const SOURCE_FILE_NAME: u32 = 16;
    pub const VIDEO_SIZE: u32 = 19;
    pub const VIDEO_FRAME_RATE: u32 = 21;
    pub const OUTPUT_FILE_NAME: u32 = 27;
    pub const CHANNEL_LAYOUT: u32 = 40;
}

/// Errors produced while parsing `makemkvcon -r` output or invoking the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MakeMkvError {
    InvalidFormat(String),
    Io(String),
    Timeout(String),
}

impl fmt::Display for MakeMkvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MakeMkvError::InvalidFormat(msg) => write!(f, "Invalid MakeMKV robot record: {msg}"),
            MakeMkvError::Io(msg) => write!(f, "MakeMKV execution error: {msg}"),
            MakeMkvError::Timeout(msg) => write!(f, "MakeMKV timed out: {msg}"),
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

impl StreamInfo {
    fn new(stream_id: u32) -> Self {
        Self {
            stream_id,
            track_type: TrackType::Unknown,
            codec_id: None,
            codec_short: None,
            codec_long: None,
            language_code: None,
            language_name: None,
            channels: None,
            channel_layout: None,
            video_size: None,
            frame_rate: None,
            name: None,
        }
    }
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

impl TitleInfo {
    fn new(title_id: u32) -> Self {
        Self {
            title_id,
            name: None,
            chapter_count: None,
            duration: None,
            duration_seconds: None,
            size_human: None,
            size_bytes: None,
            source_file_name: None,
            output_file_name: None,
            streams: Vec::new(),
        }
    }
}

/// Complete result returned after scanning an optical disc in a specific drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscScanResult {
    pub drive_index: u32,
    pub disc_name: String,
    pub titles: Vec<TitleInfo>,
}

/// Raw parsed `DRV:` line payload.
pub type DrvRecord = OpticalDrive;

/// Raw parsed `CINFO:attr_id,code,value` disc-level attribute record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CinfoRecord {
    pub attr_id: u32,
    pub code: u32,
    pub value: String,
}

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
    Cinfo(CinfoRecord),
    Tinfo(TinfoRecord),
    Sinfo(SinfoRecord),
    Prgv(PrgvRecord),
    Other { prefix: String, payload: String },
}

/// Splits a MakeMKV robot-mode comma-separated payload, unquoting `"..."` strings
/// and handling backslash escapes (`\"`, `\\`).
pub fn split_robot_fields(payload: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut was_quoted = false;

    for ch in payload.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if in_quotes {
            match ch {
                '\\' => escaped = true,
                '"' => in_quotes = false,
                other => current.push(other),
            }
        } else {
            match ch {
                '"' => {
                    in_quotes = true;
                    was_quoted = true;
                }
                ',' => {
                    if was_quoted {
                        fields.push(std::mem::take(&mut current));
                    } else {
                        fields.push(std::mem::take(&mut current).trim().to_string());
                    }
                    was_quoted = false;
                }
                other => current.push(other),
            }
        }
    }

    if was_quoted {
        fields.push(current);
    } else {
        fields.push(current.trim().to_string());
    }

    fields
}

fn parse_u32_field(fields: &[String], idx: usize, ctx: &str) -> Result<u32, MakeMkvError> {
    let raw = fields
        .get(idx)
        .ok_or_else(|| MakeMkvError::InvalidFormat(format!("{ctx}: missing field {idx}")))?;
    raw.parse::<u32>()
        .map_err(|_| MakeMkvError::InvalidFormat(format!("{ctx}: invalid integer '{raw}'")))
}

/// Parses a single line of `makemkvcon -r` stdout into a typed `RobotMessage`.
pub fn parse_robot_line(line: &str) -> Result<Option<RobotMessage>, MakeMkvError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let Some((prefix, payload)) = trimmed.split_once(':') else {
        return Ok(None);
    };

    match prefix {
        "DRV" => parse_drv(payload).map(|rec| Some(RobotMessage::Drv(rec))),
        "CINFO" => parse_cinfo(payload).map(|rec| Some(RobotMessage::Cinfo(rec))),
        "TINFO" => parse_tinfo(payload).map(|rec| Some(RobotMessage::Tinfo(rec))),
        "SINFO" => parse_sinfo(payload).map(|rec| Some(RobotMessage::Sinfo(rec))),
        "PRGV" => parse_prgv(payload).map(|rec| Some(RobotMessage::Prgv(rec))),
        _ => Ok(Some(RobotMessage::Other {
            prefix: prefix.to_string(),
            payload: payload.to_string(),
        })),
    }
}

/// Parses `DRV:index,visible,enabled,flags,drive_name,disc_name[,device_path]`.
pub fn parse_drv(payload: &str) -> Result<DrvRecord, MakeMkvError> {
    let fields = split_robot_fields(payload);
    if fields.len() < 6 {
        return Err(MakeMkvError::InvalidFormat(format!(
            "DRV expected >= 6 fields, got {}",
            fields.len()
        )));
    }

    let device_path = fields
        .get(6)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(OpticalDrive {
        index: parse_u32_field(&fields, 0, "DRV")?,
        visible: parse_u32_field(&fields, 1, "DRV")?,
        enabled: parse_u32_field(&fields, 2, "DRV")?,
        flags: parse_u32_field(&fields, 3, "DRV")?,
        drive_name: fields[4].clone(),
        disc_name: fields[5].clone(),
        device_path,
    })
}

/// Parses `CINFO:attr_id,code,value`.
pub fn parse_cinfo(payload: &str) -> Result<CinfoRecord, MakeMkvError> {
    let fields = split_robot_fields(payload);
    if fields.len() < 3 {
        return Err(MakeMkvError::InvalidFormat(format!(
            "CINFO expected 3 fields, got {}",
            fields.len()
        )));
    }
    Ok(CinfoRecord {
        attr_id: parse_u32_field(&fields, 0, "CINFO")?,
        code: parse_u32_field(&fields, 1, "CINFO")?,
        value: fields[2].clone(),
    })
}

/// Parses `TINFO:title_id,attr_id,code,value`.
pub fn parse_tinfo(payload: &str) -> Result<TinfoRecord, MakeMkvError> {
    let fields = split_robot_fields(payload);
    if fields.len() < 4 {
        return Err(MakeMkvError::InvalidFormat(format!(
            "TINFO expected 4 fields, got {}",
            fields.len()
        )));
    }
    Ok(TinfoRecord {
        title_id: parse_u32_field(&fields, 0, "TINFO")?,
        attr_id: parse_u32_field(&fields, 1, "TINFO")?,
        code: parse_u32_field(&fields, 2, "TINFO")?,
        value: fields[3].clone(),
    })
}

/// Parses `SINFO:title_id,stream_id,attr_id,code,value`.
pub fn parse_sinfo(payload: &str) -> Result<SinfoRecord, MakeMkvError> {
    let fields = split_robot_fields(payload);
    if fields.len() < 5 {
        return Err(MakeMkvError::InvalidFormat(format!(
            "SINFO expected 5 fields, got {}",
            fields.len()
        )));
    }
    Ok(SinfoRecord {
        title_id: parse_u32_field(&fields, 0, "SINFO")?,
        stream_id: parse_u32_field(&fields, 1, "SINFO")?,
        attr_id: parse_u32_field(&fields, 2, "SINFO")?,
        code: parse_u32_field(&fields, 3, "SINFO")?,
        value: fields[4].clone(),
    })
}

/// Parses `PRGV:current,total,max`.
pub fn parse_prgv(payload: &str) -> Result<PrgvRecord, MakeMkvError> {
    let fields = split_robot_fields(payload);
    if fields.len() < 3 {
        return Err(MakeMkvError::InvalidFormat(format!(
            "PRGV expected 3 fields, got {}",
            fields.len()
        )));
    }
    Ok(PrgvRecord {
        current: parse_u32_field(&fields, 0, "PRGV")?,
        total: parse_u32_field(&fields, 1, "PRGV")?,
        max: parse_u32_field(&fields, 2, "PRGV")?,
    })
}

/// Parses `H:MM:SS` or `MM:SS` duration strings into total seconds.
pub fn parse_duration_seconds(raw: &str) -> Option<u64> {
    let parts: Vec<u64> = raw
        .trim()
        .split(':')
        .map(|p| p.trim().parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [h, m, s] => Some(h * 3600 + m * 60 + s),
        [m, s] => Some(m * 60 + s),
        [s] => Some(*s),
        _ => None,
    }
}

/// Stateful builder that folds streaming `CINFO`, `TINFO`, and `SINFO` lines into a `DiscScanResult`.
#[derive(Debug, Default)]
pub struct DiscScanAssembler {
    disc_name: Option<String>,
    titles: BTreeMap<u32, TitleInfo>,
    streams: BTreeMap<u32, BTreeMap<u32, StreamInfo>>,
}

impl DiscScanAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes a single robot-mode message.
    pub fn push(&mut self, msg: RobotMessage) {
        match msg {
            RobotMessage::Cinfo(c) => {
                if c.attr_id == attr_id::NAME && !c.value.trim().is_empty() {
                    self.disc_name = Some(c.value);
                }
            }
            RobotMessage::Tinfo(t) => {
                let entry = self
                    .titles
                    .entry(t.title_id)
                    .or_insert_with(|| TitleInfo::new(t.title_id));
                match t.attr_id {
                    attr_id::NAME => entry.name = non_empty(t.value),
                    attr_id::CHAPTER_COUNT => entry.chapter_count = t.value.parse::<u32>().ok(),
                    attr_id::DURATION => {
                        entry.duration_seconds = parse_duration_seconds(&t.value);
                        entry.duration = non_empty(t.value);
                    }
                    attr_id::DISK_SIZE => entry.size_human = non_empty(t.value),
                    attr_id::DISK_SIZE_BYTES => entry.size_bytes = t.value.parse::<u64>().ok(),
                    attr_id::SOURCE_FILE_NAME => entry.source_file_name = non_empty(t.value),
                    attr_id::OUTPUT_FILE_NAME => entry.output_file_name = non_empty(t.value),
                    _ => {}
                }
            }
            RobotMessage::Sinfo(s) => {
                self.titles
                    .entry(s.title_id)
                    .or_insert_with(|| TitleInfo::new(s.title_id));
                let stream_map = self.streams.entry(s.title_id).or_default();
                let stream = stream_map
                    .entry(s.stream_id)
                    .or_insert_with(|| StreamInfo::new(s.stream_id));

                match s.attr_id {
                    attr_id::STREAM_TYPE => {
                        let lower = s.value.to_ascii_lowercase();
                        stream.track_type = if lower.contains("video") {
                            TrackType::Video
                        } else if lower.contains("audio") {
                            TrackType::Audio
                        } else if lower.contains("subtitle") {
                            TrackType::Subtitle
                        } else {
                            TrackType::Unknown
                        };
                    }
                    attr_id::NAME => stream.name = non_empty(s.value),
                    attr_id::LANG_CODE => stream.language_code = non_empty(s.value),
                    attr_id::LANG_NAME => stream.language_name = non_empty(s.value),
                    attr_id::CODEC_ID => stream.codec_id = non_empty(s.value),
                    attr_id::CODEC_SHORT => stream.codec_short = non_empty(s.value),
                    attr_id::CODEC_LONG => stream.codec_long = non_empty(s.value),
                    attr_id::AUDIO_CHANNELS => stream.channels = s.value.parse::<u8>().ok(),
                    attr_id::CHANNEL_LAYOUT => stream.channel_layout = non_empty(s.value),
                    attr_id::VIDEO_SIZE => stream.video_size = non_empty(s.value),
                    attr_id::VIDEO_FRAME_RATE => stream.frame_rate = non_empty(s.value),
                    _ => {}
                }
            }
            RobotMessage::Drv(_) | RobotMessage::Prgv(_) | RobotMessage::Other { .. } => {}
        }
    }

    /// Finalizes the scan into an ordered `DiscScanResult`, filtering by `min_length_seconds`.
    pub fn finish(mut self, drive_index: u32, min_length_seconds: u64) -> DiscScanResult {
        let mut titles = Vec::new();
        for (title_id, mut title) in self.titles {
            if let Some(stream_map) = self.streams.remove(&title_id) {
                title.streams = stream_map.into_values().collect();
            }
            let passes_filter = min_length_seconds == 0
                || title
                    .duration_seconds
                    .map_or(true, |secs| secs >= min_length_seconds);
            if passes_filter {
                titles.push(title);
            }
        }

        DiscScanResult {
            drive_index,
            disc_name: self.disc_name.unwrap_or_default(),
            titles,
        }
    }
}

fn non_empty(val: String) -> Option<String> {
    let trimmed = val.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
pub fn build_scan_disc_args(drive_index: u32, min_length_seconds: u64) -> Vec<String> {
    vec![
        "-r".to_string(),
        "--progress=-same".to_string(),
        format!("--minlength={min_length_seconds}"),
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

#[cfg(windows)]
fn configure_platform_command(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_platform_command(_cmd: &mut Command) {}

/// Enumerates connected optical drives via `makemkvcon -r`.
pub async fn scan_drives(resolved: &ResolvedCommand) -> Result<Vec<OpticalDrive>, MakeMkvError> {
    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.prefix_args)
        .args(build_scan_drives_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|err| MakeMkvError::Io(format!("Failed to launch makemkvcon: {err}")))?;

    let output = timeout(DRIVE_SCAN_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| MakeMkvError::Timeout("Optical drive enumeration timed out.".to_string()))?
        .map_err(|err| MakeMkvError::Io(format!("Failed reading drive scan output: {err}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut drives = Vec::new();
    for line in stdout.lines() {
        if let Ok(Some(RobotMessage::Drv(drive))) = parse_robot_line(line) {
            // MakeMKV reports unused slots with visible == 256 and empty drive_name.
            if drive.visible != 256 && !drive.drive_name.trim().is_empty() {
                drives.push(drive);
            }
        }
    }

    Ok(drives)
}

/// Scans an inserted optical disc in `drive_index` and assembles its titles & streams.
pub async fn scan_disc(
    resolved: &ResolvedCommand,
    drive_index: u32,
    min_length_seconds: u64,
) -> Result<DiscScanResult, MakeMkvError> {
    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.prefix_args)
        .args(build_scan_disc_args(drive_index, min_length_seconds))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|err| MakeMkvError::Io(format!("Failed to launch makemkvcon: {err}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MakeMkvError::Io("Failed to capture makemkvcon stdout.".to_string()))?;

    let scan_future = async {
        let mut assembler = DiscScanAssembler::new();
        let mut last_msg: Option<String> = None;
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|err| MakeMkvError::Io(err.to_string()))?
        {
            if let Ok(Some(msg)) = parse_robot_line(&line) {
                if let RobotMessage::Other {
                    ref prefix,
                    ref payload,
                } = msg
                {
                    if prefix == "MSG" {
                        let fields = split_robot_fields(payload);
                        if let Some(text) = fields.get(3).filter(|s| !s.trim().is_empty()) {
                            last_msg = Some(text.clone());
                        }
                    }
                }
                assembler.push(msg);
            }
        }
        let status = child.wait().await;
        let result = assembler.finish(drive_index, min_length_seconds);
        if result.titles.is_empty() {
            if let Ok(exit) = status {
                if !exit.success() {
                    let detail = last_msg.unwrap_or_else(|| {
                        format!("makemkvcon exited with status {:?}", exit.code())
                    });
                    return Err(MakeMkvError::Io(detail));
                }
            }
        }
        Ok::<DiscScanResult, MakeMkvError>(result)
    };

    timeout(DISC_SCAN_TIMEOUT, scan_future)
        .await
        .map_err(|_| MakeMkvError::Timeout("Disc structure scan timed out.".to_string()))?
}

/// Best-effort cross-platform optical tray eject helper invoked when `auto_eject_disc` is enabled.
pub async fn eject_optical_drive(device_path: Option<&str>) {
    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new("eject");
        if let Some(dev) = device_path.filter(|s| !s.trim().is_empty()) {
            cmd.arg(dev);
        }
        let _ = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = device_path;
        let _ = Command::new("drutil")
            .args(["tray", "eject"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await;
    }

    #[cfg(target_os = "windows")]
    {
        let _ = device_path;
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-Command",
            "(New-Object -ComObject Shell.Application).Namespace(17).Items() | Where-Object { $_.Type -match 'CD|DVD|BD' } | ForEach-Object { $_.InvokeVerb('Eject') }",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
        configure_platform_command(&mut cmd);
        let _ = cmd.status().await;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = device_path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_drv_tinfo_sinfo_and_prgv_lines() {
        let drv_line = r#"DRV:0,2,999,1,"BD-ROM ASUS BW-16D1HT","THE_MATRIX_1999","/dev/sr0""#;
        let Ok(Some(RobotMessage::Drv(drv))) = parse_robot_line(drv_line) else {
            panic!("expected DRV");
        };
        assert_eq!(drv.index, 0);
        assert_eq!(drv.visible, 2);
        assert_eq!(drv.drive_name, "BD-ROM ASUS BW-16D1HT");
        assert_eq!(drv.disc_name, "THE_MATRIX_1999");
        assert_eq!(drv.device_path.as_deref(), Some("/dev/sr0"));

        let prgv_line = "PRGV:32768,65536,65536";
        let Ok(Some(RobotMessage::Prgv(prgv))) = parse_robot_line(prgv_line) else {
            panic!("expected PRGV");
        };
        assert!((prgv.current_percent() - 50.0).abs() < 0.01);
        assert!((prgv.total_percent() - 100.0).abs() < 0.01);
    }

    #[test]
    fn assembles_titles_and_streams_with_min_length_filter() {
        let sample_lines = [
            r#"CINFO:2,0,"THE_MATRIX_1999""#,
            r#"TINFO:0,2,0,"The Matrix""#,
            r#"TINFO:0,8,0,"38""#,
            r#"TINFO:0,9,0,"2:16:18""#,
            r#"TINFO:0,10,0,"28.4 GB""#,
            r#"TINFO:0,11,0,"30494267801""#,
            r#"SINFO:0,0,1,6201,"Video""#,
            r#"SINFO:0,0,6,0,"Mpeg4 AVC""#,
            r#"SINFO:0,0,19,0,"1920x1080""#,
            r#"SINFO:0,1,1,6202,"Audio""#,
            r#"SINFO:0,1,4,0,"English""#,
            r#"SINFO:0,1,6,0,"TrueHD""#,
            r#"SINFO:0,1,40,0,"7.1""#,
            r#"TINFO:1,2,0,"Short Bumper""#,
            r#"TINFO:1,9,0,"0:00:15""#,
        ];

        let mut assembler = DiscScanAssembler::new();
        for line in sample_lines {
            if let Ok(Some(msg)) = parse_robot_line(line) {
                assembler.push(msg);
            }
        }

        let result = assembler.finish(0, 120);
        assert_eq!(result.disc_name, "THE_MATRIX_1999");
        assert_eq!(result.titles.len(), 1);
        assert_eq!(result.titles[0].name.as_deref(), Some("The Matrix"));
        assert_eq!(result.titles[0].duration_seconds, Some(8178));
        assert_eq!(result.titles[0].streams.len(), 2);
        assert_eq!(result.titles[0].streams[0].track_type, TrackType::Video);
        assert_eq!(result.titles[0].streams[1].track_type, TrackType::Audio);
        assert_eq!(
            result.titles[0].streams[1].channel_layout.as_deref(),
            Some("7.1")
        );
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
