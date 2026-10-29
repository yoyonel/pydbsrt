#!/usr/bin/env python3
"""o2switch Remote Batch Processing Worker.

Orchestrates multi-worker parallel pHash extraction and subtitle ingestion on o2switch
shared hosting (CloudLinux LVE 12 vCPUs) by streaming directly from the Synology NAS over
FTP with FFmpeg (Zero local video disk copy). NAS credentials reach dbsrt through the
DBSRT_FTP_USER / DBSRT_FTP_PASSWORD environment variables, never inside URLs or argv.
"""

from __future__ import annotations

import argparse
import atexit
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time
import traceback
import unicodedata
import urllib.parse
from pathlib import Path
from typing import Dict, List, Set, Tuple

# Process groups of in-flight dbsrt children: they run in their own session, so they must be
# killed explicitly on shutdown or they keep running as orphans.
ACTIVE_CHILD_PGIDS: Set[int] = set()
ACTIVE_CHILD_LOCK = threading.Lock()


def _kill_active_children(sig: int = signal.SIGTERM) -> None:
    # No lock here: called from a signal handler, list() of a set is atomic under the GIL.
    for pgid in list(ACTIVE_CHILD_PGIDS):
        try:
            os.killpg(pgid, sig)
        except ProcessLookupError:
            pass


def _handle_signal(sig: int, _frame: object) -> None:
    sig_name = signal.Signals(sig).name
    msg = f"\n⚠️ [SIGNAL] Process received signal {sig_name} ({sig}) at {time.strftime('%Y-%m-%d %H:%M:%S')}\n"
    print(msg, flush=True)
    try:
        with open("logs/batch_worker.log", "a", encoding="utf-8") as f:
            f.write(msg)
    except Exception:
        pass
    _kill_active_children()
    sys.exit(128 + sig)


def _handle_uncaught_exception(exc_type: type, exc_value: BaseException, exc_tb: object) -> None:
    msg = "".join(traceback.format_exception(exc_type, exc_value, exc_tb))  # type: ignore
    err_str = f"\n❌ [CRASH] Uncaught Exception at {time.strftime('%Y-%m-%d %H:%M:%S')}:\n{msg}\n"
    print(err_str, flush=True)
    try:
        with open("logs/batch_worker.log", "a", encoding="utf-8") as f:
            f.write(err_str)
    except Exception:
        pass


@atexit.register
def _on_exit() -> None:
    _kill_active_children()
    msg = f"\nℹ️ [EXIT] Batch worker exiting at {time.strftime('%Y-%m-%d %H:%M:%S')}\n"
    print(msg, flush=True)


sys.excepthook = _handle_uncaught_exception
for _sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT, signal.SIGQUIT):
    try:
        signal.signal(_sig, _handle_signal)
    except Exception:
        pass


DEFAULT_DESIRED_WORKERS_PATH: Path = Path("run/desired_workers")
MIN_CONCURRENT_WORKERS: int = 1
MAX_CONCURRENT_WORKERS: int = 8
TARGET_QUEUE_POLL_INTERVAL_SECS: float = 0.5
# A redeployed worker script is picked up between videos: the queue drains, then the process re-execs.
SCRIPT_PATH: Path = Path(__file__).resolve()
SCRIPT_MTIME: float = SCRIPT_PATH.stat().st_mtime
RELOAD_REQUESTED = threading.Event()
# Longest wait for in-flight videos after a redeploy; slower ones are killed and requeued by the re-exec
RELOAD_DRAIN_SECS: float = 600.0
FTP_LIST_ATTEMPTS: int = 3
FTP_LIST_RETRY_DELAY_SECS: float = 30.0


def script_updated() -> bool:
    """True when the worker script on disk differs from the one this process runs."""
    try:
        return SCRIPT_PATH.stat().st_mtime != SCRIPT_MTIME
    except OSError:
        return False


def read_desired_workers(default_workers: int) -> int:
    """Reads desired concurrency dynamically from run/desired_workers."""
    if DEFAULT_DESIRED_WORKERS_PATH.is_file():
        try:
            val = int(DEFAULT_DESIRED_WORKERS_PATH.read_text(encoding="utf-8").strip())
            return max(MIN_CONCURRENT_WORKERS, min(val, MAX_CONCURRENT_WORKERS))
        except Exception:
            pass
    return default_workers


