#!/usr/bin/env python3
"""
Automated Rust Code Complexity & Function Size Gate for pydbsrt.

Enforces strict architectural bounds on all Rust functions across `crates/`:
1. Maximum Function LOC: 150 lines (Target <= 100 LOC).
2. Maximum Cyclomatic Complexity (CCN): 20 (Target <= 15).
3. Maximum Function Parameters (Arity): 4 parameters (Use Context/Request structs beyond 4).
4. Maximum Nesting Depth: 4 levels.

Enforces a strict Ratchet mechanism via `scripts/complexity_baseline.json`:
- Grandfathered legacy functions cannot increase in LOC, CCN, or parameters.
- When a grandfathered function is refactored, its baseline entry is ratcheted down.
- Zero new functions exceeding thresholds are allowed.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# ==============================================================================
# Architectural Hard Thresholds
# ==============================================================================
MAX_FUNCTION_LOC: int = 150
MAX_CYCLOMATIC_COMPLEXITY: int = 20
MAX_FUNCTION_PARAMETERS: int = 4
MAX_NESTING_DEPTH: int = 4

DEFAULT_BASELINE_FILE: Path = Path("scripts/complexity_baseline.json")
DEFAULT_CRATES_DIR: Path = Path("crates")

# Regex to match function declarations
FN_DECL_REGEX: re.Pattern[str] = re.compile(
    r"^[ \t]*(?:pub(?:\s*\([^)]+\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern(?:\s*\"[^\"]+\")?\s+)?fn\s+([a-zA-Z0-9_]+)",
    re.MULTILINE,
)

# Regex to match impl blocks (e.g. `impl AppState`, `impl<T> Foo for Bar`)
IMPL_REGEX: re.Pattern[str] = re.compile(r"\bimpl(?:\s*<[^>]+>)?(?:\s+[a-zA-Z0-9_:]+\s+for)?\s+([a-zA-Z0-9_]+)")

# Decision points for cyclomatic complexity
DECISION_KEYWORDS: tuple[str, ...] = ("if", "while", "for", "loop")


@dataclass
class FunctionMetrics:
    file_path: str
    scope: str
    function_name: str
    start_line: int
    end_line: int
    loc: int
    ccn: int
    parameters: int
    nesting: int

    @property
    def key(self) -> str:
        return f"{self.file_path}::{self.scope}::{self.function_name}"


def count_parameters(param_str: str) -> int:
    """Counts top-level parameters in a parameter string, ignoring generics and closures."""
    clean = param_str.strip()
    if not clean:
        return 0

    parts: list[str] = []
    current: list[str] = []
    depth_angle = 0
    depth_paren = 0

    for char in clean:
        if char == "<":
            depth_angle += 1
        elif char == ">":
            depth_angle -= 1
        elif char == "(":
            depth_paren += 1
        elif char == ")":
            depth_paren -= 1
        elif char == "," and depth_angle == 0 and depth_paren == 0:
            part = "".join(current).strip()
            if part:
                parts.append(part)
            current = []
            continue
        current.append(char)

    last_part = "".join(current).strip()
    if last_part:
        parts.append(last_part)

    return len(parts)


def find_char_literal_end(content: str, i: int) -> int | None:
    """If content[i] starts a Rust char literal ('c', '\\n', '\\u{...}'), return closing quote index."""
    if content[i] != "'":
        return None
    n = len(content)
    # If preceded by alphanumeric, it's not a char literal (e.g. foo' or lifetime in some contexts)
    if i > 0 and (content[i - 1].isalnum() or content[i - 1] == "_"):
        return None
    # 'x'
    if i + 2 < n and content[i + 2] == "'" and content[i + 1] != "\\":
        return i + 2
    # '\\x' (e.g. '\\n', '\\'', '\\\\')
    if i + 3 < n and content[i + 1] == "\\" and content[i + 3] == "'":
        return i + 3
    # '\\u{...}' or '\\xNN'
    if i + 1 < n and content[i + 1] == "\\":
        end_idx = content.find("'", i + 2)
        if end_idx != -1 and end_idx - i <= 10 and "\n" not in content[i:end_idx]:
            return end_idx
    return None


def parse_function_metrics(file_path: Path) -> list[FunctionMetrics]:
    """Parses a Rust source file and computes metrics for each function with enclosing scope."""
    content = file_path.read_text(encoding="utf-8")
    rel_path = str(file_path.as_posix())
    results: list[FunctionMetrics] = []

    # Map character indices to line numbers
    line_starts: list[int] = [0]
    for idx, char in enumerate(content):
        if char == "\n":
            line_starts.append(idx + 1)

    # State machine to track depth and impl scope
    depth = 0
    in_string = False
    in_line_comment = False
    in_block_comment = False
    impl_stack: list[tuple[int, str]] = []  # (depth_at_opening, type_name)

    i = 0
    n = len(content)

    while i < n:
        c = content[i]
        if in_line_comment:
            if c == "\n":
                in_line_comment = False
        elif in_block_comment:
            if c == "*" and i + 1 < n and content[i + 1] == "/":
                in_block_comment = False
                i += 1
        elif in_string:
            if c == "\\":
                i += 1
            elif c == '"':
                in_string = False
        else:
            if c == "/" and i + 1 < n and content[i + 1] == "/":
                in_line_comment = True
                i += 1
            elif c == "/" and i + 1 < n and content[i + 1] == "*":
                in_block_comment = True
                i += 1
            elif c == '"':
                in_string = True
            elif c == "'":
                char_end = find_char_literal_end(content, i)
                if char_end is not None:
                    i = char_end
            elif c == "{":
                depth += 1
            elif c == "}":
                while impl_stack and impl_stack[-1][0] >= depth:
                    impl_stack.pop()
                depth -= 1
            elif content[i : i + 4] == "impl" and (i == 0 or not content[i - 1].isalnum()):
                impl_match = IMPL_REGEX.match(content, i)
                if impl_match:
                    type_name = impl_match.group(1)
                    brace_pos = content.find("{", i)
                    if brace_pos != -1:
                        # Will enter this scope when brace opens
                        impl_stack.append((depth + 1, type_name))
            elif content[i : i + 2] == "fn" and (i == 0 or not content[i - 1].isalnum() and content[i - 1] != "_"):
                # Potential function declaration
                # Check if preceded by valid qualifiers on the same or previous line
                decl_start = i
                # Walk back to find start of statement/attributes
                while decl_start > 0 and content[decl_start - 1] in " \t":
                    decl_start -= 1
                line_start = content.rfind("\n", 0, i) + 1
                prefix = content[line_start:i].strip()
                valid_prefix = all(
                    w in ("pub", "pub(crate)", "pub(super)", "async", "unsafe", "extern", '"C"', "")
                    or w.startswith("pub(")
                    for w in prefix.split()
                )

                if valid_prefix:
                    m = re.match(r"fn\s+([a-zA-Z0-9_]+)", content[i:])
                    if m:
                        fn_name = m.group(1)
                        # Locate opening brace
                        brace_idx = content.find("{", i + len(m.group(0)))
                        semicolon_idx = content.find(";", i + len(m.group(0)))

                        if brace_idx != -1 and (semicolon_idx == -1 or brace_idx < semicolon_idx):
                            # It has a body! Extract parameter list
                            header = content[i:brace_idx]
                            paren_match = re.search(r"\(([\s\S]*?)\)", header)
                            num_params = count_parameters(paren_match.group(1)) if paren_match else 0

                            # Scan function body to find its closing brace and calculate CCN / max nesting
                            body_depth = 0
                            fn_end_char = -1
                            fn_max_nesting = 0
                            body_in_str = False
                            body_in_lcomment = False
                            body_in_bcomment = False

                            j = brace_idx
                            while j < n:
                                bj = content[j]
                                if body_in_lcomment:
                                    if bj == "\n":
                                        body_in_lcomment = False
                                elif body_in_bcomment:
                                    if bj == "*" and j + 1 < n and content[j + 1] == "/":
                                        body_in_bcomment = False
                                        j += 1
                                elif body_in_str:
                                    if bj == "\\":
                                        j += 1
                                    elif bj == '"':
                                        body_in_str = False
                                else:
                                    if bj == "/" and j + 1 < n and content[j + 1] == "/":
                                        body_in_lcomment = True
                                        j += 1
                                    elif bj == "/" and j + 1 < n and content[j + 1] == "*":
                                        body_in_bcomment = True
                                        j += 1
                                    elif bj == '"':
                                        body_in_str = True
                                    elif bj == "'":
                                        char_end = find_char_literal_end(content, j)
                                        if char_end is not None:
                                            j = char_end
                                    elif bj == "{":
                                        body_depth += 1
                                        if body_depth > fn_max_nesting:
                                            fn_max_nesting = body_depth
                                    elif bj == "}":
                                        body_depth -= 1
                                        if body_depth == 0:
                                            fn_end_char = j
                                            break
                                j += 1

                            if fn_end_char != -1:
                                fn_start_line = content[:line_start].count("\n") + 1
                                fn_end_line = content[:fn_end_char].count("\n") + 1
                                loc = fn_end_line - fn_start_line + 1

                                body = content[brace_idx:fn_end_char]
                                ccn = 1
                                for kw in DECISION_KEYWORDS:
                                    ccn += len(re.findall(rf"\b{kw}\b", body))
                                ccn += len(re.findall(r"=>", body))
                                ccn += len(re.findall(r"(&&|\|\|)", body))
                                ccn += len(re.findall(r"\?", body))

                                current_scope = impl_stack[-1][1] if impl_stack else "mod"

                                results.append(
                                    FunctionMetrics(
                                        file_path=rel_path,
                                        scope=current_scope,
                                        function_name=fn_name,
                                        start_line=fn_start_line,
                                        end_line=fn_end_line,
                                        loc=loc,
                                        ccn=ccn,
                                        parameters=num_params,
                                        nesting=fn_max_nesting,
                                    )
                                )
        i += 1

    return results


def scan_all_crates(crates_dir: Path) -> list[FunctionMetrics]:
    """Scans all Rust files in crates, excluding integration tests and example scratchpads."""
    all_metrics: list[FunctionMetrics] = []
    seen_keys: dict[str, int] = {}

    for rs_file in sorted(crates_dir.rglob("*.rs")):
        posix = rs_file.as_posix()
        # Exclude integration test directories, standalone examples, target and benchmarks
        if "/tests/" in posix or "/examples/" in posix or "/target/" in posix or "/benches/" in posix:
            continue
        for m in parse_function_metrics(rs_file):
            base_key = f"{m.file_path}::{m.scope}::{m.function_name}"
            count = seen_keys.get(base_key, 0)
            seen_keys[base_key] = count + 1
            if count > 0:
                # Disambiguate multiple methods with same name in same scope
                m.scope = f"{m.scope}#{count + 1}"
            all_metrics.append(m)

    return all_metrics


def load_baseline(baseline_file: Path) -> dict[str, dict[str, Any]]:
    """Loads grandfathered baseline metrics from JSON file."""
    if not baseline_file.exists():
        return {}
    try:
        return json.loads(baseline_file.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"⚠️ Warning: Could not parse baseline file {baseline_file}: {exc}", file=sys.stderr)
        return {}


def save_baseline(baseline_file: Path, baseline: dict[str, dict[str, Any]]) -> None:
    """Saves baseline metrics to JSON file with sorted keys."""
    sorted_baseline = {k: baseline[k] for k in sorted(baseline.keys())}
    baseline_file.write_text(json.dumps(sorted_baseline, indent=2) + "\n", encoding="utf-8")


def generate_baseline(metrics: list[FunctionMetrics]) -> dict[str, dict[str, Any]]:
    """Generates baseline entries for all functions currently exceeding thresholds."""
    baseline: dict[str, dict[str, Any]] = {}
    for m in metrics:
        if (
            m.loc > MAX_FUNCTION_LOC
            or m.ccn > MAX_CYCLOMATIC_COMPLEXITY
            or m.parameters > MAX_FUNCTION_PARAMETERS
            or m.nesting > MAX_NESTING_DEPTH
        ):
            baseline[m.key] = {
                "file_path": m.file_path,
                "scope": m.scope,
                "function_name": m.function_name,
                "start_line": m.start_line,
                "max_loc": m.loc,
                "max_ccn": m.ccn,
                "max_parameters": m.parameters,
                "max_nesting": m.nesting,
            }
    return baseline


def run_check(
    metrics: list[FunctionMetrics],
    baseline: dict[str, dict[str, Any]],
) -> tuple[int, int, list[str]]:
    """
    Validates metrics against thresholds and baseline.
    Returns (violations_count, ratcheted_count, error_messages).
    """
    violations: list[str] = []
    ratcheted_count = 0

    for m in metrics:
        entry = baseline.get(m.key)
        if entry is not None:
            # Grandfathered function: verify it does not degrade
            max_loc = entry.get("max_loc", MAX_FUNCTION_LOC)
            max_ccn = entry.get("max_ccn", MAX_CYCLOMATIC_COMPLEXITY)
            max_params = entry.get("max_parameters", MAX_FUNCTION_PARAMETERS)
            max_nesting = entry.get("max_nesting", MAX_NESTING_DEPTH)

            reasons: list[str] = []
            if m.loc > max_loc:
                reasons.append(f"LOC increased from {max_loc} to {m.loc}")
            if m.ccn > max_ccn:
                reasons.append(f"CCN increased from {max_ccn} to {m.ccn}")
            if m.parameters > max_params:
                reasons.append(f"parameters increased from {max_params} to {m.parameters}")
            if m.nesting > max_nesting:
                reasons.append(f"nesting depth increased from {max_nesting} to {m.nesting}")

            if reasons:
                violations.append(
                    f"❌ [REGRESSION] Grandfathered function degraded in {m.file_path}:{m.start_line} (`{m.scope}::{m.function_name}`): {', '.join(reasons)}"
                )

            # Check if function was significantly refactored and could be ratcheted
            if (
                m.loc <= MAX_FUNCTION_LOC
                and m.ccn <= MAX_CYCLOMATIC_COMPLEXITY
                and m.parameters <= MAX_FUNCTION_PARAMETERS
                and m.nesting <= MAX_NESTING_DEPTH
            ):
                ratcheted_count += 1
        else:
            # New or ungrandfathered function: strict thresholds apply
            reasons = []
            if m.loc > MAX_FUNCTION_LOC:
                reasons.append(f"LOC {m.loc} > {MAX_FUNCTION_LOC}")
            if m.ccn > MAX_CYCLOMATIC_COMPLEXITY:
                reasons.append(f"CCN {m.ccn} > {MAX_CYCLOMATIC_COMPLEXITY}")
            if m.parameters > MAX_FUNCTION_PARAMETERS:
                reasons.append(f"parameters {m.parameters} > {MAX_FUNCTION_PARAMETERS}")
            if m.nesting > MAX_NESTING_DEPTH:
                reasons.append(f"nesting depth {m.nesting} > {MAX_NESTING_DEPTH}")

            if reasons:
                violations.append(
                    f"❌ [THRESHOLD VIOLATION] Function exceeds limits in {m.file_path}:{m.start_line} (`{m.scope}::{m.function_name}`): {', '.join(reasons)}"
                )

    return len(violations), ratcheted_count, violations


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Enforce strict Rust architectural complexity and function size limits."
    )
    parser.add_argument(
        "--crates-dir",
        type=Path,
        default=DEFAULT_CRATES_DIR,
        help="Path to crates directory (default: crates/)",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        default=DEFAULT_BASELINE_FILE,
        help="Path to grandfathered baseline JSON file (default: scripts/complexity_baseline.json)",
    )
    parser.add_argument(
        "--update-baseline",
        action="store_true",
        help="Regenerate baseline file with current state of codebase",
    )
    parser.add_argument(
        "--ratchet",
        action="store_true",
        help="Automatically remove cleaned grandfathered functions from baseline",
    )
    args = parser.parse_args()

    metrics = scan_all_crates(args.crates_dir)
    print(f"🔍 Scanned {len(metrics)} Rust production functions across {args.crates_dir}/...")

    if args.update_baseline:
        new_baseline = generate_baseline(metrics)
        save_baseline(args.baseline, new_baseline)
        print(f"✅ Baseline saved to {args.baseline} ({len(new_baseline)} grandfathered functions).")
        return 0

    baseline = load_baseline(args.baseline)
    if not baseline and not args.baseline.exists():
        print(f"ℹ️ No baseline found at {args.baseline}. Generating initial baseline...")
        baseline = generate_baseline(metrics)
        save_baseline(args.baseline, baseline)
        print(f"✅ Baseline initialized with {len(baseline)} functions.")

    if args.ratchet:
        current_map = {m.key: m for m in metrics}
        cleaned_keys: list[str] = []
        ratcheted_keys: list[str] = []

        for key, entry in list(baseline.items()):
            m = current_map.get(key)
            if m is None:
                cleaned_keys.append(key)
                del baseline[key]
                continue

            if (
                m.loc <= MAX_FUNCTION_LOC
                and m.ccn <= MAX_CYCLOMATIC_COMPLEXITY
                and m.parameters <= MAX_FUNCTION_PARAMETERS
                and m.nesting <= MAX_NESTING_DEPTH
            ):
                cleaned_keys.append(key)
                del baseline[key]
            else:
                updated = False
                if m.loc < entry["max_loc"]:
                    entry["max_loc"] = m.loc
                    updated = True
                if m.ccn < entry["max_ccn"]:
                    entry["max_ccn"] = m.ccn
                    updated = True
                if m.parameters < entry["max_parameters"]:
                    entry["max_parameters"] = m.parameters
                    updated = True
                if m.nesting < entry["max_nesting"]:
                    entry["max_nesting"] = m.nesting
                    updated = True
                if updated:
                    ratcheted_keys.append(key)

        save_baseline(args.baseline, baseline)
        print(
            f"🎯 Ratchet applied: {len(cleaned_keys)} functions fully cured (removed from baseline), {len(ratcheted_keys)} functions tightened."
        )

    violation_count, ratcheted_count, errors = run_check(metrics, baseline)

    if errors:
        print(f"\n🚨 Architectural Gate Failure ({violation_count} violation(s) detected):", file=sys.stderr)
        for err in errors:
            print(f"  {err}", file=sys.stderr)
        print(
            "\n💡 Requirements: Max LOC <= 150 (target <= 100), Max CCN <= 20, Max Params <= 4, Max Nesting <= 4.",
            file=sys.stderr,
        )
        return 1

    print(
        f"✅ Architectural Complexity Gate passed: 0 regressions, 0 unauthorized monoliths ({len(baseline)} grandfathered functions tracked)."
    )
    if ratcheted_count > 0:
        print(
            f"🎉 {ratcheted_count} grandfathered function(s) are now within thresholds! Run `python3 scripts/lint_complexity.py --ratchet` to lock gains."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
