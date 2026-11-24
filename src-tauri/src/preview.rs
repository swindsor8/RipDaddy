//! Automatic trickplay frame extraction and rich video/aspect-ratio probing.
//!
//! Extraction strategy (zero bundling, works across Windows, macOS, and Linux including
//! Fedora/Luma systems shipping patent-stripped `ffmpeg-free`):
//! 1. **Tier 1 (Direct `ffmpeg`):** Probes frame 0 with fast keyframe seeking + DAR correction.
//!    When full `ffmpeg` decoders are present, extracts all 8 JPEGs concurrently in ~1s.
//! 2. **Tier 2 (`HandBrakeCLI` + `ffmpeg` bridge):** When `ffmpeg` lacks `h264`/`hevc`/`vc1`
//!    decoders (e.g. Fedora `ffmpeg-free`), uses the user's `HandBrakeCLI` (which bundles full
//!    codec decoders) to emit a tiny 1-second `ffmpeg4` (MPEG-4) MKV snippet per timestamp,
//!    converts frame 0 to a DAR-corrected JPEG via `ffmpeg`, and deletes the snippet immediately.
//! 3. **Tier 3 (`browserFallback`):** When `ffmpeg` is absent, returns `extractor = "browserFallback"`
//!    so the frontend can extract frames via HTML5 `<video>` + `<canvas>`.

use crate::detect::{self, ResolvedCommand};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::time::timeout;

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const FRAME_EXTRACT_TIMEOUT: Duration = Duration::from_secs(25);
const HANDBRAKE_SNIPPET_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_CONCURRENT_EXTRACTS: usize = 4;
const DEFAULT_FRAME_COUNT: usize = 8;
const MAX_PREVIEW_WIDTH: u32 = 1920;

/// Detailed technical metadata probed from a local video file for the Preview Inspector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoMediaInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub display_width: Option<u32>,
    pub display_height: Option<u32>,
    pub display_aspect_ratio: Option<String>,
    pub sample_aspect_ratio: Option<String>,
    pub video_codec: Option<String>,
    pub pixel_format: Option<String>,
    pub fps: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub bitrate_kbps: Option<u64>,
    pub file_size_bytes: Option<u64>,
    pub audio_summary: Option<String>,
    pub subtitle_count: u32,
}

impl Default for VideoMediaInfo {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            display_width: None,
            display_height: None,
            display_aspect_ratio: None,
            sample_aspect_ratio: None,
            video_codec: None,
            pixel_format: None,
            fps: None,
            duration_seconds: None,
            bitrate_kbps: None,
            file_size_bytes: None,
            audio_summary: None,
            subtitle_count: 0,
        }
    }
}

/// Single extracted trickplay still frame with its source timestamp and file path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFrame {
    pub index: usize,
    pub timestamp_seconds: f64,
    pub timestamp_label: String,
    pub path: String,
}

/// Complete trickplay preview bundle returned to the Svelte Preview Drawer & Lightbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrickplayPreview {
    pub source_path: String,
    pub media_info: VideoMediaInfo,
    pub frames: Vec<PreviewFrame>,
    pub extractor: String,
    pub cached: bool,
    pub note: Option<String>,
}

