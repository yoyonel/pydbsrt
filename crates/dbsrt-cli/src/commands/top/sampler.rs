//! High-performance telemetry sampler decoupling process/kernel polling from TUI rendering.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

use rayon::prelude::*;
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate, System,
};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, BYTES_PER_MIB_F64, DEFAULT_BATCH_EVENTS_PATH, DEFAULT_COUPLINGS_PATH,
    DEFAULT_COUPLING_BASELINE_FPS, DEFAULT_HISTORY_SEED_PATH, DEFAULT_MEDIA_CACHE_PATH,
    DEFAULT_MEDIA_SCAN_INTERVAL_SECS, DEFAULT_MEDIA_SOURCES_PATH, DEFAULT_NAS_TVSHOW_PATH,
    DEFAULT_PHASH_DIR, DEFAULT_SELECTED_BATCH_REPORT_PATH, DEFAULT_SELECTED_BATCH_TARGETS_PATH,
    DEFAULT_SOURCE_ID_TVSHOW, DEFAULT_SOURCE_NAME_TVSHOW, DEFAULT_STREAM_EPISODE_DURATION_SECS,
    MAX_MEDIA_SCAN_LIMIT, MAX_SELECTED_BATCH_WORKERS, MIN_DELTA_TIME_FLOOR_SECS,
    MIN_PROGRESS_THRESHOLD, NAS_TVSHOW_DIR_ENV, PROC_NET_DEV_HEADER_LINES, PROC_NET_DEV_PATH,
    RAW_FRAME_BYTES, SCAN_STREAMING_CHUNK_SIZE, SESSION_WINDOW_SECS,
};
use crate::commands::top::model::{
    calculate_overall_stats, format_unix_to_iso, parse_iso_to_unix_secs, ActiveJobTelemetry,
    BatchQueueTelemetry, DiscoveredMediaItem, GpuTelemetry, HistoricalRecord, IngestionStatus,
    MediaSourceConfig, MediaSubtrackInfo, PhashFingerprintInfo, SessionThroughput, SystemTelemetry,
};

/// Shared handle for background GPU monitoring process.
static GPU_CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Global file size cache by stem to eliminate blocking CIFS network metadata calls.
static STEM_SIZE_CACHE: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Global video codec cache by stem to eliminate blocking CIFS network metadata calls.
static STEM_CODEC_CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Resolves the path to the events NDJSON log, falling back to history seed when live report is absent.
///
/// # Arguments
/// None.
///
/// # Returns
/// A [`PathBuf`] pointing to [`DEFAULT_BATCH_EVENTS_PATH`] or [`DEFAULT_HISTORY_SEED_PATH`].
///
/// # Errors
/// None.
///
/// # Panics
/// None.
///
/// # Data Flow & Invariants
/// Checks if [`DEFAULT_BATCH_EVENTS_PATH`] exists on disk; if so, uses it so local developer
/// batch telemetry is live. Otherwise falls back to repository history seed [`DEFAULT_HISTORY_SEED_PATH`].
pub fn resolve_events_log_path() -> PathBuf {
    let primary = PathBuf::from(DEFAULT_BATCH_EVENTS_PATH);
    if primary.exists() {
        primary
    } else {
        PathBuf::from(DEFAULT_HISTORY_SEED_PATH)
    }
}

/// Initializes the file size and video codec in-memory caches from reports and media cache.
///
/// # Arguments
/// None (initializes global static caches).
///
/// # Data Flow & Invariants
/// Pre-populates [`STEM_SIZE_CACHE`] and [`STEM_CODEC_CACHE`] by scanning
/// batch events log and `data/cache/discovered_media.json`.
/// Eliminates network latency spikes by avoiding blocking metadata lookups across remote CIFS shares.
/// Populates the media size lookup map from previous batch execution event logs.
///
/// # Arguments
/// * `path` - Path to the NDJSON batch events log.
/// * `map` - Mutable reference to the target stem -> size map.
///
/// # Returns
/// Nothing. Mutates `map` in place.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Reads line by line from `path`.
/// 2. Extracts `video_stem` and `file_size_bytes` from valid JSON payloads.
/// 3. Populates `map` with discovered values.
fn load_events_log_sizes(path: &Path, map: &mut HashMap<String, u64>) {
    let Ok(file) = fs::File::open(path) else {
        return;
    };
    let reader = BufReader::new(file);
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let (Some(stem), Some(size)) = (
            v.get("video_stem").and_then(|s| s.as_str()),
            v.get("file_size_bytes").and_then(|s| s.as_u64()),
        ) else {
            continue;
        };
        map.insert(stem.to_string(), size);
    }
}

/// Populates the media size lookup map from selected batch target manifests.
///
/// # Arguments
/// * `path` - Path to `selected_batch_targets.json`.
/// * `map` - Mutable reference to the target stem -> size map.
///
/// # Returns
/// Nothing. Mutates `map` in place.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Deserializes array of target dictionaries from `path`.
/// 2. Extracts `stem` and non-zero `size_bytes`.
/// 3. Registers stem sizes into `map`.
fn load_batch_target_sizes(path: &Path, map: &mut HashMap<String, u64>) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) else {
        return;
    };
    for item in arr {
        let (Some(stem), Some(size)) = (
            item.get("stem").and_then(|s| s.as_str()),
            item.get("size_bytes").and_then(|s| s.as_u64()),
        ) else {
            continue;
        };
        if size > 0 {
            map.insert(stem.to_string(), size);
        }
    }
}

pub fn preload_stem_size_cache() {
    let mut map = HashMap::new();
    let events_file = resolve_events_log_path();
    if events_file.exists() {
        load_events_log_sizes(&events_file, &mut map);
    }

    let batch_targets_file = Path::new("data/reports/selected_batch_targets.json");
    if batch_targets_file.exists() {
        load_batch_target_sizes(batch_targets_file, &mut map);
    }

    let mut codec_map = HashMap::new();
    let media_cache_file = Path::new(DEFAULT_MEDIA_CACHE_PATH);
    if media_cache_file.exists() {
        let cached = load_media_cache(media_cache_file);
        for (stem, item) in cached {
            if item.size_bytes > 0 {
                map.entry(stem.clone()).or_insert(item.size_bytes);
            }
            if let Some(codec) = item.video_codec {
                codec_map.insert(stem, codec);
            }
        }
    }

    let mut lock = STEM_SIZE_CACHE.lock().unwrap();
    *lock = Some(map);

    let mut c_lock = STEM_CODEC_CACHE.lock().unwrap();
    *c_lock = Some(codec_map);
}

/// Retrieves the cached video codecmoniker for a given media stem.
///
/// # Arguments
/// - `media_name`: Filename stem of the video (e.g. `"The.Office.US.S04E01"`).
///
/// # Returns
/// `Some(String)` if found in the pre-warmed cache, `None` otherwise.
pub fn get_file_codec(media_name: &str) -> Option<String> {
    let lock = STEM_CODEC_CACHE.lock().unwrap();
    if let Some(ref map) = *lock {
        if let Some(codec) = map.get(media_name) {
            return Some(codec.clone());
        }
    }
    None
}

/// Retrieves file size in bytes for a media item, consulting in-memory cache before touching filesystem.
///
/// # Arguments
/// - `media_name`: Filename stem of the video container.
/// - `target_path`: Optional direct filesystem path.
///
/// # Returns
/// File size in bytes, or `0` if unknown or inaccessible.
///
/// # Data Flow & Invariants
/// First checks [`STEM_SIZE_CACHE`]. If absent and `target_path` is not a remote CIFS share,
/// executes `fs::metadata`. For remote shares, avoids blocking synchronous stat calls.
pub fn get_file_size(media_name: &str, target_path: Option<&Path>) -> u64 {
    {
        let lock = STEM_SIZE_CACHE.lock().unwrap();
        if let Some(ref map) = *lock {
            if let Some(&size) = map.get(media_name) {
                return size;
            }
        }
    }

    if let Some(path) = target_path {
        let is_remote_mount = path.to_string_lossy().contains("Synology")
            || path.to_string_lossy().contains("Freebox");
        if !is_remote_mount {
            if let Ok(metadata) = fs::metadata(path) {
                let size = metadata.len();
                let mut lock = STEM_SIZE_CACHE.lock().unwrap();
                let map = lock.get_or_insert_with(HashMap::new);
                map.insert(media_name.to_string(), size);
                return size;
            }
        }
    }

    0
}

/// Spawns a background thread streaming real-time JSON telemetry from `intel_gpu_top`.
///
/// # Arguments
/// - `gpu_state`: Shared `GpuTelemetry` state container protected by `RwLock`.
/// - `stop_flag`: Atomic cancellation token.
///
/// # Data Flow & Invariants
/// Executes `/usr/bin/intel_gpu_top -J -s 1000`. Parses engine metrics (`Video/busy`,
/// `Render/3D/busy`, actual frequency, power wattage) and writes to `gpu_state`.
pub fn start_gpu_sampler(gpu_state: Arc<RwLock<GpuTelemetry>>, stop_flag: Arc<AtomicBool>) {
    let gpu_bin = Path::new("/usr/bin/intel_gpu_top");
    if !gpu_bin.exists() {
        return;
    }

    std::thread::spawn(move || {
        let child_res = Command::new(gpu_bin)
            .args(["-J", "-s", "1000"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();

        let mut child = match child_res {
            Ok(c) => c,
            Err(_) => return,
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => return,
        };

        {
            let mut lock = GPU_CHILD.lock().unwrap();
            *lock = Some(child);
        }

        let reader = BufReader::new(stdout);
        let mut buffer = String::new();

        for line_res in reader.lines() {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            let line = match line_res {
                Ok(l) => l,
                Err(_) => break,
            };

            buffer.push_str(&line);
            let trimmed = line.trim();
            if trimmed.starts_with('}') {
                let trim_chars: &[char] = &['[', ',', ' ', '\n', '\r'];
                let cleaned = buffer
                    .trim()
                    .trim_start_matches(trim_chars)
                    .trim_end_matches(trim_chars);

                if cleaned.starts_with('{') && cleaned.ends_with('}') {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(cleaned) {
                        let v_busy = json
                            .pointer("/engines/Video/busy")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let r_busy = json
                            .pointer("/engines/Render/3D/busy")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let freq = json
                            .pointer("/frequency/actual")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let power = json
                            .pointer("/power/GPU")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);

                        let mut g = gpu_state.write().unwrap();
                        g.video_busy_pct = v_busy;
                        g.render_busy_pct = r_busy;
                        g.freq_mhz = freq;
                        g.power_watts = power;
                        g.available = true;
                    }
                }
                buffer.clear();
            }
        }

        let mut lock = GPU_CHILD.lock().unwrap();
        if let Some(mut c) = lock.take() {
            let _ = c.kill();
        }
    });
}

/// Reads `/proc/{pid}/io` on Linux to extract cumulative `rchar` and `read_bytes`.
///
/// # Arguments
/// - `pid`: Linux process identifier to inspect.
///
/// # Returns
/// Volatile job progress snapshot deserialized from IPC state file.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JobProgressPayload {
    pub frames: usize,
    pub total_frames: Option<usize>,
    pub fps: Option<f64>,
    /// Writer process start time; absent in files from older writers.
    #[serde(default)]
    pub start_ticks: Option<u64>,
}

