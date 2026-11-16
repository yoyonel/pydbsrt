---
title: "Modularisation Portail Sources, Pièges monkeypatch & Isolation Zero-Pip o2switch"
tags:
  - explanation
  - portal
  - testing
  - o2switch
  - taskfile
  - diataxis
---

# 💡 Modularisation Portail Sources, Pièges `monkeypatch` & Isolation Zero-Pip `o2switch`

Ce journal d'ingénierie consigne les pièges concrets rencontrés et vérifiés durant le découpage du portail MediaSources (`3 768` LOC) vers `src/pydbsrt/portal/` (Issue #72 / `ADR-0009`).

---

## 1. Piège du Re-Export Shim face à `pytest.monkeypatch`

### Problématique & Mécanisme racine
Une façade rétrocompatible (`scripts/portal_server.py`) qui ré-exporte des fonctions et constantes (`from pydbsrt.portal import DEFAULT_GATEWAY_WAN_URL, _is_cifs_safe_to_stat`) crée de simples liaisons de noms dans son propre espace de noms module. Si un test applique `monkeypatch.setattr(ps, "DEFAULT_GATEWAY_WAN_URL", ...)` ou `monkeypatch.setattr(ps, "_is_cifs_safe_to_stat", ...)` sur le shim, les fonctions internes de `probe_service.py` ou `listing_service.py` continuent de lire leurs propres références locales et ignorent le patch.

### Conséquences concrètes
- **Hermétiquement Testé** : dès le découpage, les tests CIFS anti-hang et gateway catalogue (`tests/test_portal_server.py`) auraient conservé les valeurs réelles réseau/système au lieu des bouchons isolés.

### Solution / Invariant opéré
1. Les services lisent les attributs mutables via leur module propriétaire (`secrets_env._is_cifs_safe_to_stat()`, `portal_config.DEFAULT_GATEWAY_WAN_URL`, `portal_config.DEFAULT_GATEWAY_LAN_URL`).
2. `tests/test_portal_server.py` cible les sous-modules effectifs (`secrets_env`, `probe_service`, `portal_config`) tout en vérifiant l'identité des symboles ré-exportés par `scripts.portal_server` et `pydbsrt.portal`.

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : préserve une compatibilité 100 % sur les imports directs et évite toute fuite réseau en test.
- **Limites** : un nouveau test qui patcherait uniquement un attribut sur la façade `scripts.portal_server` ne détournerait pas les appels internes sans cibler le sous-module propriétaire.

---

## 2. Profondeur de Package, CWD de Test & `o2switch` Sans `.git`

### Problématique & Mécanisme racine
Dans `scripts/portal_server.py`, la racine du dépôt était déduite par `Path(__file__).resolve().parents[1]`. En déplaçant le code dans `src/pydbsrt/portal/services/*.py`, `.parents[1]` pointe vers `src/pydbsrt/`, cassant `data/__PHASH__`, `data/catalog.dbphash`, `data/SRT/couplings.json` et `etc/.nas_pass`. De plus, sur `o2switch`, l'arborescence déployée (`~/dbsrt/`) ne contient ni `.git` ni `Taskfile.yml`.

### Conséquences concrètes
- **Empiriquement Prouvé** : exécuter `python3 -m pydbsrt.portal --metadata` depuis `/tmp` ou via `deploy/o2switch/index.php` échoue par `ModuleNotFoundError` ou chemin introuvable si `REPO_ROOT` et `SRC_ROOT` ne sont pas résolus dynamiquement.

### Solution / Invariant opéré
- `find_repo_root()` dans `src/pydbsrt/portal/config.py` remonte `Path(__file__).resolve().parents` jusqu'à trouver `scripts/data_access` (présent localement ET sur `o2switch`), `Taskfile.yml` ou `.git` (fallback `.parents[3]`), et insère `REPO_ROOT` dans `sys.path`.
- `_resolve_data_path(rel_path)` préfère un chemin relatif existant dans le CWD courant (isolation `monkeypatch.chdir(tmp_path)` des tests), sinon retombe sur `REPO_ROOT / rel_path`.
- `scripts/infra/o2switch/o2switch_ctl.sh sync` synchronise désormais `src/pydbsrt/__init__.py` (`0` octet), `src/pydbsrt/portal/` et `src/pydbsrt/portal/static/sources.html` vers `~/dbsrt/web/sources.html`.

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : exécution agnostique au CWD et 100 % compatible Zero-Pip sur `o2switch`.
- **Limites** : la recette distante `o2switch_ctl.sh sync` + CGI live reste à exécuter lors du déploiement cible (`Gate B`).

