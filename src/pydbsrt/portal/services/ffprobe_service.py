"""Multimedia stream inspection service via ffprobe with two-tier caching."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
import unicodedata
import urllib.parse
from pathlib import Path
from typing import Any

from pydbsrt.portal.cache import _DETAILS_CACHE, _read_disk_cache, _write_disk_cache
from pydbsrt.portal.config import (
    BITS_PER_MEGABIT,
    CACHE_DETAILS_TTL_SECONDS,
    DEFAULT_FREEBOX_GATEWAY_PORT,
    DEFAULT_FREEBOX_HOST,
    DEFAULT_FREEBOX_USER,
    DEFAULT_FREEBOX_WAN_HOST,
    DEFAULT_FREEBOX_WAN_PORT,
    DEFAULT_FTP_PORT,
    DEFAULT_NAS_LAN_HOST,
    DEFAULT_NAS_USER,
    DEFAULT_NAS_WAN_HOST,
    DEFAULT_RPI_LAN_HOST,
    FFPROBE_TIMEOUT_SECONDS,
    _resolve_data_path,
    format_human_size,
    format_seconds_duration,
    parse_media_filename,
)
from pydbsrt.portal.services import secrets_env


def _resolve_synology_endpoint(source_id: str, profile_id: str, clean_path: str) -> tuple[bool, bool, str, str, str]:
    """Resolves local CIFS or remote FTP endpoint for Synology media."""
    if profile_id == "cifs":
        sub = "video" if source_id == "synology-video" else "tvshow"
        return True, False, str(Path(secrets_env._user_media_path("Synology", sub)) / clean_path), "", ""
    if profile_id in ("lan_ftp", "wan_ftp"):
        base_dir = "/video" if source_id == "synology-video" else "/tvshow"
        host = DEFAULT_NAS_LAN_HOST if profile_id == "lan_ftp" else DEFAULT_NAS_WAN_HOST
        return (
            False,
            True,
            "",
            f"ftp://{host}:{DEFAULT_FTP_PORT}{base_dir}/{clean_path}",
            secrets_env.resolve_nas_password(),
        )
    return False, False, "", "", ""


def _resolve_freebox_endpoint(profile_id: str, clean_path: str) -> tuple[bool, bool, str, str, str]:
    """Resolves local/RPi4 CIFS, LAN FTP, or HTTP range gateway for Freebox media."""
    cifs_hit = secrets_env._resolve_freebox_cifs_mount(profile_id)
    if cifs_hit is not None:
        mount_dir = Path(cifs_hit[1])
        leaf_prefix = f"{mount_dir.name}/"
        norm_clean = unicodedata.normalize("NFC", clean_path)
        norm_prefix = unicodedata.normalize("NFC", leaf_prefix)
        rel_sub = clean_path[len(leaf_prefix) :] if norm_clean.startswith(norm_prefix) else clean_path
        return True, False, str(mount_dir / rel_sub), "", ""
    if profile_id == "lan_ftp":
        return (
            False,
            True,
            "",
            f"ftp://{DEFAULT_FREEBOX_HOST}:{DEFAULT_FTP_PORT}/{clean_path}",
            secrets_env.resolve_freebox_password(),
        )
    quoted_path = "/".join(urllib.parse.quote(seg) for seg in clean_path.split("/"))
    if profile_id == "wan_http":
        return False, False, "", f"http://{DEFAULT_FREEBOX_WAN_HOST}:{DEFAULT_FREEBOX_WAN_PORT}/{quoted_path}", ""
    if profile_id == "lan_http":
        return False, False, "", f"http://{DEFAULT_RPI_LAN_HOST}:{DEFAULT_FREEBOX_GATEWAY_PORT}/{quoted_path}", ""
    if profile_id == "rpi4_gateway":
        return False, False, "", f"http://127.0.0.1:{DEFAULT_FREEBOX_GATEWAY_PORT}/{quoted_path}", ""
    return False, False, "", "", ""


def _resolve_target_endpoint(source_id: str, profile_id: str, clean_path: str) -> tuple[bool, bool, str, str, str]:
    """Resolves local vs remote target path, ftp url, and credentials."""
    if source_id == "local-snippets" and profile_id == "local":
        return True, False, str(_resolve_data_path("data/snippets") / clean_path), "", ""
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


def _build_ffprobe_command(
    is_local: bool, is_ftp: bool, abs_path: str, ftp_url: str, passwd: str, ffprobe_bin: str
) -> list[str]:
    """Builds the ffprobe CLI command for local, FTP, or HTTP Range targets."""
    base_flags = ["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]
    if is_local and Path(abs_path).is_file():
        return [ffprobe_bin, *base_flags, abs_path]
    if is_ftp and passwd:
        ftp_user = DEFAULT_FREEBOX_USER if "mafreebox" in ftp_url else DEFAULT_NAS_USER
        return [ffprobe_bin, "-v", "error", "-ftp-user", ftp_user, "-ftp-password", passwd, *base_flags[2:], ftp_url]
    if ftp_url.startswith("http://"):
        return [ffprobe_bin, *base_flags, ftp_url]
    return []


def _execute_ffprobe(
    is_local: bool, is_ftp: bool, abs_path: str, ftp_url: str, passwd: str, ffprobe_bin: str
) -> tuple[dict[str, Any] | None, str]:
    """Runs ffprobe command and returns parsed JSON or error message."""
    cmd = _build_ffprobe_command(is_local, is_ftp, abs_path, ftp_url, passwd, ffprobe_bin)
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
            if isinstance(cached_details, dict):
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
    details["bit_rate_human"] = f"{bitrate / BITS_PER_MEGABIT:.2f} Mb/s" if bitrate > 0 else "-"

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
        "coupling": secrets_env._lookup_coupling(filename),
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
