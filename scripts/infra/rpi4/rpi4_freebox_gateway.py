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
import json
import mimetypes
import os
import re
import socket
import sys
import threading
import time
import unicodedata
import urllib.parse
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Tuple


def find_repo_root() -> Path:
    """Finds git repository root with appliance fallback (~/dbsrt)."""
    p = Path(__file__).resolve().parent
    for parent in [p] + list(p.parents):
        if (parent / ".git").is_dir():
            return parent
    home_dbsrt = Path.home() / "dbsrt"
    if home_dbsrt.is_dir():
        return home_dbsrt
    return Path.cwd()


REPO_ROOT = find_repo_root()

# Named Operational Constants
DEFAULT_GATEWAY_HOST: str = "127.0.0.1"
DEFAULT_GATEWAY_PORT: int = 18089
DEFAULT_SERVE_ROOT: str = "/mnt/freebox"
STREAM_CHUNK_SIZE_BYTES: int = 64 * 1024  # 64 KB streaming buffer
CATALOG_CACHE_TTL_SECONDS: float = 300.0
SOCKET_CHECK_TIMEOUT_SECONDS: float = 0.5
RANGE_HEADER_REGEX: re.Pattern[str] = re.compile(r"^bytes=(\d+)-(\d+)?$")
# Only the media libraries are reachable from the WAN (keep in sync with scan_freebox_targets.ELIGIBLE_SUBDIRECTORIES)
SERVED_PREFIXES: tuple[str, ...] = ("Disque_1/Téléchargements/", "Disque_2/Téléchargements/", "Disque_1_1/Vidéos/")
SUPPORTED_VIDEO_EXTENSIONS: tuple[str, ...] = (".mp4", ".mkv", ".avi", ".mov", ".webm", ".m4v")
EXCLUDED_DIRECTORY_NAMES: frozenset[str] = frozenset(
    {
        "photos",
        "backup",
        "enregistrements",
        ".cameras",
        ".fbxgrabberd",
        ".fbxtimeshifting",
        "lost+found",
        ".trash-1000",
        "@eadir",
    }
)

_CATALOG_LOCK: threading.Lock = threading.Lock()
_CATALOG_CACHE: dict[str, tuple[float, list[dict[str, Any]]]] = {}


def is_served_path(rel_path: str) -> bool:
    """True when the URL-decoded relative path lies inside a served media library."""
    return unicodedata.normalize("NFC", rel_path).startswith(SERVED_PREFIXES)


def _scan_single_prefix(serve_root: Path, prefix: str) -> list[dict[str, Any]]:
    """Walks one eligible Freebox subdirectory and returns discovered video descriptors."""
    sub_root = serve_root / prefix.rstrip("/")
    if not sub_root.is_dir():
        return []
    discovered: list[dict[str, Any]] = []
    for dirpath, dirnames, filenames in os.walk(sub_root):
        dirnames[:] = sorted(d for d in dirnames if not d.startswith(".") and d.lower() not in EXCLUDED_DIRECTORY_NAMES)
        dir_obj = Path(dirpath)
        for fname in sorted(filenames):
            if fname.startswith(".") or Path(fname).suffix.lower() not in SUPPORTED_VIDEO_EXTENSIONS:
                continue
            full_path = dir_obj / fname
            try:
                st = full_path.stat()
                rel_posix = full_path.relative_to(serve_root).as_posix()
            except (OSError, ValueError):
                continue
            discovered.append(
                {
                    "path": rel_posix,
                    "filename": unicodedata.normalize("NFC", fname),
                    "size_bytes": int(st.st_size),
                    "mtime": int(st.st_mtime),
                }
            )
    return discovered


