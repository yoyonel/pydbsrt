"""Multi-network physical media container search and remote/local ffprobe inspection."""

from __future__ import annotations

import json
import os
import re
import shutil
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from scripts.data_access.constants import (
    AGENT_HTTP_USER_AGENT,
    CACHE_DIR_MODE,
    DURATION_ROUND_DECIMALS,
    EPISODE_NOISE_TOKEN_PATTERN,
    EXCLUDED_MEDIA_DIRS,
    FREEBOX_GATEWAY_PORT,
    FREEBOX_INDEXED_PREFIXES,
    HTTP_GATEWAY_TIMEOUT_SECS,
    HTTP_STATUS_OK,
    HTTPS_API_TIMEOUT_SECS,
    HTTPS_INSPECT_TIMEOUT_SECS,
    HTTPS_JSON_MAX_BYTES,
    LOCAL_FFPROBE_TIMEOUT_SECS,
    MAX_INSPECT_SUBTITLE_STREAMS,
    MAX_MEDIA_QUERY_MATCHES,
    MAX_SERIES_SUBDIR_SCANS,
    MEDIA_LISTING_CACHE_FILENAME,
    MEDIA_LISTING_TTL_SECONDS,
    PORTAL_MAX_PAGE_SIZE,
    PORTAL_MAX_SCAN_PAGES,
    SUPPORTED_VIDEO_EXTS,
)
from scripts.data_access.prober import get_cache_file_path
from scripts.data_access.sidecar_index import (
    CrossArtifactIndex,
    build_cross_artifact_index,
    filter_and_rank_entries,
    lookup_cross_artifacts,
    tokenize_search_text,
)
from scripts.data_access.topology import (
    RuntimeEndpoints,
    WorkspaceContext,
    resolve_o2switch_api_key,
    resolve_runtime_endpoints,
    run_bounded_cmd,
)

EPISODE_NOISE_RE = re.compile(EPISODE_NOISE_TOKEN_PATTERN, re.IGNORECASE)
SERIES_CONJUNCTION_STOPWORDS = frozenset({"and", "et", "the", "le", "la", "les", "de", "du", "des"})
EXCLUDED_DIR_SET = frozenset(EXCLUDED_MEDIA_DIRS)


@dataclass(frozen=True)
class MediaSearchContext:
    """Immutable execution bundle for multi-network container discovery and inspection."""

    ws: WorkspaceContext
    endpoints: RuntimeEndpoints
    status_payload: dict[str, Any]
    cross: CrossArtifactIndex


def _is_cifs_accessible_and_safe(status_payload: dict[str, Any]) -> bool:
    """True strictly when kernel routing guarantees Home LAN safety AND CIFS mounts responded."""
    net_safe = bool(status_payload.get("network", {}).get("cifs_safe_to_stat", False))
    cifs_up = bool(status_payload.get("channels", {}).get("local_cifs", {}).get("accessible", False))
    return net_safe and cifs_up


def _listing_cache_path() -> Path:
    """Returns TTL-cached media listing path colocated with the active DBSRT cache."""
    return get_cache_file_path().parent / MEDIA_LISTING_CACHE_FILENAME


def _read_listing_cache(cache_key: str) -> list[dict[str, Any]] | None:
    """Reads a TTL-bounded directory/catalog listing from local disk cache."""
    cpath = _listing_cache_path()
    if not cpath.is_file():
        return None
    try:
        store = json.loads(cpath.read_text(encoding="utf-8"))
        entry = store.get(cache_key) if isinstance(store, dict) else None
        if not isinstance(entry, dict):
            return None
        if (time.time() - float(entry.get("ts", 0.0))) <= MEDIA_LISTING_TTL_SECONDS:
            items = entry.get("items")
            return items if isinstance(items, list) else None
    except (OSError, ValueError, TypeError):
        return None
    return None


def _write_listing_cache(cache_key: str, items: list[dict[str, Any]]) -> None:
    """Updates TTL-bounded directory/catalog listing cache atomically."""
    cpath = _listing_cache_path()
    try:
        cpath.parent.mkdir(parents=True, exist_ok=True, mode=CACHE_DIR_MODE)
        store: dict[str, Any] = {}
        if cpath.is_file():
            try:
                loaded = json.loads(cpath.read_text(encoding="utf-8"))
                if isinstance(loaded, dict):
                    now = time.time()
                    store = {
                        k: v
                        for k, v in loaded.items()
                        if isinstance(v, dict) and (now - float(v.get("ts", 0.0))) <= MEDIA_LISTING_TTL_SECONDS
                    }
            except (OSError, ValueError):
                store = {}
        store[cache_key] = {"ts": time.time(), "items": items}
        tmp = cpath.with_suffix(".tmp")
        tmp.write_text(json.dumps(store, ensure_ascii=False), encoding="utf-8")
        tmp.replace(cpath)
    except OSError:
        pass


