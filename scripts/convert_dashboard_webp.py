#!/usr/bin/env python3
"""Batch converts PNG screenshots in docs/screenshots/e2e_vlc_all/ to 720p WebP

and updates manifest.json and the dashboard.
"""

import json
import os
import shutil
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path


def convert_one(png_path: Path, webp_path: Path) -> tuple[Path, int, int]:
    """Converts a single PNG to 720p WebP and removes the PNG on success."""
    cmd = [
        "cwebp",
        "-quiet",
        "-q",
        "80",
        "-resize",
        "1280",
        "720",
        str(png_path),
        "-o",
        str(webp_path),
    ]
    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"cwebp failed for {png_path}: {res.stderr}")

    orig_size = png_path.stat().st_size
    new_size = webp_path.stat().st_size
    png_path.unlink()
    return png_path, orig_size, new_size


def main():
    if not shutil.which("cwebp"):
        print("Error: 'cwebp' executable not found in PATH.", file=sys.stderr)
        sys.exit(1)

    target_dir = Path("docs/screenshots/e2e_vlc_all")
    if not target_dir.is_dir():
        print(f"Error: {target_dir} does not exist.", file=sys.stderr)
        sys.exit(1)

    png_files = sorted(target_dir.glob("*.png"))
    print(f"Found {len(png_files)} PNG files to convert to 720p WebP...")

    if not png_files:
        print("No PNG files found to convert.")
    else:
        total_orig = 0
        total_new = 0
        workers = max(1, os.cpu_count() or 4)

        with ProcessPoolExecutor(max_workers=workers) as executor:
            future_to_file = {
                executor.submit(convert_one, png_path, png_path.with_suffix(".webp")): png_path
                for png_path in png_files
            }

            done = 0
            for future in as_completed(future_to_file):
                _, orig_s, new_s = future.result()
                total_orig += orig_s
                total_new += new_s
                done += 1
                if done % 100 == 0 or done == len(png_files):
                    pct = (total_new / total_orig * 100.0) if total_orig else 0
                    print(
                        f"Converted {done}/{len(png_files)}: {total_orig / (1024 * 1024):.1f} MB -> {total_new / (1024 * 1024):.1f} MB ({pct:.1f}%)"
                    )

    manifest_path = target_dir / "manifest.json"
    if manifest_path.exists():
        print(f"Updating {manifest_path} references from .png to .webp...")
        with open(manifest_path, encoding="utf-8") as f:
            items = json.load(f)

        updated_count = 0
        for item in items:
            for key in ("screenshot", "screenshot_rel"):
                if key in item and item[key].endswith(".png"):
                    item[key] = item[key][:-4] + ".webp"
                    updated_count += 1

        with open(manifest_path, "w", encoding="utf-8") as f:
            json.dump(items, f, indent=2, ensure_ascii=False)
        print(f"Updated {updated_count} manifest references.")

    dashboard_builder = Path("scripts/build_dashboard_html.py")
    if dashboard_builder.exists():
        print("Re-generating interactive E2E dashboard...")
        subprocess.run([sys.executable, str(dashboard_builder)], check=True)

    print("WebP 720p optimization complete.")


if __name__ == "__main__":
    main()
