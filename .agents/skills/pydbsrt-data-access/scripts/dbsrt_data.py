#!/usr/bin/env python3
"""Skill entry-point wrapper delegating to scripts.data_access.cli."""

from __future__ import annotations

import sys
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[4]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

from scripts.data_access.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main())
