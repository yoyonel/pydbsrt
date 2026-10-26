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
task db:migrate:status
```

Sortie typique :
```text
Applied migrations:
  V1__initial_schema (applied on 2026-10-15)
  V2__add_spgist_hamming_index (applied on 2026-10-18)
  V3__add_sub_frames_table (applied on 2026-10-22)
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
   task db:migrate:status
   ```

4. Exécutez la suite de tests d'intégration de base de données pour vérifier la non-régression :
   ```bash
   task test:rs
   ```
