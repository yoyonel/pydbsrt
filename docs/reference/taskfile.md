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
| `task media:batch:tvshow` | `task media:batch:tvshow -- [--until <HH:MM>] [--workers <N>]` | Ingestion batch parallélisée de séries TV en direct depuis montage CIFS. |
| `task media:top` | `task media:top -- [--refresh-ms <ms>]` | Moniteur TUI natif Rust temps réel (full-frame) pour les tâches de décodage, débits CIFS et ETA. |
| `task media:top:py` | `task media:top:py -- [--stats]` | Prototype historique Python Rich du moniteur TUI. |
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
| `task db:tui` | Lance l'explorateur interactif plein écran SQL TUI Harlequin sur PostgreSQL 17 local (`:5433`). |
| `task db:backfill:medias` | Rétro-propage les métadonnées riches depuis les `.phash` et la télémétrie JSONL vers la table `medias`. |
| `task db:audit` | Génère un bilan d'intégrité exhaustif des valeurs `NULL` de la table `medias` et inspecte la vue `v_productions`. |
| `task db:probe` | Sonde les flux vidéo réels sur le NAS en multi-threading via `ffprobe` et met à jour géométrie (`width`, `height`), `video_codec` et `resolution`. |

---

## 4. Microservice & Catalogue en Mémoire (`server:*` & `catalog:*`)

| Commande | Description |
| :--- | :--- |
| `task server:daemon` | Démarre `dbsrt-server` en arrière-plan avec journalisation sous `/tmp/dbsrt-server.log`. |
| `task server:stop` | Arrête l'instance locale en cours d'exécution de `dbsrt-server`. |
| `task server:status` | Interroge l'endpoint `/v1/status` pour valider la réponse HTTP et la mémoire. |
| `task server:reload` | Déclenche le rechargement à chaud atomique (`/v1/catalog/reload`) sans coupure de service. |
| `task catalog:status` | Inspecte l'état des index en mémoire vive (Base `.dbphash` + Delta RAM). |
| `task catalog:verify` | Exécute l'audit programmatique exhaustif d'intégrité du catalogue (dbphash, couplings, PostgreSQL, phash). |
| `task catalog:compact` | Fusionne l'index Delta en mémoire dans le fichier binaire `.dbphash` consolidé. |

---

## 5. Lecteur Multimédia & Client Linux (`vlc:*`, `client:*` & `target:*`)

| Commande | Description |
| :--- | :--- |
| `task vlc:install-plugin` | Installe les scripts d'extension Lua dans le profil local VLC. |
| `task vlc:open` | Ouvre VLC avec les paramètres de journalisation et de debug activés. |
| `task client:linux:install` | Installe l'agent client `dbsrt` et configure le service systemd utilisateur. |
| `task client:linux:status` | Affiche le statut du service systemd `dbsrt-agent`. |
| `task client:linux:logs` | Affiche les logs en direct du service client Linux via `journalctl`. |
| `task target:status` | Inspecte le mode actif du démon pour VLC (Local Zero-DB, Cloud o2switch ou RPi4). |
| `task target:local` | Bascule le backend VLC en mode 100% Local autonome (Zero-DB / RAM MIH). |
| `task target:o2switch` | Bascule le backend VLC en mode relais Cloud o2switch (`O2SWITCH_URL`, ex: `https://<your-domain>/dbsrt`). |
| `task target:rpi4` | Bascule le backend VLC en mode relais sur l'appliance Raspberry Pi 4 locale. |

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
| `task test:scenarios` | Exécute la suite de scénarios TUI déclaratifs headless (`test_scenarios.rs`) en moins de 0.5s. (Alias : `task test:tui`). |
| `task test:infra` | Exécute le harnais de tests d'infrastructure et chaos testing. |

---

## 7. Appliance Domestique Raspberry Pi 4 (`rpi4:*`)

