"""Domain models, repository path resolution, and named constants for the o2switch batch worker."""

from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Dict, NamedTuple, Tuple


class VideoTarget(NamedTuple):
    """One remote video target: raw FTP/HTTP relative path, NFC file name, and size in bytes."""

    rel_path: str
    filename: str
    size_bytes: int


def find_repo_root(anchor: Path | None = None) -> Path:
    """Finds git repository root with appliance fallback (~/dbsrt)."""
    start = (anchor or Path(__file__)).resolve()
    curr = start if start.is_dir() else start.parent
    for parent in [curr, *curr.parents]:
        if (parent / ".git").is_dir() or (parent / "Taskfile.yml").is_file():
            return parent
    home_dbsrt = Path.home() / "dbsrt"
    if home_dbsrt.is_dir():
        return home_dbsrt
    return Path.cwd()


def load_env_config(path: Path) -> Dict[str, str]:
    """Loads key-value pairs from an optional local unversioned environment file."""
    if not path.is_file():
        return {}
    config: Dict[str, str] = {}
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith("#") or "=" not in stripped:
                continue
            key, val = stripped.split("=", 1)
            config[key.strip()] = val.strip().strip("'\"")
    except OSError:
        return {}
    return config


REPO_ROOT: Path = find_repo_root()

_CONFIG_ENV: Dict[str, str] = (
    load_env_config(Path("etc/.nas_config")) or load_env_config(Path("etc/.env")) or load_env_config(Path(".env"))
)


def resolve_nas_password() -> str:
    """Resolves the NAS password from NAS_PASS env, etc/.nas_pass, or local env config."""
    if os.environ.get("NAS_PASS"):
        return os.environ["NAS_PASS"]
    pass_file = Path("etc/.nas_pass")
    if pass_file.is_file():
        return pass_file.read_text(encoding="utf-8").strip()
    return _CONFIG_ENV.get("NAS_PASS", "")


# Process & Concurrency Constants
DEFAULT_LOG_PATH: Path = Path("logs/batch_worker.log")
DEFAULT_DESIRED_WORKERS_PATH: Path = Path("run/desired_workers")
DEFAULT_BATCH_PAUSED_PATH: Path = Path("run/batch_paused")
MIN_CONCURRENT_WORKERS: int = 0
MAX_CONCURRENT_WORKERS: int = 8
TARGET_QUEUE_POLL_INTERVAL_SECS: float = 0.5
RELOAD_DRAIN_SECS: float = 30.0
CPU_BUDGET_VCPUS: int = int(os.getenv("DBSRT_CPU_BUDGET_VCPUS", "10"))

# Network & Discovery Constants
FTP_LIST_ATTEMPTS: int = 3
FTP_LIST_RETRY_DELAY_SECS: float = 30.0
FTP_CONNECT_TIMEOUT_SECS: float = 15.0
DEFAULT_FTP_INDEX_MAX_AGE_SECS: float = 3600.0
DEFAULT_WORKERS: int = 4
DEFAULT_NAS_HOST: str = os.getenv("NAS_HOST") or _CONFIG_ENV.get("NAS_HOST", "")
DEFAULT_NAS_USER: str = os.getenv("NAS_USER") or _CONFIG_ENV.get("NAS_USER", "")
DEFAULT_NAS_BASE: str = "/tvshow"
DEFAULT_FTP_PORT: int = 21
DEFAULT_FFMPEG_THREADS: str = "2"
# Exceeds FFmpeg -rw_timeout (30s) + -reconnect_delay_max (60s) so transient drops recover
# seamlessly while silent half-open TCP stalls are killed in 3 min instead of 7200s.
DEFAULT_STALL_TIMEOUT_SECS: int = 180
# Minimum cumulative CPU ticks (~0.5s CPU) required to count as non-I/O progress; filters out
# 4-5 ticks/min idle socket keepalive/poll noise that bypassed the old != tick check.
STALL_MIN_CPU_TICKS_DELTA: int = 50
DEFAULT_MAX_EXTRACTION_SECS: int = 1800

# Artifact & State Paths
DEFAULT_COUPLINGS_PATH: str = "data/SRT/couplings.json"
DEFAULT_PHASH_DIR: str = "data/__PHASH__"
DEFAULT_AHASH_DIR: str = "data/__AHASH__"
DEFAULT_SUBTITLES_DIR: str = "data/SRT"
DEFAULT_EVENTS_LOG_PATH: str = "data/reports/batch_tvshow_nightly.jsonl"
DEFAULT_TARGETS_PATH: str = "data/reports/selected_batch_targets.json"
DEFAULT_FTP_INDEX_REL_PATH: str = "data/reports/nas_ftp_index.json"
DEFAULT_BATCH_QUEUE_PATH: str = "run/batch_queue.json"
DEFAULT_BATCH_ABORTED_PATH: str = "run/batch_aborted"
DEFAULT_EXCLUDE_PATH: str = "etc/batch_exclude.txt"

# Failure & Watchdog Thresholds
FAST_FAILURE_SECS: float = 10.0
MAX_FAST_FAILURE_STREAK: int = 50
MAX_FAILED_ATTEMPTS: int = 3
WATCHDOG_POLL_INTERVAL_SECS: float = 1.0
ERROR_SUMMARY_TAIL_CHARS: int = 300
BYTES_PER_MIB: int = 1024 * 1024

# Filename, Stem & Hash Parsing Constants
VIDEO_EXTENSIONS: Tuple[str, ...] = (".mkv", ".mp4", ".webm", ".avi")
FTP_SKIPPED_NAMES: Tuple[str, ...] = (".", "..", "EMPTY_DIRS", "SRT", "@eaDir")
MAX_FPS_INTEGER_DIGITS: int = 3
MAX_FPS_FRACTION_DIGITS: int = 3
MIN_FPS_TOKEN_VALUE: float = 1.0
MAX_FPS_TOKEN_VALUE: float = 200.0
AMBIGUOUS_INTEGER_FPS_TOKENS: frozenset[str] = frozenset({"24", "25", "30", "48", "50", "60"})
PHASH_FPS_SUFFIX_RE = re.compile(r"\.\d+(\.\d+)?fps$")
MOVIEHASH_RE = re.compile(r"[0-9a-f]{16}")
MEDIA_NAME_RE = re.compile(rb'"media_name"\s*:\s*"((?:[^"\\]|\\.)*)"')
MAX_LOG_STEM_BYTES: int = 120
LOG_STEM_HASH_HEX_LEN: int = 12

# OpenSubtitles MovieHash & Range I/O Constants
MOVIEHASH_CHUNK_BYTES: int = 65536
MOVIEHASH_WORD_BYTES: int = 8
UINT64_MASK: int = 0xFFFFFFFFFFFFFFFF
HTTP_RANGE_TIMEOUT_SECS: float = 15.0
FTP_MOVIEHASH_TIMEOUT_SECS: float = 15.0
HTTP_PARTIAL_CONTENT_STATUS: int = 206
