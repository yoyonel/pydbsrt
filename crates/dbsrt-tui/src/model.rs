//! # Data Models & Telemetry Pipeline for `dbsrt top`
//!
//! This module defines the domain entities, filtering predicates, sorting criteria,
//! statistical calculators, and state containers driving the `pydbsrt studio` (TUI top monitor).
//!
//! ## Architectural Pipeline & Data Flow
//! ```text
//! [NAS / Local Media Sources] ---> (Background Sampler / Scanner)
//!                                            │
//!                                            ▼
//!                                [DiscoveredMediaItem Catalog]
//!                                            │
//!                  ┌─────────────────────────┴─────────────────────────┐
//!                  ▼                                                   ▼
//!        [ActiveJobTelemetry]                                  [HistoricalRecord]
//!      (ProcFS / GPU / CIFS)                             (JSONL Logs / Batch Reports)
//!                  │                                                   │
//!                  ▼                                                   ▼
//!         Multi-Token Fuzzy Search                            Multi-Token Fuzzy Search
//!                  │                                                   │
//!                  ▼                                                   ▼
//!         [TUI View: 1 Tâches]                                [TUI View: 3/r Récents]
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::constants::{
    BYTES_PER_GIB_F64, CIVIL_DAYS_PER_400_YEARS, CIVIL_EPOCH_DAYS_OFFSET, MAX_PROGRESS_CAP,
    MAX_SANITY_DURATION_SECS, MIN_FULL_DECODE_DURATION_SECS, MIN_WALL_DURATION_NON_CACHE_SECS,
    P90_PERCENTILE_FACTOR, PROGRESS_BAR_CHAR_WIDTH, SECONDS_PER_DAY, SECONDS_PER_HOUR,
    SECONDS_PER_MINUTE,
};
pub use crate::host_info::HostSystemInfo;

/// Hardware GPU telemetry metrics sampled from `intel_gpu_top` or sysfs engines.
///
/// # Data Flow & Invariants
/// - Sampled periodically in a dedicated background worker (`sampler::start_gpu_sampler`).
/// - When Intel GPU metrics cannot be accessed (e.g. non-Intel, missing permissions, or CI sandbox),
///   `available` is set to `false` and all utilization percentages default to `0.0`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuTelemetry {
    /// Percentage utilization of the hardware video decoding engine (0.0 to 100.0).
    pub video_busy_pct: f64,
    /// Percentage utilization of the 3D/render execution units (0.0 to 100.0).
    pub render_busy_pct: f64,
    /// Current operating GPU frequency in Megahertz (MHz).
    pub freq_mhz: f64,
    /// Current package or GPU rail electrical consumption in Watts.
    pub power_watts: f64,
    /// Flag indicating whether live GPU telemetry was successfully probed and parsed.
    pub available: bool,
}

/// Real-time telemetry snapshot for a single decoding or visual pHash job process.
///
/// # Data Flow & Invariants
/// - Extracted by reading Linux `/proc/[pid]/cmdline`, `/proc/[pid]/stat`, and `/proc/[pid]/io`.
/// - Memory RSS is computed in Mebibytes (MiB); CIFS rate is derived by calculating delta bytes
///   divided by delta time across sampling intervals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveJobTelemetry {
    /// Linux Process Identifier (PID).
    pub pid: u32,
    /// Canonical base stem of the media file being decoded.
    pub media_name: String,
    /// Hardware decoding engine descriptor (`"VAAPI"`, `"QSV"`, `"CPU/sw"`).
    pub engine: String,
    /// Optional explanatory note or fallback reason (e.g. why CPU fallback occurred).
    #[serde(default)]
    pub engine_reason: Option<String>,
    /// Elapsed wall time in seconds since the job started executing.
    pub elapsed_secs: f64,
    /// Cumulative raw bytes read from filesystem / CIFS network share so far.
    pub read_bytes: u64,
    /// Total media file size in bytes.
    pub total_bytes: u64,
    /// Progress ratio normalized between `0.0` and `1.0` (read bytes / total bytes).
    pub progress_ratio: f64,
    /// Instantaneous read throughput in Mebibytes per second (MiB/s).
    pub current_rate_mib_s: f64,
    /// Instantaneous frame decoding / hashing throughput in Frames Per Second (FPS).
    #[serde(default)]
    pub hashing_fps: f64,
    /// Estimated time to completion (ETA) in seconds, if throughput is non-zero.
    pub eta_secs: Option<f64>,
    /// Percentage CPU usage for this process (can exceed 100% across multiple cores).
    pub cpu_percent: f32,
    /// Resident Set Size (RSS) physical RAM footprint in Mebibytes (MiB).
    pub ram_rss_mib: f64,
    /// Operational status string (`"RUNNING"`, `"PAUSED"`, `"EXITED"`).
    pub status: String,
    /// Complete command-line arguments vector from `/proc/[pid]/cmdline`.
    pub cmdline: Vec<String>,
}

impl ActiveJobTelemetry {
    /// Checks whether the active job matches an interactive search query.
    ///
    /// # Arguments
    /// - `query`: Search string containing space or delimiter-separated tokens.
    ///
    /// # Returns
    /// - `true` if all search query tokens match across the PID, media stem, engine,
    ///   status label, or command line arguments.
    /// - `true` if `query` is empty or only whitespace.
    /// - `false` otherwise.
    ///
    /// # Data Flow & Invariants
    /// Formats an aggregated search target string composed of PID, media stem, engine,
    /// status, and cmdline, then delegates to [`matches_fuzzy_search`].
    pub fn matches_search(&self, query: &str) -> bool {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return true;
        }
        let pool = format!(
            "{} {} {} {} {}",
            self.pid,
            self.media_name,
            self.engine,
            self.status,
            self.cmdline.join(" ")
        );
        matches_fuzzy_search(&pool, trimmed)
    }
}

/// Historical record of a completed media ingestion, pHash extraction, or certification run.
///
/// # Data Flow & Invariants
/// Deserialized from JSONL batch execution logs (e.g. `data/reports/batch_tvshow_nightly.jsonl`).
/// Each record captures the terminal status, throughput, durations, and coupling counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HistoricalRecord {
    /// Canonical filename stem of the processed media file.
    pub video_stem: String,
    /// Relative path within the media source library (if known).
    #[serde(default)]
    pub video_rel_path: Option<String>,
    /// Global exit status (`true` for nominal zero exit code, `false` on failure or cancellation).
    pub success: bool,
    /// Total wall execution duration in seconds.
    pub wall_duration_secs: f64,
    /// Average CIFS / storage read throughput sustained across execution in MiB/s.
    pub cifs_read_rate_mib_s: f64,
    /// Total file size of the media container in bytes.
    pub file_size_bytes: u64,
    /// Count of certified ground-truth subtitle tracks linked during processing.
    pub certified_couplings_count: usize,
    /// Error diagnostic details or exit signal description on failure.
    #[serde(default)]
    pub error_summary: Option<String>,
    /// ISO-8601 start timestamp of the processing run.
    pub start_iso: Option<String>,
    /// ISO-8601 completion timestamp of the processing run.
    pub end_iso: Option<String>,
}

