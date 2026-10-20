//! Library core for dbsrt-server HTTP API.

use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use dbsrt_core::{
    build_and_validate_sub_frames, compute_bytes_digest_32, compute_media_phash_digest,
    detect_cuts, extract_embedded_subtitle, extract_frames_phash, inspect_container, probe_video,
    read_phash_file, write_phash_file, CertifiedMediaCoupling, CertifiedSubtitleRecord,
    CouplingRegistry, CutItem, MediaMetadata, OpenSubtitlesProvider, SubFrameAnchor, SubtitleCache,
    SubtitleFile, SubtitleProvider,
};
use dbsrt_index::{
    parse_fps_from_phash_stem, BkItem, BkTree, DbClient, DbSubtitleCoupling, MmapDbphash,
    MmapLibrary,
};
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

/// Persistent container for a reusable `DbClient`.
/// Implements `Drop` on an isolated OS thread so `postgres::Client::drop` never calls
/// Tokio's `block_on` inside an active async runtime worker thread (which would panic).
pub struct ClientPool {
    client: std::sync::Mutex<Option<DbClient>>,
}

impl Drop for ClientPool {
    fn drop(&mut self) {
        if let Ok(mut lock) = self.client.lock() {
            if let Some(client) = lock.take() {
                let _ = std::thread::spawn(move || {
                    drop(client);
                })
                .join();
            }
        }
    }
}

#[derive(Clone)]
pub struct DbConnectionConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub dbname: String,
    pub pool: Arc<ClientPool>,
}

impl std::fmt::Debug for DbConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("dbname", &self.dbname)
            .finish()
    }
}

impl DbConnectionConfig {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        pass: impl Into<String>,
        dbname: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            pass: pass.into(),
            dbname: dbname.into(),
            pool: Arc::new(ClientPool {
                client: std::sync::Mutex::new(None),
            }),
        }
    }

    pub fn connect(&self) -> Result<DbClient> {
        DbClient::connect(&self.host, self.port, &self.user, &self.pass, &self.dbname)
    }

    /// Executes a synchronous `DbClient` closure inside an isolated OS thread scope so
    /// `postgres::Client` never panics when called from inside an active Tokio runtime.
    /// Reuses persistent connection from `pool` to avoid 10-13ms TCP handshake + SCRAM auth latency.
    pub fn with_client<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut DbClient) -> Result<T> + Send,
        T: Send,
    {
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut client = {
                    let mut lock = self
                        .pool
                        .client
                        .lock()
                        .map_err(|e| anyhow::anyhow!("DB pool lock poisoned: {}", e))?;
                    if let Some(c) = lock.take() {
                        if !c.is_closed() {
                            c
                        } else {
                            self.connect()?
                        }
                    } else {
                        self.connect()?
                    }
                };

                let res = f(&mut client);

                if !client.is_closed() {
                    if let Ok(mut lock) = self.pool.client.lock() {
                        *lock = Some(client);
                    }
                }

                res
            })
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("PostgreSQL worker thread panicked")))
        })
    }
}

#[derive(Debug, Default)]
pub struct ServerMetrics {
    pub total_requests: std::sync::atomic::AtomicU64,
    pub recognize_requests: std::sync::atomic::AtomicU64,
    pub search_requests: std::sync::atomic::AtomicU64,
    pub cache_hits: std::sync::atomic::AtomicU64,
    pub remote_errors: std::sync::atomic::AtomicU64,
    pub total_latency_us: std::sync::atomic::AtomicU64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServerLogEntry {
    pub timestamp: String,
    pub tag: String,
    pub level: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServerLogsResponse {
    pub count: usize,
    pub logs: Vec<ServerLogEntry>,
}

#[derive(Clone)]
pub struct AppState {
    pub phash_dir: PathBuf,
    pub extra_phash_dirs: Vec<PathBuf>,
    pub dbphash_path: Option<PathBuf>,
    pub dbphash_reader: Option<Arc<MmapDbphash>>,
    pub db_config: Option<DbConnectionConfig>,
    pub subtitles_dir: PathBuf,
    pub coupling_registry: Arc<std::sync::RwLock<CouplingRegistry>>,
    pub provider: Arc<dyn SubtitleProvider>,
    pub cache: SubtitleCache,
    pub start_time: Instant,
    pub metrics: Arc<ServerMetrics>,
    pub remote_api: Option<String>,
    pub is_agent: bool,
    pub log_buffer: Arc<std::sync::RwLock<std::collections::VecDeque<ServerLogEntry>>>,
}

impl AppState {
    pub fn new(
        phash_dir: PathBuf,
        subtitles_dir: PathBuf,
        provider: Arc<dyn SubtitleProvider>,
        cache: SubtitleCache,
    ) -> Self {
        let mut extra_phash_dirs = Vec::new();
        let default_repo_phash = PathBuf::from("data/__PHASH__");
        if default_repo_phash != phash_dir && default_repo_phash.exists() {
            extra_phash_dirs.push(default_repo_phash);
        }

        let reg_path = CouplingRegistry::default_path(&subtitles_dir);
        let coupling_registry = Arc::new(std::sync::RwLock::new(
            CouplingRegistry::load_or_default(&reg_path).unwrap_or_default(),
        ));

        Self {
            phash_dir,
            extra_phash_dirs,
            dbphash_path: None,
            dbphash_reader: None,
            db_config: None,
            subtitles_dir,
            coupling_registry,
            provider,
            cache,
            start_time: Instant::now(),
            metrics: Arc::new(ServerMetrics::default()),
            remote_api: None,
            is_agent: false,
            log_buffer: Arc::new(std::sync::RwLock::new(
                std::collections::VecDeque::with_capacity(LOG_BUFFER_CAPACITY),
            )),
        }
    }

    pub fn push_log(&self, tag: &str, level: &str, message: impl Into<String>) {
        if let Ok(mut buf) = self.log_buffer.write() {
            if buf.len() >= LOG_BUFFER_CAPACITY {
                buf.pop_front();
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = now.as_secs() % SECONDS_PER_DAY;
            let h = secs / SECONDS_PER_HOUR;
            let m = (secs % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
            let s = secs % SECONDS_PER_MINUTE;
            let ms = now.subsec_millis();
            let ts = format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms);
            buf.push_back(ServerLogEntry {
                timestamp: ts,
                tag: tag.to_string(),
                level: level.to_string(),
                message: message.into(),
            });
        }
    }

    /// Reloads the in-memory `CouplingRegistry` from disk.
    pub fn reload_coupling_registry(&self) {
        let reg_path = CouplingRegistry::default_path(&self.subtitles_dir);
        if let Ok(reg) = CouplingRegistry::load_or_default(&reg_path) {
            if let Ok(mut lock) = self.coupling_registry.write() {
                *lock = reg;
            }
        }
    }

    /// Sets or updates the `.dbphash` catalog and pre-warms the MIH indices in RAM.
    pub fn set_dbphash(&mut self, path: Option<PathBuf>) {
        if let Some(ref p) = path {
            if let Ok(reader) = MmapDbphash::open(p) {
                // Pre-warm MIH OnceLock indices (zero query-time allocation)
                let _ = reader.sparse_mih();
                let _ = reader.cuts_mih();
                self.dbphash_reader = Some(Arc::new(reader));
            } else {
                self.dbphash_reader = None;
            }
        } else {
            self.dbphash_reader = None;
        }
        self.dbphash_path = path;
    }

    /// Returns all active `.phash` directories with `override_dir` (or `self.phash_dir`) first.
    pub fn effective_phash_dirs(&self, override_dir: Option<&Path>) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let primary = override_dir.unwrap_or(&self.phash_dir).to_path_buf();
        dirs.push(primary);
        if override_dir.is_none() {
            for d in &self.extra_phash_dirs {
                if !dirs.contains(d) {
                    dirs.push(d.clone());
                }
            }
        }
        dirs
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
    #[serde(default)]
    pub certified_couplings_count: usize,
    #[serde(default)]
    pub postgres_connected: bool,
}

/// Canonical subtitle source origin across the pipeline.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleSource {
    Certified,
    Container,
    Catalog,
    OpenSubtitles,
}

impl SubtitleSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Certified => "certified",
            Self::Container => "container",
            Self::Catalog => "catalog",
            Self::OpenSubtitles => "opensubtitles",
        }
    }
}