def _http_get_json(url: str, timeout: float, api_key: str = "") -> dict[str, Any] | None:
    """Executes bounded HTTP/HTTPS GET returning parsed JSON dictionary or None."""
    headers = {"Accept": "application/json", "User-Agent": AGENT_HTTP_USER_AGENT}
    if api_key:
        headers["X-API-Key"] = api_key
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status != HTTP_STATUS_OK or resp.headers.get_content_type() != "application/json":
                return None
            raw = resp.read(HTTPS_JSON_MAX_BYTES).decode("utf-8", errors="replace")
            parsed = json.loads(raw)
            return parsed if isinstance(parsed, dict) else None
    except (urllib.error.URLError, OSError, ValueError):
        return None


def _portal_build_files_url(
    ctx: MediaSearchContext,
    source_id: str,
    subdir: str,
    extra: tuple[str, int],
) -> str:
    """Formats authenticated /api/files query URL for o2switch."""
    query_param, page = extra
    profile_id = "wan_http" if source_id == "freebox-disks" else "wan_ftp"
    params = urllib.parse.urlencode(
        {
            "source": source_id,
            "profile": profile_id,
            "subdir": subdir,
            "page": str(page),
            "limit": str(PORTAL_MAX_PAGE_SIZE),
            "q": query_param,
        }
    )
    return f"{ctx.endpoints.o2switch_url}/api/files?{params}"


def _portal_query_first_page(ctx: MediaSearchContext, source_id: str, query_str: str) -> dict[str, Any] | None:
    """Calls o2switch /api/files server-side query fast-path (page 1) over HTTPS :443."""
    api_key = resolve_o2switch_api_key(ctx.ws)
    if not api_key:
        return None
    url = _portal_build_files_url(ctx, source_id, "", (query_str, 1))
    return _http_get_json(url, HTTPS_API_TIMEOUT_SECS, api_key=api_key)


def _portal_list_subdir_page(
    ctx: MediaSearchContext,
    source_id: str,
    subdir: str,
    page: int,
) -> dict[str, Any] | None:
    """Calls o2switch /api/files for a single unfiltered directory page over HTTPS :443."""
    api_key = resolve_o2switch_api_key(ctx.ws)
    if not api_key:
        return None
    url = _portal_build_files_url(ctx, source_id, subdir, ("", page))
    return _http_get_json(url, HTTPS_API_TIMEOUT_SECS, api_key=api_key)


def _fetch_portal_full_subdir(ctx: MediaSearchContext, source_id: str, subdir: str) -> list[dict[str, Any]]:
    """Fetches and caches all items in a remote portal directory (up to PORTAL_MAX_SCAN_PAGES)."""
    cache_key = f"portal:{ctx.endpoints.o2switch_url}:{source_id}:{subdir}"
    cached = _read_listing_cache(cache_key)
    if cached is not None:
        return cached

    first = _portal_list_subdir_page(ctx, source_id, subdir, 1)
    if not isinstance(first, dict) or first.get("status") != "ok":
        return []
    collected: list[dict[str, Any]] = list(first.get("items", []))
    total_pages = min(int(first.get("total_pages", 1)), PORTAL_MAX_SCAN_PAGES)
    for page_num in range(2, total_pages + 1):
        nxt = _portal_list_subdir_page(ctx, source_id, subdir, page_num)
        if not isinstance(nxt, dict) or nxt.get("status") != "ok":
            break
        collected.extend(nxt.get("items", []))

    if collected:
        _write_listing_cache(cache_key, collected)
    return collected


