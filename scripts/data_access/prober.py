"""Bounded parallel channel prober, flock anti-stampede, SSH multiplexing, and SWR cache."""

from __future__ import annotations

import concurrent.futures
import fcntl
import ftplib
import hashlib
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from datetime import datetime
from pathlib import Path
from typing import Any

from scripts.data_access.backup_manifest import inspect_local_backup_health
from scripts.data_access.constants import (
    AGENT_HTTP_USER_AGENT,
    CACHE_DIR_MODE,
    CACHE_DIR_REL,
    CACHE_FILE_MODE,
    CACHE_FILENAME,
    CACHE_LOCK_FILENAME,
    CACHE_MAX_STALE_SECONDS,
    CACHE_SCHEMA_VERSION,
    CACHE_TTL_SECONDS,
    DBAHASH_HEADER_PREFIX_BYTES,
    DBAHASH_HEADER_PREFIX_FORMAT,
    DBAHASH_MAGIC_PREFIX,
    DBPHASH_HEADER_PREFIX_BYTES,
    DBPHASH_HEADER_PREFIX_FORMAT,
    DBPHASH_MAGIC_PREFIX,
    DEFAULT_LOCALHOST_IP,
    FREEBOX_GATEWAY_PORT,
    FTP_DEFAULT_PORT,
    FTP_PROBE_TIMEOUT_SECS,
    HTTP_DEFAULT_PORT,
    HTTP_GATEWAY_TIMEOUT_SECS,
    HTTP_READ_MAX_BYTES,
    HTTP_STATUS_OK,
    HTTPS_API_TIMEOUT_SECS,
    MAX_PROBE_WORKERS,
    MILLIS_PER_SEC,
    PG_LOCAL_PORT,
    PG_RPI4_PORT,
    SSH_ALIAS_O2SWITCH,
    SSH_ALIAS_RPI4,
    SSH_CONNECT_TIMEOUT_SECS,
    SSH_CONTROL_PERSIST_SECS,
    SSH_PROBE_TIMEOUT_SECS,
    STALE_MTIME_TOLERANCE_SECONDS,
    TCP_SOCKET_TIMEOUT_SECS,
)
from scripts.data_access.topology import (
    RuntimeEndpoints,
    WorkspaceContext,
    inspect_network_topology,
    lookup_pass_secret,
    resolve_o2switch_api_key,
    resolve_runtime_endpoints,
    run_bounded_cmd,
)

PROC_NET_ROUTE_PATH: Path = Path("/proc/net/route")

REMOTE_SNAPSHOT_POSIX_SH: str = (
    'd="$HOME/dbsrt/data"; '
    "cs=$(stat -c '%s:%Y' \"$d/catalog.dbphash\" 2>/dev/null || echo '0:0'); "
    "cm=$(od -An -j 12 -N 4 -t u4 \"$d/catalog.dbphash\" 2>/dev/null | tr -d ' ' || echo '0'); "
    "acs=$(stat -c '%s:%Y' \"$d/catalog.dbahash\" 2>/dev/null || echo '0:0'); "
    "acm=$(od -An -j 16 -N 4 -t u4 \"$d/catalog.dbahash\" 2>/dev/null | tr -d ' ' || echo '0'); "
    "pc=$(find \"$d/__PHASH__\" -maxdepth 1 -name '*.phash' 2>/dev/null | wc -l | tr -d ' '); "
    "pj=$(find \"$d/__PHASH__\" -maxdepth 1 -name '*.json' 2>/dev/null | wc -l | tr -d ' '); "
    "ac=$(find \"$d/__AHASH__\" -maxdepth 1 -name '*.ahash' 2>/dev/null | wc -l | tr -d ' '); "
    "aj=$(find \"$d/__AHASH__\" -maxdepth 1 -name '*.json' 2>/dev/null | wc -l | tr -d ' '); "
    "ks=$(stat -c '%s:%Y' \"$d/SRT/couplings.json\" 2>/dev/null || echo '0:0'); "
    'printf "cat=%s:%s\\nacat=%s:%s\\nphash=%s:%s\\nahash=%s:%s\\ncouplings=%s\\n" '
    '"$cs" "$cm" "$acs" "$acm" "$pc" "$pj" "$ac" "$aj" "$ks"'
)


def compute_kernel_route_digest(route_path: Path = PROC_NET_ROUTE_PATH) -> str:
    """Computes a sub-millisecond SHA-256 digest of /proc/net/route for cache invalidation."""
    forced_live = os.environ.get("DBSRT_FORCE_LIVE_PROFILE", "").strip()
    if not route_path.is_file():
        return f"no-proc-route:{forced_live}" if forced_live else "no-proc-route"
    raw = route_path.read_bytes() + (f"|{forced_live}".encode() if forced_live else b"")
    return hashlib.sha256(raw).hexdigest()[:16]


def get_cache_file_path() -> Path:
    """Returns the user-scoped cache path under ~/.cache/dbsrt/."""
    override = os.environ.get("DBSRT_DATA_ACCESS_CACHE")
    if override:
        return Path(override)
    return Path.home().joinpath(*CACHE_DIR_REL, CACHE_FILENAME)


def _ensure_cache_dir(cache_path: Path) -> Path:
    """Ensures the private 0700 cache directory exists."""
    cache_path.parent.mkdir(parents=True, exist_ok=True, mode=CACHE_DIR_MODE)
    return cache_path.parent


