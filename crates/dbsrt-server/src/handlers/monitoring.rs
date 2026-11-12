//! Web Backoffice Monitoring endpoints providing live telemetry and supervisor tracking.

use crate::constants::{
    AUDIT_ACTION_TIMEOUT_SECS, AUDIT_SCRIPT_NAME, AUDIT_SCRIPT_PATH, BYTES_PER_GIB_F64,
    CIVIL_DAYS_PER_400_YEARS, CIVIL_EPOCH_DAYS_OFFSET, DEFAULT_AUDIT_LAST_FINISHED_PATH,
    DEFAULT_AUDIT_STATUS_PATH, DEFAULT_BATCH_EVENTS_PATH, DEFAULT_BATCH_QUEUE_PATH,
    DEFAULT_ESTIMATED_MEDIA_DURATION_SECS, DEFAULT_JOBS_DIR, DEFAULT_SSE_INTERVAL_MS,
    DEFAULT_SSE_KEEPALIVE_SECS, DEFAULT_SUPERVISOR_STATUS_PATH, DEFAULT_TOP_PAGE_LIMIT,
    FALLBACK_BATCH_EVENTS_PATH, MAX_MONITORING_RECENT_EVENTS, SECONDS_PER_DAY, SECONDS_PER_HOUR,
    SECONDS_PER_MINUTE, SUPERVISOR_STALE_THRESHOLD_SECS,
};
use crate::dtos::{
    ActiveJobStatus, AuditActionResponse, AuditStatus, BatchQueueStatus, MetricsResponse,
    MonitoringStreamState, MonitoringTelemetryResponse, RecentBatchEvent,
    SupervisorHeartbeatPayload, SupervisorTelemetry, SystemStats, TopPageQuery,
};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Html;
use axum::Json;
use std::convert::Infallible;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Converts Unix epoch seconds into an ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn format_unix_to_iso(unix_secs: u64) -> String {
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
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Returns current system wall-clock time formatted as ISO-8601 UTC.
pub fn current_time_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_to_iso(secs)
}

/// Serves the single-page mobile-responsive HTML monitoring dashboard.
///
/// # Arguments
/// None.
///
/// # Returns
/// An [`Html`] payload embedding `monitoring.html`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Embeds static HTML/CSS/JS asset at compile time for zero runtime I/O overhead.
pub async fn monitoring_ui_handler() -> Html<&'static str> {
    Html(include_str!("../monitoring.html"))
}

/// Parses `/proc/meminfo` key-value pairs into kilobyte counts.
fn parse_meminfo() -> (u64, u64) {
    let Ok(content) = fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let mut total_kb = 0u64;
    let mut available_kb = 0u64;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available_kb = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
    }
    (total_kb, available_kb)
}

/// Samples host OS metrics including CPU load, RAM usage, and uptime.
///
/// # Arguments
/// None.
///
/// # Returns
/// A populated [`SystemStats`] structure.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads volatile metrics from Linux `/proc` filesystem. Falls back gracefully when running in
/// constrained environments or non-Linux hosts.
pub fn sample_system_stats() -> SystemStats {
    // /etc/hostname is missing in some jails (o2switch CageFS); the kernel value is always there
    let hostname = fs::read_to_string("/etc/hostname")
        .or_else(|_| fs::read_to_string("/proc/sys/kernel/hostname"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| {
            std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_string())
        });

    let (total_kb, avail_kb) = parse_meminfo();
    let total_bytes = total_kb * 1024;
    let used_bytes = total_bytes.saturating_sub(avail_kb * 1024);
    let ram_total_gib = total_bytes as f64 / BYTES_PER_GIB_F64;
    let ram_used_gib = used_bytes as f64 / BYTES_PER_GIB_F64;

    let cpu_pct = fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f32>().ok())
        })
        .unwrap_or(0.0);

    let uptime_secs = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
        })
        .map(|secs| secs as u64)
        .unwrap_or(0);

    SystemStats {
        hostname,
        os_name: "Linux".to_string(),
        cpu_pct,
        ram_used_gib,
        ram_total_gib,
        uptime_secs,
    }
}