impl std::str::FromStr for SubtitleSource {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "certified" | "catalog_certified" | "certified_synced" => Ok(Self::Certified),
            "container" => Ok(Self::Container),
            "catalog" => Ok(Self::Catalog),
            "opensubtitles" => Ok(Self::OpenSubtitles),
            other => Err(other.to_string()),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct CertifySyncRequest {
    pub media_path: String,
    pub subtitle_path: String,
    #[serde(default = "default_lang")]
    pub language: String,
    #[serde(default = "default_true")]
    pub strict: bool,
    pub phash_dir: Option<String>,
    pub subtitles_dir: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CertifySyncResponse {
    pub success: bool,
    pub media_name: String,
    pub media_hash: String,
    pub phash_filename: String,
    pub subtitles_hash: String,
    pub certified_srt_path: String,
    pub cue_count: usize,
    pub sub_frames_count: usize,
    pub postgres_imported: bool,
    pub registry_path: String,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SearchSubtitlesRequest {
    pub media_path: String,
    #[serde(default = "default_lang")]
    pub language: String,
    pub subtitles_dir: Option<String>,
    pub phash_dir: Option<String>,
    pub search_mode: Option<String>,
    pub index_engine: Option<String>,
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
    #[serde(default)]
    pub shift_ms: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SearchTimingStats {
    pub phash_match_ms: f64,
    pub pg_lookup_ms: f64,
    pub json_lookup_ms: f64,
    pub container_ms: f64,
    pub disk_catalog_ms: f64,
    pub moviehash_ms: f64,
    pub overhead_ms: f64,
    pub total_ms: f64,
    pub pg_cached: bool,
    pub phash_frames_count: usize,
    pub video_duration_minutes: f64,
    pub query_sample_frames: usize,
    pub index_targets_count: usize,
    pub total_hamming_comparisons: usize,
    pub us_per_frame: f64,
    pub mfps_throughput: f64,
    pub matching_tier: String,
    pub confidence_percent: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SearchSubtitlesResponse {
    pub media_path: String,
    pub language: String,
    pub matched_media: Option<String>,
    pub moviehash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_preview: Option<Vec<String>>,
    pub total_candidates: usize,
    pub candidates: Vec<SubtitleCandidate>,
    #[serde(default)]
    pub timing: SearchTimingStats,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct DownloadSubtitleRequest {
    pub media_path: String,
    pub source: String,
    pub candidate_id: Option<String>,
    #[serde(default = "default_lang")]
    pub language: String,
    #[serde(default)]
    pub resync: bool,
    #[serde(default)]
    pub shift_ms: Option<f64>,
    pub subtitles_dir: Option<String>,
    pub phash_dir: Option<String>,
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

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RecognizeRequest {
    #[serde(default)]
    pub media_path: String,
    pub query_fingerprint: Option<Vec<String>>,
    pub fps: Option<f64>,
    pub language: Option<String>,
    pub client_version: Option<String>,
    #[serde(default)]
    pub search_distance: u32,
    #[serde(default = "default_extract_duration")]
    pub nb_seconds: f64,
    pub phash_dir: Option<String>,
}

impl Default for RecognizeRequest {
    fn default() -> Self {
        Self {
            media_path: String::new(),
            query_fingerprint: None,
            fps: None,
            language: None,
            client_version: None,
            search_distance: 0,
            nb_seconds: DEFAULT_EXTRACT_SAMPLE_SECONDS,
            phash_dir: None,
        }
    }
}

fn default_extract_duration() -> f64 {
    DEFAULT_EXTRACT_SAMPLE_SECONDS
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RecognizeResponse {
    pub found: bool,
    pub media_name_search: String,
    pub media_match: Option<String>,
    pub nb_offsets_match: usize,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift_frames: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<SubtitleCandidate>>,
    pub timer_seconds: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MetricsResponse {
    pub status: String,
    pub uptime_seconds: u64,
    pub total_requests: u64,
    pub recognize_requests: u64,
    pub search_requests: u64,
    pub cache_hits: u64,
    pub remote_errors: u64,
    pub avg_latency_ms: f64,
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
        .route("/studio", get(studio_ui_handler))
        .route("/ui", get(studio_ui_handler))
        .route("/health", get(health_handler))
        .route("/v1/health", get(health_handler))
        .route("/v1/status", get(status_handler))
        .route("/v1/metrics", get(metrics_handler))
        .route("/v1/logs", get(logs_handler))
        .route("/v1/couplings", get(couplings_handler))
        .route("/v1/couplings/sub_frames", get(sub_frames_handler))
        .route("/v1/search", post(search_handler))
        .route("/v1/download", post(download_handler))
        .route("/v1/certify", post(certify_handler))
        .route("/v1/recognize", post(recognize_handler))
        .route("/v1/retarget", post(retarget_handler))
        .route("/v1/autosync", post(autosync_handler))
        .route("/v1/scan", post(scan_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresTableCounts {
    pub medias_count: i64,
    pub frames_count: i64,
    pub subtitles_count: i64,
    pub sub_frames_count: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StudioPhashEntry {
    pub filename: String,
    pub media_name: String,
    pub fps: f64,
    pub frame_count: usize,
    pub size_bytes: u64,
    pub is_certified: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StudioMediaCoupling {
    pub media_hash: String,
    pub media_name: String,
    pub phash_filename: String,
    pub fps: f64,
    pub frame_count: usize,
    pub in_postgres: bool,
    pub subtitles: Vec<CertifiedSubtitleRecord>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CouplingsOverviewResponse {
    pub postgres_connected: bool,
    pub postgres_counts: Option<PostgresTableCounts>,
    pub local_registry_path: String,
    pub indexed_phash_files: usize,
    pub couplings: Vec<StudioMediaCoupling>,
    pub catalog_phashes: Vec<StudioPhashEntry>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub struct CouplingsQuery {
    /// When true, omits `sub_frames` vectors from `/v1/couplings` so overview payloads scale to 10,000+ medias.
    pub summary: Option<bool>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub struct SubFramesQuery {
    pub media_hash: String,
    pub subtitles_hash: Option<String>,
    pub q: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub cue_index: Option<i32>,
    pub target_frame: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SubFramesPageResponse {
    pub media_hash: String,
    pub media_name: String,
    pub fps: f64,
    pub frame_count: usize,
    pub subtitles_hash: String,
    pub subtitle_name: String,
    pub lang: String,
    pub srt_path: String,
    pub total_cues: usize,
    pub filtered_cues: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
    pub window_start_frame: i32,
    pub window_end_frame: i32,
    pub minimap_ticks: Vec<i32>,
    pub selected_cue: Option<SubFrameAnchor>,
    pub items: Vec<SubFrameAnchor>,
}

pub async fn studio_ui_handler() -> Html<&'static str> {
    Html(include_str!("studio.html"))
}

pub async fn couplings_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CouplingsQuery>,
) -> Json<CouplingsOverviewResponse> {
    let summary_only = query.summary.unwrap_or(false);
    let dirs = state.effective_phash_dirs(None);
    let reg_path = CouplingRegistry::default_path(&state.subtitles_dir);
    let reg = state
        .coupling_registry
        .read()
        .map(|r| r.clone())
        .unwrap_or_default();

    let mut postgres_connected = false;
    let mut postgres_counts = None;
    let mut pg_media_names = std::collections::HashSet::new();

    if let Some(ref cfg) = state.db_config {
        if let Ok((counts_opt, subs)) = cfg.with_client(|client| {
            let counts = client.get_table_counts().ok();
            let subs = client
                .get_subtitles_for_media(None, None, None)
                .unwrap_or_default();
            Ok((counts, subs))
        }) {
            postgres_connected = true;
            if let Some((m, f, s, sf)) = counts_opt {
                postgres_counts = Some(PostgresTableCounts {
                    medias_count: m,
                    frames_count: f,
                    subtitles_count: s,
                    sub_frames_count: sf,
                });
            }
            for sub in subs {
                pg_media_names.insert(sub.media_name);
            }
        }
    }

    let couplings: Vec<StudioMediaCoupling> = reg
        .couplings
        .into_iter()
        .map(
            |CertifiedMediaCoupling {
                 media_hash,
                 media_name,
                 phash_filename,
                 fps,
                 frame_count,
                 mut subtitles,
             }| {
                let in_postgres = pg_media_names.contains(&media_name)
                    || pg_media_names.contains(&phash_filename);
                if summary_only {
                    for s in &mut subtitles {
                        s.sub_frames.clear();
                    }
                }
                StudioMediaCoupling {
                    media_hash,
                    media_name,
                    phash_filename,
                    fps,
                    frame_count,
                    in_postgres,
                    subtitles,
                }
            },
        )
        .collect();

    let certified_names: std::collections::HashSet<String> = couplings
        .iter()
        .flat_map(|c| [c.media_name.clone(), c.phash_filename.clone()])
        .collect();

    let mut catalog_phashes = Vec::new();
    let mut seen_files = std::collections::HashSet::new();
    for dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("phash") {
                    let fname = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if !seen_files.insert(fname.clone()) {
                        continue;
                    }
                    let size_bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    let frame_count = (size_bytes / 8) as usize;
                    let media_name = fname.strip_suffix(".phash").unwrap_or(&fname).to_string();
                    let fps = parse_fps_from_phash_stem(&media_name);
                    let is_certified =
                        certified_names.contains(&media_name) || certified_names.contains(&fname);
                    catalog_phashes.push(StudioPhashEntry {
                        filename: fname,
                        media_name,
                        fps,
                        frame_count,
                        size_bytes,
                        is_certified,
                    });
                }
            }
        }
    }
    catalog_phashes.sort_by(|a, b| {
        b.is_certified
            .cmp(&a.is_certified)
            .then_with(|| a.media_name.cmp(&b.media_name))
    });

    let indexed_phash_files = catalog_phashes.len();

    Json(CouplingsOverviewResponse {
        postgres_connected,
        postgres_counts,
        local_registry_path: reg_path.to_string_lossy().to_string(),
        indexed_phash_files,
        couplings,
        catalog_phashes,
    })
}

pub async fn sub_frames_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SubFramesQuery>,
) -> Result<Json<SubFramesPageResponse>, (StatusCode, String)> {
    let reg = state
        .coupling_registry
        .read()
        .map(|r| r.clone())
        .unwrap_or_default();

    let media = reg
        .couplings
        .into_iter()
        .find(|c| c.media_hash == query.media_hash || c.media_name == query.media_hash)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Media '{}' not found", query.media_hash),
            )
        })?;

    let sub = if let Some(ref sh) = query.subtitles_hash {
        media
            .subtitles
            .into_iter()
            .find(|s| s.subtitles_hash == *sh || s.lang.eq_ignore_ascii_case(sh))
    } else {
        media.subtitles.into_iter().next()
    }
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "Subtitle track not found for media".to_string(),
        )
    })?;

    let total_cues = sub.sub_frames.len();
    let step = (total_cues / 180).max(1);
    let minimap_ticks: Vec<i32> = sub
        .sub_frames
        .iter()
        .step_by(step)
        .map(|sf| sf.start_frame_offset)
        .collect();

    let q_norm = query
        .q
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());

    let filtered: Vec<SubFrameAnchor> = match q_norm {
        Some(ref q) => sub
            .sub_frames
            .iter()
            .filter(|sf| {
                let hex_str = sf
                    .anchor_phash
                    .map(|h| format!("0x{:016x}", h))
                    .unwrap_or_default();
                format!("#{}", sf.index).contains(q)
                    || sf.start_frame_offset.to_string().contains(q)
                    || sf.end_frame_offset.to_string().contains(q)
                    || hex_str.contains(q)
                    || sf.text.to_lowercase().contains(q)
            })
            .cloned()
            .collect(),
        None => sub.sub_frames.clone(),
    };

    let filtered_cues = filtered.len();
    let page_size = query.page_size.unwrap_or(50).clamp(1, 500);
    let total_pages = filtered_cues.div_ceil(page_size).max(1);

    // Resolve target cue or target frame if requested
    let mut resolved_cue_index = query.cue_index;
    if let Some(tf) = query.target_frame {
        if let Some(closest) = filtered
            .iter()
            .min_by_key(|sf| (sf.start_frame_offset - tf).abs())
        {
            resolved_cue_index = Some(closest.index);
        }
    }

    let page = if let Some(ci) = resolved_cue_index {
        if let Some(pos) = filtered.iter().position(|sf| sf.index == ci) {
            (pos / page_size) + 1
        } else {
            query.page.unwrap_or(1).clamp(1, total_pages)
        }
    } else {
        query.page.unwrap_or(1).clamp(1, total_pages)
    };

    let start_idx = (page - 1) * page_size;
    let items: Vec<SubFrameAnchor> = filtered
        .iter()
        .skip(start_idx)
        .take(page_size)
        .cloned()
        .collect();

    let window_start_frame = items.first().map(|sf| sf.start_frame_offset).unwrap_or(0);
    let window_end_frame = items
        .last()
        .map(|sf| sf.end_frame_offset)
        .unwrap_or(media.frame_count as i32);

    let selected_cue = resolved_cue_index
        .and_then(|ci| filtered.iter().find(|sf| sf.index == ci).cloned())
        .or_else(|| items.first().cloned());

    Ok(Json(SubFramesPageResponse {
        media_hash: media.media_hash,
        media_name: media.media_name,
        fps: media.fps,
        frame_count: media.frame_count,
        subtitles_hash: sub.subtitles_hash,
        subtitle_name: sub.name,
        lang: sub.lang,
        srt_path: sub.srt_path,
        total_cues,
        filtered_cues,
        page,
        page_size,
        total_pages,
        window_start_frame,
        window_end_frame,
        minimap_ticks,
        selected_cue,
        items,
    }))
}

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

/// Normalizes a language code (ISO 639-1 / 639-2) to canonical 2-letter lowercase where known.
pub fn normalize_lang_code(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "fr" | "fre" | "fra" | "french" => "fr".to_string(),
        "en" | "eng" | "english" => "en".to_string(),
        "es" | "spa" | "spanish" => "es".to_string(),
        "de" | "ger" | "deu" | "german" => "de".to_string(),
        "it" | "ita" | "italian" => "it".to_string(),
        "pt" | "por" | "pob" | "portuguese" => "pt".to_string(),
        "ru" | "rus" | "russian" => "ru".to_string(),
        "ja" | "jpn" | "japanese" => "ja".to_string(),
        "zh" | "chi" | "zho" | "chinese" => "zh".to_string(),
        "nl" | "dut" | "nld" | "dutch" => "nl".to_string(),
        "pl" | "pol" | "polish" => "pl".to_string(),
        "tr" | "tur" | "turkish" => "tr".to_string(),
        other => other.to_string(),
    }
}

/// Extracts explicit language tag from a subtitle filename, supporting:
/// - Standard suffixes: `movie.fr.srt`, `movie.eng.srt`, `movie.french.srt`
/// - Synology / ffmpeg extracted tracks: `Episode.mkv_fre_9.srt`, `Episode.mkv_eng_3.srt`
/// - Shifted/synced suffixes: `Movie.french.shift.srt`, `Movie.fr.synced.srt`
pub fn detect_subtitle_file_language(filename: &str) -> Option<String> {
    let base = filename.strip_suffix(".srt").unwrap_or(filename);
    for token in base
        .rsplit(['.', '_', '-', ' ', '(', ')', '[', ']'])
        .filter(|t| !t.is_empty())
        .take(4)
    {
        let lower = token.to_lowercase();
        if lower.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if matches!(
            lower.as_str(),
            "shift" | "synced" | "sdh" | "forced" | "srt" | "sub" | "vtt" | "ass" | "ssa"
        ) {
            continue;
        }
        let mapped = match lower.as_str() {
            "fr" | "fre" | "fra" | "french" | "vf" | "vostfr" => Some("fr"),
            "en" | "eng" | "english" | "vo" => Some("en"),
            "es" | "spa" | "spanish" => Some("es"),
            "de" | "ger" | "deu" | "german" => Some("de"),
            "it" | "ita" | "italian" => Some("it"),
            "pt" | "por" | "pob" | "portuguese" => Some("pt"),
            "ru" | "rus" | "russian" => Some("ru"),
            "ja" | "jpn" | "japanese" => Some("ja"),
            "zh" | "chi" | "zho" | "chinese" => Some("zh"),
            "nl" | "dut" | "nld" | "dutch" => Some("nl"),
            "pl" | "pol" | "polish" => Some("pl"),
            "tr" | "tur" | "turkish" => Some("tr"),
            "ar" | "ara" | "arabic" => Some("ar"),
            "he" | "heb" | "hebrew" => Some("he"),
            "sv" | "swe" | "swedish" => Some("sv"),
            "da" | "dan" | "danish" => Some("da"),
            "fi" | "fin" | "finnish" => Some("fi"),
            "no" | "nor" | "nob" | "nno" | "norwegian" => Some("no"),
            "cs" | "cze" | "ces" | "czech" => Some("cs"),
            "hu" | "hun" | "hungarian" => Some("hu"),
            "ro" | "rum" | "ron" | "romanian" => Some("ro"),
            "el" | "ell" | "gre" | "greek" => Some("el"),
            "uk" | "ukr" | "ukrainian" => Some("uk"),
            "ko" | "kor" | "korean" => Some("ko"),
            "hin" | "hindi" => Some("hi"),
            _ => None,
        };
        if let Some(code) = mapped {
            return Some(code.to_string());
        }
    }
    None
}

/// Collects candidate subtitle directories: `primary_dir` plus (when `allow_sidecars` is true)
/// the media file's parent directory and nearby `SRT/` folders (e.g. `/media/latty/Synology/tvshow/SRT`).
pub fn collect_subtitle_search_dirs(
    primary_dir: &Path,
    media_path: &Path,
    allow_sidecars: bool,
) -> Vec<PathBuf> {
    let mut dirs = vec![primary_dir.to_path_buf()];
    if !allow_sidecars {
        return dirs;
    }
    let mut push_unique = |p: PathBuf| {
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };
    if let Some(parent) = media_path.parent() {
        if !parent.as_os_str().is_empty() && parent != Path::new("/tmp") {
            push_unique(parent.to_path_buf());
            push_unique(parent.join("SRT"));
            push_unique(parent.join("Subs"));
            if let Some(gp) = parent.parent() {
                push_unique(gp.join("SRT"));
                if let Some(ggp) = gp.parent() {
                    push_unique(ggp.join("SRT"));
                }
            }
        }
    }
    dirs
}

/// Unified catalog search returning `(score, PathBuf, detected_language)` sorted by relevance.
pub fn search_subtitles_in_catalog(
    subtitles_dir: &Path,
    media_stems: &[String],
    query_title: Option<&str>,
    season: Option<u32>,
    episode: Option<u32>,
    language: &str,
) -> Vec<(i32, PathBuf, String)> {
    if !subtitles_dir.exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(subtitles_dir) else {
        return Vec::new();
    };

    let req_lang = normalize_lang_code(language);
    let any_lang = req_lang == "auto" || req_lang == "all" || req_lang.is_empty();

    let clean_stems: Vec<String> = media_stems
        .iter()
        .map(|s| {
            s.strip_suffix(".phash")
                .unwrap_or(s)
                .strip_suffix(".mp4")
                .unwrap_or(s)
                .strip_suffix(".webm")
                .unwrap_or(s)
                .strip_suffix(".mkv")
                .unwrap_or(s)
                .to_lowercase()
        })
        .filter(|s| !s.is_empty())
        .collect();

    // Infer season/episode and normalized title from stems if not explicitly passed
    let stem_meta = clean_stems.first().map(|s| MediaMetadata::parse(s));
    let eff_season = season.or_else(|| stem_meta.as_ref().and_then(|m| m.season));
    let eff_episode = episode.or_else(|| stem_meta.as_ref().and_then(|m| m.episode));

    let norm_alnum = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };

    let query_norm: Option<String> = query_title
        .map(norm_alnum)
        .filter(|q| !q.is_empty())
        .or_else(|| {
            stem_meta
                .as_ref()
                .map(|m| norm_alnum(&m.clean_title))
                .filter(|q| q.len() >= 3)
        });

    let mut results: Vec<(i32, PathBuf, String)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("srt") {
            continue;
        }
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // Ignore empty/corrupt 0-byte .srt files
        if file_size > 0 && file_size < 10 {
            continue;
        }
        if file_size == 0 {
            continue;
        }

        let Some(fname_os) = path.file_name() else {
            continue;
        };
        let fname = fname_os.to_string_lossy().to_lowercase();
        let fname_alnum = norm_alnum(&fname);
        let cand_meta = MediaMetadata::parse(&fname);
        let cand_title_norm = norm_alnum(&cand_meta.clean_title);

        // Strict Season/Episode conflict gate: NEVER match S01E01 onto S01E02 or S02E03!
        if let (Some(req_s), Some(req_e)) = (eff_season, eff_episode) {
            if let (Some(cs), Some(ce)) = (cand_meta.season, cand_meta.episode) {
                if cs != req_s || ce != req_e {
                    continue;
                }
            }
        }

        let has_exact_stem = clean_stems.iter().any(|stem_lower| {
            fname.contains(stem_lower) || stem_lower.contains(fname.trim_end_matches(".srt"))
        });

        let has_query = query_norm.as_ref().is_some_and(|qn| {
            if !cand_title_norm.is_empty() && cand_title_norm.len() >= 3 {
                cand_title_norm == *qn || (qn.len() >= 5 && fname_alnum.starts_with(qn))
            } else {
                fname_alnum.contains(qn)
            }
        });

        if !has_exact_stem && !has_query {
            continue;
        }

        // If matched via title query (not exact release stem) and target has SxxExx,
        // the candidate subtitle MUST explicitly match that SxxExx
        if !has_exact_stem {
            if let (Some(s), Some(e)) = (eff_season, eff_episode) {
                let se_match = cand_meta.season == Some(s) && cand_meta.episode == Some(e);
                let se_tag = format!("s{:02}e{:02}", s, e);
                let x_tag = format!("{}x{:02}", s, e);
                if !se_match && !fname.contains(&se_tag) && !fname.contains(&x_tag) {
                    continue;
                }
            }
        }

        let detected_lang = detect_subtitle_file_language(&fname);
        let (mut score, resolved_lang) = match detected_lang {
            Some(ref d_lang) if *d_lang == req_lang => (100, d_lang.clone()),
            Some(ref d_lang) if any_lang => (50, d_lang.clone()),
            Some(_) => {
                // Explicit language suffix on file does NOT match requested language -> filter out!
                continue;
            }
            None => {
                // File has no explicit language tag in filename (e.g. `movie.srt`)
                let fallback_lang = if any_lang {
                    "und".to_string()
                } else {
                    req_lang.clone()
                };
                (10, fallback_lang)
            }
        };

        if has_exact_stem {
            score += 50;
        }
        // Prefer full dialogue subtitle files (>= 4 KB) over tiny forced/sign tracks (< 4 KB)
        if file_size >= 4096 {
            score += 15;
        }

        results.push((score, path, resolved_lang));
    }

    results.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    results
}

/// Shared helper to fetch a remote subtitle from disk cache or download it via provider.
pub async fn fetch_or_cache_remote_subtitle(
    state: &AppState,
    meta: &MediaMetadata,
    language: &str,
    candidate: &dbsrt_core::RemoteSubtitleCandidate,
) -> Result<SubtitleFile> {
    if let Some(cached) = state.cache.get(meta, language, &candidate.subtitle_id) {
        if let Ok(parsed) = SubtitleFile::parse(&cached) {
            return Ok(parsed);
        }
    }

    let raw = state.provider.download(candidate).await?;
    let _ = state
        .cache
        .put(meta, language, &candidate.subtitle_id, &raw);
    SubtitleFile::parse(&raw)
}

pub type ExistingPhashData = (PathBuf, Vec<u64>, f64);
pub type PhashIdentificationResult = (Option<(String, f64)>, f64, Option<ExistingPhashData>);

/// Default maximum Hamming distance (bits out of 64) used for visual pHash media identification.
pub const DEFAULT_RECOGNITION_SEARCH_DISTANCE: u32 = 6;

/// Minimum temporally coherent confidence percentage (`0.0 ..= 100.0`) required to accept a pHash media match.
pub const MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT: f64 =
    dbsrt_index::MIN_TEMPORAL_COHERENCE_CONFIDENCE * 100.0;

/// Default fallback frame rate when media container metadata cannot be probed.
pub const DEFAULT_FALLBACK_FPS: f64 = 23.976;

/// Default maximum number of log entries retained in the in-memory circular buffer.
pub const LOG_BUFFER_CAPACITY: usize = 300;

/// Default sample extraction duration in seconds for media fingerprinting.
pub const DEFAULT_EXTRACT_SAMPLE_SECONDS: f64 = 1.0;

/// Default network timeout in milliseconds for remote relay requests.
pub const DEFAULT_RELAY_TIMEOUT_MS: u64 = 10_000;

/// Number of perceptual hash samples included in JSON search response preview.
pub const FINGERPRINT_PREVIEW_SAMPLE_COUNT: usize = 5;

/// Number of seconds in a standard 24-hour day.
pub const SECONDS_PER_DAY: u64 = 86_400;

/// Number of seconds in an hour.
pub const SECONDS_PER_HOUR: u64 = 3600;

/// Number of seconds in a minute.
pub const SECONDS_PER_MINUTE: u64 = 60;

/// Total bit depth of a 64-bit DCT perceptual hash.
pub const PHASH_BIT_DEPTH: f64 = 64.0;

/// Scale factor to convert ratios (0.0 ..= 1.0) into percentages (0.0 ..= 100.0).
pub const PERCENT_SCALE: f64 = 100.0;

/// Default baseline FPS when target FPS is unspecified or invalid.
pub const DEFAULT_BASELINE_FPS: f64 = 24.0;

/// Default Hamming threshold between consecutive frames to detect a scene cut.
pub const DEFAULT_CUT_THRESHOLD: u32 = 16;

/// Default Hamming search distance for BK-tree and sequence cut voting.
pub const DEFAULT_SEARCH_DISTANCE: u32 = 12;

/// Maximum number of frames (~5 seconds at 24fps) used for continuous cross-correlation sequence alignment.
pub const MAX_CORRELATION_SAMPLE_FRAMES: usize = 120;

/// Minimum number of scene cuts in both target and reference to trigger cut-consensus voting.
pub const MIN_CUTS_FOR_CONSENSUS: usize = 2;

/// Minimum confidence percentage (0.0 ..= 100.0) from cut consensus required before falling back to sliding window.
pub const MIN_CUT_CONSENSUS_CONFIDENCE: f64 = 70.0;

/// Minimum visual shift in milliseconds to consider an offset significant enough to offer a synced candidate.
pub const SIGNIFICANT_SHIFT_THRESHOLD_MS: f64 = 500.0;

/// Minimum significant shift in milliseconds to apply subtitle timeline modification.
pub const MIN_SIGNIFICANT_SHIFT_MS: f64 = 0.001;

/// Milliseconds per second conversion factor.
pub const MILLIS_PER_SEC: f64 = 1000.0;

/// Refinement window radius in step units (+/- 2 seconds).
pub const REFINEMENT_STEP_RADIUS: usize = 2;

/// Identifies a target media file against `MmapLibrary` in `phash_dir` using existing `.phash`
/// or an on-the-fly frame sample. Returns `(matched_media_name_and_conf, target_fps, existing_phash)`.
pub fn identify_media_by_phash(
    media_path: &Path,
    phash_dir: &Path,
    nb_seconds: f64,
    search_distance: u32,
) -> PhashIdentificationResult {
    identify_media_by_phash_multi(
        media_path,
        &[phash_dir.to_path_buf()],
        None,
        None,
        nb_seconds,
        search_distance,
    )
}

/// Identifies a target media file across multiple `.phash` directories, an optional pre-warmed `.dbphash`
/// reader (or path), and an optional PostgreSQL BK-tree (`DbConnectionConfig`).
pub type RoutedPhashResult = (
    Option<(String, f64, String)>,
    f64,
    Option<ExistingPhashData>,
);

/// Extracts a query sample sequence (hashes + fps) from an existing .phash file or directly from video via FFmpeg.
pub fn extract_media_query_sample(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    nb_seconds: f64,
) -> (f64, Vec<u64>, Option<ExistingPhashData>) {
    let existing_phash = phash_dirs
        .iter()
        .find_map(|dir| find_existing_phash(media_path, dir));

    let (target_fps, query_sample) = if let Some((_, ref hashes, fps)) = existing_phash {
        let sample_len = ((nb_seconds * fps).round() as usize).min(hashes.len());
        let center_start = if hashes.len() > sample_len {
            (hashes.len() / 2).saturating_sub(sample_len / 2)
        } else {
            0
        };

        // 1. Test nominal window at t=50%
        let nominal_sample = &hashes[center_start..center_start + sample_len];
        let nominal_unique = nominal_sample
            .iter()
            .copied()
            .filter(|&h| !dbsrt_core::is_blank_phash(h))
            .collect::<std::collections::HashSet<_>>()
            .len();

        let chosen_start = if nominal_unique >= 2 || hashes.len() <= sample_len {
            center_start
        } else {
            // Adaptive sliding window: find the 1-second segment with highest visual entropy
            let step = (fps.round() as usize / 2).max(1); // 0.5s step
            let mut best_start = center_start;
            let mut best_unique = nominal_unique;

            let mut cur = 0;
            while cur + sample_len <= hashes.len() {
                let win = &hashes[cur..cur + sample_len];
                let uniq = win
                    .iter()
                    .copied()
                    .filter(|&h| !dbsrt_core::is_blank_phash(h))
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                if uniq > best_unique {
                    best_unique = uniq;
                    best_start = cur;
                    if best_unique >= 6 {
                        break; // Sufficient entropy found
                    }
                }
                cur += step;
            }
            best_start
        };

        (
            fps,
            hashes[chosen_start..chosen_start + sample_len].to_vec(),
        )
    } else if media_path.exists() {
        let meta = probe_video(media_path).unwrap_or(dbsrt_core::video::VideoMetadata {
            duration: 0.0,
            fps: 25.0,
        });
        let nb_frames = ((nb_seconds * meta.fps).round() as usize).max(1);
        let start_sec = (meta.duration / 2.0).floor().max(0.0);
        let mut sample = extract_frames_phash(media_path, start_sec, nb_frames).unwrap_or_default();
        let non_blank_unique = sample
            .iter()
            .copied()
            .filter(|&h| !dbsrt_core::is_blank_phash(h))
            .collect::<std::collections::HashSet<_>>()
            .len();

        if non_blank_unique < 2 && meta.duration > 6.0 {
            // Test alternate candidate timestamps across the clip
            for offset_ratio in [0.25, 0.75, 0.1, 0.9] {
                let alt_sec = (meta.duration * offset_ratio).floor().max(0.0);
                if let Ok(alt_sample) = extract_frames_phash(media_path, alt_sec, nb_frames) {
                    let alt_unique = alt_sample
                        .iter()
                        .copied()
                        .filter(|&h| !dbsrt_core::is_blank_phash(h))
                        .collect::<std::collections::HashSet<_>>()
                        .len();
                    if alt_unique > non_blank_unique {
                        sample = alt_sample;
                        if alt_unique >= 2 {
                            break;
                        }
                    }
                }
            }
        }
        (meta.fps, sample)
    } else {
        (25.0, Vec::new())
    };

    (target_fps, query_sample, existing_phash)
}

/// Normalizes a string by keeping only lowercase alphanumeric characters.
pub fn norm_alnum(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Identifies a target media file with explicit index engine routing.
pub fn identify_media_by_phash_multi_routed(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    dbphash_reader: Option<&MmapDbphash>,
    db_config: Option<&DbConnectionConfig>,
    nb_seconds: f64,
    search_distance: u32,
    engine: Option<&str>,
) -> RoutedPhashResult {
    let (target_fps, query_sample, existing_phash) =
        extract_media_query_sample(media_path, phash_dirs, nb_seconds);

    let non_blank_unique: std::collections::HashSet<u64> = query_sample
        .iter()
        .copied()
        .filter(|&h| !dbsrt_core::is_blank_phash(h))
        .collect();

    if non_blank_unique.is_empty() || (query_sample.len() >= 10 && non_blank_unique.len() < 2) {
        return (None, target_fps, existing_phash);
    }

    let rec_distance = search_distance.min(8);
    let chosen_engine = engine.unwrap_or("auto");

    // Mode 1: Force PostgreSQL 17 SP-GiST BK-Tree
    if chosen_engine == "pg_bktree" || chosen_engine == "postgres" {
        if let Some(cfg) = db_config {
            if let Ok(Some((rec, conf))) =
                cfg.with_client(|client| client.recognize_sample(&query_sample, rec_distance))
            {
                let conf_pct = conf * 100.0;
                if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    return (
                        Some((
                            rec.name,
                            conf_pct,
                            "Tier 3 (PostgreSQL 17 SP-GiST BK-Tree)".to_string(),
                        )),
                        target_fps,
                        existing_phash,
                    );
                }
            }
        }
        return (None, target_fps, existing_phash);
    }

    // Mode 2: Force Flat Linear Scan O(N)
    if chosen_engine == "linear" {
        if let Some(reader) = dbphash_reader {
            let matches = reader.recognize_linear(&query_sample, rec_distance);
            if let Some(m) = matches.into_iter().next() {
                if m.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    return (
                        Some((
                            m.clean_title,
                            m.confidence,
                            "Tier 1 (RAM .dbphash Flat Linear O(N))".to_string(),
                        )),
                        target_fps,
                        existing_phash,
                    );
                }
            }
        }
        return (None, target_fps, existing_phash);
    }

    // Mode 3: Force MIH (Tier 1 exclusive)
    if chosen_engine == "mih" {
        if let Some(reader) = dbphash_reader {
            let matches = reader.recognize(&query_sample, rec_distance);
            if let Some(m) = matches.into_iter().next() {
                if m.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    return (
                        Some((
                            m.clean_title,
                            m.confidence,
                            "Tier 1 (RAM .dbphash MIH under 10ms)".to_string(),
                        )),
                        target_fps,
                        existing_phash,
                    );
                }
            }
        }
        return (None, target_fps, existing_phash);
    }

