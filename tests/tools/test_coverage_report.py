"""Unit tests for coverage aggregation, migration detection, and quality gate enforcement."""

from scripts.generate_coverage_report import (
    check_modified_coverage_gate,
    detect_db_migrations,
    format_migration_alert,
    generate_markdown,
    get_matched_modified_files,
    status_badge,
)


def test_detect_db_migrations_positive() -> None:
    modified_files = [
        "crates/dbsrt-cli/src/main.rs",
        "crates/dbsrt-index/migrations/V1__initial_schema.sql",
        "crates/dbsrt-index/migrations/V2__certified_couplings.sql",
        "README.md",
    ]
    detected = detect_db_migrations(modified_files)
    assert len(detected) == 2
    assert "crates/dbsrt-index/migrations/V1__initial_schema.sql" in detected
    assert "crates/dbsrt-index/migrations/V2__certified_couplings.sql" in detected


def test_detect_db_migrations_negative() -> None:
    modified_files = [
        "crates/dbsrt-cli/src/main.rs",
        "crates/dbsrt-index/src/lib.rs",
        "docs/index.md",
    ]
    detected = detect_db_migrations(modified_files)
    assert len(detected) == 0


def test_format_migration_alert() -> None:
    migrations = ["crates/dbsrt-index/migrations/V1__initial_schema.sql"]
    alert = format_migration_alert(migrations)
    assert "[!IMPORTANT]" in alert
    assert "Database Schema Migrations Detected" in alert
    assert "`crates/dbsrt-index/migrations/V1__initial_schema.sql`" in alert
    assert "task db:migrate" in alert
    assert "task db:status" in alert
    assert "task db:lint:safety" in alert


def test_generate_markdown_with_migrations() -> None:
    modified_files = ["crates/dbsrt-index/migrations/V1__initial_schema.sql"]
    rust_cov = {
        "total_lines": 100,
        "covered_lines": 90,
        "percent": 90.0,
        "crates": {"dbsrt-cli": {"total": 100, "covered": 90}},
        "files": {},
    }
    py_cov = {}
    md = generate_markdown(rust_cov, py_cov, modified_files)
    assert "[!IMPORTANT]" in md
    assert "Database Schema Migrations Detected" in md
    assert "### 📊 Test Coverage Report" in md


def test_status_badge() -> None:
    assert status_badge(85.0) == "🟢 High"
    assert status_badge(65.0) == "🟡 Good"
    assert status_badge(45.0) == "🟠 Moderate"
    assert status_badge(25.0) == "🔴 Low"


def test_get_matched_modified_files() -> None:
    all_files = {
        "crates/dbsrt-cli/src/main.rs": {"total": 50, "covered": 45, "percent": 90.0},
        "crates/dbsrt-cli/src/commands/migrate.rs": {"total": 40, "covered": 10, "percent": 25.0},
    }
    modified = ["crates/dbsrt-cli/src/commands/migrate.rs", "unrelated/file.txt"]
    matched = get_matched_modified_files(modified, all_files)
    assert len(matched) == 1
    assert matched[0][0] == "crates/dbsrt-cli/src/commands/migrate.rs"
    assert matched[0][1]["percent"] == 25.0


def test_check_modified_coverage_gate() -> None:
    matched = [
        ("crates/dbsrt-cli/src/main.rs", {"total": 50, "covered": 45, "percent": 90.0}),
        ("crates/dbsrt-cli/src/commands/migrate.rs", {"total": 42, "covered": 6, "percent": 14.29}),
    ]
    # Default 60.0% threshold (Good/High required)
    failed = check_modified_coverage_gate(matched, min_threshold=60.0)
    assert len(failed) == 1
    assert failed[0][0] == "crates/dbsrt-cli/src/commands/migrate.rs"
    assert failed[0][1] == 14.29


def test_generate_markdown_with_failed_gate() -> None:
    rust_cov = {
        "total_lines": 92,
        "covered_lines": 51,
        "percent": 55.43,
        "crates": {"dbsrt-cli": {"total": 92, "covered": 51}},
        "files": {
            "crates/dbsrt-cli/src/commands/migrate.rs": {"total": 42, "covered": 6, "percent": 14.29},
        },
    }
    py_cov = {}
    modified = ["crates/dbsrt-cli/src/commands/migrate.rs"]
    md = generate_markdown(rust_cov, py_cov, modified, min_threshold=60.0)
    assert "[!CAUTION]" in md
    assert "Coverage Quality Gate Failed" in md
    assert "`crates/dbsrt-cli/src/commands/migrate.rs`: **14.29%** (🔴 Low)" in md
