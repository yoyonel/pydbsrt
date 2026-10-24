//! Visual fingerprint recognition and media identification endpoint.

use crate::constants::{
    DEFAULT_BASELINE_FPS, DEFAULT_SEARCH_DISTANCE, MILLIS_PER_SEC,
    MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT, SIGNIFICANT_SHIFT_THRESHOLD_MS,
};
use crate::dtos::{RecognizeRequest, RecognizeResponse};
use crate::services::catalog::lookup_certified_couplings;
use crate::services::phash::{
    calculate_sequence_shift_argmin, norm_alnum, parse_fingerprint_hashes,
};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use dbsrt_core::{extract_frames_phash, probe_video, read_phash_file};
use dbsrt_index::MmapLibrary;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

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

    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());

    if let Some(ref fp) = payload.query_fingerprint {
        let hashes = parse_fingerprint_hashes(fp);
        let req_fps = payload.fps.unwrap_or(DEFAULT_BASELINE_FPS);
        let req_lang = payload.language.as_deref().unwrap_or("auto");
        let search_distance = if payload.search_distance > 0 {
            payload.search_distance
        } else {
            DEFAULT_SEARCH_DISTANCE
        };

        if hashes.is_empty() || hashes.iter().all(|&h| dbsrt_core::is_blank_phash(h)) {
            let elapsed = t_start.elapsed().as_secs_f64();
            state.metrics.total_latency_us.fetch_add(
                t_start.elapsed().as_micros() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            return Ok(Json(RecognizeResponse {
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
            }));
        }

        let mut matched: Option<(String, f64, String)> = None;
        let reader_guard = state.dbphash_reader.load();
        if let Some(ref reader) = *reader_guard {
            let m = reader.recognize(&hashes, search_distance);
            if let Some(top) = m.into_iter().next() {
                if top.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    matched = Some((
                        top.clean_title,
                        top.confidence,
                        "Tier 1 (RAM .dbphash MIH under 10ms)".to_string(),
                    ));
                }
            }
        }

        if matched.is_none() {
            if let Ok(lib) = MmapLibrary::from_directories(&phash_dirs) {
                let m = lib.recognize(&hashes, search_distance);
                if let Some(top) = m.into_iter().next() {
                    let conf_pct = top.confidence * 100.0;
                    if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                        matched = Some((
                            top.media_name,
                            conf_pct,
                            "Tier 2 (MmapLibrary Zero-DB Files)".to_string(),
                        ));
                    }
                }
            }
        }

        if matched.is_none() {
            if let Some(ref cfg) = state.db_config {
                if let Ok(Some((rec, conf))) =
                    cfg.with_client(|client| client.recognize_sample(&hashes, search_distance))
                {
                    let conf_pct = conf * 100.0;
                    if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                        matched = Some((
                            rec.name,
                            conf_pct,
                            "Tier 3 (PostgreSQL 17 SP-GiST BK-Tree)".to_string(),
                        ));
                    }
                }
            }
        }

        let elapsed = t_start.elapsed().as_secs_f64();
        state.metrics.total_latency_us.fetch_add(
            t_start.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );

        if let Some((media_match, confidence, tier)) = matched {
            let mut best_shift_pair = (0i64, 0.0f64, 0.0f64);
            let req_fps = if req_fps > 0.0 {
                req_fps
            } else {
                DEFAULT_BASELINE_FPS
            };

            for dir in &phash_dirs {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                            if name.ends_with(".phash") && !name.contains("_mid15s") {
                                let cand_stem = name.trim_end_matches(".phash");
                                let norm_m = norm_alnum(&media_match);
                                let norm_c = norm_alnum(cand_stem);
                                if norm_c.starts_with(&norm_m)
                                    || norm_m.starts_with(&norm_c)
                                    || name.contains(&media_match)
                                    || media_match.contains(cand_stem)
                                {
                                    if let Ok(ref_hashes) = read_phash_file(&p) {
                                        let (sf, match_conf) = calculate_sequence_shift_argmin(
                                            &hashes,
                                            &ref_hashes,
                                            req_fps,
                                        );
                                        if match_conf > best_shift_pair.2 {
                                            let sms = (sf as f64 / req_fps) * 1000.0;
                                            best_shift_pair = (sf, sms, match_conf);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let (shift_frames, shift_ms) =
                if best_shift_pair.2 >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    (Some(best_shift_pair.0), Some(best_shift_pair.1))
                } else {
                    (Some(0), Some(0.0))
                };

            let (cert_cands, _, _, _) = lookup_certified_couplings(
                &state,
                &state.subtitles_dir,
                &media_match,
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
                        c_synced.title =
                            format!("{} [Synced {:+.1}s]", c.title, sms / MILLIS_PER_SEC);
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

            return Ok(Json(RecognizeResponse {
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
            }));
        } else {
            return Ok(Json(RecognizeResponse {
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
            }));
        }
    }

    let media_path = PathBuf::from(&payload.media_path);
    if !media_path.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Media file not found: {:?}", media_path),
        ));
    }

    let meta =
        probe_video(&media_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (payload.nb_seconds * meta.fps).round() as usize;

    let hashes = extract_frames_phash(&media_path, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let media_name_search = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if hashes.is_empty() || hashes.iter().all(|&h| dbsrt_core::is_blank_phash(h)) {
        let elapsed = t_start.elapsed().as_secs_f64();
        state.metrics.total_latency_us.fetch_add(
            t_start.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        return Ok(Json(RecognizeResponse {
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
        }));
    }

    let library = MmapLibrary::from_directories(&phash_dirs)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let matches = library.recognize(&hashes, payload.search_distance);
    let timer_seconds = t_start.elapsed().as_secs_f64();
    state.metrics.total_latency_us.fetch_add(
        t_start.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );

    if let Some(top) = matches.first() {
        Ok(Json(RecognizeResponse {
            found: true,
            media_name_search,
            media_match: Some(top.media_name.clone()),
            nb_offsets_match: top.nb_offsets_match,
            confidence: top.confidence,
            tier: Some("Tier 2 (MmapLibrary Zero-DB Files)".to_string()),
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds,
        }))
    } else {
        Ok(Json(RecognizeResponse {
            found: false,
            media_name_search,
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            tier: None,
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds,
        }))
    }
}
