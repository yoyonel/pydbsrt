#!/usr/bin/env python3
"""
Automated Coverage Aggregator & PR Reporter for pydbsrt.

Combines Rust (cargo-llvm-cov json) and Python (pytest-cov json) coverage data.
Generates a markdown report for GitHub Actions ($GITHUB_STEP_SUMMARY & PR comments).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

MIGRATION_PATH_PATTERNS: tuple[str, ...] = (
    "crates/dbsrt-index/migrations/",
    "migrations/",
)
SQL_FILE_EXTENSION: str = ".sql"
MIGRATION_VERSION_PREFIX: str = "/V"
MIGRATION_SEPARATOR: str = "__"

COVERAGE_HIGH_THRESHOLD: float = 80.0
COVERAGE_GOOD_THRESHOLD: float = 60.0
COVERAGE_MODERATE_THRESHOLD: float = 40.0


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


def detect_db_migrations(modified_files: list[str]) -> list[str]:
    """Identify if any modified files are database schema migrations."""
    migrations = []
    for f in modified_files:
        normalized = f.replace("\\", "/")
        if any(pat in normalized for pat in MIGRATION_PATH_PATTERNS) and normalized.endswith(SQL_FILE_EXTENSION):
            migrations.append(f)
        elif (
            MIGRATION_VERSION_PREFIX in normalized
            and MIGRATION_SEPARATOR in normalized
            and normalized.endswith(SQL_FILE_EXTENSION)
        ):
            migrations.append(f)
    return sorted(list(set(migrations)))


def format_migration_alert(migrations: list[str]) -> str:
    """Format an actionable GitHub alert callout for detected DB migrations."""
    if not migrations:
        return ""
    lines = [
        "> [!IMPORTANT]",
        "> ### 🐘 Database Schema Migrations Detected",
        ">",
        "> This PR introduces or modifies database migrations:",
    ]
    for mig in migrations:
        lines.append(f"> - `{mig}`")
    lines.extend(
        [
            ">",
            "> **Operational & Deployment Checklist:**",
            "> - [ ] Run pending migrations: `task db:migrate`",
            "> - [ ] Verify migration status: `task db:status`",
            "> - [ ] Verify DDL safety & PG 15/17 compatibility: `task db:lint:safety`",
            "> - [ ] (Optional) Validate on remote host if online: `task rpi4:db:status`",
            "",
        ]
    )
    return "\n".join(lines)


def get_modified_files(base_ref: str = "origin/develop") -> list[str]:
    """Retrieve list of files modified in current branch compared to base."""
    try:
        res = subprocess.run(
            ["git", "diff", "--name-only", f"{base_ref}...HEAD"],
            capture_output=True,
            text=True,
            check=False,
        )
        if res.returncode == 0 and res.stdout.strip():
            return [line.strip() for line in res.stdout.splitlines() if line.strip()]
    except Exception:
        pass
    return []


def parse_rust_coverage(rust_json_path: Path) -> dict:
    """Parse cargo-llvm-cov json output."""
    if not rust_json_path.exists():
        return {}

    with open(rust_json_path, encoding="utf-8") as f:
        data = json.load(f)

    if not data.get("data"):
        return {}

    item = data["data"][0]
    totals = item.get("totals", {}).get("lines", {})
    files = item.get("files", [])

    # Group by crate
    crates: dict[str, dict] = {
        "dbsrt-core": {"total": 0, "covered": 0},
        "dbsrt-index": {"total": 0, "covered": 0},
        "dbsrt-cli": {"total": 0, "covered": 0},
        "dbsrt-server": {"total": 0, "covered": 0},
    }
    file_map: dict[str, dict] = {}

    for f_info in files:
        fname = f_info.get("filename", "")
        # normalize to relative path
        rel_path = fname
        if "crates/" in fname:
            rel_path = "crates/" + fname.split("crates/", 1)[1]

        lines_info = f_info.get("summary", {}).get("lines", {})
        count = lines_info.get("count", 0)
        covered = lines_info.get("covered", 0)
        percent = lines_info.get("percent", 0.0)

        file_map[rel_path] = {
            "total": count,
            "covered": covered,
            "percent": percent,
        }

        for crate_name in crates:
            if f"crates/{crate_name}/" in rel_path or f"/{crate_name}/" in rel_path:
                crates[crate_name]["total"] += count
                crates[crate_name]["covered"] += covered

    return {
        "total_lines": totals.get("count", 0),
        "covered_lines": totals.get("covered", 0),
        "percent": totals.get("percent", 0.0),
        "crates": crates,
        "files": file_map,
    }


def _merge_single_coverage_file(data: dict, merged_files: dict[str, dict]) -> None:
    """Helper to merge coverage statements from a single file payload."""
    for fname, f_info in data.get("files", {}).items():
        summary = f_info.get("summary", {})
        count = summary.get("num_statements", 0)
        exec_lines = set(f_info.get("executed_lines", []))
        if fname not in merged_files:
            merged_files[fname] = {
                "count": count,
                "executed": exec_lines,
                "max_covered": summary.get("covered_lines", 0),
            }
        else:
            merged_files[fname]["count"] = max(merged_files[fname]["count"], count)
            merged_files[fname]["executed"].update(exec_lines)
            merged_files[fname]["max_covered"] = max(
                merged_files[fname]["max_covered"],
                summary.get("covered_lines", 0),
            )


def parse_python_coverage(python_json_paths: list[Path]) -> dict:
    """Parse and merge one or more pytest-cov json outputs by unioning executed_lines."""
    merged_files: dict[str, dict] = {}
    for p in python_json_paths:
        if not p.exists():
            continue
        with open(p, encoding="utf-8") as f:
            data = json.load(f)
        _merge_single_coverage_file(data, merged_files)

    if not merged_files:
        return {}

    file_map: dict[str, dict] = {}
    groups: dict[str, dict] = {
        "src/pydbsrt/tools": {"total": 0, "covered": 0},
        "src/pydbsrt/services": {"total": 0, "covered": 0},
        "src/pydbsrt/models": {"total": 0, "covered": 0},
        "src/pydbsrt/applications": {"total": 0, "covered": 0},
    }

    total_stmts = 0
    total_covered = 0

    for fname, info in merged_files.items():
        count = info["count"]
        covered = max(len(info["executed"]), info["max_covered"])
        covered = min(covered, count)
        percent = (covered / count * 100.0) if count > 0 else 100.0

        total_stmts += count
        total_covered += covered

        file_map[fname] = {
            "total": count,
            "covered": covered,
            "percent": percent,
        }

        for grp in groups:
            if fname.startswith(grp):
                groups[grp]["total"] += count
                groups[grp]["covered"] += covered

    overall_pct = (total_covered / total_stmts * 100.0) if total_stmts > 0 else 0.0
    return {
        "total_lines": total_stmts,
        "covered_lines": total_covered,
        "percent": overall_pct,
        "groups": groups,
        "files": file_map,
    }


def status_badge(percent: float) -> str:
    if percent >= COVERAGE_HIGH_THRESHOLD:
        return "🟢 High"
    elif percent >= COVERAGE_GOOD_THRESHOLD:
        return "🟡 Good"
    elif percent >= COVERAGE_MODERATE_THRESHOLD:
        return "🟠 Moderate"
    else:
        return "🔴 Low"


def get_matched_modified_files(
    modified_files: list[str],
    all_files: dict[str, dict],
) -> list[tuple[str, dict]]:
    """Match git modified files against files present in coverage data."""
    matched = []
    for m in modified_files:
        if m in all_files:
            matched.append((m, all_files[m]))
        else:
            for k, v in all_files.items():
                if k.endswith(m) or m.endswith(k):
                    matched.append((m, v))
                    break
    return matched


def check_modified_coverage_gate(
    matched_modified: list[tuple[str, dict]],
    min_threshold: float = COVERAGE_GOOD_THRESHOLD,
) -> list[tuple[str, float]]:
    """Return list of modified files failing the minimum coverage threshold (< 60.0% / Good)."""
    return [(fname, finfo["percent"]) for fname, finfo in matched_modified if finfo["percent"] < min_threshold]


def generate_markdown(
    rust_cov: dict,
    py_cov: dict,
    modified_files: list[str],
    min_threshold: float = COVERAGE_GOOD_THRESHOLD,
) -> str:
    md = []

    migrations = detect_db_migrations(modified_files)
    if migrations:
        md.append(format_migration_alert(migrations))

    # Aggregate all files to inspect PR modified coverage
    all_files = {}
    if rust_cov:
        all_files.update(rust_cov.get("files", {}))
    if py_cov:
        all_files.update(py_cov.get("files", {}))

    matched_modified = get_matched_modified_files(modified_files, all_files)
    failed_gate = check_modified_coverage_gate(matched_modified, min_threshold)

    if failed_gate:
        md.append("> [!CAUTION]")
        md.append("> ### ❌ Coverage Quality Gate Failed")
        md.append(
            f"> The following PR modified file(s) are below the minimum required coverage of **{min_threshold:.1f}%** (Good or High):"
        )
        for fname, pct in failed_gate:
            md.append(f"> - `{fname}`: **{pct:.2f}%** ({status_badge(pct)})")
        md.append("")

    md.append("### 📊 Test Coverage Report (Rust & Python)\n")

    md.append("| Target / Component | Lines / Stmts | Covered | Coverage Rate | Status |")
    md.append("| :--- | :---: | :---: | :---: | :---: |")

    # Rust crates
    if rust_cov:
        for cname, cdata in rust_cov.get("crates", {}).items():
            tot = cdata["total"]
            cov = cdata["covered"]
            pct = (cov / tot * 100.0) if tot > 0 else 0.0
            badge = status_badge(pct)
            md.append(f"| **🦀 Rust: `{cname}`** | {tot:,} | {cov:,} | **{pct:.2f}%** | {badge} |")

        r_tot = rust_cov["total_lines"]
        r_cov = rust_cov["covered_lines"]
        r_pct = rust_cov["percent"]
        md.append(
            f"| **🦀 Total Rust Workspace** | **{r_tot:,}** | **{r_cov:,}** | **{r_pct:.2f}%** | {status_badge(r_pct)} |"
        )

    # Python
    if py_cov:
        p_tot = py_cov["total_lines"]
        p_cov = py_cov["covered_lines"]
        p_pct = py_cov["percent"]
        md.append(
            f"| **🐍 Total Python (`src/pydbsrt`)** | **{p_tot:,}** | **{p_cov:,}** | **{p_pct:.2f}%** | {status_badge(p_pct)} |"
        )

    # Combined
    if rust_cov and py_cov:
        comb_tot = rust_cov["total_lines"] + py_cov["total_lines"]
        comb_cov = rust_cov["covered_lines"] + py_cov["covered_lines"]
        comb_pct = (comb_cov / comb_tot * 100.0) if comb_tot > 0 else 0.0
        md.append(
            f"| **🌐 Combined Workspace** | **{comb_tot:,}** | **{comb_cov:,}** | **{comb_pct:.2f}%** | {status_badge(comb_pct)} |"
        )

    md.append("\n---\n")

    if matched_modified:
        md.append("<details open>")
        md.append("<summary><b>🔍 PR Modified Files Coverage Details</b></summary>\n")
        md.append("| Modified File | Statements | Covered | Coverage Rate | Status |")
        md.append("| :--- | :---: | :---: | :---: | :---: |")
        for fname, finfo in matched_modified:
            tot = finfo["total"]
            cov = finfo["covered"]
            pct = finfo["percent"]
            md.append(f"| `{fname}` | {tot} | {cov} | **{pct:.2f}%** | {status_badge(pct)} |")
        md.append("</details>\n")
    elif all_files:
        # Show top files
        md.append("<details>")
        md.append("<summary><b>📁 File-by-File Coverage Breakdown</b></summary>\n")
        md.append("| File | Statements | Covered | Coverage Rate | Status |")
        md.append("| :--- | :---: | :---: | :---: | :---: |")
        for fname, finfo in sorted(all_files.items()):
            tot = finfo["total"]
            cov = finfo["covered"]
            pct = finfo["percent"]
            if tot > 0:
                md.append(f"| `{fname}` | {tot} | {cov} | **{pct:.2f}%** | {status_badge(pct)} |")
        md.append("</details>\n")

    return "\n".join(md)


def _evaluate_quality_gate(
    rust_cov: dict,
    py_cov: dict,
    modified: list[str],
    min_threshold: float,
) -> int:
    """Evaluate coverage gate on modified files."""
    all_files: dict[str, dict] = {}
    if rust_cov:
        all_files.update(rust_cov.get("files", {}))
    if py_cov:
        all_files.update(py_cov.get("files", {}))

    matched_modified = get_matched_modified_files(modified, all_files)
    failed_gate = check_modified_coverage_gate(matched_modified, min_threshold)

    if failed_gate:
        for fname, pct in failed_gate:
            print(
                f"::error file={fname},title=Coverage Quality Gate Failed::"
                f"PR modified file coverage ({pct:.2f}%) is below required threshold of "
                f"{min_threshold:.1f}% (must be at least Good >= 60.0%)."
            )
        print(
            f"\n❌ Coverage Quality Gate Failed: {len(failed_gate)} PR modified file(s) below "
            f"{min_threshold:.1f}% coverage.",
            file=sys.stderr,
        )
        return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate unified coverage report.")
    parser.add_argument("--rust-json", type=Path, default=Path("coverage-rust.json"))
    parser.add_argument("--python-json", type=Path, default=Path("coverage-python.json"))
    parser.add_argument("--python-db-json", type=Path, default=Path("coverage-python-db.json"))
    parser.add_argument("--base-ref", type=str, default="origin/develop")
    parser.add_argument("--output-md", type=Path, default=None)
    parser.add_argument(
        "--min-modified-coverage",
        type=float,
        default=COVERAGE_GOOD_THRESHOLD,
        help="Minimum coverage percent required for modified files (default: 60.0%%)",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Check presence of input coverage files and modified files without failing exit codes",
    )
    args = parser.parse_args()

    repo_root = find_repo_root()

    def _resolve(p: Path) -> Path:
        if not p.is_absolute() and not p.exists() and (repo_root / p).exists():
            return repo_root / p
        return p

    rust_path = _resolve(args.rust_json)
    py_paths = [_resolve(args.python_json), _resolve(args.python_db_json)]
    rust_cov = parse_rust_coverage(rust_path)
    py_cov = parse_python_coverage(py_paths)
    modified = get_modified_files(args.base_ref)

    if args.check:
        print(f"REPO_ROOT         : {repo_root}")
        print(f"Rust coverage     : {rust_path} (exists: {rust_path.exists()})")
        print(f"Python coverage 1 : {py_paths[0]} (exists: {py_paths[0].exists()})")
        print(f"Python coverage 2 : {py_paths[1]} (exists: {py_paths[1].exists()})")
        print(f"Modified files    : {len(modified)} files detected vs {args.base_ref}")
        return 0

    report_md = generate_markdown(rust_cov, py_cov, modified, min_threshold=args.min_modified_coverage)

    # Workflow notice if migrations detected
    migrations = detect_db_migrations(modified)
    if migrations:
        joined_files = ", ".join(f"`{m}`" for m in migrations)
        print(
            f"::notice title=🐘 Database Schema Migrations Detected::PR introduces {len(migrations)} database migration(s): {joined_files}. Verify 'task db:migrate' and PG 15/17 compatibility before deploying."
        )

    if args.output_md:
        out_path = args.output_md if args.output_md.is_absolute() else repo_root / args.output_md
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(report_md, encoding="utf-8")
        print(f"Coverage report written to {out_path}")

    # Output to stdout
    print(report_md)

    # Append to GITHUB_STEP_SUMMARY if available
    step_summary = os.getenv("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as f:
            f.write("\n" + report_md + "\n")

    return _evaluate_quality_gate(rust_cov, py_cov, modified, args.min_modified_coverage)


if __name__ == "__main__":
    raise SystemExit(main())
