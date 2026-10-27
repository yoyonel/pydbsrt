//! Data models and application state types for the TUI monitor.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, CIVIL_DAYS_PER_400_YEARS, CIVIL_EPOCH_DAYS_OFFSET, MAX_PROGRESS_CAP,
    MAX_SANITY_DURATION_SECS, MIN_FULL_DECODE_DURATION_SECS, MIN_WALL_DURATION_NON_CACHE_SECS,
    P90_PERCENTILE_FACTOR, PROGRESS_BAR_CHAR_WIDTH, SECONDS_PER_DAY, SECONDS_PER_HOUR,
    SECONDS_PER_MINUTE,
};

/// Hardware GPU telemetry sampled from intel_gpu_top.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GpuTelemetry {
    pub video_busy_pct: f64,
    pub render_busy_pct: f64,
    pub freq_mhz: f64,
    pub power_watts: f64,
    pub available: bool,
}

/// Telemetry metrics for a single active decoding or pHash job process.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActiveJobTelemetry {
    pub pid: u32,
    pub media_name: String,
    pub engine: String,
    #[serde(default)]
    pub engine_reason: Option<String>,
    pub elapsed_secs: f64,
    pub read_bytes: u64,
    pub total_bytes: u64,
    pub progress_ratio: f64,
    pub current_rate_mib_s: f64,
    pub eta_secs: Option<f64>,
    pub cpu_percent: f32,
    pub ram_rss_mib: f64,
    pub status: String,
    pub cmdline: Vec<String>,
}

/// Historical completed media decode and extraction event record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HistoricalRecord {
    pub video_stem: String,
    #[serde(default)]
    pub video_rel_path: Option<String>,
    pub success: bool,
    pub wall_duration_secs: f64,
    pub cifs_read_rate_mib_s: f64,
    pub file_size_bytes: u64,
    pub certified_couplings_count: usize,
    #[serde(default)]
    pub error_summary: Option<String>,
    pub start_iso: Option<String>,
    pub end_iso: Option<String>,
}

/// Qualified execution status of a historical batch record.
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
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Ok => "✓ OK",
            Self::Cache => "✓ CACHE",
            Self::NoSrt => "⚠ NO_SRT",
            Self::Cancelled => "⏹ CANCEL",
            Self::Fail => "✗ FAIL",
        }
    }

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
}

/// Filtering predicate applied in the completed media view.
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
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::SuccessOnly,
            Self::SuccessOnly => Self::FailedOnly,
            Self::FailedOnly => Self::NoSrtOnly,
            Self::NoSrtOnly => Self::CancelledOnly,
            Self::CancelledOnly => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "TOUS",
            Self::SuccessOnly => "✓ SUCCÈS (OK/CACHE)",
            Self::FailedOnly => "✗ FAIL (ERREURS SEULES)",
            Self::NoSrtOnly => "⚠ SANS SOUS-TITRES (NO_SRT)",
            Self::CancelledOnly => "⏹ ANNULÉS (CANCEL)",
        }
    }

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

/// Sorting criteria applied in the completed media view.
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

/// Ingestion and certification lifecycle status of a media release.
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

    pub fn label(&self) -> &'static str {
        match self {
            Self::Virgin => "VIERGE",
            Self::InProgress => "EN COURS",
            Self::Completed => "COMPLÉTÉ",
            Self::PartialFail => "HACHÉ SEUL",
        }
    }
}

/// Information about precomputed or existing pHash visual fingerprint on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PhashFingerprintInfo {
    pub exists: bool,
    pub frame_count: usize,
    pub size_bytes: u64,
}

/// Metadata for an embedded or sidecar subtitle track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSubtrackInfo {
    pub index: usize,
    pub lang: String,
    pub codec: String,
    pub title: String,
}

/// Persistent configuration and operational state for a media storage source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MediaSourceConfig {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub enabled: bool,
    #[serde(default)]
    pub media_count: usize,
    #[serde(default)]
    pub last_scan_added: usize,
    #[serde(default)]
    pub last_scan_iso: Option<String>,
}

/// Detailed metadata for a candidate video release discovered in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredMediaItem {
    pub path: PathBuf,
    pub stem: String,
    pub size_bytes: u64,
    pub duration_secs: Option<f64>,
    pub video_codec: Option<String>,
    pub subtitles: Vec<MediaSubtrackInfo>,
    #[serde(default)]
    pub phash: Option<PhashFingerprintInfo>,
    #[serde(default)]
    pub source_id: Option<String>,
    pub status: IngestionStatus,
}

