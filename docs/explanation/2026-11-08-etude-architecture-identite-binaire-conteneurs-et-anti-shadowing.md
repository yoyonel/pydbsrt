---
title: "Étude d'Architecture : Modèle d'Identité Tripartite Conteneur-pHash, Élimination du Shadowing & Remise en État de l'Encoding"
tags:
  - explication
  - architecture
  - phash
  - identification
  - shadowing
  - container-hash
  - couplage
---

# 💡 Étude d'Architecture : Modèle d'Identité Tripartite Conteneur-pHash, Élimination du Shadowing & Remise en État de l'Encoding

**Date :** 8 novembre 2026
**Auteur :** Lionel ATTY (Antigravity pair programming)
**Statut :** Spécification d'Architecture & Plan Directeur de Remise en État Fonctionnel
**Contexte :** Constat de faux rejets d'audit (statut *Élagué* / *Pruned*) sur des œuvres de référence majeures (ex: *Spider-Man*, *Arcane*, *The Boys*) causé par le masquage (*shadowing*) des véritables médias complets (fichiers MKV de 10 à 26 Go, durées > 2h) par des empreintes partielles ou corrompues (26 à 30 secondes, voire 0 à 50 frames) portant le même nom textuel de release ou un préfixe collisionnant dans `data/__PHASH__/`.

---

## 1. Diagnostic Forensique du Parc d'Empreintes & Causes Racines

Un audit direct du référentiel `data/__PHASH__/` (4 265 fichiers `.phash` recensés au 8 novembre 2026) a mis en lumière les causes techniques de la contamination du banc de trajectoires.

### 1.1 Cartographie du Référentiel Réel

| Catégorie de Fichiers | Nombre | Description / État |
| :--- | :---: | :--- |
| **Total fichiers `.phash`** | 4 265 | Volume global d'empreintes stockées sous `data/__PHASH__/` |
| **Nommés par MovieHash (16 hex)** | 1 415 | Format canonique `<moviehash>.<fps>fps.phash` |
| **Nommés par Titre / Chaîne textuelle** | 2 850 | Format hérité `<nom_release>.<fps>fps.phash` |
| **Sidecars JSON existants (`.json`)** | 1 703 | Fichiers de métadonnées associant MovieHash et titre |
| **Fichiers tronqués critiques (< 50 Ko / < 10 min)** | **290** | **Empreintes anormalement courtes (0 octet, 50 frames, 26s)** |

### 1.2 Anatomie Forensique des 290 Fichiers Tronqués

L'inspection détaillée des 290 fichiers de moins de 50 Ko démontre que la pollution provient de deux vecteurs distincts :

1. **Extractions interrompues ou crashées (0 à 100 frames) :**
   - `The.Expanse.S04E08...mkv.24.0fps.phash` : **0 octet** (0 frame).
   - `Hercules (1997)...mkv.24.0fps.phash` : **8 octets** (1 frame unique).
   - `The.Boys.S03E02...mkv.24.0fps.phash` et son pendant `8a8294277c701d86.24.0fps.phash` : **400 octets** (exactement 50 frames, soit 2,1 secondes).
   - `Arcane S01E05` et `S01E06` : **720 octets** (90 frames, soit 3,8 secondes).

2. **Snippets d'essais et trailers nommés avec le titre du master complet (20 à 30 secondes) :**
   - `Spider-Man - Brand New Day (2026)...mkv.24.0fps.phash` : **5 088 octets** (636 frames = 26,5 s).
   - `Spider-Man Into The Spider-Verse...mkv.24.0fps.phash` : **5 896 octets** (737 frames = 30,7 s).
   - `Spider-Man Across The Spider-Verse...mkv.24.0fps.phash` : **5 824 octets** (728 frames = 30,3 s).

### 1.3 Mécanisme de Propagation du Masquage (*Shadowing Chain*)

