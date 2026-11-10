import json
from pathlib import Path

data = json.loads(Path("/tmp/scripts_inventory.json").read_text())
categories = {}
for item in data:
    cat = item["category"]
    categories.setdefault(cat, []).append(item)

out = []
out.append("# Inventaire Exhaustif des Scripts du Projet `pydbsrt`")
out.append("")
out.append("> **Date de génération :** 2026-11-10 18:57:00 (Post-Refactor PR #70 / Issue #69 Phase 1)  ")
out.append(
    "> **Statut :** Audit exhaustif de 94 scripts opérationnels (Python & Bash) après modularisation sous `scripts/`."
)
out.append("")
out.append("## 1. Vue d'Ensemble et Statistiques Globales")
out.append("")
total = len(data)
with_tasks = sum(1 for d in data if d["tasks"])
with_docs = sum(1 for d in data if d["docs"])
with_docstrings = sum(1 for d in data if d["has_docstring"])

out.append(f"- **Total scripts opérationnels audités** : {total}")
out.append(
    f"- **Scripts reliés à des cibles Taskfile (`task ...`)** : {with_tasks} / {total} ({with_tasks / total * 100:.1f}%)"
)
out.append(f"- **Scripts documentés dans Diátaxis (`docs/`)** : {with_docs} / {total} ({with_docs / total * 100:.1f}%)")
out.append(
    f"- **Scripts avec en-tête / docstring documentée** : {with_docstrings} / {total} ({with_docstrings / total * 100:.1f}%)"
)
out.append("")
out.append("| Catégorie | Répertoire | Scripts | Reliés Taskfile | Documentés Diátaxis | Docstring / En-tête |")
out.append("| :--- | :--- | :---: | :---: | :---: | :---: |")

for cat, items in sorted(categories.items()):
    c_tot = len(items)
    c_tasks = sum(1 for i in items if i["tasks"])
    c_docs = sum(1 for i in items if i["docs"])
    c_ds = sum(1 for i in items if i["has_docstring"])
    rep = items[0]["path"].rsplit("/", 1)[0]
    out.append(f"| **{cat}** | `{rep}/` | {c_tot} | {c_tasks} | {c_docs} | {c_ds} |")

out.append("")
out.append("---")
out.append("")
out.append("## 2. Inventaire Détaillé par Sous-Système")
out.append("")

cat_order = ["tooling", "benchmarks", "e2e", "ingestion", "workers", "local", "o2switch", "rpi4", "scripts"]

for cat_name in cat_order:
    items = categories.get(cat_name)
    if not items:
        continue
    dir_name = items[0]["path"].rsplit("/", 1)[0]
    out.append(f"### 2.{cat_order.index(cat_name) + 1} Module `{dir_name}/` ({len(items)} scripts)")
    out.append("")
    for item in items:
        p = item["path"]
        out.append(f"#### `{p}`")
        out.append(f"- **But / Rôle** : {item['purpose']}")
        out.append(f"- **Environnement dédié** : `{item['env']}`")

        # Tasks
        if item["tasks"]:
            task_list = ", ".join([f"`task {t}`" for t in item["tasks"]])
            out.append(f"- **Recettes GoTasks associées** : {task_list}")
        else:
            out.append("- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*")

        # Docstrings
        ds_status = "✅ Présente" if item["has_docstring"] else "⚠️ Absente ou incomplète"
        out.append(f"- **Documentation interne (docstring / header)** : {ds_status}")

        # Diataxis
        if item["docs"]:
            doc_list = ", ".join([f"`{d}`" for d in item["docs"]])
            out.append(f"- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans {doc_list}")
        else:
            out.append(
                "- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)"
            )

        out.append("")

doc_path = Path("docs/reference/scripts-inventory-2026-11-10.md")
doc_path.write_text("\n".join(out), encoding="utf-8")
print(f"Wrote inventory to {doc_path} ({len(out)} lines)")
