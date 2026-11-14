"""Declarative SSOT registry loader, channel selector, query matcher, and fetch engine."""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Any

from scripts.data_access.constants import (
    CACHE_DIR_MODE,
    CACHE_DIR_REL,
    CONTAINER_FPS_SUFFIXES,
    COUPLINGS_INDEX_CACHE_FILENAME,
    FETCH_CMD_TIMEOUT_SECS,
    MAX_RESOLVE_MATCHES,
    SNIPPET_STEM_SUFFIXES,
)
from scripts.data_access.topology import (
    RuntimeEndpoints,
    WorkspaceContext,
    resolve_runtime_endpoints,
    run_bounded_cmd,
)

REGISTRY_PATH: Path = Path(__file__).resolve().parent / "sources_registry.json"

FAMILY_ALIASES: dict[str, str] = {
    "video": "synology-video",
    "movies": "synology-video",
    "tvshow": "synology-tvshow",
    "tvshows": "synology-tvshow",
    "series": "synology-tvshow",
    "freebox": "freebox-disks",
    "disks": "freebox-disks",
    "snippets": "local-snippets",
    "snippet": "local-snippets",
    "phash": "phash-files",
    "__phash__": "phash-files",
    "ahash": "ahash-files",
    "__ahash__": "ahash-files",
    "catalog": "packed-catalog",
    "catalog.dbphash": "packed-catalog",
    "dbphash": "packed-catalog",
    "couplings": "certified-couplings",
    "couplings.json": "certified-couplings",
    "srt": "certified-couplings",
    "postgres": "postgresql-db",
    "postgresql": "postgresql-db",
    "db": "postgresql-db",
    "reports": "reports-and-inventories",
    "trajectories": "reports-and-inventories",
    "fixtures": "test-fixtures",
}


def load_raw_registry(registry_path: Path = REGISTRY_PATH) -> dict[str, Any]:
    """Loads the declarative JSON registry from disk."""
    return json.loads(registry_path.read_text(encoding="utf-8"))


def normalize_family_id(raw_id: str) -> str:
    """Normalizes user or agent shorthand names to canonical registry family IDs."""
    cleaned = raw_id.strip().lower()
    return FAMILY_ALIASES.get(cleaned, cleaned)


def _format_template_map(ws: WorkspaceContext, endpoints: RuntimeEndpoints) -> dict[str, str]:
    """Builds the string interpolation dictionary for registry target templates."""
    return {
        "local_media_root": endpoints.local_media_root,
        "effective_data_dir": str(ws.effective_data_dir),
        "repo_root": str(ws.active_repo),
        "nas_lan_host": endpoints.nas_lan_host,
        "nas_wan_host": endpoints.nas_wan_host,
        "nas_user": endpoints.nas_user,
        "rpi4_lan_host": endpoints.rpi4_lan_host,
        "o2switch_url": endpoints.o2switch_url,
    }


def _expand_str(template: str, mapping: dict[str, str]) -> str:
    """Expands placeholders in a registry string safely."""
    return template.format_map(mapping)


def expand_freebox_disks(
    ws: WorkspaceContext, endpoints: RuntimeEndpoints, raw_reg: dict[str, Any]
) -> list[dict[str, Any]]:
    """Expands runtime paths and URLs for all 3 Freebox Server disks."""
    mapping = _format_template_map(ws, endpoints)
    expanded: list[dict[str, Any]] = []
    for disk in raw_reg.get("freebox_disks", []):
        entry = dict(disk)
        for key in ("local_cifs_path", "lan_gateway_url", "wan_gateway_url"):
            if isinstance(entry.get(key), str):
                entry[key] = _expand_str(entry[key], mapping)
        expanded.append(entry)
    return expanded


def _annotate_channels(
    raw_channels: list[dict[str, Any]],
    mapping: dict[str, str],
    probed_channels: dict[str, Any],
    active_profile: str,
) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    """Annotates channels with live reachability and selects the optimal active channel."""
    annotated: list[dict[str, Any]] = []
    for ch in raw_channels:
        probe_key = ch.get("probe_key", "")
        probe_info = probed_channels.get(probe_key, {})
        item = {
            "id": ch["id"],
            "protocol": ch["protocol"],
            "target": _expand_str(ch["target"], mapping),
            "profiles": ch.get("profiles", []),
            "accessible": bool(probe_info.get("accessible", False)),
            "latency_ms": probe_info.get("latency_ms", 0.0),
        }
        if probe_info.get("error"):
            item["error"] = probe_info["error"]
        annotated.append(item)

    profile_matches = [c for c in annotated if c["accessible"] and active_profile in c["profiles"]]
    any_accessible = [c for c in annotated if c["accessible"]]
    recommended = profile_matches[0] if profile_matches else (any_accessible[0] if any_accessible else None)
    return annotated, recommended