    // Mode 4: Auto Cascade (Tier 1 MIH -> Tier 2 MmapLibrary -> Tier 3 PostgreSQL)
    if let Some(reader) = dbphash_reader {
        let matches = reader.recognize(&query_sample, rec_distance);
        if let Some(m) = matches.into_iter().next() {
            if m.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return (
                    Some((
                        m.clean_title,
                        m.confidence,
                        "Tier 1 (RAM .dbphash MIH under 10ms)".to_string(),
                    )),
                    target_fps,
                    existing_phash,
                );
            }
        }
    }

    // 2. Check MmapLibrary across all configured phash_dirs (Tier 2: Fast In-Memory Zero-DB, ~300ms)
    if let Ok(lib) = MmapLibrary::from_directories(phash_dirs) {
        if let Some(m) = lib
            .recognize(&query_sample, rec_distance)
            .into_iter()
            .next()
        {
            let conf_pct = m.confidence * 100.0;
            if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return (
                    Some((
                        m.media_name,
                        conf_pct,
                        "Tier 2 (MmapLibrary Zero-DB Files)".to_string(),
                    )),
                    target_fps,
                    existing_phash,
                );
            }
        }
    }

    // 3. Check PostgreSQL BK-Tree if configured (Tier 3: Persistent Store Fallback)
    if let Some(cfg) = db_config {
        if let Ok(Some((rec, conf))) =
            cfg.with_client(|client| client.recognize_sample(&query_sample, rec_distance))
        {
            let conf_pct = conf * 100.0;
            if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return (
                    Some((
                        rec.name,
                        conf_pct,
                        "Tier 3 (PostgreSQL 17 SP-GiST BK-Tree)".to_string(),
                    )),
                    target_fps,
                    existing_phash,
                );
            }
        }
    }

    (None, target_fps, existing_phash)
}