/// Granular domain execution status and operational severity of a historical record.
///
/// # Invariants
/// Delineates between nominal runs, fast cache re-validations, missing subtitles,
/// deliberate operator aborts, and genuine decoder/IO crashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletedStatus {
    /// Fully decoded and certified with at least one subtitle coupling.
    Ok,
    /// Fast cache hit or pre-existing entry (< 1s duration).
    Cache,
    /// Non-fatal functional skip: valid media file, but no matching .srt subtitle was found.
    NoSrt,
    /// Deliberate operator or system cancellation (SIGTERM/SIGKILL/abort).
    Cancelled,
    /// True fatal decoder, ffmpeg, or filesystem failure.
    Fail,
}

impl CompletedStatus {
    /// Returns the short ASCII badge displayed in TUI tables and logs.
    ///
    /// # Returns
    /// Static string badge (e.g. `"✓ OK"`, `"✓ CACHE"`, `"⚠ NO_SRT"`, `"⏹ CANCEL"`, `"✗ FAIL"`).
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Ok => "✓ OK",
            Self::Cache => "✓ CACHE",
            Self::NoSrt => "⚠ NO_SRT",
            Self::Cancelled => "⏹ CANCEL",
            Self::Fail => "✗ FAIL",
        }
    }

    /// Returns the descriptive human-readable French severity label used in diagnostics and contextual help.
    ///
    /// # Returns
    /// Static French description string.
    pub fn severity_label(&self) -> &'static str {
        match self {
            Self::Ok => "Succès certifié",
            Self::Cache => "En Cache",
            Self::NoSrt => "Avertissement (Sous-titres manquants)",
            Self::Cancelled => "Interrompu par l'utilisateur",
            Self::Fail => "Erreur critique décodeur",
        }
    }
}

impl HistoricalRecord {
    /// Deduces the granular domain status and operational severity of the record.
    ///
    /// # Arguments
    /// None (operates on `&self`).
    ///
    /// # Returns
    /// - [`CompletedStatus::Ok`]: Nominal execution with wall duration >= 1.0s.
    /// - [`CompletedStatus::Cache`]: Pre-existing fingerprint/cache re-validation (< 1.0s).
    /// - [`CompletedStatus::NoSrt`]: Valid media processed, but missing matching subtitle track.
    /// - [`CompletedStatus::Cancelled`]: Interrupted by SIGTERM, SIGKILL, or operator abort.
    /// - [`CompletedStatus::Fail`]: Fatal decoder error, ffmpeg crash, or filesystem fault.
    ///
    /// # Data Flow & Invariants
    /// Inspects `self.success` first. If false, inspects `self.error_summary` for known
    /// error substrings (`"no matching .srt"`, `"sigterm"`, `"signal 15"`, etc.).
    pub fn status(&self) -> CompletedStatus {
        if self.success {
            if self.wall_duration_secs < MIN_WALL_DURATION_NON_CACHE_SECS {
                CompletedStatus::Cache
            } else {
                CompletedStatus::Ok
            }
        } else if let Some(ref err) = self.error_summary {
            let err_lower = err.to_lowercase();
            if err_lower.contains("no matching .srt")
                || err_lower.contains("aucun fichier .srt")
                || err_lower.contains("no .srt")
                || err_lower.contains("missing .srt")
            {
                CompletedStatus::NoSrt
            } else if err_lower.contains("sigterm")
                || err_lower.contains("sigkill")
                || err_lower.contains("exit code -15")
                || err_lower.contains("exit code -9")
                || err_lower.contains("exit code -2")
                || err_lower.contains("interrupted")
                || err_lower.contains("signal 15")
                || err_lower.contains("signal 9")
            {
                CompletedStatus::Cancelled
            } else {
                CompletedStatus::Fail
            }
        } else {
            CompletedStatus::Fail
        }
    }

    /// Checks whether the record matches a search query across video stem, relative path,
    /// error summary, and status label.
    ///
    /// # Arguments
    /// - `query`: Search string containing space or delimiter-separated tokens.
    ///
    /// # Returns
    /// - `true` if all search query tokens match across the stem, rel path, error summary,
    ///   or localized status badge and severity label.
    /// - `true` if `query` is empty or only whitespace.
    /// - `false` otherwise.
    ///
    /// # Data Flow & Invariants
    /// Formats an aggregated candidate string and evaluates it against [`matches_fuzzy_search`].
    pub fn matches_search(&self, query: &str) -> bool {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return true;
        }
        let pool = format!(
            "{} {} {} {} {}",
            self.video_stem,
            self.video_rel_path.as_deref().unwrap_or(""),
            self.error_summary.as_deref().unwrap_or(""),
            self.status().badge(),
            self.status().severity_label()
        );
        matches_fuzzy_search(&pool, trimmed)
    }
}

/// Filtering predicate applied in the completed media view (`[3/r]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompletedFilter {
    #[default]
    All,
    SuccessOnly,
    FailedOnly,
    NoSrtOnly,
    CancelledOnly,
}

impl CompletedFilter {
    /// Cycles to the next filter state in sequence.
    ///
    /// # Arguments
    /// None (`self` consumed by value).
    ///
    /// # Returns
    /// The next [`CompletedFilter`] variant.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::SuccessOnly,
            Self::SuccessOnly => Self::FailedOnly,
            Self::FailedOnly => Self::NoSrtOnly,
            Self::NoSrtOnly => Self::CancelledOnly,
            Self::CancelledOnly => Self::All,
        }
    }

    /// Returns the uppercase French label displayed in the TUI header badge.
    ///
    /// # Returns
    /// Static string label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "TOUS",
            Self::SuccessOnly => "✓ SUCCÈS (OK/CACHE)",
            Self::FailedOnly => "✗ FAIL (ERREURS SEULES)",
            Self::NoSrtOnly => "⚠ SANS SOUS-TITRES (NO_SRT)",
            Self::CancelledOnly => "⏹ ANNULÉS (CANCEL)",
        }
    }

    /// Evaluates whether a given historical record matches this filter predicate.
    ///
    /// # Arguments
    /// - `record`: Historical record reference to evaluate.
    ///
    /// # Returns
    /// - `true` if the record's computed status satisfies the filter.
    /// - `false` otherwise.
    pub fn matches(self, record: &HistoricalRecord) -> bool {
        match self {
            Self::All => true,
            Self::SuccessOnly => {
                matches!(
                    record.status(),
                    CompletedStatus::Ok | CompletedStatus::Cache
                )
            }
            Self::FailedOnly => record.status() == CompletedStatus::Fail,
            Self::NoSrtOnly => record.status() == CompletedStatus::NoSrt,
            Self::CancelledOnly => record.status() == CompletedStatus::Cancelled,
        }
    }
}

/// Sorting criteria applied in the completed media view (`[3/r]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompletedSort {
    #[default]
    DateDesc,
    DateAsc,
    Status,
    Name,
    Duration,
    Size,
    Rate,
    Couplings,
}

