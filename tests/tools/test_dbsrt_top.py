"""Hermetic unit tests for scripts/dbsrt_top.py TUI monitor and historical statistics."""

import json
from pathlib import Path
from typing import Any, Dict, List

from rich.console import Console

from scripts.dbsrt_top import (
    ActiveJobTelemetry,
    TerminalInput,
    calculate_stats,
    format_duration,
    format_progress_bar,
    load_historical_records,
    render_dashboard,
    scan_active_jobs,
    stop_job,
    toggle_pause_job,
)


def test_format_duration_seconds() -> None:
    """Durations under 60 seconds should format as Xs."""
    assert format_duration(0.0) == "0s"
    assert format_duration(45.2) == "45s"
    assert format_duration(59.9) == "59s"


def test_format_duration_minutes() -> None:
    """Durations between 60s and 3600s should format as XmYYs."""
    assert format_duration(60.0) == "1m00s"
    assert format_duration(125.0) == "2m05s"
    assert format_duration(3599.0) == "59m59s"


def test_format_duration_hours() -> None:
    """Durations >= 3600s should format as XhYYm."""
    assert format_duration(3600.0) == "1h00m"
    assert format_duration(3665.0) == "1h01m"
    assert format_duration(7320.0) == "2h02m"


def test_format_duration_bounds() -> None:
    """Negative durations or durations exceeding one week should return N/A."""
    assert format_duration(-1.0) == "N/A"
    assert format_duration(86400 * 8) == "N/A"


def test_format_progress_bar() -> None:
    """Progress bar should properly render percentages and clamped bounds."""
    bar_0 = format_progress_bar(0.0, width=10)
    assert "[░░░░░░░░░░]   0.0%" in bar_0

    bar_50 = format_progress_bar(0.5, width=10)
    assert "[█████░░░░░]  50.0%" in bar_50

    bar_100 = format_progress_bar(1.0, width=10)
    assert "[██████████] 100.0%" in bar_100

    # Test clamping
    bar_clamped_neg = format_progress_bar(-0.5, width=10)
    assert "0.0%" in bar_clamped_neg
    bar_clamped_overflow = format_progress_bar(1.5, width=10)
    assert "100.0%" in bar_clamped_overflow


def test_calculate_stats_empty() -> None:
    """Empty datasets must safely return 0.0 for all statistics."""
    stats = calculate_stats([])
    assert stats["min"] == 0.0
    assert stats["max"] == 0.0
    assert stats["mean"] == 0.0
    assert stats["median"] == 0.0
    assert stats["p90"] == 0.0
    assert stats["stdev"] == 0.0


def test_calculate_stats_single_value() -> None:
    """Single value datasets must return that value with zero stdev."""
    stats = calculate_stats([42.0])
    assert stats["min"] == 42.0
    assert stats["max"] == 42.0
    assert stats["mean"] == 42.0
    assert stats["median"] == 42.0
    assert stats["p90"] == 42.0
    assert stats["stdev"] == 0.0


def test_calculate_stats_multiple_values() -> None:
    """Verify standard distribution calculations across multiple values."""
    values = [10.0, 20.0, 30.0, 40.0, 50.0]
    stats = calculate_stats(values)
    assert stats["min"] == 10.0
    assert stats["max"] == 50.0
    assert stats["mean"] == 30.0
    assert stats["median"] == 30.0
    assert stats["p90"] == 50.0  # index min(int(5 * 0.90), 4) = min(4, 4) = 4 -> values[4] = 50.0
    assert stats["stdev"] > 0.0