/// Resolves a relative path checking current directory, `dbsrt/`, `$HOME/dbsrt/`, or `../../dbsrt/`.
pub fn resolve_monitoring_path(rel: &str) -> PathBuf {
    let direct = PathBuf::from(rel);
    if direct.exists() {
        return direct;
    }
    let sub = PathBuf::from("dbsrt").join(rel);
    if sub.exists() {
        return sub;
    }
    if let Ok(home) = std::env::var("HOME") {
        let home_dbsrt = PathBuf::from(home).join("dbsrt").join(rel);
        if home_dbsrt.exists() {
            return home_dbsrt;
        }
    }
    let parent_sub = PathBuf::from("../../dbsrt").join(rel);
    if parent_sub.exists() {
        return parent_sub;
    }
    direct
}

/// Deserializes batch queue status from the worker control file.
///
/// # Arguments
/// None.
///
/// # Returns
/// Current [`BatchQueueStatus`], or default values if file is missing.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads `DEFAULT_BATCH_QUEUE_PATH`. If absent, reports an inactive queue.
pub fn load_batch_queue_status() -> BatchQueueStatus {
    let path = resolve_monitoring_path(DEFAULT_BATCH_QUEUE_PATH);
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(status) = serde_json::from_str::<BatchQueueStatus>(&content) {
            return status;
        }
    }
    BatchQueueStatus::default()
}

/// Parses the audit counts and checks that a `"running"` audit is still alive.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// A requested stop (SIGTERM) publishes `"stopped"` itself, so `"running"` without a live
/// audit pid means the audit was killed: the state becomes `"interrupted"`. `None` (no file)
/// reports `"none"`.
pub fn parse_audit_status(
    content: Option<&str>,
    is_audit_pid: impl Fn(u32) -> bool,
) -> AuditStatus {
    let Some(content) = content else {
        return AuditStatus {
            state: "none".to_string(),
            ..Default::default()
        };
    };
    let Ok(mut status) = serde_json::from_str::<AuditStatus>(content) else {
        return AuditStatus {
            state: "corrupted".to_string(),
            ..Default::default()
        };
    };
    if status.state == "running" && !status.pid.is_some_and(is_audit_pid) {
        status.state = "interrupted".to_string();
    }
    status
}

/// The last complete audit, or `None` when absent, unreadable or not finished.
pub fn parse_last_finished_audit(content: Option<&str>) -> Option<AuditStatus> {
    serde_json::from_str::<AuditStatus>(content?)
        .ok()
        .filter(|s| s.state == "finished")
}

/// Script arguments resuming the published run with its own name, window length and cuts.
///
/// # Errors
/// A running, finished or unknown audit has nothing to resume. The run name must stay a plain
/// file name not starting with `-`, which argparse would read as an option.
pub fn audit_resume_args(status: &AuditStatus) -> Result<Vec<String>, String> {
    if !matches!(status.state.as_str(), "stopped" | "interrupted" | "failed") {
        return Err(format!("Nothing to resume: audit is {}.", status.state));
    }
    let run = &status.run;
    let plain = run.starts_with(|c: char| c.is_ascii_alphanumeric())
        && run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c));
    if !plain {
        return Err("Invalid run name in the audit status.".to_string());
    }
    Ok(vec![
        "start".to_string(),
        run.clone(),
        status.seconds.to_string(),
        status.cuts.to_string(),
    ])
}

/// True when `pid` is alive and runs the audit script (a reused pid runs something else).
pub fn pid_runs_audit(pid: u32) -> bool {
    let needle = AUDIT_SCRIPT_NAME.as_bytes();
    fs::read(format!("/proc/{pid}/cmdline"))
        .is_ok_and(|c| c.windows(needle.len()).any(|w| w == needle))
}

/// Reads `DEFAULT_AUDIT_STATUS_PATH`, never the audit rows (they hold media names).
pub fn load_audit_status() -> AuditStatus {
    let path = resolve_monitoring_path(DEFAULT_AUDIT_STATUS_PATH);
    parse_audit_status(fs::read_to_string(path).ok().as_deref(), pid_runs_audit)
}

fn load_last_finished_audit() -> Option<AuditStatus> {
    let path = resolve_monitoring_path(DEFAULT_AUDIT_LAST_FINISHED_PATH);
    parse_last_finished_audit(fs::read_to_string(path).ok().as_deref())
}

type AuditAction = (StatusCode, Json<AuditActionResponse>);

fn audit_answer(status: StatusCode, message: impl Into<String>) -> AuditAction {
    let ok = status.is_success();
    let message = message.into();
    (status, Json(AuditActionResponse { ok, message }))
}

