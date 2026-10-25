# Guide Pratique : Façade Unifiée d'Ingestion Média (`task media:add`)

Ce guide opérationnel (format Diátaxis *How-to Guide*) présente l'utilisation de la façade unifiée `task media:add`, introduite dans la Phase 1 (P0) du RFC #22 pour simplifier radicalement l'ingestion de nouveaux médias dans `pydbsrt`.

---

## 1. Vue d'Ensemble & Comparatif UX

Jusqu'alors, l'ingestion d'un média et l'activation de ses sous-titres requéraient l'enchaînement de 5 commandes distinctes. La façade unifiée remplace cette séquence manuelle par une seule commande déclarative :

| Caractéristique | Workflow Manuel Précédent (L3) | Façade Unifiée (L1 / RFC #22) |
| :--- | :--- | :--- |
| **Nombre d'étapes** | 5 commandes séquentielles | **1 seule commande déclarative** |
| **Commandes exécutées** | `export-phash` + `cp` + `certify-coupling` + `catalog:ingest` + `reload` | `task media:add -- <video>` |
| **Résolution sous-titres** | Chemins et langues explicites obligatoires | **Auto-découverte par stem et suffixe** |
| **Résolution framerate** | Argument explicite ou lecture manuelle | **Sondage conteneur automatique via ffprobe** |
| **Gestion des doublons** | Risque d'écrasement ou d'erreurs SQL | **Déduplication CAS idempotente native** |
| **Comportement hors-ligne** | Échec bloquant de l'étape API REST | **Dégradation gracieuse avec persistance disque** |

---

## 2. Démarrage Rapide (Quickstart)

Pour ingérer une vidéo de référence et appairer automatiquement ses sous-titres :

```bash
task media:add -- /chemin/vers/film.mkv
```

Ou directement avec le binaire compilé :

```bash
./target/release/dbsrt media:add /chemin/vers/film.mkv
```

### Sortie Terminale Observée

```text
=== MEDIA:ADD INGESTION SUMMARY ===
Media Video      : "/chemin/vers/film.mkv"
Framerate (FPS)  : 23.976
pHash File       : "/tmp/film.mkv.24.0fps.phash"
Couplings Added  : 1
Couplings Skipped: 0
Catalog Ingested : true
```

---

## 3. Matrice des Auto-Détections (Convention over Configuration)

La façade applique des règles de convention strictes pour minimiser les paramètres requis :

### Découverte des Sous-Titres (Stem Pairing)

Le système extrait le stem du média (nom de fichier dépouillé de son extension conteneur) et analyse le répertoire parent ainsi que les sous-dossiers courants (`subs/`, `Subs/`, `subtitles/`) :

- **Cas 1 sous-titre trouvé** : Appairage automatique transparent.
- **Cas N sous-titres trouvés** : Si les stems correspondent avec des suffixes linguistiques distincts (`film.fr.srt`, `film.en.srt`), certification en chaîne de chaque couple.
- **Cas 0 sous-titre trouvé** : Interruption propre avec message explicatif invitant à spécifier `--srt` ou à télécharger la piste via `dbsrt fetch-subtitles`.

### Auto-Détection du Framerate (FPS)

Le framerate est déterminé selon la hiérarchie de priorité suivante :

1. Argument explicite `--fps <valeur>`.
2. Inspection du conteneur via `ffprobe` avec un délai d'attente borné (`FFPROBE_TIMEOUT_SECS = 5`).
3. Analyse lexicale des métadonnées du nom de fichier via `parse_fps_from_stem` (`23.976` pour les releases cinéma / Bluray / WEB, `25.0` pour PAL).
4. Valeur de repli par défaut : `DEFAULT_BASELINE_FPS = 24.0`.

### Détection de la Langue

Le code langue normalisé ISO 639-1 est extrait du suffixe :

- `film.fr.srt`, `film.fre.srt`, `film.fra.srt` -> `fr`
- `film.en.srt`, `film.eng.srt` -> `en`
- `film.es.srt`, `film.spa.srt` -> `es`
- `film.de.srt`, `film.ger.srt` -> `de`

---

## 4. Scénarios Concrets d'Utilisation

### Scénario 1 : Découverte Implicite (1 Vidéo + 1 Fichier SRT Sidecar)

Arborescence :
```text
mes_videos/
├── Dune.Part.Two.2024.1080p.mkv
└── Dune.Part.Two.2024.1080p.fr.srt
```

Commande :
```bash
task media:add -- mes_videos/Dune.Part.Two.2024.1080p.mkv
```

Résultat : La vidéo est sondée à 23.976 fps, le fichier `.fr.srt` est associé en langue française, l'empreinte pHash est calculée, le couplage certifié dans `couplings.json`, et le catalogue en RAM est mis à jour.

---

### Scénario 2 : Média Multilingue avec Sous-Dossier `Subs/`

Arborescence :
```text
series/
├── Episode.01.mkv
└── Subs/
    ├── Episode.01.fr.srt
    └── Episode.01.en.srt
```

Commande :
```bash
task media:add -- series/Episode.01.mkv
```

Résultat : Les pistes française et anglaise sont toutes deux certifiées et associées à la même empreinte visuelle.

---

### Scénario 3 : Précision Explicite des Paramètres

Si les fichiers ont des noms atypiques ou requièrent un forçage :

```bash
task media:add -- --video video_brute.mp4 --srt sous_titre_externe.srt --lang fr --fps 25.0
```

---

### Scénario 4 : Fichier MKV/MP4 avec Sous-Titres Embarqués

Arborescence :
```text
mes_videos/
└── Film.2024.1080p.mkv   (contient des flux de sous-titres internes fr/en)
```

Commande :
```bash
task media:add -- mes_videos/Film.2024.1080p.mkv
```

Résultat :

1. Aucun fichier `.srt` sidecar n'étant présent, le conteneur est inspecté via `ffprobe`.
2. Les flux internes de sous-titres sont identifiés et démultiplexés via `ffmpeg` vers `data/SRT/Film.2024.1080p.<lang>.srt`.
3. Les sous-titres extraits sont certifiés et couplés à l'empreinte pHash calculée.

---

## 5. Gestion des Cas Limites et Robustesse

### Idempotence et Déduplication CAS

L'exécution répétée de la commande sur un média déjà certifié ne corrompt ni ne réécrit inutilement les données :

- Calcul de l'empreinte pHash (`media_hash`) et du SHA-256 du sous-titre (`subtitles_hash`).
- Si le registre contient déjà cette paire exacte : certification ignorée (*skip*), notification informative en console, code de sortie `0`.

### Serveur `dbsrt-server` Hors-Ligne

Si le démon serveur n'est pas actif lors de l'exécution :

- L'extraction pHash et l'écriture dans `couplings.json` s'effectuent normalement sur disque.
- Un avertissement non-bloquant est émis :
  ```text
  ⚠️ dbsrt-server offline (http://127.0.0.1:8765): .phash extracted and coupling certified to disk, will be loaded on next server startup.
  ```
- Le code de sortie reste `0`. Dès le prochain démarrage du serveur, le catalogue disque charge immédiatement les nouvelles entrées.

### Mode Répétition / Dry-Run (`--skip-certify`)

Pour tester l'extraction sans modifier les registres ni notifier le serveur :

```bash
task media:add -- tests/data/big_buck_bunny_trailer_480p.mp4 --skip-certify
```

---

## 6. Référence des Options CLI

| Option CLI | Valeur par défaut | Description |
| :--- | :--- | :--- |
| `-v, --video <VIDEO>` | Positionnel 1 | Chemin vers le fichier vidéo de référence |
| `-s, --srt <SRT>` | Positionnel 2 | Chemin vers le fichier `.srt` (optionnel si découverte auto) |
| `-l, --lang <LANG>` | Auto-détecté | Code langue ISO 639-1 (ex: `fr`, `en`) |
| `--fps <FPS>` | Auto-détecté | Forçage de la fréquence d'images |
| `--phash-dir <DIR>` | `/tmp` | Répertoire de stockage des fichiers `.phash` |
| `--subtitles-dir <DIR>` | `data/SRT` | Répertoire de destination des `.srt` et de `couplings.json` |
| `--skip-certify` | `false` | Mode simulation : extrait l'empreinte sans enregistrer le couplage |
| `--json` | `false` | Affiche le rapport d'exécution au format JSON structuré |
| `--server-url <URL>` | `http://127.0.0.1:8765` | URL de l'instance `dbsrt-server` pour l'ingestion RAM |
