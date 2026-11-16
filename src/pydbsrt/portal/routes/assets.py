"""HTTP route handlers for PostgreSQL media assets catalog and detail endpoints."""

from __future__ import annotations

from pydbsrt.portal.config import DEFAULT_PAGE_SIZE, HTTP_NOT_FOUND, HTTP_OK, HTTP_SERVER_ERROR
from pydbsrt.portal.routes.common import PortalHandlerProtocol, parse_int_query_param
from pydbsrt.portal.services.assets_service import get_pg_asset_detail, get_pg_assets


def _handle_api_assets(handler: PortalHandlerProtocol, query: dict[str, list[str]]) -> None:
    """Handles GET /api/assets with pagination, filters, and sorting."""
    page_val = parse_int_query_param(query, "page", 1)
    limit_val = parse_int_query_param(query, "limit", DEFAULT_PAGE_SIZE)
    result = get_pg_assets(
        page=page_val,
        limit=limit_val,
        status_filter=query.get("status", ["all"])[0],
        source_filter=query.get("source", [""])[0],
        search_q=query.get("q", [""])[0],
        sort_by=query.get("sort", ["title_asc"])[0],
    )
    handler._send_json(result)


def handle_asset_routes(handler: PortalHandlerProtocol, path: str, query: dict[str, list[str]]) -> bool:
    """Dispatches GET /api/assets and /api/asset/details."""
    if path == "/api/assets":
        _handle_api_assets(handler, query)
        return True
    if path == "/api/asset/details":
        media_id = parse_int_query_param(query, "id", 0)
        result = get_pg_asset_detail(media_id)
        status_code = (
            HTTP_OK
            if result.get("status") == "ok"
            else (HTTP_NOT_FOUND if result.get("status") == "not_found" else HTTP_SERVER_ERROR)
        )
        handler._send_json(result, status=status_code)
        return True
    return False
