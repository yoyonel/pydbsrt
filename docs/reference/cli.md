---
title: "Référence : CLI Standalone Rust (dbsrt)"
tags:

  - reference
  - cli
  - rust
---

# 📖 Référence : CLI Standalone Rust (`dbsrt`)

Cette page documente de manière exhaustive la commande binaire `dbsrt` générée par le compilateur Rust (`target/release/dbsrt`).

---

## 1. Vue d'Ensemble & Syntaxe

```text
dbsrt [OPTIONS] <SOUS_COMMANDE> [ARGUMENTS...]
```

### Options Globales

| Option | Raccourci | Description |
| :--- | :--- | :--- |
| `--help` | `-h` | Affiche l'aide complète ou l'aide de la sous-commande spécifiée. |
| `--version` | `-V` | Affiche le numéro de version de la compilation courante. |

### Codes de Sortie (Exit Codes)

| Code | Signification |
| :---: | :--- |
| `0` | Succès de l'opération. |
| `1` | Erreur d'entrée/sortie, fichier introuvable ou erreur de parsing. |
| `2` | Erreur de validation d'argument ou syntaxe CLI invalide. |
| `3` | Échec de connexion réseau ou base de données. |

<div class="asciinema-wrapper">
  <div class="asciinema-cast" data-cast="../../assets/casts/term_cli_help.cast" data-rows="24"></div>
  <figcaption>Enregistrement terminal interactif : Aide complète et catalogue des sous-commandes de dbsrt (Asciinema).</figcaption>
</div>

---

## 2. Dictionnaire des Sous-Commandes

### `media:add` (alias `media-add`)
Façade unifiée d'ingestion média : extrait les empreintes pHash, certifie le couplage SRT et déclenche le rechargement à chaud sur le serveur.

```text
dbsrt media:add <VIDEO_PATH> [SRT_PATH] [OPTIONS]
```

- `<VIDEO_PATH>` : Chemin vers le fichier vidéo conteneur (`.mp4`, `.mkv`, `.avi`).
- `[SRT_PATH]` : *(Optionnel)* Chemin vers la piste de sous-titres de référence. Si omis, auto-découvert par stem.
- `--lang <code>` : Code de langue ISO-639-1 (`fr`, `en`, etc.). Par défaut : `fr` ou détecté via le suffixe du fichier.
- `--fps <val>` : Forcer le framerate d'échantillonnage. Par défaut : sondé via `ffprobe`.
- `--all-embedded` : Extraire et certifier tous les flux de sous-titres textuels intégrés au conteneur vidéo.
- `--hwaccel <mode>` : Moteur d'accélération matérielle (`auto`, `vaapi`, `cuda`, `qsv`, `disabled`). Par défaut : `auto` (utilise le GPU matériel si présent, avec fallback CPU automatique).
- `--force` : Forcer le recalcul et l'écrasement des empreintes existantes.
- `--no-reload` : Désactiver l'appel de hot-reload vers `dbsrt-server`.

---

### `certify-coupling` (alias `import-subtitles`)
Certifie et enregistre une relation de vérité terrain entre un fichier d'empreinte `.phash` et un sous-titre `.srt`.

```text
dbsrt certify-coupling --phash-file <PATH> --srt-file <PATH> [OPTIONS]
```

- `--phash-file <PATH>` : Fichier d'empreinte binaire de référence.
- `--srt-file <PATH>` : Fichier SRT de référence.
- `--language <LANG>` : Code langue attribué au sous-titre (défaut : `FR`).
- `--media-name <NAME>` : Nom lisible du média.
- `--output-json <PATH>` : Fichier registre JSON cible (défaut : `data/SRT/couplings.json`).

---

### `coupling:relabel` (alias `coupling-relabel`)
Modifie l'attribution linguistique ou le statut de confiance d'un couplage certifié existant.

```text
dbsrt coupling:relabel --media <MEDIA_HASH> [--sub <SUB_HASH>] --lang <LANG_CODE>
```

---

### `export-phash`
Extrait les empreintes pHash 64-bit d'une vidéo vers un fichier binaire `.phash`.

```text
dbsrt export-phash --input <VIDEO_PATH> --output <PHASH_PATH> [OPTIONS]
```

- `--fps <FPS>` : Fréquence d'extraction (défaut : `25.0` ou framerate natif).
- `--method <METHOD>` : Méthode d'échantillonnage : `fixed`, `cuts` ou `hybrid` (défaut : `hybrid`).

---

### `scan-library`
Recherche ultra-rapide en mémoire sans base de données (`Zero-DB`) par projection `mmap` et calcul SIMD Hamming.

```text
dbsrt scan-library --phash-dir <DIR> --query <SAMPLE_PHASH> [OPTIONS]
```

