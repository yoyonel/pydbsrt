---
title: "Guide Pratique : Installation From Scratch & Recette Autonome Sans IA"
tags:
  - howto
  - installation
  - from-scratch
  - recette
  - autonomie
  - test-automation
  - diataxis
---

# 🛠️ Guide Pratique : Installation From Scratch & Recette Autonome Sans IA

Ce guide fournit la procédure pas-à-pas pour installer, configurer, tester et valider l'intégralité du projet `pydbsrt` sur un nouvel ordinateur (laptop ou desktop) sous un système d'exploitation vierge (Linux, macOS), de façon **100 % programmatique et déterministe**, sans aucune assistance ni interprétation par une IA.

---

## Problème

Vous récupérez le projet sur une machine neuve dans 15 jours, 3 mois ou 1 an. Vous devez :

1. Cloner et configurer l'environnement de développement sans deviner les variables ou les chemins cachés.
2. Démarrer les services conteneurisés en local (PostgreSQL 17 SP-GiST, PgWeb).
3. Exécuter la suite complète de tests unitaires, d'intégration et de migrations de données sans risquer de corrompre la base active.
4. Valider l'état du système de façon binaire (`exit 0` = succès, `exit 1` = échec) via les codes de retour système.

---

## 1. Pré-requis Système Statiques

Sur votre machine hôte, seuls 5 outils standards doivent être installés :

| Outil | Version Minimale | Rôle | Commande de Vérification |
| :--- | :---: | :--- | :--- |
| **Git** | $\ge 2.34$ | Gestionnaire de versions | `git --version` |
| **Rust / Cargo** | $\ge 1.82$ (stable) | Compilateur workspace natif | `cargo --version` |
| **Python** | $\ge 3.10$ | Runtime des scripts de validation | `python3 --version` |
| **uv** | $\ge 0.4.0$ | Gestionnaire de packages Python | `uv --version` |
| **Docker & Compose** | Docker $\ge 24$, Compose $\ge 2$ | Infrastructure locale (PostgreSQL 17) | `docker compose version` |
| **go-task (task)** | $\ge 3.30$ | Orchestrateur unifié du projet | `task --version` |

---

## 2. Procédure d'Installation en 4 Commandes

### Étape 1 : Cloner le Référentiel et Synchroniser les Dépendances

```bash
git clone https://github.com/yoyonel/pydbsrt.git
cd pydbsrt

# Synchronise le virtualenv Python localement via uv
task sync
```

### Étape 2 : Démarrer la Stack Locale PostgreSQL 17

La commande démarre en tâche de fond le conteneur PostgreSQL 17 configuré avec l'extension SP-GiST Hamming et le visualiseur Web PgWeb :

```bash
task db:up
```

*Vérification automatique* : l'outil affiche `[+] up 5/5` attestant que `bktreedb`, `pgweb`, `prometheus`, `grafana` et `postgres_exporter` sont actifs.

---

## 3. Recette de Validation Déterministe (Zéro IA)

Pour garantir l'intégrité opérationnelle avant toute manipulation, exécutez la pyramide de tests dans l'ordre suivant :

```
             ┌─────────────────────────────┐
             │       task ci:local         │  (Lint + Tests + Validation Docs)
             └──────────────┬──────────────┘
                            │
             ┌──────────────▼──────────────┐
             │    task db:test:migration   │  (Banc DDL Sandbox + Non-Régression)
             └──────────────┬──────────────┘
                            │
             ┌──────────────▼──────────────┐
             │         task test:rs        │  (468 Assertions Rust Workspace)
             └─────────────────────────────┘
```

### Étape 3 : Banc de Test d'Intégrité DB & Migrations (Sandbox Éphémère)

Ce banc exécute 3 suites d'assertions strictes sur des bases de données de test éphémères créées et détruites à la volée (votre base active reste **100 % intouchée**) :

```bash
task db:test:migration
```

