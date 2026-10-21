//! Telemetry, health, status, metrics and logs endpoints.

use crate::dtos::{HealthResponse, MetricsResponse, StatusResponse};
use crate::state::{AppState, ServerLogsResponse};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use dbsrt_index::{MmapDbphash, MmapLibrary};
use std::sync::Arc;

pub async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        service: "dbsrt-server".to_string(),
    })
}

pub async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let dirs = state.effective_phash_dirs(None);
    let mut indexed_phash_count = MmapLibrary::from_directories(&dirs)
        .map(|lib| lib.file_count())
        .unwrap_or(0);
    if let Some(ref reader) = state.dbphash_reader {
        indexed_phash_count += reader.media_entries().len();
    } else if let Some(ref dbphash) = state.dbphash_path {
        if let Ok(reader) = MmapDbphash::open(dbphash) {
            indexed_phash_count += reader.media_entries().len();
        }
    }

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
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        service: "dbsrt-server".to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        phash_dir: state.phash_dir.to_string_lossy().to_string(),
        subtitles_dir: state.subtitles_dir.to_string_lossy().to_string(),
        indexed_phash_count,
        certified_couplings_count,
        postgres_connected,
    })
}

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
        (total_us as f64 / total as f64) / 1000.0
    } else {
        0.0
    };

    Json(MetricsResponse {
        status: "ok".to_string(),
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
