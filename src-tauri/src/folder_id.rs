//! Smart video folder scanner, messy filename/episode parser, and automatic
//! TV series + episode catalog matcher (TVDB v4 -> TMDB -> keyless TVMaze).
//!
//! Designed for batch re-encoding messy TV season folders (e.g., bloated 24 GB MKV
//! remuxes or 15 GB MP4 encodes with inconsistent filenames) so every file receives
//! uniform, server-ready naming (`Plex`, `Jellyfin`, or `Plain`).

use crate::metadata::{
    self, EpisodeInfo, MediaMetadata, MetadataProvider, MetadataProviderConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "webm"];
const MAX_SCAN_DEPTH: usize = 3;

/// Parsed filename + parent-folder hints extracted locally before querying any catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedVideoHints {
    pub show_guess: Option<String>,
    pub year_guess: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_title_hint: Option<String>,
}

/// One discovered local video file enriched with size, parsed/matched season+episode,
/// and official episode title from TVDB/TMDB/TVMaze.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentifiedVideoFile {
    pub path: String,
    pub file_name: String,
    pub file_size_bytes: Option<u64>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_title: Option<String>,
    pub matched_from_catalog: bool,
}

/// Full result returned after scanning a folder (or existing list of files) and
/// automatically identifying the show and every episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderIdentifyResult {
    pub folder_path: Option<String>,
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
    matches!(
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
    )
}

