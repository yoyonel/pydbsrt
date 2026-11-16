"""Channel probing and sources metadata service for Synology, Freebox, and local snippets."""

from __future__ import annotations

import ftplib
import json
import os
import socket
import time
import urllib.request
from pathlib import Path
from typing import Any

from pydbsrt.portal import config as portal_config
from pydbsrt.portal.config import (
    AVAILABLE_ENVIRONMENTS,
    DEFAULT_FREEBOX_GATEWAY_PORT,
    DEFAULT_FREEBOX_HOST,
    DEFAULT_FREEBOX_USER,
    DEFAULT_FTP_PORT,
    DEFAULT_NAS_LAN_HOST,
    DEFAULT_NAS_USER,
    DEFAULT_NAS_WAN_HOST,
    EXCLUDED_FREEBOX_DIR_NAMES,
    FREEBOX_FTP_ELIGIBLE_DIRS,
    MILLIS_PER_SEC,
    PROBE_FTP_TIMEOUT_SECS,
    PROBE_HTTP_TIMEOUT_SECS,
    _is_excluded_portal_entry,
    _resolve_data_path,
)
from pydbsrt.portal.services import secrets_env
from pydbsrt.portal.services.ftp_parsers import _parse_ftp_directory_items


def probe_local_fs_profile(name: str, path_str: str, is_cifs: bool = False) -> dict[str, Any]:
    """Tests existence of a local or mounted directory and counts real video files."""
    t0 = time.perf_counter()
    if is_cifs and not secrets_env._is_cifs_safe_to_stat():
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
        return {
            "name": name,
            "protocol": "local_fs",
            "target": path_str,
            "accessible": False,
            "status": "offline",
            "latency_ms": round(dt_ms, 2),
            "files_count": 0,
            "reason": "Montage CIFS bloqué hors LAN domestique (protection anti-gel SMB VPN/4G)",
        }

    p = Path(path_str)
    if not p.is_dir():
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
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
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
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
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
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

    files_count = secrets_env._count_directory_videos(p)
    has_files = files_count > 0
    dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
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


def probe_ftp_profile(name: str, host: str, user: str, passwd: str, path_str: str) -> dict[str, Any]:
    """Tests FTP directory access, authentication and lists root files."""
    t0 = time.perf_counter()
    target_uri = f"ftp://{user}@{host}:{DEFAULT_FTP_PORT}{path_str}"
    try:
        ftp = ftplib.FTP(timeout=PROBE_FTP_TIMEOUT_SECS)
        ftp.connect(host, DEFAULT_FTP_PORT, timeout=PROBE_FTP_TIMEOUT_SECS)
        ftp.login(user, passwd)
        raw_files = ftp.nlst(path_str)
        ftp.quit()
        files = [f for f in raw_files if not _is_excluded_portal_entry(Path(f).name)]
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
        accessible = len(files) > 0
        return {
            "name": name,
            "protocol": "ftp",
            "target": target_uri,
            "accessible": accessible,
            "status": "online" if accessible else "empty",
            "latency_ms": round(dt_ms, 1),
            "files_count": len(files),
            "reason": f"{len(files)} éléments listés via FTP",
        }
    except Exception as err:
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
        err_msg = "Hôte LAN inaccessible (hors réseau domestique)" if "mafreebox" in host else str(err)
        return {
            "name": name,
            "protocol": "ftp",
            "target": target_uri,
            "accessible": False,
            "status": "offline",
            "latency_ms": round(dt_ms, 1),
            "files_count": 0,
            "reason": err_msg,
        }


def _parse_gateway_health_json(resp: Any) -> tuple[bool, dict[str, Any], str]:
    """Validates that an HTTP gateway response is genuine JSON health payload (not WAN HTML)."""
    ctype = resp.headers.get_content_type()
    if ctype != "application/json":
        return False, {}, ctype
    try:
        data = json.loads(resp.read().decode("utf-8"))
    except (ValueError, UnicodeDecodeError):
        return False, {}, ctype
    if not isinstance(data, dict):
        return False, {}, ctype
    is_valid = data.get("status") == "ok" or isinstance(data.get("mounted_shares"), list)
    return is_valid, data, ctype


def _count_cached_freebox_targets() -> int:
    """Returns cached media count from freebox_batch_targets.json if present."""
    targets_file = _resolve_data_path("data/reports/freebox_batch_targets.json")
    if not targets_file.is_file():
        return 0
    try:
        return len(json.loads(targets_file.read_text(encoding="utf-8")))
    except (OSError, ValueError, TypeError):
        return 0


