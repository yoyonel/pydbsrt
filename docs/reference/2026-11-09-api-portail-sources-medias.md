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

## 3. Options CLI du Démon (`scripts/portal_server.py`)

Le script supporte à la fois le mode serveur HTTP daemonisé et l'exécution unitaire en ligne de commande pour scripts ou cron jobs :

```bash
usage: portal_server.py [-h] [--port PORT] [--metadata] [--probe SOURCE PROFILE]

pydbsrt MediaSources Portal Server

options:
  -h, --help            Affiche l'aide et quitte.
  --port PORT           Port TCP d'écoute du serveur HTTP (par défaut: 8780).
  --metadata            Affiche la structure JSON des métadonnées sources et quitte.
  --probe SOURCE PROFILE
                        Exécute une sonde unique et affiche le JSON résultant.
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
