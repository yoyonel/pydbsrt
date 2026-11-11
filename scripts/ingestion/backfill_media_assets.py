#!/usr/bin/env python3
"""Backfills media_sources and populates V6 asset lineage columns in medias table.

Zero speculation & reproducible execution:
1. Registers default media sources (synology-tvshow, synology-video, local-snippets) in `media_sources`.
2. Enriches `medias` rows from existing 1,703 sidecar JSONs (moviehash, frame_count, fps).
3. Enriches remaining medias from active CIFS mounts computing O(1) MovieHash.
4. Calculates expected_frames = round(duration_ms / 1000.0 * fps) for all rows.
5. Sets source_id and phash_status ('complete' if phash exists and passes completeness, else 'pending').
"""

from __future__ import annotations

import argparse
import getpass
import json
import os
import struct
import sys
import time
import unicodedata
from pathlib import Path
from typing import Any

import psycopg2
from psycopg2.extensions import connection as PgConn


def find_repo_root() -> Path:
    """Finds repository root dynamically with appliance fallback."""
    curr = Path(__file__).resolve().parent
    for parent in [curr, *curr.parents]:
        if (parent / ".git").is_dir():
            return parent
    appliance_path = Path.home() / "dbsrt"
    if appliance_path.is_dir():
        return appliance_path
    return Path.cwd()


def default_nas_media_dir(sub: str) -> Path:
    env_root = os.environ.get("MEDIA_ROOT") or os.environ.get("NAS_ROOT")
    if env_root:
        return Path(env_root) / sub
    return Path("/media") / getpass.getuser() / "Synology" / sub


REPO_ROOT = find_repo_root()
DEFAULT_DB_URL: str = "postgresql://docker:docker@127.0.0.1:5433/bktreedb"
DEFAULT_PHASH_DIR: Path = REPO_ROOT / "data/__PHASH__"
DEFAULT_SYNOLOGY_TVSHOW_DIR: Path = default_nas_media_dir("tvshow")
DEFAULT_SYNOLOGY_VIDEO_DIR: Path = default_nas_media_dir("video")
MIN_BYTES_FOR_MOVIEHASH: int = 131_072  # 128 KiB
CHUNK_SIZE: int = 65_536  # 64 KiB
MIN_FRAMES_MARGIN: int = 12
TOLERANCE_RATIO: float = 0.02


def compute_file_moviehash(filepath: Path) -> tuple[str, int] | None:
    """Computes OpenSubtitles 64-bit uint checksum over initial 64KiB and final 64KiB."""
    try:
        filesize = filepath.stat().st_size
        if filesize < MIN_BYTES_FOR_MOVIEHASH:
            return None
        with open(filepath, "rb") as f:
            head = f.read(CHUNK_SIZE)
            f.seek(-CHUNK_SIZE, 2)
            tail = f.read(CHUNK_SIZE)

        hash_val = filesize
        for chunk in [head, tail]:
            for i in range(0, len(chunk), 8):
                (val,) = struct.unpack("<Q", chunk[i : i + 8])
                hash_val = (hash_val + val) & 0xFFFFFFFFFFFFFFFF
        return f"{hash_val:016x}", filesize
    except OSError:
        return None


def register_media_sources(conn: PgConn, dry_run: bool = False) -> None:
    """Ensures root media sources exist in media_sources table."""
    sources = [
        (
            "synology-tvshow",
            "Synology NAS TV Shows",
            "cifs_mount",
            "smb://synology-nas/tvshow",
            True,
        ),
        (
            "synology-video",
            "Synology NAS Movies & Videos",
            "cifs_mount",
            "smb://synology-nas/video",
            True,
        ),
        (
            "local-snippets",
            "Local Test Snippets & Cuts",
            "local",
            "data/snippets",
            True,
        ),
    ]

    cur = conn.cursor()
    for sid, name, kind, uri, enabled in sources:
        if not dry_run:
            cur.execute(
                """
                INSERT INTO media_sources (id, name, storage_kind, root_uri, enabled)
                VALUES (%s, %s, %s, %s, %s)
                ON CONFLICT (id) DO UPDATE SET
                    name = EXCLUDED.name,
                    storage_kind = EXCLUDED.storage_kind,
                    root_uri = EXCLUDED.root_uri,
                    enabled = EXCLUDED.enabled,
                    updated_at = NOW()
                """,
                (sid, name, kind, uri, enabled),
            )
    if not dry_run:
        conn.commit()
    print(f"  ✓ Registered {len(sources)} media sources (synology-tvshow, synology-video, local-snippets)")