def probe_http_gateway_profile(name: str, url: str) -> dict[str, Any]:
    """Tests HTTP streaming gateway reachability and verifies non-zero eligible media count."""
    t0 = time.perf_counter()
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "dbsrt-portal/1.0"})
        with urllib.request.urlopen(req, timeout=PROBE_HTTP_TIMEOUT_SECS) as resp:
            dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
            valid_json, data, ctype = _parse_gateway_health_json(resp)
            if not valid_json:
                return {
                    "name": name,
                    "protocol": "http_gateway",
                    "target": url,
                    "accessible": False,
                    "status": "offline",
                    "latency_ms": round(dt_ms, 1),
                    "files_count": 0,
                    "reason": f"Réponse non-JSON ({ctype}) — port WAN :80 filtré par IP ou passerelle inactive",
                }
            mounted = data.get("mounted_shares", [])
            live_count = data.get("media_count")
            media_count = int(live_count) if isinstance(live_count, int) else _count_cached_freebox_targets()
            shares_str = f" ({', '.join(mounted)})" if mounted else ""
            if media_count <= 0:
                return {
                    "name": name,
                    "protocol": "http_gateway",
                    "target": url,
                    "accessible": False,
                    "status": "empty",
                    "latency_ms": round(dt_ms, 1),
                    "files_count": 0,
                    "reason": f"Passerelle HTTP jointe{shares_str} mais aucun conteneur vidéo éligible exposé",
                }
            return {
                "name": name,
                "protocol": "http_gateway",
                "target": url,
                "accessible": True,
                "status": "online",
                "latency_ms": round(dt_ms, 1),
                "files_count": media_count,
                "reason": f"Passerelle HTTP active{shares_str} • {media_count} médias indexés",
            }
    except Exception as err:
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
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


