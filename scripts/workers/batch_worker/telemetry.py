"""Log stem sanitization (errno 36), secret redaction, CPU tick accounting, and JSONL telemetry."""

from __future__ import annotations

import hashlib
import json
import re
import sys
import time
import urllib.parse
from pathlib import Path
from typing import Dict, List, Sequence

from ._facade import facade_attr
from .models import (
    BYTES_PER_MIB,
    DEFAULT_BATCH_ABORTED_PATH,
    DEFAULT_BATCH_PAUSED_PATH,
    DEFAULT_BATCH_QUEUE_PATH,
    ERROR_SUMMARY_TAIL_CHARS,
    FAST_FAILURE_SECS,
    LOG_STEM_HASH_HEX_LEN,
    MAX_LOG_STEM_BYTES,
    MOVIEHASH_RE,
    VideoTarget,
)

PROC_STAT_MIN_FIELDS_AFTER_COMM: int = 13
PROC_STAT_PGRP_INDEX: int = 2
PROC_STAT_UTIME_INDEX: int = 11
PROC_STAT_STIME_INDEX: int = 12


def safe_worker_log_stem(filename: str, moviehash: str | None = None) -> str:
    """Returns a filesystem-safe, length-bounded log stem preventing ENAMETOOLONG (os error 36)."""
    if moviehash and MOVIEHASH_RE.fullmatch(moviehash):
        return moviehash
    raw_stem = Path(filename).stem
    safe_stem = raw_stem.replace("/", "_").replace("\x00", "") or "target"
    encoded = safe_stem.encode("utf-8", errors="surrogateescape")
    if len(encoded) <= MAX_LOG_STEM_BYTES:
        return safe_stem
    digest = hashlib.sha256(encoded).hexdigest()[:LOG_STEM_HASH_HEX_LEN]
    prefix = encoded[:MAX_LOG_STEM_BYTES].decode("utf-8", errors="ignore").rstrip(" ._-")
    return f"{prefix}_{digest}"


