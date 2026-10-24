//! Subtitle search and candidate ranking handler.

use crate::constants::{
    DEFAULT_BASELINE_FPS, DEFAULT_FALLBACK_FPS, DEFAULT_RECOGNITION_SEARCH_DISTANCE,
    DEFAULT_SEARCH_DISTANCE, FINGERPRINT_PREVIEW_SAMPLE_COUNT, MILLIS_PER_SEC, SECONDS_PER_MINUTE,
    SIGNIFICANT_SHIFT_THRESHOLD_MS,
};
use crate::dtos::{
    RecognizeRequest, RecognizeResponse, SearchSubtitlesRequest, SearchSubtitlesResponse,
    SearchTimingStats, SubtitleCandidate, SubtitleSource,
};
use crate::services::catalog::{
    collect_subtitle_search_dirs, lookup_certified_couplings, normalize_lang_code,
    search_subtitles_in_catalog,
};
use crate::services::phash::{
    compute_media_phash_shift_multi, extract_media_query_sample, ExistingPhashData,
};
use crate::services::recognition::identify_media_by_phash_multi_routed;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use dbsrt_core::{inspect_container, probe_video, MediaMetadata};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

pub async fn search_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SearchSubtitlesRequest>,
) -> Json<SearchSubtitlesResponse> {
    let start_overall = Instant::now();
    state
        .metrics
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state
        .metrics
        .search_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let media_path = PathBuf::from(&payload.media_path);
    let include_sidecars = payload.subtitles_dir.is_none();
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());
    let mut candidates: Vec<SubtitleCandidate> = Vec::new();

    let mode = payload.search_mode.as_deref().unwrap_or("all");
    let use_phash = mode == "phash" || mode == "all" || mode == "auto";

    state.push_log(
        "SEARCH",
        "INFO",
        format!(
            "Request: mode='{}', lang='{}', media='{}'",
            mode,
            payload.language,
            media_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
        ),
    );

    let mut timing = SearchTimingStats::default();

    let mut matched_media: Option<String> = None;
    let mut phash_conf: f64 = 0.0;
    let mut target_fps: f64 = DEFAULT_FALLBACK_FPS;
    let mut existing_phash: Option<ExistingPhashData> = None;
    let mut query_sample_opt: Option<Vec<u64>> = None;

    if use_phash && media_path.exists() {
        if let Some(ref remote_url) = state.remote_api {
            let t0 = Instant::now();
            let (tfps, query_sample, e_phash) = extract_media_query_sample(
                &media_path,
                &phash_dirs,
                crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS,
            );
            target_fps = tfps;
            existing_phash = e_phash;
            query_sample_opt = Some(query_sample.clone());

            let extract_ms = t0.elapsed().as_secs_f64() * 1000.0;
            state.push_log(
                "RELAY",
                "INFO",
                format!(
                    "Extracted {} pHash frames from {:?} in {:.2}ms (fps={:.2})",
                    query_sample.len(),
                    media_path.file_name().unwrap_or_default(),
                    extract_ms,
                    target_fps
                ),
            );

            if !query_sample.is_empty() {
                let fp_strings: Vec<String> = query_sample
                    .iter()
                    .map(|h| format!("{:#018x}", h))
                    .collect();
                let remote_req = RecognizeRequest {
                    media_path: String::new(),
                    query_fingerprint: Some(fp_strings),
                    fps: Some(target_fps),
                    language: Some(payload.language.clone()),
                    client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    search_distance: 0,
                    nb_seconds: crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS,
                    phash_dir: None,
                };

                let relay_timeout_ms = std::env::var("DBSRT_RELAY_TIMEOUT_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(crate::constants::DEFAULT_RELAY_TIMEOUT_MS);

                let endpoint = format!("{}/v1/recognize", remote_url.trim_end_matches('/'));
                let client = reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_millis(
                        crate::constants::DEFAULT_RELAY_CONNECT_TIMEOUT_MS,
                    ))
                    .timeout(std::time::Duration::from_millis(relay_timeout_ms))
                    .build()
                    .unwrap_or_default();

                let t_net = Instant::now();
                match client.post(&endpoint).json(&remote_req).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let net_rtt_ms = t_net.elapsed().as_secs_f64() * 1000.0;
                        if let Ok(rec_resp) = resp.json::<RecognizeResponse>().await {
                            if rec_resp.found {
                                matched_media = rec_resp.media_match.clone();
                                phash_conf = rec_resp.confidence;
                                timing.confidence_percent = phash_conf;
                                timing.matching_tier = format!(
                                    "Remote Relay ({} via {})",
                                    rec_resp.tier.unwrap_or_else(|| "Tier 1 MIH".to_string()),
                                    remote_url
                                );
                                timing.phash_match_ms = net_rtt_ms;

                                state.push_log(
                                    "RELAY",
                                    "INFO",
                                    format!(
                                        "✓ Remote match: '{}' (conf={:.1}%, RTT={:.2}ms)",
                                        matched_media.as_deref().unwrap_or(""),
                                        phash_conf,
                                        net_rtt_ms
                                    ),
                                );

                                if let Some(remote_cands) = rec_resp.candidates {
                                    for mut c in remote_cands {
                                        c.details = Some(format!(
                                            "{} (Relayed from Remote Server in {:.1}ms)",
                                            c.details.as_deref().unwrap_or(""),
                                            net_rtt_ms
                                        ));
                                        candidates.push(c);
                                    }
                                }
                            } else {
                                state.push_log(
                                    "RELAY",
                                    "INFO",
                                    format!(
                                        "Remote server replied in {:.2}ms: not recognized",
                                        net_rtt_ms
                                    ),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        state
                            .metrics
                            .remote_errors
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        state.push_log(
                            "RELAY",
                            "WARN",
                            format!("✗ Remote relay error to {}: {:#}", endpoint, e),
                        );
                    }
                    _ => {
                        state
                            .metrics
                            .remote_errors
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        state.push_log(
                            "RELAY",
                            "WARN",
                            "✗ Remote relay returned unexpected non-200 status code",
                        );
                    }
                }
            }
        } else if phash_dirs.iter().any(|d| d.exists())
            || state.dbphash_path.is_some()
            || state.db_config.is_some()
        {
            let t0 = Instant::now();
            let (matched_routed, tfps, e_phash) = identify_media_by_phash_multi_routed(
                &media_path,
                &phash_dirs,
                state.dbphash_reader.as_deref(),
                state.db_config.as_ref(),
                1.0,
                DEFAULT_RECOGNITION_SEARCH_DISTANCE,
                payload.index_engine.as_deref(),
            );
            target_fps = tfps;
            existing_phash = e_phash;
            timing.phash_match_ms = t0.elapsed().as_secs_f64() * 1000.0;

            if let Some((_, ref hashes, fps)) = existing_phash {
                timing.phash_frames_count = hashes.len();
                if fps > 0.0 {
                    timing.video_duration_minutes =
                        (hashes.len() as f64 / fps) / SECONDS_PER_MINUTE as f64;
                    timing.query_sample_frames =
                        ((crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS * fps).round() as usize)
                            .min(hashes.len());
                }
            } else if media_path.exists() {
                if let Ok(meta) = probe_video(&media_path) {
                    timing.phash_frames_count = (meta.duration * meta.fps).round() as usize;
                    timing.video_duration_minutes = meta.duration / SECONDS_PER_MINUTE as f64;
                    timing.query_sample_frames = ((crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS
                        * meta.fps)
                        .round() as usize)
                        .max(1);
                }
            }

            if timing.query_sample_frames == 0 {
                timing.query_sample_frames = DEFAULT_BASELINE_FPS.round() as usize;
            }

            if let Some(ref reader) = state.dbphash_reader {
                let targets = reader.cuts_entries().len() + reader.sparse_entries().len();
                timing.index_targets_count = targets;
                timing.total_hamming_comparisons = timing.query_sample_frames * targets;
            }

            if timing.phash_match_ms > 0.0 && timing.total_hamming_comparisons > 0 {
                let total_us = timing.phash_match_ms * 1000.0;
                timing.us_per_frame = total_us / timing.query_sample_frames as f64;
                let comparisons_per_sec =
                    (timing.total_hamming_comparisons as f64) / (timing.phash_match_ms / 1000.0);
                timing.mfps_throughput = comparisons_per_sec / 1_000_000.0;
            }

            if let Some((m_name, conf, tier_label)) = matched_routed {
                matched_media = Some(m_name);
                phash_conf = conf;
                timing.confidence_percent = conf;
                timing.matching_tier = tier_label;
            }
        }
    }

    let t_meta = Instant::now();
    let is_phash_file = media_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("phash"))
        .unwrap_or(false);

    let (mut meta, moviehash) = if is_phash_file {
        let meta = MediaMetadata::parse(
            media_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(""),
        );
        (meta, None)
    } else {
        let meta = MediaMetadata::parse_from_path(&media_path);
        let mh = meta.moviehash.clone();
        (meta, mh)
    };
    timing.moviehash_ms = t_meta.elapsed().as_secs_f64() * 1000.0;

    let fingerprint_preview = query_sample_opt
        .as_deref()
        .or_else(|| {
            existing_phash
                .as_ref()
                .map(|(_, hashes, _)| hashes.as_slice())
        })
        .map(|hashes| {
            hashes
                .iter()
                .take(FINGERPRINT_PREVIEW_SAMPLE_COUNT)
                .map(|h| format!("{:#018x}", h))
                .collect::<Vec<_>>()
        });

    if mode == "phash" && matched_media.is_none() {
        timing.total_ms = start_overall.elapsed().as_secs_f64() * 1000.0;
        let accounted_ms = timing.phash_match_ms
            + timing.pg_lookup_ms
            + timing.json_lookup_ms
            + timing.container_ms
            + timing.disk_catalog_ms
            + timing.moviehash_ms;
        timing.overhead_ms = (timing.total_ms - accounted_ms).max(0.0);
        return Json(SearchSubtitlesResponse {
            media_path: payload.media_path,
            language: payload.language,
            matched_media: None,
            moviehash,
            fingerprint_preview,
            total_candidates: 0,
            candidates,
            timing,
        });
    }

    let mut seen_catalog_paths = std::collections::HashSet::new();
    let mut seen_filenames = std::collections::HashSet::new();
    if let Some(ref m_name) = matched_media {
        if candidates.is_empty() && !state.is_agent {
            let (cert_cands, pg_ms, json_ms, pg_cached) = lookup_certified_couplings(
                &state,
                &subtitles_dir,
                m_name,
                &payload.language,
                phash_conf,
            );
            timing.pg_lookup_ms = pg_ms;
            timing.json_lookup_ms = json_ms;
            timing.pg_cached = pg_cached;

            for mut c in cert_cands {
                if let Some(ref cid) = c.candidate_id {
                    seen_catalog_paths.insert(cid.clone());
                    if let Ok(canon) = Path::new(cid).canonicalize() {
                        seen_catalog_paths.insert(canon.to_string_lossy().to_string());
                    }
                }
                seen_filenames.insert(c.title.to_lowercase());

                let ref_stem = c.release.as_deref().unwrap_or(m_name);
                let (_sf, computed_shift_ms, _sconf) = compute_media_phash_shift_multi(
                    &media_path,
                    &phash_dirs,
                    ref_stem,
                    phash_conf,
                    target_fps,
                    &existing_phash,
                    DEFAULT_SEARCH_DISTANCE,
                );

                c.shift_ms = Some(0.0);
                candidates.push(c.clone());

                if computed_shift_ms.abs() > SIGNIFICANT_SHIFT_THRESHOLD_MS {
                    let mut c_synced = c.clone();
                    c_synced.source = "certified_synced".to_string();
                    c_synced.shift_ms = Some(computed_shift_ms);
                    c_synced.title = format!(
                        "{} [Synced {:+.1}s]",
                        c.title,
                        computed_shift_ms / MILLIS_PER_SEC
                    );
                    if let Some(ref d) = c.details {
                        c_synced.details = Some(format!(
                            "{} (Synchronized: {:+.2}s visual shift)",
                            d,
                            computed_shift_ms / MILLIS_PER_SEC
                        ));
                    }
                    candidates.push(c_synced);
                }
            }
        } else {
            for c in &candidates {
                if let Some(ref cid) = c.candidate_id {
                    seen_catalog_paths.insert(cid.clone());
                }
                seen_filenames.insert(c.title.to_lowercase());
            }
        }
    }

    if mode != "phash" && media_path.exists() {
        let t_cont = Instant::now();
        if let Ok(insp) = inspect_container(&media_path) {
            let req_norm = normalize_lang_code(&payload.language);
            for stream in insp.subtitle_streams {
                let stream_lang = stream.language.as_deref().unwrap_or("und");
                let stream_norm = normalize_lang_code(stream_lang);
                let lang_match = payload.language == "auto"
                    || payload.language == "all"
                    || stream_norm == req_norm
                    || stream_lang
                        .to_lowercase()
                        .starts_with(&payload.language.to_lowercase())
                    || payload
                        .language
                        .to_lowercase()
                        .starts_with(&stream_lang.to_lowercase());
                if lang_match {
                    candidates.push(SubtitleCandidate {
                        source: SubtitleSource::Container.as_str().to_string(),
                        language: stream_lang.to_string(),
                        title: stream
                            .title
                            .clone()
                            .unwrap_or_else(|| format!("Embedded #{}", stream.subtitle_index)),
                        release: media_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string()),
                        details: Some(format!(
                            "Track index {}, codec {}",
                            stream.subtitle_index, stream.codec_name
                        )),
                        candidate_id: Some(stream.subtitle_index.to_string()),
                        shift_ms: None,
                    });
                }
            }
        }
        timing.container_ms = t_cont.elapsed().as_secs_f64() * 1000.0;
    }

    let t_disk = Instant::now();
    let file_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut stems = Vec::new();
    if !file_stem.is_empty() {
        stems.push(file_stem.clone());
    }
    if let Some(ref m) = matched_media {
        if !stems.contains(m) {
            stems.push(m.clone());
        }
    }

    let catalog_query_title = if mode == "phash" {
        None
    } else {
        payload.query_title.as_deref()
    };

    let effective_season = payload.season.or(meta.season);
    let effective_episode = payload.episode.or(meta.episode);
    let search_dirs = collect_subtitle_search_dirs(&subtitles_dir, &media_path, include_sidecars);

    for dir in &search_dirs {
        for (_, path, resolved_lang) in search_subtitles_in_catalog(
            dir,
            &stems,
            catalog_query_title,
            effective_season,
            effective_episode,
            &payload.language,
        ) {
            let path_str = path.to_string_lossy().to_string();
            let canon_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let canon_str = canon_path.to_string_lossy().to_string();
            if !seen_catalog_paths.insert(path_str.clone()) || !seen_catalog_paths.insert(canon_str)
            {
                continue;
            }

            let cand_fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !seen_filenames.insert(cand_fname.to_lowercase()) {
                continue;
            }

            let details = if let Some(ref m) = matched_media {
                format!(
                    "pHash matched '{}' ({:.0}% conf) — {}",
                    m,
                    phash_conf,
                    path.display()
                )
            } else {
                path_str.clone()
            };
            candidates.push(SubtitleCandidate {
                source: SubtitleSource::Catalog.as_str().to_string(),
                language: resolved_lang,
                title: cand_fname,
                release: matched_media.clone().or_else(|| Some(file_stem.clone())),
                details: Some(details),
                candidate_id: Some(path_str),
                shift_ms: None,
            });
        }
    }
    timing.disk_catalog_ms = t_disk.elapsed().as_secs_f64() * 1000.0;

    if mode != "phash" {
        if let Some(ref qt) = payload.query_title {
            if !qt.trim().is_empty() {
                meta.clean_title = qt.trim().to_string();
            }
        }
        if let Some(s) = payload.season {
            meta.season = Some(s);
        }
        if let Some(ep) = payload.episode {
            meta.episode = Some(ep);
        }

        if let Ok(remote_cands) = state.provider.search(&meta, &payload.language).await {
            for cand in remote_cands {
                candidates.push(SubtitleCandidate {
                    source: SubtitleSource::OpenSubtitles.as_str().to_string(),
                    language: cand.language.clone(),
                    title: cand
                        .release_name
                        .clone()
                        .unwrap_or_else(|| meta.clean_title.clone()),
                    release: cand.release_name.clone(),
                    details: Some(format!(
                        "Match score: {:.2}, ID: {}",
                        cand.match_score, cand.subtitle_id
                    )),
                    candidate_id: Some(cand.subtitle_id.clone()),
                    shift_ms: None,
                });
            }
        }
    }

    let total_candidates = candidates.len();
    timing.total_ms = start_overall.elapsed().as_secs_f64() * 1000.0;
    state.metrics.total_latency_us.fetch_add(
        start_overall.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    let accounted_ms = timing.phash_match_ms
        + timing.pg_lookup_ms
        + timing.json_lookup_ms
        + timing.container_ms
        + timing.disk_catalog_ms
        + timing.moviehash_ms;
    timing.overhead_ms = (timing.total_ms - accounted_ms).max(0.0);

    state.push_log(
        "SEARCH",
        "INFO",
        format!(
            "Completed in {:.2}ms: matched='{}' (conf={:.1}%), candidates={}",
            timing.total_ms,
            matched_media.as_deref().unwrap_or("none"),
            phash_conf,
            total_candidates
        ),
    );

    Json(SearchSubtitlesResponse {
        media_path: payload.media_path,
        language: payload.language,
        matched_media,
        moviehash,
        fingerprint_preview,
        total_candidates,
        candidates,
        timing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{
        CertifiedMediaCoupling, CertifiedSubtitleRecord, MockSubtitleProvider, SubtitleCache,
    };
    use std::path::PathBuf;

    fn get_repo_root() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    fn test_app_state() -> Arc<AppState> {
        let root = get_repo_root();
        let phash_dir = root.join("tests/data");
        let srt_dir = root.join("data/SRT");
        let mock_provider = MockSubtitleProvider::new();
        mock_provider.add_candidate(
            "big buck bunny",
            dbsrt_core::RemoteSubtitleCandidate {
                provider: "mock".to_string(),
                subtitle_id: "mock_sub_1".to_string(),
                release_name: Some("Big.Buck.Bunny.720p".to_string()),
                language: "en".to_string(),
                download_url: "http://example.com/mock.srt".to_string(),
                hearing_impaired: false,
                fps: Some(25.0),
                match_score: 1.0,
            },
            "1\n00:00:01,000 --> 00:00:04,000\nMock Subtitle Text\n",
        );
        mock_provider.add_candidate(
            "agent query",
            dbsrt_core::RemoteSubtitleCandidate {
                provider: "mock".to_string(),
                subtitle_id: "mock_sub_2".to_string(),
                release_name: Some("Agent.Release".to_string()),
                language: "fr".to_string(),
                download_url: "http://example.com/agent.srt".to_string(),
                hearing_impaired: false,
                fps: Some(25.0),
                match_score: 1.0,
            },
            "1\n00:00:01,000 --> 00:00:04,000\nMock Agent Subtitle\n",
        );
        let mut state = AppState::new(
            phash_dir,
            srt_dir,
            Arc::new(mock_provider),
            SubtitleCache::default(),
        );
        state.extra_phash_dirs.push(root.join("data/__PHASH__"));
        Arc::new(state)
    }

    #[tokio::test]
    async fn test_search_missing_media_phash_early_return() {
        let state = test_app_state();
        let req = SearchSubtitlesRequest {
            media_path: "nonexistent_video_file_9999.mp4".to_string(),
            language: "fr".to_string(),
            search_mode: Some("phash".to_string()),
            query_title: None,
            season: None,
            episode: None,
            subtitles_dir: None,
            phash_dir: None,
            index_engine: None,
        };
        let res = search_handler(State(state), Json(req)).await;
        assert_eq!(res.0.total_candidates, 0);
        assert!(res.0.matched_media.is_none());
    }

    #[tokio::test]
    async fn test_search_mode_all_with_mock_provider() {
        let state = test_app_state();
        let req = SearchSubtitlesRequest {
            media_path: "nonexistent_video_query.mp4".to_string(),
            language: "en".to_string(),
            search_mode: Some("all".to_string()),
            query_title: Some("Big Buck Bunny".to_string()),
            season: Some(1),
            episode: Some(1),
            subtitles_dir: None,
            phash_dir: None,
            index_engine: None,
        };
        let res = search_handler(State(state), Json(req)).await;
        assert!(res.0.total_candidates > 0);
    }

    #[tokio::test]
    async fn test_search_phash_file_extension() {
        let root = get_repo_root();
        let phash_file = root.join("tests/data/big_buck_bunny_trailer_480p.phash");
        if !phash_file.exists() {
            return;
        }
        let state = test_app_state();
        let req = SearchSubtitlesRequest {
            media_path: phash_file.to_string_lossy().to_string(),
            language: "en".to_string(),
            search_mode: Some("catalog".to_string()),
            query_title: None,
            season: None,
            episode: None,
            subtitles_dir: Some(root.join("tests/data").to_string_lossy().to_string()),
            phash_dir: None,
            index_engine: None,
        };
        let res = search_handler(State(state), Json(req)).await;
        assert!(res.0.moviehash.is_none());
    }

    #[tokio::test]
    async fn test_search_real_media_phash_and_certified_couplings() {
        let root = get_repo_root();
        let video = root.join("tests/data/big_buck_bunny_trailer_480p.mp4");
        if !video.exists() {
            return;
        }
        let state = test_app_state();

        // Seed a coupling into registry so lookup_certified_couplings finds a match
        if let Ok(mut reg) = state.coupling_registry.write() {
            reg.couplings.push(CertifiedMediaCoupling {
                media_hash: "dummy_media_hash_bbb".to_string(),
                media_name: "big_buck_bunny_trailer_480p".to_string(),
                phash_filename: "big_buck_bunny_trailer_480p.phash".to_string(),
                fps: 25.0,
                frame_count: 50,
                subtitles: vec![CertifiedSubtitleRecord {
                    subtitles_hash: "dummy_sub_hash_bbb".to_string(),
                    name: "big_buck_bunny_trailer_480p.en".to_string(),
                    lang: "en".to_string(),
                    srt_path: root
                        .join("tests/data/big_buck_bunny_trailer_480p.en.srt")
                        .to_string_lossy()
                        .to_string(),
                    certified: true,
                    cue_count: 5,
                    sub_frames: vec![],
                }],
            });
        }

        let req = SearchSubtitlesRequest {
            media_path: video.to_string_lossy().to_string(),
            language: "en".to_string(),
            search_mode: Some("all".to_string()),
            query_title: None,
            season: None,
            episode: None,
            subtitles_dir: Some(root.join("tests/data").to_string_lossy().to_string()),
            phash_dir: Some(root.join("tests/data").to_string_lossy().to_string()),
            index_engine: None,
        };
        let res = search_handler(State(state), Json(req)).await;
        assert!(res.0.matched_media.is_some());
    }

    #[tokio::test]
    async fn test_search_container_inspection() {
        let root = get_repo_root();
        let srt_path = root.join("tests/data/big_buck_bunny_trailer_480p.en.srt");
        if !srt_path.exists() {
            return;
        }
        let tmp_mkv =
            std::env::temp_dir().join(format!("test_lookup_container_{}.mkv", std::process::id()));
        let status = std::process::Command::new("ffmpeg")
            .stdin(std::process::Stdio::null())
            .args([
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=320x240:d=1",
                "-f",
                "srt",
                "-i",
                srt_path.to_str().unwrap(),
                "-c:v",
                "mjpeg",
                "-c:s",
                "srt",
                "-metadata:s:s:0",
                "language=eng",
                tmp_mkv.to_str().unwrap(),
            ])
            .status();

        if let Ok(st) = status {
            if st.success() {
                let state = test_app_state();
                let req = SearchSubtitlesRequest {
                    media_path: tmp_mkv.to_string_lossy().to_string(),
                    language: "en".to_string(),
                    search_mode: Some("container".to_string()),
                    query_title: None,
                    season: None,
                    episode: None,
                    subtitles_dir: None,
                    phash_dir: None,
                    index_engine: None,
                };
                let res = search_handler(State(state), Json(req)).await;
                assert!(res
                    .0
                    .candidates
                    .iter()
                    .any(|c| c.source == SubtitleSource::Container.as_str()));
                let _ = std::fs::remove_file(tmp_mkv);
            }
        }
    }

    #[tokio::test]
    async fn test_search_remote_relay_unreachable() {
        let root = get_repo_root();
        let video = root.join("tests/data/big_buck_bunny_trailer_480p.mp4");
        if !video.exists() {
            return;
        }
        let mut state = (*test_app_state()).clone();
        state.remote_api = Some("http://127.0.0.1:9".to_string()); // Unreachable port
        let state = Arc::new(state);

        let req = SearchSubtitlesRequest {
            media_path: video.to_string_lossy().to_string(),
            language: "en".to_string(),
            search_mode: Some("phash".to_string()),
            query_title: None,
            season: None,
            episode: None,
            subtitles_dir: None,
            phash_dir: None,
            index_engine: None,
        };
        let _ = search_handler(State(state), Json(req)).await;
    }

    #[tokio::test]
    async fn test_search_agent_mode_candidate_filtering() {
        let mut state = (*test_app_state()).clone();
        state.is_agent = true;
        let state = Arc::new(state);

        let req = SearchSubtitlesRequest {
            media_path: "nonexistent_agent_file.mp4".to_string(),
            language: "fr".to_string(),
            search_mode: Some("all".to_string()),
            query_title: Some("Agent Query".to_string()),
            season: None,
            episode: None,
            subtitles_dir: None,
            phash_dir: None,
            index_engine: None,
        };
        let res = search_handler(State(state), Json(req)).await;
        assert!(res.0.total_candidates > 0);
    }
}
