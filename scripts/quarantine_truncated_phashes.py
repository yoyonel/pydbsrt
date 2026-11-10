#!/usr/bin/env python3
"""Quarantines truncated, corrupted, and undersized .phash files from data/__PHASH__/ into data/__QUARANTINE_PHASH__/.

Zero speculation: Moves files < 50 KB (less than ~6250 frames / ~4 min at 24fps) or non-multiples of 8 bytes,
emitting an audit trail report in JSON format.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

MIN_FILE_SIZE_BYTES: int = 50_000  # ~6,250 frames @ 8B/frame
BYTES_PER_FRAME: int = 8
DEFAULT_SRC_DIR: Path = Path("data/__PHASH__")
DEFAULT_DST_DIR: Path = Path("data/__QUARANTINE_PHASH__")
DEFAULT_REPORT_PATH: Path = Path("data/reports/quarantine_report.json")


def evaluate_file_quarantine(file_path: Path) -> tuple[str | None, int]:
    """Evaluates whether a phash file should be quarantined, returning (reason, size)."""
    size = file_path.stat().st_size
    if size == 0:
        return "zero_bytes", size
    if size % BYTES_PER_FRAME != 0:
        return "corrupted_non_multiple_of_8_bytes", size
    if size < MIN_FILE_SIZE_BYTES:
        frames = size // BYTES_PER_FRAME
        return f"truncated_under_threshold_{size}B_{frames}_frames", size
    return None, size


def move_to_quarantine(file_path: Path, dst_dir: Path, size: int, reason: str, dry_run: bool) -> dict[str, object]:
    """Moves a corrupted/truncated phash and its optional sidecar to dst_dir."""
    target_path = dst_dir / file_path.name
    record: dict[str, object] = {
        "filename": file_path.name,
        "size_bytes": size,
        "frames": size // BYTES_PER_FRAME,
        "reason": reason,
        "moved_to": str(target_path),
    }

    sidecar = file_path.with_suffix(".json")
    has_sidecar = sidecar.exists()

    if not dry_run:
        shutil.move(str(file_path), str(target_path))
        if has_sidecar:
            shutil.move(str(sidecar), str(dst_dir / sidecar.name))

    return record


def scan_and_quarantine(
    src_dir: Path,
    dst_dir: Path,
    report_path: Path,
    dry_run: bool = False,
) -> int:
    """Scans src_dir for truncated or corrupted phashes, moves them to dst_dir, and writes a report."""
    if not src_dir.exists():
        print(f"Error: Source directory does not exist: {src_dir}", file=sys.stderr)
        return 0

    dst_dir.mkdir(parents=True, exist_ok=True)
    report_path.parent.mkdir(parents=True, exist_ok=True)

    quarantined: list[dict[str, object]] = []
    files = sorted(src_dir.glob("*.phash"))

    for file_path in files:
        reason, size = evaluate_file_quarantine(file_path)
        if reason:
            rec = move_to_quarantine(file_path, dst_dir, size, reason, dry_run)
            quarantined.append(rec)

    report = {
        "timestamp_unix": int(time.time()),
        "source_dir": str(src_dir),
        "quarantine_dir": str(dst_dir),
        "total_scanned": len(files),
        "total_quarantined": len(quarantined),
        "dry_run": dry_run,
        "items": quarantined,
    }

    if not dry_run:
        report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")

    mode_label = "[DRY-RUN] " if dry_run else ""
    print(f"{mode_label}Quarantine complete: {len(quarantined)}/{len(files)} files quarantined.")
    print(f"Report written to: {report_path}")

    return len(quarantined)


def main() -> int:
    parser = argparse.ArgumentParser(description="Quarantine truncated/corrupted phash files")
    parser.add_argument("--src", type=Path, default=DEFAULT_SRC_DIR, help="Source phash directory")
    parser.add_argument("--dst", type=Path, default=DEFAULT_DST_DIR, help="Quarantine directory")
    parser.add_argument("--report", type=Path, default=DEFAULT_REPORT_PATH, help="Output JSON report")
    parser.add_argument("--dry-run", action="store_true", help="Scan and report without moving files")
    args = parser.parse_args()

    count = scan_and_quarantine(args.src, args.dst, args.report, dry_run=args.dry_run)
    return 0 if count >= 0 else 1


if __name__ == "__main__":
    sys.exit(main())
