//! Smart video folder scanner, runtime + filename/marker classifier (Movies vs. TV Series
//! vs. Mixed), and automatic per-movie / TV-series catalog matcher.
//!
//! Combines fast container-only `ffprobe` duration checks (`~20–40ms` per file, no decoding)
//! with filename & folder episode markers (`S01E04`, `S1E4`, `1x04`, `Ep04`, `Season 1`)
//! so dropping any folder — whether a jumbled multi-season TV folder, a folder of unrelated
//! raw movie backups (`THE_MATRIX_1999_t00.mkv`, `Inception.2010.REMUX.mkv`), or a mix of
//! both — automatically identifies what it's dealing with.

use crate::detect;
use crate::metadata::{
    self, EpisodeInfo, MediaKind, MediaMetadata, MetadataProvider, MetadataProviderConfig,
};
use crate::preview;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "webm"];
const MAX_SCAN_DEPTH: usize = 3;
const MAX_CONCURRENT_PROBES: usize = 6;
const MAX_CONCURRENT_MOVIE_LOOKUPS: usize = 4;

/// Feature-film runtime heuristic (`>= 75 minutes` without explicit TV episode markers).
const MOVIE_DURATION_THRESHOLD_SECS: f64 = 4500.0;

/// High-level detected folder category returned to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderContentKind {
    TvSeries,
    Movies,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenericFolderHint {
    Movies,
    Tv,
    Neutral,
    Named,
}

/// Parsed filename + parent-folder hints extracted locally before querying any catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedVideoHints {
    pub title_guess: Option<String>,
    pub year_guess: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub explicit_tv_marker: bool,
    pub in_season_folder: bool,
    pub episode_title_hint: Option<String>,
}

/// One discovered local video file enriched with size, runtime, detected `MediaKind`
/// (`movie` vs `tvEpisode`), and matched TV episode or per-row Movie metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentifiedVideoFile {
    pub path: String,
    pub file_name: String,
    pub file_size_bytes: Option<u64>,
    pub duration_seconds: Option<f64>,
    pub media_kind: MediaKind,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_title: Option<String>,
    pub movie_metadata: Option<MediaMetadata>,
    pub movie_candidates: Vec<MediaMetadata>,
    pub matched_from_catalog: bool,
    pub needs_title: bool,
}

/// Full result returned after scanning a folder (or list of files) and auto-identifying
/// whether it contains TV episodes, unrelated movies, or a mix of both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderIdentifyResult {
    pub folder_path: Option<String>,
    pub detected_kind: FolderContentKind,
    pub inferred_show_query: String,
    pub provider_used: Option<MetadataProvider>,
    pub matched_show: Option<MediaMetadata>,
    pub show_candidates: Vec<MediaMetadata>,
    pub episodes_catalog: Vec<EpisodeInfo>,
    pub files: Vec<IdentifiedVideoFile>,
    pub total_size_bytes: u64,
    pub status_message: String,
}

fn is_video_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') || name.starts_with("._") {
        return false;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            VIDEO_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

/// Recursively collects video files inside `folder` up to `MAX_SCAN_DEPTH` levels deep.
pub fn collect_video_files_in_folder(folder: &Path) -> Result<Vec<(PathBuf, Option<u64>)>, String> {
    if !folder.exists() || !folder.is_dir() {
        return Err(format!(
            "Selected folder does not exist or is not a directory: {}",
            folder.display()
        ));
    }

    let mut collected = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(folder.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                if depth < MAX_SCAN_DEPTH {
                    let hidden = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with('.'));
                    if !hidden {
                        stack.push((path, depth + 1));
                    }
                }
            } else if meta.is_file() && is_video_file(&path) {
                collected.push((path, Some(meta.len())));
            }
        }
    }

    collected.sort_by(|a, b| {
        a.0.to_string_lossy()
            .to_ascii_lowercase()
            .cmp(&b.0.to_string_lossy().to_ascii_lowercase())
    });
    Ok(collected)
}

fn classify_generic_folder_name(name: &str) -> GenericFolderHint {
    let cleaned = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    match cleaned.as_str() {
        "movies" | "movie" | "films" | "film" | "cinema" | "4k movies" | "uhd movies"
        | "bluray movies" | "blu ray movies" | "dvd movies" | "feature films"
        | "movie backups" | "movie rips" => GenericFolderHint::Movies,
        "tv" | "tv shows" | "tv show" | "shows" | "series" | "television" | "anime"
        | "tv backups" | "tv rips" => GenericFolderHint::Tv,
        "rips" | "rip" | "backups" | "backup" | "videos" | "video" | "downloads" | "download"
        | "incoming" | "unsorted" | "media" | "ripdaddy" | "staging" | "encode" | "to encode"
        | "plex" | "jellyfin" => GenericFolderHint::Neutral,
        _ => GenericFolderHint::Named,
    }
}

/// Strips bracketed release/tag chunks (`[1080p]`, `{tvdb-12345}`) from a string.
fn strip_bracketed_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut bracket_depth = 0u32;
    let mut brace_depth = 0u32;
    for ch in raw.chars() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            _ if bracket_depth == 0 && brace_depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

