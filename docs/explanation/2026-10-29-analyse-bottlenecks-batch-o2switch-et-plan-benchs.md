---
title: "Analyse Bottlenecks Batch o2switch : Mesures, Plan de Benchs & Game Changers"
tags:
  - explanation
  - performance
  - o2switch
  - ffmpeg
  - phash
  - benchmark
---

# Analyse Bottlenecks Batch o2switch : Mesures, Plan de Benchs & Game Changers

> **Date** : 29 Octobre 2026 à 22:55 CET
> **Contexte** : batch 971 films, 5 workers, budget 10 vCPU, FTP NAS Synology → o2switch.
> **Modèle de référence** : [Étude Théorique : Capacité Maximale Décodage + pHash](2026-10-29-etude-theorique-plafond-pipe-phash-o2switch.md) — ses symboles ($C$, $K$, $\alpha$, $\beta$, $B^*$) sont réutilisés ici.
> **Objet** : situer la production réelle par rapport au modèle, décomposer l'écart, chiffrer les leviers avec leur plan de validation, et lister les options de rupture (× 5 et plus) avec leurs contreparties.

---

## 1. État mesuré (29/10/2026, 22:36–22:46 CET)

### Instantané `dbsrt top` + mesure `/proc` sur 30 s

| Job | Contenu | fps mesuré | CPU mesuré | fps théorique à ~2 vCPU/job | Rendement |
|---|---|---|---|---|---|
| Le Roi et l'Oiseau 1080p | HEVC rip | 288 | 208 % | ≈ 310 ($C_{max}$) | **93 %** |
| Le Parc des Merveilles 1080p | HEVC rip | 240 | 188 % | ≈ 280 | **86 %** |
| La Fameuse Invasion des Ours | 1080p+ | 148 | 169 % | ≈ 200–250 | ~70 % |
| The Neon Demon 2160p UHD remux | 4K HEVC 10 bits | 32 | 213 % | 39–64 | 50–80 % |
| The Dead Don't Die 2160p WEB | 4K HEVC 10 bits | 24 | 204 % | 39–64 | ~50 % |
| **Total** | | **732** | ≈ 980 % | **≈ 850–1 100** | **70–90 %** |

Réseau Net RX : 25,1 MB/s sur un plafond de ≈ 113 MiB/s → **le réseau ne lie pas, le CPU lie**, exactement le régime prédit par le modèle (tous les profils courants sont sous leur $B^*$).

### Palier 5 jobs (LVE `lvecpu`, 10 s d'échantillonnage, 35 min, 200 échantillons)

| Métrique | Valeur |
|---|---|
| moyenne | 79,9 / 100 |
| p95 | 89 |
| max | 100 (4 échantillons ≥ 98, soit 2 % du temps) |
| RAM max | 2,3 GiB / 48 |
| nproc max | 114 / 400 |

Verdict : le réglage `10 vCPU / 5 workers` est **sain et proche de l'optimal local** — marge moyenne 20 points, saturation seulement aux pics.

## 2. Décomposition de l'écart vs le plafond théorique (≈ 2 800 fps)

| Cause | Poids | Réductible ? |
|---|---|---|
| **Mix de contenu** : 2 slots 4K HEVC 10 bits brûlent ≈ 42 % du budget CPU pour 7,6 % des fps ($K_{4K} \approx 5 \times K_{1080p}$) | ÷2 à ÷3 | non — propriété du média, et le travail total $\sum K_i$ doit être payé de toute façon |
| Budget 10 vCPU sur 12 | −17 % | oui (levier 1) |
| $e_{HT}$ mutualisé : jumeaux hyperthread occupés par les voisins de l'hôte | −10 à −30 % | non |
| `-threads 2` × 5 jobs au lieu de `-threads 1` × 10–12 (`DEFAULT_FFMPEG_THREADS = "2"`, `scripts/o2switch_batch_worker.py:167`) | −5 à −15 % | oui (levier 2) |
| `dbsrt top` ouvert (≈ 25 % d'un cœur) + sampler + worker Python | −2 à −3 % | oui (levier 3) |

**Rappel important** : le fps instantané est une métrique de confort pour un batch. Le temps mur total vaut $\approx \sum_i K_i / C$, indépendant de l'ordre de la file. Réordonner (1080p d'abord) gonfle l'affichage, pas la date de fin. Seuls comptent : augmenter $C$ utile, ou baisser $K$.

## 3. Leviers incrémentaux : plan de validation avant déploiement

Principe : un levier à la fois, mesure commune (sampler LVE 30 min minimum, comparaison max/p95/mean + fps agrégé), rollback défini avant de commencer.

### Levier 3 — fermer `dbsrt top` quand personne ne regarde