---

## 3. Collision d'Alias Taskfile (`:8780` Python vs `:8765` Rust) & Daemon E2E

### Problématique & Mécanisme racine
Le projet expose deux portails distincts : le portail MediaSources Python (`:8780`, `/sources.html`) et le serveur Axum Rust `dbsrt-server` (`:8765`, `/monitoring`, `/studio`, `/top`). Or `Taskfile.yml` définissait `portal:e2e` (`:8780`) pendant que `taskfiles/quality.yml` déclarait `aliases: [portal:e2e]` sur `test:portal:e2e` (`:8765`). De plus, `portal:e2e` supposait un daemon `:8780` déjà actif dans un autre terminal (`ERR_CONNECTION_REFUSED` à froid).

### Conséquences concrètes
- **Empiriquement Prouvé** : lancer `task test:portal:e2e` échoue sur `:8765/monitoring` si `dbsrt-server` est arrêté, et `task portal:e2e` échouait à froid sans `task db:portal`.

### Solution / Invariant opéré
- Création de `scripts/e2e/run_sources_e2e.sh` : réutilise `:8780` s'il répond déjà sur `/api/sources`, sinon démarre temporairement `uv run python -m pydbsrt.portal --port 8780` et l'arrête sur `trap EXIT`.
- `Taskfile.yml` expose `task sources:e2e` *(alias `portal:e2e`)* et `task sources:check` (suite complète `pytest --cov-fail-under=80` + `sources:e2e` + `ruff` + `complexipy` + `pyright`).
- `taskfiles/quality.yml` renomme l'alias de `test:portal:e2e` en `server:e2e` (`[Rust :8765]`).

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : exécution déterministe en une commande depuis un shell neuf, zéro collision d'alias.
- **Limites** : sur `o2switch`, `task test:portal:e2e:cloud` externe reste soumis au WAF Tiger Protect (utiliser `task o2switch:e2e` via SSH).

---

## 4. Typage Strict (`Protocol`), Complexité (`<= 15`) & Couverture `>= 80 %`

### Problématique & Mécanisme racine
Le découpage initial utilisait `handler: Any` dans `routes/*.py`, un `# noqa: E402` dans `routes/__init__.py` (pour casser un import circulaire avec `parse_int_query_param`) et des retours `dict[str, Any]` non vérifiés depuis `cache.py` (`no-any-return` sous `mypy --strict`). Par ailleurs, les 17 tests initiaux ne couvraient que `64.73 %` de `src/pydbsrt/portal/` (zones froides `ftp_parsers.py` à `29.9 %` et `ffprobe_service.py` à `29.7 %`).

### Conséquences concrètes
- **Empiriquement Prouvé** :
  - Extraction de `PortalHandlerProtocol` et `parse_int_query_param` dans `src/pydbsrt/portal/routes/common.py` + garde `isinstance(cached_details, dict)` dans `ffprobe_service.py` → **`0` `# noqa`**, **`0` erreur `mypy --strict`**, **`0` erreur `pyright`**, **`0` fonction `> 15` sous `complexipy`**.
  - Ajout de `tests/test_portal_modules.py` (mocks FTP `MLSD`/`LIST`/`NLST`, endpoints `ffprobe`, fallback `psql` CLI, `POST /api/trajectory/run`, CLI & `__main__.py`) → **`89.47 %` en `pytest` seul** (`24/24` verts) et **`90.65 %` en combiné `pytest` + `task sources:e2e`**, avec **100 % des 22 modules `>= 82.1 %`**.

### Solution / Invariant opéré
- Toute nouvelle route HTTP accepte `PortalHandlerProtocol` défini dans `src/pydbsrt/portal/routes/common.py`.
- `task sources:check` applique en continu le seuil `--cov-fail-under=80` sur `src/pydbsrt/portal`.

### Avantages, Inconvénients & Limites résiduelles
- **Avantages** : détecte immédiatement toute régression de couverture, de typage ou de complexité cognitive avant merge.
- **Limites** : le fallback `psql` CLI et `ftp_parsers` sont validés avec des bouchons hermétiques en local ; leur exécution live réelle sur `o2switch` relève de la recette distante.
