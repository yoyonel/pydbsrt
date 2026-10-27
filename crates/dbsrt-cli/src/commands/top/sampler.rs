//! High-performance telemetry sampler decoupling process/kernel polling from TUI rendering.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use rayon::prelude::*;
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate, System,
};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, BYTES_PER_MIB_F64, DEFAULT_BATCH_EVENTS_PATH, DEFAULT_COUPLINGS_PATH,
    DEFAULT_COUPLING_BASELINE_FPS, DEFAULT_MEDIA_CACHE_PATH, DEFAULT_MEDIA_SCAN_INTERVAL_SECS,
    DEFAULT_MEDIA_SOURCES_PATH, DEFAULT_NAS_TVSHOW_PATH, DEFAULT_PHASH_DIR,
    DEFAULT_SELECTED_BATCH_REPORT_PATH, DEFAULT_SELECTED_BATCH_TARGETS_PATH,
    DEFAULT_SOURCE_ID_TVSHOW, DEFAULT_SOURCE_NAME_TVSHOW, MAX_MEDIA_SCAN_LIMIT,
    MAX_SELECTED_BATCH_WORKERS, MIN_DELTA_TIME_FLOOR_SECS, MIN_PROGRESS_THRESHOLD,
    NAS_TVSHOW_DIR_ENV, SCAN_STREAMING_CHUNK_SIZE, SESSION_WINDOW_SECS,
};
use crate::commands::top::model::{
    calculate_overall_stats, format_unix_to_iso, parse_iso_to_unix_secs, ActiveJobTelemetry,
    DiscoveredMediaItem, GpuTelemetry, HistoricalRecord, IngestionStatus, MediaSourceConfig,
    MediaSubtrackInfo, PhashFingerprintInfo, SessionThroughput, SystemTelemetry,
};

/// Shared handle for background GPU monitoring process.
static GPU_CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Global file size cache by stem to eliminate blocking CIFS network metadata calls.
static STEM_SIZE_CACHE: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Global video codec cache by stem to eliminate blocking CIFS network metadata calls.
static STEM_CODEC_CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Initializes the file size and codec cache from batch reports and media cache if available.
pub fn preload_stem_size_cache() {
    let mut map = HashMap::new();
    let events_file = Path::new("data/reports/batch_tvshow_nightly.jsonl");
    if events_file.exists() {
        if let Ok(file) = fs::File::open(events_file) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if let (Some(stem), Some(size)) = (
                        v.get("video_stem").and_then(|s| s.as_str()),
                        v.get("file_size_bytes").and_then(|s| s.as_u64()),
                    ) {
                        map.insert(stem.to_string(), size);
                    }
                }
            }
        }
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

/// Helper to get cached video codec for a given stem.
pub fn get_file_codec(media_name: &str) -> Option<String> {
    let lock = STEM_CODEC_CACHE.lock().unwrap();
    if let Some(ref map) = *lock {
        if let Some(codec) = map.get(media_name) {
            return Some(codec.clone());
        }
    }
    None
}

/// Helper to get or insert file size for a given stem or path.
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
        let is_remote_mount = path.starts_with("/media/latty/Synology")
            || path.to_string_lossy().contains("Synology");
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

/// Spawns a background worker thread streaming JSON telemetry from intel_gpu_top.
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

/// Reads `/proc/{pid}/io` on Linux to extract `rchar` and `read_bytes`.
/// Network mounts (CIFS/SMB) increment `rchar` rather than physical `read_bytes`.
pub fn read_proc_io_bytes(pid: u32) -> u64 {
    let io_path = format!("/proc/{pid}/io");
    let mut rchar = 0u64;
    let mut read_bytes = 0u64;

    if let Ok(file) = fs::File::open(io_path) {
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            if line.starts_with("rchar:") {
                if let Some(val) = line.split_whitespace().nth(1) {
                    rchar = val.parse::<u64>().unwrap_or(0);
                }
            } else if line.starts_with("read_bytes:") {
                if let Some(val) = line.split_whitespace().nth(1) {
                    read_bytes = val.parse::<u64>().unwrap_or(0);
                }
            }
        }
    }
    rchar.max(read_bytes)
}

