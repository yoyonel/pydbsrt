---
title: "Référence : Constantes Nommées du Codebase"
tags:
  - reference
  - architecture
  - qualite
---

# 📖 Référence : Constantes Nommées du Codebase

Ce document recense les constantes opérationnelles, temporelles et algorithmiques du projet **pydbsrt**.

> [!IMPORTANT]
> **Règle Architecturale « Zéro Constante Magique »** :
> Aucun littéral numérique ou chaîne magique non nommée n'est toléré dans la logique métier, les parsers, les calculs temporels ou les endpoints HTTP. Chaque valeur fait l'objet d'une constante typée, documentée et immuable (`pub const` en Rust, `UPPER_SNAKE_CASE` en Python).

---

## 1. Moteur Vidéo & Extraction (`crates/dbsrt-core/src/video.rs`)

| Constante | Type & Valeur | Localisation | Rôle & Justification |
| :--- | :--- | :--- | :--- |
| `FFPROBE_TIMEOUT_SECS` | `u64 = 60` | `crates/dbsrt-core/src/video.rs` | Timeout d'analyse `ffprobe` tolérant les latences élevées sur stockages distants (CIFS/NFS). |
| `CHILD_POLL_INTERVAL` | `Duration = 50ms` | `crates/dbsrt-core/src/video.rs` | Période de scrutation (`try_wait`) pour surveiller l'avancement des sous-processus sans bloquer la boucle asynchrone. |
| `DEFAULT_BASELINE_FPS` | `f64 = 24.0` | `crates/dbsrt-core/src/video.rs` | Cadence de repli nominale quand les métadonnées conteneur ne spécifient aucun framerate valide. |
| `PAL_FPS` | `f64 = 25.0` | `crates/dbsrt-core/src/video.rs` | Norme de diffusion broadcast européenne (PAL / SECAM). |
| `NTSC_FILM_FPS` | `f64 = 23.976` | `crates/dbsrt-core/src/video.rs` | Norme cinéma américaine NTSC avec pull-down 3:2. |

---

## 2. Découpage Scènes & Hashing Visuel (`crates/dbsrt-core/src/cuts.rs`)

| Constante | Type & Valeur | Localisation | Rôle & Justification |
| :--- | :--- | :--- | :--- |
| `DEFAULT_THRESHOLD_DISTANCE_FOR_CUT` | `u32 = 16` | `crates/dbsrt-core/src/cuts.rs` | Distance de Hamming minimale entre trames successives qualifiant une coupure franche de plan vidéo. |
| `DEFAULT_SEARCH_DISTANCE_FOR_BKTREE` | `u32 = 0` | `crates/dbsrt-core/src/cuts.rs` | Rayon de recherche exact par défaut lors du parcours dans l'arbre métrique BK-Tree. |
| `FLAT_WHITE_PHASH` | `u64 = 0x8669_3900_3986_c639` | `crates/dbsrt-core/src/cuts.rs` | Empreinte pHash normalisée d'une trame purement blanche (rejetée lors de l'indexation). |
| `FLAT_BLACK_PHASH` | `u64 = 0x0000_0000_0000_0000` | `crates/dbsrt-core/src/cuts.rs` | Empreinte pHash normalisée d'une trame purement noire (trame d'amorce rejetée). |

---

## 3. Parsing & Arithmétique Temporelle Sous-Titres (`crates/dbsrt-core/src/srt.rs`)

