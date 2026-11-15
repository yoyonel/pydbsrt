# pyright: reportMissingImports=false
"""Portal A (Gateway Security) & Portal B Part 1 (Sources & Explorer) Playwright scenarios."""

from __future__ import annotations

import hashlib
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import TYPE_CHECKING, Any

from scripts.e2e.recette_o2switch.harness import (
    DEFAULT_TIMEOUT_MS,
    DESKTOP_USER_AGENT,
    EXPECTED_FREEBOX_CONTAINERS,
    FFPROBE_TIMEOUT_MS,
    HTTP_BAD_REQUEST,
    HTTP_FORBIDDEN,
    HTTP_NO_CONTENT,
    HTTP_NOT_ACCEPTABLE,
    HTTP_NOT_FOUND,
    HTTP_OK,
    HTTP_UNAUTHORIZED,
    MIN_CATALOG_PHASH_COUNT,
    MIN_SYNOLOGY_TVSHOW_ITEMS,
    MIN_SYNOLOGY_VIDEO_ITEMS,
    PROBE_TIMEOUT_MS,
    UI_SETTLE_MS,
    RecetteContext,
)

if TYPE_CHECKING:
    from playwright.sync_api import Page

SHA256_HEX_LEN: int = 64
EXPLORER_SORT_MODES: tuple[str, ...] = ("name_desc", "size_desc", "size_asc", "date_desc", "name_asc")
EXPLORER_LIMIT_MODES: tuple[str, ...] = ("50", "100", "25")
EXPLORER_QUICK_TAGS: tuple[str, ...] = ("4K / 2160p", "1080p", "HEVC", "MKV", "Tous")
ENV_SWITCH_SEQUENCE: tuple[str, ...] = ("all", "local", "nomad", "rpi4", "o2switch")
MAX_DRILLDOWN_DEPTH: int = 4


