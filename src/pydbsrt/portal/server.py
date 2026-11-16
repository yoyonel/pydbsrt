"""HTTP server, request handler, and static SPA delivery for pydbsrt Portal."""

from __future__ import annotations

import http.server
import json
import socketserver
import urllib.parse
from typing import Any

from pydbsrt.portal.config import DEFAULT_PORT, HTTP_NOT_FOUND, HTTP_OK, STATIC_DIR
from pydbsrt.portal.routes import (
    handle_asset_routes,
    handle_phash_routes,
    handle_source_routes,
    handle_trajectory_get_routes,
    handle_trajectory_post_routes,
)


class PortalRequestHandler(http.server.SimpleHTTPRequestHandler):
    """Custom HTTP handler serving API JSON endpoints and clean use-case HTML pages."""

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, directory=str(STATIC_DIR), **kwargs)

    def do_HEAD(self) -> None:
        self.do_GET()

    def _handle_favicon(self, path: str) -> bool:
        if path != "/favicon.ico":
            return False
        self.send_response(HTTP_OK)
        self.send_header("Content-Type", "image/svg+xml")
        svg_icon = b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>\xf0\x9f\x8e\xac</text></svg>"
        self.send_header("Content-Length", str(len(svg_icon)))
        self.end_headers()
        self.wfile.write(svg_icon)
        return True

    def _handle_api(self, path: str, query: dict[str, list[str]]) -> bool:
        return (
            handle_source_routes(self, path, query)
            or handle_asset_routes(self, path, query)
            or handle_phash_routes(self, path, query)
            or handle_trajectory_get_routes(self, path, query)
        )

    def do_POST(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        if handle_trajectory_post_routes(self, parsed.path):
            return
        self.send_error(HTTP_NOT_FOUND, "Not found")

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query)

        if self._handle_favicon(path):
            return
        if self._handle_api(path, query):
            return

        if path in ("/", "/sources", "/sources/"):
            self.path = "/sources.html"

        super().do_GET()

    def _send_json(self, payload: Any, status: int = HTTP_OK) -> None:
        raw = json.dumps(payload, indent=2, default=str).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(raw)


class ReusableThreadingTCPServer(socketserver.ThreadingTCPServer):
    """Multi-threaded TCP server allowing fast port reuse across restarts."""

    allow_reuse_address = True
    daemon_threads = True


def run_server(port: int = DEFAULT_PORT) -> None:
    """Starts the pydbsrt HTTP Portal server bound to 0.0.0.0."""
    STATIC_DIR.mkdir(parents=True, exist_ok=True)
    with ReusableThreadingTCPServer(("0.0.0.0", port), PortalRequestHandler) as httpd:
        print(f"🚀 pydbsrt Portal running on http://127.0.0.1:{port}/")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\nShutting down server.")
