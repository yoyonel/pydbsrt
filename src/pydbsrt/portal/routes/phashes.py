"""HTTP route handlers for tripartite pHash reconciliation, microscopic details, and artifact download."""

from __future__ import annotations

import hashlib
import re
import urllib.parse

from pydbsrt.portal.config import ARTIFACT_STREAM_CHUNK_BYTES, DEFAULT_PAGE_SIZE, HTTP_NOT_FOUND, HTTP_OK, REPO_ROOT
from pydbsrt.portal.models import PhashesQueryCriteria
from pydbsrt.portal.routes.common import PortalHandlerProtocol, parse_int_query_param
from pydbsrt.portal.services.phash_microscopy import inspect_phash_microscopic
from pydbsrt.portal.services.phash_reconciliation import query_phashes_list
from scripts.data_access.resolver import resolve_local_artifact_download


def stream_artifact_download(handler: PortalHandlerProtocol, query: dict[str, list[str]]) -> None:
    """Streams a validated .phash, .ahash, sidecar .json, or couplings.json artifact."""
    kind = query.get("kind", [""])[0]
    filename = query.get("file", [""])[0]
    artifact_path, reason, status_code = resolve_local_artifact_download(REPO_ROOT / "data", kind, filename)
    if artifact_path is None:
        handler._send_json({"error": reason, "kind": kind, "file": filename}, status=status_code)
        return

    hasher = hashlib.sha256()
    with artifact_path.open("rb") as fh_hash:
        while True:
            blk = fh_hash.read(ARTIFACT_STREAM_CHUNK_BYTES)
            if not blk:
                break
            hasher.update(blk)
    file_size = artifact_path.stat().st_size

    ascii_fallback = re.sub(r"[^A-Za-z0-9._-]", "_", artifact_path.name)
    utf8_quoted = urllib.parse.quote(artifact_path.name, safe="")
    handler.send_response(HTTP_OK)
    handler.send_header("Content-Type", "application/octet-stream")
    handler.send_header("Content-Length", str(file_size))
    handler.send_header(
        "Content-Disposition",
        f"attachment; filename=\"{ascii_fallback}\"; filename*=UTF-8''{utf8_quoted}",
    )
    handler.send_header("X-Checksum-SHA256", hasher.hexdigest())
    handler.send_header("Access-Control-Allow-Origin", "*")
    handler.end_headers()

    with artifact_path.open("rb") as fh_out:
        while True:
            chunk = fh_out.read(ARTIFACT_STREAM_CHUNK_BYTES)
            if not chunk:
                break
            handler.wfile.write(chunk)


def handle_phash_routes(handler: PortalHandlerProtocol, path: str, query: dict[str, list[str]]) -> bool:
    """Dispatches GET /api/artifact/download, /api/phashes, and /api/phash/details."""
    if path == "/api/artifact/download":
        stream_artifact_download(handler, query)
        return True
    if path == "/api/phashes":
        criteria = PhashesQueryCriteria(
            page=parse_int_query_param(query, "page", 1),
            limit=parse_int_query_param(query, "limit", DEFAULT_PAGE_SIZE),
            status_filter=query.get("filter", ["all"])[0],
            search_q=query.get("q", [""])[0],
            sort_by=query.get("sort", ["id_asc"])[0],
        )
        handler._send_json(query_phashes_list(criteria))
        return True
    if path == "/api/phash/details":
        file_ident = query.get("file", [""])[0]
        hash_ident = query.get("moviehash", [""])[0]
        primary = file_ident or hash_ident
        fallback = hash_ident if file_ident else ""
        result = inspect_phash_microscopic(primary, fallback_identifier=fallback)
        status_code = HTTP_OK if result.get("status") == "ok" else HTTP_NOT_FOUND
        handler._send_json(result, status=status_code)
        return True
    return False
