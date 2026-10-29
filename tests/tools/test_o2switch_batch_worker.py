"""Unit tests for o2switch_batch_worker script."""

import json
import os
import signal
import subprocess
import tempfile
import time
import urllib.parse
from pathlib import Path

import pytest

import scripts.o2switch_batch_worker as worker
from scripts.o2switch_batch_worker import (
    ACTIVE_CHILD_PGIDS,
    _kill_active_children,
    abort_batch,
    build_child_env,
    build_stream_url,
    get_process_group_cpu_ticks,
    load_blacklisted_stems,
    load_certified_stems,
    load_existing_phash_stems,
    load_targets_file,
    match_nas_paths,
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


def test_load_blacklisted_stems_needs_consecutive_failures(tmp_path: Path):
    """Only stems whose last 3 outcomes are failures are blacklisted, NFD and NFC stems merge."""
    events = tmp_path / "events.jsonl"
    rows = [("bad", False)] * 3 + [("flaky", False)] * 2 + [("flaky", True), ("flaky", False)]
    rows += [("Cruaute\u0301", False), ("Cruaut\u00e9", False), ("Cruaut\u00e9", False)]
    events.write_text(
        "\n".join(json.dumps({"video_stem": s, "success": ok}) for s, ok in rows) + "\n\n", encoding="utf-8"
    )

    assert load_blacklisted_stems(events) == {"bad", "Cruaut\u00e9"}
    assert load_blacklisted_stems(tmp_path / "missing.jsonl") == set()


def test_match_nas_paths_uses_real_nas_names():
    """NFC or 8.3-mangled target names are replaced by the raw NAS path, unknown ones dropped."""
    nas = [
        ("/tvshow/S01/01 Cruaute\u0301.mp4", "01 Cruaut\u00e9.mp4", 7),
        ("/tvshow/S04/E01 A: B.mkv", "E01 A: B.mkv", 9),
    ]
    targets = [
        ("/tvshow/S01/01 Cruaut\u00e9.mp4", "01 Cruaut\u00e9.mp4", 0),
        ("/tvshow/S01/01 Cruaute\u0301.mp4", "01 Cruaute\u0301.mp4", 0),
        ("/tvshow/S04/SY9QH1~F.MKV", "SY9QH1~F.MKV", 0),
    ]

    assert match_nas_paths(targets, nas) == [nas[0]]


def test_script_updated_detects_redeploy(tmp_path: Path, monkeypatch):
    """A newer mtime on the worker script requests a reload, a vanished file does not."""
    script = tmp_path / "worker.py"
    script.write_text("x", encoding="utf-8")
    monkeypatch.setattr(worker, "SCRIPT_PATH", script)
    monkeypatch.setattr(worker, "SCRIPT_MTIME", script.stat().st_mtime)
    assert not worker.script_updated()
    os.utime(script, (0, worker.SCRIPT_MTIME + 5))
    assert worker.script_updated()
    script.unlink()
    assert not worker.script_updated()


def test_list_nas_videos_ftp_retries_transient_errors(monkeypatch):
    """A dropped FTP connection restarts the walk; persistent failures still raise."""
    calls = []

    def flaky_walk(*_args):
        calls.append(1)
        if len(calls) < 2:
            raise ConnectionResetError("reset")
        return [("/video/a.mkv", "a.mkv", 1)]

    monkeypatch.setattr(worker, "_walk_nas_videos_ftp", flaky_walk)
    monkeypatch.setattr(worker.time, "sleep", lambda _s: None)
    assert worker.list_nas_videos_ftp("nas", "u", "p") == [("/video/a.mkv", "a.mkv", 1)]

    monkeypatch.setattr(worker, "_walk_nas_videos_ftp", lambda *_a: (_ for _ in ()).throw(TimeoutError()))
    with pytest.raises(TimeoutError):
        worker.list_nas_videos_ftp("nas", "u", "p")


def test_excluded_prefixes_match_nfc_and_nfd_paths(tmp_path: Path):
    """Exclusion prefixes skip comments and match whatever normalization the NAS uses."""
    exclude = tmp_path / "batch_exclude.txt"
    exclude.write_text("# unreadable\n\n/tvshow/Cruaut\u00e9\n", encoding="utf-8")
    prefixes = worker.load_excluded_prefixes(exclude)

    assert prefixes == ["/tvshow/Cruaut\u00e9"]
    assert worker.is_excluded("/tvshow/Cruaute\u0301/S01/e1.mkv", prefixes)
    assert not worker.is_excluded("/tvshow/Other/e1.mkv", prefixes)
    assert not worker.is_excluded("/tvshow/x.mkv", [])
    assert worker.load_excluded_prefixes(tmp_path / "missing.txt") == []
