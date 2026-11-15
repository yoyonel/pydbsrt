//! Hermetic node snapshot analyzer for `dbsrt doctor`.
//!
//! # Data Flow & Invariants
//! Converts a transport-agnostic [`RawNodeSnapshot`] (captured either from local disk or a single
//! bounded SSH round-trip) into a structured [`NodeProbeReport`] with rolling `1h`/`24h`/`72h`
//! error counts, secret-redacted deduplicated log signatures, blacklisted stem streaks (`>= 3`),
//! CloudLinux LVE faultwatch checks, and artifact freshness evaluation.

use crate::commands::doctor::constants::{
    BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD, CIVIL_DAYS_PER_400_YEARS, CIVIL_EPOCH_DAYS_OFFSET,
    FAULTWATCH_RUN_DELAY_WARN_MS, MAX_SIGNATURE_SUMMARY_CHARS, MAX_TOP_SIGNATURES_PER_COMPONENT,
    SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE, SUPERVISOR_STALE_TTL_SECS,
    TRACKED_FRESHNESS_ARTIFACTS, WINDOW_1H_SECS, WINDOW_24H_SECS, WINDOW_72H_SECS,
};
use crate::commands::doctor::models::{
    ArtifactFreshness, ComponentLogReport, DedupErrorSignature, DoctorSeverity, NodeAlert,
    NodeProbeReport, NodeRole, WindowErrorCounts,
};
use std::collections::BTreeMap;

/// ASCII byte length of a `YYYY-MM-DD HH:MM:SS` timestamp prefix.
const TS_PREFIX_BYTES: usize = 19;

/// Minimum plausible Gregorian year for log timestamps.
const MIN_LOG_YEAR: i64 = 1970;

/// Maximum plausible Gregorian year for log timestamps.
const MAX_LOG_YEAR: i64 = 2200;

/// Maximum valid calendar month (`1 ..= 12`).
const MAX_MONTH: u32 = 12;

/// Maximum valid calendar day (`1 ..= 31`).
const MAX_DAY: u32 = 31;

/// Maximum valid 24h hour (`0 ..= 23`).
const MAX_HOUR: u32 = 23;

/// Maximum valid minute/second (`0 ..= 59`).
const MAX_MIN_SEC: u32 = 59;

/// Civil calendar March pivot month (`2`).
const CIVIL_MARCH_PIVOT: u32 = 2;

/// Years per 400-year Gregorian leap cycle.
const YEARS_PER_ERA: i64 = 400;

/// Days per common year.
const DAYS_PER_COMMON_YEAR: u32 = 365;

/// Years per leap cycle.
const LEAP_STEP_YEARS: u32 = 4;

/// Years per century.
const CENTURY_STEP_YEARS: u32 = 100;

/// Month offset for March..December (`month - 3`).
const MARCH_SHIFT_HIGH: u32 = 3;

/// Month offset for January..February (`month + 9`).
const MARCH_SHIFT_LOW: u32 = 9;

/// Linear day-of-year month factor (`153`).
const DOY_FACTOR: u32 = 153;

/// Linear day-of-year bias (`2`).
const DOY_BIAS: u32 = 2;

/// Linear day-of-year divisor (`5`).
const DOY_DIV: u32 = 5;

/// Minute-level ISO prefix length before trailing `'Z'` (`YYYY-MM-DDTHH:MM`).
const MINUTE_ISO_STEM_LEN: usize = 16;

/// Banner prefix marking the start of a new `batch_worker` execution session in `batch_worker.log`.
const BATCH_SESSION_START_MARKER: &str = "=== [1/";

