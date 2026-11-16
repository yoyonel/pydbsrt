"""Microscopic pHash frame-by-frame inspection, entropy, mobility, and cut detection."""

from __future__ import annotations

import json
import re
import struct
from pathlib import Path
from typing import Any

from pydbsrt.portal.config import (
    BITS_PER_HASH,
    CATALOG_INDEX_ENTRY_BYTES,
    DEFAULT_BASELINE_FPS,
    DELTA_MOBILITY_BUCKETS,
    DELTA_MOBILITY_DESCRIPTIONS,
    DELTA_MOBILITY_LABELS,
    DELTA_STEP_BITS,
    ENTROPY_HEALTH_MAX_POPCOUNT,
    ENTROPY_HEALTH_MIN_POPCOUNT,
    ENTROPY_HISTOGRAM_BUCKETS,
    HISTOGRAM_BUCKET_WIDTH,
    MAX_FILENAME_UTF8_BYTES,
    MICROSCOPIC_CUTS_SAMPLE_LIMIT,
    MILLIS_PER_SEC,
    PERCENT_SCALE,
    PHASH_CUT_HAMMING_THRESHOLD,
    PHASH_FRAME_BYTES,
    RE_FPS,
    TIMELINE_SERIES_POINTS,
    _format_duration,
    _resolve_data_path,
    format_human_size,
)
from pydbsrt.portal.services.phash_reconciliation import _is_valid_moviehash_hex
from scripts.data_access.resolver import resolve_local_artifact_download


def _resolve_phash_via_sidecar_json(phash_dir: Path, safe_prefix: str, raw_base: str) -> Path | None:
    """Resolves a legacy-named .phash file referenced inside <moviehash>.json sidecar."""
    if not _is_valid_moviehash_hex(safe_prefix):
        return None
    sc_path = phash_dir / f"{safe_prefix.lower()}.json"
    if not sc_path.is_file():
        return None
    try:
        sc_data = json.loads(sc_path.read_text(encoding="utf-8"))
        ref_name = str(sc_data.get("phash_file") or sc_data.get("media_name") or "").strip()
        if ref_name and ref_name != raw_base:
            return _resolve_target_phash_file(ref_name)
    except Exception:
        return None
    return None


def _resolve_phash_by_moviehash_prefix(phash_dir: Path, raw_base: str) -> Path | None:
    """Resolves a .phash file by 16-hex moviehash prefix or sidecar JSON reference."""
    stem_head = raw_base.split(".")[0].strip()
    safe_prefix = re.sub(r"[^a-zA-Z0-9_-]", "", stem_head)
    if not safe_prefix:
        return None
    phash_root = phash_dir.resolve()
    try:
        matches = sorted(phash_dir.glob(f"{safe_prefix}*.phash"), key=lambda p: p.name)
    except OSError:
        matches = []
    for match in matches:
        try:
            resolved = match.resolve()
            if phash_root in resolved.parents and resolved.is_file():
                return resolved
        except OSError:
            continue
    return _resolve_phash_via_sidecar_json(phash_dir, safe_prefix, raw_base)


def _resolve_target_phash_file(identifier: str) -> Path | None:
    """Locates a .phash file safely across Unicode NFC/NFD, percent-encoded names, or moviehash."""
    if not identifier:
        return None
    cleaned_ident = identifier.strip()
    if cleaned_ident.startswith("data/__PHASH__/"):
        cleaned_ident = cleaned_ident[len("data/__PHASH__/") :]
    if "/" in cleaned_ident or "\\" in cleaned_ident or cleaned_ident in (".", ".."):
        return None

    phash_dir_raw = _resolve_data_path("data/__PHASH__")
    if not phash_dir_raw.is_dir():
        return None
    data_root = phash_dir_raw.parent
    phash_dir = phash_dir_raw.resolve()

    name_variants = [cleaned_ident]
    if not cleaned_ident.lower().endswith(".phash"):
        name_variants.append(f"{cleaned_ident}.phash")

    for variant in name_variants:
        if len(variant.encode("utf-8")) <= MAX_FILENAME_UTF8_BYTES:
            hit, _reason, _code = resolve_local_artifact_download(data_root, "phash", variant)
            if hit is not None and hit.is_file():
                return hit

    return _resolve_phash_by_moviehash_prefix(phash_dir, cleaned_ident)


