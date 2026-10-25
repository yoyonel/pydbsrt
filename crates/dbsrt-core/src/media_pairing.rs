//! Automated subtitle pairing convention (stem matching and language extraction).

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Supported video container file extensions for pairing discovery.
pub const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "webm", "avi", "mov", "m4v", "ts", "flv", "wmv", "mpg", "mpeg",
];

/// Candidate subdirectory names where subtitle sidecars are commonly placed.
pub const SUBTITLE_SUBDIRS: &[&str] = &["subs", "Subs", "subtitles", "Subtitles"];

/// Subtitle file extension (case-insensitive).
pub const SUBTITLE_EXTENSION: &str = "srt";

/// Extract normalized ISO-639-1 language code from a token.
/// STRICT: only matches 2-letter ISO 639-1 or 3-letter ISO 639-2 codes.
/// Words like "french", "english", "vff", "vf", "vostfr" are REJECTED
/// to prevent false positives from film titles (e.g. "The.French.Dispatch").
pub fn parse_language_code(token: &str) -> Option<String> {
    if token.len() < 2 || token.len() > 3 {
        return None;
    }
    let lower = token.to_lowercase();
    if !lower.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    match lower.as_str() {
        "fr" | "en" | "es" | "de" | "it" | "pt" | "ja" | "nl" | "ru" => Some(lower),
        "fre" | "fra" => Some("fr".to_string()),
        "eng" => Some("en".to_string()),
        "spa" => Some("es".to_string()),
        "ger" | "deu" => Some("de".to_string()),
        "ita" => Some("it".to_string()),
        "por" | "pob" => Some("pt".to_string()),
        "jpn" => Some("ja".to_string()),
        "dut" | "nld" => Some("nl".to_string()),
        "rus" => Some("ru".to_string()),
        _ => None,
    }
}

