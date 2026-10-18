//! Media metadata extraction and release title parsing.
//!
//! Provides fast, allocation-minimized parsing for Scene, P2P, VOD, and Anime
//! release naming conventions (e.g. SxxExx, 1x02, resolution, source, release group).

use serde::{Deserialize, Serialize};
use std::path::Path;

/// High-level media category classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaType {
    /// Feature film / movie
    Movie,
    /// Television episode (part of a series)
    TvEpisode,
    /// Japanese animation release
    Anime,
    /// Other / unrecognized media
    Other,
}

/// Structured metadata extracted from media release filenames and paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaMetadata {
    /// Raw input string / filename before parsing
    pub raw_title: String,
    /// Clean canonical title with release noise stripped
    pub clean_title: String,
    /// Detected media type
    pub media_type: MediaType,
    /// Release or production year, if identified (1900..=2099)
    pub year: Option<u16>,
    /// Season index (1-based), if identified
    pub season: Option<u32>,
    /// Episode index (1-based), if identified
    pub episode: Option<u32>,
    /// Video vertical resolution tag (e.g. "1080p", "720p", "2160p")
    pub resolution: Option<String>,
    /// Release source (e.g. "WEB-DL", "WEBRip", "BluRay", "HDTV")
    pub source: Option<String>,
    /// Release group tag (e.g. "KILLERS", "SiGMA", "MZABI", "SubsPlease")
    pub release_group: Option<String>,
    /// Optional OpenSubtitles 64-bit MovieHash (16-char hex string)
    #[serde(default)]
    pub moviehash: Option<String>,
}

/// Computes the standard 64-bit OpenSubtitles MovieHash for a local video file
/// (filesize + 64 KiB head + 64 KiB tail summed as little-endian u64 words).
pub fn compute_opensubtitles_hash<P: AsRef<Path>>(path: P) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    const CHUNK_SIZE: u64 = 65536;

    let mut file = std::fs::File::open(path.as_ref())?;
    let file_size = file.metadata()?.len();
    if file_size < CHUNK_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "File smaller than 64 KiB for OpenSubtitles MovieHash",
        ));
    }

    let mut hash: u64 = file_size;
    let mut buf = [0u8; 8];

    // Sum first 64 KiB (8192 u64 words)
    file.seek(SeekFrom::Start(0))?;
    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    // Sum last 64 KiB (8192 u64 words)
    let tail_offset = file_size.saturating_sub(CHUNK_SIZE);
    file.seek(SeekFrom::Start(tail_offset))?;
    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    Ok(format!("{:016x}", hash))
}

impl MediaMetadata {
    /// Parses a media filename or path string into structured [`MediaMetadata`].
    pub fn parse(input: &str) -> Self {
        let raw = input.trim().to_string();
        let stripped_name = strip_known_extension(&raw);

        // Check for anime pattern: [ReleaseGroup] Title - 01 (1080p)...
        if let Some(anime_meta) = parse_anime_convention(&raw, stripped_name) {
            return anime_meta;
        }

        // Standard Scene / P2P / VOD parsing
        parse_scene_convention(&raw, stripped_name)
    }

    /// Parses metadata from a filesystem path.
    pub fn parse_from_path<P: AsRef<Path>>(path: P) -> Self {
        let p = path.as_ref();
        let filename = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_else(|| p.to_str().unwrap_or(""));
        let mut meta = Self::parse(filename);
        if p.exists() {
            meta.moviehash = compute_opensubtitles_hash(p).ok();
        }
        meta
    }
}

/// Strips file extension and path components if present.
fn strip_known_extension(input: &str) -> &str {
    let name = match input.rfind('/') {
        Some(pos) => &input[pos + 1..],
        None => input,
    };
    let name = match name.rfind('\\') {
        Some(pos) => &name[pos + 1..],
        None => name,
    };

    let extensions = [
        ".phash", ".mkv", ".mp4", ".avi", ".ts", ".m2ts", ".srt", ".sub", ".idx", ".ass", ".mov",
        ".wmv", ".m4v", ".webm",
    ];

    let mut current = name;
    for ext in extensions {
        if current.to_lowercase().ends_with(ext) {
            current = &current[..current.len() - ext.len()];
            break;
        }
    }
    current
}

