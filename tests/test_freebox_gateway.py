"""Unit tests for the Freebox Range HTTP Gateway (/health, /catalog, privacy filter, RFC 7233 ranges)."""

from __future__ import annotations

import json
import threading
import urllib.error
import urllib.parse
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

import pytest

from scripts.infra.rpi4.rpi4_freebox_gateway import FreeboxRangeHTTPHandler, scan_served_catalog

SAMPLE_VIDEO_BYTES: bytes = b"MATROSKA_HEADER_0123456789_PAYLOAD_ABCDEFGHIJKLMNOPQRSTUVWXYZ"
EXPECTED_ELIGIBLE_COUNT: int = 3
RANGE_START_BYTE: int = 16
RANGE_END_BYTE: int = 25


def _populate_mock_freebox_root(root: Path) -> None:
    """Creates eligible cinema/TV directories alongside personal excluded folders."""
    d1 = root / "Disque_1" / "Téléchargements"
    d2 = root / "Disque_2" / "Téléchargements" / "Season 01"
    d3 = root / "Disque_1_1" / "Vidéos"
    d1_photos = root / "Disque_1_1" / "Photos"
    d1_backup = root / "Disque_1_1" / "Backup"
    d2_nested_backup = root / "Disque_2" / "Téléchargements" / "Backup"

    for folder in (d1, d2, d3, d1_photos, d1_backup, d2_nested_backup):
        folder.mkdir(parents=True, exist_ok=True)

    (d1 / "The.Neon.Demon.2016.2160p.mkv").write_bytes(SAMPLE_VIDEO_BYTES)
    (d2 / "Lanterns.S01E01.2160p.mp4").write_bytes(SAMPLE_VIDEO_BYTES)
    (d3 / "Joker (2019)[2160p].mkv").write_bytes(SAMPLE_VIDEO_BYTES)
    (d3 / "readme.txt").write_text("non-video sidecar", encoding="utf-8")

    # Personal / system videos that MUST NEVER be exposed or counted
    (d1_photos / "Vacances_2025.mp4").write_bytes(SAMPLE_VIDEO_BYTES)
    (d1_backup / "WhatsApp_Video_001.mp4").write_bytes(SAMPLE_VIDEO_BYTES)
    (d2_nested_backup / "PrivateClip.mkv").write_bytes(SAMPLE_VIDEO_BYTES)


def test_scan_served_catalog_strict_privacy_isolation(tmp_path: Path) -> None:
    """Ensures scan_served_catalog exposes ONLY eligible cinema/TV videos and excludes Photos/Backup."""
    _populate_mock_freebox_root(tmp_path)
    items = scan_served_catalog(tmp_path, force=True)
    assert len(items) == EXPECTED_ELIGIBLE_COUNT
    paths = {it["path"] for it in items}
    assert paths == {
        "Disque_1/Téléchargements/The.Neon.Demon.2016.2160p.mkv",
        "Disque_2/Téléchargements/Season 01/Lanterns.S01E01.2160p.mp4",
        "Disque_1_1/Vidéos/Joker (2019)[2160p].mkv",
    }
    for item in items:
        assert item["size_bytes"] == len(SAMPLE_VIDEO_BYTES)
        assert item["mtime"] > 0


def test_gateway_http_health_catalog_and_range(tmp_path: Path) -> None:
    """Verifies /health media_count, /catalog listing, 404 on Photos, and 206 Partial Content."""
    _populate_mock_freebox_root(tmp_path)

    class IsolatedGatewayHandler(FreeboxRangeHTTPHandler):
        server_root = tmp_path

    server = ThreadingHTTPServer(("127.0.0.1", 0), IsolatedGatewayHandler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base_url = f"http://127.0.0.1:{port}"

    try:
        with urllib.request.urlopen(f"{base_url}/health?force=1") as resp:
            health = json.loads(resp.read().decode("utf-8"))
        assert health["status"] == "ok"
        assert health["media_count"] == EXPECTED_ELIGIBLE_COUNT
        assert set(health["mounted_shares"]) == {"Disque_1", "Disque_1_1", "Disque_2"}

        with urllib.request.urlopen(f"{base_url}/catalog") as resp:
            cat = json.loads(resp.read().decode("utf-8"))
        assert cat["status"] == "ok"
        assert cat["total"] == EXPECTED_ELIGIBLE_COUNT
        assert len(cat["items"]) == EXPECTED_ELIGIBLE_COUNT

        # Personal Photos folder MUST return 404 Not Found
        private_url = f"{base_url}/Disque_1_1/Photos/Vacances_2025.mp4"
        with pytest.raises(urllib.error.HTTPError) as exc_info:
            urllib.request.urlopen(private_url)
        assert exc_info.value.code == 404

        # Eligible video supports HTTP 206 Byte-Range streaming
        rel_quoted = "/".join(urllib.parse.quote(seg) for seg in "Disque_1_1/Vidéos/Joker (2019)[2160p].mkv".split("/"))
        req = urllib.request.Request(
            f"{base_url}/{rel_quoted}",
            headers={"Range": f"bytes={RANGE_START_BYTE}-{RANGE_END_BYTE}"},
        )
        with urllib.request.urlopen(req) as resp:
            assert resp.status == 206
            body = resp.read()
        assert body == SAMPLE_VIDEO_BYTES[RANGE_START_BYTE : RANGE_END_BYTE + 1]
    finally:
        server.shutdown()
        server.server_close()
