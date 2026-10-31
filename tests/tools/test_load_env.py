"""Tests for scripts/load_env.sh: the local .env never overrides caller-provided variables."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

LOAD_ENV = Path(__file__).resolve().parents[2] / "scripts" / "load_env.sh"


def test_load_local_env_keeps_caller_values(tmp_path: Path) -> None:
    (tmp_path / ".env").write_text("A=from_file\nB=from_file\n")
    script = f'set -euo pipefail; source "{LOAD_ENV}"; load_local_env "{tmp_path}"; echo "$A $B"'
    env = {**os.environ, "A": "from_caller", "HOME": str(tmp_path)}
    env.pop("B", None)
    out = subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True, check=True)
    assert out.stdout.strip() == "from_caller from_file"
