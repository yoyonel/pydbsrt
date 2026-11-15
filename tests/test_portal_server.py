"""Unit and integration tests for pydbsrt MediaSources Portal server and exploration API."""

from __future__ import annotations

import http.server
import json
import threading
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import pytest

from scripts.portal_server import (
    BYTES_PER_GB,
    BYTES_PER_KB,
    BYTES_PER_MB,
    PortalRequestHandler,
    TrajectoriesQueryCriteria,
    execute_single_probe,
    format_human_size,
    format_seconds_duration,
    get_sources_metadata,
    get_trajectory_record_detail,
    inspect_file_details,
    list_source_files,
    list_trajectory_audit_files,
    parse_media_filename,
    query_trajectory_records,
)


def test_format_human_size() -> None:
    """Verifies byte size formatting with named units."""
    assert format_human_size(500) == "500 B"
    assert format_human_size(BYTES_PER_KB * 2) == "2.0 KB"
    assert format_human_size(BYTES_PER_MB * 15) == "15.00 MB"
    assert format_human_size(int(BYTES_PER_GB * 14.8)) == "14.80 GB"


def test_format_seconds_duration() -> None:
    """Verifies duration formatting into HH:MM:SS / MM:SS."""
    assert format_seconds_duration(45.2) == "00m 45s"
    assert format_seconds_duration(125.0) == "02m 05s"
    assert format_seconds_duration(3665.0) == "01h 01m 05s"
    assert format_seconds_duration(7440.0) == "02h 04m 00s"


def test_parse_media_filename_tokens() -> None:
    """Verifies release token extraction and clean title heuristics."""
    parsed1 = parse_media_filename("The.Boy.and.the.Heron.2023.2160p.WEB-DL.DUAL.HDR.DDP5.1.Atmos.H.265-FLUX.mkv")
    assert parsed1["title"] == "The Boy and the Heron"
    assert parsed1["year"] == "2023"
    assert parsed1["resolution"] == "2160P"
    assert parsed1["codec"] == "H.265"
    assert parsed1["source"] == "WEB-DL"

    parsed2 = parse_media_filename("28 Semaines Plus Tard (2007) [1080p].mkv")
    assert parsed2["title"] == "28 Semaines Plus Tard"
    assert parsed2["year"] == "2007"
    assert parsed2["resolution"] == "1080P"

    parsed3 = parse_media_filename("Watchmen.Ultimate.Cut.2009.1080p.BrRip.x264.YIFY_mid15s.mp4")
    assert parsed3["year"] == "2009"
    assert parsed3["resolution"] == "1080P"
    assert parsed3["codec"] == "X264"


def test_list_source_files_snippets() -> None:
    """Tests pagination, filtering and sorting on local snippets source."""
    snippets_dir = Path("data/snippets")
    if not snippets_dir.is_dir():
        pytest.skip("data/snippets not present in test environment")

    # 1. Page 1 with limit 10
    res = list_source_files("local-snippets", "local", page=1, limit=10, sort_mode="name_asc")
    assert res["status"] == "ok"
    assert res["total_items"] > 100
    assert len(res["items"]) == 10
    assert res["page"] == 1
    assert res["limit"] == 10

    # 2. Filtering with search query
    filtered = list_source_files("local-snippets", "local", page=1, limit=10, query_str="Watchmen")
    assert filtered["status"] == "ok"
    assert filtered["filtered_items"] >= 1
    for item in filtered["items"]:
        assert "watchmen" in item["name"].lower()


