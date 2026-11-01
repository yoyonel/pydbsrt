"""Tests for scripts/install_debian.sh: the agent unit relays to a real remote with its key off argv."""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[2] / "scripts"
REMOTE = "https://remote.example.invalid"
KEY = "test-key-0123456789"


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    """Copy of the installer in an isolated repo: the real .env and ~/.config are never read."""
    root = tmp_path / "repo"
    (root / "scripts").mkdir(parents=True)
    for name in ("install_debian.sh", "load_env.sh"):
        shutil.copy(SCRIPTS / name, root / "scripts" / name)
    (root / "target" / "release").mkdir(parents=True)
    (root / "target" / "release" / "dbsrt-server").write_text("#!/bin/sh\n")
    stubs = tmp_path / "stubs"
    stubs.mkdir()
    for tool in ("systemctl", "curl", "journalctl"):
        (stubs / tool).write_text("#!/bin/sh\nexit 0\n")
        (stubs / tool).chmod(0o755)
    # Never reach the real password store from a test
    (stubs / "pass").write_text("#!/bin/sh\nexit 1\n")
    (stubs / "pass").chmod(0o755)
    (tmp_path / "home").mkdir()
    return root


def run(repo: Path, *args: str, **extra: str) -> subprocess.CompletedProcess[str]:
    env = {k: v for k, v in os.environ.items() if k not in ("O2SWITCH_URL", "DBSRT_REMOTE_API_KEY")}
    env |= {"HOME": str(repo.parent / "home"), "PATH": f"{repo.parent / 'stubs'}:{env['PATH']}", **extra}
    return subprocess.run(
        ["bash", "scripts/install_debian.sh", *args], cwd=repo, env=env, capture_output=True, text=True
    )


def unit(repo: Path) -> Path:
    return repo.parent / "home" / ".config" / "systemd" / "user" / "dbsrt-agent.service"


def env_file(repo: Path) -> Path:
    return repo.parent / "home" / ".config" / "pydbsrt" / "dbsrt-agent.env"


def test_install_without_remote_fails_instead_of_relaying_to_itself(repo: Path) -> None:
    out = run(repo, "install")
    assert out.returncode != 0
    assert not unit(repo).exists()


@pytest.mark.parametrize("url", ["http://127.0.0.1:8765", "http://localhost:8765/"])
def test_install_rejects_the_agent_own_address(repo: Path, url: str) -> None:
    out = run(repo, "install", url)
    assert out.returncode != 0
    assert not unit(repo).exists()


def test_install_defaults_to_o2switch_url_from_local_env(repo: Path) -> None:
    (repo / ".env").write_text(f"O2SWITCH_URL={REMOTE}\n")
    out = run(repo, "install")
    assert out.returncode == 0, out.stderr
    assert f"--remote-api {REMOTE}" in unit(repo).read_text()


def test_install_puts_the_key_in_a_private_env_file_not_in_the_unit(repo: Path) -> None:
    out = run(repo, "install", REMOTE, DBSRT_REMOTE_API_KEY=KEY)
    assert out.returncode == 0, out.stderr
    text = unit(repo).read_text()
    assert KEY not in text
    assert f"EnvironmentFile=-{env_file(repo)}" in text
    assert env_file(repo).read_text() == f"DBSRT_REMOTE_API_KEY={KEY}\n"
    assert stat.S_IMODE(env_file(repo).stat().st_mode) == 0o600
    assert KEY not in out.stdout + out.stderr


def test_install_reads_the_key_from_pass(repo: Path) -> None:
    (repo.parent / "stubs" / "pass").write_text(f"#!/bin/sh\necho {KEY}\n")
    assert run(repo, "install", REMOTE).returncode == 0
    assert env_file(repo).read_text() == f"DBSRT_REMOTE_API_KEY={KEY}\n"


def test_install_without_key_drops_a_stale_env_file(repo: Path) -> None:
    assert run(repo, "install", REMOTE, DBSRT_REMOTE_API_KEY=KEY).returncode == 0
    out = run(repo, "install", REMOTE)
    assert out.returncode == 0, out.stderr
    assert not env_file(repo).exists()


def test_uninstall_removes_the_key_file(repo: Path) -> None:
    assert run(repo, "install", REMOTE, DBSRT_REMOTE_API_KEY=KEY).returncode == 0
    assert run(repo, "uninstall").returncode == 0
    assert not env_file(repo).exists()
    assert not unit(repo).exists()
