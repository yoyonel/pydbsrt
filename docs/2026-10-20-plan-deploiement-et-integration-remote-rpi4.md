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

Ajout du groupe déclaratif `rpi4:*` et `sim:rpi4:*` :

* `task rpi4:build` : Cross-compilation sur le PC hôte et packaging de l'image `dbsrt-server:rpi4` (`linux/arm64`).
* `task sim:rpi4:up` : Démarre le conteneur ARM64 en simulation locale QEMU sur x86 avec bridage RAM.
* `task sim:rpi4:test` : Exécute les tests de conformité API (`/v1/recognize`) contre le conteneur simulé.
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
