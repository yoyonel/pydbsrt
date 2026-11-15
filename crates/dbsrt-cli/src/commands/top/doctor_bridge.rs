//! Asynchronous bridge between `dbsrt doctor` multi-node probes and `dbsrt top` TUI state.
//!
//! # Data Flow & Invariants
//! Executes `evaluate_cluster_doctor` strictly inside a dedicated background worker thread,
//! mapping [`ClusterDoctorReport`] into [`ClusterHealthTelemetry`] and updating
//! `Arc<RwLock<SystemTelemetry>>` without ever stalling the 20ms keyboard poll / render loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use dbsrt_tui::constants::{CLUSTER_PROBE_POLL_STEP_MS, CLUSTER_REFRESH_INTERVAL_SECS};
use dbsrt_tui::model::{
    ClusterAlertEntry, ClusterArtifactFreshness, ClusterComponentErrorSummary,
    ClusterErrorSignature, ClusterHealthTelemetry, ClusterNodeSnapshot, ClusterParityStatus,
    ClusterScriptParity, ClusterSeverity, SystemTelemetry,
};

use crate::commands::doctor::models::{
    ClusterDoctorReport, DoctorSeverity, NodeProbeReport, ParityStatus, ScriptParityEntry,
};
use crate::commands::doctor::{evaluate_cluster_doctor, DoctorArgs};

/// Synchronization primitives coalescing manual and periodic `dbsrt doctor` probes.
///
/// # Data Flow & Invariants
/// Ensures at most one cluster probe executes at any time (`in_flight`) and collapses any
/// manual `[a]` request arriving mid-probe into that active run rather than firing a duplicate pass.
#[derive(Debug, Default)]
pub struct ClusterProbeControl {
    /// Set by UI `[a]` key when no probe is currently active.
    pub refresh_requested: AtomicBool,
    /// `true` while [`evaluate_cluster_doctor`] is actively probing nodes.
    pub in_flight: AtomicBool,
}

impl ClusterProbeControl {
    /// Attempts to schedule a manual refresh; returns `false` if a probe is already in flight.
    ///
    /// # Arguments
    /// * `&self` - Shared probe control struct.
    ///
    /// # Returns
    /// `true` when queued for immediate wakeup, `false` when coalesced into an in-flight probe.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Lock-free atomic check avoids queuing duplicate back-to-back multi-node SSH probes.
    pub fn request_manual_refresh(&self) -> bool {
        if self.in_flight.load(Ordering::SeqCst) {
            return false;
        }
        self.refresh_requested.store(true, Ordering::SeqCst);
        true
    }
}

/// Spawns the non-blocking background cluster doctor probing thread for `dbsrt top`.
///
/// # Arguments
/// * `shared_state` - Thread-safe handle to live UI telemetry updated upon each probe completion.
/// * `stop_flag` - Shared atomic shutdown flag polled at [`CLUSTER_PROBE_POLL_STEP_MS`] granularity.
/// * `control` - Shared coalescing control (`in_flight` guard + manual wake-up flag).
/// * `local_only` - When `true` (e.g., on CloudLinux shared LVE hosting), skips outbound SSH probes.
///
/// # Returns
/// `()` — detaches the worker thread cleanly; thread exits promptly when `stop_flag` turns `true`.
///
/// # Errors
/// Infallible; probe transport failures are captured inside per-node `UNREACH` statuses.
///
/// # Panics
/// Guaranteed panic-free (handles poisoned locks defensively via `if let Ok(...)`).
///
/// # Data Flow & Invariants
/// 1. Runs an initial cluster probe immediately upon TUI launch.
/// 2. Coalesces any `[a]` refresh pressed during an active probe into that running pass.
/// 3. Sleeps in bounded [`CLUSTER_PROBE_POLL_STEP_MS`] slices up to [`CLUSTER_REFRESH_INTERVAL_SECS`].
pub fn start_cluster_doctor_probe(
    shared_state: Arc<RwLock<SystemTelemetry>>,
    stop_flag: Arc<AtomicBool>,
    control: Arc<ClusterProbeControl>,
    local_only: bool,
) {
    std::thread::spawn(move || {
        run_cluster_doctor_probe_loop(&shared_state, &stop_flag, &control, local_only);
    });
}