#[cfg(windows)]
fn configure_platform_command(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_platform_command(_cmd: &mut Command) {}

/// Formats a timestamp in seconds as `H:MM:SS` (or `M:SS` when under 1 hour).
pub fn format_timestamp(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Computes `count` evenly spaced frame timestamps across `[8%..92%]` of a video's duration,
/// avoiding black opening bumpers and end credits.
pub fn compute_trickplay_timestamps(duration_seconds: f64, count: usize) -> Vec<f64> {
    let n = count.clamp(1, 16);
    if !duration_seconds.is_finite() || duration_seconds <= 1.0 {
        return vec![0.0];
    }
    if duration_seconds < 12.0 {
        let step = duration_seconds / (n as f64 + 1.0);
        return (1..=n).map(|i| (step * i as f64).max(0.0)).collect();
    }

    let start = duration_seconds * 0.08;
    let end = duration_seconds * 0.92;
    if n == 1 {
        return vec![duration_seconds * 0.5];
    }
    let span = (end - start).max(1.0);
    (0..n)
        .map(|i| start + span * (i as f64 / (n - 1) as f64))
        .collect()
}

/// Parses fractional rational strings emitted by `ffprobe` (e.g. `"24000/1001"`, `"25/1"`, `"16:9"`).
pub fn parse_ratio(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "0/0" || trimmed == "0:0" || trimmed == "N/A" {
        return None;
    }
    let sep = if trimmed.contains('/') {
        '/'
    } else if trimmed.contains(':') {
        ':'
    } else {
        return trimmed.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0);
    };
    let (num_s, den_s) = trimmed.split_once(sep)?;
    let num: f64 = num_s.trim().parse().ok()?;
    let den: f64 = den_s.trim().parse().ok()?;
    if num > 0.0 && den > 0.0 && num.is_finite() && den.is_finite() {
        Some(num / den)
    } else {
        None
    }
}

/// Produces a friendly Display Aspect Ratio label (e.g. `"16:9"`, `"2.39:1"`, `"4:3"`).
pub fn format_aspect_ratio_label(width: u32, height: u32, raw_dar: Option<&str>) -> Option<String> {
    let ratio = raw_dar
        .and_then(parse_ratio)
        .or_else(|| (width > 0 && height > 0).then(|| width as f64 / height as f64))?;

    let named = [
        (4.0 / 3.0, "4:3"),
        (3.0 / 2.0, "3:2"),
        (16.0 / 10.0, "16:10"),
        (16.0 / 9.0, "16:9"),
        (1.85, "1.85:1"),
        (2.0, "2.00:1"),
        (2.35, "2.35:1"),
        (2.39, "2.39:1"),
        (2.40, "2.40:1"),
    ];
    for (target, label) in named {
        if (ratio - target).abs() <= 0.025 {
            return Some(label.to_string());
        }
    }
    Some(format!("{ratio:.2}:1"))
}

fn friendly_codec_name(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "hevc" | "h265" => "HEVC (H.265)".to_string(),
        "h264" | "avc" => "H.264 (AVC)".to_string(),
        "av1" => "AV1".to_string(),
        "vp9" => "VP9".to_string(),
        "mpeg2video" => "MPEG-2".to_string(),
        "vc1" => "VC-1".to_string(),
        "mpeg4" => "MPEG-4".to_string(),
        "truehd" => "TrueHD".to_string(),
        "dts" => "DTS".to_string(),
        "eac3" => "E-AC-3".to_string(),
        "ac3" => "AC-3".to_string(),
        "aac" => "AAC".to_string(),
        "flac" => "FLAC".to_string(),
        "opus" => "Opus".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

/// Parses `ffprobe -print_format json -show_format -show_streams` output into `VideoMediaInfo`.
pub fn parse_ffprobe_json(json_str: &str) -> Option<VideoMediaInfo> {
    let root: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let mut info = VideoMediaInfo::default();

    if let Some(format) = root.get("format") {
        info.duration_seconds = format
            .get("duration")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0);
        info.file_size_bytes = format
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok());
        info.bitrate_kbps = format
            .get("bit_rate")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|bps| bps / 1000);
    }

    if let Some(streams) = root.get("streams").and_then(|v| v.as_array()) {
        for stream in streams {
            let codec_type = stream
                .get("codec_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match codec_type {
                "video" if info.width.is_none() => {
                    let is_attached_pic = stream
                        .get("disposition")
                        .and_then(|d| d.get("attached_pic"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        == 1;
                    if is_attached_pic {
                        continue;
                    }

                    let width = stream.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
                    let height = stream.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);
                    info.width = width;
                    info.height = height;

                    let raw_sar = stream
                        .get("sample_aspect_ratio")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty() && *s != "0:1" && *s != "N/A");
                    let raw_dar = stream
                        .get("display_aspect_ratio")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty() && *s != "0:1" && *s != "N/A");

                    info.sample_aspect_ratio = raw_sar.map(str::to_string);
                    if let (Some(w), Some(h)) = (width, height) {
                        let sar_val = raw_sar.and_then(parse_ratio).unwrap_or(1.0);
                        let disp_w = ((w as f64 * sar_val).round() as u32).max(1);
                        info.display_width = Some(disp_w);
                        info.display_height = Some(h);
                        info.display_aspect_ratio = format_aspect_ratio_label(disp_w, h, raw_dar);
                    }

                    info.video_codec = stream
                        .get("codec_name")
                        .and_then(|v| v.as_str())
                        .map(friendly_codec_name);
                    info.pixel_format = stream
                        .get("pix_fmt")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    info.fps = stream
                        .get("avg_frame_rate")
                        .and_then(|v| v.as_str())
                        .and_then(parse_ratio)
                        .or_else(|| {
                            stream
                                .get("r_frame_rate")
                                .and_then(|v| v.as_str())
                                .and_then(parse_ratio)
                        })
                        .filter(|v| *v > 1.0 && *v <= 240.0);

                    if info.duration_seconds.is_none() {
                        info.duration_seconds = stream
                            .get("duration")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<f64>().ok())
                            .filter(|v| v.is_finite() && *v > 0.0);
                    }
                }
                "audio" if info.audio_summary.is_none() => {
                    let codec = stream
                        .get("codec_name")
                        .and_then(|v| v.as_str())
                        .map(friendly_codec_name)
                        .unwrap_or_else(|| "Audio".to_string());
                    let layout = stream
                        .get("channel_layout")
                        .and_then(|v| v.as_str())
                        .map(|s| s.replace("(side)", "").trim().to_string())
                        .or_else(|| {
                            stream
                                .get("channels")
                                .and_then(|v| v.as_u64())
                                .map(|ch| format!("{ch}ch"))
                        });
                    info.audio_summary = Some(match layout {
                        Some(l) if !l.is_empty() => format!("{codec} {l}"),
                        _ => codec,
                    });
                }
                "subtitle" => {
                    info.subtitle_count += 1;
                }
                _ => {}
            }
        }
    }

    Some(info)
}

