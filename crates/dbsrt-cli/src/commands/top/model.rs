//! Data models and application state types for the TUI monitor.

use serde::{Deserialize, Serialize};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, CIVIL_DAYS_PER_400_YEARS, CIVIL_EPOCH_DAYS_OFFSET, MAX_PROGRESS_CAP,
    MAX_SANITY_DURATION_SECS, MIN_FULL_DECODE_DURATION_SECS, P90_PERCENTILE_FACTOR,
    PROGRESS_BAR_CHAR_WIDTH, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoricalRecord {
    pub video_stem: String,
    pub success: bool,
    pub wall_duration_secs: f64,
    pub cifs_read_rate_mib_s: f64,
    pub file_size_bytes: u64,
    pub certified_couplings_count: usize,
    pub start_iso: Option<String>,
    pub end_iso: Option<String>,
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
    /// Full-frame inspection view of selected task (cmdline, I/O, threads, memory, logs).
    JobInspection,
    /// Full-frame keybinding and operational cheat sheet.
    Help,
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