def _load_env_config(path: Path) -> Dict[str, str]:
    """Loads key-value pairs from an optional local unversioned environment file."""
    if not path.is_file():
        return {}
    config: Dict[str, str] = {}
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            if "=" in stripped:
                k, v = stripped.split("=", 1)
                config[k.strip()] = v.strip().strip("'\"")
    except Exception:
        pass
    return config


_CONFIG_ENV: Dict[str, str] = _load_env_config(Path("etc/.nas_config"))
if not _CONFIG_ENV:
    _CONFIG_ENV = _load_env_config(Path("etc/.env"))
if not _CONFIG_ENV:
    _CONFIG_ENV = _load_env_config(Path(".env"))

# Operational & System Constants
DEFAULT_WORKERS: int = 4
DEFAULT_NAS_HOST: str = os.getenv("NAS_HOST") or _CONFIG_ENV.get("NAS_HOST", "")
DEFAULT_NAS_USER: str = os.getenv("NAS_USER") or _CONFIG_ENV.get("NAS_USER", "")
DEFAULT_NAS_BASE: str = "/tvshow"
DEFAULT_FTP_PORT: int = 21
DEFAULT_FFMPEG_THREADS: str = "2"
DEFAULT_STALL_TIMEOUT_SECS: int = 60
DEFAULT_MAX_EXTRACTION_SECS: int = 1800
DEFAULT_COUPLINGS_PATH: str = "data/SRT/couplings.json"
DEFAULT_PHASH_DIR: str = "data/__PHASH__"
DEFAULT_SUBTITLES_DIR: str = "data/SRT"
DEFAULT_EVENTS_LOG_PATH: str = "data/reports/batch_tvshow_nightly.jsonl"
DEFAULT_TARGETS_PATH: str = "data/reports/selected_batch_targets.json"
DEFAULT_BATCH_QUEUE_PATH: str = "run/batch_queue.json"
DEFAULT_BATCH_ABORTED_PATH: str = "run/batch_aborted"
# Remote-only list of NAS path prefixes never processed, even by --retry-failed (e.g. unreadable folders)
DEFAULT_EXCLUDE_PATH: str = "etc/batch_exclude.txt"
# A systemic failure (bad ffmpeg flag, NAS down) fails every target within seconds
FAST_FAILURE_SECS: float = 10.0
MAX_FAST_FAILURE_STREAK: int = 50
# Videos failing this many times in a row (corrupt, unreadable on the NAS) are no longer queued
MAX_FAILED_ATTEMPTS: int = 3
VIDEO_EXTENSIONS: Tuple[str, ...] = (".mkv", ".mp4", ".webm", ".avi")


def resolve_nas_password() -> str:
    """Resolves the NAS password from NAS_PASS env, etc/.nas_pass or the env config file."""
    if os.environ.get("NAS_PASS"):
        return os.environ["NAS_PASS"]
    pass_file = Path("etc/.nas_pass")
    if pass_file.is_file():
        return pass_file.read_text(encoding="utf-8").strip()
    return _CONFIG_ENV.get("NAS_PASS", "")


def load_certified_stems(couplings_file: Path) -> Set[str]:
    """Loads all media stems already certified in couplings.json."""
    stems: Set[str] = set()
    if couplings_file.is_file():
        try:
            data = json.loads(couplings_file.read_text(encoding="utf-8"))
            for item in data.get("couplings", []):
                media_name = item.get("media_name", "")
                if media_name:
                    stems.add(media_name)
        except Exception as err:
            print(f"⚠️ Warning loading {couplings_file}: {err}", file=sys.stderr)
    return stems


def load_existing_phash_stems(phash_dir: Path) -> Set[str]:
    """Loads all media stems with existing .phash files on disk."""
    stems: Set[str] = set()
    if phash_dir.is_dir():
        for p in phash_dir.glob("*.phash"):
            try:
                if p.stat().st_size == 0:
                    continue
            except OSError:
                continue
            raw_name = urllib.parse.unquote(p.name)
            if raw_name.endswith(".phash"):
                raw_name = raw_name[:-6]
            raw_name = re.sub(r"\.\d+(\.\d+)?fps$", "", raw_name)
            for ext in VIDEO_EXTENSIONS:
                if raw_name.lower().endswith(ext):
                    raw_name = raw_name[: -len(ext)]
                    break
            stems.add(raw_name)
            stems.add(p.stem)
    return stems


