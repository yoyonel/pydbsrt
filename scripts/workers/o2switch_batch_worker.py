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
import contextlib
import ftplib
import hashlib
import http.client
import json
import mmap
import os
import posixpath
import re
import signal
import subprocess
import sys
import threading
import time
import traceback
import unicodedata
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, Iterator, List, NamedTuple, Set, TextIO, Tuple


class VideoTarget(NamedTuple):
    """One NAS video: raw FTP path, NFC file name and size in bytes."""

    rel_path: str
    filename: str
    size_bytes: int


# Process groups of in-flight dbsrt children: they run in their own session, so they must be
# killed explicitly on shutdown or they keep running as orphans.
ACTIVE_CHILD_PGIDS: Set[int] = set()
ACTIVE_CHILD_LOCK = threading.Lock()


def find_repo_root() -> Path:
    """Finds git repository root with appliance fallback (~/dbsrt)."""
    p = Path(__file__).resolve().parent
    for parent in [p] + list(p.parents):
        if (parent / ".git").is_dir():
            return parent
    home_dbsrt = Path.home() / "dbsrt"
    if home_dbsrt.is_dir():
        return home_dbsrt
    return Path.cwd()


REPO_ROOT: Path = find_repo_root()
DEFAULT_LOG_PATH: Path = Path("logs/batch_worker.log")


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
        DEFAULT_LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with open(DEFAULT_LOG_PATH, "a", encoding="utf-8") as f:
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
        DEFAULT_LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with open(DEFAULT_LOG_PATH, "a", encoding="utf-8") as f:
            f.write(err_str)
    except Exception:
        pass


def _on_exit() -> None:
    _kill_active_children()
    msg = f"\nℹ️ [EXIT] Batch worker exiting at {time.strftime('%Y-%m-%d %H:%M:%S')}\n"
    print(msg, flush=True)


def _install_process_handlers() -> None:
    """Installs process-wide exit/signal handlers only when running as CLI worker."""
    atexit.register(_on_exit)
    sys.excepthook = _handle_uncaught_exception
    for _sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT, signal.SIGQUIT):
        try:
            signal.signal(_sig, _handle_signal)
        except Exception:
            pass


DEFAULT_DESIRED_WORKERS_PATH: Path = Path("run/desired_workers")
DEFAULT_BATCH_PAUSED_PATH: Path = Path("run/batch_paused")
MIN_CONCURRENT_WORKERS: int = 0
MAX_CONCURRENT_WORKERS: int = 8
TARGET_QUEUE_POLL_INTERVAL_SECS: float = 0.5
# A redeployed worker script is picked up between videos: the queue drains, then the process re-execs.
SCRIPT_PATH: Path = Path(__file__).resolve()
SCRIPT_MTIME: float = SCRIPT_PATH.stat().st_mtime
RELOAD_REQUESTED = threading.Event()
# Longest wait for in-flight videos after a redeploy; slower ones are killed and requeued by the re-exec,
# which resumes their extraction from the .phash.partial checkpoint (at most ~1024 frames redone)
RELOAD_DRAIN_SECS: float = 30.0
# Jobs run ~1 vCPU per FFmpeg thread and lowering desired_workers cannot stop in-flight ones,
# so admission keeps workers x threads under this share of the 12 vCPU o2switch LVE limit.
CPU_BUDGET_VCPUS: int = int(os.getenv("DBSRT_CPU_BUDGET_VCPUS", "10"))
FTP_LIST_ATTEMPTS: int = 3
FTP_LIST_RETRY_DELAY_SECS: float = 30.0


def script_updated() -> bool:
    """True when the worker script on disk differs from the one this process runs."""
    try:
        return SCRIPT_PATH.stat().st_mtime != SCRIPT_MTIME
    except OSError:
        return False


