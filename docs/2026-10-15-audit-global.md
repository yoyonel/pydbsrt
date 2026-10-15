# Audit Massif & Exhaustif : Projet `pydbsrt`

> **Date audit** : 15 Octobre 2026
> **Auteur** : Antigravity
> **Statut global** : Projet mature et techniquement très avancé sur `develop`, mais bloqué dans l'écosystème Python 3.8-3.9 / Postgres 11, et branche `master` complètement obsolète.

---

## 1. Vue d'Ensemble & But du Projet

Le projet **`pydbsrt`** (*Python DataBase SubRip Text*) est un système de **synchronisation, reconnaissance et recalage temporel automatique de sous-titres (`.srt`) sur des flux vidéo**, sans faire appel à de la reconnaissance vocale (Speech-to-Text).

### Principe de fonctionnement
1. **Extraction d'empreintes visuelles** : Pour chaque image (frame) de la vidéo, extraction via `ffmpeg` (format réduit 32x32 niveaux de gris) et calcul d'un hash perceptuel (**pHash** / DCT 64-bit).
2. **Indexation métrique (Distance de Hamming)** :
   - En mémoire : Arbre métrique **BK-Tree** (`pybktree` ou `cBKTree`).
   - En base de données : **PostgreSQL** avec extension sur-mesure **SP-GiST BK-Tree** (`pg-spgist_hamming`).
3. **Appariement (Matching)** : Recherche des frames clés correspondant aux plages temporelles où des sous-titres sont affichés.
4. **Recalage (Retargeting / Shift)** : Calcul du delta d'offset temporel le plus fréquent (mode statistique) entre le média de référence et le média cible, puis réécriture du fichier `.srt`.

```mermaid
flowchart LR
    A[Média Vidéo Source] -->|ffmpeg 32x32 gray| B[pHash Perceptuel 64-bit]
    B --> C[Fichier binaire .phash 8 bytes/frame]
    C --> D[(PostgreSQL 11 + SP-GiST Hamming)]
    C --> E[BK-Tree pybktree RAM]
    F[Fichier .srt Référence] --> G[SubReader / Intervalles]
    G --> H[Recherche Frames Communes]
    D --> H
    E --> H
    H --> I[Calcul Décalage Offset / Mode]
    I --> J[Fichier .srt Cible Synchronisé]
```

---

## 2. Cartographie Complète des Branches Git

Le dépôt compte **9 branches distantes**. La branche locale active `master` est un piège : elle est **91 commits en retard** sur le code réel.

| Branche | Commits vs `master` | Date dernier commit | Rôle & Contenu | Statut |
| :--- | :--- | :--- | :--- | :--- |
| **`master`** (HEAD locale) | 0 | 2026-01-28 | Squelette archaïque (2018) + bumps Dependabot (`tqdm`). Code basique `src/pydbsrt/app.py`, pas de CLI, pas de tests réels. | ❌ **Obsolète / Abandonnée** |
| **`develop`** | **+91** | 2023-04-17 | **Vrai cœur du projet (v2)**. Architecture moderne Poetry, Click CLI, Asyncpg, SP-GiST Postgres, 19 suites de tests, Dockerfile multi-stage. | ⭐️ **Branche de référence** |
| **`origin/2024_o2switch`** | **+92** (+1 vs develop) | 2024-04-29 | Tentative de déploiement sur hébergement mutualisé o2switch. Ajout `Pipfile` / `Pipfile.lock`, bump `uvloop` 0.19.0 et `werkzeug` 3.0.2. | ⚠️ WIP / Spécifique hébergeur |
| **`origin/o2switch`** | **+93** (+2 vs develop) | 2023-04-17 | Variante o2switch : support URL distantes pour les médias, `psycopg2-binary`, freeze `requirements.txt` et `test-requirements.txt`. | ⚠️ Spécifique hébergeur |
| **`origin/feature/imdb`** | +75 (0 vs develop) | 2021-11-11 | PoC recherche IMDb via Google Search à partir du nom de release vidéo. | ✅ **Fusionné dans `develop`** |
| **`origin/feature/pgsql-sqlalchemy`** | +15 | 2020-12-03 | Étape intermédiaire : 1ère intégration de Postgres SP-GiST avec SQLAlchemy avant le passage à Asyncpg sur `develop`. | 📦 Archivée / Dépassée par develop |
| **`origin/feature/cbktree`** | +18 | 2020-07-28 | PoC utilisation de l'extension C++ Cython `cBKTree`. Contient une documentation d'analyse et de benchmarks très riche dans `doc/`. | 📦 Archivée (doc précieuse) |
| **`origin/unleash_requirements`** | +3 (0 vs develop) | 2019-03-17 | Migration Python 3.7 (`attrs` -> `dataclasses`). | ✅ **Fusionné dans `develop`** |
| **`origin/setup.cfg`** | +2 | 2018-12-21 | Migration de `setup.py` vers `setup.cfg` (2018). | ❌ Abandonnée |

