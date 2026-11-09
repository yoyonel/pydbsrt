---
title: "Architecture du Portail Sources Médias & Diagnostic Multi-Environnements"
tags:
  - architecture
  - portal
  - media-sources
  - diataxis
  - o2switch
  - rpi4
---

# 💡 Architecture du Portail Sources Médias & Diagnostic Multi-Environnements

Ce document explicatif détaille les principes de conception, les arbitrages d'architecture et la séparation stricte des responsabilités mis en œuvre dans la page **Sources Médias (Physique)** du portail web `pydbsrt`.

---

## 1. Séparation Conceptuelle : Réalité Physique vs État Base de Données

Un biais fréquent dans les backoffices multimédia consiste à confondre la **réalité physique du stockage** (l'existence effective de conteneurs `.mkv` et `.mp4` sur un disque ou un flux réseau) avec **l'état logique d'indexation** (les enregistrements présents dans la table `medias` de PostgreSQL ou dans un catalogue binaire).

| Dimension | Qu'est-ce que c'est ? | Support opérationnel | Question clé |
| :--- | :--- | :--- | :--- |
| **Réalité Physique & Réseau** | Les conteneurs vidéo stockés sur disques locaux, partages CIFS, serveurs FTP ou passerelles HTTP. | Disque local (`data/snippets`), CIFS NAS/Freebox, FTP WAN Synology, Passerelle RPi4. | *« Le fichier vidéo est-il accessible et lisible depuis cette machine ici et maintenant ? »* |
| **État Logique & Métadonnées** | Les lignes indexées, les coupes pHash calculées et les couplages certifiés. | PostgreSQL 17 (`medias`, `frames`), catalogue Zero-DB `catalog.dbphash`, `couplings.json`. | *« La vidéo a-t-elle été scannée, découpée et enregistrée en base de données ? »* |

> [!IMPORTANT]
> **Problème résolu :**
> Lorsque PostgreSQL était vide, l'ancienne interface affichait trompeusement « 0 Actifs », alors même que le disque local contenait physiquement 2 758 vidéos de banc d'essai et que le NAS Synology exposait 614 vidéos en streaming FTP.
> La page **Sources Médias** se consacre **exclusivement** à l'accessibilité physique des fichiers, sans aucune dépendance vis-à-vis des tables de la base de données.

---

## 2. Topologie Réseau et Profils d'Exécution Multi-Environnements

Le portail `pydbsrt` est déployé et exploité depuis trois environnements fondamentalement différents :

```mermaid
flowchart TD
    subgraph LOCAL["💻 Poste de Travail Local"]
        L_CIFS["Montages CIFS directs (/media/.../Synology, /media/.../Freebox)"]
        L_FS["Dépôt local de banc d'essai (data/snippets)"]
        L_WAN["Flux distant WAN FTP (en mobilité / au bureau)"]
    end

    subgraph RPI4["🍓 Appliance RPi4 (Maison)"]
        R_CIFS["Montages CIFS locaux (/mnt/freebox/*)"]
        R_GW["Passerelle HTTP Range locale (127.0.0.1:18089)"]
        R_FTP["FTP LAN Freebox (mafreebox.freebox.fr)"]
    end

    subgraph O2SWITCH["🌐 Cloud o2switch (Serveur Distant)"]
        O_FTP_MOVIES["Streaming WAN FTP Synology Movies (:21)"]
        O_FTP_SHOWS["Streaming WAN FTP Synology TVShows (:21)"]
        O_GW["Passerelle HTTP WAN Freebox (:80 -> RPi4)"]
    end
```

### Le non-sens des faux négatifs locaux sur un serveur cloud
Sur un serveur cloud distant (`o2switch`), tester l'existence d'un chemin de montage local comme `/media/<user>/Synology/video` renvoie systématiquement une erreur. Afficher un badge rouge « Inaccessible » pour un chemin qui n'a aucun sens d'exister sur un serveur distant constitue une pollution visuelle et fonctionnelle.

### Solution : Canaux paramétriques et profils d'environnement
Chaque profil d'accès déclare ses environnements cibles (`envs: ["local", "rpi4", "o2switch"]`) :
1. **`🌐 Cloud o2switch`** : Uniquement les flux distants WAN (FTP Synology et Passerelle HTTP). Les chemins locaux CIFS et les snippets sont filtrés et masqués.
2. **`💻 Poste Local`** : Montages locaux de dev, snippets de banc d'essai et relais distants de secours.
3. **`🍓 Appliance RPi4`** : Partages Freebox locaux montés dans `/mnt/freebox/` et service gateway local.
4. **`🔍 Diagnostic Brut (Tous)`** : Vue exhaustive permettant de sonder l'intégralité des 12 canaux déclarés.

---

## 3. Architecture Asynchrone Non-Bloquante (< 1 ms Initial)

Tester des accès réseau distants (FTP avec négociation TLS/credentials, ou passerelle HTTP avec timeout de 1,5 s) bloque tout serveur synchrone si l'évaluation est faite au chargement de la page :

```mermaid
sequenceDiagram
    autonumber
    actor User as Navigateur Web
    participant Server as scripts/portal_server.py
    participant Remote as NAS FTP / Gateway HTTP

    Note over User,Server: Étape 1 : Chargement Initial Instantané (< 1 ms)
    User->>Server: GET /sources.html
    Server-->>User: HTML + Structure JSON (/api/sources)
    Note over User: Rendu immédiat des cartes (Statut "Vérification...")

    Note over User,Remote: Étape 2 : Diagnostic Parallèle Asynchrone
    par Sonde Synology Movies
        User->>Server: GET /api/probe?source=synology-video&profile=wan_ftp
        Server->>Remote: Négociation FTP + Listing
        Remote-->>Server: 541 fichiers (300 ms)
        Server-->>User: JSON {status: "online", files_count: 541}
        Note over User: Mise à jour réactive du badge 🟢
    and Sonde Freebox HTTP
        User->>Server: GET /api/probe?source=freebox-disks&profile=wan_http
        Server->>Remote: HTTP GET /health
        Remote-->>Server: Timeout (2 500 ms)
        Server-->>User: JSON {status: "offline", reason: "RPi4 offline"}
        Note over User: Mise à jour réactive du badge 🔴
    end
```

---

## 4. Machine à États de l'Accordéon (Auto-Collapse)

Pour maximiser l'efficacité de lecture, chaque bloc de gisement adapte automatiquement son état replié/déplié en fonction des résultats du diagnostic :

```mermaid
stateDiagram-v2
    [*] --> InitialState: Chargement métadonnées

    state InitialState {
        direction LR
        Déplié: Bloc déplié par défaut
    }

    InitialState --> Probing: Lancement des sondes filtrées

    state Probing {
        Pending: Canaux en statut "Vérification..."
    }

    Probing --> AtLeastOneOnline: Au moins un canal Accessible 🟢
    Probing --> AllOffline: Tous les canaux Inaccessibles 🔴

    AtLeastOneOnline --> ExpandedState: Reste Déplié ▾
    AllOffline --> CollapsedState: Auto-Replié ▴ (Badge rouge récapitulatif)

    ExpandedState --> ManualToggle: Clic utilisateur
    CollapsedState --> ManualToggle: Clic utilisateur
    ManualToggle --> [*]: Verrouillage choix utilisateur (userToggled = true)
```

1. **Règle nominale** : Si un gisement possède au moins un canal actif (ex: FTP Synology accessible alors que CIFS local est éteint), le bloc reste grand ouvert (`uncollapsed`).
2. **Règle de repli** : Si **tous** les canaux d'un gisement sont hors service, le bloc se compacte automatiquement pour ne pas encombrer l'écran, affichant un badge de synthèse rouge.
3. **Priorité utilisateur** : Dès que l'utilisateur clique manuellement sur un en-tête ou utilise « Tout Déplier » / « Tout Replier », l'automatisme s'efface devant le choix explicite de l'opérateur.
