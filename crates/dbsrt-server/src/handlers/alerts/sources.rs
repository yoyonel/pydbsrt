//! State file, NDJSON blacklist, CloudLinux LVE faultwatch, and index staleness alert probes.
//!
//! # Data Flow & Invariants
//! Evaluates structured runtime state files (`supervisor_status.json`, `batch_aborted`,
//! `batch_queue.json`, `batch_events.jsonl` / `batch_tvshow_nightly.jsonl`, `logs/faultwatch/`,
//! `nas_ftp_index.json`) using bounded reads and deterministic Unix timestamps.

use crate::constants::{
    ALERTS_EVENTS_TAIL_BYTES, ALERTS_FAULTWATCH_TAIL_BYTES, ALERT_WINDOW_24H_SECS,
    BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD, FAULTWATCH_RUN_DELAY_WARN_MS,
    NAS_FTP_INDEX_STALE_TTL_SECS, SECONDS_PER_DAY, SECONDS_PER_HOUR,
    SUPERVISOR_STALE_THRESHOLD_SECS,
};
use crate::dtos::{AlertSeverity, BatchQueueStatus, MonitoringAlertItem};
use crate::handlers::alerts::log_scan::{
    read_file_tail_utf8, split_leading_timestamp, truncate_chars,
};
use crate::handlers::monitoring::format_unix_to_iso;
use crate::MAX_ALERT_SUMMARY_CHARS;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Evaluates `run/supervisor_status.json` for degraded state, launch errors, and stale heartbeats.
///
/// # Arguments
/// * `path` - Path to `supervisor_status.json`.
/// * `now_unix_secs` - Current reference Unix epoch timestamp in seconds.
///
/// # Returns
/// Tuple `(state_label, launch_errors, heartbeat_age_secs, alerts)`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Missing `supervisor_status.json` returns `"absent"` with zero alerts (local dev / non-supervised
/// nodes do not run a supervisor). Corrupted JSON, `"degraded"` state, `launch_error_count > 0`,
/// or non-empty `failed_batch_types` emit [`AlertSeverity::Critical`]; stale heartbeats emit
/// [`AlertSeverity::Warning`].
pub fn check_supervisor_alerts(
    path: &Path,
    now_unix_secs: u64,
) -> (String, u64, Option<u64>, Vec<MonitoringAlertItem>) {
    let Ok(content) = fs::read_to_string(path) else {
        return ("absent".to_string(), 0, None, Vec::new());
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        let alert = MonitoringAlertItem {
            id: "supervisor_corrupted".to_string(),
            severity: AlertSeverity::Critical,
            component: "supervisor".to_string(),
            summary: "Corrupted supervisor_status.json payload".to_string(),
            count: 1,
            last_seen: None,
            window_counts: None,
        };
        return ("corrupted".to_string(), 0, None, vec![alert]);
    };

    parse_supervisor_value(&val, now_unix_secs)
}

/// Extracts supervisor metrics and active alerts from a parsed JSON object.
fn parse_supervisor_value(
    val: &serde_json::Value,
    now_unix_secs: u64,
) -> (String, u64, Option<u64>, Vec<MonitoringAlertItem>) {
    let state = val
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("healthy")
        .to_string();
    let launch_errors = val
        .get("launch_error_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let unix_ts = val.get("unix_ts").and_then(|v| v.as_u64());
    let ts_iso = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| unix_ts.map(format_unix_to_iso));
    let age_secs = unix_ts.map(|ts| now_unix_secs.saturating_sub(ts));

    let failed_types = extract_failed_batch_types(val);
    let mut alerts = Vec::new();
    if state == "degraded" || launch_errors > 0 || !failed_types.is_empty() {
        let detail = if failed_types.is_empty() {
            format!("Supervisor state={state} (launch_error_count={launch_errors})")
        } else {
            format!("Supervisor state={state} (launch_errors={launch_errors}: {failed_types})")
        };
        alerts.push(MonitoringAlertItem {
            id: "supervisor_degraded".to_string(),
            severity: AlertSeverity::Critical,
            component: "supervisor".to_string(),
            summary: truncate_chars(
                &dbsrt_core::redact_secrets(&detail),
                MAX_ALERT_SUMMARY_CHARS,
            ),
            count: usize::try_from(launch_errors.max(1)).unwrap_or(1),
            last_seen: ts_iso.clone(),
            window_counts: None,
        });
    } else if state == "warning" {
        alerts.push(MonitoringAlertItem {
            id: "supervisor_warning".to_string(),
            severity: AlertSeverity::Warning,
            component: "supervisor".to_string(),
            summary: "Supervisor reported warning state".to_string(),
            count: 1,
            last_seen: ts_iso.clone(),
            window_counts: None,
        });
    }

    if let Some(age) = age_secs {
        if age > SUPERVISOR_STALE_THRESHOLD_SECS {
            alerts.push(MonitoringAlertItem {
                id: "supervisor_stale_heartbeat".to_string(),
                severity: AlertSeverity::Warning,
                component: "supervisor".to_string(),
                summary: format!(
                    "Supervisor heartbeat is stale ({age}s > {SUPERVISOR_STALE_THRESHOLD_SECS}s)"
                ),
                count: 1,
                last_seen: ts_iso,
                window_counts: None,
            });
        }
    }

    (state, launch_errors, age_secs, alerts)
}

