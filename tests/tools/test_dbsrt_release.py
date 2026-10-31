"""Tests for deploy/o2switch/dbsrt_release.sh in a temporary base dir."""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import time
from collections.abc import Iterator
from pathlib import Path

import pytest

HELPER = Path(__file__).resolve().parents[2] / "deploy" / "o2switch" / "dbsrt_release.sh"
BINARIES = ("dbsrt-server", "dbsrt")


class Remote:
    def __init__(self, base: Path) -> None:
        self.base = base
        self.env = {**os.environ, "HOME": str(base.parent)}

    def run(self, action: str, arg: str | None = None) -> subprocess.CompletedProcess[str]:
        argv = ["bash", str(HELPER), action, str(self.base), *([arg] if arg is not None else [])]
        return subprocess.run(argv, env=self.env, capture_output=True, text=True, timeout=30)

    def release(self, tag: str, sums: str | None = None) -> str:
        """Uploads a release the way o2switch_ctl.sh deploy does; returns its id."""
        stage = self.base / "stage"
        stage.mkdir()
        for b in BINARIES:
            (stage / b).write_text(f"#!/bin/sh\n# {b} {tag}\n")
            (stage / b).chmod(0o755)
        listing = sums if sums is not None else sums_of(stage)
        (stage / "SHA256SUMS").write_text(listing)
        release_id = hashlib.sha256(listing.encode()).hexdigest()[:16]
        (self.base / "releases").mkdir(exist_ok=True)
        stage.rename(self.base / "releases" / release_id)
        return release_id

    def link(self, name: str) -> str | None:
        path = self.base / name
        return Path(os.readlink(path)).name if path.is_symlink() else None

    def serves(self, binary: str) -> str:
        return (self.base / "bin" / binary).read_text()

    def state(self) -> tuple[str | None, str | None, list[str]]:
        return self.link("current"), self.link("previous"), sorted(os.listdir(self.base / "releases"))


def sums_of(directory: Path) -> str:
    return "".join(f"{hashlib.sha256((directory / b).read_bytes()).hexdigest()}  {b}\n" for b in BINARIES)


@pytest.fixture
def remote(tmp_path: Path) -> Iterator[Remote]:
    base = tmp_path / "dbsrt"
    (base / "bin").mkdir(parents=True)
    yield Remote(base)


def activate(remote: Remote, release_id: str) -> None:
    out = remote.run("activate", release_id)
    assert out.returncode == 0, out.stdout + out.stderr
    time.sleep(0.01)  # distinct mtimes for prune ordering


def test_activate_on_fresh_host_links_bins_through_current(remote: Remote) -> None:
    a = remote.release("A")

    activate(remote, a)

    assert remote.state() == (a, None, [a])
    for b in BINARIES:
        assert os.readlink(remote.base / "bin" / b) == f"../current/{b}"
        assert remote.serves(b).endswith(f"# {b} A\n")


def test_activate_imports_copied_binaries_as_previous_release(remote: Remote) -> None:
    for b in BINARIES:
        (remote.base / "bin" / b).write_text(f"legacy {b}\n")
    legacy = hashlib.sha256(sums_of(remote.base / "bin").encode()).hexdigest()[:16]
    a = remote.release("A")

    activate(remote, a)

    assert remote.state() == (a, legacy, sorted([a, legacy]))
    assert (remote.base / "releases" / legacy / "dbsrt").read_text() == "legacy dbsrt\n"
    assert remote.serves("dbsrt").endswith("# dbsrt A\n")


def test_activate_then_rollback_swaps_current_and_previous(remote: Remote) -> None:
    a = remote.release("A")
    activate(remote, a)
    b = remote.release("B")
    activate(remote, b)
    assert remote.state()[:2] == (b, a)

    first = remote.run("rollback")
    assert first.returncode == 0, first.stdout + first.stderr
    assert remote.state()[:2] == (a, b)
    assert remote.serves("dbsrt-server").endswith("A\n")

    second = remote.run("rollback")
    assert second.returncode == 0, second.stdout + second.stderr
    assert remote.state()[:2] == (b, a)


def test_activate_current_release_again_keeps_previous(remote: Remote) -> None:
    a = remote.release("A")
    activate(remote, a)
    b = remote.release("B")
    activate(remote, b)

    activate(remote, b)

    assert remote.state()[:2] == (b, a)


