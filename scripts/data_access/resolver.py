"""Declarative SSOT registry loader, channel selector, query matcher, and fetch engine."""

from __future__ import annotations

import hashlib
import json
import os
import re
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

from scripts.data_access.constants import (
    AGENT_HTTP_USER_AGENT,
    ARTIFACT_DOWNLOAD_READ_CHUNK_BYTES,
    CONTAINER_FPS_SUFFIXES,
    CONTROL_CHAR_PATTERN,
    FETCH_CMD_TIMEOUT_SECS,
    HTTP_STATUS_OK,
    HTTPS_FETCH_TIMEOUT_SECS,
    MAX_RESOLVE_MATCHES,
    SNIPPET_STEM_SUFFIXES,
    VALID_ARTIFACT_KINDS,
)
from scripts.data_access.media_search import inspect_media_container, search_media_containers
from scripts.data_access.sidecar_index import (
    load_couplings_index as _load_couplings_index,
)
from scripts.data_access.sidecar_index import (
    search_ahash_corpus,
    search_phash_corpus,
)
from scripts.data_access.topology import (
    RuntimeEndpoints,
    WorkspaceContext,
    resolve_o2switch_api_key,
    resolve_runtime_endpoints,
    run_bounded_cmd,
)

REGISTRY_PATH: Path = Path(__file__).resolve().parent / "sources_registry.json"
CONTROL_CHAR_RE = re.compile(CONTROL_CHAR_PATTERN)
HTTP_BAD_REQUEST_CODE: int = 400
HTTP_NOT_FOUND_CODE: int = 404

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
            "FREEBOX_3_DISKS: 3 shares exist (Disque_1, Disque_2, Disque_1_1) declared in SSOT & portal_server.py; "
            "direct WAN :80 is IP-restricted to o2switch."
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


def _run_family_query(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    family_id: str,
    query: str,
) -> dict[str, Any] | None:
    """Executes family-specific local or multi-network lookup when a query filter is supplied."""
    if not query:
        return None
    data_dir = ws.effective_data_dir
    if family_id == "local-snippets":
        return _search_local_snippets_with_couplings(data_dir, query)
    if family_id == "certified-couplings":
        return _search_couplings_only(data_dir, query)
    if family_id == "phash-files":
        return search_phash_corpus(ws, status_payload, query)
    if family_id == "ahash-files":
        return search_ahash_corpus(ws, status_payload, query)
    if family_id == "snippets-ahash":
        return _search_generic_directory(data_dir / "snippets_ahash", query, ".ahash")
    if family_id in ("freebox-disks", "synology-video", "synology-tvshow"):
        return search_media_containers(ws, status_payload, family_id, query)
    return {"query": query, "note": f"Item search is not applicable to family '{family_id}'."}


def inspect_family_container(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    raw_family: str,
    path_or_query: str,
) -> dict[str, Any]:
    """Resolves family alias and runs local/remote ffprobe container inspection."""
    canonical = normalize_family_id(raw_family)
    return inspect_media_container(ws, status_payload, canonical, path_or_query)


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

    query_res = _run_family_query(ws, status_payload, canonical, query)
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


def _compact_resolved_batch_entry(resolved: dict[str, Any], has_query: bool) -> dict[str, Any]:
    """Prunes redundant verbose fields from a single resolved family entry inside a batch response."""
    drop_keys = ["cross_node_freshness", "description", "channels", "category", "name", "active_profile"]
    if has_query and resolved.get("id") == "freebox-disks":
        drop_keys.append("freebox_disks")
    for drop_key in drop_keys:
        resolved.pop(drop_key, None)
    if "query_result" in resolved:
        _compact_query_languages(resolved["query_result"])
    if "warnings" in resolved:
        resolved["warnings"] = [w for w in resolved["warnings"] if not w.startswith("WORKTREE_UNSEEDED:")]
        if not resolved["warnings"]:
            resolved.pop("warnings", None)
    return resolved


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
        results[spec] = _compact_resolved_batch_entry(resolved, bool(q_part))

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


def _is_safe_artifact_basename(filename: str, allowed_exts: tuple[str, ...]) -> bool:
    """Validates Unicode basename-only artifact filenames while blocking separators and control chars."""
    if not filename or filename in (".", "..") or "/" in filename or "\\" in filename:
        return False
    if CONTROL_CHAR_RE.search(filename) or Path(filename).name != filename:
        return False
    return filename.lower().endswith(allowed_exts)


def _locate_unicode_variant(base_dir: Path, filename: str) -> Path | None:
    """Resolves exact, NFC, or NFD filename inside base_dir while enforcing parent containment."""
    variants = dict.fromkeys((filename, unicodedata.normalize("NFC", filename), unicodedata.normalize("NFD", filename)))
    for variant in variants:
        candidate = (base_dir / variant).resolve()
        if candidate.parent == base_dir and candidate.is_file():
            return candidate
    return None


def resolve_local_artifact_download(data_dir: Path, kind: str, filename: str) -> tuple[Path | None, str, int]:
    """Resolves and validates a traversal-proof artifact file path (supporting NFC/NFD Unicode)."""
    clean_kind = kind.strip().lower()
    if clean_kind not in VALID_ARTIFACT_KINDS:
        return None, "invalid_kind", HTTP_BAD_REQUEST_CODE

    if clean_kind == "couplings":
        eff_name = filename.strip() or "couplings.json"
        if eff_name != "couplings.json":
            return None, "invalid_filename", HTTP_BAD_REQUEST_CODE
        base_dir = (data_dir / "SRT").resolve()
    else:
        eff_name = filename.strip()
        exts = (".phash", ".json") if clean_kind == "phash" else (".ahash", ".json")
        if not _is_safe_artifact_basename(eff_name, exts):
            return None, "invalid_filename", HTTP_BAD_REQUEST_CODE
        folder = "__PHASH__" if clean_kind == "phash" else "__AHASH__"
        base_dir = (data_dir / folder).resolve()

    if not base_dir.is_dir():
        return None, "artifact_not_found", HTTP_NOT_FOUND_CODE
    hit = _locate_unicode_variant(base_dir, eff_name)
    if hit is None:
        return None, "artifact_not_found", HTTP_NOT_FOUND_CODE
    return hit, "ok", HTTP_STATUS_OK


