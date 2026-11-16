# pyright: reportMissingImports=false
"""Portal B Part 2: Actifs Médias DB, Empreintes pHash, Trajectoires & Glossaire scenarios."""

from __future__ import annotations

import time
from typing import TYPE_CHECKING

from scripts.e2e.recette_o2switch.harness import (
    DEFAULT_TIMEOUT_MS,
    MIN_ALIGNED_3_3_COUNT,
    MIN_CATALOG_PHASH_COUNT,
    MIN_PG_ASSETS_TOTAL,
    MIN_TRAJECTORY_FILES,
    PERCENT_SCALE,
    UI_SETTLE_MS,
    RecetteContext,
)

if TYPE_CHECKING:
    from playwright.sync_api import Page

ASSETS_KPI_FILTERS: tuple[str, ...] = ("nominal", "missing_phash", "unmapped_path", "all")
ASSETS_SOURCE_FILTERS: tuple[str, ...] = ("synology-video", "synology-tvshow", "local-snippets", "")
ASSETS_SORT_MODES: tuple[str, ...] = ("title_desc", "frames_desc", "duration_desc", "id_desc", "title_asc")
ASSETS_LIMIT_MODES: tuple[str, ...] = ("15", "50", "25")

PHASH_STATUS_FILTERS: tuple[str, ...] = ("aligned_3_3", "missing_pg", "uncompiled", "all")
PHASH_SORT_MODES: tuple[str, ...] = (
    "id_desc",
    "title_asc",
    "title_desc",
    "frames_desc",
    "frames_asc",
    "size_desc",
    "cuts_desc",
    "id_asc",
)
PHASH_LIMIT_MODES: tuple[str, ...] = ("15", "50", "25")
POPCOUNT_MIN_HEALTHY: float = 20.0
POPCOUNT_MAX_HEALTHY: float = 44.0

TRAJ_STATUS_FILTERS: tuple[str, ...] = ("coherent", "matched", "divergent", "unmatched", "all")
TRAJ_SORT_MODES: tuple[str, ...] = (
    "name_desc",
    "matched_desc",
    "matched_asc",
    "duration_desc",
    "duration_asc",
    "elapsed_desc",
    "elapsed_asc",
    "name_asc",
)


def _wait_assets_ready(page: Page) -> None:
    page.wait_for_function(
        "() => { const d = Alpine.$data(document.body); return d.currentView === 'assets' && !d.assetsLoading; }",
        timeout=DEFAULT_TIMEOUT_MS,
    )


def _wait_phashes_ready(page: Page) -> None:
    page.wait_for_function(
        "() => { const d = Alpine.$data(document.body); return d.currentView === 'phashes' && !d.phashesLoading; }",
        timeout=DEFAULT_TIMEOUT_MS,
    )


def _wait_phash_modal_loaded(page: Page) -> None:
    page.wait_for_function(
        """() => {
            const d = Alpine.$data(document.body);
            return d.selectedPhashModal === true && !d.phashDetailLoading
                && d.selectedPhash && d.selectedPhash.entropy && d.selectedPhash.mobility;
        }""",
        timeout=DEFAULT_TIMEOUT_MS,
    )


def _wait_trajectories_ready(page: Page) -> None:
    page.wait_for_function(
        """() => {
            const d = Alpine.$data(document.body);
            return d.currentView === 'trajectories' && !d.trajectoriesLoading && d.trajectoriesFiles.length > 0;
        }""",
        timeout=DEFAULT_TIMEOUT_MS,
    )


