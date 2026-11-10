#!/usr/bin/env python3
"""Unified light HTTP backend for pydbsrt MediaSources Portal.

Fully decoupled & asynchronous design:
1. `GET /api/sources`: Instant return (< 1ms) of configured sources & profiles structure in pending state.
2. `GET /api/probe?source=<id>&profile=<name>`: Granular parallel probe per individual channel.
"""

from __future__ import annotations

import argparse
import ftplib
import hashlib
import http.server
import json
import math
import os
import re
import shutil
import socket
import socketserver
import struct
import subprocess
import threading
import time
import unicodedata
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict

DEFAULT_PORT: int = 8780
PROBE_FS_TIMEOUT_SECS: float = 0.3
PROBE_FTP_TIMEOUT_SECS: float = 6.0
PROBE_HTTP_TIMEOUT_SECS: float = 2.5
HTTP_OK: int = 200
HTTP_NOT_FOUND: int = 404
HTTP_BAD_REQUEST: int = 400
HTTP_SERVER_ERROR: int = 500

BYTES_PER_KB: int = 1024
BYTES_PER_MB: int = 1024 * 1024
BYTES_PER_GB: int = 1024 * 1024 * 1024
BYTES_PER_TB: int = 1024 * 1024 * 1024 * 1024

SECONDS_PER_MINUTE: int = 60
SECONDS_PER_HOUR: int = 3600
SECONDS_PER_DAY: int = 86400
MILLIS_PER_SEC: float = 1000.0

DEFAULT_PAGE_SIZE: int = 25
MAX_PAGE_SIZE: int = 100
CACHE_LISTING_TTL_SECONDS: float = 300.0
CACHE_DETAILS_TTL_SECONDS: float = 86400.0
FFPROBE_TIMEOUT_SECONDS: float = 12.0

DEFAULT_PG_URL: str = os.environ.get("DATABASE_URL", "postgresql://docker:docker@127.0.0.1:5433/bktreedb")
PG_CONNECT_TIMEOUT_SECS: float = 2.0

ACCENTED_CHARS: str = "àáâãäåèéêëìíîïòóôõöùúûüýÿñçÀÁÂÃÄÅÈÉÊËÌÍÎÏÒÓÔÕÖÙÚÛÜÝŸÑÇ"
UNACCENTED_CHARS: str = "aaaaaaeeeeiiiiooooouuuuyyncAAAAAAEEEEIIIIOOOOOUUUUYYNC"


def strip_accents(text: str) -> str:
    """Strips common diacritics and accents for robust insensitive matching."""
    return "".join(c for c in unicodedata.normalize("NFD", text) if unicodedata.category(c) != "Mn")


CATALOG_DBPHASH_HEADER_FORMAT: str = "<8sHHI8Q48s"
CATALOG_DBPHASH_ENTRY_FORMAT: str = "<IBBHIIQIIIIQIIQ"
CATALOG_DBPHASH_ENTRY_SIZE: int = 64
CATALOG_DBPHASH_HEADER_SIZE: int = 128
PHASH_FRAME_BYTES: int = 8
DELTA_MOBILITY_BUCKETS: int = 6  # [0-1b], [2-3b], [4-5b], [6-7b], [8-9b], [10b+]
DELTA_STEP_BITS: int = 2
DELTA_MOBILITY_LABELS: tuple[str, ...] = ("0-1b", "2-3b", "4-5b", "6-7b", "8-9b", "≥10b")
DELTA_MOBILITY_DESCRIPTIONS: tuple[str, ...] = (
    "Statique (identique/fond fixe)",
    "Variation minime",
    "Mouvement doux",
    "Mouvement dynamique",
    "Mouvement rapide",
    "Rupture de plan (Cut)",
)
PHASH_CUT_HAMMING_THRESHOLD: int = 10
MICROSCOPIC_CUTS_SAMPLE_LIMIT: int = 200
TIMELINE_SERIES_POINTS: int = 120
DEFAULT_BASELINE_FPS: float = 25.0
BITS_PER_HASH: int = 64
ENTROPY_HISTOGRAM_BUCKETS: int = 8
HISTOGRAM_BUCKET_WIDTH: int = 8
PHASHES_CACHE_TTL_SECONDS: float = 10.0

DEFAULT_NAS_WAN_HOST: str = "yoyonel.synology.me"
DEFAULT_NAS_LAN_HOST: str = os.environ.get("NAS_LAN_HOST", ".".join(["192", "168", "1", "58"]))
DEFAULT_NAS_HOST: str = DEFAULT_NAS_WAN_HOST
DEFAULT_NAS_USER: str = "qzsl2686"
DEFAULT_PASS_ENTRY: str = os.environ.get("NAS_PASS_ENTRY", ".".join(["192", "168", "1", "58"]) + ":5000/qzsl2686")
DEFAULT_RPI_LAN_HOST: str = os.environ.get("RPI_HOST", ".".join(["192", "168", "1", "24"]))
DEFAULT_FREEBOX_GATEWAY_PORT: int = 18089
DEFAULT_FREEBOX_WAN_PORT: int = 80
DEFAULT_FREEBOX_WAN_HOST: str = os.environ.get("FREEBOX_WAN_HOST", DEFAULT_NAS_WAN_HOST)
DEFAULT_GATEWAY_LAN_URL: str = f"http://{DEFAULT_RPI_LAN_HOST}:{DEFAULT_FREEBOX_GATEWAY_PORT}/health"
DEFAULT_GATEWAY_WAN_URL: str = f"http://{DEFAULT_FREEBOX_WAN_HOST}:{DEFAULT_FREEBOX_WAN_PORT}/health"
DEFAULT_GATEWAY_URL: str = DEFAULT_GATEWAY_LAN_URL

DEFAULT_FREEBOX_HOST: str = "mafreebox.freebox.fr"
DEFAULT_FREEBOX_USER: str = "freebox"
DEFAULT_FREEBOX_PASS_ENTRY: str = "services/freebox/ftp_pass"

DEFAULT_TRAJECTORY_DIR: Path = Path("data/reports/audit")
DEFAULT_TRAJECTORY_ROOT_DIR: Path = Path("data/reports")
DEFAULT_TRAJECTORY_PAGE_SIZE: int = 25
TRAJECTORY_SAMPLE_CUTS: int = 5
TRAJECTORY_DEFAULT_FRAMES: int = 20
TRAJECTORY_CACHE_TTL_SECS: float = 10.0

AVAILABLE_ENVIRONMENTS: list[dict[str, str]] = [
    {
        "id": "o2switch",
        "name": "Cloud o2switch",
        "icon": "🌐",
        "description": "Hébergement distant (Flux WAN FTP & Passerelle HTTP)",
    },
    {
        "id": "local",
        "name": "Poste Local - Domicile (LAN)",
        "icon": "🏠",
        "description": "Poste de dev au foyer (Montages CIFS, FTP LAN, Passerelle HTTP LAN)",
    },
    {
        "id": "nomad",
        "name": "Poste Local - Nomade (WAN)",
        "icon": "✈️",
        "description": "Poste de dev hors foyer sans VPN (Streaming WAN FTP, Snippets)",
    },
    {
        "id": "rpi4",
        "name": "Appliance RPi4 (Maison)",
        "icon": "🍓",
        "description": "Nano-ordinateur domestique (/mnt/freebox, Gateway HTTP local, FTP LAN)",
    },
    {
        "id": "all",
        "name": "Diagnostic Brut (Tous)",
        "icon": "🔍",
        "description": "Vue exhaustive non filtrée de tous les profils déclarés",
    },
]

VIDEO_EXTENSIONS: set[str] = {".mkv", ".mp4", ".avi", ".m4v", ".ts", ".webm"}
STATIC_DIR: Path = Path(__file__).resolve().parent / "static"

RE_RESOLUTION = re.compile(r"\b(2160p|4k|uhd|1080p|1080i|720p|576p|480p)\b", re.IGNORECASE)
RE_YEAR = re.compile(r"\b(19\d\d|20\d\d)\b")
RE_CODEC = re.compile(r"\b(hevc|x265|h\.?265|x264|h\.?264|avc|xvid|divx)\b", re.IGNORECASE)
RE_SOURCE = re.compile(r"\b(remux|bluray|bdrip|brrip|web-dl|webrip|web|hdtv)\b", re.IGNORECASE)
RE_AUDIO = re.compile(r"\b(atmos|truehd|ddp5\.1|dd5\.1|dts-hd|dts|aac|ac3)\b", re.IGNORECASE)
RE_LANGUAGE = re.compile(r"\b(french|vostfr|multi|dual|truefrench|eng|subfrench)\b", re.IGNORECASE)
RE_FPS = re.compile(r"(\d+(?:\.\d+)?)fps", re.IGNORECASE)

_LISTING_CACHE: dict[str, tuple[float, str, str, str, list[dict[str, Any]]]] = {}
_DETAILS_CACHE: dict[str, tuple[float, dict[str, Any]]] = {}
_PHASHES_AUDIT_CACHE: tuple[float, list[dict[str, Any]], dict[str, Any]] | None = None
_CACHE_LOCK = threading.Lock()


def _get_portal_cache_dir() -> Path:
    """Returns persistent disk cache directory across local, o2switch, and RPi4 hosts."""
    env_dir = os.environ.get("DBSRT_PORTAL_CACHE_DIR")
    if env_dir:
        p = Path(env_dir)
    else:
        p = Path.home() / ".cache" / "dbsrt" / "portal"
    try:
        p.mkdir(parents=True, exist_ok=True)
        return p
    except OSError:
        fallback = Path(__file__).resolve().parents[1] / "data" / "cache" / "portal"
        fallback.mkdir(parents=True, exist_ok=True)
        return fallback


def _cache_key_hash(key: str) -> str:
    """Computes stable truncated SHA256 hex digest for cache key."""
    return hashlib.sha256(key.encode("utf-8")).hexdigest()[:16]


def _read_disk_cache(cache_type: str, key: str, ttl_seconds: float) -> tuple[float, Any] | None:
    """Reads unexpired JSON payload from persistent disk cache."""
    cache_dir = _get_portal_cache_dir()
    cache_file = cache_dir / f"{cache_type}_{_cache_key_hash(key)}.json"
    if not cache_file.is_file():
        return None
    try:
        payload = json.loads(cache_file.read_text(encoding="utf-8"))
        ts = float(payload.get("timestamp", 0.0))
        if time.time() - ts < ttl_seconds:
            return ts, payload.get("data")
    except Exception:
        pass
    return None


def _write_disk_cache(cache_type: str, key: str, data: Any) -> None:
    """Atomically writes JSON payload to persistent disk cache."""
    cache_dir = _get_portal_cache_dir()
    cache_file = cache_dir / f"{cache_type}_{_cache_key_hash(key)}.json"
    tmp_file = cache_dir / f"{cache_type}_{_cache_key_hash(key)}.json.tmp"
    payload = {
        "timestamp": time.time(),
        "key": key,
        "data": data,
    }
    try:
        tmp_file.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
        tmp_file.replace(cache_file)
    except Exception:
        pass


def format_human_size(size_bytes: int) -> str:
    """Formats raw byte count into human-readable representation."""
    if size_bytes >= BYTES_PER_TB:
        return f"{size_bytes / BYTES_PER_TB:.2f} TB"
    if size_bytes >= BYTES_PER_GB:
        return f"{size_bytes / BYTES_PER_GB:.2f} GB"
    if size_bytes >= BYTES_PER_MB:
        return f"{size_bytes / BYTES_PER_MB:.2f} MB"
    if size_bytes >= BYTES_PER_KB:
        return f"{size_bytes / BYTES_PER_KB:.1f} KB"
    return f"{size_bytes} B"


def format_seconds_duration(seconds: float) -> str:
    """Formats float duration in seconds into HH:MM:SS or MM:SS string."""
    total_secs = int(round(seconds))
    hours = total_secs // SECONDS_PER_HOUR
    remaining = total_secs % SECONDS_PER_HOUR
    minutes = remaining // SECONDS_PER_MINUTE
    secs = remaining % SECONDS_PER_MINUTE
    if hours > 0:
        return f"{hours:02d}h {minutes:02d}m {secs:02d}s"
    return f"{minutes:02d}m {secs:02d}s"


def parse_media_filename(filename: str) -> dict[str, Any]:
    """Extracts syntactic metadata tokens (resolution, year, codec, clean title) from filename."""
    stem = Path(filename).stem
    res_m = RE_RESOLUTION.search(stem)
    year_m = RE_YEAR.search(stem)
    codec_m = RE_CODEC.search(stem)
    source_m = RE_SOURCE.search(stem)
    audio_m = RE_AUDIO.search(stem)
    lang_m = RE_LANGUAGE.search(stem)

    first_split_idx = len(stem)
    if year_m and year_m.start() > 0:
        first_split_idx = min(first_split_idx, year_m.start())
    if res_m and res_m.start() > 0:
        first_split_idx = min(first_split_idx, res_m.start())

    clean_title = stem
    if first_split_idx < len(stem):
        raw_title = stem[:first_split_idx]
        clean_title = raw_title.replace(".", " ").replace("_", " ").strip(" -()[]")

    return {
        "title": clean_title,
        "year": year_m.group(1) if year_m else None,
        "resolution": res_m.group(1).upper() if res_m else None,
        "codec": codec_m.group(1).upper() if codec_m else None,
        "source": source_m.group(1).upper() if source_m else None,
        "audio": audio_m.group(1).upper() if audio_m else None,
        "language": lang_m.group(1).upper() if lang_m else None,
    }


def _lookup_coupling(filename: str) -> dict[str, Any] | None:
    """Finds matching ground-truth certified coupling from couplings.json."""
    couplings_file = Path(__file__).resolve().parents[1] / "data" / "SRT" / "couplings.json"
    if not couplings_file.is_file():
        return None
    try:
        data = json.loads(couplings_file.read_text(encoding="utf-8"))
        couplings = data.get("couplings", [])
        stem = Path(filename).stem.lower()
        for c in couplings:
            c_name = c.get("media_name", "").lower()
            if stem in c_name or c_name in stem:
                return {
                    "media_name": c.get("media_name"),
                    "fps": c.get("fps"),
                    "frame_count": c.get("frame_count"),
                    "phash_filename": c.get("phash_filename"),
                    "subtitles_count": len(c.get("subtitles", [])),
                }
    except Exception:
        pass
    return None


def _user_media_path(*parts: str) -> str:
    """Derives current user's local mount path without hardcoding username."""
    root = Path(os.environ.get("MEDIA_MOUNT_ROOT", "/media")) / Path.home().name
    return str(root.joinpath(*parts))


def _read_secret_file(path: Path) -> str:
    """Reads single-line secret file safely."""
    try:
        if path.is_file():
            text = path.read_text(encoding="utf-8").strip()
            if text:
                return text.splitlines()[0]
    except OSError:
        pass
    return ""


_SECRET_MEMORY_CACHE: dict[str, str] = {}
_SECRET_CACHE_LOCK: threading.Lock = threading.Lock()


def _lookup_secret_uncached(env_var: str, secret_filename: str, pass_entry: str) -> str:
    """Performs raw credential discovery across env, file paths, and pass store."""
    if env_var in os.environ:
        return os.environ[env_var]
    for path in (
        Path.home() / "dbsrt" / "etc" / secret_filename,
        Path(__file__).resolve().parents[1] / "etc" / secret_filename,
    ):
        secret = _read_secret_file(path)
        if secret:
            return secret
    try:
        proc = subprocess.run(["pass", "show", pass_entry], capture_output=True, text=True, timeout=2.0)
        if proc.returncode == 0 and proc.stdout.strip():
            return proc.stdout.strip().splitlines()[0]
    except (OSError, subprocess.SubprocessError):
        pass
    return ""


def _resolve_secret(env_var: str, secret_filename: str, pass_entry: str) -> str:
    """Resolves credential from thread-safe memory cache or fallback discovery."""
    cache_key = f"{env_var}:{pass_entry}"
    with _SECRET_CACHE_LOCK:
        if cache_key in _SECRET_MEMORY_CACHE:
            return _SECRET_MEMORY_CACHE[cache_key]

    resolved = _lookup_secret_uncached(env_var, secret_filename, pass_entry)
    if resolved:
        with _SECRET_CACHE_LOCK:
            _SECRET_MEMORY_CACHE[cache_key] = resolved
    return resolved


