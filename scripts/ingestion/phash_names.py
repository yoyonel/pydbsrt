"""Logical name of a .phash file, legacy-named or named by moviehash with its sidecar."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


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


def phash_logical_name(phash: Path) -> str:
    """<media_name>.<fps>fps.phash for a <moviehash>.<fps>fps.phash with its <moviehash>.json sidecar, else the file name.

    Same rule as dbsrt_core::phash_logical_name.
    """
    head, _, rest = phash.name.partition(".")
    if len(head) != 16 or any(c not in "0123456789abcdefABCDEF" for c in head):
        return phash.name
    try:
        media_name = json.loads(phash.with_name(f"{head}.json").read_text(encoding="utf-8")).get("media_name")
    except (OSError, ValueError, AttributeError):
        return phash.name
    return f"{media_name}.{rest}" if isinstance(media_name, str) and media_name else phash.name


def phash_media_name(phash: Path) -> str:
    """phash_logical_name without its .phash extension: what a legacy name's Path.stem gave."""
    return phash_logical_name(phash).removesuffix(".phash")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phash", type=Path, nargs="?", default=None, help="Path to a .phash file")
    parser.add_argument("--check", "--dry-run", action="store_true", help="Perform sanity check")
    args = parser.parse_args()

    if args.check or args.phash is None:
        print("[CHECK] Helper phash_names opérationnel.")
        dummy = Path("0123456789abcdef.24fps.phash")
        res = phash_logical_name(dummy)
        print(f"[CHECK] Dummy test : {dummy.name} -> {res}")
        return 0

    print(f"logical_name: {phash_logical_name(args.phash)}")
    print(f"media_name: {phash_media_name(args.phash)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
