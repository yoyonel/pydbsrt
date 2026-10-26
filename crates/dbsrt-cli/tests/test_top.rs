//! Integration and unit tests for the native Rust TUI top process monitor.

use dbsrt_cli::commands::top::constants::*;
use dbsrt_cli::commands::top::model::{
    default_progress_bar, format_duration, format_progress_bar, ActiveJobTelemetry, GpuTelemetry,
    SystemTelemetry, ViewPage,
};

#[test]
fn test_format_duration_boundaries() {
    assert_eq!(format_duration(0.0), "0s");
    assert_eq!(format_duration(45.0), "45s");
    assert_eq!(format_duration(60.0), "1m00s");
    assert_eq!(format_duration(75.0), "1m15s");
    assert_eq!(format_duration(3600.0), "1h00m");
    assert_eq!(format_duration(3665.0), "1h01m");
    assert_eq!(format_duration(7200.0), "2h00m");
    assert_eq!(format_duration(-5.0), "N/A");
    assert_eq!(format_duration(10_000_000.0), "N/A");
}

#[test]
fn test_format_progress_bar_clamping() {
    let empty_bar = format_progress_bar(0.0, 10);
    assert_eq!(empty_bar, "[░░░░░░░░░░]   0.0%");

    let half_bar = format_progress_bar(0.5, 10);
    assert_eq!(half_bar, "[█████░░░░░]  50.0%");

    let full_bar = format_progress_bar(1.0, 10);
    assert_eq!(full_bar, "[██████████] 100.0%");

    let over_bar = format_progress_bar(1.5, 10);
    assert_eq!(over_bar, "[██████████] 100.0%");

    let under_bar = format_progress_bar(-0.5, 10);
    assert_eq!(under_bar, "[░░░░░░░░░░]   0.0%");
}

#[test]
fn test_default_progress_bar_length() {
    let bar = default_progress_bar(0.75);
    assert!(bar.starts_with('['));
    assert!(bar.ends_with('%'));
    assert!(bar.contains("75.0%"));
}

#[test]
fn test_gpu_telemetry_defaults() {
    let gpu = GpuTelemetry::default();
    assert!(!gpu.available);
    assert_eq!(gpu.video_busy_pct, 0.0);
    assert_eq!(gpu.render_busy_pct, 0.0);
    assert_eq!(gpu.freq_mhz, 0.0);
    assert_eq!(gpu.power_watts, 0.0);
}

#[test]
fn test_active_job_telemetry_instantiation() {
    let job = ActiveJobTelemetry {
        pid: 12345,
        media_name: "Silicon.Valley.S01E01.mkv".to_string(),
        engine: "GPU (VA-API)".to_string(),
        elapsed_secs: 42.5,
        read_bytes: 500 * BYTES_PER_MIB,
        total_bytes: 1000 * BYTES_PER_MIB,
        progress_ratio: 0.5,
        current_rate_mib_s: 25.4,
        eta_secs: Some(19.6),
        cpu_percent: 18.5,
        ram_rss_mib: 64.0,
        status: "RUNNING".to_string(),
        cmdline: vec![
            "ffmpeg".to_string(),
            "-hwaccel".to_string(),
            "vaapi".to_string(),
        ],
    };

    assert_eq!(job.pid, 12345);
    assert_eq!(job.media_name, "Silicon.Valley.S01E01.mkv");
    assert_eq!(job.engine, "GPU (VA-API)");
    assert_eq!(job.progress_ratio, 0.5);
    assert_eq!(job.status, "RUNNING");
    assert_eq!(job.eta_secs, Some(19.6));
}

#[test]
fn test_view_page_transitions() {
    assert_eq!(ViewPage::default(), ViewPage::ActiveJobs);
    let mut page = ViewPage::ActiveJobs;
    assert_eq!(page, ViewPage::ActiveJobs);
    page = ViewPage::JobInspection;
    assert_eq!(page, ViewPage::JobInspection);
    page = ViewPage::Help;
    assert_eq!(page, ViewPage::Help);
}

#[test]
fn test_system_telemetry_aggregation() {
    let mut sys = SystemTelemetry::default();
    assert!(sys.active_jobs.is_empty());
    assert_eq!(sys.total_rate_mib_s, 0.0);

    sys.active_jobs.push(ActiveJobTelemetry {
        pid: 101,
        media_name: "Test1".to_string(),
        engine: "GPU (VA-API)".to_string(),
        elapsed_secs: 10.0,
        read_bytes: 100,
        total_bytes: 200,
        progress_ratio: 0.5,
        current_rate_mib_s: 15.0,
        eta_secs: None,
        cpu_percent: 10.0,
        ram_rss_mib: 30.0,
        status: "RUNNING".to_string(),
        cmdline: vec![],
    });

    sys.active_jobs.push(ActiveJobTelemetry {
        pid: 102,
        media_name: "Test2".to_string(),
        engine: "CPU (Software)".to_string(),
        elapsed_secs: 5.0,
        read_bytes: 50,
        total_bytes: 200,
        progress_ratio: 0.25,
        current_rate_mib_s: 20.0,
        eta_secs: None,
        cpu_percent: 45.0,
        ram_rss_mib: 40.0,
        status: "RUNNING".to_string(),
        cmdline: vec![],
    });

    sys.total_rate_mib_s = sys.active_jobs.iter().map(|j| j.current_rate_mib_s).sum();
    assert_eq!(sys.active_jobs.len(), 2);
    assert_eq!(sys.total_rate_mib_s, 35.0);
}
