---
title: Comment déployer pydbsrt sur un hébergement mutualisé o2switch (cPanel)
tags:
  - howto
  - deployment
  - o2switch
  - cpanel
  - zero-db
  - musl
---

# 🛠️ Comment Déployer pydbsrt sur un Hébergement Mutualisé o2switch (Cloud cPanel)

> Ce guide pratique détaille la procédure complète pour déployer et opérer le microservice d'empreintes visuelles `dbsrt-server` sur un hébergement web mutualisé **o2switch** (cPanel / CloudLinux).

---

## 🎯 Problématique & Contraintes du Mutualisé

Sur un hébergement mutualisé cPanel comme o2switch, les contraintes opérationnelles suivantes s'appliquent :

1. **Zéro privilège root** : Impossible d'installer des paquets système via `apt` ou d'exécuter des daemons système globaux (`/etc/systemd/system/`).
2. **Zéro conteneur Docker** : Le moteur Docker n'est pas disponible pour les utilisateurs non privilégiés.
3. **Ports réseau restreints** : Seuls les ports HTTP/HTTPS publics (80 et 443) sont ouverts sur l'extérieur. Les ports arbitraires comme 8765 sont inaccessibles publiquement.
4. **Quotas LVE stricts** : Le noyau CloudLinux LVE restreint les processus d'entrée (EP $\le$ 30-60), la mémoire vive (1-2 Go) et l'utilisation CPU.
5. **Absence d'extension PostgreSQL SP-GiST dédiée** : Seul MySQL ou un PostgreSQL standard sans extensions C compilées est accessible.

---

## 💡 Solution Technique : Binaire Musl Statique & Zero-DB

Pour contourner ces contraintes sans dégrader les performances, `pydbsrt` exploite son mode **Zero-DB** :

* **Binaire autonome 100% statique** : Compilé avec la cible `x86_64-unknown-linux-musl`, le binaire `dbsrt-server` n'a aucune dépendance dynamique vers la libc hôte.
* **Index MIH en mémoire RAM (`catalog.dbphash`)** : Chargement de l'index binaire unifié en RAM (< 30 Mo pour 5M trames, recherche en < 3 ms, 0 requête SQL).
* **Stockage CAS (`data/SRT/`)** : Les pistes de sous-titres certifiées sont stockées sous leur empreinte SHA-256 et servies directement par le microservice.
* **Passerelle Reverse-Proxy PHP & LiteSpeed (`public_html/dbsrt/`)** : Apache/LiteSpeed sur hébergement mutualisé interdit les directives `mod_proxy [P]`. Une passerelle ultra-légère (`index.php` + `.htaccess`) relaye les requêtes entrantes vers le socket Unix `~/dbsrt/run/dbsrt-server.sock` (o2switch tue tout listener TCP en quelques secondes, pas les sockets Unix) et réveille automatiquement le démon à la demande (auto-spawn en < 500 ms) en cas de récolte par CloudLinux.
* **Sécurité & Authentification Clé API** : La passerelle PHP filtre les requêtes non autorisées (`401 Unauthorized`) et exige une clé secrète via l'en-tête `X-API-Key` ou le Bearer token `Authorization` (le paramètre d'URL `?api_key=` est refusé). Sans clé provisionnée, elle répond `503` (fail closed). De plus, les chemins réels du système de fichiers hôte sont sanitisés sur la route `/v1/status` (zéro divulgation de structure interne).
* **Plafond de bande passante I/O LVE** : CloudLinux limite le débit disque à 42 MiB/s sur l'Offre Unique o2switch. Les transferts `rsync` sont bridés à 20 Mo/s (`--bwlimit=20000`) pour garantir zéro faute LVE I/O lors des synchronisations du catalogue.
* **Watchdog Crontab non-root** : Orchestré via crontab utilisateur (`*/5 * * * *`) pour relancer automatiquement le démon en cas de redémarrage hôte.
* **Gestionnaire de daemon non-root par PID** : Orchestré via `scripts/o2switch_ctl.sh` et les cibles Taskfile associées.

---

## 📋 Prérequis

* Un compte d'hébergement o2switch avec accès SSH activé dans l'interface cPanel.
* Un alias SSH configuré dans `~/.ssh/config` :
  ```sshconfig
  Host o2switch
      HostName votredomaine.fr
      User monutilisateur
      Port 22
  ```
* L'outil `task` installé sur votre poste de développement.

---

## 🚀 Étapes de Déploiement

### 1. Compilation des binaires statiques musl

Compilez les exécutables autonomes `dbsrt` et `dbsrt-server` pour l'architecture x86_64 musl :

