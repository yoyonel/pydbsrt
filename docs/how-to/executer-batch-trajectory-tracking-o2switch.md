# Exécuter un Batch de Tracking Spatiotemporel sur o2switch

Ce guide explique comment lancer, superviser, mettre en pause et rapatrier un batch d'évaluation spatiotemporelle multi-cuts en mode détaché et autonome sur l'hébergement o2switch.

---

## 1. Contexte & Objectif

L'algorithme de suivi de trajectoire multi-hypothèses évalue la cohérence spatiotemporelle des médias du catalogue à partir de leurs empreintes denses (`.phash`).

Afin de pouvoir auditer des milliers d'œuvres sans bloquer le poste de travail local, le runner s'exécute dans une session `screen` persistante sur o2switch (`dbsrt_trajectory`), totalement étanche aux déconnexions réseau.

---

## 2. Prérequis

- Accès SSH fonctionnel à l'hôte distant (`task o2switch:status`).
- Fichiers `.phash` synchronisés dans `data/__PHASH__/` sur o2switch.
- Release binaire active (`task o2switch:releases`).

---

## 3. Lancer le Batch Détaché (Autonomie Totale)

Pour lancer l'audit sur l'ensemble du corpus avec un échantillonnage haute résolution ($N=20$ cuts par film) à concurrence modérée (2 workers pour respecter les quotas LVE CloudLinux) :

```bash
task o2switch:trajectory:start -- --phash-dir data/__PHASH__ --cuts 20 -w 2 --jsonl audit/trajectory_overnight_cuts20.jsonl
```

Dès l'affichage du PID distant, **le poste local peut être éteint**. L'exécution continue sur le serveur.

---

## 4. Superviser l'Avancement

Pour interroger la télémétrie en direct depuis le poste local :

```bash
task o2switch:trajectory:status
```

Sortie typique :
```text
● Trajectory batch worker RUNNING (PID 1559362)
=== Telemetry (from run/trajectory_status.json) ===
{
  "total": 3081,
  "completed": 120,
  "remaining": 2961,
  "progress_pct": 3.9,
  "matched": 105,
  "pruned": 15,
  "errors": 0,
  "is_paused": false,
  "active_workers": 2,
  "updated_at": "2026-11-07T23:05:00Z"
}
```

Pour suivre le flux en temps réel :
```bash
task o2switch:trajectory:logs
```

---

## 5. Mettre en Pause et Reprendre

Si une opération de maintenance concurrente nécessite de libérer les ressources CPU :

```bash
# Pause douce (les jobs en cours finissent, aucun nouveau média n'est dépilé)
task o2switch:trajectory:pause

# Reprise immédiate
task o2switch:trajectory:resume
```

---

## 6. Arrêter le Batch (Idempotence de Reprise)

Pour interrompre proprement le worker :

```bash
task o2switch:trajectory:stop
```

Le runner capture le signal `SIGTERM` et termine les travaux en cours avant de quitter.

Pour relancer l'exécution ultérieurement sans recalculer les médias déjà enregistrés, il suffit de réexécuter la même commande de départ. Le fichier JSONL est analysé au démarrage et les médias traités sont automatiquement sautés.

---

## 7. Rapatrier les Résultats Locaux

Une fois le batch terminé (ou en cours d'exécution pour analyse partielle) :

```bash
task o2switch:trajectory:pull
```

Les fichiers JSONL sont synchronisés dans le répertoire local `data/reports/audit/`.
