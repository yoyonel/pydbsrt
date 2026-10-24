//! Catalog management, reload, ingestion and compaction HTTP handlers.

use crate::dtos::{CatalogIngestRequest, CatalogIngestResponse};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// Handler for `GET /v1/catalog/status`.
pub async fn catalog_status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = state.catalog_status();
    (StatusCode::OK, Json(status))
}

/// Handler for `POST /v1/catalog/reload` and `POST /v1/reload`.
/// Asynchronously reloads `catalog.dbphash`, loose `.phash` files, and `couplings.json`.
/// Pre-warms MIH indices in the background before wait-free atomic pointer swap.
pub async fn catalog_reload_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let state_clone = state.clone();
    let res = tokio::task::spawn_blocking(move || state_clone.reload_catalog()).await;

    match res {
        Ok(Ok(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(Err(e)) => {
            let err_msg = e.to_string();
            state.push_log("CATALOG", "ERROR", format!("Reload failed: {}", err_msg));
            let status = if err_msg.contains("already in progress") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(serde_json::json!({
                    "success": false,
                    "error": err_msg,
                })),
            )
                .into_response()
        }
        Err(e) => {
            state.push_log("CATALOG", "ERROR", format!("Task join error: {}", e));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": format!("Worker task panicked: {}", e),
                })),
            )
                .into_response()
        }
    }
}

/// Handler for `POST /v1/catalog/ingest`.
/// Ingests a new media stream into the RAM Delta Index in O(1) time without rewriting `catalog.dbphash`.
pub async fn catalog_ingest_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CatalogIngestRequest>,
) -> impl IntoResponse {
    let t0 = std::time::Instant::now();

    // 1. Resolve raw frame hashes and filename
    let (hashes, filename) = if let Some(ref phash_path_str) = payload.phash_path {
        let p = std::path::Path::new(phash_path_str);
        if !p.exists() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false,
                    "error": format!("File not found: {}", phash_path_str),
                })),
            )
                .into_response();
        }
        match dbsrt_core::read_phash_file(p) {
            Ok(h) => {
                let name = payload.filename.unwrap_or_else(|| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
                (h, name)
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("Failed reading .phash file: {}", e),
                    })),
                )
                    .into_response();
            }
        }
    } else if let Some(h) = payload.hashes {
        let name = payload
            .filename
            .unwrap_or_else(|| "unnamed_media".to_string());
        (h, name)
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": "Either phash_path or hashes must be provided",
            })),
        )
            .into_response();
    };

    let fps = payload.fps.unwrap_or(24.0);
    let total_frames = hashes.len();
    let clean_title = dbsrt_core::MediaMetadata::parse(&filename).clean_title;

    let state_clone = state.clone();
    let res = tokio::task::spawn_blocking(move || {
        state_clone.ingest_delta_frames(&filename, &hashes, fps)
    })
    .await;

    match res {
        Ok(Ok(media_id)) => {
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let delta_count = state
                .delta_index
                .read()
                .map(|d| d.media_count())
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(CatalogIngestResponse {
                    success: true,
                    media_id,
                    clean_title,
                    total_frames,
                    delta_medias_count: delta_count,
                    elapsed_ms,
                }),
            )
                .into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": e.to_string(),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Worker task panicked: {}", e),
            })),
        )
            .into_response(),
    }
}

/// Handler for `POST /v1/catalog/compact`.
/// Flushes the Delta Index and Base into a consolidated `catalog.dbphash` and triggers hot-reload.
pub async fn catalog_compact_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let state_clone = state.clone();
    let res = tokio::task::spawn_blocking(move || state_clone.compact_catalog()).await;

    match res {
        Ok(Ok(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": e.to_string(),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Worker task panicked: {}", e),
            })),
        )
            .into_response(),
    }
}
