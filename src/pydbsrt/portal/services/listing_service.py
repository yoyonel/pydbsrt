"""File listing, directory scanning, multi-token filtering, sorting, and two-tier caching."""

from __future__ import annotations

import ftplib
import json
import os
import time
import unicodedata
import urllib.request
from pathlib import Path
from typing import Any

from pydbsrt.portal import config as portal_config
from pydbsrt.portal.cache import _LISTING_CACHE, _read_disk_cache, _write_disk_cache
from pydbsrt.portal.config import (
    CACHE_LISTING_TTL_SECONDS,
    DEFAULT_FREEBOX_GATEWAY_PORT,
    DEFAULT_FREEBOX_HOST,
    DEFAULT_FREEBOX_USER,
    DEFAULT_FTP_PORT,
    DEFAULT_NAS_LAN_HOST,
    DEFAULT_NAS_USER,
    DEFAULT_NAS_WAN_HOST,
    DEFAULT_PAGE_SIZE,
    EXCLUDED_FREEBOX_DIR_NAMES,
    FREEBOX_FTP_ELIGIBLE_DIRS,
    FTP_LISTING_TIMEOUT_MULTIPLIER,
    GATEWAY_CATALOG_TIMEOUT_SECS,
    MAX_PAGE_SIZE,
    PROBE_FTP_TIMEOUT_SECS,
    REPO_ROOT,
    _resolve_data_path,
    format_human_size,
    parse_media_filename,
)
from pydbsrt.portal.services import secrets_env
from pydbsrt.portal.services.ftp_parsers import _parse_ftp_directory_items
from pydbsrt.portal.services.probe_service import _count_cached_freebox_targets


def _scan_local_directory(target_dir: Path, clean_sub: str) -> list[dict[str, Any]]:
    """Scans local filesystem directory for video containers and subdirectories."""
    items: list[dict[str, Any]] = []
    if not target_dir.is_dir():
        return items
    try:
        for entry in os.scandir(target_dir):
            if entry.name.startswith(".") or entry.name.lower() in EXCLUDED_FREEBOX_DIR_NAMES:
                continue
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
            elif secrets_env._is_video(entry):
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


