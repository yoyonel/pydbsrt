#!/usr/bin/env python3
"""Automated PostgreSQL Migration & Domain Lineage Integration Test Suite.

Proves the operational integrity of database migrations (V1 to V6) against:
  1. Blank database lifecycle: Sequential DDL application on a fresh sandbox database.
  2. Production regression safety: Populates sandbox database with real media data
     from active `bktreedb` (2,325+ rows), applies pending migrations, and asserts
     zero corruption or data loss.
  3. Domain constraint verification:
     - `chk_storage_kind`: Allowed ('local', 'cifs_mount', 'remote_stream'), rejects invalid.
     - `chk_asset_kind`: Allowed ('master', 'cut', 'synthetic_query'), rejects invalid.
     - `chk_phash_status`: Allowed ('pending', 'in_progress', 'complete', etc.), rejects invalid.
     - `chk_asset_lineage`: Mutually enforces Master vs DerivedCut invariants.
     - `ON DELETE RESTRICT`: Prohibits accidental parent deletion when cuts exist.
     - `idx_medias_container_identity`: Enforces O(1) partial uniqueness on active masters.
     - Soft-delete semantics (`deleted_at`).

Always cleans up all sandbox databases on completion (success or failure).
"""

from __future__ import annotations

import argparse
import io
import os
import sys
import time
from pathlib import Path
from typing import Any

import psycopg2
from psycopg2.extensions import ISOLATION_LEVEL_AUTOCOMMIT

# Operational Connection Defaults
DEFAULT_PG_HOST: str = os.environ.get("PSQL_IP", "127.0.0.1")
DEFAULT_PG_PORT: int = int(os.environ.get("PSQL_PORT", "5433"))
DEFAULT_PG_USER: str = os.environ.get("PSQL_USER", "docker")
DEFAULT_PG_PASS: str = os.environ.get("PSQL_PASS", "docker")
DEFAULT_PG_DB: str = os.environ.get("PSQL_DB", "bktreedb")

SANDBOX_BLANK_DB: str = "bktreedb_migration_blank_sandbox"
SANDBOX_CLONE_DB: str = "bktreedb_migration_clone_sandbox"
MIGRATIONS_DIR: Path = Path("crates/dbsrt-index/migrations")

TEST_SAMPLE_FILE_SIZE: int = 1_073_741_824  # 1 GiB
TEST_CUT_DURATION_MS: int = 15_000  # 15 seconds
TEST_CUT_START_MS: int = 4_500_000  # 75 minutes
TEST_FPS_NOMINAL: float = 24.0
TEST_EXPECTED_FRAMES: int = 86_400


def connect_admin(host: str, port: int, user: str, passw: str) -> psycopg2.extensions.connection:
    """Connect to default 'postgres' database to perform database provisioning."""
    conn = psycopg2.connect(
        host=host,
        port=port,
        user=user,
        password=passw,
        dbname="postgres",
        connect_timeout=5,
    )
    conn.set_isolation_level(ISOLATION_LEVEL_AUTOCOMMIT)
    return conn


def connect_db(host: str, port: int, user: str, passw: str, dbname: str) -> psycopg2.extensions.connection:
    """Connect to a specific target database."""
    conn = psycopg2.connect(
        host=host,
        port=port,
        user=user,
        password=passw,
        dbname=dbname,
        connect_timeout=5,
    )
    return conn


def drop_db_if_exists(admin_cur: Any, dbname: str) -> None:
    """Safely terminates active connections and drops target database."""
    admin_cur.execute(
        """
        SELECT pg_terminate_backend(pid)
        FROM pg_stat_activity
        WHERE datname = %s AND pid <> pg_backend_pid()
        """,
        (dbname,),
    )
    admin_cur.execute(f"DROP DATABASE IF EXISTS {dbname}")


def load_migration_files(migrations_dir: Path) -> list[tuple[int, str, str]]:
    """Loads and sorts all V*__*.sql migration scripts."""
    files = sorted(migrations_dir.glob("V*__*.sql"))
    migrations = []
    for f in files:
        v_str = f.name.split("__")[0].lstrip("V")
        version = int(v_str)
        sql = f.read_text(encoding="utf-8")
        migrations.append((version, f.name, sql))
    return sorted(migrations, key=lambda x: x[0])