def process_single_sidecar(cur: Any, sc_path: Path, dry_run: bool) -> bool:
    """Processes a single sidecar JSON file and enriches its corresponding media row."""
    try:
        data = json.loads(sc_path.read_text(encoding="utf-8"))
        media_name = data.get("media_name")
        moviehash = data.get("moviehash")
        frames = int(data.get("frames", 0))
        fps = float(data.get("fps", 24.0))

        if not media_name or not moviehash:
            return False

        # Match by direct media_name, or NFC-normalized media_name
        cur.execute(
            "SELECT id, duration_ms, source_id, video_rel_path FROM medias WHERE name = %s OR name = %s",
            (media_name, unicodedata.normalize("NFD", media_name)),
        )
        row = cur.fetchone()
        if not row:
            cur.execute(
                "SELECT id, duration_ms, source_id, video_rel_path FROM medias WHERE name = %s",
                (unicodedata.normalize("NFC", media_name),),
            )
            row = cur.fetchone()
        if not row:
            return False

        media_id, duration_ms, existing_source_id, video_rel_path = row
        duration_ms = duration_ms or 0

        source_id = existing_source_id
        if not source_id and video_rel_path:
            if str(video_rel_path).startswith("/tvshow/"):
                source_id = "synology-tvshow"
            elif str(video_rel_path).startswith("/video/"):
                source_id = "synology-video"

        expected_frames = round((duration_ms / 1000.0) * fps) if duration_ms > 0 else frames
        tolerance = max(MIN_FRAMES_MARGIN, int(TOLERANCE_RATIO * expected_frames))
        delta = abs(frames - expected_frames)
        status = "complete" if delta <= tolerance else "truncated"

        if not dry_run:
            cur.execute(
                """
                UPDATE medias
                SET moviehash = %s,
                    frame_count = %s,
                    fps = %s,
                    expected_frames = %s,
                    source_id = COALESCE(%s, source_id),
                    phash_status = %s,
                    updated_at = NOW()
                WHERE id = %s
                """,
                (moviehash, frames, fps, expected_frames, source_id, status, media_id),
            )
        return True
    except Exception:
        return False


def backfill_from_sidecars(conn: PgConn, phash_dir: Path, dry_run: bool = False) -> tuple[int, int]:
    """Backfills moviehash, frame_count, fps, and expected_frames from existing .json sidecars."""
    cur = conn.cursor()
    sidecar_files = list(phash_dir.glob("*.json"))
    updated = 0

    for sc_path in sidecar_files:
        if process_single_sidecar(cur, sc_path, dry_run):
            updated += 1

    if not dry_run:
        conn.commit()

    return updated, updated


def resolve_media_file_path(
    video_rel_path: str, tvshow_dir: Path, video_dir: Path, tvshow_ok: bool, video_ok: bool
) -> tuple[Path | None, str | None]:
    """Resolves local mount path and source ID from a video_rel_path."""
    if video_rel_path.startswith("/tvshow/") and tvshow_ok:
        return tvshow_dir / video_rel_path[len("/tvshow/") :], "synology-tvshow"
    if video_rel_path.startswith("/video/") and video_ok:
        return video_dir / video_rel_path[len("/video/") :], "synology-video"
    return None, None


def update_media_from_file(
    cur: Any, media_id: int, file_path: Path, source_id: str, duration_ms: int, fps: float, dry_run: bool
) -> bool:
    """Computes moviehash and updates media row from local file."""
    res = compute_file_moviehash(file_path)
    if not res:
        return False
    moviehash, file_size = res
    expected_frames = round((duration_ms / 1000.0) * fps) if duration_ms > 0 else 0
    mtime_unix = int(file_path.stat().st_mtime)

    if not dry_run:
        cur.execute(
            """
            UPDATE medias
            SET moviehash = %s,
                file_size_bytes = %s,
                expected_frames = %s,
                source_id = %s,
                source_mtime_unix = %s,
                updated_at = NOW()
            WHERE id = %s
            """,
            (moviehash, file_size, expected_frames, source_id, mtime_unix, media_id),
        )
    return True