- `--max-distance <DIST>` : Distance de Hamming maximale admissible (défaut : `10`).
- `--limit <LIMIT>` : Nombre maximal de candidats retournés.

---

### `recognize-media`
Identifie un extrait vidéo inconnu en calculant ses empreintes à la volée et en interrogeant la base ou le catalogue local.

```text
dbsrt recognize-media --video <VIDEO_SAMPLE> [OPTIONS]
```

---

### `retarget-subtitles`
Recale une piste de sous-titres depuis un média de référence vers une vidéo cible en calculant le décalage temporel \( \Delta t \) par consensus.

```text
dbsrt retarget-subtitles --target-video <VIDEO> --reference-phash <PHASH> --subtitles <SRT> --output <OUT_SRT>
```

---

### `import-phash`
Insère les empreintes d'un fichier `.phash` dans PostgreSQL 17 avec indexation SP-GiST BK-Tree.

```text
dbsrt import-phash --input <PHASH_FILE> [OPTIONS]
```

---

### `pack`
Compile un ensemble de fichiers unitaires `.phash` dans une archive binaire haute densité `.dbphash` v2.

```text
dbsrt pack --input-dir <DIR> --output <CATALOG.dbphash>
```

---

### `migrate`
Pilote les migrations de schéma de base de données PostgreSQL via le moteur Refinery.

```text
dbsrt migrate [--check]
```

- `--check` : Affiche l'état des migrations sans appliquer de modification.

---

### `fetch-subtitles`
Télécharge des sous-titres depuis l'API OpenSubtitles REST avec mise en cache locale.

```text
dbsrt fetch-subtitles --imdb-id <ID> --lang <LANG> --output <DIR>
```

---

### `top`
Moniteur interactif TUI temps réel en Rust natif (Ratatui, full-frame) affichant les tâches de décodage et extraction pHash en cours, les métriques matérielles GPU VA-API, les débits réseau CIFS/SMB, les statistiques descriptives complètes et le registre des médias complétés.

```text
dbsrt top [OPTIONS]
```

- `--refresh-ms <REFRESH_MS>` : Intervalle de rafraîchissement des télémesures et de l'UI en millisecondes (défaut : `1000`).
- `--json` : Échantillonne et affiche un instantané unique de la télémétrie complète (actifs, statistiques, session, historique) au format JSON (mode sonde automatisée non-interactif).

<div class="asciinema-wrapper">
  <div class="asciinema-cast" data-cast="../../assets/casts/term_top_recette.cast" data-rows="26" data-cols="140" data-autoplay="false"></div>
  <figcaption>Enregistrement terminal interactif : Recette E2E du moniteur Rust TUI dbsrt top sur flux réel NAS Synology (Asciinema).</figcaption>
</div>

#### Structure de l'Interface Full-Frame

1. **Bannière d'En-Tête Système Multi-Lignes (Style Fastfetch Dynamique)** :
   - Ligne 1 : Horloge temps réel (`HH:MM:SS`), nom d'hôte (`🖥️ hostname`), distribution OS et architecture (`Debian 13 x86_64`, `CloudLinux 8`, etc.), nombre de workers actifs, progression de la file batch active (`Batch: X/Y (Z%)` si un batch est en cours), pourcentage CPU global hôte, mémoire vive consommée / totale (`RAM: X.X/Y.Y GiB`).
   - Ligne 2 : Modèle et nombre de cœurs processeur (`🧠 CPU: <modèle> (N cœurs)`), modèle de carte graphique ou accélérateur détecté (`🎮 GPU: <modèle>`) avec télémesures de décodage matériel en direct (`intel_gpu_top` : `Décodage: X.X% [bar]`, Render/3D, fréquence `MHz`, puissance `W`) ou mention transparente du mode actif (`Mode CPU Multithread` ou `Non détecté`).
   - Ligne 3 : Débits réseau et pipe : Instantané réseau ingress physique (`Net RX`), débit de traitement du pipe de hachage (`Pipe pHash`), ainsi que le débit cumulé global de la session courante (`X.X MB/s` sur `Y.Y GiB` en `XhYYm` sur `N` flux).

2. **Onglet 1 : Tâches en Cours (`[1]`)** :
   - Table interactive des processus actifs (`dbsrt` et `ffmpeg`), moteurs d'exécution (`GPU VA-API`, `CPU Software`, `Sub Extraction`), débits instantanés, barres de progression textuelles, ETA et consommation mémoire/CPU.
   - En-tête dynamique contextuel : affiche le décompte des tâches actives et la progression globale de la file de traitement (`[X/Y actifs │ File: C/T traités, R en attente]`).

