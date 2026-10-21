//! Subtitle search, candidate ranking, CAS download, and visual fingerprint recognition handlers.

use crate::constants::{
    DEFAULT_BASELINE_FPS, DEFAULT_FALLBACK_FPS, DEFAULT_RECOGNITION_SEARCH_DISTANCE,
    DEFAULT_SEARCH_DISTANCE, FINGERPRINT_PREVIEW_SAMPLE_COUNT, MILLIS_PER_SEC,
    MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT, MIN_SIGNIFICANT_SHIFT_MS, SECONDS_PER_MINUTE,
    SIGNIFICANT_SHIFT_THRESHOLD_MS,
};
use crate::dtos::{
    DownloadSubtitleRequest, DownloadSubtitleResponse, RecognizeRequest, RecognizeResponse,
    ScanMatchItem, ScanRequest, ScanResponse, SearchSubtitlesRequest, SearchSubtitlesResponse,
    SearchTimingStats, SubtitleCandidate, SubtitleSource,
};
use crate::services::catalog::{
    collect_subtitle_search_dirs, fetch_or_cache_remote_subtitle, lookup_certified_couplings,
    normalize_lang_code, search_subtitles_in_catalog,
};
use crate::services::phash::{
    calculate_sequence_shift_argmin, compute_media_phash_shift_multi, extract_media_query_sample,
    norm_alnum, parse_fingerprint_hashes, ExistingPhashData,
};
use crate::services::recognition::{
    identify_media_by_phash_multi_routed, identify_media_by_phash_multi_with_reader,
};
use crate::state::AppState;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dbsrt_core::{
    extract_embedded_subtitle, extract_frames_phash, inspect_container, probe_video,
    read_phash_file, MediaMetadata, SubtitleFile,
};
use dbsrt_index::MmapLibrary;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

pub async fn search_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SearchSubtitlesRequest>,
) -> impl IntoResponse {
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

                let endpoint = format!("{}/v1/recognize", remote_url.trim_end_matches('/'));
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_millis(
                        crate::constants::DEFAULT_RELAY_TIMEOUT_MS,
                    ))
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

