"""Tripartite pHash reconciliation service (RAW .phash, Zero-DB catalog.dbphash, PostgreSQL)."""

from __future__ import annotations

import json
import struct
import unicodedata
import urllib.parse
from pathlib import Path
from typing import Any

from pydbsrt.portal.cache import get_phashes_audit_cache, set_phashes_audit_cache
from pydbsrt.portal.config import (
    CATALOG_DBPHASH_ENTRY_FORMAT,
    CATALOG_DBPHASH_ENTRY_SIZE,
    CATALOG_DBPHASH_HEADER_FORMAT,
    CATALOG_DBPHASH_HEADER_SIZE,
    CATALOG_INDEX_ENTRY_BYTES,
    DEFAULT_BASELINE_FPS,
    HEX_ALPHABET,
    MAX_PAGE_SIZE,
    MILLIS_PER_SEC,
    MIN_SIDECAR_STEM_LENGTH,
    MOVIEHASH_HEX_LENGTH,
    PHASH_FRAME_BYTES,
    RE_TRAILING_FPS_TAG,
    _format_duration,
    _resolve_data_path,
    format_human_size,
    strip_accents,
)
from pydbsrt.portal.models import PhashesQueryCriteria, PhashReconciliationContext
from pydbsrt.portal.services.assets_service import _connect_pg, _query_pg_cli


def _load_phash_sidecars(phash_dir: Path) -> dict[str, dict[str, Any]]:
    """Loads all *.json sidecars in __PHASH__ directory into dictionary indexed by stem."""
    sidecars: dict[str, dict[str, Any]] = {}
    if not phash_dir.is_dir():
        return sidecars
    for p in phash_dir.glob("*.json"):
        try:
            sidecars[p.stem.lower()] = json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            pass
    return sidecars


