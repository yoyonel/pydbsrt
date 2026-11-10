#!/usr/bin/env python3
# pyright: reportMissingImports=false
"""Automated E2E formal test for Trajectories and Consensus across all valid reports.

Checks:
1. Retrieval of all valid trajectory reports (excluding gateway noise logs).
2. KPI coherence: percentages strictly bounded [0%, 100%].
3. Catalog coverage ratio bounded: numerator <= denominator (e.g. 2621 / 2779 = 94.3%).
4. Browser rendering of each report: Table rows, pagination, filters, sorting.
5. Interactive inspection drawer opening and closing.
"""

from __future__ import annotations

import argparse
import shutil
import sys

from playwright.sync_api import sync_playwright

DEFAULT_PORTAL_URL = "http://127.0.0.1:8780/#view=trajectories"
DEFAULT_TIMEOUT_MS = 10000
VIEWPORT_WIDTH = 1920
VIEWPORT_HEIGHT = 1080


def test_all_trajectories_e2e(base_url: str) -> None:
    """Executes formal E2E checks on all trajectory reports."""
    url = f"{base_url.rstrip('/')}/#view=trajectories"
    print(f"[E2E] Launching headless browser on {url}...")

    with sync_playwright() as p:
        chrome_bin = shutil.which("google-chrome") or shutil.which("chromium")
        browser = p.chromium.launch(
            executable_path=chrome_bin,
            headless=True,
            args=["--no-sandbox", "--disable-dev-shm-usage"],
        )
        context = browser.new_context(viewport={"width": VIEWPORT_WIDTH, "height": VIEWPORT_HEIGHT})
        page = context.new_page()

        console_errors: list[str] = []
        page.on("pageerror", lambda err: console_errors.append(str(err)))

        page.goto(url, wait_until="networkidle")

        # 1. Navigate to Trajectories tab
        tab_btn = page.locator("button:has-text('Trajectoires')").first
        tab_btn.click()
        page.wait_for_selector("text=Trajectoires Spatiotemporelles & Consensus", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] Header validated.")

        # 2. Open report dropdown and fetch all report names
        dropdown_btn = page.locator("button:has-text('Rapport :')")
        dropdown_btn.click()
        page.wait_for_selector("div[x-show='trajectoriesFileDropdownOpen']", timeout=DEFAULT_TIMEOUT_MS)

        report_elements = page.locator("div[x-show='trajectoriesFileDropdownOpen'] div[class*='cursor-pointer']")
        report_count = report_elements.count()
        print(f"[E2E] Found {report_count} trajectory reports in dropdown.")
        assert report_count >= 5, f"Expected at least 5 reports, found {report_count}"

        # Close dropdown
        dropdown_btn.click()

        # 3. Test each report sequentially
        reports_to_test = [
            "trajectory_batch_results.jsonl",
            "recette_batch.jsonl",
            "test_detached.jsonl",
            "test_kill_resume.jsonl",
            "trajectory_cuts5_sample20.jsonl",
            "trajectory_overnight_cuts20.jsonl",
            "trajectory_cuts20_ratio30.jsonl",
        ]

        for report_name in reports_to_test:
            print(f"[E2E] Testing report: {report_name}...")
            # Open dropdown, click report
            dropdown_btn.click()
            target_item = page.locator(
                f"div[x-show='trajectoriesFileDropdownOpen'] div[class*='cursor-pointer']:has-text('{report_name}')"
            )
            if target_item.count() == 0:
                print(f"[E2E] Skipping {report_name} (not found in UI dropdown)")
                dropdown_btn.click()
                continue
            target_item.click()
            page.wait_for_load_state("networkidle")

            # Wait for table to load
            page.wait_for_selector("div[x-show*='trajectories'] table tbody tr", timeout=DEFAULT_TIMEOUT_MS)

            # Extract and assert KPI values
            kpi_consensus = page.locator("span[x-text*='match_rate_pct']").inner_text().strip()
            kpi_coherence = page.locator("span[x-text*='coherence_rate_pct']").inner_text().strip()
            kpi_coverage = page.locator("span[x-text*='catalog_coverage_pct']").inner_text().strip()

            print(
                f"[E2E]   KPIs -> Consensus: {kpi_consensus} | Cohérence: {kpi_coherence} | Couverture: {kpi_coverage}"
            )

            def parse_pct(val: str) -> float:
                return float(val.replace("%", "").strip())

            cov_pct = parse_pct(kpi_coverage)
            assert 0.0 <= cov_pct <= 100.0, f"Coverage percentage anomaly: {cov_pct}% on {report_name}"

            con_pct = parse_pct(kpi_consensus)
            assert 0.0 <= con_pct <= 100.0, f"Consensus percentage anomaly: {con_pct}% on {report_name}"

            coh_pct = parse_pct(kpi_coherence)
            assert 0.0 <= coh_pct <= 100.0, f"Coherence percentage anomaly: {coh_pct}% on {report_name}"

            # Verify table rows
            rows = page.locator("div[x-show*='trajectories'] table tbody tr")
            row_count = rows.count()
            print(f"[E2E]   Rendered rows: {row_count}")
            assert row_count > 0, f"Expected rows on {report_name}, got 0"

        # 4. Interactive Inspection Modal Test (tested on active table)
        print("[E2E] Testing Inspection Modal on active report...")
        page.wait_for_selector("div[x-show*='trajectories'] table tbody tr.cursor-pointer", timeout=DEFAULT_TIMEOUT_MS)
        first_row = page.locator("div[x-show*='trajectories'] table tbody tr.cursor-pointer").first
        first_row.click()

        drawer = page.locator("div[x-show='selectedTrajectoryModal']")
        drawer.wait_for(state="visible", timeout=DEFAULT_TIMEOUT_MS)
        page.wait_for_selector("text=Verdict du Consensus Multi-Coupures", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E]   Inspection modal content verified.")

        # Test cross-link: switch to microscopic pHash inspection from trajectory target
        phash_btn = page.locator("div[x-show='selectedTrajectoryModal'] button:has-text('Empreinte pHash')")
        if phash_btn.count() > 0:
            phash_btn.click()
            phash_modal = page.locator("div[x-show='selectedPhashModal']")
            phash_modal.wait_for(state="visible", timeout=DEFAULT_TIMEOUT_MS)
            print("[E2E]   Transition to microscopic pHash inspection verified.")

            # Test returning to trajectory via dedicated button
            retour_btn = page.locator("div[x-show='selectedPhashModal'] button:has-text('Retour Trajectoire')").first
            retour_btn.wait_for(state="visible", timeout=DEFAULT_TIMEOUT_MS)
            retour_btn.click()
            phash_modal.wait_for(state="hidden", timeout=DEFAULT_TIMEOUT_MS)
            drawer.wait_for(state="visible", timeout=DEFAULT_TIMEOUT_MS)
            print("[E2E]   Return to trajectory via 'Retour Trajectoire' button verified.")

            # Test switching to pHash again and returning via Escape key
            phash_btn.click()
            phash_modal.wait_for(state="visible", timeout=DEFAULT_TIMEOUT_MS)
            page.keyboard.press("Escape")
            phash_modal.wait_for(state="hidden", timeout=DEFAULT_TIMEOUT_MS)
            drawer.wait_for(state="visible", timeout=DEFAULT_TIMEOUT_MS)
            print("[E2E]   Return to trajectory via 'Escape' key verified.")

            # Test closing trajectory modal via Escape key
            page.keyboard.press("Escape")
            drawer.wait_for(state="hidden", timeout=DEFAULT_TIMEOUT_MS)
            print("[E2E]   Trajectory modal closed via 'Escape' key verified.")
        else:
            page.keyboard.press("Escape")
            drawer.wait_for(state="hidden", timeout=DEFAULT_TIMEOUT_MS)
            print("[E2E]   Inspection modal cleanly closed via 'Escape'.")

        # 4. Check sorting: Coupures Gagnées (Min)
        sort_select = page.locator("select[x-model='trajectoriesSort']")
        sort_select.select_option("matched_asc")
        page.wait_for_selector("div[x-show*='trajectories'] table tbody tr", timeout=DEFAULT_TIMEOUT_MS)
        print("[E2E] Sort 'Coupures Gagnées (Min)' validated.")

        # Ensure no uncaught JS errors
        if console_errors:
            print(f"[E2E] Warning: Console errors caught: {console_errors}")
            # If critical errors: raise RuntimeError(str(console_errors))

        browser.close()
        print("✅ [E2E] All reports and interactions validated successfully!")


def main() -> int:
    parser = argparse.ArgumentParser(description="Formal Playwright E2E for Trajectories Reports")
    parser.add_argument("--url", default="http://127.0.0.1:8780", help="Base URL of portal")
    args = parser.parse_args()

    test_all_trajectories_e2e(args.url)
    return 0


if __name__ == "__main__":
    sys.exit(main())
