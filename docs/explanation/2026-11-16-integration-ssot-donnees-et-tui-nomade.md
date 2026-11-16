---
title: Intégration SSOT Données Multi-Réseaux, Mode Humain & TUI dbsrt top
tags:
  - explanation
  - data-access
  - tui
  - work-vpn
  - architecture
---

# 💡 Intégration SSOT Données Multi-Réseaux, Mode Humain (`--human`) & TUI `dbsrt top`

> **Journal d'ingénierie & notes de terrain (`#99` & `#100`)** — Retour sur l'ouverture de l'accès multi-réseaux (`scripts/data_access/`) aux opérateurs humains via tableaux `Rich` et le moniteur natif Rust `dbsrt top`, en conditions réelles hors LAN (`work_vpn`).

---

## 1. Import paresseux de `rich` : Préserver le contrat stdlib-only sous `python3` système

### Problématique & Mécanisme racine
Les scripts d'Agent Skill (`.agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py`) et le pont Rust (`data_access_bridge.rs`) invoquent l'interpréteur `python3` sans dépendre obligatoirement de `uv run`. Lorsqu'un import `from scripts.data_access.human_view import ...` (qui importe `rich`) a été placé au niveau module dans `scripts/data_access/cli.py`, tout appel JSON exécuté avec le `python3` système échouait immédiatement sur `ModuleNotFoundError: No module named 'rich'`.

### Conséquences concrètes
- **Empiriquement prouvé en test PTY `tmux`** : la 1re exécution du binaire `dbsrt top` échouait silencieusement lors des appels `status --compact` et `query --compact`, laissant l'onglet `[8/u]` sans topologie SSOT et l'onglet `[4/m]` vide (`0/0 DISPONIBLES`).
- Les appels d'agents IA exécutant `python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py ...` étaient exposés au même crash hors `.venv`.

### Solution / Invariant opéré
- **Import paresseux strict** dans `scripts/data_access/cli.py` : `scripts.data_access.human_view` (et donc `rich`) n'est importé que dans la branche `if getattr(args, "human", False):`.
- **Résolution d'interpréteur dans `data_access_bridge.rs`** : préfère `.venv/bin/python` lorsqu'il existe dans le repo, avec repli sur `python3`.

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : 100 % des commandes JSON (`status`, `resolve`, `query`, `inspect`, `fetch`, `--check`) restent strictement `stdlib-only` sous `python3` brut (`0` dépendance externe requise).
- **Limite résiduelle** : le drapeau `--human` (`task data:ui`) nécessite l'environnement Python du projet (`.venv` via `uv run` / `task data:*`).

---

## 2. Préservation du garde-fou CIFS & non-écrasement du catalogue distant dans `dbsrt top`

### Problématique & Mécanisme racine
1. Sur poste portable hors LAN (`work_vpn` avec tunnel `/16` ou hotspot `nomad_wan`), `data/reports/media_sources.json` contient souvent un chemin CIFS monté à domicile (`/media/$USER/Synology/tvshow`). Appeler `Path::exists()` sur ce chemin risque un blocage noyau SMB (`D-state`).
2. Par ailleurs, la boucle périodique `load_and_publish_remote_batch_state` de `sampler.rs` écrasait `state.discovered_media = remote_items` à chaque cycle : lorsque `selected_batch_targets.json` était vide (`[]`), elle effaçait les résultats chargés à la demande depuis `o2switch`.

### Conséquences concrètes
- **Empiriquement prouvé** : en `work_vpn`, `run_top_app` et `execute_sources_scan_cycle` devaient être protégés par `cached_cifs_safe_to_stat()`, et `load_and_publish_remote_batch_state` ne doit pas écraser les résultats distants par un vecteur vide.