pub fn identify_media_by_phash_multi_with_reader(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    dbphash_reader: Option<&MmapDbphash>,
    db_config: Option<&DbConnectionConfig>,
    nb_seconds: f64,
    search_distance: u32,
) -> PhashIdentificationResult {
    let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_routed(
        media_path,
        phash_dirs,
        dbphash_reader,
        db_config,
        nb_seconds,
        search_distance,
        None,
    );
    let matched_pair = matched.map(|(name, conf, _tier)| (name, conf));
    (matched_pair, target_fps, existing_phash)
}

/// Identifies a target media file across multiple `.phash` directories, an optional `.dbphash`
/// catalog, and an optional PostgreSQL BK-tree (`DbConnectionConfig`).
pub fn identify_media_by_phash_multi(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    dbphash_path: Option<&Path>,
    db_config: Option<&DbConnectionConfig>,
    nb_seconds: f64,
    search_distance: u32,
) -> PhashIdentificationResult {
    let reader = dbphash_path.and_then(|p| MmapDbphash::open(p).ok());
    identify_media_by_phash_multi_with_reader(
        media_path,
        phash_dirs,
        reader.as_ref(),
        db_config,
        nb_seconds,
        search_distance,
    )
}

/// Computes visual pHash shift between `media_path` and a certified reference in `phash_dir`.
/// Strictly adheres to `GEMINI.md`: pHash recalage is ONLY active if the media is recognized
/// against a known reference `.phash` in `phash_dir`.
pub fn compute_media_phash_shift(
    media_path: &Path,
    phash_dir: &Path,
    matched_stem: &str,
    match_conf: f64,
    target_fps: f64,
    existing_phash: &Option<(PathBuf, Vec<u64>, f64)>,
    search_distance: u32,
) -> (i64, f64, f64) {
    compute_media_phash_shift_multi(
        media_path,
        &[phash_dir.to_path_buf()],
        matched_stem,
        match_conf,
        target_fps,
        existing_phash,
        search_distance,
    )
}

pub fn compute_media_phash_shift_multi(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    matched_stem: &str,
    match_conf: f64,
    target_fps: f64,
    existing_phash: &Option<(PathBuf, Vec<u64>, f64)>,
    search_distance: u32,
) -> (i64, f64, f64) {
    let ref_hashes_opt = phash_dirs
        .iter()
        .find_map(|dir| read_ref_phash_for_media(dir, matched_stem).ok());

    let (shift_frames, confidence_percent) = match ref_hashes_opt {
        Some(ref_hashes) => {
            let target_hashes = if let Some((_, ref hashes, _)) = existing_phash {
                hashes.clone()
            } else if let Ok(m) = probe_video(media_path) {
                let total_frames = (m.duration * m.fps).floor() as usize;
                extract_frames_phash(media_path, 0.0, total_frames).unwrap_or_default()
            } else {
                Vec::new()
            };

            if !target_hashes.is_empty() {
                let target_cuts = detect_cuts(&target_hashes, DEFAULT_CUT_THRESHOLD);
                let ref_cuts = detect_cuts(&ref_hashes, DEFAULT_CUT_THRESHOLD);
                let (sf_cut, conf_cut) = if target_cuts.len() >= MIN_CUTS_FOR_CONSENSUS
                    && ref_cuts.len() >= MIN_CUTS_FOR_CONSENSUS
                {
                    calculate_shift_consensus(&target_cuts, &ref_cuts, search_distance, match_conf)
                } else {
                    (0, 0.0)
                };

                if conf_cut >= MIN_CUT_CONSENSUS_CONFIDENCE {
                    (sf_cut, conf_cut)
                } else {
                    calculate_sequence_shift_argmin(&target_hashes, &ref_hashes, target_fps)
                }
            } else {
                (0, match_conf)
            }
        }
        None => (0, match_conf),
    };

    let fps = if target_fps > 0.0 {
        target_fps
    } else {
        DEFAULT_FALLBACK_FPS
    };
    let shift_ms = (shift_frames as f64 / fps) * MILLIS_PER_SEC;
    (shift_frames, shift_ms, confidence_percent)
}

