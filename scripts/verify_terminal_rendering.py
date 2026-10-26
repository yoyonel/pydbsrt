#!/usr/bin/env python3
"""Formal Browser Verification for Asciinema Terminal Recordings in MkDocs.

Spins up a lightweight static HTTP server on site/, launches headless Google Chrome,
asserts that AsciinemaPlayer mounts and renders terminal DOM nodes (.ap-player, .ap-terminal),
and captures high-resolution screenshots for visual verification.
"""

from __future__ import annotations

import socketserver
import subprocess
import threading
import time
from http.server import SimpleHTTPRequestHandler
from pathlib import Path
from typing import List, Tuple

from PIL import Image

SITE_DIR: Path = Path("site")
VERIFY_PORT: int = 8019
CHROME_TIMEOUT_SEC: int = 25
CHROME_WINDOW_WIDTH: int = 1920
CHROME_WINDOW_HEIGHT: int = 1400

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
    cmd = [
        "google-chrome",
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
    cmd = [
        "google-chrome",
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


def main() -> int:
    if not SITE_DIR.exists():
        print("❌ Error: site/ directory does not exist. Run task doc:build first.")
        return 1

    print(f"🚀 Starting static test server on http://127.0.0.1:{VERIFY_PORT}...")
    server = start_test_server(VERIFY_PORT)
    time.sleep(0.5)

    all_passed = True
    tmp_png = Path("/tmp/asciinema_verify.png")

    try:
        for title, path, out_webp_rel in VERIFY_PAGES:
            url = f"http://127.0.0.1:{VERIFY_PORT}{path}"
            print(f"\n🔍 Verifying page: {title} ({path})")

            # 1. DOM Hydration Check
            try:
                dom = run_chrome_dom_check(url)
            except Exception as e:
                print(f"  ❌ Failed to fetch page DOM via Headless Chrome: {e}")
                all_passed = False
                continue

            has_wrapper = "asciinema-wrapper" in dom
            has_player_dom = "ap-player" in dom
            has_terminal = "ap-terminal" in dom

            if not has_wrapper:
                print("  ❌ Missing .asciinema-wrapper in page HTML")
                all_passed = False
            elif not (has_player_dom and has_terminal):
                print("  ❌ AsciinemaPlayer failed to mount in DOM (.ap-player / .ap-terminal missing)")
                all_passed = False
            else:
                print("  ✅ AsciinemaPlayer mounted & hydrated (.ap-player & .ap-terminal found in DOM)")

            # 2. Screenshot Capture Check
            try:
                run_chrome_screenshot(url, tmp_png)
                if tmp_png.exists() and tmp_png.stat().st_size > 50000:
                    out_webp = Path(out_webp_rel)
                    out_webp.parent.mkdir(parents=True, exist_ok=True)
                    with Image.open(tmp_png) as im:
                        im.save(out_webp, "WEBP", quality=85)
                    tmp_png.unlink()
                    print(f"  📸 Screenshot verified: {out_webp} ({out_webp.stat().st_size // 1024} KB)")
                else:
                    print("  ❌ Screenshot capture produced an empty or tiny file")
                    all_passed = False
            except Exception as e:
                print(f"  ❌ Screenshot capture failed: {e}")
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