/// Parses HandBrakeCLI `--scan --json` output (`JSON Title Set: { ... }`) as a fallback
/// media prober when `ffprobe` is unavailable.
pub fn parse_handbrake_scan_json(stdout: &str) -> Option<VideoMediaInfo> {
    let marker = "JSON Title Set:";
    let idx = stdout.find(marker)?;
    let rest = &stdout[idx + marker.len()..];
    let brace_start = rest.find('{')?;
    let json_slice = &rest[brace_start..];

    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut end_idx = None;
    for (i, ch) in json_slice.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_str {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
        } else if ch == '"' {
            in_str = true;
        } else if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                end_idx = Some(i + 1);
                break;
            }
        }
    }

    let block = &json_slice[..end_idx?];
    let root: serde_json::Value = serde_json::from_str(block).ok()?;
    let title = root.get("TitleList")?.as_array()?.first()?;

    let mut info = VideoMediaInfo::default();
    if let Some(dur) = title.get("Duration") {
        let h = dur.get("Hours").and_then(|v| v.as_u64()).unwrap_or(0);
        let m = dur.get("Minutes").and_then(|v| v.as_u64()).unwrap_or(0);
        let s = dur.get("Seconds").and_then(|v| v.as_u64()).unwrap_or(0);
        let total = (h * 3600 + m * 60 + s) as f64;
        if total > 0.0 {
            info.duration_seconds = Some(total);
        }
    }
    if let Some(geom) = title.get("Geometry") {
        let w = geom.get("Width").and_then(|v| v.as_u64()).map(|v| v as u32);
        let h = geom.get("Height").and_then(|v| v.as_u64()).map(|v| v as u32);
        info.width = w;
        info.height = h;
        let par_num = geom
            .get("PAR")
            .and_then(|p| p.get("Num"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let par_den = geom
            .get("PAR")
            .and_then(|p| p.get("Den"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        if par_num > 0 && par_den > 0 {
            info.sample_aspect_ratio = Some(format!("{par_num}:{par_den}"));
            if let (Some(width), Some(height)) = (w, h) {
                let disp_w = ((width as f64 * (par_num as f64 / par_den as f64)).round() as u32)
                    .max(1);
                info.display_width = Some(disp_w);
                info.display_height = Some(height);
                info.display_aspect_ratio = format_aspect_ratio_label(disp_w, height, None);
            }
        }
    }
    if let Some(fr) = title.get("FrameRate") {
        let num = fr.get("Num").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let den = fr.get("Den").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if num > 0.0 && den > 0.0 {
            info.fps = Some(num / den);
        }
    }
    if let Some(codec) = title.get("VideoCodec").and_then(|v| v.as_str()) {
        info.video_codec = Some(friendly_codec_name(codec));
    }
    if let Some(audio) = title
        .get("AudioList")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
    {
        if let Some(desc) = audio.get("Description").and_then(|v| v.as_str()) {
            info.audio_summary = Some(desc.to_string());
        }
    }
    if let Some(subs) = title.get("SubtitleList").and_then(|v| v.as_array()) {
        info.subtitle_count = subs.len() as u32;
    }

    Some(info)
}

async fn probe_with_ffprobe(ffprobe: &ResolvedCommand, input_path: &Path) -> Option<VideoMediaInfo> {
    let mut cmd = Command::new(&ffprobe.program);
    cmd.args(&ffprobe.prefix_args)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(input_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let child = cmd.spawn().ok()?;
    let output = timeout(PROBE_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_ffprobe_json(&stdout)
}

/// Fast lightweight runtime probe (`~20–40ms`) using `ffprobe` container metadata only
/// (requires zero video decoding, so it works on all codecs even with Fedora `ffmpeg-free`).
/// Fast container-header probe (duration + embedded title/show/season/episode tags)
/// extracted in a single ~20ms `ffprobe` call without decoding any video frames.
#[derive(Debug, Clone, Default)]
pub struct QuickContainerProbe {
    pub duration_seconds: Option<f64>,
    pub title_tag: Option<String>,
    pub show_tag: Option<String>,
    pub season_tag: Option<u32>,
    pub episode_tag: Option<u32>,
}

fn parse_first_uint(raw: &str) -> Option<u32> {
    let digits: String = raw
        .trim()
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

/// Probes container duration and embedded media tags (`title`, `show`, `season_number`,
/// `episode_sort`, `episode_id`) in one fast header-only `ffprobe` pass.
pub async fn probe_video_quick_info(
    ffprobe: &ResolvedCommand,
    input_path: &Path,
) -> QuickContainerProbe {
    let mut cmd = Command::new(&ffprobe.program);
    cmd.args(&ffprobe.prefix_args)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:format_tags=title,show,season_number,episode_sort,episode_id,album",
            "-of",
            "json",
        ])
        .arg(input_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let Ok(child) = cmd.spawn() else {
        return QuickContainerProbe::default();
    };
    let Ok(Ok(output)) = timeout(Duration::from_secs(6), child.wait_with_output()).await else {
        return QuickContainerProbe::default();
    };
    if !output.status.success() {
        return QuickContainerProbe::default();
    }

    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return QuickContainerProbe::default();
    };
    let Some(fmt) = json.get("format") else {
        return QuickContainerProbe::default();
    };

    let duration_seconds = fmt
        .get("duration")
        .and_then(|v| v.as_str())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0);

    let tags = fmt.get("tags").and_then(|v| v.as_object());
    let get_tag = |keys: &[&str]| -> Option<String> {
        let map = tags?;
        for wanted in keys {
            for (k, v) in map {
                if k.eq_ignore_ascii_case(wanted) {
                    if let Some(s) = v.as_str() {
                        let t = s.trim();
                        if !t.is_empty() {
                            return Some(t.to_string());
                        }
                    }
                }
            }
        }
        None
    };

    let title_tag = get_tag(&["title"]);
    let show_tag = get_tag(&["show", "series"]);
    let season_tag = get_tag(&["season_number", "season"]).and_then(|s| parse_first_uint(&s));
    let episode_tag =
        get_tag(&["episode_sort", "episode_id", "episode", "track"]).and_then(|s| {
            let n = parse_first_uint(&s)?;
            (1..=500).contains(&n).then_some(n)
        });

    QuickContainerProbe {
        duration_seconds,
        title_tag,
        show_tag,
        season_tag,
        episode_tag,
    }
}

pub async fn probe_video_duration_seconds(
    ffprobe: &ResolvedCommand,
    input_path: &Path,
) -> Option<f64> {
    probe_video_quick_info(ffprobe, input_path)
        .await
        .duration_seconds
}

async fn probe_with_handbrake(
    handbrake: &ResolvedCommand,
    input_path: &Path,
) -> Option<VideoMediaInfo> {
    let mounted = match input_path.parent() {
        Some(parent) => handbrake.with_flatpak_paths(&[parent]),
        None => handbrake.clone(),
    };
    let mut cmd = Command::new(&mounted.program);
    cmd.args(&mounted.prefix_args)
        .args(["--json", "--scan", "--previews", "2:0", "-i"])
        .arg(input_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let child = cmd.spawn().ok()?;
    let output = timeout(PROBE_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_handbrake_scan_json(&stdout)
}

fn vf_dar_scale_filter() -> String {
    format!(
        "scale=trunc(iw*if(gt(sar\\,0)\\,sar\\,1)/2)*2:ih,setsar=1,scale='min({MAX_PREVIEW_WIDTH},iw)':-2:flags=lanczos,format=yuv420p"
    )
}

fn first_stderr_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Extracts a single JPEG frame directly with `ffmpeg` fast keyframe seeking.
async fn extract_frame_direct_ffmpeg(
    ffmpeg: &ResolvedCommand,
    input_path: &Path,
    out_path: &Path,
    idx: usize,
    ts: f64,
) -> Result<PreviewFrame, String> {
    let filter = vf_dar_scale_filter();
    let ss_arg = format!("{ts:.3}");
    let mut cmd = Command::new(&ffmpeg.program);
    cmd.args(&ffmpeg.prefix_args)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-ss",
            ss_arg.as_str(),
            "-i",
        ])
        .arg(input_path)
        .args([
            "-an",
            "-sn",
            "-dn",
            "-frames:v",
            "1",
            "-vf",
            filter.as_str(),
            "-q:v",
            "3",
            "-y",
        ])
        .arg(out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|err| format!("Failed launching ffmpeg: {err}"))?;
    let output = timeout(FRAME_EXTRACT_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "ffmpeg timed out extracting frame".to_string())?
        .map_err(|err| format!("ffmpeg wait failed: {err}"))?;

    if output.status.success() && tokio::fs::try_exists(out_path).await.unwrap_or(false) {
        Ok(PreviewFrame {
            index: idx,
            timestamp_seconds: ts,
            timestamp_label: format_timestamp(ts),
            path: out_path.to_string_lossy().into_owned(),
        })
    } else {
        Err(first_stderr_line(&output.stderr)
            .unwrap_or_else(|| format!("ffmpeg exited with {:?}", output.status.code())))
    }
}

/// Fallback extractor for systems whose `ffmpeg` lacks patent-encumbered decoders
/// (such as Fedora/Luma's `ffmpeg-free` which disables `h264`, `hevc`, and `vc1`).
/// Uses `HandBrakeCLI` (full built-in decoders) to decode a 1-second `ffmpeg4` (MPEG-4)
/// MKV snippet, extracts frame 0 to a DAR-corrected JPEG with `ffmpeg`, and deletes the snippet.
async fn extract_frame_via_handbrake_bridge(
    handbrake: &ResolvedCommand,
    ffmpeg: &ResolvedCommand,
    input_path: &Path,
    preview_dir: &Path,
    out_path: &Path,
    idx: usize,
    ts: f64,
    duration_seconds: f64,
) -> Result<PreviewFrame, String> {
    let snippet_path = preview_dir.join(format!("tmp_snippet_{idx:02}.mkv"));
    let max_start = (duration_seconds.floor() as u64).saturating_sub(2);
    let start_sec = (ts.floor() as u64).min(max_start);

    let mut mount_dirs: Vec<&Path> = vec![preview_dir];
    if let Some(in_parent) = input_path.parent() {
        mount_dirs.push(in_parent);
    }
    let mounted_hb = handbrake.with_flatpak_paths(&mount_dirs);

    let input_str = input_path.to_string_lossy().into_owned();
    let snippet_str = snippet_path.to_string_lossy().into_owned();
    let start_arg = format!("seconds:{start_sec}");
    let max_w_arg = MAX_PREVIEW_WIDTH.to_string();

    let mut hb_cmd = Command::new(&mounted_hb.program);
    hb_cmd
        .args(&mounted_hb.prefix_args)
        .args([
            "-i",
            input_str.as_str(),
            "-o",
            snippet_str.as_str(),
            "--previews",
            "1:0",
            "--start-at",
            start_arg.as_str(),
            "--stop-at",
            "seconds:1",
            "--format",
            "av_mkv",
            "--encoder",
            "ffmpeg4",
            "--quality",
            "2",
            "--audio",
            "none",
            "--crop",
            "0:0:0:0",
            "--maxWidth",
            max_w_arg.as_str(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut hb_cmd);

    let hb_child = hb_cmd
        .spawn()
        .map_err(|err| format!("Failed launching HandBrakeCLI for preview: {err}"))?;
    let hb_output = timeout(HANDBRAKE_SNIPPET_TIMEOUT, hb_child.wait_with_output())
        .await
        .map_err(|_| "HandBrakeCLI preview snippet timed out".to_string())?
        .map_err(|err| format!("HandBrakeCLI preview wait failed: {err}"))?;

    if !hb_output.status.success() || !tokio::fs::try_exists(&snippet_path).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&snippet_path).await;
        let detail = String::from_utf8_lossy(&hb_output.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("HandBrakeCLI snippet failed")
            .trim()
            .to_string();
        return Err(detail);
    }

    // Convert frame 0 of the MPEG-4 snippet to a DAR-corrected JPEG via ffmpeg
    let filter = vf_dar_scale_filter();
    let mut ff_cmd = Command::new(&ffmpeg.program);
    ff_cmd
        .args(&ffmpeg.prefix_args)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&snippet_path)
        .args([
            "-an",
            "-sn",
            "-dn",
            "-frames:v",
            "1",
            "-vf",
            filter.as_str(),
            "-q:v",
            "3",
            "-y",
        ])
        .arg(out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut ff_cmd);

    let ff_res = match ff_cmd.spawn() {
        Ok(child) => match timeout(FRAME_EXTRACT_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(out)) => {
                if out.status.success() && tokio::fs::try_exists(out_path).await.unwrap_or(false) {
                    Ok(())
                } else {
                    Err(first_stderr_line(&out.stderr)
                        .unwrap_or_else(|| format!("ffmpeg exited with {:?}", out.status.code())))
                }
            }
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => Err("ffmpeg timed out converting preview snippet".to_string()),
        },
        Err(err) => Err(err.to_string()),
    };

    let _ = tokio::fs::remove_file(&snippet_path).await;

    ff_res?;
    Ok(PreviewFrame {
        index: idx,
        timestamp_seconds: ts,
        timestamp_label: format_timestamp(ts),
        path: out_path.to_string_lossy().into_owned(),
    })
}

/// Deterministic FNV-1a 64-bit hash used to build stable per-file trickplay cache directories.
fn fnv1a_hex(parts: &[&[u8]]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for &b in *part {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

async fn compute_file_cache_key(
    input_path: &Path,
    count: usize,
    max_width: u32,
) -> (String, Option<u64>) {
    let meta = tokio::fs::metadata(input_path).await.ok();
    let size = meta.as_ref().map(|m| m.len());
    let mtime = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let key = fnv1a_hex(&[
        input_path.to_string_lossy().as_bytes(),
        &size.unwrap_or(0).to_le_bytes(),
        &mtime.to_le_bytes(),
        &(count as u64).to_le_bytes(),
        &(max_width as u64).to_le_bytes(),
    ]);
    (key, size)
}

/// Generates (or loads cached) evenly spaced trickplay still JPEG frames across a local video file.
pub async fn generate_trickplay_preview(
    input_path: &Path,
    temp_dir: &Path,
    handbrake_cmd: Option<&ResolvedCommand>,
    frame_count: Option<usize>,
    force_regenerate: bool,
) -> Result<TrickplayPreview, String> {
    if !tokio::fs::try_exists(input_path).await.unwrap_or(false) {
        return Err(format!("Video file does not exist: {}", input_path.display()));
    }

    let count = frame_count.unwrap_or(DEFAULT_FRAME_COUNT).clamp(4, 16);
    let (cache_key, fs_size) = compute_file_cache_key(input_path, count, MAX_PREVIEW_WIDTH).await;
    let preview_dir = temp_dir.join("previews").join(format!("tp_{cache_key}"));

    let ffprobe_cmd = detect::locate_optional_tool("ffprobe");
    let ffmpeg_cmd = detect::locate_optional_tool("ffmpeg");

    let mut media_info = match ffprobe_cmd {
        Some(ref probe) => probe_with_ffprobe(probe, input_path).await.unwrap_or_default(),
        None => VideoMediaInfo::default(),
    };
    if media_info.duration_seconds.is_none() || media_info.width.is_none() {
        if let Some(hb) = handbrake_cmd {
            if let Some(hb_info) = probe_with_handbrake(hb, input_path).await {
                if media_info.width.is_none() {
                    media_info.width = hb_info.width;
                    media_info.height = hb_info.height;
                    media_info.display_width = hb_info.display_width;
                    media_info.display_height = hb_info.display_height;
                    media_info.display_aspect_ratio = hb_info.display_aspect_ratio;
                    media_info.sample_aspect_ratio = hb_info.sample_aspect_ratio;
                }
                if media_info.duration_seconds.is_none() {
                    media_info.duration_seconds = hb_info.duration_seconds;
                }
                if media_info.video_codec.is_none() {
                    media_info.video_codec = hb_info.video_codec;
                }
                if media_info.fps.is_none() {
                    media_info.fps = hb_info.fps;
                }
                if media_info.audio_summary.is_none() {
                    media_info.audio_summary = hb_info.audio_summary;
                }
                if media_info.subtitle_count == 0 {
                    media_info.subtitle_count = hb_info.subtitle_count;
                }
            }
        }
    }
    if media_info.file_size_bytes.is_none() {
        media_info.file_size_bytes = fs_size;
    }

    let Some(ffmpeg) = ffmpeg_cmd else {
        return Ok(TrickplayPreview {
            source_path: input_path.to_string_lossy().into_owned(),
            media_info,
            frames: Vec::new(),
            extractor: "browserFallback".to_string(),
            cached: false,
            note: Some(
                "ffmpeg was not found on PATH — using built-in browser frame preview when supported."
                    .to_string(),
            ),
        });
    };

    let duration = media_info.duration_seconds.unwrap_or(60.0);
    let timestamps = compute_trickplay_timestamps(duration, count);

    // Return cached frames immediately if all expected JPEGs already exist on disk
    if !force_regenerate {
        let mut all_present = true;
        let mut cached_frames = Vec::with_capacity(timestamps.len());
        for (idx, ts) in timestamps.iter().enumerate() {
            let frame_path = preview_dir.join(format!("frame_{idx:02}.jpg"));
            if !tokio::fs::try_exists(&frame_path).await.unwrap_or(false) {
                all_present = false;
                break;
            }
            cached_frames.push(PreviewFrame {
                index: idx,
                timestamp_seconds: *ts,
                timestamp_label: format_timestamp(*ts),
                path: frame_path.to_string_lossy().into_owned(),
            });
        }
        if all_present && !cached_frames.is_empty() {
            return Ok(TrickplayPreview {
                source_path: input_path.to_string_lossy().into_owned(),
                media_info,
                frames: cached_frames,
                extractor: "ffmpeg".to_string(),
                cached: true,
                note: None,
            });
        }
    }

    tokio::fs::create_dir_all(&preview_dir)
        .await
        .map_err(|err| format!("Failed creating preview cache directory: {err}"))?;

    // Tier 1: Probe frame 0 directly with ffmpeg (~80ms). If direct ffmpeg can decode this codec,
    // extract all remaining frames concurrently with ffmpeg.
    let first_ts = timestamps[0];
    let first_out = preview_dir.join("frame_00.jpg");
    let first_attempt =
        extract_frame_direct_ffmpeg(&ffmpeg, input_path, &first_out, 0, first_ts).await;

    let mut frames = Vec::new();
    let mut direct_err: Option<String> = None;
    let mut used_extractor = "ffmpeg".to_string();

    match first_attempt {
        Ok(frame0) => {
            frames.push(frame0);
            let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_EXTRACTS));
            let mut handles = Vec::new();
            for (idx, ts) in timestamps.iter().enumerate().skip(1) {
                let sem = Arc::clone(&semaphore);
                let ff = ffmpeg.clone();
                let in_buf = input_path.to_path_buf();
                let out_buf = preview_dir.join(format!("frame_{idx:02}.jpg"));
                let ts_val = *ts;
                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.ok()?;
                    extract_frame_direct_ffmpeg(&ff, &in_buf, &out_buf, idx, ts_val)
                        .await
                        .ok()
                }));
            }
            for handle in handles {
                if let Ok(Some(f)) = handle.await {
                    frames.push(f);
                }
            }
        }
        Err(err) => {
            direct_err = Some(err);
            // Tier 2: HandBrakeCLI (full decoders) -> 1-second MPEG-4 snippet -> ffmpeg JPEG
            if let Some(hb) = handbrake_cmd {
                used_extractor = "handbrake+ffmpeg".to_string();
                let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_EXTRACTS));
                let mut handles = Vec::new();
                for (idx, ts) in timestamps.iter().enumerate() {
                    let sem = Arc::clone(&semaphore);
                    let hb_owned = hb.clone();
                    let ff_owned = ffmpeg.clone();
                    let in_buf = input_path.to_path_buf();
                    let dir_buf = preview_dir.clone();
                    let out_buf = preview_dir.join(format!("frame_{idx:02}.jpg"));
                    let ts_val = *ts;
                    handles.push(tokio::spawn(async move {
                        let _permit = match sem.acquire().await {
                            Ok(p) => p,
                            Err(_) => return Err("Semaphore closed".to_string()),
                        };
                        extract_frame_via_handbrake_bridge(
                            &hb_owned,
                            &ff_owned,
                            &in_buf,
                            &dir_buf,
                            &out_buf,
                            idx,
                            ts_val,
                            duration,
                        )
                        .await
                    }));
                }
                let mut hb_err: Option<String> = None;
                for handle in handles {
                    match handle.await {
                        Ok(Ok(f)) => frames.push(f),
                        Ok(Err(e)) if hb_err.is_none() => hb_err = Some(e),
                        _ => {}
                    }
                }
                if frames.is_empty() {
                    if let Some(e) = hb_err {
                        direct_err = Some(format!(
                            "ffmpeg: {}; HandBrake bridge: {e}",
                            direct_err.unwrap_or_default()
                        ));
                    }
                }
            }
        }
    }

    frames.sort_by_key(|f| f.index);

    if frames.is_empty() {
        let note = match direct_err {
            Some(detail) => format!(
                "Could not extract video frames ({detail})."
            ),
            None => "Could not extract video frames for this file.".to_string(),
        };
        return Ok(TrickplayPreview {
            source_path: input_path.to_string_lossy().into_owned(),
            media_info,
            frames: Vec::new(),
            extractor: "browserFallback".to_string(),
            cached: false,
            note: Some(note),
        });
    }

    Ok(TrickplayPreview {
        source_path: input_path.to_string_lossy().into_owned(),
        media_info,
        frames,
        extractor: used_extractor,
        cached: false,
        note: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_evenly_spaced_trickplay_timestamps() {
        let stamps = compute_trickplay_timestamps(3600.0, 8);
        assert_eq!(stamps.len(), 8);
        assert!((stamps[0] - 288.0).abs() < 0.01);
        assert!((stamps[7] - 3312.0).abs() < 0.01);
        for pair in stamps.windows(2) {
            assert!(pair[1] > pair[0]);
        }
    }

    #[test]
    fn formats_timestamps_and_aspect_ratios() {
        assert_eq!(format_timestamp(65.0), "1:05");
        assert_eq!(format_timestamp(5025.0), "1:23:45");
        assert_eq!(
            format_aspect_ratio_label(1920, 1080, Some("16:9")).as_deref(),
            Some("16:9")
        );
        assert_eq!(
            format_aspect_ratio_label(1920, 804, None).as_deref(),
            Some("2.39:1")
        );
        assert_eq!(
            format_aspect_ratio_label(720, 480, Some("4:3")).as_deref(),
            Some("4:3")
        );
    }

    #[test]
    fn parses_ffprobe_json_with_anamorphic_dvd_aspect_ratio() {
        let sample = r#"{
            "format": {
                "duration": "6125.50",
                "size": "4831838208",
                "bit_rate": "6310000"
            },
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "mpeg2video",
                    "width": 720,
                    "height": 480,
                    "sample_aspect_ratio": "32:27",
                    "display_aspect_ratio": "16:9",
                    "pix_fmt": "yuv420p",
                    "avg_frame_rate": "24000/1001"
                },
                {
                    "codec_type": "audio",
                    "codec_name": "ac3",
                    "channel_layout": "5.1(side)",
                    "channels": 6
                },
                {
                    "codec_type": "subtitle",
                    "codec_name": "dvd_subtitle"
                }
            ]
        }"#;

        let info = parse_ffprobe_json(sample).expect("parse ffprobe json");
        assert_eq!(info.width, Some(720));
        assert_eq!(info.height, Some(480));
        assert_eq!(info.display_width, Some(853));
        assert_eq!(info.display_height, Some(480));
        assert_eq!(info.display_aspect_ratio.as_deref(), Some("16:9"));
        assert_eq!(info.video_codec.as_deref(), Some("MPEG-2"));
        assert_eq!(info.audio_summary.as_deref(), Some("AC-3 5.1"));
        assert_eq!(info.subtitle_count, 1);
        assert!((info.fps.unwrap() - 23.976).abs() < 0.01);
    }

    #[test]
    fn parses_handbrake_scan_json_title_set() {
        let sample = r#"Version: { "Name": "HandBrake" }
JSON Title Set: {
    "TitleList": [
        {
            "Duration": { "Hours": 0, "Minutes": 3, "Seconds": 42 },
            "Geometry": { "Width": 1920, "Height": 1080, "PAR": { "Num": 1, "Den": 1 } },
            "FrameRate": { "Num": 24000, "Den": 1001 },
            "VideoCodec": "hevc",
            "AudioList": [ { "Description": "English (AAC) (2.0 ch)" } ],
            "SubtitleList": []
        }
    ]
}"#;
        let info = parse_handbrake_scan_json(sample).expect("parse handbrake scan");
        assert_eq!(info.width, Some(1920));
        assert_eq!(info.height, Some(1080));
        assert_eq!(info.display_aspect_ratio.as_deref(), Some("16:9"));
        assert_eq!(info.duration_seconds, Some(222.0));
        assert_eq!(info.video_codec.as_deref(), Some("HEVC (H.265)"));
    }
}