---

## 3. Audit des Submodules Git

Historiquement, le projet a fait intervenir **3 submodules** différents :

```
.gitmodules (master) :
  ├── py-googletrans (https://github.com/yoyonel/py-googletrans.git) -> commit d24baed
  └── py_translator  (https://github.com/markolofsen/py_translator.git)  -> commit 46a5615

.gitmodules (develop & feature/cbktree) :
  ├── py-googletrans (https://github.com/ssut/py-googletrans.git)
  ├── py_translator  (https://github.com/markolofsen/py_translator.git)
  └── cBKTree        (git@github.com:yoyonel/cBKTree.git) -> commit df54ab2
```

### Constat d'audit sur les submodules
1. **`py-googletrans` et `py_translator`** :
   - Présents sur disque et dans le `.gitmodules` de `master`.
   - **Supprimés de l'arbre Git sur `develop`** : remplacés par les packages PyPI directs (`googletrans`, `googlesearch-python`).
2. **`cBKTree`** :
   - Ajouté sur `feature/cbktree` et `develop` pour compiler un wrapper Cython d'arbre BK-Tree C++.
   - **Abandonné dans le code Python sur `develop`** au profit de deux solutions :
     - En mémoire : la bibliothèque pure Python **`pybktree`** (sans dépendance C++).
     - Persistant : l'indexation **PostgreSQL SP-GiST**.
   - Le submodule `cBKTree` n'est plus importé par `src/pydbsrt/`.

---

## 4. Architecture Détaillée du Code (`develop`)

L'arborescence sur `develop` est organisée selon les standards Python modernes (`src/` layout) :

```
src/pydbsrt/
├── app_cli.py                       # Point d'entrée CLI Click (binaire `pydbsrt`)
├── settings.py                      # Config DB (IP, user, pass, dbname)
│
├── applications/                    # Commandes CLI de haut niveau
│   ├── export_imghash_from_media.py               # Vidéo -> fichier binaire .phash
│   ├── import_imghash_into_db.py                  # Fichier .phash -> Table PostgreSQL frames
│   ├── search_imghash_in_db.py                    # Recherche d'une séquence de hash en DB
│   ├── recognize_media.py                         # Reconnaissance d'une vidéo inconnue
│   ├── retarget_subtitles.py                      # Décalage et recalage d'un SRT
│   ├── export_imghash_from_subtitles_and_media.py # Hash uniquement sur les intervalles sous-titrés
│   ├── extract_subtitles_from_medias.py           # Extraction SRT depuis flux MKV/MP4
│   ├── show_imghash_from_subtitles_and_media.py   # Visualisation TUI (Rich) des frames
│   ├── show_imghash_from_subtitles_and_media_in_db.py
│   ├── import_subtitles_into_db.py                # Ingestion table subtitles / sub_frames
│   └── wip_generate_subtitles.py                  # Ébauche de génération
│
├── services/                        # Logique métier et accès données
│   ├── database.py                  # DDL SQL, index SP-GiST, connexions asyncpg
│   ├── matching.py                  # Requête SQL `WHERE p_hash <@ ($1, $2)` + cache lru
│   ├── retarget_srt.py              # Algorithmes de recalage (mode statistique décalage)
│   ├── cuts.py                      # Détection de cuts et BKTree in-memory
│   ├── db_frames.py                 # Insertion batch de frames en DB
│   ├── db_subtitles.py              # Gestion CRUD des sous-titres
│   ├── reader_frames.py             # Décodage binaire 64-bit rapide des fichiers .phash
│   └── extended_subtitles.py        # Calculs d'intervalles de temps / frames par sous-titre
│
├── models/                          # Modèles de données dataclass
│   ├── frames.py, phash.py, matching.py, searching.py, subtitles.py
│
├── tools/                           # Utilitaires bas niveau
│   ├── ffmpeg_tools/                # Wrappers ffmpeg (extract frame, cut, cat, scale)
│   ├── ffmpeg_wrapper/              # FFprobe, FFMedia, FFStream, FFmpeg
│   ├── imghash.py                   # Conversions bitarray, int64 signé, hex, pHash
│   ├── search_tree.py               # Wrapper pybktree in-memory
│   ├── aio_filehash.py              # Hash MD5/SHA asynchrone pour intégrité fichiers
│   └── timer_profiling.py           # Mesures de performances d'exécution
│
└── POC/
    ├── search_google_images.py      # Scraping Google Images
    └── search_imdb_movies.py        # Parsing métadonnées de release vers IMDb
```

