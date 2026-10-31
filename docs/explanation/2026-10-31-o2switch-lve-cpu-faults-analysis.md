# Analyse des fautes LVE CPU sur o2switch (cPanel + logs applicatifs)

> **Date** : 2026-10-31 (journal ouvert : compléter les sections datées au fil des mesures).
> **Question** : qu'est-ce qui déclenche les « faults » CPU LVE du compte o2switch pendant les batchs pHash, et quel réglage les évite sans perdre de débit ?
> **Périmètre** : journée du 2026-10-30, 00:00 à 12:07 (heure cPanel), 727 minutes, 104 minutes en faute, 178 fautes CPU.

## 1. Sources de données

| Source | Contenu | Granularité | Limite |
|---|---|---|---|
| cPanel « Resource Usage » (copié-collé du tableau) | par minute : SPEED (CPU), PMEM, IO, EP, NPROC, IOPS — moyenne, limite, nombre de fautes | 1 min | seule source des fautes ; pas d'export, pas d'historique au-delà de l'interface |
| API `/v1/monitoring/top` (historique des jobs) | début/fin, durée, taille, débit de lecture NAS, succès de chaque job `media:add` | par job | 2 420 jobs récupérés par pages de 500 (421 intermittents, d'où des retries) |
| `dbsrt/logs/batch_worker.log` (o2switch) | lancement, cibles, mode retry, scaling, sortie de chaque run du batch worker | événement | contient des chemins de médias : **ne jamais recopier** |
| `dbsrt/logs/rpi4_supervisor.log` (RPi4) | CPU LVE échantillonné toutes les 60 s, décisions d'autoscale | 60 s | moyenne glissante sur 3 échantillons |
| `/proc` depuis CageFS | `stat`, `schedstat` des processus du compte | 1 s | `/proc/lve`, cgroups et `lve_suwrapper` inaccessibles ; uapi `ResourceUsage get_usages` ne donne que l'instant présent |

**Alignement horaire.** cPanel affiche l'heure locale **CET = UTC+1**. Vérifié sur l'accalmie 03:38–04:04 (vitesse ≈ 0 %) : 0 job actif avec un décalage de +1 h, des jobs actifs avec 0 h ou +2 h.

**Données personnelles (RGPD).** Les logs du batch et l'historique des jobs contiennent des chemins de médias et le nom d'hôte du NAS ; les logs d'accès contiennent des IP clientes. Ce document ne publie que des agrégats. Les extractions brutes de travail ont été faites dans un répertoire temporaire, à supprimer en fin d'étude.

## 2. Résultats

### 2.1 Pas de seuil dur sur la moyenne minute

La limite LVE est de 12 vCPU (1 200 %). Les fautes apparaissent bien avant :

| SPEED moyenne (minute) | minutes | minutes en faute | P(faute) |
|---|---|---|---|
| 850–899 % | 64 | 0 | 0 % |
| 900–949 % | 149 | 2 | 1 % |
| 950–999 % | 209 | 20 | 10 % |
| 1 000–1 049 % | 102 | 19 | 19 % |
| 1 050–1 099 % | 16 | 7 | 44 % |
| 1 100–1 149 % | 39 | 23 | 59 % |
| ≥ 1 150 % | 34 | 32 | 94 % |

- Plus basse moyenne minute avec faute : **835 %**. Plus haute sans faute : **1 161 %**.
- Meilleur seuil unique ≈ 1 050 % (F1 0,64) : la moyenne minute n'explique qu'une partie du phénomène.
- **Inférence** : la faute naît de rafales sous la seconde au-dessus de 12 cœurs, que la moyenne minute lisse. Une faute CPU est un *throttling* (bridage), pas un kill de processus.

### 2.2 Même moyenne, issue différente

| Fenêtre (cPanel) | SPEED moyenne | minutes en faute | fautes |
|---|---|---|---|
| 00:00–01:30 | 971 % | 0 | 0 |
| 05:00–06:00 | 916 % | 0 | 0 |
| 09:38–10:18 | 992 % | 19 | 19 |
| 01:31–03:31 | 1 072 % | 64 | 137 |

00:00–01:30 et 09:38–10:18 ont la même moyenne à 2 % près, mais l'une n'a aucune faute et l'autre une faute par minute. Les deux fenêtres traitent des gros fichiers au même débit NAS ; la différence n'est pas expliquée par nos données (charge d'autres processus du compte ou de l'hôte non vérifiée).

### 2.3 Le *churn* de jobs compte, pas la concurrence

- Concurrence réelle maximale : **5 jobs** (un premier calcul à « 8–24 jobs » comptait tous les jobs chevauchant la minute ; c'était un artefact).
- À charge égale (4–7 jobs touchant la minute, SPEED ≥ 950 %) : **P(faute) = 17 % avec un démarrage de job à ± 1 min, contre 9 % sans**.
- Lecture : le démarrage d'un job (ffprobe, ouverture du flux, montée en charge du décodeur) ajoute une rafale CPU au-dessus du régime établi.

### 2.4 Origine de la rafale 01:31–03:31

- 807 jobs démarrés en 2 h (≈ 7 démarrages/min), tous réussis, sans re-traitement.
- Contenu : un seul dossier de vidéos courtes (tutoriels) ; durée médiane de job très courte, donc démarrages permanents.
- Lanceur : un run du batch worker en mode **retry des 971 échecs** (`fresh_only=False`, 6 workers initiaux, scaling dynamique), démarré le 29/10 vers 22:14 CET, terminé le 30/10 à 03:33 CET (963 succès, 8 échecs). Le run précédent s'est arrêté sans ligne de sortie (probable kill dur) puis a été relancé.
- Le cron ne lance pas de batch (`*/5` respawn serveur, `*/15` pack). Le batch est lancé et surveillé par le superviseur RPi4 (`scripts/rpi4_o2switch_supervisor.py`, unité systemd utilisateur `dbsrt-supervisor.service`).
- À ce moment, la bande d'autoscale était **78–92 % LVE (936–1 104 %)**, soit en plein dans la zone 10–44 % de P(faute) du tableau 2.1.

### 2.5 NPROC, EP, IO, PMEM : pas de corrélation

Question posée : les fautes suivent-elles le nombre de processus actifs ?

| Métrique cPanel | plage observée | limite | corrélation avec fautes/min |
|---|---|---|---|
| NPROC | 0–202 | 400 | −0,06 |
| EP (entry processes) | 0–2 | — | −0,09 |
| IO | 0–7,1 MiB/s | 42 MiB/s | 0,05 |
| PMEM | 21 Mio–2,4 Gio | 48 Gio | −0,18 |
| IOPS | 0 | 2 048 | non mesurée (toujours 0) |

- NPROC reste à 80–99 pendant 628 minutes sur 727 : le régime batch a un nombre de processus quasi constant.
- À vitesse égale, NPROC haut ou bas ne change pas P(faute) (ex. 1 000–1 049 % : 18 % sous la médiane NPROC, 19 % au-dessus).
- Variation minute à minute de NPROC : corrélation −0,02 avec les fautes.
- Aucune faute NPROC, EP, PMEM. **Une seule faute IO** : 10:55 (IO moyenne 3,55 MiB/s, donc une rafale brève vers la limite de 42 MiB/s).

**Conclusion** : les fautes sont purement CPU. Le nombre de processus n'est ni une cause ni un bon indicateur.

### 2.6 Hors sujet mais lié : les kills de `dbsrt-server`

`dbsrt-server` est tué toutes les ≈ 20,8 s et relancé par la passerelle PHP (`deploy/o2switch/index.php`, `spawn_daemon`), ce qui produit des 421 intermittents sur la passerelle. Une faute CPU bride sans tuer : **ces kills ne s'expliquent pas par les fautes LVE**.

**Cause trouvée (31/10)** : o2switch termine les process qui écoutent sur un port TCP, même en `127.0.0.1`. Un listener sur socket Unix n'est pas touché. Le démon écoute désormais sur une socket Unix. Voir l'[enquête](2026-10-31-enquete-kill-periodique-dbsrt-server-o2switch.md) et l'[ADR](../decisions/2026-10-31-dbsrt-server-unix-socket-o2switch.md).

## 3. Action prise : bande d'autoscale 60–80 %

- `AUTOSCALE_TARGET_HIGH_PCT` 92 → 80 (`scripts/rpi4_o2switch_supervisor.py`). Plafond 80 % = 960 %, sous la zone où P(faute) dépasse 10 %.
- Premier essai à 68–80 %, déployé sur le RPi4 le 2026-10-31 à 00:47 (CET) : `5 -> 4 workers` à 00:53 (moyenne 85,6 %), puis `4 -> 5` dès 00:56 (moyenne 62,0 %). Un worker vaut ≈ 2 vCPU sur 12, soit ≈ 17 points : une bande de 12 points oscille forcément entre 4 et 5 workers.
- `AUTOSCALE_TARGET_LOW_PCT` 78 → **60** : la bande (20 points) est plus large qu'un pas de worker.
- Coût attendu : environ un worker de moins en régime gros fichiers, soit au plus ≈ −20 % de débit. À confirmer avec la section 4.

## 4. Watcher de fautes (en continu)

Les fautes ne sont visibles que dans cPanel. `scripts/o2switch_fault_watch.py` enregistre sur o2switch ce que CageFS laisse voir, une ligne JSON par minute UTC dans `dbsrt/logs/faultwatch/AAAA-MM-JJ.jsonl` (rétention 14 jours) :

| Champ | Sens |
|---|---|
| `cpu_mean`, `cpu_p90`, `cpu_max` | CPU du compte en % des 12 vCPU, échantillonné chaque seconde |
| `secs_over_90`, `secs_over_100` | secondes de la minute au-dessus de 90 % / 100 % de la limite (rafales) |
| `run_delay_ms_mean`, `run_delay_ms_max` | temps d'attente ordonnanceur par seconde, sommé sur tous les threads (`/proc/<pid>/task/*/schedstat` ; le `schedstat` du pid ne couvre que le thread principal) : indicateur de bridage |
| `nproc_max`, `threads_max` | processus et threads du compte (≈ 25–31 processus et ≈ 210 threads au 31/10 ; le NPROC cPanel, ≈ 85–95, ne compte ni l'un ni l'autre) |
| `spawns`, `ffmpeg_spawns`, `dbsrt-server_spawns` | nouveaux processus : *churn* de jobs, respawns du serveur |
| `ffmpeg_max` | décodeurs simultanés |

Commandes :

```bash
task o2switch:faultwatch:start      # déploie le script et le lance dans screen dbsrt_faultwatch
task o2switch:faultwatch:status
task o2switch:faultwatch:pull       # rapatrie dans data/reports/faultwatch/ (non versionné)
task o2switch:faultwatch:correlate -- /chemin/vers/tableau_cpanel.txt
```

`correlate` joint chaque minute cPanel (CET) à la minute du watcher (UTC) et affiche, par champ, la corrélation avec les fautes et P(faute) par quartile. Le tableau cPanel se copie tel quel depuis l'interface (une cellule par ligne).

Le watcher consomme ≈ 0,5 % d'un cœur.

## 5. Un seul média à la fois avec tous les threads ?

Question : décoder un seul média avec 10–12 threads ffmpeg plutôt que 4–5 médias à `-threads 2` donnerait-il plus de débit ?

Bench local (12 cœurs logiques, vidéo synthétique 1080p H.264 bruitée de 60 s, `-vf scale=32:32 -f null -`, même commande que le batch) :

| Configuration | fps agrégé | threads ffmpeg |
|---|---|---|
| 1 média, `-threads 1` | 25,8 | 1 |
| 1 média, `-threads 2` | 43,1 | 2 |
| 1 média, `-threads 6` | 79,3 | 6 |
| 1 média, `-threads 8` | 89,4 | 8 |
| 1 média, `-threads 12` | 99,1 | 12 |
| 3 médias × `-threads 2` | 79,4 | 6 |
| 5 médias × `-threads 2` | 100,5 | 10 |
| 6 médias × `-threads 2` | 103,1 | 12 |

- À threads égaux, le décodage pur donne le même débit (6 threads : 79 fps dans les deux cas) ; à budget égal de 10 threads, 5 × 2 fait mieux que 1 × 8–12.
- Le mono-média perd sur tout ce qui n'est pas du décodage : ffprobe, ouverture FTP, sous-titres, certification et écriture d'index de chaque job laissent le CPU inactif. Sur la rafale de vidéos courtes du 30/10 (≈ 7 démarrages/min), ce temps mort domine.
- Le nombre de threads est figé au lancement de ffmpeg : l'autoscale ne pourrait agir qu'entre deux médias (un film dure plus d'une heure), au lieu de pas de 1/N du CPU aujourd'hui.
- Un seul ffmpeg à 12 threads vise exactement la limite LVE, avec des rafales synchronisées (frame threading) : c'est le profil qui génère des fautes.
- Gain réel : simplicité (pas de concurrence, une seule connexion NAS).

**Conclusion** : pas de gain de débit, perte probable sur les médias courts et plus de risque de fautes. Le levier pertinent reste celui de `2026-10-29-analyse-bottlenecks-batch-o2switch-et-plan-benchs.md` (levier 2 : moins de threads par job, plus de jobs). Limite : bench local, pas sur o2switch ni sur du HEVC/4K réel.

## 6. Questions ouvertes

1. 09:38–10:18 contre 00:00–01:30 : même moyenne, fautes très différentes. Le watcher (`secs_over_100`, `run_delay`) doit dire si ce sont des rafales du compte ou un hôte plus chargé.
2. Les ≈ 93 fautes du 30/10 après 12:07 ne sont pas analysées (hors du tableau collé).
3. Faut-il lisser les démarrages de jobs (espacement minimal entre deux lancements) plutôt que baisser encore la bande ?
4. ~~Les kills de `dbsrt-server` toutes les ≈ 20,8 s (section 2.6).~~ Résolu : listener TCP tué par o2switch, passage en socket Unix (section 2.6).
