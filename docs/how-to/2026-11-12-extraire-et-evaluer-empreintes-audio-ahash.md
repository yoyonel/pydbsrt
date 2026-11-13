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

Ce guide opérationnel décrit comment générer des empreintes acoustiques spectrales Haitsma-Kalker (`.ahash` v1.0), maintenir la convention canonique `<moviehash_16hex>[.a<k>].86.1fps.ahash` + sidecar `.json`, ingérer des médias en mode dual-hash et exécuter les bancs métrologiques.

---

## 1. Contexte & Convention de Nommage

L'empreinte visuelle pHash présente des limites sur les séquences visuellement statiques (écrans noirs, cartons de titre identiques, génériques récurrents). L'empreinte acoustique `.ahash` résout cette ambiguïté en échantillonnant le spectre audio à \(86.125\text{ fps}\) (\(5\,512.5\text{ Hz} / 64\), pas de \(11.6\text{ ms}\)), assurant un calage temporel sub-seconde exact.

Chaque empreinte acoustique produite dans `data/__AHASH__/` respecte la convention canonique :

- **Fichier binaire** : `<moviehash_16hex>[.a<k>].86.1fps.ahash` (ex. `8e245d9679d31e12.86.1fps.ahash` pour la piste par défaut, ou `8e245d9679d31e12.a1.86.1fps.ahash` pour la piste relative `0:a:1`).
- **Sidecar typé (`AhashSidecar` v1)** : `<moviehash_16hex>[.a<k>].json` stockant `media_name`, `moviehash`, la piste décodée (`selected_audio_stream`) et l'inventaire complet des pistes audio du conteneur (`container_audio_streams`).

---

## 2. Extraire une Empreinte Acoustique Unitaire

### Via la commande Taskfile (recommandé)
Pour extraire l'empreinte `.ahash` et son sidecar `.json` d'un fichier vidéo ou audio :

```bash
# Extraction vers le chemin canonique <moviehash_16hex>.86.1fps.ahash
task audio:export-ahash -- \
    --media /data/video.mkv \
    --output data/__AHASH__/8e245d9679d31e12.86.1fps.ahash

# Extraction d'une piste audio spécifique (-map 0:a:1) sur une fenêtre de 30s
task audio:export-ahash -- \
    --media /data/video.mkv \
    --audio-track 1 \
    --start-sec 90.0 \
    --duration 30.0 \
    --output data/__AHASH__/8e245d9679d31e12.a1.86.1fps.ahash
```

---

## 3. Ingestion Conjointe Dual-Hash (pHash + aHash)

Pour indexer un nouveau média dans le catalogue tout en générant simultanément l'empreinte vidéo et l'empreinte audio :

```bash
task media:add -- /data/medias/film.mkv --ahash-dir data/__AHASH__
```

Le processus exécute :

1. L'extraction du pHash 64-bit et l'enregistrement du couplage SRT.
2. L'extraction de l'empreinte `data/__AHASH__/<moviehash_16hex>.86.1fps.ahash` et de son sidecar `<moviehash_16hex>.json`.
3. Le rechargement atomique à chaud du démon `dbsrt-server` (`POST /v1/catalog/reload`).

!!! note "Conteneurs sans piste audio"
    Si la vidéo ne comporte aucune piste audio (vidéo muette), le calcul aHash est ignoré sans interrompre l'ingestion du pHash visuel.

---

## 4. Moisson Massive en Streaming & Backfill Sidecars

Sur le serveur cloud mutualisé o2switch ou depuis le poste local, les tâches dédiées orchestrent l'extraction en streaming et la mise à niveau sans perte des sidecars :

```bash
# Lancer l'extraction audio batch sur toutes les cibles (séries + films) sur o2switch
task o2switch:batch:ahash:all

# Rattraper et vérifier par sondage Hamming les sidecars JSON (<stem>.json)
task ahash:backfill-sidecars -- --limit 100 --workers 4

# Migrer sans perte (0 recalcul FFT) d'anciens fichiers nommés en clair vers <moviehash_16hex>
task ahash:migrate-moviehash -- --dry-run
```

---

## 5. Exécuter les Bancs d'Évaluation Métrologique & Multimodale

Pour évaluer la précision temporelle, la discrimination inter-médias, l'indexation Coarse-to-Fine et le consensus multimodal :

