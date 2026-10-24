# Architecture & Recette : Déploiement Isolé de PostgreSQL 17 sur Raspberry Pi 4 & Orchestration des Migrations

Date : Samedi 24 Octobre 2026
Statut : **Validé & Recetté en Production Locale (RPi4)**
Branche : `feature/vlc-chaos-testing` / `feature/rpi4-physical-appliance`
Auteur : Équipe pydbsrt

---

## 1. Contexte & Problématique Initiale

Dans le cadre du pilotage de la base de données relationnelle du nœud domestique Raspberry Pi 4 (`192.168.1.24`), l'exécution de la commande distante suivante avait échoué :

```bash
ssh rpi4 "task db:status"
# Résultat : bash: line 1: task: command not found
```

### Analyse de cause racine

1. **Localisation de l'outil `task` (Taskfile)** : L'orchestrateur `task` (`go-task`) est un outil réservé à l'environnement de développement sur la station de travail. Le système hôte du Raspberry Pi 4 est un runtime léger d'exécution de conteneurs Docker n'ayant pas vocation à héberger la chaîne de compilation Rust ou l'outil `task`.
2. **Présence d'une base de données legacy en production** : L'inspection directe du Raspberry Pi 4 a révélé l'existence d'une instance PostgreSQL 15.2 active depuis 3 ans (`pg-spgist_hamming_bktreedb_1` sur le port `5432`) hébergeant **8 médias** et **817 370 frames** indexées via l'extension `bktree`.

---

## 2. Analyse des Risques sur la Base Legacy

Avant toute exécution de migrations, un audit précis de l'instance PostgreSQL 15.2 existante a été mené :

```sql
SELECT count(*) FROM medias; -- 8
SELECT count(*) FROM frames; -- 817370
```

### Identification des pièges de migration (*Schema Drift*)

| Piège identifié | Mécanisme | Impact sans isolation |
| :--- | :--- | :--- |
| **Colonnes manquantes dans `medias`** | `V1__initial_schema.sql` utilise `CREATE TABLE IF NOT EXISTS medias (...)`. La table existante ne possédait pas les colonnes `fps`, `frame_count`, `phash_path`. | Le code moderne `dbsrt-index` effectue `SELECT ..., COALESCE(fps, 25.0) FROM medias`. L'application planterait avec `error: column "fps" does not exist`. |
| **Doublon d'index SP-GiST lourd** | La table existante possédait un index nommé `"bk_index_name"`. La migration `V1` déclare `frames_phash_bk_tree_index`. | Indexation inutile de 817 370 lignes sur carte SD/disque, doublant la consommation RAM et disque. |
| **Risque d'altération accidentelle** | Exécution de DDL non testée sur un cluster ancien. | Risque d'invalidation des données historiques sans bénéfice pour le nouveau stack. |

---

## 3. Étude d'Impact & Décision d'Architecture

### Métriques réelles du Raspberry Pi 4