/// Transport-agnostic raw files, hashes, mtimes, and log tails collected from one node.
///
/// # Data Flow & Invariants
/// Populated identically by local filesystem reads (`probe_directory`) and remote SSH probes
/// (`probe_ssh_host`), guaranteeing identical alert and parity semantics across all 3 tiers.
#[derive(Debug, Clone, Default)]
pub struct RawNodeSnapshot {
    /// Node reference wall-clock Unix epoch seconds.
    pub now_unix_secs: u64,
    /// Whether `dbsrt-server` is currently running (`None` if not probed).
    pub daemon_running: Option<bool>,
    /// Canonical script name -> lowercase SHA-256 hex digest.
    pub script_hashes: BTreeMap<String, String>,
    /// Relative artifact path -> modification Unix timestamp in seconds.
    pub artifact_mtimes: BTreeMap<String, u64>,
    /// Contents of `run/supervisor_status.json` if present.
    pub supervisor_json: Option<String>,
    /// Contents of `run/batch_queue.json` if present.
    pub batch_queue_json: Option<String>,
    /// Contents of `run/batch_aborted` if present.
    pub batch_aborted_text: Option<String>,
    /// Bounded tail of `logs/dbsrt-server.log`.
    pub server_log_tail: Option<String>,
    /// Bounded tail of `logs/batch_worker.log`.
    pub worker_log_tail: Option<String>,
    /// Bounded tail of `logs/rpi4_supervisor.log`.
    pub supervisor_log_tail: Option<String>,
    /// Bounded tails of NDJSON batch event files.
    pub batch_events_tails: Vec<String>,
    /// Bounded tail of recent `logs/faultwatch/*.jsonl` entries.
    pub faultwatch_tail: Option<String>,
}

/// Formats Unix epoch seconds into an ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// # Arguments
/// * `unix_secs` - Seconds elapsed since `1970-01-01T00:00:00Z`.
///
/// # Returns
/// Formatted 20-character ISO-8601 UTC string.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Pure civil-from-days Gregorian conversion with zero external timezone state.
pub fn format_unix_iso(unix_secs: u64) -> String {
    let secs_of_day = unix_secs % SECONDS_PER_DAY;
    let hour = secs_of_day / SECONDS_PER_HOUR;
    let min = (secs_of_day % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    let sec = secs_of_day % SECONDS_PER_MINUTE;

    let z = (unix_secs / SECONDS_PER_DAY) as i64 + CIVIL_EPOCH_DAYS_OFFSET;
    let era = if z >= 0 {
        z
    } else {
        z - (CIVIL_DAYS_PER_400_YEARS - 1)
    } / CIVIL_DAYS_PER_400_YEARS;
    let doe = (z - era * CIVIL_DAYS_PER_400_YEARS) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / DAYS_PER_COMMON_YEAR;
    let y = i64::from(yoe) + era * YEARS_PER_ERA;
    let doy = doe - (DAYS_PER_COMMON_YEAR * yoe + yoe / LEAP_STEP_YEARS - yoe / CENTURY_STEP_YEARS);
    let mp = (DOY_DIV * doy + DOY_BIAS) / DOY_FACTOR;
    let day = doy - (DOY_FACTOR * mp + DOY_BIAS) / DOY_DIV + 1;
    let month = if mp < 10 {
        mp + MARCH_SHIFT_HIGH
    } else {
        mp - MARCH_SHIFT_LOW
    };
    let year = if month <= CIVIL_MARCH_PIVOT { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Analyzes a [`RawNodeSnapshot`] into a structured [`NodeProbeReport`].
///
/// # Arguments
/// * `role` - Logical cluster tier (`Local`, `Rpi4`, `O2switch`).
/// * `target` - Display label for target host or path.
/// * `probe_ms` - Measured probe wall-clock latency in milliseconds.
/// * `snap` - Collected [`RawNodeSnapshot`].
///
/// # Returns
/// Fully populated [`NodeProbeReport`].
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Evaluates supervisor health, `batch_aborted`, queue failures, blacklisted media streaks,
/// LVE faultwatch, 3 component log streams, and 4 freshness artifacts. Derives node `status`
/// as the maximum severity across all active `alerts`.
pub fn analyze_node_snapshot(
    role: NodeRole,
    target: String,
    probe_ms: u64,
    snap: RawNodeSnapshot,
) -> NodeProbeReport {
    let now = snap.now_unix_secs;
    let mut alerts = Vec::new();

    let supervisor_state = evaluate_supervisor(snap.supervisor_json.as_deref(), now, &mut alerts);
    let batch_aborted = evaluate_batch_aborted(snap.batch_aborted_text.as_deref(), &mut alerts);
    let failed_queue_targets = evaluate_batch_queue(snap.batch_queue_json.as_deref(), &mut alerts);
    let blacklisted_stems = evaluate_blacklisted_stems(&snap.batch_events_tails, &mut alerts);
    let lve_faults_24h = evaluate_faultwatch(snap.faultwatch_tail.as_deref(), now, &mut alerts);

    let components = vec![
        analyze_component_log(
            "dbsrt-server",
            snap.server_log_tail.as_deref(),
            now,
            &mut alerts,
        ),
        analyze_component_log(
            "batch_worker",
            snap.worker_log_tail.as_deref(),
            now,
            &mut alerts,
        ),
        analyze_component_log(
            "rpi4_supervisor",
            snap.supervisor_log_tail.as_deref(),
            now,
            &mut alerts,
        ),
    ];
    let freshness = evaluate_freshness_artifacts(&snap.artifact_mtimes, now, &mut alerts);

    alerts.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
    let status = alerts
        .iter()
        .map(|a| a.severity)
        .max()
        .unwrap_or(DoctorSeverity::Ok);

    NodeProbeReport {
        node: role.as_str().to_string(),
        target,
        reachable: true,
        skipped: false,
        probe_ms,
        status,
        probe_error: None,
        daemon_running: snap.daemon_running,
        supervisor_state,
        batch_aborted,
        failed_queue_targets,
        blacklisted_stems,
        lve_faults_24h,
        components,
        freshness,
        alerts,
        script_hashes: snap.script_hashes,
    }
}

/// Evaluates `supervisor_status.json` content and pushes any active supervisor alerts.
fn evaluate_supervisor(
    raw_json: Option<&str>,
    now_unix_secs: u64,
    alerts: &mut Vec<NodeAlert>,
) -> Option<String> {
    let text = raw_json?.trim();
    if text.is_empty() {
        return None;
    }
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else {
        alerts.push(NodeAlert {
            id: "supervisor_corrupted".to_string(),
            severity: DoctorSeverity::Critical,
            component: "rpi4_supervisor".to_string(),
            summary: "Corrupted supervisor_status.json".to_string(),
        });
        return Some("corrupted".to_string());
    };
    let state = val
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("healthy")
        .to_string();
    let launch_errs = val
        .get("launch_error_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let failed_types: Vec<String> = val
        .get("failed_batch_types")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or("failed")))
                .collect()
        })
        .unwrap_or_default();

    if state == "degraded" || launch_errs > 0 || !failed_types.is_empty() {
        let detail = if failed_types.is_empty() {
            format!("Supervisor {state} (launch_error_count={launch_errs})")
        } else {
            format!(
                "Supervisor {state} (launch_errors={launch_errs}: {})",
                failed_types.join(", ")
            )
        };
        alerts.push(NodeAlert {
            id: "supervisor_degraded".to_string(),
            severity: DoctorSeverity::Critical,
            component: "rpi4_supervisor".to_string(),
            summary: sanitize_summary(&detail),
        });
    }
    if let Some(ts) = val.get("unix_ts").and_then(|v| v.as_u64()) {
        let age = now_unix_secs.saturating_sub(ts);
        if age > SUPERVISOR_STALE_TTL_SECS {
            alerts.push(NodeAlert {
                id: "supervisor_stale".to_string(),
                severity: DoctorSeverity::Warning,
                component: "rpi4_supervisor".to_string(),
                summary: format!(
                    "Supervisor heartbeat stale ({age}s > {SUPERVISOR_STALE_TTL_SECS}s)"
                ),
            });
        }
    }
    Some(state)
}