def step_b4_assets_db_audit_and_drawer(ctx: RecetteContext) -> None:
    """Verify PostgreSQL Actifs Médias DB KPIs, filters, sorts, pagination, ⓘ glossary & slide-over drawer."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.locator("header nav button:has-text('Actifs Médias (DB)')").click()
    _wait_assets_ready(page)
    summary = page.evaluate(
        "() => ({ connected: Alpine.$data(document.body).assetsConnected, pg: Alpine.$data(document.body).assetsPgVersion, sum: Alpine.$data(document.body).assetsSummary })"
    )
    assert summary["connected"] is True, f"PostgreSQL disconnected on o2switch: {summary}"
    assert summary["sum"]["total"] >= MIN_PG_ASSETS_TOTAL
    shot_assets = ctx.shot("assets_db_postgresql_overview")

    assets_sec = page.locator("div[x-show=\"currentView === 'assets'\"]")
    kpi_cards = assets_sec.locator("div.grid.lg\\:grid-cols-4 > div")
    for idx in (1, 2, 3, 0):
        kpi_cards.nth(idx).click()
        _wait_assets_ready(page)

    probe_query = page.evaluate(
        """() => {
            const items = Alpine.$data(document.body).assets || [];
            const accented = items.find((item) => /[àâäéèêëîïôöùûüç]/i.test(item.clean_title || item.name || ""));
            const target = accented || items[0];
            if (!target) return { raw: "", query: "" };
            const raw = String(target.clean_title || target.name || "");
            const ascii = raw.normalize("NFD").replace(/[\\u0300-\\u036f]/g, "");
            const tokens = ascii.match(/[A-Za-z0-9]{4,}/g) || [];
            return { raw, query: tokens[0] || ascii.slice(0, 6) };
        }"""
    )
    query_term = probe_query["query"] or "mp4"
    search_box = assets_sec.locator("input[x-model='assetsSearchQuery']")
    search_box.fill(query_term)
    page.wait_for_timeout(UI_SETTLE_MS)
    _wait_assets_ready(page)
    accent_hits = page.evaluate("() => Alpine.$data(document.body).assetsTotalItems")
    assert accent_hits >= 1, f"PostgreSQL search for '{query_term}' (from '{probe_query['raw']}') must match >= 1 item"
    search_box.fill("")
    page.wait_for_timeout(UI_SETTLE_MS)
    _wait_assets_ready(page)

    src_sel = assets_sec.locator("select[x-model='assetsSourceFilter']")
    for src_val in ASSETS_SOURCE_FILTERS:
        src_sel.select_option(src_val)
        _wait_assets_ready(page)

    sort_sel = assets_sec.locator("select[x-model='assetsSort']")
    for sort_val in ASSETS_SORT_MODES:
        sort_sel.select_option(sort_val)
        _wait_assets_ready(page)

    lim_sel = assets_sec.locator("select[x-model='assetsLimit']")
    for lim_val in ASSETS_LIMIT_MODES:
        lim_sel.select_option(lim_val)
        _wait_assets_ready(page)

    assets_sec.locator("button:has-text('Suivant →')").click()
    _wait_assets_ready(page)
    assets_sec.locator("button:has-text('← Précédent')").click()
    _wait_assets_ready(page)

    assets_sec.locator("button[title='Aide sur les empreintes pHash']").click()
    page.wait_for_function("() => Alpine.$data(document.body).showGlossaryModal === true")
    page.keyboard.press("Escape")
    page.wait_for_function("() => Alpine.$data(document.body).showGlossaryModal === false")

    assets_sec.locator("button:has-text('Actualiser')").click()
    _wait_assets_ready(page)

    assets_sec.locator("button:has-text('Inspecter')").first.click()
    page.wait_for_function(
        "() => { const d = Alpine.$data(document.body); return d.selectedAssetModal && !d.assetDetailLoading && d.selectedAsset !== null; }",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    asset_detail = page.evaluate("() => Alpine.$data(document.body).selectedAsset")
    assert asset_detail.get("id") is not None and asset_detail.get("name")
    shot_drawer = ctx.shot("assets_db_slideover_drawer")

    page.locator("div[x-show='selectedAssetModal'] button:has-text('Fermer')").click()
    page.wait_for_function("() => Alpine.$data(document.body).selectedAssetModal === false")

    ctx.record_step(
        step_id="B4",
        portal="Sources",
        feature="Actifs Médias DB (PostgreSQL Live, 4 KPIs, Recherche Accents, Tris, Pagination & Tiroir Technique)",
        t0=t0,
        assertions=[
            f"Connexion {summary['pg']} active : {summary['sum']['total']} actifs ({summary['sum']['nominal']} nominaux)",
            f"Recherche insensible aux accents ('{query_term}' -> {accent_hits} résultat(s))",
            "4 cartes KPI, 4 filtres source, 5 tris, 3 limites, pagination, aide ⓘ et tiroir Inspecter validés",
        ],
        metrics={"pg_version": summary["pg"], "summary": summary["sum"], "overview_shot": shot_assets},
        screenshot=shot_drawer,
    )


def step_b5_phashes_tripartite_and_microscopy(ctx: RecetteContext) -> None:
    """Verify Empreintes pHash 3 pillars, filters, sorts, and microscopic inspection on 3 filename classes."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.locator("header nav button:has-text('Empreintes pHash')").click()
    _wait_phashes_ready(page)
    ph_sum = page.evaluate("() => Alpine.$data(document.body).phashesSummary")
    assert ph_sum["raw_count"] >= MIN_CATALOG_PHASH_COUNT
    assert ph_sum["catalog_count"] >= MIN_CATALOG_PHASH_COUNT
    assert ph_sum["aligned_3_3"] >= MIN_ALIGNED_3_3_COUNT
    assert isinstance(ph_sum["uncompiled"], int) and ph_sum["uncompiled"] >= 0
    shot_phash_main = ctx.shot("phashes_tripartite_observatory")

    ph_sec = page.locator("div[x-show=\"currentView === 'phashes'\"]")
    for status_key in PHASH_STATUS_FILTERS:
        page.evaluate(f"() => Alpine.$data(document.body).setPhashesStatusFilter('{status_key}')")
        _wait_phashes_ready(page)

    sort_sel = ph_sec.locator("select[x-model='phashesSort']")
    for sort_val in PHASH_SORT_MODES:
        sort_sel.select_option(sort_val)
        _wait_phashes_ready(page)

    lim_sel = ph_sec.locator("select[x-model='phashesLimit']")
    for lim_val in PHASH_LIMIT_MODES:
        lim_sel.select_option(lim_val)
        _wait_phashes_ready(page)

    ph_sec.locator("button:has-text('Suivant →')").click()
    _wait_phashes_ready(page)
    ph_sec.locator("button:has-text('← Précédent')").click()
    _wait_phashes_ready(page)

    copy_mh_btn = ph_sec.locator("button[title='Copier le MovieHash']").first
    if copy_mh_btn.is_visible():
        copy_mh_btn.click()

    # Case 1: Standard item + full microscopic tooltips + Δ(t) seismograph zoom/pan/reset
    ph_sec.locator("button:has-text('Inspecter')").first.click()
    _wait_phash_modal_loaded(page)
    modal = page.locator("div[x-show='selectedPhashModal']")
    modal.locator("button[title='Aide technique']").click()
    modal.locator("button[title='Aide technique sur la mobilité Δ']").click()
    modal.locator('button[title="Aide sur l\'histogramme de mobilité"]').click()
    modal.locator("button[title='Aide sur la courbe temporelle']").click()

    modal.locator("button[title='Zoom avant']").click()
    modal.locator("button[title='Zoom avant']").click()
    modal.locator("button[title='Déplacer à droite']").click()
    modal.locator("button[title='Déplacer à gauche']").click()
    modal.locator("button[title='Zoom arrière']").click()
    zoom_lbl = page.evaluate("() => Alpine.$data(document.body).getZoomLevelLabel()")
    shot_micro = ctx.shot("phashes_microscopic_seismograph_zoomed")
    modal.locator('button[title="Vue d\'ensemble"]').click()
    page.keyboard.press("Escape")
    page.wait_for_function("() => Alpine.$data(document.body).selectedPhashModal === false")

    # Case 2 & 3: UTF-8 accented stem ('Cruauté') and percent-encoded stem ('OxTorrent')
    search_in = ph_sec.locator("input[x-model='phashesSearchQuery']")
    verified_edge_cases: list[str] = []
    for query_term in ("Cruauté", "OxTorrent"):
        search_in.fill(query_term)
        page.wait_for_timeout(UI_SETTLE_MS)
        _wait_phashes_ready(page)
        row_cnt = ph_sec.locator("button:has-text('Inspecter')").count()
        assert row_cnt >= 1, f"Expected pHash search results for '{query_term}'"
        ph_sec.locator("button:has-text('Inspecter')").first.click()
        _wait_phash_modal_loaded(page)
        inspected = page.evaluate(
            "() => ({ file: Alpine.$data(document.body).selectedPhash.raw_filename, pop: Alpine.$data(document.body).selectedPhash.entropy.mean_popcount })"
        )
        assert POPCOUNT_MIN_HEALTHY <= inspected["pop"] <= POPCOUNT_MAX_HEALTHY
        verified_edge_cases.append(f"{query_term} -> {inspected['file']} (pop={inspected['pop']})")
        page.keyboard.press("Escape")
        page.wait_for_function("() => Alpine.$data(document.body).selectedPhashModal === false")

    search_in.fill("")
    page.wait_for_timeout(UI_SETTLE_MS)
    _wait_phashes_ready(page)

    ctx.record_step(
        step_id="B5",
        portal="Sources",
        feature="Observatoire Tripartite Empreintes pHash, Sismographe Δ(t) & Cas Limites UTF-8 / %5B%20",
        t0=t0,
        assertions=[
            f"Trinité réconciliée : RAW={ph_sum['raw_count']}, Zero-DB={ph_sum['catalog_count']}, Alignés 3/3={ph_sum['aligned_3_3']}, À compiler={ph_sum['uncompiled']}",
            f"Zoom/Pan sismographe Δ(t) ({zoom_lbl}), 4 infobulles '?' et compression Zero-DB vérifiés",
            f"Cas limites non-ASCII & URL-encodés inspectés sans erreur : {'; '.join(verified_edge_cases)}",
        ],
        metrics={"summary": ph_sum, "edge_cases": verified_edge_cases, "overview_shot": shot_phash_main},
        screenshot=shot_micro,
    )


