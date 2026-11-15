//! Named operational constants, TTL thresholds, and tracked script registry for `dbsrt doctor`.
//!
//! # Data Flow & Invariants
//! Centralizes all byte bounds, timeout limits, exit codes, temporal unit factors, and cross-node
//! deployment script specifications so business logic contains zero magic literals.

/// Process exit code returned when the cluster is healthy or has only non-blocking warnings.
pub const DOCTOR_EXIT_OK: i32 = 0;

/// Process exit code returned when any `CRITICAL` / `ERROR` alert or `SHA256_DRIFT` is active.
pub const DOCTOR_EXIT_ALERT: i32 = 2;

/// Default SSH host alias for the Raspberry Pi 4 appliance node.
pub const DEFAULT_RPI4_HOST: &str = "rpi4";

/// Default SSH host alias for the CloudLinux o2switch production node.
pub const DEFAULT_O2SWITCH_HOST: &str = "o2switch";

/// Default relative directory under `$HOME` on remote nodes (`~/dbsrt`).
pub const DEFAULT_REMOTE_BASE_DIR: &str = "dbsrt";

/// Default per-node probe timeout in seconds.
pub const DEFAULT_DOCTOR_TIMEOUT_SECS: u64 = 5;

/// Minimum SSH `ConnectTimeout` value in seconds.
pub const MIN_SSH_CONNECT_TIMEOUT_SECS: u64 = 1;

/// Polling sleep interval in milliseconds while waiting on a bounded SSH child process.
pub const SSH_POLL_INTERVAL_MS: u64 = 20;

/// Maximum trailing byte window (64 KiB) inspected per daemon/worker log file during `doctor` probes.
pub const DOCTOR_LOG_TAIL_BYTES: u64 = 65_536;

/// Margin in bytes used to detect when a remote `tail -c` output began mid-file.
pub const TAIL_TRUNCATION_MARGIN_BYTES: usize = 64;

/// Maximum trailing byte window (256 KiB) inspected per NDJSON batch event file.
pub const DOCTOR_EVENTS_TAIL_BYTES: u64 = 262_144;

/// Maximum trailing byte window (128 KiB) inspected across `logs/faultwatch/*.jsonl` files.
pub const DOCTOR_FAULTWATCH_TAIL_BYTES: u64 = 131_072;

/// Number of trailing lines requested per remote faultwatch JSONL file.
pub const DOCTOR_FAULTWATCH_TAIL_LINES: usize = 120;

/// Consecutive failures per `video_stem` required to classify a media stem as blacklisted.
pub const BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD: usize = 3;

/// Maximum heartbeat age in seconds before `supervisor_status.json` is flagged stale.
pub const SUPERVISOR_STALE_TTL_SECS: u64 = 180;

/// Freshness TTL in seconds (24 hours) for `data/reports/nas_ftp_index.json`.
pub const NAS_FTP_INDEX_TTL_SECS: u64 = 86_400;

/// Freshness TTL in seconds (7 days) for `data/reports/freebox_batch_targets.json`.
pub const FREEBOX_TARGETS_TTL_SECS: u64 = 604_800;

/// Freshness TTL in seconds (24 hours) for `run/batch_queue.json`.
pub const BATCH_QUEUE_TTL_SECS: u64 = 86_400;

/// Rolling 1-hour error aggregation window in seconds.
pub const WINDOW_1H_SECS: u64 = 3_600;

/// Rolling 24-hour error aggregation window in seconds.
pub const WINDOW_24H_SECS: u64 = 86_400;

/// Rolling 72-hour error aggregation window in seconds.
pub const WINDOW_72H_SECS: u64 = 259_200;

/// Seconds per standard 24-hour day.
pub const SECONDS_PER_DAY: u64 = 86_400;

/// Seconds per hour.
pub const SECONDS_PER_HOUR: u64 = 3_600;

/// Seconds per minute.
pub const SECONDS_PER_MINUTE: u64 = 60;

/// Milliseconds per second factor (`1,000`).
pub const MILLIS_PER_SEC_U64: u64 = 1_000;

/// Scheduler run-queue delay threshold in milliseconds indicating CloudLinux LVE throttling.
pub const FAULTWATCH_RUN_DELAY_WARN_MS: f64 = 500.0;

/// Maximum Unicode scalar count kept per deduplicated error signature summary.
pub const MAX_SIGNATURE_SUMMARY_CHARS: usize = 160;

