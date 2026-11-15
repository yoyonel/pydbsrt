#!/usr/bin/env python3
# pyright: reportMissingImports=false
"""Exhaustive Playwright E2E Recette for https://2026-pydbsrt.fr/dbsrt (All 4 Portals & 43 Routes)."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


def _bootstrap_repo_root() -> Path:
    current = Path(__file__).resolve().parent
    for candidate in [current, *current.parents]:
        if (candidate / ".git").exists() or (candidate / "Taskfile.yml").is_file():
            if str(candidate) not in sys.path:
                sys.path.insert(0, str(candidate))
            return candidate
    return Path.cwd()


REPO_ROOT = _bootstrap_repo_root()

from playwright.sync_api import sync_playwright  # noqa: E402

from scripts.e2e.recette_o2switch.harness import RecetteContext  # noqa: E402
from scripts.e2e.recette_o2switch.scenarios_gateway_and_sources import (  # noqa: E402
    run_gateway_and_sources_suite,
)
from scripts.e2e.recette_o2switch.scenarios_monitoring_and_studio import (  # noqa: E402
    run_monitoring_and_top_suite,
    run_studio_suite,
)
from scripts.e2e.recette_o2switch.scenarios_observatory import (  # noqa: E402
    run_observatory_suite,
)

DEFAULT_TARGET_URL: str = "https://2026-pydbsrt.fr/dbsrt"
PASS_SECRET_ENTRY: str = "services/pydbsrt/o2switch_api_key"
EXPECTED_VIDEO_COUNT: int = 3


def resolve_api_key(explicit_key: str | None) -> str:
    """Resolve o2switch API key from CLI arg, DBSRT_API_KEY env, or pass vault."""
    if explicit_key:
        return explicit_key.strip()
    env_key = os.environ.get("DBSRT_API_KEY", "").strip()
    if env_key:
        return env_key
    out = subprocess.check_output(["pass", "show", PASS_SECRET_ENTRY], text=True)
    return out.splitlines()[0].strip()


def prepare_output_dirs(output_dir: Path) -> tuple[Path, Path, Path]:
    """Reset output directories cleanly before starting a fresh E2E recording run."""
    if output_dir.exists():
        shutil.rmtree(output_dir)
    screenshots_dir = output_dir / "screenshots"
    screencasts_dir = output_dir / "screencasts"
    raw_video_dir = output_dir / "raw_webm"
    screenshots_dir.mkdir(parents=True, exist_ok=True)
    screencasts_dir.mkdir(parents=True, exist_ok=True)
    raw_video_dir.mkdir(parents=True, exist_ok=True)
    return screenshots_dir, screencasts_dir, raw_video_dir


def parse_args() -> argparse.Namespace:
    """Parse CLI options for target URL, API key, output path, and headed mode."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=DEFAULT_TARGET_URL, help="Target o2switch portal base URL")
    parser.add_argument("--key", default=None, help="Explicit API key (defaults to DBSRT_API_KEY or pass)")
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=REPO_ROOT / "target" / "recette_o2switch",
        help="Directory for screenshots, H.264 MP4 screencasts, and JSON manifest",
    )
    parser.add_argument("--headed", action="store_true", help="Run Chromium in headed mode")
    return parser.parse_args()


def launch_chromium_resilient(pw: Any, headed: bool) -> Any:
    """Launch Playwright Chromium, falling back to latest cached ms-playwright binary if PyPI bumped."""
    chromium = pw.chromium
    try:
        return chromium.launch(headless=not headed)
    except Exception:
        cache_root = Path.home() / ".cache" / "ms-playwright"
        candidates = sorted(cache_root.glob("chromium-*/chrome-linux64/chrome"), reverse=True)
        if candidates:
            return chromium.launch(headless=not headed, executable_path=str(candidates[0]))
        raise


def main() -> int:
    """Execute the 3 recorded E2E sessions and enforce 100% pass & 0-error gates."""
    args = parse_args()
    base_url = args.url.rstrip("/")
    api_key = resolve_api_key(args.key)
    screenshots_dir, screencasts_dir, raw_video_dir = prepare_output_dirs(args.output_dir)

    started_iso = datetime.now(UTC).isoformat()
    t0 = time.monotonic()
    print(f"=== Exhaustive Playwright E2E Recette: {base_url} ===")

    with sync_playwright() as pw:
        browser = launch_chromium_resilient(pw, args.headed)
        ctx = RecetteContext(
            browser=browser,
            base_url=base_url,
            api_key=api_key,
            output_dir=args.output_dir,
            screenshots_dir=screenshots_dir,
            screencasts_dir=screencasts_dir,
            raw_video_dir=raw_video_dir,
        )
        try:
            print("\n[Session 1/3] Portal A (Gateway Security) + Portal B (Sources & Observatoire)...")
            ctx.open_recorded_session()
            run_gateway_and_sources_suite(ctx)
            run_observatory_suite(ctx)
            ctx.close_and_save_video("01_auth_gateway_and_sources_observatory")

            print("\n[Session 2/3] Portal C (Monitoring Temps Réel & Web Top)...")
            ctx.open_recorded_session()
            run_monitoring_and_top_suite(ctx)
            ctx.close_and_save_video("02_monitoring_and_web_top")

            print("\n[Session 3/3] Portal D (DBSrt Studio Relational Backoffice)...")
            ctx.open_recorded_session()
            run_studio_suite(ctx)
            ctx.close_and_save_video("03_studio_relational_backoffice")
        finally:
            if ctx.context is not None:
                ctx.close_and_save_video("99_partial_failure_capture")
            browser.close()

    shutil.rmtree(raw_video_dir, ignore_errors=True)
    total_sec = time.monotonic() - t0
    finished_iso = datetime.now(UTC).isoformat()
    manifest_path = ctx.export_manifest(started_iso, finished_iso, total_sec)

    ep_cov = ctx.tracker.summary()
    v8_cov = ctx.v8_coverage.summary()
    png_count = len(list(screenshots_dir.glob("*.png")))
    gif_count = len(list(args.output_dir.rglob("*.gif")))

    print("\n=== Formal E2E Recette Summary ===")
    print(f"  Steps Passed      : {len(ctx.steps)} / {len(ctx.steps)} ({total_sec:.1f}s)")
    print(f"  Endpoint Coverage : {ep_cov['covered_count']} / {ep_cov['total_declared']} ({ep_cov['coverage_pct']}%)")
    print(
        f"  V8 JS Coverage    : {v8_cov['covered_functions']} / {v8_cov['total_functions']} fns "
        f"({v8_cov['function_coverage_pct']}%), bytes={v8_cov['byte_coverage_pct']}%"
    )
    print(f"  Screenshots (PNG) : {png_count} (1920x1080)")
    print(f"  Screencasts (MP4) : {len(ctx.videos)} H.264 MP4 (GIF count={gif_count})")
    print(f"  Console/JS Errors : {len(ctx.console_guard.errors)}")
    print(f"  Audit Manifest    : {manifest_path}")

    if ep_cov["uncovered"]:
        print(f"  ✗ Uncovered routes: {ep_cov['uncovered']}", file=sys.stderr)
        return 1
    if ctx.console_guard.errors:
        print(f"  ✗ Console/JS errors: {ctx.console_guard.errors}", file=sys.stderr)
        return 1
    if len(ctx.videos) != EXPECTED_VIDEO_COUNT or gif_count != 0:
        print("  ✗ Video invariant failed (expected 3 MP4s and 0 GIFs)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