/// Formats `failed_batch_types` map entries into a compact comma-separated description.
fn extract_failed_batch_types(val: &serde_json::Value) -> String {
    let Some(obj) = val.get("failed_batch_types").and_then(|v| v.as_object()) else {
        return String::new();
    };
    obj.iter()
        .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or("failed")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Checks for the presence of the `run/batch_aborted` fast-failure sentinel file.
///
/// # Arguments
/// * `path` - Path to `run/batch_aborted`.
///
/// # Returns
/// `Some(MonitoringAlertItem)` with [`AlertSeverity::Critical`] if the sentinel exists, else `None`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Redacts secrets and URL credentials from the sentinel reason string before building the alert.
pub fn check_batch_aborted_alert(path: &Path) -> Option<MonitoringAlertItem> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    let (ts_opt, rest) = split_leading_timestamp(trimmed);
    let raw_reason = if rest.is_empty() {
        "Fast-failure batch_aborted sentinel active"
    } else {
        rest
    };
    let redacted = dbsrt_core::redact_url_userinfo(&dbsrt_core::redact_secrets(raw_reason));
    let last_seen = ts_opt
        .map(format_unix_to_iso)
        .or_else(|| file_mtime_unix_secs(path).map(format_unix_to_iso));

    Some(MonitoringAlertItem {
        id: "batch_aborted".to_string(),
        severity: AlertSeverity::Critical,
        component: "batch_worker".to_string(),
        summary: truncate_chars(
            &format!("Batch aborted: {redacted}"),
            MAX_ALERT_SUMMARY_CHARS,
        ),
        count: 1,
        last_seen,
        window_counts: None,
    })
}

/// Checks `run/batch_queue.json` for non-zero `failed_targets`.
///
/// # Arguments
/// * `path` - Path to `run/batch_queue.json`.
///
/// # Returns
/// `(failed_targets_count, Option<MonitoringAlertItem>)`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Emits [`AlertSeverity::Warning`] when `failed_targets > 0`.
pub fn check_batch_queue_alert(path: &Path) -> (usize, Option<MonitoringAlertItem>) {
    let Ok(content) = fs::read_to_string(path) else {
        return (0, None);
    };
    let Ok(queue) = serde_json::from_str::<BatchQueueStatus>(&content) else {
        return (0, None);
    };
    if queue.failed_targets == 0 {
        return (0, None);
    }
    let last_seen = (!queue.updated_at.is_empty()).then_some(queue.updated_at);
    let alert = MonitoringAlertItem {
        id: "batch_queue_failures".to_string(),
        severity: AlertSeverity::Warning,
        component: "batch_worker".to_string(),
        summary: format!(
            "Batch queue reports {} failed target(s) out of {}",
            queue.failed_targets, queue.total_targets
        ),
        count: queue.failed_targets,
        last_seen,
        window_counts: None,
    };
    (queue.failed_targets, Some(alert))
}

/// Counts distinct `video_stem` entries with `>= 3` consecutive failures across NDJSON event files.
///
/// # Arguments
/// * `paths` - Ordered candidate NDJSON event paths (`logs/batch_events.jsonl`, `data/reports/batch_tvshow_nightly.jsonl`).
///
/// # Returns
/// `(blacklisted_count, Option<MonitoringAlertItem>)`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Mirrors `scripts/workers/batch_worker/stems.py`: a `success: true` record resets that stem's
/// consecutive failure streak to `0`; a `success: false` (or `status == "FAILED" | "ERROR"`)
/// increments it. Any stem ending with `streak >= BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD` (`3`)
/// is counted as blacklisted.
pub fn check_blacklisted_media_alert(paths: &[&Path]) -> (usize, Option<MonitoringAlertItem>) {
    let mut streaks: BTreeMap<String, (usize, Option<String>)> = BTreeMap::new();
    for path in paths {
        let Some(tail) = read_file_tail_utf8(path, ALERTS_EVENTS_TAIL_BYTES) else {
            continue;
        };
        accumulate_event_streaks(&tail, &mut streaks);
    }

    let (blacklisted, latest_iso) = summarize_blacklisted_streaks(&streaks);
    if blacklisted == 0 {
        return (0, None);
    }
    let alert = MonitoringAlertItem {
        id: "blacklisted_media".to_string(),
        severity: AlertSeverity::Warning,
        component: "batch_worker".to_string(),
        summary: format!(
            "{blacklisted} media stem(s) blacklisted (>={BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD} consecutive failures)"
        ),
        count: blacklisted,
        last_seen: latest_iso,
        window_counts: None,
    };
    (blacklisted, Some(alert))
}