def _format_epoch_iso(epoch: float | int) -> str | None:
    """Formats a UNIX timestamp into an ISO-8601 string with local timezone offset."""
    if not epoch or epoch <= 0:
        return None
    return datetime.fromtimestamp(float(epoch)).astimezone().isoformat(timespec="seconds")


def probe_tcp_port(host: str, port: int, label: str) -> dict[str, Any]:
    """Probes raw TCP reachability with a strict connect timeout."""
    t0 = time.perf_counter()
    try:
        with socket.create_connection((host, port), timeout=TCP_SOCKET_TIMEOUT_SECS):
            dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
            return {"label": label, "accessible": True, "latency_ms": dt_ms}
    except OSError as exc:
        dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
        return {"label": label, "accessible": False, "latency_ms": dt_ms, "error": exc.__class__.__name__}


def probe_ftp_channel(label: str, host: str, user: str, passwd: str, remote_dir: str) -> dict[str, Any]:
    """Verifies FTP control login and passive NLST directory listing."""
    t0 = time.perf_counter()
    if not host or "<" in host or not passwd:
        return {"label": label, "accessible": False, "latency_ms": 0.0, "error": "missing_credentials"}
    try:
        ftp = ftplib.FTP()
        ftp.connect(host, FTP_DEFAULT_PORT, timeout=FTP_PROBE_TIMEOUT_SECS)
        ftp.login(user, passwd)
        entries = ftp.nlst(remote_dir)
        ftp.quit()
        dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
        return {"label": label, "accessible": True, "latency_ms": dt_ms, "items_count": len(entries)}
    except Exception as exc:  # noqa: BLE001
        dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
        return {"label": label, "accessible": False, "latency_ms": dt_ms, "error": exc.__class__.__name__}


def probe_http_json_gateway(label: str, url: str) -> dict[str, Any]:
    """Verifies Freebox HTTP gateway strictly requiring application/json and mounted_shares."""
    t0 = time.perf_counter()
    if "<" in url:
        return {"label": label, "accessible": False, "latency_ms": 0.0, "error": "unconfigured_host"}
    try:
        req = urllib.request.Request(url, headers={"User-Agent": AGENT_HTTP_USER_AGENT})
        with urllib.request.urlopen(req, timeout=HTTP_GATEWAY_TIMEOUT_SECS) as resp:
            ctype = resp.headers.get_content_type()
            raw = resp.read(HTTP_READ_MAX_BYTES).decode("utf-8", errors="replace")
            dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
            if ctype != "application/json":
                return {
                    "label": label,
                    "accessible": False,
                    "latency_ms": dt_ms,
                    "error": f"non_json_response:{ctype}",
                }
            payload = json.loads(raw)
            shares = payload.get("mounted_shares", [])
            ok = payload.get("status") == "ok" or (isinstance(shares, list) and bool(shares))
            return {"label": label, "accessible": ok, "latency_ms": dt_ms, "mounted_shares": shares}
    except Exception as exc:  # noqa: BLE001
        dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
        return {"label": label, "accessible": False, "latency_ms": dt_ms, "error": exc.__class__.__name__}


def probe_ssh_snapshot(alias: str) -> tuple[dict[str, Any], dict[str, Any] | None]:
    """Probes SSH with ControlMaster multiplexing (~20ms reuse) and collects remote snapshot."""
    t0 = time.perf_counter()
    ctrl_dir = _ensure_cache_dir(get_cache_file_path())
    ctrl_path = str(ctrl_dir / "ssh-%C")
    cmd = [
        "ssh",
        "-o",
        "BatchMode=yes",
        "-o",
        f"ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}",
        "-o",
        "ControlMaster=auto",
        "-o",
        f"ControlPath={ctrl_path}",
        "-o",
        f"ControlPersist={SSH_CONTROL_PERSIST_SECS}",
        alias,
        REMOTE_SNAPSHOT_POSIX_SH,
    ]
    rc, out, err = run_bounded_cmd(cmd, SSH_PROBE_TIMEOUT_SECS)
    dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
    if rc != 0:
        reason = "ssh_timeout_port_22_filtered" if rc == 124 or "timed out" in err.lower() else (err or f"rc={rc}")
        return {"label": f"SSH {alias}", "accessible": False, "latency_ms": dt_ms, "error": reason}, None
    return {"label": f"SSH {alias}", "accessible": True, "latency_ms": dt_ms}, _parse_remote_snapshot(alias, out)


def _parse_pair(raw: str) -> tuple[int, int]:
    """Parses 'a:b' integer pairs safely."""
    parts = raw.split(":")
    first = int(parts[0]) if parts and parts[0].isdigit() else 0
    second = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0
    return first, second


def _parse_triple(raw: str) -> tuple[int, int, int]:
    """Parses 'size:mtime:count' integer triples safely."""
    parts = raw.split(":")
    first = int(parts[0]) if parts and parts[0].isdigit() else 0
    second = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0
    third = int(parts[2]) if len(parts) > 2 and parts[2].isdigit() else 0
    return first, second, third


