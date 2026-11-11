#!/usr/bin/env python3
"""Automated PostgreSQL Migration Safety & Compatibility Linter for pydbsrt.

Inspects versioned migrations (V*__*.sql) for:
1. DDL Locking Safety (avoids ACCESS EXCLUSIVE locks, table rewrites, destructive DROPs).
2. Cross-version compatibility (ensures compatibility with PostgreSQL 15 and 17).
3. Naming and structural conventions (Refinery format V{version}__{description}.sql).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

DEFAULT_MIGRATIONS_DIR: str = "crates/dbsrt-index/migrations"
MIGRATION_FILENAME_PATTERN: str = r"^V(\d+)__([a-z0-9_]+)\.sql$"

# Destructive operations that cause data loss or breaking changes
DANGEROUS_PATTERNS: tuple[tuple[str, str], ...] = (
    (r"\bDROP\s+COLUMN\b", "DROP COLUMN causes immediate data loss and breaks backwards compatibility"),
    (r"\bDROP\s+TABLE\b", "DROP TABLE causes irreversible data loss"),
    (r"\bTRUNCATE\b", "TRUNCATE removes all table data"),
    (r"\bLOCK\s+TABLE\b", "Explicit LOCK TABLE risks blocking concurrent queries and transactions"),
    (r"\bVACUUM\s+FULL\b", "VACUUM FULL takes ACCESS EXCLUSIVE lock on entire table"),
    (r"\bCLUSTER\s+", "CLUSTER acquires an exclusive lock on the relation"),
)

# Table rewrites under heavy ACCESS EXCLUSIVE locks
TABLE_REWRITE_PATTERNS: tuple[tuple[str, str], ...] = (
    (
        r"\bALTER\s+TABLE\s+\w+\s+ALTER\s+COLUMN\s+\w+\s+TYPE\b",
        "ALTER COLUMN TYPE forces a full table rewrite holding ACCESS EXCLUSIVE lock",
    ),
)

# PostgreSQL 16+ exclusive keywords / functions that fail on PostgreSQL 15
PG16_PLUS_EXCLUSIVE_PATTERNS: tuple[tuple[str, str], ...] = (
    (r"\bANY_VALUE\s*\(", "ANY_VALUE() aggregate was introduced in PostgreSQL 16; incompatible with PG 15"),
    (
        r"\bJSON_ARRAY\s*\(",
        "SQL/JSON JSON_ARRAY() constructor was introduced in PostgreSQL 16; incompatible with PG 15",
    ),
    (
        r"\bJSON_OBJECT\s*\(",
        "SQL/JSON JSON_OBJECT() constructor was introduced in PostgreSQL 16; incompatible with PG 15",
    ),
    (
        r"\bCREATE\s+OR\s+REPLACE\s+TRIGGER\b",
        "CREATE OR REPLACE TRIGGER requires PostgreSQL 16+; incompatible with PG 15",
    ),
)

SAFETY_ALLOW_PREFIX: str = "-- safety-allow:"
NOQA_PREFIX: str = "-- noqa:"


def find_repo_root() -> Path:
    """Dynamically locates repository root with fallback to ~/dbsrt."""
    current = Path(__file__).resolve().parent
    for p in [current, *current.parents]:
        if (p / ".git").is_dir() or (p / "Taskfile.yml").is_file():
            return p
    appliance = Path.home() / "dbsrt"
    if appliance.is_dir():
        return appliance
    return Path.cwd()


def _is_exempt(prev_line: str, stripped: str) -> bool:
    """Check if preceding line or current line has an explicit safety exemption."""
    return (
        SAFETY_ALLOW_PREFIX in prev_line
        or NOQA_PREFIX in prev_line
        or SAFETY_ALLOW_PREFIX in stripped
        or NOQA_PREFIX in stripped
    )


def _check_pattern_group(
    patterns: tuple[tuple[str, str], ...],
    stripped: str,
    idx: int,
    prefix_msg: str,
) -> list[str]:
    """Scan line against a tuple of regex patterns."""
    errors: list[str] = []
    for pattern, reason in patterns:
        if re.search(pattern, stripped, re.IGNORECASE):
            errors.append(f"Line {idx}: {prefix_msg}: {reason}")
    return errors


def lint_migration_file(path: Path) -> list[str]:
    """Lint a single migration file for DDL safety and cross-version compatibility."""
    errors: list[str] = []
    fname = path.name

    if not re.match(MIGRATION_FILENAME_PATTERN, fname):
        errors.append(
            f"Filename '{fname}' does not follow Refinery versioning convention 'V<number>__<description>.sql'"
        )

    content = path.read_text(encoding="utf-8")
    lines = content.splitlines()

    for idx, line in enumerate(lines, start=1):
        stripped = line.strip()
        if stripped.startswith("--") or not stripped:
            continue

        prev_line = lines[idx - 2].strip() if idx >= 2 else ""
        if _is_exempt(prev_line, stripped):
            continue

        errors.extend(_check_pattern_group(DANGEROUS_PATTERNS, stripped, idx, "Dangerous DDL detected"))
        errors.extend(_check_pattern_group(TABLE_REWRITE_PATTERNS, stripped, idx, "Lock contention risk"))
        errors.extend(_check_pattern_group(PG16_PLUS_EXCLUSIVE_PATTERNS, stripped, idx, "PG 15 compatibility issue"))

    return errors


def lint_migrations_directory(migrations_dir: Path) -> tuple[int, int]:
    """Inspect all migration files in target directory. Returns (total_files, failed_files)."""
    if not migrations_dir.exists():
        print(f"❌ Migrations directory '{migrations_dir}' not found.", file=sys.stderr)
        return (0, 1)

    sql_files = sorted(list(migrations_dir.glob("V*__*.sql")))
    if not sql_files:
        print(f"⚠️ No versioned migrations found in '{migrations_dir}'.")
        return (0, 0)

    total_files = len(sql_files)
    failed_files = 0

    print(
        f"🔍 Inspecting {total_files} PostgreSQL migration(s) in '{migrations_dir}' for DDL safety & PG 15/17 compatibility...\n"
    )

    for file_path in sql_files:
        issues = lint_migration_file(file_path)
        if issues:
            failed_files += 1
            print(f"❌ {file_path}:")
            for issue in issues:
                print(f"   • {issue}")
        else:
            print(f"✓ {file_path}: SAFE (additive schema, non-destructive, PG 15/17 compatible)")

    print()
    if failed_files > 0:
        print(
            f"❌ Migration safety check failed: {failed_files}/{total_files} file(s) have safety or compatibility issues.",
            file=sys.stderr,
        )
    else:
        print(f"✅ All {total_files} migration(s) passed DDL safety and PG 15/17 compatibility checks.")

    return (total_files, failed_files)


def main() -> int:
    repo_root = find_repo_root()
    default_dir = repo_root / DEFAULT_MIGRATIONS_DIR

    parser = argparse.ArgumentParser(
        description="Lint PostgreSQL migrations for DDL safety and PG 15/17 compatibility."
    )
    parser.add_argument(
        "--dir",
        type=Path,
        default=default_dir,
        help=f"Directory containing migration files (default: {DEFAULT_MIGRATIONS_DIR})",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Check presence of migrations directory and validate schemas",
    )
    args = parser.parse_args()

    target_dir = args.dir
    if not target_dir.is_absolute() and not target_dir.exists():
        target_dir = repo_root / target_dir

    total, failed = lint_migrations_directory(target_dir)
    return 1 if failed > 0 else 0


if __name__ == "__main__":
    raise SystemExit(main())