/// Runs `bin/python3 -I scripts/o2switch_audit_remote.py <args>` from the dbsrt home.
///
/// Hosts without the audit script (RPi4, workstation) answer 404. The script's stdout (pids,
/// run names, counts) is returned; its stderr may hold home paths and only goes to the log.
async fn run_audit_script(args: Vec<String>) -> AuditAction {
    let script = resolve_monitoring_path(AUDIT_SCRIPT_PATH);
    // <home>/scripts/<script>: <home> holds bin/python3 and the run/ files the script uses
    let home = script
        .parent()
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let python = home.join("bin").join("python3");
    if !script.is_file() || !python.is_file() {
        return audit_answer(StatusCode::NOT_FOUND, "No audit on this host.");
    }
    let mut command = tokio::process::Command::new(&python);
    command
        .arg("-I")
        .arg(AUDIT_SCRIPT_PATH)
        .args(&args)
        .current_dir(&home)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let timeout = std::time::Duration::from_secs(AUDIT_ACTION_TIMEOUT_SECS);
    let output = match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            tracing::error!("audit {:?}: spawn failed: {}", args.first(), e);
            return audit_answer(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Audit script failed to start.",
            );
        }
        Err(_) => {
            return audit_answer(
                StatusCode::GATEWAY_TIMEOUT,
                "Audit script did not answer in time.",
            )
        }
    };
    if !output.stderr.is_empty() {
        tracing::warn!(
            "audit {:?} stderr: {}",
            args.first(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let status = if output.status.success() {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    };
    tracing::info!("audit {:?} from the portal: {}", args.first(), message);
    audit_answer(status, message)
}

/// `POST /v1/monitoring/audit/stop`: SIGTERM through the audit script, which checks the pid
/// really runs the audit; the run then publishes `"stopped"` and can be resumed.
pub async fn monitoring_audit_stop_handler() -> AuditAction {
    run_audit_script(vec!["stop".to_string()]).await
}

/// `POST /v1/monitoring/audit/resume`: restarts the published run where it stopped.
///
/// The script's lock refuses a second audit, so two clicks never start two runs.
pub async fn monitoring_audit_resume_handler() -> AuditAction {
    match audit_resume_args(&load_audit_status()) {
        Ok(args) => run_audit_script(args).await,
        Err(message) => audit_answer(StatusCode::CONFLICT, message),
    }
}

fn extract_media_name_flag(parts: &[&str]) -> Option<String> {
    let idx = parts.iter().position(|&p| p == "--media-name")?;
    let name = *parts.get(idx + 1)?;
    if !name.starts_with('-') && !name.is_empty() {
        Some(name.to_string())
    } else {
        None
    }
}

fn extract_video_filename_stem(parts: &[&str]) -> Option<String> {
    for &part in parts {
        let is_video = part.ends_with(".mkv") || part.ends_with(".mp4") || part.ends_with(".avi");
        if is_video {
            if let Some(stem) = Path::new(part).file_stem().and_then(|s| s.to_str()) {
                return Some(stem.to_string());
            }
        }
    }
    None
}

/// Extracts media stem from process command line string.
fn extract_media_stem_from_cmdline(cmdline: &str) -> String {
    let parts: Vec<&str> = cmdline.split('\0').collect();
    if let Some(name) = extract_media_name_flag(&parts) {
        return name;
    }
    extract_video_filename_stem(&parts).unwrap_or_else(|| "unknown_video".to_string())
}

/// Reads start ticks and parent PID (PPID) from `/proc/<pid>/stat`.
fn read_proc_stat_info(pid: u32) -> Option<(u32, u64)> {
    let content = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let r_idx = content.rfind(')')?;
    let rest = &content[r_idx + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 20 {
        return None;
    }
    let ppid: u32 = fields[1].parse().ok()?;
    let start_ticks: u64 = fields[19].parse().ok()?;
    Some((ppid, start_ticks))
}

/// Scans active video decoding worker processes and parses their progress payloads.
///
/// # Arguments
/// None.
///
/// # Returns
/// A list of [`ActiveJobStatus`] entries for all live extraction jobs.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads active job progress files in `DEFAULT_JOBS_DIR`. Validates that the recorded PID
/// actually exists in `/proc` and belongs to the current execution run.
pub fn sample_active_jobs() -> Vec<ActiveJobStatus> {
    let jobs_dir = resolve_monitoring_path(DEFAULT_JOBS_DIR);
    let Ok(entries) = fs::read_dir(&jobs_dir) else {
        return Vec::new();
    };

    let mut active = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(pid) = stem.parse::<u32>() else {
            continue;
        };

        let Some((ppid, live_ticks)) = read_proc_stat_info(pid) else {
            continue;
        };

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };

        let file_start_ticks = val.get("start_ticks").and_then(|v| v.as_u64());
        if !dbsrt_core::job_progress::progress_belongs_to_process(
            file_start_ticks,
            Some(live_ticks),
        ) {
            continue;
        }

        let frames = val.get("frames").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let total_frames = val
            .get("total_frames")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let fps = val.get("fps").and_then(|v| v.as_f64()).unwrap_or(24.0);

        let cmdline = fs::read_to_string(format!("/proc/{ppid}/cmdline"))
            .or_else(|_| fs::read_to_string(format!("/proc/{pid}/cmdline")))
            .unwrap_or_default();
        let media_name = extract_media_stem_from_cmdline(&cmdline);

        let progress_ratio = match total_frames {
            Some(total) if total > 0 => (frames as f64 / total as f64).clamp(0.0, 1.0),
            _ => (frames as f64 / (DEFAULT_ESTIMATED_MEDIA_DURATION_SECS * fps)).clamp(0.0, 0.99),
        };

        let eta_secs = if fps > 0.0 {
            total_frames.map(|total| total.saturating_sub(frames) as f64 / fps)
        } else {
            None
        };

        active.push(ActiveJobStatus {
            pid,
            media_name,
            frames_decoded: frames,
            total_frames,
            fps,
            elapsed_secs: 0.0,
            progress_ratio,
            eta_secs,
            cpu_percent: 100.0,
            status: "RUNNING".to_string(),
        });
    }

    active.sort_by_key(|a| a.pid);
    active
}

/// Reads autonomous supervisor state and heartbeat metrics.
///
/// # Arguments
/// None.
///
/// # Returns
/// Instantiated [`SupervisorTelemetry`] struct with online status flag.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads `DEFAULT_SUPERVISOR_STATUS_PATH`. Checks heartbeat age against
/// `SUPERVISOR_STALE_THRESHOLD_SECS` to detect crashed or disconnected supervisors.
pub fn load_supervisor_status() -> SupervisorTelemetry {
    let path = resolve_monitoring_path(DEFAULT_SUPERVISOR_STATUS_PATH);
    let Ok(content) = fs::read_to_string(&path) else {
        return SupervisorTelemetry {
            supervisor_host: "unknown".to_string(),
            state: "offline".to_string(),
            is_online: false,
            ..Default::default()
        };
    };

    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        return SupervisorTelemetry {
            state: "corrupted".to_string(),
            is_online: false,
            ..Default::default()
        };
    };

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let unix_ts = val.get("unix_ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let heartbeat_age = now_ts.saturating_sub(unix_ts);
    let is_online = heartbeat_age <= SUPERVISOR_STALE_THRESHOLD_SECS;

    let host = val
        .get("supervisor_host")
        .and_then(|v| v.as_str())
        .unwrap_or("rpi4");
    let pid = val
        .get("supervisor_pid")
        .and_then(|v| v.as_u64())
        .map(|p| p as u32);
    let state_str = val
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("healthy");

    let remote_batch = val.get("remote_batch");
    let lve_cpu_pct = remote_batch
        .and_then(|b| b.get("lve_cpu_pct"))
        .and_then(|v| v.as_f64());
    let lve_cpu_avg_pct = remote_batch
        .and_then(|b| b.get("avg_cpu_pct").or_else(|| b.get("lve_cpu_avg_pct")))
        .and_then(|v| v.as_f64());
    let desired_workers = remote_batch
        .and_then(|b| b.get("desired_workers"))
        .and_then(|v| v.as_u64())
        .map(|w| w as usize);
    let running_workers = remote_batch
        .and_then(|b| b.get("running_jobs").or_else(|| b.get("running_workers")))
        .and_then(|v| v.as_u64())
        .map(|w| w as usize);

    let recent_logs: Vec<String> = val
        .get("recent_logs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.as_str().map(dbsrt_core::redact_secrets))
                .collect()
        })
        .unwrap_or_default();

    SupervisorTelemetry {
        supervisor_host: host.to_string(),
        supervisor_pid: pid,
        state: state_str.to_string(),
        is_online,
        last_heartbeat_iso: val
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        heartbeat_age_secs: heartbeat_age,
        lve_cpu_pct,
        lve_cpu_avg_pct,
        desired_workers,
        running_workers,
        last_backup_iso: val
            .get("last_backup_time")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        last_sync_iso: val
            .get("last_sync_time")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        recent_logs,
    }
}