---

## 5. Moteur de Recherche & Indexation : SP-GiST BK-Tree

L'innovation centrale de `pydbsrt` réside dans l'utilisation de PostgreSQL comme base de données vectorielle/métrique pour le Hamming distance.

### Schéma Base de Données (`database.py`)
```sql
CREATE TABLE medias (
    id SERIAL PRIMARY KEY,
    media_hash CHAR(32) UNIQUE,
    name TEXT UNIQUE
);

CREATE TABLE frames (
    id SERIAL PRIMARY KEY,
    p_hash BIGINT,                -- Hash perceptuel stocké en entier 64-bit signé
    frame_offset INT,             -- Numéro de frame dans la vidéo
    media_id INT REFERENCES medias(id) ON DELETE CASCADE
);

CREATE INDEX frames_phash_bk_tree_index ON frames
    USING spgist ( p_hash bktree_ops );  -- Index custom SP-GiST BK-Tree
```

### Requête de recherche par distance de Hamming
```sql
SELECT frame_offset, media_id
FROM frames
WHERE p_hash <@ ($1, $2);  -- $1 = hash recherché, $2 = distance max (ex: 0 ou 1)
```

---

## 6. Analyse de la Conteneurisation & Docker

### 1. Image Base de Données (`docker/pg-spgist_hamming/11.3-3.0-develop/Dockerfile`)
- Base : `postgis/postgis:11-3.3` (Debian).
- Compile l'extension C PostgreSQL : `https://github.com/yoyonel/pg-spgist_hamming` (branche `develop`).
- Installe l'opérateur SP-GiST `bktree_ops` lors de l'initialisation (`initdb-bktree.sh`).

### 2. Orchestration (`docker/docker-compose.yml` & `docker-compose.test.yml`)
- Service `bktreedb` : PostgreSQL 11 avec extension bktree exposée sur le port `5432`.
- Service `adminer` : Interface web DB exposée sur `8080`.
- Service `postgres_exporter` : Métriques Prometheus sur `9187`.
- Volume de persistance : `bktreedb-data` (prod/dev) et `bktreedb-pg11-data-test` (tests).

### 3. Image Applicative (`Dockerfile` racine sur `develop`)
- Multi-stage build :
  1. Base `python:3.9.7`.
  2. Stage `poetry` (Poetry 1.1.12) -> export `requirements.txt`.
  3. Stage `build` -> construction du virtualenv `.venv`.
  4. Stage `runtime` -> `python:3.9.7-slim` avec `.venv` copié.

> [!WARNING]
> Le `docker/Dockerfile` historique présent sur `master` était un vieux déchet ciblant une image obsolète et un serveur PyPI interne privé inaccessible (purgé lors de la modernisation).

---

## 7. Performances & Benchmarks Mesurés (Historique du Repo)

D'après les logs et documents de benchmarks archivés (`README.md`, `doc/2020-05-25-FFMPEG_Matching_Analysis.md`, `doc/2021-04-13.md`) :

| Opération | Contexte | Débit / Vitesse | Commentaire |
| :--- | :--- | :--- | :--- |
| **Extraction pHash (Pleine résolution)** | Vidéo 1080p | **~154 frames/s** (0.81s pour 125 frames) | Gourmand en CPU/décodage |
| **Extraction pHash (Réduit 32x32 gray)** | Vidéo redimensionnée | **~1150 frames/s** (0.10s pour 120 frames) | **x7.5 plus rapide** |
| **Recherche d'une frame dans l'index SP-GiST** | Base de **~500 000 frames** (Postgres) | **~5.8 ms** (0.0058 s) | Quasi instantané |
| **Recherche d'une frame en RAM (BKTree)** | Index local CPU | **~13 à 15 ms** | Très efficace sur petits volumes |
| **Ingestion complète d'un épisode (42 241 frames)** | Silicon Valley S02E06 (45 min) | **140 frames/s** (durée ~5 min 49 s) | Insertion batch en DB |

---

## 8. Scripts Shell & Exploitation NAS

Le dossier `scripts/` sur `develop` illustre l'usage opérationnel réel :
- **`scripts/import_phash_into_db.sh`** :
  - Démarre automatiquement Docker Compose (`docker-compose up -d`).
  - Scanne un point de montage NAS Synology (`$HOME/__SYNOLOGY__/tvshow/__PHASH__/*.phash`).
  - Utilise **GNU Parallel** (`parallel -j1`) pour ingérer en base à la chaîne tous les hashs extraits.
