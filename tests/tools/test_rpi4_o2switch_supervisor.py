"""Hermetic unit tests for scripts/rpi4_o2switch_supervisor.py."""

from __future__ import annotations

from unittest.mock import MagicMock, patch

import pytest

from scripts.rpi4_o2switch_supervisor import (
    AUTOSCALE_MAX_WORKERS,
    AUTOSCALE_MIN_WORKERS,
    RELAUNCH_BACKOFF_SECS,
    check_remote_batch_status,
    compute_lve_cpu_pct,
    decide_next_batch,
    decide_workers,
    get_remote_desired_workers,
    get_remote_running_jobs,
    parse_proc_cpu_seconds,
    pull_remote_catalog,
    restart_remote_batch,
    set_remote_desired_workers,
    supervisor_loop,
)


def _completed(stdout: str = "", returncode: int = 0) -> MagicMock:
    res = MagicMock()
    res.stdout = stdout
    res.stderr = ""
    res.returncode = returncode
    return res


def test_parse_proc_cpu_seconds_handles_comm_with_spaces():
    """utime+stime are read after the last ')' and converted with CLK_TCK."""
    output = "100\n42 (python3) S 1 42 42 0 0 0 0 0 0 0 150 50 0 0\n43 (ff (a) b) R 42 42 42 0 0 0 0 0 0 0 10 0 0 0\n"
    assert parse_proc_cpu_seconds(output) == {42: 2.0, 43: 0.1}


def test_compute_lve_cpu_pct_uses_deltas_and_counts_new_pids():
    """Percentage is the CPU time used during the interval over 12 vCPUs, not a lifetime average."""
    prev = (100.0, {1: 1000.0, 2: 50.0})
    # pid 1 used 60s, pid 2 exited (ignored), pid 3 is new with 36s
    cur = (110.0, {1: 1060.0, 3: 36.0})
    # (60 + 36) / 10s / 12 vCPUs = 80%
    assert compute_lve_cpu_pct(prev, cur) == pytest.approx(80.0)
    assert compute_lve_cpu_pct(cur, cur) == 0.0


@pytest.mark.parametrize(
    ("cpu", "history", "workers", "cooldown_elapsed", "expected"),
    [
        (97.0, [97.0], 4, False, 3),  # emergency spike ignores window and cooldown
        (97.0, [97.0], AUTOSCALE_MIN_WORKERS, False, AUTOSCALE_MIN_WORKERS),  # floor
        (50.0, [50.0, 50.0], 4, True, 4),  # window not full
        (50.0, [50.0, 50.0, 50.0], 4, False, 4),  # cooldown running
        (50.0, [50.0, 50.0, 50.0], 3, True, 4),  # scale up
        (55.0, [55.0, 55.0, 55.0], 4, True, 5),  # 4 videos at ~55 % LVE: room for a fifth
        (67.0, [67.0, 67.0, 67.0], 4, True, 4),  # 4 workers at ~67 %: a fifth would cross 80 % (flapping)
        (50.0, [50.0, 50.0, 50.0], AUTOSCALE_MAX_WORKERS, True, AUTOSCALE_MAX_WORKERS),  # ceiling
        (94.0, [94.0, 94.0, 94.0], 4, True, 3),  # scale down
        (85.0, [85.0, 85.0, 85.0], 4, True, 3),  # above 80 %: CPU fault zone, scale down
        (80.0, [80.0, 80.0, 80.0], 4, True, 4),  # upper bound included in target zone
        (60.0, [60.0, 60.0, 60.0], 4, True, 4),  # lower bound included in target zone
        (75.0, [75.0, 75.0, 75.0], 4, True, 4),  # target zone
    ],
)
def test_decide_workers(cpu, history, workers, cooldown_elapsed, expected):
    assert decide_workers(cpu, history, workers, cooldown_elapsed) == expected


@pytest.mark.parametrize(
    ("cpu", "history", "running", "expected"),
    [
        (99.0, [99.0], 4, 3),  # in-flight jobs exceed target: wait, a lower target changes nothing yet
        (95.0, [95.0, 95.0, 95.0], 4, 3),  # same for the rolling-average step
        (99.0, [99.0], 3, 2),  # target already bites: emergency step allowed
        (95.0, [95.0, 95.0, 95.0], 2, 2),
        (99.0, [99.0], None, 2),  # probe failed: previous behaviour
    ],
)
def test_decide_workers_waits_for_in_flight_jobs_before_scaling_down(cpu, history, running, expected):
    assert decide_workers(cpu, history, 3, True, running) == expected


def test_get_remote_running_jobs():
    """Counts in-flight jobs, None when the probe output is unusable."""
    with patch("subprocess.run", return_value=_completed("3\n")) as mock_run:
        assert get_remote_running_jobs("o2switch") == 3
        assert "[d]bsrt media:add" in mock_run.call_args[0][0][-1]
    with patch("subprocess.run", return_value=_completed("", returncode=255)):
        assert get_remote_running_jobs("o2switch") is None