/// Reads the last N completed event records from the batch report log.
///
/// # Arguments
/// None.
///
/// # Returns
/// A vector of [`RecentBatchEvent`] ordered newest first.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads up to `MAX_MONITORING_RECENT_EVENTS` lines from `DEFAULT_BATCH_EVENTS_PATH`
/// or `FALLBACK_BATCH_EVENTS_PATH`.
pub fn load_recent_events() -> Vec<RecentBatchEvent> {
    let primary = resolve_monitoring_path(DEFAULT_BATCH_EVENTS_PATH);
    let path = if primary.exists() {
        primary
    } else {
        resolve_monitoring_path(FALLBACK_BATCH_EVENTS_PATH)
    };

    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };

    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
    let mut events = Vec::new();

    for line in lines.iter().rev().take(MAX_MONITORING_RECENT_EVENTS) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };

        let video_stem = v
            .get("video_stem")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_string();
        let file_size_bytes = v
            .get("file_size_bytes")
            .and_then(|s| s.as_u64())
            .unwrap_or(0);
        let wall_duration_secs = v
            .get("wall_duration_secs")
            .and_then(|s| s.as_f64())
            .unwrap_or(0.0);
        let rate_mib_s = v
            .get("rate_mib_s")
            .or_else(|| v.get("io_read_rate_mib_s"))
            .or_else(|| v.get("cifs_read_rate_mib_s"))
            .and_then(|s| s.as_f64())
            .unwrap_or(0.0);
        let status = v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("COMPLETED")
            .to_string();
        let success = v.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
        let timestamp_iso = v
            .get("timestamp_iso")
            .or_else(|| v.get("end_iso"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        events.push(RecentBatchEvent {
            video_stem,
            file_size_bytes,
            wall_duration_secs,
            rate_mib_s,
            status,
            success,
            timestamp_iso,
        });
    }

    events
}

