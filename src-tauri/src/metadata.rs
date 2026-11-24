//! Media titling, live IMDb (OMDb) / TVDB v4 / TMDB / TVMaze HTTP search,
//! identifier normalization, disc-label cleanup, and Plex / Jellyfin / Plain formatting.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "RipDaddy/0.1 (https://github.com/ripdaddy)";
const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p/w342";

/// Distinguishes feature films from episodic television content for naming rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Movie,
    TvEpisode,
}

/// External metadata catalog providers supported by RipDaddy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataProvider {
    Imdb,
    Tvdb,
    Tmdb,
    Tvmaze,
}

/// Output directory and filename tagging style for media servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NamingConvention {
    /// e.g. `The Matrix (1999) {imdb-tt0133093}/The Matrix (1999) {imdb-tt0133093}.mkv`
    Plex,
    /// e.g. `The Matrix (1999) [imdbid-tt0133093]/The Matrix (1999) [imdbid-tt0133093].mkv`
    Jellyfin,
    /// Flat clean filename without external ID tags: `The Matrix (1999).mkv`
    Plain,
}

/// Controls how RipDaddy organizes encoded output files into Show / Season subfolders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderOrganization {
    /// Creates `<Show (Year) [tag]>/Season 01/<filename>` (or `<Show (Year)>/...` for Plain).
    ShowAndSeason,
    /// Creates `Season 01/<filename>` directly inside the chosen output directory.
    SeasonOnly,
    /// Writes `<filename>` flat into the chosen output directory without creating subfolders.
    Flat,
}

impl FolderOrganization {
    /// Default folder organization matching a naming convention when none is explicitly chosen.
    pub fn default_for(convention: NamingConvention) -> Self {
        match convention {
            NamingConvention::Plex | NamingConvention::Jellyfin => Self::ShowAndSeason,
            NamingConvention::Plain => Self::Flat,
        }
    }
}

/// External catalog IDs attached to a movie or series.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIds {
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub tmdb_id: Option<String>,
}

/// Official episode entry returned by TVDB, TMDB, or keyless TVMaze.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeInfo {
    pub season: u32,
    pub episode: u32,
    pub title: String,
}

impl ExternalIds {
    /// Normalizes raw IDs or pasted IMDb/TVDB/TMDB URLs into canonical ID tokens.
    pub fn normalized(&self) -> Self {
        Self {
            imdb_id: self.imdb_id.as_deref().and_then(normalize_imdb_id),
            tvdb_id: self.tvdb_id.as_deref().and_then(normalize_numeric_id),
            tmdb_id: self.tmdb_id.as_deref().and_then(normalize_numeric_id),
        }
    }
}

/// Media titling payload attached to a title, encode job, or lookup search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    pub kind: MediaKind,
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub episode_title: Option<String>,
    pub ids: ExternalIds,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
}

/// Query parameters for searching an external metadata provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSearchQuery {
    pub query: String,
    pub year: Option<u16>,
    pub kind: MediaKind,
    pub provider: MetadataProvider,
}

/// User-supplied API keys stored locally via `tauri-plugin-store`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataProviderConfig {
    pub omdb_api_key: Option<String>,
    pub tvdb_api_key: Option<String>,
    pub tmdb_api_key: Option<String>,
}

/// Result produced by cleaning a raw optical disc volume label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscLabelGuess {
    pub cleaned_title: String,
    pub guessed_year: Option<u16>,
    pub guessed_season: Option<u32>,
}

/// Errors produced by metadata validation or live HTTP provider lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataError {
    InvalidInput(String),
    MissingApiKey(MetadataProvider),
    Network(String),
    ProviderError(String),
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetadataError::InvalidInput(msg) => write!(f, "{msg}"),
            MetadataError::MissingApiKey(provider) => match provider {
                MetadataProvider::Imdb => write!(
                    f,
                    "Add your OMDb API key in Settings → Metadata & Naming (or use Free TVMaze for TV shows)."
                ),
                MetadataProvider::Tvdb => write!(
                    f,
                    "Add your TVDB v4 API key in Settings → Metadata & Naming (or use Free TVMaze for TV shows)."
                ),
                MetadataProvider::Tmdb => write!(
                    f,
                    "Add your TMDB API key or Read Access Token in Settings → Metadata & Naming."
                ),
                MetadataProvider::Tvmaze => write!(f, "TVMaze does not require an API key."),
            },
            MetadataError::Network(msg) => write!(f, "Network error: {msg}"),
            MetadataError::ProviderError(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for MetadataError {}

/// Extracts an IMDb `tt...` identifier from a bare ID or full IMDb title URL.
pub fn normalize_imdb_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    for (pos, _) in lower.match_indices("tt") {
        let digits: String = lower[pos + 2..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return Some(format!("tt{digits}"));
        }
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("tt{trimmed}"));
    }
    None
}

