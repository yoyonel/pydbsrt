# Rapport d'Exécution & Profiling Batch TVShow (Synology NAS)

**Date d'exécution** : 2026-10-26 15:46:30
**Cible** : `//192.168.1.58/tvshow` (Stream CIFS direct, 0 Mo copié en local)
**Statut Global** : `100% NOMINAL`

## 1. Synthèse Exécutive & Métriques Clés

| Métrique | Valeur Mesurée | Note Opérationnelle |
| :--- | :--- | :--- |
| **Vidéos Traitées** | **10** | `10 OK` / `0 FAIL` |
| **Couplings Certifiés** | **15** | Multi-langues CAS dans `data/SRT/couplings.json` |
| **Volume Vidéo Lu (CIFS)** | **18.05 GiB** | Décodé en direct sans copie locale |
| **Temps d'Exécution Total** | **53.9 min** (3234.7s) | Débit moyen `5.7 MiB/s` |
| **Charge CPU Moyenne / Pic** | **3.1% / 101.1%** | Profiling multi-cœurs psutil |
| **Empreinte RAM Pic (RSS)** | **200.8 MiB** | Confinement strict mémoire sans fuite |
| **Bande Passante CIFS Moy.** | **8.0 MiB/s** | Lecture flux réseau direct Synology |
| **Horaire Butoir Configuré** | `07:00` | Arrêt automatisé à l'heure programmée |
| **Incident Réseau / Coupure** | `Aucun (Connexion NAS nominale toute la session)` | Statut liaison CIFS |

## 2. Matrice de Validation Multilingue & Alphabets

Vérification de la conformité du moteur de décodage UTF-8/legacy pour tous les alphabets détectés :

| Langue (ISO) | Pistes Extraites | Total Cues | Alphabets / Écritures Détectés | Codecs Conteneurs |
| :--- | :--- | :--- | :--- | :--- |
| **`en`** | 10 | 4122 | Latin | `ass, subrip` |

## 3. Registre Exhaustif des Erreurs & Anomalies

Inventaire technique complet de tout incident pour guidage des futurs refactorings / correctifs :

| Vidéo / Release | Composant / Catégorie | Message & Cause Racine |
| :--- | :--- | :--- |
| `Bojack.Horseman.S01E07.MULTi.[1080p` | **Subtrack #0 (und)** | `Extraction omitted or failed` |

## 4. Journal Détaillé par Média Traité

| Vidéo | Taille | Durée | Couplings | CPU (Moy/Pic) | RAM Pic | Débit CIFS | Statut |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `S03E04 The Gang Gets Held Hostage` | 284.7 MB | 23.0m | +1 | 6% / 100% | 190 MB | 3.7 MB/s | ✅ OK |
| `[CBM]_Cowboy_Bebop_-_Session_06_-_S` | 575.5 MB | 24.7m | +1 | 6% / 100% | 179 MB | 6.3 MB/s | ✅ OK |
| `cowboy.bebop.s01e05.repack.1080p.bl` | 2231.9 MB | 24.0m | +1 | 3% / 98% | 190 MB | 14.0 MB/s | ✅ OK |
| `Halt.and.Catch.Fire.S03E04.Rules.of` | 2917.3 MB | 43.0m | +1 | 4% / 100% | 180 MB | 15.1 MB/s | ✅ OK |
| `Bojack.Horseman.S01E07.MULTi.[1080p` | 1033.2 MB | 25.3m | +1 | 3% / 98% | 191 MB | 5.0 MB/s | ✅ OK |
| `His.Dark.Materials.S02E07.AEsahaett` | 3296.6 MB | 47.5m | +2 | 3% / 101% | 192 MB | 14.1 MB/s | ✅ OK |
| `Devs.S01E07.1080p.AMZN.WEB-DL.DDP5.` | 3639.0 MB | 53.7m | +2 | 3% / 100% | 189 MB | 14.6 MB/s | ✅ OK |
| `Fargo (2014) - S03E09 - Aporia (108` | 1834.6 MB | 50.4m | +1 | 1% / 100% | 192 MB | 3.1 MB/s | ✅ OK |
| `Carnivàle (2003) - S01E06 - Pick a ` | 1684.8 MB | 56.1m | +2 | 1% / 101% | 201 MB | 2.5 MB/s | ✅ OK |
| `Euphoria.US.S02E04.1080p.WEBRip.x26` | 988.7 MB | 62.0m | +3 | 1% / 94% | 167 MB | 1.3 MB/s | ✅ OK |
