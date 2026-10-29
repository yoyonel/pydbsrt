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

/// Axum handler for `POST /v1/search`: discovers, matches, and ranks subtitles for a video.
///
/// # Arguments
/// - `state`: Shared server state container (`Arc<AppState>`).
/// - `payload`: JSON request specifying `media_path`, `language`, `search_mode`, `title`, `season`, `episode`.
///
/// # Returns
/// `Json<SearchSubtitlesResponse>` containing ranked subtitle candidates, visual shift, and timing diagnostics.
///
/// # Data Flow & Invariants
/// 1. Extracts video container metadata (duration, FPS, embedded tracks).
/// 2. If `search_mode == "phash"` or `"all"`, samples visual fingerprints from target video.
/// 3. Identifies reference media release in Tier-1 RAM index or PostgreSQL.
/// 4. Queries certified ground-truth subtitle couplings using episodic isolation constraints.
/// 5. Computes visual frame offset and millisecond shift (`vshift_ms`) between reference and target stream.
/// 6. Collects catalog and OpenSubtitles fallback candidates, scoring and ranking them before returning.
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
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let override_phash_dir = payload.phash_dir.as_deref().map(PathBuf::from);
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

    let (matched_media, phash_conf, target_fps, existing_phash, query_sample_opt) =
        execute_phash_recognition(
            &state,
            &payload,
            &media_path,
            &phash_dirs,
            use_phash,
            &mut candidates,
            &mut timing,
        )
        .await;

    let (mut meta, moviehash, fingerprint_preview) = extract_target_metadata_and_preview(
        &media_path,
        query_sample_opt.as_deref(),
        existing_phash.as_ref(),
        &mut timing,
    );

    let effective_season = payload.season.or(meta.season);
    let effective_episode = payload.episode.or(meta.episode);

    let mut seen_catalog_paths = std::collections::HashSet::new();
    let mut seen_filenames = std::collections::HashSet::new();

    if let Some(ref m_name) = matched_media {
        match_certified_couplings_and_shifts(
            &state,
            &media_path,
            &subtitles_dir,
            &phash_dirs,
            m_name,
            phash_conf,
            target_fps,
            &existing_phash,
            effective_season,
            effective_episode,
            &payload.language,
            &mut candidates,
            &mut seen_catalog_paths,
            &mut seen_filenames,
            &mut timing,
        );
    }

    if mode != "phash" && media_path.exists() {
        extract_embedded_container_subtitles(
            &media_path,
            &payload.language,
            &mut candidates,
            &mut timing,
        );
    }

    let catalog_query_title = if mode == "phash" {
        None
    } else {
        payload.query_title.as_deref()
    };

    search_disk_catalog_subtitles(
        &media_path,
        &subtitles_dir,
        include_sidecars,
        matched_media.as_deref(),
        phash_conf,
        effective_season,
        effective_episode,
        catalog_query_title,
        &payload.language,
        &mut candidates,
        &mut seen_catalog_paths,
        &mut seen_filenames,
        &mut timing,
    );

    fetch_remote_provider_and_filter(
        &state,
        mode,
        &payload,
        &mut meta,
        effective_season,
        effective_episode,
        &mut candidates,
    )
    .await;

    finalize_search_response(
        payload,
        matched_media,
        moviehash,
        fingerprint_preview,
        candidates,
        timing,
        start_overall,
        &state,
        phash_conf,
    )
}

struct RemoteRelayProcessingContext<'a> {
    state: &'a AppState,
    remote_url: &'a str,
    media_path: &'a Path,
    candidates: &'a mut Vec<SubtitleCandidate>,
    timing: &'a mut SearchTimingStats,
}

/// True when the local file stem and the remote match title contain one another
/// (case-insensitive). Both must be non-empty: `contains("")` is always true.
fn stem_and_title_overlap(file_stem: &str, match_title: &str) -> bool {
    let stem = file_stem.trim().to_lowercase();
    let title = match_title.trim().to_lowercase();
    !stem.is_empty() && !title.is_empty() && (title.contains(&stem) || stem.contains(&title))
}