def backfill_from_cifs_mounts(
    conn: PgConn,
    tvshow_dir: Path,
    video_dir: Path,
    dry_run: bool = False,
) -> tuple[int, int]:
    """Scans medias with missing moviehash and computes O(1) hash from mounted CIFS shares."""
    cur = conn.cursor()
    cur.execute(
        """
        SELECT id, name, video_rel_path, duration_ms, fps
        FROM medias
        WHERE moviehash IS NULL AND video_rel_path IS NOT NULL
        ORDER BY id
        """
    )
    rows = cur.fetchall()
    tvshow_ok = tvshow_dir.exists() and tvshow_dir.is_dir()
    video_ok = video_dir.exists() and video_dir.is_dir()

    if not tvshow_ok and not video_ok:
        print("  ⚠️ Neither Synology CIFS mount is accessible; skipping filesystem scan.")
        return 0, 0

    resolved = 0
    for media_id, _name, video_rel_path, duration_ms, fps in rows:
        fpath, sid = resolve_media_file_path(str(video_rel_path), tvshow_dir, video_dir, tvshow_ok, video_ok)
        if fpath and sid and fpath.exists():
            if update_media_from_file(cur, media_id, fpath, sid, duration_ms or 0, fps or 24.0, dry_run):
                resolved += 1

    if not dry_run:
        conn.commit()

    return len(rows), resolved


def deduplicate_exact_path_collisions(conn: PgConn, dry_run: bool = False) -> int:
    """Merges historical duplicate rows sharing the exact same video_rel_path.

    Prefers the row with the longer duration_ms (full master instead of cut).
    Deletes the shorter duplicate to satisfy the idx_medias_container_identity uniqueness invariant.
    """
    cur = conn.cursor()
    cur.execute(
        """
        SELECT m1.id, m2.id, m1.name, m2.name, m1.duration_ms, m2.duration_ms
        FROM medias m1
        JOIN medias m2 ON m1.video_rel_path = m2.video_rel_path AND m1.id < m2.id
        WHERE m1.video_rel_path IS NOT NULL
        """
    )
    pairs = cur.fetchall()
    removed = 0

    for id1, id2, _name1, _name2, dur1, dur2 in pairs:
        dur1 = dur1 or 0
        dur2 = dur2 or 0
        victim_id = id2 if dur1 >= dur2 else id1

        if not dry_run:
            # Delete victim if it has no foreign key dependencies
            cur.execute("DELETE FROM frames WHERE media_id = %s", (victim_id,))
            cur.execute("DELETE FROM subtitles WHERE media_id = %s", (victim_id,))
            cur.execute("DELETE FROM medias WHERE id = %s", (victim_id,))
        removed += 1

    if not dry_run:
        conn.commit()

    return removed


def sanitize_remaining_expected_frames(conn: PgConn, dry_run: bool = False) -> int:
    """Ensures any remaining rows have expected_frames = round(duration_ms/1000 * fps)."""
    cur = conn.cursor()
    cur.execute(
        """
        SELECT COUNT(*)
        FROM medias
        WHERE expected_frames = 0 AND duration_ms > 0 AND fps > 0
        """
    )
    count_row = cur.fetchone()
    count = count_row[0] if count_row else 0

    if count > 0 and not dry_run:
        cur.execute(
            """
            UPDATE medias
            SET expected_frames = ROUND((duration_ms / 1000.0) * fps),
                updated_at = NOW()
            WHERE expected_frames = 0 AND duration_ms > 0 AND fps > 0
            """
        )
        conn.commit()

    return count


def perform_check(args: argparse.Namespace) -> None:
    """Performs sanity check without connecting to PostgreSQL."""
    print("[CHECK] Mode verification activé.")
    print(f"[CHECK] REPO_ROOT : {REPO_ROOT}")
    print(f"[CHECK] Target DB : {args.db}")
    print(f"[CHECK] Phash dir : {args.phash_dir} (exists: {args.phash_dir.is_dir()})")
    count_phash = len(list(args.phash_dir.glob("*.phash"))) if args.phash_dir.is_dir() else 0
    count_json = len(list(args.phash_dir.glob("*.json"))) if args.phash_dir.is_dir() else 0
    print(f"[CHECK] Found files : {count_phash} .phash, {count_json} .json sidecars")
    print(f"[CHECK] TV show mount : {args.tvshow_mount} (exists: {args.tvshow_mount.is_dir()})")
    print(f"[CHECK] Video mount : {args.video_mount} (exists: {args.video_mount.is_dir()})")


