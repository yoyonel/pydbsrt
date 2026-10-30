---
title: "Guide Pratique : Valider l'ISO du Web-Top contre dbsrt top sur o2switch"
tags:
  - how-to
  - tui
  - web-top
  - o2switch
  - tests
---

# 🛠️ Guide Pratique : Valider l'ISO du Web-Top contre `dbsrt top` sur o2switch

Ce guide vérifie que le web-top déployé (`/dbsrt/top/`, Ratzilla WASM) affiche exactement le même écran que le `dbsrt top` natif lancé sur o2switch.

## Prérequis

- Accès SSH à l'alias `o2switch`.
- `node`, `chromium` et `tmux` installés localement.
- La clé lecture seule installée sur le serveur (`task o2switch:rotate-readonly-key`). Le harness la lit par SSH ; elle n'est jamais écrite sur disque ni affichée.

## 1. Lancer la comparaison complète

```bash
task web-top:iso
```

Le harness rejoue la même séquence de touches des deux côtés, sur deux viewports : bureau `1400x900` et mobile `412x915` (émulation smartphone). La séquence couvre la navigation entre onglets, la recherche (`/`) et le défilement (`PageDown`, `End`, `Home`).

Codes de sortie :

| Code | Signification |
|---|---|
| `0` | Toutes les étapes sont identiques |
| `1` | Au moins un écart ISO |
| `2` | Échec du harness (SSH, Chromium, CDP) |
| `3` | Écarts uniquement pendant une coupure du flux web (infra, pas ISO) |

## 2. Restreindre la comparaison

```bash
# Un seul viewport
ISO_VIEWPORTS=412x915 task web-top:iso

# Une séquence de touches ciblée (texte multi-caractères = saisie de recherche)
ISO_STEPS="3,/,the,Enter,Esc" ISO_OUT_DIR=target/top_iso_rerun task web-top:iso
```

## 3. Lire un écart

Chaque étape produit trois fichiers dans `target/top_iso/` (ou `ISO_OUT_DIR`) :

- `<viewport>_<n>_<touche>.native.txt` : écran natif brut.
- `<viewport>_<n>_<touche>.web.txt` : écran web brut.
- `<viewport>_<n>_<touche>.diff.txt` : lignes différentes après masquage des champs volatils.

Les champs volatils (dates, débits, pourcentages, durées, barres) sont masqués par la liste `VOLATILE` de `scripts/top_iso_compare.mjs`.

## 4. Distinguer un vrai écart d'une course de données

Un écart limité à l'en-tête (`Workers: 0` contre `Workers: 1`, `Batch: 15/15` contre `14/15`) signifie qu'un batch a démarré ou fini entre les deux captures. Relancez les étapes concernées quand aucun batch ne tourne :

```bash
ssh o2switch 'pgrep -u "$(whoami)" -fc "dbsrt (batch|ingest|scan)"'
```

Un écart de mise en page, de libellé ou de contenu de liste est un vrai défaut ISO. Ne l'ajoutez pas à `VOLATILE` : corrigez le rendu.

## 5. Nettoyer

Le harness quitte le `dbsrt top` distant à la fin (`Esc`, `q`, puis fermeture de la session tmux locale `dbsrt_top_iso`). Après une interruption, vérifiez qu'aucun processus ne reste :

```bash
ssh o2switch 'pgrep -u "$(whoami)" -af "dbsrt top"'
```