/// Reads the volatile IPC job progress file for an active decoding PID, ignoring files left by
/// a dead process or by an earlier process that had the same PID.
pub fn read_job_progress_file(pid: u32) -> Option<JobProgressPayload> {
    let dir = dbsrt_core::job_progress::jobs_run_dir();
    read_job_progress_in(
        &dir,
        pid,
        dbsrt_core::job_progress::process_start_ticks(pid),
    )
}

/// Reads `<dir>/<pid>.json` and keeps it only if it belongs to the process started at
/// `live_start_ticks`.
pub fn read_job_progress_in(
    dir: &Path,
    pid: u32,
    live_start_ticks: Option<u64>,
) -> Option<JobProgressPayload> {
    let path = dbsrt_core::job_progress::job_progress_path(dir, pid);
    let payload: JobProgressPayload = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    dbsrt_core::job_progress::progress_belongs_to_process(payload.start_ticks, live_start_ticks)
        .then_some(payload)
}

fn parse_proc_io_val(line: &str) -> Option<u64> {
    if line.starts_with("rchar:") || line.starts_with("read_bytes:") || line.starts_with("wchar:") {
        line.split_whitespace().nth(1)?.parse::<u64>().ok()
    } else {
        None
    }
}

/// Cumulative read bytes. Returns `0` if `/proc/[pid]/io` is missing or access is denied.
///
/// # Data Flow & Invariants
/// Network filesystem mounts (CIFS/SMB) increment virtual `rchar` rather than physical block `read_bytes`.
/// Direct socket streams (FTP/HTTP) decode uncompressed frames directly to stdout (`wchar`).
/// Computes `rchar.max(read_bytes).max(wchar)` to guarantee accurate throughput across local, CIFS, and streaming.
pub fn read_proc_io_bytes(pid: u32) -> u64 {
    let io_path = format!("/proc/{pid}/io");
    let mut max_bytes = 0u64;

    if let Ok(file) = fs::File::open(io_path) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Some(val) = parse_proc_io_val(&line) {
                max_bytes = max_bytes.max(val);
            }
        }
    }
    max_bytes
}

/// Checks if the given PID is a child thread task rather than a main process thread group leader.
///
/// # Arguments
/// - `pid`: Linux process / task identifier.
///
/// # Returns
/// - `true` if `Tgid != Pid` in `/proc/[pid]/status` (sub-thread).
/// - `false` if `Tgid == Pid` (main process leader).
pub fn is_thread_task(pid: u32) -> bool {
    if let Ok(content) = fs::read_to_string(format!("/proc/{pid}/status")) {
        let mut tgid = None;
        let mut p_pid = None;
        for line in content.lines() {
            if line.starts_with("Tgid:") {
                tgid = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u32>().ok());
            } else if line.starts_with("Pid:") {
                p_pid = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u32>().ok());
            }
            if tgid.is_some() && p_pid.is_some() {
                break;
            }
        }
        if let (Some(tg), Some(p)) = (tgid, p_pid) {
            return tg != p;
        }
    }
    false
}

/// Loads historical execution records from batch report NDJSON logs with certified couplings cross-referencing.
///
/// # Arguments
/// - `events_path`: Path to batch execution report (`data/reports/batch_tvshow_nightly.jsonl`).
/// - `couplings_path`: Path to Zero-DB certified couplings registry (`data/SRT/couplings.json`).
///
/// # Returns
/// Chronological vector of [`HistoricalRecord`] entries with deduped stems and accurate coupling counts.
///
/// # Data Flow & Invariants
/// 1. Reads events file line by line, parsing JSON record payloads.
/// 2. If `certified_couplings_count == 0` but coupling registry contains entries for the stem,
///    backfills coupling counts from ground-truth registry.
/// 3. Retains latest record per media stem to eliminate stale runs.
pub fn load_historical_records(events_path: &Path, couplings_path: &Path) -> Vec<HistoricalRecord> {
    let mut raw_records = Vec::new();
    if events_path.exists() {
        if let Ok(file) = fs::File::open(events_path) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if let Some(stem) = v.get("video_stem").and_then(|s| s.as_str()) {
                        let video_rel_path = v
                            .get("video_rel_path")
                            .and_then(|p| p.as_str())
                            .map(|s| s.to_string());
                        let error_summary = v
                            .get("error_summary")
                            .and_then(|e| e.as_str())
                            .map(|s| s.to_string());
                        let success = v.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
                        let wall_duration_secs = v
                            .get("wall_duration_secs")
                            .and_then(|w| w.as_f64())
                            .unwrap_or(0.0);
                        let rate = v
                            .get("cifs_read_rate_mib_s")
                            .or_else(|| v.get("io_read_rate_mib_s"))
                            .and_then(|r| r.as_f64())
                            .unwrap_or(0.0);
                        let file_size_bytes = v
                            .get("file_size_bytes")
                            .and_then(|s| s.as_u64())
                            .unwrap_or(0);
                        let certified_couplings_count =
                            v.get("certified_couplings_count")
                                .and_then(|c| c.as_u64())
                                .unwrap_or(0) as usize;
                        let start_iso = v
                            .get("start_iso")
                            .or_else(|| v.get("timestamp_iso"))
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        let end_iso = v
                            .get("end_iso")
                            .or_else(|| v.get("timestamp_iso"))
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());

                        raw_records.push(HistoricalRecord {
                            video_stem: stem.to_string(),
                            video_rel_path,
                            success,
                            wall_duration_secs,
                            cifs_read_rate_mib_s: rate,
                            file_size_bytes,
                            certified_couplings_count,
                            error_summary,
                            start_iso,
                            end_iso,
                        });
                    }
                }
            }
        }
    }

    // Deduplicate retaining latest record per video_stem
    let mut by_stem: HashMap<String, HistoricalRecord> = HashMap::new();
    for r in raw_records {
        by_stem.insert(r.video_stem.clone(), r);
    }

    // Fallback to couplings.json if no batch records exist
    if by_stem.is_empty() && couplings_path.exists() {
        if let Ok(content) = fs::read_to_string(couplings_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(couplings_arr) = v.get("couplings").and_then(|c| c.as_array()) {
                    for c in couplings_arr {
                        if let Some(media_name) = c.get("media_name").and_then(|m| m.as_str()) {
                            let frame_count =
                                c.get("frame_count").and_then(|f| f.as_f64()).unwrap_or(0.0);
                            let fps = c
                                .get("fps")
                                .and_then(|f| f.as_f64())
                                .unwrap_or(DEFAULT_COUPLING_BASELINE_FPS);
                            let dur_secs = if fps > 0.0 { frame_count / fps } else { 0.0 };
                            let subs_count = c
                                .get("subtitles")
                                .and_then(|s| s.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);

                            by_stem.insert(
                                media_name.to_string(),
                                HistoricalRecord {
                                    video_stem: media_name.to_string(),
                                    video_rel_path: None,
                                    success: true,
                                    wall_duration_secs: dur_secs,
                                    cifs_read_rate_mib_s: 0.0,
                                    file_size_bytes: 0,
                                    certified_couplings_count: subs_count,
                                    error_summary: None,
                                    start_iso: None,
                                    end_iso: None,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    let mut items: Vec<HistoricalRecord> = by_stem.into_values().collect();
    items.sort_by(|a, b| {
        let key_a = a
            .end_iso
            .as_deref()
            .or(a.start_iso.as_deref())
            .unwrap_or(&a.video_stem);
        let key_b = b
            .end_iso
            .as_deref()
            .or(b.start_iso.as_deref())
            .unwrap_or(&b.video_stem);
        key_a.cmp(key_b)
    });
    items
}

/// Computes global session throughput across recent batch records (within rolling 2 hours) and active jobs.
pub fn compute_session_throughput(
    history_records: &[HistoricalRecord],
    active_jobs: &[ActiveJobTelemetry],
    now_unix_secs: f64,
) -> SessionThroughput {
    let mut recent_session_records = Vec::new();
    for r in history_records {
        if let Some(ref end_iso) = r.end_iso {
            if let Some(end_unix) = parse_iso_to_unix_secs(end_iso) {
                let diff = now_unix_secs - end_unix;
                if (0.0..SESSION_WINDOW_SECS).contains(&diff) {
                    recent_session_records.push(r);
                }
            }
        }
    }

    let history_bytes: u64 = recent_session_records
        .iter()
        .map(|r| r.file_size_bytes)
        .sum();
    let active_bytes: u64 = active_jobs.iter().map(|j| j.read_bytes).sum();
    let session_bytes = history_bytes + active_bytes;

    let mut session_starts = Vec::new();
    for r in &recent_session_records {
        if let Some(ref start_iso) = r.start_iso {
            if let Some(start_unix) = parse_iso_to_unix_secs(start_iso) {
                session_starts.push(start_unix);
            }
        }
    }
    for j in active_jobs {
        session_starts.push(now_unix_secs - j.elapsed_secs);
    }

    let mut session_dur_secs = 0.0;
    let mut session_rate_mib_s = 0.0;
    if !session_starts.is_empty() {
        let earliest = session_starts.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        session_dur_secs = (now_unix_secs - earliest).max(1.0);
        session_rate_mib_s = (session_bytes as f64 / BYTES_PER_MIB_F64) / session_dur_secs;
    }

    let stream_count = recent_session_records.len() + active_jobs.len();

    SessionThroughput {
        session_bytes,
        session_duration_secs: session_dur_secs,
        session_rate_mib_s,
        stream_count,
    }
}

/// Recursively scans a directory for video files up to a maximum limit.
pub fn collect_video_files(dir: &Path, files: &mut Vec<PathBuf>, max_limit: usize) {
    if files.len() >= max_limit {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(Result::ok) {
            if files.len() >= max_limit {
                return;
            }
            let p = entry.path();
            if p.is_dir() {
                collect_video_files(&p, files, max_limit);
            } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if ext_lower == "mkv"
                    || ext_lower == "mp4"
                    || ext_lower == "webm"
                    || ext_lower == "avi"
                {
                    files.push(p);
                }
            }
        }
    }
}

/// Extracts the set of all certified media stems from couplings.json and events log.
pub fn get_certified_stems(events_path: &Path, couplings_path: &Path) -> HashSet<String> {
    let mut stems = HashSet::new();
    if events_path.exists() {
        if let Ok(file) = fs::File::open(events_path) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                    if v.get("success").and_then(|s| s.as_bool()).unwrap_or(false) {
                        if let Some(stem) = v.get("video_stem").and_then(|s| s.as_str()) {
                            stems.insert(stem.to_string());
                        }
                    }
                }
            }
        }
    }
    if couplings_path.exists() {
        if let Ok(content) = fs::read_to_string(couplings_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(arr) = v.get("couplings").and_then(|c| c.as_array()) {
                    for c in arr {
                        if let Some(m) = c.get("media_name").and_then(|m| m.as_str()) {
                            stems.insert(m.to_string());
                        }
                    }
                }
            }
        }
    }
    stems
}

/// Loads cached discovered media items from disk, enriching with local pHash metadata if present.
pub fn load_media_cache(cache_path: &Path) -> HashMap<String, DiscoveredMediaItem> {
    let mut map = HashMap::new();
    let phash_dir = Path::new(DEFAULT_PHASH_DIR);
    if cache_path.exists() {
        if let Ok(content) = fs::read_to_string(cache_path) {
            if let Ok(items) = serde_json::from_str::<Vec<DiscoveredMediaItem>>(&content) {
                for mut item in items {
                    if item.phash.is_none() {
                        let phash_file = phash_dir.join(format!("{}.phash", item.stem));
                        if let Ok(meta) = fs::metadata(&phash_file) {
                            let bytes = meta.len();
                            let frame_count = (bytes / 8) as usize;
                            item.phash = Some(PhashFingerprintInfo {
                                exists: true,
                                frame_count,
                                size_bytes: bytes,
                            });
                        }
                    }
                    map.insert(item.stem.clone(), item);
                }
            }
        }
    }
    map
}

/// Persists discovered media items into the disk cache.
pub fn save_media_cache(cache_path: &Path, items: &[DiscoveredMediaItem]) {
    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(items) {
        let _ = fs::write(cache_path, json);
    }
}

/// Inspects container metadata, duration, video codec, and subtitle tracks via ffprobe.
pub fn probe_media_file(path: &Path) -> (Option<f64>, Option<String>, Vec<MediaSubtrackInfo>) {
    let output = build_probe_media_command(path).output();

    if let Ok(out) = output {
        if out.status.success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                let duration_secs = v.pointer("/format/duration").and_then(|d| {
                    d.as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .or_else(|| d.as_f64())
                });

                let mut video_codec = None;
                let mut subtitles = Vec::new();

                if let Some(streams) = v.get("streams").and_then(|s| s.as_array()) {
                    for s in streams {
                        let ctype = s.get("codec_type").and_then(|c| c.as_str()).unwrap_or("");
                        let codec = s
                            .get("codec_name")
                            .and_then(|c| c.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        if ctype == "video" && video_codec.is_none() {
                            let profile = s.get("profile").and_then(|p| p.as_str()).unwrap_or("");
                            let pix_fmt = s.get("pix_fmt").and_then(|p| p.as_str()).unwrap_or("");
                            let display_codec = if codec == "h264"
                                && (profile.contains("High 10")
                                    || pix_fmt.contains("10le")
                                    || pix_fmt.contains("10"))
                            {
                                "h264 Hi10P".to_string()
                            } else {
                                codec
                            };
                            video_codec = Some(display_codec);
                        } else if ctype == "subtitle" {
                            let idx = s.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            let lang = s
                                .pointer("/tags/language")
                                .or_else(|| s.pointer("/tags/LANGUAGE"))
                                .and_then(|l| l.as_str())
                                .unwrap_or("und")
                                .to_string();
                            let title = s
                                .pointer("/tags/title")
                                .or_else(|| s.pointer("/tags/TITLE"))
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            subtitles.push(MediaSubtrackInfo {
                                index: idx,
                                lang,
                                codec,
                                title,
                            });
                        }
                    }
                }
                return (duration_secs, video_codec, subtitles);
            }
        }
    }
    (None, None, Vec::new())
}

