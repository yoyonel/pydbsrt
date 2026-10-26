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

### `bench-phash`, `bench-video`, `bench-cuts`
Micro-benchmarks de performance matérielle (débit de calcul pHash en frames/sec, décodage vidéo matériel vs CPU, détection des changements de plan).
