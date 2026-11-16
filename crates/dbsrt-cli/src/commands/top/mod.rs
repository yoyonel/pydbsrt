//! Native TUI process & real-time pHash decoding monitor (dbsrt top).

pub mod app;
pub mod constants;
pub mod data_access_bridge;
pub mod doctor_bridge;
pub mod host_info;
pub mod model;
pub mod process_ctl;
pub mod sampler;
pub mod ui;
pub mod views;

use anyhow::Result;
use clap::Args;

pub use app::run_top_app;
pub use constants::DEFAULT_REFRESH_INTERVAL_MS;
pub use host_info::HostSystemInfo;
pub use model::{ActiveJobTelemetry, GpuTelemetry, MediaSourceConfig, SystemTelemetry, ViewPage};

/// Command line arguments for `dbsrt top`.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct TopArgs {
    /// Telemetry sampling & UI refresh interval in milliseconds.
    #[arg(long, default_value_t = DEFAULT_REFRESH_INTERVAL_MS)]
    pub refresh_ms: u64,

    /// Output a single telemetry snapshot as JSON and exit (non-interactive).
    #[arg(long)]
    pub json: bool,
}

/// Samples the system over 2 sampling intervals to produce an accurate non-interactive JSON telemetry snapshot.
pub fn probe_telemetry_snapshot(refresh_ms: u64) -> Result<SystemTelemetry> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, RwLock};

    let interval = refresh_ms.clamp(
        constants::PROBE_MIN_INTERVAL_MS,
        constants::PROBE_MAX_INTERVAL_MS,
    );
    let mut initial_tel = SystemTelemetry::default();
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
    sys.refresh_memory_specifics(sysinfo::MemoryRefreshKind::everything());
    initial_tel.host_info = host_info::detect_host_system_info_with_sys(&sys);
    initial_tel.ram_total_gib = sys.total_memory() as f64 / constants::BYTES_PER_GIB_F64;
    initial_tel.ram_used_gib = sys.used_memory() as f64 / constants::BYTES_PER_GIB_F64;

    let sources_path = std::path::PathBuf::from(constants::DEFAULT_MEDIA_SOURCES_PATH);
    initial_tel.media_sources = sampler::load_media_sources(&sources_path);
    let cifs_safe = data_access_bridge::cached_cifs_safe_to_stat().unwrap_or(true);
    let has_local_sources = cifs_safe
        && initial_tel
            .media_sources
            .iter()
            .any(|s| s.enabled && s.path.exists());
    initial_tel.is_remote_streaming = !has_local_sources
        && std::path::Path::new(constants::DEFAULT_SELECTED_BATCH_TARGETS_PATH).exists();

    let doctor_report =
        crate::commands::doctor::evaluate_cluster_doctor(&crate::commands::doctor::DoctorArgs {
            local_only: true,
            ..Default::default()
        });
    initial_tel.cluster = Some(doctor_bridge::cluster_telemetry_from_report(
        &doctor_report,
        true,
        false,
    ));

    let shared_state = Arc::new(RwLock::new(initial_tel));
    let gpu_state = Arc::new(RwLock::new(GpuTelemetry::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let shared = Arc::clone(&shared_state);
    let gpu = Arc::clone(&gpu_state);
    let stop = Arc::clone(&stop_flag);

    let handle = std::thread::spawn(move || {
        sampler::run_telemetry_sampler_loop(shared, gpu, interval, stop);
    });

    // Wait for 2 completed sampling intervals (accounting for tick-0 /proc warmup) for non-zero delta rates
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(interval * 2 + constants::PROBE_WARMUP_ALLOWANCE_MS);
    std::thread::sleep(std::time::Duration::from_millis(interval * 2));
    while std::time::Instant::now() < deadline {
        let ready = shared_state.read().is_ok_and(|s| {
            !s.active_jobs.is_empty() && s.active_jobs.iter().any(|j| j.current_rate_mib_s > 0.0)
        });
        if ready {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(
            constants::PROBE_POLL_STEP_MS,
        ));
    }
    stop_flag.store(true, Ordering::Relaxed);
    let _ = handle.join();

    let snap = shared_state.read().unwrap().clone();
    Ok(snap)
}

/// Runs the native Rust interactive TUI monitor or outputs a single JSON probe.
pub fn run_top(args: &TopArgs) -> Result<()> {
    if args.json {
        let snapshot = probe_telemetry_snapshot(args.refresh_ms)?;
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        Ok(())
    } else {
        run_top_app(args.refresh_ms)
    }
}