/// Scans the NAS library directory, cross-referencing candidate media against certified couplings and active jobs.
pub fn scan_nas_tvshows(
    nas_root: &Path,
    couplings_path: &Path,
    events_path: &Path,
    phash_dir: &Path,
    active_jobs: &[ActiveJobTelemetry],
    cache_path: &Path,
) -> Vec<DiscoveredMediaItem> {
    let mut cached_map = load_media_cache(cache_path);
    let certified_stems = get_certified_stems(events_path, couplings_path);

    let mut discovered = Vec::new();
    let mut video_paths = Vec::new();

    if nas_root.exists() && nas_root.is_dir() {
        collect_video_files(nas_root, &mut video_paths, MAX_MEDIA_SCAN_LIMIT);
    }

    if video_paths.is_empty() && !cached_map.is_empty() {
        discovered = cached_map.into_values().collect();
    } else {
        for p in video_paths {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if stem.is_empty() {
                continue;
            }

            let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);

            // Re-use cached metadata if file size matches, otherwise probe
            let (dur, codec, subs) = if let Some(item) = cached_map.get(&stem) {
                if item.size_bytes == size && item.duration_secs.is_some() {
                    (
                        item.duration_secs,
                        item.video_codec.clone(),
                        item.subtitles.clone(),
                    )
                } else {
                    probe_media_file(&p)
                }
            } else {
                probe_media_file(&p)
            };

            let phash_path = phash_dir.join(format!("{stem}.phash"));
            let phash_info = if let Ok(meta) = fs::metadata(&phash_path) {
                let bytes = meta.len();
                let frame_count = (bytes / 8) as usize;
                Some(PhashFingerprintInfo {
                    exists: true,
                    frame_count,
                    size_bytes: bytes,
                })
            } else {
                None
            };

            let status = if active_jobs.iter().any(|j| j.media_name == stem) {
                IngestionStatus::InProgress
            } else if certified_stems.contains(&stem) {
                IngestionStatus::Completed
            } else if phash_info.is_some() {
                IngestionStatus::PartialFail
            } else {
                IngestionStatus::Virgin
            };

            let item = DiscoveredMediaItem {
                path: p,
                stem: stem.clone(),
                size_bytes: size,
                duration_secs: dur,
                video_codec: codec,
                subtitles: subs,
                phash: phash_info,
                source_id: Some(DEFAULT_SOURCE_ID_TVSHOW.to_string()),
                status,
            };

            cached_map.insert(stem, item.clone());
            discovered.push(item);
        }

        save_media_cache(cache_path, &discovered);
    }

    if let Ok(mut c_lock) = STEM_CODEC_CACHE.lock() {
        let map = c_lock.get_or_insert_with(HashMap::new);
        for item in &discovered {
            if let Some(ref codec) = item.video_codec {
                map.insert(item.stem.clone(), codec.clone());
            }
        }
    }

    discovered.sort_by_key(|a| a.stem.to_lowercase());
    discovered
}

/// Global trigger flag to request an immediate asynchronous rescan across all media sources.
pub static MEDIA_RESCAN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Global flag signaling whether media library scanner is actively traversing disks and probing containers.
pub static IS_MEDIA_SCANNING: AtomicBool = AtomicBool::new(false);

/// Signals background scanner thread to trigger an immediate multi-source scan.
pub fn trigger_media_sources_rescan() {
    IS_MEDIA_SCANNING.store(true, Ordering::SeqCst);
    MEDIA_RESCAN_REQUESTED.store(true, Ordering::Relaxed);
}

/// Resolves the default NAS TVShow directory path dynamically without hardcoded machine paths.
///
/// # Arguments
/// None.
///
/// # Returns
/// A [`PathBuf`] determined by environment variable, dynamic user mount, or fallback constant.
///
/// # Errors
/// None.
///
/// # Panics
/// None.
///
/// # Data Flow & Invariants
/// 1. Prioritizes `NAS_TVSHOW_DIR` environment variable if present.
/// 2. Checks `/media/$USER/Synology/tvshow` if that directory actually exists on disk.
/// 3. Falls back to [`DEFAULT_NAS_TVSHOW_PATH`] (`/media/storage/tvshow`).
pub fn resolve_default_nas_tvshow_path() -> PathBuf {
    if let Ok(dir) = std::env::var(NAS_TVSHOW_DIR_ENV) {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(user) = std::env::var("USER") {
        let user_synology = PathBuf::from(format!("/media/{}/Synology/tvshow", user));
        if user_synology.exists() {
            return user_synology;
        }
    }
    PathBuf::from(DEFAULT_NAS_TVSHOW_PATH)
}

/// Loads configured media sources from disk, initializing default NAS TVShow source if unconfigured.
pub fn load_media_sources(path: &Path) -> Vec<MediaSourceConfig> {
    if path.exists() {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(sources) = serde_json::from_str::<Vec<MediaSourceConfig>>(&content) {
                if !sources.is_empty() {
                    return sources;
                }
            }
        }
    }

    let default_path = resolve_default_nas_tvshow_path();

    let default_sources = vec![MediaSourceConfig {
        id: DEFAULT_SOURCE_ID_TVSHOW.to_string(),
        name: DEFAULT_SOURCE_NAME_TVSHOW.to_string(),
        path: default_path,
        enabled: true,
        media_count: 0,
        last_scan_added: 0,
        last_scan_iso: None,
    }];

    let _ = save_media_sources(path, &default_sources);
    default_sources
}

