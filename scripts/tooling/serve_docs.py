#!/usr/bin/env python3
"""Intelligent launcher for MkDocs documentation server.

Auto-detects free ports, respects environment variables (DOC_HOST, DOC_PORT),
anchors mkdocs to repo root, and prevents address collision errors without touching foreign processes.
"""

from __future__ import annotations

import argparse
import os
import shutil
import socket
import sys
from pathlib import Path

DEFAULT_DOC_HOST: str = "127.0.0.1"
DEFAULT_DOC_PORT: int = 8000
DEFAULT_PORT_SCAN_ATTEMPTS: int = 100


def find_repo_root() -> Path:
    """Dynamically locates repository root with fallback to ~/dbsrt."""
    current = Path(__file__).resolve().parent
    for p in [current, *current.parents]:
        if (p / ".git").is_dir() or (p / "Taskfile.yml").is_file():
            return p
    appliance = Path.home() / "dbsrt"
    if appliance.is_dir():
        return appliance
    return Path.cwd()


def find_free_port(host: str, start_port: int, max_attempts: int = DEFAULT_PORT_SCAN_ATTEMPTS) -> int:
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


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Intelligent launcher for MkDocs documentation server.",
        add_help=True,
    )
    parser.add_argument(
        "--host",
        type=str,
        default=os.environ.get("DOC_HOST", DEFAULT_DOC_HOST),
        help=f"Listen host (default: {DEFAULT_DOC_HOST})",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=int(os.environ.get("DOC_PORT", os.environ.get("PORT", str(DEFAULT_DOC_PORT)))),
        help=f"Preferred listen port (default: {DEFAULT_DOC_PORT})",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Check port availability and mkdocs configuration without serving",
    )

    args, remaining_argv = parser.parse_known_args(argv)

    repo_root = find_repo_root()
    mkdocs_config = repo_root / "mkdocs.yml"
    if not mkdocs_config.is_file():
        print(f"Error: mkdocs.yml not found at {mkdocs_config}", file=sys.stderr)
        return 1

    port = find_free_port(args.host, args.port)
    if args.check:
        print(f"REPO_ROOT      : {repo_root}")
        print(f"MkDocs config  : {mkdocs_config} (OK)")
        print(f"Host / Port    : {args.host}:{port} (free, preferred was {args.port})")
        print(f"mkdocs binary  : {shutil.which('mkdocs') or 'not found'}")
        return 0

    if port != args.port:
        print(f"[docs] Note: Port {args.port} is already in use. Auto-selected available port {port}.")

    print(f"[docs] Starting documentation server on http://{args.host}:{port}/")

    cmd = ["mkdocs", "serve", "-f", str(mkdocs_config), "-a", f"{args.host}:{port}"] + remaining_argv
    os.execvp("mkdocs", cmd)


if __name__ == "__main__":
    sys.exit(main())
