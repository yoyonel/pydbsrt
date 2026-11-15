"""Dynamic autoscaling worker pool (`_BatchRun`) and batch ingestion scheduler."""

from __future__ import annotations

import contextlib
import sys
import threading
import time
from pathlib import Path
from typing import Callable, Dict, List, Tuple

from ._facade import facade_attr
from .models import (
    CPU_BUDGET_VCPUS,
    DEFAULT_AHASH_DIR,
    DEFAULT_BATCH_PAUSED_PATH,
    DEFAULT_DESIRED_WORKERS_PATH,
    DEFAULT_FFMPEG_THREADS,
    FAST_FAILURE_SECS,
    MAX_CONCURRENT_WORKERS,
    MAX_FAST_FAILURE_STREAK,
    MIN_CONCURRENT_WORKERS,
    RELOAD_DRAIN_SECS,
    TARGET_QUEUE_POLL_INTERVAL_SECS,
    VideoTarget,
)
from .runner import RELOAD_REQUESTED, _kill_active_children, process_single_video, script_updated
from .telemetry import abort_batch, next_fast_failure_streak, write_batch_queue_state


def max_workers_for_budget(ffmpeg_threads: str) -> int:
    """Highest concurrency whose FFmpeg decode threads fit inside `CPU_BUDGET_VCPUS`."""
    budget = facade_attr("CPU_BUDGET_VCPUS", CPU_BUDGET_VCPUS)
    threads = int(ffmpeg_threads) if ffmpeg_threads.isdigit() else 0
    per_job = threads if threads > 0 else budget
    return max(1, min(MAX_CONCURRENT_WORKERS, budget // per_job))


def read_desired_workers(default_workers: int, max_workers: int = MAX_CONCURRENT_WORKERS) -> int:
    """Reads desired concurrency dynamically from `run/desired_workers` or `run/batch_paused`."""
    if DEFAULT_BATCH_PAUSED_PATH.is_file():
        return 0
    val = default_workers
    if DEFAULT_DESIRED_WORKERS_PATH.is_file():
        with contextlib.suppress(OSError, ValueError):
            val = int(DEFAULT_DESIRED_WORKERS_PATH.read_text(encoding="utf-8").strip())
    return max(MIN_CONCURRENT_WORKERS, min(val, max_workers))


class _BatchRun:
    """Thread-safe queue, counters, and autoscaled worker threads for one batch execution."""

    def __init__(
        self,
        targets: List[VideoTarget],
        run_video: Callable[[VideoTarget], Dict[str, object]],
        initial_workers: int,
        max_workers: int,
    ) -> None:
        self.total = len(targets)
        self.remaining = list(targets)
        self.run_video = run_video
        self.initial_workers = initial_workers
        self.max_workers = max_workers
        self.state_lock = threading.Lock()
        self.threads: List[threading.Thread] = []
        self.success_count = 0
        self.fail_count = 0
        self.fast_failure_streak = 0
        self.reload_since = 0.0

    def desired_workers(self) -> int:
        return read_desired_workers(self.initial_workers, self.max_workers)

    def publish(self, active_workers: int | None = None) -> None:
        active = len(self.threads) if active_workers is None else active_workers
        desired = self.desired_workers()
        is_paused = DEFAULT_BATCH_PAUSED_PATH.is_file() or desired == 0
        writer = facade_attr("write_batch_queue_state", write_batch_queue_state)
        try:
            writer(
                self.total, self.success_count, self.fail_count, active, desired_workers=desired, is_paused=is_paused
            )
        except TypeError:
            writer(self.total, self.success_count, self.fail_count, active)

    def worker_loop(self) -> None:
        while True:
            with self.state_lock:
                if len(self.threads) > self.desired_workers() or self.desired_workers() == 0 or not self.remaining:
                    return
                item = self.remaining.pop(0)
            try:
                rec = self.run_video(item)
            except Exception as exc:
                with self.state_lock:
                    self.fail_count += 1
                    print(f"  ❌ Exception processing {item.filename}: {exc}", flush=True)
                    self.publish()
                continue
            if rec.get("status") == "RELOAD":
                return
            with self.state_lock:
                self.record(rec)
                self.publish()

    def record(self, rec: Dict[str, object]) -> None:
        if rec.get("success"):
            self.success_count += 1
            print(
                f"  ✓ [{self.success_count}/{self.total}] {rec['video_stem']} "
                f"({rec['wall_duration_secs']}s, {rec['io_read_rate_mib_s']} MB/s)",
                flush=True,
            )
        else:
            self.fail_count += 1
            print(
                f"  ❌ [{self.fail_count} FAILED] {rec['video_stem']}: {rec.get('error_summary', 'Unknown error')}",
                flush=True,
            )
        self.fast_failure_streak = next_fast_failure_streak(self.fast_failure_streak, rec)
        if self.fast_failure_streak == MAX_FAST_FAILURE_STREAK:
            abort_batch(
                self.remaining,
                f"{self.fast_failure_streak} consecutive failures under {FAST_FAILURE_SECS:.0f}s: "
                "systemic error, check logs/workers/ before relaunching",
            )

    def check_reload(self) -> None:
        updated_fn = facade_attr("script_updated", script_updated)
        drain_secs = facade_attr("RELOAD_DRAIN_SECS", RELOAD_DRAIN_SECS)
        if self.remaining and updated_fn():
            print("🔄 [RELOAD] Worker script updated: finishing in-flight videos before re-exec", flush=True)
            self.remaining.clear()
            RELOAD_REQUESTED.set()
            self.reload_since = time.time()
        if self.reload_since and time.time() - self.reload_since >= drain_secs:
            print("🔄 [RELOAD] Drain timeout: stopping in-flight videos, the re-exec requeues them", flush=True)
            _kill_active_children()
            self.reload_since = 0.0

    def schedule(self) -> bool:
        with self.state_lock:
            self.check_reload()
            alive = [t for t in self.threads if t.is_alive()]
            exited = len(alive) != len(self.threads)
            self.threads = alive
            if exited:
                self.publish()
            if not self.remaining and not self.threads:
                return False
            desired = self.desired_workers()
            while len(self.threads) < desired and self.remaining:
                thread = threading.Thread(target=self.worker_loop, daemon=True)
                self.threads.append(thread)
                thread.start()
                self.publish()
        return True


def execute_dynamic_batch_ingestion(
    targets: List[VideoTarget],
    stream_base: str,
    dbsrt_bin: str,
    child_env: Dict[str, str],
    events_log: Path,
    max_extraction_secs: int,
    initial_workers: int,
    ahash_dir: str = DEFAULT_AHASH_DIR,
    mode: str = "both",
) -> Tuple[int, int, float]:
    """Executes batch processing with dynamic worker scaling via `run/desired_workers`."""
    log_lock = threading.Lock()
    DEFAULT_DESIRED_WORKERS_PATH.parent.mkdir(parents=True, exist_ok=True)
    if not DEFAULT_DESIRED_WORKERS_PATH.exists():
        try:
            DEFAULT_DESIRED_WORKERS_PATH.write_text(str(initial_workers), encoding="utf-8")
        except OSError as err:
            print(f"⚠️ Failed writing {DEFAULT_DESIRED_WORKERS_PATH}: {err}", file=sys.stderr)

    max_workers = max_workers_for_budget(child_env.get("DBSRT_FFMPEG_THREADS", DEFAULT_FFMPEG_THREADS))
    budget = facade_attr("CPU_BUDGET_VCPUS", CPU_BUDGET_VCPUS)
    print(f"CPU budget: {budget} vCPU, at most {max_workers} concurrent videos", flush=True)

    def run_video(target: VideoTarget) -> Dict[str, object]:
        proc_fn = facade_attr("process_single_video", process_single_video)
        return proc_fn(
            target,
            stream_base,
            dbsrt_bin,
            child_env,
            log_lock,
            events_log,
            max_extraction_secs,
            None,
            None,
            ahash_dir,
            mode,
        )

    batch = _BatchRun(targets, run_video, initial_workers, max_workers)
    batch.publish(initial_workers)
    start_batch = time.time()
    try:
        while batch.schedule():
            time.sleep(facade_attr("TARGET_QUEUE_POLL_INTERVAL_SECS", TARGET_QUEUE_POLL_INTERVAL_SECS))
    finally:
        batch.publish(0)
    return batch.success_count, batch.fail_count, time.time() - start_batch
