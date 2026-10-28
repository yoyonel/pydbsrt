//! Subtitle catalog discovery, language filtering, and cache retrieval.

use crate::dtos::{SubtitleCandidate, SubtitleSource};
use crate::state::AppState;
use anyhow::Result;
use dbsrt_core::{CouplingRegistry, MediaMetadata, SubtitleFile};
use dbsrt_index::DbSubtitleCoupling;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Normalizes a language code (ISO 639-1 / 639-2 or common name) to canonical 2-letter lowercase.
///
/// # Arguments
/// - `lang`: Raw language string (e.g. `"fre"`, `"French"`, `"VF"`, `"en"`).
///
/// # Returns
/// Canonical 2-letter ISO code (`"fr"`, `"en"`, `"es"`, etc.) or unchanged lowercase string.
pub fn normalize_lang_code(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "fr" | "fre" | "fra" | "french" => "fr".to_string(),
        "en" | "eng" | "english" => "en".to_string(),
        "es" | "spa" | "spanish" => "es".to_string(),
        "de" | "ger" | "deu" | "german" => "de".to_string(),
        "it" | "ita" | "italian" => "it".to_string(),
        "pt" | "por" | "pob" | "portuguese" => "pt".to_string(),
        "ru" | "rus" | "russian" => "ru".to_string(),
        "ja" | "jpn" | "japanese" => "ja".to_string(),
        "zh" | "chi" | "zho" | "chinese" => "zh".to_string(),
        "nl" | "dut" | "nld" | "dutch" => "nl".to_string(),
        "pl" | "pol" | "polish" => "pl".to_string(),
        "tr" | "tur" | "turkish" => "tr".to_string(),
        other => other.to_string(),
    }
}

/// Extracts explicit language tag from a subtitle filename.
///
/// # Arguments
/// - `filename`: Basename of the subtitle file (e.g. `"movie.1080p.NF.fr.srt"`).
///
/// # Returns
/// - `Some(String)`: Canonical 2-letter language code if detected.
/// - `None`: If no language token is identified in the trailing segments.
///
/// # Data Flow & Invariants
/// Tokenizes the filename from right to left across separators (`.`, `_`, `-`, ` `)
/// ignoring noise tokens (`"forced"`, `"sdh"`, `"synced"`) and maps common language abbreviations.
const SUBTITLE_LANG_MAP: &[(&[&str], &str)] = &[
    (&["fr", "fre", "fra", "french", "vf", "vostfr"], "fr"),
    (&["en", "eng", "english", "vo"], "en"),
    (&["es", "spa", "spanish"], "es"),
    (&["de", "ger", "deu", "german"], "de"),
    (&["it", "ita", "italian"], "it"),
    (&["pt", "por", "pob", "portuguese"], "pt"),
    (&["ru", "rus", "russian"], "ru"),
    (&["ja", "jpn", "japanese"], "ja"),
    (&["zh", "chi", "zho", "chinese"], "zh"),
    (&["nl", "dut", "nld", "dutch"], "nl"),
    (&["pl", "pol", "polish"], "pl"),
    (&["tr", "tur", "turkish"], "tr"),
    (&["ar", "ara", "arabic"], "ar"),
    (&["he", "heb", "hebrew"], "he"),
    (&["sv", "swe", "swedish"], "sv"),
    (&["da", "dan", "danish"], "da"),
    (&["fi", "fin", "finnish"], "fi"),
    (&["no", "nor", "nob", "nno", "norwegian"], "no"),
    (&["cs", "cze", "ces", "czech"], "cs"),
    (&["hu", "hun", "hungarian"], "hu"),
    (&["ro", "rum", "ron", "romanian"], "ro"),
    (&["el", "ell", "gre", "greek"], "el"),
    (&["uk", "ukr", "ukrainian"], "uk"),
    (&["ko", "kor", "korean"], "ko"),
    (&["hi", "hin", "hindi"], "hi"),
    (&["id", "ind", "indonesian"], "id"),
    (&["th", "tha", "thai"], "th"),
    (&["vi", "vie", "vietnamese"], "vi"),
    (&["ms", "msa", "may", "malay"], "ms"),
    (&["tl", "tgl", "fil", "tagalog"], "tl"),
];

fn map_subtitle_lang_token(lower: &str) -> Option<&'static str> {
    for &(aliases, code) in SUBTITLE_LANG_MAP {
        if aliases.contains(&lower) {
            return Some(code);
        }
    }
    None
}