def max_workers_for_budget(ffmpeg_threads: str) -> int:
    """Highest concurrency whose decode threads fit in CPU_BUDGET_VCPUS ("0" lets FFmpeg take every core)."""
    threads = int(ffmpeg_threads) if ffmpeg_threads.isdigit() else 0
    per_job = threads if threads > 0 else CPU_BUDGET_VCPUS
    return max(1, min(MAX_CONCURRENT_WORKERS, CPU_BUDGET_VCPUS // per_job))


def read_desired_workers(default_workers: int, max_workers: int = MAX_CONCURRENT_WORKERS) -> int:
    """Reads desired concurrency dynamically from run/desired_workers or run/batch_paused, capped at max_workers."""
    if DEFAULT_BATCH_PAUSED_PATH.is_file():
        return 0
    val = default_workers
    if DEFAULT_DESIRED_WORKERS_PATH.is_file():
        # A half-written or garbage file keeps the default until the next poll
        with contextlib.suppress(OSError, ValueError):
            val = int(DEFAULT_DESIRED_WORKERS_PATH.read_text(encoding="utf-8").strip())
    return max(MIN_CONCURRENT_WORKERS, min(val, max_workers))


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
DEFAULT_AHASH_DIR: str = "data/__AHASH__"
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
# MLSD entries that never hold library videos (Synology metadata, subtitle and placeholder folders)
FTP_SKIPPED_NAMES: Tuple[str, ...] = (".", "..", "EMPTY_DIRS", "SRT", "@eaDir")
MAX_FPS_INTEGER_DIGITS: int = 3
MAX_FPS_FRACTION_DIGITS: int = 3
MIN_FPS_TOKEN_VALUE: float = 1.0
MAX_FPS_TOKEN_VALUE: float = 200.0
AMBIGUOUS_INTEGER_FPS_TOKENS: frozenset[str] = frozenset({"24", "25", "30", "48", "50", "60"})
PHASH_FPS_SUFFIX_RE = re.compile(r"\.\d+(\.\d+)?fps$")
MOVIEHASH_RE = re.compile(r"[0-9a-f]{16}")
MAX_LOG_STEM_BYTES: int = 120
LOG_STEM_HASH_HEX_LEN: int = 12


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


def resolve_nas_password() -> str:
    """Resolves the NAS password from NAS_PASS env, etc/.nas_pass or the env config file."""
    if os.environ.get("NAS_PASS"):
        return os.environ["NAS_PASS"]
    pass_file = Path("etc/.nas_pass")
    if pass_file.is_file():
        return pass_file.read_text(encoding="utf-8").strip()
    return _CONFIG_ENV.get("NAS_PASS", "")


MEDIA_NAME_RE = re.compile(rb'"media_name"\s*:\s*"((?:[^"\\]|\\.)*)"')


def load_certified_stems(couplings_file: Path) -> Set[str]:
    """Loads all media stems already certified in couplings.json.

    The registry embeds every subtitle's sub-frame anchors (hundreds of MB) and json.loads on it
    peaked at 1.7 GB, so only the media_name values are scanned from a read-only mmap.
    """
    stems: Set[str] = set()
    if couplings_file.is_file() and couplings_file.stat().st_size > 0:
        try:
            with open(couplings_file, "rb") as f, mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ) as data:
                for match in MEDIA_NAME_RE.finditer(data):
                    media_name = json.loads(b'"' + match.group(1) + b'"')
                    if media_name:
                        stems.add(media_name)
        except Exception as err:
            print(f"⚠️ Warning loading {couplings_file}: {err}", file=sys.stderr)
    return stems


def _is_valid_fps_int_part(token: str) -> bool:
    return (
        1 <= len(token) <= MAX_FPS_INTEGER_DIGITS and token.isdigit() and not (len(token) > 1 and token.startswith("0"))
    )


def strip_fps_token(without_ext: str) -> str:
    """Strips a trailing `.<fps>fps` suffix without amputating release years or episode numbers."""
    if not without_ext.endswith("fps"):
        return without_ext
    before_fps = without_ext[:-3]
    last_dot = before_fps.rfind(".")
    if last_dot < 0:
        return without_ext
    tail = before_fps[last_dot + 1 :]
    prev_dot = before_fps.rfind(".", 0, last_dot)
    if prev_dot >= 0:
        int_part = before_fps[prev_dot + 1 : last_dot]
        if (
            _is_valid_fps_int_part(int_part)
            and 1 <= len(tail) <= MAX_FPS_FRACTION_DIGITS
            and tail.isdigit()
            and tail not in AMBIGUOUS_INTEGER_FPS_TOKENS
        ):
            val = float(f"{int_part}.{tail}")
            if MIN_FPS_TOKEN_VALUE <= val <= MAX_FPS_TOKEN_VALUE:
                return before_fps[:prev_dot]
    if _is_valid_fps_int_part(tail):
        val = float(tail)
        if MIN_FPS_TOKEN_VALUE <= val <= MAX_FPS_TOKEN_VALUE:
            return before_fps[:last_dot]
    return without_ext


def ahash_logical_stem(ahash_name: str) -> str:
    """Logical stem matching Rust `ahash_logical_stem`: strips `.ahash` and trailing `.<fps>fps`."""
    return strip_fps_token(ahash_name.removesuffix(".ahash"))


def phash_media_stem(phash_name: str) -> str:
    """Media stem of a .phash or .ahash file name: URL-decoded, without the extension, fps and video extension suffixes."""
    unquoted = urllib.parse.unquote(phash_name).removesuffix(".phash").removesuffix(".ahash")
    raw_name = strip_fps_token(unquoted)
    for ext in VIDEO_EXTENSIONS:
        if raw_name.lower().endswith(ext):
            return raw_name[: -len(ext)]
    return raw_name


def _file_size(path: Path) -> int:
    try:
        return path.stat().st_size
    except OSError:
        return 0


def _valid_ahash_sidecar_media_name(ahash: Path) -> str | None:
    """Reads `<stem>.json` beside `<stem>.<fps>fps.ahash` and returns `media_name` only for valid typed AhashSidecar files."""
    sidecar = ahash.with_name(f"{ahash_logical_stem(ahash.name)}.json")
    if _file_size(sidecar) <= 0:
        return None
    try:
        payload = json.loads(sidecar.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(payload, dict):
        return None
    media_name = payload.get("media_name")
    selection = payload.get("selection")
    header = payload.get("header")
    if (
        not isinstance(media_name, str)
        or not media_name.strip()
        or not isinstance(selection, str)
        or not isinstance(header, dict)
    ):
        return None
    return media_name


def _sidecar_media_name(phash: Path, fallback_dir: Path | str | None = None) -> str:
    """media_name of the <moviehash>.json sidecar next to a <moviehash>.<fps>fps.phash, or ""."""
    head = phash.name.split(".", 1)[0]
    if not MOVIEHASH_RE.fullmatch(head):
        return ""
    candidates = [phash.with_name(f"{head}.json")]
    if fallback_dir:
        candidates.append(Path(fallback_dir) / f"{head}.json")
    for sidecar in candidates:
        try:
            if sidecar.is_file():
                media_name = json.loads(sidecar.read_text(encoding="utf-8")).get("media_name")
                if isinstance(media_name, str):
                    return media_name
        except (OSError, ValueError, AttributeError):
            pass
    return ""


def load_existing_phash_stems(phash_dir: Path) -> Set[str]:
    """Loads all media stems with existing non-empty .phash files on disk."""
    stems: Set[str] = set()
    if phash_dir.is_dir():
        for p in phash_dir.glob("*.phash"):
            if _file_size(p) > 0:
                stems.update((phash_media_stem(p.name), p.stem))
                media_name = _sidecar_media_name(p)
                if media_name:
                    stems.add(phash_media_stem(media_name))
    return stems


def load_existing_ahash_stems(ahash_dir: Path) -> Set[str]:
    """Loads media stems having BOTH a non-empty .ahash file and a valid typed `<stem>.json` sidecar."""
    stems: Set[str] = set()
    if ahash_dir.is_dir():
        for p in ahash_dir.glob("*.ahash"):
            if _file_size(p) <= 0:
                continue
            media_name = _valid_ahash_sidecar_media_name(p)
            if not media_name:
                continue
            stems.update((phash_media_stem(p.name), p.stem, phash_media_stem(media_name)))
    return stems


def load_nas_ftp_index_targets(index_path: Path, base_dir: str) -> List[VideoTarget]:
    """Loads video targets directly from cached nas_ftp_index.json if available."""
    if not index_path.is_file():
        return []
    try:
        with open(index_path, encoding="utf-8") as f:
            raw = json.load(f)
        entries = raw.get("index", raw)
        clean_base = base_dir.strip("/")
        targets = []
        for filename, rel in entries.items():
            clean_rel = rel.lstrip("/")
            if clean_base and clean_base != "all":
                if not (clean_rel.startswith(clean_base + "/") or clean_rel == clean_base):
                    continue
            targets.append(
                VideoTarget(
                    rel_path="/" + clean_rel,
                    filename=unicodedata.normalize("NFC", filename),
                    size_bytes=0,
                )
            )
        return targets
    except Exception as err:
        print(f"⚠️ Failed reading {index_path}: {err}", file=sys.stderr)
        return []


def list_nas_videos_ftp(
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    base_dir: str = DEFAULT_NAS_BASE,
    ftp_port: int = DEFAULT_FTP_PORT,
    max_depth: int = 8,
) -> List[VideoTarget]:
    """Discovers video files on Synology NAS using cached index or FTP MLSD protocol.

    Transient FTP errors restart the whole walk, so a dropped connection never yields a partial list.
    """
    index_path = REPO_ROOT / "data/reports/nas_ftp_index.json"
    cached = load_nas_ftp_index_targets(index_path, base_dir)
    if cached:
        print(f"✓ Loaded {len(cached)} targets from cached {index_path.name} (base: {base_dir}).")
        return cached

    for attempt in range(1, FTP_LIST_ATTEMPTS + 1):
        try:
            return _walk_nas_videos_ftp(nas_host, nas_user, nas_pass, base_dir, ftp_port, max_depth)
        except (OSError, EOFError, ftplib.error_temp, ftplib.error_reply, ftplib.error_proto) as err:
            if attempt == FTP_LIST_ATTEMPTS:
                raise
            print(f"⚠️ NAS FTP listing failed ({err}), retry {attempt}/{FTP_LIST_ATTEMPTS - 1}", file=sys.stderr)
            time.sleep(FTP_LIST_RETRY_DELAY_SECS * attempt)
    return []


def _list_ftp_dir(ftp: ftplib.FTP, path: str) -> List[Tuple[str, Dict[str, str]]]:
    """MLSD entries of `path` without hidden and NAS system entries."""
    try:
        entries = list(ftp.mlsd(path))
    except ftplib.error_perm:
        try:
            ftp.cwd(path if path.startswith("/") else "/" + path)
            entries = list(ftp.mlsd())
        except Exception:
            return []
    return [(name, facts) for name, facts in entries if name not in FTP_SKIPPED_NAMES and not name.startswith(".")]


def _walk_nas_videos_ftp(
    nas_host: str, nas_user: str, nas_pass: str, base_dir: str, ftp_port: int, max_depth: int
) -> List[VideoTarget]:
    ftp = ftplib.FTP()
    ftp.connect(nas_host, ftp_port, timeout=15)
    ftp.login(nas_user, nas_pass)
    ftp.encoding = "utf-8"

    results: List[VideoTarget] = []
    clean_base = base_dir.lstrip("/")
    stack: List[str] = ["tvshow", "video"] if clean_base in ("", "all") else [clean_base]
    while stack:
        curr = stack.pop()
        for name, facts in _list_ftp_dir(ftp, curr):
            full_path = posixpath.join(curr, name)
            ftype = facts.get("type", "")
            if ftype == "dir" and full_path.count("/") <= max_depth:
                stack.append(full_path)
            elif ftype == "file" and name.lower().endswith(VIDEO_EXTENSIONS):
                rel = full_path if full_path.startswith("/") else "/" + full_path
                # The NAS mixes NFC and NFD names: the path keeps the raw name the FTP server
                # expects, the media name is NFC like the rest of the catalog
                results.append(VideoTarget(rel, unicodedata.normalize("NFC", name), int(facts.get("size", 0))))

    # The listing is complete: a connection already dropped by the server must not discard it
    with contextlib.suppress(Exception):
        ftp.quit()
    return results


# OpenSubtitles moviehash: file size plus its 64 KiB head and tail summed as little-endian u64 words
MOVIEHASH_CHUNK_BYTES: int = 65536
MOVIEHASH_WORD_BYTES: int = 8
UINT64_MASK: int = 0xFFFFFFFFFFFFFFFF
HTTP_RANGE_TIMEOUT_SECS: float = 15.0
FTP_MOVIEHASH_TIMEOUT_SECS: float = 15.0
HTTP_PARTIAL_CONTENT_STATUS: int = 206


def moviehash_from_chunks(size: int, head: bytes, tail: bytes) -> str:
    """Same value as compute_opensubtitles_hash (dbsrt-core) for the same file."""
    total = size
    for block in (head, tail):
        for i in range(0, len(block), MOVIEHASH_WORD_BYTES):
            total += int.from_bytes(block[i : i + MOVIEHASH_WORD_BYTES], "little")
    return f"{total & UINT64_MASK:016x}"


def compute_local_moviehash(path: Path) -> str | None:
    """Computes OpenSubtitles 64-bit MovieHash from a local file without loading it in full."""
    try:
        size = path.stat().st_size
        if size < MOVIEHASH_CHUNK_BYTES:
            return None
        with path.open("rb") as fh:
            head = fh.read(MOVIEHASH_CHUNK_BYTES)
            fh.seek(size - MOVIEHASH_CHUNK_BYTES)
            tail = fh.read(MOVIEHASH_CHUNK_BYTES)
    except OSError:
        return None
    if len(head) < MOVIEHASH_CHUNK_BYTES or len(tail) < MOVIEHASH_CHUNK_BYTES:
        return None
    return moviehash_from_chunks(size, head, tail)


def _ftp_path_candidates(path: str) -> Tuple[str, ...]:
    """Returns deduplicated raw/NFD/NFC FTP path candidates for Synology mixed encodings."""
    candidates: List[str] = []
    for variant in (path, unicodedata.normalize("NFD", path), unicodedata.normalize("NFC", path)):
        if variant not in candidates:
            candidates.append(variant)
    return tuple(candidates)


def _ftp_resolve_size_and_path(connect: Callable[[], ftplib.FTP], path: str) -> Tuple[str, int] | None:
    """Queries remote file size in binary mode over FTP, handling NFC/NFD path normalization."""
    ftp = connect()
    try:
        ftp.voidcmd("TYPE I")
        for candidate in _ftp_path_candidates(path):
            try:
                size = ftp.size(candidate)
            except ftplib.error_perm:
                continue
            if size is not None:
                return candidate, int(size)
        return None
    except ftplib.all_errors:
        return None
    finally:
        with contextlib.suppress(Exception):
            ftp.close()


def _ftp_read_at(connect: Callable[[], ftplib.FTP], path: str, offset: int) -> bytes:
    """Reads MOVIEHASH_CHUNK_BYTES at `offset` in binary mode in its own FTP session, then drops the rest."""
    ftp = connect()
    try:
        ftp.voidcmd("TYPE I")
        conn = ftp.transfercmd(f"RETR {path}", rest=offset)
        data = bytearray()
        try:
            while len(data) < MOVIEHASH_CHUNK_BYTES:
                chunk = conn.recv(MOVIEHASH_CHUNK_BYTES - len(data))
                if not chunk:
                    break
                data += chunk
        finally:
            conn.close()
        # An early close gets 426 (226 when the file ended); the Synology NAS drops the control connection
        # instead (EOFError), which is why each read opens its own session
        with contextlib.suppress(ftplib.error_temp, ftplib.error_reply, EOFError):
            ftp.voidresp()
        return bytes(data)
    finally:
        with contextlib.suppress(Exception):
            ftp.close()


def _http_file_size(url: str) -> int | None:
    """Resolves remote HTTP file size via HEAD Content-Length or 1-byte Range Content-Range."""
    try:
        head_req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(head_req, timeout=HTTP_RANGE_TIMEOUT_SECS) as resp:
            content_len = resp.headers.get("Content-Length", "").strip()
            if content_len.isdigit():
                return int(content_len)
    except (OSError, http.client.HTTPException):
        pass
    try:
        probe_req = urllib.request.Request(url, headers={"Range": "bytes=0-0"})
        with urllib.request.urlopen(probe_req, timeout=HTTP_RANGE_TIMEOUT_SECS) as resp:
            if resp.status != HTTP_PARTIAL_CONTENT_STATUS:
                return None
            content_range = resp.headers.get("Content-Range", "")
            total = content_range.rsplit("/", 1)[-1].strip()
            return int(total) if total.isdigit() else None
    except (OSError, http.client.HTTPException):
        return None


def _http_content_range_matches(content_range: str, offset: int, expected_total: int | None) -> bool:
    """Validates `bytes <offset>-<end>/<total>` against requested offset and expected total file size."""
    range_spec, sep, total_str = content_range.rpartition("/")
    if sep != "/" or not range_spec.startswith(f"bytes {offset}-"):
        return False
    clean_total = total_str.strip()
    if not clean_total.isdigit():
        return False
    return expected_total is None or int(clean_total) == expected_total


def _http_read_at(url: str, offset: int, expected_total: int | None = None) -> bytes:
    """Reads MOVIEHASH_CHUNK_BYTES at `offset` with a Range request; empty unless offset and `/total` match."""
    request = urllib.request.Request(url, headers={"Range": f"bytes={offset}-{offset + MOVIEHASH_CHUNK_BYTES - 1}"})
    with urllib.request.urlopen(request, timeout=HTTP_RANGE_TIMEOUT_SECS) as resp:
        # Reject full-body 200 responses and stale inventory sizes disagreeing with `/total`
        if resp.status != HTTP_PARTIAL_CONTENT_STATUS or not _http_content_range_matches(
            resp.headers.get("Content-Range", ""), offset, expected_total
        ):
            return b""
        return resp.read(MOVIEHASH_CHUNK_BYTES)


def ranged_moviehash(read_at: Callable[[int], bytes], size: int) -> str | None:
    """Moviehash of a remote file from two ranged reads; None when the exact bytes cannot be read."""
    if size < MOVIEHASH_CHUNK_BYTES:
        return None
    head = read_at(0)
    tail = read_at(size - MOVIEHASH_CHUNK_BYTES)
    if len(head) < MOVIEHASH_CHUNK_BYTES or len(tail) < MOVIEHASH_CHUNK_BYTES:
        return None
    return moviehash_from_chunks(size, head, tail)


def ftp_moviehash(connect: Callable[[], ftplib.FTP], path: str, size: int = 0) -> str | None:
    """Moviehash of a NAS file read over FTP; always resolves authoritative binary SIZE and NFD/NFC path."""
    _ = size
    resolved = _ftp_resolve_size_and_path(connect, path)
    if resolved is None:
        return None
    resolved_path, actual_size = resolved
    return ranged_moviehash(lambda offset: _ftp_read_at(connect, resolved_path, offset), actual_size)


def target_moviehash(target: VideoTarget, stream_base: str, child_env: Dict[str, str]) -> str | None:
    """Moviehash of a target on the FTP NAS or behind the HTTP gateway; None on any read error (the stem names the files)."""
    url = urllib.parse.urlsplit(stream_base)
    if url.scheme in ("http", "https"):
        media_url = build_stream_url(stream_base, target.rel_path)
        try:
            size = target.size_bytes if target.size_bytes > 0 else (_http_file_size(media_url) or 0)
            return ranged_moviehash(lambda offset: _http_read_at(media_url, offset, expected_total=size), size)
        except (OSError, http.client.HTTPException):
            return None
    if url.scheme != "ftp" or not url.hostname:
        return None
    host = url.hostname

    def connect() -> ftplib.FTP:
        ftp = ftplib.FTP(encoding="utf-8")
        try:
            ftp.connect(host, url.port or DEFAULT_FTP_PORT, timeout=FTP_MOVIEHASH_TIMEOUT_SECS)
            ftp.login(child_env.get("DBSRT_FTP_USER", ""), child_env.get("DBSRT_FTP_PASSWORD", ""))
        except BaseException:
            ftp.close()
            raise
        return ftp

    try:
        return ftp_moviehash(connect, target.rel_path, target.size_bytes)
    except ftplib.all_errors:
        return None


def load_targets_file(targets_path: Path, nas_base: str, skip_stems: Set[str]) -> List[VideoTarget]:
    """Loads explicit targets (list of paths or {path, filename, stem, size_bytes} dicts).

    Paths are rebased on `nas_base`; stems in `skip_stems` are dropped. Raises on unreadable JSON.
    """
    targets: List[VideoTarget] = []
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
        if nas_base and nas_base != "/":
            idx = p_str.find(nas_base)
            rel_path = p_str[idx:] if idx != -1 else p_str
        else:
            rel_path = p_str
        targets.append(VideoTarget(rel_path, filename, size_bytes))
    return targets


def iter_events(events_log: Path) -> Iterator[Tuple[str, Dict[str, Any]]]:
    """Yields (NFC video stem, record) for each parseable event of the log, oldest first."""
    if not events_log.is_file():
        return
    for line in events_log.read_text(encoding="utf-8").splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        stem = unicodedata.normalize("NFC", rec.get("video_stem", ""))
        if stem:
            yield stem, rec


def resolve_retry_targets(events_log: Path, skip_stems: Set[str]) -> List[VideoTarget]:
    """Returns videos whose last outcome in the events log is a failure, minus `skip_stems`."""
    last_outcome: Dict[str, Tuple[bool, VideoTarget]] = {}
    for stem, rec in iter_events(events_log):
        rel_path = rec.get("video_rel_path", "")
        if rel_path:
            item = VideoTarget(rel_path, Path(rel_path).name, rec.get("file_size_bytes", 0))
            last_outcome[stem] = (bool(rec.get("success", False)), item)
    return [item for stem, (ok, item) in last_outcome.items() if not ok and stem not in skip_stems]


def load_blacklisted_stems(events_log: Path, max_attempts: int = MAX_FAILED_ATTEMPTS) -> Set[str]:
    """Returns NFC stems whose last `max_attempts` outcomes in the events log are all failures."""
    streaks: Dict[str, int] = {}
    for stem, rec in iter_events(events_log):
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


def match_nas_paths(targets: List[VideoTarget], nas_videos: List[VideoTarget]) -> List[VideoTarget]:
    """Replaces each target with the NAS entry of the same NFC stem; unknown targets are dropped.

    Targets selected on a CIFS mount or replayed from the events log can carry NFC-normalized or
    8.3-mangled names that the FTP server rejects: only the FTP listing holds the real paths.
    """
    by_stem = {Path(video.filename).stem: video for video in nas_videos}
    matched = (by_stem.get(unicodedata.normalize("NFC", Path(target.filename).stem)) for target in targets)
    return list(dict.fromkeys(video for video in matched if video))


def write_batch_queue_state(
    total: int,
    completed: int,
    failed: int,
    active_workers: int,
    desired_workers: int | None = None,
    is_paused: bool | None = None,
) -> None:
    """Persists volatile batch queue telemetry to disk for dbsrt top and monitoring."""
    try:
        q_path = Path(DEFAULT_BATCH_QUEUE_PATH)
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
            "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        tmp_path = q_path.with_suffix(".tmp")
        tmp_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
        tmp_path.replace(q_path)
    except OSError as err:
        # Telemetry only: a full disk must not stop the batch
        print(f"⚠️ Failed writing {DEFAULT_BATCH_QUEUE_PATH}: {err}", file=sys.stderr)


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
    """Joins the streaming base and a raw target path.

    FTP expects verbatim paths; HTTP requires standard URL path encoding (spaces, brackets).
    """
    clean_path = rel_path if rel_path.startswith("/") else "/" + rel_path
    if stream_base.lower().startswith(("http://", "https://")):
        clean_path = urllib.parse.quote(clean_path, safe="/")
        return f"{stream_base}{clean_path}"
    if stream_base.lower().startswith("file://"):
        return f"{stream_base.removeprefix('file://')}{clean_path}"
    if not stream_base:
        return clean_path
    return f"{stream_base}{clean_path}"


def _kill_child(proc: subprocess.Popen, out_f: TextIO, reason: str) -> None:
    out_f.write(f"\n❌ [WATCHDOG] Killed process: {reason}.\n")
    out_f.flush()
    with contextlib.suppress(ProcessLookupError):
        os.killpg(proc.pid, signal.SIGKILL)
    proc.wait()


def _watch_child(proc: subprocess.Popen, out_f: TextIO, start_time: float, max_extraction_secs: int) -> str:
    """Polls a dbsrt child until it exits or a watchdog kills it, and returns the final status."""
    last_cpu_ticks = 0
    stall_duration = 0
    while (returncode := proc.poll()) is None:
        time.sleep(1.0)
        cur_cpu_ticks = get_process_group_cpu_ticks(proc.pid)
        stall_duration = 0 if cur_cpu_ticks != last_cpu_ticks else stall_duration + 1
        last_cpu_ticks = cur_cpu_ticks
        # Stall detector: zero CPU activity in the whole group for >= DEFAULT_STALL_TIMEOUT_SECS
        if stall_duration >= DEFAULT_STALL_TIMEOUT_SECS:
            _kill_child(proc, out_f, f"zero progress / activity for {stall_duration}s (Stall Detector)")
            return "STALLED"
        if time.time() - start_time >= max_extraction_secs:
            _kill_child(proc, out_f, f"exceeded max duration of {max_extraction_secs}s")
            return "TIMEOUT"
    if returncode == -signal.SIGTERM and RELOAD_REQUESTED.is_set():
        return "RELOAD"
    return "COMPLETED" if returncode == 0 else "ERROR"


def redact_nas(text: str, host: str, password: str) -> str:
    """Masks NAS credentials and host in text bound for batch_worker.log, the events file and monitoring.

    Fails safe: any token glued before '@<any host>' goes with that host, so userinfo fragments cut by
    truncation go too, even for a NAS host that is no longer the configured one.
    """
    if password:
        variants = {password, urllib.parse.quote(password), urllib.parse.quote(password, safe="")}
        for secret in sorted(variants, key=len, reverse=True):
            text = text.replace(secret, "<redacted>")
    text = re.sub(r"[^\s\"'/]*@(?:\[[0-9A-Fa-f:.]+\]|[\w.-]+)", "<nas>", text)
    if host:
        text = re.sub(re.escape(host), "<nas>", text, flags=re.IGNORECASE)
    return text


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
    end_time = time.time()
    end_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(end_time))
    wall_duration = end_time - start_time
    success = status == "COMPLETED"
    log_text = log_path.read_text(encoding="utf-8", errors="ignore") if log_path.exists() else ""

    # size / duration is a NAS read rate only when the CLI states it decoded the whole file: a run
    # resumed from a checkpoint or served by an existing .phash reads a part of it, or nothing.
    resumed = re.search(r"Extraction: resumed (\d+) frames from checkpoint", log_text)
    full_decode = success and "full decode" in log_text and not resumed
    actual_bytes = target.size_bytes if full_decode else 0
    rate_mib_s = (actual_bytes / (1024 * 1024)) / wall_duration if wall_duration > 0 else 0.0

    duration_m = re.search(r"duration\s*:\s*([\d\.]+)", log_text, re.IGNORECASE)
    duration_s = float(duration_m.group(1)) if duration_m else None
    fps_m = re.search(r"fps\s*:\s*([\d\.]+)", log_text, re.IGNORECASE)
    fps_val = float(fps_m.group(1)) if fps_m else None

    record: Dict[str, object] = {
        "video_stem": Path(target.filename).stem,
        "video_rel_path": target.rel_path,
        "moviehash": moviehash,
        "file_size_bytes": target.size_bytes,
        "wall_duration_secs": round(wall_duration, 2),
        "duration_seconds": duration_s,
        "fps": fps_val,
        "active_workers": active_workers,
        "desired_workers": desired_workers,
        "io_read_rate_mib_s": round(rate_mib_s, 2),
        "cifs_read_rate_mib_s": round(rate_mib_s, 2),
        "resumed_frames": int(resumed.group(1)) if resumed else 0,
        "start_iso": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(start_time)),
        "end_iso": end_iso,
        "timestamp_iso": end_iso,
        "success": success,
        "status": status,
    }
    if not success and log_path.exists():
        # Redact before truncating: a cut inside a raw password would leave an unmatched fragment
        record["error_summary"] = redact_nas(log_text, nas_host, nas_pass)[-300:]
    return record


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
) -> Dict[str, object]:
    """Processes a single video by invoking dbsrt media:add directly over FTP."""
    base_cmd = [
        dbsrt_bin,
        "media:add",
        build_stream_url(stream_base, target.rel_path),
        f"--media-name={target.filename}",
        "--hwaccel",
        "disabled",
        "--phash-dir",
        DEFAULT_PHASH_DIR,
        "--ahash-dir",
        ahash_dir,
        "--subtitles-dir",
        DEFAULT_SUBTITLES_DIR,
        "--skip-certify",
    ]
    moviehash = target_moviehash(target, stream_base, child_env)
    if moviehash:
        base_cmd += ["--moviehash", moviehash]

    start_time = time.time()
    print(f"--> [DEBUG] Launching dbsrt for {target.filename}...", flush=True)

    # The moviehash or truncated/hashed safe_stem names the log: a 250+ byte media name cannot (os error 36)
    safe_stem = safe_worker_log_stem(target.filename, moviehash)
    log_path = Path("logs") / "workers" / f"{safe_stem}.log"
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


