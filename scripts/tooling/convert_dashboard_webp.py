#!/usr/bin/env python3
"""Batch converts PNG screenshots in docs/screenshots/e2e_vlc_all/ to 720p WebP.

Also updates manifest.json and regenerates the dashboard.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

DEFAULT_SCREENSHOTS_DIR = "docs/screenshots/e2e_vlc_all"
WEBP_QUALITY = "80"
TARGET_WIDTH = "1280"
TARGET_HEIGHT = "720"
BYTES_PER_MB = 1024 * 1024
DEFAULT_MAX_WORKERS_FALLBACK = 4


def find_repo_root() -> Path:
    """Dynamically locates repository root with fallback to ~/dbsrt."""
    current = Path(__file__).resolve().parent
    for p in [current, *current.parents]:
        if (p / ".git").is_dir() or (p / "Taskfile.yml").is_file():
            return p
    appliance = Path.home() / "dbsrt"
    if appliance.is_dir():
        return appliance
    return Path.cwd()


def convert_one(png_path: Path, webp_path: Path) -> tuple[Path, int, int]:
    """Converts a single PNG to 720p WebP and removes the PNG on success."""
    cmd = [
        "cwebp",
        "-quiet",
        "-q",
        WEBP_QUALITY,
        "-resize",
        TARGET_WIDTH,
        TARGET_HEIGHT,
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


def run_conversion_pool(png_files: list[Path]) -> tuple[int, int]:
    """Executes parallel conversion of PNG files to WebP."""
    if not png_files:
        print("No PNG files found to convert.")
        return 0, 0

    total_orig = 0
    total_new = 0
    workers = max(1, os.cpu_count() or DEFAULT_MAX_WORKERS_FALLBACK)

    with ProcessPoolExecutor(max_workers=workers) as executor:
        future_to_file = {
            executor.submit(convert_one, png_path, png_path.with_suffix(".webp")): png_path for png_path in png_files
        }

        done = 0
        for future in as_completed(future_to_file):
            _, orig_s, new_s = future.result()
            total_orig += orig_s
            total_new += new_s
            done += 1
            if done % 100 == 0 or done == len(png_files):
                pct = (total_new / total_orig * 100.0) if total_orig else 0.0
                print(
                    f"Converted {done}/{len(png_files)}: "
                    f"{total_orig / BYTES_PER_MB:.1f} MB -> {total_new / BYTES_PER_MB:.1f} MB ({pct:.1f}%)"
                )
    return total_orig, total_new


def update_manifest_references(manifest_path: Path) -> int:
    """Updates .png file references to .webp inside manifest.json."""
    if not manifest_path.is_file():
        return 0

    print(f"Updating {manifest_path} references from .png to .webp...")
    with open(manifest_path, encoding="utf-8") as f:
        items = json.load(f)

    updated_count = 0
    for item in items:
        for key in ("screenshot", "screenshot_rel"):
            val = item.get(key)
            if isinstance(val, str) and val.endswith(".png"):
                item[key] = val[:-4] + ".webp"
                updated_count += 1

    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(items, f, indent=2, ensure_ascii=False)
    print(f"Updated {updated_count} manifest references.")
    return updated_count


def rebuild_dashboard(repo_root: Path) -> None:
    """Re-executes build_dashboard_html.py to refresh the markdown dashboard."""
    builder = repo_root / "scripts/tooling/build_dashboard_html.py"
    if builder.is_file():
        print("Re-generating interactive E2E dashboard...")
        subprocess.run([sys.executable, str(builder)], check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description="Batch converts PNG screenshots in docs/screenshots/ to 720p WebP.")
    parser.add_argument(
        "--dir",
        type=str,
        default=None,
        help=f"Target directory containing screenshots (default: {DEFAULT_SCREENSHOTS_DIR}).",
    )
    parser.add_argument(
        "--no-rebuild",
        action="store_true",
        help="Skip rebuilding the dashboard HTML.",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Check tool availability and file counts without converting.",
    )
    args = parser.parse_args()

    repo_root = find_repo_root()
    cwebp_path = shutil.which("cwebp")
    if not cwebp_path:
        print("Error: 'cwebp' executable not found in PATH.", file=sys.stderr)
        sys.exit(1)

    if args.dir:
        target_dir = Path(args.dir)
        if not target_dir.is_absolute():
            target_dir = repo_root / target_dir
    else:
        target_dir = repo_root / DEFAULT_SCREENSHOTS_DIR

    if not target_dir.is_dir():
        print(f"Error: {target_dir} does not exist.", file=sys.stderr)
        sys.exit(1)

    png_files = sorted(target_dir.glob("*.png"))
    webp_files = sorted(target_dir.glob("*.webp"))
    manifest_path = target_dir / "manifest.json"

    if args.check:
        print(f"cwebp binary     : {cwebp_path} (OK)")
        print(f"Target directory : {target_dir}")
        print(f"PNG count        : {len(png_files)}")
        print(f"WebP count       : {len(webp_files)}")
        print(f"Manifest exists  : {manifest_path.is_file()}")
        return

    print(f"Found {len(png_files)} PNG files to convert to 720p WebP...")
    run_conversion_pool(png_files)
    update_manifest_references(manifest_path)

    if not args.no_rebuild:
        rebuild_dashboard(repo_root)

    print("WebP 720p optimization complete.")


if __name__ == "__main__":
    main()