def list_nas_videos_ftp(
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    base_dir: str = DEFAULT_NAS_BASE,
    ftp_port: int = DEFAULT_FTP_PORT,
    max_depth: int = 8,
) -> List[Tuple[str, str, int]]:
    """Discovers video files on Synology NAS using standard FTP MLSD protocol.

    Transient FTP errors restart the whole walk, so a dropped connection never yields a partial list.

    Returns:
        List of tuples: (relative_path, filename, size_bytes).
    """
    import ftplib

    for attempt in range(1, FTP_LIST_ATTEMPTS + 1):
        try:
            return _walk_nas_videos_ftp(nas_host, nas_user, nas_pass, base_dir, ftp_port, max_depth)
        except (OSError, EOFError, ftplib.error_temp, ftplib.error_reply, ftplib.error_proto) as err:
            if attempt == FTP_LIST_ATTEMPTS:
                raise
            print(f"⚠️ NAS FTP listing failed ({err}), retry {attempt}/{FTP_LIST_ATTEMPTS - 1}", file=sys.stderr)
            time.sleep(FTP_LIST_RETRY_DELAY_SECS * attempt)
    return []


def _walk_nas_videos_ftp(
    nas_host: str, nas_user: str, nas_pass: str, base_dir: str, ftp_port: int, max_depth: int
) -> List[Tuple[str, str, int]]:
    import ftplib
    import posixpath

    clean_base = base_dir.lstrip("/")
    ftp = ftplib.FTP()
    ftp.connect(nas_host, ftp_port, timeout=15)
    ftp.login(nas_user, nas_pass)
    ftp.encoding = "utf-8"

    results: List[Tuple[str, str, int]] = []
    stack: List[str] = [clean_base]

    while stack:
        curr = stack.pop()
        try:
            entries = list(ftp.mlsd(curr))
        except ftplib.error_perm:
            # 550 on a folder the account cannot read: skip it, the rest of the library stays usable
            continue
        for name, facts in entries:
            if name in (".", "..", "EMPTY_DIRS", "SRT", "@eaDir") or name.startswith("."):
                continue
            full_path = posixpath.join(curr, name)
            ftype = facts.get("type", "")
            if ftype == "dir":
                if full_path.count("/") <= max_depth:
                    stack.append(full_path)
            elif ftype == "file":
                if any(name.lower().endswith(ext) for ext in VIDEO_EXTENSIONS):
                    size = int(facts.get("size", 0))
                    rel = "/" + full_path if not full_path.startswith("/") else full_path
                    # The NAS mixes NFC and NFD names: the path keeps the raw name the FTP server
                    # expects, the media name is NFC like the rest of the catalog
                    results.append((rel, unicodedata.normalize("NFC", name), size))

    try:
        ftp.quit()
    except Exception:
        pass
    return results


def load_targets_file(targets_path: Path, nas_base: str, skip_stems: Set[str]) -> List[Tuple[str, str, int]]:
    """Loads explicit targets (list of paths or {path, filename, stem, size_bytes} dicts).

    Paths are rebased on `nas_base`; stems in `skip_stems` are dropped. Raises on unreadable JSON.
    """
    targets: List[Tuple[str, str, int]] = []
    for item in json.loads(targets_path.read_text(encoding="utf-8")):
        if isinstance(item, dict):
            p_str = item.get("path", "")
            filename = item.get("filename", Path(p_str).name)
            stem = item.get("stem", Path(p_str).stem)
            size_bytes = item.get("size_bytes", 0)
        else:
            p_str = str(item)
            filename = Path(p_str).name
            stem = Path(p_str).stem
            size_bytes = 0
        if stem in skip_stems:
            continue
        idx = p_str.find(nas_base)
        rel_path = p_str[idx:] if idx != -1 else p_str
        targets.append((rel_path, filename, size_bytes))
    return targets


def resolve_retry_targets(events_log: Path, skip_stems: Set[str]) -> List[Tuple[str, str, int]]:
    """Returns videos whose last outcome in the events log is a failure, minus `skip_stems`."""
    last_outcome: Dict[str, Tuple[bool, Tuple[str, str, int]]] = {}
    if events_log.is_file():
        for line in events_log.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            stem = unicodedata.normalize("NFC", rec.get("video_stem", ""))
            rel_path = rec.get("video_rel_path", "")
            if stem and rel_path:
                item = (rel_path, Path(rel_path).name, rec.get("file_size_bytes", 0))
                last_outcome[stem] = (bool(rec.get("success", False)), item)
    return [item for stem, (ok, item) in last_outcome.items() if not ok and stem not in skip_stems]


