---
title: Portail d'Ingénierie dbsrt — pHash Subtitle Sync & Studio
hide:
  - navigation
  - toc
---

<div class="dbsrt-hero">
  <div class="dbsrt-hero-top">
    <span class="dbsrt-pill dbsrt-pill-cyan">⚡ Moteur v2 Rust Zero-DB + PostgreSQL 17 SP-GiST</span>
    <span class="dbsrt-pill dbsrt-pill-emerald">🟢 CI/CD 82.8% Rust Coverage • 17/17 Pre-commit Hooks</span>
  </div>
  <h1>Portail d'Ingénierie <code>dbsrt</code> &amp; <code>pydbsrt</code></h1>
  <p class="dbsrt-hero-sub">
    Synchronisation automatique de sous-titres par empreintes visuelles <strong>pHash 64-bit</strong>, backoffice relationnel <strong>dbsrt Studio</strong> et extensions natives <strong>VLC / VLSub</strong>.
  </p>

  <div class="dbsrt-hero-actions" id="dbsrt-hero-actions">
    <a href="quickstart-and-reference/" class="dbsrt-btn dbsrt-btn-primary">⚡ Guide Opérationnel &amp; CLI/API</a>
    <a href="2026-10-18-vlsub-fork-and-restoration/" class="dbsrt-btn dbsrt-btn-secondary">🎬 Plugin VLC &amp; Démo 1080p</a>
    <a href="2026-10-18-studio-backoffice-and-relational-couplings/" class="dbsrt-btn dbsrt-btn-secondary">🎛️ Architecture Studio &amp; Couplages</a>
    <a href="2026-10-15-rust-e2e-benchmarks-and-tradeoffs/" class="dbsrt-btn dbsrt-btn-secondary">📊 Benchmarks Rust vs Python</a>
    <span id="dbsrt-local-services-slot" style="display:inline-flex;gap:0.65rem;flex-wrap:wrap;"></span>
  </div>

  <div class="dbsrt-kpi-grid">
    <div class="dbsrt-kpi-card">
      <div class="dbsrt-kpi-val">72 000 <small>fps</small></div>
      <div class="dbsrt-kpi-label">Débit pHash SIMD Rust (+1200× vs Python)</div>
    </div>
    <div class="dbsrt-kpi-card">
      <div class="dbsrt-kpi-val">&lt; 3 <small>ms</small></div>
      <div class="dbsrt-kpi-label">Recherche Zero-DB <code>mmap</code> &amp; Injection OSD</div>
    </div>
    <div class="dbsrt-kpi-card">
      <div class="dbsrt-kpi-val">0.0% <small>échec</small></div>
      <div class="dbsrt-kpi-label">Format <code>.dbphash</code> v2 Hybride (Cuts + 1 fps)</div>
    </div>
    <div class="dbsrt-kpi-card">
      <div class="dbsrt-kpi-val">21 <small>médias</small></div>
      <div class="dbsrt-kpi-label">Couplages Certifiés (PG17 + <code>couplings.json</code>)</div>
    </div>
  </div>
</div>

## 🧭 Les 4 Quadrants Documentaires Diátaxis