/// Maximum top error signatures retained per service component in `NodeProbeReport`.
pub const MAX_TOP_SIGNATURES_PER_COMPONENT: usize = 3;

/// Short hex prefix length rendered for SHA-256 digests in the human-readable table.
pub const SHA256_SHORT_HEX_LEN: usize = 12;

/// Expected hex length of a full SHA-256 digest.
pub const SHA256_FULL_HEX_LEN: usize = 64;

/// Days per 400-year Gregorian civil cycle (Howard Hinnant algorithm).
pub const CIVIL_DAYS_PER_400_YEARS: i64 = 146_097;

/// Days offset between Unix epoch (`1970-01-01`) and civil epoch (`0000-03-01`).
pub const CIVIL_EPOCH_DAYS_OFFSET: i64 = 719_468;

/// Specification of a critical operational script verified for SHA-256 parity across nodes.
///
/// # Data Flow & Invariants
/// Maps canonical logical names (`"o2switch_batch_worker.py"`, `"batch_worker/cli.py"`, etc.) to
/// both repository-nested local paths (`scripts/workers/...`, `scripts/infra/...`) and flat
/// deployed `~/dbsrt/scripts/...` paths.
#[derive(Debug, Clone, Copy)]
pub struct TrackedScriptSpec {
    /// Canonical logical script identifier displayed in `dbsrt doctor`.
    pub canonical_name: &'static str,
    /// Primary repository-relative path in a developer Git workspace.
    pub local_repo_path: &'static str,
    /// Deployed relative path under `~/dbsrt/` (also accepted as flat fallback in hermetic tests).
    pub deployed_rel_path: &'static str,
    /// True when the script MUST exist on `rpi4` when `rpi4` is reachable.
    pub required_on_rpi4: bool,
    /// True when the script MUST exist on `o2switch` when `o2switch` is reachable.
    pub required_on_o2switch: bool,
}

/// Registry of the 5 critical orchestration and worker scripts checked for cross-node SHA-256 drift.
pub const TRACKED_SCRIPTS: [TrackedScriptSpec; 5] = [
    TrackedScriptSpec {
        canonical_name: "o2switch_batch_worker.py",
        local_repo_path: "scripts/workers/o2switch_batch_worker.py",
        deployed_rel_path: "scripts/o2switch_batch_worker.py",
        required_on_rpi4: true,
        required_on_o2switch: true,
    },
    TrackedScriptSpec {
        canonical_name: "batch_worker/cli.py",
        local_repo_path: "scripts/workers/batch_worker/cli.py",
        deployed_rel_path: "scripts/batch_worker/cli.py",
        required_on_rpi4: true,
        required_on_o2switch: true,
    },
    TrackedScriptSpec {
        canonical_name: "o2switch_ctl.sh",
        local_repo_path: "scripts/infra/o2switch/o2switch_ctl.sh",
        deployed_rel_path: "scripts/o2switch_ctl.sh",
        required_on_rpi4: true,
        required_on_o2switch: false,
    },
    TrackedScriptSpec {
        canonical_name: "freebox_batch_ctl.sh",
        local_repo_path: "scripts/infra/rpi4/freebox_batch_ctl.sh",
        deployed_rel_path: "scripts/freebox_batch_ctl.sh",
        required_on_rpi4: true,
        required_on_o2switch: false,
    },
    TrackedScriptSpec {
        canonical_name: "rpi4_o2switch_supervisor.py",
        local_repo_path: "scripts/infra/rpi4/rpi4_o2switch_supervisor.py",
        deployed_rel_path: "scripts/rpi4_o2switch_supervisor.py",
        required_on_rpi4: true,
        required_on_o2switch: false,
    },
];

/// Relative paths of state and dataset index files checked for existence and mtime freshness.
pub const TRACKED_FRESHNESS_ARTIFACTS: [(&str, &str, u64); 4] = [
    (
        "nas_ftp_index.json",
        "data/reports/nas_ftp_index.json",
        NAS_FTP_INDEX_TTL_SECS,
    ),
    (
        "freebox_batch_targets.json",
        "data/reports/freebox_batch_targets.json",
        FREEBOX_TARGETS_TTL_SECS,
    ),
    (
        "supervisor_status.json",
        "run/supervisor_status.json",
        SUPERVISOR_STALE_TTL_SECS,
    ),
    (
        "batch_queue.json",
        "run/batch_queue.json",
        BATCH_QUEUE_TTL_SECS,
    ),
];
