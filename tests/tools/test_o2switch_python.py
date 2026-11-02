"""o2switch runs the repo's Python through bin/python3, linked to 3.13 by o2switch:sync, never the system python3 (3.6)."""

import re
from pathlib import Path

import pytest

from scripts.o2switch_audit import REMOTE_PYTHON

ROOT = Path(__file__).resolve().parents[2]
# A remote interpreter call reads its program from stdin or a flag; local calls run `python3 scripts/...`
BARE_REMOTE_PYTHON = re.compile(r"(?<![/\w])python3 -")


def test_sync_links_bin_python3_to_the_alt_python313() -> None:
    ctl = (ROOT / "scripts/o2switch_ctl.sh").read_text()

    assert 'O2SWITCH_PYTHON="/opt/alt/python313/bin/python3"' in ctl
    assert "ln -sfn ${O2SWITCH_PYTHON} ${REMOTE_BASE}/bin/python3" in ctl
    assert REMOTE_PYTHON == "./bin/python3"


@pytest.mark.parametrize(
    "path",
    ["scripts/o2switch_ctl.sh", "scripts/o2switch_e2e.py", "scripts/o2switch_audit.py", "scripts/freebox_batch_ctl.sh"],
)
def test_remote_commands_never_call_the_system_python3(path: str) -> None:
    lines = (ROOT / path).read_text().splitlines()

    assert [line.strip() for line in lines if BARE_REMOTE_PYTHON.search(line)] == []
