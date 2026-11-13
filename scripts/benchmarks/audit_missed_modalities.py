#!/usr/bin/env python3
"""Parametric Root-Cause Audit Engine for Multimodal Benchmark Misses.

Investigates discrepancies and false-negatives between visual (pHash) and acoustic (aHash)
modalities across arbitrary dataset sizes and streaming environments (Local CIFS / NAS FTP / Freebox HTTP).

Taxonomy of Root Causes:
  - CAT_1_AUDIO_DUB_MISMATCH: Container has multiple audio tracks; track 0 differs from catalog dub.
  - CAT_2_WINDOW_UNDER_SAMPLING_5S: 5s query window failed but 15s window matches nominally (dist <= 14.0b).
  - CAT_3_LOW_ENERGY_SILENCE: Sampled query window has low acoustic energy / silence (ones_ratio < 0.20 or > 0.80).
  - CAT_4_DIFFERENT_CUT_EDIT: Audio diverges completely across all windows despite single track (different rip/cut).
  - CAT_5_C2F_COARSE_PROBE_MISS: 5s query matches reference directly (dist <= 14.0b); coarse probe or shift consensus missed.
  - CAT_6_CONTAINER_READ_FAILURE: Streaming or container demuxing failed.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime
import json
import os
import struct
import subprocess
import tempfile
import time
import urllib.parse
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, List, Tuple

REPO_ROOT = Path(__file__).resolve().parents[2]

# Operational Constants
DEFAULT_INPUT_BENCHMARK = REPO_ROOT / "data/reports/eval_scale_multimodal_multiwindow_latest.jsonl"
DEFAULT_INPUT_AUDIT = REPO_ROOT / "data/reports/exhaustive_audit_missed_90_results.json"
DEFAULT_AHASH_DIR = REPO_ROOT / "data/__AHASH__"
DEFAULT_REPORTS_DIR = REPO_ROOT / "data/reports"
DEFAULT_WORKERS = 4
DEFAULT_QUERY_WINDOW_SEC = 5.0
DEFAULT_EXTENDED_WINDOW_SEC = 15.0
DEFAULT_MATCH_DISTANCE_THRESHOLD = 14.0
RANDOM_AUDIO_DISTANCE_BITS = 32.0
SILENCE_LOWER_RATIO = 0.20
SILENCE_UPPER_RATIO = 0.80
SLIDING_RADIUS_SEC = 30.0
DEFAULT_AUDIO_FPS = 86.125
HEADER_SIZE_BYTES = 176
SUBPROCESS_TIMEOUT_SECS = 30
HASH_WORD_BYTES = 8
HASH_BIT_WIDTH = 64
MAX_HAMMING_BITS = 64.0
MIDPOINT_RATIO = 0.5


def _first_env(*names: str) -> str:
    """Returns the first non-empty environment variable among `names`, else empty string."""
    for name in names:
        value = os.environ.get(name, "")
        if value:
            return value
    return ""


def _require_env(label: str, *names: str) -> str:
    """Returns the first non-empty env var among `names` or raises an explicit error."""
    value = _first_env(*names)
    if not value:
        raise RuntimeError(f"Missing {label}: set one of {', '.join(names)} in the environment to stream remote media.")
    return value


def resolve_ftp_credentials() -> Dict[str, str]:
    """Builds FTP_USER / FTP_PASSWORD env entries from the environment (never from the URL)."""
    return {
        "FTP_USER": _require_env("NAS FTP user", "DBSRT_NAS_USER", "FTP_USER"),
        "FTP_PASSWORD": _require_env("NAS FTP password", "DBSRT_NAS_PASS", "FTP_PASSWORD"),
    }


def resolve_stream_url(path_str: str) -> str:
    """Translates unmounted CIFS paths into active FTP or HTTP streaming URLs.

    FTP URLs never embed credentials (visible in /proc/<pid>/cmdline); callers pass
    them via FTP_USER / FTP_PASSWORD in the subprocess environment.

    NOTE: Never calls os.path.exists() on unmounted network mount points when on WAN,
    as stale kernel CIFS mounts block threads in cifs_wait_for_server_reconnect.

    Raises:
        RuntimeError: If a remote URL must be built but its env variables are missing.
    """
    syno_marker = "/Synology/"
    if syno_marker in path_str:
        rel_path = path_str.split(syno_marker, 1)[1]
        host = _require_env("NAS host", "DBSRT_NAS_HOST")
        return f"ftp://{host}/{rel_path}"

    freebox_marker = "/Freebox/"
    if freebox_marker in path_str:
        rel_path = path_str.split(freebox_marker, 1)[1]
        base = _require_env("Freebox HTTP base URL", "DBSRT_FREEBOX_HTTP_BASE").rstrip("/")
        enc_path = urllib.parse.quote(rel_path, safe="/")
        return f"{base}/{enc_path}"

    return path_str


def load_ahash_file(path: Path) -> Tuple[float, List[int]]:
    """Loads binary .ahash v1.0 file returning audio framerate and list of 64-bit frame hashes."""
    if not path.is_file():
        return (DEFAULT_AUDIO_FPS, [])
    try:
        data = path.read_bytes()
        if len(data) < HEADER_SIZE_BYTES:
            return (DEFAULT_AUDIO_FPS, [])
        _, sample_rate, _, hop_size = struct.unpack_from("<HIHH", data, 8)
        fps = sample_rate / hop_size if hop_size > 0 else DEFAULT_AUDIO_FPS
        payload = data[HEADER_SIZE_BYTES:]
        count = len(payload) // 8
        hashes = list(struct.unpack(f"<{count}Q", payload[: count * 8]))
        return (fps, hashes)
    except Exception:
        return (DEFAULT_AUDIO_FPS, [])


def extract_audio_hashes(media_path: str, start_sec: float, duration_sec: float) -> List[int]:
    """Invokes dbsrt export-ahash over local or remote stream inside an isolated TemporaryDirectory."""
    target_url = resolve_stream_url(media_path)
    run_env: Dict[str, str] | None = None
    if target_url.startswith("ftp://"):
        run_env = {**os.environ, **resolve_ftp_credentials()}
    dbsrt_bin = REPO_ROOT / "target/release/dbsrt"
    if not dbsrt_bin.exists():
        dbsrt_bin = Path("dbsrt")

    with tempfile.TemporaryDirectory(prefix="dbsrt_audit_") as tmp_dir:
        tmp_out = Path(tmp_dir) / f"window_{int(start_sec)}.ahash"
        cmd = [
            str(dbsrt_bin),
            "export-ahash",
            "--media",
            target_url,
            "--start-sec",
            f"{start_sec:.3f}",
            "--duration",
            f"{duration_sec:.3f}",
            "--output",
            str(tmp_out),
        ]
        try:
            res = subprocess.run(cmd, capture_output=True, timeout=SUBPROCESS_TIMEOUT_SECS, env=run_env)
            if res.returncode != 0:
                return []
            _, hashes = load_ahash_file(tmp_out)
            return hashes
        except Exception:
            return []


def min_sliding_distance(
    query: List[int],
    reference: List[int],
    center_sec: float,
    fps: float,
    radius_sec: float = SLIDING_RADIUS_SEC,
) -> Tuple[float, float, int]:
    """Computes minimum mean Hamming distance and SNR bits using CPU popcount bit_count()."""
    q_len = len(query)
    if q_len == 0 or len(reference) < q_len:
        return (64.0, 0.0, 0)

    center_frame = int(center_sec * fps)
    radius_frame = int(radius_sec * fps)
    min_f = max(0, center_frame - radius_frame)
    max_f = min(len(reference) - q_len, center_frame + radius_frame)

    if min_f > max_f:
        return (64.0, 0.0, 0)

    best_dist = 64.0
    best_shift = min_f

    for s in range(min_f, max_f + 1):
        sub = reference[s : s + q_len]
        dist = sum((q ^ r).bit_count() for q, r in zip(query, sub)) / q_len
        if dist < best_dist:
            best_dist = dist
            best_shift = s

    snr = max(0.0, RANDOM_AUDIO_DISTANCE_BITS - best_dist)
    return (best_dist, snr, best_shift)


@dataclass
class TargetAuditItem:
    idx: int
    media_stem: str
    media_path: str
    duration_sec: float
    cat_file: str
    track_count: int = 1
    track0_lang: str | None = None
    all_langs: List[str] = field(default_factory=list)
    phash_match: str | None = None
    previous_category: str | None = None


@dataclass
class DiagnosisResult:
    idx: int
    media_stem: str
    media_path: str
    duration_sec: float
    cat_file: str
    category: str
    diagnosis: str
    dist_5s: float
    snr_5s: float
    dist_15s: float | None
    snr_15s: float | None
    track_count: int
    track0_lang: str | None
    all_langs: List[str]
    phash_match: str | None


def diagnose_single_target(item: TargetAuditItem, ahash_dir: Path) -> DiagnosisResult:
    """Performs deep empirical diagnosis on a single target item against catalog reference."""
    ref_path = ahash_dir / item.cat_file
    if not ref_path.exists():
        return DiagnosisResult(
            idx=item.idx,
            media_stem=item.media_stem,
            media_path=item.media_path,
            duration_sec=item.duration_sec,
            cat_file=item.cat_file,
            category="CAT_7_REFERENCE_MISSING",
            diagnosis="Reference .ahash file not found in catalog",
            dist_5s=64.0,
            snr_5s=0.0,
            dist_15s=None,
            snr_15s=None,
            track_count=item.track_count,
            track0_lang=item.track0_lang,
            all_langs=item.all_langs or [],
            phash_match=item.phash_match,
        )

    ref_fps, ref_hashes = load_ahash_file(ref_path)
    if not ref_hashes:
        return DiagnosisResult(
            idx=item.idx,
            media_stem=item.media_stem,
            media_path=item.media_path,
            duration_sec=item.duration_sec,
            cat_file=item.cat_file,
            category="CAT_7_REFERENCE_EMPTY",
            diagnosis="Reference .ahash file payload is empty",
            dist_5s=64.0,
            snr_5s=0.0,
            dist_15s=None,
            snr_15s=None,
            track_count=item.track_count,
            track0_lang=item.track0_lang,
            all_langs=item.all_langs or [],
            phash_match=item.phash_match,
        )

    center_sec = item.duration_sec * 0.5
    q5_hashes = extract_audio_hashes(item.media_path, center_sec, DEFAULT_QUERY_WINDOW_SEC)

    if not q5_hashes:
        return DiagnosisResult(
            idx=item.idx,
            media_stem=item.media_stem,
            media_path=item.media_path,
            duration_sec=item.duration_sec,
            cat_file=item.cat_file,
            category="CAT_6_CONTAINER_READ_FAILURE",
            diagnosis="Failed to stream or decode audio from media source",
            dist_5s=64.0,
            snr_5s=0.0,
            dist_15s=None,
            snr_15s=None,
            track_count=item.track_count,
            track0_lang=item.track0_lang,
            all_langs=item.all_langs or [],
            phash_match=item.phash_match,
        )

    total_ones = sum(h.bit_count() for h in q5_hashes)
    ones_ratio = total_ones / (len(q5_hashes) * 64)
    is_silent = ones_ratio < SILENCE_LOWER_RATIO or ones_ratio > SILENCE_UPPER_RATIO

    dist_5s, snr_5s, _ = min_sliding_distance(q5_hashes, ref_hashes, center_sec, ref_fps)

    dist_15s: float | None = None
    snr_15s: float | None = None

    if is_silent:
        category = "CAT_3_LOW_ENERGY_SILENCE"
        diagnosis = f"Query window at 50% is silent/flat (ones_ratio={ones_ratio:.2f})"
    elif dist_5s <= DEFAULT_MATCH_DISTANCE_THRESHOLD:
        category = "CAT_5_C2F_COARSE_PROBE_MISS"
        diagnosis = f"5s window matches reference at dist={dist_5s:.1f}b (SNR={snr_5s:.1f}b); missed by coarse probe or consensus"
    else:
        # Test 15s window
        q15_hashes = extract_audio_hashes(item.media_path, center_sec, DEFAULT_EXTENDED_WINDOW_SEC)
        dist_15s, snr_15s, _ = min_sliding_distance(q15_hashes, ref_hashes, center_sec, ref_fps)

        if dist_15s <= DEFAULT_MATCH_DISTANCE_THRESHOLD:
            category = "CAT_2_WINDOW_UNDER_SAMPLING_5S"
            diagnosis = (
                f"5s failed (dist={dist_5s:.1f}b) but 15s matches nominally (dist={dist_15s:.1f}b, SNR={snr_15s:.1f}b)"
            )
        elif item.track_count > 1:
            category = "CAT_1_AUDIO_DUB_MISMATCH"
            diagnosis = f"Track 0 ({item.track0_lang or 'unknown'}) diverges (dist={dist_5s:.1f}b); multi-track dub present: {item.all_langs}"
        else:
            category = "CAT_4_DIFFERENT_CUT_EDIT"
            diagnosis = f"Audio diverges completely (dist={dist_5s:.1f}b, SNR={snr_5s:.1f}b) despite single track (cut/master variation)"

    return DiagnosisResult(
        idx=item.idx,
        media_stem=item.media_stem,
        media_path=item.media_path,
        duration_sec=item.duration_sec,
        cat_file=item.cat_file,
        category=category,
        diagnosis=diagnosis,
        dist_5s=round(dist_5s, 2),
        snr_5s=round(snr_5s, 2),
        dist_15s=round(dist_15s, 2) if dist_15s is not None else None,
        snr_15s=round(snr_15s, 2) if snr_15s is not None else None,
        track_count=item.track_count,
        track0_lang=item.track0_lang,
        all_langs=item.all_langs or [],
        phash_match=item.phash_match,
    )


def load_targets_from_audit(audit_file: Path, filter_cat: str) -> List[TargetAuditItem]:
    """Loads targets from a previous audit JSON, filtering by category."""
    with open(audit_file, encoding="utf-8") as f:
        raw_items = json.load(f)

    targets = []
    for it in raw_items:
        prev_cat = it.get("category", "")
        if filter_cat.upper() != "ALL" and prev_cat != filter_cat:
            continue
        targets.append(
            TargetAuditItem(
                idx=it.get("idx", len(targets) + 1),
                media_stem=it.get("media_stem", ""),
                media_path=it.get("media_path", ""),
                duration_sec=it.get("duration_sec", 0.0),
                cat_file=it.get("cat_file", ""),
                track_count=it.get("track_count", 1),
                track0_lang=it.get("track0_lang"),
                all_langs=it.get("all_langs", []),
                phash_match=it.get("phash_match"),
                previous_category=prev_cat,
            )
        )
    return targets


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Parametric root-cause audit engine for multimodal benchmark misses.")
    parser.add_argument(
        "--from-audit",
        type=Path,
        default=DEFAULT_INPUT_AUDIT,
        help="Path to previous audit JSON file to refine.",
    )
    parser.add_argument(
        "--filter-cat",
        type=str,
        default="CAT_6_CONTAINER_READ_FAILURE",
        help="Category to filter and re-audit (e.g. 'CAT_6_CONTAINER_READ_FAILURE' or 'ALL').",
    )
    parser.add_argument(
        "--ahash-dir",
        type=Path,
        default=DEFAULT_AHASH_DIR,
        help="Directory containing catalog .ahash reference files.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Destination JSON path for detailed audit results.",
    )
    parser.add_argument(
        "--workers",
        type=int,
        default=DEFAULT_WORKERS,
        help="Number of concurrent audio extraction workers (default: 4).",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="Maximum items to audit (0 for all matching items).",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    reports_dir = DEFAULT_REPORTS_DIR
    reports_dir.mkdir(parents=True, exist_ok=True)

    timestamp_slug = datetime.datetime.now(datetime.UTC).strftime("%Y%m%d_%H%M%S")
    out_file = args.output or reports_dir / f"audit_multimodal_misses_{timestamp_slug}.json"
    latest_link = reports_dir / "audit_multimodal_misses_latest.json"

    targets = load_targets_from_audit(args.from_audit, args.filter_cat)
    if args.limit > 0:
        targets = targets[: args.limit]

    print("=== PARAMETRIC MULTIMODAL MISSES AUDIT ===", flush=True)
    print(f"Input source      : {args.from_audit}", flush=True)
    print(f"Filter category   : {args.filter_cat} ({len(targets)} targets)", flush=True)
    print(f"Workers           : {args.workers}", flush=True)
    print(f"Output report     : {out_file}", flush=True)
    print(f"Acoustic catalog  : {args.ahash_dir}", flush=True)

    if not targets:
        print("✓ No targets matching specified category filter.", flush=True)
        return

    results: List[DiagnosisResult] = []
    category_counts: Dict[str, int] = {}
    completed_count = 0
    total_count = len(targets)
    start_time = time.time()

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        future_map = {executor.submit(diagnose_single_target, target, args.ahash_dir): target for target in targets}
        for future in concurrent.futures.as_completed(future_map):
            completed_count += 1
            res = future.result()
            results.append(res)
            category_counts[res.category] = category_counts.get(res.category, 0) + 1
            print(
                f"[{completed_count:>2}/{total_count}] ({completed_count / total_count * 100:>5.1f}%) | "
                f"{res.category:<30} | {res.media_stem[:40]}",
                flush=True,
            )

    results.sort(key=lambda r: r.idx)
    with open(out_file, "w", encoding="utf-8") as f:
        json.dump([asdict(r) for r in results], f, indent=2)

    # Update latest pointer
    try:
        if latest_link.is_symlink() or latest_link.exists():
            latest_link.unlink()
        latest_link.symlink_to(out_file.name)
    except Exception:
        pass

    print(f"\n✓ Completed audit across {len(results)} targets in {time.time() - start_time:.1f}s", flush=True)
    print(f"✓ Saved results to: {out_file}", flush=True)
    print("\n=== REQUALIFICATION SUMMARY ===", flush=True)
    for cat, cnt in sorted(category_counts.items(), key=lambda x: -x[1]):
        pct = (cnt / len(results)) * 100 if results else 0.0
        print(f"  • {cat:<32} : {cnt:>2} ({pct:>5.1f}%)", flush=True)


if __name__ == "__main__":
    main()
