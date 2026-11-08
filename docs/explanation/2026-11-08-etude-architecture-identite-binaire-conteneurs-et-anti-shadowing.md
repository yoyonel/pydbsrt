---
title: "Étude d'Architecture : Identification Binaire Unique des Conteneurs Vidéo & Élimination du Shadowing d'Empreintes pHash"
tags:
  - explication
  - architecture
  - phash
  - identification
  - shadowing
  - container-hash
---

# 💡 Étude d'Architecture : Identification Binaire Unique des Conteneurs Vidéo & Élimination du Shadowing d'Empreintes pHash

**Date :** 8 novembre 2026
**Auteur :** Lionel ATTY (Antigravity pair programming)
**Statut :** Brouillon de Travail / R&D en Cours (À affiner avant implémentation)
**Contexte :** Constat de faux rejets d'audit (statut *Élagué*) sur des œuvres majeures (ex: *Spider-Man*) causé par le masquage (*shadowing*) des véritables médias complets (fichiers MKV de 10 à 26 Go, durées > 2h) par des empreintes partielles issues de snippets de tests (26 à 30 secondes) portant le même nom textuel de release dans `data/__PHASH__/`.

---

## 1. Diagnostic de la Problématique de Shadowing

### 1.1 Collusion par Nommage Textuel

Actuellement, les fichiers d'empreintes perceptuelles `.phash` sont indexés et nommés selon la chaîne textuelle du fichier d'origine :

- Extrait court de 26s : `Spider-Man - Brand New Day (2026) 2160p....phash` (taille 5 Ko, 636 frames).
- Œuvre complète de 2h30 : `Spider-Man - Brand New Day (2026) 2160p....phash` (taille ~4 Mo, 216 000 frames).

Lorsqu'un extrait de banc d'essai est généré dans `data/__PHASH__/`, il adopte l'identité textuelle de la release. Par conséquent :

1. Le daemon de surveillance et d'ingestion considère que l'œuvre est « Déjà traitée / Présente en cache » et ignore le fichier vidéo complet de 26 Go.
2. Les bancs d'évaluation de suivi de trajectoire multi-coupes (*multi-cuts*) découpent un fragment de 26 secondes en 20 tranches de 1.3s (tombant sur des logos et écrans noirs) au lieu de balayer l'œuvre sur 2h30, provoquant un rejet artificiel sous le seuil de consensus (⚪ *Élagué*).

### 1.2 Découplage entre Flux Visuel (pHash) et Conteneur Physique

Le perceptual hash (`pHash`) modélise la texture visuelle basse fréquence de chaque trame décodée. Cependant :

- Le pHash ne contient intrinsèquement **aucune signature cryptographique du fichier physique** dont il est issu.
- Deux fichiers ayant des durées, des conteneurs, des flux audio ou des découpages temporels radicalement différents peuvent partager la même sémantique textuelle s'ils ne sont pas formellement rattachés à l'empreinte binaire de leur conteneur source.

---

## 2. Spécification de l'ID Binaire Unique Conteneur (`container_bin_hash`)

Pour garantir un rattachement univoque et mathématiquement infalsifiable sans introduire de goulot d'étranglement I/O sur des fichiers vidéo de 10 à 80 Go, le système doit calculer un identifiant de conteneur en temps constant $O(1)$.

### 2.1 Algorithme Hybride OpenSubtitles-like + Checksum Début/Fin

- **Taille du scan** : 128 Ko (64 Ko initiaux + 64 Ko finaux) combinés à la taille exacte du fichier en octets (`file_size`).
- **Fonction de hachage** :
  - Standard OpenSubtitles 64-bit uint checksum (compatibilité avec les référentiels publics).
  - SHA-256 tronqué sur `[size_bytes || head_64k || tail_64k]` produisant un identifiant hexadécimal univoque de 16 ou 32 caractères.
- **Complexité** : < 2 ms par fichier, même sur liaisons SMB/CIFS distantes (Freebox NAS, Synology).

### 2.2 Propriété Clé

