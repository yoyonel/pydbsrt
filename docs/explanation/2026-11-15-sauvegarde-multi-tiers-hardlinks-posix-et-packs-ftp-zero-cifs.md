---
title: "Journal d'Ingénierie : Sauvegarde Multi-Tiers, Hardlinks POSIX & Streaming PAX TAR Zéro-CIFS"
tags:
  - diataxis
  - explication
  - architecture
  - backup
  - ftp
  - rsync
  - sha256
---

# 🔬 Journal d'Ingénierie : Sauvegarde Multi-Tiers, Hardlinks POSIX & Streaming PAX TAR Zéro-CIFS

Ce journal consigne les quatre verrous techniques rencontrés et résolus lors de la conception du pipeline de persistance versionnée multi-tiers (**Issue #86** / **[ADR-0008](../decisions/2026-11-15-multi-tier-deduplicated-dataset-backups.md)**), selon la grille standardisée à quatre sous-sections du projet.

---

## 1. Contournement de l'Absence SSH sur Synology NAS sans Montage Noyau `mount.cifs` ni Tempête `PASV`

### 1.1 Problématique & Mécanisme racine
Le NAS Synology (`1.02 TiB` libres sur `/home`) expose uniquement les ports `21` (FTP), `445` (SMB) et `2049` (NFS) — les démons SSH (`:22`) et `rsyncd` (`:873`) y sont désactivés. Deux approches naïves échouent :

1. Monter le partage SMB via `mount.cifs` réintroduit le piège critique documenté dans `pydbsrt-data-access` : hors Wi-Fi domestique ou sous VPN d'entreprise (`ppp0` `/16`), tout `stat()` sur un point de montage CIFS orphelin fige le processus appelant en sommeil noyau ininterruptible (`D-state`).
2. Transférer individuellement en FTP passif les `21 681` fichiers de `data/` impose `21 681` négociations `PASV` + ouvertures/fermetures de sockets TCP séquentielles et interdit toute déduplication par hardlinks côté serveur FTP.

### 1.2 Conséquences concrètes
- **Empiriquement prouvé** : sonde directe TCP/FTP sur le NAS (`port 21 OPEN`, `port 22 closed`, `port 873 closed`, extensions RFC 3659 `SIZE`, `MDTM`, `MFMT`, `MLSD` actives sur `/home` avec `1 073 278 836` KiB disponibles). Un script historique (`o2switch_ctl.sh backup-nas`) dépourvu de montage CIFS actif retombait silencieusement sur le dossier local `data/__PHASH__`, créant une illusion de sauvegarde distante.

### 1.3 Solution / Invariant opéré
Implémentation dans `scripts/data_access/backup_targets.py` (`SynologyFtpTarget`) d'un transport **100 % espace utilisateur (`ftplib` avec `FTP_DATA_TIMEOUT_SECS = 120.0`)** regroupant chaque dataset sous forme d'un pack TAR adressé par le condensat Merkle de ses membres : `packs/<dataset>_<tree_sha256[:16]>.tar`. Avant tout transfert, `MLSD` inspecte les tailles présentes dans `packs/` : si le pack de même `tree_sha256` et de taille exacte `compute_expected_tar_bytes()` existe déjà, l'envoi est intégralement court-circuité (`0 octet` transféré).

### 1.4 Avantages, Inconvénients & Limites résiduelles
- **Avantages** : `9` à `11` flux TCP continus à débit maximal LAN au lieu de `21 681` transactions `PASV` ; immunité absolue aux hangs noyau `D-state` ; déduplication inter-snapshots sans SSH.
- **Limites résiduelles** : la modification d'un seul fichier au sein d'un gros répertoire (`__PHASH__/`) régénère le pack complet de ce dataset sur le NAS (alors qu'`o2switch` ne transfère que le fichier modifié).

---

## 2. Piège des Mutations à Taille Constante sur le Pré-Liage par Hardlinks POSIX

### 2.1 Problématique & Mécanisme racine
Lorsqu'un second snapshot est préparé sur un coffre POSIX (`posix_vault_agent.py`), réutiliser par `os.link()` un fichier du snapshot précédent uniquement sur égalité de chemin relatif et de taille (`prev_src.stat().st_size == size_bytes`) est **mathématiquement faux** : un fichier JSON de configuration, de score ou un en-tête binaire de taille fixe réécrit avec le même nombre d'octets conserve exactement `st_size` (et potentiellement la même seconde `st_mtime` si modifié dans la même seconde UNIX), mais possède un `SHA-256` différent.

### 2.2 Conséquences concrètes
- **Hermétiquement prouvé** (`tests/tools/test_data_backup.py::test_posix_vault_seed_hardlinks_and_point_in_time_restore`) : la modification de `benchmarks/sub/score.json` de `{"acc": 1.0}` (`12` octets) vers `{"acc": 0.5}` (`12` octets) liait à tort l'ancien inode (`linked_from_prev == 6` au lieu de `5`), propageant silencieusement l'ancienne valeur dans le nouveau snapshot et faisant échouer le commit ou corrompant l'historique.

### 2.3 Solution / Invariant opéré
**Invariant (`ADR-0008`)** : dans `scripts/data_access/posix_vault_agent.py` (`_resolve_prev_snapshot` et `_try_hardlink_existing`), le pré-liage depuis `snapshots/<prev_id>/data/` charge systématiquement la table `prev_hashes` (`rel_path -> sha256`) du manifeste vérifié `<prev_id>.json` et n'autorise `os.link()` que si **`prev_hashes.get(rel) == exp_sha` ET `st_size == size_bytes`**. Pour le répertoire de production actif (`--seed-data-dir ~/dbsrt/data`), non couvert par un manifeste immuable, `_sha256_file(seed_src) == exp_sha` est systématiquement calculé avant `os.link()`.

### 2.4 Avantages, Inconvénients & Limites résiduelles
- **Avantages** : complexité mémoire/temps $\mathcal{O}(1)$ par fichier (`~15 ms` pour charger `21 681` entrées JSON) sans relire les octets des snapshots déjà certifiés, et détection garantie à `100 %` des réécritures *in-place* de même taille.
- **Limites résiduelles** : suppose que les fichiers déjà commis dans `snapshots/<prev_id>/` restent en lecture seule (non mutés manuellement hors de l'agent).

---

## 3. Streaming PAX TAR Déterministe à Zéro Disque Temporaire (`DeterministicTarStreamReader`)

### 3.1 Problématique & Mécanisme racine
Empaqueter `catalog.dbahash` (`5.25 GiB`) ou `__AHASH__/` (`4.8 GiB`) dans un fichier `.tar` temporaire sous `/tmp` ou `data/` avant envoi FTP consommerait temporairement jusqu'à `5.25 GiB` de NVMe local et doublerait les écritures SSD. En outre, le format TAR POSIX `PAX_FORMAT` insère dynamiquement des blocs d'extension `PaxHeader` (`1 024` octets supplémentaires) dès qu'un chemin relatif comporte des caractères UTF-8 non-ASCII (ex. `Comédie_éàô_mid15s.mp4`) ou dépasse `100` octets, rendant fausse toute formule simpliste `512 + size + pad`.

### 3.2 Conséquences concrètes
- **Hermétiquement et empiriquement prouvé** : un contrôle strict `ftp.size(remote_tmp) == expected_total_bytes` après `STOR` rejetterait systématiquement tout dataset contenant des titres français accentués si la longueur des en-têtes PAX n'était pas modélisée à l'identique.

### 3.3 Solution / Invariant opéré
`DeterministicTarStreamReader` (`scripts/data_access/backup_manifest.py`) implémente `io.RawIOBase.read(size)` en générant les en-têtes via une fonction pure unique `build_tar_header_bytes(rel_path, size_bytes, mtime_epoch)` (`uid=0`, `gid=0`, `uname=""`, `gname=""`, `mode=0644`, `tarfile.PAX_FORMAT`). `expected_total_bytes` somme la longueur exacte de ces mêmes tampons d'en-tête, les tailles fichiers alignées sur `TAR_BLOCK_BYTES = 512` et les `2` blocs nuls de fin (`TAR_EOF_BLOCKS_COUNT = 2`), tout en lisant les payloads par tranches de `BACKUP_HASH_READ_CHUNK_BYTES = 1 MiB`.

### 3.4 Avantages, Inconvénients & Limites résiduelles
- **Avantages** : **`0 octet` écrit sur le disque local**, empreinte RAM constante (`~1 MiB`), compatibilité native avec GNU `tar` et `tarfile.open(mode="r|")`, et vérification au bit près par `FTP SIZE` avant renommage atomique `RNFR/RNTO`.
- **Limites résiduelles** : si un fichier source local est tronqué par un processus tiers pendant le transfert réseau d'un pack, `DeterministicTarStreamReader` lève immédiatement `OSError`, avortant proprement le fichier `.partial` sans écraser le pack valide précédent.

---

## 4. Garde-Fous d'Éviction Locale : Re-Hachage Forcé & Quorum Multi-Réplicas

### 4.1 Problématique & Mécanisme racine
Autoriser la suppression locale de plusieurs gigaoctets (`task data:evict`) en se fiant uniquement au cache local `(size, mtime_ns) -> sha256` ou à la présence d'un seul serveur distant expose à deux modes de perte irréversible : (a) une modification locale récente non sauvegardée ou altérée sans changement de taille apparente, et (b) une panne disque sur l'unique cible distante après suppression locale.

### 4.2 Conséquences concrètes
- **Hermétiquement prouvé** (`tests/tools/test_data_backup.py::test_synology_ftp_pack_dedup_corruption_and_evict_restore`) :
  1. Avec 1 seul réplica disponible lorsque `DEFAULT_MIN_EVICT_REPLICAS = 2`, `run_evict_cmd` retourne `EXIT_STALE_OR_DRIFT = 4` (`blocked_unverified_replica`) et supprime `0` fichier.
  2. Toute altération d'un seul octet local (`{"acc": 1.0}` $\rightarrow$ `{"acc": 0.9}`) entre le backup et l'éviction est immédiatement bloquée (`hash_drift`), `0` fichier supprimé.

### 4.3 Solution / Invariant opéré
Dans `scripts/data_access/backup_cli.py` (`run_evict_cmd`) :

1. Refus immédiat sur tout lien symbolique (`is_symlink()`, protection des `git worktree`).
2. Appel obligatoire de `build_backup_manifest(..., force_rehash=True)` relisant intégralement sur disque les octets candidats à la suppression.
3. Exécution de `verify_snapshot()` + `fetch_manifest()` sur chaque cible joignable et vérification exhaustive `(rel_path, sha256, size_bytes)` sur au moins `--min-replicas` cibles (`2` par défaut).
4. Mode `--dry-run` actif par défaut tant que `--apply` n'est pas fourni explicitement, et écriture atomique préalable du pointeur de restauration dans `data/.evicted_manifest.json`.

### 4.4 Avantages, Inconvénients & Limites résiduelles
- **Avantages** : garantie mathématique de zéro perte silencieuse et traçabilité immédiate des datasets évincés dans `dbsrt_data.py status`.
- **Limites résiduelles** : `verify_snapshot()` par défaut pendant `evict` contrôle la signature cryptographique du manifeste distant et les tailles exactes des packs/fichiers distants (le recalcul complet `SHA-256` distant de `17 GiB` via `--check-hashes` reste disponible à la demande via `task data:verify -- --check-hashes`).
