---
title: Guide Pratique — Exécuter, Écrire et Diagnostiquer les Tests de Scénarios TUI Headless
tags:
  - how-to
  - tui
  - ratatui
  - tests
  - scenarios
  - ci-cd
  - review
---

# 🛠️ Guide Pratique : Exécuter, Écrire et Diagnostiquer les Tests de Scénarios TUI Headless

> **Quadrant Diátaxis : 🛠️ Guide Pratique (How-To)**
> Ce guide détaille l'utilisation opérationnelle de la suite de tests de scénarios TUI déclaratifs de `dbsrt top` (`dbsrt studio`). Il explique comment exécuter ces tests en local et en CI/CD, comment interpréter les rapports d'erreur lors des revues de code, et comment enrichir la suite avec de nouveaux scénarios utilisateur.

---

## 1. Objectif & Intégration CI/CD

La suite de scénarios située dans `crates/dbsrt-cli/tests/test_scenarios.rs` et structurée en sous-modules sous `crates/dbsrt-cli/tests/scenarios/` simule les interactions réelles d'un opérateur sans ouvrir de terminal physique ni de serveur d'affichage (mode headless via `ratatui::backend::TestBackend`).

Elle combine deux approches complémentaires issues des meilleures pratiques du framework Ratatui :

- **Tests de Scénarios Fonctionnels (25 tests)** : simulation événementielle de frappes clavier, transitions d'états, invalidations mémoires, et vérification des libellés critiques.
- **Golden Snapshot Testing via `insta` (9 tests de grille visuelle)** : capture au caractère près de la matrice bidimensionnelle du terminal (ISO Ratatui golden buffer tests), figeant la disposition des 5 vues principales, des modales de saisie, des fiches de diagnostic et de l'aide.

### Architecture Modulaire de la Suite

```text
crates/dbsrt-cli/tests/
├── test_scenarios.rs                 # Point d'entrée de la suite d'intégration
└── scenarios/
    ├── mod.rs                        # Déclaration des modules
    ├── common.rs                     # Harnais TestHarness & mocks télémétrie
    ├── sources_crud.rs               # Gestion CRUD sources NAS & protection source unique
    ├── media_browser.rs              # Catalogue, filtres statut/source, tris, batch launch
    ├── process_lifecycle.rs          # Métriques workers, pause, signaux SIGTERM/SIGKILL
    ├── history_diagnostics.rs        # Registre récents, sévérités, fiches diagnostics d'échec
    ├── contextual_help.rs            # Parité stricte 1:1 aide contextuelle & glossaire
    ├── navigation_resizing.rs        # Navigation circulaire Tab, raccourcis & redimensionnement
    ├── visual_snapshots.rs           # Snapshots golden grids via insta
    └── snapshots/                    # Fichiers .snap validés et versionnés
```

### Est-elle présente dans la CI/CD ?

**Oui, à 100%.** Dans le pipeline GitHub Actions (`.github/workflows/main.yml`), l'étape de couverture de code exécute :

```bash
cargo llvm-cov --workspace --json --output-path coverage-rust.json
```

Cette commande englobe automatiquement l'ensemble des cibles de tests d'intégration du workspace (`crates/*/tests/`). Si l'un des scénarios TUI échoue, le job CI échoue immédiatement et bloque la Pull Request.

---

## 2. Comment Utiliser la Suite de Tests ?

### A. Exécution Rapide de la Suite de Scénarios (< 0.5s)

Pour vérifier l'interface sans lancer l'ensemble des 260 tests du workspace :

```bash
task test:scenarios
```

*Alias équivalent :* `task test:tui` ou `cargo test --package dbsrt-cli --test test_scenarios -- --nocapture`.

### B. Validation Pré-Commit Complète (< 10s)

Pour exécuter les linters, tous les tests Rust (y compris les scénarios) et la conformité de la documentation :

```bash
task ci:local
```

### C. Exécution Exhaustive du Workspace

```bash
task test:rs
```

---

## 3. Typologie des Erreurs Détectées & Formats de Sortie

La suite remonte 5 grandes catégories d'anomalies lors des tests automatiques :

| Catégorie d'Erreur | Cause Typique | Format de la Remontée dans la Console |
| :--- | :--- | :--- |
| **1. Régression de transition d'état** | Un raccourci clavier ne modifie pas la vue ou l'état mémoire attendu. | `assertion left == right failed (left: ViewPage::ActiveJobs, right: ViewPage::MediaBrowser)` |
| **2. Invalidation mémoire défaillante** | La désactivation d'une source NAS n'a pas purgé immédiatement les médias en mémoire vive. | `assertion left == right failed (left: 4, right: 2)` |
| **3. Tronquage ou régression graphique** | Un libellé, une colonne ou un compteur n'apparaît plus sur la grille écran. | `Expected buffer to contain '0 média(s) coché(s)', but was not found. Snippet: Buffer { ... }` |
| **4. Dérive de l'Aide Contextuelle** | Une colonne ou un statut métier a été ajouté au code mais omis dans l'Aide `[?]` (Règle `GEMINI.md`). | `Expected buffer to contain 'MOTEUR', but was not found. Help tab did not match originating view.` |
| **5. Panic / Crash sur frappe clavier** | Indexation hors-borne ou déréférencement `None` lors d'une action utilisateur sur liste vide. | `thread 'test_scenario_...' panicked at 'index out of bounds: the len is 0 but the index is 0'` |