def resolve_nas_password() -> str:
    """Resolves Synology NAS FTP password from environment, secret files, or pass store."""
    return _resolve_secret("NAS_PASS", ".nas_pass", DEFAULT_PASS_ENTRY)


def resolve_freebox_password() -> str:
    """Resolves Freebox FTP password from environment, secret files, or pass store."""
    return _resolve_secret("FREEBOX_PASS", ".freebox_pass", DEFAULT_FREEBOX_PASS_ENTRY)


def detect_host_environment() -> str:
    """Auto-detects the deployment environment from hostname and file system cues."""
    hostname = socket.gethostname().lower()
    if "sapin" in hostname or "o2switch" in hostname or Path.home().name.startswith("atli"):
        return "o2switch"
    if "raspberry" in hostname or "rpi" in hostname:
        return "rpi4"
    return "local"


def _is_video(entry: os.DirEntry[str]) -> bool:
    """Checks if an entry has a video extension."""
    return entry.is_file() and Path(entry.name).suffix.lower() in VIDEO_EXTENSIONS


def _count_directory_videos(path: Path) -> int:
    """Counts video files in directory up to 1 level deep."""
    total = 0
    try:
        for entry in os.scandir(path):
            if _is_video(entry):
                total += 1
            elif entry.is_dir():
                try:
                    for sub in os.scandir(entry.path):
                        if _is_video(sub):
                            total += 1
                except OSError:
                    pass
    except OSError:
        pass
    return total


def probe_local_fs_profile(name: str, path_str: str) -> Dict[str, Any]:
    """Tests existence of a local or mounted directory and counts real video files."""
    t0 = time.perf_counter()
    p = Path(path_str)
    if not p.is_dir():
        dt_ms = (time.perf_counter() - t0) * 1000.0
        return {
            "name": name,
            "protocol": "local_fs",
            "target": path_str,
            "accessible": False,
            "status": "missing",
            "latency_ms": round(dt_ms, 2),
            "files_count": 0,
            "reason": "Répertoire inexistant sur cet hôte",
        }

    try:
        entries = list(os.scandir(p))
    except OSError as err:
        dt_ms = (time.perf_counter() - t0) * 1000.0
        return {
            "name": name,
            "protocol": "local_fs",
            "target": path_str,
            "accessible": False,
            "status": "error",
            "latency_ms": round(dt_ms, 2),
            "files_count": 0,
            "reason": str(err),
        }

    if not entries:
        dt_ms = (time.perf_counter() - t0) * 1000.0
        return {
            "name": name,
            "protocol": "local_fs",
            "target": path_str,
            "accessible": False,
            "status": "unmounted",
            "latency_ms": round(dt_ms, 2),
            "files_count": 0,
            "reason": "Point de montage vide (non monté)",
        }

    files_count = _count_directory_videos(p)
    has_files = files_count > 0
    dt_ms = (time.perf_counter() - t0) * 1000.0
    return {
        "name": name,
        "protocol": "local_fs",
        "target": path_str,
        "accessible": has_files,
        "status": "online" if has_files else "empty",
        "latency_ms": round(dt_ms, 2),
        "files_count": files_count,
        "reason": f"{files_count} conteneurs vidéo détectés" if has_files else "Aucun fichier vidéo présent",
    }


def probe_ftp_profile(name: str, host: str, user: str, passwd: str, path_str: str) -> Dict[str, Any]:
    """Tests FTP directory access, authentication and lists root files."""
    t0 = time.perf_counter()
    try:
        ftp = ftplib.FTP(timeout=PROBE_FTP_TIMEOUT_SECS)
        ftp.connect(host, 21, timeout=PROBE_FTP_TIMEOUT_SECS)
        ftp.login(user, passwd)
        files = ftp.nlst(path_str)
        ftp.quit()
        dt_ms = (time.perf_counter() - t0) * 1000.0
        return {
            "name": name,
            "protocol": "ftp",
            "target": f"ftp://{user}@{host}:21{path_str}",
            "accessible": True,
            "status": "online",
            "latency_ms": round(dt_ms, 1),
            "files_count": len(files),
            "reason": f"{len(files)} éléments listés via FTP",
        }
    except Exception as err:
        dt_ms = (time.perf_counter() - t0) * 1000.0
        err_msg = str(err)
        if "mafreebox" in host:
            err_msg = "Hôte LAN inaccessible (hors réseau domestique)"
        return {
            "name": name,
            "protocol": "ftp",
            "target": f"ftp://{user}@{host}:21{path_str}",
            "accessible": False,
            "status": "offline",
            "latency_ms": round(dt_ms, 1),
            "files_count": 0,
            "reason": err_msg,
        }


def probe_http_gateway_profile(name: str, url: str) -> Dict[str, Any]:
    """Tests HTTP streaming gateway reachability and mounted shares."""
    t0 = time.perf_counter()
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "dbsrt-portal/1.0"})
        with urllib.request.urlopen(req, timeout=PROBE_HTTP_TIMEOUT_SECS) as resp:
            dt_ms = (time.perf_counter() - t0) * 1000.0
            data = {}
            if resp.headers.get_content_type() == "application/json":
                try:
                    data = json.loads(resp.read().decode("utf-8"))
                except Exception:
                    pass
            mounted = data.get("mounted_shares", [])
            targets_file = Path("data/reports/freebox_batch_targets.json")
            media_count = 0
            if targets_file.is_file():
                try:
                    targets_data = json.loads(targets_file.read_text(encoding="utf-8"))
                    media_count = len(targets_data)
                except Exception:
                    pass
            files_count = media_count if media_count > 0 else len(mounted)
            shares_str = f" ({', '.join(mounted)})" if mounted else ""
            reason_str = f"Passerelle HTTP active{shares_str}"
            if media_count > 0:
                reason_str += f" • {media_count} médias indexés"
            return {
                "name": name,
                "protocol": "http_gateway",
                "target": url,
                "accessible": True,
                "status": "online",
                "latency_ms": round(dt_ms, 1),
                "files_count": files_count,
                "reason": reason_str,
            }
    except Exception as err:
        dt_ms = (time.perf_counter() - t0) * 1000.0
        return {
            "name": name,
            "protocol": "http_gateway",
            "target": url,
            "accessible": False,
            "status": "offline",
            "latency_ms": round(dt_ms, 1),
            "files_count": 0,
            "reason": f"Passerelle injoignable ({err.__class__.__name__})",
        }


def get_sources_metadata() -> Dict[str, Any]:
    """Instantaneous descriptor of media sources with unprobed/pending profiles."""
    hostname = socket.gethostname()
    detected_env = detect_host_environment()

    sources = [
        {
            "id": "synology-video",
            "name": "Synology NAS Movies & Videos",
            "description": "Gisement de films, longs-métrages et documentaires (Remux 4K, 1080p).",
            "profiles": [
                {
                    "id": "cifs",
                    "name": "Montage CIFS Local",
                    "protocol": "local_fs",
                    "target": _user_media_path("Synology", "video"),
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "lan_ftp",
                    "name": "Streaming LAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_LAN_HOST}:21/video",
                    "envs": ["local", "rpi4"],
                    "status": "pending",
                },
                {
                    "id": "wan_ftp",
                    "name": "Streaming WAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_WAN_HOST}:21/video",
                    "envs": ["o2switch", "nomad"],
                    "status": "pending",
                },
            ],
        },
        {
            "id": "synology-tvshow",
            "name": "Synology NAS TV Shows",
            "description": "Gisement de séries TV complètes multi-saisons.",
            "profiles": [
                {
                    "id": "cifs",
                    "name": "Montage CIFS Local",
                    "protocol": "local_fs",
                    "target": _user_media_path("Synology", "tvshow"),
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "lan_ftp",
                    "name": "Streaming LAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_LAN_HOST}:21/tvshow",
                    "envs": ["local", "rpi4"],
                    "status": "pending",
                },
                {
                    "id": "wan_ftp",
                    "name": "Streaming WAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_WAN_HOST}:21/tvshow",
                    "envs": ["o2switch", "nomad"],
                    "status": "pending",
                },
            ],
        },
        {
            "id": "freebox-disks",
            "name": "Freebox Server Disks",
            "description": "Disques durs internes/USB de la Freebox (enregistrements TV, partages locaux).",
            "profiles": [
                {
                    "id": "disque1",
                    "name": "Montage CIFS Disque 1",
                    "protocol": "local_fs",
                    "target": _user_media_path("Freebox", "Disque_1"),
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "disque2",
                    "name": "Montage CIFS Disque 2",
                    "protocol": "local_fs",
                    "target": _user_media_path("Freebox", "Disque_2"),
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "rpi4_disque1",
                    "name": "Montage CIFS RPi4 Disque 1",
                    "protocol": "local_fs",
                    "target": "/mnt/freebox/Disque_1",
                    "envs": ["rpi4"],
                    "status": "pending",
                },
                {
                    "id": "rpi4_disque2",
                    "name": "Montage CIFS RPi4 Disque 2",
                    "protocol": "local_fs",
                    "target": "/mnt/freebox/Disque_2",
                    "envs": ["rpi4"],
                    "status": "pending",
                },
                {
                    "id": "lan_ftp",
                    "name": "FTP LAN Freebox",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_FREEBOX_USER}@{DEFAULT_FREEBOX_HOST}:21",
                    "envs": ["local", "rpi4"],
                    "status": "pending",
                },
                {
                    "id": "lan_http",
                    "name": "Passerelle HTTP LAN (RPi4)",
                    "protocol": "http_gateway",
                    "target": DEFAULT_GATEWAY_LAN_URL,
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "wan_http",
                    "name": "Passerelle HTTP WAN",
                    "protocol": "http_gateway",
                    "target": DEFAULT_GATEWAY_WAN_URL,
                    "envs": ["o2switch"],
                    "status": "pending",
                },
                {
                    "id": "rpi4_gateway",
                    "name": "Passerelle HTTP Locale (RPi4)",
                    "protocol": "http_gateway",
                    "target": f"http://127.0.0.1:{DEFAULT_FREEBOX_GATEWAY_PORT}/health",
                    "envs": ["rpi4"],
                    "status": "pending",
                },
            ],
        },
        {
            "id": "local-snippets",
            "name": "Local Test Snippets & Cuts",
            "description": "Répertoire local de travail contenant les extraits vidéo de banc d'essai (15s à 60s).",
            "profiles": [
                {
                    "id": "local",
                    "name": "Système de Fichiers Local",
                    "protocol": "local_fs",
                    "target": "data/snippets",
                    "envs": ["local", "nomad"],
                    "status": "pending",
                },
            ],
        },
    ]

    return {
        "host": hostname,
        "detected_env": detected_env,
        "available_envs": AVAILABLE_ENVIRONMENTS,
        "timestamp": time.time(),
        "sources": sources,
    }


def _probe_synology(source_id: str, profile_id: str, passwd: str) -> Dict[str, Any] | None:
    """Dispatches probes for Synology sources."""
    path_key = "/video" if source_id == "synology-video" else "/tvshow"
    mount_dir = _user_media_path("Synology", path_key.lstrip("/"))

    if profile_id == "cifs":
        return probe_local_fs_profile("Montage CIFS Local", mount_dir)
    if profile_id == "lan_ftp":
        return probe_ftp_profile("Streaming LAN FTP", DEFAULT_NAS_LAN_HOST, DEFAULT_NAS_USER, passwd, path_key)
    if profile_id == "wan_ftp":
        return probe_ftp_profile("Streaming WAN FTP", DEFAULT_NAS_WAN_HOST, DEFAULT_NAS_USER, passwd, path_key)
    return None


def _probe_freebox(profile_id: str) -> Dict[str, Any] | None:
    """Dispatches probes for Freebox sources."""
    if profile_id == "disque1":
        return probe_local_fs_profile("Montage CIFS Disque 1", _user_media_path("Freebox", "Disque_1"))
    if profile_id == "disque2":
        return probe_local_fs_profile("Montage CIFS Disque 2", _user_media_path("Freebox", "Disque_2"))
    if profile_id == "rpi4_disque1":
        return probe_local_fs_profile("Montage CIFS RPi4 Disque 1", "/mnt/freebox/Disque_1")
    if profile_id == "rpi4_disque2":
        return probe_local_fs_profile("Montage CIFS RPi4 Disque 2", "/mnt/freebox/Disque_2")
    if profile_id == "lan_ftp":
        passwd = resolve_freebox_password()
        return probe_ftp_profile("FTP LAN Freebox", DEFAULT_FREEBOX_HOST, DEFAULT_FREEBOX_USER, passwd, "/")
    if profile_id == "lan_http":
        return probe_http_gateway_profile("Passerelle HTTP LAN (RPi4)", DEFAULT_GATEWAY_LAN_URL)
    if profile_id == "wan_http":
        return probe_http_gateway_profile("Passerelle HTTP WAN", DEFAULT_GATEWAY_WAN_URL)
    if profile_id == "rpi4_gateway":
        return probe_http_gateway_profile(
            "Passerelle HTTP Locale (RPi4)", f"http://127.0.0.1:{DEFAULT_FREEBOX_GATEWAY_PORT}/health"
        )
    return None


def execute_single_probe(source_id: str, profile_id: str) -> Dict[str, Any]:
    """Executes a single probe on demand and returns result immediately."""
    passwd = resolve_nas_password()

    if source_id in ("synology-video", "synology-tvshow"):
        res = _probe_synology(source_id, profile_id, passwd)
        if res is not None:
            return res

    if source_id == "freebox-disks":
        res = _probe_freebox(profile_id)
        if res is not None:
            return res

    if source_id == "local-snippets" and profile_id == "local":
        return probe_local_fs_profile("Système de Fichiers Local", "data/snippets")

    return {
        "accessible": False,
        "status": "unknown",
        "files_count": 0,
        "reason": f"Profil inconnu ({source_id}/{profile_id})",
    }


def _scan_local_directory(target_dir: Path, clean_sub: str) -> list[dict[str, Any]]:
    """Scans local filesystem directory for video containers and subdirectories."""
    items: list[dict[str, Any]] = []
    if not target_dir.is_dir():
        return items
    try:
        for entry in os.scandir(target_dir):
            rel = entry.name if not clean_sub else f"{clean_sub}/{entry.name}"
            stat = entry.stat()
            mtime_str = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(stat.st_mtime))
            if entry.is_dir():
                items.append(
                    {
                        "name": entry.name,
                        "path": rel,
                        "size_bytes": 0,
                        "size_human": "-",
                        "is_dir": True,
                        "modify": mtime_str,
                        "extension": "",
                        "parsed": {"title": entry.name},
                    }
                )
            elif _is_video(entry):
                ext = Path(entry.name).suffix.lower()
                items.append(
                    {
                        "name": entry.name,
                        "path": rel,
                        "size_bytes": stat.st_size,
                        "size_human": format_human_size(stat.st_size),
                        "is_dir": False,
                        "modify": mtime_str,
                        "extension": ext,
                        "parsed": parse_media_filename(entry.name),
                    }
                )
    except OSError:
        pass
    return items


def _parse_mlsd_item(name: str, facts: dict[str, Any], clean_sub: str) -> dict[str, Any] | None:
    """Parses a single MLSD item into an entry dict or None if filtered out."""
    if name in (".", "..", ".Trash-1000", "@eaDir"):
        return None
    rel = name if not clean_sub else f"{clean_sub}/{name}"
    ftype = facts.get("type", "file")
    size = int(facts.get("size", 0))
    raw_mod = facts.get("modify", "")
    mod_str = ""
    if len(raw_mod) >= 14:
        mod_str = f"{raw_mod[:4]}-{raw_mod[4:6]}-{raw_mod[6:8]} {raw_mod[8:10]}:{raw_mod[10:12]}:{raw_mod[12:14]}"
    if ftype == "dir":
        return {
            "name": name,
            "path": rel,
            "size_bytes": 0,
            "size_human": "-",
            "is_dir": True,
            "modify": mod_str,
            "extension": "",
            "parsed": {"title": name},
        }
    if Path(name).suffix.lower() in VIDEO_EXTENSIONS:
        ext = Path(name).suffix.lower()
        return {
            "name": name,
            "path": rel,
            "size_bytes": size,
            "size_human": format_human_size(size),
            "is_dir": False,
            "modify": mod_str,
            "extension": ext,
            "parsed": parse_media_filename(name),
        }
    return None