def test_load_historical_records_ndjson(tmp_path: Path) -> None:
    """Verify parsing, corrupted line tolerance, stem deduplication, and chronological sorting."""
    events_file = tmp_path / "batch_events.jsonl"
    lines = [
        # Earlier dry run (0.01s)
        json.dumps(
            {
                "video_stem": "Show.S01E01",
                "wall_duration_secs": 0.01,
                "cifs_read_rate_mib_s": 5000.0,
                "file_size_bytes": 1000000,
                "success": True,
                "end_iso": "2026-10-26T10:00:00",
            }
        ),
        # Corrupted line (must be skipped gracefully)
        "{ this is invalid json line }",
        # Full decode for the same stem (longer duration, must overwrite earlier dry run)
        json.dumps(
            {
                "video_stem": "Show.S01E01",
                "wall_duration_secs": 320.5,
                "cifs_read_rate_mib_s": 3.5,
                "file_size_bytes": 1000000,
                "success": True,
                "end_iso": "2026-10-26T10:20:00",
            }
        ),
        # Second show completed later
        json.dumps(
            {
                "video_stem": "Show.S01E02",
                "wall_duration_secs": 180.0,
                "cifs_read_rate_mib_s": 4.2,
                "file_size_bytes": 2000000,
                "success": True,
                "end_iso": "2026-10-26T10:30:00",
            }
        ),
    ]
    events_file.write_text("\n".join(lines), encoding="utf-8")

    records = load_historical_records(events_file)
    assert len(records) == 2

    # Verify deduplication kept the longer run for S01E01
    s01e01 = next(r for r in records if r["video_stem"] == "Show.S01E01")
    assert s01e01["wall_duration_secs"] == 320.5

    # Verify chronological ordering: S01E01 (ended 10:20) before S01E02 (ended 10:30)
    assert records[0]["video_stem"] == "Show.S01E01"
    assert records[1]["video_stem"] == "Show.S01E02"