/// Counts stems meeting [`BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD`] and returns their latest timestamp.
fn summarize_blacklisted_streaks(
    streaks: &BTreeMap<String, (usize, Option<String>)>,
) -> (usize, Option<String>) {
    let mut blacklisted = 0usize;
    let mut latest_iso: Option<String> = None;
    for (streak, ts_iso) in streaks.values() {
        if *streak < BLACKLIST_CONSECUTIVE_FAILURES_THRESHOLD {
            continue;
        }
        blacklisted += 1;
        update_latest_iso(&mut latest_iso, ts_iso.as_deref());
    }
    (blacklisted, latest_iso)
}

/// Updates `slot` when `candidate` is lexicographically newer.
fn update_latest_iso(slot: &mut Option<String>, candidate: Option<&str>) {
    let Some(ts) = candidate else {
        return;
    };
    if slot.as_deref().is_none_or(|prev| ts > prev) {
        *slot = Some(ts.to_string());
    }
}

/// Updates per-stem consecutive failure streaks from NDJSON lines.
fn accumulate_event_streaks(ndjson: &str, streaks: &mut BTreeMap<String, (usize, Option<String>)>) {
    for line in ndjson.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let Some(stem) = val.get("video_stem").and_then(|v| v.as_str()) else {
            continue;
        };
        if stem.is_empty() {
            continue;
        }
        let status = val.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let success = val
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(!matches!(status, "FAILED" | "ERROR"));
        let ts = val
            .get("timestamp_iso")
            .or_else(|| val.get("end_iso"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if success {
            streaks.insert(stem.to_string(), (0, ts));
        } else {
            let next = streaks.get(stem).map_or(1, |(c, _)| c + 1);
            streaks.insert(stem.to_string(), (next, ts));
        }
    }
}

/// Scans `logs/faultwatch/` for recent CloudLinux LVE CPU saturation, throttling, or fault records.
///
/// # Arguments
/// * `dir` - Path to `logs/faultwatch`.
/// * `now_unix_secs` - Reference current Unix epoch timestamp in seconds.
///
/// # Returns
/// `(fault_entries_24h, Option<MonitoringAlertItem>)`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Inspects files in `logs/faultwatch/` whose mtime falls within `24h` (or any non-empty JSONL/log
/// file with recent minute timestamps). Counts records exhibiting LVE faults (`faults > 0`,
/// `anyf > 0`, `secs_over_100 > 0`, or `run_delay_ms_max >= FAULTWATCH_RUN_DELAY_WARN_MS`).
pub fn check_faultwatch_alert(
    dir: &Path,
    now_unix_secs: u64,
) -> (usize, Option<MonitoringAlertItem>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (0, None);
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();

    let mut count_24h = 0usize;
    let mut hard_faults = false;
    let mut last_seen: Option<String> = None;

    for path in paths {
        let mtime = file_mtime_unix_secs(&path);
        if mtime.is_some_and(|mt| now_unix_secs.saturating_sub(mt) > ALERT_WINDOW_24H_SECS * 2) {
            continue;
        }
        let Some(tail) = read_file_tail_utf8(&path, ALERTS_FAULTWATCH_TAIL_BYTES) else {
            continue;
        };
        scan_faultwatch_tail(
            &tail,
            (now_unix_secs, mtime),
            (&mut count_24h, &mut hard_faults, &mut last_seen),
        );
    }

    if count_24h == 0 {
        return (0, None);
    }
    let severity = if hard_faults {
        AlertSeverity::Critical
    } else {
        AlertSeverity::Warning
    };
    let alert = MonitoringAlertItem {
        id: "cloudlinux_lve_faultwatch".to_string(),
        severity,
        component: "faultwatch".to_string(),
        summary: format!(
            "CloudLinux LVE faultwatch recorded {count_24h} throttling/fault event(s) in 24h"
        ),
        count: count_24h,
        last_seen,
        window_counts: None,
    };
    (count_24h, Some(alert))
}

/// Evaluates lines from a single `faultwatch` file and updates 24h counters.
fn scan_faultwatch_tail(
    tail: &str,
    clocks: (u64, Option<u64>),
    acc: (&mut usize, &mut bool, &mut Option<String>),
) {
    let (now_unix_secs, file_mtime) = clocks;
    let (count_24h, hard_faults, last_seen) = acc;

    for line in tail.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((is_hard, ts_iso)) = classify_faultwatch_line(trimmed, now_unix_secs, file_mtime)
        else {
            continue;
        };
        *count_24h += 1;
        *hard_faults |= is_hard;
        if ts_iso.is_some() {
            *last_seen = ts_iso;
        }
    }
}