def _parse_remote_snapshot(node: str, stdout: str) -> dict[str, Any]:
    """Parses the key=value output of REMOTE_SNAPSHOT_POSIX_SH."""
    kv: dict[str, str] = {}
    for line in stdout.splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            kv[k.strip()] = v.strip()

    cat_size, cat_mtime, cat_medias = _parse_triple(kv.get("cat", "0:0:0"))
    acat_size, acat_mtime, acat_medias = _parse_triple(kv.get("acat", "0:0:0"))

    phash_cnt, phash_json = _parse_pair(kv.get("phash", "0:0"))
    ahash_cnt, ahash_json = _parse_pair(kv.get("ahash", "0:0"))
    coup_size, coup_mtime = _parse_pair(kv.get("couplings", "0:0"))

    return {
        "node": node,
        "transport": "ssh",
        "catalog_dbphash": {
            "exists": cat_size > 0,
            "path": f"{node}:~/dbsrt/data/catalog.dbphash",
            "size_bytes": cat_size,
            "mtime_epoch": cat_mtime,
            "mtime_iso": _format_epoch_iso(cat_mtime),
            "media_count": cat_medias,
        },
        "catalog_dbahash": {
            "exists": acat_size > 0,
            "path": f"{node}:~/dbsrt/data/catalog.dbahash",
            "size_bytes": acat_size,
            "mtime_epoch": acat_mtime,
            "mtime_iso": _format_epoch_iso(acat_mtime),
            "media_count": acat_medias,
        },
        "phash_files": {"count": phash_cnt, "sidecar_json_count": phash_json, "path": f"{node}:~/dbsrt/data/__PHASH__"},
        "ahash_files": {"count": ahash_cnt, "sidecar_json_count": ahash_json, "path": f"{node}:~/dbsrt/data/__AHASH__"},
        "couplings_json": {
            "exists": coup_size > 0,
            "path": f"{node}:~/dbsrt/data/SRT/couplings.json",
            "size_bytes": coup_size,
            "mtime_epoch": coup_mtime,
            "mtime_iso": _format_epoch_iso(coup_mtime),
        },
    }


def probe_o2switch_https(base_url: str, api_key: str) -> dict[str, Any]:
    """Probes authenticated HTTPS :443 API on o2switch without triggering Tiger Protect WAF."""
    t0 = time.perf_counter()
    if not base_url or not api_key:
        return {"label": "HTTPS o2switch API", "accessible": False, "latency_ms": 0.0, "error": "missing_api_key"}
    req = urllib.request.Request(
        f"{base_url}/v1/health",
        headers={"X-API-Key": api_key, "Accept": "application/json", "User-Agent": AGENT_HTTP_USER_AGENT},
    )
    try:
        with urllib.request.urlopen(req, timeout=HTTPS_API_TIMEOUT_SECS) as resp:
            ctype = resp.headers.get_content_type()
            body = resp.read(HTTP_READ_MAX_BYTES).decode("utf-8", errors="replace")
            dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
            ok = resp.status == HTTP_STATUS_OK and ctype == "application/json" and '"ok"' in body
            return {"label": "HTTPS o2switch API", "accessible": ok, "latency_ms": dt_ms, "http_status": resp.status}
    except urllib.error.HTTPError as exc:
        dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
        return {"label": "HTTPS o2switch API", "accessible": False, "latency_ms": dt_ms, "error": f"HTTP_{exc.code}"}
    except Exception as exc:  # noqa: BLE001
        dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 1)
        return {
            "label": "HTTPS o2switch API",
            "accessible": False,
            "latency_ms": dt_ms,
            "error": exc.__class__.__name__,
        }


def _fetch_o2switch_https_snapshot(base_url: str, api_key: str) -> dict[str, Any] | None:
    """Fetches catalog and pHash counts over HTTPS :443 when SSH :22 is filtered (4G/5G CGNAT)."""
    if not base_url or not api_key:
        return None
    req = urllib.request.Request(
        f"{base_url}/api/phashes?limit=1",
        headers={"X-API-Key": api_key, "Accept": "application/json", "User-Agent": AGENT_HTTP_USER_AGENT},
    )
    try:
        with urllib.request.urlopen(req, timeout=HTTPS_API_TIMEOUT_SECS) as resp:
            if resp.status != HTTP_STATUS_OK or resp.headers.get_content_type() != "application/json":
                return None
            payload = json.loads(resp.read(HTTP_READ_MAX_BYTES * 4).decode("utf-8", errors="replace"))
            summary = payload.get("summary") if isinstance(payload, dict) else None
            if not isinstance(summary, dict):
                return None
            cat_size = int(summary.get("catalog_size_bytes") or 0)
            cat_cnt = int(summary.get("catalog_count") or 0)
            raw_cnt = int(summary.get("raw_count") or 0)
            return {
                "node": SSH_ALIAS_O2SWITCH,
                "transport": "https_api",
                "catalog_dbphash": {
                    "exists": cat_size > 0,
                    "path": f"{SSH_ALIAS_O2SWITCH}:~/dbsrt/data/catalog.dbphash",
                    "size_bytes": cat_size,
                    "mtime_epoch": 0,
                    "mtime_iso": None,
                    "media_count": cat_cnt,
                },
                "phash_files": {
                    "count": raw_cnt,
                    "sidecar_json_count": 0,
                    "path": f"{SSH_ALIAS_O2SWITCH}:~/dbsrt/data/__PHASH__",
                },
            }
    except Exception:  # noqa: BLE001
        return None