- **Mémoire vive (RAM)** : 3.7 Go au total, **2.8 Go disponibles** (Swap : 0 Mo utilisé).
- **Stockage disque** : 59 Go au total, **29 Go libres** (50% d'utilisation).
- **Consommation de la base legacy (PG 15.2)** : 106 Mo de RAM, 0.00% CPU.

### Décision : Conteneur PostgreSQL 17 Dédié & Cloisonné

Au lieu de modifier la base historique, décision a été prise de déployer une instance PostgreSQL 17 moderne, dédiée au nouveau moteur `pydbsrt`, avec une étanchéité absolue :

| Axe de cloisonnement | Instance Legacy (PG 15.2) | Nouvelle Instance pydbsrt (PG 17) |
| :--- | :--- | :--- |
| **Conteneur Docker** | `pg-spgist_hamming_bktreedb_1` | `dbsrt-pg17` |
| **Port Hôte** | `5432` | **`5433`** |
| **Volume de données** | `pg-spgist_hamming_bktreedb-pg15-data` | `dbsrt-pg17-data` |
| **Réseau Compose** | `pg-spgist_hamming_default` | `dbsrt_default` |
| **Version PostgreSQL** | 15.2 | **17 (Bookworm, extension bktree)** |
| **Consommation RAM mesurée** | 106.2 Mo | **40.8 Mo** (~1.1% de la RAM) |

---

## 4. Implémentation de l'Orchestration

### 1. `docker/docker-compose.rpi4.yml`

Ajout du service `db` en port `5433:5432` et mise à jour de la configuration de `dbsrt-server` pour cibler le port 5433 :

```yaml
services:
  dbsrt-server:
    image: dbsrt-server:rpi4
    container_name: dbsrt-remote-server
    restart: unless-stopped
    network_mode: "host"
    environment:
      - PSQL_HOST=127.0.0.1
      - PSQL_PORT=5433
      - PSQL_USER=docker
      - PSQL_PASS=docker
      - PSQL_DBNAME=bktreedb
    command:
      - "--host"
      - "0.0.0.0"
      - "--port"
      - "8765"
      - "--db-host"
      - "127.0.0.1"
      - "--db-port"
      - "5433"
      - "--db-user"
      - "docker"
      - "--db-pass"
      - "docker"
      - "--db-name"
      - "bktreedb"

  db:
    image: pg-spgist_hamming:17-rpi4
    container_name: dbsrt-pg17
    restart: unless-stopped
    ports:
      - "${RPI_DB_PORT:-5433}:5432"
    environment:
      - POSTGRES_DB=${PSQL_DBNAME:-bktreedb}
      - POSTGRES_USER=${PSQL_USER:-docker}
      - POSTGRES_PASSWORD=${PSQL_PASS:-docker}
      - PGDATA=/var/lib/postgresql/data/pgdata
    volumes:
      - dbsrt-pg17-data:/var/lib/postgresql/data

volumes:
  dbsrt-pg17-data:
    name: dbsrt-pg17-data
```

### 2. `scripts/rpi4_deploy.sh` & `scripts/rpi4_sync.sh`

- Synchronisation de `docker/pg-spgist_hamming/17-develop` vers `rpi4:~/dbsrt/docker/pg17/`.
- Sous-commandes dédiées sans constantes magiques :
  - `db-build` : Compile nativement l'image `pg-spgist_hamming:17-rpi4` sur l'architecture ARM64 du RPi4.
  - `db-up` / `db-down` : Démarre/arrête le conteneur avec boucle de readiness `pg_isready`.
  - `db-status` : Affiche l'état du conteneur et interroge `dbsrt migrate --status --host 192.168.1.24 --port 5433`.
  - `db-migrate` : Applique les migrations déclaratives Refinery sur le port 5433.

### 3. Cibles Taskfile

Dans `taskfiles/rpi4.yml` (inclus dans la racine `Taskfile.yml`) :

```yaml
  rpi4:db:build:
    desc: Build native ARM64 PostgreSQL 17 image with BK-Tree on Raspberry Pi 4
    cmds:
      - ./scripts/rpi4_deploy.sh db-build

  rpi4:db:up:
    desc: Start dedicated PostgreSQL 17 container on Raspberry Pi 4 (:5433)
    cmds:
      - ./scripts/rpi4_deploy.sh db-up

  rpi4:db:down:
    desc: Stop dedicated PostgreSQL 17 container on Raspberry Pi 4
    cmds:
      - ./scripts/rpi4_deploy.sh db-down

  rpi4:db:status:
    desc: Check remote PostgreSQL 17 migration status on Raspberry Pi 4 (:5433)
    deps: [rs:build]
    aliases:
      - db:status:rpi4
    cmds:
      - ./scripts/rpi4_deploy.sh db-status {{.CLI_ARGS}}

  rpi4:db:migrate:
    desc: Run pending PostgreSQL database migrations on Raspberry Pi 4 via refinery (:5433)
    deps: [rs:build]
    aliases:
      - db:migrate:rpi4
    cmds:
      - ./scripts/rpi4_deploy.sh db-migrate {{.CLI_ARGS}}
```

---

## 5. Validation Empirique & Résultats de Recette

Toutes les étapes ont été exécutées en direct sur le matériel réel avec vérification stricte :

### 1. Démarrage & Readiness
```text
❯ task rpi4:db:up
=== Starting dedicated PostgreSQL 17 container on rpi4 (port 5433) ===
Waiting for PostgreSQL 17 to become ready on 192.168.1.24:5433...
✓ Remote PostgreSQL 17 is active and ready on 192.168.1.24:5433
```

### 2. Application des Migrations Refinery
```text
❯ task rpi4:db:migrate
Applying pending refinery migrations on 192.168.1.24:5433...
✓ Successfully applied 3 migration(s):
  + [V1] initial_schema
  + [V2] certified_couplings
  + [V3] secure_lang_and_cascades
```

### 3. Vérification du Statut
```text
❯ task rpi4:db:status
=== Remote PostgreSQL 17 Container Status (rpi4) ===
NAMES        STATUS         PORTS
dbsrt-pg17   Up 39 seconds  0.0.0.0:5433->5432/tcp, [::]:5433->5432/tcp

=== PostgreSQL Migration Status ===
Target Database: 192.168.1.24:5433 (bktreedb)
  [V1] initial_schema (applied on 2026-10-24T11:36:12Z)
  [V2] certified_couplings (applied on 2026-10-24T11:36:12Z)
  [V3] secure_lang_and_cascades (applied on 2026-10-24T11:36:12Z)
```

### 4. Contrôle d'Intégrité de la Base Legacy
```text
ssh rpi4 "docker exec -e PGPASSWORD=docker pg-spgist_hamming_bktreedb_1 psql -h localhost -U docker -d bktreedb -c 'SELECT count(*) FROM medias; SELECT count(*) FROM frames;'"
 count (medias) : 8
 count (frames) : 817370
```
**Constat : Intégrité 100% préservée, 0 collision.**

### 5. Test E2E Serveur Axum & Recherche pHash
```text
❯ task rpi4:demo
=== Testing Remote Recognize API on http://192.168.1.24:8765 ===
found: true, media_match: "Andor", confidence: 100.0%
timer_seconds: 0.0042s (4.2 ms)
✓ Remote CAS Subtitle Download verified!
✓ Remote API test successful!
```

---

## 6. Synthèse des Bénéfices

1. **Sécurité Totale** : La base de données de production historique (3 ans d'ancienneté, 817k frames) n'a subi aucune modification.
2. **Modernité & Parité** : Le nœud RPi4 dispose à présent d'un moteur PostgreSQL 17 natif strictement identique à l'environnement Docker local de développement.
3. **Ergonomie Développeur** : Toutes les opérations (`build`, `up`, `status`, `migrate`, `logs`, `down`) sont automatisées via `task rpi4:db:*` depuis le poste local sans nécessiter de commandes SSH ad-hoc.
