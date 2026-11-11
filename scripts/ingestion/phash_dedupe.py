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


def find_repo_root() -> Path:
    """Finds repository root dynamically with appliance fallback."""
    curr = Path(__file__).resolve().parent
    for parent in [curr, *curr.parents]:
        if (parent / ".git").is_dir():
            return parent
    appliance_path = Path.home() / "dbsrt"
    if appliance_path.is_dir():
        return appliance_path
    return Path.cwd()


REPO_ROOT = find_repo_root()
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.ingestion.phash_names import phash_logical_name  # noqa: E402


def dedupe_legacy_twins(phash_dir: Path, apply: bool) -> Dict[str, int]:
    counts = {"moviehash": 0, "legacy_twin": 0, "identical": 0, "differs": 0, "removed": 0}
    for phash in sorted(phash_dir.glob("*.phash")):
        logical = phash_logical_name(phash)
        if logical == phash.name:
            continue
        counts["moviehash"] += 1
        try:
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
        except OSError:
            continue
    return counts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "phash_dir",
        type=Path,
        nargs="?",
        default=REPO_ROOT / "data/__PHASH__",
        help="Directory containing .phash files (default: data/__PHASH__)",
    )
    parser.add_argument("--apply", action="store_true", help="remove the identical legacy twins")
    parser.add_argument("--check", "--dry-run", action="store_true", help="dry-run verification mode")
    args = parser.parse_args()
    if not args.phash_dir.is_dir():
        print("phash_dir_missing=1")
        return 1
    print(f"mode={'apply' if args.apply and not args.check else 'dry-run'}")
    for key, value in dedupe_legacy_twins(args.phash_dir, args.apply and not args.check).items():
        print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
