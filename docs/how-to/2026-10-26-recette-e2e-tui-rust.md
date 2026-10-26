# Recette de Test E2E : Moniteur TUI Rust `dbsrt top` & Ingestion Synology NAS

Ce guide de recette valide de bout en bout l'intégration et l'exécution du moniteur de processus natif en Rust (`dbsrt top`) face à un flux réel d'ingestion et de décodage vidéo pHash depuis le NAS Synology (`/media/latty/Synology/tvshow`).

## Contexte d'Exécution & Environnement

- **Système d'Exploitation** : Linux x86_64
- **Binaire Testé** : `target/release/dbsrt top` (Rust 1.84+, Ratatui 0.29, Sysinfo 0.33)
- **Source des Médias Réels** : Synology NAS CIFS (`/media/latty/Synology/tvshow`)
- **Accélération Matérielle** : Intel Iris Xe Graphics (VA-API `/dev/dri/renderD128`)
- **Télémétrie GPU** : `intel_gpu_top` JSON streaming daemon
- **Enregistrement ASCIInema** : `data/reports/dbsrt_top_e2e_recette.cast` (259.4 KiB)

## Scénarios de Test Exécutés & Résultats

| N° | Scénario & Fonctionnalité | Action Opératoire | Résultat Observé | Statut |
|:---|:---|:---|:---|:---:|
| 1 | **Bannière d'En-Tête Système** | Affichage multi-lignes CPU, RAM, GPU Video Engine %, Conso W, Fréq MHz, Débits CIFS (Instant + Session) | Métriques réelles actualisées toutes les 500ms sans saccade | **PASS** |
| 2 | **Onglet 1 : Tâches Actives** | Détection automatique des processus de décodage GPU VA-API et Sub Extraction | Affichage des PIDs, barres de progression, débits MB/s et ETAs réels | **PASS** |
| 3 | **Navigation & Sauts Liste Tâches** | Touches `[j]`, `[k]`, `[PgDn]`, `[PgUp]`, `[Home]`, `[End]` | Déplacement immédiat du curseur `►`, sauts de 10 lignes et bornes début/fin | **PASS** |
| 4 | **Contrôle Processus (SIGSTOP / SIGCONT)** | Touche `[p]` sur tâche sélectionnée | Pause immédiate du worker (`SIGSTOP`) puis reprise (`SIGCONT`), bandeau d'accusé affiché | **PASS** |
| 5 | **Inspection Complète (Full-Frame)** | Touche `[Enter]` sur tâche active | Affichage plein écran de la ligne de commande, fichiers ouverts, threads et RSS. Retour propre via `[Esc]` | **PASS** |
| 6 | **Onglet 2 : Stats Réelles** | Touches `[2]` puis toggle `[h]` | Affichage plein écran des 4 dimensions (115 médias décodés) : Débit MB/s, Durée mins, Taille GiB, Couplings | **PASS** |
| 7 | **Onglet 3 : Médias Complétés** | Touches `[3]` puis toggle `[r]` | Affichage plein écran du registre (230 médias référencés), statuts `✓ OK` / `✓ CACHE` / `✗ FAIL` | **PASS** |
| 8 | **Défilement & Raccourcis Registre** | Touches `[j]`, `[k]`, `[PgDn]`, `[PgUp]`, `[Home]`, `[End]` | Défilement fluide de la table des médias récents avec maintien du viewport | **PASS** |
| 9 | **Onglet Aide & Cheat Sheet** | Touche `[?]` (Ouverture / Fermeture) | Affichage plein écran des raccourcis et de l'architecture de télémesure. Fermeture immédiate | **PASS** |
| 10 | **Cycle Onglets (`[Tab]` / `[Shift+Tab]`)** | `[Tab]` (avant) et `[Shift+Tab]` (arrière) | Transition circulaire bidirectionnelle parfaite entre tous les onglets | **PASS** |
| 11 | **Arrêt Processus Contrôlé (SIGTERM)** | Touche `[s]` sur tâche sélectionnée | Émission propre du signal `SIGTERM` au worker sélectionné, feedback visuel | **PASS** |
| 12 | **Arrêt Propre & Restauration Terminal** | Touche `[q]` | Quitte l'alternate screen sans résidu ANSI, réactivation du curseur | **PASS** |
| 13 | **Sonde Observabilité (`--json`)** | Exécution non-interactive `./target/release/dbsrt top --json` | Export JSON valide contenant la totalité des structures système, actives, session, stats et historique | **PASS** |

## Mesures Réelles Capturées (Sonde Snapshot)

```json
{
  "host_cpu_pct": 11.3,
  "ram_used_gib": 11.60,
  "ram_total_gib": 31.03,
  "session": {
    "stream_count": 10,
    "session_bytes_gib": 18.05,
    "session_rate_mib_s": 2.72
  },
  "stats": {
    "sample_count": 115,
    "rate_median_mb_s": 3.70,
    "duration_median_mins": 6.25,
    "size_median_gib": 1.77,
    "couplings_median": 2.0
  }
}
```

## Démonstration Interactive & Player Asciinema

<div class="asciinema-wrapper">
  <div class="asciinema-cast" data-cast="../../assets/casts/term_top_recette.cast" data-rows="32" data-cols="140" data-autoplay="false"></div>
  <figcaption>Enregistrement terminal interactif : Recette E2E du moniteur Rust TUI dbsrt top sur flux réel NAS Synology (Asciinema).</figcaption>
</div>

## Relecture de la Session ASCIInema en CLI

La session complète enregistrée est archivée au format standard asciicast v2 dans `docs/assets/casts/term_top_recette.cast` et `data/reports/dbsrt_top_e2e_recette.cast`.

Pour rejouer fidèlement l'intégralité de la recette dans le terminal :

```bash
uvx asciinema play docs/assets/casts/term_top_recette.cast
```

Pour extraire le flux textuel brut des écrans :

```bash
uvx asciinema cat docs/assets/casts/term_top_recette.cast
```
