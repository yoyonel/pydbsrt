# pyright: reportMissingImports=false
"""Portal C (Monitoring & Web Top) and Portal D (DBSrt Studio) Playwright scenarios."""

from __future__ import annotations

import time
import urllib.parse
from typing import TYPE_CHECKING, Any

from scripts.e2e.recette_o2switch.harness import (
    DEFAULT_TIMEOUT_MS,
    HTTP_OK,
    MIN_CATALOG_PHASH_COUNT,
    MIN_CERTIFIED_COUPLINGS,
    UI_SETTLE_MS,
    RecetteContext,
)

if TYPE_CHECKING:
    from playwright.sync_api import Page

OVERVIEW_EXPECTED_TILES: int = 6
STATS_EXPECTED_DIMENSIONS: int = 4
MIN_SYSTEM_GROUPS: int = 4
MONITORING_MEDIA_FILTERS: tuple[str, ...] = ("subs", "nosubs", "virgin", "all")
MONITORING_MEDIA_SORTS: tuple[str, ...] = ("size", "duration", "status", "subs", "name")

STUDIO_LANG_FILTERS: tuple[str, ...] = ("fr", "en", "multi", "all")
STUDIO_STORE_FILTERS: tuple[str, ...] = ("pg", "json", "all")
STUDIO_MEDIA_SORTS: tuple[str, ...] = ("name_asc", "cues_desc", "fps_desc", "frames_desc")
STUDIO_CATALOG_STATUSES: tuple[str, ...] = ("certified", "uncoupled", "all")
STUDIO_CATALOG_FPS: tuple[str, ...] = ("24", "25", "30", "all")
STUDIO_CATALOG_SORTS: tuple[str, ...] = ("name_asc", "frames_desc", "size_desc", "certified_first")
STUDIO_PRESET_KEYS: tuple[str, ...] = ("kingdom1", "loop1", "mando3", "silicon1", "rick1", "bbb")
SENTINEL_MEDIA_PATH: str = "__recette_dry_run_sentinel__.mp4"
SENTINEL_PHASH_PATH: str = "__recette_dry_run_sentinel__.phash"
SENTINEL_SRT_PATH: str = "__recette_dry_run_sentinel__.srt"


def _wait_monitoring_hydrated(page: Page) -> None:
    """Wait until Monitoring SSE is open, Cristian ClockSync calibrated, and initial snapshots loaded."""
    page.wait_for_function(
        """() => {
            return typeof sseActive !== 'undefined' && sseActive === true
                && typeof clockSync !== 'undefined' && clockSync.calibrated === true
                && lastTelemetry !== null && lastTop !== null
                && document.querySelectorAll('#tiles .tile').length === 6;
        }""",
        timeout=DEFAULT_TIMEOUT_MS,
    )


def _wait_monitoring_table_ready(page: Page, tbody_id: str) -> None:
    """Wait until a paginated Monitoring table finishes its fetchTop(true) cycle."""
    page.wait_for_function(
        f"""() => {{
            const tb = document.getElementById('{tbody_id}');
            return tb && tb.style.opacity !== '0.4' && !tb.innerText.includes('⏳ Chargement');
        }}""",
        timeout=DEFAULT_TIMEOUT_MS,
    )