3. **Onglet 2 : Historique & Statistiques Réelles (`[2]` ou `[h]`)** :
   - Tableau de distribution statistique descriptive (Min, Médiane, Moyenne, 90e percentile P90, Max, Écart-type $\sigma$) sur les décodages intégraux pHash (durée > 60s) : débits effectifs (`MB/s`), durées de traitement (`minutes`), tailles des fichiers (`GiB`) et couplings de sous-titres certifiés.

4. **Onglet 3 : Derniers Traitements Complétés & Certifiés (`[3]` ou `[r]`)** :
   - Journal complet et scrollable de l'ensemble des médias traités, avec hiérarchie rigoureuse des statuts et sévérités : succès (`✓ OK`, `✓ CACHE`), avertissement fonctionnel sans sous-titres (`⚠ NO_SRT`), interruption volontaire opérateur (`⏹ CANCEL`) et échecs critiques système (`✗ FAIL`).
   - Filtrage dynamique cyclique (`[f]`) : `TOUS`, `SUCCÈS (OK/CACHE)`, `FAIL (ERREURS SEULES)`, `SANS SOUS-TITRES (NO_SRT)`, `ANNULÉS (CANCEL)`.
   - Tri multicritère cyclique (`[o]`) : `DATE (RÉCENT)`, `DATE (ANCIEN)`, `STATUT (FAIL / CANCEL / NO_SRT)`, `NOM (A-Z)`, `DURÉE`, `TAILLE`, `DÉBIT CIFS`, `COUPLINGS`.
   - Fiche Diagnostic & Erreur (`[Enter]` ou `[i]`) : inspection plein écran affichant l'explication textuelle complète de l'échec ou de l'annulation (`error_summary`), le chemin vidéo complet, les métriques d'exécution et la découverte automatique des logs d'exécution sur disque (`data/logs/` et `/tmp`).

5. **Onglet 4 : Explorateur de Médias Multi-Sources (`[4]` ou `[m]`)** :
   - Catalogue unifié de tous les répertoires sources configurés et actifs, présentant l'inventaire des fichiers vidéo, source d'origine, tailles, durées estimées, codecs vidéo détectés (`hevc`, `h264`, `h264 Hi10P`, etc.), présence et langues des pistes de sous-titres intégrées, ainsi que le cycle de vie d'ingestion (`VIERGE`, `EN COURS`, `COMPLÉTÉ`, `HACHÉ SEUL`).
   - **Mode Nœud Distant Streaming (Zéro-NVMe / Cloud)** : Sur un nœud sans répertoire local monté (ex: worker o2switch opérant en streaming réseau pur), l'explorateur est alimenté automatiquement par le registre des cibles du batch en cours (`data/reports/selected_batch_targets.json`). Les scans disques locaux périodiques sont automatiquement neutralisés pour éliminer tout clignotement ou latence I/O inutile.
   - Filtrage dynamique cyclique (`[f]`) : `TOUS`, `VIERGES SEULS`, `EN COURS`, `COMPLÉTÉS`, `HACHÉS SEULS`, `AVEC SOUS-TITRES`, `SANS SOUS-TITRES`.
   - Tri multicritère cyclique (`[o]`) : `NOM (A-Z)`, `TAILLE`, `DURÉE`, `STATUT`, `SOUS-TITRES`.
   - Sélection multiple par case à cocher : touche `[Espace]` pour cocher/décocher le média pointé, touche `[a]` pour tout cocher/décocher.
   - Déclenchement de lot ciblé (`[b]`) : exécute le batch d'ingestion en tâche de fond exclusivement sur les médias sélectionnés et rebascule automatiquement sur l'onglet des Tâches avec notification.

6. **Onglet 5 : Gestion des Sources de Médias / Pont Streaming Réseau (`[5]` ou `[s]`)** :
   - **Mode Local (Station de Dev / Hôte)** : Gestionnaire interactif CRUD complet des répertoires de stockage vidéo scannés en parallèle (stocké dans `data/reports/media_sources.json`).
     - Initialisation automatique par défaut avec la source NAS (`$NAS_TVSHOW_DIR` ou bibliothèque TVShow par défaut).
     - `[a]` : Ajout interactif d'une nouvelle source via modal (validation stricte : chemin absolu obligatoire et répertoire existant).
     - `[e]` : Édition du chemin et du nom de la source sélectionnée (validation du répertoire).
     - `[Espace]` : Activation / désactivation dynamique de la source (statut `[✓] ACTIF` vs `[ ] DÉSACTIVÉ`).
     - `[d]` : Suppression de la source sélectionnée avec dialogue modal de confirmation (protection de la dernière source restante).
     - `[r]` : Déclenchement d'un rescan asynchrone immédiat de l'ensemble des sources actives.
   - **Mode Déporté Réseau (Zéro-NVMe / o2switch / RPi4)** : En l'absence de montages de stockage physiques locaux, la vue commute automatiquement en moniteur de flux streaming réseau pour la source FTP du NAS (ex: `ftp://nas:21`).
     - Affiche l'état en direct du canal de transit, le nombre de cibles batch cataloguées et le respect du plafond de bande passante CloudLinux LVE ($42\text{ MiB/s}$).
     - Actions d'ajout/édition de répertoires physiques verrouillées avec notification (l'administration physique étant dévolue à la station hôte).
     - `[r]` : Actualisation immédiate du registre des cibles streaming réseau sans déclenchement de scan disque superflu.