| Constante | Type & Valeur | Localisation | Rôle & Justification |
| :--- | :--- | :--- | :--- |
| `MILLIS_PER_HOUR` | `i64 = 3_600_000` | `crates/dbsrt-core/src/srt.rs` | Facteur de conversion des heures en millisecondes pour les timecodes SRT / ASS. |
| `MILLIS_PER_MINUTE` | `i64 = 60_000` | `crates/dbsrt-core/src/srt.rs` | Facteur de conversion des minutes en millisecondes. |
| `MILLIS_PER_SECOND` | `i64 = 1_000` | `crates/dbsrt-core/src/srt.rs` | Facteur de conversion des secondes en millisecondes. |
| `MILLIS_PER_CENTISECOND` | `i64 = 10` | `crates/dbsrt-core/src/srt.rs` | Facteur de conversion des centièmes de seconde (format ASS `H:MM:SS.cc`). |
| `SECONDS_PER_MINUTE` | `i64 = 60` | `crates/dbsrt-core/src/srt.rs` | Facteur temporel standard. |
| `SECONDS_PER_HOUR` | `i64 = 3_600` | `crates/dbsrt-core/src/srt.rs` | Facteur temporel standard. |
| `ASS_DIALOGUE_FIELD_COUNT` | `usize = 10` | `crates/dbsrt-core/src/srt.rs` | Nombre de colonnes standard dans une entrée `Dialogue:` Advanced SubStation Alpha. |
| `ASS_START_TIME_FIELD_INDEX` | `usize = 1` | `crates/dbsrt-core/src/srt.rs` | Indice de colonne du timecode de début dans les dialogues ASS. |
| `ASS_END_TIME_FIELD_INDEX` | `usize = 2` | `crates/dbsrt-core/src/srt.rs` | Indice de colonne du timecode de fin dans les dialogues ASS. |
| `ASS_TEXT_FIELD_INDEX` | `usize = 9` | `crates/dbsrt-core/src/srt.rs` | Indice de colonne du texte du sous-titre dans les dialogues ASS. |

---

## 4. Appairage Fichiers Média & Sous-Titres (`crates/dbsrt-core/src/media_pairing.rs`)

| Constante | Type & Valeur | Localisation | Rôle & Justification |
| :--- | :--- | :--- | :--- |
| `SUPPORTED_VIDEO_EXTENSIONS` | `&[&str]` | `crates/dbsrt-core/src/media_pairing.rs` | Liste blanche des extensions conteneurs supportées (`mkv`, `mp4`, `webm`, `avi`, `mov`, `m4v`). |
| `SUBTITLE_SUBDIRS` | `&[&str]` | `crates/dbsrt-core/src/media_pairing.rs` | Répertoires conventionnels explorés pour auto-découvrir les sous-titres (`subs`, `subtitles`). |
| `SUBTITLE_EXTENSION` | `&str = "srt"` | `crates/dbsrt-core/src/media_pairing.rs` | Extension canonique des sous-titres cibles. |

---

## 5. Indexation Métrique & Base de Données (`crates/dbsrt-index/`)

| Constante | Type & Valeur | Localisation | Rôle & Justification |
| :--- | :--- | :--- | :--- |
| `DBPHASH_MAGIC` | `[u8; 8] = *b"DBSRT\x02\x00\x00"` | `crates/dbsrt-index/src/dbphash.rs` | Magic number d'en-tête du format binaire compact `.dbphash` (v2). |
| `DBPHASH_VERSION` | `u16 = 2` | `crates/dbsrt-index/src/dbphash.rs` | Version active du schéma binaire compact. |
| `NUM_TABLES` | `usize = 4` | `crates/dbsrt-index/src/mih.rs` | Nombre de sous-tables de hachage pour le Multi-Index Hashing (4 morceaux de 16 bits). |
| `BUCKETS_PER_TABLE` | `usize = 65_536` | `crates/dbsrt-index/src/mih.rs` | Capacité d'adressage direct de chaque sous-table MIH (\(2^{16}\) entrées). |
| `COPY_BUFFER_INITIAL_CAPACITY` | `usize = 64 * 1024` | `crates/dbsrt-index/src/db.rs` | Taille mémoire initiale du buffer d'ingestion streaming PostgreSQL `COPY`. |
| `COPY_BUFFER_FLUSH_THRESHOLD` | `usize = 32 * 1024` | `crates/dbsrt-index/src/db.rs` | Seuil de purge automatique vers la socket PostgreSQL. |
| `DB_CONNECT_TIMEOUT_SECONDS` | `u64 = 2` | `crates/dbsrt-index/src/db.rs` | Timeout de connexion TCP à la base PostgreSQL 17. |
| `DEFAULT_POSTGRES_PORT_CUSTOM` | `u16 = 5433` | `crates/dbsrt-index/src/db.rs` | Port d'écoute du conteneur PostgreSQL BK-Tree isolé. |
| `DEFAULT_MIN_RECOGNITION_CONFIDENCE` | `f64 = 80.0` | `crates/dbsrt-index/src/engine.rs` | Seuil de confiance minimum pour valider une reconnaissance vidéo automatique. |

