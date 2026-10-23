"""Unit tests for PostgreSQL migration safety & compatibility linter."""

import tempfile
from pathlib import Path

from scripts.lint_migrations import lint_migration_file


def test_lint_migration_file_valid() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        sql_file = Path(tmpdir) / "V1__initial_schema.sql"
        sql_file.write_text(
            "CREATE TABLE IF NOT EXISTS medias (id SERIAL PRIMARY KEY, name TEXT UNIQUE);\n",
            encoding="utf-8",
        )
        issues = lint_migration_file(sql_file)
        assert len(issues) == 0


def test_lint_migration_file_invalid_name() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        sql_file = Path(tmpdir) / "invalid_name.sql"
        sql_file.write_text("CREATE TABLE test (id INT);\n", encoding="utf-8")
        issues = lint_migration_file(sql_file)
        assert any("convention" in issue for issue in issues)


def test_lint_migration_file_dangerous_ddl() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        sql_file = Path(tmpdir) / "V2__drop_col.sql"
        sql_file.write_text("ALTER TABLE medias DROP COLUMN name;\n", encoding="utf-8")
        issues = lint_migration_file(sql_file)
        assert any("DROP COLUMN" in issue for issue in issues)


def test_lint_migration_file_pg16_incompatible() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        sql_file = Path(tmpdir) / "V3__pg16_func.sql"
        sql_file.write_text("SELECT ANY_VALUE(name) FROM medias;\n", encoding="utf-8")
        issues = lint_migration_file(sql_file)
        assert any("PG 15 compatibility" in issue for issue in issues)


def test_lint_migration_file_exemption() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        sql_file = Path(tmpdir) / "V4__allowed_rewrite.sql"
        sql_file.write_text(
            "-- safety-allow: table-rewrite (catalog is small)\nALTER TABLE subtitles ALTER COLUMN lang TYPE TEXT;\n",
            encoding="utf-8",
        )
        issues = lint_migration_file(sql_file)
        assert len(issues) == 0
