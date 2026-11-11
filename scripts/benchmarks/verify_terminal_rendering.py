#!/usr/bin/env python3
"""Formal Browser Verification for Asciinema Terminal Recordings in MkDocs.

Spins up a lightweight static HTTP server on site/, launches headless Google Chrome,
asserts that AsciinemaPlayer mounts and renders terminal DOM nodes (.ap-player, .ap-terminal),
and captures high-resolution screenshots for visual verification.
"""

from __future__ import annotations

import argparse
import shutil
import socketserver
import subprocess
import threading
import time
from http.server import SimpleHTTPRequestHandler
from pathlib import Path
from typing import List, Tuple

from PIL import Image


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


REPO_ROOT: Path = find_repo_root()
SITE_DIR: Path = REPO_ROOT / "site"
VERIFY_PORT: int = 8019
CHROME_TIMEOUT_SEC: int = 25
CHROME_WINDOW_WIDTH: int = 1920
CHROME_WINDOW_HEIGHT: int = 1400

CHROME_CANDIDATES: tuple[str, ...] = (
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
)


def find_chrome_binary() -> str:
    """Locate the first available Chrome/Chromium executable binary on PATH."""
    for candidate in CHROME_CANDIDATES:
        if shutil.which(candidate):
            return candidate
    raise RuntimeError(
        "No Chrome/Chromium binary found for terminal verification. "
        f"Probed candidates: {', '.join(CHROME_CANDIDATES)}. "
        "Install google-chrome or chromium on your system."
    )


VERIFY_PAGES: List[Tuple[str, str, str]] = [
    (
        "How-To: Ingestion Média",
        "/how-to/ajouter-et-indexer-un-media/",
        "docs/assets/screenshots/cast_verify_media_add.webp",
    ),
    (
        "Tutorial: Sync VLC & Statut",
        "/tutorials/01-premiers-pas-synchronisation-vlc/",
        "docs/assets/screenshots/cast_verify_status.webp",
    ),
    (
        "Reference: CLI dbsrt",
        "/reference/cli/",
        "docs/assets/screenshots/cast_verify_cli.webp",
    ),
    (
        "Tutorial: Doc Check & Contribution",
        "/tutorials/03-environnement-developpement-et-contribution/",
        "docs/assets/screenshots/cast_verify_doc_check.webp",
    ),
]


class SilentHandler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(SITE_DIR), **kwargs)

    def log_message(self, format, *args):
        pass  # Quiet logs


