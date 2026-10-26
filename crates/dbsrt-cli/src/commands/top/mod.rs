//! Native TUI process & real-time pHash decoding monitor (dbsrt top).

pub mod app;
pub mod constants;
pub mod model;
pub mod process_ctl;
pub mod sampler;
pub mod ui;

use anyhow::Result;
use clap::Args;

pub use app::run_top_app;
pub use constants::DEFAULT_REFRESH_INTERVAL_MS;
pub use model::{ActiveJobTelemetry, GpuTelemetry, SystemTelemetry, ViewPage};

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

    let interval = refresh_ms.clamp(200, 1000);
    let shared_state = Arc::new(RwLock::new(SystemTelemetry::default()));
    let gpu_state = Arc::new(RwLock::new(GpuTelemetry::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let shared = Arc::clone(&shared_state);
    let gpu = Arc::clone(&gpu_state);
    let stop = Arc::clone(&stop_flag);

    let handle = std::thread::spawn(move || {
        sampler::run_telemetry_sampler_loop(shared, gpu, interval, stop);
    });

    // Sleep for 2 sampling intervals + margin to guarantee delta rate calculation
    std::thread::sleep(std::time::Duration::from_millis(interval * 2 + 50));
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
