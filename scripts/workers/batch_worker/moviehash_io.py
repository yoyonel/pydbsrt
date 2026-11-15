"""OpenSubtitles 64-bit MovieHash ranged I/O over local files, Synology FTP REST, and HTTP 206."""

from __future__ import annotations

import contextlib
import ftplib
import http.client
import unicodedata
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Callable, Dict, List, Tuple

from .models import (
    DEFAULT_FTP_PORT,
    FTP_MOVIEHASH_TIMEOUT_SECS,
    HTTP_PARTIAL_CONTENT_STATUS,
    HTTP_RANGE_TIMEOUT_SECS,
    MOVIEHASH_CHUNK_BYTES,
    MOVIEHASH_WORD_BYTES,
    UINT64_MASK,
    VideoTarget,
)


def build_stream_url(stream_base: str, rel_path: str) -> str:
    """Joins `stream_base` and `rel_path` (verbatim for FTP/local, percent-encoded UTF-8 for HTTP)."""
    clean_path = rel_path if rel_path.startswith("/") else "/" + rel_path
    if stream_base.lower().startswith(("http://", "https://")):
        clean_path = urllib.parse.quote(clean_path, safe="/")
        return f"{stream_base}{clean_path}"
    if stream_base.lower().startswith("file://"):
        return f"{stream_base.removeprefix('file://')}{clean_path}"
    if not stream_base:
        return clean_path
    return f"{stream_base}{clean_path}"


def moviehash_from_chunks(size: int, head: bytes, tail: bytes) -> str:
    """Computes OpenSubtitles 64-bit MovieHash from total size and 64 KiB head/tail chunks."""
    total = size
    for block in (head, tail):
        for idx in range(0, len(block), MOVIEHASH_WORD_BYTES):
            total += int.from_bytes(block[idx : idx + MOVIEHASH_WORD_BYTES], "little")
    return f"{total & UINT64_MASK:016x}"


def compute_local_moviehash(path: Path) -> str | None:
    """Computes OpenSubtitles 64-bit MovieHash from a local file via two 64 KiB seeks."""
    try:
        size = path.stat().st_size
        if size < MOVIEHASH_CHUNK_BYTES:
            return None
        with path.open("rb") as fh:
            head = fh.read(MOVIEHASH_CHUNK_BYTES)
            fh.seek(size - MOVIEHASH_CHUNK_BYTES)
            tail = fh.read(MOVIEHASH_CHUNK_BYTES)
    except OSError:
        return None
    if len(head) < MOVIEHASH_CHUNK_BYTES or len(tail) < MOVIEHASH_CHUNK_BYTES:
        return None
    return moviehash_from_chunks(size, head, tail)


def _ftp_path_candidates(path: str) -> Tuple[str, ...]:
    """Returns deduplicated raw, NFD, and NFC FTP path variants for Synology mixed encodings."""
    candidates: List[str] = []
    for variant in (path, unicodedata.normalize("NFD", path), unicodedata.normalize("NFC", path)):
        if variant not in candidates:
            candidates.append(variant)
    return tuple(candidates)


def _ftp_resolve_size_and_path(connect: Callable[[], ftplib.FTP], path: str) -> Tuple[str, int] | None:
    """Queries authoritative binary SIZE over FTP while resolving NFC/NFD path variants."""
    ftp = connect()
    try:
        ftp.voidcmd("TYPE I")
        for candidate in _ftp_path_candidates(path):
            try:
                size = ftp.size(candidate)
            except ftplib.error_perm:
                continue
            if size is not None:
                return candidate, int(size)
        return None
    except ftplib.all_errors:
        return None
    finally:
        with contextlib.suppress(Exception):
            ftp.close()


def _ftp_read_at(connect: Callable[[], ftplib.FTP], path: str, offset: int) -> bytes:
    """Reads `MOVIEHASH_CHUNK_BYTES` at `offset` in binary mode in a dedicated FTP session."""
    ftp = connect()
    try:
        ftp.voidcmd("TYPE I")
        conn = ftp.transfercmd(f"RETR {path}", rest=offset)
        data = bytearray()
        try:
            while len(data) < MOVIEHASH_CHUNK_BYTES:
                chunk = conn.recv(MOVIEHASH_CHUNK_BYTES - len(data))
                if not chunk:
                    break
                data += chunk
        finally:
            conn.close()
        with contextlib.suppress(ftplib.error_temp, ftplib.error_reply, EOFError):
            ftp.voidresp()
        return bytes(data)
    finally:
        with contextlib.suppress(Exception):
            ftp.close()


