#!/usr/bin/env python3
"""Extracts and verifies embedded subtitles from NAS video files matching snippets."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


def main():
    nas_root = Path("/media/latty/Synology")
    data_srt_dir = Path("data/SRT")
    data_srt_dir.mkdir(parents=True, exist_ok=True)

    with open("data/snippets/metadata.json") as f:
        snippets = json.load(f)

    # Map base names to full video paths on NAS
    nas_videos: dict[str, Path] = {}
    print("Indexing NAS video containers...")
    for p in nas_root.rglob("*"):
        if p.is_file() and p.suffix.lower() in (".mkv", ".mp4", ".avi", ".webm"):
            nas_videos[p.stem] = p

    print(f"Found {len(nas_videos)} candidate video containers on NAS.")

    extracted_count = 0
    for s_name, s_info in snippets.items():
        raw_stem = s_info.get("raw_name", "").replace("_mid15s.mp4", "")
        if raw_stem not in nas_videos:
            continue

        vpath = nas_videos[raw_stem]
        dst_srt = data_srt_dir / f"{s_name}.fr.srt"
        if dst_srt.exists():
            continue

        # Probe for subtitle streams
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
                continue
            data = json.loads(res.stdout)
            streams = data.get("streams", [])
            if not streams:
                continue

            # Find French stream first, then any stream
            target_idx = None
            for s in streams:
                lang = s.get("tags", {}).get("language", "").lower()
                if lang in ("fre", "fra", "fr"):
                    target_idx = s.get("index")
                    break
            if target_idx is None:
                # English fallback if no French
                for s in streams:
                    lang = s.get("tags", {}).get("language", "").lower()
                    if lang in ("eng", "en"):
                        target_idx = s.get("index")
                        dst_srt = data_srt_dir / f"{s_name}.en.srt"
                        break

            if target_idx is not None:
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
                ext_res = subprocess.run(cmd_ext, capture_output=True, timeout=30)
                if ext_res.returncode == 0 and dst_srt.exists() and dst_srt.stat().st_size > 100:
                    extracted_count += 1
                    if extracted_count % 20 == 0:
                        print(f"Extracted {extracted_count} embedded subtitles...")
        except Exception:
            pass

    print(f"Total embedded subtitles extracted and certified: {extracted_count}")


if __name__ == "__main__":
    main()