def abort_batch(remaining_targets: List[VideoTarget], reason: str) -> None:
    """Drops the queued targets and leaves run/batch_aborted so the supervisor does not relaunch."""
    remaining_targets.clear()
    print(f"🛑 [ABORT] {reason}", flush=True)
    try:
        aborted_path = Path(DEFAULT_BATCH_ABORTED_PATH)
        aborted_path.parent.mkdir(parents=True, exist_ok=True)
        aborted_path.write_text(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {reason}\n", encoding="utf-8")
    except OSError as err:
        print(f"⚠️ Failed writing {DEFAULT_BATCH_ABORTED_PATH}: {err}", file=sys.stderr)


class _BatchRun:
    """Queue, counters and worker threads of one batch; mutated only under `state_lock`."""

    def __init__(
        self,
        targets: List[VideoTarget],
        run_video: Callable[[VideoTarget], Dict[str, object]],
        initial_workers: int,
        max_workers: int,
    ) -> None:
        self.total = len(targets)
        self.remaining = list(targets)
        self.run_video = run_video
        self.initial_workers = initial_workers
        self.max_workers = max_workers
        self.state_lock = threading.Lock()
        self.threads: List[threading.Thread] = []
        self.success_count = 0
        self.fail_count = 0
        self.fast_failure_streak = 0
        self.reload_since = 0.0

    def desired_workers(self) -> int:
        return read_desired_workers(self.initial_workers, self.max_workers)

    def publish(self, active_workers: int | None = None) -> None:
        active = len(self.threads) if active_workers is None else active_workers
        desired = self.desired_workers()
        is_paused = DEFAULT_BATCH_PAUSED_PATH.is_file() or desired == 0
        try:
            write_batch_queue_state(
                self.total, self.success_count, self.fail_count, active, desired_workers=desired, is_paused=is_paused
            )
        except TypeError:
            # Fallback for mock lambdas expecting 4 positional arguments
            write_batch_queue_state(self.total, self.success_count, self.fail_count, active)

    def worker_loop(self) -> None:
        while True:
            with self.state_lock:
                if len(self.threads) > self.desired_workers() or self.desired_workers() == 0 or not self.remaining:
                    return
                item = self.remaining.pop(0)
            try:
                rec = self.run_video(item)
            except Exception as exc:
                with self.state_lock:
                    self.fail_count += 1
                    print(f"  ❌ Exception processing {item.filename}: {exc}", flush=True)
                    self.publish()
                continue
            if rec.get("status") == "RELOAD":
                return
            with self.state_lock:
                self.record(rec)
                self.publish()

    def record(self, rec: Dict[str, object]) -> None:
        if rec.get("success"):
            self.success_count += 1
            print(
                f"  ✓ [{self.success_count}/{self.total}] {rec['video_stem']} ({rec['wall_duration_secs']}s, {rec['io_read_rate_mib_s']} MB/s)",
                flush=True,
            )
        else:
            self.fail_count += 1
            print(
                f"  ❌ [{self.fail_count} FAILED] {rec['video_stem']}: {rec.get('error_summary', 'Unknown error')}",
                flush=True,
            )
        self.fast_failure_streak = next_fast_failure_streak(self.fast_failure_streak, rec)
        if self.fast_failure_streak == MAX_FAST_FAILURE_STREAK:
            abort_batch(
                self.remaining,
                f"{self.fast_failure_streak} consecutive failures under {FAST_FAILURE_SECS:.0f}s: "
                "systemic error, check logs/workers/ before relaunching",
            )

    def check_reload(self) -> None:
        if self.remaining and script_updated():
            print("🔄 [RELOAD] Worker script updated: finishing in-flight videos before re-exec", flush=True)
            self.remaining.clear()
            RELOAD_REQUESTED.set()
            self.reload_since = time.time()
        if self.reload_since and time.time() - self.reload_since >= RELOAD_DRAIN_SECS:
            print("🔄 [RELOAD] Drain timeout: stopping in-flight videos, the re-exec requeues them", flush=True)
            _kill_active_children()
            self.reload_since = 0.0

    def schedule(self) -> bool:
        """One scheduler tick: reload checks, then spawns workers up to the target. False once drained."""
        with self.state_lock:
            self.check_reload()
            alive = [t for t in self.threads if t.is_alive()]
            exited = len(alive) != len(self.threads)
            self.threads = alive
            if exited:
                # A worker leaving an empty queue publishes nothing itself: refresh the count here.
                self.publish()
            if not self.remaining and not self.threads:
                return False
            desired = self.desired_workers()
            while len(self.threads) < desired and self.remaining:
                t = threading.Thread(target=self.worker_loop, daemon=True)
                self.threads.append(t)
                t.start()
                self.publish()
        return True


def execute_dynamic_batch_ingestion(
    targets: List[VideoTarget],
    stream_base: str,
    dbsrt_bin: str,
    child_env: Dict[str, str],
    events_log: Path,
    max_extraction_secs: int,
    initial_workers: int,
    ahash_dir: str = DEFAULT_AHASH_DIR,
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
    DEFAULT_DESIRED_WORKERS_PATH.parent.mkdir(parents=True, exist_ok=True)
    if not DEFAULT_DESIRED_WORKERS_PATH.exists():
        try:
            DEFAULT_DESIRED_WORKERS_PATH.write_text(str(initial_workers), encoding="utf-8")
        except OSError as err:
            print(f"⚠️ Failed writing {DEFAULT_DESIRED_WORKERS_PATH}: {err}", file=sys.stderr)

    max_workers = max_workers_for_budget(child_env.get("DBSRT_FFMPEG_THREADS", DEFAULT_FFMPEG_THREADS))
    print(f"CPU budget: {CPU_BUDGET_VCPUS} vCPU, at most {max_workers} concurrent videos", flush=True)

    def run_video(target: VideoTarget) -> Dict[str, object]:
        return process_single_video(
            target,
            stream_base,
            dbsrt_bin,
            child_env,
            log_lock,
            events_log,
            max_extraction_secs,
            None,
            None,
            ahash_dir,
        )

    batch = _BatchRun(targets, run_video, initial_workers, max_workers)
    batch.publish(initial_workers)
    start_batch = time.time()
    try:
        while batch.schedule():
            time.sleep(TARGET_QUEUE_POLL_INTERVAL_SECS)
    finally:
        batch.publish(0)
    return batch.success_count, batch.fail_count, time.time() - start_batch


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="o2switch Remote Batch Processing Worker")
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS, help="Parallel workers")
    parser.add_argument("--ftp-port", type=int, default=DEFAULT_FTP_PORT, help="NAS FTP port")
    parser.add_argument("--fresh-only", action="store_true", help="Process only virgin unindexed videos")
    parser.add_argument(
        "--retry-failed",
        action="store_true",
        help="Retry only videos whose last run failed in the events log (a non-default --targets-file narrows it)",
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
        "--stream-base",
        default=None,
        help="Explicit stream base URL (e.g. http://127.0.0.1:18089 or ftp://host:port)",
    )
    parser.add_argument(
        "--targets-file",
        default=DEFAULT_TARGETS_PATH,
        help="JSON file listing target video paths; the NAS is scanned when it does not exist",
    )
    parser.add_argument(
        "--mode",
        choices=["phash", "ahash", "both"],
        default="both",
        help="Batch processing mode: phash only, ahash only, or both",
    )
    parser.add_argument(
        "--ahash-dir",
        default=DEFAULT_AHASH_DIR,
        help="Directory storing .ahash binary audio fingerprints",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        dest="check",
        action="store_true",
        help="Inspect environment, binary and directory availability without executing batch",
    )
    return parser


