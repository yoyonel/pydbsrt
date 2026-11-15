"""Child process-group watchdog, hot-reload detection, and single-video `dbsrt media:add` runner."""

from __future__ import annotations

import atexit
import contextlib
import json
import os
import signal
import subprocess
import sys
import threading
import time
import traceback
import urllib.parse
from pathlib import Path
from typing import Dict, List, Set, TextIO

from ._facade import facade_attr
from .models import (
    DEFAULT_AHASH_DIR,
    DEFAULT_FFMPEG_THREADS,
    DEFAULT_LOG_PATH,
    DEFAULT_MAX_EXTRACTION_SECS,
    DEFAULT_PHASH_DIR,
    DEFAULT_STALL_TIMEOUT_SECS,
    DEFAULT_SUBTITLES_DIR,
    WATCHDOG_POLL_INTERVAL_SECS,
    VideoTarget,
)
from .moviehash_io import build_stream_url, target_moviehash
from .telemetry import _event_record, get_process_group_cpu_ticks, safe_worker_log_stem

ACTIVE_CHILD_PGIDS: Set[int] = set()
ACTIVE_CHILD_LOCK = threading.Lock()
RELOAD_REQUESTED = threading.Event()

_PACKAGE_DIR: Path = Path(__file__).resolve().parent
DEFAULT_FACADE_SCRIPT_PATH: Path = (_PACKAGE_DIR.parent / "o2switch_batch_worker.py").resolve()
SCRIPT_PATH: Path = DEFAULT_FACADE_SCRIPT_PATH if DEFAULT_FACADE_SCRIPT_PATH.exists() else Path(__file__).resolve()


def _max_package_mtime(script_path: Path) -> float:
    mtime = script_path.stat().st_mtime
    if script_path.resolve() == DEFAULT_FACADE_SCRIPT_PATH and _PACKAGE_DIR.is_dir():
        for mod_file in _PACKAGE_DIR.glob("*.py"):
            with contextlib.suppress(OSError):
                mtime = max(mtime, mod_file.stat().st_mtime)
    return mtime


SCRIPT_MTIME: float = _max_package_mtime(SCRIPT_PATH)


def _kill_active_children(sig: int = signal.SIGTERM) -> None:
    """Terminates all tracked child process groups (safe inside signal handlers)."""
    for pgid in list(ACTIVE_CHILD_PGIDS):
        with contextlib.suppress(ProcessLookupError):
            os.killpg(pgid, sig)


def _append_log_banner(msg: str) -> None:
    print(msg, flush=True)
    with contextlib.suppress(Exception):
        DEFAULT_LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with open(DEFAULT_LOG_PATH, "a", encoding="utf-8") as fh:
            fh.write(msg)


def _handle_signal(sig: int, _frame: object) -> None:
    sig_name = signal.Signals(sig).name
    _append_log_banner(
        f"\n⚠️ [SIGNAL] Process received signal {sig_name} ({sig}) at {time.strftime('%Y-%m-%d %H:%M:%S')}\n"
    )
    _kill_active_children()
    sys.exit(128 + sig)


def _handle_uncaught_exception(exc_type: type, exc_value: BaseException, exc_tb: object) -> None:
    msg = "".join(traceback.format_exception(exc_type, exc_value, exc_tb))  # type: ignore[arg-type]
    _append_log_banner(f"\n❌ [CRASH] Uncaught Exception at {time.strftime('%Y-%m-%d %H:%M:%S')}:\n{msg}\n")


def _on_exit() -> None:
    _kill_active_children()
    print(f"\nℹ️ [EXIT] Batch worker exiting at {time.strftime('%Y-%m-%d %H:%M:%S')}\n", flush=True)


_HANDLERS_INSTALLED = False


def install_process_handlers(force: bool = False) -> None:
    """Installs process-wide exit and signal handlers once when invoked as a CLI daemon."""
    global _HANDLERS_INSTALLED
    if _HANDLERS_INSTALLED or ("pytest" in sys.modules and not force):
        return
    _HANDLERS_INSTALLED = True
    atexit.register(_on_exit)
    sys.excepthook = _handle_uncaught_exception
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT, signal.SIGQUIT):
        with contextlib.suppress(Exception):
            signal.signal(sig, _handle_signal)


def script_updated() -> bool:
    """Returns True when the facade script or any `batch_worker/` module was modified on disk."""
    cur_path = facade_attr("SCRIPT_PATH", SCRIPT_PATH)
    recorded_mtime = facade_attr("SCRIPT_MTIME", SCRIPT_MTIME)
    try:
        return _max_package_mtime(cur_path) != recorded_mtime
    except OSError:
        return False


def build_child_env(nas_user: str, nas_pass: str, bin_dir: Path) -> Dict[str, str]:
    """Builds child environment passing FTP credentials out-of-band without leaking `NAS_PASS`."""
    env = os.environ.copy()
    env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"
    env["DBSRT_FTP_USER"] = nas_user
    env["DBSRT_FTP_PASSWORD"] = nas_pass
    env.setdefault("DBSRT_FFMPEG_THREADS", DEFAULT_FFMPEG_THREADS)
    env.pop("NAS_PASS", None)
    return env