def _scan_local_cifs_subdir(base_root: Path, rel_subdir: str) -> list[dict[str, Any]]:
    """Scans a local CIFS directory ONLY after caller verified _is_cifs_accessible_and_safe."""
    target = base_root if not rel_subdir else (base_root / rel_subdir)
    if not target.is_dir():
        return []
    out: list[dict[str, Any]] = []
    try:
        for entry in os.scandir(target):
            if entry.name.startswith(".") or entry.name.lower() in EXCLUDED_DIR_SET:
                continue
            is_dir = entry.is_dir()
            ext = Path(entry.name).suffix.lower()
            if not is_dir and ext not in SUPPORTED_VIDEO_EXTS:
                continue
            rel_p = f"{rel_subdir}/{entry.name}".lstrip("/") if rel_subdir else entry.name
            out.append(
                {
                    "name": unicodedata.normalize("NFC", entry.name),
                    "path": unicodedata.normalize("NFC", rel_p),
                    "is_dir": is_dir,
                    "size_bytes": 0 if is_dir else entry.stat().st_size,
                }
            )
    except OSError:
        return []
    return out


def _list_synology_subdir(ctx: MediaSearchContext, source_id: str, subdir: str) -> tuple[str, list[dict[str, Any]]]:
    """Lists a Synology share subdirectory via safe local CIFS or remote HTTPS o2switch."""
    share_sub = "video" if source_id == "synology-video" else "tvshow"
    if _is_cifs_accessible_and_safe(ctx.status_payload):
        cifs_root = Path(ctx.endpoints.local_media_root) / "Synology" / share_sub
        items = _scan_local_cifs_subdir(cifs_root, subdir)
        if items:
            return "local_cifs", items

    if ctx.status_payload.get("channels", {}).get("https_o2switch", {}).get("accessible"):
        return "o2switch_https_bounce", _fetch_portal_full_subdir(ctx, source_id, subdir)
    return "unavailable", []


def _extract_series_folder_query(query: str) -> str:
    """Extracts series-title search tokens by stripping SxxExx, resolution, and conjunction noise."""
    raw_tokens = tokenize_search_text(query)
    series_tokens = [tok for tok in raw_tokens if not EPISODE_NOISE_RE.match(tok)]
    without_conj = [tok for tok in series_tokens if tok not in SERIES_CONJUNCTION_STOPWORDS]
    effective = without_conj if without_conj else (series_tokens if series_tokens else raw_tokens)
    return " ".join(effective)


def _populate_freebox_access_fields(ctx: MediaSearchContext, rel_path: str, cifs_ok: bool) -> dict[str, Any]:
    """Builds disk, LAN HTTP stream_url, and local_cifs_path fields for a Freebox container."""
    fields: dict[str, Any] = {"disk": rel_path.split("/", 1)[0] if "/" in rel_path else ""}
    quoted = "/".join(urllib.parse.quote(seg) for seg in rel_path.split("/"))
    if ctx.status_payload.get("channels", {}).get("lan_http_freebox", {}).get("accessible"):
        fields["stream_url"] = f"http://{ctx.endpoints.rpi4_lan_host}:{FREEBOX_GATEWAY_PORT}/{quoted}"
    if cifs_ok:
        fields["local_cifs_path"] = str(Path(ctx.endpoints.local_media_root) / "Freebox" / rel_path)
    return fields


def _populate_synology_access_fields(
    ctx: MediaSearchContext,
    family_id: str,
    rel_path: str,
    cifs_ok: bool,
) -> dict[str, Any]:
    """Builds local_cifs_path and FTP URI fields for a Synology NAS container."""
    share = "video" if family_id == "synology-video" else "tvshow"
    fields: dict[str, Any] = {}
    if cifs_ok:
        fields["local_cifs_path"] = str(Path(ctx.endpoints.local_media_root) / "Synology" / share / rel_path)
    ftp_host = (
        ctx.endpoints.nas_lan_host
        if ctx.status_payload.get("channels", {}).get("lan_ftp_synology", {}).get("accessible")
        else ctx.endpoints.nas_wan_host
    )
    fields["ftp_uri"] = f"ftp://{ctx.endpoints.nas_user}@{ftp_host}:21/{share}/{rel_path}"
    return fields