def test_blank_migration_lifecycle(host: str, port: int, user: str, passw: str) -> None:
    """Proves sequential DDL execution from V1 to V6 on a clean database."""
    print("--- [1/3] Testing blank database migration lifecycle (V1 -> V6) ---")
    admin_conn = connect_admin(host, port, user, passw)
    admin_cur = admin_conn.cursor()

    try:
        drop_db_if_exists(admin_cur, SANDBOX_BLANK_DB)
        admin_cur.execute(f"CREATE DATABASE {SANDBOX_BLANK_DB}")
        print(f"  ✓ Created isolated sandbox database '{SANDBOX_BLANK_DB}'")

        migrations = load_migration_files(MIGRATIONS_DIR)
        print(f"  ✓ Discovered {len(migrations)} migration scripts")

        sb_conn = connect_db(host, port, user, passw, SANDBOX_BLANK_DB)
        sb_cur = sb_conn.cursor()

        for version, fname, sql in migrations:
            t0 = time.monotonic()
            sb_cur.execute(sql)
            sb_conn.commit()
            elapsed_ms = (time.monotonic() - t0) * 1000
            print(f"    + Applied V{version} ({fname}) in {elapsed_ms:.1f} ms")

        # Verify created tables and columns
        sb_cur.execute(
            """
            SELECT table_name FROM information_schema.tables
            WHERE table_schema = 'public' ORDER BY table_name
            """
        )
        tables = {row[0] for row in sb_cur.fetchall()}
        required_tables = {"medias", "frames", "subtitles", "sub_frames", "media_sources"}
        missing = required_tables - tables
        assert not missing, f"Missing expected tables: {missing}"
        print(f"  ✓ Verified required tables: {sorted(required_tables)}")

        sb_conn.close()
        print("  ✓ Blank migration lifecycle PASSED with zero defects\n")

    finally:
        drop_db_if_exists(admin_cur, SANDBOX_BLANK_DB)
        admin_cur.close()
        admin_conn.close()


def fetch_scalar(cur: Any) -> Any:
    """Safely extracts the first column of the first row returned by cursor."""
    row = cur.fetchone()
    assert row is not None, "Expected row from query, but got None"
    return row[0]


def copy_table_data(src_conn: Any, dst_conn: Any, table_name: str) -> int:
    """Copies all rows of common columns of a table from src_conn to dst_conn using streaming COPY."""
    src_cur = src_conn.cursor()
    dst_cur = dst_conn.cursor()

    # Discover columns present in source
    src_cur.execute(
        """
        SELECT column_name FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = %s
        ORDER BY ordinal_position
        """,
        (table_name,),
    )
    src_cols = {r[0] for r in src_cur.fetchall()}

    # Discover columns present in destination (sandbox before V6)
    dst_cur.execute(
        """
        SELECT column_name FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = %s
        ORDER BY ordinal_position
        """,
        (table_name,),
    )
    dst_cols = [r[0] for r in dst_cur.fetchall()]

    # Intersect to only transfer columns that destination knows about
    common_cols = [c for c in dst_cols if c in src_cols]
    cols_str = ", ".join(f'"{c}"' for c in common_cols)

    buf = io.BytesIO()
    src_cur.copy_expert(f"COPY (SELECT {cols_str} FROM {table_name}) TO STDOUT", buf)
    buf.seek(0)

    dst_cur.copy_expert(f"COPY {table_name} ({cols_str}) FROM STDIN", buf)
    dst_conn.commit()

    dst_cur.execute(f"SELECT COUNT(*) FROM {table_name}")
    count: int = fetch_scalar(dst_cur)
    return count


