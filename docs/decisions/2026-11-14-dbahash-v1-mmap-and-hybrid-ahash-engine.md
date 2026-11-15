---
title: "ADR-0007 : Archive Binaire mmap .dbahash v1, Ordonnancement Thermique & Moteur Hybride AhashEngine"
tags:
  - adr
  - decisions
  - ahash
  - mmap
  - coarse-to-fine
  - mih
  - performance
---

# ADR-0007 : Archive Binaire `mmap` `.dbahash` v1, Ordonnancement Thermique & Moteur Hybride `AhashEngine`

- **ID** : `ADR-0007`
- **Date** : 2026-11-14
- **Statut** : Accepté (Issue #81)
- **Périmètre** : `crates/dbsrt-index/src/dbahash/`, `crates/dbsrt-index/src/coarse_to_fine.rs`, `crates/dbsrt-cli` (`pack-ahash`), `crates/dbsrt-server` (`AppState`)
- **Journal d'Ingénierie associé** : [Architecture `.dbahash` v1 `mmap`, Multi-Order Folios Linux 6.6+ & Coarse-to-Fine Zéro-Alloc](../explanation/2026-11-14-architecture-dbahash-mmap-folios-linux-et-c2f-zero-alloc.md)

---

## 1. Contexte & Contraintes Système

Sur le catalogue acoustique complet synchronisé depuis o2switch (**`2 490` pistes audio uniques**, **`628 946 940` sous-empreintes** Haitsma-Kalker 64-bit à \(86.125\text{ fps}\)), la structure historique `AHashCatalog` en mémoire vive dupliquait les vecteurs `entries[i].hashes` (`4.69 GiB`) et le tableau plat `flat_hashes` (`4.69 GiB`), mobilisant **`9.37 GiB` de tas anonyme (`RssAnon`)** avant même la construction de l'index grossier `MIH-32` (`+300 MiB`), et exigeait la lecture + désérialisation de `2 503` fichiers unitaires + tri parallèle à chaque démarrage CLI ou `reload` serveur. Sur une station développeur de `32 GiB` (`~9.8 GiB` disponibles en session active), l'évaluation matricielle en RAM devenait sujette au *swap* / OOM, et sur l'instance de production o2switch (`PMEM = 48 GiB` LVE sur NVMe local), chaque rechargement monopolisait inutilement `~10 GiB` de tas privé non partageable. En parallèle, l'étage d'affinage Coarse-to-Fine allouait un `BTreeMap` par requête et balayait une fenêtre trop large de \(\pm 2.0\text{ s}\) (`345` décalages).

L'architecture cible impose quatre invariants d'ingénierie :

1. **Zéro tas dense (`RssAnon < 48 MiB`) & Page-Cache maîtrisé** : éliminer toute copie `Vec<u64>` des trames denses (`4.69 GiB`) afin que seules les tables `MIH-32` (`299.92 MiB` à \(K=32\)) et les quelques pages `4 KiB` des fenêtres fines vérifiées résident en `RssFile` partageable par l'OS.
2. **Ouverture instantanée** : `MmapDbahash::open() < 50 ms` (cible `< 5 ms`) et `AhashEngine::open() < 100 ms` pour ne pénaliser ni les commandes CLI courtes (`dbsrt recognize-media`) ni les rechargements atomiques à chaud (`POST /v1/catalog/reload`).
3. **Visibilité immédiate des ingestions fraîches & immunité aux jumeaux binaires** : un nouveau fichier `.ahash` extrait après le dernier `pack-ahash` doit être interrogeable immédiatement en delta RAM, tandis que les jumeaux binaires inter-releases (`720p` vs `1080p` partageant la même bande-son) déjà absorbés par le pack doivent être ignorés en \(\mathcal{O}(1)\) sans lecture disque ni défaut de page `mmap`.
4. **Zéro régression de rappel et de précision temporelle** : maintien d'un rappel 3-fenêtres optimal (`99.41 %` sur `2 541` vidéos indexées) et d'une dérive de recalage strictement nulle (`0.0 ms`) face à l'étalon exhaustif \(K=1\).

---

## 2. Options Évaluées

### Axe A — Stockage et Ordonnancement Binaire de `catalog.dbahash`

1. **Option A1 — Blocs entrelacés par média (`[Meta + Coarse + Dense]` répétés \(N\) fois)** : **Rejetée**. Disperse les métadonnées et les trames grossières sur `5.25 GiB` d'espace disque, brise la contiguïté des tables `MIH-32` globales et provoque des défauts de page majeurs dans les zones denses froides lors de la lecture des descripteurs.
2. **Option A2 — `DenseHashes` en tête suivi de `MIH-32` en pied de fichier (style SSTable)** : **Rejetée**. Empêche d'isoler les sections chaudes dans le préfixe contigu initial et complique l'application d'un `madvise` ciblé sur la queue froide du fichier.
3. **Option A3 — Ordonnancement thermique strict du chaud vers le froid (`Header -> MediaTable -> Strings [+ Trailer \0TWINS1\0] -> CoarseHashes -> MIH-32 Upper -> MIH-32 Lower -> DenseHashes`) aligné sur 64 octets** : **Retenue**. Regroupe toutes les sections chaudes dans le préfixe et relègue les `4.69 GiB` (`4 798.5 MiB`) de trames denses froides en toute fin d'archive, permettant une compilation streaming en 2 passes à RAM bornée (`64 KiB` par thread) sans `seek` arrière.

### Axe B — Facteur de Décimation Grossier \(K\) par Défaut

1. **Option B1 — `K = 16` (`5.38 fps`, `39.31 M` trames grossières)** : **Non retenue par défaut** (mais conservée via `--factor-k 16`). Double la taille des tables `MIH-32` à **`599.85 MiB`** sans gain significatif sur le rappel multi-fenêtres.
2. **Option B2 — `K = 32` (`2.69 fps`, `19.66 M` trames grossières)** : **Retenue comme plafond mathématique nominal** (`DEFAULT_DBAHASH_FACTOR_K = 32`). Contient les deux tables `MIH-32` dans **`299.92 MiB`** (`0.12 MiB` par piste audio, plus léger que `pHash` à `0.16 MiB` par média) tout en garantissant `13` à `14` ancres exactes par requête de `5.0 s` (`430` trames), soit **`99.41 %`** de rappel 3-fenêtres et **`0.0 ms`** de dérive.
3. **Option B3 — `K >= 48` ou `K = 64`** : **Strictement rejetée**. Fait chuter le nombre d'ancres par fenêtre de `5.0 s` à `6–8`, dégradant la survie du seuil à `>= 2` votes sur les ré-encodages bruités (`AAC` bas débit, *downmix* `5.1 -> 2.0`).

### Axe C — Gestion du Page-Cache Linux 6.6+ (`Multi-Order Folios`) & Régions VMA `madvise`

1. **Option C1 — `madvise(MADV_RANDOM)` global sur l'ensemble du `mmap`** : **Rejetée**. Deux effets pervers prouvés empiriquement : (a) n'empêche pas le mappage des *multi-order folios* (`~384 KiB`) résidents juste après `pack-ahash`, et (b) désactive le `fault_around_bytes` (`64 KiB`) du noyau sur les `299.92 MiB` des tables `MIH-32`, dégradant la latence froide à `39.5 ms`/requête (`76 800` défauts synchrones `4 KiB` `QD=1`).
2. **Option C2 — `posix_fadvise(POSIX_FADV_DONTNEED)` dans `MmapDbahash::open()` ou `POSIX_FADV_RANDOM` sur le `fd` partagé** : **Rejetée**. Purger `XArray` dans `open()` bloque le démarrage pendant des dizaines de millisecondes, et `POSIX_FADV_RANDOM` sous Linux (`mm/fadvise.c`) arme `FMODE_RANDOM` sur tout le `struct file` en ignorant `offset`, désactivant là aussi le *fault-around* sur `MIH-32`.
3. **Option C3 — Découplage Write-Time (`POSIX_FADV_DONTNEED` hors-ligne en fin de `pack-ahash`) + Open-Time VMA Ciblé (`mmap.advise_range(Advice::Random, offset_dense_hashes..len)`)** : **Retenue**. Évince les folios multi-ordres des sections froides dès la fin du build (`17.58 s`), préserve le *fault-around* `64 KiB` sur les sections chaudes `[0 .. offset_dense_hashes]`, et impose des défauts `order-0` stricts de `4 KiB` exclusivement sur `[offset_dense_hashes .. end]` (`< 3 MiB` de pages denses résidentes après 200 requêtes aléatoires sur `4.69 GiB` !).

### Axe D — Coexistence `catalog.dbahash` + Fichiers `.ahash` Unitaires & Jumeaux Inter-Releases

1. **Option D1 — Exclusion mutuelle stricte (ignorer `data/__AHASH__/` si `catalog.dbahash` existe)** : **Rejetée**. Rend invisibles les nouvelles pistes audio extraites entre deux exécutions de `task ahash:pack`.
2. **Option D2 — Indexation du seul `file_path` gagnant + repli `has_identical_dense_payload` à l'ouverture** : **Rejetée seule**. Sur un corpus contenant des jumeaux binaires (doublons legacy ou releases `720p`/`1080p` partageant la même piste audio), chaque jumeau non reconnu par nom forçait `AhashEngine::open` à lire son fichier `.ahash` puis à comparer sa tranche `mmap` dense complète (`2.44 s` d'ouverture et `750 MiB` de `RssFile` fautés au boot sur `546` jumeaux !).
3. **Option D3 — Remorque d'alias `\0TWINS1\0` dans `Strings` (`stat()` \(\mathcal{O}(1)\)) + Pré-filtre grossier `1/K` dans `has_identical_dense_payload` + Nettoyage `phash_dedupe.py`** : **Retenue**. `pack-ahash` consigne tous les noms de jumeaux absorbés en fin de table de chaînes (couvert par le checksum FNV-1a) : `AhashEngine::open` écarte **100 %** des pistes packées et de leurs jumeaux inter-releases par simple `stat()` (`11.69 ms` pour `2 503` fichiers, `0` octet lu, `VmRSS = 7.08 MiB`), et un éventuel jumeau renommé a posteriori est filtré d'abord sur `coarse_hashes` (`32x` plus compact) avant de toucher `dense_hashes`.