/// Executes periodic and manual-trigger `dbsrt doctor` probes until `stop_flag` turns `true`.
fn run_cluster_doctor_probe_loop(
    shared_state: &Arc<RwLock<SystemTelemetry>>,
    stop_flag: &Arc<AtomicBool>,
    control: &ClusterProbeControl,
    local_only: bool,
) {
    let refresh_interval = Duration::from_secs(CLUSTER_REFRESH_INTERVAL_SECS);
    let poll_step = Duration::from_millis(CLUSTER_PROBE_POLL_STEP_MS);

    while !stop_flag.load(Ordering::Relaxed) {
        control.in_flight.store(true, Ordering::SeqCst);
        mark_cluster_refreshing(shared_state, true);
        let args = DoctorArgs {
            local_only,
            ..Default::default()
        };
        let report = evaluate_cluster_doctor(&args);
        let snapshot = cluster_telemetry_from_report(&report, local_only, false);
        if let Ok(mut state) = shared_state.write() {
            state.cluster = Some(snapshot);
        }
        control.refresh_requested.store(false, Ordering::SeqCst);
        control.in_flight.store(false, Ordering::SeqCst);

        wait_until_next_cluster_probe(refresh_interval, poll_step, stop_flag, control);
    }
}

/// Sleeps up to `refresh_interval` in interruptible slices, returning early on shutdown or refresh.
fn wait_until_next_cluster_probe(
    refresh_interval: Duration,
    poll_step: Duration,
    stop_flag: &AtomicBool,
    control: &ClusterProbeControl,
) {
    let wait_started = Instant::now();
    while wait_started.elapsed() < refresh_interval {
        if stop_flag.load(Ordering::Relaxed)
            || control.refresh_requested.swap(false, Ordering::SeqCst)
        {
            break;
        }
        std::thread::sleep(poll_step);
    }
}

/// Marks existing cluster telemetry as refreshing without clearing previously displayed tables.
fn mark_cluster_refreshing(shared_state: &Arc<RwLock<SystemTelemetry>>, refreshing: bool) {
    if let Ok(mut state) = shared_state.write() {
        if let Some(cluster) = state.cluster.as_mut() {
            cluster.refreshing = refreshing;
        }
    }
}

/// Converts a [`ClusterDoctorReport`] produced by `dbsrt doctor` into UI-ready [`ClusterHealthTelemetry`].
///
/// # Arguments
/// * `report` - Evaluated multi-node report returned by [`evaluate_cluster_doctor`].
/// * `local_only` - Whether remote SSH checks were skipped during this probe pass.
/// * `refreshing` - Whether an asynchronous refresh is currently active.
///
/// # Returns
/// Owned [`ClusterHealthTelemetry`] preserving all nodes, parity digests, freshness, and alerts.
///
/// # Errors
/// Infallible mapping.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Combines parity `SHA256_DRIFT` entries and per-node `alerts` into a unified sorted list,
/// and aggregates rolling `1h`/`24h`/`72h` error counts per node across all reported components.
pub fn cluster_telemetry_from_report(
    report: &ClusterDoctorReport,
    local_only: bool,
    refreshing: bool,
) -> ClusterHealthTelemetry {
    let nodes = report.nodes.iter().map(convert_node_report).collect();
    let script_parity = report
        .script_parity
        .iter()
        .map(convert_parity_entry)
        .collect();
    let alerts = collect_cluster_alerts(&report.nodes, &report.script_parity);

    ClusterHealthTelemetry {
        status: convert_severity(report.status),
        exit_code: report.exit_code,
        checked_at_iso: report.checked_at_iso.clone(),
        elapsed_ms: report.elapsed_ms,
        active_critical_count: report.active_critical_count,
        active_warning_count: report.active_warning_count,
        sha256_drift_count: report.sha256_drift_count,
        local_only_probe: local_only,
        refreshing,
        nodes,
        script_parity,
        alerts,
    }
}

/// Maps CLI [`DoctorSeverity`] onto decoupled `dbsrt-tui` [`ClusterSeverity`].
const fn convert_severity(sev: DoctorSeverity) -> ClusterSeverity {
    match sev {
        DoctorSeverity::Ok => ClusterSeverity::Ok,
        DoctorSeverity::Warning => ClusterSeverity::Warning,
        DoctorSeverity::Critical => ClusterSeverity::Critical,
    }
}