/// Resolves certified subtitle couplings from PostgreSQL (`subtitles` JOIN `medias`)
/// and/or the local `couplings.json` registry (`CouplingRegistry`).
/// Returns (candidates, pg_lookup_ms, json_lookup_ms, pg_cached).
fn lookup_certified_couplings(
    state: &AppState,
    subtitles_dir: &Path,
    matched_media: &str,
    language: &str,
    phash_conf: f64,
) -> (Vec<SubtitleCandidate>, f64, f64, bool) {
    let mut certified_candidates = Vec::new();
    let req_lang = normalize_lang_code(language);
    let any_lang = req_lang == "auto" || req_lang == "all" || req_lang.is_empty();
    let mut seen_paths = std::collections::HashSet::new();

    // 1. Check in-memory cached `couplings.json` registry first (sub-millisecond RAM lookup)
    let json_start = std::time::Instant::now();
    let reg_fallback = if subtitles_dir != state.subtitles_dir {
        // Request passed an ad-hoc custom subtitles_dir (e.g. in test fixtures)
        let reg_path = CouplingRegistry::default_path(subtitles_dir);
        Some(CouplingRegistry::load_or_default(&reg_path).unwrap_or_default())
    } else {
        None
    };

    let reg_guard = if reg_fallback.is_none() {
        state.coupling_registry.read().ok()
    } else {
        None
    };

    let reg_ref: Option<&CouplingRegistry> = match (&reg_guard, &reg_fallback) {
        (Some(guard), _) => Some(&**guard),
        (_, Some(fallback)) => Some(fallback),
        _ => None,
    };

    if let Some(reg) = reg_ref {
        if let Some(coupling) = reg.find_by_matched_media(matched_media) {
            for sub in &coupling.subtitles {
                let sub_lang = normalize_lang_code(&sub.lang);
                if !any_lang && sub_lang != req_lang {
                    continue;
                }
                let full_srt_path = if Path::new(&sub.srt_path).is_absolute()
                    || Path::new(&sub.srt_path).exists()
                {
                    PathBuf::from(&sub.srt_path)
                } else {
                    subtitles_dir.join(&sub.srt_path)
                };
                let full_str = full_srt_path.to_string_lossy().to_string();
                if seen_paths.insert(full_str.clone()) {
                    certified_candidates.push(SubtitleCandidate {
                        source: SubtitleSource::Certified.as_str().to_string(),
                        language: sub_lang,
                        title: sub.name.clone(),
                        release: Some(coupling.phash_filename.clone()),
                        details: Some(format!(
                            "Certified Coupling ({} cues, SHA256: {}…) for '{}' ({:.0}% conf)",
                            sub.cue_count,
                            &sub.subtitles_hash[..8.min(sub.subtitles_hash.len())],
                            coupling.phash_filename,
                            phash_conf
                        )),
                        candidate_id: Some(full_str),
                        shift_ms: None,
                    });
                }
            }
        }
    }
    let json_lookup_ms = json_start.elapsed().as_secs_f64() * 1000.0;

    // 2. Fallback to PostgreSQL relational `subtitles` table ONLY if cache didn't find candidates
    let mut pg_lookup_ms = 0.0;
    let mut pg_cached = false;
    if !certified_candidates.is_empty() {
        pg_cached = true;
    } else if let Some(ref cfg) = state.db_config {
        let pg_start = std::time::Instant::now();
        let lang_opt = if any_lang {
            None
        } else {
            Some(req_lang.as_str())
        };
        if let Ok(db_subs) = cfg.with_client(|client| {
            client.get_subtitles_for_media(None, Some(matched_media), lang_opt)
        }) {
            for DbSubtitleCoupling {
                id,
                name,
                lang,
                media_name,
                srt_path,
                ..
            } in db_subs
            {
                let resolved_path = srt_path.clone().unwrap_or_else(|| {
                    subtitles_dir
                        .join(format!("{}.srt", name))
                        .to_string_lossy()
                        .to_string()
                });
                if seen_paths.insert(resolved_path.clone()) {
                    certified_candidates.push(SubtitleCandidate {
                        source: SubtitleSource::Certified.as_str().to_string(),
                        language: lang,
                        title: format!("{}.srt", name.trim_end_matches(".srt")),
                        release: Some(media_name.clone()),
                        details: Some(format!(
                            "PostgreSQL Certified Coupling #{} for '{}' ({:.0}% conf)",
                            id, media_name, phash_conf
                        )),
                        candidate_id: Some(resolved_path),
                        shift_ms: None,
                    });
                }
            }
        }
        pg_lookup_ms = pg_start.elapsed().as_secs_f64() * 1000.0;
    }

    (
        certified_candidates,
        pg_lookup_ms,
        json_lookup_ms,
        pg_cached,
    )
}

