//! Telemetry, health, status, metrics and logs endpoints.
//!
//! Exposes lightweight liveness (`/health`, `/v1/health`), constant-time server status
//! (`/v1/status`), atomic request telemetry (`/v1/metrics`), and in-memory log buffer
//! snapshots (`/v1/logs`).

use crate::dtos::{HealthResponse, MetricsResponse, StatusResponse};
use crate::state::{AppState, ServerLogsResponse};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// Canonical service identifier emitted in health and status telemetry payloads.
const SERVICE_NAME: &str = "dbsrt-server";

/// Nominal operational status label emitted by health, status, and metrics endpoints.
const STATUS_OK: &str = "ok";

/// Microseconds-to-milliseconds conversion factor (`1 ms = 1,000 µs`).
const MICROS_PER_MILLISECOND: f64 = 1_000.0;

/// Handles `GET /health` and `GET /v1/health` liveness probes.
///
/// # Arguments
/// None.
///
/// # Returns
/// HTTP `200 OK` JSON [`HealthResponse`] containing `status: "ok"`, Cargo package version,
/// and canonical service name.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Executes in constant time with zero lock acquisitions, zero disk I/O, and zero database probes.
pub async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: STATUS_OK.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        service: SERVICE_NAME.to_string(),
    })
}

/// Handles `GET /v1/status` daemon status and index summary probes.
///
/// # Arguments
/// * `State(state)` - Shared [`AppState`] reference holding pre-loaded catalog readers,
///   delta index, acoustic `AhashEngine`, coupling registry, and optional PostgreSQL configuration.
///
/// # Returns
/// HTTP `200 OK` JSON [`StatusResponse`] reporting uptime, configured directories,
/// distinct indexed visual (`pHash`) and acoustic (`aHash`) media counts, certified subtitle
/// couplings count, and PostgreSQL connectivity.
///
/// # Panics
/// Never panics (gracefully defaults to `0` if an internal `RwLock` is poisoned).
///
/// # Data Flow & Invariants
/// - Queries [`AppState::indexed_media_count`] (`base + delta + loose`) and
///   [`AppState::indexed_ahash_count`] (`MmapDbahash` base + delta) in $\mathcal{O}(1)$
///   in-memory time with **zero** filesystem directory scans or ad-hoc `mmap` allocations.
/// - Preserves [`AppState::with_options`]'s anti-bloat invariant: when compiled
///   `catalog.dbphash` / `catalog.dbahash` archives are loaded, loose files already packed
///   into base archives are neither re-mapped on Tokio worker threads nor double-counted.
pub async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let indexed_phash_count = state.indexed_media_count();
    let indexed_ahash_count = state.indexed_ahash_count();

    let certified_couplings_count = state
        .coupling_registry
        .read()
        .map(|r| r.couplings.iter().map(|c| c.subtitles.len()).sum())
        .unwrap_or(0);

    let postgres_connected = state
        .db_config
        .as_ref()
        .is_some_and(|cfg| cfg.with_client(|_| Ok(())).is_ok());

    Json(StatusResponse {
        status: STATUS_OK.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        service: SERVICE_NAME.to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        phash_dir: state.phash_dir.to_string_lossy().to_string(),
        subtitles_dir: state.subtitles_dir.to_string_lossy().to_string(),
        indexed_phash_count,
        indexed_ahash_count,
        certified_couplings_count,
        postgres_connected,
    })
}

/// Handles `GET /v1/metrics` atomic request and latency counters.
///
/// # Arguments
/// * `State(state)` - Shared [`AppState`] reference holding lock-free atomic [`ServerMetrics`](crate::state::ServerMetrics).
///
/// # Returns
/// HTTP `200 OK` JSON [`MetricsResponse`] with cumulative request counts and mean latency in milliseconds.
///
/// # Panics
/// Never panics (`total == 0` guards against division by zero).
///
/// # Data Flow & Invariants
/// Reads `Relaxed` atomic counters without locks or allocations and converts cumulative microseconds
/// to milliseconds via [`MICROS_PER_MILLISECOND`].
pub async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let total = state
        .metrics
        .total_requests
        .load(std::sync::atomic::Ordering::Relaxed);
    let total_us = state
        .metrics
        .total_latency_us
        .load(std::sync::atomic::Ordering::Relaxed);
    let avg_latency_ms = if total > 0 {
        (total_us as f64 / total as f64) / MICROS_PER_MILLISECOND
    } else {
        0.0
    };

    Json(MetricsResponse {
        status: STATUS_OK.to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        total_requests: total,
        recognize_requests: state
            .metrics
            .recognize_requests
            .load(std::sync::atomic::Ordering::Relaxed),
        search_requests: state
            .metrics
            .search_requests
            .load(std::sync::atomic::Ordering::Relaxed),
        ahash_search_requests: state
            .metrics
            .ahash_search_requests
            .load(std::sync::atomic::Ordering::Relaxed),
        cache_hits: state
            .metrics
            .cache_hits
            .load(std::sync::atomic::Ordering::Relaxed),
        remote_errors: state
            .metrics
            .remote_errors
            .load(std::sync::atomic::Ordering::Relaxed),
        avg_latency_ms,
    })
}