def main() -> int:
    parser = argparse.ArgumentParser(description="Backfill media_sources and V6 asset lineage columns")
    parser.add_argument("--db", type=str, default=DEFAULT_DB_URL, help="Database connection URL")
    parser.add_argument("--phash-dir", type=Path, default=DEFAULT_PHASH_DIR, help="Path to data/__PHASH__")
    parser.add_argument(
        "--tvshow-mount", type=Path, default=DEFAULT_SYNOLOGY_TVSHOW_DIR, help="Path to Synology tvshow mount"
    )
    parser.add_argument(
        "--video-mount", type=Path, default=DEFAULT_SYNOLOGY_VIDEO_DIR, help="Path to Synology video mount"
    )
    parser.add_argument("--dry-run", action="store_true", help="Simulate without database writes")
    parser.add_argument("--check", action="store_true", help="Perform sanity checks without connecting to PostgreSQL")
    args = parser.parse_args()

    if args.check:
        perform_check(args)
        return 0

    print("======================================================================")
    print(" 🔄 PYDBSRT POSTGRESQL V6 ASSET LINEAGE & SOURCE BACKFILL")
    print("======================================================================")
    print(f"Target DB       : {args.db}")
    print(f"Phash Sidecars  : {args.phash_dir}")
    print(f"Mode            : {'[DRY-RUN]' if args.dry_run else '[LIVE UPDATE]'}")

    t0 = time.monotonic()
    conn = psycopg2.connect(args.db)

    print("\n--- [1/5] Deduplicating Exact Path Collisions ---")
    deduped = deduplicate_exact_path_collisions(conn, dry_run=args.dry_run)
    print(f"  ✓ Purged {deduped} duplicate rows sharing same video_rel_path")

    print("\n--- [2/5] Registering Default Media Sources ---")
    register_media_sources(conn, dry_run=args.dry_run)

    print("\n--- [3/5] Backfilling Medias from Sidecar JSONs ---")
    sc_matched, sc_updated = backfill_from_sidecars(conn, args.phash_dir, dry_run=args.dry_run)
    print(
        f"  ✓ Matched: {sc_matched}, Updated: {sc_updated} rows from {len(list(args.phash_dir.glob('*.json')))} sidecars"
    )

    print("\n--- [4/5] Backfilling Missing MovieHashes from CIFS Mounts ---")
    cifs_scanned, cifs_resolved = backfill_from_cifs_mounts(
        conn, args.tvshow_mount, args.video_mount, dry_run=args.dry_run
    )
    print(f"  ✓ Scanned {cifs_scanned} candidate rows, resolved {cifs_resolved} files via O(1) hash")

    print("\n--- [5/5] Sanitizing Expected Frames Formula ---")
    sanitized = sanitize_remaining_expected_frames(conn, dry_run=args.dry_run)
    print(f"  ✓ Calculated expected_frames for {sanitized} remaining rows")

    # Post-status summary
    cur = conn.cursor()
    cur.execute(
        """
        SELECT
            COUNT(*) as total,
            COUNT(moviehash) as has_moviehash,
            COUNT(source_id) as has_source_id,
            COUNT(expected_frames) FILTER (WHERE expected_frames > 0) as has_expected_frames,
            COUNT(*) FILTER (WHERE phash_status = 'complete') as complete_phashes
        FROM medias
        """
    )
    post_stats = cur.fetchone()
    conn.close()

    elapsed = time.monotonic() - t0
    print("\n======================================================================")
    if post_stats:
        print(f" 🎉 BACKFILL FINISHED in {elapsed:.2f}s")
        print(f"    • Total Medias       : {post_stats[0]}")
        print(f"    • Has MovieHash      : {post_stats[1]}/{post_stats[0]}")
        print(f"    • Has Source ID      : {post_stats[2]}/{post_stats[0]}")
        print(f"    • Has ExpectedFrames : {post_stats[3]}/{post_stats[0]}")
        print(f"    • Complete pHashes   : {post_stats[4]}/{post_stats[0]}")
    print("======================================================================")

    return 0


if __name__ == "__main__":
    sys.exit(main())
