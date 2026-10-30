"""Tests for RPi4 supervisor telemetry push and log reader."""

import json
from pathlib import Path
from unittest.mock import MagicMock, patch

from scripts.rpi4_o2switch_supervisor import (
    _LoopState,
    push_supervisor_telemetry_to_remote,
    read_recent_supervisor_logs,
)


def test_read_recent_supervisor_logs(tmp_path: Path):
    log_file = tmp_path / "test_supervisor.log"
    log_file.write_text(
        "line 1\nline 2\n\nline 3\nline 4\nline 5\n",
        encoding="utf-8",
    )
    lines = read_recent_supervisor_logs(log_file, max_lines=3)
    assert lines == ["line 3", "line 4", "line 5"]


def test_read_recent_supervisor_logs_missing():
    assert read_recent_supervisor_logs(Path("/nonexistent/file.log")) == []


@patch("subprocess.run")
def test_push_supervisor_telemetry_to_remote(mock_run, tmp_path: Path):
    mock_run.return_value = MagicMock(returncode=0)
    log_file = tmp_path / "test.log"
    log_file.write_text("[2026-10-30 10:00:00] Healthy\n", encoding="utf-8")

    loop = _LoopState(
        last_cpu=62.5,
        last_avg_cpu=70.0,
        last_workers=4,
        last_running=4,
        last_backup_iso="2026-10-30T09:00:00Z",
        last_sync_iso="2026-10-30T09:30:00Z",
    )
    status = {"state": "running", "pid": 12345, "type": "movies"}

    ok = push_supervisor_telemetry_to_remote("mock_host", "dbsrt", loop, status, log_file)
    assert ok is True
    assert mock_run.called

    call_kwargs = mock_run.call_args[1]
    payload = json.loads(call_kwargs["input"])
    assert payload["supervisor_host"] == "rpi4"
    assert payload["state"] == "healthy"
    assert payload["remote_batch"]["lve_cpu_pct"] == 62.5
    assert payload["remote_batch"]["avg_cpu_pct"] == 70.0
    assert payload["last_backup_time"] == "2026-10-30T09:00:00Z"
    assert len(payload["recent_logs"]) == 1
