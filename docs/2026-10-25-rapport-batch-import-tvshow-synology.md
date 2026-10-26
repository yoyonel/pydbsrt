# Rapport d'Exécution & Profiling Batch TVShow (Synology NAS)

**Date d'exécution** : 2026-10-26 11:29:33
**Cible** : `//192.168.1.58/tvshow` (Stream CIFS direct, 0 Mo copié en local)
**Statut Global** : `100% NOMINAL`

## 1. Synthèse Exécutive & Métriques Clés

| Métrique | Valeur Mesurée | Note Opérationnelle |
| :--- | :--- | :--- |
| **Vidéos Traitées** | **1** | `1 OK` / `0 FAIL` |
| **Couplings Certifiés** | **2** | Multi-langues CAS dans `data/SRT/couplings.json` |
| **Volume Vidéo Lu (CIFS)** | **1.47 GiB** | Décodé en direct sans copie locale |
| **Temps d'Exécution Total** | **8.0 min** (481.1s) | Débit moyen `3.1 MiB/s` |
| **Charge CPU Moyenne / Pic** | **1.2% / 81.1%** | Profiling multi-cœurs psutil |
| **Empreinte RAM Pic (RSS)** | **171.3 MiB** | Confinement strict mémoire sans fuite |
| **Bande Passante CIFS Moy.** | **3.1 MiB/s** | Lecture flux réseau direct Synology |
| **Horaire Butoir Configuré** | `07:00` | Arrêt automatisé à l'heure programmée |
| **Incident Réseau / Coupure** | `Aucun (Connexion NAS nominale toute la session)` | Statut liaison CIFS |

## 2. Matrice de Validation Multilingue & Alphabets

Vérification de la conformité du moteur de décodage UTF-8/legacy pour tous les alphabets détectés :

| Langue (ISO) | Pistes Extraites | Total Cues | Alphabets / Écritures Détectés | Codecs Conteneurs |
| :--- | :--- | :--- | :--- | :--- |
| **`en`** | 1 | 758 | Latin | `subrip` |

## 3. Registre Exhaustif des Erreurs & Anomalies

Inventaire technique complet de tout incident pour guidage des futurs refactorings / correctifs :

✅ **Zéro anomalie rencontrée** : 100% des sous-titres textuels et flux vidéo ont été certifiés sans erreur.

## 4. Journal Détaillé par Média Traité

| Vidéo | Taille | Durée | Couplings | CPU (Moy/Pic) | RAM Pic | Débit CIFS | Statut |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `Carnivàle (2003) - S01E03 - Tipton ` | 1503.7 MB | 49.7m | +2 | 1% / 81% | 171 MB | 3.1 MB/s | ✅ OK |