def _format_container_hit(
    ctx: MediaSearchContext,
    family_id: str,
    via_channel: str,
    raw_item: dict[str, Any],
) -> dict[str, Any]:
    """Formats a discovered media container with access URIs, inspect command, and cross-artifacts."""
    rel_path = str(raw_item.get("path") or raw_item.get("name") or "").lstrip("/")
    fname = str(raw_item.get("filename") or raw_item.get("name") or Path(rel_path).name)
    art = lookup_cross_artifacts(ctx.cross, fname)
    cifs_ok = _is_cifs_accessible_and_safe(ctx.status_payload)

    hit: dict[str, Any] = {
        "filename": fname,
        "path": rel_path,
        "size_bytes": int(raw_item.get("size_bytes") or 0),
        "via": via_channel,
    }
    if family_id == "freebox-disks":
        hit.update(_populate_freebox_access_fields(ctx, rel_path, cifs_ok))
    else:
        hit.update(_populate_synology_access_fields(ctx, family_id, rel_path, cifs_ok))

    short_fam = {"freebox-disks": "freebox", "synology-video": "video", "synology-tvshow": "tvshow"}[family_id]
    hit["moviehash"] = art["moviehash"]
    hit["phash_file"] = art["phash_file"]
    hit["ahash_file"] = art["ahash_file"]
    hit["coupled_srt"] = art["coupled_srt"]
    if art["coupled_srt"]:
        hit["coupled_subtitles_count"] = art["coupled_subtitles_count"]
        hit["coupled_languages"] = art["coupled_languages"]
    hit["inspect_cmd"] = (
        f'python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py inspect {short_fam} "{rel_path}"'
    )
    return hit


def _walk_freebox_share_root(fb_root: Path, sub_root: Path) -> list[dict[str, Any]]:
    """Walks a single indexed Freebox share subdirectory excluding Photos/Backup/hidden dirs."""
    items: list[dict[str, Any]] = []
    for dirpath, dirnames, filenames in os.walk(sub_root):
        dirnames[:] = [d for d in dirnames if not d.startswith(".") and d.lower() not in EXCLUDED_DIR_SET]
        for fn in sorted(filenames):
            if Path(fn).suffix.lower() in SUPPORTED_VIDEO_EXTS and not fn.startswith("."):
                fp = Path(dirpath) / fn
                items.append(
                    {
                        "filename": unicodedata.normalize("NFC", fn),
                        "path": unicodedata.normalize("NFC", fp.relative_to(fb_root).as_posix()),
                        "size_bytes": fp.stat().st_size,
                    }
                )
    return items


def _walk_safe_freebox_cifs(fb_root: Path) -> list[dict[str, Any]]:
    """Walks the 3 official Freebox CIFS share directories when Home LAN safety is verified."""
    walked: list[dict[str, Any]] = []
    for mount_dir, indexed_sub in FREEBOX_INDEXED_PREFIXES:
        sub_root = fb_root / mount_dir / indexed_sub
        if sub_root.is_dir():
            walked.extend(_walk_freebox_share_root(fb_root, sub_root))
    return walked


def _fetch_lan_freebox_catalog(rpi4_host: str) -> list[dict[str, Any]]:
    """Queries or reads cached Freebox HTTP gateway /catalog on Home LAN."""
    gw_url = f"http://{rpi4_host}:{FREEBOX_GATEWAY_PORT}/catalog"
    cache_key = f"fbgw:{gw_url}"
    cached = _read_listing_cache(cache_key)
    if cached is not None:
        return cached
    payload = _http_get_json(gw_url, HTTP_GATEWAY_TIMEOUT_SECS)
    items = payload.get("items", []) if isinstance(payload, dict) else []
    if isinstance(items, list) and items:
        _write_listing_cache(cache_key, items)
        return items
    return []


def _load_offline_freebox_targets(data_dir: Path) -> list[dict[str, Any]]:
    """Loads fallback offline Freebox targets list when all live transports are unreachable."""
    fallback_file = data_dir / "reports" / "freebox_batch_targets.json"
    if not fallback_file.is_file():
        return []
    try:
        raw = json.loads(fallback_file.read_text(encoding="utf-8"))
        return raw if isinstance(raw, list) and raw else []
    except (OSError, ValueError):
        return []


def _collect_freebox_items(ctx: MediaSearchContext, query: str) -> tuple[str, list[dict[str, Any]]]:
    """Discovers Freebox containers across all 3 disks via LAN HTTP, safe CIFS, HTTPS o2switch, or cache."""
    chans = ctx.status_payload.get("channels", {})
    if chans.get("lan_http_freebox", {}).get("accessible"):
        gw_items = _fetch_lan_freebox_catalog(ctx.endpoints.rpi4_lan_host)
        if gw_items:
            return "lan_http_gateway", gw_items

    if _is_cifs_accessible_and_safe(ctx.status_payload):
        walked = _walk_safe_freebox_cifs(Path(ctx.endpoints.local_media_root) / "Freebox")
        if walked:
            return "local_cifs", walked

    if chans.get("https_o2switch", {}).get("accessible"):
        fast = _portal_query_first_page(ctx, "freebox-disks", query)
        fast_items = fast.get("items", []) if isinstance(fast, dict) and fast.get("status") == "ok" else []
        return "o2switch_https_bounce", fast_items or _fetch_portal_full_subdir(ctx, "freebox-disks", "")

    offline = _load_offline_freebox_targets(ctx.ws.effective_data_dir)
    return ("offline_batch_targets_json", offline) if offline else ("unavailable", [])


