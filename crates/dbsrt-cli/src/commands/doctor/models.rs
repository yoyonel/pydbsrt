//! Serializable data models for `dbsrt doctor` multi-node observability reports.
//!
//! # Data Flow & Invariants
//! Defines deterministic, JSON-serializable structures representing per-node health, rolling
//! `1h`/`24h`/`72h` error counters, artifact freshness, and cross-node SHA-256 script parity.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Logical identifier for each cluster tier probed by `dbsrt doctor`.
///
/// # Data Flow & Invariants
/// Fixed 3-tier topology: `Local` developer workstation, `Rpi4` supervisor appliance, and
/// `O2switch` CloudLinux server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    /// Local developer workspace or custom `--local-root`.
    Local,
    /// Raspberry Pi 4 autonomous watchdog & Freebox orchestrator.
    Rpi4,
    /// CloudLinux o2switch production API & zero-NVMe worker host.
    O2switch,
}

impl NodeRole {
    /// Returns the canonical lowercase identifier for this node tier.
    ///
    /// # Arguments
    /// * `&self` - Node role variant.
    ///
    /// # Returns
    /// Static string `"local"`, `"rpi4"`, or `"o2switch"`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Used consistently across JSON reports and aligned CLI tables.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Rpi4 => "rpi4",
            Self::O2switch => "o2switch",
        }
    }
}

/// Operational health severity across nodes, alerts, and cluster roll-ups.
///
/// # Data Flow & Invariants
/// Ordered `Ok < Warning < Critical` so `Ord::max` computes monotonic roll-ups.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum DoctorSeverity {
    /// Healthy / nominal state with zero active warnings or errors.
    Ok,
    /// Degraded non-fatal condition (stale index, unreachable remote host, historical errors).
    Warning,
    /// Active critical error, aborted batch, degraded supervisor, or SHA-256 script drift.
    Critical,
}

impl DoctorSeverity {
    /// Uppercase badge string for aligned CLI table rendering.
    ///
    /// # Arguments
    /// * `&self` - Severity enum variant.
    ///
    /// # Returns
    /// `"OK"`, `"WARN"`, or `"CRIT"`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Pure constant mapping.
    pub const fn badge(&self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warning => "WARN",
            Self::Critical => "CRIT",
        }
    }
}

/// Cross-node SHA-256 verification outcome for a tracked deployment script.
///
/// # Data Flow & Invariants
/// Serializes `"SHA256_DRIFT"` verbatim when any reachable node diverges from `local` or misses
/// a mandatory deployed script.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParityStatus {
    /// All reachable nodes hold an identical SHA-256 digest to `local`.
    #[serde(rename = "OK")]
    Ok,
    /// Diverging SHA-256 digest or missing mandatory script detected across nodes.
    #[serde(rename = "SHA256_DRIFT")]
    Sha256Drift,
    /// Remote parity skipped (`--local-only` or all remote nodes unreachable) while `local` exists.
    #[serde(rename = "LOCAL_ONLY")]
    LocalOnly,
}

/// Rolling temporal occurrence counters for log error lines.
///
/// # Data Flow & Invariants
/// Satisfies `h1 <= h24 <= h72` for timestamped log entries; `tail_total` counts all error lines
/// (timestamped or un-timestamped) within the bounded log tail.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowErrorCounts {
    /// Errors within the last 1 hour (`<= 3,600s`).
    pub h1: usize,
    /// Errors within the last 24 hours (`<= 86,400s`).
    pub h24: usize,
    /// Errors within the last 72 hours (`<= 259,200s`).
    pub h72: usize,
    /// Total error lines observed in the bounded tail buffer.
    pub tail_total: usize,
}

/// Deduplicated error signature summary for a daemon or worker log stream.
///
/// # Data Flow & Invariants
/// `summary` is always redacted of FTP/API credentials prior to storage.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DedupErrorSignature {
    /// Redacted human-readable representative error line.
    pub summary: String,
    /// Active occurrence count (`h72` when timestamped, else `tail_total`).
    pub count: usize,
    /// ISO-8601 UTC timestamp of latest occurrence if parseable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
    /// True when the signature carries a `FATAL` or `🛑` marker or occurred within `1h`.
    pub critical: bool,
}

/// Per-service log error telemetry on a single node (`dbsrt-server`, `batch_worker`, `rpi4_supervisor`).
///
/// # Data Flow & Invariants
/// Retains up to `MAX_TOP_SIGNATURES_PER_COMPONENT` active deduplicated error signatures.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentLogReport {
    /// Service component name.
    pub component: String,
    /// Rolling temporal error counters.
    pub counts: WindowErrorCounts,
    /// Top active deduplicated error signatures.
    pub top_signatures: Vec<DedupErrorSignature>,
}

