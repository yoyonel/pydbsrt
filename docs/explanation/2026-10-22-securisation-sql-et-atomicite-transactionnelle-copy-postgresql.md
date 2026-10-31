# Sécurisation SQL des Codes de Langue & Atomicité Transactionnelle COPY PostgreSQL

> **Statut :** Validé & Empiriquement Éprouvé (PostgreSQL 17 & PostgreSQL 15.2 ARM64)
> **Composant :** `crates/dbsrt-index` (`db.rs`, `lib.rs`)
> **Référence :** Issue #13, PR liée, Audits croisés GLM-4 & Claude Opus 5.5
> **Date :** 22 Octobre 2026

---

## 1. Contexte & Vulnérabilités Résolues

Lors de l'audit architectural croisé sur l'ingestion PostgreSQL dans le moteur relationnel `dbsrt-index`, deux faiblesses critiques ont été identifiées :

1. **Interpolation dynamique de chaîne SQL sur `lang` (`db.rs:328`)** :
   Le code exécutait un `UPDATE subtitles SET lang = '{}' WHERE id = $1` en insérant la chaîne nettoyée par `replace('\'', "")`. Bien que le guillemet simple fût retiré, l'interpolation de variables non assainies dans des requêtes SQL dynamiques constituait un anti-pattern dangereux face à des codes de langue malveillants ou mal formés transitant par les payloads HTTP ou la CLI.
2. **Absence d'atomicité sur le flux `COPY ... FROM STDIN` (`db.rs:213-238` & `db.rs:310-360`)** :
   L'insertion de la ligne parente (`medias` ou `subtitles`) était exécutée en autocommit avant le streaming des trames (`frames` ou `sub_frames`) via le protocole PostgreSQL `COPY`. En cas de rupture de socket réseau, de crash de processus ou de données corrompues au milieu du flux COPY, un enregistrement orphelin subsistait en base, corrompant les index et bloquant les imports ultérieurs en raison des contraintes d'unicité `UNIQUE(media_hash)`.

---

## 2. Architecture de Sécurité & Validation `validate_lang_code`

### Assainissement et Validation Typée
La fonction `validate_lang_code` valide rigoureusement chaque code de langue avant toute interaction avec la base de données :

- **Longueur bornée** : entre `MIN_LANG_CODE_LEN` (2) et `MAX_LANG_CODE_LEN` (8 caractères).
- **Format normalisé** : ISO 639-1 / ISO 639-2 ou BCP 47 (ex: `FR`, `EN`, `pt-BR`, `es-419`, `und`).
- **Rejet absolu** : tout caractère spécial, espace non périphérique, ponctuation SQL (`'`, `;`, `--`, `/*`), octets nuls et formats à tirets multiples sont rejetés avec une erreur explicite.
- **Requête SQL 100% paramétrée** : l'interpolation dynamique a été supprimée au profit du paramètre typé `$3` (`&validated_lang`) directement dans le `INSERT INTO subtitles`.

```mermaid
flowchart TD
    INPUT["Entrée brute (CLI / HTTP payload)"] --> VAL{"validate_lang_code()"}
    VAL -->|Invalide / Injection| ERR["Erreur immédiate (Bail / Reject)"]
    VAL -->|Valide (ex: 'fr', 'en-US')| NORM["Normalisation Majuscules ('FR', 'EN')"]
    NORM --> SQL["Requête SQL paramétrée ($1, $2, $3)"]
```

---

## 3. Atomicité Transactionnelle des Flux `COPY`

Les méthodes d'ingestion `import_phash_file` et `import_subtitle_coupling` ont été encapsulées dans des transactions explicites :

1. Ouverture de transaction : `let mut tx = self.client.transaction()?;`
2. Insertion de l'entité parente (`medias` ou `subtitles`) avec récupération de l'ID via `RETURNING id`.
3. Streaming massif des trames dans le flux `COPY ... FROM STDIN` au sein de la transaction.
4. Validation terminale : `tx.commit()?;`

Si une interruption réseau survient ou si les données du flux sont rejetées, l'objet transactionnel `tx` sort du scope sans appel à `commit()` : le driver `postgres` émet un `ROLLBACK` automatique, garantissant **zéro enregistrement orphelin**.

