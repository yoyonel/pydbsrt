"""Hermetic unit tests for scripts/rpi4_o2switch_supervisor.py."""

from __future__ import annotations

from unittest.mock import MagicMock, patch

from scripts.rpi4_o2switch_supervisor import (
    DEFAULT_DAY_WORKERS,
    DEFAULT_NIGHT_WORKERS,
    check_remote_batch_status,
    compute_desired_workers,
    pull_remote_catalog,
    restart_remote_batch,
    supervisor_loop,
)


def test_compute_desired_workers():
    """Verifies that workers are scaled up during night and scaled down during day."""
    with patch("scripts.rpi4_o2switch_supervisor.datetime") as mock_dt:
        # 02:00 -> Night (4 workers)
        mock_dt.datetime.now.return_value.hour = 2
        assert compute_desired_workers() == DEFAULT_NIGHT_WORKERS

        # 23:30 -> Night (4 workers)
        mock_dt.datetime.now.return_value.hour = 23
        assert compute_desired_workers() == DEFAULT_NIGHT_WORKERS

        # 14:00 -> Day (2 workers)
        mock_dt.datetime.now.return_value.hour = 14
        assert compute_desired_workers() == DEFAULT_DAY_WORKERS

        # 07:00 -> Day (2 workers)
        mock_dt.datetime.now.return_value.hour = 7
        assert compute_desired_workers() == DEFAULT_DAY_WORKERS


def test_check_remote_batch_status_running():
    """Verifies parsing of active running state with PID."""
    mock_res = MagicMock()
    mock_res.stdout = """=== Remote TVShow Batch Worker Status (o2switch) ===
● Batch worker is RUNNING (PID: 1849938)
    PID USER     %CPU %MEM     ELAPSED COMMAND
1849938 atli6249  3.5  0.1       02:30 python3
"""
    mock_res.returncode = 0

    with patch("subprocess.run", return_value=mock_res) as mock_run:
        status = check_remote_batch_status("o2switch", "dbsrt")
        assert status["running"] is True
        assert status["pid"] == 1849938
        mock_run.assert_called_once()


def test_check_remote_batch_status_dead():
    """Verifies detection of dead / inactive batch worker."""
    mock_res = MagicMock()
    mock_res.stdout = """=== Remote TVShow Batch Worker Status (o2switch) ===
○ Batch worker is NOT running.
"""
    mock_res.returncode = 0

    with patch("subprocess.run", return_value=mock_res):
        status = check_remote_batch_status("o2switch", "dbsrt")
        assert status["running"] is False
        assert status["pid"] is None


def test_restart_remote_batch_success():
    """Verifies correct SSH invocation for restart."""
    mock_res = MagicMock()
    mock_res.stdout = "Started persistent background batch worker in screen 'dbsrt_batch' (PID 1849938)."
    mock_res.returncode = 0

    with patch("subprocess.run", return_value=mock_res) as mock_run:
        success = restart_remote_batch("o2switch", "dbsrt", 4)
        assert success is True
        mock_run.assert_called_once()
        args = mock_run.call_args[0][0]
        assert "o2switch_ctl.sh" in args[0]
        assert "--workers" in args


def test_pull_remote_catalog_success():
    """Verifies catalog pull invocation."""
    mock_res = MagicMock()
    mock_res.returncode = 0
    with patch("subprocess.run", return_value=mock_res) as mock_run:
        assert pull_remote_catalog("o2switch", "dbsrt") is True
        mock_run.assert_called_once()


def test_supervisor_loop_single_iteration():
    """Verifies supervisor loop executes status check, auto-healing, and pull then terminates gracefully."""
    with (
        patch("scripts.rpi4_o2switch_supervisor.check_remote_batch_status") as mock_status,
        patch("scripts.rpi4_o2switch_supervisor.restart_remote_batch") as mock_restart,
        patch("scripts.rpi4_o2switch_supervisor.pull_remote_catalog") as mock_pull,
        patch("scripts.rpi4_o2switch_supervisor.time.sleep", side_effect=KeyboardInterrupt),
    ):
        mock_status.return_value = {"running": False, "pid": None}

        try:
            supervisor_loop(
                remote_host="o2switch",
                remote_base="dbsrt",
                check_interval=1,
                pull_interval=0,  # forces immediate pull
            )
        except KeyboardInterrupt:
            pass

        mock_status.assert_called_once_with("o2switch", "dbsrt")
        mock_restart.assert_called_once()
        mock_pull.assert_called_once()
