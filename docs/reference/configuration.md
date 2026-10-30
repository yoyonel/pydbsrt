---
title: "Référence : Configuration & Variables d'Environnement"
tags:

  - reference
  - configuration
  - ports
  - environnement
---

# 📖 Référence : Configuration & Variables d'Environnement

Cette page recense l'ensemble des variables d'environnement, des ports réseau par défaut et des constantes système régissant le fonctionnement de `pydbsrt`.

---

## 1. Matrice des Ports Réseau par Défaut

Tous les services exposent des ports configurables pour garantir la coexistence sans conflit avec d'autres conteneurs de développement :

| Service | Port par Défaut | Variable d'Environnement de Surcharge | Rôle |
| :--- | :---: | :--- | :--- |
| **`dbsrt-server` (Axum REST & Studio)** | `8765` | `DBSRT_PORT` | Microservice principal et IHM Studio |
| **PostgreSQL 17 (`bktreedb`)** | `5433` | `PSQL_PORT` | Persistance relationnelle & index SP-GiST |
| **PgWeb** | `8086` | `PGWEB_PORT` | Console SQL d'exploration web |
| **MkDocs Material** | `8000` (plage 8000–8020) | `DOCS_PORT` | Serveur de documentation avec allocation dynamique |
| **Grafana** | `3000` | `GRAFANA_PORT` | Tableaux de bord de métriques temps réel |
| **Prometheus** | `9090` | `PROMETHEUS_PORT` | Moteur de collecte métrique (TSDB) |
| **Postgres Exporter** | `9187` | `PG_EXPORTER_PORT` | Scrape des métriques internes PostgreSQL |

---

## 2. Variables d'Environnement du Serveur & CLI

Ces variables peuvent être définies dans votre environnement shell ou dans un fichier `.env` à la racine :

### Paramètres Microservice (`dbsrt-server`)
- `DBSRT_HOST` (défaut : `127.0.0.1`) : Adresse IP d'écoute de l'interface réseau.
- `DBSRT_PORT` (défaut : `8765`) : Port d'écoute HTTP. Si défini à `0`, alloue un port libre aléatoire.
- `DBSRT_PHASH_DIR` (défaut : `/tmp`) : Répertoire par défaut de lecture/écriture des fichiers d'empreintes unitaire `.phash`.
- `DBSRT_SUBTITLES_DIR` (défaut : `data/SRT`) : Répertoire de stockage des fichiers de sous-titres `.srt`.
- `DBSRT_DBPHASH` (défaut : *vide*) : Chemin optionnel vers le fichier d'archive haute densité `.dbphash` pré-compilé.
- `DBSRT_REMOTE_API` (défaut : *vide*) : URL d'un serveur distant `dbsrt-server` pour le mode relais Cloud/o2switch.
- `DBSRT_AGENT` (défaut : *faux*) : Active le mode agent client allégé sans hébergement de base locale.
- `DBSRT_API_KEY_FILE` (défaut : *vide*) : Fichier de clés API (une par ligne, `#` ignoré). Si défini, toutes les routes sauf `/health` et `/v1/health` exigent `Authorization: Bearer` ou `X-API-Key`. Équivalent de `--api-key-file`.

### Paramètres PostgreSQL (`crates/dbsrt-index`)
- `PSQL_IP` ou `PGHOST` (défaut : `127.0.0.1`) : Hôte de la base PostgreSQL 17.
- `PSQL_PORT` ou `PGPORT` (défaut : `5433`) : Port de connexion PostgreSQL.
- `PSQL_USER` ou `PGUSER` (défaut : `docker`) : Utilisateur de la base de données.
- `PSQL_PASS` ou `PGPASSWORD` (défaut : `docker`) : Mot de passe de la base de données.
- `PSQL_DBNAME` ou `PGDATABASE` (défaut : `bktreedb`) : Nom de la base de données cible.

---

## 3. Constantes Système & Algorithmiques Clés

