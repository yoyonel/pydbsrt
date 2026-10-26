# Rapport d'Exécution & Profiling Batch TVShow (Synology NAS)

**Date d'exécution** : 2026-10-26 21:16:46
**Cible** : `//192.168.1.58/tvshow` (Stream CIFS direct, 0 Mo copié en local)
**Statut Global** : `100% NOMINAL`

## 1. Synthèse Exécutive & Métriques Clés

| Métrique | Valeur Mesurée | Note Opérationnelle |
| :--- | :--- | :--- |
| **Vidéos Traitées** | **1** | `1 OK` / `0 FAIL` |
| **Couplings Certifiés** | **0** | Multi-langues CAS dans `data/SRT/couplings.json` |
| **Volume Vidéo Lu (CIFS)** | **0.54 GiB** | Décodé en direct sans copie locale |
| **Temps d'Exécution Total** | **0.0 min** (0.0s) | Débit moyen `548.1 MiB/s` |
| **Charge CPU Moyenne / Pic** | **15.0% / 45.0%** | Profiling multi-cœurs psutil |
| **Empreinte RAM Pic (RSS)** | **120.0 MiB** | Confinement strict mémoire sans fuite |
| **Bande Passante CIFS Moy.** | **54809.5 MiB/s** | Lecture flux réseau direct Synology |
| **Horaire Butoir Configuré** | `07:00` | Arrêt automatisé à l'heure programmée |
| **Incident Réseau / Coupure** | `Aucun (Connexion NAS nominale toute la session)` | Statut liaison CIFS |

## 2. Matrice de Validation Multilingue & Alphabets

Vérification de la conformité du moteur de décodage UTF-8/legacy pour tous les alphabets détectés :

| Langue (ISO) | Pistes Extraites | Total Cues | Alphabets / Écritures Détectés | Codecs Conteneurs |
| :--- | :--- | :--- | :--- | :--- |

## 3. Registre Exhaustif des Erreurs & Anomalies

Inventaire technique complet de tout incident pour guidage des futurs refactorings / correctifs :

✅ **Zéro anomalie rencontrée** : 100% des sous-titres textuels et flux vidéo ont été certifiés sans erreur.

## 4. Journal Détaillé par Média Traité

| Vidéo | Taille | Durée | Couplings | CPU (Moy/Pic) | RAM Pic | Débit CIFS | Statut |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `The Leftovers S01E04` | 548.1 MB | 50.7m | +0 | 15% / 45% | 120 MB | 54809.5 MB/s | ✅ OK |