pub fn detect_subtitle_file_language(filename: &str) -> Option<String> {
    let base = filename.strip_suffix(".srt").unwrap_or(filename);
    for token in base
        .rsplit(['.', '_', '-', ' ', '(', ')', '[', ']'])
        .filter(|t| !t.is_empty())
        .take(4)
    {
        let lower = token.to_lowercase();
        if lower.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if matches!(
            lower.as_str(),
            "shift" | "synced" | "sdh" | "forced" | "srt" | "sub" | "vtt" | "ass" | "ssa"
        ) {
            continue;
        }
        if let Some(code) = map_subtitle_lang_token(&lower) {
            return Some(code.to_string());
        }
    }
    None
}

/// Collects candidate subtitle directories: `primary_dir` plus optional sidecar folders.
///
/// # Arguments
/// - `primary_dir`: Server default subtitles directory (`data/SRT`).
/// - `media_path`: Filesystem path of the media file being played.
/// - `allow_sidecars`: Whether adjacent folders (parent, parent/SRT, parent/Subs) should be included.
///
/// # Returns
/// Deduplicated vector of filesystem paths where subtitles may reside.
pub fn collect_subtitle_search_dirs(
    primary_dir: &Path,
    media_path: &Path,
    allow_sidecars: bool,
) -> Vec<PathBuf> {
    let mut dirs = vec![primary_dir.to_path_buf()];
    if !allow_sidecars {
        return dirs;
    }
    let mut push_unique = |p: PathBuf| {
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };
    if let Some(parent) = media_path.parent() {
        if !parent.as_os_str().is_empty() && parent != Path::new("/tmp") {
            push_unique(parent.to_path_buf());
            push_unique(parent.join("SRT"));
            push_unique(parent.join("Subs"));
            if let Some(gp) = parent.parent() {
                push_unique(gp.join("SRT"));
                if let Some(ggp) = gp.parent() {
                    push_unique(ggp.join("SRT"));
                }
            }
        }
    }
    dirs
}

struct CatalogFilterCriteria<'a> {
    clean_stems: &'a [String],
    query_norm: Option<&'a str>,
    eff_season: Option<u32>,
    eff_episode: Option<u32>,
    req_lang: &'a str,
    any_lang: bool,
}

fn matches_season_and_stem(
    fname: &str,
    cand_meta: &MediaMetadata,
    crit: &CatalogFilterCriteria<'_>,
) -> Option<(bool, bool)> {
    if let (Some(req_s), Some(req_e)) = (crit.eff_season, crit.eff_episode) {
        if let (Some(cs), Some(ce)) = (cand_meta.season, cand_meta.episode) {
            if cs != req_s || ce != req_e {
                return None;
            }
        }
    }

    let cand_stem = fname.strip_suffix(".srt").unwrap_or(fname);
    let is_exact_sidecar = crit.clean_stems.iter().any(|s| cand_stem == s);
    let has_exact_stem = crit
        .clean_stems
        .iter()
        .any(|stem_lower| fname.contains(stem_lower) || stem_lower.contains(cand_stem));

    let norm_alnum = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    let fname_alnum = norm_alnum(fname);
    let cand_title_norm = norm_alnum(&cand_meta.clean_title);

    let has_query = crit.query_norm.as_ref().is_some_and(|&qn| {
        if !cand_title_norm.is_empty() && cand_title_norm.len() >= 3 {
            cand_title_norm == qn || (qn.len() >= 5 && fname_alnum.starts_with(qn))
        } else {
            fname_alnum.contains(qn)
        }
    });

    if !has_exact_stem && !has_query {
        return None;
    }

    if !has_exact_stem {
        if let (Some(s), Some(e)) = (crit.eff_season, crit.eff_episode) {
            let se_match = cand_meta.season == Some(s) && cand_meta.episode == Some(e);
            let se_tag = format!("s{:02}e{:02}", s, e);
            let x_tag = format!("{}x{:02}", s, e);
            if !se_match && !fname.contains(&se_tag) && !fname.contains(&x_tag) {
                return None;
            }
        }
    }

    Some((has_exact_stem, is_exact_sidecar))
}

fn evaluate_catalog_subtitle(
    fname: &str,
    file_size: u64,
    crit: &CatalogFilterCriteria<'_>,
) -> Option<(i32, String)> {
    let cand_meta = MediaMetadata::parse(fname);
    let (has_exact_stem, is_exact_sidecar) = matches_season_and_stem(fname, &cand_meta, crit)?;

    let detected_lang = detect_subtitle_file_language(fname);
    let (mut score, resolved_lang) = match detected_lang {
        Some(ref d_lang) if *d_lang == crit.req_lang => (100, d_lang.clone()),
        Some(ref d_lang) if crit.any_lang => (50, d_lang.clone()),
        Some(_) => return None,
        None => {
            if is_exact_sidecar {
                (50, crit.req_lang.to_string())
            } else if crit.any_lang {
                (10, "und".to_string())
            } else {
                return None;
            }
        }
    };

    if has_exact_stem {
        score += 50;
    }
    if file_size >= 4096 {
        score += 15;
    }

    Some((score, resolved_lang))
}