```bash
task o2switch:build
```

Cette cible compile en mode `--release`, puis applique `strip` sur les binaires pour réduire l'empreinte disque (~9 Mo chacun).

### 2. Validation empirique locale en mode Zero-DB

Avant tout transfert réseau, validez le bon fonctionnement du serveur autonome sur votre poste :

```bash
task o2switch:test-local
```

Cette cible lance un test hermétique automatisé :

1. Démarrage de `dbsrt-server` sur un port de test isolé (`8769`) avec chargement de `data/catalog.dbphash`.
2. Interrogation de la sonde de santé (`/v1/health`) et du statut (`/v1/status`).
3. Émission d'une requête de reconnaissance pHash (`/v1/recognize`) et téléchargement d'un sous-titre CAS (`/v1/subtitles/{hash}`).
4. Mesure de l'empreinte mémoire RSS et arrêt propre du processus.

### 3. Synchronisation de la pile vers o2switch

Transférez les binaires statiques, le catalogue d'indexation Zero-DB, le dépôt de sous-titres et la configuration reverse-proxy :

```bash
task o2switch:sync
```

Les répertoires distants suivants sont initialisés sur l'hébergement :

* `~/dbsrt/bin/` : Exécutables `dbsrt` et `dbsrt-server`.
* `~/dbsrt/data/` : Catalogue `catalog.dbphash` et sous-titres `SRT/`.
* `~/dbsrt/run/` : Fichier de suivi de PID (`dbsrt-server.pid`).
* `~/dbsrt/logs/` : Fichier journal d'exécution (`dbsrt-server.log`).
* `~/public_html/dbsrt/.htaccess` : Règle de reverse-proxy Apache / LiteSpeed.

### 4. Démarrage du daemon utilisateur

Démarrez le serveur en arrière-plan sans privilèges root :

```bash
task o2switch:start
```

Le script compacte d'abord les `.phash` du batch (`data/__PHASH__`) dans `data/catalog.dbphash` via `dbsrt pack` (~1,5 s). Il démarre ensuite `dbsrt-server` via `setsid` avec un dossier `.phash` vide (`run/empty_phash`), enregistre le PID et attend la sonde de santé via SSH.

> [!WARNING]
> **o2switch tue (SIGKILL) tout processus utilisateur en écoute sur un port après ~10 s**, même `python3 -m http.server`. Le daemon ne reste donc pas résident : la passerelle `index.php` le relance à la demande (~1,3 s à froid, ~80 ms à chaud), et le cron watchdog fait de même toutes les 5 min. Le spawn à la demande ne recompacte pas : un second cron (toutes les 15 min) intègre les nouveaux `.phash` du batch au catalogue, rechargé à chaud.
>
> Ne pas pointer `--phash-dir` sur `data/__PHASH__` : chaque `.phash` isolé est mappé avec un index MIH en RAM (~6x sa taille disque). Avec ~1 200 fichiers, le serveur dépasse 2,7 Go et se fait tuer. Le catalogue compacté tient en ~340 Mo.

### 5. Vérification du statut et de l'empreinte mémoire

Contrôlez l'état du processus et la télémétrie de l'API :

```bash
task o2switch:status
```

La sortie affiche l'état du PID, l'utilisation CPU/RAM (~340 Mo RSS avec le catalogue compacté de ~1 200 médias), et la réponse JSON de `/v1/status`.

### 6. Test de reconnaissance de bout en bout

Exécutez la démonstration de reconnaissance pour vérifier l'appairage et la récupération de sous-titres :

```bash
task o2switch:demo
```

Pour tester l'accès public HTTPS via la passerelle sécurisée, définissez la variable d'environnement `O2SWITCH_URL` (la clé API est automatiquement résolue depuis votre coffre `pass`) :

```bash
O2SWITCH_URL=https://votredomaine.fr/dbsrt task o2switch:demo
```

---

## 🔐 Sécurité, Authentification & Rotation des Clés API

Le déploiement Cloud mutualisé applique un modèle de **sécurité en profondeur à 5 niveaux** :