/// Processes upstream remote recognition response, verifying candidate presence and title consistency.
///
/// # Arguments
/// - `ctx`: Mutable relay processing context containing state, URL, media path, candidates, and timing stats.
/// - `rec_resp`: Upstream recognition response payload.
/// - `net_rtt_ms`: Network round-trip latency in milliseconds.
///
/// # Returns
/// Tuple containing `(matched_media, phash_confidence)`.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// 1. Checks `rec_resp.found`; logs non-match and returns `(None, 0.0)` if not found.
/// 2. If remote returned 0 candidates and matched title is disparate from target media stem,
///    discards spurious cross-media collision to prevent false relabeling.
/// 3. Updates search timing metrics, logs recognition success, and decorates candidate details with relay latency.
fn process_remote_recognition_result(
    ctx: &mut RemoteRelayProcessingContext<'_>,
    rec_resp: RecognizeResponse,
    net_rtt_ms: f64,
) -> (Option<String>, f64) {
    if !rec_resp.found {
        ctx.state.push_log(
            "RELAY",
            "INFO",
            format!(
                "Remote server replied in {:.2}ms: not recognized",
                net_rtt_ms
            ),
        );
        return (None, 0.0);
    }

    let file_stem = ctx
        .media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let has_cands = rec_resp
        .candidates
        .as_ref()
        .map(|c| !c.is_empty())
        .unwrap_or(false);
    let match_title = rec_resp.media_match.as_deref().unwrap_or("");
    let stem_matches = stem_and_title_overlap(&file_stem, match_title);

    if !has_cands && !stem_matches {
        ctx.state.push_log(
            "RELAY",
            "WARN",
            format!(
                "Discarded unverified remote match '{}' (0 candidates, disparate stem '{}')",
                match_title, file_stem
            ),
        );
        return (None, 0.0);
    }

    let matched_media = rec_resp.media_match;
    let phash_conf = rec_resp.confidence;
    ctx.timing.confidence_percent = phash_conf;
    ctx.timing.matching_tier = format!(
        "Remote Relay ({} via {})",
        rec_resp.tier.unwrap_or_else(|| "Tier 1 MIH".to_string()),
        ctx.remote_url
    );
    ctx.timing.phash_match_ms = net_rtt_ms;

    ctx.state.push_log(
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
            ctx.candidates.push(c);
        }
    }

    (matched_media, phash_conf)
}

/// Dispatches visual recognition request to an upstream remote DBS-RT server.
///
/// # Arguments
/// - `state`: Shared server state container (`AppState`).
/// - `media_path`: File path to the video stream being queried.
/// - `phash_dirs`: Search directories holding cached pHash fingerprints.
/// - `remote_url`: Base URL of the upstream DBS-RT server.
/// - `language`: Target subtitle language code.
/// - `candidates`: Mutable vector of candidates to receive relayed results.
/// - `timing`: Mutable search timing statistics to record RTT.
///
/// # Returns
/// Tuple containing `(matched_media, phash_confidence, target_fps, existing_phash, query_sample)`.
///
/// # Errors
/// None. Relay connectivity or HTTP protocol errors are logged as functional warnings and increment metrics.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Extracts visual sample frames from target video, constructs [`RecognizeRequest`], executes
/// asynchronous HTTP POST to upstream `/v1/recognize` within timeout, unpacks response candidates
/// and decorates them with relay round-trip latency.
async fn execute_remote_relay_recognition(
    state: &AppState,
    media_path: &Path,
    phash_dirs: &[PathBuf],
    remote_url: &str,
    language: &str,
    candidates: &mut Vec<SubtitleCandidate>,
    timing: &mut SearchTimingStats,
) -> (
    Option<String>,
    f64,
    f64,
    Option<ExistingPhashData>,
    Option<Vec<u64>>,
) {
    let t0 = Instant::now();
    let (target_fps, query_sample, existing_phash) = extract_media_query_sample(
        media_path,
        phash_dirs,
        crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS,
    );

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

    let mut matched_media = None;
    let mut phash_conf = 0.0;
    let query_sample_opt = Some(query_sample.clone());

    if !dbsrt_core::is_blank_sequence(&query_sample) {
        let fp_strings: Vec<String> = query_sample
            .iter()
            .map(|h| format!("{:#018x}", h))
            .collect();
        let remote_req = RecognizeRequest {
            media_path: String::new(),
            query_fingerprint: Some(fp_strings),
            fps: Some(target_fps),
            language: Some(language.to_string()),
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
        if let Some((rec_resp, net_rtt_ms)) =
            dispatch_remote_relay_call(state, &endpoint, &remote_req, relay_timeout_ms).await
        {
            let mut ctx = RemoteRelayProcessingContext {
                state,
                remote_url,
                media_path,
                candidates,
                timing,
            };
            let (m_opt, conf) = process_remote_recognition_result(&mut ctx, rec_resp, net_rtt_ms);
            matched_media = m_opt;
            phash_conf = conf;
        }
    }

    (
        matched_media,
        phash_conf,
        target_fps,
        existing_phash,
        query_sample_opt,
    )
}

/// Dispatches HTTP POST `/v1/recognize` to remote relay endpoint and unpacks JSON response.
///
/// # Arguments
/// - `state`: Shared server state container (`AppState`).
/// - `endpoint`: Target HTTP endpoint URL.
/// - `remote_req`: Serialized request payload.
/// - `relay_timeout_ms`: Maximum timeout allowed for HTTP connection and transfer.
///
/// # Returns
/// `Some((response, round_trip_ms))` on successful HTTP 200 JSON decode; `None` on network or protocol error.
///
/// # Errors
/// None. Network errors are logged and increment Prometheus error counters.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Instantiates isolated HTTP client with connection and request timeouts, measures network latency,
/// and surfaces error causes recursively in logs on failure.
async fn dispatch_remote_relay_call(
    state: &AppState,
    endpoint: &str,
    remote_req: &RecognizeRequest,
    relay_timeout_ms: u64,
) -> Option<(RecognizeResponse, f64)> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(
            crate::constants::DEFAULT_RELAY_CONNECT_TIMEOUT_MS,
        ))
        .timeout(std::time::Duration::from_millis(relay_timeout_ms))
        .build()
        .unwrap_or_default();

    let mut req_builder = client.post(endpoint).json(remote_req);
    if let Some(ref api_key) = state.remote_api_key {
        req_builder = req_builder.header("X-API-Key", api_key);
    }

    let t_net = Instant::now();
    match req_builder.send().await {
        Ok(resp) if resp.status().is_success() => {
            let net_rtt_ms = t_net.elapsed().as_secs_f64() * 1000.0;
            resp.json::<RecognizeResponse>()
                .await
                .ok()
                .map(|r| (r, net_rtt_ms))
        }
        Err(e) => {
            let mut full_err = format!("{}", e);
            let mut cur: Option<&dyn std::error::Error> = std::error::Error::source(&e);
            while let Some(src) = cur {
                full_err.push_str(&format!(" -> {}", src));
                cur = src.source();
            }
            state
                .metrics
                .remote_errors
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            state.push_log(
                "RELAY",
                "WARN",
                format!("✗ Remote relay error to {}: {}", endpoint, full_err),
            );
            None
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
            None
        }
    }
}

