---
title: "Référence : Formats de Fichiers & Spécifications Binaires"
tags:

  - reference
  - formats
  - binaire
  - dbphash
  - json
---

# 📖 Référence : Formats de Fichiers & Spécifications Binaires

Cette page détaille la disposition binaire et le schéma des formats de données propriétaires et normalisés utilisés par `pydbsrt`.

---

## 1. Format Binaire Brut d'Empreinte (`.phash`)

Le format unitaire `.phash` stocke la séquence brute d'empreintes perceptuelles d'un média sans en-tête lourd :

- **Taille de l'empreinte** : 64 bits (8 octets non signés, `uint64_t`).
- **Endianness** : Little-endian standard IEEE.
- **Cadence implicite** : Définie par le nom de fichier ou la table `medias` (typiquement 25.0 fps ou 1 fps selon la phase).
- **Calcul du timestamp** : Pour la trame d'indice \( i \) (à partir de 0) :
  \( t_i = \frac{i}{\text{fps}} \text{ (en secondes)} \)

---

## 1bis. Format Binaire d'Empreinte Audio Spectrale (`.ahash` v1.0)

Le format unitaire `.ahash` stocke la séquence temporelle d'empreintes audio 64-bit calculées selon Haitsma-Kalker :

- **Taille de l'empreinte** : 64 bits (8 octets non signés, `uint64_t`).
- **En-tête structuré** : 176 octets garantissant un alignement strict à 8 octets pour projection mémoire `mmap`.
- **Magic** : `b"AHASH\0\1\0"`.
- **Spécification détaillée** : Voir la spécification complète dans [RFC Format Binaire `.ahash`](2026-11-10-rfc-ahash-format-audio.md).

### 1ter. Sidecar JSON d'un `.ahash` (`<stem>.json`, schéma v1)

Chaque `.ahash` écrit par `dbsrt export-ahash`, `dbsrt media:add --ahash-dir` ou rattrapé par `dbsrt ahash:sidecar` est accompagné d'un sidecar JSON (struct `AhashSidecar`, modules `dbsrt-core/src/ahash_sidecar.rs` et `dbsrt-core/src/ahash_backfill.rs`). Il documente le média source **et** la piste audio réellement décodée.

- **Nommage** : `<stem>.json` dans le même dossier que le `.ahash`, où `<stem>` est le nom de fichier sans `.ahash` ni `.<fps>fps` (symétrique à `__PHASH__/<moviehash>.json`) :
  - `05b6126884056c42.86.1fps.ahash` → `05b6126884056c42.json`
  - `Show.mkv.86.1fps.ahash` → `Show.mkv.json`
  - piste explicite non-défaut (`--audio-track 1`, chemin par défaut) : `Show.mkv.a1.86.1fps.ahash` → `Show.mkv.a1.json`
