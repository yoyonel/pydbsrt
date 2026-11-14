"""Cached sidecar JSON indexer (.ahash & .phash), accent-insensitive matcher, and cross-artifact lookup."""

from __future__ import annotations

import json
import os
import re
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from scripts.data_access.constants import (
    AGENT_HTTP_USER_AGENT,
    AHASH_SIDECAR_CACHE_FILENAME,
    CACHE_DIR_MODE,
    CONTAINER_FPS_SUFFIXES,
    COUPLINGS_INDEX_CACHE_FILENAME,
    DURATION_ROUND_DECIMALS,
    HTTP_READ_MAX_BYTES,
    HTTP_STATUS_OK,
    HTTPS_API_TIMEOUT_SECS,
    MAX_REMOTE_SIDECAR_FETCHES,
    MAX_RESOLVE_MATCHES,
    MOVIEHASH_HEX_LEN,
    PHASH_SIDECAR_CACHE_FILENAME,
    SNIPPET_STEM_SUFFIXES,
)
from scripts.data_access.prober import get_cache_file_path
from scripts.data_access.topology import (
    WorkspaceContext,
    resolve_o2switch_api_key,
    resolve_runtime_endpoints,
)

NON_ALNUM_TOKEN_RE = re.compile(r"[^a-z0-9]+")
HEX_CHARS = frozenset("0123456789abcdef")
FPS_STEM_RE = re.compile(r"\.\d+(?:\.\d+)?fps$", re.IGNORECASE)
LIGATURE_MAP = str.maketrans({"œ": "oe", "Œ": "oe", "æ": "ae", "Æ": "ae"})


@dataclass(frozen=True)
class CrossArtifactIndex:
    """In-memory O(1) lookup tables mapping normalized media stems and moviehashes to artifacts."""

    ahash_by_stem: dict[str, dict[str, Any]]
    ahash_by_hash: dict[str, dict[str, Any]]
    phash_by_stem: dict[str, dict[str, Any]]
    phash_by_hash: dict[str, dict[str, Any]]
    couplings_by_stem: dict[str, dict[str, Any]]


def fold_search_text(text: str) -> str:
    """Normalizes Unicode text to accent-free lowercase ASCII/alphanumeric form."""
    replaced = text.translate(LIGATURE_MAP)
    decomposed = unicodedata.normalize("NFD", replaced)
    stripped = "".join(ch for ch in decomposed if unicodedata.category(ch) != "Mn")
    return stripped.lower()


def tokenize_search_text(text: str) -> list[str]:
    """Splits folded text into non-empty alphanumeric search tokens."""
    return [tok for tok in NON_ALNUM_TOKEN_RE.split(fold_search_text(text)) if tok]


def normalize_media_key(raw_name: str) -> str:
    """Produces an accent-folded, extension-stripped canonical release key for cross-referencing."""
    decoded = urllib.parse.unquote(raw_name).strip()
    for ext in (".mp4", ".mkv", ".avi", ".mov", ".m4v", ".webm", ".ts", ".phash", ".ahash", ".json", ".srt"):
        if decoded.lower().endswith(ext):
            decoded = decoded[: -len(ext)]
    for fps_suf in CONTAINER_FPS_SUFFIXES:
        if decoded.lower().endswith(fps_suf):
            decoded = decoded[: -len(fps_suf)]
    decoded = FPS_STEM_RE.sub("", decoded)
    for snip_suf in SNIPPET_STEM_SUFFIXES:
        if decoded.lower().endswith(snip_suf):
            decoded = decoded[: -len(snip_suf)]
    return " ".join(tokenize_search_text(decoded))


def matches_folded_query(haystack: str, query: str) -> tuple[bool, bool]:
    """Returns (substring_match, token_prefix_match) for accent-folded multi-token queries."""
    q_tokens = tokenize_search_text(query)
    if not q_tokens:
        q_fold = fold_search_text(query).strip()
        return (q_fold in fold_search_text(haystack), False) if q_fold else (False, False)
    h_fold = fold_search_text(haystack)
    if not all(qt in h_fold for qt in q_tokens):
        return False, False
    h_tokens = tokenize_search_text(haystack)
    prefix_ok = all(any(ht.startswith(qt) for ht in h_tokens) for qt in q_tokens)
    return True, prefix_ok


