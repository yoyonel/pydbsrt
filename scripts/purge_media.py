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


def purge_filesystem(stem: str) -> list[str]:
    """Removes video, phash, srt, and screenshots matching stem."""
    removed = []
    patterns = [
        ("data/snippets", f"{stem}*"),
        ("data/SRT", f"{stem}*"),
        ("docs/screenshots/e2e_vlc_all", f"*{stem}*"),
        ("docs/screenshots/e2e_vlc_50", f"*{stem}*"),
    ]
    for dir_name, pattern in patterns:
        d = Path(dir_name)
        if d.exists():
            for p in d.glob(pattern):
                if p.is_file():
                    p.unlink()
                    removed.append(str(p))
    return removed


def purge_couplings(stem: str) -> bool:
    """Removes media from data/SRT/couplings.json."""
    p = Path("data/SRT/couplings.json")
    if not p.exists():
        return False
    try:
        with open(p, encoding="utf-8") as f:
            data = json.load(f)
        couplings = data.get("couplings", [])
        before = len(couplings)
        filtered = [c for c in couplings if stem not in c.get("media_name", "")]
        if len(filtered) != before:
            data["couplings"] = filtered
            with open(p, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=2, ensure_ascii=False)
            return True
    except Exception as e:
        print(f"Warning: Failed to update couplings.json: {e}")
    return False


def purge_postgres(stem: str):
    """Purges media record in PostgreSQL 17 (ON DELETE CASCADE handles frames/subs)."""
    psql_port = os.environ.get("PSQL_PORT", "5433")
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


def purge_manifests(stem: str) -> bool:
    """Removes media entry from test manifests."""
    manifest_p = Path("docs/screenshots/e2e_vlc_all/manifest.json")
    if not manifest_p.exists():
        return False
    try:
        with open(manifest_p, encoding="utf-8") as f:
            items = json.load(f)
        before = len(items)
        filtered = [item for item in items if stem not in item.get("video", "")]
        if len(filtered) != before:
            with open(manifest_p, "w", encoding="utf-8") as f:
                json.dump(filtered, f, indent=2, ensure_ascii=False)
            return True
    except Exception as e:
        print(f"Warning: Failed to update manifest.json: {e}")
    return False


def main():
    parser = argparse.ArgumentParser(description="Purge media across all tiers")
    parser.add_argument("stem", help="Media stem or partial filename to purge globally")
    args = parser.parse_args()

    stem = args.stem.strip()
    if not stem:
        print("Error: Empty stem provided")
        sys.exit(1)

    print(f"=== Purging media '{stem}' globally across all tiers ===")
    removed = purge_filesystem(stem)
    print(f"1. Filesystem: {len(removed)} file(s) deleted")
    for r in removed:
        print(f"   • {r}")

    coup_updated = purge_couplings(stem)
    print(f"2. Couplings registry: {'Updated' if coup_updated else 'No matching record'}")

    purge_postgres(stem)
    print("3. PostgreSQL: Media cascade deleted (if connected)")

    manifest_updated = purge_manifests(stem)
    print(f"4. Test manifest: {'Updated' if manifest_updated else 'No matching record'}")

    # Rebuild dashboard
    if manifest_updated:
        print("5. Rebuilding interactive dashboard...")
        subprocess.run([sys.executable, "scripts/build_dashboard_html.py"], check=False)

    print(f"✓ Global purge completed for '{stem}'.")


if __name__ == "__main__":
    main()