$$\text{container\_bin\_hash}(\text{Fichier\_Source\_26Go}) \ne \text{container\_bin\_hash}(\text{Extrait\_Snippet\_26s})$$

Le fichier de 26 Go et l'extrait de 26 secondes possèdent deux identifiants binaires distincts et indépendants.

---

## 3. Modélisation des Métadonnées d'Empreinte (Sidecar / En-tête)

Chaque ensemble d'empreintes pHash doit être accompagné d'un bloc de métadonnées d'intégrité (sidecar JSON ou en-tête structuré) :

```json
{
  "container_bin_hash": "a1b2c3d4e5f60718",
  "source_file_size": 27575971840,
  "source_file_name": "Spider-Man - Brand New Day (2026) 2160p.mkv",
  "is_complete_media": true,
  "sample_range": null,
  "fps": 24.0,
  "frame_count": 216000,
  "phash_digest": "3c9a66708998e2e0e006f1609c6c46d5",
  "created_at": "2026-11-08T16:45:00Z"
}
```

Pour un cut / snippet de banc d'essai :

```json
{
  "container_bin_hash": "e5f60718a1b2c3d4",
  "source_file_size": 1677721,
  "source_file_name": "Spider-Man_-_Brand_New_Day_mid15s.mp4",
  "is_complete_media": false,
  "sample_range": {
    "start_sec": 12.0,
    "duration_sec": 15.0
  },
  "parent_media_bin_hash": "a1b2c3d4e5f60718",
  "fps": 24.0,
  "frame_count": 360,
  "phash_digest": "78a9bc01..."
}
```

Le champ `parent_media_bin_hash` permet, si le lien de parenté est connu lors de l'extraction, de tracer l'origine exacte du cut par rapport au master.

---

## 4. Plan de Migration Multi-Environnements (Local, o2switch, RPi4)

### 4.1 Rétro-Qualification des Données Historiques

Un outil d'audit et de migration (`scripts/migrate_phash_identities.py`) doit analyser l'ensemble des fichiers `.phash` existants sur les dépôts de stockage :

1. **Cas Nominal (Fichier vidéo source existant sur disque)** :

   Calculer le `container_bin_hash` du conteneur physique et émettre le sidecar certifié.

2. **Cas Orphelin (Empreinte seule sans vidéo source présente localement)** :

   - Qualification par seuil temporel et heuristique de nommage :
     - Si $\text{durée} < 120\,\text{s}$ ou nommage `_mid15s` / `snippet` : marqué `is_complete_media: false` et consigné dans le registre des bancs de tests.
     - Si $\text{durée} > 600\,\text{s}$ : considéré provisionnellement comme master complet.

### 4.2 Évolution du Schéma PostgreSQL (o2switch)

Ajout des attributs d'identité conteneur dans la table `medias` :

- `container_bin_hash CHAR(16)` avec index unique optionnel.
- `is_complete_media BOOLEAN NOT NULL DEFAULT TRUE`.
- `source_file_size BIGINT`.

### 4.3 Réactivation du Processing des Médias Masqués

1. **Suppression du court-circuit basé sur le nom** :

   Le daemon d'ingestion ne vérifie plus si `phash_exists(stem)`, mais si `has_complete_phash(media.container_bin_hash)`.

2. **Campagne de rattrapage** :

   Les vidéos de `storage_root/Disque_2/Téléchargements/` dont le `container_bin_hash` n'a pas d'empreinte complète associée sont automatiquement détectées comme non indexées et programmées dans la file d'ingestion.

---

## 5. Prochaines Étapes de Réflexion (Points Ouverts)

1. Arbitrer entre format sidecar JSON externe (`.phash` + `.json`) ou conteneur binaire unique avec en-tête typé (type Magic Header `DBSRT_PHASH\x01` incluant métadonnées et frames).
2. Définir le protocole de synchronisation différentielle des identités binaires entre nœuds locaux (NAS/RPi4) et cloud (o2switch).
3. Spécifier la commande de certification unifiée `dbsrt media add` intégrant nativement ce calcul d'identité binaire.
