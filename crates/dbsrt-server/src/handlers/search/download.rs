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

/// Helper to construct uniform failure response payloads.
fn error_download_response(
    source: String,
    language: String,
    error: String,
) -> Json<DownloadSubtitleResponse> {
    Json(DownloadSubtitleResponse {
        success: false,
        output_srt_path: String::new(),
        shift_frames: 0,
        shift_ms: 0.0,
        confidence_percent: 0.0,
        source,
        language,
        error: Some(error),
    })
}

/// Extracts and parses an embedded subtitle stream from a video container.
///
/// # Arguments
/// * `media_path` - Path to source video file.
/// * `candidate_id` - Optional string representation of stream index.
///
/// # Returns
/// * `Result<SubtitleFile, String>` - Parsed subtitle file or descriptive error.
///
/// # Errors
/// * Returns `Err` if track extraction or SRT parsing fails.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Parses stream index defaulting to 0 if omitted or invalid.
/// 2. Invokes `extract_embedded_subtitle`.
/// 3. Parses raw bytes into structured `SubtitleFile`.
pub fn extract_container_subtitle(
    media_path: &Path,
    candidate_id: Option<&str>,
) -> Result<SubtitleFile, String> {
    let idx: usize = candidate_id.and_then(|s| s.parse().ok()).unwrap_or(0);
    let raw = extract_embedded_subtitle(media_path, idx)
        .map_err(|e| format!("Failed to extract container subtitle track: {:#}", e))?;
    SubtitleFile::parse(&raw).map_err(|e| format!("Failed to parse container subtitle: {:#}", e))
}

/// Request parameters for fetching a subtitle from an upstream remote relay endpoint.
struct RemoteFetchRequest<'a> {
    pub remote_url: &'a str,
    pub candidate_id: &'a str,
    pub clean_hash: &'a str,
    pub is_cas_hash: bool,
}

