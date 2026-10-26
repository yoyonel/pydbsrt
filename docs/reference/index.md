---
title: Quadrant Référence Technique (Information)
tags:

  - diataxis
  - reference
  - cli
  - api
  - schema
---

# 📖 Quadrant Référence : Spécifications Techniques

> Bienvenue dans la section **Référence Technique** de `pydbsrt`.
> Selon la méthodologie **[Diátaxis](https://diataxis.fr/)**, ce quadrant est strictement **orienté vers l'information factuelle et la description de la machine**.
> Le style y est neutre, rigoureux, précis et exhaustif. Vous n'y trouverez ni récits d'initiation ni recettes de résolution, mais la description exacte de chaque commande, route API, schéma SQL et format binaire.

---

## 📸 Aperçu du Quadrant Référence

<figure markdown="1">
  <img src="../assets/screenshots/portal_reference_1080p.webp" alt="Référence Technique dbsrt" width="100%">
  <figcaption>Figure 1 : Interface du portail de référence technique et spécifications formelles.</figcaption>
</figure>

---

## 📑 Sommaire des Spécifications

<div class="dbsrt-bento-grid">
  <div class="dbsrt-bento-card bento-cyan">
    <div class="bento-header">
      <span class="bento-icon">💻</span>
      <span class="bento-badge">LIGNE DE COMMANDE</span>
    </div>
    <h3>CLI Standalone Rust (`dbsrt`)</h3>
    <p>Catalogue exhaustif des sous-commandes, arguments, drapeaux (flags), variables d'environnement associées et codes de sortie.</p>
    <div class="bento-links">
      <a href="cli/">→ Consulter la référence CLI</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-indigo">
    <div class="bento-header">
      <span class="bento-icon">🌐</span>
      <span class="bento-badge">MICROSERVICE AXUM</span>
    </div>
    <h3>API REST (`dbsrt-server`)</h3>
    <p>Spécification des endpoints HTTP (`/v1/*`), contrats JSON des requêtes et réponses, statuts HTTP et gestion des erreurs.</p>
    <div class="bento-links">
      <a href="api-rest/">→ Consulter la référence API</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-emerald">
    <div class="bento-header">
      <span class="bento-icon">🐘</span>
      <span class="bento-badge">POSTGRESQL 17</span>
    </div>
    <h3>Schéma de Base de Données</h3>
    <p>Tables relationnelles (`medias`, `frames`, `subtitles`, `sub_frames`), index SP-GiST Hamming, clés étrangères et contraintes.</p>
    <div class="bento-links">
      <a href="schema-postgresql/">→ Consulter le schéma SQL</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-amber">
    <div class="bento-header">
      <span class="bento-icon">📦</span>
      <span class="bento-badge">FORMATS DE DONNÉES</span>
    </div>
    <h3>Formats de Fichiers & Spécifications</h3>
    <p>Structure binaire des fichiers `.dbphash` v2 (en-tête magic bytes, empreintes 64-bit), schéma JSON de `couplings.json` et normalisation SRT.</p>
    <div class="bento-links">
      <a href="formats-fichiers/">→ Consulter les formats</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-rose">
    <div class="bento-header">
      <span class="bento-icon">⚙️</span>
      <span class="bento-badge">ORCHESTRATION</span>
    </div>
    <h3>Catalogue Taskfile (`Taskfile.yml`)</h3>
    <p>Dictionnaire complet de toutes les cibles du projet réparties par namespace (`dev`, `db:*`, `server:*`, `media:*`, `test:*`, `doc:*`).</p>
    <div class="bento-links">
      <a href="taskfile/">→ Consulter le dictionnaire Taskfile</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-purple">
    <div class="bento-header">
      <span class="bento-icon">🔧</span>
      <span class="bento-badge">SYSTÈME & PORTS</span>
    </div>
    <h3>Configuration & Variables d'Environnement</h3>
    <p>Matrice des variables d'environnement (`.env`), allocation des ports réseau (`8765`, `5433`, `8086`, etc.) et constantes système.</p>
    <div class="bento-links">
      <a href="configuration/">→ Consulter la configuration</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-cyan">
    <div class="bento-header">
      <span class="bento-icon">📐</span>
      <span class="bento-badge">ZERO MAGIC CONSTANTS</span>
    </div>
    <h3>Constantes Nommées & Seuils Algorithmiques</h3>
    <p>Répertoire centralisé de toutes les constantes typées du codebase (Rust & Python) : seuils pHash, facteurs temporels et timeouts réseau.</p>
    <div class="bento-links">
      <a href="constants/">→ Consulter les constantes</a>
    </div>
  </div>
</div>