impl CompletedSort {
    /// Cycles to the next sort criterion in sequence.
    ///
    /// # Returns
    /// The next [`CompletedSort`] variant.
    pub fn next(self) -> Self {
        match self {
            Self::DateDesc => Self::Status,
            Self::Status => Self::Name,
            Self::Name => Self::Duration,
            Self::Duration => Self::Size,
            Self::Size => Self::Rate,
            Self::Rate => Self::Couplings,
            Self::Couplings => Self::DateAsc,
            Self::DateAsc => Self::DateDesc,
        }
    }

    /// Returns the uppercase French label displayed in the TUI header.
    ///
    /// # Returns
    /// Static string label.
    pub fn label(self) -> &'static str {
        match self {
            Self::DateDesc => "DATE FIN (RÉCENT -> ANCIEN)",
            Self::DateAsc => "DATE FIN (ANCIEN -> RÉCENT)",
            Self::Status => "STATUT (FAIL / CANCEL / NO_SRT)",
            Self::Name => "NOM (A-Z)",
            Self::Duration => "DURÉE (DÉCROISSANT)",
            Self::Size => "TAILLE (DÉCROISSANT)",
            Self::Rate => "DÉBIT CIFS (DÉCROISSANT)",
            Self::Couplings => "COUPLINGS (DÉCROISSANT)",
        }
    }
}

/// Ingestion and certification lifecycle status of a candidate media release.
///
/// # Lifecycle Pipeline
/// 1. `Virgin`: Discovered on disk, zero fingerprint extracted yet.
/// 2. `InProgress`: Currently being decoded by an active background worker.
/// 3. `Completed`: Decoded, pHash index stored, and certified ground-truth subtitles linked.
/// 4. `PartialFail`: Video fingerprinted, but subtitle track was missing or invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IngestionStatus {
    /// Fresh, un-processed release with zero precomputed fingerprints.
    #[default]
    Virgin,
    /// Currently being decoded or processed by an active background worker.
    InProgress,
    /// Fully processed with ground-truth subtitle couplings stored.
    Completed,
    /// Processed or fingerprinted with missing or failed subtitle tracks.
    PartialFail,
}

impl IngestionStatus {
    /// Exhaustive slice of all variants for testing, cycling and reflection.
    pub const ALL: &'static [Self] = &[
        Self::Virgin,
        Self::InProgress,
        Self::Completed,
        Self::PartialFail,
    ];

    /// Returns the uppercase display badge string.
    ///
    /// # Returns
    /// Static string label (`"VIERGE"`, `"EN COURS"`, `"COMPLÉTÉ"`, `"HACHÉ SEUL"`).
    pub fn label(&self) -> &'static str {
        match self {
            Self::Virgin => "VIERGE",
            Self::InProgress => "EN COURS",
            Self::Completed => "COMPLÉTÉ",
            Self::PartialFail => "HACHÉ SEUL",
        }
    }
}

/// Information about precomputed or existing visual pHash fingerprint on disk (`.dbphash`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PhashFingerprintInfo {
    /// Whether the `.dbphash` binary sidecar exists on disk.
    pub exists: bool,
    /// Count of 64-bit frame visual hashes stored in the file.
    pub frame_count: usize,
    /// File size in bytes of the fingerprint binary.
    pub size_bytes: u64,
}

/// Metadata for an embedded or sidecar subtitle track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSubtrackInfo {
    /// Subtitle stream or file index.
    pub index: usize,
    /// Normalized ISO-639-1 language code (`"fr"`, `"en"`).
    pub lang: String,
    /// Subtitle format/codec descriptor (`"SubRip"`, `"ASS"`, etc.).
    pub codec: String,
    /// Stream title or track commentary.
    pub title: String,
}

/// Persistent configuration and operational state for a media storage source.
///
/// # Data Flow & Invariants
/// Serialized and persisted to `data/config/media_sources.json`.
/// Sources can be dynamically enabled/disabled in Tab `[5/s]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MediaSourceConfig {
    /// Unique machine identifier for this source (e.g. `"src_tvshow"`).
    pub id: String,
    /// Human-friendly display name (e.g. `"Synology NAS (Séries TV)"`).
    pub name: String,
    /// Filesystem directory root path.
    pub path: PathBuf,
    /// Whether this source is actively scanned and included in default views.
    pub enabled: bool,
    /// Total count of media containers discovered under this root.
    #[serde(default)]
    pub media_count: usize,
    /// Count of new media files added during the most recent scan.
    #[serde(default)]
    pub last_scan_added: usize,
    /// ISO-8601 timestamp of the most recent library scan.
    #[serde(default)]
    pub last_scan_iso: Option<String>,
}

/// Detailed metadata for a candidate video release discovered in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredMediaItem {
    /// Absolute filesystem path to the video file container.
    pub path: PathBuf,
    /// Base filename stem without container extension.
    pub stem: String,
    /// Total file size in bytes.
    pub size_bytes: u64,
    /// Total container duration in seconds (if probed via ffprobe).
    pub duration_secs: Option<f64>,
    /// Video codec moniker (`"hevc"`, `"h264"`).
    pub video_codec: Option<String>,
    /// List of embedded or sidecar subtitle tracks found.
    pub subtitles: Vec<MediaSubtrackInfo>,
    /// Optional status of visual pHash fingerprint on disk.
    #[serde(default)]
    pub phash: Option<PhashFingerprintInfo>,
    /// Source identifier (`MediaSourceConfig.id`) that discovered this item.
    #[serde(default)]
    pub source_id: Option<String>,
    /// Ingestion lifecycle state.
    pub status: IngestionStatus,
}

impl DiscoveredMediaItem {
    /// Checks whether the media item matches a search query across stem, file path, codec, source ID, and status.
    ///
    /// # Arguments
    /// - `query`: Search string containing space or delimiter-separated tokens.
    ///
    /// # Returns
    /// - `true` if all search query tokens match across stem, path, codec, source ID, or status label.
    /// - `true` if `query` is empty or only whitespace.
    /// - `false` otherwise.
    ///
    /// # Data Flow & Invariants
    /// Formats an aggregated pool string and delegates to [`matches_fuzzy_search`].
    pub fn matches_search(&self, query: &str) -> bool {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return true;
        }
        let pool = format!(
            "{} {} {} {} {}",
            self.stem,
            self.path.display(),
            self.video_codec.as_deref().unwrap_or(""),
            self.source_id.as_deref().unwrap_or(""),
            self.status.label()
        );
        matches_fuzzy_search(&pool, trimmed)
    }
}

/// Filtering predicate applied in the media browser view (`[4/m]`).
///
/// # Invariants
/// Structurally and mathematically guaranteed 100% aligned with [`IngestionStatus`]
/// through the `Status(IngestionStatus)` variant and exhaustive pattern matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaFilter {
    #[default]
    All,
    Status(IngestionStatus),
    WithSubtitles,
    WithoutSubtitles,
}

impl From<IngestionStatus> for MediaFilter {
    fn from(status: IngestionStatus) -> Self {
        Self::Status(status)
    }
}

