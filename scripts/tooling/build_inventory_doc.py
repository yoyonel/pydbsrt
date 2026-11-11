#!/usr/bin/env python3
"""Builds the comprehensive script inventory documentation from an inventory JSON dataset."""

import argparse
import json
import sys
from pathlib import Path
from typing import Any


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


def resolve_input_path(repo_root: Path, custom_input: str | None = None) -> Path | None:
    """Locates scripts inventory JSON candidate."""
    if custom_input:
        p = Path(custom_input)
        if not p.is_absolute():
            p = repo_root / p
        return p if p.is_file() else None

    candidates = [
        Path("/tmp/scripts_inventory.json"),
        repo_root / "data/reports/scripts_inventory.json",
        repo_root / "docs/reference/scripts_inventory.json",
    ]
    for c in candidates:
        if c.is_file():
            return c
    return None


def format_category_table(categories: dict[str, list[dict[str, Any]]]) -> list[str]:
    """Generates the Markdown overview table for categories."""
    lines: list[str] = [
        "| Catégorie | Répertoire | Scripts | Reliés Taskfile | Documentés Diátaxis | Docstring / En-tête |",
        "| :--- | :--- | :---: | :---: | :---: | :---: |",
    ]
    for cat, items in sorted(categories.items()):
        c_tot = len(items)
        c_tasks = sum(1 for i in items if i.get("tasks"))
        c_docs = sum(1 for i in items if i.get("docs"))
        c_ds = sum(1 for i in items if i.get("has_docstring"))
        rep = items[0]["path"].rsplit("/", 1)[0]
        lines.append(f"| **{cat}** | `{rep}/` | {c_tot} | {c_tasks} | {c_docs} | {c_ds} |")
    return lines


def format_script_item(item: dict[str, Any]) -> list[str]:
    """Formats a single script entry into Markdown."""
    lines: list[str] = [
        f"#### `{item['path']}`",
        f"- **But / Rôle** : {item.get('purpose', '')}",
        f"- **Environnement dédié** : `{item.get('env', '')}`",
    ]
    tasks = item.get("tasks") or []
    if tasks:
        task_list = ", ".join([f"`task {t}`" for t in tasks])
        lines.append(f"- **Recettes GoTasks associées** : {task_list}")
    else:
        lines.append("- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*")

    ds_status = "✅ Présente" if item.get("has_docstring") else "⚠️ Absente ou incomplète"
    lines.append(f"- **Documentation interne (docstring / header)** : {ds_status}")

    docs = item.get("docs") or []
    if docs:
        doc_list = ", ".join([f"`{d}`" for d in docs])
        lines.append(f"- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans {doc_list}")
    else:
        lines.append(
            "- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)"
        )

    lines.append("")
    return lines


def build_inventory_markdown(data: list[dict[str, Any]]) -> str:
    """Constructs complete Markdown documentation string."""
    categories: dict[str, list[dict[str, Any]]] = {}
    for item in data:
        cat = item.get("category", "scripts")
        categories.setdefault(cat, []).append(item)

    total = len(data)
    with_tasks = sum(1 for d in data if d.get("tasks"))
    with_docs = sum(1 for d in data if d.get("docs"))
    with_docstrings = sum(1 for d in data if d.get("has_docstring"))

    out: list[str] = [
        "# Inventaire Exhaustif des Scripts du Projet `pydbsrt`",
        "",
        "> **Date de génération :** 2026-11-10 18:57:00 (Post-Refactor PR #70 / Issue #69 Phase 1)  ",
        "> **Statut :** Audit exhaustif de 94 scripts opérationnels (Python & Bash) après modularisation sous `scripts/`.",
        "",
        "## 1. Vue d'Ensemble et Statistiques Globales",
        "",
        f"- **Total scripts opérationnels audités** : {total}",
        f"- **Scripts reliés à des cibles Taskfile (`task ...`)** : {with_tasks} / {total} ({with_tasks / total * 100:.1f}%)"
        if total
        else "- **Total**: 0",
        f"- **Scripts documentés dans Diátaxis (`docs/`)** : {with_docs} / {total} ({with_docs / total * 100:.1f}%)"
        if total
        else "",
        f"- **Scripts avec en-tête / docstring documentée** : {with_docstrings} / {total} ({with_docstrings / total * 100:.1f}%)"
        if total
        else "",
        "",
    ]
    out.extend(format_category_table(categories))
    out.extend(["", "---", "", "## 2. Inventaire Détaillé par Sous-Système", ""])

    cat_order = ["tooling", "benchmarks", "e2e", "ingestion", "workers", "local", "o2switch", "rpi4", "scripts"]
    idx = 1
    for cat_name in cat_order:
        items = categories.get(cat_name)
        if not items:
            continue
        dir_name = items[0]["path"].rsplit("/", 1)[0]
        out.append(f"### 2.{idx} Module `{dir_name}/` ({len(items)} scripts)")
        out.append("")
        idx += 1
        for item in items:
            out.extend(format_script_item(item))

    return "\n".join(out)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Builds the comprehensive script inventory documentation from an inventory JSON dataset."
    )
    parser.add_argument(
        "--input",
        type=str,
        default=None,
        help="Path to scripts inventory JSON (default: /tmp/scripts_inventory.json).",
    )
    parser.add_argument(
        "--output",
        type=str,
        default=None,
        help="Destination markdown file (default: docs/reference/scripts-inventory-2026-11-10.md).",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Validate inventory data and print summary without writing file.",
    )
    args = parser.parse_args()

    repo_root = find_repo_root()
    input_path = resolve_input_path(repo_root, args.input)
    if not input_path:
        print("Error: Scripts inventory JSON file not found.", file=sys.stderr)
        sys.exit(1)

    with open(input_path, encoding="utf-8") as f:
        data = json.load(f)

    if not isinstance(data, list):
        print(f"Error: Expected list of script items, got {type(data).__name__}.", file=sys.stderr)
        sys.exit(1)

    if args.check:
        print(f"Inventory OK: {input_path} ({len(data)} scripts)")
        return

    content = build_inventory_markdown(data)

    if args.output:
        out_path = Path(args.output)
        if not out_path.is_absolute():
            out_path = repo_root / out_path
    else:
        out_path = repo_root / "docs/reference/scripts-inventory-2026-11-10.md"

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(content, encoding="utf-8")
    print(f"Wrote inventory to {out_path} ({len(content.splitlines())} lines)")


if __name__ == "__main__":
    main()
