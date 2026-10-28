# Rapport d'Exécution & Profiling Batch TVShow (Synology NAS)

**Date d'exécution** : 2026-10-28 00:10:28
**Cible** : `//synology.local/tvshow` (Stream CIFS direct, 0 Mo copié en local)
**Statut Global** : `SUCCÈS PARTIEL`

## 1. Synthèse Exécutive & Métriques Clés

| Métrique | Valeur Mesurée | Note Opérationnelle |
| :--- | :--- | :--- |
| **Vidéos Traitées** | **8** | `4 OK` / `4 FAIL` |
| **Couplings Certifiés** | **4** | Multi-langues CAS dans `data/SRT/couplings.json` |
| **Volume Vidéo Lu (CIFS)** | **8.04 GiB** | Décodé en direct sans copie locale |
| **Temps d'Exécution Total** | **5.5 min** (329.9s) | Débit moyen `24.9 MiB/s` |
| **Charge CPU Moyenne / Pic** | **3.4% / 101.2%** | Profiling multi-cœurs psutil |
| **Empreinte RAM Pic (RSS)** | **507.9 MiB** | Confinement strict mémoire sans fuite |
| **Bande Passante CIFS Moy.** | **248.9 MiB/s** | Lecture flux réseau direct Synology |
| **Horaire Butoir Configuré** | `07:00` | Arrêt automatisé à l'heure programmée |
| **Incident Réseau / Coupure** | `Aucun (Connexion NAS nominale toute la session)` | Statut liaison CIFS |

## 2. Matrice de Validation Multilingue & Alphabets

Vérification de la conformité du moteur de décodage UTF-8/legacy pour tous les alphabets détectés :

| Langue (ISO) | Pistes Extraites | Total Cues | Alphabets / Écritures Détectés | Codecs Conteneurs |
| :--- | :--- | :--- | :--- | :--- |
| **`en`** | 4 | 876 | Latin | `ass, subrip` |

## 3. Registre Exhaustif des Erreurs & Anomalies

Inventaire technique complet de tout incident pour guidage des futurs refactorings / correctifs :

| Vidéo / Release | Composant / Catégorie | Message & Cause Racine |
| :--- | :--- | :--- |
| `Les Origines _ L'odyssée de l'écrit` | **Media Ingestion** | `Error: ❌ No matching .srt file found for '"/media/storage/tvshow/ARTE - L'odyssée de l'écriture/Les Origines _ L'odyssée de l'écriture (1_3) _ ARTE [jDFacmXZFGY].webm"'. Specify a subtitle file with '--srt <path>' or download one using 'dbsrt fetch-subtitles -q "<title>" -l fr'.` |
| `Demon Slayer – Saison 2 Episode 10 ` | **Media Ingestion** | `Error: ❌ No matching .srt file found for '"/media/storage/tvshow/DEMON SLAYER/S02/Demon Slayer – Saison 2 Episode 10 VOSTFR.mp4"'. Specify a subtitle file with '--srt <path>' or download one using 'dbsrt fetch-subtitles -q "<title>" -l fr'.` |
| `Creating the Scene - Brother Justin` | **Media Ingestion** | `Error: ❌ No matching .srt file found for '"/media/storage/tvshow/Carnivàle (2003)[1080p][x265.HEVC.10bit.AAC.5.1]/Featurettes/Creating the Scene - Brother Justin.mkv"'. Specify a subtitle file with '--srt <path>' or download one using 'dbsrt fetch-subtitles -q "<title>" -l fr'.` |
| `L'illusion du temps _ La magie du c` | **Media Ingestion** | `Error: ❌ No matching .srt file found for '"/media/storage/tvshow/ARTE/La magie du cosmos/L'illusion du temps _ La magie du cosmos (1_4) _ ARTE [HQ8YrXFo_wM].webm"'. Specify a subtitle file with '--srt <path>' or download one using 'dbsrt fetch-subtitles -q "<title>" -l fr'.` |
| `Bojack.Horseman.S01E10.MULTi.[1080p` | **Subtrack #0 (und)** | `Extraction omitted or failed` |

## 4. Journal Détaillé par Média Traité

| Vidéo | Taille | Durée | Couplings | CPU (Moy/Pic) | RAM Pic | Débit CIFS | Statut |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `Les Origines _ L'odyssée de l'écrit` | 443.9 MB | 51.3m | +0 | 0% / 0% | 5 MB | 851.9 MB/s | ❌ FAIL |
| `Demon Slayer – Saison 2 Episode 10 ` | 133.3 MB | 23.7m | +0 | 0% / 0% | 6 MB | 215.7 MB/s | ❌ FAIL |
| `Creating the Scene - Brother Justin` | 10.4 MB | 3.4m | +0 | 0% / 0% | 5 MB | 15.7 MB/s | ❌ FAIL |
| `L'illusion du temps _ La magie du c` | 547.8 MB | 52.5m | +0 | 0% / 0% | 5 MB | 823.5 MB/s | ❌ FAIL |
| `Bojack.Horseman.S01E10.MULTi.[1080p` | 1132.1 MB | 25.3m | +1 | 10% / 101% | 508 MB | 17.0 MB/s | ✅ OK |
| `cowboy.bebop.s01e07.repack.1080p.bl` | 2232.1 MB | 24.7m | +1 | 8% / 101% | 508 MB | 29.2 MB/s | ✅ OK |
| `[CBM]_Cowboy_Bebop_-_Session_09_-_J` | 753.8 MB | 24.6m | +1 | 6% / 101% | 508 MB | 9.4 MB/s | ✅ OK |
| `Halt.and.Catch.Fire.S03E06.And.She.` | 2976.0 MB | 44.0m | +1 | 3% / 100% | 195 MB | 28.4 MB/s | ✅ OK |
