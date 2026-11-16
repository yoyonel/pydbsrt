"""PostgreSQL media assets service with psycopg2 primary and psql CLI fallback."""

from __future__ import annotations

import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any

from pydbsrt.portal.config import (
    ACCENTED_CHARS,
    ASSET_TIMESTAMP_SLICE_LEN,
    ASSETS_SORT_MAP,
    DEFAULT_BASELINE_FPS,
    DEFAULT_FTP_PORT,
    DEFAULT_NAS_HOST,
    DEFAULT_NAS_USER,
    DEFAULT_PAGE_SIZE,
    DEFAULT_PG_URL,
    PG_CLI_TIMEOUT_SECS,
    PG_CONNECT_TIMEOUT_SECS,
    PG_O2SWITCH_PORT,
    REPO_ROOT,
    UNACCENTED_CHARS,
    _format_duration,
    format_human_size,
    strip_accents,
)
from pydbsrt.portal.models import AssetsQueryCriteria
from pydbsrt.portal.services import secrets_env


def _connect_pg() -> Any:
    """Safely connects to PostgreSQL (local Docker :5433 or remote cPanel :5432) or returns None."""
    env_url = os.environ.get("DATABASE_URL")
    if env_url:
        try:
            import psycopg2

            return psycopg2.connect(env_url, connect_timeout=int(PG_CONNECT_TIMEOUT_SECS))
        except Exception:
            pass

    o2_pass_file = Path.home() / "dbsrt" / "etc" / ".pg_pass"
    if o2_pass_file.is_file():
        try:
            import psycopg2

            pw = o2_pass_file.read_text(encoding="utf-8").strip()
            return psycopg2.connect(
                dbname="atli6249_bktreedb",
                user="atli6249_dbsrt",
                password=pw,
                host="127.0.0.1",
                port=int(PG_O2SWITCH_PORT),
                connect_timeout=int(PG_CONNECT_TIMEOUT_SECS),
            )
        except Exception:
            pass

    try:
        import psycopg2

        return psycopg2.connect(DEFAULT_PG_URL, connect_timeout=int(PG_CONNECT_TIMEOUT_SECS))
    except Exception:
        return None


def _query_pg_cli(sql: str) -> str | None:
    """Executes SQL via psql CLI wrapper when psycopg2 is not installed in the python environment."""
    o2_pass_file = Path.home() / "dbsrt" / "etc" / ".pg_pass"
    if not o2_pass_file.is_file():
        return None
    try:
        pw = o2_pass_file.read_text(encoding="utf-8").strip()
        env = os.environ.copy()
        env["PGPASSWORD"] = pw
        cmd = [
            "psql",
            "-h",
            "127.0.0.1",
            "-p",
            PG_O2SWITCH_PORT,
            "-U",
            "atli6249_dbsrt",
            "-d",
            "atli6249_bktreedb",
            "-t",
            "-A",
            "-c",
            sql,
        ]
        out = subprocess.check_output(cmd, env=env, timeout=PG_CLI_TIMEOUT_SECS, stderr=subprocess.DEVNULL)
        return out.decode("utf-8").strip()
    except Exception:
        return None


def _detect_pg_version(conn: Any | None = None) -> str:
    """Detects active PostgreSQL server version dynamically."""
    if conn is not None:
        try:
            cur = conn.cursor()
            cur.execute("SHOW server_version;")
            row = cur.fetchone()
            if row:
                raw = str(row[0] if isinstance(row, tuple | list) else row.get("server_version", ""))
                major_minor = ".".join(raw.split()[0].split(".")[:2])
                return f"PostgreSQL {major_minor}"
        except Exception:
            pass
    raw_ver = _query_pg_cli("SHOW server_version;")
    if raw_ver:
        major_minor = ".".join(raw_ver.strip().split()[0].split(".")[:2])
        return f"PostgreSQL {major_minor}"
    return "PostgreSQL"