def _handle_check_mode(args: argparse.Namespace, dbsrt_bin: str) -> int:
    """Prints environment inspection and exits with 0."""
    nas_pass = resolve_nas_password() if not args.stream_base else ""
    has_nas_creds = bool(args.nas_host and args.nas_user and nas_pass)
    couplings_file = REPO_ROOT / DEFAULT_COUPLINGS_PATH
    phash_dir = REPO_ROOT / DEFAULT_PHASH_DIR
    ahash_dir = REPO_ROOT / args.ahash_dir
    print("=== DBSrt o2switch Batch Worker Inspection ===")
    print(f"✓ REPO_ROOT     : {REPO_ROOT}")
    print(f"✓ Binary dbsrt  : {dbsrt_bin} ({'found' if Path(dbsrt_bin).exists() else 'missing'})")
    print(f"✓ Mode          : {args.mode}")
    print(f"✓ Workers budget: {args.workers} workers (budget: {CPU_BUDGET_VCPUS} vCPUs)")
    print(f"✓ Couplings path: {couplings_file} ({'found' if couplings_file.is_file() else 'missing'})")
    print(f"✓ pHash dir     : {phash_dir} ({'found' if phash_dir.is_dir() else 'missing'})")
    print(f"✓ aHash dir     : {ahash_dir} ({'found' if ahash_dir.is_dir() else 'missing'})")
    print(f"✓ NAS config    : {'configured' if has_nas_creds else 'unconfigured (stream mode required)'}")
    print(f"✓ Log path      : {REPO_ROOT / DEFAULT_LOG_PATH}")
    return 0