def _collect_family_warnings(family_id: str, status_payload: dict[str, Any]) -> list[str]:
    """Builds concise operational warnings tailored to the resolved data family."""
    warnings: list[str] = []
    ws_info = status_payload["workspace"]
    net_info = status_payload["network"]
    fresh = status_payload["freshness"]

    if ws_info.get("unseeded_items"):
        warnings.append(
            f"WORKTREE_UNSEEDED: active worktree lacks {len(ws_info['unseeded_items'])} dataset symlinks; "
            f"auto-resolved to parent {ws_info['effective_data_dir']} (run `seed-worktree` before launching daemons)."
        )
    if not net_info.get("cifs_safe_to_stat") and family_id in ("synology-video", "synology-tvshow", "freebox-disks"):
        warnings.append(
            "CIFS_UNSAFE_OFF_HOME_LAN: DO NOT stat/ls /media/.../Synology or Freebox (kernel SMB hang over VPN/WAN)."
        )
    if family_id == "packed-catalog" and fresh["packed_catalog"]["status"] != "UP_TO_DATE":
        cat_f = fresh["packed_catalog"]
        warnings.append(
            f"STALE_LOCAL: local catalog.dbphash lags {cat_f['authoritative_node']} by "
            f"{cat_f.get('delta_medias', 0)} medias ({cat_f.get('delta_bytes', 0)} bytes)."
        )
    if family_id == "ahash-files" and fresh["ahash_corpus"]["status"] != "UP_TO_DATE":
        ah_f = fresh["ahash_corpus"]
        warnings.append(
            f"PARTIAL_LOCAL: most complete .ahash source is {ah_f['most_complete_node']} "
            f"({ah_f['most_complete_count']} files vs local {ah_f['counts_by_node'].get('local', 0)}; rpi4 has 0)."
        )
    if family_id == "freebox-disks":
        warnings.append(
            "FREEBOX_3_DISKS: 3 shares exist (Disque_1, Disque_2, Disque_1_1); "
            "legacy portal_server.py omits Disque_1_1 and direct WAN :80 is IP-restricted to o2switch."
        )
    return warnings


def normalize_release_stem(raw_name: str) -> str:
    """Strips container extensions, FPS suffixes, and snippet cut suffixes for coupling lookup."""
    stem = raw_name.strip()
    for ext in (".mp4", ".mkv", ".avi", ".phash", ".ahash", ".json", ".srt"):
        if stem.lower().endswith(ext):
            stem = stem[: -len(ext)]
    for fps_suf in CONTAINER_FPS_SUFFIXES:
        if stem.lower().endswith(fps_suf):
            stem = stem[: -len(fps_suf)]
    for snip_suf in SNIPPET_STEM_SUFFIXES:
        if stem.lower().endswith(snip_suf):
            stem = stem[: -len(snip_suf)]
    return stem.lower()


def _couplings_index_cache_path() -> Path:
    """Returns user-scoped mtime-keyed cache path for the 41 MB couplings.json index."""
    return Path.home().joinpath(*CACHE_DIR_REL, COUPLINGS_INDEX_CACHE_FILENAME)


def _try_read_couplings_cache(cache_path: Path, source_key: str) -> tuple[dict[str, dict[str, Any]], int] | None:
    """Loads pre-indexed couplings summary in ~1.5ms when couplings.json mtime+size match."""
    if not cache_path.is_file():
        return None
    try:
        cached = json.loads(cache_path.read_text(encoding="utf-8"))
        if cached.get("source_key") == source_key and isinstance(cached.get("by_norm"), dict):
            return cached["by_norm"], int(cached.get("total_entries", 0))
    except (OSError, ValueError, TypeError):
        return None
    return None