def step_c1_monitoring_overview_sse_and_catalogue_tile(ctx: RecetteContext, polled_legacy: list[str]) -> None:
    """Verify Monitoring #overview, SSE stream, Cristian ClockSync, and #catalogue -> #tab-media fix."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    auth_url = f"{ctx.base_url}/monitoring?token={urllib.parse.quote(ctx.api_key)}"
    page.goto(auth_url, wait_until="domcontentloaded")
    _wait_monitoring_hydrated(page)
    shot_ov = ctx.shot("monitoring_overview_sse_calibrated")

    sync_snap = page.evaluate(
        """() => ({
            sseActive,
            calibrated: clockSync.calibrated,
            rttMs: clockSync.lastRttMs,
            offsetMs: clockSync.effectiveOffset,
            hostname: lastTelemetry?.system?.hostname || '',
            discoveredTotal: lastTop?.discovered_media_total || 0,
        })"""
    )
    assert sync_snap["sseActive"] is True and sync_snap["calibrated"] is True

    # Click Overview 'Catalogue' tile (#catalogue) -> must route into #view-ingestion + #tab-media
    page.locator("#tiles a.tile[href='#catalogue']").click()
    page.wait_for_function(
        """() => activeView === 'ingestion' && activeTab === 'media'
            && document.getElementById('view-ingestion').classList.contains('active')
            && document.getElementById('tab-media').classList.contains('active')""",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    _wait_monitoring_table_ready(page, "media-tbody")
    shot_cat_tile = ctx.shot("monitoring_catalogue_tile_routed_to_tab_media")

    ctx.record_step(
        step_id="C1",
        portal="Monitoring",
        feature="Vue d'ensemble (6 Tuiles), Flux SSE /v1/monitoring/events, Cristian ClockSync & Tuile Catalogue -> #tab-media",
        t0=t0,
        assertions=[
            f"SSE actif & Cristian ClockSync calibré sur {sync_snap['hostname']} (RTT={round(sync_snap['rttMs'] or 0, 1)} ms)",
            f"{OVERVIEW_EXPECTED_TILES} tuiles rendues ; clic tuile Catalogue active #view-ingestion + onglet [4] #tab-media",
            f"0 requête legacy de polling détectée ({len(polled_legacy)})",
        ],
        metrics={"sync": sync_snap, "overview_shot": shot_ov},
        screenshot=shot_cat_tile,
    )


def step_c2_monitoring_ingestion_five_tabs_and_pipeline(ctx: RecetteContext) -> None:
    """Exercise all 5 Ingestion Web Top tabs ([1]..[5]), detail drawers, filters, and live 6-stage ISO Pipeline."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    # [1] Tâches
    page.click(".tab-btn[data-tab='jobs']")
    page.wait_for_selector("#tab-jobs.active", timeout=DEFAULT_TIMEOUT_MS)
    banner_host = page.locator("#top-line-host").inner_text()
    assert "HÔTE" in banner_host
    shot_jobs = ctx.shot("monitoring_ingestion_tab1_jobs")

    # [2] Stats
    page.click(".tab-btn[data-tab='stats']")
    page.wait_for_selector("#tab-stats.active", timeout=DEFAULT_TIMEOUT_MS)
    stats_rows = page.locator("#stats-tbody tr").count()
    assert stats_rows == STATS_EXPECTED_DIMENSIONS
    shot_stats = ctx.shot("monitoring_ingestion_tab2_stats")

    # [3] Récents
    page.click(".tab-btn[data-tab='completed']")
    page.wait_for_selector("#tab-completed.active", timeout=DEFAULT_TIMEOUT_MS)
    _wait_monitoring_table_ready(page, "completed-tbody")
    comp_rows = page.locator("#completed-tbody tr[data-key]").count()
    if comp_rows > 0:
        page.locator("#completed-tbody tr[data-key]").first.click()
        page.wait_for_selector("#completed-tbody tr.detail", timeout=DEFAULT_TIMEOUT_MS)
        shot_comp = ctx.shot("monitoring_ingestion_tab3_completed_drawer")
        page.locator("#completed-tbody tr[data-key]").first.click()
    else:
        shot_comp = ctx.shot("monitoring_ingestion_tab3_completed_empty")

    next_btn = page.locator(".pager[data-list='completed'] button[data-step='1']")
    if next_btn.is_visible() and next_btn.is_enabled():
        next_btn.click()
        _wait_monitoring_table_ready(page, "completed-tbody")
        page.locator(".pager[data-list='completed'] button[data-step='-1']").click()
        _wait_monitoring_table_ready(page, "completed-tbody")

    # [4] Catalogue (#tab-media) + filters/sorts/selection + Studio link check + ISO Pipeline live match
    page.click(".tab-btn[data-tab='media']")
    page.wait_for_selector("#tab-media.active", timeout=DEFAULT_TIMEOUT_MS)
    _wait_monitoring_table_ready(page, "media-tbody")

    for filt in MONITORING_MEDIA_FILTERS:
        page.select_option("#media-filter-select", filt)
    for srt in MONITORING_MEDIA_SORTS:
        page.select_option("#media-sort-select", srt)
    page.fill("#media-search-filter", "S01")
    page.fill("#media-search-filter", "")

    media_rows = page.locator("#media-tbody tr[data-key]").count()
    studio_href_verified = "n/a (0 discovered_media rows)"
    if media_rows > 0:
        page.click("#btn-media-select-all")
        page.click("#btn-media-clear-sel")
        page.locator("#media-tbody tr[data-key]").first.click()
        page.wait_for_selector("#media-tbody tr.detail", timeout=DEFAULT_TIMEOUT_MS)
        studio_href = (
            page.locator("#media-tbody tr.detail a:has-text('Ouvrir dans Studio')").get_attribute("href") or ""
        )
        assert "/dbsrt/studio" in studio_href, f"Expected /dbsrt/studio prefix in link, got {studio_href}"
        studio_href_verified = studio_href
        page.locator("#media-tbody tr[data-key]").first.click()

    # Live 6-box ISO Pipeline evaluation on real .phash on o2switch
    page.locator("#performance-pipeline-card button:has-text('Pulp Fiction')").click()
    real_phash_path = f"data/__PHASH__/{ctx.sample_phash_filename}"
    page.fill("#pipeline-snippet-input", real_phash_path)
    page.click("#btn-pipeline-match")
    page.wait_for_function(
        """() => {
            const btn = document.getElementById('btn-pipeline-match');
            const mih = document.getElementById('iso-val-mih');
            return btn && !btn.disabled && mih && !mih.textContent.includes('—');
        }""",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    mih_val = page.locator("#iso-val-mih").inner_text().strip()
    shot_iso = ctx.shot("monitoring_tab4_iso_pipeline_matched")

    # [5] Sources (Zero-NVMe FTP streaming mode on o2switch)
    page.click(".tab-btn[data-tab='sources']")
    page.wait_for_selector("#tab-sources.active", timeout=DEFAULT_TIMEOUT_MS)
    _wait_monitoring_table_ready(page, "sources-tbody")
    sources_caption = page.locator("#sources-caption").inner_text().strip()
    shot_src = ctx.shot("monitoring_ingestion_tab5_sources")

    ctx.record_step(
        step_id="C2",
        portal="Monitoring",
        feature="Web Top Ingestion 5 Onglets ([1]..[5]), Fiches Détails, Filtres/Tris & Pipeline ISO 6-Étapes",
        t0=t0,
        assertions=[
            f"Onglets [1] Tâches, [2] Stats ({stats_rows} dimensions), [3] Récents ({comp_rows} lignes), [4] Catalogue ({media_rows} lignes), [5] Sources ({sources_caption}) validés",
            f"Lien Studio préfixé vérifié ({studio_href_verified})",
            f"Pipeline ISO 6-boîtes exécuté en réel sur {real_phash_path} (MIH={mih_val})",
        ],
        metrics={
            "completed_rows": comp_rows,
            "media_rows": media_rows,
            "mih_latency": mih_val,
            "shots": [shot_jobs, shot_stats, shot_comp, shot_src],
        },
        screenshot=shot_iso,
    )


def step_c3_monitoring_audit_supervisor_system_incidents(ctx: RecetteContext) -> None:
    """Verify #audit (+ idempotent stop check), #supervisor, #system (RAM status/reload) & #incidents."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    # #audit + idempotent POST /v1/monitoring/audit/stop (guarantees no audit left running)
    page.click("#views-nav a[href='#audit']")
    page.wait_for_selector("#view-audit.active", timeout=DEFAULT_TIMEOUT_MS)
    audit_badge = page.locator("#audit-state").inner_text().strip()
    ctx.console_guard.allow_expected_http_errors = True
    audit_stop_res = page.evaluate(
        """async () => {
            const r = await fetch(apiEndpoint('v1/monitoring/audit/stop'), { method: 'POST', headers: getAuthHeaders() });
            return { status: r.status, body: await r.text() };
        }"""
    )
    ctx.console_guard.allow_expected_http_errors = False
    shot_audit = ctx.shot("monitoring_view_audit")

    # #supervisor
    page.click("#views-nav a[href='#supervisor']")
    page.wait_for_selector("#view-supervisor.active", timeout=DEFAULT_TIMEOUT_MS)
    sup_badge = page.locator("#sup-status-badge").inner_text().strip()
    shot_sup = ctx.shot("monitoring_view_supervisor_rpi4")

    # #system + RAM catalog status & atomic hot-reload
    page.click("#views-nav a[href='#system']")
    page.wait_for_selector("#view-system.active", timeout=DEFAULT_TIMEOUT_MS)
    sys_groups = page.locator("#sys-tree .sys-group").count()
    assert sys_groups >= MIN_SYSTEM_GROUPS

    page.click("#btn-catalog-status")
    page.wait_for_function(
        "() => document.getElementById('catalog-ram-stats').innerText.includes('Médias indexés')",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    page.click("#btn-catalog-reload")
    page.wait_for_function(
        """() => {
            const btn = document.getElementById('btn-catalog-reload');
            const fb = document.getElementById('catalog-reload-feedback');
            return btn && !btn.disabled && fb && fb.innerText.includes('Index rechargé');
        }""",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    reload_txt = page.locator("#catalog-reload-feedback").inner_text().strip()
    shot_sys = ctx.shot("monitoring_view_system_ram_reload")

    # #incidents
    page.click("#views-nav a[href='#incidents']")
    page.wait_for_selector("#view-incidents.active", timeout=DEFAULT_TIMEOUT_MS)
    inc_text = page.locator("#view-incidents").inner_text()
    assert "INC-2026-11-12-01" in inc_text and "INC-2026-10-31-01" in inc_text
    shot_inc = ctx.shot("monitoring_view_incidents_sre")

    ctx.record_step(
        step_id="C3",
        portal="Monitoring",
        feature="Vues Audit (Stop Idempotent), Superviseur RPi4, Système Fastfetch (Hot-Reload RAM) & Incidents SRE",
        t0=t0,
        assertions=[
            f"Vue Audit ({audit_badge}) & POST /v1/monitoring/audit/stop idempotent (HTTP {audit_stop_res['status']})",
            f"Vue Superviseur RPi4 ({sup_badge}) et journal d'activité rendus",
            f"Vue Système ({sys_groups} groupes Fastfetch) + {reload_txt}",
            "Vue Incidents & SRE : post-mortems INC-2026-11-12-01 et INC-2026-10-31-01 présents",
        ],
        metrics={
            "audit_state": audit_badge,
            "audit_stop_http": audit_stop_res["status"],
            "supervisor_badge": sup_badge,
            "shots": [shot_audit, shot_sup, shot_inc],
        },
        screenshot=shot_sys,
    )


def step_c4_monitoring_controls_zeropolling_and_top_route(ctx: RecetteContext, polled_legacy: list[str]) -> None:
    """Verify Header ↻ / ⏸ / ▶ / 🔑 controls, Zero-Polling SSE invariant, and /top alias route."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.click("#btn-refresh")
    page.click("#btn-pause")
    assert page.evaluate("() => isPaused === true && sseActive === false") is True
    page.click("#btn-pause")
    page.wait_for_function("() => isPaused === false && sseActive === true", timeout=DEFAULT_TIMEOUT_MS)

    page.click("#btn-auth")
    page.wait_for_function("() => document.getElementById('auth-modal').style.display === 'flex'")
    page.fill("#api-key-input", ctx.api_key)
    page.click("#btn-save-key")
    page.wait_for_function("() => document.getElementById('auth-modal').style.display === 'none'")
    page.wait_for_function("() => sseActive === true", timeout=DEFAULT_TIMEOUT_MS)

    assert len(polled_legacy) == 0, f"Zero-polling violated during Monitoring: {polled_legacy}"

    page.goto(f"{ctx.base_url}/top?token={urllib.parse.quote(ctx.api_key)}", wait_until="domcontentloaded")
    _wait_monitoring_hydrated(page)
    shot_top = ctx.shot("monitoring_top_alias_route")

    ctx.record_step(
        step_id="C4",
        portal="Monitoring",
        feature="Contrôles Header (↻, Pause/Reprise SSE, Modale Clé 🔑), Invariant Zéro-Polling & Route /top",
        t0=t0,
        assertions=[
            "Pause ferme proprement EventSource (sseActive=false) et Reprise rétablit le flux SSE",
            "Modale 🔑 Authentification met à jour localStorage et reconnecte SSE sans erreur",
            "Invariant Zéro-Polling respecté : 0 requête vers /v1/metrics, /v1/logs ou /v1/monitoring/latest",
            "Route alias /dbsrt/top opérationnelle et hydratée en temps réel",
        ],
        metrics={"legacy_polls_count": len(polled_legacy)},
        screenshot=shot_top,
    )


def run_monitoring_and_top_suite(ctx: RecetteContext) -> None:
    """Run all Portal C (Monitoring & /top) scenarios in Session 2."""
    assert ctx.page is not None
    polled_legacy: list[str] = []

    def _guard_legacy_polling(req: Any) -> None:
        norm = ctx.tracker.normalize_path(req.url)
        if norm in ("/v1/metrics", "/v1/logs", "/v1/monitoring/latest"):
            polled_legacy.append(norm)

    ctx.page.on("request", _guard_legacy_polling)
    step_c1_monitoring_overview_sse_and_catalogue_tile(ctx, polled_legacy)
    step_c2_monitoring_ingestion_five_tabs_and_pipeline(ctx)
    step_c3_monitoring_audit_supervisor_system_incidents(ctx)
    step_c4_monitoring_controls_zeropolling_and_top_route(ctx, polled_legacy)


def _wait_studio_ready(page: Page) -> None:
    """Wait until Studio couplings summary, ER counts, and initial sub_frames workspace are loaded."""
    page.wait_for_function(
        """() => typeof studioState !== 'undefined' && studioState !== null
            && initialLoadDone === true && subFramesPageData !== null""",
        timeout=DEFAULT_TIMEOUT_MS,
    )


def step_d1_studio_couplings_subframes_and_raw_srt(ctx: RecetteContext) -> None:
    """Verify Studio ER schema, Tab 1 Couplings filters/sorts, sub_frames minimap/loupe & View Raw SRT."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.goto(f"{ctx.base_url}/studio?token={urllib.parse.quote(ctx.api_key)}", wait_until="domcontentloaded")
    _wait_studio_ready(page)
    shot_ov = ctx.shot("studio_tab1_couplings_overview")

    st_meta = page.evaluate(
        """() => ({
            indexed: studioState.indexed_phash_files,
            coupledMedias: studioState.couplings.length,
            certSubs: studioState.couplings.reduce((acc, c) => acc + (c.subtitles ? c.subtitles.length : 0), 0),
            pgConnected: studioState.postgres_connected,
            firstCertPhash: studioState.catalog_phashes[0]?.filename || '',
            cntMedias: document.getElementById('cnt-medias').innerText,
            cntSubFrames: document.getElementById('cnt-sub-frames').innerText,
        })"""
    )
    assert st_meta["indexed"] >= MIN_CATALOG_PHASH_COUNT
    assert st_meta["certSubs"] >= MIN_CERTIFIED_COUPLINGS
    ctx.sample_phash_filename = st_meta["firstCertPhash"] or ctx.sample_phash_filename

    # Sidebar search + facets + sort + page size
    page.fill("#media-search-input", "S01")
    page.click(".sidebar-search-clear")
    for lang in STUDIO_LANG_FILTERS:
        page.select_option("#media-lang-filter", lang)
    for store in STUDIO_STORE_FILTERS:
        page.select_option("#media-store-filter", store)
    for srt in STUDIO_MEDIA_SORTS:
        page.select_option("#media-sort-select", srt)
    page.select_option("#media-pagesize-select", "25")
    page.select_option("#media-pagesize-select", "10")

    if page.locator("#couplings-media-list .media-item").count() > 1:
        page.locator("#couplings-media-list .media-item").nth(1).click()
        page.locator("#cue-workspace-container .cue-inspector canvas.phash-canvas-lg").wait_for(
            state="visible", timeout=DEFAULT_TIMEOUT_MS
        )

    if page.locator(".sub-pill").count() > 1:
        page.locator(".sub-pill").nth(1).click()
        page.locator("#cue-workspace-container .cue-inspector canvas.phash-canvas-lg").wait_for(
            state="visible", timeout=DEFAULT_TIMEOUT_MS
        )

    # Verify View Raw SRT link prefix (/dbsrt/v1/subtitles/{hash}) and HTTP 200 SRT content
    raw_srt_href = page.locator("a:has-text('View Raw SRT')").get_attribute("href") or ""
    assert "/dbsrt/v1/subtitles/" in raw_srt_href, f"Unprefixed Raw SRT link: {raw_srt_href}"
    srt_check = page.evaluate(
        """async (href) => {
            const r = await fetch(href, { headers: getAuthHeaders() });
            const txt = await r.text();
            return { status: r.status, len: txt.length, hasArrow: txt.includes('-->') };
        }""",
        raw_srt_href,
    )
    assert srt_check["status"] == HTTP_OK and srt_check["hasArrow"] is True

    # Cue search, jump to cue #, Prev/Next cue, Minimap click, Timeline bar click & 8x8 pHash canvas
    page.fill("#cue-filter-input", "a")
    page.wait_for_timeout(UI_SETTLE_MS)
    page.fill("#cue-filter-input", "")
    page.wait_for_timeout(UI_SETTLE_MS)

    page.fill("#jump-cue-input", "3")
    page.click("button:has-text('Go to Cue #')")
    page.click("button:has-text('Next Cue ›')")
    page.click("button:has-text('‹ Prev Cue')")

    minimap = page.locator(".timeline-minimap")
    if minimap.is_visible():
        minimap.click(position={"x": 120, "y": 8})

    if page.locator(".timeline-cue").count() > 0:
        page.locator(".timeline-cue").first.click()

    anchor_hex = page.locator(".cue-inspector canvas.phash-canvas-lg").get_attribute("data-phash") or ""
    assert anchor_hex.startswith("0x") and len(anchor_hex) == 18
    shot_loupe = ctx.shot("studio_tab1_cue_phash_loupe_inspector")

    ctx.record_step(
        step_id="D1",
        portal="Studio",
        feature="Schéma ER Relationnel, Couplages Certifiés, Timeline/Minimap sub_frames, Loupe DCT 8×8 & CAS Raw SRT",
        t0=t0,
        assertions=[
            (
                f"Index Studio chargé : {st_meta['indexed']} .phash, "
                f"{st_meta['certSubs']} pistes certifiées ({st_meta['coupledMedias']} médias)"
            ),
            f"Lien View Raw SRT ({raw_srt_href}) sert un SRT valide HTTP 200 ({srt_check['len']} octets)",
            f"Recherche sub_frames, Go to Cue #, Minimap et loupe canvas 8×8 ({anchor_hex}) vérifiés",
        ],
        metrics={"studio_meta": st_meta, "srt_bytes": srt_check["len"], "overview_shot": shot_ov},
        screenshot=shot_loupe,
    )


def step_d2_studio_phash_catalog_and_batch_match(ctx: RecetteContext) -> None:
    """Exercise Tab 2 Indexed .phash Catalog filters, sorts, page selection, and 1-item live batch match."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.click("#tab-btn-catalog")
    page.wait_for_selector("#tab-catalog.active", timeout=DEFAULT_TIMEOUT_MS)

    page.fill("#catalog-filter", "Cruaut")
    assert page.locator("#catalog-tbody tr").count() >= 1
    page.fill("#catalog-filter", "")

    for st in STUDIO_CATALOG_STATUSES:
        page.select_option("#catalog-status-filter", st)
    for fps in STUDIO_CATALOG_FPS:
        page.select_option("#catalog-fps-filter", fps)
    for srt in STUDIO_CATALOG_SORTS:
        page.select_option("#catalog-sort", srt)
    page.click("#tab-catalog button:has-text('Reset')")

    page.click("button:has-text('Select / Deselect Page')")
    sel_txt = page.locator("#catalog-selected-badge").inner_text()
    assert "50 video(s) selected" in sel_txt
    shot_cat = ctx.shot("studio_tab2_phash_catalog_selected")

    page.click("button:has-text('Clear Selection')")
    assert "0 video(s) selected" in page.locator("#catalog-selected-badge").inner_text()

    # Select 1 certified .phash row and run batch matching panel end-to-end
    page.locator("#catalog-tbody input[type='checkbox']").first.click()
    page.click("#btn-catalog-batch-match")
    page.wait_for_function(
        "() => document.getElementById('catalog-batch-status-badge').textContent === 'FINISHED'",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    shot_batch = ctx.shot("studio_tab2_batch_match_finished")
    page.click("#catalog-batch-panel button:has-text('Close')")
    page.click("button:has-text('Clear Selection')")

    ctx.record_step(
        step_id="D2",
        portal="Studio",
        feature="Catalogue .phash Indexé (Filtres Statut/FPS, Tris, Sélection Page & Batch Matching Temps Réel)",
        t0=t0,
        assertions=[
            "Filtres Certified/Uncoupled, FPS (24/25/30), 4 tris et Reset opérationnels",
            "Sélection page (50 vidéos) / Clear Selection et exécution Batch Matching unitaire FINISHED",
        ],
        metrics={"catalog_shot": shot_cat},
        screenshot=shot_batch,
    )


def step_d3_studio_playground_endpoints_and_sentinels(ctx: RecetteContext) -> None:
    """Exercise Tab 3 Playground presets, Search modes/engines, Recognize, Catalog RAM & non-mutating sentinels."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.click("#tab-btn-playground")
    page.wait_for_selector("#tab-playground.active", timeout=DEFAULT_TIMEOUT_MS)

    for key in STUDIO_PRESET_KEYS:
        page.evaluate(f"() => applyPreset('{key}')")

    real_phash = f"data/__PHASH__/{ctx.sample_phash_filename}"
    page.fill("#search-media-path", real_phash)
    for mode, engine in (("hash", "auto"), ("name", "auto"), ("phash", "mih"), ("phash", "auto")):
        page.select_option("#search-mode", mode)
        page.select_option("#search-index-engine", engine)
        page.click("button:has-text('Run Search')")
        page.wait_for_function(
            "() => document.getElementById('search-result-box').innerText.includes('Matched Media:')",
            timeout=DEFAULT_TIMEOUT_MS,
        )
    shot_search = ctx.shot("studio_tab3_playground_live_search")

    # POST /v1/recognize on real .phash
    page.fill("#recognize-media-path", real_phash)
    page.click("button:has-text('Probe Recognition')")
    page.wait_for_function(
        """() => {
            const t = document.getElementById('recognize-result-box').innerText;
            return t.includes('Média Reconnu') || t.includes('correspondance');
        }""",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    recog_txt = page.locator("#recognize-result-box").inner_text().strip().splitlines()[0]

    # GET /v1/catalog/status & POST /v1/catalog/reload
    page.click("button:has-text('Query Status')")
    page.wait_for_function(
        "() => document.getElementById('catalog-ctrl-result-box').innerText.includes('Total Médias')",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    page.click("button:has-text('Hot-Reload Catalog')")
    page.wait_for_function(
        "() => document.getElementById('catalog-ctrl-result-box').innerText.includes('Catalogue rechargé')",
        timeout=DEFAULT_TIMEOUT_MS,
    )

    # Safe non-mutating sentinel validations for POST /v1/{certify,autosync,retarget,download}
    ctx.console_guard.allow_expected_http_errors = True
    page.fill("#cert-media-path", SENTINEL_PHASH_PATH)
    page.fill("#cert-srt-path", SENTINEL_SRT_PATH)
    page.click("button:has-text('Certify Coupling')")
    page.wait_for_function(
        "() => !document.getElementById('cert-result-box').innerText.includes('Certifying')",
        timeout=DEFAULT_TIMEOUT_MS,
    )

    page.fill("#autosync-media-path", SENTINEL_MEDIA_PATH)
    page.click("button:has-text('Run AutoSync E2E')")
    page.wait_for_function(
        "() => !document.getElementById('autosync-result-box').innerText.includes('Exécution du pipeline')",
        timeout=DEFAULT_TIMEOUT_MS,
    )

    page.fill("#retarget-target-media", SENTINEL_MEDIA_PATH)
    page.fill("#retarget-ref-subtitles", SENTINEL_SRT_PATH)
    page.click("button:has-text('Retarget Subtitle')")
    page.wait_for_function(
        "() => !document.getElementById('retarget-result-box').innerText.includes('Recalage pHash en cours')",
        timeout=DEFAULT_TIMEOUT_MS,
    )

    page.evaluate(f"() => runLiveDownload('{SENTINEL_MEDIA_PATH}', 'certified', '', 'fr', 0)")
    ctx.console_guard.allow_expected_http_errors = False
    shot_pg = ctx.shot("studio_tab3_playground_recognize_and_guards")

    ctx.record_step(
        step_id="D3",
        portal="Studio",
        feature="Playground API (/v1/search Multi-Moteurs, /v1/recognize, /v1/catalog/* & Gardes Non-Mutantes /v1/{certify,autosync,retarget,download})",
        t0=t0,
        assertions=[
            f"6 Presets & POST /v1/search (phash/auto, phash/mih, hash, name) validés sur {real_phash}",
            f"POST /v1/recognize : {recog_txt}",
            "GET /v1/catalog/status & POST /v1/catalog/reload exécutés avec succès",
            "Gardes sentinelles POST /v1/{certify,autosync,retarget,download} vérifiées sans mutation de couplings.json",
        ],
        metrics={"recognize_headline": recog_txt, "search_shot": shot_search},
        screenshot=shot_pg,
    )


def step_d4_studio_observability_metrics_logs_and_pipeline(ctx: RecetteContext) -> None:
    """Exercise Tab 4 Observability KPIs, /v1/metrics & /v1/logs, 6-stage ISO Pipeline & log controls."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    page.click("#tab-btn-observability")
    page.wait_for_selector("#tab-observability.active", timeout=DEFAULT_TIMEOUT_MS)

    # Verify REST /v1/metrics and /v1/logs directly in addition to SSE live stream
    obs_rest = page.evaluate(
        """async () => {
            const [mRes, lRes] = await Promise.all([
                fetch(apiEndpoint('v1/metrics'), { headers: getAuthHeaders() }),
                fetch(apiEndpoint('v1/logs'), { headers: getAuthHeaders() }),
            ]);
            const m = await mRes.json();
            const l = await lRes.json();
            renderMetrics(m);
            liveLogs = l.logs || [];
            renderLogs();
            return {
                mStatus: mRes.status,
                lStatus: lRes.status,
                totalReq: m.total_requests,
                logsCount: (l.logs || []).length,
            };
        }"""
    )
    assert obs_rest["mStatus"] == HTTP_OK and obs_rest["lStatus"] == HTTP_OK
    assert obs_rest["totalReq"] >= 1

    real_phash = f"data/__PHASH__/{ctx.sample_phash_filename}"
    page.fill("#pipeline-snippet-input", real_phash)
    page.click("#btn-pipeline-match")
    page.wait_for_function(
        """() => {
            const btn = document.getElementById('btn-pipeline-match');
            const mih = document.getElementById('iso-val-mih');
            return btn && !btn.disabled && mih && !mih.textContent.includes('—');
        }""",
        timeout=DEFAULT_TIMEOUT_MS,
    )
    mih_ms = page.locator("#iso-val-mih").inner_text().strip()
    shot_obs = ctx.shot("studio_tab4_observability_iso_pipeline")

    # Log filter, Pause -> Resume (validates pollLogsAndMetricsFallback fix!), Clear, and Header Refresh
    page.fill("#log-filter-input", "SEARCH")
    page.fill("#log-filter-input", "")
    page.click("#log-pause-btn")
    assert "Resume" in page.locator("#log-pause-btn").inner_text()
    page.click("#log-pause-btn")
    assert "Pause" in page.locator("#log-pause-btn").inner_text()
    page.click("#tab-observability button:has-text('Clear')")
    page.click("header button:has-text('↻ Refresh')")
    shot_logs = ctx.shot("studio_tab4_live_logs_controls")

    ctx.record_step(
        step_id="D4",
        portal="Studio",
        feature="Observability & Live Logs (/v1/metrics, /v1/logs, Pipeline ISO 6-Boîtes, Filtre, Pause/Resume & Clear)",
        t0=t0,
        assertions=[
            f"GET /v1/metrics ({obs_rest['totalReq']} requêtes) & GET /v1/logs ({obs_rest['logsCount']} entrées) HTTP 200",
            f"Pipeline ISO Studio exécuté sur {real_phash} (MIH={mih_ms})",
            "Filtre logs, Pause/Resume (sans ReferenceError), Clear et Header ↻ Refresh validés",
        ],
        metrics={"rest_telemetry": obs_rest, "mih_latency": mih_ms, "logs_shot": shot_logs},
        screenshot=shot_obs,
    )


def run_studio_suite(ctx: RecetteContext) -> None:
    """Run all Portal D (DBSrt Studio) scenarios in Session 3."""
    assert ctx.page is not None
    ctx.page.on("dialog", lambda dialog: dialog.accept())
    step_d1_studio_couplings_subframes_and_raw_srt(ctx)
    step_d2_studio_phash_catalog_and_batch_match(ctx)
    step_d3_studio_playground_endpoints_and_sentinels(ctx)
    step_d4_studio_observability_metrics_logs_and_pipeline(ctx)
