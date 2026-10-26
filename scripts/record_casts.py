#!/usr/bin/env python3
"""Deterministic Asciinema Cast Generator for pydbsrt documentation.

Generates polished, paced, and interactive terminal recording sessions (.cast)
ready for rendering via Asciinema Player in MkDocs.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Any, List, Tuple

PROMPT_PREFIX: str = "\x1b[1;34mpydbsrt\x1b[0m [\x1b[32mdevelop\x1b[0m] \x1b[1;36m❯\x1b[0m "
KEYSTROKE_DELAY_SEC: float = 0.035
COMMAND_START_DELAY_SEC: float = 0.4
POST_EXECUTION_DELAY_SEC: float = 0.1
LINE_DELAY_SEC: float = 0.015
FINAL_PAUSE_SEC: float = 6.0
DEFAULT_TERMINAL_WIDTH: int = 92
DEFAULT_TERMINAL_HEIGHT: int = 24

CASTS_OUTPUT_DIR: Path = Path("docs/assets/casts")


def build_cast_events(
    command_text: str,
    output_lines: List[str],
    width: int = DEFAULT_TERMINAL_WIDTH,
    height: int = DEFAULT_TERMINAL_HEIGHT,
) -> List[Tuple[float, str, str]]:
    """Build a list of asciicast v2 timed events for a shell command run."""
    events: List[Tuple[float, str, str]] = []
    current_time: float = 0.0

    # 1. Initial Prompt
    events.append((round(current_time, 3), "o", PROMPT_PREFIX))
    current_time += COMMAND_START_DELAY_SEC

    # 2. Simulated Keystrokes
    for char in command_text:
        events.append((round(current_time, 3), "o", char))
        current_time += KEYSTROKE_DELAY_SEC

    # 3. Enter key
    events.append((round(current_time, 3), "o", "\r\n"))
    current_time += POST_EXECUTION_DELAY_SEC

    # 4. Command Output
    for line in output_lines:
        normalized_line = line.rstrip("\r\n") + "\r\n"
        events.append((round(current_time, 3), "o", normalized_line))
        current_time += LINE_DELAY_SEC

    # 5. Return to Prompt
    current_time += POST_EXECUTION_DELAY_SEC
    events.append((round(current_time, 3), "o", PROMPT_PREFIX))

    # 6. Idle pause so users can read comfortably
    current_time += FINAL_PAUSE_SEC
    events.append((round(current_time, 3), "o", ""))

    return events


def write_cast_file(
    out_path: Path,
    title: str,
    command_text: str,
    output_lines: List[str],
    width: int = DEFAULT_TERMINAL_WIDTH,
    height: int = DEFAULT_TERMINAL_HEIGHT,
) -> None:
    """Serialize header and events into a valid Asciicast v2 JSON Lines file."""
    header: dict[str, Any] = {
        "version": 2,
        "width": width,
        "height": height,
        "timestamp": 1729900000,
        "title": title,
        "env": {"SHELL": "/bin/bash", "TERM": "xterm-256color"},
    }

    events = build_cast_events(command_text, output_lines, width=width, height=height)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write(json.dumps(header) + "\n")
        for t, kind, text in events:
            f.write(json.dumps([t, kind, text]) + "\n")


def generate_cli_help_cast() -> None:
    """Generate cast for dbsrt --help."""
    res = subprocess.run(["./target/release/dbsrt", "--help"], capture_output=True, text=True, check=True)
    lines = res.stdout.splitlines()
    write_cast_file(
        CASTS_OUTPUT_DIR / "term_cli_help.cast",
        title="dbsrt --help",
        command_text="dbsrt --help",
        output_lines=lines,
        height=26,
    )


def generate_media_add_cast() -> None:
    """Generate cast for dbsrt media:add --help."""
    res = subprocess.run(["./target/release/dbsrt", "media:add", "--help"], capture_output=True, text=True, check=True)
    lines = res.stdout.splitlines()
    write_cast_file(
        CASTS_OUTPUT_DIR / "term_media_add.cast",
        title="dbsrt media:add --help",
        command_text="dbsrt media:add --help",
        output_lines=lines,
        height=28,
    )


def generate_status_cast() -> None:
    """Generate cast for task status."""
    res = subprocess.run(["task", "status"], capture_output=True, text=True, check=True)
    lines = res.stdout.splitlines()
    write_cast_file(
        CASTS_OUTPUT_DIR / "term_status.cast",
        title="task status",
        command_text="task status",
        output_lines=lines,
        height=32,
    )


def generate_doc_check_cast() -> None:
    """Generate cast for task doc:check.

    NOTE: Uses synthetic hardcoded output (not a recursive `task doc:check` execution)
    to ensure a deterministic, polished, and rapid cast generation for documentation.
    Real run output varies with timings and would trigger recursive validation.
    If the doc:check validation pipeline changes, update the lines below to match.
    """
    lines = [
        # Synthetic output - do not replace with recursive subprocess call
        "task: [doc:check] uv run --no-sync python scripts/validate_docs.py",
        "🔍 [0/4] Checking file extensions and forbidden machine-local paths in docs/...",
        "✅ Zero unauthorized files & zero machine-local paths: OK",
        "🔍 [1/4] Building documentation with mkdocs build --strict...",
        "✅ MkDocs strict build: OK",
        "🔍 [2/4] Validating rendered HTML AST & integrity...",
        "✅ HTML AST & LaTeX validation: 0 errors",
        "🔍 [3/4] Running Headless Chrome DOM & MathJax check...",
        "✅ Headless Chrome vector math rendering: OK",
        "",
        "🎉 Documentation 100% valid! Zero formatting, LaTeX, or rendering defects.",
    ]
    write_cast_file(
        CASTS_OUTPUT_DIR / "term_doc_check.cast",
        title="task doc:check",
        command_text="task doc:check",
        output_lines=lines,
        height=20,
    )


def main() -> int:
    CASTS_OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    print("🎬 Generating deterministic Asciinema cast sessions under docs/assets/casts/...")

    generate_cli_help_cast()
    print("  • Generated term_cli_help.cast")

    generate_media_add_cast()
    print("  • Generated term_media_add.cast")

    generate_status_cast()
    print("  • Generated term_status.cast")

    generate_doc_check_cast()
    print("  • Generated term_doc_check.cast")

    print("✅ All 4 Asciinema casts recorded successfully.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