/// Persists configured media sources to disk.
pub fn save_media_sources(path: &Path, sources: &[MediaSourceConfig]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(sources)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(path, json)
}

/// Merges scan metrics (media_count, last_scan_added, last_scan_iso) into target sources without modifying user-configured flags (enabled, name, path).
pub fn merge_scanned_source_metrics(
    target_sources: &mut [MediaSourceConfig],
    scanned_sources: &[MediaSourceConfig],
) {
    for target in target_sources.iter_mut() {
        if let Some(scanned) = scanned_sources.iter().find(|s| s.id == target.id) {
            target.media_count = scanned.media_count;
            target.last_scan_added = scanned.last_scan_added;
            if scanned.last_scan_iso.is_some() {
                target.last_scan_iso = scanned.last_scan_iso.clone();
            }
        }
    }
}

/// Adds a new media source to the configuration.
pub fn add_media_source(
    sources_path: &Path,
    sources: &mut Vec<MediaSourceConfig>,
    name: String,
    path: PathBuf,
) -> Result<MediaSourceConfig, String> {
    if path.as_os_str().is_empty() {
        return Err("Le chemin du répertoire ne peut pas être vide.".to_string());
    }

    let id_base = if !name.trim().is_empty() {
        name.trim()
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric(), "_")
    } else {
        path.file_name()
            .map(|s| {
                s.to_string_lossy()
                    .to_lowercase()
                    .replace(|c: char| !c.is_alphanumeric(), "_")
            })
            .unwrap_or_else(|| "source".to_string())
    };

    let mut id = id_base.clone();
    let mut counter = 1;
    while sources.iter().any(|s| s.id == id) {
        id = format!("{id_base}_{counter}");
        counter += 1;
    }

    let display_name = if !name.trim().is_empty() {
        name.trim().to_string()
    } else {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Nouvelle Source".to_string())
    };

    let new_source = MediaSourceConfig {
        id,
        name: display_name,
        path,
        enabled: true,
        media_count: 0,
        last_scan_added: 0,
        last_scan_iso: None,
    };

    sources.push(new_source.clone());
    let _ = save_media_sources(sources_path, sources);
    trigger_media_sources_rescan();
    Ok(new_source)
}

/// Updates an existing media source's name and path.
pub fn update_media_source(
    sources_path: &Path,
    sources: &mut [MediaSourceConfig],
    index: usize,
    name: String,
    path: PathBuf,
) -> Result<MediaSourceConfig, String> {
    if path.as_os_str().is_empty() {
        return Err("Le chemin du répertoire ne peut pas être vide.".to_string());
    }
    if let Some(src) = sources.get_mut(index) {
        if !name.trim().is_empty() {
            src.name = name.trim().to_string();
        }
        src.path = path;
        let updated = src.clone();
        let _ = save_media_sources(sources_path, sources);
        trigger_media_sources_rescan();
        Ok(updated)
    } else {
        Err("Index de source introuvable.".to_string())
    }
}

/// Deletes a media source by index.
pub fn delete_media_source(
    sources_path: &Path,
    sources: &mut Vec<MediaSourceConfig>,
    index: usize,
) -> Result<MediaSourceConfig, String> {
    if sources.len() <= 1 {
        return Err("Impossible de supprimer la seule source restante.".to_string());
    }
    if index >= sources.len() {
        return Err("Index de source invalide.".to_string());
    }
    let removed = sources.remove(index);
    let _ = save_media_sources(sources_path, sources);
    trigger_media_sources_rescan();
    Ok(removed)
}

/// Toggles enabled state of a media source by index.
pub fn toggle_media_source(
    sources_path: &Path,
    sources: &mut [MediaSourceConfig],
    index: usize,
) -> Option<bool> {
    if let Some(src) = sources.get_mut(index) {
        src.enabled = !src.enabled;
        let new_state = src.enabled;
        let _ = save_media_sources(sources_path, sources);
        trigger_media_sources_rescan();
        Some(new_state)
    } else {
        None
    }
}

/// Parameters for discovering metadata of an individual candidate media file.
pub struct MediaDiscoveryCriteria<'a> {
    pub phash_dir: &'a Path,
    pub active_jobs: &'a [ActiveJobTelemetry],
    pub certified_stems: &'a std::collections::HashSet<String>,
    pub source_id: &'a str,
}

/// Discovers metadata, codec, duration, and fingerprint status for a single candidate video file.
///
/// # Arguments
/// * `p` - Candidate video file path on disk.
/// * `cached_map` - Existing in-memory cache of previously probed media items.
/// * `criteria` - Static discovery criteria holding paths, active jobs, and source identity.
///
/// # Returns
/// `Some(DiscoveredMediaItem)` on successful discovery, `None` if filename stem is empty.
fn discover_single_media_file(
    p: &Path,
    cached_map: &HashMap<String, DiscoveredMediaItem>,
    criteria: &MediaDiscoveryCriteria,
) -> Option<DiscoveredMediaItem> {
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if stem.is_empty() {
        return None;
    }

    let size = fs::metadata(p).map(|m| m.len()).unwrap_or(0);

    let (dur, codec, subs) = if let Some(item) = cached_map.get(&stem) {
        if item.size_bytes == size && item.duration_secs.is_some() {
            (
                item.duration_secs,
                item.video_codec.clone(),
                item.subtitles.clone(),
            )
        } else {
            probe_media_file(p)
        }
    } else {
        probe_media_file(p)
    };

    let phash_path = criteria.phash_dir.join(format!("{stem}.phash"));
    let phash_info = if let Ok(meta) = fs::metadata(&phash_path) {
        let bytes = meta.len();
        let frame_count = (bytes / 8) as usize;
        Some(PhashFingerprintInfo {
            exists: true,
            frame_count,
            size_bytes: bytes,
        })
    } else {
        None
    };

    let status = if criteria.active_jobs.iter().any(|j| j.media_name == stem) {
        IngestionStatus::InProgress
    } else if criteria.certified_stems.contains(&stem) {
        IngestionStatus::Completed
    } else if phash_info.is_some() {
        IngestionStatus::PartialFail
    } else {
        IngestionStatus::Virgin
    };

    Some(DiscoveredMediaItem {
        path: p.to_path_buf(),
        stem,
        size_bytes: size,
        duration_secs: dur,
        video_codec: codec,
        subtitles: subs,
        phash: phash_info,
        source_id: Some(criteria.source_id.to_string()),
        status,
    })
}

/// Extracts a [`DiscoveredMediaItem`] from a single JSON element in `selected_batch_targets.json`.
///
/// # Arguments
/// * `val` - JSON value containing either a string path or a target object dictionary.
/// * `criteria` - Static discovery criteria holding paths, active jobs, and source identity.
///
/// # Returns
/// `Some(DiscoveredMediaItem)` if a valid stem could be resolved, `None` otherwise.
///
/// # Data Flow & Invariants
/// 1. Extracts path string, filename stem, and optional size from string or object dictionary.
/// 2. Probes existence of `.phash` fingerprint in `criteria.phash_dir`.
/// 3. Resolves ingestion status: InProgress if in `active_jobs`, Completed if in `certified_stems`,
///    PartialFail if `.phash` exists on disk, otherwise Virgin.
fn parse_batch_target_value(
    val: &serde_json::Value,
    criteria: &MediaDiscoveryCriteria,
) -> Option<DiscoveredMediaItem> {
    let (path_str, stem, size_bytes) = if let Some(s) = val.as_str() {
        let p = Path::new(s);
        let stem = p.file_stem()?.to_string_lossy().to_string();
        (s.to_string(), stem, 0)
    } else {
        let obj = val.as_object()?;
        let path = obj.get("path").and_then(|p| p.as_str())?;
        let stem = obj
            .get("stem")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            })?;
        let size = obj.get("size_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
        (path.to_string(), stem, size)
    };

    let phash_path = criteria.phash_dir.join(format!("{stem}.phash"));
    let phash_info = if let Ok(meta) = fs::metadata(&phash_path) {
        let bytes = meta.len();
        let frame_count = (bytes / 8) as usize;
        Some(PhashFingerprintInfo {
            exists: true,
            frame_count,
            size_bytes: bytes,
        })
    } else {
        None
    };

    let status = if criteria.active_jobs.iter().any(|j| j.media_name == stem) {
        IngestionStatus::InProgress
    } else if criteria.certified_stems.contains(&stem) {
        IngestionStatus::Completed
    } else if phash_info.is_some() {
        IngestionStatus::PartialFail
    } else {
        IngestionStatus::Virgin
    };

    Some(DiscoveredMediaItem {
        path: PathBuf::from(path_str),
        stem,
        size_bytes,
        duration_secs: None,
        video_codec: None,
        subtitles: Vec::new(),
        phash: phash_info,
        source_id: Some(criteria.source_id.to_string()),
        status,
    })
}

/// Loads candidate media files from the remote batch streaming targets manifest (`selected_batch_targets.json`).
///
/// # Arguments
/// * `targets_path` - Path to the JSON targets file.
/// * `criteria` - Static discovery criteria holding paths, active jobs, and source identity.
///
/// # Returns
/// A sorted [`Vec<DiscoveredMediaItem>`] containing all remote streaming batch entries.
///
/// # Panics
/// Guarantee: Never panics. Returns empty vector on missing file or parse failure.
///
/// # Data Flow & Invariants
/// Deserializes array of targets, maps each entry via [`parse_batch_target_value`],
/// and sorts items deterministically by stem (case-insensitive).
pub fn load_remote_batch_media_items(
    targets_path: &Path,
    criteria: &MediaDiscoveryCriteria,
) -> Vec<DiscoveredMediaItem> {
    let Ok(content) = fs::read_to_string(targets_path) else {
        return Vec::new();
    };
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&content) else {
        return Vec::new();
    };

    let mut items: Vec<DiscoveredMediaItem> = arr
        .iter()
        .filter_map(|val| parse_batch_target_value(val, criteria))
        .collect();

    items.sort_by_key(|a| a.stem.to_lowercase());
    items
}

