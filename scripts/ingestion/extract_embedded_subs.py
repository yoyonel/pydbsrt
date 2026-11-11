#!/usr/bin/env python3
"""Extracts and verifies embedded subtitles from NAS video files matching snippets."""

from __future__ import annotations

import argparse
import getpass
import json
import os
import shutil
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


def default_media_root() -> Path:
    env_root = os.environ.get("MEDIA_ROOT") or os.environ.get("NAS_ROOT")
    if env_root:
        return Path(env_root)
    user = os.environ.get("USER") or getpass.getuser()
    candidate = Path("/media") / user / "Synology"
    if candidate.exists():
        return candidate
    return Path.cwd()


def find_subtitle_stream(streams: list[dict]) -> tuple[int | None, str]:
    """Finds best subtitle stream (French first, then English fallback)."""
    for s in streams:
        lang = s.get("tags", {}).get("language", "").lower()
        if lang in ("fre", "fra", "fr"):
            return s.get("index"), "fr"
    for s in streams:
        lang = s.get("tags", {}).get("language", "").lower()
        if lang in ("eng", "en"):
            return s.get("index"), "en"
    return None, ""


def extract_target_subtitle(vpath: Path, target_idx: int, dst_srt: Path) -> bool:
    """Extracts a single subtitle stream via ffmpeg into dst_srt."""
    cmd_ext = [
        "ffmpeg",
        "-y",
        "-v",
        "error",
        "-i",
        str(vpath),
        "-map",
        f"0:{target_idx}",
        str(dst_srt),
    ]
    try:
        ext_res = subprocess.run(cmd_ext, capture_output=True, timeout=30)
        return ext_res.returncode == 0 and dst_srt.exists() and dst_srt.stat().st_size > 100
    except Exception:
        return False


def probe_and_extract_subtitle(vpath: Path, s_name: str, data_srt_dir: Path) -> bool:
    """Probes container and extracts best subtitle stream if needed."""
    dst_fr = data_srt_dir / f"{s_name}.fr.srt"
    dst_en = data_srt_dir / f"{s_name}.en.srt"
    if dst_fr.exists() or dst_en.exists():
        return False

    cmd_probe = [
        "ffprobe",
        "-v",
        "error",
        "-select_streams",
        "s",
        "-show_entries",
        "stream=index:stream_tags=language,title",
        "-of",
        "json",
        str(vpath),
    ]
    try:
        res = subprocess.run(cmd_probe, capture_output=True, text=True, timeout=10)
        if res.returncode != 0:
            return False
        data = json.loads(res.stdout)
        streams = data.get("streams", [])
        if not streams:
            return False

        target_idx, lang = find_subtitle_stream(streams)
        if target_idx is None:
            return False

        dst_srt = data_srt_dir / f"{s_name}.{lang}.srt"
        return extract_target_subtitle(vpath, target_idx, dst_srt)
    except Exception:
        return False


def perform_check(media_root: Path, srt_dir: Path, metadata_file: Path) -> None:
    """Performs sanity check of input files and tools."""
    print("[CHECK] Mode verification activé.")
    print(f"[CHECK] REPO_ROOT : {REPO_ROOT}")
    print(f"[CHECK] Media root : {media_root} (exists: {media_root.is_dir()})")
    print(f"[CHECK] SRT dir : {srt_dir} (exists: {srt_dir.is_dir()})")
    srt_count = len(list(srt_dir.glob("*.srt"))) if srt_dir.is_dir() else 0
    print(f"[CHECK] Existing .srt files : {srt_count}")
    print(f"[CHECK] Metadata file : {metadata_file} (exists: {metadata_file.is_file()})")
    has_ffprobe = shutil.which("ffprobe") is not None
    has_ffmpeg = shutil.which("ffmpeg") is not None
    print(f"[CHECK] Tools available : ffprobe={has_ffprobe}, ffmpeg={has_ffmpeg}")


def main():
    parser = argparse.ArgumentParser(description="Extract embedded subtitles from video files")
    parser.add_argument(
        "--media-root",
        type=Path,
        default=default_media_root(),
        help="Root path to search for video files (default: $MEDIA_ROOT, Synology or current directory)",
    )
    parser.add_argument(
        "--srt-dir",
        type=Path,
        default=REPO_ROOT / "data/SRT",
        help="Target subtitles directory (default: data/SRT)",
    )
    parser.add_argument(
        "--metadata-file",
        type=Path,
        default=REPO_ROOT / "data/snippets/metadata.json",
        help="Path to metadata.json catalog",
    )
    parser.add_argument(
        "--check", "--dry-run", action="store_true", help="Perform sanity checks without extracting subtitles"
    )
    args = parser.parse_args()

    if args.check:
        perform_check(args.media_root, args.srt_dir, args.metadata_file)
        return 0

    media_root = args.media_root
    data_srt_dir = args.srt_dir
    data_srt_dir.mkdir(parents=True, exist_ok=True)

    if not args.metadata_file.exists():
        print(f"Error: {args.metadata_file} not found.", file=sys.stderr)
        return 1

    with open(args.metadata_file) as f:
        snippets = json.load(f)

    # Map base names to full video paths
    media_videos: dict[str, Path] = {}
    print("Indexing media video containers...")
    for p in media_root.rglob("*"):
        if p.is_file() and p.suffix.lower() in (".mkv", ".mp4", ".avi", ".webm"):
            media_videos[p.stem] = p

    print(f"Found {len(media_videos)} candidate video containers.")

    extracted_count = 0
    for s_name, s_info in snippets.items():
        raw_stem = s_info.get("raw_name", "").replace("_mid15s.mp4", "")
        if raw_stem not in media_videos:
            continue

        vpath = media_videos[raw_stem]
        if probe_and_extract_subtitle(vpath, s_name, data_srt_dir):
            extracted_count += 1
            if extracted_count % 20 == 0:
                print(f"Extracted {extracted_count} embedded subtitles...")

    print(f"Total embedded subtitles extracted and certified: {extracted_count}")
    return 0


if __name__ == "__main__":
    main()