def _resolve_physical_file_exists(source_id: str | None, video_rel_path: str | None) -> tuple[bool, str]:
    """Checks whether video_rel_path exists physically on current host local mounts/directories."""
    if not video_rel_path:
        return False, ""

    clean_path = video_rel_path.lstrip("/")
    resolved_path: Path | None = None

    if source_id == "synology-video":
        resolved_path = Path(secrets_env._user_media_path("Synology", "video")) / clean_path.replace("video/", "", 1)
    elif source_id == "synology-tvshow":
        resolved_path = Path(secrets_env._user_media_path("Synology", "tvshow")) / clean_path.replace("tvshow/", "", 1)
    elif source_id == "local-snippets":
        resolved_path = REPO_ROOT / "data" / "snippets" / Path(clean_path).name
    else:
        cand = Path("/" + clean_path)
        if cand.is_file():
            return True, str(cand)
        cand_user = Path(secrets_env._user_media_path("Synology")) / clean_path
        if cand_user.is_file():
            return True, str(cand_user)
        resolved_path = REPO_ROOT / clean_path

    if resolved_path and resolved_path.is_file():
        return True, str(resolved_path)
    return False, str(resolved_path) if resolved_path else ""


def _check_phash_file_exists(moviehash: str | None, fps: float | None) -> bool:
    """Checks presence of .phash file in data/__PHASH__/ on disk."""
    if not moviehash:
        return False
    phash_dir = REPO_ROOT / "data" / "__PHASH__"
    fps_val = f"{fps:.1f}" if fps is not None else f"{DEFAULT_BASELINE_FPS:.1f}"
    for candidate_fps in (fps_val, "24.0", "23.976"):
        if (phash_dir / f"{moviehash}.{candidate_fps}fps.phash").is_file():
            return True
    return False


def _compute_asset_status_tag(phash_status: str | None, video_rel_path: str | None) -> str:
    """Computes nominal, missing_phash, or unmapped_path business tag."""
    if phash_status == "complete" and video_rel_path:
        return "nominal"
    if phash_status != "complete":
        return "missing_phash"
    return "unmapped_path"


def _get_asset_alt_channels(source_id: str | None, video_rel_path: str | None) -> list[dict[str, Any]]:
    """Builds remote streaming WAN FTP fallback profile when source is Synology."""
    if source_id in ("synology-video", "synology-tvshow") and video_rel_path:
        return [
            {
                "id": "wan_ftp",
                "name": "Streaming WAN FTP (Synology)",
                "accessible": True,
                "target": f"ftp://{DEFAULT_NAS_USER}@{DEFAULT_NAS_HOST}:{DEFAULT_FTP_PORT}{video_rel_path}",
            }
        ]
    return []


def _format_asset_row(r: dict[str, Any]) -> dict[str, Any]:
    """Formats an asset database record into an enriched frontend descriptor."""
    phys_exists, phys_path = _resolve_physical_file_exists(r.get("source_id"), r.get("video_rel_path"))
    phash_on_disk = _check_phash_file_exists(r.get("moviehash"), r.get("fps"))
    status_tag = _compute_asset_status_tag(r.get("phash_status"), r.get("video_rel_path"))
    alt_channels = _get_asset_alt_channels(r.get("source_id"), r.get("video_rel_path"))

    codec_raw = r.get("video_codec") or "—"
    return {
        "id": r["id"],
        "name": r["name"],
        "clean_title": r.get("clean_title") or r["name"],
        "season": r.get("season"),
        "episode": r.get("episode"),
        "year": r.get("year"),
        "resolution": r.get("resolution") or "—",
        "video_codec": codec_raw.upper(),
        "fps": r.get("fps") or DEFAULT_BASELINE_FPS,
        "is_vfr": r.get("is_vfr") or False,
        "frame_count": r.get("frame_count") or 0,
        "expected_frames": r.get("expected_frames") or 0,
        "duration_ms": r.get("duration_ms") or 0,
        "duration_fmt": _format_duration(r.get("duration_ms")),
        "file_size_bytes": r.get("file_size_bytes") or 0,
        "file_size_human": format_human_size(r.get("file_size_bytes") or 0),
        "moviehash": r.get("moviehash") or "",
        "media_hash": r.get("media_hash") or "",
        "source_id": r.get("source_id") or "",
        "video_rel_path": r.get("video_rel_path") or "",
        "physical_exists": phys_exists,
        "physical_resolved_path": phys_path,
        "phash_status": r.get("phash_status"),
        "phash_on_disk": phash_on_disk,
        "status_tag": status_tag,
        "alt_channels": alt_channels,
    }


