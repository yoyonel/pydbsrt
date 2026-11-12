# Modèle de Rapport d'Incident & Post-Mortem Blameless (Standard SRE pydbsrt)

Ce document formalise le format standard des **Rapports d'Incidents de Production (Post-Mortem)** pour l'infrastructure distribuée de `pydbsrt` (Cloud o2switch, Appliance RPi4, NAS Synology, Clients VLC).

---

## 1. Structure Normée d'un Rapport d'Incident

```markdown
# Incident Post-Mortem : [INC-YYYY-MM-DD-XX] <Titre Concis>

| Métadonnée | Valeur |
| :--- | :--- |
| **Date & Heure d'impact** | YYYY-MM-DD HH:MM:SS UTC -> YYYY-MM-DD HH:MM:SS UTC |
| **Durée totale d'interruption** | X heures Y minutes |
| **Sévérité** | P1 (Critique / Down) | P2 (Majeur / Dégradé) | P3 (Mineur / Batch bloqué) |
| **Composant(s) impacté(s)** | Ex: Batch Worker, Gateway REST, Superviseur RPi4 |
| **Auteur(s) / Lead On-Call** | @lead |
| **Statut de clôture** | Résolu / Actions préventives en cours |

---

### 1. Résumé Exécutif (Executive Summary)
*Un paragraphe de 3-5 lignes résumant l'impact métier, la cause technique et la résolution appliquée.*

---

### 2. Impact Utilisateur & Données (Impact Metrics)
- **Trafic / Ingestion perdue** : X heures de traitement stoppées, Y cibles en retard.
- **Requêtes API impactées** : Taux d'erreurs (5xx/timeouts), latence résiduelle.
- **Intégrité des données** : 0 perte de données / corruption (state CAS préservé).

---

### 3. Chronologie des Événements (Timeline UTC / CET)
- `HH:MM` : Déclenchement de l'événement racine.
- `HH:MM` : Détection (alerte superviseur ou monitoring).
- `HH:MM` : Diagnostic initial.
- `HH:MM` : Application du correctif / relance.
- `HH:MM` : Retour au statut nominal.

---

### 4. Analyse de la Cause Racine (Root Cause Analysis - 5 Whys)
1. **Pourquoi l'ingestion s'est-elle arrêtée ?** ...
2. **Pourquoi le worker a-t-il été tué ?** ...
3. **Pourquoi le superviseur n'a-t-il pas relancé le worker ?** ...
4. **Pourquoi la commande appelée n'existait pas sur le contrôleur ?** ...
5. **Pourquoi le script distant n'était-il pas à jour ?** ...

---

### 5. Ce qui a bien fonctionné vs Ce qui a failli
#### ✅ Ce qui a bien fonctionné :
- Point 1 (ex: reprise sur état `batch_queue.json`, zéro corruption d'empreinte).
- Point 2 (ex: détection du crash par le superviseur RPi4 dès le premier cycle).

#### ❌ Ce qui a failli :
- Point 1 (ex: désynchronisation entre script distant et commandes appelées).
- Point 2 (ex: concurrence excessive 5 workers simultanée avec cron de packing).

---

### 6. Actions Correctives & Préventives (Action Items)
| Priorité | Action | Assigné | Ticket / PR | Statut |
| :---: | :--- | :---: | :---: | :---: |
| **P0** | Déployer la commande `batch-all` sur RPi4 et o2switch | @lead | PR #67 | ✅ Fait |
| **P1** | Réguler la concurrence batch à 4 workers pour éviter OOM LVE | @lead | Taskfile | ✅ Fait |
| **P2** | Ajouter un check de version/capacité dans le superviseur avant restart | @lead | Issue #74 | 📋 À faire |
```

---

## 2. Application Réelle : Post-Mortem de l'Incident du 12 Novembre 2026

# Incident Post-Mortem : [INC-2026-11-12-01] Blocage de l'Auto-Heal du Batch o2switch

| Métadonnée | Valeur |
| :--- | :--- |
| **Date & Heure d'impact** | 2026-11-12 04:58 UTC (05:58 CET) $\rightarrow$ 2026-11-12 08:51 UTC (09:51 CET) |
| **Durée totale d'interruption** | 3 heures 53 minutes |
| **Sévérité** | **P3 (Batch bloqué / Service REST opérationnel)** |
| **Composant(s) impacté(s)** | `o2switch_batch_worker.py` (Worker Cloud), `rpi4_o2switch_supervisor.py` (Watchdog RPi4) |
| **Auteur(s)** | Lionel ATTY / Antigravity |
| **Statut de clôture** | **Résolu** (Batch relancé à 09:51 avec 4 workers, synchronisation réussie) |

---