/// Fetches and caches a subtitle from an upstream remote relay server.
///
/// # Arguments
/// * `state` - Application state containing client configuration and cache.
/// * `req` - Request parameters targeting the remote subtitle.
///
/// # Returns
/// `Option<SubtitleFile>` containing parsed subtitle on success, or `None` on network/parsing error.
///
/// # Errors
/// Non-blocking, returns `None` and records structured logs on connection or parsing failure.
///
/// # Panics
/// Never panics under any execution condition.
///
/// # Data Flow & Invariants
/// Dispatches authenticated HTTP GET to `/v1/subtitles/{id}` on the upstream relay.
/// On successful HTTP 200 response, caches the raw `.srt` bytes locally under `clean_hash.srt`.
async fn fetch_remote_cas_subtitle(
    state: &AppState,
    req: RemoteFetchRequest<'_>,
) -> Option<SubtitleFile> {
    let fetch_target = if req.is_cas_hash {
        req.clean_hash.to_string()
    } else {
        Path::new(req.candidate_id)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(req.candidate_id)
            .to_string()
    };

    let url = format!(
        "{}/v1/subtitles/{}",
        req.remote_url.trim_end_matches('/'),
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

    let mut req_builder = client.get(&url);
    if let Some(ref api_key) = state.remote_api_key {
        req_builder = req_builder.header("X-API-Key", api_key);
    }

    let resp = req_builder.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    match SubtitleFile::from_bytes(&bytes) {
        Ok(s) => {
            let cache_dir = state.cache.base_dir();
            let _ = std::fs::create_dir_all(cache_dir);
            let cache_file = cache_dir.join(format!("{}.srt", req.clean_hash));
            let _ = std::fs::write(&cache_file, &bytes);
            Some(s)
        }
        Err(e) => {
            state.push_log(
                "REMOTE_CAS",
                "ERROR",
                format!("Failed to parse remote subtitle content: {:#}", e),
            );
            None
        }
    }
}

/// Resolves a subtitle by CAS SHA256 digest or filesystem path across local registry and remote relay.
///
/// # Arguments
/// * `state` - Application state with registry, cache, and optional remote relay configuration.
/// * `candidate_id` - Identifier which may be a CAS SHA256 digest or local filesystem path.
/// * `subtitles_dir` - Fallback directory for resolving relative subtitle paths.
///
/// # Returns
/// `Ok(SubtitleFile)` containing verified subtitle cues, or `Err(String)` describing lookup failure.
///
/// # Errors
/// Returns error if file cannot be found in local registry, remote relay, or local filesystem.
///
/// # Panics
/// Never panics under any execution condition.
///
/// # Data Flow & Invariants
/// 1. Detects whether `candidate_id` is a CAS SHA256 identifier (`sub:sha256:` / `sha256:`).
/// 2. If non-CAS, attempts direct local filesystem open.
/// 3. If CAS, queries in-memory `CouplingRegistry` for certified couplings and opens file.
/// 4. If still unresolved and `remote_api` is configured, queries remote `/v1/subtitles/{hash}` and caches result.
/// 5. As last fallback, tries opening `candidate_id` as direct filesystem path.
pub async fn fetch_cas_or_catalog_subtitle(
    state: &AppState,
    candidate_id: &str,
    subtitles_dir: &Path,
) -> Result<SubtitleFile, String> {
    let is_cas_hash =
        candidate_id.starts_with("sub:sha256:") || candidate_id.starts_with("sha256:");
    let clean_hash = candidate_id
        .trim_start_matches("sub:sha256:")
        .trim_start_matches("sha256:")
        .trim();

    let mut fetched_file: Option<SubtitleFile> = None;

    if !is_cas_hash {
        let local_path = PathBuf::from(candidate_id);
        if local_path.exists() {
            if let Ok(s) = SubtitleFile::open(&local_path) {
                fetched_file = Some(s);
            }
        }
    } else if let Ok(reg) = state.coupling_registry.read() {
        if let Some((_coupling, sub)) = reg.find_subtitle_by_hash(clean_hash) {
            let p = if Path::new(&sub.srt_path).is_absolute() || Path::new(&sub.srt_path).exists() {
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

    if fetched_file.is_none() {
        if let Some(ref remote_url) = state.remote_api {
            fetched_file = fetch_remote_cas_subtitle(
                state,
                RemoteFetchRequest {
                    remote_url,
                    candidate_id,
                    clean_hash,
                    is_cas_hash,
                },
            )
            .await;
        }
    }

    if fetched_file.is_none() {
        let path = PathBuf::from(candidate_id);
        match SubtitleFile::open(&path) {
            Ok(s) => fetched_file = Some(s),
            Err(e) => {
                return Err(format!(
                    "Failed to resolve subtitle (CAS hash '{}'): {:#}",
                    clean_hash, e
                ));
            }
        }
    }

    fetched_file.ok_or_else(|| format!("Subtitle not found for '{}'", clean_hash))
}

/// Fetches a subtitle from OpenSubtitles provider with local caching.
///
/// # Arguments
/// * `state` - Application state with cache and OpenSubtitles client.
/// * `media_path` - Path to media video file.
/// * `candidate_id` - Remote subtitle identifier.
/// * `language` - Target ISO language code.
///
/// # Returns
/// * `Result<SubtitleFile, String>` - Downloaded subtitle or error string.
///
/// # Errors
/// * Returns `Err` if provider fails or file parsing fails.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Extracts video metadata from `media_path`.
/// 2. Assembles `RemoteSubtitleCandidate`.
/// 3. Invokes `fetch_or_cache_remote_subtitle`.
pub async fn fetch_opensubtitles_subtitle(
    state: &AppState,
    media_path: &Path,
    candidate_id: Option<&str>,
    language: &str,
) -> Result<SubtitleFile, String> {
    let sub_id = candidate_id.unwrap_or_default();
    let meta = MediaMetadata::parse_from_path(media_path);
    let cand = dbsrt_core::RemoteSubtitleCandidate {
        provider: SubtitleSource::OpenSubtitles.as_str().to_string(),
        subtitle_id: sub_id.to_string(),
        release_name: None,
        language: language.to_string(),
        download_url: String::new(),
        hearing_impaired: false,
        fps: None,
        match_score: 1.0,
    };

    fetch_or_cache_remote_subtitle(state, &meta, language, &cand)
        .await
        .map_err(|e| format!("Failed to download/parse subtitle ID '{}': {:#}", sub_id, e))
}

/// Dispatches subtitle resolution according to declared subtitle source.
///
/// # Arguments
/// * `state` - Application state.
/// * `source` - Declared subtitle source enum.
/// * `media_path` - Path to source video.
/// * `candidate_id` - Optional candidate identifier or path.
/// * `language` - Target language code.
/// * `subtitles_dir` - Subtitle storage directory.
///
/// # Returns
/// * `Result<(SubtitleFile, String), String>` - Pair of resolved subtitle file and source name string.
///
/// # Errors
/// * Returns `Err` if underlying fetcher fails.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Directs to `extract_container_subtitle` for container tracks.
/// 2. Directs to `fetch_cas_or_catalog_subtitle` for certified/catalog files.
/// 3. Directs to `fetch_opensubtitles_subtitle` for OpenSubtitles downloads.
pub async fn resolve_subtitle_content(
    state: &AppState,
    source: SubtitleSource,
    media_path: &Path,
    candidate_id: Option<&str>,
    language: &str,
    subtitles_dir: &Path,
) -> Result<(SubtitleFile, String), String> {
    match source {
        SubtitleSource::Container => {
            let s = extract_container_subtitle(media_path, candidate_id)?;
            Ok((s, SubtitleSource::Container.as_str().to_string()))
        }
        SubtitleSource::Certified | SubtitleSource::Catalog => {
            let s = fetch_cas_or_catalog_subtitle(
                state,
                candidate_id.unwrap_or_default(),
                subtitles_dir,
            )
            .await?;
            Ok((s, source.as_str().to_string()))
        }
        SubtitleSource::OpenSubtitles => {
            let s = fetch_opensubtitles_subtitle(state, media_path, candidate_id, language).await?;
            Ok((s, SubtitleSource::OpenSubtitles.as_str().to_string()))
        }
    }
}

/// Calculates synchronization shift either from explicit offset or visual pHash recognition.
///
/// # Arguments
/// * `state` - Application state containing DB reader and configs.
/// * `payload` - Download request options (shift_ms, resync).
/// * `media_path` - Path to video file.
/// * `phash_dirs` - Directories storing reference .phash files.
///
/// # Returns
/// * `(i64, f64, f64)` - Tuple of `(shift_frames, shift_ms, confidence_percent)`.
///
/// # Errors
/// * Guarantee: Never returns Err; defaults to 0 on failure.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. If explicit `shift_ms` is supplied and exceeds `MIN_SIGNIFICANT_SHIFT_MS`: computes frames via `probe_video` fps.
/// 2. Otherwise if `resync` flag is set and video exists: performs visual fingerprint matching and calculates shift consensus.
/// 3. In all other cases: returns zero shift.
pub fn compute_subtitle_shift(
    state: &AppState,
    payload: &DownloadSubtitleRequest,
    media_path: &Path,
    phash_dirs: &[PathBuf],
) -> (i64, f64, f64) {
    if let Some(explicit_shift) = payload.shift_ms {
        if explicit_shift.abs() > MIN_SIGNIFICANT_SHIFT_MS {
            let fps = probe_video(media_path)
                .map(|m| m.fps)
                .unwrap_or(DEFAULT_FALLBACK_FPS);
            let frames = (explicit_shift / MILLIS_PER_SEC * fps).round() as i64;
            return (frames, explicit_shift, 100.0);
        }
    }

    if payload.resync && media_path.exists() {
        let reader_guard = state.dbphash_reader.load();
        let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_with_reader(
            media_path,
            phash_dirs,
            reader_guard.as_deref(),
            state.db_config.as_ref(),
            1.0,
            DEFAULT_RECOGNITION_SEARCH_DISTANCE,
        );
        if let Some((matched_stem, match_conf)) = matched {
            let (sf, sms, conf) = compute_media_phash_shift_multi(
                media_path,
                phash_dirs,
                &matched_stem,
                match_conf,
                target_fps,
                &existing_phash,
                12,
            );
            return (sf, sms, conf);
        }
    }

    (0, 0.0, 0.0)
}

/// Saves subtitle to target destination with automatic fallback to cache directory.
///
/// # Arguments
/// * `state` - Application state for logging and cache directory resolution.
/// * `srt_file` - Subtitle file to persist.
/// * `media_path` - Source video path (determines primary output directory and stem).
/// * `language` - Subtitle ISO language code.
/// * `shift_ms` - Applied synchronization shift in milliseconds.
///
/// # Returns
/// * `Result<(PathBuf, bool), String>` - Pair of persisted output path and whether fallback cache was used.
///
/// # Errors
/// * Returns `Err` if both primary write and fallback cache write fail.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Constructs standard filename (`.synced.<lang>.srt` if shifted, else `.<lang>.srt`).
/// 2. Attempts write to video's parent directory.
/// 3. If primary write fails (read-only filesystem, permissions), writes to `state.cache.base_dir()`.
/// 4. Logs warnings and errors to state log ring-buffer.
pub fn persist_retargeted_subtitle(
    state: &AppState,
    srt_file: &SubtitleFile,
    media_path: &Path,
    language: &str,
    shift_ms: f64,
) -> Result<(PathBuf, bool), String> {
    let out_dir = media_path
        .parent()
        .filter(|p| p.is_dir() && !p.to_string_lossy().is_empty())
        .unwrap_or_else(|| Path::new("/tmp"));
    let media_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "subtitles".to_string());

    let out_name = if shift_ms.abs() > 0.001 {
        format!("{}.synced.{}.srt", media_stem, language)
    } else {
        format!("{}.{}.srt", media_stem, language)
    };
    let out_path = out_dir.join(&out_name);

    match srt_file.save(&out_path) {
        Ok(()) => Ok((out_path, false)),
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
                return Err(format!(
                    "Failed to write subtitle file to {:?} ({:#}) and cache fallback failed ({:#})",
                    out_path, e, mkdir_err
                ));
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
                    Ok((fallback_path, true))
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
                    Err(format!(
                        "Failed to write subtitle to {:?} ({:#}) and fallback to {:?} failed ({:#})",
                        out_path, e, fallback_path, fallback_err
                    ))
                }
            }
        }
    }
}