Le masquage du fichier vidéo complet par le snippet s'opère en 4 étapes consécutives à travers le code :

```
[Extrait court 26s généré dans data/__PHASH__]
                      │
                      ▼ (Étape 1 : scan_existing_phash)
  Recherche par stem / préfixe trouve le fichier court (5 Ko).
  Absence de vérification du nombre de frames attendu.
  => Court-circuit : "Reused existing .phash (636 frames), no decode".
                      │
                      ▼ (Étape 2 : Daemon d'ingestion & batch workers)
  o2switch_batch_worker.py / batch_media_add_tvshow.py :
  done_stems = load_existing_phash_stems(...)
  Le master vidéo de 26 Go est considéré comme "déjà fait" et sauté à jamais !
                      │
                      ▼ (Étape 3 : dbsrt pack)
  Compilation aveugle de catalog.dbphash intégrant Spider-Man avec :
  duration_ms = 26 500 ms (26.5s) au lieu de 9 000 000 ms (2h30).
                      │
                      ▼ (Étape 4 : trajectory_bench_runner)
  Évaluation multi-coupes (ex: 20 coupes réparties sur duration_ms) :
  20 coupes espacées de 1.3s tombent sur les cartons de logos et fonds noirs.
  Score de consensus nul => L'œuvre majeure est faussement rejetée en ⚪ Élagué.
```

---

## 2. Modèle Formel d'Identité Tripartite Conteneur-pHash

Un hash perceptuel (`pHash`) seul ne possède aucune signature binaire du conteneur physique. Réciproquement, un hash de conteneur (`moviehash`) seul ne garantit pas la complétude du flux perceptuel (une extraction écourtée à 50 frames partage le même `moviehash` que le fichier complet).

L'identité certifiée d'un média dans `pydbsrt` est donc définie mathématiquement comme un triplet indissociable :

$$
\mathcal{I}_{\text{media}} = \Big(\mathcal{H}_{\text{container}}, \;\mathcal{S}_{\text{stream}}, \;\mathcal{D}_{\text{phash}}\Big)
$$

### 2.1 Couche 1 : Empreinte Binaire Conteneur ($\mathcal{H}_{\text{container}}$)

- **Algorithme** : MovieHash OpenSubtitles 64-bit uint checksum (`dbsrt_core::metadata::compute_opensubtitles_hash`).
- **Signature** : 16 caractères hexadécimaux calculés sur la formule $\text{checksum} = \text{file\_size} + \sum_{i=0}^{8191} \text{chunk}_{\text{head}}[i] + \sum_{i=0}^{8191} \text{chunk}_{\text{tail}}[i]$.
- **Attributs complémentaires** :
    - `file_size_bytes` : Taille exacte du conteneur en octets.
    - `source_filename` : Nom de fichier source de la release.
- **Complexité** : $\mathcal{O}(1)$ (lecture de 128 Ko au total), temps d'exécution $< 2\,\text{ms}$, compatible avec les partages CIFS distants.

### 2.2 Couche 2 : Spécification Temporelle du Flux ($\mathcal{S}_{\text{stream}}$)

