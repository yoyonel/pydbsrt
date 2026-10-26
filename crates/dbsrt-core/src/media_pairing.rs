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
        "fr" | "en" | "es" | "de" | "it" | "pt" | "ja" | "nl" | "ru" | "ar" | "pl" | "zh"
        | "ko" | "sv" | "da" | "fi" | "el" | "cs" | "tr" | "uk" | "ro" | "hu" | "he" | "hr"
        | "id" | "no" | "th" | "vi" | "tl" | "ms" | "bg" | "sl" | "sk" | "ca" => Some(lower),
        "fre" | "fra" => Some("fr".to_string()),
        "eng" => Some("en".to_string()),
        "spa" => Some("es".to_string()),
        "ger" | "deu" => Some("de".to_string()),
        "ita" => Some("it".to_string()),
        "por" | "pob" => Some("pt".to_string()),
        "jpn" => Some("ja".to_string()),
        "dut" | "nld" => Some("nl".to_string()),
        "rus" => Some("ru".to_string()),
        "ara" => Some("ar".to_string()),
        "pol" => Some("pl".to_string()),
        "zho" | "chi" => Some("zh".to_string()),
        "kor" => Some("ko".to_string()),
        "swe" => Some("sv".to_string()),
        "dan" => Some("da".to_string()),
        "fin" => Some("fi".to_string()),
        "ell" | "gre" => Some("el".to_string()),
        "ces" | "cze" => Some("cs".to_string()),
        "tur" => Some("tr".to_string()),
        "ukr" => Some("uk".to_string()),
        "ron" | "rum" => Some("ro".to_string()),
        "hun" => Some("hu".to_string()),
        "heb" => Some("he".to_string()),
        "hrv" => Some("hr".to_string()),
        "ind" => Some("id".to_string()),
        "nob" | "nor" => Some("no".to_string()),
        "tha" => Some("th".to_string()),
        "vie" => Some("vi".to_string()),
        "fil" | "tgl" => Some("tl".to_string()),
        "may" | "msa" => Some("ms".to_string()),
        "bul" => Some("bg".to_string()),
        "slv" => Some("sl".to_string()),
        "slk" | "slo" => Some("sk".to_string()),
        "cat" => Some("ca".to_string()),
        _ => None,
    }
}