def build_media_add_cmd(
    dbsrt_bin: str,
    stream_url: str,
    filename: str,
    phash_dir: str,
    ahash_dir: str,
    subtitles_dir: str,
    moviehash: str | None,
    mode: str = "both",
) -> List[str]:
    """Builds `dbsrt media:add` argv honoring `mode` (`phash`, `ahash` -> `--skip-phash`, or `both`)."""
    cmd = [dbsrt_bin, "media:add", stream_url, f"--media-name={filename}", "--hwaccel", "disabled"]
    if mode == "ahash":
        cmd += ["--skip-phash", "--ahash-dir", ahash_dir]
    elif mode == "phash":
        cmd += ["--phash-dir", phash_dir]
    else:
        cmd += ["--phash-dir", phash_dir, "--ahash-dir", ahash_dir]
    cmd += ["--subtitles-dir", subtitles_dir, "--skip-certify"]
    if moviehash:
        cmd += ["--moviehash", moviehash]
    return cmd


def _kill_child(proc: subprocess.Popen[bytes], out_f: TextIO, reason: str) -> None:
    out_f.write(f"\n❌ [WATCHDOG] Killed process: {reason}.\n")
    out_f.flush()
    with contextlib.suppress(ProcessLookupError):
        os.killpg(proc.pid, signal.SIGKILL)
    proc.wait()


def _watch_child(proc: subprocess.Popen[bytes], out_f: TextIO, start_time: float, max_extraction_secs: int) -> str:
    """Monitors a `dbsrt` child process group for CPU stall (`60s`) or wall timeout."""
    last_cpu_ticks = 0
    stall_duration = 0
    while (returncode := proc.poll()) is None:
        time.sleep(WATCHDOG_POLL_INTERVAL_SECS)
        cur_cpu_ticks = get_process_group_cpu_ticks(proc.pid)
        stall_duration = 0 if cur_cpu_ticks != last_cpu_ticks else stall_duration + 1
        last_cpu_ticks = cur_cpu_ticks
        if stall_duration >= DEFAULT_STALL_TIMEOUT_SECS:
            _kill_child(proc, out_f, f"zero progress / activity for {stall_duration}s (Stall Detector)")
            return "STALLED"
        if time.time() - start_time >= max_extraction_secs:
            _kill_child(proc, out_f, f"exceeded max duration of {max_extraction_secs}s")
            return "TIMEOUT"
    if returncode == -signal.SIGTERM and RELOAD_REQUESTED.is_set():
        return "RELOAD"
    return "COMPLETED" if returncode == 0 else "ERROR"


def process_single_video(
    target: VideoTarget,
    stream_base: str,
    dbsrt_bin: str,
    child_env: Dict[str, str],
    log_lock: threading.Lock,
    events_log: Path,
    max_extraction_secs: int = DEFAULT_MAX_EXTRACTION_SECS,
    active_workers: int | None = None,
    desired_workers: int | None = None,
    ahash_dir: str = DEFAULT_AHASH_DIR,
    mode: str = "both",
) -> Dict[str, object]:
    """Processes a single video target by invoking `dbsrt media:add` over FTP or HTTP Range."""
    moviehash_fn = facade_attr("target_moviehash", target_moviehash)
    moviehash = moviehash_fn(target, stream_base, child_env)
    cmd = build_media_add_cmd(
        dbsrt_bin=dbsrt_bin,
        stream_url=build_stream_url(stream_base, target.rel_path),
        filename=target.filename,
        phash_dir=facade_attr("DEFAULT_PHASH_DIR", DEFAULT_PHASH_DIR),
        ahash_dir=ahash_dir,
        subtitles_dir=facade_attr("DEFAULT_SUBTITLES_DIR", DEFAULT_SUBTITLES_DIR),
        moviehash=moviehash,
        mode=mode,
    )

    start_time = time.time()
    print(f"--> [DEBUG] Launching dbsrt for {target.filename} (mode={mode})...", flush=True)
    log_path = Path("logs") / "workers" / f"{safe_worker_log_stem(target.filename, moviehash)}.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    status = "ERROR"
    returncode: int | None = None
    try:
        with open(log_path, "w", encoding="utf-8") as out_f:
            proc = subprocess.Popen(cmd, stdout=out_f, stderr=subprocess.STDOUT, env=child_env, start_new_session=True)
            with ACTIVE_CHILD_LOCK:
                ACTIVE_CHILD_PGIDS.add(proc.pid)
            try:
                status = _watch_child(proc, out_f, start_time, max_extraction_secs)
                returncode = proc.returncode
            finally:
                with ACTIVE_CHILD_LOCK:
                    ACTIVE_CHILD_PGIDS.discard(proc.pid)
    except Exception as exc:
        with open(log_path, "a", encoding="utf-8") as out_f:
            out_f.write(f"\n❌ Execution exception: {exc}\n")

    print(f"--> [DEBUG] dbsrt {status} (returncode {returncode}) for {target.filename}", flush=True)
    nas_host = urllib.parse.urlsplit(stream_base).hostname or ""
    record = _event_record(
        target,
        start_time,
        status,
        log_path,
        nas_host,
        child_env.get("DBSRT_FTP_PASSWORD", ""),
        moviehash=moviehash,
        active_workers=active_workers,
        desired_workers=desired_workers,
    )
    if status == "RELOAD":
        return record
    with log_lock:
        events_log.parent.mkdir(parents=True, exist_ok=True)
        with open(events_log, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(record) + "\n")
    return record
