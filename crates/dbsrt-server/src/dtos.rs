//! Data Transfer Objects (DTOs) and request/response models for dbsrt-server REST API.

use crate::constants::DEFAULT_EXTRACT_SAMPLE_SECONDS;
use dbsrt_core::{CertifiedSubtitleRecord, SubFrameAnchor};
use serde::{Deserialize, Serialize};

fn default_lang() -> String {
    "en".to_string()
}

fn default_true() -> bool {
    true
}

fn default_extract_duration() -> f64 {
    DEFAULT_EXTRACT_SAMPLE_SECONDS
}

fn default_scan_limit() -> usize {
    5
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CatalogReloadResponse {
    pub success: bool,
    pub elapsed_ms: f64,
    pub indexed_medias: usize,
    pub catalog_medias: usize,
    pub cuts_entries: usize,
    pub sparse_entries: usize,
    pub certified_couplings: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CatalogStatusResponse {
    pub base_medias: usize,
    pub base_cuts: usize,
    pub base_sparse: usize,
    pub delta_medias: usize,
    pub delta_cuts: usize,
    pub delta_sparse: usize,
    pub certified_couplings: usize,
    pub total_medias: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CatalogIngestRequest {
    /// Path to a .phash file on disk
    pub phash_path: Option<String>,
    /// Optional filename/title to parse metadata from
    pub filename: Option<String>,
    /// Optional raw hashes if sending directly via JSON payload
    pub hashes: Option<Vec<u64>>,
    /// Optional frame rate (defaults to 24.0)
    pub fps: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CatalogIngestResponse {
    pub success: bool,
    pub media_id: u32,
    pub clean_title: String,
    pub total_frames: usize,
    pub delta_medias_count: usize,
    pub elapsed_ms: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CatalogCompactResponse {
    pub success: bool,
    pub compacted: bool,
    pub elapsed_ms: f64,
    pub total_medias: usize,
    pub delta_medias_flushed: usize,
    pub message: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
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

/// Host operating system counters and identity for the web monitoring dashboard.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SystemStats {
    /// Hostname of the current server node.
    pub hostname: String,
    /// Operating system kernel description.
    pub os_name: String,
    /// Host CPU usage percentage (or load average normalized).
    pub cpu_pct: f32,
    /// Physical RAM consumed in Gibibytes (GiB).
    pub ram_used_gib: f64,
    /// Total physical RAM installed in Gibibytes (GiB).
    pub ram_total_gib: f64,
    /// System uptime in seconds.
    pub uptime_secs: u64,
}

/// Batch ingestion queue progress and worker allocation state.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BatchQueueStatus {
    /// Total count of video targets in current batch manifest.
    pub total_targets: usize,
    /// Count of successfully processed targets.
    pub completed_targets: usize,
    /// Count of targets that resulted in extraction failure.
    pub failed_targets: usize,
    /// Count of targets remaining in the processing queue.
    pub remaining_targets: usize,
    /// Overall completion percentage (`0.0 ..= 100.0`).
    pub progress_pct: f64,
    /// Number of concurrent background decoding workers active.
    pub active_workers: usize,
    /// Flag indicating whether the batch queue is actively in-flight.
    pub is_active: bool,
    /// ISO-8601 timestamp of last queue status update.
    pub updated_at: String,
}

/// Real-time status for an in-flight video extraction and hashing worker.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ActiveJobStatus {
    /// Linux process identifier (PID) of decoding worker or child ffmpeg.
    pub pid: u32,
    /// Clean title or stem of the media file being processed.
    pub media_name: String,
    /// Number of frames decoded so far.
    pub frames_decoded: usize,
    /// Total expected frames if known from container headers.
    pub total_frames: Option<usize>,
    /// Instantaneous decoding throughput in Frames Per Second (FPS).
    pub fps: f64,
    /// Elapsed execution wall time in seconds.
    pub elapsed_secs: f64,
    /// Normalized completion ratio (`0.0 ..= 1.0`).
    pub progress_ratio: f64,
    /// Estimated time to completion in seconds.
    pub eta_secs: Option<f64>,
    /// Worker process CPU percentage.
    pub cpu_percent: f32,
    /// Operational status string (`"RUNNING"`, `"PAUSED"`).
    pub status: String,
}

/// Remote watchdog supervisor status and recent activity logs.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SupervisorTelemetry {
    /// Host identifier where supervisor runs (e.g. `"rpi4"`).
    pub supervisor_host: String,
    /// Process identifier of the supervisor watchdog daemon.
    pub supervisor_pid: Option<u32>,
    /// High-level operational state (`"healthy"`, `"degraded"`, `"offline"`).
    pub state: String,
    /// Flag indicating whether heartbeat is fresh (within stale threshold).
    pub is_online: bool,
    /// ISO-8601 timestamp of last heartbeat received from supervisor.
    pub last_heartbeat_iso: String,
    /// Elapsed seconds since last received supervisor heartbeat.
    pub heartbeat_age_secs: u64,
    /// Instantaneous measured CloudLinux LVE vCPU percentage on target.
    pub lve_cpu_pct: Option<f64>,
    /// Rolling average CloudLinux LVE vCPU percentage over sample window.
    pub lve_cpu_avg_pct: Option<f64>,
    /// Target worker concurrency managed by supervisor.
    pub desired_workers: Option<usize>,
    /// In-flight worker jobs observed by supervisor.
    pub running_workers: Option<usize>,
    /// ISO-8601 timestamp of last persistent NAS backup.
    pub last_backup_iso: Option<String>,
    /// ISO-8601 timestamp of last catalog synchronization.
    pub last_sync_iso: Option<String>,
    /// Ring buffer of recent timestamped log lines from `rpi4_supervisor.log`.
    pub recent_logs: Vec<String>,
}

/// Record of a recently completed or failed batch video extraction.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RecentBatchEvent {
    /// Canonical video stem name.
    pub video_stem: String,
    /// Video container size in bytes.
    pub file_size_bytes: u64,
    /// Wall execution duration in seconds.
    pub wall_duration_secs: f64,
    /// Streaming read rate in Mebibytes per second (MiB/s).
    pub rate_mib_s: f64,
    /// Final job outcome label (`"COMPLETED"`, `"FAILED"`, `"ERROR"`).
    pub status: String,
    /// Boolean flag indicating success.
    pub success: bool,
    /// ISO-8601 completion timestamp.
    pub timestamp_iso: String,
}

/// Unified telemetry payload for the mobile-responsive web monitoring dashboard.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MonitoringTelemetryResponse {
    /// System and host counters.
    pub system: SystemStats,
    /// Batch queue progress metrics.
    pub batch_queue: BatchQueueStatus,
    /// Currently running video extraction jobs.
    pub active_jobs: Vec<ActiveJobStatus>,
    /// Autonomous RPi4 supervisor telemetry and log tail.
    pub supervisor: SupervisorTelemetry,
    /// Ring buffer of recently finished video events.
    pub recent_events: Vec<RecentBatchEvent>,
    /// Server wall time at snapshot creation (ISO-8601).
    pub server_time_iso: String,
}

/// Incoming heartbeat payload pushed by RPi4 supervisor agent.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SupervisorHeartbeatPayload {
    /// Host identifier where supervisor runs.
    pub supervisor_host: String,
    /// Process identifier of supervisor daemon.
    pub supervisor_pid: u32,
    /// Supervisor health state.
    pub state: String,
    /// Measured instantaneous LVE CPU percentage.
    pub lve_cpu_pct: Option<f64>,
    /// Rolling average LVE CPU percentage.
    pub lve_cpu_avg_pct: Option<f64>,
    /// Active desired worker concurrency.
    pub desired_workers: Option<usize>,
    /// In-flight worker jobs count.
    pub running_workers: Option<usize>,
    /// Recent log lines.
    #[serde(default)]
    pub recent_logs: Vec<String>,
}
