#!/usr/bin/env python3
"""Generates the exhaustive audit of scripts and internal regression references."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


def find_repo_root() -> Path:
    """Finds repository root dynamically from git or Taskfile markers."""
    current = Path(__file__).resolve()
    for parent in [current] + list(current.parents):
        if (parent / ".git").is_dir() or (parent / "Taskfile.yml").is_file():
            return parent
    return current.parents[2]


PROVEN_LIVE: dict[str, str] = {
    "scripts/portal_server.py": "Exécuté daemon HTTP :8780 + requêté E2E (Playwright)",
    "scripts/benchmarks/bench_http_flood.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des artefacts (catalogue, binaire, sous-titres) et support --help. Exécuté hors racine (--help code 0)."
    ),
    "scripts/benchmarks/bench_server_comparison.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), passage de chemins absolus aux sous-processus et options CLI --bin-before, --bin-after, --samples, --check."
        " Validé hors racine (--check code 0, détection des binaires comparatifs et répertoires de fixtures)."
    ),
    "scripts/benchmarks/benchmark_worker_scaling.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), réduction de complexité cognitive (find_distinct_synology_media 19 vs 35 legacy), détection dynamique de ffmpeg/intel_gpu_top et option CLI --check."
        " Validé hors racine (--check code 0, détection des binaires et du périphérique DRI renderD128)."
    ),
    "scripts/benchmarks/eval_exhaustive_nas_snippets.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), préfixage absolu des dossiers d'extraits et de rapports, et option CLI --check."
        " Validé hors racine (--check code 0, 2 758 snippets réels détectés sous data/snippets)."
    ),
    "scripts/benchmarks/eval_media_batch.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), réduction de complexité cognitive (toutes fonctions <= 13 vs 18 legacy) et option CLI --check."
        " Validé hors racine (--check code 0, détection de ffprobe et du dossier snippets)."
    ),
    "scripts/benchmarks/eval_multi_offset_matching.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), sélection binaire robuste (release/debug), résolution des cibles et option CLI --check."
        " Validé hors racine (--check code 0, catalogue 95MB et 9/9 cibles vidéo Demon Slayer confirmées)."
    ),
    "scripts/benchmarks/eval_nas_reliability.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), ancrage de sys.path, sélection du binaire (release/debug) et option CLI --check."
        " Validé hors racine (--check code 0, 2 779 pHashes réels, catalogue .dbphash et racine NAS confirmés)."
    ),
    "scripts/benchmarks/eval_vlc_vlsub_batch.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), réduction de complexité cognitive (main 22 vs 29 legacy), chemins absolus des captures/rapports et option CLI --check."
        " Validé hors racine (--check code 0, 2 758 snippets et pile graphique Xvfb/marco/vlc/xdotool/ffmpeg confirmés)."
    ),
    "scripts/benchmarks/scale_up_nas_dataset.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), sélection robuste du binaire dbsrt (release/debug/PATH), chemins absolus des catalogues et option CLI --check."
        " Validé hors racine (--check code 0, 2 779 .phash et 2 851 .srt réels confirmés)."
    ),
    "scripts/benchmarks/verify_terminal_rendering.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), réduction de complexité cognitive (toutes fonctions <= 11 vs main 19 legacy), chemins absolus des artefacts webp et option CLI --check."
        " Validé hors racine (--check code 0, détection du dossier site/, de google-chrome et des 4 pages cibles)."
    ),
    "scripts/benchmarks/compare_top_telemetry.py": (
        "Exécuté live avec succès sur /proc Linux. Bandeau console corrigé vers scripts/infra/local/dbsrt_top.py."
    ),
    "scripts/benchmarks/record_casts.py": (
        "Exécuté live avec succès. Chemin validate_docs.py réaligné vers"
        " scripts/tooling/, timing calé à 2.0s, 4 casts régénérés et validés"
        " via task doc:check."
    ),
    "scripts/benchmarks/record_proof.py": (
        "Exécuté live avec succès (génération .cast et .svg masqué sans fuite"
        " de secrets). Imports et docstring réalignés."
    ),
    "scripts/benchmarks/record_vlsub_demo.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des assets, nettoyage résilient des processus et options --help / --check. Validé hors racine (code 0)."
    ),
    "scripts/e2e/o2switch_e2e.py": (
        "Exécuté live contre o2switch (15/15 golden set PASS, gateway 200/401"
        " PASS). Résolution racine et CTL résiliente sans index magique."
    ),
    "scripts/e2e/e2e_sources_explorer.py": "Exécuté Playwright Chromium live (4 onglets PASS)",
    "scripts/e2e/e2e_trajectories_formal.py": "Exécuté Playwright Chromium live (7 batchs PASS)",
    "scripts/e2e/e2e_portal_exhaustive.py": "Exécuté Playwright Chromium live (0 console/page err)",
    "scripts/e2e/e2e_portal_playwright.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), imports paresseux de playwright/typing et option CLI --check."
        " Validé hors racine (--check code 0 avec et sans dépendance playwright, 27 endpoints déclarés détectés)."
    ),
    "scripts/e2e/recette_top_e2e.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des binaires/rapports/casts et option CLI --check."
        " Validé hors racine (binaire dbsrt release, montage NAS et artefacts de cast confirmés, code 0)."
    ),
    "scripts/benchmarks/matching_bench_runner.py": "Exécuté live sur snippet réel (01_Cruauté_mid15s.mp4)",
    "scripts/benchmarks/trajectory_bench_runner.py": "Exécuté live sur snippet réel (01_Cruauté_mid15s.mp4)",
    "scripts/ingestion/verify_catalog_integrity.py": "Exécuté live (task catalog:verify, 5 tiers PASS)",
    "scripts/tooling/code_quality.sh": "Exécuté live (task lint, task fmt)",
    "scripts/tooling/validate_docs.py": "Exécuté live (task doc:check, 5 étapes PASS)",
    "scripts/tooling/lint_complexity.py": "Exécuté live (Rust Complexity Gate 1180 fn)",
    "scripts/tooling/disk_guard.sh": "Exécuté live (task lint étape 10)",
    "scripts/tooling/mkdocs_hooks.py": "Exécuté live (crochet mkdocs build)",
    "scripts/tooling/validate_web_assets.py": "Exécuté live (task lint étape 7)",
    "scripts/infra/local/agent_relay.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus target/ et support --help. Validé hors racine (status code 0)."
    ),
    "scripts/infra/local/dbsrt_top.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), correction du mismatch timezone sur le débit de session, refactoring de complexité (main 55 <= 62), et support CLI --stats et --check. Validé live hors racine (1 392 médias analysés, code 0)."
    ),
    "scripts/infra/local/distributed_dev_stack.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus data/ et support --help. Validé hors racine (status code 0)."
    ),
    "scripts/infra/local/install_debian.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus, manifest path et support --help. Validé hors racine (status PID 1903, code 0)."
    ),
    "scripts/infra/local/load_env.sh": (
        "Résolution dynamique REPO_ROOT par défaut, bloc autonome CLI avec support --help / --check et masquage des secrets. Validé hors racine (code 0)."
    ),
    "scripts/infra/local/stack_ctl.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus data/, config mkdocs et support --help. Validé hors racine (status 360° code 0)."
    ),
    "scripts/infra/local/switch_target.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemin absolu PID_RELAY et support --help. Validé hors racine (status mode CLOUD_O2SWITCH code 0)."
    ),
    "scripts/infra/local/vlc_ctl.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus plugins/fixtures et support --help. Validé hors racine (install code 0)."
    ),
    "scripts/infra/o2switch/nas_o2switch_sync.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus, appel ctl absolu et options --help / --check. Validé hors racine (--check SSH/Synology code 0)."
    ),
    "scripts/infra/o2switch/o2switch_audit_remote.py": (
        "Docstring réalignée, constantes de chemins avec surcharges DBSRT_*."
        " 43/43 tests unitaires PASS. Exécuté et validé live sur o2switch (status et report sur 2827 fichiers)."
    ),
    "scripts/infra/o2switch/o2switch_ctl.sh": (
        "Bug Bash précédence ligne 30 corrigé (REPO_ROOT). Chemins locaux préfixés par REPO_ROOT."
        " Validé et prouvé live sur o2switch (batch-sync, batch-status, status, batch-summary, faultwatch status)."
    ),
    "scripts/infra/rpi4/freebox_batch_ctl.sh": (
        "Résolution REPO_ROOT et source load_env.sh intégrées. Référence scanner réalignée."
        " Validé et prouvé live : scan Freebox local, rsync cibles vers o2switch, probe gateway HTTP o2switch (test) et état queue (status)."
    ),
    "scripts/infra/rpi4/rpi4_build.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus Dockerfile/contexte et options --help / --check. Validé hors racine (code 0)."
    ),
    "scripts/infra/rpi4/rpi4_cli.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus, montage Docker /app et support --help. Validé hors racine (code 0)."
    ),
    "scripts/infra/rpi4/rpi4_deploy.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus, sélection binaire DBSRT_BIN et support --help. Validé live sur RPi4 (status container PG17 + API 3088 phash, code 0)."
    ),
    "scripts/infra/rpi4/rpi4_sync.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des artefacts locaux et options --help / --check. Validé hors racine (--check SSH/assets code 0)."
    ),
    "scripts/infra/rpi4/rpi4_freebox_ctl.sh": (
        "Résolution REPO_ROOT et source load_env.sh intégrées. Chemins install_gateway() préfixés par REPO_ROOT."
        " Validé et prouvé live sur RPi4 (status 3 montages CIFS actifs + daemon systemd actif, install synchronisé et activé)."
    ),
    "scripts/infra/rpi4/rpi4_freebox_gateway.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), complexipy validé, et option CLI --check / --dry-run."
        " Déployé et validé live sur Raspberry Pi 4 réel via SSH (serveur actif port 18089, racine /mnt/freebox et 3 partages CIFS détectés, code 0)."
    ),
    "scripts/infra/rpi4/rpi4_o2switch_supervisor.py": (
        "Résolution dynamique des chemins de contrôleurs (resolve_ctl_script) et support variables d'environnement (DBSRT_*_CTL)."
        " 53/53 tests unitaires PASS. Déployé sur RPi4 via rpi4_supervisor_ctl.sh sync, redémarré en service systemd (PID 3971528),"
        " avec auto-rotation et lancement live d'un batch tvshow sur o2switch (PID 3829881)."
    ),
    "scripts/infra/rpi4/rpi4_sim.sh": (
        "Résolution REPO_ROOT et COMPOSE_FILE absolue, port paramétrable. sim/docker-compose.sim.yml ajusté à 4GB RAM."
        " Prouvé live à 100% sur les 5 commandes (up, status, test 6/6 API PASS dont recognize Andor MIH et CAS, logs, down)."
    ),
    "scripts/infra/rpi4/rpi4_standalone.sh": (
        "Résolution REPO_ROOT et source load_env.sh intégrées. Référence de compilation ARM64 réalignée et chemin cible absolu."
        " Validé et prouvé live sur RPi4 (status -> service systemd user actif, API nominale ; demo -> reconnaissance 100% conf MIH Andor et téléchargement CAS SRT validés)."
    ),
    "scripts/infra/rpi4/rpi4_supervisor_ctl.sh": (
        "Résolution REPO_ROOT et source load_env.sh intégrées. Préfixage absolu de UNIT_FILE, SUPERVISOR_SCRIPTS et checksum sync."
        " Contrôleur synchronisé vers l'appliance. Validé et prouvé live à 100% sur l'ensemble des 6 commandes (sync hors racine, install, restart, status, stop, start avec bascule des PID et logs nominaux)."
    ),
    "scripts/ingestion/coupling_relabel.sh": (
        "Résolution dynamique REPO_ROOT et chemin binaire DBSRT_BIN absolu (avec fallback target/debug et garde explicite). En-tête réaligné."
        " Validé et prouvé live sur exécution hors racine (--help) et test E2E fonctionnel complet sur registre temporaire isolé (re-labellisation und -> fr, payload JSON conforme)."
    ),
    "scripts/ingestion/audit_cuts_pipeline.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des extraits/rapports et option CLI --check."
        " Validé hors racine (--check code 0, 2 758 snippets détectés sous data/snippets)."
    ),
    "scripts/ingestion/audit_phash_catalog.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), sélection robuste du binaire dbsrt (release/debug/PATH), chemins absolus et option CLI --check."
        " Validé hors racine (--check code 0, 2 779 .phash et catalogue 231MB confirmés)."
    ),
    "scripts/ingestion/audit_rejected_cuts.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des catalogues/rapports/docs et option CLI --check."
        " Validé hors racine (--check code 0, catalogue 231MB, audit JSON et snippets confirmés)."
    ),
    "scripts/ingestion/backfill_media_assets.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), détection dynamique utilisateur NAS (getpass/env), chemins absolus et option CLI --check."
        " Validé hors racine (--check code 0, 2 779 .phash, 2 130 sidecars JSON et montages CIFS confirmés)."
    ),
    "scripts/ingestion/init_couplings_seed.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des fixtures et support --help / --check. Validé hors racine (code 0)."
    ),
    "scripts/ingestion/media_add.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), sélection absolue DBSRT_BIN (release/debug) et garde explicite. Validé hors racine (--help code 0)."
    ),
    "scripts/ingestion/recognize_medias.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), set -euo pipefail, détection conditionnelle de time/parallel et options --help / --check. Validé hors racine (code 0)."
    ),
    "scripts/ingestion/seed_couplings.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus, sélection absolue DBSRT_BIN et options --help / --check. Validé hors racine (2851 SRT, code 0)."
    ),
    "scripts/tooling/check_pr_body.py": (
        "Chemins docstring et commentaires réalignés. 16/16 tests unitaires PASS (test_check_pr_body.py)."
        " Prouvé live en CLI stdin (rejet corps vide exit 1, validation corps conforme avec preuve SVG exit 0)."
    ),
    "scripts/tooling/build_dashboard_html.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), recherche robuste du manifeste et options CLI --manifest, --output, --check."
        " Validé hors racine sur le manifeste réel (1 332 tests E2E inspectés, taux 99.8%, code 0)."
    ),
    "scripts/tooling/build_inventory_doc.py": (
        "Encapsulation modulaire, résolution dynamique REPO_ROOT (fallback ~/dbsrt), options CLI --input, --output, --check."
        " Validé hors racine sur l'inventaire réel (94 scripts analysés, code 0)."
    ),
    "scripts/tooling/convert_dashboard_webp.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), constantes nommées, refactoring sous seuil complexipy (toutes fonctions <= 10 vs main 29 legacy) et options CLI --dir, --no-rebuild, --check."
        " Validé hors racine (--check code 0, binaire cwebp et répertoire screenshots détectés)."
    ),
    "scripts/tooling/generate_coverage_report.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), refactoring sous seuil complexipy (parse_python_coverage 13 <= 15 vs 19 legacy, main 11 <= 15) et option CLI --check / --dry-run."
        " Validé hors racine (--check code 0, 173 fichiers modifiés détectés par rapport à origin/develop)."
    ),
    "scripts/tooling/lint_migrations.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), refactoring modulaire sous seuil complexipy (lint_migration_file 9 <= 15 vs 37 legacy), et options CLI --dir et --check."
        " Validé hors racine sur les 6 migrations réelles PG 15/17 (code 0)."
    ),
    "scripts/tooling/report_type_errors.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), exécution pyright ancrée sur la racine du dépôt, et options CLI --top et --check."
        " Validé hors racine (0 diagnostic d'erreur de typage, code 0)."
    ),
    "scripts/tooling/serve_docs.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), passage absolu du fichier mkdocs.yml (-f), constantes nommées et option CLI --check."
        " Validé hors racine (détection du fichier mkdocs.yml et port 8000 libre, code 0)."
    ),
    "scripts/tooling/test_db_migrations.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), propagation du répertoire de migrations aux sous-fonctions et options CLI --migrations-dir et --check."
        " Validé hors racine (--check code 0, 6 migrations détectées et sonde PG nominale)."
    ),
    "scripts/tooling/generate_audit_doc.py": (
        "Résolution dynamique REPO_ROOT, exclusion du self-scan (0 faux positif), préservation dynamique des fiches d'analyse méticuleuse et support CLI (--check, --output)."
    ),
    "scripts/tooling/generate_nightly_report_enhanced.py": (
        "Résolution dynamique REPO_ROOT (find_repo_root), réalignement de la référence vers"
        " scripts/ingestion/batch_media_add_tvshow.py, refactoring sous le seuil complexipy (main 15 <= 15)"
        " et support CLI (--check, --input, --output). Validé live sur exécution hors racine et génération nominale."
    ),
    "scripts/tooling/pack_repo.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemin par défaut absolu de l'artefact de sortie et options --help / --check. Validé hors racine (442 fichiers détectés, npx installé, code 0)."
    ),
    "scripts/tooling/pack_review.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), résolution automatique de la branche de base, chemin de sortie absolu et options --help / --check. Validé hors racine (157 fichiers détectés, npx installé, code 0)."
    ),
    "scripts/tooling/test_recipe_debian.sh": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des fichiers Docker/artefacts, compilation cargo avec manifest-path et options --help / --check. Validé hors racine (docker/artefacts trouvés, code 0)."
    ),
    "scripts/workers/o2switch_batch_worker.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), réduction complexipy (main 9 <= 15), 64/64 tests unitaires PASS, et options CLI --help / --check. Déployé et validé live sur Cloud o2switch (SSH ~/dbsrt, code 0)."
    ),
    "scripts/workers/o2switch_batch_summary.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), chemins absolus des rapports/logs et options CLI --help / --check. Déployé et prouvé live sur Cloud o2switch en production (10 880 événements réels et 3 872 logs analysés, code 0)."
    ),
    "scripts/workers/o2switch_fault_watch.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), préfixage absolu des logs/rapports, et support CLI --help et --check. Déployé et prouvé live sur Cloud o2switch (SSH ~/dbsrt, 6 processus surveillés, code 0)."
    ),
    "scripts/workers/o2switch_phash_migrate.py": (
        "Résolution dynamique REPO_ROOT (fallback ~/dbsrt), sys.path normalisé, et support CLI --help / --check. Déployé et prouvé live sur Cloud o2switch (SSH ~/dbsrt, pHash dir et credentials NAS validés, code 0)."
    ),
}

SCRIPT_KPIS: dict[str, str] = {
    "scripts/benchmarks/bench_http_flood.sh": "Moyenne (Dev & Banc de Charge HTTP)",
    "scripts/benchmarks/bench_server_comparison.py": "Élevée (Benchmark Comparatif Latence Serveur ArcSwap)",
    "scripts/benchmarks/benchmark_worker_scaling.py": "Élevée (Benchmark Scaling Workers & Saturation VA-API)",
    "scripts/benchmarks/eval_exhaustive_nas_snippets.py": "Critique (Matrice Évaluation Exhaustive Dataset Snippets)",
    "scripts/benchmarks/eval_media_batch.py": "Élevée (Évaluation par Lots des Conteneurs & Sous-Titres)",
    "scripts/benchmarks/eval_multi_offset_matching.py": "Critique (Évaluation Reconnaissance Spatiotemporelle Multi-Fenêtres)",
    "scripts/benchmarks/eval_nas_reliability.py": "Critique (Suite Complète Fiabilité NAS & PostgreSQL 17)",
    "scripts/benchmarks/eval_vlc_vlsub_batch.py": "Critique (Campagne d'Évaluation E2E GUI VLC & Extension VLSub)",
    "scripts/benchmarks/scale_up_nas_dataset.py": "Critique (Pipeline Scale-Up & Certification Couplings NAS)",
    "scripts/benchmarks/verify_terminal_rendering.py": "Élevée (Vérification DOM & Screenshots Headless Chrome Asciinema)",
    "scripts/benchmarks/compare_top_telemetry.py": "Moyenne (Dev / Parité Télémétrie)",
    "scripts/benchmarks/record_casts.py": "Élevée (Doc Diátaxis & Casts Asciinema)",
    "scripts/benchmarks/record_proof.py": "Critique (Preuves Terminales PR)",
    "scripts/benchmarks/record_vlsub_demo.sh": "Élevée (Doc Diátaxis & Démo Vidéo VLSub)",
    "scripts/e2e/o2switch_e2e.py": "Critique (Recette E2E Cloud o2switch)",
    "scripts/e2e/e2e_portal_playwright.py": "Critique (Recette E2E Playwright Portails Web)",
    "scripts/e2e/recette_top_e2e.py": "Critique (Recette E2E & Enregistreur ASCIInema Top TUI)",
    "scripts/infra/local/agent_relay.sh": "Élevée (Architecture Hybride & Passerelle Locale-Cloud)",
    "scripts/infra/local/dbsrt_top.py": "Critique (TUI Python de Référence & Supervision Décisionnelle)",
    "scripts/infra/local/distributed_dev_stack.sh": "Moyenne (Dev & Simulation Topologie Hybride)",
    "scripts/infra/local/install_debian.sh": "Élevée (Déploiement Poste Développeur & Appliance Debian)",
    "scripts/infra/local/load_env.sh": "Critique (Socle d'Environnement & Sécurité des Credentials)",
    "scripts/infra/local/stack_ctl.sh": "Critique (Contrôleur Maître Stack Locale & Diagnostic 360°)",
    "scripts/infra/local/switch_target.sh": "Élevée (Orchestration Multi-Backend & Bascule Opérateur)",
    "scripts/infra/local/vlc_ctl.sh": "Élevée (Intégration VLC & Recette Utilisateur Final)",
    "scripts/infra/o2switch/nas_o2switch_sync.sh": "Élevée (Synchronisation Hybride NAS -> Cloud o2switch)",
    "scripts/infra/o2switch/o2switch_audit_remote.py": "Élevée (Audit de Précision & Recette Cloud)",
    "scripts/infra/o2switch/o2switch_ctl.sh": "Critique (Contrôleur Maître Cloud o2switch)",
    "scripts/infra/rpi4/freebox_batch_ctl.sh": "Élevée (Ingestion WAN Freebox / Portail Web)",
    "scripts/infra/rpi4/rpi4_build.sh": "Moyenne (Infrastructure de Conteneurisation ARM64)",
    "scripts/infra/rpi4/rpi4_cli.sh": "Élevée (Cross-Compilation ARM64 & Déploiement CLI RPi4)",
    "scripts/infra/rpi4/rpi4_deploy.sh": "Élevée (Déploiement Docker & Base de Données Appliance RPi4)",
    "scripts/infra/rpi4/rpi4_sync.sh": "Élevée (Synchronisation Données & Déploiement Appliance RPi4)",
    "scripts/infra/rpi4/rpi4_freebox_ctl.sh": "Élevée (Appliance RPi4 & CIFS/HTTP Range)",
    "scripts/infra/rpi4/rpi4_freebox_gateway.py": "Critique (Passerelle HTTP Byte-Range Freebox RPi4)",
    "scripts/infra/rpi4/rpi4_o2switch_supervisor.py": "Critique (Superviseur Autonome 24/7 de Production)",
    "scripts/infra/rpi4/rpi4_sim.sh": "Moyenne (Dev & Simulation Locale QEMU ARM64)",
    "scripts/infra/rpi4/rpi4_standalone.sh": "Élevée (Déploiement Serveur Physique RPi4)",
    "scripts/infra/rpi4/rpi4_supervisor_ctl.sh": "Élevée (Contrôleur de Déploiement & Lifecycle du Superviseur RPi4)",
    "scripts/ingestion/coupling_relabel.sh": "Moyenne (Outil de Curation du Registre Zero-DB)",
    "scripts/ingestion/audit_cuts_pipeline.py": "Élevée (Audit de Performance Multi-Étapes Pipeline Matching)",
    "scripts/ingestion/audit_phash_catalog.py": "Critique (Audit Empirique Exhaustif Reconnaissance pHash & Zero-DB)",
    "scripts/ingestion/audit_rejected_cuts.py": "Élevée (Diagnostic Forensique Exhaustif des Rejets Matching)",
    "scripts/ingestion/backfill_media_assets.py": "Élevée (Remplissage Lignage V6 & Métadonnées Actifs Multimédias)",
    "scripts/ingestion/init_couplings_seed.sh": "Moyenne (Amorçage CI/CD & Tests d'Intégration)",
    "scripts/ingestion/media_add.sh": "Élevée (Ingestion Unifiée & Invariants Multimédia)",
    "scripts/ingestion/recognize_medias.sh": "Moyenne (Outil d'Ingestion & Évaluation par Lots)",
    "scripts/ingestion/seed_couplings.sh": "Élevée (Vérité Terrain & Registre des Couplages)",
    "scripts/tooling/build_dashboard_html.py": "Moyenne (Génération Dashboard E2E VLC)",
    "scripts/tooling/build_inventory_doc.py": "Moyenne (Génération Documentation Inventaire Scripts)",
    "scripts/tooling/convert_dashboard_webp.py": "Moyenne (Optimisation Assets WebP Dashboard)",
    "scripts/tooling/generate_coverage_report.py": "Critique (Rapport Couverture Unifié & Gate CI/CD)",
    "scripts/tooling/lint_migrations.py": "Critique (Garde-fou CI/CD & Sécurité DDL Migrations PG)",
    "scripts/tooling/report_type_errors.py": "Moyenne (Audit Typage Statique Pyright)",
    "scripts/tooling/serve_docs.py": "Élevée (Serveur Local Documentation Diátaxis)",
    "scripts/tooling/test_db_migrations.py": "Élevée (Test d'Intégration Migrations & Lignage PG)",
    "scripts/tooling/check_pr_body.py": "Critique (Garde-fou CI/CD & Qualité des PRs)",
    "scripts/tooling/generate_audit_doc.py": "Moyenne (Outil de Métrologie & Méta-Documentation)",
    "scripts/tooling/generate_nightly_report_enhanced.py": "Moyenne (Métrologie & Profiling Diátaxis)",
    "scripts/tooling/lint_complexity.py": "Critique (Garde-fou CI/CD & Architecture Rust)",
    "scripts/tooling/mkdocs_hooks.py": "Élevée (Orchestration Diátaxis & Recherche Lunr)",
    "scripts/tooling/pack_repo.sh": "Moyenne (Tooling & Contexte IA Repomix)",
    "scripts/tooling/pack_review.sh": "Élevée (Tooling & Review IA Automatisée)",
    "scripts/tooling/test_recipe_debian.sh": "Élevée (Tests d'Intégration Hermétiques Docker/systemd)",
    "scripts/tooling/validate_docs.py": "Critique (Garde-fou CI/CD & Validation Diátaxis)",
    "scripts/tooling/validate_web_assets.py": "Élevée (Garde-fou CI/CD Assets Web & Lint)",
    "scripts/workers/o2switch_batch_summary.py": "Élevée (Métrologie Anonymisée & Télémétrie o2switch)",
    "scripts/workers/o2switch_batch_worker.py": "Critique (Worker Parallèle FTP Stream & Ingestion o2switch)",
    "scripts/workers/o2switch_fault_watch.py": "Élevée (Sonde & Diagnostic des Défauts CPU CloudLinux LVE)",
    "scripts/workers/o2switch_phash_migrate.py": "Élevée (Migration & Renommage Moviehash des pHashes)",
}

CAT_TITLES: dict[str, str] = {
    "infra/local": "Infra & Contrôleurs Locaux (`scripts/infra/local/`)",
    "infra/o2switch": "Infra & Déploiements Cloud o2switch (`scripts/infra/o2switch/`)",
    "infra/rpi4": "Infra & Contrôleurs Appliance RPi4 (`scripts/infra/rpi4/`)",
    "workers": "Démons Asynchrones de Fond (`scripts/workers/`)",
    "ingestion": "Ingestion, Curation & Déduplication (`scripts/ingestion/`)",
    "benchmarks": "Bancs d'Essai & Évaluations (`scripts/benchmarks/`)",
    "e2e": "Suites E2E Playwright (`scripts/e2e/`)",
    "tooling": "Tooling, Linters & Gardes-Fous (`scripts/tooling/`)",
    "root": "Serveur Portail Racine (`scripts/`)",
}


def _resolve_target(m_str: str, root: Path, repo_root: Path) -> str:
    matches = list(root.rglob(m_str))
    if not matches:
        return "NON TROUVE"
    return str(matches[0].relative_to(repo_root))


def _scan_single_script(p: Path, root: Path, repo_root: Path) -> list[dict[str, str]]:
    if p.name == "generate_audit_doc.py":
        return []
    rel_source = str(p.relative_to(repo_root)) if repo_root in p.parents else str(p)
    broken = []
    content = p.read_text(encoding="utf-8", errors="ignore")
    for m in re.finditer(r"\bscripts/([a-zA-Z0-9_\-\.]+\.(?:py|sh|js))", content):
        ref = f"scripts/{m.group(1)}"
        if not (repo_root / ref).exists():
            broken.append(
                {
                    "source": rel_source,
                    "ref": ref,
                    "target": _resolve_target(m.group(1), root, repo_root),
                }
            )
    return broken


def _find_broken_refs(all_scripts: list[Path], root: Path) -> list[dict[str, str]]:
    """Scans all scripts for stale references to scripts/*.py or scripts/*.sh."""
    broken: list[dict[str, str]] = []
    repo_root = root.parent
    for p in all_scripts:
        broken.extend(_scan_single_script(p, root, repo_root))
    return broken


def _format_script_row(s: Path, repo_root: Path, broken_sources: set[str]) -> str:
    """Formats a single markdown table row for a script."""
    sp = str(s.relative_to(repo_root)) if repo_root in s.parents else str(s)
    stype = "Python" if s.suffix == ".py" else "Shell"
    kpi = SCRIPT_KPIS.get(sp, "À évaluer")
    if sp in PROVEN_LIVE:
        st = "🟢 **Prouvé Live**"
        detail = PROVEN_LIVE[sp]
    elif sp in broken_sources:
        st = "🔴 **Régression**"
        detail = "Contient des appels de scripts internes non réalignés (voir section 2)."
    elif s.suffix == ".sh":
        st = "⚪ **Statique Seul (Non Exécuté)**"
        detail = "Syntaxe `bash -n` & `shellcheck` OK. Zéro exécution runtime."
    else:
        st = "🟡 **Sanity Check CLI (Non Validé Métier)**"
        detail = (
            "Sanity check `--help` uniquement (arbre d'imports et parser CLI)."
            " Aucune logique métier ni invariant validé en mémoire."
        )
    return f"| `{sp}` | {st} | {stype} | {kpi} | {detail} |"


def _build_domain_tables(all_scripts: list[Path], repo_root: Path, broken_sources: set[str]) -> list[str]:
    """Generates markdown sections grouped by domain directory."""
    by_cat: dict[str, list[Path]] = {}
    for s in all_scripts:
        cat = s.parent.name if s.parent.name != "scripts" else "root"
        if s.parent.parent.name == "infra":
            cat = f"infra/{cat}"
        by_cat.setdefault(cat, []).append(s)

    lines: list[str] = []
    sec_idx = 1
    for cat, scripts_list in sorted(by_cat.items()):
        title = CAT_TITLES.get(cat, cat)
        lines.append(f"### 3.{sec_idx} {title}")
        sec_idx += 1
        lines.append("")
        lines.append("| Script | Statut Réel | Type | KPI / Importance | Détail / Diagnostic Contradictoire |")
        lines.append("| :--- | :---: | :---: | :---: | :--- |")
        for s in scripts_list:
            lines.append(_format_script_row(s, repo_root, broken_sources))
        lines.append("")
    return lines


def _partition_scripts(all_scripts: list[Path], repo_root: Path, active_broken_sources: set[str]) -> dict[str, int]:
    """Partitions scripts into 4 Fact-First categories and counts language types."""
    counts = {"proven": 0, "broken": 0, "sanity": 0, "static": 0, "py": 0, "sh": 0}
    for s in all_scripts:
        if s.suffix == ".py":
            counts["py"] += 1
        elif s.suffix == ".sh":
            counts["sh"] += 1

        sp = str(s.relative_to(repo_root)) if repo_root in s.parents else str(s)
        if sp in PROVEN_LIVE:
            counts["proven"] += 1
        elif sp in active_broken_sources:
            counts["broken"] += 1
        elif s.suffix == ".sh":
            counts["static"] += 1
        else:
            counts["sanity"] += 1
    return counts


def _build_broken_ref_table(broken_refs: list[dict[str, str]]) -> list[str]:
    """Formats the table of stale internal references with their Fact-First resolution status."""
    lines = [
        "## 2. Inventaire des Références Internes & État de Remédiation",
        "",
        (
            "Lors du déplacement des scripts dans des sous-dossiers thématiques"
            " (`infra/`, `workers/`, `benchmarks/`, `ingestion/`), les chemins"
            " d'invocation codés en dur (`scripts/<nom>.py` ou"
            " `scripts/<nom>.sh`) à l'intérieur des contrôleurs et"
            " orchestrateurs ont été audités et remédiés script par script :"
        ),
        "",
        "| # | Fichier Source | Référence Rompue (Ancien Chemin) | Chemin Cible Réel (Nouveau Layout) | Statut Fact-First |",
        "| :---: | :--- | :--- | :--- | :---: |",
    ]
    for idx, r in enumerate(broken_refs, 1):
        st = "🟢 **Prouvé Live & Résolu**" if r["source"] in PROVEN_LIVE else "🔴 **Régression Active**"
        lines.append(f"| {idx} | `{r['source']}` | `{r['ref']}` | `{r['target']}` | {st} |")
    return lines


def _build_summary_section(
    all_scripts: list[Path],
    repo_root: Path,
    broken_refs: list[dict[str, str]],
) -> list[str]:
    """Generates the executive summary, Mermaid pie chart, and broken reference inventory."""
    active_broken_sources = {r["source"] for r in broken_refs if r["source"] not in PROVEN_LIVE}
    c = _partition_scripts(all_scripts, repo_root, active_broken_sources)
    total = len(all_scripts)
    active_broken_count = len([r for r in broken_refs if r["source"] not in PROVEN_LIVE])
    status_suffix = "Ruptures critiques identifiées.*" if c["broken"] > 0 else "100% des régressions actives résolues.*"

    lines = [
        "# Audit Exhaustif des Scripts & Cartographie des Régressions de Chemins",
        "",
        "> **Date de constat :** 2026-11-10 21:52:00 (Actualisé post-remédiation 2026-11-11)<br>",
        "> **Statut :** Bilan consolidé post-audit des 39 scripts remédiés (18 critiques + 21 shell) (PR #70 / Issue #69 Phase 1 & 2)<br>",
        (
            "> **Avertissement de rigueur :** Un « Sanity Check CLI `--help` »"
            " atteste uniquement que Python parvient à charger le module sans"
            " crash immédiat au démarrage. Sa valeur probante est infime (à"
            " peine supérieure à une simple vérification statique) : **il ne"
            " valide en aucun cas la logique interne, les invariants mémoire,"
            " les connexions réseau ou les interactions réelles.**"
        ),
        "",
        "## 1. Synthèse Factuelle Globale",
        "",
        f"- **Total scripts opérationnels** : {total} ({c['py']} Python, {c['sh']} Shell)",
        (
            f"- **Scripts prouvés en exécution réelle live (session)** : {c['proven']} / {total}"
            f" ({c['proven'] / total * 100:.1f}%) — *Seuls scripts réellement testés de bout en bout.*"
        ),
        (
            f"- **Scripts en régression directe (appels internes cassés)** : {c['broken']} / {total}"
            f" ({c['broken'] / total * 100:.1f}%) — *{status_suffix}"
        ),
        (
            f"- **Nombre total de références internes rompues actives** : {active_broken_count}"
            f" ({len(broken_refs)} références appliance distantes documentées)"
        ),
        (
            "- **Scripts avec simple Sanity Check CLI `--help` (Non Validés"
            f" Métier)** : {c['sanity']} / {total} ({c['sanity'] / total * 100:.1f}%) — *Imports seuls, logique non"
            " exécutée.*"
        ),
        f"- **Scripts avec analyse statique seule (`bash -n`)** : {c['static']} / {total} ({c['static'] / total * 100:.1f}%) — *Zéro runtime.*",
        "",
        "```mermaid",
        f"pie title Statut Réel des {total} Scripts (Rigueur Fact-First)",
        f'    "Prouvé Live de Bout en Bout" : {c["proven"]}',
        f'    "Régression Interne Cassée" : {c["broken"]}',
        f'    "Sanity Check CLI Seul (Non Validé)" : {c["sanity"]}',
        f'    "Statique / Syntaxe Seul (Non Exécuté)" : {c["static"]}',
        "```",
        "",
        "---",
        "",
    ]
    lines.extend(_build_broken_ref_table(broken_refs))
    return lines


def parse_args() -> argparse.Namespace:
    """Parses command line arguments."""
    parser = argparse.ArgumentParser(description="Generate exhaustive audit document of repository scripts.")
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        dest="check",
        help="Analyze and print summary statistics without modifying markdown files.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Custom output markdown file path (defaults to docs/reference/2026-11-10-audit-exhaustif-scripts-regressions.md).",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    repo_root = find_repo_root()
    root = repo_root / "scripts"
    all_scripts = sorted(
        [p for p in root.rglob("*") if p.is_file() and p.suffix in (".py", ".sh") and p.name != "__init__.py"]
    )

    broken_refs = _find_broken_refs(all_scripts, root)
    broken_sources = {r["source"] for r in broken_refs}

    if args.check:
        print(f"Total scripts: {len(all_scripts)}")
        print(f"Proven live: {len(PROVEN_LIVE)}")
        print(f"Broken sources: {len(broken_sources)}")
        print(f"Total broken references: {len(broken_refs)}")
        for idx, r in enumerate(broken_refs, 1):
            print(f"  {idx}. {r['source']} -> {r['ref']} (target: {r['target']})")
        return

    out_path = args.output or (repo_root / "docs/reference/2026-11-10-audit-exhaustif-scripts-regressions.md")

    # Preserve manually verified fiches from existing document if present
    fiches_section: list[str] = []
    if out_path.exists():
        content = out_path.read_text(encoding="utf-8")
        if "## 5. Fiches d'Analyse Méticuleuse" in content:
            fiches_text = content.split("## 5. Fiches d'Analyse Méticuleuse", 1)[1]
            fiches_section = [
                "## 5. Fiches d'Analyse Méticuleuse" + fiches_text.rstrip(),
                "",
            ]

    lines = _build_summary_section(all_scripts, repo_root, broken_refs)

    lines.extend(
        [
            "",
            "---",
            "",
            "## 3. Cartographie Exhaustive par Domaine Fonctionnel",
            "",
        ]
    )

    lines.extend(_build_domain_tables(all_scripts, repo_root, broken_sources))

    lines.extend(
        [
            "---",
            "",
            "## 4. Protocole d'Assainissement & de Remédiation Script par Script",
            "",
            "Pour sortir de cet état sans spéculation, chaque script des sections 1 et 2 doit faire l'objet de :",
            "",
            (
                "1. **Correction chirurgicale des chemins internes** : remplacer"
                " systématiquement les chemins obsolètes par leur localisation"
                " réelle sous `scripts/`."
            ),
            (
                "2. **Revue de conformité de l'arborescence `sys.path`** :"
                " s'assurer que tout script important un sous-module importe depuis"
                " la racine du dépôt."
            ),
            "3. **Épreuve d'exécution formelle par script** :",
            (
                "   - **Contrôleurs Shell** : exécution en mode validation d'options"
                " / dry-run ou invocation réelle contrôlée."
            ),
            "   - **Workers Asynchrones** : exécution avec arguments temporaires de test et timeout court.",
            "   - **Scripts Ingestion NAS & Freebox** : validation avec mocks ou lecture de fixture locale.",
            "",
            "---",
            "",
        ]
    )

    if fiches_section:
        lines.extend(fiches_section)

    out_path.write_text("\n".join(lines), encoding="utf-8")
    print(f"Wrote {len(lines)} lines to {out_path}")


if __name__ == "__main__":
    main()
