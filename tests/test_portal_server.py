"""Unit and integration tests for pydbsrt MediaSources Portal server and exploration API."""

from __future__ import annotations

import http.server
import json
import threading
import urllib.request
from pathlib import Path

import pytest

from scripts.portal_server import (
    BYTES_PER_GB,
    BYTES_PER_KB,
    BYTES_PER_MB,
    PortalRequestHandler,
    execute_single_probe,
    format_human_size,
    format_seconds_duration,
    get_sources_metadata,
    inspect_file_details,
    list_source_files,
    parse_media_filename,
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

        # 4. GET /sources.html
        with urllib.request.urlopen(f"{base_url}/sources.html", timeout=5) as resp:
            assert resp.status == 200
            html = resp.read().decode()
            assert "Sources Médias" in html

    finally:
        server.shutdown()
        server.server_close()
