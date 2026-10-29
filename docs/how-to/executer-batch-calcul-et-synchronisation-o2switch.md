# Comment orchestrer les batchs de calcul pHash distants, monitorer et synchroniser les catalogues avec o2switch

Ce guide pratique détaille pas-à-pas comment déporter le calcul intensif d'empreintes visuelles (**pHash**) et l'extraction de sous-titres sur l'infrastructure Cloud mutualisée **o2switch**, monitorer les traitements en direct depuis votre terminal, et rapatrier les catalogues synchronisés de façon idempotente vers votre environnement local.

---

## 🎯 Objectifs opérationnels

1. **Calcul distant intensif** : Exécuter l'ingestion par batch des épisodes non traités (**"Vierges"**) sur les 12 vCPUs d'o2switch avec une concurrence dynamique (1 à 8 workers, ajustable à chaud via `run/desired_workers`) sans saturation ni blocage CloudLinux LVE. Le superviseur RPi4 l'auto-régule entre 2 et 6 workers (cible 78% à 92% de CPU LVE).
2. **Zéro-Copie NVMe** : Traiter les flux vidéo directement en mémoire RAM via FFmpeg, lu directement en FTP (port 21) depuis le NAS Synology, sans jamais copier de fichiers vidéo volumineux sur l'hébergement mutualisé.
3. **Supervision temps réel TUI** : Suivre l'exécution, les PIDs, le décodage, les débits réseau I/O et les prédictions d'ETA directement dans le terminal via `dbsrt top` en session SSH.
4. **Synchronisation idempotente** : Rapatrier les empreintes binaires `.phash`, les sous-titres adressés par le contenu (CAS SHA-256) et consolider le registre `couplings.json` et le catalogue haute densité `.dbphash`.

---

## 📋 Prérequis

Avant de lancer les opérations, assurez-vous d'avoir validé les points suivants :

- **Accès SSH configuré** : L'hôte `o2switch` doit être défini dans `~/.ssh/config` avec votre clé Ed25519 (`ssh o2switch "echo OK"`).
- **Accès NAS Synology** : Le service FTP (port 21) du NAS Synology doit être accessible depuis o2switch, avec une redirection NAT Freebox filtrée sur l'IP sortante d'o2switch et un compte NAS en lecture seule (voir le guide [Interconnecter le NAS Synology et o2switch](interconnecter-nas-synology-et-o2switch-ftp.md)).
- **Identifiants NAS** : `NAS_HOST`, `NAS_USER` et `NAS_PASS` proviennent de l'environnement ou du fichier `.env` local non versionné (aucune valeur par défaut versionnée). `o2switch_ctl.sh` les transmet par stdin vers `etc/.nas_config` et `etc/.nas_pass` (mode 600) sur o2switch.
- **Binaires et outils déployés** : Les binaires statiques musl (`dbsrt`, `dbsrt-server`) et Python 3.11 sont déployés sur o2switch par `task o2switch:sync`. `ffmpeg` / `ffprobe` ne le sont pas : voir ci-dessous.

### Installer ou mettre à jour FFmpeg sur o2switch

o2switch (CloudLinux 8, glibc 2.28) exige un FFmpeg entièrement statique. Les builds BtbN demandent glibc 2.31 et ne démarrent pas. On utilise les builds statiques musl de l'image `mwader/static-ffmpeg`, en restant sur la dernière branche `x.1` (les branches FFmpeg maintenues le plus longtemps). Version installée : **8.1.2**.

```bash
V=8.1.2
c=$(docker create mwader/static-ffmpeg:$V-amd64)
docker cp "$c:/ffmpeg" ./ffmpeg && docker cp "$c:/ffprobe" ./ffprobe && docker rm "$c"
rsync -az ffmpeg o2switch:dbsrt/bin/ffmpeg-$V && rsync -az ffprobe o2switch:dbsrt/bin/ffprobe-$V
# Bascule atomique, l'ancienne version reste en .bak. Les processus en cours gardent l'ancien binaire.
ssh o2switch "cd dbsrt/bin && for b in ffmpeg ffprobe; do cp -p \$b \$b.bak && mv \$b-$V \$b; done && ./ffmpeg -hide_banner -version | head -1"
```

Avant la bascule, vérifier que la nouvelle version produit des `.phash` identiques à l'ancienne sur une vidéo du NAS (`dbsrt media:add ... --skip-certify` lancé deux fois, avec chaque version en tête du `PATH`). Retour arrière : `mv ffmpeg.bak ffmpeg` (idem `ffprobe`).

