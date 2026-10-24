//! Catalog management, reload and synchronization HTTP handlers.

use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

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
