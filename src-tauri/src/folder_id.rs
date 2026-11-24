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

/// Feature-film runtime heuristic (`>= 72 minutes` without explicit TV episode markers).
const MOVIE_DURATION_THRESHOLD_SECS: f64 = 4320.0;
/// Typical TV episode runtime ceiling (`< 68 minutes`).
const TV_MAX_DURATION_SECS: f64 = 4080.0;

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

fn is_season_keyword(upper: &str) -> bool {
    matches!(
        upper,
        "S" | "SE" | "SEA" | "SEAS" | "SEASO" | "SEASON" | "SSN" | "SN" | "SERIES" | "SER"
    )
}

fn is_episode_keyword(upper: &str) -> bool {
    matches!(
        upper,
        "E" | "EP"
            | "EPI"
            | "EPIS"
            | "EPISO"
            | "EPISOD"
            | "EPISODE"
            | "EPS"
            | "PART"
            | "PT"
    )
}

const SEASON_PREFIXES: &[&str] = &[
    "SEASON", "SERIES", "SEASO", "SEAS", "SEA", "SSN", "SER", "SE", "SN", "S",
];

const EPISODE_PREFIXES: &[&str] = &[
    "EPISODE", "EPISOD", "EPISO", "EPIS", "PART", "EPI", "EPS", "EP", "PT", "E",
];

fn strip_season_prefix(upper: &str) -> Option<&str> {
    for pref in SEASON_PREFIXES {
        if let Some(rest) = upper.strip_prefix(pref) {
            return Some(rest);
        }
    }
    None
}

fn strip_episode_prefix(upper: &str) -> Option<&str> {
    for pref in EPISODE_PREFIXES {
        if let Some(rest) = upper.strip_prefix(pref) {
            return Some(rest);
        }
    }
    None
}

fn parse_season_only_token(upper: &str) -> Option<u32> {
    let rest = strip_season_prefix(upper)?;
    if !rest.is_empty() && rest.len() <= 2 && rest.chars().all(|c| c.is_ascii_digit()) {
        return rest.parse::<u32>().ok().filter(|&n| n <= 50);
    }
    None
}

fn parse_glued_season_episode_token(upper: &str) -> Option<(u32, u32)> {
    let after_season = strip_season_prefix(upper)?;
    let s_digits: String = after_season
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if s_digits.is_empty() || s_digits.len() > 2 {
        return None;
    }
    let rest = &after_season[s_digits.len()..];
    let after_ep = strip_episode_prefix(rest)?;
    let e_digits: String = after_ep
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if e_digits.is_empty() || e_digits.len() > 3 {
        return None;
    }
    // Make sure trailing junk isn't part of an unrelated word (trailing glued text is split in tokenize_stem_smart)
    if e_digits.len() != after_ep.len() {
        return None;
    }
    let season = s_digits.parse::<u32>().ok()?;
    let episode = e_digits.parse::<u32>().ok()?;
    (season <= 50 && episode <= 500).then_some((season, episode))
}

fn parse_episode_only_token(upper: &str) -> Option<u32> {
    let rest = strip_episode_prefix(upper)?;
    if !rest.is_empty() && rest.len() <= 3 && rest.chars().all(|c| c.is_ascii_digit()) {
        return rest.parse::<u32>().ok().filter(|&n| (1..=500).contains(&n));
    }
    None
}

