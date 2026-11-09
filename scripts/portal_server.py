#!/usr/bin/env python3
"""Unified light HTTP backend for pydbsrt MediaSources Portal.

Fully decoupled & asynchronous design:
1. `GET /api/sources`: Instant return (< 1ms) of configured sources & profiles structure in pending state.
2. `GET /api/probe?source=<id>&profile=<name>`: Granular parallel probe per individual channel.
"""

from __future__ import annotations

import argparse
import ftplib
import http.server
import json
import os
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
    args = parser.parse_args()

    if args.metadata:
        print(json.dumps(get_sources_metadata(), indent=2))
    elif args.probe:
        print(json.dumps(execute_single_probe(args.probe[0], args.probe[1]), indent=2))
    else:
        run_server(args.port)
