"""o2switch runs the repo's Python through bin/python3, linked to 3.13 by o2switch:sync, never the system python3 (3.6)."""

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
# A remote interpreter call reads its program from stdin or a flag; local calls run `python3 scripts/...`
BARE_REMOTE_PYTHON = re.compile(r"(?<![/\w])python3 -")


def test_sync_links_bin_python3_to_the_alt_python313() -> None:
    ctl = (ROOT / "scripts/infra/o2switch/o2switch_ctl.sh").read_text()

    assert 'O2SWITCH_PYTHON="/opt/alt/python313/bin/python3"' in ctl
    assert "ln -sfn ${O2SWITCH_PYTHON} ${REMOTE_BASE}/bin/python3" in ctl


@pytest.mark.parametrize(
    "path",
    [
        "scripts/infra/o2switch/o2switch_ctl.sh",
        "scripts/e2e/o2switch_e2e.py",
        "scripts/infra/rpi4/freebox_batch_ctl.sh",
    ],
)
def test_remote_commands_never_call_the_system_python3(path: str) -> None:
    lines = (ROOT / path).read_text().splitlines()

    assert [line.strip() for line in lines if BARE_REMOTE_PYTHON.search(line)] == []


def test_the_audit_runs_on_o2switch_through_bin_python3() -> None:
    ctl = (ROOT / "scripts/infra/o2switch/o2switch_ctl.sh").read_text()

    assert "./bin/python3 -I scripts/o2switch_audit_remote.py" in ctl
