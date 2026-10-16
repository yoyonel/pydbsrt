#!/usr/bin/env python3
"""Intelligent launcher for MkDocs documentation server.

Auto-detects free ports, respects environment variables (DOC_PORT, DOC_HOST),
and prevents address collision errors without touching foreign processes.
"""

import os
import socket
import sys
from typing import List


def find_free_port(host: str, start_port: int, max_attempts: int = 100) -> int:
    """Find the first available TCP port starting from start_port."""
    port = start_port
    for _ in range(max_attempts):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            try:
                probe.bind((host, port))
                return port
            except OSError:
                port += 1
    raise RuntimeError(f"Unable to find an open port in range {start_port}..{start_port + max_attempts}")


def main(argv: List[str] | None = None) -> int:
    if argv is None:
        argv = sys.argv[1:]

    host = os.environ.get("DOC_HOST", "127.0.0.1")
    requested_port = int(os.environ.get("DOC_PORT", os.environ.get("PORT", "8000")))

    port = find_free_port(host, requested_port)
    if port != requested_port:
        print(f"[docs] Note: Port {requested_port} is already in use. Auto-selected available port {port}.")

    print(f"[docs] Starting documentation server on http://{host}:{port}/")

    cmd = ["mkdocs", "serve", "-a", f"{host}:{port}"] + argv
    # Replace current process with mkdocs
    os.execvp("mkdocs", cmd)


if __name__ == "__main__":
    sys.exit(main())