def _write_couplings_cache(cache_path: Path, source_key: str, by_norm: dict[str, Any], total: int) -> None:
    """Persists lightweight couplings summary index (~80 KB vs 41 MB raw cues)."""
    try:
        cache_path.parent.mkdir(parents=True, exist_ok=True, mode=CACHE_DIR_MODE)
        cache_path.write_text(
            json.dumps({"source_key": source_key, "total_entries": total, "by_norm": by_norm}, ensure_ascii=False),
            encoding="utf-8",
        )
    except OSError:
        pass


def _load_couplings_index(data_dir: Path) -> tuple[dict[str, dict[str, Any]], int]:
    """Indexes certified couplings by normalized release stem with O(1) mtime cache."""
    coup_path = data_dir / "SRT" / "couplings.json"
    if not coup_path.is_file():
        return {}, 0
    st = coup_path.stat()
    source_key = f"{coup_path.resolve()}:{st.st_size}:{st.st_mtime_ns}"
    idx_cache = _couplings_index_cache_path()
    hit = _try_read_couplings_cache(idx_cache, source_key)
    if hit is not None:
        return hit

    payload = json.loads(coup_path.read_text(encoding="utf-8"))
    entries = payload.get("couplings", []) if isinstance(payload, dict) else []
    by_norm: dict[str, dict[str, Any]] = {}
    for idx, entry in enumerate(entries):
        media_name = str(entry.get("media_name", ""))
        if not media_name:
            continue
        subs = entry.get("subtitles", [])
        langs = sorted({str(s.get("lang", "")) for s in subs if isinstance(s, dict) and s.get("lang")})
        summary = {
            "coupling_index": idx,
            "media_name": media_name,
            "media_hash": entry.get("media_hash"),
            "phash_filename": entry.get("phash_filename"),
            "certified_subtitles_count": len(subs),
            "languages": langs,
        }
        by_norm[media_name.lower()] = summary
        by_norm[normalize_release_stem(media_name)] = summary
    _write_couplings_cache(idx_cache, source_key, by_norm, len(entries))
    return by_norm, len(entries)


def _matches_release_token(filename: str, query_low: str) -> bool:
    """Checks if query matches a word/token prefix in a media release name (avoids 'andor' matching 'Pandora')."""
    tokens = [tok for tok in re.split(r"[._\-\s()\[\]]+", filename.lower()) if tok]
    return any(tok.startswith(query_low) for tok in tokens)


def _search_local_snippets_with_couplings(data_dir: Path, query: str) -> dict[str, Any]:
    """Finds matching local MP4 snippets and cross-checks each against couplings.json."""
    snip_dir = data_dir / "snippets"
    if not snip_dir.is_dir():
        return {"query": query, "matches_count": 0, "matches": []}
    q_low = query.lower()
    coup_index, total_coupled = _load_couplings_index(data_dir)

    all_substr = sorted(
        entry.name for entry in os.scandir(snip_dir) if entry.name.endswith(".mp4") and q_low in entry.name.lower()
    )
    token_filtered = [name for name in all_substr if _matches_release_token(name, q_low)]
    mp4_names = token_filtered if token_filtered else all_substr

    matches: list[dict[str, Any]] = []
    for name in mp4_names[:MAX_RESOLVE_MATCHES]:
        full = snip_dir / name
        stem_exact = name[:-4].lower()
        norm_stem = normalize_release_stem(name)
        direct_hit = coup_index.get(stem_exact)
        parent_hit = coup_index.get(norm_stem)
        sidecars = [
            suf
            for suf in (".phash", ".en.srt", ".fr.srt", ".synced.fr.srt")
            if (snip_dir / f"{name[:-4]}{suf}").is_file()
        ]
        matches.append(
            {
                "filename": name,
                "path": str(full),
                "size_bytes": full.stat().st_size,
                "sidecars_present": sidecars,
                "direct_snippet_in_couplings": direct_hit is not None,
                "parent_media_coupled": parent_hit is not None,
                "parent_coupling": parent_hit,
            }
        )
    return {
        "query": query,
        "couplings_registry_total_medias": total_coupled,
        "matches_count": len(mp4_names),
        "matches": matches,
    }


def _search_generic_directory(folder: Path, query: str, suffix: str) -> dict[str, Any]:
    """Searches a local artifact folder by substring up to MAX_RESOLVE_MATCHES."""
    if not folder.is_dir():
        return {"query": query, "matches_count": 0, "matches": []}
    q_low = query.lower()
    matched_names = sorted(e.name for e in os.scandir(folder) if e.name.endswith(suffix) and q_low in e.name.lower())
    items = [
        {"filename": name, "path": str(folder / name), "size_bytes": (folder / name).stat().st_size}
        for name in matched_names[:MAX_RESOLVE_MATCHES]
    ]
    return {"query": query, "matches_count": len(matched_names), "matches": items}