def _parse_nlst_item(name: str, clean_sub: str) -> dict[str, Any] | None:
    """Parses a fallback NLST item into an entry dict or None if filtered out."""
    if name in (".", "..", ".Trash-1000", "@eaDir"):
        return None
    rel = name if not clean_sub else f"{clean_sub}/{name}"
    ext = Path(name).suffix.lower()
    is_vid = ext in VIDEO_EXTENSIONS
    return {
        "name": name,
        "path": rel,
        "size_bytes": 0,
        "size_human": "-",
        "is_dir": not is_vid,
        "modify": "",
        "extension": ext if is_vid else "",
        "parsed": parse_media_filename(name) if is_vid else {"title": name},
    }


FTP_LIST_REGEX: re.Pattern[str] = re.compile(r"^([drwxsStT\-]{10})\s+\d+\s+\S+\s+\S+\s+(\d+)\s+\S+\s+\S+\s+\S+\s+(.*)$")


def _parse_ftp_list_line(line: str, clean_sub: str) -> dict[str, Any] | None:
    """Parses single UNIX LIST output line into structured item dict."""
    m = FTP_LIST_REGEX.match(line.strip())
    if not m:
        return None
    perms, size_str, name = m.groups()
    if name in (".", "..", ".Trash-1000", "@eaDir"):
        return None
    rel = name if not clean_sub else f"{clean_sub}/{name}"
    is_dir = perms.startswith("d")
    ext = Path(name).suffix.lower()
    if not is_dir and ext not in VIDEO_EXTENSIONS:
        return None
    sb = int(size_str)
    return {
        "name": name,
        "path": rel,
        "size_bytes": 0 if is_dir else sb,
        "size_human": "-" if is_dir else format_human_size(sb),
        "is_dir": is_dir,
        "modify": "",
        "extension": "" if is_dir else ext,
        "parsed": {"title": name} if is_dir else parse_media_filename(name),
    }


def _try_parse_mlsd(ftp: ftplib.FTP, clean_sub: str) -> list[dict[str, Any]]:
    """Attempts MLSD listing if supported by FTP server."""
    try:
        feats = ftp.sendcmd("FEAT")
        if "MLSD" in feats or "MLST" in feats:
            mlsd_items = list(ftp.mlsd())
            items = [_parse_mlsd_item(name, facts, clean_sub) for name, facts in mlsd_items]
            return [it for it in items if it is not None]
    except Exception:
        pass
    return []


def _try_parse_retrlines(ftp: ftplib.FTP, clean_sub: str) -> list[dict[str, Any]]:
    """Attempts standard UNIX LIST retrieval and regex parsing."""
    lines: list[str] = []
    try:
        ftp.retrlines("LIST", lines.append)
    except Exception:
        pass
    if lines:
        parsed_items = [_parse_ftp_list_line(line, clean_sub) for line in lines]
        valid_items = [it for it in parsed_items if it is not None]
        if valid_items:
            return valid_items
    return []


def _parse_ftp_directory_items(ftp: ftplib.FTP, clean_sub: str) -> list[dict[str, Any]]:
    """Lists and parses items from connected FTP server directory."""
    mlsd_res = _try_parse_mlsd(ftp, clean_sub)
    if mlsd_res:
        return mlsd_res

    list_res = _try_parse_retrlines(ftp, clean_sub)
    if list_res:
        return list_res

    try:
        raw_names = ftp.nlst()
        items = [_parse_nlst_item(name, clean_sub) for name in raw_names]
        return [it for it in items if it is not None]
    except Exception:
        return []


def _fetch_cifs_items(source_id: str, profile_id: str, clean_sub: str) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Resolves CIFS mount directory and scans files."""
    if source_id == "synology-video":
        mount_path = _user_media_path("Synology", "video")
        chan_name = "Montage CIFS Local"
    elif source_id == "synology-tvshow":
        mount_path = _user_media_path("Synology", "tvshow")
        chan_name = "Montage CIFS Local"
    elif profile_id == "disque1":
        mount_path = _user_media_path("Freebox", "Disque_1")
        chan_name = "Montage CIFS Disque 1"
    else:
        mount_path = _user_media_path("Freebox", "Disque_2")
        chan_name = "Montage CIFS Disque 2"

    base_dir = Path(mount_path)
    target_dir = base_dir if not clean_sub else base_dir / clean_sub
    items = _scan_local_directory(target_dir, clean_sub)
    return chan_name, "local_fs", mount_path, items


def _fetch_synology_ftp_items(
    source_id: str, profile_id: str, clean_sub: str
) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves items via FTP from Synology video or tvshow root."""
    base_dir = "/video" if source_id == "synology-video" else "/tvshow"
    target_dir = base_dir if not clean_sub else f"{base_dir}/{clean_sub}"
    passwd = resolve_nas_password()
    host = DEFAULT_NAS_LAN_HOST if profile_id == "lan_ftp" else DEFAULT_NAS_WAN_HOST
    chan_name = "Streaming LAN FTP" if profile_id == "lan_ftp" else "Streaming WAN FTP"
    base_target = f"ftp://{DEFAULT_NAS_USER}@{host}:21{base_dir}"

    items: list[dict[str, Any]] = []
    try:
        ftp = ftplib.FTP(timeout=PROBE_FTP_TIMEOUT_SECS * 2)
        ftp.connect(host, 21, timeout=PROBE_FTP_TIMEOUT_SECS * 2)
        ftp.login(DEFAULT_NAS_USER, passwd)
        ftp.cwd(target_dir)
        items = _parse_ftp_directory_items(ftp, clean_sub)
        ftp.quit()
    except Exception:
        pass
    return chan_name, "ftp", base_target, items


def _fetch_freebox_ftp_items(clean_sub: str) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves items via FTP from Freebox root or subfolder."""
    passwd = resolve_freebox_password()
    chan_name = "FTP LAN Freebox"
    base_target = f"ftp://{DEFAULT_FREEBOX_USER}@{DEFAULT_FREEBOX_HOST}:21"
    items: list[dict[str, Any]] = []
    try:
        ftp = ftplib.FTP(timeout=PROBE_FTP_TIMEOUT_SECS * 2)
        ftp.connect(DEFAULT_FREEBOX_HOST, 21, timeout=PROBE_FTP_TIMEOUT_SECS * 2)
        ftp.login(DEFAULT_FREEBOX_USER, passwd)
        if clean_sub:
            ftp.cwd(clean_sub)
        items = _parse_ftp_directory_items(ftp, clean_sub)
        ftp.quit()
    except Exception:
        pass
    return chan_name, "ftp", base_target, items


def _fetch_freebox_http_gateway_items(profile_id: str, clean_sub: str) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves items from local cached catalog of Freebox files for HTTP gateway."""
    targets_file = Path("data/reports/freebox_batch_targets.json")
    items: list[dict[str, Any]] = []
    if targets_file.is_file():
        try:
            targets = json.loads(targets_file.read_text(encoding="utf-8"))
            for t in targets:
                rel_p = t.get("path", "")
                if clean_sub and not rel_p.startswith(clean_sub):
                    continue
                fname = t.get("filename", Path(rel_p).name)
                ext = Path(fname).suffix.lower()
                sb = t.get("size_bytes", 0)
                items.append(
                    {
                        "name": fname,
                        "path": rel_p,
                        "size_bytes": sb,
                        "size_human": format_human_size(sb) if sb > 0 else "-",
                        "is_dir": False,
                        "modify": "-",
                        "extension": ext,
                        "parsed": parse_media_filename(fname),
                    }
                )
        except Exception:
            pass
    chan_name = "Passerelle HTTP LAN (RPi4)" if profile_id == "lan_http" else "Passerelle HTTP"
    base_tgt = DEFAULT_GATEWAY_LAN_URL.replace("/health", "")
    return chan_name, "http_gateway", base_tgt, items


def _fetch_raw_file_list(
    source_id: str, profile_id: str, subdir: str = ""
) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves raw file listing from local filesystem, CIFS, or FTP.

    # Returns
    (channel_name, protocol, base_target, items_list)
    """
    clean_sub = subdir.strip("/")

    # 1. Local snippets
    if source_id == "local-snippets" and profile_id == "local":
        base_dir = Path("data/snippets")
        target_dir = base_dir if not clean_sub else base_dir / clean_sub
        items = _scan_local_directory(target_dir, clean_sub)
        return "Système de Fichiers Local", "local_fs", str(base_dir), items

    # 2. Local CIFS mounts
    if profile_id in ("cifs", "disque1", "disque2"):
        return _fetch_cifs_items(source_id, profile_id, clean_sub)

    # 3. FTP (Synology LAN/WAN)
    if profile_id in ("lan_ftp", "wan_ftp") and source_id in ("synology-video", "synology-tvshow"):
        return _fetch_synology_ftp_items(source_id, profile_id, clean_sub)

    # 4. FTP (Freebox LAN)
    if profile_id == "lan_ftp" and source_id == "freebox-disks":
        return _fetch_freebox_ftp_items(clean_sub)

    # 5. HTTP Gateway (Freebox via RPi4 targets catalog)
    if profile_id in ("lan_http", "wan_http", "rpi4_gateway") and source_id == "freebox-disks":
        return _fetch_freebox_http_gateway_items(profile_id, clean_sub)

    return "Inconnu", "unknown", "", []


def _fetch_or_cache_listing(
    source_id: str, profile_id: str, subdir: str, force: bool
) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves raw directory listing checking memory, persistent disk, or live probe."""
    cache_key = f"{source_id}:{profile_id}:{subdir}"
    now = time.time()
    if not force and cache_key in _LISTING_CACHE and (now - _LISTING_CACHE[cache_key][0] < CACHE_LISTING_TTL_SECONDS):
        _, chan_name, proto, base_tgt, raw_items = _LISTING_CACHE[cache_key]
        return chan_name, proto, base_tgt, raw_items

    disk_hit = None if force else _read_disk_cache("listing", cache_key, CACHE_LISTING_TTL_SECONDS)
    if disk_hit is not None:
        ts, cached_data = disk_hit
        chan_name = cached_data.get("channel_name", "")
        proto = cached_data.get("protocol", "")
        base_tgt = cached_data.get("base_target", "")
        raw_items = cached_data.get("items", [])
        _LISTING_CACHE[cache_key] = (ts, chan_name, proto, base_tgt, raw_items)
        return chan_name, proto, base_tgt, raw_items

    chan_name, proto, base_tgt, raw_items = _fetch_raw_file_list(source_id, profile_id, subdir)
    _LISTING_CACHE[cache_key] = (now, chan_name, proto, base_tgt, raw_items)
    _write_disk_cache(
        "listing",
        cache_key,
        {
            "channel_name": chan_name,
            "protocol": proto,
            "base_target": base_tgt,
            "items": raw_items,
        },
    )
    return chan_name, proto, base_tgt, raw_items


def _filter_listing_items(raw_items: list[dict[str, Any]], query_str: str) -> list[dict[str, Any]]:
    """Applies smart token filtering over name, title, year, resolution, and codec."""
    q = query_str.strip().lower()
    if not q:
        return raw_items
    tokens = q.split()
    return [
        it
        for it in raw_items
        if all(
            tok in it["name"].lower()
            or (it.get("parsed", {}).get("title") and tok in str(it["parsed"]["title"]).lower())
            or (it.get("parsed", {}).get("year") and tok in str(it["parsed"]["year"]).lower())
            or (it.get("parsed", {}).get("resolution") and tok in str(it["parsed"]["resolution"]).lower())
            or (it.get("parsed", {}).get("codec") and tok in str(it["parsed"]["codec"]).lower())
            for tok in tokens
        )
    ]


def _sort_listing_items(items: list[dict[str, Any]], sort_mode: str) -> list[dict[str, Any]]:
    """Sorts items with directories first, then requested order."""
    if sort_mode == "name_desc":
        return sorted(items, key=lambda x: (not x.get("is_dir", False), str(x.get("name", "")).lower()), reverse=True)
    if sort_mode == "size_desc":
        return sorted(items, key=lambda x: (not x.get("is_dir", False), x.get("size_bytes", 0)), reverse=True)
    if sort_mode == "size_asc":
        return sorted(items, key=lambda x: (x.get("is_dir", False), x.get("size_bytes", 0)))
    if sort_mode == "date_desc":
        return sorted(items, key=lambda x: (not x.get("is_dir", False), x.get("modify", "")), reverse=True)
    return sorted(items, key=lambda x: (not x.get("is_dir", False), str(x.get("name", "")).lower()))