/// Maps a single [`NodeProbeReport`] including summed rolling error counters and component tails.
fn convert_node_report(n: &NodeProbeReport) -> ClusterNodeSnapshot {
    let (mut err_h1, mut err_h24, mut err_h72, mut err_tail_total) = (0, 0, 0, 0);
    let mut components = Vec::with_capacity(n.components.len());

    for comp in &n.components {
        err_h1 += comp.counts.h1;
        err_h24 += comp.counts.h24;
        err_h72 += comp.counts.h72;
        err_tail_total += comp.counts.tail_total;

        let top_signatures = comp
            .top_signatures
            .iter()
            .map(|sig| ClusterErrorSignature {
                summary: sig.summary.clone(),
                count: sig.count,
                last_seen: sig.last_seen.clone(),
                critical: sig.critical,
            })
            .collect();

        components.push(ClusterComponentErrorSummary {
            component: comp.component.clone(),
            h1: comp.counts.h1,
            h24: comp.counts.h24,
            h72: comp.counts.h72,
            tail_total: comp.counts.tail_total,
            top_signatures,
        });
    }

    let freshness = n
        .freshness
        .iter()
        .map(|f| ClusterArtifactFreshness {
            name: f.name.clone(),
            exists: f.exists,
            age_secs: f.age_secs,
            ttl_secs: f.ttl_secs,
            stale: f.stale,
        })
        .collect();

    ClusterNodeSnapshot {
        node: n.node.clone(),
        target: n.target.clone(),
        reachable: n.reachable,
        skipped: n.skipped,
        probe_ms: n.probe_ms,
        status: convert_severity(n.status),
        probe_error: n.probe_error.clone(),
        daemon_running: n.daemon_running,
        supervisor_state: n.supervisor_state.clone(),
        batch_aborted: n.batch_aborted,
        failed_queue_targets: n.failed_queue_targets,
        blacklisted_stems: n.blacklisted_stems,
        lve_faults_24h: n.lve_faults_24h,
        err_h1,
        err_h24,
        err_h72,
        err_tail_total,
        components,
        freshness,
    }
}

/// Converts a single [`ScriptParityEntry`] onto UI parity model.
fn convert_parity_entry(p: &ScriptParityEntry) -> ClusterScriptParity {
    ClusterScriptParity {
        script: p.script.clone(),
        local_sha256: p.local_sha256.clone(),
        rpi4_sha256: p.rpi4_sha256.clone(),
        o2switch_sha256: p.o2switch_sha256.clone(),
        status: match p.status {
            ParityStatus::Ok => ClusterParityStatus::Ok,
            ParityStatus::Sha256Drift => ClusterParityStatus::Sha256Drift,
            ParityStatus::LocalOnly => ClusterParityStatus::LocalOnly,
        },
        detail: p.detail.clone(),
    }
}

/// Flattens SHA-256 script drifts and node alerts into a single list sorted critical-first.
fn collect_cluster_alerts(
    nodes: &[NodeProbeReport],
    parity: &[ScriptParityEntry],
) -> Vec<ClusterAlertEntry> {
    let mut alerts = Vec::new();
    for p in parity
        .iter()
        .filter(|entry| entry.status == ParityStatus::Sha256Drift)
    {
        let detail = p.detail.as_deref().unwrap_or("diverging SHA-256");
        alerts.push(ClusterAlertEntry {
            node: "parity".to_string(),
            severity: ClusterSeverity::Critical,
            component: p.script.clone(),
            summary: format!("SHA256_DRIFT: {detail}"),
        });
    }
    for n in nodes {
        for a in &n.alerts {
            alerts.push(ClusterAlertEntry {
                node: n.node.clone(),
                severity: convert_severity(a.severity),
                component: a.component.clone(),
                summary: a.summary.clone(),
            });
        }
    }
    alerts.sort_by_key(|a| std::cmp::Reverse(a.severity));
    alerts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_cluster_doctor_probe_populates_and_stops_cleanly() {
        let shared = Arc::new(RwLock::new(SystemTelemetry::default()));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let control = Arc::new(ClusterProbeControl::default());

        start_cluster_doctor_probe(
            Arc::clone(&shared),
            Arc::clone(&stop_flag),
            Arc::clone(&control),
            true,
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if shared.read().is_ok_and(|s| s.cluster.is_some()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let has_cluster = shared.read().unwrap().cluster.is_some();
        assert!(
            has_cluster,
            "background probe must populate local cluster telemetry"
        );
        mark_cluster_refreshing(&shared, true);
        assert!(shared.read().unwrap().cluster.as_ref().unwrap().refreshing);
        stop_flag.store(true, Ordering::Relaxed);
    }

    #[test]
    fn test_cluster_probe_control_coalesces_when_in_flight() {
        let control = ClusterProbeControl::default();
        control.in_flight.store(true, Ordering::SeqCst);
        assert!(!control.request_manual_refresh());
        assert!(!control.refresh_requested.load(Ordering::SeqCst));

        control.in_flight.store(false, Ordering::SeqCst);
        assert!(control.request_manual_refresh());
        assert!(control.refresh_requested.load(Ordering::SeqCst));
    }
}
