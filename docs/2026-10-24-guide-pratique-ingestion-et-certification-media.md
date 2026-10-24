# Guide Pratique : Ingestion d'un Média, Extraction pHash et Certification de Sous-Titres

Ce guide opérationnel pas-à-pas (format Diátaxis *How-to Guide*) détaille la procédure complète pour :

1. Extraire l'empreinte visuelle pHash d'une vidéo de référence.
2. Certifier l'alignement d'un fichier de sous-titres (`.srt`) synchronisé.
3. Rendre ce couplage immédiatement interrogeable par l'extension VLC sans redémarrer le serveur (`dbsrt-server`).
4. Consolider les données dynamiques via la compaction.

---

## 1. Prérequis & Modèle Conceptuel

> [!IMPORTANT]
> **Règle Fondamentale : Aucun pHash sur un fichier texte `.srt`**
> Un sous-titre ne contient aucune image. Le recalage visuel par pHash nécessite impérativement un **Couplage Certifié (Ground-Truth)** :
>
> ```text
> [Vidéo Référence (pHash)] <==== Couplage Certifié ====> [Sous-Titre Validé (.srt)]
>          ▲
>          │ Comparaison Visuelle (Hamming / MIH)
>          ▼
> [Vidéo Cible Utilisateur] ──────── Décalage Δt ────────► Injection SRT Recalé dans VLC
> ```

### Outils & Binaire CLI
Assurez-vous que le binaire release est compilé :

```bash
cargo build --release -p dbsrt-cli -p dbsrt-server
# Binaire CLI disponible : ./target/release/dbsrt
```

---

## 2. Procédure Pas-à-Pas (Workflow Complet)

### Étape 1 : Extraire l'empreinte perceptuelle (`dbsrt export-phash`)

Pour extraire les 24 empreintes DCT-II par seconde et détecter les coupures de scènes :

```bash
./target/release/dbsrt export-phash \
  --media "/chemin/vers/film_reference.mkv" \
  -o "data/__PHASH__/MonFilm.2024.1080p.phash"
```

**Sortie terminale observée :**
```text
Probing media "MonFilm.2024.1080p.mkv"...
Media duration: 7200.00s, FPS: 24.000
Extracting pHash (Full, hwaccel=Disabled) -> "data/__PHASH__/MonFilm.2024.1080p.phash"...
Computed 172800 frame hashes in 12.4s (13935 fps, 580x real-time)
✓ Successfully wrote binary phash to "data/__PHASH__/MonFilm.2024.1080p.phash"
```

---

### Étape 2 : Déposer et Certifier le Sous-Titre Synchronisé (`dbsrt certify-coupling`)

Placez le fichier `.srt` vérifié dans le répertoire des sous-titres :

```bash
cp "/chemin/vers/MonFilm.2024.fr.srt" "data/SRT/MonFilm.2024.1080p.fr.srt"
```

Certifiez le lien direct entre l'empreinte vidéo et la piste de sous-titres :

```bash
./target/release/dbsrt certify-coupling \
  --phash-file "data/__PHASH__/MonFilm.2024.1080p.phash" \
  --subtitle "data/SRT/MonFilm.2024.1080p.fr.srt" \
  --lang fr \
  --subtitles-dir data/SRT
```

**Sortie terminale observée :**
```text
=== CERTIFIED COUPLING REGISTERED ===
Media Name       : MonFilm.2024.1080p
pHash File       : "data/__PHASH__/MonFilm.2024.1080p.phash" (172800 frames @ 24.00 fps)
Media Digest     : f455fa01902b0549f63ef5816f5f4a27
Subtitle File    : "data/SRT/MonFilm.2024.1080p.fr.srt" (lang: fr)
Subtitle Digest  : aaab2d83cf4785661b8b92288c082f3e
Validated Cues   : 1420 cues -> 1420 frame anchors (start/end offsets)
Local Registry   : "data/SRT/couplings.json"
```

