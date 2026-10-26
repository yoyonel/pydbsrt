---
title: "Guide Pratique : Ajouter et Indexer un Média (Façade Unifiée)"
tags:

  - howto
  - media-add
  - ingestion
  - indexation
---

# 🛠️ Guide Pratique : Ajouter et Indexer un Média via `task media:add`

Ce guide présente les commandes et options pour ingérer un nouveau fichier vidéo, extraire son empreinte pHash et certifier automatiquement ses pistes de sous-titres associées.

---

## Problème

Vous disposez d'une vidéo (film, épisode d'animé ou série) et d'un ou plusieurs fichiers de sous-titres `.srt`. Vous souhaitez les intégrer dans la base relationnelle PostgreSQL et dans le catalogue en mémoire de `dbsrt-server` sans taper manuellement les commandes de bas niveau (`ffmpeg`, `dbsrt extract`, `dbsrt certify-coupling`, `sql copy`).

---

## Cas d'Utilisation Fréquents

### 1. Ingestion Nominale avec Auto-Découverte du Sous-Titre

Si le fichier de sous-titres se trouve dans le même répertoire que la vidéo ou dans un sous-dossier `subs/` avec le même radical :

```bash
# Exemple : Le film mon_film.mkv et son sous-titre mon_film.srt sont dans le même dossier
task media:add -- /chemin/vers/mon_film.mkv
```

La façade :

1. Détecte le framerate de la vidéo via `ffprobe`.
2. Découvre automatiquement `mon_film.srt` ou `mon_film.fr.srt`.
3. Extrait l'empreinte pHash 64-bit sous `/tmp/`.
4. Certifie le couplage dans `couplings.json` et PostgreSQL.
5. Déclenche un rechargement à chaud atomique (`zero-downtime reload`) du serveur.

<div class="asciinema-wrapper">
  <div class="asciinema-cast" data-cast="../../assets/casts/term_media_add.cast" data-rows="24"></div>
  <figcaption>Enregistrement terminal interactif : Ingestion unifiée, auto-découverte et options de ligne de commande (Asciinema).</figcaption>
</div>

### 2. Spécification Explicite du Fichier SRT et de la Langue

Lorsque le fichier de sous-titres porte un nom arbitraire :

```bash
task media:add -- /data/series/s01e01.mp4 /data/subtitles/custom_french.srt --lang fr
```

### 3. Forcer le Recalcul d'une Empreinte Déjà Existante (`--force`)

Par défaut, si l'empreinte pHash ou le couplage existe déjà (déduplication par contenu), l'opération est sautée de façon idempotente. Pour forcer une réextraction complète :

```bash
task media:add -- /data/film.mkv --force
```

### 4. Ingestion par Lots (Batch Scripting)

Pour ingérer récursivement un dossier de vidéos :

```bash
find /data/video_corpus/ -name "*.mp4" -o -name "*.mkv" | while read -r video; do
  task media:add -- "$video"
done
```

---

## Validation Immédiate

Pour vous assurer que le média est actif et prêt à répondre aux requêtes de recherche :

```bash
# Vérifier la présence du média dans le catalogue en mémoire
task catalog:status

# Afficher les métriques de la stack
task status
```
