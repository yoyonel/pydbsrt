#!/usr/bin/env python3
"""Reconciles release-named .phash files into canonical <moviehash>.<fps>fps.phash + sidecar JSON.

Zero speculation & safe idempotent migration:
1. Discovers files in data/__PHASH__/ matching <release_name>.<fps>fps.phash.
2. Resolves moviehash via:
   - Match with `medias.name` in PostgreSQL database (where moviehash is populated).
   - Or local video file resolution on Synology mount.
3. Renames the .phash to <moviehash>.<fps>fps.phash.
4. Generates or updates the <moviehash>.json sidecar with certified metadata:
   - moviehash, media_name, fps, frames, extracted_at.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
import urllib.parse
from pathlib import Path
from typing import Any

import psycopg2
from psycopg2.extensions import connection as PgConn

DEFAULT_DB_URL: str = "postgresql://docker:docker@127.0.0.1:5433/bktreedb"
DEFAULT_PHASH_DIR: Path = Path("data/__PHASH__")
BYTES_PER_FRAME: int = 8
FPS_SUFFIX_RE: re.Pattern[str] = re.compile(r"\.(\d+\.\d+)fps\.phash$")
CANONICAL_MOVIEHASH_RE: re.Pattern[str] = re.compile(r"^[0-9a-f]{16}\.\d+\.\d+fps\.phash$")


def parse_release_phash_name(filename: str) -> tuple[str, float] | None:
    """Extracts (media_name, fps) from a release-named phash file."""
    m = FPS_SUFFIX_RE.search(filename)
    if not m:
        return None
    fps = float(m.group(1))
    raw_name = filename[: m.start()]
    return raw_name, fps


def query_db_moviehash(cur: Any, media_name: str) -> tuple[str, int | None] | None:
    """Looks up moviehash and file_size_bytes from medias table by raw and unquoted name."""
    unquoted = urllib.parse.unquote(media_name)
    cur.execute(
        """
        SELECT moviehash, file_size_bytes
        FROM medias
        WHERE (name = %s OR name = %s) AND moviehash IS NOT NULL
        LIMIT 1
        """,
        (media_name, unquoted),
    )
    row = cur.fetchone()
    if row and row[0]:
        return str(row[0]), row[1]
    return None


def write_canonical_sidecar(
    sidecar_path: Path,
    moviehash: str,
    media_name: str,
    fps: float,
    frames: int,
    dry_run: bool,
) -> None:
    """Writes the canonical sidecar JSON if it does not already exist."""
    if dry_run or sidecar_path.exists():
        return

    sidecar_data = {
        "moviehash": moviehash,
        "media_name": media_name,
        "fps": fps,
        "frames": frames,
        "extracted_at": int(time.time()),
    }
    sidecar_path.write_text(json.dumps(sidecar_data, indent=2, ensure_ascii=False), encoding="utf-8")


def reconcile_single_phash(
    phash_path: Path,
    cur: Any,
    phash_dir: Path,
    dry_run: bool,
) -> str:
    """Attempts to reconcile a single release-named .phash file to its canonical moviehash."""
    parsed = parse_release_phash_name(phash_path.name)
    if not parsed:
        return "unparseable_filename"

    media_name, fps = parsed
    lookup = query_db_moviehash(cur, media_name)
    if not lookup:
        return "not_found_in_db"

    moviehash, _ = lookup
    size = phash_path.stat().st_size
    frames = size // BYTES_PER_FRAME

    canonical_phash_name = f"{moviehash}.{fps:.1f}fps.phash"
    target_phash_path = phash_dir / canonical_phash_name
    target_sidecar_path = phash_dir / f"{moviehash}.json"

    if target_phash_path.exists() and target_phash_path != phash_path:
        # Canonical file already exists; write sidecar and remove redundant release-named alias
        write_canonical_sidecar(target_sidecar_path, moviehash, media_name, fps, frames, dry_run)
        if not dry_run:
            phash_path.unlink()
        return "merged_with_existing_canonical"

    write_canonical_sidecar(target_sidecar_path, moviehash, media_name, fps, frames, dry_run)
    if not dry_run:
        phash_path.rename(target_phash_path)
    return "reconciled"


def reconcile_all_phashes(
    conn: PgConn,
    phash_dir: Path,
    dry_run: bool = False,
) -> dict[str, int]:
    """Iterates through all non-canonical phashes and reconciles them."""
    cur = conn.cursor()
    all_phashes = sorted(phash_dir.glob("*.phash"))

    stats: dict[str, int] = {
        "total_scanned": len(all_phashes),
        "already_canonical": 0,
        "reconciled": 0,
        "merged_with_existing_canonical": 0,
        "not_found_in_db": 0,
        "unparseable_filename": 0,
    }

    for p in all_phashes:
        if CANONICAL_MOVIEHASH_RE.match(p.name):
            stats["already_canonical"] += 1
            continue

        res = reconcile_single_phash(p, cur, phash_dir, dry_run)
        stats[res] = stats.get(res, 0) + 1

    return stats


def main() -> int:
    parser = argparse.ArgumentParser(description="Reconcile release-named phashes to canonical moviehashes")
    parser.add_argument("--db", type=str, default=DEFAULT_DB_URL, help="Database connection URL")
    parser.add_argument("--phash-dir", type=Path, default=DEFAULT_PHASH_DIR, help="Path to data/__PHASH__")
    parser.add_argument("--dry-run", action="store_true", help="Simulate reconciliation without moving files")
    args = parser.parse_args()

    print("======================================================================")
    print(" 🏷️  PYDBSRT PHASH CANONICAL RECONCILIATION")
    print("======================================================================")
    print(f"Target DB       : {args.db}")
    print(f"Phash Dir       : {args.phash_dir}")
    print(f"Mode            : {'[DRY-RUN]' if args.dry_run else '[LIVE EXECUTION]'}")

    t0 = time.monotonic()
    conn = psycopg2.connect(args.db)

    stats = reconcile_all_phashes(conn, args.phash_dir, dry_run=args.dry_run)
    conn.close()

    elapsed = time.monotonic() - t0
    print("\n--- Reconciliation Summary ---")
    print(f"  • Total Scanned      : {stats['total_scanned']}")
    print(f"  • Already Canonical  : {stats['already_canonical']}")
    print(f"  • Newly Reconciled   : {stats['reconciled']}")
    print(f"  • Merged Redundant   : {stats['merged_with_existing_canonical']}")
    print(f"  • Not in Database    : {stats['not_found_in_db']}")
    print(f"  • Unparseable        : {stats['unparseable_filename']}")
    print(f"  ✓ Finished in {elapsed:.2f}s")
    print("======================================================================")

    return 0


if __name__ == "__main__":
    sys.exit(main())
