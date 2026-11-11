#!/usr/bin/env python3
"""Statistical summary reporter for Pyright type checker diagnostics."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

DEFAULT_TOP_FILES_COUNT: int = 15
PYRIGHT_AUDIT_CMD: list[str] = ["uv", "run", "pyright", "--outputjson"]
RULE_COLUMN_WIDTH: int = 32


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


def collect_diagnostics(repo_root: Path) -> list[dict[str, Any]]:
    """Execute pyright in JSON mode from repo root and collect diagnostics."""
    proc = subprocess.run(
        PYRIGHT_AUDIT_CMD,
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    )
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        sys.stderr.write(f"Failed to parse Pyright JSON output:\n{proc.stdout[:500]}\n")
        sys.exit(1)

    diagnostics: list[dict[str, Any]] = data.get("generalDiagnostics", [])
    return diagnostics


def print_summary(
    diagnostics: list[dict[str, Any]],
    repo_root: Path,
    top_count: int = DEFAULT_TOP_FILES_COUNT,
) -> None:
    """Format and print structured diagnostic distribution."""
    total = len(diagnostics)

    print(f"\n🔍 Pyright Type Check Audit: {total} total diagnostic(s)\n")

    by_rule = Counter(d.get("rule", "other") for d in diagnostics)
    print("📊 Distribution by Diagnostic Rule:")
    for rule, count in by_rule.most_common():
        print(f"  • {rule:<{RULE_COLUMN_WIDTH}} : {count:3d}")

    by_file = Counter(d.get("file", "unknown") for d in diagnostics)
    print("\n📁 Top Files with Type Mismatches:")
    for filepath, count in by_file.most_common(top_count):
        try:
            rel = os.path.relpath(filepath, repo_root)
        except ValueError:
            rel = filepath
        print(f"  • {count:2d} err  {rel}")

    print()


def main() -> None:
    """CLI entrypoint."""
    parser = argparse.ArgumentParser(description="Statistical summary reporter for Pyright type checker diagnostics.")
    parser.add_argument(
        "--top",
        type=int,
        default=DEFAULT_TOP_FILES_COUNT,
        help=f"Number of top files to display (default: {DEFAULT_TOP_FILES_COUNT})",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Quick check of Pyright tool availability and report summary",
    )
    args = parser.parse_args()

    repo_root = find_repo_root()
    if not shutil.which("uv"):
        print("Error: 'uv' executable not found in PATH.", file=sys.stderr)
        sys.exit(1)

    diagnostics = collect_diagnostics(repo_root)
    print_summary(diagnostics, repo_root=repo_root, top_count=args.top)


if __name__ == "__main__":
    main()