pub async fn search_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SearchSubtitlesRequest>,
) -> impl IntoResponse {
    let start_overall = std::time::Instant::now();
    state
        .metrics
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state
        .metrics
        .search_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let media_path = PathBuf::from(&payload.media_path);
    let include_sidecars = payload.subtitles_dir.is_none();
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());
    let mut candidates: Vec<SubtitleCandidate> = Vec::new();

    let mode = payload.search_mode.as_deref().unwrap_or("all");
    let use_phash = mode == "phash" || mode == "all" || mode == "auto";

    state.push_log(
        "SEARCH",
        "INFO",
        format!(
            "Request: mode='{}', lang='{}', media='{}'",
            mode,
            payload.language,
            media_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
        ),
    );

    let mut timing = SearchTimingStats::default();

    // 0. Visual pHash Recognition when mode allows pHash identification
    let mut matched_media: Option<String> = None;
    let mut phash_conf: f64 = 0.0;
    let mut target_fps: f64 = DEFAULT_FALLBACK_FPS;
    let mut existing_phash: Option<ExistingPhashData> = None;
    let mut query_sample_opt: Option<Vec<u64>> = None;

    if use_phash && media_path.exists() {
        if let Some(ref remote_url) = state.remote_api {
            let t0 = std::time::Instant::now();
            let (tfps, query_sample, e_phash) = extract_media_query_sample(
                &media_path,
                &phash_dirs,
                DEFAULT_EXTRACT_SAMPLE_SECONDS,
            );
            target_fps = tfps;
            existing_phash = e_phash;
            query_sample_opt = Some(query_sample.clone());

            let extract_ms = t0.elapsed().as_secs_f64() * 1000.0;
            state.push_log(
                "RELAY",
                "INFO",
                format!(
                    "Extracted {} pHash frames from {:?} in {:.2}ms (fps={:.2})",
                    query_sample.len(),
                    media_path.file_name().unwrap_or_default(),
                    extract_ms,
                    target_fps
                ),
            );
            tracing::info!(
                "[Relay] Extracted {} pHash frames from {:?} in {:.2}ms (fps={:.2})",
                query_sample.len(),
                media_path.file_name().unwrap_or_default(),
                extract_ms,
                target_fps
            );

            if !query_sample.is_empty() {
                let fp_strings: Vec<String> = query_sample
                    .iter()
                    .map(|h| format!("{:#018x}", h))
                    .collect();
                let remote_req = RecognizeRequest {
                    media_path: String::new(),
                    query_fingerprint: Some(fp_strings),
                    fps: Some(target_fps),
                    language: Some(payload.language.clone()),
                    client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    search_distance: 0,
                    nb_seconds: DEFAULT_EXTRACT_SAMPLE_SECONDS,
                    phash_dir: None,
                };

                let http_client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_millis(DEFAULT_RELAY_TIMEOUT_MS))
                    .build()
                    .unwrap_or_default();

                let endpoint = format!("{}/v1/recognize", remote_url.trim_end_matches('/'));
                state.push_log(
                    "RELAY",
                    "INFO",
                    format!(
                        "Transmitting 200B fingerprint ({} hashes) to {}",
                        query_sample.len(),
                        endpoint
                    ),
                );
                tracing::info!(
                    "[Relay] Transmitting 200-byte fingerprint ({} hashes) to remote API: {}",
                    query_sample.len(),
                    endpoint
                );

                let t_net = std::time::Instant::now();
                match http_client.post(&endpoint).json(&remote_req).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let net_rtt_ms = t_net.elapsed().as_secs_f64() * 1000.0;
                        if let Ok(rec_res) = resp.json::<RecognizeResponse>().await {
                            if rec_res.found {
                                matched_media = rec_res.media_match.clone();
                                phash_conf = rec_res.confidence;
                                timing.matching_tier = format!(
                                    "Remote Relay -> {}",
                                    rec_res.tier.as_deref().unwrap_or("o2switch")
                                );
                                timing.confidence_percent = phash_conf;
                                timing.phash_match_ms = t0.elapsed().as_secs_f64() * 1000.0;
                                timing.query_sample_frames = query_sample.len();

                                let cands_count =
                                    rec_res.candidates.as_ref().map(|c| c.len()).unwrap_or(0);
                                state.push_log(
                                    "RELAY",
                                    "INFO",
                                    format!(
                                        "✓ Remote recognition in {:.2}ms (RTT={:.2}ms): matched='{}' (conf={:.1}%, tier='{}', candidates={})",
                                        rec_res.timer_seconds * 1000.0,
                                        net_rtt_ms,
                                        matched_media.as_deref().unwrap_or("unknown"),
                                        phash_conf,
                                        timing.matching_tier,
                                        cands_count
                                    ),
                                );
                                tracing::info!(
                                    "[Relay] ✓ Remote recognition in {:.2}ms (RTT={:.2}ms): matched='{}' (conf={:.1}%, tier='{}', candidates={})",
                                    rec_res.timer_seconds * 1000.0,
                                    net_rtt_ms,
                                    matched_media.as_deref().unwrap_or("unknown"),
                                    phash_conf,
                                    timing.matching_tier,
                                    cands_count
                                );

                                if let Some(remote_cands) = rec_res.candidates {
                                    for c in remote_cands {
                                        candidates.push(c);
                                    }
                                }
                            } else {
                                state.push_log(
                                    "RELAY",
                                    "INFO",
                                    format!(
                                        "Remote server replied in {:.2}ms: media not recognized (found=false)",
                                        net_rtt_ms
                                    ),
                                );
                                tracing::info!(
                                    "[Relay] Remote server replied in {:.2}ms: media not recognized (found=false)",
                                    net_rtt_ms
                                );
                            }
                        }
                    }
                    Err(e) => {
                        state
                            .metrics
                            .remote_errors
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        state.push_log(
                            "RELAY",
                            "WARN",
                            format!("✗ Remote relay error to {}: {:#}", endpoint, e),
                        );
                        tracing::warn!("[Relay] ✗ Remote relay error to {}: {:#}", endpoint, e);
                    }
                    _ => {
                        state
                            .metrics
                            .remote_errors
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        state.push_log(
                            "RELAY",
                            "WARN",
                            "✗ Remote relay returned unexpected non-200 status code",
                        );
                        tracing::warn!(
                            "[Relay] ✗ Remote relay returned unexpected non-200 status code"
                        );
                    }
                }
            }
        } else if phash_dirs.iter().any(|d| d.exists())
            || state.dbphash_path.is_some()
            || state.db_config.is_some()
        {
            let t0 = std::time::Instant::now();
            let (matched_routed, tfps, e_phash) = identify_media_by_phash_multi_routed(
                &media_path,
                &phash_dirs,
                state.dbphash_reader.as_deref(),
                state.db_config.as_ref(),
                1.0,
                DEFAULT_RECOGNITION_SEARCH_DISTANCE,
                payload.index_engine.as_deref(),
            );
            target_fps = tfps;
            existing_phash = e_phash;
            timing.phash_match_ms = t0.elapsed().as_secs_f64() * 1000.0;

            if let Some((_, ref hashes, fps)) = existing_phash {
                timing.phash_frames_count = hashes.len();
                if fps > 0.0 {
                    timing.video_duration_minutes =
                        (hashes.len() as f64 / fps) / SECONDS_PER_MINUTE as f64;
                    timing.query_sample_frames =
                        ((DEFAULT_EXTRACT_SAMPLE_SECONDS * fps).round() as usize).min(hashes.len());
                }
            } else if media_path.exists() {
                if let Ok(meta) = probe_video(&media_path) {
                    timing.phash_frames_count = (meta.duration * meta.fps).round() as usize;
                    timing.video_duration_minutes = meta.duration / SECONDS_PER_MINUTE as f64;
                    timing.query_sample_frames =
                        ((DEFAULT_EXTRACT_SAMPLE_SECONDS * meta.fps).round() as usize).max(1);
                }
            }

            if timing.query_sample_frames == 0 {
                timing.query_sample_frames = DEFAULT_BASELINE_FPS.round() as usize;
            }

            // Calculate index targets count and atomic comparisons
            if let Some(ref reader) = state.dbphash_reader {
                let targets = reader.cuts_entries().len() + reader.sparse_entries().len();
                timing.index_targets_count = targets;
                timing.total_hamming_comparisons = timing.query_sample_frames * targets;
            } else if let Some(ref dbphash) = state.dbphash_path {
                if let Ok(reader) = MmapDbphash::open(dbphash) {
                    let targets = reader.cuts_entries().len() + reader.sparse_entries().len();
                    timing.index_targets_count = targets;
                    timing.total_hamming_comparisons = timing.query_sample_frames * targets;
                }
            }

            if timing.phash_match_ms > 0.0 && timing.total_hamming_comparisons > 0 {
                // Microseconds per atomic frame comparison
                timing.us_per_frame =
                    (timing.phash_match_ms * 1000.0) / (timing.total_hamming_comparisons as f64);
                // Millions of frame comparisons per second (MFPS)
                timing.mfps_throughput = (timing.total_hamming_comparisons as f64 / 1_000_000.0)
                    / (timing.phash_match_ms / 1000.0);
            }

            if let Some((m_name, conf, tier_label)) = matched_routed {
                matched_media = Some(m_name);
                phash_conf = conf;
                timing.confidence_percent = conf;
                timing.matching_tier = tier_label;
            }
        }
    }

    let t_meta = std::time::Instant::now();
    let is_phash_file = media_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("phash"))
        .unwrap_or(false);

    let (mut meta, moviehash) = if is_phash_file {
        // Fast path: skip disk moviehash I/O on .phash files (not a video container)
        let meta = MediaMetadata::parse(
            media_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(""),
        );
        (meta, None)
    } else {
        let meta = MediaMetadata::parse_from_path(&media_path);
        let mh = meta.moviehash.clone();
        (meta, mh)
    };
    timing.moviehash_ms = t_meta.elapsed().as_secs_f64() * 1000.0;

    let fingerprint_preview = query_sample_opt
        .as_deref()
        .or_else(|| {
            existing_phash
                .as_ref()
                .map(|(_, hashes, _)| hashes.as_slice())
        })
        .map(|hashes| {
            hashes
                .iter()
                .take(FINGERPRINT_PREVIEW_SAMPLE_COUNT)
                .map(|h| format!("{:#018x}", h))
                .collect::<Vec<_>>()
        });

    // Strict pHash mode: if the user explicitly requested "Search by pHash" and the video
    // does not match any reference .phash in PostgreSQL / MmapLibrary, return 0 candidates immediately.
    if mode == "phash" && matched_media.is_none() {
        timing.total_ms = start_overall.elapsed().as_secs_f64() * 1000.0;
        let accounted_ms = timing.phash_match_ms
            + timing.pg_lookup_ms
            + timing.json_lookup_ms
            + timing.container_ms
            + timing.disk_catalog_ms
            + timing.moviehash_ms;
        timing.overhead_ms = (timing.total_ms - accounted_ms).max(0.0);
        return Json(SearchSubtitlesResponse {
            media_path: payload.media_path,
            language: payload.language,
            matched_media: None,
            moviehash,
            fingerprint_preview,
            total_candidates: 0,
            candidates,
            timing,
        });
    }

    // 0b. Certified Couplings (PostgreSQL `subtitles` JOIN `medias` + `couplings.json`)
    let mut seen_catalog_paths = std::collections::HashSet::new();
    let mut seen_filenames = std::collections::HashSet::new();
    if let Some(ref m_name) = matched_media {
        if candidates.is_empty() && !state.is_agent {
            let (cert_cands, pg_ms, json_ms, pg_cached) = lookup_certified_couplings(
                &state,
                &subtitles_dir,
                m_name,
                &payload.language,
                phash_conf,
            );
            timing.pg_lookup_ms = pg_ms;
            timing.json_lookup_ms = json_ms;
            timing.pg_cached = pg_cached;

            for mut c in cert_cands {
                if let Some(ref cid) = c.candidate_id {
                    seen_catalog_paths.insert(cid.clone());
                    if let Ok(canon) = Path::new(cid).canonicalize() {
                        seen_catalog_paths.insert(canon.to_string_lossy().to_string());
                    }
                }
                seen_filenames.insert(c.title.to_lowercase());

                let ref_stem = c.release.as_deref().unwrap_or(m_name);
                let (_sf, computed_shift_ms, _sconf) = compute_media_phash_shift_multi(
                    &media_path,
                    &phash_dirs,
                    ref_stem,
                    phash_conf,
                    target_fps,
                    &existing_phash,
                    DEFAULT_SEARCH_DISTANCE,
                );

                // 1. Raw candidate (original timeline, zero shift)
                c.shift_ms = Some(0.0);
                candidates.push(c.clone());

                // 2. Synced candidate if visual offset is significant (> 500ms)
                if computed_shift_ms.abs() > SIGNIFICANT_SHIFT_THRESHOLD_MS {
                    let mut c_synced = c.clone();
                    c_synced.source = "certified_synced".to_string();
                    c_synced.shift_ms = Some(computed_shift_ms);
                    c_synced.title = format!(
                        "{} [Synced {:+.1}s]",
                        c.title,
                        computed_shift_ms / MILLIS_PER_SEC
                    );
                    if let Some(ref d) = c.details {
                        c_synced.details = Some(format!(
                            "{} (Synchronized: {:+.2}s visual shift)",
                            d,
                            computed_shift_ms / MILLIS_PER_SEC
                        ));
                    }
                    candidates.push(c_synced);
                }
            }
        } else {
            // Already populated via remote relay
            for c in &candidates {
                if let Some(ref cid) = c.candidate_id {
                    seen_catalog_paths.insert(cid.clone());
                }
                seen_filenames.insert(c.title.to_lowercase());
            }
        }
    }

    // 1. Inspect container if media exists (skip when mode == "phash" so only pHash-backed results appear)
    if mode != "phash" && media_path.exists() {
        let t_cont = std::time::Instant::now();
        if let Ok(insp) = inspect_container(&media_path) {
            let req_norm = normalize_lang_code(&payload.language);
            for stream in insp.subtitle_streams {
                let stream_lang = stream.language.as_deref().unwrap_or("und");
                let stream_norm = normalize_lang_code(stream_lang);
                let lang_match = payload.language == "auto"
                    || payload.language == "all"
                    || stream_norm == req_norm
                    || stream_lang
                        .to_lowercase()
                        .starts_with(&payload.language.to_lowercase())
                    || payload
                        .language
                        .to_lowercase()
                        .starts_with(&stream_lang.to_lowercase());
                if lang_match {
                    candidates.push(SubtitleCandidate {
                        source: SubtitleSource::Container.as_str().to_string(),
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
                        shift_ms: None,
                    });
                }
            }
        }
        timing.container_ms = t_cont.elapsed().as_secs_f64() * 1000.0;
    }

    // 2. Inspect local subtitles directory (matching pHash matched media, or file stem / query_title when mode != "phash")
    let t_disk = std::time::Instant::now();
    let file_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut stems = Vec::new();
    if !file_stem.is_empty() {
        stems.push(file_stem.clone());
    }
    if let Some(ref m) = matched_media {
        if !stems.contains(m) {
            stems.push(m.clone());
        }
    }

    let catalog_query_title = if mode == "phash" {
        None
    } else {
        payload.query_title.as_deref()
    };

    let effective_season = payload.season.or(meta.season);
    let effective_episode = payload.episode.or(meta.episode);

    let search_dirs = collect_subtitle_search_dirs(&subtitles_dir, &media_path, include_sidecars);

    for dir in &search_dirs {
        for (_, path, resolved_lang) in search_subtitles_in_catalog(
            dir,
            &stems,
            catalog_query_title,
            effective_season,
            effective_episode,
            &payload.language,
        ) {
            let path_str = path.to_string_lossy().to_string();
            let canon_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let canon_str = canon_path.to_string_lossy().to_string();
            if !seen_catalog_paths.insert(path_str.clone()) || !seen_catalog_paths.insert(canon_str)
            {
                continue;
            }

            let cand_fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !seen_filenames.insert(cand_fname.to_lowercase()) {
                continue;
            }

            let details = if let Some(ref m) = matched_media {
                format!(
                    "pHash matched '{}' ({:.0}% conf) — {}",
                    m,
                    phash_conf,
                    path.display()
                )
            } else {
                path_str.clone()
            };
            candidates.push(SubtitleCandidate {
                source: SubtitleSource::Catalog.as_str().to_string(),
                language: resolved_lang,
                title: cand_fname,
                release: matched_media.clone().or_else(|| Some(file_stem.clone())),
                details: Some(details),
                candidate_id: Some(path_str),
                shift_ms: None,
            });
        }
    }
    timing.disk_catalog_ms = t_disk.elapsed().as_secs_f64() * 1000.0;

    // 3. Search remote provider (skip in strict "phash" mode so uncoupled remote subs are not mixed in)
    if mode != "phash" {
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
                    source: SubtitleSource::OpenSubtitles.as_str().to_string(),
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
                    shift_ms: None,
                });
            }
        }
    }

    let total_candidates = candidates.len();
    timing.total_ms = start_overall.elapsed().as_secs_f64() * 1000.0;
    state.metrics.total_latency_us.fetch_add(
        start_overall.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    let accounted_ms = timing.phash_match_ms
        + timing.pg_lookup_ms
        + timing.json_lookup_ms
        + timing.container_ms
        + timing.disk_catalog_ms
        + timing.moviehash_ms;
    timing.overhead_ms = (timing.total_ms - accounted_ms).max(0.0);

    state.push_log(
        "SEARCH",
        "INFO",
        format!(
            "Completed in {:.2}ms: matched='{}' (conf={:.1}%), candidates={}",
            timing.total_ms,
            matched_media.as_deref().unwrap_or("none"),
            phash_conf,
            total_candidates
        ),
    );

    Json(SearchSubtitlesResponse {
        media_path: payload.media_path,
        language: payload.language,
        matched_media,
        moviehash,
        fingerprint_preview,
        total_candidates,
        candidates,
        timing,
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

    // 1. Fetch raw subtitle according to source
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
            let path = payload
                .candidate_id
                .map(PathBuf::from)
                .unwrap_or_else(|| subtitles_dir.clone());
            match SubtitleFile::open(&path) {
                Ok(s) => (s, parsed_source.as_str().to_string()),
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

    // 2. Perform certified pHash resync if requested and target media is recognized in MmapLibrary
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
        let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_with_reader(
            &media_path,
            &phash_dirs,
            state.dbphash_reader.as_deref(),
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
        state.push_log(
            "DOWNLOAD",
            "ERROR",
            format!("Failed to write subtitle file to {:?}: {:#}", out_path, e),
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
                "Failed to write subtitle file to {:?}: {:#}",
                out_path, e
            )),
        });
    }

    state.push_log(
        "DOWNLOAD",
        "INFO",
        format!(
            "Saved subtitle to {:?} (source='{}', lang='{}', shift={:.2}ms)",
            out_path.file_name().unwrap_or_default(),
            source_name,
            payload.language,
            shift_ms
        ),
    );

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

pub async fn certify_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CertifySyncRequest>,
) -> Json<CertifySyncResponse> {
    let media_path = PathBuf::from(&payload.media_path);
    let subtitle_path = PathBuf::from(&payload.subtitle_path);
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let err_resp = |msg: String| {
        Json(CertifySyncResponse {
            success: false,
            media_name: String::new(),
            media_hash: String::new(),
            phash_filename: String::new(),
            subtitles_hash: String::new(),
            certified_srt_path: String::new(),
            cue_count: 0,
            sub_frames_count: 0,
            postgres_imported: false,
            registry_path: String::new(),
            error: Some(msg),
        })
    };

    if !media_path.exists() {
        return err_resp(format!("Media file not found: {:?}", media_path));
    }
    if !subtitle_path.exists() {
        return err_resp(format!("Subtitle file not found: {:?}", subtitle_path));
    }

    let (phash_file, hashes, fps) = match resolve_or_extract_phash(&media_path, &phash_dir) {
        Ok(res) => res,
        Err(e) => {
            return err_resp(format!(
                "Failed to resolve/extract media .phash in {:?}: {:#}",
                phash_dir, e
            ));
        }
    };

    let srt_file = match SubtitleFile::open(&subtitle_path) {
        Ok(s) => s,
        Err(e) => {
            return err_resp(format!("Failed to parse subtitle file: {:#}", e));
        }
    };

    let (sub_frames, fps) = match build_and_validate_sub_frames(&srt_file, &hashes, fps) {
        Ok(sf) => (sf, fps),
        Err(e) if (fps - 25.0).abs() < 1e-3 => {
            if let Ok(sf) = build_and_validate_sub_frames(&srt_file, &hashes, 23.976) {
                (sf, 23.976)
            } else {
                return err_resp(format!("Subtitle frame validation failed: {:#}", e));
            }
        }
        Err(e) => {
            return err_resp(format!("Subtitle frame validation failed: {:#}", e));
        }
    };

    let lang = normalize_lang_code(&payload.language);
    let lang = if lang == "auto" || lang == "all" || lang.is_empty() {
        subtitle_path
            .file_name()
            .and_then(|n| detect_subtitle_file_language(&n.to_string_lossy()))
            .unwrap_or_else(|| "und".to_string())
    } else {
        lang
    };

    let _ = std::fs::create_dir_all(&subtitles_dir);
    let phash_fname = phash_file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let media_name = phash_fname
        .strip_suffix(".phash")
        .unwrap_or(&phash_fname)
        .to_string();

    // Ensure the certified SRT exists inside subtitles_dir so catalog/certified lookups can serve it
    let dest_srt_path = if subtitle_path.starts_with(&subtitles_dir) {
        subtitle_path.clone()
    } else {
        let base_stem = media_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| media_name.clone());
        let cert_filename = format!("{}.{}.srt", base_stem, lang);
        let dest = subtitles_dir.join(&cert_filename);
        if dest != subtitle_path {
            if let Err(e) = std::fs::copy(&subtitle_path, &dest) {
                return err_resp(format!(
                    "Failed to copy certified subtitle to {:?}: {:#}",
                    dest, e
                ));
            }
        }
        dest
    };

    let raw_srt_bytes = match std::fs::read(&dest_srt_path) {
        Ok(b) => b,
        Err(e) => {
            return err_resp(format!("Failed to read certified subtitle bytes: {:#}", e));
        }
    };

    let media_hash = compute_media_phash_digest(&hashes);
    let subtitles_hash = compute_bytes_digest_32(&raw_srt_bytes);
    let rel_srt_name = dest_srt_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let reg_path = CouplingRegistry::default_path(&subtitles_dir);
    let mut reg = match CouplingRegistry::load_or_default(&reg_path) {
        Ok(r) => r,
        Err(e) => {
            return err_resp(format!("Failed to load coupling registry: {:#}", e));
        }
    };

    reg.upsert_coupling(
        media_hash.clone(),
        media_name.clone(),
        phash_fname.clone(),
        fps,
        hashes.len(),
        CertifiedSubtitleRecord {
            name: rel_srt_name.clone(),
            srt_path: rel_srt_name,
            lang: lang.clone(),
            subtitles_hash: subtitles_hash.clone(),
            certified: true,
            cue_count: srt_file.cues.len(),
            sub_frames: sub_frames.clone(),
        },
    );

    if let Err(e) = reg.save(&reg_path) {
        return err_resp(format!("Failed to save coupling registry: {:#}", e));
    }

    // Update in-memory registry cache so subsequent searches see the new coupling instantly
    if let Ok(mut lock) = state.coupling_registry.write() {
        *lock = reg;
    }

    let mut postgres_imported = false;
    if let Some(ref cfg) = state.db_config {
        if cfg
            .with_client(|client| {
                client.init_schema()?;
                client.import_subtitle_coupling(&phash_file, &dest_srt_path, &lang, payload.strict)
            })
            .is_ok()
        {
            postgres_imported = true;
        }
    }

    Json(CertifySyncResponse {
        success: true,
        media_name,
        media_hash,
        phash_filename: phash_fname,
        subtitles_hash,
        certified_srt_path: dest_srt_path.to_string_lossy().to_string(),
        cue_count: srt_file.cues.len(),
        sub_frames_count: sub_frames.len(),
        postgres_imported,
        registry_path: reg_path.to_string_lossy().to_string(),
        error: None,
    })
}