/// Matches ISO 639-1 / 639-2 codes as well as common natural language names
/// and release tags (e.g. "English", "French", "VF", "VOSTFR", "Spanish", "German").
///
/// Safe for subtitle track pairing where the video stem has already been stripped
/// or where tracks are inside a dedicated subtitle directory (`Subs/<stem>/`).
pub fn parse_extended_language_token(token: &str) -> Option<String> {
    if let Some(code) = parse_language_code(token) {
        return Some(code);
    }
    let lower = token.trim().to_lowercase();
    match lower.as_str() {
        "english" => Some("en".to_string()),
        "french" | "francais" | "vff" | "vf" | "vostfr" => Some("fr".to_string()),
        "spanish" | "espanol" => Some("es".to_string()),
        "german" | "deutsch" => Some("de".to_string()),
        "italian" | "italiano" => Some("it".to_string()),
        "portuguese" | "portugues" => Some("pt".to_string()),
        "dutch" | "nederlands" => Some("nl".to_string()),
        "russian" => Some("ru".to_string()),
        "japanese" => Some("ja".to_string()),
        "chinese" => Some("zh".to_string()),
        "korean" => Some("ko".to_string()),
        "arabic" => Some("ar".to_string()),
        "polish" => Some("pl".to_string()),
        "swedish" => Some("sv".to_string()),
        "norwegian" => Some("no".to_string()),
        "danish" => Some("da".to_string()),
        "finnish" => Some("fi".to_string()),
        "greek" => Some("el".to_string()),
        "czech" => Some("cs".to_string()),
        "turkish" => Some("tr".to_string()),
        "hungarian" => Some("hu".to_string()),
        "romanian" => Some("ro".to_string()),
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

    let mut search_dirs: Vec<(PathBuf, bool)> = vec![(parent_dir.to_path_buf(), false)];
    let mut seen_dirs = HashSet::new();
    seen_dirs.insert(fs::canonicalize(parent_dir).unwrap_or_else(|_| parent_dir.to_path_buf()));

    let stem_in_parent = parent_dir.join(video_stem);
    if stem_in_parent.is_dir() {
        let canon = fs::canonicalize(&stem_in_parent).unwrap_or_else(|_| stem_in_parent.clone());
        if seen_dirs.insert(canon) {
            search_dirs.push((stem_in_parent, true));
        }
    }
    for subdir_name in SUBTITLE_SUBDIRS {
        let sub = parent_dir.join(subdir_name);
        if sub.is_dir() {
            let canon_sub = fs::canonicalize(&sub).unwrap_or_else(|_| sub.clone());
            if seen_dirs.insert(canon_sub) {
                search_dirs.push((sub.clone(), false));
            }
            let sub_stem = sub.join(video_stem);
            if sub_stem.is_dir() {
                let canon_sub_stem =
                    fs::canonicalize(&sub_stem).unwrap_or_else(|_| sub_stem.clone());
                if seen_dirs.insert(canon_sub_stem) {
                    search_dirs.push((sub_stem, true));
                }
            }
        }
    }

    let mut pairs = Vec::new();
    let mut seen_canonical = HashSet::new();

    for (dir, is_stem_dir) in search_dirs {
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

            let mut norm_srt_stem = srt_stem.to_lowercase();
            // Handle double extensions like .mkv.srt, .mp4.srt
            for vid_ext in SUPPORTED_VIDEO_EXTENSIONS {
                let dotted = format!(".{}", vid_ext);
                if norm_srt_stem.ends_with(&dotted) {
                    norm_srt_stem = norm_srt_stem.strip_suffix(&dotted).unwrap().to_string();
                    break;
                }
            }

            // Match exact stem or stem followed by delimiter (. / _ / -)
            let lang_opt = if norm_srt_stem == norm_video_stem {
                None
            } else if let Some(rem) = norm_srt_stem.strip_prefix(&norm_video_stem) {
                // Must be separated by a delimiter (. _ -)
                if !rem.starts_with(['.', '_', '-']) {
                    continue;
                }
                // Find first valid language token in the remaining suffix
                rem.split(['.', '_', '-'])
                    .find_map(parse_extended_language_token)
            } else if is_stem_dir {
                // Inside video-stem dedicated directory (e.g. Subs/<video_stem>/track.srt):
                // any .srt belongs to this video; parse language token if present
                norm_srt_stem
                    .split(['.', '_', '-'])
                    .find_map(parse_extended_language_token)
            } else {
                continue;
            };

            let dedup_key = fs::canonicalize(&path)
                .unwrap_or_else(|_| path.clone())
                .to_string_lossy()
                .to_lowercase();
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
    fn test_parse_extended_language_token() {
        assert_eq!(
            parse_extended_language_token("English"),
            Some("en".to_string())
        );
        assert_eq!(
            parse_extended_language_token("French"),
            Some("fr".to_string())
        );
        assert_eq!(
            parse_extended_language_token("vostfr"),
            Some("fr".to_string())
        );
        assert_eq!(
            parse_extended_language_token("Spanish"),
            Some("es".to_string())
        );
        assert_eq!(
            parse_extended_language_token("German"),
            Some("de".to_string())
        );
        assert_eq!(parse_extended_language_token("ita"), Some("it".to_string()));
        assert_eq!(parse_extended_language_token("unknown_token"), None);
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

    #[test]
    fn test_double_extension_video_srt_pairing() {
        let temp = TempTestDir::new("double_ext");
        let video = temp.path.join("carnivale.s01e06.mkv");
        let srt = temp.path.join("carnivale.s01e06.mkv.srt");
        File::create(&video).unwrap();
        File::create(&srt).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, srt);
        assert_eq!(pairs[0].1, None); // Untagged language -> LangStatus::Unknown
    }

    #[test]
    fn test_subs_video_stem_dedicated_folder() {
        let temp = TempTestDir::new("subs_stem_folder");
        let video = temp.path.join("Euphoria.S01E03.mp4");
        let subs_stem_dir = temp.path.join("Subs").join("Euphoria.S01E03");
        fs::create_dir_all(&subs_stem_dir).unwrap();
        let srt_en = subs_stem_dir.join("2_English.srt");
        let srt_fr = subs_stem_dir.join("track_fr.srt");
        File::create(&video).unwrap();
        File::create(&srt_en).unwrap();
        File::create(&srt_fr).unwrap();

        let pairs = find_subtitle_pairs(&video).unwrap();
        assert_eq!(pairs.len(), 2);
        let langs: Vec<Option<String>> = pairs.iter().map(|p| p.1.clone()).collect();
        assert!(langs.contains(&Some("en".to_string())));
        assert!(langs.contains(&Some("fr".to_string())));
    }
}