/// Parses anime releases typically formatted as:
/// `[Group] Title - 01 (1080p) [CRC].mkv` or `[Group] Title Season - 02 [1080p].mkv`
fn parse_anime_convention(raw: &str, name: &str) -> Option<MediaMetadata> {
    if !name.starts_with('[') {
        return None;
    }

    let close_bracket = name.find(']')?;
    let group = &name[1..close_bracket];
    let remainder = name[close_bracket + 1..].trim();

    let dash_pos = remainder.find(" - ")?;
    let raw_title_part = remainder[..dash_pos].trim();
    let after_dash = remainder[dash_pos + 3..].trim();

    let mut ep_digits = String::new();
    for c in after_dash.chars() {
        if c.is_ascii_digit() {
            ep_digits.push(c);
        } else if !ep_digits.is_empty() {
            break;
        }
    }

    if ep_digits.is_empty() {
        return None;
    }

    let episode: Option<u32> = ep_digits.parse().ok();
    let resolution = extract_resolution(remainder);
    let year = extract_year(raw_title_part);
    let clean_title = clean_dots_and_spaces(raw_title_part);

    Some(MediaMetadata {
        raw_title: raw.to_string(),
        clean_title,
        media_type: MediaType::Anime,
        year,
        season: None,
        episode,
        resolution,
        source: extract_source(remainder),
        release_group: Some(group.to_string()),
        moviehash: None,
    })
}

/// Parses standard Scene / P2P releases:
/// e.g. `Silicon.Valley.S02E01.Sand.Hill.Shuffle.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA`
fn parse_scene_convention(raw: &str, name: &str) -> MediaMetadata {
    // 0. Strip leading scene tags like daa-, repack-, proper-
    let mut normalized_name = name;
    let prefixes = ["daa-", "repack-", "proper-", "repack.", "proper."];
    for prefix in prefixes {
        if normalized_name.to_lowercase().starts_with(prefix) {
            normalized_name = &normalized_name[prefix.len()..];
            break;
        }
    }

    // 1. Extract release group (typically after the last hyphen)
    let (base_name, release_group) = extract_release_group(normalized_name);

    // 2. Tokenize base name by delimiters [. _ - ( ) [ ] ]
    let tokens = tokenize(base_name);

    // 3. Locate markers: Season/Episode, Year, Resolution, Source
    let mut season = None;
    let mut episode = None;
    let mut resolution = None;
    let mut source = None;

    let mut se_token_idx = None;
    let mut tech_spec_token_idx = None;
    let mut year_indices = Vec::new();

    for (idx, token) in tokens.iter().enumerate() {
        let t = token.as_str();

        // Check for SxxExx or 1x02
        if season.is_none() && episode.is_none() {
            if let Some((s, e)) = parse_season_episode(t) {
                season = Some(s);
                episode = e;
                if se_token_idx.is_none() {
                    se_token_idx = Some(idx);
                }
                continue;
            }
        }

        // Check for single Season marker: S01, Season 1
        if season.is_none() {
            if let Some(s) = parse_season_only(t) {
                season = Some(s);
                if se_token_idx.is_none() {
                    se_token_idx = Some(idx);
                }
                continue;
            }
        }

        // Check for Resolution
        if let Some(res) = parse_resolution_token(t) {
            if resolution.is_none() {
                resolution = Some(res);
            }
            if tech_spec_token_idx.is_none() {
                tech_spec_token_idx = Some(idx);
            }
            continue;
        }

        // Check for Source
        if let Some(src) = parse_source_token(t) {
            if source.is_none() {
                source = Some(src);
            }
            if tech_spec_token_idx.is_none() {
                tech_spec_token_idx = Some(idx);
            }
            continue;
        }

        // Check for Year (1900..=2099)
        if let Some(y) = parse_year_token(t) {
            year_indices.push((idx, y));
            continue;
        }

        // Codec or audio noise tokens
        if is_codec_or_noise_token(t) && tech_spec_token_idx.is_none() {
            tech_spec_token_idx = Some(idx);
        }
    }

    // Determine the boundary before which technical specs or season/episode start
    let spec_boundary = match (se_token_idx, tech_spec_token_idx) {
        (Some(s), Some(t)) => Some(s.min(t)),
        (Some(s), None) => Some(s),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    };

    // Filter year tokens that appear before the spec boundary
    let candidate_years: Vec<(usize, u16)> = match spec_boundary {
        Some(bound) => year_indices
            .into_iter()
            .filter(|(idx, _)| *idx <= bound)
            .collect(),
        None => year_indices,
    };

    let mut year = None;
    let mut title_cutoff_idx = spec_boundary.unwrap_or(tokens.len());

    if !candidate_years.is_empty() {
        if candidate_years.len() > 1 {
            // Multiple year tokens: last one is the release year, preceding ones are title components
            let (last_idx, last_year) = candidate_years.last().copied().unwrap();
            year = Some(last_year);
            title_cutoff_idx = title_cutoff_idx.min(last_idx);
        } else {
            // Single year token
            let (only_idx, only_year) = candidate_years[0];
            // If the single year is at position 0 followed by other title words before boundary,
            // e.g. "2001 A Space Odyssey", then 2001 is part of title, not release year!
            if only_idx == 0 && spec_boundary.is_some_and(|b| b > 1) {
                // Keep 2001 in title
                year = None;
            } else {
                year = Some(only_year);
                title_cutoff_idx = title_cutoff_idx.min(only_idx);
            }
        }
    }

    // Extract title tokens up to cutoff
    let title_parts: Vec<&str> = tokens[..title_cutoff_idx]
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut clean_title = title_parts.join(" ").trim().to_string();
    if clean_title.is_empty() {
        clean_title = name.to_string();
    }

    // Media type classification
    let media_type = if season.is_some() || episode.is_some() {
        MediaType::TvEpisode
    } else if year.is_some() || resolution.is_some() || source.is_some() {
        MediaType::Movie
    } else {
        MediaType::Other
    };

    MediaMetadata {
        raw_title: raw.to_string(),
        clean_title,
        media_type,
        year,
        season,
        episode,
        resolution,
        source,
        release_group,
        moviehash: None,
    }
}