### Axe E — Périmètre du Checksum d'Intégrité à l'Ouverture

1. **Option E1 — Checksum intégral sur les `5.25 GiB`** : **Rejetée** à l'ouverture car elle force la lecture et la mise en page-cache de 100 % des trames froides (`> 2 s` et `VmRSS ~ 5.25 GiB`).
2. **Option E2 — Checksum FNV-1a 64-bit sur `DbahashHeader + MediaTable + Strings` (incluant `\0TWINS1\0`) + validation stricte des bornes et alignements 64 octets** : **Retenue**. S'exécute en `~1.4 ms` sur `2 490` pistes sans toucher une seule page froide.

---

## 3. Décision Retenue

1. **Spécification binaire `.dbahash` v1 (`DBAHASH\0`, version `1`)** : en-tête fixe `DbahashHeader` (`128 B`, `#[repr(C)]`), descripteurs `DbahashMediaEntry` (`64 B` par piste audio supportant nativement les conteneurs multi-pistes `.a1.ahash` / `.a2.ahash` et leur tag ISO 639-2 inline sur `7` octets), remorque optionnelle `\0TWINS1\0` en queue du pool `Strings` listant les jumeaux binaires absorbés, alignement universel des sections sur `64` octets (`DBAHASH_SECTION_ALIGN`, compatible `x86_64` et `aarch64`), tables `MIH-32` (`Mih32Entry` `Pod`) pré-triées à `K = 32` et section froide `dense_hashes` placée en fin de fichier, écrite atomiquement via `<output>.<pid>.<seq>.tmp` + `sync_all()` + `advise_cold_archive_sections` + `rename()`.
2. **Recherche Coarse-to-Fine zéro-allocation** : remplacement du `BTreeMap` par une arène réutilisable par thread (`TLS_COARSE_SCRATCHPAD`), clés packées `(m_idx << 32) | offset` triées sur place + RLE linéaire \(\mathcal{O}(H)\), rayon fin resserré à `DEFAULT_FINE_WINDOW_RADIUS_FRAMES = 16` (`±185.8 ms`), filtrage primaire à `>= 2` votes complété d'un repli contrôlé Top-`2` à `1` vote, et somme `POPCNT` bornée sans perte retournant `Option<u64>`.
3. **Intégration transparente CLI / Serveur / Benchmarks / Sync** : sous-commande `dbsrt pack-ahash` (`task ahash:pack`), déduplication automatique des jumeaux legacy `.ahash`/`.json` dans `scripts/ingestion/phash_dedupe.py` lors de `task data:sync`, bascule automatique de `bench-scale-multimodal` sur `AhashEngine` au-delà de `150M` trames (`MAX_IN_RAM_DENSE_BENCH_FRAMES`), et surveillance dédiée `spawn_catalog_file_watcher` déclenchant un swap atomique `ArcSwap` dans `dbsrt-server`.