def filter_and_rank_entries(
    entries: list[dict[str, Any]],
    query: str,
    text_fields: tuple[str, ...],
    limit: int = MAX_RESOLVE_MATCHES,
) -> tuple[int, list[dict[str, Any]]]:
    """Filters and ranks index entries preferring token-prefix hits over inner-substring hits."""
    prefix_hits: list[dict[str, Any]] = []
    substr_hits: list[dict[str, Any]] = []
    for entry in entries:
        haystack = " ".join(str(entry.get(f) or "") for f in text_fields)
        sub_ok, pref_ok = matches_folded_query(haystack, query)
        if pref_ok:
            prefix_hits.append(entry)
        elif sub_ok:
            substr_hits.append(entry)
    ranked = prefix_hits if prefix_hits else substr_hits
    return len(ranked), ranked[:limit]


def _sidecar_cache_path(filename: str) -> Path:
    """Resolves cache path colocated with the active DBSRT data access cache directory."""
    return get_cache_file_path().parent / filename


def _read_indexed_cache(cache_path: Path, source_key: str) -> list[dict[str, Any]] | None:
    """Loads cached sidecar index list when directory mtime and file counts match."""
    if not cache_path.is_file():
        return None
    try:
        payload = json.loads(cache_path.read_text(encoding="utf-8"))
        if payload.get("source_key") == source_key and isinstance(payload.get("entries"), list):
            return payload["entries"]
    except (OSError, ValueError, TypeError):
        return None
    return None


def _write_indexed_cache(cache_path: Path, source_key: str, entries: list[dict[str, Any]]) -> None:
    """Persists compact sidecar index JSON atomically."""
    try:
        cache_path.parent.mkdir(parents=True, exist_ok=True, mode=CACHE_DIR_MODE)
        tmp_path = cache_path.with_suffix(".tmp")
        tmp_path.write_text(
            json.dumps({"source_key": source_key, "entries": entries}, ensure_ascii=False),
            encoding="utf-8",
        )
        tmp_path.replace(cache_path)
    except OSError:
        pass


