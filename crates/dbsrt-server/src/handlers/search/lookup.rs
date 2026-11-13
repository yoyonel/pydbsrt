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
use crate::handlers::autosync::{extract_ahash_query_window, AcousticAlignment};
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
use dbsrt_core::ahash::AhashFile;
use dbsrt_core::{inspect_container, probe_video, MediaMetadata};
use dbsrt_index::AHashSearchCriteria;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Query start (seconds) of an `.ahash` sidecar: it fingerprints the media from its beginning.
const AHASH_SIDECAR_QUERY_START_SECONDS: f64 = 0.0;

struct RecognitionRoutingContext<'a> {
    state: &'a Arc<AppState>,
    payload: &'a SearchSubtitlesRequest,
    media_path: &'a Path,
    phash_dirs: &'a [PathBuf],
}

/// Outcome of the recognition routing stage.
struct RoutedRecognition {
    /// Recognized reference media, if any.
    matched_media: Option<String>,
    /// Recognition confidence (0.0 to 100.0).
    phash_conf: f64,
    /// Framerate of the target stream.
    target_fps: f64,
    /// Pre-existing pHash dataset of the target, if loaded.
    existing_phash: Option<ExistingPhashData>,
    /// Visual query sample extracted during relay, if any.
    query_sample: Option<Vec<u64>>,
    /// Exact acoustic subtitle shift (ms) `query_start_sec - ref_offset_sec`; set only when
    /// the match came from the aHash catalog, `None` for pHash matches.
    acoustic_shift_ms: Option<f64>,
}

/// Routes media recognition across aHash and pHash pipelines based on search mode.
///
/// # Arguments
/// - `ctx`: Routing context (state, payload, media path, pHash directories).
/// - `candidates`: Mutable candidate list (relay results).
/// - `timing`: Mutable timing statistics.
///
/// # Returns
/// [`RoutedRecognition`]; `acoustic_shift_ms` is populated for `ahash` mode and for the
/// `ahash` fallback taken when pHash recognition failed.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// pHash results never carry an acoustic shift: their shift is computed later visually.
async fn route_recognition_pipeline(
    ctx: RecognitionRoutingContext<'_>,
    candidates: &mut Vec<SubtitleCandidate>,
    timing: &mut SearchTimingStats,
) -> RoutedRecognition {
    let mode = ctx.payload.search_mode.as_deref().unwrap_or("all");
    let use_phash = mode == "phash" || mode == "all" || mode == "auto";
    let use_ahash = mode == "ahash";

    if use_ahash {
        return execute_ahash_recognition(ctx.state, ctx.payload, ctx.media_path, timing)
            .await
            .into_routed();
    }

    let (m, conf, fps, ex, qs) = execute_phash_recognition(
        ctx.state,
        ctx.payload,
        ctx.media_path,
        ctx.phash_dirs,
        use_phash,
        candidates,
        timing,
    )
    .await;
    if m.is_none() && (mode == "all" || mode == "auto") && ctx.media_path.exists() {
        return execute_ahash_recognition(ctx.state, ctx.payload, ctx.media_path, timing)
            .await
            .into_routed();
    }
    RoutedRecognition {
        matched_media: m,
        phash_conf: conf,
        target_fps: fps,
        existing_phash: ex,
        query_sample: qs,
        acoustic_shift_ms: None,
    }
}

/// Logs the incoming search request (mode, language, media file name).
///
/// # Arguments
/// - `state`: Application state owning the log ring buffer.
/// - `mode`: Effective search mode.
/// - `language`: Requested subtitle language code.
/// - `media_path`: Target media path (only the file name is logged).
///
/// # Returns
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Pure side effect on the log buffer; no behavior depends on it.
fn log_search_request(state: &AppState, mode: &str, language: &str, media_path: &Path) {
    state.push_log(
        "SEARCH",
        "INFO",
        format!(
            "Request: mode='{}', lang='{}', media='{}'",
            mode,
            language,
            media_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
        ),
    );
}

/// Normalizes a media path that may arrive as a `file://` URI (POSIX `file:///path` or Windows `file:///C:/path`).
pub(crate) fn normalize_media_path_uri(raw_path: &str) -> PathBuf {
    let trimmed = raw_path.trim();
    if let Some(after_triple) = trimmed.strip_prefix("file:///") {
        let bytes = after_triple.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return PathBuf::from(after_triple);
        }
        return PathBuf::from(format!("/{after_triple}"));
    }
    if let Some(after_scheme) = trimmed.strip_prefix("file://") {
        return PathBuf::from(after_scheme);
    }
    PathBuf::from(trimmed)
}

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

    let media_path = normalize_media_path_uri(&payload.media_path);
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

    log_search_request(&state, mode, &payload.language, &media_path);

    let mut timing = SearchTimingStats::default();

    let RoutedRecognition {
        matched_media,
        phash_conf,
        target_fps,
        existing_phash,
        query_sample: query_sample_opt,
        acoustic_shift_ms,
    } = route_recognition_pipeline(
        RecognitionRoutingContext {
            state: &state,
            payload: &payload,
            media_path: &media_path,
            phash_dirs: &phash_dirs,
        },
        &mut candidates,
        &mut timing,
    )
    .await;

    let (mut meta, moviehash, fingerprint_preview) = extract_target_metadata_and_preview(
        &media_path,
        query_sample_opt.as_deref(),
        existing_phash.as_ref(),
        &mut timing,
    )
    .await;

    let effective_season = payload.season.or(meta.season);
    let effective_episode = payload.episode.or(meta.episode);

    let mut seen_catalog_paths = std::collections::HashSet::new();
    let mut seen_filenames = std::collections::HashSet::new();

    if let Some(ref m_name) = matched_media {
        match_certified_couplings_and_shifts(
            &CertifiedMatchContext {
                state: &state,
                media_path: &media_path,
                subtitles_dir: &subtitles_dir,
                phash_dirs: &phash_dirs,
                matched_media: m_name,
                phash_conf,
                target_fps,
                existing_phash: &existing_phash,
                acoustic_shift_ms,
                effective_season,
                effective_episode,
                language: &payload.language,
            },
            &mut candidates,
            &mut seen_catalog_paths,
            &mut seen_filenames,
            &mut timing,
        )
        .await;
    }

    if mode != "phash" && media_path.exists() {
        extract_embedded_container_subtitles(
            &media_path,
            &payload.language,
            &mut candidates,
            &mut timing,
        )
        .await;
    }

    let catalog_query_title = if mode == "phash" {
        None
    } else {
        payload.query_title.as_deref()
    };

    search_disk_catalog_subtitles(
        &DiskCatalogSearchContext {
            media_path: &media_path,
            subtitles_dir: &subtitles_dir,
            include_sidecars,
            matched_media: matched_media.as_deref(),
            phash_conf,
            is_ahash_match: acoustic_shift_ms.is_some(),
            effective_season,
            effective_episode,
            catalog_query_title,
            language: &payload.language,
        },
        &mut candidates,
        (&mut seen_catalog_paths, &mut seen_filenames),
        &mut timing,
    )
    .await;

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
        &state,
        FinalizeSearchContext {
            payload,
            matched_media,
            moviehash,
            fingerprint_preview,
            candidates,
            timing,
            start_overall,
            phash_conf,
        },
    )
}