/// Evaluates `run/batch_aborted` sentinel content.
fn evaluate_batch_aborted(raw_text: Option<&str>, alerts: &mut Vec<NodeAlert>) -> bool {
    let Some(text) = raw_text else {
        return false;
    };
    let reason = if text.trim().is_empty() {
        "batch_aborted sentinel active"
    } else {
        text.trim()
    };
    alerts.push(NodeAlert {
        id: "batch_aborted".to_string(),
        severity: DoctorSeverity::Critical,
        component: "batch_worker".to_string(),
        summary: sanitize_summary(&format!("Batch aborted: {reason}")),
    });
    true
}

/// Evaluates `run/batch_queue.json` for failed targets.
fn evaluate_batch_queue(raw_json: Option<&str>, alerts: &mut Vec<NodeAlert>) -> usize {
    let Some(text) = raw_json else {
        return 0;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return 0;
    };
    let failed = val
        .get("failed_targets")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let total = val
        .get("total_targets")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if failed > 0 {
        alerts.push(NodeAlert {
            id: "batch_queue_failed".to_string(),
            severity: DoctorSeverity::Warning,
            component: "batch_worker".to_string(),
            summary: format!("Batch queue has {failed}/{total} failed target(s)"),
        });
    }
    failed
}

/// Counts distinct `video_stem` entries with `>= 3` consecutive failures across NDJSON tails.
fn evaluate_blacklisted_stems(tails: &[String], alerts: &mut Vec<NodeAlert>) -> usize {
    let mut streaks: BTreeMap<String, usize> = BTreeMap::new();
    for tail in tails {
        for line in tail.lines() {
            update_stem_streak(line.trim(), &mut streaks);
        }
    }
    let count = streaks
        .values()
        .filter(|&&c| c >= BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD)
        .count();
    if count > 0 {
        alerts.push(NodeAlert {
            id: "blacklisted_stems".to_string(),
            severity: DoctorSeverity::Warning,
            component: "batch_worker".to_string(),
            summary: format!(
                "{count} media stem(s) blacklisted (>={BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD} consecutive failures)"
            ),
        });
    }
    count
}

