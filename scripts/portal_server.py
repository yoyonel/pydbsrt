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
import os
import re
import shutil
import socket
import socketserver
import subprocess
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Dict

DEFAULT_PORT: int = 8780
PROBE_FS_TIMEOUT_SECS: float = 0.3
PROBE_FTP_TIMEOUT_SECS: float = 3.0
PROBE_HTTP_TIMEOUT_SECS: float = 1.5
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

DEFAULT_NAS_HOST: str = "yoyonel.synology.me"
DEFAULT_NAS_USER: str = "qzsl2686"
DEFAULT_PASS_ENTRY: str = os.environ.get("NAS_PASS_ENTRY", ".".join(["192", "168", "1", "58"]) + ":5000/qzsl2686")
DEFAULT_GATEWAY_URL: str = "http://yoyonel.synology.me:18089/health"

DEFAULT_FREEBOX_HOST: str = "mafreebox.freebox.fr"
DEFAULT_FREEBOX_USER: str = "freebox"
DEFAULT_FREEBOX_PASS_ENTRY: str = "services/freebox/ftp_pass"

AVAILABLE_ENVIRONMENTS: list[dict[str, str]] = [
    {
        "id": "o2switch",
        "name": "Cloud o2switch",
        "icon": "🌐",
        "description": "Hébergement distant (Flux WAN FTP & Passerelle HTTP)",
    },
    {
        "id": "local",
        "name": "Poste Local (debian-latty)",
        "icon": "💻",
        "description": "Poste de dev (Montages CIFS locaux, Snippets, FTP WAN)",
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

_LISTING_CACHE: dict[str, tuple[float, str, str, str, list[dict[str, Any]]]] = {}
_DETAILS_CACHE: dict[str, tuple[float, dict[str, Any]]] = {}


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


def _resolve_secret(env_var: str, secret_filename: str, pass_entry: str) -> str:
    """Resolves credential from env, ~/.dbsrt/etc/<file>, or pass store."""
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
        proc = subprocess.run(["pass", "show", pass_entry], capture_output=True, text=True, timeout=1.0)
        if proc.returncode == 0 and proc.stdout.strip():
            return proc.stdout.strip().splitlines()[0]
    except (OSError, subprocess.SubprocessError):
        pass
    return ""


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
    """Tests HTTP streaming gateway reachability."""
    t0 = time.perf_counter()
    try:
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req, timeout=PROBE_HTTP_TIMEOUT_SECS) as resp:
            dt_ms = (time.perf_counter() - t0) * 1000.0
            return {
                "name": name,
                "protocol": "http_gateway",
                "target": url,
                "accessible": True,
                "status": "online",
                "latency_ms": round(dt_ms, 1),
                "files_count": 0,
                "reason": f"Passerelle HTTP active (HTTP {resp.status})",
            }
    except Exception:
        dt_ms = (time.perf_counter() - t0) * 1000.0
        return {
            "name": name,
            "protocol": "http_gateway",
            "target": url,
            "accessible": False,
            "status": "offline",
            "latency_ms": round(dt_ms, 1),
            "files_count": 0,
            "reason": "Passerelle injoignable (RPi4 offline)",
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
                    "id": "wan_ftp",
                    "name": "Streaming WAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_HOST}:21/video",
                    "envs": ["local", "o2switch", "rpi4"],
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
                    "id": "wan_ftp",
                    "name": "Streaming WAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_HOST}:21/tvshow",
                    "envs": ["local", "o2switch", "rpi4"],
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
                    "id": "wan_http",
                    "name": "Passerelle HTTP WAN",
                    "protocol": "http_gateway",
                    "target": DEFAULT_GATEWAY_URL,
                    "envs": ["local", "o2switch"],
                    "status": "pending",
                },
                {
                    "id": "rpi4_gateway",
                    "name": "Passerelle HTTP Locale (RPi4)",
                    "protocol": "http_gateway",
                    "target": "http://127.0.0.1:18089/health",
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
                    "envs": ["local"],
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
    if profile_id == "wan_ftp":
        return probe_ftp_profile("Streaming WAN FTP", DEFAULT_NAS_HOST, DEFAULT_NAS_USER, passwd, path_key)
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
    if profile_id == "wan_http":
        return probe_http_gateway_profile("Passerelle HTTP WAN", DEFAULT_GATEWAY_URL)
    if profile_id == "rpi4_gateway":
        return probe_http_gateway_profile("Passerelle HTTP Locale (RPi4)", "http://127.0.0.1:18089/health")
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


def _parse_ftp_directory_items(ftp: ftplib.FTP, clean_sub: str) -> list[dict[str, Any]]:
    """Lists and parses items from connected FTP server directory."""
    try:
        mlsd_items = list(ftp.mlsd())
    except Exception:
        mlsd_items = []

    if mlsd_items:
        items = [_parse_mlsd_item(name, facts, clean_sub) for name, facts in mlsd_items]
        return [it for it in items if it is not None]

    items = [_parse_nlst_item(name, clean_sub) for name in ftp.nlst()]
    return [it for it in items if it is not None]


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

    # 3. WAN FTP (Synology)
    if profile_id == "wan_ftp" and source_id in ("synology-video", "synology-tvshow"):
        base_dir = "/video" if source_id == "synology-video" else "/tvshow"
        target_dir = base_dir if not clean_sub else f"{base_dir}/{clean_sub}"
        passwd = resolve_nas_password()
        chan_name = "Streaming WAN FTP"
        base_target = f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_HOST}:21{base_dir}"

        items: list[dict[str, Any]] = []
        try:
            ftp = ftplib.FTP(timeout=PROBE_FTP_TIMEOUT_SECS * 2)
            ftp.connect(DEFAULT_NAS_HOST, 21, timeout=PROBE_FTP_TIMEOUT_SECS * 2)
            ftp.login(DEFAULT_NAS_USER, passwd)
            ftp.cwd(target_dir)
            items = _parse_ftp_directory_items(ftp, clean_sub)
            ftp.quit()
        except Exception:
            pass
        return chan_name, "ftp", base_target, items

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
        return sorted(items, key=lambda x: (not x["is_dir"], x["name"].lower()), reverse=True)
    if sort_mode == "size_desc":
        return sorted(items, key=lambda x: (not x["is_dir"], x["size_bytes"]), reverse=True)
    if sort_mode == "size_asc":
        return sorted(items, key=lambda x: (x["is_dir"], x["size_bytes"]))
    if sort_mode == "date_desc":
        return sorted(items, key=lambda x: (not x["is_dir"], x.get("modify", "")), reverse=True)
    return sorted(items, key=lambda x: (not x["is_dir"], x["name"].lower()))


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


def _resolve_target_endpoint(source_id: str, profile_id: str, clean_path: str) -> tuple[bool, bool, str, str, str]:
    """Resolves local vs remote target path, ftp url, and credentials."""
    if source_id == "local-snippets" and profile_id == "local":
        return True, False, str(Path("data/snippets") / clean_path), "", ""
    if profile_id in ("cifs", "disque1", "disque2"):
        base_cifs = _user_media_path("Synology", "video" if source_id == "synology-video" else "tvshow")
        return True, False, str(Path(base_cifs) / clean_path), "", ""
    if profile_id == "wan_ftp":
        base_dir = "/video" if source_id == "synology-video" else "/tvshow"
        ftp_url = f"ftp://{DEFAULT_NAS_HOST}:21{base_dir}/{clean_path}"
        return False, True, "", ftp_url, resolve_nas_password()
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
        cmd = [
            ffprobe_bin,
            "-v",
            "error",
            "-ftp-user",
            DEFAULT_NAS_USER,
            "-ftp-password",
            passwd,
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


class PortalRequestHandler(http.server.SimpleHTTPRequestHandler):
    """Custom HTTP handler serving API JSON endpoints and clean use-case HTML pages."""

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, directory=str(STATIC_DIR), **kwargs)

    def do_HEAD(self) -> None:
        self.do_GET()

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query)

        # 0. Favicon handler (clean zero-404)
        if path == "/favicon.ico":
            self.send_response(HTTP_OK)
            self.send_header("Content-Type", "image/svg+xml")
            svg_icon = b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>\xf0\x9f\x8e\xac</text></svg>"
            self.send_header("Content-Length", str(len(svg_icon)))
            self.end_headers()
            self.wfile.write(svg_icon)
            return

        # 1. Instant metadata definition
        if path == "/api/sources":
            data = get_sources_metadata()
            self._send_json(data)
            return

        # 2. Granular probe per channel
        if path == "/api/probe":
            source_id = query.get("source", [""])[0]
            profile_id = query.get("profile", [""])[0]
            result = execute_single_probe(source_id, profile_id)
            self._send_json(result)
            return

        # 3. Explore files listing (paginated, filtered, sorted)
        if path == "/api/files":
            source_id = query.get("source", [""])[0]
            profile_id = query.get("profile", [""])[0]
            subdir = query.get("subdir", [""])[0]
            page_val = int(query.get("page", ["1"])[0]) if query.get("page", ["1"])[0].isdigit() else 1
            limit_val = (
                int(query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0])
                if query.get("limit", [str(DEFAULT_PAGE_SIZE)])[0].isdigit()
                else DEFAULT_PAGE_SIZE
            )
            q_val = query.get("q", [""])[0]
            sort_val = query.get("sort", ["name_asc"])[0]
            force_val = query.get("force", ["0"])[0] in ("1", "true", "yes")
            result = list_source_files(
                source_id, profile_id, subdir, page_val, limit_val, q_val, sort_val, force=force_val
            )
            self._send_json(result)
            return

        # 4. Detailed file inspection (multimedia ffprobe & DB metadata)
        if path == "/api/file/details":
            source_id = query.get("source", [""])[0]
            profile_id = query.get("profile", [""])[0]
            file_path = query.get("path", [""])[0]
            force_val = query.get("force", ["0"])[0] in ("1", "true", "yes")
            result = inspect_file_details(source_id, profile_id, file_path, force=force_val)
            self._send_json(result)
            return

        if path in ("/", "/sources", "/sources/"):
            self.path = "/sources.html"

        super().do_GET()

    def _send_json(self, payload: Any, status: int = HTTP_OK) -> None:
        raw = json.dumps(payload, indent=2).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(raw)


class ReusableTCPServer(socketserver.TCPServer):
    allow_reuse_address = True


def run_server(port: int = DEFAULT_PORT) -> None:
    STATIC_DIR.mkdir(parents=True, exist_ok=True)
    with ReusableTCPServer(("0.0.0.0", port), PortalRequestHandler) as httpd:
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
    parser.add_argument("--force", action="store_true", help="Bypass cache and force fresh live probe")
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
    else:
        run_server(args.port)