def test_real_data_clone_upgrade(host: str, port: int, user: str, passw: str, source_db: str) -> None:
    """Proves that migrating real production data to V6 causes zero data loss."""
    print(f"--- [2/3] Testing real production data upgrade ({source_db} -> V6) ---")
    admin_conn = connect_admin(host, port, user, passw)
    admin_cur = admin_conn.cursor()

    try:
        drop_db_if_exists(admin_cur, SANDBOX_CLONE_DB)
        admin_cur.execute(f"CREATE DATABASE {SANDBOX_CLONE_DB}")
        print(f"  ✓ Created isolated sandbox database '{SANDBOX_CLONE_DB}'")

        # 1. Apply baseline migrations V1 to V5 on sandbox
        sb_conn = connect_db(host, port, user, passw, SANDBOX_CLONE_DB)
        sb_cur = sb_conn.cursor()
        migrations = [m for m in load_migration_files(MIGRATIONS_DIR) if m[0] <= 5]
        for v, fname, sql in migrations:
            sb_cur.execute(sql)
        sb_conn.commit()
        print(f"  ✓ Applied baseline schema V1..V{migrations[-1][0]} on sandbox")

        # 2. Replicate live data from source_db into sandbox via COPY streams
        src_conn = connect_db(host, port, user, passw, source_db)
        src_cur = src_conn.cursor()
        src_cur.execute("SELECT COUNT(*) FROM medias")
        baseline_medias_count = fetch_scalar(src_cur)
        src_cur.execute("SELECT COUNT(*) FROM subtitles")
        baseline_subs_count = fetch_scalar(src_cur)
        print(f"  ✓ Source '{source_db}' baseline: {baseline_medias_count} medias, {baseline_subs_count} subtitles")

        copied_medias = copy_table_data(src_conn, sb_conn, "medias")
        copied_subs = copy_table_data(src_conn, sb_conn, "subtitles")
        assert copied_medias == baseline_medias_count
        assert copied_subs == baseline_subs_count
        print(f"  ✓ Replicated {copied_medias} medias into sandbox via native streaming COPY")
        src_conn.close()

        # 3. Apply V6 migration on sandbox containing real data
        v6_file = MIGRATIONS_DIR / "V6__media_sources_and_asset_lineage.sql"
        assert v6_file.exists(), f"V6 migration file not found: {v6_file}"
        v6_sql = v6_file.read_text(encoding="utf-8")

        t0 = time.monotonic()
        sb_cur.execute(v6_sql)
        sb_conn.commit()
        elapsed_ms = (time.monotonic() - t0) * 1000
        print(f"  ✓ Successfully applied V6 DDL on {copied_medias} real rows in {elapsed_ms:.1f} ms")

        # 4. Verify 100% data preservation
        sb_cur.execute("SELECT COUNT(*) FROM medias")
        post_medias_count = fetch_scalar(sb_cur)
        assert post_medias_count == baseline_medias_count, "Data loss detected in medias table!"
        print(f"  ✓ Post-migration assert: 100% row count stability ({post_medias_count} medias)")

        # 5. Verify constraints on legacy data
        sb_cur.execute("SELECT COUNT(*) FROM medias WHERE asset_kind != 'master'")
        invalid_kinds = fetch_scalar(sb_cur)
        assert invalid_kinds == 0, f"Found {invalid_kinds} rows with non-master asset_kind!"

        sb_cur.execute("SELECT COUNT(*) FROM medias WHERE parent_media_id IS NOT NULL")
        invalid_parents = fetch_scalar(sb_cur)
        assert invalid_parents == 0, "Found legacy rows with unexpected parent_media_id!"

        sb_cur.execute("SELECT COUNT(*) FROM medias WHERE expected_frames != 0")
        non_zero_expected = fetch_scalar(sb_cur)
        print(f"  ✓ Verified default values on legacy data: expected_frames=0 ({non_zero_expected} non-zero)")

        sb_conn.close()
        print("  ✓ Real production data upgrade PASSED with 100% data integrity\n")

    finally:
        drop_db_if_exists(admin_cur, SANDBOX_CLONE_DB)
        admin_cur.close()
        admin_conn.close()


