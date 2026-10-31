# ADR: `dbsrt-server` sur socket Unix chez o2switch

- Date: 2026-10-31
- Statut: Accepté
- Contexte:
  - Sur o2switch (mutualisé CloudLinux/LVE, Apache), la passerelle PHP `deploy/o2switch/index.php` lance `dbsrt-server` via `setsid` et lui relaie les requêtes.
  - En TCP (`127.0.0.1:8765`), le démon ne vivait que 5 à 20 s. Son log ne contient aucun `Shutdown signal received`. Le faultwatch a compté 54 relances en 184 min.
  - Chaque relance coûte un démarrage à froid (rechargement du catalogue) et produit des `421`/`502` intermittents sur la passerelle.
  - L'enquête complète est dans [Enquête : kill périodique de `dbsrt-server` sur o2switch](../explanation/2026-10-31-enquete-kill-periodique-dbsrt-server-o2switch.md).
- Options mesurées:
  1. **TCP + relance (statu quo)**: rejeté. Ça fonctionne, mais avec des erreurs intermittentes et un démarrage à froid toutes les ~20 s.
  2. **CLI Rust par requête depuis PHP (sans démon)**: non retenu. Rien n'écoute, donc rien à tuer. Mais chaque requête paie un démarrage, il faut recoder `/v1/monitoring/top` en CLI, et `max_execution_time` borne les requêtes.
  3. **App Passenger (cPanel « Setup Python App »)**: non retenu pour l'instant. C'est le seul mode serveur documenté par o2switch, mais il touche à la config cPanel. Il faudrait soit un proxy Python vers le binaire Rust, soit exposer le moteur en PyO3/WSGI (gros chantier).
  4. **Socket Unix**: retenu. Test sur o2switch : un listener TCP est tué en 5 à 19 s ; un listener sur socket Unix vit 180 s sur 180, comme un process qui n'écoute pas.
- Décision:
  - `dbsrt-server` accepte `--unix-socket <chemin>` (env `DBSRT_UNIX_SOCKET`). Sans cette option, le serveur reste en TCP.
  - Sur o2switch, le démon écoute sur `~/dbsrt/run/dbsrt-server.sock`. La passerelle PHP (`CURLOPT_UNIX_SOCKET_PATH`), `scripts/o2switch_ctl.sh` (`curl --unix-socket`) et le watchdog cron utilisent ce chemin.
  - Au démarrage, le serveur refuse un chemin qui n'est pas une socket. Il refuse aussi une socket vivante (un autre démon y répond). Il supprime une socket morte, se lie, puis passe le fichier en `0600`.
  - À l'arrêt propre, le serveur supprime le fichier de socket.
  - L'agent relais local, le RPi4 et le dev restent en TCP.
- Conséquences:
  - Mesuré après déploiement : même PID pendant plus de 606 s, aucune relance au passage du cron. `/v1/health` en 0,08 s ; `/v1/monitoring/top` en 2,7 s à froid puis 0,1 s ; 8 appels `/v1/recognize` en `200` (0,19 à 0,77 s).
  - En mode socket Unix, le serveur n'écrit pas `/tmp/dbsrt-server.json` (fichier de découverte). Seul VLC en local le lit.
  - Socket en `0600` : les autres comptes du mutualisé ne peuvent pas s'y connecter. C'est plus strict que le TCP local, ouvert à tout l'hôte.
  - Trois lanceurs peuvent démarrer le démon : la passerelle PHP (sérialisée entre requêtes par `run/gateway_spawn.lock`), le watchdog cron et `o2switch_ctl.sh start`. Sur une socket morte, deux lanceurs simultanés pouvaient chacun la juger morte : le second supprimait la socket neuve du premier, laissant un démon orphelin qui gardait sa RAM. `bind_unix_listener` prend donc un `flock` exclusif sur `<socket>.lock` (mode `0600`, jamais supprimé) pendant la vérification, la suppression et le bind. Le second lanceur voit alors la socket vivante et s'arrête.
  - Les trois lanceurs passent par `deploy/o2switch/dbsrt_server_ctl.sh` (cron, `start`) ou partagent son verrou (passerelle). `stop` tue tout processus `bin/dbsrt-server --unix-socket <socket>` de l'utilisateur et pose `run/dbsrt-server.stopped` sous le verrou : la passerelle répond alors `503` et le watchdog ne relance plus, jusqu'au prochain `start`.
  - Le verrou ne doit pas fuir dans le démon. La passerelle ouvrait `gateway_spawn.lock` sans close-on-exec, et le `sh` de `exec()` gardait le pipe de sortie ouvert : la requête PHP bloquait, était tuée avant de libérer le verrou, et le démon héritait du fd verrouillé. Tout `stop` ou watchdog suivant attendait ce verrou indéfiniment (constaté en production). La passerelle ouvre donc le verrou en mode `'ce'`, détache le démon (`exec setsid`, sortie redirigée), et le helper borne l'attente du verrou (`DBSRT_LOCK_WAIT_SEC`, 30 s par défaut). Après ce délai, `stop` tue les démons de la socket puis réessaie ; les autres actions échouent au lieu de bloquer.
  - Le mécanisme exact du kill o2switch reste inconnu (sortie `exit=0`, aucun signal capturable). Si o2switch étend sa politique aux sockets Unix, il faudra reprendre l'option 2 ou 3.
- Sites:
  - `crates/dbsrt-server/src/lib.rs` (`bind_unix_listener`)
  - `crates/dbsrt-server/src/main.rs` (`ServerCli::unix_socket`, `BoundListener`, `run_server_until`)
  - `deploy/o2switch/index.php` (`UPSTREAM_SOCKET_REL_PATH`, `is_daemon_healthy`, `spawn_daemon`)
  - `deploy/o2switch/dbsrt_server_ctl.sh` (`start`, `stop`, `status`, `pids`, `watchdog`)
  - `scripts/o2switch_ctl.sh` (`SERVER_SOCKET`, `remote_server_ctl`, `setup-cron`)
- Tests:
  - `crates/dbsrt-server/src/lib.rs::tests::test_bind_unix_listener_cases`
  - `crates/dbsrt-server/src/lib.rs::tests::test_bind_unix_listener_waits_for_spawn_lock`
  - `crates/dbsrt-server/src/main.rs::tests::test_server_unix_socket_lifecycle`
  - `tests/tools/test_dbsrt_server_ctl.py` (stop, start, status, pids, watchdog sur faux démons ; verrou tenu par un démon ou par un autre processus)
  - `scripts/o2switch_e2e.py` (`task o2switch:e2e`) : recette live sur o2switch (cron, start/stop/restart, `503` après stop, relance passerelle et watchdog après crash, aucun processus ne garde le verrou, reconnaissance du bon média)
