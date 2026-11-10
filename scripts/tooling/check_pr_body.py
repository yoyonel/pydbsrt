#!/usr/bin/env python3
"""Fails a PR whose description lacks business goals or a prod proof (or why there is none).

Usage: printf '%s' "$PR_BODY" | python3 scripts/check_pr_body.py
"""

from __future__ import annotations

import re
import sys
import unicodedata

SECTION = "Objectif métier"
PROOF_SECTION = "Validation Empirique"
BENEFIT_RE = re.compile(r"^- \*\*[^*]+\*\*\s*\S")
# Masked terminal proof recorded by `task proof:record` (scripts/record_proof.py)
PROOF_RE = re.compile(r"\.github/proofs/[\w-]+\.svg")


def _fold(text: str) -> str:
    """Lowercase ASCII form, so headings match with or without accents."""
    return unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode().lower()


def _section_lines(body: str, title: str) -> list[str] | None:
    body = re.sub(r"<!--.*?-->", "", body, flags=re.DOTALL)
    lines = [line.strip() for line in body.splitlines()]
    starts = [i for i, line in enumerate(lines) if line.startswith("## ") and _fold(title) in _fold(line)]
    if not starts:
        return None
    section = []
    for line in lines[starts[0] + 1 :]:
        if line.startswith("## "):
            break
        section.append(line)
    return section


def _has_labelled_value(lines: list[str], label: str) -> bool:
    """True when a `**<label>...**` line carries text after the bold label."""
    for line in lines:
        match = re.match(r"\*\*(.+?)\*\*(.*)", line)
        if match and _fold(match.group(1)).startswith(_fold(label)) and match.group(2).strip(" :"):
            return True
    return False


def _check_goals(body: str) -> list[str]:
    lines = _section_lines(body, SECTION)
    if lines is None:
        return [f"Section « ## {SECTION} » absente : voir .github/pull_request_template.md"]
    errors = []
    if not any(BENEFIT_RE.match(line) for line in lines):
        errors.append("Aucun bénéfice : ajouter « - **Bénéfice.** explication simple »")
    if not _has_labelled_value(lines, "Hors périmètre"):
        errors.append("« **Hors périmètre :** » vide : dire ce que la PR ne fait pas (ou « aucun »)")
    if not _has_labelled_value(lines, "KPI"):
        errors.append("« **KPI / preuve :** » vide : chiffre mesuré ou comment on le voit en prod")
    return errors


def _check_proof(body: str) -> list[str]:
    lines = _section_lines(body, PROOF_SECTION)
    if lines is None:
        return [f"Section « ## {PROOF_SECTION} » absente : voir .github/pull_request_template.md"]
    if any(PROOF_RE.search(line) for line in lines) or _has_labelled_value(lines, "Non déployé"):
        return []
    return [
        "Ni preuve prod (image .github/proofs/<nom>.svg, via task proof:record) "
        "ni « **Non déployé :** raison » dans la validation"
    ]


def check(body: str) -> list[str]:
    """Returns one message per missing item; empty when the description is complete."""
    return _check_goals(body) + _check_proof(body)


def main() -> int:
    errors = check(sys.stdin.read())
    for error in errors:
        print(f"PR body: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
