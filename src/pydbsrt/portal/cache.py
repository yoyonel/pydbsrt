"""Two-tier memory and atomic persistent disk cache for pydbsrt Portal."""

from __future__ import annotations

import hashlib
import json
import os
import threading
import time
from pathlib import Path
from typing import Any

from pydbsrt.portal.config import CACHE_KEY_HEX_LEN, PHASHES_CACHE_TTL_SECONDS, REPO_ROOT

_LISTING_CACHE: dict[str, tuple[float, str, str, str, list[dict[str, Any]]]] = {}
_DETAILS_CACHE: dict[str, tuple[float, dict[str, Any]]] = {}
_PHASHES_AUDIT_CACHE: tuple[float, list[dict[str, Any]], dict[str, Any]] | None = None
_CACHE_LOCK = threading.Lock()


def _get_portal_cache_dir() -> Path:
    """Returns persistent disk cache directory across local, o2switch, and RPi4 hosts."""
    env_dir = os.environ.get("DBSRT_PORTAL_CACHE_DIR")
    p = Path(env_dir) if env_dir else Path.home() / ".cache" / "dbsrt" / "portal"
    try:
        p.mkdir(parents=True, exist_ok=True)
        return p
    except OSError:
        fallback = REPO_ROOT / "data" / "cache" / "portal"
        fallback.mkdir(parents=True, exist_ok=True)
        return fallback


def _cache_key_hash(key: str) -> str:
    """Computes stable truncated SHA256 hex digest for cache key."""
    return hashlib.sha256(key.encode("utf-8")).hexdigest()[:CACHE_KEY_HEX_LEN]


def _read_disk_cache(cache_type: str, key: str, ttl_seconds: float) -> tuple[float, Any] | None:
    """Reads unexpired JSON payload from persistent disk cache."""
    cache_dir = _get_portal_cache_dir()
    cache_file = cache_dir / f"{cache_type}_{_cache_key_hash(key)}.json"
    if not cache_file.is_file():
        return None
    try:
        payload = json.loads(cache_file.read_text(encoding="utf-8"))
        ts = float(payload.get("timestamp", 0.0))
        if time.time() - ts < ttl_seconds:
            return ts, payload.get("data")
    except Exception:
        pass
    return None


def _write_disk_cache(cache_type: str, key: str, data: Any) -> None:
    """Atomically writes JSON payload to persistent disk cache."""
    cache_dir = _get_portal_cache_dir()
    key_hash = _cache_key_hash(key)
    cache_file = cache_dir / f"{cache_type}_{key_hash}.json"
    tmp_file = cache_dir / f"{cache_type}_{key_hash}.json.tmp"
    payload = {
        "timestamp": time.time(),
        "key": key,
        "data": data,
    }
    try:
        tmp_file.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
        tmp_file.replace(cache_file)
    except Exception:
        pass


def get_phashes_audit_cache() -> tuple[list[dict[str, Any]], dict[str, Any]] | None:
    """Returns cached tripartite pHash summary if still within TTL."""
    now = time.time()
    with _CACHE_LOCK:
        if _PHASHES_AUDIT_CACHE and (now - _PHASHES_AUDIT_CACHE[0]) < PHASHES_CACHE_TTL_SECONDS:
            return _PHASHES_AUDIT_CACHE[1], _PHASHES_AUDIT_CACHE[2]
    return None


def set_phashes_audit_cache(reconciled: list[dict[str, Any]], summary: dict[str, Any]) -> None:
    """Stores tripartite pHash summary in thread-safe L1 memory cache."""
    global _PHASHES_AUDIT_CACHE
    with _CACHE_LOCK:
        _PHASHES_AUDIT_CACHE = (time.time(), reconciled, summary)