### Solution / Invariant opéré
- `cached_cifs_safe_to_stat()` lit uniquement `$HOME/.cache/dbsrt/data_access_cache.json` (`< 1 ms`, zéro paquet réseau, zéro `stat()` CIFS) avant tout test `s.path.exists()` dans `mod.rs`, `app.rs` (`run_top_app`) et `sampler.rs`.
- Dans `sampler.rs`, `if !remote_items.is_empty() { state.discovered_media = remote_items; }` préserve les découvertes SSOT (`[w]` / `[/] + Entrée`) entre les cycles du scanner.

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : zéro blocage SMB en mode nomade/VPN tout en conservant la réactivité immédiate du TUI.
- **Limite résiduelle** : si `~/.cache/dbsrt/data_access_cache.json` n'existe pas encore au premier démarrage, le garde-fou repose sur la sonde asynchrone `probe_data_access_topology()` lancée en parallèle dans `[8/u]`.

---

## 3. Résilience `https_o2switch`, position des flags CLI & `inspect snippets` (`#99`)

### Problématique & Mécanisme racine
1. Dans `argparse`, partager `parents=[common]` sur le parser racine **et** chaque sous-parser avec des valeurs par défaut (`default=False`) fait que le sous-parser écrase `--force-refresh` lorsque celui-ci est placé *avant* la sous-commande (`dbsrt_data.py --force-refresh query ...`).
2. Un timeout transitoire sur `probe_o2switch_https` (`4.5 s` sans retry) persistait `accessible: false` en cache pendant `300 s` (et jusqu'à `24 h` en SWR), bloquant `freebox`/`video`/`tvshow` même quand `wan_ftp_synology` répondait en `~500 ms`.
3. `inspect_media_container` court-circuitait `_needs_path_autoresolve` pour `local-snippets`, rendant `inspect snippets "<titre>"` inopérant sur titre partiel.

### Conséquences concrètes
- **Empiriquement prouvé** sous `work_vpn` : le premier `status` avait caché `https_o2switch=false` (`URLError` à `4585.9 ms`), rendant `freebox:Matrix`, `video:Matrix` et `tvshow:Andor S01E01` indisponibles alors que le endpoint répondait en `107.6 ms`.

### Solution / Invariant opéré
- `_build_common_parser(suppress_defaults=True)` utilise `argparse.SUPPRESS` sur les sous-parsers (`cli.py`).
- `probe_o2switch_https` effectue jusqu'à `2` tentatives sur erreur réseau transitoire, et `_has_transient_https_failure` plafonne le cache d'un échec transitoire à `HTTPS_TRANSIENT_CACHE_TTL_SECONDS = 15.0 s`.
- Repli automatique de `freebox-disks` sur `freebox_batch_targets.json` et de `synology-*` sur `FTP.mlsd` (`wan_ftp`/`lan_ftp`).
- `_resolve_local_snippet_target` résout les titres partiels dans `data/snippets/`.

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : tolérance aux micro-coupures HTTPS à froid et cohérence 100 % avec le contrat `SKILL.md`.
- **Limite résiduelle** : le rebond `Freebox` sans montage local ni tunnel LAN reste dépendant de la joignabilité du relais HTTPS `o2switch` (ou du fallback `freebox_batch_targets.json`).

---

## 4. Binaire de session `task top` vs installation globale `~/.local/bin/dbsrt`

### Problématique & Mécanisme racine
Un binaire compilé antérieurement dans `~/.local/bin/dbsrt` (dans le `$PATH` utilisateur) reste figé tant qu'il n'est pas explicitement redéployé, alors que `task top` / `task media:top` exécutent `./target/release/dbsrt top` depuis le workspace actif.

### Conséquences concrètes
- Lancer `dbsrt top` directement dans un terminal peut exécuter une ancienne révision binaire sans les évolutions locales en cours de test.

### Solution / Invariant opéré
- Toujours utiliser **`task top`** (`taskfiles/stack.yml`) pour tester et exploiter les évolutions de `dbsrt top` sur la branche courante.

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : garantit la synchronisation entre le code Rust compilé (`cli:build`) et la session TUI lancée.
- **Limite résiduelle** : aucune (discipline d'exécution conforme à la règle `Taskfile-First`).