def load_blacklisted_stems(events_log: Path, max_attempts: int = MAX_FAILED_ATTEMPTS) -> Set[str]:
    """Returns NFC stems whose last `max_attempts` outcomes in the events log are all failures."""
    streaks: Dict[str, int] = {}
    if events_log.is_file():
        for line in events_log.read_text(encoding="utf-8").splitlines():
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            stem = unicodedata.normalize("NFC", rec.get("video_stem", ""))
            if stem:
                streaks[stem] = 0 if rec.get("success") else streaks.get(stem, 0) + 1
    return {stem for stem, count in streaks.items() if count >= max_attempts}


def load_excluded_prefixes(exclude_file: Path) -> List[str]:
    """Reads NFC NAS path prefixes, one per line; blank lines and '#' comments are ignored."""
    if not exclude_file.is_file():
        return []
    lines = (line.strip() for line in exclude_file.read_text(encoding="utf-8").splitlines())
    return [unicodedata.normalize("NFC", line) for line in lines if line and not line.startswith("#")]


def is_excluded(rel_path: str, prefixes: List[str]) -> bool:
    """True when the NFC form of a NAS path starts with one of the excluded prefixes."""
    return unicodedata.normalize("NFC", rel_path).startswith(tuple(prefixes))


def match_nas_paths(
    targets: List[Tuple[str, str, int]], nas_videos: List[Tuple[str, str, int]]
) -> List[Tuple[str, str, int]]:
    """Replaces each target with the NAS entry of the same NFC stem; unknown targets are dropped.

    Targets selected on a CIFS mount or replayed from the events log can carry NFC-normalized or
    8.3-mangled names that the FTP server rejects: only the FTP listing holds the real paths.
    """
    by_stem = {Path(video[1]).stem: video for video in nas_videos}
    matched = (by_stem.get(unicodedata.normalize("NFC", Path(target[1]).stem)) for target in targets)
    return list(dict.fromkeys(video for video in matched if video))