/// Existence and mtime freshness state of a tracked index or queue file on a node.
///
/// # Data Flow & Invariants
/// `stale` is true if and only if `exists` is true and `age_secs > ttl_secs`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ArtifactFreshness {
    /// Short artifact filename (`"nas_ftp_index.json"`, `"supervisor_status.json"`, etc.).
    pub name: String,
    /// True when the file is present on the node.
    pub exists: bool,
    /// Age in seconds since last modification when `exists` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_secs: Option<u64>,
    /// Configured staleness TTL threshold in seconds.
    pub ttl_secs: u64,
    /// True when `age_secs` exceeds `ttl_secs`.
    pub stale: bool,
}

/// Individual alert raised on a specific cluster node.
///
/// # Data Flow & Invariants
/// Any `NodeAlert` with `severity == DoctorSeverity::Critical` elevates the cluster exit code to `2`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NodeAlert {
    /// Stable alert identifier.
    pub id: String,
    /// Alert severity (`"warning"` or `"critical"`).
    pub severity: DoctorSeverity,
    /// Originating component on the node.
    pub component: String,
    /// Redacted human-readable description.
    pub summary: String,
}

/// Complete diagnostic snapshot for one probed node (`local`, `rpi4`, or `o2switch`).
///
/// # Data Flow & Invariants
/// Unreachable or `--local-only` skipped nodes populate `reachable = false` / `skipped = true`
/// without aborting cluster evaluation.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NodeProbeReport {
    /// Canonical node tier (`"local"`, `"rpi4"`, `"o2switch"`).
    pub node: String,
    /// Target host alias or local directory path inspected.
    pub target: String,
    /// True when the probe succeeded.
    pub reachable: bool,
    /// True when skipped via `--local-only`.
    pub skipped: bool,
    /// Wall-clock probe latency in milliseconds.
    pub probe_ms: u64,
    /// Roll-up node health status.
    pub status: DoctorSeverity,
    /// Optional transport/SSH error message when `reachable == false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
    /// Whether a live `dbsrt-server` process was observed on the node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_running: Option<bool>,
    /// Supervisor state from `run/supervisor_status.json` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_state: Option<String>,
    /// True when `run/batch_aborted` sentinel is present.
    pub batch_aborted: bool,
    /// Failed target count from `run/batch_queue.json`.
    pub failed_queue_targets: usize,
    /// Distinct `video_stem` count with `>= 3` consecutive failures.
    pub blacklisted_stems: usize,
    /// CloudLinux LVE throttling/fault entries observed in `logs/faultwatch/` within 24h.
    pub lve_faults_24h: usize,
    /// Per-component log error counters (`dbsrt-server`, `batch_worker`, `rpi4_supervisor`).
    pub components: Vec<ComponentLogReport>,
    /// Freshness status of index and queue state artifacts.
    pub freshness: Vec<ArtifactFreshness>,
    /// Active alerts detected on this node.
    pub alerts: Vec<NodeAlert>,
    /// Map of canonical script name -> lowercase 64-char SHA-256 hex digest.
    pub script_hashes: BTreeMap<String, String>,
}

/// Cross-node SHA-256 parity row for one tracked script.
///
/// # Data Flow & Invariants
/// Compares `rpi4_sha256` and `o2switch_sha256` against `local_sha256`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ScriptParityEntry {
    /// Canonical script name.
    pub script: String,
    /// SHA-256 hex digest on `local`.
    pub local_sha256: Option<String>,
    /// SHA-256 hex digest on `rpi4`.
    pub rpi4_sha256: Option<String>,
    /// SHA-256 hex digest on `o2switch`.
    pub o2switch_sha256: Option<String>,
    /// Parity evaluation status (`"OK"`, `"SHA256_DRIFT"`, or `"LOCAL_ONLY"`).
    pub status: ParityStatus,
    /// Diagnostic explanation when `status == ParityStatus::Sha256Drift` or `"LOCAL_ONLY"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Top-level multi-node cluster diagnostic report emitted by `dbsrt doctor`.
///
/// # Data Flow & Invariants
/// `exit_code` is `DOCTOR_EXIT_ALERT` (`2`) whenever `active_critical_count > 0` or
/// `sha256_drift_count > 0`; otherwise `DOCTOR_EXIT_OK` (`0`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClusterDoctorReport {
    /// Overall cluster health roll-up (`"ok"`, `"warning"`, `"critical"`).
    pub status: DoctorSeverity,
    /// Computed CLI exit code (`0` or `2`).
    pub exit_code: i32,
    /// ISO-8601 UTC timestamp of report generation.
    pub checked_at_iso: String,
    /// Total end-to-end parallel probe duration in milliseconds.
    pub elapsed_ms: u64,
    /// Total count of `Critical` alerts across all probed nodes.
    pub active_critical_count: usize,
    /// Total count of `Warning` alerts across all probed nodes.
    pub active_warning_count: usize,
    /// Count of scripts flagged with `SHA256_DRIFT`.
    pub sha256_drift_count: usize,
    /// Ordered per-node reports (`local`, `rpi4`, `o2switch`).
    pub nodes: Vec<NodeProbeReport>,
    /// Cross-node SHA-256 script parity matrix.
    pub script_parity: Vec<ScriptParityEntry>,
}