fn parse_sxxeyy_token(upper: &str) -> Option<(u32, u32)> {
    let rest = upper.strip_prefix('S')?;
    let e_pos = rest.find('E')?;
    let s_part = &rest[..e_pos];
    let e_part = &rest[e_pos + 1..];
    if s_part.is_empty() || s_part.len() > 3 || !s_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // Handle multi-episode like S01E01E02 or S01E01-E02 by taking first episode number
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

fn clean_show_tokens(tokens: &[String]) -> (Option<String>, Option<u16>) {
    let mut words = Vec::new();
    let mut year_guess: Option<u16> = None;

    for (idx, tok) in tokens.iter().enumerate() {
        let upper = tok.to_ascii_uppercase();

        // Stop if we hit release/quality/codec junk after capturing show words
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

        // Year token (e.g. 2008 or 2013)
        if tok.len() == 4 {
            if let Ok(yr) = tok.parse::<u16>() {
                if (1920..=2099).contains(&yr) {
                    // Don't treat "1984" or "2020" at index 0 with nothing else as only a year
                    if !words.is_empty() || idx > 0 {
                        year_guess = Some(yr);
                        break;
                    }
                }
            }
        }

        words.push(tok.clone());
    }

    let joined = words.join(" ").trim().to_string();
    ((!joined.is_empty()).then_some(joined), year_guess)
}

fn clean_episode_title_tokens(tokens: &[String]) -> Option<String> {
    let mut words = Vec::new();
    for tok in tokens {
        let upper = tok.to_ascii_uppercase();
        if is_noise_token(&upper) {
            break;
        }
        // Skip audio channel pairs like "5" "1" or standalone resolution/year at tail
        if tok.len() == 4 {
            if let Ok(yr) = tok.parse::<u16>() {
                if (1920..=2099).contains(&yr) && !words.is_empty() {
                    break;
                }
            }
        }
        words.push(tok.clone());
    }
    let joined = words.join(" ").trim().to_string();
    (!joined.is_empty()).then_some(joined)
}

/// Parses messy filenames AND parent folder names into show name, year, season, episode,
/// and episode title hints.
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

    let mut season: Option<u32> = None;
    let mut episode: Option<u32> = None;
    let mut ep_token_start: Option<usize> = None;
    let mut ep_token_end: Option<usize> = None;

    // Pass 1: Look for explicit SxxEyy, NxNN, "Season X Episode Y", or "Sxx Eyy"
    let mut idx = 0;
    while idx < tokens.len() {
        let upper = tokens[idx].to_ascii_uppercase();
        if let Some((s, e)) = parse_sxxeyy_token(&upper) {
            season = Some(s);
            episode = Some(e);
            ep_token_start = Some(idx);
            ep_token_end = Some(idx + 1);
            break;
        }
        if let Some((s, e)) = parse_nxnn_token(&upper) {
            season = Some(s);
            episode = Some(e);
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
                    ep_token_start = Some(idx);
                    ep_token_end = Some(idx + 2);
                    break;
                }
            }
        }
        idx += 1;
    }

    // Pass 2: Look for "Episode N", "Ep N", "E04", "Ep04"
    if episode.is_none() {
        for (i, tok) in tokens.iter().enumerate() {
            let upper = tok.to_ascii_uppercase();
            if let Some(e_val) = parse_episode_only_token(&upper) {
                episode = Some(e_val);
                ep_token_start = Some(i);
                ep_token_end = Some(i + 1);
                break;
            }
            if matches!(upper.as_str(), "EPISODE" | "EP") {
                if let Some(e_val) = tokens.get(i + 1).and_then(|t| t.parse::<u32>().ok()) {
                    episode = Some(e_val);
                    ep_token_start = Some(i);
                    ep_token_end = Some(i + 2);
                    break;
                }
            }
        }
    }

    // Pass 3: Leading episode number like "04 - New York Weenie.mkv" or "01. Pilot.mp4"
    if episode.is_none() && !tokens.is_empty() {
        let first = &tokens[0];
        if first.len() <= 3 && first.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(num) = first.parse::<u32>() {
                if (1..=300).contains(&num) && tokens.len() >= 2 {
                    episode = Some(num);
                    ep_token_start = Some(0);
                    ep_token_end = Some(1);
                }
            }
        }
    }

    // Pass 4: 3-digit scene code like "The Office 201 The Dundies" (avoid years & 480/720/264/265)
    if episode.is_none() {
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
                        ep_token_start = Some(i);
                        ep_token_end = Some(i + 1);
                        break;
                    }
                }
            }
        }
    }

    // Extract show name before episode token, and episode title hint after episode token
    let before_tokens = match ep_token_start {
        Some(start) => &tokens[..start],
        None => &tokens[..],
    };
    let (mut show_guess, mut year_guess) = clean_show_tokens(before_tokens);
    let episode_title_hint = ep_token_end
        .filter(|&end| end < tokens.len())
        .and_then(|end| clean_episode_title_tokens(&tokens[end..]));

    // Inspect parent & grandparent folder names for missing season / show name / year
    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());
    let grandparent_name = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    if season.is_none() {
        if let Some(p) = parent_name {
            season = parse_folder_season_hint(p);
        }
    }

    if show_guess.is_none() || show_guess.as_deref().is_some_and(is_generic_stem) {
        let folder_candidate = parent_name
            .filter(|p| !is_season_only_folder_name(p))
            .or(grandparent_name.filter(|p| !is_season_only_folder_name(p)));

        if let Some(folder_str) = folder_candidate {
            let folder_stripped = strip_bracketed_tags(folder_str);
            let folder_tokens: Vec<String> = folder_stripped
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            let (folder_show, folder_year) = clean_show_tokens(&folder_tokens);
            if folder_show.is_some() {
                show_guess = folder_show;
            }
            if year_guess.is_none() {
                year_guess = folder_year;
            }
        }
    }

    ParsedVideoHints {
        show_guess,
        year_guess,
        season,
        episode,
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
    lower.is_empty()
        || lower.starts_with("title")
        || lower.starts_with("video")
        || lower.starts_with("track")
        || lower.starts_with("b1_")
        || lower.starts_with("t0")
        || lower.chars().all(|c| c.is_ascii_digit())
}

fn normalize_title_words(raw: &str) -> HashSet<String> {
    raw.split(|c: char| !c.is_ascii_alphanumeric())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| w.len() >= 2 && !matches!(w.as_str(), "the" | "and" | "of" | "to" | "in" | "at"))
        .collect()
}

/// Fuzzy-matches a messy filename's episode title hint against the fetched episode catalog.
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