/// Extracts a numeric TVDB or TMDB identifier from a bare ID or pasted URL.
pub fn normalize_numeric_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }
    let mut best: Option<String> = None;
    let mut current = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if best.as_ref().map_or(true, |b| current.len() > b.len()) {
                best = Some(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if !current.is_empty() && best.as_ref().map_or(true, |b| current.len() > b.len()) {
        best = Some(current);
    }
    best
}

/// Cleans an optical disc volume label (e.g. `THE_MATRIX_1999_DISC_1` or `BREAKING_BAD_S01_D2`)
/// into a human-friendly title guess with optional year and season hints.
pub fn clean_disc_label(raw_label: &str) -> DiscLabelGuess {
    let tokens: Vec<String> = raw_label
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .map(|tok| tok.to_string())
        .collect();

    let mut words = Vec::new();
    let mut guessed_year: Option<u16> = None;
    let mut guessed_season: Option<u32> = None;
    let mut skip_next = false;

    for (idx, token) in tokens.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }

        let upper = token.to_ascii_uppercase();

        if upper == "DISC" || upper == "DISK" || upper == "SEASON" {
            if upper == "SEASON" {
                if let Some(next_tok) = tokens.get(idx + 1) {
                    if let Ok(season_num) = next_tok.parse::<u32>() {
                        guessed_season = Some(season_num);
                        skip_next = true;
                        continue;
                    }
                }
            } else if let Some(next_tok) = tokens.get(idx + 1) {
                if next_tok.chars().all(|c| c.is_ascii_digit()) {
                    skip_next = true;
                }
            }
            continue;
        }

        if let Some(rest) = upper.strip_prefix('S') {
            if !rest.is_empty() && rest.len() <= 2 && rest.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(season_num) = rest.parse::<u32>() {
                    guessed_season = Some(season_num);
                    continue;
                }
            }
        }

        if let Some(rest) = upper.strip_prefix('D') {
            if !rest.is_empty() && rest.len() <= 2 && rest.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
        }

        if matches!(
            upper.as_str(),
            "BD" | "BLURAY" | "UHD" | "4K" | "DVD" | "BONUS" | "EXTRAS" | "REGIONA" | "REGIONB"
        ) {
            continue;
        }

        if token.len() == 4 {
            if let Ok(year) = token.parse::<u16>() {
                if (1900..=2099).contains(&year) {
                    guessed_year = Some(year);
                    continue;
                }
            }
        }

        words.push(title_case_word(token));
    }

    DiscLabelGuess {
        cleaned_title: words.join(" "),
        guessed_year,
        guessed_season,
    }
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.extend(first.to_uppercase());
    out.push_str(&chars.as_str().to_ascii_lowercase());
    out
}

/// Removes filesystem-unsafe characters across Windows, macOS, and Linux.
pub fn sanitize_filename(input: &str) -> String {
    let mut cleaned = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            cleaned.push(' ');
        } else {
            cleaned.push(ch);
        }
    }
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches('.').trim().to_string()
}

/// Formats the highest-priority external ID tag according to media type and server convention.
///
/// - Movies prefer IMDb -> TMDB -> TVDB.
/// - TV episodes prefer TVDB -> IMDb -> TMDB.
pub fn format_provider_tag(
    kind: MediaKind,
    ids: &ExternalIds,
    convention: NamingConvention,
) -> Option<String> {
    if convention == NamingConvention::Plain {
        return None;
    }

    let normalized = ids.normalized();
    let chosen: Option<(&str, &str, String)> = match kind {
        MediaKind::Movie => normalized
            .imdb_id
            .map(|id| ("imdb", "imdbid", id))
            .or_else(|| normalized.tmdb_id.map(|id| ("tmdb", "tmdbid", id)))
            .or_else(|| normalized.tvdb_id.map(|id| ("tvdb", "tvdbid", id))),
        MediaKind::TvEpisode => normalized
            .tvdb_id
            .map(|id| ("tvdb", "tvdbid", id))
            .or_else(|| normalized.imdb_id.map(|id| ("imdb", "imdbid", id)))
            .or_else(|| normalized.tmdb_id.map(|id| ("tmdb", "tmdbid", id))),
    };

    chosen.map(|(plex_key, jellyfin_key, value)| match convention {
        NamingConvention::Plex => format!("{{{plex_key}-{value}}}"),
        NamingConvention::Jellyfin => format!("[{jellyfin_key}-{value}]"),
        NamingConvention::Plain => String::new(),
    })
}

/// Builds a media-server-ready relative output path (folder + filename) using the
/// convention's default folder organization.
pub fn format_output_relative_path(
    meta: &MediaMetadata,
    convention: NamingConvention,
    extension: &str,
) -> String {
    format_output_relative_path_with_org(
        meta,
        convention,
        extension,
        FolderOrganization::default_for(convention),
    )
}

/// Builds a media-server-ready relative output path with explicit Show / Season / Flat
/// folder organization so multi-season folders can be split into `Season 01`, `Season 02`,
/// etc., or kept flat when the target folder is already organized.
pub fn format_output_relative_path_with_org(
    meta: &MediaMetadata,
    convention: NamingConvention,
    extension: &str,
    organization: FolderOrganization,
) -> String {
    let ext = extension.trim().trim_start_matches('.');
    let ext = if ext.is_empty() { "mkv" } else { ext };

    let title = sanitize_filename(&meta.title);
    let title = if title.is_empty() {
        "Untitled".to_string()
    } else {
        title
    };

    let base_name = match meta.year {
        Some(year) => format!("{title} ({year})"),
        None => title,
    };

    let tagged_base = match format_provider_tag(meta.kind, &meta.ids, convention) {
        Some(tag) => format!("{base_name} {tag}"),
        None => base_name.clone(),
    };

    match meta.kind {
        MediaKind::Movie => match organization {
            FolderOrganization::Flat => match convention {
                NamingConvention::Plain => format!("{base_name}.{ext}"),
                NamingConvention::Plex | NamingConvention::Jellyfin => {
                    format!("{tagged_base}.{ext}")
                }
            },
            FolderOrganization::ShowAndSeason | FolderOrganization::SeasonOnly => {
                match convention {
                    NamingConvention::Plain => format!("{base_name}/{base_name}.{ext}"),
                    NamingConvention::Plex | NamingConvention::Jellyfin => {
                        format!("{tagged_base}/{tagged_base}.{ext}")
                    }
                }
            }
        },
        MediaKind::TvEpisode => {
            let season = meta.season.unwrap_or(1);
            let episode = meta.episode.unwrap_or(1);
            let ep_code = format!("S{season:02}E{episode:02}");
            let ep_title = meta
                .episode_title
                .as_deref()
                .map(sanitize_filename)
                .filter(|s| !s.is_empty());

            let filename = match ep_title {
                Some(ep) => format!("{base_name} - {ep_code} - {ep}.{ext}"),
                None => format!("{base_name} - {ep_code}.{ext}"),
            };

            match organization {
                FolderOrganization::Flat => filename,
                FolderOrganization::SeasonOnly => format!("Season {season:02}/{filename}"),
                FolderOrganization::ShowAndSeason => {
                    format!("{tagged_base}/Season {season:02}/{filename}")
                }
            }
        }
    }
}

fn build_http_client() -> Result<reqwest::Client, MetadataError> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|err| MetadataError::Network(err.to_string()))
}