/// Unified catalog search returning ranked candidates matching release identity and requested language.
///
/// # Arguments
/// - `subtitles_dir`: Directory to search within.
/// - `media_stems`: Recognized or queried media stems to match against.
/// - `query_title`: Optional explicit show/movie title.
/// - `season`: Optional season filter.
/// - `episode`: Optional episode filter.
/// - `language`: Target language code (`"fr"`, `"en"`, or `"auto"`).
///
/// # Returns
/// Vector of tuples `(score, path, detected_lang)` sorted by descending relevance score.
///
/// # Data Flow & Invariants
/// Enforces strict season and episode consistency to prevent cross-episode leakage.
/// Scores matches based on exact stem overlap, title containment, and release tags.
pub fn search_subtitles_in_catalog(
    subtitles_dir: &Path,
    media_stems: &[String],
    query_title: Option<&str>,
    season: Option<u32>,
    episode: Option<u32>,
    language: &str,
) -> Vec<(i32, PathBuf, String)> {
    if !subtitles_dir.exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(subtitles_dir) else {
        return Vec::new();
    };

    let req_lang = normalize_lang_code(language);
    let any_lang = req_lang == "auto" || req_lang == "all" || req_lang.is_empty();

    let clean_stems: Vec<String> = media_stems
        .iter()
        .map(|s| {
            s.trim_end_matches(".phash")
                .trim_end_matches(".mp4")
                .trim_end_matches(".webm")
                .trim_end_matches(".mkv")
                .to_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect();

    let stem_meta = clean_stems.first().map(|s| MediaMetadata::parse(s));
    let eff_season = season.or_else(|| stem_meta.as_ref().and_then(|m| m.season));
    let eff_episode = episode.or_else(|| stem_meta.as_ref().and_then(|m| m.episode));

    let norm_alnum = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };

    let query_norm: Option<String> = query_title
        .map(norm_alnum)
        .filter(|q| !q.is_empty())
        .or_else(|| {
            stem_meta
                .as_ref()
                .map(|m| norm_alnum(&m.clean_title))
                .filter(|q| q.len() >= 3)
        });

    let criteria = CatalogFilterCriteria {
        clean_stems: &clean_stems,
        query_norm: query_norm.as_deref(),
        eff_season,
        eff_episode,
        req_lang: &req_lang,
        any_lang,
    };

    let mut results: Vec<(i32, PathBuf, String)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("srt") {
            continue;
        }
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if file_size == 0 || (file_size > 0 && file_size < 10) {
            continue;
        }

        let Some(fname_os) = path.file_name() else {
            continue;
        };
        let fname = fname_os.to_string_lossy().to_lowercase();

        if let Some((score, resolved_lang)) =
            evaluate_catalog_subtitle(&fname, file_size, &criteria)
        {
            results.push((score, path, resolved_lang));
        }
    }

    results.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    results
}

pub fn find_subtitle_in_catalog(
    subtitles_dir: &Path,
    media_stem: &str,
    language: &str,
) -> Option<PathBuf> {
    let stems = vec![media_stem.to_string()];
    search_subtitles_in_catalog(subtitles_dir, &stems, None, None, None, language)
        .into_iter()
        .next()
        .map(|(_, path, _)| path)
}

/// Shared helper to fetch a remote subtitle from disk cache or download it via provider.
pub async fn fetch_or_cache_remote_subtitle(
    state: &AppState,
    meta: &MediaMetadata,
    language: &str,
    candidate: &dbsrt_core::RemoteSubtitleCandidate,
) -> Result<SubtitleFile> {
    if let Some(cached) = state.cache.get(meta, language, &candidate.subtitle_id) {
        if let Ok(parsed) = SubtitleFile::parse(&cached) {
            return Ok(parsed);
        }
    }

    let raw = state.provider.download(candidate).await?;
    let _ = state
        .cache
        .put(meta, language, &candidate.subtitle_id, &raw);
    SubtitleFile::parse(&raw)
}