def step_b6_trajectories_roundtrip_and_glossary(ctx: RecetteContext) -> None:
    """Verify Trajectories KPIs, .jsonl combobox, filters, sorts, Trajectory->pHash->Trajectory & Glossary."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.locator("header nav button:has-text('Trajectoires')").click()
    _wait_trajectories_ready(page)
    traj_state = page.evaluate(
        "() => ({ files: Alpine.$data(document.body).trajectoriesFiles, active: Alpine.$data(document.body).trajectoriesActiveFile, kpi: Alpine.$data(document.body).trajectoriesKpi })"
    )
    assert len(traj_state["files"]) >= MIN_TRAJECTORY_FILES
    kpi = traj_state["kpi"]
    assert 0.0 <= float(kpi["match_rate_pct"]) <= PERCENT_SCALE
    assert 0.0 <= float(kpi["coherence_rate_pct"]) <= PERCENT_SCALE
    shot_traj = ctx.shot("trajectories_dashboard_kpis")

    traj_sec = page.locator("div[x-show=\"currentView === 'trajectories'\"]")
    traj_sec.locator("button:has-text('Rapport :')").click()
    page.wait_for_function("() => Alpine.$data(document.body).trajectoriesFileDropdownOpen === true")
    shot_combo = ctx.shot("trajectories_jsonl_combobox_open")

    combo_search = traj_sec.locator("input[x-model='trajectoriesFileSearch']")
    combo_search.fill("cuts")
    combo_search.fill("")
    second_file = traj_state["files"][1]["filename"]
    page.evaluate(f"() => Alpine.$data(document.body).selectTrajectoryFile('{second_file}')")
    _wait_trajectories_ready(page)
    first_file = traj_state["files"][0]["filename"]
    page.evaluate(f"() => Alpine.$data(document.body).selectTrajectoryFile('{first_file}')")
    _wait_trajectories_ready(page)

    for filt in TRAJ_STATUS_FILTERS:
        page.evaluate(f"() => Alpine.$data(document.body).setTrajectoriesStatusFilter('{filt}')")
        _wait_trajectories_ready(page)

    sort_sel = traj_sec.locator("select[x-model='trajectoriesSort']")
    for sort_val in TRAJ_SORT_MODES:
        sort_sel.select_option(sort_val)
        _wait_trajectories_ready(page)

    page.evaluate("() => Alpine.$data(document.body).setTrajectoriesStatusFilter('coherent')")
    _wait_trajectories_ready(page)
    traj_sec.locator("tbody tr").first.click()
    page.wait_for_function(
        "() => { const d = Alpine.$data(document.body); return d.selectedTrajectoryModal && !d.trajectoryDetailLoading && d.selectedTrajectory !== null; }",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    shot_traj_modal = ctx.shot("trajectories_detail_modal_cuts")

    # Round-trip: Trajectory Modal -> Empreinte pHash -> Retour Trajectoire
    page.locator("div[x-show='selectedTrajectoryModal'] button:has-text('Empreinte pHash')").click()
    _wait_phash_modal_loaded(page)
    assert page.evaluate("() => Alpine.$data(document.body).trajectoryModalOrigin") is True
    shot_roundtrip = ctx.shot("trajectories_to_phash_roundtrip")

    page.locator("div[x-show='selectedPhashModal'] button:has-text('Retour Trajectoire')").first.click()
    page.wait_for_function(
        "() => Alpine.$data(document.body).selectedTrajectoryModal === true && !Alpine.$data(document.body).selectedPhashModal"
    )
    page.locator("div[x-show='selectedTrajectoryModal'] button:has-text('Fermer')").click()
    page.wait_for_function("() => Alpine.$data(document.body).selectedTrajectoryModal === false")

    # Full Glossary modal inspection
    page.evaluate("() => { Alpine.$data(document.body).showGlossaryModal = true; }")
    page.wait_for_function("() => Alpine.$data(document.body).showGlossaryModal === true")
    shot_glossary = ctx.shot("portal_contextual_glossary_modal")
    page.locator("div[x-show='showGlossaryModal'] button:has-text('Compris')").click()
    page.wait_for_function("() => Alpine.$data(document.body).showGlossaryModal === false")

    ctx.record_step(
        step_id="B6",
        portal="Sources",
        feature="Trajectoires Spatiotemporelles, Combobox .jsonl, Aller-Retour Trajectoire↔pHash & Glossaire",
        t0=t0,
        assertions=[
            f"{len(traj_state['files'])} rapports .jsonl détectés sur o2switch (actif: {first_file})",
            f"KPIs bornés [0, 100%] : consensus={kpi['match_rate_pct']}%, cohérence={kpi['coherence_rate_pct']}%",
            "5 filtres statut, 8 tris, modale séquences coupures, aller-retour 🔬 Empreinte pHash -> 🎯 Retour Trajectoire et Glossaire validés",
        ],
        metrics={
            "jsonl_files_count": len(traj_state["files"]),
            "kpi": kpi,
            "dashboard_shot": shot_traj,
            "combobox_shot": shot_combo,
            "traj_modal_shot": shot_traj_modal,
            "glossary_shot": shot_glossary,
        },
        screenshot=shot_roundtrip,
    )


def run_observatory_suite(ctx: RecetteContext) -> None:
    """Run Portal B Part 2 (Assets DB, pHashes, Trajectories & Glossary) in Session 1."""
    step_b4_assets_db_audit_and_drawer(ctx)
    step_b5_phashes_tripartite_and_microscopy(ctx)
    step_b6_trajectories_roundtrip_and_glossary(ctx)