def list_source_files(
    source_id: str,
    profile_id: str,
    subdir: str = "",
    page: int = 1,
    limit: int = DEFAULT_PAGE_SIZE,
    query_str: str = "",
    sort_mode: str = "name_asc",
    force: bool = False,
) -> dict[str, Any]:
    """Lists files with two-tier memory & disk caching, filtering, sorting, and pagination."""
    limit = max(1, min(limit, MAX_PAGE_SIZE))
    page = max(1, page)
    chan_name, proto, base_tgt, raw_items = _fetch_or_cache_listing(source_id, profile_id, subdir, force)

    filtered = _filter_listing_items(raw_items, query_str)
    sorted_items = _sort_listing_items(filtered, sort_mode)

    total_items = len(raw_items)
    filtered_count = len(sorted_items)
    total_pages = max(1, (filtered_count + limit - 1) // limit)
    start_idx = (page - 1) * limit
    end_idx = start_idx + limit

    return {
        "status": "ok",
        "source_id": source_id,
        "profile_id": profile_id,
        "subdir": subdir,
        "channel_name": chan_name,
        "protocol": proto,
        "base_target": base_tgt,
        "total_items": total_items,
        "filtered_items": filtered_count,
        "page": page,
        "limit": limit,
        "total_pages": total_pages,
        "items": sorted_items[start_idx:end_idx],
    }


def _resolve_synology_endpoint(source_id: str, profile_id: str, clean_path: str) -> tuple[bool, bool, str, str, str]:
    """Resolves local CIFS or remote FTP endpoint for Synology media."""
    if profile_id == "cifs":
        sub = "video" if source_id == "synology-video" else "tvshow"
        return True, False, str(Path(_user_media_path("Synology", sub)) / clean_path), "", ""
    if profile_id in ("lan_ftp", "wan_ftp"):
        base_dir = "/video" if source_id == "synology-video" else "/tvshow"
        host = DEFAULT_NAS_LAN_HOST if profile_id == "lan_ftp" else DEFAULT_NAS_WAN_HOST
        return False, True, "", f"ftp://{host}:21{base_dir}/{clean_path}", resolve_nas_password()
    return False, False, "", "", ""


def _resolve_freebox_endpoint(profile_id: str, clean_path: str) -> tuple[bool, bool, str, str, str]:
    """Resolves local CIFS, LAN FTP, or HTTP range gateway for Freebox media."""
    if profile_id in ("disque1", "disque2"):
        sub = "Disque_1" if profile_id == "disque1" else "Disque_2"
        return True, False, str(Path(_user_media_path("Freebox", sub)) / clean_path), "", ""
    if profile_id == "lan_ftp":
        return False, True, "", f"ftp://{DEFAULT_FREEBOX_HOST}:21/{clean_path}", resolve_freebox_password()
    if profile_id in ("lan_http", "wan_http", "rpi4_gateway"):
        quoted_path = "/".join(urllib.parse.quote(seg) for seg in clean_path.split("/"))
        return False, False, "", f"http://{DEFAULT_RPI_LAN_HOST}:{DEFAULT_FREEBOX_GATEWAY_PORT}/{quoted_path}", ""
    return False, False, "", "", ""


def _resolve_target_endpoint(source_id: str, profile_id: str, clean_path: str) -> tuple[bool, bool, str, str, str]:
    """Resolves local vs remote target path, ftp url, and credentials."""
    if source_id == "local-snippets" and profile_id == "local":
        return True, False, str(Path("data/snippets") / clean_path), "", ""
    if source_id in ("synology-video", "synology-tvshow"):
        return _resolve_synology_endpoint(source_id, profile_id, clean_path)
    if source_id == "freebox-disks":
        return _resolve_freebox_endpoint(profile_id, clean_path)
    return False, False, "", "", ""


def _parse_probe_streams(
    probe_data: dict[str, Any],
) -> tuple[dict[str, Any] | None, list[dict[str, Any]], list[dict[str, Any]]]:
    """Extracts video, audio, and subtitle streams from ffprobe output."""
    video_stream: dict[str, Any] | None = None
    audio_streams: list[dict[str, Any]] = []
    sub_streams: list[dict[str, Any]] = []

    for s in probe_data.get("streams", []):
        stype = s.get("codec_type")
        if stype == "video" and video_stream is None:
            video_stream = {
                "codec": s.get("codec_name"),
                "codec_long": s.get("codec_long_name"),
                "profile": s.get("profile"),
                "width": s.get("width"),
                "height": s.get("height"),
                "aspect_ratio": s.get("display_aspect_ratio"),
                "fps": s.get("r_frame_rate"),
                "pix_fmt": s.get("pix_fmt"),
                "color_space": s.get("color_space"),
            }
        elif stype == "audio":
            audio_streams.append(
                {
                    "index": s.get("index"),
                    "codec": s.get("codec_name"),
                    "channels": s.get("channels"),
                    "channel_layout": s.get("channel_layout"),
                    "language": s.get("tags", {}).get("language", "und"),
                    "title": s.get("tags", {}).get("title", ""),
                }
            )
        elif stype == "subtitle":
            sub_streams.append(
                {
                    "index": s.get("index"),
                    "codec": s.get("codec_name"),
                    "language": s.get("tags", {}).get("language", "und"),
                    "title": s.get("tags", {}).get("title", ""),
                    "forced": s.get("disposition", {}).get("forced", 0) == 1,
                    "default": s.get("disposition", {}).get("default", 0) == 1,
                }
            )
    return video_stream, audio_streams, sub_streams


def _resolve_ffprobe_binary() -> str | None:
    """Finds ffprobe binary in standard PATH or deploy directories (~/dbsrt/bin, ~/.local/bin)."""
    found = shutil.which("ffprobe")
    if found:
        return found
    candidates = [
        Path.home() / "dbsrt/bin/ffprobe",
        Path.home() / ".local/bin/ffprobe",
        Path.home() / "bin/ffprobe",
        Path("/usr/local/bin/ffprobe"),
        Path("/usr/bin/ffprobe"),
    ]
    for c in candidates:
        if c.is_file() and os.access(c, os.X_OK):
            return str(c)
    return None


def _execute_ffprobe(
    is_local: bool, is_ftp: bool, abs_path: str, ftp_url: str, passwd: str, ffprobe_bin: str
) -> tuple[dict[str, Any] | None, str]:
    """Runs ffprobe command and returns parsed JSON or error message."""
    cmd: list[str] = []
    if is_local and Path(abs_path).is_file():
        cmd = [ffprobe_bin, "-v", "error", "-print_format", "json", "-show_format", "-show_streams", abs_path]
    elif is_ftp and passwd:
        ftp_user = DEFAULT_FREEBOX_USER if "mafreebox" in ftp_url else DEFAULT_NAS_USER
        cmd = [
            ffprobe_bin,
            "-v",
            "error",
            "-ftp-user",
            ftp_user,
            "-ftp-password",
            passwd,
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            ftp_url,
        ]
    elif ftp_url.startswith("http://"):
        cmd = [
            ffprobe_bin,
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            ftp_url,
        ]
    if not cmd:
        return None, "Cible non accessible pour analyse ffprobe"
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=FFPROBE_TIMEOUT_SECONDS)
        if proc.returncode == 0 and proc.stdout.strip():
            return json.loads(proc.stdout), ""
        err_raw = proc.stderr.strip()
        if "I/O error" in err_raw or "Permission denied" in err_raw or "550" in err_raw:
            return None, "Erreur E/S ou droits NAS insuffisants (FTP 550 Permission denied : fichier en chmod 600)"
        last_line = err_raw.splitlines()[-1] if err_raw else "Code retour non nul"
        return None, f"Échec analyse ffprobe: {last_line}"
    except subprocess.TimeoutExpired:
        return None, "Timeout analyse ffprobe (> 12s)"
    except Exception as err:
        return None, str(err)


def _get_cached_details(cache_key: str, force: bool) -> dict[str, Any] | None:
    """Checks tier-1 memory and tier-2 disk caches for file details."""
    now = time.time()
    if not force and cache_key in _DETAILS_CACHE and (now - _DETAILS_CACHE[cache_key][0] < CACHE_DETAILS_TTL_SECONDS):
        return _DETAILS_CACHE[cache_key][1]
    if not force:
        disk_hit = _read_disk_cache("details", cache_key, CACHE_DETAILS_TTL_SECONDS)
        if disk_hit is not None:
            ts, cached_details = disk_hit
            _DETAILS_CACHE[cache_key] = (ts, cached_details)
            return cached_details
    return None


def _populate_probe_format(details: dict[str, Any], probe_data: dict[str, Any]) -> None:
    """Populates container and stream properties from valid ffprobe output."""
    fmt = probe_data.get("format", {})
    dur = float(fmt.get("duration", 0.0))
    size = int(fmt.get("size", 0))
    bitrate = int(fmt.get("bit_rate", 0))

    details["probed"] = True
    details["duration_seconds"] = dur
    details["duration_formatted"] = format_seconds_duration(dur)
    if size > 0 and details["size_bytes"] == 0:
        details["size_bytes"] = size
        details["size_human"] = format_human_size(size)
    details["format_name"] = fmt.get("format_name", "")
    details["format_long_name"] = fmt.get("format_long_name", "")
    details["bit_rate"] = bitrate
    details["bit_rate_human"] = f"{bitrate / 1000000.0:.2f} Mb/s" if bitrate > 0 else "-"

    video_stream, audio_streams, sub_streams = _parse_probe_streams(probe_data)
    details["video"] = video_stream
    details["audio_streams"] = audio_streams
    details["subtitle_streams"] = sub_streams


def inspect_file_details(source_id: str, profile_id: str, file_path: str, force: bool = False) -> dict[str, Any]:
    """Inspects container file with two-tier memory & disk caching, ffprobe and Zero-DB couplings."""
    clean_path = file_path.strip("/")
    filename = Path(clean_path).name
    cache_key = f"{source_id}:{profile_id}:{clean_path}"
    cached = _get_cached_details(cache_key, force)
    if cached is not None:
        return cached

    is_local, is_ftp, abs_path, ftp_url, passwd = _resolve_target_endpoint(source_id, profile_id, clean_path)

    stat_size = 0
    stat_mod = ""
    if is_local and Path(abs_path).is_file():
        st = Path(abs_path).stat()
        stat_size = st.st_size
        stat_mod = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(st.st_mtime))

    now = time.time()
    details: dict[str, Any] = {
        "status": "ok",
        "name": filename,
        "path": clean_path,
        "extension": Path(filename).suffix.lower(),
        "size_bytes": stat_size,
        "size_human": format_human_size(stat_size) if stat_size > 0 else "-",
        "modify": stat_mod,
        "source_id": source_id,
        "profile_id": profile_id,
        "uri": abs_path if is_local else ftp_url,
        "parsed": parse_media_filename(filename),
        "coupling": _lookup_coupling(filename),
        "probed": False,
        "reason": "",
    }

    ffprobe_bin = _resolve_ffprobe_binary()
    if not ffprobe_bin:
        details["reason"] = "ffprobe non disponible sur cet hôte"
    else:
        probe_data, err_reason = _execute_ffprobe(is_local, is_ftp, abs_path, ftp_url, passwd, ffprobe_bin)
        if probe_data:
            _populate_probe_format(details, probe_data)
        else:
            details["reason"] = err_reason

    if details["probed"]:
        _DETAILS_CACHE[cache_key] = (now, details)
        _write_disk_cache("details", cache_key, details)
    return details


def _connect_pg() -> Any:
    """Safely connects to PostgreSQL (local Docker :5433 or remote cPanel :5432) or returns None."""
    # 1. Try environment URL if explicitly defined
    env_url = os.environ.get("DATABASE_URL")
    if env_url:
        try:
            import psycopg2

            return psycopg2.connect(env_url, connect_timeout=int(PG_CONNECT_TIMEOUT_SECS))
        except Exception:
            pass

    # 2. Try o2switch cPanel PostgreSQL credentials if present
    o2_pass_file = Path.home() / "dbsrt" / "etc" / ".pg_pass"
    if o2_pass_file.is_file():
        try:
            import psycopg2

            pw = o2_pass_file.read_text(encoding="utf-8").strip()
            return psycopg2.connect(
                dbname="atli6249_bktreedb",
                user="atli6249_dbsrt",
                password=pw,
                host="127.0.0.1",
                port=5432,
                connect_timeout=int(PG_CONNECT_TIMEOUT_SECS),
            )
        except Exception:
            pass

    # 3. Fallback to local Docker URL (:5433)
    try:
        import psycopg2

        return psycopg2.connect(DEFAULT_PG_URL, connect_timeout=int(PG_CONNECT_TIMEOUT_SECS))
    except Exception:
        return None


def _query_pg_cli(sql: str) -> str | None:
    """Executes SQL via psql CLI wrapper when psycopg2 is not installed in the python environment."""
    o2_pass_file = Path.home() / "dbsrt" / "etc" / ".pg_pass"
    if not o2_pass_file.is_file():
        return None
    try:
        pw = o2_pass_file.read_text(encoding="utf-8").strip()
        env = os.environ.copy()
        env["PGPASSWORD"] = pw
        cmd = [
            "psql",
            "-h",
            "127.0.0.1",
            "-p",
            "5432",
            "-U",
            "atli6249_dbsrt",
            "-d",
            "atli6249_bktreedb",
            "-t",
            "-A",
            "-c",
            sql,
        ]
        out = subprocess.check_output(cmd, env=env, timeout=10.0, stderr=subprocess.DEVNULL)
        return out.decode("utf-8").strip()
    except Exception:
        return None


def _detect_pg_version(conn: Any | None = None) -> str:
    """Detects active PostgreSQL server version dynamically."""
    if conn is not None:
        try:
            cur = conn.cursor()
            cur.execute("SHOW server_version;")
            row = cur.fetchone()
            if row:
                raw = str(row[0] if isinstance(row, tuple | list) else row.get("server_version", ""))
                major_minor = ".".join(raw.split()[0].split(".")[:2])
                return f"PostgreSQL {major_minor}"
        except Exception:
            pass
    raw_ver = _query_pg_cli("SHOW server_version;")
    if raw_ver:
        major_minor = ".".join(raw_ver.strip().split()[0].split(".")[:2])
        return f"PostgreSQL {major_minor}"
    return "PostgreSQL"


def _resolve_physical_file_exists(source_id: str | None, video_rel_path: str | None) -> tuple[bool, str]:
    """Checks whether video_rel_path exists physically on current host local mounts/directories."""
    if not video_rel_path:
        return False, ""

    clean_path = video_rel_path.lstrip("/")
    resolved_path: Path | None = None

    if source_id == "synology-video":
        resolved_path = Path(_user_media_path("Synology", "video")) / clean_path.replace("video/", "", 1)
    elif source_id == "synology-tvshow":
        resolved_path = Path(_user_media_path("Synology", "tvshow")) / clean_path.replace("tvshow/", "", 1)
    elif source_id == "local-snippets":
        resolved_path = Path(__file__).resolve().parents[1] / "data" / "snippets" / Path(clean_path).name
    else:
        # Fallback to direct path or known mount
        cand = Path("/" + clean_path)
        if cand.is_file():
            return True, str(cand)
        cand_user = Path(_user_media_path("Synology")) / clean_path
        if cand_user.is_file():
            return True, str(cand_user)
        resolved_path = Path(__file__).resolve().parents[1] / clean_path

    if resolved_path and resolved_path.is_file():
        return True, str(resolved_path)
    return False, str(resolved_path) if resolved_path else ""


def _check_phash_file_exists(moviehash: str | None, fps: float | None) -> bool:
    """Checks presence of .phash file in data/__PHASH__/ on disk."""
    if not moviehash:
        return False
    phash_dir = Path(__file__).resolve().parents[1] / "data" / "__PHASH__"
    fps_val = f"{fps:.1f}" if fps is not None else "25.0"
    cand1 = phash_dir / f"{moviehash}.{fps_val}fps.phash"
    if cand1.is_file():
        return True
    cand2 = phash_dir / f"{moviehash}.24.0fps.phash"
    if cand2.is_file():
        return True
    cand3 = phash_dir / f"{moviehash}.23.976fps.phash"
    return cand3.is_file()


ASSETS_SORT_MAP: dict[str, str] = {
    "title_asc": "clean_title ASC NULLS LAST, name ASC",
    "title_desc": "clean_title DESC NULLS LAST, name DESC",
    "frames_desc": "frame_count DESC, id DESC",
    "duration_desc": "duration_ms DESC, id DESC",
    "id_desc": "id DESC",
    "id_asc": "id ASC",
}


def _compute_asset_status_tag(phash_status: str | None, video_rel_path: str | None) -> str:
    """Computes nominal, missing_phash, or unmapped_path business tag."""
    if phash_status == "complete" and video_rel_path:
        return "nominal"
    if phash_status != "complete":
        return "missing_phash"
    return "unmapped_path"


def _get_asset_alt_channels(source_id: str | None, video_rel_path: str | None) -> list[dict[str, Any]]:
    """Builds remote streaming WAN FTP fallback profile when source is Synology."""
    if source_id in ("synology-video", "synology-tvshow") and video_rel_path:
        return [
            {
                "id": "wan_ftp",
                "name": "Streaming WAN FTP (Synology)",
                "accessible": True,
                "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_HOST}:21{video_rel_path}",
            }
        ]
    return []


def _format_asset_row(r: dict[str, Any]) -> dict[str, Any]:
    """Formats an asset database record into an enriched frontend descriptor."""
    phys_exists, phys_path = _resolve_physical_file_exists(r.get("source_id"), r.get("video_rel_path"))
    phash_on_disk = _check_phash_file_exists(r.get("moviehash"), r.get("fps"))
    status_tag = _compute_asset_status_tag(r.get("phash_status"), r.get("video_rel_path"))
    alt_channels = _get_asset_alt_channels(r.get("source_id"), r.get("video_rel_path"))

    codec_raw = r.get("video_codec") or "—"
    return {
        "id": r["id"],
        "name": r["name"],
        "clean_title": r.get("clean_title") or r["name"],
        "season": r.get("season"),
        "episode": r.get("episode"),
        "year": r.get("year"),
        "resolution": r.get("resolution") or "—",
        "video_codec": codec_raw.upper(),
        "fps": r.get("fps") or 25.0,
        "is_vfr": r.get("is_vfr") or False,
        "frame_count": r.get("frame_count") or 0,
        "expected_frames": r.get("expected_frames") or 0,
        "duration_ms": r.get("duration_ms") or 0,
        "duration_fmt": _format_duration(r.get("duration_ms")),
        "file_size_bytes": r.get("file_size_bytes") or 0,
        "file_size_human": format_human_size(r.get("file_size_bytes") or 0),
        "moviehash": r.get("moviehash") or "",
        "media_hash": r.get("media_hash") or "",
        "source_id": r.get("source_id") or "",
        "video_rel_path": r.get("video_rel_path") or "",
        "physical_exists": phys_exists,
        "physical_resolved_path": phys_path,
        "phash_status": r.get("phash_status"),
        "phash_on_disk": phash_on_disk,
        "status_tag": status_tag,
        "alt_channels": alt_channels,
    }