def test_list_source_files_freebox_gateway(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Tests listing files from Freebox targets catalog over HTTP gateway profile across all 3 disks."""
    reports_dir = tmp_path / "data" / "reports"
    reports_dir.mkdir(parents=True)
    sample_targets = [
        {
            "path": f"Disque_1_1/Vidéos/Film_{idx:03d}.1080p.mkv",
            "filename": f"Film_{idx:03d}.1080p.mkv",
            "size_bytes": 1_500_000_000 + idx,
        }
        for idx in range(120)
    ]
    (reports_dir / "freebox_batch_targets.json").write_text(json.dumps(sample_targets), encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("DBSRT_PORTAL_CACHE_DIR", str(tmp_path / "cache"))

    res = list_source_files("freebox-disks", "lan_http", page=1, limit=5, force=True)
    assert res["status"] == "ok"
    assert res["total_items"] == 120
    assert len(res["items"]) == 5
    assert res["items"][0]["size_bytes"] > 0
    assert not res["items"][0]["is_dir"]


def test_inspect_file_details_snippet() -> None:
    """Verifies detailed multimedia inspection on a local snippet file."""
    sample = Path("data/snippets/Watchmen.Ultimate.Cut.2009.1080p.BrRip.x264.YIFY_mid15s.mp4")
    if not sample.is_file():
        pytest.skip("Watchmen snippet not present")

    details = inspect_file_details("local-snippets", "local", sample.name)
    assert details["status"] == "ok"
    assert details["name"] == sample.name
    assert details["size_bytes"] > 0
    assert details["extension"] == ".mp4"

    # Multimédia stream verification
    if details.get("probed"):
        assert "video" in details
        assert details["video"]["codec"] == "h264"
        assert details["video"]["width"] > 0
        assert details["video"]["height"] > 0
        assert details["duration_seconds"] > 0


def test_persistent_disk_cache(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Verifies persistent disk caching lifecycle and atomic invalidation."""
    from scripts.portal_server import _read_disk_cache, _write_disk_cache

    monkeypatch.setenv("DBSRT_PORTAL_CACHE_DIR", str(tmp_path))
    test_key = "test_source:test_profile:sub"
    payload = {"sample_file": "movie.mkv", "size": 12345}

    # 1. Miss initially
    assert _read_disk_cache("listing", test_key, 60.0) is None

    # 2. Write
    _write_disk_cache("listing", test_key, payload)

    # 3. Hit
    hit = _read_disk_cache("listing", test_key, 60.0)
    assert hit is not None
    ts, data = hit
    assert data == payload
    assert ts > 0

    # 4. Expiration check with 0s TTL
    assert _read_disk_cache("listing", test_key, 0.0) is None


def test_get_sources_metadata() -> None:
    """Verifies declarative schema of all media sources, environments, and profiles."""
    meta = get_sources_metadata()
    assert "sources" in meta
    assert "available_envs" in meta
    assert "detected_env" in meta
    assert len(meta["sources"]) >= 4

    source_ids = {s["id"] for s in meta["sources"]}
    assert "local-snippets" in source_ids
    assert "synology-video" in source_ids
    assert "synology-tvshow" in source_ids
    assert "freebox-disks" in source_ids

    fbx = next(s for s in meta["sources"] if s["id"] == "freebox-disks")
    fbx_profiles = {p["id"]: p for p in fbx["profiles"]}
    for local_disk_id in ("disque1", "disque2", "disque1_1"):
        assert local_disk_id in fbx_profiles
        assert fbx_profiles[local_disk_id]["envs"] == ["local"]
    for rpi4_disk_id in ("rpi4_disque1", "rpi4_disque2", "rpi4_disque1_1"):
        assert rpi4_disk_id in fbx_profiles
        assert fbx_profiles[rpi4_disk_id]["envs"] == ["rpi4"]
    assert "lan_ftp" in fbx_profiles
    assert fbx_profiles["lan_ftp"]["envs"] == ["local", "rpi4"]
    assert "lan_http" in fbx_profiles
    assert fbx_profiles["lan_http"]["envs"] == ["local"]
    assert "wan_http" in fbx_profiles
    assert fbx_profiles["wan_http"]["envs"] == ["o2switch"]
    assert "rpi4_gateway" in fbx_profiles
    assert fbx_profiles["rpi4_gateway"]["envs"] == ["rpi4"]

    syn_v = next(s for s in meta["sources"] if s["id"] == "synology-video")
    syn_profiles = {p["id"]: p for p in syn_v["profiles"]}
    assert "lan_ftp" in syn_profiles
    assert syn_profiles["lan_ftp"]["envs"] == ["local", "rpi4"]
    assert "wan_ftp" in syn_profiles
    assert syn_profiles["wan_ftp"]["envs"] == ["o2switch", "nomad"]


def test_execute_single_probe_snippets() -> None:
    """Verifies single channel diagnostic probe on local snippets."""
    if not Path("data/snippets").is_dir():
        pytest.skip("data/snippets not present in test environment")

    res = execute_single_probe("local-snippets", "local")
    assert res["status"] == "online"
    assert res["accessible"] is True
    assert res["files_count"] > 100
    assert res["latency_ms"] >= 0


def test_execute_single_probe_unknown() -> None:
    """Verifies that probing an invalid source or profile fails gracefully."""
    res = execute_single_probe("non-existent-source", "bad-profile")
    assert res["status"] in ("unknown", "missing")
    assert res["accessible"] is False


def test_trajectory_api_records() -> None:
    """Verifies listing and querying spatiotemporal trajectory records."""
    files = list_trajectory_audit_files()
    if not files:
        pytest.skip("No trajectory audit files available in test environment")

    assert len(files) >= 1
    first_file = files[0]["filename"]

    # 1. Query records from first available file
    criteria = TrajectoriesQueryCriteria(file_name=first_file, page=1, limit=5)
    res = query_trajectory_records(criteria)
    assert res["status"] == "ok"
    assert "kpi" in res
    assert "total_audits" in res["kpi"]
    assert "total_matched" in res["kpi"]
    assert "coherence_rate_pct" in res["kpi"]
    assert len(res["items"]) <= 5

    # 2. Get detail of first item if present
    if res["items"]:
        first_item = res["items"][0]
        target = first_item["target_media"]
        detail = get_trajectory_record_detail(first_file, target)
        assert detail["status"] == "ok"
        assert "cuts" in detail["item"]


def test_portal_http_api_endpoints() -> None:
    """Spins up a lightweight test HTTP daemon on port 0 and verifies REST API endpoints."""
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), PortalRequestHandler)
    port = server.server_port
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    base_url = f"http://127.0.0.1:{port}"

    try:
        # 1. GET /api/sources
        with urllib.request.urlopen(f"{base_url}/api/sources", timeout=5) as resp:
            assert resp.status == 200
            data = json.loads(resp.read().decode())
            assert "sources" in data
            assert len(data["sources"]) >= 4

        # 2. GET /api/files (snippets)
        if Path("data/snippets").is_dir():
            files_url = f"{base_url}/api/files?source=local-snippets&profile=local&page=1&limit=5"
            with urllib.request.urlopen(files_url, timeout=5) as resp:
                assert resp.status == 200
                fdata = json.loads(resp.read().decode())
                assert fdata["status"] == "ok"
                assert fdata["total_items"] > 100
                assert len(fdata["items"]) == 5

            # 3. GET /api/file/details
            sample = Path("data/snippets/Watchmen.Ultimate.Cut.2009.1080p.BrRip.x264.YIFY_mid15s.mp4")
            if sample.is_file():
                detail_url = f"{base_url}/api/file/details?source=local-snippets&profile=local&path={sample.name}"
                with urllib.request.urlopen(detail_url, timeout=5) as resp:
                    assert resp.status == 200
                    ddata = json.loads(resp.read().decode())
                    assert ddata["status"] == "ok"
                    assert ddata["name"] == sample.name

        # 4. GET /api/assets (PostgreSQL / graceful fallback)
        with urllib.request.urlopen(f"{base_url}/api/assets?limit=5", timeout=5) as resp:
            assert resp.status == 200
            assets_data = json.loads(resp.read().decode())
            assert "status" in assets_data
            assert "summary" in assets_data
            assert "items" in assets_data

        # 4b. GET /api/assets with page exceeding total_pages -> auto-clamped to page 1
        with urllib.request.urlopen(f"{base_url}/api/assets?page=9999&limit=25", timeout=5) as resp:
            assert resp.status == 200
            assets_data = json.loads(resp.read().decode())
            if assets_data["status"] == "ok" and assets_data["total_items"] > 0:
                assert assets_data["page"] == 1
                assert len(assets_data["items"]) > 0

        # 4c. GET /api/phashes (Tripartite pHash reconciliation)
        with urllib.request.urlopen(f"{base_url}/api/phashes?limit=5", timeout=5) as resp:
            assert resp.status == 200
            phashes_data = json.loads(resp.read().decode())
            assert phashes_data["status"] == "ok"
            assert "summary" in phashes_data
            assert "items" in phashes_data
            assert phashes_data["summary"]["raw_count"] >= 0
            assert phashes_data["summary"]["catalog_count"] >= 0
            if Path("data/__PHASH__").is_dir() and any(Path("data/__PHASH__").glob("*.phash")):
                assert phashes_data["summary"]["raw_count"] > 0
            if Path("data/catalog.dbphash").is_file():
                assert phashes_data["summary"]["catalog_count"] > 0
            assert len(phashes_data["items"]) <= 5

        # 4d. GET /api/phash/details & /api/artifact/download (Microscopic inspection, Binary Stream & Traversal Guard)
        with pytest.raises(urllib.error.HTTPError) as exc_info:
            urllib.request.urlopen(f"{base_url}/api/phash/details?moviehash=../../etc/passwd", timeout=5)
        assert exc_info.value.code == 404

        with pytest.raises(urllib.error.HTTPError) as dl_trav_exc:
            urllib.request.urlopen(f"{base_url}/api/artifact/download?kind=phash&file=../../etc/passwd", timeout=5)
        assert dl_trav_exc.value.code == 400

        sample_phash = Path("data/__PHASH__/002835455e305309.24.0fps.phash")
        if sample_phash.is_file():
            import hashlib

            dl_url = f"{base_url}/api/artifact/download?kind=phash&file={sample_phash.name}"
            with urllib.request.urlopen(dl_url, timeout=5) as dl_resp:
                assert dl_resp.status == 200
                dl_bytes = dl_resp.read()
                assert dl_bytes == sample_phash.read_bytes()
                assert dl_resp.headers.get("X-Checksum-SHA256") == hashlib.sha256(dl_bytes).hexdigest()

            detail_url = f"{base_url}/api/phash/details?moviehash=002835455e305309"
            with urllib.request.urlopen(detail_url, timeout=5) as resp:
                assert resp.status == 200
                pdetail = json.loads(resp.read().decode())
                assert pdetail["status"] == "ok"
                assert pdetail["moviehash"] == "002835455e305309"
                assert "entropy" in pdetail
                assert "cuts" in pdetail
                assert "compression" in pdetail
                assert "mobility" in pdetail
                assert "mean_delta" in pdetail["mobility"]
                assert "histogram" in pdetail["mobility"]
                assert len(pdetail["mobility"]["histogram"]) == 6
                assert pdetail["entropy"]["mean_popcount"] > 0
                assert "timeline" in pdetail["cuts"]
                assert len(pdetail["cuts"]["timeline"]) > 0

        # 4e. GET /api/trajectory/files, /api/trajectories, /api/trajectory/details
        files_url = f"{base_url}/api/trajectory/files"
        with urllib.request.urlopen(files_url, timeout=5) as resp:
            assert resp.status == 200
            files_data = json.loads(resp.read().decode())
            assert files_data["status"] == "ok"
            assert "files" in files_data

        traj_url = f"{base_url}/api/trajectories?limit=5"
        with urllib.request.urlopen(traj_url, timeout=5) as resp:
            assert resp.status == 200
            traj_data = json.loads(resp.read().decode())
            assert traj_data["status"] == "ok"
            assert "kpi" in traj_data
            assert "items" in traj_data
            if traj_data["items"]:
                first_target = traj_data["items"][0]["target_media"]
                detail_url = f"{base_url}/api/trajectory/details?target={urllib.parse.quote(first_target)}"
                with urllib.request.urlopen(detail_url, timeout=5) as dresp:
                    assert dresp.status == 200
                    detail_data = json.loads(dresp.read().decode())
                    assert detail_data["status"] == "ok"
                    assert "cuts" in detail_data["item"]

        # 5. GET /sources.html (Includes Interactive Sismographe & Zoom & Brush & Mobility)
        with urllib.request.urlopen(f"{base_url}/sources.html", timeout=5) as resp:
            assert resp.status == 200
            html = resp.read().decode()
            assert "Sources Médias" in html
            assert "Actifs Médias (DB)" in html
            assert "Aide & Définitions Métier" in html
            assert "Sismographe" in html
            assert "VUE PANORAMIQUE" in html
            assert "timelineZoom" in html
            assert "Mobilité Temporelle" in html
            assert "getHoverCursorY" in html

    finally:
        server.shutdown()
        server.server_close()