def scan_served_catalog(serve_root: Path, force: bool = False) -> list[dict[str, Any]]:
    """Returns thread-safe TTL-cached list of eligible video files across SERVED_PREFIXES."""
    root_resolved = serve_root.resolve()
    cache_key = str(root_resolved)
    now = time.monotonic()
    if not force:
        with _CATALOG_LOCK:
            cached = _CATALOG_CACHE.get(cache_key)
            if cached is not None and (now - cached[0]) < CATALOG_CACHE_TTL_SECONDS:
                return cached[1]

    items: list[dict[str, Any]] = []
    if root_resolved.is_dir():
        for prefix in SERVED_PREFIXES:
            items.extend(_scan_single_prefix(root_resolved, prefix))
    items.sort(key=lambda entry: entry["path"].lower())

    with _CATALOG_LOCK:
        _CATALOG_CACHE[cache_key] = (time.monotonic(), items)
    return items


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
        root_resolved = self.server_root.resolve()
        for form in (unquoted, unicodedata.normalize("NFC", unquoted), unicodedata.normalize("NFD", unquoted)):
            try:
                resolved = (self.server_root / form).resolve()
                resolved.relative_to(root_resolved)
                if resolved.is_file():
                    return resolved
            except (OSError, ValueError):
                continue
        return None

    def _route_name(self) -> str:
        """Returns normalized top-level route segment without query parameters."""
        return self.path.split("?")[0].strip("/")

    def _is_force_requested(self) -> bool:
        """Returns True when query string sets force=1, force=true, or force=yes."""
        parsed = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(parsed.query)
        return query.get("force", ["0"])[0].lower() in ("1", "true", "yes")

    def _send_json_payload(self, data: dict[str, Any]) -> None:
        """Serializes and writes an UTF-8 JSON HTTP 200 response with CORS headers."""
        payload = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(payload)

    def do_HEAD(self) -> None:
        """Handles HTTP HEAD requests for probing file presence, size, catalog, or health."""
        if self._route_name() in ("health", "catalog"):
            self.send_response(HTTPStatus.OK)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", "0")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            return

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
        """Handles HTTP GET requests for /health, /catalog, or 206 Partial Content media streams."""
        route = self._route_name()
        if route == "health":
            mounted = sorted(p.name for p in self.server_root.glob("Disque_*") if p.is_dir())
            items = scan_served_catalog(self.server_root, force=self._is_force_requested())
            self._send_json_payload(
                {
                    "status": "ok",
                    "service": "dbsrt-freebox-gateway",
                    "serve_root": str(self.server_root),
                    "mounted_shares": mounted,
                    "media_count": len(items),
                }
            )
            return

        if route == "catalog":
            items = scan_served_catalog(self.server_root, force=self._is_force_requested())
            self._send_json_payload(
                {
                    "status": "ok",
                    "service": "dbsrt-freebox-gateway",
                    "total": len(items),
                    "items": items,
                }
            )
            return

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


def _handle_check_mode(host: str, port: int, serve_root: Path) -> int:
    """Inspects root mount and port availability without starting continuous server."""
    mounted = sorted(p.name for p in serve_root.glob("Disque_*") if p.is_dir()) if serve_root.is_dir() else []
    media_items = scan_served_catalog(serve_root, force=True) if serve_root.is_dir() else []
    port_available = True
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.settimeout(SOCKET_CHECK_TIMEOUT_SECONDS)
        if s.connect_ex((host, port)) == 0:
            port_available = False

    print("=== DBSrt Freebox Range HTTP Gateway Inspection ===")
    print(f"✓ REPO_ROOT     : {REPO_ROOT}")
    print(f"✓ Host / Port   : {host}:{port} ({'listening active' if not port_available else 'port free'})")
    print(f"✓ Serve root    : {serve_root} ({'found' if serve_root.is_dir() else 'missing'})")
    print(f"✓ Mounted shares: {len(mounted)} Disque_* shares ({', '.join(mounted) if mounted else 'none'})")
    print(f"✓ Eligible media: {len(media_items)} containers")
    return 0


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
    parser.add_argument(
        "--check",
        "--dry-run",
        dest="check",
        action="store_true",
        help="Inspect serve root and network availability without serving continuously",
    )
    args = parser.parse_args()

    serve_root = Path(args.root).resolve()
    if args.check:
        return _handle_check_mode(args.host, args.port, serve_root)

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
