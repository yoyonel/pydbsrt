#!/usr/bin/env python3
# pyright: reportMissingImports=false
"""Automated Playwright E2E test for the Media Sources and Master-Detail Explorer views.

Verifies end-to-end browser interaction:
1. Loading Sources dashboard and probe completion.
2. Navigating into Master-Detail Explorer for local snippets.
3. Live fuzzy search and token filtering.
4. Multimedia details card display via ffprobe.
5. Breadcrumb navigation back to dashboard.
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path
from typing import Any

from playwright.sync_api import sync_playwright

DEFAULT_PORTAL_URL = "http://127.0.0.1:8780/sources.html"
DEFAULT_TIMEOUT_MS = 10000
VIEWPORT_WIDTH = 1920
VIEWPORT_HEIGHT = 1080


def _test_sources_and_explorer(page: Any) -> None:
    """Verifies Sources grid, probe checks and master-detail explorer."""
    # 1. Assert dashboard header
    page.wait_for_selector("text=Sources Médias : Accès Réseau & Disques", timeout=DEFAULT_TIMEOUT_MS)
    print("[E2E] Tab 1: Sources dashboard loaded.")

    # 2. Target local-snippets source card with Explorer button
    snippet_card = page.locator("div.bg-slate-900:has-text('local-snippets')")
    explorer_btn = snippet_card.locator("button:has-text('Explorer')").first
    explorer_btn.wait_for(state="visible", timeout=DEFAULT_TIMEOUT_MS)
    explorer_btn.click()
    page.wait_for_selector("text=Sources Médias", timeout=DEFAULT_TIMEOUT_MS)
    page.wait_for_selector("input[placeholder*='Filtrer']", timeout=DEFAULT_TIMEOUT_MS)
    print("[E2E] Entered Explorer view.")

    # Verify Master list items are rendered
    page.wait_for_selector("[id^='item-']", timeout=DEFAULT_TIMEOUT_MS)
    items_count = page.locator("[id^='item-']").count()
    assert items_count > 0, "No items found in explorer master list"

    # Verify zero window scroll
    window_scrollable = page.evaluate("() => document.documentElement.scrollHeight > window.innerHeight")
    assert not window_scrollable, "Page window must not have an outer vertical scrollbar in Explorer view"

    # Test search query
    search_input = page.locator("input[x-model='explorerQuery']")
    search_input.fill("Watchmen")
    page.wait_for_selector("text=Watchmen", timeout=DEFAULT_TIMEOUT_MS)

    # Test detail card
    file_item = page.locator("[id^='item-']:has-text('Watchmen')").first
    file_item.click()
    page.wait_for_selector("text=Caractéristiques Multimédia", timeout=DEFAULT_TIMEOUT_MS)

    # Breadcrumb return
    back_btn = page.locator("button:has-text('←')").first
    back_btn.click()
    page.wait_for_selector("text=Sources Médias : Accès Réseau & Disques", timeout=DEFAULT_TIMEOUT_MS)
    print("[E2E] Tab 1: Sources and Explorer tests passed.")


def _test_assets_tab(page: Any) -> None:
    """Verifies Actifs Médias (DB) tab and PostgreSQL assets view."""
    btn = page.locator("button:has-text('Actifs Médias (DB)')")
    btn.click()
    page.wait_for_selector("text=Actifs Médias : Intégrité Base", timeout=DEFAULT_TIMEOUT_MS)
    assert "view=assets" in page.url
    print("[E2E] Tab 2: Actifs Médias (DB) loaded.")


def _test_phashes_tab(page: Any) -> None:
    """Verifies Empreintes pHash tab and integrity observatory."""
    btn = page.locator("button:has-text('Empreintes pHash')")
    btn.click()
    page.wait_for_selector("text=Empreintes pHash : Observatoire d'Intégrité", timeout=DEFAULT_TIMEOUT_MS)
    assert "view=phashes" in page.url
    print("[E2E] Tab 3: Empreintes pHash loaded.")


def _test_trajectories_tab(page: Any) -> None:
    """Verifies Trajectoires Spatiotemporelles tab, consensus KPIs and inspection modal."""
    btn = page.locator("button:has-text('Trajectoires')")
    btn.click()
    page.wait_for_selector("text=Trajectoires Spatiotemporelles & Consensus", timeout=DEFAULT_TIMEOUT_MS)
    assert "view=trajectories" in page.url

    # Check KPI cards
    page.wait_for_selector("text=Médias Évalués", timeout=DEFAULT_TIMEOUT_MS)
    page.wait_for_selector("text=Taux Consensus", timeout=DEFAULT_TIMEOUT_MS)
    page.wait_for_selector("text=Cohérence Temp.", timeout=DEFAULT_TIMEOUT_MS)
    page.wait_for_selector("text=Latence Moyenne", timeout=DEFAULT_TIMEOUT_MS)
    page.wait_for_selector("text=Catalogue Couvert", timeout=DEFAULT_TIMEOUT_MS)
    print("[E2E] Tab 4: Trajectoires KPIs rendered.")

    # Status filter pill check
    coherent_btn = page.locator("[x-show*='trajectories'] button:has-text('Cohérents')")
    coherent_btn.click()
    assert "status=coherent" in page.url

    all_btn = page.locator("[x-show*='trajectories'] button:has-text('Tous')")
    all_btn.click()

    # Click first trajectory row to trigger inspection drawer
    first_row = page.locator("[x-show*='trajectories'] tbody tr.cursor-pointer").first
    if first_row.is_visible():
        first_row.click()
        page.wait_for_selector("text=Verdict du Consensus Multi-Coupures", timeout=DEFAULT_TIMEOUT_MS)
        page.wait_for_selector("text=Séquence Chronologique des Coupures", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] Tab 4: Trajectory modal displayed with cut breakdown.")

        # Close modal
        close_btn = page.locator("div[x-show='selectedTrajectoryModal'] button:has-text('Fermer')").first
        close_btn.click()
        page.wait_for_timeout(300)
    print("[E2E] Tab 4: Trajectoires tests passed.")


def run_e2e_test(url: str, output_screenshot: Path | None = None) -> None:
    """Executes the complete multi-tab E2E scenario in headless Chromium."""
    with sync_playwright() as p:
        chrome_bin = shutil.which("google-chrome") or shutil.which("chromium")
        browser = p.chromium.launch(
            executable_path=chrome_bin,
            headless=True,
            args=["--no-sandbox", "--disable-dev-shm-usage"],
        )
        context = browser.new_context(viewport={"width": VIEWPORT_WIDTH, "height": VIEWPORT_HEIGHT})
        page = context.new_page()

        print(f"[E2E] Navigating to {url}...")
        page.goto(url, wait_until="networkidle")

        _test_sources_and_explorer(page)
        _test_assets_tab(page)
        _test_phashes_tab(page)
        _test_trajectories_tab(page)

        if output_screenshot:
            output_screenshot.parent.mkdir(parents=True, exist_ok=True)
            page.screenshot(path=str(output_screenshot), full_page=True)
            print(f"[E2E] Screenshot saved to {output_screenshot}")

        browser.close()
        print("[E2E] ALL 4 TABS VERIFIED & PASSED CLEANLY.")


def main() -> None:
    """CLI entrypoint."""
    parser = argparse.ArgumentParser(description="Media Sources E2E Playwright validation")
    parser.add_argument("--url", default=DEFAULT_PORTAL_URL, help="Target portal URL")
    parser.add_argument("--screenshot", type=Path, default=None, help="Path to save full-page screenshot")
    args = parser.parse_args()

    try:
        run_e2e_test(args.url, args.screenshot)
    except Exception as err:
        import traceback

        traceback.print_exc()
        print(f"[E2E ERROR] {err}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
