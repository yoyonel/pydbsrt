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
from pathlib import Path


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


def parse_python_coverage(python_json_paths: list[Path]) -> dict:
    """Parse and merge one or more pytest-cov json outputs by unioning executed_lines."""
    merged_files: dict[str, dict] = {}
    for p in python_json_paths:
        if not p.exists():
            continue
        with open(p, encoding="utf-8") as f:
            data = json.load(f)
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
    if percent >= 80.0:
        return "🟢 High"
    elif percent >= 60.0:
        return "🟡 Good"
    elif percent >= 40.0:
        return "🟠 Moderate"
    else:
        return "🔴 Low"


def generate_markdown(
    rust_cov: dict,
    py_cov: dict,
    modified_files: list[str],
) -> str:
    md = []
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

    # Modified files section
    all_files = {}
    if rust_cov:
        all_files.update(rust_cov.get("files", {}))
    if py_cov:
        all_files.update(py_cov.get("files", {}))

    matched_modified = []
    for m in modified_files:
        if m in all_files:
            matched_modified.append((m, all_files[m]))
        else:
            # Try partial matching
            for k, v in all_files.items():
                if k.endswith(m) or m.endswith(k):
                    matched_modified.append((m, v))
                    break

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


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate unified coverage report.")
    parser.add_argument("--rust-json", type=Path, default=Path("coverage-rust.json"))
    parser.add_argument("--python-json", type=Path, default=Path("coverage-python.json"))
    parser.add_argument("--python-db-json", type=Path, default=Path("coverage-python-db.json"))
    parser.add_argument("--base-ref", type=str, default="origin/develop")
    parser.add_argument("--output-md", type=Path, default=None)
    args = parser.parse_args()

    rust_cov = parse_rust_coverage(args.rust_json)
    py_cov = parse_python_coverage([args.python_json, args.python_db_json])
    modified = get_modified_files(args.base_ref)

    report_md = generate_markdown(rust_cov, py_cov, modified)

    if args.output_md:
        args.output_md.write_text(report_md, encoding="utf-8")
        print(f"Coverage report written to {args.output_md}")

    # Output to stdout
    print(report_md)

    # Append to GITHUB_STEP_SUMMARY if available
    step_summary = os.getenv("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as f:
            f.write("\n" + report_md + "\n")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
