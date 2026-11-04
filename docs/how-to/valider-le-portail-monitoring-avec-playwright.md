---
title: "Valider le Portail de Monitoring avec Playwright"
description: "Procédure d'exécution et de validation end-to-end automatisée du portail de monitoring dbsrt à l'aide de Playwright, incluant la capture vidéo et les assertions multi-vues."
tags:
  - How-To
  - Testing
  - Playwright
  - E2E
  - Monitoring
---

# Valider le Portail de Monitoring avec Playwright

Ce guide décrit la procédure pour exécuter la suite de tests end-to-end (E2E) automatisée sur le portail web de monitoring embarqué (`dbsrt-server`), générer un screencast vidéo de session et vérifier formellement la conformité des 13 scopes de fonctionnalités héritées.

---

## 1. Prérequis & Environnement

Pour exécuter la recette E2E sans installer de dépendances système lourdes :

- **Navigateur** : Google Chrome ou Chromium installé sur le système (`/usr/bin/google-chrome` ou `/usr/bin/chromium`).
- **Gestionnaire Python** : `uv` installé pour isoler automatiquement les dépendances Playwright.
- **Daemon local dbsrt** : Le serveur dbsrt doit être actif sur le port local par défaut (`http://127.0.0.1:8765`).

Vérifiez la disponibilité du démon avant de lancer le test :

```bash
task server:status
# ou pour le service utilisateur systemd :
task client:linux:status
```

---

## 2. Exécution du Test E2E Automatisé

La suite complète est déclarée dans le Taskfile et s'invoque via une commande unique :

```bash
task test:portal:e2e
```

Cette commande orchestre les étapes suivantes :

1. Téléchargement à la volée de Playwright et de son module d'enregistrement vidéo FFmpeg.
2. Lancement de Chrome en mode Headless (résolution 1400×900).
3. Parcours complet des 8 étapes d'automatisation et assertion des 13 scopes fonctionnels.
4. Découpage et capture de 9 captures d'écran HD.
5. Finalisation de l'enregistrement vidéo WebM et transcodage automatique en MP4 (H.264 / AAC) lisible sur le web.

---

## 3. Scopes Fonctionnels Validés par le Scénario

Le script `scripts/e2e_portal_playwright.py` valide séquentiellement :

| Étape | Scope Testé | Assertions Clés |
| :--- | :--- | :--- |
| **1** | En-tête & Contrôles | Présence du titre, pastille d'état `Actif`, bascule pause/reprise (`⏸` ➔ `▶` ➔ `⏸`), affichage de la modal d'authentification API. |
| **2** | Vue d'Ensemble (`#overview`) | Chargement asynchrone des 5 tuiles d'état système. |
| **3** | Ingestion & Décodage (`#ingestion`) | Bannière TUI 4 lignes, métriques de la file batch, sous-onglets Tâches (`tab-jobs`), Statistiques réelles (`tab-stats`), Récents (`tab-completed`) avec dépliement de l'accordéon détail et pagination, et Sources (`tab-sources`). |
| **4** | Catalogue (`#catalogue`) | Affichage de l'explorateur multi-sources et métadonnées pHash. |
| **5** | Audit (`#audit`) | Vérification du cartouche de run, boutons de contrôle et indicateurs de précision. |
| **6** | Superviseur (`#supervisor`) | Affichage de la carte d'état RPi4, vCPU LVE et terminal de logs. |
| **7** | Système (`#system`) | Rendu de l'arbre d'informations système et matériel fastfetch. |
| **8** | Pied de Page | Vérification des métriques hôte, RAM, Uptime, latence RTT et âge de synchronisation. |

---

## 4. Consultation des Livrables & Captures

Par défaut, les artefacts d'exécution sont générés dans `target/e2e_portal/` :

- **Captures d'écran individuelles** : `target/e2e_portal/screenshots/*.png`
- **Screencast vidéo MP4 (H.264)** : `target/e2e_portal/screencasts/portal_e2e_screencast.mp4`
- **Screencast vidéo WebM** : `target/e2e_portal/screencasts/portal_e2e_screencast.webm`

Pour rediriger le dossier de sortie vers un répertoire personnalisé :

```bash
PORTAL_E2E_OUTPUT_DIR=/tmp/mon_rapport task test:portal:e2e
```

---

## 5. Dépannage des Erreurs Courantes

### Le test échoue sur `Executable doesn't exist at /home/.../ffmpeg-linux`
Playwright nécessite son binaire FFmpeg interne pour la capture vidéo. Installez-le avec :

```bash
uv run --with playwright playwright install ffmpeg
```

### Le test échoue avec `TimeoutError: Page.wait_for_selector`
Si le serveur local subit une contention disque lors du snapshot `dbsrt top --json`, augmentez le délai d'attente ou assurez-vous que le binaire `dbsrt` est présent dans le `PATH` :

```bash
which dbsrt || cp target/release/dbsrt ~/.local/bin/dbsrt
```