def write_batch_queue_state(total: int, completed: int, failed: int, active_workers: int) -> None:
    """Persists volatile batch queue telemetry to disk for dbsrt top and monitoring."""
    try:
        q_path = Path(DEFAULT_BATCH_QUEUE_PATH)
        q_path.parent.mkdir(parents=True, exist_ok=True)
        rem = max(0, total - (completed + failed))
        pct = round(((completed + failed) / total * 100.0), 1) if total > 0 else 0.0
        state = {
            "total_targets": total,
            "completed_targets": completed,
            "failed_targets": failed,
            "remaining_targets": rem,
            "progress_pct": pct,
            "active_workers": active_workers,
            "is_active": rem > 0 and active_workers > 0,
            "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        tmp_path = q_path.with_suffix(".tmp")
        tmp_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
        tmp_path.replace(q_path)
    except Exception:
        pass


def get_process_group_cpu_ticks(pgid: int, proc_root: Path = Path("/proc")) -> int:
    """Sums user+system CPU ticks of every live process in a process group (dbsrt and its ffmpeg)."""
    total_ticks = 0
    for stat_path in proc_root.glob("[0-9]*/stat"):
        try:
            content = stat_path.read_text(encoding="utf-8")
        except OSError:
            continue
        # After "comm)": fields[0]=state, fields[2]=pgrp, fields[11]=utime, fields[12]=stime.
        fields = content[content.rfind(")") + 2 :].split()
        try:
            if len(fields) > 12 and int(fields[2]) == pgid:
                total_ticks += int(fields[11]) + int(fields[12])
        except ValueError:
            continue
    return total_ticks


def build_child_env(nas_user: str, nas_pass: str, bin_dir: Path) -> Dict[str, str]:
    """Builds the dbsrt child environment: FTP credentials, decode threads and bin PATH."""
    env = os.environ.copy()
    env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"
    env["DBSRT_FTP_USER"] = nas_user
    # Password lands in the ffmpeg argv: FFmpeg (checked up to 8.1) cannot read protocol options
    # from a file, and o2switch mounts /proc with hidepid=2 so other accounts cannot see it.
    env["DBSRT_FTP_PASSWORD"] = nas_pass
    env.setdefault("DBSRT_FFMPEG_THREADS", DEFAULT_FFMPEG_THREADS)
    env.pop("NAS_PASS", None)
    return env


def build_stream_url(stream_base: str, rel_path: str) -> str:
    """Joins the FTP base and a raw NAS path.

    Paths come raw from MLSD, the targets file and the events log, and FFmpeg
    libavformat/ftp.c expects them verbatim: never URL-decode them (a literal `%41` must stay).
    """
    return f"{stream_base}{rel_path if rel_path.startswith('/') else '/' + rel_path}"


def process_single_video(
    video_info: Tuple[str, str, int],
    stream_base: str,
    dbsrt_bin: str,
    child_env: Dict[str, str],
    log_lock: threading.Lock,
    events_log: Path,
    max_extraction_secs: int = DEFAULT_MAX_EXTRACTION_SECS,
) -> Dict[str, object]:
    """Processes a single video by invoking dbsrt media:add directly over FTP."""
    rel_path, filename, file_size = video_info
    stem = Path(filename).stem

    stream_url = build_stream_url(stream_base, rel_path)

    base_cmd = [
        dbsrt_bin,
        "media:add",
        stream_url,
        "--media-name",
        filename,
        "--hwaccel",
        "disabled",
        "--phash-dir",
        DEFAULT_PHASH_DIR,
        "--subtitles-dir",
        DEFAULT_SUBTITLES_DIR,
        "--skip-certify",
    ]

    start_time = time.time()
    start_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(start_time))
    print(f"--> [DEBUG] Launching dbsrt for {filename}...", flush=True)

    log_path = Path("logs") / "workers" / f"{stem}.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    status = "ERROR"
    returncode: int | None = None
    try:
        with open(log_path, "w", encoding="utf-8") as out_f:
            proc = subprocess.Popen(
                base_cmd,
                stdout=out_f,
                stderr=subprocess.STDOUT,
                env=child_env,
                start_new_session=True,
            )
            with ACTIVE_CHILD_LOCK:
                ACTIVE_CHILD_PGIDS.add(proc.pid)
            try:
                last_cpu_ticks = 0
                stall_duration = 0
                while True:
                    returncode = proc.poll()
                    if returncode is not None:
                        status = "COMPLETED" if returncode == 0 else "ERROR"
                        if returncode == -signal.SIGTERM and RELOAD_REQUESTED.is_set():
                            status = "RELOAD"
                        break

                    time.sleep(1.0)

                    cur_cpu_ticks = get_process_group_cpu_ticks(proc.pid)
                    if cur_cpu_ticks != last_cpu_ticks:
                        stall_duration = 0
                        last_cpu_ticks = cur_cpu_ticks
                    else:
                        stall_duration += 1

                    # Stall detector: zero CPU activity in the whole group for >= DEFAULT_STALL_TIMEOUT_SECS
                    if stall_duration >= DEFAULT_STALL_TIMEOUT_SECS:
                        status = "STALLED"
                        out_f.write(
                            f"\n❌ [WATCHDOG] Killed process: zero progress / activity for {stall_duration}s (Stall Detector).\n"
                        )
                    elif time.time() - start_time >= max_extraction_secs:
                        status = "TIMEOUT"
                        out_f.write(
                            f"\n❌ [WATCHDOG] Killed process: exceeded max duration of {max_extraction_secs}s.\n"
                        )
                    else:
                        continue
                    out_f.flush()
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    proc.wait()
                    break
            finally:
                with ACTIVE_CHILD_LOCK:
                    ACTIVE_CHILD_PGIDS.discard(proc.pid)
    except Exception as exc:
        with open(log_path, "a", encoding="utf-8") as out_f:
            out_f.write(f"\n❌ Execution exception: {exc}\n")

    print(f"--> [DEBUG] dbsrt {status} (returncode {returncode}) for {filename}", flush=True)

    end_time = time.time()
    end_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(end_time))
    wall_duration = end_time - start_time
    success = status == "COMPLETED"

    # A completed extraction decodes the whole file, so the average NAS read rate is size / duration.
    actual_bytes = file_size if success else 0
    rate_mib_s = (actual_bytes / (1024 * 1024)) / wall_duration if wall_duration > 0 else 0.0

    record: Dict[str, object] = {
        "video_stem": stem,
        "video_rel_path": rel_path,
        "file_size_bytes": file_size,
        "wall_duration_secs": round(wall_duration, 2),
        "io_read_rate_mib_s": round(rate_mib_s, 2),
        "cifs_read_rate_mib_s": round(rate_mib_s, 2),
        "start_iso": start_iso,
        "end_iso": end_iso,
        "timestamp_iso": end_iso,
        "success": success,
        "status": status,
    }
    if not success and log_path.exists():
        record["error_summary"] = log_path.read_text(encoding="utf-8", errors="ignore")[-300:]

    if status == "RELOAD":
        # Not a failure: no event, so the re-exec requeues it and the blacklist ignores it
        return record

    with log_lock:
        events_log.parent.mkdir(parents=True, exist_ok=True)
        with open(events_log, "a", encoding="utf-8") as f:
            f.write(json.dumps(record) + "\n")

    return record