/// Discovers subtitle `.srt` files matching the video filename stem.
///
/// Returns a list of `(subtitle_path, detected_lang)` pairs.
pub fn find_subtitle_pairs(video_path: &Path) -> Result<Vec<(PathBuf, Option<String>)>> {
    let video_file_name = video_path
        .file_name()
        .and_then(|f| f.to_str())
        .context("Invalid or non-UTF-8 video filename")?;

    let video_stem = match video_file_name.rsplit_once('.') {
        Some((stem, _ext)) => stem,
        None => video_file_name,
    };
    let norm_video_stem = video_stem.to_lowercase();

    let parent_dir = video_path.parent().unwrap_or_else(|| Path::new("."));
    if !parent_dir.exists() {
        return Ok(Vec::new());
    }

    let mut search_dirs = vec![parent_dir.to_path_buf()];
    for subdir_name in SUBTITLE_SUBDIRS {
        let sub = parent_dir.join(subdir_name);
        if sub.is_dir() {
            search_dirs.push(sub);
        }
    }

    let mut pairs = Vec::new();
    let mut seen_canonical = HashSet::new();

    for dir in search_dirs {
        let entries = match fs::read_dir(&dir) {
            Ok(iter) => iter,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let srt_name = match path.file_name().and_then(|f| f.to_str()) {
                Some(name) => name,
                None => continue,
            };

            // Check if extension is .srt (case-insensitive)
            let srt_stem = match srt_name.rsplit_once('.') {
                Some((stem, ext)) if ext.eq_ignore_ascii_case(SUBTITLE_EXTENSION) => stem,
                _ => continue,
            };

            let norm_srt_stem = srt_stem.to_lowercase();

            // Match exact stem or stem followed by delimiter (. / _ / -)
            let lang_opt = if norm_srt_stem == norm_video_stem {
                None
            } else if let Some(rem) = norm_srt_stem.strip_prefix(&norm_video_stem) {
                // Must be separated by a delimiter (. _ -)
                if !rem.starts_with(['.', '_', '-']) {
                    continue;
                }
                // Find first valid ISO language token in the remaining suffix
                rem.split(['.', '_', '-']).find_map(parse_language_code)
            } else {
                continue;
            };

            let dedup_key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if seen_canonical.insert(dedup_key) {
                pairs.push((path, lang_opt));
            }
        }
    }

    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    struct TempTestDir {
        path: PathBuf,
    }

    impl TempTestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dbsrt_test_pairing_{}_{}_{}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).expect("Failed to create temp test directory");
            Self { path }
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_parse_language_code_rejects_full_words() {
        // Words from movie titles must not match
        assert_eq!(parse_language_code("french"), None);
        assert_eq!(parse_language_code("spanish"), None);
        assert_eq!(parse_language_code("english"), None);
        assert_eq!(parse_language_code("vff"), None);
        assert_eq!(parse_language_code("vostfr"), None);
        assert_eq!(
            parse_language_code("pt4"),
            None,
            "Should reject alphanumerics"
        );
    }

    #[test]
    fn test_parse_language_code_strict_iso_codes() {
        assert_eq!(parse_language_code("fr"), Some("fr".to_string()));
        assert_eq!(parse_language_code("fre"), Some("fr".to_string()));
        assert_eq!(parse_language_code("ENG"), Some("en".to_string()));
        assert_eq!(parse_language_code("pt"), Some("pt".to_string()));
        assert_eq!(
            parse_language_code("p"),
            None,
            "Should reject 1-char tokens"
        );
    }

    #[test]
    fn test_zero_srt_returns_empty_list() {
        let temp = TempTestDir::new("zero_srt");
        let video = temp.path.join("movie.mkv");
        File::create(&video).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert!(pairs.is_empty(), "Expected empty vector for 0 SRT files");
    }

    #[test]
    fn test_single_srt_exact_stem_match() {
        let temp = TempTestDir::new("single_exact");
        let video = temp.path.join("movie.mkv");
        let srt = temp.path.join("movie.srt");
        File::create(&video).unwrap();
        File::create(&srt).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, srt);
        assert_eq!(pairs[0].1, None);
    }

    #[test]
    fn test_single_srt_with_lang_suffix() {
        let temp = TempTestDir::new("single_lang");
        let video = temp.path.join("movie.mkv");
        let srt = temp.path.join("movie.fr.srt");
        File::create(&video).unwrap();
        File::create(&srt).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, srt);
        assert_eq!(pairs[0].1, Some("fr".to_string()));
    }

    #[test]
    fn test_multiple_srt_multilingual_ordering() {
        let temp = TempTestDir::new("multi_lang");
        let video = temp.path.join("matrix.1999.1080p.mkv");
        let srt_en = temp.path.join("matrix.1999.1080p.en.srt");
        let srt_fr = temp.path.join("matrix.1999.1080p.fr.srt");
        let srt_unrelated = temp.path.join("other_film.fr.srt");
        File::create(&video).unwrap();
        File::create(&srt_en).unwrap();
        File::create(&srt_fr).unwrap();
        File::create(&srt_unrelated).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert_eq!(pairs.len(), 2);
        let langs: Vec<Option<String>> = pairs.iter().map(|p| p.1.clone()).collect();
        assert!(langs.contains(&Some("en".to_string())));
        assert!(langs.contains(&Some("fr".to_string())));
    }

    #[test]
    fn test_srt_in_subfolder() {
        let temp = TempTestDir::new("subfolder");
        let video = temp.path.join("inception.mp4");
        let subs_dir = temp.path.join("Subs");
        fs::create_dir_all(&subs_dir).unwrap();
        let srt = subs_dir.join("inception.fr.srt");
        File::create(&video).unwrap();
        File::create(&srt).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, srt);
        assert_eq!(pairs[0].1, Some("fr".to_string()));
    }

    #[test]
    fn test_case_insensitive_stem_matching() {
        let temp = TempTestDir::new("case_insensitive");
        let video = temp.path.join("AVATAR.2009.MKV");
        let srt = temp.path.join("avatar.2009.FR.SRT");
        File::create(&video).unwrap();
        File::create(&srt).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, srt);
        assert_eq!(pairs[0].1, Some("fr".to_string()));
    }
}
