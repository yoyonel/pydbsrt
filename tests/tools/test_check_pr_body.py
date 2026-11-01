"""Tests for scripts/check_pr_body.py (mandatory business-goals section of a PR)."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check_pr_body.py"
TEMPLATE = ROOT / ".github" / "pull_request_template.md"

_spec = importlib.util.spec_from_file_location("check_pr_body", SCRIPT)
assert _spec and _spec.loader
check_pr_body = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_pr_body)

VALID = """## 🎯 Objectif métier (en 30 secondes)

<!-- guide -->

- **Ne plus gaspiller le worker.** Un média au nom trop long échoue en ~1 s.
- **Un débit NAS fiable.** Les reprises ne gonflent plus le débit.

**Hors périmètre :** rendre extractibles les médias à nom long.

**KPI / preuve :** 5/5 noms trop longs échouent en ~1 s sur o2switch.

---

## 🛡️ Changements & Périmètre Technique

- détail technique

---

## 🧪 Validation Empirique & Preuves Terminales

<!-- guide -->

![o2switch_status](https://github.com/yoyonel/pydbsrt/raw/0123abc/.github/proofs/o2switch_status.svg)

**Non déployé :**
"""


def test_valid_body_passes() -> None:
    assert check_pr_body.check(VALID) == []


def test_heading_without_accent_is_accepted() -> None:
    assert check_pr_body.check(VALID.replace("Objectif métier", "Objectif metier")) == []


def test_missing_sections_fail() -> None:
    errors = check_pr_body.check("## Summary\n\n- **Fix.** something\n")
    assert len(errors) == 2, errors
    assert "Objectif métier" in errors[0]
    assert "Validation Empirique" in errors[1]


def test_unfilled_template_fails_on_every_item() -> None:
    errors = check_pr_body.check(TEMPLATE.read_text(encoding="utf-8"))
    assert len(errors) == 4, errors


def test_not_deployed_with_a_reason_replaces_the_prod_proof() -> None:
    body = VALID.replace("![o2switch_status]", "[lien]").replace(".github/proofs/o2switch_status.svg", "x.png")
    assert len(check_pr_body.check(body)) == 1
    assert check_pr_body.check(body.replace("**Non déployé :**", "**Non déployé :** doc seule")) == []


@pytest.mark.parametrize(
    "old,new",
    [
        (".github/proofs/o2switch_status.svg", "docs/assets/x.svg"),
        ("![o2switch_status]", "<!-- ![o2switch_status]"),
        ("**Non déployé :**", "**Non déployé :** :"),
    ],
)
def test_validation_without_prod_proof_fails(old: str, new: str) -> None:
    body = VALID.replace(old, new)
    if new.startswith("<!--"):
        body = body.replace("o2switch_status.svg)", "o2switch_status.svg) -->")
    if old.startswith("**Non"):
        body = body.replace(".github/proofs/o2switch_status.svg", "docs/assets/x.svg")
    errors = check_pr_body.check(body)
    assert len(errors) == 1, errors
    assert ".github/proofs/" in errors[0] and "Non déployé" in errors[0]


@pytest.mark.parametrize(
    "old,new,needle",
    [
        ("- **Ne plus gaspiller le worker.** Un", "- Ne plus gaspiller le worker. Un", None),
        ("**Hors périmètre :** rendre extractibles les médias à nom long.", "**Hors périmètre :**", "Hors périmètre"),
        ("**KPI / preuve :** 5/5 noms trop longs échouent en ~1 s sur o2switch.", "**KPI / preuve :**", "KPI"),
    ],
)
def test_each_missing_item_is_reported(old: str, new: str, needle: str | None) -> None:
    body = VALID.replace(old, new)
    if needle is None:
        body = body.replace("- **Un débit NAS fiable.** Les", "- Un débit NAS fiable. Les")
        needle = "bénéfice"
    errors = check_pr_body.check(body)
    assert len(errors) == 1, errors
    assert needle in errors[0]


def test_items_outside_the_section_do_not_count() -> None:
    body = """## 🎯 Objectif métier

---

## Autre

- **Bénéfice.** texte

**Hors périmètre :** rien

**KPI / preuve :** chiffre
"""
    assert len(check_pr_body.check(body)) == 4


def test_items_hidden_in_comments_do_not_count() -> None:
    body = VALID.replace("**KPI / preuve :** 5/5", "<!-- **KPI / preuve :** 5/5").replace(
        "sur o2switch.", "sur o2switch. -->"
    )
    errors = check_pr_body.check(body)
    assert len(errors) == 1 and "KPI" in errors[0]


@pytest.mark.parametrize("body,rc", [(VALID, 0), ("## Summary\n", 1), ("", 1)])
def test_cli_reads_stdin_and_sets_exit_code(body: str, rc: int) -> None:
    run = subprocess.run(
        [sys.executable, str(SCRIPT)], input=body, capture_output=True, text=True, encoding="utf-8", timeout=30
    )
    assert run.returncode == rc, run.stderr
    assert bool(run.stderr) == bool(rc)
