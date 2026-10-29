# Comment orchestrer les batchs de calcul pHash distants, monitorer et synchroniser les catalogues avec o2switch

Ce guide pratique détaille pas-à-pas comment déporter le calcul intensif d'empreintes visuelles (**pHash**) et l'extraction de sous-titres sur l'infrastructure Cloud mutualisée **o2switch**, monitorer les traitements en direct depuis votre terminal, et rapatrier les catalogues synchronisés de façon idempotente vers votre environnement local.

---

## 🎯 Objectifs opérationnels

1. **Calcul distant intensif** : Exécuter l'ingestion par batch des épisodes non traités (**"Vierges"**) sur les 12 vCPUs d'o2switch en ciblant $\approx 80\%$ d'utilisation (8 à 9 workers parallèles) sans saturation ni blocage CloudLinux LVE.
2. **Zéro-Copie NVMe** : Traiter les flux vidéo directement en mémoire RAM via FFmpeg et le pont réseau SFTP Synology NAS sans jamais copier de fichiers vidéo volumineux sur l'hébergement mutualisé.
3. **Supervision temps réel TUI** : Suivre l'exécution, les PIDs, le décodage, les débits réseau I/O et les prédictions d'ETA directement dans le terminal via `dbsrt top` en session SSH.
4. **Synchronisation idempotente** : Rapatrier les empreintes binaires `.phash`, les sous-titres adressés par le contenu (CAS SHA-256) et consolider le registre `couplings.json` et le catalogue haute densité `.dbphash`.

---

## 📋 Prérequis

Avant de lancer les opérations, assurez-vous d'avoir validé les points suivants :