impl MediaFilter {
    /// Ordered list of all filter variants for cycler `[f]`.
    ///
    /// # Returns
    /// Vector containing all filter variants in logical display sequence.
    pub fn all_variants() -> Vec<Self> {
        let mut variants = vec![Self::All];
        for &status in IngestionStatus::ALL {
            variants.push(Self::Status(status));
        }
        variants.push(Self::WithSubtitles);
        variants.push(Self::WithoutSubtitles);
        variants
    }

    /// Cycles to the next filter state in sequence.
    ///
    /// # Returns
    /// The next [`MediaFilter`] variant.
    pub fn next(&self) -> Self {
        let variants = Self::all_variants();
        let idx = variants.iter().position(|v| v == self).unwrap_or(0);
        let next_idx = (idx + 1) % variants.len();
        variants[next_idx]
    }

    /// Extracts target status if this filter is a status-specific variant.
    ///
    /// # Returns
    /// `Some(IngestionStatus)` if variant is `Status(...)`, `None` otherwise.
    pub fn target_status(&self) -> Option<IngestionStatus> {
        match self {
            Self::Status(s) => Some(*s),
            _ => None,
        }
    }

    /// Returns the uppercase French label displayed in the TUI header.
    ///
    /// # Returns
    /// Static string label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "TOUS",
            Self::Status(s) => match s {
                IngestionStatus::Virgin => "VIERGES SEULS",
                IngestionStatus::InProgress => "EN COURS",
                IngestionStatus::Completed => "COMPLÉTÉS",
                IngestionStatus::PartialFail => "HACHÉS SEULS",
            },
            Self::WithSubtitles => "AVEC SOUS-TITRES",
            Self::WithoutSubtitles => "SANS SOUS-TITRE",
        }
    }
}

/// Sorting order criterion for the media browser (`[4/m]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaSort {
    #[default]
    Name,
    Size,
    Duration,
    Status,
    Subtitles,
}

impl MediaSort {
    /// Cycles to the next sort criterion in sequence.
    ///
    /// # Returns
    /// The next [`MediaSort`] variant.
    pub fn next(&self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Duration,
            Self::Duration => Self::Status,
            Self::Status => Self::Subtitles,
            Self::Subtitles => Self::Name,
        }
    }

    /// Returns the uppercase French label displayed in the TUI header.
    ///
    /// # Returns
    /// Static string label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Name => "NOM (A-Z)",
            Self::Size => "TAILLE",
            Self::Duration => "DURÉE",
            Self::Status => "STATUT",
            Self::Subtitles => "SOUS-TITRES",
        }
    }
}

/// Descriptive statistics across a single performance or operational dimension.
///
/// # Invariants
/// Calculated by [`calculate_dimension_stats`]. Computes minimum, median, mean,
/// 90th percentile (P90), maximum, and sample standard deviation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DimensionStats {
    /// Minimum observed sample value.
    pub min: f64,
    /// 50th percentile (median) sample value.
    pub median: f64,
    /// Arithmetic mean across all samples.
    pub mean: f64,
    /// 90th percentile (P90) tail metric.
    pub p90: f64,
    /// Maximum observed sample value.
    pub max: f64,
    /// Sample standard deviation.
    pub stdev: f64,
}

/// Statistical summary across all decoded media dimensions in Tab `[2/h]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OverallStats {
    /// Total count of successful full-length decoding runs included in the sample.
    pub sample_count: usize,
    /// Read throughput statistics in MiB/s.
    pub rate: DimensionStats,
    /// Wall decoding duration statistics in minutes.
    pub duration_mins: DimensionStats,
    /// Media container size statistics in Gibibytes (GiB).
    pub size_gib: DimensionStats,
    /// Count of certified ground-truth subtitle couplings per media.
    pub couplings: DimensionStats,
}

/// Cumulative throughput and volume across the active user session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionThroughput {
    /// Total bytes processed across the session.
    pub session_bytes: u64,
    /// Elapsed session monitoring time in seconds.
    pub session_duration_secs: f64,
    /// Aggregate sustained read rate in MiB/s.
    pub session_rate_mib_s: f64,
    /// Total count of media streams processed during the session.
    pub stream_count: usize,
}

/// Full-frame view page identifier for the top TUI state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewPage {
    /// Full-frame interactive table of all active decoding/pHash jobs (`[1]`).
    #[default]
    ActiveJobs,
    /// Full-frame statistical distribution table of historical decoding metrics (`[2/h]`).
    StatsHistory,
    /// Full-frame scrollable log of all certified and completed media (`[3/r]`).
    CompletedMedia,
    /// Full-frame interactive browser of available media files on storage source (`[4/m]`).
    MediaBrowser,
    /// Full-frame manager for configured media library sources (`[5/s]`).
    MediaSources,
    /// Full-frame Fastfetch-like system and hardware host diagnostics view (`[6/y]`).
    System,
    /// Full-frame inspection view of selected task (cmdline, I/O, threads, memory, logs).
    JobInspection,
    /// Full-frame inspection and diagnostic view of candidate media release.
    MediaInspection,
    /// Full-frame inspection and diagnostic view of completed/failed media execution.
    CompletedInspection,
    /// Full-frame contextual keybinding and operational cheat sheet (`[?]`).
    Help,
}

/// Selected tab within the fullframe contextual Help & Glossary view (`[?]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTab {
    #[default]
    General,
    ActiveJobs,
    StatsHistory,
    Completed,
    MediaBrowser,
    MediaSources,
    System,
}

impl HelpTab {
    /// All help tabs in display order.
    pub const ALL: [HelpTab; 7] = [
        HelpTab::General,
        HelpTab::ActiveJobs,
        HelpTab::StatsHistory,
        HelpTab::Completed,
        HelpTab::MediaBrowser,
        HelpTab::MediaSources,
        HelpTab::System,
    ];