/// Updates consecutive failure streak for one NDJSON batch event line.
fn update_stem_streak(line: &str, streaks: &mut BTreeMap<String, usize>) {
    if line.is_empty() {
        return;
    }
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let Some(stem) = val.get("video_stem").and_then(|v| v.as_str()) else {
        return;
    };
    if stem.is_empty() {
        return;
    }
    let status = val.get("status").and_then(|v| v.as_str()).unwrap_or("");
    let success = val
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(!matches!(status, "FAILED" | "ERROR"));
    if success {
        streaks.insert(stem.to_string(), 0);
    } else {
        let next = streaks.get(stem).copied().unwrap_or(0) + 1;
        streaks.insert(stem.to_string(), next);
    }
}

/// Scans `faultwatch` NDJSON lines for CloudLinux LVE faults/saturation within the last 24h.
fn evaluate_faultwatch(
    raw_tail: Option<&str>,
    now_unix_secs: u64,
    alerts: &mut Vec<NodeAlert>,
) -> usize {
    let Some(tail) = raw_tail else {
        return 0;
    };
    let mut count = 0usize;
    let mut hard_fault = false;
    for line in tail.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(is_hard) = is_active_faultwatch_line(trimmed, now_unix_secs) {
            count += 1;
            hard_fault |= is_hard;
        }
    }
    if count > 0 {
        alerts.push(NodeAlert {
            id: "lve_faultwatch".to_string(),
            severity: if hard_fault {
                DoctorSeverity::Critical
            } else {
                DoctorSeverity::Warning
            },
            component: "faultwatch".to_string(),
            summary: format!("CloudLinux LVE faultwatch recorded {count} event(s) in 24h"),
        });
    }
    count
}