def write_batch_queue_state(
    total: int,
    completed: int,
    failed: int,
    active_workers: int,
    desired_workers: int | None = None,
    is_paused: bool | None = None,
) -> None:
    """Persists volatile batch queue telemetry atomically to `run/batch_queue.json`."""
    try:
        q_path = Path(facade_attr("DEFAULT_BATCH_QUEUE_PATH", DEFAULT_BATCH_QUEUE_PATH))
        q_path.parent.mkdir(parents=True, exist_ok=True)
        rem = max(0, total - (completed + failed))
        pct = round(((completed + failed) / total * 100.0), 1) if total > 0 else 0.0
        paused = DEFAULT_BATCH_PAUSED_PATH.is_file() if is_paused is None else is_paused
        state = {
            "total_targets": total,
            "completed_targets": completed,
            "failed_targets": failed,
            "remaining_targets": rem,
            "progress_pct": pct,
            "active_workers": active_workers,
            "desired_workers": desired_workers,
            "is_paused": paused,
            "is_active": rem > 0 and (active_workers > 0 or paused),
            "updated_at": facade_attr("time", time).strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        tmp_path = q_path.with_suffix(".tmp")
        tmp_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
        tmp_path.replace(q_path)
    except OSError as err:
        print(f"⚠️ Failed writing {DEFAULT_BATCH_QUEUE_PATH}: {err}", file=sys.stderr)


def get_process_group_cpu_ticks(pgid: int, proc_root: Path = Path("/proc")) -> int:
    """Sums user + system CPU ticks of every live process in process group `pgid`."""
    total_ticks = 0
    for stat_path in proc_root.glob("[0-9]*/stat"):
        try:
            content = stat_path.read_text(encoding="utf-8")
        except OSError:
            continue
        fields = content[content.rfind(")") + 2 :].split()
        try:
            if len(fields) >= PROC_STAT_MIN_FIELDS_AFTER_COMM and int(fields[PROC_STAT_PGRP_INDEX]) == pgid:
                total_ticks += int(fields[PROC_STAT_UTIME_INDEX]) + int(fields[PROC_STAT_STIME_INDEX])
        except ValueError:
            continue
    return total_ticks


def redact_nas(text: str, host: str, password: str) -> str:
    """Masks NAS credentials and hostnames before writing logs or truncated error summaries."""
    if password:
        variants = {password, urllib.parse.quote(password), urllib.parse.quote(password, safe="")}
        for secret in sorted(variants, key=len, reverse=True):
            text = text.replace(secret, "<redacted>")
    text = re.sub(r"[^\s\"'/]*@(?:\[[0-9A-Fa-f:.]+\]|[\w.-]+)", "<nas>", text)
    if host:
        text = re.sub(re.escape(host), "<nas>", text, flags=re.IGNORECASE)
    return text


def _parse_optional_float(pattern: str, text: str) -> float | None:
    match = re.search(pattern, text, re.IGNORECASE)
    return float(match.group(1)) if match else None


def _event_record(
    target: VideoTarget,
    start_time: float,
    status: str,
    log_path: Path,
    nas_host: str = "",
    nas_pass: str = "",
    moviehash: str | None = None,
    active_workers: int | None = None,
    desired_workers: int | None = None,
) -> Dict[str, object]:
    """Builds a structured, secret-redacted JSONL batch event record for `target`."""
    time_mod = facade_attr("time", time)
    end_time = time_mod.time()
    end_iso = time_mod.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(end_time))
    wall_duration = end_time - start_time
    success = status == "COMPLETED"
    log_text = log_path.read_text(encoding="utf-8", errors="ignore") if log_path.exists() else ""

    resumed = re.search(r"Extraction: resumed (\d+) frames from checkpoint", log_text)
    full_decode = success and "full decode" in log_text and not resumed
    actual_bytes = target.size_bytes if full_decode else 0
    rate_mib_s = (actual_bytes / BYTES_PER_MIB) / wall_duration if wall_duration > 0 else 0.0

    record: Dict[str, object] = {
        "video_stem": Path(target.filename).stem,
        "video_rel_path": target.rel_path,
        "moviehash": moviehash,
        "file_size_bytes": target.size_bytes,
        "wall_duration_secs": round(wall_duration, 2),
        "duration_seconds": _parse_optional_float(r"duration\s*:\s*([\d\.]+)", log_text),
        "fps": _parse_optional_float(r"fps\s*:\s*([\d\.]+)", log_text),
        "active_workers": active_workers,
        "desired_workers": desired_workers,
        "io_read_rate_mib_s": round(rate_mib_s, 2),
        "cifs_read_rate_mib_s": round(rate_mib_s, 2),
        "resumed_frames": int(resumed.group(1)) if resumed else 0,
        "start_iso": time_mod.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(start_time)),
        "end_iso": end_iso,
        "timestamp_iso": end_iso,
        "success": success,
        "status": status,
    }
    if not success and log_path.exists():
        record["error_summary"] = redact_nas(log_text, nas_host, nas_pass)[-ERROR_SUMMARY_TAIL_CHARS:]
    return record


def next_fast_failure_streak(streak: int, rec: Dict[str, object]) -> int:
    """Counts consecutive failures shorter than `FAST_FAILURE_SECS`; any other outcome resets to 0."""
    wall = rec.get("wall_duration_secs", 0.0)
    fast_failure = not rec.get("success") and isinstance(wall, int | float) and wall < FAST_FAILURE_SECS
    return streak + 1 if fast_failure else 0


def abort_batch(remaining_targets: List[VideoTarget], reason: str) -> None:
    """Clears queued targets and writes `run/batch_aborted` to prevent supervisor relaunch loops."""
    remaining_targets.clear()
    print(f"🛑 [ABORT] {reason}", flush=True)
    try:
        aborted_path = Path(facade_attr("DEFAULT_BATCH_ABORTED_PATH", DEFAULT_BATCH_ABORTED_PATH))
        aborted_path.parent.mkdir(parents=True, exist_ok=True)
        now_iso = facade_attr("time", time).strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        aborted_path.write_text(f"{now_iso} {reason}\n", encoding="utf-8")
    except OSError as err:
        print(f"⚠️ Failed writing {DEFAULT_BATCH_ABORTED_PATH}: {err}", file=sys.stderr)


__all__: Sequence[str] = [
    "_event_record",
    "abort_batch",
    "get_process_group_cpu_ticks",
    "next_fast_failure_streak",
    "redact_nas",
    "safe_worker_log_stem",
    "write_batch_queue_state",
]
