"""Secrets resolution, host environment detection, CIFS safety guard, and Freebox disk profiles."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import threading
import time
from pathlib import Path
from typing import Any

from pydbsrt.portal.config import (
    CIFS_SAFETY_CACHE_TTL_SECS,
    DEFAULT_FREEBOX_PASS_ENTRY,
    DEFAULT_PASS_ENTRY,
    EXCLUDED_FREEBOX_DIR_NAMES,
    REPO_ROOT,
    SUBPROCESS_PASS_TIMEOUT_SECS,
    VIDEO_EXTENSIONS,
    _resolve_data_path,
)
from scripts.data_access.resolver import load_raw_registry
from scripts.data_access.topology import inspect_network_topology

_SECRET_MEMORY_CACHE: dict[str, str] = {}
_SECRET_CACHE_LOCK: threading.Lock = threading.Lock()
_CIFS_SAFETY_CACHE: tuple[float, bool] | None = None
_CIFS_SAFETY_LOCK: threading.Lock = threading.Lock()


def _lookup_coupling(filename: str) -> dict[str, Any] | None:
    """Finds matching ground-truth certified coupling from couplings.json."""
    couplings_file = _resolve_data_path("data/SRT/couplings.json")
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


def _lookup_secret_uncached(env_var: str, secret_filename: str, pass_entry: str) -> str:
    """Performs raw credential discovery across env, file paths, and pass store."""
    if env_var in os.environ:
        return os.environ[env_var]
    for path in (
        Path.home() / "dbsrt" / "etc" / secret_filename,
        REPO_ROOT / "etc" / secret_filename,
    ):
        secret = _read_secret_file(path)
        if secret:
            return secret
    try:
        proc = subprocess.run(
            ["pass", "show", pass_entry],
            capture_output=True,
            text=True,
            timeout=SUBPROCESS_PASS_TIMEOUT_SECS,
        )
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
    """Recursively counts eligible video files while pruning excluded personal/system folders."""
    total = 0
    if not path.is_dir():
        return 0
    try:
        for _dirpath, dirnames, filenames in os.walk(path):
            dirnames[:] = [d for d in dirnames if not d.startswith(".") and d.lower() not in EXCLUDED_FREEBOX_DIR_NAMES]
            for fname in filenames:
                if not fname.startswith(".") and Path(fname).suffix.lower() in VIDEO_EXTENSIONS:
                    total += 1
    except OSError:
        pass
    return total


def _is_cifs_safe_to_stat() -> bool:
    """Checks kernel network topology to prevent SMB D-state hangs off Home LAN."""
    global _CIFS_SAFETY_CACHE
    now = time.monotonic()
    with _CIFS_SAFETY_LOCK:
        if _CIFS_SAFETY_CACHE is not None and (now - _CIFS_SAFETY_CACHE[0]) < CIFS_SAFETY_CACHE_TTL_SECS:
            return _CIFS_SAFETY_CACHE[1]
    safe = bool(inspect_network_topology().get("cifs_safe_to_stat", False))
    with _CIFS_SAFETY_LOCK:
        _CIFS_SAFETY_CACHE = (now, safe)
    return safe


def _get_freebox_disk_specs() -> list[dict[str, Any]]:
    """Loads the 3 Freebox Server disk definitions from the SSOT registry."""
    try:
        disks = load_raw_registry().get("freebox_disks", [])
        if isinstance(disks, list) and disks:
            return disks
    except OSError:
        pass
    return [
        {
            "id": "disque1",
            "smb_share": "Disque 1",
            "mount_folder": "Disque_1",
            "indexed_subdir": "Téléchargements",
            "rpi4_cifs_path": "/mnt/freebox/Disque_1",
        },
        {
            "id": "disque2",
            "smb_share": "Disque 2",
            "mount_folder": "Disque_2",
            "indexed_subdir": "Téléchargements",
            "rpi4_cifs_path": "/mnt/freebox/Disque_2",
        },
        {
            "id": "disque1_1",
            "smb_share": "Disque 1 1",
            "mount_folder": "Disque_1_1",
            "indexed_subdir": "Vidéos",
            "rpi4_cifs_path": "/mnt/freebox/Disque_1_1",
        },
    ]


def _freebox_local_library_path(disk: dict[str, Any]) -> str:
    """Returns local CIFS path scoped strictly to the disk's eligible media subdirectory."""
    sub = str(disk.get("indexed_subdir") or "").strip("/")
    base = _user_media_path("Freebox", disk["mount_folder"])
    return str(Path(base) / sub) if sub else base


def _freebox_rpi4_library_path(disk: dict[str, Any]) -> str:
    """Returns RPi4 CIFS path scoped strictly to the disk's eligible media subdirectory."""
    sub = str(disk.get("indexed_subdir") or "").strip("/")
    base = str(disk["rpi4_cifs_path"])
    return str(Path(base) / sub) if sub else base


def _build_freebox_cifs_profiles() -> list[dict[str, Any]]:
    """Builds local and RPi4 CIFS profile descriptors scoped to official media libraries."""
    specs = _get_freebox_disk_specs()
    local_profiles = [
        {
            "id": d["id"],
            "name": f"Montage CIFS {d['smb_share']}",
            "protocol": "local_fs",
            "target": _freebox_local_library_path(d),
            "envs": ["local"],
            "status": "pending",
        }
        for d in specs
    ]
    rpi4_profiles = [
        {
            "id": f"rpi4_{d['id']}",
            "name": f"Montage CIFS RPi4 {d['smb_share']}",
            "protocol": "local_fs",
            "target": _freebox_rpi4_library_path(d),
            "envs": ["rpi4"],
            "status": "pending",
        }
        for d in specs
    ]
    return local_profiles + rpi4_profiles


def _resolve_freebox_cifs_mount(profile_id: str) -> tuple[str, str] | None:
    """Maps a Freebox local or RPi4 CIFS profile ID to (display_name, scoped_library_path)."""
    for disk in _get_freebox_disk_specs():
        if profile_id == disk["id"]:
            return f"Montage CIFS {disk['smb_share']}", _freebox_local_library_path(disk)
        if profile_id == f"rpi4_{disk['id']}":
            return f"Montage CIFS RPi4 {disk['smb_share']}", _freebox_rpi4_library_path(disk)
    return None
