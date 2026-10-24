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

## 📚 Documentation & Synchronisation

- [ ] Rapport technique horodaté ajouté sous `docs/YYYY-MM-DD-*.md` (si évolution d'architecture)
- [ ] Documentation Diátaxis mise à jour dans `mkdocs.yml`
- [ ] Tableaux des commandes Taskfile et architecture synchronisés dans `README.md`

---

## ✅ Checklist d'Assurance Qualité (DoD)

- [ ] **Linters** : `task lint` exécuté avec succès (100% vert, 9 linters)
- [ ] **Typage** : `task typecheck` validé sans erreur ni avertissement (Pyright)
- [ ] **Tests Rust** : `task test:rs` ou `cargo test` validé sans échec
- [ ] **Tests VLC / Python** : Tests unitaires et E2E exécutés avec succès
- [ ] **Zéro Régression** : Pipeline complet `task ci:local` réussi (<10s)
- [ ] **Zéro Constante Magique** : Constantes déclarées, typées et documentées
- [ ] **Zéro Chemin Absolu** : Aucun chemin local machine (`/home/...`) dans les fichiers versionnés
