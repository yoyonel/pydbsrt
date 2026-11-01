#!/usr/bin/env python3
"""Drop a legacy-named .phash once its moviehash-named twin sits in the same directory.

rsync --ignore-existing pulls <moviehash>.<fps>fps.phash next to the legacy copy it replaces on o2switch.
Only a byte-identical legacy twin is removed; dry run unless --apply. Prints counts only.
"""

from __future__ import annotations

import argparse
import filecmp
import sys
from pathlib import Path
from typing import Dict

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from scripts.phash_names import phash_logical_name  # noqa: E402


def dedupe_legacy_twins(phash_dir: Path, apply: bool) -> Dict[str, int]:
    counts = {"moviehash": 0, "legacy_twin": 0, "identical": 0, "differs": 0, "removed": 0}
    for phash in sorted(phash_dir.glob("*.phash")):
        logical = phash_logical_name(phash)
        if logical == phash.name:
            continue
        counts["moviehash"] += 1
        twin = phash.with_name(logical)
        if not twin.is_file():
            continue
        counts["legacy_twin"] += 1
        if not filecmp.cmp(phash, twin, shallow=False):
            counts["differs"] += 1
            continue
        counts["identical"] += 1
        if apply:
            twin.unlink()
            counts["removed"] += 1
    return counts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phash_dir", type=Path)
    parser.add_argument("--apply", action="store_true", help="remove the identical legacy twins")
    args = parser.parse_args()
    if not args.phash_dir.is_dir():
        print("phash_dir_missing=1")
        return 1
    print(f"mode={'apply' if args.apply else 'dry-run'}")
    for key, value in dedupe_legacy_twins(args.phash_dir, args.apply).items():
        print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
