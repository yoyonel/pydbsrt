//! Real-time centralized monitoring alerts aggregator (`GET /v1/monitoring/alerts`).
//!
//! # Data Flow & Invariants
//! Aggregates local daemon, batch worker, RPi4 supervisor, CloudLinux LVE faultwatch, NDJSON
//! media blacklists, and dataset freshness into a single structured [`MonitoringAlertsResponse`].
//! Supports hermetic testing via [`MonitoringAlertsConfig::root_dir`] without touching production
//! workspace files.

pub mod log_scan;
pub mod sources;

use crate::constants::{
    DEFAULT_ALT_BATCH_EVENTS_PATH, DEFAULT_BATCH_ABORTED_PATH, DEFAULT_BATCH_EVENTS_PATH,
    DEFAULT_BATCH_QUEUE_PATH, DEFAULT_BATCH_WORKER_LOG_PATH, DEFAULT_FAULTWATCH_DIR,
    DEFAULT_NAS_FTP_INDEX_PATH, DEFAULT_SERVER_LOG_PATH, DEFAULT_SUPERVISOR_LOG_PATH,
    DEFAULT_SUPERVISOR_STATUS_PATH,
};
use crate::dtos::{
    AlertHealthStatus, AlertSeverity, MonitoringAlertItem, MonitoringAlertsMetrics,
    MonitoringAlertsResponse,
};
use crate::handlers::alerts::log_scan::scan_log_file_alerts;
use crate::handlers::alerts::sources::{
    check_batch_aborted_alert, check_batch_queue_alert, check_blacklisted_media_alert,
    check_faultwatch_alert, check_nas_index_staleness_alert, check_supervisor_alerts,
};
use crate::handlers::monitoring::{format_unix_to_iso, resolve_monitoring_path};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for collecting centralized monitoring alerts on a node.
///
/// # Data Flow & Invariants
/// When `root_dir` is `Some(root)`, all relative state/log paths resolve strictly under `root`.
/// When `root_dir` is `None`, paths resolve via [`resolve_monitoring_path`].
#[derive(Debug, Clone)]
pub struct MonitoringAlertsConfig {
    /// Optional isolated filesystem root directory (used by hermetic tests and custom deployments).
    pub root_dir: Option<PathBuf>,
    /// Reference wall-clock Unix epoch timestamp in seconds.
    pub now_unix_secs: u64,
}

impl Default for MonitoringAlertsConfig {
    /// Creates a default [`MonitoringAlertsConfig`] anchored to current system UTC time.
    ///
    /// # Returns
    /// Initialized [`MonitoringAlertsConfig`] with `root_dir: None`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Samples `SystemTime::now()` once so all window calculations in a single collection pass
    /// share a consistent timestamp.
    fn default() -> Self {
        let now_unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            root_dir: None,
            now_unix_secs,
        }
    }
}

impl MonitoringAlertsConfig {
    /// Builds a [`MonitoringAlertsConfig`] from [`AppState`] (`monitoring_root` override if set).
    ///
    /// # Arguments
    /// * `state` - Shared server [`AppState`].
    ///
    /// # Returns
    /// Configured [`MonitoringAlertsConfig`].
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Propagates `state.monitoring_root` directly so Axum integration tests remain 100% hermetic.
    pub fn from_state(state: &AppState) -> Self {
        Self {
            root_dir: state.monitoring_root.clone(),
            ..Self::default()
        }
    }

    /// Resolves a relative monitoring file/directory path against `self.root_dir` or default search locations.
    ///
    /// # Arguments
    /// * `rel` - Relative path under the `dbsrt` runtime layout (e.g. `"run/batch_aborted"`).
    ///
    /// # Returns
    /// Resolved [`PathBuf`].
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Never escapes `root_dir` when `root_dir` is `Some`.
    pub fn resolve(&self, rel: &str) -> PathBuf {
        match &self.root_dir {
            Some(root) => root.join(rel),
            None => resolve_monitoring_path(rel),
        }
    }
}