fn parse_year_prefix(raw: &str) -> Option<u16> {
    let digits: String = raw
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() >= 4 {
        let year = digits[..4].parse::<u16>().ok()?;
        if (1880..=2100).contains(&year) {
            return Some(year);
        }
    }
    None
}

fn strip_html_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty_url(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("N/A"))
        .map(str::to_string)
}

fn value_as_string(val: &Value) -> Option<String> {
    match val {
        Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Live HTTP search across IMDb (OMDb), TVDB v4, TMDB v3/v4, or keyless TVMaze.
pub async fn search_metadata(
    query: MetadataSearchQuery,
    config: MetadataProviderConfig,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    let trimmed_query = query.query.trim();
    if trimmed_query.is_empty() {
        return Err(MetadataError::InvalidInput(
            "Enter a movie or TV series title to look up.".to_string(),
        ));
    }

    let client = build_http_client()?;

    match query.provider {
        MetadataProvider::Imdb => {
            if let Some(api_key) = config
                .omdb_api_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
            {
                search_omdb(&client, trimmed_query, query.year, query.kind, api_key).await
            } else if query.kind == MediaKind::Movie {
                let res = search_imdb_public_suggestions(&client, trimmed_query, query.year).await?;
                if !res.is_empty() {
                    Ok(res)
                } else {
                    search_itunes_movies(&client, trimmed_query, query.year).await
                }
            } else {
                Err(MetadataError::MissingApiKey(MetadataProvider::Imdb))
            }
        }
        MetadataProvider::Tvdb => {
            let api_key = config
                .tvdb_api_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .ok_or(MetadataError::MissingApiKey(MetadataProvider::Tvdb))?;
            search_tvdb(&client, trimmed_query, query.year, query.kind, api_key).await
        }
        MetadataProvider::Tmdb => {
            let api_key = config
                .tmdb_api_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .ok_or(MetadataError::MissingApiKey(MetadataProvider::Tmdb))?;
            search_tmdb(&client, trimmed_query, query.year, query.kind, api_key).await
        }
        MetadataProvider::Tvmaze => search_tvmaze(&client, trimmed_query).await,
    }
}

/// Fetches IMDb and TVDB external IDs for a selected TMDB movie or TV series.
pub async fn fetch_tmdb_external_ids(
    kind: MediaKind,
    tmdb_id: &str,
    config: MetadataProviderConfig,
) -> Result<ExternalIds, MetadataError> {
    let id = normalize_numeric_id(tmdb_id)
        .ok_or_else(|| MetadataError::InvalidInput("Invalid TMDB ID.".to_string()))?;
    let api_key = config
        .tmdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or(MetadataError::MissingApiKey(MetadataProvider::Tmdb))?;

    let client = build_http_client()?;
    let segment = match kind {
        MediaKind::Movie => "movie",
        MediaKind::TvEpisode => "tv",
    };
    let url = format!("https://api.themoviedb.org/3/{segment}/{id}/external_ids");

    let mut req = client.get(&url);
    if is_tmdb_bearer_token(api_key) {
        req = req.bearer_auth(api_key);
    } else {
        req = req.query(&[("api_key", api_key)]);
    }

    let resp = req
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;
    if !resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "TMDB external IDs request failed with HTTP {}.",
            resp.status()
        )));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;

    Ok(ExternalIds {
        imdb_id: json
            .get("imdb_id")
            .and_then(value_as_string)
            .and_then(|s| normalize_imdb_id(&s)),
        tvdb_id: json
            .get("tvdb_id")
            .and_then(value_as_string)
            .and_then(|s| normalize_numeric_id(&s)),
        tmdb_id: Some(id),
    })
}

fn is_tmdb_bearer_token(key: &str) -> bool {
    key.starts_with("eyJ") || key.len() > 40
}

async fn search_omdb(
    client: &reqwest::Client,
    query: &str,
    year: Option<u16>,
    kind: MediaKind,
    api_key: &str,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    let mut params: Vec<(&str, String)> = vec![("apikey", api_key.to_string())];

    if let Some(imdb_id) = normalize_imdb_id(query) {
        params.push(("i", imdb_id));
    } else {
        params.push(("s", query.to_string()));
        params.push((
            "type",
            match kind {
                MediaKind::Movie => "movie".to_string(),
                MediaKind::TvEpisode => "series".to_string(),
            },
        ));
        if let Some(y) = year {
            params.push(("y", y.to_string()));
        }
    }

    let resp = client
        .get("https://www.omdbapi.com/")
        .query(&params)
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;

    if !resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "OMDb API returned HTTP {}.",
            resp.status()
        )));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;

    parse_omdb_response(&json, kind)
}

fn parse_omdb_response(
    json: &Value,
    default_kind: MediaKind,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    if json
        .get("Response")
        .and_then(Value::as_str)
        .is_some_and(|r| r.eq_ignore_ascii_case("False"))
    {
        let err_msg = json
            .get("Error")
            .and_then(Value::as_str)
            .unwrap_or("No OMDb results found.");
        return Err(MetadataError::ProviderError(err_msg.to_string()));
    }

    let map_item = |item: &Value| -> Option<MediaMetadata> {
        let title = item.get("Title").and_then(Value::as_str)?.trim().to_string();
        if title.is_empty() {
            return None;
        }
        let item_kind = match item.get("Type").and_then(Value::as_str) {
            Some("series") | Some("episode") => MediaKind::TvEpisode,
            Some("movie") => MediaKind::Movie,
            _ => default_kind,
        };
        let year = item
            .get("Year")
            .and_then(Value::as_str)
            .and_then(parse_year_prefix);
        let imdb_id = item
            .get("imdbID")
            .and_then(Value::as_str)
            .and_then(normalize_imdb_id);
        let poster_url = non_empty_url(item.get("Poster").and_then(Value::as_str));
        let overview = non_empty_url(item.get("Plot").and_then(Value::as_str));

        Some(MediaMetadata {
            kind: item_kind,
            title,
            year,
            season: None,
            episode: None,
            episode_title: None,
            ids: ExternalIds {
                imdb_id,
                tvdb_id: None,
                tmdb_id: None,
            },
            overview,
            poster_url,
        })
    };

    if let Some(items) = json.get("Search").and_then(Value::as_array) {
        return Ok(items.iter().filter_map(map_item).take(10).collect());
    }

    if let Some(single) = map_item(json) {
        return Ok(vec![single]);
    }

    Ok(Vec::new())
}

