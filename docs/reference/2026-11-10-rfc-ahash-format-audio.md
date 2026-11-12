---
title: "RFC : Format binaire .ahash v1.0 — Empreinte spectrale audio"
tags:
  - reference
  - formats
  - audio
  - ahash
  - rfc
---

# 📖 RFC : Format Binaire `.ahash` v1.0 — Empreinte Spectrale Audio

- **Statut** : Brouillon Spécifié & Validé (R&D)
- **Auteur** : pydbsrt Architecture Team
- **Date** : 2026-11-10
- **Branche** : `feat/65-ahash-audio-fingerprint`

---

## 1. Objectif & Contexte

Le format `.ahash` (Audio Hash) définit la disposition binaire d'une séquence temporelle d'empreintes spectrales audio 64-bit, calculée selon l'algorithme différentiel de Haitsma-Kalker (2002).

Ce format est conçu pour :

1. **Être 100% interopérable avec l'infrastructure métrique existante** : chaque trame audio produit un entier non signé 64-bit (`u64`), compatible avec la distance de Hamming matérielle (`popcnt`), les structures d'indexation BK-Tree et Multi-Index Hashing (MIH).
2. **Garantir l'alignement mémoire 64-bit strict pour `mmap`** : en-tête dimensionné à un multiple exact de 8 octets (176 octets) pour éviter tout défaut d'alignement ou panique sur architecture ARM64 (Raspberry Pi 4).
3. **Résoudre les angles morts du pHash visuel** : offrir une signature dense et contrastée lors de génériques noirs, intros statiques, transitions ou tutoriels vidéo avec narration.

---

## 2. Disposition Binaire Spécifiée

### 2.1 Schéma Global de la Mémoire