    /// Returns the tab title badge string.
    pub fn label(&self) -> &'static str {
        match self {
            HelpTab::General => "[0/g] Général",
            HelpTab::ActiveJobs => "[1] Tâches",
            HelpTab::StatsHistory => "[2/h] Stats",
            HelpTab::Completed => "[3/r] Récents",
            HelpTab::MediaBrowser => "[4/m] Médias",
            HelpTab::MediaSources => "[5/s] Sources",
            HelpTab::System => "[6/y] Système",
        }
    }

    /// Cycles to the next contextual help tab.
    pub fn next(&self) -> Self {
        match self {
            HelpTab::General => HelpTab::ActiveJobs,
            HelpTab::ActiveJobs => HelpTab::StatsHistory,
            HelpTab::StatsHistory => HelpTab::Completed,
            HelpTab::Completed => HelpTab::MediaBrowser,
            HelpTab::MediaBrowser => HelpTab::MediaSources,
            HelpTab::MediaSources => HelpTab::System,
            HelpTab::System => HelpTab::General,
        }
    }

    /// Cycles to the previous contextual help tab.
    pub fn prev(&self) -> Self {
        match self {
            HelpTab::General => HelpTab::System,
            HelpTab::ActiveJobs => HelpTab::General,
            HelpTab::StatsHistory => HelpTab::ActiveJobs,
            HelpTab::Completed => HelpTab::StatsHistory,
            HelpTab::MediaBrowser => HelpTab::Completed,
            HelpTab::MediaSources => HelpTab::MediaBrowser,
            HelpTab::System => HelpTab::MediaSources,
        }
    }

    /// Maps an active TUI view page to its corresponding contextual help tab.
    ///
    /// # Arguments
    /// - `view`: The active [`ViewPage`] before opening help.
    ///
    /// # Returns
    /// The mapped [`HelpTab`].
    pub fn from_view(view: ViewPage) -> Self {
        match view {
            ViewPage::ActiveJobs | ViewPage::JobInspection => HelpTab::ActiveJobs,
            ViewPage::StatsHistory => HelpTab::StatsHistory,
            ViewPage::CompletedMedia | ViewPage::CompletedInspection => HelpTab::Completed,
            ViewPage::MediaBrowser | ViewPage::MediaInspection => HelpTab::MediaBrowser,
            ViewPage::MediaSources => HelpTab::MediaSources,
            ViewPage::System => HelpTab::System,
            ViewPage::Help => HelpTab::General,
        }
    }
}

/// Volatile batch processing queue progress metrics.
///
/// # Data Flow & Invariants
/// Reflects the state of the active batch queue from `run/batch_queue.json` or fallback
/// target manifest. Invariant: `remaining_targets == total_targets.saturating_sub(completed_targets + failed_targets)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BatchQueueTelemetry {
    /// Total number of target media items discovered for this batch.
    pub total_targets: usize,
    /// Number of media items successfully processed.
    pub completed_targets: usize,
    /// Number of media items that failed processing.
    pub failed_targets: usize,
    /// Number of media items still pending in the queue.
    pub remaining_targets: usize,
    /// Overall batch progress percentage (0.0% to 100.0%).
    pub progress_pct: f64,
    /// Number of parallel worker processes actively decoding.
    pub active_workers: usize,
    /// Flag indicating whether the batch queue is actively running.
    pub is_active: bool,
    /// ISO 8601 UTC timestamp of last status update.
    pub updated_at: String,
}

/// System-wide combined telemetry snapshot.
///
/// # Invariants
/// Aggregates hardware CPU/RAM/GPU counters, active processes, CIFS transfer rates,
/// batch history records, and media catalog state into a single cohesive snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemTelemetry {
    /// Static host and hardware identification.
    pub host_info: HostSystemInfo,
    /// Total host CPU load percentage across all logical cores.
    pub host_cpu_pct: f32,
    /// Physical RAM consumed in Gibibytes (GiB).
    pub ram_used_gib: f64,
    /// Total installed system RAM in Gibibytes (GiB).
    pub ram_total_gib: f64,
    /// RAM charged to this account in GiB: cPanel LVE physical memory when `uapi` is
    /// available, else the sum of PSS of this user's processes. `/proc/meminfo` reports the
    /// whole shared host under CloudLinux LVE.
    #[serde(default)]
    pub account_ram_gib: Option<f64>,
    /// Account RAM limit in GiB (cPanel LVE contract), when known.
    #[serde(default)]
    pub account_ram_limit_gib: Option<f64>,
    /// Account CPU load in percent of its cPanel LVE limit (`lvecpu`), when known.
    /// `host_cpu_pct` covers the whole shared host.
    #[serde(default)]
    pub account_cpu_pct: Option<f64>,
    /// Hardware Intel GPU telemetry.
    pub gpu: GpuTelemetry,
    /// Active decoding and pHash job snapshots.
    pub active_jobs: Vec<ActiveJobTelemetry>,
    /// Aggregate instantaneous disk/CIFS read rate in MiB/s across all workers.
    pub total_rate_mib_s: f64,
    /// Network interface ingress bandwidth in MiB/s.
    pub net_rx_mib_s: f64,
    /// Cumulative session processing throughput.
    pub session: SessionThroughput,
    /// Historical completed batch records loaded from JSONL.
    pub history_records: Vec<HistoricalRecord>,
    /// Computed statistical distribution metrics.
    pub stats: OverallStats,
    /// Catalog of discovered media items in storage sources.
    pub discovered_media: Vec<DiscoveredMediaItem>,
    /// Configured media library storage roots.
    pub media_sources: Vec<MediaSourceConfig>,
    /// Flag indicating whether a background directory scan is actively running.
    pub is_scanning: bool,
    /// Active batch processing queue telemetry.
    pub batch_queue: BatchQueueTelemetry,
    /// Flag indicating whether the node operates in remote streaming mode (Zero-NVMe FTP).
    #[serde(default)]
    pub is_remote_streaming: bool,
}

impl SystemTelemetry {
    /// Formats the account RAM as `used/limit GiB (pct%)`, or `used GiB` without a limit.
    pub fn account_ram_label(&self) -> Option<String> {
        let used = self.account_ram_gib?;
        Some(match self.account_ram_limit_gib {
            Some(limit) if limit > 0.0 => {
                format!("{used:.2}/{limit:.0} GiB ({:.1}%)", used / limit * 100.0)
            }
            _ => format!("{used:.2} GiB"),
        })
    }
}

/// Active input field focus within the source configuration modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceModalField {
    Name,
    #[default]
    Path,
}

/// Operation mode for source configuration dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceModalMode {
    /// Adding a brand new media source.
    Add,
    /// Editing an existing media source at the given index in `media_sources`.
    Edit(usize),
    /// Confirming deletion of an existing media source at the given index.
    DeleteConfirm(usize),
}

/// State of the interactive modal dialog for adding, editing, or deleting a media source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInputModalState {
    /// Active dialog operational mode (Add, Edit, DeleteConfirm).
    pub mode: SourceModalMode,
    /// Text input buffer for custom source display name.
    pub name_input: String,
    /// Text input buffer for filesystem path.
    pub path_input: String,
    /// Currently focused text field for keyboard input.
    pub active_field: SourceModalField,
    /// Indicates whether target source is currently active (used for delete warnings).
    pub is_active_source: bool,
    /// Transient inline validation error message rendered inside the modal.
    pub error_msg: Option<String>,
}

impl SourceInputModalState {
    /// Instantiates modal state for adding a new media source.
    ///
    /// # Returns
    /// Initialized [`SourceInputModalState`] in [`SourceModalMode::Add`] mode.
    pub fn new_add() -> Self {
        Self {
            mode: SourceModalMode::Add,
            name_input: String::new(),
            path_input: String::new(),
            active_field: SourceModalField::Path,
            is_active_source: false,
            error_msg: None,
        }
    }