def _narrow_to_targets_file(
    args: argparse.Namespace, targets: List[VideoTarget], all_videos: List[VideoTarget]
) -> List[VideoTarget]:
    """Keeps the targets listed in a non-default --targets-file; the default one is rewritten by every scan."""
    if args.targets_file == DEFAULT_TARGETS_PATH or not Path(args.targets_file).is_file():
        return targets
    listed = match_nas_paths(load_targets_file(Path(args.targets_file), "", set()), all_videos)
    wanted = {video.filename for video in listed}
    return [video for video in targets if video.filename in wanted]


def _initial_targets(
    args: argparse.Namespace,
    all_videos: List[VideoTarget],
    events_log: Path,
    done_stems: Set[str],
    skip_stems: Set[str],
) -> List[VideoTarget]:
    """Failed videos with --retry-failed, else the targets file, else the whole NAS listing."""
    if args.stream_base:
        targets_path = Path(args.targets_file)
        if targets_path.is_file():
            targets = load_targets_file(targets_path, "", skip_stems)
            print(f"✓ Loaded {len(targets)} targets from {targets_path} (stream mode).")
            return targets
        print(f"❌ Error: --stream-base requires a valid targets file ({args.targets_file}).", file=sys.stderr)
        return []
    if args.retry_failed:
        # An explicit retry ignores the blacklist
        targets = _narrow_to_targets_file(
            args, match_nas_paths(resolve_retry_targets(events_log, done_stems), all_videos), all_videos
        )
        print(f"✓ Found {len(targets)} failed videos to retry under {args.nas_base}.")
        return targets
    targets_path = Path(args.targets_file)
    if targets_path.is_file():
        try:
            targets = match_nas_paths(load_targets_file(targets_path, args.nas_base, skip_stems), all_videos)
            print(f"✓ Loaded {len(targets)} targets from {targets_path} found on the NAS.")
            return targets
        except Exception as err:
            print(f"⚠️ Failed reading {targets_path}: {err}. Falling back to remote scan.", file=sys.stderr)
    return [video for video in all_videos if Path(video.filename).stem not in skip_stems]


