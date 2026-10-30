---
title: "Guide Pratique : Migrer le Schéma PostgreSQL avec Refinery"
tags:

  - howto
  - postgres
  - migrations
  - refinery
---

# 🛠️ Guide Pratique : Migrer le Schéma PostgreSQL avec Refinery

Ce guide détaille comment appliquer les migrations de base de données PostgreSQL, vérifier leur état et créer de nouvelles migrations versionnées via l'outil **Refinery** intégré dans `pydbsrt`.

---

## Problème

Vous développez une nouvelle fonctionnalité nécessitant une évolution de schéma (nouvelle table, nouvel index SP-GiST, ajout d'une colonne). Vous devez appliquer les migrations en local, en CI et en production de façon reproductible et transactionnelle.

---

## Commandes Opérationnelles

### 1. Vérifier l'État des Migrations

Pour connaître les migrations déjà appliquées et celles en attente :

```bash
task db:status
```

Sortie typique :
```text
Applied migrations:
  V1__initial_schema
  V2__certified_couplings
  V3__secure_lang_and_cascades
  V4__rich_media_metadata
  V5__production_view_and_stream_specs
Pending migrations:
  None
```

### 2. Appliquer les Migrations en Attente

Pour exécuter toutes les migrations non encore jouées sur la base cible :

```bash
task db:migrate
```

Refinery encapsule chaque script de migration dans une transaction SQL atomique. En cas d'erreur de syntaxe ou de violation de contrainte, la transaction est intégralement annulée (`ROLLBACK`), préservant l'intégrité de la base.

### 3. Créer une Nouvelle Migration Versionnée

Toutes les migrations sont stockées sous `crates/dbsrt-index/migrations/`.

1. Respectez strictement la convention de nommage Refinery :
   `V<NUMÉRO_INCRÉMENTAL>__<description_en_snake_case>.sql`
   *Exemple* : `V4__add_media_duration_seconds.sql`.

2. Créez le fichier SQL avec des instructions DDL idempotentes :
   ```sql
   -- V4__add_media_duration_seconds.sql
   ALTER TABLE medias ADD COLUMN IF NOT EXISTS duration_seconds DOUBLE PRECISION;
   COMMENT ON COLUMN medias.duration_seconds IS 'Media total duration in seconds extracted via ffprobe';
   ```

3. Testez la migration sur votre base locale :
   ```bash
   task db:migrate
   task db:status
   ```

4. Exécutez la suite de vérification de sécurité DDL et de non-régression :
   ```bash
   task db:lint:safety
   task test:rs
   ```

---

### 4. Hydratation et Rétro-propagation (`task db:backfill:medias`)

Après une migration enrichissant les colonnes de la table `medias` (ex: V4), exécutez le script d'hydratation par lot pour peupler les métadonnées depuis les archives `.phash` et la télémétrie JSONL :

```bash
task db:backfill:medias
```

Ce script effectue un `UPSERT` par lots (`psycopg2.extras.execute_batch`) à plus de 10 000 lignes/seconde sans insérer de doublons.

---

### 5. Extraction de la Géométrie Réelle par Sonde Réseau (`task db:probe`)

Pour remplacer les déductions textuelles fragiles par des sources de vérité physiques (résolution native, géométrie `width` x `height`, codec conteneur) :

```bash
# Sonde locale via ffprobe sur le NAS Synology (12 workers parallèles)
task db:probe

# Réplication instantanée des spécifications physiques vers le Raspberry Pi 4
task rpi4:db:sync
```

---

### 6. Audit des NULLs et Inspection TUI Plein Écran (`task db:audit`, `task db:tui`)

Pour contrôler la complétude des colonnes physiques et la vue conceptuelle `v_productions` :

```bash
# Bilan exhaustif des valeurs NULL et distribution des productions
task db:audit

# Même bilan sur le Raspberry Pi 4 distant
task rpi4:db:audit

# Inspection visuelle interactive complète via Harlequin SQL TUI
task db:tui
task rpi4:db:tui
```