/// Aggregates live system, batch queue, worker jobs, supervisor, and event telemetry.
///
/// # Arguments
/// * `state` - Shared [`AppState`] reference.
///
/// # Returns
/// Unified [`MonitoringTelemetryResponse`] encoded as JSON.
///
/// # Errors
/// None.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Gathers a unified snapshot of volatile host state, batch queue, and supervisor tracking.
///
/// # Arguments
/// None.
///
/// # Returns
/// A [`MonitoringTelemetryResponse`] instance populated with current host and pipeline metrics.
///
/// # Errors
/// Infallible (missing or corrupt files fall back to nominal defaults).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Samples volatile Linux host state, batch queue, active processes, and supervisor heartbeats
/// in sub-millisecond execution time without holding long locks.
pub fn sample_monitoring_telemetry() -> MonitoringTelemetryResponse {
    let now_iso = current_time_iso();
    let system = sample_system_stats();
    let batch_queue = load_batch_queue_status();
    let active_jobs = sample_active_jobs();
    let supervisor = load_supervisor_status();
    let recent_events = load_recent_events();
    let audit = load_audit_status();

    MonitoringTelemetryResponse {
        system,
        batch_queue,
        active_jobs,
        supervisor,
        recent_events,
        audit,
        audit_last_finished: load_last_finished_audit(),
        server_time_iso: now_iso,
    }
}

/// Builds an HTTP header map containing the server Unix epoch timestamp in milliseconds.
///
/// # Arguments
/// None.
///
/// # Returns
/// [`axum::http::HeaderMap`] containing `X-Server-Time-Ms`.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Samples [`std::time::SystemTime::now()`] and formats milliseconds as decimal ASCII.
fn server_time_header() -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    if let Ok(duration) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        if let Ok(val) = axum::http::HeaderValue::from_str(&duration.as_millis().to_string()) {
            headers.insert(axum::http::HeaderName::from_static("x-server-time-ms"), val);
        }
    }
    headers
}

/// Serves live system and queue telemetry via HTTP GET with server clock sync.
///
/// # Arguments
/// * `_state` - Axum application state.
///
/// # Returns
/// Tuple containing [`axum::http::HeaderMap`] with `X-Server-Time-Ms` and [`Json<MonitoringTelemetryResponse>`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Attaches [`server_time_header`] and samples telemetry via [`sample_monitoring_telemetry`].
pub async fn monitoring_telemetry_handler(
    State(_state): State<Arc<AppState>>,
) -> (axum::http::HeaderMap, Json<MonitoringTelemetryResponse>) {
    (server_time_header(), Json(sample_monitoring_telemetry()))
}