fn is_noise_token(upper: &str) -> bool {
    if matches!(
        upper,
        "2160P"
            | "1080P"
            | "1080I"
            | "720P"
            | "576P"
            | "480P"
            | "4K"
            | "UHD"
            | "HDR"
            | "HDR10"
            | "HDR10PLUS"
            | "DV"
            | "DOVI"
            | "SDR"
            | "BLURAY"
            | "BDRIP"
            | "BRRIP"
            | "REMUX"
            | "WEBDL"
            | "WEBRIP"
            | "WEB"
            | "HDTV"
            | "DVDRIP"
            | "DVD"
            | "BD"
            | "X264"
            | "X265"
            | "H264"
            | "H265"
            | "HEVC"
            | "AVC"
            | "AV1"
            | "VP9"
            | "XVID"
            | "DIVX"
            | "10BIT"
            | "8BIT"
            | "HI10P"
            | "DTS"
            | "DTSHD"
            | "MA"
            | "TRUEHD"
            | "ATMOS"
            | "EAC3"
            | "AC3"
            | "AAC"
            | "AAC2"
            | "FLAC"
            | "OPUS"
            | "DDP"
            | "DDP5"
            | "DD5"
            | "DD2"
            | "51"
            | "71"
            | "20"
            | "PROPER"
            | "REPACK"
            | "RERIP"
            | "EXTENDED"
            | "UNCUT"
            | "UNRATED"
            | "DIRECTORS"
            | "MULTI"
            | "DUAL"
            | "COMPLETE"
            | "SERIES"
            | "NF"
            | "AMZN"
            | "DSNP"
            | "HMAX"
            | "ATVP"
            | "HULU"
            | "MAX"
            | "ENG"
            | "ENGLISH"
            | "SUB"
            | "SUBS"
            | "DUB"
            | "INTERNAL"
            | "DISC1"
            | "DISC2"
            | "DISK1"
            | "DISK2"
    ) {
        return true;
    }
    // Strip raw MakeMKV track suffixes like T00, T01, B1, C1, D1
    if let Some(rest) = upper.strip_prefix('T') {
        if rest.len() == 2 && rest.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    if let Some(rest) = upper.strip_prefix('D') {
        if !rest.is_empty() && rest.len() <= 2 && rest.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

fn parse_sxxeyy_token(upper: &str) -> Option<(u32, u32)> {
    let rest = upper.strip_prefix('S')?;
    let e_pos = rest.find('E')?;
    let s_part = &rest[..e_pos];
    let e_part = &rest[e_pos + 1..];
    if s_part.is_empty() || s_part.len() > 3 || !s_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let e_digits: String = e_part.chars().take_while(|c| c.is_ascii_digit()).collect();
    if e_digits.is_empty() || e_digits.len() > 3 {
        return None;
    }
    let season = s_part.parse::<u32>().ok()?;
    let episode = e_digits.parse::<u32>().ok()?;
    Some((season, episode))
}

fn parse_nxnn_token(upper: &str) -> Option<(u32, u32)> {
    let x_pos = upper.find('X')?;
    let left = &upper[..x_pos];
    let right = &upper[x_pos + 1..];
    if left.is_empty()
        || left.len() > 2
        || right.is_empty()
        || right.len() > 3
        || !left.chars().all(|c| c.is_ascii_digit())
        || !right.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let season = left.parse::<u32>().ok()?;
    let episode = right.parse::<u32>().ok()?;
    if season <= 50 && episode <= 500 {
        Some((season, episode))
    } else {
        None
    }
}

fn parse_season_only_token(upper: &str) -> Option<u32> {
    let rest = upper.strip_prefix('S')?;
    if !rest.is_empty() && rest.len() <= 2 && rest.chars().all(|c| c.is_ascii_digit()) {
        return rest.parse::<u32>().ok();
    }
    None
}

fn parse_episode_only_token(upper: &str) -> Option<u32> {
    let rest = upper
        .strip_prefix("EP")
        .or_else(|| upper.strip_prefix('E'))?;
    if !rest.is_empty() && rest.len() <= 3 && rest.chars().all(|c| c.is_ascii_digit()) {
        return rest.parse::<u32>().ok();
    }
    None
}

fn parse_folder_season_hint(folder_name: &str) -> Option<u32> {
    let cleaned = strip_bracketed_tags(folder_name);
    let tokens: Vec<String> = cleaned
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    for (idx, tok) in tokens.iter().enumerate() {
        let upper = tok.to_ascii_uppercase();
        if matches!(upper.as_str(), "SEASON" | "SERIES" | "S") {
            if let Some(next) = tokens.get(idx + 1) {
                if let Ok(num) = next.parse::<u32>() {
                    if num <= 99 {
                        return Some(num);
                    }
                }
            }
        }
        if let Some(s) = parse_season_only_token(&upper) {
            return Some(s);
        }
    }
    None
}

fn title_case_if_shouting(words: Vec<String>) -> String {
    let joined = words.join(" ").trim().to_string();
    if joined.is_empty() {
        return joined;
    }
    let has_alpha = joined.chars().any(|c| c.is_ascii_alphabetic());
    let all_upper = has_alpha
        && joined
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .all(|c| c.is_ascii_uppercase());
    if all_upper {
        words
            .into_iter()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => {
                        let mut out = String::new();
                        out.extend(first.to_uppercase());
                        out.push_str(&chars.as_str().to_ascii_lowercase());
                        out
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        joined
    }
}

fn clean_title_tokens(tokens: &[String]) -> (Option<String>, Option<u16>) {
    let mut words = Vec::new();
    let mut year_guess: Option<u16> = None;

    for (idx, tok) in tokens.iter().enumerate() {
        let upper = tok.to_ascii_uppercase();

        if is_noise_token(&upper) {
            if !words.is_empty() {
                break;
            }
            continue;
        }
        if matches!(upper.as_str(), "SEASON" | "EPISODE" | "EP" | "DISC" | "DISK") {
            if !words.is_empty() {
                break;
            }
            continue;
        }
        if parse_sxxeyy_token(&upper).is_some()
            || parse_nxnn_token(&upper).is_some()
            || parse_season_only_token(&upper).is_some()
        {
            if !words.is_empty() {
                break;
            }
            continue;
        }

        if tok.len() == 4 {
            if let Ok(yr) = tok.parse::<u16>() {
                if (1920..=2099).contains(&yr) && (!words.is_empty() || idx > 0) {
                    year_guess = Some(yr);
                    break;
                }
            }
        }

        words.push(tok.clone());
    }

    let formatted = title_case_if_shouting(words);
    ((!formatted.is_empty()).then_some(formatted), year_guess)
}

fn clean_episode_title_tokens(tokens: &[String]) -> Option<String> {
    let mut words = Vec::new();
    for tok in tokens {
        let upper = tok.to_ascii_uppercase();
        if is_noise_token(&upper) {
            break;
        }
        if tok.len() == 4 {
            if let Ok(yr) = tok.parse::<u16>() {
                if (1920..=2099).contains(&yr) && !words.is_empty() {
                    break;
                }
            }
        }
        words.push(tok.clone());
    }
    let formatted = title_case_if_shouting(words);
    (!formatted.is_empty()).then_some(formatted)
}

/// Parses messy filenames AND parent folder names into title, year, season, episode,
/// explicit TV marker flag, and episode title hints.
pub fn parse_video_filename_hints(path: &Path) -> ParsedVideoHints {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let stripped_stem = strip_bracketed_tags(stem);
    let tokens: Vec<String> = stripped_stem
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();

    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());
    let grandparent_name = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    let parent_season_hint = parent_name.and_then(parse_folder_season_hint);
    let in_season_folder = parent_name.is_some_and(is_season_only_folder_name)
        || parent_season_hint.is_some();

    let mut season: Option<u32> = None;
    let mut episode: Option<u32> = None;
    let mut explicit_tv_marker = false;
    let mut ep_token_start: Option<usize> = None;
    let mut ep_token_end: Option<usize> = None;

    // Pass 1: Explicit SxxEyy, S1E4, NxNN, "Season X Episode Y", or "Sxx Eyy"
    let mut idx = 0;
    while idx < tokens.len() {
        let upper = tokens[idx].to_ascii_uppercase();
        if let Some((s, e)) = parse_sxxeyy_token(&upper) {
            season = Some(s);
            episode = Some(e);
            explicit_tv_marker = true;
            ep_token_start = Some(idx);
            ep_token_end = Some(idx + 1);
            break;
        }
        if let Some((s, e)) = parse_nxnn_token(&upper) {
            season = Some(s);
            episode = Some(e);
            explicit_tv_marker = true;
            ep_token_start = Some(idx);
            ep_token_end = Some(idx + 1);
            break;
        }
        if upper == "SEASON" {
            if let Some(s_val) = tokens.get(idx + 1).and_then(|t| t.parse::<u32>().ok()) {
                if let Some(ep_kw) = tokens.get(idx + 2).map(|t| t.to_ascii_uppercase()) {
                    if matches!(ep_kw.as_str(), "EPISODE" | "EP" | "E") {
                        if let Some(e_val) =
                            tokens.get(idx + 3).and_then(|t| t.parse::<u32>().ok())
                        {
                            season = Some(s_val);
                            episode = Some(e_val);
                            explicit_tv_marker = true;
                            ep_token_start = Some(idx);
                            ep_token_end = Some(idx + 4);
                            break;
                        }
                    }
                }
            }
        }
        if let Some(s_val) = parse_season_only_token(&upper) {
            if let Some(next) = tokens.get(idx + 1) {
                if let Some(e_val) = parse_episode_only_token(&next.to_ascii_uppercase()) {
                    season = Some(s_val);
                    episode = Some(e_val);
                    explicit_tv_marker = true;
                    ep_token_start = Some(idx);
                    ep_token_end = Some(idx + 2);
                    break;
                }
            }
        }
        idx += 1;
    }

    // Pass 2: Explicit "Episode N", "Ep N", "E04", "Ep04"
    if episode.is_none() {
        for (i, tok) in tokens.iter().enumerate() {
            let upper = tok.to_ascii_uppercase();
            if let Some(e_val) = parse_episode_only_token(&upper) {
                episode = Some(e_val);
                explicit_tv_marker = true;
                ep_token_start = Some(i);
                ep_token_end = Some(i + 1);
                break;
            }
            if matches!(upper.as_str(), "EPISODE" | "EP") {
                if let Some(e_val) = tokens.get(i + 1).and_then(|t| t.parse::<u32>().ok()) {
                    episode = Some(e_val);
                    explicit_tv_marker = true;
                    ep_token_start = Some(i);
                    ep_token_end = Some(i + 2);
                    break;
                }
            }
        }
    }

    // Pass 3: Leading episode number ("04 - New York Weenie.mkv") ONLY when inside a season folder
    // (prevents misparsing movies like "12 Angry Men.mkv" or "300 - Rise of an Empire.mkv")
    if episode.is_none() && in_season_folder && !tokens.is_empty() {
        let first = &tokens[0];
        if first.len() <= 3 && first.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(num) = first.parse::<u32>() {
                if (1..=300).contains(&num) && tokens.len() >= 2 {
                    episode = Some(num);
                    explicit_tv_marker = true;
                    ep_token_start = Some(0);
                    ep_token_end = Some(1);
                }
            }
        }
    }

    // Pass 4: 3-digit scene code like "The Office 201 The Dundies" ONLY when inside a season/TV folder
    if episode.is_none() && in_season_folder {
        for (i, tok) in tokens.iter().enumerate() {
            if tok.len() == 3 && tok.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(code) = tok.parse::<u32>() {
                    let s = code / 100;
                    let e = code % 100;
                    if (1..=9).contains(&s)
                        && (1..=45).contains(&e)
                        && !matches!(code, 264 | 265 | 480 | 576 | 720)
                        && i > 0
                    {
                        season = Some(s);
                        episode = Some(e);
                        explicit_tv_marker = true;
                        ep_token_start = Some(i);
                        ep_token_end = Some(i + 1);
                        break;
                    }
                }
            }
        }
    }

    let before_tokens = match ep_token_start {
        Some(start) => &tokens[..start],
        None => &tokens[..],
    };
    let (mut title_guess, mut year_guess) = clean_title_tokens(before_tokens);
    let episode_title_hint = ep_token_end
        .filter(|&end| end < tokens.len())
        .and_then(|end| clean_episode_title_tokens(&tokens[end..]));

    if season.is_none() {
        season = parent_season_hint;
    }

    // Only inherit title from parent/grandparent folder if it is NOT a generic folder ("Movies", "Rips", etc.)
    if title_guess.is_none() || title_guess.as_deref().is_some_and(is_generic_stem) {
        let folder_candidate = parent_name
            .filter(|p| {
                !is_season_only_folder_name(p)
                    && classify_generic_folder_name(p) == GenericFolderHint::Named
            })
            .or(grandparent_name.filter(|p| {
                !is_season_only_folder_name(p)
                    && classify_generic_folder_name(p) == GenericFolderHint::Named
            }));

        if let Some(folder_str) = folder_candidate {
            let folder_stripped = strip_bracketed_tags(folder_str);
            let folder_tokens: Vec<String> = folder_stripped
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            let (folder_title, folder_year) = clean_title_tokens(&folder_tokens);
            if folder_title.is_some() {
                title_guess = folder_title;
            }
            if year_guess.is_none() {
                year_guess = folder_year;
            }
        }
    }

    ParsedVideoHints {
        title_guess,
        year_guess,
        season,
        episode,
        explicit_tv_marker,
        in_season_folder,
        episode_title_hint,
    }
}

fn is_season_only_folder_name(name: &str) -> bool {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "specials" {
        return true;
    }
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    match words.as_slice() {
        ["season", num] | ["series", num] | ["s", num] => {
            num.chars().all(|c| c.is_ascii_digit())
        }
        [single] => parse_season_only_token(&single.to_ascii_uppercase()).is_some(),
        _ => false,
    }
}

fn is_generic_stem(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    if lower.is_empty()
        || lower.starts_with("title")
        || lower.starts_with("video")
        || lower.starts_with("track")
        || lower.starts_with("movie")
        || lower.starts_with("b1_")
        || lower.starts_with("c1_")
        || lower.starts_with("t0")
        || lower.chars().all(|c| c.is_ascii_digit())
    {
        return true;
    }
    // Single-letter + digits MakeMKV dump patterns like "b1", "a1", "f1"
    if lower.len() <= 3
        && lower.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && lower[1..].chars().all(|c| c.is_ascii_digit())
    {
        return true;
    }
    false
}

/// Classifies each individual video file as `MediaKind::Movie` or `MediaKind::TvEpisode`
/// by fusing filename/folder markers (`S01E04`, `1x04`, `Season 1`, `Movies/`) with
/// actual runtime length (`>= 75 min` -> Movie unless explicit TV markers exist).
pub fn classify_files_and_folder(
    folder_path: Option<&Path>,
    hints: &[ParsedVideoHints],
    durations: &[Option<f64>],
) -> (FolderContentKind, Vec<MediaKind>) {
    let folder_hint = folder_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(classify_generic_folder_name)
        .unwrap_or(GenericFolderHint::Neutral);

    // Check if files share one dominant title prefix (strong TV series signal)
    let mut prefix_counts: HashMap<String, usize> = HashMap::new();
    let mut distinct_titles = HashSet::new();
    for h in hints {
        if let Some(ref t) = h.title_guess {
            if !is_generic_stem(t) {
                let norm = t.to_ascii_lowercase();
                *prefix_counts.entry(norm.clone()).or_insert(0) += 1;
                distinct_titles.insert(norm);
            }
        }
    }
    let max_shared_prefix = prefix_counts.values().copied().max().unwrap_or(0);
    let shares_one_show_name = hints.len() >= 2
        && max_shared_prefix >= 2
        && (max_shared_prefix as f64 / hints.len() as f64) >= 0.6;

    let any_explicit_tv = hints.iter().any(|h| h.explicit_tv_marker || h.in_season_folder);

    let mut per_file_kinds = Vec::with_capacity(hints.len());
    for (hint, dur_opt) in hints.iter().zip(durations.iter()) {
        // 1. Explicit episode markers (S01E04, S1E4, 1x04, Ep04, Season 1 folder) always win
        if hint.explicit_tv_marker || hint.in_season_folder {
            per_file_kinds.push(MediaKind::TvEpisode);
            continue;
        }

        // 2. Feature-film runtime (>= 75 min) without explicit TV markers -> Movie!
        if let Some(dur) = *dur_opt {
            if dur >= MOVIE_DURATION_THRESHOLD_SECS {
                per_file_kinds.push(MediaKind::Movie);
                continue;
            }
        }

        // 3. Explicit "Movies" / "Films" folder name without episode markers -> Movie!
        if folder_hint == GenericFolderHint::Movies {
            per_file_kinds.push(MediaKind::Movie);
            continue;
        }

        // 4. Multiple files sharing a single show name -> TV episodes
        if shares_one_show_name || folder_hint == GenericFolderHint::Tv {
            per_file_kinds.push(MediaKind::TvEpisode);
            continue;
        }

        // 5. Multiple distinct titles (e.g. "The Matrix", "Inception", "Toy Story") without TV markers -> Movies!
        if distinct_titles.len() >= 2 && !any_explicit_tv {
            per_file_kinds.push(MediaKind::Movie);
            continue;
        }

        // 6. Shorter unnamed rips (< 75 min) in a named folder -> TV episodes; otherwise Movie if year present
        if let Some(dur) = *dur_opt {
            if dur < MOVIE_DURATION_THRESHOLD_SECS && folder_hint == GenericFolderHint::Named {
                per_file_kinds.push(MediaKind::TvEpisode);
                continue;
            }
        }

        if hint.year_guess.is_some() && !any_explicit_tv {
            per_file_kinds.push(MediaKind::Movie);
        } else {
            per_file_kinds.push(MediaKind::TvEpisode);
        }
    }

    let movie_count = per_file_kinds
        .iter()
        .filter(|k| **k == MediaKind::Movie)
        .count();
    let tv_count = per_file_kinds.len() - movie_count;

    let folder_kind = if movie_count > 0 && tv_count > 0 {
        FolderContentKind::Mixed
    } else if movie_count > 0 {
        FolderContentKind::Movies
    } else {
        FolderContentKind::TvSeries
    };

    (folder_kind, per_file_kinds)
}

fn normalize_title_words(raw: &str) -> HashSet<String> {
    raw.split(|c: char| !c.is_ascii_alphanumeric())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| w.len() >= 2 && !matches!(w.as_str(), "the" | "and" | "of" | "to" | "in" | "at"))
        .collect()
}