Le mot de passe NAS est passé à `ffmpeg` par `-ftp-password` et apparaît donc dans son argv. FFmpeg (vérifié jusqu'à 8.1) refuse `-/ftp-password <fichier>` pour les options de protocole. Sur o2switch, `/proc` est monté avec `hidepid=2` : les autres comptes ne voient pas nos processus.

---

## 🛠️ Recette 1 : Lancer le traitement par batch des vidéos "Vierges"

### Problématique
Une vidéothèque contient des centaines d'épisodes. Vous souhaitez traiter uniquement les épisodes non encore analysés (notés **"Vierge"** dans le studio `dbsrt top`), sans recalculer les empreintes déjà certifiées dans `couplings.json` ou présentes dans `data/__PHASH__/`.

### Procédure Taskfile

Pour lancer le batch sur o2switch en ciblant les vidéos vierges avec 8 workers en parallèle :

```bash
task o2switch:batch:tvshow -- --workers 8 --fresh-only
```

Pour lancer un traitement de masse en tâche de fond permanente (démon persistant même si vous fermez votre session SSH ou éteignez votre PC portable) :

```bash
# Ingestion des séries TV (/tvshow)
task o2switch:batch:tvshow -- --workers 4 --fresh-only --daemon

# Ingestion des films (/video)
task o2switch:batch:movies -- --workers 4 --fresh-only --daemon
```

Par défaut, le worker lit ses cibles dans `data/reports/selected_batch_targets.json` (`--targets-file`). `o2switch_ctl.sh` résout ce fichier localement depuis le montage NAS quand il est disponible ; sinon le worker liste le NAS en FTP.

Pour inspecter l'état du travailleur de fond (processus actif, PID, CPU, RAM, et progression détaillée de la file) :

```bash
task o2switch:batch:status
```

Exemple de retour télémétrique en direct :

```text
BATCH_STATE=running PID=<pid> TYPE=tvshow
● Batch worker is RUNNING (PID: <pid>)
  PID USER          %CPU %MEM     ELAPSED COMMAND
<pid> <cpanel_user>  0.3  0.1       30:54 python3

--- File de Traitement Batch (Progression) ---
  • Progression file : 434 / 674 traités (64.4%)
  • En attente       : 240 cibles restantes (Échecs: 0)
  • Concurrence      : 4 workers parallèles
```

La première ligne `BATCH_STATE=<running|stopped|aborted|finished|crashed|absent> PID=<pid|none> TYPE=<tvshow|movies>` est lue par le superviseur RPi4.

Pour suivre le journal d'exécution en streaming ou interrompre le traitement (`batch:stop` pose le marqueur `run/batch_stopped` : le superviseur RPi4 ne relance jamais un batch arrêté ou terminé) :

```bash
task o2switch:batch:logs
task o2switch:batch:stop
```

Pour ajuster la concurrence à chaud, sans interrompre les vidéos en cours (bornes 1 à 8) :

```bash
task o2switch:batch:workers -- 6
```

Pour limiter l'exécution à un sous-ensemble d'épisodes (ex: tester 5 épisodes lors d'une phase de validation) :

```bash
task o2switch:batch:tvshow -- --workers 2 --fresh-only --limit 5
```

### Mécanisme de ciblage des ressources (Palier 80%)
- **Allocation CPU** : L'environnement o2switch dispose d'un quota CloudLinux LVE de 12 vCPUs. Le réglage `--workers 8` (maximum autorisé) mobilise $\approx 80\%$ de la capacité CPU totale, laissant une marge de sécurité de 2 à 4 cœurs pour le reverse-proxy Apache, SSH et les démons système.
- **Empreinte RAM** : Chaque worker consomme $\approx 150\text{ à }200\text{ MiB}$ de mémoire vive (buffer FFmpeg et décompression de trame 32x32). Le total de 8 workers ($\approx 1.5\text{ GiB}$) reste largement sous le quota LVE de $12\text{ GiB}$.
- **Débit réseau I/O** : Le débit cumulé de 8 flux streaming ($\approx 8 \times 2.5\text{ MiB/s} = 20\text{ MiB/s}$) respecte scrupuleusement le plafond CloudLinux LVE fixé à $42\text{ MiB/s}$.

---

## 🌊 Recette 2 : Vérifier le streaming direct Zéro-NVMe

### Problématique
Les hébergements mutualisés disposent d'un espace disque limité et de quotas d'inodes stricts. Copier des fichiers vidéo MKV/MP4 de 1 à 4 Go sur le disque NVMe o2switch saturerait l'espace de stockage et déclencherait des pénalités d'I/O disque.

### Fonctionnement garanti
1. Le worker distant lance `dbsrt media:add` sur une URL FTP directe `ftp://<NAS_HOST>:21/<chemin>`, sans identifiants dans l'URL. Il positionne `DBSRT_FTP_USER` / `DBSRT_FTP_PASSWORD` dans l'environnement de chaque processus enfant.
2. `dbsrt` délègue la lecture à `ffmpeg` / `ffprobe` (options `-ftp-user` / `-ftp-password`, jamais journalisées ; les URLs journalisées sont expurgées), qui démultiplexe et décode le conteneur directement en mémoire RAM (`DBSRT_FFMPEG_THREADS`, `2` posé par le worker).
3. Seuls les artefacts finaux à haute valeur ajoutée sont écrits sur le disque o2switch :
   - Fichier binaire `.phash` : $\approx 3\text{ Ko}$ par épisode (51 frames clés $\times$ 64 bits).
   - Sous-titres extraits `.srt` : $\approx 50\text{ Ko}$ par piste de langue.
4. **Vérification de l'occupation disque** :
   ```bash
   task o2switch:status
   ```
   L'espace disque consommé par les vidéos est rigoureusement de **0 octet**.

---

## 🖥️ Recette 3 : Monitorer les calculs en cours (`dbsrt top`)

### Problématique
Vous souhaitez suivre en direct la charge des 12 cœurs CPU, la mémoire consommée, les débits réseau entrants du NAS Synology, ainsi que l'avancement et l'ETA de chaque worker individuel.

### Procédure Taskfile

Lancez le moniteur de processus TUI interactif en SSH :

```bash
task o2switch:top
```

Le script alloue un pseudo-terminal PTY (`ssh -t`) et active l'interface Ratatui plein écran :

- **Onglet 1 : Tâches Actives (`1`)** :
  - Liste chaque worker `ffmpeg` / `dbsrt` actif.
  - Affiche le nom de l'épisode, son PID, sa consommation CPU % et RAM RSS.
  - Indique le débit streaming instantané (MiB/s) mesuré via `/proc/<pid>/io`.
  - Fournit la jauge de progression et l'ETA calculé dynamiquement.
- **Onglet 4 : Explorateur de Médias & Cibles Batch Distantes (`4` ou `m`)** :
  - Sur nœud distant sans stockage local monté (ex: o2switch avec flux FTP Zéro-NVMe), alimenté directement par le registre des cibles batch (`data/reports/selected_batch_targets.json`).
  - Affiche l'ensemble des médias du batch avec leur statut en direct (`[EN COURS]` si un worker décode, `[COMPLÉTÉ]` dès certification `.phash`, `[VIERGE]` si en attente).
  - Suppression automatique des scans disque en arrière-plan (aucun clignotement ni message intempestif *"Scan en cours..."* en l'absence de montages physiques locaux).
- **Onglet 5 : Sources de Flux Réseau (`5` ou `s`)** :
  - Commute automatiquement en moniteur de flux réseau en mode Zéro-NVMe : affiche la source NAS FTP (ex: `ftp://nas:21`), le débit de transit et le total des cibles batch.
  - Verrouille les actions d'ajout de répertoires physiques locaux (qui n'existent pas sur le nœud distant) et dédie `[r]` au rafraîchissement rapide du registre JSON.
- **En-tête de télémétrie** :
  - Jauge globale CPU (permet de vérifier visuellement l'atteinte du palier cible $\approx 80\%$).
  - Jauge globale RAM et débit réseau entrant (`net_rx_mib_s`).
  - Progression globale de la file active (`Batch: X/Y (Z%)`).
- **Raccourcis de navigation** :
  - `1` : Tâches Actives
  - `2` / `h` : Statistiques & Distribution descriptive
  - `3` / `r` : Historique des médias complétés & diagnostics d'erreurs
  - `4` / `m` : Explorateur de Médias & Cibles batch distantes
  - `5` / `s` : Sources de flux réseau (FTP)
  - `6` / `y` : Télémétrie système hôte (Fastfetch)
  - `?` : Aide contextuelle et glossaire des statuts
  - `q` : Quitter le moniteur (les calculs en arrière-plan continuent normalement).

> [!TIP]
> Pour inspecter l'état sous forme de snapshot JSON non-interactif (ex: dans un script d'audit ou une sonde de monitoring) :
> ```bash
> task o2switch:top -- --json
> ```

---

## 📦 Recette 4 : Rapatrier les résultats et synchroniser les catalogues

### Problématique
Une fois le batch d'épisodes terminé sur o2switch, les fichiers `.phash` et `.srt` doivent être rapatriés sur la machine locale et consolidés dans les catalogues de référence.

### Étape 1 : Rapatriement différentiel des artefacts

Exécutez la tâche de synchronisation descendante :

```bash
task o2switch:pull-catalog
```

Cette commande exécute un `rsync` différentiel sécurisé (`--ignore-existing`) plafonné à $20\text{ MB/s}$ :

- Rapatrie tous les nouveaux fichiers `*.phash` dans `data/__PHASH__/`.
- Rapatrie toutes les nouvelles pistes de sous-titres dans `data/SRT/<hash>.srt`.
- Télécharge le journal d'événements `batch_tvshow_nightly.jsonl` dans `data/reports/`.

### Étape 2 : Consolider et certifier les couplages locaux

Intégrez les nouvelles empreintes et sous-titres dans le registre local `couplings.json` :

```bash
task coupling:certify
```

### Étape 3 : Recompacter le catalogue binaire haute densité

Reconstruisez le fichier d'index binaire `data/catalog.dbphash` pour intégrer toutes les nouvelles empreintes :

```bash
task catalog:compact
```

### Étape 4 (Optionnel) : Synchroniser la base PostgreSQL 17

Si vous utilisez le moteur d'indexation BK-Tree en base relationnelle PostgreSQL :

```bash
task db:import:all-phash
```

---

## 🛡️ Recette 5 : Gestion des pannes et idempotence

### Que se passe-t-il en cas de déconnexion réseau ou d'interruption ?
- **Idempotence CAS (Content-Addressed Storage)** : Les fichiers de sous-titres sont nommés selon le hachage SHA-256 de leur contenu (`sub:sha256:<hash>.srt`). Si un fichier a déjà été téléchargé ou calculé, l'écriture est ignorée sans écrasement ni duplication.
- **Verrouillage atomique du registre** : `couplings.json` est mis à jour via un mécanisme de copie temporaire suivie d'un renommage atomique POSIX (`RegistryLock`). Aucun fichier corrompu à moitié écrit ne peut survenir en cas de crash.
- **Reprise après coupure** :
  Si le batch est relancé avec la même commande :
  ```bash
  task o2switch:batch:tvshow -- --workers 8 --fresh-only
  ```
  Le filtre scanne les fichiers `.phash` existants et les entrées dans `couplings.json`. Les épisodes déjà traités lors de la session précédente sont automatiquement sautés en $0\text{ ms}$, et le calcul reprend immédiatement sur le premier épisode vierge restant.

---

## 🔄 Recette 6 : Relancer le traitement ciblé sur les vidéos en échec (FAIL)

### Problématique
Dans l'onglet `[3/r] Récents` de `dbsrt top` (avec le filtre `[x FAIL (ERREURS SEULES)]`), certaines vidéos apparaissent avec le statut `x FAIL` (statuts worker `ERROR`, `STALLED` ou `TIMEOUT`, par opposition à `COMPLETED`). Vous souhaitez relancer le traitement uniquement sur ces vidéos en échec sans rescanner l'intégralité des milliers de fichiers du NAS Synology.

### Diagnostics et Causes Fréquentes d'Échec
1. **Lecture refusée par le NAS** (`550 Permission denied` sur `RETR`, alors que le listing et `SIZE` répondent) : le compte FTP n'a pas le droit de lecture sur le fichier. Côté `dbsrt`, l'erreur apparaît comme `No video frames could be extracted`. Corriger les permissions du dossier partagé sur le Synology, puis relancer avec `--retry-failed`.
2. **Noms Unicode** : le NAS mélange des noms NFC (dossiers) et NFD (fichiers copiés depuis macOS, ex. `Cruauté`). Le worker garde le nom brut renvoyé par le serveur FTP pour l'URL, et n'utilise la forme NFC que pour le nom du média.
3. **Noms 8.3 du montage CIFS** : un montage CIFS local remplace les noms contenant `:` par des noms courts (`SY9QH1~F.MKV`) que le serveur FTP ne connaît pas. Le worker liste toujours le NAS en FTP et rattache chaque cible (fichier de cibles local ou `--retry-failed`) au vrai chemin de même nom ; les cibles introuvables sont écartées.
4. **Dépassement du temps limite (Watchdog)** : les vidéos 4K UHD 2160p volumineuses (5 à 7 Go en HEVC) dépassent parfois le plafond par défaut de **1800s (30 minutes)**, ajustable via `--max-extraction-secs`.

### Liste noire automatique

Une vidéo dont les **3 dernières tentatives** du journal d'événements sont des échecs n'est plus mise en file par les lancements normaux (`--fresh-only`, superviseur RPi4). Un succès remet le compteur à zéro. `--retry-failed` ignore cette liste noire : c'est le moyen de retenter ces vidéos après correction (permissions NAS, fichier remplacé).

### Exclusion manuelle

`etc/batch_exclude.txt` (sur o2switch uniquement, jamais synchronisé) liste des préfixes de chemins NAS, un par ligne, `#` pour commenter. Les vidéos concernées sont écartées de **tous** les lancements, `--retry-failed` compris. Usage type : dossiers que le compte FTP ne peut pas lire (`550` sur `RETR`) en attendant la correction des ACL dans DSM. Retirez la ligne une fois le dossier lisible.

### Procédure de Relance Ciblée

Arrêtez le travailleur distant en cours, puis relancez-le avec l'argument `--retry-failed` :

```bash
# 1. Arrêter le batch en cours
task o2switch:batch:stop

# 2. Relancer le traitement ciblé sur les échecs répertoriés
task o2switch:batch:tvshow -- --retry-failed --daemon
```

Le script lit le journal d'événements `data/reports/batch_tvshow_nightly.jsonl`, isole les vidéos en échec qui ne possèdent toujours pas d'artefact `.phash` sur disque, et lance leur calcul immédiat. Seules les vidéos de la bibliothèque lancée (`/tvshow` ou `/video`) sont retentées.

Pour ajuster le timeout maximal par vidéo (ex: 45 minutes pour de très gros fichiers 4K) :

```bash
task o2switch:batch:tvshow -- --retry-failed --max-extraction-secs 2700 --daemon
```

---

## 📊 Tableau de synthèse des commandes Taskfile

| Tâche | Description opérationnelle | Contexte d'exécution |
| :--- | :--- | :--- |
| `task o2switch:sync` | Déploie les binaires statiques, catalogues et passerelle vers o2switch | Local $\rightarrow$ o2switch |
| `task o2switch:batch:tvshow` | Lance le batch parallèle sur les séries vierges (/tvshow, 4-8 workers) | Exécution distante o2switch |
| `task o2switch:batch:movies` | Lance le batch parallèle sur les films vierges (/video, 4-8 workers) | Exécution distante o2switch |
| `task o2switch:top` | Ouvre le moniteur TUI plein écran en direct sur o2switch via SSH | Terminal interactif SSH |
| `task o2switch:pull-catalog` | Rapatrie les `.phash`, `.srt` et journaux télémétriques vers le local | o2switch $\rightarrow$ Local |
| `task coupling:certify` | Certifie et référence les nouveaux couplages dans `couplings.json` | Local |
| `task catalog:compact` | Recompacte le catalogue binaire haute densité `catalog.dbphash` | Local |
| `task o2switch:status` | Vérifie l'état du démon distant, sa consommation RAM et la santé API | Local $\rightarrow$ o2switch |
| `task o2switch:logs` | Affiche en direct les logs du démon `dbsrt-server` sur o2switch | Local $\rightarrow$ o2switch |

---

## 🔗 Documentation associée

- 💡 **Explication d'Architecture** : [Architecture du déport de calcul, stockage et synchronisation sur o2switch](../explanation/architecture-deport-calcul-stockage-et-synchronisation-o2switch.md)
- 🛠️ **Guide Interconnexion** : [Interconnecter le NAS Synology et o2switch en FTP](interconnecter-nas-synology-et-o2switch-ftp.md)
- 🛠️ **Guide Déploiement** : [Déployer dbsrt-server sur o2switch (Cloud cPanel)](deployer-o2switch-cloud-cpanel.md)
- 📖 **Référence Taskfile** : [Référence complète des cibles Taskfile](../reference/taskfile.md)
