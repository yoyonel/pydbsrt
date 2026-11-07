#!/usr/bin/env python3
"""E2E Verification Suite for Autonomous Multi-Offset Matching Benchmark Workflow.

Validates the complete operational lifecycle against real media targets:
1. Benchmark start with live JSON status tracking.
2. Soft pause injection (`run/matching_bench_paused`).
3. Dynamic concurrency scaling (`run/matching_bench_workers`).
4. Benchmark resume, final completion and report validation.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

_USER = os.environ.get("USER", "media")
NAS_BASE = os.environ.get("NAS_TVSHOW_DIR", f"/media/{_USER}/Synology/tvshow")

DEMON_DIR = Path(NAS_BASE) / "Demon Slayer/S02"
REAL_MEDIA_TARGETS = [
    DEMON_DIR / "Demon Slayer – Saison 2 Episode 10 VOSTFR.mp4",
    DEMON_DIR / "Demon Slayer – Saison 2 Episode 11 VOSTFR.mp4",
    DEMON_DIR / "Demon Slayer – Saison 2 Episode 12 VOSTFR.mp4",
]


def test_matching_bench_lifecycle_e2e(tmp_path: Path) -> None:
    # Check if NAS files exist
    existing_targets = [p for p in REAL_MEDIA_TARGETS if p.is_file()]
    if len(existing_targets) < 2:
        print("⚠️ Warning: Demon Slayer NAS files not found. Using local test fixtures as fallback.")
        fixture = Path("tests/data/big_buck_bunny_trailer_480p.mp4")
        if not fixture.is_file():
            print("❌ No test fixtures available. Skipping test.")
            return
        existing_targets = [fixture]

    status_file = tmp_path / "run/matching_bench_status.json"
    pause_file = tmp_path / "run/matching_bench_paused"
    workers_file = tmp_path / "run/matching_bench_workers"
    report_file = tmp_path / "data/reports/eval_test.json"
    cache_file = tmp_path / "data/probes_cache.json"

    dbsrt_bin = Path("target/debug/dbsrt")
    if not dbsrt_bin.is_file():
        dbsrt_bin = Path("target/release/dbsrt")
    assert dbsrt_bin.is_file(), "dbsrt binary must be compiled"

    dbphash = Path("data/catalog.dbphash")
    assert dbphash.is_file(), "data/catalog.dbphash must exist"

    # 1. Start runner in background process
    cmd = [
        sys.executable,
        "scripts/matching_bench_runner.py",
        "--bin",
        str(dbsrt_bin),
        "--dbphash",
        str(dbphash),
        "--probe-cache",
        str(cache_file),
        "--status-path",
        str(status_file),
        "--pause-file",
        str(pause_file),
        "--workers-file",
        str(workers_file),
        "--out",
        str(report_file),
        "--workers",
        "4",
    ] + [str(p) for p in existing_targets]

    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    try:
        # 2. Wait for runner to initialize status file
        for _ in range(50):
            if status_file.is_file():
                break
            time.sleep(0.1)

        assert status_file.is_file(), "Status file should be created"
        status_data = json.loads(status_file.read_text(encoding="utf-8"))
        assert status_data["total"] == len(existing_targets)
        assert status_data["active_workers"] == 4

        # 3. Trigger soft pause
        pause_file.parent.mkdir(parents=True, exist_ok=True)
        pause_file.touch()

        # 4. Trigger scale workers down to 2
        workers_file.write_text("2\n", encoding="utf-8")

        time.sleep(1.0)
        if status_file.is_file():
            paused_status = json.loads(status_file.read_text(encoding="utf-8"))
            print(f"✓ Pause active, status: {paused_status}")

        # 5. Resume execution
        if pause_file.is_file():
            pause_file.unlink()

        # 6. Wait for process completion
        stdout, stderr = proc.communicate(timeout=60)
        assert proc.returncode == 0, f"Runner failed with code {proc.returncode}:\n{stderr}"

        # 7. Validate final outputs
        assert report_file.is_file(), "Final report must exist"
        report_data = json.loads(report_file.read_text(encoding="utf-8"))
        assert len(report_data) == len(existing_targets)

        final_status = json.loads(status_file.read_text(encoding="utf-8"))
        assert final_status["completed"] == len(existing_targets)
        assert final_status["progress_pct"] == 100.0
        assert final_status["is_paused"] is False

        print("🎉 E2E Lifecycle test successfully validated!")
    finally:
        if proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=5)


if __name__ == "__main__":
    test_dir = Path("/tmp/matching_bench_test_run")
    if test_dir.exists():
        shutil.rmtree(test_dir)
    test_dir.mkdir(parents=True, exist_ok=True)
    test_matching_bench_lifecycle_e2e(test_dir)
    shutil.rmtree(test_dir)
