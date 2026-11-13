#!/usr/bin/env python3
"""Backfill `<stem>.json` sidecars for existing `.ahash` binaries using `dbsrt ahash:sidecar`.

Resolves media identity via `<hash>.json` (in `__AHASH__/` or `__PHASH__/`) or filename stem,
maps media locations to remote streaming URIs (Synology NAS FTP / Freebox HTTP WAN / local
test files) strictly by string parsing (never calling `stat()`/`exists()` on local CIFS mount paths),
and invokes `dbsrt ahash:sidecar` concurrently (bounded <= 4 workers for CloudLinux LVE safety).
Credentials (`DBSRT_FTP_USER`, `DBSRT_FTP_PASSWORD`, `FTP_USER`, `FTP_PASSWORD`) are injected
only via child process environment variables, never in URLs or command-line arguments.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import ftplib
import hashlib
import json
import os
import posixpath
import re
import subprocess
import sys
import time
import unicodedata
import urllib.parse
from collections import Counter
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Tuple


def _find_repo_root() -> Path:
    curr = Path(__file__).resolve().parent
    for parent in [curr, *curr.parents]:
        if (parent / ".git").is_dir():
            return parent
    appliance_dir = Path.home() / "dbsrt"
    return appliance_dir if appliance_dir.is_dir() else Path.cwd()


REPO_ROOT: Path = _find_repo_root()
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

try:
    from scripts.workers import o2switch_batch_worker as _worker
except ImportError:
    import o2switch_batch_worker as _worker  # type: ignore[import-not-found,no-redef]

# Operational Constants (Zero Magic Numbers)
DEFAULT_AHASH_DIR: Path = REPO_ROOT / "data/__AHASH__"
DEFAULT_PHASH_DIR: Path = REPO_ROOT / "data/__PHASH__"
DEFAULT_REPORTS_DIR: Path = REPO_ROOT / "data/reports"
DEFAULT_DBSRT_BIN: Path = REPO_ROOT / "target/release/dbsrt"
DEFAULT_FTP_PORT: int = 21
DEFAULT_FTP_TIMEOUT_SECS: float = 20.0
DEFAULT_BACKFILL_WORKERS: int = 2
MAX_BACKFILL_WORKERS: int = 4
DEFAULT_FFMPEG_THREADS: str = "2"
DEFAULT_SUBPROCESS_TIMEOUT_SECS: int = 180
SHA256_CHUNK_BYTES: int = 65536
SYNO_PATH_MARKERS: Tuple[str, ...] = ("/Synology/", "/synology/")
FREEBOX_PATH_MARKERS: Tuple[str, ...] = ("/Freebox/", "/freebox/")
FTP_SCAN_ROOTS: Tuple[str, ...] = ("/video", "/tvshow")
FTP_SKIP_ENTRIES: frozenset[str] = frozenset({".", "..", "EMPTY_DIRS", "SRT", "@eaDir", "#recycle"})
VIDEO_EXTENSIONS: Tuple[str, ...] = (".mkv", ".mp4", ".webm", ".avi", ".mov", ".m4v")
CERTAIN_OR_VERIFIED_SELECTIONS: frozenset[str] = frozenset({"explicit", "single_track", "verified_backfill"})
UPGRADEABLE_SELECTIONS: frozenset[str] = frozenset({"ffmpeg_default", "inferred_backfill", "unknown"})
MOVIEHASH_RE = re.compile(r"[0-9a-fA-F]{16}")
AHASH_FPS_SUFFIX_RE = re.compile(r"\.\d+(?:\.\d+)?fps$")
AUDIO_TRACK_TAG_RE = re.compile(r"\.a\d+$")
SIDECAR_KEYS_FILENAME: Tuple[str, ...] = ("media_name", "clean_title", "filename")
SELECTION_COMPLETENESS_RANK: Dict[str, int] = {
    "explicit": 4,
    "verified_backfill": 3,
    "single_track": 3,
    "inferred_backfill": 2,
    "ffmpeg_default": 2,
    "unknown": 1,
}
SIDECAR_FILLABLE_KEYS: Tuple[str, ...] = (
    "schema_version",
    "media_name",
    "fps",
    "frames",
    "duration_sec",
    "header",
    "selection",
    "selected_audio_stream",
    "container_audio_streams",
    "extracted_at",
    "source_id",
    "source_rel_path",
    "verified_distance",
)


@dataclass(frozen=True)
class SourceLocation:
    """Logical origin of a source media file without any embedded credentials."""

    source_id: str
    rel_path: str
    transport: str  # "ftp" | "http" | "local"


@dataclass(frozen=True)
class SourceEnvConfig:
    """Environment-derived endpoint & credential lookup configuration (zero hardcoded secrets)."""

    nas_host: str
    nas_user: str
    nas_pass_entry: str
    explicit_nas_pass: str
    freebox_base: str


@dataclass(frozen=True)
class AhashTargetItem:
    """Resolved unit of backfill work for one `.ahash` file."""

    ahash_path: Path
    sidecar_path: Path
    logical_stem: str
    moviehash: str | None
    media_name: str
    existing_selection: str | None
    existing_track_count: int
    location: SourceLocation | None
    skip_reason: str | None
    existing_language: str | None = None
    existing_audio_index: int | None = None
    existing_verified_distance: float | None = None


@dataclass(frozen=True)
class MigrationCandidate:
    """Legacy non-16-hex `.ahash` file queued for lossless `<moviehash_16hex>` migration."""

    ahash_path: Path
    sidecar_path: Path
    logical_stem: str
    base_stem: str
    track_tag: str
    fps_suffix: str
    media_name: str
    typed_sidecar: Dict[str, Any] | None
    location: SourceLocation | None


@dataclass
class BackfillRecord:
    """JSONL audit record emitted for every evaluated `.ahash` file."""

    ahash_file: str
    sidecar_file: str
    logical_stem: str
    media_name: str
    moviehash: str | None
    source_id: str | None
    source_rel_path: str | None
    status: str
    selection: str | None = None
    audio_index: int | None = None
    language: str | None = None
    track_count: int | None = None
    verified_distance: float | None = None
    elapsed_sec: float = 0.0
    error: str | None = None


def load_env_files(repo_root: Path) -> Dict[str, str]:
    """Loads unversioned `.env` / `etc/.nas_config` files plus live process env."""
    merged: Dict[str, str] = {}
    for rel in (".env", "etc/.env", "etc/.nas_config"):
        candidate = repo_root / rel
        if not candidate.is_file():
            continue
        try:
            for raw_line in candidate.read_text(encoding="utf-8").splitlines():
                line = raw_line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                k, v = line.split("=", 1)
                merged[k.strip()] = v.strip().strip("'\"")
        except OSError:
            continue
    merged.update(os.environ)
    return merged


def _first_nonempty(env_map: Dict[str, str], *keys: str) -> str:
    for key in keys:
        val = env_map.get(key, "").strip()
        if val:
            return val
    return ""


def build_source_env_config(repo_root: Path, env_map: Dict[str, str] | None = None) -> SourceEnvConfig:
    """Constructs endpoint configuration solely from env / `.env` files (zero hardcoded hosts)."""
    cfg = env_map if env_map is not None else load_env_files(repo_root)
    nas_host = _first_nonempty(cfg, "DBSRT_NAS_HOST", "NAS_HOST")
    nas_user = _first_nonempty(cfg, "DBSRT_NAS_USER", "NAS_USER", "DBSRT_FTP_USER", "FTP_USER")
    nas_pass_entry = _first_nonempty(cfg, "NAS_PASS_ENTRY")
    explicit_pass = _first_nonempty(cfg, "DBSRT_NAS_PASS", "NAS_PASS", "DBSRT_FTP_PASSWORD", "FTP_PASSWORD")

    freebox_base = _first_nonempty(cfg, "DBSRT_FREEBOX_HTTP_BASE", "FREEBOX_STREAM_BASE")
    if not freebox_base:
        wan_host = _first_nonempty(cfg, "FREEBOX_WAN_HOST")
        wan_port = _first_nonempty(cfg, "FREEBOX_GATEWAY_WAN_PORT")
        if wan_host:
            freebox_base = f"http://{wan_host}:{wan_port}" if wan_port else f"http://{wan_host}"

    pass_file = repo_root / "etc/.nas_pass"
    if not explicit_pass and pass_file.is_file():
        try:
            explicit_pass = pass_file.read_text(encoding="utf-8").strip()
        except OSError:
            pass

    return SourceEnvConfig(
        nas_host=nas_host,
        nas_user=nas_user,
        nas_pass_entry=nas_pass_entry,
        explicit_nas_pass=explicit_pass,
        freebox_base=freebox_base.rstrip("/"),
    )


def resolve_nas_password(cfg: SourceEnvConfig) -> str:
    """Resolves NAS FTP password lazily from env or `pass show <NAS_PASS_ENTRY>`."""
    if cfg.explicit_nas_pass:
        return cfg.explicit_nas_pass
    if not cfg.nas_pass_entry:
        return ""
    try:
        res = subprocess.run(
            ["pass", "show", cfg.nas_pass_entry],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        if res.returncode == 0:
            return res.stdout.strip().splitlines()[0] if res.stdout.strip() else ""
    except OSError:
        pass
    return ""


def ahash_logical_stem(file_name: str) -> str:
    """Matches `dbsrt_core::ahash_logical_stem`: strips `.ahash` and trailing `.<fps>fps`."""
    without_ext = file_name.removesuffix(".ahash")
    return AHASH_FPS_SUFFIX_RE.sub("", without_ext)


def _strip_audio_track_tag(stem: str) -> str:
    return AUDIO_TRACK_TAG_RE.sub("", stem)


def _append_normalized_forms(seen: set[str], out: List[str], token: str) -> None:
    for candidate in (token, unicodedata.normalize("NFC", token), token.lower()):
        if candidate and candidate not in seen:
            seen.add(candidate)
            out.append(candidate)


def _normalize_lookup_keys(name: str) -> List[str]:
    """Returns normalized candidate lookup keys for matching filenames across catalogs."""
    if not name:
        return []
    keys: List[str] = []
    seen: set[str] = set()
    for variant in (name, urllib.parse.unquote(name)):
        base = posixpath.basename(variant.strip())
        if not base:
            continue
        _append_normalized_forms(seen, keys, base)
        _append_normalized_forms(seen, keys, _strip_video_ext(base))
    return keys


def _strip_video_ext(name: str) -> str:
    lower = name.lower()
    for ext in VIDEO_EXTENSIONS:
        if lower.endswith(ext):
            return name[: -len(ext)]
    return name


def _index_insert(index: Dict[str, SourceLocation], filename: str, loc: SourceLocation) -> None:
    for key in _normalize_lookup_keys(filename):
        index[key] = loc


def parse_storage_string(raw_path: str) -> SourceLocation | None:
    """Pure string parser translating path strings into `SourceLocation` without any `stat()`."""
    clean = raw_path.strip()
    if not clean:
        return None
    for marker in SYNO_PATH_MARKERS:
        if marker in clean:
            rel = clean.split(marker, 1)[1].lstrip("/")
            source_id = "synology_video" if rel.startswith("video/") else "synology_tvshow"
            return SourceLocation(source_id=source_id, rel_path=rel, transport="ftp")
    for marker in FREEBOX_PATH_MARKERS:
        if marker in clean:
            rel = clean.split(marker, 1)[1].lstrip("/")
            return SourceLocation(source_id="freebox_disks", rel_path=rel, transport="http")
    norm = clean.lstrip("/")
    if norm.startswith("tvshow/"):
        return SourceLocation(source_id="synology_tvshow", rel_path=norm, transport="ftp")
    if norm.startswith("video/"):
        return SourceLocation(source_id="synology_video", rel_path=norm, transport="ftp")
    return None


def _load_json_safe(path: Path) -> object:
    if not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def _ingest_nas_ftp_index(reports_dir: Path, index: Dict[str, SourceLocation]) -> None:
    payload = _load_json_safe(reports_dir / "nas_ftp_index.json")
    if not isinstance(payload, dict):
        return
    entries = payload.get("index", payload)
    if not isinstance(entries, dict):
        return
    for fname, rel in entries.items():
        if isinstance(fname, str) and isinstance(rel, str) and rel.strip():
            clean_rel = rel.lstrip("/")
            sid = "synology_video" if clean_rel.startswith("video/") else "synology_tvshow"
            _index_insert(index, fname, SourceLocation(source_id=sid, rel_path=clean_rel, transport="ftp"))


def _ingest_snippets_report(reports_dir: Path, index: Dict[str, SourceLocation]) -> None:
    payload = _load_json_safe(reports_dir / "snippets_fill_report.json")
    if not isinstance(payload, list):
        return
    for row in payload:
        if not isinstance(row, dict):
            continue
        fname = row.get("original_filename")
        spath = row.get("source_path")
        stype = row.get("source_type")
        if not (isinstance(fname, str) and isinstance(spath, str) and spath.strip()):
            continue
        if stype == "freebox_http":
            _index_insert(index, fname, SourceLocation("freebox_disks", spath.lstrip("/"), "http"))
        elif stype == "syno_ftp":
            clean_rel = spath.lstrip("/")
            sid = "synology_video" if clean_rel.startswith("video/") else "synology_tvshow"
            _index_insert(index, fname, SourceLocation(sid, clean_rel, "ftp"))


def _ingest_targets_list(path: Path, default_freebox: bool, index: Dict[str, SourceLocation]) -> None:
    payload = _load_json_safe(path)
    if not isinstance(payload, list):
        return
    for row in payload:
        if not isinstance(row, dict):
            continue
        raw_p = row.get("path")
        if not isinstance(raw_p, str) or not raw_p.strip():
            continue
        loc = parse_storage_string(raw_p)
        if loc is None and default_freebox:
            loc = SourceLocation("freebox_disks", raw_p.lstrip("/"), "http")
        if loc is not None:
            fname = row.get("filename") or posixpath.basename(loc.rel_path)
            _index_insert(index, str(fname), loc)


def _ingest_jsonl_report(path: Path, field_name: str, index: Dict[str, SourceLocation]) -> None:
    if not path.is_file():
        return
    try:
        with path.open(encoding="utf-8") as fh:
            for line in fh:
                line_s = line.strip()
                if not line_s:
                    continue
                try:
                    row = json.loads(line_s)
                except ValueError:
                    continue
                raw_p = row.get(field_name) if isinstance(row, dict) else None
                if isinstance(raw_p, str):
                    loc = parse_storage_string(raw_p)
                    if loc is not None:
                        _index_insert(index, posixpath.basename(loc.rel_path), loc)
    except OSError:
        return


def _scan_ftp_directory(
    ftp: ftplib.FTP,
    curr: str,
    stack: List[str],
    index: Dict[str, SourceLocation],
) -> int:
    discovered = 0
    for entry_name, facts in ftp.mlsd(curr):
        if entry_name in FTP_SKIP_ENTRIES:
            continue
        etype = facts.get("type", "")
        child = posixpath.join(curr, entry_name)
        if etype == "dir":
            stack.append(child)
        elif etype == "file" and entry_name.lower().endswith(VIDEO_EXTENSIONS):
            rel = child.lstrip("/")
            sid = "synology_video" if rel.startswith("video/") else "synology_tvshow"
            _index_insert(index, entry_name, SourceLocation(sid, rel, "ftp"))
            discovered += 1
    return discovered


def _scan_nas_ftp_index(cfg: SourceEnvConfig, password: str, index: Dict[str, SourceLocation]) -> int:
    """Optionally walks `/video` and `/tvshow` over FTP to populate missing entries cleanly."""
    if not (cfg.nas_host and cfg.nas_user and password):
        return 0
    discovered = 0
    ftp = ftplib.FTP()
    try:
        ftp.connect(cfg.nas_host, DEFAULT_FTP_PORT, timeout=DEFAULT_FTP_TIMEOUT_SECS)
        ftp.login(cfg.nas_user, password)
        stack: List[str] = list(FTP_SCAN_ROOTS)
        while stack:
            try:
                discovered += _scan_ftp_directory(ftp, stack.pop(), stack, index)
            except ftplib.all_errors:
                continue
    except ftplib.all_errors:
        return discovered
    finally:
        try:
            ftp.quit()
        except Exception:
            pass
    return discovered


def _ingest_local_test_fixtures(index: Dict[str, SourceLocation]) -> None:
    fixtures_dir = REPO_ROOT / "tests/data"
    if not fixtures_dir.is_dir():
        return
    for p in fixtures_dir.iterdir():
        if p.is_file() and p.name.lower().endswith(VIDEO_EXTENSIONS):
            rel = str(p.relative_to(REPO_ROOT))
            _index_insert(index, p.name, SourceLocation("local_fixture", rel, "local"))


def build_source_index(
    reports_dir: Path,
    cfg: SourceEnvConfig,
    scan_ftp: bool = False,
    nas_password: str = "",
) -> Dict[str, SourceLocation]:
    """Builds unified lookup map from media filename/stem to logical storage origin."""
    index: Dict[str, SourceLocation] = {}
    _ingest_local_test_fixtures(index)
    _ingest_jsonl_report(reports_dir / "batch_tvshow_nightly.jsonl", "video_rel_path", index)
    _ingest_targets_list(reports_dir / "selected_batch_targets.json", False, index)
    _ingest_targets_list(reports_dir / "unindexed_movies.json", False, index)
    _ingest_jsonl_report(reports_dir / "eval_scale_multimodal_multiwindow.jsonl", "media_path", index)
    _ingest_snippets_report(reports_dir, index)
    _ingest_targets_list(reports_dir / "freebox_batch_targets.json", True, index)
    _ingest_nas_ftp_index(reports_dir, index)
    if scan_ftp:
        _scan_nas_ftp_index(cfg, nas_password, index)
    return index


def _read_sidecar_name_only(path: Path) -> str | None:
    payload = _load_json_safe(path)
    if not isinstance(payload, dict):
        return None
    for key in SIDECAR_KEYS_FILENAME:
        val = payload.get(key)
        if isinstance(val, str) and val.strip():
            return val.strip()
    return None


def _lookup_source_location(
    media_name: str,
    typed_sidecar: dict | None,
    source_index: Dict[str, SourceLocation],
) -> SourceLocation | None:
    if isinstance(typed_sidecar, dict):
        sid = typed_sidecar.get("source_id")
        rel = typed_sidecar.get("source_rel_path")
        if isinstance(sid, str) and isinstance(rel, str) and sid and rel:
            transport = "http" if sid == "freebox_disks" else ("local" if sid.startswith("local") else "ftp")
            return SourceLocation(source_id=sid, rel_path=rel, transport=transport)
    for key in _normalize_lookup_keys(media_name):
        if key in source_index:
            return source_index[key]
    return None


def _evaluate_skip_reason(
    existing_selection: str | None,
    existing_tracks: int,
    no_verify: bool,
    force: bool,
) -> str | None:
    if force or not existing_selection:
        return None
    if existing_selection in CERTAIN_OR_VERIFIED_SELECTIONS:
        return "already_certain_or_verified"
    if no_verify and existing_selection in UPGRADEABLE_SELECTIONS and existing_tracks > 0:
        return "already_present_no_verify"
    return None


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        while chunk := fh.read(SHA256_CHUNK_BYTES):
            digest.update(chunk)
    return digest.hexdigest()


def _index_named_ahash_by_size(ahash_files: List[Path]) -> Dict[int, List[Tuple[Path, str]]]:
    by_size: Dict[int, List[Tuple[Path, str]]] = {}
    for p in ahash_files:
        base_stem = _strip_audio_track_tag(ahash_logical_stem(p.name))
        if MOVIEHASH_RE.fullmatch(base_stem):
            continue
        try:
            by_size.setdefault(p.stat().st_size, []).append((p, urllib.parse.unquote(base_stem)))
        except OSError:
            continue
    return by_size


def _resolve_sha256_twin_name(
    ahash_path: Path,
    named_by_size: Dict[int, List[Tuple[Path, str]]],
    sha_cache: Dict[Path, str],
) -> str | None:
    try:
        size = ahash_path.stat().st_size
    except OSError:
        return None
    candidates = named_by_size.get(size)
    if not candidates:
        return None
    target_sha = sha_cache.setdefault(ahash_path, _sha256_file(ahash_path))
    for cand_path, cand_name in candidates:
        if sha_cache.setdefault(cand_path, _sha256_file(cand_path)) == target_sha:
            return cand_name
    return None


def _read_sidecar_state(
    typed_dict: Dict[str, Any] | None,
) -> Tuple[str | None, int, str | None, int | None, float | None]:
    """Extracts selection, track count, language, audio index, and distance from a typed sidecar."""
    if not typed_dict:
        return None, 0, None, None, None
    raw_sel = typed_dict.get("selection")
    selection = str(raw_sel) if isinstance(raw_sel, str) else None
    tracks = typed_dict.get("container_audio_streams")
    track_count = len(tracks) if isinstance(tracks, list) else 0
    selected = typed_dict.get("selected_audio_stream")
    language: str | None = None
    audio_index: int | None = None
    if isinstance(selected, dict):
        raw_lang = selected.get("language")
        language = str(raw_lang) if isinstance(raw_lang, str) and raw_lang.strip() else None
        raw_idx = selected.get("audio_index")
        audio_index = int(raw_idx) if isinstance(raw_idx, int) else None
    raw_dist = typed_dict.get("verified_distance")
    verified_distance = float(raw_dist) if isinstance(raw_dist, int | float) else None
    return selection, track_count, language, audio_index, verified_distance


def plan_target_item(
    ahash_path: Path,
    phash_dir: Path,
    source_index: Dict[str, SourceLocation],
    no_verify: bool = False,
    force: bool = False,
    named_by_size: Dict[int, List[Tuple[Path, str]]] | None = None,
    sha_cache: Dict[Path, str] | None = None,
) -> AhashTargetItem:
    """Resolves identity, existing sidecar state, and remote origin for one `.ahash` file."""
    logical = ahash_logical_stem(ahash_path.name)
    base_stem = _strip_audio_track_tag(logical)
    sidecar_path = ahash_path.with_name(f"{logical}.json")
    moviehash = base_stem.lower() if MOVIEHASH_RE.fullmatch(base_stem) else None

    typed_payload = _load_json_safe(sidecar_path)
    typed_dict = typed_payload if isinstance(typed_payload, dict) else None
    existing_sel_str, track_count, existing_lang, existing_idx, existing_dist = _read_sidecar_state(typed_dict)

    media_name = _read_sidecar_name_only(sidecar_path)
    if not media_name and moviehash:
        media_name = _read_sidecar_name_only(phash_dir / f"{moviehash}.json")
    if not media_name and moviehash and named_by_size is not None and sha_cache is not None:
        media_name = _resolve_sha256_twin_name(ahash_path, named_by_size, sha_cache)
    if not media_name:
        media_name = urllib.parse.unquote(base_stem) if not moviehash else base_stem

    skip_reason = _evaluate_skip_reason(existing_sel_str, track_count, no_verify, force)
    location = _lookup_source_location(media_name, typed_dict, source_index)
    return AhashTargetItem(
        ahash_path=ahash_path,
        sidecar_path=sidecar_path,
        logical_stem=logical,
        moviehash=moviehash,
        media_name=media_name,
        existing_selection=existing_sel_str,
        existing_track_count=track_count,
        location=location,
        skip_reason=skip_reason,
        existing_language=existing_lang,
        existing_audio_index=existing_idx,
        existing_verified_distance=existing_dist,
    )


def build_stream_url(loc: SourceLocation, cfg: SourceEnvConfig) -> Tuple[str | None, str | None]:
    """Builds a credential-free streaming URL or returns a diagnostic reason if unconfigured."""
    clean_rel = loc.rel_path.lstrip("/")
    if loc.transport == "ftp":
        if not cfg.nas_host:
            return None, "missing_nas_host_env"
        if "|" in clean_rel:
            return None, "unsupported_pipe_char_in_ftp_path"
        return f"ftp://{cfg.nas_host}/{clean_rel}", None
    if loc.transport == "http":
        if not cfg.freebox_base:
            return None, "missing_freebox_http_base_env"
        quoted = urllib.parse.quote(clean_rel, safe="/")
        return f"{cfg.freebox_base}/{quoted}", None
    return loc.rel_path, None


def build_child_env(cfg: SourceEnvConfig, nas_password: str) -> Dict[str, str]:
    """Injects FTP credentials strictly via environment variables (never URLs or argv)."""
    env = os.environ.copy()
    local_bin = str(REPO_ROOT / "bin")
    if Path(local_bin).is_dir():
        env["PATH"] = f"{local_bin}:{env.get('PATH', '')}"
    if cfg.nas_user:
        env["DBSRT_FTP_USER"] = cfg.nas_user
        env["FTP_USER"] = cfg.nas_user
    if nas_password:
        env["DBSRT_FTP_PASSWORD"] = nas_password
        env["FTP_PASSWORD"] = nas_password
    env.setdefault("DBSRT_FFMPEG_THREADS", DEFAULT_FFMPEG_THREADS)
    return env


def resolve_dbsrt_binary(explicit_bin: Path | None = None) -> Path:
    """Selects local `dbsrt` executable (`target/release/dbsrt`, `bin/dbsrt`, `target/debug/dbsrt`)."""
    candidates = [explicit_bin] if explicit_bin else []
    candidates.extend(
        [
            REPO_ROOT / "bin/dbsrt",
            REPO_ROOT / "target/release/dbsrt",
            REPO_ROOT / "target/debug/dbsrt",
        ]
    )
    for cand in candidates:
        if cand is not None and cand.is_file():
            return cand
    return Path("dbsrt")


def build_ahash_sidecar_cmd(
    dbsrt_bin: Path,
    item: AhashTargetItem,
    stream_url: str,
    no_verify: bool,
    force: bool,
) -> List[str]:
    """Constructs credential-free `dbsrt ahash:sidecar` argv for one target."""
    cmd = [
        str(dbsrt_bin),
        "ahash:sidecar",
        "--ahash",
        str(item.ahash_path),
        "--media",
        stream_url,
        "--media-name",
        item.media_name,
        "--json",
    ]
    if item.moviehash:
        cmd.extend(["--moviehash", item.moviehash])
    if item.location:
        cmd.extend(["--source-id", item.location.source_id, "--source-rel-path", item.location.rel_path])
    if no_verify:
        cmd.append("--no-verify")
    if force:
        cmd.append("--force")
    return cmd


def _make_record(item: AhashTargetItem, status: str, error: str | None = None) -> BackfillRecord:
    return BackfillRecord(
        ahash_file=item.ahash_path.name,
        sidecar_file=item.sidecar_path.name,
        logical_stem=item.logical_stem,
        media_name=item.media_name,
        moviehash=item.moviehash,
        source_id=item.location.source_id if item.location else None,
        source_rel_path=item.location.rel_path if item.location else None,
        status=status,
        selection=item.existing_selection,
        audio_index=item.existing_audio_index,
        language=item.existing_language,
        track_count=item.existing_track_count or None,
        verified_distance=item.existing_verified_distance,
        error=error,
    )


def execute_backfill_item(
    item: AhashTargetItem,
    cfg: SourceEnvConfig,
    child_env: Dict[str, str],
    dbsrt_bin: Path,
    args: argparse.Namespace,
) -> BackfillRecord:
    """Runs or dry-runs `dbsrt ahash:sidecar` for one `AhashTargetItem`."""
    if item.skip_reason:
        return _make_record(item, "skipped")
    if item.location is None:
        return _make_record(item, "unresolved_source", "source path not found in inventories")

    stream_url, url_err = build_stream_url(item.location, cfg)
    if not stream_url:
        return _make_record(item, "unresolved_url", url_err)
    if args.dry_run:
        return _make_record(item, "dry_run_ready")

    cmd = build_ahash_sidecar_cmd(dbsrt_bin, item, stream_url, args.no_verify, args.force)
    t0 = time.monotonic()
    try:
        res = subprocess.run(
            cmd,
            env=child_env,
            capture_output=True,
            text=True,
            timeout=DEFAULT_SUBPROCESS_TIMEOUT_SECS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return _make_record(item, "error", f"{type(exc).__name__}: {exc}")

    elapsed = round(time.monotonic() - t0, 3)
    if res.returncode != 0:
        err_line = (res.stderr or res.stdout or f"exit {res.returncode}").strip().splitlines()[-1]
        rec = _make_record(item, "error", err_line)
        rec.elapsed_sec = elapsed
        return rec

    rec = _make_record(item, "ok")
    rec.elapsed_sec = elapsed
    try:
        parsed = json.loads(res.stdout.strip().splitlines()[-1])
        rec.status = str(parsed.get("action", "ok"))
        rec.selection = parsed.get("selection")
        rec.audio_index = parsed.get("audio_index")
        rec.language = parsed.get("language")
        rec.track_count = parsed.get("track_count")
        rec.verified_distance = parsed.get("verified_distance")
    except (ValueError, IndexError):
        pass
    return rec


def clamp_workers(requested: int) -> int:
    """Bounds backfill worker concurrency to `[1, MAX_BACKFILL_WORKERS]` (CloudLinux LVE limit)."""
    return max(1, min(requested, MAX_BACKFILL_WORKERS))


def write_jsonl_report(records: Iterable[BackfillRecord], report_path: Path) -> None:
    """Writes timestamped JSONL audit trail atomically via `.tmp` rename."""
    report_path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = report_path.with_suffix(report_path.suffix + ".tmp")
    with tmp_path.open("w", encoding="utf-8") as fh:
        for rec in records:
            fh.write(json.dumps(asdict(rec), ensure_ascii=False) + "\n")
    tmp_path.replace(report_path)


def build_phash_moviehash_index(phash_dir: Path) -> Dict[str, str]:
    """Builds an unambiguous reverse index `normalized_media_key -> moviehash_16hex` from `__PHASH__/*.json`."""
    if not phash_dir.is_dir():
        return {}
    buckets: Dict[str, set[str]] = {}
    for sidecar_path in sorted(phash_dir.glob("*.json")):
        payload = _load_json_safe(sidecar_path)
        if not isinstance(payload, dict):
            continue
        mh = payload.get("moviehash")
        media_name = payload.get("media_name")
        if not isinstance(mh, str) or not MOVIEHASH_RE.fullmatch(mh):
            continue
        if not isinstance(media_name, str) or not media_name.strip():
            continue
        norm_mh = mh.lower()
        for key in _normalize_lookup_keys(media_name):
            buckets.setdefault(key, set()).add(norm_mh)
    return {k: next(iter(vals)) for k, vals in buckets.items() if len(vals) == 1}


def _build_migration_candidate(
    ahash_path: Path,
    source_index: Dict[str, SourceLocation],
) -> MigrationCandidate | None:
    """Returns a `MigrationCandidate` for non-16-hex `.ahash` files, or `None` if already 16-hex."""
    logical = ahash_logical_stem(ahash_path.name)
    base_stem = _strip_audio_track_tag(logical)
    if MOVIEHASH_RE.fullmatch(base_stem):
        return None
    track_tag = logical[len(base_stem) :]
    without_ext = ahash_path.name.removesuffix(".ahash")
    fps_match = AHASH_FPS_SUFFIX_RE.search(without_ext)
    fps_suffix = fps_match.group(0) if fps_match else ""

    sidecar_path = ahash_path.with_name(f"{logical}.json")
    raw_sidecar = _load_json_safe(sidecar_path)
    typed_sidecar = raw_sidecar if isinstance(raw_sidecar, dict) else None
    media_name = _read_sidecar_name_only(sidecar_path) or urllib.parse.unquote(base_stem)
    location = _lookup_source_location(media_name, typed_sidecar, source_index)
    if location is None:
        location = _lookup_source_location(urllib.parse.unquote(base_stem), typed_sidecar, source_index)
    return MigrationCandidate(
        ahash_path=ahash_path,
        sidecar_path=sidecar_path,
        logical_stem=logical,
        base_stem=base_stem,
        track_tag=track_tag,
        fps_suffix=fps_suffix,
        media_name=media_name,
        typed_sidecar=typed_sidecar,
        location=location,
    )


def _lookup_phash_index(cand: MigrationCandidate, phash_index: Dict[str, str]) -> str | None:
    for probe in (cand.media_name, urllib.parse.unquote(cand.base_stem)):
        for key in _normalize_lookup_keys(probe):
            if key in phash_index:
                return phash_index[key]
    return None


def _remote_stream_base(
    transport: str,
    cfg: SourceEnvConfig,
    child_env: Dict[str, str],
) -> Tuple[str | None, str | None]:
    if transport == "ftp":
        has_creds = bool(cfg.nas_host and cfg.nas_user and child_env.get("DBSRT_FTP_PASSWORD"))
        return (f"ftp://{cfg.nas_host}:{DEFAULT_FTP_PORT}", None) if has_creds else (None, "unresolved_url")
    if transport == "http":
        return (cfg.freebox_base, None) if cfg.freebox_base else (None, "unresolved_url")
    return None, "unresolved_source"


def _compute_remote_or_local_moviehash(
    cand: MigrationCandidate,
    cfg: SourceEnvConfig,
    child_env: Dict[str, str],
) -> Tuple[str | None, str | None]:
    """Resolves 64-bit OpenSubtitles MovieHash lightly without downloading the media file."""
    loc = cand.location
    if loc is None:
        return None, "unresolved_source"
    if loc.transport == "local":
        rel = Path(loc.rel_path)
        mh = _worker.compute_local_moviehash(rel if rel.is_absolute() else REPO_ROOT / rel)
        return (mh, None) if mh else (None, "error")
    stream_base, err_status = _remote_stream_base(loc.transport, cfg, child_env)
    if not stream_base:
        return None, err_status
    target = _worker.VideoTarget("/" + loc.rel_path.lstrip("/"), cand.media_name, 0)
    mh = _worker.target_moviehash(target, stream_base, child_env)
    return (mh, None) if mh else (None, "error")


def resolve_candidate_moviehash(
    cand: MigrationCandidate,
    phash_index: Dict[str, str],
    cfg: SourceEnvConfig,
    child_env: Dict[str, str],
) -> Tuple[str | None, str | None]:
    """Resolves `(moviehash, error_status)` via `__PHASH__` reverse index, else lightweight 128 KiB probe."""
    existing_mh = cand.typed_sidecar.get("moviehash") if cand.typed_sidecar else None
    if isinstance(existing_mh, str) and MOVIEHASH_RE.fullmatch(existing_mh):
        return existing_mh.lower(), None
    indexed = _lookup_phash_index(cand, phash_index)
    if indexed:
        return indexed, None
    return _compute_remote_or_local_moviehash(cand, cfg, child_env)


def _sidecar_completeness_score(payload: Dict[str, Any] | None) -> Tuple[int, int, int, int]:
    if not payload:
        return (0, 0, 0, 0)
    sel = str(payload.get("selection") or "")
    streams = payload.get("container_audio_streams")
    stream_cnt = len(streams) if isinstance(streams, list) else 0
    has_selected = 1 if isinstance(payload.get("selected_audio_stream"), dict) else 0
    has_source = 1 if payload.get("source_rel_path") else 0
    return (SELECTION_COMPLETENESS_RANK.get(sel, 0), stream_cnt, has_selected, has_source)


def _should_fill_field(current: object, candidate: object) -> bool:
    if candidate is None:
        return False
    if current is None:
        return True
    return current == [] and isinstance(candidate, list) and bool(candidate)


def merge_ahash_sidecars(
    target_dict: Dict[str, Any] | None,
    legacy_dict: Dict[str, Any] | None,
    moviehash: str,
    target_ahash_name: str,
    media_name: str,
) -> Dict[str, Any] | None:
    """Merges two sidecar payloads keeping the most complete track verification and filling gaps."""
    if target_dict is None and legacy_dict is None:
        return None
    if _sidecar_completeness_score(target_dict) >= _sidecar_completeness_score(legacy_dict):
        primary, secondary = target_dict, legacy_dict
    else:
        primary, secondary = legacy_dict, target_dict
    merged: Dict[str, Any] = dict(primary or {})
    if secondary:
        for key in SIDECAR_FILLABLE_KEYS:
            candidate = secondary.get(key)
            if _should_fill_field(merged.get(key), candidate):
                merged[key] = candidate
    merged["moviehash"] = moviehash
    merged["ahash_file"] = target_ahash_name
    if not merged.get("media_name"):
        merged["media_name"] = media_name
    return merged


def _write_json_atomic(path: Path, payload: Dict[str, Any]) -> None:
    tmp_path = path.with_name(f"{path.name}.{os.getpid()}.tmp")
    with tmp_path.open("w", encoding="utf-8") as fh:
        fh.write(json.dumps(payload, indent=2, ensure_ascii=False))
        fh.flush()
        os.fsync(fh.fileno())
    os.replace(tmp_path, path)


def _migration_record(
    cand: MigrationCandidate,
    moviehash: str | None,
    target_sidecar_name: str,
    status: str,
    error: str | None = None,
) -> BackfillRecord:
    sel, tracks, lang, audio_idx, dist = _read_sidecar_state(cand.typed_sidecar)
    return BackfillRecord(
        ahash_file=cand.ahash_path.name,
        sidecar_file=target_sidecar_name,
        logical_stem=cand.logical_stem,
        media_name=cand.media_name,
        moviehash=moviehash,
        source_id=cand.location.source_id if cand.location else None,
        source_rel_path=cand.location.rel_path if cand.location else None,
        status=status,
        selection=sel,
        audio_index=audio_idx,
        language=lang,
        track_count=tracks or None,
        verified_distance=dist,
        error=error,
    )


def _dedup_or_promote_binary(legacy_ahash: Path, target_ahash: Path) -> str | None:
    """Keeps the larger/most complete `.ahash` binary and removes the duplicate; returns warning if hashes differ at equal size."""
    legacy_size = legacy_ahash.stat().st_size
    target_size = target_ahash.stat().st_size
    if legacy_size > target_size:
        os.replace(legacy_ahash, target_ahash)
        return None
    warning: str | None = None
    if legacy_size == target_size and _sha256_file(legacy_ahash) != _sha256_file(target_ahash):
        warning = "equal_size_sha256_mismatch_kept_existing"
    legacy_ahash.unlink(missing_ok=True)
    return warning


def apply_migration_item(
    cand: MigrationCandidate,
    moviehash: str | None,
    err_status: str | None,
    dry_run: bool,
) -> BackfillRecord:
    """Applies or dry-runs lossless `<moviehash_16hex>` rename or binary deduplication."""
    if not moviehash:
        status = err_status or "error"
        return _migration_record(cand, None, cand.sidecar_path.name, status, f"cannot resolve moviehash ({status})")

    target_ahash = cand.ahash_path.with_name(f"{moviehash}{cand.track_tag}{cand.fps_suffix}.ahash")
    target_sidecar = cand.ahash_path.with_name(f"{moviehash}{cand.track_tag}.json")
    target_exists = target_ahash.exists()
    if dry_run:
        return _migration_record(
            cand, moviehash, target_sidecar.name, "dry_run_dedup" if target_exists else "dry_run_migrate"
        )

    raw_target = _load_json_safe(target_sidecar)
    target_dict = raw_target if isinstance(raw_target, dict) else None
    merged = merge_ahash_sidecars(target_dict, cand.typed_sidecar, moviehash, target_ahash.name, cand.media_name)

    warning: str | None = None
    if target_exists:
        warning = _dedup_or_promote_binary(cand.ahash_path, target_ahash)
        if merged is not None:
            _write_json_atomic(target_sidecar, merged)
        if cand.sidecar_path != target_sidecar:
            cand.sidecar_path.unlink(missing_ok=True)
        return _migration_record(cand, moviehash, target_sidecar.name, "deduplicated", warning)

    if merged is not None:
        _write_json_atomic(target_sidecar, merged)
    os.replace(cand.ahash_path, target_ahash)
    if cand.sidecar_path != target_sidecar:
        cand.sidecar_path.unlink(missing_ok=True)
    return _migration_record(cand, moviehash, target_sidecar.name, "migrated")


def _run_migration(
    args: argparse.Namespace,
    cfg: SourceEnvConfig,
    child_env: Dict[str, str],
    source_index: Dict[str, SourceLocation],
    workers: int,
) -> List[BackfillRecord]:
    """Resolves MovieHashes in parallel and applies lossless migration sequentially."""
    phash_index = build_phash_moviehash_index(args.phash_dir)
    ahash_files = sorted(args.ahash_dir.glob("*.ahash")) if args.ahash_dir.is_dir() else []
    candidates = [c for p in ahash_files if (c := _build_migration_candidate(p, source_index)) is not None]
    if args.limit is not None and args.limit > 0:
        candidates = candidates[: args.limit]
    print(
        f"Migrating {len(candidates)} legacy non-hex .ahash files (phash_index={len(phash_index)}, workers={workers})...",
        flush=True,
    )
    if not candidates:
        return []

    if workers == 1:
        resolved = [resolve_candidate_moviehash(c, phash_index, cfg, child_env) for c in candidates]
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
            resolved = list(pool.map(lambda c: resolve_candidate_moviehash(c, phash_index, cfg, child_env), candidates))

    records: List[BackfillRecord] = []
    total = len(candidates)
    for idx, (cand, (mh, err_status)) in enumerate(zip(candidates, resolved), start=1):
        rec = apply_migration_item(cand, mh, err_status, args.dry_run)
        records.append(rec)
        _emit_progress(idx, total, rec, args.dry_run)
    return records


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ahash-dir", type=Path, default=DEFAULT_AHASH_DIR)
    parser.add_argument("--phash-dir", type=Path, default=DEFAULT_PHASH_DIR)
    parser.add_argument("--reports-dir", type=Path, default=DEFAULT_REPORTS_DIR)
    parser.add_argument("--dbsrt-bin", type=Path, default=None)
    parser.add_argument("--report-out", type=Path, default=None)
    parser.add_argument("--workers", type=int, default=DEFAULT_BACKFILL_WORKERS)
    parser.add_argument("--limit", type=int, default=None)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-verify", action="store_true")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--scan-ftp", action="store_true", help="Refresh FTP listing live from Synology")
    parser.add_argument(
        "--migrate-to-moviehash",
        action="store_true",
        help="Losslessly migrate legacy plaintext .ahash/.json pairs to <moviehash_16hex>",
    )
    return parser


def _print_counter_section(title: str, counter: Counter[str]) -> None:
    if not counter:
        return
    print(f"  {title}:")
    for key in sorted(counter):
        print(f"    - {key:<20}: {counter[key]}")


def _print_summary(records: List[BackfillRecord], report_path: Path, dry_run: bool) -> None:
    status_counts = Counter(rec.status for rec in records)
    selection_counts = Counter(rec.selection or "none" for rec in records)
    language_counts = Counter((rec.language or "unknown") for rec in records if rec.selection or rec.language)
    total = len(records)
    resolved = sum(v for k, v in status_counts.items() if k not in {"unresolved_source", "unresolved_url"})
    pct = (resolved / total * 100.0) if total else 0.0
    mode_tag = "DRY-RUN" if dry_run else "LIVE"
    print(f"=== aHash Sidecar Backfill Summary ({mode_tag}) ===")
    print(f"  Total evaluated : {total}")
    print(f"  Path/URL active : {resolved}/{total} ({pct:.1f}%)")
    for key in sorted(status_counts):
        print(f"  - {key:<22}: {status_counts[key]}")
    _print_counter_section("By selection", selection_counts)
    _print_counter_section("By language", language_counts)
    print(f"  Audit report    : {report_path}", flush=True)


def _format_progress_line(index: int, total: int, rec: BackfillRecord) -> str:
    """Formats one single-line progress status for live backfill runs."""
    details: List[str] = []
    if rec.selection:
        details.append(rec.selection)
    if rec.language:
        details.append(rec.language)
    if rec.verified_distance is not None:
        details.append(f"d={rec.verified_distance:.2f}")
    if rec.elapsed_sec > 0.0:
        details.append(f"{rec.elapsed_sec:.2f}s")
    if rec.error:
        details.append(rec.error)
    suffix = f" ({', '.join(details)})" if details else ""
    return f"[{index}/{total}] {rec.status}{suffix} -> {rec.sidecar_file}"


def _emit_progress(index: int, total: int, rec: BackfillRecord, dry_run: bool) -> None:
    if not dry_run:
        print(_format_progress_line(index, total, rec), flush=True)


def _select_planned_targets(
    args: argparse.Namespace,
    source_index: Dict[str, SourceLocation],
) -> List[AhashTargetItem]:
    ahash_files = sorted(args.ahash_dir.glob("*.ahash")) if args.ahash_dir.is_dir() else []
    named_by_size = _index_named_ahash_by_size(ahash_files)
    sha_cache: Dict[Path, str] = {}
    planned = [
        plan_target_item(
            p,
            args.phash_dir,
            source_index,
            no_verify=args.no_verify,
            force=args.force,
            named_by_size=named_by_size,
            sha_cache=sha_cache,
        )
        for p in ahash_files
    ]
    if args.limit is None or args.limit <= 0:
        return planned
    pending = [item for item in planned if not item.skip_reason]
    if not args.dry_run:
        pending.sort(key=lambda item: (item.location is None, item.ahash_path.name))
    return pending[: args.limit]


def _run_planned_batch(
    planned: List[AhashTargetItem],
    cfg: SourceEnvConfig,
    child_env: Dict[str, str],
    dbsrt_bin: Path,
    args: argparse.Namespace,
    workers: int,
) -> List[BackfillRecord]:
    total = len(planned)
    records: List[BackfillRecord] = []
    if args.dry_run or workers == 1:
        for idx, item in enumerate(planned, start=1):
            rec = execute_backfill_item(item, cfg, child_env, dbsrt_bin, args)
            records.append(rec)
            _emit_progress(idx, total, rec, args.dry_run)
        return records

    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(execute_backfill_item, item, cfg, child_env, dbsrt_bin, args) for item in planned]
        for idx, fut in enumerate(concurrent.futures.as_completed(futures), start=1):
            rec = fut.result()
            records.append(rec)
            _emit_progress(idx, total, rec, args.dry_run)
    return records


def main(argv: List[str] | None = None) -> int:
    args = build_arg_parser().parse_args(argv)
    workers = clamp_workers(args.workers)
    cfg = build_source_env_config(REPO_ROOT, load_env_files(REPO_ROOT))
    need_pass = args.scan_ftp or args.migrate_to_moviehash or not args.dry_run
    nas_pass = resolve_nas_password(cfg) if need_pass else ""

    source_index = build_source_index(args.reports_dir, cfg, scan_ftp=args.scan_ftp, nas_password=nas_pass)
    child_env = build_child_env(cfg, nas_pass)
    if args.migrate_to_moviehash:
        records = _run_migration(args, cfg, child_env, source_index, workers)
        prefix = "ahash_moviehash_migration"
    else:
        planned = _select_planned_targets(args, source_index)
        print(f"Scanning {len(planned)} .ahash files (workers={workers})...", flush=True)
        dbsrt_bin = resolve_dbsrt_binary(args.dbsrt_bin)
        records = _run_planned_batch(planned, cfg, child_env, dbsrt_bin, args, workers)
        prefix = "ahash_sidecar_backfill"

    ts = datetime.datetime.now(datetime.UTC).strftime("%Y%m%d_%H%M%S")
    report_path = args.report_out or (args.reports_dir / f"{prefix}_{ts}.jsonl")
    write_jsonl_report(records, report_path)
    _print_summary(records, report_path, args.dry_run)
    failed = any(
        r.status == "error"
        or (args.migrate_to_moviehash and not args.dry_run and r.status in {"unresolved_source", "unresolved_url"})
        for r in records
    )
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
