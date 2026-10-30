#!/usr/bin/env python3
"""Lightweight HTTP Streaming Gateway for Freebox Volumes with Byte-Range Support.

Designed to run on Raspberry Pi 4 to serve mounted Freebox CIFS shares (/mnt/freebox),
reached by o2switch through a Freebox NAT rule restricted to the o2switch source IP
(o2switch kills SSH reverse tunnels after ~20s and only allows outbound ports 21/80/443).
Enables remote FFmpeg decoders to seek efficiently via HTTP 206 Partial Content
without pulling entire multi-gigabyte video files over the WAN.
"""

from __future__ import annotations

import argparse
import mimetypes
import re
import sys
import unicodedata
import urllib.parse
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Tuple

# Named Operational Constants
DEFAULT_GATEWAY_HOST: str = "127.0.0.1"
DEFAULT_GATEWAY_PORT: int = 18089
DEFAULT_SERVE_ROOT: str = "/mnt/freebox"
STREAM_CHUNK_SIZE_BYTES: int = 64 * 1024  # 64 KB streaming buffer
RANGE_HEADER_REGEX: re.Pattern[str] = re.compile(r"^bytes=(\d+)-(\d+)?$")
# Only the media libraries are reachable from the WAN (keep in sync with scan_freebox_targets.ELIGIBLE_SUBDIRECTORIES)
SERVED_PREFIXES: tuple[str, ...] = ("Disque_1/Téléchargements/", "Disque_2/Téléchargements/", "Disque_1_1/Vidéos/")


def is_served_path(rel_path: str) -> bool:
    """True when the URL-decoded relative path lies inside a served media library."""
    return unicodedata.normalize("NFC", rel_path).startswith(SERVED_PREFIXES)


def parse_byte_range(range_header: str | None, file_size: int) -> Tuple[int, int] | None:
    """Parses an RFC 7233 HTTP Range header into an inclusive (start, end) byte tuple.

    # Arguments
    * `range_header`: Raw header string (e.g. 'bytes=1024-2048' or 'bytes=1024-')
    * `file_size`: Total file size in bytes

    # Returns
    `(start_byte, end_byte)` if valid and satisfiable, otherwise `None`.
    """
    if not range_header or file_size == 0:
        return None
    match = RANGE_HEADER_REGEX.match(range_header.strip())
    if not match:
        return None
    start = int(match.group(1))
    end = int(match.group(2)) if match.group(2) else file_size - 1
    if start > end or start >= file_size:
        return None
    end = min(end, file_size - 1)
    return (start, end)


class FreeboxRangeHTTPHandler(BaseHTTPRequestHandler):
    """HTTP request handler supporting GET, HEAD and RFC 7233 Byte-Range streaming."""

    server_root: Path = Path(DEFAULT_SERVE_ROOT)

    def log_message(self, format: str, *args: object) -> None:
        """Emits concise, timestamped request logs to stdout."""
        sys.stdout.write(f"[{self.log_date_time_string()}] {self.address_string()} - {format % args}\n")
        sys.stdout.flush()

    def _resolve_target_path(self) -> Path | None:
        """Resolves and bounds-checks the requested URL path against server_root."""
        unquoted = urllib.parse.unquote(self.path.split("?")[0].lstrip("/"))
        if not is_served_path(unquoted):
            return None
        resolved = (self.server_root / unquoted).resolve()
        try:
            resolved.relative_to(self.server_root.resolve())
        except ValueError:
            return None
        return resolved if resolved.is_file() else None

    def do_HEAD(self) -> None:
        """Handles HTTP HEAD requests for probing file presence and size."""
        target = self._resolve_target_path()
        if not target:
            self.send_error(HTTPStatus.NOT_FOUND, "File not found")
            return
        stat_res = target.stat()
        mime_type, _ = mimetypes.guess_type(str(target))
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", mime_type or "application/octet-stream")
        self.send_header("Content-Length", str(stat_res.st_size))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()

    def _stream_range(self, f: Any, start: int, content_length: int) -> None:
        """Streams a bounded slice of bytes to client."""
        f.seek(start)
        remaining = content_length
        while remaining > 0:
            chunk = f.read(min(STREAM_CHUNK_SIZE_BYTES, remaining))
            if not chunk:
                break
            self.wfile.write(chunk)
            remaining -= len(chunk)

    def _stream_full(self, f: Any) -> None:
        """Streams entire file to client."""
        while chunk := f.read(STREAM_CHUNK_SIZE_BYTES):
            self.wfile.write(chunk)

    def do_GET(self) -> None:
        """Handles HTTP GET requests with transparent 206 Partial Content byte ranges."""
        target = self._resolve_target_path()
        if not target:
            self.send_error(HTTPStatus.NOT_FOUND, "File not found")
            return

        file_size = target.stat().st_size
        mime_type, _ = mimetypes.guess_type(str(target))
        content_type = mime_type or "application/octet-stream"
        range_header = self.headers.get("Range")
        byte_range = parse_byte_range(range_header, file_size)

        if range_header and byte_range is None:
            self.send_response(HTTPStatus.REQUESTED_RANGE_NOT_SATISFIABLE)
            self.send_header("Content-Range", f"bytes */{file_size}")
            self.end_headers()
            return

        self._serve_content(target, byte_range, file_size, content_type)

    def _serve_content(
        self, target: Path, byte_range: tuple[int, int] | None, file_size: int, content_type: str
    ) -> None:
        """Dispatches range or full response with error isolation."""
        try:
            with target.open("rb") as f:
                if byte_range is not None:
                    start, end = byte_range
                    content_length = end - start + 1
                    self.send_response(HTTPStatus.PARTIAL_CONTENT)
                    self.send_header("Content-Type", content_type)
                    self.send_header("Content-Length", str(content_length))
                    self.send_header("Content-Range", f"bytes {start}-{end}/{file_size}")
                    self.send_header("Accept-Ranges", "bytes")
                    self.end_headers()
                    self._stream_range(f, start, content_length)
                else:
                    self.send_response(HTTPStatus.OK)
                    self.send_header("Content-Type", content_type)
                    self.send_header("Content-Length", str(file_size))
                    self.send_header("Accept-Ranges", "bytes")
                    self.end_headers()
                    self._stream_full(f)
        except (BrokenPipeError, ConnectionResetError):
            pass


def main() -> int:
    """CLI entry point for running the Freebox Range HTTP Gateway."""
    parser = argparse.ArgumentParser(description="Freebox Range HTTP Gateway for Remote FFmpeg Streaming")
    parser.add_argument("--host", default=DEFAULT_GATEWAY_HOST, help=f"Listen host (default: {DEFAULT_GATEWAY_HOST})")
    parser.add_argument(
        "--port", type=int, default=DEFAULT_GATEWAY_PORT, help=f"Listen port (default: {DEFAULT_GATEWAY_PORT})"
    )
    parser.add_argument(
        "--root", default=DEFAULT_SERVE_ROOT, help=f"Directory to serve (default: {DEFAULT_SERVE_ROOT})"
    )
    args = parser.parse_args()

    serve_root = Path(args.root).resolve()
    if not serve_root.is_dir():
        print(f"❌ Error: serve root directory '{serve_root}' does not exist.", file=sys.stderr)
        return 1

    FreeboxRangeHTTPHandler.server_root = serve_root
    ThreadingHTTPServer.allow_reuse_address = True
    server = ThreadingHTTPServer((args.host, args.port), FreeboxRangeHTTPHandler)
    print(f"🚀 Freebox Range HTTP Gateway listening on http://{args.host}:{args.port}/ (Root: {serve_root})")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n🛑 Gateway shutting down gracefully.")
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