def _fetch_cifs_items(source_id: str, profile_id: str, clean_sub: str) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Resolves CIFS mount directory and scans files safely."""
    if source_id == "synology-video":
        chan_name, mount_path = "Montage CIFS Local", secrets_env._user_media_path("Synology", "video")
    elif source_id == "synology-tvshow":
        chan_name, mount_path = "Montage CIFS Local", secrets_env._user_media_path("Synology", "tvshow")
    else:
        cifs_hit = secrets_env._resolve_freebox_cifs_mount(profile_id)
        if cifs_hit is None:
            return "Inconnu", "unknown", "", []
        chan_name, mount_path = cifs_hit

    if not secrets_env._is_cifs_safe_to_stat():
        return chan_name, "local_fs", mount_path, []

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
    passwd = secrets_env.resolve_nas_password()
    host = DEFAULT_NAS_LAN_HOST if profile_id == "lan_ftp" else DEFAULT_NAS_WAN_HOST
    chan_name = "Streaming LAN FTP" if profile_id == "lan_ftp" else "Streaming WAN FTP"
    base_target = f"ftp://{DEFAULT_NAS_USER}@{host}:{DEFAULT_FTP_PORT}{base_dir}"
    ftp_timeout = PROBE_FTP_TIMEOUT_SECS * FTP_LISTING_TIMEOUT_MULTIPLIER

    items: list[dict[str, Any]] = []
    try:
        ftp = ftplib.FTP(timeout=ftp_timeout)
        ftp.connect(host, DEFAULT_FTP_PORT, timeout=ftp_timeout)
        ftp.login(DEFAULT_NAS_USER, passwd)
        ftp.cwd(target_dir)
        items = _parse_ftp_directory_items(ftp, clean_sub)
        ftp.quit()
    except Exception:
        pass
    return chan_name, "ftp", base_target, items


def _is_eligible_freebox_ftp_subpath(clean_sub: str) -> bool:
    """Checks that a Freebox FTP subpath lies inside an official library and avoids excluded folders."""
    norm_sub = unicodedata.normalize("NFC", clean_sub.strip("/"))
    if not any(norm_sub == p or norm_sub.startswith(f"{p}/") for p in FREEBOX_FTP_ELIGIBLE_DIRS):
        return False
    return not any(part.lower() in EXCLUDED_FREEBOX_DIR_NAMES for part in norm_sub.split("/"))


def _fetch_freebox_ftp_items(clean_sub: str) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves items via FTP from Freebox strictly scoped to official media libraries."""
    chan_name = "FTP LAN Freebox"
    base_target = f"ftp://{DEFAULT_FREEBOX_USER}@{DEFAULT_FREEBOX_HOST}:{DEFAULT_FTP_PORT}"
    if not clean_sub:
        root_dirs = [
            {
                "name": rel_dir,
                "path": rel_dir,
                "size_bytes": 0,
                "size_human": "-",
                "is_dir": True,
                "modify": "",
                "extension": "",
                "parsed": {"title": rel_dir},
            }
            for rel_dir in FREEBOX_FTP_ELIGIBLE_DIRS
        ]
        return chan_name, "ftp", base_target, root_dirs

    if not _is_eligible_freebox_ftp_subpath(clean_sub):
        return chan_name, "ftp", base_target, []

    passwd = secrets_env.resolve_freebox_password()
    ftp_timeout = PROBE_FTP_TIMEOUT_SECS * FTP_LISTING_TIMEOUT_MULTIPLIER
    items: list[dict[str, Any]] = []
    try:
        ftp = ftplib.FTP(timeout=ftp_timeout)
        ftp.connect(DEFAULT_FREEBOX_HOST, DEFAULT_FTP_PORT, timeout=ftp_timeout)
        ftp.login(DEFAULT_FREEBOX_USER, passwd)
        ftp.cwd(f"/{clean_sub.strip('/')}")
        raw_items = _parse_ftp_directory_items(ftp, clean_sub)
        items = [it for it in raw_items if str(it.get("name", "")).lower() not in EXCLUDED_FREEBOX_DIR_NAMES]
        ftp.quit()
    except Exception:
        pass
    return chan_name, "ftp", base_target, items


def _fetch_gateway_catalog_http(base_target: str) -> list[dict[str, Any]]:
    """Queries live GET /catalog from the Freebox HTTP Range Gateway."""
    catalog_url = f"{base_target.rstrip('/')}/catalog"
    try:
        req = urllib.request.Request(catalog_url, headers={"User-Agent": "dbsrt-portal/1.0"})
        with urllib.request.urlopen(req, timeout=GATEWAY_CATALOG_TIMEOUT_SECS) as resp:
            if resp.headers.get_content_type() != "application/json":
                return []
            payload = json.loads(resp.read().decode("utf-8"))
            raw_items = payload.get("items", []) if isinstance(payload, dict) else []
            return raw_items if isinstance(raw_items, list) else []
    except Exception:
        return []


def _load_cached_freebox_targets_list() -> list[dict[str, Any]]:
    """Loads fallback target descriptors from freebox_batch_targets.json when non-empty."""
    targets_file = _resolve_data_path("data/reports/freebox_batch_targets.json")
    if not targets_file.is_file():
        return []
    try:
        parsed = json.loads(targets_file.read_text(encoding="utf-8"))
        return parsed if isinstance(parsed, list) else []
    except (OSError, ValueError, TypeError):
        return []


def _has_isolated_cwd_targets_override() -> bool:
    """True when running in an isolated test CWD containing a non-empty freebox_batch_targets.json."""
    try:
        if Path.cwd().resolve() == REPO_ROOT.resolve():
            return False
    except OSError:
        return False
    cwd_targets = Path("data/reports/freebox_batch_targets.json")
    return cwd_targets.is_file() and _count_cached_freebox_targets() > 0