def get_sources_metadata() -> dict[str, Any]:
    """Instantaneous descriptor of media sources with unprobed/pending profiles."""
    hostname = socket.gethostname()
    detected_env = secrets_env.detect_host_environment()

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
                    "target": secrets_env._user_media_path("Synology", "video"),
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "lan_ftp",
                    "name": "Streaming LAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_LAN_HOST}:{DEFAULT_FTP_PORT}/video",
                    "envs": ["local", "rpi4"],
                    "status": "pending",
                },
                {
                    "id": "wan_ftp",
                    "name": "Streaming WAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_WAN_HOST}:{DEFAULT_FTP_PORT}/video",
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
                    "target": secrets_env._user_media_path("Synology", "tvshow"),
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "lan_ftp",
                    "name": "Streaming LAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_LAN_HOST}:{DEFAULT_FTP_PORT}/tvshow",
                    "envs": ["local", "rpi4"],
                    "status": "pending",
                },
                {
                    "id": "wan_ftp",
                    "name": "Streaming WAN FTP",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_WAN_HOST}:{DEFAULT_FTP_PORT}/tvshow",
                    "envs": ["o2switch", "nomad"],
                    "status": "pending",
                },
            ],
        },
        {
            "id": "freebox-disks",
            "name": "Freebox Server Disks",
            "description": "Disques durs internes/USB de la Freebox (Disque_1, Disque_2, Disque_1_1).",
            "profiles": [
                *secrets_env._build_freebox_cifs_profiles(),
                {
                    "id": "lan_ftp",
                    "name": "FTP LAN Freebox",
                    "protocol": "ftp",
                    "target": f"ftp://{DEFAULT_FREEBOX_USER}@{DEFAULT_FREEBOX_HOST}:{DEFAULT_FTP_PORT}",
                    "envs": ["local", "rpi4"],
                    "status": "pending",
                },
                {
                    "id": "lan_http",
                    "name": "Passerelle HTTP LAN (RPi4)",
                    "protocol": "http_gateway",
                    "target": portal_config.DEFAULT_GATEWAY_LAN_URL,
                    "envs": ["local"],
                    "status": "pending",
                },
                {
                    "id": "wan_http",
                    "name": "Passerelle HTTP WAN",
                    "protocol": "http_gateway",
                    "target": portal_config.DEFAULT_GATEWAY_WAN_URL,
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


def _probe_synology(source_id: str, profile_id: str, passwd: str) -> dict[str, Any] | None:
    """Dispatches probes for Synology sources."""
    path_key = "/video" if source_id == "synology-video" else "/tvshow"
    mount_dir = secrets_env._user_media_path("Synology", path_key.lstrip("/"))

    if profile_id == "cifs":
        return probe_local_fs_profile("Montage CIFS Local", mount_dir, is_cifs=True)
    if profile_id == "lan_ftp":
        return probe_ftp_profile("Streaming LAN FTP", DEFAULT_NAS_LAN_HOST, DEFAULT_NAS_USER, passwd, path_key)
    if profile_id == "wan_ftp":
        return probe_ftp_profile("Streaming WAN FTP", DEFAULT_NAS_WAN_HOST, DEFAULT_NAS_USER, passwd, path_key)
    return None


def probe_freebox_ftp_profile(name: str, host: str, user: str, passwd: str) -> dict[str, Any]:
    """Probes Freebox FTP strictly inside the 3 eligible media directories, excluding personal folders."""
    t0 = time.perf_counter()
    target_uri = f"ftp://{user}@{host}:{DEFAULT_FTP_PORT}"
    try:
        ftp = ftplib.FTP(timeout=PROBE_FTP_TIMEOUT_SECS)
        ftp.connect(host, DEFAULT_FTP_PORT, timeout=PROBE_FTP_TIMEOUT_SECS)
        ftp.login(user, passwd)
        eligible_items = 0
        for rel_dir in FREEBOX_FTP_ELIGIBLE_DIRS:
            try:
                ftp.cwd(f"/{rel_dir}")
                parsed = _parse_ftp_directory_items(ftp, rel_dir)
                eligible_items += len(
                    [it for it in parsed if str(it.get("name", "")).lower() not in EXCLUDED_FREEBOX_DIR_NAMES]
                )
            except Exception:
                continue
        ftp.quit()
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
        has_items = eligible_items > 0
        return {
            "name": name,
            "protocol": "ftp",
            "target": target_uri,
            "accessible": has_items,
            "status": "online" if has_items else "empty",
            "latency_ms": round(dt_ms, 1),
            "files_count": eligible_items,
            "reason": (
                f"{eligible_items} éléments éligibles listés via FTP"
                if has_items
                else "Aucun média éligible listé via FTP"
            ),
        }
    except Exception as err:
        dt_ms = (time.perf_counter() - t0) * MILLIS_PER_SEC
        err_msg = "Hôte LAN inaccessible (hors réseau domestique)" if "mafreebox" in host else str(err)
        return {
            "name": name,
            "protocol": "ftp",
            "target": target_uri,
            "accessible": False,
            "status": "offline",
            "latency_ms": round(dt_ms, 1),
            "files_count": 0,
            "reason": err_msg,
        }


def _probe_freebox(profile_id: str) -> dict[str, Any] | None:
    """Dispatches probes for Freebox sources across all 3 SSOT disks and gateways."""
    cifs_hit = secrets_env._resolve_freebox_cifs_mount(profile_id)
    if cifs_hit is not None:
        return probe_local_fs_profile(cifs_hit[0], cifs_hit[1], is_cifs=True)
    if profile_id == "lan_ftp":
        passwd = secrets_env.resolve_freebox_password()
        return probe_freebox_ftp_profile("FTP LAN Freebox", DEFAULT_FREEBOX_HOST, DEFAULT_FREEBOX_USER, passwd)
    if profile_id == "lan_http":
        return probe_http_gateway_profile("Passerelle HTTP LAN (RPi4)", portal_config.DEFAULT_GATEWAY_LAN_URL)
    if profile_id == "wan_http":
        return probe_http_gateway_profile("Passerelle HTTP WAN", portal_config.DEFAULT_GATEWAY_WAN_URL)
    if profile_id == "rpi4_gateway":
        return probe_http_gateway_profile(
            "Passerelle HTTP Locale (RPi4)", f"http://127.0.0.1:{DEFAULT_FREEBOX_GATEWAY_PORT}/health"
        )
    return None


def execute_single_probe(source_id: str, profile_id: str) -> dict[str, Any]:
    """Executes a single probe on demand and returns result immediately."""
    passwd = secrets_env.resolve_nas_password()

    if source_id in ("synology-video", "synology-tvshow"):
        res = _probe_synology(source_id, profile_id, passwd)
        if res is not None:
            return res

    if source_id == "freebox-disks":
        res = _probe_freebox(profile_id)
        if res is not None:
            return res

    if source_id == "local-snippets" and profile_id == "local":
        return probe_local_fs_profile("Système de Fichiers Local", str(_resolve_data_path("data/snippets")))

    return {
        "accessible": False,
        "status": "unknown",
        "files_count": 0,
        "reason": f"Profil inconnu ({source_id}/{profile_id})",
    }
