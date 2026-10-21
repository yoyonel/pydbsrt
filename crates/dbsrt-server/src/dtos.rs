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
