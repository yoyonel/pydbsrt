---
title: "Modèle de Données Inputs : Sources Médias, Lignée des Actifs & Découplage de l'Extraction"
tags:
  - explication
  - architecture
  - modelisation
  - media-sources
  - media-assets
  - data-model
  - diataxis
---

# 💡 Modèle de Données Inputs : Sources Médias, Lignée des Actifs & Découplage de l'Extraction

**Date :** 8 novembre 2026
**Auteur :** Lionel ATTY (Antigravity pair programming)
**Statut :** Spécification Conceptuelle & Modèle de Données Référent (Revu post-audit)
**Contexte :** Refonte architecturale orientée *Inputs-First*. Découplage fondamental entre les sources médias physiques (conteneurs d'origine, extraits) et les artefacts calculés aval (empreintes pHash, sous-titres alignés).

---

## 1. Changement de Paradigme : De « pHash-First » à « Inputs-First »

### 1.1 L'Impasse du Modèle « Artefacts Aval »

Dans l'architecture initiale, le flux perceptuel pHash était traité comme une entité primaire flottant sur disque (`data/__PHASH__/*.phash`). Les outils de détection, les daemons d'ingestion et les compilateurs de catalogue tentaient de reconstruire l'identité du média source par rétro-ingénierie textuelle sur le nom de fichier :

- **Collisions de noms** : Un extrait de banc d'essai (cut de 26 secondes) nommé selon la release masquait le fichier physique complet de 26 Go (*shadowing*).
- **Indétermination de la complétude** : L'existence d'un fichier `.phash` sur disque était interprétée comme un « succès d'ingestion », y compris lorsque l'extraction s'était interrompue à 50 frames (2 secondes).
- **Absence de généalogie** : Aucun lien de parenté formel ne rattachait un snippet à son master source.

### 1.2 Le Principe Fondateur « Inputs-First »

Une empreinte visuelle n'a pas d'existence autonome. Elle constitue une vue projetée et dérivée d'un conteneur vidéo physique connu et validé :

$$
\text{PHashArtifact} = \mathcal{F}_{\text{extract}}\big(\text{MediaAsset}\big)
$$

Toute opération de calcul, de benchmarking ou d'alignement doit donc reposer sur un sol géométrique et sémantique préalablement certifié : le **Modèle des Inputs Médias**.

---

## 2. Modèle Conceptuel de Données (MCD)

Le modèle s'articule autour de quatre entités fondamentales :

```
┌─────────────────────────────────────────────────────────────┐
│                 MediaSource (Bibliothèque)                  │
│  - id: String (ex: "synology_tvshow")                       │
│  - root_uri: String (chemin racine ou montage CIFS)         │
│  - storage_kind: Local | CifsMount | RemoteStream           │
│  - enabled: Boolean                                         │
└──────────────────────────────┬──────────────────────────────┘
                               │ 1..* découvre
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                  MediaAsset (Actif Vidéo)                   │
│  - id: Integer (u32, aligné DB & catalog.dbphash)           │
│  - source_id: FK -> MediaSource                             │
│  - asset_kind: Master | DerivedCut | SyntheticQuery         │
│  - parent_asset_id: Option<FK -> MediaAsset> [RESTRICT]     │
│  - sample_range: Option<(start_ms, duration_ms)>            │
│  - container_identity: MovieHash (16 hex) + file_size       │
│  - source_mtime_unix: u64 (signal d'invalidation)           │
│  - probe_specs: duration_ms, fps, expected_frames, codec    │
│  - deleted_at: Option<Timestamp> (soft-delete)              │
└──────────────┬───────────────────────────────┬──────────────┘
               │ 1..*                          │ 1..1 (projection)
               ▼                               ▼
┌──────────────────────────────┐ ┌────────────────────────────┐
│      SubtitleTrack           │ │       PHashArtifact        │
│  - subtitles_hash: SHA-256   │ │  - media_hash: SHA-256    │
│  - source: Embedded | Sidecar│ │  - frame_count: Integer    │
│  - lang: ISO-639-1           │ │  - status: Complete | ...  │
│  - cue_count: Integer        │ │  - phash_rel_path: String  │
└──────────────────────────────┘ └────────────────────────────┘
```

> [!NOTE]
> **Projection 1:1 de PHashArtifact dans MediaAsset :**
> Au niveau physique (SQL et mémoire), `PHashArtifact` est modélisé comme une projection 1:1 intégrée directement dans `medias` (`media_hash`, `frame_count`, `phash_status`, `expected_frames`) pour éliminer tout surcoût de jointure sur les chemins critiques de recherche.

---

## 3. Spécification Détaillée des Entités

### 3.1 `MediaSource` : Racine de Découverte & Stockage

Représente un gisement de fichiers conteneurs (partage réseau NAS, disque local, dépôt de bancs de test). Correspond 1:1 avec `MediaSourceConfig` du TUI.

- **`id`** : Identifiant textuel pérenne (ex: `"synology_tvshow"`, `"synology_video"`, `"local_snippets"`).
- **`root_uri`** : Emplacement racine absolu ou chemin relatif canonique (`/media/storage/tvshow`, `data/snippets`).
- **`storage_kind`** : Nature physique du stockage (`Local`, `CifsMount`, `RemoteStream`).
- **`enabled`** : Drapeau d'activation pour les scans d'arrière-plan et le TUI.
- **`scan_policy`** : Règles d'inclusion/exclusion (extensions vidéo admises `.mkv`, `.mp4` ; exclusion stricte des répertoires `@eaDir`, `.recycle`, `__PHASH__`).

### 3.2 `MediaAsset` : L'Actif Conteneur Physique

Représente l'unité physique manipulée. Tout fichier vidéo scanné reçoit une fiche d'identité avant tout décodage.

#### Identifiants & Clés Métier

- **`id`** : Identifiant entier `u32` (correspondant à `SERIAL / INTEGER` dans PostgreSQL et à l'indexation 32 bits de `catalog.dbphash`).
- **Clé universelle multi-nœuds** : L'identité déterministe entre nœuds (Local, o2switch, RPi4) est assurée par le couple métier `(moviehash, file_size_bytes)`, totalement indépendante de la clé primaire séquentielle locale.

#### Typologie d'Actifs (`asset_kind`)

1. **`Master`** : Œuvre complète et intègre (film, épisode complet).
2. **`DerivedCut`** : Extrait temporel découpé depuis un `Master`. Porte obligatoirement un `parent_asset_id` valide et un intervalle temporel `sample_range = (start_ms, duration_ms)`.
3. **`SyntheticQuery`** : Fichier d'évaluation généré pour banc d'essai, structuré selon `SyntheticQueryKind` :
   - `TimeShift { offset_ms: i64 }`
   - `Noise { snr_db: f32 }`
   - `Recode { codec: String, crf: u8 }`

#### Identité de Conteneur Inviolable (`container_identity`)

- **`moviehash`** : Checksum 64-bit uint OpenSubtitles ($O(1)$) sur les 64 Ko initiaux, 64 Ko finaux et la taille fichier. Sérialisé en 16 caractères hexadécimaux.
- **`file_size_bytes`** : Taille physique exacte du fichier en octets.

*(Note : `source_mtime_unix` est déporté hors de `container_identity` pour servir uniquement de signal de détection de modification physique sur disque, sans polluer l'identité immuable).*

#### Spécifications Géométriques & Temporelles (`probe_specs`)

Obtenues en temps constant ($< 50\,\text{ms}$) par lecture des métadonnées du conteneur via `ffprobe` (sans décodage de flux) :

- **`duration_ms`** : Durée totale en millisecondes.
- **`fps`** : Fréquence d'images nominale décimale pour affichage (`f64`).
- **`fps_rational`** : Ratio exact `(u32, u32)` préservant la précision NTSC (ex: `(24000, 1001)` pour 23.976 fps).
- **`is_vfr`** : Indicateur de fréquence variable (détecté si `avg_frame_rate != r_frame_rate`).
- **`expected_frames`** : Nombre théorique de trames calculé par arrondi strict : $\text{expected\_frames} = \text{round}\left(\frac{\text{duration\_ms}}{1000} \times \text{fps}\right)$.
- **`resolution`** : Tuple `(width, height)` en pixels.
- **`video_codec`** : Codec du flux primaire (`hevc`, `h264`, `av1`).

### 3.3 `PHashArtifact` : L'Empreinte Visuelle Dérivée

L'empreinte perceptuelle est un objet dérivé, enfant exclusif d'un unique `MediaAsset`.

- **`media_asset_id`** : Clé étrangère stricte vers l'actif parent.
- **`media_hash`** : Empreinte cryptographique SHA-256 (32 hex) du flux pHash brut.
- **`frame_count`** : Nombre réel de frames décodées et enregistrées.
- **`status`** : Cycle de vie de l'empreinte (`PHashLifecycleStatus`) :
  - `Pending` : Actif découvert, extraction non encore démarrée.
  - `InProgress` : Décodage en cours (suivi par checkpoint `.partial`).
  - `Complete` : Extraction validée par la barrière de complétude.
  - `Truncated` : Extraction incomplète ou crashée ($N \ll \text{expected\_frames}$).
  - `Corrupted` : Fichier corrompu ou taille non multiple de 8 octets.
  - `Obsolete` : Rendu obsolète suite à un ré-encodage du fichier source sur disque (`moviehash` ou taille modifiée pour le même `path_rel`).
- **`phash_rel_path`** : Emplacement du fichier physique sur disque (nommé par `<moviehash>.<fps>fps.phash`).

### 3.4 `SubtitleTrack` : Piste de Sous-Titres Découverte

- **`media_asset_id`** : Clé étrangère vers le conteneur porteur.
- **`subtitles_hash`** : Empreinte SHA-256 (32 hex) du contenu textuel normalisé `.srt`.
- **`track_origin`** : `Embedded` (piste interne au MKV/MP4) ou `Sidecar` (fichier `.srt` adjacent sur disque).
- **`lang`** : Code langue ISO-639-1 (`fr`, `en`, `es`, etc.).
- **`cue_count`** : Nombre de répliques horodatées.

---

## 4. Invariants Opérationnels & Règle d'Isolation

### 4.1 Invariant d'Isolation de Lignée (Anti-Shadowing)

Un `DerivedCut` ne peut en aucun cas masquer ou remplacer son `Master` parent :

$$
\text{Asset}(\text{Spider-Man Master}) \ne \text{Asset}(\text{Spider-Man Snippet 26s})
$$

1. Le snippet reçoit son propre identifiant d'actif `asset_id_snippet`.
2. Son enregistrement mentionne explicitement `parent_asset_id = asset_id_master`.
3. L'empreinte pHash du snippet est stockée sous `data/snippets/<snippet_moviehash>.phash`.
4. Le compilateur de catalogue (`dbsrt pack`) et le banc de test filtrent les actifs selon `asset_kind = 'master'`, interdisant à tout snippet d'entrer dans la matrice de référence.

### 4.2 Invariant de la Barrière de Complétude (*CompletenessGate*)

La transition d'un `PHashArtifact` vers l'état `Complete` est conditionnée par l'assertion mathématique suivante :

$$
\big| \text{frame\_count} - \text{expected\_frames} \big| \le \max\Big(12, \; \alpha \times \text{expected\_frames}\Big)
$$

où la tolérance $\alpha$ est adaptée à la régularité temporelle du flux :
- $\alpha = 0.02$ ($2\,\%$) pour les flux à framerate constant (CFR standard).
- $\alpha = 0.05$ ($5\,\%$) pour les flux à framerate variable (`is_vfr = true`).

Si `expected_frames = 0` (actif nouvellement inséré n'ayant pas encore subi le probe), le gate est neutralisé temporairement jusqu'au probe.

---

## 5. Traduction Relationnelle (PostgreSQL 17 - Spécification V6)

Spécification formelle de la migration SQL `V6` intégrant l'ensemble des garde-fous :

```sql
-- 1. Table des sources médias
CREATE TABLE IF NOT EXISTS media_sources (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    root_uri TEXT NOT NULL,
    storage_kind VARCHAR(32) NOT NULL DEFAULT 'local',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_storage_kind CHECK (storage_kind IN ('local', 'cifs_mount', 'remote_stream'))
);

-- 2. Évolution de la table medias pour intégrer la lignée, le soft-delete et le statut
ALTER TABLE medias
    ADD COLUMN IF NOT EXISTS source_id VARCHAR(64) REFERENCES media_sources(id),
    ADD COLUMN IF NOT EXISTS asset_kind VARCHAR(32) NOT NULL DEFAULT 'master',
    ADD COLUMN IF NOT EXISTS parent_media_id INTEGER REFERENCES medias(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS sample_start_ms BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS sample_duration_ms BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS expected_frames INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS is_vfr BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS phash_status VARCHAR(32) NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS source_mtime_unix BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ,
    ADD CONSTRAINT chk_asset_kind CHECK (asset_kind IN ('master', 'cut', 'synthetic_query')),
    ADD CONSTRAINT chk_phash_status CHECK (phash_status IN ('pending', 'in_progress', 'complete', 'truncated', 'corrupted', 'obsolete'));

-- 3. Garde-fou strict sur la cohérence de lignée (Master vs Cut)
ALTER TABLE medias ADD CONSTRAINT chk_asset_lineage CHECK (
    (asset_kind = 'master'
        AND parent_media_id IS NULL
        AND sample_start_ms = 0
        AND sample_duration_ms = 0)
    OR (asset_kind IN ('cut', 'synthetic_query')
        AND parent_media_id IS NOT NULL
        AND sample_duration_ms > 0)
);

-- 4. Index unique sur l'identité conteneur pour le scan O(1)
CREATE UNIQUE INDEX IF NOT EXISTS idx_medias_container_identity
    ON medias (moviehash, file_size_bytes)
    WHERE asset_kind = 'master' AND deleted_at IS NULL;

-- 5. Indexation pour la navigation, la lignée et les filtres d'ingestion
CREATE INDEX IF NOT EXISTS idx_medias_asset_kind ON medias (asset_kind);
CREATE INDEX IF NOT EXISTS idx_medias_parent_id ON medias (parent_media_id) WHERE parent_media_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_medias_phash_status ON medias (phash_status);
CREATE INDEX IF NOT EXISTS idx_medias_source_id ON medias (source_id);
```

---

## 6. Traduction Zero-DB / Rust (Structures en Mémoire)

Dans `dbsrt-core` et `dbsrt-tui`, les structures manipulées reflètent rigoureusement ce modèle :

```rust
/// Nature de l'actif vidéo dans la bibliothèque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    /// Œuvre intégrale complète (film, épisode, documentaire).
    Master,
    /// Extrait temporel rattaché à son master d'origine.
    DerivedCut {
        parent_asset_id: u32,
        start_ms: u64,
        duration_ms: u64,
    },
    /// Échantillon synthétique d'évaluation pour banc d'essai.
    SyntheticQuery {
        parent_asset_id: u32,
        kind: SyntheticQueryKind,
    },
}

/// Typologie paramétrique des requêtes synthétiques de banc de test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyntheticQueryKind {
    TimeShift { offset_ms: i64 },
    Noise { snr_db: u32 },
    Recode { codec: String, crf: u8 },
}

/// Identifiant physique immuable calculé en temps constant O(1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContainerIdentity {
    pub moviehash: String,
    pub file_size_bytes: u64,
}

/// Fiche technique probe extraite sans décodage vidéo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeSpecs {
    pub duration_ms: u64,
    pub fps: f64,
    pub fps_rational: (u32, u32),
    pub is_vfr: bool,
    pub expected_frames: usize,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
}

/// Statut du cycle de vie de l'empreinte perceptuelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PHashLifecycleStatus {
    Pending,
    InProgress,
    Complete,
    Truncated { frames_found: usize, expected: usize },
    Corrupted,
    Obsolete,
}

/// Actif média unifié (Input certifié).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: u32,
    pub source_id: String,
    pub kind: AssetKind,
    pub container: ContainerIdentity,
    pub source_mtime_unix: u64,
    pub specs: ProbeSpecs,
    pub filename: String,
    pub path_rel: String,
    pub phash_status: PHashLifecycleStatus,
}
```

---

## 7. Plan de Transition & Rétro-qualification des `.phash` Existants

Pour traiter les 4 265 fichiers `.phash` existants sans rupture de service :

1. **Phase 1 : Raccordement Sidecar Automatique** :
   - Les 1 703 fichiers disposant déjà d'un sidecar `<moviehash>.json` sont certifiés conformes avec leur identité `(moviehash, file_size)`.
2. **Phase 2 : Rétro-qualification des Fichiers Nommés par Nom de Release** :
   - Pour chaque fichier `<release_name>.<fps>fps.phash`, si le fichier vidéo source existe sur disque : calcul du `MovieHash` et renommage / émission du sidecar canonique.
3. **Phase 3 : Éviction & Quarantaine des Tronqués** :
   - Les 290 fichiers `< 50 Ko` ou dont le `frame_count` ne franchit pas le `CompletenessGate` sont mis en quarantaine hors de `data/__PHASH__/`.
