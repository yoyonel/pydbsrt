#!/usr/bin/env python3
"""Media Stream Physical Specs Prober and Null Audit for pydbsrt.

Queries PostgreSQL 17 for media files missing physical container specs (width, height,
video_codec, resolution), probes the real stream over Synology NAS LAN FTP via ffprobe,
and updates database records with ground-truth data. Also provides exhaustive NULL audits.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.parse
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from typing import Dict, List, Tuple

import psycopg2
from psycopg2.extras import execute_batch


def _resolve_ssh_hostname(host_alias: str) -> str:
    """Extracts configured HostName for an SSH alias if present, else returns alias."""
    try:
        proc = subprocess.run(
            ["ssh", "-G", host_alias],
            capture_output=True,
            text=True,
            timeout=2,
            check=False,
        )
        for line in proc.stdout.splitlines():
            if line.startswith("hostname "):
                return line.split()[1]
    except Exception:
        pass
    return host_alias


# Operational & Database Constants
DEFAULT_WORKERS: int = 8
DEFAULT_LOCAL_PG_PORT: int = 5433
DEFAULT_RPI4_PG_PORT: int = 5433
DEFAULT_RPI4_HOST: str = os.getenv("RPI4_HOST") or _resolve_ssh_hostname("rpi4")
DEFAULT_DB_NAME: str = "bktreedb"
DEFAULT_DB_USER: str = os.getenv("PGUSER", "docker")
DEFAULT_DB_PASS: str = os.getenv("PGPASSWORD", "docker")

DEFAULT_NAS_HOST: str = os.getenv("NAS_HOST", "yoyonel.synology.me")
DEFAULT_NAS_USER: str = os.getenv("NAS_USER", "qzsl2686")
DEFAULT_NAS_PASS: str = os.getenv("NAS_PASS", "-2Tz^_V1d(*um%rSNRqx+kZM")
DEFAULT_NAS_PORT: int = 21

DEFAULT_PROBE_TIMEOUT_SECS: int = 15
DEFAULT_BATCH_CHUNK_SIZE: int = 100

# Resolution Height Thresholds
RES_4K_MIN_HEIGHT: int = 2000
RES_4K_MIN_WIDTH: int = 3800
RES_1080P_MIN_HEIGHT: int = 1000
RES_1080P_MIN_WIDTH: int = 1800
RES_720P_MIN_HEIGHT: int = 700
RES_720P_MIN_WIDTH: int = 1200
RES_576P_MIN_HEIGHT: int = 500
RES_480P_MIN_HEIGHT: int = 400


@dataclass
class ProbeTarget:
    """Media target to probe."""

    media_id: int
    media_hash: str
    name: str
    video_rel_path: str | None


@dataclass
class StreamSpecs:
    """Probe-extracted physical stream specifications."""

    width: int
    height: int
    codec: str
    resolution: str


def classify_resolution(width: int, height: int) -> str:
    """Determines standardized resolution label from physical pixel dimensions."""
    if height >= RES_4K_MIN_HEIGHT or width >= RES_4K_MIN_WIDTH:
        return "2160p"
    if height >= RES_1080P_MIN_HEIGHT or width >= RES_1080P_MIN_WIDTH:
        return "1080p"
    if height >= RES_720P_MIN_HEIGHT or width >= RES_720P_MIN_WIDTH:
        return "720p"
    if height >= RES_576P_MIN_HEIGHT:
        return "576p"
    if height >= RES_480P_MIN_HEIGHT:
        return "480p"
    return "360p"


def build_ftp_url(rel_path: str, host: str, user: str, password: str) -> str:
    """Builds a URL-encoded FTP URI compatible with FFmpeg ffprobe."""
    encoded_user = urllib.parse.quote(user, safe="")
    encoded_pass = urllib.parse.quote(password, safe="")
    clean_path = rel_path if rel_path.startswith("/") else f"/{rel_path}"
    return f"ftp://{encoded_user}:{encoded_pass}@{host}{clean_path}"


def probe_stream(
    rel_path: str,
    host: str = DEFAULT_NAS_HOST,
    user: str = DEFAULT_NAS_USER,
    password: str = DEFAULT_NAS_PASS,
    timeout_secs: int = DEFAULT_PROBE_TIMEOUT_SECS,
) -> StreamSpecs | None:
    """Interrogates remote video stream via ffprobe to extract geometry and codec."""
    ftp_url = build_ftp_url(rel_path, host, user, password)
    cmd = [
        "ffprobe",
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,codec_name",
        "-of",
        "json",
        ftp_url,
    ]
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout_secs,
            check=False,
        )
        if proc.returncode != 0:
            return None
        data = json.loads(proc.stdout)
        streams = data.get("streams", [])
        if not streams:
            return None
        vstream = streams[0]
        width = int(vstream.get("width", 0))
        height = int(vstream.get("height", 0))
        codec = str(vstream.get("codec_name", "")).strip().lower()
        if width <= 0 or height <= 0:
            return None
        return StreamSpecs(
            width=width,
            height=height,
            codec=codec,
            resolution=classify_resolution(width, height),
        )
    except Exception:
        return None


def connect_pg(host: str, port: int) -> psycopg2.extensions.connection:
    """Connects to PostgreSQL instance."""
    return psycopg2.connect(
        host=host,
        port=port,
        dbname=DEFAULT_DB_NAME,
        user=DEFAULT_DB_USER,
        password=DEFAULT_DB_PASS,
    )


def audit_null_report(conn: psycopg2.extensions.connection, target_label: str) -> None:
    """Generates an exhaustive report of NULL columns in medias and summary of v_productions."""
    with conn.cursor() as cur:
        cur.execute("SELECT COUNT(*) FROM medias;")
        res = cur.fetchone()
        total_medias: int = res[0] if res else 0

        cur.execute("SELECT COUNT(*) FROM v_productions;")
        res_p = cur.fetchone()
        total_productions: int = res_p[0] if res_p else 0

        columns = (
            "clean_title",
            "media_type",
            "year",
            "season",
            "episode",
            "resolution",
            "source",
            "release_group",
            "width",
            "height",
            "video_codec",
            "video_rel_path",
            "duration_ms",
            "file_size_bytes",
            "moviehash",
            "phash_path",
        )

        null_counts: Dict[str, int] = {}
        for col in columns:
            cur.execute(f"SELECT COUNT(*) FROM medias WHERE {col} IS NULL;")
            r = cur.fetchone()
            null_counts[col] = r[0] if r else 0

        cur.execute("SELECT media_type, COUNT(*) FROM medias GROUP BY media_type ORDER BY COUNT(*) DESC;")
        type_distribution = cur.fetchall()

    print("\n=======================================================")
    print(f"📊 PostgreSQL NULL & SoC Audit Report ({target_label})")
    print("=======================================================")
    print(f"Total Physical Media Releases (`medias`):      {total_medias:,}")
    print(f"Total Conceptual Productions (`v_productions`): {total_productions:,}")
    print("\n--- Colonnes Intangibles (Production) ---")
    for col in ("clean_title", "media_type", "year", "season", "episode"):
        cnt = null_counts[col]
        pct = (cnt / total_medias * 100) if total_medias else 0.0
        print(f"  • {col:<16} : {cnt:>5} NULLs ({pct:>5.1f}%)")

    print("\n--- Colonnes Physiques & Hardware Container ---")
    for col in ("width", "height", "video_codec", "resolution", "source", "release_group"):
        cnt = null_counts[col]
        pct = (cnt / total_medias * 100) if total_medias else 0.0
        print(f"  • {col:<16} : {cnt:>5} NULLs ({pct:>5.1f}%)")

    print("\n--- Stockage, Identifiants & Fichiers ---")
    for col in ("video_rel_path", "moviehash", "duration_ms", "file_size_bytes", "phash_path"):
        cnt = null_counts[col]
        pct = (cnt / total_medias * 100) if total_medias else 0.0
        print(f"  • {col:<16} : {cnt:>5} NULLs ({pct:>5.1f}%)")

    print("\n--- Répartition par Catégorie Média ---")
    for mtype, count in type_distribution:
        print(f"  • {mtype:<12} : {count:>5} releases")
    print("=======================================================\n")


def fetch_probe_targets(conn: psycopg2.extensions.connection, limit: int | None = None) -> List[ProbeTarget]:
    """Retrieves media records having video_rel_path but missing width/height/codec."""
    query = """
        SELECT id, media_hash, name, video_rel_path
        FROM medias
        WHERE video_rel_path IS NOT NULL
          AND (width IS NULL OR height IS NULL OR video_codec IS NULL OR resolution IS NULL)
        ORDER BY id ASC
    """
    if limit is not None:
        query += f" LIMIT {limit}"

    with conn.cursor() as cur:
        cur.execute(query)
        rows = cur.fetchall()

    return [
        ProbeTarget(
            media_id=row[0],
            media_hash=row[1],
            name=row[2],
            video_rel_path=row[3],
        )
        for row in rows
    ]


def _probe_worker(target: ProbeTarget, host: str, user: str, password: str) -> Tuple[ProbeTarget, StreamSpecs | None]:
    """Worker task executing individual stream probe."""
    if not target.video_rel_path:
        return target, None
    specs = probe_stream(target.video_rel_path, host, user, password)
    return target, specs


def run_probe_updates(
    conn: psycopg2.extensions.connection,
    targets: List[ProbeTarget],
    host: str,
    user: str,
    password: str,
    workers: int = DEFAULT_WORKERS,
) -> Tuple[int, int]:
    """Probes video targets concurrently over FTP and updates database records."""
    success_count = 0
    fail_count = 0
    update_batch_records: List[Tuple[int, int, str, str, int]] = []

    update_sql = """
        UPDATE medias
        SET width = %s,
            height = %s,
            video_codec = %s,
            resolution = %s,
            updated_at = NOW()
        WHERE id = %s;
    """

    total = len(targets)
    print(f"🔍 Probing {total} target streams over FTP ({host}) with {workers} workers...")

    with ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [executor.submit(_probe_worker, target, host, user, password) for target in targets]

        for idx, future in enumerate(as_completed(futures), 1):
            target, specs = future.result()
            if specs:
                update_batch_records.append((specs.width, specs.height, specs.codec, specs.resolution, target.media_id))
                success_count += 1
            else:
                fail_count += 1

            if idx % 50 == 0 or idx == total:
                print(f"  [{idx}/{total}] Probed ({success_count} ok, {fail_count} failed)...")

            if len(update_batch_records) >= DEFAULT_BATCH_CHUNK_SIZE:
                with conn.cursor() as cur:
                    execute_batch(cur, update_sql, update_batch_records)
                conn.commit()
                update_batch_records.clear()

    if update_batch_records:
        with conn.cursor() as cur:
            execute_batch(cur, update_sql, update_batch_records)
        conn.commit()
        update_batch_records.clear()

    return success_count, fail_count


def sync_local_to_remote(src_conn: psycopg2.extensions.connection, dst_conn: psycopg2.extensions.connection) -> int:
    """Replicates probed physical specs from local PostgreSQL to remote instance."""
    with src_conn.cursor() as cur:
        cur.execute("""
            SELECT media_hash, width, height, video_codec, resolution
            FROM medias
            WHERE width IS NOT NULL;
        """)
        rows = cur.fetchall()

    if not rows:
        return 0

    update_sql = """
        UPDATE medias
        SET width = %s,
            height = %s,
            video_codec = %s,
            resolution = %s,
            updated_at = NOW()
        WHERE media_hash = %s;
    """
    records = [(r[1], r[2], r[3], r[4], r[0]) for r in rows]
    with dst_conn.cursor() as cur:
        execute_batch(cur, update_sql, records, page_size=DEFAULT_BATCH_CHUNK_SIZE)
    dst_conn.commit()
    return len(records)


def parse_args() -> argparse.Namespace:
    """Parses CLI arguments."""
    parser = argparse.ArgumentParser(description="Probe media container specs and audit NULLs.")
    parser.add_argument(
        "--target",
        choices=["local", "rpi4", "all"],
        default="local",
        help="Target database to inspect/update (default: local)",
    )
    parser.add_argument("--audit", action="store_true", help="Print NULL audit report only")
    parser.add_argument(
        "--sync-rpi4",
        action="store_true",
        help="Sync probed stream specs from local DB to RPi4 DB directly",
    )
    parser.add_argument("--limit", type=int, default=None, help="Limit number of probes")
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS, help="Parallel probe workers")
    parser.add_argument("--nas-host", default=DEFAULT_NAS_HOST, help="NAS host or IP")
    parser.add_argument("--nas-user", default=DEFAULT_NAS_USER, help="NAS FTP user")
    parser.add_argument("--nas-pass", default=DEFAULT_NAS_PASS, help="NAS FTP password")
    return parser.parse_args()


def main() -> int:
    """Main orchestrator."""
    args = parse_args()

    if args.sync_rpi4:
        print("🔄 Synchronizing probed physical specs from Local DB to RPi4 DB...")
        local_conn = connect_pg("127.0.0.1", DEFAULT_LOCAL_PG_PORT)
        rpi4_conn = connect_pg(DEFAULT_RPI4_HOST, DEFAULT_RPI4_PG_PORT)
        try:
            count = sync_local_to_remote(local_conn, rpi4_conn)
            print(f"✓ Replicated {count} stream specs to RPi4 PostgreSQL.")
            audit_null_report(rpi4_conn, "Remote RPi4 PostgreSQL (POST-SYNC)")
        finally:
            local_conn.close()
            rpi4_conn.close()
        return 0

    targets_config: List[Tuple[str, str, int]] = []

    if args.target in ("local", "all"):
        targets_config.append(("Local PostgreSQL", "127.0.0.1", DEFAULT_LOCAL_PG_PORT))
    if args.target in ("rpi4", "all"):
        targets_config.append(("Remote RPi4 PostgreSQL", DEFAULT_RPI4_HOST, DEFAULT_RPI4_PG_PORT))

    for label, host, port in targets_config:
        try:
            conn = connect_pg(host, port)
        except Exception as err:
            print(f"❌ Failed connecting to {label} ({host}:{port}): {err}", file=sys.stderr)
            continue

        try:
            if args.audit:
                audit_null_report(conn, label)
            else:
                audit_null_report(conn, f"{label} - AVANT RATTRAPAGE")
                probe_targets = fetch_probe_targets(conn, limit=args.limit)
                if probe_targets:
                    ok, failed = run_probe_updates(
                        conn=conn,
                        targets=probe_targets,
                        host=args.nas_host,
                        user=args.nas_user,
                        password=args.nas_pass,
                        workers=args.workers,
                    )
                    print(f"\n✓ {label}: {ok} probed and updated, {failed} failed.")
                    audit_null_report(conn, f"{label} - APRÈS RATTRAPAGE")
                else:
                    print(f"✓ {label}: Aucune cible nécessitant un probe.")
        finally:
            conn.close()

    return 0


if __name__ == "__main__":
    sys.exit(main())
