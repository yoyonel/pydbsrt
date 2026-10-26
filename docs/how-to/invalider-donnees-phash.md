---
title: "Guide Pratique : Invalider et Purger des Données pHash"
tags:

  - howto
  - invalidation
  - purge
  - cache
  - hot-reload
---

# 🛠️ Guide Pratique : Invalider et Purger des Données pHash

Ce guide explique comment retirer proprement un média obsolète, erroné ou corrompu de la base de données relationnelle et forcer le rafraîchissement immédiat de l'index en mémoire de `dbsrt-server` sans interruption de service.

---

## Problème

Une vidéo a été ré-encodée ou renommée, ou un couplage SRT a été certifié avec une mauvaise piste de langue. Vous devez supprimer les anciennes entrées pour éviter les faux positifs lors de la recherche par pHash.

---

## Procédure

### 1. Identifier le Hash SHA-256 du Média

Pour retrouver l'identifiant unique du média à invalider :

```bash
# Lister les médias enregistrés avec leur hash SHA-256
task catalog:status
```

Ou en interrogeant la base via `psql` / `PgWeb` :

```sql
SELECT id, filename, media_hash FROM medias WHERE filename LIKE '%titre_a_purger%';
```

### 2. Supprimer le Média et ses Dépendances dans PostgreSQL

La base de données applique des contraintes de clés étrangères avec suppression en cascade (`ON DELETE CASCADE`). La suppression de l'entrée dans la table `medias` purge automatiquement les tables associées (`frames`, `subtitles`, `sub_frames`) :

```sql
-- Suppression atomique en cascade
DELETE FROM medias WHERE media_hash = '<MEDIA_HASH>';
```

### 3. Nettoyer le Fichier de Couplages Local (`couplings.json`)

Si le couplage était également inscrit dans le fichier de registre Zero-DB :

1. Ouvrez `data/SRT/couplings.json`.
2. Supprimez le bloc JSON correspondant au `media_hash`.
3. Supprimez le fichier d'empreinte correspondant sous `data/__PHASH__/<nom>.phash`.

### 4. Forcer le Rechargement à Chaud du Serveur (`server:reload`)

Pour que le démon Axum prenne en compte la suppression sans redémarrer le processus ni déconnecter les clients VLC :

```bash
task server:reload
```

Le serveur vide ses structures MIH / mmap en mémoire vive et recharge instantanément l'état assaini de la base.
Un code retour HTTP 200 confirme l'opération :

```json
{"status": "ok", "reloaded_medias": 20, "elapsed_ms": 2}
```