> [!IMPORTANT]
> **Distinction des familles de cibles RPi4** :
>
> * **Serveur Backend (Docker)** : `task rpi4:build` (build image `dbsrt-server`), `task rpi4:push` (stream SSH de l'image), `task rpi4:up` (stack Docker).
> * **Synchronisation des Données** : `task rpi4:sync` (transfère `catalog.dbphash`, `data/SRT` et docker-compose par `rsync`, **aucun** binaire).
> * **CLI Standalone & TUI (`top`)** : `task rpi4:cli:build` (compilation Debian Bullseye GLIBC 2.31), `task rpi4:cli:deploy` (copie SCP de `dbsrt` vers `~/.local/bin/`), `task rpi4:top` (session interactive SSH).

| Commande | Description |
| :--- | :--- |
| `task rpi4:build` | Cross-compile le binaire aarch64 et génère l'image Docker ARM64 de `dbsrt-server`. |
| `task rpi4:push` | Transfère l'image Docker vers le Raspberry Pi 4 via SSH. |
| `task rpi4:up` | Démarre la pile de conteneurs sur le Raspberry Pi distant. |
| `task rpi4:down` | Arrête la pile de conteneurs sur le Raspberry Pi distant. |
| `task rpi4:status` | Interroge la sonde de santé HTTP du Raspberry Pi 4. |
| `task rpi4:logs` | Suit les journaux en direct de `dbsrt-server` sur le Raspberry Pi. |
| `task rpi4:sync` | Synchronise le catalogue Zero-DB et les sous-titres par rsync vers le Raspberry Pi. |
| `task rpi4:demo` | Teste la reconnaissance d'un extrait de 200 octets sur l'appliance distante. |
| `task rpi4:cli:build` | Cross-compile le binaire standalone ARM64 `dbsrt` avec parité Debian Bullseye GLIBC 2.31. |
| `task rpi4:cli:test` | Valide localement sur l'hôte le binaire ARM64 sous Debian Bullseye via émulation QEMU binfmt. |
| `task rpi4:cli:deploy` | Déploie le binaire standalone ARM64 `dbsrt` vers `~/.local/bin/dbsrt` sur le Raspberry Pi 4 via SSH. |
| `task rpi4:top` | Lance une session interactive SSH de `dbsrt top` directement sur le Raspberry Pi 4. |
| `task rpi4:server:deploy` | Déploie le binaire standalone ARM64 `dbsrt-server` et configure le service systemd utilisateur. |
| `task rpi4:server:start` | Démarre le service systemd utilisateur `dbsrt-server` sur le Raspberry Pi 4. |
| `task rpi4:server:stop` | Arrête le service systemd utilisateur `dbsrt-server` sur le Raspberry Pi 4. |
| `task rpi4:server:restart` | Redémarre le service systemd utilisateur `dbsrt-server` sur le Raspberry Pi 4. |
| `task rpi4:server:status` | Vérifie l'état du service systemd et interroge la sonde `/v1/status` sur le Raspberry Pi 4. |
| `task rpi4:server:logs` | Suit en direct les logs journald du service systemd `dbsrt-server` sur le Raspberry Pi 4. |
| `task rpi4:server:demo` | Exécute un test de reconnaissance et de téléchargement CAS sur le serveur standalone RPi4. |
| `task rpi4:db:up` | Démarre le conteneur PostgreSQL 17 SP-GiST BK-Tree sur le Raspberry Pi 4 (port 5433). |
| `task rpi4:db:status` | Vérifie l'état des migrations Refinery sur l'instance PostgreSQL distante du Raspberry Pi 4. |
| `task rpi4:db:migrate` | Applique les migrations Refinery sur l'instance PostgreSQL distante du Raspberry Pi 4. |
| `task rpi4:db:down` | Arrête le conteneur PostgreSQL 17 sur le Raspberry Pi 4. |
| `task rpi4:db:tui` | Lance l'explorateur interactif plein écran SQL TUI Harlequin sur le PostgreSQL 17 du RPi4 (port 5433). |
| `task rpi4:db:audit` | Génère le bilan des valeurs `NULL` et de la vue `v_productions` sur le PostgreSQL 17 du RPi4. |
| `task rpi4:db:sync` | Réplique instantanément les spécifications physiques sondées (`width`, `height`, `codec`, `resolution`) du local vers le RPi4. |
| `task rpi4:db:probe` | Sonde les flux NAS et met à jour directement le PostgreSQL 17 du Raspberry Pi 4. |

---

## 8. Hébergement Mutualisé o2switch (`o2switch:*`)

| Commande | Description |
| :--- | :--- |
| `task o2switch:build` | Cross-compile les exécutables statiques autonomes `x86_64-unknown-linux-musl` (`dbsrt`, `dbsrt-server`) et applique `strip`. |
| `task o2switch:test-local` | Valide empiriquement le binaire musl en mode Zero-DB localement sur le port `8769`. |
| `task o2switch:sync` | Synchronise les binaires statiques, la passerelle `index.php` + `.htaccess` et les sous-titres (`--update`) par rsync. Le catalogue `catalog.dbphash` local n'est envoyé que s'il manque sur o2switch : le catalogue distant est recompacté sur place. |
| `task o2switch:batch:tvshow` | Exécute l'ingestion par batch distante sur les séries vierges (/tvshow, supporte `--daemon`, `--fresh-only`, `--retry-failed`, `--max-extraction-secs`). |
| `task o2switch:batch:movies` | Exécute l'ingestion par batch distante sur les films vierges (/video, supporte `--daemon`, `--fresh-only`, `--retry-failed`, `--max-extraction-secs`). |
| `task o2switch:batch:status` | Vérifie l'état d'exécution du démon batch (PID, CPU, mémoire, progression détaillée de la file). |
| `task o2switch:batch:workers` | Interroge ou règle dynamiquement la concurrence des workers sur o2switch (`run/desired_workers`). |
| `task o2switch:batch:stop` | Arrête proprement le travailleur batch en arrière-plan sur o2switch. |
| `task o2switch:batch:logs` | Suit en direct (`tail -f`) les journaux d'exécution du batch distant sur o2switch. |
| `task o2switch:monitoring` | Affiche l'URL publique du tableau de bord web et le lien direct d'authentification mobile 4G/5G. |
| `task o2switch:top` | Ouvre le moniteur TUI plein écran en direct sur o2switch via SSH (`ssh -t`). |
| `task o2switch:pull-catalog` | Rapatrie le catalogue compilé `catalog.dbphash`, les sous-titres SRT et journaux depuis o2switch vers le stockage local / appliance. |
| `task o2switch:backup-nas` | Rapatrie les empreintes brutes unitaires `.phash` depuis o2switch vers le NAS Synology / stockage persistant de sauvegarde. |
| `task o2switch:start` | Démarre le démon en arrière-plan sans root sur o2switch et attend la validation de la sonde `/v1/health`. |
| `task o2switch:stop` | Arrête proprement le démon à distance via signal `SIGTERM` (avec bascule `SIGKILL` de sécurité). |
| `task o2switch:restart` | Redémarre le démon `dbsrt-server` sur o2switch. |
| `task o2switch:status` | Affiche l'état du processus (PID, mémoire RSS, CPU) et la télémétrie de l'API `/v1/status`. |
| `task o2switch:logs` | Suit en temps réel le fichier journal `dbsrt-server.log` sur o2switch via SSH. |
| `task o2switch:demo` | Teste la reconnaissance d'empreintes et la récupération CAS de sous-titres sur o2switch. |
| `task o2switch:setup-cron` | Installe le watchdog utilisateur non-root (`*/5 * * * *`) et le recompactage du catalogue (`*/15 * * * *`) dans la crontab. |
| `task o2switch:remove-cron` | Supprime le watchdog utilisateur de la crontab. |
| `task o2switch:rotate-key` | Génère, déploie et valide une nouvelle clé API 256 bits avec synchronisation dans `pass`. |
| `task o2switch:sync-nas` | Compile le catalogue binaire depuis le NAS Synology et synchronise l'index et les sous-titres sur o2switch. |
| `task o2switch:sync-nas:dry-run` | Simule à sec la synchronisation du catalogue NAS vers o2switch sans transfert effectif ni redémarrage. |
| `task o2switch:agent:start` | Démarre l'agent relais local connecté au Cloud o2switch pour VLC et le NAS Synology. |
| `task o2switch:agent:stop` | Arrête l'agent relais local. |
| `task o2switch:agent:restart` | Redémarre l'agent relais local. |
| `task o2switch:agent:status` | Affiche l'état et les métriques des requêtes relayées de l'agent local. |
| `task o2switch:agent:logs` | Suit en direct les logs d'exécution de l'agent relais local. |
| `task o2switch:agent:demo` | Valide de bout en bout la reconnaissance d'un fichier réel du NAS via le Cloud. |