def _load_synology_video_root(ctx: MediaSearchContext, query: str) -> tuple[str, list[dict[str, Any]]]:
    """Fetches Synology /video root entries via local CIFS or o2switch HTTPS fast query."""
    https_ok = bool(ctx.status_payload.get("channels", {}).get("https_o2switch", {}).get("accessible"))
    if not _is_cifs_accessible_and_safe(ctx.status_payload) and https_ok:
        fast = _portal_query_first_page(ctx, "synology-video", query)
        fast_items = fast.get("items", []) if isinstance(fast, dict) and fast.get("status") == "ok" else []
        if fast_items:
            return "o2switch_https_bounce", fast_items
    return _list_synology_subdir(ctx, "synology-video", "")


def _search_synology_video(ctx: MediaSearchContext, query: str) -> tuple[str, int, list[dict[str, Any]]]:
    """Searches Synology /video root and expands up to MAX_SERIES_SUBDIR_SCANS matching movie folders."""
    via, root_items = _load_synology_video_root(ctx, query)
    _, matched_entries = filter_and_rank_entries(root_items, query, ("name", "path"), limit=PORTAL_MAX_PAGE_SIZE)
    files: list[dict[str, Any]] = [it for it in matched_entries if not it.get("is_dir")]
    dirs: list[dict[str, Any]] = [it for it in matched_entries if it.get("is_dir")][:MAX_SERIES_SUBDIR_SCANS]

    for folder in dirs:
        folder_path = str(folder.get("path") or folder.get("name") or "")
        _, children = _list_synology_subdir(ctx, "synology-video", folder_path)
        files.extend(ch for ch in children if not ch.get("is_dir"))

    total_cnt, ranked = filter_and_rank_entries(files, query, ("name", "path"), limit=MAX_MEDIA_QUERY_MATCHES)
    if total_cnt == 0 and files:
        return via, len(files), files[:MAX_MEDIA_QUERY_MATCHES]
    return via, total_cnt, ranked


def _collect_series_episodes(ctx: MediaSearchContext, sdir: dict[str, Any]) -> list[dict[str, Any]]:
    """Collects episode containers inside a single matched series directory and its season subfolders."""
    if not sdir.get("is_dir", True):
        return [sdir]
    series_path = str(sdir.get("path") or sdir.get("name") or "")
    _, level1 = _list_synology_subdir(ctx, "synology-tvshow", series_path)
    episodes = [it for it in level1 if not it.get("is_dir")]
    season_dirs = [it for it in level1 if it.get("is_dir")]
    for season in season_dirs[:MAX_SERIES_SUBDIR_SCANS]:
        season_path = str(season.get("path") or f"{series_path}/{season.get('name')}")
        _, level2 = _list_synology_subdir(ctx, "synology-tvshow", season_path)
        episodes.extend(it for it in level2 if not it.get("is_dir"))
    return episodes


def _search_synology_tvshow(ctx: MediaSearchContext, query: str) -> tuple[str, int, list[dict[str, Any]]]:
    """Resolves hierarchical /tvshow/<Series>[/<Season>]/<Episode> containers matching query."""
    via, root_dirs = _list_synology_subdir(ctx, "synology-tvshow", "")
    if not root_dirs:
        return via, 0, []

    series_q = _extract_series_folder_query(query)
    _, matched_series = filter_and_rank_entries(root_dirs, series_q, ("name", "path"), limit=MAX_SERIES_SUBDIR_SCANS)

    candidate_episodes: list[dict[str, Any]] = []
    for sdir in matched_series:
        candidate_episodes.extend(_collect_series_episodes(ctx, sdir))

    total_cnt, ranked = filter_and_rank_entries(
        candidate_episodes,
        query,
        ("name", "path"),
        limit=MAX_MEDIA_QUERY_MATCHES,
    )
    if total_cnt == 0 and candidate_episodes:
        sorted_eps = sorted(candidate_episodes, key=lambda x: str(x.get("path") or x.get("name") or "").lower())
        return via, len(sorted_eps), sorted_eps[:MAX_MEDIA_QUERY_MATCHES]
    return via, total_cnt, ranked