```
               Requête HTTPS entrante
                         │
                         ▼
        ┌───────────────────────────────────┐
        │  Niveau 1 : Certificat TLS & 301  │  (Let's Encrypt RSA 2048, HSTS / 301)
        └─────────────────┬─────────────────┘
                          │
                          ▼
        ┌───────────────────────────────────┐
        │  Niveau 2 : Authentification API  │  (hash_equals timing-safe, rejet 401)
        └─────────────────┬─────────────────┘
                          │
                          ▼
        ┌───────────────────────────────────┐
        │  Niveau 3 : Auto-Spawn Résilient  │  (Réveil daemon en <500ms si récolté)
        └─────────────────┬─────────────────┘
                          │
                          ▼
        ┌───────────────────────────────────┐
        │  Niveau 4 : Démon dbsrt-server    │  (socket Unix run/dbsrt-server.sock, Zero-DB, 0 accès SQL)
        └─────────────────┬─────────────────┘
                          │
                          ▼
        ┌───────────────────────────────────┐
        │  Niveau 5 : Sanitisation Télémétrie│ (Masquage paths locaux sur /v1/status)
        └───────────────────────────────────┘
```

### 1. Formats d'authentification acceptés

La passerelle `index.php` prend en charge deux méthodes d'authentification interchangeables. Le paramètre d'URL `?api_key=` n'est **plus accepté** (une clé dans l'URL fuiterait dans les journaux d'accès et l'historique navigateur). Les exemples ci-dessous utilisent l'URL de base de la passerelle, définie localement dans `O2SWITCH_URL` (environnement ou `.env` non versionné, ex: `https://<your-domain>/dbsrt`, sans valeur par défaut versionnée) :

1. **En-tête standard (Recommandé en CLI / Scripting)** :
   ```http
   GET /dbsrt/v1/health HTTP/1.1
   Host: <your-domain>
   X-API-Key: <VOTRE_CLE_256_BITS>
   ```
2. **Token Bearer (Recommandé pour les microservices / OAuth)** :
   ```http
   GET /dbsrt/v1/health HTTP/1.1
   Host: <your-domain>
   Authorization: Bearer <VOTRE_CLE_256_BITS>
   ```

Exemple en ligne de commande :

```bash
curl -H "X-API-Key: <VOTRE_CLE_256_BITS>" https://<your-domain>/dbsrt/v1/status
```

VLC ne s'authentifie pas directement : il passe par l'agent relais local (`127.0.0.1:8765`), qui ajoute l'en-tête `X-API-Key` (voir section 5).

Toute requête dépourvue d'une clé valide est immédiatement rejetée avec le statut `401 Unauthorized` :

```json
{
    "error": "Unauthorized",
    "message": "Valid API key required via X-API-Key header or Bearer token."
}
```

Si `~/dbsrt/etc/api_key.secret` est absent ou vide, la passerelle refuse **toutes** les requêtes avec `503 Service Unavailable` (comportement *fail closed*) au lieu de laisser passer les appels non authentifiés.

### 2. Stockage sécurisé et isolation du secret

* **Sur le serveur distant** : La clé est enregistrée dans `~/dbsrt/etc/api_key.secret`. Ce fichier est situé **hors de l'arborescence web** `public_html/`, empêchant tout téléchargement direct, avec des permissions strictes `chmod 600` (lecture/écriture réservée à l'utilisateur système).
* **Sur le poste opérateur** : La clé est automatiquement sauvegardée dans le gestionnaire de mots de passe chiffré Unix `pass` sous l'entrée `services/pydbsrt/o2switch_api_key`.
* **Support Multi-Clés (Zero-Downtime Rolling Update)** : Le fichier `api_key.secret` accepte plusieurs clés (une par ligne). La passerelle valide la requête dès qu'une des clés correspond, permettant des transitions progressives sans coupure de service.
* **Clé lecture seule** : `~/dbsrt/etc/api_key_readonly.secret` (même format, entrée `pass` `services/pydbsrt/o2switch_readonly_key`) n'ouvre que les `GET`/`HEAD` sur `/monitoring`, `/top`, `/health`, `/v1/health` et `/v1/monitoring/*` ; tout le reste répond `403`. C'est elle qu'embarque `task o2switch:monitoring`. Création ou rotation : `task o2switch:rotate-readonly-key`.
* **Défense en profondeur côté démon** : `dbsrt-server` est lancé avec `--api-key-file ~/dbsrt/etc/api_key.secret` (spawn PHP, `start`, watchdog cron). Toute route hors `/health` et `/v1/health` exige alors `Authorization: Bearer <clé maître>` ou `X-API-Key`, y compris sur le socket Unix (déjà en mode `0600`) : un autre compte de l'hébergement mutualisé ne peut plus l'interroger. La passerelle retire les identifiants du client et injecte la clé maître. Le fichier est relu à chaque requête (rotation sans redémarrage) ; illisible, le démon répond `503`. Sans `--api-key-file` (dev, RPi4, tests) l'API reste ouverte.

