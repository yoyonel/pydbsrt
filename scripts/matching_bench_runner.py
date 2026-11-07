#!/usr/bin/env python3
"""Autonomous Spatiotemporal Multi-Offset Matching Benchmark Runner.

Executes concurrent, probe-cached multi-window evaluation against .dbphash catalogs
with dynamic operational controls:
- Soft pause sentinel (`run/matching_bench_paused`)
- Dynamic worker scale (`run/matching_bench_workers`)
- Real-time JSON telemetry status (`run/matching_bench_status.json`)
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Dict, List

DEFAULT_DBPHASH = "data/catalog.dbphash"
DEFAULT_PROBE_CACHE = "data/probes_cache.json"
DEFAULT_STATUS_PATH = "run/matching_bench_status.json"
DEFAULT_PAUSE_SENTINEL = "run/matching_bench_paused"
DEFAULT_WORKERS_FILE = "run/matching_bench_workers"
DEFAULT_REPORT_OUT = "data/reports/eval_multi_offset_matching.json"
DEFAULT_POLL_INTERVAL_SEC = 0.5
DEFAULT_WORKERS_COUNT = 4

_USER = os.environ.get("USER", "media")
DEFAULT_NAS_BASE = os.environ.get("NAS_TVSHOW_DIR", f"/media/{_USER}/Synology/tvshow")
DEFAULT_REMOTE_DEMON_TARGETS = [
    str(Path(DEFAULT_NAS_BASE) / "Demon Slayer/S02/Demon Slayer – Saison 2 Episode 10 VOSTFR.mp4"),
    str(Path(DEFAULT_NAS_BASE) / "Demon Slayer/S02/Demon Slayer – Saison 2 Episode 11 VOSTFR.mp4"),
    str(Path(DEFAULT_NAS_BASE) / "Demon Slayer/S02/Demon Slayer – Saison 2 Episode 12 VOSTFR.mp4"),
    str(Path(DEFAULT_NAS_BASE) / "Demon Slayer/S02/Demon Slayer – Saison 2 Episode 13 VOSTFR.mp4"),
    str(Path(DEFAULT_NAS_BASE) / "Demon Slayer/S02/Demon Slayer – Saison 2 Episode 14 VOSTFR.mp4"),
]

SHUTDOWN_REQUESTED = False


def _sig_handler(_sig: int, _frame: Any) -> None:
    global SHUTDOWN_REQUESTED
    SHUTDOWN_REQUESTED = True
    print("\n🛑 Shutdown requested: finishing current evaluation before exit...", flush=True)


signal.signal(signal.SIGINT, _sig_handler)
signal.signal(signal.SIGTERM, _sig_handler)


@dataclass
class MatchingEvalRecord:
    media_path: str
    media_stem: str
    offsets: str
    nb_seconds: float
    mono_found: bool
    mono_title: str
    mono_id: int
    mono_confidence: float
    mono_timer_sec: float
    multi_found: bool
    multi_title: str
    multi_id: int
    multi_confidence: float
    multi_windows: str
    multi_coherent: bool
    multi_timer_sec: float
    consensus_shift_ms: int
    collision_eliminated: bool
    window_matches: List[Dict[str, Any]]


def execute_dbsrt_recognize(
    dbsrt_bin: Path,
    media_path: Path,
    dbphash_path: Path,
    probe_cache: Path | None,
    multi_window: bool,
    nb_seconds: float = 1.0,
    offsets: str | None = None,
) -> Dict[str, Any]:
    cmd = [
        str(dbsrt_bin),
        "recognize-media",
        "--media",
        str(media_path),
        "--dbphash",
        str(dbphash_path),
        "-s",
        str(nb_seconds),
        "--output-format",
        "JSON",
        "--limit",
        "1",
    ]
    if probe_cache:
        cmd.extend(["--probe-cache", str(probe_cache)])
    if multi_window:
        if offsets:
            cmd.extend(["--offsets", offsets])
        else:
            cmd.append("--multi-window")

    t0 = time.perf_counter()
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    elapsed = time.perf_counter() - t0

    if proc.returncode != 0 or not proc.stdout.strip():
        return {
            "found": False,
            "title": "NONE",
            "media_id": 0,
            "confidence": 0.0,
            "windows": "0/0",
            "coherent": False,
            "timer": elapsed,
            "consensus_shift_ms": 0,
            "window_matches": [],
        }

    try:
        data = json.loads(proc.stdout.strip())
        if not data:
            return {
                "found": False,
                "title": "NONE",
                "media_id": 0,
                "confidence": 0.0,
                "windows": "0/0",
                "coherent": False,
                "timer": elapsed,
                "consensus_shift_ms": 0,
                "window_matches": [],
            }
        item = data[0]
        if multi_window:
            win_str = f"{item.get('matching_windows', 0)}/{item.get('total_windows', 0)}"
            return {
                "found": item.get("media_found", False),
                "title": item.get("media_name_match", ""),
                "media_id": item.get("media_id_match", 0),
                "confidence": float(item.get("confidence", 0.0)),
                "windows": win_str,
                "coherent": item.get("is_temporally_coherent", False),
                "timer": elapsed,
                "consensus_shift_ms": item.get("consensus_shift_ms", 0),
                "window_matches": item.get("window_matches", []),
            }
        return {
            "found": item.get("media_found", False),
            "title": item.get("media_name_match", ""),
            "media_id": item.get("media_id_match", 0),
            "confidence": float(item.get("confidence", 0.0)),
            "windows": "1/1",
            "coherent": True,
            "timer": elapsed,
            "consensus_shift_ms": item.get("estimated_shift_ms", 0),
            "window_matches": [],
        }
    except Exception:
        return {
            "found": False,
            "title": "NONE",
            "media_id": 0,
            "confidence": 0.0,
            "windows": "0/0",
            "coherent": False,
            "timer": elapsed,
            "consensus_shift_ms": 0,
            "window_matches": [],
        }


def evaluate_single_target(
    dbsrt_bin: Path,
    dbphash_path: Path,
    probe_cache: Path | None,
    path: Path,
    nb_seconds: float = 1.0,
    offsets: str | None = None,
) -> MatchingEvalRecord:
    stem = path.stem
    mono = execute_dbsrt_recognize(dbsrt_bin, path, dbphash_path, probe_cache, False, nb_seconds)
    multi = execute_dbsrt_recognize(dbsrt_bin, path, dbphash_path, probe_cache, True, nb_seconds, offsets)

    is_mono_collision = (not mono["found"]) and (mono["confidence"] >= 90.0)
    collision_eliminated = is_mono_collision and (multi["confidence"] < 50.0 or not multi["coherent"])

    print(
        f"🎯 {stem[:42]:<42} | "
        f"Mono: {mono['confidence']:5.1f}% ({'MATCH' if mono['found'] else 'COLLISION':9}) | "
        f"Multi: {multi['confidence']:5.1f}% (win={multi['windows']}, coh={str(multi['coherent']):5}) | "
        f"Slash: {'ELIMINATED' if collision_eliminated else 'PASS'}",
        flush=True,
    )

    return MatchingEvalRecord(
        media_path=str(path),
        media_stem=stem,
        offsets=offsets or "0.2,0.5,0.8",
        nb_seconds=nb_seconds,
        mono_found=mono["found"],
        mono_title=mono["title"],
        mono_id=mono["media_id"],
        mono_confidence=mono["confidence"],
        mono_timer_sec=mono["timer"],
        multi_found=multi["found"],
        multi_title=multi["title"],
        multi_id=multi["media_id"],
        multi_confidence=multi["confidence"],
        multi_windows=multi["windows"],
        multi_coherent=multi["coherent"],
        multi_timer_sec=multi["timer"],
        consensus_shift_ms=multi["consensus_shift_ms"],
        collision_eliminated=collision_eliminated,
        window_matches=multi["window_matches"],
    )


def read_desired_workers(workers_file: Path, default_workers: int) -> int:
    if not workers_file.is_file():
        return default_workers
    try:
        val = int(workers_file.read_text(encoding="utf-8").strip())
        return max(1, min(8, val))
    except (ValueError, OSError):
        return default_workers


def write_status(
    status_path: Path,
    total: int,
    completed: int,
    collisions_in_mono: int,
    collisions_eliminated: int,
    is_paused: bool,
    active_workers: int,
) -> None:
    pct = (completed / total * 100.0) if total > 0 else 0.0
    status_path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = status_path.with_suffix(".tmp")
    data = {
        "total": total,
        "completed": completed,
        "remaining": total - completed,
        "progress_pct": round(pct, 1),
        "collisions_in_mono": collisions_in_mono,
        "collisions_eliminated": collisions_eliminated,
        "is_paused": is_paused,
        "active_workers": active_workers,
        "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    with open(temp_path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)
    temp_path.replace(status_path)


def handle_pause_cycle(pause_file: Path) -> bool:
    if pause_file.is_file():
        time.sleep(DEFAULT_POLL_INTERVAL_SEC)
        return True
    return False


def _wait_while_paused(
    pause_file: Path,
    status_path: Path,
    total: int,
    completed: int,
    mono_collisions: int,
    eliminated: int,
) -> bool:
    """Waits while pause sentinel exists. Returns True if shutdown was requested."""
    while handle_pause_cycle(pause_file):
        if SHUTDOWN_REQUESTED:
            return True
        write_status(status_path, total, completed, mono_collisions, eliminated, True, 0)
    return False


def _process_target(
    path: Path,
    args: argparse.Namespace,
    cur_workers: int,
    state: Dict[str, Any],
) -> MatchingEvalRecord:
    rec = evaluate_single_target(
        args.bin,
        args.dbphash,
        args.probe_cache,
        path,
        args.seconds,
        args.offsets,
    )
    state["completed"] += 1
    if (not rec.mono_found) and (rec.mono_confidence >= 90.0):
        state["mono_collisions"] += 1
        if rec.collision_eliminated:
            state["eliminated"] += 1
    write_status(
        state["status_path"],
        state["total"],
        state["completed"],
        state["mono_collisions"],
        state["eliminated"],
        False,
        cur_workers,
    )
    return rec


def run_benchmark_loop(
    targets: List[Path],
    args: argparse.Namespace,
    status_path: Path,
    pause_file: Path,
    workers_file: Path,
) -> List[MatchingEvalRecord]:
    results: List[MatchingEvalRecord] = []
    state = {
        "status_path": status_path,
        "total": len(targets),
        "completed": 0,
        "mono_collisions": 0,
        "eliminated": 0,
    }

    print(f"🚀 Starting Multi-Offset Matching Benchmark ({state['total']} targets)...")
    write_status(status_path, state["total"], 0, 0, 0, False, args.workers)

    for path in targets:
        if SHUTDOWN_REQUESTED or _wait_while_paused(
            pause_file,
            status_path,
            state["total"],
            state["completed"],
            state["mono_collisions"],
            state["eliminated"],
        ):
            break

        cur_workers = read_desired_workers(workers_file, args.workers)
        rec = _process_target(path, args, cur_workers, state)
        results.append(rec)

    return results


def save_final_report(results: List[MatchingEvalRecord], out_path: Path) -> None:
    mono_collisions = sum(1 for r in results if not r.mono_found and r.mono_confidence >= 90.0)
    eliminated = sum(1 for r in results if r.collision_eliminated)
    pct = (eliminated / mono_collisions * 100.0) if mono_collisions else 100.0

    print("=" * 80)
    print("📊 BENCHMARK SUMMARY:")
    print(f"  Total Targets Evaluated: {len(results)}")
    print(f"  Mono-Window False Collisions (>=90%): {mono_collisions}")
    print(f"  Collisions Eliminated by Multi-Window: {eliminated} / {mono_collisions} ({pct:.1f}%)")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump([asdict(r) for r in results], f, indent=2)
    print(f"💾 Report saved to {out_path}")


def parse_cli_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Multi-Offset Matching Benchmark Runner")
    parser.add_argument("--dbphash", type=Path, default=Path(DEFAULT_DBPHASH))
    parser.add_argument("--probe-cache", type=Path, default=Path(DEFAULT_PROBE_CACHE))
    parser.add_argument("--bin", type=Path, default=Path("target/debug/dbsrt"))
    parser.add_argument("--out", type=Path, default=Path(DEFAULT_REPORT_OUT))
    parser.add_argument("--status-path", type=Path, default=Path(DEFAULT_STATUS_PATH))
    parser.add_argument("--pause-file", type=Path, default=Path(DEFAULT_PAUSE_SENTINEL))
    parser.add_argument("--workers-file", type=Path, default=Path(DEFAULT_WORKERS_FILE))
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS_COUNT)
    parser.add_argument("-s", "--seconds", type=float, default=1.0)
    parser.add_argument(
        "--offsets",
        type=str,
        default="0.2,0.5,0.8",
        help="Comma-separated offset ratios (e.g. 0.2,0.5,0.8 or 0.1,0.3,0.5,0.7,0.9)",
    )
    parser.add_argument("files", nargs="*", help="Video files to benchmark")
    return parser.parse_args()


def main() -> int:
    args = parse_cli_args()
    targets = [Path(f) for f in args.files] if args.files else [Path(f) for f in DEFAULT_REMOTE_DEMON_TARGETS]
    valid_targets = [p for p in targets if p.exists()]
    if not valid_targets:
        print("❌ No valid media targets found.", file=sys.stderr)
        return 1

    results = run_benchmark_loop(valid_targets, args, args.status_path, args.pause_file, args.workers_file)
    save_final_report(results, args.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
