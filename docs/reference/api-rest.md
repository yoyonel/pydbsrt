---
title: "Référence : API REST Axum (dbsrt-server)"
tags:

  - reference
  - api
  - rest
  - axum
---

# 📖 Référence : API REST Axum (`dbsrt-server`)

Cette page fournit les spécifications techniques de l'API HTTP exposée par le microservice `dbsrt-server` (par défaut sur `http://127.0.0.1:8765`).

---

## 1. Principes Généraux

- **Format des échanges** : JSON (`Content-Type: application/json; charset=utf-8`).
- **Encodage binaire** : Les séquences d'empreintes pHash sont transmises soit sous forme de tableau d'entiers 64-bit non-signés (`u64`), soit en chaîne hexadécimale, soit en base64 pour les gros volumes.
- **Cross-Origin Resource Sharing (CORS)** : Toutes les origines sont admises en mode local.
- **Port d'écoute** : Défini par l'option `--port` ou la variable `DBSRT_PORT` (défaut : `8765`).
- **Authentification & Sécurité (Passerelle Cloud o2switch)** :
  - **En-têtes acceptés** : `X-API-Key: <token>` ou `Authorization: Bearer <token>`.
  - **Jeton URL direct** : `?token=<token>` ou `?key=<token>` (idéal pour liens magiques et scan QR Code mobile).
  - **Cookie de session** : `dbsrt_session` (`HttpOnly; Secure; SameSite=Lax`), durée de validité de **30 jours**.
  - **Durée de validité de la clé secrète** : Illimitée (persistée dans `etc/api_key.secret`), rotation via `task o2switch:rotate-key`.
  - **Clé lecture seule** (`etc/api_key_readonly.secret`, rotation via `task o2switch:rotate-readonly-key`) : limitée à `GET`/`HEAD` sur `/monitoring`, `/top`, `/health`, `/v1/health` et `/v1/monitoring/*`. Toute autre requête renvoie `403 Forbidden`.
  - **Protection anti-bruteforce** : Verrouillage 15 minutes (HTTP 429) après 5 échecs consécutifs par IP.
- **Authentification du démon** (`--api-key-file` ou `DBSRT_API_KEY_FILE`) :
  - Sans fichier de clés, l'API reste ouverte (usage local).
  - Avec fichier : toutes les routes exigent `Authorization: Bearer <clé>` ou `X-API-Key: <clé>`, sauf `/health` et `/v1/health`.
  - Format du fichier : une clé par ligne, lignes vides et `#` ignorées. Relu à chaque requête : une rotation s'applique sans redémarrage.
  - Codes : `401 Unauthorized` (clé absente ou invalide), `503 Service Unavailable` (fichier illisible ou absent).
  - Sur o2switch, la passerelle PHP retire les en-têtes `Authorization`, `X-API-Key` et `Cookie` du client et injecte la clé maître. La clé lecture seule n'est donc jamais acceptée directement par le démon.

---

## 2. Endpoints de Santé & Télémétrie

### `GET /health` & `GET /v1/health`
Sonde de vivacité (Liveness probe) pour reverse-proxies et orchestrateurs conteneurs.
**Réponse** :
  ```json
  {"status": "ok", "version": "0.2.0"}
  ```

### `GET /v1/status`
Rapport de santé complet du démon, état de la connexion PostgreSQL 17 et décompte des médias.
**Réponse** :
  ```json
  {
    "status": "healthy",
    "uptime_seconds": 3600,
    "database": {
      "connected": true,
      "medias_count": 21,
      "frames_count": 52400,
      "subtitles_count": 21
    },
    "memory_rss_bytes": 48234496
  }
  ```

### `GET /v1/metrics`
Exposition des métriques au format Prometheus (`text/plain; version=0.0.4`).

### `GET /v1/logs`
Retourne les dernières lignes du journal d'exécution en mémoire vive du serveur.

### `GET /monitoring` & `GET /top`
Tableau de bord web interactif monopage (HTML5/CSS/JS), responsive pour smartphones (4G/5G) et ordinateurs. Affiche la progression globale de la file batch, les workers actifs, la télémétrie du superviseur RPi4 et le journal en direct.

