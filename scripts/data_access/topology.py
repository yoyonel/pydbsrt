"""Fast kernel network topology, CIFS hang guard, and Git worktree resolver."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from scripts.data_access.constants import (
    CIFS_FSTYPE,
    CORP_VPN_HIJACK_CIDR,
    CORP_WIFI_SSID_PREFIX,
    DEFAULT_NAS_LAN_IP,
    DEFAULT_O2SWITCH_API_KEY_PASS_ENTRY,
    DEFAULT_RPI4_LAN_IP,
    GIT_CMD_TIMEOUT_SECS,
    HOME_SUBNET_CIDR,
    HOME_WIFI_SSID_PREFIX,
    IGNORED_IFACE_PREFIXES,
    IGNORED_NM_TYPES,
    IP_CMD_TIMEOUT_SECS,
    MILLIS_PER_SEC,
    NET_HOME_LAN,
    NET_NOMAD_WAN,
    NET_O2SWITCH,
    NET_RPI4,
    NET_WORK_VPN,
    NMCLI_MIN_FIELDS,
    NMCLI_TIMEOUT_SECS,
    PASS_LOOKUP_TIMEOUT_SECS,
    PROC_MOUNT_MIN_FIELDS,
    PROC_MOUNTS_PATH,
    TUNNEL_DEVICE_PREFIXES,
    WORKTREE_SEED_TARGETS,
)


@dataclass(frozen=True)
class WorkspaceContext:
    """Resolved active repository, parent worktree repository, and dataset roots."""

    active_repo: Path
    common_repo: Path
    is_worktree: bool
    effective_data_dir: Path
    unseeded_items: tuple[str, ...]
    env_vars: dict[str, str]


@dataclass(frozen=True)
class RuntimeEndpoints:
    """Redacted runtime endpoint parameters expanded from environment and .env."""

    local_media_root: str
    nas_lan_host: str
    nas_wan_host: str
    nas_user: str
    nas_pass_entry: str
    rpi4_lan_host: str
    o2switch_url: str


def run_bounded_cmd(cmd: list[str], timeout: float, cwd: Path | None = None) -> tuple[int, str, str]:
    """Executes a subprocess with a strict wall-clock timeout and UTF-8 capture."""
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=cwd, check=False)
        return proc.returncode, proc.stdout.strip(), proc.stderr.strip()
    except subprocess.TimeoutExpired:
        return 124, "", f"timeout after {timeout}s"
    except OSError as exc:
        return 127, "", str(exc)


def parse_dotenv_file(env_path: Path) -> dict[str, str]:
    """Parses simple KEY=VALUE entries from a .env file without shell evaluation."""
    parsed: dict[str, str] = {}
    if not env_path.is_file():
        return parsed
    for raw_line in env_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, val = line.split("=", 1)
        parsed[key.strip()] = val.strip().strip("'\"")
    return parsed


def resolve_workspace_context(start_dir: Path | None = None) -> WorkspaceContext:
    """Detects active repo root and parent git-common-dir for unseeded worktrees."""
    anchor = (start_dir or Path(__file__).resolve().parents[2]).resolve()
    rc_top, out_top, _ = run_bounded_cmd(["git", "rev-parse", "--show-toplevel"], GIT_CMD_TIMEOUT_SECS, cwd=anchor)
    active_repo = Path(out_top).resolve() if rc_top == 0 and out_top else anchor

    rc_com, out_com, _ = run_bounded_cmd(
        ["git", "rev-parse", "--git-common-dir"], GIT_CMD_TIMEOUT_SECS, cwd=active_repo
    )
    common_git = (active_repo / out_com).resolve() if rc_com == 0 and out_com else active_repo / ".git"
    common_repo = common_git.parent if common_git.name == ".git" else active_repo
    is_worktree = active_repo != common_repo

    unseeded: list[str] = []
    if is_worktree:
        for rel in WORKTREE_SEED_TARGETS:
            if (common_repo / rel).exists() and not (active_repo / rel).exists():
                unseeded.append(rel)

    use_parent_data = is_worktree and bool(unseeded)
    effective_data_dir = (common_repo if use_parent_data else active_repo) / "data"

    merged_env = parse_dotenv_file(common_repo / ".env")
    merged_env.update(parse_dotenv_file(active_repo / ".env"))
    for key, val in os.environ.items():
        merged_env[key] = val

    return WorkspaceContext(
        active_repo=active_repo,
        common_repo=common_repo,
        is_worktree=is_worktree,
        effective_data_dir=effective_data_dir,
        unseeded_items=tuple(unseeded),
        env_vars=merged_env,
    )


def resolve_runtime_endpoints(ws: WorkspaceContext) -> RuntimeEndpoints:
    """Builds runtime host and mount parameters without exposing any secret values."""
    env = ws.env_vars
    media_base = Path(env.get("MEDIA_MOUNT_ROOT", "/media")) / Path.home().name
    nas_wan = env.get("NAS_HOST", "<NAS_HOST>")
    nas_user = env.get("NAS_USER", "<NAS_USER>")
    default_pass_entry = f"{DEFAULT_NAS_LAN_IP}:5000/{nas_user}"
    return RuntimeEndpoints(
        local_media_root=str(media_base),
        nas_lan_host=env.get("NAS_LAN_HOST", DEFAULT_NAS_LAN_IP),
        nas_wan_host=nas_wan,
        nas_user=nas_user,
        nas_pass_entry=env.get("NAS_PASS_ENTRY", default_pass_entry),
        rpi4_lan_host=env.get("RPI_HOST", DEFAULT_RPI4_LAN_IP),
        o2switch_url=env.get("O2SWITCH_URL", "https://2026-pydbsrt.fr/dbsrt").rstrip("/"),
    )


def lookup_pass_secret(env_vars: dict[str, str], env_key: str, pass_entry: str) -> str:
    """Resolves a credential from environment or password-store without logging it."""
    if env_vars.get(env_key):
        return env_vars[env_key]
    if not pass_entry or "<" in pass_entry:
        return ""
    rc, out, _ = run_bounded_cmd(["pass", "show", pass_entry], PASS_LOOKUP_TIMEOUT_SECS)
    if rc == 0 and out:
        return out.splitlines()[0].strip()
    return ""


def resolve_o2switch_api_key(ws: WorkspaceContext) -> str:
    """Resolves the master X-API-Key for authenticated HTTPS calls to o2switch."""
    entry = ws.env_vars.get("O2SWITCH_API_KEY_ENTRY", DEFAULT_O2SWITCH_API_KEY_PASS_ENTRY)
    return lookup_pass_secret(ws.env_vars, "DBSRT_API_KEY", entry)


def detect_host_environment() -> str:
    """Identifies whether execution runs on o2switch, rpi4, or workstation."""
    hostname = socket.gethostname().lower()
    home_name = Path.home().name.lower()
    if "sapin" in hostname or "o2switch" in hostname or home_name.startswith("atli"):
        return NET_O2SWITCH
    if "raspberry" in hostname or "rpi" in hostname:
        return NET_RPI4
    return "workstation"


def _read_active_nm_connections() -> list[dict[str, str]]:
    """Reads active NetworkManager connections without triggering Wi-Fi scans."""
    rc, out, _ = run_bounded_cmd(
        ["nmcli", "-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"],
        NMCLI_TIMEOUT_SECS,
    )
    if rc != 0 or not out:
        return []
    conns: list[dict[str, str]] = []
    for line in out.splitlines():
        parts = line.split(":")
        if len(parts) >= NMCLI_MIN_FIELDS and parts[1] not in IGNORED_NM_TYPES:
            conns.append({"name": parts[0], "type": parts[1], "device": parts[2]})
    return conns


def _read_up_interfaces() -> dict[str, list[str]]:
    """Reads global IPv4/IPv6 interface addresses from kernel netlink JSON."""
    rc, out, _ = run_bounded_cmd(["ip", "-j", "addr"], IP_CMD_TIMEOUT_SECS)
    if rc != 0 or not out:
        return {}
    up_map: dict[str, list[str]] = {}
    for iface in json.loads(out):
        ifname = str(iface.get("ifname", ""))
        if not ifname or ifname.startswith(IGNORED_IFACE_PREFIXES):
            continue
        addrs = [
            f"{a['local']}/{a['prefixlen']}"
            for a in iface.get("addr_info", [])
            if a.get("family") in ("inet", "inet6") and a.get("scope") == "global"
        ]
        if addrs or "UP" in iface.get("flags", []):
            up_map[ifname] = addrs
    return up_map


def _read_cifs_proc_mounts(mounts_path: Path = Path(PROC_MOUNTS_PATH)) -> list[dict[str, str]]:
    """Parses /proc/mounts for CIFS shares with zero stat() calls on mountpoints."""
    if not mounts_path.is_file():
        return []
    mounts: list[dict[str, str]] = []
    for line in mounts_path.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) >= PROC_MOUNT_MIN_FIELDS and parts[2] == CIFS_FSTYPE:
            mounts.append({"source": parts[0], "mountpoint": parts[1]})
    return mounts


def _classify_workstation_profile(
    active_conns: list[dict[str, str]],
    up_ifaces: dict[str, list[str]],
    routes: list[dict[str, Any]],
) -> tuple[str, bool, bool]:
    """Classifies network situation and computes CIFS stat() safety invariant."""
    home_route = next((r for r in routes if r.get("dst") == HOME_SUBNET_CIDR), None)
    vpn_hijack = any(r.get("dst") == CORP_VPN_HIJACK_CIDR for r in routes)
    has_tunnel = any(ifn.startswith(TUNNEL_DEVICE_PREFIXES) for ifn in up_ifaces)
    route_dev = str(home_route.get("dev", "")) if home_route else ""
    route_is_direct = home_route is not None and not route_dev.startswith(TUNNEL_DEVICE_PREFIXES)

    on_home_wifi = any(c["name"].startswith(HOME_WIFI_SSID_PREFIX) for c in active_conns)
    on_corp_wifi = any(c["name"].startswith(CORP_WIFI_SSID_PREFIX) for c in active_conns)

    is_home_lan = route_is_direct and (on_home_wifi or not active_conns)
    if is_home_lan:
        return NET_HOME_LAN, True, vpn_hijack
    if on_corp_wifi or (has_tunnel and vpn_hijack and not active_conns):
        return NET_WORK_VPN, False, vpn_hijack
    return NET_NOMAD_WAN, False, vpn_hijack


def inspect_network_topology(simulated_profile: str | None = None) -> dict[str, Any]:
    """Inspects kernel network state in <35ms without touching stale CIFS mounts."""
    t0 = time.perf_counter()
    host_env = detect_host_environment()
    cifs_mounts = _read_cifs_proc_mounts()

    if simulated_profile:
        cifs_safe = simulated_profile == NET_HOME_LAN
        return {
            "host_env": host_env,
            "profile": simulated_profile,
            "simulated": True,
            "fingerprint": f"sim:{simulated_profile}",
            "detect_ms": round((time.perf_counter() - t0) * MILLIS_PER_SEC, 2),
            "active_connections": [],
            "cifs_mounts": cifs_mounts,
            "cifs_mounts_count": len(cifs_mounts),
            "cifs_safe_to_stat": cifs_safe,
            "vpn_192_168_hijack": simulated_profile in (NET_WORK_VPN, NET_NOMAD_WAN),
        }

    if host_env in (NET_O2SWITCH, NET_RPI4):
        return {
            "host_env": host_env,
            "profile": host_env,
            "simulated": False,
            "fingerprint": f"host:{host_env}",
            "detect_ms": round((time.perf_counter() - t0) * MILLIS_PER_SEC, 2),
            "active_connections": [],
            "cifs_mounts": cifs_mounts,
            "cifs_mounts_count": len(cifs_mounts),
            "cifs_safe_to_stat": host_env == NET_RPI4,
            "vpn_192_168_hijack": False,
        }

    active_conns = _read_active_nm_connections()
    up_ifaces = _read_up_interfaces()
    rc_rt, out_rt, _ = run_bounded_cmd(["ip", "-j", "route"], IP_CMD_TIMEOUT_SECS)
    routes: list[dict[str, Any]] = json.loads(out_rt) if rc_rt == 0 and out_rt else []
    profile, cifs_safe, vpn_hijack = _classify_workstation_profile(active_conns, up_ifaces, routes)

    fp_tokens = [f"{c['type']}:{c['name']}" for c in sorted(active_conns, key=lambda x: x["name"])]
    for ifname in sorted(up_ifaces):
        if ifname.startswith(TUNNEL_DEVICE_PREFIXES):
            fp_tokens.append(f"tun:{ifname}")
    fingerprint = f"{profile}|" + ("|".join(fp_tokens) if fp_tokens else "no-nm")

    return {
        "host_env": host_env,
        "profile": profile,
        "simulated": False,
        "fingerprint": fingerprint,
        "detect_ms": round((time.perf_counter() - t0) * MILLIS_PER_SEC, 2),
        "active_connections": active_conns,
        "cifs_mounts": cifs_mounts,
        "cifs_mounts_count": len(cifs_mounts),
        "cifs_safe_to_stat": cifs_safe,
        "vpn_192_168_hijack": vpn_hijack,
    }


def seed_worktree_datasets(ws: WorkspaceContext, dry_run: bool = False) -> dict[str, Any]:
    """Symlinks unversioned datasets from parent repo into an isolated git worktree."""
    if not ws.is_worktree:
        return {
            "status": "noop_not_worktree",
            "active_repo": str(ws.active_repo),
            "created": [],
            "skipped_existing": [],
        }

    created: list[str] = []
    skipped: list[str] = []
    missing_in_parent: list[str] = []

    for rel in WORKTREE_SEED_TARGETS:
        src = ws.common_repo / rel
        dst = ws.active_repo / rel
        if not src.exists():
            missing_in_parent.append(rel)
            continue
        if dst.exists() or dst.is_symlink():
            skipped.append(rel)
            continue
        if not dry_run:
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.symlink_to(src)
        created.append(rel)

    return {
        "status": "dry_run" if dry_run else "seeded",
        "active_repo": str(ws.active_repo),
        "common_repo": str(ws.common_repo),
        "created": created,
        "skipped_existing": skipped,
        "missing_in_parent": missing_in_parent,
    }