struct RemoteRelayProcessingContext<'a> {
    state: &'a AppState,
    remote_url: &'a str,
    media_path: &'a Path,
    candidates: &'a mut Vec<SubtitleCandidate>,
    timing: &'a mut SearchTimingStats,
}

/// True when the local file stem and the remote match title share significant words
/// (case-insensitive, ignoring underscores, dots, hyphens, and punctuation).
///
/// # Arguments
/// - `file_stem`: Local media filename stem.
/// - `match_title`: Candidate media title answered by recognition service.
///
/// # Returns
/// `true` if titles overlap significantly; `false` otherwise.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Normalizes strings by replacing punctuation and delimiters with spaces, then checks
/// substring inclusion and word set intersection.
fn stem_and_title_overlap(file_stem: &str, match_title: &str) -> bool {
    let normalize = |s: &str| {
        s.to_lowercase()
            .replace(['_', '.', '-', '(', ')', '[', ']', '+'], " ")
    };
    let norm_stem = normalize(file_stem);
    let norm_title = normalize(match_title);

    let trimmed_stem = norm_stem.trim();
    let trimmed_title = norm_title.trim();
    if trimmed_stem.is_empty() || trimmed_title.is_empty() {
        return false;
    }

    if trimmed_stem.contains(trimmed_title) || trimmed_title.contains(trimmed_stem) {
        return true;
    }

    let stem_words: std::collections::HashSet<&str> = norm_stem
        .split_whitespace()
        .filter(|w| w.len() > 1)
        .collect();
    let title_words: Vec<&str> = norm_title
        .split_whitespace()
        .filter(|w| w.len() > 1)
        .collect();

    if title_words.is_empty() {
        return false;
    }

    let matches = title_words
        .iter()
        .filter(|w| stem_words.contains(*w))
        .count();
    matches >= title_words.len().min(2) || (matches * 2) >= title_words.len()
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
    let server_mih_ms = rec_resp.timer_seconds * 1000.0;
    ctx.timing.phash_match_ms = server_mih_ms;
    ctx.timing.wan_rtt_ms = (net_rtt_ms - server_mih_ms).max(0.0);
    ctx.timing.matching_tier = format!(
        "Remote Relay ({} via {})",
        rec_resp.tier.as_deref().unwrap_or("Tier 1 MIH"),
        ctx.remote_url
    );

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

/// Runs the ffmpeg-backed query sample extraction on the blocking pool so it does not stall
/// async runtime workers; a panicked extraction yields an empty sample.
async fn extract_query_sample_off_runtime(
    media_path: &Path,
    phash_dirs: &[PathBuf],
) -> (f64, Vec<u64>, Option<ExistingPhashData>) {
    let (media_path, phash_dirs) = (media_path.to_path_buf(), phash_dirs.to_vec());
    tokio::task::spawn_blocking(move || {
        extract_media_query_sample(
            &media_path,
            &phash_dirs,
            crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS,
        )
    })
    .await
    .unwrap_or_else(|_| (DEFAULT_BASELINE_FPS, Vec::new(), None))
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
    let (target_fps, query_sample, existing_phash) =
        extract_query_sample_off_runtime(media_path, phash_dirs).await;
    let extract_ms = t0.elapsed().as_secs_f64() * 1000.0;
    timing.ffmpeg_ms = extract_ms;
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

    let mut req_builder = client
        .post(endpoint)
        .header(reqwest::header::USER_AGENT, "dbsrt-agent/0.1.0")
        .json(remote_req);
    if let Some(ref api_key) = state.remote_api_key {
        req_builder = req_builder
            .header("X-API-Key", api_key)
            .header("Authorization", format!("Bearer {}", api_key));
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
        Ok(resp) => {
            state
                .metrics
                .remote_errors
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            state.push_log(
                "RELAY",
                "WARN",
                format!(
                    "✗ Remote relay returned unexpected non-200 status: HTTP {}",
                    resp.status()
                ),
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
/// Runs `execute_local_phash_recognition` on the blocking thread pool under a heavy-task permit.
async fn run_local_phash_off_runtime(
    state: &Arc<AppState>,
    media_path: &Path,
    phash_dirs: &[PathBuf],
    payload: &SearchSubtitlesRequest,
) -> (
    (Option<String>, f64, f64, Option<ExistingPhashData>),
    SearchTimingStats,
) {
    let _permit = state.heavy_task_semaphore.acquire().await.ok();
    let st = Arc::clone(state);
    let mp = media_path.to_path_buf();
    let pdirs = phash_dirs.to_vec();
    let eng = payload.index_engine.clone();
    let mut local_timing = SearchTimingStats::default();
    tokio::task::spawn_blocking(move || {
        let r =
            execute_local_phash_recognition(&st, &mp, &pdirs, eng.as_deref(), &mut local_timing);
        (r, local_timing)
    })
    .await
    .unwrap_or((
        (None, 0.0, DEFAULT_FALLBACK_FPS, None),
        SearchTimingStats::default(),
    ))
}

async fn execute_phash_recognition(
    state: &Arc<AppState>,
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
        let (res, updated_timing) =
            run_local_phash_off_runtime(state, media_path, phash_dirs, payload).await;
        *timing = updated_timing;
        let (matched_media, phash_conf, target_fps, existing_phash) = res;
        (matched_media, phash_conf, target_fps, existing_phash, None)
    } else {
        (None, 0.0, DEFAULT_FALLBACK_FPS, None, None)
    }
}

/// Outcome of the acoustic recognition stage.
struct AhashRecognition {
    /// Recognized reference media, if any.
    matched_media: Option<String>,
    /// Recognition confidence (0.0 to 100.0).
    confidence: f64,
    /// Probe position versus reference match offset, if a match was found.
    alignment: Option<AcousticAlignment>,
}

impl AhashRecognition {
    /// A recognition attempt that produced no match.
    fn none() -> Self {
        Self {
            matched_media: None,
            confidence: 0.0,
            alignment: None,
        }
    }

    /// Converts into the routing outcome (aHash has no pHash dataset nor visual sample).
    fn into_routed(self) -> RoutedRecognition {
        RoutedRecognition {
            matched_media: self.matched_media,
            phash_conf: self.confidence,
            target_fps: DEFAULT_BASELINE_FPS,
            existing_phash: None,
            query_sample: None,
            acoustic_shift_ms: self.alignment.map(|a| a.shift_ms()),
        }
    }
}

/// Extracts or reads the `.ahash` query window on a blocking thread.
async fn extract_or_read_ahash_query(media_path: &Path) -> Option<(AhashFile, f64)> {
    let is_ahash_file = media_path.extension().is_some_and(|e| e == "ahash");
    let p = media_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if is_ahash_file {
            AhashFile::read_from_path(&p)
                .ok()
                .map(|f| (f, AHASH_SIDECAR_QUERY_START_SECONDS))
        } else {
            extract_ahash_query_window(&p)
        }
    })
    .await
    .ok()
    .flatten()
}

/// Queries the local in-memory `AHashCatalog` on the blocking thread pool.
async fn try_local_ahash_catalog_match(
    state: &AppState,
    hashes: &[u64],
    audio_fps: f64,
    query_start_sec: f64,
) -> Option<(AhashRecognition, String, f64)> {
    let catalog = state.ahash_catalog.load_full();
    if catalog.entries.is_empty() || (state.is_agent && state.remote_api.is_some()) {
        return None;
    }
    let t_match = Instant::now();
    let owned_hashes = hashes.to_vec();
    let matches = tokio::task::spawn_blocking(move || {
        catalog.search(&AHashSearchCriteria::new(&owned_hashes, audio_fps))
    })
    .await
    .unwrap_or_default();
    let match_ms = t_match.elapsed().as_secs_f64() * MILLIS_PER_SEC;
    let best = matches.first()?;
    let conf = best.score * 100.0;
    let tier = format!("Tier aHash Catalog (SNR={:.1} bits)", best.snr_bits);
    state.push_log(
        "AHASH",
        "INFO",
        format!(
            "✓ aHash match: '{}' (conf={:.1}%, SNR={:.1} bits, offset={:.2}s, query_start={:.2}s)",
            best.media_name, conf, best.snr_bits, best.offset_sec, query_start_sec
        ),
    );
    let rec = AhashRecognition {
        matched_media: Some(best.media_name.clone()),
        confidence: conf,
        alignment: Some(AcousticAlignment {
            query_start_sec,
            ref_offset_sec: best.offset_sec,
        }),
    };
    Some((rec, tier, match_ms))
}

/// Relays query hashes to `POST {remote_api}/v1/search/ahash` and formats timing telemetry.
async fn try_remote_ahash_relay_match(
    state: &AppState,
    hashes: &[u64],
    audio_fps: f64,
    query_start_sec: f64,
) -> Option<(AhashRecognition, String, f64, f64)> {
    let remote_url = state.remote_api.as_deref()?;
    let (resp, net_rtt_ms) = crate::handlers::search::ahash::relay_remote_ahash_search(
        state, remote_url, hashes, audio_fps,
    )
    .await?;
    let best = resp
        .best_match
        .or_else(|| resp.candidates.into_iter().next())?;
    let conf = best.score * 100.0;
    let server_ms = resp.elapsed_seconds * MILLIS_PER_SEC;
    let wan_rtt_ms = (net_rtt_ms - server_ms).max(0.0);
    let tier = format!(
        "Remote Relay (Tier aHash SNR={:.1} bits via {})",
        best.snr_bits, remote_url
    );
    state.push_log(
        "RELAY",
        "INFO",
        format!(
            "✓ Remote aHash match: '{}' (conf={:.1}%, SNR={:.1} bits, offset={:.2}s, RTT={:.1}ms)",
            best.media_name, conf, best.snr_bits, best.offset_sec, net_rtt_ms
        ),
    );
    let rec = AhashRecognition {
        matched_media: Some(best.media_name),
        confidence: conf,
        alignment: Some(AcousticAlignment {
            query_start_sec,
            ref_offset_sec: best.offset_sec,
        }),
    };
    Some((rec, tier, server_ms, wan_rtt_ms))
}

async fn execute_ahash_recognition(
    state: &AppState,
    _payload: &SearchSubtitlesRequest,
    media_path: &Path,
    timing: &mut SearchTimingStats,
) -> AhashRecognition {
    if !media_path.exists()
        || (state.ahash_catalog.load().entries.is_empty() && state.remote_api.is_none())
    {
        return AhashRecognition::none();
    }

    let _permit = state.heavy_task_semaphore.acquire().await.ok();
    let t0 = Instant::now();
    let extracted = extract_or_read_ahash_query(media_path).await;
    timing.ffmpeg_ms = t0.elapsed().as_secs_f64() * MILLIS_PER_SEC;

    let (ahash_file, query_start_sec) = match extracted {
        Some((f, start)) if f.hashes.len() >= dbsrt_index::MIN_QUERY_FRAMES => (f, start),
        _ => {
            state.push_log(
                "AHASH",
                "WARN",
                format!(
                    "Failed to extract audio fingerprint from {:?}",
                    media_path.file_name()
                ),
            );
            return AhashRecognition::none();
        }
    };

    let audio_fps = ahash_file.header.audio_fps();
    if let Some((rec, tier, match_ms)) =
        try_local_ahash_catalog_match(state, &ahash_file.hashes, audio_fps, query_start_sec).await
    {
        timing.matching_tier = tier;
        timing.phash_match_ms = match_ms;
        timing.confidence_percent = rec.confidence;
        return rec;
    }

    if let Some((rec, tier, server_ms, wan_rtt_ms)) =
        try_remote_ahash_relay_match(state, &ahash_file.hashes, audio_fps, query_start_sec).await
    {
        timing.matching_tier = tier;
        timing.phash_match_ms = server_ms;
        timing.wan_rtt_ms = wan_rtt_ms;
        timing.confidence_percent = rec.confidence;
        return rec;
    }

    state.push_log(
        "AHASH",
        "INFO",
        format!(
            "aHash search: 0 matches found in catalog for {:?}",
            media_path.file_name()
        ),
    );
    AhashRecognition::none()
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
/// Parses filename metadata or probes physical file for MovieHash on blocking pool, filters out
/// uniform blank fingerprints to assemble a human-readable 8-sample hexadecimal preview.
async fn extract_target_metadata_and_preview(
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
        let p = media_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let meta = MediaMetadata::parse_from_path(&p);
            let mh = meta.moviehash.clone();
            (meta, mh)
        })
        .await
        .unwrap_or_else(|_| (MediaMetadata::parse(""), None))
    };
    timing.moviehash_ms = t_meta.elapsed().as_secs_f64() * MILLIS_PER_SEC;

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

/// Immutable inputs of [`match_certified_couplings_and_shifts`].
struct CertifiedMatchContext<'a> {
    /// Shared server state container.
    state: &'a Arc<AppState>,
    /// File path to the video stream being queried.
    media_path: &'a Path,
    /// Base directory holding local subtitle storage.
    subtitles_dir: &'a Path,
    /// Search directories holding cached pHash fingerprints.
    phash_dirs: &'a [PathBuf],
    /// Reference release stem identified by recognition.
    matched_media: &'a str,
    /// Recognition confidence score (0.0 to 100.0).
    phash_conf: f64,
    /// Framerate of the target video stream.
    target_fps: f64,
    /// Optional pre-existing pHash dataset of the target video.
    existing_phash: &'a Option<ExistingPhashData>,
    /// Exact acoustic shift (`query_start_sec - ref_offset_sec`) when recognized by aHash;
    /// used directly instead of re-running visual pHash consensus.
    acoustic_shift_ms: Option<f64>,
    /// Target season filter constraint.
    effective_season: Option<u32>,
    /// Target episode filter constraint.
    effective_episode: Option<u32>,
    /// Target subtitle language code.
    language: &'a str,
}