/// Checks if the given PID is a thread task rather than a main process leader (Tgid != Pid).
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

/// Loads historical processing profile records from the batch events NDJSON log with couplings fallback.
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
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        let end_iso = v
                            .get("end_iso")
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

    // Deduplicate keeping max wall_duration_secs per video_stem
    let mut by_stem: HashMap<String, HistoricalRecord> = HashMap::new();
    for r in raw_records {
        match by_stem.get(&r.video_stem) {
            Some(existing) if existing.wall_duration_secs >= r.wall_duration_secs => {}
            _ => {
                by_stem.insert(r.video_stem.clone(), r);
            }
        }
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
    let output = Command::new("ffprobe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=index,codec_type,codec_name,profile,pix_fmt:stream_tags=language,title",
            "-of",
            "json",
        ])
        .arg(path)
        .output();

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

    let default_path = std::env::var(NAS_TVSHOW_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_NAS_TVSHOW_PATH));

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

/// Scans all configured and enabled media sources with progressive chunk updates to the UI.
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
    let mut cached_map = load_media_cache(cache_path);
    let certified_stems = get_certified_stems(events_path, couplings_path);

    let mut updated_sources = sources.to_vec();

    let has_enabled_sources = sources.iter().any(|s| s.enabled);
    if !has_enabled_sources {
        let mut full_catalog: Vec<_> = cached_map.into_values().collect();
        full_catalog.sort_by_key(|a| a.stem.to_lowercase());
        return (full_catalog, updated_sources);
    }

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now_iso = format_unix_to_iso(now_secs);

    for src_idx in 0..updated_sources.len() {
        if !updated_sources[src_idx].enabled {
            continue;
        }

        let mut video_paths = Vec::new();
        let src_path = updated_sources[src_idx].path.clone();
        let src_id = updated_sources[src_idx].id.clone();
        if src_path.exists() && src_path.is_dir() {
            collect_video_files(&src_path, &mut video_paths, MAX_MEDIA_SCAN_LIMIT);
        }

        // Clean up entries deleted from disk for this active source only
        let found_stems: std::collections::HashSet<String> = video_paths
            .iter()
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .collect();
        cached_map.retain(|stem, item| {
            item.source_id.as_deref() != Some(&src_id) || found_stems.contains(stem)
        });

        let mut count = 0;
        let mut added_count = 0;

        for chunk in video_paths.chunks(SCAN_STREAMING_CHUNK_SIZE) {
            let chunk_items: Vec<DiscoveredMediaItem> = chunk
                .par_iter()
                .filter_map(|p| {
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

                    Some(DiscoveredMediaItem {
                        path: p.clone(),
                        stem,
                        size_bytes: size,
                        duration_secs: dur,
                        video_codec: codec,
                        subtitles: subs,
                        phash: phash_info,
                        source_id: Some(src_id.clone()),
                        status,
                    })
                })
                .collect();

            for item in chunk_items {
                if !cached_map.contains_key(&item.stem) {
                    added_count += 1;
                }
                cached_map.insert(item.stem.clone(), item);
                count += 1;
            }

            updated_sources[src_idx].media_count = count;
            updated_sources[src_idx].last_scan_added = added_count;
            updated_sources[src_idx].last_scan_iso = Some(now_iso.clone());

            let mut progressive_items: Vec<_> = cached_map.values().cloned().collect();
            progressive_items.sort_by_key(|a| a.stem.to_lowercase());
            on_chunk(&progressive_items, &updated_sources);
        }
    }

    let mut full_catalog: Vec<_> = cached_map.into_values().collect();
    save_media_cache(cache_path, &full_catalog);

    if let Ok(mut c_lock) = STEM_CODEC_CACHE.lock() {
        let map = c_lock.get_or_insert_with(HashMap::new);
        for item in &full_catalog {
            if let Some(ref codec) = item.video_codec {
                map.insert(item.stem.clone(), codec.clone());
            }
        }
    }

    full_catalog.sort_by_key(|a| a.stem.to_lowercase());
    (full_catalog, updated_sources)
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
        Err(e) => (false, format!("Échec du lancement batch : {e}")),
    }
}

