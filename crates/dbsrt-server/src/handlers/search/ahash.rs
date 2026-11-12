//! Acoustic fingerprint search handler (`POST /v1/search/ahash`).
//!
//! Evaluates incoming audio sub-fingerprints against the in-memory catalog
//! via sliding cross-correlation and returns ranked media candidates.

use crate::constants::{DEFAULT_AHASH_FPS, DEFAULT_AHASH_MAX_DISTANCE, MILLIS_PER_SEC};
use crate::dtos::{AHashMatchCandidate, AHashSearchRequest, AHashSearchResponse};
use crate::services::phash::parse_fingerprint_hashes;
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use base64::prelude::*;
use dbsrt_index::{AHashSearchCriteria, MIN_QUERY_FRAMES};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

/// Decodes raw little-endian 64-bit integer hashes from a base64 string.
fn decode_base64_hashes(encoded: &str) -> Option<Vec<u64>> {
    let bytes = BASE64_STANDARD.decode(encoded.trim()).ok()?;
    let (chunks, _) = bytes.as_chunks::<8>();
    Some(chunks.iter().map(|c| u64::from_le_bytes(*c)).collect())
}

/// Resolves audio query hashes from explicit array, hex strings, or base64 bytes.
fn resolve_query_hashes(payload: &AHashSearchRequest) -> Option<Vec<u64>> {
    if let Some(ref h) = payload.hashes {
        if !h.is_empty() {
            return Some(h.clone());
        }
    }
    if let Some(ref fp) = payload.fingerprint {
        let parsed = parse_fingerprint_hashes(fp);
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }
    if let Some(ref b64) = payload.ahash_bytes {
        return decode_base64_hashes(b64);
    }
    None
}

/// Axum handler for `POST /v1/search/ahash`.
///
/// # Arguments
/// - `state`: Shared server state container (`Arc<AppState>`).
/// - `payload`: Incoming search request specifying hashes, thresholds and limit.
///
/// # Returns
/// `Result<Json<AHashSearchResponse>, (StatusCode, String)>` containing ranked candidate matches.
///
/// # Errors
/// Returns `StatusCode::BAD_REQUEST` if query hashes are absent or shorter than [`MIN_QUERY_FRAMES`].
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Increments telemetry request counters.
/// 2. Resolves query hashes from `hashes`, `fingerprint` (hex strings) or `ahash_bytes` (base64).
/// 3. Rejects queries shorter than [`MIN_QUERY_FRAMES`] (8 frames).
/// 4. Loads current atomic snapshot of [`dbsrt_index::AHashCatalog`].
/// 5. Executes parallel cross-correlation search across catalog via Rayon.
/// 6. Formats candidates, logs operational audit line, and returns JSON response.
pub async fn search_ahash_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AHashSearchRequest>,
) -> Result<Json<AHashSearchResponse>, (StatusCode, String)> {
    let t_start = Instant::now();
    state.metrics.total_requests.fetch_add(1, Ordering::Relaxed);
    state
        .metrics
        .ahash_search_requests
        .fetch_add(1, Ordering::Relaxed);

    let query_hashes = resolve_query_hashes(&payload).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing query audio hashes (must provide 'hashes', 'fingerprint', or 'ahash_bytes')"
                .to_string(),
        )
    })?;

    if query_hashes.len() < MIN_QUERY_FRAMES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Audio query too short (minimum {} frames required, got {})",
                MIN_QUERY_FRAMES,
                query_hashes.len()
            ),
        ));
    }

    let audio_fps = payload.audio_fps.unwrap_or(DEFAULT_AHASH_FPS);
    let max_distance = payload.max_distance.unwrap_or(DEFAULT_AHASH_MAX_DISTANCE);
    let criteria = AHashSearchCriteria {
        query: &query_hashes,
        audio_fps,
        max_distance,
        limit: payload.limit,
        target_media: payload.media_name.as_deref(),
    };

    let catalog = state.ahash_catalog.load();
    let total_catalog_files = catalog.entries.len();
    let total_catalog_frames = catalog.total_frames;
    let matches = catalog.search(&criteria);

    let candidates: Vec<AHashMatchCandidate> = matches
        .into_iter()
        .map(|m| AHashMatchCandidate {
            media_name: m.media_name,
            file_path: m.file_path,
            ref_frame_offset: m.ref_frame_offset,
            offset_sec: m.offset_sec,
            offset_ms: m.offset_ms,
            mean_distance: m.mean_distance,
            score: m.score,
            snr_bits: m.snr_bits,
            audio_fps: m.audio_fps,
        })
        .collect();

    let best_match = candidates.first().cloned();
    let success = best_match.is_some();
    let elapsed_seconds = t_start.elapsed().as_secs_f64();

    state
        .metrics
        .total_latency_us
        .fetch_add(t_start.elapsed().as_micros() as u64, Ordering::Relaxed);

    state.push_log(
        "AHASH",
        "INFO",
        format!(
            "Search: {} frames, found {} match(es) across {} files ({:.2}ms)",
            query_hashes.len(),
            candidates.len(),
            total_catalog_files,
            elapsed_seconds * MILLIS_PER_SEC
        ),
    );

    Ok(Json(AHashSearchResponse {
        success,
        best_match,
        candidates,
        total_catalog_files,
        total_catalog_frames,
        elapsed_seconds,
    }))
}