Le projet applique une règle stricte de **zéro constante magique** : toutes les valeurs de configuration algorithmique sont typées et déclarées sous forme de constantes documentées :

| Constante | Valeur | Unité / Rôle |
| :--- | :---: | :--- |
| `DEFAULT_BASELINE_FPS` | `25.0` | Cadence de référence par défaut pour l'échantillonnage pHash |
| `DEFAULT_EXTRACT_SAMPLE_SECONDS` | `15` | Durée minimale d'échantillonnage vidéo pour identifier un flux inconnu |
| `DEFAULT_MAX_HAMMING_DISTANCE` | `10` | Seuil maximal de distance de Hamming toléré pour une correspondance positive (sur 64 bits) |
| `DEFAULT_RELAY_TIMEOUT_MS` | `1500` | Délai maximal d'attente sur les requêtes distribuées avant dégradation gracieuse |
| `PHASH_BIT_LENGTH` | `64` | Longueur binaire de l'empreinte perceptuelle d'une trame visuelle |
| `MIH_CHUNK_COUNT` | `4` | Nombre de sous-divisions de 16 bits pour l'indexation Multi-Index Hashing |

---

## 4. Paramètres de Déploiement Multi-Cibles (o2switch & Raspberry Pi 4)

Ces variables pilotent les scripts d'orchestration et cibles Taskfile pour les déploiements physiques et hébergés :

### Cible Hébergement Mutualisé o2switch (`scripts/o2switch_ctl.sh`)
- `O2SWITCH_HOST` (défaut : `o2switch`) : Hôte SSH ou alias configuré dans `~/.ssh/config`.
- `SERVER_PORT` (défaut : `8765`) : Port d'écoute interne du démon sur l'hôte distant (`127.0.0.1`).
- `REMOTE_BASE` (défaut : `dbsrt`) : Répertoire racine du projet sur l'hébergement mutualisé.
- `REMOTE_PUBLIC_HTML` (défaut : `public_html/dbsrt`) : Répertoire web public recevant la règle de reverse-proxy `.htaccess`.
- `O2SWITCH_URL` (défaut : *vide*) : URL publique HTTPS du reverse-proxy pour les tests de démonstration (ex. `https://votredomaine.fr/dbsrt`).
- `O2SWITCH_API_KEY` (défaut : auto-résolu depuis `pass show services/pydbsrt/o2switch_api_key`) : Clé secrète d'API (256 bits) envoyée dans l'en-tête `X-API-Key` pour s'authentifier auprès de la passerelle.
- `O2SWITCH_READONLY_KEY` (défaut : auto-résolu depuis `pass show services/pydbsrt/o2switch_readonly_key`) : Clé lecture seule utilisée par `monitoring`, `web-top-qr` et le harness ISO. Créée par `rotate-readonly-key`.
- `READONLY_KEY_PASS_ENTRY` (défaut : `services/pydbsrt/o2switch_readonly_key`) : Entrée `pass` de la clé lecture seule.
- `DEFAULT_RSYNC_BWLIMIT_KBPS` (valeur : `20000`) : Plafond strict de bande passante rsync (20 Mo/s) pour respecter le quota CloudLinux LVE I/O de 42 MiB/s.

### Cible Appliance Raspberry Pi 4 (`scripts/rpi4_*.sh`)
- `RPI_HOST` (défaut : `rpi4`) : Hôte SSH ou alias pour le Raspberry Pi 4.
- `REMOTE_BIN_DIR` (défaut : `~/.local/bin`) : Répertoire d'installation des exécutables autonomes `dbsrt` et `dbsrt-server`.
- `REMOTE_BASE` (défaut : `~/dbsrt`) : Répertoire racine des données et configurations sur le Raspberry Pi.
- `REMOTE_SYSTEMD_DIR` (défaut : `~/.config/systemd/user`) : Répertoire des unités de service systemd en mode utilisateur.