def _search_couplings_only(data_dir: Path, query: str) -> dict[str, Any]:
    """Searches certified couplings.json entries matching a substring query."""
    coup_index, total_coupled = _load_couplings_index(data_dir)
    q_low = query.lower()
    seen_hashes: set[str] = set()
    matches: list[dict[str, Any]] = []
    for summary in coup_index.values():
        key = str(summary.get("media_hash") or summary["media_name"])
        if key in seen_hashes:
            continue
        if q_low in summary["media_name"].lower():
            seen_hashes.add(key)
            matches.append(summary)
    matches.sort(key=lambda x: int(x["coupling_index"]))
    return {
        "query": query,
        "couplings_registry_total_medias": total_coupled,
        "matches_count": len(matches),
        "matches": matches[:MAX_RESOLVE_MATCHES],
    }


def _run_family_query(family_id: str, data_dir: Path, query: str) -> dict[str, Any] | None:
    """Executes family-specific local lookup when an optional query filter is supplied."""
    if not query:
        return None
    if family_id == "local-snippets":
        return _search_local_snippets_with_couplings(data_dir, query)
    if family_id == "certified-couplings":
        return _search_couplings_only(data_dir, query)
    if family_id == "phash-files":
        return _search_generic_directory(data_dir / "__PHASH__", query, ".phash")
    if family_id == "ahash-files":
        return _search_generic_directory(data_dir / "__AHASH__", query, ".ahash")
    return {"query": query, "note": "Live substring search is supported locally for snippets, couplings, phash, ahash."}


def resolve_data_family(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    raw_family: str,
    query: str = "",
) -> dict[str, Any]:
    """Resolves optimal access channels, freshness state, and optional item query."""
    raw_reg = load_raw_registry()
    endpoints = resolve_runtime_endpoints(ws)
    mapping = _format_template_map(ws, endpoints)
    probed = status_payload["channels"]
    active_profile = status_payload["network"]["profile"]

    canonical = normalize_family_id(raw_family)
    families_by_id = {f["id"]: f for f in raw_reg["families"]}

    if canonical == "all":
        summary_items = []
        for fam in raw_reg["families"]:
            chans, rec = _annotate_channels(fam["channels"], mapping, probed, active_profile)
            summary_items.append(
                {
                    "id": fam["id"],
                    "desync_tier": fam["desync_tier"],
                    "recommended_channel": rec,
                    "accessible_channels_count": sum(1 for c in chans if c["accessible"]),
                }
            )
        return {
            "profile": active_profile,
            "families": summary_items,
            "freebox_disks": expand_freebox_disks(ws, endpoints, raw_reg),
        }

    if canonical not in families_by_id:
        return {
            "error": f"unknown_family:{raw_family}",
            "available_families": sorted(families_by_id.keys()),
        }

    fam = families_by_id[canonical]
    annotated_channels, recommended = _annotate_channels(fam["channels"], mapping, probed, active_profile)
    result: dict[str, Any] = {
        "id": fam["id"],
        "name": fam["name"],
        "category": fam["category"],
        "desync_tier": fam["desync_tier"],
        "description": fam["description"],
        "active_profile": active_profile,
        "recommended_channel": recommended,
        "channels": annotated_channels,
        "warnings": _collect_family_warnings(canonical, status_payload),
    }
    if canonical == "freebox-disks":
        result["freebox_disks"] = expand_freebox_disks(ws, endpoints, raw_reg)
        lan_gw = probed.get("lan_http_freebox", {})
        if lan_gw.get("mounted_shares"):
            result["gateway_mounted_shares_live"] = lan_gw["mounted_shares"]
    if canonical in ("packed-catalog", "phash-files", "ahash-files", "certified-couplings"):
        result["cross_node_freshness"] = status_payload["freshness"]

    query_res = _run_family_query(canonical, ws.effective_data_dir, query)
    if query_res is not None:
        result["query_result"] = query_res
    return result


