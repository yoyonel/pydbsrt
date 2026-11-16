---
title: "ADR-0009 : Modularisation du Portail Sources Médias vers src/pydbsrt/portal/ & Façade Zero-Pip"
tags:
  - adr
  - decisions
  - portal
  - soc
  - o2switch
  - architecture
---

# ADR-0009 : Modularisation du Portail Sources Médias vers `src/pydbsrt/portal/` & Façade Zero-Pip

- **ID** : `ADR-0009`
- **Date** : 2026-11-16
- **Statut** : Accepté (Issue #72)
- **Périmètre** : `src/pydbsrt/portal/`, `scripts/portal_server.py`, `scripts/infra/o2switch/o2switch_ctl.sh`, `Taskfile.yml`, `taskfiles/quality.yml`, `pyproject.toml`
- **Journal d'Ingénierie associé** : [Modularisation Portail Sources, Pièges `monkeypatch` & Isolation Zero-Pip o2switch](../explanation/2026-11-16-modularisation-portail-sources-et-pieges-ci-o2switch.md)

---

## 1. Contexte & Contraintes Système

Le serveur du Portail Sources Médias (`scripts/portal_server.py`) avait atteint **`3 768` LOC** dans un fichier unique regroupant la configuration réseau, les caches L1/L2, les sondes CIFS/FTP/HTTP, le parsing `ffprobe`, les requêtes PostgreSQL (`psycopg2` + fallback `psql` CLI), la réconciliation tripartite `.phash` / `.dbphash`, la microscopie d'entropie/cuts, le suivi de trajectoires JSONL, les routes HTTP et la CLI.

Cette migration vers un package modulaire devait respecter quatre contraintes impératives :

1. **Compatibilité Zero-Pip sur `o2switch`** : sur l'hébergement mutualisé distant, `deploy/o2switch/index.php` invoque `~/dbsrt/bin/python3 ~/dbsrt/scripts/portal_server.py --metadata` sans environnement virtuel `pip` complet (`asyncpg`, `sqlalchemy`, `imageio` absents).
2. **Rétrocompatibilité CGI, Skills & Tests** : aucun appelant existant (`index.php`, documentation opérationnelle, `tests/test_portal_server.py`) ne devait casser.
3. **Résolution indépendante du CWD et de la profondeur** : les modules profonds (`src/pydbsrt/portal/services/*.py`) sont au niveau 4 de l'arborescence, là où l'ancien `Path(__file__).resolve().parents[1]` pointerait à tort vers `src/pydbsrt/` au lieu de la racine du dépôt.
4. **Désambiguïsation Taskfile** : `portal:e2e` entrait en collision entre le portail Python (`:8780`) et le portail Rust `dbsrt-server` (`:8765`).

---

## 2. Options Évaluées

1. **Option 1 — Sous-package direct sous `scripts/portal/`** : **Rejetée**. Maintient une logique applicative de premier rang dans `scripts/` et contourne l'organisation standard `src/pydbsrt/`.
2. **Option 2 — Migration vers `src/pydbsrt/portal/` avec suppression de `scripts/portal_server.py`** : **Rejetée**. Casserait `deploy/o2switch/index.php`, la documentation existante et tous les appels CGI sans déploiement atomique simultané de PHP et Python.
3. **Option 3 — Package en couches `src/pydbsrt/portal/` (`__init__.py` racine `0 octet`) + Shim rétrocompatible `scripts/portal_server.py`** : **Retenue**.
   - `src/pydbsrt/__init__.py` reste vide (`0` octet) : importer `pydbsrt.portal.*` ne déclenche aucun import des dépendances lourdes legacy de `src/pydbsrt/` et reste 100 % stdlib Python + `scripts.data_access`.
   - `scripts/portal_server.py` devient une façade légère (~67 LOC) qui injecte `REPO_ROOT` et `REPO_ROOT / "src"` dans `sys.path` puis ré-exporte l'API publique.

---

## 3. Décision & Architecture Retenue

1. **DAG strict sans cycle** :
   - Transport : `cli.py`, `server.py`, `routes/{common,sources,assets,phashes,trajectories}.py` typés via `PortalHandlerProtocol` (`0` `# noqa: E402`, `0` erreur `mypy --strict`).
   - Services métier : `services/{secrets_env,ftp_parsers,probe_service,listing_service,ffprobe_service,assets_service,phash_reconciliation,phash_microscopy,trajectory_service}.py`.
   - Socle transverse : `config.py`, `models.py`, `cache.py` + actif statique `static/sources.html` déplacé par `git mv`.
2. **Résolution racine dynamique (`find_repo_root()`)** : remonte `Path(__file__).resolve().parents` jusqu'à trouver `scripts/data_access`, `Taskfile.yml` ou `.git` (compatible local, worktree Git et déploiement `o2switch` sans `.git`), couplée à `_resolve_data_path()` pour préserver les isolations `chdir(tmp_path)` des tests.
3. **Synchronisation `o2switch_ctl.sh sync`** : copie `src/pydbsrt/__init__.py`, `src/pydbsrt/portal/` et `src/pydbsrt/portal/static/sources.html` vers `~/dbsrt/web/sources.html`.
4. **Gates qualité & Séparation Taskfile** : `pyproject.toml` étend `[tool.complexipy]` (`<= 15`) à `src/pydbsrt/portal` ; `Taskfile.yml` expose `task sources:e2e` *(alias `portal:e2e`)* via le runner auto-portant `scripts/e2e/run_sources_e2e.sh` et `task sources:check`, tandis que `taskfiles/quality.yml` isole `task test:portal:e2e` *(alias `server:e2e`)* pour le serveur Rust `:8765`.

---

## 4. Conséquences & Limites Résiduelles

- **Positif** : chaque module respecte `complexipy <= 15`, `pyright standard` (`0` erreur), `mypy --strict` (`0` erreur sur `src/pydbsrt/portal/`), et atteint **89.47 %** en `pytest` seul (`24/24` verts) et **90.65 %** en combiné `pytest` + `task sources:e2e`.
- **Limites résiduelles** : `src/pydbsrt/portal/static/sources.html` reste monolithique (Alpine.js / Tailwind CSS) dans le cadre de ce ticket ; les fallbacks `except Exception` silencieux et le mode `psql` CLI hérités du monolithe restent documentés pour un durcissement ultérieur.