/// Classifies one trimmed `faultwatch` line (JSON or plain text) into `Some((is_hard_fault, last_seen_iso))`.
fn classify_faultwatch_line(
    trimmed: &str,
    now_unix_secs: u64,
    file_mtime: Option<u64>,
) -> Option<(bool, Option<String>)> {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return evaluate_faultwatch_json(&val, now_unix_secs);
    }
    let recent =
        file_mtime.is_none_or(|mt| now_unix_secs.saturating_sub(mt) <= ALERT_WINDOW_24H_SECS);
    recent.then(|| (false, file_mtime.map(format_unix_to_iso)))
}

/// Checks whether a single `faultwatch` JSON record represents a recent (`<= 24h`) LVE fault/throttle.
fn evaluate_faultwatch_json(
    val: &serde_json::Value,
    now_unix_secs: u64,
) -> Option<(bool, Option<String>)> {
    let minute_str = val
        .get("minute")
        .or_else(|| val.get("timestamp"))
        .and_then(|v| v.as_str());
    if let Some(m_str) = minute_str {
        let normalized = normalize_minute_iso(m_str);
        if let (Some(ts), _) = split_leading_timestamp(&normalized) {
            if ts <= now_unix_secs.saturating_add(SECONDS_PER_DAY)
                && now_unix_secs.saturating_sub(ts) > ALERT_WINDOW_24H_SECS
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
    let delay_ms = val
        .get("run_delay_ms_max")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    if faults == 0 && over_100 == 0 && delay_ms < FAULTWATCH_RUN_DELAY_WARN_MS {
        return None;
    }
    Some((faults > 0, minute_str.map(str::to_string)))
}

/// Expands `YYYY-MM-DDTHH:MMZ` minute strings into `YYYY-MM-DDTHH:MM:00Z` for timestamp parsing.
fn normalize_minute_iso(raw: &str) -> String {
    if let Some(stem) = raw.strip_suffix('Z') {
        if stem.len() == 16 {
            return format!("{stem}:00Z");
        }
    }
    raw.to_string()
}

/// Checks `data/reports/nas_ftp_index.json` mtime staleness when the index file exists.
///
/// # Arguments
/// * `path` - Path to `nas_ftp_index.json`.
/// * `now_unix_secs` - Reference current Unix epoch timestamp in seconds.
///
/// # Returns
/// `(Option<age_secs>, Option<MonitoringAlertItem>)`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Absent files return `(None, None)` (cloud worker nodes stream on-demand without local NAS index).
/// When present and `age_secs > NAS_FTP_INDEX_STALE_TTL_SECS`, emits [`AlertSeverity::Warning`].
pub fn check_nas_index_staleness_alert(
    path: &Path,
    now_unix_secs: u64,
) -> (Option<u64>, Option<MonitoringAlertItem>) {
    let Some(mtime) = file_mtime_unix_secs(path) else {
        return (None, None);
    };
    let age_secs = now_unix_secs.saturating_sub(mtime);
    if age_secs <= NAS_FTP_INDEX_STALE_TTL_SECS {
        return (Some(age_secs), None);
    }
    let hours = age_secs / SECONDS_PER_HOUR;
    let alert = MonitoringAlertItem {
        id: "nas_ftp_index_stale".to_string(),
        severity: AlertSeverity::Warning,
        component: "dataset".to_string(),
        summary: format!(
            "NAS FTP index is stale ({hours}h old, TTL={}h)",
            NAS_FTP_INDEX_STALE_TTL_SECS / SECONDS_PER_HOUR
        ),
        count: 1,
        last_seen: Some(format_unix_to_iso(mtime)),
        window_counts: None,
    };
    (Some(age_secs), Some(alert))
}

/// Reads the Unix epoch modification timestamp in seconds for `path`.
pub fn file_mtime_unix_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