def search_media_containers(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    family_id: str,
    query: str,
) -> dict[str, Any]:
    """Searches physical video containers across Freebox (3 disks) and Synology (video/tvshow)."""
    if status_payload.get("network", {}).get("simulated"):
        return {
            "query": query,
            "simulated": True,
            "matches_count": 0,
            "matches": [],
            "note": "Live network/CIFS container listing disabled under --simulate-profile.",
        }

    ctx = MediaSearchContext(
        ws=ws,
        endpoints=resolve_runtime_endpoints(ws),
        status_payload=status_payload,
        cross=build_cross_artifact_index(ws.effective_data_dir),
    )

    if family_id == "freebox-disks":
        via, raw_pool = _collect_freebox_items(ctx, query)
        total_cnt, raw_hits = filter_and_rank_entries(
            raw_pool,
            query,
            ("filename", "name", "path"),
            limit=MAX_MEDIA_QUERY_MATCHES,
        )
    elif family_id == "synology-video":
        via, total_cnt, raw_hits = _search_synology_video(ctx, query)
    else:
        via, total_cnt, raw_hits = _search_synology_tvshow(ctx, query)

    formatted = [_format_container_hit(ctx, family_id, via, item) for item in raw_hits]
    return {
        "query": query,
        "via_channel": via,
        "matches_count": total_cnt,
        "matches": formatted,
    }


def _strip_known_mount_prefix(ctx: MediaSearchContext, family_id: str, raw: str) -> str:
    """Strips local mount or snippet prefixes so agents can pass either full or relative paths."""
    cleaned = raw.strip()
    media_root = Path(ctx.endpoints.local_media_root)
    prefixes = {
        "freebox-disks": str(media_root / "Freebox") + "/",
        "synology-video": str(media_root / "Synology" / "video") + "/",
        "synology-tvshow": str(media_root / "Synology" / "tvshow") + "/",
        "local-snippets": str(ctx.ws.effective_data_dir / "snippets") + "/",
    }
    pref = prefixes.get(family_id, "")
    if pref and cleaned.startswith(pref):
        cleaned = cleaned[len(pref) :]
    return cleaned.strip("/")


PRIORITY_SUB_LANGS = frozenset({"fr", "fre", "fra", "en", "eng"})


def _summarize_subtitle_streams(subs: list[dict[str, Any]]) -> dict[str, Any]:
    """Compacts large subtitle stream lists while keeping full language inventory and FR/EN tracks."""
    langs = sorted({str(s.get("language") or "und") for s in subs})
    ordered = sorted(
        subs,
        key=lambda s: (
            0 if str(s.get("language") or "").lower() in PRIORITY_SUB_LANGS or s.get("forced") else 1,
            int(s.get("index") or 0),
        ),
    )
    return {
        "subtitle_streams_count": len(subs),
        "subtitle_languages": ",".join(langs) if langs else None,
        "subtitle_streams": ordered[:MAX_INSPECT_SUBTITLE_STREAMS],
        "subtitle_streams_truncated": len(subs) > MAX_INSPECT_SUBTITLE_STREAMS,
    }


def _parse_local_ffprobe_json(probe_json: dict[str, Any]) -> dict[str, Any]:
    """Extracts compact stream metadata from raw ffprobe JSON output."""
    fmt = probe_json.get("format", {})
    dur = round(float(fmt.get("duration", 0.0) or 0.0), DURATION_ROUND_DECIMALS)
    size = int(fmt.get("size", 0) or 0)
    bitrate = int(fmt.get("bit_rate", 0) or 0)

    video: dict[str, Any] | None = None
    audios: list[dict[str, Any]] = []
    subs: list[dict[str, Any]] = []
    for st in probe_json.get("streams", []):
        ctype = st.get("codec_type")
        tags = st.get("tags") if isinstance(st.get("tags"), dict) else {}
        if ctype == "video" and video is None:
            video = {
                "codec": st.get("codec_name"),
                "profile": st.get("profile"),
                "width": st.get("width"),
                "height": st.get("height"),
                "fps": st.get("r_frame_rate"),
                "pix_fmt": st.get("pix_fmt"),
            }
        elif ctype == "audio":
            audios.append(
                {
                    "index": st.get("index"),
                    "codec": st.get("codec_name"),
                    "channels": st.get("channels"),
                    "language": tags.get("language", "und"),
                    "title": tags.get("title", ""),
                }
            )
        elif ctype == "subtitle":
            disp = st.get("disposition") if isinstance(st.get("disposition"), dict) else {}
            subs.append(
                {
                    "index": st.get("index"),
                    "codec": st.get("codec_name"),
                    "language": tags.get("language", "und"),
                    "title": tags.get("title", ""),
                    "forced": disp.get("forced", 0) == 1,
                }
            )
    return {
        "size_bytes": size,
        "duration_sec": dur,
        "format_name": fmt.get("format_name"),
        "bit_rate_bps": bitrate,
        "video": video,
        "audio_streams_count": len(audios),
        "audio_streams": audios,
        **_summarize_subtitle_streams(subs),
    }


