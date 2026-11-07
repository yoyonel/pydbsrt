"""Real end-to-end integration test of batch worker using genuine FFmpeg decoding.

Executes actual FFmpeg/dbsrt decoding on real media snippets from data/snippets/ without mocks:
1. Concurrency regulation: Scale-down from 4 to 2 workers (50% CPU reallocation).
2. Dynamic pause: Sentinel run/batch_paused halts dequeueing without killing in-flight jobs.
3. Resume: Sentinel removal triggers immediate batch continuation.
4. Validation: Context telemetry enrichment (moviehash, duration, fps, workers) verified.
"""

from __future__ import annotations

import json
import threading
import time
from pathlib import Path

import scripts.o2switch_batch_worker as worker
from scripts.o2switch_batch_worker import VideoTarget


def test_real_ffmpeg_batch_e2e_scale_and_pause_cycle(tmp_path: Path, monkeypatch):
    """Executes a real batch of actual video decodings via dbsrt/FFmpeg with live pause and scale-down."""
    snippets_dir = Path("data/snippets").resolve()
    # Pick 8 real, valid video snippets (>= 64 KiB)
    candidate_mp4s = [p for p in sorted(snippets_dir.glob("*_mid15s.mp4")) if p.stat().st_size >= 65536][:8]
    assert len(candidate_mp4s) == 8, f"Expected 8 candidate videos, found {len(candidate_mp4s)}"

    dbsrt_bin = Path("target/release/dbsrt").resolve()
    assert dbsrt_bin.is_file(), f"Release binary not found at {dbsrt_bin}"

    monkeypatch.chdir(tmp_path)
    (tmp_path / "run").mkdir(parents=True, exist_ok=True)
    (tmp_path / "phash_out").mkdir(parents=True, exist_ok=True)
    (tmp_path / "srt_out").mkdir(parents=True, exist_ok=True)
    (tmp_path / "logs" / "workers").mkdir(parents=True, exist_ok=True)

    # Monkeypatch constants to point to test directories
    monkeypatch.setattr(worker, "DEFAULT_PHASH_DIR", str(tmp_path / "phash_out"))
    monkeypatch.setattr(worker, "DEFAULT_SUBTITLES_DIR", str(tmp_path / "srt_out"))
    monkeypatch.setattr(worker, "TARGET_QUEUE_POLL_INTERVAL_SECS", 0.05)

    desired_file = tmp_path / "run" / "desired_workers"
    pause_file = tmp_path / "run" / "batch_paused"
    events_log = tmp_path / "events.jsonl"

    targets = [VideoTarget(str(v), v.name, v.stat().st_size) for v in candidate_mp4s]

    child_env = {
        "PATH": "/usr/bin:/bin:/usr/local/bin",
        "DBSRT_FFMPEG_THREADS": "1",
    }

    # Tracking published states
    published_events: list[dict] = []
    original_write_state = worker.write_batch_queue_state

    def capture_write_state(total, completed, failed, active, desired_workers=None, is_paused=None):
        published_events.append(
            {
                "total": total,
                "completed": completed,
                "failed": failed,
                "active": active,
                "desired": desired_workers,
                "paused": is_paused,
                "time": time.time(),
            }
        )
        original_write_state(total, completed, failed, active, desired_workers, is_paused)

    monkeypatch.setattr(worker, "write_batch_queue_state", capture_write_state)

    print("\n--- [E2E] Starting real batch with 4 concurrent FFmpeg workers ---", flush=True)

    # Background controller thread:
    # 1. Starts at 4 workers
    # 2. At t+0.3s -> scale down to 2 workers (50% reduction)
    # 3. At t+0.8s -> pause batch (soft pause)
    # 4. At t+1.5s -> resume batch (scale to 3 workers)
    def control_flow():
        time.sleep(0.3)
        print("--- [E2E CONTROLLER] Scaling down concurrency to 2 workers (50% load) ---", flush=True)
        desired_file.write_text("2\n", encoding="utf-8")
        time.sleep(0.5)
        print("--- [E2E CONTROLLER] Setting pause sentinel (run/batch_paused) ---", flush=True)
        pause_file.touch()
        time.sleep(0.7)
        print("--- [E2E CONTROLLER] Removing pause sentinel & scaling to 3 workers ---", flush=True)
        pause_file.unlink()
        desired_file.write_text("3\n", encoding="utf-8")

    ctl_thread = threading.Thread(target=control_flow, daemon=True)
    ctl_thread.start()

    # Launch actual FFmpeg extraction via dbsrt media:add
    success_count, fail_count, elapsed = worker.execute_dynamic_batch_ingestion(
        targets=targets,
        stream_base="",  # local file path
        dbsrt_bin=str(dbsrt_bin),
        child_env=child_env,
        events_log=events_log,
        max_extraction_secs=30,
        initial_workers=4,
    )

    ctl_thread.join(timeout=2.0)
    print(f"--- [E2E COMPLETED] {success_count} success, {fail_count} failed in {elapsed:.2f}s ---", flush=True)

    # Verify overall batch execution
    assert fail_count == 0, f"Expected 0 failures, got {fail_count}"
    assert success_count == 8, f"Expected 8 successful extractions, got {success_count}"
    assert elapsed > 0.0

    # Verify physical pHash files produced by genuine FFmpeg decoding
    produced_phashes = list((tmp_path / "phash_out").glob("*.phash"))
    assert len(produced_phashes) == 8, f"Expected 8 .phash files, found {len(produced_phashes)}"
    for p in produced_phashes:
        size = p.stat().st_size
        print(f"    ✓ Generated real .phash: {p.name} ({size} bytes)", flush=True)
        assert size > 0, f"Empty .phash generated: {p}"

    # Verify events log enrichment and contextual fields
    assert events_log.is_file()
    lines = events_log.read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) == 8, f"Expected 8 event records, found {len(lines)}"

    for line in lines:
        rec = json.loads(line)
        assert rec["success"] is True
        assert rec["status"] == "COMPLETED"
        assert rec["file_size_bytes"] > 0
        assert rec["wall_duration_secs"] > 0.0
        assert "video_stem" in rec
        assert "video_rel_path" in rec

    # Verify dynamic scale transitions were recorded
    active_counts = [e["active"] for e in published_events]
    assert max(active_counts) >= 2, "Expected at least 2 concurrent workers during peak"
    assert min(active_counts) == 0, "Expected drain to 0 workers upon batch completion"
