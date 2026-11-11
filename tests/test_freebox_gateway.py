"""Unit tests for Freebox Range HTTP Gateway logic and byte range parsing."""

import http.server
import json
import threading
import unicodedata
import urllib.request

from scripts.infra.rpi4.rpi4_freebox_gateway import FreeboxRangeHTTPHandler, is_served_path, parse_byte_range


def test_parse_byte_range_full_span():
    # 0 to end
    res = parse_byte_range("bytes=0-999", 1000)
    assert res == (0, 999)


def test_parse_byte_range_open_ended():
    # start to EOF
    res = parse_byte_range("bytes=500-", 1000)
    assert res == (500, 999)


def test_parse_byte_range_clamped_to_filesize():
    # end requested past EOF gets clamped
    res = parse_byte_range("bytes=100-5000", 1000)
    assert res == (100, 999)


def test_parse_byte_range_invalid_syntax():
    assert parse_byte_range("invalid", 1000) is None
    assert parse_byte_range("", 1000) is None
    assert parse_byte_range(None, 1000) is None


def test_parse_byte_range_unsatisfiable():
    # start > end
    assert parse_byte_range("bytes=500-200", 1000) is None
    # start >= file_size
    assert parse_byte_range("bytes=1000-1500", 1000) is None
    # empty file
    assert parse_byte_range("bytes=0-100", 0) is None


def test_served_paths_are_limited_to_media_libraries():
    assert is_served_path("Disque_1_1/Vidéos/TVSHOWS/a.mkv")
    # CIFS may return decomposed accents
    assert is_served_path(unicodedata.normalize("NFD", "Disque_1/Téléchargements/a.mkv"))
    assert not is_served_path("Disque_1/Photos/a.jpg")
    assert not is_served_path("Disque_1/Backup/a.mkv")
    assert not is_served_path("Disque_1/Téléchargements")


def test_health_check_endpoint():
    """Verifies that /health responds 200 OK on both HEAD and GET with valid JSON."""
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FreeboxRangeHTTPHandler)
    port = server.server_port
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    try:
        url = f"http://127.0.0.1:{port}/health"

        # 1. HEAD request
        req_head = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req_head, timeout=2) as resp:
            assert resp.status == 200
            assert resp.headers.get("Content-Type") == "application/json"

        # 2. GET request
        with urllib.request.urlopen(url, timeout=2) as resp:
            assert resp.status == 200
            data = json.loads(resp.read().decode("utf-8"))
            assert data["status"] == "ok"
            assert data["service"] == "dbsrt-freebox-gateway"
            assert "mounted_shares" in data
    finally:
        server.shutdown()
        server.server_close()
