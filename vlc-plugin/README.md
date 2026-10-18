# Plugin VLC dbsrt Subtitle Auto-Sync (v2.0)

> Extension Lua officielle pour VLC Media Player pour la détection et synchronisation automatique de sous-titres par analyse visuelle des frames (DCT-II SIMD) avec affichage On-Screen Display (OSD).

## 1. Démonstration Visuelle

![Démonstration Plugin VLC v2.0](../docs/assets/vlc-v2/03_autosync_completed.png)

---

## 2. Fonctionnalités v2.0

* **Auto-Sync Événementiel Zero-Click** : Dès qu'une vidéo démarre dans VLC, l'écouteur `input_changed` contacte `dbsrt-server` en arrière-plan sans bloquer la lecture.
* **Notification OSD Native LibVLC** : Feedback visuel incrusté en surimpression sur la vidéo (`✓ dbsrt: [FR] Synced (+0.12s, 96% conf) via container`).
* **Recherche Multi-Sources (`/v1/search`)** : Inspection unifiée des pistes conteneur (MKV/MP4), du catalogue local pHash, et du provider distant OpenSubtitles.
* **Panneau de Contrôle Interactif (Qt)** :
  * Sélecteur de langue rapide (`[FR]`, `[EN]`, `[AUTO]`).
  * Interrupteur Auto-Sync on/off.
  * Télémétrie daemon (`/v1/status` : uptime, version, nombre de médias indexés).
* **Interface Console Headless (`cvlc`)** : Interface `dbsrt_intf.lua` pour CI/CD et pipelines de scripts autonomes avec terminaison déterministe (`DBSRT_AUTOKILL=1`).

---

## 3. Guide d'Installation & Utilisation

```bash
# 1. Compiler et déployer les plugins VLC dans ~/.local/share/vlc/lua/
task vlc:install-plugin

# 2. Démarrer le daemon backend en arrière-plan
task server:daemon

# 3. Vérifier la télémétrie du daemon
task server:status

# 4. Tester l'intégration headless avec cvlc
task vlc:test-cvlc
```

### Utilisation dans VLC (Interface Graphique) :
1. Démarrer VLC et lancer une vidéo.
2. Si l'auto-sync est activé, une notification OSD apparaît automatiquement à l'écran confirmant le calage.
3. Pour ouvrir le panneau de contrôle : Menu **`Vue`** > **`dbsrt Subtitle Auto-Sync`**.
4. Cliquer sur **`🔍 Search Candidates`** pour explorer les pistes disponibles ou **`⚡ Auto-Sync Subtitle`** pour forcer un réalignement.
