---
title: "Tutoriel : Ingestion & Certification d'un Premier Média"
tags:

  - tutoriel
  - ingestion
  - certification
  - phash
---

# 🎓 Tutoriel : Ingestion & Certification d'un Premier Média

Dans ce tutoriel pas à pas, vous allez apprendre à transformer un extrait vidéo brut et son fichier de sous-titres en un **couplage certifié** dans la base de vérité terrain de `pydbsrt`.

> [!NOTE]
> **Temps estimé** : 10 minutes.
> **Objectif** : Générer une empreinte pHash, certifier le couplage temporel vidéo \( \leftrightarrow \) sous-titres et observer l'enregistrement dans PostgreSQL et le Studio.

---

## 1. Prérequis

- Avoir démarré la stack de développement (`task dev` ou `task studio`).
- Disposer d'un extrait vidéo test (`.mp4` ou `.mkv`) et d'un fichier de sous-titres correspondant (`.srt`).
- Le dépôt fournit déjà des fichiers de test dans `data/snippets/` et `data/SRT/`.

---

## 2. Étape 1 : Préparer les Fichiers Témoins

Pour ce tutoriel, nous utilisons un extrait déjà présent dans le projet :

- Fichier vidéo : `data/snippets/Apprendre_CakePHP_-_SessionHelper_mid15s.mp4`
- Fichier sous-titres : `data/SRT/Apprendre_CakePHP_-_SessionHelper_mid15s.srt`

Vérifiez la présence de ces fichiers dans votre terminal :

```bash
ls -lh data/snippets/Apprendre_CakePHP_-_SessionHelper_mid15s.mp4 data/SRT/Apprendre_CakePHP_-_SessionHelper_mid15s.srt
```

---

## 3. Étape 2 : Ingestion en Une Seule Commande (`task media:add`)

Le système met à votre disposition une façade unifiée d'ingestion. Lancez l'ingestion avec la commande suivante :

```bash
task media:add -- data/snippets/Apprendre_CakePHP_-_SessionHelper_mid15s.mp4 data/SRT/Apprendre_CakePHP_-_SessionHelper_mid15s.srt --lang fr
```

### Ce qui s'exécute en coulisses

1. **Sondage du média (`ffprobe`)** : détection automatique de la durée et du framerate de la vidéo.
2. **Calcul des empreintes pHash** : extraction des trames clés visuelles (64 bits par trame) via le moteur SIMD optimisé.
3. **Normalisation du fichier SRT** : vérification de l'encodage (UTF-8) et parsing des ancres temporelles (`00:00:01,000 --> 00:00:04,000`).
4. **Enregistrement transactionnel** : insertion dans PostgreSQL 17 des tables `medias`, `frames`, `subtitles` et `sub_frames`.
5. **Rechargement à chaud** : notification instantanée envoyée au démon en mémoire pour rendre le nouveau média immédiatement interrogeable.

Le terminal affiche un récapitulatif structuré :

```text
=== MEDIA:ADD INGESTION SUMMARY ===
Media Video      : "data/snippets/Apprendre_CakePHP_-_SessionHelper_mid15s.mp4"
Framerate (FPS)  : 25.0
pHash File       : "/tmp/Apprendre_CakePHP_-_SessionHelper_mid15s.phash"
Couplings Added  : 1
Couplings Skipped: 0
Catalog Ingested : true
```

<div class="asciinema-wrapper">
  <div class="asciinema-cast" data-cast="../../assets/casts/term_media_add.cast" data-rows="24"></div>
  <figcaption>Enregistrement terminal interactif : Ingestion déclarative et options de la façade media:add (Asciinema).</figcaption>
</div>

---

## 4. Étape 3 : Inspecter l'Enregistrement dans PostgreSQL

Vérifiez que les données ont été insérées dans la base de données relationnelle via la tâche de statut :

```bash
task status
```

Vous observez les compteurs SQL incrémentés :

- Total des médias indexés.
- Total des trames visuelles enregistrées.
- Total des sous-titres certifiés.

Vous pouvez également ouvrir l'interface **PgWeb** dans votre navigateur :

```text
http://127.0.0.1:8086
```

Sélectionnez la table `subtitles` pour visualiser les colonnes `media_hash`, `sub_hash`, `language`, et les métadonnées de certification.

---

## 5. Étape 4 : Visualiser la Ligne Temporelle dans le Studio

Rendez-vous sur l'interface du Studio :

```text
http://127.0.0.1:8765/studio
```

1. Dans la liste des médias, cliquez sur le média nouvellement ingéré.
2. Observez la **timeline bi-piste** :
   - En haut : les vignettes visuelles et leurs hashes 64-bit respectifs.
   - En bas : les blocs de sous-titres synchronisés avec leurs timestamps précis.
3. Cliquez sur une trame pour zoomer dans la matrice pHash 8×8.

<figure markdown="1">
  <img src="../../assets/screenshots/studio_backoffice_1080p.webp" alt="dbsrt Studio Timeline" width="100%">
  <figcaption>Inspection dans dbsrt Studio : Visualisation bi-piste et validation des trames vérité terrain.</figcaption>
</figure>

---

## 6. Félicitations !

Vous maîtrisez désormais le cycle complet d'ingestion :

- Détection des paramètres vidéo et calcul d'empreinte.
- Association vérité-terrain avec une piste SRT.
- Validation relationnelle dans PostgreSQL et inspection dans le Studio.

Pour apprendre à configurer votre environnement de travail et contribuer au code :
👉 **[Environnement de Développement & Contribution](03-environnement-developpement-et-contribution.md)**.