```mermaid
sequenceDiagram
    autonumber
    participant CLI as dbsrt-index / Client
    participant TX as Transaction PostgreSQL (tx)
    participant DB as Tables (medias / frames)

    CLI->>TX: BEGIN Transaction
    CLI->>TX: INSERT INTO medias (...) RETURNING id
    TX->>DB: Écriture ligne parent (non visible hors tx)
    CLI->>TX: COPY frames (...) FROM STDIN
    alt Flux Nominal
        CLI->>TX: Écriture chunks par seuil (32 KiB)
        CLI->>TX: writer.finish()
        CLI->>TX: tx.commit()
        TX->>DB: COMMIT (Persistance atomique parent + frames)
    else Échec Socket ou Corruption Données
        CLI->>TX: Exception / Erreur COPY
        Note over TX,DB: Sortie de scope sans commit
        TX->>DB: ROLLBACK automatique (zéro orphelin)
    end
```

---

## 4. Règle Zéro Magic Constants

Toutes les dimensions de buffers et paramètres de domaine sont déclarés comme constantes typées et documentées :

| Constante | Valeur | Unité & Rôle |
| :--- | :--- | :--- |
| `COPY_BUFFER_INITIAL_CAPACITY` | `64 * 1024` | Octets alloués initialement pour le buffer de batching COPY |
| `COPY_BUFFER_FLUSH_THRESHOLD` | `32 * 1024` | Seuil de flush vers le socket PostgreSQL |
| `DB_CONNECT_TIMEOUT_SECONDS` | `2` | Timeout de connexion en secondes |
| `MIN_LANG_CODE_LEN` | `2` | Longueur minimale d'un code de langue valide |
| `MAX_LANG_CODE_LEN` | `8` | Longueur maximale d'un code de langue valide |
| `DEFAULT_LANG_CODE` | `"FR"` | Code langue par défaut en cas d'entrée vide |
| `DEFAULT_BASELINE_FPS` | `25.0` | Cadence par défaut si absente des métadonnées |
| `NTSC_FILM_FPS` | `23.976` | Cadence de référence NTSC |
| `PAL_FPS` | `25.0` | Cadence de référence PAL |
| `DEFAULT_POSTGRES_PORT_CUSTOM` | `5433` | Port PostgreSQL conteneur local dev |
| `DEFAULT_POSTGRES_PORT_STANDARD`| `5432` | Port standard PostgreSQL (RPi4 / Host) |

---

## 5. Rétro-compatibilité Éprouvée (PostgreSQL 15.2 ARM64 & 17)

La compatibilité a été vérifiée empiriquement sur deux générations de serveurs de bases de données :

1. **PostgreSQL 17 SP-GiST BK-Tree** (Environnement de développement Docker local) :
   - Migration idempotente de schéma : convertit les colonnes legacy `ENUM('FR', 'EN')` vers `TEXT` de manière non-bloquante sans verrou exclusif `AccessExclusiveLock`.
   - 100% des tests unitaires et d'intégration validés.
2. **PostgreSQL 15.2 (Debian 15.2-1.pgdg110+1) aarch64** (Raspberry Pi 4 physique distant via `ssh rpi4`) :
   - Exécution directe de la suite de tests `db::tests` via `PSQL_IP=rpi4 PSQL_PORT=5432 cargo test -p dbsrt-index`.
   - Création de schéma, contraintes d'intégrité, requêtes paramétrées et rollbacks transactionnels testés avec succès (**6 passed, 0 failed**).

---

## 6. Matrice des Preuves de Test

| Catégorie | Test | Cible | Statut |
| :--- | :--- | :--- | :--- |
| **Sécurité Unitaire** | `test_validate_lang_code_accepts_valid` | `fr`, `en`, `pt-BR`, `es-419`, `und` | **PASS** |
| **Sécurité Unitaire** | `test_validate_lang_code_rejects_malicious_and_invalid` | Injections SQL, octets nuls, longueurs invalides | **PASS** |
| **Intégration DB** | `test_copy_transaction_rollback_on_failure` | Rollback `medias` sur flux `COPY frames` corrompu | **PASS** |
| **Intégration DB** | `test_subtitle_coupling_transaction_rollback_on_failure` | Rollback `subtitles` sur flux `COPY sub_frames` corrompu | **PASS** |
| **Intégration DB** | `test_db_client_full_relational_lifecycle` | Cycle de vie relationnel complet | **PASS** |
| **E2E Moteur** | `task demo:sync` | Recalage visuel de sous-titres en 0.4 ms | **PASS** |
| **E2E Ingestion** | `task coupling:certify` | Ingestion et certification de 42 couplages réels | **PASS** |
| **Multi-Plateforme** | `rpi4:tests` | RPi4 distant (PostgreSQL 15.2 aarch64) | **PASS** |
