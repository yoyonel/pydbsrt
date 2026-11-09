---
title: "Spécification API REST & CLI du Portail Sources Médias"
tags:
  - reference
  - api
  - portal
  - media-sources
  - rest
---

# 📖 Spécification API REST & CLI du Portail Sources Médias

Cette référence technique décrit les spécifications formelles des points d'accès HTTP et du mode CLI exposés par le serveur léger de portail multimédia (`scripts/portal_server.py`).

---

## 1. Vue d'Ensemble & Endpoints HTTP

| Méthode | Chemin | Description | Latence cible |
| :--- | :--- | :--- | :--- |
| `GET` | `/sources.html` | Interface monopage réactive (HTML5 / Tailwind CSS / Alpine.js). | $< 5\text{ ms}$ |
| `GET` | `/favicon.ico` | Favicon SVG vectoriel (zéro erreur 404 dans la console). | $< 1\text{ ms}$ |
| `GET` | `/api/sources` | Métadonnées statiques des gisements, profils et environnements déclarés. | $< 1\text{ ms}$ |
| `GET` | `/api/probe` | Exécution synchrone d'une sonde granulaire par canal. | $5\text{ ms}$ (FS) à $3\text{ s}$ (FTP) |
| `GET` | `/api/files` | Listing paginé, filtré et trié des conteneurs réels d'un canal accessible. | $< 1\text{ ms}$ (cache) à $400\text{ ms}$ (FTP) |
| `GET` | `/api/file/details` | Inspection multimédia approfondie (`ffprobe`) et statut couplage Zero-DB. | $50\text{ ms}$ (FS) à $1.5\text{ s}$ (FTP) |
| `GET` | `/api/assets` | Audit paginé, filtré et trié des actifs vidéos répertoriés en base PostgreSQL. | $< 15\text{ ms}$ |
| `GET` | `/api/asset/details` | Fiche d'inspection technique unitaire d'un actif média en base PostgreSQL. | $< 10\text{ ms}$ |

---

## 2. Spécification des Endpoints REST

### `GET /api/sources`

Retourne la définition complète de l'architecture des sources médias, l'hôte détecté et les environnements d'exécution disponibles.

#### Paramètres
*Aucun paramètre.*

#### Schéma de Réponse (`200 OK`)

```json
{
  "host": "sapin.o2switch.net",
  "detected_env": "o2switch",
  "available_envs": [
    {
      "id": "o2switch",
      "name": "Cloud o2switch",
      "icon": "🌐",
      "description": "Hébergement distant (Flux WAN FTP & Passerelle HTTP)"
    },
    {
      "id": "local",
      "name": "Poste Local (debian-latty)",
      "icon": "💻",
      "description": "Poste de dev (Montages CIFS locaux, Snippets, FTP WAN)"
    },
    {
      "id": "rpi4",
      "name": "Appliance RPi4 (Maison)",
      "icon": "🍓",
      "description": "Nano-ordinateur domestique (/mnt/freebox, Gateway HTTP local, FTP LAN)"
    },
    {
      "id": "all",
      "name": "Diagnostic Brut (Tous)",
      "icon": "🔍",
      "description": "Vue exhaustive non filtrée de tous les profils déclarés"
    }
  ],
  "timestamp": 1794215904.19,
  "sources": [
    {
      "id": "synology-video",
      "name": "Synology NAS Movies & Videos",
      "description": "Gisement de films, longs-métrages et documentaires (Remux 4K, 1080p).",
      "profiles": [
        {
          "id": "cifs",
          "name": "Montage CIFS Local",
          "protocol": "local_fs",
          "target": "/media/<user>/Synology/video",
          "envs": ["local"],
          "status": "pending"
        },
        {
          "id": "wan_ftp",
          "name": "Streaming WAN FTP",
          "protocol": "ftp",
          "target": "ftp://<user>@<nas_host>:21/video",
          "envs": ["local", "o2switch", "rpi4"],
          "status": "pending"
        }
      ]
    }
  ]
}
```

---

### `GET /api/probe`

