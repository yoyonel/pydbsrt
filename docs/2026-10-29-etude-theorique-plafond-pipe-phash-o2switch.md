---
title: "Étude Théorique : Plafond Asymptotique du Pipe pHash sur o2switch"
tags:
  - explanation
  - performance
  - o2switch
  - ffmpeg
  - phash
---

# Étude Théorique : Plafond Asymptotique du Pipe pHash sur o2switch

> **Date** : 29 Octobre 2026 à 22:17 CET
> **Nature** : analyse théorique (contraintes matérielles + coût algorithmique). Aucune mesure de débit/fps de la plateforme n'est utilisée, pour ne pas biaiser le modèle.
> **Question** : quelle est la valeur maximale atteignable par l'en-tête `Pipe pHash: xx MB/s (xxx fps)` de `dbsrt top` sur o2switch, avec un code « parfait » ?
> **Complément empirique** : [Analyse Performance Décodage CPU vs Réseau (o2switch)](2026-10-28-analyse-performance-decodage-cpu-vs-reseau-o2switch.md).

---

## 1. Ce que mesure la métrique

- **MB/s** = somme, sur les jobs actifs, du débit de lecture de l'entrée **compressée** (`total_rate_mib_s`, `crates/dbsrt-cli/src/commands/top/sampler.rs`). L'unité réelle est le **MiB/s**.
- **fps** = somme des `hashing_fps` des jobs actifs : frames décodées puis hachées par seconde.
- Lien entre les deux, pour un contenu de `B` bits par frame : `MiB/s = fps × B / (8 × 2^20)`.

Pipeline par job (`crates/dbsrt-core/src/video.rs`) :
`ffmpeg -i ftp://… -vf scale=32:32 -pix_fmt gray -f rawvideo -` → pipe (1 KiB/frame) → DCT pHash 32×32.

## 2. Contraintes physiques (vérifiées sur l'hôte)

| Ressource | Valeur | Rôle |
|---|---|---|
| CPU | Intel Xeon Gold 6263CY (Cascade Lake), AVX2 + AVX-512, max rapporté 3,4 GHz | moteur de décodage |
| Quota CPU LVE | 12 vCPU (1200 %) | **plafond calcul** |
| GPU / VA-API | aucun | décodage 100 % logiciel |
| RAM LVE | 48 GiB | non liant (§5) |
| nproc LVE | 400 | non liant (§5) |
| I/O LVE | 42 MiB/s, 2048 IOPS | I/O bloc uniquement (§4) |

**Budget de cycles** `C = 12 vCPU × f × e_HT`, avec `f` = 3,3 GHz retenu en all-core turbo :

- `C_max = 12 × 3,3·10⁹ ≈ 3,96·10¹⁰ cycles/s` : vCPU sans contention sur le jumeau hyperthread.
- `C_min = C_max × 0,6 ≈ 2,38·10¹⁰ cycles/s` : jumeaux HT saturés par les autres clients de l'hôte mutualisé.

## 3. Modèle de coût par frame

| Étape | Coût | Remarque |
|---|---|---|
| Entropie (CABAC) | `β · B`, β ≈ 9 cycles/bit | ≈ 1,3 bin/bit × 6–7 cycles/bin, série par frame, **incompressible** |
| Reconstruction + `scale` | `α · P`, α ≈ 6 (H.264 8 bits) / 10 (HEVC 10 bits) cycles/px | prédiction, IDCT, deblock/SAO, réduction 32×32 luma |
| DCT pHash 32×32 | ≈ 3·10⁴ cycles | < 0,2 % de `C`, négligeable |
| Pipe 1 KiB/frame | ≈ 3 MiB/s au pire | négligeable |

```
fps_max = C / (α·P + β·B)
MiB/s   = fps_max × B / (8 × 2^20)
```

α et β sont des ordres de grandeur issus de la littérature sur les décodeurs FFmpeg (précision ≈ ±50 %), pas des mesures sur cet hôte.

**Asymptote CABAC.** Quand le débit binaire `B → ∞`, `MiB/s → C / (8β)`, soit **≈ 525 MiB/s** (`C_max`) et ≈ 315 MiB/s (`C_min`). Aucun codec CABAC ne peut dépasser ce débit d'entrée sur 12 vCPU.

**Code « parfait ».** Il garde une sortie bit-identique, donc les mêmes hashes. Il suppose :

- `-threads 1` par process ffmpeg (aucune synchronisation de frame-threading) ;
- au moins 12 jobs indépendants ;
- 1200 % de CPU utilisé en continu ;
- zéro attente I/O.

C'est exactement l'hypothèse du modèle ci-dessus.