**Comportement déterministe vérifié :**

1. **Cycle de vie à blanc (V1 $\to$ V6)** : applique séquentiellement toutes les migrations sur `bktreedb_migration_blank_sandbox` et vérifie la présence physique des tables `medias`, `frames`, `subtitles`, `sub_frames`, `media_sources`.
2. **Crash-test de non-régression sur données réelles** : réplique les 2 325 médias de la base active dans une sandbox via flux natif `COPY`, applique la migration `V6` en temps réel, et assert la préservation absolue à 100 % des lignes (`expected_frames = 0`, `asset_kind = 'master'`).
3. **Stress-test des contraintes de lignée** :
   - Rejette les sources au protocole non autorisé (`chk_storage_kind`).
   - Rejette un Master doté d'un parent ou d'un décalage temporel (`chk_asset_lineage`).
   - Rejette un Cut sans parent ou sans durée (`chk_asset_lineage`).
   - Bloque la suppression d'un Master parent référencé par un Cut (`ON DELETE RESTRICT`).
   - Rejette les doublons de conteneurs pour les Masters actifs (`idx_medias_container_identity`).
4. **Auto-nettoyage** : détruit systématiquement les bases sandbox en fin d'exécution (`exit 0`).

### Étape 4 : Tests Unitaires et d'Intégration Rust

Exécute les 468 assertions mathématiques et logiques du workspace :

```bash
task test:rs
```

*Résultat attendu* : `test result: ok. 468 passed; 0 failed`.

### Étape 5 : Lancement de la Suite CI Locale Complète

Vérifie le formatage, les linters multi-langages, le typage statique et la validité stricte de la documentation :

```bash
task ci:local
```

Cette commande enchaîne :

- `task lint` : validation multi-langages (Ruff, Pyright, Rustfmt, Clippy, Shellcheck, Stylua, Yamllint).
- `task test:rs` : exécution des tests Rust.
- `task doc:check` : compilation stricte MkDocs et validation de l'arbre syntaxique HTML AST.

---

## 4. Outils d'Observation & Recette Visuelle

Une fois les tests automatisés validés, vous pouvez inspecter visuellement la base et le système :

### Visualiseur Web PgWeb

Ouvrez votre navigateur sur :

```text
http://127.0.0.1:8086
```

L'interface permet de parcourir directement les tables, exécuter des requêtes SQL et inspecter les données sans installer de client lourd.

### Client Terminal Interactif (Harlequin TUI)

Pour explorer le schéma et exécuter des requêtes depuis votre terminal :

```bash
task db:tui
```

### Moniteur de Processus Temps Réel (dbsrt top)

Pour observer les sources médias configurées, les tâches de calcul pHash en cours et le catalogue :

```bash
task top
```

Raccourcis de navigation :
- `[4/m]` : Navigateur de médias (*MediaBrowser*).
- `[5/s]` : Sources de médias (*MediaSources*).
- `[7/t]` : Audit des trajectoires spatiotemporelles (*Trajectory*).
- `[q]` : Quitter le TUI.

---

## 5. Dépannage & Diagnostic Sans IA

| Symptôme | Cause Probable | Action Corrective |
| :--- | :--- | :--- |
| `task: command not found` | Go-task non installé | Installer via `sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b /usr/local/bin` ou package manager. |
| `psycopg2.OperationalError: could not connect to server` | PostgreSQL non démarré sur le port 5433 | Exécuter `task db:up` et vérifier avec `docker compose -f docker/docker-compose.yml ps`. |
| `psycopg2.errors.ObjectInUse` | Session concurrente verrouillant la DB modèle | Normal lors d'un `TEMPLATE` direct ; résolu nativement dans `task db:test:migration` via flux `COPY`. |
| `Refinery: error running migrations` | Conflit de version dans `refinery_schema_history` | Inspecter l'historique avec `task db:status` ou inspecter les logs avec `task db:logs`. |
