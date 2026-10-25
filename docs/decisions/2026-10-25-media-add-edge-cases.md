# Architecture Decision Record (ADR) : Façade Unifiée `task media:add` & Gestion des Cas Limites (Edge Cases)

- **Date** : 2026-10-25
- **Statut** : Accepté / RFC #22 Phase 1 (P0)
- **Contexte** : Roadmap UX pydbsrt — Unification de l'ingestion média en 1 commande CLI/Taskfile
- **Périmètre** : Façade CLI unifiée `dbsrt media:add` et cible Taskfile `task media:add` (P0)

---

## 1. Contexte & Problématique

Jusqu'à la version 0.1.x, l'ingestion d'un nouveau média et la mise à disposition de ses sous-titres synchronisés nécessitent l'enchaînement manuel de 5 commandes distinctes (L3) :

1. `dbsrt export-phash --media <video> -o <path.phash>`
2. Déplacement/copie du fichier `.srt` vérifié dans le catalogue
3. `dbsrt certify-coupling --phash-file <f.phash> --subtitle <f.srt> --lang <lg>`
4. `task catalog:ingest -- <f.phash> <fps>` (`POST /v1/catalog/ingest`)
5. Rechargement serveur éventuel (`task catalog:reload`)

Cette dispersion engendre un taux d'erreur utilisateur élevé (~15% d'omissions de flags), une friction cognitive importante et un temps d'ingestion fragmenté.

L'objectif de la **Phase 1 (P0)** du RFC #22 est de fournir une façade unifiée :

```bash
task media:add -- <chemin_video> [chemin_srt] [--lang <lg>] [--fps <val>]
```

ou directement :

```bash
dbsrt media:add --video <chemin_video> [--srt <chemin_srt>] [--lang <lg>] [--fps <val>]
```

Ce document consigne les décisions d'architecture et la résolution systématique des 5 cas limites (*edge cases*) identifiés.

---

## 2. Décisions d'Architecture sur les 5 Cas Limites

### Edge Case 1 : `task media:add film.mkv` sans SRT explicite

**Problématique** : L'utilisateur fournit uniquement le fichier vidéo sans préciser de sous-titre.

**Règles de Résolution (Convention over Configuration)** :

1. **Recherche de paires par stem** : Le système extrait le stem du média (nom de fichier dépouillé de son extension conteneur `.mkv`, `.mp4`, `.webm`, `.avi`, `.mov`) et recherche les fichiers `*.srt` résidant dans le même répertoire ou sous-dossiers `Subs/` (recherche insensible à la casse).
2. **Scénario 0 SRT sidecar trouvé (Fallback Conteneur Embarqué)** :
    - *Inspection automatique du conteneur* : Le système inspecte le conteneur vidéo via `dbsrt_core::inspect_container` (`ffprobe`).
    - *Extraction des flux embarqués* : Si des pistes de sous-titres internes sont détectées, elles sont extraites sans ré-encodage via `dbsrt_core::extract_embedded_subtitle` (`ffmpeg`) et écrites dans `data/SRT/<video_stem>.<lang>.srt`.
    - *Erreur bloquante uniquement si aucun flux détecté* : Si aucun flux interne n'existe ou si l'inspection échoue, émission d'une erreur bloquante : `Aucun sous-titre .srt trouvé pour '<video>'. Spécifiez un fichier via '--srt <path>' ou téléchargez-le via 'dbsrt fetch-subtitles -q \"<titre>\" -l fr'.` Code de sortie : `1`.
3. **Scénario 1 SRT trouvé** : Association automatique transparente, détection de la langue via le stem/suffixe ou repli.
4. **Scénario N (2+) SRT trouvés** : Si tous les SRT découverts portent le même stem avec des suffixes linguistiques distincts et valides (ex: `film.fr.srt` et `film.en.srt`), le système traite et certifie chaque couple linguistique séquentiellement. Si les fichiers trouvés sont ambigus ou ne suivent pas la convention de nommage par langue, l'opération s'interrompt avec une erreur explicite demandant d'indiquer le sous-titre cible via `--srt <path>`.

---

### Edge Case 2 : Média déjà certifié (Idempotence & Déduplication CAS)

**Problématique** : Ré-exécution accidentelle ou automatisée de `media:add` sur un média déjà ingéré.

**Algorithme CAS (Content-Addressable Storage)** :

1. Calcul du digest pHash du média : `media_hash` via `dbsrt_core::compute_media_phash_digest`.
2. Calcul du digest SHA-256 du sous-titre : `subtitles_hash` via `dbsrt_core::compute_bytes_digest_32`.
3. Consultation du registre `CouplingRegistry` (`data/SRT/couplings.json`) :
    - **Cas A : Identité Totale (`media_hash` ET `subtitles_hash` déjà présents)** : Skip complet de la certification et de la copie de fichier. Notification : `Média déjà certifié avec empreinte identique (media_hash=<hash>, lang=<lang>). Ingestion ignorée (idempotent).` Exit code : `0` (succès nominal sans ré-écriture).
    - **Cas B : Même Média, Nouveau ou Autre Sous-Titre (`media_hash` existant, `subtitles_hash` différent)** : Mise à jour/ajout (*upsert*) de la piste de sous-titre dans le registre. Notification : `Sous-titre mis à jour pour le média existant (media_hash=<hash>, lang=<lang>).` Poursuite du flux vers l'ingestion RAM.