- **Gain attendu** : +2 %. **Risque** : nul. **Coût** : 0.
- **Test** : 10 min de sampler avec top fermé vs ouvert, même charge. Go si delta lvecpu ≥ 2 points.
- **Probabilité de succès** : ≈ 100 % (coût ~25 % d'un cœur déjà observé).

### Levier 2 — `-threads 1` par ffmpeg + autoscale 10–12 jobs

- **Gain attendu** : +5 à +15 % (zéro synchro frame-threading, meilleur rendement cache).
- **Risques réels** : rendement du frame-threading (peut-être déjà bon), et le NAS qui refuse ou ralentit 10–12 connexions FTP simultanées (limite « connexions max par IP » du DSM).
- **Test A (rendement, 30 min, hors prod)** : même fichier 1080p HEVC du NAS, 120 s de décodage : `ffmpeg -threads 1` × 2 process en parallèle vs `ffmpeg -threads 2` × 1 process, pipeline identique (`scale=32:32`, gray, rawvideo → `/dev/null`), frames comptées via `-progress`. **Go si** `fps(2×threads1) / fps(1×threads2) ≥ 1,05` à CPU égal.
- **Test B (FTP concurrence)** : 12 `curl ftp://… -o /dev/null --max-time 60` en parallèle sur 12 fichiers distincts. **Go si** 0 refus et débit agrégé ≥ 25 MiB/s. Un refus = réglage DSM à monter, pas un échec du levier.
- **Risque résiduel après go** : un reload/drain avec 10–12 jobs en vol perd plus de travail requeué. Mitigation : déployer juste après un drain.
- **Probabilité de succès** : ≈ 70 % pour ≥ +5 %, ≈ 30 % pour ≥ +10 %. Coût d'échec : 30 min de bench, rien déployé.

### Levier 1 — budget 10 → 11,5 vCPU (canari 1 h)

- **Gain attendu** : +10 à +15 %.
- **Risque réel** : throttling LVE (CPU faults) qui rend le gain nul ou négatif, et ssh/top qui rament.
- **Mesure objective du risque** : compteurs de faults avant/après — `lveinfo --period 1h` (colonne CPU faults ; syntaxe exacte à vérifier sur l'hôte).
- **Test** : `DBSRT_CPU_BUDGET_VCPUS=11.5` dans l'env du worker, hot-reload existant, 60 min avec sampler. **Go si** : faults ≈ 0, lvecpu p95 ≤ 96, fps agrégé ≥ +10 %, `time ssh o2switch true` < 2 s.
- **Rollback** : remettre `10`, re-exec (1 variable, ~10 min de drain).
- **Probabilité de succès** : ≈ 60 % pour +10–15 % net ; l'échec le plus probable est un résultat neutre (throttling), pas une casse.

**Cumul réaliste des trois leviers : +25 à +35 %** (≈ 900–1 000 fps sur le mix courant). C'est le plafond des optimisations *incrémentales* : au-delà, la physique du mutualisé (12 vCPU, pas de GPU) est la borne.

## 4. Game changers (× 5 à × 20) — changements d'algorithme, pas de réglage

Aucune optimisation du code actuel ne dépassera ~×1,35 : le décodage complet de chaque frame est incompressible ($K = \alpha P + \beta B$). Les ruptures changent **ce qu'on décode**, avec des contreparties produit :

| Option | Multiplicateur | Principe | Contrepartie |
|---|---|---|---|
| **A. Échantillonnage keyframes/cuts à l'ingestion** | **× 10–20 en CPU** | `-skip_frame nokey` : les paquets non-clés sont jetés avant le CABAC. Le mode `KeyFrames` existe déjà dans `dbsrt` (sampling) ; le batch l'ignore aujourd'hui | les hashes produits changent de sémantique → base à réingérer, et le matching doit fonctionner en keyframes/cuts (voir [l'étude comparative cuts/keyframes/hybride](2026-10-17-etude-comparative-identification-frames-cuts-keyframes-hybride.md)) |
| **B. Lecture par plages d'octets des keyframes** (MKV Cues / MP4 `stss` + FTP `REST`) | × 5–10 en réseau, cumulable avec A | ne transférer que les octets des I-frames au lieu du fichier entier | développement spécifique (parse des Cues côté Rust, reprises FTP), fragile sur conteneurs mal indexés |
| **C. Délocaliser le décodage** sur une machine avec GPU (PC local, NVDEC) | × 5–15 | NVDEC décode 4K HEVC à 400–900 fps par carte, CPU libre | o2switch ne sert plus qu'au stockage/matching ; architecture à revoir, machine allumée à demeure |
| **D. `-skip_loop_filter all`** | × 1,2–1,3 seulement | sauter deblocking/SAO (baisse $\alpha$) | hashes légèrement différents → risque de dérive vs la base, à valider sur corpus avant tout usage |

**Recommandation** : si un « wouah » est attendu, il est dans **A (+ B)** — c'est le seul chemin vers ×10 sur cette plateforme, et la moitié du travail (mode `KeyFrames`, étude cuts/keyframes) existe déjà dans le repo. Décision produit requise : accepter une base de hashes keyframes/cuts pour l'ingestion de masse, la base full-frame restant pour les cas exigeants.

## 5. Ordre d'exécution proposé

1. Levier 3 (gratuit, immédiat).
2. Levier 2 : tests A et B hors prod, déploiement si go.
3. Levier 1 : canari 1 h, garde si go.
4. En parallèle, instruire la décision produit du game changer A : bench de reconnaissance sur hashes keyframes vs full-frame (le corpus E2E de 1 336 extraits existe déjà comme jeu de validation).

---

## Non vérifié

- Bitrates exacts des 5 fichiers mesurés (estimés taille/durée) ; `/proc/<pid>/io` rend 0 sur cet hôte, le débit FTP par job n'est pas mesurable par process.
- Syntaxe exacte de `lveinfo` et limite de connexions FTP du DSM : à découvrir au premier essai.
- $\alpha$, $\beta$, $e_{HT}$ : littérature, non profilés sur l'hôte (±50 %).