pub async fn download_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DownloadSubtitleRequest>,
) -> Json<DownloadSubtitleResponse> {
    state
        .metrics
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let media_path = PathBuf::from(&payload.media_path);
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());

    let parsed_source = match payload.source.parse::<SubtitleSource>() {
        Ok(s) => s,
        Err(other) => {
            return Json(DownloadSubtitleResponse {
                success: false,
                output_srt_path: String::new(),
                shift_frames: 0,
                shift_ms: 0.0,
                confidence_percent: 0.0,
                source: other.clone(),
                language: payload.language,
                error: Some(format!("Unknown subtitle source: {}", other)),
            });
        }
    };

    let (mut srt_file, source_name) = match parsed_source {
        SubtitleSource::Container => {
            let idx: usize = payload
                .candidate_id
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            match extract_embedded_subtitle(&media_path, idx) {
                Ok(raw) => match SubtitleFile::parse(&raw) {
                    Ok(s) => (s, SubtitleSource::Container.as_str().to_string()),
                    Err(e) => {
                        return Json(DownloadSubtitleResponse {
                            success: false,
                            output_srt_path: String::new(),
                            shift_frames: 0,
                            shift_ms: 0.0,
                            confidence_percent: 0.0,
                            source: payload.source,
                            language: payload.language,
                            error: Some(format!("Failed to parse container subtitle: {:#}", e)),
                        });
                    }
                },
                Err(e) => {
                    return Json(DownloadSubtitleResponse {
                        success: false,
                        output_srt_path: String::new(),
                        shift_frames: 0,
                        shift_ms: 0.0,
                        confidence_percent: 0.0,
                        source: payload.source,
                        language: payload.language,
                        error: Some(format!(
                            "Failed to extract container subtitle track: {:#}",
                            e
                        )),
                    });
                }
            }
        }
        SubtitleSource::Certified | SubtitleSource::Catalog => {
            let cand_id = payload.candidate_id.as_deref().unwrap_or_default();
            let is_cas_hash = cand_id.starts_with("sub:sha256:") || cand_id.starts_with("sha256:");
            let clean_hash = cand_id
                .trim_start_matches("sub:sha256:")
                .trim_start_matches("sha256:")
                .trim();

            let mut fetched_file: Option<SubtitleFile> = None;

            if !is_cas_hash {
                let local_path = PathBuf::from(cand_id);
                if local_path.exists() {
                    if let Ok(s) = SubtitleFile::open(&local_path) {
                        fetched_file = Some(s);
                    }
                }
            } else if let Ok(reg) = state.coupling_registry.read() {
                if let Some((_coupling, sub)) = reg.find_subtitle_by_hash(clean_hash) {
                    let p = if Path::new(&sub.srt_path).is_absolute()
                        || Path::new(&sub.srt_path).exists()
                    {
                        PathBuf::from(&sub.srt_path)
                    } else {
                        subtitles_dir.join(&sub.srt_path)
                    };
                    if p.exists() {
                        if let Ok(s) = SubtitleFile::open(&p) {
                            fetched_file = Some(s);
                        }
                    }
                }
            }

            if fetched_file.is_none() && state.remote_api.is_some() {
                if let Some(ref remote_url) = state.remote_api {
                    let fetch_target = if is_cas_hash {
                        clean_hash.to_string()
                    } else {
                        Path::new(cand_id)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(cand_id)
                            .to_string()
                    };

                    let url = format!(
                        "{}/v1/subtitles/{}",
                        remote_url.trim_end_matches('/'),
                        fetch_target
                    );
                    state.push_log(
                        "REMOTE_CAS",
                        "INFO",
                        format!("Fetching subtitle from remote Subtitle Service: {}", url),
                    );

                    let client = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(5))
                        .build()
                        .unwrap_or_default();

                    match client.get(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            if let Ok(text) = resp.text().await {
                                match SubtitleFile::parse(&text) {
                                    Ok(s) => {
                                        fetched_file = Some(s);
                                        let cache_dir = state.cache.base_dir();
                                        let _ = std::fs::create_dir_all(cache_dir);
                                        let cache_file =
                                            cache_dir.join(format!("{}.srt", clean_hash));
                                        let _ = std::fs::write(&cache_file, &text);
                                    }
                                    Err(e) => {
                                        state.push_log(
                                            "REMOTE_CAS",
                                            "ERROR",
                                            format!(
                                                "Failed to parse remote subtitle content: {:#}",
                                                e
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            if fetched_file.is_none() {
                let path = PathBuf::from(cand_id);
                match SubtitleFile::open(&path) {
                    Ok(s) => fetched_file = Some(s),
                    Err(e) => {
                        return Json(DownloadSubtitleResponse {
                            success: false,
                            output_srt_path: String::new(),
                            shift_frames: 0,
                            shift_ms: 0.0,
                            confidence_percent: 0.0,
                            source: payload.source,
                            language: payload.language,
                            error: Some(format!(
                                "Failed to resolve subtitle (CAS hash '{}'): {:#}",
                                clean_hash, e
                            )),
                        });
                    }
                }
            }

            let s = fetched_file.unwrap();
            (s, parsed_source.as_str().to_string())
        }
        SubtitleSource::OpenSubtitles => {
            let sub_id = payload.candidate_id.clone().unwrap_or_default();
            let meta = MediaMetadata::parse_from_path(&media_path);
            let cand = dbsrt_core::RemoteSubtitleCandidate {
                provider: SubtitleSource::OpenSubtitles.as_str().to_string(),
                subtitle_id: sub_id.clone(),
                release_name: None,
                language: payload.language.clone(),
                download_url: String::new(),
                hearing_impaired: false,
                fps: None,
                match_score: 1.0,
            };

            match fetch_or_cache_remote_subtitle(&state, &meta, &payload.language, &cand).await {
                Ok(s) => (s, SubtitleSource::OpenSubtitles.as_str().to_string()),
                Err(e) => {
                    return Json(DownloadSubtitleResponse {
                        success: false,
                        output_srt_path: String::new(),
                        shift_frames: 0,
                        shift_ms: 0.0,
                        confidence_percent: 0.0,
                        source: payload.source,
                        language: payload.language,
                        error: Some(format!(
                            "Failed to download/parse subtitle ID '{}': {:#}",
                            sub_id, e
                        )),
                    });
                }
            }
        }
    };

    let mut shift_frames = 0i64;
    let mut shift_ms = 0.0f64;
    let mut confidence_percent = 0.0f64;

    if let Some(explicit_shift) = payload.shift_ms {
        if explicit_shift.abs() > MIN_SIGNIFICANT_SHIFT_MS {
            shift_ms = explicit_shift;
            let fps = probe_video(&media_path)
                .map(|m| m.fps)
                .unwrap_or(DEFAULT_FALLBACK_FPS);
            shift_frames = (shift_ms / MILLIS_PER_SEC * fps).round() as i64;
            confidence_percent = 100.0;
        }
    } else if payload.resync && media_path.exists() {
        let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_with_reader(
            &media_path,
            &phash_dirs,
            state.dbphash_reader.as_deref(),
            state.db_config.as_ref(),
            1.0,
            DEFAULT_RECOGNITION_SEARCH_DISTANCE,
        );
        if let Some((matched_stem, match_conf)) = matched {
            let (sf, sms, conf) = compute_media_phash_shift_multi(
                &media_path,
                &phash_dirs,
                &matched_stem,
                match_conf,
                target_fps,
                &existing_phash,
                12,
            );
            shift_frames = sf;
            shift_ms = sms;
            confidence_percent = conf;
        }
    }

    if shift_ms.abs() > 0.001 {
        srt_file.shift(shift_ms);
    }

    let out_dir = media_path
        .parent()
        .filter(|p| p.is_dir() && !p.to_string_lossy().is_empty())
        .unwrap_or_else(|| Path::new("/tmp"));
    let media_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "subtitles".to_string());

    let out_name = if shift_ms.abs() > 0.001 {
        format!("{}.synced.{}.srt", media_stem, payload.language)
    } else {
        format!("{}.{}.srt", media_stem, payload.language)
    };
    let out_path = out_dir.join(&out_name);

    if let Err(e) = srt_file.save(&out_path) {
        state.push_log(
            "DOWNLOAD",
            "ERROR",
            format!("Failed to write subtitle file to {:?}: {:#}", out_path, e),
        );
        return Json(DownloadSubtitleResponse {
            success: false,
            output_srt_path: String::new(),
            shift_frames: 0,
            shift_ms: 0.0,
            confidence_percent: 0.0,
            source: source_name,
            language: payload.language,
            error: Some(format!(
                "Failed to write subtitle file to {:?}: {:#}",
                out_path, e
            )),
        });
    }

    state.push_log(
        "DOWNLOAD",
        "INFO",
        format!(
            "Saved subtitle to {:?} (source='{}', lang='{}', shift={:.2}ms)",
            out_path.file_name().unwrap_or_default(),
            source_name,
            payload.language,
            shift_ms
        ),
    );

    Json(DownloadSubtitleResponse {
        success: true,
        output_srt_path: out_path.to_string_lossy().to_string(),
        shift_frames,
        shift_ms,
        confidence_percent,
        source: source_name,
        language: payload.language,
        error: None,
    })
}

pub async fn get_subtitle_by_hash_handler(
    AxumPath(hash): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    state
        .metrics
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let clean_hash = hash
        .trim_start_matches("sub:sha256:")
        .trim_start_matches("sha256:")
        .trim();

    state.push_log(
        "CAS",
        "INFO",
        format!("Requested subtitle by hash: '{}'", clean_hash),
    );

    let mut resolved_path: Option<(PathBuf, String)> = None;
    if let Ok(reg) = state.coupling_registry.read() {
        if let Some((_coupling, sub)) = reg.find_subtitle_by_hash(clean_hash) {
            let p = if Path::new(&sub.srt_path).is_absolute() || Path::new(&sub.srt_path).exists() {
                PathBuf::from(&sub.srt_path)
            } else {
                state.subtitles_dir.join(&sub.srt_path)
            };
            if p.exists() {
                resolved_path = Some((p, sub.name.clone()));
            }
        }
    }

    if resolved_path.is_none() {
        if let Some(ref cfg) = state.db_config {
            let _ = cfg.with_client(|client| {
                if let Ok(subs) = client.get_subtitles_for_media(None, None, None) {
                    for s in subs {
                        if s.subtitles_hash.eq_ignore_ascii_case(clean_hash)
                            || s.subtitles_hash.starts_with(clean_hash)
                            || clean_hash.starts_with(&s.subtitles_hash)
                        {
                            let p = s.srt_path.map(PathBuf::from).unwrap_or_else(|| {
                                state.subtitles_dir.join(format!("{}.srt", s.name))
                            });
                            if p.exists() {
                                resolved_path = Some((p, s.name));
                                break;
                            }
                        }
                    }
                }
                Ok(())
            });
        }
    }

    if resolved_path.is_none() {
        if let Ok(entries) = std::fs::read_dir(&state.subtitles_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let fname_opt = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string());
                if let Some(fname) = fname_opt {
                    if fname.ends_with(".srt")
                        && (fname.contains(clean_hash) || clean_hash.contains(&fname))
                    {
                        resolved_path = Some((p, fname));
                        break;
                    }
                }
            }
        }
    }

    match resolved_path {
        Some((path, filename)) => {
            let content = std::fs::read_to_string(&path).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to read subtitle file: {:#}", e),
                )
            })?;

            let res = axum::response::Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; charset=utf-8")
                .header("Cache-Control", "public, max-age=31536000, immutable")
                .header("ETag", format!("\"{}\"", clean_hash))
                .header(
                    "Content-Disposition",
                    format!("inline; filename=\"{}\"", filename),
                )
                .body(axum::body::Body::from(content))
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to build response: {:#}", e),
                    )
                })?;

            Ok(res)
        }
        None => Err((
            StatusCode::NOT_FOUND,
            format!("Subtitle resource with hash '{}' not found", clean_hash),
        )),
    }
}

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
        if let Some(ref reader) = state.dbphash_reader {
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

pub async fn scan_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ScanRequest>,
) -> Result<Json<ScanResponse>, (StatusCode, String)> {
    let query_media = PathBuf::from(&payload.query_media);
    if !query_media.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Query media not found: {:?}", query_media),
        ));
    }

    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());

    let start = Instant::now();
    let meta = probe_video(&query_media)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (1.0 * meta.fps).round() as usize;

    let query_hashes = extract_frames_phash(&query_media, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if query_hashes.is_empty() || query_hashes.iter().all(|&h| dbsrt_core::is_blank_phash(h)) {
        return Ok(Json(ScanResponse {
            total_files: 0,
            total_frames: 0,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            top_matches: Vec::new(),
        }));
    }

    let library = MmapLibrary::from_directories(&phash_dirs)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total_files = library.file_count();
    let total_frames = library.total_frames();

    let matches = library.recognize(&query_hashes, payload.search_distance);
    let elapsed_seconds = start.elapsed().as_secs_f64();

    let top_matches = matches
        .into_iter()
        .take(payload.limit)
        .map(|m| ScanMatchItem {
            media_name: m.media_name,
            matching_frames: m.nb_offsets_match,
            confidence_percent: m.confidence * 100.0,
        })
        .collect();

    Ok(Json(ScanResponse {
        total_files,
        total_frames,
        elapsed_seconds,
        top_matches,
    }))
}