async fn search_tvdb(
    client: &reqwest::Client,
    query: &str,
    year: Option<u16>,
    kind: MediaKind,
    api_key: &str,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    let login_resp = client
        .post("https://api4.thetvdb.com/v4/login")
        .json(&serde_json::json!({ "apikey": api_key }))
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;

    if !login_resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "TVDB login failed (HTTP {}). Verify your TVDB v4 API key in Settings.",
            login_resp.status()
        )));
    }

    let login_json: Value = login_resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;

    let token = login_json
        .get("data")
        .and_then(|d| d.get("token"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            MetadataError::ProviderError("TVDB login response did not include a token.".to_string())
        })?;

    let mut params: Vec<(&str, String)> = vec![
        ("query", query.to_string()),
        (
            "type",
            match kind {
                MediaKind::Movie => "movie".to_string(),
                MediaKind::TvEpisode => "series".to_string(),
            },
        ),
    ];
    if let Some(y) = year {
        params.push(("year", y.to_string()));
    }

    let search_resp = client
        .get("https://api4.thetvdb.com/v4/search")
        .bearer_auth(token)
        .query(&params)
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;

    if !search_resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "TVDB search failed with HTTP {}.",
            search_resp.status()
        )));
    }

    let search_json: Value = search_resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;

    Ok(parse_tvdb_response(&search_json, kind))
}

fn parse_tvdb_response(json: &Value, default_kind: MediaKind) -> Vec<MediaMetadata> {
    let Some(items) = json.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let title = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string();

            let item_kind = match item.get("type").and_then(Value::as_str) {
                Some("movie") => MediaKind::Movie,
                Some("series") => MediaKind::TvEpisode,
                _ => default_kind,
            };

            let year = item
                .get("year")
                .and_then(value_as_string)
                .and_then(|s| parse_year_prefix(&s));

            let tvdb_id = item
                .get("tvdb_id")
                .and_then(value_as_string)
                .and_then(|s| normalize_numeric_id(&s));

            let mut imdb_id = None;
            let mut tmdb_id = None;
            if let Some(remotes) = item.get("remote_ids").and_then(Value::as_array) {
                for remote in remotes {
                    let source = remote
                        .get("sourceName")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let raw_id = remote.get("id").and_then(value_as_string);
                    if let Some(val) = raw_id {
                        if source.contains("imdb") && imdb_id.is_none() {
                            imdb_id = normalize_imdb_id(&val);
                        } else if source.contains("themoviedb") || source.contains("tmdb") {
                            if tmdb_id.is_none() {
                                tmdb_id = normalize_numeric_id(&val);
                            }
                        }
                    }
                }
            }

            let poster_url = non_empty_url(item.get("image_url").and_then(Value::as_str))
                .or_else(|| non_empty_url(item.get("thumbnail").and_then(Value::as_str)));
            let overview = non_empty_url(item.get("overview").and_then(Value::as_str));

            Some(MediaMetadata {
                kind: item_kind,
                title,
                year,
                season: None,
                episode: None,
                episode_title: None,
                ids: ExternalIds {
                    imdb_id,
                    tvdb_id,
                    tmdb_id,
                },
                overview,
                poster_url,
            })
        })
        .take(10)
        .collect()
}

async fn search_tmdb(
    client: &reqwest::Client,
    query: &str,
    year: Option<u16>,
    kind: MediaKind,
    api_key: &str,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    let endpoint = match kind {
        MediaKind::Movie => "https://api.themoviedb.org/3/search/movie",
        MediaKind::TvEpisode => "https://api.themoviedb.org/3/search/tv",
    };

    let mut params: Vec<(&str, String)> = vec![("query", query.to_string())];
    if let Some(y) = year {
        let year_param = match kind {
            MediaKind::Movie => "year",
            MediaKind::TvEpisode => "first_air_date_year",
        };
        params.push((year_param, y.to_string()));
    }

    let mut req = client.get(endpoint);
    if is_tmdb_bearer_token(api_key) {
        req = req.bearer_auth(api_key);
    } else {
        params.push(("api_key", api_key.to_string()));
    }

    let resp = req
        .query(&params)
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;

    if !resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "TMDB search failed (HTTP {}). Verify your TMDB key/token in Settings.",
            resp.status()
        )));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;

    Ok(parse_tmdb_response(&json, kind))
}

fn parse_tmdb_response(json: &Value, kind: MediaKind) -> Vec<MediaMetadata> {
    let Some(items) = json.get("results").and_then(Value::as_array) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let title = match kind {
                MediaKind::Movie => item.get("title"),
                MediaKind::TvEpisode => item.get("name"),
            }
            .or_else(|| item.get("title"))
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)?
            .trim()
            .to_string();

            if title.is_empty() {
                return None;
            }

            let date_str = match kind {
                MediaKind::Movie => item.get("release_date"),
                MediaKind::TvEpisode => item.get("first_air_date"),
            }
            .and_then(Value::as_str);
            let year = date_str.and_then(parse_year_prefix);

            let tmdb_id = item
                .get("id")
                .and_then(value_as_string)
                .and_then(|s| normalize_numeric_id(&s));

            let poster_url = item
                .get("poster_path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(|path| format!("{TMDB_IMAGE_BASE}{path}"));

            let overview = non_empty_url(item.get("overview").and_then(Value::as_str));

            Some(MediaMetadata {
                kind,
                title,
                year,
                season: None,
                episode: None,
                episode_title: None,
                ids: ExternalIds {
                    imdb_id: None,
                    tvdb_id: None,
                    tmdb_id,
                },
                overview,
                poster_url,
            })
        })
        .take(10)
        .collect()
}

async fn search_tvmaze(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    let resp = client
        .get("https://api.tvmaze.com/search/shows")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;

    if !resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "TVMaze search failed with HTTP {}.",
            resp.status()
        )));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;

    Ok(parse_tvmaze_response(&json))
}