### 1. Résumé Exécutif
À 05:58 CET le 12 novembre 2026, le batch d'ingestion dual-hash (`pHash` + `aHash`) sur o2switch (PID 101141) a été tué par le gestionnaire de ressources CloudLinux LVE suite à un pic de charge cumulé (5 workers FFmpeg + cron `dbsrt pack` de 06:00). Le superviseur autonome 24/7 sur Raspberry Pi 4 a immédiatement détecté le crash (`state: crashed`), mais ses tentatives de redémarrage automatique ont échoué en boucle avec l'erreur `Usage: scripts/o2switch_ctl.sh` car la sous-commande `batch-all` n'était pas encore déployée sur le script distant. Après synchronisation du script et lancement via `task o2switch:batch:all --workers 4`, le batch a repris nominalement à 09:51 sans perte de progression (reprise à chaud sur le fichier d'état `run/batch_queue.json`).

---

### 2. Impact Utilisateur & Données
- **Ingestion suspendue** : 3h53 d'inactivité nocturne du worker batch (environ 60 à 80 épisodes non traités sur ce créneau).
- **Service REST de production** : **0 impact**, le serveur `dbsrt-server` (socket Unix + proxy PHP) est resté 100% disponible pour les plugins VLC.
- **Intégrité des données** : **100% préservée**. Les 530 fichiers `.ahash` et 3 224 `.phash` générés avant 05:58 sont intègres et validés.

---

### 3. Chronologie des Événements (Heures CET)
- **05:58:58** : Lancement simultané des jobs 476 à 480 (*Mr Robot S04*).
- **05:59:00** : Déclenchement de la crontab o2switch `dbsrt pack` (compactage des 3 224 `.phash`).
- **05:59:02** : CloudLinux LVE tue le groupe de processus du worker PID 101141 (`SIGKILL` immédiat).
- **09:08:37** : Le superviseur RPi4 détecte l'état `crashed` et tente d'exécuter `scripts/o2switch_ctl.sh batch-all --workers 5`.
- **09:08:37** : Échec immédiat : `Usage: scripts/o2switch_ctl.sh {...|batch-tvshow|batch-movies|...}` (commande inconnue sur l'ancienne version distante).
- **09:41:00** : Diagnostic de la désynchronisation du script `o2switch_ctl.sh`.
- **09:50:57** : Synchronisation du script du batch worker (`task o2switch:batch:sync`).
- **09:51:24** : Relance du batch via `task o2switch:batch:all -- --daemon --workers 4` (nouveau PID 725026).
- **09:51:34** : Vérification nominale (`BATCH_STATE=running`, 4 workers actifs, traitement en cours).

---

### 4. Analyse de la Cause Racine (5 Whys)
1. **Pourquoi l'ingestion a-t-elle été interrompue pendant 3h53 ?**
   Le worker s'est arrêté et le superviseur RPi4 n'a pas pu le relancer automatiquement.
2. **Pourquoi le superviseur RPi4 n'a pas pu relancer le worker ?**
   Il a invoqué `o2switch_ctl.sh batch-all`, rejeté par le script distant comme argument inconnu.
3. **Pourquoi l'argument était-il inconnu sur le script distant ?**
   La sous-commande `batch-all` a été introduite dans notre branche de développement locale (`feat/65-ahash-audio-fingerprint`) mais n'avait pas encore été déployée sur o2switch et RPi4.
4. **Pourquoi le worker initial s'est-il arrêté à 05:58 ?**
   Il a reçu un `SIGKILL` du kernel/LVE pour dépassement des plafonds CPU/NPROC suite au cumul de 5 workers FFmpeg et du cron de packing catalog de 06:00.
5. **Pourquoi la concurrence était-elle à 5 workers ?**
   L'autoscaler du superviseur avait augmenté la concurrence à 5 sans prendre en compte la fenêtre horaire du cron lourd de packing `dbsrt pack`.

---

### 5. Ce qui a bien fonctionné vs Ce qui a failli
#### ✅ Ce qui a bien fonctionné :
- Le fichier d'état `run/batch_queue.json` a persisté l'état exact (480 terminés, 30 échecs, 1 974 restants) : zéro retraitement superflu, reprise instantanée.
- Zéro impact sur le service en ligne `dbsrt-server` (isolation stricte par process et cgroups LVE).
- Détection continue du superviseur RPi4 (cycle de probing à 60s respecté).

#### ❌ Ce qui a failli :
- Rupture de contrat d'interface CLI entre le superviseur RPi4 et le script `o2switch_ctl.sh` non synchronisé.
- Concurrence excessive (5 workers) pendant les fenêtres de maintenance cron (`dbsrt pack`).

---

### 6. Plan d'Actions Préventives (Action Items)

| Priorité | Action Préventive | Assigné | Statut |
| :---: | :--- | :---: | :---: |
| **P0** | Déployer le nouveau `o2switch_ctl.sh` avec support unifié `batch-all` | Assistant | ✅ Fait |
| **P0** | Relancer le batch en production avec budget sécurisé (4 workers) | Assistant | ✅ Fait (PID 725026) |
| **P1** | Réguler le superviseur pour capter à 3-4 workers max pendant la fenêtre cron de 06:00 | @lead | 📋 À planifier |
| **P1** | Programmer une surveillance active toutes les 10 minutes | Assistant | ✅ Planifié (/schedule) |