---

## 4. Guide d'Interprétation pour la Revue Humaine (PR Review)

Lorsqu'une Pull Request modifie l'interface TUI, voici la grille d'interprétation pour valider les changements :

### Cas 1 : Tous les tests sont verts (`test result: ok. 34 passed`)

La validation humaine manuelle n'est plus nécessaire pour les points suivants :

- Les 5 vues principales et leurs fiches détaillées s'affichent sans régression visuelle.
- Les 9 grilles de snapshots dorés (`insta`) sont conformes au pixel/caractère près.
- Les touches de navigation, tris `[o]`, filtres `[f]`/`[S]`, sélection `[Space]`/`[a]` et batch `[b]` fonctionnent.
- L'invalidation atomique à 0 ms des sources et la protection contre la suppression de la dernière source sont garanties.
- Les statuts d'erreur et causes de pannes sont correctement qualifiés.
- L'Aide contextuelle est strictement synchronisée avec le modèle.

### Cas 2 : Échec sur une assertion de buffer (`assert_buffer_contains`)

La sortie de test affiche un extrait de la grille mémoire (`Captured buffer:`).

- **Analyse** : Comparez le texte attendu avec le contenu de la grille affichée dans le log.
- **Diagnostic de renommage** : Un libellé a été renommé dans l'UI sans mettre à jour les assertions du test.
- **Diagnostic de dimensionnement** : Une colonne a été élargie, ce qui repousse un texte au-delà de la bordure d'affichage.
- **Diagnostic de filtre** : Le filtre actif a masqué l'élément attendu.

### Cas 3 : Échec de Golden Snapshot `insta` (`snapshot assertion failed`)

Lorsqu'une modification UI altère la mise en page d'un écran :

```text
-old snapshot: │ [video] Movie.A.2024    4.2 GiB    2h00m ... │
+new results:  │ [NAS]   Movie.A.2024    4.2 GiB    2h00m ... │
```

- **Analyse** : `insta` affiche le diff visuel précis ligne par ligne entre la référence dorée et le rendu courant.
- **Validation** : Si le changement visuel est intentionnel et validé lors de la revue :
  - Mettre à jour les snapshots de référence via `INSTA_UPDATE=always cargo test --test test_scenarios` ou `cargo insta accept`.
  - Commiter les fichiers `.snap` mis à jour dans `crates/dbsrt-cli/tests/scenarios/snapshots/`.
- **Régression** : Si la modification est imprévue (décalage de bordure, retour à la ligne involontaire), corrigez le composant dans `ui.rs`.

### Cas 4 : Échec de parité de l'Aide (`test_scenario_contextual_help_calling_parity_and_glossary`)

- **Analyse** : Ce test garantit le respect de la consigne d'intégrité de `GEMINI.md`.
- **Diagnostic** : Un nouveau terme ou badge a été introduit dans les vues métier, mais la fonction correspondante dans `crates/dbsrt-cli/src/commands/top/ui.rs` (`build_help_*`) n'a pas documenté ce terme dans son glossaire. La PR doit mettre à jour l'aide avant d'être fusionnée.

---

## 5. Comment Écrire un Nouveau Scénario Déclaratif ?

Pour ajouter une nouvelle User Story dans `crates/dbsrt-cli/tests/test_scenarios.rs` :

### Étape 1 : Instancier le Harnais de Test

```rust
let tel = SystemTelemetry {
    discovered_media: vec![/* éléments de test */],
    ..Default::default()
};

// Utiliser une largeur standard 240 colonnes x 45 lignes
let mut harness = TestHarness::new(240, 45, tel);
```

### Étape 2 : Simuler les Actions Utilisateur

```rust
// Naviguer vers la vue Explorateur de Médias
harness.send_key(KeyCode::Char('4'));
assert_eq!(harness.app.view, ViewPage::MediaBrowser);

// Appliquer un filtre
harness.send_key(KeyCode::Char('f'));

// Sélectionner un élément avec Espace
harness.send_key(KeyCode::Char(' '));
```

### Étape 3 : Vérifier l'État Mémoire et le Rendu Visuel

```rust
// Vérifier la mutation de l'état applicatif
assert_eq!(harness.app.selected_media_paths.len(), 1);

// Vérifier que le buffer virtuel affiche bien la mise à jour à l'écran
harness.assert_buffer_contains("1 média(s) coché(s)");
```
