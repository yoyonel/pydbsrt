//! Unified multi-node cluster diagnostic command (`dbsrt doctor`).
//!
//! # Data Flow & Invariants
//! Probes `local`, `rpi4`, and `o2switch` concurrently via `std::thread::scope`, aggregates
//! rolling `1h`/`24h`/`72h` error counters, evaluates cross-node SHA-256 script parity, checks
//! dataset/queue freshness, renders human or `--json` output, and returns exit code `2`
//! ([`DOCTOR_EXIT_ALERT`]) when any `CRITICAL` alert or `SHA256_DRIFT` is active (`0` otherwise).

pub mod analyzer;
pub mod constants;
pub mod models;
pub mod parity;
pub mod probe;
pub mod render;

pub use constants::{DOCTOR_EXIT_ALERT, DOCTOR_EXIT_OK};
pub use models::{
    ArtifactFreshness, ClusterDoctorReport, ComponentLogReport, DoctorSeverity, NodeAlert,
    NodeProbeReport, NodeRole, ParityStatus, ScriptParityEntry, WindowErrorCounts,
};

use crate::commands::doctor::analyzer::format_unix_iso;
use crate::commands::doctor::constants::{
    DEFAULT_DOCTOR_TIMEOUT_SECS, DEFAULT_O2SWITCH_HOST, DEFAULT_REMOTE_BASE_DIR, DEFAULT_RPI4_HOST,
};
use crate::commands::doctor::parity::evaluate_script_parity;
use crate::commands::doctor::probe::{probe_node, NodeProbeTarget};
use crate::commands::doctor::render::render_human_report;
use anyhow::Result;
use clap::Args;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// CLI arguments for `dbsrt doctor`.
///
/// # Data Flow & Invariants
/// Parsed by `clap` under `dbsrt doctor`. Hidden `--*-root` overrides enable 100% hermetic
/// subprocess integration testing without network or SSH dependencies.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct DoctorArgs {
    /// Emit structured JSON report instead of aligned human-readable tables.
    #[arg(long)]
    pub json: bool,

    /// Probe only the local workspace and skip SSH checks to `rpi4` and `o2switch`.
    #[arg(long)]
    pub local_only: bool,

    /// SSH host alias or `user@host` for the Raspberry Pi 4 supervisor appliance.
    #[arg(long, default_value = DEFAULT_RPI4_HOST)]
    pub rpi4_host: String,

    /// SSH host alias or `user@host` for the CloudLinux o2switch node.
    #[arg(long, default_value = DEFAULT_O2SWITCH_HOST)]
    pub o2switch_host: String,

    /// Per-node probe timeout in seconds (`>= 1`).
    #[arg(long, default_value_t = DEFAULT_DOCTOR_TIMEOUT_SECS)]
    pub timeout_secs: u64,

    /// Optional local workspace root override (used for hermetic CLI tests).
    #[arg(long, hide = true)]
    pub local_root: Option<PathBuf>,

    /// Optional local directory stand-in for `rpi4` (used for hermetic multi-node parity tests).
    #[arg(long, hide = true)]
    pub rpi4_local_root: Option<PathBuf>,

    /// Optional local directory stand-in for `o2switch` (used for hermetic multi-node parity tests).
    #[arg(long, hide = true)]
    pub o2switch_local_root: Option<PathBuf>,
}

impl Default for DoctorArgs {
    /// Returns default [`DoctorArgs`] probing `local`, `rpi4`, and `o2switch` with a 5s timeout.
    ///
    /// # Returns
    /// Default [`DoctorArgs`].
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Pure constant initialization.
    fn default() -> Self {
        Self {
            json: false,
            local_only: false,
            rpi4_host: DEFAULT_RPI4_HOST.to_string(),
            o2switch_host: DEFAULT_O2SWITCH_HOST.to_string(),
            timeout_secs: DEFAULT_DOCTOR_TIMEOUT_SECS,
            local_root: None,
            rpi4_local_root: None,
            o2switch_local_root: None,
        }
    }
}