fn match_episode_by_title<'a>(
    hint: &str,
    season_filter: Option<u32>,
    catalog: &'a [EpisodeInfo],
) -> Option<&'a EpisodeInfo> {
    let hint_words = normalize_title_words(hint);
    if hint_words.is_empty() {
        return None;
    }

    let mut best: Option<(f64, &'a EpisodeInfo)> = None;
    for ep in catalog {
        if let Some(s) = season_filter {
            if ep.season != s {
                continue;
            }
        }
        let ep_words = normalize_title_words(&ep.title);
        if ep_words.is_empty() {
            continue;
        }
        let overlap = hint_words.intersection(&ep_words).count() as f64;
        if overlap == 0.0 {
            continue;
        }
        let score = (2.0 * overlap) / (hint_words.len() as f64 + ep_words.len() as f64);
        if score >= 0.6 && best.map_or(true, |(best_score, _)| score > best_score) {
            best = Some((score, ep));
        }
    }
    best.map(|(_, ep)| ep)
}

fn infer_consensus_show(
    folder_path: Option<&Path>,
    tv_hints: &[&ParsedVideoHints],
) -> (String, Option<u16>, Option<u32>) {
    let mut counts: HashMap<String, (usize, String)> = HashMap::new();
    let mut year_counts: HashMap<u16, usize> = HashMap::new();
    let mut season_counts: HashMap<u32, usize> = HashMap::new();

    for h in tv_hints {
        if let Some(ref show) = h.title_guess {
            if !is_generic_stem(show) {
                let key = show.to_ascii_lowercase();
                let entry = counts.entry(key).or_insert((0, show.clone()));
                entry.0 += 1;
            }
        }
        if let Some(y) = h.year_guess {
            *year_counts.entry(y).or_insert(0) += 1;
        }
        if let Some(s) = h.season {
            *season_counts.entry(s).or_insert(0) += 1;
        }
    }

    if let Some(folder) = folder_path {
        if let Some(name) = folder.file_name().and_then(|n| n.to_str()) {
            if let Some(s) = parse_folder_season_hint(name) {
                *season_counts.entry(s).or_insert(0) += 2;
            }
            let target_folder = if is_season_only_folder_name(name) {
                folder
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
            } else {
                Some(name)
            };
            if let Some(f_name) = target_folder {
                if classify_generic_folder_name(f_name) == GenericFolderHint::Named {
                    let stripped = strip_bracketed_tags(f_name);
                    let toks: Vec<String> = stripped
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
                        .filter(|t| !t.is_empty())
                        .map(str::to_string)
                        .collect();
                    let (f_show, f_year) = clean_title_tokens(&toks);
                    if let Some(s) = f_show {
                        let key = s.to_ascii_lowercase();
                        let entry = counts.entry(key).or_insert((0, s));
                        entry.0 += 1;
                    }
                    if let Some(y) = f_year {
                        *year_counts.entry(y).or_insert(0) += 2;
                    }
                }
            }
        }
    }

    let best_show = counts
        .into_values()
        .max_by_key(|(c, label)| (*c, label.len()))
        .map(|(_, label)| label)
        .unwrap_or_default();

    let best_year = year_counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(y, _)| y);

    let best_season = season_counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(s, _)| s);

    (best_show, best_year, best_season)
}

