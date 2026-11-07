//! Telemetry endpoint for live search pipeline diagnostics.

use crate::dtos::LatestSearchTelemetry;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use std::sync::Arc;

/// Axum handler for `GET /v1/search/latest`: returns the real-time 4-stage timing breakdown
/// of the most recent search query processed by the server or relay agent.
///
/// # Arguments
/// - `state`: Shared server state container (`Arc<AppState>`).
///
/// # Returns
/// `Json<Option<LatestSearchTelemetry>>` containing timing breakdown or `None` if no search has run.
///
/// # Errors
/// None. Returns HTTP 200 with JSON body.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Acquires read lock on `state.latest_search` without blocking worker threads.
pub async fn latest_search_handler(
    State(state): State<Arc<AppState>>,
) -> Json<Option<LatestSearchTelemetry>> {
    let telemetry = state.latest_search.read().ok().and_then(|g| g.clone());
    Json(telemetry)
}

/// Returns true when a file path matches accepted video container extensions.
///
/// # Arguments
/// - `path`: File path to inspect.
///
/// # Returns
/// `true` if extension is mp4, mkv, avi, or webm; `false` otherwise.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Extracts path extension, lowercases ASCII bytes, and checks against supported video formats.
fn is_supported_video_file(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4" | "mkv" | "avi" | "webm"
    )
}

/// Scans directory entries and appends matching video snippet paths into collector.
///
/// # Arguments
/// - `dir`: Directory path to inspect.
/// - `out`: Mutable vector collecting string file paths.
///
/// # Returns
/// None.
///
/// # Errors
/// None. I/O read errors are ignored gracefully.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Flattens directory entries and appends paths satisfying [`is_supported_video_file`].
fn scan_video_snippets(dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_supported_video_file(&path) {
            out.push(path.to_string_lossy().to_string());
        }
    }
}

/// Axum handler for `GET /v1/snippets`: enumerates available snippet video files
/// located in `data/snippets/` for interactive UI evaluation.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Json<Vec<String>>` sorted list of relative snippet file paths.
///
/// # Errors
/// None. Returns empty list if directory is unreadable or empty.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// 1. Reads candidate directories (`data/snippets`, `./snippets`).
/// 2. Filters files matching video extensions via [`scan_video_snippets`].
/// 3. Sorts filenames alphabetically and wraps in JSON response.
pub async fn list_snippets_handler() -> Json<Vec<String>> {
    let mut snippets = Vec::new();
    let candidates = [
        std::path::Path::new("data/snippets"),
        std::path::Path::new("snippets"),
    ];

    for dir in &candidates {
        scan_video_snippets(dir, &mut snippets);
        if !snippets.is_empty() {
            break;
        }
    }

    snippets.sort();
    Json(snippets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{MockSubtitleProvider, SubtitleCache};
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_latest_search_telemetry_lifecycle() {
        let state = Arc::new(AppState::new(
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
            Arc::new(MockSubtitleProvider::default()),
            SubtitleCache::default(),
        ));

        // 1. Initially empty
        let initial = latest_search_handler(State(state.clone())).await;
        assert!(initial.0.is_none());

        // 2. Record telemetry
        let record = LatestSearchTelemetry {
            timestamp: "12:00:00.000".to_string(),
            media_path: "test.mp4".to_string(),
            filename: "test.mp4".to_string(),
            matched_media: Some("Pulp Fiction".to_string()),
            confidence_percent: 98.5,
            ffmpeg_ms: 250.0,
            wan_rtt_ms: 700.0,
            mih_match_ms: 8.5,
            srt_disk_ms: 45.0,
            moviehash_ms: 0.2,
            total_ms: 1003.7,
            ffmpeg_pct: 24.9,
            wan_pct: 69.7,
            mih_pct: 0.8,
            srt_pct: 4.5,
            is_relay: true,
            total_candidates: 3,
            matching_tier: "Tier 1 MIH".to_string(),
        };
        state.record_latest_search(record);

        // 3. Inspect updated telemetry
        let updated = latest_search_handler(State(state)).await;
        assert!(updated.0.is_some());
        let res = updated.0.unwrap();
        assert_eq!(res.filename, "test.mp4");
        assert_eq!(res.matched_media.as_deref(), Some("Pulp Fiction"));
        assert_eq!(res.total_candidates, 3);
        assert!((res.total_ms - 1003.7).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_list_snippets_handler_returns_json() {
        let res = list_snippets_handler().await;
        // Verify response can be serialized / non-panicking
        let _ = serde_json::to_string(&res.0).unwrap();
    }
}