def _build_assets_search_clause(search_q: str, is_psycopg: bool) -> tuple[str, list[Any]]:
    """Builds accent-insensitive SQL LIKE search predicate for assets queries."""
    q_norm = strip_accents(search_q.strip()).lower()
    title_sql = f"translate(lower(clean_title), '{ACCENTED_CHARS}', '{UNACCENTED_CHARS}')"
    name_sql = f"translate(lower(name), '{ACCENTED_CHARS}', '{UNACCENTED_CHARS}')"
    if is_psycopg:
        clean_q = f"%{q_norm}%"
        return (
            f"({title_sql} LIKE %s OR {name_sql} LIKE %s OR moviehash ILIKE %s)",
            [clean_q, clean_q, f"%{search_q.strip()}%"],
        )
    safe_q = re.sub(r"['\";\\]", "", q_norm)
    safe_raw = re.sub(r"['\";\\]", "", search_q.strip())
    return (
        f"({title_sql} LIKE '%{safe_q}%' OR {name_sql} LIKE '%{safe_q}%' OR moviehash ILIKE '%{safe_raw}%')",
        [],
    )


def _build_assets_where(
    status_filter: str, source_filter: str, search_q: str, is_psycopg: bool
) -> tuple[str, list[Any]]:
    """Builds SQL WHERE clause and parameters for assets queries."""
    where_clauses: list[str] = []
    params: list[Any] = []
    status_predicates = {
        "nominal": "phash_status = 'complete' AND video_rel_path IS NOT NULL AND video_rel_path != ''",
        "missing_phash": "phash_status != 'complete'",
        "unmapped_path": "(video_rel_path IS NULL OR video_rel_path = '')",
    }
    if status_filter in status_predicates:
        where_clauses.append(status_predicates[status_filter])

    if source_filter:
        if is_psycopg:
            where_clauses.append("source_id = %s")
            params.append(source_filter)
        else:
            safe_src = re.sub(r"[^a-zA-Z0-9_-]", "", source_filter)
            where_clauses.append(f"source_id = '{safe_src}'")

    if search_q:
        clause, extra_params = _build_assets_search_clause(search_q, is_psycopg)
        where_clauses.append(clause)
        params.extend(extra_params)

    where_sql = ("WHERE " + " AND ".join(where_clauses)) if where_clauses else ""
    return where_sql, params


def _empty_assets_error(criteria: AssetsQueryCriteria, message: str) -> dict[str, Any]:
    """Builds standardized empty error payload for assets queries."""
    return {
        "status": "error",
        "message": message,
        "connected": False,
        "summary": {"total": 0, "nominal": 0, "missing_phash": 0, "unmapped_path": 0},
        "page": criteria.page,
        "limit": criteria.limit,
        "total_items": 0,
        "total_pages": 0,
        "items": [],
    }


def _get_pg_assets_cli(criteria: AssetsQueryCriteria) -> dict[str, Any]:
    """Queries assets using native psql CLI (fallback when psycopg2 is absent)."""
    cli_summary_raw = _query_pg_cli("""
        SELECT json_build_object(
            'total', count(*),
            'nominal', count(CASE WHEN phash_status = 'complete' AND video_rel_path IS NOT NULL AND video_rel_path != '' THEN 1 END),
            'missing_phash', count(CASE WHEN phash_status != 'complete' THEN 1 END),
            'unmapped_path', count(CASE WHEN video_rel_path IS NULL OR video_rel_path = '' THEN 1 END)
        )::text FROM medias;
    """)
    if not cli_summary_raw:
        return _empty_assets_error(criteria, "PostgreSQL non disponible (:5433 / :5432). Vérifiez 'task db:up'.")

    try:
        summary = json.loads(cli_summary_raw)
        where_sql, _ = _build_assets_where(
            criteria.status_filter, criteria.source_filter, criteria.search_q, is_psycopg=False
        )

        count_raw = _query_pg_cli(f"SELECT count(*) FROM medias {where_sql};")
        total_items = int(count_raw) if count_raw and count_raw.isdigit() else 0

        total_pages = (total_items + criteria.limit - 1) // criteria.limit if total_items > 0 else 1
        page = 1 if criteria.page > total_pages and total_items > 0 else criteria.page
        offset = max(0, (page - 1) * criteria.limit)
        order_sql = ASSETS_SORT_MAP.get(criteria.sort_by, "clean_title ASC NULLS LAST, name ASC")

        data_raw = _query_pg_cli(f"""
            SELECT json_agg(t)::text FROM (
                SELECT
                    id, name, clean_title, season, episode, year, resolution, video_codec,
                    fps, frame_count, expected_frames, is_vfr, duration_ms, file_size_bytes,
                    moviehash, media_hash, video_rel_path, source_id, phash_status
                FROM medias
                {where_sql}
                ORDER BY {order_sql}
                LIMIT {criteria.limit} OFFSET {offset}
            ) t;
        """)
        rows = json.loads(data_raw) if data_raw and data_raw != "null" else []
        items = [_format_asset_row(r) for r in rows]

        return {
            "status": "ok",
            "connected": True,
            "pg_version": _detect_pg_version(),
            "summary": summary,
            "page": page,
            "limit": criteria.limit,
            "total_items": total_items,
            "total_pages": total_pages,
            "items": items,
        }
    except Exception as exc:
        return _empty_assets_error(criteria, str(exc))


