---
title: "Guide Pratique : Certifier un Couplage Média-Sous-Titres"
tags:

  - howto
  - certification
  - couplage
  - vérité-terrain
---

# 🛠️ Guide Pratique : Certifier un Couplage Média-Sous-Titres

Ce guide explique comment créer ou modifier une relation de **vérité terrain** entre une empreinte visuelle extraite et un fichier de sous-titres, afin de permettre le recalage automatique d'autres releases.

---

## Contexte : Qu'est-ce qu'un Couplage Certifié ?

Dans `pydbsrt`, un sous-titre ne contient aucune information visuelle. Pour qu'un sous-titre puisse être aligné sur d'autres versions d'une même vidéo, il doit obligatoirement être **certifié** contre un flux vidéo de référence dont l'alignement temporel est garanti parfait.

Le couplage certifié stocke :

- L'identifiant du média de référence (`media_hash` SHA-256).
- Le hash du sous-titre de référence (`sub_hash` SHA-256).
- La table d'ancrage liant chaque timestamp de réplique aux trames pHash correspondantes (`sub_frames`).

---

## Procédure

### 1. Certification Directe via le CLI Rust

Si vous disposez d'un fichier pHash déjà extrait (`.phash`) et d'un sous-titre (`.srt`) :

```bash
dbsrt certify-coupling \
  --phash-file data/__PHASH__/Berserk.1997.EP01.phash \
  --srt-file data/SRT/Berserk.1997.EP01.fre.srt \
  --language fr \
  --media-name "Berserk EP01"
```

Cette opération :

1. Parse le fichier SRT et extrait tous les intervalles temporels de sous-titres (`start_ms`, `end_ms`, `text`).
2. Associe chaque intervalle aux trames pHash de la vidéo comprises dans cette fenêtre.
3. Écrit l'entrée dans le registre local `data/SRT/couplings.json`.
4. Si PostgreSQL est actif, insère les lignes correspondantes dans les tables `subtitles` et `sub_frames`.

### 2. Certification en Lot de Tous les Médias du Répertoire

Pour synchroniser automatiquement tous les fichiers `.phash` et `.srt` présents dans le dépôt :

```bash
task coupling:certify
```

### 3. Modifier la Langue d'un Couplage Existant (`coupling:relabel`)

Si une piste a été importée avec un code de langue erroné (ex: `und` ou `en` au lieu de `fr`) :

```bash
task coupling:relabel -- --media <hash_du_media> --lang fr
```

### 4. Vérifier le Couplage dans dbsrt Studio

1. Ouvrez `http://127.0.0.1:8765/studio`.
2. Cliquez sur le titre du média.
3. Vérifiez visuellement que le texte des sous-titres en bas coïncide avec les actions affichées dans les vignettes de trames clés.