/// Session configuration and callbacks for scanning media sources.
struct SourceScanSession<'a, F> {
    phash_dir: &'a Path,
    active_jobs: &'a [ActiveJobTelemetry],
    certified_stems: &'a std::collections::HashSet<String>,
    now_iso: &'a str,
    on_chunk: &'a mut F,
}

/// In-memory catalog buffers and mutable sources state for multi-source scanner.
struct SourceScanData<'a> {
    persistent_cache: &'a mut HashMap<String, DiscoveredMediaItem>,
    active_catalog: &'a mut Vec<DiscoveredMediaItem>,
    all_sources: &'a mut [MediaSourceConfig],
}

/// Scans a single media source directory, probing new files while preserving existing cache entries.
///
/// # Arguments
/// * `session` - Active scan session context and UI chunk notification callback.
/// * `src_idx` - Index of the target media source in `data.all_sources`.
/// * `data` - Mutable references to cache, active catalog, and source configurations.
///
/// # Data Flow & Invariants
/// 1. Collects physical video files in `data.all_sources[src_idx].path`.
/// 2. If directory is empty or unmounted (`video_paths.is_empty()`): resets counts to 0 but
///    STRICTLY PRESERVES all entries in `persistent_cache` to eliminate recomputing on remount.
/// 3. If files exist: purges only stems deleted from this source, probes new files via Rayon,
///    and appends discovered items to `active_catalog` while updating `persistent_cache`.
fn scan_single_source_chunked<F>(
    session: &mut SourceScanSession<F>,
    src_idx: usize,
    data: &mut SourceScanData,
) where
    F: FnMut(&[DiscoveredMediaItem], &[MediaSourceConfig]),
{
    let src_path = data.all_sources[src_idx].path.clone();
    let src_id = data.all_sources[src_idx].id.clone();

    let mut video_paths = Vec::new();
    collect_video_files(&src_path, &mut video_paths, MAX_MEDIA_SCAN_LIMIT);

    if video_paths.is_empty() {
        data.all_sources[src_idx].media_count = 0;
        data.all_sources[src_idx].last_scan_added = 0;
        data.all_sources[src_idx].last_scan_iso = Some(session.now_iso.to_string());
        return;
    }

    let found_stems: std::collections::HashSet<String> = video_paths
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();

    data.persistent_cache.retain(|stem, item| {
        item.source_id.as_deref() != Some(&src_id) || found_stems.contains(stem)
    });

    let criteria = MediaDiscoveryCriteria {
        phash_dir: session.phash_dir,
        active_jobs: session.active_jobs,
        certified_stems: session.certified_stems,
        source_id: &src_id,
    };

    let mut count = 0;
    let mut added_count = 0;

    for chunk in video_paths.chunks(SCAN_STREAMING_CHUNK_SIZE) {
        let chunk_items: Vec<DiscoveredMediaItem> = chunk
            .par_iter()
            .filter_map(|p| discover_single_media_file(p, data.persistent_cache, &criteria))
            .collect();

        for item in chunk_items {
            if !data.persistent_cache.contains_key(&item.stem) {
                added_count += 1;
            }
            data.persistent_cache
                .insert(item.stem.clone(), item.clone());
            data.active_catalog.push(item);
            count += 1;
        }

        data.all_sources[src_idx].media_count = count;
        data.all_sources[src_idx].last_scan_added = added_count;
        data.all_sources[src_idx].last_scan_iso = Some(session.now_iso.to_string());

        let mut progressive_items = data.active_catalog.clone();
        progressive_items.sort_by_key(|a| a.stem.to_lowercase());
        (session.on_chunk)(&progressive_items, data.all_sources);
    }
}

/// Updates the global in-memory video codec moniker cache from discovered items.
fn update_stem_codec_cache(items: &[DiscoveredMediaItem]) {
    if let Ok(mut c_lock) = STEM_CODEC_CACHE.lock() {
        let map = c_lock.get_or_insert_with(HashMap::new);
        for item in items {
            if let Some(ref codec) = item.video_codec {
                map.insert(item.stem.clone(), codec.clone());
            }
        }
    }
}

/// Scans all configured and enabled media sources with progressive chunk updates to the UI.
///
/// # Arguments
/// * `sources` - Slice of configured media sources.
/// * `couplings_path` - Path to certified subtitle couplings registry.
/// * `events_path` - Path to batch history events JSONL.
/// * `phash_dir` - Directory containing computed `.phash` fingerprint files.
/// * `active_jobs` - Current active jobs telemetry for in-progress match indicators.
/// * `cache_path` - Path to persistent `discovered_media.json` disk cache.
/// * `on_chunk` - Callback invoked progressively as chunks of media items are parsed.
///
/// # Returns
/// Pair containing `(active_catalog, updated_sources)`.
///
/// # Data Flow & Invariants
/// Loads persistent disk cache once. Iterates over enabled sources and delegates to
/// [`scan_single_source_chunked`]. Preserves cached probe data across unmounted volumes.
/// Persists updated cache back to disk upon completion.
pub fn scan_all_media_sources_with_progress<F>(
    sources: &[MediaSourceConfig],
    couplings_path: &Path,
    events_path: &Path,
    phash_dir: &Path,
    active_jobs: &[ActiveJobTelemetry],
    cache_path: &Path,
    mut on_chunk: F,
) -> (Vec<DiscoveredMediaItem>, Vec<MediaSourceConfig>)
where
    F: FnMut(&[DiscoveredMediaItem], &[MediaSourceConfig]),
{
    let mut persistent_cache = load_media_cache(cache_path);
    let certified_stems = get_certified_stems(events_path, couplings_path);

    let mut updated_sources = sources.to_vec();

    let has_enabled_sources = sources.iter().any(|s| s.enabled);
    if !has_enabled_sources {
        let mut full_catalog: Vec<_> = persistent_cache.into_values().collect();
        full_catalog.sort_by_key(|a| a.stem.to_lowercase());
        return (full_catalog, updated_sources);
    }

    let now_iso = format_unix_to_iso(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );

    let mut active_catalog = Vec::new();
    let mut data = SourceScanData {
        persistent_cache: &mut persistent_cache,
        active_catalog: &mut active_catalog,
        all_sources: &mut updated_sources,
    };

    let mut session = SourceScanSession {
        phash_dir,
        active_jobs,
        certified_stems: &certified_stems,
        now_iso: &now_iso,
        on_chunk: &mut on_chunk,
    };

    for src_idx in 0..data.all_sources.len() {
        if data.all_sources[src_idx].enabled {
            scan_single_source_chunked(&mut session, src_idx, &mut data);
        }
    }

    let all_cached: Vec<_> = persistent_cache.values().cloned().collect();
    save_media_cache(cache_path, &all_cached);
    update_stem_codec_cache(&all_cached);

    active_catalog.sort_by_key(|a| a.stem.to_lowercase());
    (active_catalog, updated_sources)
}

/// Scans all configured and enabled media sources, discovering candidate media and assigning source IDs.
pub fn scan_all_media_sources(
    sources: &[MediaSourceConfig],
    couplings_path: &Path,
    events_path: &Path,
    phash_dir: &Path,
    active_jobs: &[ActiveJobTelemetry],
    cache_path: &Path,
) -> (Vec<DiscoveredMediaItem>, Vec<MediaSourceConfig>) {
    scan_all_media_sources_with_progress(
        sources,
        couplings_path,
        events_path,
        phash_dir,
        active_jobs,
        cache_path,
        |_, _| {},
    )
}