Déclenche le test en direct d'un canal d'accès spécifique et renvoie la mesure de latence et le nombre effectif de fichiers conteneurs trouvés.

#### Paramètres Query String

| Paramètre | Type | Requis | Valeurs admises | Description |
| :--- | :--- | :--- | :--- | :--- |
| `source` | `string` | **Oui** | `synology-video`, `synology-tvshow`, `freebox-disks`, `local-snippets` | Identifiant du gisement parent. |
| `profile` | `string` | **Oui** | `cifs`, `wan_ftp`, `disque1`, `disque2`, `lan_ftp`, `wan_http`, `local` | Identifiant du canal d'accès à tester. |
| `token` | `string` | Non | Clé secrète hexadécimale | Jeton d'authentification sur passerelle o2switch. |

#### Réponses Possibles

**Cas nominal : Canal Accessible (`200 OK`)**
```json
{
  "name": "Streaming WAN FTP",
  "protocol": "ftp",
  "target": "ftp://qzsl2686@yoyonel.synology.me:21/video",
  "accessible": true,
  "status": "online",
  "latency_ms": 300.9,
  "files_count": 541,
  "reason": "541 éléments listés via FTP"
}
```

**Cas d'échec : Montage absent ou distant injoignable (`200 OK`)**
```json
{
  "name": "Passerelle HTTP WAN",
  "protocol": "http_gateway",
  "target": "http://yoyonel.synology.me:18089/health",
  "accessible": false,
  "status": "offline",
  "latency_ms": 2534.3,
  "files_count": 0,
  "reason": "Passerelle injoignable (RPi4 offline)"
}
```

**Cas non authentifié sur passerelle o2switch (`401 Unauthorized`)**
```json
{
  "error": "Unauthorized",
  "message": "Valid API key required via X-API-Key header, Bearer token, session cookie, or query token."
}
```

---

### `GET /api/files`

Retourne la liste paginée, filtrée et triée des fichiers vidéo conteneurs et sous-dossiers disponibles sur un canal accessible.

> [!NOTE]
> **Architecture de Cache Double Étage (Mémoire & Disque)** :
>
> 1. **Étage 1 (Mémoire vive)** : TTL de 300 s au sein du démon HTTP local.
> 2. **Étage 2 (Disque persistant)** : Enregistré dans `~/.cache/dbsrt/portal/listing_<hash>.json`. Partagé universellement entre le démon résident, les nano-ordinateurs (RPi4) et les invocations CLI cPanel sur le cloud distant (`o2switch`), éliminant les latences WAN répétées ($< 1\text{ ms}$ au lieu de $1.2\text{ s}$).

#### Paramètres Query String

| Paramètre | Type | Requis | Défaut | Description |
| :--- | :--- | :--- | :--- | :--- |
| `source` | `string` | **Oui** | - | Identifiant du gisement (`synology-video`, `synology-tvshow`, etc.). |
| `profile` | `string` | **Oui** | - | Identifiant du canal (`wan_ftp`, `local`, `cifs`). |
| `subdir` | `string` | Non | `""` | Sous-répertoire relatif pour la navigation hiérarchique. |
| `page` | `integer` | Non | `1` | Numéro de la page (commence à 1). |
| `limit` | `integer` | Non | `25` | Nombre d'éléments par page (`25`, `50`, `100`, max 100). |
| `q` | `string` | Non | `""` | Requête de recherche syntaxique / fuzzy (titre, année, 4K, 1080p, HEVC, etc.). |
| `sort` | `string` | Non | `name_asc` | Ordre de tri (`name_asc`, `name_desc`, `size_desc`, `size_asc`, `date_desc`). |
| `force` | `boolean` | Non | `false` | Forcer l'invalidation du cache et relancer une sonde brute (`1` ou `true`). |
| `token` | `string` | Non | - | Jeton d'authentification sur passerelle o2switch. |

#### Schéma de Réponse (`200 OK`)