/// Builds the auto-synchronized (`certified_synced`) variant of a certified candidate.
///
/// # Arguments
/// - `c`: Baseline certified candidate (shift 0 ms).
/// - `shift_ms`: Computed shift in milliseconds.
///
/// # Returns
/// Cloned candidate with `source = "certified_synced"`, the shift, and annotated title/details.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Pure function; `c` is left untouched.
fn build_synced_variant(c: &SubtitleCandidate, shift_ms: f64) -> SubtitleCandidate {
    let mut c_synced = c.clone();
    c_synced.source = "certified_synced".to_string();
    c_synced.shift_ms = Some(shift_ms);
    c_synced.title = format!("{} [Synced {:+.1}s]", c.title, shift_ms / MILLIS_PER_SEC);
    if let Some(ref d) = c.details {
        c_synced.details = Some(format!(
            "{} (Synchronized: {:+.2}s visual shift)",
            d,
            shift_ms / MILLIS_PER_SEC
        ));
    }
    c_synced
}

/// Records a certified candidate in the deduplication sets (raw and canonical catalog path).
///
/// # Arguments
/// - `c`: Certified candidate.
/// - `seen_catalog_paths`: Deduplication set tracking catalog paths.
/// - `seen_filenames`: Deduplication set tracking lowercase subtitle filenames.
///
/// # Returns
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Inserts the candidate id, its canonicalized form when resolvable, and the lowercase title.
fn register_certified_as_seen(
    c: &SubtitleCandidate,
    seen_catalog_paths: &mut std::collections::HashSet<String>,
    seen_filenames: &mut std::collections::HashSet<String>,
) {
    if let Some(ref cid) = c.candidate_id {
        seen_catalog_paths.insert(cid.clone());
        if let Ok(canon) = Path::new(cid).canonicalize() {
            seen_catalog_paths.insert(canon.to_string_lossy().to_string());
        }
    }
    seen_filenames.insert(c.title.to_lowercase());
}