def _format_gateway_target_item(t: dict[str, Any], norm_sub: str) -> dict[str, Any] | None:
    """Formats a single Freebox gateway /catalog item when matching subdir filter."""
    rel_p = str(t.get("path", ""))
    if not rel_p or (norm_sub and not unicodedata.normalize("NFC", rel_p).startswith(norm_sub)):
        return None
    fname = str(t.get("filename") or Path(rel_p).name)
    sb = int(t.get("size_bytes", 0) or 0)
    mtime_raw = int(t.get("mtime", 0) or 0)
    mod_str = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(mtime_raw)) if mtime_raw > 0 else "-"
    return {
        "name": fname,
        "path": rel_p,
        "size_bytes": sb,
        "size_human": format_human_size(sb) if sb > 0 else "-",
        "is_dir": False,
        "modify": mod_str,
        "extension": Path(fname).suffix.lower(),
        "parsed": parse_media_filename(fname),
    }


def _fetch_freebox_http_gateway_items(profile_id: str, clean_sub: str) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves eligible Freebox media items from live gateway /catalog with local JSON fallback."""
    tgt_map = {
        "wan_http": ("Passerelle HTTP WAN", portal_config.DEFAULT_GATEWAY_WAN_URL.replace("/health", "")),
        "lan_http": ("Passerelle HTTP LAN (RPi4)", portal_config.DEFAULT_GATEWAY_LAN_URL.replace("/health", "")),
    }
    chan_name, base_tgt = tgt_map.get(
        profile_id, ("Passerelle HTTP Locale (RPi4)", f"http://127.0.0.1:{DEFAULT_FREEBOX_GATEWAY_PORT}")
    )

    if _has_isolated_cwd_targets_override():
        targets = _load_cached_freebox_targets_list()
    else:
        targets = _fetch_gateway_catalog_http(base_tgt) or _load_cached_freebox_targets_list()

    norm_sub = unicodedata.normalize("NFC", clean_sub)
    items = [fmt for t in targets if (fmt := _format_gateway_target_item(t, norm_sub)) is not None]
    return chan_name, "http_gateway", base_tgt, items


def _fetch_raw_file_list(
    source_id: str, profile_id: str, subdir: str = ""
) -> tuple[str, str, str, list[dict[str, Any]]]:
    """Retrieves raw file listing from local filesystem, CIFS, FTP, or HTTP gateway."""
    clean_sub = subdir.strip("/")

    if source_id == "local-snippets" and profile_id == "local":
        base_dir = _resolve_data_path("data/snippets")
        target_dir = base_dir if not clean_sub else base_dir / clean_sub
        items = _scan_local_directory(target_dir, clean_sub)
        return "Système de Fichiers Local", "local_fs", str(base_dir), items

    if profile_id == "cifs" or secrets_env._resolve_freebox_cifs_mount(profile_id) is not None:
        return _fetch_cifs_items(source_id, profile_id, clean_sub)

    if profile_id in ("lan_ftp", "wan_ftp") and source_id in ("synology-video", "synology-tvshow"):
        return _fetch_synology_ftp_items(source_id, profile_id, clean_sub)

    if profile_id == "lan_ftp" and source_id == "freebox-disks":
        return _fetch_freebox_ftp_items(clean_sub)

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


def _item_matches_token(item: dict[str, Any], tok: str) -> bool:
    """Checks whether a single listing item matches a lowercase search token."""
    if tok in item["name"].lower():
        return True
    parsed = item.get("parsed", {})
    for field in ("title", "year", "resolution", "codec"):
        val = parsed.get(field)
        if val and tok in str(val).lower():
            return True
    return False


def _filter_listing_items(raw_items: list[dict[str, Any]], query_str: str) -> list[dict[str, Any]]:
    """Applies smart token filtering over name, title, year, resolution, and codec."""
    q = query_str.strip().lower()
    if not q:
        return raw_items
    tokens = q.split()
    return [it for it in raw_items if all(_item_matches_token(it, tok) for tok in tokens)]


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
