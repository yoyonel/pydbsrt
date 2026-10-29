"""Unit tests for o2switch_batch_worker script."""

import json
import os
import signal
import subprocess
import tempfile
import time
import urllib.parse
from pathlib import Path

from scripts.o2switch_batch_worker import (
    ACTIVE_CHILD_PGIDS,
    _kill_active_children,
    abort_batch,
    build_child_env,
    build_stream_url,
    get_process_group_cpu_ticks,
    load_certified_stems,
    load_existing_phash_stems,
    load_targets_file,
    next_fast_failure_streak,
    resolve_retry_targets,
)


def test_load_existing_phash_stems_comprehensive():
    """Verifies that .phash stems are correctly resolved regardless of fps suffix or urlencoding."""
    with tempfile.TemporaryDirectory() as tmp_dir:
        phash_dir = Path(tmp_dir)

        # Standard clean stem (non-empty valid file)
        (phash_dir / "clean_movie.mkv.24.0fps.phash").write_bytes(b"DATA")
        # Stem with float fps (e.g. 23.976)
        (phash_dir / "episode_float.mkv.23.976fps.phash").write_bytes(b"DATA")
        # Stem with URL-encoded special characters
        encoded_name = urllib.parse.quote("L'illusion du temps (1_4) [ARTE].webm") + ".25.0fps.phash"
        (phash_dir / encoded_name).write_bytes(b"DATA")
        # Legacy stem without fps
        (phash_dir / "legacy_movie.mkv.phash").write_bytes(b"DATA")
        # Corrupted 0-byte file (must be ignored)
        (phash_dir / "corrupted_movie.mkv.24.0fps.phash").touch()

        stems = load_existing_phash_stems(phash_dir)

        assert "clean_movie" in stems
        assert "episode_float" in stems
        assert "L'illusion du temps (1_4) [ARTE]" in stems
        assert "legacy_movie" in stems
        assert "corrupted_movie" not in stems


def test_load_certified_stems_from_couplings():
    """Verifies parsing of certified stems from Zero-DB couplings.json."""
    with tempfile.TemporaryDirectory() as tmp_dir:
        couplings_path = Path(tmp_dir) / "couplings.json"
        couplings_path.write_text(
            '{"couplings": [{"media_name": "Show.S01E01"}, {"media_name": "Show.S01E02"}]}',
            encoding="utf-8",
        )

        stems = load_certified_stems(couplings_path)
        assert "Show.S01E01" in stems
        assert "Show.S01E02" in stems
        assert len(stems) == 2


def test_resolve_retry_targets_keeps_only_last_failures(tmp_path: Path):
    """A video retried successfully, or already done locally, must not be retried again."""
    events = tmp_path / "events.jsonl"
    lines = [
        {"video_stem": "a", "video_rel_path": "/tvshow/a.mkv", "file_size_bytes": 10, "success": False},
        {"video_stem": "a", "video_rel_path": "/tvshow/a.mkv", "file_size_bytes": 10, "success": True},
        {"video_stem": "b", "video_rel_path": "/tvshow/b.mkv", "file_size_bytes": 20, "success": False},
        {"video_stem": "c", "video_rel_path": "/tvshow/c.mkv", "file_size_bytes": 30, "success": False},
    ]
    events.write_text("\n".join(json.dumps(r) for r in lines) + "\nnot-json\n", encoding="utf-8")

    assert resolve_retry_targets(events, skip_stems={"c"}) == [("/tvshow/b.mkv", "b.mkv", 20)]
    assert resolve_retry_targets(tmp_path / "missing.jsonl", set()) == []


def test_load_targets_file_rebases_and_skips(tmp_path: Path):
    """Targets are rebased on the NAS base dir and done stems are dropped."""
    targets = tmp_path / "targets.json"
    targets.write_text(
        json.dumps(
            [
                {"path": "/media/nas/tvshow/S/e1.mkv", "size_bytes": 5},
                "/media/nas/tvshow/S/e2.mkv",
                "/media/nas/tvshow/S/done.mkv",
            ]
        ),
        encoding="utf-8",
    )

    assert load_targets_file(targets, "/tvshow", skip_stems={"done"}) == [
        ("/tvshow/S/e1.mkv", "e1.mkv", 5),
        ("/tvshow/S/e2.mkv", "e2.mkv", 0),
    ]


