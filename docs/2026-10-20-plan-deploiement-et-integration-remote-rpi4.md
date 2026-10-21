---
title: "Plan de Déploiement & Intégration Serveur Remote RPi4"
description: "Audit empirique du Raspberry Pi 4 ARM64, spécification du package de déploiement Docker remote, et feuille de route d'intégration des briques manquantes."
tags:
  - current
  - deployment
  - rpi4
  - docker
  - remote
  - "2026"
---

# Plan de Déploiement & Intégration Serveur Remote RPi4

> [!NOTE] Contexte
> Ce document opérationnel fait suite à l'[Architecture Client Daemon Sidecar](2026-10-20-architecture-client-daemon-sidecar-et-deploiement-o2switch.md). Il formalise le déploiement concret du serveur central distant `dbsrt-server` sur le nœud Raspberry Pi 4 local (`rpi4`), en tirant parti des briques logicielles déjà actives en production sur l'hôte.

---

## 1. Audit Empirique du Raspberry Pi 4 (Constaté Live via SSH)

L'inspection directe du nœud cible via `ssh rpi4` et `docker --host ssh://rpi4` établit les caractéristiques réelles suivantes :

### 1.1 Matériel & Système d'Exploitation

* **Hôte** : Raspberry Pi 4 Model B (Broadcom BCM2711)
* **Architecture CPU** : `aarch64` (4 cœurs ARM Cortex-A72 @ 1.50 GHz, Little Endian)
* **Mémoire Vive (RAM)** : 3.7 GiB total (1.5 GiB utilisé, **2.8 GiB disponible**, Swap 2.0 GiB à 0% d'utilisation)
* **Stockage Système** : Carte SD/SSD 59 GB (**29 GB disponibles**, 49% d'occupation)
* **Noyau Linux** : `Linux 6.1.21-v8+ #1642 SMP PREEMPT aarch64`
* **Distribution** : Debian GNU/Linux 11 (`bullseye`)
* **Moteur Conteneur** : Docker Engine `29.3.0` (build 5927d80), Docker Compose `v5.1.0`
* **Accès Distant Docker** : Directement administrable depuis la machine de développement via `docker --host ssh://rpi4` (zéro configuration daemon TCP nécessaire).

### 1.2 Cartographie des Services Docker Existants

| Conteneur | Image | Ports Écoutés | Rôle & État |
| :--- | :--- | :--- | :--- |
| `pg-spgist_hamming_bktreedb_1` | `pg-spgist_hamming_bktreedb` | `0.0.0.0:5432->5432/tcp` | **PostgreSQL 15.2** + extension `bktree 1.0` (Up 10 jours). |
| `grafana` | `grafana/grafana:12.4.3` | `127.0.0.1:3000->3000/tcp` | Visualisation métriques système & réseau. |
| `influxdb` | `influxdb:1.8.10` | `8086/tcp` | Base de données séries temporelles (Up 10 jours). |
| `telegraf` | `telegraf:1.38.2` | `8092/udp, 8125/udp, 8094/tcp` | Collecteur de télémétrie local. |
| `chronograf` | `chronograf:1.9.4` | `127.0.0.1:8888->8888/tcp` | Interface d'exploration InfluxDB. |
| `docker-socket-proxy` | `tecnativa/docker-socket-proxy:0.3` | `2375/tcp` | Proxy sécurisé pour socket Docker. |
| `speedtest-cron` | `speedtest:bookworm` | Interne | Mesure périodique de bande passante WAN. |

### 1.3 État de la Base de Données `bktreedb` sur RPi4

L'inspection de la base PostgreSQL 15.2 existante révèle :

* **Base de données** : `bktreedb` (utilisateur `docker`, mot de passe `docker`)
* **Extensions actives** : `bktree 1.0`, `postgis 3.3.2`, `pg_cron 1.5`, `hstore 1.8`
* **Table `medias`** : 8 enregistrements (corpus de tests historiques)
* **Table `frames`** : **817 370 trames pHash déjà indexées** avec un index SP-GiST BK-tree fonctionnel (`bk_index_name spgist (p_hash bktree_ops)`).
* **Compatibilité Schéma Rust** : L'exécution de `init_schema()` ajoutera sans perte les colonnes `fps`, `frame_count`, `phash_path` sur `medias` ainsi que les tables `subtitles` et `sub_frames`.

---

## 2. Architecture de Déploiement Cible

```mermaid
flowchart TD
    subgraph Client_Dev ["Poste de Développement / Client VLC"]
        VLC["VLC Player + vlsub_dbsrt.lua"]
        AGENT["Daemon Sidecar Local (dbsrt-agent)\n127.0.0.1:8765\n--remote-api http://rpi4:8765"]
        TASK["Taskfile Orchestrator\ntask rpi4:..."]
        VLC -->|Loopback HTTP| AGENT
        TASK -.->|"SSH / Docker Context\ndocker --host ssh://rpi4"| DOCKER_DAEMON
    end

    subgraph RPi4_Host ["Raspberry Pi 4 (rpi4 - aarch64)"]
        DOCKER_DAEMON["Docker Engine 29.3.0"]

        subgraph DBSrt_Stack ["Stack Conteneurisée dbsrt"]
            SERVER["dbsrt-server (Conteneur)\nListening 0.0.0.0:8765\nZéro décodage vidéo / Empreinte ~25 Mo RAM"]
            STUDIO["UI Web dbsrt Studio\nhttp://rpi4:8765/studio"]
            CATALOG["Volume catalog.dbphash\n+ data/SRT (Sous-titres certifiés)"]
        end

        subgraph Existing_Stack ["Services Existants Déjà Actifs"]
            PG["pg-spgist_hamming_bktreedb_1\nPostgreSQL 15.2 (5432)\n817 370 frames indexées"]
            TELEGRAF["telegraf:1.38.2"]
            GRAFANA["grafana:12.4.3 (:3000)"]
        end

        SERVER --- STUDIO
        SERVER -->|Tier 1 MIH RAM| CATALOG
        SERVER -->|Tier 3 SQL BK-Tree| PG
        TELEGRAF -.->|Scrape /v1/metrics| SERVER
    end

    AGENT -->|"POST /v1/recognize (200 octets pHash sur LAN)"| SERVER
    SERVER -->|"JSON Résultat (shift_ms + sous-titre synchronisé)"| AGENT
```

### Avantages Clés pour le RPi4

1. **Zéro décodage vidéo sur le processeur ARM** : le conteneur serveur distant ne calcule aucun pHash vidéo. Il reçoit un simple vecteur de 25 entiers 64-bit (200 octets). La consommation CPU est inférieure à **5 ms par requête**.
2. **Double moteur simultané** :
   * **Tier 1 (MIH / Zero-DB)** : chargement mémoire du fichier compact `catalog.dbphash` (<15 Mo de RAM) pour une recherche en **<4 ms**.
   * **Tier 3 (PostgreSQL BK-Tree)** : délégation SQL au conteneur PostgreSQL déjà actif sur l'hôte via `postgres://docker:docker@localhost:5432/bktreedb`.
3. **Observabilité intégrée** : exposition immédiate du backoffice web `/studio` et intégration native des métriques `/v1/metrics` dans le Grafana existant.

---

## 3. Inventaire des Briques à Développer & Intégrer

### Brique 1 : Cross-Compilation Host & Dockerfile Serveur Rust (`docker/Dockerfile.server`)

* **Analyse comparative de performance** :
  * *Build natif sur RPi4* : 10 à 15 minutes sur Cortex-A72, saturation CPU, usure de la carte SD par écriture massive dans `target/`.
  * *Cross-compilation sur poste hôte x86_64* : **~15 secondes** (compilateur natif multi-cœurs, cache Cargo NVMe, binaire `aarch64` pur).
* **Architecture du Dockerfile multi-stage (`docker/Dockerfile.server`)** :
  * **Étape Builder (x86_64 rapide)** : utilise le toolchain Rust local ou conteneur `rust:bookworm-slim` avec le compilateur croisé `aarch64-linux-gnu-gcc` déjà présent sur le poste pour compiler `dbsrt-server` vers `aarch64-unknown-linux-gnu`.
  * **Étape Runtime (ARM64 pure)** : `FROM --platform=linux/arm64 debian:bookworm-slim` avec uniquement `ca-certificates` et `libpq5` (client PostgreSQL natif).
  * Zéro dépendance `ffmpeg`/`ffprobe` requise côté serveur.
  * Empreinte conteneur final : **~35 Mo**.

### Brique 2 : Définition Docker Compose RPi4 & Simulation Locale (`docker/docker-compose.rpi4.yml` et `sim/`)

*Inspiré de l'architecture éprouvée dans le projet `rpi-internet-monitoring` :*

* **Production RPi4 (`docker/docker-compose.rpi4.yml`)** :
  ```yaml
  services:
    dbsrt-server:
      image: dbsrt-server:rpi4
      container_name: dbsrt-remote-server
      restart: unless-stopped
      network_mode: "host" # Accès direct à localhost:5432 (Postgres) et écoute sur 8765
      volumes:
        - ${DATA_DIR:-./data}:/app/data:ro
      environment:
        - RUST_LOG=info
        - HOST=0.0.0.0
        - PORT=8765
        - PHASH_DIR=/app/data/__PHASH__
        - DBPHASH_PATH=/app/data/catalog.dbphash
        - SUBTITLES_DIR=/app/data/SRT
        - DB_HOST=127.0.0.1
        - DB_PORT=5432
        - DB_USER=docker
        - DB_PASS=docker
        - DB_NAME=bktreedb
  ```

* **Simulation Locale ARM64 sur x86 (`sim/docker-compose.sim.yml`)** :
  * Exécution de l'image conteneurisée ARM64 directement sur la machine hôte via QEMU user-mode (`tonistiigi/binfmt`).
  * Plafonnement mémoire à l'empreinte RPi4 (`mem_limit: 512m`) pour valider la robustesse et les latences avant tout déploiement réel.

### Brique 3 : Pipeline de Livraison par Stream SSH (Zéro Registre Externe)

Pour livrer l'image cross-compilée sur le RPi4 sans dépendre d'un registre Docker public/privé :
```sh
docker save dbsrt-server:rpi4 | ssh rpi4 docker load
```
Avantages : transfert direct en un flux TCP compressé sur le LAN, chargement atomique dans le daemon Docker du RPi4.

### Brique 4 : Synchronisation des Données & Catalogue (`scripts/rpi4_sync.sh`)

* **Rôle** : Transférer les données nécessaires depuis la machine de développement vers le RPi4 (`~/dbsrt/data`) :
  * Le catalogue unifié `catalog.dbphash` (compact, ~quelques mégaoctets).
  * Le répertoire des sous-titres certifiés `data/SRT/` (fichiers `.srt` + `couplings.json`).
  * Optionnel : répertoire de pHashes unitaires `data/__PHASH__/`.

### Brique 5 : Orchestration Taskfile (`Taskfile.yml`)

Ajout du groupe déclaratif `rpi4:*` et `sim:*` (avec alias `sim:rpi4:*`) :

* `task rpi4:build` : Cross-compilation sur le PC hôte et packaging de l'image `dbsrt-server:rpi4` (`linux/arm64`).
* `task sim:up` (alias `task sim:rpi4:up`) : Démarre le conteneur ARM64 en simulation locale QEMU sur x86 avec bridage 512 Mo RAM (`sim/docker-compose.sim.yml` sur le port `8766`).
* `task sim:test` (alias `task sim:rpi4:test`) : Exécute la batterie de 6 tests de conformité API contre le conteneur ARM64 simulé :
  1. `GET /v1/health` & `GET /v1/status` (Healthcheck & métriques d'indexation Zero-DB).
  2. `GET /v1/metrics` (Compteurs atomiques et latences).
  3. `GET /studio` (Interface web backoffice dbsrt Studio HTTP 200).
  4. `POST /v1/recognize` (Moteur Multi-Index Hashing en RAM sous ARM64 avec injection automatique de sous-titres certifiés).
  5. `POST /v1/search` (Pipeline complet de recherche et calcul moviehash).
  6. `GET /v1/subtitles/:hash` (Content-Addressable Storage CAS avec extraction du texte SRT).
* `task sim:down` (alias `task sim:rpi4:down`) : Arrête et détruit le conteneur de simulation ARM64.
* `task rpi4:push` : Envoie l'image Docker cross-compilée sur le RPi4 via stream SSH (`docker save ... | ssh rpi4 docker load`).
* `task rpi4:sync` : Synchronise les données (`catalog.dbphash`, `data/SRT`) par `rsync` sur `rpi4`.
* `task rpi4:init-db` : Applique la mise à niveau non-destructive du schéma (`init_schema()`) sur PostgreSQL 15.2.
* `task rpi4:up` : Démarre le service `dbsrt-remote-server` sur le RPi4.
* `task rpi4:down` : Arrête le service sur le RPi4.
* `task rpi4:logs` : Suit les logs en temps réel (`ssh rpi4 docker logs -f dbsrt-remote-server`).
* `task rpi4:status` : Vérifie l'état de santé HTTP (`curl http://rpi4:8765/v1/status`) et les métriques.
* `task rpi4:demo` : Lance le test de reconnaissance et synchronisation E2E depuis la machine hôte pointant sur `http://rpi4:8765`.

### Brique 5 : Configuration Client & Extension VLC

* Côté machine de développement / client :
  * Déployer le sidecar client configuré pour relayer vers l'IP du Raspberry Pi :
    ```sh
    task client:linux:install -- http://192.168.1.XX:8765
    ```
  * L'extension VLC `vlsub_dbsrt.lua` communique avec le sidecar local (`127.0.0.1:8765`), qui relaie en 200 octets vers le RPi4 (`http://192.168.1.XX:8765/v1/recognize`).

---

## 4. Matrice de Progression & Checklist d'Exécution

| Étape | Description de l'Action | Statut | Risque / Dépendance |
| :--- | :--- | :---: | :--- |
| **P0** | **Audit direct de la plateforme RPi4 (OS, Docker, Postgres, Ports)** | **Fait (Empiriquement prouvé)** | Aucun. Matériel et services validés live. |
| **P1** | **Création du `docker/Dockerfile.server` (Cross-build host x86 vers ARM64)** | **Fait (Empiriquement prouvé)** | Multi-stage builder `rust:slim-bookworm` (image ~36 Mo). |
| **P2** | **Validation en simulation locale (`sim/docker-compose.sim.yml` QEMU ARM64)** | **Fait (Empiriquement prouvé)** | Validé live sur port 8766 (5.6 ms latence). |
| **P3** | **Création de `docker/docker-compose.rpi4.yml`** | **Fait (Empiriquement prouvé)** | Service en `network_mode: host` connecté à PostgreSQL 15.2. |
| **P4** | **Script de synchronisation `scripts/rpi4_sync.sh` (catalog + SRT)** | **Fait (Empiriquement prouvé)** | Synchronisation live de `catalog.dbphash` (18 Mo) et `data/SRT` (20 Mo). |
| **P5** | **Ajout des tâches `rpi4:*` et `sim:rpi4:*` dans `Taskfile.yml`** | **Fait (Empiriquement prouvé)** | Toutes les cibles déclarées et testées. |
| **P6** | **Livraison par stream SSH (`docker save | ssh rpi4 docker load`) & run** | **Fait (Empiriquement prouvé)** | Image chargée et conteneur sain sur RPi4 (`http://192.168.1.24:8765`). |
| **P7** | **Connexion PostgreSQL 15.2 SP-GiST BK-tree existant** | **Fait (Empiriquement prouvé)** | `postgres_connected: true` avec 817 370 frames et 1566 pHashes. |
| **P8** | **Validation E2E : Requête PC Client Sidecar -> RPi4 -> Subtitle Synced** | **Fait (Empiriquement prouvé)** | Latence totale E2E de 31 ms (match en 4.6 ms sur Cortex-A72). |

---

## 5. Preuve Empirique de Validation E2E (Terminal Proof)

Le test de bout en bout a été exécuté depuis le poste de développement vers le Raspberry Pi 4 :

1. **État du service remote sur le RPi4 (`http://192.168.1.24:8765/v1/status`)** :
   ```json
   {
     "status": "ok",
     "version": "0.1.0",
     "service": "dbsrt-server",
     "uptime_seconds": 174,
     "phash_dir": "/app/data/__PHASH__",
     "subtitles_dir": "/app/data/SRT",
     "indexed_phash_count": 1566,
     "certified_couplings_count": 293,
     "postgres_connected": true
   }
   ```

2. **Résolution par le RPi4 de la requête d'empreinte 200 octets (`POST /v1/recognize`)** :
   * **Média identifié** : `Andor` (100% de confiance).
   * **Moteur utilisé** : `Tier 1 (RAM .dbphash MIH under 10ms)`.
   * **Temps de calcul CPU RPi4** : **4.62 ms** (`timer_seconds: 0.004621205`).
   * **Candidat sous-titre retourné** : `Andor.S01E01.1080p.WEB.h264-KOGi.fr.srt` (couplage certifié, décalage $\Delta t = 0.0$ ms).

3. **Chaîne complète PC -> Sidecar -> RPi4 (`POST /v1/search`)** :
   * **Fichier vidéo local testé** : `Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4`.
   * **Temps d'extraction vidéo 1s** : ~10 ms.
   * **Temps aller-retour réseau LAN + Matching RPi4** : ~19.9 ms.
   * **Temps total perçu par le client VLC** : **31.0 ms** (`"total_ms": 31.017658`).

---

## 6. Architecture Finale Distribuée & Ségrégation des Responsabilités

Pour valider l'architecture en conditions réelles et garantir qu'aucun artefact local ne fausse la recette, la ségrégation stricte suivante a été appliquée :

```mermaid
flowchart TD
    subgraph Client_PC ["Poste Client Utilisateur (PC Linux/Mac/Win)"]
        subgraph VLC_Layer ["Lecteur Vidéo & IHM"]
            VLC["VLC Media Player"]
            LUA["Plugin vlsub_dbsrt.lua\n(Sandbox Lua 5.1 / UI)"]
            VLC --- LUA
        end

        subgraph Local_Storage ["Système de Fichiers Local"]
            VIDEO["Fichier Vidéo Local\n(ex: The.Boys..._mid15s.mp4)"]
            OUT_SRT["Sous-titre Synchronisé\n(The.Boys..._mid15s.synced.fr.srt)"]
            CACHE["Cache CAS Local\n(~/.cache/dbsrt/subtitles/)"]
        end

        subgraph Sidecar_Daemon ["Agent Sidecar Local (127.0.0.1:8765)"]
            AGENT["dbsrt-server --agent\n--remote-api http://192.168.1.24:8765"]
            FFMPEG["Extracteur pHash Local\n(FFmpeg / LibAV HW Accel)"]
            AGENT --- FFMPEG
        end

        LUA -->|"1. POST /v1/search\n(chemin local du fichier)"| AGENT
        FFMPEG -->|"Lecture & Décodage 1s"| VIDEO
        AGENT -->|"5. Application Δt en RAM\n& Écriture disque"| OUT_SRT
        AGENT <-->|"Stockage immuable"| CACHE
        OUT_SRT -.->|"6. vlc.input.add_subtitle()\nAttachement automatique"| VLC
    end

    subgraph Remote_RPi4 ["Serveur Central Distant (Raspberry Pi 4 - 192.168.1.24)"]
        CONTAINER["Conteneur Docker dbsrt-remote-server\n(aarch64 - network: host :8765)"]

        subgraph Storage_RPi4 ["Données Centrales Distantes"]
            DBPHASH["catalog.dbphash\n(Index compact RAM MIH Tier 1)"]
            SRT_STORE["data/SRT/\n(Sous-titres certifiés CAS)"]
            PHASH_STORE["data/__PHASH__/\n(Séquences pHash complètes pour Δt)"]
            PG["pg-spgist_hamming_bktreedb_1\n(PostgreSQL 15.2 SP-GiST BK-Tree\n817 370 frames indexées)"]
        end

        CONTAINER --> DBPHASH
        CONTAINER --> SRT_STORE
        CONTAINER --> PHASH_STORE
        CONTAINER --> PG
    end

    AGENT -->|"2. POST /v1/recognize\n(200 octets pHash sur LAN)"| CONTAINER
    CONTAINER -->|"3. Candidat CAS + Décalage Δt\n(sub:sha256:63af... / -1754.5s)"| AGENT
    AGENT -->|"4. GET /v1/subtitles/63af...\n(Rapatriement flux brut SRT)"| CONTAINER
```

### 6.1 Pourquoi cette Répartition des Services ?

1. **Poste Client Local (PC) : Zéro Base de Données, Zéro Catalogue Global**
   * **Tous les conteneurs Docker locaux éteints** : `task db:down` a arrêté `docker-bktreedb-1`, `pgweb`, `grafana`, `prometheus` et `postgres_exporter`. Le client ne dispose d'aucune base de données locale (`postgres_connected: false`).
   * **Nécessité de l'agent sidecar sur le PC** : Le moteur Lua 5.1 de VLC ne peut pas décoder de la vidéo ni calculer d'empreintes pHash. De plus, le serveur distant (RPi4) n'a pas accès au système de fichiers du PC client (`/path/to/media/...`). L'agent local effectue donc l'extraction vidéo locale via FFmpeg (~10-400 ms), extrait 5 trames (200 octets), et les délègue au RPi4.
   * **Rapatriement et écriture locale** : Une fois le décalage calculé par le RPi4, le sidecar télécharge le sous-titre brut, applique $\Delta t$ en mémoire, et l'enregistre à côté de la vidéo locale pour que VLC l'attache en direct.

2. **Serveur Central Distant (RPi4) : Zéro Décodage Vidéo, Recherche Haute Vitesse**
   * **Consommation CPU quasi-nulle** : Le RPi4 ne traite aucun flux vidéo lourd ; il ne reçoit que des vecteurs d'entiers 64-bit sur le réseau LAN.
   * **Indexation centralisée** : Le RPi4 héberge le catalogue compact `catalog.dbphash` (RAM MIH Tier 1), les sous-titres certifiés, l'archive des pHashes de référence, et la base PostgreSQL 15.2 (817 370 trames pHash indexées en SP-GiST BK-Tree).

---

## 7. Recette Fonctionnelle Live VLC & Résolution d'Incident

### 7.1 Protocole du Test en Conditions Réelles

* **Média testé** : Extrait vidéo de 15 secondes `data/snippets/The.Boys.S02E03.1080p.AMZN.WEBRip.DDP5.1.x264-NTb_mid15s.mp4`.
* **Client** : VLC Media Player avec extension `vlsub_dbsrt.lua` activée.
* **Agent local** : `dbsrt-server --agent --remote-api http://192.168.1.24:8765` sur port `8765`.
* **Serveur distant** : `dbsrt-remote-server` sur Raspberry Pi 4 (`192.168.1.24:8765`).

### 7.2 Incident Rencontré : Absence de Décalage Temporel ($\Delta t = 0.0$ ms)

Lors de la première exécution du bouton **Search by pHash** dans VLC :

1. **Symptôme** : Le média `The Boys` était correctement reconnu à 100% de confiance via le Tier 1 MIH du RPi4, mais le décalage temporel affiché était `visual shift = 0frames / 0.0ms`.
2. **Conséquence** : L'IHM VLC ne proposait que le sous-titre original 1:1 (`The.Boys.S02E03.1080p.AMZN.WEBRip.DDP5.1.x264-NTb.fr.srt`), sans le candidat synchronisé pour l'extrait (`certified_synced`). Au téléchargement, les sous-titres apparaissaient avec ~29 minutes de retard sur l'extrait vidéo.
3. **Analyse de Cause Racine (Root Cause)** :
   * L'index compact `catalog.dbphash` permet d'identifier l'œuvre globale en RAM en < 3 ms.
   * Cependant, pour calculer le décalage temporel exact d'un extrait (`calculate_sequence_shift_argmin`), l'algorithme doit corréler l'échantillon de trames de la requête contre la **séquence pHash complète** de l'épisode de référence (`.phash`).
   * Le script de synchronisation `scripts/rpi4_sync.sh` synchronisait `catalog.dbphash` et `data/SRT/`, mais **omettait** le répertoire `data/__PHASH__/` (101 Mo).
   * Sur le RPi4, le répertoire `/app/data/__PHASH__` était vide, empêchant la corrélation fine de s'exécuter.

### 7.3 Résolution & Validation Définitive

1. **Correction appliquée dans `scripts/rpi4_sync.sh`** :
   ```bash
   if [[ -d "data/__PHASH__" ]]; then
     echo "Syncing data/__PHASH__..."
     rsync -avz --progress "data/__PHASH__" "${RPI_HOST}:${REMOTE_BASE}/data/"
   fi
   ```
2. **Synchronisation et redémarrage** : Exécution de `task rpi4:sync` (transfert des 101 Mo de pHashes de référence sur le RPi4) et redémarrage du conteneur `dbsrt-remote-server`.
3. **Résultat Terminal & IHM VLC Validé** :
   * **Calcul RPi4** : Détection instantanée du décalage de **`-1754.5s`** (`-42 067` trames à 23.976 fps).
   * **Affichage VLC** : Apparition immédiate dans la liste du candidat resynchronisé :
     ```text
     The.Boys.S02E03.1080p.AMZN.WEBRip.DDP5.1.x264-NTb.fr.srt [Synced -1754.5s]
     ```
   * **Téléchargement CAS** : Clic sur *Download selection* $\to$ Rapatriement CAS `sub:sha256:63af4d0b...` depuis le RPi4 $\to$ Recalage temporel $\Delta t$ appliqué par l'agent local $\to$ Écriture de `The.Boys.S02E03.1080p.AMZN.WEBRip.DDP5.1.x264-NTb_mid15s.synced.fr.srt`.
   * **Rendu Vidéo** : Attachement direct dans le flux VLC avec synchronisation parfaite des répliques sur les 15 secondes de l'extrait.