## 4. Plafond réseau

- **I/O LVE (42 MiB/s)** : cette limite cible le cgroup blkio, donc les lectures disque. Le flux FTP est une lecture socket, entièrement en mémoire jusqu'au pipe. Elle ne s'applique donc pas au pipeline. *Non vérifié sur cet hôte.*
- **Lien NAS** : NIC 1 GbE (hypothèse, non vérifiée). Charge utile TCP ≈ 1 Gb/s × 0,949 ≈ **113 MiB/s**.
- **TCP mono-connexion** : avec l'autotuning (≈ 6 Mo de fenêtre) et 20 ms de RTT, le plafond est ≈ 300 MB/s. Non liant.

## 5. Contraintes secondaires (non liantes)

- **RAM** : DPB 4K HEVC 10 bits ≤ 16 × 8,3 Mpx × 1,5 × 2 o ≈ 0,4 GB par job. 12 jobs ≈ 6 GB ≪ 48 GiB.
- **nproc** : ≈ 15 tâches par job (ffmpeg en `-threads 1` + dbsrt). 12 jobs ≈ 180 < 400.
- **Bande passante mémoire** : ≈ 35–40 GB/s aux plafonds du §6, sur ≈ 140 GB/s théoriques par socket (DDR4-2933, 6 canaux). Elle est partagée avec les voisins : c'est le premier goulot secondaire possible.

## 6. Plafonds par profil de contenu

Hypothèse : 23,976 fps source ; `P` = pixels par frame, `B` = bits par frame.

| Profil | P | B | cycles/frame | fps max (`C_max` / `C_min`) | MiB/s (`C_max` / `C_min`) |
|---|---|---|---|---|---|
| 4K HEVC 10 bits remux, 60 Mb/s | 8,29 M | 2,50 Mbit | 105,5 M | **375** / 226 | **112** / 67 |
| 1080p HEVC 10 bits rip, 8 Mb/s | 2,07 M | 0,33 Mbit | 23,7 M | 1 668 / 1 003 | 66 / 40 |
| 1080p H.264 remux BD, 25 Mb/s | 2,07 M | 1,04 Mbit | 21,8 M | 1 814 / 1 090, ramenés à **≈ 900** par le réseau | CPU 225 / 135, ramenés à **113** par le réseau |
| 1080p H.264 web, 5 Mb/s | 2,07 M | 0,21 Mbit | 14,3 M | 2 765 / 1 662 | 69 / 41 |

Lecture :

- **Le fps dépend surtout des pixels.** Un même budget CPU donne environ 7× plus de fps en 1080p qu'en 4K.
- **Le MiB/s dépend surtout du débit binaire.** Plus le contenu est compressé, plus le MiB/s plafonne bas.
- **Bas débit → borne CPU.** Les contenus faiblement compressés restent limités par le CPU.
- **Haut débit → borne réseau.** Les contenus à fort débit atteignent la limite du réseau.

## 7. Hors périmètre du plafond (sortie différente)

Ces leviers dépassent le plafond, mais **changent les hashes ou la sémantique**. Ils ne font donc pas partie du « même calcul » :

- **`-skip_loop_filter all`** : environ −25 % de cycles en HEVC, mais les hashes dérivent par rapport à la base existante.
- **`-skip_frame nokey`** : ne hache que les keyframes. Le CPU devient quasi nul, et le pipeline est alors 100 % borné par le réseau (113 MiB/s).
- **Lecture des seules plages keyframes** (index MKV Cues / MP4 `stss` + FTP `REST`) : la métrique MiB/s perd son sens.

---

## Conclusion

Hypothèses du plafond :

- pipeline actuel, toutes les frames hachées, sortie bit-identique ;
- code parfait, 12 vCPU saturés, sans contention hyperthread.

**Sur o2switch, la vitesse théorique maximale (asymptotique) est ≈ 375 fps → ≈ 112 MiB/s.**

Ce point correspond au profil le plus lourd du catalogue, le remux 4K HEVC 10 bits. Le CPU (12 vCPU) et le réseau (NAS 1 GbE, 113 MiB/s) y saturent **en même temps**.

**Pourquoi 112 MiB/s est aussi le plafond absolu en MiB/s, quel que soit le contenu :** le réseau borne tout à 113 MiB/s, bien en dessous de l'asymptote CPU de 525 MiB/s. Aucun profil ne peut donc dépasser ≈ 112–113 MiB/s.

**Ce que le fps devient selon le contenu :** il monte quand la résolution baisse. Il atteint ≈ 2 765 fps en 1080p, mais le débit d'entrée retombe alors à ≈ 66–69 MiB/s.