/// Automatically searches the best available metadata service for a TV series
/// (TVDB v4 if key is configured -> TMDB if key is configured -> keyless TVMaze fallback),
/// returning `(provider_used, candidates)`.
pub async fn auto_search_tv_series(
    query: &str,
    year: Option<u16>,
    config: &MetadataProviderConfig,
) -> Result<(MetadataProvider, Vec<MediaMetadata>), MetadataError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(MetadataError::InvalidInput(
            "Could not infer a show title from the selected folder or filenames.".to_string(),
        ));
    }

    let client = build_http_client()?;

    // 1. Prefer TVDB v4 if user configured a TVDB key
    if let Some(tvdb_key) = config
        .tvdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        if let Ok(results) =
            search_tvdb(&client, trimmed, year, MediaKind::TvEpisode, tvdb_key).await
        {
            if !results.is_empty() {
                return Ok((MetadataProvider::Tvdb, results));
            }
        }
    }

    // 2. Try TMDB if user configured a TMDB key/token
    if let Some(tmdb_key) = config
        .tmdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        if let Ok(mut results) =
            search_tmdb(&client, trimmed, year, MediaKind::TvEpisode, tmdb_key).await
        {
            if !results.is_empty() {
                // Hydrate top candidate with TVDB + IMDb external IDs
                if let Some(tmdb_id) = results[0].ids.tmdb_id.clone() {
                    if let Ok(ext) =
                        fetch_tmdb_external_ids(MediaKind::TvEpisode, &tmdb_id, config.clone())
                            .await
                    {
                        results[0].ids = ext;
                    }
                }
                return Ok((MetadataProvider::Tmdb, results));
            }
        }
    }

    // 3. Always-available keyless TVMaze fallback (includes TVDB + IMDb external IDs!)
    let mut tvmaze_results = search_tvmaze(&client, trimmed).await?;
    // Prefer exact/year-matching show at index 0 when year hint is present
    if let Some(target_year) = year {
        if let Some(pos) = tvmaze_results.iter().position(|m| m.year == Some(target_year)) {
            if pos > 0 {
                let matched = tvmaze_results.remove(pos);
                tvmaze_results.insert(0, matched);
            }
        }
    }
    Ok((MetadataProvider::Tvmaze, tvmaze_results))
}

/// Automatically searches the best available metadata service for a feature film:
/// OMDb (IMDb) -> TMDB (with IMDb external_ids hydration) -> TVDB v4 -> free keyless
/// IMDb suggestion / iTunes movie lookup so batch movie folders work even with zero API keys.
pub async fn auto_search_movie(
    query: &str,
    year: Option<u16>,
    config: &MetadataProviderConfig,
) -> Result<(MetadataProvider, Vec<MediaMetadata>), MetadataError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(MetadataError::InvalidInput(
            "Could not infer a movie title from filename.".to_string(),
        ));
    }

    let client = build_http_client()?;

    // 1. OMDb (IMDb) if configured
    if let Some(omdb_key) = config
        .omdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        if let Ok(results) =
            search_omdb(&client, trimmed, year, MediaKind::Movie, omdb_key).await
        {
            if !results.is_empty() {
                return Ok((MetadataProvider::Imdb, results));
            }
        }
    }

    // 2. TMDB if configured (+ hydrate IMDb ID on top match)
    if let Some(tmdb_key) = config
        .tmdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        if let Ok(mut results) =
            search_tmdb(&client, trimmed, year, MediaKind::Movie, tmdb_key).await
        {
            if !results.is_empty() {
                if let Some(tmdb_id) = results[0].ids.tmdb_id.clone() {
                    if let Ok(ext) =
                        fetch_tmdb_external_ids(MediaKind::Movie, &tmdb_id, config.clone()).await
                    {
                        results[0].ids = ext;
                    }
                }
                return Ok((MetadataProvider::Tmdb, results));
            }
        }
    }

    // 3. TVDB v4 movies if configured
    if let Some(tvdb_key) = config
        .tvdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        if let Ok(results) =
            search_tvdb(&client, trimmed, year, MediaKind::Movie, tvdb_key).await
        {
            if !results.is_empty() {
                return Ok((MetadataProvider::Tvdb, results));
            }
        }
    }

    // 4. Keyless IMDb public suggestion endpoint (returns canonical title, year, tt-ID, poster)
    if let Ok(results) = search_imdb_public_suggestions(&client, trimmed, year).await {
        if !results.is_empty() {
            return Ok((MetadataProvider::Imdb, results));
        }
    }

    // 5. Keyless Apple iTunes movie catalog fallback
    let itunes_results = search_itunes_movies(&client, trimmed, year).await?;
    Ok((MetadataProvider::Imdb, itunes_results))
}

fn encode_imdb_suggestion_slug(query: &str) -> Option<(char, String)> {
    let mut slug = String::new();
    let mut prev_underscore = false;
    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !slug.is_empty() && !prev_underscore {
            slug.push('_');
            prev_underscore = true;
        }
    }
    let slug = slug.trim_matches('_').to_string();
    let first = slug.chars().find(|c| c.is_ascii_alphanumeric())?;
    Some((first, slug))
}

async fn search_imdb_public_suggestions(
    client: &reqwest::Client,
    query: &str,
    year: Option<u16>,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    let Some((first, slug)) = encode_imdb_suggestion_slug(query) else {
        return Ok(Vec::new());
    };
    let url = format!("https://v3.sg.media-imdb.com/suggestion/{first}/{slug}.json");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;
    Ok(parse_imdb_suggestion_movies(&json, year))
}

