#!/usr/bin/env python3
"""Extracts diverse 15-second video snippets and computes pHashes from Synology NAS.

Discovers video files on the NAS (tvshow and video), stratifies selection across
series and films, extracts a 15-second snippet at t=50% (milieu) with lightweight
encoding (480p, CRF 28, no audio), computes the perceptual hash stream via dbsrt,
and enforces a strict disk storage budget (default: 2000 MB / 2 GB).
"""

from __future__ import annotations

import argparse
import getpass
import json
import os
import re
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


def default_nas_root() -> Path:
    env_root = os.environ.get("NAS_ROOT")
    if env_root:
        return Path(env_root)
    return Path("/media") / getpass.getuser() / "Synology"


def get_dir_size(path: Path) -> int:
    """Calculates total size in bytes of a directory."""
    if not path.exists():
        return 0
    total = 0
    for entry in path.glob("**/*"):
        if entry.is_file():
            try:
                total += entry.stat().st_size
            except OSError:
                pass
    return total


def sanitize_filename(name: str) -> str:
    """Replaces unsafe filename characters with underscores."""
    clean = re.sub(r'[\\/*?:"<>|]', "_", name)
    clean = re.sub(r"\s+", "_", clean).strip("._")
    return clean[:120] if len(clean) > 120 else clean


def probe_duration(video_path: Path) -> float | None:
    """Probes video duration in seconds via ffprobe."""
    cmd = [
        "ffprobe",
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
        str(video_path),
    ]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
        if res.returncode == 0 and res.stdout.strip():
            val = float(res.stdout.strip())
            return val if val > 0 else None
    except (subprocess.TimeoutExpired, ValueError, OSError):
        pass
    return None


def discover_and_stratify_videos(nas_root: Path) -> list[tuple[Path, str]]:
    """Discovers all videos on NAS and stratifies across directories for maximum variety."""
    video_exts = (".mkv", ".mp4", ".avi", ".webm")
    groups: dict[str, list[Path]] = {}

    subdirs = [nas_root / "tvshow", nas_root / "video"]
    for sdir in subdirs:
        if not sdir.exists():
            continue
        for root, _dirs, files in os.walk(sdir):
            rpath = Path(root)
            group_key = rpath.name if rpath != sdir else "root"
            for f in files:
                if f.lower().endswith(video_exts):
                    groups.setdefault(group_key, []).append(rpath / f)

    # Sort files inside each group deterministically
    for gkey in groups:
        groups[gkey].sort(key=lambda p: p.name)

    # Round-robin selection across groups to ensure maximum diversity
    stratified: list[tuple[Path, str]] = []
    group_keys = sorted(groups.keys())
    max_in_any = max((len(v) for v in groups.values()), default=0)

    for round_idx in range(max_in_any):
        for gkey in group_keys:
            items = groups[gkey]
            if round_idx < len(items):
                stratified.append((items[round_idx], gkey))

    return stratified


