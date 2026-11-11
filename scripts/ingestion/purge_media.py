#!/usr/bin/env python3
"""Unified media purge script for pydbsrt.

Purges media, subtitles, fingerprints, couplings, and cache across all 5 system tiers:
1. Local snippet and phash files (data/snippets/)
2. Local SRT files (data/SRT/)
3. PostgreSQL 17 relational database (CASCADE purge)
4. couplings.json Zero-DB registry
5. Test manifests and dashboard records
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
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


def purge_filesystem(stem: str, dry_run: bool = False) -> list[str]:
    """Removes video, phash, srt, and screenshots matching stem."""
    removed = []
    patterns = [
        (REPO_ROOT / "data/snippets", f"{stem}*"),
        (REPO_ROOT / "data/SRT", f"{stem}*"),
        (REPO_ROOT / "docs/screenshots/e2e_vlc_all", f"*{stem}*"),
        (REPO_ROOT / "docs/screenshots/e2e_vlc_50", f"*{stem}*"),
    ]
    for d, pattern in patterns:
        if d.exists():
            for p in d.glob(pattern):
                if p.is_file():
                    if not dry_run:
                        p.unlink()
                    removed.append(str(p))
    return removed


def purge_couplings(stem: str, dry_run: bool = False) -> bool:
    """Removes media from data/SRT/couplings.json."""
    p = REPO_ROOT / "data/SRT/couplings.json"
    if not p.exists():
        return False
    try:
        with open(p, encoding="utf-8") as f:
            data = json.load(f)
        couplings = data.get("couplings", [])
        before = len(couplings)
        filtered = [c for c in couplings if stem not in c.get("media_name", "")]
        if len(filtered) != before:
            if not dry_run:
                data["couplings"] = filtered
                with open(p, "w", encoding="utf-8") as f:
                    json.dump(data, f, indent=2, ensure_ascii=False)
            return True
    except Exception as e:
        print(f"Warning: Failed to update couplings.json: {e}")
    return False


def purge_postgres(stem: str, dry_run: bool = False):
    """Purges media record in PostgreSQL 17 (ON DELETE CASCADE handles frames/subs)."""
    if dry_run:
        return
    psql_port = os.environ.get("PSQL_PORT", "5432")
    cmd = [
        "psql",
        "-h",
        "127.0.0.1",
        "-p",
        psql_port,
        "-U",
        "postgres",
        "-d",
        "pydbsrt",
        "-c",
        f"DELETE FROM medias WHERE name ILIKE '%{stem}%';",
    ]
    try:
        subprocess.run(cmd, capture_output=True, timeout=5, env=dict(os.environ, PGPASSWORD="password"))
    except Exception:
        pass


def purge_manifests(stem: str, dry_run: bool = False) -> bool:
    """Removes media entry from test manifests."""
    manifest_p = REPO_ROOT / "docs/screenshots/e2e_vlc_all/manifest.json"
    if not manifest_p.exists():
        return False
    try:
        with open(manifest_p, encoding="utf-8") as f:
            items = json.load(f)
        before = len(items)
        filtered = [item for item in items if stem not in item.get("video", "")]
        if len(filtered) != before:
            if not dry_run:
                with open(manifest_p, "w", encoding="utf-8") as f:
                    json.dump(filtered, f, indent=2, ensure_ascii=False)
            return True
    except Exception as e:
        print(f"Warning: Failed to update manifest.json: {e}")
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description="Purge media across all tiers")
    parser.add_argument("stem", nargs="?", default="", help="Media stem or partial filename to purge globally")
    parser.add_argument(
        "--check", "--dry-run", dest="dry_run", action="store_true", help="Simulate purge without deleting anything"
    )
    args = parser.parse_args()

    stem = args.stem.strip()
    if not stem:
        if args.dry_run:
            print("[CHECK] Mode verification activé (aucune suppression).")
            print(f"[CHECK] REPO_ROOT : {REPO_ROOT}")
            print(f"[CHECK] Couplings exists : {(REPO_ROOT / 'data/SRT/couplings.json').is_file()}")
            print(f"[CHECK] Snippets exists : {(REPO_ROOT / 'data/snippets').is_dir()}")
            return 0
        print("Error: Empty stem provided", file=sys.stderr)
        return 1

    mode_str = " (SIMULATION DRY-RUN)" if args.dry_run else ""
    print(f"=== Purging media '{stem}' globally across all tiers{mode_str} ===")
    removed = purge_filesystem(stem, dry_run=args.dry_run)
    action_label = "would be deleted" if args.dry_run else "file(s) deleted"
    print(f"1. Filesystem: {len(removed)} {action_label}")
    for r in removed:
        print(f"   • {r}")

    coup_updated = purge_couplings(stem, dry_run=args.dry_run)
    print(f"2. Couplings registry: {'Found and matched' if coup_updated else 'No matching record'}")

    purge_postgres(stem, dry_run=args.dry_run)
    pg_label = "Skipped in dry-run" if args.dry_run else "Media cascade deleted (if connected)"
    print(f"3. PostgreSQL: {pg_label}")

    manifest_updated = purge_manifests(stem, dry_run=args.dry_run)
    print(f"4. Test manifest: {'Found and matched' if manifest_updated else 'No matching record'}")

    if manifest_updated and not args.dry_run:
        print("5. Rebuilding interactive dashboard...")
        builder = REPO_ROOT / "scripts/tooling/build_dashboard_html.py"
        if builder.is_file():
            subprocess.run([sys.executable, str(builder)], check=False)

    print(f"✓ Global purge{mode_str} completed for '{stem}'.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
