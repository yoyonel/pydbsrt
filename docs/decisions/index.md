---
title: "Registre des Décisions d'Architecture (ADR)"
tags:
  - diataxis
  - architecture
  - adr
  - decisions
  - gouvernance
---

# 🏛️ Registre des Décisions d'Architecture (ADR)

Ce registre centralise l'ensemble des **Architecture Decision Records (ADR)** du projet `pydbsrt`. Il applique un protocole continu à **deux niveaux complémentaires** afin qu'aucun arbitrage structurant ni aucun piège bas-niveau découvert empiriquement ne se perde au fil du temps.

---

## 1. Architecture Documentaire Bi-Couche : ADR vs Journal d'Ingénierie

Tous les enseignements techniques d'une implémentation n'ont pas la même granularité ni la même durée de vie. `pydbsrt` sépare explicitement deux artefacts complémentaires :

| Artefact | Emplacement | Rôle & Question Résolue | Critères d'Éligibilité |
| :--- | :--- | :--- | :--- |
| **1. Décision d'Architecture (`ADR`)** | `docs/decisions/` | *« Quel contrat structurant avons-nous retenu face aux alternatives mesurées, et pourquoi ? »* | 1. **Coûteux à inverser** (format binaire, topologie réseau, schéma d'état)<br/>2. **Surprenant sans contexte** pour un relecteur futur<br/>3. **Fruit d'un vrai compromis** entre options explicites |
| **2. Journal d'Ingénierie (`Engineering Field Notes`)** | `docs/explanation/` | *« Quel mur physique (noyau OS, CPU, mathématique, concurrence) avons-nous heurté, et quel invariant interdit toute régression ? »* | Comportement non intuitif révélé par un échec de test, un profilage mémoire/latence ou un tir de benchmark réel |

!!! important "Traçabilité Bidirectionnelle avec le Code Source (`# Data Flow & Invariants`)"
    Lorsqu'un invariant de code protège un comportement subtil documenté dans un ADR ou un Journal d'Ingénierie (par exemple une garde anti-*underflow*, un ordre d'appels système `posix_fadvise` ou un encodage binaire de clé), la section Rustdoc `# Data Flow & Invariants` de la fonction mentionne explicitement `Invariant (ADR-XXXX): ...` afin d'empêcher toute « simplification » destructrice future.

---

## 2. Grille Standardisée des 4 Sous-Blocs d'Analyse

Chaque difficulté technique résolue ou arbitrage paramétrique consigné dans `docs/explanation/` (et synthétisé dans les PR) respecte systématiquement quatre sous-sections fixes :

1. **Problématique & Mécanisme racine** : symptôme exact observé et cause mécanique sous-jacente (noyau Linux, micro-architecture CPU, formule combinatoire ou propriété algébrique).
2. **Conséquences concrètes** : impact quantifié (distinguant explicitement **Empiriquement Prouvé** sur corpus/matériel réel vs **Hermétiquement Testé** en isolation) sur la précision, le rappel, le `VmRSS`, la latence ou l'herméticité.
3. **Solution / Invariant opéré** : mécanisme exact retenu, constantes nommées associées et emplacements relatifs dans l'arbre des sources.
4. **Avantages, Inconvénients & Limites résiduelles** : gains vérifiés, surcoûts acceptés et cas limites honnêtes restant hors couverture.

---

## 3. Index Chronologique des ADR

| ID | Date | Domaine | Décision d'Architecture | Statut | Étude / Journal Associé |
| :--- | :---: | :---: | :--- | :---: | :--- |
| **ADR-0001** | 2026-10-25 | `srt` | [Détection d'encodage des sous-titres à 3 niveaux (BOM, UTF-8, `chardetng`)](2026-10-25-subtitle-encoding-detection.md) | Accepté | — |
| **ADR-0002** | 2026-10-25 | `ingestion` | [Façade unifiée `task media:add` & résolution déterministe des 5 cas limites](2026-10-25-media-add-edge-cases.md) | Accepté | [Guide Façade `media:add`](../how-to/2026-10-25-guide-media-add-facade.md) |
| **ADR-0003** | 2026-10-30 | `infra` / `wan` | [Transport Freebox vers o2switch via NAT filtré & passerelle HTTP Range RPi4](2026-10-30-transport-freebox-nat-gateway-http.md) | Accepté | [Architecture Déport o2switch](../explanation/architecture-deport-calcul-stockage-et-synchronisation-o2switch.md) |
| **ADR-0004** | 2026-10-31 | `o2switch` | [Écoute `dbsrt-server` sur socket Unix (`0600` + `flock`) chez o2switch](2026-10-31-dbsrt-server-unix-socket-o2switch.md) | Accepté | [Enquête Kill Périodique o2switch](../explanation/2026-10-31-enquete-kill-periodique-dbsrt-server-o2switch.md) |
| **ADR-0005** | 2026-10-31 | `phash` / `cas` | [MovieHash OpenSubtitles 64-bit comme identifiant canonique des `.phash` & sidecars](2026-10-31-phash-identifiant-moviehash.md) | Accepté | [Identité Binaire & Anti-Shadowing](../explanation/2026-11-08-etude-architecture-identite-binaire-conteneurs-et-anti-shadowing.md) |
| **ADR-0006** | 2026-10-31 | `metrologie` | [Banc de mesure continu de la reconnaissance (`/v1/recognize`)](2026-10-31-banc-continu-reconnaissance.md) | Proposé | [Benchmark Multimodal 3 180 Vidéos](../explanation/2026-11-14-benchmark-exhaustif-multimodal-3180-videos-reelles-phash-ahash-c2f-mih.md) |
| **ADR-0007** | 2026-11-14 | `ahash` / `mmap` | [Archive binaire `mmap` `.dbahash` v1, ordonnancement thermique & moteur hybride `AhashEngine`](2026-11-14-dbahash-v1-mmap-and-hybrid-ahash-engine.md) | Accepté | [Ingénierie `.dbahash` v1, Folios Linux 6.6+ & C2F Zéro-Alloc](../explanation/2026-11-14-architecture-dbahash-mmap-folios-linux-et-c2f-zero-alloc.md) |
| **ADR-0008** | 2026-11-15 | `backup` / `storage` | [Persistance multi-tiers dédupliquée (`o2switch` + `Synology FTP`), manifestes `SHA-256` & éviction zéro-perte](2026-11-15-multi-tier-deduplicated-dataset-backups.md) | Accepté | [Hardlinks POSIX & Streaming PAX TAR Zéro-CIFS](../explanation/2026-11-15-sauvegarde-multi-tiers-hardlinks-posix-et-packs-ftp-zero-cifs.md) |
| **ADR-0009** | 2026-11-16 | `portal` / `soc` | [Modularisation du Portail Sources Médias vers `src/pydbsrt/portal/` & Façade Zero-Pip](2026-11-16-modularisation-portail-sources-pydbsrt-portal.md) | Accepté | [Modularisation Portail Sources, Pièges `monkeypatch` & Isolation Zero-Pip](../explanation/2026-11-16-modularisation-portail-sources-et-pieges-ci-o2switch.md) |
