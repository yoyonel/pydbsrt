# Inventaire Exhaustif des Scripts du Projet `pydbsrt`

> **Date de génération :** 2026-11-10 18:57:00 (Post-Refactor PR #70 / Issue #69 Phase 1)
> **Statut :** Audit exhaustif de 94 scripts opérationnels (Python & Bash) après modularisation sous `scripts/`.

## 1. Vue d'Ensemble et Statistiques Globales

- **Total scripts opérationnels audités** : 94
- **Scripts reliés à des cibles Taskfile (`task ...`)** : 68 / 94 (72.3%)
- **Scripts documentés dans Diátaxis (`docs/`)** : 14 / 94 (14.9%)
- **Scripts avec en-tête / docstring documentée** : 91 / 94 (96.8%)

| Catégorie | Répertoire | Scripts | Reliés Taskfile | Documentés Diátaxis | Docstring / En-tête |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **benchmarks** | `scripts/benchmarks/` | 16 | 12 | 4 | 15 |
| **e2e** | `scripts/e2e/` | 6 | 6 | 0 | 6 |
| **ingestion** | `scripts/ingestion/` | 27 | 18 | 4 | 25 |
| **local** | `scripts/infra/local/` | 8 | 7 | 0 | 8 |
| **o2switch** | `scripts/infra/o2switch/` | 3 | 2 | 1 | 3 |
| **rpi4** | `scripts/infra/rpi4/` | 11 | 9 | 0 | 11 |
| **scripts** | `scripts/` | 1 | 1 | 1 | 1 |
| **tooling** | `scripts/tooling/` | 18 | 12 | 3 | 18 |
| **workers** | `scripts/workers/` | 4 | 1 | 1 | 4 |

---

## 2. Inventaire Détaillé par Sous-Système

### 2.1 Module `scripts/tooling/` (18 scripts)

#### `scripts/tooling/build_dashboard_html.py`
- **But / Rôle** : Generates a rock-solid, responsive HTML dashboard for the E2E VLC tests.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/check_pr_body.py`
- **But / Rôle** : Fails a PR whose description lacks business goals or a prod proof (or why there is none).
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/code_quality.sh`
- **But / Rôle** : Unified code quality runner for lint and format across .py, .rs, .sh, .lua, .yml, .json, .html.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task coverage`, `task fmt`, `task lint`, `task typecheck`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/convert_dashboard_webp.py`
- **But / Rôle** : Batch converts PNG screenshots in docs/screenshots/e2e_vlc_all/ to 720p WebP
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task doc:optimize-images`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/disk_guard.sh`
- **But / Rôle** : Disk space safety harness & automated cleanup for pydbsrt.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task clean`, `task disk:check`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/generate_coverage_report.py`
- **But / Rôle** : Automated Coverage Aggregator & PR Reporter for pydbsrt.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/ci-cd.md`

#### `scripts/tooling/generate_nightly_report_enhanced.py`
- **But / Rôle** : Generates the enhanced Diátaxis-compliant TVShow Batch Ingestion Report.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/lint_complexity.py`
- **But / Rôle** : Automated Rust Code Complexity & Function Size Gate for pydbsrt.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task complexity`, `task complexity:report`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/ci-cd.md`

#### `scripts/tooling/lint_migrations.py`
- **But / Rôle** : Automated PostgreSQL Migration Safety & Compatibility Linter for pydbsrt.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task db:lint:safety`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/mkdocs_hooks.py`
- **But / Rôle** : MkDocs Lifecycle Hooks for pydbsrt Documentation Portal.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/pack_repo.sh`
- **But / Rôle** : Pack entire repository source code into an AI-friendly markdown bundle using repomix
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task repo:pack`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/pack_review.sh`
- **But / Rôle** : Pack branch diffs and modified files into an AI-friendly markdown bundle using repomix
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task review:pack`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/report_type_errors.py`
- **But / Rôle** : Statistical summary reporter for Pyright type checker diagnostics.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task typecheck:summary`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/serve_docs.py`
- **But / Rôle** : Intelligent launcher for MkDocs documentation server.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task doc`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/test_db_migrations.py`
- **But / Rôle** : Automated PostgreSQL Migration & Domain Lineage Integration Test Suite.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task db:test:migration`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/taskfile.md`

#### `scripts/tooling/test_recipe_debian.sh`
- **But / Rôle** : Automated Hermetic Integration Test for Debian/Ubuntu Deployment Recipe
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task test:recipe:debian`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/validate_docs.py`
- **But / Rôle** : Automated Documentation Quality & Visual Integrity Validator for pydbsrt.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task doc:check`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/tooling/validate_web_assets.py`
- **But / Rôle** : Validates HTML structure, embedded JavaScript syntax (via node --check), and JSON files.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

### 2.2 Module `scripts/benchmarks/` (16 scripts)

#### `scripts/benchmarks/bench_http_flood.sh`
- **But / Rôle** : Orchestrates end-to-end HTTP matching flood benchmark against local dbsrt-server
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task bench:flood`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/bench_server_comparison.py`
- **But / Rôle** : Objective comparative benchmark measuring dbsrt-server matching latency:
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/benchmark_worker_scaling.py`
- **But / Rôle** : Empirical Benchmark: Worker Scaling & GPU/CIFS Saturation for VA-API pHash Ingestion.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/compare_top_telemetry.py`
- **But / Rôle** : Simultaneous side-by-side telemetry comparison: Python vs Rust.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/eval_exhaustive_nas_snippets.py`
- **But / Rôle** : Exhaustive Evaluation & E2E Validation Matrix on 100% of the NAS Snippets Dataset.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task eval:nas-snippets`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/taskfile.md`

#### `scripts/benchmarks/eval_media_batch.py`
- **But / Rôle** : Generic batch media evaluation runner for pydbsrt.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/eval_multi_offset_matching.py`
- **But / Rôle** : Evaluation Harness comparing Mono-Offset (t=50%) vs Multi-Offset Spatiotemporal Consensus.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task eval:matching:multi-offset`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/taskfile.md`

#### `scripts/benchmarks/eval_nas_reliability.py`
- **But / Rôle** : Massive Quantitative Evaluation Benchmark for pydbsrt on NAS & PostgreSQL 17.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task eval:nas`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/eval_vlc_vlsub_batch.py`
- **But / Rôle** : Batch E2E Validation Runner: VLC GUI + VLSub Extension on Xvfb (:98).
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task vlc:eval-all`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/matching_bench_runner.py`
- **But / Rôle** : Autonomous Spatiotemporal Multi-Offset Matching Benchmark Runner.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task eval:matching:runner`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/record_casts.py`
- **But / Rôle** : Deterministic Asciinema Cast Generator for pydbsrt documentation.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task doc:record-casts`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/constants.md`

#### `scripts/benchmarks/record_proof.py`
- **But / Rôle** : Records a masked terminal proof (asciicast + animated SVG) to embed in a public PR.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task proof:record`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/record_vlsub_demo.sh`
- **But / Rôle** : N/A
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task vlc:record-demo`
- **Documentation interne (docstring / header)** : ⚠️ Absente ou incomplète
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/scale_up_nas_dataset.py`
- **But / Rôle** : Massive NAS Dataset Scale-Up & Ground-Truth Certification Pipeline.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task eval:scale-up`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/trajectory_bench_runner.py`
- **But / Rôle** : Autonomous Spatiotemporal Trajectory Tracking Batch Runner.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task eval:trajectory:batch`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/benchmarks/verify_terminal_rendering.py`
- **But / Rôle** : Formal Browser Verification for Asciinema Terminal Recordings in MkDocs.
- **Environnement dédié** : `Banc d'évaluation (Local / Workstation)`
- **Recettes GoTasks associées** : `task doc:verify-terminal`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/constants.md`

### 2.3 Module `scripts/e2e/` (6 scripts)

#### `scripts/e2e/e2e_portal_exhaustive.py`
- **But / Rôle** : Automated exhaustive Playwright E2E test for the entire pydbsrt Portal.
- **Environnement dédié** : `Playwright / Headless Browser`
- **Recettes GoTasks associées** : `task portal:e2e`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/e2e/e2e_portal_playwright.py`
- **But / Rôle** : E2E Playwright validation of the dbsrt monitoring and studio portals.
- **Environnement dédié** : `Playwright / Headless Browser`
- **Recettes GoTasks associées** : `task test:portal:e2e`, `task test:portal:e2e:cloud`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/e2e/e2e_sources_explorer.py`
- **But / Rôle** : Automated Playwright E2E test for the Media Sources and Master-Detail Explorer views.
- **Environnement dédié** : `Playwright / Headless Browser`
- **Recettes GoTasks associées** : `task portal:e2e`, `task test:sources:e2e`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/e2e/e2e_trajectories_formal.py`
- **But / Rôle** : Automated E2E formal test for Trajectories and Consensus across all valid reports.
- **Environnement dédié** : `Playwright / Headless Browser`
- **Recettes GoTasks associées** : `task portal:e2e`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/e2e/o2switch_e2e.py`
- **But / Rôle** : E2E recette of dbsrt-server on o2switch: lifecycle, respawn paths, cron and public gateway.
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : `task o2switch:e2e`, `task o2switch:e2e:reset-golden`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/e2e/recette_top_e2e.py`
- **But / Rôle** : E2E Recette & ASCIInema Session Recorder for Native Rust top TUI.
- **Environnement dédié** : `Playwright / Headless Browser`
- **Recettes GoTasks associées** : `task top:recette`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

### 2.4 Module `scripts/ingestion/` (27 scripts)

#### `scripts/ingestion/audit_cuts_pipeline.py`
- **But / Rôle** : Comprehensive performance audit of the 6-stage matching pipeline across all video cuts/snippets.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task eval:cuts:audit`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/taskfile.md`

#### `scripts/ingestion/audit_phash_catalog.py`
- **But / Rôle** : Exhaustive empirical audit script for pydbsrt pHash recognition catalog.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task dataset:audit`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/audit_rejected_cuts.py`
- **But / Rôle** : Exhaustive forensic diagnostic of all 849 rejected cuts in the matching pipeline.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task eval:cuts:audit:rejected`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/taskfile.md`

#### `scripts/ingestion/backfill_media_assets.py`
- **But / Rôle** : Backfills media_sources and populates V6 asset lineage columns in medias table.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task db:backfill:assets`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/backfill_medias_pg.py`
- **But / Rôle** : Backfill and synchronize rich media metadata into PostgreSQL 17 'medias' table.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task db:backfill:medias`, `task rpi4:db:backfill:medias`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/batch_media_add_tvshow.py`
- **But / Rôle** : Direct NAS TVShow Batch Ingestion & Profiling (Zero Local Video Copy).
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task media:batch:tvshow`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/constants.md`

#### `scripts/ingestion/coupling_relabel.sh`
- **But / Rôle** : scripts/coupling_relabel.sh: Thin orchestrator for dbsrt coupling:relabel CLI subcommand
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task coupling:relabel`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/download_and_audit_all_trailers.py`
- **But / Rôle** : Exhaustive Remote Video Audit: Downloads public trailers/clips from YouTube for catalog works
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task eval:trailers`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/enrich_snippets_metadata.py`
- **But / Rôle** : Enriches NAS video snippets with structured metadata for database insertion.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task nas:snippets:metadata`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/extract_embedded_subs.py`
- **But / Rôle** : Extracts and verifies embedded subtitles from NAS video files matching snippets.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/extract_nas_snippets.py`
- **But / Rôle** : Extracts diverse 15-second video snippets and computes pHashes from Synology NAS.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task nas:extract:snippets`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/fill_nas_snippets_catalog.py`
- **But / Rôle** : Fills missing local 15-second video snippets from Synology NAS and Freebox.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task nas:snippets:fill`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/init_couplings_seed.sh`
- **But / Rôle** : Copies the CI seed fixture to data/SRT/couplings.json if absent.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/ci-cd.md`

#### `scripts/ingestion/media_add.sh`
- **But / Rôle** : N/A
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task media:add`
- **Documentation interne (docstring / header)** : ⚠️ Absente ou incomplète
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/phash_dedupe.py`
- **But / Rôle** : Drop a legacy-named .phash once its moviehash-named twin sits in the same directory.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/phash_names.py`
- **But / Rôle** : Logical name of a .phash file, legacy-named or named by moviehash with its sidecar.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/probe_medias_metadata.py`
- **But / Rôle** : Media Stream Physical Specs Prober and Null Audit for pydbsrt.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task db:audit`, `task db:probe`, `task rpi4:db:audit`, `task rpi4:db:probe`, `task rpi4:db:sync`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/purge_media.py`
- **But / Rôle** : Unified media purge script for pydbsrt.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/quarantine_truncated_phashes.py`
- **But / Rôle** : Quarantines truncated, corrupted, and undersized .phash files from data/__PHASH__/ into data/__QUARANTINE_PHASH__/.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task phash:quarantine`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/recognize_medias.sh`
- **But / Rôle** : N/A
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ⚠️ Absente ou incomplète
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/reconcile_phash_filenames.py`
- **But / Rôle** : Reconciles release-named .phash files into canonical <moviehash>.<fps>fps.phash + sidecar JSON.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task phash:reconcile`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/register_adjacent_couplings.py`
- **But / Rôle** : Registers verified adjacent .srt files into data/SRT/couplings.json.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/repair_corrupted_couplings.py`
- **But / Rôle** : Repairs corrupted subtitle cue text (U+FFFD replacement characters) in couplings.json.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/scan_freebox_targets.py`
- **But / Rôle** : Scans mounted Freebox disks to discover eligible movies and TV shows for pydbsrt ingestion.
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/scan_nas_ftp.py`
- **But / Rôle** : Robust, resilient Synology NAS FTP scanner with explicit timeout and error tracking.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task nas:ftp:scan`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/seed_couplings.sh`
- **But / Rôle** : Auto-discovers and certifies ground-truth (video/.phash <-> .srt) couplings
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task coupling:certify`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/ingestion/verify_catalog_integrity.py`
- **But / Rôle** : Comprehensive integrity audit and diagnostic validator for pydbsrt catalog and storage tiers.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task catalog:verify`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

### 2.5 Module `scripts/workers/` (4 scripts)

#### `scripts/workers/o2switch_batch_summary.py`
- **But / Rôle** : Counts-only summary of the o2switch batch worker, safe to publish (no media name, no path, no URL).
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/workers/o2switch_batch_worker.py`
- **But / Rôle** : o2switch Remote Batch Processing Worker.
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/workers/o2switch_fault_watch.py`
- **But / Rôle** : Per-minute proxies of o2switch LVE CPU faults, joinable with the cPanel "Resource Usage" table.
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : `task o2switch:faultwatch:correlate`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/formats-fichiers.md`

#### `scripts/workers/o2switch_phash_migrate.py`
- **But / Rôle** : Renames legacy <media file>.<fps>fps.phash to <moviehash>.<fps>fps.phash and writes its <moviehash>.json sidecar.
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

### 2.6 Module `scripts/infra/local/` (8 scripts)

#### `scripts/infra/local/agent_relay.sh`
- **But / Rôle** : Controls the local dbsrt client relay agent connected to Cloud o2switch
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task o2switch:agent:demo`, `task o2switch:agent:logs`, `task o2switch:agent:restart`, `task o2switch:agent:start`, `task o2switch:agent:status`, `task o2switch:agent:stop`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/local/dbsrt_top.py`
- **But / Rôle** : dbsrt top: Real-time TUI Process & Ingestion Monitor (ctop/btop style).
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task media:top:py`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/local/distributed_dev_stack.sh`
- **But / Rôle** : Dual-process Distributed Dev Stack Controller
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task stack:distributed:down`, `task stack:distributed:status`, `task stack:distributed:up`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/local/install_debian.sh`
- **But / Rôle** : Installation & Lifecycle Manager for Debian/Ubuntu (Priority #1 target)
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task client:linux:install`, `task client:linux:logs`, `task client:linux:restart`, `task client:linux:start`, `task client:linux:status`, `task client:linux:stop`, `task client:linux:uninstall`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/local/load_env.sh`
- **But / Rôle** : Sourced helper: loads the optional unversioned local env file (<repo>/.env, then
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/local/stack_ctl.sh`
- **But / Rôle** : Unified daemon and health controller for dbsrt-server, MkDocs, PgWeb, Prometheus, and Grafana.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task catalog:compact`, `task catalog:ingest`, `task catalog:reload`, `task catalog:status`, `task doc:daemon`, `task doc:logs`, `task doc:stop`, `task server:daemon`, `task server:logs`, `task server:reload`, `task server:status`, `task server:stop`, `task status`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/local/switch_target.sh`
- **But / Rôle** : Switches the active backend endpoint for the local VLC client
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task target:local`, `task target:o2switch`, `task target:rpi4`, `task target:status`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/local/vlc_ctl.sh`
- **But / Rôle** : Helper script for VLC Lua plugin installation and headless cvlc E2E validation.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task vlc:demo`, `task vlc:install-plugin`, `task vlc:run`, `task vlc:test-cvlc`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

### 2.7 Module `scripts/infra/o2switch/` (3 scripts)

#### `scripts/infra/o2switch/nas_o2switch_sync.sh`
- **But / Rôle** : Synchronizes Synology NAS .phash fingerprints, compiled binary catalog,
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : `task o2switch:sync-nas`, `task o2switch:sync-nas:dry-run`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/o2switch/o2switch_audit_remote.py`
- **But / Rôle** : Catalog self-recognition audit, run ON o2switch (cwd ~/dbsrt, ./bin/python3 = 3.13) by o2switch_ctl.sh audit.
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/o2switch/o2switch_ctl.sh`
- **But / Rôle** : Controls deployment, sync, lifecycle, and telemetry of dbsrt-server on o2switch (Cloud cPanel)
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : `task o2switch:audit:burst`, `task o2switch:audit:report`, `task o2switch:audit:start`, `task o2switch:audit:status`, `task o2switch:audit:stop`, `task o2switch:backup-nas`, `task o2switch:batch:logs`, `task o2switch:batch:movies`, `task o2switch:batch:pause`, `task o2switch:batch:resume`, `task o2switch:batch:scale`, `task o2switch:batch:status`, `task o2switch:batch:stop`, `task o2switch:batch:summary`, `task o2switch:batch:sync`, `task o2switch:batch:tvshow`, `task o2switch:batch:workers`, `task o2switch:build`, `task o2switch:demo`, `task o2switch:deploy`, `task o2switch:faultwatch:pull`, `task o2switch:faultwatch:start`, `task o2switch:faultwatch:status`, `task o2switch:faultwatch:stop`, `task o2switch:logs`, `task o2switch:monitoring`, `task o2switch:phash:migrate`, `task o2switch:pull-catalog`, `task o2switch:releases`, `task o2switch:remove-cron`, `task o2switch:restart`, `task o2switch:rollback`, `task o2switch:rotate-key`, `task o2switch:rotate-readonly-key`, `task o2switch:setup-cron`, `task o2switch:start`, `task o2switch:status`, `task o2switch:stop`, `task o2switch:sync`, `task o2switch:test-local`, `task o2switch:top`, `task o2switch:trajectory:logs`, `task o2switch:trajectory:pause`, `task o2switch:trajectory:pull`, `task o2switch:trajectory:resume`, `task o2switch:trajectory:run`, `task o2switch:trajectory:start`, `task o2switch:trajectory:status`, `task o2switch:trajectory:stop`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/reference/configuration.md`

### 2.8 Module `scripts/infra/rpi4/` (11 scripts)

#### `scripts/infra/rpi4/freebox_batch_ctl.sh`
- **But / Rôle** : Controls Freebox batch ingestion on o2switch, streaming from the RPi4 HTTP gateway
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task freebox:batch:start`, `task freebox:batch:status`, `task freebox:batch:stop`, `task freebox:scan`, `task freebox:test`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_build.sh`
- **But / Rôle** : Builds dbsrt-server ARM64 Docker image with Bookworm glibc parity
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task rpi4:build`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_cli.sh`
- **But / Rôle** : Manages cross-compilation, local ARM64 simulation, and deployment of dbsrt CLI on Raspberry Pi 4
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task rpi4:cli:build`, `task rpi4:cli:deploy`, `task rpi4:cli:test`, `task rpi4:top`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_deploy.sh`
- **But / Rôle** : Controls deployment and lifecycle of dbsrt-server on Raspberry Pi 4
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task rpi4:db:build`, `task rpi4:db:down`, `task rpi4:db:logs`, `task rpi4:db:migrate`, `task rpi4:db:status`, `task rpi4:db:up`, `task rpi4:demo`, `task rpi4:down`, `task rpi4:logs`, `task rpi4:push`, `task rpi4:status`, `task rpi4:up`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_freebox_ctl.sh`
- **But / Rôle** : Manages the Freebox CIFS mounts and the HTTP Range gateway on the RPi4.
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task freebox:batch:restart`, `task freebox:install`, `task freebox:mount`, `task freebox:restart`, `task freebox:start`, `task freebox:status`, `task freebox:stop`, `task freebox:unmount`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_freebox_gateway.py`
- **But / Rôle** : Lightweight HTTP Streaming Gateway for Freebox Volumes with Byte-Range Support.
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_o2switch_supervisor.py`
- **But / Rôle** : RPi4 Autonomous Watchdog Supervisor for o2switch Remote Batch Ingestion.
- **Environnement dédié** : `o2switch (Cloud / SSH)`
- **Recettes GoTasks associées** : *Aucune (script autonome / CLI direct / sous-processus)*
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_sim.sh`
- **But / Rôle** : Manages local ARM64 simulation of dbsrt-server on x86 host via QEMU
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task sim:down`, `task sim:logs`, `task sim:status`, `task sim:test`, `task sim:up`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_standalone.sh`
- **But / Rôle** : Controls deployment and lifecycle of standalone dbsrt-server on Raspberry Pi 4 (systemd user service)
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task rpi4:server:demo`, `task rpi4:server:deploy`, `task rpi4:server:logs`, `task rpi4:server:restart`, `task rpi4:server:start`, `task rpi4:server:status`, `task rpi4:server:stop`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_supervisor_ctl.sh`
- **But / Rôle** : Controls deployment, lifecycle, and telemetry of the autonomous RPi4 supervisor watchdog
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task rpi4:supervisor:install`, `task rpi4:supervisor:logs`, `task rpi4:supervisor:restart`, `task rpi4:supervisor:start`, `task rpi4:supervisor:status`, `task rpi4:supervisor:stop`, `task rpi4:supervisor:sync`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

#### `scripts/infra/rpi4/rpi4_sync.sh`
- **But / Rôle** : Synchronizes catalogue and subtitles to Raspberry Pi 4
- **Environnement dédié** : `Raspberry Pi 4 / Freebox`
- **Recettes GoTasks associées** : `task rpi4:sync`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ❌ Non référencé directement (Candidat à la vague de mise à jour doc)

### 2.9 Module `scripts/` (1 scripts)

#### `scripts/portal_server.py`
- **But / Rôle** : Unified light HTTP backend for pydbsrt MediaSources Portal.
- **Environnement dédié** : `Local / CLI`
- **Recettes GoTasks associées** : `task db:portal`
- **Documentation interne (docstring / header)** : ✅ Présente
- **Documentation Diátaxis (`docs/`)** : ✅ Référencé dans `docs/explanation/2026-11-09-architecture-portail-medias-et-sources-multi-environnements.md`, `docs/explanation/2026-11-09-reconciliation-actifs-medias-et-empreintes-phash.md`, `docs/how-to/2026-11-09-diagnostiquer-sources-medias-multi-environnements.md`, `docs/reference/2026-11-09-api-portail-sources-medias.md`