/// Spawns a background worker thread periodically refreshing candidate media across all configured sources.
pub fn start_media_sources_scanner(
    shared_state: Arc<RwLock<SystemTelemetry>>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let events_path = PathBuf::from(DEFAULT_BATCH_EVENTS_PATH);
        let couplings_path = PathBuf::from(DEFAULT_COUPLINGS_PATH);
        let sources_path = PathBuf::from(DEFAULT_MEDIA_SOURCES_PATH);
        let phash_dir = PathBuf::from(DEFAULT_PHASH_DIR);
        let media_cache_path = PathBuf::from(DEFAULT_MEDIA_CACHE_PATH);

        let publish_scan_update = |catalog_items: &[DiscoveredMediaItem],
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

        // Run initial scan once at startup
        IS_MEDIA_SCANNING.store(true, Ordering::SeqCst);
        if let Ok(mut state) = shared_state.write() {
            state.is_scanning = true;
        }

        let initial_sources = load_media_sources(&sources_path);
        let active_jobs = {
            shared_state
                .read()
                .map(|s| s.active_jobs.clone())
                .unwrap_or_default()
        };
        let (items, updated_sources) = scan_all_media_sources_with_progress(
            &initial_sources,
            &couplings_path,
            &events_path,
            &phash_dir,
            &active_jobs,
            &media_cache_path,
            |partial_items, partial_sources| {
                publish_scan_update(partial_items, partial_sources, true);
            },
        );
        IS_MEDIA_SCANNING.store(false, Ordering::SeqCst);
        publish_scan_update(&items, &updated_sources, false);

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

            IS_MEDIA_SCANNING.store(true, Ordering::SeqCst);
            if let Ok(mut state) = shared_state.write() {
                state.is_scanning = true;
            }

            let sources = load_media_sources(&sources_path);
            let active_jobs = shared_state
                .read()
                .map(|s| s.active_jobs.clone())
                .unwrap_or_default();

            let (items, updated_sources) = scan_all_media_sources_with_progress(
                &sources,
                &couplings_path,
                &events_path,
                &phash_dir,
                &active_jobs,
                &media_cache_path,
                |partial_items, partial_sources| {
                    publish_scan_update(partial_items, partial_sources, true);
                },
            );

            IS_MEDIA_SCANNING.store(false, Ordering::SeqCst);
            publish_scan_update(&items, &updated_sources, false);
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
    let mut previous_io_samples: HashMap<u32, (Instant, u64)> = HashMap::new();

    let events_path = PathBuf::from(DEFAULT_BATCH_EVENTS_PATH);
    let couplings_path = PathBuf::from(DEFAULT_COUPLINGS_PATH);
    let media_cache_path = PathBuf::from(DEFAULT_MEDIA_CACHE_PATH);

    let mut last_events_mtime = fs::metadata(&events_path)
        .ok()
        .and_then(|m| m.modified().ok());
    let mut cached_history_records = load_historical_records(&events_path, &couplings_path);
    let mut cached_stats = calculate_overall_stats(&cached_history_records);
    let cached_map = load_media_cache(&media_cache_path);
    let mut cached_media_items: Vec<DiscoveredMediaItem> = cached_map.into_values().collect();
    cached_media_items.sort_by_key(|a| a.stem.to_lowercase());

    preload_stem_size_cache();

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();

        // 0. Check for historical record log updates
        if let Ok(meta) = fs::metadata(&events_path) {
            if let Ok(mtime) = meta.modified() {
                if last_events_mtime != Some(mtime) {
                    cached_history_records = load_historical_records(&events_path, &couplings_path);
                    cached_stats = calculate_overall_stats(&cached_history_records);
                    last_events_mtime = Some(mtime);
                }
            }
        }

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
        networks.refresh(true);
        let net_dt = now
            .duration_since(last_net_instant)
            .as_secs_f64()
            .max(MIN_DELTA_TIME_FLOOR_SECS);
        last_net_instant = now;
        let total_rx_bytes: u64 = networks
            .iter()
            .filter(|(name, _)| !name.starts_with("lo") && !name.starts_with("virbr"))
            .map(|(_, data)| data.received())
            .sum();
        let net_rx_mib_s = (total_rx_bytes as f64 / BYTES_PER_MIB_F64) / net_dt;

        // 2. Discover active candidate processes
        let mut dbsrt_pids = Vec::new();
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

            if is_dbsrt {
                dbsrt_pids.push(pid);
            }
            if is_ffmpeg {
                ffmpeg_ppids.push(ppid);
            }

            raw_candidates.push((
                pid,
                ppid,
                cmdline,
                cmd_str,
                is_dbsrt,
                is_ffmpeg,
                proc.cpu_usage(),
                proc.memory(),
                proc.run_time(),
            ));
        }

        let (active_jobs, new_previous_io) =
            process_candidate_jobs(raw_candidates, &ffmpeg_ppids, previous_io_samples, now);
        previous_io_samples = new_previous_io;

        // Compute total active VFS throughput
        let total_rate_mib_s: f64 = active_jobs.iter().map(|j| j.current_rate_mib_s).sum();
        let gpu_snap = gpu_state.read().unwrap().clone();

        let now_unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let session =
            compute_session_throughput(&cached_history_records, &active_jobs, now_unix_secs);

        {
            let mut state = shared_state.write().unwrap();
            state.host_cpu_pct = host_cpu;
            state.ram_used_gib = ram_used;
            state.ram_total_gib = ram_total;
            state.gpu = gpu_snap;
            state.total_rate_mib_s = total_rate_mib_s;
            state.net_rx_mib_s = net_rx_mib_s;
            state.session = session;
            state.history_records = cached_history_records.clone();
            state.stats = cached_stats.clone();
            state.is_scanning = IS_MEDIA_SCANNING.load(Ordering::Relaxed);

            if state.discovered_media.is_empty() {
                state.discovered_media = cached_media_items.clone();
            }

            for item in &mut state.discovered_media {
                if active_jobs.iter().any(|j| j.media_name == item.stem) {
                    item.status = IngestionStatus::InProgress;
                }
            }

            state.active_jobs = active_jobs;
        }

        std::thread::sleep(std::time::Duration::from_millis(refresh_interval_ms));
    }
}

