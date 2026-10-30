# Comment superviser et auto-guérir les batchs o2switch en continu depuis un Raspberry Pi 4 (24/7)

Ce guide pratique détaille la mise en œuvre d'un agent de supervision autonome (**Watchdog Supervisor**) exécuté en continu sur Raspberry Pi 4 (ou serveur local Debian/Linux). Cet agent surveille l'état d'exécution des batchs sur o2switch, auto-guérit les pannes de processus, régule dynamiquement le nombre de workers selon l'horaire pour préserver la bande passante du NAS Synology, et rapatrie automatiquement les empreintes visuelles `.phash` produites.

---

## 🎯 Objectifs opérationnels

1. **Surveillance permanente (24/7)** : Vérifier régulièrement l'intégrité de la session et des workers sur o2switch sans intervention humaine.
2. **Auto-guérison (Auto-Healing)** : Relancer automatiquement le batch en tâche de fond (`screen -dmS`) s'il a planté (`crashed`) ou disparu (`absent`). Quand une file est vidée (`finished`), le superviseur passe à la bibliothèque suivante (`tvshow` → `movies` → `freebox` → `tvshow`) pour occuper les CPU en continu. Si le lancement de `freebox` échoue (gateway injoignable), il lance directement `tvshow`. Deux lancements sont espacés d'au moins 30 min, pour qu'une file de fichiers toujours en échec ne boucle pas. Un arrêt manuel (`stopped`) ou une sonde en échec (`unknown`) ne déclenchent jamais de lancement.
3. **Autoscaling adaptatif en boucle fermée** : Sonde en temps réel la charge CPU CloudLinux LVE sur o2switch (quota de 12 vCPUs) et adapte dynamiquement la concurrence (2 à 5 workers) pour exploiter la marge résiduelle (cible 78% à 92%), avec repli d'urgence immédiat si pic > 96% et ajustement fluide via `run/desired_workers` sans interruption des flux en cours. Budget CPU : le worker n'admet jamais plus de `DBSRT_CPU_BUDGET_VCPUS // DBSRT_FFMPEG_THREADS` vidéos simultanées (9 // 2 = 4), quel que soit `run/desired_workers` ; le superviseur ne baisse la cible que lorsque les jobs en cours ne la dépassent plus (un job lancé n'est jamais interrompu).
4. **Rapatriement continu (Continuous Pull)** : Synchroniser périodiquement les nouveaux fichiers `.phash` et sous-titres vers le catalogue local (`data/__PHASH__`, stockage Zero-DB).

---

## 🏗️ Architecture du superviseur

```mermaid
flowchart LR
    subgraph RPi4["Raspberry Pi 4 (Superviseur 24/7)"]
        WATCH["scripts/rpi4_o2switch_supervisor.py"]
        LOCAL_CAT[("data/__PHASH__ (Zero-DB)")]
    end

    subgraph O2["Hébergement CloudLinux o2switch"]
        SCREEN["Session screen (dbsrt_batch)"]
        WORKERS["2 à 5 Workers (dbsrt + ffmpeg)"]
    end

    subgraph NAS["Synology NAS"]
        FTP["FTP (Port 21)"]
    end

    WATCH -->|"1. Probe état batch (toutes les 60s)"| SCREEN
    WATCH -->|"2. Relance si crashed/absent"| SCREEN
    SCREEN --> WORKERS
    WORKERS -->|"3. Décodage streaming direct"| FTP
    WATCH -->|"4. Rapatriement régulier (.phash)"| LOCAL_CAT
```

---

## 🛠️ Déploiement et exploitation

### Étape 0 : Installer le service sur le RPi4

L'unité systemd est versionnée dans `deploy/rpi4/dbsrt-supervisor.service`. L'installation synchronise les scripts (`rpi4_o2switch_supervisor.py`, `o2switch_ctl.sh`, `o2switch_batch_worker.py`) vers `~/dbsrt/scripts/`, copie l'unité dans `~/.config/systemd/user/`, puis exécute `daemon-reload` et `enable` :

```bash
task rpi4:supervisor:install
task rpi4:supervisor:restart   # charge le nouveau code
```

Relancez cette installation après toute modification de ces scripts.

### Étape 1 : Démarrer l'agent superviseur

Le cycle de vie de l'agent est piloté par le service systemd utilisateur `dbsrt-supervisor.service` (`Restart=always`) installé sur le Raspberry Pi 4. Les cibles `task rpi4:supervisor:*` appellent `systemctl --user` : ne lancez jamais le script à la main, le verrou `run/rpi4_supervisor.pid` refuserait le doublon et systemd le relancerait en boucle.

Pour démarrer le service :

```bash
task rpi4:supervisor:start
```

### Étape 2 : Vérifier l'état de l'agent superviseur

Pour contrôler que l'agent s'exécute correctement :

```bash
task rpi4:supervisor:status
```

Sortie attendue :
```text
● dbsrt-supervisor.service - dbsrt o2switch 24/7 Watchdog Supervisor
     Active: active (running) since ...
```

### Étape 3 : Suivre le journal d'activité en direct

Pour visualiser en temps réel les vérifications, relances et synchronisations :

```bash
task rpi4:supervisor:logs
```

Exemple d'activité consignée :
```text
[2026-10-28 22:20:44] 🚀 RPi4 Supervisor Agent active (polling: 60s, pull: 600s).
[2026-10-28 22:20:45] ● Remote batch healthy (PID 1815957, workers: 4, CPU baseline pending).
[2026-10-28 22:30:45] 📥 Synchronizing catalog and .phash artifacts from o2switch...
[2026-10-28 22:30:48] ✓ Catalog synchronization complete.
```

### Étape 4 : Inspecter ou régler manuellement la concurrence (Workers)

Pour inspecter la concurrence active sur o2switch :

```bash
task o2switch:batch:workers
# === Querying remote batch concurrency on o2switch ===
# 5
```

Pour forcer un palier de concurrence sans interrompre le worker en vol :

```bash
task o2switch:batch:workers -- 5
# === Setting remote batch concurrency to 5 workers on o2switch ===
# ✓ Set desired workers to 5.
```

Le worker lit dynamiquement `run/desired_workers` et fait monter ou descendre le nombre de threads d'ingestion à la fin de chaque tâche vidéo.

---

### Étape 5 : Suivi Web & Monitoring Mobile (4G/5G)

Pour suivre en direct l'avancement du batch o2switch et l'état du superviseur RPi4 depuis un smartphone (hors du domicile, en 4G/5G Free mobile), un tableau de bord web responsive est exposé publiquement avec une couche d'authentification renforcée :

```bash
task o2switch:monitoring
```

Sortie générée :
```text
=== dbsrt Web Monitoring Dashboard (o2switch & RPi4) ===
  • Dashboard URL : https://votre-domaine.fr/dbsrt/monitoring
  • Mobile Link   : https://votre-domaine.fr/dbsrt/monitoring?token=<CLE_LECTURE_SEULE>
  (Ce lien configure automatiquement le cookie sécurisé et nettoie l'URL pour votre smartphone 4G/5G)

=== QR Code 4G/5G (Scan Smartphone) ===
[... QR Code UTF-8 affiché directement dans le terminal ...]
```

#### Sécurité et Authentification Publique

Le lien mobile et le QR Code embarquent la **clé lecture seule**, jamais la clé maître. Créez-la une fois avant le premier `task o2switch:monitoring` :

```bash
task o2switch:rotate-readonly-key
```

Cette clé ne donne accès qu'en `GET` au tableau de bord, au web-top et aux endpoints `/v1/monitoring/*`. Toute autre requête renvoie `403`. En cas de fuite du QR Code, relancez la même commande : l'ancienne clé est invalidée.

1. **Scan QR Code & Jeton d'authentification mobile (URL Token)** :
   Scannez le QR Code affiché dans le terminal avec l'appareil photo de votre smartphone ou ouvrez le `Mobile Link`. La passerelle valide la clé, crée un cookie de session chiffré (`HttpOnly; Secure; SameSite=Lax`) valide **30 jours**, sert la page directement en HTTP 200 OK (sans saut de redirection 302 afin de garantir la compatibilité avec tous les lecteurs QR et webviews mobiles), et nettoie silencieusement la barre d'adresse du navigateur via l'API HTML5 `window.history.replaceState`. Une copie de secours est également conservée dans le `localStorage` du client.
2. **Durée de vie des identifiants & sessions** :
   - **Clé Secrète API** : Validité illimitée (stockée de manière chiffrée/sécurisée dans `etc/api_key.secret` sur o2switch et dans votre coffre `pass`), renouvelable à la demande via `task o2switch:rotate-key`.
   - **Session Smartphone (Cookie `dbsrt_session`)** : Valide 30 jours consécutifs, renouvelée à chaque reconnexion.
   - **Stockage Local (`localStorage`)** : Persistance permanente sur le navigateur mobile jusqu'à déconnexion explicite.
3. **Écran de connexion mobile dédié** :
   En accédant directement à `https://votre-domaine.fr/dbsrt/monitoring` sans session active ni paramètre de token, un formulaire dark-mode épuré invite à saisir la clé secrète API.
4. **Protection anti force-brute (Rate Limiting)** :
   Toute adresse IP échouant à s'authentifier plus de 5 fois consécutives est temporairement verrouillée pendant 15 minutes (HTTP 429).
5. **Télémétrie unifiée en temps réel** :
   Le tableau de bord actualise toutes les 3 secondes :
   - La jauge de progression globale de la file batch (traités, en attente, échecs, total).
   - Les cartes individuelles de chaque worker en vol (PID, titre vidéo, %, FPS de décodage, ETA).
   - L'état de santé du superviseur RPi4 (ONLINE/OFFLINE, charge LVE CPU instantanée et moyenne mobile).
   - Les 40 dernières lignes du journal `rpi4_supervisor.log` avec coloration syntaxique.

---

### Étape 6 : Arrêter l'agent superviseur

Pour interrompre proprement l'agent :

```bash
task rpi4:supervisor:stop
```

---

## 🔄 Cycle de vie du code et synchronisation multi-niveaux (Workstation ↔ RPi4 ↔ o2switch)

### Problématique
Le système repose sur une architecture distribuée à trois niveaux. Lorsque vous modifiez un script de traitement (comme `scripts/o2switch_batch_worker.py` ou `scripts/o2switch_ctl.sh`) sur votre poste de développement, comment ces modifications se répercutent-elles sur le superviseur autonome Raspberry Pi 4 et les travailleurs distants o2switch ?

### Répartition des Rôles et Stockage du Code

```mermaid
flowchart TD
    DEV["Poste de Travail / Git\n(scripts/o2switch_batch_worker.py)"] -->|"1. scp ou sync"| RPI["Raspberry Pi 4 Superviseur\n(~/dbsrt/scripts/)"]
    DEV -->|"1. scp direct"| O2S["Hébergement o2switch\n(~/dbsrt/scripts/)"]
    RPI -->|"2. Watchdog 60s :\nAuto-healing & relance"| O2S
```

1. **Poste de développement** : Contient le code source sous contrôle Git.
2. **Raspberry Pi 4 (`~/dbsrt/scripts/`)** : Le démon superviseur exécute `rpi4_o2switch_supervisor.py`, qui appelle `o2switch_ctl.sh batch-status`, `batch-workers` et, en cas de crash, `batch-<type>` (type mémorisé dans `run/batch_type` sur o2switch). Lors d'une relance, `o2switch_ctl.sh` synchronise sa copie locale de `o2switch_batch_worker.py` vers o2switch.
3. **Serveur distant o2switch (`~/dbsrt/scripts/`)** : Le script `o2switch_batch_worker.py` y est exécuté en tâche de fond dans une session `screen` détachée (`dbsrt_batch`).

### États du batch (`batch-status`)

`task o2switch:batch:status` affiche une ligne machine lue par le superviseur :

```text
BATCH_STATE=<running|stopped|aborted|finished|crashed|absent> PID=<pid|none> TYPE=<tvshow|movies|freebox>
```

| État | Signification | Action du superviseur |
| :--- | :--- | :--- |
| `running` | Le PID du pidfile est vivant. | Autoscaling uniquement. |
| `stopped` | Marqueur `run/batch_stopped` posé par `batch:stop`. | Aucune relance. |
| `aborted` | Marqueur `run/batch_aborted` posé par le worker après 50 échecs consécutifs de moins de 10 s (erreur systémique : ffmpeg, NAS, identifiants). | Aucune relance, ni changement de bibliothèque. Corriger la cause, puis relancer à la main. |
| `finished` | `run/batch_queue.json` indique `remaining_targets: 0`. | Lance le type suivant (`tvshow` → `movies` → `freebox` → `tvshow`) `--fresh-only`. |
| `crashed` | Pidfile présent, processus mort, sans marqueur d'arrêt. | Relance le même type `--fresh-only --daemon`. |
| `absent` | Aucun pidfile ni marqueur. | Relance `--fresh-only --daemon`. |

### Conséquences Opérationnelles d'une Modification

> [!IMPORTANT]
>
> - **Règle de double déploiement :** Le RPi4 injecte **sa propre copie** de `o2switch_batch_worker.py` lors de chaque relance. Après modification, exécutez `task rpi4:supervisor:install` en plus de `task o2switch:sync`.
> - **Persistance en mémoire RAM :** Modifier un script Python sur le disque o2switch ne modifie **PAS** le processus en vol : l'interpréteur a déjà chargé le code au démarrage.
> - **Un arrêt manuel est définitif :** `task o2switch:batch:stop` pose le marqueur `run/batch_stopped` ; le superviseur ne relance plus le batch. Seul un nouveau `batch-tvshow` / `batch-movies` / `freebox:batch:start` efface ce marqueur.

### Procédure de Déploiement et Prise en Compte Immédiate

Pour déployer un correctif de worker et l'activer immédiatement :

```bash
# Étape 1 : Pousser les scripts à jour vers o2switch ET le superviseur RPi4
task o2switch:sync
task rpi4:supervisor:install
task rpi4:supervisor:restart

# Étape 2 : Arrêter le batch en cours puis le relancer explicitement
task o2switch:batch:stop
task o2switch:batch:tvshow -- --fresh-only --daemon   # ou o2switch:batch:movies
```

---

## 🔐 Variables d'environnement

Toutes ces valeurs se définissent dans l'environnement ou dans le fichier `.env` local non versionné. Aucune valeur par défaut n'est versionnée.

| Variable | Lu par | Rôle |
| :--- | :--- | :--- |
| `NAS_HOST`, `NAS_USER` | `o2switch_ctl.sh`, worker | Hôte et compte FTP du NAS. Écrits dans `etc/.nas_config` (mode 600) sur o2switch s'ils sont définis localement. |
| `NAS_PASS` | `o2switch_ctl.sh`, worker | Mot de passe FTP. Transmis par stdin et écrit dans `etc/.nas_pass` (mode 600) ; jamais en argument ni dans une URL. |
| `DBSRT_FTP_USER`, `DBSRT_FTP_PASSWORD` | `dbsrt` (ffmpeg/ffprobe) | Positionnés par le worker pour chaque processus enfant, puis passés à ffmpeg via `-ftp-user` / `-ftp-password`. Jamais journalisés. |
| `DBSRT_FFMPEG_THREADS` | `dbsrt` (ffmpeg) | Threads de décodage par worker (`2` posé par le worker o2switch ; `0` = auto hors worker). |
| `DBSRT_CPU_BUDGET_VCPUS` | worker o2switch | vCPU maximum engagés par les décodages simultanés (`10`, soit 5 vidéos à 2 threads et environ 85 % du quota LVE de 12 mesuré par `uapi ResourceUsage get_usages`). |
| `O2SWITCH_URL` | `o2switch_ctl.sh rotate-key` | URL publique de la passerelle ; la vérification post-rotation est sautée si absente. |

> [!WARNING]
>
> FTP transporte les identifiants en clair. Restreignez la règle NAT du port 21 à l'IP sortante d'o2switch et utilisez un compte NAS en lecture seule.

---

## ⚙️ Configuration avancée

L'agent accepte les options suivantes en ligne de commande :

| Option | Défaut | Description |
| :--- | :--- | :--- |
| `--check-interval` | `60` | Intervalle de contrôle de l'état distant (en secondes). |
| `--pull-interval` | `600` | Intervalle de synchronisation descendante du catalogue compilé `catalog.dbphash` et sous-titres (en secondes). |
| `--backup-interval` | `3600` | Intervalle d'archivage automatique des empreintes brutes `*.phash` vers le NAS Synology (en secondes). |
| `--remote-host` | `o2switch` | Alias de l'hôte distant défini dans `~/.ssh/config`. |
| `--remote-base` | `dbsrt` | Répertoire distant racine du projet sur o2switch. |

> [!NOTE]
> **Pipeline de sauvegarde autonome RPi4 $\rightarrow$ NAS** :
> En plus du rapatriement léger du catalogue toutes les 10 min, le superviseur déclenche automatiquement toutes les heures (`nas_backup: 3600s`) un `rsync` différentiel des fichiers `.phash` bruts depuis o2switch vers le montage NFS du Synology (`/mnt/synology/video/__PHASH__`). Le RPi4 agit en pur relais réseau en mémoire RAM : **0 écriture sur la carte SD**, sauvegarde 100% pérenne et autonome 24/7.

---

## 🔗 Guides associés

- [Exécuter les batchs de calcul et synchroniser avec o2switch](executer-batch-calcul-et-synchronisation-o2switch.md)
- [Architecture déport calcul, stockage et synchronisation o2switch](../explanation/architecture-deport-calcul-stockage-et-synchronisation-o2switch.md)
- [Déploiement appliance Raspberry Pi 4](../tutorials/04-deploiement-appliance-rpi4.md)