/// Axum HTTP handler for `POST /v1/download`: resolves, retargets, and caches subtitles.
///
/// # Arguments
/// * `State(state)` - Shared server state.
/// * `Json(payload)` - Download and synchronization parameters.
///
/// # Returns
/// * `Json<DownloadSubtitleResponse>` - Structured response with file path, applied shift, and status.
///
/// # Errors
/// * Guarantee: Never returns HTTP 500; returns JSON payload with `success: false` and error message.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Increments request metrics and parses `SubtitleSource`.
/// 2. Resolves subtitle content via `resolve_subtitle_content`.
/// 3. Computes time shift via `compute_subtitle_shift` and mutates cue timestamps if needed.
/// 4. Persists retargeted subtitle via `persist_retargeted_subtitle`.
/// 5. Logs outcome and returns structured response.
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
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let override_phash_dir = payload.phash_dir.as_deref().map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());

    let parsed_source = match payload.source.parse::<SubtitleSource>() {
        Ok(s) => s,
        Err(other) => {
            return error_download_response(
                other.clone(),
                payload.language,
                format!("Unknown subtitle source: {}", other),
            );
        }
    };

    let (mut srt_file, source_name) = match resolve_subtitle_content(
        &state,
        parsed_source,
        &media_path,
        payload.candidate_id.as_deref(),
        &payload.language,
        &subtitles_dir,
    )
    .await
    {
        Ok(res) => res,
        Err(err_msg) => {
            return error_download_response(payload.source, payload.language, err_msg);
        }
    };

    let (shift_frames, shift_ms, confidence_percent) =
        compute_subtitle_shift(&state, &payload, &media_path, &phash_dirs);

    if shift_ms.abs() > 0.001 {
        srt_file.shift(shift_ms);
    }

    let (final_path, was_fallback) = match persist_retargeted_subtitle(
        &state,
        &srt_file,
        &media_path,
        &payload.language,
        shift_ms,
    ) {
        Ok(res) => res,
        Err(err_msg) => {
            return error_download_response(source_name, payload.language, err_msg);
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