/// Executes multi-tier routed visual pHash recognition across local indexes and PostgreSQL.
///
/// # Arguments
/// - `state`: Shared server state container (`AppState`).
/// - `media_path`: File path to the video stream being queried.
/// - `phash_dirs`: Search directories holding cached pHash fingerprints.
/// - `index_engine`: Optional engine override (e.g. "mih", "bktree", "hybrid").
/// - `timing`: Mutable search timing statistics to record latency, throughput, and comparison counts.
///
/// # Returns
/// Tuple containing `(matched_media, phash_confidence, target_fps, existing_phash)`.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Accesses Tier-1 RAM index through arc-swap lock, executes multi-tier routing (Tier 1 MIH -> Tier 2 BK-Tree/PostgreSQL),
/// measures execution latency, computes Hamming distance comparisons per second, and records profiling metrics.
fn execute_local_phash_recognition(
    state: &AppState,
    media_path: &Path,
    phash_dirs: &[PathBuf],
    index_engine: Option<&str>,
    timing: &mut SearchTimingStats,
) -> (Option<String>, f64, f64, Option<ExistingPhashData>) {
    let t0 = Instant::now();
    let reader_guard = state.dbphash_reader.load();
    let (matched_routed, target_fps, existing_phash) = identify_media_by_phash_multi_routed(
        media_path,
        phash_dirs,
        reader_guard.as_deref(),
        state.db_config.as_ref(),
        1.0,
        DEFAULT_RECOGNITION_SEARCH_DISTANCE,
        index_engine,
    );
    timing.phash_match_ms = t0.elapsed().as_secs_f64() * 1000.0;

    if let Some((_, ref hashes, fps)) = existing_phash {
        timing.phash_frames_count = hashes.len();
        if fps > 0.0 {
            timing.video_duration_minutes = (hashes.len() as f64 / fps) / SECONDS_PER_MINUTE as f64;
            timing.query_sample_frames = ((crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS * fps)
                .round() as usize)
                .min(hashes.len());
        }
    } else if media_path.exists() {
        if let Ok(meta) = probe_video(media_path) {
            timing.phash_frames_count = (meta.duration * meta.fps).round() as usize;
            timing.video_duration_minutes = meta.duration / SECONDS_PER_MINUTE as f64;
            timing.query_sample_frames =
                ((crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS * meta.fps).round() as usize)
                    .max(1);
        }
    }

    if timing.query_sample_frames == 0 {
        timing.query_sample_frames = DEFAULT_BASELINE_FPS.round() as usize;
    }

    if let Some(ref reader) = *reader_guard {
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

    let mut matched_media = None;
    let mut phash_conf = 0.0;
    if let Some((m_name, conf, tier_label)) = matched_routed {
        matched_media = Some(m_name);
        phash_conf = conf;
        timing.confidence_percent = conf;
        timing.matching_tier = tier_label;
    }

    (matched_media, phash_conf, target_fps, existing_phash)
}

/// Orchestrates visual recognition dispatching between upstream remote relay and local multi-tier engines.
///
/// # Arguments
/// - `state`: Shared server state container (`AppState`).
/// - `payload`: Search request specifying search mode and language.
/// - `media_path`: File path to the target video.
/// - `phash_dirs`: Search directories holding cached pHash fingerprints.
/// - `use_phash`: Boolean indicating whether visual recognition is enabled.
/// - `candidates`: Mutable vector to receive candidate subtitles from remote relay.
/// - `timing`: Mutable search timing statistics.
///
/// # Returns
/// Tuple containing `(matched_media, phash_confidence, target_fps, existing_phash, query_sample)`.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// If visual recognition is enabled and the media file exists, selects remote relay if configured,
/// otherwise executes local multi-tier recognition if indexes or PostgreSQL are configured.
async fn execute_phash_recognition(
    state: &AppState,
    payload: &SearchSubtitlesRequest,
    media_path: &Path,
    phash_dirs: &[PathBuf],
    use_phash: bool,
    candidates: &mut Vec<SubtitleCandidate>,
    timing: &mut SearchTimingStats,
) -> (
    Option<String>,
    f64,
    f64,
    Option<ExistingPhashData>,
    Option<Vec<u64>>,
) {
    if !use_phash || !media_path.exists() {
        return (None, 0.0, DEFAULT_FALLBACK_FPS, None, None);
    }

    if let Some(ref remote_url) = state.remote_api {
        execute_remote_relay_recognition(
            state,
            media_path,
            phash_dirs,
            remote_url,
            &payload.language,
            candidates,
            timing,
        )
        .await
    } else if phash_dirs.iter().any(|d| d.exists())
        || state.dbphash_path().is_some()
        || state.db_config.is_some()
    {
        let (matched_media, phash_conf, target_fps, existing_phash) =
            execute_local_phash_recognition(
                state,
                media_path,
                phash_dirs,
                payload.index_engine.as_deref(),
                timing,
            );
        (matched_media, phash_conf, target_fps, existing_phash, None)
    } else {
        (None, 0.0, DEFAULT_FALLBACK_FPS, None, None)
    }
}

/// Extracts video container metadata, moviehash, and formats visual fingerprint previews.
///
/// # Arguments
/// - `media_path`: File path to the video file or pHash sidecar.
/// - `query_sample`: Optional slice of sampled visual hashes extracted during relay.
/// - `existing_phash`: Optional pre-existing pHash dataset loaded from disk cache.
/// - `timing`: Mutable search timing statistics to record moviehash extraction duration.
///
/// # Returns
/// Tuple containing `(meta, moviehash, fingerprint_preview)`.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Parses filename metadata or probes physical file for MovieHash, filters out uniform blank
/// fingerprints to assemble a human-readable 8-sample hexadecimal preview for UI diagnostics.
fn extract_target_metadata_and_preview(
    media_path: &Path,
    query_sample: Option<&[u64]>,
    existing_phash: Option<&ExistingPhashData>,
    timing: &mut SearchTimingStats,
) -> (MediaMetadata, Option<String>, Option<Vec<String>>) {
    let t_meta = Instant::now();
    let is_phash_file = media_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("phash"))
        .unwrap_or(false);

    let (meta, moviehash) = if is_phash_file {
        let meta = MediaMetadata::parse(
            media_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(""),
        );
        (meta, None)
    } else {
        let meta = MediaMetadata::parse_from_path(media_path);
        let mh = meta.moviehash.clone();
        (meta, mh)
    };
    timing.moviehash_ms = t_meta.elapsed().as_secs_f64() * 1000.0;

    let fingerprint_preview = query_sample
        .or_else(|| existing_phash.map(|(_, hashes, _)| hashes.as_slice()))
        .map(|hashes| {
            let non_blank: Vec<_> = hashes
                .iter()
                .copied()
                .filter(|&h| !dbsrt_core::is_blank_phash(h))
                .take(FINGERPRINT_PREVIEW_SAMPLE_COUNT)
                .collect();
            let chosen = if non_blank.len() >= FINGERPRINT_PREVIEW_SAMPLE_COUNT {
                non_blank
            } else {
                hashes
                    .iter()
                    .copied()
                    .take(FINGERPRINT_PREVIEW_SAMPLE_COUNT)
                    .collect()
            };
            chosen
                .into_iter()
                .map(|h| format!("{:#018x}", h))
                .collect::<Vec<_>>()
        });

    (meta, moviehash, fingerprint_preview)
}