/// Handles `GET /v1/logs` ring-buffer log retrieval.
///
/// # Arguments
/// * `State(state)` - Shared [`AppState`] reference holding the bounded `log_buffer`.
///
/// # Returns
/// HTTP `200 OK` JSON [`ServerLogsResponse`] containing buffered [`ServerLogEntry`](crate::state::ServerLogEntry) items.
///
/// # Panics
/// Never panics (returns an empty vector if the lock is poisoned).
///
/// # Data Flow & Invariants
/// Clones at most `LOG_BUFFER_CAPACITY` entries under a short-lived shared read lock.
pub async fn logs_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let logs = state
        .log_buffer
        .read()
        .map(|b| b.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    Json(ServerLogsResponse {
        count: logs.len(),
        logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppStateInitOptions;
    use axum::body::to_bytes;
    use dbsrt_core::ahash::{AhashFile, AhashHeader, HaitsmaKalkerExtractor};
    use dbsrt_core::{MediaMetadata, MediaType, OpenSubtitlesProvider};
    use dbsrt_index::DbphashBuilder;

    const TEST_BODY_LIMIT_BYTES: usize = 65_536;
    const TEST_DURATION_MS: u64 = 120_000;
    const TEST_FPS: f64 = 24.0;
    const TEST_HASH_A: u64 = 0x0123_4567_89ab_cdef;
    const TEST_HASH_B: u64 = 0xfedc_ba98_7654_3210;
    const TEST_AHASH_SAMPLE_RATE: u32 = 5_512;
    const TEST_AHASH_FRAME_SIZE: usize = 2_048;
    const TEST_AHASH_HOP_SIZE: u16 = 64;
    const TEST_AHASH_NUM_BANDS: u8 = 33;
    const TEST_AHASH_SPECTRAL_LAG: u8 = 2;
    const TEST_AHASH_FRAME_COUNT: usize = 64;

    #[tokio::test]
    async fn status_handler_uses_o1_indexed_count_without_double_counting_loose_files() {
        let tmp_dir =
            std::env::temp_dir().join(format!("dbsrt_health_status_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let phash_dir = tmp_dir.join("loose_phash");
        let ahash_dir = tmp_dir.join("ahash");
        let srt_dir = tmp_dir.join("srt");
        let dbphash_path = tmp_dir.join("catalog.dbphash");
        std::fs::create_dir_all(&phash_dir).expect("create phash_dir");
        std::fs::create_dir_all(&ahash_dir).expect("create ahash_dir");
        std::fs::create_dir_all(&srt_dir).expect("create srt_dir");

        dbsrt_core::write_phash_file(
            &phash_dir.join("Loose.Movie.2024.24.0fps.phash"),
            &[TEST_HASH_A, TEST_HASH_B],
        )
        .expect("write loose phash");

        let (_bins, band_bounds) = HaitsmaKalkerExtractor::compute_band_layout(
            TEST_AHASH_SAMPLE_RATE,
            TEST_AHASH_FRAME_SIZE,
        );
        let ahash_frames: Vec<u64> = (0..TEST_AHASH_FRAME_COUNT)
            .map(|i| TEST_HASH_A.wrapping_add(i as u64))
            .collect();
        let ahash_header = AhashHeader {
            version: 1,
            sample_rate: TEST_AHASH_SAMPLE_RATE,
            frame_size: TEST_AHASH_FRAME_SIZE as u16,
            hop_size: TEST_AHASH_HOP_SIZE,
            num_bands: TEST_AHASH_NUM_BANDS,
            spectral_lag: TEST_AHASH_SPECTRAL_LAG,
            frame_count: ahash_frames.len() as u64,
            band_bounds,
        };
        AhashFile::write_to_path(
            &ahash_dir.join("0123456789abcdef.86.1fps.ahash"),
            &ahash_header,
            &ahash_frames,
        )
        .expect("write test ahash");

        let mut builder = DbphashBuilder::new();
        let meta = MediaMetadata {
            raw_title: "Packed.Movie.2024.mkv".to_string(),
            clean_title: "Packed Movie".to_string(),
            media_type: MediaType::Movie,
            year: Some(2024),
            season: None,
            episode: None,
            resolution: Some("1080p".to_string()),
            source: None,
            release_group: None,
            moviehash: None,
        };
        let frames = [(TEST_HASH_A, 0u32), (TEST_HASH_B, 1_000u32)];
        builder.add_media(&meta, TEST_DURATION_MS, TEST_FPS, &frames, &frames);
        builder.write_to_file(&dbphash_path).expect("write dbphash");

        let state = Arc::new(AppState::with_options(
            phash_dir,
            srt_dir,
            Arc::new(OpenSubtitlesProvider::new(None)),
            AppStateInitOptions {
                dbphash_path: Some(dbphash_path),
                extra_phash_dirs: Vec::new(),
                ahash_dir: Some(ahash_dir),
                dbahash_path: None,
            },
        ));

        let resp = status_handler(State(state.clone())).await.into_response();
        let bytes = to_bytes(resp.into_body(), TEST_BODY_LIMIT_BYTES)
            .await
            .expect("body bytes");
        let parsed: StatusResponse = serde_json::from_slice(&bytes).expect("valid StatusResponse");
        assert_eq!(parsed.status, STATUS_OK);
        assert_eq!(parsed.indexed_phash_count, 1);
        assert_eq!(parsed.indexed_phash_count, state.indexed_media_count());
        assert_eq!(parsed.indexed_ahash_count, 1);
        assert_eq!(parsed.indexed_ahash_count, state.indexed_ahash_count());
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