/// Checks whether a single `faultwatch` JSONL line represents an LVE saturation or fault within 24h.
fn is_active_faultwatch_line(line: &str, now_unix_secs: u64) -> Option<bool> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
        return None;
    };
    if let Some(m_str) = val
        .get("minute")
        .or_else(|| val.get("timestamp"))
        .and_then(|v| v.as_str())
    {
        let norm = match m_str.strip_suffix('Z') {
            Some(s) if s.len() == MINUTE_ISO_STEM_LEN => format!("{s}:00Z"),
            _ => m_str.to_string(),
        };
        if let (Some(ts), _) = parse_leading_timestamp(&norm) {
            if ts <= now_unix_secs.saturating_add(SECONDS_PER_DAY)
                && now_unix_secs.saturating_sub(ts) > WINDOW_24H_SECS
            {
                return None;
            }
        }
    }
    let faults = val
        .get("faults")
        .or_else(|| val.get("anyf"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let over_100 = val
        .get("secs_over_100")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let delay = val
        .get("run_delay_ms_max")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if faults == 0 && over_100 == 0 && delay < FAULTWATCH_RUN_DELAY_WARN_MS {
        return None;
    }
    Some(faults > 0)
}

/// Internal deduplication bucket for log error signatures.
#[derive(Default)]
struct LogBucket {
    summary: String,
    fatal: bool,
    tail_total: usize,
    has_ts: bool,
    last_ts: Option<u64>,
    counts: WindowErrorCounts,
}

/// Analyzes one component log tail into rolling window counters, signatures, and alerts.
fn analyze_component_log(
    component: &str,
    raw_tail: Option<&str>,
    now_unix_secs: u64,
    alerts: &mut Vec<NodeAlert>,
) -> ComponentLogReport {
    let Some(tail) = raw_tail else {
        return ComponentLogReport {
            component: component.to_string(),
            ..Default::default()
        };
    };
    let latest_session = count_batch_sessions(component, tail);
    let mut current_session = 0usize;
    let mut totals = WindowErrorCounts::default();
    let mut buckets: BTreeMap<String, LogBucket> = BTreeMap::new();
    let mut last_ts: Option<u64> = None;

    for line in tail.lines() {
        if latest_session > 0 && line.contains(BATCH_SESSION_START_MARKER) {
            current_session += 1;
            last_ts = None;
        }
        let (ts_opt, body) = parse_leading_timestamp(line);
        if ts_opt.is_some() {
            last_ts = ts_opt;
        }
        let eff_ts = ts_opt.or(last_ts);
        if eff_ts.is_none() && current_session < latest_session {
            continue;
        }
        let (is_err, is_fatal) = is_error_line(body);
        if !is_err || body.is_empty() {
            continue;
        }
        totals.tail_total += 1;
        record_log_hit(
            &mut buckets,
            &mut totals,
            (body, is_fatal),
            (eff_ts, now_unix_secs),
        );
    }

    let top_signatures = finalize_signatures(buckets, component, alerts);
    ComponentLogReport {
        component: component.to_string(),
        counts: totals,
        top_signatures,
    }
}

/// Counts `BATCH_SESSION_START_MARKER` banners when scanning `batch_worker` multi-session logs.
fn count_batch_sessions(component: &str, tail: &str) -> usize {
    if component != "batch_worker" {
        return 0;
    }
    tail.lines()
        .filter(|line| line.contains(BATCH_SESSION_START_MARKER))
        .count()
}

/// Updates signature bucket and window totals for one error log line.
fn record_log_hit(
    buckets: &mut BTreeMap<String, LogBucket>,
    totals: &mut WindowErrorCounts,
    line_info: (&str, bool),
    clocks: (Option<u64>, u64),
) {
    let (body, is_fatal) = line_info;
    let (eff_ts, now_unix_secs) = clocks;
    let summary = sanitize_summary(body);
    let key = normalize_digits(&summary);

    let b = buckets.entry(key).or_default();
    b.summary = summary;
    b.fatal |= is_fatal;
    b.tail_total += 1;

    if let Some(ts) = eff_ts {
        if ts <= now_unix_secs.saturating_add(SECONDS_PER_DAY) {
            b.has_ts = true;
            b.last_ts = Some(b.last_ts.map_or(ts, |p| p.max(ts)));
            let age = now_unix_secs.saturating_sub(ts);
            if age <= WINDOW_1H_SECS {
                b.counts.h1 += 1;
                totals.h1 += 1;
            }
            if age <= WINDOW_24H_SECS {
                b.counts.h24 += 1;
                totals.h24 += 1;
            }
            if age <= WINDOW_72H_SECS {
                b.counts.h72 += 1;
                totals.h72 += 1;
            }
        }
    }
}

/// Converts deduplicated buckets into sorted signatures and pushes active alerts.
fn finalize_signatures(
    buckets: BTreeMap<String, LogBucket>,
    component: &str,
    alerts: &mut Vec<NodeAlert>,
) -> Vec<DedupErrorSignature> {
    let mut active: Vec<LogBucket> = buckets
        .into_values()
        .filter(|b| !b.has_ts || b.counts.h72 > 0)
        .collect();
    active.sort_by(|a, b| {
        b.fatal
            .cmp(&a.fatal)
            .then_with(|| b.counts.h1.cmp(&a.counts.h1))
            .then_with(|| b.counts.h24.cmp(&a.counts.h24))
            .then_with(|| b.tail_total.cmp(&a.tail_total))
    });

    active
        .into_iter()
        .take(MAX_TOP_SIGNATURES_PER_COMPONENT)
        .enumerate()
        .map(|(i, b)| {
            let critical = b.fatal || b.counts.h1 > 0;
            let count = if b.has_ts { b.counts.h72 } else { b.tail_total };
            alerts.push(NodeAlert {
                id: format!("log_{}_{}", component.replace('-', "_"), i + 1),
                severity: if critical {
                    DoctorSeverity::Critical
                } else {
                    DoctorSeverity::Warning
                },
                component: component.to_string(),
                summary: format!("{} (x{count})", b.summary),
            });
            DedupErrorSignature {
                summary: b.summary,
                count,
                last_seen: b.last_ts.map(format_unix_iso),
                critical,
            }
        })
        .collect()
}

/// Evaluates mtime freshness across [`TRACKED_FRESHNESS_ARTIFACTS`].
fn evaluate_freshness_artifacts(
    mtimes: &BTreeMap<String, u64>,
    now_unix_secs: u64,
    alerts: &mut Vec<NodeAlert>,
) -> Vec<ArtifactFreshness> {
    let mut out = Vec::with_capacity(TRACKED_FRESHNESS_ARTIFACTS.len());
    for (short_name, rel_path, ttl_secs) in TRACKED_FRESHNESS_ARTIFACTS {
        let mt = mtimes.get(rel_path).copied();
        let age_secs = mt.map(|t| now_unix_secs.saturating_sub(t));
        let stale = age_secs.is_some_and(|age| age > ttl_secs);
        if stale && short_name == "nas_ftp_index.json" {
            let age_h = age_secs.unwrap_or(0) / SECONDS_PER_HOUR;
            alerts.push(NodeAlert {
                id: "stale_nas_ftp_index".to_string(),
                severity: DoctorSeverity::Warning,
                component: "dataset".to_string(),
                summary: format!(
                    "nas_ftp_index.json is stale ({age_h}h > {}h)",
                    ttl_secs / SECONDS_PER_HOUR
                ),
            });
        }
        out.push(ArtifactFreshness {
            name: short_name.to_string(),
            exists: mt.is_some(),
            age_secs,
            ttl_secs,
            stale,
        });
    }
    out
}

/// Parses `[YYYY-MM-DD HH:MM:SS]` or ISO-8601 leading timestamps into `(Option<unix_secs>, rest)`.
pub fn parse_leading_timestamp(line: &str) -> (Option<u64>, &str) {
    let trimmed = line.trim_start();
    let (cand, bracketed) = match trimmed.strip_prefix('[') {
        Some(inner) => (inner, true),
        None => (trimmed, false),
    };
    if cand.len() < TS_PREFIX_BYTES || !cand.is_char_boundary(TS_PREFIX_BYTES) {
        return (None, trimmed);
    }
    let b = cand.as_bytes();
    let ok = b[4] == b'-'
        && b[7] == b'-'
        && (b[10] == b' ' || b[10] == b'T')
        && b[13] == b':'
        && b[16] == b':';
    if !ok {
        return (None, trimmed);
    }
    let Some(ts) = parse_datetime_slice(&cand[..TS_PREFIX_BYTES]) else {
        return (None, trimmed);
    };
    let tail = strip_ts_remainder(&cand[TS_PREFIX_BYTES..], bracketed);
    (Some(ts), tail.trim_start())
}

/// Parses an exact 19-byte `YYYY-MM-DD?HH:MM:SS` prefix into Unix epoch seconds.
fn parse_datetime_slice(s: &str) -> Option<u64> {
    let (year, month, day) = parse_ymd_fields(s)?;
    let (hour, min, sec) = parse_hms_fields(s)?;
    let days = civil_ymd_to_epoch_days(year, month, day)?;
    let secs =
        u64::from(hour) * SECONDS_PER_HOUR + u64::from(min) * SECONDS_PER_MINUTE + u64::from(sec);
    Some(days * SECONDS_PER_DAY + secs)
}

/// Parses and range-checks `(year, month, day)` from a 19-byte datetime slice.
fn parse_ymd_fields(s: &str) -> Option<(i64, u32, u32)> {
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let valid = (MIN_LOG_YEAR..=MAX_LOG_YEAR).contains(&year)
        && (1..=MAX_MONTH).contains(&month)
        && (1..=MAX_DAY).contains(&day);
    valid.then_some((year, month, day))
}

/// Parses and range-checks `(hour, minute, second)` from a 19-byte datetime slice.
fn parse_hms_fields(s: &str) -> Option<(u32, u32, u32)> {
    let hour: u32 = s.get(11..13)?.parse().ok()?;
    let min: u32 = s.get(14..16)?.parse().ok()?;
    let sec: u32 = s.get(17..19)?.parse().ok()?;
    let valid = hour <= MAX_HOUR && min <= MAX_MIN_SEC && sec <= MAX_MIN_SEC;
    valid.then_some((hour, min, sec))
}

/// Converts a Gregorian `(year, month, day)` triple into non-negative days since `1970-01-01`.
fn civil_ymd_to_epoch_days(year: i64, month: u32, day: u32) -> Option<u64> {
    let y_adj = if month <= CIVIL_MARCH_PIVOT {
        year - 1
    } else {
        year
    };
    let era = y_adj.div_euclid(YEARS_PER_ERA);
    let yoe = y_adj.rem_euclid(YEARS_PER_ERA) as u32;
    let m_shift = if month > CIVIL_MARCH_PIVOT {
        month - MARCH_SHIFT_HIGH
    } else {
        month + MARCH_SHIFT_LOW
    };
    let doy = (DOY_FACTOR * m_shift + DOY_BIAS) / DOY_DIV + day - 1;
    let doe = yoe * DAYS_PER_COMMON_YEAR + yoe / LEAP_STEP_YEARS - yoe / CENTURY_STEP_YEARS + doy;
    let days = era * CIVIL_DAYS_PER_400_YEARS + i64::from(doe) - CIVIL_EPOCH_DAYS_OFFSET;
    u64::try_from(days).ok()
}

/// Strips fractional seconds, timezone offset, or closing `]` from a timestamp suffix.
fn strip_ts_remainder(rem: &str, bracketed: bool) -> &str {
    if bracketed {
        return rem.strip_prefix(']').unwrap_or(rem);
    }
    let rest = match rem.strip_prefix('.') {
        Some(f) => f.trim_start_matches(|c: char| c.is_ascii_digit()),
        None => rem,
    };
    if let Some(z) = rest.strip_prefix('Z') {
        return z;
    }
    if rest.starts_with('+') || rest.starts_with('-') {
        return rest.split_once(char::is_whitespace).map_or("", |(_, t)| t);
    }
    rest
}

/// Classifies a log line into `(is_error, is_fatal)`, ignoring `[DEBUG]` traces and graceful shutdowns.
fn is_error_line(body: &str) -> (bool, bool) {
    if body.contains("[DEBUG]") || (body.contains("Received signal") && body.contains("Stopping")) {
        return (false, false);
    }
    if body.contains("🛑") || contains_word(body, "FATAL") {
        return (true, true);
    }
    if body.contains('❌') || contains_word(body, "ERROR") {
        return (true, false);
    }
    (false, false)
}

/// Word-boundary ASCII token match.
fn contains_word(haystack: &str, word: &str) -> bool {
    let hb = haystack.as_bytes();
    let wb = word.as_bytes();
    if wb.is_empty() || hb.len() < wb.len() {
        return false;
    }
    for i in 0..=(hb.len() - wb.len()) {
        if &hb[i..i + wb.len()] != wb {
            continue;
        }
        let l_ok = i == 0 || !(hb[i - 1].is_ascii_alphanumeric() || hb[i - 1] == b'_');
        let r_ok = i + wb.len() == hb.len()
            || !(hb[i + wb.len()].is_ascii_alphanumeric() || hb[i + wb.len()] == b'_');
        if l_ok && r_ok {
            return true;
        }
    }
    false
}

/// Redacts secrets and URL credentials and bounds character length.
fn sanitize_summary(raw: &str) -> String {
    let red = dbsrt_core::redact_url_userinfo(&dbsrt_core::redact_secrets(raw.trim()));
    if red.chars().count() <= MAX_SIGNATURE_SUMMARY_CHARS {
        return red;
    }
    let mut s: String = red.chars().take(MAX_SIGNATURE_SUMMARY_CHARS).collect();
    s.push('…');
    s
}

/// Collapses digit runs into `<N>` for error signature deduplication.
fn normalize_digits(summary: &str) -> String {
    let mut out = String::with_capacity(summary.len());
    let mut in_num = false;
    for ch in summary.chars() {
        if ch.is_ascii_digit() {
            if !in_num {
                out.push_str("<N>");
                in_num = true;
            }
        } else {
            in_num = false;
            out.push(ch);
        }
    }
    out
}