/// Reuse window for the `dbsrt top --json` snapshot shared by concurrent web clients.
const TOP_SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(15);

/// Fast telemetry sampling window in milliseconds passed to `dbsrt top --json`.
const TOP_CLI_FAST_REFRESH_MS: &str = "200";

/// Cached `dbsrt top --json` output and the instant it was captured.
pub type TopSnapshotCache = tokio::sync::Mutex<Option<(std::time::Instant, axum::body::Bytes)>>;

/// Last native snapshot; the lock is held across the spawn so only one probe runs at a time.
static TOP_SNAPSHOT_CACHE: TopSnapshotCache = tokio::sync::Mutex::const_new(None);

/// Atomic flag tracking whether a background cache revalidation task is already active.
static TOP_PROBE_IN_FLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Spawns `dbsrt top --json --refresh-ms 200` to probe a full system telemetry snapshot.
///
/// # Arguments
/// * `server_exe` - Filesystem path to the current server executable.
///
/// # Returns
/// Raw JSON bytes redacted of sensitive credentials.
///
/// # Errors
/// * [`StatusCode::INTERNAL_SERVER_ERROR`] when the binary cannot be spawned or environment paths fail.
/// * [`StatusCode::BAD_GATEWAY`] when the process exits non-zero.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Executes the CLI binary with fast sampling (200ms) and inherits directory PATH so relative
/// Zero-DB state files resolve identically.
async fn execute_top_probe(
    server_exe: &std::path::Path,
) -> Result<axum::body::Bytes, (StatusCode, String)> {
    let cli = server_exe.with_file_name("dbsrt");
    let mut command = tokio::process::Command::new(&cli);
    command
        .args(["top", "--json", "--refresh-ms", TOP_CLI_FAST_REFRESH_MS])
        .kill_on_drop(true);
    if let Some(dir) = server_exe.parent() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(
            std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&path)),
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        command.env("PATH", joined);
    }
    let output = command.output().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn {}: {e}", cli.display()),
        )
    })?;
    if !output.status.success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            dbsrt_core::redact_url_userinfo(&String::from_utf8_lossy(&output.stderr)),
        ));
    }

    Ok(axum::body::Bytes::from(dbsrt_core::redact_url_userinfo(
        &String::from_utf8_lossy(&output.stdout),
    )))
}