@pytest.mark.parametrize(
    ("state", "batch_type", "since_launch", "expected"),
    [
        ("crashed", "movies", RELAUNCH_BACKOFF_SECS, "movies"),  # resume same library
        ("absent", "tvshow", RELAUNCH_BACKOFF_SECS, "tvshow"),
        ("finished", "tvshow", RELAUNCH_BACKOFF_SECS, "movies"),  # drained: switch library
        ("finished", "movies", RELAUNCH_BACKOFF_SECS, "freebox"),
        ("finished", "freebox", RELAUNCH_BACKOFF_SECS, "tvshow"),
        ("finished", "tvshow", RELAUNCH_BACKOFF_SECS - 1, None),  # backoff blocks tight loops
        ("crashed", "tvshow", 60, None),
        ("stopped", "tvshow", RELAUNCH_BACKOFF_SECS, None),  # manual stop is final
        ("aborted", "tvshow", RELAUNCH_BACKOFF_SECS, None),  # systemic failure waits for a human
        ("running", "tvshow", RELAUNCH_BACKOFF_SECS, None),
    ],
)
def test_decide_next_batch(state, batch_type, since_launch, expected):
    assert decide_next_batch(state, batch_type, since_launch) == expected


def test_get_remote_desired_workers():
    """Verifies reading desired workers count from remote file, with fallback on garbage."""
    with patch("subprocess.run", return_value=_completed("5\n")):
        assert get_remote_desired_workers("o2switch", "dbsrt", default_workers=4) == 5
    with patch("subprocess.run", return_value=_completed("garbage\n")):
        assert get_remote_desired_workers("o2switch", "dbsrt", default_workers=4) == 4


def test_set_remote_desired_workers_goes_through_validated_ctl():
    """Concurrency changes use o2switch_ctl.sh batch-workers, which validates the bounds."""
    with patch("subprocess.run", return_value=_completed()) as mock_run:
        assert set_remote_desired_workers("o2switch", "dbsrt", count=5) is True
        cmd = mock_run.call_args[0][0]
        assert cmd[1:] == ["batch-workers", "5"]
        assert mock_run.call_args.kwargs["env"]["O2SWITCH_HOST"] == "o2switch"
    with patch("subprocess.run", return_value=_completed(returncode=1)):
        assert set_remote_desired_workers("o2switch", "dbsrt", count=99) is False


@pytest.mark.parametrize(
    ("stdout", "returncode", "expected"),
    [
        (
            "=== Remote Batch Worker Status (o2switch) ===\n"
            "BATCH_STATE=running PID=1849938 TYPE=movies\n"
            "● Batch worker is RUNNING (PID: 1849938)\n",
            0,
            {"state": "running", "pid": 1849938, "type": "movies"},
        ),
        (
            "BATCH_STATE=finished PID=none TYPE=tvshow\n○ Batch worker is not running (finished).\n",
            0,
            {"state": "finished", "pid": None, "type": "tvshow"},
        ),
        ("", 255, {"state": "unknown", "pid": None, "type": "tvshow"}),  # ssh failure
        ("no state line\n", 0, {"state": "unknown", "pid": None, "type": "tvshow"}),
    ],
)
def test_check_remote_batch_status(stdout, returncode, expected):
    with patch("subprocess.run", return_value=_completed(stdout, returncode)):
        assert check_remote_batch_status("o2switch", "dbsrt") == expected


def test_restart_remote_batch_resumes_batch_type():
    """Restart relaunches the recorded batch type as a fresh-only daemon."""
    stdout = "✓ Started persistent background batch worker (screen: dbsrt_batch, PID: 1849938)."
    with patch("subprocess.run", return_value=_completed(stdout)) as mock_run:
        assert restart_remote_batch("o2switch", "dbsrt", 4, "movies") is True
        args = mock_run.call_args[0][0]
        assert args[1:] == ["batch-movies", "--workers", "4", "--fresh-only", "--daemon"]


def test_pull_remote_catalog_success():
    """Verifies catalog pull invocation."""
    with patch("subprocess.run", return_value=_completed()) as mock_run:
        assert pull_remote_catalog("o2switch", "dbsrt") is True
        mock_run.assert_called_once()


def _run_one_iteration(state: str, batch_type: str = "tvshow") -> MagicMock:
    with (
        patch(
            "scripts.rpi4_o2switch_supervisor.check_remote_batch_status",
            return_value={"state": state, "pid": None, "type": batch_type},
        ),
        patch("scripts.rpi4_o2switch_supervisor.get_remote_desired_workers", return_value=4),
        patch("scripts.rpi4_o2switch_supervisor.sample_remote_cpu", return_value=None),
        patch("scripts.rpi4_o2switch_supervisor.get_remote_running_jobs", return_value=None),
        patch("scripts.rpi4_o2switch_supervisor.restart_remote_batch") as mock_restart,
        patch("scripts.rpi4_o2switch_supervisor.pull_remote_catalog") as mock_pull,
        patch("scripts.rpi4_o2switch_supervisor.time.sleep", side_effect=KeyboardInterrupt),
    ):
        with pytest.raises(KeyboardInterrupt):
            supervisor_loop(remote_host="o2switch", remote_base="dbsrt", check_interval=1, pull_interval=0)
        mock_pull.assert_called_once()
        return mock_restart


@pytest.mark.parametrize("state", ["crashed", "absent"])
def test_supervisor_loop_heals_crashed_batch(state):
    _run_one_iteration(state).assert_called_once_with("o2switch", "dbsrt", 4, "tvshow")


def test_supervisor_loop_switches_library_when_drained():
    _run_one_iteration("finished", "tvshow").assert_called_once_with("o2switch", "dbsrt", 4, "movies")


@pytest.mark.parametrize("state", ["running", "stopped", "unknown"])
def test_supervisor_loop_does_not_restart(state):
    """Manual stops and failed probes must never trigger a restart."""
    _run_one_iteration(state).assert_not_called()