pub fn parse_imdb_suggestion_movies(json: &Value, year_hint: Option<u16>) -> Vec<MediaMetadata> {
    let Some(items) = json.get("d").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<MediaMetadata> = items
        .iter()
        .filter_map(|item| {
            let id_raw = item.get("id").and_then(Value::as_str)?;
            let imdb_id = normalize_imdb_id(id_raw)?;
            let qid = item
                .get("qid")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let q = item
                .get("q")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let is_movie = qid == "movie"
                || qid == "tvmovie"
                || qid == "video"
                || q.contains("feature");
            if !is_movie {
                return None;
            }
            let title = item.get("l").and_then(Value::as_str)?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let year = item
                .get("y")
                .and_then(Value::as_u64)
                .and_then(|y| u16::try_from(y).ok());
            let poster_url = item
                .get("i")
                .and_then(|img| img.get("imageUrl"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(MediaMetadata {
                kind: MediaKind::Movie,
                title,
                year,
                season: None,
                episode: None,
                episode_title: None,
                ids: ExternalIds {
                    imdb_id: Some(imdb_id),
                    tvdb_id: None,
                    tmdb_id: None,
                },
                overview: None,
                poster_url,
            })
        })
        .take(8)
        .collect();

    if let Some(target_year) = year_hint {
        if let Some(pos) = out.iter().position(|m| m.year == Some(target_year)) {
            if pos > 0 {
                let matched = out.remove(pos);
                out.insert(0, matched);
            }
        }
    }
    out
}

async fn search_itunes_movies(
    client: &reqwest::Client,
    query: &str,
    year_hint: Option<u16>,
) -> Result<Vec<MediaMetadata>, MetadataError> {
    let resp = client
        .get("https://itunes.apple.com/search")
        .query(&[
            ("media", "movie"),
            ("entity", "movie"),
            ("limit", "6"),
            ("term", query),
        ])
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;
    let Some(items) = json.get("results").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<MediaMetadata> = items
        .iter()
        .filter_map(|item| {
            let title = item
                .get("trackName")
                .and_then(Value::as_str)?
                .trim()
                .to_string();
            if title.is_empty() {
                return None;
            }
            let year = item
                .get("releaseDate")
                .and_then(Value::as_str)
                .and_then(parse_year_prefix);
            let poster_url = item
                .get("artworkUrl100")
                .and_then(Value::as_str)
                .map(|url| url.replace("100x100bb", "400x600bb"));
            let overview = non_empty_url(item.get("longDescription").and_then(Value::as_str))
                .or_else(|| non_empty_url(item.get("shortDescription").and_then(Value::as_str)));
            Some(MediaMetadata {
                kind: MediaKind::Movie,
                title,
                year,
                season: None,
                episode: None,
                episode_title: None,
                ids: ExternalIds::default(),
                overview,
                poster_url,
            })
        })
        .collect();

    if let Some(target_year) = year_hint {
        if let Some(pos) = out.iter().position(|m| m.year == Some(target_year)) {
            if pos > 0 {
                let matched = out.remove(pos);
                out.insert(0, matched);
            }
        }
    }
    Ok(out)
}

/// Fetches official `(season, episode, title)` records for a TV show using whichever
/// metadata service is available (TVDB v4 -> TMDB -> keyless TVMaze).
pub async fn fetch_series_episodes(
    title: &str,
    ids: &ExternalIds,
    seasons_hint: &[u32],
    config: &MetadataProviderConfig,
) -> Result<(MetadataProvider, Vec<EpisodeInfo>), MetadataError> {
    let client = build_http_client()?;
    let norm = ids.normalized();

    // 1. TVDB v4 official episode list when key + tvdb_id exist
    if let (Some(tvdb_key), Some(tvdb_id)) = (
        config
            .tvdb_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty()),
        norm.tvdb_id.as_deref(),
    ) {
        if let Ok(eps) = fetch_tvdb_episodes(&client, tvdb_id, tvdb_key).await {
            if !eps.is_empty() {
                return Ok((MetadataProvider::Tvdb, eps));
            }
        }
    }

    // 2. TMDB season episodes when key + tmdb_id exist
    if let (Some(tmdb_key), Some(tmdb_id)) = (
        config
            .tmdb_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty()),
        norm.tmdb_id.as_deref(),
    ) {
        let target_seasons: Vec<u32> = if seasons_hint.is_empty() {
            vec![1]
        } else {
            let mut dedup = seasons_hint.to_vec();
            dedup.sort_unstable();
            dedup.dedup();
            dedup.into_iter().take(20).collect()
        };
        if let Ok(eps) = fetch_tmdb_episodes(&client, tmdb_id, &target_seasons, tmdb_key).await {
            if !eps.is_empty() {
                return Ok((MetadataProvider::Tmdb, eps));
            }
        }
    }

    // 3. Keyless TVMaze episode catalog (lookup by TVDB ID -> IMDb ID -> title search)
    let eps = fetch_tvmaze_episodes(&client, title, &norm).await?;
    Ok((MetadataProvider::Tvmaze, eps))
}

async fn fetch_tvdb_episodes(
    client: &reqwest::Client,
    tvdb_id: &str,
    api_key: &str,
) -> Result<Vec<EpisodeInfo>, MetadataError> {
    let login_resp = client
        .post("https://api4.thetvdb.com/v4/login")
        .json(&serde_json::json!({ "apikey": api_key }))
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;
    if !login_resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "TVDB login failed (HTTP {}).",
            login_resp.status()
        )));
    }
    let login_json: Value = login_resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;
    let token = login_json
        .get("data")
        .and_then(|d| d.get("token"))
        .and_then(Value::as_str)
        .ok_or_else(|| MetadataError::ProviderError("TVDB login missing token.".to_string()))?;

    let mut episodes = Vec::new();
    for page in 0..4u32 {
        let url = format!("https://api4.thetvdb.com/v4/series/{tvdb_id}/episodes/default");
        let resp = client
            .get(&url)
            .bearer_auth(token)
            .query(&[("page", page.to_string())])
            .send()
            .await
            .map_err(|err| MetadataError::Network(err.to_string()))?;
        if !resp.status().is_success() {
            break;
        }
        let json: Value = resp
            .json()
            .await
            .map_err(|err| MetadataError::ProviderError(err.to_string()))?;
        let batch = parse_tvdb_episodes_response(&json);
        if batch.is_empty() {
            break;
        }
        episodes.extend(batch);
        let has_next = json
            .get("links")
            .and_then(|l| l.get("next"))
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        if !has_next {
            break;
        }
    }

    Ok(episodes)
}

