#!/usr/bin/env python3
"""Autonomous Spatiotemporal Trajectory Tracking Batch Runner.

Executes concurrent, probe-free or video-based trajectory tracking evaluations
against .dbphash catalogs across video files or pre-extracted .phash files.

Features:
- Streaming JSONL output for crash-resilient, resumable batch runs.
- Dynamic operational controls:
  - Soft pause sentinel (`run/trajectory_bench_paused`)
  - Dynamic worker scale (`run/trajectory_bench_workers`)
  - Real-time JSON telemetry status (`run/trajectory_bench_status.json`)
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import signal
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Dict, List

DEFAULT_DBPHASH = "data/catalog.dbphash"
DEFAULT_STATUS_PATH = "run/trajectory_status.json"
DEFAULT_PAUSE_SENTINEL = "run/trajectory_paused"
DEFAULT_WORKERS_FILE = "run/trajectory_workers"
DEFAULT_REPORT_OUT = "data/reports/trajectory_batch_report.json"
DEFAULT_JSONL_OUT = "data/reports/trajectory_batch_results.jsonl"
DEFAULT_POLL_INTERVAL_SEC = 0.5
DEFAULT_CUTS_SPEC = "3"
DEFAULT_FRAMES_COUNT = 20
DEFAULT_WORKERS_COUNT = 4
MAX_WORKERS_LIMIT = 16
DEFAULT_TEMPORAL_SLACK_SEC = 3.5

SHUTDOWN_REQUESTED = False


def _sig_handler(_sig: int, _frame: Any) -> None:
    global SHUTDOWN_REQUESTED
    SHUTDOWN_REQUESTED = True
    print("\n🛑 Shutdown requested: finishing active jobs before exit...", flush=True)


signal.signal(signal.SIGINT, _sig_handler)
signal.signal(signal.SIGTERM, _sig_handler)


@dataclass
class TrajectoryBatchResult:
    target_path: str
    target_media: str
    start_sec: float
    nb_frames: int
    evaluated_frames_count: int
    initial_candidates_count: int
    surviving_trajectories_count: int
    winner_media_id: int | None
    winner_title: str
    winner_matched_frames: int
    winner_coverage_ratio: float
    winner_score: float
    winner_initial_timestamp_ms: int
    elapsed_seconds: float
    success: bool
    error_message: str | None = None


@dataclass
class MediaTrajectoryAudit:
    target_path: str
    target_media: str
    duration_sec: float
    cuts_count: int
    matched_cuts_count: int
    consensus_media_id: int | None
    consensus_title: str
    is_temporally_coherent: bool
    total_elapsed_seconds: float
    success: bool
    cuts: List[TrajectoryBatchResult]
    error_message: str | None = None


def check_temporal_coherence(matching_cuts: List[TrajectoryBatchResult]) -> bool:
    if len(matching_cuts) < 2:
        return True
    for i in range(len(matching_cuts) - 1):
        c1, c2 = matching_cuts[i], matching_cuts[i + 1]
        delta_target_sec = c2.start_sec - c1.start_sec
        delta_ref_sec = (c2.winner_initial_timestamp_ms - c1.winner_initial_timestamp_ms) / 1000.0
        if abs(delta_target_sec - delta_ref_sec) > DEFAULT_TEMPORAL_SLACK_SEC:
            return False
    return True


def get_media_duration_sec(media_path: str) -> float:
    p = Path(media_path)
    if p.suffix == ".phash":
        size_bytes = p.stat().st_size
        total_frames = size_bytes // 8
        fps = 24.0
        m = re.search(r"(\d+(?:\.\d+)?)fps", p.stem)
        if m:
            try:
                fps = float(m.group(1))
            except ValueError:
                fps = 24.0
        return total_frames / fps if fps > 0 else total_frames / 24.0
    return 3600.0


def compute_cut_offsets(duration_sec: float, cuts_spec: str) -> List[float]:
    if "," in cuts_spec:
        ratios = [float(x.strip()) for x in cuts_spec.split(",") if x.strip()]
    else:
        preset_map = {
            1: [0.50],
            3: [0.20, 0.50, 0.80],
            5: [0.10, 0.30, 0.50, 0.70, 0.90],
            7: [0.10, 0.25, 0.40, 0.55, 0.70, 0.85, 0.95],
        }
        try:
            n = int(cuts_spec)
            ratios = preset_map.get(n, [0.20, 0.50, 0.80])
        except ValueError:
            ratios = [0.20, 0.50, 0.80]
    return [round(r * duration_sec, 2) for r in ratios]


def elect_consensus_candidate(
    matched_cuts: List[TrajectoryBatchResult],
    cuts_count: int,
) -> tuple[int | None, str, bool]:
    """Elects consensus media ID and checks temporal coherence across matching cuts."""
    if not matched_cuts:
        return None, "NONE", False

    vote_counts: Dict[int, int] = {}
    title_map: Dict[int, str] = {}
    for c in matched_cuts:
        if c.winner_media_id is not None:
            vote_counts[c.winner_media_id] = vote_counts.get(c.winner_media_id, 0) + 1
            title_map[c.winner_media_id] = c.winner_title

    if not vote_counts:
        return None, "NONE", False

    top_id, count = max(vote_counts.items(), key=lambda kv: kv[1])
    min_required_votes = max(1, cuts_count // 2)
    if count < min_required_votes:
        return None, "NONE", False

    consensus_title = title_map.get(top_id, "NONE")
    id_cuts = [c for c in matched_cuts if c.winner_media_id == top_id]
    is_coherent = check_temporal_coherence(id_cuts)
    return top_id, consensus_title, is_coherent


def run_media_audit(
    dbsrt_bin: str,
    media_path: str,
    dbphash: str,
    cuts_spec: str,
    nb_frames: int,
    min_coverage: float,
    max_misses: int,
) -> MediaTrajectoryAudit:
    duration_sec = get_media_duration_sec(media_path)
    cut_offsets = compute_cut_offsets(duration_sec, cuts_spec)
    cut_results: List[TrajectoryBatchResult] = []
    t0 = time.perf_counter()

    for offset in cut_offsets:
        res = run_single_trajectory(dbsrt_bin, media_path, dbphash, offset, nb_frames, min_coverage, max_misses)
        cut_results.append(res)

    total_wall_sec = time.perf_counter() - t0
    matched_cuts = [c for c in cut_results if c.success and c.winner_media_id is not None]
    consensus_id, consensus_title, is_coherent = elect_consensus_candidate(matched_cuts, len(cut_offsets))

    return MediaTrajectoryAudit(
        target_path=media_path,
        target_media=Path(media_path).stem,
        duration_sec=round(duration_sec, 2),
        cuts_count=len(cut_offsets),
        matched_cuts_count=len(matched_cuts),
        consensus_media_id=consensus_id,
        consensus_title=consensus_title,
        is_temporally_coherent=is_coherent,
        total_elapsed_seconds=round(total_wall_sec, 3),
        success=True,
        cuts=cut_results,
    )


def find_dbsrt_binary() -> str:
    """Locates the dbsrt release binary or looks up in PATH."""
    repo_root = Path(__file__).resolve().parent.parent
    candidates = [
        repo_root / "target" / "release" / "dbsrt",
        repo_root / "bin" / "dbsrt",
        Path.cwd() / "bin" / "dbsrt",
        Path("bin/dbsrt"),
        Path("/usr/local/bin/dbsrt"),
    ]
    for cand in candidates:
        if cand.is_file() and os.access(cand, os.X_OK):
            return str(cand)
    return "dbsrt"


def run_single_trajectory(
    dbsrt_bin: str,
    media_path: str,
    dbphash: str,
    start_sec: float,
    nb_frames: int,
    min_coverage: float,
    max_misses: int,
) -> TrajectoryBatchResult:
    cmd = [
        dbsrt_bin,
        "match-trajectory",
        "--media",
        media_path,
        "--dbphash",
        dbphash,
        "-s",
        str(start_sec),
        "-n",
        str(nb_frames),
        "--min-coverage",
        str(min_coverage),
        "--max-misses",
        str(max_misses),
        "--output-format",
        "json",
    ]

    t0 = time.perf_counter()
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        total_wall_sec = time.perf_counter() - t0
        if proc.returncode != 0:
            return TrajectoryBatchResult(
                target_path=media_path,
                target_media=Path(media_path).stem,
                start_sec=start_sec,
                nb_frames=nb_frames,
                evaluated_frames_count=0,
                initial_candidates_count=0,
                surviving_trajectories_count=0,
                winner_media_id=None,
                winner_title="ERROR",
                winner_matched_frames=0,
                winner_coverage_ratio=0.0,
                winner_score=0.0,
                winner_initial_timestamp_ms=0,
                elapsed_seconds=total_wall_sec,
                success=False,
                error_message=proc.stderr.strip() or f"Process returned code {proc.returncode}",
            )

        data = json.loads(proc.stdout.strip())
        return TrajectoryBatchResult(
            target_path=media_path,
            target_media=data.get("target_media", Path(media_path).stem),
            start_sec=start_sec,
            nb_frames=nb_frames,
            evaluated_frames_count=data.get("evaluated_frames_count", 0),
            initial_candidates_count=data.get("initial_candidates_count", 0),
            surviving_trajectories_count=data.get("surviving_trajectories_count", 0),
            winner_media_id=data.get("winner_media_id"),
            winner_title=data.get("winner_title", "NONE"),
            winner_matched_frames=data.get("winner_matched_frames", 0),
            winner_coverage_ratio=data.get("winner_coverage_ratio", 0.0),
            winner_score=data.get("winner_score", 0.0),
            winner_initial_timestamp_ms=data.get("winner_initial_timestamp_ms", 0),
            elapsed_seconds=data.get("elapsed_seconds", total_wall_sec),
            success=True,
            error_message=None,
        )
    except Exception as exc:
        return TrajectoryBatchResult(
            target_path=media_path,
            target_media=Path(media_path).stem,
            start_sec=start_sec,
            nb_frames=nb_frames,
            evaluated_frames_count=0,
            initial_candidates_count=0,
            surviving_trajectories_count=0,
            winner_media_id=None,
            winner_title="EXCEPTION",
            winner_matched_frames=0,
            winner_coverage_ratio=0.0,
            winner_score=0.0,
            winner_initial_timestamp_ms=0,
            elapsed_seconds=time.perf_counter() - t0,
            success=False,
            error_message=str(exc),
        )


def read_desired_workers(workers_file: Path, default_workers: int) -> int:
    if not workers_file.is_file():
        return default_workers
    try:
        val = int(workers_file.read_text(encoding="utf-8").strip())
        return max(1, min(MAX_WORKERS_LIMIT, val))
    except (ValueError, OSError):
        return default_workers


def write_status(
    status_path: Path,
    total: int,
    completed: int,
    matched: int,
    pruned: int,
    errors: int,
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
        "matched": matched,
        "pruned": pruned,
        "errors": errors,
        "is_paused": is_paused,
        "active_workers": active_workers,
        "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    with open(temp_path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)
    temp_path.replace(status_path)


def load_completed_targets(jsonl_path: Path) -> set[str]:
    completed = set()
    if not jsonl_path.is_file():
        return completed
    with open(jsonl_path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
                if "target_path" in row:
                    completed.add(row["target_path"])
            except json.JSONDecodeError:
                continue
    return completed


def collect_dir_targets(dir_path: Path) -> List[str]:
    items: List[str] = [str(p) for p in dir_path.glob("**/*.phash")]
    for ext in ("*.mp4", "*.mkv", "*.avi", "*.webm"):
        items.extend(str(p) for p in dir_path.glob(f"**/{ext}"))
    return items


def collect_targets(targets_arg: List[str], phash_dir: str | None, limit: int | None) -> List[str]:
    items: List[str] = []
    if phash_dir and Path(phash_dir).is_dir():
        items.extend(str(p) for p in sorted(Path(phash_dir).glob("*.phash")))

    for arg in targets_arg:
        p = Path(arg)
        if p.is_file():
            items.append(str(p))
        elif p.is_dir():
            items.extend(collect_dir_targets(p))

    deduped = sorted(list(set(items)))
    return deduped[:limit] if limit and limit > 0 else deduped


def build_arg_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Autonomous Spatiotemporal Trajectory Tracking Batch Runner")
    p.add_argument("targets", nargs="*", help="Media files or directories to evaluate")
    p.add_argument("--phash-dir", help="Directory of .phash files to evaluate (e.g. data/__PHASH__)")
    p.add_argument("--dbphash", default=DEFAULT_DBPHASH, help="Path to .dbphash catalog file")
    p.add_argument(
        "-c",
        "--cuts",
        default=DEFAULT_CUTS_SPEC,
        help="Multi-cuts specification (e.g. '3', '5', or '0.2,0.5,0.8') [default: '3']",
    )
    p.add_argument(
        "-n", "--frames", type=int, default=DEFAULT_FRAMES_COUNT, help="Number of frames per cut (default: 20)"
    )
    p.add_argument("--min-coverage", type=float, default=0.50, help="Min coverage ratio (default: 0.50)")
    p.add_argument("--max-misses", type=int, default=3, help="Max consecutive misses (default: 3)")
    p.add_argument("-w", "--workers", type=int, default=DEFAULT_WORKERS_COUNT, help="Worker concurrency (default: 4)")
    p.add_argument("--limit", type=int, help="Optional maximum number of targets to evaluate")
    p.add_argument("--jsonl", default=DEFAULT_JSONL_OUT, help="Path to streaming output JSONL file")
    p.add_argument("--report", default=DEFAULT_REPORT_OUT, help="Path to final JSON summary report")
    p.add_argument("--status-file", default=DEFAULT_STATUS_PATH, help="Path to runtime telemetry JSON")
    p.add_argument("--pause-sentinel", default=DEFAULT_PAUSE_SENTINEL, help="Path to pause sentinel file")
    p.add_argument("--workers-file", default=DEFAULT_WORKERS_FILE, help="Path to dynamic workers file")
    return p


def process_future_result(
    fut: concurrent.futures.Future[MediaTrajectoryAudit],
    target_path: str,
    done_count: int,
    total: int,
    out_f: Any,
) -> tuple[int, int, int]:
    matched_inc, pruned_inc, error_inc = 0, 0, 0
    try:
        res = fut.result()
        out_f.write(json.dumps(asdict(res)) + "\n")
        out_f.flush()
        if res.success:
            if res.consensus_media_id is not None:
                matched_inc = 1
                coherent_tag = "coherent" if res.is_temporally_coherent else "drift"
                tag = f"✅ {res.consensus_title} ({res.matched_cuts_count}/{res.cuts_count} cuts, {coherent_tag})"
            else:
                pruned_inc = 1
                tag = f"⚪ NONE ({res.matched_cuts_count}/{res.cuts_count} cuts)"
        else:
            error_inc = 1
            tag = f"❌ ERROR: {res.error_message}"

        pct = done_count / total * 100.0 if total > 0 else 0.0
        print(
            f"[{done_count}/{total} - {pct:.1f}%] {Path(target_path).name[:40]:<40} -> {tag} ({res.total_elapsed_seconds:.2f}s)",
            flush=True,
        )
    except Exception as exc:
        error_inc = 1
        print(f"[{done_count}/{total}] Error evaluating {target_path}: {exc}", file=sys.stderr)
    return matched_inc, pruned_inc, error_inc


def spawn_job(
    executor: concurrent.futures.ThreadPoolExecutor,
    pending_iter: Any,
    in_flight: Dict[concurrent.futures.Future[MediaTrajectoryAudit], str],
    dbsrt_bin: str,
    args: argparse.Namespace,
) -> bool:
    try:
        target = next(pending_iter)
        fut = executor.submit(
            run_media_audit,
            dbsrt_bin,
            target,
            args.dbphash,
            args.cuts,
            args.frames,
            args.min_coverage,
            args.max_misses,
        )
        in_flight[fut] = target
        return True
    except StopIteration:
        return False


def drain_step(
    in_flight: Dict[concurrent.futures.Future[MediaTrajectoryAudit], str],
    executor: concurrent.futures.ThreadPoolExecutor,
    pending_iter: Any,
    dbsrt_bin: str,
    args: argparse.Namespace,
    total: int,
    done_count: int,
    out_f: Any,
    allow_spawn: bool = True,
) -> tuple[int, int, int, int]:
    done_futs, _ = concurrent.futures.wait(
        in_flight.keys(),
        timeout=DEFAULT_POLL_INTERVAL_SEC,
        return_when=concurrent.futures.FIRST_COMPLETED,
    )
    m_tot, p_tot, e_tot = 0, 0, 0
    cur_done = done_count
    for fut in done_futs:
        target_path = in_flight.pop(fut)
        cur_done += 1
        m, p, e = process_future_result(fut, target_path, cur_done, total, out_f)
        m_tot += m
        p_tot += p
        e_tot += e
        if allow_spawn and not SHUTDOWN_REQUESTED:
            spawn_job(executor, pending_iter, in_flight, dbsrt_bin, args)
    return cur_done, m_tot, p_tot, e_tot


def execute_batch(
    args: argparse.Namespace,
    dbsrt_bin: str,
    pending_targets: List[str],
    total: int,
    completed_paths: set[str],
) -> None:
    jsonl_path = Path(args.jsonl)
    jsonl_path.parent.mkdir(parents=True, exist_ok=True)
    status_path = Path(args.status_file)
    pause_sentinel = Path(args.pause_sentinel)
    workers_file = Path(args.workers_file)

    done_count = len(completed_paths)
    matched_count, pruned_count, error_count = 0, 0, 0
    current_workers = read_desired_workers(workers_file, args.workers)

    write_status(
        status_path,
        total,
        done_count,
        matched_count,
        pruned_count,
        error_count,
        pause_sentinel.exists(),
        current_workers,
    )

    with open(jsonl_path, "a", encoding="utf-8") as out_f:
        with concurrent.futures.ThreadPoolExecutor(max_workers=MAX_WORKERS_LIMIT) as executor:
            pending_iter = iter(pending_targets)
            in_flight: Dict[concurrent.futures.Future[MediaTrajectoryAudit], str] = {}

            while not SHUTDOWN_REQUESTED:
                is_paused = pause_sentinel.exists()
                current_workers = read_desired_workers(workers_file, args.workers)

                if is_paused:
                    if in_flight:
                        done_count, m, p, e = drain_step(
                            in_flight,
                            executor,
                            pending_iter,
                            dbsrt_bin,
                            args,
                            total,
                            done_count,
                            out_f,
                            allow_spawn=False,
                        )
                        matched_count += m
                        pruned_count += p
                        error_count += e
                    else:
                        time.sleep(DEFAULT_POLL_INTERVAL_SEC)
                    write_status(
                        status_path,
                        total,
                        done_count,
                        matched_count,
                        pruned_count,
                        error_count,
                        True,
                        len(in_flight),
                    )
                    continue

                # Not paused: refill in_flight up to current_workers
                while len(in_flight) < current_workers and not SHUTDOWN_REQUESTED:
                    if not spawn_job(executor, pending_iter, in_flight, dbsrt_bin, args):
                        break

                if not in_flight:
                    break

                done_count, m, p, e = drain_step(
                    in_flight,
                    executor,
                    pending_iter,
                    dbsrt_bin,
                    args,
                    total,
                    done_count,
                    out_f,
                    allow_spawn=True,
                )
                matched_count += m
                pruned_count += p
                error_count += e

                write_status(
                    status_path,
                    total,
                    done_count,
                    matched_count,
                    pruned_count,
                    error_count,
                    pause_sentinel.exists(),
                    len(in_flight),
                )

    print(
        f"\n🏁 Finished Trajectory Batch: {done_count}/{total} evaluated ({matched_count} matched, {pruned_count} pruned, {error_count} errors).",
        flush=True,
    )


def main() -> int:
    parser = build_arg_parser()
    args = parser.parse_args()

    dbsrt_bin = find_dbsrt_binary()
    all_targets = collect_targets(args.targets, args.phash_dir, args.limit)
    if not all_targets:
        print("❌ No targets found to evaluate.", file=sys.stderr)
        return 1

    jsonl_path = Path(args.jsonl)
    completed_paths = load_completed_targets(jsonl_path)
    pending_targets = [t for t in all_targets if t not in completed_paths]

    total = len(all_targets)
    done_count = len(completed_paths)
    print(
        f"🚀 Trajectory Tracking Batch: {total} total, {done_count} already completed, {len(pending_targets)} pending.",
        flush=True,
    )

    execute_batch(args, dbsrt_bin, pending_targets, total, completed_paths)
    return 0


if __name__ == "__main__":
    sys.exit(main())