def probe_local_cifs_safe(net: dict[str, Any], endpoints: RuntimeEndpoints) -> dict[str, Any]:
    """Checks local CIFS mounts ONLY when kernel routing proves direct Home LAN safety."""
    if not net.get("cifs_safe_to_stat", False):
        return {
            "label": "Local CIFS Mounts",
            "accessible": False,
            "latency_ms": 0.0,
            "mounted_dirs": [],
            "error": "blocked_by_stale_cifs_hang_guard",
        }
    t0 = time.perf_counter()
    root = Path(endpoints.local_media_root)
    candidates = (
        root / "Synology" / "video",
        root / "Synology" / "tvshow",
        root / "Freebox" / "Disque_1",
        root / "Freebox" / "Disque_2",
        root / "Freebox" / "Disque_1_1",
    )
    accessible_dirs: list[str] = []
    for folder in candidates:
        try:
            if folder.is_dir() and any(os.scandir(folder)):
                accessible_dirs.append(str(folder))
        except OSError:
            continue
    dt_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 2)
    return {
        "label": "Local CIFS Mounts",
        "accessible": bool(accessible_dirs),
        "latency_ms": dt_ms,
        "mounted_dirs": accessible_dirs,
    }


def _read_dbphash_media_count(cat_path: Path) -> int:
    """Reads the 32-bit media_count field from the 16-byte DBSRT v2 header in O(1)."""
    try:
        with open(cat_path, "rb") as handle:
            raw = handle.read(DBPHASH_HEADER_PREFIX_BYTES)
        if len(raw) < DBPHASH_HEADER_PREFIX_BYTES:
            return 0
        magic, _ver, _flags, media_cnt = struct.unpack(DBPHASH_HEADER_PREFIX_FORMAT, raw)
        return int(media_cnt) if magic.startswith(DBPHASH_MAGIC_PREFIX) else 0
    except OSError:
        return 0


def _read_dbahash_media_count(cat_path: Path) -> int:
    """Reads the 32-bit media_count field from the 20-byte DBAHASH v1 header in O(1) (zero cold page faults)."""
    try:
        with open(cat_path, "rb") as handle:
            raw = handle.read(DBAHASH_HEADER_PREFIX_BYTES)
        if len(raw) < DBAHASH_HEADER_PREFIX_BYTES:
            return 0
        magic, _ver, _flags, _factor_k, media_cnt = struct.unpack(DBAHASH_HEADER_PREFIX_FORMAT, raw)
        return int(media_cnt) if magic == DBAHASH_MAGIC_PREFIX else 0
    except OSError:
        return 0


def _count_ext_pair(folder: Path, primary_ext: str) -> tuple[int, int]:
    """Counts primary files and .json sidecars in a local directory via single scandir pass."""
    if not folder.is_dir():
        return 0, 0
    prim = 0
    side = 0
    try:
        for entry in os.scandir(folder):
            if entry.name.endswith(primary_ext):
                prim += 1
            elif entry.name.endswith(".json"):
                side += 1
    except OSError:
        pass
    return prim, side


def inspect_worktree_direct_counts(active_repo: Path) -> dict[str, Any]:
    """Inspects direct dataset presence in active_repo/data so agents never need `ls data/`."""
    wt_data = active_repo / "data"
    ahash_cnt, _ = _count_ext_pair(wt_data / "__AHASH__", ".ahash")
    phash_cnt, _ = _count_ext_pair(wt_data / "__PHASH__", ".phash")
    snip_cnt, _ = _count_ext_pair(wt_data / "snippets", ".mp4")
    return {
        "data_dir": str(wt_data),
        "catalog_dbphash_present": (wt_data / "catalog.dbphash").exists(),
        "catalog_dbahash_present": (wt_data / "catalog.dbahash").exists(),
        "couplings_json_present": (wt_data / "SRT" / "couplings.json").exists(),
        "ahash_count": ahash_cnt,
        "phash_count": phash_cnt,
        "snippets_mp4_count": snip_cnt,
    }


def inspect_local_dataset_snapshot(ws: WorkspaceContext) -> dict[str, Any]:
    """Inspects local ext4/NVMe dataset artifacts (zero CIFS access)."""
    data_dir = ws.effective_data_dir
    cat_path = data_dir / "catalog.dbphash"
    cat_stat = cat_path.stat() if cat_path.is_file() else None

    acat_path = data_dir / "catalog.dbahash"
    acat_stat = acat_path.stat() if acat_path.is_file() else None

    coup_path = data_dir / "SRT" / "couplings.json"
    coup_stat = coup_path.stat() if coup_path.is_file() else None

    phash_cnt, phash_json = _count_ext_pair(data_dir / "__PHASH__", ".phash")
    ahash_cnt, ahash_json = _count_ext_pair(data_dir / "__AHASH__", ".ahash")
    snip_cnt, _ = _count_ext_pair(data_dir / "snippets", ".mp4")
    snip_ahash_cnt, _ = _count_ext_pair(data_dir / "snippets_ahash", ".ahash")

    return {
        "node": "local",
        "data_dir": str(data_dir),
        "from_parent_worktree": ws.is_worktree and data_dir == (ws.common_repo / "data"),
        "catalog_dbphash": {
            "exists": cat_stat is not None,
            "path": str(cat_path),
            "size_bytes": cat_stat.st_size if cat_stat else 0,
            "mtime_epoch": int(cat_stat.st_mtime) if cat_stat else 0,
            "mtime_iso": _format_epoch_iso(cat_stat.st_mtime) if cat_stat else None,
            "media_count": _read_dbphash_media_count(cat_path) if cat_stat else 0,
        },
        "catalog_dbahash": {
            "exists": acat_stat is not None,
            "path": str(acat_path),
            "size_bytes": acat_stat.st_size if acat_stat else 0,
            "mtime_epoch": int(acat_stat.st_mtime) if acat_stat else 0,
            "mtime_iso": _format_epoch_iso(acat_stat.st_mtime) if acat_stat else None,
            "media_count": _read_dbahash_media_count(acat_path) if acat_stat else 0,
        },
        "phash_files": {"count": phash_cnt, "sidecar_json_count": phash_json, "path": str(data_dir / "__PHASH__")},
        "ahash_files": {"count": ahash_cnt, "sidecar_json_count": ahash_json, "path": str(data_dir / "__AHASH__")},
        "couplings_json": {
            "exists": coup_stat is not None,
            "path": str(coup_path),
            "size_bytes": coup_stat.st_size if coup_stat else 0,
            "mtime_epoch": int(coup_stat.st_mtime) if coup_stat else 0,
            "mtime_iso": _format_epoch_iso(coup_stat.st_mtime) if coup_stat else None,
        },
        "snippets_mp4": {"count": snip_cnt, "path": str(data_dir / "snippets")},
        "snippets_ahash": {"count": snip_ahash_cnt, "path": str(data_dir / "snippets_ahash")},
    }


