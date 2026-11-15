---
title: "ADR-0008 : Persistance Multi-Tiers Dédupliquée, Manifestes SHA-256 & Éviction Zéro-Perte"
tags:
  - adr
  - decisions
  - backup
  - storage
  - o2switch
  - synology
  - sha256
---

# ADR-0008 : Persistance Multi-Tiers Dédupliquée, Manifestes `SHA-256` & Éviction Locale Zéro-Perte

- **ID** : `ADR-0008`
- **Date** : 2026-11-15
- **Statut** : Accepté (Issue #86)
- **Périmètre** : `scripts/data_access/backup_manifest.py`, `scripts/data_access/posix_vault_agent.py`, `scripts/data_access/backup_targets.py`, `scripts/data_access/backup_cli.py`, `taskfiles/skills.yml`
- **Journal d'Ingénierie associé** : [Sauvegarde Multi-Tiers : Hardlinks POSIX, Packs PAX TAR en Streaming FTP & Garde-Fous d'Éviction](../explanation/2026-11-15-sauvegarde-multi-tiers-hardlinks-posix-et-packs-ftp-zero-cifs.md)

---

## 1. Contexte & Contraintes Système

Sur la station de développement (`~74 GiB` libres sur NVMe), le répertoire non versionné `data/` concentre **`21 681` fichiers (`16.66 GiB`)** répartis en 11 familles (`catalog.dbahash` `5.25 GiB`, `__AHASH__/` `4.8 GiB`, `snippets/` `3.3 GiB`, `__PHASH__/` `2.4 GiB`, `SRT/` `500 MiB`, `catalog.dbphash` `378 MiB`, `reports/` `204 MiB`, `snippets_ahash/` `137 MiB`, `quarantine_phash`, `benchmarks`, `seed`). Avant cette décision, **`3.55 GiB` de données critiques d'évaluation et d'extraits vidéo (`snippets/`, `snippets_ahash/`, `reports/`, `benchmarks/`) n'existaient sur aucun serveur distant** (*Single Point of Failure* sur le poste local), et aucun mécanisme de snapshot immuable ne protégeait `couplings.json` ou les catalogues binaires contre un écrasement accidentel.

L'audit empirique des quatre nœuds physiques impose des contraintes hétérogènes :

1. **Cloud `o2switch`** (`2.3 TiB` libres sur `/dev/md125`) : système de fichiers POSIX accessible en SSH/`rsync` (plafonné à `20 MB/s` sous CloudLinux LVE `42 MiB/s`), hébergeant déjà `13.8 GiB` actifs dans `~/dbsrt/data/`.
2. **NAS Synology** (`1.02 TiB` libres sur `/home`) : **ports SSH `:22` et `rsync` `:873` fermés** ; accessible en **FTP passif `:21`** (RFC 3659 `SIZE`, `MDTM`, `MFMT`, `MLSD`) et en SMB `:445`. Or tout montage noyau `mount.cifs` expose la station à des blocages processus ininterruptibles (`D-state`) lors des bascules Wi-Fi / 4G / VPN d'entreprise (`ADR` & topologie `pydbsrt-data-access`).
3. **Freebox Server** (`467 GiB` libres répartis sur 3 disques déjà occupés à `65–83 %` par les médias sources) et **Raspberry Pi 4** (`28 GiB` libres) : inadaptés comme coffres d'archives multi-versions.

---

## 2. Options Évaluées

### Axe A — Topologie des Cibles de Sauvegarde (`3-2-1`)

1. **Option A1 — `o2switch` seul** : **Rejetée**. Laisse dépendre toute restauration massive de la bande passante montante/descendante WAN (`20 MB/s`) et n'offre qu'un seul réplica hors-poste en cas d'éviction locale.
2. **Option A2 — Tri-Tier incluant les disques Freebox** : **Rejetée**. Fragmente les `467 GiB` restants dédiés aux conteneurs vidéo sources montés en lecture seule sur le RPi4.
3. **Option A3 — Bi-Tier `o2switch` Cloud (`2.3 TiB`) + `Synology NAS` LAN FTP `/home/pydbsrt-backups` (`1.02 TiB`)** : **Retenue**. Respecte la règle `3-2-1` (ou `2` réplicas géographiquement et matériellement indépendants lorsque le poste local évince un dataset lourd).

### Axe B — Représentation Physique Dédupliquée sur Stockages Hétérogènes