- **Attributs fondamentaux** :
    - `fps` : Fréquence de décodage certifiée (ex: `24.0`, `23.976`, `25.0`).
    - `frame_count` : Nombre exact d'empreintes 64 bits dans le flux.
    - `duration_ms` : Durée temporelle calculée selon $\text{round}\left(\frac{\text{frame\_count}}{\text{fps}} \times 1000\right)$.
    - `is_complete` : Booléen (`true` si extrait sur l'intégralité du média $\pm 1.5\,\text{s}$ par rapport au conteneur, `false` si snippet/cut).
    - `sample_range` : Si `is_complete == false`, tuple `(start_sec, duration_sec)`.

### 2.3 Couche 3 : Empreinte Cryptographique du Flux pHash ($\mathcal{D}_{\text{phash}}$)

- **Algorithme** : `compute_media_phash_digest` (`dbsrt_core::coupling`).
- **Signature** : SHA-256 (tronqué à 32 hex) appliqué sur les octets bruts big-endian du flux pHash selon la relation $\mathcal{D}_{\text{phash}} = \text{SHA-256}\Big(\big[\text{to\_be\_bytes}(h_0), \dots, \text{to\_be\_bytes}(h_{N-1})\big]\Big)_{[0..32]}$.
- **Rôle** : Clé primaire d'intégrité visuelle liant le flux pHash aux sous-titres certifiés dans PostgreSQL (`medias.media_hash`) et Zero-DB (`couplings.json`).

---

## 3. Invariants Architecturaux Anti-Shadowing

Pour mathématiquement interdire toute récurrence de masquage ou de pollution, cinq invariants stricts sont érigés :

### Invariant 1 : Barrière d'Intégrité de Complétude du Cache (`scan_existing_phash`)

Tout court-circuit d'extraction basé sur un fichier `.phash` existant sur disque DOIT valider la complétude par rapport au probe du conteneur source :

$$
| \text{hashes.len()} - \lfloor \text{meta.duration} \times \text{meta.fps} \rfloor | \le \max\Big(12, \;0.02 \times \lfloor \text{meta.duration} \times \text{meta.fps} \rfloor\Big)
$$

Les deux règles d'application sont immédiates :

- Si le fichier `.phash` en cache a un nombre d'empreintes inférieur de plus de 2 % au nombre de frames attendu pour la vidéo probe, le cache est **rejeté**, un avertissement opérationnel est émis, et l'extraction complète est déclenchée.
- Les fichiers de taille nulle (0 octet) ou manifestement tronqués (< 120 frames sur un long métrage) sont automatiquement invalidés.

### Invariant 2 : Suppression du Fallback Laxe par Sous-Chaîne (`match_stem_phash`)

La fonction `match_stem_phash` qui acceptait arbitrairement n'importe quel fichier contenant la racine du nom sans vérification de taille est bannie.
La résolution d'un cache existant suit désormais une hiérarchie stricte en 2 passes sans ambiguïté :

1. **Passe 1 (Prioritaire & Déterministe)** : Correspondance exacte par MovieHash : `<phash_dir>/<moviehash>.<fps>fps.phash`.
2. **Passe 2 (Repli Hérité Sécurisé)** : Correspondance exacte par nom de fichier complet de la release `<full_release_name>.<fps>fps.phash`, soumise obligatoirement à la barrière de complétude de l'Invariant 1.

### Invariant 3 : Cloisonnement Strict des Espaces de Nommage

Les principes d'isolation des données d'évaluation s'énoncent ainsi :

- `data/__PHASH__/` est le référentiel de production **strictement réservé aux œuvres complètes certifiées**.
- Les extraits de banc d'essai (cuts, snippets 15s/30s) doivent impérativement être générés et stockés dans `data/snippets/` ou `data/bench/`.
- Aucun script d'évaluation ou d'extraction de snippet ne doit écrire dans `data/__PHASH__/` sans suffixe explicite `_cut` et marquage `is_complete = false`.

### Invariant 4 : Filtre de Sécurité au Packing (`dbsrt pack`)

Le compilateur de catalogue Zero-DB (`run_pack`) intègre un filtre de garde opérationnel :

- Tout fichier `.phash` dont la durée calculée est inférieure à $180\,\text{s}$ (3 minutes) est exclu du `catalog.dbphash` de référence, sauf argument explicite `--allow-short-media`.
- Ce filtre garantit que des résidus de 26 secondes ne peuvent en aucun cas s'immiscer dans la matrice de recherche globale.

### Invariant 5 : Détection et Élimination des Faux Rejets dans le Trajectory Runner

Dans `trajectory_bench_runner.py`, la sélection des positions de coupe (`compute_cut_offsets`) applique un contrôle d'éligibilité :

- Si la durée déclarée de la cible dans `catalog.dbphash` est inférieure à 120 s alors que le nom textuel ou le stem indique un long métrage (absence de tags `trailer`, `spot`, `snippet`), le runner émet un statut d'alerte spécifique **⚠️ Erreur Identité / Empreinte Tronquée** au lieu du faux statut *Élagué*.

---

## 4. Spécification Technique de la Sérialisation & Métadonnées

Pour concilier compatibilité ascendante avec les 4 265 fichiers existants et traçabilité inviolable, deux formats complémentaires sont standardisés :

### 4.1 Sidecar JSON v2 (`<moviehash>.json`)

Le sidecar JSON v2 formalise le contrat d'identité complet :

```json
{
  "$schema": "https://dbsrt.org/schemas/phash_sidecar_v2.json",
  "moviehash": "a1b2c3d4e5f60718",
  "source_file_size": 27575971840,
  "source_file_name": "Spider-Man - Brand New Day (2026) 2160p.mkv",
  "is_complete_media": true,
  "sample_range": null,
  "fps": 24.0,
  "frame_count": 216000,
  "duration_ms": 9000000,
  "phash_digest": "3c9a66708998e2e0e006f1609c6c46d5",
  "extracted_at": 1794154800,
  "extractor_version": "dbsrt 0.1.0"
}
```

Pour un cut de banc de test :

```json
{
  "$schema": "https://dbsrt.org/schemas/phash_sidecar_v2.json",
  "moviehash": "e5f60718a1b2c3d4",
  "source_file_size": 1677721,
  "source_file_name": "Spider-Man_-_Brand_New_Day_mid15s.mp4",
  "is_complete_media": false,
  "sample_range": {
    "start_sec": 4500.0,
    "duration_sec": 15.0
  },
  "parent_moviehash": "a1b2c3d4e5f60718",
  "fps": 24.0,
  "frame_count": 360,
  "duration_ms": 15000,
  "phash_digest": "78a9bc01d93214ef560128ba771239aa",
  "extracted_at": 1794154800,
  "extractor_version": "dbsrt 0.1.0"
}
```

### 4.2 Format Conteneur Binaire Unifié `.phash` v2 (En-tête Autonome)

Afin d'éviter la fragilité des sidecars séparés (risque de perte lors d'un `rsync` partiel), un en-tête fixe de 64 octets est spécifié pour les nouvelles extractions :

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       Magic: 'D' 'B' 'S' 'R' 'T' '_' 'P' 'H'                  |  8 octets (0x44425352545F5048)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          Version (u16)        |          Flags (u16)          |  4 octets (v=2, flags: 0x01=complete)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Source MovieHash (u64)                    |  8 octets (OpenSubtitles checksum)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                    Source File Size (u64)                     |  8 octets (Taille octets vidéo)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       FPS (f32 LE)                            |  4 octets (ex: 24.0)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Frame Count (u32 LE)                      |  4 octets (Nombre de frames)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Duration Millis (u64)                     |  8 octets (Durée millisecondes)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                    Reserved / Padding                         | 20 octets (Alignement strict 64B)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|          PAYLOAD: [u64 BE; Frame Count] Empreintes DCT-II      | (Frame Count * 8 octets)
|                                                               |
```

- **Détection Automatique (Zero-Cost Reader)** : La routine `read_phash_file` inspecte les 8 premiers octets. S'ils valent `DBSRT_PH`, le header de 64 octets est parsé et les empreintes débutent à l'offset 64. Sinon, le fichier est traité comme un flux brut v1 (compatibilité 100 % immédiate).

---

## 5. Plan Opérationnel de Remise en État en 5 Phases

Pour restaurer l'intégrité opérationnelle du système sans interrompre les bancs de tests, le déploiement s'articule en 5 étapes séquentielles :

### Phase 1 : Assainissement & Quarantaine du Référentiel Local

1. Déploiement d'un script d'audit et de mise en quarantaine (`scripts/quarantine_truncated_phashes.py`).
2. Déplacement des 290 fichiers polluants (< 50 Ko, 0 octet, ou snippets connus comme Spider-Man 26s) depuis `data/__PHASH__/` vers un dossier isolé `data/__QUARANTINE_PHASH__/`.
3. Émission d'un rapport JSON horodaté de quarantaine listant chaque fichier déplacé, sa taille, et le motif d'éviction.

### Phase 2 : Durcissement du Moteur Rust (`dbsrt-core` & `dbsrt-cli`)

1. **`crates/dbsrt-cli/src/commands/shared.rs`** :
    - Mise à niveau de `scan_existing_phash` pour éliminer le fallback permissif `match_stem_phash`.
    - Application de la barrière d'intégrité de complétude comparant `hashes.len()` à `meta.duration * meta.fps`.
2. **`crates/dbsrt-cli/src/commands/pack.rs`** :
    - Ajout du seuil minimal de garde ($180\,\text{s}$) empêchant les stubs d'entrer dans `catalog.dbphash`.
3. **`crates/dbsrt-core/src/cuts.rs`** :
    - Support transparent de l'en-tête v2 `DBSRT_PH` dans `read_phash_file` et `write_phash_file`.

### Phase 3 : Harmonisation des Scripts d'Ingestion & de Benchmarking

1. **`scripts/o2switch_batch_worker.py` & `scripts/batch_media_add_tvshow.py`** :
    - Remplacement de la vérification de présence par simple préfixe `stem` par une vérification de la taille minimale (> 50 Ko) et de complétude.
2. **`scripts/trajectory_bench_runner.py`** :
    - Détection des cibles anomalement courtes et émission du statut d'erreur d'identité explicite.

### Phase 4 : Campagne de Ré-extraction & Recouplage des Masters Masqués

1. Exécution ciblée de `task media:add` sur les œuvres majeures précédemment masquées (*Spider-Man*, *Arcane*, *The Boys*).
2. Vérification que l'empreinte complète (ex: ~216 000 frames pour 2h30, ~1,7 Mo) est correctement générée sous `<moviehash>.24.0fps.phash`.
3. Certification du couplage avec les sous-titres officiels dans `data/SRT/couplings.json`.

### Phase 5 : Recompilation du Catalogue Zero-DB & Validation E2E

1. Recompilation du catalogue maître via `task pack:catalog`.
2. Exécution du banc de suivi de trajectoire sur Spider-Man et les œuvres réhabilitées via `task bench:trajectory`.
3. Confirmation empirique de l'élimination des faux rejets *Élagué* et bascule vers les statuts nominaux (🟢 *Certifié* / 🟡 *Candidat*).

---

## 6. Synthèse des Décisions Techniques Arbitrées

| Point d'Arbitrage | Décision Retenue | Justification Technique |
| :--- | :--- | :--- |
| **Identifiant Unique** | MovieHash OpenSubtitles (16 hex) | Déjà intégré dans `dbsrt-core`, PostgreSQL V4 et CLI ; $\mathcal{O}(1)$ sur montages CIFS/SMB. |
| **Format pHash** | Dual : Sidecar v2 + Header Binaire v2 | Permet une inspection humaine rapide (`cat *.json`) tout en garantissant l'atomicité binaire transportable. |
| **Gestion des Snippets** | Quarantaine et isolation stricte hors de `data/__PHASH__/` | Élimine à la source le risque de collision entre bancs de test et référentiel de production. |
| **Vérification Cache** | Barrière de complétude tolérance 2 % | Interdit définitivement à une extraction avortée ou un extrait court de masquer une vidéo complète. |
| **Packing Catalogue** | Garde minimale à 180 secondes | Garantit la pureté mathématique des répartitions multi-coupes dans `trajectory_bench_runner`. |
