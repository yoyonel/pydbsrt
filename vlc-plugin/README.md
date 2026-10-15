# Plugin VLC dbsrt Subtitle Auto-Sync

> Extension Lua officielle pour VLC Media Player pour la détection et synchronisation automatique de sous-titres par analyse visuelle des frames (DCT-II SIMD).

---

## 1. Fonctionnement

Contrairement au plugin VLSub traditionnel qui se base sur le hash binaire d'OpenSubtitles (qui échoue dès qu'une vidéo est réencodée ou provient d'une release différente) :

1. L'extension VLC `dbsrt_subsync.lua` lit l'URI de la vidéo en cours de lecture dans VLC.
2. Elle interroge le microservice local Rust `dbsrt-server` (sur `http://127.0.0.1:8080/v1/retarget`).
3. Le serveur analyse les cuts visuels, extrait les empreintes pHash et résout le consensus de décalage temporel $\Delta t$.
4. Le sous-titre recalé est généré et injecté **directement à chaud** dans VLC via l'API `vlc.input.add_subtitle()`.

---

## 2. Installation Rapide

```bash
# 1. Compiler et lancer le daemon Rust
task server:build
task server:run  # écoute sur http://127.0.0.1:8080

# 2. Installer l'extension dans VLC (copie dans ~/.local/share/vlc/lua/extensions/)
task vlc:install-plugin
```

Puis dans VLC :
1. Redémarrer VLC (ou aller dans `Outils` > `Extensions et greffons` > recharger).
2. Lancer la lecture d'une vidéo.
3. Ouvrir le menu **`Vue`** > **`dbsrt Subtitle Auto-Sync`**.
4. Cliquer sur **`⚡ Auto-Sync Subtitle`**.
5. Le sous-titre est synchronisé et appliqué automatiquement à la milliseconde près !
