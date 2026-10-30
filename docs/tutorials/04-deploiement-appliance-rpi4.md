---
title: "Tutoriel : Déploiement d'une Appliance Raspberry Pi 4"
tags:

  - tutoriel
  - embarqué
  - rpi4
  - arm64
  - déploiement
---

# 🎓 Tutoriel : Déploiement d'une Appliance Raspberry Pi 4

Dans ce tutoriel pas à pas, vous allez apprendre à compiler, déployer et exécuter la pile `dbsrt-server` sur un boîtier **Raspberry Pi 4 (ARM64)** au sein de votre réseau local, transformant celui-ci en serveur domestique de synchronisation de sous-titres à haute disponibilité.

> [!NOTE]
> **Temps estimé** : 20 minutes.
> **Objectif** : Cross-compiler l'image Docker ARM64, transférer le conteneur sur le Raspberry Pi 4 via SSH et valider la réponse du démon HTTP distant.

---

## 1. Prérequis

- Un poste de développement Linux avec Docker et Taskfile installés.
- Un boîtier **Raspberry Pi 4** (2 Go RAM ou supérieur) sous Raspberry Pi OS 64-bit (Debian Bookworm aarch64).
- Un accès SSH configuré sans mot de passe (clé publique) vers votre Raspberry Pi :
  ```bash
  ssh pi@rpi4.local
  ```

- Docker installé sur le Raspberry Pi (`curl -fsSL https://get.docker.com | sh`).

---

## 2. Étape 1 : Configurer l'Inventaire Réseau

Dans le fichier d'inventaire ou de variables d'environnement (`.env` ou `ansible/inventory.yaml`), indiquez l'adresse IP ou le nom d'hôte de votre Raspberry Pi 4 :

```bash
export RPI4_HOST="pi@rpi4.local"
```

Testez la connectivité directe :

```bash
ssh "$RPI4_HOST" "uname -m"
```

Le retour doit afficher `aarch64`.

---

## 3. Étape 2 : Cross-Compiler l'Image ARM64 (`task rpi4:build`)

Sur votre machine de développement x86_64, lancez la compilation croisée du binaire Rust et la génération de l'image Docker ARM64 :

```bash
task rpi4:build
```

Cette étape utilise Docker Buildx pour générer une image minimale optimisée pour le microprocesseur ARM Cortex-A72 du Raspberry Pi 4.

---

## 4. Étape 3 : Transférer l'Image Docker vers le Pi (`task rpi4:push`)

Transférez l'image générée directement vers le démon Docker du Raspberry Pi à travers le tunnel SSH, sans passer par un registre public :

```bash
task rpi4:push
```

L'image est chargée instantanément dans le cache d'images local du Raspberry Pi.

---

## 5. Étape 4 : Démarrer la Stack sur le Raspberry Pi (`task rpi4:up`)

Lancez le démarrage des conteneurs distants :

```bash
task rpi4:up
```

Cette commande démarre sur le Raspberry Pi :

1. Le conteneur `dbsrt-server` (port HTTP `8765`).
2. Le conteneur PostgreSQL 17 dédié (`bktreedb`, port `5433`).

---

## 6. Étape 5 : Synchroniser le Catalogue et Tester la Reconnaissance

Synchronisez les fichiers de couplages et sous-titres certifiés vers l'appliance :

```bash
task rpi4:sync
```

Interrogez ensuite la sonde de statut à distance pour valider que le serveur ARM64 répond :

```bash
task rpi4:status
```

Exécutez le test de reconnaissance de démonstration envoyant une trame test de 200 octets :

```bash
task rpi4:demo
```

Le serveur distant répond avec le titre du média appairé et le code de statut HTTP 200 en moins de 15 millisecondes.

---

## 7. Étape 6 : Déployer et Lancer le Moniteur Temps Réel TUI (`dbsrt top`)

Pour superviser les métriques matérielles (CPU, RAM, débits réseau) et les tâches de traitement en direct sur l'appliance Raspberry Pi 4 :

1. Compilez le binaire CLI ARM64 avec parité Debian Bullseye GLIBC 2.31 :
   ```bash
   task rpi4:cli:build
   ```