def _read_catalog_dbphash(cat_path: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Reads fixed 128-byte header and media table entries from catalog.dbphash."""
    if not cat_path.is_file():
        return {}, []

    with open(cat_path, "rb") as f:
        hdr_raw = f.read(CATALOG_DBPHASH_HEADER_SIZE)
        if len(hdr_raw) < CATALOG_DBPHASH_HEADER_SIZE:
            return {}, []
        unpacked_hdr = struct.unpack(CATALOG_DBPHASH_HEADER_FORMAT, hdr_raw)
        (
            magic,
            ver,
            _flags,
            media_cnt,
            total_cuts,
            total_sparse,
            off_media,
            _off_cuts,
            _off_sparse,
            off_str,
            str_len,
            _csum,
            _,
        ) = unpacked_hdr

        if not magic.startswith(b"DBSRT"):
            return {}, []

        f.seek(off_str)
        string_table = f.read(str_len)

        f.seek(off_media)
        catalog_items: list[dict[str, Any]] = []
        for _ in range(media_cnt):
            entry_raw = f.read(CATALOG_DBPHASH_ENTRY_SIZE)
            if len(entry_raw) < CATALOG_DBPHASH_ENTRY_SIZE:
                break
            mid, _mtype, has_fc, yr, s, ep, dur, fps_x1000, toff, tlen, cc, _c_idx, sc, _s_idx, fc = struct.unpack(
                CATALOG_DBPHASH_ENTRY_FORMAT, entry_raw
            )
            title = string_table[toff : toff + tlen].decode("utf-8", errors="replace")
            catalog_items.append(
                {
                    "catalog_id": mid,
                    "title": title,
                    "year": yr if yr > 0 else None,
                    "season": s if s > 0 else None,
                    "episode": ep if ep > 0 else None,
                    "duration_ms": dur,
                    "fps": fps_x1000 / MILLIS_PER_SEC,
                    "cuts_count": cc,
                    "sparse_count": sc,
                    "frame_count": fc if has_fc else 0,
                }
            )

        header_info = {
            "version": ver,
            "media_count": media_cnt,
            "total_cuts": total_cuts,
            "total_sparse": total_sparse,
            "file_size": cat_path.stat().st_size,
        }
        return header_info, catalog_items


def _index_media_record(r: dict[str, Any], medias: dict[str, dict[str, Any]]) -> None:
    """Indexes single media record by moviehash, phash_path basename, and name."""
    mhash = r.get("moviehash")
    if mhash:
        medias[mhash.lower()] = r
    ppath = r.get("phash_path")
    if ppath:
        medias[Path(ppath).name.lower()] = r
    name = r.get("name")
    if name:
        medias[name.lower()] = r


def _query_pg_medias_psycopg2(conn: Any) -> tuple[dict[str, dict[str, Any]], int, int]:
    """Queries medias and frames using psycopg2 connection."""
    from psycopg2.extras import RealDictCursor

    medias: dict[str, dict[str, Any]] = {}
    total_pg_medias = 0
    frames_table_count = 0
    try:
        cur = conn.cursor(cursor_factory=RealDictCursor)
        cur.execute("SELECT count(*) AS total FROM medias;")
        total_pg_medias = cur.fetchone()["total"]

        try:
            cur.execute("SELECT count(*) AS fc FROM frames;")
            frames_table_count = cur.fetchone()["fc"]
        except Exception:
            pass

        cur.execute("SELECT id, name, clean_title, moviehash, phash_path, frame_count, fps, phash_status FROM medias;")
        for row in cur.fetchall():
            _index_media_record(dict(row), medias)
    except Exception:
        pass
    finally:
        conn.close()
    return medias, total_pg_medias, frames_table_count


def _query_pg_medias_cli() -> tuple[dict[str, dict[str, Any]], int, int]:
    """Queries medias and frames using psql CLI fallback."""
    medias: dict[str, dict[str, Any]] = {}
    total_pg_medias = 0
    frames_table_count = 0

    raw_cnt = _query_pg_cli("SELECT count(*) FROM medias;")
    if raw_cnt and raw_cnt.isdigit():
        total_pg_medias = int(raw_cnt)

    has_frames = _query_pg_cli(
        "SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'frames';"
    )
    if has_frames:
        raw_fc = _query_pg_cli("SELECT count(*) FROM frames;")
        if raw_fc and raw_fc.isdigit():
            frames_table_count = int(raw_fc)

    raw_json = _query_pg_cli(
        "SELECT json_agg(t)::text FROM (SELECT id, name, clean_title, moviehash, phash_path, frame_count, fps, phash_status FROM medias) t;"
    )
    if raw_json and raw_json != "null":
        try:
            for r in json.loads(raw_json):
                _index_media_record(r, medias)
        except Exception:
            pass
    return medias, total_pg_medias, frames_table_count


def _query_pg_medias_lookup() -> tuple[dict[str, dict[str, Any]], int, int]:
    """Loads media records and frame counts from PostgreSQL for tripartite alignment."""
    conn = _connect_pg()
    if conn is not None:
        return _query_pg_medias_psycopg2(conn)
    return _query_pg_medias_cli()


def _is_valid_moviehash_hex(candidate: str) -> bool:
    """Returns True when candidate is an exact 16-char hexadecimal OpenSubtitles moviehash."""
    return len(candidate) == MOVIEHASH_HEX_LENGTH and all(c in HEX_ALPHABET for c in candidate)


def _normalize_phash_media_stem(raw_name: str) -> str:
    """Normalizes a .phash or media filename to an NFC lowercased stem without .phash or .fps tags."""
    decoded = unicodedata.normalize("NFC", urllib.parse.unquote(raw_name)).strip().lower()
    if decoded.endswith(".phash"):
        decoded = decoded[: -len(".phash")]
    return RE_TRAILING_FPS_TAG.sub("", decoded)


def _build_sidecar_name_index(sidecars: dict[str, dict[str, Any]]) -> dict[str, str]:
    """Builds an O(1) exact stem -> moviehash lookup table without substring false-positives."""
    index: dict[str, str] = {}
    for key_stem, sc in sidecars.items():
        mhash_raw = str(sc.get("moviehash") or key_stem).strip().lower()
        if not _is_valid_moviehash_hex(mhash_raw):
            continue
        mname = str(sc.get("media_name") or "").strip()
        if not mname:
            continue
        norm_full = _normalize_phash_media_stem(mname)
        if norm_full:
            index.setdefault(norm_full, mhash_raw)
            bare_stem = Path(norm_full).stem
            if len(bare_stem) >= MIN_SIDECAR_STEM_LENGTH:
                index.setdefault(bare_stem, mhash_raw)
    return index


def _resolve_phash_moviehash(
    filename: str,
    sidecars: dict[str, dict[str, Any]],
    sidecar_name_index: dict[str, str] | None = None,
) -> str | None:
    """Extracts 16-hex moviehash from filename or exact normalized sidecar match."""
    stem = filename.split(".")[0]
    if _is_valid_moviehash_hex(stem):
        return stem.lower()
    lookup = sidecar_name_index if sidecar_name_index is not None else _build_sidecar_name_index(sidecars)
    norm_name = _normalize_phash_media_stem(filename)
    if not norm_name:
        return None
    return lookup.get(norm_name) or lookup.get(Path(norm_name).stem)


def _lookup_pg_record(mhash: str | None, raw_fname: str, pg_medias: dict[str, dict[str, Any]]) -> dict[str, Any] | None:
    """Finds matching PostgreSQL media record by moviehash or filename."""
    if mhash and mhash in pg_medias:
        return pg_medias[mhash]
    if raw_fname and raw_fname.lower() in pg_medias:
        return pg_medias[raw_fname.lower()]
    return None


def _compute_phash_integrity_status(has_raw: bool, has_catalog: bool, has_pg: bool) -> tuple[str, str]:
    """Computes integrity status code and human label based on tripartite presence."""
    if has_raw and has_catalog and has_pg:
        return "aligned_3_3", "Aligné 3/3"
    if has_raw and has_catalog:
        return "missing_pg", "Non ingéré PG"
    if has_raw:
        return "uncompiled", "À compiler"
    return "orphan_catalog", "Orphelin catalogue"


def _extract_raw_file_specs(raw_p: Path | None) -> tuple[str, int, int]:
    """Extracts filename, size in bytes, and frame count from optional raw Path."""
    if not raw_p:
        return "", 0, 0
    sz = raw_p.stat().st_size
    return raw_p.name, sz, sz // PHASH_FRAME_BYTES


def _compute_catalog_specs(
    cat_e: dict[str, Any] | None, raw_fc: int, default_title: str
) -> tuple[str, float, int, int, int, int]:
    """Extracts title, fps, frame count, duration in ms, cuts and sparse counts from catalog entry."""
    if not cat_e:
        return (
            default_title or "—",
            DEFAULT_BASELINE_FPS,
            raw_fc,
            int(raw_fc / DEFAULT_BASELINE_FPS * MILLIS_PER_SEC),
            0,
            0,
        )
    fps = cat_e["fps"]
    fc = cat_e["frame_count"] if cat_e["frame_count"] > 0 else raw_fc
    dur_ms = cat_e["duration_ms"] if cat_e["duration_ms"] > 0 else int(fc / fps * MILLIS_PER_SEC)
    return cat_e["title"], fps, fc, dur_ms, cat_e["cuts_count"], cat_e["sparse_count"]


def _calculate_compression_ratio(raw_size: int, cuts_c: int, sparse_c: int) -> str:
    """Calculates compression ratio multiplier between raw pHash frames and index entries."""
    indexed_bytes = (cuts_c + sparse_c) * CATALOG_INDEX_ENTRY_BYTES
    if indexed_bytes > 0 and raw_size > 0:
        return f"{raw_size / indexed_bytes:.1f}x"
    return "—"


def _build_reconciled_phash_item(
    raw_p: Path | None,
    cat_e: dict[str, Any] | None,
    idx: int,
    ctx: PhashReconciliationContext,
) -> dict[str, Any]:
    """Builds a single reconciled tripartite item descriptor from raw file, catalog entry, and PG context."""
    raw_fname, raw_size, raw_fc = _extract_raw_file_specs(raw_p)
    mhash = _resolve_phash_moviehash(raw_fname, ctx.sidecars, ctx.sidecar_name_index) if raw_fname else None
    pg_record = _lookup_pg_record(mhash, raw_fname, ctx.pg_medias)

    title, fps, fc, dur_ms, cuts_c, sparse_c = _compute_catalog_specs(cat_e, raw_fc, raw_fname)
    status_code, status_label = _compute_phash_integrity_status(
        raw_p is not None, cat_e is not None, pg_record is not None
    )
    comp_ratio = _calculate_compression_ratio(raw_size, cuts_c, sparse_c)

    return {
        "catalog_id": cat_e["catalog_id"] if cat_e else idx + 1,
        "title": title,
        "year": cat_e.get("year") if cat_e else None,
        "moviehash": mhash or "—",
        "raw_filename": raw_fname,
        "fps": fps,
        "frame_count": fc,
        "duration_ms": dur_ms,
        "duration_fmt": _format_duration(dur_ms),
        "raw_present": raw_p is not None,
        "raw_size_bytes": raw_size,
        "raw_size_human": format_human_size(raw_size),
        "catalog_present": cat_e is not None,
        "catalog_cuts_count": cuts_c,
        "catalog_sparse_count": sparse_c,
        "catalog_compression_ratio": comp_ratio,
        "pg_present": pg_record is not None,
        "pg_media_id": pg_record.get("id") if pg_record else None,
        "pg_status": pg_record.get("phash_status") if pg_record else None,
        "integrity_status": status_code,
        "integrity_label": status_label,
    }


def get_phash_tripartite_summary() -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Gathers and caches the 3 pHash pillars: RAW Store, Catalog Zero-DB, BKtreeDB."""
    cached = get_phashes_audit_cache()
    if cached is not None:
        return cached

    phash_dir = _resolve_data_path("data/__PHASH__")
    cat_path = _resolve_data_path("data/catalog.dbphash")

    raw_files = sorted(list(phash_dir.glob("*.phash")), key=lambda p: p.name) if phash_dir.is_dir() else []
    sidecars = _load_phash_sidecars(phash_dir)
    sidecar_name_index = _build_sidecar_name_index(sidecars)
    hdr, cat_items = _read_catalog_dbphash(cat_path)
    pg_medias, total_pg_medias, frames_table_count = _query_pg_medias_lookup()

    ctx = PhashReconciliationContext(
        sidecars=sidecars,
        sidecar_name_index=sidecar_name_index,
        pg_medias=pg_medias,
    )
    total_raw_bytes = sum(p.stat().st_size for p in raw_files)
    catalog_bytes = hdr.get("file_size", 0)

    reconciled: list[dict[str, Any]] = []
    status_counts = {"aligned_3_3": 0, "missing_pg": 0, "uncompiled": 0, "orphan_catalog": 0}

    max_len = max(len(raw_files), len(cat_items))
    for idx in range(max_len):
        raw_p = raw_files[idx] if idx < len(raw_files) else None
        cat_e = cat_items[idx] if idx < len(cat_items) else None
        item = _build_reconciled_phash_item(raw_p, cat_e, idx, ctx)
        status_val = item["integrity_status"]
        if status_val in status_counts:
            status_counts[status_val] += 1
        reconciled.append(item)

    summary = {
        "raw_count": len(raw_files),
        "raw_size_bytes": total_raw_bytes,
        "raw_size_human": format_human_size(total_raw_bytes),
        "catalog_count": len(cat_items),
        "catalog_size_bytes": catalog_bytes,
        "catalog_size_human": format_human_size(catalog_bytes),
        "catalog_cuts_total": hdr.get("total_cuts", 0),
        "catalog_sparse_total": hdr.get("total_sparse", 0),
        "pg_count": total_pg_medias,
        "pg_frames_total": frames_table_count,
        **status_counts,
    }

    set_phashes_audit_cache(reconciled, summary)
    return reconciled, summary


PHASHES_SORT_MAP: dict[str, tuple[Any, bool]] = {
    "id_asc": (lambda x: x["catalog_id"], False),
    "id_desc": (lambda x: x["catalog_id"], True),
    "title_asc": (lambda x: x["title"].lower(), False),
    "title_desc": (lambda x: x["title"].lower(), True),
    "frames_desc": (lambda x: x["frame_count"], True),
    "frames_asc": (lambda x: x["frame_count"], False),
    "size_desc": (lambda x: x["raw_size_bytes"], True),
    "size_asc": (lambda x: x["raw_size_bytes"], False),
    "cuts_desc": (lambda x: x["catalog_cuts_count"], True),
    "cuts_asc": (lambda x: x["catalog_cuts_count"], False),
}


def query_phashes_list(criteria: PhashesQueryCriteria) -> dict[str, Any]:
    """Filters, sorts and paginates reconciled tripartite pHash fingerprints."""
    items, summary = get_phash_tripartite_summary()

    filtered = items
    if criteria.status_filter in ("aligned_3_3", "missing_pg", "uncompiled", "orphan_catalog"):
        filtered = [x for x in filtered if x["integrity_status"] == criteria.status_filter]

    if criteria.search_q:
        q_norm = strip_accents(criteria.search_q.strip()).lower()
        filtered = [
            x
            for x in filtered
            if q_norm in strip_accents(x["title"]).lower()
            or q_norm in strip_accents(x["raw_filename"]).lower()
            or q_norm in x["moviehash"].lower()
        ]

    key_func, rev = PHASHES_SORT_MAP.get(criteria.sort_by, (lambda x: x["catalog_id"], False))
    sorted_items = sorted(filtered, key=key_func, reverse=rev)

    total_items = len(sorted_items)
    limit = max(1, min(criteria.limit, MAX_PAGE_SIZE))
    total_pages = (total_items + limit - 1) // limit if total_items > 0 else 1
    page = 1 if criteria.page > total_pages and total_items > 0 else max(1, criteria.page)
    offset = (page - 1) * limit
    page_items = sorted_items[offset : offset + limit]

    return {
        "status": "ok",
        "summary": summary,
        "page": page,
        "limit": limit,
        "total_items": total_items,
        "total_pages": total_pages,
        "items": page_items,
    }