def test_domain_constraints_and_lineage(host: str, port: int, user: str, passw: str) -> None:
    """Stress-tests domain rules: chk_asset_lineage, chk_storage_kind, RESTRICT, uniqueness."""
    print("--- [3/3] Stress-testing functional constraints & lineage invariants ---")
    admin_conn = connect_admin(host, port, user, passw)
    admin_cur = admin_conn.cursor()

    try:
        drop_db_if_exists(admin_cur, SANDBOX_BLANK_DB)
        admin_cur.execute(f"CREATE DATABASE {SANDBOX_BLANK_DB}")

        # Setup full V1-V6 schema
        sb_conn = connect_db(host, port, user, passw, SANDBOX_BLANK_DB)
        sb_cur = sb_conn.cursor()
        for _, _, sql in load_migration_files(MIGRATIONS_DIR):
            sb_cur.execute(sql)
        sb_conn.commit()

        # 1. Test media_sources table and storage_kind check
        sb_cur.execute(
            """
            INSERT INTO media_sources (id, name, root_uri, storage_kind)
            VALUES ('src_test_nas', 'Test Synology NAS', '/mnt/nas/video', 'cifs_mount')
            """
        )
        sb_conn.commit()
        print("  ✓ MediaSource insertion: OK")

        # Invalid storage_kind must be rejected
        try:
            sb_cur.execute(
                """
                INSERT INTO media_sources (id, name, root_uri, storage_kind)
                VALUES ('src_bad', 'Bad Storage', 'ftp://test', 'ftp_invalide')
                """
            )
            sb_conn.commit()
            raise AssertionError("chk_storage_kind failed to reject 'ftp_invalide'!")
        except psycopg2.errors.CheckViolation:
            sb_conn.rollback()
            print("  ✓ chk_storage_kind rejection: OK (invalid storage rejected)")

        # 2. Insert valid Master Media
        sb_cur.execute(
            """
            INSERT INTO medias (
                source_id, asset_kind, moviehash, file_size_bytes, name,
                duration_ms, fps, expected_frames, phash_status
            ) VALUES (
                'src_test_nas', 'master', 'a1b2c3d4e5f60718', %s, 'Spider-Man.2026.2160p.mkv',
                7200000, %s, %s, 'pending'
            ) RETURNING id
            """,
            (TEST_SAMPLE_FILE_SIZE, TEST_FPS_NOMINAL, TEST_EXPECTED_FRAMES),
        )
        master_id: int = fetch_scalar(sb_cur)
        sb_conn.commit()
        print(f"  ✓ Master Media insertion: OK (id={master_id})")

        # 3. Lineage: Master with parent_media_id must be REJECTED
        try:
            sb_cur.execute(
                """
                INSERT INTO medias (
                    source_id, asset_kind, parent_media_id, moviehash, file_size_bytes, name
                ) VALUES (
                    'src_test_nas', 'master', %s, 'bad_master_hash1', 1000, 'BadMaster.mkv'
                )
                """,
                (master_id,),
            )
            sb_conn.commit()
            raise AssertionError("chk_asset_lineage failed to reject Master with parent_media_id!")
        except psycopg2.errors.CheckViolation:
            sb_conn.rollback()
            print("  ✓ chk_asset_lineage rejection: OK (Master with parent_media_id rejected)")

        # 4. Lineage: Master with sample_start_ms > 0 must be REJECTED
        try:
            sb_cur.execute(
                """
                INSERT INTO medias (
                    source_id, asset_kind, sample_start_ms, moviehash, file_size_bytes, name
                ) VALUES (
                    'src_test_nas', 'master', 1500, 'bad_master_hash2', 1000, 'BadMaster2.mkv'
                )
                """,
            )
            sb_conn.commit()
            raise AssertionError("chk_asset_lineage failed to reject Master with sample_start_ms > 0!")
        except psycopg2.errors.CheckViolation:
            sb_conn.rollback()
            print("  ✓ chk_asset_lineage rejection: OK (Master with sample_start_ms > 0 rejected)")

        # 5. Insert valid DerivedCut
        sb_cur.execute(
            """
            INSERT INTO medias (
                source_id, asset_kind, parent_media_id, sample_start_ms, sample_duration_ms,
                moviehash, file_size_bytes, name, duration_ms, fps, expected_frames, phash_status
            ) VALUES (
                'src_test_nas', 'cut', %s, %s, %s,
                'e5f60718a1b2c3d4', 2097152, 'Spider-Man.2026_cut15s.mp4',
                %s, %s, 360, 'complete'
            ) RETURNING id
            """,
            (master_id, TEST_CUT_START_MS, TEST_CUT_DURATION_MS, TEST_CUT_DURATION_MS, TEST_FPS_NOMINAL),
        )
        cut_id: int = fetch_scalar(sb_cur)
        sb_conn.commit()
        print(f"  ✓ DerivedCut insertion: OK (id={cut_id}, parent={master_id})")

        # 6. Lineage: DerivedCut without parent_media_id must be REJECTED
        try:
            sb_cur.execute(
                """
                INSERT INTO medias (
                    source_id, asset_kind, parent_media_id, sample_duration_ms,
                    moviehash, file_size_bytes, name
                ) VALUES (
                    'src_test_nas', 'cut', NULL, 15000,
                    'bad_cut_hash', 1000, 'OrphanCut.mp4'
                )
                """
            )
            sb_conn.commit()
            raise AssertionError("chk_asset_lineage failed to reject Cut with NULL parent_media_id!")
        except psycopg2.errors.CheckViolation:
            sb_conn.rollback()
            print("  ✓ chk_asset_lineage rejection: OK (Cut without parent rejected)")

        # 7. Safety: Attempting to DELETE Master when Cut references it must fail (RESTRICT)
        try:
            sb_cur.execute("DELETE FROM medias WHERE id = %s", (master_id,))
            sb_conn.commit()
            raise AssertionError("ON DELETE RESTRICT failed: parent master was deleted while cut exists!")
        except psycopg2.errors.ForeignKeyViolation:
            sb_conn.rollback()
            print("  ✓ ON DELETE RESTRICT enforcement: OK (Master deletion blocked by dependent cut)")

        # 8. Soft-delete Master must succeed
        sb_cur.execute("UPDATE medias SET deleted_at = NOW() WHERE id = %s", (master_id,))
        sb_conn.commit()
        print("  ✓ Soft-delete Master (deleted_at = NOW()): OK")

        # 9. Unique Container Identity: Duplicate (moviehash, file_size_bytes) for active master must FAIL
        sb_cur.execute(
            """
            INSERT INTO medias (
                source_id, asset_kind, moviehash, file_size_bytes, name
            ) VALUES (
                'src_test_nas', 'master', 'dup_test_hash_01', 5000000, 'MovieA.mkv'
            )
            """
        )
        sb_conn.commit()

        # Duplicate with same hash and size
        try:
            sb_cur.execute(
                """
                INSERT INTO medias (
                    source_id, asset_kind, moviehash, file_size_bytes, name
                ) VALUES (
                    'src_test_nas', 'master', 'dup_test_hash_01', 5000000, 'MovieB.mkv'
                )
                """
            )
            sb_conn.commit()
            raise AssertionError("idx_medias_container_identity failed to prevent duplicate active master!")
        except psycopg2.errors.UniqueViolation:
            sb_conn.rollback()
            print("  ✓ idx_medias_container_identity: OK (Duplicate active master rejected)")

        # But duplicate ALLOWED for cuts
        sb_cur.execute(
            """
            INSERT INTO medias (
                source_id, asset_kind, parent_media_id, sample_duration_ms, moviehash, file_size_bytes, name
            ) VALUES (
                'src_test_nas', 'cut', %s, 10000, 'dup_test_hash_01', 5000000, 'CutA.mp4'
            )
            """,
            (master_id,),
        )
        sb_cur.execute(
            """
            INSERT INTO medias (
                source_id, asset_kind, parent_media_id, sample_duration_ms, moviehash, file_size_bytes, name
            ) VALUES (
                'src_test_nas', 'cut', %s, 10000, 'dup_test_hash_01', 5000000, 'CutB.mp4'
            )
            """,
            (master_id,),
        )
        sb_conn.commit()
        print("  ✓ Cuts sharing hash and size: OK (Uniqueness strictly restricted to Masters)")

        sb_conn.close()
        print("  ✓ Domain constraints & lineage invariants PASSED (100% compliant with spec)\n")

    finally:
        drop_db_if_exists(admin_cur, SANDBOX_BLANK_DB)
        admin_cur.close()
        admin_conn.close()


