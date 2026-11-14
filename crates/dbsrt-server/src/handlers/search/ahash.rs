//! Acoustic fingerprint search handler (`POST /v1/search/ahash`).
//!
//! Evaluates incoming audio sub-fingerprints against the in-memory catalog
//! via sliding cross-correlation and returns ranked media candidates.

use crate::constants::{
    AHASH_BYTES_PER_FRAME, DEFAULT_AHASH_FPS, DEFAULT_AHASH_MAX_DISTANCE,
    DEFAULT_AHASH_SEARCH_LIMIT, DEFAULT_RELAY_CONNECT_TIMEOUT_MS, DEFAULT_RELAY_TIMEOUT_MS,
    MAX_AHASH_HAMMING_DISTANCE, MAX_AHASH_QUERY_FRAMES, MAX_AHASH_SEARCH_LIMIT, MILLIS_PER_SEC,
};
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

/// Encodes a slice of 64-bit hashes as raw little-endian bytes in standard Base64.
///
/// # Arguments
/// * `hashes` - Contiguous slice of 64-bit sub-fingerprint hashes.
///
/// # Returns
/// Standard Base64 string encoding `hashes.len() * 8` little-endian bytes.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Used by `--agent` relay clients to transmit compact binary `.ahash` queries to `/v1/search/ahash`.
pub fn encode_base64_hashes(hashes: &[u64]) -> String {
    let mut bytes = Vec::with_capacity(hashes.len() * AHASH_BYTES_PER_FRAME);
    for &h in hashes {
        bytes.extend_from_slice(&h.to_le_bytes());
    }
    BASE64_STANDARD.encode(&bytes)
}

/// Decodes raw little-endian 64-bit integer hashes from a base64 string.
///
/// # Errors
/// Returns `Err((StatusCode::BAD_REQUEST, String))` if the payload is not valid base64 or its
/// decoded byte length is empty or not an exact multiple of 8 bytes.
fn decode_base64_hashes(encoded: &str) -> Result<Vec<u64>, (StatusCode, String)> {
    let bytes = BASE64_STANDARD.decode(encoded.trim()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid base64 in 'ahash_bytes': {e}"),
        )
    })?;
    if bytes.is_empty() || bytes.len() % AHASH_BYTES_PER_FRAME != 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid 'ahash_bytes' payload length: must be a non-zero multiple of {} bytes, got {}",
                AHASH_BYTES_PER_FRAME,
                bytes.len()
            ),
        ));
    }
    let (chunks, _) = bytes.as_chunks::<AHASH_BYTES_PER_FRAME>();
    Ok(chunks.iter().map(|c| u64::from_le_bytes(*c)).collect())
}

/// Resolves audio query hashes from explicit array, hex strings, or base64 bytes.
fn resolve_query_hashes(payload: &AHashSearchRequest) -> Result<Vec<u64>, (StatusCode, String)> {
    if let Some(ref h) = payload.hashes {
        if !h.is_empty() {
            return Ok(h.clone());
        }
    }
    if let Some(ref fp) = payload.fingerprint {
        let parsed = parse_fingerprint_hashes(fp);
        if !parsed.is_empty() {
            return Ok(parsed);
        }
    }
    if let Some(ref b64) = payload.ahash_bytes {
        return decode_base64_hashes(b64);
    }
    Err((
        StatusCode::BAD_REQUEST,
        "Missing query audio hashes (must provide 'hashes', 'fingerprint', or 'ahash_bytes')"
            .to_string(),
    ))
}

