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

# Valider les Portails Web et l'API avec Playwright

Ce guide décrit la procédure pour exécuter la suite de tests end-to-end (E2E) automatisée sur les portails web (`monitoring.html`, `studio.html`, web top `/top`) et la passerelle de sécurité o2switch, générer un screencast vidéo de session, et vérifier formellement la couverture d'API à 100% ainsi que le profilage JavaScript V8 via CDP.

---

## 1. Prérequis & Environnement

Pour exécuter la recette E2E sans installer de dépendances système lourdes :

- **Navigateur** : Google Chrome ou Chromium installé sur le système (`/usr/bin/google-chrome` ou `/usr/bin/chromium`).
- **Gestionnaire Python** : `uv` installé pour isoler automatiquement les dépendances Playwright.
- **Daemon dbsrt** : Actif localement (`http://127.0.0.1:8765` ou port auto-découvert via `/tmp/dbsrt-server.json`), ou distant sur o2switch (`O2SWITCH_URL`).

Vérifiez la disponibilité du démon local avant de lancer le test :

```bash
task server:status
```

---

## 2. Exécution des Tests E2E Automatisés

La suite propose deux cibles déclaratives dans le Taskfile :

### Cible 1 : Validation locale (Daemon dbsrt-server)

```bash
task test:portal:e2e
```

### Cible 2 : Validation Cloud réelle (o2switch cPanel Gateway)

```bash
task test:portal:e2e:cloud
```

Cette commande orchestre les étapes suivantes :

1. Lancement de Chromium en mode Headless (résolution 1440×920).
2. Activation du profiler V8 CDP (`Profiler.startPreciseCoverage`) pour mesurer la couverture de code JavaScript.
3. Interception et traçage de 100% des requêtes réseau (`page.on("request")` et `page.on("response")`).
4. Parcours séquentiel des **8 groupes de scénarios use cases**.
5. Capture de copies d'écran HD par étape dans `target/e2e_portal/screenshots/`.
6. Enregistrement vidéo continu WebM transcodé automatiquement en MP4 (H.264 / AAC) lisible sur le web.
7. Calcul mathématique et affichage de la matrice de couverture d'API (`coverage_endpoints.json`) et du rapport V8 JS (`coverage_v8_js.json`).

---

## 3. Scopes Fonctionnels Validés par les 8 Groupes

Le script `scripts/e2e_portal_playwright.py` valide séquentiellement :

| Groupe | Composant Testé | Assertions Clés |
| :--- | :--- | :--- |
| **G1** | Passerelle Sécurisée & Auth | Formulaire de login HTML (401), rejet de clé invalide (.err-box), session cookie `dbsrt_session`, accès direct URL token (`?token=`), et déconnexion `/logout`. |
| **G2** | Stream SSE & Anti-Polling | Connexion persistance `/v1/monitoring/events`, header `X-Accel-Buffering: no`, invariant strict zéro-polling (0 appel vers `metrics`, `logs`, `latest`). |
| **G3** | Horloge de Cristian | Présence header `X-Server-Time-Ms`, calcul de la latence RTT et du jitter réseau dans le footer (`#sys-sync`). |
| **G4** | Monitoring 6 Vues & SWR | Navigation `#overview`, `#ingestion`, `#catalogue`, `#audit`, `#supervisor`, `#system`, cache SWR (<15 ms), pagination et drawer détail accordéon. |
| **G5** | Studio Couplages & Timeline | Recherche/filtres médias, sélection de piste SRT, clic sur cue bar dans la minimap timeline -> inspecteur `#inspector-card`, extraction brute `/v1/subtitles/{hash}`. |
| **G6** | Studio Catalogue & Playground | Consultation statut catalogue (`/v1/catalog/status`), hot-reload (`/v1/catalog/reload`), formulaires interactifs (`/v1/search`, `/v1/recognize`, `/v1/certify`, `/v1/retarget`, `/v1/autosync`, `/v1/download`). |
| **G7** | Studio Observabilité 5 Tiers | Sélection preset (`2-Blue Forever`), déclenchement matching `#btn-pipeline-match`, rendu des 5 tiers avec timings réels >0 ms (FFmpeg, WAN, MIH, SRT, Calage). |
| **G8** | Portail Web Top (`/top`) | Chargement de la vue top web et validation du rendu des tables de processus. |

---

## 4. Consultation des Livrables & Captures

Par défaut, les artefacts d'exécution sont générés dans `target/e2e_portal/` :

- **Rapport de Couverture d'API (100% routes)** : `target/e2e_portal/coverage_endpoints.json`
- **Rapport de Profilage V8 JavaScript** : `target/e2e_portal/coverage_v8_js.json`
- **Captures d'écran individuelles par groupe** : `target/e2e_portal/screenshots/*.png`
- **Screencast vidéo complet MP4 (H.264)** : `target/e2e_portal/screencasts/portal_e2e_screencast.mp4`

---

## 5. Dépannage des Erreurs Courantes

### Le test échoue sur `Executable doesn't exist at /home/.../ffmpeg-linux`
Playwright nécessite son binaire FFmpeg interne pour la capture vidéo. Installez-le avec :

```bash
uv run --with playwright playwright install ffmpeg
```

### Le test échoue avec `TimeoutError: Page.wait_for_selector`
Si le serveur local subit une contention disque lors du snapshot `dbsrt top --json`, assurez-vous que le binaire `dbsrt` est présent dans le `PATH` :

```bash
which dbsrt || cp target/release/dbsrt ~/.local/bin/dbsrt
```

### Le test Cloud (`--cloud`) échoue sur HTTP 503 / 429 (WAF Tiger Protect)

L'hébergeur o2switch applique un filtrage WAF strict (**Tiger Protect**) bloquant les navigateurs headless non interactifs venant du WAN public. Pour tester le serveur o2switch sans blocage :

- Utilisez la recette E2E SSH hermétique interne :
  ```bash
  task o2switch:e2e
  ```

- Ou effectuez les tests Playwright locaux ou sur l'appliance Raspberry Pi 4 :
  ```bash
  uv run --with playwright python scripts/e2e_portal_playwright.py --url http://<RPI4_IP>:8766 --key ""
  ```