pub fn parse_fingerprint_hashes(fingerprint: &[String]) -> Vec<u64> {
    fingerprint
        .iter()
        .filter_map(|s| {
            let s_trim = s.trim();
            if let Some(hex_part) = s_trim
                .strip_prefix("0x")
                .or_else(|| s_trim.strip_prefix("0X"))
            {
                u64::from_str_radix(hex_part, 16).ok()
            } else if s_trim.chars().any(|c| matches!(c, 'a'..='f' | 'A'..='F')) {
                u64::from_str_radix(s_trim, 16).ok()
            } else {
                s_trim
                    .parse::<u64>()
                    .ok()
                    .or_else(|| u64::from_str_radix(s_trim, 16).ok())
            }
        })
        .collect()
}

pub async fn recognize_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RecognizeRequest>,
) -> Result<Json<RecognizeResponse>, (StatusCode, String)> {
    let t_start = Instant::now();
    state
        .metrics
        .total_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state
        .metrics
        .recognize_requests
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());

    // Branch A: Direct Fingerprint Mode (Remote o2switch / NAS / RPi4+ contract, 200 bytes, zero video I/O)
    if let Some(ref fp) = payload.query_fingerprint {
        let hashes = parse_fingerprint_hashes(fp);
        let req_fps = payload.fps.unwrap_or(DEFAULT_BASELINE_FPS);
        let req_lang = payload.language.as_deref().unwrap_or("auto");
        let search_distance = if payload.search_distance > 0 {
            payload.search_distance
        } else {
            DEFAULT_SEARCH_DISTANCE
        };

        tracing::info!(
            "[Recognize] Incoming fingerprint: {} frames (fps={:.2}, lang='{}')",
            hashes.len(),
            req_fps,
            req_lang
        );

        if hashes.is_empty() || hashes.iter().all(|&h| dbsrt_core::is_blank_phash(h)) {
            let elapsed = t_start.elapsed().as_secs_f64();
            state.metrics.total_latency_us.fetch_add(
                t_start.elapsed().as_micros() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            return Ok(Json(RecognizeResponse {
                found: false,
                media_name_search: "fingerprint_query".to_string(),
                media_match: None,
                nb_offsets_match: 0,
                confidence: 0.0,
                tier: None,
                shift_frames: None,
                shift_ms: None,
                candidates: None,
                timer_seconds: elapsed,
            }));
        }

        // 1. Tier 1: Check in-memory .dbphash MIH reader (<10ms)
        let mut matched: Option<(String, f64, String)> = None;
        if let Some(ref reader) = state.dbphash_reader {
            let m = reader.recognize(&hashes, search_distance);
            if let Some(top) = m.into_iter().next() {
                if top.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    matched = Some((
                        top.clean_title,
                        top.confidence,
                        "Tier 1 (RAM .dbphash MIH under 10ms)".to_string(),
                    ));
                }
            }
        }

        // 2. Tier 2: Check MmapLibrary Zero-DB files in phash_dirs
        if matched.is_none() {
            if let Ok(lib) = MmapLibrary::from_directories(&phash_dirs) {
                let m = lib.recognize(&hashes, search_distance);
                if let Some(top) = m.into_iter().next() {
                    let conf_pct = top.confidence * 100.0;
                    if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                        matched = Some((
                            top.media_name,
                            conf_pct,
                            "Tier 2 (MmapLibrary Zero-DB Files)".to_string(),
                        ));
                    }
                }
            }
        }

        // 3. Tier 3: Check PostgreSQL BK-Tree if configured
        if matched.is_none() {
            if let Some(ref cfg) = state.db_config {
                if let Ok(Some((rec, conf))) =
                    cfg.with_client(|client| client.recognize_sample(&hashes, search_distance))
                {
                    let conf_pct = conf * 100.0;
                    if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                        matched = Some((
                            rec.name,
                            conf_pct,
                            "Tier 3 (PostgreSQL 17 SP-GiST BK-Tree)".to_string(),
                        ));
                    }
                }
            }
        }

        let elapsed = t_start.elapsed().as_secs_f64();
        state.metrics.total_latency_us.fetch_add(
            t_start.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );

        if let Some((media_match, confidence, tier)) = matched {
            // Find reference hashes to compute shift Δt across all candidate prints
            let mut best_shift_pair = (0i64, 0.0f64, 0.0f64);
            let req_fps = if req_fps > 0.0 {
                req_fps
            } else {
                DEFAULT_BASELINE_FPS
            };

            for dir in &phash_dirs {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                            if name.ends_with(".phash") && !name.contains("_mid15s") {
                                let cand_stem = name.trim_end_matches(".phash");
                                let norm_m = norm_alnum(&media_match);
                                let norm_c = norm_alnum(cand_stem);
                                if norm_c.starts_with(&norm_m)
                                    || norm_m.starts_with(&norm_c)
                                    || name.contains(&media_match)
                                    || media_match.contains(cand_stem)
                                {
                                    if let Ok(ref_hashes) = read_phash_file(&p) {
                                        let (sf, match_conf) = calculate_sequence_shift_argmin(
                                            &hashes,
                                            &ref_hashes,
                                            req_fps,
                                        );
                                        if match_conf > best_shift_pair.2 {
                                            let sms = (sf as f64 / req_fps) * 1000.0;
                                            best_shift_pair = (sf, sms, match_conf);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let (shift_frames, shift_ms) =
                if best_shift_pair.2 >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    (Some(best_shift_pair.0), Some(best_shift_pair.1))
                } else {
                    (Some(0), Some(0.0))
                };

            // Lookup certified subtitles
            let (cert_cands, _, _, _) = lookup_certified_couplings(
                &state,
                &state.subtitles_dir,
                &media_match,
                req_lang,
                confidence,
            );

            // Generate candidates: raw 1:1 original + synced if visual shift is significant (> 500ms)
            let mut final_cands = Vec::new();
            for c in cert_cands {
                let mut c_raw = c.clone();
                c_raw.shift_ms = Some(0.0);
                final_cands.push(c_raw);

                if let Some(sms) = shift_ms {
                    if sms.abs() > SIGNIFICANT_SHIFT_THRESHOLD_MS {
                        let mut c_synced = c.clone();
                        c_synced.source = "certified_synced".to_string();
                        c_synced.shift_ms = Some(sms);
                        c_synced.title =
                            format!("{} [Synced {:+.1}s]", c.title, sms / MILLIS_PER_SEC);
                        if let Some(ref d) = c.details {
                            c_synced.details = Some(format!(
                                "{} (Synchronized: {:+.2}s visual shift)",
                                d,
                                sms / MILLIS_PER_SEC
                            ));
                        }
                        final_cands.push(c_synced);
                    }
                }
            }

            tracing::info!(
                "[Recognize] ✓ Matched in {:.2}ms: media='{}' (conf={:.1}%, tier='{}', candidates={})",
                elapsed * 1000.0,
                media_match,
                confidence,
                tier,
                final_cands.len()
            );

            state.push_log(
                "RECOGNIZE",
                "INFO",
                format!(
                    "✓ Matched in {:.2}ms: '{}' (conf={:.1}%, tier='{}', candidates={})",
                    elapsed * 1000.0,
                    media_match,
                    confidence,
                    tier,
                    final_cands.len()
                ),
            );

            return Ok(Json(RecognizeResponse {
                found: true,
                media_name_search: format!("fingerprint:{}frames", hashes.len()),
                media_match: Some(media_match),
                nb_offsets_match: hashes.len(),
                confidence,
                tier: Some(tier),
                shift_frames,
                shift_ms,
                candidates: Some(final_cands),
                timer_seconds: elapsed,
            }));
        } else {
            tracing::info!(
                "[Recognize] ✗ No match found in {:.2}ms (tested {} frames)",
                elapsed * 1000.0,
                hashes.len()
            );

            state.push_log(
                "RECOGNIZE",
                "INFO",
                format!(
                    "✗ No match found in {:.2}ms (tested {} frames)",
                    elapsed * 1000.0,
                    hashes.len()
                ),
            );

            return Ok(Json(RecognizeResponse {
                found: false,
                media_name_search: format!("fingerprint:{}frames", hashes.len()),
                media_match: None,
                nb_offsets_match: 0,
                confidence: 0.0,
                tier: None,
                shift_frames: None,
                shift_ms: None,
                candidates: None,
                timer_seconds: elapsed,
            }));
        }
    }

    // Branch B: Legacy Local Video Path Mode (probe_video + extract_frames_phash)
    let media_path = PathBuf::from(&payload.media_path);
    if !media_path.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Media file not found: {:?}", media_path),
        ));
    }

    let meta =
        probe_video(&media_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (payload.nb_seconds * meta.fps).round() as usize;

    let hashes = extract_frames_phash(&media_path, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let media_name_search = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if hashes.is_empty() || hashes.iter().all(|&h| dbsrt_core::is_blank_phash(h)) {
        let elapsed = t_start.elapsed().as_secs_f64();
        state.metrics.total_latency_us.fetch_add(
            t_start.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        return Ok(Json(RecognizeResponse {
            found: false,
            media_name_search,
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            tier: None,
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds: elapsed,
        }));
    }

    let library = MmapLibrary::from_directories(&phash_dirs)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let matches = library.recognize(&hashes, payload.search_distance);
    let timer_seconds = t_start.elapsed().as_secs_f64();
    state.metrics.total_latency_us.fetch_add(
        t_start.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );

    if let Some(top) = matches.first() {
        Ok(Json(RecognizeResponse {
            found: true,
            media_name_search,
            media_match: Some(top.media_name.clone()),
            nb_offsets_match: top.nb_offsets_match,
            confidence: top.confidence,
            tier: Some("Tier 2 (MmapLibrary Zero-DB Files)".to_string()),
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds,
        }))
    } else {
        Ok(Json(RecognizeResponse {
            found: false,
            media_name_search,
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            tier: None,
            shift_frames: None,
            shift_ms: None,
            candidates: None,
            timer_seconds,
        }))
    }
}

fn parse_phash_fps(name: &str, media_file: &Path) -> f64 {
    let mut fps = if let Some(fps_match) = name.strip_suffix("fps.phash") {
        let parts: Vec<&str> = fps_match.split('.').collect();
        if parts.len() >= 2 {
            let last_two = format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]);
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
    fps
}

pub fn find_existing_phash(
    media_file: &Path,
    phash_dir: &Path,
) -> Option<(PathBuf, Vec<u64>, f64)> {
    let full_name = media_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if media_file.extension().and_then(|e| e.to_str()) == Some("phash") && media_file.exists() {
        if let Ok(hashes) = read_phash_file(media_file) {
            let fps = parse_fps_from_phash_stem(&stem);
            return Some((media_file.to_path_buf(), hashes, fps));
        }
    }

    if !phash_dir.exists() {
        return None;
    }
    let entries: Vec<_> = std::fs::read_dir(phash_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();

    // Pass 1: Prefer exact full filename match (e.g. `foo.mp4.25.0fps.phash` over `foo.webm.25.0fps.phash`)
    if !full_name.is_empty() {
        for p in &entries {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.starts_with(&full_name) && name.ends_with(".phash") {
                    if let Ok(hashes) = read_phash_file(p) {
                        let fps = parse_phash_fps(name, media_file);
                        return Some((p.clone(), hashes, fps));
                    }
                }
            }
        }
    }

    // Pass 2: Fallback to stem match ONLY if the candidate does not belong to a different container extension
    let known_video_exts = [".mp4.", ".webm.", ".mkv.", ".avi.", ".mov."];
    let my_ext = media_file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}.", e.to_lowercase()));

    if !stem.is_empty() {
        for p in &entries {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.contains(&stem) && name.ends_with(".phash") {
                    let name_lower = name.to_lowercase();
                    let has_conflicting_ext = known_video_exts
                        .iter()
                        .any(|ext| name_lower.contains(ext) && my_ext.as_deref() != Some(*ext));
                    if has_conflicting_ext {
                        continue;
                    }
                    if let Ok(hashes) = read_phash_file(p) {
                        let fps = parse_phash_fps(name, media_file);
                        return Some((p.clone(), hashes, fps));
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
    let target_cuts = detect_cuts(&target_hashes, DEFAULT_CUT_THRESHOLD);
    let ref_cuts = detect_cuts(&ref_hashes, DEFAULT_CUT_THRESHOLD);

    let (shift_frames, confidence_percent) =
        calculate_shift_consensus(&target_cuts, &ref_cuts, payload.search_distance, 0.0);

    let shift_ms = (shift_frames as f64 / target_fps) * MILLIS_PER_SEC;

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
        let confidence_percent = (dominance * support * PERCENT_SCALE).clamp(0.0, PERCENT_SCALE);
        (shift_frames, confidence_percent)
    } else {
        (0, fallback_confidence)
    }
}

/// Finds the best frame shift between target_hashes and ref_hashes by sliding window
/// cross-correlation over 64-bit DCT pHashes.
/// Handles both snippet within full media and relative timeline offsets between releases.
pub fn calculate_sequence_shift_argmin(
    target_hashes: &[u64],
    ref_hashes: &[u64],
    target_fps: f64,
) -> (i64, f64) {
    if target_hashes.is_empty() || ref_hashes.is_empty() {
        return (0, 0.0);
    }

    let fps = if target_fps > 0.0 {
        target_fps
    } else {
        DEFAULT_BASELINE_FPS
    };
    let sample_len = target_hashes.len().min(MAX_CORRELATION_SAMPLE_FRAMES);
    let query = &target_hashes[..sample_len];

    // Case 1: Target is shorter or equal to ref (e.g. snippet inside full episode)
    if target_hashes.len() <= ref_hashes.len() {
        let max_pos = ref_hashes.len() - sample_len;
        let step = (fps.round() as usize).max(1);
        let mut coarse_best = 0;
        let mut coarse_min = u64::MAX;
        let mut pos = 0;

        while pos <= max_pos {
            let mut dist_sum = 0u64;
            for j in (0..sample_len).step_by(step) {
                dist_sum += (query[j] ^ ref_hashes[pos + j]).count_ones() as u64;
            }
            if dist_sum < coarse_min {
                coarse_min = dist_sum;
                coarse_best = pos;
            }
            pos += step;
        }

        // Fine refinement: +/- REFINEMENT_STEP_RADIUS steps around coarse_best
        let refine_radius = step * REFINEMENT_STEP_RADIUS;
        let refine_start = coarse_best.saturating_sub(refine_radius);
        let refine_end = (coarse_best + refine_radius).min(max_pos);
        let mut min_dist_sum = u64::MAX;
        let mut best_offset = coarse_best;

        for r_pos in refine_start..=refine_end {
            let dist_sum: u64 = query
                .iter()
                .zip(&ref_hashes[r_pos..r_pos + sample_len])
                .map(|(&q, &r)| (q ^ r).count_ones() as u64)
                .sum();
            if dist_sum < min_dist_sum {
                min_dist_sum = dist_sum;
                best_offset = r_pos;
            }
        }

        let avg_dist = min_dist_sum as f64 / sample_len as f64;
        let conf = ((1.0 - (avg_dist / PHASH_BIT_DEPTH)) * PERCENT_SCALE).clamp(0.0, PERCENT_SCALE);
        (-(best_offset as i64), conf)
    } else {
        // Case 2: Ref is shorter than target (ref is snippet or target has earlier frames)
        let max_pos = target_hashes.len() - sample_len;
        let step = (fps.round() as usize).max(1);
        let ref_sample_len = ref_hashes.len().min(MAX_CORRELATION_SAMPLE_FRAMES);
        let ref_query = &ref_hashes[..ref_sample_len];

        let mut coarse_best = 0;
        let mut coarse_min = u64::MAX;
        let mut pos = 0;

        while pos <= max_pos {
            let mut dist_sum = 0u64;
            for j in (0..ref_sample_len).step_by(step) {
                dist_sum += (ref_query[j] ^ target_hashes[pos + j]).count_ones() as u64;
            }
            if dist_sum < coarse_min {
                coarse_min = dist_sum;
                coarse_best = pos;
            }
            pos += step;
        }

        let refine_radius = step * REFINEMENT_STEP_RADIUS;
        let refine_start = coarse_best.saturating_sub(refine_radius);
        let refine_end = (coarse_best + refine_radius).min(max_pos);
        let mut min_dist_sum = u64::MAX;
        let mut best_offset = coarse_best;

        for t_pos in refine_start..=refine_end {
            let dist_sum: u64 = ref_query
                .iter()
                .zip(&target_hashes[t_pos..t_pos + ref_sample_len])
                .map(|(&r, &t)| (r ^ t).count_ones() as u64)
                .sum();
            if dist_sum < min_dist_sum {
                min_dist_sum = dist_sum;
                best_offset = t_pos;
            }
        }

        let avg_dist = min_dist_sum as f64 / ref_sample_len as f64;
        let conf = ((1.0 - (avg_dist / PHASH_BIT_DEPTH)) * PERCENT_SCALE).clamp(0.0, PERCENT_SCALE);
        (best_offset as i64, conf)
    }
}

pub fn find_subtitle_in_catalog(
    subtitles_dir: &Path,
    media_stem: &str,
    language: &str,
) -> Option<PathBuf> {
    let stems = vec![media_stem.to_string()];
    search_subtitles_in_catalog(subtitles_dir, &stems, None, None, None, language)
        .into_iter()
        .next()
        .map(|(_, path, _)| path)
}

pub fn read_ref_phash_for_media(phash_dir: &Path, media_name: &str) -> Result<Vec<u64>> {
    let clean_name = media_name.strip_suffix(".phash").unwrap_or(media_name);
    let entries = std::fs::read_dir(phash_dir)?;
    let mut candidate_paths = Vec::new();
    let norm_target = norm_alnum(clean_name);

    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            // Never treat a snippet file (*_mid15s*) as a reference print
            if name.ends_with(".phash") && !name.contains("_mid15s") {
                let cand_stem = name.trim_end_matches(".phash");
                if name == format!("{}.phash", clean_name) || name == clean_name {
                    return read_phash_file(&p);
                }
                let norm_cand = norm_alnum(cand_stem);
                if name.contains(clean_name)
                    || clean_name.contains(cand_stem)
                    || norm_cand.starts_with(&norm_target)
                    || norm_target.starts_with(&norm_cand)
                {
                    candidate_paths.push(p);
                }
            }
        }
    }
    if let Some(p) = candidate_paths.first() {
        return read_phash_file(p);
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

    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());
    let include_sidecars = payload.subtitles_dir.is_none();
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let media_stem = media_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // 1 & 2. Identify media across all configured directories via Multi-Tier pHash recognition
    let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_with_reader(
        &media_path,
        &phash_dirs,
        state.dbphash_reader.as_deref(),
        state.db_config.as_ref(),
        payload.nb_seconds,
        payload.search_distance,
    );

    let (is_recognized, matched_stem_opt, match_conf) = match matched {
        Some((m_name, conf)) => (true, Some(m_name), conf),
        None => (false, None, 0.0),
    };

    // 3. Find reference subtitle: catalog on disk OR embedded subtitle stream in container OR remote provider
    let search_stem = matched_stem_opt.as_deref().unwrap_or(&media_stem);
    let search_dirs = collect_subtitle_search_dirs(&subtitles_dir, &media_path, include_sidecars);
    let found_catalog = search_dirs
        .iter()
        .find_map(|d| find_subtitle_in_catalog(d, search_stem, &payload.language));
    let (mut srt, subtitle_source) = match found_catalog {
        Some(p) => match SubtitleFile::open(&p) {
            Ok(s) => (s, SubtitleSource::Catalog.as_str().to_string()),
            Err(e) => {
                return Json(AutoSyncResponse {
                    success: false,
                    matched_media: matched_stem_opt,
                    language: payload.language.clone(),
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: match_conf,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    source: Some(SubtitleSource::Catalog.as_str().to_string()),
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
                            Ok(s) => (s, SubtitleSource::Container.as_str().to_string()),
                            Err(e) => {
                                return Json(AutoSyncResponse {
                                    success: false,
                                    matched_media: matched_stem_opt,
                                    language: payload.language.clone(),
                                    shift_frames: 0,
                                    shift_ms: 0.0,
                                    output_srt_path: String::new(),
                                    confidence_percent: match_conf,
                                    elapsed_seconds: start.elapsed().as_secs_f64(),
                                    source: Some(SubtitleSource::Container.as_str().to_string()),
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
                                matched_media: matched_stem_opt,
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: Some(SubtitleSource::Container.as_str().to_string()),
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
                                matched_media: matched_stem_opt,
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: Some(SubtitleSource::OpenSubtitles.as_str().to_string()),
                                error: Some(format!(
                                    "Remote provider search failed for '{}' ({}): {:#}",
                                    meta.clean_title, payload.language, e
                                )),
                            });
                        }
                    };

                    let mut downloaded_srt = None;
                    for cand in candidates.iter().take(3) {
                        if let Ok(parsed) =
                            fetch_or_cache_remote_subtitle(&state, &meta, &payload.language, cand)
                                .await
                        {
                            downloaded_srt = Some(parsed);
                            break;
                        }
                    }

                    match downloaded_srt {
                        Some(s) => (s, SubtitleSource::OpenSubtitles.as_str().to_string()),
                        None => {
                            let err_msg = if is_recognized {
                                format!(
                                    "Media recognized as '{}' but no '{}' subtitle found in local catalog {:?}, container tracks, nor remote providers",
                                    matched_stem_opt.as_deref().unwrap_or(""), payload.language, subtitles_dir
                                )
                            } else {
                                format!(
                                    "Media not recognized by perceptual hash fingerprint in indexed libraries ({:?}) and no '{}' subtitle found in local catalog, container tracks, nor remote providers",
                                    phash_dirs, payload.language
                                )
                            };
                            return Json(AutoSyncResponse {
                                success: false,
                                matched_media: matched_stem_opt,
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: None,
                                error: Some(err_msg),
                            });
                        }
                    }
                }
            }
        }
    };

    // 4. Calculate shift via shared multi-directory helper if recognized:
    let (shift_frames, shift_ms, confidence_percent) = if let Some(ref ref_stem) = matched_stem_opt
    {
        compute_media_phash_shift_multi(
            &media_path,
            &phash_dirs,
            ref_stem,
            match_conf,
            target_fps,
            &existing_phash,
            payload.search_distance,
        )
    } else {
        (0, 0.0, 0.0)
    };

    // 5. Shift & Save Subtitle
    if shift_ms != 0.0 {
        srt.shift(shift_ms);
    }

    let out_dir = media_path
        .parent()
        .filter(|p| p.is_dir() && !p.to_string_lossy().is_empty())
        .unwrap_or_else(|| Path::new("/tmp"));

    let out_name = format!(
        "{}.{}.synced.srt",
        media_path.file_name().unwrap().to_string_lossy(),
        payload.language
    );
    let out_path = out_dir.join(&out_name);
    if let Err(e) = srt.save(&out_path) {
        return Json(AutoSyncResponse {
            success: false,
            matched_media: matched_stem_opt,
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
        matched_media: matched_stem_opt,
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
