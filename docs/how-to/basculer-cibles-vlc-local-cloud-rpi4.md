---
title: "Guide Pratique : Basculer les Cibles de Backend VLC (Local, Cloud o2switch, RPi4)"
tags:
  - howto
  - vlc
  - cloud
  - o2switch
  - rpi4
  - target
---

# 🛠️ Guide Pratique : Basculer les Cibles de Backend VLC (Local, Cloud o2switch, RPi4)

> Ce guide opérationnel détaille comment choisir, basculer et diagnostiquer facilement la cible de traitement et d'indexation pour le plugin VLC `vlsub_dbsrt.lua` entre un serveur 100% local, le relais Cloud o2switch et une appliance Raspberry Pi 4.

---

## 🎯 Problème

En tant qu'utilisateur du lecteur multimédia VLC :

- Vous souhaitez utiliser le **Cloud o2switch** (`https://2026-pydbsrt.fr/dbsrt`) pour profiter d'un catalogue distant sans consommer de mémoire RAM locale.
- Vous souhaitez basculer en mode **100% Local (Zero-DB)** lorsque vous êtes hors-ligne ou sur un réseau isolé (Air-Gapped).
- Vous souhaitez déporter les calculs sur une appliance **Raspberry Pi 4** connectée à votre réseau local.
- Vous vous demandez quel endpoint configurer dans le plugin VLC et pourquoi l'extension continue d'afficher `http://127.0.0.1:8765`.

---

## 💡 Comprendre l'Architecture : Pourquoi VLC Pointe Toujours sur `127.0.0.1:8765`

Le plugin VLC ne se connecte pas directement aux serveurs distants en HTTPS pour deux raisons d'ingénierie fondamentales :

```mermaid
flowchart TD
    VLC["🎬 Lecteur VLC (Lua)"] -->|TCP HTTP brut /tmp/dbsrt-server.json| Agent["🛡️ Passerelle Locale dbsrt (127.0.0.1:8765)"]
    Agent -->|1. Extraction pHash (ffmpeg local / GPU)| Disk["📁 Vidéos sur disques locaux / NAS"]
    Agent -->|2a. Mode Local| RAM["⚡ RAM locale (catalog.dbphash)"]
    Agent -->|2b. Mode Cloud| Cloud["☁️ Cloud o2switch (HTTPS TLS via REST)"]
    Agent -->|2c. Mode RPi4| RPi4["🍓 Appliance RPi4 (LAN)"]
```

1. **Absence de moteur TLS dans VLC Lua** : L'API réseau interne de VLC (`vlc.net.connect_tcp`) ne supporte que les sockets TCP brutes non chiffrées. Elle est incapable d'établir une liaison chiffrée HTTPS vers `https://2026-pydbsrt.fr/dbsrt`.
2. **Accès aux fichiers disques locaux** : Un serveur distant (o2switch ou RPi4 distant) n'a pas accès à vos montages disques ou NAS locaux pour y faire tourner `ffmpeg`. C'est l'agent local qui extrait les 5 trames clés pHash en 50 ms avant d'envoyer la signature compacte (200 octets).
3. **Découverte automatique zéro-configuration** : À chaque bascule, l'agent écrit le fichier `/tmp/dbsrt-server.json`. Le plugin VLC le relit automatiquement lors de chaque recherche sans intervention manuelle.

---

## 🚀 Solution : Basculer de Cible en 1 Commande (Taskfile)

Quatre cibles unifiées sont déclarées dans le `Taskfile.yml` pour piloter instantanément le routage :

### Option 1 : Basculer vers le Cloud o2switch (Recommandé)

Pour déléguer l'indexation au Cloud tout en laissant l'agent local extraire les empreintes :

```bash
task target:o2switch
```

*Effet* :

- Arrête tout serveur local autonome en cours.
- Démarre l'agent relais local sur `127.0.0.1:8765` connecté à `https://2026-pydbsrt.fr/dbsrt`.
- Dans VLC : Chaque résultat et message OSD portera le badge `[CLOUD o2switch]`.