- **Écriture atomique** : `<stem>.json.tmp` + `fsync` + `rename` (jamais de JSON tronqué si le process est tué, ex. limite LVE o2switch).
- **Rattrapage** : quand `media:add` réutilise un `.ahash` existant sans sidecar, le sidecar est créé (lecture de l'en-tête 176 octets + un `ffprobe`, sans décodage audio ni FFT). Un sidecar existant n'est jamais écrasé par ce chemin.

| Champ | Type | Description |
| :--- | :--- | :--- |
| `schema_version` | `u32` | Version du schéma (`1`). |
| `moviehash` | `string \| null` | MovieHash OpenSubtitles 16-hex si connu. |
| `media_name` | `string` | Nom canonique du fichier média (clé partagée avec le sidecar pHash). |
| `ahash_file` | `string` | Nom du binaire `.ahash` décrit. |
| `fps` | `f64` | Cadence des sous-empreintes (`sample_rate / hop_size`, ~86.1). |
| `frames` | `usize` | Nombre de sous-empreintes. |
| `duration_sec` | `f64` | `frames / fps` (0 si `fps = 0`). |
| `header` | objet | Copie de l'en-tête binaire : `version`, `sample_rate`, `frame_size`, `hop_size`, `num_bands`, `spectral_lag`, `frame_count`. |
| `selection` | enum | Mode de sélection de la piste (tableau ci-dessous). |
| `selected_audio_stream` | `AudioStreamInfo \| null` | Piste décodée : `index` (absolu, `0:<index>`), `audio_index` (relatif, `0:a:<k>`), `codec_name`, `channels`, `sample_rate`, `language` (ISO 639-2, ex. `fre`/`jpn`/`eng`), `title`, `is_default`. |
| `container_audio_streams` | `AudioStreamInfo[]` | Toutes les pistes audio du conteneur (vide si inconnu). |
| `extracted_at` | `u64` | Horodatage Unix (s) de génération du sidecar. |
| `source_id` | `string` (opt.) | Identifiant logique de source média lors du backfill (jamais de secrets). |
| `source_rel_path` | `string` (opt.) | Chemin relatif dans la source média lors du backfill. |
| `verified_distance` | `f64` (opt.) | Distance de Hamming moyenne mesurée lors de la vérification multi-pistes. |

| `selection` | Condition | Certitude |
| :--- | :--- | :--- |
| `explicit` | `--audio-track <k>` → `-map 0:a:<k>` | Certaine. |
| `single_track` | Pas de `-map`, une seule piste audio | Certaine. |
| `verified_backfill` | Backfill multi-pistes confirmé par sondage 5s à 50% (`±2s`, distance `<= 14.0` bits) | Certaine (vérifiée). |
| `ffmpeg_default` | Pas de `-map`, plusieurs pistes | **Déduite** : 1ʳᵉ piste `is_default`, sinon la 1ʳᵉ ayant le plus de `channels`, sinon `0:a:0`. |
| `inferred_backfill` | Backfill multi-pistes avec `--no-verify` (règle par défaut FFmpeg) | **Déduite**. |
| `unknown` | `ffprobe` indisponible, aucune piste ou aucune piste `<= 14.0` bits lors du sondage | Inconnue. |

!!! note "Lecture par le catalogue"
    `AHashCatalog::load_from_dir` lit le sidecar typé en priorité (`media_name`, `audio_language`, `audio_index`). À défaut, il garde l'ancien repli nom seul (`<moviehash>.json` dans `__AHASH__/` puis `../__PHASH__/`, clés `media_name` / `clean_title` / `filename`).


---

## 2. Spécification de l'Archive Haute Densité (`.dbphash` v2)

Le format `.dbphash` regroupe plusieurs millions d'empreintes dans un fichier contigu unique projetable directement en mémoire via `mmap` sans allocation tas (`Zero-Allocation`).

### En-Tête Binaire (Header) — 64 octets

```text
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       Magic Bytes: 'D' 'B' 'P' 'H'            |  Version: 2   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| Flags (16-bit)|             Réservé (32-bit)                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Nombre total de Médias (uint64)               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Nombre total de Trames (uint64)               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Offset de la Table des Médias (uint64)        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Offset de la Table des Index MIH (uint64)     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Corps du Fichier
1. **Section Trames** : Tableau contigu de `u64` pHash ordonné par média.
2. **Table des Médias** : Pour chaque média :
   - `media_hash` : 16 octets (128 bits).
   - `start_frame_idx` : `uint64`.
   - `frame_count` : `uint32`.
   - `fps` : `float32`.
   - Longueur du titre (uint16) suivi de la chaîne UTF-8 du titre.
3. **Index MIH (Multi-Index Hashing)** : Découpage des 64 bits en 4 chunks de 16 bits pour un routage direct dans les tables de hachage inversées.

---

## 3. Schéma du Registre Vérité-Terrain (`couplings.json`)

Le fichier `data/SRT/couplings.json` garantit l'alignement certifié en mode Zero-DB (sans PostgreSQL) :

```json
{
  "version": "1.0",
  "generated_at": "2026-10-25T20:00:00Z",
  "couplings": [
    {
      "media_hash": "a1b2c3d4e5f60718293a4b5c6d7e8f90",
      "media_name": "Berserk.1997.EP01.MULTi.1080p.WEB.x264-FRATERNiTY",
      "fps": 25.0,
      "phash_path": "data/__PHASH__/Berserk.1997.EP01.phash",
      "subtitles": [
        {
          "subtitles_hash": "7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c",
          "filename": "Berserk.1997.EP01.fre.srt",
          "lang": "fr",
          "srt_path": "data/SRT/Berserk.1997.EP01.fre.srt",
          "certified": true,
          "cues_count": 242
        }
      ]
    }
  ]
}
```

---

## 4. Spécification des Fichiers de Sous-Titres Normalisés (`.srt`)

- **Jeu de caractères** : Strictement `UTF-8` sans BOM (Byte Order Mark). Les encodages legacy (CP1252, ISO-8859-1) sont automatiquement convertis lors de l'ingestion par la bibliothèque `chardetng` / `encoding_rs`.
- **Sauts de ligne** : Normalisés en fins de ligne Unix (`\n`, LF).

---

## 5. Fichiers de Contrôle d'Exécution & Sentinelles (`run/`)

Ces fichiers légers orchestrent dynamiquement l'exécution des processus d'ingestion et des daemons sans nécessiter de redémarrage.

### 5.1. Concurrence Dynamique : `run/desired_workers`
- **Format** : Fichier texte brut ASCII contenant un entier unique terminé par un retour chariot (ex. `2\n`).
- **Domaine de valeurs** : Entier compris entre `0` (mise en pause complète) et `8` (plafond maximal autorisé sous CloudLinux LVE).
- **Cycle de vie** :
  - **Initialisation** : Créé au lancement du worker batch avec la valeur par défaut `DEFAULT_WORKERS = 4`.
  - **Modification** : Altéré à chaud par l'opérateur via `task o2switch:batch:workers -- <N>` ou `task o2switch:batch:scale -- <N>`.
  - **Consommation** : Relu de manière non bloquante toutes les secondes (`TARGET_QUEUE_POLL_INTERVAL_SECS`) par la boucle d'orchestration `execute_dynamic_batch_ingestion`.
  - **Effet dynamique** : Si `desired < active`, le superviseur ne démarre aucun nouveau worker et attend la terminaison naturelle des tâches en vol sans émettre de `SIGKILL`. Si `desired > active`, de nouveaux sous-processus `dbsrt` sont immédiatement instanciés.

### 5.2. Sentinelle de Pause Douce : `run/batch_paused`
- **Format** : Fichier sentinelle vide (`0 octet`).
- **Cycle de vie** :
  - **Création** : Posé atomiquement par `task o2switch:batch:pause` (`touch run/batch_paused`).
  - **Détection** : Contrôlé à chaque itération de la boucle de dépilage.
  - **Effet dynamique** : Suspend immédiatement la prise en charge de nouvelles cibles vidéo. Les workers en cours d'exécution achèvent leur traitement vidéo en cours (zéro corruption, zéro troncature de fichier `.phash`).
  - **Reprise** : Supprimé par `task o2switch:batch:resume` (`rm -f run/batch_paused`), reprenant instantanément le dépilage des flux.

### 5.3. État Global de la File : `run/batch_queue.json`
- **Format** : Document JSON UTF-8 mis à jour atomiquement (écriture temporaire puis `os.replace`).
- **Champs & Schéma (vérifié sur données réelles de production)** :
  ```json
  {
    "total_targets": 401,
    "completed_targets": 12,
    "failed_targets": 0,
    "remaining_targets": 389,
    "progress_pct": 2.99,
    "active_workers": 2,
    "is_active": true,
    "updated_at": "2026-11-07T16:18:32Z"
  }
  ```
- **Cycle de vie** :
  - Généré dès l'évaluation du plan de cibles initial.
  - Réécrit à chaque début et fin d'ingestion de vidéo unitaire par un worker.
  - Consommé par les dashboards TUI (`dbsrt top`), les sondes de monitoring HTTP et la commande `task o2switch:batch:status`.

---

## 6. Journaux d'Événements & Télémétrie Cloud (`data/reports/` & `logs/`)

### 6.1. Journal d'Ingestion : `data/reports/batch_tvshow_nightly.jsonl`
- **Format** : Lignes JSON délimitées par `\n` (`JSON Lines`), écrites en mode append non bufferisé (`a+`).
- **Schéma Réel (extrait direct o2switch)** :
  ```json
  {
    "video_stem": "Cold.War (2018) [1080p](Pendant la guerre froide...)",
    "video_rel_path": "/video/Cold.War (2018) [1080p](...).mkv",
    "file_size_bytes": 7037755790,
    "wall_duration_secs": 893.91,
    "io_read_rate_mib_s": 7.51,
    "cifs_read_rate_mib_s": 7.51,
    "resumed_frames": 0,
    "start_iso": "2026-11-01T09:18:49Z",
    "end_iso": "2026-11-01T09:33:43Z",
    "timestamp_iso": "2026-11-01T09:33:43Z",
    "success": true,
    "status": "COMPLETED"
  }
  ```
- **Utilisation** :
  - **Reprise après crash** : Au démarrage, `select_targets` rejoue ce journal via `iter_events` pour exclure automatiquement les vidéos déjà marquées `success: true`.
  - **Blacklist automatique** : Les vidéos cumulant plus de 3 échecs consécutifs (`status != COMPLETED`) sont isolées via `load_blacklisted_stems` pour ne pas saturer la file.

### 6.2. Surveillance Quotas LVE CloudLinux : `logs/faultwatch/YYYY-MM-DD.jsonl`
- **Format** : Enregistrement minute par minute sous format JSONL horodaté UTC.
- **Schéma Réel (extrait direct o2switch)** :
  ```json
  {
    "minute": "2026-11-07T15:26Z",
    "n": 60,
    "cpu_mean": 0.1,
    "cpu_p90": 0.2,
    "cpu_max": 0.2,
    "secs_over_90": 0,
    "secs_over_100": 0,
    "run_delay_ms_mean": 0.0,
    "run_delay_ms_max": 0.0,
    "nproc_max": 7,
    "threads_max": 107,
    "ffmpeg_max": 0,
    "spawns": 12,
    "ffmpeg_spawns": 0,
    "dbsrt-server_spawns": 0
  }
  ```
- **Cycle de vie** :
  - Produit en continu par le daemon de surveillance `scripts/workers/o2switch_fault_watch.py` tournant sous `screen`.
  - Échantillonne 60 fois par minute l'état des processus et de l'ordonnanceur.
  - Consommé par `task o2switch:faultwatch:status` et `task o2switch:faultwatch:correlate` pour prouver l'absence d'impact LVE CPU et I/O.

---

## 7. Matrice des Cycles de Vie des Ressources

| Ressource | Phase Initiale | En Cours de Processing | Pause Douce (`task ...:pause`) | Redimensionnement (`scale N`) | Clôture / Terminaison |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`desired_workers`** | Créé à 4 | Lu à chaque boucle (1s) | Préservé inchangé | Mis à jour atomiquement | Préservé |
| **`batch_paused`** | Absent | Absent | Créé (0 octet) | Inchangé | Supprimé |
| **`batch_queue.json`** | Initialisé (`is_active: true`) | Réécrit à chaque vidéo | Mis à jour (`active_workers: 0` à terme) | Ajusté avec la nouvelle cible | `is_active: false` |
| **`*.phash` temporaires** | Absents | Fichier ouvert en écriture séquentielle | Écriture menée à son terme | Pas d'interruption violente | Fermé et vérifié bit-à-bit |
| **`events.jsonl`** | Chargé pour filtrage | Écrit à la complétion de chaque tâche | Aucune écriture (attente) | Écritures au fil de l'eau | Enregistrement final |
| **`faultwatch/*.jsonl`** | Rotation journalière | 1 ligne JSONL / minute | Poursuite de la mesure (repos) | Poursuite de la mesure | Poursuite continue H24 |
- **Format temporel** : `HH:MM:SS,mmm --> HH:MM:SS,mmm` (virgule décimale pour les millisecondes).