> [!TIP]
> **Découverte automatique par lot :**
> Si vous déposez plusieurs couples `*.phash` et `*.srt` dans `data/`, vous pouvez tous les certifier en une seule commande via Taskfile :
> ```bash
> task coupling:certify
> ```

---

### Étape 3 : Rendre le Média Visible par le Serveur

Deux méthodes sont disponibles selon votre environnement :

#### Méthode A : Ingestion Dynamique à Chaud en RAM (Idéal Appliance / Zero-Rebuild)
Injectez l'empreinte directement dans le `DeltaIndex` du serveur sans recompiler le fichier `catalog.dbphash` :

```bash
task catalog:ingest -- "data/__PHASH__/MonFilm.2024.1080p.phash" 24.0
```

**Équivalent direct via API REST (`curl`) :**
```bash
curl -s -X POST http://127.0.0.1:8765/v1/catalog/ingest \
  -H "Content-Type: application/json" \
  -d '{
    "phash_path": "data/__PHASH__/MonFilm.2024.1080p.phash",
    "fps": 24.0
  }' | jq .
```

**Résultat instantané (< 5 ms) :**
```json
{
  "success": true,
  "media_id": 1000001,
  "clean_title": "MonFilm",
  "total_frames": 172800,
  "delta_medias_count": 1,
  "elapsed_ms": 3.86
}
```

#### Méthode B : Rechargement Atomique Global (Catalogue Disque)
Si vous compilez le corpus complet via `dbsrt pack` :

```bash
task catalog:reload
# Équivalent curl : curl -X POST http://127.0.0.1:8765/v1/catalog/reload
```

---

### Étape 4 : Vérifier l'État du Serveur (`task catalog:status`)

Vérifiez que le serveur a bien pris en compte le média dans son étage Base ou Delta :

```bash
task catalog:status
```

**Sortie JSON type :**
```json
{
  "base_medias": 1566,
  "base_cuts": 586258,
  "base_sparse": 567009,
  "delta_medias": 1,
  "delta_cuts": 25,
  "delta_sparse": 71,
  "certified_couplings": 294,
  "total_medias": 1567
}
```

Le média est immédiatement actif pour toute requête de reconnaissance (`/v1/recognize`) et synchronisation VLC.

---

### Étape 5 : Consolidation Périodique (`task catalog:compact`)

Au fil des ajouts, le `DeltaIndex` grandit en mémoire vive. Périodiquement (ex: une fois par semaine ou avant une sauvegarde), consolidez la RAM dans le fichier disque `catalog.dbphash` :

```bash
task catalog:compact
# Équivalent curl : curl -X POST http://127.0.0.1:8765/v1/catalog/compact
```

**Sortie terminale :**
```json
{
  "success": true,
  "compacted": true,
  "elapsed_ms": 142.15,
  "total_medias": 1567,
  "delta_medias_flushed": 1,
  "message": "Successfully compacted into data/catalog.dbphash"
}
```

- Le Delta est vidé de manière atomique sans interruption de trafic.
- Les requêtes concurrentes arrivant pendant la compaction sont préservées sans perte de données.

---

## 3. Synthèse des Commandes Opérationnelles

| Opération | Commande Taskfile | Commande CLI Directe / API |
| :--- | :--- | :--- |
| **Extraction pHash** | `task phash:extract -- <vidéo>` | `./target/release/dbsrt export-phash --media <vidéo> -o <path.phash>` |
| **Certification Couplage** | `task coupling:certify` | `./target/release/dbsrt certify-coupling --phash-file <f.phash> --subtitle <f.srt> --lang <lg>` |
| **Injection Dynamique RAM** | `task catalog:ingest -- <f.phash>` | `curl -X POST http://localhost:8765/v1/catalog/ingest -d '{"phash_path":"..."}'` |
| **Vérification État** | `task catalog:status` | `curl http://localhost:8765/v1/catalog/status` |
| **Rechargement Zéro Coupure** | `task catalog:reload` | `curl -X POST http://localhost:8765/v1/catalog/reload` |
| **Compaction Base + Delta** | `task catalog:compact` | `curl -X POST http://localhost:8765/v1/catalog/compact` |
