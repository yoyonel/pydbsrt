#!/usr/bin/env python3
"""Registers verified adjacent .srt files into data/SRT/couplings.json."""

from __future__ import annotations

import argparse
import hashlib
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


def file_hash(path: Path) -> str:
    h = hashlib.md5()
    h.update(path.read_bytes())
    return h.hexdigest()


def count_cues(srt_path: Path) -> int:
    try:
        text = srt_path.read_text(encoding="utf-8", errors="ignore")
        return len([line for line in text.splitlines() if "-->" in line])
    except Exception:
        return 0


def build_coupling_entry(
    m_hash: str,
    stem: str,
    phash_cand: Path,
    sub_hash: str,
    srt: Path,
    cues: int,
) -> dict:
    """Builds certified coupling entry dictionary."""
    name_lower = srt.name.lower()
    lang = "fr" if ".fr." in name_lower or "vostfr" in name_lower or "french" in name_lower else "en"
    return {
        "media_hash": m_hash,
        "media_name": stem,
        "phash_filename": phash_cand.name,
        "fps": 24.0,
        "frame_count": 360,
        "in_postgres": False,
        "subtitles": [
            {
                "subtitles_hash": sub_hash,
                "name": srt.name,
                "lang": lang,
                "srt_path": srt.name,
                "certified": True,
                "cue_count": cues,
            }
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Register adjacent .srt files into couplings.json")
    parser.add_argument(
        "--couplings-file",
        type=Path,
        default=REPO_ROOT / "data/SRT/couplings.json",
        help="Path to couplings registry (default: data/SRT/couplings.json)",
    )
    parser.add_argument(
        "--srt-dir",
        type=Path,
        default=REPO_ROOT / "data/SRT",
        help="Path to SRT directory (default: data/SRT)",
    )
    parser.add_argument(
        "--snippets-dir",
        type=Path,
        default=REPO_ROOT / "data/snippets",
        help="Path to snippets directory (default: data/snippets)",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        dest="dry_run",
        action="store_true",
        help="Scan and report without writing to couplings.json",
    )
    args = parser.parse_args()

    couplings_path = args.couplings_file
    if couplings_path.exists():
        with open(couplings_path, encoding="utf-8") as f:
            registry = json.load(f)
    else:
        registry = {"version": 1, "couplings": []}

    existing_media_names = {c["media_name"] for c in registry.get("couplings", [])}
    data_srt_dir = args.srt_dir
    snippets_dir = args.snippets_dir

    added = 0
    for srt in sorted(data_srt_dir.glob("*_mid15s.srt")):
        stem = srt.stem  # e.g. "Severance.S01E01..._mid15s"
        phash_cand = snippets_dir / f"{stem}.phash"
        if not phash_cand.exists() or stem in existing_media_names:
            continue

        cues = count_cues(srt)
        if cues == 0:
            continue

        m_hash = file_hash(phash_cand)
        sub_hash = file_hash(srt)
        entry = build_coupling_entry(m_hash, stem, phash_cand, sub_hash, srt, cues)
        registry["couplings"].append(entry)
        existing_media_names.add(stem)
        added += 1

    if not args.dry_run:
        couplings_path.parent.mkdir(parents=True, exist_ok=True)
        with open(couplings_path, "w", encoding="utf-8") as f:
            json.dump(registry, f, indent=2, ensure_ascii=False)

    mode_label = "[DRY-RUN] " if args.dry_run else ""
    print(f"{mode_label}Registered {added} verified adjacent couplings. Total couplings: {len(registry['couplings'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