/// Fetches remote certified couplings from `{remote_url}/v1/couplings?summary=true` and caches them in memory.
async fn fetch_and_cache_remote_couplings(state: &AppState, remote_url: &str) -> bool {
    let url = format!(
        "{}/v1/couplings?summary=true",
        remote_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(
            crate::constants::DEFAULT_RELAY_CONNECT_TIMEOUT_MS,
        ))
        .timeout(std::time::Duration::from_millis(
            crate::constants::DEFAULT_RELAY_TIMEOUT_MS,
        ))
        .build()
        .unwrap_or_default();
    let mut req = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, "dbsrt-agent/0.1.0");
    if let Some(ref api_key) = state.remote_api_key {
        req = req
            .header("X-API-Key", api_key)
            .header("Authorization", format!("Bearer {}", api_key));
    }
    let Ok(resp) = req.send().await else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    let Ok(overview) = resp.json::<crate::dtos::CouplingsOverviewResponse>().await else {
        return false;
    };
    let Ok(mut reg) = state.coupling_registry.write() else {
        return false;
    };
    for c in overview.couplings {
        for sub in c.subtitles {
            reg.upsert_coupling(
                c.media_hash.clone(),
                c.media_name.clone(),
                c.phash_filename.clone(),
                c.fps,
                c.frame_count,
                sub,
            );
        }
    }
    true
}