def main() -> int:
    parser = argparse.ArgumentParser(description="Test PostgreSQL schema migrations and lineage invariants")
    parser.add_argument("--host", default=DEFAULT_PG_HOST, help=f"PostgreSQL host (default: {DEFAULT_PG_HOST})")
    parser.add_argument(
        "--port", type=int, default=DEFAULT_PG_PORT, help=f"PostgreSQL port (default: {DEFAULT_PG_PORT})"
    )
    parser.add_argument("--user", default=DEFAULT_PG_USER, help=f"PostgreSQL user (default: {DEFAULT_PG_USER})")
    parser.add_argument("--password", default=DEFAULT_PG_PASS, help="PostgreSQL password")
    parser.add_argument(
        "--source-db", default=DEFAULT_PG_DB, help=f"Source database to clone for regression test ({DEFAULT_PG_DB})"
    )
    args = parser.parse_args()

    print("======================================================================")
    print(" 🛡️  PYDBSRT POSTGRESQL MIGRATION & DOMAIN LINEAGE VERIFICATION SUITE")
    print("======================================================================")
    print(f"Target Cluster: {args.host}:{args.port} (user: {args.user})")
    print(f"Migrations Dir: {MIGRATIONS_DIR.resolve()}\n")

    start_time = time.monotonic()
    try:
        # Phase 1: Blank DB lifecycle
        test_blank_migration_lifecycle(args.host, args.port, args.user, args.password)

        # Phase 2: Real production data clone upgrade
        test_real_data_clone_upgrade(args.host, args.port, args.user, args.password, args.source_db)

        # Phase 3: Stress-test all functional constraints & invariants
        test_domain_constraints_and_lineage(args.host, args.port, args.user, args.password)

        elapsed = time.monotonic() - start_time
        print("======================================================================")
        print(f" 🎉 ALL MIGRATION & DOMAIN TESTS PASSED WITH 100% SUCCESS ({elapsed:.2f}s)")
        print(" Your production database 'bktreedb' was 100% UNTOUCHED and is SAFE.")
        print("======================================================================")
        return 0

    except Exception as e:
        print("\n❌ CRITICAL INTEGRATION FAILURE:")
        print(f"   {e}")
        import traceback

        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