2. (Optionnel) Validez le binaire ARM64 localement sur votre poste de développement via l'émulateur QEMU :
   ```bash
   task rpi4:cli:test
   ```
3. Déployez le binaire compilé `dbsrt` vers `~/.local/bin/dbsrt` sur le Raspberry Pi :
   ```bash
   task rpi4:cli:deploy
   ```
4. Lancez le moniteur plein écran interactif à distance via SSH :
   ```bash
   task rpi4:top
   ```

> [!NOTE]
> Pour compiler, déployer et lancer en une seule commande : `task rpi4:cli:build && task rpi4:cli:deploy && task rpi4:top`.
> Attention à ne pas confondre avec `task rpi4:build` (qui prépare l'image Docker du backend serveur `dbsrt-server`) ou `task rpi4:sync` (qui synchronise les bases de données et les fichiers SRT). Pour le guide complet, consultez [Comment compiler, tester, déployer et lancer le CLI dbsrt sur RPi4](../how-to/compiler-deployer-cli-top-sur-rpi4.md).

---

## 8. Alternative Légère : Serveur Standalone sans Docker (`rpi4:server:*`)

Si vous préférez exécuter `dbsrt-server` sans surcharge de conteneur Docker ni dépendance PostgreSQL (mode **Zero-DB pur** avec index `catalog.dbphash` en RAM) :

1. Déployez le binaire autonome ARM64 et installez l'unité de service systemd en mode utilisateur :
   ```bash
   task rpi4:server:deploy
   ```
2. Démarrez le service systemd :
   ```bash
   task rpi4:server:start
   ```
3. Contrôlez le statut du service et la télémétrie de l'API :
   ```bash
   task rpi4:server:status
   ```
4. Suivez les journaux journald en direct :
   ```bash
   task rpi4:server:logs
   ```
5. Validez le fonctionnement avec la requête test :
   ```bash
   task rpi4:server:demo
   ```

---

## 9. Étape 7 : Activer le Superviseur Autonome 24/7 & Backup NAS Synology

Pour que le Raspberry Pi 4 gère de manière totalement autonome la synchronisation du catalogue avec le Cloud o2switch et archive en continu les empreintes brutes vers le NAS Synology sans intervention manuelle :

1. Vérifiez que les partages NFS du Synology sont configurés dans `/etc/fstab` sur le Pi :
   ```text
   ds418.home:/volume1/video   /mnt/synology/video   nfs   rw,soft,timeo=30,nofail,_netdev   0   0
   ds418.home:/volume1/tvshow  /mnt/synology/tvshow  nfs   rw,soft,timeo=30,nofail,_netdev   0   0
   ```
2. Installez et activez le démon superviseur systemd utilisateur sur le Pi :
   ```bash
   task rpi4:supervisor:install
   task rpi4:supervisor:start
   ```
3. Suivez l'activité du superviseur en temps réel :
   ```bash
   task rpi4:supervisor:logs
   ```

Le superviseur exécute trois rôles en parallèle :

- **Toutes les 60 s** : Contrôle de vie et régulation adaptative CPU du batch o2switch (2 à 5 workers).
- **Toutes les 10 min** : Rapatriement différentiel atomique de `catalog.dbphash` (150 Mo, `mmap` pur, 0 fichier unitaire sur la SD).
- **Toutes les heures** : Sauvegarde des empreintes `*.phash` brutes vers le NAS Synology via montage NFS (relais en RAM, **0 écriture flash SD**).

---

## 10. Félicitations !

Votre Raspberry Pi 4 est désormais une **appliance domestique autonome** de reconnaissance et d'alignement de sous-titres :

- Vos lecteurs VLC sur le réseau local peuvent se connecter directement sur l'IP du Raspberry Pi (`http://<ip-rpi4>:8765`).
- L'empreinte mémoire reste inférieure à 120 Mo de RAM.

Pour configurer les clients VLC de votre réseau vers cette appliance, consultez :
👉 **[Comment synchroniser des sous-titres dans VLC](../how-to/synchroniser-sous-titres-vlc.md)**.