/// Processes raw detected candidate processes into typed telemetry jobs and updates delta IO samples.
#[allow(clippy::type_complexity)]
pub fn process_candidate_jobs(
    raw_candidates: Vec<(u32, u32, Vec<String>, String, bool, bool, f32, u64, u64)>,
    ffmpeg_ppids: &[u32],
    previous_io_samples: HashMap<u32, (Instant, u64)>,
    now: Instant,
) -> (Vec<ActiveJobTelemetry>, HashMap<u32, (Instant, u64)>) {
    let mut active_jobs = Vec::new();
    let mut active_pids = HashMap::new();

    for (pid, _ppid, cmdline, cmd_str, is_dbsrt, _is_ffmpeg, cpu_pct, mem_bytes, run_time_secs) in
        raw_candidates
    {
        // If dbsrt spawned a child ffmpeg doing the decoding work, skip the parent coordinator
        if is_dbsrt && ffmpeg_ppids.contains(&pid) {
            continue;
        }

        // Identify media path and name
        let mut target_path: Option<PathBuf> = None;
        for (idx, arg) in cmdline.iter().enumerate() {
            if arg.ends_with(".mkv")
                || arg.ends_with(".mp4")
                || arg.ends_with(".webm")
                || arg.ends_with(".avi")
            {
                target_path = Some(PathBuf::from(arg));
                break;
            }
            if arg == "-i" && idx + 1 < cmdline.len() {
                target_path = Some(PathBuf::from(&cmdline[idx + 1]));
                break;
            }
        }

        let media_name = target_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Tâche Active".to_string());

        let is_hi10p = cmd_str.contains("Hi10P")
            || target_path.as_ref().is_some_and(|p| {
                p.to_string_lossy().contains("Hi10P")
                    || get_file_codec(&media_name).is_some_and(|c| c.contains("Hi10P"))
            })
            || get_file_codec(&media_name).is_some_and(|c| c.contains("Hi10P"));

        let (engine, engine_reason) = if is_dbsrt {
            if cmd_str.contains("--hwaccel disabled") || cmd_str.contains("--hwaccel cpu") {
                (
                    "CPU (Software)".to_string(),
                    Some(
                        "Décodage logiciel CPU forcé par option CLI (--hwaccel disabled/cpu)."
                            .to_string(),
                    ),
                )
            } else if is_hi10p {
                (
                    "CPU (Hi10P)".to_string(),
                    Some("H.264 Hi10P (10-bit) non supporté par l'ASIC matériel GPU (Intel/Nvidia/AMD). Repli automatique en décodage logiciel CPU multi-thread.".to_string()),
                )
            } else {
                (
                    "GPU (VA-API)".to_string(),
                    Some("Accélération matérielle Intel QuickSync / VA-API active (/dev/dri/renderD128).".to_string()),
                )
            }
        } else if cmd_str.contains("-f srt") || cmd_str.contains("0:s:") {
            (
                "Sub Extraction".to_string(),
                Some("Extraction de flux sous-titre texte/SRT (démultiplexage rapide, aucun décodage vidéo).".to_string()),
            )
        } else if cmd_str.contains("-hwaccel vaapi") {
            (
                "GPU (VA-API)".to_string(),
                Some("Accélération matérielle Intel QuickSync / VA-API active (/dev/dri/renderD128).".to_string()),
            )
        } else if is_hi10p {
            (
                "CPU (Hi10P)".to_string(),
                Some("H.264 Hi10P (10-bit) non supporté par l'ASIC matériel GPU (Intel/Nvidia/AMD). Repli automatique en décodage logiciel CPU multi-thread.".to_string()),
            )
        } else {
            (
                "CPU (Software)".to_string(),
                Some("Décodage logiciel CPU multi-cœurs (sans accélération matérielle GPU détectée).".to_string()),
            )
        };

        let elapsed = (run_time_secs as f64).max(1.0);
        let total_bytes = get_file_size(&media_name, target_path.as_deref());
        let read_bytes = read_proc_io_bytes(pid);

        // Compute rate strictly via delta; first tick is 0.0 to prevent artificial spikes
        let current_rate_mib_s = if let Some(&(prev_t, prev_b)) = previous_io_samples.get(&pid) {
            let dt = now
                .duration_since(prev_t)
                .as_secs_f64()
                .max(MIN_DELTA_TIME_FLOOR_SECS);
            let db = read_bytes.saturating_sub(prev_b) as f64;
            (db / BYTES_PER_MIB_F64) / dt
        } else {
            0.0
        };

        active_pids.insert(pid, (now, read_bytes));

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

        let ram_rss_mib = mem_bytes as f64 / BYTES_PER_MIB_F64;
        let status = if crate::commands::top::process_ctl::check_if_stopped(pid) {
            "PAUSED".to_string()
        } else {
            "RUNNING".to_string()
        };

        active_jobs.push(ActiveJobTelemetry {
            pid,
            media_name,
            engine,
            engine_reason,
            elapsed_secs: elapsed,
            read_bytes,
            total_bytes,
            progress_ratio: ratio,
            current_rate_mib_s,
            eta_secs,
            cpu_percent: cpu_pct,
            ram_rss_mib,
            status,
            cmdline,
        });
    }

    (active_jobs, active_pids)
}
