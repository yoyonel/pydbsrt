---
title: "Tutoriel : Environnement de Dév & Contribution"
tags:

  - tutoriel
  - développeur
  - contribution
  - tests
  - qualité
---

# 🎓 Tutoriel : Environnement de Développement & Contribution

Dans ce tutoriel pas à pas, vous allez préparer votre poste de travail pour contribuer au code de `pydbsrt`, exécuter les suites de tests et linters automatisés, et valider vos modifications sans régression.

> [!NOTE]
> **Temps estimé** : 15 minutes.
> **Objectif** : Configurer la chaîne de compilation polyglotte (Rust, Python, Shell, Lua), exécuter la suite de tests unitaires et intégration, et faire passer le pipeline qualité local.

---

## 1. Prérequis Système

Installez les outils de base sur votre distribution Linux (ex: Debian / Ubuntu) :

```bash
# Outils système, compilateurs et dépendances multimédia
sudo apt-get update
sudo apt-get install -y git curl build-essential pkg-config libssl-dev ffmpeg vlc
```

Installez ensuite les trois piliers de l'outillage moderne du projet :

1. **Rust & Cargo** : via [rustup](https://rustup.rs/) (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).
2. **Python & uv** : gestionnaire de paquets ultra-rapide (`curl -LsSf https://astral.sh/uv/install.sh | sh`).
3. **Taskfile** : orchestrateur de commandes déclaratif (`sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b ~/.local/bin`).

---

## 2. Étape 1 : Initialiser le Dépôt et les Environnements Virtuels

Clonez le dépôt et synchronisez les dépendances Python via `uv` :

```bash
uv sync --all-groups
```

Activez les hooks de contrôle de qualité git (`pre-commit`) :

```bash
uv run pre-commit install
```

Désormais, tout commit exécutera localement les vérifications syntaxiques et typages stricts.

---

## 3. Étape 2 : Compiler la Suite Rust en Mode Debug & Release

Le cœur algorithmique de `pydbsrt` est écrit en Rust (espace de travail Cargo modulaire sous `crates/`).

Compilez l'ensemble des crates du workspace :

```bash
# Compilation rapide pour le développement
cargo build --workspace

# Compilation optimisée (SIMD AVX2/NEON) pour les benchmarks
task cli:build
```

Les exécutables générés se trouvent sous `target/debug/` et `target/release/` :

- `dbsrt` : CLI standalone (extraction pHash, certification, façade d'ingestion).
- `dbsrt-server` : Démon HTTP Axum et serveur Studio.

---

## 4. Étape 3 : Exécuter la Suite de Tests Unitaires et d'Intégration

Le projet applique une politique stricte de **zéro régression**.
Exécutez les tests Rust :

```bash
task test:rs
```

Exécutez les tests d'intégration Python et extensions VLC :

```bash
task test:unit
```

Pour lancer l'ensemble des vérifications locales de non-régression (tests + linters) en moins de 10 secondes :

```bash
task ci:local
```

---

## 5. Étape 4 : Lancer le Serveur de Documentation en Direct

Le portail documentaire utilise MkDocs Material avec rechargement à chaud automatique.
Pour explorer et modifier la documentation localement :

```bash
task doc
```

Ouvrez votre navigateur à l'adresse indiquée (par défaut `http://127.0.0.1:8000`).
Toute modification apportée à un fichier Markdown sous `docs/` est immédiatement répercutée dans votre navigateur.

Pour valider l'intégrité globale de la documentation (liens internes, rendu LaTeX MathJax, propreté du DOM HTML) :

```bash
task doc:check
```

---

## 6. Étape 5 : Votre Premier Cycle de Contribution

Pour proposer une modification au projet :

1. Créez une branche thématique depuis `develop` :
   ```bash
   git checkout -b feature/ma-nouvelle-fonctionnalite
   ```

2. Apportez vos modifications de code et mettez à jour la documentation selon le quadrant Diátaxis correspondant.
3. Vérifiez la conformité globale :
   ```bash
   task lint
   task typecheck
   task doc:check
   ```

4. Soumettez votre Pull Request en complétant scrupuleusement le gabarit défini dans `.github/pull_request_template.md`.

---

## 7. Félicitations !

Votre poste de travail est opérationnel et configuré selon les plus hauts standards d'ingénierie logicielle du projet.
Consultez le quadrant **[Guides Pratiques (How-To)](../how-to/)** pour découvrir les recettes avancées.
