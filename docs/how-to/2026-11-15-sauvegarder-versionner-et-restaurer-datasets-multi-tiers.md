---
title: "Sauvegarder, Versionner, Évincer et Restaurer les Datasets Multi-Tiers (o2switch & Synology NAS)"
tags:
  - diataxis
  - howto
  - backup
  - restore
  - o2switch
  - synology
  - sha256
---

# 🛠️ Sauvegarder, Versionner, Évincer et Restaurer les Datasets Multi-Tiers

Ce guide pratique détaille comment sécuriser les jeux de données non versionnés de `data/` (`catalog.dbphash`, `catalog.dbahash`, `__PHASH__/`, `__AHASH__/`, `SRT/`, `snippets/`, `snippets_ahash/`, `reports/`, `benchmarks/`, `seed/`) sur les deux paliers distants (**Cloud `o2switch`** et **NAS Synology LAN en FTP sans montage CIFS**), vérifier l'intégrité `SHA-256`, libérer de l'espace disque local sans risque et restaurer un état historique précis.

---

## 1. Prérequis

- Accès SSH configuré vers `o2switch` (alias `~/.ssh/config`) lorsque vous êtes sur une IP autorisée.
- Identifiants FTP du NAS Synology configurés dans `.env` (`NAS_USER`, `NAS_LAN_HOST`) et `pass` (`NAS_PASS_ENTRY`), utilisés en pur espace utilisateur (`ftplib`, aucun montage noyau `mount.cifs` requis).

---

## 2. Inspecter l'Inventaire et Générer un Manifeste `SHA-256` Local

Pour calculer (ou mettre à jour en `< 1 s` via le cache incrémental `~/.cache/dbsrt/backup_hash_cache_v1.json`) les empreintes `SHA-256` et les condensats d'arbre (`tree_sha256`) des datasets locaux :

```bash
task data:manifest
```

Pour forcer la relecture intégrale des octets sur disque sans utiliser le cache `mtime` :

```bash
task data:manifest -- --force-rehash
```

---

## 3. Lancer une Sauvegarde Multi-Tiers Dédupliquée (`o2switch` + `Synology NAS`)

### 3.1 Simulation à blanc (`--dry-run`)

```bash
task data:backup -- --dry-run
```

### 3.2 Sauvegarde complète vers les 2 tiers par défaut

```bash
task data:backup
```

- **Sur `o2switch` (`~/dbsrt/backups/snapshots/<id>/`)** : tous les fichiers déjà présents dans `~/dbsrt/data/` ou dans le snapshot précédent sont liés par **hardlinks POSIX (`0 octet` disque supplémentaire, `0 octet` réseau ré-uploadé)** ; seuls les nouveaux fichiers transitent via `rsync --bwlimit=20000`.
- **Sur `Synology NAS` (`ftp://<nas>:21/home/pydbsrt-backups/packs/`)** : seuls les datasets dont le `tree_sha256` a changé sont streamés à la volée en format TAR PAX déterministe (`0 octet` temporaire sur le NVMe local).

### 3.3 Cibler un dataset ou un tier spécifique

```bash
# Sauvegarder uniquement les rapports et benchmarks sur les 2 tiers
task data:backup -- --dataset reports --dataset benchmarks

# Sauvegarder vers un disque externe local monté
task data:backup -- --target local:/mnt/usb_vault
```

---

## 4. Lister et Vérifier Cryptographiquement les Snapshots Distants

### 4.1 Consulter l'historique des snapshots publiés

```bash
task data:snapshots
```

### 4.2 Vérifier les signatures `SHA-256` et les tailles de packs/fichiers

```bash
# Contrôle rapide (signatures manifestes + présence et tailles exactes)
task data:verify

# Contrôle bit-à-bit complet (recalcule tous les SHA-256 côté distant)
task data:verify -- --check-hashes
```

L'état de fraîcheur local vis-à-vis du dernier snapshot est également remonté instantanément dans le résolveur SSOT :

```bash
task data:status
```

---

## 5. Libérer de l'Espace Disque sur le Poste de Dév (`task data:evict`)

!!! important "Garde-fou zéro perte (`DEFAULT_MIN_EVICT_REPLICAS = 2`)"
    `task data:evict` fonctionne en **`--dry-run` par défaut**, recalcule systématiquement le `SHA-256` réel des fichiers locaux sur disque, et refuse toute suppression (code de sortie `4`) si au moins **2 réplicas distants indépendants** (`o2switch` + `synology_nas`) ne détiennent pas exactement chaque fichier vérifié.

### 5.1 Vérifier l'éligibilité et le gain disque (`dry-run`)

```bash
task data:evict -- catalog.dbahash snippets
```

### 5.2 Exécuter l'éviction après validation de parité

```bash
task data:evict -- catalog.dbahash snippets --apply
```

Les datasets évincés sont tracés dans `data/.evicted_manifest.json` et signalés dans `evicted_datasets` par `task data:status`.

---

## 6. Restaurer un Dataset ou un Fichier Précis (`task data:restore`)

La restauration sélectionne automatiquement la source vérifiée la plus rapide disponible (**Synology NAS LAN Gigabit** en priorité $\rightarrow$ fallback **Cloud `o2switch`**), ignore les fichiers locaux déjà intacts, contrôle le `SHA-256` de chaque fichier extrait avant écriture finale, et nettoie automatiquement `data/.evicted_manifest.json`.

```bash
# Restaurer un dataset évincé depuis le dernier snapshot vérifié
task data:restore -- catalog.dbahash

# Restaurer un dataset à une date/version précise (point-in-time rollback)
task data:restore -- srt_store --snapshot 20261115T110900Z_fe26d89

# Restaurer un fichier unique
task data:restore -- --path SRT/couplings.json
```

---

## 7. Pour Aller Plus Loin

- **Décision d'architecture** : [ADR-0008 — Persistance Multi-Tiers Dédupliquée & Manifestes `SHA-256`](../decisions/2026-11-15-multi-tier-deduplicated-dataset-backups.md)
- **Mécanismes internes & pièges évités** : [Journal d'Ingénierie — Hardlinks POSIX & Streaming PAX TAR Zéro-CIFS](../explanation/2026-11-15-sauvegarde-multi-tiers-hardlinks-posix-et-packs-ftp-zero-cifs.md)
- **Référence Taskfile** : [Catalogue des Commandes Taskfile](../reference/taskfile.md)
