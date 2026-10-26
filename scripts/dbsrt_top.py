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
import statistics
import subprocess
import threading
import time
from dataclasses import dataclass
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

# In-memory tracking for instantaneous delta rates
PREVIOUS_IO_SAMPLES: Dict[int, tuple[float, int]] = {}
FILE_SIZE_CACHE: Dict[Path, int] = {}


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

    for proc in psutil.process_iter(
        ["pid", "ppid", "name", "cmdline", "create_time", "cpu_percent", "memory_info", "io_counters"]
    ):
        try:
            cmdline = proc.info.get("cmdline") or []
            if not cmdline:
                continue

            cmd_str = " ".join(cmdline)
            is_dbsrt = "dbsrt" in cmd_str and "media:add" in cmd_str
            is_ffmpeg = "ffmpeg" in cmd_str and ("-f rawvideo" in cmd_str or "scale" in cmd_str)

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
            else:
                engine = "GPU (VA-API)" if "-hwaccel vaapi" in cmd_str else "CPU (Software)"

            create_time = proc.info.get("create_time", now)
            elapsed = max(now - create_time, 0.1)

            total_bytes = 0
            if target_path:
                if target_path in FILE_SIZE_CACHE:
                    total_bytes = FILE_SIZE_CACHE[target_path]
                else:
                    try:
                        total_bytes = target_path.stat().st_size
                        FILE_SIZE_CACHE[target_path] = total_bytes
                    except OSError:
                        pass

            io_counters = proc.info.get("io_counters")
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

            mem_info = proc.info.get("memory_info")
            ram_rss = (mem_info.rss / BYTES_PER_MIB) if mem_info else 0.0
            cpu_pct = proc.info.get("cpu_percent") or 0.0

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

    return list(by_stem.values())


def render_dashboard(
    active_jobs: List[ActiveJobTelemetry],
    history_records: List[Dict[str, Any]],
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

    # 2. Active Tasks Table
    tasks_table = Table(
        title="⚡ TÂCHES EN COURS (DÉCODAGE & PHASH TEMPS RÉEL)",
        title_style="bold yellow",
        expand=True,
        header_style="bold cyan",
    )
    tasks_table.add_column("PID", justify="right", width=8)
    tasks_table.add_column("Média / Release", width=34)
    tasks_table.add_column("Taille", justify="right", width=10)
    tasks_table.add_column("Moteur", justify="center", width=16)
    tasks_table.add_column("Écoulé", justify="right", width=9)
    tasks_table.add_column("Débit Effectif", justify="right", width=14)
    tasks_table.add_column("Progression", width=28)
    tasks_table.add_column("ETA Restant", justify="right", width=12)

    if not active_jobs:
        tasks_table.add_row(
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
        for job in active_jobs:
            name_truncated = (
                job.media_name[:MAX_STEM_DISPLAY_CHARS] + "…"
                if len(job.media_name) > MAX_STEM_DISPLAY_CHARS
                else job.media_name
            )
            size_styled = f"{job.total_bytes / BYTES_PER_MIB:.0f} MB" if job.total_bytes > 0 else "-"
            engine_styled = (
                f"[green]{job.engine}[/green]" if "VA-API" in job.engine else f"[yellow]{job.engine}[/yellow]"
            )
            rate_styled = f"{job.current_rate_mib_s:.1f} MB/s"
            eta_str = format_duration(job.eta_secs) if job.eta_secs is not None else "~ incalculable"

            tasks_table.add_row(
                str(job.pid),
                name_truncated,
                size_styled,
                engine_styled,
                format_duration(job.elapsed_secs),
                rate_styled,
                format_progress_bar(job.progress_ratio),
                eta_str,
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
                "[bold cyan]Σ[/bold cyan]",
                f"[bold white]TOTAL CUMULÉ ({len(active_jobs)} flux)[/bold white]",
                tot_size_str,
                engine_summary,
                format_duration(max_elapsed),
                f"[bold magenta]{total_active_rate:.1f} MB/s[/bold magenta]",
                format_progress_bar(avg_ratio),
                format_duration(max_eta) if max_eta is not None else "~ incalculable",
            )

    # 3. Statistical Analysis Table (Separated pHash Full Decode)
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

    # 4. Recent Completed Media Table
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
        status_disp = "[green]✓ OK[/green]" if ok else "[red]✗ FAIL[/red]"
        sz_mb = r.get("file_size_bytes", 0) / BYTES_PER_MIB
        wall = r.get("wall_duration_secs", 0.0)
        c_count = r.get("certified_couplings_count", 0)
        r_rate = r.get("cifs_read_rate_mib_s", 0.0)

        # Format rate with sanity check for short-lived/skipped items
        if wall < 1.0 or r_rate > MAX_SENSIBLE_RATE_MIB_S:
            rate_disp = "[dim]N/A (Cache)[/dim]"
        else:
            rate_disp = f"{r_rate:.1f} MB/s"

        recent_table.add_row(
            stem_disp,
            status_disp,
            f"{sz_mb:.0f} MB",
            format_duration(wall),
            f"+{c_count}",
            rate_disp,
        )

    # Assemble main group
    group = Group(
        Panel(header_text, style="cyan"),
        tasks_table,
        stats_table,
        recent_table,
    )

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
        dashboard = render_dashboard(active, records)
        console.print(dashboard)
        stop_gpu_monitoring()
        return 0

    try:
        with Live(console=console, refresh_per_second=int(1.0 / max(args.refresh, 0.2)), screen=True) as live:
            while True:
                active = scan_active_jobs()
                records = load_historical_records(args.events_file)
                panel = render_dashboard(active, records)
                live.update(panel)
                time.sleep(args.refresh)
    except KeyboardInterrupt:
        console.print("\n[yellow]Arrêt du moniteur dbsrt top.[/yellow]")
        return 0
    finally:
        stop_gpu_monitoring()


if __name__ == "__main__":
    raise SystemExit(main())
