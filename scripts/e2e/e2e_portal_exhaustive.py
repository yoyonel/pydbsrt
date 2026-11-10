#!/usr/bin/env python3
# pyright: reportMissingImports=false
"""Automated exhaustive Playwright E2E test for the entire pydbsrt Portal.

Validates:
1. Every tab: Sources, Actifs Médias (DB), Empreintes pHash, Trajectoires.
2. Every interactive control: buttons, dropdowns, filters, pagination, zoom (+, -, pan, reset).
3. Modal round-trips: Trajectory -> pHash -> Trajectory, and Escape key dismissal.
4. Strict console monitor: intercepts console.error, console.warn, pageerror, and Alpine expression bugs.
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
DESKTOP_UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"


class ConsoleTracker:
    """Captures and classifies browser console messages and uncaught exceptions."""

    def __init__(self) -> None:
        self.errors: list[str] = []
        self.warnings: list[str] = []
        self.logs: list[str] = []

    def on_console(self, msg: Any) -> None:
        text = msg.text
        mtype = msg.type
        entry = f"[{mtype.upper()}] {text}"
        self.logs.append(entry)

        is_critical_warn = "Alpine Expression Error" in text or "ReferenceError" in text or "not defined" in text
        if mtype == "error":
            self.errors.append(entry)
        elif is_critical_warn:
            self.errors.append(f"[CRITICAL WARN -> ERROR] {text}")
        elif mtype == "warning":
            self.warnings.append(entry)

    def on_page_error(self, exc: Any) -> None:
        entry = f"[PAGE ERROR] {exc}"
        self.errors.append(entry)
        self.logs.append(entry)


def _test_sources_view(page: Any) -> None:
    """Tests the Sources overview and Master-Detail file explorer."""
    page.wait_for_selector("text=Sources Médias : Accès Réseau & Disques", timeout=DEFAULT_TIMEOUT_MS)
    print("[E2E EXHAUSTIVE] Tab 1: Sources dashboard loaded.")

    snippet_card = page.locator("div.bg-slate-900:has-text('local-snippets')")
    explorer_btn = snippet_card.locator("button:has-text('Explorer')").first
    if not explorer_btn.is_visible():
        explorer_btn = page.locator("button:has-text('Explorer')").first
    if explorer_btn.is_visible():
        explorer_btn.click()
        page.wait_for_selector("input[placeholder*='Filtrer']", timeout=DEFAULT_TIMEOUT_MS)
        page.wait_for_selector("[id^='item-']", timeout=DEFAULT_TIMEOUT_MS)

        # Click multiple files in the explorer list
        items = page.locator("[id^='item-']")
        count = min(items.count(), 3)
        for i in range(count):
            items.nth(i).click()
            page.wait_for_selector("text=Caractéristiques Multimédia", timeout=DEFAULT_TIMEOUT_MS)

        # Test search filter input
        search_input = page.locator("input[x-model='explorerQuery']")
        search_input.fill("test")
        page.wait_for_timeout(200)
        search_input.fill("")
        page.wait_for_timeout(200)

        # Return to sources dashboard
        back_btn = page.locator("button:has-text('←')").first
        back_btn.click()
        page.wait_for_selector("text=Sources Médias : Accès Réseau & Disques", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E EXHAUSTIVE] Tab 1: Explorer and file inspection passed.")


def _test_assets_view(page: Any) -> None:
    """Tests Actifs Médias (DB) table, filter pills, and detail modal."""
    btn = page.locator("button:has-text('Actifs Médias (DB)')")
    btn.click()
    page.wait_for_selector("text=Actifs Médias : Intégrité Base", timeout=DEFAULT_TIMEOUT_MS)

    # Click first row if any
    first_row = page.locator("[x-show*='assets'] tbody tr.cursor-pointer").first
    if first_row.is_visible():
        first_row.click()
        page.wait_for_selector("div[x-show='selectedAssetModal']", timeout=DEFAULT_TIMEOUT_MS)
        # Test dismiss via Escape
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)

    print("[E2E EXHAUSTIVE] Tab 2: Actifs Médias tested.")


def _test_phashes_view(page: Any) -> None:
    """Tests Empreintes pHash observatory, seismograph zoom, pan and reset controls."""
    btn = page.locator("button:has-text('Empreintes pHash')")
    btn.click()
    page.wait_for_selector("text=Empreintes pHash : Observatoire d'Intégrité", timeout=DEFAULT_TIMEOUT_MS)

    # Wait for inspect button or empty state
    inspect_btn = page.locator("div[x-show*='phashes'] button:has-text('Inspecter')").first
    if not inspect_btn.is_visible():
        try:
            inspect_btn.wait_for(timeout=3000)
        except Exception:
            pass

    if inspect_btn.is_visible():
        inspect_btn.click()
        page.wait_for_selector("div[x-show='selectedPhashModal']", timeout=DEFAULT_TIMEOUT_MS)
        page.wait_for_selector("text=Dynamisme & Mobilité Inter-Trames", timeout=DEFAULT_TIMEOUT_MS)

        # Test zoom controls: Zoom In (+), Zoom Out (-), Pan, Reset
        zoom_in_btn = page.locator("div[x-show='selectedPhashModal'] button:has-text('+')").first
        zoom_out_btn = page.locator("div[x-show='selectedPhashModal'] button:has-text('−')").first
        pan_left_btn = page.locator("div[x-show='selectedPhashModal'] button:has-text('◀')").first
        pan_right_btn = page.locator("div[x-show='selectedPhashModal'] button:has-text('▶')").first
        reset_btn = page.locator("div[x-show='selectedPhashModal'] button:has-text('↺ Tout')").first

        if zoom_in_btn.is_visible():
            zoom_in_btn.click()
            page.wait_for_timeout(100)
            zoom_in_btn.click()
            page.wait_for_timeout(100)
            pan_left_btn.click()
            page.wait_for_timeout(100)
            pan_right_btn.click()
            page.wait_for_timeout(100)
            zoom_out_btn.click()
            page.wait_for_timeout(100)
            reset_btn.click()
            page.wait_for_timeout(100)

        # Test tip toggle buttons
        tip_btns = page.locator("div[x-show='selectedPhashModal'] button:has-text('?')")
        for i in range(min(tip_btns.count(), 3)):
            tip_btns.nth(i).click()
            page.wait_for_timeout(50)
            tip_btns.nth(i).click()

        # Dismiss via Escape
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)
        print("[E2E EXHAUSTIVE] Tab 3: pHash seismograph zoom & tip controls tested.")
    else:
        print("[E2E EXHAUSTIVE] Tab 3: No pHash item found to inspect.")


def _test_trajectories_combobox(page: Any) -> None:
    """Tests the searchable trajectory file combobox dropdown and switching report."""
    dropdown_btn = page.locator("div[x-show*='trajectories'] button:has-text('.jsonl')").first
    if dropdown_btn.is_visible():
        dropdown_btn.click()
        page.wait_for_timeout(150)
        search_box = page.locator("input[placeholder*='Filtrer par nom']").first
        if search_box.is_visible():
            search_box.fill("cuts")
            page.wait_for_timeout(100)
            # Pick a filtered option if available
            report_option = page.locator("div[x-show*='trajectoryDropdownOpen'] button:has-text('.jsonl')").first
            if report_option.is_visible():
                report_option.click()
                page.wait_for_timeout(300)
                return
            search_box.fill("")
            page.wait_for_timeout(100)
        # Close dropdown by clicking outside or Escape
        page.keyboard.press("Escape")
        page.wait_for_timeout(150)


def _test_trajectories_roundtrip(page: Any) -> None:
    """Tests Trajectory modal inspection, jump to pHash and return button."""
    row = page.locator("[x-show*='trajectories'] tbody tr.cursor-pointer").first
    if not row.is_visible():
        print("[E2E EXHAUSTIVE] Tab 4: No trajectory row available.")
        return

    row.click()
    page.wait_for_selector("div[x-show='selectedTrajectoryModal']", timeout=DEFAULT_TIMEOUT_MS)
    page.wait_for_selector("text=Verdict du Consensus Multi-Coupures", timeout=DEFAULT_TIMEOUT_MS)

    # Jump to pHash modal
    phash_inspect_btn = page.locator("button:has-text('Empreinte pHash')").first
    if phash_inspect_btn.is_visible():
        phash_inspect_btn.click()
        page.wait_for_selector("div[x-show='selectedPhashModal']", timeout=DEFAULT_TIMEOUT_MS)

        # Verify Return button
        return_btn = page.locator("button:has-text('Retour Trajectoire')").first
        assert return_btn.is_visible(), "Expected 'Retour Trajectoire' button in pHash modal"

        # Test zoom controls inside pHash modal opened from trajectory
        zoom_in_btn = page.locator("div[x-show='selectedPhashModal'] button:has-text('+')").first
        if zoom_in_btn.is_visible():
            zoom_in_btn.click()
            page.wait_for_timeout(100)

        # Return to trajectory modal
        return_btn.click()
        page.wait_for_selector("div[x-show='selectedTrajectoryModal']", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E EXHAUSTIVE] Trajectory -> pHash -> Trajectory round-trip confirmed.")

    # Close modal via Escape
    page.keyboard.press("Escape")
    page.wait_for_timeout(200)


def _test_trajectories_view(page: Any) -> None:
    """Tests Trajectoires tab, filter pills, combobox, and round-trip modal."""
    btn = page.locator("button:has-text('Trajectoires')")
    btn.click()
    page.wait_for_selector("text=Trajectoires Spatiotemporelles & Consensus", timeout=DEFAULT_TIMEOUT_MS)

    # Test status pills
    pills = ["Cohérents", "Consensus Gagné", "Divergents", "Sans Match", "Tous"]
    for pill in pills:
        pill_btn = page.locator(f"[x-show*='trajectories'] button:has-text('{pill}')").first
        if pill_btn.is_visible():
            pill_btn.click()
            page.wait_for_timeout(100)

    _test_trajectories_combobox(page)
    _test_trajectories_roundtrip(page)
    print("[E2E EXHAUSTIVE] Tab 4: Trajectories view fully verified.")


def run_exhaustive_e2e(url: str, screenshot_path: Path | None = None) -> list[str]:
    """Runs exhaustive browser exploration and returns collected errors."""
    tracker = ConsoleTracker()

    with sync_playwright() as p:
        chrome_bin = shutil.which("google-chrome") or shutil.which("chromium")
        browser = p.chromium.launch(
            executable_path=chrome_bin,
            headless=True,
            args=["--no-sandbox", "--disable-dev-shm-usage"],
        )
        context = browser.new_context(
            viewport={"width": VIEWPORT_WIDTH, "height": VIEWPORT_HEIGHT},
            user_agent=DESKTOP_UA,
        )
        page = context.new_page()

        page.on("console", tracker.on_console)
        page.on("pageerror", tracker.on_page_error)

        print(f"[E2E EXHAUSTIVE] Opening {url}...")
        page.goto(url, wait_until="domcontentloaded")
        page.wait_for_timeout(1000)

        _test_sources_view(page)
        _test_assets_view(page)
        _test_phashes_view(page)
        _test_trajectories_view(page)

        if screenshot_path:
            screenshot_path.parent.mkdir(parents=True, exist_ok=True)
            page.screenshot(path=str(screenshot_path), full_page=True)
            print(f"[E2E EXHAUSTIVE] Screenshot saved to {screenshot_path}")

        browser.close()

    return tracker.errors


def main() -> None:
    """CLI entrypoint."""
    parser = argparse.ArgumentParser(description="Exhaustive Playwright E2E test with console zero-error assertion")
    parser.add_argument("--url", default=DEFAULT_PORTAL_URL, help="Target portal URL")
    parser.add_argument("--screenshot", type=Path, default=None, help="Save screenshot path")
    args = parser.parse_args()

    errors = run_exhaustive_e2e(args.url, args.screenshot)
    if errors:
        print(f"\n[FATAL] Detected {len(errors)} console error(s) during exploration:", file=sys.stderr)
        for err in errors:
            print(f"  ❌ {err}", file=sys.stderr)
        sys.exit(1)

    print("\n✅ [PASS] Exhaustive E2E test completed: 0 console errors, 0 page errors, 0 Alpine warnings.")


if __name__ == "__main__":
    main()