/// Spawns background batch execution on the selected media paths with custom target and report file paths.
pub fn launch_selected_batch_with_target_file(
    media_paths: &[PathBuf],
    targets_file: &Path,
    report_file: &Path,
) -> (bool, String) {
    if media_paths.is_empty() {
        return (false, "Aucun média sélectionné pour le batch.".to_string());
    }

    let count = media_paths.len();
    if let Some(parent) = targets_file.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let paths_json = match serde_json::to_string_pretty(media_paths) {
        Ok(j) => j,
        Err(e) => return (false, format!("Erreur sérialisation cibles batch : {e}")),
    };

    if let Err(e) = fs::write(targets_file, paths_json) {
        return (false, format!("Erreur écriture cibles batch : {e}"));
    }

    let workers_count = count.clamp(1, MAX_SELECTED_BATCH_WORKERS);

    let res = Command::new("uv")
        .args([
            "run",
            "python",
            "scripts/batch_media_add_tvshow.py",
            "--targets-file",
            targets_file
                .to_str()
                .unwrap_or(DEFAULT_SELECTED_BATCH_TARGETS_PATH),
            "--report-path",
            report_file
                .to_str()
                .unwrap_or(DEFAULT_SELECTED_BATCH_REPORT_PATH),
            "--workers",
            &workers_count.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match res {
        Ok(_) => (
            true,
            format!(
                "Batch démarré sur {count} média(s) ({workers_count} workers parallèles). Bascule vers Tâches..."
            ),
        ),
        Err(e) => (false, format!("Échec du lancement Batch : {e}")),
    }
}

/// Context bundling file paths for remote batch streaming synchronization.
struct RemoteBatchScanContext<'a> {
    batch_targets_path: &'a Path,
    phash_dir: &'a Path,
    events_path: &'a Path,
    couplings_path: &'a Path,
    sources_path: &'a Path,
}

/// Synchronizes remote batch streaming targets into shared telemetry without triggering disk scans.
///
/// # Arguments
/// * `ctx` - File path context for targets, phashes, events, couplings, and sources.
/// * `sources` - Configured media sources slice.
/// * `shared_state` - Target shared telemetry state lock.
///
/// # Data Flow & Invariants
/// When no local storage directories exist on disk (e.g. cloud streaming nodes), loads candidate
/// media from `selected_batch_targets.json`, sets `is_scanning = false`, and updates source metrics.
fn load_and_publish_remote_batch_state(
    ctx: &RemoteBatchScanContext,
    sources: &[MediaSourceConfig],
    shared_state: &Arc<RwLock<SystemTelemetry>>,
) {
    if !ctx.batch_targets_path.exists() {
        if let Ok(mut state) = shared_state.write() {
            state.is_scanning = false;
        }
        return;
    }

    let active_jobs = shared_state
        .read()
        .map(|s| s.active_jobs.clone())
        .unwrap_or_default();

    let certified_stems = get_certified_stems(ctx.events_path, ctx.couplings_path);
    let default_source_id = sources
        .first()
        .map(|s| s.id.as_str())
        .unwrap_or(DEFAULT_SOURCE_ID_TVSHOW);

    let criteria = MediaDiscoveryCriteria {
        phash_dir: ctx.phash_dir,
        active_jobs: &active_jobs,
        certified_stems: &certified_stems,
        source_id: default_source_id,
    };

    let remote_items = load_remote_batch_media_items(ctx.batch_targets_path, &criteria);

    let mut updated_sources = sources.to_vec();
    if let Some(src) = updated_sources
        .iter_mut()
        .find(|s| s.id == default_source_id)
    {
        src.media_count = remote_items.len();
        src.last_scan_iso = Some(format_unix_to_iso(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ));
    }

    if let Ok(mut state) = shared_state.write() {
        state.discovered_media = remote_items;
        state.is_scanning = false;
        state.is_remote_streaming = true;
        merge_scanned_source_metrics(&mut state.media_sources, &updated_sources);
        let _ = save_media_sources(ctx.sources_path, &state.media_sources);
    }
}

/// Executes a full progressive local disk scan across all configured sources.
fn run_local_sources_scan(
    sources: &[MediaSourceConfig],
    ctx: &RemoteBatchScanContext,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
    publish: &mut impl FnMut(&[DiscoveredMediaItem], &[MediaSourceConfig], bool),
) {
    IS_MEDIA_SCANNING.store(true, Ordering::SeqCst);
    if let Ok(mut state) = shared_state.write() {
        state.is_scanning = true;
    }

    let active_jobs = shared_state
        .read()
        .map(|s| s.active_jobs.clone())
        .unwrap_or_default();

    let (items, updated_sources) = scan_all_media_sources_with_progress(
        sources,
        ctx.couplings_path,
        ctx.events_path,
        ctx.phash_dir,
        &active_jobs,
        Path::new(DEFAULT_MEDIA_CACHE_PATH),
        |partial_items, partial_sources| {
            publish(partial_items, partial_sources, true);
        },
    );

    IS_MEDIA_SCANNING.store(false, Ordering::SeqCst);
    publish(&items, &updated_sources, false);
}

/// Executes a single scan cycle, branching cleanly between local disk traversal and remote batch sync.
fn execute_sources_scan_cycle(
    ctx: &RemoteBatchScanContext,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
    publish: &mut impl FnMut(&[DiscoveredMediaItem], &[MediaSourceConfig], bool),
) {
    let sources = load_media_sources(ctx.sources_path);
    let has_local = sources.iter().any(|s| s.enabled && s.path.exists());

    if has_local {
        run_local_sources_scan(&sources, ctx, shared_state, publish);
    } else {
        IS_MEDIA_SCANNING.store(false, Ordering::SeqCst);
        load_and_publish_remote_batch_state(ctx, &sources, shared_state);
    }
}

/// Spawns a background worker thread periodically refreshing candidate media across all configured sources.
pub fn start_media_sources_scanner(
    shared_state: Arc<RwLock<SystemTelemetry>>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let events_path = resolve_events_log_path();
        let couplings_path = PathBuf::from(DEFAULT_COUPLINGS_PATH);
        let sources_path = PathBuf::from(DEFAULT_MEDIA_SOURCES_PATH);
        let phash_dir = PathBuf::from(DEFAULT_PHASH_DIR);
        let batch_targets_path = PathBuf::from(DEFAULT_SELECTED_BATCH_TARGETS_PATH);

        let remote_ctx = RemoteBatchScanContext {
            batch_targets_path: &batch_targets_path,
            phash_dir: &phash_dir,
            events_path: &events_path,
            couplings_path: &couplings_path,
            sources_path: &sources_path,
        };

        let mut publish_scan_update = |catalog_items: &[DiscoveredMediaItem],
                                       scanned_sources: &[MediaSourceConfig],
                                       is_scanning: bool| {
            if let Ok(mut state) = shared_state.write() {
                if state.media_sources.is_empty() {
                    state.media_sources = scanned_sources.to_vec();
                } else {
                    merge_scanned_source_metrics(&mut state.media_sources, scanned_sources);
                }
                state.discovered_media = catalog_items.to_vec();
                state.is_scanning = is_scanning;
                let _ = save_media_sources(&sources_path, &state.media_sources);
            }
        };

        execute_sources_scan_cycle(&remote_ctx, &shared_state, &mut publish_scan_update);

        while !stop_flag.load(Ordering::Relaxed) {
            let sleep_ticks = DEFAULT_MEDIA_SCAN_INTERVAL_SECS * 10;
            for _ in 0..sleep_ticks {
                if stop_flag.load(Ordering::Relaxed) {
                    return;
                }
                if MEDIA_RESCAN_REQUESTED.swap(false, Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            execute_sources_scan_cycle(&remote_ctx, &shared_state, &mut publish_scan_update);
        }
    });
}

/// Spawns a background worker thread periodically refreshing NAS candidate media without blocking the telemetry loop.
/// Backward compatibility alias for start_media_sources_scanner.
pub fn start_nas_media_scanner(
    shared_state: Arc<RwLock<SystemTelemetry>>,
    stop_flag: Arc<AtomicBool>,
) {
    start_media_sources_scanner(shared_state, stop_flag);
}

/// Spawns background batch execution on the selected media paths using default paths.
pub fn launch_selected_batch(media_paths: &[PathBuf]) -> (bool, String) {
    launch_selected_batch_with_target_file(
        media_paths,
        Path::new(DEFAULT_SELECTED_BATCH_TARGETS_PATH),
        Path::new(DEFAULT_SELECTED_BATCH_REPORT_PATH),
    )
}

/// Cached historical records and statistical aggregates for top telemetry.
struct HistoryTelemetryCache {
    records: Vec<HistoricalRecord>,
    stats: crate::commands::top::model::OverallStats,
    last_mtime: Option<std::time::SystemTime>,
}

/// Conditionally reloads historical batch events if the events log has been touched.
fn maybe_reload_history(
    events_path: &Path,
    couplings_path: &Path,
    cache: &mut HistoryTelemetryCache,
) {
    if let Ok(mtime) = fs::metadata(events_path).and_then(|m| m.modified()) {
        if cache.last_mtime != Some(mtime) {
            cache.records = load_historical_records(events_path, couplings_path);
            cache.stats = calculate_overall_stats(&cache.records);
            cache.last_mtime = Some(mtime);
        }
    }
}

/// Marks discovered media items as InProgress if currently being processed by an active worker.
fn update_discovered_media_status(
    media: &mut [DiscoveredMediaItem],
    active_jobs: &[ActiveJobTelemetry],
    history_records: &[HistoricalRecord],
) {
    for item in media {
        if active_jobs.iter().any(|j| j.media_name == item.stem) {
            item.status = IngestionStatus::InProgress;
        } else if history_records
            .iter()
            .any(|r| r.success && r.video_stem == item.stem)
        {
            item.status = IngestionStatus::Completed;
        }
    }
}

/// Discovers and filters active dbsrt and ffmpeg candidate processes from sysinfo.
fn collect_candidate_processes(system: &System) -> (Vec<RawCandidateProcess>, Vec<u32>) {
    let mut ffmpeg_ppids = Vec::new();
    let mut raw_candidates = Vec::new();

    for (pid_obj, proc) in system.processes() {
        let cmdline: Vec<String> = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        if cmdline.is_empty() {
            continue;
        }

        let cmd_str = cmdline.join(" ");
        let is_dbsrt = cmd_str.contains("dbsrt") && cmd_str.contains("media:add");
        let is_ffmpeg =
            cmd_str.contains("ffmpeg") && (cmd_str.contains("-i") || cmd_str.contains("-f"));

        if !is_dbsrt && !is_ffmpeg {
            continue;
        }

        let pid = pid_obj.as_u32();

        // Strict liveness check: exclude dead, zombie or already terminated processes
        if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
            continue;
        }

        // Strict process filter: exclude individual thread tasks to prevent thread duplication
        if proc.thread_kind().is_some() || is_thread_task(pid) {
            continue;
        }

        let ppid = proc.parent().map(|p| p.as_u32()).unwrap_or(0);
        if is_ffmpeg {
            ffmpeg_ppids.push(ppid);
        }

        raw_candidates.push(RawCandidateProcess {
            pid,
            ppid,
            cmdline,
            cmd_str,
            is_dbsrt,
            is_ffmpeg,
            cpu_pct: proc.cpu_usage(),
            mem_bytes: proc.memory(),
            run_time_secs: proc.run_time(),
        });
    }

    (raw_candidates, ffmpeg_ppids)
}