### 3. Rotation automatisée des clés via Taskfile

Pour renouveler la clé secrète à chaud :

```bash
task o2switch:rotate-key
```

Cette commande exécute une séquence 100% automatisée et vérifiée :

1. Génération cryptographique d'une clé aléatoire de 256 bits (`openssl rand -hex 32`).
2. Déploiement distant atomique dans `~/dbsrt/etc/api_key.secret` avec permissions `0600`.
3. Synchronisation automatique dans le magasin local `pass` (`services/pydbsrt/o2switch_api_key`).
4. Test direct de validation HTTPS sur l'endpoint `/v1/health` (assertion `HTTP 200 OK`), sauté si `O2SWITCH_URL` n'est pas défini.
5. Assertion de sécurité négative : vérification formelle que les requêtes non authentifiées retournent bien `HTTP 401 Unauthorized`.

### 4. Sanitisation anti-divulgation (`/v1/status`)

La route `/v1/status` du serveur Rust expose par défaut les chemins absolus des répertoires de stockage (`phash_dir` et `subtitles_dir`). La passerelle PHP filtre systématiquement ces champs avant d'émettre la réponse au client externe :

```json
{
    "status": "ok",
    "version": "0.1.0",
    "service": "dbsrt-server",
    "uptime_seconds": 12,
    "indexed_phash_count": 1566,
    "certified_couplings_count": 2505,
    "postgres_connected": false
}
```

### 5. Mode Agent Client Relais (VLC + NAS Synology)

Pour interconnecter le lecteur VLC local avec les vidéos hébergées sur votre NAS Synology et le serveur Cloud o2switch :

1. Démarrez l'agent relais local :
   ```bash
   task o2switch:agent:start
   ```
   * Écoute sur `127.0.0.1:8765` et publie le descripteur de découverte `/tmp/dbsrt-server.json` pour VLC.
   * Lit automatiquement la clé secrète depuis le magasin `pass` (`services/pydbsrt/o2switch_api_key`).
   * Extrait les trames pHash de la vidéo locale ou CIFS en temps réel (décodage FFmpeg/GPU local) et relaye la signature visuelle vers `${O2SWITCH_URL}/v1/recognize` (ex: `https://<your-domain>/dbsrt/v1/recognize`) avec l'en-tête `X-API-Key`. L'agent refuse de démarrer si `O2SWITCH_URL` n'est pas défini.
2. Testez la chaîne complète sur un média réel du Synology :
   ```bash
   task o2switch:agent:demo
   ```
   * Vérifie la lecture du fichier volumineux (ex. *Andor S01E01* de 1.8 Go sur le partage Synology `/mnt/synology/tvshow/...`).
   * Valide la reconnaissance 100% confiance en Cloud o2switch et le téléchargement du sous-titre officiel.

### 6. Synchronisation NAS Synology vers Cloud o2switch

Deux approches opérationnelles permettent d'alimenter en continu le catalogue Cloud o2switch depuis le NAS Synology :

#### Approche A : Tâche locale pilotée par Taskfile (Recommandée)
Lorsque le NAS Synology est monté sur le poste de travail (`/media/$USER/Synology/tvshow` ou `$HOME/__SYNOLOGY__/tvshow`) :
```bash
# Simulation à sec sans transfert ni redémarrage démon
task o2switch:sync-nas:dry-run

# Synchronisation effective et redémarrage à chaud du démon
task o2switch:sync-nas
```
Cette tâche orchestre automatiquement :

1. La découverte dynamique des dossiers `.phash` sur les montages NAS actifs.
2. La compilation instantanée (< 2s) de l'index binaire unifié `target/catalog_nas.dbphash` via `dbsrt pack`.
3. Le transfert rsync sécurisé sous quota CloudLinux LVE (`--bwlimit=20000`, 20 Mo/s maximum).
4. La synchronisation du magasin de sous-titres `data/SRT/`.
5. Le redémarrage à chaud du démon `dbsrt-server` sur o2switch et le contrôle de conformité HTTP `/v1/status`.

#### Approche B : Tâche planifiée autonome dans Synology DSM

Pour synchroniser directement depuis le NAS Synology sans station de travail :

