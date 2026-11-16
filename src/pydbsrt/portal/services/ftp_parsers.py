"""FTP directory parsers (MLSD, UNIX LIST, NLST fallback) with exclusion filtering."""

from __future__ import annotations

import ftplib
from pathlib import Path
from typing import Any

from pydbsrt.portal.config import (
    FTP_LIST_REGEX,
    FTP_MODIFY_TIMESTAMP_MIN_LEN,
    VIDEO_EXTENSIONS,
    _is_excluded_portal_entry,
    format_human_size,
    parse_media_filename,
)


def _parse_mlsd_item(name: str, facts: dict[str, Any], clean_sub: str) -> dict[str, Any] | None:
    """Parses a single MLSD item into an entry dict or None if filtered out."""
    if _is_excluded_portal_entry(name):
        return None
    rel = name if not clean_sub else f"{clean_sub}/{name}"
    ftype = facts.get("type", "file")
    size = int(facts.get("size", 0))
    raw_mod = facts.get("modify", "")
    mod_str = ""
    if len(raw_mod) >= FTP_MODIFY_TIMESTAMP_MIN_LEN:
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
    ext = Path(name).suffix.lower()
    if ext in VIDEO_EXTENSIONS:
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
    if _is_excluded_portal_entry(name):
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


def _parse_ftp_list_line(line: str, clean_sub: str) -> dict[str, Any] | None:
    """Parses single UNIX LIST output line into structured item dict."""
    m = FTP_LIST_REGEX.match(line.strip())
    if not m:
        return None
    perms, size_str, name = m.groups()
    if _is_excluded_portal_entry(name):
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
