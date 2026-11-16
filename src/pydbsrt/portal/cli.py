"""CLI entrypoint for pydbsrt MediaSources Portal (one-shot CGI JSON queries and HTTP daemon)."""

from __future__ import annotations

import argparse
import json
from collections.abc import Callable
from typing import Any

from pydbsrt.portal.config import DEFAULT_PAGE_SIZE, DEFAULT_PORT
from pydbsrt.portal.models import PhashesQueryCriteria, TrajectoriesQueryCriteria
from pydbsrt.portal.server import run_server
from pydbsrt.portal.services.assets_service import get_pg_asset_detail, get_pg_assets
from pydbsrt.portal.services.ffprobe_service import inspect_file_details
from pydbsrt.portal.services.listing_service import list_source_files
from pydbsrt.portal.services.phash_microscopy import inspect_phash_microscopic
from pydbsrt.portal.services.phash_reconciliation import query_phashes_list
from pydbsrt.portal.services.probe_service import execute_single_probe, get_sources_metadata
from pydbsrt.portal.services.trajectory_service import (
    get_trajectory_record_detail,
    list_trajectory_audit_files,
    query_trajectory_records,
)


def build_arg_parser() -> argparse.ArgumentParser:
    """Builds the Portal CLI argument parser preserving all flags for o2switch CGI and local use."""
    parser = argparse.ArgumentParser(description="pydbsrt MediaSources Portal Server")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT, help=f"HTTP port (default {DEFAULT_PORT})")
    parser.add_argument("--metadata", action="store_true", help="Print sources metadata JSON and exit")
    parser.add_argument("--probe", nargs=2, metavar=("SOURCE", "PROFILE"), help="Execute single probe and exit")
    parser.add_argument("--files", nargs=2, metavar=("SOURCE", "PROFILE"), help="List files for source/profile")
    parser.add_argument("--subdir", default="", help="Subdirectory path within source")
    parser.add_argument("--page", type=int, default=1, help="Page number (default 1)")
    parser.add_argument("--limit", type=int, default=DEFAULT_PAGE_SIZE, help="Page limit (default 25)")
    parser.add_argument("--search", default="", help="Search / filter query")
    parser.add_argument(
        "--sort", default="name_asc", help="Sort order (name_asc, name_desc, size_desc, size_asc, date_desc)"
    )
    parser.add_argument("--details", nargs=3, metavar=("SOURCE", "PROFILE", "PATH"), help="Inspect multimedia details")
    parser.add_argument("--assets", action="store_true", help="List PG media assets (JSON)")
    parser.add_argument("--asset-id", type=int, help="Single media asset ID for detailed inspection")
    parser.add_argument(
        "--status", default="all", help="Status filter for assets (all, nominal, missing_phash, unmapped_path)"
    )
    parser.add_argument("--source", default="", help="Source filter for assets")
    parser.add_argument("--force", action="store_true", help="Bypass cache and force fresh live probe")
    parser.add_argument("--phashes", action="store_true", help="List tripartite pHash fingerprints (JSON)")
    parser.add_argument("--phash-moviehash", default="", help="Inspect microscopic details of single phash")
    parser.add_argument("--phash-fallback", default="", help="Secondary fallback moviehash for microscopic inspection")
    parser.add_argument(
        "--phash-filter",
        default="all",
        help="Filter for phashes (all, aligned_3_3, missing_pg, uncompiled, orphan_catalog)",
    )
    parser.add_argument("--trajectory-files", action="store_true", help="List available trajectory audit files (JSON)")
    parser.add_argument("--trajectories", action="store_true", help="Query spatiotemporal trajectory records (JSON)")
    parser.add_argument("--trajectory-file", default="", help="Specific trajectory file to query")
    parser.add_argument("--trajectory-details", default="", help="Inspect detailed cuts sequence for target media")
    parser.add_argument(
        "--trajectory-filter",
        default="all",
        help="Trajectory filter (all, coherent, matched, divergent, unmatched)",
    )
    return parser


def _print_json(payload: Any) -> None:
    """Prints JSON payload to stdout using standard indentation and string fallback."""
    print(json.dumps(payload, indent=2, default=str))


def _dispatch_cli_action(args: argparse.Namespace) -> bool:
    """Executes one-shot CLI query actions and returns True if an action matched."""
    handlers: list[tuple[bool, Callable[[], Any]]] = [
        (bool(args.metadata), get_sources_metadata),
        (bool(args.probe), lambda: execute_single_probe(args.probe[0], args.probe[1])),
        (
            bool(args.files),
            lambda: list_source_files(
                args.files[0],
                args.files[1],
                args.subdir,
                args.page,
                args.limit,
                args.search,
                args.sort,
                force=args.force,
            ),
        ),
        (
            bool(args.details),
            lambda: inspect_file_details(args.details[0], args.details[1], args.details[2], force=args.force),
        ),
        (
            bool(args.assets),
            lambda: get_pg_assets(
                page=args.page,
                limit=args.limit,
                status_filter=args.status,
                source_filter=args.source,
                search_q=args.search,
                sort_by=args.sort,
            ),
        ),
        (bool(args.asset_id), lambda: get_pg_asset_detail(args.asset_id)),
        (
            bool(args.phashes),
            lambda: query_phashes_list(
                PhashesQueryCriteria(
                    page=args.page,
                    limit=args.limit,
                    status_filter=args.phash_filter if args.phash_filter != "all" else args.status,
                    search_q=args.search,
                    sort_by=args.sort,
                )
            ),
        ),
        (
            bool(args.phash_moviehash),
            lambda: inspect_phash_microscopic(args.phash_moviehash, fallback_identifier=args.phash_fallback),
        ),
        (bool(args.trajectory_files), lambda: {"status": "ok", "files": list_trajectory_audit_files()}),
        (
            bool(args.trajectories),
            lambda: query_trajectory_records(
                TrajectoriesQueryCriteria(
                    file_name=args.trajectory_file,
                    page=args.page,
                    limit=args.limit,
                    status_filter=args.trajectory_filter if args.trajectory_filter != "all" else args.status,
                    search_q=args.search,
                    sort_by=args.sort,
                )
            ),
        ),
        (
            bool(args.trajectory_details),
            lambda: get_trajectory_record_detail(args.trajectory_file, args.trajectory_details),
        ),
    ]
    for is_active, action_fn in handlers:
        if is_active:
            _print_json(action_fn())
            return True
    return False


def main(argv: list[str] | None = None) -> None:
    """Main CLI entrypoint for pydbsrt Portal."""
    args = build_arg_parser().parse_args(argv)
    if not _dispatch_cli_action(args):
        run_server(args.port)