/// Picks the strongest consensus show title and year across the folder name + all parsed files.
fn infer_consensus_show(
    folder_path: Option<&Path>,
    hints: &[ParsedVideoHints],
) -> (String, Option<u16>, Option<u32>) {
    let mut counts: HashMap<String, (usize, String)> = HashMap::new();
    let mut year_counts: HashMap<u16, usize> = HashMap::new();
    let mut season_counts: HashMap<u32, usize> = HashMap::new();

    for h in hints {
        if let Some(ref show) = h.show_guess {
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

    // Also inspect folder name + parent folder name if files had messy/empty prefixes
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
                let stripped = strip_bracketed_tags(f_name);
                let toks: Vec<String> = stripped
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect();
                let (f_show, f_year) = clean_show_tokens(&toks);
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

/// Scans a folder of video files and automatically identifies the TV show, external IDs
/// (TVDB/IMDb/TMDB), season/episode numbers, and official episode titles.
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

/// Runs automatic show + episode identification over an already-selected list of local video files.
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

    let (inferred_show, inferred_year, default_season) = infer_consensus_show(folder_path, &hints);
    let total_size_bytes: u64 = entries.iter().filter_map(|(_, sz)| *sz).sum();

    let mut provider_used: Option<MetadataProvider> = None;
    let mut show_candidates: Vec<MediaMetadata> = Vec::new();
    let mut matched_show: Option<MediaMetadata> = None;
    let mut episodes_catalog: Vec<EpisodeInfo> = Vec::new();

    if !inferred_show.trim().is_empty() {
        if let Ok((prov, candidates)) =
            metadata::auto_search_tv_series(&inferred_show, inferred_year, &config).await
        {
            provider_used = Some(prov);
            if let Some(top) = candidates.first().cloned() {
                let mut seasons_hint: Vec<u32> = hints.iter().filter_map(|h| h.season).collect();
                if let Some(ds) = default_season {
                    seasons_hint.push(ds);
                }
                if let Ok((ep_prov, eps)) =
                    metadata::fetch_series_episodes(&top.title, &top.ids, &seasons_hint, &config)
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

    let files = match_files_to_episodes(&entries, &hints, default_season, &episodes_catalog);

    let catalog_matched_count = files.iter().filter(|f| f.matched_from_catalog).count();
    let status_message = match (&matched_show, provider_used) {
        (Some(show), Some(prov)) => {
            let prov_label = match prov {
                MetadataProvider::Tvdb => "TVDB",
                MetadataProvider::Tmdb => "TMDB",
                MetadataProvider::Tvmaze => "TVMaze",
                MetadataProvider::Imdb => "IMDb",
            };
            format!(
                "Matched \"{}\" via {prov_label} · {catalog_matched_count}/{} episodes matched",
                show.title,
                files.len()
            )
        }
        _ => format!(
            "Loaded {} video files (parsed filenames locally — search above to match a show).",
            files.len()
        ),
    };

    Ok(FolderIdentifyResult {
        folder_path: folder_path.map(|p| p.to_string_lossy().into_owned()),
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

/// Maps discovered files + parsed hints against an official episode catalog and sorts
/// the resulting rows naturally by `(season, episode, file_name)`.
pub fn match_files_to_episodes(
    entries: &[(PathBuf, Option<u64>)],
    hints: &[ParsedVideoHints],
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

    // First pass: resolve files that have explicit episode numbers or fuzzy title matches
    for ((path, size), hint) in entries.iter().zip(hints.iter()) {
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
                used_episodes.insert((s, e));
            } else {
                used_episodes.insert((s, e));
            }
        } else if let Some(ref title_hint) = hint.episode_title_hint {
            if let Some(official) = match_episode_by_title(title_hint, hint.season, catalog) {
                season = Some(official.season);
                episode = Some(official.episode);
                ep_title = Some(official.title.clone());
                matched_from_catalog = true;
                used_episodes.insert((official.season, official.episode));
            }
        } else if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            // Try matching whole cleaned filename stem against episode titles
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
            season,
            episode,
            episode_title: ep_title,
            matched_from_catalog,
        });
    }

    // Second pass: assign sequential episode slots for completely unnumbered files (e.g. title_t00.mkv)
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

    results.sort_by(|a, b| {
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

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_messy_tv_filenames_and_folder_context() {
        let p1 = Path::new(
            "/media/Breaking.Bad.S01.1080p.REMUX/Breaking.Bad.S01E01.Pilot.1080p.BluRay.REMUX.AVC.DTS-HD.MA.5.1.mkv",
        );
        let h1 = parse_video_filename_hints(p1);
        assert_eq!(h1.show_guess.as_deref(), Some("Breaking Bad"));
        assert_eq!(h1.season, Some(1));
        assert_eq!(h1.episode, Some(1));
        assert_eq!(h1.episode_title_hint.as_deref(), Some("Pilot"));

        let p2 = Path::new("/media/Breaking Bad (2008)/Season 01/1x02_Cats_in_the_Bag_x264.mp4");
        let h2 = parse_video_filename_hints(p2);
        assert_eq!(h2.show_guess.as_deref(), Some("Breaking Bad"));
        assert_eq!(h2.year_guess, Some(2008));
        assert_eq!(h2.season, Some(1));
        assert_eq!(h2.episode, Some(2));
        assert_eq!(h2.episode_title_hint.as_deref(), Some("Cats in the Bag"));

        let p3 = Path::new("/media/Mickey Mouse (2013)/Season 1/Mickey Mouse S01E04 - New York Weenie.mp4");
        let h3 = parse_video_filename_hints(p3);
        assert_eq!(h3.show_guess.as_deref(), Some("Mickey Mouse"));
        assert_eq!(h3.season, Some(1));
        assert_eq!(h3.episode, Some(4));
        assert_eq!(h3.episode_title_hint.as_deref(), Some("New York Weenie"));

        let p4 = Path::new("/media/The Office (2005)/Season 02/03 - Office Olympics.mkv");
        let h4 = parse_video_filename_hints(p4);
        assert_eq!(h4.show_guess.as_deref(), Some("The Office"));
        assert_eq!(h4.year_guess, Some(2005));
        assert_eq!(h4.season, Some(2));
        assert_eq!(h4.episode, Some(3));
        assert_eq!(h4.episode_title_hint.as_deref(), Some("Office Olympics"));
    }

    #[test]
    fn matches_messy_season_folder_against_official_episode_catalog() {
        let entries = vec![
            (
                PathBuf::from("/tv/Breaking Bad (2008)/Season 1/title_t00.mkv"),
                Some(24_000_000_000),
            ),
            (
                PathBuf::from("/tv/Breaking Bad (2008)/Season 1/Breaking.Bad.1x03.1080p.REMUX.mkv"),
                Some(22_500_000_000),
            ),
            (
                PathBuf::from("/tv/Breaking Bad (2008)/Season 1/Cats in the Bag [15GB].mp4"),
                Some(15_200_000_000),
            ),
        ];
        let hints: Vec<ParsedVideoHints> = entries
            .iter()
            .map(|(p, _)| parse_video_filename_hints(p))
            .collect();

        let catalog = vec![
            EpisodeInfo {
                season: 1,
                episode: 1,
                title: "Pilot".to_string(),
            },
            EpisodeInfo {
                season: 1,
                episode: 2,
                title: "Cat's in the Bag...".to_string(),
            },
            EpisodeInfo {
                season: 1,
                episode: 3,
                title: "...And the Bag's in the River".to_string(),
            },
        ];

        let matched = match_files_to_episodes(&entries, &hints, Some(1), &catalog);
        assert_eq!(matched.len(), 3);
        // Sorted cleanly S01E01 -> S01E02 -> S01E03
        assert_eq!(matched[0].episode, Some(1));
        assert_eq!(matched[0].episode_title.as_deref(), Some("Pilot"));
        assert!(matched[0].matched_from_catalog);

        assert_eq!(matched[1].episode, Some(2));
        assert_eq!(
            matched[1].episode_title.as_deref(),
            Some("Cat's in the Bag...")
        );
        assert!(matched[1].matched_from_catalog);

        assert_eq!(matched[2].episode, Some(3));
        assert_eq!(
            matched[2].episode_title.as_deref(),
            Some("...And the Bag's in the River")
        );
        assert!(matched[2].matched_from_catalog);
    }
}