def _http_file_size(url: str) -> int | None:
    """Resolves remote HTTP file size via HEAD Content-Length or 1-byte Range Content-Range."""
    try:
        head_req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(head_req, timeout=HTTP_RANGE_TIMEOUT_SECS) as resp:
            content_len = resp.headers.get("Content-Length", "").strip()
            if content_len.isdigit():
                return int(content_len)
    except (OSError, http.client.HTTPException):
        pass
    try:
        probe_req = urllib.request.Request(url, headers={"Range": "bytes=0-0"})
        with urllib.request.urlopen(probe_req, timeout=HTTP_RANGE_TIMEOUT_SECS) as resp:
            if resp.status != HTTP_PARTIAL_CONTENT_STATUS:
                return None
            content_range = resp.headers.get("Content-Range", "")
            total = content_range.rsplit("/", 1)[-1].strip()
            return int(total) if total.isdigit() else None
    except (OSError, http.client.HTTPException):
        return None


def _http_content_range_matches(content_range: str, offset: int, expected_total: int | None) -> bool:
    """Validates `bytes <offset>-<end>/<total>` against requested offset and expected total size."""
    range_spec, sep, total_str = content_range.rpartition("/")
    if sep != "/" or not range_spec.startswith(f"bytes {offset}-"):
        return False
    clean_total = total_str.strip()
    if not clean_total.isdigit():
        return False
    return expected_total is None or int(clean_total) == expected_total


def _http_read_at(url: str, offset: int, expected_total: int | None = None) -> bytes:
    """Reads `MOVIEHASH_CHUNK_BYTES` at `offset` via HTTP 206 Range request with Content-Range verification."""
    request = urllib.request.Request(url, headers={"Range": f"bytes={offset}-{offset + MOVIEHASH_CHUNK_BYTES - 1}"})
    with urllib.request.urlopen(request, timeout=HTTP_RANGE_TIMEOUT_SECS) as resp:
        if resp.status != HTTP_PARTIAL_CONTENT_STATUS or not _http_content_range_matches(
            resp.headers.get("Content-Range", ""), offset, expected_total
        ):
            return b""
        return resp.read(MOVIEHASH_CHUNK_BYTES)


def ranged_moviehash(read_at: Callable[[int], bytes], size: int) -> str | None:
    """Computes MovieHash from two ranged 64 KiB reads; returns None on short/truncated reads."""
    if size < MOVIEHASH_CHUNK_BYTES:
        return None
    head = read_at(0)
    tail = read_at(size - MOVIEHASH_CHUNK_BYTES)
    if len(head) < MOVIEHASH_CHUNK_BYTES or len(tail) < MOVIEHASH_CHUNK_BYTES:
        return None
    return moviehash_from_chunks(size, head, tail)


def ftp_moviehash(connect: Callable[[], ftplib.FTP], path: str, size: int = 0) -> str | None:
    """Computes MovieHash of a NAS file over FTP after resolving authoritative binary SIZE and NFD/NFC path."""
    _ = size
    resolved = _ftp_resolve_size_and_path(connect, path)
    if resolved is None:
        return None
    resolved_path, actual_size = resolved
    return ranged_moviehash(lambda offset: _ftp_read_at(connect, resolved_path, offset), actual_size)


def target_moviehash(target: VideoTarget, stream_base: str, child_env: Dict[str, str]) -> str | None:
    """Computes MovieHash of a target over HTTP Range or FTP REST; returns None on network/range error."""
    url = urllib.parse.urlsplit(stream_base)
    if url.scheme in ("http", "https"):
        media_url = build_stream_url(stream_base, target.rel_path)
        try:
            size = target.size_bytes if target.size_bytes > 0 else (_http_file_size(media_url) or 0)
            return ranged_moviehash(lambda offset: _http_read_at(media_url, offset, expected_total=size), size)
        except (OSError, http.client.HTTPException):
            return None
    if url.scheme != "ftp" or not url.hostname:
        return None
    host = url.hostname

    def connect() -> ftplib.FTP:
        ftp = ftplib.FTP(encoding="utf-8")
        try:
            ftp.connect(host, url.port or DEFAULT_FTP_PORT, timeout=FTP_MOVIEHASH_TIMEOUT_SECS)
            ftp.login(child_env.get("DBSRT_FTP_USER", ""), child_env.get("DBSRT_FTP_PASSWORD", ""))
        except BaseException:
            ftp.close()
            raise
        return ftp

    try:
        return ftp_moviehash(connect, target.rel_path, target.size_bytes)
    except ftplib.all_errors:
        return None