/// Launches a background tokio task to revalidate the `dbsrt top` snapshot without blocking.
///
/// # Arguments
/// * `exe` - Server executable path used to locate the sibling `dbsrt` CLI.
///
/// # Returns
/// None. Task runs detached and clears the in-flight atomic flag upon completion.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Ensures strictly one background probe executes concurrently via [`TOP_PROBE_IN_FLIGHT`].
fn spawn_top_probe_background(exe: std::path::PathBuf) {
    if TOP_PROBE_IN_FLIGHT.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        if let Ok(fresh) = execute_top_probe(&exe).await {
            let mut g = TOP_SNAPSHOT_CACHE.lock().await;
            *g = Some((std::time::Instant::now(), fresh));
        }
        TOP_PROBE_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

/// Runs `dbsrt top --json` next to `server_exe`, reusing `cache` with stale-while-revalidate.
///
/// # Arguments
/// * `server_exe` - Filesystem path to the server binary.
/// * `cache` - Shared locked cache holding the latest snapshot.
///
/// # Returns
/// Redacted JSON bytes of the snapshot.
///
/// # Errors
/// Returns error on initial cold-start if binary fails to execute.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Implements non-blocking Stale-While-Revalidate: if a previous snapshot is cached but expired,
/// it is returned immediately to the caller in 0ms while a detached tokio task refreshes the
/// cache in the background without blocking the HTTP request thread.
pub async fn top_snapshot(
    server_exe: &std::path::Path,
    cache: &TopSnapshotCache,
) -> Result<axum::body::Bytes, (StatusCode, String)> {
    let mut guard = cache.lock().await;
    if let Some((at, body)) = guard.as_ref() {
        if at.elapsed() < TOP_SNAPSHOT_TTL {
            return Ok(body.clone());
        }
    }

    if let Some((_, stale_body)) = guard.as_ref() {
        let stale = stale_body.clone();
        spawn_top_probe_background(server_exe.to_path_buf());
        return Ok(stale);
    }

    let fresh = execute_top_probe(server_exe).await?;
    *guard = Some((std::time::Instant::now(), fresh.clone()));
    Ok(fresh)
}

/// Lists of `dbsrt top --json` paginated by `/v1/monitoring/top`, each with its total field.
const TOP_PAGED_LISTS: [(&str, &str); 2] = [
    ("history_records", "history_total"),
    ("discovered_media", "discovered_media_total"),
];

/// Keeps `limit` items from `offset` in each list of [`TOP_PAGED_LISTS`] and adds its total.
///
/// Lists are first put in the default order of the `dbsrt top` tabs: newest history record
/// first, media by case-insensitive name. `discovered_media_per_source` counts media per
/// `source_id` over the whole list.
///
/// # Errors
/// [`StatusCode::BAD_GATEWAY`] when `body` is not JSON.
pub fn paginate_top_snapshot(
    body: &[u8],
    offset: usize,
    limit: usize,
) -> Result<axum::body::Bytes, (StatusCode, String)> {
    let mut snapshot: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("dbsrt top --json: {e}")))?;
    if let Some(fields) = snapshot.as_object_mut() {
        if let Some(serde_json::Value::Array(records)) = fields.get_mut("history_records") {
            records.reverse();
        }
        if let Some(serde_json::Value::Array(media)) = fields.get_mut("discovered_media") {
            media.sort_by_cached_key(|m| m["stem"].as_str().unwrap_or_default().to_lowercase());
            let mut per_source = serde_json::Map::new();
            for id in media.iter().filter_map(|m| m["source_id"].as_str()) {
                let count = per_source
                    .get(id)
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                per_source.insert(id.to_string(), (count + 1).into());
            }
            fields.insert("discovered_media_per_source".to_string(), per_source.into());
        }
        for (list, total) in TOP_PAGED_LISTS {
            if let Some(serde_json::Value::Array(items)) = fields.get_mut(list) {
                let len = items.len();
                *items = items.drain(..).skip(offset).take(limit).collect();
                fields.insert(total.to_string(), len.into());
            }
        }
    }
    serde_json::to_vec(&snapshot)
        .map(Into::into)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Serves the `SystemTelemetry` rendered by the native `dbsrt top`, heavy lists paginated.
///
/// # Arguments
/// * `_state` - Axum application state.
/// * `page` - Query parameters specifying pagination offset and limit.
///
/// # Returns
/// HTTP response tuple containing headers (`Content-Type: application/json` and `X-Server-Time-Ms`)
/// and JSON bytes from the sibling `dbsrt top --json` binary.
///
/// # Errors
/// Same as [`top_snapshot`], plus [`StatusCode::INTERNAL_SERVER_ERROR`] when the server
/// executable path is unknown.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Samples [`top_snapshot`], paginates results, attaches [`server_time_header`], and returns response bytes.
pub async fn monitoring_top_handler(
    State(_state): State<Arc<AppState>>,
    Query(page): Query<TopPageQuery>,
) -> Result<(axum::http::HeaderMap, axum::body::Bytes), (StatusCode, String)> {
    let exe =
        std::env::current_exe().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let body = top_snapshot(&exe, &TOP_SNAPSHOT_CACHE).await?;
    let body = paginate_top_snapshot(
        &body,
        page.offset.unwrap_or(0),
        page.limit.unwrap_or(DEFAULT_TOP_PAGE_LIMIT),
    )?;
    let mut headers = server_time_header();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    Ok((headers, body))
}

