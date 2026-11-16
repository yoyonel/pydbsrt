"""HTTP route dispatchers for pydbsrt Portal."""

from __future__ import annotations

from pydbsrt.portal.routes.assets import handle_asset_routes
from pydbsrt.portal.routes.common import PortalHandlerProtocol, parse_int_query_param
from pydbsrt.portal.routes.phashes import handle_phash_routes, stream_artifact_download
from pydbsrt.portal.routes.sources import handle_source_routes
from pydbsrt.portal.routes.trajectories import (
    handle_trajectory_get_routes,
    handle_trajectory_post_routes,
)

__all__ = [
    "PortalHandlerProtocol",
    "handle_asset_routes",
    "handle_phash_routes",
    "handle_source_routes",
    "handle_trajectory_get_routes",
    "handle_trajectory_post_routes",
    "parse_int_query_param",
    "stream_artifact_download",
]
