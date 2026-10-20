# Plugin VLC `vlsub_dbsrt` (Studio Edition v2.0)

> Extension Lua officielle pour VLC Media Player : fork moderne de VLSub avec recherche multi-sources, synchronisation visuelle d'après empreintes perceptuelles (pHash SIMD DCT-II), certification relationnelle (PostgreSQL 17 / Zero-DB) et interface console headless.

## 1. Démonstration Visuelle

![VLSub dbsrt Studio Edition](../docs/assets/vlsub-dbsrt/02_vlsub_search_results.png)

Vidéo de démonstration HD 1080p disponible dans `docs/assets/vlsub-dbsrt/vlsub_dbsrt_workflow_1080p.mp4`.

---

## 2. Architecture & Composants

* **`vlsub_dbsrt.lua` (Extension IHM Qt)** :
  * Déployé dans `~/.local/share/vlc/lua/extensions/`.
  * Menu VLC : **`Vue`** > **`VLSub (dbsrt Enhanced)`**.
  * Restauration de l'ergonomie VLSub 1:1 : sélection de langue, titre, saison, épisode.
  * 4 modes de recherche : `Smart Search (All)`, `Search by pHash`, `Search by Hash`, `Search by Name`.
  * Téléchargement, décalage temporel automatique pHash (`DCT-II SIMD`) et injection immédiate `vlc.input.add_subtitle`.
  * Bouton **`Certify Coupling`** : enregistre le couplage certifié dans PostgreSQL 17 et `data/SRT/couplings.json`.
* **`dbsrt_intf.lua` (Interface Headless CLI)** :
  * Déployé dans `~/.local/share/vlc/lua/intf/`.
  * Utilisé par `cvlc` pour les tests CI/CD déterministes (`--lua-intf dbsrt_intf`).

---

## 3. Guide d'Installation & Utilisation

```bash
# 1. Déployer l'extension et l'interface CLI dans ~/.local/share/vlc/lua/
task vlc:install-plugin

# 2. Démarrer le démon local Rust Axum
task server:daemon

# 3. Vérifier le statut du démon
task server:status

# 4. Exécuter le test d'intégration E2E cvlc
task vlc:test-cvlc
```

### Utilisation dans VLC (Interface Graphique) :
1. Lancer VLC et ouvrir une vidéo.
2. Ouvrir le menu **`Vue`** > **`VLSub (dbsrt Enhanced)`**.
3. Choisir la langue et cliquer sur **`Smart Search (All)`** ou **`Search by pHash`**.
4. Sélectionner le sous-titre désiré dans la liste et cliquer sur **`Download + Sync`** (ou **`1-Click Auto-Sync`**).
