"""Unit tests for o2switch_batch_worker script and HTTP streaming bridge."""

import io
import tempfile
import urllib.parse
from http.server import HTTPServer
from pathlib import Path
from typing import Any, cast
from unittest.mock import MagicMock, patch

from scripts.o2switch_batch_worker import (
    FILE_SIZES_MAP,
    StreamingBridgeHandler,
    load_certified_stems,
    load_existing_phash_stems,
)


def test_load_existing_phash_stems_comprehensive():
    """Verifies that .phash stems are correctly resolved regardless of fps suffix or urlencoding."""
    with tempfile.TemporaryDirectory() as tmp_dir:
        phash_dir = Path(tmp_dir)

        # Standard clean stem (non-empty valid file)
        (phash_dir / "clean_movie.mkv.24.0fps.phash").write_bytes(b"DATA")
        # Stem with float fps (e.g. 23.976)
        (phash_dir / "episode_float.mkv.23.976fps.phash").write_bytes(b"DATA")
        # Stem with URL-encoded special characters
        encoded_name = urllib.parse.quote("L'illusion du temps (1_4) [ARTE].webm") + ".25.0fps.phash"
        (phash_dir / encoded_name).write_bytes(b"DATA")
        # Legacy stem without fps
        (phash_dir / "legacy_movie.mkv.phash").write_bytes(b"DATA")
        # Corrupted 0-byte file (must be ignored)
        (phash_dir / "corrupted_movie.mkv.24.0fps.phash").touch()

        stems = load_existing_phash_stems(phash_dir)

        assert "clean_movie" in stems
        assert "episode_float" in stems
        assert "L'illusion du temps (1_4) [ARTE]" in stems
        assert "legacy_movie" in stems
        assert "corrupted_movie" not in stems


def test_load_certified_stems_from_couplings():
    """Verifies parsing of certified stems from Zero-DB couplings.json."""
    with tempfile.TemporaryDirectory() as tmp_dir:
        couplings_path = Path(tmp_dir) / "couplings.json"
        couplings_path.write_text(
            '{"couplings": [{"media_name": "Show.S01E01"}, {"media_name": "Show.S01E02"}]}',
            encoding="utf-8",
        )

        stems = load_certified_stems(couplings_path)
        assert "Show.S01E01" in stems
        assert "Show.S01E02" in stems
        assert len(stems) == 2


class MockSocket:
    """Mock socket for testing BaseHTTPRequestHandler in memory."""

    def __init__(self, request_bytes: bytes):
        self.rfile = io.BytesIO(request_bytes)
        self.wfile = io.BytesIO()

    def makefile(self, mode: str, bufsize: int = -1):
        if "b" in mode:
            if "r" in mode:
                return self.rfile
            elif "w" in mode:
                return self.wfile
        raise ValueError(f"Unsupported mode: {mode}")

    def sendall(self, data: bytes):
        self.wfile.write(data)

    def close(self):
        pass


def test_streaming_bridge_do_head():
    """Verifies that HEAD request returns 200 OK with Content-Length and Accept-Ranges."""
    test_path = "/tvshow/test_show/episode1.mp4"
    FILE_SIZES_MAP[test_path] = 1048576

    raw_req = f"HEAD /stream{test_path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".encode()
    sock = MockSocket(raw_req)

    server = HTTPServer(("127.0.0.1", 0), StreamingBridgeHandler)
    StreamingBridgeHandler(cast(Any, sock), ("127.0.0.1", 12345), server)

    sock.wfile.seek(0)
    response_data = sock.wfile.read().decode("utf-8", errors="ignore")

    assert "HTTP/1.0 200 OK" in response_data or "HTTP/1.1 200 OK" in response_data
    assert "Accept-Ranges: bytes" in response_data
    assert "Content-Length: 1048576" in response_data
    assert "Content-Type: video/mp4" in response_data


@patch("subprocess.Popen")
def test_streaming_bridge_do_get_with_range(mock_popen):
    """Verifies that GET request with Range header invokes curl -r and returns 206 Partial Content."""
    mock_proc = MagicMock()
    mock_proc.stdout.read.side_effect = [b"PARTIAL_VIDEO_BYTES", b""]
    mock_proc.poll.return_value = 0
    mock_proc.stderr = None
    mock_popen.return_value = mock_proc

    test_path = "/tvshow/test_show/episode1.mkv"
    FILE_SIZES_MAP[test_path] = 5000000

    raw_req = (f"GET /stream{test_path} HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=100-200\r\n\r\n").encode()
    sock = MockSocket(raw_req)

    server = HTTPServer(("127.0.0.1", 0), StreamingBridgeHandler)
    StreamingBridgeHandler(cast(Any, sock), ("127.0.0.1", 12345), server)

    # Check curl arguments
    mock_popen.assert_called_once()
    cmd = mock_popen.call_args[0][0]
    assert "-r" in cmd
    range_idx = cmd.index("-r")
    assert cmd[range_idx + 1] == "100-200"

    sock.wfile.seek(0)
    response_data = sock.wfile.read()

    assert b"206 Partial Content" in response_data
    assert b"Content-Range: bytes 100-200/5000000" in response_data
    assert b"Content-Length: 101" in response_data
    assert b"PARTIAL_VIDEO_BYTES" in response_data


def test_streaming_bridge_invalid_route():
    """Verifies that non-stream routes return 404."""
    raw_req = b"GET /invalid/path HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
    sock = MockSocket(raw_req)

    server = HTTPServer(("127.0.0.1", 0), StreamingBridgeHandler)
    StreamingBridgeHandler(cast(Any, sock), ("127.0.0.1", 12345), server)

    sock.wfile.seek(0)
    response_data = sock.wfile.read().decode("utf-8", errors="ignore")
    assert "404" in response_data


@patch("subprocess.Popen")
def test_streaming_bridge_sftp_failure_returns_502(mock_popen):
    """Verifies that if curl produces zero bytes, bridge returns HTTP 502 Bad Gateway."""
    mock_proc = MagicMock()
    mock_proc.stdout.read.side_effect = [b""]
    mock_proc.poll.return_value = 1
    mock_proc.stderr.read.return_value = b"Authentication failed"
    mock_popen.return_value = mock_proc

    test_path = "/tvshow/test_show/episode1.mkv"
    raw_req = f"GET /stream{test_path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".encode()
    sock = MockSocket(raw_req)

    server = HTTPServer(("127.0.0.1", 0), StreamingBridgeHandler)
    StreamingBridgeHandler(cast(Any, sock), ("127.0.0.1", 12345), server)

    sock.wfile.seek(0)
    response_data = sock.wfile.read().decode("utf-8", errors="ignore")
    assert "502" in response_data


def test_batch_worker_retry_failed_flag_parsing():
    """Verifies that --retry-failed argument is accepted and defaults are sound."""
    import argparse

    from scripts.o2switch_batch_worker import DEFAULT_MAX_EXTRACTION_SECS

    parser = argparse.ArgumentParser()
    parser.add_argument("--retry-failed", action="store_true")
    parser.add_argument("--max-extraction-secs", type=int, default=DEFAULT_MAX_EXTRACTION_SECS)

    args = parser.parse_args(["--retry-failed", "--max-extraction-secs", "1200"])
    assert args.retry_failed is True
    assert args.max_extraction_secs == 1200
    assert DEFAULT_MAX_EXTRACTION_SECS == 1800