```json
{
  "status": "ok",
  "source_id": "synology-video",
  "profile_id": "wan_ftp",
  "subdir": "",
  "channel_name": "Streaming WAN FTP",
  "protocol": "ftp",
  "base_target": "ftp://<user>@<nas_host>:21/video",
  "total_items": 474,
  "filtered_items": 2,
  "page": 1,
  "limit": 25,
  "total_pages": 1,
  "items": [
    {
      "name": "Dune (2021)[2160p](fr SRT).mkv",
      "path": "Dune (2021)[2160p](fr SRT).mkv",
      "size_bytes": 22344195047,
      "size_human": "20.81 GB",
      "is_dir": false,
      "modify": "2022-02-17 11:21:49",
      "extension": ".mkv",
      "parsed": {
        "title": "Dune",
        "year": "2021",
        "resolution": "2160P",
        "codec": null,
        "source": null,
        "audio": null,
        "language": null
      }
    }
  ]
}
```

---

### `GET /api/file/details`

Exécute l'inspection multimédia approfondie via `ffprobe` (ou fallback transparent si non disponible) et vérifie l'existence d'un couplage certifié dans le registre Zero-DB local (`couplings.json`). Les résultats sont mis en cache persistant sur disque (TTL de 24 heures) pour éviter toute ré-analyse superflue.

#### Paramètres Query String

| Paramètre | Type | Requis | Défaut | Description |
| :--- | :--- | :--- | :--- | :--- |
| `source` | `string` | **Oui** | - | Identifiant du gisement. |
| `profile` | `string` | **Oui** | - | Identifiant du canal d'accès. |
| `path` | `string` | **Oui** | - | Chemin relatif du conteneur vidéo à inspecter. |
| `force` | `boolean` | Non | `false` | Forcer une ré-analyse live `ffprobe` en ignorant le cache (`1` ou `true`). |
| `token` | `string` | Non | - | Jeton d'authentification sur passerelle o2switch. |

#### Schéma de Réponse (`200 OK`)

```json
{
  "status": "ok",
  "name": "Dune (2021)[2160p](fr SRT).mkv",
  "path": "Dune (2021)[2160p](fr SRT).mkv",
  "extension": ".mkv",
  "size_bytes": 22344195047,
  "size_human": "20.81 GB",
  "modify": "2022-02-17 11:21:49",
  "source_id": "synology-video",
  "profile_id": "wan_ftp",
  "uri": "ftp://<nas_host>:21/video/Dune (2021)[2160p](fr SRT).mkv",
  "parsed": {
    "title": "Dune",
    "year": "2021",
    "resolution": "2160P"
  },
  "coupling": null,
  "probed": true,
  "reason": "",
  "duration_seconds": 9354.984,
  "duration_formatted": "02h 35m 55s",
  "video": {
    "codec": "hevc",
    "codec_long": "H.265 / HEVC",
    "profile": "Main 10",
    "width": 3840,
    "height": 2160,
    "aspect_ratio": "16:9",
    "fps": "24000/1001",
    "pix_fmt": "yuv420p10le"
  },
  "audio_streams": [
    {
      "index": 1,
      "codec": "eac3",
      "channels": 6,
      "channel_layout": "5.1(side)",
      "language": "fre",
      "title": "French"
    }
  ],
  "subtitle_streams": [
    {
      "index": 1,
      "codec": "subrip",
      "language": "fre",
      "title": "Forced",
      "forced": true,
      "default": true
    }
  ]
}
```

---

### `GET /api/assets`