async fn probe_all_durations(entries: &[(PathBuf, Option<u64>)]) -> Vec<Option<f64>> {
    let Some(ffprobe) = detect::locate_optional_tool("ffprobe") else {
        return vec![None; entries.len()];
    };
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PROBES));
    let mut handles = Vec::with_capacity(entries.len());

    for (path, _) in entries {
        let sem = Arc::clone(&semaphore);
        let probe = ffprobe.clone();
        let p = path.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            preview::probe_video_duration_seconds(&probe, &p).await
        }));
    }

    let mut durations = Vec::with_capacity(entries.len());
    for h in handles {
        durations.push(h.await.ok().flatten());
    }
    durations
}

/// Scans a folder of video files, probes container durations, classifies Movies vs. TV vs. Mixed,
/// and identifies every movie and/or TV episode via available catalogs.
pub async fn identify_video_folder(
    folder: &Path,
    config: MetadataProviderConfig,
) -> Result<FolderIdentifyResult, String> {
    let entries = collect_video_files_in_folder(folder)?;
    if entries.is_empty() {
        return Err(format!(
            "No video files (.mkv, .mp4, .m4v, .mov, .avi, .ts, .m2ts) were found in {}",
            folder.display()
        ));
    }
    identify_entries(Some(folder), entries, config).await
}

