# Rapport d'Exécution & Profiling Batch TVShow (Synology NAS)

**Date d'exécution** : 2026-10-26 18:03:35
**Cible** : `//192.168.1.58/tvshow` (Stream CIFS direct, 0 Mo copié en local)
**Statut Global** : `100% NOMINAL`

## 1. Synthèse Exécutive & Métriques Clés

| Métrique | Valeur Mesurée | Note Opérationnelle |
| :--- | :--- | :--- |
| **Vidéos Traitées** | **10** | `10 OK` / `0 FAIL` |
| **Couplings Certifiés** | **39** | Multi-langues CAS dans `data/SRT/couplings.json` |
| **Volume Vidéo Lu (CIFS)** | **16.78 GiB** | Décodé en direct sans copie locale |
| **Temps d'Exécution Total** | **53.7 min** (3223.4s) | Débit moyen `5.3 MiB/s` |
| **Charge CPU Moyenne / Pic** | **3.1% / 102.1%** | Profiling multi-cœurs psutil |
| **Empreinte RAM Pic (RSS)** | **201.3 MiB** | Confinement strict mémoire sans fuite |
| **Bande Passante CIFS Moy.** | **7.1 MiB/s** | Lecture flux réseau direct Synology |
| **Horaire Butoir Configuré** | `07:00` | Arrêt automatisé à l'heure programmée |
| **Incident Réseau / Coupure** | `Aucun (Connexion NAS nominale toute la session)` | Statut liaison CIFS |

## 2. Matrice de Validation Multilingue & Alphabets

Vérification de la conformité du moteur de décodage UTF-8/legacy pour tous les alphabets détectés :

| Langue (ISO) | Pistes Extraites | Total Cues | Alphabets / Écritures Détectés | Codecs Conteneurs |
| :--- | :--- | :--- | :--- | :--- |
| **`ar`** | 1 | 600 | Arabic, Latin | `subrip` |
| **`cs`** | 1 | 589 | Latin | `subrip` |
| **`da`** | 1 | 488 | Latin | `subrip` |
| **`de`** | 1 | 541 | Latin | `subrip` |
| **`el`** | 1 | 593 | Greek, Latin | `subrip` |
| **`en`** | 12 | 3212 | Latin | `ass, subrip` |
| **`es`** | 2 | 1166 | Latin | `subrip` |
| **`fi`** | 1 | 515 | Latin | `subrip` |
| **`fr`** | 1 | 597 | Latin | `subrip` |
| **`he`** | 1 | 603 | Hebrew, Latin | `subrip` |
| **`hu`** | 1 | 592 | Latin | `subrip` |
| **`id`** | 1 | 601 | Latin | `subrip` |
| **`it`** | 1 | 554 | Latin | `subrip` |
| **`ja`** | 1 | 581 | CJK (Ideographs), Japanese (Kana), Latin | `subrip` |
| **`ko`** | 1 | 612 | Korean (Hangul), Latin | `subrip` |
| **`nl`** | 1 | 496 | Latin | `subrip` |
| **`no`** | 1 | 498 | Latin | `subrip` |
| **`pl`** | 1 | 590 | Latin | `subrip` |
| **`pt`** | 2 | 1186 | Latin | `subrip` |
| **`ro`** | 1 | 584 | Latin | `subrip` |
| **`ru`** | 1 | 589 | Cyrillic, Latin | `subrip` |
| **`sv`** | 1 | 538 | Latin | `subrip` |
| **`th`** | 1 | 611 | Latin, Thai | `subrip` |
| **`tr`** | 1 | 539 | Latin | `subrip` |
| **`vi`** | 1 | 597 | Latin | `subrip` |
| **`zh`** | 2 | 1200 | CJK (Ideographs), Latin | `subrip` |

## 3. Registre Exhaustif des Erreurs & Anomalies

Inventaire technique complet de tout incident pour guidage des futurs refactorings / correctifs :

| Vidéo / Release | Composant / Catégorie | Message & Cause Racine |
| :--- | :--- | :--- |
| `Bojack.Horseman.S01E08.MULTi.[1080p` | **Subtrack #0 (und)** | `Extraction omitted or failed` |

## 4. Journal Détaillé par Média Traité

| Vidéo | Taille | Durée | Couplings | CPU (Moy/Pic) | RAM Pic | Débit CIFS | Statut |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `S03E05 The Aluminum Monster vs. Fat` | 276.7 MB | 22.4m | +1 | 6% / 101% | 177 MB | 3.0 MB/s | ✅ OK |
| `[CBM]_Cowboy_Bebop_-_Session_07_-_H` | 614.0 MB | 24.7m | +1 | 3% / 98% | 157 MB | 4.0 MB/s | ✅ OK |
| `Bojack.Horseman.S01E08.MULTi.[1080p` | 1141.6 MB | 25.3m | +1 | 4% / 101% | 193 MB | 7.3 MB/s | ✅ OK |
| `cowboy.bebop.s01e06.repack.1080p.bl` | 2231.6 MB | 24.8m | +1 | 3% / 101% | 192 MB | 13.1 MB/s | ✅ OK |
| `Halt.and.Catch.Fire.S03E05.Yerba.Bu` | 3084.1 MB | 45.5m | +1 | 4% / 100% | 182 MB | 15.4 MB/s | ✅ OK |
| `Devs.S01E08.1080p.AMZN.WEB-DL.DDP5.` | 3505.6 MB | 52.6m | +2 | 4% / 101% | 182 MB | 14.1 MB/s | ✅ OK |
| `Locke.and.Key.S01E02.Trapper..Keepe` | 1808.5 MB | 50.2m | +26 | 3% / 100% | 199 MB | 6.6 MB/s | ✅ OK |
| `Fargo (2014) - S03E10 - Somebody to` | 1910.5 MB | 52.3m | +1 | 1% / 100% | 201 MB | 3.2 MB/s | ✅ OK |
| `Carnivàle (2003) - S01E08 - Lonniga` | 1752.5 MB | 58.0m | +2 | 1% / 102% | 201 MB | 2.6 MB/s | ✅ OK |
| `Euphoria.US.S02E05.1080p.WEBRip.x26` | 861.7 MB | 54.0m | +3 | 1% / 86% | 199 MB | 1.3 MB/s | ✅ OK |