7. **Onglet 6 : Télémétrie Système & Matériel (Vue Fastfetch `[6]` ou `[y]`)** :
   - Tableau de bord exhaustif en double colonne structuré par domaines thématiques :
     - **Applicatif (`dbsrt`)** : version sémantique, hash git de provenance, date de compilation, chemin d'exécutable, PID et thread pool Rayon.
     - **Nœud & OS** : hostname FQDN, distribution Linux, version du noyau (`uname -r`), architecture CPU, et temps de fonctionnement (`uptime`).
     - **Processeur (CPU)** : modèle CPU commercial complet, topologie (cœurs physiques + logiques), fréquence d'horloge et jauge de charge.
     - **Accélération Matérielle (GPU)** : périphérique détecté, pilote/moteur actif (`Intel Iris Xe Graphics`, `VA-API iHD`), statut et barres d'activité matérielle.
     - **Mémoire & Stockage** : RAM vive (utilisée, totale, pourcentage), swap actif, et espace disque libre sur la partition de travail.
     - **Débits & Flux Réseau** : bande passante ingress (`Net RX`), débit d'ingestion pHash et volume global cumulé sur la session.

#### Raccourcis Clavier

- `[1]` : Afficher l'onglet des tâches en cours
- `[2]` / `[h]` : Afficher l'onglet des statistiques historiques réelles
- `[3]` / `[r]` : Afficher l'onglet des traitements complétés et certifiés
- `[4]` / `[m]` : Afficher l'onglet de l'explorateur de médias multi-sources
- `[5]` / `[s]` : Afficher l'onglet de gestion des sources de médias (ou `[s]` pour arrêter une tâche dans l'onglet 1)
- `[6]` / `[y]` : Afficher la vue diagnostics système Fastfetch
- `[Tab]` / `[Shift+Tab]` : Navigation circulaire bidirectionnelle entre les 6 onglets
- `[↑]` / `[k]` : Déplacer le curseur vers le haut / élément précédent
- `[↓]` / `[j]` : Déplacer le curseur vers le bas / élément suivant
- `[PgUp]` / `[PgDn]` : Défilement rapide par page (onglets 1, 3, 4 et 5)
- `[Home]` / `[End]` : Saut direct en début ou fin de liste (onglets 1, 3, 4 et 5)
- `[Espace]` : Cocher / décocher un média (onglet 4) ou activer/désactiver une source (onglet 5)
- `[a]` : Cocher / décocher tous les médias visibles (onglet 4) ou ajouter une source (onglet 5)
- `[e]` : Éditer la source sélectionnée (onglet 5)
- `[d]` : Supprimer la source sélectionnée (onglet 5)
- `[r]` : Déclencher un rescan complet des sources (onglet 5)
- `[f]` : Changer le filtre des éléments visibles (onglets 3 et 4)
- `[o]` : Changer le critère de tri (onglets 3 et 4)
- `[b]` : Lancer l'ingestion batch sur les médias cochés (onglet 4)
- `[S]` : Filtrer les médias par source d'origine (cyclique : toutes -> source 1 -> source 2...) (onglet 4)
- `[p]` : Mettre en pause ou reprendre le processus sélectionné (SIGSTOP / SIGCONT) (onglet 1)
- `[s]` : Arrêter proprement le processus sélectionné (SIGTERM) (onglet 1)
- `[K]` : Tuer immédiatement le processus sélectionné (SIGKILL en inspection) (onglet 1)
- `[x]` : Arrêter proprement TOUT le batch en cours (coordinateur Python + workers ffmpeg via SIGTERM) (onglets 1 et 4)
- `[X]` : Tuer immédiatement (SIGKILL) TOUT le batch en cours (onglets 1 et 4)
- `[Enter]` / `[i]` / `[l]` : Basculer sur la page d'inspection / diagnostic (onglets 1, 3 et 4)
- `[Esc]` / `[Backspace]` : Revenir à l'onglet précédent ou à la liste principale (ou quitter depuis l'onglet 1)
- `[?]` : Afficher / masquer la page d'aide
- `[q]` : Quitter le moniteur TUI

---

### `bench-phash`, `bench-video`, `bench-cuts`
Micro-benchmarks de performance matérielle (débit de calcul pHash en frames/sec, décodage vidéo matériel vs CPU, détection des changements de plan).