---

## 4. Conséquences Mesurées & Limites Résiduelles

### Gains Empiriques Vérifiés (`2 490` pistes / `628.95 M` trames denses = `5 248.81 MiB`, `5 757` vidéos réelles / `9 529` fenêtres audio)

- **Compilation** : `2 503` fichiers `.ahash` compilés en **`17.58 s`** (`2 490` pistes uniques + `13` jumeaux inter-releases dans `\0TWINS1\0`, `64 KiB` de tampon dense par thread).
- **Ouverture** : `MmapDbahash::open = 1.426 ms` (`2.221 ms` post-pack) ; `AhashEngine::open = 11.685 ms` incluant `2 503` `stat()` et `0` entrée delta (`VmRSS = 7.08 MiB`, `RssAnon = 2.85 MiB`).
- **Empreinte mémoire** : **`VmRSS = 307.29 MiB`** après `200` requêtes aléatoires (`RssAnon = 4.63 MiB`, soit **`-99.95 %` de tas anonyme** face aux `~9 597 MiB` d'`AHashCatalog` ; `RssFile = 302.66 MiB` dont `299.92 MiB` de `MIH-32` et `< 3 MiB` de pages denses `4 KiB` sur `4.69 GiB`).
- **Latence de recherche** : **`1.387 ms`** en moyenne en régime nominal (`p50 = 1.384 ms`, `p95 = 2.151 ms` sur `task bench:dbahash:rss`) et **`2.750 ms`** par fenêtre sur la matrice multimodale complète de `5 757` vidéos (`p50 = 2.470 ms`, `p95 = 5.115 ms`, `8.249 ms` par vidéo 3-fenêtres).
- **Qualité métrologique (`2 541` vidéos indexées `aHash`)** : **`98.86 %`** de rappel 1-fenêtre (`2 512 / 2 541`), **`99.41 %`** de rappel 3-fenêtres (`2 526 / 2 541`, Précision `98.40 %`, F1 `98.90 %`, SNR `31.12 bits`), et **dérive `0.0 ms` moyenne / `0 ms` max (`100.0 % <= 15 ms`)** face à l'étalon exhaustif \(K=1\).

### Limites Résiduelles Connues

1. **Immuabilité de `catalog.dbahash`** : l'intégration définitive de nouveaux `.ahash` dans la couche `mmap` passe par une re-compilation complète (`task ahash:pack`, `17.58 s` pour `2 503` fichiers) ; entre deux compilations, les nouveautés résident dans le delta RAM de `AhashEngine`.
2. **Amorçage froid initial de `MIH-32` après reboot OS** : lors des toutes premières requêtes suivant une purge complète du page-cache OS (ou juste après une écriture massive de `5.25 GiB` saturant la file NVMe), la pagination initiale des `299.92 MiB` de `MIH-32` par *fault-around* `64 KiB` induit une latence transitoire (`mean = 23.8 ms` sur les 200 premières requêtes post-pack) avant stabilisation immédiate à `1.39 ms`.
3. **Re-foliation par lecteur séquentiel externe** : si un processus tiers (`cp`, `sha256sum`) lit séquentiellement `catalog.dbahash` pendant que le démon tourne, Linux peut recréer des folios multi-ordres dans le page-cache partagé (`RssFile` propre, 100 % évictable sous pression mémoire sans OOM mais visible dans `VmRSS`).
4. **Modification sur place à taille identique** : remplacer un `.ahash` existant en gardant exactement le même nom et le même nombre de trames impose de relancer `task ahash:pack` pour invalider le fast-skip `stat()`.

---

## 5. Sites & Validation Automatisée

- **Code** :
    - `crates/dbsrt-index/src/dbahash/{format.rs, builder.rs, reader.rs, engine.rs}`
    - `crates/dbsrt-index/src/coarse_to_fine.rs` & `crates/dbsrt-index/src/ahash.rs`
    - `crates/dbsrt-cli/src/commands/{pack_ahash.rs, bench_scale.rs, bench_matrix.rs}`
    - `crates/dbsrt-server/src/{state.rs, main.rs}`
    - `scripts/ingestion/{phash_names.py, phash_dedupe.py}` & `scripts/infra/o2switch/o2switch_ctl.sh`
- **Tests & Benchmarks** :
    - Hermétiques : `crates/dbsrt-index/tests/test_dbahash.rs` (parité exacte `AHashCatalog` vs `MmapDbahash` sur toutes les phases `0..32`, corruption magic/checksum/troncature, remorque `\0TWINS1\0` sur jumeaux amont/aval, multi-audio `.a1`/`.a2`), `crates/dbsrt-server/tests/test_server_ahash.rs` et `tests/tools/test_phash_dedupe.py`.
    - Empiriques : `task ahash:pack`, `task bench:dbahash:rss` (`test_empirical_real_catalog_dbahash_open_rss_and_latency`), `task bench:multimodal:matrix`.