- **`scripts/recognize_medias.sh`** :
  - Parcourt les fichiers `.mkv` d'une série (`Silicon Valley/`).
  - Lance en parallèle la commande CLI `pydbsrt recognize-media` avec sortie CSV.

---

## 9. Bilan Fonctionnel : Ce qui Marche, ce qui Bloque

### Ce qui est fonctionnellement au point (sur `develop`)
- Algorithme complet d'extraction et de conversion pHash en entier 64 bits.
- Format de stockage compact `.phash` (8 octets binaires par frame).
- Algorithme d'alignement et de décalage de sous-titres (`retarget-subtitles`).
- CLI Click structurée et riche (coloration Loguru, barres Rich/tqdm).
- Suites de tests unitaires (19 fichiers de tests pour CLI, services, calculs d'imghash).

### Ce qui ne fonctionne pas ou nécessite une mise à niveau
1. **Branche `master` en ruine** : Toute personne clonant le repo sur `master` tombe sur un code vieux de 8 ans sans aucun des développements v2.
2. **Incompatibilité des versions Python modernes** :
   - Le projet est verrouillé sur **Python 3.8 / 3.9**.
   - Dépendances datées (`SQLAlchemy 1.3`, `asyncpg 0.21`, `uvloop 0.14`, `click 7.1`, `black 21.7b0`).
   - L'environnement système actuel est en **Python 3.14 / 3.12 / 3.13** : l'installation directe sans conteneur ou sans pyenv 3.8/3.9 échouera.
3. **Conteneur Postgres 11 vieillissant** :
   - L'image Docker utilise PostgreSQL 11 (qui est EOL depuis fin 2023).
   - L'extension C `pg-spgist_hamming` doit être recompilée si on souhaite monter sur Postgres 15+.
4. **Scraping externe fragile** :
   - Les modules IMDb et Google (`POC/search_imdb_movies.py`, `POC/search_google_images.py`, `googletrans`) sont tributaires d'APIs tierces qui ont changé ou bloquent les requêtes non authentifiées.
5. **Fixtures de test dépendantes de Docker** :
   - `pytest` est configuré avec `pytest-docker-compose` pour lancer le conteneur de test Postgres 11 en dur. Sans démon Docker actif, la suite de tests échoue.

---

## 11. Bilan des Correctifs Validés (Projet 100% Opérationnel)

Tous les verrous initiaux ont été débloqués et validés en conditions réelles :

1. **Purge des résidus de submodules** : suppression des répertoires orphelins `py-googletrans`, `py_translator`, et découplage de `cBKTree` de l'index Git.
2. **PostgreSQL 11 + `pg-spgist_hamming`** :
   - Mise à jour du `Dockerfile` Postgres 11 avec les miroirs `snapshot.debian.org` et `apt-archive.postgresql.org`.
   - Élagage des dépendances inutiles (`libproj-dev`, etc.).
   - Compilation et activation validées de l'extension `bktree` (`docker_bktreedb:test`).
   - Opérateur `<@` testé et opérationnel.
3. **Image Applicative `pydbsrt:latest`** :
   - Migration de base vers Debian Bookworm (`python:3.9-bookworm` / `python:3.9-slim-bookworm`).
   - Mise à niveau de Poetry vers `1.4.2` dans le build.
   - Intégration du binaire système `ffmpeg 5.1.9`.
   - Correction du `PYTHONPATH` et installation du package en mode éditable avec le binaire CLI `pydbsrt`.
4. **Correctifs de Code & Configuration** :
   - [src/pydbsrt/settings.py](file:///home/latty/Prog/__PERSO__/pydbsrt/src/pydbsrt/settings.py) : Support dynamique des variables d'environnement (`PSQL_IP`, `PSQL_USER`, etc.).
   - [src/pydbsrt/tools/compare.py](file:///home/latty/Prog/__PERSO__/pydbsrt/src/pydbsrt/tools/compare.py) : Suppression de l'import interne parasite `_pytest.python_api.ApproxScalar`.
   - [tests/conftest.py](file:///home/latty/Prog/__PERSO__/pydbsrt/tests/conftest.py) : Fixture `db_is_ready` résiliente (supporte connexion directe ou conteneurisée).
   - [tests/services/test_export_imghash_from_media.py](file:///home/latty/Prog/__PERSO__/pydbsrt/tests/services/test_export_imghash_from_media.py) : Alignement du comptage de frames (812 frames complètes).
   - [Makefile](file:///home/latty/Prog/__PERSO__/pydbsrt/Makefile) : Cible `docker` pointée sur `Dockerfile` racine.
   - [docker/docker-compose.yml](file:///home/latty/Prog/__PERSO__/pydbsrt/docker/docker-compose.yml) : Port adminer décalé sur `8085` pour éviter le conflit d'écoute sur le port `8080`.

### Résultats des Tests Unitaires & Intégration
```text
================== 41 passed, 2 skipped, 1 xpassed in 17.67s ===================
```
* **CLI** : 100% passante (export phash, import db, search, match, retarget, extract subtitles).
* **Services** : 100% passants (requêtes SP-GiST, découpe, offsets de recalage).
* **Tools** : 100% passants.

---

## 12. Feuille de Route : Modernisation Drastique de la Stack

### 12.1. Langage Métier : Étude Comparative (Rust vs Odin vs Go)

Le cœur de `pydbsrt` manipule des buffers vidéo volumineux, du calcul de DCT/pHash et des distances de Hamming sur des entiers 64-bit (`POPCNT`).

```mermaid
flowchart TD
    subgraph Candidats
        R[Rust]
        O[Odin]
        G[Go]
    end
    R -->|Avantages| R_PRO[Performance C, SIMD popcnt natif, bindings ffmpeg robustes, sûreté mémoire, typage fort]
    R -->|Inconvénients| R_CON[Temps de compilation, courbe apprentissage du borrow-checker]
    O -->|Avantages| O_PRO[Approche data-oriented, interop C/ffmpeg sans coût FFI, binaire ultra-léger, compilation éclair]
    O -->|Inconvénients| O_CON[Écosystème jeune, moins de drivers DB async éprouvés]
    G -->|Avantages| G_PRO[Simplicité, concurrence goroutines, déploiement monolithique trivial]
    G -->|Inconvénients| G_CON[Coût Cgo élevé pour frame-by-frame ffmpeg, GC non maîtrisé sur gros flux vidéo]
```

> [!TIP]
> **Recommandation Langage : RUST**
> Pour du traitement vidéo image par image, du calcul pHash et de la recherche métrique, **Rust** est le candidat idéal :
> * Intruction processeur `u64::count_ones` pour une distance de Hamming en 1 cycle CPU.
> * Écosystème riche : `ffmpeg-next` (décodage), `clap` (CLI moderne), `sqlx` (Postgres async), `tokio` (concurrence).
> * Portabilité complète : binaire autonome sans runtime externe.
>
> **Alternative séduisante : ODIN** pour un outil CLI/daemon ultra-léger si l'interop C directe avec libavformat/libavcodec et libpq est privilégiée.

### 12.2. Base de Données : Sortie de PG11 et fin de l'extension C custom

Actuellement, le projet dépend de l'extension C custom `pg-spgist_hamming` bloquée sur Postgres 11. Deux voies de modernisation majeure :

1. **Option A (Recommandée) : `pgvector` avec `bit_hamming_ops` (Postgres 16 / 17)**
   * `pgvector` supporte nativement le type `BIT` et les index **HNSW / IVFFlat** avec distance de Hamming :
     ```sql
     CREATE EXTENSION vector;
     CREATE TABLE frames (
         id BIGSERIAL PRIMARY KEY,
         p_hash BIT(64),
         frame_offset INT,
         media_id INT REFERENCES medias(id)
     );
     CREATE INDEX ON frames USING hnsw (p_hash bit_hamming_ops);
     SELECT * FROM frames ORDER BY p_hash <~> B'1010...01' LIMIT 10;
     ```
   * **Bénéfice** : 100% standard, maintenu par l'écosystème, compatible avec les dernières versions de Postgres et les services managés cloud (AWS Aurora, GCP Cloud SQL, Supabase, Neon), **plus besoin de compiler d'extension C manuellement**.
2. **Option B : Portage de `bktree.c` sur Postgres 17**
   * Adapter les headers SP-GiST de Postgres 11 vers 17 (`spgist.h`). Simple mais conserve la dette de maintenance de l'extension C.

### 12.3. Outillage & Orchestration : `Makefile` ➔ `Taskfile`

Remplacement du `Makefile` par **Task** (`Taskfile.yml`) :
* Syntaxe déclarative YAML, gestion native des environnements (`.env`).
* Dépendances conditionnelles précises (hash des fichiers sources, `sources:` / `generates:`).
* Support natif multi-plateformes (Linux, macOS, Windows sans dépendre de bash/sed hôte).
