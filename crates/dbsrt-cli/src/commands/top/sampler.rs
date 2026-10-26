//! High-performance telemetry sampler decoupling process/kernel polling from TUI rendering.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate, System,
};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, BYTES_PER_MIB_F64, MIN_DELTA_TIME_FLOOR_SECS, MIN_PROGRESS_THRESHOLD,
};
use crate::commands::top::model::{ActiveJobTelemetry, GpuTelemetry, SystemTelemetry};

/// Shared handle for background GPU monitoring process.
static GPU_CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Global file size cache by stem to eliminate blocking CIFS network metadata calls.
static STEM_SIZE_CACHE: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Initializes the file size cache from batch reports and couplings if available.
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

    let mut lock = STEM_SIZE_CACHE.lock().unwrap();
    *lock = Some(map);
}

/// Helper to get or insert file size for a given stem or path.
fn get_file_size(media_name: &str, target_path: Option<&Path>) -> u64 {
    {
        let lock = STEM_SIZE_CACHE.lock().unwrap();
        if let Some(ref map) = *lock {
            if let Some(&size) = map.get(media_name) {
                return size;
            }
        }
    }

    if let Some(path) = target_path {
        if let Ok(metadata) = fs::metadata(path) {
            let size = metadata.len();
            let mut lock = STEM_SIZE_CACHE.lock().unwrap();
            if let Some(ref mut map) = *lock {
                map.insert(media_name.to_string(), size);
            }
            return size;
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
fn read_proc_io_bytes(pid: u32) -> u64 {
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
fn is_thread_task(pid: u32) -> bool {
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

    preload_stem_size_cache();

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();

        // 1. Refresh system-wide CPU and memory
        system.refresh_cpu_specifics(CpuRefreshKind::everything());
        system.refresh_memory_specifics(MemoryRefreshKind::everything());
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            false,
            ProcessRefreshKind::everything(),
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
            let pid = pid_obj.as_u32();

            // Strict process filter: exclude individual thread tasks to prevent thread duplication
            if proc.thread_kind().is_some() || is_thread_task(pid) {
                continue;
            }

            let ppid = proc.parent().map(|p| p.as_u32()).unwrap_or(0);
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

        let mut active_jobs = Vec::new();
        let mut active_pids = HashMap::new();

        for (
            pid,
            _ppid,
            cmdline,
            cmd_str,
            is_dbsrt,
            _is_ffmpeg,
            cpu_pct,
            mem_bytes,
            run_time_secs,
        ) in raw_candidates
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

            let engine = if is_dbsrt {
                if cmd_str.contains("--hwaccel disabled") || cmd_str.contains("--hwaccel cpu") {
                    "CPU (Software)".to_string()
                } else {
                    "GPU (VA-API)".to_string()
                }
            } else if cmd_str.contains("-f srt") || cmd_str.contains("0:s:") {
                "Sub Extraction".to_string()
            } else if cmd_str.contains("-hwaccel vaapi") {
                "GPU (VA-API)".to_string()
            } else {
                "CPU (Software)".to_string()
            };

            let elapsed = (run_time_secs as f64).max(1.0);
            let total_bytes = get_file_size(&media_name, target_path.as_deref());
            let read_bytes = read_proc_io_bytes(pid);

            // Compute rate strictly via delta; first tick is 0.0 to prevent artificial spikes
            let current_rate_mib_s = if let Some(&(prev_t, prev_b)) = previous_io_samples.get(&pid)
            {
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

        previous_io_samples = active_pids;

        // Compute total active VFS throughput
        let total_rate_mib_s: f64 = active_jobs.iter().map(|j| j.current_rate_mib_s).sum();
        let gpu_snap = gpu_state.read().unwrap().clone();

        {
            let mut state = shared_state.write().unwrap();
            state.host_cpu_pct = host_cpu;
            state.ram_used_gib = ram_used;
            state.ram_total_gib = ram_total;
            state.gpu = gpu_snap;
            state.active_jobs = active_jobs;
            state.total_rate_mib_s = total_rate_mib_s;
            state.net_rx_mib_s = net_rx_mib_s;
        }

        std::thread::sleep(std::time::Duration::from_millis(refresh_interval_ms));
    }
}
