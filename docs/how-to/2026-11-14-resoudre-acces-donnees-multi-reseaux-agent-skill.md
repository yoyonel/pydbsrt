---
title: "Comment résoudre instantanément les accès aux données multi-réseaux (SSOT & Agent Skill)"
tags:

  - howto
  - skills
  - data-access
  - ssot
  - network
---

# 🛠️ Comment résoudre instantanément les accès aux données multi-réseaux (`pydbsrt-data-access`)

Ce guide pratique explique comment utiliser le résolveur **Single Source of Truth (SSOT)** (`scripts/data_access/` et le skill `.agents/skills/pydbsrt-data-access/`) pour déterminer automatiquement, en moins de `2 ms` (cache chaud), où résident les sources médias et les catalogues internes de `pydbsrt` et par quel canal sûr y accéder selon votre contexte réseau actif.

---

## 1. Diagnostiquer la topologie réseau et la fraîcheur inter-nœuds

Pour inspecter le profil réseau actif (`home_lan`, `nomad_wan` 4G/5G, `work_vpn` avec tunnel `ppp0`, `rpi4`, `o2switch`), la sûreté des montages CIFS et la fraîcheur comparée des artefacts entre `local`, `rpi4` et `o2switch` :

```bash
task data:status
```

Pour forcer une nouvelle sonde parallèle bornée en ignorant le cache (`~/.cache/dbsrt/data_access_cache.json`, TTL 300 s invalidé automatiquement dès que `/proc/net/route` change) :

```bash
task data:status -- --force-refresh
```

> [!WARNING]
> Lorsque `network.cifs_safe_to_stat` vaut `false` (partage de connexion 4G/5G ou Wi-Fi d'entreprise avec VPN `ppp0` capturant le sous-réseau privé `/16`), n'exécutez **jamais** `ls`, `stat` ou `find` sur `$MEDIA_MOUNT_ROOT/$USER/Synology` ou `$MEDIA_MOUNT_ROOT/$USER/Freebox` : les partages CIFS restent inscrits dans `/proc/mounts` et bloquent les processus en attente noyau (`D-state`).

---

## 2. Résoudre le canal optimal en une seule passe groupée (`data:query` & `data:resolve`)

Pour inspecter plusieurs familles de données, la fraîcheur inter-nœuds et des filtres de recherche en **un seul appel dédupliqué** (réflexe recommandé aux agents IA), utilisez `task data:query` :

```bash
# Requête groupée : résout ahash, catalog, les 3 disques Freebox et les snippets filtrés en 1 passe
task data:query -- ahash catalog freebox snippets:Andor
```

Pour une famille isolée, utilisez `task data:resolve` avec l'identifiant canonique ou son alias court (`all`, `video`, `tvshow`, `freebox`, `snippets`, `phash`, `ahash`, `catalog`, `couplings`, `db`, `reports`, `fixtures`) :

```bash
# Vue synthétique des 12 familles de données et des 3 disques Freebox
task data:resolve -- all

# Détail des 3 volumes Freebox (Disque_1, Disque_2, Disque_1_1) et passerelles HTTP
task data:resolve -- freebox

# Recherche d'extraits locaux + vérification automatique du couplage parent dans couplings.json
task data:resolve -- snippets Andor
```

---

## 3. Rapatrier un catalogue ou un corpus en retard (`STALE_LOCAL`)

Lorsque `task data:status` signale `STALE_LOCAL` sur `catalog.dbphash` ou `PARTIAL_LOCAL` sur `ahash-files` / `phash-files`, prévisualisez puis exécutez le transfert depuis le nœud autoritaire joignable (`rpi4` en LAN domestique, `o2switch` pour `__AHASH__/`) :

```bash
# Prévisualisation sans transfert (--dry-run)
task data:fetch -- catalog --dry-run

# Synchronisation effective
task data:fetch -- catalog
```

> [!NOTE]
> En mobilité 4G/5G CGNAT, le pare-feu IP SSH (`:22`) d'`o2switch` filtre les adresses mobiles non inscrites en liste blanche. Le résolveur bascule alors automatiquement les vérifications et requêtes vers l'API authentifiée `HTTPS :443` (`O2SWITCH_URL` + `X-API-Key`) et le flux `WAN FTP` passif du NAS.

---

## 4. Initialiser les symlinks `data/` d'un `git worktree` isolé

Dans un `git worktree` fraîchement créé, `scripts/data_access/` résout automatiquement le dossier `data/` et le `.env` du dépôt parent via `git rev-parse --git-common-dir`. Avant de lancer des démons locaux ou des benchmarks dans ce worktree, créez les liens symboliques attendus :

```bash
task data:seed-worktree -- --dry-run
task data:seed-worktree
```

---

## 5. Valider, Profiler et Étendre les Agent Skills du Workspace

Pour ajouter un nouveau support ou gisement de données, déclarez-le dans `scripts/data_access/sources_registry.json`, puis exécutez la suite de validation statique, d'évaluation déterministe et de profilage temporel :

```bash
task skills:list
task skills:validate
task skills:test

# Noter automatiquement des transcriptions JSONL de benchmark (16 assertions, zéro LLM)
task skills:eval-benchmark -- <transcript_full.jsonl>

# Profiler seconde par seconde (temps outils vs temps IA) et détecter les goulets d'étranglement
task skills:profile -- --summary-only <transcript_full.jsonl>
```

Pour comprendre le protocole complet d'évaluation A/B à l'aveugle, les pièges de performance découverts (`10 Ko`, quotas de caractères, rafales parallèles) et la stratégie de sélection des modèles (`pro` vs `flash` / `flash_lite`), consultez [Méthodologie d'Ingénierie, Profiling et Optimisation des Agent Skills](../explanation/2026-11-14-methodologie-ingenierie-profiling-et-optimisation-agent-skills.md).