```bash
# 1. Benchmark métrologique complet (Axe 1 temporel, Axe 2 faux-positifs, Axe 3 débit POPCNT)
task bench:ahash:matching

# 2. Personnaliser les paramètres du benchmark métrologique
task bench:ahash:matching -- \
    --ahash-dir data/__AHASH__ \
    --samples 200 \
    --cross-media-trials 500 \
    --max-distance 12.5 \
    --json-out data/reports/eval_ahash_matching.json

# 3. Matrice comparative Linear O(N) vs MIH-16 vs MIH-32 et Coarse-to-Fine (K=1..31)
task bench:ahash:mih:matrix
task bench:ahash:coarse-to-fine

# 4. Benchmark multimodal multi-fenêtres à l'échelle (pHash vs aHash C2F K=16) et audit
task bench:multimodal:scale -- --limit 50
task eval:multimodal:audit
```

---

## 6. Interpréter les Résultats du Benchmark

Le rapport console et le fichier JSON de télémétrie (`data/reports/eval_ahash_matching.json`) fournissent les indicateurs clés :

1. **Taux de Match Exact (\(\le 25\text{ ms}\))** : Pourcentage de requêtes dont le décalage calculé \(\Delta t\) est exact à \(\pm 2\) trames près (nominal : \(100.0\%\)).
2. **Erreur Médiane & Percentile 95** : Écart temporel absolu vis-à-vis de la vérité terrain (nominal : \(0.00\text{ ms}\)).
3. **Marge SNR** : Différence entre la distance de Hamming au pic d'alignement et la distance moyenne sur le bruit non corrélé (nominal : \(\ge 28\text{ bits}\)).
4. **Faux Positifs Croisés (Axe 2)** : Nombre de correspondances erronées détectées sur des paires de médias distincts (nominal : \(0 / 500\) pour un seuil \(\le 12.5\text{ bits}\)).
5. **Débit POPCNT (Axe 3)** : Vitesse brute de balayage exprimée en heures de média analysées par seconde par cœur CPU (nominal : \(> 10\,000\,000\text{ h/s/cœur}\)).

---

## 7. Interroger le Démon via l'API REST `/v1/search/ahash`

Une fois le serveur démarré (`task server:daemon` sur le port `8765`), les clients (lecteurs VLC, agents relais ou scripts) peuvent soumettre un extrait sub-fingerprint (`8..=8192` trames, corps max `256 KiB`, `limit` dans `1..=50`) pour obtenir l'offset milliseconde exact $\Delta t$ :

```bash
# Exemple de requête HTTP sur le port 8765 (hashes u64, fingerprint hex ou ahash_bytes base64)
curl -s -X POST http://127.0.0.1:8765/v1/search/ahash \
  -H "Content-Type: application/json" \
  -d '{
    "fingerprint": [
      "0x123456789abcdef0",
      "0x23456789abcdef01",
      "0x3456789abcdef012",
      "0x456789abcdef0123",
      "0x56789abcdef01234",
      "0x6789abcdef012345",
      "0x789abcdef0123456",
      "0x89abcdef01234567"
    ],
    "audio_fps": 86.125,
    "max_distance": 14.0,
    "limit": 3
  }'
```

Le serveur répond avec le meilleur candidat reconnu (`file_path: "data/__AHASH__/<moviehash_16hex>.86.1fps.ahash"`), la marge SNR et le décalage calculé en millisecondes (`offset_ms`).
Pour recharger à chaud de nouveaux fichiers `.ahash` sans redémarrer le démon :

```bash
task server:reload
# ou directement : curl -s -X POST http://127.0.0.1:8765/v1/catalog/reload
```

---

## 8. Utilisation depuis l'Extension VLC (`Search by aHash`)

L'extension VLC `vlsub_dbsrt.lua` intègre le bouton **Search by aHash** situé sous *Search by pHash* :

1. Ouvrez votre vidéo dans VLC et lancez l'extension via **Vue > VLSub (dbsrt Enhanced)**.
2. Cliquez sur le bouton **Search by aHash**.
3. Le plugin soumet la requête au démon local (`POST /v1/search` avec `"search_mode": "ahash"`).
4. Le serveur extrait 5.0 secondes d'audio (430 trames à `86.125 fps`, 5512 Hz mono), génère l'empreinte aHash de la requête et interroge le catalogue audio en mémoire (`AHashCatalog`) ou relaie `ahash_bytes` vers le serveur maître en mode `--agent`.
5. Les sous-titres certifiés associés au média reconnu s'affichent dans la liste des résultats avec l'offset calculé en millisecondes.
