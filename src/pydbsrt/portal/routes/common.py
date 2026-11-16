"""Shared HTTP route helpers and typed handler protocol for pydbsrt Portal."""

from __future__ import annotations

from email.message import Message
from io import BufferedIOBase
from typing import Any, Protocol

from pydbsrt.portal.config import HTTP_OK


class PortalHandlerProtocol(Protocol):
    """Structural typing protocol satisfied by PortalRequestHandler."""

    headers: Message
    rfile: BufferedIOBase
    wfile: BufferedIOBase

    def _send_json(self, payload: Any, status: int = HTTP_OK) -> None: ...

    def send_response(self, code: int, message: str | None = None) -> None: ...

    def send_header(self, keyword: str, value: str) -> None: ...

    def end_headers(self) -> None: ...


def parse_int_query_param(query: dict[str, list[str]], key: str, default: int) -> int:
    """Parses a non-negative integer query parameter with a typed fallback default."""
    raw = query.get(key, [str(default)])[0]
    return int(raw) if raw.isdigit() else default