Interroge la table relationnelle `medias` dans PostgreSQL (avec fallback transparent sur le binaire `psql` CLI si le module Python `psycopg2` est indisponible sur l'hôte, ex: hébergement mutualisé o2switch). Calcule les KPIs globaux du catalogue, audite la présence physique des fichiers conteneurs, vérifie l'ancrage des sidecars `.phash` sur disque, et clampe automatiquement la pagination si la page demandée dépasse le total d'enregistrements filtrés.

#### Paramètres Query String

| Paramètre | Type | Requis | Défaut | Description |
| :--- | :--- | :--- | :--- | :--- |
| `page` | `integer` | Non | `1` | Index de la page courante (auto-clampé à 1 si supérieur au nombre de pages). |
| `limit` | `integer` | Non | `25` | Nombre d'enregistrements par page (`15`, `25`, `50`, `100`). |
| `status` | `string` | Non | `all` | Filtre métier : `all` (tous), `nominal` (complet & localisé), `missing_phash` (empreinte absente/pending), `unmapped_path` (chemin manquant). |
| `source` | `string` | Non | `""` | Filtre par identifiant de gisement (`synology-video`, `synology-tvshow`, `local-snippets`). |
| `q` | `string` | Non | `""` | Recherche textuelle insensible à la casse sur `clean_title`, `name` ou `moviehash`. |
| `sort` | `string` | Non | `title_asc` | Critère d'ordonnancement : `title_asc`, `title_desc`, `frames_desc`, `duration_desc`, `id_desc`, `id_asc`. |
| `token` | `string` | Non | - | Jeton d'authentification sur passerelle distante o2switch. |

#### Schéma de Réponse (`200 OK`)

```json
{
  "status": "ok",
  "connected": true,
  "summary": {
    "total": 2321,
    "nominal": 1782,
    "missing_phash": 398,
    "unmapped_path": 505
  },
  "page": 1,
  "limit": 25,
  "total_items": 24,
  "total_pages": 1,
  "items": [
    {
      "id": 1358,
      "name": "Carnivàle (2003) - S01E01 - Milfay (1080p AMZN WEBRip x265 Panda).mkv",
      "clean_title": "Carnivàle",
      "season": 1,
      "episode": 1,
      "year": 2003,
      "resolution": "1080p",
      "video_codec": "HEVC",
      "fps": 24.0,
      "is_vfr": false,
      "frame_count": 82950,
      "expected_frames": 82950,
      "duration_ms": 3456250,
      "duration_fmt": "57:36",
      "file_size_bytes": 1420580922,
      "file_size_human": "1.32 GB",
      "moviehash": "8f2a1b9c3e4d5f6a",
      "media_hash": "dd599e4f1f9b6478edc4b779cd172ff4",
      "source_id": "synology-tvshow",
      "video_rel_path": "/tvshow/Carnivàle/Season 01/S01E01.mkv",
      "physical_exists": true,
      "physical_resolved_path": "/media/user/Synology/tvshow/Carnivàle/Season 01/S01E01.mkv",
      "phash_status": "complete",
      "phash_on_disk": true,
      "status_tag": "nominal",
      "alt_channels": [
        {
          "id": "wan_ftp",
          "name": "Streaming WAN FTP (Synology)",
          "accessible": true,
          "target": "ftp://user@nas.me:21/tvshow/Carnivàle/Season 01/S01E01.mkv"
        }
      ]
    }
  ]
}
```

---

### `GET /api/asset/details`

Retourne la fiche d'inspection unitaire détaillée pour un média spécifique identifié par son identifiant unique PostgreSQL (`id`).

#### Paramètres Query String

| Paramètre | Type | Requis | Défaut | Description |
| :--- | :--- | :--- | :--- | :--- |
| `id` | `integer` | **Oui** | - | Identifiant numérique primaire du média (`medias.id`). |
| `token` | `string` | Non | - | Jeton d'authentification sur passerelle distante o2switch. |

#### Schéma de Réponse (`200 OK`)

```json
{
  "status": "ok",
  "id": 1358,
  "name": "Carnivàle (2003) - S01E01 - Milfay (1080p AMZN WEBRip x265 Panda).mkv",
  "clean_title": "Carnivàle",
  "season": 1,
  "episode": 1,
  "year": 2003,
  "resolution": "1080p",
  "video_codec": "HEVC",
  "width": 1920,
  "height": 1080,
  "fps": 24.0,
  "frame_count": 82950,
  "expected_frames": 82950,
  "is_vfr": false,
  "duration_ms": 3456250,
  "duration_fmt": "57:36",
  "file_size_bytes": 1420580922,
  "file_size_human": "1.32 GB",
  "moviehash": "8f2a1b9c3e4d5f6a",
  "media_hash": "dd599e4f1f9b6478edc4b779cd172ff4",
  "video_rel_path": "/tvshow/Carnivàle/Season 01/S01E01.mkv",
  "source_id": "synology-tvshow",
  "phash_status": "complete",
  "phash_path": "data/__PHASH__/8f2a1b9c3e4d5f6a_24fps.phash",
  "frames_in_db": 82950,
  "physical_exists": true,
  "physical_resolved_path": "/media/user/Synology/tvshow/Carnivàle/Season 01/S01E01.mkv",
  "phash_on_disk": true,
  "created_at_fmt": "2026-10-24 14:12:00",
  "updated_at_fmt": "2026-11-09 11:30:15",
  "alt_channels": [
    {
      "id": "wan_ftp",
      "name": "Streaming WAN FTP (Synology NAS)",
      "protocol": "ftp",
      "accessible": true,
      "target": "ftp://user@nas.me:21/tvshow/Carnivàle/Season 01/S01E01.mkv",
      "note": "Accessible publiquement via Internet avec authentification pass"
    }
  ]
}
```

---

## 3. Options CLI du Démon (`scripts/portal_server.py`)

Le script supporte à la fois le mode serveur HTTP daemonisé et l'exécution unitaire en ligne de commande pour scripts ou cron jobs :

```bash
usage: portal_server.py [-h] [--port PORT] [--metadata] [--probe SOURCE PROFILE]
                        [--files SOURCE PROFILE] [--subdir SUBDIR] [--page PAGE]
                        [--limit LIMIT] [--search SEARCH] [--sort SORT]
                        [--details SOURCE PROFILE PATH] [--assets]
                        [--asset-id ASSET_ID] [--status STATUS]
                        [--source SOURCE] [--force]

pydbsrt MediaSources Portal Server

options:
  -h, --help            Affiche l'aide et quitte.
  --port PORT           Port TCP d'écoute du serveur HTTP (par défaut: 8780).
  --metadata            Affiche la structure JSON des métadonnées sources et quitte.
  --probe SOURCE PROFILE
                        Exécute une sonde unique et affiche le JSON résultant.
  --files SOURCE PROFILE
                        Liste les fichiers conteneurs du canal spécifié.
  --subdir SUBDIR       Sous-répertoire au sein du gisement (défaut: racine).
  --page PAGE           Numéro de la page demandée (défaut: 1).
  --limit LIMIT         Nombre d'éléments par page (défaut: 25).
  --search SEARCH       Termes de recherche / filtre (nom, année, 4K, HEVC...).
  --sort SORT           Ordre de tri (title_asc, title_desc, frames_desc, duration_desc, id_desc).
  --details SOURCE PROFILE PATH
                        Inspecte les caractéristiques multimédia complètes d'un fichier.
  --assets              Interroge et audite les actifs vidéo de PostgreSQL 17.
  --asset-id ASSET_ID   Affiche la fiche détaillée d'un actif média par son ID PostgreSQL.
  --status STATUS       Filtre de statut métier pour les actifs (all, nominal, missing_phash, unmapped_path).
  --source SOURCE       Filtre de gisement pour les actifs ou les conteneurs.
  --force               Ignore le cache disque et force une nouvelle sonde brute.
```

---

## 4. Matrice des Canaux et Protocoles

| Source ID | Profile ID | Protocole | Environnements Cibles | Port / Cible |
| :--- | :--- | :--- | :--- | :--- |
| `synology-video` | `cifs` | `local_fs` | `local` | `/media/<user>/Synology/video` |
| `synology-video` | `wan_ftp` | `ftp` | `local`, `o2switch`, `rpi4` | `TCP 21` (FTP PASV) |
| `synology-tvshow` | `cifs` | `local_fs` | `local` | `/media/<user>/Synology/tvshow` |
| `synology-tvshow` | `wan_ftp` | `ftp` | `local`, `o2switch`, `rpi4` | `TCP 21` (FTP PASV) |
| `freebox-disks` | `disque1` | `local_fs` | `local` | `/media/<user>/Freebox/Disque_1` |
| `freebox-disks` | `disque2` | `local_fs` | `local` | `/media/<user>/Freebox/Disque_2` |
| `freebox-disks` | `rpi4_disque1` | `local_fs` | `rpi4` | `/mnt/freebox/Disque_1` |
| `freebox-disks` | `rpi4_disque2` | `local_fs` | `rpi4` | `/mnt/freebox/Disque_2` |
| `freebox-disks` | `lan_ftp` | `ftp` | `local`, `rpi4` | `mafreebox.freebox.fr:21` |
| `freebox-disks` | `wan_http` | `http_gateway` | `local`, `o2switch` | `TCP 80 / 18089` (Range Gateway) |
| `freebox-disks` | `rpi4_gateway` | `http_gateway` | `rpi4` | `127.0.0.1:18089/health` |
| `local-snippets` | `local` | `local_fs` | `local` | `data/snippets` |

---

## 5. Routage Client SPA, État Persistant & Layout Découplé

L'interface web monopage (`sources.html`) implémente un contrat d'état bidirectionnel sans dépendance externe lourde :

### Grammaire du Hash URL

Le fragment d'URL encapsule l'intégralité du contexte d'exploration et d'audit :

```text
#view=<sources|explorer|assets>&env=<local|o2switch|rpi4|all>&source=<id>&profile=<id>&subdir=<rel_path>&page=<int>&limit=<int>&q=<query>&sort=<key>&tag=<tag>&file=<rel_path>&status=<status>
```

| Clé | Type | Rôle | Valeur par défaut |
| :--- | :--- | :--- | :--- |
| `view` | `string` | Vue active (`sources`, `explorer` ou `assets`). | `sources` |
| `env` | `string` | Filtre d'environnement actif. | Détection dynamique de l'hôte |
| `status` | `string` | Filtre de statut pour les actifs médias (`all`, `nominal`, `missing_phash`, `unmapped_path`). | `all` |
| `source` | `string` | Identifiant du gisement sélectionné. | `""` |
| `profile` | `string` | Canal d'accès choisi (vue explorer). | `""` |
| `subdir` | `string` | Sous-chemin relatif dans le gisement (vue explorer). | `""` (racine) |
| `page` | `integer` | Index de pagination courante (auto-reset à 1 lors de filtre ou recherche). | `1` |
| `limit` | `integer` | Nombre d'enregistrements par page. | `25` |
| `q` | `string` | Terme de recherche textuelle. | `""` |
| `sort` | `string` | Critère d'ordonnancement. | `name_asc` (explorer) / `title_asc` (assets) |
| `tag` | `string` | Filtre rapide de définition ou format (`2160p`, `1080p`, `hevc`, `mkv`). | `""` |
| `file` | `string` | Chemin relatif du média inspecté dans la fiche de détail (vue explorer). | `""` |

### Architecture de Défilement Découplé

Afin de garantir un confort d'usage optimal sans saut de page lors de l'inspection de bibliothèques volumineuses :

1. **Isolation Viewport (`overflow: hidden`)** : En mode explorateur, le conteneur principal (`<main>`) et le `<body>` sont strictement verrouillés en hauteur viewport (`100vh`). La fenêtre racine n'a aucun ascenseur global (`window.scrollY = 0`).
2. **Double Conteneur Flex Scrollable** :
   - Le bloc de gauche (Master) possède son propre conteneur scrollable (`overflow-y: auto`) avec barre de pagination et bouton de retour parent épinglés (`shrink-0`).
   - Le bloc de droite (Detail) possède son propre conteneur scrollable (`overflow-y: auto`) affichant l'intégralité des flux multimédia sans influencer la position de la liste de gauche.
3. **Thème Sombre Intégral** : La directive `color-scheme: dark` couplée aux pseudo-éléments `::-webkit-scrollbar` et `scrollbar-color` garantit l'affichage exclusif de barres de défilement sombres ardoise/indigo compactes (6px) sur tous les navigateurs modernes (Chromium, Firefox, Safari).