@pytest.mark.parametrize(
    "bad",
    ["", ".", "..", "../x", "a/b", "-rf", " a", "missing"],
    ids=["empty", "dot", "dotdot", "parent-path", "nested", "leading-dash", "space", "unknown"],
)
def test_activate_refuses_bad_ids_and_changes_nothing(remote: Remote, bad: str) -> None:
    a = remote.release("A")
    activate(remote, a)
    before = remote.state()

    out = remote.run("activate", bad)

    assert out.returncode == 1, out.stdout + out.stderr
    assert remote.state() == before


@pytest.mark.parametrize("damage", ["tampered", "missing-entry", "missing-file"])
def test_activate_refuses_a_corrupt_release(remote: Remote, damage: str) -> None:
    a = remote.release("A")
    activate(remote, a)
    if damage == "missing-entry":
        b = remote.release("B", sums="0" * 64 + "  dbsrt-server\n")
    else:
        b = remote.release("B")
        target = remote.base / "releases" / b / "dbsrt"
        if damage == "tampered":
            target.write_text("partial upload")
        else:
            target.unlink()

    out = remote.run("activate", b)

    assert out.returncode == 1, out.stdout + out.stderr
    assert remote.state()[:2] == (a, None)
    assert remote.serves("dbsrt").endswith("A\n")


def test_rollback_without_previous_fails(remote: Remote) -> None:
    a = remote.release("A")
    activate(remote, a)

    out = remote.run("rollback")

    assert out.returncode == 1
    assert "no previous release" in out.stdout
    assert remote.state()[:2] == (a, None)


def test_rollback_refuses_a_corrupt_previous(remote: Remote) -> None:
    a = remote.release("A")
    activate(remote, a)
    b = remote.release("B")
    activate(remote, b)
    (remote.base / "releases" / a / "dbsrt-server").write_text("corrupt")

    out = remote.run("rollback")

    assert out.returncode == 1
    assert remote.state()[:2] == (b, a)


def test_prune_keeps_current_previous_and_newest(remote: Remote) -> None:
    ids = []
    for tag in "ABCDE":
        ids.append(remote.release(tag))
        activate(remote, ids[-1])
    remote.run("rollback")  # current D, previous E: both kept, plus the newest other (C)

    out = remote.run("prune", "3")

    assert out.returncode == 0, out.stdout + out.stderr
    current, previous, left = remote.state()
    assert (current, previous) == (ids[3], ids[4])
    assert left == sorted([ids[2], ids[3], ids[4]])


@pytest.mark.parametrize("keep", ["", "1", "x", "-3"])
def test_prune_refuses_to_keep_less_than_two(remote: Remote, keep: str) -> None:
    a = remote.release("A")
    activate(remote, a)

    assert remote.run("prune", keep).returncode == 1
    assert remote.state()[2] == [a]


def test_list_marks_current_and_previous_newest_first(remote: Remote) -> None:
    a = remote.release("A")
    activate(remote, a)
    b = remote.release("B")
    activate(remote, b)
    (remote.base / "releases" / b / "BUILD").write_text("abc123-dirty feat/x\n")

    out = remote.run("list")

    assert out.stdout.splitlines() == [f"* {b} abc123-dirty feat/x", f"- {a} "]


def test_running_reports_the_release_a_daemon_executes(remote: Remote) -> None:
    sleep = Path(shutil.which("sleep") or "/bin/sleep").resolve()
    if sleep.name != "sleep":
        pytest.skip("sleep is a multi-call binary")
    a = remote.release("A")
    activate(remote, a)
    shutil.copy(sleep, remote.base / "releases" / a / "dbsrt-server")
    socket = remote.base / "run" / "dbsrt-server.sock"
    # argv[0] carries the daemon command line that dbsrt_server_ctl.sh pids matches
    daemon = subprocess.Popen(
        [f"{remote.base}/bin/dbsrt-server --unix-socket {socket}", "60"],
        executable=remote.base / "bin" / "dbsrt-server",
    )
    try:
        out = remote.run("running", str(socket))
    finally:
        daemon.kill()
        daemon.wait()

    assert out.stdout.split() == [a]


def test_unknown_action_fails(remote: Remote) -> None:
    assert remote.run("bogus").returncode == 2