def _build_catalog_sync_command(
    channels: dict[str, Any],
    nodes: dict[str, dict[str, Any]],
    local_path: str,
) -> str:
    """Synthesizes the recommended catalog synchronization command for the active network."""
    if channels.get("ssh_rpi4", {}).get("accessible") and "rpi4" in nodes:
        return f"rsync -ah --info=progress2 rpi4:~/dbsrt/data/catalog.dbphash {local_path}"
    if channels.get("ssh_o2switch", {}).get("accessible"):
        return "task o2switch:pull-catalog"
    return "SSH :22 filtered on current network; use HTTPS recognize API or switch to whitelisted IP"


def _evaluate_catalog_freshness(nodes: dict[str, dict[str, Any]], channels: dict[str, Any]) -> dict[str, Any]:
    """Compares local catalog.dbphash against remote nodes and synthesizes sync guidance."""
    local_cat = nodes["local"]["catalog_dbphash"]
    candidates = [
        (name, snap["catalog_dbphash"])
        for name, snap in nodes.items()
        if name != "local" and snap and snap["catalog_dbphash"]["exists"]
    ]
    if not candidates:
        status = "LOCAL_ONLY_UNVERIFIED" if local_cat["exists"] else "MISSING_LOCAL"
        return {"status": status, "authoritative_node": "local", "delta_bytes": 0, "delta_medias": 0, "sync_cmd": None}

    best_node, best_cat = max(
        candidates,
        key=lambda item: (item[1]["media_count"], item[1]["size_bytes"], item[1]["mtime_epoch"]),
    )
    is_stale = (
        best_cat["media_count"] > local_cat["media_count"]
        or best_cat["size_bytes"] > local_cat["size_bytes"]
        or (
            best_cat["mtime_epoch"] > 0
            and best_cat["mtime_epoch"] > local_cat["mtime_epoch"] + STALE_MTIME_TOLERANCE_SECONDS
        )
    )
    status = "MISSING_LOCAL" if not local_cat["exists"] else ("STALE_LOCAL" if is_stale else "UP_TO_DATE")
    sync_cmd = _build_catalog_sync_command(channels, nodes, local_cat["path"]) if status != "UP_TO_DATE" else None
    delta_secs = (best_cat["mtime_epoch"] - local_cat["mtime_epoch"]) if best_cat["mtime_epoch"] > 0 else 0
    return {
        "status": status,
        "authoritative_node": best_node,
        "delta_bytes": best_cat["size_bytes"] - local_cat["size_bytes"],
        "delta_medias": best_cat["media_count"] - local_cat["media_count"],
        "delta_seconds": delta_secs,
        "sync_cmd": sync_cmd,
    }


def _build_ahash_sync_command(channels: dict[str, Any], local_path: str) -> str:
    """Synthesizes the recommended catalog.dbahash synchronization command for the active network."""
    if channels.get("ssh_o2switch", {}).get("accessible"):
        return f"rsync -az --partial o2switch:~/dbsrt/data/catalog.dbahash {local_path}"
    return "SSH :22 filtered on current network; use HTTPS /v1/search/ahash API or whitelisted IP"