pub fn parse_tvdb_episodes_response(json: &Value) -> Vec<EpisodeInfo> {
    let Some(items) = json
        .get("data")
        .and_then(|d| d.get("episodes"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|ep| {
            let season = ep.get("seasonNumber").and_then(Value::as_u64)? as u32;
            let number = ep.get("number").and_then(Value::as_u64)? as u32;
            let title = ep
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string();
            Some(EpisodeInfo {
                season,
                episode: number,
                title,
            })
        })
        .collect()
}

async fn fetch_tmdb_episodes(
    client: &reqwest::Client,
    tmdb_id: &str,
    seasons: &[u32],
    api_key: &str,
) -> Result<Vec<EpisodeInfo>, MetadataError> {
    let mut episodes = Vec::new();
    for &season_num in seasons {
        let url = format!("https://api.themoviedb.org/3/tv/{tmdb_id}/season/{season_num}");
        let mut req = client.get(&url);
        if is_tmdb_bearer_token(api_key) {
            req = req.bearer_auth(api_key);
        } else {
            req = req.query(&[("api_key", api_key)]);
        }
        let Ok(resp) = req.send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(json) = resp.json::<Value>().await else {
            continue;
        };
        if let Some(items) = json.get("episodes").and_then(Value::as_array) {
            for ep in items {
                let s = ep
                    .get("season_number")
                    .and_then(Value::as_u64)
                    .unwrap_or(season_num as u64) as u32;
                let Some(num) = ep.get("episode_number").and_then(Value::as_u64) else {
                    continue;
                };
                let Some(name) = ep
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                else {
                    continue;
                };
                episodes.push(EpisodeInfo {
                    season: s,
                    episode: num as u32,
                    title: name.to_string(),
                });
            }
        }
    }
    Ok(episodes)
}

async fn fetch_tvmaze_episodes(
    client: &reqwest::Client,
    title: &str,
    ids: &ExternalIds,
) -> Result<Vec<EpisodeInfo>, MetadataError> {
    let mut tvmaze_show_id: Option<u64> = None;

    if let Some(ref tvdb_id) = ids.tvdb_id {
        if let Ok(resp) = client
            .get("https://api.tvmaze.com/lookup/shows")
            .query(&[("thetvdb", tvdb_id.as_str())])
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<Value>().await {
                    tvmaze_show_id = json.get("id").and_then(Value::as_u64);
                }
            }
        }
    }

    if tvmaze_show_id.is_none() {
        if let Some(ref imdb_id) = ids.imdb_id {
            if let Ok(resp) = client
                .get("https://api.tvmaze.com/lookup/shows")
                .query(&[("imdb", imdb_id.as_str())])
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(json) = resp.json::<Value>().await {
                        tvmaze_show_id = json.get("id").and_then(Value::as_u64);
                    }
                }
            }
        }
    }

    if tvmaze_show_id.is_none() && !title.trim().is_empty() {
        if let Ok(resp) = client
            .get("https://api.tvmaze.com/singlesearch/shows")
            .query(&[("q", title.trim())])
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<Value>().await {
                    tvmaze_show_id = json.get("id").and_then(Value::as_u64);
                }
            }
        }
    }

    let Some(show_id) = tvmaze_show_id else {
        return Ok(Vec::new());
    };

    let url = format!("https://api.tvmaze.com/shows/{show_id}/episodes");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|err| MetadataError::Network(err.to_string()))?;
    if !resp.status().is_success() {
        return Err(MetadataError::ProviderError(format!(
            "TVMaze episodes lookup failed (HTTP {}).",
            resp.status()
        )));
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|err| MetadataError::ProviderError(err.to_string()))?;
    Ok(parse_tvmaze_episodes_response(&json))
}

pub fn parse_tvmaze_episodes_response(json: &Value) -> Vec<EpisodeInfo> {
    let Some(items) = json.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|ep| {
            let season = ep.get("season").and_then(Value::as_u64)? as u32;
            let number = ep.get("number").and_then(Value::as_u64)? as u32;
            let title = ep
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string();
            Some(EpisodeInfo {
                season,
                episode: number,
                title,
            })
        })
        .collect()
}