def _get_pg_assets_psycopg2(conn: Any, criteria: AssetsQueryCriteria) -> dict[str, Any]:
    """Queries assets using native psycopg2 connection."""
    from psycopg2.extras import RealDictCursor

    try:
        cur = conn.cursor(cursor_factory=RealDictCursor)
        cur.execute("""
            SELECT
                count(*) AS total,
                count(CASE WHEN phash_status = 'complete' AND video_rel_path IS NOT NULL AND video_rel_path != '' THEN 1 END) AS nominal,
                count(CASE WHEN phash_status != 'complete' THEN 1 END) AS missing_phash,
                count(CASE WHEN video_rel_path IS NULL OR video_rel_path = '' THEN 1 END) AS unmapped_path
            FROM medias;
        """)
        srow = cur.fetchone()
        summary = {
            "total": srow["total"] if srow else 0,
            "nominal": srow["nominal"] if srow else 0,
            "missing_phash": srow["missing_phash"] if srow else 0,
            "unmapped_path": srow["unmapped_path"] if srow else 0,
        }

        where_sql, params = _build_assets_where(
            criteria.status_filter, criteria.source_filter, criteria.search_q, is_psycopg=True
        )
        cur.execute(f"SELECT count(*) FROM medias {where_sql};", params)
        total_items = cur.fetchone()["count"]

        total_pages = (total_items + criteria.limit - 1) // criteria.limit if total_items > 0 else 1
        page = 1 if criteria.page > total_pages and total_items > 0 else criteria.page
        offset = max(0, (page - 1) * criteria.limit)
        order_sql = ASSETS_SORT_MAP.get(criteria.sort_by, "clean_title ASC NULLS LAST, name ASC")

        data_query = f"""
            SELECT
                id, name, clean_title, season, episode, year, resolution, video_codec,
                fps, frame_count, expected_frames, is_vfr, duration_ms, file_size_bytes,
                moviehash, media_hash, video_rel_path, source_id, phash_status
            FROM medias
            {where_sql}
            ORDER BY {order_sql}
            LIMIT %s OFFSET %s;
        """
        cur.execute(data_query, params + [criteria.limit, offset])
        items = [_format_asset_row(r) for r in cur.fetchall()]

        return {
            "status": "ok",
            "connected": True,
            "pg_version": _detect_pg_version(conn),
            "summary": summary,
            "page": page,
            "limit": criteria.limit,
            "total_items": total_items,
            "total_pages": total_pages,
            "items": items,
        }
    except Exception as exc:
        return _empty_assets_error(criteria, str(exc))
    finally:
        conn.close()


def get_pg_assets(
    page: int = 1,
    limit: int = DEFAULT_PAGE_SIZE,
    status_filter: str = "all",
    source_filter: str = "",
    search_q: str = "",
    sort_by: str = "title_asc",
) -> dict[str, Any]:
    """Queries PostgreSQL medias table, computes physical & pHash health metrics."""
    criteria = AssetsQueryCriteria(
        page=page,
        limit=limit,
        status_filter=status_filter,
        source_filter=source_filter,
        search_q=search_q,
        sort_by=sort_by,
    )
    conn = _connect_pg()
    if conn is None:
        return _get_pg_assets_cli(criteria)
    return _get_pg_assets_psycopg2(conn, criteria)