def _evaluate_ahash_catalog_freshness(nodes: dict[str, dict[str, Any]], channels: dict[str, Any]) -> dict[str, Any]:
    """Compares local catalog.dbahash against remote nodes (o2switch) and synthesizes sync guidance."""
    empty_cat = {"exists": False, "size_bytes": 0, "mtime_epoch": 0, "media_count": 0, "path": ""}
    local_cat = nodes["local"].get("catalog_dbahash", empty_cat)
    candidates = [
        (name, snap["catalog_dbahash"])
        for name, snap in nodes.items()
        if name != "local" and snap and snap.get("catalog_dbahash", {}).get("exists")
    ]
    if not candidates:
        status = "LOCAL_ONLY_UNVERIFIED" if local_cat["exists"] else "MISSING_LOCAL"
        return {"status": status, "authoritative_node": "local", "delta_bytes": 0, "delta_medias": 0, "sync_cmd": None}

    best_node, best_cat = max(
        candidates,
        key=lambda item: (item[1]["media_count"], item[1]["size_bytes"], item[1]["mtime_epoch"]),
    )
    mtime_stale = (
        best_cat["mtime_epoch"] > 0
        and best_cat["mtime_epoch"] > local_cat["mtime_epoch"] + STALE_MTIME_TOLERANCE_SECONDS
    )
    is_stale = (
        best_cat["media_count"] > local_cat["media_count"]
        or best_cat["size_bytes"] > local_cat["size_bytes"]
        or mtime_stale
    )
    status = "MISSING_LOCAL" if not local_cat["exists"] else ("STALE_LOCAL" if is_stale else "UP_TO_DATE")
    sync_cmd = _build_ahash_sync_command(channels, local_cat["path"]) if status != "UP_TO_DATE" else None
    delta_secs = (best_cat["mtime_epoch"] - local_cat["mtime_epoch"]) if best_cat["mtime_epoch"] > 0 else 0
    return {
        "status": status,
        "authoritative_node": best_node,
        "delta_bytes": best_cat["size_bytes"] - local_cat["size_bytes"],
        "delta_medias": best_cat["media_count"] - local_cat["media_count"],
        "delta_seconds": delta_secs,
        "sync_cmd": sync_cmd,
    }


def _evaluate_corpus_counts(nodes: dict[str, dict[str, Any]], field_key: str) -> dict[str, Any]:
    """Identifies the most complete node for a file corpus (.phash or .ahash)."""
    per_node = {name: snap[field_key]["count"] for name, snap in nodes.items() if snap and field_key in snap}
    best_node = max(per_node, key=lambda k: per_node[k]) if per_node else "local"
    local_cnt = per_node.get("local", 0)
    best_cnt = per_node.get(best_node, 0)
    status = "UP_TO_DATE" if local_cnt >= best_cnt and best_cnt > 0 else "PARTIAL_LOCAL"
    return {
        "status": status,
        "counts_by_node": per_node,
        "most_complete_node": best_node,
        "most_complete_count": best_cnt,
        "missing_locally": max(0, best_cnt - local_cnt),
    }


def _probe_ftp_with_lazy_pass(
    ws: WorkspaceContext,
    endpoints: RuntimeEndpoints,
    use_lan: bool,
) -> dict[str, Any]:
    """Resolves NAS password inside worker thread and probes FTP."""
    nas_pass = lookup_pass_secret(ws.env_vars, "NAS_PASS", endpoints.nas_pass_entry)
    host = endpoints.nas_lan_host if use_lan else endpoints.nas_wan_host
    label = "LAN FTP Synology" if use_lan else "WAN FTP Synology"
    return probe_ftp_channel(label, host, endpoints.nas_user, nas_pass, "/video")


def _probe_https_with_lazy_key(ws: WorkspaceContext, endpoints: RuntimeEndpoints) -> dict[str, Any]:
    """Resolves API key inside worker thread and probes o2switch HTTPS."""
    api_key = resolve_o2switch_api_key(ws)
    return probe_o2switch_https(endpoints.o2switch_url, api_key)


def _record_o2switch_probe_results(
    ws: WorkspaceContext,
    endpoints: RuntimeEndpoints,
    f_ssh_o2: concurrent.futures.Future[Any] | None,
    state: tuple[dict[str, Any], dict[str, dict[str, Any]]],
) -> None:
    """Populates o2switch SSH/HTTPS channel results and remote snapshot fallback."""
    channels, remote_nodes = state
    if f_ssh_o2 is not None:
        ssh_o2_res, snap_o2 = f_ssh_o2.result()
        channels["ssh_o2switch"] = ssh_o2_res
        if snap_o2:
            remote_nodes["o2switch"] = snap_o2
    else:
        channels["ssh_o2switch"] = {
            "label": f"SSH {SSH_ALIAS_O2SWITCH}",
            "accessible": False,
            "latency_ms": 0.0,
            "error": "ssh_timeout_port_22_filtered",
        }
    if SSH_ALIAS_O2SWITCH not in remote_nodes and channels["https_o2switch"].get("accessible"):
        https_snap = _fetch_o2switch_https_snapshot(endpoints.o2switch_url, resolve_o2switch_api_key(ws))
        if https_snap is not None:
            remote_nodes[SSH_ALIAS_O2SWITCH] = https_snap


def _record_rpi4_probe_results(
    f_ssh_rpi4: concurrent.futures.Future[Any] | None,
    skipped_lan: dict[str, Any],
    state: tuple[dict[str, Any], dict[str, dict[str, Any]]],
) -> None:
    """Populates rpi4 SSH channel result and remote snapshot when Home LAN is active."""
    channels, remote_nodes = state
    if f_ssh_rpi4 is None:
        channels["ssh_rpi4"] = dict(skipped_lan, label="SSH rpi4")
        return
    ssh_rpi_res, snap_rpi = f_ssh_rpi4.result()
    channels["ssh_rpi4"] = ssh_rpi_res
    if snap_rpi:
        remote_nodes["rpi4"] = snap_rpi


