"""Spatiotemporal trajectory JSONL audit service, KPIs, and on-demand runner."""

from __future__ import annotations

import json
import math
import re
import subprocess
import time
import urllib.parse
from pathlib import Path
from typing import Any

from pydbsrt.portal.config import (
    ACTIVE_BATCH_RECENCY_SECONDS,
    DEFAULT_CATALOG_FALLBACK_COUNT,
    DEFAULT_TRAJECTORY_DIR,
    DEFAULT_TRAJECTORY_ROOT_DIR,
    PERCENT_SCALE,
    REPO_ROOT,
    SUBPROCESS_TRAJECTORY_TIMEOUT_SECS,
    TRAJECTORY_DEFAULT_FRAMES,
    TRAJECTORY_SAMPLE_CUTS,
    _resolve_data_path,
    format_human_size,
    strip_accents,
)
from pydbsrt.portal.models import TrajectoriesQueryCriteria
from pydbsrt.portal.services.phash_reconciliation import _read_catalog_dbphash

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

    candidates: list[Path] = []
    if target_path:
        cand_p = Path(target_path)
        candidates.append(cand_p.with_suffix(".json"))
        candidates.append(cand_p.parent / f"{moviehash}.json")
    candidates.append(_resolve_data_path("data/__PHASH__") / f"{moviehash}.json")
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


def _active_trajectory_dirs() -> tuple[Path, Path]:
    """Resolves trajectory audit and root report directories across CWD overrides and REPO_ROOT."""
    traj_dir = DEFAULT_TRAJECTORY_DIR if DEFAULT_TRAJECTORY_DIR.is_dir() else _resolve_data_path("data/reports/audit")
    root_dir = (
        DEFAULT_TRAJECTORY_ROOT_DIR if DEFAULT_TRAJECTORY_ROOT_DIR.is_dir() else _resolve_data_path("data/reports")
    )
    return traj_dir, root_dir


def list_trajectory_audit_files() -> list[dict[str, Any]]:
    """Scans and lists available trajectory tracking audit JSONL files."""
    traj_dir, root_dir = _active_trajectory_dirs()
    candidates: list[Path] = []
    if traj_dir.is_dir():
        candidates.extend(traj_dir.glob("*.jsonl"))
    if root_dir.is_dir():
        candidates.extend(root_dir.glob("*trajectory*.jsonl"))

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
    """Resolves a trajectory audit JSONL file path by name or falls back to newest file."""
    traj_dir, root_dir = _active_trajectory_dirs()
    if file_name:
        cand1 = traj_dir / file_name
        if cand1.is_file():
            return cand1
        cand2 = root_dir / file_name
        if cand2.is_file():
            return cand2
    files = list_trajectory_audit_files()
    if files:
        return Path(files[0]["path"])
    return None


def _load_trajectory_records(file_name: str) -> tuple[Path | None, list[dict[str, Any]]]:
    """Loads and mtime-caches trajectory records from a JSONL audit file."""
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
    cat_path = _resolve_data_path("data/catalog.dbphash")
    _hdr, cat_items = _read_catalog_dbphash(cat_path) if cat_path.is_file() else ({}, [])
    catalog_ids = {c["catalog_id"] for c in cat_items} if cat_items else set()
    catalog_total = len(catalog_ids) if catalog_ids else DEFAULT_CATALOG_FALLBACK_COUNT

    if catalog_ids:
        unique_covered = len(matched_media_ids.intersection(catalog_ids))
    else:
        unique_covered = min(len(matched_media_ids), catalog_total)

    pct = round(unique_covered / float(catalog_total) * PERCENT_SCALE, 1) if catalog_total > 0 else 0.0
    return unique_covered, catalog_total, max(0.0, min(PERCENT_SCALE, pct))


def _compute_trajectory_kpi(records: list[dict[str, Any]]) -> dict[str, Any]:
    """Computes aggregated KPIs and bounded catalog coverage across trajectory records."""
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

    coherence_rate = round(total_coherent / float(total_matched) * PERCENT_SCALE, 1) if total_matched > 0 else 0.0
    match_rate = round(total_matched / float(total_audits) * PERCENT_SCALE, 1) if total_audits > 0 else 0.0
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
    """Checks whether a trajectory record matches the requested status filter."""
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
    """Sorts trajectory audit records by name, duration, elapsed time, or matched cuts."""
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


def _build_trajectory_summary_item(r: dict[str, Any]) -> dict[str, Any]:
    """Formats a single trajectory record for paginated list responses."""
    t_media = str(r.get("target_media", ""))
    t_path = str(r.get("target_path", ""))
    resolved_title = _resolve_phash_target_title(t_media, t_path)
    cuts_raw = r.get("cuts")
    cuts_list: list[dict[str, Any]] = cuts_raw if isinstance(cuts_raw, list) else []
    return {
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


def _matches_trajectory_query(r: dict[str, Any], q: str) -> bool:
    """Checks accent-insensitive query match across target media, title, and path."""
    if not q:
        return True
    target = strip_accents(str(r.get("target_media", "")).lower())
    title = strip_accents(str(r.get("consensus_title", "")).lower())
    tpath = strip_accents(str(r.get("target_path", "")).lower())
    return q in target or q in title or q in tpath


def query_trajectory_records(criteria: TrajectoriesQueryCriteria) -> dict[str, Any]:
    """Queries, filters and paginates spatiotemporal trajectory records."""
    path, all_records = _load_trajectory_records(criteria.file_name)
    kpis = _compute_trajectory_kpi(all_records)

    q = strip_accents(criteria.search_q.lower().strip())
    filtered = [
        r
        for r in all_records
        if _matches_trajectory_status(r, criteria.status_filter) and _matches_trajectory_query(r, q)
    ]

    sorted_records = _sort_trajectory_records(filtered, criteria.sort_by)
    start_idx = (criteria.page - 1) * criteria.limit
    end_idx = start_idx + criteria.limit
    items = [_build_trajectory_summary_item(r) for r in sorted_records[start_idx:end_idx]]

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

    runner_script = str(REPO_ROOT / "scripts/benchmarks/trajectory_bench_runner.py")
    dbphash_path = str(_resolve_data_path("data/catalog.dbphash"))
    cmd = [
        "python3",
        runner_script,
        target_path,
        "-c",
        str(cuts),
        "-n",
        str(frames),
        "--dbphash",
        dbphash_path,
    ]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=SUBPROCESS_TRAJECTORY_TIMEOUT_SECS)
    except subprocess.TimeoutExpired:
        return {"status": "error", "error": "Trajectory tracking execution timed out (60s)"}

    if proc.returncode != 0:
        return {"status": "error", "error": proc.stderr or proc.stdout or "Execution failed"}

    res_file = _resolve_data_path("data/reports/trajectory_batch_results.jsonl")
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