def test_load_historical_records_couplings_fallback(tmp_path: Path, monkeypatch: Any) -> None:
    """Verify fallback to couplings.json when events file does not exist."""
    fake_couplings = tmp_path / "couplings.json"
    fake_couplings.write_text(
        json.dumps(
            {
                "couplings": [
                    {
                        "media_name": "FallbackShow",
                        "subtitles": ["sub1.srt", "sub2.srt"],
                        "fps": 25.0,
                        "frame_count": 2500,
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(
        "scripts.dbsrt_top.Path", lambda p: fake_couplings if p == "data/SRT/couplings.json" else Path(p)
    )

    non_existent = tmp_path / "missing_events.jsonl"
    records = load_historical_records(non_existent)
    assert len(records) == 1
    assert records[0]["video_stem"] == "FallbackShow"
    assert records[0]["wall_duration_secs"] == 100.0
    assert records[0]["certified_couplings_count"] == 2


def test_render_dashboard_empty() -> None:
    """Rendering dashboard with zero active jobs and zero historical records must succeed."""
    panel = render_dashboard(active_jobs=[], history_records=[])
    console = Console(record=True, width=120)
    console.print(panel)
    output = console.export_text()
    assert "pydbsrt top" in output
    assert "Aucun traitement dbsrt" in output


def test_render_dashboard_with_cache_and_real_entries() -> None:
    """Dashboard must correctly distinguish CACHE hits from full decodes in status and rate columns."""
    active_jobs = [
        ActiveJobTelemetry(
            pid=12345,
            media_name="ActiveMovie.2024",
            engine="GPU (VA-API)",
            elapsed_secs=120.0,
            read_bytes=100 * 1024 * 1024,
            total_bytes=2 * 1024 * 1024 * 1024,
            progress_ratio=0.033,
            current_rate_mib_s=5.2,
            eta_secs=3500.0,
            cpu_percent=14.5,
            ram_rss_mib=180.0,
        )
    ]
    history_records: List[Dict[str, Any]] = [
        # Cache hit entry
        {
            "video_stem": "FastCachedShow",
            "file_size_bytes": 500 * 1024 * 1024,
            "duration_secs": 1800.0,
            "fps": 24.0,
            "wall_duration_secs": 0.01,
            "cifs_read_rate_mib_s": 50000.0,
            "success": True,
            "certified_couplings_count": 2,
            "end_iso": "2026-10-26T10:00:00",
        },
        # Full decode entry
        {
            "video_stem": "FullDecodedShow",
            "file_size_bytes": 1500 * 1024 * 1024,
            "duration_secs": 3000.0,
            "fps": 25.0,
            "wall_duration_secs": 480.0,
            "cifs_read_rate_mib_s": 3.1,
            "success": True,
            "certified_couplings_count": 3,
            "end_iso": "2026-10-26T10:20:00",
        },
    ]

    # Render with expanded tables
    panel = render_dashboard(
        active_jobs=active_jobs,
        history_records=history_records,
        show_stats=True,
        show_recent=True,
    )
    console = Console(record=True, width=140)
    console.print(panel)
    output = console.export_text()

    # Active jobs check
    assert "ActiveMovie.2024" in output
    assert "GPU (VA-API)" in output

    # Recent completed check
    assert "FastCachedShow" in output
    assert "CACHE" in output
    assert "FullDecodedShow" in output
    assert "3.1 MB/s" in output


def test_render_dashboard_collapsed_by_default() -> None:
    """Dashboard must collapse stats and recents by default to maximize active jobs area."""
    panel = render_dashboard(active_jobs=[], history_records=[])
    console = Console(record=True, width=120)
    console.print(panel)
    output = console.export_text()
    assert "Appuyer sur 'h' pour déplier" in output
    assert "Appuyer sur 'r' pour déplier" in output


def test_render_dashboard_action_menu() -> None:
    """Dashboard must render cTop-style action menu popup when is_menu_open is True."""
    job = ActiveJobTelemetry(
        pid=5555,
        media_name="ActionTargetJob",
        engine="GPU (VA-API)",
        elapsed_secs=30.0,
        read_bytes=10 * 1024 * 1024,
        total_bytes=100 * 1024 * 1024,
        progress_ratio=0.1,
        current_rate_mib_s=2.5,
        eta_secs=60.0,
        cpu_percent=10.0,
        ram_rss_mib=120.0,
    )
    panel = render_dashboard(
        active_jobs=[job],
        history_records=[],
        selected_index=0,
        is_menu_open=True,
    )
    console = Console(record=True, width=120)
    console.print(panel)
    output = console.export_text()

    assert "Menu d'Actions (cTop)" in output
    assert "PID 5555" in output
    assert "[p]" in output
    assert "SIGSTOP" in output
    assert "[s]" in output
    assert "SIGTERM" in output
    assert "[l]" in output


def test_render_dashboard_log_modal() -> None:
    """Dashboard must render detailed inspection modal when is_log_modal_open is True."""
    job = ActiveJobTelemetry(
        pid=7777,
        media_name="InspectTargetJob",
        engine="CPU (Software)",
        elapsed_secs=45.0,
        read_bytes=20 * 1024 * 1024,
        total_bytes=200 * 1024 * 1024,
        progress_ratio=0.1,
        current_rate_mib_s=1.8,
        eta_secs=120.0,
        cpu_percent=40.0,
        ram_rss_mib=90.0,
        cmdline=["dbsrt", "media:add", "test.mkv"],
        open_files=["/tmp/test.mkv"],
    )
    panel = render_dashboard(
        active_jobs=[job],
        history_records=[],
        selected_index=0,
        is_log_modal_open=True,
    )
    console = Console(record=True, width=120)
    console.print(panel)
    output = console.export_text()

    assert "Inspection Processus & Logs" in output
    assert "PID: 7777" in output
    assert "/tmp/test.mkv" in output
    assert "fermer l'inspection" in output


def test_toggle_pause_and_stop_job_error_handling() -> None:
    """Process signals must gracefully handle terminated/non-existent PIDs."""
    fake_pid = 99999999
    ok_pause, msg_pause = toggle_pause_job(fake_pid)
    assert not ok_pause
    assert "introuvable" in msg_pause or "terminé" in msg_pause

    ok_stop, msg_stop = stop_job(fake_pid)
    assert not ok_stop
    assert "introuvable" in msg_stop or "terminé" in msg_stop


def test_terminal_input_non_tty() -> None:
    """TerminalInput must safely return None when executed outside an interactive TTY."""
    with TerminalInput() as term_in:
        key = term_in.poll_key(timeout=0.01)
        assert key is None


def test_terminal_input_tmux_escape_sequence(monkeypatch: Any) -> None:
    """Verify TMUX SS3 arrow escape sequences (\x1bOA) are parsed as UP."""
    term_in = TerminalInput()
    term_in.fd = 0
    # Simulate TMUX sending ESC, then O, then A
    chars = iter(["\x1b", "O", "A"])
    monkeypatch.setattr("sys.stdin.isatty", lambda: True)
    monkeypatch.setattr("select.select", lambda r, w, x, t: ([0], [], []))
    monkeypatch.setattr("sys.stdin.read", lambda n: next(chars))

    key = term_in.poll_key(timeout=0.01)
    assert key == "UP"


def test_render_dashboard_active_job_none_eta() -> None:
    """Active jobs with None ETA must render cleanly with dash placeholder."""
    active_jobs = [
        ActiveJobTelemetry(
            pid=9999,
            media_name="IndeterminateJob",
            engine="CPU (ffmpeg)",
            elapsed_secs=5.0,
            read_bytes=1024,
            total_bytes=1024 * 1024,
            progress_ratio=0.001,
            current_rate_mib_s=0.2,
            eta_secs=None,
            cpu_percent=50.0,
            ram_rss_mib=64.0,
        )
    ]
    panel = render_dashboard(active_jobs=active_jobs, history_records=[])
    console = Console(record=True, width=140)
    console.print(panel)
    output = console.export_text()
    assert "IndeterminateJob" in output
    assert "CPU" in output
    assert "incalcul" in output


def test_scan_active_jobs_detects_subtitle_extraction(monkeypatch: Any) -> None:
    """Active jobs scanner must detect and classify ffmpeg subtitle extraction tasks."""

    class MockIOCounters:
        read_chars = 50 * 1024 * 1024
        read_bytes = 10 * 1024 * 1024

    class MockMemoryInfo:
        rss = 45 * 1024 * 1024
        vms = 120 * 1024 * 1024

    class MockProcess:
        def __init__(self, pid: int, ppid: int, name: str, cmdline: List[str]) -> None:
            self.info = {"pid": pid, "ppid": ppid, "name": name}
            self._cmdline = cmdline

        def cmdline(self) -> List[str]:
            return self._cmdline

        def create_time(self) -> float:
            return 1000.0

        def io_counters(self) -> MockIOCounters:
            return MockIOCounters()

        def memory_info(self) -> MockMemoryInfo:
            return MockMemoryInfo()

        def cpu_percent(self, interval: Any = None) -> float:
            return 12.5

        def status(self) -> str:
            return "running"

    mock_procs = [
        MockProcess(
            pid=2070919,
            ppid=2070108,
            name="ffmpeg",
            cmdline=[
                "ffmpeg",
                "-v",
                "error",
                "-y",
                "-i",
                "/media/synology/SERIES/Halt.S03E03.mkv",
                "-map",
                "0:s:0",
                "-f",
                "srt",
                "-",
            ],
        ),
        MockProcess(
            pid=2070108,
            ppid=2070000,
            name="dbsrt",
            cmdline=[
                "/target/release/dbsrt",
                "media:add",
                "/media/synology/SERIES/Halt.S03E03.mkv",
                "--all-embedded",
            ],
        ),
    ]

    monkeypatch.setattr("psutil.process_iter", lambda attrs: mock_procs)
    monkeypatch.setattr("time.time", lambda: 1010.0)

    jobs = scan_active_jobs()
    assert len(jobs) == 1
    job = jobs[0]
    assert job.pid == 2070919
    assert job.media_name == "Halt.S03E03"
    assert job.engine == "Sub Extraction"
    assert job.status == "running"