def _run_all_probes(
    ws: WorkspaceContext,
    net: dict[str, Any],
    endpoints: RuntimeEndpoints,
) -> tuple[dict[str, Any], dict[str, dict[str, Any]]]:
    """Executes bounded network probes + GPG lookups concurrently."""
    lan_ok = bool(net.get("cifs_safe_to_stat", False))
    channels: dict[str, Any] = {
        "local_fs": {"label": "Local Workstation FS", "accessible": True, "latency_ms": 0.1},
        "local_cifs": probe_local_cifs_safe(net, endpoints),
    }
    remote_nodes: dict[str, dict[str, Any]] = {"local": inspect_local_dataset_snapshot(ws)}

    if net.get("simulated"):
        return _populate_simulated_channels(net["profile"], channels), remote_nodes

    block_ssh_22 = net.get("forced_live_profile") == "nomad_wan"
    with concurrent.futures.ThreadPoolExecutor(max_workers=MAX_PROBE_WORKERS) as pool:
        f_https_o2 = pool.submit(_probe_https_with_lazy_key, ws, endpoints)
        f_ssh_o2 = None if block_ssh_22 else pool.submit(probe_ssh_snapshot, SSH_ALIAS_O2SWITCH)
        f_pg_local = pool.submit(probe_tcp_port, DEFAULT_LOCALHOST_IP, PG_LOCAL_PORT, "PostgreSQL Local (:5433)")
        f_wan_ftp = None if lan_ok else pool.submit(_probe_ftp_with_lazy_pass, ws, endpoints, False)
        f_lan_ftp = pool.submit(_probe_ftp_with_lazy_pass, ws, endpoints, True) if lan_ok else None
        f_lan_http = (
            pool.submit(
                probe_http_json_gateway,
                "LAN HTTP Freebox Gateway (:18089)",
                f"http://{endpoints.rpi4_lan_host}:{FREEBOX_GATEWAY_PORT}/health",
            )
            if lan_ok
            else None
        )
        f_ssh_rpi4 = pool.submit(probe_ssh_snapshot, SSH_ALIAS_RPI4) if lan_ok else None
        f_pg_rpi4 = (
            pool.submit(probe_tcp_port, endpoints.rpi4_lan_host, PG_RPI4_PORT, "PostgreSQL RPi4 (:5432)")
            if lan_ok
            else None
        )

        channels["https_o2switch"] = f_https_o2.result()
        channels["pg_local"] = f_pg_local.result()
        channels["wan_http_freebox_direct"] = {
            "label": f"Direct WAN HTTP Freebox (:{HTTP_DEFAULT_PORT})",
            "accessible": False,
            "latency_ms": 0.0,
            "error": "ip_restricted_to_o2switch_egress",
        }
        _record_o2switch_probe_results(ws, endpoints, f_ssh_o2, (channels, remote_nodes))

        skipped_lan = {"accessible": False, "latency_ms": 0.0, "error": "skipped_no_direct_home_lan_route"}
        channels["lan_ftp_synology"] = f_lan_ftp.result() if f_lan_ftp else dict(skipped_lan, label="LAN FTP Synology")
        channels["wan_ftp_synology"] = (
            f_wan_ftp.result()
            if f_wan_ftp
            else {"label": "WAN FTP Synology", "accessible": True, "latency_ms": 0.0, "note": "deferred_on_home_lan"}
        )
        channels["lan_http_freebox"] = (
            f_lan_http.result() if f_lan_http else dict(skipped_lan, label="LAN HTTP Freebox Gateway (:18089)")
        )
        channels["pg_rpi4"] = f_pg_rpi4.result() if f_pg_rpi4 else dict(skipped_lan, label="PostgreSQL RPi4 (:5432)")
        _record_rpi4_probe_results(f_ssh_rpi4, skipped_lan, (channels, remote_nodes))

    return channels, remote_nodes


def _populate_simulated_channels(profile: str, channels: dict[str, Any]) -> dict[str, Any]:
    """Populates deterministic hermetic channel states when --simulate-profile is requested."""
    is_home = profile == "home_lan"
    is_work = profile == "work_vpn"
    channels["lan_ftp_synology"] = {"label": "LAN FTP Synology", "accessible": is_home, "latency_ms": 8.0}
    channels["wan_ftp_synology"] = {"label": "WAN FTP Synology", "accessible": True, "latency_ms": 420.0}
    channels["lan_http_freebox"] = {"label": "LAN HTTP Freebox", "accessible": is_home, "latency_ms": 12.0}
    channels["wan_http_freebox_direct"] = {
        "label": "Direct WAN HTTP Freebox (:80)",
        "accessible": profile == "o2switch",
        "latency_ms": 35.0,
    }
    channels["ssh_rpi4"] = {"label": "SSH rpi4", "accessible": is_home, "latency_ms": 15.0}
    channels["ssh_o2switch"] = {"label": "SSH o2switch", "accessible": is_home or is_work, "latency_ms": 140.0}
    channels["https_o2switch"] = {"label": "HTTPS o2switch API", "accessible": True, "latency_ms": 135.0}
    channels["pg_local"] = {"label": "PostgreSQL Local (:5433)", "accessible": False, "latency_ms": 0.5}
    channels["pg_rpi4"] = {"label": "PostgreSQL RPi4 (:5432)", "accessible": is_home, "latency_ms": 4.0}
    return channels