def test_build_child_env_passes_credentials_out_of_band(tmp_path: Path, monkeypatch):
    """Credentials go to dbsrt via env only; NAS_PASS itself is not leaked to children."""
    monkeypatch.setenv("NAS_PASS", "secret")
    monkeypatch.delenv("DBSRT_FFMPEG_THREADS", raising=False)

    env = build_child_env("svc", "p@ss#%", tmp_path)

    assert env["DBSRT_FTP_USER"] == "svc"
    assert env["DBSRT_FTP_PASSWORD"] == "p@ss#%"
    assert env["DBSRT_FFMPEG_THREADS"] == "2"
    assert env["PATH"].startswith(f"{tmp_path}:")
    assert "NAS_PASS" not in env


def test_get_process_group_cpu_ticks_sums_matching_group(tmp_path: Path):
    """Only processes of the requested group are summed, comm with spaces/parens is handled."""
    stats = {
        "100": "100 (dbsrt) S 1 100 100 0 0 0 0 0 0 0 7 3 0 0",
        "101": "101 (ff mpeg (x)) R 100 100 100 0 0 0 0 0 0 0 20 5 0 0",
        "200": "200 (other) S 1 200 200 0 0 0 0 0 0 0 99 99 0 0",
    }
    for pid, content in stats.items():
        (tmp_path / pid).mkdir()
        (tmp_path / pid / "stat").write_text(content, encoding="utf-8")

    assert get_process_group_cpu_ticks(100, proc_root=tmp_path) == 35
    assert get_process_group_cpu_ticks(999, proc_root=tmp_path) == 0


def test_kill_active_children_kills_whole_group():
    """Shutdown kills tracked child sessions including grandchildren."""
    proc = subprocess.Popen(["sh", "-c", "sleep 30 & wait"], start_new_session=True)
    ACTIVE_CHILD_PGIDS.add(proc.pid)
    try:
        time.sleep(0.2)
        _kill_active_children()
        assert proc.wait(timeout=5) == -signal.SIGTERM
        time.sleep(0.2)
        try:
            os.killpg(proc.pid, 0)
            group_alive = True
        except ProcessLookupError:
            group_alive = False
        assert not group_alive
    finally:
        ACTIVE_CHILD_PGIDS.discard(proc.pid)


def test_build_stream_url_keeps_raw_paths_verbatim():
    """Percent signs, '#' and spaces in NAS names are never URL-decoded."""
    base = "ftp://nas:21"
    assert build_stream_url(base, "/video/100%41 Live #10.mkv") == "ftp://nas:21/video/100%41 Live #10.mkv"
    assert build_stream_url(base, "tvshow/a b.mkv") == "ftp://nas:21/tvshow/a b.mkv"


def test_next_fast_failure_streak_counts_only_fast_failures():
    """Fast failures accumulate; a success or a slow failure resets the streak."""
    assert next_fast_failure_streak(3, {"success": False, "wall_duration_secs": 1.0}) == 4
    assert next_fast_failure_streak(3, {"success": False, "wall_duration_secs": 600.0}) == 0
    assert next_fast_failure_streak(3, {"success": True, "wall_duration_secs": 1.0}) == 0


def test_abort_batch_drains_queue_and_leaves_marker(tmp_path: Path, monkeypatch):
    """Abort empties the shared queue and writes the marker read by batch-status."""
    monkeypatch.chdir(tmp_path)
    queue = [("/video/a.mkv", "a.mkv", 1), ("/video/b.mkv", "b.mkv", 2)]

    abort_batch(queue, "50 consecutive failures")

    assert queue == []
    assert "50 consecutive failures" in (tmp_path / "run" / "batch_aborted").read_text(encoding="utf-8")