def start_test_server(port: int) -> socketserver.TCPServer:
    socketserver.TCPServer.allow_reuse_address = True
    httpd = socketserver.TCPServer(("127.0.0.1", port), SilentHandler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    return httpd


def run_chrome_dom_check(url: str) -> str:
    """Run headless Chrome with virtual time budget to allow fetch + JS DOM hydration."""
    chrome_bin = find_chrome_binary()
    cmd = [
        chrome_bin,
        "--headless=new",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--disable-gpu",
        f"--window-size={CHROME_WINDOW_WIDTH},{CHROME_WINDOW_HEIGHT}",
        "--virtual-time-budget=2000",
        "--dump-dom",
        url,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=CHROME_TIMEOUT_SEC, check=True)
    return proc.stdout


def run_chrome_screenshot(url: str, out_png: Path) -> None:
    """Capture a crisp screenshot with virtual time budget."""
    chrome_bin = find_chrome_binary()
    cmd = [
        chrome_bin,
        "--headless=new",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--disable-gpu",
        "--hide-scrollbars",
        f"--window-size={CHROME_WINDOW_WIDTH},{CHROME_WINDOW_HEIGHT}",
        "--virtual-time-budget=2000",
        f"--screenshot={out_png}",
        url,
    ]
    subprocess.run(cmd, capture_output=True, text=True, timeout=CHROME_TIMEOUT_SEC, check=True)


def perform_check(site_dir: Path) -> None:
    """Performs sanity checks of paths, tools and pages."""
    print("[CHECK] Mode verification activé.")
    print(f"[CHECK] REPO_ROOT : {REPO_ROOT}")
    print(f"[CHECK] site/ dir : {site_dir} (exists: {site_dir.is_dir()})")
    for cand in CHROME_CANDIDATES:
        path = shutil.which(cand)
        if path:
            print(f"[CHECK] Chrome candidate '{cand}' found: {path}")
            break
    else:
        print("[CHECK] No Chrome candidate currently on PATH.")
    print(f"[CHECK] Pages to verify : {len(VERIFY_PAGES)}")


def verify_page_dom(url: str) -> bool:
    """Verifies that Asciinema DOM elements are present and hydrated."""
    try:
        dom = run_chrome_dom_check(url)
    except Exception as e:
        print(f"  ❌ Failed to fetch page DOM via Headless Chrome: {e}")
        return False

    has_wrapper = "asciinema-wrapper" in dom
    has_player_dom = "ap-player" in dom
    has_terminal = "ap-terminal" in dom

    if not has_wrapper:
        print("  ❌ Missing .asciinema-wrapper in page HTML")
        return False
    if not (has_player_dom and has_terminal):
        print("  ❌ AsciinemaPlayer failed to mount in DOM (.ap-player / .ap-terminal missing)")
        return False

    print("  ✅ AsciinemaPlayer mounted & hydrated (.ap-player & .ap-terminal found in DOM)")
    return True


def verify_page_screenshot(url: str, out_webp: Path, tmp_png: Path) -> bool:
    """Captures screenshot and converts to WEBP."""
    try:
        run_chrome_screenshot(url, tmp_png)
        if tmp_png.exists() and tmp_png.stat().st_size > 50000:
            out_webp.parent.mkdir(parents=True, exist_ok=True)
            with Image.open(tmp_png) as im:
                im.save(out_webp, "WEBP", quality=85)
            tmp_png.unlink()
            print(f"  📸 Screenshot verified: {out_webp} ({out_webp.stat().st_size // 1024} KB)")
            return True
        print("  ❌ Screenshot capture produced an empty or tiny file")
        return False
    except Exception as e:
        print(f"  ❌ Screenshot capture failed: {e}")
        return False


def main() -> int:
    parser = argparse.ArgumentParser(description="Formal Browser Verification for Asciinema Terminal Recordings.")
    parser.add_argument("--site-dir", type=Path, default=SITE_DIR)
    parser.add_argument("--port", type=int, default=VERIFY_PORT)
    parser.add_argument(
        "--check", "--dry-run", action="store_true", help="Perform sanity checks without launching Chrome or server"
    )
    args = parser.parse_args()

    if args.check:
        perform_check(args.site_dir)
        return 0

    if not args.site_dir.exists():
        print(f"❌ Error: {args.site_dir} directory does not exist. Run task doc:build first.")
        return 1

    try:
        chrome_bin = find_chrome_binary()
        print(f"🌐 Found Chrome/Chromium executable: {chrome_bin}")
    except RuntimeError as e:
        print(f"❌ {e}")
        return 1

    print(f"🚀 Starting static test server on http://127.0.0.1:{args.port}...")
    server = start_test_server(args.port)
    time.sleep(0.5)

    all_passed = True
    tmp_png = Path("/tmp/asciinema_verify.png")

    try:
        for title, path, out_webp_rel in VERIFY_PAGES:
            url = f"http://127.0.0.1:{args.port}{path}"
            out_webp = REPO_ROOT / out_webp_rel
            print(f"\n🔍 Verifying page: {title} ({path})")

            if not verify_page_dom(url):
                all_passed = False
                continue

            if not verify_page_screenshot(url, out_webp, tmp_png):
                all_passed = False

    finally:
        server.shutdown()
        server.server_close()
        if tmp_png.exists():
            tmp_png.unlink()

    if all_passed:
        print("\n🎉 Formal Verification Succeeded: All Asciinema players functional and verified in browser!")
        return 0
    else:
        print("\n❌ Formal Verification Failed: One or more Asciinema terminal players failed to render.")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