def test_trajectory_kpi_coverage_clamped_and_file_filtering() -> None:
    """Verifies that catalog coverage KPI is strictly bounded in [0%, 100%] and invalid files are filtered."""
    import tempfile

    from scripts.portal_server import _compute_trajectory_kpi, _is_valid_trajectory_file

    # 1. Invalid file filtering
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as f:
        f.write('{"_meta": "killtest", "cuts": 1, "seconds": 12}\n')
        invalid_path = Path(f.name)

    try:
        assert _is_valid_trajectory_file(invalid_path) is False
    finally:
        invalid_path.unlink()

    # 2. Valid file structure
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as f:
        f.write('{"target_media": "test.mp4", "cuts": [{"start_sec": 10}], "consensus_media_id": 42}\n')
        valid_path = Path(f.name)

    try:
        assert _is_valid_trajectory_file(valid_path) is True
    finally:
        valid_path.unlink()

    # 3. KPI coverage bound check even with massive mock ids
    mock_records = [{"consensus_media_id": i, "is_temporally_coherent": True, "cuts_count": 3} for i in range(1, 5000)]
    kpis = _compute_trajectory_kpi(mock_records)
    assert 0.0 <= kpis["catalog_coverage_pct"] <= 100.0
    assert kpis["unique_catalog_covered"] <= kpis["catalog_total"]


