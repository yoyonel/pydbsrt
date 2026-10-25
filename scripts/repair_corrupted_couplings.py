#!/usr/bin/env python3
"""Repairs corrupted subtitle cue text (U+FFFD replacement characters) in couplings.json.

Re-reads original source .srt files from disk with multi-encoding fallback
(UTF-8 with BOM removal, Windows-1252 / ISO-8859-1) to restore lost accents
while strictly preserving SHA-256 hashes, timestamps, and relational frame anchors.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import sys
import tempfile
from pathlib import Path

DEFAULT_COUPLINGS_PATH = "data/SRT/couplings.json"
DEFAULT_SRT_DIR = "data/SRT"
REPLACEMENT_CHAR = "\ufffd"
ENCODING_CANDIDATES = ("utf-8-sig", "cp1252", "iso-8859-1", "iso-8859-15")

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("repair_couplings")


def decode_srt_file(path: Path) -> str | None:
    """Decodes subtitle file bytes using candidate encodings."""
    raw_bytes = path.read_bytes()
    for enc in ENCODING_CANDIDATES:
        try:
            return raw_bytes.decode(enc)
        except (UnicodeDecodeError, LookupError):
            continue
    return None


def parse_srt_cues(content: str) -> dict[int, str]:
    """Parses an SRT content string into a mapping of {cue_index: text}."""
    cue_map: dict[int, str] = {}
    normalized = content.replace("\r\n", "\n")
    blocks = normalized.split("\n\n")

    for block in blocks:
        lines = [line for line in block.splitlines() if line.strip()]
        if len(lines) >= 3 and "-->" in lines[1]:
            try:
                idx = int(lines[0].strip())
                text = "\n".join(lines[2:])
                cue_map[idx] = text
            except ValueError:
                continue

    return cue_map


def repair_registry(
    couplings_path: Path,
    srt_dir: Path,
    dry_run: bool = False,
) -> tuple[int, int]:
    """Inspects and repairs corrupted cue text in the couplings registry."""
    if not couplings_path.exists():
        logger.error("Couplings file not found: %s", couplings_path)
        sys.exit(1)

    with open(couplings_path, encoding="utf-8") as f:
        registry = json.load(f)

    repaired_cues_count = 0
    clean_cues_count = 0
    repaired_tracks_count = 0

    couplings = registry.get("couplings", [])
    for coupling in couplings:
        for sub in coupling.get("subtitles", []):
            sub_frames = sub.get("sub_frames", [])
            if not sub_frames:
                continue

            has_corruption = any(REPLACEMENT_CHAR in sf.get("text", "") for sf in sub_frames)
            if not has_corruption:
                clean_cues_count += len(sub_frames)
                continue

            srt_rel = sub.get("srt_path", "")
            srt_full = srt_dir / srt_rel
            if not srt_full.exists():
                logger.warning("Source SRT file missing on disk: %s", srt_full)
                continue

            content = decode_srt_file(srt_full)
            if content is None:
                logger.warning("Failed to decode SRT file: %s", srt_full)
                continue

            cue_map = parse_srt_cues(content)
            track_fixed = 0

            for sf in sub_frames:
                if REPLACEMENT_CHAR in sf.get("text", ""):
                    cue_idx = sf.get("index")
                    if cue_idx in cue_map:
                        sf["text"] = cue_map[cue_idx]
                        repaired_cues_count += 1
                        track_fixed += 1
                else:
                    clean_cues_count += 1

            if track_fixed > 0:
                repaired_tracks_count += 1
                logger.info(
                    "Repaired %d cues in track '%s' (%s)",
                    track_fixed,
                    sub.get("name", "unnamed"),
                    srt_rel,
                )

    logger.info(
        "Repaired %d corrupted cue entries across %d tracks, skipped %d already-clean entries",
        repaired_cues_count,
        repaired_tracks_count,
        clean_cues_count,
    )

    if not dry_run and repaired_cues_count > 0:
        parent_dir = couplings_path.parent
        with tempfile.NamedTemporaryFile("w", dir=parent_dir, delete=False, encoding="utf-8") as tmp:
            json.dump(registry, tmp, indent=2, ensure_ascii=False)
            tmp_path = Path(tmp.name)

        os.replace(tmp_path, couplings_path)
        logger.info("Atomically updated %s", couplings_path)

    return repaired_cues_count, clean_cues_count


def main() -> None:
    parser = argparse.ArgumentParser(description="Repairs corrupted subtitle cue text (U+FFFD) in couplings.json")
    parser.add_argument(
        "--couplings-path",
        type=Path,
        default=Path(DEFAULT_COUPLINGS_PATH),
        help=f"Path to couplings.json (default: {DEFAULT_COUPLINGS_PATH})",
    )
    parser.add_argument(
        "--srt-dir",
        type=Path,
        default=Path(DEFAULT_SRT_DIR),
        help=f"Base directory containing .srt files (default: {DEFAULT_SRT_DIR})",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Dry run without modifying couplings.json",
    )
    args = parser.parse_args()

    repair_registry(args.couplings_path, args.srt_dir, args.dry_run)


if __name__ == "__main__":
    main()
