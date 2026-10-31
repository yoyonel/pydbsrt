---
title: "Rapport de Campagne E2E VLC GUI & Plugin VLSub (Corpus NAS Exhaustif)"
date: 2026-10-20
description: "Rapport d'exécution bout-en-bout automatisée avec VLC 3.0.24 et plugin VLSub sous Xvfb :98 avec capture d'écran 1080p probatoire pour chaque média."
tags:
  - benchmarks
  - empirical
  - vlc
  - vlsub
  - e2e
---

# Rapport de Campagne E2E VLC GUI & Plugin VLSub (Corpus NAS Exhaustif)

> **Classification Factuelle** : **Empiriquement Prouvé**
> **Horodatage du rapport** : 2026-10-20 07:05:24
> **Statut de la campagne** : ✓ Complète
> **Progression** : **1 / 1** médias évalués (**0.0% de succès** à $\ge 90\%$ de confiance)

---

## 1. Métriques Globales d'Exécution

* **Vidéos évaluées** : 1 / 1
* **Reconnaissance réussie (Score $\ge 90\%$)** : **0** (0.0%)
* **Échecs ou non indexés** : 1
* **Chaîne d'exécution validée** : VLC 3.0.24 GUI + Marco WM (`Xvfb :98`) + `vlsub_dbsrt.lua` + `dbsrt-server` (Axum)
* **Preuves visuelles** : 1 capture PNG 1080p par vidéo avec OSD, liste interactive et console verte.

---

## 2. Tableau Exhaustif des Résultats & Preuves Visuelles

| # | Fichier Vidéo Testé | Média Apparié (pHash) | Score | Statut | Preuve Écran |
|---|---|---|---|---|---|
| 0001 | `pure_black.mp4` | `Non indexé` | **0.0%** | ❌ Échec | [Capture 0001](../screenshots/test_negative_e2e_black.png) |
