"""Synology FTP MLSD discovery (with TTL-bounded index cache), target lists, and retry/exclusion filters."""

from __future__ import annotations

import contextlib
import ftplib
import json
import posixpath
import sys
import time
import unicodedata
from pathlib import Path
from typing import Any, Dict, Iterator, List, Set, Tuple

from ._facade import facade_attr
from .models import (
    DEFAULT_FTP_INDEX_MAX_AGE_SECS,
    DEFAULT_FTP_INDEX_REL_PATH,
    DEFAULT_FTP_PORT,
    DEFAULT_NAS_BASE,
    FTP_CONNECT_TIMEOUT_SECS,
    FTP_LIST_ATTEMPTS,
    FTP_LIST_RETRY_DELAY_SECS,
    FTP_SKIPPED_NAMES,
    MAX_FAILED_ATTEMPTS,
    REPO_ROOT,
    VIDEO_EXTENSIONS,
    VideoTarget,
)


def is_ftp_index_fresh(index_path: Path, max_age_secs: float = DEFAULT_FTP_INDEX_MAX_AGE_SECS) -> bool:
    """Returns True only when `nas_ftp_index.json` exists and is younger than `max_age_secs`."""
    try:
        age = facade_attr("time", time).time() - index_path.stat().st_mtime
    except OSError:
        return False
    return age <= max_age_secs


def load_nas_ftp_index_targets(index_path: Path, base_dir: str) -> List[VideoTarget]:
    """Loads video targets from `nas_ftp_index.json` filtered by `base_dir`."""
    if not index_path.is_file():
        return []
    try:
        raw = json.loads(index_path.read_text(encoding="utf-8"))
        entries = raw.get("index", raw)
        clean_base = base_dir.strip("/")
        targets: List[VideoTarget] = []
        for filename, rel in entries.items():
            clean_rel = str(rel).lstrip("/")
            if clean_base and clean_base != "all":
                if not (clean_rel.startswith(clean_base + "/") or clean_rel == clean_base):
                    continue
            targets.append(
                VideoTarget(
                    rel_path="/" + clean_rel,
                    filename=unicodedata.normalize("NFC", str(filename)),
                    size_bytes=0,
                )
            )
        return targets
    except Exception as err:
        print(f"⚠️ Failed reading {index_path}: {err}", file=sys.stderr)
        return []


def _list_ftp_dir(ftp: ftplib.FTP, path: str) -> List[Tuple[str, Dict[str, str]]]:
    """MLSD entries of `path` without hidden or Synology system directories."""
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
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    base_dir: str,
    ftp_port: int,
    max_depth: int,
) -> List[VideoTarget]:
    """Walks Synology NAS directories via FTP MLSD, preserving raw FTP paths and NFC media names."""
    ftp = ftplib.FTP()
    ftp.connect(nas_host, ftp_port, timeout=FTP_CONNECT_TIMEOUT_SECS)
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
                results.append(VideoTarget(rel, unicodedata.normalize("NFC", name), int(facts.get("size", 0))))

    with contextlib.suppress(Exception):
        ftp.quit()
    return results


def list_nas_videos_ftp(
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    base_dir: str = DEFAULT_NAS_BASE,
    ftp_port: int = DEFAULT_FTP_PORT,
    max_depth: int = 8,
    bypass_index_cache: bool = False,
) -> List[VideoTarget]:
    """Discovers NAS videos via fresh index cache (`<= 1 h`) or live FTP MLSD walk.

    When `bypass_index_cache` is True (e.g. `--fresh-only`) or `nas_ftp_index.json` is older than
    `DEFAULT_FTP_INDEX_MAX_AGE_SECS`, queries live FTP MLSD so newly downloaded media are never masked.
    """
    index_path = facade_attr("REPO_ROOT", REPO_ROOT) / DEFAULT_FTP_INDEX_REL_PATH
    loader = facade_attr("load_nas_ftp_index_targets", load_nas_ftp_index_targets)
    if not bypass_index_cache and is_ftp_index_fresh(index_path):
        cached = loader(index_path, base_dir)
        if cached:
            print(f"✓ Loaded {len(cached)} targets from fresh {index_path.name} (base: {base_dir}).")
            return cached

    walker = facade_attr("_walk_nas_videos_ftp", _walk_nas_videos_ftp)
    time_mod = facade_attr("time", time)
    for attempt in range(1, FTP_LIST_ATTEMPTS + 1):
        try:
            return walker(nas_host, nas_user, nas_pass, base_dir, ftp_port, max_depth)
        except (OSError, EOFError, ftplib.error_temp, ftplib.error_reply, ftplib.error_proto) as err:
            if attempt == FTP_LIST_ATTEMPTS:
                raise
            print(f"⚠️ NAS FTP listing failed ({err}), retry {attempt}/{FTP_LIST_ATTEMPTS - 1}", file=sys.stderr)
            time_mod.sleep(FTP_LIST_RETRY_DELAY_SECS * attempt)
    return []