---

## 6. Démon Serveur & API REST (`crates/dbsrt-server/src/constants.rs`)

| Constante | Type & Valeur | Localisation | Rôle & Justification |
| :--- | :--- | :--- | :--- |
| `DEFAULT_RECOGNITION_SEARCH_DISTANCE` | `u32 = 6` | `crates/dbsrt-server/src/constants.rs` | Distance de Hamming tolérée pour le matching initial de trame (environ 90,6% de similarité). |
| `MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT` | `f64 = 75.0` | `crates/dbsrt-server/src/constants.rs` | Seuil de confiance minimal requis pour accepter une concordance globale de média. |
| `LOG_BUFFER_CAPACITY` | `usize = 300` | `crates/dbsrt-server/src/constants.rs` | Capacité circulaire du ring-buffer de logs accessible via `/v1/logs`. |
| `DEFAULT_RELAY_TIMEOUT_MS` | `u64 = 3_500` | `crates/dbsrt-server/src/constants.rs` | Timeout maximal lors de la délégation de requêtes vers un nœud upstream o2switch. |
| `DEFAULT_RELAY_CONNECT_TIMEOUT_MS` | `u64 = 2_000` | `crates/dbsrt-server/src/constants.rs` | Timeout d'établissement TCP vers les serveurs relais. |
| `PHASH_BIT_DEPTH` | `f64 = 64.0` | `crates/dbsrt-server/src/constants.rs` | Longueur binaire de l'empreinte perceptuelle dbsrt (64 bits = \(8 \times 8\) DCT). |
| `SIGNIFICANT_SHIFT_THRESHOLD_MS` | `f64 = 500.0` | `crates/dbsrt-server/src/constants.rs` | Décalage temporel minimum nécessitant la réécriture d'un sous-titre `.dbsrt.srt`. |

---

## 7. Scripts d'Orchestration & Validation (`scripts/`)

| Constante | Type & Valeur | Fichier Source | Rôle & Justification |
| :--- | :--- | :--- | :--- |
| `KEYSTROKE_DELAY_SEC` | `0.035` | `scripts/record_casts.py` | Intervalle simulé entre deux frappes clavier pour un rendu fluide dans Asciinema. |
| `FINAL_PAUSE_SEC` | `6.0` | `scripts/record_casts.py` | Pause terminale avant rebouclage pour permettre la lecture des résultats CLI. |
| `CHROME_TIMEOUT_SEC` | `25` | `scripts/verify_terminal_rendering.py` | Timeout de sécurité lors du pilotage de Chrome Headless pour les captures. |
| `CHROME_WINDOW_WIDTH` | `1920` | `scripts/verify_terminal_rendering.py` | Largeur d'affichage standard Full HD pour les captures d'interface. |
| `CHROME_WINDOW_HEIGHT` | `1400` | `scripts/verify_terminal_rendering.py` | Hauteur d'affichage adaptée englobant le terminal complet. |
| `DEFAULT_PARALLEL_WORKERS` | `8` | `scripts/batch_media_add_tvshow.py` | Nombre de processus parallèles d'ingestion CIFS. |

---

## Liens Utiles du Quadrant Référence

- 📖 [Référence CLI Standalone](cli.md) — Documentation exhaustive des sous-commandes et options.
- 📖 [Référence Taskfile](taskfile.md) — Spécification des tâches déclaratives d'orchestration.
- 📖 [Schéma PostgreSQL 17](schema-postgresql.md) — Spécification relationnelle des tables et index SP-GiST.
- 📖 [Spécification des Formats de Fichiers](formats-fichiers.md) — Formats binaires `.phash`, `.dbphash` et manifestes `.json`.