/// Background sampler worker updating the shared telemetry state at regular intervals.
pub fn run_telemetry_sampler_loop(
    shared_state: Arc<RwLock<SystemTelemetry>>,
    gpu_state: Arc<RwLock<GpuTelemetry>>,
    refresh_interval_ms: u64,
    stop_flag: Arc<AtomicBool>,
) {
    let mut system = System::new();
    let mut networks = Networks::new_with_refreshed_list();
    let mut last_net_instant = Instant::now();
    let mut last_proc_rx: Option<(Instant, u64)> = None;
    let mut previous_io_samples: HashMap<u32, (Instant, u64)> = HashMap::new();

    let events_path = resolve_events_log_path();
    let couplings_path = PathBuf::from(DEFAULT_COUPLINGS_PATH);

    let initial_records = load_historical_records(&events_path, &couplings_path);
    let initial_stats = calculate_overall_stats(&initial_records);
    let mut history_cache = HistoryTelemetryCache {
        last_mtime: fs::metadata(&events_path)
            .ok()
            .and_then(|m| m.modified().ok()),
        records: initial_records,
        stats: initial_stats,
    };

    preload_stem_size_cache();

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();

        // 0. Check for historical record log updates
        maybe_reload_history(&events_path, &couplings_path, &mut history_cache);

        // 1. Refresh system-wide CPU and memory
        system.refresh_cpu_specifics(CpuRefreshKind::everything());
        system.refresh_memory_specifics(MemoryRefreshKind::everything());
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true, // remove_dead_processes: purge terminated processes on each tick
            ProcessRefreshKind::everything()
                .without_user()
                .without_environ(),
        );

        let host_cpu = system.global_cpu_usage();
        let ram_used = system.used_memory() as f64 / BYTES_PER_GIB_F64;
        let ram_total = system.total_memory() as f64 / BYTES_PER_GIB_F64;

        // Refresh physical network interface RX throughput
        let (net_rx_mib_s, new_last_net) =
            compute_net_rx_rate(&mut networks, now, last_net_instant, &mut last_proc_rx);
        last_net_instant = new_last_net;

        // 2. Discover active candidate processes
        let (raw_candidates, ffmpeg_ppids) = collect_candidate_processes(&system);

        let (active_jobs, new_previous_io) =
            process_candidate_jobs(raw_candidates, &ffmpeg_ppids, previous_io_samples, now);
        previous_io_samples = new_previous_io;

        // Compute total active VFS throughput
        let total_rate_mib_s: f64 =
            sanitize_non_negative_rate(active_jobs.iter().map(|j| j.current_rate_mib_s).sum());
        let gpu_snap = gpu_state.read().unwrap().clone();

        let now_unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let session =
            compute_session_throughput(&history_cache.records, &active_jobs, now_unix_secs);

        let batch_queue = sample_batch_queue(active_jobs.len());

        {
            let mut state = shared_state.write().unwrap();
            state.host_cpu_pct = host_cpu;
            state.ram_used_gib = ram_used;
            state.ram_total_gib = ram_total;
            state.gpu = gpu_snap;
            state.total_rate_mib_s = total_rate_mib_s;
            state.net_rx_mib_s = net_rx_mib_s;
            state.session = session;
            state.history_records = history_cache.records.clone();
            state.stats = history_cache.stats.clone();
            state.is_scanning = IS_MEDIA_SCANNING.load(Ordering::Relaxed);
            state.batch_queue = batch_queue;
            update_discovered_media_status(
                &mut state.discovered_media,
                &active_jobs,
                &history_cache.records,
            );
            state.active_jobs = active_jobs;
        }

        std::thread::sleep(std::time::Duration::from_millis(refresh_interval_ms));
    }
}

/// Deserializes volatile batch queue state from a JSON file.
fn load_batch_queue_from_json(path: &Path) -> Option<BatchQueueTelemetry> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<BatchQueueTelemetry>(&content).ok()
}

/// Fallback reconstruction of batch queue metrics from target manifest and events log.
fn load_batch_queue_from_manifest(targets_path: &Path) -> Option<BatchQueueTelemetry> {
    let content = fs::read_to_string(targets_path).ok()?;
    let arr = serde_json::from_str::<Vec<serde_json::Value>>(&content).ok()?;
    let total = arr.len();
    let events_path = resolve_events_log_path();
    let completed = count_completed_events_fast(&events_path);
    let rem = total.saturating_sub(completed);
    let pct = if total > 0 {
        ((completed as f64 / total as f64) * 100.0 * 10.0).round() / 10.0
    } else {
        0.0
    };
    Some(BatchQueueTelemetry {
        total_targets: total,
        completed_targets: completed,
        failed_targets: 0,
        remaining_targets: rem,
        progress_pct: pct,
        active_workers: 0,
        is_active: false,
        updated_at: String::new(),
    })
}

/// Reads volatile batch queue state from `run/batch_queue.json`, or falls back to target manifest calculation.
///
/// # Arguments
/// * `active_workers` - Count of parallel decoding workers actively running.
///
/// # Returns
/// Instantiated [`BatchQueueTelemetry`] structure.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// 1. Reads `run/batch_queue.json` written by batch worker daemon.
/// 2. If missing, attempts fallback count from `data/reports/selected_batch_targets.json`.
/// 3. Returns default empty struct if no active queue manifest is detected.
pub fn sample_batch_queue(active_workers: usize) -> BatchQueueTelemetry {
    let queue_path = Path::new("run/batch_queue.json");
    if let Some(mut data) = load_batch_queue_from_json(queue_path) {
        if active_workers > 0 {
            data.active_workers = active_workers;
            data.is_active = true;
        }
        return data;
    }

    let targets_path = Path::new("data/reports/selected_batch_targets.json");
    if let Some(mut data) = load_batch_queue_from_manifest(targets_path) {
        data.active_workers = active_workers;
        data.is_active = active_workers > 0 && data.remaining_targets > 0;
        return data;
    }

    BatchQueueTelemetry::default()
}

fn count_completed_events_fast(events_path: &Path) -> usize {
    if !events_path.is_file() {
        return 0;
    }
    let Ok(content) = fs::read_to_string(events_path) else {
        return 0;
    };
    content
        .lines()
        .filter(|l| l.contains("\"success\": true") || l.contains("\"success\":true"))
        .count()
}

/// Sanitizes floating point transfer rates to eliminate negative zero representations.
fn sanitize_non_negative_rate(val: f64) -> f64 {
    if val <= 0.0 {
        0.0
    } else {
        val
    }
}

/// Name prefixes of loopback, bridge, container, VPN and tunnel interfaces whose traffic is
/// not host network ingress.
const VIRTUAL_INTERFACE_PREFIXES: &[&str] = &[
    "lo", "virbr", "docker", "veth", "br-", "tun", "tap", "wg", "cni", "flannel", "podman",
];

/// True for interfaces counted in host RX throughput (both sysinfo and `/proc/net/dev`).
pub fn is_physical_interface(name: &str) -> bool {
    !VIRTUAL_INTERFACE_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Parses a single `/proc/net/dev` interface line and extracts received bytes if non-virtual.
fn parse_proc_net_line(line: &str) -> Option<u64> {
    let (iface, rest) = line.split_once(':')?;
    if !is_physical_interface(iface.trim()) {
        return None;
    }
    rest.split_whitespace().next()?.parse::<u64>().ok()
}

/// Reads aggregate network interface receive bytes directly from `/proc/net/dev`.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Some(u64)` representing cumulative RX bytes across non-loopback interfaces, or `None` if inaccessible.
///
/// # Errors
/// Silently falls back to `None` if `/proc/net/dev` cannot be opened or parsed.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Bypasses `/sys/class/net/` to guarantee network ingress visibility inside containerized
/// environments (e.g. CloudLinux LVE on shared web hosting). Ignores virtual bridge and loopback devices.
pub fn read_proc_net_dev_rx_bytes() -> Option<u64> {
    let content = fs::read_to_string(PROC_NET_DEV_PATH).ok()?;
    let total_rx = content
        .lines()
        .skip(PROC_NET_DEV_HEADER_LINES)
        .filter_map(parse_proc_net_line)
        .fold(0u64, |acc, bytes| acc.saturating_add(bytes));
    Some(total_rx)
}

/// Computes physical network interface RX throughput in MiB/s.
///
/// # Arguments
/// - `networks`: Mutable reference to `sysinfo::Networks` catalog.
/// - `now`: Current sampling timestamp instant.
/// - `last_instant`: Previous sampling timestamp instant.
/// - `last_proc_rx`: Persistent state holding previous `(Instant, u64)` sample for `/proc/net/dev`.
///
/// # Returns
/// A tuple `(f64, Instant)` containing the computed ingress rate in MiB/s and the updated timestamp.
///
/// # Errors
/// Never returns an error; sanitized to non-negative floats.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Uses the per-refresh deltas of `sysinfo::Networks` whenever it lists a physical interface
/// (an idle link reports 0). Only when sysinfo sees none (e.g. missing `/sys/class/net`) does it
/// fall back to delta computation from `/proc/net/dev`.
fn compute_net_rx_rate(
    networks: &mut Networks,
    now: Instant,
    last_instant: Instant,
    last_proc_rx: &mut Option<(Instant, u64)>,
) -> (f64, Instant) {
    networks.refresh(true);
    let dt = now
        .duration_since(last_instant)
        .as_secs_f64()
        .max(MIN_DELTA_TIME_FLOOR_SECS);
    let physical_rx: Vec<u64> = networks
        .iter()
        .filter(|(name, _)| is_physical_interface(name))
        .map(|(_, data)| data.received())
        .collect();

    if !physical_rx.is_empty() {
        let total_rx_bytes: u64 = physical_rx.iter().sum();
        return (
            sanitize_non_negative_rate((total_rx_bytes as f64 / BYTES_PER_MIB_F64) / dt),
            now,
        );
    }

    if let Some(curr_rx) = read_proc_net_dev_rx_bytes() {
        if let Some((prev_t, prev_rx)) = *last_proc_rx {
            let proc_dt = now
                .duration_since(prev_t)
                .as_secs_f64()
                .max(MIN_DELTA_TIME_FLOOR_SECS);
            let delta_bytes = curr_rx.saturating_sub(prev_rx) as f64;
            *last_proc_rx = Some((now, curr_rx));
            return (
                sanitize_non_negative_rate((delta_bytes / BYTES_PER_MIB_F64) / proc_dt),
                now,
            );
        } else {
            *last_proc_rx = Some((now, curr_rx));
        }
    }

    (0.0, now)
}

/// Builds the ffprobe command listing duration and stream codecs/tags for `probe_media_file`.
fn build_probe_media_command(path: &Path) -> Command {
    let mut cmd = Command::new(dbsrt_core::video::resolve_ffprobe());
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=index,codec_type,codec_name,profile,pix_fmt:stream_tags=language,title",
            "-of",
            "json",
        ]);
    dbsrt_core::apply_ftp_credentials(&mut cmd, path);
    cmd.arg(path);
    cmd
}