/// Validates numeric search parameters (`query_len`, `audio_fps`, `max_distance`, `limit`).
fn validate_ahash_search_params(
    query_len: usize,
    audio_fps: f64,
    max_distance: f64,
    limit: usize,
) -> Result<(), (StatusCode, String)> {
    if query_len < MIN_QUERY_FRAMES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Audio query too short (minimum {} frames required, got {})",
                MIN_QUERY_FRAMES, query_len
            ),
        ));
    }
    if query_len > MAX_AHASH_QUERY_FRAMES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Audio query too long (maximum {} frames allowed, got {})",
                MAX_AHASH_QUERY_FRAMES, query_len
            ),
        ));
    }
    if !audio_fps.is_finite() || audio_fps <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid 'audio_fps': must be finite and > 0.0, got {audio_fps}"),
        ));
    }
    if !max_distance.is_finite() || !(0.0..=MAX_AHASH_HAMMING_DISTANCE).contains(&max_distance) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid 'max_distance': must be finite and within 0.0..={MAX_AHASH_HAMMING_DISTANCE}, got {max_distance}"
            ),
        ));
    }
    if limit == 0 || limit > MAX_AHASH_SEARCH_LIMIT {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid 'limit': must be within 1..={MAX_AHASH_SEARCH_LIMIT}, got {limit}"),
        ));
    }
    Ok(())
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
/// Returns `StatusCode::BAD_REQUEST` if query hashes are absent, malformed, out of length bounds
/// (`MIN_QUERY_FRAMES..=MAX_AHASH_QUERY_FRAMES`), or if `audio_fps`, `max_distance`, or `limit`
/// are out of range.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Increments telemetry request counters.
/// 2. Resolves query hashes from `hashes`, `fingerprint` (hex strings) or `ahash_bytes` (base64).
/// 3. Validates query frame count (`8..=8_192`), `audio_fps`, `max_distance`, and `limit`.
/// 4. Acquires a heavy-task semaphore permit and loads current atomic snapshot of [`dbsrt_index::AHashCatalog`].
/// 5. Executes parallel search across catalog via Rayon inside `spawn_blocking`.
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

    let query_hashes = resolve_query_hashes(&payload)?;
    let audio_fps = payload.audio_fps.unwrap_or(DEFAULT_AHASH_FPS);
    let max_distance = payload.max_distance.unwrap_or(DEFAULT_AHASH_MAX_DISTANCE);
    validate_ahash_search_params(query_hashes.len(), audio_fps, max_distance, payload.limit)?;

    let _permit = state.heavy_task_semaphore.acquire().await.ok();
    let catalog = state.ahash_catalog.load_full();
    let total_catalog_files = catalog.entries.len();
    let total_catalog_frames = catalog.total_frames;
    let matches = {
        let cat = Arc::clone(&catalog);
        let owned_query = query_hashes.clone();
        let owned_media = payload.media_name.clone();
        let (limit, fps, dist) = (payload.limit, audio_fps, max_distance);
        tokio::task::spawn_blocking(move || {
            let c = AHashSearchCriteria {
                query: &owned_query,
                audio_fps: fps,
                max_distance: dist,
                limit,
                target_media: owned_media.as_deref(),
            };
            cat.search(&c)
        })
        .await
        .unwrap_or_default()
    };

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

