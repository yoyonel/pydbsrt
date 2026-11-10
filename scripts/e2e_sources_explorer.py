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
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

DEFAULT_PORTAL_URL = "http://127.0.0.1:8780/sources.html#assets"
DEFAULT_TIMEOUT_MS = 10000
VIEWPORT_WIDTH = 1920
VIEWPORT_HEIGHT = 1080


def run_e2e_test(url: str, output_screenshot: Path | None = None) -> None:
    """Executes the complete E2E scenario in headless Chromium."""
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(viewport={"width": VIEWPORT_WIDTH, "height": VIEWPORT_HEIGHT})
        page = context.new_page()

        print(f"[E2E] Navigating to {url}...")
        page.goto(url, wait_until="networkidle")

        # 1. Assert dashboard header
        page.wait_for_selector("text=Sources Médias : Accès Réseau & Disques", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] Dashboard loaded.")

        # 2. Wait for at least one accessible source with Explorer button
        explorer_btn = page.wait_for_selector(
            "button:has-text('Explorer')",
            timeout=DEFAULT_TIMEOUT_MS,
        )
        assert explorer_btn is not None
        print("[E2E] Found Explorer button on accessible channel.")

        # 3. Click Explorer button to enter Master-Detail view
        explorer_btn.click()
        page.wait_for_selector("text=Sources Médias", timeout=DEFAULT_TIMEOUT_MS)
        page.wait_for_selector("input[placeholder*='Filtrer']", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] Entered Explorer view.")

        # 4. Verify Master list items are rendered
        page.wait_for_selector("[id^='item-']", timeout=DEFAULT_TIMEOUT_MS)
        items_count = page.locator("[id^='item-']").count()
        assert items_count > 0, "No items found in explorer master list"
        print(f"[E2E] Explorer master list displayed {items_count} items on first page.")

        # Verify URL hash synchronized
        assert "view=explorer" in page.url, f"Expected view=explorer in url: {page.url}"

        # 4b. Verify decoupled scrollbars (zero window scroll, independent pane scrolls)
        window_scrollable = page.evaluate("() => document.documentElement.scrollHeight > window.innerHeight")
        assert not window_scrollable, "Page window must not have an outer vertical scrollbar in Explorer view"

        # Verify left pane can scroll independently without affecting window scroll
        page.evaluate("() => { const p = document.querySelectorAll('.custom-scroll'); if (p[0]) p[0].scrollTop = 50; }")
        assert page.evaluate("() => window.scrollY") == 0, "Window scroll must remain 0 when scrolling master pane"
        print("[E2E] Window has zero vertical scrollbar (decoupled layout verified, window.scrollY = 0).")

        # 5. Test fuzzy search query
        search_input = page.locator("input[placeholder*='Filtrer']")
        search_input.fill("Watchmen")
        page.wait_for_selector("text=Watchmen", timeout=DEFAULT_TIMEOUT_MS)
        filtered_count = page.locator("[id^='item-']").count()
        assert filtered_count >= 1, "Expected at least 1 filtered item for query 'Watchmen'"
        print(f"[E2E] Search 'Watchmen' yielded {filtered_count} matching files.")

        # 6. Click on matching file and verify detail card
        file_item = page.locator("[id^='item-']:has-text('Watchmen')").first
        file_item.click()
        page.wait_for_selector("text=Caractéristiques Multimédia", timeout=DEFAULT_TIMEOUT_MS)
        page.wait_for_selector("text=Flux Vidéo", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] Detail card successfully probed and displayed multimedia streams.")

        # 7. Test page reload persistence (F5)
        print("[E2E] Reloading page to verify state persistence...")
        page.reload(wait_until="networkidle")
        page.wait_for_selector("text=Caractéristiques Multimédia", timeout=DEFAULT_TIMEOUT_MS)
        page.wait_for_selector("text=Watchmen", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] State successfully restored after page reload.")

        # 8. Take verification screenshot if requested
        if output_screenshot:
            output_screenshot.parent.mkdir(parents=True, exist_ok=True)
            page.screenshot(path=str(output_screenshot), full_page=True)
            print(f"[E2E] Screenshot saved to {output_screenshot}")

        # 9. Test breadcrumb return
        back_btn = page.locator("button:has-text('←')").first
        back_btn.click()
        page.wait_for_selector("text=Sources Médias : Accès Réseau & Disques", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] Successfully navigated back via breadcrumbs.")

        browser.close()
        print("[E2E] All scenarios PASSED cleanly.")


def main() -> None:
    """CLI entrypoint."""
    parser = argparse.ArgumentParser(description="Media Sources E2E Playwright validation")
    parser.add_argument("--url", default=DEFAULT_PORTAL_URL, help="Target portal URL")
    parser.add_argument("--screenshot", type=Path, default=None, help="Path to save full-page screenshot")
    args = parser.parse_args()

    try:
        run_e2e_test(args.url, args.screenshot)
    except Exception as err:
        print(f"[E2E ERROR] {err}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