/// Splits a glued TV marker + title word like `"S02E02Fire"` -> `("S02E02", "Fire")`
/// or `"ep2Fire"` -> `("ep2", "Fire")` or `"se02epis4New"` -> `("se02epis4", "New")`.
fn split_glued_marker_and_word(chunk: &str) -> Option<(&str, &str)> {
    let upper = chunk.to_ascii_uppercase();

    // Case 1: `<season_prefix><digits><ep_prefix><digits><Word>`
    if let Some(after_s) = strip_season_prefix(&upper) {
        let s_len = upper.len() - after_s.len();
        let s_dig = after_s.bytes().take_while(|b| b.is_ascii_digit()).count();
        if (1..=2).contains(&s_dig) {
            let rest = &after_s[s_dig..];
            if let Some(after_e) = strip_episode_prefix(rest) {
                let e_pref_len = rest.len() - after_e.len();
                let e_dig = after_e.bytes().take_while(|b| b.is_ascii_digit()).count();
                let split_at = s_len + s_dig + e_pref_len + e_dig;
                if (1..=3).contains(&e_dig)
                    && split_at < chunk.len()
                    && chunk[split_at..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    return Some((&chunk[..split_at], &chunk[split_at..]));
                }
            }
        }
    }

    // Case 2: `<1-2 digits>X<1-3 digits><Word>` (e.g. `1x04New`, `2x02Fire`)
    if let Some(x_pos) = upper.find('X') {
        if (1..=2).contains(&x_pos) && upper[..x_pos].bytes().all(|b| b.is_ascii_digit()) {
            let after_x = &upper[x_pos + 1..];
            let e_dig = after_x.bytes().take_while(|b| b.is_ascii_digit()).count();
            let split_at = x_pos + 1 + e_dig;
            if (1..=3).contains(&e_dig)
                && split_at < chunk.len()
                && chunk[split_at..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
            {
                return Some((&chunk[..split_at], &chunk[split_at..]));
            }
        }
    }

    // Case 3: `<ep_prefix><digits><Word>` (e.g. `ep2Fire`, `epis4New`, `e02Pilot`)
    if let Some(after_e) = strip_episode_prefix(&upper) {
        let e_pref_len = upper.len() - after_e.len();
        let e_dig = after_e.bytes().take_while(|b| b.is_ascii_digit()).count();
        let split_at = e_pref_len + e_dig;
        if (1..=3).contains(&e_dig)
            && split_at < chunk.len()
            && chunk[split_at..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            return Some((&chunk[..split_at], &chunk[split_at..]));
        }
    }

    None
}

fn is_ordinal_suffix(tail_upper: &str) -> bool {
    matches!(tail_upper, "ST" | "ND" | "RD" | "TH")
}

/// Tokenizes a filename stem while gently splitting:
/// - glued marker + word (`ep2Fire` -> `ep2`, `Fire`; `S02E02Fire` -> `S02E02`, `Fire`)
/// - leading digits glued onto a word (`2Fire` -> `2`, `Fire`) while preserving `1080p`, `4k`, `1x04`, `3rd`
/// - CamelCase boundaries (`BreakingBad` -> `Breaking`, `Bad`)
fn tokenize_stem_smart(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in raw
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .filter(|t| !t.is_empty())
    {
        let upper = chunk.to_ascii_uppercase();
        if parse_sxxeyy_token(&upper).is_some()
            || parse_glued_season_episode_token(&upper).is_some()
            || parse_nxnn_token(&upper).is_some()
            || parse_season_only_token(&upper).is_some()
            || parse_episode_only_token(&upper).is_some()
            || is_noise_token(&upper)
        {
            out.push(chunk.to_string());
            continue;
        }

        // Glued TV marker + trailing title word (`ep2Fire`, `S02E02Fire`)
        if let Some((marker, tail)) = split_glued_marker_and_word(chunk) {
            out.push(marker.to_string());
            out.extend(tokenize_stem_smart(tail));
            continue;
        }

        // Leading digits glued onto a word (`2Fire` -> `2`, `Fire`; `04Pilot` -> `04`, `Pilot`)
        let lead_digits = chunk.bytes().take_while(|b| b.is_ascii_digit()).count();
        if (1..=3).contains(&lead_digits) && lead_digits < chunk.len() {
            let tail = &chunk[lead_digits..];
            let tail_upper = tail.to_ascii_uppercase();
            if tail.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && !is_ordinal_suffix(&tail_upper)
                && !matches!(
                    tail_upper.as_str(),
                    "P" | "I" | "K" | "D" | "BIT" | "FPS" | "CH"
                )
            {
                out.push(chunk[..lead_digits].to_string());
                out.extend(tokenize_stem_smart(tail));
                continue;
            }
        }

        // Split lowercase -> Uppercase boundaries (`BreakingBad` -> `Breaking`, `Bad`)
        let chars: Vec<char> = chunk.chars().collect();
        let mut current = String::new();
        for (i, &ch) in chars.iter().enumerate() {
            if i > 0 && ch.is_ascii_uppercase() && chars[i - 1].is_ascii_lowercase() {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            current.push(ch);
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
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
        if is_season_keyword(&upper) {
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
        if is_season_keyword(&upper)
            || is_episode_keyword(&upper)
            || matches!(upper.as_str(), "DISC" | "DISK")
        {
            if !words.is_empty() {
                break;
            }
            continue;
        }
        if parse_sxxeyy_token(&upper).is_some()
            || parse_glued_season_episode_token(&upper).is_some()
            || parse_nxnn_token(&upper).is_some()
            || parse_season_only_token(&upper).is_some()
            || parse_episode_only_token(&upper).is_some()
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
    let tokens = tokenize_stem_smart(&stripped_stem);

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

    // Pass 1: Explicit SxxEyy, se02ep02, NxNN, "season 1 epis 4", "se02 episode 2", "season 1 ep4", "se02 e02"
    let mut idx = 0;
    while idx < tokens.len() {
        let upper = tokens[idx].to_ascii_uppercase();
        if let Some((s, e)) = parse_sxxeyy_token(&upper)
            .or_else(|| parse_glued_season_episode_token(&upper))
        {
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
        // `<season_kw> <s_num> ...` (e.g. `season 1 epis 4`, `se 2 episode 2`, `season 1 ep4`)
        if is_season_keyword(&upper) {
            if let Some(s_val) = tokens
                .get(idx + 1)
                .and_then(|t| t.parse::<u32>().ok())
                .filter(|&n| n <= 50)
            {
                // 4-token: `<season_kw> <s_num> <ep_kw> <e_num>`
                if let Some(ep_kw) = tokens.get(idx + 2).map(|t| t.to_ascii_uppercase()) {
                    if is_episode_keyword(&ep_kw) {
                        if let Some(e_val) = tokens
                            .get(idx + 3)
                            .and_then(|t| t.parse::<u32>().ok())
                            .filter(|&n| (1..=500).contains(&n))
                        {
                            season = Some(s_val);
                            episode = Some(e_val);
                            explicit_tv_marker = true;
                            ep_token_start = Some(idx);
                            ep_token_end = Some(idx + 4);
                            break;
                        }
                    }
                    // 3-token: `<season_kw> <s_num> <glued_ep>` (`season 1 epis4`, `se 2 ep02`)
                    if let Some(e_val) = parse_episode_only_token(&ep_kw) {
                        season = Some(s_val);
                        episode = Some(e_val);
                        explicit_tv_marker = true;
                        ep_token_start = Some(idx);
                        ep_token_end = Some(idx + 3);
                        break;
                    }
                }
            }
        }
        // `<glued_season> ...` (e.g. `se02 episode 2`, `s02 epis 4`, `se02 ep02`)
        if let Some(s_val) = parse_season_only_token(&upper) {
            if let Some(next_upper) = tokens.get(idx + 1).map(|t| t.to_ascii_uppercase()) {
                // 2-token: `<glued_season> <glued_ep>` (`se02 ep2`, `s01 e04`)
                if let Some(e_val) = parse_episode_only_token(&next_upper) {
                    season = Some(s_val);
                    episode = Some(e_val);
                    explicit_tv_marker = true;
                    ep_token_start = Some(idx);
                    ep_token_end = Some(idx + 2);
                    break;
                }
                // 3-token: `<glued_season> <ep_kw> <e_num>` (`se02 episode 2`, `s02 epis 4`)
                if is_episode_keyword(&next_upper) {
                    if let Some(e_val) = tokens
                        .get(idx + 2)
                        .and_then(|t| t.parse::<u32>().ok())
                        .filter(|&n| (1..=500).contains(&n))
                    {
                        season = Some(s_val);
                        episode = Some(e_val);
                        explicit_tv_marker = true;
                        ep_token_start = Some(idx);
                        ep_token_end = Some(idx + 3);
                        break;
                    }
                }
            }
        }
        idx += 1;
    }

    // Pass 1b: Standalone season marker anywhere in filename (`se02`, `season 1`)
    // so even separated forms like `se02 - Fire Escape - epis 2` still capture Season!
    if season.is_none() {
        for (i, tok) in tokens.iter().enumerate() {
            let upper = tok.to_ascii_uppercase();
            if let Some(s_val) = parse_season_only_token(&upper) {
                season = Some(s_val);
                explicit_tv_marker = true;
                break;
            }
            if is_season_keyword(&upper) {
                if let Some(s_val) = tokens
                    .get(i + 1)
                    .and_then(|t| t.parse::<u32>().ok())
                    .filter(|&n| n <= 50)
                {
                    season = Some(s_val);
                    explicit_tv_marker = true;
                    break;
                }
            }
        }
    }

    // Pass 2: Explicit "Episode N", "Epis N", "Ep N", "Part N", "E04", "Ep04", or "#04"
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
            if is_episode_keyword(&upper) {
                if let Some(e_val) = tokens.get(i + 1).and_then(|t| t.parse::<u32>().ok()) {
                    if (1..=500).contains(&e_val) {
                        episode = Some(e_val);
                        explicit_tv_marker = true;
                        ep_token_start = Some(i);
                        ep_token_end = Some(i + 2);
                        break;
                    }
                }
            }
        }
    }

    // Pass 2b: Explicit `#N` marker in raw stem (e.g. "Breaking Bad #3 - Bags.mkv")
    if episode.is_none() {
        if let Some(hash_pos) = stripped_stem.find('#') {
            let after = &stripped_stem[hash_pos + 1..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() && digits.len() <= 3 {
                if let Ok(e_val) = digits.parse::<u32>() {
                    if (1..=500).contains(&e_val) {
                        episode = Some(e_val);
                        explicit_tv_marker = true;
                        if let Some(tok_i) = tokens.iter().position(|t| t == &digits) {
                            ep_token_start = Some(tok_i);
                            ep_token_end = Some(tok_i + 1);
                        }
                    }
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
        [kw, num] if is_season_keyword(&kw.to_ascii_uppercase()) => {
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

/// Enriches filename-parsed hints with embedded container tags (`show`, `title`,
/// `season_number`, `episode_sort`) extracted during the fast `ffprobe` pass.
fn enrich_hints_with_probes(
    hints: &mut [ParsedVideoHints],
    probes: &[preview::QuickContainerProbe],
) {
    for (hint, probe) in hints.iter_mut().zip(probes.iter()) {
        if let Some(ref show) = probe.show_tag {
            hint.explicit_tv_marker = true;
            if hint.title_guess.is_none()
                || hint.title_guess.as_deref().is_some_and(is_generic_stem)
            {
                hint.title_guess = Some(show.clone());
            }
        }
        if hint.season.is_none() {
            if let Some(s) = probe.season_tag {
                hint.season = Some(s);
                hint.explicit_tv_marker = true;
            }
        }
        if hint.episode.is_none() {
            if let Some(e) = probe.episode_tag {
                hint.episode = Some(e);
                hint.explicit_tv_marker = true;
            }
        }
        if let Some(ref title_tag) = probe.title_tag {
            if !is_generic_stem(title_tag) {
                if hint.explicit_tv_marker || probe.show_tag.is_some() {
                    if hint.episode_title_hint.is_none() {
                        hint.episode_title_hint = Some(title_tag.clone());
                    }
                } else if hint.title_guess.is_none()
                    || hint.title_guess.as_deref().is_some_and(is_generic_stem)
                {
                    hint.title_guess = Some(title_tag.clone());
                }
            }
        }
    }
}

fn is_video_spec_number(num: u32) -> bool {
    matches!(num, 264 | 265 | 480 | 576 | 720 | 1080 | 2160) || (1920..=2099).contains(&num)
}

/// Extracts `(prefix_before_number, trailing_number)` from a stem like
/// `"Breaking Bad 03"`, `"messyshow2"`, `"The_Office_-_104"`.
fn split_trailing_number(stem: &str) -> Option<(String, u32)> {
    let stripped = strip_bracketed_tags(stem);
    let tokens = tokenize_stem_smart(&stripped);
    if tokens.is_empty() {
        return None;
    }

    // Drop trailing codec/resolution noise tokens first
    let mut end = tokens.len();
    while end > 0 && is_noise_token(&tokens[end - 1].to_ascii_uppercase()) {
        end -= 1;
    }
    if end == 0 {
        return None;
    }

    let last = &tokens[end - 1];
    // Case 1: standalone numeric token at end (`"Breaking Bad" + "3"`)
    if last.chars().all(|c| c.is_ascii_digit()) && end >= 2 {
        let num = last.parse::<u32>().ok()?;
        if (1..=999).contains(&num) && !is_video_spec_number(num) {
            let prefix = tokens[..end - 1].join(" ");
            if !prefix.trim().is_empty() {
                return Some((prefix, num));
            }
        }
    }

    // Case 2: digits glued directly to end of last word (`"breakingbad03"`, `"show2"`)
    let digit_start = last
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    if digit_start > 0 && digit_start < last.len() {
        let word_part = &last[..digit_start];
        let num_part = &last[digit_start..];
        let num = num_part.parse::<u32>().ok()?;
        if (1..=999).contains(&num) && !is_video_spec_number(num) {
            let mut prefix_tokens = tokens[..end - 1].to_vec();
            prefix_tokens.push(word_part.to_string());
            let prefix = prefix_tokens.join(" ");
            if !prefix.trim().is_empty() {
                return Some((prefix, num));
            }
        }
    }

    None
}

/// Cross-file sibling pass that rescues intentionally messy TV folders where filenames
/// lack `S01E01` tags (e.g. `Breaking Bad 1.mkv` / `Breaking Bad 2.mkv`, `show1.mkv` / `show2.mkv`,
/// `01 Pilot.mkv` / `02 Cats.mkv` outside a Season folder, or `Show 101` / `Show 102`).
pub fn refine_hints_with_siblings(
    entries: &[(PathBuf, Option<u64>)],
    hints: &mut [ParsedVideoHints],
) {
    if entries.len() < 2 {
        return;
    }

    // Pattern A: >= 2 sibling files sharing the same prefix + distinct trailing numbers
    let mut by_prefix: HashMap<String, Vec<(usize, String, u32)>> = HashMap::new();
    for (idx, ((path, _), hint)) in entries.iter().zip(hints.iter()).enumerate() {
        if hint.episode.is_some() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some((prefix, num)) = split_trailing_number(stem) {
            let key = prefix.to_ascii_lowercase();
            by_prefix.entry(key).or_default().push((idx, prefix, num));
        }
    }

    for group in by_prefix.values() {
        let distinct_nums: HashSet<u32> = group.iter().map(|(_, _, n)| *n).collect();
        if group.len() >= 2 && distinct_nums.len() >= 2 {
            let three_digit_scene_count = group
                .iter()
                .filter(|(_, _, n)| {
                    let s = n / 100;
                    let e = n % 100;
                    (1..=9).contains(&s) && (1..=45).contains(&e)
                })
                .count();
            let treat_as_see = three_digit_scene_count >= 2 && three_digit_scene_count == group.len();

            for (idx, prefix, num) in group {
                let hint = &mut hints[*idx];
                if treat_as_see {
                    hint.season = Some(num / 100);
                    hint.episode = Some(num % 100);
                } else {
                    hint.episode = Some(*num);
                }
                hint.explicit_tv_marker = true;
                let cleaned_prefix = title_case_if_shouting(
                    prefix.split_whitespace().map(str::to_string).collect(),
                );
                if !cleaned_prefix.is_empty() && !is_generic_stem(&cleaned_prefix) {
                    hint.title_guess = Some(cleaned_prefix);
                }
            }
        }
    }

    // Pattern B: >= 2 sibling files starting with distinct leading episode numbers
    // (`01 Pilot.mkv`, `02 Cats in the Bag.mkv` — even outside a "Season X" subfolder!)
    let mut leading_candidates: Vec<(usize, u32, Option<String>)> = Vec::new();
    for (idx, ((path, _), hint)) in entries.iter().zip(hints.iter()).enumerate() {
        if hint.episode.is_some() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let stripped = strip_bracketed_tags(stem);
        let tokens = tokenize_stem_smart(&stripped);
        if tokens.len() >= 2 && tokens[0].chars().all(|c| c.is_ascii_digit()) {
            if let Ok(num) = tokens[0].parse::<u32>() {
                if (1..=300).contains(&num) && !is_video_spec_number(num) {
                    let ep_hint = clean_episode_title_tokens(&tokens[1..]);
                    leading_candidates.push((idx, num, ep_hint));
                }
            }
        }
    }

    let distinct_leading: HashSet<u32> = leading_candidates.iter().map(|(_, n, _)| *n).collect();
    if leading_candidates.len() >= 2 && distinct_leading.len() >= 2 {
        for (idx, num, ep_hint) in leading_candidates {
            let hint = &mut hints[idx];
            hint.episode = Some(num);
            hint.explicit_tv_marker = true;
            if hint.episode_title_hint.is_none() {
                hint.episode_title_hint = ep_hint;
            }
            // Prefer parent folder name as show title rather than `"01 Pilot"`
            let parent_show = entries[idx]
                .0
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .filter(|p| {
                    !is_season_only_folder_name(p)
                        && classify_generic_folder_name(p) == GenericFolderHint::Named
                })
                .and_then(|p| {
                    let toks = tokenize_stem_smart(&strip_bracketed_tags(p));
                    clean_title_tokens(&toks).0
                });
            if let Some(show) = parent_show {
                hint.title_guess = Some(show);
            }
        }
    }
}

/// Checks if >= 60% of files share a common leading 2+ word show title prefix
/// (e.g. `"Breaking Bad Pilot"`, `"Breaking Bad Cat"`, `"Breaking Bad Bags"` -> `"breaking bad"`).
fn has_shared_word_prefix(hints: &[ParsedVideoHints]) -> bool {
    if hints.len() < 2 {
        return false;
    }
    let mut prefix_counts: HashMap<String, usize> = HashMap::new();
    for h in hints {
        let Some(ref t) = h.title_guess else {
            continue;
        };
        if is_generic_stem(t) {
            continue;
        }
        let words: Vec<String> = t
            .split_whitespace()
            .map(|w| w.to_ascii_lowercase())
            .collect();
        if words.len() >= 2 {
            let p2 = format!("{} {}", words[0], words[1]);
            *prefix_counts.entry(p2).or_insert(0) += 1;
        }
    }
    let best = prefix_counts.values().copied().max().unwrap_or(0);
    best >= 2 && (best as f64 / hints.len() as f64) >= 0.6
}

/// Classifies each individual video file as `MediaKind::Movie` or `MediaKind::TvEpisode`
/// by fusing filename/folder markers (`S01E04`, `1x04`, `Season 1`, `Movies/`), sibling
/// numbering patterns, and actual runtime length (`>= 72 min` -> Movie, `< 68 min` -> TV).
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
    let shares_one_show_name = (hints.len() >= 2
        && max_shared_prefix >= 2
        && (max_shared_prefix as f64 / hints.len() as f64) >= 0.6)
        || has_shared_word_prefix(hints);

    let any_explicit_tv = hints.iter().any(|h| h.explicit_tv_marker || h.in_season_folder);

    // Probed runtime consensus: a batch of 22-65 min videos is TV, even with totally scrambled names!
    let probed_count = durations.iter().filter(|d| d.is_some()).count();
    let short_tv_count = durations
        .iter()
        .filter_map(|d| *d)
        .filter(|&dur| dur < TV_MAX_DURATION_SECS)
        .count();
    let mostly_tv_runtimes = probed_count >= 2
        && (short_tv_count as f64 / probed_count as f64) >= 0.65
        && folder_hint != GenericFolderHint::Movies;

    let mut per_file_kinds = Vec::with_capacity(hints.len());
    for (hint, dur_opt) in hints.iter().zip(durations.iter()) {
        // 1. Explicit episode markers (S01E04, S1E4, 1x04, Ep04, sibling numbering, Season folder, tags) always win
        if hint.explicit_tv_marker || hint.in_season_folder {
            per_file_kinds.push(MediaKind::TvEpisode);
            continue;
        }

        // 2. Feature-film runtime (>= 72 min) without explicit TV markers -> Movie!
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

        // 4. Short TV-length runtime (< 68 min) in a TV-runtime batch or named show folder -> TV!
        //    (Must run BEFORE `distinct_titles >= 2` so scrambled episode names aren't misread as movies.)
        if let Some(dur) = *dur_opt {
            if dur < TV_MAX_DURATION_SECS
                && (mostly_tv_runtimes || folder_hint == GenericFolderHint::Named || folder_hint == GenericFolderHint::Tv)
            {
                per_file_kinds.push(MediaKind::TvEpisode);
                continue;
            }
        }

        // 5. Multiple files sharing a single show name / prefix, or explicit TV folder -> TV episodes
        if shares_one_show_name || folder_hint == GenericFolderHint::Tv {
            per_file_kinds.push(MediaKind::TvEpisode);
            continue;
        }

        // 6. Multiple distinct titles (e.g. "The Matrix", "Inception") when durations aren't TV-length -> Movies!
        if distinct_titles.len() >= 2 && !any_explicit_tv && !mostly_tv_runtimes {
            per_file_kinds.push(MediaKind::Movie);
            continue;
        }

        if hint.year_guess.is_some() && !any_explicit_tv && !mostly_tv_runtimes {
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

fn normalize_title_words(raw: &str) -> Vec<String> {
    raw.split(|c: char| !c.is_ascii_alphanumeric())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| {
            w.len() >= 2
                && !matches!(
                    w.as_str(),
                    "the" | "and" | "of" | "to" | "in" | "at" | "on" | "for" | "with" | "an"
                )
                && !is_noise_token(&w.to_ascii_uppercase())
        })
        .collect()
}

fn normalized_alnum_compact(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_bytes: Vec<char> = a.chars().collect();
    let b_bytes: Vec<char> = b.chars().collect();
    let m = a_bytes.len();
    let n = b_bytes.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for (i, &ac) in a_bytes.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &bc) in b_bytes.iter().enumerate() {
            let cost = usize::from(ac != bc);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

fn word_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 {
        return 0.0;
    }
    // Prefix match tolerance for plurals/possessives/truncations (e.g. "cats" vs "cat", "pilott" vs "pilot")
    let dist = levenshtein_distance(a, b);
    1.0 - (dist as f64 / max_len as f64)
}

/// Computes fuzzy title similarity between a filename episode-title hint and an official
/// catalog episode title (0.0..=1.0).
fn episode_title_match_score(
    hint: &str,
    show_title_to_strip: Option<&str>,
    official_title: &str,
) -> f64 {
    let show_words: HashSet<String> = show_title_to_strip
        .map(|s| normalize_title_words(s).into_iter().collect())
        .unwrap_or_default();

    let mut hint_words = normalize_title_words(hint);
    if !show_words.is_empty() {
        let stripped: Vec<String> = hint_words
            .iter()
            .filter(|w| !show_words.contains(*w))
            .cloned()
            .collect();
        if !stripped.is_empty() {
            hint_words = stripped;
        }
    }
    let ep_words = normalize_title_words(official_title);
    if hint_words.is_empty() || ep_words.is_empty() {
        return 0.0;
    }

    let hint_compact = hint_words.join("");
    let ep_compact = normalized_alnum_compact(official_title);

    let compact_score = if ep_compact.len() >= 4 && hint_compact.len() >= 4 {
        if hint_compact.contains(&ep_compact) || ep_compact.contains(&hint_compact) {
            let ratio = hint_compact.len().min(ep_compact.len()) as f64
                / hint_compact.len().max(ep_compact.len()) as f64;
            if ratio >= 0.55 {
                0.78 + 0.22 * ratio
            } else {
                0.0
            }
        } else {
            word_similarity(&hint_compact, &ep_compact)
        }
    } else {
        0.0
    };

    let mut ep_hits = 0.0f64;
    for ew in &ep_words {
        let best_w = hint_words
            .iter()
            .map(|hw| word_similarity(hw, ew))
            .fold(0.0f64, f64::max);
        if best_w >= 0.72 {
            ep_hits += best_w;
        }
    }
    let recall = ep_hits / ep_words.len() as f64;
    let dice = (2.0 * ep_hits) / (hint_words.len() as f64 + ep_words.len() as f64);
    let word_score = if ep_words.len() == 1 && ep_words[0].len() >= 4 && recall >= 0.78 {
        recall * 0.85
    } else {
        dice.max(recall * 0.82)
    };

    compact_score.max(word_score)
}

/// Fuzzy episode-title matcher supporting typos (`pilott`, `ozymandius`), glued words
/// (`graymatter`), extra filename junk, and cross-season fallback search.
/// Returns `(score, &EpisodeInfo)`.
fn match_episode_by_title_scored<'a>(
    hint: &str,
    show_title_to_strip: Option<&str>,
    season_filter: Option<u32>,
    catalog: &'a [EpisodeInfo],
    used_episodes: &HashSet<(u32, u32)>,
    min_score: f64,
) -> Option<(f64, &'a EpisodeInfo)> {
    let score_candidates = |restrict_season: Option<u32>| -> Option<(f64, &'a EpisodeInfo)> {
        let mut best: Option<(f64, &'a EpisodeInfo)> = None;
        for ep in catalog {
            if used_episodes.contains(&(ep.season, ep.episode)) {
                continue;
            }
            if let Some(s) = restrict_season {
                if ep.season != s {
                    continue;
                }
            }
            let score = episode_title_match_score(hint, show_title_to_strip, &ep.title);
            if score >= min_score && best.map_or(true, |(best_score, _)| score > best_score) {
                best = Some((score, ep));
            }
        }
        best
    };

    if let Some(hit) = score_candidates(season_filter) {
        return Some(hit);
    }
    if season_filter.is_some() {
        if let Some(hit) = score_candidates(None) {
            return Some(hit);
        }
    }
    None
}

fn match_episode_by_title<'a>(
    hint: &str,
    show_title_to_strip: Option<&str>,
    season_filter: Option<u32>,
    catalog: &'a [EpisodeInfo],
    used_episodes: &HashSet<(u32, u32)>,
) -> Option<&'a EpisodeInfo> {
    match_episode_by_title_scored(
        hint,
        show_title_to_strip,
        season_filter,
        catalog,
        used_episodes,
        0.62,
    )
    .map(|(_, ep)| ep)
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

async fn probe_all_quick_info(
    entries: &[(PathBuf, Option<u64>)],
) -> Vec<preview::QuickContainerProbe> {
    let Some(ffprobe) = detect::locate_optional_tool("ffprobe") else {
        return vec![preview::QuickContainerProbe::default(); entries.len()];
    };
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PROBES));
    let mut handles = Vec::with_capacity(entries.len());

    for (path, _) in entries {
        let sem = Arc::clone(&semaphore);
        let probe = ffprobe.clone();
        let p = path.clone();
        handles.push(tokio::spawn(async move {
            let Ok(_permit) = sem.acquire().await else {
                return preview::QuickContainerProbe::default();
            };
            preview::probe_video_quick_info(&probe, &p).await
        }));
    }

    let mut probes = Vec::with_capacity(entries.len());
    for h in handles {
        probes.push(h.await.unwrap_or_default());
    }
    probes
}

/// Evaluates up to the top 4 show candidates by fetching their episode catalogs and
/// counting how many of the folder's `episode_title_hint`s match each candidate's episodes!
/// This prevents `"Mickey Mouse"` from picking `"Mickey Mouse Clubhouse"` when the folder
/// contains `"New York Weenie"` and `"Fire Escape"` from `"Mickey Mouse" (2013)`.
async fn select_best_show_with_episodes(
    mut candidates: Vec<MediaMetadata>,
    tv_hints: &[&ParsedVideoHints],
    seasons_hint: &[u32],
    config: &MetadataProviderConfig,
) -> (
    Option<MediaMetadata>,
    Vec<MediaMetadata>,
    Option<MetadataProvider>,
    Vec<EpisodeInfo>,
) {
    if candidates.is_empty() {
        return (None, candidates, None, Vec::new());
    }

    let title_hints: Vec<&str> = tv_hints
        .iter()
        .filter_map(|h| h.episode_title_hint.as_deref())
        .map(str::trim)
        .filter(|s| s.len() >= 3)
        .collect();

    // If we have episode title hints and multiple candidates, probe up to 4 candidates
    // so we lock onto the exact series that owns those episodes!
    let max_probe = if !title_hints.is_empty() && candidates.len() > 1 {
        candidates.len().min(4)
    } else {
        1
    };

    let mut best_idx = 0usize;
    let mut best_score = (0usize, 0usize, 0usize); // (title_hits, se_hits, non_empty_catalog)
    let mut best_provider: Option<MetadataProvider> = None;
    let mut best_episodes: Vec<EpisodeInfo> = Vec::new();
    let empty_used: HashSet<(u32, u32)> = HashSet::new();

    for (idx, cand) in candidates.iter().take(max_probe).enumerate() {
        let Ok((ep_prov, eps)) =
            metadata::fetch_series_episodes(&cand.title, &cand.ids, seasons_hint, config).await
        else {
            continue;
        };

        let mut title_hits = 0usize;
        for th in &title_hints {
            if match_episode_by_title(th, Some(&cand.title), None, &eps, &empty_used).is_some() {
                title_hits += 1;
            }
        }
        let se_hits = tv_hints
            .iter()
            .filter(|h| match (h.season, h.episode) {
                (Some(s), Some(e)) => eps.iter().any(|ep| ep.season == s && ep.episode == e),
                _ => false,
            })
            .count();
        let non_empty = usize::from(!eps.is_empty());
        let score = (title_hits, se_hits, non_empty);

        if idx == 0 || score > best_score {
            best_idx = idx;
            best_score = score;
            best_provider = Some(ep_prov);
            best_episodes = eps;
            // Short-circuit if every single episode title hint matched this candidate!
            if !title_hints.is_empty() && title_hits == title_hints.len() {
                break;
            }
        }
    }

    if best_idx > 0 && best_idx < candidates.len() {
        let winner = candidates.remove(best_idx);
        candidates.insert(0, winner);
    }
    let chosen = candidates.first().cloned();
    (chosen, candidates, best_provider, best_episodes)
}

fn named_folder_show_guess(folder_path: Option<&Path>) -> Option<(String, Option<u16>)> {
    let folder = folder_path?;
    let name = folder.file_name().and_then(|n| n.to_str())?;
    let target_name = if is_season_only_folder_name(name) {
        folder
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())?
    } else {
        name
    };
    if classify_generic_folder_name(target_name) != GenericFolderHint::Named {
        return None;
    }
    let toks = tokenize_stem_smart(&strip_bracketed_tags(target_name));
    let (title, year) = clean_title_tokens(&toks);
    title.map(|t| (t, year))
}

/// Re-parses and fuzzy-matches an existing list of local files against a manually chosen
/// TV show and its official episode catalog (used when the user flips a folder to TV or
/// picks a show manually in the search bar).
pub async fn rematch_files_to_series(
    paths: &[String],
    show_title: &str,
    catalog: &[EpisodeInfo],
    default_season: Option<u32>,
) -> Vec<IdentifiedVideoFile> {
    let mut entries = Vec::with_capacity(paths.len());
    for raw in paths {
        let p = PathBuf::from(raw);
        let size = std::fs::metadata(&p).ok().map(|m| m.len());
        entries.push((p, size));
    }
    let mut hints: Vec<ParsedVideoHints> = entries
        .iter()
        .map(|(path, _)| parse_video_filename_hints(path))
        .collect();
    let probes = probe_all_quick_info(&entries).await;
    enrich_hints_with_probes(&mut hints, &probes);
    refine_hints_with_siblings(&entries, &mut hints);
    let durations: Vec<Option<f64>> = probes.iter().map(|p| p.duration_seconds).collect();

    let mut matched = match_tv_files_to_episodes(
        &entries,
        &hints,
        &durations,
        default_season,
        Some(show_title),
        catalog,
    );
    matched.sort_by(|a, b| {
        (
            a.season.unwrap_or(u32::MAX),
            a.episode.unwrap_or(u32::MAX),
            a.file_name.to_ascii_lowercase(),
        )
            .cmp(&(
                b.season.unwrap_or(u32::MAX),
                b.episode.unwrap_or(u32::MAX),
                b.file_name.to_ascii_lowercase(),
            ))
    });
    matched
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
    let mut hints: Vec<ParsedVideoHints> = entries
        .iter()
        .map(|(path, _)| parse_video_filename_hints(path))
        .collect();
    let probes = probe_all_quick_info(&entries).await;
    enrich_hints_with_probes(&mut hints, &probes);
    refine_hints_with_siblings(&entries, &mut hints);
    let durations: Vec<Option<f64>> = probes.iter().map(|p| p.duration_seconds).collect();

    let (mut detected_kind, mut per_file_kinds) =
        classify_files_and_folder(folder_path, &hints, &durations);
    let total_size_bytes: u64 = entries.iter().filter_map(|(_, sz)| *sz).sum();

    let mut provider_used: Option<MetadataProvider> = None;
    let mut show_candidates: Vec<MediaMetadata> = Vec::new();
    let mut matched_show: Option<MediaMetadata> = None;
    let mut episodes_catalog: Vec<EpisodeInfo> = Vec::new();
    let mut inferred_show = String::new();

    // 1. Identify Movie subset individually & concurrently first so we can self-correct
    //    if a named folder of messed-up TV filenames failed movie lookups.
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

        for idx in &movie_indices {
            let idx = *idx;
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

    // Catalog-guided self-correction: if a named folder of >= 2 files was guessed as Movies,
    // fewer than half matched movies, and they aren't feature-film runtimes, test the folder
    // name as a TV series and pivot if it matches a real show!
    if detected_kind == FolderContentKind::Movies && entries.len() >= 2 {
        let matched_movie_count = movie_matches
            .values()
            .filter(|(_, list)| !list.is_empty())
            .count();
        let feature_length_count = durations
            .iter()
            .filter_map(|d| *d)
            .filter(|&dur| dur >= MOVIE_DURATION_THRESHOLD_SECS)
            .count();
        let poor_movie_hit_rate = (matched_movie_count as f64 / entries.len() as f64) < 0.5;
        let not_feature_films = feature_length_count * 2 < entries.len();

        if poor_movie_hit_rate && not_feature_films {
            if let Some((folder_show, folder_year)) = named_folder_show_guess(folder_path) {
                if let Ok((prov, candidates)) =
                    metadata::auto_search_tv_series(&folder_show, folder_year, &config).await
                {
                    if !candidates.is_empty() {
                        detected_kind = FolderContentKind::TvSeries;
                        per_file_kinds.fill(MediaKind::TvEpisode);
                        movie_matches.clear();
                        inferred_show = folder_show;
                        provider_used = Some(prov);
                        let all_hints: Vec<&ParsedVideoHints> = hints.iter().collect();
                        let mut seasons_hint: Vec<u32> =
                            hints.iter().filter_map(|h| h.season).collect();
                        if seasons_hint.is_empty() {
                            seasons_hint.push(1);
                        }
                        let (chosen, reordered, ep_prov, eps) = select_best_show_with_episodes(
                            candidates,
                            &all_hints,
                            &seasons_hint,
                            &config,
                        )
                        .await;
                        if let Some(p) = ep_prov {
                            if !eps.is_empty() {
                                provider_used = Some(p);
                                episodes_catalog = eps;
                            }
                        }
                        matched_show = chosen;
                        show_candidates = reordered;
                    }
                }
            }
        }
    }

    // 2. Identify TV subset (if any files are classified as TV episodes and not already matched above)
    let tv_indices: Vec<usize> = per_file_kinds
        .iter()
        .enumerate()
        .filter_map(|(i, k)| (*k == MediaKind::TvEpisode).then_some(i))
        .collect();
    let mut tv_default_season: Option<u32> = None;

    if !tv_indices.is_empty() {
        let tv_hints: Vec<&ParsedVideoHints> = tv_indices.iter().map(|&i| &hints[i]).collect();
        let (show_q, year_q, def_season) = infer_consensus_show(folder_path, &tv_hints);
        tv_default_season = def_season;

        if matched_show.is_none() {
            // Prefer the named parent folder when filenames inside have low consensus / scrambled stems
            let folder_show_fallback = named_folder_show_guess(folder_path);
            inferred_show = if !show_q.trim().is_empty() {
                show_q
            } else {
                folder_show_fallback
                    .as_ref()
                    .map(|(s, _)| s.clone())
                    .unwrap_or_default()
            };

            let mut tried_queries: Vec<(String, Option<u16>)> = Vec::new();
            if !inferred_show.trim().is_empty() {
                tried_queries.push((inferred_show.clone(), year_q));
            }
            if let Some((f_show, f_year)) = folder_show_fallback {
                if !tried_queries
                    .iter()
                    .any(|(q, _)| q.eq_ignore_ascii_case(&f_show))
                {
                    tried_queries.push((f_show, f_year));
                }
            }

            for (query_str, q_year) in tried_queries {
                if let Ok((prov, candidates)) =
                    metadata::auto_search_tv_series(&query_str, q_year, &config).await
                {
                    if !candidates.is_empty() {
                        inferred_show = query_str;
                        provider_used = Some(prov);
                        let mut seasons_hint: Vec<u32> =
                            tv_hints.iter().filter_map(|h| h.season).collect();
                        if let Some(ds) = def_season {
                            seasons_hint.push(ds);
                        }
                        let (chosen, reordered, ep_prov, eps) = select_best_show_with_episodes(
                            candidates,
                            &tv_hints,
                            &seasons_hint,
                            &config,
                        )
                        .await;
                        if let Some(p) = ep_prov {
                            if !eps.is_empty() {
                                provider_used = Some(p);
                                episodes_catalog = eps;
                            }
                        }
                        matched_show = chosen;
                        show_candidates = reordered;
                        break;
                    }
                }
            }
        }
    }

    // 3. Assemble TV + Movie items cleanly
    let mut files: Vec<IdentifiedVideoFile> = Vec::with_capacity(entries.len());

    // Build TV rows using fuzzy multi-season episode catalog matcher
    if !tv_indices.is_empty() {
        let tv_entries: Vec<(PathBuf, Option<u64>)> =
            tv_indices.iter().map(|&i| entries[i].clone()).collect();
        let tv_hints_owned: Vec<ParsedVideoHints> =
            tv_indices.iter().map(|&i| hints[i].clone()).collect();
        let tv_durs: Vec<Option<f64>> = tv_indices.iter().map(|&i| durations[i]).collect();

        let show_name_for_strip = matched_show
            .as_ref()
            .map(|s| s.title.as_str())
            .or_else(|| (!inferred_show.is_empty()).then_some(inferred_show.as_str()));

        let tv_files = match_tv_files_to_episodes(
            &tv_entries,
            &tv_hints_owned,
            &tv_durs,
            tv_default_season,
            show_name_for_strip,
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

/// Maps discovered TV files + parsed hints against an official episode catalog
/// using explicit S/E numbers first, then fuzzy multi-season episode-title matching,
/// and sequential fill only for remaining completely unnamed files.
pub fn match_tv_files_to_episodes(
    entries: &[(PathBuf, Option<u64>)],
    hints: &[ParsedVideoHints],
    durations: &[Option<f64>],
    default_season: Option<u32>,
    show_title: Option<&str>,
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
            let se_official = by_se.get(&(s, e)).copied();
            let mut resolved_via_title_override = false;

            // If the filename ALSO carries an episode title (e.g. `se02 episode 2Fire Escape.mp4`),
            // verify it against the catalog entry for (s, e). If (s, e) disagrees with the title
            // (or doesn't exist), and the title strongly matches an official episode elsewhere in
            // the catalog, auto-correct (season, episode) AND stamp the exact official title!
            if let Some(ref th) = hint.episode_title_hint {
                let se_agrees = se_official.is_some_and(|off| {
                    episode_title_match_score(th, show_title, &off.title) >= 0.55
                });
                if !se_agrees {
                    if let Some((_, title_hit)) = match_episode_by_title_scored(
                        th,
                        show_title,
                        Some(s),
                        catalog,
                        &used_episodes,
                        0.78,
                    ) {
                        season = Some(title_hit.season);
                        episode = Some(title_hit.episode);
                        ep_title = Some(title_hit.title.clone());
                        matched_from_catalog = true;
                        used_episodes.insert((title_hit.season, title_hit.episode));
                        resolved_via_title_override = true;
                    }
                }
            }

            if !resolved_via_title_override {
                if let Some(official) = se_official {
                    ep_title = Some(official.title.clone());
                    matched_from_catalog = true;
                }
                used_episodes.insert((s, e));
            }
        } else {
            // Try fuzzy title match against episode_title_hint first, then full stem across ALL seasons
            let matched_ep = hint
                .episode_title_hint
                .as_deref()
                .and_then(|th| {
                    match_episode_by_title(th, show_title, hint.season, catalog, &used_episodes)
                })
                .or_else(|| {
                    path.file_stem().and_then(|s| s.to_str()).and_then(|stem| {
                        match_episode_by_title(
                            stem,
                            show_title,
                            hint.season.or(default_season),
                            catalog,
                            &used_episodes,
                        )
                    })
                });

            if let Some(official) = matched_ep {
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

    #[test]
    fn rescues_intentionally_messed_up_tv_filenames_from_movie_misclassification() {
        // Case 1: Trailing numbers (`Breaking Bad 1.mkv`, `Breaking Bad 2.mkv`) without S01E01
        let entries1 = vec![
            (PathBuf::from("/media/Breaking Bad/Breaking Bad 1.mkv"), None),
            (PathBuf::from("/media/Breaking Bad/Breaking Bad 2.mkv"), None),
            (PathBuf::from("/media/Breaking Bad/Breaking Bad 3.mkv"), None),
        ];
        let mut hints1: Vec<ParsedVideoHints> = entries1
            .iter()
            .map(|(p, _)| parse_video_filename_hints(p))
            .collect();
        refine_hints_with_siblings(&entries1, &mut hints1);
        let (kind1, kinds1) = classify_files_and_folder(
            Some(Path::new("/media/Breaking Bad")),
            &hints1,
            &[None, None, None],
        );
        assert_eq!(kind1, FolderContentKind::TvSeries);
        assert!(kinds1.iter().all(|k| *k == MediaKind::TvEpisode));
        assert_eq!(hints1[0].title_guess.as_deref(), Some("Breaking Bad"));
        assert_eq!(hints1[0].episode, Some(1));
        assert_eq!(hints1[1].episode, Some(2));
        assert_eq!(hints1[2].episode, Some(3));

        // Case 2: Totally scrambled episode-title-only filenames with typos + 45-min TV runtimes
        let entries2 = vec![
            (PathBuf::from("/tv/Breaking Bad/pilott_messy.mkv"), None),
            (PathBuf::from("/tv/Breaking Bad/ozymandius_rip.mkv"), None),
            (PathBuf::from("/tv/Breaking Bad/graymatter.mkv"), None),
        ];
        let mut hints2: Vec<ParsedVideoHints> = entries2
            .iter()
            .map(|(p, _)| parse_video_filename_hints(p))
            .collect();
        refine_hints_with_siblings(&entries2, &mut hints2);
        let durs2 = [Some(2800.0), Some(2850.0), Some(2790.0)];
        let (kind2, kinds2) = classify_files_and_folder(
            Some(Path::new("/tv/Breaking Bad")),
            &hints2,
            &durs2,
        );
        assert_eq!(kind2, FolderContentKind::TvSeries);
        assert!(kinds2.iter().all(|k| *k == MediaKind::TvEpisode));

        let catalog = vec![
            EpisodeInfo { season: 1, episode: 1, title: "Pilot".to_string() },
            EpisodeInfo { season: 1, episode: 5, title: "Gray Matter".to_string() },
            EpisodeInfo { season: 5, episode: 14, title: "Ozymandias".to_string() },
        ];
        let matched = match_tv_files_to_episodes(
            &entries2,
            &hints2,
            &durs2,
            Some(1),
            Some("Breaking Bad"),
            &catalog,
        );
        assert_eq!(matched[0].season, Some(1));
        assert_eq!(matched[0].episode, Some(1));
        assert_eq!(matched[0].episode_title.as_deref(), Some("Pilot"));
        assert_eq!(matched[1].season, Some(5));
        assert_eq!(matched[1].episode, Some(14));
        assert_eq!(matched[1].episode_title.as_deref(), Some("Ozymandias"));
        assert_eq!(matched[2].season, Some(1));
        assert_eq!(matched[2].episode, Some(5));
        assert_eq!(matched[2].episode_title.as_deref(), Some("Gray Matter"));
    }

    #[test]
    fn parses_abbreviated_season_episode_and_glued_digits() {
        // Real-world intentionally messed up filenames!
        let p1 = Path::new("/shows/Mickey Mouse/se02 episode 2Fire Escape.mp4");
        let h1 = parse_video_filename_hints(p1);
        assert!(h1.explicit_tv_marker);
        assert_eq!(h1.season, Some(2));
        assert_eq!(h1.episode, Some(2));
        assert_eq!(h1.title_guess.as_deref(), Some("Mickey Mouse"));
        assert_eq!(h1.episode_title_hint.as_deref(), Some("Fire Escape"));

        let p2 = Path::new("/shows/Mickey Mouse/season 1 epis 4 - New York Weenie.mp4");
        let h2 = parse_video_filename_hints(p2);
        assert!(h2.explicit_tv_marker);
        assert_eq!(h2.season, Some(1));
        assert_eq!(h2.episode, Some(4));
        assert_eq!(h2.title_guess.as_deref(), Some("Mickey Mouse"));
        assert_eq!(h2.episode_title_hint.as_deref(), Some("New York Weenie"));

        let (kind, kinds) = classify_files_and_folder(
            Some(Path::new("/shows/Mickey Mouse")),
            &[h1.clone(), h2.clone()],
            &[None, None],
        );
        assert_eq!(kind, FolderContentKind::TvSeries);
        assert_eq!(kinds, vec![MediaKind::TvEpisode, MediaKind::TvEpisode]);

        // Even if the user put the wrong S/E number on `Fire Escape` (saying se02 episode 2
        // when Fire Escape is actually S01E17 in the catalog), the strong title match auto-fixes it!
        let catalog = vec![
            EpisodeInfo { season: 1, episode: 4, title: "New York Weenie".to_string() },
            EpisodeInfo { season: 1, episode: 17, title: "Fire Escape".to_string() },
            EpisodeInfo { season: 2, episode: 2, title: "Eau de Minnie".to_string() },
        ];
        let matched = match_tv_files_to_episodes(
            &[(p1.to_path_buf(), None), (p2.to_path_buf(), None)],
            &[h1, h2],
            &[None, None],
            Some(1),
            Some("Mickey Mouse"),
            &catalog,
        );
        assert_eq!(matched[0].season, Some(1));
        assert_eq!(matched[0].episode, Some(17));
        assert_eq!(matched[0].episode_title.as_deref(), Some("Fire Escape"));
        assert!(matched[0].matched_from_catalog);
        assert_eq!(matched[1].season, Some(1));
        assert_eq!(matched[1].episode, Some(4));
        assert_eq!(matched[1].episode_title.as_deref(), Some("New York Weenie"));
        assert!(matched[1].matched_from_catalog);
    }
}
