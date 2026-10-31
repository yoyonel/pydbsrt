#!/usr/bin/env python3
"""Scans mounted Freebox disks to discover eligible movies and TV shows for pydbsrt ingestion.

Applies strict directory filtering to isolate cinema/TV releases (Téléchargements, Vidéos)
and systematically exclude personal media (Photos, WhatsApp backups, recordings).
Outputs a target list ready for o2switch HTTP stream batch ingestion.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import unicodedata
from pathlib import Path
from typing import Any, Dict, List, Set

# Named Operational Constants
DEFAULT_LOCAL_FREEBOX_ROOT: str = f"/media/{os.getenv('USER', 'latty')}/Freebox"
DEFAULT_RPI4_FREEBOX_ROOT: str = "/mnt/freebox"
DEFAULT_OUTPUT_TARGETS_FILE: str = "data/reports/freebox_batch_targets.json"

SUPPORTED_VIDEO_EXTENSIONS: tuple[str, ...] = (
    ".mp4",
    ".mkv",
    ".avi",
    ".mov",
    ".webm",
    ".m4v",
)

# Strict Whitelist: only official media libraries are scanned
ELIGIBLE_SUBDIRECTORIES: tuple[str, ...] = (
    "Disque_1/Téléchargements",
    "Disque_2/Téléchargements",
    "Disque_1_1/Vidéos",
)

# Strict Blacklist: personal media and system artifacts are ignored
EXCLUDED_DIRECTORY_NAMES: Set[str] = {
    "photos",
    "backup",
    "enregistrements",
    ".cameras",
    ".fbxgrabberd",
    ".fbxtimeshifting",
    "lost+found",
    ".trash-1000",
}


def resolve_freebox_root(explicit_root: str | None) -> Path:
    """Resolves the active Freebox root mount point (local machine or RPi4)."""
    if explicit_root:
        return Path(explicit_root).resolve()
    for candidate in (DEFAULT_RPI4_FREEBOX_ROOT, DEFAULT_LOCAL_FREEBOX_ROOT):
        cand_path = Path(candidate)
        if cand_path.is_dir() and any(cand_path.iterdir()):
            return cand_path.resolve()
    return Path(DEFAULT_LOCAL_FREEBOX_ROOT).resolve()


def is_path_excluded(rel_parts: tuple[str, ...]) -> bool:
    """Checks if any segment in the path hierarchy matches excluded directories."""
    return any(part.lower() in EXCLUDED_DIRECTORY_NAMES for part in rel_parts)


def _extract_target_from_file(full_file_path: Path, root_dir: Path) -> Dict[str, Any] | None:
    """Builds a target entry for a single video file."""
    ext = full_file_path.suffix.lower()
    if ext not in SUPPORTED_VIDEO_EXTENSIONS:
        return None
    try:
        size_bytes = full_file_path.stat().st_size
    except OSError:
        return None

    rel_file_path = full_file_path.relative_to(root_dir).as_posix()
    fname = full_file_path.name
    return {
        "path": rel_file_path,
        "filename": unicodedata.normalize("NFC", fname),
        "stem": unicodedata.normalize("NFC", full_file_path.stem),
        "size_bytes": size_bytes,
    }


def _collect_dir_targets(dirpath: str, filenames: list[str], root_dir: Path) -> List[Dict[str, Any]]:
    """Extracts valid target entries from filenames in a single directory."""
    found: List[Dict[str, Any]] = []
    for fname in sorted(filenames):
        target = _extract_target_from_file(Path(dirpath) / fname, root_dir)
        if target:
            found.append(target)
    return found


def _walk_directory_targets(sub_path: Path, root_dir: Path, targets: List[Dict[str, Any]], limit: int | None) -> bool:
    """Collects targets from a single subdirectory tree. Returns True if limit reached."""
    for dirpath, dirnames, filenames in os.walk(sub_path):
        rel_dir = Path(dirpath).relative_to(root_dir)
        dirnames[:] = [d for d in dirnames if d.lower() not in EXCLUDED_DIRECTORY_NAMES]
        if is_path_excluded(rel_dir.parts):
            continue

        for item in _collect_dir_targets(dirpath, filenames, root_dir):
            targets.append(item)
            if limit is not None and len(targets) >= limit:
                return True
    return False


def scan_freebox_targets(root_dir: Path, limit: int | None = None) -> List[Dict[str, Any]]:
    """Walks the eligible Freebox subdirectories and collects video targets."""
    targets: List[Dict[str, Any]] = []
    for rel_sub in ELIGIBLE_SUBDIRECTORIES:
        sub_path = root_dir / rel_sub
        if sub_path.is_dir() and _walk_directory_targets(sub_path, root_dir, targets, limit):
            break
    return targets


def main() -> int:
    """CLI entry point for scanning Freebox media targets."""
    parser = argparse.ArgumentParser(description="Scan Freebox volumes for eligible cinema and TV media targets.")
    parser.add_argument("--root", default=None, help="Root mount directory of Freebox (default: auto-detect)")
    parser.add_argument("--output", default=DEFAULT_OUTPUT_TARGETS_FILE, help="Output JSON path")
    parser.add_argument("--limit", type=int, default=None, help="Optional maximum number of targets to collect")
    args = parser.parse_args()

    root_path = resolve_freebox_root(args.root)
    if not root_path.is_dir():
        print(f"❌ Error: Freebox root '{root_path}' does not exist or is not mounted.", file=sys.stderr)
        return 1

    print(f"🔍 Scanning eligible media from Freebox root: {root_path}...")
    targets = scan_freebox_targets(root_path, limit=args.limit)

    out_file = Path(args.output)
    out_file.parent.mkdir(parents=True, exist_ok=True)
    out_file.write_text(json.dumps(targets, indent=2, ensure_ascii=False), encoding="utf-8")

    print(f"✓ Found {len(targets)} qualified video targets (Photos/ and Backup/ excluded).")
    print(f"✓ Saved targets list to: {out_file}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
