#!/usr/bin/env python3
"""Statistical summary reporter for Pyright type checker diagnostics."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from collections import Counter
from typing import Any

DEFAULT_TOP_FILES_COUNT: int = 15
PYRIGHT_AUDIT_CMD: list[str] = ["uv", "run", "pyright", "--outputjson"]
RULE_COLUMN_WIDTH: int = 32


def collect_diagnostics() -> list[dict[str, Any]]:
    """Execute pyright in JSON mode and collect diagnostics."""
    proc = subprocess.run(
        PYRIGHT_AUDIT_CMD,
        capture_output=True,
        text=True,
    )
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        sys.stderr.write("Failed to parse Pyright JSON output\n")
        sys.exit(1)

    diagnostics: list[dict[str, Any]] = data.get("generalDiagnostics", [])
    return diagnostics


def print_summary(diagnostics: list[dict[str, Any]], top_count: int = DEFAULT_TOP_FILES_COUNT) -> None:
    """Format and print structured diagnostic distribution."""
    total = len(diagnostics)
    cwd = os.getcwd()

    print(f"\n🔍 Pyright Type Check Audit: {total} total diagnostic(s)\n")

    by_rule = Counter(d.get("rule", "other") for d in diagnostics)
    print("📊 Distribution by Diagnostic Rule:")
    for rule, count in by_rule.most_common():
        print(f"  • {rule:<{RULE_COLUMN_WIDTH}} : {count:3d}")

    by_file = Counter(d.get("file", "unknown") for d in diagnostics)
    print("\n📁 Top Files with Type Mismatches:")
    for filepath, count in by_file.most_common(top_count):
        try:
            rel = os.path.relpath(filepath, cwd)
        except ValueError:
            rel = filepath
        print(f"  • {count:2d} err  {rel}")

    print()


def main() -> None:
    """CLI entrypoint."""
    diagnostics = collect_diagnostics()
    print_summary(diagnostics)


if __name__ == "__main__":
    main()