def _http_raw_request(url: str, method: str, api_key: str = "") -> tuple[int, dict[str, str], bytes]:
    """Execute a direct HTTP request for protocol-level checks (OPTIONS, HEAD, binary SHA256)."""
    headers = {"User-Agent": DESKTOP_USER_AGENT}
    if api_key:
        headers["X-API-Key"] = api_key
    req = urllib.request.Request(url, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            hdrs = {k.lower(): v for k, v in resp.headers.items()}
            return resp.status, hdrs, resp.read()
    except urllib.error.HTTPError as err:
        hdrs = {k.lower(): v for k, v in err.headers.items()} if err.headers else {}
        return err.code, hdrs, err.read()


def _wait_sources_probes_done(page: Page) -> None:
    """Wait deterministically until Alpine probingAny completes."""
    page.wait_for_function(
        """() => {
            const d = window.Alpine ? Alpine.$data(document.body) : null;
            return d && d.sources && d.sources.length > 0 && d.probingAny === false;
        }""",
        timeout=PROBE_TIMEOUT_MS,
    )


def _wait_explorer_ready(page: Page) -> None:
    """Wait until Explorer view has finished loading explorerData."""
    page.wait_for_function(
        """() => {
            const d = window.Alpine ? Alpine.$data(document.body) : null;
            return d && d.currentView === 'explorer' && d.explorerLoading === false && d.explorerData !== null;
        }""",
        timeout=PROBE_TIMEOUT_MS,
    )


def _drill_to_video_container(page: Page) -> dict[str, Any]:
    """Drill into subdirectories up to MAX_DRILLDOWN_DEPTH until a video file is listed."""
    for _ in range(MAX_DRILLDOWN_DEPTH):
        state = page.evaluate(
            """() => {
                const d = Alpine.$data(document.body);
                const files = d.explorerFiles || [];
                const vidIdx = files.findIndex(f => !f.is_dir);
                const dirIdx = files.findIndex(f => f.is_dir);
                return { vidIdx, dirIdx, count: files.length, subdir: d.explorerSubdir };
            }"""
        )
        if state["vidIdx"] >= 0:
            return state
        assert state["dirIdx"] >= 0, f"No video or directory found in {state['subdir']}"
        page.locator("[id^='item-']").nth(state["dirIdx"]).click()
        _wait_explorer_ready(page)
    raise AssertionError("Exceeded MAX_DRILLDOWN_DEPTH without finding a video container")


def step_a1_gateway_auth_flows(ctx: RecetteContext) -> None:
    """Verify anonymous 401 login page, invalid key error, form POST login, and /logout."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    resp = page.goto(f"{ctx.base_url}/monitoring", wait_until="domcontentloaded")
    assert resp is not None and resp.status in (HTTP_OK, HTTP_UNAUTHORIZED)
    page.wait_for_selector("form input[name='api_key']", timeout=DEFAULT_TIMEOUT_MS)
    shot_login = ctx.shot("gateway_login_anonymous_401")

    page.fill("input[name='api_key']", "invalid-recette-key-0000")
    page.click("button[type='submit']")
    page.wait_for_selector(".err-box", timeout=DEFAULT_TIMEOUT_MS)
    err_text = page.locator(".err-box").inner_text().strip()
    assert "invalide" in err_text.lower()
    shot_err = ctx.shot("gateway_login_invalid_key")

    page.fill("input[name='api_key']", ctx.api_key)
    page.click("button[type='submit']")
    page.wait_for_selector("#views-nav", timeout=DEFAULT_TIMEOUT_MS)
    assert "dbsrt Monitor" in page.title()
    shot_ok = ctx.shot("gateway_login_form_authenticated")

    ctx.record_step(
        step_id="A1",
        portal="Gateway",
        feature="Authentification HTML (401 Anonyme -> Clé Invalide -> Login Cookie POST)",
        t0=t0,
        assertions=[
            "Formulaire HTML 401 affiché sans clé",
            f"Rejet clé invalide avec bandeau .err-box ({err_text})",
            "POST valide établit le cookie HttpOnly dbsrt_session et ouvre dbsrt Monitor",
        ],
        metrics={"login_shot": shot_login, "invalid_shot": shot_err},
        screenshot=shot_ok,
    )


def step_a2_gateway_protocols_and_artifacts(ctx: RecetteContext) -> None:
    """Verify CORS OPTIONS 204, favicon SVG, status sanitization, artifact SHA256, guards, and logout."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    opt_code, opt_hdrs, _ = _http_raw_request(f"{ctx.base_url}/v1/health", "OPTIONS")
    ctx.tracker.record_manual("OPTIONS", f"{ctx.base_url}/v1/health", opt_code)
    assert opt_code == HTTP_NO_CONTENT, f"Expected 204 on OPTIONS, got {opt_code}"
    assert opt_hdrs.get("access-control-allow-origin") == "*"

    checks = page.evaluate(
        """async (base) => {
            const [fav, h1, h2, st, ph] = await Promise.all([
                fetch(`${base}/favicon.ico`).then(async r => ({ status: r.status, body: await r.text() })),
                fetch(`${base}/health`).then(async r => ({ status: r.status, json: await r.json() })),
                fetch(`${base}/v1/health`).then(async r => ({ status: r.status, json: await r.json() })),
                fetch(`${base}/v1/status`).then(async r => ({ status: r.status, json: await r.json() })),
                fetch(`${base}/api/phashes?limit=5`).then(async r => ({ status: r.status, json: await r.json() })),
            ]);
            return { fav, h1, h2, st, ph };
        }""",
        ctx.base_url,
    )
    assert checks["fav"]["status"] == HTTP_OK and "<svg" in checks["fav"]["body"]
    ctx.tracker.record_manual("GET", f"{ctx.base_url}/favicon.ico", int(checks["fav"]["status"]))
    assert checks["h1"]["status"] == HTTP_OK and checks["h2"]["status"] == HTTP_OK
    st_json = checks["st"]["json"]
    assert checks["st"]["status"] == HTTP_OK
    assert st_json.get("indexed_phash_count", 0) >= MIN_CATALOG_PHASH_COUNT
    assert "phash_dir" not in st_json and "subtitles_dir" not in st_json

    items = checks["ph"]["json"].get("items", [])
    raw_candidates = [it["raw_filename"] for it in items if it.get("raw_present") and it.get("raw_filename")]
    assert raw_candidates, "Expected at least one raw .phash filename from /api/phashes"
    ctx.sample_phash_filename = raw_candidates[0]

    encoded_file = urllib.parse.quote(ctx.sample_phash_filename)
    art_url = f"{ctx.base_url}/api/artifact/download?kind=phash&file={encoded_file}"
    head_code, head_hdrs, _ = _http_raw_request(art_url, "HEAD", ctx.api_key)
    ctx.tracker.record_manual("HEAD", art_url, head_code)
    assert head_code == HTTP_OK and len(head_hdrs.get("x-checksum-sha256", "")) == SHA256_HEX_LEN

    get_code, get_hdrs, raw_bytes = _http_raw_request(art_url, "GET", ctx.api_key)
    ctx.tracker.record_manual("GET", art_url, get_code)
    computed_sha = hashlib.sha256(raw_bytes).hexdigest()
    assert get_code == HTTP_OK and computed_sha == get_hdrs.get("x-checksum-sha256")

    coup_url = f"{ctx.base_url}/api/artifact/download?kind=couplings"
    coup_code, coup_hdrs, coup_bytes = _http_raw_request(coup_url, "GET", ctx.api_key)
    ctx.tracker.record_manual("GET", coup_url, coup_code)
    assert (
        coup_code == HTTP_OK and len(coup_bytes) > 100 and len(coup_hdrs.get("x-checksum-sha256", "")) == SHA256_HEX_LEN
    )

    ctx.console_guard.allow_expected_http_errors = True
    guards = page.evaluate(
        """async (base) => {
            const [trav, waf, miss] = await Promise.all([
                fetch(`${base}/api/artifact/download?kind=phash&file=../traversal.phash`).then(r => r.status),
                fetch(`${base}/api/artifact/download?kind=phash&file=../etc/passwd`).then(r => r.status),
                fetch(`${base}/api/artifact/download?kind=phash&file=__nonexistent_recette__.phash`).then(r => r.status),
            ]);
            return { trav, waf, miss };
        }""",
        ctx.base_url,
    )
    ctx.console_guard.allow_expected_http_errors = False
    assert guards["trav"] == HTTP_BAD_REQUEST, f"Expected 400 on PHP path traversal, got {guards}"
    assert guards["waf"] in (HTTP_BAD_REQUEST, HTTP_FORBIDDEN, HTTP_NOT_ACCEPTABLE), (
        f"Expected 400/403/406 on /etc/passwd LFI probe, got {guards}"
    )
    assert guards["miss"] == HTTP_NOT_FOUND

    page.goto(f"{ctx.base_url}/logout", wait_until="domcontentloaded")
    page.wait_for_selector("form input[name='api_key']", timeout=DEFAULT_TIMEOUT_MS)
    shot_logout = ctx.shot("gateway_logout_revoked")

    page.goto(f"{ctx.base_url}/sources?token={urllib.parse.quote(ctx.api_key)}", wait_until="domcontentloaded")
    page.wait_for_selector("h1:has-text('Sources Médias : Accès Réseau & Disques')", timeout=DEFAULT_TIMEOUT_MS)

    ctx.record_step(
        step_id="A2",
        portal="Gateway",
        feature="CORS 204, Favicon, Sanitization /v1/status, Zero-Spawn /api/artifact/download SHA256 & /logout",
        t0=t0,
        assertions=[
            "OPTIONS /v1/health -> HTTP 204 + CORS *",
            "/v1/status masque phash_dir et subtitles_dir côté public",
            f"HEAD & GET /api/artifact/download ({ctx.sample_phash_filename}) SHA256 vérifié ({computed_sha[:16]}...)",
            "Path traversal ../etc/passwd rejeté HTTP 400 et fichier absent HTTP 404",
            "/logout révoque dbsrt_session puis ?token= ré-authentifie vers /sources",
        ],
        metrics={
            "sample_phash": ctx.sample_phash_filename,
            "phash_bytes": len(raw_bytes),
            "couplings_bytes": len(coup_bytes),
        },
        screenshot=shot_logout,
    )


def step_b1_sources_dashboard_and_profiles(ctx: RecetteContext) -> None:
    """Verify /sources & /sources.html, o2switch probes (Freebox 595, Synology WAN), accordion & env selector."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    _wait_sources_probes_done(page)
    alias_status = page.evaluate("async (base) => (await fetch(`${base}/sources.html`)).status", ctx.base_url)
    assert alias_status == HTTP_OK
    shot_probes = ctx.shot("sources_dashboard_o2switch_live_probes")

    probe_snap = page.evaluate(
        """() => {
            const d = Alpine.$data(document.body);
            const map = {};
            for (const s of d.sources) {
                map[s.id] = {};
                for (const p of s.profiles) {
                    map[s.id][p.id] = {
                        status: p.status,
                        accessible: p.accessible,
                        files_count: p.files_count,
                        latency_ms: p.latency_ms,
                    };
                }
            }
            return { host: d.hostName, env: d.activeEnv, map };
        }"""
    )
    assert probe_snap["host"] == "sapin.o2switch.net"
    assert probe_snap["env"] == "o2switch"
    fbx_wan = probe_snap["map"]["freebox-disks"]["wan_http"]
    syn_vid = probe_snap["map"]["synology-video"]["wan_ftp"]
    syn_tv = probe_snap["map"]["synology-tvshow"]["wan_ftp"]
    assert fbx_wan["accessible"] is True and fbx_wan["status"] == "online"
    assert fbx_wan["files_count"] == EXPECTED_FREEBOX_CONTAINERS, f"Expected 595 Freebox files, got {fbx_wan}"
    assert syn_vid["accessible"] is True and syn_vid["files_count"] >= MIN_SYNOLOGY_VIDEO_ITEMS
    assert syn_tv["accessible"] is True and syn_tv["files_count"] >= MIN_SYNOLOGY_TVSHOW_ITEMS

    page.click("button:has-text('Tout Replier')")
    collapsed_all = page.evaluate("() => Alpine.$data(document.body).visibleSources().every(s => s.collapsed)")
    assert collapsed_all is True
    shot_collapsed = ctx.shot("sources_dashboard_all_collapsed")

    page.click("button:has-text('Tout Déplier')")
    expanded_all = page.evaluate("() => Alpine.$data(document.body).visibleSources().every(s => !s.collapsed)")
    assert expanded_all is True

    for env_id in ENV_SWITCH_SEQUENCE:
        page.select_option("#env-select", env_id)
    _wait_sources_probes_done(page)

    page.click("button:has-text('Tester Accessibilité')")
    _wait_sources_probes_done(page)

    ctx.record_step(
        step_id="B1",
        portal="Sources",
        feature="Dashboard Sources Médias, Probes WAN Réels, Accordéons & Sélecteur d'Environnement",
        t0=t0,
        assertions=[
            "Hôte détecté = sapin.o2switch.net, profil actif = o2switch",
            f"Freebox wan_http Accessible 🟢 avec exactement {fbx_wan['files_count']} conteneurs éligibles (0 faux-positif)",
            f"Synology Video wan_ftp Accessible 🟢 ({syn_vid['files_count']} entrées) & TV Shows wan_ftp ({syn_tv['files_count']} séries)",
            "Tout Replier / Tout Déplier, sélecteur #env-select (5 profils) et Tester Accessibilité vérifiés",
        ],
        metrics={
            "freebox_wan_http": fbx_wan,
            "synology_video_wan_ftp": syn_vid,
            "synology_tvshow_wan_ftp": syn_tv,
            "collapsed_shot": shot_collapsed,
        },
        screenshot=shot_probes,
    )


def step_b2_freebox_wan_explorer_and_ffprobe(ctx: RecetteContext) -> None:
    """Exercise Freebox wan_http Explorer (595 containers, drill-down, filters, sorts, pagination, live ffprobe)."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    fbx_card = page.locator("div.rounded-xl.p-5", has=page.locator("span:text-is('freebox-disks')"))
    fbx_card.locator("button:has-text('Explorer')").first.click()
    _wait_explorer_ready(page)

    root_meta = page.evaluate("() => Alpine.$data(document.body).explorerData")
    assert root_meta["total_items"] >= 1
    assert len(root_meta["items"]) > 0, "Freebox root explorer must never be empty (0/0 regression guard)"
    shot_root = ctx.shot("explorer_freebox_wan_http_root")

    drill_state = _drill_to_video_container(page)
    shot_sub = ctx.shot("explorer_freebox_subfolder_containers")

    page.locator("[id^='item-']").nth(drill_state["vidIdx"]).click()
    page.wait_for_function(
        """() => {
            const d = Alpine.$data(document.body);
            return d.detailsLoading === false && d.selectedDetails !== null;
        }""",
        timeout=FFPROBE_TIMEOUT_MS,
    )
    details = page.evaluate("() => Alpine.$data(document.body).selectedDetails")
    assert details.get("probed") is True, f"Expected live HTTP Range ffprobe on Freebox WAN, got: {details}"
    assert details.get("video", {}).get("width", 0) > 0
    shot_ffprobe = ctx.shot("explorer_freebox_wan_ffprobe_card")

    page.click("button:has-text('Relancer ffprobe')")
    page.wait_for_function("() => !Alpine.$data(document.body).detailsLoading", timeout=FFPROBE_TIMEOUT_MS)
    page.click("button:has-text('Copier')")

    search_input = page.locator("input[x-model='explorerQuery']")
    search_input.fill("a")
    page.wait_for_timeout(UI_SETTLE_MS)
    _wait_explorer_ready(page)
    clear_btn = page.locator("button:text-is('✕')").first
    if clear_btn.is_visible():
        clear_btn.click()
    else:
        search_input.fill("")
    page.wait_for_timeout(UI_SETTLE_MS)
    _wait_explorer_ready(page)

    for tag_label in EXPLORER_QUICK_TAGS:
        page.locator("div[x-show=\"currentView === 'explorer'\"]").locator(f"button:text-is('{tag_label}')").click()
        _wait_explorer_ready(page)

    sort_sel = page.locator("select[x-model='explorerSort']")
    for sort_val in EXPLORER_SORT_MODES:
        sort_sel.select_option(sort_val)
        _wait_explorer_ready(page)

    limit_sel = page.locator("select[x-model='explorerLimit']")
    for lim_val in EXPLORER_LIMIT_MODES:
        limit_sel.select_option(lim_val)
        _wait_explorer_ready(page)

    pages_total = page.evaluate("() => Alpine.$data(document.body).explorerData?.total_pages || 1")
    if pages_total > 1:
        page.click("button:has-text('Suivant ▶')")
        _wait_explorer_ready(page)
        page.click("button:has-text('Dernier')")
        _wait_explorer_ready(page)
        page.click("button:has-text('◀ Précédent')")
        _wait_explorer_ready(page)
        page.click("button:has-text('Premier')")
        _wait_explorer_ready(page)

    page.click("button[title='Rafraîchir la liste']")
    _wait_explorer_ready(page)

    if page.locator("button[title='Remonter au dossier parent']").is_visible():
        page.click("button[title='Remonter au dossier parent']")
        _wait_explorer_ready(page)

    page.click("button:has-text('Sources Médias')")
    page.wait_for_function("() => Alpine.$data(document.body).currentView === 'sources'", timeout=DEFAULT_TIMEOUT_MS)

    ctx.record_step(
        step_id="B2",
        portal="Sources",
        feature="Explorateur Master-Detail Freebox HTTP WAN (Drill-down, Tags, Tris, Pagination & ffprobe HTTP 206)",
        t0=t0,
        assertions=[
            f"Racine Freebox wan_http non vide ({len(root_meta['items'])} disques éligibles), sous-dossier peuplé",
            f"Inspection ffprobe HTTP Range 206 réussie : {details['video'].get('codec')} {details['video'].get('width')}x{details['video'].get('height')}",
            "Relancer ffprobe, Copier URI, recherche floue, 5 tags, 5 tris, 3 limites, pagination et remontée parent validés",
        ],
        metrics={
            "probed_file": details.get("name"),
            "resolution": f"{details['video'].get('width')}x{details['video'].get('height')}",
            "codec": details["video"].get("codec"),
            "audio_tracks": len(details.get("audio_streams", [])),
            "subtitle_tracks": len(details.get("subtitle_streams", [])),
            "root_shot": shot_root,
            "subfolder_shot": shot_sub,
        },
        screenshot=shot_ffprobe,
    )


def step_b3_synology_ftp_explorer_and_ffprobe(ctx: RecetteContext) -> None:
    """Exercise Synology WAN FTP Explorer, series folder drill-down, and live FTP stream ffprobe."""
    assert ctx.page is not None
    page = ctx.page
    t0 = time.monotonic()

    syn_card = page.locator("div.rounded-xl.p-5", has=page.locator("span:text-is('synology-tvshow')"))
    syn_card.locator("button:has-text('Explorer')").first.click()
    _wait_explorer_ready(page)
    page.click("button[title='Rafraîchir la liste']")
    _wait_explorer_ready(page)

    listed_names = page.evaluate(
        "() => (Alpine.$data(document.body).explorerFiles || []).map(f => f.name.toLowerCase())"
    )
    assert "__phash__" not in listed_names and "#recycle" not in listed_names, (
        f"System/backup folders leaked into video explorer: {listed_names[:5]}"
    )

    drill_state = _drill_to_video_container(page)
    page.locator("[id^='item-']").nth(drill_state["vidIdx"]).click()
    page.wait_for_function(
        """() => {
            const d = Alpine.$data(document.body);
            return d.detailsLoading === false && d.selectedDetails !== null;
        }""",
        timeout=FFPROBE_TIMEOUT_MS,
    )
    details = page.evaluate("() => Alpine.$data(document.body).selectedDetails")
    assert details.get("probed") is True, f"Expected live FTP stream ffprobe on Synology WAN, got: {details}"
    shot_syn = ctx.shot("explorer_synology_wan_ftp_ffprobe")

    page.locator("header div.cursor-pointer", has_text="PYDBSRT").first.click()
    page.wait_for_function("() => Alpine.$data(document.body).currentView === 'sources'", timeout=DEFAULT_TIMEOUT_MS)

    ctx.record_step(
        step_id="B3",
        portal="Sources",
        feature="Explorateur Master-Detail Synology WAN FTP & ffprobe Streaming Direct",
        t0=t0,
        assertions=[
            "Navigation arborescente série Synology WAN FTP fonctionnelle",
            f"ffprobe streaming FTP distant réussi : {details.get('video', {}).get('codec')} {details.get('duration_formatted')}",
            "Retour dashboard via clic logo PYDBSRT STUDIO vérifié",
        ],
        metrics={
            "probed_file": details.get("name"),
            "duration": details.get("duration_formatted"),
            "codec": details.get("video", {}).get("codec"),
        },
        screenshot=shot_syn,
    )


def run_gateway_and_sources_suite(ctx: RecetteContext) -> None:
    """Run Portal A (Gateway) and Portal B Part 1 (Sources & Explorer) in Session 1."""
    step_a1_gateway_auth_flows(ctx)
    step_a2_gateway_protocols_and_artifacts(ctx)
    step_b1_sources_dashboard_and_profiles(ctx)
    step_b2_freebox_wan_explorer_and_ffprobe(ctx)
    step_b3_synology_ftp_explorer_and_ffprobe(ctx)
