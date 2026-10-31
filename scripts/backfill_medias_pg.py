#!/usr/bin/env python3
"""Backfill and synchronize rich media metadata into PostgreSQL 17 'medias' table.

Extracts metadata from:
1. Binary .phash files: media_hash (SHA256 32-hex), frame_count, fps, duration_ms, release filename.
2. logs/events.jsonl (or data/reports/*.jsonl): video_rel_path, file_size_bytes.
3. Media release parser (clean_title, season, episode, year, resolution, source, release_group, media_type).

Inserts/Upserts strictly into 'medias' table (zero touching of 'frames' or SP-GiST BK-tree).
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import re
import time
import unicodedata
import urllib.parse
from pathlib import Path
from typing import Any, Dict, List, Tuple

import psycopg2
from psycopg2.extras import execute_batch

DEFAULT_FPS: float = 25.0
MILLIS_PER_SEC: float = 1000.0
HASH_BYTE_LEN: int = 8
PAGE_BATCH_SIZE: int = 500
CONNECT_TIMEOUT_SECS: int = 5
KNOWN_VIDEO_EXTS: tuple[str, ...] = (".mkv", ".mp4", ".avi", ".webm", ".ts", ".m2ts")


def compute_media_phash_digest(phash_bytes: bytes) -> str:
    """Computes SHA-256 (first 32 hex chars) of big-endian 64-bit uints."""
    return hashlib.sha256(phash_bytes).hexdigest()[:32]


def parse_fps_from_stem(stem: str) -> float:
    """Extracts floating fps from dot-separated filename stem."""
    match = re.search(r"\.(\d+(?:\.\d+)?)fps(?:\.phash)?$", stem)
    if match:
        try:
            return float(match.group(1))
        except ValueError:
            pass
    return DEFAULT_FPS


def clean_media_name(raw_name: str) -> str:
    """Normalizes URL-encoded characters and strips .phash and .fps suffixes."""
    unquoted = urllib.parse.unquote(raw_name)
    if unquoted.endswith(".phash"):
        unquoted = unquoted[:-6]
    return re.sub(r"\.\d+(?:\.\d+)?fps$", "", unquoted)


def strip_known_video_ext(name: str) -> str:
    """Strips common video extensions from filename stem."""
    for ext in KNOWN_VIDEO_EXTS:
        if name.lower().endswith(ext):
            return name[: -len(ext)]
    return name


def detect_season_episode(clean: str) -> Tuple[str, int | None, int | None, str | None]:
    """Detects anime pattern or standard SxxExx season/episode notation."""
    anime_match = re.search(r"^\[([^\]]+)\]\s*(.+?)\s*-\s*(\d{1,3})\b", clean)
    if anime_match:
        return "anime", None, int(anime_match.group(3)), anime_match.group(1)

    s_match = re.search(r"[sS](\d{1,2})[eE](\d{1,3})", clean)
    if not s_match:
        s_match = re.search(r"\b(\d{1,2})x(\d{1,3})\b", clean)
    if s_match:
        return "tv_episode", int(s_match.group(1)), int(s_match.group(2)), None

    return "movie", None, None, None


def extract_clean_title(clean: str) -> str:
    """Strips release tokens, years, codecs and metadata from the title."""
    cut_tokens = (
        r"\b(19\d\d|20\d\d)\b",
        r"[sS]\d{1,2}[eE]\d{1,3}",
        r"\b\d{1,2}x\d{1,3}\b",
        r"\b(2160p|1080p|720p|WEB-DL|BluRay|HDTV)\b",
    )
    first_cut = len(clean)
    for tok in cut_tokens:
        m = re.search(tok, clean)
        if m and 2 < m.start() < first_cut:
            first_cut = m.start()
    clean_title = clean[:first_cut].strip(" .-_[]()")
    return re.sub(r"[._]", " ", clean_title).strip()


def classify_and_parse(title: str) -> Dict[str, Any]:
    """Parses a release title into structured clean metadata."""
    clean = strip_known_video_ext(title)
    media_type, season, episode, release_group = detect_season_episode(clean)

    y_match = re.search(r"\b(19\d\d|20\d\d)\b", clean)
    year = int(y_match.group(1)) if y_match else None

    res_match = re.search(r"\b(2160p|1080p|1080i|720p|576p|480p|4k|uhd)\b", clean, re.IGNORECASE)
    resolution = res_match.group(1).lower() if res_match else None

    src_match = re.search(r"\b(WEB-DL|WEBRip|BluRay|BDRip|BRRip|HDTV|DVDRip)\b", clean, re.IGNORECASE)
    source = src_match.group(1) if src_match else None

    if not release_group:
        grp_match = re.search(r"-([a-zA-Z0-9_]+)$", clean)
        if grp_match:
            release_group = grp_match.group(1)

    return {
        "clean_title": extract_clean_title(clean),
        "media_type": media_type,
        "year": year,
        "season": season,
        "episode": episode,
        "resolution": resolution,
        "source": source,
        "release_group": release_group,
    }


def parse_telemetry_line(line: str, telemetry: Dict[str, Dict[str, Any]]) -> None:
    """Parses a single JSONL line and updates the index if successful."""
    with contextlib.suppress(Exception):
        d = json.loads(line)
        if not d.get("success"):
            return
        stem = d.get("video_stem", "")
        rel = d.get("video_rel_path", "")
        rec = {"video_rel_path": rel, "file_size_bytes": d.get("file_size_bytes", 0)}
        telemetry[unicodedata.normalize("NFC", stem)] = rec
        if rel:
            telemetry[unicodedata.normalize("NFC", Path(rel).name)] = rec
            telemetry[unicodedata.normalize("NFC", Path(rel).stem)] = rec


def load_telemetry_index(reports_dir: Path) -> Dict[str, Dict[str, Any]]:
    """Loads and normalizes successful execution telemetry from JSONL report files."""
    telemetry: Dict[str, Dict[str, Any]] = {}
    for rfile in reports_dir.glob("*.jsonl"):
        with open(rfile, encoding="utf-8", errors="ignore") as f:
            for raw_line in f:
                line = raw_line.strip()
                if line:
                    parse_telemetry_line(line, telemetry)
    return telemetry


def build_media_row(pfile: Path, telemetry: Dict[str, Dict[str, Any]]) -> Tuple[Any, ...] | None:
    """Extracts raw frames and parsed metadata from an individual .phash file."""
    data = pfile.read_bytes()
    if len(data) < HASH_BYTE_LEN or len(data) % HASH_BYTE_LEN != 0:
        return None

    frame_count = len(data) // HASH_BYTE_LEN
    fps = parse_fps_from_stem(pfile.name)
    duration_ms = int(round(frame_count / fps * MILLIS_PER_SEC))
    media_hash = compute_media_phash_digest(data)

    clean_name = clean_media_name(pfile.name)
    raw_name = clean_name if any(clean_name.endswith(ext) for ext in KNOWN_VIDEO_EXTS) else f"{clean_name}.mkv"
    meta = classify_and_parse(clean_name)

    cand_keys = [
        unicodedata.normalize("NFC", clean_name),
        unicodedata.normalize("NFC", raw_name),
        unicodedata.normalize("NFC", urllib.parse.unquote(pfile.name)),
    ]
    t_info = next((telemetry[k] for k in cand_keys if k in telemetry), None)
    video_rel_path = t_info["video_rel_path"] if t_info else None
    file_size_bytes = t_info["file_size_bytes"] if t_info else 0

    return (
        media_hash,
        raw_name,
        fps,
        frame_count,
        pfile.name,
        meta["clean_title"],
        meta["media_type"],
        meta["year"],
        meta["season"],
        meta["episode"],
        meta["resolution"],
        meta["source"],
        meta["release_group"],
        duration_ms,
        file_size_bytes,
        video_rel_path,
    )


def execute_pg_upsert(args: argparse.Namespace, rows: List[Tuple[Any, ...]]) -> None:
    """Executes atomic batch upsert of media rows into PostgreSQL 17."""
    print(f"Connecting to PostgreSQL at {args.host}:{args.port} ({args.dbname})...")
    conn = psycopg2.connect(
        host=args.host,
        port=args.port,
        user=args.user,
        password=args.password,
        dbname=args.dbname,
        connect_timeout=CONNECT_TIMEOUT_SECS,
    )
    conn.autocommit = False
    cursor = conn.cursor()

    upsert_sql = """
    INSERT INTO medias (
        media_hash, name, fps, frame_count, phash_path,
        clean_title, media_type, year, season, episode,
        resolution, source, release_group, duration_ms,
        file_size_bytes, video_rel_path, updated_at
    ) VALUES (
        %s, %s, %s, %s, %s,
        %s, %s, %s, %s, %s,
        %s, %s, %s, %s,
        %s, %s, NOW()
    )
    ON CONFLICT (media_hash) DO UPDATE SET
        name = EXCLUDED.name,
        fps = EXCLUDED.fps,
        frame_count = EXCLUDED.frame_count,
        phash_path = EXCLUDED.phash_path,
        clean_title = EXCLUDED.clean_title,
        media_type = EXCLUDED.media_type,
        year = EXCLUDED.year,
        season = EXCLUDED.season,
        episode = EXCLUDED.episode,
        resolution = EXCLUDED.resolution,
        source = EXCLUDED.source,
        release_group = EXCLUDED.release_group,
        duration_ms = EXCLUDED.duration_ms,
        file_size_bytes = CASE WHEN EXCLUDED.file_size_bytes > 0 THEN EXCLUDED.file_size_bytes ELSE medias.file_size_bytes END,
        video_rel_path = COALESCE(EXCLUDED.video_rel_path, medias.video_rel_path),
        updated_at = NOW();
    """

    print("Executing batch upsert...")
    t0 = time.time()
    execute_batch(cursor, upsert_sql, rows, page_size=PAGE_BATCH_SIZE)
    conn.commit()
    elapsed = time.time() - t0
    rate = len(rows) / elapsed if elapsed > 0 else 0.0
    print(f"✓ Upserted {len(rows)} media records in {elapsed:.2f}s ({rate:.0f} rows/s).")

    cursor.execute("SELECT COUNT(*) FROM medias;")
    res = cursor.fetchone()
    total_count = res[0] if res else 0
    print(f"Total medias in database: {total_count}")

    cursor.close()
    conn.close()


def main() -> None:
    """CLI Entry point parsing flags and driving the backfill pipeline."""
    parser = argparse.ArgumentParser(description="Backfill medias table in PostgreSQL 17")
    parser.add_argument("--phash-dir", type=Path, default=Path("data/__PHASH__"))
    parser.add_argument("--reports-dir", type=Path, default=Path("data/reports"))
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=5433)
    parser.add_argument("--user", default="docker")
    parser.add_argument("--password", default="docker")
    parser.add_argument("--dbname", default="bktreedb")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    telemetry = load_telemetry_index(args.reports_dir)
    print(f"Loaded {len(telemetry)} telemetry entries.")

    phash_files = sorted(args.phash_dir.glob("*.phash"))
    print(f"Found {len(phash_files)} .phash files to process.")

    rows: List[Tuple[Any, ...]] = []
    for pfile in phash_files:
        row = build_media_row(pfile, telemetry)
        if row is not None:
            rows.append(row)

    print(f"Prepared {len(rows)} media rows for PostgreSQL upsert.")

    if args.dry_run:
        sample = rows[0] if rows else "None"
        print(f"[DRY-RUN] Sample row 0: {sample}")
        return

    execute_pg_upsert(args, rows)


if __name__ == "__main__":
    main()