def resolve_done_stems(
    mode: str,
    certified_stems: Set[str],
    existing_phash: Set[str],
    existing_ahash: Set[str],
) -> Set[str]:
    """Filters completed stems according to the target mode (ahash, phash, or both)."""
    if mode == "ahash":
        print(f"✓ Found {len(existing_ahash)} .ahash files locally (mode: ahash).")
        return {unicodedata.normalize("NFC", stem) for stem in existing_ahash}
    if mode == "phash":
        print(
            f"✓ Found {len(certified_stems)} certified stems and {len(existing_phash)} .phash files locally (mode: phash)."
        )
        return {unicodedata.normalize("NFC", stem) for stem in certified_stems | existing_phash}
    print(f"✓ Found {len(existing_phash)} .phash and {len(existing_ahash)} .ahash files locally (mode: both).")
    return {unicodedata.normalize("NFC", stem) for stem in (certified_stems | existing_phash) & existing_ahash}


def select_targets(args: argparse.Namespace, nas_pass: str, events_log: Path) -> List[VideoTarget]:
    """Steps 1-2: loads what is already done or blacklisted, lists the NAS and picks this run's videos."""
    print("=== [1/4] Scanning local catalogs and couplings ===")
    certified_stems = load_certified_stems(Path(DEFAULT_COUPLINGS_PATH))
    existing_phash = load_existing_phash_stems(Path(DEFAULT_PHASH_DIR))
    existing_ahash = load_existing_ahash_stems(Path(args.ahash_dir))
    done_stems = resolve_done_stems(args.mode, certified_stems, existing_phash, existing_ahash)

    blacklisted = load_blacklisted_stems(events_log)
    print(f"✓ Skipping {len(blacklisted)} videos that failed {MAX_FAILED_ATTEMPTS} times in a row.")
    skip_stems = (done_stems if args.fresh_only else set()) | blacklisted

    if args.stream_base:
        all_videos: List[VideoTarget] = []
    else:
        print(f"=== [2/4] Discovering remote NAS video library (<nas>:{args.nas_base}) ===")
        all_videos = list_nas_videos_ftp(
            nas_host=args.nas_host,
            nas_user=args.nas_user,
            nas_pass=nas_pass,
            base_dir=args.nas_base,
            ftp_port=args.ftp_port,
        )
        print(f"✓ Discovered {len(all_videos)} video files on NAS.")

    targets = _initial_targets(args, all_videos, events_log, done_stems, skip_stems)
    excluded = load_excluded_prefixes(Path(DEFAULT_EXCLUDE_PATH))
    if excluded:
        kept = [video for video in targets if not is_excluded(video.rel_path, excluded)]
        print(f"✓ Excluded {len(targets) - len(kept)} videos listed in {DEFAULT_EXCLUDE_PATH}.")
        targets = kept
    return targets[: args.limit] if args.limit else targets