/// Gathers a unified snapshot for real-time SSE streaming without heavy lists.
///
/// # Arguments
/// * `state` - Shared server state container.
///
/// # Returns
/// A [`MonitoringStreamState`] holding current telemetry, optional top snapshot, and pipeline timing.
///
/// # Errors
/// Infallible (defaults to `None` on sub-probe failure).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Samples live telemetry, queries cached `dbsrt top` snapshot without heavy lists
/// (offset 0, limit 0), and reads the latest search pipeline timing metrics.
pub async fn sample_monitoring_stream_state(state: &AppState) -> MonitoringStreamState {
    let sent_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let telemetry = sample_monitoring_telemetry();
    let top = match std::env::current_exe() {
        Ok(exe) => match top_snapshot(&exe, &TOP_SNAPSHOT_CACHE).await {
            Ok(body) => match paginate_top_snapshot(&body, 0, 0) {
                Ok(bytes) => serde_json::from_slice(&bytes).ok(),
                Err(_) => None,
            },
            Err(_) => None,
        },
        Err(_) => None,
    };
    let pipeline = state.latest_search.read().ok().and_then(|g| g.clone());

    let total = state
        .metrics
        .total_requests
        .load(std::sync::atomic::Ordering::Relaxed);
    let total_us = state
        .metrics
        .total_latency_us
        .load(std::sync::atomic::Ordering::Relaxed);
    let avg_latency_ms = if total > 0 {
        (total_us as f64 / total as f64) / 1000.0
    } else {
        0.0
    };
    let server_metrics = Some(MetricsResponse {
        status: "ok".to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        total_requests: total,
        recognize_requests: state
            .metrics
            .recognize_requests
            .load(std::sync::atomic::Ordering::Relaxed),
        search_requests: state
            .metrics
            .search_requests
            .load(std::sync::atomic::Ordering::Relaxed),
        ahash_search_requests: state
            .metrics
            .ahash_search_requests
            .load(std::sync::atomic::Ordering::Relaxed),
        cache_hits: state
            .metrics
            .cache_hits
            .load(std::sync::atomic::Ordering::Relaxed),
        remote_errors: state
            .metrics
            .remote_errors
            .load(std::sync::atomic::Ordering::Relaxed),
        avg_latency_ms,
    });
    let server_logs = state
        .log_buffer
        .read()
        .map(|b| b.iter().cloned().collect::<Vec<_>>())
        .ok();

    MonitoringStreamState {
        sent_at_ms,
        telemetry,
        top,
        pipeline,
        server_metrics,
        server_logs,
    }
}

/// Handles persistent Server-Sent Events (SSE) connections for live monitoring.
///
/// # Arguments
/// * `state` - Axum application state.
///
/// # Returns
/// An SSE stream pushing live telemetry and top metrics every [`DEFAULT_SSE_INTERVAL_MS`].
///
/// # Errors
/// Infallible (client disconnection drops stream).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Emits unified `state` events containing [`MonitoringStreamState`]. Axum automatically
/// manages HTTP keep-alive pings and stream teardown upon socket closure.
pub async fn monitoring_events_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = futures_util::stream::unfold(state, |state| async move {
        tokio::time::sleep(std::time::Duration::from_millis(DEFAULT_SSE_INTERVAL_MS)).await;
        let payload = sample_monitoring_stream_state(&state).await;
        let json_str = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        let event = Event::default().event("state").data(json_str);
        Some((Ok(event), state))
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(DEFAULT_SSE_KEEPALIVE_SECS))
            .text("keep-alive"),
    )
}

/// Receives and persists supervisor heartbeat payloads from RPi4 agent.
///
/// # Arguments
/// * `state` - Shared [`AppState`] reference.
/// * `payload` - Incoming [`SupervisorHeartbeatPayload`].
///
/// # Returns
/// HTTP [`StatusCode::OK`] on successful write.
///
/// # Errors
/// Returns [`StatusCode::INTERNAL_SERVER_ERROR`] if atomic write fails.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Atomically persists incoming heartbeat to `DEFAULT_SUPERVISOR_STATUS_PATH` using a temporary file.
pub async fn supervisor_heartbeat_handler(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<SupervisorHeartbeatPayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now_iso = current_time_iso();

    let json_data = serde_json::json!({
        "timestamp": now_iso,
        "unix_ts": now_ts,
        "supervisor_host": payload.supervisor_host,
        "supervisor_pid": payload.supervisor_pid,
        "state": payload.state,
        "remote_batch": {
            "lve_cpu_pct": payload.lve_cpu_pct,
            "avg_cpu_pct": payload.lve_cpu_avg_pct,
            "desired_workers": payload.desired_workers,
            "running_jobs": payload.running_workers,
        },
        // Log lines embed remote command output; mask secrets before they reach the dashboard
        "recent_logs": payload.recent_logs.iter().map(|l| dbsrt_core::redact_secrets(l)).collect::<Vec<_>>(),
    });

    let target_path = PathBuf::from(DEFAULT_SUPERVISOR_STATUS_PATH);
    if let Some(parent) = target_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp_path = target_path.with_extension("tmp");

    let serialized = serde_json::to_string_pretty(&json_data).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Serialization failure: {e}"),
        )
    })?;

    fs::write(&tmp_path, serialized).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Write failure: {e}"),
        )
    })?;

    fs::rename(&tmp_path, &target_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Atomic rename failure: {e}"),
        )
    })?;

    Ok(StatusCode::OK)
}