---

### Edge Case 3 : Serveur `dbsrt-server` hors-ligne (Offline Graceful Degradation)

**Problématique** : L'ingestion est exécutée alors que le serveur HTTP n'a pas été démarré.

**Options évaluées** :

- **Option A (Rejetée pour P0)** : `media:add` délègue entièrement le traitement à une nouvelle route HTTP unifiée `POST /v1/media/pipeline` (L2 API pipeline). Fausse abstraction si le serveur est arrêté ; impose une modification du serveur Axum et échoue hors-ligne.
- **Option B (Retenue — L1 orchestrant L3)** : `media:add` orchestre directement en local les primitives CLI/core du moteur, puis tente une synchronisation réseau non bloquante.

**Séquence Opérationnelle de l'Option B** :

1. Extraction de l'empreinte perceptuelle vers le fichier `.phash`.
2. Validation et certification du couplage avec écriture atomique dans `couplings.json`.
3. Tentative d'appel HTTP vers `POST http://127.0.0.1:8765/v1/catalog/ingest` avec timeout strict (`DEFAULT_HTTP_TIMEOUT_SECS = 2`).
4. Si le serveur répond HTTP 200 : confirmation de l'ingestion dynamique en RAM (`DeltaIndex`).
5. Si connexion refusée ou timeout : affichage d'un avertissement non-bloquant indiquant que le serveur est hors-ligne et que l'empreinte et le couplage enregistrés sur disque seront chargés au prochain démarrage.
6. Code de sortie final : `0`.

**Justification** : Robustesse maximale en environnement autonome (scripts batch, import NAS sans démon serveur actif), découplage strict, zéro interruption du workflow CLI.

---

### Edge Case 4 : Auto-détection de la Fréquence d'Images (Framerate / FPS)

**Problématique** : Les décalages de sous-titres requièrent le framerate exact pour convertir millisecondes et trames.

**Hiérarchie de Priorité Stricte** :

1. **Priorité 1** : Argument explicite `--fps <val>` fourni par l'utilisateur.
2. **Priorité 2** : Détection conteneur via `ffprobe` (`dbsrt_core::probe_video`) encapsulé avec timeout strict (`FFPROBE_TIMEOUT_SECS = 5`).
3. **Priorité 3** : Heuristique sur le nom de fichier via `dbsrt_index::parse_fps_from_phash_stem` (ex: `film_23.976fps.mkv`).
4. **Priorité 4 (Repli final)** : Constante documentée `DEFAULT_BASELINE_FPS = 24.0`.

**Tolérance aux pannes** : Si `ffprobe` est absent du système (`Command::new("ffprobe")` échoue), un log d'avertissement est émis et le système bascule immédiatement sur les priorités 3 et 4 sans interrompre le traitement.

---

### Edge Case 5 : Auto-détection de la Langue du Sous-Titre

**Problématique** : Les fichiers de sous-titres contiennent généralement leur code linguistique dans le nom de fichier (`.fr.srt`, `.en.srt`), mais peuvent être ambigus (`film.srt`).

**Hiérarchie de Résolution** :

1. **Priorité 1** : Argument explicite `--lang <code>` passé en CLI.
2. **Priorité 2** : Analyse lexicale des tokens du nom de fichier via `detect_cli_subtitle_lang` (prend en compte `.fr`, `.fre`, `.fra`, `.vff`, `.vostfr`, `.en`, `.eng`, etc.).
3. **Priorité 3** : Si indéterminable et aucun argument explicite n'a été fourni, émission d'une erreur explicite invitant l'utilisateur à préciser la langue via `--lang <fr|en|...>`. Code de sortie : `1`.

---

## 3. Classification Fact-First Rigor

Conformément aux standards du projet `pydbsrt` :

- **Empirically Proven** : Extraction de pHash et détection des coupures via `dbsrt export-phash` sur vidéo réelle (`tests/data/big_buck_bunny_trailer_480p.mp4`). Écriture atomique dans `couplings.json` via `CouplingRegistry`. Ingestion dynamique RAM via `POST /v1/catalog/ingest` sur `dbsrt-server` actif.
- **Hermetically Mocked** : Découverte de paires stem dans `media_pairing.rs` validée sur arborescence temporaire mockée (`tempfile`). Cas de serveur hors-ligne simulé avec port inaccessible / écouteur TCP fermé. Détection ffprobe avec flux simulés et timeout contrôlé.
- **Hypothetical / Untested** : Pipeline distant L2 (`POST /v1/media/pipeline`) : exclu du périmètre de la Phase 1. Extraction de sous-titres embarqués dans MKV : différé à la Phase 2.

---

## 4. Stratégie de Fixtures de Test

Pour garantir une couverture > 60% et satisfaire les critères de validation :