def process_video_snippet(
    video_path: Path,
    category: str,
    output_dir: Path,
    dbsrt_bin: Path,
    duration_s: float,
    crf: int,
    scale: str,
) -> dict:
    """Extracts 15s snippet at t=50% and computes pHash."""
    t0 = time.monotonic()
    safe_stem = sanitize_filename(video_path.stem)
    out_mp4 = output_dir / f"{safe_stem}_mid15s.mp4"
    out_phash = output_dir / f"{safe_stem}_mid15s.phash"

    # If already fully extracted and valid, return cached info
    if out_mp4.exists() and out_phash.exists():
        try:
            mp4_sz = out_mp4.stat().st_size
            phash_sz = out_phash.stat().st_size
            if mp4_sz > 1024 and phash_sz > 4:
                return {
                    "video": video_path.name,
                    "category": category,
                    "status": "cached",
                    "mp4_file": out_mp4.name,
                    "mp4_bytes": mp4_sz,
                    "phash_file": out_phash.name,
                    "phash_frames": (phash_sz - 4) // 8,
                    "elapsed_s": round(time.monotonic() - t0, 2),
                }
        except OSError:
            pass

    # Probe duration
    total_dur = probe_duration(video_path)
    if total_dur is None or total_dur < 30.0:
        return {
            "video": video_path.name,
            "category": category,
            "status": "skipped_too_short_or_unreadable",
            "elapsed_s": round(time.monotonic() - t0, 2),
        }

    # Extract 15s at middle (t = 50%)
    start_time = max(0.0, (total_dur * 0.5) - (duration_s * 0.5))

    ffmpeg_cmd = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-ss",
        f"{start_time:.2f}",
        "-i",
        str(video_path),
        "-t",
        f"{duration_s:.2f}",
        "-vf",
        scale,
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        str(crf),
        "-an",
        str(out_mp4),
    ]

    try:
        res = subprocess.run(ffmpeg_cmd, capture_output=True, text=True, timeout=60)
        if res.returncode != 0 or not out_mp4.exists() or out_mp4.stat().st_size < 1024:
            return {
                "video": video_path.name,
                "category": category,
                "status": "error_ffmpeg",
                "error": res.stderr[-200:] if res.stderr else "ffmpeg failed",
                "elapsed_s": round(time.monotonic() - t0, 2),
            }
    except (subprocess.TimeoutExpired, OSError) as e:
        return {
            "video": video_path.name,
            "category": category,
            "status": "error_timeout_ffmpeg",
            "error": str(e),
            "elapsed_s": round(time.monotonic() - t0, 2),
        }

    # Compute pHash via dbsrt export-phash
    dbsrt_cmd = [
        str(dbsrt_bin),
        "export-phash",
        "--media",
        str(out_mp4),
        "--output",
        str(out_phash),
        "--hwaccel",
        "disabled",
    ]
    try:
        res_phash = subprocess.run(dbsrt_cmd, capture_output=True, text=True, timeout=30)
        if res_phash.returncode != 0 or not out_phash.exists() or out_phash.stat().st_size < 4:
            return {
                "video": video_path.name,
                "category": category,
                "status": "error_phash",
                "error": res_phash.stderr[-200:] if res_phash.stderr else "phash export failed",
                "elapsed_s": round(time.monotonic() - t0, 2),
            }
    except (subprocess.TimeoutExpired, OSError) as e:
        return {
            "video": video_path.name,
            "category": category,
            "status": "error_timeout_phash",
            "error": str(e),
            "elapsed_s": round(time.monotonic() - t0, 2),
        }

    mp4_bytes = out_mp4.stat().st_size
    phash_frames = (out_phash.stat().st_size - 4) // 8

    return {
        "video": video_path.name,
        "category": category,
        "status": "success",
        "total_duration_s": round(total_dur, 1),
        "start_time_s": round(start_time, 1),
        "mp4_file": out_mp4.name,
        "mp4_bytes": mp4_bytes,
        "phash_file": out_phash.name,
        "phash_frames": phash_frames,
        "elapsed_s": round(time.monotonic() - t0, 2),
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Extract diverse 15-second video snippets and pHashes within a strict storage budget."
    )
    parser.add_argument(
        "--nas-root",
        type=Path,
        default=default_nas_root(),
        help="Root path to Synology NAS (defaults to $NAS_ROOT or /media/$USER/Synology)",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("data/snippets"),
        help="Output directory for snippets and phash files (default: data/snippets)",
    )
    parser.add_argument(
        "--max-budget-mb",
        type=int,
        default=2000,
        help="Maximum disk budget in MB for output directory (default: 2000 MB / 2 GB)",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="Optional maximum number of videos to process",
    )
    parser.add_argument(
        "--workers",
        type=int,
        default=4,
        help="Number of concurrent worker threads (default: 4)",
    )
    parser.add_argument(
        "--duration",
        type=float,
        default=15.0,
        help="Snippet duration in seconds (default: 15.0)",
    )
    parser.add_argument(
        "--crf",
        type=int,
        default=28,
        help="H.264 CRF compression factor (default: 28)",
    )
    parser.add_argument(
        "--scale",
        type=str,
        default="scale=-2:480",
        help="FFmpeg video scale filter (default: scale=-2:480)",
    )
    parser.add_argument(
        "--dbsrt-bin",
        type=Path,
        default=Path("target/release/dbsrt"),
        help="Path to compiled dbsrt binary (default: target/release/dbsrt)",
    )

    args = parser.parse_args()

    if not args.nas_root.exists():
        print(f"❌ Error: NAS root directory not found: {args.nas_root}", file=sys.stderr)
        return 1

    if not args.dbsrt_bin.exists():
        print(f"❌ Error: dbsrt binary not found: {args.dbsrt_bin}. Run `task rs:build` first.", file=sys.stderr)
        return 1

    args.output_dir.mkdir(parents=True, exist_ok=True)
    max_budget_bytes = args.max_budget_mb * 1024 * 1024

    print("================================================================================")
    print(" 🎬 SYNOLOGY NAS SNIPPET & PHASH EXTRACTION PIPELINE")
    print("================================================================================")
    print(f"NAS Root:       {args.nas_root}")
    print(f"Output Dir:     {args.output_dir}")
    print(f"Disk Budget:    {args.max_budget_mb} MB ({(args.max_budget_mb / 1024):.2f} GB)")
    print(f"Snippet:        {args.duration}s @ mid 50% ({args.scale}, CRF {args.crf}, no-audio)")
    print(f"Workers:        {args.workers}")
    print("--------------------------------------------------------------------------------")

    print("🔍 Discovering and stratifying NAS videos across series and movies...")
    stratified = discover_and_stratify_videos(args.nas_root)
    total_found = len(stratified)
    print(f"✅ Discovered {total_found} videos across diverse series and movie directories.")

    if args.limit:
        stratified = stratified[: args.limit]
        print(f"📌 Limited target to {len(stratified)} videos.")

    current_size = get_dir_size(args.output_dir)
    print(
        f"📊 Current output dir size: {current_size / (1024 * 1024):.1f} MB / {args.max_budget_mb} MB "
        f"({(current_size / max_budget_bytes) * 100:.1f}%)"
    )

    manifest_path = args.output_dir / "manifest.json"
    manifest: dict[str, dict] = {}
    if manifest_path.exists():
        try:
            with open(manifest_path, encoding="utf-8") as f:
                manifest = json.load(f)
        except (json.JSONDecodeError, OSError):
            pass

    success_count = 0
    cached_count = 0
    skipped_count = 0
    error_count = 0
    t_start = time.monotonic()

    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        futures = {}
        iterator = iter(stratified)

        # Helper to submit next item
        def submit_next():
            nonlocal current_size
            if current_size >= max_budget_bytes:
                return False
            try:
                vpath, cat = next(iterator)
                fut = executor.submit(
                    process_video_snippet,
                    vpath,
                    cat,
                    args.output_dir,
                    args.dbsrt_bin,
                    args.duration,
                    args.crf,
                    args.scale,
                )
                futures[fut] = (vpath, cat)
                return True
            except StopIteration:
                return False

        # Fill initial workers pool
        for _ in range(args.workers * 2):
            if not submit_next():
                break

        idx = 0
        while futures:
            for fut in as_completed(list(futures.keys())):
                del futures[fut]
                idx += 1
                try:
                    res = fut.result()
                    status = res.get("status")
                    vname = res.get("video", "unknown")
                    if status in ("success", "cached"):
                        if status == "success":
                            success_count += 1
                        else:
                            cached_count += 1
                        manifest[vname] = res
                    elif "skipped" in str(status):
                        skipped_count += 1
                    else:
                        error_count += 1

                    # Check current size
                    if idx % 10 == 0 or status == "success":
                        current_size = get_dir_size(args.output_dir)

                    if idx % 25 == 0 or idx == total_found:
                        mb_used = current_size / (1024 * 1024)
                        pct = (current_size / max_budget_bytes) * 100.0
                        elapsed = time.monotonic() - t_start
                        print(
                            f"[{idx:4d}] Success: {success_count} | Cached: {cached_count} | "
                            f"Errors: {error_count} | Disk: {mb_used:.1f} MB ({pct:.1f}%) | "
                            f"Elapsed: {elapsed:.1f}s"
                        )

                    if current_size >= max_budget_bytes:
                        print(
                            f"\n⚠️  Budget limit reached: {current_size / (1024 * 1024):.1f} MB >= {args.max_budget_mb} MB"
                        )
                        executor.shutdown(wait=False, cancel_futures=True)
                        break

                except Exception as e:
                    error_count += 1
                    print(f"⚠️ Worker error: {e}", file=sys.stderr)

                # Submit next item if budget permits
                submit_next()
                break

    # Save manifest
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, ensure_ascii=False)

    total_time = time.monotonic() - t_start
    final_size = get_dir_size(args.output_dir)
    print("\n================================================================================")
    print(" 🎉 NAS SNIPPET EXTRACTION SUMMARY")
    print("================================================================================")
    print(f"Total Processed:    {idx}")
    print(f"Newly Extracted:    {success_count}")
    print(f"Already Cached:     {cached_count}")
    print(f"Skipped / Errors:   {skipped_count} / {error_count}")
    print(f"Final Disk Footprint: {final_size / (1024 * 1024):.1f} MB / {args.max_budget_mb} MB")
    print(f"Manifest Saved:     {manifest_path}")
    print(f"Total Elapsed Time: {total_time:.2f} s")
    print("================================================================================")

    return 0


if __name__ == "__main__":
    sys.exit(main())