/// Collects and deduplicates all active monitoring alerts for the configured node root.
///
/// # Arguments
/// * `config` - [`MonitoringAlertsConfig`] specifying root path resolution and reference clock.
///
/// # Returns
/// Populated [`MonitoringAlertsResponse`] with roll-up status, sorted alerts, and numeric metrics.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. Evaluates supervisor status, fast-failure `batch_aborted` sentinel, and `batch_queue.json`.
/// 2. Scans bounded tails of `batch_worker.log`, `dbsrt-server.log`, and `rpi4_supervisor.log`.
/// 3. Computes consecutive failure streaks (`>= 3`) across NDJSON batch event logs.
/// 4. Evaluates `logs/faultwatch/` LVE throttling and `nas_ftp_index.json` staleness.
/// 5. Sorts alerts critical-first and derives monotonic roll-up `status`.
pub fn collect_monitoring_alerts(config: &MonitoringAlertsConfig) -> MonitoringAlertsResponse {
    let now = config.now_unix_secs;
    let mut alerts: Vec<MonitoringAlertItem> = Vec::new();

    let sup_path = config.resolve(DEFAULT_SUPERVISOR_STATUS_PATH);
    let (sup_state, sup_launch_errs, sup_age, sup_alerts) = check_supervisor_alerts(&sup_path, now);
    alerts.extend(sup_alerts);

    let abort_path = config.resolve(DEFAULT_BATCH_ABORTED_PATH);
    let abort_alert = check_batch_aborted_alert(&abort_path);
    let batch_aborted = abort_alert.is_some();
    if let Some(a) = abort_alert {
        alerts.push(a);
    }

    let queue_path = config.resolve(DEFAULT_BATCH_QUEUE_PATH);
    let (failed_queue_targets, queue_alert) = check_batch_queue_alert(&queue_path);
    if let Some(a) = queue_alert {
        alerts.push(a);
    }

    let (log_sigs, log_1h, log_24h, log_72h) = collect_all_log_alerts(config, &mut alerts);

    let alt_events = config.resolve(DEFAULT_ALT_BATCH_EVENTS_PATH);
    let main_events = config.resolve(DEFAULT_BATCH_EVENTS_PATH);
    let event_refs: [&Path; 2] = [alt_events.as_path(), main_events.as_path()];
    let (blacklisted_media_count, bl_alert) = check_blacklisted_media_alert(&event_refs);
    if let Some(a) = bl_alert {
        alerts.push(a);
    }

    let fw_dir = config.resolve(DEFAULT_FAULTWATCH_DIR);
    let (lve_fault_entries_24h, fw_alert) = check_faultwatch_alert(&fw_dir, now);
    if let Some(a) = fw_alert {
        alerts.push(a);
    }

    let nas_idx = config.resolve(DEFAULT_NAS_FTP_INDEX_PATH);
    let (nas_ftp_index_age_secs, nas_alert) = check_nas_index_staleness_alert(&nas_idx, now);
    if let Some(a) = nas_alert {
        alerts.push(a);
    }

    sort_alerts_by_severity(&mut alerts);
    let status = derive_overall_status(&alerts);
    let active_alerts_count = alerts.len();

    MonitoringAlertsResponse {
        status,
        active_alerts_count,
        alerts,
        metrics: MonitoringAlertsMetrics {
            supervisor_state: sup_state,
            supervisor_launch_errors: sup_launch_errs,
            supervisor_heartbeat_age_secs: sup_age,
            batch_aborted,
            failed_queue_targets,
            blacklisted_media_count,
            log_error_signatures: log_sigs,
            log_errors_1h: log_1h,
            log_errors_24h: log_24h,
            log_errors_72h: log_72h,
            lve_fault_entries_24h,
            nas_ftp_index_age_secs,
        },
        server_time_iso: format_unix_to_iso(now),
    }
}

/// Scans `batch_worker.log`, `dbsrt-server.log`, and `rpi4_supervisor.log` and appends alerts.
fn collect_all_log_alerts(
    config: &MonitoringAlertsConfig,
    alerts: &mut Vec<MonitoringAlertItem>,
) -> (usize, usize, usize, usize) {
    let sources = [
        (DEFAULT_BATCH_WORKER_LOG_PATH, "batch_worker"),
        (DEFAULT_SERVER_LOG_PATH, "dbsrt-server"),
        (DEFAULT_SUPERVISOR_LOG_PATH, "supervisor"),
    ];
    let mut sig_count = 0usize;
    let mut h1 = 0usize;
    let mut h24 = 0usize;
    let mut h72 = 0usize;

    for (rel_path, component) in sources {
        let path = config.resolve(rel_path);
        let outcome = scan_log_file_alerts(&path, component, config.now_unix_secs);
        sig_count += outcome.alerts.len();
        h1 += outcome.totals.h1;
        h24 += outcome.totals.h24;
        h72 += outcome.totals.h72;
        alerts.extend(outcome.alerts);
    }
    (sig_count, h1, h24, h72)
}

/// Orders alerts critical-first, then by descending count and deterministic alert ID.
fn sort_alerts_by_severity(alerts: &mut [MonitoringAlertItem]) {
    alerts.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Computes roll-up [`AlertHealthStatus`] from active alerts.
fn derive_overall_status(alerts: &[MonitoringAlertItem]) -> AlertHealthStatus {
    if alerts.iter().any(|a| a.severity == AlertSeverity::Critical) {
        AlertHealthStatus::Critical
    } else if !alerts.is_empty() {
        AlertHealthStatus::Warning
    } else {
        AlertHealthStatus::Ok
    }
}

/// Builds an HTTP header map containing `X-Server-Time-Ms` in Unix epoch milliseconds.
fn alerts_server_time_header() -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
        if let Ok(val) = axum::http::HeaderValue::from_str(&duration.as_millis().to_string()) {
            headers.insert(axum::http::HeaderName::from_static("x-server-time-ms"), val);
        }
    }
    headers
}

/// Serves real-time centralized monitoring alerts (`GET /v1/monitoring/alerts`).
///
/// # Arguments
/// * `state` - Shared [`AppState`] reference.
///
/// # Returns
/// Tuple containing `X-Server-Time-Ms` response header and [`Json<MonitoringAlertsResponse>`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Runs bounded tail reads via `tokio::task::spawn_blocking` so disk seeks never stall async workers.
pub async fn monitoring_alerts_handler(
    State(state): State<Arc<AppState>>,
) -> (axum::http::HeaderMap, Json<MonitoringAlertsResponse>) {
    let cfg = MonitoringAlertsConfig::from_state(&state);
    let response = tokio::task::spawn_blocking(move || collect_monitoring_alerts(&cfg))
        .await
        .unwrap_or_else(|_| collect_monitoring_alerts(&MonitoringAlertsConfig::from_state(&state)));
    (alerts_server_time_header(), Json(response))
}
