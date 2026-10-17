---
title: Portail de Documentation pydbsrt & dbsrt-rs
---

# 🚀 Portail Technique : pydbsrt & dbsrt-rs

Bienvenue sur le portail technique unifié du projet **pydbsrt** (moteur v1 Python/PostgreSQL) et **dbsrt-rs** (moteur v2 Rust Zero-DB / VLC Daemon).

Ce centre de documentation regroupe l'ensemble des études d'ingénierie, architectures matérielles, benchmarks de performance, rapports d'audit et feuilles de route de passage à l'échelle (1920-2026).

---

## 📑 Matrice de Navigation et Statuts

Utilisez cette matrice interactive ou la barre de recherche (`F` ou `S`) pour filtrer par statut, thématique ou millésime.

| Document | Date | Domaine | Statut | Tags Clés | Résumé Synthétique |
| :--- | :--- | :--- | :--- | :--- | :--- |
| [Étude Comparative : Frames, Cuts, Keyframes & Hybride](2026-10-17-etude-comparative-identification-frames-cuts-keyframes-hybride.md) | `2026-10-17` | **Benchmarks** | :octicons-check-circle-fill-16: **CURRENT** | `dbphash`, `hybrid`, `cuts`, `keyframes` | Étude sur 5,08M frames (125 épisodes) & 5 encodages : écueil I-frames (27% GOP VOD), format `.dbphash` v2.0 Hybride (cuts + 1 fps, 0% échec, 6.3x compression). |
| [Spécification Axe 5 : Providers Distants](2026-10-16-axe5-remote-subtitles-providers.md) | `2026-10-16` | **Architecture** | :octicons-rocket-16: **TODO** | `providers`, `opensubtitles`, `vlc` | Architecture des connecteurs distants REST (OpenSubtitles v1), scoring heuristique, cache disque et fallback autonome. |
| [Feuille de Route & Échelle (1920-2026)](2026-10-15-deep-dive-modernization-and-scaling-roadmap.md) | `2026-10-15` | **Roadmap** | :octicons-rocket-16: **TODO** | `roadmap`, `scaling`, `hwaccel`, `ransac` | Étude approfondie des 4 axes : décodage HW sub-seconde, régression 2D drift PAL/NTSC, index 1M vidéos (13 Go), rétro-ingénierie VLSub et extraction MKV. |
| [Intégration VLC & Daemon Axum](2026-10-15-vlc-integration-daemon-and-plugins.md) | `2026-10-15` | **Architecture** | :octicons-check-circle-fill-16: **CURRENT** | `vlc`, `axum`, `daemon`, `lua`, `osd` | Daemon REST Axum, découverte dynamique `/tmp/dbsrt-server.json`, plugins Lua GUI & CLI (`cvlc`), calage et injection OSD < 3 ms. |
| [Moteur Zero-DB mmap](2026-10-15-scaling-architecture-rust-mmap.md) | `2026-10-15` | **Architecture** | :octicons-check-circle-fill-16: **CURRENT** | `mmap`, `zero-db`, `rust`, `simd` | Pourquoi Python a échoué face à l'échelle (PyObject/GIL/GC). Moteur Rust mmap zero-copy (8 Go pour 1 milliard de frames). |
| [Benchmarks E2E Rust vs Python](2026-10-15-rust-e2e-benchmarks-and-tradeoffs.md) | `2026-10-15` | **Benchmarks** | :octicons-check-circle-fill-16: **CURRENT** | `benchmarks`, `perf`, `rust`, `simd` | Mesures réelles : pHash 72k fps (+1200x), recognize-media (+15x), retarget SRT en 0.5 ms. Analyse des trade-offs. |
| [Migration PG17 & BK-Tree](2026-10-15-pg17-bktree-benchmark-migration.md) | `2026-10-15` | **Benchmarks** | :octicons-check-circle-fill-16: **CURRENT** | `postgres`, `pg17`, `bktree`, `pgvector` | Validation de l'extension C SP-GiST BK-Tree sur PostgreSQL 17. Comparatif avec pgvector HNSW. |
| [Modernization Blueprint](2026-10-15-modernization-blueprint.md) | `2026-10-15` | **Roadmap** | :octicons-check-circle-fill-16: **CURRENT** | `uv`, `taskfile`, `hatchling`, `ruff` | Plan directeur de bascule : suppression Poetry/Make au profit de UV, Taskfile, Rust workspace. |
| [Audit Global du Dépôt](2026-10-15-audit-global.md) | `2026-10-15` | **Audit** | :octicons-archive-16: **OLD** | `audit`, `legacy`, `python`, `security` | Inventaire complet initial du dépôt legacy (2020-2023), dette technique, failles et dépendances. |
| [Notes 2021 Fast Hamming Postgres](history/2021-04-13-fast-hamming-postgres.md) | `2021-04-13` | **Archives** | :octicons-archive-16: **OLD** | `history`, `cython`, `spgist`, `postgres` | Notes de recherche initiales sur l'indexation de distance de Hamming dans Postgres (SP-GiST et Cython cBKTree). |

---

## 🛠️ Commandes du Serveur de Documentation

```bash
# Démarrer le serveur de doc local avec rechargement à chaud (Hot-Reload)
task doc:serve

# Compiler le site statique autonome (HTML/CSS/JS minifié)
task doc:build
```

Le serveur s'exécute par défaut sur [http://127.0.0.1:8000](http://127.0.0.1:8000).

---

## 🧭 Navigation par Tags & Filtres

Accédez à l'**[Index exhaustif des Tags et Filtres](tags.md)** pour explorer les documents groupés par statut (`current`, `todo`, `old`), par composant technique (`rust`, `vlc`, `postgres`, `simd`, `mmap`) ou par millésime (`2021`, `2026`).
