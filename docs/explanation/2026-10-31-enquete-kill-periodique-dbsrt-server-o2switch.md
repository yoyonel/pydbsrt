# Enquête : kill périodique de `dbsrt-server` sur o2switch

> **Question** : pourquoi `dbsrt-server`, lancé par la passerelle PHP sur o2switch, meurt-il toutes les 5 à 20 s alors que les batchs pHash tournent des heures ?
>
> **Réponse courte** : o2switch termine les process qui écoutent sur un port TCP, même en `127.0.0.1`. Un listener sur socket Unix n'est pas touché. Le démon écoute donc désormais sur `~/dbsrt/run/dbsrt-server.sock` (voir l'[ADR socket Unix](../decisions/2026-10-31-dbsrt-server-unix-socket-o2switch.md)).

Ce document garde la trace des pistes suivies, y compris celles qui se sont révélées fausses, pour ne pas les reparcourir.

## 1. Symptômes

- Le log du serveur montre un `daemon listening on http://127.0.0.1:8765` toutes les ~21 s, sans aucun `Shutdown signal received` entre deux.
- L'absence de message d'arrêt écarte un arrêt propre (SIGTERM/SIGINT gérés par le serveur) et un panic (qui laisse une trace).
- La requête suivante arrive sur `deploy/o2switch/index.php`, qui relance le démon (`spawn_daemon`). Elle revient en `421` ou `502`. Dans l'access log, les `421` tombent toutes les ~21 s, au rythme des relances.
- Le faultwatch (`dbsrt-server_spawns`) a compté 54 relances en 184 min, réparties sur 23 minutes différentes.
- Les kills ont commencé le 30/10 vers 12h UTC. Avant, le démon tenait longtemps. La cause du changement côté o2switch n'est pas connue.
- Les batchs ne sont pas touchés : le batch worker (`scripts/o2switch_batch_worker.py`, dans `screen`) et ses `dbsrt media:add` vivent toute la durée d'une vidéo. Aucun n'écoute sur un port.
- L'ingestion ne dépend pas du serveur : `media:add` écrit catalogue et couplages sur disque. Le log du serveur ne contient aucun appel `catalog/ingest`. Le coût des kills se limite donc aux démarrages à froid et aux erreurs de la passerelle.

## 2. Pistes écartées

### 2.1 Fautes LVE CPU

Une faute CPU LVE bride les process (*throttling*), elle ne les tue pas. Les kills ne suivent pas les pics de fautes. Détails dans l'[analyse des fautes LVE CPU](../2026-10-31-o2switch-lve-cpu-faults-analysis.md).

### 2.2 Limite mémoire, NPROC, EP, IO

Hypothèse un temps retenue : la limite mémoire LVE (PMEM) tuerait le démon, qui charge le catalogue en RAM. Le tableau cPanel « Resource Usage » ne montre aucune faute PMEM, NPROC ou EP sur la période (PMEM max 2,4 Gio pour 48 Gio autorisés). Un kill OOM laisserait de toute façon un code `137`, pas la sortie `0` du §3.3.

### 2.3 LiteSpeed / lsphp qui tue les enfants de PHP

Hypothèse : le serveur web tue les process lancés depuis un worker PHP, même détachés par `setsid`. Écartée pour deux raisons :

- o2switch sert le compte avec **Apache**, pas LiteSpeed.
- Le test du §3 reproduit le kill depuis une session `ssh`/`screen`, hors de tout PHP.

Le `421` ne vient pas de notre code (aucune occurrence dans le dépôt). C'est le frontal qui le renvoie quand l'amont meurt pendant la requête.

### 2.4 Outils LVE côté compte

Impossible de voir qui tue quoi depuis le shell : CageFS masque `lve-read-snapshot`, `lvectl`, `lve_suwrapper`, `/var/lve`, `/proc/lve` et `/sys/fs/cgroup`. `uapi ResourceUsage` ne donne que l'instant présent. Le cPanel ne montre que les fautes par minute.

### 2.5 Documentation o2switch

- La FAQ « Setup Python App » décrit **Phusion Passenger** (`cloudlinux-selector`, `.htaccess`). C'est le seul mode serveur documenté. Passenger parle à ses apps par socket Unix.
- La page du sélecteur PHP ne parle ni de `exec`, ni de process de fond, ni de ports.
- Aucune page ne mentionne une politique contre les listeners.

## 3. Piste retenue : un watchdog qui vise les listeners TCP

### 3.1 Indice : le tunnel `ssh -R`

Le 30/10, un tunnel `ssh -R` RPi4 vers o2switch a été fermé ~20 s après `remote forward success`, 3 fois sur 3. Un `ssh -N` sans `-R` tenait 150 s (voir l'[ADR transport Freebox](../decisions/2026-10-30-transport-freebox-nat-gateway-http.md)). Point commun avec `dbsrt-server` : un port en écoute côté o2switch, et la même durée de vie d'environ 20 s.

### 3.2 Test 1 : TCP contre aucune écoute

Deux process Python lancés dans `screen`, durée prévue 180 s :

| Process | Durée de vie | Code de sortie |
| :--- | :--- | :--- |
| écoute sur `127.0.0.1:8799` | **5 s** | `0` |
| n'écoute pas | 180 s (fin normale) | `0` |

### 3.3 Test 2 : TCP, socket Unix, aucune écoute

Trois process en parallèle, 180 s. Chacun installe des handlers pour `TERM`, `HUP`, `INT`, `QUIT`, `USR1` et `PIPE`, et logue toute exception :

| Process | Résultat |
| :--- | :--- |
| écoute TCP `127.0.0.1:8799` | **terminé à 19 s**, `exit=0`, aucun signal capturé, aucune exception |
| écoute socket Unix `run/killtest.sock` | vivant pendant les 180 s |
| n'écoute pas | vivant pendant les 180 s |

### 3.4 Lecture

- o2switch termine les listeners TCP en 5 à 20 s et ne touche pas aux sockets Unix. C'est cohérent avec Passenger, qui sert ses apps par socket Unix.
- Le mécanisme reste inconnu. Un SIGKILL donnerait `137`, un SIGTERM non géré `143`. Ici la sortie est `0` et aucun handler ne se déclenche. On sait seulement que le process est terminé.

## 4. Correction et vérification

- Le démon écoute sur `~/dbsrt/run/dbsrt-server.sock` (mode `0600`). La passerelle PHP, `scripts/o2switch_ctl.sh` et le watchdog cron passent par cette socket.
- Après déploiement : même PID à 27, 119, 240, 362, 484 et 606 s, aucune relance pendant ces 10 min de suivi. Le passage du watchdog cron (`*/5`) n'a pas relancé le démon.
- Faultwatch (`dbsrt-server_spawns`) : 90 relances de 05:00 à 08:17 UTC (198 min, TCP), 0 dans les 14 min qui suivent le déploiement (socket Unix).
- Via la passerelle publique : `/v1/health` en 0,08 s ; `/v1/monitoring/top` en 2,7 s à froid puis 0,1 s ; 8 appels `/v1/recognize` en `200`, `found=true`, confiance 100, Tier 1.

## 5. Limites

- Chaque test n'a été lancé qu'une fois, sur 180 s, depuis `ssh`/`screen`. Le contexte réel (spawn depuis PHP) n'a été vérifié qu'après déploiement, sur ~10 min.
- Rien ne garantit que la politique d'o2switch épargnera toujours les sockets Unix. Le faultwatch (`dbsrt-server_spawns`) sert d'alarme : s'il remonte, reprendre les options « CLI par requête » ou « Passenger » de l'ADR.
- Pas de question posée au support o2switch à ce jour.
