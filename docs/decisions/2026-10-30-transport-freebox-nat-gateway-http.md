# ADR: Transport Freebox vers o2switch (NAT + gateway HTTP RPi4)

- Date: 2026-10-30
- Statut: Accepté
- Contexte:
  - Le calcul pHash tourne sur o2switch (CPU), les vidéos sont sur les disques de la Freebox. Le RPi4 est trop faible pour calculer lui-même.
  - o2switch doit lire les vidéos à distance, avec accès aléatoire (`Range`) pour que ffmpeg puisse se déplacer dans le fichier.
  - Le RPi4 monte les disques Freebox en CIFS et les sert via `scripts/rpi4_freebox_gateway.py` (HTTP, `206 Partial Content`).
- Options mesurées:
  1. **Tunnel SSH inverse RPi4 → o2switch (`ssh -R`)**: rejeté. o2switch ferme la session environ 20 s après `remote forward success`, 3 fois sur 3, avec ou sans `-N`, `-tt` ou commande distante. Un `ssh -N` sans `-R` tient 150 s : c'est une politique de l'hébergeur, pas une option ssh.
  2. **FTP natif de la Freebox**: rejeté. Il fonctionne (`AUTH TLS` accepté), mais o2switch ne sort que sur `21`, `80` et `443`, et le port WAN `21` est déjà redirigé vers le NAS Synology.
  3. **Redirection NAT Freebox WAN `80` → RPi4 `18089`**, filtrée sur l'IP source d'o2switch: retenu.
- Décision:
  - o2switch lit `http://<NAS_HOST>:80/<chemin>` ; la Freebox redirige vers le gateway du RPi4.
  - Le gateway écoute sur `0.0.0.0:18089` et ne sert que `Disque_1/Téléchargements/`, `Disque_2/Téléchargements/` et `Disque_1_1/Vidéos/` (`SERVED_PREFIXES`, à garder synchronisé avec `scan_freebox_targets.ELIGIBLE_SUBDIRECTORIES`). Photos, sauvegardes et enregistrements renvoient `404`.
  - Les partages sont montés en lecture seule depuis `/etc/fstab`, avec un fichier d'identifiants `/etc/freebox.cred` en mode `600` : le mot de passe ne passe jamais sur une ligne de commande.
  - Le batch `freebox` entre dans la rotation du superviseur (`tvshow` → `movies` → `freebox`). Il est optionnel : si son lancement échoue, le superviseur lance la bibliothèque suivante au lieu de s'arrêter.
- Conséquences:
  - Le flux circule en HTTP clair sur Internet. Seuls le filtre IP source de la Freebox et la liste blanche des chemins servis le protègent.
  - Si l'IP sortante d'o2switch change, la règle bloque o2switch sans bruit : `task freebox:test` repasse en `HTTP 000`.
  - Débit mesuré : environ 2,1 à 2,7 Mo/s par vidéo, 5 vidéos en parallèle.
  - Le port WAN `80` de la Freebox est consommé par le gateway.
- Incident associé:
  - Pendant les essais du tunnel, 616 échecs de transport ont été écrits dans `data/reports/batch_tvshow_nightly.jsonl` et 54 vidéos Freebox sont passées en liste noire (3 échecs consécutifs). Ces lignes ont été retirées, avec sauvegarde, une fois le transport réparé.
- Sites:
  - `scripts/rpi4_freebox_gateway.py` (`is_served_path`, `SERVED_PREFIXES`)
  - `scripts/rpi4_freebox_ctl.sh`, `scripts/freebox_batch_ctl.sh` (`check_gateway`, `resolve_stream_base`)
  - `scripts/rpi4_o2switch_supervisor.py` (`NEXT_BATCH_TYPE`, `OPTIONAL_BATCH_TYPES`)
  - `deploy/rpi4/dbsrt-freebox-gateway.service`, `taskfiles/freebox.yml`
- Tests:
  - `tests/test_freebox_gateway.py::test_served_paths_are_limited_to_media_libraries`
  - `tests/test_freebox_supervisor_rotation.py::test_failed_freebox_launch_falls_back_to_next_library`
