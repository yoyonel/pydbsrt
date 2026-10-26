---
title: "Référence : Formats de Fichiers & Spécifications Binaires"
tags:

  - reference
  - formats
  - binaire
  - dbphash
  - json
---

# 📖 Référence : Formats de Fichiers & Spécifications Binaires

Cette page détaille la disposition binaire et le schéma des formats de données propriétaires et normalisés utilisés par `pydbsrt`.

---

## 1. Format Binaire Brut d'Empreinte (`.phash`)

Le format unitaire `.phash` stocke la séquence brute d'empreintes perceptuelles d'un média sans en-tête lourd :

- **Taille de l'empreinte** : 64 bits (8 octets non signés, `uint64_t`).
- **Endianness** : Little-endian standard IEEE.
- **Cadence implicite** : Définie par le nom de fichier ou la table `medias` (typiquement 25.0 fps ou 1 fps selon la phase).
- **Calcul du timestamp** : Pour la trame d'indice \( i \) (à partir de 0) :
  \( t_i = \frac{i}{\text{fps}} \text{ (en secondes)} \)

---

## 2. Spécification de l'Archive Haute Densité (`.dbphash` v2)

Le format `.dbphash` regroupe plusieurs millions d'empreintes dans un fichier contigu unique projetable directement en mémoire via `mmap` sans allocation tas (`Zero-Allocation`).

### En-Tête Binaire (Header) — 64 octets

```text
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       Magic Bytes: 'D' 'B' 'P' 'H'            |  Version: 2   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| Flags (16-bit)|             Réservé (32-bit)                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Nombre total de Médias (uint64)               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Nombre total de Trames (uint64)               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Offset de la Table des Médias (uint64)        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Offset de la Table des Index MIH (uint64)     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Corps du Fichier
1. **Section Trames** : Tableau contigu de `u64` pHash ordonné par média.
2. **Table des Médias** : Pour chaque média :
   - `media_hash` : 16 octets (128 bits).
   - `start_frame_idx` : `uint64`.
   - `frame_count` : `uint32`.
   - `fps` : `float32`.
   - Longueur du titre (uint16) suivi de la chaîne UTF-8 du titre.
3. **Index MIH (Multi-Index Hashing)** : Découpage des 64 bits en 4 chunks de 16 bits pour un routage direct dans les tables de hachage inversées.

---

## 3. Schéma du Registre Vérité-Terrain (`couplings.json`)

Le fichier `data/SRT/couplings.json` garantit l'alignement certifié en mode Zero-DB (sans PostgreSQL) :

```json
{
  "version": "1.0",
  "generated_at": "2026-10-25T20:00:00Z",
  "couplings": [
    {
      "media_hash": "a1b2c3d4e5f60718293a4b5c6d7e8f90",
      "media_name": "Berserk.1997.EP01.MULTi.1080p.WEB.x264-FRATERNiTY",
      "fps": 25.0,
      "phash_path": "data/__PHASH__/Berserk.1997.EP01.phash",
      "subtitles": [
        {
          "subtitles_hash": "7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c",
          "filename": "Berserk.1997.EP01.fre.srt",
          "lang": "fr",
          "srt_path": "data/SRT/Berserk.1997.EP01.fre.srt",
          "certified": true,
          "cues_count": 242
        }
      ]
    }
  ]
}
```

---

## 4. Spécification des Fichiers de Sous-Titres Normalisés (`.srt`)

- **Jeu de caractères** : Strictement `UTF-8` sans BOM (Byte Order Mark). Les encodages legacy (CP1252, ISO-8859-1) sont automatiquement convertis lors de l'ingestion par la bibliothèque `chardetng` / `encoding_rs`.
- **Sauts de ligne** : Normalisés en fins de ligne Unix (`\n`, LF).
- **Format temporel** : `HH:MM:SS,mmm --> HH:MM:SS,mmm` (virgule décimale pour les millisecondes).