### Option 2 : Basculer en 100% Local (Zero-DB / Air-Gapped)

Pour travailler de manière totalement autonome sans aucune dépendance réseau :

```bash
task target:local
```

*Effet* :

- Arrête l'agent relais.
- Démarre le serveur local `dbsrt-server` sur `127.0.0.1:8765` avec indexation en mémoire vive (`catalog.dbphash`) et base locale PostgreSQL (si active).
- Dans VLC : Chaque résultat portera le badge `[LOCAL]`.

### Option 3 : Basculer vers l'Appliance Raspberry Pi 4

Pour déporter les requêtes sur une Raspberry Pi 4 de votre réseau local :

```bash
# Avec le nom d'hôte par défaut (rpi4.local)
task target:rpi4

# Ou en spécifiant l'adresse IP de votre Raspberry Pi
RPI4_HOST="rpi4-hostname" task target:rpi4
```

*Effet* :

- L'agent local relaie les 200 octets pHash vers l'appliance RPi4 via le réseau local.

---

## 🔍 Vérifier le Mode Actif (`task target:status`)

Pour contrôler immédiatement quel mode est actif et quel PID écoute pour VLC :

```bash
task target:status
```

Exemple de sortie :

```text
=== Active VLC Backend Endpoint Status ===
• Current Mode     : CLOUD_O2SWITCH (Relay -> https://2026-pydbsrt.fr/dbsrt)
• VLC Discovery    : /tmp/dbsrt-server.json
  {"host":"127.0.0.1","pid":2112861,"port":8765,"url":"http://127.0.0.1:8765","version":"0.1.0"}
• VLC Config Target: http://127.0.0.1:8765
```

---

## 🖥️ Configuration Manuelle Directe dans VLC ("Show config")

Si vous souhaitez forcer une URL personnalisée sans passer par le terminal :

1. Dans VLC, lancez une vidéo et ouvrez le menu **Vue** > **VLSub (dbsrt Enhanced)**.
2. Cliquez sur le bouton **Show config** en bas à gauche de la boîte de dialogue.
3. Le champ **dbsrt Server Endpoint** affiche l'URL courante :
   - Valeur standard : `http://127.0.0.1:8765` (laisse la passerelle locale router automatiquement).
   - RPi4 direct (si la RPi a accès aux mêmes fichiers multimédias via NFS/SMB) : `http://rpi4.local:8765`.
4. Cliquez sur **Save**.

---

## 👁️ Identification Visuelle dans VLC

Une fois la recherche déclenchée (**Recherche par pHash**), vous identifiez immédiatement où le calcul a eu lieu grâce aux trois indicateurs visuels :

1. **Préfixe dans la liste des résultats** :
   - `[CLOUD o2switch] Titre.Du.Film.srt [FR] (catalog)`
   - `[LOCAL] Titre.Du.Film.srt [FR] (catalog)`
2. **Bannière d'état sous la liste** :
   - `[CLOUD o2switch] MATCH: 1 subtitle(s) found (ref: Andor, 72ms)`
   - `[LOCAL] MATCH: 1 subtitle(s) found (ref: Andor, 1.2ms)`
3. **Incrustation OSD sur la vidéo** :
   - `✓ [CLOUD o2switch] Subtitles loaded: Titre.srt (+0.0ms)`
   - `✓ [LOCAL] Subtitles loaded: Titre.srt (+0.0ms)`

---

## 📖 Étalon de Qualification & Preuves d'Essais

Pour consulter le rapport de tests réels verbatim, les journaux d'exécution `cvlc` et les métriques de validation sur les trois plateformes, référez-vous au document étalon :

* [Référence Étalon : Sanity Check Multi-Cibles VLC](../reference/2026-10-28-etalon-sanity-check-multi-cibles-vlc.md)