1. **Option B1 — Fichiers unitaires sur les deux cibles (`rsync` sur `o2switch` + `STOR` unitaire en FTP sur Synology)** : **Rejetée**. Transférer `21 681` petits fichiers en FTP passif exige `21 681` handshakes TCP `PASV`, et le protocole FTP ne dispose d'aucune primitive `hardlink` pour dédupliquer deux snapshots successifs.
2. **Option B2 — Archives `.tar` adressées par contenu sur les deux cibles** : **Rejetée**. Dupliquerait inutilement `+13.8 GiB` sur `o2switch` alors que ces fichiers résident déjà sur la même partition `/dev/md125` dans `~/dbsrt/data/`, et empêcherait l'inspection directe ou la restauration unitaire par `rsync` sur le Cloud.
3. **Option B3 — Modèle Hybride Unifié par un Manifeste Signé `SHA-256`** : **Retenue**.
   - **Couche logique commune** : un manifeste immuable `manifest_<UTC>_<git>.json` signé par `manifest_body_sha256`, listant le `SHA-256` individuel de chaque fichier ET le `tree_sha256` canonique de chaque dataset.
   - **Tier 1 (`o2switch` POSIX)** : pré-indexation par hardlinks `os.link` (`posix_vault_agent.py`) depuis le snapshot précédent (vérifié via `prev_hashes`) ET depuis `~/dbsrt/data/` (vérifié par `SHA-256`), suivie d'un `rsync --files-from` du seul delta manquant.
   - **Tier 2 (`Synology NAS` FTP userspace)** : flux `DeterministicTarStreamReader` générant à la volée (`0 octet` temporaire sur le NVMe local, `1 MiB` RAM) une archive PAX déterministe `packs/<dataset>_<tree_sha256[:16]>.tar` envoyée en continu à plein débit Gigabit LAN, réutilisée à coût nul dès que `tree_sha256` est inchangé.

---

## 3. Décision & Protocole d'Atomicité

1. **Ordre de validation *Crash-Safe*** : sur chaque cible, les données sont écrites dans un espace temporaire (`.staging_<id>` ou `.partial.<pid>`), contrôlées en taille exacte (et en `SHA-256` sur POSIX), renommées atomiquement, et le manifeste `manifests/<id>.json` + `latest.json` n'est publié qu'**en toute dernière étape**. Une coupure réseau ne laisse jamais un snapshot visible incomplet.
2. **Quorum d'éviction locale (`DEFAULT_MIN_EVICT_REPLICAS = 2`)** : `task data:evict` opère en `--dry-run` par défaut. En mode `--apply`, il recalcule systématiquement le `SHA-256` réel des fichiers locaux sur disque (`force_rehash=True`), exige la parité cryptographique sur au moins `2` réplicas distants sains (`o2switch` + `synology_nas`), inscrit la pierre tombale `data/.evicted_manifest.json` avant tout `unlink()`, et bloque immédiatement avec le code `EXIT_STALE_OR_DRIFT = 4` à la moindre dérive.
3. **Visibilité SSOT** : `dbsrt_data.py status` expose en `< 1 ms` le bloc `backup_state` (`BACKED_UP`, `DRIFT_DETECTED`, `evicted_datasets`).

---

## 4. Conséquences & Limites Résiduelles

- **Gains** : élimination intégrale du *Single Point of Failure* poste de dev (`3.55 GiB` d'extraits et rapports sécurisés sur 2 tiers), immunité totale aux hangs noyau CIFS (`ftplib` pur espace utilisateur), coût disque incrémental nul pour les datasets stables, et capacité de libérer à la demande `10+ GiB` sur le NVMe local sans perte.
- **Limites résiduelles** :
  - Sur `Synology FTP`, la déduplication opère à la **granularité du dataset** (`pack_filename` indexé par `tree_sha256`) : l'ajout d'un fichier `.phash` dans `__PHASH__/` produit un nouveau pack `__PHASH__` lors du snapshot suivant (tandis que `catalog.dbahash`, `snippets/`, etc. restent dédupliqués à `0 octet`). Une politique de rétention périodique des anciens packs orphelins pourra être ajoutée lorsque le volume `/home` (`1.02 TiB` libres) dépassera un seuil significatif.
  - Depuis un hotspot mobile 4G/5G où le port SSH `:22` d'`o2switch` est filtré par liste blanche IP cPanel, la cible WAN FTP passive Synology (`<NAS_HOST>:21`) demeure utilisable mais le tier `o2switch` requiert une IP autorisée en SSH.

---

## 5. Sites d'Implémentation & Couverture de Tests

- **Sources** : `scripts/data_access/constants.py`, `scripts/data_access/backup_manifest.py`, `scripts/data_access/posix_vault_agent.py`, `scripts/data_access/backup_targets.py`, `scripts/data_access/backup_cli.py`, `scripts/data_access/prober.py`, `taskfiles/skills.yml`.
- **Tests hermétiques** : `tests/tools/test_data_backup.py` (signature et altération de manifeste, streaming PAX TAR UTF-8 par petits blocs, hardlinks POSIX et immunité aux mutations de taille constante, serveur FTP passif loopback complet, refus d'éviction sur quorum insuffisant ou dérive d'octet, cycle complet `evict` $\rightarrow$ `restore`).
