---
title: "Référence : Catalogue des Commandes Taskfile"
tags:

  - reference
  - taskfile
  - orchestration
  - cli
---

# 📖 Référence : Catalogue des Commandes Taskfile (`Taskfile.yml`)

Cette page dresse le dictionnaire exhaustif de toutes les cibles d'orchestration déclarées dans `Taskfile.yml` et les fichiers modulaires sous `taskfiles/`.

---

## 1. Raccourcis Globaux & Cycle de Vie

| Commande | Raccourcis | Description |
| :--- | :--- | :--- |
| `task dev` | `task up` | Démarre la pile complète de développement (Docker DB, compilation Rust, certification, daemon, plugin VLC, serveur MkDocs). |
| `task down` | `task stop` | Arrête proprement l'ensemble des démons d'arrière-plan et conteneurs Docker. |
| `task status` | - | Sonde d'intégrité unifiée affichant l'état des services HTTP et le décompte des entités SQL. |
| `task studio` | - | Démarre la base de données, applique les couplages et lance le Studio sur `http://127.0.0.1:8765/studio`. |
| `task build` | `task rs:build` | Compile l'ensemble des crates du workspace Cargo en mode optimisé (`--release`). |
| `task clean` | - | Purge tous les fichiers temporaires, caches de couverture et binaires sous `target/`. |
| `task ci:local` | - | Pipeline de reproduction CI ultra-rapide (<10s) : linters, tests Rust et validation docs. |

---

## 2. Ingestion & Couplages (`media:*` & `coupling:*`)

| Commande | Syntaxe & Arguments | Description |
| :--- | :--- | :--- |
| `task media:add` | `task media:add -- <video> [srt] [--lang <lg>] [--fps <val>]` | Façade unifiée d'ingestion média avec auto-détection et injection à chaud. |
| `task coupling:certify` | `task coupling:certify` | Découvre et certifie en lot tous les fichiers `.phash` et `.srt` présents. |
| `task coupling:relabel` | `task coupling:relabel -- --media <hash> [--sub <hash>] --lang <code>` | Modifie la langue ou la confiance d'un couplage existant. |

---

## 3. Base de Données PostgreSQL (`db:*`)

| Commande | Description |
| :--- | :--- |
| `task db:up` | Démarre le conteneur PostgreSQL 17 (`bktreedb`) et PgWeb en arrière-plan. |
| `task db:down` | Arrête le conteneur de base de données. |
| `task db:migrate` | Applique les migrations de schéma SQL pendantes via Refinery. |
| `task db:status` | Affiche l'historique et la version courante du schéma de base. |
| `task db:psql` | Ouvre une session interactive `psql` dans le conteneur PostgreSQL actif. |
| `task db:import:all-phash` | Importe en lot les fichiers `.phash` dans PostgreSQL via flux binaire `COPY`. |
| `task db:lint:safety` | Vérifie la compatibilité et la non-régression des scripts de migration DDL. |

---

## 4. Microservice & Catalogue en Mémoire (`server:*` & `catalog:*`)

| Commande | Description |
| :--- | :--- |
| `task server:daemon` | Démarre `dbsrt-server` en arrière-plan avec journalisation sous `/tmp/dbsrt-server.log`. |
| `task server:stop` | Arrête l'instance locale en cours d'exécution de `dbsrt-server`. |
| `task server:status` | Interroge l'endpoint `/v1/status` pour valider la réponse HTTP et la mémoire. |
| `task server:reload` | Déclenche le rechargement à chaud atomique (`/v1/catalog/reload`) sans coupure de service. |
| `task catalog:status` | Inspecte l'état des index en mémoire vive (Base `.dbphash` + Delta RAM). |
| `task catalog:compact` | Fusionne l'index Delta en mémoire dans le fichier binaire `.dbphash` consolidé. |

---

## 5. Lecteur Multimédia & Client Linux (`vlc:*` & `client:*`)

| Commande | Description |
| :--- | :--- |
| `task vlc:install-plugin` | Installe les scripts d'extension Lua dans le profil local VLC. |
| `task vlc:open` | Ouvre VLC avec les paramètres de journalisation et de debug activés. |
| `task client:linux:install` | Installe l'agent client `dbsrt` et configure le service systemd utilisateur. |
| `task client:linux:status` | Affiche le statut du service systemd `dbsrt-agent`. |
| `task client:linux:logs` | Affiche les logs en direct du service client Linux via `journalctl`. |

---

## 6. Documentation & Qualité (`doc:*`, `lint:*`, `test:*`)

| Commande | Description |
| :--- | :--- |
| `task doc` | Démarre le serveur MkDocs Material local avec port dynamique (`8000`–`8020`). |
| `task doc:check` | Validateur automatisé strict (build MkDocs, BeautifulSoup AST, rendu MathJax, zéro chemin local). |
| `task doc:build` | Génère le site statique sous `site/` pour publication. |
| `task doc:deploy:gh-pages` | Déploie la documentation compilée sur GitHub Pages. |
| `task lint` | Exécute la suite de linters unifiés (Ruff, clippy, shellcheck, shfmt, yamllint, hadolint). |
| `task typecheck` | Analyse statique de types Python via Pyright en mode strict. |
| `task test` | Exécute l'intégralité des suites de tests unitaires et intégration (Rust + Python). |
| `task test:rs` | Exécute la suite de tests Cargo du workspace Rust. |
| `task test:infra` | Exécute le harnais de tests d'infrastructure et chaos testing. |

---

## 7. Appliance Domestique Raspberry Pi 4 (`rpi4:*`)

| Commande | Description |
| :--- | :--- |
| `task rpi4:build` | Cross-compile le binaire aarch64 et génère l'image Docker ARM64. |
| `task rpi4:push` | Transfère l'image Docker vers le Raspberry Pi 4 via SSH. |
| `task rpi4:up` | Démarre la pile de conteneurs sur le Raspberry Pi distant. |
| `task rpi4:status` | Interroge la sonde de santé HTTP du Raspberry Pi 4. |
| `task rpi4:demo` | Teste la reconnaissance d'un extrait de 200 octets sur l'appliance distante. |
