//! Web Backoffice Monitoring endpoints providing live telemetry and supervisor tracking.

use crate::constants::{
    BYTES_PER_GIB_F64, CIVIL_DAYS_PER_400_YEARS, CIVIL_EPOCH_DAYS_OFFSET,
    DEFAULT_BATCH_EVENTS_PATH, DEFAULT_BATCH_QUEUE_PATH, DEFAULT_ESTIMATED_MEDIA_DURATION_SECS,
    DEFAULT_JOBS_DIR, DEFAULT_SUPERVISOR_STATUS_PATH, DEFAULT_TOP_PAGE_LIMIT,
    FALLBACK_BATCH_EVENTS_PATH, MAX_MONITORING_RECENT_EVENTS, SECONDS_PER_DAY, SECONDS_PER_HOUR,
    SECONDS_PER_MINUTE, SUPERVISOR_STALE_THRESHOLD_SECS,
};
use crate::dtos::{
    ActiveJobStatus, BatchQueueStatus, MonitoringTelemetryResponse, RecentBatchEvent,
    SupervisorHeartbeatPayload, SupervisorTelemetry, SystemStats, TopPageQuery,
};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::Json;
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
/// Samples volatile Linux host state, batch queue, active processes, and supervisor heartbeats
/// in sub-millisecond execution time without holding long locks.
pub async fn monitoring_telemetry_handler(
    State(_state): State<Arc<AppState>>,
) -> Json<MonitoringTelemetryResponse> {
    let now_iso = current_time_iso();
    let system = sample_system_stats();
    let batch_queue = load_batch_queue_status();
    let active_jobs = sample_active_jobs();
    let supervisor = load_supervisor_status();
    let recent_events = load_recent_events();

    Json(MonitoringTelemetryResponse {
        system,
        batch_queue,
        active_jobs,
        supervisor,
        recent_events,
        server_time_iso: now_iso,
    })
}

/// Reuse window for the `dbsrt top --json` snapshot shared by concurrent web clients.
const TOP_SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(2);

/// Cached `dbsrt top --json` output and the instant it was captured.
pub type TopSnapshotCache = tokio::sync::Mutex<Option<(std::time::Instant, axum::body::Bytes)>>;

/// Last native snapshot; the lock is held across the spawn so only one probe runs at a time.
static TOP_SNAPSHOT_CACHE: TopSnapshotCache = tokio::sync::Mutex::const_new(None);

/// Runs `dbsrt top --json` next to `server_exe`, reusing `cache` for [`TOP_SNAPSHOT_TTL`].
///
/// # Errors
/// [`StatusCode::INTERNAL_SERVER_ERROR`] when the binary cannot be spawned,
/// [`StatusCode::BAD_GATEWAY`] when it exits non-zero.
///
/// # Data Flow & Invariants
/// Runs the CLI with the server cwd and its directory first in `PATH`, so relative state paths
/// (`~/dbsrt` on o2switch) resolve identically to an interactive `dbsrt top`.
pub async fn top_snapshot(
    server_exe: &std::path::Path,
    cache: &TopSnapshotCache,
) -> Result<axum::body::Bytes, (StatusCode, String)> {
    let mut cache = cache.lock().await;
    if let Some((at, body)) = cache.as_ref() {
        if at.elapsed() < TOP_SNAPSHOT_TTL {
            return Ok(body.clone());
        }
    }

    let cli = server_exe.with_file_name("dbsrt");
    let mut command = tokio::process::Command::new(&cli);
    command.args(["top", "--json"]).kill_on_drop(true);
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

    // Defense in depth over every field (paths, error tails): no URL password leaves the server.
    // JSON-safe: an authority ends at '"' and ':'/'@' never sit inside an escape sequence.
    let body = axum::body::Bytes::from(dbsrt_core::redact_url_userinfo(&String::from_utf8_lossy(
        &output.stdout,
    )));
    *cache = Some((std::time::Instant::now(), body.clone()));
    Ok(body)
}

/// Lists of `dbsrt top --json` paginated by `/v1/monitoring/top`, each with its total field.
const TOP_PAGED_LISTS: [(&str, &str); 2] = [
    ("history_records", "history_total"),
    ("discovered_media", "discovered_media_total"),
];

/// Keeps `limit` items from `offset` in each list of [`TOP_PAGED_LISTS`] and adds its total.
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
/// # Returns
/// JSON from the sibling `dbsrt top --json` binary (see [`top_snapshot`]), with
/// `history_records` and `discovered_media` cut by [`paginate_top_snapshot`].
///
/// # Errors
/// Same as [`top_snapshot`], plus [`StatusCode::INTERNAL_SERVER_ERROR`] when the server
/// executable path is unknown.
pub async fn monitoring_top_handler(
    State(_state): State<Arc<AppState>>,
    Query(page): Query<TopPageQuery>,
) -> Result<
    (
        [(axum::http::header::HeaderName, &'static str); 1],
        axum::body::Bytes,
    ),
    (StatusCode, String),
> {
    let exe =
        std::env::current_exe().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let body = top_snapshot(&exe, &TOP_SNAPSHOT_CACHE).await?;
    let body = paginate_top_snapshot(
        &body,
        page.offset.unwrap_or(0),
        page.limit.unwrap_or(DEFAULT_TOP_PAGE_LIMIT),
    )?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    ))
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