/// Runs automatic length + marker classification and catalog identification on an existing list of files.
pub async fn identify_video_file_list(
    paths: &[String],
    config: MetadataProviderConfig,
) -> Result<FolderIdentifyResult, String> {
    if paths.is_empty() {
        return Err("No video files to identify.".to_string());
    }
    let mut entries = Vec::with_capacity(paths.len());
    for raw in paths {
        let p = PathBuf::from(raw);
        let size = std::fs::metadata(&p).ok().map(|m| m.len());
        entries.push((p, size));
    }
    let common_parent = entries.first().and_then(|(p, _)| p.parent()).map(Path::to_path_buf);
    identify_entries(common_parent.as_deref(), entries, config).await
}

async fn identify_entries(
    folder_path: Option<&Path>,
    entries: Vec<(PathBuf, Option<u64>)>,
    config: MetadataProviderConfig,
) -> Result<FolderIdentifyResult, String> {
    let hints: Vec<ParsedVideoHints> = entries
        .iter()
        .map(|(path, _)| parse_video_filename_hints(path))
        .collect();
    let durations = probe_all_durations(&entries).await;

    let (detected_kind, per_file_kinds) =
        classify_files_and_folder(folder_path, &hints, &durations);
    let total_size_bytes: u64 = entries.iter().filter_map(|(_, sz)| *sz).sum();

    let mut provider_used: Option<MetadataProvider> = None;
    let mut show_candidates: Vec<MediaMetadata> = Vec::new();
    let mut matched_show: Option<MediaMetadata> = None;
    let mut episodes_catalog: Vec<EpisodeInfo> = Vec::new();
    let mut inferred_show = String::new();

    // 1. Identify TV subset (if any files are classified as TV episodes)
    let tv_indices: Vec<usize> = per_file_kinds
        .iter()
        .enumerate()
        .filter_map(|(i, k)| (*k == MediaKind::TvEpisode).then_some(i))
        .collect();
    let mut tv_default_season: Option<u32> = None;

    if !tv_indices.is_empty() {
        let tv_hints: Vec<&ParsedVideoHints> = tv_indices.iter().map(|&i| &hints[i]).collect();
        let (show_q, year_q, def_season) = infer_consensus_show(folder_path, &tv_hints);
        inferred_show = show_q;
        tv_default_season = def_season;

        if !inferred_show.trim().is_empty() {
            if let Ok((prov, candidates)) =
                metadata::auto_search_tv_series(&inferred_show, year_q, &config).await
            {
                provider_used = Some(prov);
                if let Some(top) = candidates.first().cloned() {
                    let mut seasons_hint: Vec<u32> =
                        tv_hints.iter().filter_map(|h| h.season).collect();
                    if let Some(ds) = def_season {
                        seasons_hint.push(ds);
                    }
                    if let Ok((ep_prov, eps)) = metadata::fetch_series_episodes(
                        &top.title,
                        &top.ids,
                        &seasons_hint,
                        &config,
                    )
                    .await
                    {
                        if !eps.is_empty() {
                            provider_used = Some(ep_prov);
                            episodes_catalog = eps;
                        }
                    }
                    matched_show = Some(top);
                }
                show_candidates = candidates;
            }
        }
    }

    // 2. Identify Movie subset individually & concurrently
    let movie_indices: Vec<usize> = per_file_kinds
        .iter()
        .enumerate()
        .filter_map(|(i, k)| (*k == MediaKind::Movie).then_some(i))
        .collect();

    let mut movie_matches: HashMap<usize, (Option<MetadataProvider>, Vec<MediaMetadata>)> =
        HashMap::new();

    if !movie_indices.is_empty() {
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_MOVIE_LOOKUPS));
        let mut handles = Vec::with_capacity(movie_indices.len());

        for idx in movie_indices {
            let hint = hints[idx].clone();
            let cfg = config.clone();
            let sem = Arc::clone(&semaphore);
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.ok()?;
                let query = hint.title_guess.as_deref().map(str::trim).unwrap_or("");
                if query.is_empty() || is_generic_stem(query) {
                    return Some((idx, None, Vec::new()));
                }
                match metadata::auto_search_movie(query, hint.year_guess, &cfg).await {
                    Ok((prov, results)) => Some((idx, Some(prov), results)),
                    Err(_) => Some((idx, None, Vec::new())),
                }
            }));
        }

        for handle in handles {
            if let Ok(Some((idx, prov, results))) = handle.await {
                if provider_used.is_none() && prov.is_some() {
                    provider_used = prov;
                }
                movie_matches.insert(idx, (prov, results));
            }
        }
    }

    // 3. Assemble TV + Movie items cleanly
    let mut files: Vec<IdentifiedVideoFile> = Vec::with_capacity(entries.len());

    // Build TV rows using episode catalog matcher
    if !tv_indices.is_empty() {
        let tv_entries: Vec<(PathBuf, Option<u64>)> =
            tv_indices.iter().map(|&i| entries[i].clone()).collect();
        let tv_hints_owned: Vec<ParsedVideoHints> =
            tv_indices.iter().map(|&i| hints[i].clone()).collect();
        let tv_durs: Vec<Option<f64>> = tv_indices.iter().map(|&i| durations[i]).collect();

        let tv_files = match_tv_files_to_episodes(
            &tv_entries,
            &tv_hints_owned,
            &tv_durs,
            tv_default_season,
            &episodes_catalog,
        );
        files.extend(tv_files);
    }

    // Build per-row Movie items
    for (idx, ((path, size), hint)) in entries.iter().zip(hints.iter()).enumerate() {
        if per_file_kinds[idx] != MediaKind::Movie {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let candidates = movie_matches
            .remove(&idx)
            .map(|(_, list)| list)
            .unwrap_or_default();
        let top_match = candidates.first().cloned();
        let matched_from_catalog = top_match.is_some();

        let clean_title = hint
            .title_guess
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !is_generic_stem(s));

        let fallback_movie_meta = if top_match.is_none() {
            clean_title.map(|t| MediaMetadata {
                kind: MediaKind::Movie,
                title: t.to_string(),
                year: hint.year_guess,
                season: None,
                episode: None,
                episode_title: None,
                ids: Default::default(),
                overview: None,
                poster_url: None,
            })
        } else {
            None
        };

        let needs_title = top_match.is_none() && clean_title.is_none();

        files.push(IdentifiedVideoFile {
            path: path.to_string_lossy().into_owned(),
            file_name,
            file_size_bytes: *size,
            duration_seconds: durations[idx],
            media_kind: MediaKind::Movie,
            season: None,
            episode: None,
            episode_title: None,
            movie_metadata: top_match.or(fallback_movie_meta),
            movie_candidates: candidates,
            matched_from_catalog,
            needs_title,
        });
    }

    // Sort Movies alphabetically by matched title (or filename) while preserving TV season/episode order
    files.sort_by(|a, b| {
        let kind_rank = |k: MediaKind| match k {
            MediaKind::Movie => 0u8,
            MediaKind::TvEpisode => 1u8,
        };
        let a_key = (
            kind_rank(a.media_kind),
            a.season.unwrap_or(u32::MAX),
            a.episode.unwrap_or(u32::MAX),
            a.movie_metadata
                .as_ref()
                .map(|m| m.title.to_ascii_lowercase())
                .unwrap_or_else(|| a.file_name.to_ascii_lowercase()),
        );
        let b_key = (
            kind_rank(b.media_kind),
            b.season.unwrap_or(u32::MAX),
            b.episode.unwrap_or(u32::MAX),
            b.movie_metadata
                .as_ref()
                .map(|m| m.title.to_ascii_lowercase())
                .unwrap_or_else(|| b.file_name.to_ascii_lowercase()),
        );
        a_key.cmp(&b_key)
    });

    let movie_total = files
        .iter()
        .filter(|f| f.media_kind == MediaKind::Movie)
        .count();
    let movie_matched = files
        .iter()
        .filter(|f| f.media_kind == MediaKind::Movie && f.matched_from_catalog)
        .count();
    let tv_total = files
        .iter()
        .filter(|f| f.media_kind == MediaKind::TvEpisode)
        .count();
    let tv_matched = files
        .iter()
        .filter(|f| f.media_kind == MediaKind::TvEpisode && f.matched_from_catalog)
        .count();

    let status_message = match detected_kind {
        FolderContentKind::Movies => {
            let needs = files.iter().filter(|f| f.needs_title).count();
            if needs > 0 {
                format!(
                    "Detected {movie_total} movies · {movie_matched}/{movie_total} auto-matched · {needs} need a title (click to inspect frames)"
                )
            } else {
                format!(
                    "Detected {movie_total} movies · {movie_matched}/{movie_total} auto-matched"
                )
            }
        }
        FolderContentKind::Mixed => {
            let show_part = matched_show
                .as_ref()
                .map(|s| format!(" (\"{}\")", s.title))
                .unwrap_or_default();
            format!(
                "Detected Mixed folder · {movie_total} movie{} ({movie_matched} matched) + {tv_total} TV episode{}{show_part}",
                if movie_total == 1 { "" } else { "s" },
                if tv_total == 1 { "" } else { "s" },
            )
        }
        FolderContentKind::TvSeries => match (&matched_show, provider_used) {
            (Some(show), Some(prov)) => {
                let prov_label = match prov {
                    MetadataProvider::Tvdb => "TVDB",
                    MetadataProvider::Tmdb => "TMDB",
                    MetadataProvider::Tvmaze => "TVMaze",
                    MetadataProvider::Imdb => "IMDb",
                };
                format!(
                    "Detected TV series \"{}\" via {prov_label} · {tv_matched}/{tv_total} episodes matched",
                    show.title,
                )
            }
            _ => format!(
                "Detected {tv_total} TV episodes (parsed locally — search above to match a show)."
            ),
        },
    };

    Ok(FolderIdentifyResult {
        folder_path: folder_path.map(|p| p.to_string_lossy().into_owned()),
        detected_kind,
        inferred_show_query: inferred_show,
        provider_used,
        matched_show,
        show_candidates,
        episodes_catalog,
        files,
        total_size_bytes,
        status_message,
    })
}

