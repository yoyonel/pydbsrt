## 🎯 Contexte & Objectif

<!-- Décrire le problème résolu, l'objectif architectural ou la fonctionnalité ajoutée. -->
<!-- Lier l'issue ou le projet GitHub associé (ex: Closes #14, Réf #16, Projet 3). -->

Closes #

---

## 🛡️ Changements & Périmètre Technique

<!-- Résumer de façon concise les modifications apportées par composant / module. -->

### 1. Composant A (ex: Rust / CLI / Server)
-

### 2. Composant B (ex: Base de Données / Migrations / Docker)
-

### 3. Orchestration & Taskfile
-

---

## 🧪 Validation Empirique & Preuves Terminales

<!-- Rappel de la règle : Zéro Spéculation & Preuve Terminale Obligatoire. -->
<!-- Coller la sortie terminale vérifiée (logs réels, métriques de temps ou RAM, etc.). -->

```text
# Commande exécutée et résultat réel
```

---

## 📚 Documentation & Synchronisation Diátaxis (https://diataxis.fr/)

- [ ] **🎓 Tutoriels** (`docs/tutorials/`) : Nouveau parcours d'apprentissage guidé (si nouveau sous-système)
- [ ] **🛠️ Guides Pratiques** (`docs/how-to/`) : Recette opérationnelle orientée tâche résolue
- [ ] **📖 Référence** (`docs/reference/`) : Commandes CLI (`cli.md`), API Axum (`api-rest.md`), schéma SQL ou formats mis à jour
- [ ] **💡 Explications** (`docs/explanation/`) : Choix d'architecture et compromis clarifiés
- [ ] **Navigation & Portail** : `mkdocs.yml` et `README.md` synchronisés avec les nouvelles cibles
- [ ] **Validation Qualité** : `task doc:check` exécuté et 100% vert (strict build, zéro fuite LaTeX, 0 chemin absolu)

---

## ✅ Checklist d'Assurance Qualité (DoD)

- [ ] **Linters** : `task lint` exécuté avec succès (100% vert, 9 linters)
- [ ] **Typage** : `task typecheck` validé sans erreur ni avertissement (Pyright)
- [ ] **Tests Rust** : `task test:rs` ou `cargo test` validé sans échec
- [ ] **Tests VLC / Python** : Tests unitaires et E2E exécutés avec succès
- [ ] **Zéro Régression** : Pipeline complet `task ci:local` réussi (<10s)
- [ ] **Zéro Constante Magique** : Constantes déclarées, typées et documentées
- [ ] **Zéro Chemin Absolu** : Aucun chemin local machine (`/home/...`) dans les fichiers versionnés