/// Relays an extracted 64-bit audio hash slice to a remote `/v1/search/ahash` endpoint.
///
/// # Arguments
/// * `state` - Shared server state providing HTTP client, auth headers, and audit logger.
/// * `remote_url` - Base URL of the remote master server (e.g. `https://2026-pydbsrt.fr`).
/// * `hashes` - Contiguous slice of 64-bit acoustic sub-fingerprints.
/// * `audio_fps` - Query audio frame rate in Hz.
///
/// # Returns
/// `Some((AHashSearchResponse, search_ms))` when the remote master responds with a match,
/// or `None` on network error or non-success HTTP status.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Encodes `hashes` as compact little-endian Base64 (`ahash_bytes`).
/// 2. Dispatches `POST {remote_url}/v1/search/ahash` with auth headers and relay timeout.
/// 3. Deserializes [`AHashSearchResponse`] and records operational relay telemetry.
pub async fn relay_remote_ahash_search(
    state: &AppState,
    remote_url: &str,
    hashes: &[u64],
    audio_fps: f64,
) -> Option<(AHashSearchResponse, f64)> {
    let t_relay = Instant::now();
    let url = format!("{}/v1/search/ahash", remote_url.trim_end_matches('/'));
    let req_body = AHashSearchRequest {
        hashes: None,
        fingerprint: None,
        ahash_bytes: Some(encode_base64_hashes(hashes)),
        audio_fps: Some(audio_fps),
        max_distance: Some(DEFAULT_AHASH_MAX_DISTANCE),
        limit: DEFAULT_AHASH_SEARCH_LIMIT,
        media_name: None,
    };
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(
            DEFAULT_RELAY_CONNECT_TIMEOUT_MS,
        ))
        .timeout(std::time::Duration::from_millis(DEFAULT_RELAY_TIMEOUT_MS))
        .build()
        .unwrap_or_default();
    let mut req = client
        .post(&url)
        .header(reqwest::header::USER_AGENT, "dbsrt-agent/0.1.0")
        .json(&req_body);
    if let Some(ref api_key) = state.remote_api_key {
        req = req
            .header("X-API-Key", api_key)
            .header("Authorization", format!("Bearer {}", api_key));
    }

    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            state.push_log(
                "AGENT-RELAY",
                "WARN",
                format!("Remote aHash search HTTP {} from {}", r.status(), url),
            );
            return None;
        }
        Err(e) => {
            state.push_log(
                "AGENT-RELAY",
                "WARN",
                format!("Remote aHash search network error ({}): {}", url, e),
            );
            return None;
        }
    };

    let parsed = resp.json::<AHashSearchResponse>().await.ok()?;
    let search_ms = t_relay.elapsed().as_secs_f64() * MILLIS_PER_SEC;
    Some((parsed, search_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, AppStateInitOptions};
    use axum::extract::State;
    use dbsrt_core::providers::mock::MockSubtitleProvider;

    fn dummy_state() -> Arc<AppState> {
        let tmp = tempfile::tempdir().unwrap();
        Arc::new(AppState::with_options(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            Arc::new(MockSubtitleProvider::new()),
            AppStateInitOptions::default(),
        ))
    }

    #[tokio::test]
    async fn test_search_ahash_rejects_unaligned_base64_and_invalid_params() {
        let state = dummy_state();

        // 1. Base64 length not a multiple of 8 bytes -> 400 Bad Request
        let bad_b64 = BASE64_STANDARD.encode([1u8, 2, 3, 4, 5, 6, 7]);
        let req = AHashSearchRequest {
            hashes: None,
            fingerprint: None,
            ahash_bytes: Some(bad_b64),
            audio_fps: Some(DEFAULT_AHASH_FPS),
            max_distance: Some(DEFAULT_AHASH_MAX_DISTANCE),
            limit: 5,
            media_name: None,
        };
        let err = search_ahash_handler(State(Arc::clone(&state)), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("multiple of 8 bytes"));

        // 2. Query exceeding MAX_AHASH_QUERY_FRAMES (8_192) -> 400 Bad Request
        let oversized = vec![0xAA55_AA55_AA55_AA55u64; MAX_AHASH_QUERY_FRAMES + 1];
        let req_oversized = AHashSearchRequest {
            hashes: Some(oversized),
            fingerprint: None,
            ahash_bytes: None,
            audio_fps: Some(DEFAULT_AHASH_FPS),
            max_distance: Some(DEFAULT_AHASH_MAX_DISTANCE),
            limit: 5,
            media_name: None,
        };
        let err = search_ahash_handler(State(Arc::clone(&state)), Json(req_oversized))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("too long"));

        // 3. Invalid audio_fps (<= 0.0 or NaN) -> 400 Bad Request
        let valid_hashes = vec![0x1234_5678_9ABC_DEF0u64; MIN_QUERY_FRAMES];
        let req_bad_fps = AHashSearchRequest {
            hashes: Some(valid_hashes.clone()),
            fingerprint: None,
            ahash_bytes: None,
            audio_fps: Some(0.0),
            max_distance: Some(DEFAULT_AHASH_MAX_DISTANCE),
            limit: 5,
            media_name: None,
        };
        let err = search_ahash_handler(State(Arc::clone(&state)), Json(req_bad_fps))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("audio_fps"));

        // 4. Invalid max_distance (> 64.0) -> 400 Bad Request
        let req_bad_dist = AHashSearchRequest {
            hashes: Some(valid_hashes.clone()),
            fingerprint: None,
            ahash_bytes: None,
            audio_fps: Some(DEFAULT_AHASH_FPS),
            max_distance: Some(65.0),
            limit: 5,
            media_name: None,
        };
        let err = search_ahash_handler(State(Arc::clone(&state)), Json(req_bad_dist))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("max_distance"));

        // 5. Invalid limit (0 or > 50) -> 400 Bad Request
        let req_bad_limit = AHashSearchRequest {
            hashes: Some(valid_hashes),
            fingerprint: None,
            ahash_bytes: None,
            audio_fps: Some(DEFAULT_AHASH_FPS),
            max_distance: Some(DEFAULT_AHASH_MAX_DISTANCE),
            limit: MAX_AHASH_SEARCH_LIMIT + 1,
            media_name: None,
        };
        let err = search_ahash_handler(State(state), Json(req_bad_limit))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("limit"));
    }
}
