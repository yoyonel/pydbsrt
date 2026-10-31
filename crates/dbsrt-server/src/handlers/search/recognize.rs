//! Visual fingerprint recognition and media identification endpoint.

use crate::constants::{
    DEFAULT_BASELINE_FPS, DEFAULT_SEARCH_DISTANCE, MILLIS_PER_SEC,
    MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT, SIGNIFICANT_SHIFT_THRESHOLD_MS,
};
use crate::dtos::{RecognizeRequest, RecognizeResponse, SubtitleCandidate};
use crate::services::catalog::lookup_certified_couplings;
use crate::services::phash::{
    calculate_sequence_shift_argmin, norm_alnum, parse_fingerprint_hashes,
};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use dbsrt_core::{extract_frames_phash, probe_video, read_phash_file};
use dbsrt_index::{rank_media_matches, DbphashMediaMatch, MmapLibrary};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Matches a sequence of pHash hashes against Tier 1 (.dbphash), Tier 1-Delta (RAM), Tier 2 (MmapLibrary), and Tier 3 (PostgreSQL).
///
/// # Arguments
/// * `state` - Application state containing readers, delta index, and DB config.
/// * `hashes` - Slice of 64-bit perceptual hash values.
/// * `search_distance` - Maximum allowed Hamming distance.
/// * `phash_dirs` - Reference directories storing .phash files.
///
/// # Returns
/// * `Option<(String, f64, String)>` - Matched media stem, confidence percentage (0-100), and tier description.
///
/// # Errors
/// * Guarantee: Never returns Err; unhandled errors fall through to subsequent tiers.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Queries in-memory Tier 1 (`dbphash_reader`) and Tier 1-Delta (`delta_index`).
/// 2. Sorts candidates by confidence, then match precision.
/// 3. Falls through to Tier 2 (`MmapLibrary`) across `phash_dirs`.
/// 4. Falls through to Tier 3 (PostgreSQL 17 SP-GiST BK-Tree) if configured.
pub fn match_fingerprint_multitier(
    state: &AppState,
    hashes: &[u64],
    search_distance: u32,
    phash_dirs: &[PathBuf],
) -> Option<(String, f64, String)> {
    if dbsrt_core::is_blank_sequence(hashes) {
        return None;
    }

    let mut candidates: Vec<(DbphashMediaMatch, &str)> = Vec::new();

    let reader_guard = state.dbphash_reader.load();
    if let Some(ref reader) = *reader_guard {
        for m in reader.recognize(hashes, search_distance) {
            candidates.push((m, "Tier 1 (RAM .dbphash MIH under 10ms)"));
        }
    }

    let delta_snap = {
        if let Ok(delta) = state.delta_index.read() {
            if !delta.is_empty() {
                Some(delta.snapshot())
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(delta) = delta_snap {
        for m in delta.recognize(hashes, search_distance) {
            candidates.push((m, "Tier 1-Delta (RAM DeltaIndex MIH under 1ms)"));
        }
    }

    candidates.sort_by(|a, b| rank_media_matches(&a.0, &b.0));

    if let Some((m, tier)) = candidates.into_iter().next() {
        if m.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
            return Some((m.clean_title, m.confidence, tier.to_string()));
        }
    }

    if let Ok(lib) = MmapLibrary::from_directories(phash_dirs) {
        let m = lib.recognize(hashes, search_distance);
        if let Some(top) = m.into_iter().next() {
            let conf_pct = top.confidence * 100.0;
            if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return Some((
                    top.media_name,
                    conf_pct,
                    "Tier 2 (MmapLibrary Zero-DB Files)".to_string(),
                ));
            }
        }
    }

    if let Some(ref cfg) = state.db_config {
        if let Ok(Some((rec, conf))) =
            cfg.with_client(|client| client.recognize_sample(hashes, search_distance))
        {
            let conf_pct = conf * 100.0;
            if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return Some((
                    rec.name,
                    conf_pct,
                    "Tier 3 (PostgreSQL 17 SP-GiST BK-Tree)".to_string(),
                ));
            }
        }
    }

    None
}