- **Fixtures Vidéos Existantes** :
    - `tests/data/big_buck_bunny_trailer_480p.mp4` (vidéo réelle avec piste vidéo et audio)
    - `tests/data/big_buck_bunny_trailer_480p.fr.srt` (sous-titres français valides)
    - `tests/data/big_buck_bunny_trailer_480p.en.srt` (sous-titres anglais valides)
- **Nouvelle Fixture Synthétique MKV** :
    - Générer un échantillon conteneur Matroska ultra-léger (~30 Ko, 1 seconde, 24 fps, flux vidéo testsrc généré via `ffmpeg`) pour vérifier explicitement la compatibilité conteneur MKV et le probing `ffprobe`.
    - Intégrer un helper de génération à la volée dans les tests unitaires/d'intégration si `ffmpeg` est présent, avec fallback hermétique mocké si absent.
- **Fixtures de Découverte de Fichiers (Stem Pairing)** :
    - Tests unitaires utilisant `tempfile::TempDir` pour créer des arbres de fichiers inertes (`film.mkv`, `film.fr.srt`, `film.en.srt`, `autre.srt`) sans exiger de flux vidéo réel pour la logique d'appairage.

---

## 5. Liste Ordonnée des Commits Prévus

1. `feat(cli): auto-discover subtitle pairs by filename stem`
    - Module `crates/dbsrt-core/src/media_pairing.rs`
    - Découverte stem insensible à la casse, 0/1/N sous-titres, tests unitaires
2. `feat(core): auto-detect fps via ffprobe with graceful fallback`
    - Extension `probe_video` avec constante `FFPROBE_TIMEOUT_SECS = 5` et replis ordonnés
    - Tests unitaires avec ffprobe et mock
3. `feat(cli): add unified media:add subcommand orchestrating phash+certify+ingest`
    - Module `crates/dbsrt-cli/src/commands/media_add.rs`
    - Sous-commande `dbsrt media:add` et options CLI associées
    - Tests d'intégration E2E
4. `feat(taskfile): add media:add orchestration target`
    - Déclaration de `task media:add` dans `taskfiles/stack.yml`
    - Script d'orchestration `scripts/media_add.sh` (conforme Pure Taskfile Orchestrator)
5. `feat(cli): idempotent media:add with CAS deduplication`
    - Méthode `CouplingRegistry::is_already_certified`
    - Déduplication CAS évitant toute ré-écriture superflue
6. `docs(diataxis): add media:add unified facade user guide`
    - Documentation utilisateur `docs/2026-10-25-guide-media-add-facade.md`
    - Enregistrement dans `mkdocs.yml` sous *Accueil & Guide*

---

## 6. Phase 1.5 : Élimination de la Corruption Silencieuse de Langue (`LangStatus` Enum)

### Problématique & Cas Réel Reproduit (Euphoria US S01E02)

Lors des tests sur des fichiers réels de la scène (`Euphoria.US.S01E02.1080p.WEBRip.x265-RARBG.mp4` avec `Euphoria.US.S01E02.1080p.WEBRip.x265-RARBG.srt` en contenu français) :

- L'analyseur lexical rencontrait le token `"rarbg"`, qui provoquait une interruption prématurée (`break`) de la boucle `rsplit`.
- Le fallback historique renvoyait alors `DEFAULT_SUBTITLE_LANG = "en"`.
- Le couplage était certifié silencieusement avec la langue `"en"`, créant une corruption de vérité-terrain (*ground-truth*).

**Règle d'ingénierie fondamentale** : Un système de vérité-terrain ne doit **jamais** deviner la langue. S'il ne peut pas déterminer le code langue avec certitude, il doit l'expliciter via `lang="und"` (ISO 639-2 *undetermined*) et tracer le niveau de confiance via un statut dédié.

### Architecture du Type `LangStatus`

Dans `dbsrt_core::lang_status` :

- `Confirmed(String)` : Langue explicitement fournie par l'utilisateur via `--lang <code_iso>`. Confiance maximale.
- `Detected(String)` : Langue extraite d'un suffixe ou jeton ISO 639 valide (`.fr.srt`, `.en.srt`, métadonnées conteneur). Confiance haute.
- `Inferred(String)` : Repli heuristique historique. Niveau incertain. Conservé pour rétrocompatibilité des anciens registres.
- `Unknown` : Langue indéterminée. Le couplage est certifié avec `lang="und"`. L'utilisateur est notifié et invité à spécifier `--lang` ou à requalifier ultérieurement.

### Stratégie de Migration des Couplages Existants

Les entrées `couplings.json` générées avant cette mise à jour ne possèdent pas le champ `lang_status`. Lors de la désérialisation, l'attribut `#[serde(default = "default_lang_status_for_legacy")]` leur assigne automatiquement `LangStatus::Inferred("und")`.

- Aucun écrasement ni perte de données : la valeur de `lang` d'origine est préservée.
- La recherche peut filtrer via `is_certain()` pour ne pas exposer de couplages incertains sur des requêtes linguistiques strictes.
- Une commande ultérieure (`dbsrt coupling:relabel`, Phase 3) permettra de requalifier formellement les entrées legacy.
