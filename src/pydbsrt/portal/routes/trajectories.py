"""HTTP route handlers for spatiotemporal trajectory audit files, queries, and on-demand runs."""

from __future__ import annotations

import json

from pydbsrt.portal.config import (
    DEFAULT_TRAJECTORY_PAGE_SIZE,
    HTTP_NOT_FOUND,
    HTTP_OK,
    HTTP_SERVER_ERROR,
    TRAJECTORY_DEFAULT_FRAMES,
    TRAJECTORY_SAMPLE_CUTS,
)
from pydbsrt.portal.models import TrajectoriesQueryCriteria
from pydbsrt.portal.routes.common import PortalHandlerProtocol, parse_int_query_param
from pydbsrt.portal.services.trajectory_service import (
    get_trajectory_record_detail,
    list_trajectory_audit_files,
    query_trajectory_records,
    run_on_demand_trajectory,
)


def handle_trajectory_get_routes(handler: PortalHandlerProtocol, path: str, query: dict[str, list[str]]) -> bool:
    """Dispatches GET /api/trajectory/files, /api/trajectories, and /api/trajectory/details."""
    if path == "/api/trajectory/files":
        handler._send_json({"status": "ok", "files": list_trajectory_audit_files()})
        return True
    if path == "/api/trajectories":
        criteria = TrajectoriesQueryCriteria(
            file_name=query.get("file", [""])[0],
            page=parse_int_query_param(query, "page", 1),
            limit=parse_int_query_param(query, "limit", DEFAULT_TRAJECTORY_PAGE_SIZE),
            status_filter=query.get("filter", ["all"])[0],
            search_q=query.get("q", [""])[0],
            sort_by=query.get("sort", ["name_asc"])[0],
        )
        handler._send_json(query_trajectory_records(criteria))
        return True
    if path == "/api/trajectory/details":
        f_name = query.get("file", [""])[0]
        target = query.get("target", [""])[0]
        result = get_trajectory_record_detail(f_name, target)
        status_code = HTTP_OK if result.get("status") == "ok" else HTTP_NOT_FOUND
        handler._send_json(result, status=status_code)
        return True
    return False


def handle_trajectory_post_routes(handler: PortalHandlerProtocol, path: str) -> bool:
    """Dispatches POST /api/trajectory/run."""
    if path != "/api/trajectory/run":
        return False
    length = int(handler.headers.get("Content-Length", 0))
    body = handler.rfile.read(length).decode("utf-8") if length > 0 else "{}"
    try:
        data = json.loads(body)
    except Exception:
        data = {}
    target = data.get("target", "")
    cuts = int(data.get("cuts", TRAJECTORY_SAMPLE_CUTS))
    frames = int(data.get("frames", TRAJECTORY_DEFAULT_FRAMES))
    result = run_on_demand_trajectory(target, cuts=cuts, frames=frames)
    status_code = HTTP_OK if result.get("status") == "ok" else HTTP_SERVER_ERROR
    handler._send_json(result, status=status_code)
    return True
