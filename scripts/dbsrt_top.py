#!/usr/bin/env python3
"""dbsrt top: Real-time TUI Process & Ingestion Monitor (ctop/btop style).

Displays real-time pHash video decoding, GPU VA-API vs CPU acceleration,
CIFS I/O bandwidth, progression bars, moving average throughput, ETA predictions,
and post-processing historical telemetry statistics.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import select
import signal
import statistics
import subprocess
import sys
import termios
import threading
import time
import tty
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List

import psutil
from rich.console import Console, Group
from rich.live import Live
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

# Operational & Formatting Constants
DEFAULT_REFRESH_INTERVAL_SECS: float = 1.0
DEFAULT_EVENTS_FILE: Path = Path("data/reports/batch_tvshow_nightly.jsonl")
BYTES_PER_KIB: int = 1024
BYTES_PER_MIB: int = 1024 * 1024
BYTES_PER_GIB: int = 1024 * 1024 * 1024
SECONDS_PER_MINUTE: int = 60
SECONDS_PER_HOUR: int = 3600
MIN_PROGRESS_THRESHOLD: float = 0.001
MAX_PROGRESS_CAP: float = 1.0
MAX_STEM_DISPLAY_CHARS: int = 40
RECENT_COMPLETED_DISPLAY_COUNT: int = 6
PROGRESS_BAR_WIDTH: int = 18
MAX_SENSIBLE_RATE_MIB_S: float = 500.0
DEFAULT_KEY_POLL_TIMEOUT_SECS: float = 0.15
ESC_SEQUENCE_TIMEOUT_SECS: float = 0.01
UI_EVENT_LOOP_POLL_TIMEOUT_SECS: float = 0.02
MAX_LOG_LINES_DISPLAY: int = 25
FEEDBACK_DISPLAY_TIMEOUT_SECS: float = 3.5
MAX_OPEN_FILES_PREVIEW: int = 8

# Shared in-memory telemetry state for decoupling I/O from UI reactivity
SHARED_ACTIVE_JOBS: List[ActiveJobTelemetry] = []
SHARED_HISTORICAL_RECORDS: List[Dict[str, Any]] = []
TELEMETRY_LOCK: threading.Lock = threading.Lock()
NEW_DATA_EVENT: threading.Event = threading.Event()
STOP_SAMPLER_EVENT: threading.Event = threading.Event()

# In-memory tracking for instantaneous delta rates
PREVIOUS_IO_SAMPLES: Dict[int, tuple[float, int]] = {}
FILE_SIZE_CACHE: Dict[Path, int] = {}
STEM_SIZE_CACHE: Dict[str, int] = {}


@dataclass
class GpuTelemetry:
    """Hardware GPU telemetry sampled in real time from intel_gpu_top."""

    video_busy_pct: float = 0.0
    render_busy_pct: float = 0.0
    freq_mhz: float = 0.0
    power_watts: float = 0.0
    available: bool = False


LATEST_GPU_TELEMETRY: GpuTelemetry = GpuTelemetry()
GPU_MONITOR_PROC: subprocess.Popen | None = None


def _gpu_monitoring_worker() -> None:
    """Background streaming worker that continuously updates LATEST_GPU_TELEMETRY from intel_gpu_top."""
    global LATEST_GPU_TELEMETRY, GPU_MONITOR_PROC
    cmd = ["/usr/bin/intel_gpu_top", "-J", "-s", "1000"]
    try:
        GPU_MONITOR_PROC = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
    except (FileNotFoundError, OSError):
        return

    buffer = ""
    try:
        while GPU_MONITOR_PROC and GPU_MONITOR_PROC.stdout:
            line = GPU_MONITOR_PROC.stdout.readline()
            if not line:
                break
            buffer += line
            if line.strip().startswith("}"):
                try:
                    cleaned = buffer.strip().lstrip("[, \r\n").rstrip(", \r\n")
                    if cleaned.startswith("{") and cleaned.endswith("}"):
                        obj = json.loads(cleaned)
                        v_busy = float(obj.get("engines", {}).get("Video", {}).get("busy", 0.0))
                        r_busy = float(obj.get("engines", {}).get("Render/3D", {}).get("busy", 0.0))
                        freq = float(obj.get("frequency", {}).get("actual", 0.0))
                        power = float(obj.get("power", {}).get("GPU", 0.0))
                        LATEST_GPU_TELEMETRY = GpuTelemetry(
                            video_busy_pct=v_busy,
                            render_busy_pct=r_busy,
                            freq_mhz=freq,
                            power_watts=power,
                            available=True,
                        )
                        buffer = ""
                except Exception:
                    pass
    except Exception:
        pass


def start_gpu_monitoring_thread() -> None:
    """Spawns background GPU monitoring daemon thread."""
    if Path("/usr/bin/intel_gpu_top").exists():
        t = threading.Thread(target=_gpu_monitoring_worker, daemon=True)
        t.start()


def stop_gpu_monitoring() -> None:
    """Terminates intel_gpu_top subprocess cleanly."""
    global GPU_MONITOR_PROC
    if GPU_MONITOR_PROC:
        try:
            GPU_MONITOR_PROC.terminate()
            GPU_MONITOR_PROC.wait(timeout=1.0)
        except Exception:
            try:
                GPU_MONITOR_PROC.kill()
            except Exception:
                pass
        GPU_MONITOR_PROC = None


def _telemetry_sampler_worker(events_file: Path, refresh_interval: float) -> None:
    """Background daemon worker continuously updating active jobs and historical records."""
    global SHARED_ACTIVE_JOBS, SHARED_HISTORICAL_RECORDS
    last_mtime: float = -1.0
    cached_records: List[Dict[str, Any]] = []

    while not STOP_SAMPLER_EVENT.is_set():
        try:
            if events_file.exists():
                current_mtime = events_file.stat().st_mtime
                if current_mtime != last_mtime:
                    cached_records = load_historical_records(events_file)
                    last_mtime = current_mtime
            elif not cached_records:
                cached_records = load_historical_records(events_file)

            active = scan_active_jobs()

            with TELEMETRY_LOCK:
                SHARED_ACTIVE_JOBS = active
                SHARED_HISTORICAL_RECORDS = cached_records

            NEW_DATA_EVENT.set()
        except Exception:
            pass

        STOP_SAMPLER_EVENT.wait(timeout=refresh_interval)


@dataclass
class ActiveJobTelemetry:
    """Telemetry captured for a currently running dbsrt or ffmpeg process."""

    pid: int
    media_name: str
    engine: str
    elapsed_secs: float
    read_bytes: int
    total_bytes: int
    progress_ratio: float
    current_rate_mib_s: float
    eta_secs: float | None
    cpu_percent: float
    ram_rss_mib: float
    status: str = "running"
    cmdline: List[str] = field(default_factory=list)
    open_files: List[str] = field(default_factory=list)


def toggle_pause_job(pid: int) -> tuple[bool, str]:
    """Pauses or resumes a job process using SIGSTOP / SIGCONT."""
    try:
        proc = psutil.Process(pid)
        status = proc.status()
        if status == psutil.STATUS_STOPPED:
            os.kill(pid, signal.SIGCONT)
            return True, f"PID {pid} repris (SIGCONT)"
        else:
            os.kill(pid, signal.SIGSTOP)
            return True, f"PID {pid} mis en pause (SIGSTOP)"
    except psutil.NoSuchProcess:
        return False, f"PID {pid} introuvable (processus terminé)"
    except PermissionError:
        return False, f"Permission refusée pour envoyer le signal au PID {pid}"
    except OSError as err:
        return False, f"Erreur signal PID {pid}: {err}"


def stop_job(pid: int) -> tuple[bool, str]:
    """Gracefully terminates a running job process using SIGTERM."""
    try:
        os.kill(pid, signal.SIGTERM)
        return True, f"Signal SIGTERM envoyé au PID {pid}"
    except ProcessLookupError:
        return False, f"PID {pid} introuvable (déjà terminé)"
    except PermissionError:
        return False, f"Permission refusée pour arrêter le PID {pid}"
    except OSError as err:
        return False, f"Erreur arrêt PID {pid}: {err}"


def get_job_logs(job: ActiveJobTelemetry, max_lines: int = MAX_LOG_LINES_DISPLAY) -> List[str]:
    """Retrieves real-time execution logs or detailed process inspection lines for the target job."""
    lines: List[str] = []

    # 1. Process metadata summary
    lines.append(f"PID: {job.pid}  │  Moteur: {job.engine}  │  État: {job.status.upper()}")
    lines.append(f"Temps écoulé: {format_duration(job.elapsed_secs)}  │  Débit CIFS: {job.current_rate_mib_s:.1f} MB/s")
    lines.append(
        f"Progression: {job.progress_ratio * 100.0:.1f}%  │  Taille: {job.total_bytes / BYTES_PER_MIB:.0f} MB (Lus: {job.read_bytes / BYTES_PER_MIB:.0f} MB)"
    )
    if job.cmdline:
        lines.append(f"Ligne de commande: {' '.join(job.cmdline)}")

    lines.append("-" * 75)

    # 2. Check for log file in data/logs or reports
    log_found = False
    possible_log_dirs = [Path("data/logs"), Path("/tmp"), Path("data/reports")]
    for ldir in possible_log_dirs:
        if ldir.exists():
            candidates = list(ldir.glob(f"*{job.media_name[:20]}*.log")) + list(ldir.glob(f"*{job.pid}*.log"))
            if candidates:
                try:
                    content = candidates[0].read_text(encoding="utf-8", errors="replace")
                    tail_lines = content.strip().splitlines()[-max_lines:]
                    lines.append(f"--- Extrait Log ({candidates[0].name}) ---")
                    lines.extend(tail_lines)
                    log_found = True
                    break
                except OSError:
                    pass

    # 3. Process inspection fallback
    if not log_found:
        lines.append("Fichiers ouverts & descripteurs I/O actifs:")
        target_open_files: List[str] = list(job.open_files)
        if not target_open_files:
            try:
                proc = psutil.Process(job.pid)
                for of in proc.open_files():
                    target_open_files.append(of.path)
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                pass

        if target_open_files:
            for f in target_open_files[:MAX_OPEN_FILES_PREVIEW]:
                lines.append(f"  • {f}")
            if len(target_open_files) > MAX_OPEN_FILES_PREVIEW:
                lines.append(f"  • ... et {len(target_open_files) - MAX_OPEN_FILES_PREVIEW} autres fichiers ouverts")
        else:
            lines.append("  • Aucun descripteur fichier spécifique détecté")

        try:
            proc = psutil.Process(job.pid)
            lines.append(f"Threads actifs: {proc.num_threads()}  │  Fils: {len(proc.children())}")
            lines.append(
                f"Mémoire RSS: {proc.memory_info().rss / BYTES_PER_MIB:.1f} MB  │  VMS: {proc.memory_info().vms / BYTES_PER_MIB:.1f} MB"
            )
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass

    return lines


class TerminalInput:
    """Context manager for non-blocking keyboard input in raw/cbreak mode."""

    def __init__(self) -> None:
        self.fd: int | None = None
        self.old_settings: Any = None

    def __enter__(self) -> TerminalInput:
        if sys.stdin.isatty():
            try:
                fd = sys.stdin.fileno()
                self.fd = fd
                self.old_settings = termios.tcgetattr(fd)
                tty.setcbreak(fd)
            except Exception:
                self.old_settings = None
        return self

    def __exit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        if self.fd is not None and self.old_settings is not None:
            try:
                termios.tcsetattr(self.fd, termios.TCSADRAIN, self.old_settings)
            except Exception:
                pass

    def poll_key(self, timeout: float = DEFAULT_KEY_POLL_TIMEOUT_SECS) -> str | None:
        if self.fd is None or not sys.stdin.isatty():
            time.sleep(timeout)
            return None
        rlist, _, _ = select.select([self.fd], [], [], timeout)
        if not rlist:
            return None
        try:
            ch = sys.stdin.read(1)
            if ch == "\x1b":
                rlist2, _, _ = select.select([self.fd], [], [], ESC_SEQUENCE_TIMEOUT_SECS)
                if rlist2:
                    ch2 = sys.stdin.read(1)
                    if ch2 in ("[", "O"):  # Both CSI (\x1b[) and SS3 (\x1bO used in TMUX/xterm)
                        rlist3, _, _ = select.select([self.fd], [], [], ESC_SEQUENCE_TIMEOUT_SECS)
                        if rlist3:
                            ch3 = sys.stdin.read(1)
                            if ch3 == "A":
                                return "UP"
                            elif ch3 == "B":
                                return "DOWN"
                            elif ch3 == "C":
                                return "RIGHT"
                            elif ch3 == "D":
                                return "LEFT"
                return "ESC"
            elif ch in ("\r", "\n"):
                return "ENTER"
            elif ch == " ":
                return "SPACE"
            return ch
        except (OSError, UnicodeDecodeError):
            return None


def format_duration(seconds: float) -> str:
    """Formats duration in seconds to compact human-readable string."""
    if seconds < 0 or seconds > 86400 * 7:
        return "N/A"
    m, s = divmod(int(seconds), SECONDS_PER_MINUTE)
    h, m = divmod(m, SECONDS_PER_MINUTE)
    if h > 0:
        return f"{h}h{m:02d}m"
    if m > 0:
        return f"{m}m{s:02d}s"
    return f"{s}s"


def format_progress_bar(ratio: float, width: int = PROGRESS_BAR_WIDTH) -> str:
    """Renders a text progress bar with filled and empty blocks."""
    clamped = max(0.0, min(ratio, MAX_PROGRESS_CAP))
    filled = int(round(clamped * width))
    bar = "█" * filled + "░" * (width - filled)
    return f"[{bar}] {clamped * 100.0:>5.1f}%"


def calculate_stats(values: List[float]) -> Dict[str, float]:
    """Computes descriptive statistics (min, max, mean, median, p90, stdev)."""
    if not values:
        return {"min": 0.0, "max": 0.0, "mean": 0.0, "median": 0.0, "p90": 0.0, "stdev": 0.0}
    s = sorted(values)
    n = len(s)
    p90_idx = min(int(n * 0.90), n - 1)
    return {
        "min": round(min(s), 2),
        "max": round(max(s), 2),
        "mean": round(statistics.mean(s), 2),
        "median": round(statistics.median(s), 2),
        "p90": round(s[p90_idx], 2),
        "stdev": round(statistics.stdev(s) if n > 1 else 0.0, 2),
    }


def scan_active_jobs() -> List[ActiveJobTelemetry]:
    """Scans running system processes to detect active dbsrt media:add and ffmpeg decoding tasks."""
    global PREVIOUS_IO_SAMPLES
    jobs: List[ActiveJobTelemetry] = []

    # First pass: collect raw candidate processes
    raw_candidates: List[dict] = []
    dbsrt_pids: set[int] = set()
    ffmpeg_ppids: set[int] = set()

    for proc in psutil.process_iter(["pid", "ppid", "name"]):
        try:
            p_name = (proc.info.get("name") or "").lower()
            if not any(t in p_name for t in ("dbsrt", "ffmpeg", "python", "python3")):
                continue

            cmdline = proc.cmdline()
            if not cmdline:
                continue

            cmd_str = " ".join(cmdline)
            is_dbsrt = "dbsrt" in cmd_str and "media:add" in cmd_str
            is_ffmpeg = "ffmpeg" in cmd_str and ("-i" in cmd_str or "-f" in cmd_str)

            if not (is_dbsrt or is_ffmpeg):
                continue

            pid = proc.info["pid"]
            ppid = proc.info.get("ppid", 0)

            if is_dbsrt:
                dbsrt_pids.add(pid)
            if is_ffmpeg:
                ffmpeg_ppids.add(ppid)

            raw_candidates.append(
                {
                    "proc": proc,
                    "pid": pid,
                    "ppid": ppid,
                    "cmdline": cmdline,
                    "cmd_str": cmd_str,
                    "is_dbsrt": is_dbsrt,
                    "is_ffmpeg": is_ffmpeg,
                }
            )
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue

    active_pids: set[int] = set()
    now = time.time()

    for item in raw_candidates:
        try:
            proc = item["proc"]
            pid = item["pid"]
            cmdline = item["cmdline"]
            cmd_str = item["cmd_str"]
            is_dbsrt = item["is_dbsrt"]

            # If this is dbsrt and it has an active child ffmpeg doing the decoding, skip parent
            if is_dbsrt and pid in ffmpeg_ppids:
                continue

            # Identify target media path from arguments
            target_path: Path | None = None
            for idx, arg in enumerate(cmdline):
                if arg.endswith((".mkv", ".mp4", ".webm", ".avi")):
                    target_path = Path(arg)
                    break
                if arg == "-i" and idx + 1 < len(cmdline):
                    target_path = Path(cmdline[idx + 1])
                    break

            media_name = target_path.stem if target_path else "Active Task"
            if is_dbsrt:
                engine = (
                    "CPU (Software)"
                    if ("--hwaccel disabled" in cmd_str or "--hwaccel cpu" in cmd_str)
                    else "GPU (VA-API)"
                )
            elif "-f srt" in cmd_str or "0:s:" in cmd_str:
                engine = "Sub Extraction"
            else:
                engine = "GPU (VA-API)" if "-hwaccel vaapi" in cmd_str else "CPU (Software)"

            try:
                create_time = proc.create_time()
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                create_time = now
            elapsed = max(now - create_time, 0.1)

            total_bytes = 0
            if media_name in STEM_SIZE_CACHE:
                total_bytes = STEM_SIZE_CACHE[media_name]
            elif target_path:
                if target_path in FILE_SIZE_CACHE:
                    total_bytes = FILE_SIZE_CACHE[target_path]
                else:
                    try:
                        total_bytes = target_path.stat().st_size
                        FILE_SIZE_CACHE[target_path] = total_bytes
                        STEM_SIZE_CACHE[media_name] = total_bytes
                    except OSError:
                        pass

            try:
                io_counters = proc.io_counters()
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                io_counters = None

            # Linux kernel detail: read_bytes in /proc/[pid]/io only counts physical block-level I/O.
            # Network mounts (CIFS/SMB, NFS) and VFS pagecache reads increment read_chars (rchar).
            read_bytes = 0
            if io_counters:
                read_bytes = max(
                    getattr(io_counters, "read_chars", 0),
                    getattr(io_counters, "read_bytes", 0),
                )

            # Compute rate: instantaneous delta rate between refreshes if available
            if pid in PREVIOUS_IO_SAMPLES:
                prev_t, prev_b = PREVIOUS_IO_SAMPLES[pid]
                dt = max(now - prev_t, 0.05)
                db = max(read_bytes - prev_b, 0)
                rate_mib = (db / BYTES_PER_MIB) / dt
            else:
                rate_mib = (read_bytes / BYTES_PER_MIB) / elapsed if elapsed > 0 else 0.0

            PREVIOUS_IO_SAMPLES[pid] = (now, read_bytes)
            active_pids.add(pid)

            ratio = 0.0
            eta_secs: float | None = None
            if total_bytes > 0:
                ratio = min(read_bytes / total_bytes, MAX_PROGRESS_CAP)
                # Compute ETA based on average rate for stability
                avg_rate_mib = (read_bytes / BYTES_PER_MIB) / elapsed if elapsed > 0 else 0.0
                if ratio > MIN_PROGRESS_THRESHOLD and avg_rate_mib > 0.0:
                    remaining_bytes = max(total_bytes - read_bytes, 0)
                    eta_secs = (remaining_bytes / BYTES_PER_MIB) / avg_rate_mib

            try:
                mem_info = proc.memory_info()
                ram_rss = (mem_info.rss / BYTES_PER_MIB) if mem_info else 0.0
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                ram_rss = 0.0

            try:
                cpu_pct = proc.cpu_percent(interval=None) or 0.0
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                cpu_pct = 0.0

            try:
                proc_status = proc.status()
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                proc_status = "running"

            jobs.append(
                ActiveJobTelemetry(
                    pid=pid,
                    media_name=media_name,
                    engine=engine,
                    elapsed_secs=elapsed,
                    read_bytes=read_bytes,
                    total_bytes=total_bytes,
                    progress_ratio=ratio,
                    current_rate_mib_s=rate_mib,
                    eta_secs=eta_secs,
                    cpu_percent=cpu_pct,
                    ram_rss_mib=ram_rss,
                    status=proc_status,
                    cmdline=cmdline,
                )
            )
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue

    # Cleanup terminated PIDs from tracking dictionary
    PREVIOUS_IO_SAMPLES = {p: v for p, v in PREVIOUS_IO_SAMPLES.items() if p in active_pids}

    return jobs


def load_historical_records(events_file: Path) -> List[Dict[str, Any]]:
    """Loads completed processing profile records from the events NDJSON log with couplings.json fallback."""
    records: List[Dict[str, Any]] = []
    if events_file.exists():
        try:
            with open(events_file, encoding="utf-8") as f:
                for line in f:
                    stripped = line.strip()
                    if stripped:
                        try:
                            records.append(json.loads(stripped))
                        except json.JSONDecodeError:
                            continue
        except OSError:
            pass

    # Deduplicate keeping max wall_duration_secs per video_stem
    by_stem: Dict[str, Dict[str, Any]] = {}
    for r in records:
        stem = r.get("video_stem")
        if stem:
            if "file_size_bytes" in r:
                try:
                    STEM_SIZE_CACHE[stem] = int(r["file_size_bytes"])
                except (ValueError, TypeError):
                    pass
            if stem not in by_stem or r.get("wall_duration_secs", 0) > by_stem[stem].get("wall_duration_secs", 0):
                by_stem[stem] = r

    # Fallback to data/SRT/couplings.json if no events exist
    if not by_stem:
        couplings_path = Path("data/SRT/couplings.json")
        if couplings_path.exists():
            try:
                data = json.loads(couplings_path.read_text(encoding="utf-8"))
                for c in data.get("couplings", []):
                    m_name = c.get("media_name", "")
                    subs = c.get("subtitles", [])
                    fps = c.get("fps", 25.0)
                    frames = c.get("frame_count", 0)
                    dur_secs = (frames / fps) if fps > 0 else 0.0
                    by_stem[m_name] = {
                        "video_stem": m_name,
                        "success": True,
                        "wall_duration_secs": dur_secs,
                        "cifs_read_rate_mib_s": 0.0,
                        "file_size_bytes": 0,
                        "certified_couplings_count": len(subs),
                    }
            except (json.JSONDecodeError, OSError):
                pass

    items = list(by_stem.values())
    items.sort(key=lambda x: str(x.get("end_iso") or x.get("start_iso") or ""))
    return items


def render_dashboard(
    active_jobs: List[ActiveJobTelemetry],
    history_records: List[Dict[str, Any]],
    selected_index: int = 0,
    show_stats: bool = False,
    show_recent: bool = False,
    is_menu_open: bool = False,
    is_log_modal_open: bool = False,
    feedback_msg: str | None = None,
) -> Panel:
    """Renders the complete rich TUI dashboard layout."""
    now_str = datetime.datetime.now().strftime("%H:%M:%S")

    host_cpu = psutil.cpu_percent(interval=None)
    mem = psutil.virtual_memory()
    host_ram_used_gib = mem.used / BYTES_PER_GIB
    host_ram_total_gib = mem.total / BYTES_PER_GIB

    total_active_rate = sum(j.current_rate_mib_s for j in active_jobs)

    # Compute global session throughput across recent batch entries + active jobs
    now_dt = datetime.datetime.now()
    recent_session_records = []
    for r in history_records:
        end_iso = r.get("end_iso")
        if end_iso:
            try:
                end_dt = datetime.datetime.fromisoformat(end_iso)
                if (now_dt - end_dt).total_seconds() < 7200:  # Last 2 hours
                    recent_session_records.append(r)
            except ValueError:
                pass

    session_bytes = sum(r.get("file_size_bytes", 0) for r in recent_session_records) + sum(
        j.read_bytes for j in active_jobs
    )
    session_starts = [
        datetime.datetime.fromisoformat(r["start_iso"]) for r in recent_session_records if r.get("start_iso")
    ]
    if active_jobs:
        active_earliest_start = min(now_dt - datetime.timedelta(seconds=j.elapsed_secs) for j in active_jobs)
        session_starts.append(active_earliest_start)

    session_rate_mib_s = 0.0
    session_dur_secs = 0.0
    if session_starts:
        earliest_start = min(session_starts)
        session_dur_secs = max((now_dt - earliest_start).total_seconds(), 1.0)
        session_rate_mib_s = (session_bytes / BYTES_PER_MIB) / session_dur_secs

    session_gib = session_bytes / BYTES_PER_GIB
    session_count = len(recent_session_records) + len(active_jobs)

    # 1. Header Banner
    header_text = Text()
    header_text.append(f"⏱️  {now_str}  │  ", style="bold cyan")
    header_text.append(f"Workers Actifs: {len(active_jobs)}  │  ", style="bold green" if active_jobs else "yellow")
    header_text.append(f"Host CPU: {host_cpu:.0f}%  │  ", style="bold white")
    header_text.append(f"RAM: {host_ram_used_gib:.1f}/{host_ram_total_gib:.1f} GiB\n", style="bold white")

    # Real-time Hardware GPU Telemetry
    gpu = LATEST_GPU_TELEMETRY
    if gpu.available:
        v_bar = format_progress_bar(gpu.video_busy_pct / 100.0, width=12)
        v_style = (
            "bold red" if gpu.video_busy_pct > 80.0 else ("bold green" if gpu.video_busy_pct > 15.0 else "dim white")
        )
        header_text.append("🎮 GPU INTEL IRIS XE :  ", style="bold cyan")
        header_text.append("Décodage (Video Engine) : ", style="bold white")
        header_text.append(f"{gpu.video_busy_pct:>5.1f}% {v_bar}  │  ", style=v_style)
        header_text.append(f"Render/3D: {gpu.render_busy_pct:.1f}%  │  ", style="dim white")
        header_text.append(f"Fréq: {gpu.freq_mhz:.0f} MHz  │  ", style="yellow")
        header_text.append(f"Conso: {gpu.power_watts:.1f} W\n", style="bright_magenta")

    header_text.append("⚡ DÉBITS RÉSEAU CIFS :  ", style="bold bright_blue")
    header_text.append(f"Instantané: {total_active_rate:.1f} MB/s", style="bold magenta")
    if session_count > 0 and session_dur_secs > 5.0:
        header_text.append("  │  ", style="dim")
        header_text.append(
            f"Débit Cumulé Global Session: {session_rate_mib_s:.1f} MB/s ({session_gib:.1f} GiB en {format_duration(session_dur_secs)} sur {session_count} flux)",
            style="bold bright_green",
        )

    # 2. Active Tasks Table (Interactive Selection & ctop-style highlight)
    tasks_table = Table(
        title="⚡ TÂCHES EN COURS (DÉCODAGE & PHASH TEMPS RÉEL)",
        title_style="bold yellow",
        expand=True,
        header_style="bold cyan",
    )
    tasks_table.add_column("Sel", justify="center", width=3)
    tasks_table.add_column("PID", justify="right", width=7)
    tasks_table.add_column("Média / Release", width=33)
    tasks_table.add_column("Taille", justify="right", width=9)
    tasks_table.add_column("Moteur / État", justify="center", width=16)
    tasks_table.add_column("Écoulé", justify="right", width=8)
    tasks_table.add_column("Débit Effectif", justify="right", width=13)
    tasks_table.add_column("Progression", width=25)
    tasks_table.add_column("ETA Restant", justify="right", width=12)

    if not active_jobs:
        tasks_table.add_row(
            "-",
            "-",
            "[dim italic]Aucun traitement dbsrt / ffmpeg en cours d'exécution[/dim italic]",
            "-",
            "-",
            "-",
            "0.0 MB/s",
            format_progress_bar(0.0),
            "-",
        )
    else:
        for idx, job in enumerate(active_jobs):
            is_selected = idx == selected_index
            sel_marker = "[bold bright_cyan]▶[/bold bright_cyan]" if is_selected else " "
            row_style = "bold white on navy_blue" if is_selected else None

            name_truncated = (
                job.media_name[:MAX_STEM_DISPLAY_CHARS] + "…"
                if len(job.media_name) > MAX_STEM_DISPLAY_CHARS
                else job.media_name
            )
            size_styled = f"{job.total_bytes / BYTES_PER_MIB:.0f} MB" if job.total_bytes > 0 else "-"

            if job.status == "stopped":
                engine_styled = "[bold yellow]⏸ PAUSE (STOP)[/bold yellow]"
            elif "VA-API" in job.engine:
                engine_styled = f"[green]{job.engine}[/green]"
            else:
                engine_styled = f"[yellow]{job.engine}[/yellow]"

            rate_styled = f"{job.current_rate_mib_s:.1f} MB/s"
            eta_str = format_duration(job.eta_secs) if job.eta_secs is not None else "~ incalculable"

            tasks_table.add_row(
                sel_marker,
                str(job.pid),
                name_truncated,
                size_styled,
                engine_styled,
                format_duration(job.elapsed_secs),
                rate_styled,
                format_progress_bar(job.progress_ratio),
                eta_str,
                style=row_style,
            )

        if len(active_jobs) > 1:
            tot_read = sum(j.read_bytes for j in active_jobs)
            tot_vid = sum(j.total_bytes for j in active_jobs)
            avg_ratio = (tot_read / tot_vid) if tot_vid > 0 else 0.0
            max_eta = max((j.eta_secs for j in active_jobs if j.eta_secs is not None), default=None)
            max_elapsed = max(j.elapsed_secs for j in active_jobs)
            all_gpu = all("VA-API" in j.engine for j in active_jobs)
            engine_summary = (
                "[bold green]GPU (VA-API)[/bold green]" if all_gpu else "[bold yellow]Mixte GPU/CPU[/bold yellow]"
            )
            tot_size_str = (
                f"[bold cyan]{tot_vid / BYTES_PER_GIB:.2f} GiB[/bold cyan]"
                if tot_vid >= BYTES_PER_GIB
                else f"[bold cyan]{tot_vid / BYTES_PER_MIB:.0f} MB[/bold cyan]"
            )

            tasks_table.add_section()
            tasks_table.add_row(
                "",
                "[bold cyan]Σ[/bold cyan]",
                f"[bold white]TOTAL CUMULÉ ({len(active_jobs)} flux)[/bold white]",
                tot_size_str,
                engine_summary,
                format_duration(max_elapsed),
                f"[bold magenta]{total_active_rate:.1f} MB/s[/bold magenta]",
                format_progress_bar(avg_ratio),
                format_duration(max_eta) if max_eta is not None else "~ incalculable",
            )

    # 3. cTop-Style Action Menu Popup (when Enter/m is pressed on selected task)
    menu_component = None
    if is_menu_open and active_jobs:
        selected_job = active_jobs[max(0, min(selected_index, len(active_jobs) - 1))]
        pause_label = (
            "Reprendre le processus (SIGCONT)"
            if selected_job.status == "stopped"
            else "Mettre en Pause le processus (SIGSTOP)"
        )
        menu_table = Table.grid(padding=(0, 2))
        menu_table.add_column(style="bold bright_yellow", justify="center")
        menu_table.add_column(style="bold white")
        menu_table.add_row(r"\[p]", pause_label)
        menu_table.add_row(r"\[s]", "Arrêter la tâche (SIGTERM)")
        menu_table.add_row(r"\[l]", "Afficher les logs & détails du processus")
        menu_table.add_row(r"\[c]", "Annuler / Fermer le menu (Échap)")

        menu_component = Panel(
            menu_table,
            title=(
                f" [bold bright_yellow]Menu d'Actions (cTop)[/bold bright_yellow] ── PID [bold bright_cyan]{selected_job.pid}[/bold bright_cyan] ({selected_job.media_name[:35]}) "
            ),
            border_style="bright_yellow",
            style="on grey15",
        )

    # 4. Process Inspection & Log Modal (when 'l' is chosen)
    log_component = None
    if is_log_modal_open and active_jobs:
        selected_job = active_jobs[max(0, min(selected_index, len(active_jobs) - 1))]
        log_lines = get_job_logs(selected_job)
        log_text = Text("\n".join(log_lines), style="bright_cyan")
        log_component = Panel(
            log_text,
            title=f" [bold bright_cyan]Inspection Processus & Logs[/bold bright_cyan] ── PID {selected_job.pid} ",
            subtitle=" [bold yellow]Appuyer sur 'c', 'Esc' ou 'Entrée' pour fermer l'inspection[/bold yellow] ",
            border_style="bright_cyan",
            style="on grey11",
        )

    # 5. Statistical Analysis Table (Collapsible, default: collapsed)
    phash_full_records = [
        r
        for r in history_records
        if r.get("success", False) and r.get("wall_duration_secs", 0) > 60.0  # Full decodes exceed 1 min
    ]

    rates = [r.get("cifs_read_rate_mib_s", 0.0) for r in phash_full_records]
    durations_m = [r.get("wall_duration_secs", 0.0) / SECONDS_PER_MINUTE for r in phash_full_records]
    sizes_gib = [r.get("file_size_bytes", 0) / BYTES_PER_GIB for r in phash_full_records]
    couplings = [r.get("certified_couplings_count", 0) for r in phash_full_records]

    rate_s = calculate_stats(rates)
    dur_s = calculate_stats(durations_m)
    size_s = calculate_stats(sizes_gib)
    coup_s = calculate_stats(couplings)

    stats_table = Table(
        title=f"📊 HISTORIQUE & STATISTIQUES RÉELLES (DÉCODAGE INTÉGRAL PHASH - {len(phash_full_records)} MÉDIAS)",
        title_style="bold green",
        expand=True,
        header_style="bold magenta",
    )
    stats_table.add_column("Dimension Analysée", width=30)
    stats_table.add_column("Min", justify="right")
    stats_table.add_column("Médiane", justify="right")
    stats_table.add_column("Moyenne", justify="right")
    stats_table.add_column("P90 (90e pct)", justify="right")
    stats_table.add_column("Max", justify="right")
    stats_table.add_column("Écart-type (σ)", justify="right")

    stats_table.add_row(
        "Débit Décodage pHash (MB/s)",
        f"{rate_s['min']:.2f}",
        f"{rate_s['median']:.2f}",
        f"{rate_s['mean']:.2f}",
        f"{rate_s['p90']:.2f}",
        f"{rate_s['max']:.2f}",
        f"{rate_s['stdev']:.2f}",
    )
    stats_table.add_row(
        "Durée Décodage (minutes)",
        f"{dur_s['min']:.1f}",
        f"{dur_s['median']:.1f}",
        f"{dur_s['mean']:.1f}",
        f"{dur_s['p90']:.1f}",
        f"{dur_s['max']:.1f}",
        f"{dur_s['stdev']:.1f}",
    )
    stats_table.add_row(
        "Taille Vidéo Décodée (GiB)",
        f"{size_s['min']:.2f}",
        f"{size_s['median']:.2f}",
        f"{size_s['mean']:.2f}",
        f"{size_s['p90']:.2f}",
        f"{size_s['max']:.2f}",
        f"{size_s['stdev']:.2f}",
    )
    stats_table.add_row(
        "Couplings Certifiés",
        f"{coup_s['min']:.0f}",
        f"{coup_s['median']:.0f}",
        f"{coup_s['mean']:.1f}",
        f"{coup_s['p90']:.0f}",
        f"{coup_s['max']:.0f}",
        f"{coup_s['stdev']:.1f}",
    )

    if show_stats:
        stats_renderable = stats_table
    else:
        stats_renderable = Panel(
            Text.from_markup(
                f"▶ [bold cyan]📊 HISTORIQUE & STATISTIQUES RÉELLES[/bold cyan] ({len(phash_full_records)} MÉDIAS DÉCODÉS)  ──  [dim yellow]Appuyer sur 'h' pour déplier[/dim yellow]"
            ),
            border_style="blue",
        )

    # 6. Recent Completed Media Table (Collapsible, default: collapsed)
    recent_table = Table(
        title="📋 DERNIERS TRAITEMENTS COMPLÉTÉS & CERTIFIÉS",
        title_style="bold cyan",
        expand=True,
        header_style="bold blue",
    )
    recent_table.add_column("Média / Épisode Traité", width=42)
    recent_table.add_column("Statut", justify="center", width=10)
    recent_table.add_column("Taille", justify="right", width=10)
    recent_table.add_column("Durée", justify="right", width=12)
    recent_table.add_column("Couplings", justify="center", width=12)
    recent_table.add_column("Débit CIFS", justify="right", width=12)

    recent_slice = history_records[-RECENT_COMPLETED_DISPLAY_COUNT:] if history_records else []
    for r in reversed(recent_slice):
        stem = r.get("video_stem", "Media")
        stem_disp = stem[:40] + "…" if len(stem) > 40 else stem
        ok = r.get("success", False)
        sz_mb = r.get("file_size_bytes", 0) / BYTES_PER_MIB
        wall = r.get("wall_duration_secs", 0.0)
        c_count = r.get("certified_couplings_count", 0)
        r_rate = r.get("cifs_read_rate_mib_s", 0.0)

        is_cache_or_dry = wall < 1.0 or r_rate > MAX_SENSIBLE_RATE_MIB_S
        if not ok:
            status_disp = "[red]✗ FAIL[/red]"
        elif is_cache_or_dry:
            status_disp = "[yellow]✓ CACHE[/yellow]"
        else:
            status_disp = "[green]✓ OK[/green]"

        # Format rate with sanity check for short-lived/skipped items
        if is_cache_or_dry:
            rate_disp = "[dim]N/A (Cache)[/dim]"
            dur_disp = "< 1s"
        else:
            rate_disp = f"{r_rate:.1f} MB/s"
            dur_disp = format_duration(wall)

        recent_table.add_row(
            stem_disp,
            status_disp,
            f"{sz_mb:.0f} MB",
            dur_disp,
            f"+{c_count}",
            rate_disp,
        )

    if show_recent:
        recent_renderable = recent_table
    else:
        recent_renderable = Panel(
            Text.from_markup(
                f"▶ [bold cyan]📋 DERNIERS TRAITEMENTS COMPLÉTÉS & CERTIFIÉS[/bold cyan] ({len(history_records)} RÉFÉRENCÉS)  ──  [dim yellow]Appuyer sur 'r' pour déplier[/dim yellow]"
            ),
            border_style="cyan",
        )

    # 7. Navigation & Shortcuts Footer
    footer_text = Text()
    footer_text.append(" [↑/↓ (j/k)] ", style="bold black on bright_cyan")
    footer_text.append(" Naviguer  ", style="white")
    footer_text.append(" [Entrée/m] ", style="bold black on bright_yellow")
    footer_text.append(" Menu Actions  ", style="white")
    footer_text.append(" [h] ", style="bold black on bright_blue")
    footer_text.append(" Stats (replier)" if show_stats else " Stats (déplier)", style="white")
    footer_text.append(" [r] ", style="bold black on bright_magenta")
    footer_text.append(" Récents (replier)" if show_recent else " Récents (déplier)", style="white")
    footer_text.append(" [q] ", style="bold black on bright_red")
    footer_text.append(" Quitter", style="white")

    if feedback_msg:
        footer_text.append(f"   ⚡ {feedback_msg}", style="bold bright_yellow")

    footer_panel = Panel(footer_text, border_style="dim")

    # Assemble components
    components = [
        Panel(header_text, style="cyan"),
        tasks_table,
    ]
    if menu_component:
        components.append(menu_component)
    if log_component:
        components.append(log_component)

    components.append(stats_renderable)
    components.append(recent_renderable)
    components.append(footer_panel)

    group = Group(*components)

    return Panel(
        group,
        title="[bold white]pydbsrt top ── Moniteur de Décodage pHash & Ingestion Temps Réel[/bold white]",
        border_style="bright_blue",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="dbsrt top: Real-time TUI Process & Ingestion Monitor")
    parser.add_argument(
        "--events-file",
        type=Path,
        default=DEFAULT_EVENTS_FILE,
        help="Path to batch telemetry events NDJSON log",
    )
    parser.add_argument(
        "--refresh",
        type=float,
        default=DEFAULT_REFRESH_INTERVAL_SECS,
        help="Refresh rate in seconds (default: 1.0)",
    )
    parser.add_argument(
        "--stats",
        action="store_true",
        help="Print statistical analysis snapshot and exit immediately",
    )
    args = parser.parse_args()

    console = Console()
    start_gpu_monitoring_thread()

    if args.stats:
        time.sleep(0.35)  # Allow background monitor to catch initial snapshot
        records = load_historical_records(args.events_file)
        active = scan_active_jobs()
        dashboard = render_dashboard(active, records, show_stats=True, show_recent=True)
        console.print(dashboard)
        stop_gpu_monitoring()
        return 0

    show_stats = False  # Collapsed by default
    show_recent = False  # Collapsed by default
    selected_index = 0
    is_menu_open = False
    is_log_modal_open = False
    feedback_msg: str | None = None
    feedback_time: float = 0.0

    # Prime initial cache synchronously for instant first frame
    initial_records = load_historical_records(args.events_file)
    with TELEMETRY_LOCK:
        SHARED_HISTORICAL_RECORDS = initial_records

    # Start background telemetry thread (1.0s interval)
    STOP_SAMPLER_EVENT.clear()
    sampler_thread = threading.Thread(
        target=_telemetry_sampler_worker,
        args=(args.events_file, args.refresh),
        daemon=True,
    )
    sampler_thread.start()

    try:
        with TerminalInput() as term_in:
            with Live(console=console, auto_refresh=False, screen=True) as live:
                needs_redraw = True
                last_render_time = time.time()
                while True:
                    now = time.time()
                    if feedback_msg and now - feedback_time > FEEDBACK_DISPLAY_TIMEOUT_SECS:
                        feedback_msg = None
                        needs_redraw = True

                    if NEW_DATA_EVENT.is_set():
                        NEW_DATA_EVENT.clear()
                        needs_redraw = True

                    # Refresh at least once per second for the clock
                    if now - last_render_time >= 1.0:
                        needs_redraw = True

                    if needs_redraw:
                        with TELEMETRY_LOCK:
                            active = list(SHARED_ACTIVE_JOBS)
                            records = list(SHARED_HISTORICAL_RECORDS)

                        if active:
                            selected_index = max(0, min(selected_index, len(active) - 1))
                        else:
                            selected_index = 0
                            is_menu_open = False
                            is_log_modal_open = False

                        panel = render_dashboard(
                            active_jobs=active,
                            history_records=records,
                            selected_index=selected_index,
                            show_stats=show_stats,
                            show_recent=show_recent,
                            is_menu_open=is_menu_open,
                            is_log_modal_open=is_log_modal_open,
                            feedback_msg=feedback_msg,
                        )
                        live.update(panel, refresh=True)
                        last_render_time = now
                        needs_redraw = False

                    key = term_in.poll_key(timeout=UI_EVENT_LOOP_POLL_TIMEOUT_SECS)
                    if key is not None:
                        if key in ("q", "Q"):
                            break
                        elif key in ("h", "H"):
                            show_stats = not show_stats
                            needs_redraw = True
                        elif key in ("r", "R"):
                            show_recent = not show_recent
                            needs_redraw = True
                        elif key in ("UP", "k", "K"):
                            with TELEMETRY_LOCK:
                                n_jobs = len(SHARED_ACTIVE_JOBS)
                            if n_jobs > 0:
                                selected_index = max(0, selected_index - 1)
                                needs_redraw = True
                        elif key in ("DOWN", "j", "J"):
                            with TELEMETRY_LOCK:
                                n_jobs = len(SHARED_ACTIVE_JOBS)
                            if n_jobs > 0:
                                selected_index = min(n_jobs - 1, selected_index + 1)
                                needs_redraw = True
                        elif key in ("ENTER", "SPACE", "m", "M"):
                            if is_log_modal_open:
                                is_log_modal_open = False
                            else:
                                with TELEMETRY_LOCK:
                                    has_jobs = bool(SHARED_ACTIVE_JOBS)
                                if has_jobs:
                                    is_menu_open = not is_menu_open
                            needs_redraw = True
                        elif key in ("ESC", "c", "C"):
                            if is_log_modal_open:
                                is_log_modal_open = False
                                needs_redraw = True
                            elif is_menu_open:
                                is_menu_open = False
                                needs_redraw = True
                        elif is_menu_open:
                            with TELEMETRY_LOCK:
                                active_copy = list(SHARED_ACTIVE_JOBS)
                            if active_copy:
                                target_job = active_copy[max(0, min(selected_index, len(active_copy) - 1))]
                                if key in ("p", "P"):
                                    _, msg = toggle_pause_job(target_job.pid)
                                    feedback_msg = msg
                                    feedback_time = time.time()
                                    needs_redraw = True
                                elif key in ("s", "S"):
                                    _, msg = stop_job(target_job.pid)
                                    feedback_msg = msg
                                    feedback_time = time.time()
                                    is_menu_open = False
                                    needs_redraw = True
                                elif key in ("l", "L"):
                                    is_log_modal_open = True
                                    is_menu_open = False
                                    needs_redraw = True
    except KeyboardInterrupt:
        console.print("\n[yellow]Arrêt du moniteur dbsrt top.[/yellow]")
        return 0
    finally:
        STOP_SAMPLER_EVENT.set()
        stop_gpu_monitoring()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