def _try_load_valid_cache(
    cache_path: Path,
    route_digest: str,
    active_repo_str: str,
    max_age_secs: float,
) -> dict[str, Any] | None:
    """Loads cached status in <2ms if /proc/net/route digest and max_age_secs match."""
    if not cache_path.is_file():
        return None
    try:
        payload = json.loads(cache_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if payload.get("schema_version") != CACHE_SCHEMA_VERSION:
        return None
    if payload.get("route_digest") != route_digest:
        return None
    if payload.get("workspace", {}).get("active_repo") != active_repo_str:
        return None
    age = time.time() - float(payload.get("created_at_epoch", 0.0))
    if age < 0 or age > max_age_secs:
        return None
    payload["cache"] = {
        "hit": True,
        "age_seconds": round(age, 2),
        "ttl_seconds": CACHE_TTL_SECONDS,
        "stale_revalidating": age > CACHE_TTL_SECONDS,
        "path": str(cache_path),
    }
    return payload


def _spawn_background_refresh(active_repo: Path) -> None:
    """Spawns a detached background refresh when returning a stale-while-revalidate cache hit."""
    cli_script = Path(__file__).resolve().parent / "cli.py"
    try:
        subprocess.Popen(
            [sys.executable, str(cli_script), "status", "--force-refresh", "--repo", str(active_repo)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError:
        pass


def _atomic_write_cache(cache_path: Path, payload: dict[str, Any]) -> None:
    """Writes cache JSON atomically with 0600 permissions."""
    try:
        cache_dir = _ensure_cache_dir(cache_path)
        fd, tmp_name = tempfile.mkstemp(dir=str(cache_dir), prefix=".dbsrt_cache_", suffix=".tmp")
        try:
            os.chmod(fd, CACHE_FILE_MODE)
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                json.dump(payload, handle, ensure_ascii=False, indent=2)
            os.replace(tmp_name, cache_path)
        except OSError:
            Path(tmp_name).unlink(missing_ok=True)
    except OSError:
        pass


def _build_fresh_status_payload(
    ws: WorkspaceContext,
    route_digest: str,
    simulated_profile: str | None,
    ttl_secs: float,
    t0: float,
) -> dict[str, Any]:
    """Executes fresh probes and assembles the canonical status dictionary."""
    net = inspect_network_topology(simulated_profile=simulated_profile)
    endpoints = resolve_runtime_endpoints(ws)
    channels, nodes = _run_all_probes(ws, net, endpoints)

    freshness = {
        "packed_catalog": _evaluate_catalog_freshness(nodes, channels),
        "packed_ahash_catalog": _evaluate_ahash_catalog_freshness(nodes, channels),
        "phash_corpus": _evaluate_corpus_counts(nodes, "phash_files"),
        "ahash_corpus": _evaluate_corpus_counts(nodes, "ahash_files"),
        "nodes": nodes,
    }
    seed_cmd = "task data:seed-worktree" if ws.unseeded_items else None
    total_ms = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 2)

    return {
        "schema_version": CACHE_SCHEMA_VERSION,
        "created_at_epoch": time.time(),
        "created_at_iso": _format_epoch_iso(time.time()),
        "route_digest": route_digest,
        "cache": {
            "hit": False,
            "age_seconds": 0.0,
            "ttl_seconds": ttl_secs,
            "stale_revalidating": False,
            "path": str(get_cache_file_path()),
            "lookup_ms": total_ms,
        },
        "workspace": {
            "active_repo": str(ws.active_repo),
            "common_repo": str(ws.common_repo),
            "is_worktree": ws.is_worktree,
            "active_worktree_direct": inspect_worktree_direct_counts(ws.active_repo) if ws.is_worktree else None,
            "effective_data_dir": str(ws.effective_data_dir),
            "unseeded_items": list(ws.unseeded_items),
            "seed_command": seed_cmd,
        },
        "network": net,
        "channels": channels,
        "freshness": freshness,
        "backup_state": inspect_local_backup_health(ws.effective_data_dir),
    }


def collect_data_access_status(
    ws: WorkspaceContext,
    force_refresh: bool = False,
    simulated_profile: str | None = None,
    ttl_secs: float = CACHE_TTL_SECONDS,
) -> dict[str, Any]:
    """Returns instant cached (<2ms, SWR up to 24h on same route) or flock-serialized probe."""
    t0 = time.perf_counter()
    cache_path = get_cache_file_path()
    route_digest = compute_kernel_route_digest()

    if simulated_profile:
        return _build_fresh_status_payload(ws, route_digest, simulated_profile, ttl_secs, t0)

    if not force_refresh:
        cached = _try_load_valid_cache(cache_path, route_digest, str(ws.active_repo), CACHE_MAX_STALE_SECONDS)
        if cached is not None:
            cached["backup_state"] = inspect_local_backup_health(ws.effective_data_dir)
            cached["cache"]["lookup_ms"] = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 2)
            if cached["cache"].get("stale_revalidating"):
                _spawn_background_refresh(ws.active_repo)
            return cached

    lock_path = _ensure_cache_dir(cache_path) / CACHE_LOCK_FILENAME
    with open(lock_path, "w", encoding="utf-8") as lock_handle:
        fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX)
        if not force_refresh:
            after_lock = _try_load_valid_cache(cache_path, route_digest, str(ws.active_repo), ttl_secs)
            if after_lock is not None:
                after_lock["backup_state"] = inspect_local_backup_health(ws.effective_data_dir)
                after_lock["cache"]["lookup_ms"] = round((time.perf_counter() - t0) * MILLIS_PER_SEC, 2)
                return after_lock
        payload = _build_fresh_status_payload(ws, route_digest, None, ttl_secs, t0)
        _atomic_write_cache(cache_path, payload)
        return payload