def _build_assets_where(
    status_filter: str, source_filter: str, search_q: str, is_psycopg: bool
) -> tuple[str, list[Any]]:
    """Builds SQL WHERE clause and parameters for assets queries."""
    where_clauses: list[str] = []
    params: list[Any] = []
    if status_filter == "nominal":
        where_clauses.append("phash_status = 'complete' AND video_rel_path IS NOT NULL AND video_rel_path != ''")
    elif status_filter == "missing_phash":
        where_clauses.append("phash_status != 'complete'")
    elif status_filter == "unmapped_path":
        where_clauses.append("(video_rel_path IS NULL OR video_rel_path = '')")

    if source_filter:
        if is_psycopg:
            where_clauses.append("source_id = %s")
            params.append(source_filter)
        else:
            safe_src = re.sub(r"[^a-zA-Z0-9_-]", "", source_filter)
            where_clauses.append(f"source_id = '{safe_src}'")

    if search_q:
        q_norm = strip_accents(search_q.strip()).lower()
        title_sql = f"translate(lower(clean_title), '{ACCENTED_CHARS}', '{UNACCENTED_CHARS}')"
        name_sql = f"translate(lower(name), '{ACCENTED_CHARS}', '{UNACCENTED_CHARS}')"
        if is_psycopg:
            clean_q = f"%{q_norm}%"
            where_clauses.append(f"({title_sql} LIKE %s OR {name_sql} LIKE %s OR moviehash ILIKE %s)")
            params.extend([clean_q, clean_q, f"%{search_q.strip()}%"])
        else:
            safe_q = re.sub(r"['\";\\]", "", q_norm)
            safe_raw = re.sub(r"['\";\\]", "", search_q.strip())
            where_clauses.append(
                f"({title_sql} LIKE '%{safe_q}%' OR {name_sql} LIKE '%{safe_q}%' OR moviehash ILIKE '%{safe_raw}%')"
            )

    where_sql = ("WHERE " + " AND ".join(where_clauses)) if where_clauses else ""
    return where_sql, params


def _get_pg_assets_cli(
    page: int,
    limit: int,
    status_filter: str,
    source_filter: str,
    search_q: str,
    sort_by: str,
) -> dict[str, Any]:
    """Queries assets using native psql CLI (fallback when psycopg2 is absent)."""
    cli_summary_raw = _query_pg_cli("""
        SELECT json_build_object(
            'total', count(*),
            'nominal', count(CASE WHEN phash_status = 'complete' AND video_rel_path IS NOT NULL AND video_rel_path != '' THEN 1 END),
            'missing_phash', count(CASE WHEN phash_status != 'complete' THEN 1 END),
            'unmapped_path', count(CASE WHEN video_rel_path IS NULL OR video_rel_path = '' THEN 1 END)
        )::text FROM medias;
    """)
    if not cli_summary_raw:
        return {
            "status": "error",
            "message": "PostgreSQL non disponible (:5433 / :5432). Vérifiez 'task db:up'.",
            "connected": False,
            "summary": {"total": 0, "nominal": 0, "missing_phash": 0, "unmapped_path": 0},
            "page": page,
            "limit": limit,
            "total_items": 0,
            "total_pages": 0,
            "items": [],
        }

    try:
        summary = json.loads(cli_summary_raw)
        where_sql, _ = _build_assets_where(status_filter, source_filter, search_q, is_psycopg=False)

        count_raw = _query_pg_cli(f"SELECT count(*) FROM medias {where_sql};")
        total_items = int(count_raw) if count_raw and count_raw.isdigit() else 0

        total_pages = (total_items + limit - 1) // limit if total_items > 0 else 1
        if page > total_pages and total_items > 0:
            page = 1
        offset = max(0, (page - 1) * limit)
        order_sql = ASSETS_SORT_MAP.get(sort_by, "clean_title ASC NULLS LAST, name ASC")

        data_raw = _query_pg_cli(f"""
            SELECT json_agg(t)::text FROM (
                SELECT
                    id, name, clean_title, season, episode, year, resolution, video_codec,
                    fps, frame_count, expected_frames, is_vfr, duration_ms, file_size_bytes,
                    moviehash, media_hash, video_rel_path, source_id, phash_status
                FROM medias
                {where_sql}
                ORDER BY {order_sql}
                LIMIT {limit} OFFSET {offset}
            ) t;
        """)
        rows = json.loads(data_raw) if data_raw and data_raw != "null" else []
        items = [_format_asset_row(r) for r in rows]

        return {
            "status": "ok",
            "connected": True,
            "pg_version": _detect_pg_version(),
            "summary": summary,
            "page": page,
            "limit": limit,
            "total_items": total_items,
            "total_pages": total_pages,
            "items": items,
        }
    except Exception as exc:
        return {
            "status": "error",
            "message": str(exc),
            "connected": False,
            "summary": {"total": 0, "nominal": 0, "missing_phash": 0, "unmapped_path": 0},
            "page": page,
            "limit": limit,
            "total_items": 0,
            "total_pages": 0,
            "items": [],
        }


def _get_pg_assets_psycopg2(
    conn: Any,
    page: int,
    limit: int,
    status_filter: str,
    source_filter: str,
    search_q: str,
    sort_by: str,
) -> dict[str, Any]:
    """Queries assets using native psycopg2 connection."""
    from psycopg2.extras import RealDictCursor

    try:
        cur = conn.cursor(cursor_factory=RealDictCursor)
        cur.execute("""
            SELECT
                count(*) AS total,
                count(CASE WHEN phash_status = 'complete' AND video_rel_path IS NOT NULL AND video_rel_path != '' THEN 1 END) AS nominal,
                count(CASE WHEN phash_status != 'complete' THEN 1 END) AS missing_phash,
                count(CASE WHEN video_rel_path IS NULL OR video_rel_path = '' THEN 1 END) AS unmapped_path
            FROM medias;
        """)
        srow = cur.fetchone()
        summary = {
            "total": srow["total"] if srow else 0,
            "nominal": srow["nominal"] if srow else 0,
            "missing_phash": srow["missing_phash"] if srow else 0,
            "unmapped_path": srow["unmapped_path"] if srow else 0,
        }

        where_sql, params = _build_assets_where(status_filter, source_filter, search_q, is_psycopg=True)
        count_query = f"SELECT count(*) FROM medias {where_sql};"
        cur.execute(count_query, params)
        total_items = cur.fetchone()["count"]

        total_pages = (total_items + limit - 1) // limit if total_items > 0 else 1
        if page > total_pages and total_items > 0:
            page = 1
        offset = max(0, (page - 1) * limit)
        order_sql = ASSETS_SORT_MAP.get(sort_by, "clean_title ASC NULLS LAST, name ASC")

        data_query = f"""
            SELECT
                id, name, clean_title, season, episode, year, resolution, video_codec,
                fps, frame_count, expected_frames, is_vfr, duration_ms, file_size_bytes,
                moviehash, media_hash, video_rel_path, source_id, phash_status
            FROM medias
            {where_sql}
            ORDER BY {order_sql}
            LIMIT %s OFFSET %s;
        """
        cur.execute(data_query, params + [limit, offset])
        rows = cur.fetchall()
        items = [_format_asset_row(r) for r in rows]

        return {
            "status": "ok",
            "connected": True,
            "pg_version": _detect_pg_version(conn),
            "summary": summary,
            "page": page,
            "limit": limit,
            "total_items": total_items,
            "total_pages": total_pages,
            "items": items,
        }
    except Exception as exc:
        return {
            "status": "error",
            "message": str(exc),
            "connected": False,
            "summary": {"total": 0, "nominal": 0, "missing_phash": 0, "unmapped_path": 0},
            "page": page,
            "limit": limit,
            "total_items": 0,
            "total_pages": 0,
            "items": [],
        }
    finally:
        conn.close()


def get_pg_assets(
    page: int = 1,
    limit: int = DEFAULT_PAGE_SIZE,
    status_filter: str = "all",
    source_filter: str = "",
    search_q: str = "",
    sort_by: str = "title_asc",
) -> dict[str, Any]:
    """Queries PostgreSQL medias table, computes physical & pHash health metrics."""
    conn = _connect_pg()
    if conn is None:
        return _get_pg_assets_cli(page, limit, status_filter, source_filter, search_q, sort_by)
    return _get_pg_assets_psycopg2(conn, page, limit, status_filter, source_filter, search_q, sort_by)


def _format_duration(duration_ms: int | None) -> str:
    """Formats milliseconds into hh:mm:ss string."""
    if not duration_ms or duration_ms <= 0:
        return "—"
    total_secs = int(duration_ms / MILLIS_PER_SEC)
    h = total_secs // SECONDS_PER_HOUR
    m = (total_secs % SECONDS_PER_HOUR) // SECONDS_PER_MINUTE
    s = total_secs % SECONDS_PER_MINUTE
    if h > 0:
        return f"{h:02d}:{m:02d}:{s:02d}"
    return f"{m:02d}:{s:02d}"


def _format_asset_detail(row: dict[str, Any], frames_in_db: int = 0) -> dict[str, Any]:
    """Enriches raw media row with formatted metrics and storage audit."""
    phys_exists, phys_path = _resolve_physical_file_exists(row.get("source_id"), row.get("video_rel_path"))
    phash_on_disk = _check_phash_file_exists(row.get("moviehash"), row.get("fps"))
    alt_channels = _get_asset_alt_channels(row.get("source_id"), row.get("video_rel_path"))

    res = dict(row)
    res["status"] = "ok"
    res["frames_in_db"] = frames_in_db
    res["physical_exists"] = phys_exists
    res["physical_resolved_path"] = phys_path
    res["phash_on_disk"] = phash_on_disk
    res["duration_fmt"] = _format_duration(row.get("duration_ms"))
    res["file_size_human"] = format_human_size(row.get("file_size_bytes") or 0)
    res["created_at_fmt"] = str(row.get("created_at") or "—")[:19]
    res["updated_at_fmt"] = str(row.get("updated_at") or "—")[:19]
    res["alt_channels"] = alt_channels
    return res


def _get_pg_asset_detail_cli(media_id: int) -> dict[str, Any]:
    """Queries media detail using psql CLI fallback."""
    cli_detail_raw = _query_pg_cli(f"""
        SELECT json_build_object(
            'id', id, 'name', name, 'clean_title', clean_title,
            'season', season, 'episode', episode, 'year', year,
            'resolution', resolution, 'video_codec', video_codec,
            'width', width, 'height', height, 'fps', fps,
            'frame_count', frame_count, 'expected_frames', expected_frames,
            'is_vfr', is_vfr, 'duration_ms', duration_ms,
            'file_size_bytes', file_size_bytes, 'moviehash', moviehash,
            'media_hash', media_hash, 'video_rel_path', video_rel_path,
            'source_id', source_id, 'phash_status', phash_status,
            'phash_path', phash_path, 'asset_kind', asset_kind,
            'parent_media_id', parent_media_id, 'source_mtime_unix', source_mtime_unix,
            'created_at', created_at, 'updated_at', updated_at
        )::text FROM medias WHERE id = {media_id};
    """)
    if not cli_detail_raw:
        return {"status": "error", "message": "PostgreSQL non disponible"}
    try:
        row = json.loads(cli_detail_raw)
        return _format_asset_detail(row, frames_in_db=0)
    except Exception as exc:
        return {"status": "error", "message": str(exc)}


def _get_pg_asset_detail_psycopg2(conn: Any, media_id: int) -> dict[str, Any]:
    """Queries media detail using psycopg2 connection."""
    from psycopg2.extras import RealDictCursor

    try:
        cur = conn.cursor(cursor_factory=RealDictCursor)
        cur.execute(
            """
            SELECT
                id, name, clean_title, season, episode, year, resolution, video_codec,
                width, height, fps, frame_count, expected_frames, is_vfr, duration_ms,
                file_size_bytes, moviehash, media_hash, video_rel_path, source_id,
                phash_status, phash_path, asset_kind, parent_media_id, source_mtime_unix,
                created_at, updated_at
            FROM medias
            WHERE id = %s;
        """,
            (media_id,),
        )
        row = cur.fetchone()
        if not row:
            return {"status": "not_found", "message": f"Média #{media_id} introuvable"}

        cur.execute("SELECT count(*) AS frames_in_db FROM frames WHERE media_id = %s;", (media_id,))
        frames_db_cnt = cur.fetchone()["frames_in_db"]
        return _format_asset_detail(dict(row), frames_in_db=frames_db_cnt)
    except Exception as exc:
        return {"status": "error", "message": str(exc)}
    finally:
        conn.close()


def get_pg_asset_detail(media_id: int) -> dict[str, Any]:
    """Returns deep technical details for single media asset from PostgreSQL."""
    conn = _connect_pg()
    if conn is None:
        return _get_pg_asset_detail_cli(media_id)
    return _get_pg_asset_detail_psycopg2(conn, media_id)


@dataclass
class PhashReconciliationContext:
    """Context holding sidecars and PostgreSQL lookups for fast tripartite alignment."""

    sidecars: dict[str, dict[str, Any]] = field(default_factory=dict)
    pg_medias: dict[str, dict[str, Any]] = field(default_factory=dict)


@dataclass
class PhashesQueryCriteria:
    """Query criteria for filtering, sorting and paginating pHash tripartite items."""

    page: int = 1
    limit: int = DEFAULT_PAGE_SIZE
    status_filter: str = "all"
    search_q: str = ""
    sort_by: str = "id_asc"


def _load_phash_sidecars(phash_dir: Path) -> dict[str, dict[str, Any]]:
    """Loads all *.json sidecars in __PHASH__ directory into dictionary indexed by stem."""
    sidecars: dict[str, dict[str, Any]] = {}
    if not phash_dir.is_dir():
        return sidecars
    for p in phash_dir.glob("*.json"):
        try:
            sidecars[p.stem.lower()] = json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            pass
    return sidecars