def test_portal_cifs_hang_guard_and_gateway_html_rejection(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Verifies CIFS D-state hang guard off Home LAN, Disque_1_1 SSOT resolution, and WAN HTML rejection."""
    from unittest.mock import MagicMock

    import scripts.portal_server as ps

    monkeypatch.setattr(ps, "_is_cifs_safe_to_stat", lambda: False)
    monkeypatch.setattr(ps.os, "scandir", lambda _p: (_ for _ in ()).throw(AssertionError("scandir must not run!")))
    blocked = ps.probe_local_fs_profile("Montage CIFS Disque 1 1", str(tmp_path), is_cifs=True)
    assert blocked["accessible"] is False
    assert blocked["status"] == "offline"
    assert "anti-gel SMB" in blocked["reason"]

    is_loc, is_ftp, resolved_p, _, _ = ps._resolve_freebox_endpoint("disque1_1", "Vidéos/film.mkv")
    assert is_loc is True and is_ftp is False
    assert resolved_p.endswith("Freebox/Disque_1_1/Vidéos/film.mkv")

    html_resp = MagicMock()
    html_resp.headers.get_content_type.return_value = "text/html"
    html_resp.read.return_value = b"<html>Synology Web Station</html>"
    html_resp.__enter__.return_value = html_resp
    monkeypatch.setattr(ps.urllib.request, "urlopen", lambda *_a, **_kw: html_resp)
    bad_gw = ps.probe_http_gateway_profile("Passerelle HTTP WAN", "http://nas.example.test:80/health")
    assert bad_gw["accessible"] is False
    assert bad_gw["status"] == "offline"
    assert "non-JSON" in bad_gw["reason"]


def test_freebox_gateway_zero_false_positive_and_live_catalog(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Verifies gateway probe returns empty/inaccessible when 0 media exist and lists live /catalog."""
    import scripts.portal_server as ps
    from scripts.infra.rpi4.rpi4_freebox_gateway import FreeboxRangeHTTPHandler

    monkeypatch.setenv("DBSRT_PORTAL_CACHE_DIR", str(tmp_path / "cache"))
    empty_root = tmp_path / "empty_fbx"
    for share in ("Disque_1", "Disque_2", "Disque_1_1"):
        (empty_root / share).mkdir(parents=True)

    class EmptyGatewayHandler(FreeboxRangeHTTPHandler):
        server_root = empty_root

    srv_empty = http.server.ThreadingHTTPServer(("127.0.0.1", 0), EmptyGatewayHandler)
    port_empty = srv_empty.server_address[1]
    threading.Thread(target=srv_empty.serve_forever, daemon=True).start()
    try:
        probe_empty = ps.probe_http_gateway_profile(
            "Passerelle HTTP WAN", f"http://127.0.0.1:{port_empty}/health?force=1"
        )
        assert probe_empty["accessible"] is False
        assert probe_empty["status"] == "empty"
        assert probe_empty["files_count"] == 0
    finally:
        srv_empty.shutdown()
        srv_empty.server_close()

    # Now populate 2 eligible videos and 1 excluded personal video
    vid_dir = empty_root / "Disque_1_1" / "Vidéos"
    photos_dir = empty_root / "Disque_1_1" / "Photos"
    vid_dir.mkdir(parents=True)
    photos_dir.mkdir(parents=True)
    (vid_dir / "Joker (2019)[2160p].mkv").write_bytes(b"\x00" * 128)
    (vid_dir / "Cruauté.2024.1080p.mp4").write_bytes(b"\x00" * 256)
    (photos_dir / "Private.mp4").write_bytes(b"\x00" * 512)

    srv_live = http.server.ThreadingHTTPServer(("127.0.0.1", 0), EmptyGatewayHandler)
    port_live = srv_live.server_address[1]
    threading.Thread(target=srv_live.serve_forever, daemon=True).start()
    try:
        live_health = f"http://127.0.0.1:{port_live}/health?force=1"
        probe_live = ps.probe_http_gateway_profile("Passerelle HTTP WAN", live_health)
        assert probe_live["accessible"] is True
        assert probe_live["status"] == "online"
        assert probe_live["files_count"] == 2

        monkeypatch.setattr(ps, "DEFAULT_GATEWAY_WAN_URL", f"http://127.0.0.1:{port_live}/health")
        listed = ps.list_source_files("freebox-disks", "wan_http", page=1, limit=25, force=True)
        assert listed["status"] == "ok"
        assert listed["total_items"] == 2
        assert {it["name"] for it in listed["items"]} == {"Joker (2019)[2160p].mkv", "Cruauté.2024.1080p.mp4"}
    finally:
        srv_live.shutdown()
        srv_live.server_close()


def test_phash_resolution_unicode_percent_and_no_substring_collision(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Ensures pHash sidecar matching avoids substring collisions and resolves Unicode/percent names."""
    import struct
    import unicodedata

    import scripts.portal_server as ps

    sidecars = {
        "0ce0eae17be4ec9a": {"media_name": "28.mkv"},
        "abcdef0123456789": {"media_name": "Cruauté (2019) [1080p].mkv"},
    }
    idx = ps._build_sidecar_name_index(sidecars)
    assert ps._resolve_phash_moviehash("28 Semaines Plus Tard (2007).24.0fps.phash", sidecars, idx) is None
    assert (
        ps._resolve_phash_moviehash(
            unicodedata.normalize("NFD", "Cruauté (2019) [1080p].mkv.23.98fps.phash"), sidecars, idx
        )
        == "abcdef0123456789"
    )

    phash_dir = tmp_path / "data" / "__PHASH__"
    phash_dir.mkdir(parents=True)
    sample_hashes = struct.pack(">4Q", 0x0F0F0F0F0F0F0F0F, 0xF0F0F0F0F0F0F0F0, 0xAAAAAAAAAAAAAAAA, 0x5555555555555555)
    long_pct_name = ("%20".join(["The", "Dark", "Knight"] * 9)) + ".23.98fps.phash"
    unicode_name = "01 Cruauté [1080p] (VOSTFR) l'épée....23.98fps.phash"
    (phash_dir / long_pct_name).write_bytes(sample_hashes)
    (phash_dir / unicode_name).write_bytes(sample_hashes)

    monkeypatch.chdir(tmp_path)
    res_long = ps.inspect_phash_microscopic(long_pct_name)
    assert res_long["status"] == "ok"
    assert res_long["frame_count"] == 4

    res_oversized = ps.inspect_phash_microscopic(("A" * 350) + ".phash")
    assert res_oversized["status"] == "not_found"

    res_nfd = ps.inspect_phash_microscopic(unicodedata.normalize("NFD", unicode_name))
    assert res_nfd["status"] == "ok"
    assert res_nfd["frame_count"] == 4


def test_ftp_and_portal_excludes_system_and_fingerprint_dirs() -> None:
    """Ensures #recycle, __PHASH__, __AHASH__, @eaDir, and dotfiles are filtered from FTP/Explorer."""
    import scripts.portal_server as ps

    for excluded in ("__PHASH__", "__AHASH__", "#recycle", "@eaDir", ".Trash-1000", ".hidden"):
        assert ps._is_excluded_portal_entry(excluded) is True
        assert ps._parse_mlsd_item(excluded, {"type": "dir"}, "") is None
        assert ps._parse_nlst_item(excluded, "") is None

    valid_dir = ps._parse_mlsd_item("Andor", {"type": "dir"}, "")
    assert valid_dir is not None and valid_dir["is_dir"] is True and valid_dir["name"] == "Andor"