/// Runs `lookup_certified_couplings` on the blocking thread pool.
async fn run_lookup_certified_off_runtime(
    ctx: &CertifiedMatchContext<'_>,
) -> (Vec<SubtitleCandidate>, f64, f64, bool) {
    let st = Arc::clone(ctx.state);
    let sdir = ctx.subtitles_dir.to_path_buf();
    let mname = ctx.matched_media.to_string();
    let (s, e) = (ctx.effective_season, ctx.effective_episode);
    let lang = ctx.language.to_string();
    let conf = ctx.phash_conf;
    tokio::task::spawn_blocking(move || {
        lookup_certified_couplings(&st, &sdir, &mname, s, e, &lang, conf)
    })
    .await
    .unwrap_or_else(|_| (Vec::new(), 0.0, 0.0, false))
}

/// Queries certified ground-truth subtitle couplings and computes visual shifts.
///
/// # Arguments
/// - `ctx`: Immutable inputs ([`CertifiedMatchContext`]).
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
/// Queries PostgreSQL and couplings registry (syncing from remote `/v1/couplings?summary=true`
/// in `--agent` mode when local registry has no match) for certified pairings matching the reference release.
/// For each certified subtitle, uses `acoustic_shift_ms` when present; otherwise runs perceptual
/// visual consensus (`compute_media_phash_shift_multi`) against target video frames on blocking pool.
/// Resolves certified candidates locally and falls back to remote `/v1/couplings?summary=true` when empty.
async fn resolve_certified_candidates_with_remote_fallback(
    ctx: &CertifiedMatchContext<'_>,
    timing: &mut SearchTimingStats,
) -> Vec<SubtitleCandidate> {
    let (mut cert_cands, pg_ms, json_ms, pg_cached) = run_lookup_certified_off_runtime(ctx).await;
    if cert_cands.is_empty() {
        if let Some(ref remote_url) = ctx.state.remote_api {
            if fetch_and_cache_remote_couplings(ctx.state, remote_url).await {
                let (reloaded, _, _, _) = run_lookup_certified_off_runtime(ctx).await;
                cert_cands = reloaded;
            }
        }
    }
    timing.pg_lookup_ms = pg_ms;
    timing.json_lookup_ms = json_ms;
    timing.pg_cached = pg_cached;
    cert_cands
}

/// Computes the temporal shift (ms) for a certified candidate on the blocking thread pool.
async fn compute_certified_candidate_shift_ms(
    ctx: &CertifiedMatchContext<'_>,
    ref_stem: String,
) -> f64 {
    if let Some(shift_ms) = ctx.acoustic_shift_ms {
        return shift_ms;
    }
    let mp = ctx.media_path.to_path_buf();
    let pdirs = ctx.phash_dirs.to_vec();
    let conf = ctx.phash_conf;
    let fps = ctx.target_fps;
    let ex = ctx.existing_phash.clone();
    tokio::task::spawn_blocking(move || {
        compute_media_phash_shift_multi(
            &mp,
            &pdirs,
            &ref_stem,
            conf,
            fps,
            &ex,
            DEFAULT_SEARCH_DISTANCE,
        )
        .1
    })
    .await
    .unwrap_or(0.0)
}

async fn match_certified_couplings_and_shifts(
    ctx: &CertifiedMatchContext<'_>,
    candidates: &mut Vec<SubtitleCandidate>,
    seen_catalog_paths: &mut std::collections::HashSet<String>,
    seen_filenames: &mut std::collections::HashSet<String>,
    timing: &mut SearchTimingStats,
) {
    if !candidates.is_empty() {
        for c in candidates.iter() {
            if let Some(ref cid) = c.candidate_id {
                seen_catalog_paths.insert(cid.clone());
            }
            seen_filenames.insert(c.title.to_lowercase());
        }
        return;
    }

    let cert_cands = resolve_certified_candidates_with_remote_fallback(ctx, timing).await;
    for mut c in cert_cands {
        register_certified_as_seen(&c, seen_catalog_paths, seen_filenames);
        let ref_stem = c
            .release
            .as_deref()
            .unwrap_or(ctx.matched_media)
            .to_string();
        let computed_shift_ms = compute_certified_candidate_shift_ms(ctx, ref_stem).await;

        c.shift_ms = Some(0.0);
        candidates.push(c.clone());

        if computed_shift_ms.abs() > SIGNIFICANT_SHIFT_THRESHOLD_MS {
            candidates.push(build_synced_variant(&c, computed_shift_ms));
        }
    }
}

/// Runs `inspect_container` on the blocking thread pool.
async fn inspect_container_off_runtime(
    media_path: &Path,
) -> Option<dbsrt_core::ContainerInspection> {
    let mp = media_path.to_path_buf();
    tokio::task::spawn_blocking(move || inspect_container(&mp).ok())
        .await
        .ok()
        .flatten()
}