def _compute_entropy_and_mobility(
    hashes: tuple[int, ...],
) -> tuple[float, float, list[int], list[int], float]:
    """Computes popcounts, DCT entropy histogram, consecutive delta mobility, and mean delta."""
    popcounts: list[int] = []
    distribution = [0] * ENTROPY_HISTOGRAM_BUCKETS
    delta_distribution = [0] * DELTA_MOBILITY_BUCKETS
    total_delta = 0

    for i, h in enumerate(hashes):
        pop = h.bit_count()
        popcounts.append(pop)
        bucket_idx = min(pop // HISTOGRAM_BUCKET_WIDTH, ENTROPY_HISTOGRAM_BUCKETS - 1)
        distribution[bucket_idx] += 1

        if i > 0:
            delta = (h ^ hashes[i - 1]).bit_count()
            total_delta += delta
            d_bucket = min(delta // DELTA_STEP_BITS, DELTA_MOBILITY_BUCKETS - 1)
            delta_distribution[d_bucket] += 1

    mean_pop = sum(popcounts) / len(popcounts) if popcounts else 0.0
    entropy_pct = round(mean_pop / float(BITS_PER_HASH) * PERCENT_SCALE, 1)
    mean_delta = (total_delta / float(len(hashes) - 1)) if len(hashes) > 1 else 0.0
    return mean_pop, entropy_pct, distribution, delta_distribution, mean_delta


def _detect_cuts(hashes: tuple[int, ...], fps: float) -> tuple[list[dict[str, Any]], int]:
    """Scans perceptual hashes for scene cuts exceeding the Hamming threshold."""
    cuts: list[dict[str, Any]] = []
    total_cuts_count = 0
    for i in range(1, len(hashes)):
        delta = (hashes[i] ^ hashes[i - 1]).bit_count()
        if delta >= PHASH_CUT_HAMMING_THRESHOLD:
            total_cuts_count += 1
            t_ms = int(i / fps * MILLIS_PER_SEC)
            cuts.append(
                {
                    "frame": i,
                    "time_ms": t_ms,
                    "time_fmt": _format_duration(t_ms),
                    "distance": delta,
                    "hash": f"{hashes[i]:016x}",
                }
            )

    if len(cuts) > MICROSCOPIC_CUTS_SAMPLE_LIMIT:
        step = len(cuts) / float(MICROSCOPIC_CUTS_SAMPLE_LIMIT)
        sampled_cuts = [cuts[int(idx * step)] for idx in range(MICROSCOPIC_CUTS_SAMPLE_LIMIT)]
    else:
        sampled_cuts = cuts
    return sampled_cuts, total_cuts_count


def _aggregate_timeline(hashes: tuple[int, ...], fps: float) -> list[dict[str, Any]]:
    """Builds downsampled timeline points measuring mean mobility and peak cuts."""
    total_frames = len(hashes)
    bucket_size = max(1, total_frames // TIMELINE_SERIES_POINTS)
    timeline: list[dict[str, Any]] = []
    cur_bucket_sum = 0
    cur_bucket_count = 0
    cur_bucket_max = 0

    for i in range(1, total_frames):
        delta = (hashes[i] ^ hashes[i - 1]).bit_count()
        cur_bucket_sum += delta
        cur_bucket_count += 1
        if delta > cur_bucket_max:
            cur_bucket_max = delta

        is_last = i % bucket_size == 0 or i == total_frames - 1
        if is_last:
            t_ms = int(i / fps * MILLIS_PER_SEC)
            mean_d = round(cur_bucket_sum / float(cur_bucket_count), 2) if cur_bucket_count > 0 else 0.0
            timeline.append(
                {
                    "time_fmt": _format_duration(t_ms),
                    "frame": i,
                    "delta": mean_d,
                    "delta_mean": mean_d,
                    "delta_max": cur_bucket_max,
                }
            )
            cur_bucket_sum = 0
            cur_bucket_count = 0
            cur_bucket_max = 0

    return timeline


def _extract_cuts_and_timeline(
    hashes: tuple[int, ...], fps: float
) -> tuple[list[dict[str, Any]], int, list[dict[str, Any]]]:
    """Extracts sampled cuts, total cuts count, and timeline mobility series."""
    sampled_cuts, total_cuts = _detect_cuts(hashes, fps)
    timeline = _aggregate_timeline(hashes, fps)
    return sampled_cuts, total_cuts, timeline


def _detect_phash_cuts_and_entropy(
    hashes: tuple[int, ...], fps: float
) -> tuple[float, float, list[int], list[dict[str, Any]], int, list[dict[str, Any]], list[int], float]:
    """Scans perceptual hashes, detecting scene cuts, visual entropy, timeline, and delta mobility."""
    mean_pop, entropy_pct, distribution, delta_dist, mean_delta = _compute_entropy_and_mobility(hashes)
    sampled_cuts, total_cuts_count, timeline = _extract_cuts_and_timeline(hashes, fps)
    return mean_pop, entropy_pct, distribution, sampled_cuts, total_cuts_count, timeline, delta_dist, mean_delta


def _parse_phash_filename_fps(filename: str) -> float:
    """Extracts FPS from a .phash filename or falls back to DEFAULT_BASELINE_FPS."""
    fps_m = RE_FPS.search(filename)
    if not fps_m:
        return DEFAULT_BASELINE_FPS
    try:
        fps_val = float(fps_m.group(1))
        return fps_val if fps_val > 0.0 else DEFAULT_BASELINE_FPS
    except ValueError:
        return DEFAULT_BASELINE_FPS


def inspect_phash_microscopic(identifier: str, fallback_identifier: str = "") -> dict[str, Any]:
    """Performs deep microscopic frame-by-frame inspection of a single .phash file."""
    primary = identifier.strip()
    secondary = fallback_identifier.strip()
    if not primary and not secondary:
        return {"status": "error", "message": "Identifiant moviehash ou nom de fichier requis"}

    target_file = _resolve_target_phash_file(primary) if primary else None
    if (not target_file or not target_file.is_file()) and secondary and secondary != primary:
        target_file = _resolve_target_phash_file(secondary)
    if not target_file or not target_file.is_file():
        label = primary or secondary
        return {"status": "not_found", "message": f"Fichier .phash pour '{label}' introuvable sur disque"}

    raw_bytes = target_file.read_bytes()
    fc = len(raw_bytes) // PHASH_FRAME_BYTES
    if fc == 0:
        return {"status": "error", "message": "Fichier .phash vide"}

    hashes = struct.unpack(f">{fc}Q", raw_bytes)
    fps = _parse_phash_filename_fps(target_file.name)

    mean_pop, entropy_pct, distribution, cuts, total_cuts_count, timeline, delta_dist, mean_delta = (
        _detect_phash_cuts_and_entropy(hashes, fps)
    )
    duration_ms = int(fc / fps * MILLIS_PER_SEC)
    sparse_count = duration_ms // int(MILLIS_PER_SEC)
    indexed_bytes = (total_cuts_count + sparse_count) * CATALOG_INDEX_ENTRY_BYTES
    saved_pct = (
        round((1.0 - (indexed_bytes / float(len(raw_bytes)))) * PERCENT_SCALE, 1)
        if len(raw_bytes) > indexed_bytes
        else 0.0
    )
    ratio = round(len(raw_bytes) / float(indexed_bytes), 1) if indexed_bytes > 0 else 1.0

    return {
        "status": "ok",
        "filename": target_file.name,
        "moviehash": target_file.name.split(".")[0],
        "fps": fps,
        "frame_count": fc,
        "duration_ms": duration_ms,
        "duration_fmt": _format_duration(duration_ms),
        "file_size_bytes": len(raw_bytes),
        "file_size_human": format_human_size(len(raw_bytes)),
        "entropy": {
            "mean_popcount": round(mean_pop, 2),
            "entropy_pct": entropy_pct,
            "histogram": distribution,
            "health": (
                "Excellente (trame visuelle diversifiée)"
                if ENTROPY_HEALTH_MIN_POPCOUNT <= mean_pop <= ENTROPY_HEALTH_MAX_POPCOUNT
                else "Anormale (potentielle trame statique)"
            ),
        },
        "mobility": {
            "mean_delta": round(mean_delta, 2),
            "histogram": delta_dist,
            "labels": list(DELTA_MOBILITY_LABELS),
            "descriptions": list(DELTA_MOBILITY_DESCRIPTIONS),
        },
        "cuts": {
            "cuts_sample": cuts,
            "cuts_count": total_cuts_count,
            "threshold": PHASH_CUT_HAMMING_THRESHOLD,
            "timeline": timeline,
        },
        "compression": {
            "raw_bytes": len(raw_bytes),
            "estimated_indexed_bytes": indexed_bytes,
            "saved_space_pct": saved_pct,
            "ratio": ratio,
            "cuts_count": total_cuts_count,
            "sparse_count": sparse_count,
        },
    }