* **Serveur SSH distant (rsync)** : `<o2switch_server>` (Port 22, utilisateur `<cpanel_user>`).
* **Authentification** : Clé SSH Ed25519 dédiée (`id_ed25519_o2switch`).
* **Méthodes d'intégration Synology DSM** :

  1. **Synology Cloud Sync / Hyper Backup** : Configurer une tâche de synchronisation unidirectionnelle pointant vers le dossier distant `dbsrt/data/__PHASH__/` et `dbsrt/data/SRT/`.
  2. **Tâche planifiée DSM (Planificateur de tâches)** :
     ```bash
     rsync -avz --bwlimit=20000 -e "ssh -p 22 -i /volume1/homes/admin/.ssh/id_ed25519_o2switch" \
       /volume1/tvshow/__PHASH__/ <cpanel_user>@<o2switch_server>:dbsrt/data/__PHASH__/
     ```
     *Le paramètre `--bwlimit=20000` (20 Mo/s) est obligatoire pour respecter le plafond CloudLinux LVE de 42 MiB/s.*

---

## 🔄 Commandes de Cycle de Vie & Maintenance

| Commande | Description |
| :--- | :--- |
| `task o2switch:build` | Cross-compile les binaires x86_64 musl statiques et applique le `strip`. |
| `task o2switch:test-local` | Valide empiriquement le binaire musl en mode Zero-DB localement. |
| `task o2switch:sync` | Synchronise les binaires, le catalogue et la passerelle sous quota I/O (20 Mo/s). |
| `task o2switch:sync-nas` | Compile le catalogue NAS et synchronise l'index et les sous-titres vers o2switch. |
| `task o2switch:sync-nas:dry-run` | Simule à sec la synchronisation NAS vers o2switch sans transfert ni redémarrage. |
| `task o2switch:start` | Démarre le démon en arrière-plan sur o2switch. |
| `task o2switch:status` | Affiche l'état du PID et la télémétrie API à distance. |
| `task o2switch:logs` | Suit en direct les logs d'exécution sur le serveur distant. |
| `task o2switch:restart` | Redémarre le démon à distance. |
| `task o2switch:stop` | Arrête proprement le démon via signal `SIGTERM` (puis `SIGKILL` si nécessaire). |
| `task o2switch:demo` | Exécute un test de reconnaissance et de téléchargement de sous-titres. |
| `task o2switch:setup-cron` | Installe le watchdog utilisateur (`*/5 * * * *`) et le recompactage du catalogue (`*/15 * * * *`, seulement si un `.phash` est plus récent que `catalog.dbphash`). |
| `task o2switch:remove-cron` | Supprime le watchdog utilisateur de la crontab. |
| `task o2switch:rotate-key` | Génère, déploie et valide une nouvelle clé API 256 bits avec mise à jour dans `pass`. |
| `task o2switch:rotate-readonly-key` | Génère et déploie la clé lecture seule (supervision partagée), vérifie `GET /v1/health` = 200 et `POST /v1/catalog/reload` = 403. |
| `task o2switch:agent:start` | Démarre l'agent relais local connecté à o2switch pour VLC et le NAS Synology. |
| `task o2switch:agent:stop` | Arrête l'agent relais local. |
| `task o2switch:agent:restart` | Redémarre l'agent relais local. |
| `task o2switch:agent:status` | Affiche l'état et les métriques de requêtes relayées de l'agent local. |
| `task o2switch:agent:logs` | Suit en direct les journaux d'exécution de l'agent relais local. |
| `task o2switch:agent:demo` | Valide de bout en bout la reconnaissance d'un fichier réel du NAS via le Cloud. |

## ↩️ Revenir au démon TCP

Le démon écoute sur la socket Unix `~/dbsrt/run/dbsrt-server.sock`, car o2switch tue les listeners TCP en 5 à 20 s (voir l'[ADR socket Unix](../decisions/2026-10-31-dbsrt-server-unix-socket-o2switch.md)). Le binaire garde le mode TCP : sans `--unix-socket`, il écoute sur `--host`/`--port`. Pour revenir au TCP, par exemple si o2switch se met à tuer aussi les sockets Unix :

1. Déployer la version TCP de `deploy/o2switch/index.php` (révision antérieure à l'ADR) dans `~/public_html/dbsrt/index.php`, après l'avoir validée sur le serveur avec `php -l`.
2. Depuis la même révision, réinstaller le watchdog : `task o2switch:setup-cron`. Sinon, le cron relance le démon sur la socket.
3. Arrêter le démon en cours. `task o2switch:stop` ne suffit pas : il ne lit que `run/dbsrt-server.pid`, que ni la passerelle PHP ni le cron n'écrivent. Sur le serveur : `pkill -u "$(id -u)" -f 'bin/dbsrt-server --unix-socket'`. La passerelle le relance en TCP à la requête suivante.
4. Vérifier que les relances reprennent ou non : le compteur `dbsrt-server_spawns` du faultwatch reste le meilleur indicateur.