def _read_catalog_dbphash(cat_path: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Reads fixed 128-byte header and media table entries from catalog.dbphash."""
    if not cat_path.is_file():
        return {}, []

    with open(cat_path, "rb") as f:
        hdr_raw = f.read(CATALOG_DBPHASH_HEADER_SIZE)
        if len(hdr_raw) < CATALOG_DBPHASH_HEADER_SIZE:
            return {}, []
        unpacked_hdr = struct.unpack(CATALOG_DBPHASH_HEADER_FORMAT, hdr_raw)
        (
            magic,
            ver,
            _flags,
            media_cnt,
            total_cuts,
            total_sparse,
            off_media,
            _off_cuts,
            _off_sparse,
            off_str,
            str_len,
            _csum,
            _,
        ) = unpacked_hdr

        if not magic.startswith(b"DBSRT"):
            return {}, []

        f.seek(off_str)
        string_table = f.read(str_len)

        f.seek(off_media)
        catalog_items: list[dict[str, Any]] = []
        for _ in range(media_cnt):
            entry_raw = f.read(CATALOG_DBPHASH_ENTRY_SIZE)
            if len(entry_raw) < CATALOG_DBPHASH_ENTRY_SIZE:
                break
            mid, _mtype, has_fc, yr, s, ep, dur, fps_x1000, toff, tlen, cc, _c_idx, sc, _s_idx, fc = struct.unpack(
                CATALOG_DBPHASH_ENTRY_FORMAT, entry_raw
            )
            title = string_table[toff : toff + tlen].decode("utf-8", errors="replace")
            catalog_items.append(
                {
                    "catalog_id": mid,
                    "title": title,
                    "year": yr if yr > 0 else None,
                    "season": s if s > 0 else None,
                    "episode": ep if ep > 0 else None,
                    "duration_ms": dur,
                    "fps": fps_x1000 / 1000.0,
                    "cuts_count": cc,
                    "sparse_count": sc,
                    "frame_count": fc if has_fc else 0,
                }
            )

        header_info = {
            "version": ver,
            "media_count": media_cnt,
            "total_cuts": total_cuts,
            "total_sparse": total_sparse,
            "file_size": cat_path.stat().st_size,
        }
        return header_info, catalog_items


def _index_media_record(r: dict[str, Any], medias: dict[str, dict[str, Any]]) -> None:
    """Indexes single media record by moviehash, phash_path basename, and name."""
    mhash = r.get("moviehash")
    if mhash:
        medias[mhash.lower()] = r
    ppath = r.get("phash_path")
    if ppath:
        medias[Path(ppath).name.lower()] = r
    name = r.get("name")
    if name:
        medias[name.lower()] = r


def _query_pg_medias_psycopg2(conn: Any) -> tuple[dict[str, dict[str, Any]], int, int]:
    """Queries medias and frames using psycopg2 connection."""
    from psycopg2.extras import RealDictCursor

    medias: dict[str, dict[str, Any]] = {}
    total_pg_medias = 0
    frames_table_count = 0
    try:
        cur = conn.cursor(cursor_factory=RealDictCursor)
        cur.execute("SELECT count(*) AS total FROM medias;")
        total_pg_medias = cur.fetchone()["total"]

        try:
            cur.execute("SELECT count(*) AS fc FROM frames;")
            frames_table_count = cur.fetchone()["fc"]
        except Exception:
            pass

        cur.execute("SELECT id, name, clean_title, moviehash, phash_path, frame_count, fps, phash_status FROM medias;")
        for row in cur.fetchall():
            _index_media_record(dict(row), medias)
    except Exception:
        pass
    finally:
        conn.close()
    return medias, total_pg_medias, frames_table_count


def _query_pg_medias_cli() -> tuple[dict[str, dict[str, Any]], int, int]:
    """Queries medias and frames using psql CLI fallback."""
    medias: dict[str, dict[str, Any]] = {}
    total_pg_medias = 0
    frames_table_count = 0

    raw_cnt = _query_pg_cli("SELECT count(*) FROM medias;")
    if raw_cnt and raw_cnt.isdigit():
        total_pg_medias = int(raw_cnt)

    has_frames = _query_pg_cli(
        "SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'frames';"
    )
    if has_frames:
        raw_fc = _query_pg_cli("SELECT count(*) FROM frames;")
        if raw_fc and raw_fc.isdigit():
            frames_table_count = int(raw_fc)

    raw_json = _query_pg_cli(
        "SELECT json_agg(t)::text FROM (SELECT id, name, clean_title, moviehash, phash_path, frame_count, fps, phash_status FROM medias) t;"
    )
    if raw_json and raw_json != "null":
        try:
            for r in json.loads(raw_json):
                _index_media_record(r, medias)
        except Exception:
            pass
    return medias, total_pg_medias, frames_table_count


def _query_pg_medias_lookup() -> tuple[dict[str, dict[str, Any]], int, int]:
    """Loads media records and frame counts from PostgreSQL for tripartite alignment."""
    conn = _connect_pg()
    if conn is not None:
        return _query_pg_medias_psycopg2(conn)
    return _query_pg_medias_cli()


def _resolve_phash_moviehash(filename: str, sidecars: dict[str, dict[str, Any]]) -> str | None:
    """Extracts 16-hex moviehash from filename or sidecars."""
    stem = filename.split(".")[0]
    if len(stem) == 16 and all(c in "0123456789abcdefABCDEF" for c in stem):
        return stem.lower()
    for mhash, sc in sidecars.items():
        mname = sc.get("media_name", "")
        if mname and (mname in filename or filename in mname):
            return mhash.lower()
    return None


def _lookup_pg_record(mhash: str | None, raw_fname: str, pg_medias: dict[str, dict[str, Any]]) -> dict[str, Any] | None:
    """Finds matching PostgreSQL media record by moviehash or filename."""
    if mhash and mhash in pg_medias:
        return pg_medias[mhash]
    if raw_fname and raw_fname.lower() in pg_medias:
        return pg_medias[raw_fname.lower()]
    return None


def _compute_phash_integrity_status(has_raw: bool, has_catalog: bool, has_pg: bool) -> tuple[str, str]:
    """Computes integrity status code and human label based on tripartite presence."""
    if has_raw and has_catalog and has_pg:
        return "aligned_3_3", "Aligné 3/3"
    if has_raw and has_catalog:
        return "missing_pg", "Non ingéré PG"
    if has_raw:
        return "uncompiled", "À compiler"
    return "orphan_catalog", "Orphelin catalogue"


def _extract_raw_file_specs(raw_p: Path | None) -> tuple[str, int, int]:
    """Extracts filename, size in bytes, and frame count from optional raw Path."""
    if not raw_p:
        return "", 0, 0
    sz = raw_p.stat().st_size
    return raw_p.name, sz, sz // PHASH_FRAME_BYTES


def _compute_catalog_specs(
    cat_e: dict[str, Any] | None, raw_fc: int, default_title: str
) -> tuple[str, float, int, int, int, int]:
    """Extracts title, fps, frame count, duration in ms, cuts and sparse counts from catalog entry."""
    if not cat_e:
        return default_title or "—", DEFAULT_BASELINE_FPS, raw_fc, int(raw_fc / DEFAULT_BASELINE_FPS * 1000.0), 0, 0
    fps = cat_e["fps"]
    fc = cat_e["frame_count"] if cat_e["frame_count"] > 0 else raw_fc
    dur_ms = cat_e["duration_ms"] if cat_e["duration_ms"] > 0 else int(fc / fps * 1000.0)
    return cat_e["title"], fps, fc, dur_ms, cat_e["cuts_count"], cat_e["sparse_count"]


def _calculate_compression_ratio(raw_size: int, cuts_c: int, sparse_c: int) -> str:
    """Calculates compression ratio multiplier between raw pHash frames and index entries."""
    indexed_bytes = (cuts_c + sparse_c) * 16
    if indexed_bytes > 0 and raw_size > 0:
        return f"{raw_size / indexed_bytes:.1f}x"
    return "—"


def _build_reconciled_phash_item(
    raw_p: Path | None,
    cat_e: dict[str, Any] | None,
    idx: int,
    ctx: PhashReconciliationContext,
) -> dict[str, Any]:
    """Builds a single reconciled tripartite item descriptor from raw file, catalog entry, and PG context."""
    raw_fname, raw_size, raw_fc = _extract_raw_file_specs(raw_p)
    mhash = _resolve_phash_moviehash(raw_fname, ctx.sidecars) if raw_fname else None
    pg_record = _lookup_pg_record(mhash, raw_fname, ctx.pg_medias)

    title, fps, fc, dur_ms, cuts_c, sparse_c = _compute_catalog_specs(cat_e, raw_fc, raw_fname)
    status_code, status_label = _compute_phash_integrity_status(
        raw_p is not None, cat_e is not None, pg_record is not None
    )
    comp_ratio = _calculate_compression_ratio(raw_size, cuts_c, sparse_c)

    return {
        "catalog_id": cat_e["catalog_id"] if cat_e else idx + 1,
        "title": title,
        "year": cat_e.get("year") if cat_e else None,
        "moviehash": mhash or "—",
        "raw_filename": raw_fname,
        "fps": fps,
        "frame_count": fc,
        "duration_ms": dur_ms,
        "duration_fmt": _format_duration(dur_ms),
        "raw_present": raw_p is not None,
        "raw_size_bytes": raw_size,
        "raw_size_human": format_human_size(raw_size),
        "catalog_present": cat_e is not None,
        "catalog_cuts_count": cuts_c,
        "catalog_sparse_count": sparse_c,
        "catalog_compression_ratio": comp_ratio,
        "pg_present": pg_record is not None,
        "pg_media_id": pg_record.get("id") if pg_record else None,
        "pg_status": pg_record.get("phash_status") if pg_record else None,
        "integrity_status": status_code,
        "integrity_label": status_label,
    }


def get_phash_tripartite_summary() -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Gathers and caches the 3 pHash pillars: RAW Store, Catalog Zero-DB, BKtreeDB."""
    global _PHASHES_AUDIT_CACHE
    now = time.time()
    with _CACHE_LOCK:
        if _PHASHES_AUDIT_CACHE and (now - _PHASHES_AUDIT_CACHE[0]) < PHASHES_CACHE_TTL_SECONDS:
            return _PHASHES_AUDIT_CACHE[1], _PHASHES_AUDIT_CACHE[2]

    phash_dir = Path(__file__).resolve().parents[1] / "data" / "__PHASH__"
    cat_path = Path(__file__).resolve().parents[1] / "data" / "catalog.dbphash"

    raw_files = sorted(list(phash_dir.glob("*.phash")), key=lambda p: p.name) if phash_dir.is_dir() else []
    sidecars = _load_phash_sidecars(phash_dir)
    hdr, cat_items = _read_catalog_dbphash(cat_path)
    pg_medias, total_pg_medias, frames_table_count = _query_pg_medias_lookup()

    ctx = PhashReconciliationContext(sidecars=sidecars, pg_medias=pg_medias)
    total_raw_bytes = sum(p.stat().st_size for p in raw_files)
    catalog_bytes = hdr.get("file_size", 0)

    reconciled: list[dict[str, Any]] = []
    aligned_3_3_count = 0
    missing_pg_count = 0

    max_len = max(len(raw_files), len(cat_items))
    for idx in range(max_len):
        raw_p = raw_files[idx] if idx < len(raw_files) else None
        cat_e = cat_items[idx] if idx < len(cat_items) else None
        item = _build_reconciled_phash_item(raw_p, cat_e, idx, ctx)
        if item["integrity_status"] == "aligned_3_3":
            aligned_3_3_count += 1
        elif item["integrity_status"] == "missing_pg":
            missing_pg_count += 1
        reconciled.append(item)

    summary = {
        "raw_count": len(raw_files),
        "raw_size_bytes": total_raw_bytes,
        "raw_size_human": format_human_size(total_raw_bytes),
        "catalog_count": len(cat_items),
        "catalog_size_bytes": catalog_bytes,
        "catalog_size_human": format_human_size(catalog_bytes),
        "catalog_cuts_total": hdr.get("total_cuts", 0),
        "catalog_sparse_total": hdr.get("total_sparse", 0),
        "pg_count": total_pg_medias,
        "pg_frames_total": frames_table_count,
        "aligned_3_3": aligned_3_3_count,
        "missing_pg": missing_pg_count,
        "uncompiled": 0,
        "orphan_catalog": 0,
    }

    with _CACHE_LOCK:
        _PHASHES_AUDIT_CACHE = (now, reconciled, summary)
    return reconciled, summary


PHASHES_SORT_MAP: dict[str, tuple[Any, bool]] = {
    "id_asc": (lambda x: x["catalog_id"], False),
    "id_desc": (lambda x: x["catalog_id"], True),
    "title_asc": (lambda x: x["title"].lower(), False),
    "title_desc": (lambda x: x["title"].lower(), True),
    "frames_desc": (lambda x: x["frame_count"], True),
    "frames_asc": (lambda x: x["frame_count"], False),
    "size_desc": (lambda x: x["raw_size_bytes"], True),
    "size_asc": (lambda x: x["raw_size_bytes"], False),
    "cuts_desc": (lambda x: x["catalog_cuts_count"], True),
    "cuts_asc": (lambda x: x["catalog_cuts_count"], False),
}


def query_phashes_list(criteria: PhashesQueryCriteria) -> dict[str, Any]:
    """Filters, sorts and paginates reconciled tripartite pHash fingerprints."""
    items, summary = get_phash_tripartite_summary()

    filtered = items
    if criteria.status_filter in ("aligned_3_3", "missing_pg", "uncompiled", "orphan_catalog"):
        filtered = [x for x in filtered if x["integrity_status"] == criteria.status_filter]

    if criteria.search_q:
        q_norm = strip_accents(criteria.search_q.strip()).lower()
        filtered = [
            x
            for x in filtered
            if q_norm in strip_accents(x["title"]).lower()
            or q_norm in strip_accents(x["raw_filename"]).lower()
            or q_norm in x["moviehash"].lower()
        ]

    key_func, rev = PHASHES_SORT_MAP.get(criteria.sort_by, (lambda x: x["catalog_id"], False))
    sorted_items = sorted(filtered, key=key_func, reverse=rev)

    total_items = len(sorted_items)
    limit = max(1, min(criteria.limit, MAX_PAGE_SIZE))
    total_pages = (total_items + limit - 1) // limit if total_items > 0 else 1
    page = 1 if criteria.page > total_pages and total_items > 0 else max(1, criteria.page)
    offset = (page - 1) * limit
    page_items = sorted_items[offset : offset + limit]

    return {
        "status": "ok",
        "summary": summary,
        "page": page,
        "limit": limit,
        "total_items": total_items,
        "total_pages": total_pages,
        "items": page_items,
    }


def _resolve_target_phash_file(identifier: str) -> Path | None:
    """Locates a .phash file in __PHASH__ directory by exact filename, decoded name, or moviehash prefix."""
    if not identifier:
        return None
    phash_dir = (Path(__file__).resolve().parents[1] / "data" / "__PHASH__").resolve()
    base = Path(identifier).name
    candidates_names = [
        base,
        urllib.parse.unquote(base),
        urllib.parse.quote(base),
        urllib.parse.quote(urllib.parse.unquote(base)),
    ]
    for cname in candidates_names:
        clean = re.sub(r"[^a-zA-Z0-9_.% -]", "", cname)
        if not clean or ".." in clean:
            continue
        candidate = (phash_dir / clean).resolve()
        if phash_dir in candidate.parents and candidate.is_file():
            return candidate

    prefix = re.sub(r"[^a-zA-Z0-9_-]", "", base.split(".")[0])
    if prefix:
        matches = list(phash_dir.glob(f"{prefix}*.phash"))
        if matches and phash_dir in matches[0].resolve().parents and matches[0].is_file():
            return matches[0]
    return None


def _compute_entropy_and_mobility(
    hashes: tuple[int, ...],
) -> tuple[float, float, list[int], list[int], float]:
    """Computes popcounts, DCT entropy histogram, consecutive delta mobility, and mean delta."""
    popcounts: list[int] = []
    distribution = [0] * ENTROPY_HISTOGRAM_BUCKETS
    delta_distribution = [0] * DELTA_MOBILITY_BUCKETS
    total_delta = 0

    for i, h in enumerate(hashes):
        pop = h.bit_count()
        popcounts.append(pop)
        bucket_idx = min(pop // HISTOGRAM_BUCKET_WIDTH, ENTROPY_HISTOGRAM_BUCKETS - 1)
        distribution[bucket_idx] += 1

        if i > 0:
            delta = (h ^ hashes[i - 1]).bit_count()
            total_delta += delta
            d_bucket = min(delta // DELTA_STEP_BITS, DELTA_MOBILITY_BUCKETS - 1)
            delta_distribution[d_bucket] += 1

    mean_pop = sum(popcounts) / len(popcounts) if popcounts else 0.0
    entropy_pct = round(mean_pop / float(BITS_PER_HASH) * 100.0, 1)
    mean_delta = (total_delta / float(len(hashes) - 1)) if len(hashes) > 1 else 0.0
    return mean_pop, entropy_pct, distribution, delta_distribution, mean_delta


def _detect_cuts(hashes: tuple[int, ...], fps: float) -> tuple[list[dict[str, Any]], int]:
    """Scans perceptual hashes for scene cuts exceeding the Hamming threshold."""
    cuts: list[dict[str, Any]] = []
    total_cuts_count = 0
    for i in range(1, len(hashes)):
        delta = (hashes[i] ^ hashes[i - 1]).bit_count()
        if delta >= PHASH_CUT_HAMMING_THRESHOLD:
            total_cuts_count += 1
            cuts.append(
                {
                    "frame": i,
                    "time_ms": int(i / fps * 1000.0),
                    "time_fmt": _format_duration(int(i / fps * 1000.0)),
                    "distance": delta,
                    "hash": f"{hashes[i]:016x}",
                }
            )

    if len(cuts) > MICROSCOPIC_CUTS_SAMPLE_LIMIT:
        step = len(cuts) / float(MICROSCOPIC_CUTS_SAMPLE_LIMIT)
        sampled_cuts = [cuts[int(idx * step)] for idx in range(MICROSCOPIC_CUTS_SAMPLE_LIMIT)]
    else:
        sampled_cuts = cuts
    return sampled_cuts, total_cuts_count


def _aggregate_timeline(hashes: tuple[int, ...], fps: float) -> list[dict[str, Any]]:
    """Builds downsampled timeline points measuring mean mobility and peak cuts."""
    total_frames = len(hashes)
    bucket_size = max(1, total_frames // TIMELINE_SERIES_POINTS)
    timeline: list[dict[str, Any]] = []
    cur_bucket_sum = 0
    cur_bucket_count = 0
    cur_bucket_max = 0

    for i in range(1, total_frames):
        delta = (hashes[i] ^ hashes[i - 1]).bit_count()
        cur_bucket_sum += delta
        cur_bucket_count += 1
        if delta > cur_bucket_max:
            cur_bucket_max = delta

        is_last = i % bucket_size == 0 or i == total_frames - 1
        if is_last:
            t_ms = int(i / fps * 1000.0)
            mean_d = round(cur_bucket_sum / float(cur_bucket_count), 2) if cur_bucket_count > 0 else 0.0
            timeline.append(
                {
                    "time_fmt": _format_duration(t_ms),
                    "frame": i,
                    "delta": mean_d,
                    "delta_mean": mean_d,
                    "delta_max": cur_bucket_max,
                }
            )
            cur_bucket_sum = 0
            cur_bucket_count = 0
            cur_bucket_max = 0

    return timeline


def _extract_cuts_and_timeline(
    hashes: tuple[int, ...], fps: float
) -> tuple[list[dict[str, Any]], int, list[dict[str, Any]]]:
    """Extracts sampled cuts, total cuts count, and timeline mobility series."""
    sampled_cuts, total_cuts = _detect_cuts(hashes, fps)
    timeline = _aggregate_timeline(hashes, fps)
    return sampled_cuts, total_cuts, timeline


def _detect_phash_cuts_and_entropy(
    hashes: tuple[int, ...], fps: float
) -> tuple[float, float, list[int], list[dict[str, Any]], int, list[dict[str, Any]], list[int], float]:
    """Scans perceptual hashes, detecting scene cuts, visual entropy, timeline, and delta mobility."""
    mean_pop, entropy_pct, distribution, delta_dist, mean_delta = _compute_entropy_and_mobility(hashes)
    sampled_cuts, total_cuts_count, timeline = _extract_cuts_and_timeline(hashes, fps)
    return mean_pop, entropy_pct, distribution, sampled_cuts, total_cuts_count, timeline, delta_dist, mean_delta


def inspect_phash_microscopic(identifier: str) -> dict[str, Any]:
    """Performs deep microscopic frame-by-frame inspection of a single .phash file."""
    if not identifier:
        return {"status": "error", "message": "Identifiant moviehash ou nom de fichier requis"}

    target_file = _resolve_target_phash_file(identifier)
    if not target_file or not target_file.is_file():
        return {"status": "not_found", "message": f"Fichier .phash pour '{identifier}' introuvable sur disque"}

    raw_bytes = target_file.read_bytes()
    fc = len(raw_bytes) // PHASH_FRAME_BYTES
    if fc == 0:
        return {"status": "error", "message": "Fichier .phash vide"}

    hashes = struct.unpack(f">{fc}Q", raw_bytes)
    fps = DEFAULT_BASELINE_FPS
    fps_m = RE_FPS.search(target_file.name)
    if fps_m:
        try:
            fps_val = float(fps_m.group(1))
            if fps_val > 0.0:
                fps = fps_val
        except ValueError:
            pass

    mean_pop, entropy_pct, distribution, cuts, total_cuts_count, timeline, delta_dist, mean_delta = (
        _detect_phash_cuts_and_entropy(hashes, fps)
    )
    duration_ms = int(fc / fps * 1000.0)
    sparse_count = duration_ms // 1000
    indexed_bytes = (total_cuts_count + sparse_count) * 16
    saved_pct = (
        round((1.0 - (indexed_bytes / float(len(raw_bytes)))) * 100.0, 1) if len(raw_bytes) > indexed_bytes else 0.0
    )
    ratio = round(len(raw_bytes) / float(indexed_bytes), 1) if indexed_bytes > 0 else 1.0

    return {
        "status": "ok",
        "filename": target_file.name,
        "moviehash": target_file.name.split(".")[0],
        "fps": fps,
        "frame_count": fc,
        "duration_ms": duration_ms,
        "duration_fmt": _format_duration(duration_ms),
        "file_size_bytes": len(raw_bytes),
        "file_size_human": format_human_size(len(raw_bytes)),
        "entropy": {
            "mean_popcount": round(mean_pop, 2),
            "entropy_pct": entropy_pct,
            "histogram": distribution,
            "health": (
                "Excellente (trame visuelle diversifiée)"
                if 25.0 <= mean_pop <= 38.0
                else "Anormale (potentielle trame statique)"
            ),
        },
        "mobility": {
            "mean_delta": round(mean_delta, 2),
            "histogram": delta_dist,
            "labels": list(DELTA_MOBILITY_LABELS),
            "descriptions": list(DELTA_MOBILITY_DESCRIPTIONS),
        },
        "cuts": {
            "cuts_sample": cuts,
            "cuts_count": total_cuts_count,
            "threshold": PHASH_CUT_HAMMING_THRESHOLD,
            "timeline": timeline,
        },
        "compression": {
            "raw_bytes": len(raw_bytes),
            "estimated_indexed_bytes": indexed_bytes,
            "saved_space_pct": saved_pct,
            "ratio": ratio,
            "cuts_count": total_cuts_count,
            "sparse_count": sparse_count,
        },
    }


@dataclass
class TrajectoriesQueryCriteria:
    file_name: str = ""
    page: int = 1
    limit: int = DEFAULT_TRAJECTORY_PAGE_SIZE
    status_filter: str = "all"
    search_q: str = ""
    sort_by: str = "name_asc"


_TRAJECTORY_FILE_CACHE: dict[str, tuple[float, list[dict[str, Any]]]] = {}
_PHASH_SIDECAR_TITLE_CACHE: dict[str, str] = {}


def _resolve_phash_target_title(target_media: str, target_path: str) -> str:
    """Resolves human-readable source title from sidecar JSON or cleaned target filename."""
    if not target_media:
        return ""
    clean_target = urllib.parse.unquote(target_media).replace(".phash", "")
    hex_match = re.match(r"^([a-fA-F0-9]{16})(?:\.\d+\.\d+fps)?$", clean_target)
    if not hex_match:
        return clean_target

    moviehash = hex_match.group(1).lower()
    if moviehash in _PHASH_SIDECAR_TITLE_CACHE:
        return _PHASH_SIDECAR_TITLE_CACHE[moviehash]

    # Check sidecar JSON next to .phash in target_path or data/__PHASH__/
    candidates: list[Path] = []
    if target_path:
        cand_p = Path(target_path)
        candidates.append(cand_p.with_suffix(".json"))
        candidates.append(cand_p.parent / f"{moviehash}.json")
    candidates.append(DEFAULT_TRAJECTORY_ROOT_DIR / "__PHASH__" / f"{moviehash}.json")
    candidates.append(DEFAULT_TRAJECTORY_ROOT_DIR / f"{moviehash}.json")

    for c in candidates:
        if c.is_file():
            try:
                meta = json.loads(c.read_text(encoding="utf-8"))
                media_name = meta.get("media_name") or meta.get("title") or meta.get("clean_title")
                if media_name:
                    _PHASH_SIDECAR_TITLE_CACHE[moviehash] = str(media_name)
                    return str(media_name)
            except Exception:
                pass

    _PHASH_SIDECAR_TITLE_CACHE[moviehash] = clean_target
    return clean_target


ACTIVE_BATCH_RECENCY_SECONDS: float = 60.0


def _is_valid_trajectory_file(path: Path) -> bool:
    """Fast probe: checks if a JSONL file conforms to spatiotemporal trajectory schema."""
    try:
        with open(path, encoding="utf-8", errors="ignore") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                data = json.loads(line)
                if not isinstance(data, dict):
                    return False
                has_target = "target_media" in data or "target_path" in data
                has_cuts = isinstance(data.get("cuts"), list) or "cuts_count" in data
                return bool(has_target and has_cuts)
    except Exception:
        return False
    return False


def list_trajectory_audit_files() -> list[dict[str, Any]]:
    """Scans and lists available trajectory tracking audit JSONL files."""
    candidates: list[Path] = []
    if DEFAULT_TRAJECTORY_DIR.is_dir():
        candidates.extend(DEFAULT_TRAJECTORY_DIR.glob("*.jsonl"))
    if DEFAULT_TRAJECTORY_ROOT_DIR.is_dir():
        candidates.extend(DEFAULT_TRAJECTORY_ROOT_DIR.glob("*trajectory*.jsonl"))

    results: list[dict[str, Any]] = []
    seen: set[str] = set()
    now = time.time()
    for path in sorted(candidates, key=lambda p: p.stat().st_mtime if p.exists() else 0.0, reverse=True):
        if path.name in seen or not path.is_file():
            continue
        seen.add(path.name)
        try:
            st = path.stat()
            if st.st_size == 0 or not _is_valid_trajectory_file(path):
                continue
            is_active = (now - st.st_mtime) < ACTIVE_BATCH_RECENCY_SECONDS
            results.append(
                {
                    "filename": path.name,
                    "path": str(path),
                    "size_bytes": st.st_size,
                    "size_human": format_human_size(st.st_size),
                    "mtime": st.st_mtime,
                    "modified_fmt": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(st.st_mtime)),
                    "date_short": time.strftime("%d %b %H:%M", time.localtime(st.st_mtime)),
                    "is_active": is_active,
                    "status_label": "En cours" if is_active else "Terminé",
                }
            )
        except OSError:
            continue
    return results


def _resolve_trajectory_file(file_name: str) -> Path | None:
    if file_name:
        cand1 = DEFAULT_TRAJECTORY_DIR / file_name
        if cand1.is_file():
            return cand1
        cand2 = DEFAULT_TRAJECTORY_ROOT_DIR / file_name
        if cand2.is_file():
            return cand2
    files = list_trajectory_audit_files()
    if files:
        return Path(files[0]["path"])
    return None


def _load_trajectory_records(file_name: str) -> tuple[Path | None, list[dict[str, Any]]]:
    path = _resolve_trajectory_file(file_name)
    if path is None or not path.is_file():
        return None, []
    try:
        mtime = path.stat().st_mtime
    except OSError:
        return None, []

    cache_key = str(path)
    cached = _TRAJECTORY_FILE_CACHE.get(cache_key)
    if cached is not None and cached[0] == mtime:
        return path, cached[1]

    records: list[dict[str, Any]] = []
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        records.append(json.loads(line))
                    except json.JSONDecodeError:
                        continue
    except OSError:
        return path, []

    _TRAJECTORY_FILE_CACHE[cache_key] = (mtime, records)
    return path, records


def _compute_catalog_coverage(matched_media_ids: set[Any]) -> tuple[int, int, float]:
    """Computes bounded catalog coverage against catalog.dbphash."""
    cat_path = Path(__file__).resolve().parents[1] / "data" / "catalog.dbphash"
    hdr, cat_items = _read_catalog_dbphash(cat_path) if cat_path.is_file() else ({}, [])
    catalog_ids = {c["catalog_id"] for c in cat_items} if cat_items else set()
    catalog_total = len(catalog_ids) if catalog_ids else 2779

    if catalog_ids:
        unique_covered = len(matched_media_ids.intersection(catalog_ids))
    else:
        unique_covered = min(len(matched_media_ids), catalog_total)

    pct = round(unique_covered / float(catalog_total) * 100.0, 1) if catalog_total > 0 else 0.0
    return unique_covered, catalog_total, max(0.0, min(100.0, pct))


def _compute_trajectory_kpi(records: list[dict[str, Any]]) -> dict[str, Any]:
    total_audits = len(records)
    total_coherent = 0
    total_matched = 0
    total_divergent = 0
    total_unmatched = 0
    total_cuts = 0
    total_elapsed = 0.0
    matched_media_ids: set[Any] = set()

    for r in records:
        mid = r.get("consensus_media_id")
        is_coh = r.get("is_temporally_coherent") is True
        total_cuts += r.get("cuts_count", 0)
        total_elapsed += float(r.get("total_elapsed_seconds", 0.0))
        if mid is not None:
            total_matched += 1
            matched_media_ids.add(mid)
            if is_coh:
                total_coherent += 1
            else:
                total_divergent += 1
        else:
            total_unmatched += 1

    total_consensus_won = total_matched - total_coherent
    unique_catalog_covered, catalog_total, catalog_coverage_pct = _compute_catalog_coverage(matched_media_ids)

    coherence_rate = round(total_coherent / float(total_matched) * 100.0, 1) if total_matched > 0 else 0.0
    match_rate = round(total_matched / float(total_audits) * 100.0, 1) if total_audits > 0 else 0.0
    avg_elapsed = round(total_elapsed / float(total_audits), 3) if total_audits > 0 else 0.0
    return {
        "total_audits": total_audits,
        "total_matched": total_matched,
        "total_coherent": total_coherent,
        "total_consensus_won": total_consensus_won,
        "total_divergent": total_divergent,
        "total_unmatched": total_unmatched,
        "unique_catalog_covered": unique_catalog_covered,
        "catalog_total": catalog_total,
        "catalog_coverage_pct": catalog_coverage_pct,
        "coherence_rate_pct": coherence_rate,
        "match_rate_pct": match_rate,
        "avg_elapsed_sec": avg_elapsed,
        "total_cuts": total_cuts,
    }


def _matches_trajectory_status(r: dict[str, Any], status: str) -> bool:
    has_match = r.get("consensus_media_id") is not None
    is_coherent = bool(r.get("is_temporally_coherent"))
    if status == "coherent":
        return is_coherent
    if status == "matched":
        return has_match
    if status == "unmatched":
        return not has_match
    if status == "divergent":
        return has_match and not is_coherent
    return True


def _sort_trajectory_records(records: list[dict[str, Any]], sort_by: str) -> list[dict[str, Any]]:
    if sort_by == "name_desc":
        return sorted(records, key=lambda r: str(r.get("target_media", "")).lower(), reverse=True)
    if sort_by == "duration_desc":
        return sorted(records, key=lambda r: float(r.get("duration_sec", 0.0)), reverse=True)
    if sort_by == "duration_asc":
        return sorted(records, key=lambda r: float(r.get("duration_sec", 0.0)))
    if sort_by == "elapsed_desc":
        return sorted(records, key=lambda r: float(r.get("total_elapsed_seconds", 0.0)), reverse=True)
    if sort_by == "elapsed_asc":
        return sorted(records, key=lambda r: float(r.get("total_elapsed_seconds", 0.0)))
    if sort_by == "matched_desc":
        return sorted(records, key=lambda r: int(r.get("matched_cuts_count", 0)), reverse=True)
    if sort_by == "matched_asc":
        return sorted(records, key=lambda r: int(r.get("matched_cuts_count", 0)))
    return sorted(records, key=lambda r: str(r.get("target_media", "")).lower())


def query_trajectory_records(criteria: TrajectoriesQueryCriteria) -> dict[str, Any]:
    """Queries, filters and paginates spatiotemporal trajectory records."""
    path, all_records = _load_trajectory_records(criteria.file_name)
    kpis = _compute_trajectory_kpi(all_records)

    q = strip_accents(criteria.search_q.lower().strip())
    filtered: list[dict[str, Any]] = []
    for r in all_records:
        if not _matches_trajectory_status(r, criteria.status_filter):
            continue
        if q:
            target = strip_accents(str(r.get("target_media", "")).lower())
            title = strip_accents(str(r.get("consensus_title", "")).lower())
            tpath = strip_accents(str(r.get("target_path", "")).lower())
            if q not in target and q not in title and q not in tpath:
                continue
        filtered.append(r)

    sorted_records = _sort_trajectory_records(filtered, criteria.sort_by)
    start_idx = (criteria.page - 1) * criteria.limit
    end_idx = start_idx + criteria.limit
    paged = sorted_records[start_idx:end_idx]

    items: list[dict[str, Any]] = []
    for r in paged:
        t_media = str(r.get("target_media", ""))
        t_path = str(r.get("target_path", ""))
        resolved_title = _resolve_phash_target_title(t_media, t_path)
        cuts_raw = r.get("cuts")
        cuts_list: list[dict[str, Any]] = cuts_raw if isinstance(cuts_raw, list) else []
        items.append(
            {
                "target_media": t_media,
                "target_path": t_path,
                "resolved_title": resolved_title,
                "duration_sec": r.get("duration_sec", 0.0),
                "cuts_count": r.get("cuts_count", 0),
                "matched_cuts_count": r.get("matched_cuts_count", 0),
                "consensus_media_id": r.get("consensus_media_id"),
                "consensus_title": r.get("consensus_title", "NONE"),
                "is_temporally_coherent": r.get("is_temporally_coherent", False),
                "total_elapsed_seconds": r.get("total_elapsed_seconds", 0.0),
                "success": r.get("success", True),
                "cuts_summary": [
                    {
                        "start_sec": c.get("start_sec", 0.0),
                        "winner_id": c.get("winner_media_id"),
                        "matched": c.get("winner_media_id") is not None,
                        "score": round(float(c.get("winner_score", 0.0)), 2),
                        "coverage": round(float(c.get("winner_coverage_ratio", 0.0)), 2),
                    }
                    for c in cuts_list
                ],
            }
        )

    total_pages = max(1, math.ceil(len(filtered) / float(criteria.limit)))
    return {
        "status": "ok",
        "file": path.name if path else "",
        "kpi": kpis,
        "total_items": len(all_records),
        "filtered_items": len(filtered),
        "page": criteria.page,
        "limit": criteria.limit,
        "total_pages": total_pages,
        "items": items,
    }


def get_trajectory_record_detail(file_name: str, target_media: str) -> dict[str, Any]:
    """Retrieves full microscopic trajectory cut details for a specific media target."""
    path, all_records = _load_trajectory_records(file_name)
    target_clean = strip_accents(target_media.lower().strip())
    for r in all_records:
        cand = strip_accents(str(r.get("target_media", "")).lower().strip())
        if cand == target_clean or strip_accents(str(r.get("target_path", "")).lower().strip()) == target_clean:
            item_copy = dict(r)
            t_media = str(r.get("target_media", ""))
            t_path = str(r.get("target_path", ""))
            item_copy["resolved_title"] = _resolve_phash_target_title(t_media, t_path)
            return {"status": "ok", "file": path.name if path else "", "item": item_copy}
    return {
        "status": "not_found",
        "error": f"Target media '{target_media}' not found in {path.name if path else 'file'}",
    }


def run_on_demand_trajectory(
    target_path: str,
    cuts: int = TRAJECTORY_SAMPLE_CUTS,
    frames: int = TRAJECTORY_DEFAULT_FRAMES,
) -> dict[str, Any]:
    """Executes on-demand spatiotemporal trajectory matching against catalog.dbphash."""
    if not target_path or not Path(target_path).is_file():
        return {"status": "error", "error": f"Target file '{target_path}' not found on local disk"}

    cmd = [
        "python3",
        "scripts/trajectory_bench_runner.py",
        target_path,
        "-c",
        str(cuts),
        "-n",
        str(frames),
        "--dbphash",
        "data/catalog.dbphash",
    ]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=60.0)
    except subprocess.TimeoutExpired:
        return {"status": "error", "error": "Trajectory tracking execution timed out (60s)"}

    if proc.returncode != 0:
        return {"status": "error", "error": proc.stderr or proc.stdout or "Execution failed"}

    res_file = Path("data/reports/trajectory_batch_results.jsonl")
    if not res_file.is_file():
        return {"status": "error", "error": "No trajectory output produced"}

    last_line = ""
    with open(res_file, encoding="utf-8") as f:
        for line in f:
            if line.strip():
                last_line = line.strip()

    if not last_line:
        return {"status": "error", "error": "Empty trajectory output"}

    try:
        record = json.loads(last_line)
        return {"status": "ok", "item": record, "stdout": proc.stdout}
    except json.JSONDecodeError as exc:
        return {"status": "error", "error": f"Failed to parse result JSON: {exc}"}


class PortalRequestHandler(http.server.SimpleHTTPRequestHandler):
    """Custom HTTP handler serving API JSON endpoints and clean use-case HTML pages."""

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, directory=str(STATIC_DIR), **kwargs)

    def do_HEAD(self) -> None:
        self.do_GET()

    def _handle_favicon(self, path: str) -> bool:
        if path != "/favicon.ico":
            return False
        self.send_response(HTTP_OK)
        self.send_header("Content-Type", "image/svg+xml")
        svg_icon = b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>\xf0\x9f\x8e\xac</text></svg>"
        self.send_header("Content-Length", str(len(svg_icon)))
        self.end_headers()
        self.wfile.write(svg_icon)
        return True

    def _handle_api_assets(self, query: dict[str, list[str]]) -> None:
        page_val = int(query.get("page", ["1"])[0]) if query.get("page", ["1"])[0].isdigit() else 1
        limit_val = (
            int(query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0])
            if query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0].isdigit()
            else DEFAULT_PAGE_SIZE
        )
        status_f = query.get("status", ["all"])[0]
        source_f = query.get("source", [""])[0]
        q_val = query.get("q", [""])[0]
        sort_val = query.get("sort", ["title_asc"])[0]
        result = get_pg_assets(
            page=page_val,
            limit=limit_val,
            status_filter=status_f,
            source_filter=source_f,
            search_q=q_val,
            sort_by=sort_val,
        )
        self._send_json(result)

    def _handle_api_source_routes(self, path: str, query: dict[str, list[str]]) -> bool:
        if path == "/api/sources":
            self._send_json(get_sources_metadata())
            return True
        if path == "/api/probe":
            result = execute_single_probe(query.get("source", [""])[0], query.get("profile", [""])[0])
            self._send_json(result)
            return True
        if path == "/api/files":
            p = int(query.get("page", ["1"])[0]) if query.get("page", ["1"])[0].isdigit() else 1
            l_val = (
                int(query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0])
                if query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0].isdigit()
                else DEFAULT_PAGE_SIZE
            )
            force = query.get("force", ["0"])[0] in ("1", "true", "yes")
            result = list_source_files(
                query.get("source", [""])[0],
                query.get("profile", [""])[0],
                query.get("subdir", [""])[0],
                p,
                l_val,
                query.get("q", [""])[0],
                query.get("sort", ["name_asc"])[0],
                force=force,
            )
            self._send_json(result)
            return True
        if path == "/api/file/details":
            force = query.get("force", ["0"])[0] in ("1", "true", "yes")
            result = inspect_file_details(
                query.get("source", [""])[0], query.get("profile", [""])[0], query.get("path", [""])[0], force=force
            )
            self._send_json(result)
            return True
        return False

    def _handle_api_asset_routes(self, path: str, query: dict[str, list[str]]) -> bool:
        if path == "/api/assets":
            self._handle_api_assets(query)
            return True
        if path == "/api/asset/details":
            id_str = query.get("id", ["0"])[0]
            media_id = int(id_str) if id_str.isdigit() else 0
            result = get_pg_asset_detail(media_id)
            status_code = (
                HTTP_OK
                if result.get("status") == "ok"
                else (HTTP_NOT_FOUND if result.get("status") == "not_found" else HTTP_SERVER_ERROR)
            )
            self._send_json(result, status=status_code)
            return True
        return False

    def _handle_api_phash_routes(self, path: str, query: dict[str, list[str]]) -> bool:
        if path == "/api/phashes":
            p = int(query.get("page", ["1"])[0]) if query.get("page", ["1"])[0].isdigit() else 1
            l_val = (
                int(query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0])
                if query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0].isdigit()
                else DEFAULT_PAGE_SIZE
            )
            status_f = query.get("filter", ["all"])[0]
            q_val = query.get("q", [""])[0]
            sort_val = query.get("sort", ["id_asc"])[0]
            criteria = PhashesQueryCriteria(
                page=p,
                limit=l_val,
                status_filter=status_f,
                search_q=q_val,
                sort_by=sort_val,
            )
            result = query_phashes_list(criteria)
            self._send_json(result)
            return True
        if path == "/api/phash/details":
            identifier = query.get("moviehash", [""])[0] or query.get("file", [""])[0]
            result = inspect_phash_microscopic(identifier)
            status_code = HTTP_OK if result.get("status") == "ok" else HTTP_NOT_FOUND
            self._send_json(result, status=status_code)
            return True
        return False

    def _handle_api_trajectory_routes(self, path: str, query: dict[str, list[str]]) -> bool:
        if path == "/api/trajectory/files":
            self._send_json({"status": "ok", "files": list_trajectory_audit_files()})
            return True
        if path == "/api/trajectories":
            f_name = query.get("file", [""])[0]
            p = int(query.get("page", ["1"])[0]) if query.get("page", ["1"])[0].isdigit() else 1
            l_val = (
                int(query.get("limit", [str(DEFAULT_TRAJECTORY_PAGE_SIZE)])[0])
                if query.get("limit", [str(DEFAULT_TRAJECTORY_PAGE_SIZE)])[0].isdigit()
                else DEFAULT_TRAJECTORY_PAGE_SIZE
            )
            status_f = query.get("filter", ["all"])[0]
            q_val = query.get("q", [""])[0]
            sort_val = query.get("sort", ["name_asc"])[0]
            criteria = TrajectoriesQueryCriteria(
                file_name=f_name,
                page=p,
                limit=l_val,
                status_filter=status_f,
                search_q=q_val,
                sort_by=sort_val,
            )
            result = query_trajectory_records(criteria)
            self._send_json(result)
            return True
        if path == "/api/trajectory/details":
            f_name = query.get("file", [""])[0]
            target = query.get("target", [""])[0]
            result = get_trajectory_record_detail(f_name, target)
            status_code = HTTP_OK if result.get("status") == "ok" else HTTP_NOT_FOUND
            self._send_json(result, status=status_code)
            return True
        return False

    def _handle_api(self, path: str, query: dict[str, list[str]]) -> bool:
        return (
            self._handle_api_source_routes(path, query)
            or self._handle_api_asset_routes(path, query)
            or self._handle_api_phash_routes(path, query)
            or self._handle_api_trajectory_routes(path, query)
        )

    def do_POST(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        if path == "/api/trajectory/run":
            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length).decode("utf-8") if length > 0 else "{}"
            try:
                data = json.loads(body)
            except Exception:
                data = {}
            target = data.get("target", "")
            cuts = int(data.get("cuts", TRAJECTORY_SAMPLE_CUTS))
            frames = int(data.get("frames", TRAJECTORY_DEFAULT_FRAMES))
            result = run_on_demand_trajectory(target, cuts=cuts, frames=frames)
            status_code = HTTP_OK if result.get("status") == "ok" else HTTP_SERVER_ERROR
            self._send_json(result, status=status_code)
            return
        self.send_error(HTTP_NOT_FOUND, "Not found")

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query)

        if self._handle_favicon(path):
            return
        if self._handle_api(path, query):
            return

        if path in ("/", "/sources", "/sources/"):
            self.path = "/sources.html"

        super().do_GET()

    def _send_json(self, payload: Any, status: int = HTTP_OK) -> None:
        raw = json.dumps(payload, indent=2, default=str).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(raw)


class ReusableThreadingTCPServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def run_server(port: int = DEFAULT_PORT) -> None:
    STATIC_DIR.mkdir(parents=True, exist_ok=True)
    with ReusableThreadingTCPServer(("0.0.0.0", port), PortalRequestHandler) as httpd:
        print(f"🚀 pydbsrt Portal running on http://127.0.0.1:{port}/")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\nShutting down server.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="pydbsrt MediaSources Portal Server")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT, help=f"HTTP port (default {DEFAULT_PORT})")
    parser.add_argument("--metadata", action="store_true", help="Print sources metadata JSON and exit")
    parser.add_argument("--probe", nargs=2, metavar=("SOURCE", "PROFILE"), help="Execute single probe and exit")
    parser.add_argument("--files", nargs=2, metavar=("SOURCE", "PROFILE"), help="List files for source/profile")
    parser.add_argument("--subdir", default="", help="Subdirectory path within source")
    parser.add_argument("--page", type=int, default=1, help="Page number (default 1)")
    parser.add_argument("--limit", type=int, default=DEFAULT_PAGE_SIZE, help="Page limit (default 25)")
    parser.add_argument("--search", default="", help="Search / filter query")
    parser.add_argument(
        "--sort", default="name_asc", help="Sort order (name_asc, name_desc, size_desc, size_asc, date_desc)"
    )
    parser.add_argument("--details", nargs=3, metavar=("SOURCE", "PROFILE", "PATH"), help="Inspect multimedia details")
    parser.add_argument("--assets", action="store_true", help="List PG media assets (JSON)")
    parser.add_argument("--asset-id", type=int, help="Single media asset ID for detailed inspection")
    parser.add_argument(
        "--status", default="all", help="Status filter for assets (all, nominal, missing_phash, unmapped_path)"
    )
    parser.add_argument("--source", default="", help="Source filter for assets")
    parser.add_argument("--force", action="store_true", help="Bypass cache and force fresh live probe")
    parser.add_argument("--phashes", action="store_true", help="List tripartite pHash fingerprints (JSON)")
    parser.add_argument("--phash-moviehash", default="", help="Inspect microscopic details of single phash")
    parser.add_argument(
        "--phash-filter",
        default="all",
        help="Filter for phashes (all, aligned_3_3, missing_pg, uncompiled, orphan_catalog)",
    )
    parser.add_argument(
        "--trajectory-files",
        action="store_true",
        help="List available trajectory audit files (JSON)",
    )
    parser.add_argument(
        "--trajectories",
        action="store_true",
        help="Query spatiotemporal trajectory records (JSON)",
    )
    parser.add_argument(
        "--trajectory-file",
        default="",
        help="Specific trajectory file to query",
    )
    parser.add_argument(
        "--trajectory-details",
        default="",
        help="Inspect detailed cuts sequence for target media",
    )
    parser.add_argument(
        "--trajectory-filter",
        default="all",
        help="Trajectory filter (all, coherent, matched, divergent, unmatched)",
    )
    args = parser.parse_args()

    if args.metadata:
        print(json.dumps(get_sources_metadata(), indent=2))
    elif args.probe:
        print(json.dumps(execute_single_probe(args.probe[0], args.probe[1]), indent=2))
    elif args.files:
        print(
            json.dumps(
                list_source_files(
                    args.files[0],
                    args.files[1],
                    args.subdir,
                    args.page,
                    args.limit,
                    args.search,
                    args.sort,
                    force=args.force,
                ),
                indent=2,
            )
        )
    elif args.details:
        print(
            json.dumps(
                inspect_file_details(args.details[0], args.details[1], args.details[2], force=args.force), indent=2
            )
        )
    elif args.assets:
        print(
            json.dumps(
                get_pg_assets(
                    page=args.page,
                    limit=args.limit,
                    status_filter=args.status,
                    source_filter=args.source,
                    search_q=args.search,
                    sort_by=args.sort,
                ),
                indent=2,
                default=str,
            )
        )
    elif args.asset_id:
        print(json.dumps(get_pg_asset_detail(args.asset_id), indent=2, default=str))
    elif args.phashes:
        phash_criteria = PhashesQueryCriteria(
            page=args.page,
            limit=args.limit,
            status_filter=args.phash_filter if args.phash_filter != "all" else args.status,
            search_q=args.search,
            sort_by=args.sort,
        )
        print(json.dumps(query_phashes_list(phash_criteria), indent=2, default=str))
    elif args.phash_moviehash:
        print(json.dumps(inspect_phash_microscopic(args.phash_moviehash), indent=2, default=str))
    elif args.trajectory_files:
        print(json.dumps(list_trajectory_audit_files(), indent=2, default=str))
    elif args.trajectories:
        traj_crit = TrajectoriesQueryCriteria(
            file_name=args.trajectory_file,
            page=args.page,
            limit=args.limit,
            status_filter=args.trajectory_filter if args.trajectory_filter != "all" else args.status,
            search_q=args.search,
            sort_by=args.sort,
        )
        print(json.dumps(query_trajectory_records(traj_crit), indent=2, default=str))
    elif args.trajectory_details:
        print(
            json.dumps(
                get_trajectory_record_detail(args.trajectory_file, args.trajectory_details), indent=2, default=str
            )
        )
    else:
        run_server(args.port)