### `GET /v1/monitoring/telemetry`
Snapshot unifié en JSON de la télémétrie du serveur (statistiques système, file batch, workers d'extraction actifs, télémétrie et logs du superviseur RPi4, derniers événements terminés).

- **En-têtes de réponse** : `X-Server-Time-Ms: <epoch_ms>` pour la calibration de l'horloge OS (algorithme de Cristian côté client).

### `GET /v1/monitoring/top`
Snapshot JSON de `dbsrt top --json` (binaire `dbsrt` voisin de `dbsrt-server`), destiné aux clients web de supervision (lecture seule).

- **En-têtes de réponse** : `X-Server-Time-Ms: <epoch_ms>` pour la synchronisation d'horloge et l'évaluation du RTT.

**Architecture de Cache Non-Bloquant (Stale-While-Revalidate)** :

- **TTL de réutilisation** : 15 s (`TOP_SNAPSHOT_TTL`).
- **Comportement à l'expiration** : Si un snapshot précédent existe en mémoire, il est renvoyé **immédiatement (0 ms de temps serveur)** au client HTTP sans le bloquer.
- **Réévaluation en tâche de fond** : Une tâche Tokio détachée (`spawn_top_probe_background`) exécute `dbsrt top --json --refresh-ms 200` en arrière-plan pour actualiser le cache silencieusement.
- **Protection contre l'avalanche** : Drapeau atomique `TOP_PROBE_IN_FLIGHT` garantissant qu'un seul sous-processus d'échantillonnage ne s'exécute à la fois.

Les listes lourdes `history_records` et `discovered_media` sont paginées en mémoire vive :

| Paramètre | Défaut | Rôle |
|---|---|---|
| `offset` | `0` | Éléments sautés au début de chaque liste. |
| `limit` | `50` | Éléments gardés par liste ; `0` ne garde que les champs live et les totaux. |

La réponse ajoute `history_total` et `discovered_media_total` (taille de chaque liste avant découpe), et `discovered_media_per_source` (nombre de médias par `source_id`).

Ordre avant découpe, comme les onglets de `dbsrt top` : `history_records` du plus récent au plus ancien, `discovered_media` par nom (insensible à la casse).

Secrets masqués : la valeur suivant `-ftp-password` et le mot de passe des URL `scheme://user:pass@host` deviennent `***` (lignes de commande des jobs, corps et `stderr`).

- `200 OK` : snapshot JSON compact (quelques Ko avec `limit=0`, environ 1 Mo non paginé).
- `502 Bad Gateway` : `dbsrt top` a terminé en erreur ; le corps contient son `stderr`.
- `500 Internal Server Error` : binaire `dbsrt` introuvable ou non exécutable.

### `GET /v1/monitoring/events`
Flux continu Server-Sent Events (SSE, `text/event-stream`) poussant l'état consolidé (`state`) en temps réel toutes les 2 secondes avec pings keep-alive toutes les 15 secondes.

- **Authentification** : en-têtes HTTP (`Authorization: Bearer <clé>`, `X-API-Key`) ou paramètres d'URL query `?token=<clé>` / `?key=<clé>` (requis pour l'`EventSource` natif du navigateur).
- **Structure de l'événement** :
  - `event: state`
  - `data: {"telemetry": {...}, "top": {...}, "pipeline": {...}}`
    - `telemetry` : [`MonitoringTelemetryResponse`](#get-v1monitoringtelemetry) (système, file batch, workers, superviseur, audit).
    - `top` : [`dbsrt top` snapshot](#get-v1monitoringtop) paginé en `limit=0` (métriques chaudes uniquement, zéro surcoût de liste catalogue).
    - `pipeline` : chronométrie des 6 étapes du dernier matching exécuté (FFmpeg, WAN, MIH, SRT).
- `200 OK` : flux persistant HTTP/1.1 ou HTTP/2 streaming.

### `POST /v1/supervisor/heartbeat`
Réceptionne et persiste de manière atomique la pulsation et les logs de l'agent superviseur RPi4 dans `run/supervisor_status.json`. Les secrets FTP des logs sont masqués à l'écriture et à la lecture.

---

## 3. Endpoints de Recherche & Synchronisation

### `POST /v1/search`
Recherche de sous-titres et reconnaissance multimodale (pHash visuel et/ou aHash acoustique selon `search_mode`: `"all"` / `"auto"`, `"phash"`, `"ahash"`). En mode agent relais (`--agent <REMOTE_URL>`), lorsque `search_mode = "ahash"` ou en repli automatique `"all"`/`"auto"`, le démon local extrait l'empreinte audio, relaie la charge utile binaire Base64 (`ahash_bytes`) vers `POST {remote_url}/v1/search/ahash` et résout les sous-titres correspondants.

**Corps de la requête** :
  ```json
  {
    "media_path": "/data/videos/film.mkv",
    "language": "fr",
    "search_mode": "ahash"
  }
  ```
**Réponse** :
  ```json
  {
    "media_path": "/data/videos/film.mkv",
    "language": "fr",
    "matched_media": "Berserk.1997.EP01",
    "moviehash": "8e245d9679d31e12",
    "total_candidates": 1,
    "candidates": [
      {
        "source": "catalog",
        "language": "fr",
        "title": "Berserk.1997.EP01 [FR]",
        "candidate_id": "Berserk.1997.EP01.fr.srt",
        "shift_ms": 697.0
      }
    ],
    "timing": {
      "matching_tier": "Tier 1 (RAM .ahash Cross-Correlation)",
      "confidence_percent": 96.2
    }
  }
  ```

### `POST /v1/search/ahash`
Recherche acoustique par cross-corrélation temporelle d'empreintes sub-fingerprint Haitsma-Kalker (`.ahash` v1.0) indexées en mémoire (`MIH-32` + repli linéaire borné à `MAX_LINEAR_FALLBACK_FRAMES = 500_000` trames, exécuté dans `tokio::task::spawn_blocking` sous sémaphore de tâches lourdes).
Supporte nativement la fenêtre optimale empirique ($W = 5.0\text{ s}$, 430 trames à `86.125 fps`) ainsi que 3 formats d'encodage du flux d'entrée (`hashes`, `fingerprint` hex ou `ahash_bytes` base64).

**Garde-fous et validation d'entrée (`400 Bad Request` / `413 Payload Too Large`)** :

* **Taille maximale du corps HTTP** : `256 KiB` (`MAX_AHASH_BODY_BYTES = 262_144` octets via `DefaultBodyLimit::max`).
* **Nombre de trames de requête** : borné dans `8..=8192` trames (`MIN_QUERY_FRAMES = 8`, `MAX_AHASH_QUERY_FRAMES = 8_192`, soit ~95 s à `86.125 fps`).
* **Alignement Base64 (`ahash_bytes`)** : la charge décodée doit être non vide et multiple exact de `8` octets (`u64` little-endian).
* **`audio_fps`** : flottant fini strictement positif (`> 0.0`, défaut : `86.125`).
* **`max_distance`** : flottant fini compris dans `0.0..=64.0` (défaut : `14.0`).
* **`limit`** : entier compris dans `1..=50` (`MAX_AHASH_SEARCH_LIMIT = 50`, défaut : `5`).
* **Garde anti-silence / plat** : les requêtes composées de trames nulles (`0x0000000000000000`), saturées (`0xFFFFFFFFFFFFFFFF`) ou à entropie spectrale plate sont rejetées sans faux positif.

**Corps de la requête** :
  ```json
  {
    "hashes": [11649234873721389056, 11649234873721389057],
    "audio_fps": 86.125,
    "max_distance": 14.0,
    "limit": 5,
    "media_name": "Berserk.1997.EP01"
  }
  ```
* `hashes` : tableau de `8..=8192` entiers 64 bits non signés (alternative : `fingerprint` avec chaînes hexadécimales `"0x..."`, ou `ahash_bytes` avec charge binaire little-endian encodée en base64).
* `audio_fps` : cadence d'échantillonnage de l'empreinte requête (défaut : `86.125` fps).
* `max_distance` : seuil moyen maximal de bits erronés par trame sur 64 bits dans `0.0..=64.0` (défaut : `14.0`).
* `limit` : nombre maximal de candidats retournés dans `1..=50` (défaut : `5`).
* `media_name` : filtrage optionnel pour cibler un média de référence précis.

**Réponse** :
  ```json
  {
    "success": true,
    "best_match": {
      "media_name": "Berserk.1997.EP01",
      "file_path": "data/__AHASH__/8e245d9679d31e12.86.1fps.ahash",
      "ref_frame_offset": 60,
      "offset_sec": 0.69666,
      "offset_ms": 697,
      "mean_distance": 2.45,
      "score": 0.9617,
      "snr_bits": 29.55,
      "audio_fps": 86.125
    },
    "candidates": [
      {
        "media_name": "Berserk.1997.EP01",
        "file_path": "data/__AHASH__/8e245d9679d31e12.86.1fps.ahash",
        "ref_frame_offset": 60,
        "offset_sec": 0.69666,
        "offset_ms": 697,
        "mean_distance": 2.45,
        "score": 0.9617,
        "snr_bits": 29.55,
        "audio_fps": 86.125
      }
    ],
    "total_catalog_files": 583,
    "total_catalog_frames": 106093804,
    "elapsed_seconds": 0.0142
  }
  ```

### `POST /v1/autosync`
Endpoint combiné utilisé par les plugins VLC (`vlsub_dbsrt.lua` / `dbsrt_intf.lua`) : analyse les empreintes extraites du média en cours (`search_mode`: `"phash"`, `"ahash"` ou `"auto"`), calcule le décalage consensus (\( \Delta t \)), et écrit le sous-titre recalé en une seule passe.

* En mode acoustique (`search_mode = "ahash"` ou repli `"auto"`), extrait `5.0 s` d'audio (ou lit le sidecar `.ahash`), interroge `AHashCatalog` (ou relaie `ahash_bytes` vers le serveur maître en mode `--agent`), et calcule `shift_ms = (query_start_sec - ref_offset_sec) * 1000.0`.
* **Garde d'intégrité de source** : `shift_ms` n'est appliqué que lorsque le sous-titre provient du catalogue certifié (`source == "catalog"` / `"certified"`). Si le sous-titre est extrait du conteneur cible (`source == "container"`) ou téléchargé depuis un provider tiers non certifié (`source == "opensubtitles"`), le décalage acoustique/visuel du média de référence est forcé à `0.0 ms` pour empêcher toute désynchronisation d'une piste déjà alignée.

**Corps de la requête** :
  ```json
  {
    "media_path": "/data/videos/film.mkv",
    "language": "fr",
    "search_mode": "ahash"
  }
  ```
**Réponse** :
  ```json
  {
    "success": true,
    "matched_media": "Apprendre_CakePHP_-_SessionHelper",
    "language": "fr",
    "shift_frames": 30,
    "shift_ms": 1250.0,
    "output_srt_path": "/data/videos/film.fr.dbsrt.srt",
    "confidence_percent": 99.2,
    "elapsed_seconds": 0.14,
    "source": "catalog",
    "error": null
  }
  ```

### `POST /v1/recognize`
Reconnaissance visuelle d'un média à partir d'empreintes pHash déjà extraites (`query_fingerprint`) ou d'un fichier local (`media_path`). Utilisé par l'agent local en mode relais vers le serveur distant. Protégé par `X-API-Key` quand le serveur est lancé avec `--api-key`.
**Corps de la requête** :
  ```json
  {
    "query_fingerprint": ["0x1234567890abcdef", "0xfedcba0987654321"],
    "fps": 25.0,
    "language": "fr",
    "search_distance": 6
  }
  ```
**Réponse** :
  ```json
  {
    "found": true,
    "media_name_search": "fingerprint_query",
    "media_match": "Andor",
    "season": 1,
    "episode": 1,
    "nb_offsets_match": 240,
    "confidence": 100.0,
    "tier": "Tier 1 (RAM .dbphash MIH)",
    "shift_frames": -29175,
    "shift_ms": -1167000.0,
    "candidates": [],
    "timer_seconds": 0.18
  }
  ```
* `media_match` : titre nettoyé (Tier 1) ou nom de release complet (Tier 2/3). Seul, il est partagé par tous les épisodes d'une série.
* `season` / `episode` : épisode reconnu, absents si inconnus (films, noms non parsables). Les `candidates` ne contiennent que les sous-titres certifiés de cet épisode, et le décalage est calculé sur son seul `.phash`.
* À confiance égale, le média retenu est celui de plus faible distance de Hamming cohérente, puis par titre, saison et épisode (ordre déterministe).

### `GET /v1/subtitles/{hash}`
Télécharge le contenu du fichier SRT correspondant au hash demandé.
**Paramètre de requête** : `?shift_ms=<INT>` : applique à la volée le décalage temporel spécifié à toutes les balises de timing du fichier SRT retourné.

---

## 4. Endpoints de Gestion du Catalogue & Vérité Terrain

### `GET /v1/couplings`
Retourne la liste complète des médias certifiés et de leurs sous-titres associés.

### `GET /v1/couplings/sub_frames`
Retourne les intervalles d'ancrage temporels trames clés \( \leftrightarrow \) répliques de sous-titres d'un média donné (`?media_id=<INT>`).

### `POST /v1/certify`
Enregistre un nouveau couplage certifié dans la base de données.

### `POST /v1/catalog/reload` (alias `/v1/reload`)
Déclenche le rechargement à chaud atomique (zéro coupure de service) des index visuels (`.dbphash` + Delta) et du catalogue acoustique (`AHashCatalog` sous `data/__AHASH__/`) en mémoire vive.
**Réponse** :
  ```json
  {
    "success": true,
    "elapsed_ms": 3.4,
    "indexed_medias": 21,
    "catalog_medias": 2758,
    "cuts_entries": 612040,
    "sparse_entries": 2096680,
    "certified_couplings": 21,
    "ahash_medias": 584,
    "ahash_frames": 106096618
  }
  ```

### `GET /v1/catalog/status`
Retourne l'état des tables de mémoire MIH et les métriques de segmentation du catalogue (incluant `ahash_medias` et `ahash_frames` pour l'index acoustique).

- `total_frames` : nombre exact de frames pHash indexées (Base + Delta, même périmètre que
  `total_medias`), somme des comptes stockés par média à l'empaquetage (`dbsrt pack`) ou à
  l'ingestion.
- `medias_without_frame_count` : médias dont le compte est inconnu (catalogue empaqueté avant
  l'ajout du champ). Tant qu'il vaut plus de 0, `total_frames` est un minorant : réempaqueter.

### `POST /v1/catalog/ingest`
Ingère dynamiquement un lot d'empreintes pHash dans le catalogue en cours d'exécution.