/// Evaluates the 3-node cluster state according to `args` without writing to stdout.
///
/// # Arguments
/// * `args` - [`DoctorArgs`] specifying targets, `--local-only`, and timeout bounds.
///
/// # Returns
/// Fully populated [`ClusterDoctorReport`] including computed `exit_code` (`0` or `2`).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Executes `local`, `rpi4`, and `o2switch` probes concurrently inside `std::thread::scope` so
/// total latency equals `max(node_latencies)` (`< 3s` in nominal SSH operation).
pub fn evaluate_cluster_doctor(args: &DoctorArgs) -> ClusterDoctorReport {
    let t0 = Instant::now();
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let (local_t, rpi4_t, o2_t) = build_probe_targets(args);
    let timeout = args.timeout_secs.max(1);

    let (local_rep, rpi4_rep, o2_rep) = std::thread::scope(|s| {
        let h_loc = s.spawn(|| probe_node(NodeRole::Local, &local_t, timeout, now_secs));
        let h_rpi = s.spawn(|| probe_node(NodeRole::Rpi4, &rpi4_t, timeout, now_secs));
        let h_o2 = s.spawn(|| probe_node(NodeRole::O2switch, &o2_t, timeout, now_secs));
        (
            h_loc.join().expect("local probe thread"),
            h_rpi.join().expect("rpi4 probe thread"),
            h_o2.join().expect("o2switch probe thread"),
        )
    });

    let script_parity = evaluate_script_parity(&local_rep, &rpi4_rep, &o2_rep);
    let sha256_drift_count = script_parity
        .iter()
        .filter(|p| p.status == ParityStatus::Sha256Drift)
        .count();

    let nodes = vec![local_rep, rpi4_rep, o2_rep];
    let (crit_count, warn_count) = count_cluster_alerts(&nodes);

    let status = if crit_count > 0 || sha256_drift_count > 0 {
        DoctorSeverity::Critical
    } else if warn_count > 0 {
        DoctorSeverity::Warning
    } else {
        DoctorSeverity::Ok
    };
    let exit_code = if status == DoctorSeverity::Critical {
        DOCTOR_EXIT_ALERT
    } else {
        DOCTOR_EXIT_OK
    };

    ClusterDoctorReport {
        status,
        exit_code,
        checked_at_iso: format_unix_iso(now_secs),
        elapsed_ms: t0.elapsed().as_millis() as u64,
        active_critical_count: crit_count,
        active_warning_count: warn_count,
        sha256_drift_count,
        nodes,
        script_parity,
    }
}

/// Resolves [`NodeProbeTarget`]s for `local`, `rpi4`, and `o2switch` from [`DoctorArgs`].
fn build_probe_targets(args: &DoctorArgs) -> (NodeProbeTarget, NodeProbeTarget, NodeProbeTarget) {
    let local_dir = args
        .local_root
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let local_t = NodeProbeTarget::LocalDir(local_dir);

    if args.local_only {
        return (
            local_t,
            NodeProbeTarget::Skipped(args.rpi4_host.clone()),
            NodeProbeTarget::Skipped(args.o2switch_host.clone()),
        );
    }

    let rpi4_t = match &args.rpi4_local_root {
        Some(p) => NodeProbeTarget::LocalDir(p.clone()),
        None => NodeProbeTarget::SshHost {
            host: args.rpi4_host.clone(),
            remote_base: DEFAULT_REMOTE_BASE_DIR.to_string(),
        },
    };
    let o2_t = match &args.o2switch_local_root {
        Some(p) => NodeProbeTarget::LocalDir(p.clone()),
        None => NodeProbeTarget::SshHost {
            host: args.o2switch_host.clone(),
            remote_base: DEFAULT_REMOTE_BASE_DIR.to_string(),
        },
    };
    (local_t, rpi4_t, o2_t)
}

/// Sums `(critical_alerts, warning_alerts)` across all node reports.
fn count_cluster_alerts(nodes: &[NodeProbeReport]) -> (usize, usize) {
    let all_alerts = nodes.iter().flat_map(|n| n.alerts.iter());
    let mut crit = 0usize;
    let mut warn = 0usize;
    for alert in all_alerts {
        if alert.severity == DoctorSeverity::Critical {
            crit += 1;
        } else if alert.severity == DoctorSeverity::Warning {
            warn += 1;
        }
    }
    (crit, warn)
}

/// Executes `dbsrt doctor`, prints the formatted or `--json` report to stdout, and returns the exit code.
///
/// # Arguments
/// * `args` - Parsed [`DoctorArgs`].
///
/// # Returns
/// `Ok(exit_code)` where `exit_code` is [`DOCTOR_EXIT_OK`] (`0`) or [`DOCTOR_EXIT_ALERT`] (`2`).
///
/// # Errors
/// Returns an error only if JSON serialization fails.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Calls [`evaluate_cluster_doctor`], prints to stdout, and leaves `std::process::exit` to `main.rs`
/// so library callers and integration tests can inspect the returned exit code directly.
pub fn run_doctor(args: &DoctorArgs) -> Result<i32> {
    let report = evaluate_cluster_doctor(args);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_human_report(&report));
    }
    Ok(report.exit_code)
}