async fn extract_embedded_container_subtitles(
    media_path: &Path,
    language: &str,
    candidates: &mut Vec<SubtitleCandidate>,
    timing: &mut SearchTimingStats,
) {
    let t_cont = Instant::now();
    if let Some(insp) = inspect_container_off_runtime(media_path).await {
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
    timing.container_ms = t_cont.elapsed().as_secs_f64() * MILLIS_PER_SEC;
}

/// Discovers and scores local on-disk subtitle files across library and sidecar directories.
struct CatalogCandidateContext<'a> {
    file_stem: &'a str,
    matched_media: Option<&'a str>,
    phash_conf: f64,
    is_ahash_match: bool,
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
    let method_label = if ctx.is_ahash_match { "aHash" } else { "pHash" };
    let details = if !direct_match && ctx.matched_media.is_some() {
        format!(
            "{} matched '{}' ({:.0}% conf) — {}",
            method_label,
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

/// Immutable inputs for [`search_disk_catalog_subtitles`].
struct DiskCatalogSearchContext<'a> {
    media_path: &'a Path,
    subtitles_dir: &'a Path,
    include_sidecars: bool,
    matched_media: Option<&'a str>,
    phash_conf: f64,
    is_ahash_match: bool,
    effective_season: Option<u32>,
    effective_episode: Option<u32>,
    catalog_query_title: Option<&'a str>,
    language: &'a str,
}

/// Scans on-disk subtitle directories on the blocking thread pool.
async fn collect_disk_catalog_hits_off_runtime(
    ctx: &DiskCatalogSearchContext<'_>,
) -> Option<(
    String,
    Option<String>,
    PathBuf,
    Vec<(i32, PathBuf, PathBuf, String)>,
)> {
    let media_path = ctx.media_path.to_path_buf();
    let subtitles_dir = ctx.subtitles_dir.to_path_buf();
    let include_sidecars = ctx.include_sidecars;
    let matched_media = ctx.matched_media.map(String::from);
    let effective_season = ctx.effective_season;
    let effective_episode = ctx.effective_episode;
    let catalog_query_title = ctx.catalog_query_title.map(String::from);
    let language = ctx.language.to_string();

    tokio::task::spawn_blocking(move || {
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
        let search_dirs =
            collect_subtitle_search_dirs(&subtitles_dir, &media_path, include_sidecars);
        let mut hits = Vec::new();
        for dir in &search_dirs {
            for (score, path, resolved_lang) in search_subtitles_in_catalog(
                dir,
                &stems,
                catalog_query_title.as_deref(),
                effective_season,
                effective_episode,
                &language,
            ) {
                let canon_path = path.canonicalize().unwrap_or_else(|_| path.clone());
                hits.push((score, path, canon_path, resolved_lang));
            }
        }
        (file_stem, matched_media, media_path, hits)
    })
    .await
    .ok()
}

async fn search_disk_catalog_subtitles(
    ctx: &DiskCatalogSearchContext<'_>,
    candidates: &mut Vec<SubtitleCandidate>,
    seen: (
        &mut std::collections::HashSet<String>,
        &mut std::collections::HashSet<String>,
    ),
    timing: &mut SearchTimingStats,
) {
    let t_disk = Instant::now();
    let Some((file_stem, matched_media_owned, media_path_owned, hits)) =
        collect_disk_catalog_hits_off_runtime(ctx).await
    else {
        timing.disk_catalog_ms = t_disk.elapsed().as_secs_f64() * MILLIS_PER_SEC;
        return;
    };

    let (seen_catalog_paths, seen_filenames) = seen;
    let cand_ctx = CatalogCandidateContext {
        file_stem: &file_stem,
        matched_media: matched_media_owned.as_deref(),
        phash_conf: ctx.phash_conf,
        is_ahash_match: ctx.is_ahash_match,
        media_parent: media_path_owned.parent(),
    };

    let mut catalog_results = Vec::new();
    for (score, path, canon_path, resolved_lang) in hits {
        let path_str = path.to_string_lossy().to_string();
        let canon_str = canon_path.to_string_lossy().to_string();
        if !seen_catalog_paths.insert(canon_str) {
            continue;
        }
        seen_catalog_paths.insert(path_str);

        let cand_fname = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !seen_filenames.insert(cand_fname.to_lowercase()) {
            continue;
        }

        catalog_results.push(build_disk_catalog_candidate(
            &cand_ctx,
            &path,
            resolved_lang,
            score,
        ));
    }

    catalog_results.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, cand) in catalog_results {
        candidates.push(cand);
    }
    timing.disk_catalog_ms = t_disk.elapsed().as_secs_f64() * MILLIS_PER_SEC;
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
struct SearchTelemetryPayload<'a> {
    pub media_path: &'a str,
    pub matched_media: Option<&'a str>,
    pub phash_conf: f64,
    pub total_candidates: usize,
}

/// Records end-to-end 4-stage search telemetry into AppState for real-time dashboard visualization.
///
/// # Arguments
/// - `state`: Shared server state container (`AppState`).
/// - `payload`: Core query metadata and results (`SearchTelemetryPayload`).
/// - `timing`: Search timing profiling statistics (`SearchTimingStats`).
///
/// # Returns
/// Nothing (`()`).
///
/// # Errors
/// Guaranteed infallible.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Extracts filename basename, formats local ISO timestamp, calculates relative stage percentages,
/// clamps percentages to `[0.0, 100.0]`, and atomically stores record in `state.latest_search`.
fn record_search_telemetry(
    state: &AppState,
    payload: &SearchTelemetryPayload<'_>,
    timing: &SearchTimingStats,
) {
    let total_for_pct = if timing.total_ms > 0.0 {
        timing.total_ms
    } else {
        1.0
    };
    let ffmpeg_ms = if timing.ffmpeg_ms > 0.0 {
        timing.ffmpeg_ms
    } else {
        timing.container_ms.max(timing.overhead_ms)
    };
    let wan_ms = timing.wan_rtt_ms;
    let mih_ms = timing.phash_match_ms;
    let srt_ms = timing.disk_catalog_ms;

    let filename = std::path::Path::new(payload.media_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| payload.media_path.to_string());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() % crate::constants::SECONDS_PER_DAY;
    let h = secs / crate::constants::SECONDS_PER_HOUR;
    let m = (secs % crate::constants::SECONDS_PER_HOUR) / crate::constants::SECONDS_PER_MINUTE;
    let s = secs % crate::constants::SECONDS_PER_MINUTE;
    let ms = now.subsec_millis();
    let ts = format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms);

    let telemetry = crate::dtos::LatestSearchTelemetry {
        timestamp: ts,
        media_path: payload.media_path.to_string(),
        filename,
        matched_media: payload.matched_media.map(|s| s.to_string()),
        confidence_percent: payload.phash_conf,
        ffmpeg_ms,
        wan_rtt_ms: wan_ms,
        mih_match_ms: mih_ms,
        srt_disk_ms: srt_ms,
        moviehash_ms: timing.moviehash_ms,
        total_ms: timing.total_ms,
        ffmpeg_pct: (ffmpeg_ms / total_for_pct * 100.0).clamp(0.0, 100.0),
        wan_pct: (wan_ms / total_for_pct * 100.0).clamp(0.0, 100.0),
        mih_pct: (mih_ms / total_for_pct * 100.0).clamp(0.0, 100.0),
        srt_pct: (srt_ms / total_for_pct * 100.0).clamp(0.0, 100.0),
        is_relay: state.remote_api.is_some(),
        total_candidates: payload.total_candidates,
        matching_tier: timing.matching_tier.clone(),
    };
    state.record_latest_search(telemetry);
}