- **Accès SSH configuré** : L'hôte `o2switch` doit être défini dans `~/.ssh/config` avec votre clé Ed25519 (`ssh o2switch "echo OK"`).
- **Accès NAS Synology** : Le port SFTP (port 22 ou personnalisé) du NAS Synology doit être accessible depuis o2switch (voir le guide [Interconnecter le NAS Synology et o2switch](interconnecter-nas-synology-et-o2switch-sftp.md)).
- **Binaires et outils déployés** : Les binaires statiques musl (`dbsrt`, `dbsrt-server`), `ffmpeg` statique et Python 3.11 doivent être déployés sur o2switch (`task o2switch:sync`).

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
task o2switch:batch:tvshow -- --workers 4 --fresh-only --daemon
```

Pour inspecter l'état du travailleur de fond (processus actif, PID, CPU, RAM, et dernières vidéos traitées) :

```bash
task o2switch:batch:status
```

Pour suivre le journal d'exécution en streaming ou interrompre le traitement :

```bash
task o2switch:batch:logs
task o2switch:batch:stop
```

Pour limiter l'exécution à un sous-ensemble d'épisodes (ex: tester 5 épisodes lors d'une phase de validation) :

```bash
task o2switch:batch:tvshow -- --workers 2 --fresh-only --limit 5
```

### Mécanisme de ciblage des ressources (Palier 80%)
- **Allocation CPU** : L'environnement o2switch dispose d'un quota CloudLinux LVE de 12 vCPUs. Le réglage `--workers 8` (ou 9) mobilise $\approx 80\%$ de la capacité CPU totale, laissant une marge de sécurité de 2 à 4 cœurs pour le reverse-proxy Apache, SSH et les démons système.
- **Empreinte RAM** : Chaque worker consomme $\approx 150\text{ à }200\text{ MiB}$ de mémoire vive (buffer FFmpeg et décompression de trame 32x32). Le total de 8 workers ($\approx 1.5\text{ GiB}$) reste largement sous le quota LVE de $12\text{ GiB}$.
- **Débit réseau I/O** : Le débit cumulé de 8 flux streaming ($\approx 8 \times 2.5\text{ MiB/s} = 20\text{ MiB/s}$) respecte scrupuleusement le plafond CloudLinux LVE fixé à $42\text{ MiB/s}$.

---

## 🌊 Recette 2 : Vérifier le streaming direct Zéro-NVMe

### Problématique
Les hébergements mutualisés disposent d'un espace disque limité et de quotas d'inodes stricts. Copier des fichiers vidéo MKV/MP4 de 1 à 4 Go sur le disque NVMe o2switch saturerait l'espace de stockage et déclencherait des pénalités d'I/O disque.

### Fonctionnement garanti
1. Le worker distant ouvre une socket SFTP vers le NAS Synology et expose un pont HTTP local éphémère (`http://127.0.0.1:8769/stream/<id>/<nom_media>.mkv`).
2. `dbsrt media:add` délègue la lecture à `ffmpeg` qui démultiplexe et décode le conteneur directement en mémoire RAM via des buffers circulaires.
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
- **En-tête de télémétrie** :
  - Jauge globale CPU (permet de vérifier visuellement l'atteinte du palier cible $\approx 80\%$).
  - Jauge globale RAM et débit réseau entrant (`net_rx_mib_s`).
- **Raccourcis de navigation** :
  - `1` : Tâches Actives
  - `2` : Médiathèque découverte & Statuts d'ingestion
  - `3` : Historique des sessions et débits consolidés
  - `4` : Aide contextuelle et glossaire des statuts
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
Dans l'onglet `[3/r] Récents` de `dbsrt top` (avec le filtre `[x FAIL (ERREURS SEULES)]`), certaines vidéos apparaissent avec le statut `x FAIL`. Vous souhaitez relancer le traitement uniquement sur ces vidéos en échec sans rescanner l'intégralité des milliers de fichiers du NAS Synology.

### Diagnostics et Causes Fréquentes d'Échec
1. **Espaces et crochets dans les dossiers FTP** : Les répertoires contenant des espaces (ex: `/tvshow/What IF/`, `/tvshow/Mr Robot/`) provoquaient une erreur de parsing d'URL dans FFmpeg (`No video frames could be extracted`). Le script applique désormais un percent-encoding systématique via `urllib.parse.quote(..., safe="/")`.
2. **Dépassement du temps limite (Watchdog 900s)** : Les vidéos 4K UHD 2160p volumineuses (5 à 7 Go en HEVC) requièrent plus de 15 minutes en décodage logiciel CPU multithread et étaient prématurément tuées par le watchdog. Le plafond par défaut est désormais relevé à **1800s (30 minutes)** via `--max-extraction-secs 1800`.

### Procédure de Relance Ciblée

Arrêtez le travailleur distant en cours, puis relancez-le avec l'argument `--retry-failed` :

```bash
# 1. Arrêter le batch en cours
task o2switch:batch:stop

# 2. Relancer le traitement ciblé sur les échecs répertoriés
task o2switch:batch:tvshow -- --retry-failed --daemon
```

Le script lit le journal d'événements `data/reports/batch_tvshow_nightly.jsonl`, isole les vidéos en échec qui ne possèdent toujours pas d'artefact `.phash` sur disque, et lance leur calcul immédiat.

Pour ajuster le timeout maximal par vidéo (ex: 45 minutes pour de très gros fichiers 4K) :

```bash
task o2switch:batch:tvshow -- --retry-failed --max-extraction-secs 2700 --daemon
```

---

## 📊 Tableau de synthèse des commandes Taskfile

| Tâche | Description opérationnelle | Contexte d'exécution |
| :--- | :--- | :--- |
| `task o2switch:sync` | Déploie les binaires statiques, catalogues et passerelle vers o2switch | Local $\rightarrow$ o2switch |
| `task o2switch:batch:tvshow` | Lance le batch parallèle sur les vidéos vierges (8-9 workers, palier 80%) | Exécution distante o2switch |
| `task o2switch:top` | Ouvre le moniteur TUI plein écran en direct sur o2switch via SSH | Terminal interactif SSH |
| `task o2switch:pull-catalog` | Rapatrie les `.phash`, `.srt` et journaux télémétriques vers le local | o2switch $\rightarrow$ Local |
| `task coupling:certify` | Certifie et référence les nouveaux couplages dans `couplings.json` | Local |
| `task catalog:compact` | Recompacte le catalogue binaire haute densité `catalog.dbphash` | Local |
| `task o2switch:status` | Vérifie l'état du démon distant, sa consommation RAM et la santé API | Local $\rightarrow$ o2switch |
| `task o2switch:logs` | Affiche en direct les logs du démon `dbsrt-server` sur o2switch | Local $\rightarrow$ o2switch |

---

## 🔗 Documentation associée

- 💡 **Explication d'Architecture** : [Architecture du déport de calcul, stockage et synchronisation sur o2switch](../explanation/architecture-deport-calcul-stockage-et-synchronisation-o2switch.md)
- 🛠️ **Guide Interconnexion** : [Interconnecter le NAS Synology et o2switch en SFTP](interconnecter-nas-synology-et-o2switch-sftp.md)
- 🛠️ **Guide Déploiement** : [Déployer dbsrt-server sur o2switch (Cloud cPanel)](deployer-o2switch-cloud-cpanel.md)
- 📖 **Référence Taskfile** : [Référence complète des cibles Taskfile](../reference/taskfile.md)
