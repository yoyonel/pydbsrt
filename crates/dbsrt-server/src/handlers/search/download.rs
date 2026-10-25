//! Subtitle download, extraction, retargeting, and sidecar caching handler.

use crate::constants::{
    DEFAULT_FALLBACK_FPS, DEFAULT_RECOGNITION_SEARCH_DISTANCE, MILLIS_PER_SEC,
    MIN_SIGNIFICANT_SHIFT_MS,
};
use crate::dtos::{DownloadSubtitleRequest, DownloadSubtitleResponse, SubtitleSource};
use crate::services::catalog::fetch_or_cache_remote_subtitle;
use crate::services::phash::compute_media_phash_shift_multi;
use crate::services::recognition::identify_media_by_phash_multi_with_reader;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use dbsrt_core::{extract_embedded_subtitle, probe_video, MediaMetadata, SubtitleFile};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub async fn download_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DownloadSubtitleRequest>,
) -> Json<DownloadSubtitleResponse> {
    state
        .metrics
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let media_path = PathBuf::from(&payload.media_path);
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());

    let parsed_source = match payload.source.parse::<SubtitleSource>() {
        Ok(s) => s,
        Err(other) => {
            return Json(DownloadSubtitleResponse {
                success: false,
                output_srt_path: String::new(),
                shift_frames: 0,
                shift_ms: 0.0,
                confidence_percent: 0.0,
                source: other.clone(),
                language: payload.language,
                error: Some(format!("Unknown subtitle source: {}", other)),
            });
        }
    };

    let (mut srt_file, source_name) = match parsed_source {
        SubtitleSource::Container => {
            let idx: usize = payload
                .candidate_id
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            match extract_embedded_subtitle(&media_path, idx) {
                Ok(raw) => match SubtitleFile::parse(&raw) {
                    Ok(s) => (s, SubtitleSource::Container.as_str().to_string()),
                    Err(e) => {
                        return Json(DownloadSubtitleResponse {
                            success: false,
                            output_srt_path: String::new(),
                            shift_frames: 0,
                            shift_ms: 0.0,
                            confidence_percent: 0.0,
                            source: payload.source,
                            language: payload.language,
                            error: Some(format!("Failed to parse container subtitle: {:#}", e)),
                        });
                    }
                },
                Err(e) => {
                    return Json(DownloadSubtitleResponse {
                        success: false,
                        output_srt_path: String::new(),
                        shift_frames: 0,
                        shift_ms: 0.0,
                        confidence_percent: 0.0,
                        source: payload.source,
                        language: payload.language,
                        error: Some(format!(
                            "Failed to extract container subtitle track: {:#}",
                            e
                        )),
                    });
                }
            }
        }
        SubtitleSource::Certified | SubtitleSource::Catalog => {
            let cand_id = payload.candidate_id.as_deref().unwrap_or_default();
            let is_cas_hash = cand_id.starts_with("sub:sha256:") || cand_id.starts_with("sha256:");
            let clean_hash = cand_id
                .trim_start_matches("sub:sha256:")
                .trim_start_matches("sha256:")
                .trim();

            let mut fetched_file: Option<SubtitleFile> = None;

            if !is_cas_hash {
                let local_path = PathBuf::from(cand_id);
                if local_path.exists() {
                    if let Ok(s) = SubtitleFile::open(&local_path) {
                        fetched_file = Some(s);
                    }
                }
            } else if let Ok(reg) = state.coupling_registry.read() {
                if let Some((_coupling, sub)) = reg.find_subtitle_by_hash(clean_hash) {
                    let p = if Path::new(&sub.srt_path).is_absolute()
                        || Path::new(&sub.srt_path).exists()
                    {
                        PathBuf::from(&sub.srt_path)
                    } else {
                        subtitles_dir.join(&sub.srt_path)
                    };
                    if p.exists() {
                        if let Ok(s) = SubtitleFile::open(&p) {
                            fetched_file = Some(s);
                        }
                    }
                }
            }

            if fetched_file.is_none() && state.remote_api.is_some() {
                if let Some(ref remote_url) = state.remote_api {
                    let fetch_target = if is_cas_hash {
                        clean_hash.to_string()
                    } else {
                        Path::new(cand_id)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(cand_id)
                            .to_string()
                    };

                    let url = format!(
                        "{}/v1/subtitles/{}",
                        remote_url.trim_end_matches('/'),
                        fetch_target
                    );
                    state.push_log(
                        "REMOTE_CAS",
                        "INFO",
                        format!("Fetching subtitle from remote Subtitle Service: {}", url),
                    );

                    let client = reqwest::Client::builder()
                        .connect_timeout(std::time::Duration::from_millis(
                            crate::constants::DEFAULT_RELAY_CONNECT_TIMEOUT_MS,
                        ))
                        .timeout(std::time::Duration::from_millis(
                            crate::constants::DEFAULT_DOWNLOAD_RELAY_TIMEOUT_MS,
                        ))
                        .build()
                        .unwrap_or_default();

                    match client.get(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            if let Ok(bytes) = resp.bytes().await {
                                match SubtitleFile::from_bytes(&bytes) {
                                    Ok(s) => {
                                        fetched_file = Some(s);
                                        let cache_dir = state.cache.base_dir();
                                        let _ = std::fs::create_dir_all(cache_dir);
                                        let cache_file =
                                            cache_dir.join(format!("{}.srt", clean_hash));
                                        let _ = std::fs::write(&cache_file, &bytes);
                                    }
                                    Err(e) => {
                                        state.push_log(
                                            "REMOTE_CAS",
                                            "ERROR",
                                            format!(
                                                "Failed to parse remote subtitle content: {:#}",
                                                e
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            if fetched_file.is_none() {
                let path = PathBuf::from(cand_id);
                match SubtitleFile::open(&path) {
                    Ok(s) => fetched_file = Some(s),
                    Err(e) => {
                        return Json(DownloadSubtitleResponse {
                            success: false,
                            output_srt_path: String::new(),
                            shift_frames: 0,
                            shift_ms: 0.0,
                            confidence_percent: 0.0,
                            source: payload.source,
                            language: payload.language,
                            error: Some(format!(
                                "Failed to resolve subtitle (CAS hash '{}'): {:#}",
                                clean_hash, e
                            )),
                        });
                    }
                }
            }

            let s = fetched_file.unwrap();
            (s, parsed_source.as_str().to_string())
        }
        SubtitleSource::OpenSubtitles => {
            let sub_id = payload.candidate_id.clone().unwrap_or_default();
            let meta = MediaMetadata::parse_from_path(&media_path);
            let cand = dbsrt_core::RemoteSubtitleCandidate {
                provider: SubtitleSource::OpenSubtitles.as_str().to_string(),
                subtitle_id: sub_id.clone(),
                release_name: None,
                language: payload.language.clone(),
                download_url: String::new(),
                hearing_impaired: false,
                fps: None,
                match_score: 1.0,
            };

            match fetch_or_cache_remote_subtitle(&state, &meta, &payload.language, &cand).await {
                Ok(s) => (s, SubtitleSource::OpenSubtitles.as_str().to_string()),
                Err(e) => {
                    return Json(DownloadSubtitleResponse {
                        success: false,
                        output_srt_path: String::new(),
                        shift_frames: 0,
                        shift_ms: 0.0,
                        confidence_percent: 0.0,
                        source: payload.source,
                        language: payload.language,
                        error: Some(format!(
                            "Failed to download/parse subtitle ID '{}': {:#}",
                            sub_id, e
                        )),
                    });
                }
            }
        }
    };

    let mut shift_frames = 0i64;
    let mut shift_ms = 0.0f64;
    let mut confidence_percent = 0.0f64;

    if let Some(explicit_shift) = payload.shift_ms {
        if explicit_shift.abs() > MIN_SIGNIFICANT_SHIFT_MS {
            shift_ms = explicit_shift;
            let fps = probe_video(&media_path)
                .map(|m| m.fps)
                .unwrap_or(DEFAULT_FALLBACK_FPS);
            shift_frames = (shift_ms / MILLIS_PER_SEC * fps).round() as i64;
            confidence_percent = 100.0;
        }
    } else if payload.resync && media_path.exists() {
        let reader_guard = state.dbphash_reader.load();
        let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_with_reader(
            &media_path,
            &phash_dirs,
            reader_guard.as_deref(),
            state.db_config.as_ref(),
            1.0,
            DEFAULT_RECOGNITION_SEARCH_DISTANCE,
        );
        if let Some((matched_stem, match_conf)) = matched {
            let (sf, sms, conf) = compute_media_phash_shift_multi(
                &media_path,
                &phash_dirs,
                &matched_stem,
                match_conf,
                target_fps,
                &existing_phash,
                12,
            );
            shift_frames = sf;
            shift_ms = sms;
            confidence_percent = conf;
        }
    }

    if shift_ms.abs() > 0.001 {
        srt_file.shift(shift_ms);
    }

    let out_dir = media_path
        .parent()
        .filter(|p| p.is_dir() && !p.to_string_lossy().is_empty())
        .unwrap_or_else(|| Path::new("/tmp"));
    let media_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "subtitles".to_string());

    let out_name = if shift_ms.abs() > 0.001 {
        format!("{}.synced.{}.srt", media_stem, payload.language)
    } else {
        format!("{}.{}.srt", media_stem, payload.language)
    };
    let out_path = out_dir.join(&out_name);

    let (final_path, was_fallback) = match srt_file.save(&out_path) {
        Ok(()) => (out_path, false),
        Err(e) => {
            let fallback_dir = state.cache.base_dir();
            if let Err(mkdir_err) = std::fs::create_dir_all(fallback_dir) {
                state.push_log(
                    "DOWNLOAD",
                    "ERROR",
                    format!(
                        "Failed to write subtitle to primary path {:?} ({:#}) and failed to create fallback cache dir {:?}: {:#}",
                        out_path, e, fallback_dir, mkdir_err
                    ),
                );
                return Json(DownloadSubtitleResponse {
                    success: false,
                    output_srt_path: String::new(),
                    shift_frames: 0,
                    shift_ms: 0.0,
                    confidence_percent: 0.0,
                    source: source_name,
                    language: payload.language,
                    error: Some(format!(
                        "Failed to write subtitle file to {:?} ({:#}) and cache fallback failed ({:#})",
                        out_path, e, mkdir_err
                    )),
                });
            }
            let fallback_path = fallback_dir.join(&out_name);
            match srt_file.save(&fallback_path) {
                Ok(()) => {
                    state.push_log(
                        "DOWNLOAD",
                        "WARN",
                        format!(
                            "Primary write to {:?} failed ({:#}), saved to fallback cache path {:?}",
                            out_path, e, fallback_path
                        ),
                    );
                    (fallback_path, true)
                }
                Err(fallback_err) => {
                    state.push_log(
                        "DOWNLOAD",
                        "ERROR",
                        format!(
                            "Primary write to {:?} failed ({:#}) and cache fallback to {:?} failed ({:#})",
                            out_path, e, fallback_path, fallback_err
                        ),
                    );
                    return Json(DownloadSubtitleResponse {
                        success: false,
                        output_srt_path: String::new(),
                        shift_frames: 0,
                        shift_ms: 0.0,
                        confidence_percent: 0.0,
                        source: source_name,
                        language: payload.language,
                        error: Some(format!(
                            "Failed to write subtitle to {:?} ({:#}) and fallback to {:?} failed ({:#})",
                            out_path, e, fallback_path, fallback_err
                        )),
                    });
                }
            }
        }
    };

    state.push_log(
        "DOWNLOAD",
        "INFO",
        format!(
            "Saved subtitle to {:?} (source='{}', lang='{}', shift={:.2}ms, fallback={})",
            final_path.file_name().unwrap_or_default(),
            source_name,
            payload.language,
            shift_ms,
            was_fallback
        ),
    );

    Json(DownloadSubtitleResponse {
        success: true,
        output_srt_path: final_path.to_string_lossy().to_string(),
        shift_frames,
        shift_ms,
        confidence_percent,
        source: source_name,
        language: payload.language,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtos::DownloadSubtitleRequest;
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
    async fn test_download_unknown_source() {
        let state = test_app_state();
        let req = DownloadSubtitleRequest {
            media_path: "video.mp4".to_string(),
            source: "unknown_bogus_source".to_string(),
            candidate_id: None,
            language: "fr".to_string(),
            resync: false,
            shift_ms: None,
            subtitles_dir: None,
            phash_dir: None,
        };
        let res = download_handler(State(state), Json(req)).await;
        assert!(!res.0.success);
        assert!(res.0.error.unwrap().contains("Unknown subtitle source"));
    }

    #[tokio::test]
    async fn test_download_container_missing_media() {
        let state = test_app_state();
        let req = DownloadSubtitleRequest {
            media_path: "nonexistent_video.mkv".to_string(),
            source: "container".to_string(),
            candidate_id: Some("0".to_string()),
            language: "fr".to_string(),
            resync: false,
            shift_ms: None,
            subtitles_dir: None,
            phash_dir: None,
        };
        let res = download_handler(State(state), Json(req)).await;
        assert!(!res.0.success);
        assert!(res
            .0
            .error
            .unwrap()
            .contains("Failed to extract container subtitle track"));
    }

    #[tokio::test]
    async fn test_download_catalog_missing_file() {
        let state = test_app_state();
        let req = DownloadSubtitleRequest {
            media_path: "tests/data/big_buck_bunny_trailer_480p.mp4".to_string(),
            source: "catalog".to_string(),
            candidate_id: Some("nonexistent_path_to_subtitle.srt".to_string()),
            language: "fr".to_string(),
            resync: false,
            shift_ms: None,
            subtitles_dir: None,
            phash_dir: None,
        };
        let res = download_handler(State(state), Json(req)).await;
        assert!(!res.0.success);
        assert!(res.0.error.unwrap().contains("Failed to resolve subtitle"));
    }

    #[tokio::test]
    async fn test_download_opensubtitles_unconfigured_error() {
        let state = test_app_state();
        let req = DownloadSubtitleRequest {
            media_path: "tests/data/big_buck_bunny_trailer_480p.mp4".to_string(),
            source: "opensubtitles".to_string(),
            candidate_id: Some("sub_id_99999999".to_string()),
            language: "fr".to_string(),
            resync: false,
            shift_ms: None,
            subtitles_dir: None,
            phash_dir: None,
        };
        let res = download_handler(State(state), Json(req)).await;
        assert!(!res.0.success);
        assert!(res
            .0
            .error
            .unwrap()
            .contains("Failed to download/parse subtitle"));
    }

    #[tokio::test]
    async fn test_download_catalog_existing_file_with_shift() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
        let srt_fixture = repo_root.join("tests/data/big_buck_bunny_trailer_480p.en.srt");
        let video_fixture = repo_root.join("tests/data/big_buck_bunny_trailer_480p.mp4");
        if !srt_fixture.exists() || !video_fixture.exists() {
            return;
        }
        let tmp_media =
            std::env::temp_dir().join(format!("test_shift_media_{}.mp4", std::process::id()));
        let _ = std::fs::copy(&video_fixture, &tmp_media);
        let state = test_app_state();
        let tmp_out = std::env::temp_dir();
        let req = DownloadSubtitleRequest {
            media_path: tmp_media.to_string_lossy().to_string(),
            source: "catalog".to_string(),
            candidate_id: Some(srt_fixture.to_string_lossy().to_string()),
            language: "en".to_string(),
            resync: false,
            shift_ms: Some(1500.0),
            subtitles_dir: Some(tmp_out.to_string_lossy().to_string()),
            phash_dir: None,
        };
        let res = download_handler(State(state), Json(req)).await;
        assert!(res.0.success);
        assert_eq!(res.0.shift_ms, 1500.0);
        let out_file = PathBuf::from(&res.0.output_srt_path);
        if out_file.exists() {
            let _ = std::fs::remove_file(out_file);
        }
        let _ = std::fs::remove_file(tmp_media);
    }

    #[tokio::test]
    async fn test_download_certified_resync_branch() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
        let srt_fixture = repo_root.join("tests/data/big_buck_bunny_trailer_480p.en.srt");
        let video_fixture = repo_root.join("tests/data/big_buck_bunny_trailer_480p.mp4");
        let test_phash_dir = repo_root.join("tests/data");
        if !srt_fixture.exists() || !video_fixture.exists() {
            return;
        }
        let tmp_media =
            std::env::temp_dir().join(format!("test_resync_media_{}.mp4", std::process::id()));
        let _ = std::fs::copy(&video_fixture, &tmp_media);
        let state = test_app_state();
        let tmp_out = std::env::temp_dir();
        let req = DownloadSubtitleRequest {
            media_path: tmp_media.to_string_lossy().to_string(),
            source: "certified".to_string(),
            candidate_id: Some(srt_fixture.to_string_lossy().to_string()),
            language: "en".to_string(),
            resync: true,
            shift_ms: None,
            subtitles_dir: Some(tmp_out.to_string_lossy().to_string()),
            phash_dir: Some(test_phash_dir.to_string_lossy().to_string()),
        };
        let res = download_handler(State(state), Json(req)).await;
        assert!(res.0.success);
        let out_file = PathBuf::from(&res.0.output_srt_path);
        if out_file.exists() {
            let _ = std::fs::remove_file(out_file);
        }
        let _ = std::fs::remove_file(tmp_media);
    }
}
