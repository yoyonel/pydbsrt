#!/usr/bin/env python3
"""Validates HTML structure, embedded JavaScript syntax (via node --check), and JSON files."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from html.parser import HTMLParser
from pathlib import Path

VOID_ELEMENTS = {
    "area",
    "base",
    "br",
    "col",
    "embed",
    "hr",
    "img",
    "input",
    "link",
    "meta",
    "param",
    "source",
    "track",
    "wbr",
}


class StrictHTMLValidator(HTMLParser):
    """Validates HTML tag nesting and extracts inline <script> blocks."""

    def __init__(self, filepath: Path) -> None:
        super().__init__(convert_charrefs=False)
        self.filepath = filepath
        self.tag_stack: list[tuple[str, int]] = []
        self.in_script = False
        self.current_script: list[str] = []
        self.scripts: list[str] = []
        self.errors: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        t = tag.lower()
        if t not in VOID_ELEMENTS:
            self.tag_stack.append((t, self.getpos()[0]))
        if t == "script":
            self.in_script = True
            self.current_script = []

    def handle_endtag(self, tag: str) -> None:
        t = tag.lower()
        if t == "script" and self.in_script:
            self.in_script = False
            self.scripts.append("".join(self.current_script))
            self.current_script = []
        if t in VOID_ELEMENTS:
            return
        if not self.tag_stack:
            self.errors.append(f"{self.filepath}:{self.getpos()[0]}: unexpected closing </{t}>")
            return
        last_tag, line = self.tag_stack[-1]
        if last_tag == t:
            self.tag_stack.pop()
        else:
            self.errors.append(
                f"{self.filepath}:{self.getpos()[0]}: mismatched closing </{t}> "
                f"(expected </{last_tag}> opened at line {line})"
            )

    def handle_data(self, data: str) -> None:
        if self.in_script:
            self.current_script.append(data)


def validate_html_file(path: Path) -> list[str]:
    content = path.read_text(encoding="utf-8")
    validator = StrictHTMLValidator(path)
    validator.feed(content)
    errors = list(validator.errors)
    for tag, line in validator.tag_stack:
        errors.append(f"{path}:{line}: unclosed <{tag}>")

    node_bin = shutil.which("node")
    if node_bin and validator.scripts:
        for idx, script_code in enumerate(validator.scripts, start=1):
            with tempfile.NamedTemporaryFile(mode="w", suffix=".js", encoding="utf-8", delete=False) as tmp:
                tmp.write(script_code)
                tmp_path = Path(tmp.name)
            try:
                proc = subprocess.run(
                    [node_bin, "--check", str(tmp_path)],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                if proc.returncode != 0:
                    errors.append(f"{path} (<script #{idx}>): JS syntax error:\n{proc.stderr.strip()}")
            finally:
                tmp_path.unlink(missing_ok=True)
    return errors


def validate_json_file(path: Path) -> list[str]:
    try:
        json.loads(path.read_text(encoding="utf-8"))
        return []
    except Exception as exc:  # noqa: BLE001
        return [f"{path}: invalid JSON: {exc}"]


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    html_files = list((root / "crates").rglob("*.html"))
    json_files = [p for p in (root / "data").rglob("*.json") if ".venv" not in p.parts and "target" not in p.parts]

    all_errors: list[str] = []
    for hf in html_files:
        all_errors.extend(validate_html_file(hf))
    for jf in json_files:
        all_errors.extend(validate_json_file(jf))

    if all_errors:
        for err in all_errors:
            print(f"✗ {err}", file=sys.stderr)
        return 1

    print(f"✓ Web & JSON assets validated ({len(html_files)} HTML/JS, {len(json_files)} JSON)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