    /// Instantiates modal state for editing an existing media source.
    ///
    /// # Arguments
    /// * `index` - Index of the target media source in the sources collection.
    /// * `name` - Initial display name.
    /// * `path` - Initial filesystem path.
    /// * `is_active` - Whether the target source is currently enabled.
    ///
    /// # Returns
    /// Initialized [`SourceInputModalState`] in [`SourceModalMode::Edit`] mode.
    pub fn new_edit(index: usize, name: String, path: String, is_active: bool) -> Self {
        Self {
            mode: SourceModalMode::Edit(index),
            name_input: name,
            path_input: path,
            active_field: SourceModalField::Path,
            is_active_source: is_active,
            error_msg: None,
        }
    }

    /// Instantiates modal state for confirming deletion of a media source.
    ///
    /// # Arguments
    /// * `index` - Index of the target media source to delete.
    /// * `name` - Source display name.
    /// * `path` - Source filesystem path.
    /// * `is_active` - Whether the target source is currently enabled.
    ///
    /// # Returns
    /// Initialized [`SourceInputModalState`] in [`SourceModalMode::DeleteConfirm`] mode.
    pub fn new_delete(index: usize, name: String, path: String, is_active: bool) -> Self {
        Self {
            mode: SourceModalMode::DeleteConfirm(index),
            name_input: name,
            path_input: path,
            active_field: SourceModalField::Path,
            is_active_source: is_active,
            error_msg: None,
        }
    }
}

/// Computes descriptive statistics (min, max, mean, median, p90, stdev) for a series of values.
///
/// # Arguments
/// - `values`: Slice of 64-bit floating point numbers.
///
/// # Returns
/// - [`DimensionStats`] containing the statistical dispersion metrics.
/// - Returns default zeroed stats if `values` is empty.
///
/// # Data Flow & Invariants
/// 1. Clones and sorts input slice ascending using `partial_cmp`.
/// 2. Calculates min (`sorted[0]`), max (`sorted[n - 1]`), and mean (`sum / n`).
/// 3. Computes median: midpoint for odd counts, average of midpoints for even counts.
/// 4. Extracts P90 using [`P90_PERCENTILE_FACTOR`] (0.90 * n).
/// 5. Derives sample standard deviation with Bessel correction `(n - 1)`.
pub fn calculate_dimension_stats(values: &[f64]) -> DimensionStats {
    if values.is_empty() {
        return DimensionStats::default();
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();

    let min = sorted[0];
    let max = sorted[n - 1];
    let sum: f64 = sorted.iter().sum();
    let mean = sum / (n as f64);

    let median = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    };

    let p90_idx = ((n as f64 * P90_PERCENTILE_FACTOR) as usize).min(n - 1);
    let p90 = sorted[p90_idx];

    let stdev = if n > 1 {
        let variance: f64 =
            sorted.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / ((n - 1) as f64);
        variance.sqrt()
    } else {
        0.0
    };

    DimensionStats {
        min,
        median,
        mean,
        p90,
        max,
        stdev,
    }
}

/// Computes aggregated statistics across all successful full-length decoding records.
///
/// # Arguments
/// - `records`: Slice of historical execution records from batch runs.
///
/// # Returns
/// - [`OverallStats`] summarizing throughput, wall duration, size, and couplings count.
/// - Returns default zeroed stats if no successful records exceeding [`MIN_FULL_DECODE_DURATION_SECS`] exist.
///
/// # Data Flow & Invariants
/// Filters out cache re-validations and failures (`wall_duration_secs <= MIN_FULL_DECODE_DURATION_SECS`),
/// extracts dimension vectors, and computes separate [`DimensionStats`] for each metric.
pub fn calculate_overall_stats(records: &[HistoricalRecord]) -> OverallStats {
    let full_records: Vec<&HistoricalRecord> = records
        .iter()
        .filter(|r| r.success && r.wall_duration_secs > MIN_FULL_DECODE_DURATION_SECS)
        .collect();

    let sample_count = full_records.len();
    if sample_count == 0 {
        return OverallStats::default();
    }

    let rates: Vec<f64> = full_records
        .iter()
        .map(|r| r.cifs_read_rate_mib_s)
        .collect();
    let durations_m: Vec<f64> = full_records
        .iter()
        .map(|r| r.wall_duration_secs / (SECONDS_PER_MINUTE as f64))
        .collect();
    let sizes_gib: Vec<f64> = full_records
        .iter()
        .map(|r| r.file_size_bytes as f64 / BYTES_PER_GIB_F64)
        .collect();
    let couplings: Vec<f64> = full_records
        .iter()
        .map(|r| r.certified_couplings_count as f64)
        .collect();

    OverallStats {
        sample_count,
        rate: calculate_dimension_stats(&rates),
        duration_mins: calculate_dimension_stats(&durations_m),
        size_gib: calculate_dimension_stats(&sizes_gib),
        couplings: calculate_dimension_stats(&couplings),
    }
}

/// Parses an ISO 8601 string (e.g. `2026-10-25T22:36:56.415607`) to Unix epoch seconds.
///
/// # Arguments
/// - `iso_str`: Timestamp string in ISO-8601 calendar format.
///
/// # Returns
/// - `Some(f64)`: Fractional Unix epoch seconds if format is valid.
/// - `None`: If string is malformed, invalid length, or invalid calendar values.
///
/// # Data Flow & Invariants
/// Implements pure, dependency-free Gregorian calendar civil day projection
/// (Howard Hinnant algorithm) to compute exact Unix seconds without timezone shift.
pub fn parse_iso_to_unix_secs(iso_str: &str) -> Option<f64> {
    if iso_str.len() < 19 {
        return None;
    }
    let s = iso_str.as_bytes();
    if s[4] != b'-'
        || s[7] != b'-'
        || (s[10] != b'T' && s[10] != b' ')
        || s[13] != b':'
        || s[16] != b':'
    {
        return None;
    }
    let year: i32 = iso_str[0..4].parse().ok()?;
    let month: u32 = iso_str[5..7].parse().ok()?;
    let day: u32 = iso_str[8..10].parse().ok()?;
    let hour: u64 = iso_str[11..13].parse().ok()?;
    let min: u64 = iso_str[14..16].parse().ok()?;
    let sec: u64 = iso_str[17..19].parse().ok()?;

    if month == 0 || month > 12 || day == 0 || day > 31 || hour >= 24 || min >= 60 || sec >= 60 {
        return None;
    }

    let y = year - (month <= 2) as i32;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * CIVIL_DAYS_PER_400_YEARS + doe as i64 - CIVIL_EPOCH_DAYS_OFFSET;

    let total_secs = days * (SECONDS_PER_DAY as i64)
        + (hour * SECONDS_PER_HOUR + min * SECONDS_PER_MINUTE + sec) as i64;
    Some(total_secs as f64)
}

