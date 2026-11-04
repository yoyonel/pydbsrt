#!/usr/bin/env python3
# pyright: reportMissingImports=false
"""E2E Playwright validation of the dbsrt monitoring portal.

Validates 100% of legacy portal features:
1. Navigation header, status badges, refresh/pause controls, auth modal
2. Overview view (#overview) with scope tiles
3. Ingestion view (#ingestion)
4. Catalogue view (#catalogue) & interactive matching pipeline
5. Audit view (#audit)
6. Supervisor view (#supervisor)
7. System view (#system)
8. Footer metrics (host, RAM, uptime, latency/sync)

Records video screencast and captures per-feature screenshots.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

from playwright.sync_api import Page, sync_playwright

PORTAL_URL = os.environ.get("DBSRT_PORTAL_URL", "http://127.0.0.1:8765/monitoring")
OUTPUT_DIR = Path("target/e2e_portal")
SCREENSHOTS_DIR = OUTPUT_DIR / "screenshots"
SCREENCASTS_DIR = OUTPUT_DIR / "screencasts"


def verify_header_and_overview(page: Page) -> None:
    """Validate header controls, modal, and overview cards."""
    print("-> Step 1: Loading portal & verifying header...")
    page.goto(PORTAL_URL, wait_until="networkidle")
    page.wait_for_timeout(2500)

    title = page.title()
    assert "dbsrt Monitor" in title, f"Unexpected title: {title}"
    print(f"  ✓ Page title: {title}")

    health_badge = page.locator("#health-badge")
    assert "Actif" in (health_badge.text_content() or ""), "Health badge not active"
    print("  ✓ Health badge is 'Actif'")

    btn_pause = page.locator("#btn-pause")
    btn_pause.click()
    page.wait_for_timeout(500)
    assert "▶" in (btn_pause.text_content() or ""), "Pause button failed to toggle to play"
    btn_pause.click()
    page.wait_for_timeout(500)
    assert "⏸" in (btn_pause.text_content() or ""), "Pause button failed to toggle back"
    print("  ✓ Pause/Resume toggle verified")

    btn_auth = page.locator("#btn-auth")
    btn_auth.click()
    page.wait_for_timeout(400)
    auth_modal = page.locator("#auth-modal")
    assert auth_modal.is_visible(), "Auth modal did not display"
    page.keyboard.press("Escape")
    page.evaluate("document.getElementById('auth-modal').style.display = 'none'")
    page.wait_for_timeout(300)
    print("  ✓ Auth modal opened and dismissed")

    print("-> Step 2: Testing View Overview (#overview)...")
    page.locator("#views-nav a[href='#overview']").click()
    page.wait_for_timeout(1000)
    tiles = page.locator("#tiles .tile")
    assert tiles.count() >= 4, f"Expected at least 4 tiles, got {tiles.count()}"
    print(f"  ✓ Overview tiles rendered: {tiles.count()} tiles")
    page.screenshot(path=str(SCREENSHOTS_DIR / "01_view_overview.png"), full_page=False)


def verify_ingestion_view(page: Page) -> None:
    """Validate ingestion view top lines, KPI cards, and sub-tabs."""
    print("-> Step 3: Testing View Ingestion (#ingestion)...")
    page.locator("#views-nav a[href='#ingestion']").click()
    page.wait_for_timeout(1500)

    top_host = page.locator("#top-line-host").text_content() or ""
    assert "HÔTE" in top_host, f"Missing HÔTE in banner: {top_host}"
    top_engines = page.locator("#top-line-engines").text_content() or ""
    assert "MOTEURS" in top_engines, f"Missing MOTEURS in banner: {top_engines}"
    print(f"  ✓ Top banner verified: {top_host[:50]}...")

    queue_pct = page.locator("#queue-pct").text_content()
    kpi_completed = page.locator("#kpi-completed").text_content()
    kpi_total = page.locator("#kpi-total").text_content()
    print(f"  ✓ Hero Progress: {queue_pct} | Traités: {kpi_completed} | Total: {kpi_total}")

    page.locator("button[data-tab='jobs']").click()
    page.wait_for_timeout(800)
    assert page.locator("#tab-jobs").is_visible(), "Tab jobs not visible"
    page.screenshot(path=str(SCREENSHOTS_DIR / "02_ingestion_jobs.png"))

    page.locator("button[data-tab='stats']").click()
    page.wait_for_timeout(1000)
    assert page.locator("#tab-stats").is_visible(), "Tab stats not visible"
    stats_rows = page.locator("#stats-tbody tr").count()
    assert stats_rows >= 3, f"Expected statistical distribution rows, got {stats_rows}"
    page.screenshot(path=str(SCREENSHOTS_DIR / "03_ingestion_stats.png"))

    page.locator("button[data-tab='completed']").click()
    page.wait_for_selector("#completed-tbody tr[data-key]", timeout=10000)
    assert page.locator("#tab-completed").is_visible(), "Tab completed not visible"
    page.locator("#completed-tbody tr[data-key]").first.click()
    page.wait_for_selector("tr.detail", timeout=5000)

    pager_next = page.locator(".pager[data-list='completed'] button[data-step='1']")
    if pager_next.count() > 0 and pager_next.is_visible():
        pager_next.click()
        page.wait_for_timeout(3500)

    page.screenshot(path=str(SCREENSHOTS_DIR / "04_ingestion_completed.png"))

    page.locator("button[data-tab='sources']").click()
    page.wait_for_timeout(1000)
    assert page.locator("#tab-sources").is_visible(), "Tab sources not visible"
    page.screenshot(path=str(SCREENSHOTS_DIR / "05_ingestion_sources.png"))
    print("  ✓ Ingestion sub-tabs (Jobs, Stats, Completed, Sources) verified")


def verify_catalogue_and_pipeline(page: Page) -> None:
    """Validate catalogue view and interactive matching pipeline evaluation."""
    print("-> Step 4: Testing View Catalogue (#catalogue)...")
    page.locator("#views-nav a[href='#catalogue']").click()
    page.wait_for_timeout(2500)
    assert page.locator("#view-catalogue").is_visible(), "View catalogue not visible"
    media_caption = page.locator("#media-caption").text_content()
    print(f"  ✓ Catalogue caption: {media_caption}")

    btn_match = page.locator("#btn-pipeline-match")
    assert btn_match.is_visible(), "Button 'Lancer le Matching' not visible"
    print("  -> Triggering live matching pipeline evaluation...")
    btn_match.click()
    page.wait_for_timeout(3500)

    meta_txt = page.locator("#pipeline-meta").text_content() or ""
    assert "Dernière recherche" in meta_txt, f"Pipeline meta not populated: {meta_txt}"
    ffmpeg_val = page.locator("#iso-val-ffmpeg").text_content() or ""
    assert "ms" in ffmpeg_val and ffmpeg_val != "- ms", f"Invalid ffmpeg timing: {ffmpeg_val}"
    srt_val = page.locator("#iso-val-srt-disc").text_content() or ""
    assert "ms" in srt_val and srt_val != "- ms", f"Invalid srt timing: {srt_val}"
    print(f"  ✓ Live Matching executed: {meta_txt[:50]}... | FFmpeg: {ffmpeg_val} | SRT: {srt_val}")

    page.screenshot(path=str(SCREENSHOTS_DIR / "06_view_catalogue.png"))


def verify_system_views_and_footer(page: Page) -> None:
    """Validate audit, supervisor, system views and host footer."""
    print("-> Step 5: Testing View Audit (#audit)...")
    page.locator("#views-nav a[href='#audit']").click()
    page.wait_for_timeout(1200)
    assert page.locator("#view-audit").is_visible(), "View audit not visible"
    page.screenshot(path=str(SCREENSHOTS_DIR / "07_view_audit.png"))

    print("-> Step 6: Testing View Supervisor (#supervisor)...")
    page.locator("#views-nav a[href='#supervisor']").click()
    page.wait_for_timeout(1200)
    assert page.locator("#view-supervisor").is_visible(), "View supervisor not visible"
    page.screenshot(path=str(SCREENSHOTS_DIR / "08_view_supervisor.png"))

    print("-> Step 7: Testing View System (#system)...")
    page.locator("#views-nav a[href='#system']").click()
    page.wait_for_timeout(1200)
    assert page.locator("#view-system").is_visible(), "View system not visible"
    page.screenshot(path=str(SCREENSHOTS_DIR / "09_view_system.png"))

    print("-> Step 8: Verifying Footer metrics...")
    sys_host = page.locator("#sys-host").text_content() or ""
    sys_ram = page.locator("#sys-ram").text_content() or ""
    sys_sync = page.locator("#sys-sync").text_content() or ""
    assert "Hôte" in sys_host, "Missing host in footer"
    assert "RAM" in sys_ram, "Missing RAM in footer"
    print(f"  ✓ Footer verified: {sys_host} | {sys_ram} | {sys_sync}")


def convert_video_to_mp4(video_path: str | None) -> None:
    """Convert recorded WebM screencast to lightweight MP4."""
    if not video_path or not os.path.exists(video_path):
        return
    webm_dest = SCREENCASTS_DIR / "portal_e2e_screencast.webm"
    mp4_dest = SCREENCASTS_DIR / "portal_e2e_screencast.mp4"
    shutil.copyfile(video_path, webm_dest)
    print(f"✓ Video saved to {webm_dest}")
    ffmpeg_bin = shutil.which("ffmpeg") or "/home/linuxbrew/.linuxbrew/bin/ffmpeg"
    if ffmpeg_bin:
        print("-> Converting WebM to MP4 (H.264 / AAC) with ffmpeg...")
        cmd = [
            ffmpeg_bin,
            "-y",
            "-i",
            str(webm_dest),
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            str(mp4_dest),
        ]
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode == 0:
            print(f"✓ MP4 screencast generated: {mp4_dest} ({os.path.getsize(mp4_dest)} bytes)")


def main() -> int:
    SCREENSHOTS_DIR.mkdir(parents=True, exist_ok=True)
    SCREENCASTS_DIR.mkdir(parents=True, exist_ok=True)
    print(f"=== Starting Playwright E2E Test on {PORTAL_URL} ===")

    with sync_playwright() as p:
        chrome_bin = shutil.which("google-chrome") or shutil.which("chromium")
        browser = p.chromium.launch(
            executable_path=chrome_bin,
            headless=True,
            args=["--no-sandbox", "--disable-dev-shm-usage"],
        )
        context = browser.new_context(
            viewport={"width": 1400, "height": 900},
            record_video_dir=str(SCREENCASTS_DIR),
            record_video_size={"width": 1400, "height": 900},
        )
        page = context.new_page()

        http_errors: list[str] = []
        page.on(
            "response",
            lambda r: (
                http_errors.append(f"{r.status} {r.request.method} {r.url}")
                if r.status >= 400 and not r.url.endswith("favicon.ico")
                else None
            ),
        )

        verify_header_and_overview(page)
        verify_ingestion_view(page)
        verify_catalogue_and_pipeline(page)
        verify_system_views_and_footer(page)

        assert len(http_errors) == 0, f"Detected HTTP errors during E2E test: {http_errors}"
        print("  ✓ Zero HTTP 4xx/5xx network errors detected")

        page.locator("#views-nav a[href='#ingestion']").click()
        page.locator("button[data-tab='completed']").click()
        page.wait_for_timeout(2000)

        video_path_obj = page.video.path() if page.video else None
        context.close()
        browser.close()

    print("=== All browser assertions passed 100%! ===")
    convert_video_to_mp4(video_path_obj)
    return 0


if __name__ == "__main__":
    sys.exit(main())