```text
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       MAGIC (8 octets)                        |
|                     b"AHASH\0\1\0"                            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|         VERSION (u16)         |       SAMPLE_RATE (u32)...    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       ...SAMPLE_RATE          |        FRAME_SIZE (u16)       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|         HOP_SIZE (u16)        | NUM_BANDS(u8) | SPECTRAL_LAG  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      RESERVED (12 octets, 0x00)               |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     FRAME_COUNT (u64 LE)                      |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|             BAND_BOUNDS (33 x u32 LE = 132 octets)            |
|                              ...                              |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 ALIGNMENT_PADDING (4 octets, 0x00)            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|             PAYLOAD : FRAME_COUNT x u64 LE                    |
|             (Offset 176, garanti aligné 8 octets)             |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

---

### 2.2 Table des Champs de l'En-tête (Header : 176 octets)

| Champ | Offset (octets) | Taille (octets) | Type | Rôle / Valeur Canonique |
|---|---|---|---|---|
| `MAGIC` | `0` | 8 | `[u8; 8]` | Signature constante `b"AHASH\0\1\0"` (v1.0). |
| `VERSION` | `8` | 2 | `u16 LE` | Version de la spécification (`1`). |
| `SAMPLE_RATE` | `10` | 4 | `u32 LE` | Fréquence d'échantillonnage audio en Hz (`5512`). |
| `FRAME_SIZE` | `14` | 2 | `u16 LE` | Taille de la fenêtre FFT en échantillons (`2048`). |
| `HOP_SIZE` | `16` | 2 | `u16 LE` | Pas d'avancement temporel en échantillons (`64` pour ~86.1 fps, `230` pour ~24.0 fps). |
| `NUM_BANDS` | `18` | 1 | `u8` | Nombre de sous-bandes spectrales (`33`). |
| `SPECTRAL_LAG` | `19` | 1 | `u8` | Décalage pour la dérivée spectrale inter-bandes (`2`). |
| `RESERVED` | `20` | 12 | `[u8; 12]` | Zéros de réserve pour extensions futures. |
| `FRAME_COUNT` | `32` | 8 | `u64 LE` | Nombre total d'empreintes `u64` dans le payload. |
| `BAND_BOUNDS` | `40` | `4 * NUM_BANDS` | `[u32; 33]` | Limites fréquentielles supérieures en Hz (132 octets). |
| `ALIGNMENT_PADDING` | `172` | 4 | `[u8; 4]` | 4 octets de zéros garantissant `176 % 8 == 0`. |

> [!IMPORTANT]
> **Correction de l'alignement ARM64 (RPi4)** : L'ébauche initiale produisait un en-tête de 172 octets (`172 % 8 = 4`). Le padding de 4 octets à l'offset 172 garantit un offset 176 rigoureusement aligné sur 8 octets, indispensable pour une projection `mmap` sans copie ni comportement indéfini (`misaligned pointer dereference`).

---

## 3. Sémantique et Encodage du Hash 64-bit

### 3.1 Décomposition Binaire

Chaque trame audio d'indice \( t \) produit un entier `u64` découpé en deux familles de dérivées relatives :

```text
Bit 63                                  Bit 33 Bit 32                      Bit 0
+---------------------------------------------+--------------------------------+
|       DÉRIVÉE SPECTRALE (31 bits)          |  DÉRIVÉE TEMPORELLE (33 bits)  |
|       sign(E[b, t] - E[b + 2, t])           |  sign(E[b, t] - E[b, t - 1])   |
+---------------------------------------------+--------------------------------+
```

### 3.2 Calcul des Composantes

#### Dérivée Temporelle (Poids Faibles, Bits 0 à 32 — 33 bits)

Pour chaque bande spectrale $b \in [0, 32]$ :

- $\text{bit}_t(b, t) = 1$ si $E[b, t] - E[b, t-1] > 0$
- $\text{bit}_t(b, t) = 0$ sinon

Pour la première trame ($t = 0$), tous les bits temporels sont forcés à 0.

#### Dérivée Spectrale (Poids Forts, Bits 33 à 63 — 31 bits)

Pour chaque bande spectrale $b \in [0, 30]$ avec décalage $\text{LAG} = 2$ :

- $\text{bit}_s(b, t) = 1$ si $E[b, t] - E[b + 2, t] > 0$
- $\text{bit}_s(b, t) = 0$ sinon

#### Invariant 64-bit

$\text{NUM\_BANDS} + (\text{NUM\_BANDS} - \text{SPECTRAL\_LAG}) = 33 + (33 - 2) = 64$ bits exacts.

---

## 4. Invariants de Validation au Chargement

Tout lecteur conforme du format `.ahash` doit rejeter immédiatement tout fichier violant l'un des invariants suivants :

1. `MAGIC == b"AHASH\0\1\0"`
2. `VERSION == 1`
3. `0 < SAMPLE_RATE <= 48000`
4. `FRAME_SIZE > 0` et puissance de deux (`FRAME_SIZE.is_power_of_two()`)
5. `0 < HOP_SIZE <= FRAME_SIZE`
6. `8 <= NUM_BANDS <= 64`
7. `1 <= SPECTRAL_LAG < NUM_BANDS`
8. `2 * NUM_BANDS - SPECTRAL_LAG == 64`
9. `BAND_BOUNDS` strictement croissant et `BAND_BOUNDS[NUM_BANDS - 1] == SAMPLE_RATE / 2`
10. `FRAME_COUNT > 0`
11. Taille totale du fichier exactement égale à `176 + 8 * FRAME_COUNT` octets.

---

## 5. Convention de Nommage & Intégration Système

Les fichiers `.ahash` adoptent la même convention d'isolation de cadence que `.phash` :

```text
<media_stem>.<cadence_fps>fps.ahash
```

Exemples canoniques :

- `Day.Shift.2022.1080p.86.1fps.ahash` (mode haute résolution, `HOP_SIZE = 64`)
- `Day.Shift.2022.1080p.24.0fps.ahash` (mode synchronisé vidéo, `HOP_SIZE = 230`)