def _download_https_stream(url: str, api_key: str, target_path: Path) -> dict[str, Any]:
    """Streams an authenticated HTTPS binary artifact to disk with SHA-256 verification."""
    headers = {"User-Agent": AGENT_HTTP_USER_AGENT}
    if api_key:
        headers["X-API-Key"] = api_key
    req = urllib.request.Request(url, headers=headers)
    tmp_path = target_path.with_name(f".{target_path.name}.part")
    target_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        with urllib.request.urlopen(req, timeout=HTTPS_FETCH_TIMEOUT_SECS) as resp:
            expected_sha = (resp.headers.get("X-Checksum-SHA256") or "").strip().lower()
            hasher = hashlib.sha256()
            total_bytes = 0
            with tmp_path.open("wb") as out_fh:
                while True:
                    chunk = resp.read(ARTIFACT_DOWNLOAD_READ_CHUNK_BYTES)
                    if not chunk:
                        break
                    out_fh.write(chunk)
                    hasher.update(chunk)
                    total_bytes += len(chunk)
        actual_sha = hasher.hexdigest()
        if expected_sha and actual_sha != expected_sha:
            tmp_path.unlink(missing_ok=True)
            return {"ok": False, "error": f"sha256_mismatch:{actual_sha}!={expected_sha}", "url": url}
        tmp_path.replace(target_path)
        return {
            "ok": True,
            "dry_run": False,
            "source_node": "o2switch_https",
            "protocol": "https_api",
            "url": url,
            "destination": str(target_path),
            "bytes_written": total_bytes,
            "sha256": actual_sha,
        }
    except (urllib.error.URLError, OSError) as exc:
        tmp_path.unlink(missing_ok=True)
        return {"ok": False, "error": f"https_download_failed:{exc}", "url": url}


def _try_https_artifact_fallback(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    canonical: str,
    item: str,
    target_path: Path,
    dry_run: bool,
) -> dict[str, Any] | None:
    """Falls back to authenticated HTTPS :443 download on 4G/5G when SSH :22 is filtered."""
    if not status_payload["channels"].get("https_o2switch", {}).get("accessible"):
        return None
    kind_map = {"phash-files": "phash", "ahash-files": "ahash", "certified-couplings": "couplings"}
    kind = kind_map.get(canonical)
    if not kind:
        return None

    eff_item = "couplings.json" if kind == "couplings" and not item else item
    if not eff_item:
        return {
            "ok": False,
            "family": canonical,
            "error": "https_fallback_requires_item",
            "hint": "SSH :22 is filtered on 4G/5G; pass an explicit filename (`fetch ahash <file.ahash>`) to download over HTTPS :443.",
        }

    endpoints = resolve_runtime_endpoints(ws)
    url = f"{endpoints.o2switch_url}/api/artifact/download?kind={kind}&file={urllib.parse.quote(eff_item, safe='')}"
    if dry_run:
        return {
            "ok": True,
            "dry_run": True,
            "source_node": "o2switch_https",
            "protocol": "https_api",
            "url": url,
            "destination": str(target_path),
        }
    api_key = resolve_o2switch_api_key(ws)
    if not api_key:
        return {"ok": False, "family": canonical, "error": "missing_o2switch_api_key", "url": url}
    return _download_https_stream(url, api_key, target_path)


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


def _compute_fetch_target_path(ws: WorkspaceContext, fam: dict[str, Any], item: str, dest_override: str) -> Path:
    """Computes local destination path for an rsync or HTTPS fetch."""
    local_rel = str(fam["local_rel_path"])
    base_dest = Path(dest_override) if dest_override else (ws.effective_data_dir.parent / local_rel)
    if item and not dest_override and fam["id"] != "certified-couplings":
        return base_dest / item
    return base_dest


def build_or_run_fetch(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    raw_family: str,
    item: str = "",
    dest_override: str = "",
    dry_run: bool = False,
) -> dict[str, Any]:
    """Synthesizes and optionally executes a safe rsync or HTTPS fallback transfer."""
    canonical = normalize_family_id(raw_family)
    raw_reg = load_raw_registry()
    families_by_id = {f["id"]: f for f in raw_reg["families"]}
    fam = families_by_id.get(canonical)
    if not fam or "remote_rel_path" not in fam:
        return {"ok": False, "error": f"fetch_unsupported_for_family:{canonical}"}

    target_path = _compute_fetch_target_path(ws, fam, item, dest_override)
    node = _select_rsync_source_node(canonical, status_payload)
    if not node:
        https_res = _try_https_artifact_fallback(ws, status_payload, canonical, item, target_path, dry_run)
        if https_res is not None:
            return https_res
        return {
            "ok": False,
            "family": canonical,
            "error": "no_ssh_channel_reachable",
            "hint": "SSH :22 is filtered on 4G/5G hotspot; use HTTPS :443 endpoints or connect from whitelisted IP.",
        }

    remote_rel = str(fam["remote_rel_path"])
    if item and canonical != "certified-couplings":
        remote_spec = f"{node}:~/{remote_rel}/{item}"
    elif canonical in ("phash-files", "ahash-files", "reports-and-inventories"):
        remote_spec = f"{node}:~/{remote_rel}/"
    else:
        remote_spec = f"{node}:~/{remote_rel}"

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
