"""Tests for deploy/o2switch/dbsrt_server_ctl.sh against fake daemons in a temporary base dir.

Fake daemons mimic the three o2switch starters (PHP gateway, cron watchdog, start): same
absolute command line, no PID file.
"""

from __future__ import annotations

import fcntl
import os
import subprocess
import time
from collections.abc import Iterator
from pathlib import Path

import pytest

CTL = Path(__file__).resolve().parents[2] / "deploy" / "o2switch" / "dbsrt_server_ctl.sh"

FAKE_SERVER = """#!/usr/bin/env python3
import os, signal, time
if os.environ.get("FAKE_IGNORE_TERM"):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
time.sleep(3600)
"""

# Fake packer: writes the -o target like `dbsrt pack`
FAKE_CLI = """#!/usr/bin/env python3
import sys
open(sys.argv[sys.argv.index("-o") + 1], "w").close()
"""


class Remote:
    def __init__(self, base: Path) -> None:
        self.base = base
        self.socket = base / "run" / "dbsrt-server.sock"
        self.marker = base / "run" / "dbsrt-server.stopped"
        self.env = {
            **os.environ,
            "HOME": str(base.parent),
            "DBSRT_SHUTDOWN_TIMEOUT_SEC": "2",
            "DBSRT_LOCK_WAIT_SEC": "1",
        }

    def ctl(self, action: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(CTL), action, str(self.base)], env=self.env, capture_output=True, text=True, timeout=30
        )

    def pids(self) -> list[int]:
        out = subprocess.run(
            ["pgrep", "-f", "--", f"bin/dbsrt-server --unix-socket {self.socket}( |$)"],
            capture_output=True,
            text=True,
        )
        return sorted(int(p) for p in out.stdout.split())

    def spawn_foreign(self, ignore_term: bool = False, hold_lock: bool = False, argv: list[str] | None = None) -> int:
        """Starts a daemon the way the PHP gateway or cron do (no PID file).

        hold_lock: the daemon inherits a locked spawn-lock fd, as from a gateway request killed
        before its unlock.
        """
        env = {**self.env, **({"FAKE_IGNORE_TERM": "1"} if ignore_term else {})}
        lock_fd = os.open(self.base / "run" / "gateway_spawn.lock", os.O_WRONLY | os.O_CREAT)
        if hold_lock:
            fcntl.flock(lock_fd, fcntl.LOCK_EX)
        try:
            proc = subprocess.Popen(
                argv or [str(self.base / "bin" / "dbsrt-server"), "--unix-socket", str(self.socket)],
                env=env,
                start_new_session=True,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                pass_fds=(lock_fd,) if hold_lock else (),
            )
        finally:
            os.close(lock_fd)
        time.sleep(0.3)  # let python install its SIGTERM handler
        return proc.pid


@pytest.fixture
def remote(tmp_path: Path) -> Iterator[Remote]:
    base = tmp_path / "dbsrt"
    for d in ("bin", "run", "logs", "data/__PHASH__", "etc"):
        (base / d).mkdir(parents=True)
    for name, body in (("dbsrt-server", FAKE_SERVER), ("dbsrt", FAKE_CLI)):
        (base / "bin" / name).write_text(body)
        (base / "bin" / name).chmod(0o755)
    r = Remote(base)
    yield r
    for pid in r.pids():
        os.kill(pid, 9)


@pytest.mark.parametrize(
    ("foreign", "ignore_term"),
    [(1, False), (2, False), (1, True)],
    ids=["one", "two-duplicates", "ignores-sigterm"],
)
def test_stop_kills_every_daemon_and_blocks_respawn(remote: Remote, foreign: int, ignore_term: bool) -> None:
    for _ in range(foreign):
        remote.spawn_foreign(ignore_term=ignore_term)
    assert len(remote.pids()) == foreign

    out = remote.ctl("stop")

    assert out.returncode == 0, out.stdout + out.stderr
    assert remote.pids() == []
    assert remote.marker.exists()


def test_stop_kills_a_daemon_holding_the_spawn_lock(remote: Remote) -> None:
    remote.spawn_foreign(hold_lock=True)

    out = remote.ctl("stop")

    assert out.returncode == 0, out.stdout + out.stderr
    assert remote.pids() == []
    assert remote.marker.exists()


@pytest.mark.parametrize("action", ["start", "stop", "watchdog"])
def test_lock_held_by_a_non_daemon_fails_instead_of_hanging(remote: Remote, action: str) -> None:
    holder = remote.spawn_foreign(hold_lock=True, argv=["sleep", "60"])
    try:
        out = remote.ctl(action)
    finally:
        os.kill(holder, 9)

    assert out.returncode == 1, out.stdout + out.stderr
    assert "spawn lock" in out.stdout + out.stderr
    assert remote.pids() == []


def test_stop_when_nothing_runs_still_sets_marker(remote: Remote) -> None:
    out = remote.ctl("stop")
    assert out.returncode == 0, out.stdout + out.stderr
    assert remote.marker.exists()


@pytest.mark.parametrize(
    ("stopped", "running", "expected"),
    [(False, 0, 1), (False, 1, 1), (True, 0, 0)],
    ids=["respawns-missing", "keeps-single", "honors-stop"],
)
def test_watchdog(remote: Remote, stopped: bool, running: int, expected: int) -> None:
    if stopped:
        remote.marker.touch()
    before = [remote.spawn_foreign() for _ in range(running)]

    out = remote.ctl("watchdog")
    time.sleep(0.5)

    assert out.returncode == 0, out.stdout + out.stderr
    pids = remote.pids()
    assert len(pids) == expected
    if running:
        assert pids == sorted(before)


def test_start_clears_stop_marker_and_is_idempotent(remote: Remote) -> None:
    remote.marker.touch()

    first = remote.ctl("start")
    pids = remote.pids()
    second = remote.ctl("start")

    assert first.returncode == 0, first.stdout + first.stderr
    assert second.returncode == 0, second.stdout + second.stderr
    assert len(pids) == 1
    assert remote.pids() == pids
    assert not remote.marker.exists()


@pytest.mark.parametrize(
    ("stopped", "running", "code", "needle"),
    [
        (False, 0, 3, "not running"),
        (True, 0, 3, "stopped by operator"),
        (False, 1, 0, "running"),
        (False, 2, 1, "2 daemons"),
    ],
    ids=["down", "stopped", "up", "duplicates"],
)
def test_status(remote: Remote, stopped: bool, running: int, code: int, needle: str) -> None:
    if stopped:
        remote.marker.touch()
    for _ in range(running):
        remote.spawn_foreign()

    out = remote.ctl("status")

    assert out.returncode == code, out.stdout + out.stderr
    assert needle in out.stdout


def test_pids_lists_only_this_socket_daemons(remote: Remote) -> None:
    pid = remote.spawn_foreign()
    other = Remote(remote.base)
    other.socket = remote.base / "run" / "other.sock"
    other_pid = other.spawn_foreign()
    try:
        out = remote.ctl("pids")
    finally:
        os.kill(other_pid, 9)

    assert out.stdout.split() == [str(pid)]


def test_unknown_action_fails(remote: Remote) -> None:
    assert remote.ctl("bogus").returncode == 2