def _compact_query_languages(query_res: dict[str, Any]) -> None:
    """Compacts 21-item language arrays into comma-separated strings for batch queries."""
    for match in query_res.get("matches", []):
        if isinstance(match.get("languages"), list):
            match["languages"] = ",".join(match["languages"])
        pc = match.get("parent_coupling")
        if isinstance(pc, dict) and isinstance(pc.get("languages"), list):
            pc["languages"] = ",".join(pc["languages"])


def resolve_batch_specs(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    specs: list[str],
) -> dict[str, Any]:
    """Resolves multiple 'family[:query]' specs in a single deduplicated <6KB payload."""
    results: dict[str, Any] = {}
    for spec in specs:
        fam_part, _, q_part = spec.partition(":")
        resolved = resolve_data_family(ws, status_payload, fam_part, query=q_part)
        for drop_key in ("cross_node_freshness", "description", "channels", "category", "name", "active_profile"):
            resolved.pop(drop_key, None)
        if "query_result" in resolved:
            _compact_query_languages(resolved["query_result"])
        if "warnings" in resolved:
            resolved["warnings"] = [w for w in resolved["warnings"] if not w.startswith("WORKTREE_UNSEEDED:")]
            if not resolved["warnings"]:
                resolved.pop("warnings", None)
        results[spec] = resolved

    return {
        "cache": status_payload["cache"],
        "workspace": status_payload["workspace"],
        "network": {
            "host_env": status_payload["network"]["host_env"],
            "profile": status_payload["network"]["profile"],
            "cifs_safe_to_stat": status_payload["network"]["cifs_safe_to_stat"],
        },
        "freshness": status_payload["freshness"],
        "results": results,
    }


def _select_rsync_source_node(canonical: str, status_payload: dict[str, Any]) -> str | None:
    """Chooses the best SSH-reachable remote node for fetching a dataset family."""
    chans = status_payload["channels"]
    rpi_ok = bool(chans.get("ssh_rpi4", {}).get("accessible"))
    o2_ok = bool(chans.get("ssh_o2switch", {}).get("accessible"))

    if canonical == "ahash-files":
        return "o2switch" if o2_ok else None
    if canonical in ("packed-catalog", "phash-files", "certified-couplings", "reports-and-inventories"):
        if rpi_ok:
            return "rpi4"
        if o2_ok:
            return "o2switch"
    return None


def build_or_run_fetch(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    raw_family: str,
    item: str = "",
    dest_override: str = "",
    dry_run: bool = False,
) -> dict[str, Any]:
    """Synthesizes and optionally executes a safe rsync transfer from rpi4 or o2switch."""
    canonical = normalize_family_id(raw_family)
    raw_reg = load_raw_registry()
    families_by_id = {f["id"]: f for f in raw_reg["families"]}
    fam = families_by_id.get(canonical)
    if not fam or "remote_rel_path" not in fam:
        return {"ok": False, "error": f"fetch_unsupported_for_family:{canonical}"}

    node = _select_rsync_source_node(canonical, status_payload)
    if not node:
        return {
            "ok": False,
            "family": canonical,
            "error": "no_ssh_channel_reachable",
            "hint": "SSH :22 is filtered on 4G/5G hotspot; use HTTPS :443 endpoints or connect from whitelisted IP.",
        }

    remote_rel = str(fam["remote_rel_path"])
    local_rel = str(fam["local_rel_path"])
    base_dest = Path(dest_override) if dest_override else (ws.effective_data_dir.parent / local_rel)

    if item:
        remote_spec = f"{node}:~/{remote_rel}/{item}"
        target_path = base_dest if dest_override else (base_dest / item)
    elif canonical in ("phash-files", "ahash-files", "reports-and-inventories"):
        remote_spec = f"{node}:~/{remote_rel}/"
        target_path = base_dest
    else:
        remote_spec = f"{node}:~/{remote_rel}"
        target_path = base_dest

    cmd = ["rsync", "-az", "--partial", remote_spec, str(target_path)]
    if dry_run:
        return {"ok": True, "dry_run": True, "source_node": node, "command": cmd, "destination": str(target_path)}

    target_path.parent.mkdir(parents=True, exist_ok=True)
    rc, out, err = run_bounded_cmd(cmd, FETCH_CMD_TIMEOUT_SECS)
    return {
        "ok": rc == 0,
        "dry_run": False,
        "source_node": node,
        "command": cmd,
        "destination": str(target_path),
        "returncode": rc,
        "stdout": out,
        "stderr": err,
    }
