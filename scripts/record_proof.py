#!/usr/bin/env python3
"""Records a masked terminal proof (asciicast + animated SVG) to embed in a public PR.

Usage: uv run python scripts/record_proof.py NAME COMMAND...
       (task proof:record -- o2switch_status task o2switch:status)

The command runs once for real; its output is masked (/home paths, URLs, emails, IPs, auth values,
every value of the local env files) before anything is written. Each written file is then checked
again and deleted if anything is left to mask: a proof never reaches git unmasked.
"""

from __future__ import annotations

import argparse
import html
import re
import shlex
import subprocess
import sys
import time
from datetime import UTC, datetime
from pathlib import Path

from record_casts import DEFAULT_TERMINAL_WIDTH, write_cast_file

ROOT = Path(__file__).resolve().parents[1]
PROOFS_DIR = ROOT / ".github" / "proofs"
# Same files as scripts/load_env.sh: they hold the NAS host/user and the public URL
ENV_FILES = [ROOT / ".env", Path.home() / ".config" / "pydbsrt" / "env"]
SVG_TERM = ["npx", "-y", "svg-term-cli@2.1.1"]
COMMAND_TIMEOUT_SEC = 600
MAX_TERMINAL_WIDTH = 160
# No branch in the prompt: record_casts' `[develop]` would be a made-up fact in a proof
PROOF_PROMPT = "\x1b[1;34mpydbsrt\x1b[0m \x1b[1;36m\u276f\x1b[0m "
ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
NAME_RE = re.compile(r"[a-z0-9][a-z0-9_-]*")
# Namespaces svg-term writes into every SVG: not host data
ALLOWED_URLS = {"http://www.w3.org/2000/svg", "http://www.w3.org/1999/xlink"}

_TOKEN = r"[^\s\"'<>\\\x1b]"
_SECRET_KEY = r"(?:authorization|x-api-key|api[_-]?key|password|passwd|token|secret)"
RULES = [
    ("url", re.compile(rf"\b[a-z][a-z0-9+.-]*://{_TOKEN}+", re.IGNORECASE), "<url>"),
    ("home", re.compile(r"/home/(?!<user>)[^/\s\"'<>\\\x1b]+"), "/home/<user>"),
    ("email", re.compile(r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+"), "<email>"),
    ("ipv4", re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b"), "<ip>"),
    ("secret", re.compile(rf"(?i)\b((?:bearer|basic)\s+)(?!<redacted>){_TOKEN}+"), r"\1<redacted>"),
    (
        "secret",
        re.compile(
            rf"(?i)\b({_SECRET_KEY}\"?\s*[:=]\s*\"?(?:(?:bearer|basic)\s+)?)(?!<redacted>|(?:bearer|basic)\s){_TOKEN}+"
        ),
        r"\1<redacted>",
    ),
]


def _value_re(values: list[str]) -> re.Pattern[str] | None:
    """Longest first, so a value is never half-masked by one of its prefixes."""
    ordered = sorted({v for v in values if v}, key=len, reverse=True)
    return re.compile("|".join(map(re.escape, ordered)), re.IGNORECASE) if ordered else None


def mask(text: str, values: list[str]) -> str:
    value_re = _value_re(values)
    if value_re:
        text = value_re.sub("<masked>", text)
    for _, pattern, replacement in RULES:
        text = pattern.sub(replacement, text)
    return text


def leaks(text: str, values: list[str]) -> list[str]:
    """Categories still present in `text`; never the values themselves (they end up in CI logs)."""
    # SVG text is XML-escaped: `&lt;user&gt;` is a mask, `&#46;` can hide a real value
    text = html.unescape(text)
    value_re = _value_re(values)
    found = ["value"] if value_re and value_re.search(text) else []
    for category, pattern, _ in RULES:
        hits = [m.group() for m in pattern.finditer(text) if m.group() not in ALLOWED_URLS]
        if hits and category not in found:
            found.append(category)
    return found


def env_values(paths: list[Path]) -> list[str]:
    values: list[str] = []
    for path in paths:
        if not path.is_file():
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            key, sep, value = line.strip().partition("=")
            value = value.strip().strip("'\"")
            if sep and key and not key.startswith("#") and value and value not in values:
                values.append(value)
    return values


def ensure_clean(path: Path, values: list[str]) -> None:
    found = leaks(path.read_text(encoding="utf-8", errors="replace"), values)
    if found:
        path.unlink()
        raise SystemExit(f"record_proof: {path.name} still leaks {', '.join(found)}: deleted")


def record(name: str, command: list[str], out_dir: Path, values: list[str]) -> Path:
    if not NAME_RE.fullmatch(name):
        raise SystemExit(f"record_proof: invalid name {name!r} (expected {NAME_RE.pattern})")
    run = subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        errors="replace",
        timeout=COMMAND_TIMEOUT_SEC,
        check=False,
    )
    if run.returncode != 0:
        raise SystemExit(f"record_proof: command failed rc={run.returncode}, no proof written")
    lines = mask(run.stdout, values).splitlines()
    recorded_at = datetime.now(UTC).strftime("%Y-%m-%d %H:%M UTC")
    command_text = f"{mask(shlex.join(command), values)}  # recorded {recorded_at}"
    longest = max(len(ANSI_RE.sub("", line)) for line in [command_text, *lines])
    cast = out_dir / f"{name}.cast"
    write_cast_file(
        cast,
        title=command_text,
        command_text=command_text,
        output_lines=lines,
        width=min(MAX_TERMINAL_WIDTH, max(DEFAULT_TERMINAL_WIDTH, longest + 12)),
        height=len(lines) + 3,
        timestamp=int(time.time()),
        prompt=PROOF_PROMPT,
    )
    ensure_clean(cast, values)
    return cast


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Record a masked terminal proof for a PR.")
    parser.add_argument("name", help="file name under .github/proofs/ (a-z, 0-9, _ and -)")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="command to run and record")
    args = parser.parse_args(argv)
    if not args.command:
        parser.error("missing command")
    # shortcut: only values found in the env files are masked by name, the remote o2switch user is
    # caught by the /home rule only; add PROOF_MASK_VALUES if a command ever prints it alone.
    values = env_values(ENV_FILES)
    cast = record(args.name, args.command, PROOFS_DIR, values)
    svg = cast.with_suffix(".svg")
    subprocess.run([*SVG_TERM, "--in", str(cast), "--out", str(svg), "--window"], check=True)
    with svg.open("a", encoding="utf-8") as out:
        out.write("\n")  # svg-term ends without one; end-of-file-fixer would reject the commit
    ensure_clean(svg, values)
    rel = svg.relative_to(ROOT)
    print(f"{cast.relative_to(ROOT)}\n{rel}")
    print(f"Commit both, then embed in the PR: ![{args.name}](https://github.com/yoyonel/pydbsrt/raw/<sha>/{rel})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