fn parse_tvmaze_response(json: &Value) -> Vec<MediaMetadata> {
    let Some(items) = json.as_array() else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|entry| {
            let show = entry.get("show")?;
            let title = show.get("name").and_then(Value::as_str)?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let year = show
                .get("premiered")
                .and_then(Value::as_str)
                .and_then(parse_year_prefix);

            let externals = show.get("externals");
            let imdb_id = externals
                .and_then(|e| e.get("imdb"))
                .and_then(value_as_string)
                .and_then(|s| normalize_imdb_id(&s));
            let tvdb_id = externals
                .and_then(|e| e.get("thetvdb"))
                .and_then(value_as_string)
                .and_then(|s| normalize_numeric_id(&s));

            let poster_url = show
                .get("image")
                .and_then(|img| img.get("medium").or_else(|| img.get("original")))
                .and_then(Value::as_str)
                .map(str::to_string);

            let overview = show
                .get("summary")
                .and_then(Value::as_str)
                .map(strip_html_tags)
                .filter(|s| !s.is_empty());

            Some(MediaMetadata {
                kind: MediaKind::TvEpisode,
                title,
                year,
                season: None,
                episode: None,
                episode_title: None,
                ids: ExternalIds {
                    imdb_id,
                    tvdb_id,
                    tmdb_id: None,
                },
                overview,
                poster_url,
            })
        })
        .take(10)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_bluray_and_tv_disc_labels() {
        let movie = clean_disc_label("THE_MATRIX_1999_UHD_DISC_1");
        assert_eq!(movie.cleaned_title, "The Matrix");
        assert_eq!(movie.guessed_year, Some(1999));
        assert_eq!(movie.guessed_season, None);

        let tv = clean_disc_label("BREAKING_BAD_S02_D1");
        assert_eq!(tv.cleaned_title, "Breaking Bad");
        assert_eq!(tv.guessed_season, Some(2));
    }

    #[test]
    fn normalizes_ids_from_raw_values_and_urls() {
        let ids = ExternalIds {
            imdb_id: Some("https://www.imdb.com/title/tt0133093/".to_string()),
            tvdb_id: Some("https://thetvdb.com/series/81189".to_string()),
            tmdb_id: Some("603".to_string()),
        }
        .normalized();

        assert_eq!(ids.imdb_id.as_deref(), Some("tt0133093"));
        assert_eq!(ids.tvdb_id.as_deref(), Some("81189"));
        assert_eq!(ids.tmdb_id.as_deref(), Some("603"));
    }

    #[test]
    fn formats_plex_movie_and_tv_paths() {
        let movie = MediaMetadata {
            kind: MediaKind::Movie,
            title: "The Matrix".to_string(),
            year: Some(1999),
            season: None,
            episode: None,
            episode_title: None,
            ids: ExternalIds {
                imdb_id: Some("tt0133093".to_string()),
                tvdb_id: None,
                tmdb_id: Some("603".to_string()),
            },
            overview: None,
            poster_url: None,
        };
        assert_eq!(
            format_output_relative_path(&movie, NamingConvention::Plex, "mkv"),
            "The Matrix (1999) {imdb-tt0133093}/The Matrix (1999) {imdb-tt0133093}.mkv"
        );

        let tv = MediaMetadata {
            kind: MediaKind::TvEpisode,
            title: "Breaking Bad".to_string(),
            year: Some(2008),
            season: Some(1),
            episode: Some(1),
            episode_title: Some("Pilot".to_string()),
            ids: ExternalIds {
                imdb_id: Some("tt0903747".to_string()),
                tvdb_id: Some("81189".to_string()),
                tmdb_id: None,
            },
            overview: None,
            poster_url: None,
        };
        assert_eq!(
            format_output_relative_path(&tv, NamingConvention::Plex, "mkv"),
            "Breaking Bad (2008) {tvdb-81189}/Season 01/Breaking Bad (2008) - S01E01 - Pilot.mkv"
        );
        assert_eq!(
            format_output_relative_path_with_org(
                &tv,
                NamingConvention::Plex,
                "mkv",
                FolderOrganization::SeasonOnly
            ),
            "Season 01/Breaking Bad (2008) - S01E01 - Pilot.mkv"
        );
        assert_eq!(
            format_output_relative_path_with_org(
                &tv,
                NamingConvention::Plex,
                "mkv",
                FolderOrganization::Flat
            ),
            "Breaking Bad (2008) - S01E01 - Pilot.mkv"
        );
        assert_eq!(
            format_output_relative_path_with_org(
                &tv,
                NamingConvention::Plain,
                "mp4",
                FolderOrganization::ShowAndSeason
            ),
            "Breaking Bad (2008)/Season 01/Breaking Bad (2008) - S01E01 - Pilot.mp4"
        );
    }

    #[test]
    fn parses_omdb_tvdb_tmdb_and_tvmaze_payloads() {
        let omdb = serde_json::json!({
            "Response": "True",
            "Search": [{
                "Title": "The Matrix",
                "Year": "1999",
                "imdbID": "tt0133093",
                "Type": "movie",
                "Poster": "https://img.example/matrix.jpg"
            }]
        });
        let omdb_res = parse_omdb_response(&omdb, MediaKind::Movie).unwrap();
        assert_eq!(omdb_res.len(), 1);
        assert_eq!(omdb_res[0].ids.imdb_id.as_deref(), Some("tt0133093"));
        assert_eq!(omdb_res[0].year, Some(1999));

        let tvdb = serde_json::json!({
            "data": [{
                "tvdb_id": "81189",
                "name": "Breaking Bad",
                "year": "2008",
                "type": "series",
                "image_url": "https://artworks.thetvdb.com/bb.jpg",
                "remote_ids": [{ "sourceName": "IMDB", "id": "tt0903747" }]
            }]
        });
        let tvdb_res = parse_tvdb_response(&tvdb, MediaKind::TvEpisode);
        assert_eq!(tvdb_res.len(), 1);
        assert_eq!(tvdb_res[0].ids.tvdb_id.as_deref(), Some("81189"));
        assert_eq!(tvdb_res[0].ids.imdb_id.as_deref(), Some("tt0903747"));

        let tmdb = serde_json::json!({
            "results": [{
                "id": 603,
                "title": "The Matrix",
                "release_date": "1999-03-31",
                "poster_path": "/matrix.jpg",
                "overview": "A hacker discovers reality."
            }]
        });
        let tmdb_res = parse_tmdb_response(&tmdb, MediaKind::Movie);
        assert_eq!(tmdb_res.len(), 1);
        assert_eq!(tmdb_res[0].ids.tmdb_id.as_deref(), Some("603"));
        assert_eq!(tmdb_res[0].year, Some(1999));

        let tvmaze = serde_json::json!([{
            "show": {
                "name": "Breaking Bad",
                "premiered": "2008-01-20",
                "externals": { "thetvdb": 81189, "imdb": "tt0903747" },
                "image": { "medium": "https://tvmaze.example/bb.jpg" },
                "summary": "<p>A chemistry <b>teacher</b>.</p>"
            }
        }]);
        let tvmaze_res = parse_tvmaze_response(&tvmaze);
        assert_eq!(tvmaze_res.len(), 1);
        assert_eq!(tvmaze_res[0].ids.tvdb_id.as_deref(), Some("81189"));
        assert_eq!(tvmaze_res[0].ids.imdb_id.as_deref(), Some("tt0903747"));
        assert_eq!(tvmaze_res[0].overview.as_deref(), Some("A chemistry teacher."));

        let imdb_sug = serde_json::json!({
            "d": [
                {
                    "id": "tt0133093",
                    "l": "The Matrix",
                    "qid": "movie",
                    "y": 1999,
                    "i": { "imageUrl": "https://m.media-amazon.com/matrix.jpg" }
                },
                {
                    "id": "nm0000206",
                    "l": "Keanu Reeves"
                }
            ]
        });
        let sug_res = parse_imdb_suggestion_movies(&imdb_sug, Some(1999));
        assert_eq!(sug_res.len(), 1);
        assert_eq!(sug_res[0].title, "The Matrix");
        assert_eq!(sug_res[0].ids.imdb_id.as_deref(), Some("tt0133093"));
        assert_eq!(sug_res[0].year, Some(1999));
    }
}