/// Resolves certified subtitle couplings from PostgreSQL and/or the local `couplings.json` registry,
/// enforcing strict Season and Episode compatibility when specified.
///
/// # Arguments
/// - `state`: Shared server application state.
/// - `subtitles_dir`: Subtitles root directory.
/// - `matched_media`: Media release name or stem identified by visual pHash.
/// - `season`: Expected season number if known.
/// - `episode`: Expected episode number if known.
/// - `language`: Requested subtitle language (e.g. `"fr"` or `"auto"`).
/// - `phash_conf`: Visual recognition confidence score (0.0 to 1.0).
///
/// # Returns
/// A tuple `(candidates, pg_lookup_ms, json_lookup_ms, pg_cached)`:
/// - `candidates`: Vector of resolved certified [`SubtitleCandidate`] items.
/// - `pg_lookup_ms`: PostgreSQL query latency in milliseconds.
/// - `json_lookup_ms`: In-memory `couplings.json` query latency in milliseconds.
/// - `pg_cached`: Whether PostgreSQL connection pool hit local cache.
///
/// # Data Flow & Invariants
/// Prevents false-positive subtitle matching across episodes of the same series:
/// 1. Queries `CouplingRegistry` using `find_all_by_matched_media_opts`.
/// 2. Verifies subtitle and media metadata parsed from filenames do not contradict `season` and `episode`.
/// 3. Normalizes subtitle language codes to prevent mismatch between `"French"`, `"fre"`, and `"fr"`.
pub fn lookup_certified_couplings(
    state: &AppState,
    subtitles_dir: &Path,
    matched_media: &str,
    season: Option<u32>,
    episode: Option<u32>,
    language: &str,
    phash_conf: f64,
) -> (Vec<SubtitleCandidate>, f64, f64, bool) {
    let mut certified_candidates = Vec::new();
    let req_lang = normalize_lang_code(language);
    let any_lang = req_lang == "auto" || req_lang == "all" || req_lang.is_empty();
    let mut seen_paths = HashSet::new();

    let json_start = std::time::Instant::now();
    let reg_fallback = if subtitles_dir != state.subtitles_dir {
        let reg_path = CouplingRegistry::default_path(subtitles_dir);
        Some(CouplingRegistry::load_or_default(&reg_path).unwrap_or_default())
    } else {
        None
    };

    let reg_guard = if reg_fallback.is_none() {
        state.coupling_registry.read().ok()
    } else {
        None
    };

    let reg_ref: Option<&CouplingRegistry> = match (&reg_guard, &reg_fallback) {
        (Some(guard), _) => Some(&**guard),
        (_, Some(fallback)) => Some(fallback),
        _ => None,
    };

    if let Some(reg) = reg_ref {
        let matching_couplings = reg.find_all_by_matched_media_opts(matched_media, season, episode);
        for coupling in matching_couplings {
            for sub in &coupling.subtitles {
                let sub_lang = normalize_lang_code(&sub.lang);
                if !any_lang && sub_lang != req_lang {
                    continue;
                }

                // Verify subtitle metadata doesn't contradict requested season/episode
                let sub_meta = MediaMetadata::parse(&sub.name);
                let c_meta = MediaMetadata::parse(&coupling.media_name);
                let sub_s = sub_meta.season.or(c_meta.season);
                let sub_e = sub_meta.episode.or(c_meta.episode);
                if let Some(req_s) = season {
                    if let Some(s) = sub_s {
                        if s != req_s {
                            continue;
                        }
                    }
                }
                if let Some(req_e) = episode {
                    if let Some(e) = sub_e {
                        if e != req_e
                            && !sub.name.to_lowercase().contains(&format!("e{:02}", req_e))
                            && !coupling
                                .media_name
                                .to_lowercase()
                                .contains(&format!("e{:02}", req_e))
                        {
                            continue;
                        }
                    }
                }

                let cas_id = format!("sub:sha256:{}", sub.subtitles_hash);
                if seen_paths.insert(sub.subtitles_hash.clone()) {
                    certified_candidates.push(SubtitleCandidate {
                        source: SubtitleSource::Certified.as_str().to_string(),
                        language: sub_lang,
                        title: sub.name.clone(),
                        release: Some(coupling.phash_filename.clone()),
                        details: Some(format!(
                            "Certified Coupling ({} cues, SHA256: {}…) for '{}' ({:.0}% conf)",
                            sub.cue_count,
                            &sub.subtitles_hash[..8.min(sub.subtitles_hash.len())],
                            coupling.phash_filename,
                            phash_conf
                        )),
                        candidate_id: Some(cas_id),
                        shift_ms: None,
                    });
                }
            }
        }
    }
    let json_lookup_ms = json_start.elapsed().as_secs_f64() * 1000.0;

    let mut pg_lookup_ms = 0.0;
    let mut pg_cached = false;
    if !certified_candidates.is_empty() {
        pg_cached = true;
    } else if let Some(ref cfg) = state.db_config {
        let pg_start = std::time::Instant::now();
        let lang_opt = if any_lang {
            None
        } else {
            Some(req_lang.as_str())
        };
        if let Ok(db_subs) = cfg.with_client(|client| {
            client.get_subtitles_for_media(None, Some(matched_media), lang_opt)
        }) {
            for DbSubtitleCoupling {
                id,
                subtitles_hash,
                name,
                lang,
                media_name,
                ..
            } in db_subs
            {
                let sub_lang = normalize_lang_code(&lang);
                if !any_lang && sub_lang != req_lang {
                    continue;
                }

                // Verify DB subtitle metadata doesn't contradict requested season/episode
                let sub_meta = MediaMetadata::parse(&name);
                let med_meta = MediaMetadata::parse(&media_name);
                let sub_s = sub_meta.season.or(med_meta.season);
                let sub_e = sub_meta.episode.or(med_meta.episode);
                if let Some(req_s) = season {
                    if let Some(s) = sub_s {
                        if s != req_s {
                            continue;
                        }
                    }
                }
                if let Some(req_e) = episode {
                    if let Some(e) = sub_e {
                        if e != req_e
                            && !name.to_lowercase().contains(&format!("e{:02}", req_e))
                            && !media_name
                                .to_lowercase()
                                .contains(&format!("e{:02}", req_e))
                        {
                            continue;
                        }
                    }
                }

                let clean_h = subtitles_hash.trim().to_string();
                let cas_id = format!("sub:sha256:{}", clean_h);
                if seen_paths.insert(clean_h) {
                    certified_candidates.push(SubtitleCandidate {
                        source: SubtitleSource::Certified.as_str().to_string(),
                        language: lang,
                        title: format!("{}.srt", name.trim_end_matches(".srt")),
                        release: Some(media_name.clone()),
                        details: Some(format!(
                            "PostgreSQL Certified Coupling #{} for '{}' ({:.0}% conf)",
                            id, media_name, phash_conf
                        )),
                        candidate_id: Some(cas_id),
                        shift_ms: None,
                    });
                }
            }
        }
        pg_lookup_ms = pg_start.elapsed().as_secs_f64() * 1000.0;
    }

    (
        certified_candidates,
        pg_lookup_ms,
        json_lookup_ms,
        pg_cached,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_subtitle_file_language_multilingual() {
        assert_eq!(
            detect_subtitle_file_language(
                "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_fre_5.srt"
            ),
            Some("fr".to_string())
        );
        assert_eq!(
            detect_subtitle_file_language(
                "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_ind_22.srt"
            ),
            Some("id".to_string())
        );
        assert_eq!(
            detect_subtitle_file_language(
                "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_tha_24.srt"
            ),
            Some("th".to_string())
        );
        assert_eq!(
            detect_subtitle_file_language(
                "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_vie_23.srt"
            ),
            Some("vi".to_string())
        );
        assert_eq!(
            detect_subtitle_file_language(
                "Maniac.2018.S01E01.MULTi.720p.NF.WEB-DL.DD5.1.x264-ARK01.mkv_fre_4.srt"
            ),
            Some("fr".to_string())
        );
        assert_eq!(
            detect_subtitle_file_language("Beacon.23.S01E01.fr.srt"),
            Some("fr".to_string())
        );
        assert_eq!(
            detect_subtitle_file_language("Unknown.Release.mkv_xyz_99.srt"),
            None
        );
    }

    #[test]
    fn test_search_subtitles_in_catalog_strict_language_filter() {
        let temp_dir =
            std::env::temp_dir().join(format!("dbsrt_test_catalog_lang_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let files = [
            "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_fre_5.srt",
            "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_ind_22.srt",
            "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_tha_24.srt",
            "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_vie_23.srt",
            "Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.mkv_xyz_99.srt",
        ];

        for f in &files {
            let p = temp_dir.join(f);
            std::fs::write(&p, "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n").unwrap();
        }

        let stems =
            vec!["Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI".to_string()];

        // Request French: strictly fre_5 must be returned, all others skipped!
        let fr_results =
            search_subtitles_in_catalog(&temp_dir, &stems, None, Some(1), Some(1), "fr");
        assert_eq!(fr_results.len(), 1, "Only 1 French subtitle returned");
        assert!(fr_results[0].1.to_string_lossy().contains("fre_5.srt"));
        assert_eq!(fr_results[0].2, "fr");

        // Request All: all 5 are returned
        let all_results =
            search_subtitles_in_catalog(&temp_dir, &stems, None, Some(1), Some(1), "all");
        assert_eq!(all_results.len(), 5, "All 5 subtitles returned under 'all'");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