<div class="dbsrt-bento-grid">
  <div class="dbsrt-bento-card bento-cyan">
    <div class="bento-header">
      <span class="bento-icon">🎓</span>
      <span class="bento-badge">TUTORIELS • APPRENTISSAGE</span>
    </div>
    <h3>Parcours Débutants &amp; Prise en Main Pas à Pas</h3>
    <p>Leçons guidées linéaires, sans alternative ni bifurcation, garantissant une réussite immédiate pour découvrir la stack et le recalage.</p>
    <div class="bento-links">
      <a href="tutorials/01-premiers-pas-synchronisation-vlc/">→ 1. Premiers Pas &amp; Synchronisation VLC</a>
      <a href="tutorials/02-ingestion-et-certification-premier-media/">→ 2. Ingestion &amp; Certification Média</a>
      <a href="tutorials/03-environnement-developpement-et-contribution/">→ 3. Environnement Dév &amp; Contribution</a>
      <a href="tutorials/04-deploiement-appliance-rpi4/">→ 4. Déploiement Appliance RPi4</a>
      <a href="tutorials/index.md">→ Explorer l'index des Tutoriels</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-indigo">
    <div class="bento-header">
      <span class="bento-icon">🛠️</span>
      <span class="bento-badge">GUIDES PRATIQUES • HOW-TO</span>
    </div>
    <h3>Recettes Opérationnelles &amp; Résolution de Tâches</h3>
    <p>Guides directs orientés résolution de problème pour les utilisateurs et développeurs autonomes en situation réelle.</p>
    <div class="bento-links">
      <a href="how-to/synchroniser-sous-titres-vlc/">→ Synchroniser des Sous-Titres dans VLC</a>
      <a href="how-to/ajouter-et-indexer-un-media/">→ Façade Unifiée (task media:add)</a>
      <a href="how-to/certifier-un-couplage/">→ Certifier un Couplage Vérité Terrain</a>
      <a href="how-to/migrer-schema-postgresql/">→ Migrer PostgreSQL avec Refinery</a>
      <a href="how-to/surveiller-stack-telemetrie/">→ Surveiller avec Prometheus &amp; Grafana</a>
      <a href="how-to/contribuer-documentation-diataxis/">→ ✍️ Contribuer à la Doc selon Diátaxis</a>
      <a href="how-to/index.md">→ Explorer tous les Guides Pratiques</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-emerald">
    <div class="bento-header">
      <span class="bento-icon">📖</span>
      <span class="bento-badge">RÉFÉRENCE • INFORMATION</span>
    </div>
    <h3>Spécifications Exhaustives &amp; Descriptions Neutres</h3>
    <p>Description factuelle et austère des machines : options CLI, contrats d'API HTTP, schémas relationnels et formats binaires.</p>
    <div class="bento-links">
      <a href="reference/cli/">→ CLI Standalone Rust (dbsrt)</a>
      <a href="reference/api-rest/">→ API REST Axum (:8765)</a>
      <a href="reference/schema-postgresql/">→ Schéma PostgreSQL 17 &amp; SP-GiST</a>
      <a href="reference/formats-fichiers/">→ Formats .dbphash v2 &amp; Couplings JSON</a>
      <a href="reference/taskfile/">→ Catalogue des Commandes Taskfile</a>
      <a href="reference/configuration/">→ Configuration &amp; Variables d'Environnement</a>
      <a href="reference/index.md">→ Explorer l'index de Référence</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-amber">
    <div class="bento-header">
      <span class="bento-icon">💡</span>
      <span class="bento-badge">EXPLICATIONS • COMPRÉHENSION</span>
    </div>
    <h3>Architecture, Théorie &amp; Analyse des Choix</h3>
    <p>Articles discursifs apportant du recul, éclairant les modèles mathématiques et déconstruisant les fausses intuitions.</p>
    <div class="bento-links">
      <a href="explanation/principe-fondamental-phash-et-retargeting/">→ Pourquoi le pHash Requièrt 2 Flux Vidéo</a>
      <a href="explanation/echantillonnage-hybride-cuts-et-1fps/">→ Échantillonnage Hybride (Cuts + 1 fps)</a>
      <a href="explanation/indexation-metrique-mih-vs-bktree/">→ Indexation Métrique : MIH vs BK-Tree</a>
      <a href="explanation/modele-relationnel-couplages-certifies/">→ Vérité Terrain &amp; Couplages Certifiés</a>
      <a href="explanation/architecture-daemon-axum-et-vlc-osd/">→ Architecture Démon Axum &amp; VLC OSD</a>
      <a href="explanation/stockage-decentralise-cas-et-distribution/">→ Stockage Décentralisé CAS &amp; Distribution</a>
      <a href="explanation/index.md">→ Explorer l'index des Explications</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-rose">
    <div class="bento-header">
      <span class="bento-icon">📊</span>
      <span class="bento-badge">PREUVES EMPIRIQUES</span>
    </div>
    <h3>Benchmarks E2E &amp; Dataset Réel (5M Frames)</h3>
    <p>Campagnes de mesures sur 125 épisodes (5,08M frames), comparatifs Rust vs Python et validation E2E sur NAS Synology.</p>
    <div class="bento-links">
      <a href="2026-10-20-dashboard-interactif-e2e-vlc-corpus-nas/">→ ⚡ Dashboard E2E VLC GUI (1 336 Tests)</a>
      <a href="2026-10-15-rust-e2e-benchmarks-and-tradeoffs/">→ Benchmarks E2E Rust vs Python</a>
      <a href="2026-10-16-phash-dataset-scale-and-benchmarks/">→ Passage à l'Échelle (5,08M frames)</a>
      <a href="2026-10-19-mih-100m-scale-benchmark-report/">→ Stress-Test 100M Frames (MIH)</a>
    </div>
  </div>

  <div class="dbsrt-bento-card bento-purple">
    <div class="bento-header">
      <span class="bento-icon">🗺️</span>
      <span class="bento-badge">VISION &amp; ARCHIVES</span>
    </div>
    <h3>Feuille de Route (1920–2026) &amp; Audits</h3>
    <p>Régression affine 2D (drift PAL/NTSC), décodage matériel GPU, audits techniques de l'existant et index des tags.</p>
    <div class="bento-links">
      <a href="diataxis/">→ 🧭 Cadre Méthodologique Diátaxis</a>
      <a href="2026-10-21-feuille-de-route-post-merge-develop-et-decoupage-projets/">→ Feuilles de Route (5 Projets v0.2+)</a>
      <a href="2026-10-18-vlsub-legacy-audit/">→ Audit Technique Legacy VLSub</a>
      <a href="tags/">→ Explorer l'Index des Tags</a>
    </div>
  </div>