/// Decodes simple percent-encoded ASCII/UTF-8 substrings in media filenames (e.g. `%20` -> ` `, `%5B` -> `[`).
///
/// # Arguments
/// * `input` - Raw percent-encoded string slice.
///
/// # Returns
/// Fully decoded string or original string if no percent markers exist.
///
/// # Errors
/// Never returns an error (malformed escape sequences fall back to verbatim copy).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Scans bytes sequentially. Triplets `%XY` with two valid hex digits become raw bytes; the byte
/// buffer is then decoded as UTF-8 (invalid sequences become U+FFFD).
pub fn url_decode_simple(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    // Decode into bytes first: multi-byte UTF-8 escapes (e.g. `%C3%A9`) span several triplets.
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        let hex_pair = bytes
            .get(idx + 1..idx + 3)
            .filter(|pair| pair.iter().all(u8::is_ascii_hexdigit))
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[idx], hex_pair) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                idx += 3;
            }
            (other, _) => {
                decoded.push(other);
                idx += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Processes raw detected candidate processes into typed telemetry jobs and updates delta IO samples.
#[allow(clippy::type_complexity)]
/// Extracts target video path and media name from process command-line arguments.
fn extract_media_info(cmdline: &[String]) -> (Option<PathBuf>, String) {
    let mut target_path: Option<PathBuf> = None;
    let mut explicit_name: Option<String> = None;
    for (idx, arg) in cmdline.iter().enumerate() {
        if arg == "--media-name" && idx + 1 < cmdline.len() {
            explicit_name = Some(cmdline[idx + 1].clone());
        }
        if arg.ends_with(".mkv")
            || arg.ends_with(".mp4")
            || arg.ends_with(".webm")
            || arg.ends_with(".avi")
        {
            target_path = Some(PathBuf::from(arg));
            if explicit_name.is_some() {
                break;
            }
        }
        if arg == "-i" && idx + 1 < cmdline.len() {
            target_path = Some(PathBuf::from(&cmdline[idx + 1]));
            if explicit_name.is_some() {
                break;
            }
        }
    }

    let raw_name = explicit_name.unwrap_or_else(|| {
        target_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Tâche Active".to_string())
    });

    let media_name = url_decode_simple(&raw_name);
    (target_path, media_name)
}

/// DRM render node probed for VA-API availability.
const VAAPI_RENDER_NODE: &str = "/dev/dri/renderD128";

/// Render-node presence, probed once per process instead of on every sampling tick.
fn has_vaapi_render_node() -> bool {
    static HAS_RENDER_NODE: OnceLock<bool> = OnceLock::new();
    *HAS_RENDER_NODE.get_or_init(|| Path::new(VAAPI_RENDER_NODE).exists())
}

/// Vendor-neutral diagnostic for VA-API hardware decoding.
fn vaapi_engine_reason() -> String {
    format!("Acc\u{e9}l\u{e9}ration mat\u{e9}rielle VA-API active ({VAAPI_RENDER_NODE}).")
}

/// True when any command-line argument is a remote stream URL (`ftp://`, `https://`, ...).
pub fn cmdline_has_remote_stream(cmdline: &[String]) -> bool {
    cmdline
        .iter()
        .any(|arg| arg.contains("://") && dbsrt_core::container::is_remote_stream_or_pipe(arg))
}

/// Detects the decoding engine and diagnostic reason for an active process candidate.
fn detect_candidate_engine(
    cmd_str: &str,
    is_dbsrt: bool,
    is_hi10p: bool,
) -> (String, Option<String>) {
    let has_vaapi = has_vaapi_render_node();
    if is_dbsrt {
        if cmd_str.contains("--hwaccel disabled")
            || cmd_str.contains("--hwaccel cpu")
            || (!cmd_str.contains("--hwaccel vaapi") && !has_vaapi)
        {
            (
                "CPU (Software)".to_string(),
                Some(
                    "Décodage logiciel CPU (sans accélération matérielle GPU active).".to_string(),
                ),
            )
        } else if is_hi10p {
            (
                "CPU (Hi10P)".to_string(),
                Some("H.264 Hi10P (10-bit) non supporté par l'ASIC matériel GPU (Intel/Nvidia/AMD). Repli automatique en décodage logiciel CPU multi-thread.".to_string()),
            )
        } else {
            ("GPU (VA-API)".to_string(), Some(vaapi_engine_reason()))
        }
    } else if cmd_str.contains("-f srt") || cmd_str.contains("0:s:") {
        (
            "Sub Extraction".to_string(),
            Some("Extraction de flux sous-titre texte/SRT (démultiplexage rapide, aucun décodage vidéo).".to_string()),
        )
    } else if cmd_str.contains("-hwaccel vaapi") {
        ("GPU (VA-API)".to_string(), Some(vaapi_engine_reason()))
    } else if is_hi10p {
        (
            "CPU (Hi10P)".to_string(),
            Some("H.264 Hi10P (10-bit) non supporté par l'ASIC matériel GPU (Intel/Nvidia/AMD). Repli automatique en décodage logiciel CPU multi-thread.".to_string()),
        )
    } else {
        (
            "CPU (Software)".to_string(),
            Some(
                "Décodage logiciel CPU multi-cœurs (sans accélération matérielle GPU détectée)."
                    .to_string(),
            ),
        )
    }
}

/// Raw telemetry attributes collected for an active candidate process.
#[derive(Debug, Clone)]
pub struct RawCandidateProcess {
    pub pid: u32,
    pub ppid: u32,
    pub cmdline: Vec<String>,
    pub cmd_str: String,
    pub is_dbsrt: bool,
    pub is_ffmpeg: bool,
    pub cpu_pct: f32,
    pub mem_bytes: u64,
    pub run_time_secs: u64,
}

pub fn process_candidate_jobs(
    raw_candidates: Vec<RawCandidateProcess>,
    ffmpeg_ppids: &[u32],
    previous_io_samples: HashMap<u32, (Instant, u64)>,
    now: Instant,
) -> (Vec<ActiveJobTelemetry>, HashMap<u32, (Instant, u64)>) {
    let mut active_jobs = Vec::new();
    let mut active_pids = HashMap::new();

    for candidate in raw_candidates {
        // If dbsrt spawned a child ffmpeg doing the decoding work, skip the parent coordinator
        if candidate.is_dbsrt && ffmpeg_ppids.contains(&candidate.pid) {
            continue;
        }

        // Identify media path and name
        let (target_path, media_name) = extract_media_info(&candidate.cmdline);

        let is_hi10p = candidate.cmd_str.contains("Hi10P")
            || target_path.as_ref().is_some_and(|p| {
                p.to_string_lossy().contains("Hi10P")
                    || get_file_codec(&media_name).is_some_and(|c| c.contains("Hi10P"))
            })
            || get_file_codec(&media_name).is_some_and(|c| c.contains("Hi10P"));

        let (engine, engine_reason) =
            detect_candidate_engine(&candidate.cmd_str, candidate.is_dbsrt, is_hi10p);

        let elapsed = (candidate.run_time_secs as f64).max(1.0);
        let mut total_bytes = get_file_size(&media_name, target_path.as_deref());
        let mut read_bytes = read_proc_io_bytes(candidate.pid);
        let mut hashing_fps = 0.0;

        let is_streaming = cmdline_has_remote_stream(&candidate.cmdline);

        if let Some(progress) = read_job_progress_file(candidate.pid) {
            read_bytes = (progress.frames as f64 * RAW_FRAME_BYTES) as u64;
            if let Some(total_f) = progress.total_frames {
                total_bytes = (total_f as f64 * RAW_FRAME_BYTES) as u64;
            } else if is_streaming && total_bytes == 0 {
                let fps = progress.fps.unwrap_or(DEFAULT_COUPLING_BASELINE_FPS);
                total_bytes = (DEFAULT_STREAM_EPISODE_DURATION_SECS * fps * RAW_FRAME_BYTES) as u64;
            }
        } else if is_streaming && total_bytes == 0 {
            total_bytes = (DEFAULT_STREAM_EPISODE_DURATION_SECS
                * DEFAULT_COUPLING_BASELINE_FPS
                * RAW_FRAME_BYTES) as u64;
        }

        // Compute rate strictly via delta; first tick is 0.0 to prevent artificial spikes
        let current_rate_mib_s =
            if let Some(&(prev_t, prev_b)) = previous_io_samples.get(&candidate.pid) {
                let dt = now
                    .duration_since(prev_t)
                    .as_secs_f64()
                    .max(MIN_DELTA_TIME_FLOOR_SECS);
                let db = read_bytes.saturating_sub(prev_b) as f64;
                hashing_fps = (db / RAW_FRAME_BYTES) / dt;
                (db / BYTES_PER_MIB_F64) / dt
            } else {
                0.0
            };

        active_pids.insert(candidate.pid, (now, read_bytes));

        // Compute progression ratio and ETA
        let mut ratio = 0.0;
        let mut eta_secs = None;
        if total_bytes > 0 {
            ratio = (read_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0);
            let avg_rate = (read_bytes as f64 / BYTES_PER_MIB_F64) / elapsed;
            if ratio > MIN_PROGRESS_THRESHOLD && avg_rate > 0.0 {
                let remaining = total_bytes.saturating_sub(read_bytes) as f64;
                eta_secs = Some((remaining / BYTES_PER_MIB_F64) / avg_rate);
            }
        }

        let ram_rss_mib = candidate.mem_bytes as f64 / BYTES_PER_MIB_F64;
        let status = if crate::commands::top::process_ctl::check_if_stopped(candidate.pid) {
            "PAUSED".to_string()
        } else {
            "RUNNING".to_string()
        };

        active_jobs.push(ActiveJobTelemetry {
            pid: candidate.pid,
            media_name,
            engine,
            engine_reason,
            elapsed_secs: elapsed,
            read_bytes,
            total_bytes,
            progress_ratio: ratio,
            current_rate_mib_s,
            hashing_fps,
            eta_secs,
            cpu_percent: candidate.cpu_pct,
            ram_rss_mib,
            status,
            cmdline: candidate.cmdline,
        });
    }

    (active_jobs, active_pids)
}