/// Queries certified ground-truth subtitle couplings and computes visual shifts.
///
/// # Arguments
/// - `state`: Shared server state container (`AppState`).
/// - `media_path`: File path to the video stream being queried.
/// - `subtitles_dir`: Base directory holding local subtitle storage.
/// - `phash_dirs`: Search directories holding cached pHash fingerprints.
/// - `matched_media`: Reference release stem identified by visual recognition.
/// - `phash_conf`: Recognition confidence score (0.0 to 100.0).
/// - `target_fps`: Framerate of the target video stream.
/// - `existing_phash`: Optional pre-existing pHash dataset of the target video.
/// - `effective_season`: Target season filter constraint.
/// - `effective_episode`: Target episode filter constraint.
/// - `language`: Target subtitle language code.
/// - `candidates`: Mutable vector to receive certified and retargeted candidates.
/// - `seen_catalog_paths`: Deduplication set tracking catalog paths.
/// - `seen_filenames`: Deduplication set tracking lowercase subtitle filenames.
/// - `timing`: Mutable search timing statistics to record database lookup latency.
///
/// # Returns
/// None. Mutates candidates and timing in place.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Queries PostgreSQL and couplings registry for certified pairings matching the reference release.
/// For each certified subtitle, runs perceptual visual consensus (`compute_media_phash_shift_multi`)
/// against target video frames. Emits baseline cue (shift 0ms) and, if shift exceeds threshold,
/// an auto-synchronized candidate variant (`certified_synced`).
#[allow(clippy::too_many_arguments)]
fn match_certified_couplings_and_shifts(
    state: &AppState,
    media_path: &Path,
    subtitles_dir: &Path,
    phash_dirs: &[PathBuf],
    matched_media: &str,
    phash_conf: f64,
    target_fps: f64,
    existing_phash: &Option<ExistingPhashData>,
    effective_season: Option<u32>,
    effective_episode: Option<u32>,
    language: &str,
    candidates: &mut Vec<SubtitleCandidate>,
    seen_catalog_paths: &mut std::collections::HashSet<String>,
    seen_filenames: &mut std::collections::HashSet<String>,
    timing: &mut SearchTimingStats,
) {
    if candidates.is_empty() && !state.is_agent {
        let (cert_cands, pg_ms, json_ms, pg_cached) = lookup_certified_couplings(
            state,
            subtitles_dir,
            matched_media,
            effective_season,
            effective_episode,
            language,
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

            let ref_stem = c.release.as_deref().unwrap_or(matched_media);
            let (_sf, computed_shift_ms, _sconf) = compute_media_phash_shift_multi(
                media_path,
                phash_dirs,
                ref_stem,
                phash_conf,
                target_fps,
                existing_phash,
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
        for c in candidates.iter() {
            if let Some(ref cid) = c.candidate_id {
                seen_catalog_paths.insert(cid.clone());
            }
            seen_filenames.insert(c.title.to_lowercase());
        }
    }
}

/// Inspects media container tracks and extracts embedded subtitles matching requested language.
///
/// # Arguments
/// - `media_path`: File path to the video file.
/// - `language`: Target subtitle language code.
/// - `candidates`: Mutable vector to receive embedded subtitle candidates.
/// - `timing`: Mutable search timing statistics to record container inspection latency.
///
/// # Returns
/// None. Mutates candidates and timing in place.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Calls FFmpeg/container prober, normalizes stream language codes (ISO-639), matches against requested
/// language code with fallback support for "auto" and "all", and adds candidate descriptors.
fn extract_embedded_container_subtitles(
    media_path: &Path,
    language: &str,
    candidates: &mut Vec<SubtitleCandidate>,
    timing: &mut SearchTimingStats,
) {
    let t_cont = Instant::now();
    if let Ok(insp) = inspect_container(media_path) {
        let req_norm = normalize_lang_code(language);
        for stream in insp.subtitle_streams {
            let stream_lang = stream.language.as_deref().unwrap_or("und");
            let stream_norm = normalize_lang_code(stream_lang);
            let lang_match = language == "auto"
                || language == "all"
                || stream_norm == req_norm
                || stream_lang
                    .to_lowercase()
                    .starts_with(&language.to_lowercase())
                || language
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

/// Discovers and scores local on-disk subtitle files across library and sidecar directories.
///
/// # Arguments
/// - `media_path`: File path to the target video.
/// - `subtitles_dir`: Base directory holding local subtitle storage.
/// - `include_sidecars`: Boolean indicating whether adjacent directory search is enabled.
/// - `matched_media`: Reference release stem identified by visual recognition.
/// - `phash_conf`: Recognition confidence score.
/// - `effective_season`: Target season filter constraint.
/// - `effective_episode`: Target episode filter constraint.
/// - `catalog_query_title`: Optional textual title query.
/// - `language`: Target subtitle language code.
/// - `candidates`: Mutable vector to receive catalog candidates.
/// - `seen_catalog_paths`: Deduplication set tracking catalog paths.
/// - `seen_filenames`: Deduplication set tracking lowercase subtitle filenames.
/// - `timing`: Mutable search timing statistics to record disk search latency.
///
/// # Returns
/// None. Mutates candidates and timing in place.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Iterates over candidate directories, applies fuzzy title matching and episodic token parsing,
/// assigns heuristic bonuses for exact stem matches (+1000) or directory proximity (+500),
/// deduplicates across paths and filenames, and sorts candidates by descending score.
struct CatalogCandidateContext<'a> {
    file_stem: &'a str,
    matched_media: Option<&'a str>,
    phash_conf: f64,
    media_parent: Option<&'a Path>,
}

/// Evaluates a disk catalog subtitle candidate, assigning heuristic ranking bonuses and release metadata.
///
/// # Arguments
/// - `ctx`: Shared context containing target file stem, matched media name, confidence, and media directory.
/// - `path`: Candidate subtitle file path.
/// - `resolved_lang`: Detected or metadata-parsed language code.
/// - `score`: Base text search matching score.
///
/// # Returns
/// Tuple containing `(final_score, candidate)`.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// 1. Awards +1000 score bonus if subtitle stem directly matches target media file stem.
/// 2. Awards +500 score bonus if subtitle resides in the same directory as the media file.
/// 3. Retains target media file stem as release for direct matches, avoiding erroneous third-party relabels.
fn build_disk_catalog_candidate(
    ctx: &CatalogCandidateContext<'_>,
    path: &Path,
    resolved_lang: String,
    score: i32,
) -> (i32, SubtitleCandidate) {
    let cand_fname = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut final_score = score;
    let cand_stem = cand_fname.strip_suffix(".srt").unwrap_or(&cand_fname);
    let direct_match = !ctx.file_stem.is_empty()
        && (cand_stem == ctx.file_stem
            || cand_stem.starts_with(ctx.file_stem)
            || ctx.file_stem.starts_with(cand_stem));
    if direct_match {
        final_score += 1000;
    } else if ctx.media_parent.is_some() && path.parent() == ctx.media_parent {
        final_score += 500;
    }

    let path_str = path.to_string_lossy().to_string();
    let release_val = if direct_match {
        Some(ctx.file_stem.to_string())
    } else {
        ctx.matched_media
            .map(String::from)
            .or_else(|| Some(ctx.file_stem.to_string()))
    };
    let details = if !direct_match && ctx.matched_media.is_some() {
        format!(
            "pHash matched '{}' ({:.0}% conf) — {}",
            ctx.matched_media.unwrap_or_default(),
            ctx.phash_conf,
            path.display()
        )
    } else {
        path_str.clone()
    };
    (
        final_score,
        SubtitleCandidate {
            source: SubtitleSource::Catalog.as_str().to_string(),
            language: resolved_lang,
            title: cand_fname,
            release: release_val,
            details: Some(details),
            candidate_id: Some(path_str),
            shift_ms: None,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn search_disk_catalog_subtitles(
    media_path: &Path,
    subtitles_dir: &Path,
    include_sidecars: bool,
    matched_media: Option<&str>,
    phash_conf: f64,
    effective_season: Option<u32>,
    effective_episode: Option<u32>,
    catalog_query_title: Option<&str>,
    language: &str,
    candidates: &mut Vec<SubtitleCandidate>,
    seen_catalog_paths: &mut std::collections::HashSet<String>,
    seen_filenames: &mut std::collections::HashSet<String>,
    timing: &mut SearchTimingStats,
) {
    let t_disk = Instant::now();
    let file_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut stems = Vec::new();
    if !file_stem.is_empty() {
        stems.push(file_stem.clone());
    }
    if let Some(m) = matched_media {
        if !stems.contains(&m.to_string()) {
            stems.push(m.to_string());
        }
    }

    let search_dirs = collect_subtitle_search_dirs(subtitles_dir, media_path, include_sidecars);
    let mut catalog_results = Vec::new();
    let ctx = CatalogCandidateContext {
        file_stem: &file_stem,
        matched_media,
        phash_conf,
        media_parent: media_path.parent(),
    };

    for dir in &search_dirs {
        for (score, path, resolved_lang) in search_subtitles_in_catalog(
            dir,
            &stems,
            catalog_query_title,
            effective_season,
            effective_episode,
            language,
        ) {
            let path_str = path.to_string_lossy().to_string();
            let canon_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let canon_str = canon_path.to_string_lossy().to_string();
            if !seen_catalog_paths.insert(canon_str.clone()) {
                continue;
            }
            seen_catalog_paths.insert(path_str.clone());

            let cand_fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !seen_filenames.insert(cand_fname.to_lowercase()) {
                continue;
            }

            catalog_results.push(build_disk_catalog_candidate(
                &ctx,
                &path,
                resolved_lang,
                score,
            ));
        }
    }

    catalog_results.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, cand) in catalog_results {
        candidates.push(cand);
    }
    timing.disk_catalog_ms = t_disk.elapsed().as_secs_f64() * 1000.0;
}

/// Queries external providers (OpenSubtitles) and enforces strict episodic consistency filters.
///
/// # Arguments
/// - `state`: Shared server state container (`AppState`).
/// - `mode`: Active search mode ("all", "phash", "text", "auto").
/// - `payload`: Search request containing optional title, season, and episode overrides.
/// - `meta`: Mutable reference to media metadata.
/// - `effective_season`: Target season filter constraint.
/// - `effective_episode`: Target episode filter constraint.
/// - `candidates`: Mutable vector of candidates to filter.
///
/// # Returns
/// None. Mutates candidates in place.
///
/// # Errors
/// None. Remote provider failures are gracefully ignored.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Overrides parsed metadata with user query title/season/episode if supplied, queries provider,
/// appends results, and executes a final episodic retention filter asserting that every candidate
/// matches required season/episode numbers.
async fn fetch_remote_provider_and_filter(
    state: &AppState,
    mode: &str,
    payload: &SearchSubtitlesRequest,
    meta: &mut MediaMetadata,
    effective_season: Option<u32>,
    effective_episode: Option<u32>,
    candidates: &mut Vec<SubtitleCandidate>,
) {
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

        if let Ok(remote_cands) = state.provider.search(meta, &payload.language).await {
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

    if let Some(req_s) = effective_season {
        candidates.retain(|c| {
            let c_meta = MediaMetadata::parse(&c.title);
            let r_meta = c.release.as_deref().map(MediaMetadata::parse);
            let cand_s = c_meta
                .season
                .or_else(|| r_meta.as_ref().and_then(|m| m.season));
            if let Some(cs) = cand_s {
                cs == req_s
            } else {
                true
            }
        });
    }
    if let Some(req_e) = effective_episode {
        candidates.retain(|c| {
            let c_meta = MediaMetadata::parse(&c.title);
            let r_meta = c.release.as_deref().map(MediaMetadata::parse);
            let cand_e = c_meta
                .episode
                .or_else(|| r_meta.as_ref().and_then(|m| m.episode));
            if let Some(ce) = cand_e {
                ce == req_e
                    || c.title.to_lowercase().contains(&format!("e{:02}", req_e))
                    || c.release
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&format!("e{:02}", req_e))
            } else {
                true
            }
        });
    }
}

/// Assembles timing diagnostics, updates global latency metrics, logs and returns the HTTP response.
///
/// # Arguments
/// - `payload`: Original search request.
/// - `matched_media`: Reference media name identified by recognition.
/// - `moviehash`: 64-bit OpenSubtitles container hash.
/// - `fingerprint_preview`: Sample of visual fingerprints for UI display.
/// - `candidates`: Ranked vector of subtitle candidates.
/// - `timing`: Search timing statistics.
/// - `start_overall`: Timestamp marking start of request processing.
/// - `state`: Shared server state container (`AppState`).
/// - `phash_conf`: Perceptual recognition confidence score.
///
/// # Returns
/// `Json<SearchSubtitlesResponse>` containing ranked subtitle candidates, visual shift, and timing diagnostics.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Computes total elapsed time, accumulates latency into server atomics, calculates unaccounted overhead,
/// logs structured operation completion, and packages response JSON.
#[allow(clippy::too_many_arguments)]
fn finalize_search_response(
    payload: SearchSubtitlesRequest,
    matched_media: Option<String>,
    moviehash: Option<String>,
    fingerprint_preview: Option<Vec<String>>,
    candidates: Vec<SubtitleCandidate>,
    mut timing: SearchTimingStats,
    start_overall: Instant,
    state: &AppState,
    phash_conf: f64,
) -> Json<SearchSubtitlesResponse> {
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
        CertifiedMediaCoupling, CertifiedSubtitleRecord, LangStatus, MockSubtitleProvider,
        SubtitleCache,
    };
    use std::path::PathBuf;

    #[test]
    fn test_stem_and_title_overlap_rejects_empty_title() {
        assert!(!stem_and_title_overlap("Big.Buck.Bunny", ""));
        assert!(!stem_and_title_overlap("Big.Buck.Bunny", "   "));
        assert!(!stem_and_title_overlap("", "Big Buck Bunny"));
        assert!(stem_and_title_overlap(
            "big_buck_bunny_trailer",
            "BIG_BUCK_BUNNY"
        ));
        assert!(!stem_and_title_overlap("sintel", "big_buck_bunny"));
    }

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
                    lang_status: LangStatus::Detected("en".to_string()),
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

    #[tokio::test]
    async fn test_search_strict_season_episode_filtering() {
        let root = get_repo_root();
        let state = test_app_state();

        // Seed with What If S2E3 and S2E1, and The Office S5E1
        if let Ok(mut reg) = state.coupling_registry.write() {
            let make_coupling = |name: &str| CertifiedMediaCoupling {
                media_hash: format!("hash_{}", name),
                media_name: name.to_string(),
                phash_filename: format!("{}.phash", name),
                fps: 24.0,
                frame_count: 100,
                subtitles: vec![CertifiedSubtitleRecord {
                    subtitles_hash: format!("sub_hash_{}", name),
                    name: format!("{}.fr.srt", name),
                    lang: "fr".to_string(),
                    lang_status: LangStatus::Confirmed("fr".to_string()),
                    srt_path: format!("{}.fr.srt", name),
                    certified: true,
                    cue_count: 50,
                    sub_frames: vec![],
                }],
            };

            reg.couplings.push(make_coupling(
                "What.If.2021.S02E03.What.if.Happy.Hogan.Saved.Christmas.2160p",
            ));
            reg.couplings.push(make_coupling(
                "What.If.2021.S02E01.What.if.Nebula.Joined.the.Nova.Corps.2160p",
            ));
            reg.couplings
                .push(make_coupling("The.Office.US.S05E01.1080p.NF.WEB.x264-MRSK"));
        }

        // Test 1: Query What If Season 2 Episode 1
        let req_what_if = SearchSubtitlesRequest {
            media_path: "What.If.2021.S02E01.What.if.Nebula.Joined.the.Nova.Corps.2160p.mkv"
                .to_string(),
            language: "fr".to_string(),
            search_mode: Some("name".to_string()),
            query_title: Some("What If".to_string()),
            season: Some(2),
            episode: Some(1),
            subtitles_dir: Some(root.join("data/SRT").to_string_lossy().to_string()),
            phash_dir: None,
            index_engine: None,
        };
        let res_what_if = search_handler(State(state.clone()), Json(req_what_if)).await;
        for c in &res_what_if.0.candidates {
            assert!(
                !c.title.contains("S02E03"),
                "Candidate list must NEVER contain S02E03 when searching for S02E01: {}",
                c.title
            );
        }

        // Test 2: Query The Office Season 4 Episode 1
        let req_office = SearchSubtitlesRequest {
            media_path: "The.Office.US.S04E01E02.Fun.Run.1080p.mkv".to_string(),
            language: "fr".to_string(),
            search_mode: Some("name".to_string()),
            query_title: Some("The Office US".to_string()),
            season: Some(4),
            episode: Some(1),
            subtitles_dir: Some(root.join("data/SRT").to_string_lossy().to_string()),
            phash_dir: None,
            index_engine: None,
        };
        let res_office = search_handler(State(state.clone()), Json(req_office)).await;
        for c in &res_office.0.candidates {
            assert!(
                !c.title.contains("S05E01"),
                "Candidate list must NEVER contain S05E01 when searching for S04E01: {}",
                c.title
            );
        }
    }
}
