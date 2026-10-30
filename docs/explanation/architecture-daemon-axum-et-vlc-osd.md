---
title: "Explication : Architecture Démon Axum & VLC OSD"
tags:

  - explication
  - architecture
  - vlc
  - lua
  - axum
  - osd
---

# 💡 Explication : Architecture Démon Axum & Intégration VLC OSD

Cet article détaille l'architecture découplée entre le lecteur multimédia **VLC** (moteur C/Qt et scripts Lua) et le microservice **`dbsrt-server`** (Rust Axum), ainsi que les choix d'ingénierie garantissant une réactivité fluide de l'interface utilisateur sans blocage.

---

## 1. La Contrainte Critique du Moteur Lua de VLC

L'environnement de script de VLC impose des contraintes architecturales sévères :

1. **Exécution synchrone sur le thread d'interface Qt** : Dans l'architecture de VLC, les extensions Lua de type dialogue (`vlsub_dbsrt.lua`) s'exécutent dans le contexte direct de la boucle d'événements de l'interface graphique.
2. **Risque de gel de l'application (Hang / Unresponsive)** : Tout appel bloquant (tel qu'un calcul lourd d'empreintes vidéo en Lua ou un appel réseau HTTP qui attend un timeout de 30 secondes) fige instantanément la fenêtre de VLC, provoquant une invite de plantage du gestionnaire de fenêtres du système d'exploitation.

```mermaid
flowchart LR
    subgraph VLC ["VLC Media Player (C / Qt)"]
        UI["Interface Graphique Qt"]
        Lua["Script Lua vlsub_dbsrt"]
        OSD["Moteur OSD (On-Screen Display)"]
        UI <--> Lua
        Lua --> OSD
    end

    subgraph Daemon ["dbsrt-server (Rust Axum)"]
        HTTP["Endpoint Asynchrone Tokio /v1/autosync"]
        Engine["Moteur pHash SIMD & Cache MIH"]
        HTTP <--> Engine
    end

    Lua <-->|"HTTP REST Asynchrone (Timeout < 500ms)"| HTTP
```

---

## 2. Le Découpage en 3 Couches

Pour éliminer définitivement tout risque de gel de l'interface, le système délègue tout le travail lourd au démon externe :

### 1. La Couche Découverte & Configuration Dynamique
Au démarrage de `dbsrt-server`, celui-ci écrit un fichier éphémère atomique sous `/tmp/dbsrt-server.json` consignant son port d'écoute et son jeton de session.
Le script Lua lit ce fichier pour s'auto-configurer sans imposer à l'utilisateur de configurer manuellement une adresse IP ou un port.

### 2. Le Protocole HTTP Asynchrone & Timeouts Stricts
Tous les appels HTTP initiés depuis le script Lua vers le démon appliquent un **timeout strict de 500 millisecondes**. Si le démon ne répond pas immédiatement, la requête est interrompue, l'interface graphique est immédiatement libérée et un message d'information non-bloquant est affiché.

### 3. Les Notifications OSD Natives (On-Screen Display)
Plutôt que d'obliger l'utilisateur à garder une fenêtre de dialogue ouverte au-dessus de la vidéo, le plugin utilise le canal OSD natif de VLC (`vlc.osd.message(...)`). Les informations de recalage, la confiance et le décalage temporel (\( \Delta t \)) s'affichent directement sur l'image vidéo en surimpression pendant 3 secondes, sans perturber le visionnage plein écran.

---

## 3. Dégradation Gracieuse & Mode Hors-Ligne

Si `dbsrt-server` n'est pas démarré sur la machine locale :

- L'extension détecte immédiatement l'absence de socket active sur `/tmp/dbsrt-server.json`.
- Le plugin bascule gracieusement en mode standard OpenSubtitles (téléchargement classique de sous-titres par hash de fichier).
- Aucun message d'erreur bloquant n'interrompt la lecture de l'utilisateur.

## 4. Authentification en Profondeur (Déploiement o2switch)

Sur o2switch, le démon écoute sur `127.0.0.1`, derrière la passerelle PHP `index.php`. Historiquement, seule la passerelle vérifiait la clé API : tout processus local de l'hébergement mutualisé pouvait appeler le démon sans clé.

Le démon applique donc lui-même la clé, via le middleware Axum `auth::require_api_key` (option `--api-key-file`) :

- **Deux barrières indépendantes** : la passerelle filtre le trafic public (rate limiting, cookie de session, clé lecture seule) ; le démon refuse tout appel loopback sans clé maître.
- **Sondes publiques** : `/health` et `/v1/health` restent ouverts, car le watchdog cron et la passerelle les interrogent sans clé.
- **Rotation sans redémarrage** : le fichier de clés est relu à chaque requête. Un fichier illisible échoue en mode fermé (`503`), jamais en mode ouvert.
- **Comparaison à temps constant** : la réponse ne révèle pas le préfixe correct d'une clé.

La **clé lecture seule** sert aux accès exposés hors du poste opérateur (QR Code, web-top sur smartphone). Elle ne quitte jamais la passerelle : celle-ci limite cette clé aux lectures de supervision (`403` sinon), retire les en-têtes d'authentification du client et injecte la clé maître vers le démon. Une fuite du QR Code expose donc la supervision, pas le catalogue ni les actions d'écriture.

Sans `--api-key-file` (usage local, VLC sur le poste), l'API reste ouverte : le socket n'écoute que sur la boucle locale de la machine de l'utilisateur.