def _format_asset_detail(row: dict[str, Any], frames_in_db: int = 0) -> dict[str, Any]:
    """Enriches raw media row with formatted metrics and storage audit."""
    phys_exists, phys_path = _resolve_physical_file_exists(row.get("source_id"), row.get("video_rel_path"))
    phash_on_disk = _check_phash_file_exists(row.get("moviehash"), row.get("fps"))
    alt_channels = _get_asset_alt_channels(row.get("source_id"), row.get("video_rel_path"))

    res = dict(row)
    res["status"] = "ok"
    res["frames_in_db"] = frames_in_db
    res["physical_exists"] = phys_exists
    res["physical_resolved_path"] = phys_path
    res["phash_on_disk"] = phash_on_disk
    res["duration_fmt"] = _format_duration(row.get("duration_ms"))
    res["file_size_human"] = format_human_size(row.get("file_size_bytes") or 0)
    res["created_at_fmt"] = str(row.get("created_at") or "—")[:ASSET_TIMESTAMP_SLICE_LEN]
    res["updated_at_fmt"] = str(row.get("updated_at") or "—")[:ASSET_TIMESTAMP_SLICE_LEN]
    res["alt_channels"] = alt_channels
    return res


def _get_pg_asset_detail_cli(media_id: int) -> dict[str, Any]:
    """Queries media detail using psql CLI fallback."""
    cli_detail_raw = _query_pg_cli(f"""
        SELECT json_build_object(
            'id', id, 'name', name, 'clean_title', clean_title,
            'season', season, 'episode', episode, 'year', year,
            'resolution', resolution, 'video_codec', video_codec,
            'width', width, 'height', height, 'fps', fps,
            'frame_count', frame_count, 'expected_frames', expected_frames,
            'is_vfr', is_vfr, 'duration_ms', duration_ms,
            'file_size_bytes', file_size_bytes, 'moviehash', moviehash,
            'media_hash', media_hash, 'video_rel_path', video_rel_path,
            'source_id', source_id, 'phash_status', phash_status,
            'phash_path', phash_path, 'asset_kind', asset_kind,
            'parent_media_id', parent_media_id, 'source_mtime_unix', source_mtime_unix,
            'created_at', created_at, 'updated_at', updated_at
        )::text FROM medias WHERE id = {media_id};
    """)
    if not cli_detail_raw:
        return {"status": "error", "message": "PostgreSQL non disponible"}
    try:
        row = json.loads(cli_detail_raw)
        return _format_asset_detail(row, frames_in_db=0)
    except Exception as exc:
        return {"status": "error", "message": str(exc)}


def _get_pg_asset_detail_psycopg2(conn: Any, media_id: int) -> dict[str, Any]:
    """Queries media detail using psycopg2 connection."""
    from psycopg2.extras import RealDictCursor

    try:
        cur = conn.cursor(cursor_factory=RealDictCursor)
        cur.execute(
            """
            SELECT
                id, name, clean_title, season, episode, year, resolution, video_codec,
                width, height, fps, frame_count, expected_frames, is_vfr, duration_ms,
                file_size_bytes, moviehash, media_hash, video_rel_path, source_id,
                phash_status, phash_path, asset_kind, parent_media_id, source_mtime_unix,
                created_at, updated_at
            FROM medias
            WHERE id = %s;
        """,
            (media_id,),
        )
        row = cur.fetchone()
        if not row:
            return {"status": "not_found", "message": f"Média #{media_id} introuvable"}

        cur.execute("SELECT count(*) AS frames_in_db FROM frames WHERE media_id = %s;", (media_id,))
        frames_db_cnt = cur.fetchone()["frames_in_db"]
        return _format_asset_detail(dict(row), frames_in_db=frames_db_cnt)
    except Exception as exc:
        return {"status": "error", "message": str(exc)}
    finally:
        conn.close()


def get_pg_asset_detail(media_id: int) -> dict[str, Any]:
    """Returns deep technical details for single media asset from PostgreSQL."""
    conn = _connect_pg()
    if conn is None:
        return _get_pg_asset_detail_cli(media_id)
    return _get_pg_asset_detail_psycopg2(conn, media_id)