/// Tokenizes filename by delimiters (dots, spaces, underscores, brackets).
fn tokenize(name: &str) -> Vec<String> {
    // Normalize hyphens when used before resolution or codecs: e.g. -1080p, -720p, -x264
    let mut normalized = name.to_string();
    for tag in &[
        "-4320p", "-2160p", "-1080p", "-720p", "-576p", "-480p", "-1080i", "-720i", "-x264",
        "-x265", "-h264", "-hevc",
    ] {
        let dot_replacement = format!(".{}", &tag[1..]);
        normalized = normalized.replace(tag, &dot_replacement);
    }

    let mut tokens = Vec::new();
    let mut current = String::new();

    for c in normalized.chars() {
        if c == '.' || c == '_' || c == ' ' || c == '(' || c == ')' || c == '[' || c == ']' {
            if !current.is_empty() {
                tokens.push(current);
                current = String::new();
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Extracts release group if present at the end of the filename (after `-`).
fn extract_release_group(name: &str) -> (&str, Option<String>) {
    let clean_end = if let Some(open) = name.rfind('[') {
        if name.ends_with(']') && open > 0 {
            name[..open].trim_end()
        } else {
            name
        }
    } else {
        name
    };

    if let Some(pos) = clean_end.rfind('-') {
        let candidate = &clean_end[pos + 1..].trim();
        let cand_lower = candidate.to_lowercase();
        if !candidate.is_empty()
            && !candidate.contains('.')
            && !is_resolution_or_codec(&cand_lower)
            && candidate.len() <= 24
        {
            return (&clean_end[..pos], Some(candidate.to_string()));
        }
    }

    (clean_end, None)
}

fn is_resolution_or_codec(s: &str) -> bool {
    let resolutions = [
        "4320p", "2160p", "1080p", "1080i", "720p", "720i", "576p", "576i", "480p", "480i", "4k",
        "8k",
    ];
    if resolutions.contains(&s) {
        return true;
    }
    let codecs = [
        "x264", "x265", "h264", "h265", "hevc", "avc", "av1", "xvid", "divx",
    ];
    if codecs.iter().any(|c| s.starts_with(c)) {
        return true;
    }
    false
}

/// Parses Season & Episode from tokens like:
/// - `S01E02`, `s01e02`, `s1e2`
/// - `1x02`, `01x02`
/// - `S01E01-E02`, `S01E01E02`
fn parse_season_episode(t: &str) -> Option<(u32, Option<u32>)> {
    let upper = t.to_uppercase();

    // SxxExx pattern
    if upper.starts_with('S') {
        if let Some(e_idx) = upper.find('E') {
            if e_idx > 1 {
                let season_part = &upper[1..e_idx];
                let episode_part = &upper[e_idx + 1..];

                if let Ok(s) = season_part.parse::<u32>() {
                    let mut ep_str = String::new();
                    for c in episode_part.chars() {
                        if c.is_ascii_digit() {
                            ep_str.push(c);
                        } else {
                            break;
                        }
                    }
                    let e = ep_str.parse::<u32>().ok();
                    return Some((s, e));
                }
            }
        }
    }

    // 1x02 pattern
    if let Some(x_idx) = upper.find('X') {
        if x_idx > 0 && x_idx < upper.len() - 1 {
            let season_part = &upper[..x_idx];
            let episode_part = &upper[x_idx + 1..];

            if let (Ok(s), Ok(e)) = (season_part.parse::<u32>(), episode_part.parse::<u32>()) {
                return Some((s, Some(e)));
            }
        }
    }

    None
}

/// Parses single season token like `S01`, `S1`, `Season1`.
fn parse_season_only(t: &str) -> Option<u32> {
    let upper = t.to_uppercase();
    if upper.starts_with('S') && upper.len() > 1 && upper[1..].chars().all(|c| c.is_ascii_digit()) {
        return upper[1..].parse().ok();
    }
    if upper.starts_with("SEASON")
        && upper.len() > 6
        && upper[6..].chars().all(|c| c.is_ascii_digit())
    {
        return upper[6..].parse().ok();
    }
    None
}

/// Parses 4-digit year tokens within 1900..=2099.
fn parse_year_token(t: &str) -> Option<u16> {
    if t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) {
        let val: u16 = t.parse().ok()?;
        if (1900..=2099).contains(&val) {
            return Some(val);
        }
    }
    None
}

/// Normalizes resolution tokens (e.g. `1080p`, `720p`, `2160p`, `4k`).
fn parse_resolution_token(t: &str) -> Option<String> {
    let lower = t.to_lowercase();
    match lower.as_str() {
        "4320p" | "8k" => Some("4320p".to_string()),
        "2160p" | "4k" | "uhd" => Some("2160p".to_string()),
        "1080p" | "1080i" | "fhd" => Some("1080p".to_string()),
        "720p" | "720i" | "hd" => Some("720p".to_string()),
        "576p" | "576i" | "480p" | "480i" | "sd" => Some(lower),
        _ => {
            if lower.ends_with('p') || lower.ends_with('i') {
                let num_part = &lower[..lower.len() - 1];
                if let Ok(num) = num_part.parse::<u32>() {
                    if (360..=8640).contains(&num) {
                        return Some(lower);
                    }
                }
            }
            None
        }
    }
}

/// Normalizes source tokens (e.g. `WEB-DL`, `WEBRip`, `BluRay`, `HDTV`).
fn parse_source_token(t: &str) -> Option<String> {
    let upper = t.to_uppercase();
    match upper.as_str() {
        "WEB-DL" | "WEBDL" => Some("WEB-DL".to_string()),
        "WEBRIP" | "WEB-RIP" | "WEB" => Some("WEBRip".to_string()),
        "BLURAY" | "BLU-RAY" | "BDRIP" | "BRRIP" => Some("BluRay".to_string()),
        "HDTV" | "PDTV" | "DVB" => Some("HDTV".to_string()),
        "DVDRIP" | "DVD" => Some("DVD".to_string()),
        _ => None,
    }
}

/// Returns true if token is standard noise/codec/audio token that marks non-title content.
fn is_codec_or_noise_token(t: &str) -> bool {
    let upper = t.to_uppercase();
    matches!(
        upper.as_str(),
        "X264"
            | "X265"
            | "H264"
            | "H265"
            | "HEVC"
            | "AVC"
            | "AV1"
            | "AAC"
            | "DTS"
            | "DDP5"
            | "DD5"
            | "ATMOS"
            | "REPACK"
            | "PROPER"
            | "INTERNAL"
            | "COMPLETE"
            | "LIMITED"
            | "EXTRAS"
    )
}

fn extract_resolution(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    for res in &["2160p", "1080p", "720p", "480p", "576p", "4k"] {
        if lower.contains(res) {
            return match *res {
                "4k" => Some("2160p".to_string()),
                other => Some(other.to_string()),
            };
        }
    }
    None
}

fn extract_year(s: &str) -> Option<u16> {
    for token in tokenize(s) {
        if let Some(y) = parse_year_token(&token) {
            return Some(y);
        }
    }
    None
}

fn extract_source(s: &str) -> Option<String> {
    for token in tokenize(s) {
        if let Some(src) = parse_source_token(&token) {
            return Some(src);
        }
    }
    None
}

fn clean_dots_and_spaces(s: &str) -> String {
    s.replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