def load_targets_file(targets_path: Path, nas_base: str, skip_stems: Set[str]) -> List[VideoTarget]:
    """Loads explicit targets (paths or `{path, filename, stem, size_bytes}` dicts), rebasing on `nas_base`."""
    targets: List[VideoTarget] = []
    for item in json.loads(targets_path.read_text(encoding="utf-8")):
        if isinstance(item, dict):
            p_str = str(item.get("path", ""))
            filename = str(item.get("filename", Path(p_str).name))
            stem = str(item.get("stem", Path(p_str).stem))
            size_bytes = int(item.get("size_bytes", 0))
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
    """Yields `(NFC video stem, record)` for each parseable JSONL event, oldest first."""
    if not events_log.is_file():
        return
    for line in events_log.read_text(encoding="utf-8").splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        stem = unicodedata.normalize("NFC", str(rec.get("video_stem", "")))
        if stem:
            yield stem, rec


def resolve_retry_targets(events_log: Path, skip_stems: Set[str]) -> List[VideoTarget]:
    """Returns videos whose last outcome in `events_log` failed, excluding `skip_stems`."""
    last_outcome: Dict[str, Tuple[bool, VideoTarget]] = {}
    for stem, rec in iter_events(events_log):
        rel_path = str(rec.get("video_rel_path", ""))
        if rel_path:
            item = VideoTarget(rel_path, Path(rel_path).name, int(rec.get("file_size_bytes", 0) or 0))
            last_outcome[stem] = (bool(rec.get("success", False)), item)
    return [item for stem, (ok, item) in last_outcome.items() if not ok and stem not in skip_stems]


def load_blacklisted_stems(events_log: Path, max_attempts: int = MAX_FAILED_ATTEMPTS) -> Set[str]:
    """Returns NFC stems whose last `max_attempts` outcomes in `events_log` are consecutive failures."""
    streaks: Dict[str, int] = {}
    for stem, rec in iter_events(events_log):
        streaks[stem] = 0 if rec.get("success") else streaks.get(stem, 0) + 1
    return {stem for stem, count in streaks.items() if count >= max_attempts}


def load_excluded_prefixes(exclude_file: Path) -> List[str]:
    """Reads NFC NAS path prefixes from `exclude_file`, ignoring blank lines and `#` comments."""
    if not exclude_file.is_file():
        return []
    lines = (line.strip() for line in exclude_file.read_text(encoding="utf-8").splitlines())
    return [unicodedata.normalize("NFC", line) for line in lines if line and not line.startswith("#")]


def is_excluded(rel_path: str, prefixes: List[str]) -> bool:
    """Returns True when the NFC form of `rel_path` starts with any prefix in `prefixes`."""
    return unicodedata.normalize("NFC", rel_path).startswith(tuple(prefixes))


def match_nas_paths(targets: List[VideoTarget], nas_videos: List[VideoTarget]) -> List[VideoTarget]:
    """Replaces targets with authoritative FTP NAS entries matching their NFC stem."""
    by_stem = {Path(video.filename).stem: video for video in nas_videos}
    matched = (by_stem.get(unicodedata.normalize("NFC", Path(target.filename).stem)) for target in targets)
    return list(dict.fromkeys(video for video in matched if video))
