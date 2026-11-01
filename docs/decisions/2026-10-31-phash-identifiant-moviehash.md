# Identifiant des `.phash` par moviehash (spec validée le 01/11)

## Problème

Le nom d'un `.phash` est dérivé du nom du média (`<nom>.<fps>fps.phash`). Sur o2switch :

- un nom long dépasse `NAME_MAX` (255 octets) : `Filename too long (os error 36)`. Le fail-fast
  (branche `fix/batch_worker_reporting`) fait échouer en 1 s au lieu de 7-12 min, mais ces médias
  ne sont toujours **pas** traités ;
- `scan_existing_phash` (`crates/dbsrt-cli/src/commands/shared.rs`) retrouve un `.phash` par
  préfixe ou par stem, sans le loguer : un run « COMPLETED » en 2 s n'a rien décodé, et deux
  médias de même stem peuvent partager une empreinte.

## Décision proposée

- Identifiant = moviehash OpenSubtitles (`compute_opensubtitles_hash`,
  `crates/dbsrt-core/src/metadata.rs`), 16 caractères hexadécimaux, calculé aussi sur un flux FTP/HTTP
  (lecture des 64 Kio de début et de fin + taille).
- Média FTP : le worker batch (`o2switch_batch_worker.py`) calcule le moviehash via `ftplib`
  (commande `REST` sur les 64 Kio de début et de fin, taille issue du listing MLSD) et le passe à
  `media:add`. Côté Rust, aucun client FTP ni nouvelle dépendance : seul ffmpeg lit le flux.
- Fichiers : `<moviehash>.<fps>fps.phash` et un sidecar `<moviehash>.json` (nom d'origine, chemin
  relatif, taille, fps, durée, date d'extraction).
- Lecture : recherche par moviehash d'abord ; l'ancien nommage reste lu (compatibilité) le temps
  de la migration.

## Périmètre (et rien d'autre)

1. Calcul du moviehash sur fichier local (Rust) et sur flux FTP (worker Python), même résultat
   sur les mêmes octets.
2. Écriture `<moviehash>...phash` + sidecar JSON dans `media:add` ; le log du worker
   (`logs/workers/<stem>.log`, dérivé du nom du média) est nommé par moviehash lui aussi.
3. Lecture par moviehash dans `scan_existing_phash`, avec une ligne de log
   `Extraction: reused existing .phash (N frames), no decode` quand un fichier est réutilisé.
4. Consommateurs du nom de fichier : `mmap.rs`, dbphash, couplages, correspondance par stem du
   catalogue.
5. Migration des `.phash` existants (tâche `task`, avec simulation d'abord).

Hors périmètre : la reprise d'extraction (branche `wip/resume_seek_ab`), l'alignement des versions
de ffmpeg.

## Critères de fin

- Un média au nom de 250+ octets est extrait sur o2switch et son `.phash` est créé (test réel).
- Deux médias de même stem et de contenu différent produisent deux `.phash` distincts (test unitaire).
- Toute réutilisation d'un `.phash` existant est loguée (test unitaire + log réel).
- Les `.phash` à l'ancien nom restent trouvés jusqu'à leur migration (test unitaire).
- Recette : `cargo test`, `pytest tests/tools/`, pre-commit, puis déploiement et vérification réelle
  avant commit final.
