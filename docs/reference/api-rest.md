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

---

## 3. Endpoints de Recherche & Synchronisation

### `POST /v1/search`
Recherche de correspondances par empreintes pHash.
**Corps de la requête** :
  ```json
  {
    "hashes": ["0x1234567890abcdef", "0xfedcba0987654321"],
    "max_distance": 10,
    "limit": 5
  }
  ```
**Réponse** :
  ```json
  {
    "matches": [
      {
        "media_hash": "a1b2c3d4e5f60718293a4b5c6d7e8f90",
        "media_name": "Berserk.1997.EP01",
        "distance": 2,
        "offset_seconds": 15.0,
        "confidence": 98.5
      }
    ]
  }
  ```

### `POST /v1/autosync`
Endpoint combiné utilisé par les plugins VLC : analyse les empreintes extraites de la lecture en cours, calcule le décalage consensus (\( \Delta t \)), et renvoie le sous-titre aligné en une seule passe.
**Corps de la requête** :
  ```json
  {
    "sample_phash": "base64_encoded_binary_sample...",
    "target_fps": 25.0,
    "preferred_lang": "fr"
  }
  ```
**Réponse** :
  ```json
  {
    "matched": true,
    "media_name": "Apprendre_CakePHP_-_SessionHelper",
    "shift_ms": 1250,
    "confidence": 99.2,
    "subtitles_hash": "e3b0c44298fc1c149afbf4c8996fb924",
    "download_url": "/v1/subtitles/e3b0c44298fc1c149afbf4c8996fb924?shift_ms=1250"
  }
  ```

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
Déclenche le rechargement à chaud atomique (zéro coupure de service) des index en mémoire vive.
**Réponse** :
  ```json
  {
    "status": "ok",
    "reloaded_medias": 21,
    "elapsed_ms": 3
  }
  ```

### `GET /v1/catalog/status`
Retourne l'état des tables de mémoire MIH et les métriques de segmentation du catalogue.

### `POST /v1/catalog/ingest`
Ingère dynamiquement un lot d'empreintes pHash dans le catalogue en cours d'exécution.