/// Filtering predicate applied in the media browser view.
/// Structurally and mathematically guaranteed 100% aligned with IngestionStatus
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
    /// Ordered list of all filter variants for cycler [f].
    pub fn all_variants() -> Vec<Self> {
        let mut variants = vec![Self::All];
        for &status in IngestionStatus::ALL {
            variants.push(Self::Status(status));
        }
        variants.push(Self::WithSubtitles);
        variants.push(Self::WithoutSubtitles);
        variants
    }

    pub fn next(&self) -> Self {
        let variants = Self::all_variants();
        let idx = variants.iter().position(|v| v == self).unwrap_or(0);
        let next_idx = (idx + 1) % variants.len();
        variants[next_idx]
    }

    pub fn target_status(&self) -> Option<IngestionStatus> {
        match self {
            Self::Status(s) => Some(*s),
            _ => None,
        }
    }

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

/// Sorting order criterion for the media browser.
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
    pub fn next(&self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Duration,
            Self::Duration => Self::Status,
            Self::Status => Self::Subtitles,
            Self::Subtitles => Self::Name,
        }
    }

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

/// Descriptive statistics across a single performance dimension.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DimensionStats {
    pub min: f64,
    pub median: f64,
    pub mean: f64,
    pub p90: f64,
    pub max: f64,
    pub stdev: f64,
}

/// Statistical summary across all decoded media dimensions.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct OverallStats {
    pub sample_count: usize,
    pub rate: DimensionStats,
    pub duration_mins: DimensionStats,
    pub size_gib: DimensionStats,
    pub couplings: DimensionStats,
}

/// Cumulative throughput and volume across the active user session.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SessionThroughput {
    pub session_bytes: u64,
    pub session_duration_secs: f64,
    pub session_rate_mib_s: f64,
    pub stream_count: usize,
}

/// Full-frame view page state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewPage {
    /// Full-frame interactive table of all active decoding/pHash jobs.
    #[default]
    ActiveJobs,
    /// Full-frame statistical distribution table of historical decoding metrics.
    StatsHistory,
    /// Full-frame scrollable log of all certified and completed media.
    CompletedMedia,
    /// Full-frame interactive browser of available media files on storage source.
    MediaBrowser,
    /// Full-frame manager for configured media library sources (add, edit, scan, delete).
    MediaSources,
    /// Full-frame inspection view of selected task (cmdline, I/O, threads, memory, logs).
    JobInspection,
    /// Full-frame inspection and diagnostic view of candidate media release.
    MediaInspection,
    /// Full-frame inspection and diagnostic view of completed/failed media execution.
    CompletedInspection,
    /// Full-frame keybinding and operational cheat sheet.
    Help,
}

/// Selected tab within the fullframe contextual Help & Glossary view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTab {
    #[default]
    General,
    ActiveJobs,
    StatsHistory,
    Completed,
    MediaBrowser,
    MediaSources,
}

impl HelpTab {
    pub const ALL: [HelpTab; 6] = [
        HelpTab::General,
        HelpTab::ActiveJobs,
        HelpTab::StatsHistory,
        HelpTab::Completed,
        HelpTab::MediaBrowser,
        HelpTab::MediaSources,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            HelpTab::General => "[0/g] Général",
            HelpTab::ActiveJobs => "[1] Tâches",
            HelpTab::StatsHistory => "[2/h] Stats",
            HelpTab::Completed => "[3/r] Récents",
            HelpTab::MediaBrowser => "[4/m] Médias",
            HelpTab::MediaSources => "[5/s] Sources",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            HelpTab::General => HelpTab::ActiveJobs,
            HelpTab::ActiveJobs => HelpTab::StatsHistory,
            HelpTab::StatsHistory => HelpTab::Completed,
            HelpTab::Completed => HelpTab::MediaBrowser,
            HelpTab::MediaBrowser => HelpTab::MediaSources,
            HelpTab::MediaSources => HelpTab::General,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            HelpTab::General => HelpTab::MediaSources,
            HelpTab::ActiveJobs => HelpTab::General,
            HelpTab::StatsHistory => HelpTab::ActiveJobs,
            HelpTab::Completed => HelpTab::StatsHistory,
            HelpTab::MediaBrowser => HelpTab::Completed,
            HelpTab::MediaSources => HelpTab::MediaBrowser,
        }
    }

    pub fn from_view(view: ViewPage) -> Self {
        match view {
            ViewPage::ActiveJobs | ViewPage::JobInspection => HelpTab::ActiveJobs,
            ViewPage::StatsHistory => HelpTab::StatsHistory,
            ViewPage::CompletedMedia | ViewPage::CompletedInspection => HelpTab::Completed,
            ViewPage::MediaBrowser | ViewPage::MediaInspection => HelpTab::MediaBrowser,
            ViewPage::MediaSources => HelpTab::MediaSources,
            ViewPage::Help => HelpTab::General,
        }
    }
}

