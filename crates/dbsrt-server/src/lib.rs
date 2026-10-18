//! Library core for dbsrt-server HTTP API.

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use dbsrt_core::{
    detect_cuts, extract_embedded_subtitle, extract_frames_phash, inspect_container, probe_video,
    read_phash_file, write_phash_file, CutItem, MediaMetadata, OpenSubtitlesProvider,
    SubtitleCache, SubtitleFile, SubtitleProvider,
};
use dbsrt_index::{BkItem, BkTree, MmapLibrary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub const DISCOVERY_FILE: &str = "/tmp/dbsrt-server.json";

pub fn write_discovery_file(addr: SocketAddr) -> Result<()> {
    let payload = serde_json::json!({
        "host": addr.ip().to_string(),
        "port": addr.port(),
        "url": format!("http://{}", addr),
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION")
    });
    std::fs::write(DISCOVERY_FILE, serde_json::to_string_pretty(&payload)?)
        .with_context(|| format!("Failed to write discovery file {:?}", DISCOVERY_FILE))?;
    Ok(())
}

pub fn remove_discovery_file() {
    let _ = std::fs::remove_file(DISCOVERY_FILE);
}

pub async fn bind_listener(
    host: &str,
    start_port: u16,
) -> Result<(tokio::net::TcpListener, SocketAddr)> {
    if start_port == 0 {
        let addr: SocketAddr = format!("{}:0", host).parse()?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        return Ok((listener, local_addr));
    }

    let mut current_port = start_port;
    loop {
        let addr: SocketAddr = format!("{}:{}", host, current_port).parse()?;
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                let local_addr = listener.local_addr()?;
                if current_port != start_port {
                    tracing::warn!(
                        "Port {} was already in use, successfully auto-selected port {}",
                        start_port,
                        current_port
                    );
                }
                return Ok((listener, local_addr));
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::AddrInUse && current_port < start_port + 20 =>
            {
                current_port += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub phash_dir: PathBuf,
    pub subtitles_dir: PathBuf,
    pub provider: Arc<dyn SubtitleProvider>,
    pub cache: SubtitleCache,
    pub start_time: Instant,
}

impl AppState {
    pub fn new(
        phash_dir: PathBuf,
        subtitles_dir: PathBuf,
        provider: Arc<dyn SubtitleProvider>,
        cache: SubtitleCache,
    ) -> Self {
        Self {
            phash_dir,
            subtitles_dir,
            provider,
            cache,
            start_time: Instant::now(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct AutoSyncRequest {
    pub media_path: String,
    #[serde(default = "default_lang")]
    pub language: String,
    pub phash_dir: Option<String>,
    pub subtitles_dir: Option<String>,
    #[serde(default)]
    pub search_distance: u32,
    #[serde(default = "default_extract_duration")]
    pub nb_seconds: f64,
}

fn default_lang() -> String {
    "en".to_string()
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AutoSyncResponse {
    pub success: bool,
    pub matched_media: Option<String>,
    pub language: String,
    pub shift_frames: i64,
    pub shift_ms: f64,
    pub output_srt_path: String,
    pub confidence_percent: f64,
    pub elapsed_seconds: f64,
    #[serde(default)]
    pub source: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub service: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StatusResponse {
    pub status: String,
    pub version: String,
    pub service: String,
    pub uptime_seconds: u64,
    pub phash_dir: String,
    pub subtitles_dir: String,
    pub indexed_phash_count: usize,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SearchSubtitlesRequest {
    pub media_path: String,
    #[serde(default = "default_lang")]
    pub language: String,
    pub subtitles_dir: Option<String>,
    pub query_title: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SubtitleCandidate {
    pub source: String,
    pub language: String,
    pub title: String,
    pub release: Option<String>,
    pub details: Option<String>,
    pub candidate_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SearchSubtitlesResponse {
    pub media_path: String,
    pub language: String,
    pub total_candidates: usize,
    pub candidates: Vec<SubtitleCandidate>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct DownloadSubtitleRequest {
    pub media_path: String,
    pub source: String,
    pub candidate_id: Option<String>,
    #[serde(default = "default_lang")]
    pub language: String,
    #[serde(default = "default_true")]
    pub resync: bool,
    pub subtitles_dir: Option<String>,
    pub phash_dir: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DownloadSubtitleResponse {
    pub success: bool,
    pub output_srt_path: String,
    pub shift_frames: i64,
    pub shift_ms: f64,
    pub confidence_percent: f64,
    pub source: String,
    pub language: String,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RecognizeRequest {
    pub media_path: String,
    #[serde(default)]
    pub search_distance: u32,
    #[serde(default = "default_extract_duration")]
    pub nb_seconds: f64,
    pub phash_dir: Option<String>,
}

fn default_extract_duration() -> f64 {
    1.0
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RecognizeResponse {
    pub found: bool,
    pub media_name_search: String,
    pub media_match: Option<String>,
    pub nb_offsets_match: usize,
    pub confidence: f64,
    pub timer_seconds: f64,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RetargetRequest {
    pub target_media: String,
    pub ref_subtitles: String,
    pub ref_media: Option<String>,
    pub phash_dir: Option<String>,
    #[serde(default)]
    pub search_distance: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RetargetResponse {
    pub success: bool,
    pub shift_frames: i64,
    pub shift_ms: f64,
    pub output_srt_path: String,
    pub confidence_percent: f64,
    pub elapsed_seconds: f64,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ScanRequest {
    pub query_media: String,
    pub phash_dir: Option<String>,
    #[serde(default)]
    pub search_distance: u32,
    #[serde(default = "default_scan_limit")]
    pub limit: usize,
}

fn default_scan_limit() -> usize {
    5
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ScanMatchItem {
    pub media_name: String,
    pub matching_frames: usize,
    pub confidence_percent: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ScanResponse {
    pub total_files: usize,
    pub total_frames: usize,
    pub elapsed_seconds: f64,
    pub top_matches: Vec<ScanMatchItem>,
}

/// Builds the Axum router for dbsrt-server with default providers.
pub fn create_router(phash_dir: PathBuf, subtitles_dir: PathBuf) -> Router {
    create_router_with_state(AppState::new(
        phash_dir,
        subtitles_dir,
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    ))
}

/// Builds the Axum router for dbsrt-server with custom AppState.
pub fn create_router_with_state(state: AppState) -> Router {
    let state = Arc::new(state);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/health", get(health_handler))
        .route("/v1/status", get(status_handler))
        .route("/v1/search", post(search_handler))
        .route("/v1/download", post(download_handler))
        .route("/v1/recognize", post(recognize_handler))
        .route("/v1/retarget", post(retarget_handler))
        .route("/v1/autosync", post(autosync_handler))
        .route("/v1/scan", post(scan_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        service: "dbsrt-server".to_string(),
    })
}

pub async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let indexed_phash_count = std::fs::read_dir(&state.phash_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("phash"))
                .count()
        })
        .unwrap_or(0);

    Json(StatusResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        service: "dbsrt-server".to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        phash_dir: state.phash_dir.to_string_lossy().to_string(),
        subtitles_dir: state.subtitles_dir.to_string_lossy().to_string(),
        indexed_phash_count,
    })
}

pub async fn search_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SearchSubtitlesRequest>,
) -> impl IntoResponse {
    let media_path = PathBuf::from(&payload.media_path);
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let mut candidates: Vec<SubtitleCandidate> = Vec::new();

    // 1. Inspect container if media exists
    if media_path.exists() {
        if let Ok(insp) = inspect_container(&media_path) {
            for stream in insp.subtitle_streams {
                let stream_lang = stream.language.as_deref().unwrap_or("und");
                let lang_match = payload.language == "auto"
                    || payload.language == "all"
                    || stream_lang
                        .to_lowercase()
                        .starts_with(&payload.language.to_lowercase())
                    || payload
                        .language
                        .to_lowercase()
                        .starts_with(&stream_lang.to_lowercase());
                if lang_match {
                    candidates.push(SubtitleCandidate {
                        source: "container".to_string(),
                        language: stream_lang.to_string(),
                        title: stream
                            .title
                            .clone()
                            .unwrap_or_else(|| format!("Embedded #{}", stream.subtitle_index)),
                        release: media_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string()),
                        details: Some(format!(
                            "Track index {}, codec {}",
                            stream.subtitle_index, stream.codec_name
                        )),
                        candidate_id: Some(stream.subtitle_index.to_string()),
                    });
                }
            }
        }
    }

    // 2. Inspect local subtitles directory
    if subtitles_dir.exists() {
        let clean_stem = media_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem_lower = clean_stem.to_lowercase();
        let req_lang = payload.language.to_lowercase();

        if let Ok(entries) = std::fs::read_dir(&subtitles_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("srt") {
                    continue;
                }
                let fname = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let has_stem = fname.contains(&stem_lower)
                    || stem_lower.contains(&fname.trim_end_matches(".srt").to_string())
                    || (stem_lower.len() > 10 && fname.contains(&stem_lower[..10]));

                if has_stem {
                    let lang_in_file =
                        if fname.contains(&req_lang) || req_lang == "auto" || req_lang == "all" {
                            payload.language.clone()
                        } else {
                            "unknown".to_string()
                        };
                    candidates.push(SubtitleCandidate {
                        source: "catalog".to_string(),
                        language: lang_in_file,
                        title: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        release: Some(clean_stem.clone()),
                        details: Some(path.to_string_lossy().to_string()),
                        candidate_id: Some(path.to_string_lossy().to_string()),
                    });
                }
            }
        }
    }

    // 3. Search remote provider
    let mut meta = MediaMetadata::parse_from_path(&media_path);
    if let Some(ref qt) = payload.query_title {
        if !qt.trim().is_empty() {
            meta.clean_title = qt.trim().to_string();
        }
    }
    if let Some(s) = payload.season {
        meta.season = Some(s);
    }
    if let Some(ep) = payload.episode {
        meta.episode = Some(ep);
    }

    if let Ok(remote_cands) = state.provider.search(&meta, &payload.language).await {
        for cand in remote_cands {
            candidates.push(SubtitleCandidate {
                source: "opensubtitles".to_string(),
                language: cand.language.clone(),
                title: cand
                    .release_name
                    .clone()
                    .unwrap_or_else(|| meta.clean_title.clone()),
                release: cand.release_name.clone(),
                details: Some(format!(
                    "Match score: {:.2}, ID: {}",
                    cand.match_score, cand.subtitle_id
                )),
                candidate_id: Some(cand.subtitle_id.clone()),
            });
        }
    }

    let total_candidates = candidates.len();
    Json(SearchSubtitlesResponse {
        media_path: payload.media_path,
        language: payload.language,
        total_candidates,
        candidates,
    })
}

pub async fn download_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DownloadSubtitleRequest>,
) -> Json<DownloadSubtitleResponse> {
    let media_path = PathBuf::from(&payload.media_path);
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    // 1. Fetch raw subtitle according to source
    let (mut srt_file, source_name) = match payload.source.as_str() {
        "container" => {
            let idx: usize = payload
                .candidate_id
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            match extract_embedded_subtitle(&media_path, idx) {
                Ok(raw) => match SubtitleFile::parse(&raw) {
                    Ok(s) => (s, "container".to_string()),
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
        "catalog" => {
            let path = payload
                .candidate_id
                .map(PathBuf::from)
                .unwrap_or_else(|| subtitles_dir.clone());
            match SubtitleFile::open(&path) {
                Ok(s) => (s, "catalog".to_string()),
                Err(e) => {
                    return Json(DownloadSubtitleResponse {
                        success: false,
                        output_srt_path: String::new(),
                        shift_frames: 0,
                        shift_ms: 0.0,
                        confidence_percent: 0.0,
                        source: payload.source,
                        language: payload.language,
                        error: Some(format!("Failed to open catalog subtitle: {:#}", e)),
                    });
                }
            }
        }
        "opensubtitles" => {
            let sub_id = payload.candidate_id.clone().unwrap_or_default();
            let meta = MediaMetadata::parse_from_path(&media_path);
            let mut raw_content = None;

            if let Some(cached) = state.cache.get(&meta, &payload.language, &sub_id) {
                raw_content = Some(cached);
            } else {
                let cand = dbsrt_core::RemoteSubtitleCandidate {
                    provider: "opensubtitles".to_string(),
                    subtitle_id: sub_id.clone(),
                    release_name: None,
                    language: payload.language.clone(),
                    download_url: String::new(),
                    hearing_impaired: false,
                    fps: None,
                    match_score: 1.0,
                };
                if let Ok(raw) = state.provider.download(&cand).await {
                    let _ = state.cache.put(&meta, &payload.language, &sub_id, &raw);
                    raw_content = Some(raw);
                }
            }

            match raw_content {
                Some(raw) => match SubtitleFile::parse(&raw) {
                    Ok(s) => (s, "opensubtitles".to_string()),
                    Err(e) => {
                        return Json(DownloadSubtitleResponse {
                            success: false,
                            output_srt_path: String::new(),
                            shift_frames: 0,
                            shift_ms: 0.0,
                            confidence_percent: 0.0,
                            source: payload.source,
                            language: payload.language,
                            error: Some(format!("Failed to parse remote subtitle: {:#}", e)),
                        });
                    }
                },
                None => {
                    return Json(DownloadSubtitleResponse {
                        success: false,
                        output_srt_path: String::new(),
                        shift_frames: 0,
                        shift_ms: 0.0,
                        confidence_percent: 0.0,
                        source: payload.source,
                        language: payload.language,
                        error: Some(format!("Failed to download subtitle ID '{}'", sub_id)),
                    });
                }
            }
        }
        other => {
            return Json(DownloadSubtitleResponse {
                success: false,
                output_srt_path: String::new(),
                shift_frames: 0,
                shift_ms: 0.0,
                confidence_percent: 0.0,
                source: other.to_string(),
                language: payload.language,
                error: Some(format!("Unknown subtitle source: {}", other)),
            });
        }
    };

    // 2. Perform resync if requested and possible
    let mut shift_frames = 0i64;
    let mut shift_ms = 0.0f64;
    let mut confidence_percent = 0.0f64;

    if payload.resync && media_path.exists() {
        if let Some((_, ref target_hashes, target_fps)) =
            find_existing_phash(&media_path, &phash_dir)
        {
            if let Ok(entries) = std::fs::read_dir(&phash_dir) {
                let target_cuts = detect_cuts(target_hashes, 16);
                let target_stem = media_path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("phash")
                        && !p.to_string_lossy().contains(&target_stem)
                    {
                        if let Ok(ref_hashes) = read_phash_file(&p) {
                            let ref_cuts = detect_cuts(&ref_hashes, 16);
                            let (s_frames, conf) =
                                calculate_shift_consensus(&target_cuts, &ref_cuts, 0, 0.0);
                            if conf > 50.0 {
                                shift_frames = s_frames;
                                shift_ms = (shift_frames as f64 / target_fps) * 1000.0;
                                confidence_percent = conf;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    if shift_ms.abs() > 0.001 {
        srt_file.shift(shift_ms);
    }

    // 3. Save subtitle file to disk
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

    if let Err(e) = srt_file.save(&out_path) {
        return Json(DownloadSubtitleResponse {
            success: false,
            output_srt_path: String::new(),
            shift_frames: 0,
            shift_ms: 0.0,
            confidence_percent: 0.0,
            source: source_name,
            language: payload.language,
            error: Some(format!(
                "Failed to write subtitle file to {:?}: {:#}",
                out_path, e
            )),
        });
    }

    Json(DownloadSubtitleResponse {
        success: true,
        output_srt_path: out_path.to_string_lossy().to_string(),
        shift_frames,
        shift_ms,
        confidence_percent,
        source: source_name,
        language: payload.language,
        error: None,
    })
}

pub async fn recognize_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RecognizeRequest>,
) -> Result<Json<RecognizeResponse>, (StatusCode, String)> {
    let media_path = PathBuf::from(&payload.media_path);
    if !media_path.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Media file not found: {:?}", media_path),
        ));
    }

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let start = Instant::now();
    let meta =
        probe_video(&media_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (payload.nb_seconds * meta.fps).round() as usize;

    let hashes = extract_frames_phash(&media_path, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let library = MmapLibrary::from_directory(&phash_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let matches = library.recognize(&hashes, payload.search_distance);
    let timer_seconds = start.elapsed().as_secs_f64();

    let media_name_search = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if let Some(top) = matches.first() {
        Ok(Json(RecognizeResponse {
            found: true,
            media_name_search,
            media_match: Some(top.media_name.clone()),
            nb_offsets_match: top.nb_offsets_match,
            confidence: top.confidence,
            timer_seconds,
        }))
    } else {
        Ok(Json(RecognizeResponse {
            found: false,
            media_name_search,
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            timer_seconds,
        }))
    }
}

pub fn find_existing_phash(
    media_file: &Path,
    phash_dir: &Path,
) -> Option<(PathBuf, Vec<u64>, f64)> {
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if phash_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(phash_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.contains(&stem) && name.ends_with(".phash") {
                        if let Ok(hashes) = read_phash_file(&p) {
                            let mut fps = if let Some(fps_match) = name.strip_suffix("fps.phash") {
                                let parts: Vec<&str> = fps_match.split('.').collect();
                                if parts.len() >= 2 {
                                    let last_two = format!(
                                        "{}.{}",
                                        parts[parts.len() - 2],
                                        parts[parts.len() - 1]
                                    );
                                    if let Ok(v) = last_two.parse::<f64>() {
                                        v
                                    } else {
                                        parts
                                            .last()
                                            .and_then(|p| p.parse::<f64>().ok())
                                            .unwrap_or(25.0)
                                    }
                                } else {
                                    parts
                                        .last()
                                        .and_then(|p| p.parse::<f64>().ok())
                                        .unwrap_or(25.0)
                                }
                            } else {
                                probe_video(media_file).map(|m| m.fps).unwrap_or(25.0)
                            };
                            if fps <= 0.0 || fps.is_nan() {
                                fps = 25.0;
                            }
                            return Some((p, hashes, fps));
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn resolve_or_extract_phash(
    media_file: &Path,
    phash_dir: &Path,
) -> Result<(PathBuf, Vec<u64>, f64)> {
    if let Some(existing) = find_existing_phash(media_file, phash_dir) {
        return Ok(existing);
    }

    let meta = probe_video(media_file)?;
    let total_frames = (meta.duration * meta.fps).floor() as usize;
    let hashes = extract_frames_phash(media_file, 0.0, total_frames)?;

    let out_name = format!(
        "{}.{:.1}fps.phash",
        media_file.file_name().unwrap().to_string_lossy(),
        meta.fps
    );
    let out_path = phash_dir.join(&out_name);
    write_phash_file(&out_path, &hashes)?;

    Ok((out_path, hashes, meta.fps))
}

pub async fn retarget_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RetargetRequest>,
) -> Json<RetargetResponse> {
    let target_media = PathBuf::from(&payload.target_media);
    let ref_subtitles = PathBuf::from(&payload.ref_subtitles);

    if !target_media.exists() {
        return Json(RetargetResponse {
            success: false,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: 0.0,
            error: Some(format!("Target media file not found: {:?}", target_media)),
        });
    }
    if !ref_subtitles.exists() {
        return Json(RetargetResponse {
            success: false,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: 0.0,
            error: Some(format!(
                "Subtitle file not found on disk: {:?}",
                ref_subtitles
            )),
        });
    }

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let start = Instant::now();

    // 1. Resolve target phash
    let (target_phash_file, target_hashes, target_fps) =
        match resolve_or_extract_phash(&target_media, &phash_dir) {
            Ok(res) => res,
            Err(e) => {
                return Json(RetargetResponse {
                    success: false,
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: 0.0,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    error: Some(format!("Failed to extract target media phash: {:#}", e)),
                });
            }
        };

    // 2. Resolve reference phash (either from ref_media or find in phash_dir)
    let (_ref_phash_file, ref_hashes, _ref_fps) = if let Some(ref ref_media_str) = payload.ref_media
    {
        let ref_media = PathBuf::from(ref_media_str);
        match resolve_or_extract_phash(&ref_media, &phash_dir) {
            Ok(res) => res,
            Err(e) => {
                return Json(RetargetResponse {
                    success: false,
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: 0.0,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    error: Some(format!("Failed to extract reference media phash: {:#}", e)),
                });
            }
        }
    } else {
        (target_phash_file.clone(), target_hashes.clone(), target_fps)
    };

    // 3. Detect cuts and calculate shift consensus
    let target_cuts = detect_cuts(&target_hashes, 16);
    let ref_cuts = detect_cuts(&ref_hashes, 16);

    let (shift_frames, confidence_percent) =
        calculate_shift_consensus(&target_cuts, &ref_cuts, payload.search_distance, 0.0);

    let shift_ms = (shift_frames as f64 / target_fps) * 1000.0;

    let mut srt = match SubtitleFile::open(&ref_subtitles) {
        Ok(s) => s,
        Err(e) => {
            return Json(RetargetResponse {
                success: false,
                shift_frames: 0,
                shift_ms: 0.0,
                output_srt_path: String::new(),
                confidence_percent: 0.0,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                error: Some(format!("Failed to parse subtitle file: {:#}", e)),
            });
        }
    };
    srt.shift(shift_ms);

    let out_name = format!(
        "{}.synced.srt",
        target_media.file_name().unwrap().to_string_lossy()
    );
    let out_path = phash_dir.join(&out_name);
    if let Err(e) = srt.save(&out_path) {
        return Json(RetargetResponse {
            success: false,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            error: Some(format!("Failed to save shifted subtitle file: {:#}", e)),
        });
    }

    Json(RetargetResponse {
        success: true,
        shift_frames,
        shift_ms,
        output_srt_path: out_path.to_string_lossy().to_string(),
        confidence_percent,
        elapsed_seconds: start.elapsed().as_secs_f64(),
        error: None,
    })
}

/// Calculates the optimal consensus frame shift and confidence percentage using
/// BK-Tree candidate voting, Lowe's dominance ratio, and cut consensus support.
pub fn calculate_shift_consensus(
    target_cuts: &[CutItem],
    ref_cuts: &[CutItem],
    search_distance: u32,
    fallback_confidence: f64,
) -> (i64, f64) {
    if ref_cuts.is_empty() || target_cuts.is_empty() {
        return (0, fallback_confidence);
    }

    let mut target_tree = BkTree::new();
    for cut in target_cuts {
        target_tree.insert(BkItem {
            phash: cut.phash,
            offset: cut.offset,
        });
    }

    let mut diff_counter: HashMap<i64, usize> = HashMap::new();
    for ref_cut in ref_cuts {
        let matches = target_tree.search(ref_cut.phash, search_distance);
        for m in matches {
            let diff = m.offset as i64 - ref_cut.offset as i64;
            *diff_counter.entry(diff).or_default() += 1;
        }
    }

    let mut sorted_diffs: Vec<(i64, usize)> = diff_counter.into_iter().collect();
    sorted_diffs.sort_by_key(|a| std::cmp::Reverse(a.1));

    if let Some(&(shift_frames, count)) = sorted_diffs.first() {
        let second_count = sorted_diffs.get(1).map(|(_, c)| *c).unwrap_or(0);
        let dominance = if count > 0 {
            1.0 - (second_count as f64 / count as f64)
        } else {
            0.0
        };
        let support = (count as f64 / ref_cuts.len() as f64).min(1.0);
        let confidence_percent = (dominance * support * 100.0).clamp(0.0, 100.0);
        (shift_frames, confidence_percent)
    } else {
        (0, fallback_confidence)
    }
}

pub fn find_subtitle_in_catalog(
    subtitles_dir: &Path,
    media_stem: &str,
    language: &str,
) -> Option<PathBuf> {
    if !subtitles_dir.exists() {
        return None;
    }
    let lang = language.to_lowercase();
    let entries = std::fs::read_dir(subtitles_dir).ok()?;
    let mut candidates: Vec<(i32, PathBuf)> = Vec::new();

    let clean_stem = media_stem
        .strip_suffix(".phash")
        .unwrap_or(media_stem)
        .strip_suffix(".mp4")
        .unwrap_or(media_stem)
        .strip_suffix(".webm")
        .unwrap_or(media_stem)
        .strip_suffix(".mkv")
        .unwrap_or(media_stem);
    let stem_lower = clean_stem.to_lowercase();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("srt") {
            continue;
        }
        let fname = path.file_name()?.to_string_lossy().to_lowercase();

        let has_stem = fname.contains(&stem_lower)
            || stem_lower.contains(&fname.trim_end_matches(".srt").to_string())
            || (stem_lower.len() > 10 && fname.contains(&stem_lower[..10]));
        if !has_stem {
            continue;
        }

        // Check language match
        let is_exact_lang = fname.ends_with(&format!(".{}.srt", lang))
            || fname.ends_with(&format!("_{}.srt", lang))
            || fname.contains(&format!(".{}.", lang));

        if is_exact_lang {
            return Some(path);
        }

        if lang == "auto" || lang == "all" || fname.contains(&lang) {
            candidates.push((10, path));
        } else {
            candidates.push((1, path));
        }
    }

    candidates.sort_by_key(|c| std::cmp::Reverse(c.0));
    candidates.first().map(|c| c.1.clone())
}

pub fn read_ref_phash_for_media(phash_dir: &Path, media_name: &str) -> Result<Vec<u64>> {
    let clean_name = media_name.strip_suffix(".phash").unwrap_or(media_name);
    let entries = std::fs::read_dir(phash_dir)?;
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if (name.contains(clean_name) || clean_name.contains(name.trim_end_matches(".phash")))
                && name.ends_with(".phash")
            {
                return read_phash_file(&p);
            }
        }
    }
    anyhow::bail!("Reference phash not found for media '{}'", media_name);
}

pub async fn autosync_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AutoSyncRequest>,
) -> Json<AutoSyncResponse> {
    let start = Instant::now();
    let media_path = PathBuf::from(&payload.media_path);
    if !media_path.exists() {
        return Json(AutoSyncResponse {
            success: false,
            matched_media: None,
            language: payload.language.clone(),
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            source: None,
            error: Some(format!("Target media file not found: {:?}", media_path)),
        });
    }

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());

    // 1. Try to find existing target phash or extract small sample for library recognition
    let existing_phash = find_existing_phash(&media_path, &phash_dir);
    let (target_fps, query_sample) = if let Some((_, ref hashes, fps)) = existing_phash {
        let sample_len = ((payload.nb_seconds * fps).round() as usize).min(hashes.len());
        let sample_start = if hashes.len() > sample_len * 2 {
            hashes.len() / 4
        } else {
            0
        };
        (
            fps,
            hashes[sample_start..sample_start + sample_len].to_vec(),
        )
    } else {
        let meta = probe_video(&media_path).unwrap_or(dbsrt_core::video::VideoMetadata {
            duration: 0.0,
            fps: 25.0,
        });
        let nb_frames = ((payload.nb_seconds * meta.fps).round() as usize).max(1);
        let start_sec = (meta.duration / 2.0).floor().max(0.0);
        let sample = extract_frames_phash(&media_path, start_sec, nb_frames).unwrap_or_default();
        (meta.fps, sample)
    };

    // 2. Identify media in index library
    let library = match MmapLibrary::from_directory(&phash_dir) {
        Ok(lib) => lib,
        Err(e) => {
            return Json(AutoSyncResponse {
                success: false,
                matched_media: None,
                language: payload.language.clone(),
                shift_frames: 0,
                shift_ms: 0.0,
                output_srt_path: String::new(),
                confidence_percent: 0.0,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                source: None,
                error: Some(format!("Failed to load mmap library: {:#}", e)),
            });
        }
    };

    let matches = if !query_sample.is_empty() {
        library.recognize(&query_sample, payload.search_distance)
    } else {
        Vec::new()
    };

    let (matched_stem, match_conf) = if let Some(m) = matches.first() {
        (m.media_name.clone(), m.confidence * 100.0)
    } else {
        let stem = media_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        (stem, 80.0)
    };

    // 3. Find reference subtitle: catalog on disk OR embedded subtitle stream in container OR remote provider
    let (mut srt, subtitle_source) = match find_subtitle_in_catalog(
        &subtitles_dir,
        &matched_stem,
        &payload.language,
    ) {
        Some(p) => match SubtitleFile::open(&p) {
            Ok(s) => (s, "catalog".to_string()),
            Err(e) => {
                return Json(AutoSyncResponse {
                    success: false,
                    matched_media: Some(matched_stem.clone()),
                    language: payload.language.clone(),
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: match_conf,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    source: Some("catalog".to_string()),
                    error: Some(format!("Failed to parse catalog subtitle {:?}: {:#}", p, e)),
                });
            }
        },
        None => {
            // Fallback: check if target media container has an embedded subtitle track matching requested language
            let embedded = inspect_container(&media_path)
                .ok()
                .and_then(|insp| insp.find_by_language(&payload.language).cloned());

            match embedded {
                Some(stream) => {
                    match extract_embedded_subtitle(&media_path, stream.subtitle_index) {
                        Ok(raw_srt) => match SubtitleFile::parse(&raw_srt) {
                            Ok(s) => (s, "container".to_string()),
                            Err(e) => {
                                return Json(AutoSyncResponse {
                                    success: false,
                                    matched_media: Some(matched_stem.clone()),
                                    language: payload.language.clone(),
                                    shift_frames: 0,
                                    shift_ms: 0.0,
                                    output_srt_path: String::new(),
                                    confidence_percent: match_conf,
                                    elapsed_seconds: start.elapsed().as_secs_f64(),
                                    source: Some("container".to_string()),
                                    error: Some(format!(
                                        "Failed to parse embedded subtitle track: {:#}",
                                        e
                                    )),
                                });
                            }
                        },
                        Err(e) => {
                            return Json(AutoSyncResponse {
                                success: false,
                                matched_media: Some(matched_stem.clone()),
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: Some("container".to_string()),
                                error: Some(format!(
                                    "Failed to extract embedded subtitle track: {:#}",
                                    e
                                )),
                            });
                        }
                    }
                }
                None => {
                    // Fallback Step 3: Remote Subtitle Provider Web Fallback (Axe 5)
                    let meta = MediaMetadata::parse_from_path(&media_path);
                    let candidates = match state.provider.search(&meta, &payload.language).await {
                        Ok(cands) => cands,
                        Err(e) => {
                            return Json(AutoSyncResponse {
                                success: false,
                                matched_media: Some(matched_stem.clone()),
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: Some("opensubtitles".to_string()),
                                error: Some(format!(
                                    "Remote provider search failed for '{}' ({}): {:#}",
                                    meta.clean_title, payload.language, e
                                )),
                            });
                        }
                    };

                    let mut downloaded_srt = None;
                    for cand in candidates.iter().take(3) {
                        // Check disk cache first
                        if let Some(cached_content) =
                            state.cache.get(&meta, &payload.language, &cand.subtitle_id)
                        {
                            if let Ok(parsed) = SubtitleFile::parse(&cached_content) {
                                downloaded_srt = Some(parsed);
                                break;
                            }
                        }

                        // Otherwise download from provider
                        if let Ok(raw_content) = state.provider.download(cand).await {
                            let _ = state.cache.put(
                                &meta,
                                &payload.language,
                                &cand.subtitle_id,
                                &raw_content,
                            );
                            if let Ok(parsed) = SubtitleFile::parse(&raw_content) {
                                downloaded_srt = Some(parsed);
                                break;
                            }
                        }
                    }

                    match downloaded_srt {
                        Some(s) => (s, "opensubtitles".to_string()),
                        None => {
                            return Json(AutoSyncResponse {
                                success: false,
                                matched_media: Some(matched_stem.clone()),
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: None,
                                error: Some(format!(
                                    "Media recognized as '{}' but no '{}' subtitle found in local catalog {:?}, container tracks, nor remote providers",
                                    matched_stem, payload.language, subtitles_dir
                                )),
                            });
                        }
                    }
                }
            }
        }
    };

    // 4. Calculate shift:
    let (shift_frames, confidence_percent) =
        match read_ref_phash_for_media(&phash_dir, &matched_stem) {
            Ok(ref_hashes) => {
                let target_hashes = if let Some((_, ref hashes, _)) = existing_phash {
                    hashes.clone()
                } else if let Ok(m) = probe_video(&media_path) {
                    let total_frames = (m.duration * m.fps).floor() as usize;
                    extract_frames_phash(&media_path, 0.0, total_frames).unwrap_or_default()
                } else {
                    Vec::new()
                };

                if !target_hashes.is_empty() {
                    let target_cuts = detect_cuts(&target_hashes, 16);
                    let ref_cuts = detect_cuts(&ref_hashes, 16);
                    calculate_shift_consensus(
                        &target_cuts,
                        &ref_cuts,
                        payload.search_distance,
                        match_conf,
                    )
                } else {
                    (0, match_conf)
                }
            }
            Err(_) => (0, match_conf),
        };

    let shift_ms = (shift_frames as f64 / target_fps) * 1000.0;

    // 5. Shift & Save Subtitle
    srt.shift(shift_ms);

    let out_name = format!(
        "{}.{}.synced.srt",
        media_path.file_name().unwrap().to_string_lossy(),
        payload.language
    );
    let out_path = phash_dir.join(&out_name);
    if let Err(e) = srt.save(&out_path) {
        return Json(AutoSyncResponse {
            success: false,
            matched_media: Some(matched_stem),
            language: payload.language,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            source: Some(subtitle_source),
            error: Some(format!("Failed to save synced subtitle: {:#}", e)),
        });
    }

    Json(AutoSyncResponse {
        success: true,
        matched_media: Some(matched_stem),
        language: payload.language,
        shift_frames,
        shift_ms,
        output_srt_path: out_path.to_string_lossy().to_string(),
        confidence_percent,
        elapsed_seconds: start.elapsed().as_secs_f64(),
        source: Some(subtitle_source),
        error: None,
    })
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

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let start = Instant::now();
    let meta = probe_video(&query_media)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (1.0 * meta.fps).round() as usize;

    let query_hashes = extract_frames_phash(&query_media, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let library = MmapLibrary::from_directory(&phash_dir)
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

    #[test]
    fn test_calculate_shift_consensus_empty() {
        let (shift, conf) = calculate_shift_consensus(&[], &[], 0, 42.0);
        assert_eq!(shift, 0);
        assert_eq!(conf, 42.0);
    }

    #[test]
    fn test_calculate_shift_consensus_perfect() {
        let cuts = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 25,
            },
        ];
        let (shift, conf) = calculate_shift_consensus(&cuts, &cuts, 0, 0.0);
        assert_eq!(shift, 0);
        assert!((conf - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_calculate_shift_consensus_with_shift() {
        let target = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 20,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 35,
            },
        ];
        let reference = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 25,
            },
        ];
        // target offset - ref offset = 10
        let (shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        assert_eq!(shift, 10);
        assert!((conf - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_calculate_shift_consensus_tie() {
        // Two conflicting candidates with 1 vote each
        let target = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 20,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 50,
            },
        ];
        let reference = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            }, // diff = +10
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 10,
            }, // diff = +40
        ];
        let (_shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        // Tie between diff 10 and diff 40: dominance is 1.0 - (1/1) = 0.0
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn test_calculate_shift_consensus_dominance() {
        let mut target = Vec::new();
        let mut reference = Vec::new();
        for i in 0..10 {
            let h = 0x1000_0000_0000_0000 + i;
            target.push(CutItem {
                phash: h,
                offset: 100 + i as usize,
            });
            reference.push(CutItem {
                phash: h,
                offset: 100 + i as usize,
            });
        }
        // Add 1 noise cut cross-matching
        let h_noise = 0x9999_9999_9999_9999;
        target.push(CutItem {
            phash: h_noise,
            offset: 200,
        });
        reference.push(CutItem {
            phash: h_noise,
            offset: 250,
        }); // diff = -50 (1 vote)

        let (shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        assert_eq!(shift, 0);
        // 11 votes total, 10 votes for 0, 1 vote for -50
        // Dominance: 1 - 1/10 = 0.90
        // Support: 10 / 11 = 0.90909
        // Conf: 0.90 * (10/11) * 100 = 81.818%
        assert!(conf > 80.0 && conf < 85.0);
    }

    #[tokio::test]
    async fn test_bind_listener_ephemeral() {
        let (listener, addr) = bind_listener("127.0.0.1", 0).await.unwrap();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert!(addr.port() > 0);
        drop(listener);
    }

    #[test]
    fn test_discovery_file_lifecycle() {
        let addr = "127.0.0.1:9999".parse().unwrap();
        write_discovery_file(addr).expect("write discovery file");
        assert!(std::path::Path::new(DISCOVERY_FILE).exists());
        let content = std::fs::read_to_string(DISCOVERY_FILE).unwrap();
        assert!(content.contains("9999"));
        assert!(content.contains("version"));
        remove_discovery_file();
        assert!(!std::path::Path::new(DISCOVERY_FILE).exists());
    }

    #[test]
    fn test_find_subtitle_in_catalog_fallback() {
        let empty_dir = std::env::temp_dir().join("empty_srt_dir_test_nonexistent");
        assert!(find_subtitle_in_catalog(&empty_dir, "media", "en").is_none());
    }
}