def _resolve_freebox_direct_target(ctx: MediaSearchContext, rel_path: str) -> tuple[str, str]:
    """Resolves Freebox direct LAN HTTP Range URL or safe CIFS file path."""
    if ctx.status_payload.get("channels", {}).get("lan_http_freebox", {}).get("accessible"):
        quoted = "/".join(urllib.parse.quote(seg) for seg in rel_path.split("/"))
        return f"http://{ctx.endpoints.rpi4_lan_host}:{FREEBOX_GATEWAY_PORT}/{quoted}", "lan_http_ffprobe"
    if _is_cifs_accessible_and_safe(ctx.status_payload):
        cand = Path(ctx.endpoints.local_media_root) / "Freebox" / rel_path
        return (str(cand), "local_cifs_ffprobe") if cand.is_file() else ("", "")
    return "", ""


def _resolve_direct_ffprobe_target(ctx: MediaSearchContext, family_id: str, rel_path: str) -> tuple[str, str]:
    """Resolves (target_uri_or_path, via_label) for direct workstation/LAN ffprobe when safe."""
    if family_id == "local-snippets":
        cand = ctx.ws.effective_data_dir / "snippets" / rel_path
        return (str(cand), "local_fs_ffprobe") if cand.is_file() else ("", "")
    if family_id == "freebox-disks":
        return _resolve_freebox_direct_target(ctx, rel_path)
    if family_id in ("synology-video", "synology-tvshow") and _is_cifs_accessible_and_safe(ctx.status_payload):
        share = "video" if family_id == "synology-video" else "tvshow"
        cand = Path(ctx.endpoints.local_media_root) / "Synology" / share / rel_path
        return (str(cand), "local_cifs_ffprobe") if cand.is_file() else ("", "")
    return "", ""


def _try_direct_ffprobe(ctx: MediaSearchContext, family_id: str, rel_path: str) -> dict[str, Any] | None:
    """Runs local ffprobe on local snippets, safe mounted CIFS, or LAN HTTP Freebox Range URL."""
    ffprobe_bin = shutil.which("ffprobe")
    if not ffprobe_bin:
        return None
    target, via = _resolve_direct_ffprobe_target(ctx, family_id, rel_path)
    if not target:
        return None
    cmd = [ffprobe_bin, "-v", "error", "-print_format", "json", "-show_format", "-show_streams", target]
    rc, out, _ = run_bounded_cmd(cmd, LOCAL_FFPROBE_TIMEOUT_SECS)
    if rc != 0 or not out:
        return None
    try:
        parsed = _parse_local_ffprobe_json(json.loads(out))
        parsed["via"] = via
        return parsed
    except ValueError:
        return None


