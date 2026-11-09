---
title: "Étude R&D : Empreinte Spectrale Audio .ahash (Haitsma-Kalker) & Multimodalité"
tags:
  - explanation
  - audio
  - ahash
  - haitsma-kalker
  - matching
  - r&d
---

# 💡 Étude R&D : Empreinte Spectrale Audio `.ahash` (Haitsma-Kalker) & Multimodalité

- **Statut** : Analyse d'Architecture & Étude Comparative
- **Date** : 2026-11-10
- **Branche** : `feat/ahash-audio-fingerprint`

---

## 1. Contexte & Problématique Métier

Le moteur d'identification et de recalage de sous-titres de `pydbsrt` repose historiquement sur le hachage perceptuel visuel (`pHash` 2D DCT-II sur images de luminance $32 \times 32$).

Si cette approche offre une universalité absolue indépendamment de la langue parlée (un film a la même image qu'il soit visionné en VO, VF ou VOST), elle présente des fragilités structurelles sur certains profils de contenus :

1. **Scènes visuellement plates ou sous-exposées** : écrans noirs prolongés, transitions lentes au noir, génériques minimalistes où les trames successives présentent une distance de Hamming inter-trames quasi-nulle ($\Delta \approx 0$).
2. **Plans fixes avec narration** : documentaires, cours magistraux, tutoriels vidéo où l'image reste inchangée pendant des dizaines de secondes alors que le flux de parole est riche et continu.
3. **Coût computationnel du décodage vidéo** : décoder des flux vidéo haute définition (H.264/HEVC 1080p ou 4K) sans accélération matérielle VA-API sur des cibles contraintes comme le Raspberry Pi 4 sature les cœurs CPU.

L'objectif de cette étude est d'évaluer l'intégration de l'empreinte spectrale audio différentielle de **Haitsma-Kalker (2002)**, packée dans un format 64-bit `.ahash`, en tant que modalité secondaire.

---

## 2. Principes Algorithmiques : Le Hashing Différentiel de Haitsma-Kalker

Contrairement aux approches de reconnaissance musicale basées sur des constellations de pics (ex: Shazam), l'algorithme de Haitsma-Kalker produit un flux continu de signatures 64-bit à cadence fixe, mathématiquement isomorphe à une séquence de trames vidéo.

```mermaid
flowchart TD
    A["Flux Audio (MKV / MP4 / WebM)"] --> B["Décodage Mono 5512 Hz (f32le)"]
    B --> C["Fenêtrage Hann (N=2048, ~371 ms)"]
    C --> D["FFT 2048 -> Densité Spectrale de Puissance |X(k)|^2"]
    D --> E["Intégration en 33 Bandes Logarithmiques (300 Hz - 2756 Hz)"]
    E --> F["Calcul Dérivée Temporelle : sign(E[b, t] - E[b, t-1]) (33 bits)"]
    E --> G["Calcul Dérivée Spectrale : sign(E[b, t] - E[b+2, t]) (31 bits)"]
    F --> H["Packing MSB-first dans un entier u64"]
    G --> H
    H --> I["Flux .ahash (Distance de Hamming / BK-Tree / MIH)"]
```

### 2.1 Dérivée Relative & Invariance au Volume

L'algorithme n'évalue jamais l'énergie absolue d'une fréquence, mais uniquement le **signe des variations différentielles** :

- **Variation Temporelle** : le bit est à 1 si l'énergie de la bande $b$ croît par rapport à la trame précédente, 0 sinon. Cette mesure est insensible à la dynamique globale du mixage ou au volume d'écoute.
- **Variation Spectrale** : le bit est à 1 si l'énergie de la bande $b$ dépasse celle de la bande $b+2$, capturant la pente du timbre indépendamment de la normalisation sonore.

---

## 3. Analyse Comparative : pHash Visuel vs aHash Audio

| Critère | pHash Visuel (2D DCT-II) | aHash Audio (Haitsma-Kalker) | Impact Architectural |
|---|---|---|---|
| **Dimension unitaire** | `u64` (64 bits) | `u64` (64 bits) | 100% interopérable avec BK-Tree, MIH, `popcnt`. |
| **Coût CPU d'extraction** | 100 ms à 500 ms par seconde vidéo | 2 ms à 10 ms par seconde audio | **Gain de vitesse de 10x à 50x**. |
| **Accélération matérielle requise** | Recommandée (VA-API / GPU) | Nulle (CPU pur mono thread) | Idéal pour appliances RPi4 ou VPS o2switch. |
| **Résistance au ré-encodage vidéo** | Sensible aux crops, aspect ratio, logos | **100% insensible** aux modifications visuelles | Excellente stabilité entre éditions BluRay/WEBRip. |
| **Sensibilité au doublage (VO/VF)** | **Insensible** (même vidéo pour toutes langues) | **Échec complet** (spectres de parole divergents) | Contrainte majeure : nécessite la même langue. |
| **Résolution temporelle native** | 24.0 fps ($\Delta t \approx 41.6$ ms) | ~86.1 fps ($\Delta t \approx 11.6$ ms) | Précision temporelle fine au détriment de la taille. |
| **Taille pour 2h de film** | ~1.38 MB | ~4.84 MB (à 86 fps) / ~1.35 MB (à 24 fps) | Nécessite un compromis de cadence (`HOP_SIZE`). |

---

## 4. Les Limites Critiques & Pièges d'Intégration

### 4.1 L'Écueil du Multi-Langue (Doublage International)

Dans l'écosystème de `pydbsrt`, les utilisateurs synchronisent régulièrement des pistes sous-titrées sur des releases étrangères (ex: sous-titre français sur release japonaise ou américaine).

- Si l'audio de la référence certifiée est en version originale (VO anglaise) et que la cible locale est une version doublée (VF française), l'empreinte spectrale Haitsma-Kalker ne présentera **aucune corrélation** sur la bande de parole (300–2000 Hz).
- Par conséquent, l'aHash **ne peut pas se substituer** au pHash vidéo comme identifiant primaire universel. Il doit opérer comme modalité **hybride conditionnelle** ou comme vérificateur d'alignement lorsque les pistes audio partagent la même langue.

### 4.2 L'Effet du PAL Speedup (24 fps vers 25 fps)

Le passage d'un master cinéma (24 fps) à une diffusion européenne (25 fps) accélère le média de 4.1%.

- **En vidéo** : l'axe temporel est étiré, mais chaque trame visuelle individuelle reste identique.
- **En audio** : si aucun algorithme de pitch-shifting n'a été appliqué au ré-encodage, toutes les composantes fréquentielles sont translatées de +4.1% ($f' = 1.041 \times f$).
- Avec 33 bandes d'environ 52 Hz de largeur, un décalage de +40 Hz à 1000 Hz fait baver l'énergie sur la sous-bande voisine, augmentant artificiellement la distance de Hamming.

### 4.3 Dimensionnement & Alignement Mémoire ARM64

L'en-tête du format `.ahash` est rigoureusement fixé à **176 octets** (avec padding de 4 octets). Cette disposition assure un alignement strict à 8 octets, permettant un accès sans copie (`zero-copy mmap`) sur processeur ARM Cortex-A72 (Raspberry Pi 4) sans risque de faute matérielle ou de baisse de débit.

---

## 5. Stratégie de Fusion Hybride Recommandée

```text
Query Cible (Vidéo + Audio)
      │
      ├──> Extraction Audio Rapide (~50 ms) ────────┐
      │                                             ▼
      │                                   Scoring aHash (Fast Probe)
      │                                   - Si score >= 0.85 et langue confirmée -> Match validé (Delta t exact)
      │                                   - Si score indécis ou divergence
      ▼                                             │
Extraction Vidéo pHash (Full / Probes) <────────────┘
      │
      ▼
Scoring pHash Visuel (Garantie universelle)
      │
      ▼
Consensus Spatiotemporel Final
```

Cette architecture à deux étages permet de diviser par 10 le temps de qualification sur les releases partageant la même piste audio, tout en conservant le filet de sécurité absolu du pHash vidéo pour les versions doublées.