/// Scans reference .phash files in directories to calculate visual sequence shift against query hashes.
///
/// # Arguments
/// * `hashes` - Query visual fingerprint sequence.
/// * `media_match` - Matched media title or stem.
/// * `req_fps` - Query framerate.
/// * `phash_dirs` - Directories storing reference .phash files.
///
/// # Returns
/// * `(i64, f64, f64)` - Tuple of `(shift_frames, shift_ms, match_confidence)`.
///
/// # Errors
/// * Guarantee: Never panics or returns Err; defaults to 0 on failure.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Iterates reference `.phash` files matching `media_match` name.
/// 2. Executes `calculate_sequence_shift_argmin` against each reference.
/// 3. Retains reference with highest sequence alignment confidence.
pub fn calculate_best_sequence_shift(
    hashes: &[u64],
    media_match: &str,
    req_fps: f64,
    phash_dirs: &[PathBuf],
) -> (i64, f64, f64) {
    let mut best_shift = (0i64, 0.0f64, 0.0f64);
    let fps = if req_fps > 0.0 {
        req_fps
    } else {
        DEFAULT_BASELINE_FPS
    };

    for dir in phash_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".phash") && !name.contains("_mid15s") {
                        let cand_stem = name.trim_end_matches(".phash");
                        let norm_m = norm_alnum(media_match);
                        let norm_c = norm_alnum(cand_stem);
                        if norm_c.starts_with(&norm_m)
                            || norm_m.starts_with(&norm_c)
                            || name.contains(media_match)
                            || media_match.contains(cand_stem)
                        {
                            if let Ok(ref_hashes) = read_phash_file(&p) {
                                let (sf, match_conf) =
                                    calculate_sequence_shift_argmin(hashes, &ref_hashes, fps);
                                if match_conf > best_shift.2 {
                                    let sms = (sf as f64 / fps) * 1000.0;
                                    best_shift = (sf, sms, match_conf);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    best_shift
}

/// Queries certified couplings and enriches candidates with visual shifts.
///
/// # Arguments
/// * `state` - Application state with coupling registry.
/// * `media_match` - Matched media release name.
/// * `req_lang` - Requested ISO language code or "auto".
/// * `confidence` - Recognition confidence percentage.
/// * `shift_ms` - Computed visual shift in milliseconds.
///
/// # Returns
/// * `Vec<CandidateSubtitleRecord>` - Formatted subtitle candidates including raw and synced pairs.
///
/// # Errors
/// * Guarantee: Never panics.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Calls `lookup_certified_couplings`.
/// 2. For each certified candidate, emits raw candidate with 0 shift.
/// 3. If `shift_ms` is significant (> `SIGNIFICANT_SHIFT_THRESHOLD_MS`), generates a `certified_synced` candidate.
pub fn resolve_certified_candidates_with_shift(
    state: &AppState,
    media_match: &str,
    req_lang: &str,
    confidence: f64,
    shift_ms: Option<f64>,
) -> Vec<SubtitleCandidate> {
    let (cert_cands, _, _, _) = lookup_certified_couplings(
        state,
        &state.subtitles_dir,
        media_match,
        None,
        None,
        req_lang,
        confidence,
    );

    let mut final_cands = Vec::new();
    for c in cert_cands {
        let mut c_raw = c.clone();
        c_raw.shift_ms = Some(0.0);
        final_cands.push(c_raw);

        if let Some(sms) = shift_ms {
            if sms.abs() > SIGNIFICANT_SHIFT_THRESHOLD_MS {
                let mut c_synced = c.clone();
                c_synced.source = "certified_synced".to_string();
                c_synced.shift_ms = Some(sms);
                c_synced.title = format!("{} [Synced {:+.1}s]", c.title, sms / MILLIS_PER_SEC);
                if let Some(ref d) = c.details {
                    c_synced.details = Some(format!(
                        "{} (Synchronized: {:+.2}s visual shift)",
                        d,
                        sms / MILLIS_PER_SEC
                    ));
                }
                final_cands.push(c_synced);
            }
        }
    }

    final_cands
}

/// Processes a visual fingerprint recognition query from pre-extracted hashes.
///
/// # Arguments
/// * `state` - Application state.
/// * `hashes` - Slice of query pHash hashes.
/// * `payload` - Parsed recognize request parameters.
/// * `phash_dirs` - Configured directories storing .phash files.
/// * `elapsed` - Duration elapsed since request start.
///
/// # Returns
/// * `RecognizeResponse` - Structured recognition outcome.
///
/// # Errors
/// * Guarantee: Never returns Err.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Rejects blank hashes immediately.
/// 2. Performs multi-tier recognition via `match_fingerprint_multitier`.
/// 3. Computes sequence shift via `calculate_best_sequence_shift`.
/// 4. Enriches certified couplings via `resolve_certified_candidates_with_shift`.
pub fn recognize_from_fingerprint(
    state: &AppState,
    hashes: &[u64],
    payload: &RecognizeRequest,
    phash_dirs: &[PathBuf],
    elapsed: f64,
) -> RecognizeResponse {
    let req_fps = payload.fps.unwrap_or(DEFAULT_BASELINE_FPS);
    let req_lang = payload.language.as_deref().unwrap_or("auto");
    let search_distance = if payload.search_distance > 0 {
        payload.search_distance
    } else {
        DEFAULT_SEARCH_DISTANCE
    };

    if hashes.is_empty() || hashes.iter().all(|&h| dbsrt_core::is_blank_phash(h)) {
        return RecognizeResponse {
            found: false,
            media_name_search: "fingerprint_query".to_string(),
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            tier: None,
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds: elapsed,
        };
    }

    let matched = match_fingerprint_multitier(state, hashes, search_distance, phash_dirs);

    if let Some((media_match, confidence, tier)) = matched {
        let best_shift = calculate_best_sequence_shift(hashes, &media_match, req_fps, phash_dirs);
        let (shift_frames, shift_ms) = if best_shift.2 >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
            (Some(best_shift.0), Some(best_shift.1))
        } else {
            (Some(0), Some(0.0))
        };

        let final_cands = resolve_certified_candidates_with_shift(
            state,
            &media_match,
            req_lang,
            confidence,
            shift_ms,
        );

        RecognizeResponse {
            found: true,
            media_name_search: format!("fingerprint:{}frames", hashes.len()),
            media_match: Some(media_match),
            nb_offsets_match: hashes.len(),
            confidence,
            tier: Some(tier),
            shift_frames,
            shift_ms,
            candidates: Some(final_cands),
            timer_seconds: elapsed,
        }
    } else {
        RecognizeResponse {
            found: false,
            media_name_search: format!("fingerprint:{}frames", hashes.len()),
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            tier: None,
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds: elapsed,
        }
    }
}

/// Probes a video file on disk, extracts sample visual frames, and matches using MmapLibrary.
///
/// # Arguments
/// * `media_path` - Path to target video file.
/// * `payload` - Recognize request options (nb_seconds, search_distance).
/// * `phash_dirs` - Configured directories storing .phash files.
/// * `elapsed` - Duration elapsed since request start.
///
/// # Returns
/// * `Result<RecognizeResponse, (StatusCode, String)>` - Recognition result or HTTP error tuple.
///
/// # Errors
/// * Returns `Err(StatusCode::BAD_REQUEST)` if media file does not exist.
/// * Returns `Err(StatusCode::INTERNAL_SERVER_ERROR)` if FFmpeg probe/extraction fails or library cannot be opened.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Probes video duration and fps using `probe_video`.
/// 2. Extracts frames around middle of file using `extract_frames_phash`.
/// 3. Queries `MmapLibrary` across `phash_dirs`.
pub fn recognize_from_media_file(
    media_path: &Path,
    payload: &RecognizeRequest,
    phash_dirs: &[PathBuf],
    elapsed: f64,
) -> Result<RecognizeResponse, (StatusCode, String)> {
    if !media_path.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Media file not found: {:?}", media_path),
        ));
    }

    let meta =
        probe_video(media_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (payload.nb_seconds * meta.fps).round() as usize;

    let hashes = extract_frames_phash(media_path, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let media_name_search = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if hashes.is_empty() || hashes.iter().all(|&h| dbsrt_core::is_blank_phash(h)) {
        return Ok(RecognizeResponse {
            found: false,
            media_name_search,
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            tier: None,
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds: elapsed,
        });
    }

    let library = MmapLibrary::from_directories(phash_dirs)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let matches = library.recognize(&hashes, payload.search_distance);

    if let Some(top) = matches.first() {
        Ok(RecognizeResponse {
            found: true,
            media_name_search,
            media_match: Some(top.media_name.clone()),
            nb_offsets_match: top.nb_offsets_match,
            confidence: top.confidence,
            tier: Some("Tier 2 (MmapLibrary Zero-DB Files)".to_string()),
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds: elapsed,
        })
    } else {
        Ok(RecognizeResponse {
            found: false,
            media_name_search,
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            tier: None,
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds: elapsed,
        })
    }
}

/// Axum handler for `POST /v1/recognize`: identifies a video release by its visual pHash fingerprint.
///
/// # Arguments
/// - `state`: Shared server state container (`Arc<AppState>`).
/// - `payload`: JSON request specifying `media_path`, optional `query_fingerprint`, `phash_dir`.
///
/// # Returns
/// - `Ok(Json<RecognizeResponse>)`: Matched release name, confidence score, tier index engine used, and latency.
/// - `Err((StatusCode, String))`: HTTP 400 if media cannot be probed, opened, or fingerprinted.
///
/// # Data Flow & Invariants
/// 1. Records request metrics and determines effective phash directories.
/// 2. If client supplied `query_fingerprint`, resolves via `recognize_from_fingerprint`.
/// 3. Otherwise extracts frames from file on disk via `recognize_from_media_file`.
/// 4. Records total latency and returns structured response.
pub async fn recognize_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RecognizeRequest>,
) -> Result<Json<RecognizeResponse>, (StatusCode, String)> {
    let t_start = Instant::now();
    state
        .metrics
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state
        .metrics
        .recognize_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let override_phash_dir = payload.phash_dir.as_deref().map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());

    let response = if let Some(ref fp) = payload.query_fingerprint {
        let hashes = parse_fingerprint_hashes(fp);
        let elapsed = t_start.elapsed().as_secs_f64();
        recognize_from_fingerprint(&state, &hashes, &payload, &phash_dirs, elapsed)
    } else {
        let media_path = PathBuf::from(&payload.media_path);
        let elapsed = t_start.elapsed().as_secs_f64();
        recognize_from_media_file(&media_path, &payload, &phash_dirs, elapsed)?
    };

    state.metrics.total_latency_us.fetch_add(
        t_start.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};

    fn test_app_state() -> Arc<AppState> {
        let phash_dir = PathBuf::from("tests/data");
        let srt_dir = PathBuf::from("data/SRT");
        Arc::new(AppState::new(
            phash_dir,
            srt_dir,
            Arc::new(OpenSubtitlesProvider::new(None)),
            SubtitleCache::default(),
        ))
    }

    #[tokio::test]
    async fn test_recognize_missing_media_file() {
        let state = test_app_state();
        let req = RecognizeRequest {
            media_path: "nonexistent_movie_9999.mp4".to_string(),
            query_fingerprint: None,
            fps: None,
            language: None,
            client_version: None,
            search_distance: 6,
            nb_seconds: 1.0,
            phash_dir: None,
        };
        let res = recognize_handler(State(state), Json(req)).await;
        assert!(res.is_err());
        let (status, msg) = res.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("Media file not found"));
    }

    #[tokio::test]
    async fn test_recognize_blank_fingerprint() {
        let state = test_app_state();
        let req = RecognizeRequest {
            media_path: String::new(),
            query_fingerprint: Some(vec![
                "0x0000000000000000".to_string(),
                "0x0000000000000000".to_string(),
            ]),
            fps: Some(25.0),
            language: Some("fr".to_string()),
            client_version: None,
            search_distance: 6,
            nb_seconds: 1.0,
            phash_dir: None,
        };
        let res = recognize_handler(State(state), Json(req)).await.unwrap();
        assert!(!res.0.found);
        assert_eq!(res.0.confidence, 0.0);
    }

    #[test]
    fn test_match_static_textured_scene_reaches_index() {
        const STATIC_SCENE_PHASH: u64 = 0xAAAA_AAAA_5555_5555;
        const STATIC_SCENE_FRAMES: usize = 250;
        const QUERY_FRAMES: usize = 25;
        const FPS: f64 = 25.0;
        let state = test_app_state();
        let indexed = vec![STATIC_SCENE_PHASH; STATIC_SCENE_FRAMES];
        state.delta_index.write().unwrap().ingest_raw_frames(
            "Static.Scene.2020.1080p.mkv",
            &indexed,
            FPS,
        );
        // A static but textured query must not be rejected before the index lookup.
        let query = vec![STATIC_SCENE_PHASH; QUERY_FRAMES];
        assert!(dbsrt_core::is_static_sequence(&query));
        let hit = match_fingerprint_multitier(&state, &query, DEFAULT_SEARCH_DISTANCE, &[]);
        assert!(hit.is_some(), "static textured scene must reach the index");
    }

    #[tokio::test]
    async fn test_recognize_zero_entropy_identical_frames() {
        let state = test_app_state();
        // 5 consecutive identical hashes with 32 ones (exact real-world black/intro studio hash)
        let identical_black_hashes = vec![
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
        ];
        let req = RecognizeRequest {
            media_path: String::new(),
            query_fingerprint: Some(identical_black_hashes),
            fps: Some(25.0),
            language: Some("fr".to_string()),
            client_version: None,
            search_distance: 6,
            nb_seconds: 1.0,
            phash_dir: None,
        };
        let res = recognize_handler(State(state), Json(req)).await.unwrap();
        assert!(!res.0.found, "Zero-entropy identical frames must not match");
        assert_eq!(res.0.confidence, 0.0);
        assert_eq!(res.0.media_match, None);
    }
}