def _try_o2switch_remote_inspect(ctx: MediaSearchContext, family_id: str, rel_path: str) -> dict[str, Any] | None:
    """Delegates ffprobe stream inspection to o2switch /api/file/details over HTTPS :443."""
    if not ctx.status_payload.get("channels", {}).get("https_o2switch", {}).get("accessible"):
        return None
    api_key = resolve_o2switch_api_key(ctx.ws)
    if not api_key:
        return None
    profile_map = {
        "freebox-disks": "wan_http",
        "synology-video": "wan_ftp",
        "synology-tvshow": "wan_ftp",
        "local-snippets": "local",
    }
    prof = profile_map.get(family_id, "wan_ftp")
    params = urllib.parse.urlencode({"source": family_id, "profile": prof, "path": rel_path})
    url = f"{ctx.endpoints.o2switch_url}/api/file/details?{params}"
    payload = _http_get_json(url, HTTPS_INSPECT_TIMEOUT_SECS, api_key=api_key)
    if not isinstance(payload, dict) or not payload.get("probed"):
        err = payload.get("reason") if isinstance(payload, dict) else "https_inspect_failed"
        return {"error": str(err or "remote_ffprobe_failed")}

    audios = payload.get("audio_streams", [])
    subs = payload.get("subtitle_streams", [])
    return {
        "via": "o2switch_https_api",
        "size_bytes": int(payload.get("size_bytes", 0) or 0),
        "duration_sec": round(float(payload.get("duration_seconds", 0.0) or 0.0), DURATION_ROUND_DECIMALS),
        "format_name": payload.get("format_name"),
        "bit_rate_bps": int(payload.get("bit_rate", 0) or 0),
        "video": payload.get("video"),
        "audio_streams_count": len(audios),
        "audio_streams": audios,
        **_summarize_subtitle_streams(subs),
    }


def _needs_path_autoresolve(family_id: str, clean_path: str) -> bool:
    """True when caller passed a title/substring query or bare filename missing required directory prefix."""
    has_video_ext = Path(clean_path).suffix.lower() in SUPPORTED_VIDEO_EXTS
    if not has_video_ext:
        return True
    if family_id == "freebox-disks" and "/" not in clean_path:
        return True
    if family_id == "synology-tvshow" and "/" not in clean_path:
        return True
    return False


def _probe_with_folder_fallback(
    ctx: MediaSearchContext,
    family_id: str,
    clean_path: str,
) -> tuple[str, dict[str, Any] | None]:
    """Runs direct or remote ffprobe, retrying inside movie subfolder when bare /video filename was passed."""
    probed = _try_direct_ffprobe(ctx, family_id, clean_path) or _try_o2switch_remote_inspect(ctx, family_id, clean_path)
    if (probed is None or "error" in probed) and family_id == "synology-video" and "/" not in clean_path:
        fallback_search = search_media_containers(ctx.ws, ctx.status_payload, family_id, clean_path)
        fb_hits = fallback_search.get("matches", [])
        if fb_hits and str(fb_hits[0]["path"]) != clean_path:
            clean_path = str(fb_hits[0]["path"])
            probed = _try_direct_ffprobe(ctx, family_id, clean_path) or _try_o2switch_remote_inspect(
                ctx, family_id, clean_path
            )
    return clean_path, probed


def inspect_media_container(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    family_id: str,
    path_or_query: str,
) -> dict[str, Any]:
    """Inspects ffprobe streams and cross-artifacts of a local, NAS, or Freebox container."""
    supported = ("freebox-disks", "synology-video", "synology-tvshow", "local-snippets")
    if family_id not in supported:
        return {"ok": False, "error": f"inspect_unsupported_for_family:{family_id}", "supported": list(supported)}
    if status_payload.get("network", {}).get("simulated"):
        return {"ok": False, "simulated": True, "error": "inspect_disabled_in_simulated_mode"}

    endpoints = resolve_runtime_endpoints(ws)
    cross = build_cross_artifact_index(ws.effective_data_dir)
    ctx = MediaSearchContext(ws=ws, endpoints=endpoints, status_payload=status_payload, cross=cross)

    clean_path = _strip_known_mount_prefix(ctx, family_id, path_or_query)
    other_matches = 0
    if family_id != "local-snippets" and _needs_path_autoresolve(family_id, clean_path):
        search_res = search_media_containers(ws, status_payload, family_id, clean_path)
        hits = search_res.get("matches", [])
        if not hits:
            return {"ok": False, "family": family_id, "query": path_or_query, "error": "container_not_found"}
        clean_path = str(hits[0]["path"])
        other_matches = max(0, int(search_res.get("matches_count", 1)) - 1)

    clean_path, probed = _probe_with_folder_fallback(ctx, family_id, clean_path)
    if probed is None or "error" in probed:
        err_msg = probed.get("error", "no_reachable_ffprobe_channel") if isinstance(probed, dict) else "unreachable"
        return {"ok": False, "family": family_id, "path": clean_path, "error": err_msg}

    fname = Path(clean_path).name
    result: dict[str, Any] = {
        "ok": True,
        "family": family_id,
        "path": clean_path,
        "filename": fname,
        **probed,
        "artifacts": lookup_cross_artifacts(cross, fname),
    }
    if other_matches > 0:
        result["other_matches_count"] = other_matches
    return result
