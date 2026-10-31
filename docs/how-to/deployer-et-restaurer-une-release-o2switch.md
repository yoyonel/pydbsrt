---
title: Comment déployer et restaurer une release des binaires sur o2switch
tags:
  - howto
  - deployment
  - o2switch
  - release
  - rollback
---

# 🛠️ Comment Déployer et Restaurer une Release des Binaires sur o2switch

Ce guide met à jour `dbsrt-server` et `dbsrt` sur o2switch sans interrompre le démon en cours, avec retour arrière automatique si la nouvelle version ne sert pas.

---

## 🎯 Principe

Chaque déploiement envoie une **release** immuable. Un seul lien symbolique désigne la release active :

```text
~/dbsrt/
├── releases/
│   ├── 06eb1d401c9485a9/   dbsrt-server, dbsrt, SHA256SUMS, BUILD
│   └── 58363bec36010e19/
├── current  -> releases/06eb1d401c9485a9
├── previous -> releases/58363bec36010e19
└── bin/
    ├── dbsrt-server -> ../current/dbsrt-server
    └── dbsrt        -> ../current/dbsrt
```

* **Identifiant** : les 16 premiers caractères du SHA-256 de `SHA256SUMS`. Mêmes binaires, même release.
* **`BUILD`** : commit (`-dirty` si l'arbre local n'était pas propre) et branche du build.
* **Bascule atomique** : le nouveau lien est créé à côté puis renommé sur `current` (`rename(2)`). Les deux binaires changent ensemble, sans instant où `bin/` est vide ou à moitié copié.
* **Démon en cours** : il garde son binaire jusqu'au redémarrage. Le redémarrage repacke aussi le catalogue avec le nouveau `dbsrt`.
* Le premier déploiement sur un hôte installé par copie importe les binaires en place comme release précédente.

Le script distant est `deploy/o2switch/dbsrt_release.sh`. Les tâches ci-dessous le déposent dans `~/dbsrt/bin/` à chaque appel.

---

## 🚀 Déployer une nouvelle version

```bash
task o2switch:deploy
```

Étapes enchaînées :

1. Compilation musl (`o2switch:build`) et calcul de l'identifiant.
2. Envoi de la release dans `~/dbsrt/releases/<id>/` (débit limité à 20 Mo/s).
3. Vérification des sommes SHA-256 sur l'hôte, puis bascule de `current`. Une release incomplète ou altérée est refusée, sans aucun changement.
4. Redémarrage du démon (`stop` puis `start`), contrôle `/v1/health`, puis vérification que le démon exécute bien la nouvelle release (`/proc/<pid>/exe`).
5. En cas d'échec : retour automatique à la release précédente, redémarrage, code de sortie `1`.
6. En cas de succès : seules les 5 releases les plus récentes sont gardées (dont `current` et `previous`).

!!! warning "Coupure de service"
    Le redémarrage coupe le service le temps du repack du catalogue (environ une minute). La passerelle répond `503` pendant ce temps.

---

## ↩️ Revenir à la version précédente

```bash
task o2switch:rollback
```

`current` et `previous` sont échangés, puis le démon redémarre sur l'ancienne release. Relancer la commande revient à la version de départ. Une release précédente altérée est refusée.

---

## 🔍 Vérifier ce qui tourne

```bash
task o2switch:releases
```

```text
=== Releases on o2switch (* current, - previous) ===
* 06eb1d401c9485a9 e319602fd70f-dirty fix/recognize_episode_identity
- 58363bec36010e19
Running daemon(s): 06eb1d401c9485a9
```

`Running daemon(s)` doit afficher l'identifiant marqué `*`. Une valeur `unknown` signale un démon lancé depuis un binaire hors de `releases/` : redémarrez-le avec `task o2switch:restart`.

---

## ✅ Valider en production

```bash
task o2switch:e2e
```

La section `== releases` de la recette contrôle que le démon exécute la release courante. Elle fait ensuite deux rollbacks pour revenir à la release de départ, avec un contrôle de santé après chacun.

!!! danger "Ne jamais copier sur `bin/`"
    `cp` ou `rsync` vers `~/dbsrt/bin/dbsrt-server` remplacerait le lien par un fichier et sortirait le binaire du suivi des releases. `task o2switch:sync` n'envoie plus les binaires : passez toujours par `task o2switch:deploy`.