/// Formats Unix epoch seconds into an ISO 8601 string (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// # Arguments
/// - `unix_secs`: Unix timestamp in seconds.
///
/// # Returns
/// Formatted UTC ISO-8601 string.
///
/// # Data Flow & Invariants
/// Utilizes pure, safe Howard Hinnant algorithm to compute civil Gregorian date
/// without C FFI or architecture-dependent `time_t` assumptions.
pub fn format_unix_to_iso(unix_secs: u64) -> String {
    let secs_of_day = unix_secs % SECONDS_PER_DAY;
    let hour = secs_of_day / SECONDS_PER_HOUR;
    let min = (secs_of_day % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    let sec = secs_of_day % SECONDS_PER_MINUTE;

    let z = (unix_secs / SECONDS_PER_DAY) as i64 + CIVIL_EPOCH_DAYS_OFFSET;
    let era = if z >= 0 {
        z
    } else {
        z - (CIVIL_DAYS_PER_400_YEARS - 1)
    } / CIVIL_DAYS_PER_400_YEARS;
    let doe = (z - era * CIVIL_DAYS_PER_400_YEARS) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Formats duration in seconds into a human-readable string (e.g. `"12m34s"` or `"1h05m"`).
///
/// # Arguments
/// - `seconds`: Total duration in seconds.
///
/// # Returns
/// - Formatted concise string: `"Xs"` (< 60s), `"XmYYs"` (< 1h), or `"XhYYm"` (>= 1h).
/// - `"N/A"` if `seconds < 0.0` or exceeds sanity cap.
pub fn format_duration(seconds: f64) -> String {
    if seconds < 0.0 || seconds > MAX_SANITY_DURATION_SECS as f64 {
        return "N/A".to_string();
    }
    let total_secs = seconds as u64;
    let s = total_secs % SECONDS_PER_MINUTE;
    let m = (total_secs / SECONDS_PER_MINUTE) % SECONDS_PER_MINUTE;
    let h = total_secs / (SECONDS_PER_MINUTE * SECONDS_PER_MINUTE);

    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Formats progress ratio into a filled block visual string.
///
/// # Arguments
/// - `ratio`: Fractional progress from `0.0` to `1.0`.
/// - `width`: Character width of the progress bar display.
///
/// # Returns
/// Formatted string with filled unicode blocks (`"█"`), empty blocks (`"░"`), and percentage.
pub fn format_progress_bar(ratio: f64, width: usize) -> String {
    let clamped = ratio.clamp(0.0, MAX_PROGRESS_CAP);
    let filled = ((clamped * width as f64).round() as usize).min(width);
    let empty = width.saturating_sub(filled);
    let bar: String = "█".repeat(filled) + &"░".repeat(empty);
    format!("[{bar}] {:>5.1}%", clamped * 100.0)
}

/// Formats progress bar with default character width ([`PROGRESS_BAR_CHAR_WIDTH`]).
///
/// # Arguments
/// - `ratio`: Fractional progress from `0.0` to `1.0`.
///
/// # Returns
/// Formatted progress bar string.
pub fn default_progress_bar(ratio: f64) -> String {
    format_progress_bar(ratio, PROGRESS_BAR_CHAR_WIDTH)
}

/// Selection criteria for filtering media items by their source origin in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MediaSourceFilter {
    /// Show only media from sources marked active/enabled in Tab 5 (default).
    #[default]
    ActiveOnly,
    /// Show all media in the persistent catalog regardless of source active state.
    All,
    /// Show only media from a specific source ID.
    Specific(String),
}

impl MediaSourceFilter {
    /// Returns human-readable source filter label displayed in TUI header.
    ///
    /// # Arguments
    /// - `sources`: Slice of configured media sources for name resolution.
    ///
    /// # Returns
    /// Clean display string (e.g. `"ACTIVES"`, `"TOUTES"`, or resolved source name).
    pub fn label<'a>(&'a self, sources: &'a [MediaSourceConfig]) -> &'a str {
        match self {
            MediaSourceFilter::ActiveOnly => "ACTIVES",
            MediaSourceFilter::All => "TOUTES",
            MediaSourceFilter::Specific(id) => sources
                .iter()
                .find(|s| &s.id == id)
                .map(|s| {
                    let clean = s
                        .name
                        .trim_start_matches("Synology NAS (")
                        .trim_end_matches(')')
                        .trim();
                    if clean.is_empty() {
                        s.id.as_str()
                    } else {
                        clean
                    }
                })
                .unwrap_or(id.as_str()),
        }
    }
}

/// Filters and sorts a slice of discovered media items according to the active criteria.
///
/// # Arguments
/// - `items`: Slice of discovered candidate media items.
/// - `filter`: Filter predicate to apply.
/// - `sort`: Sorting order to apply.
///
/// # Returns
/// New filtered and sorted vector of media items.
pub fn filter_and_sort_media(
    items: &[DiscoveredMediaItem],
    filter: MediaFilter,
    sort: MediaSort,
) -> Vec<DiscoveredMediaItem> {
    filter_and_sort_media_with_source(items, filter, sort, &MediaSourceFilter::All, &[])
}

/// Filters active job telemetry items by search query across PID, media name, engine, status, and cmdline.
///
/// # Arguments
/// - `jobs`: Slice of active job snapshots.
/// - `search_query`: Optional interactive search filter text.
///
/// # Returns
/// Filtered vector of active job telemetry matching all search query tokens.
pub fn filter_active_jobs_with_search(
    jobs: &[ActiveJobTelemetry],
    search_query: Option<&str>,
) -> Vec<ActiveJobTelemetry> {
    let query = search_query.unwrap_or("").trim();
    if query.is_empty() {
        jobs.to_vec()
    } else {
        jobs.iter()
            .filter(|j| j.matches_search(query))
            .cloned()
            .collect()
    }
}

/// Filters and sorts a slice of discovered media items according to active criteria, source ID, and optional search query.
///
/// # Arguments
/// - `items`: Candidate media items in the persistent catalog.
/// - `filter`: Subtitle / lifecycle status filter predicate.
/// - `sort`: Sorting dimension (Name, Size, Duration, Status, Subtitles).
/// - `source_filter`: Active, All, or Specific source filter.
/// - `sources`: Configured source metadata slice for active state resolution.
/// - `search_query`: Optional search string for multi-token fuzzy matching.
///
/// # Returns
/// Cloned, filtered, and sorted vector of discovered media items.
///
/// # Data Flow & Invariants
/// 1. Collects active source IDs from `sources` when `source_filter == ActiveOnly`.
/// 2. Filters items based on source ID, subtitle status, and multi-token search query.
/// 3. Sorts resulting vector in-place by chosen dimension.
pub fn filter_and_sort_media_with_source_and_search(
    items: &[DiscoveredMediaItem],
    filter: MediaFilter,
    sort: MediaSort,
    source_filter: &MediaSourceFilter,
    sources: &[MediaSourceConfig],
    search_query: Option<&str>,
) -> Vec<DiscoveredMediaItem> {
    let query = search_query.unwrap_or("").trim();
    let active_source_ids: std::collections::HashSet<&str> = sources
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.id.as_str())
        .collect();

    let mut filtered: Vec<DiscoveredMediaItem> = items
        .iter()
        .filter(|item| {
            match source_filter {
                MediaSourceFilter::ActiveOnly => {
                    if !item
                        .source_id
                        .as_deref()
                        .is_none_or(|id| active_source_ids.contains(id))
                    {
                        return false;
                    }
                }
                MediaSourceFilter::All => {}
                MediaSourceFilter::Specific(ref expected_source) => {
                    if item.source_id.as_deref() != Some(expected_source.as_str()) {
                        return false;
                    }
                }
            }
            let matches_filter = match filter {
                MediaFilter::All => true,
                MediaFilter::Status(expected) => item.status == expected,
                MediaFilter::WithSubtitles => !item.subtitles.is_empty(),
                MediaFilter::WithoutSubtitles => item.subtitles.is_empty(),
            };
            if !matches_filter {
                return false;
            }

            if !query.is_empty() && !item.matches_search(query) {
                return false;
            }

            true
        })
        .cloned()
        .collect();

    match sort {
        MediaSort::Name => {
            filtered.sort_by_key(|a| a.stem.to_lowercase());
        }
        MediaSort::Size => {
            filtered.sort_by_key(|a| std::cmp::Reverse(a.size_bytes));
        }
        MediaSort::Duration => {
            filtered.sort_by(|a, b| {
                let dur_a = a.duration_secs.unwrap_or(0.0);
                let dur_b = b.duration_secs.unwrap_or(0.0);
                dur_b
                    .partial_cmp(&dur_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        MediaSort::Status => {
            filtered.sort_by_key(|a| match a.status {
                IngestionStatus::Virgin => 0,
                IngestionStatus::InProgress => 1,
                IngestionStatus::PartialFail => 2,
                IngestionStatus::Completed => 3,
            });
        }
        MediaSort::Subtitles => {
            filtered.sort_by_key(|a| std::cmp::Reverse(a.subtitles.len()));
        }
    }

    filtered
}

/// Filters and sorts a slice of discovered media items according to active criteria and optional source ID.
///
/// # Arguments
/// - `items`: Candidate media items in the persistent catalog.
/// - `filter`: Subtitle / lifecycle status filter predicate.
/// - `sort`: Sorting dimension.
/// - `source_filter`: Active, All, or Specific source filter.
/// - `sources`: Configured source metadata slice.
///
/// # Returns
/// Filtered and sorted vector without search query constraint.
pub fn filter_and_sort_media_with_source(
    items: &[DiscoveredMediaItem],
    filter: MediaFilter,
    sort: MediaSort,
    source_filter: &MediaSourceFilter,
    sources: &[MediaSourceConfig],
) -> Vec<DiscoveredMediaItem> {
    filter_and_sort_media_with_source_and_search(items, filter, sort, source_filter, sources, None)
}

/// Determines if a candidate text matches a search query using multi-token substring & normalized matching.
///
/// # Arguments
/// - `candidate`: The target text to search within (e.g. filename, PID, status, path).
/// - `query`: The user input query string.
///
/// # Returns
/// - `true` if every non-empty token in `query` matches `candidate` under one of the 3 rules.
/// - `true` if `query` is empty or only whitespace.
/// - `false` if any token fails to match.
///
/// # Matching Rules & Algorithmic Guarantee
/// Whitespace and punctuation (`.`, `_`, `-`) are treated as token separators.
/// All tokens must match (logical AND).
/// A token matches if:
/// 1. **Case-Insensitive Substring**: Candidate contains the token verbatim.
/// 2. **Normalized Alphanumeric**: Candidate stripped of punctuation contains token stripped of punctuation.
/// 3. **Acronym / Initials**: Initials of words in candidate contain the token (restricted to short tokens <= 3 chars).
pub fn matches_fuzzy_search(candidate: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let target = candidate.to_lowercase();
    let tokens: Vec<&str> = q
        .split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return true;
    }

    let norm_target: String = target.chars().filter(|c| c.is_alphanumeric()).collect();
    let words: Vec<&str> = target
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();

    tokens.iter().all(|token| {
        if target.contains(token) {
            return true;
        }

        // Normalized alphanumeric match: ignores '.', '_', '-', ' ' separators
        let norm_token: String = token.chars().filter(|c| c.is_alphanumeric()).collect();
        if !norm_token.is_empty() && norm_target.contains(&norm_token) {
            return true;
        }

        // Acronym match: initials of words contain token (for short tokens <= 3 chars)
        if token.len() <= 3 && !token.is_empty() && initials.contains(token) {
            return true;
        }

        false
    })
}

/// Filters and sorts historical completed records with an optional interactive search query.
///
/// # Arguments
/// - `records`: Historical execution records from batch execution logs.
/// - `filter`: Filter predicate (All, SuccessOnly, FailedOnly, NoSrtOnly, CancelledOnly).
/// - `sort`: Sorting order (DateDesc, DateAsc, Status, Name, Duration, Size, Rate, Couplings).
/// - `search_query`: Optional multi-token search filter query.
///
/// # Returns
/// Filtered and sorted vector of historical records.
///
/// # Data Flow & Invariants
/// Filters records by status and search match, then sorts in-place according to `sort`.
pub fn filter_and_sort_completed_with_search(
    records: &[HistoricalRecord],
    filter: CompletedFilter,
    sort: CompletedSort,
    search_query: Option<&str>,
) -> Vec<HistoricalRecord> {
    let query = search_query.unwrap_or("").trim();
    let mut filtered: Vec<HistoricalRecord> = records
        .iter()
        .filter(|r| filter.matches(r) && (query.is_empty() || r.matches_search(query)))
        .cloned()
        .collect();

    match sort {
        CompletedSort::DateDesc => {
            filtered.reverse();
        }
        CompletedSort::DateAsc => {}
        CompletedSort::Status => {
            filtered.sort_by_key(|r| match r.status() {
                CompletedStatus::Fail => 0,
                CompletedStatus::Cancelled => 1,
                CompletedStatus::NoSrt => 2,
                CompletedStatus::Ok => 3,
                CompletedStatus::Cache => 4,
            });
        }
        CompletedSort::Name => {
            filtered.sort_by_key(|r| r.video_stem.to_lowercase());
        }
        CompletedSort::Duration => {
            filtered.sort_by(|a, b| {
                b.wall_duration_secs
                    .partial_cmp(&a.wall_duration_secs)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        CompletedSort::Size => {
            filtered.sort_by_key(|r| std::cmp::Reverse(r.file_size_bytes));
        }
        CompletedSort::Rate => {
            filtered.sort_by(|a, b| {
                b.cifs_read_rate_mib_s
                    .partial_cmp(&a.cifs_read_rate_mib_s)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        CompletedSort::Couplings => {
            filtered.sort_by_key(|r| std::cmp::Reverse(r.certified_couplings_count));
        }
    }

    filtered
}

/// Filters and sorts historical completed records according to the active filter and sort criteria.
///
/// # Arguments
/// - `records`: Historical execution records from batch execution logs.
/// - `filter`: Filter predicate.
/// - `sort`: Sorting order.
///
/// # Returns
/// Filtered and sorted vector without search query constraint.
pub fn filter_and_sort_completed(
    records: &[HistoricalRecord],
    filter: CompletedFilter,
    sort: CompletedSort,
) -> Vec<HistoricalRecord> {
    filter_and_sort_completed_with_search(records, filter, sort, None)
}