def _parse_ahash_sidecar(
    json_path: Path,
    ahash_by_nfc: dict[str, tuple[str, int]],
    covered_nfc: set[str],
) -> dict[str, Any] | None:
    """Parses a single AhashSidecar v1 JSON file and links its binary .ahash target."""
    try:
        meta = json.loads(json_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(meta, dict):
        return None

    mhash = meta.get("moviehash")
    raw_ahash = str(meta.get("ahash_file") or "")
    nfc_key = unicodedata.normalize("NFC", raw_ahash) if raw_ahash else ""
    hit = ahash_by_nfc.get(nfc_key)
    if hit is None and isinstance(mhash, str) and mhash:
        fallback_key = unicodedata.normalize("NFC", f"{mhash}.86.1fps.ahash")
        hit = ahash_by_nfc.get(fallback_key)
        if hit is not None:
            nfc_key = fallback_key

    actual_ahash = hit[0] if hit else raw_ahash
    size_bytes = hit[1] if hit else 0
    if nfc_key:
        covered_nfc.add(nfc_key)

    dur_raw = meta.get("duration_sec")
    dur_val = round(float(dur_raw), DURATION_ROUND_DECIMALS) if isinstance(dur_raw, int | float) else None
    raw_stream = meta.get("selected_audio_stream")
    stream: dict[str, Any] = raw_stream if isinstance(raw_stream, dict) else {}
    return {
        "ahash_file": actual_ahash,
        "sidecar_file": json_path.name,
        "moviehash": mhash if isinstance(mhash, str) else None,
        "media_name": str(meta.get("media_name") or json_path.stem),
        "duration_sec": dur_val,
        "frames": meta.get("frames"),
        "audio_codec": stream.get("codec_name"),
        "audio_lang": stream.get("language"),
        "size_bytes": size_bytes,
    }


def load_ahash_sidecar_index(data_dir: Path) -> list[dict[str, Any]]:
    """Builds or loads O(1)-cached AhashSidecar + orphan .ahash index for data/__AHASH__/."""
    ahash_dir = data_dir / "__AHASH__"
    if not ahash_dir.is_dir():
        return []

    ahash_by_nfc: dict[str, tuple[str, int]] = {}
    json_files: list[Path] = []
    for entry in os.scandir(ahash_dir):
        if entry.name.endswith(".ahash"):
            nfc_name = unicodedata.normalize("NFC", entry.name)
            ahash_by_nfc[nfc_name] = (entry.name, entry.stat().st_size)
        elif entry.name.endswith(".json"):
            json_files.append(Path(entry.path))

    dir_mtime = ahash_dir.stat().st_mtime_ns
    source_key = f"{ahash_dir.resolve()}:{len(ahash_by_nfc)}:{len(json_files)}:{dir_mtime}"
    cache_path = _sidecar_cache_path(AHASH_SIDECAR_CACHE_FILENAME)
    cached = _read_indexed_cache(cache_path, source_key)
    if cached is not None:
        return cached

    covered_nfc: set[str] = set()
    entries: list[dict[str, Any]] = []
    for jpath in sorted(json_files, key=lambda p: p.name):
        parsed = _parse_ahash_sidecar(jpath, ahash_by_nfc, covered_nfc)
        if parsed is not None:
            entries.append(parsed)

    for nfc_name, (disk_name, sz) in sorted(ahash_by_nfc.items()):
        if nfc_name in covered_nfc:
            continue
        stem_prefix = disk_name.split(".", 1)[0].lower()
        mhash = stem_prefix if len(stem_prefix) == MOVIEHASH_HEX_LEN and set(stem_prefix) <= HEX_CHARS else None
        entries.append(
            {
                "ahash_file": disk_name,
                "sidecar_file": None,
                "moviehash": mhash,
                "media_name": urllib.parse.unquote(disk_name),
                "duration_sec": None,
                "frames": None,
                "audio_codec": None,
                "audio_lang": None,
                "size_bytes": sz,
            }
        )

    _write_indexed_cache(cache_path, source_key, entries)
    return entries


def _parse_phash_sidecar(
    json_path: Path,
    phash_by_hash: dict[str, list[tuple[str, int]]],
    covered_names: set[str],
) -> dict[str, Any] | None:
    """Parses a single PhashSidecar JSON file and links its binary .phash file."""
    try:
        meta = json.loads(json_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(meta, dict):
        return None

    mhash = str(meta.get("moviehash") or json_path.stem).lower()
    candidates = phash_by_hash.get(mhash, [])
    phash_name = candidates[0][0] if candidates else f"{mhash}.{meta.get('fps', 24.0)}fps.phash"
    size_bytes = candidates[0][1] if candidates else 0
    for cname, _ in candidates:
        covered_names.add(cname)

    return {
        "phash_file": phash_name,
        "sidecar_file": json_path.name,
        "moviehash": mhash if len(mhash) == MOVIEHASH_HEX_LEN else None,
        "media_name": str(meta.get("media_name") or json_path.stem),
        "fps": meta.get("fps"),
        "frames": meta.get("frames"),
        "size_bytes": size_bytes,
    }


def _scan_phash_directory(phash_dir: Path) -> tuple[dict[str, int], dict[str, list[tuple[str, int]]], list[Path]]:
    """Scans data/__PHASH__ once into (.phash sizes, moviehash->files map, .json sidecar paths)."""
    phash_files: dict[str, int] = {}
    phash_by_hash: dict[str, list[tuple[str, int]]] = {}
    json_files: list[Path] = []
    for entry in os.scandir(phash_dir):
        if entry.name.endswith(".phash"):
            sz = entry.stat().st_size
            phash_files[entry.name] = sz
            prefix = entry.name.split(".", 1)[0].lower()
            if len(prefix) == MOVIEHASH_HEX_LEN and set(prefix) <= HEX_CHARS:
                phash_by_hash.setdefault(prefix, []).append((entry.name, sz))
        elif entry.name.endswith(".json"):
            json_files.append(Path(entry.path))
    return phash_files, phash_by_hash, json_files


def load_phash_sidecar_index(data_dir: Path) -> list[dict[str, Any]]:
    """Builds or loads O(1)-cached PhashSidecar + URL-decoded .phash index for data/__PHASH__/."""
    phash_dir = data_dir / "__PHASH__"
    if not phash_dir.is_dir():
        return []

    phash_files, phash_by_hash, json_files = _scan_phash_directory(phash_dir)
    dir_mtime = phash_dir.stat().st_mtime_ns
    source_key = f"{phash_dir.resolve()}:{len(phash_files)}:{len(json_files)}:{dir_mtime}"
    cache_path = _sidecar_cache_path(PHASH_SIDECAR_CACHE_FILENAME)
    cached = _read_indexed_cache(cache_path, source_key)
    if cached is not None:
        return cached

    covered_names: set[str] = set()
    entries: list[dict[str, Any]] = []
    for jpath in sorted(json_files, key=lambda p: p.name):
        parsed = _parse_phash_sidecar(jpath, phash_by_hash, covered_names)
        if parsed is not None:
            entries.append(parsed)

    for disk_name, sz in sorted(phash_files.items()):
        if disk_name in covered_names:
            continue
        prefix = disk_name.split(".", 1)[0].lower()
        mhash = prefix if len(prefix) == MOVIEHASH_HEX_LEN and set(prefix) <= HEX_CHARS else None
        entries.append(
            {
                "phash_file": disk_name,
                "sidecar_file": None,
                "moviehash": mhash,
                "media_name": urllib.parse.unquote(disk_name),
                "fps": None,
                "frames": None,
                "size_bytes": sz,
            }
        )

    _write_indexed_cache(cache_path, source_key, entries)
    return entries


def _fetch_remote_json(url: str, api_key: str) -> dict[str, Any] | None:
    """Fetches a small JSON document over authenticated HTTPS from o2switch."""
    headers = {"Accept": "application/json", "User-Agent": AGENT_HTTP_USER_AGENT}
    if api_key:
        headers["X-API-Key"] = api_key
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=HTTPS_API_TIMEOUT_SECS) as resp:
            if resp.status != HTTP_STATUS_OK:
                return None
            return json.loads(resp.read(HTTP_READ_MAX_BYTES * 8).decode("utf-8", errors="replace"))
    except (urllib.error.URLError, OSError, ValueError):
        return None


def _build_remote_hash_hit(
    kind: str,
    mhash: str,
    asset: dict[str, Any],
    side_meta: dict[str, Any] | None,
) -> dict[str, Any] | None:
    """Formats a remote o2switch ahash/phash fallback item from /api/assets + sidecar metadata."""
    if kind == "ahash" and isinstance(side_meta, dict):
        ah_name = str(side_meta.get("ahash_file") or f"{mhash}.86.1fps.ahash")
        return {
            "ahash_file": ah_name,
            "sidecar_file": f"{mhash}.json",
            "moviehash": mhash,
            "media_name": str(side_meta.get("media_name") or asset.get("name") or mhash),
            "duration_sec": side_meta.get("duration_sec"),
            "frames": side_meta.get("frames"),
            "location": "o2switch_remote_only",
            "fetch_cmd": f"python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py fetch ahash {ah_name}",
        }
    if kind == "phash" and asset.get("phash_status") == "complete":
        fps = asset.get("fps") or 24.0
        ph_name = f"{mhash}.{fps:.1f}fps.phash" if isinstance(fps, int | float) else f"{mhash}.phash"
        return {
            "phash_file": ph_name,
            "sidecar_file": f"{mhash}.json",
            "moviehash": mhash,
            "media_name": str(asset.get("name") or mhash),
            "frames": asset.get("frame_count"),
            "location": "o2switch_remote_only",
            "fetch_cmd": f"python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py fetch phash {ph_name}",
        }
    return None


def _search_remote_o2switch_hashes(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    query: str,
    kind: str,
) -> list[dict[str, Any]]:
    """Discovers remote .ahash or .phash entries via o2switch /api/assets when missing locally."""
    net_sim = bool(status_payload.get("network", {}).get("simulated"))
    https_up = bool(status_payload.get("channels", {}).get("https_o2switch", {}).get("accessible"))
    api_key = resolve_o2switch_api_key(ws) if (not net_sim and https_up) else ""
    if not api_key:
        return []

    endpoints = resolve_runtime_endpoints(ws)
    enc_q = urllib.parse.quote(query, safe="")
    assets_url = f"{endpoints.o2switch_url}/api/assets?q={enc_q}&limit={MAX_REMOTE_SIDECAR_FETCHES}"
    payload = _fetch_remote_json(assets_url, api_key)
    items = payload.get("items", []) if isinstance(payload, dict) else []

    remote_hits: list[dict[str, Any]] = []
    for asset in items[:MAX_REMOTE_SIDECAR_FETCHES]:
        mhash = str(asset.get("moviehash") or "").lower()
        if len(mhash) != MOVIEHASH_HEX_LEN:
            continue
        side_url = f"{endpoints.o2switch_url}/api/artifact/download?kind={kind}&file={mhash}.json"
        formatted = _build_remote_hash_hit(kind, mhash, asset, _fetch_remote_json(side_url, api_key))
        if formatted is not None:
            remote_hits.append(formatted)
    return remote_hits


def _strip_dot_release_stem(media_name: str) -> str:
    """Strips container, FPS, and snippet suffixes while preserving dot/hyphen separators."""
    dot_stem = media_name.strip()
    for ext in (".mp4", ".mkv", ".avi", ".phash", ".ahash", ".json", ".srt"):
        if dot_stem.lower().endswith(ext):
            dot_stem = dot_stem[: -len(ext)]
    for fps_suf in CONTAINER_FPS_SUFFIXES:
        if dot_stem.lower().endswith(fps_suf):
            dot_stem = dot_stem[: -len(fps_suf)]
    for snip_suf in SNIPPET_STEM_SUFFIXES:
        if dot_stem.lower().endswith(snip_suf):
            dot_stem = dot_stem[: -len(snip_suf)]
    return dot_stem.lower()


def _try_read_couplings_index_cache(cache_path: Path, source_key: str) -> tuple[dict[str, dict[str, Any]], int] | None:
    """Reads cached couplings.json summary dictionary when mtime+size match."""
    if not cache_path.is_file():
        return None
    try:
        cached = json.loads(cache_path.read_text(encoding="utf-8"))
        if cached.get("source_key") == source_key and isinstance(cached.get("by_norm"), dict):
            return cached["by_norm"], int(cached.get("total_entries", 0))
    except (OSError, ValueError, TypeError):
        return None
    return None


def load_couplings_index(data_dir: Path) -> tuple[dict[str, dict[str, Any]], int]:
    """Indexes certified couplings.json by normalized release stem with O(1) mtime+size cache."""
    coup_path = data_dir / "SRT" / "couplings.json"
    if not coup_path.is_file():
        return {}, 0
    st = coup_path.stat()
    source_key = f"{coup_path.resolve()}:{st.st_size}:{st.st_mtime_ns}"
    cache_path = _sidecar_cache_path(COUPLINGS_INDEX_CACHE_FILENAME)
    hit = _try_read_couplings_index_cache(cache_path, source_key)
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
        by_norm[normalize_media_key(media_name)] = summary
        by_norm[_strip_dot_release_stem(media_name)] = summary
    try:
        cache_path.parent.mkdir(parents=True, exist_ok=True, mode=CACHE_DIR_MODE)
        cache_path.write_text(
            json.dumps(
                {"source_key": source_key, "total_entries": len(entries), "by_norm": by_norm}, ensure_ascii=False
            ),
            encoding="utf-8",
        )
    except OSError:
        pass
    return by_norm, len(entries)


def _index_sidecar_maps(items: list[dict[str, Any]]) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    """Indexes sidecar entries into (by_normalized_stem, by_moviehash) maps."""
    by_stem: dict[str, dict[str, Any]] = {}
    by_hash: dict[str, dict[str, Any]] = {}
    for item in items:
        key = normalize_media_key(str(item.get("media_name") or ""))
        if key and (key not in by_stem or item.get("moviehash")):
            by_stem[key] = item
        if item.get("moviehash"):
            by_hash[str(item["moviehash"]).lower()] = item
    return by_stem, by_hash


def build_cross_artifact_index(data_dir: Path) -> CrossArtifactIndex:
    """Builds unified O(1) cross-reference tables across .ahash, .phash, and couplings.json."""
    couplings_by_norm, _ = load_couplings_index(data_dir)
    ah_stem, ah_hash = _index_sidecar_maps(load_ahash_sidecar_index(data_dir))
    ph_stem, ph_hash = _index_sidecar_maps(load_phash_sidecar_index(data_dir))

    coup_stem: dict[str, dict[str, Any]] = {}
    for summary in couplings_by_norm.values():
        key = normalize_media_key(str(summary.get("media_name") or ""))
        if key:
            coup_stem[key] = summary

    return CrossArtifactIndex(
        ahash_by_stem=ah_stem,
        ahash_by_hash=ah_hash,
        phash_by_stem=ph_stem,
        phash_by_hash=ph_hash,
        couplings_by_stem=coup_stem,
    )


def lookup_cross_artifacts(
    cross: CrossArtifactIndex,
    media_name: str,
    moviehash: str | None = None,
) -> dict[str, Any]:
    """Resolves matching .phash, .ahash, and certified couplings for any media filename."""
    key = normalize_media_key(media_name)
    mhash = moviehash.lower() if isinstance(moviehash, str) and moviehash else None

    ph_hit = (cross.phash_by_hash.get(mhash) if mhash else None) or cross.phash_by_stem.get(key)
    if not mhash and ph_hit and ph_hit.get("moviehash"):
        mhash = str(ph_hit["moviehash"]).lower()

    ah_hit = (cross.ahash_by_hash.get(mhash) if mhash else None) or cross.ahash_by_stem.get(key)
    if not mhash and ah_hit and ah_hit.get("moviehash"):
        mhash = str(ah_hit["moviehash"]).lower()

    coup_hit = cross.couplings_by_stem.get(key)
    langs = coup_hit.get("languages", []) if coup_hit else []
    lang_str = ",".join(langs) if isinstance(langs, list) else str(langs)

    return {
        "moviehash": mhash,
        "phash_file": ph_hit["phash_file"] if ph_hit else None,
        "ahash_file": ah_hit["ahash_file"] if ah_hit else None,
        "coupled_srt": coup_hit is not None,
        "coupled_subtitles_count": int(coup_hit["certified_subtitles_count"]) if coup_hit else 0,
        "coupled_languages": lang_str if lang_str else None,
    }


def search_ahash_corpus(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    query: str,
) -> dict[str, Any]:
    """Searches .ahash corpus by human media_name, hex moviehash, or filename with HTTPS fallback."""
    data_dir = ws.effective_data_dir
    entries = load_ahash_sidecar_index(data_dir)
    total_cnt, matched = filter_and_rank_entries(
        entries,
        query,
        ("media_name", "moviehash", "ahash_file", "sidecar_file"),
        limit=MAX_RESOLVE_MATCHES,
    )
    cross = build_cross_artifact_index(data_dir)
    ahash_dir = data_dir / "__AHASH__"

    items: list[dict[str, Any]] = []
    for entry in matched:
        ah_file = str(entry.get("ahash_file") or "")
        art = lookup_cross_artifacts(cross, str(entry.get("media_name") or ah_file), entry.get("moviehash"))
        items.append(
            {
                "ahash_file": ah_file,
                "path": str(ahash_dir / ah_file) if ah_file else None,
                "sidecar_file": entry.get("sidecar_file"),
                "moviehash": entry.get("moviehash"),
                "media_name": entry.get("media_name"),
                "duration_sec": entry.get("duration_sec"),
                "frames": entry.get("frames"),
                "audio_codec": entry.get("audio_codec"),
                "audio_lang": entry.get("audio_lang"),
                "size_bytes": entry.get("size_bytes"),
                "phash_file": art["phash_file"],
                "coupled_srt": art["coupled_srt"],
            }
        )

    if not items:
        remote_items = _search_remote_o2switch_hashes(ws, status_payload, query, "ahash")
        if remote_items:
            return {
                "query": query,
                "indexed_sidecars_local": len(entries),
                "matches_count": len(remote_items),
                "source": "o2switch_https_fallback",
                "matches": remote_items,
            }

    return {
        "query": query,
        "indexed_sidecars_local": len(entries),
        "matches_count": total_cnt,
        "matches": items,
    }


def search_phash_corpus(
    ws: WorkspaceContext,
    status_payload: dict[str, Any],
    query: str,
) -> dict[str, Any]:
    """Searches .phash corpus by human media_name (sidecar/URL-decoded), hex moviehash, or filename."""
    data_dir = ws.effective_data_dir
    entries = load_phash_sidecar_index(data_dir)
    total_cnt, matched = filter_and_rank_entries(
        entries,
        query,
        ("media_name", "moviehash", "phash_file", "sidecar_file"),
        limit=MAX_RESOLVE_MATCHES,
    )
    cross = build_cross_artifact_index(data_dir)
    phash_dir = data_dir / "__PHASH__"

    items: list[dict[str, Any]] = []
    for entry in matched:
        ph_file = str(entry.get("phash_file") or "")
        art = lookup_cross_artifacts(cross, str(entry.get("media_name") or ph_file), entry.get("moviehash"))
        items.append(
            {
                "filename": ph_file,
                "path": str(phash_dir / ph_file) if ph_file else None,
                "sidecar_file": entry.get("sidecar_file"),
                "moviehash": entry.get("moviehash"),
                "media_name": entry.get("media_name"),
                "fps": entry.get("fps"),
                "frames": entry.get("frames"),
                "size_bytes": entry.get("size_bytes"),
                "ahash_file": art["ahash_file"],
                "coupled_srt": art["coupled_srt"],
            }
        )

    if not items:
        remote_items = _search_remote_o2switch_hashes(ws, status_payload, query, "phash")
        if remote_items:
            return {
                "query": query,
                "indexed_entries_local": len(entries),
                "matches_count": len(remote_items),
                "source": "o2switch_https_fallback",
                "matches": remote_items,
            }

    return {
        "query": query,
        "indexed_entries_local": len(entries),
        "matches_count": total_cnt,
        "matches": items,
    }
