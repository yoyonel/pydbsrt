---
title: "Guide Pratique : Extraire et Évaluer les Empreintes Acoustiques aHash"
tags:
  - howto
  - audio
  - ahash
  - benchmark
  - extraction
  - o2switch
---

# 🛠️ Guide Pratique : Extraire et Évaluer les Empreintes Acoustiques aHash

Ce guide opérationnel décrit comment générer des empreintes acoustiques spectrales Haitsma-Kalker (`.ahash` v1.0), ingérer des médias en mode dual-hash et exécuter le banc métrologique de corrélation croisée.

---

## 1. Contexte & Problématique

L'empreinte visuelle pHash présente des limites sur les séquences visuellement statiques (écrans noirs, cartons de titre identiques, génériques récurrents). L'empreinte acoustique `.ahash` résout cette ambiguïté en échantillonnant le spectre audio à \(86.13\text{ fps}\) avec un pas de \(11.6\text{ ms}\), assurant un calage temporel sub-secondebit-exact.

---

## 2. Extraire une Empreinte Acoustique Unitaire

### Via la commande Taskfile
Pour extraire l'empreinte `.ahash` d'un fichier vidéo ou audio local :

```bash
task audio:export-ahash -- --media /data/video.mkv --output data/__AHASH__/video.ahash
```

### Via la CLI Standalone `dbsrt`
La commande supporte les fichiers locaux et les flux de streaming réseau distants :

```bash
# Extraction locale complète
dbsrt export-ahash --media /chemin/vers/film.mkv --output data/__AHASH__/film.ahash

# Extraction sur extrait temporel (ex: 30 secondes à partir de 1 minute 30)
dbsrt export-ahash \
    --media /chemin/vers/film.mkv \
    --output data/__AHASH__/extrait.ahash \
    --start-sec 90.0 \
    --duration 30.0

# Extraction directe depuis un flux FTP NAS en streaming (zéro stockage vidéo local)
dbsrt export-ahash \
    --media "ftp://user:pass@nas.local:21/tvshow/ep01.mkv" \
    --output data/__AHASH__/ep01.ahash
```

---

## 3. Ingestion Conjointe Dual-Hash (pHash + aHash)

Pour indexer un nouveau média dans le catalogue tout en générant simultanément l'empreinte vidéo et l'empreinte audio :

```bash
task media:add -- /data/medias/film.mkv --ahash-dir data/__AHASH__
```

Le processus exécute :

1. L'extraction du pHash 64-bit et l'enregistrement du couplage SRT.
2. L'extraction de l'empreinte `.ahash` dans `data/__AHASH__/film.ahash`.
3. Le rechargement atomique à chaud du démon `dbsrt-server`.

> [!NOTE]
> Si la vidéo ne comporte aucune piste audio (vidéo muette), le calcul aHash est ignoré sans interrompre l'ingestion du pHash visuel.

---

## 4. Moisson Massive en Streaming sur o2switch

Sur le serveur cloud mutualisé o2switch, le worker autonome peut être lancé pour extraire les empreintes `.ahash` sur toute la bibliothèque du NAS sans transiter par le disque local :

```bash
# Lancer l'extraction audio batch sur toutes les cibles (séries + films)
task o2switch:batch:ahash:all

# Surveiller la progression et la charge CPU du worker
task o2switch:batch:status

# Suivre les journaux d'extraction en direct
task o2switch:batch:logs
```

---

## 5. Exécuter le Banc d'Évaluation Métrologique

Pour évaluer la précision temporelle, la discrimination inter-médias et le débit d'exécution POPCNT sur l'ensemble du corpus d'empreintes :

```bash
# Exécution du benchmark complet avec export télémétrique
task bench:ahash:matching
```

### Paramètres Avancés
Pour affiner l'évaluation ou tester un sous-ensemble d'échantillons :

```bash
target/release/dbsrt bench-ahash \
    --ahash-dir data/__AHASH__ \
    --samples 200 \
    --cross-media-trials 500 \
    --max-distance 12.5 \
    --json-out data/reports/eval_ahash_matching.json
```

---

## 6. Interpréter les Résultats du Benchmark

Le rapport console et le fichier JSON de télémétrie (`data/reports/eval_ahash_matching.json`) fournissent les indicateurs clés :

1. **Taux de Match Exact (\(\le 25\text{ ms}\))** : Pourcentage de requêtes dont le décalage calculé \(\Delta t\) est exact à \(\pm 2\) trames près (nominal : \(100.0\%\)).
2. **Erreur Médiane & Percentile 95** : Écart temporel absolu vis-à-vis de la vérité terrain (nominal : \(0.00\text{ ms}\)).
3. **Marge SNR** : Différence entre la distance de Hamming au pic d'alignement et la distance moyenne sur le bruit non corrélé (nominal : \(\ge 28\text{ bits}\)).
4. **Faux Positifs Croisés (Axe 2)** : Nombre de correspondances erronées détectées sur des paires de médias distincts (nominal : \(0 / 500\) pour un seuil \(\le 12.5\text{ bits}\)).
5. **Débit POPCNT (Axe 3)** : Vitesse brute de balayage exprimée en heures de média analysées par seconde par cœur CPU (nominal : \(> 10\,000\,000\text{ h/s/cœur}\)).
