"""HTTP route handlers for media sources, channel probes, file explorer, and ffprobe details."""

from __future__ import annotations

from pydbsrt.portal.config import DEFAULT_PAGE_SIZE
from pydbsrt.portal.routes.common import PortalHandlerProtocol, parse_int_query_param
from pydbsrt.portal.services.ffprobe_service import inspect_file_details
from pydbsrt.portal.services.listing_service import list_source_files
from pydbsrt.portal.services.probe_service import execute_single_probe, get_sources_metadata


def handle_source_routes(handler: PortalHandlerProtocol, path: str, query: dict[str, list[str]]) -> bool:
    """Dispatches GET /api/sources, /api/probe, /api/files, and /api/file/details."""
    if path == "/api/sources":
        handler._send_json(get_sources_metadata())
        return True
    if path == "/api/probe":
        result = execute_single_probe(query.get("source", [""])[0], query.get("profile", [""])[0])
        handler._send_json(result)
        return True
    if path == "/api/files":
        page = parse_int_query_param(query, "page", 1)
        limit = parse_int_query_param(query, "limit", DEFAULT_PAGE_SIZE)
        force = query.get("force", ["0"])[0] in ("1", "true", "yes")
        result = list_source_files(
            query.get("source", [""])[0],
            query.get("profile", [""])[0],
            query.get("subdir", [""])[0],
            page,
            limit,
            query.get("q", [""])[0],
            query.get("sort", ["name_asc"])[0],
            force=force,
        )
        handler._send_json(result)
        return True
    if path == "/api/file/details":
        force = query.get("force", ["0"])[0] in ("1", "true", "yes")
        result = inspect_file_details(
            query.get("source", [""])[0],
            query.get("profile", [""])[0],
            query.get("path", [""])[0],
            force=force,
        )
        handler._send_json(result)
        return True
    return False