</div>

---

## 📑 Catalogue Interactif des Documents

<div class="dbsrt-catalog-toolbar">
  <div class="dbsrt-filter-pills" id="dbsrt-filter-pills">
    <button class="dbsrt-filter-btn active" data-filter="all">Tous (18)</button>
    <button class="dbsrt-filter-btn" data-filter="essential">⭐ Essentiels</button>
    <button class="dbsrt-filter-btn" data-filter="vlc">🎬 VLC &amp; VLSub</button>
    <button class="dbsrt-filter-btn" data-filter="studio">🎛️ Studio &amp; Couplages</button>
    <button class="dbsrt-filter-btn" data-filter="phash">⚡ pHash &amp; DB</button>
    <button class="dbsrt-filter-btn" data-filter="benchmarks">📊 Benchmarks</button>
    <button class="dbsrt-filter-btn" data-filter="archive">🗄️ Audits &amp; Archives</button>
  </div>
  <div class="dbsrt-search-box">
    <input type="text" id="dbsrt-catalog-search" placeholder="🔍 Filtrer instantanément par titre, mot-clé ou tag (ex: pgweb, mmap, vlsub)..." />
  </div>
</div>

| Document | Date | Pôle | Statut | Tags Clés | Résumé Synthétique |
| :--- | :---: | :---: | :---: | :--- | :--- |
| [⚡ Dashboard Interactif E2E VLC (1 334 Tests)](2026-10-20-dashboard-interactif-e2e-vlc-corpus-nas.md) | `2026-10-20` | **Benchmarks** | :octicons-star-fill-16: **ESSENTIEL** | `dashboard`, `e2e`, `vlc`, `benchmarks`, `essential` | Dashboard interactif haute performance sur 1 334 vidéos réelles (99.8% succès) avec captures 720p WebP, navigation clavier flèches et synchronisation curseur. |
| [Architecture de Retrait & Invalidation Globale](2026-10-20-architecture-retrait-invalidation-donnees-globales.md) | `2026-10-20` | **Studio** | :octicons-check-circle-fill-16: **CURRENT** | `purge`, `invalidation`, `postgres`, `zero-db`, `studio` | Procédure d'invalidation et retrait de médias corrompus sur 5 paliers synchronisés (FS, Zero-DB JSON, PostgreSQL 17 ON DELETE CASCADE, catalog.dbphash). |
| [⚡ Indexation Métrique : cBKTree Plat vs MIH](2026-10-19-indexing-metric-trees-and-multi-index-hashing.md) | `2026-10-19` | **pHash** | :octicons-star-fill-16: **ESSENTIEL** | `phash`, `bktree`, `cbktree`, `mih`, `indexing`, `scaling`, `essential` | Limite d'échelle du scan linéaire brut $O(N)$, conception du cBKTree plat zéro-copie et spécification du Multi-Index Hashing (MIH) sous les 5 ms. |
| [Audit Exhaustif Matching E2E & Preuves Empiriques](2026-10-19-audit-exhaustif-matching-e2e-phash.md) | `2026-10-19` | **Benchmarks** | :octicons-check-circle-fill-16: **CURRENT** | `benchmarks`, `phash`, `e2e`, `audit`, `dataset` | Campagne empirique 100% catalogue (233 empreintes, 13M frames), 57 flux réels YouTube, 96.6% Top-1 et analyse des limites. |
| [⚡ Guide Opérationnel & Référence CLI / API](quickstart-and-reference.md) | `2026-10-18` | **Guide** | :octicons-star-fill-16: **ESSENTIEL** | `taskfile`, `studio`, `vlc`, `api`, `essential` | Commandes `task` par pôle, URLs locales (`:8085`, `:8086`, `:8000`), référence des endpoints REST Axum et workflow de démarrage en 60s. |
| [dbsrt Studio, Couplages Relationnels & Qualité Code](2026-10-18-studio-backoffice-and-relational-couplings.md) | `2026-10-18` | **Studio** | :octicons-check-circle-fill-16: **CURRENT** | `studio`, `postgres`, `phash`, `quality`, `essential` | Modèle relationnel (`medias`/`frames`/`subtitles`/`sub_frames`), backoffice `dbsrt Studio` (`/studio`) + `PgWeb` (`:8086`), timeline bi-piste et `17/17` hooks. |
| [Mécanique E2E Search by pHash & Couplages Certifiés](2026-10-18-e2e-phash-search-mechanics-and-roadmap.md) | `2026-10-18` | **Studio** | :octicons-check-circle-fill-16: **CURRENT** | `vlc`, `vlsub`, `phash`, `studio`, `essential` | Chaîne E2E `Search by pHash` en 4 maillons, résolution prioritaire par `CouplingRegistry` / PostgreSQL et certification via `dbsrt certify-coupling`. |
| [Fork & Modernisation VLSub : Restauration UX & Démo 1080p](2026-10-18-vlsub-fork-and-restoration.md) | `2026-10-18` | **VLC** | :octicons-check-circle-fill-16: **CURRENT** | `vlsub`, `fork`, `vlc`, `mp4`, `essential` | Fork bonifié de VLSub : IHM 1:1 préservée, backend REST Axum `/v1/download`, preuve vidéo HD 1080p MP4 et captures étape par étape. |
| [Audit Technique : Extension Legacy VLSub](2026-10-18-vlsub-legacy-audit.md) | `2026-10-18` | **VLC** | :octicons-check-circle-fill-16: **CURRENT** | `vlsub`, `legacy`, `lua`, `vlc`, `archive` | Autopsie objective du plugin historique de VLC : forces ergonomiques (grille Qt, recherche titre), faiblesses techniques (XML-RPC mort, zéro resync). |
| [Validation UI/UX : Plugin VLC v2.0 & OSD](2026-10-17-validation-ui-ux-vlc-plugin-v2.md) | `2026-10-17` | **VLC** | :octicons-check-circle-fill-16: **CURRENT** | `vlc`, `ui-ux`, `osd`, `benchmarks` | Validation empirique avec captures d'écran et démo animée : interface Qt v2.0, exploration multi-sources `/v1/search` et notification OSD native. |
| [Étude Comparative : Frames, Cuts, Keyframes & Hybride](2026-10-17-etude-comparative-identification-frames-cuts-keyframes-hybride.md) | `2026-10-17` | **pHash** | :octicons-check-circle-fill-16: **CURRENT** | `dbphash`, `hybrid`, `cuts`, `phash`, `essential` | Étude sur 5,08M frames (125 épisodes) & 5 encodages : écueil I-frames (27% GOP VOD), format `.dbphash` v2.0 Hybride (cuts + 1 fps, 0% échec). |
| [Validation Empirique E2E (Synology NAS)](2026-10-17-validation-empirique-e2e-synology.md) | `2026-10-17` | **Benchmarks** | :octicons-check-circle-fill-16: **CURRENT** | `benchmarks`, `synology`, `e2e`, `phash` | Campagne de validation réelle sur corpus NAS Synology multi-encodages et vérification de résilience. |
| [Spécification Axe 5 : Providers Distants](2026-10-16-axe5-remote-subtitles-providers.md) | `2026-10-16` | **Studio** | :octicons-check-circle-fill-16: **CURRENT** | `providers`, `opensubtitles`, `vlc`, `studio` | Architecture des connecteurs distants REST (OpenSubtitles v1), scoring heuristique, cache disque et fallback autonome. |
| [Dataset Réel à l'Échelle (5,08M frames)](2026-10-16-phash-dataset-scale-and-benchmarks.md) | `2026-10-16` | **Benchmarks** | :octicons-check-circle-fill-16: **CURRENT** | `benchmarks`, `phash`, `scaling`, `dataset` | Analyse statistique de distribution de Hamming et performances d'indexation sur 125 épisodes complets (5,08 millions de frames). |
| [Feuille de Route & Échelle (1920-2026)](2026-10-15-deep-dive-modernization-and-scaling-roadmap.md) | `2026-10-15` | **Roadmap** | :octicons-rocket-16: **ROADMAP** | `roadmap`, `scaling`, `hwaccel`, `ransac` | Étude approfondie des 4 axes : décodage HW sub-seconde, régression 2D drift PAL/NTSC, index 1M vidéos (13 Go) et extraction MKV. |
| [Intégration VLC & Daemon Axum](2026-10-15-vlc-integration-daemon-and-plugins.md) | `2026-10-15` | **VLC** | :octicons-check-circle-fill-16: **CURRENT** | `vlc`, `axum`, `daemon`, `lua`, `osd` | Daemon REST Axum, découverte dynamique `/tmp/dbsrt-server.json`, plugins Lua GUI & CLI (`cvlc`), calage et injection OSD < 3 ms. |
| [Moteur Zero-DB mmap SIMD](2026-10-15-scaling-architecture-rust-mmap.md) | `2026-10-15` | **pHash** | :octicons-check-circle-fill-16: **CURRENT** | `mmap`, `zero-db`, `rust`, `simd`, `phash` | Pourquoi Python a échoué face à l'échelle (PyObject/GIL/GC). Moteur Rust mmap zero-copy (8 Go pour 1 milliard de frames). |
| [Benchmarks E2E Rust vs Python](2026-10-15-rust-e2e-benchmarks-and-tradeoffs.md) | `2026-10-15` | **Benchmarks** | :octicons-check-circle-fill-16: **CURRENT** | `benchmarks`, `perf`, `rust`, `simd` | Mesures réelles : pHash 72k fps (+1200×), `recognize-media` (+15×), `retarget` SRT en 0.5 ms. Analyse des trade-offs. |
| [Migration PG17 & BK-Tree vs pgvector](2026-10-15-pg17-bktree-benchmark-migration.md) | `2026-10-15` | **pHash** | :octicons-check-circle-fill-16: **CURRENT** | `postgres`, `pg17`, `bktree`, `phash`, `benchmarks` | Validation de l'extension C SP-GiST BK-Tree sur PostgreSQL 17. Comparatif de latence avec `pgvector` HNSW. |
| [Modernization Blueprint (UV / Taskfile)](2026-10-15-modernization-blueprint.md) | `2026-10-15` | **Guide** | :octicons-check-circle-fill-16: **CURRENT** | `uv`, `taskfile`, `hatchling`, `ruff` | Plan directeur de bascule : suppression Poetry/Make au profit de UV, Taskfile déclaratif et workspace Rust. |
| [Audit Global Initial du Dépôt](2026-10-15-audit-global.md) | `2026-10-15` | **Archive** | :octicons-archive-16: **ARCHIVE** | `audit`, `legacy`, `python`, `archive` | Inventaire complet initial du dépôt legacy (2020-2023), dette technique, failles et dépendances. |
| [Notes 2021 Fast Hamming Postgres](history/2021-04-13-fast-hamming-postgres.md) | `2021-04-13` | **Archive** | :octicons-archive-16: **ARCHIVE** | `history`, `cython`, `spgist`, `postgres`, `archive` | Notes de recherche initiales sur l'indexation de distance de Hamming dans Postgres (SP-GiST et Cython cBKTree). |