/// System-wide combined telemetry snapshot.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SystemTelemetry {
    pub host_cpu_pct: f32,
    pub ram_used_gib: f64,
    pub ram_total_gib: f64,
    pub gpu: GpuTelemetry,
    pub active_jobs: Vec<ActiveJobTelemetry>,
    pub total_rate_mib_s: f64,
    pub net_rx_mib_s: f64,
    pub session: SessionThroughput,
    pub history_records: Vec<HistoricalRecord>,
    pub stats: OverallStats,
    pub discovered_media: Vec<DiscoveredMediaItem>,
    pub media_sources: Vec<MediaSourceConfig>,
}

/// Active field focus within the source configuration modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceModalField {
    Name,
    #[default]
    Path,
}

/// Operation mode for source configuration dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceModalMode {
    Add,
    Edit(usize),
}

/// State of the interactive modal dialog for adding or editing a media source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInputModalState {
    pub mode: SourceModalMode,
    pub name_input: String,
    pub path_input: String,
    pub active_field: SourceModalField,
}

/// Computes descriptive statistics (min, max, mean, median, p90, stdev) for a series of values.
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
pub fn format_unix_to_iso(unix_secs: u64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time = unix_secs as libc::time_t;
    unsafe {
        libc::gmtime_r(&time, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Formats duration in seconds into a human-readable string (e.g. "12m34s" or "1h05m").
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
pub fn format_progress_bar(ratio: f64, width: usize) -> String {
    let clamped = ratio.clamp(0.0, MAX_PROGRESS_CAP);
    let filled = ((clamped * width as f64).round() as usize).min(width);
    let empty = width.saturating_sub(filled);
    let bar: String = "█".repeat(filled) + &"░".repeat(empty);
    format!("[{bar}] {:>5.1}%", clamped * 100.0)
}

/// Formats progress bar with default width.
pub fn default_progress_bar(ratio: f64) -> String {
    format_progress_bar(ratio, PROGRESS_BAR_CHAR_WIDTH)
}

/// Filters and sorts a slice of discovered media items according to the active criteria.
pub fn filter_and_sort_media(
    items: &[DiscoveredMediaItem],
    filter: MediaFilter,
    sort: MediaSort,
) -> Vec<DiscoveredMediaItem> {
    filter_and_sort_media_with_source(items, filter, sort, None)
}

/// Filters and sorts a slice of discovered media items according to active criteria and optional source ID.
pub fn filter_and_sort_media_with_source(
    items: &[DiscoveredMediaItem],
    filter: MediaFilter,
    sort: MediaSort,
    source_id_filter: Option<&str>,
) -> Vec<DiscoveredMediaItem> {
    let mut filtered: Vec<DiscoveredMediaItem> = items
        .iter()
        .filter(|item| {
            if let Some(expected_source) = source_id_filter {
                if item.source_id.as_deref() != Some(expected_source) {
                    return false;
                }
            }
            match filter {
                MediaFilter::All => true,
                MediaFilter::Status(expected) => item.status == expected,
                MediaFilter::WithSubtitles => !item.subtitles.is_empty(),
                MediaFilter::WithoutSubtitles => item.subtitles.is_empty(),
            }
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

/// Filters and sorts historical completed records according to the active filter and sort criteria.
pub fn filter_and_sort_completed(
    records: &[HistoricalRecord],
    filter: CompletedFilter,
    sort: CompletedSort,
) -> Vec<HistoricalRecord> {
    let mut filtered: Vec<HistoricalRecord> = records
        .iter()
        .filter(|r| filter.matches(r))
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