def next_fast_failure_streak(streak: int, rec: Dict[str, object]) -> int:
    """Counts consecutive failures shorter than FAST_FAILURE_SECS; any other outcome resets it."""
    wall = rec.get("wall_duration_secs", 0.0)
    fast_failure = not rec.get("success") and isinstance(wall, int | float) and wall < FAST_FAILURE_SECS
    return streak + 1 if fast_failure else 0


def abort_batch(remaining_targets: List[Tuple[str, str, int]], reason: str) -> None:
    """Drops the queued targets and leaves run/batch_aborted so the supervisor does not relaunch."""
    remaining_targets.clear()
    print(f"🛑 [ABORT] {reason}", flush=True)
    try:
        aborted_path = Path(DEFAULT_BATCH_ABORTED_PATH)
        aborted_path.parent.mkdir(parents=True, exist_ok=True)
        aborted_path.write_text(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {reason}\n", encoding="utf-8")
    except OSError as err:
        print(f"⚠️ Failed writing {DEFAULT_BATCH_ABORTED_PATH}: {err}", file=sys.stderr)


def execute_dynamic_batch_ingestion(
    targets: List[Tuple[str, str, int]],
    stream_base: str,
    dbsrt_bin: str,
    child_env: Dict[str, str],
    events_log: Path,
    max_extraction_secs: int,
    initial_workers: int,
) -> Tuple[int, int, float]:
    """Executes batch processing with dynamic concurrency scaling.

    Spawns and retires worker threads dynamically based on run/desired_workers,
    allowing real-time scale-up and scale-down without killing active extraction jobs.

    # Arguments
    * `targets`: Ordered list of video targets (relative path, filename, size bytes).
    * `stream_base`: FTP streaming endpoint base URL (no credentials).
    * `dbsrt_bin`: Path to dbsrt binary executable.
    * `child_env`: Environment passed to every dbsrt child.
    * `events_log`: Path to events log file.
    * `max_extraction_secs`: Max wall duration per video.
    * `initial_workers`: Baseline concurrency count.

    # Returns
    * Tuple of (success_count, fail_count, total_duration_secs).
    """
    log_lock = threading.Lock()
    state_lock = threading.Lock()
    success_count = 0
    fail_count = 0
    fast_failure_streak = 0
    remaining_targets = list(targets)
    active_threads: List[threading.Thread] = []

    DEFAULT_DESIRED_WORKERS_PATH.parent.mkdir(parents=True, exist_ok=True)
    if not DEFAULT_DESIRED_WORKERS_PATH.exists():
        try:
            DEFAULT_DESIRED_WORKERS_PATH.write_text(str(initial_workers), encoding="utf-8")
        except Exception:
            pass

    write_batch_queue_state(len(targets), success_count, fail_count, initial_workers)

    def worker_loop() -> None:
        nonlocal success_count, fail_count, fast_failure_streak
        while True:
            with state_lock:
                desired = read_desired_workers(initial_workers)
                if len(active_threads) > desired or not remaining_targets:
                    return
                item = remaining_targets.pop(0)

            try:
                rec = process_single_video(
                    item,
                    stream_base,
                    dbsrt_bin,
                    child_env,
                    log_lock,
                    events_log,
                    max_extraction_secs,
                )
                if rec.get("status") == "RELOAD":
                    return
                with state_lock:
                    if rec.get("success"):
                        success_count += 1
                        print(
                            f"  ✓ [{success_count}/{len(targets)}] {rec['video_stem']} ({rec['wall_duration_secs']}s, {rec['io_read_rate_mib_s']} MB/s)",
                            flush=True,
                        )
                    else:
                        fail_count += 1
                        print(
                            f"  ❌ [{fail_count} FAILED] {rec['video_stem']}: {rec.get('error_summary', 'Unknown error')}",
                            flush=True,
                        )
                    fast_failure_streak = next_fast_failure_streak(fast_failure_streak, rec)
                    if fast_failure_streak == MAX_FAST_FAILURE_STREAK:
                        abort_batch(
                            remaining_targets,
                            f"{fast_failure_streak} consecutive failures under {FAST_FAILURE_SECS:.0f}s: "
                            "systemic error, check logs/workers/ before relaunching",
                        )
                    write_batch_queue_state(len(targets), success_count, fail_count, len(active_threads))
            except Exception as exc:
                with state_lock:
                    fail_count += 1
                    print(f"  ❌ Exception processing {item[1]}: {exc}", flush=True)
                    write_batch_queue_state(len(targets), success_count, fail_count, len(active_threads))

    start_batch = time.time()
    reload_since = 0.0
    try:
        while True:
            with state_lock:
                if remaining_targets and script_updated():
                    print("🔄 [RELOAD] Worker script updated: finishing in-flight videos before re-exec", flush=True)
                    remaining_targets.clear()
                    RELOAD_REQUESTED.set()
                    reload_since = time.time()
                if reload_since and time.time() - reload_since >= RELOAD_DRAIN_SECS:
                    print("🔄 [RELOAD] Drain timeout: stopping in-flight videos, the re-exec requeues them", flush=True)
                    _kill_active_children()
                    reload_since = 0.0
                active_threads = [t for t in active_threads if t.is_alive()]
                desired = read_desired_workers(initial_workers)
                if not remaining_targets and not active_threads:
                    break
                while len(active_threads) < desired and remaining_targets:
                    t = threading.Thread(target=worker_loop, daemon=True)
                    active_threads.append(t)
                    t.start()
                    write_batch_queue_state(len(targets), success_count, fail_count, len(active_threads))

            time.sleep(TARGET_QUEUE_POLL_INTERVAL_SECS)

        for t in active_threads:
            t.join(timeout=30)
    finally:
        write_batch_queue_state(len(targets), success_count, fail_count, 0)

    total_duration = time.time() - start_batch
    return success_count, fail_count, total_duration


def main() -> int:
    """CLI Entry point for o2switch batch processing."""
    parser = argparse.ArgumentParser(description="o2switch Remote Batch Processing Worker")
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS, help="Parallel workers")
    parser.add_argument("--ftp-port", type=int, default=DEFAULT_FTP_PORT, help="NAS FTP port")
    parser.add_argument("--fresh-only", action="store_true", help="Process only virgin unindexed videos")
    parser.add_argument(
        "--retry-failed",
        action="store_true",
        help="Retry only videos whose last run failed in the events log (ignores --targets-file)",
    )
    parser.add_argument(
        "--max-extraction-secs",
        type=int,
        default=DEFAULT_MAX_EXTRACTION_SECS,
        help=f"Maximum extraction wall duration in seconds before watchdog kill (default: {DEFAULT_MAX_EXTRACTION_SECS}s)",
    )
    parser.add_argument("--limit", type=int, default=None, help="Maximum number of videos to process")
    parser.add_argument("--nas-host", default=DEFAULT_NAS_HOST, help="Synology NAS DDNS or IP (env NAS_HOST)")
    parser.add_argument("--nas-user", default=DEFAULT_NAS_USER, help="Synology NAS FTP username (env NAS_USER)")
    parser.add_argument("--nas-base", default=DEFAULT_NAS_BASE, help="NAS base directory")
    parser.add_argument(
        "--targets-file",
        default=DEFAULT_TARGETS_PATH,
        help="JSON file listing target video paths; the NAS is scanned when it does not exist",
    )
    args = parser.parse_args()

    nas_pass = resolve_nas_password()
    missing = [
        name
        for name, value in (("NAS_HOST", args.nas_host), ("NAS_USER", args.nas_user), ("NAS_PASS", nas_pass))
        if not value
    ]
    if missing:
        print(
            f"❌ Error: missing NAS config {', '.join(missing)} (env, etc/.nas_config or etc/.nas_pass).",
            file=sys.stderr,
        )
        return 1

    dbsrt_bin = "./bin/dbsrt" if Path("./bin/dbsrt").exists() else "dbsrt"
    events_log = Path(DEFAULT_EVENTS_LOG_PATH)

    print("=== [1/4] Scanning local catalogs and couplings ===")
    certified_stems = load_certified_stems(Path(DEFAULT_COUPLINGS_PATH))
    existing_phash = load_existing_phash_stems(Path(DEFAULT_PHASH_DIR))
    print(f"✓ Found {len(certified_stems)} certified stems and {len(existing_phash)} .phash files locally.")
    done_stems = {unicodedata.normalize("NFC", stem) for stem in certified_stems | existing_phash}
    blacklisted = load_blacklisted_stems(events_log)
    print(f"✓ Skipping {len(blacklisted)} videos that failed {MAX_FAILED_ATTEMPTS} times in a row.")
    skip_stems = (done_stems if args.fresh_only else set()) | blacklisted

    print(f"=== [2/4] Discovering remote NAS video library ({args.nas_host}:{args.nas_base}) ===")
    all_videos = list_nas_videos_ftp(
        nas_host=args.nas_host,
        nas_user=args.nas_user,
        nas_pass=nas_pass,
        base_dir=args.nas_base,
        ftp_port=args.ftp_port,
    )
    print(f"✓ Discovered {len(all_videos)} video files on NAS.")

    targets: List[Tuple[str, str, int]] = []
    targets_path = Path(args.targets_file)
    targets_loaded = False
    if args.retry_failed:
        # An explicit retry ignores the blacklist
        targets = match_nas_paths(resolve_retry_targets(events_log, done_stems), all_videos)
        targets_loaded = True
        print(f"✓ Found {len(targets)} failed videos to retry under {args.nas_base}.")
    elif targets_path.is_file():
        try:
            targets = match_nas_paths(load_targets_file(targets_path, args.nas_base, skip_stems), all_videos)
            targets_loaded = True
            print(f"✓ Loaded {len(targets)} targets from {targets_path} found on the NAS.")
        except Exception as err:
            print(f"⚠️ Failed reading {targets_path}: {err}. Falling back to remote scan.", file=sys.stderr)

    if not targets_loaded:
        targets = [video for video in all_videos if Path(video[1]).stem not in skip_stems]

    excluded = load_excluded_prefixes(Path(DEFAULT_EXCLUDE_PATH))
    if excluded:
        kept = [video for video in targets if not is_excluded(video[0], excluded)]
        print(f"✓ Excluded {len(targets) - len(kept)} videos listed in {DEFAULT_EXCLUDE_PATH}.")
        targets = kept

    if args.limit and len(targets) > args.limit:
        targets = targets[: args.limit]

    print(f"✓ Selected {len(targets)} target videos to process (fresh_only={args.fresh_only}, limit={args.limit}).")
    if not targets:
        print("🎉 No target videos to process. Library is up-to-date!")
        return 0

    print(f"=== [3/4] Direct FTP Native Streaming Mode ({args.nas_host}:{args.ftp_port}) ===")
    stream_base = f"ftp://{args.nas_host}:{args.ftp_port}"
    child_env = build_child_env(args.nas_user, nas_pass, Path("bin").resolve())

    print(f"=== [4/4] Executing Batch Ingestion ({args.workers} initial parallel workers, dynamic scaling active) ===")
    success_count, fail_count, total_duration = execute_dynamic_batch_ingestion(
        targets=targets,
        stream_base=stream_base,
        dbsrt_bin=dbsrt_bin,
        child_env=child_env,
        events_log=events_log,
        max_extraction_secs=args.max_extraction_secs,
        initial_workers=args.workers,
    )

    if RELOAD_REQUESTED.is_set():
        # Same PID, argv and interpreter flags: pid file and supervisor stay valid; done videos are skipped
        print(f"🔄 [RELOAD] Re-exec after {success_count} ok / {fail_count} failed", flush=True)
        os.execv(sys.executable, sys.orig_argv)

    print("=" * 60)
    print(f"🏁 Batch completed in {total_duration:.1f}s: {success_count} succeeded, {fail_count} failed.")
    return 0 if fail_count == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
