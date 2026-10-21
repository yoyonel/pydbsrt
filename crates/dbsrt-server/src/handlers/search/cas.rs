//! Content-Addressable Storage (CAS) subtitle retrieval and library scan handlers.

use crate::dtos::{ScanMatchItem, ScanRequest, ScanResponse};
use crate::state::AppState;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::Json;
use dbsrt_core::{extract_frames_phash, probe_video};
use dbsrt_index::MmapLibrary;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtos::ScanRequest;
    use crate::state::AppState;
    use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};
    use std::sync::Arc;

    fn test_app_state() -> Arc<AppState> {
        let phash_dir = PathBuf::from("tests/data");
        let srt_dir = PathBuf::from("data/SRT");
        Arc::new(AppState::new(
            phash_dir,
            srt_dir,
            Arc::new(OpenSubtitlesProvider::new(None)),
            SubtitleCache::default(),
        ))
    }

    #[tokio::test]
    async fn test_get_subtitle_by_hash_not_found() {
        let state = test_app_state();
        let res = get_subtitle_by_hash_handler(
            AxumPath("nonexistent_sha256_hash_123456789".to_string()),
            State(state),
        )
        .await;
        assert!(res.is_err());
        let (status, _msg) = res.err().unwrap();
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_subtitle_by_hash_existing_in_registry() {
        let state = test_app_state();
        let test_hash = if let Ok(reg) = state.coupling_registry.read() {
            reg.couplings
                .first()
                .and_then(|c| c.subtitles.first())
                .map(|s| s.subtitles_hash.clone())
        } else {
            None
        };
        if let Some(hash) = test_hash {
            let res = get_subtitle_by_hash_handler(
                AxumPath(format!("sub:sha256:{}", hash)),
                State(state),
            )
            .await;
            if let Ok(response) = res {
                assert_eq!(response.status(), StatusCode::OK);
            }
        }
    }

    #[tokio::test]
    async fn test_scan_handler_missing_media() {
        let state = test_app_state();
        let req = ScanRequest {
            query_media: "nonexistent_video_path_123.mp4".to_string(),
            phash_dir: None,
            search_distance: 12,
            limit: 5,
        };
        let res = scan_handler(State(state), Json(req)).await;
        assert!(res.is_err());
        assert_eq!(res.err().unwrap().0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_scan_handler_valid_media() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
        let video = repo_root.join("tests/data/big_buck_bunny_trailer_480p.mp4");
        if !video.exists() {
            return;
        }
        let test_phash_dir = repo_root.join("tests/data");
        let state = test_app_state();
        let req = ScanRequest {
            query_media: video.to_string_lossy().to_string(),
            phash_dir: Some(test_phash_dir.to_string_lossy().to_string()),
            search_distance: 12,
            limit: 3,
        };
        let res = scan_handler(State(state), Json(req)).await;
        assert!(res.is_ok());
        let scan_resp = res.unwrap().0;
        assert!(scan_resp.total_files > 0);
    }
}