struct FinalizeSearchContext {
    pub payload: SearchSubtitlesRequest,
    pub matched_media: Option<String>,
    pub moviehash: Option<String>,
    pub fingerprint_preview: Option<Vec<String>>,
    pub candidates: Vec<SubtitleCandidate>,
    pub timing: SearchTimingStats,
    pub start_overall: Instant,
    pub phash_conf: f64,
}

fn finalize_search_response(
    state: &AppState,
    mut ctx: FinalizeSearchContext,
) -> Json<SearchSubtitlesResponse> {
    let total_candidates = ctx.candidates.len();
    ctx.timing.total_ms = ctx.start_overall.elapsed().as_secs_f64() * 1000.0;
    state.metrics.total_latency_us.fetch_add(
        ctx.start_overall.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    let accounted_ms = ctx.timing.phash_match_ms
        + ctx.timing.pg_lookup_ms
        + ctx.timing.json_lookup_ms
        + ctx.timing.container_ms
        + ctx.timing.disk_catalog_ms
        + ctx.timing.moviehash_ms;
    ctx.timing.overhead_ms = (ctx.timing.total_ms - accounted_ms).max(0.0);

    record_search_telemetry(
        state,
        &SearchTelemetryPayload {
            media_path: &ctx.payload.media_path,
            matched_media: ctx.matched_media.as_deref(),
            phash_conf: ctx.phash_conf,
            total_candidates,
        },
        &ctx.timing,
    );

    state.push_log(
        "SEARCH",
        "INFO",
        format!(
            "Completed in {:.2}ms: matched='{}' (conf={:.1}%), candidates={}",
            ctx.timing.total_ms,
            ctx.matched_media.as_deref().unwrap_or("none"),
            ctx.phash_conf,
            total_candidates
        ),
    );

    Json(SearchSubtitlesResponse {
        media_path: ctx.payload.media_path,
        language: ctx.payload.language,
        matched_media: ctx.matched_media,
        moviehash: ctx.moviehash,
        fingerprint_preview: ctx.fingerprint_preview,
        total_candidates,
        candidates: ctx.candidates,
        timing: ctx.timing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn test_extract_query_sample_off_runtime_missing_media() {
        // Runs on the blocking pool even from a single-threaded runtime.
        let missing = std::env::temp_dir().join("dbsrt_missing_relay_media.mkv");
        let (_, sample, existing) = extract_query_sample_off_runtime(&missing, &[]).await;
        assert!(sample.is_empty());
        assert!(existing.is_none());
    }
    use dbsrt_core::{
        CertifiedMediaCoupling, CertifiedSubtitleRecord, LangStatus, MockSubtitleProvider,
    };
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_remote_relay_non_success_logs_the_http_status() {
        let mock_remote = axum::Router::new().route(
            "/v1/recognize",
            axum::routing::post(|| async { axum::http::StatusCode::TOO_MANY_REQUESTS }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, mock_remote).await.unwrap() });

        let state = test_app_state();
        let endpoint = format!("http://{}/v1/recognize", addr);
        let res =
            dispatch_remote_relay_call(&state, &endpoint, &RecognizeRequest::default(), 5000).await;

        assert!(res.is_none());
        let logs = state.log_buffer.read().unwrap();
        assert!(
            logs.iter()
                .any(|l| l.tag == "RELAY" && l.message.contains("HTTP 429")),
            "relay log must carry the remote HTTP status"
        );
    }

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
        let srt_dir = root.join("tests/data");
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
        let state = AppState::with_options(
            phash_dir,
            srt_dir,
            Arc::new(mock_provider),
            crate::state::AppStateInitOptions {
                dbphash_path: Some(PathBuf::from("/nonexistent_test_catalog.dbphash")),
                extra_phash_dirs: Vec::new(),
                ahash_dir: None,
            },
        );
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

    #[test]
    fn test_stem_and_title_overlap_normalizes_delimiters() {
        // Legitimate matches with underscores and dots
        assert!(stem_and_title_overlap(
            "Miracle_Workers_(2019)_-_S02E09_-_Moving_Out_Part_1_(1080p_AMZN_WEB-DL_x265_t3nzin)_mid15s",
            "Miracle Workers"
        ));
        assert!(stem_and_title_overlap(
            "Bullet.Train.2022.1080p.WEBRip.x264",
            "Bullet Train"
        ));

        // Disparate cross-media mismatches must be rejected
        assert!(!stem_and_title_overlap(
            "Pulp.Fiction.1994.1080p.BrRip.x264.YIFY_mid15s",
            "24 L'Entrai 24 0fps"
        ));
        assert!(!stem_and_title_overlap(
            "Mad.Max.1979.2160p.4K.BluRay.x265.10bit.AAC5.1-[YTS.MX]_mid15s",
            "L'été de kikujiro"
        ));
    }

    #[test]
    fn test_normalize_media_path_uri_handles_windows_and_posix_file_schemes() {
        assert_eq!(
            normalize_media_path_uri("file:///C:/Videos/Movie.mkv"),
            PathBuf::from("C:/Videos/Movie.mkv")
        );
        assert_eq!(
            normalize_media_path_uri("file:///d:/Media/Show.mp4"),
            PathBuf::from("d:/Media/Show.mp4")
        );
        assert_eq!(
            normalize_media_path_uri("file:///var/media/Videos/clip.mkv"),
            PathBuf::from("/var/media/Videos/clip.mkv")
        );
    }

    fn build_mock_remote_router() -> axum::Router {
        use crate::dtos::{
            AHashMatchCandidate, AHashSearchRequest, AHashSearchResponse,
            CouplingsOverviewResponse, StudioMediaCoupling,
        };

        axum::Router::new()
            .route(
                "/v1/search/ahash",
                axum::routing::post(|Json(req): Json<AHashSearchRequest>| async move {
                    assert!(
                        req.ahash_bytes.is_some(),
                        "agent relay must send base64 ahash_bytes"
                    );
                    let cand = AHashMatchCandidate {
                        media_name: "Remote.Reference.Movie.2024".to_string(),
                        file_path: "Remote.Reference.Movie.2024.86.1fps.ahash".to_string(),
                        ref_frame_offset: 5166,
                        offset_sec: 60.0,
                        offset_ms: 60_000,
                        mean_distance: 4.5,
                        score: 0.93,
                        snr_bits: 18.0,
                        audio_fps: 86.1328125,
                    };
                    Json(AHashSearchResponse {
                        success: true,
                        best_match: Some(cand.clone()),
                        candidates: vec![cand],
                        total_catalog_files: 1,
                        total_catalog_frames: 10000,
                        elapsed_seconds: 0.002,
                    })
                }),
            )
            .route(
                "/v1/couplings",
                axum::routing::get(|| async move {
                    let sub = CertifiedSubtitleRecord {
                        name: "Remote.Reference.Movie.2024.fr.srt".to_string(),
                        srt_path: "Remote.Reference.Movie.2024.fr.srt".to_string(),
                        lang: "fr".to_string(),
                        lang_status: LangStatus::Confirmed("fr".to_string()),
                        subtitles_hash:
                            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                                .to_string(),
                        certified: true,
                        cue_count: 120,
                        sub_frames: Vec::new(),
                    };
                    let coupling = StudioMediaCoupling {
                        media_hash: "remote_media_hash_1".to_string(),
                        media_name: "Remote.Reference.Movie.2024".to_string(),
                        phash_filename: "Remote.Reference.Movie.2024.phash".to_string(),
                        fps: 24.0,
                        frame_count: 1000,
                        in_postgres: false,
                        subtitles: vec![sub],
                    };
                    Json(CouplingsOverviewResponse {
                        postgres_connected: false,
                        postgres_counts: None,
                        local_registry_path: String::new(),
                        indexed_phash_files: 1,
                        couplings: vec![coupling],
                        catalog_phashes: Vec::new(),
                    })
                }),
            )
    }

    #[tokio::test]
    async fn test_agent_mode_relays_ahash_and_resolves_remote_certified_couplings() {
        use dbsrt_core::ahash::AhashHeader;

        let mock_remote = build_mock_remote_router();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, mock_remote).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let ahash_sidecar = tmp.path().join("cut_from_60s.86.1fps.ahash");
        let hashes: Vec<u64> = (0..32)
            .map(|i| 0x1234_5678_9ABC_DEF0u64.wrapping_mul((i as u64) + 1))
            .collect();
        let (_, band_bounds) =
            dbsrt_core::ahash::HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
        let header = AhashHeader {
            version: 1,
            sample_rate: 5512,
            frame_size: 2048,
            hop_size: 64,
            num_bands: 33,
            spectral_lag: 2,
            frame_count: hashes.len() as u64,
            band_bounds,
        };
        AhashFile::write_to_path(&ahash_sidecar, &header, &hashes).unwrap();

        let mut state = AppState::with_options(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            Arc::new(MockSubtitleProvider::new()),
            crate::state::AppStateInitOptions::default(),
        );
        state.is_agent = true;
        state.remote_api = Some(format!("http://{addr}"));
        let state = Arc::new(state);

        let req = SearchSubtitlesRequest {
            media_path: ahash_sidecar.to_string_lossy().to_string(),
            language: "fr".to_string(),
            search_mode: Some("ahash".to_string()),
            query_title: None,
            season: None,
            episode: None,
            subtitles_dir: Some(tmp.path().to_string_lossy().to_string()),
            phash_dir: None,
            index_engine: None,
        };
        let res = search_handler(State(state), Json(req)).await.0;
        assert_eq!(
            res.matched_media.as_deref(),
            Some("Remote.Reference.Movie.2024")
        );
        assert!(
            res.candidates.iter().any(|c| c.source == "certified"),
            "agent must resolve remote certified candidate"
        );
        assert!(
            res.candidates
                .iter()
                .any(|c| c.source == "certified_synced" && c.shift_ms == Some(-60_000.0)),
            "agent must emit certified_synced candidate with -60000 ms shift"
        );
    }
}