/// Maps discovered TV files + parsed hints against an official episode catalog.
pub fn match_tv_files_to_episodes(
    entries: &[(PathBuf, Option<u64>)],
    hints: &[ParsedVideoHints],
    durations: &[Option<f64>],
    default_season: Option<u32>,
    catalog: &[EpisodeInfo],
) -> Vec<IdentifiedVideoFile> {
    let mut by_se: HashMap<(u32, u32), &EpisodeInfo> = HashMap::new();
    for ep in catalog {
        by_se.insert((ep.season, ep.episode), ep);
    }

    let fallback_season = default_season.unwrap_or(1);
    let mut used_episodes: HashSet<(u32, u32)> = HashSet::new();
    let mut results: Vec<IdentifiedVideoFile> = Vec::with_capacity(entries.len());

    for (((path, size), hint), dur) in entries.iter().zip(hints.iter()).zip(durations.iter()) {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let mut season = hint.season.or_else(|| hint.episode.map(|_| fallback_season));
        let mut episode = hint.episode;
        let mut ep_title = hint.episode_title_hint.clone();
        let mut matched_from_catalog = false;

        if let (Some(s), Some(e)) = (season, episode) {
            if let Some(official) = by_se.get(&(s, e)) {
                ep_title = Some(official.title.clone());
                matched_from_catalog = true;
            }
            used_episodes.insert((s, e));
        } else if let Some(ref title_hint) = hint.episode_title_hint {
            if let Some(official) = match_episode_by_title(title_hint, hint.season, catalog) {
                season = Some(official.season);
                episode = Some(official.episode);
                ep_title = Some(official.title.clone());
                matched_from_catalog = true;
                used_episodes.insert((official.season, official.episode));
            }
        } else if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if let Some(official) = match_episode_by_title(stem, Some(fallback_season), catalog) {
                season = Some(official.season);
                episode = Some(official.episode);
                ep_title = Some(official.title.clone());
                matched_from_catalog = true;
                used_episodes.insert((official.season, official.episode));
            }
        }

        results.push(IdentifiedVideoFile {
            path: path.to_string_lossy().into_owned(),
            file_name,
            file_size_bytes: *size,
            duration_seconds: *dur,
            media_kind: MediaKind::TvEpisode,
            season,
            episode,
            episode_title: ep_title,
            movie_metadata: None,
            movie_candidates: Vec::new(),
            matched_from_catalog,
            needs_title: false,
        });
    }

    let mut next_ep = 1u32;
    for item in &mut results {
        if item.episode.is_none() {
            let s = item.season.unwrap_or(fallback_season);
            while used_episodes.contains(&(s, next_ep)) {
                next_ep += 1;
            }
            item.season = Some(s);
            item.episode = Some(next_ep);
            used_episodes.insert((s, next_ep));
            if let Some(official) = by_se.get(&(s, next_ep)) {
                item.episode_title = Some(official.title.clone());
                item.matched_from_catalog = true;
            }
            next_ep += 1;
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_messy_tv_and_movie_filenames() {
        let p1 = Path::new(
            "/media/Breaking.Bad.S01.1080p.REMUX/Breaking.Bad.S01E01.Pilot.1080p.BluRay.REMUX.mkv",
        );
        let h1 = parse_video_filename_hints(p1);
        assert_eq!(h1.title_guess.as_deref(), Some("Breaking Bad"));
        assert_eq!(h1.season, Some(1));
        assert_eq!(h1.episode, Some(1));
        assert!(h1.explicit_tv_marker);

        let p2 = Path::new("/media/Movies/THE_MATRIX_1999_t00.mkv");
        let h2 = parse_video_filename_hints(p2);
        assert_eq!(h2.title_guess.as_deref(), Some("The Matrix"));
        assert_eq!(h2.year_guess, Some(1999));
        assert!(!h2.explicit_tv_marker);

        let p3 = Path::new("/media/Movies/12 Angry Men (1957).mkv");
        let h3 = parse_video_filename_hints(p3);
        assert_eq!(h3.title_guess.as_deref(), Some("12 Angry Men"));
        assert_eq!(h3.year_guess, Some(1957));
        assert_eq!(h3.episode, None);
        assert!(!h3.explicit_tv_marker);
    }

    #[test]
    fn classifies_movies_tv_and_mixed_folders_by_length_and_markers() {
        // 1. Raw unnamed MakeMKV dumps of feature films (>= 75 min) -> Movies
        let movie_paths = [
            Path::new("/rips/Movies/title_t00.mkv"),
            Path::new("/rips/Movies/THE_MATRIX_1999_t01.mkv"),
            Path::new("/rips/Movies/Inception.2010.REMUX.mkv"),
        ];
        let movie_hints: Vec<ParsedVideoHints> = movie_paths
            .iter()
            .map(|p| parse_video_filename_hints(p))
            .collect();
        let movie_durs = [Some(6800.0), Some(8178.0), Some(8880.0)];
        let (kind, kinds) =
            classify_files_and_folder(Some(Path::new("/rips/Movies")), &movie_hints, &movie_durs);
        assert_eq!(kind, FolderContentKind::Movies);
        assert!(kinds.iter().all(|k| *k == MediaKind::Movie));

        // 2. Long 85-minute TV episodes WITH explicit S04E01 markers -> still TV!
        let tv_paths = [
            Path::new("/tv/Stranger Things/Stranger.Things.S04E01.mkv"),
            Path::new("/tv/Stranger Things/Stranger.Things.S04E02.mkv"),
        ];
        let tv_hints: Vec<ParsedVideoHints> =
            tv_paths.iter().map(|p| parse_video_filename_hints(p)).collect();
        let tv_durs = [Some(5100.0), Some(4900.0)];
        let (tv_kind, tv_kinds) = classify_files_and_folder(
            Some(Path::new("/tv/Stranger Things")),
            &tv_hints,
            &tv_durs,
        );
        assert_eq!(tv_kind, FolderContentKind::TvSeries);
        assert!(tv_kinds.iter().all(|k| *k == MediaKind::TvEpisode));

        // 3. Mixed folder: 1 feature film (2h28m) + 2 TV episodes (22m, S01E04 & S02E01)
        let mixed_paths = [
            Path::new("/rips/Unsorted/Inception.2010.mkv"),
            Path::new("/rips/Unsorted/Mickey Mouse S1E4 - New York Weenie.mp4"),
            Path::new("/rips/Unsorted/Mickey Mouse S2E1 - Cable Car Chaos.mp4"),
        ];
        let mixed_hints: Vec<ParsedVideoHints> = mixed_paths
            .iter()
            .map(|p| parse_video_filename_hints(p))
            .collect();
        let mixed_durs = [Some(8880.0), Some(215.0), Some(220.0)];
        let (mix_kind, mix_kinds) = classify_files_and_folder(
            Some(Path::new("/rips/Unsorted")),
            &mixed_hints,
            &mixed_durs,
        );
        assert_eq!(mix_kind, FolderContentKind::Mixed);
        assert_eq!(
            mix_kinds,
            vec![MediaKind::Movie, MediaKind::TvEpisode, MediaKind::TvEpisode]
        );
    }
}
