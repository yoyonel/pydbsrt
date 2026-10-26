//! Data models and application state types for the TUI monitor.

use serde::Serialize;

use crate::commands::top::constants::{
    MAX_PROGRESS_CAP, MAX_SANITY_DURATION_SECS, PROGRESS_BAR_CHAR_WIDTH, SECONDS_PER_MINUTE,
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

/// Full-frame view page state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewPage {
    /// Full-frame interactive table of all active decoding/pHash jobs.
    #[default]
    ActiveJobs,
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