def _resolve_dbsrt_bin() -> str:
    """Finds dbsrt binary under bin/ or target/."""
    for cand in (REPO_ROOT / "bin/dbsrt", REPO_ROOT / "target/release/dbsrt", REPO_ROOT / "target/debug/dbsrt"):
        if cand.exists():
            return str(cand)
    return "dbsrt"


def _validate_nas_credentials(args: argparse.Namespace, nas_pass: str) -> bool:
    """Verifies NAS credentials in non-stream mode."""
    if args.stream_base:
        return True
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
        return False
    return True


def main() -> int:
    """CLI Entry point for o2switch batch processing."""
    _install_process_handlers()
    args = build_arg_parser().parse_args()
    dbsrt_bin = _resolve_dbsrt_bin()

    if args.check:
        return _handle_check_mode(args, dbsrt_bin)

    nas_pass = resolve_nas_password() if not args.stream_base else ""
    if not _validate_nas_credentials(args, nas_pass):
        return 1

    events_log = Path(DEFAULT_EVENTS_LOG_PATH)
    targets = select_targets(args, nas_pass, events_log)
    print(f"✓ Selected {len(targets)} target videos to process (fresh_only={args.fresh_only}, limit={args.limit}).")
    if not targets:
        print("🎉 No target videos to process. Library is up-to-date!")
        # A drained queue is what batch-status reports as "finished": without it an empty library reads
        # as "crashed" and the supervisor relaunches it instead of rotating to the next one
        write_batch_queue_state(0, 0, 0, 0)
        return 0

    if args.stream_base:
        stream_base = args.stream_base.rstrip("/")
        nas_host = urllib.parse.urlsplit(stream_base).hostname or ""
        print(f"=== [3/4] Direct HTTP Native Streaming Mode ({redact_nas(stream_base, nas_host, '')}) ===")
    else:
        print(f"=== [3/4] Direct FTP Native Streaming Mode (<nas>:{args.ftp_port}) ===")
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
        ahash_dir=args.ahash_dir,
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
