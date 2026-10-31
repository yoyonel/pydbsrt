---
title: "Explication : État & Qualité de la Reconnaissance (/v1/recognize)"
tags:
  - explication
  - recognize
  - phash
  - qualité
---

# 💡 Explication : État & Qualité de la Reconnaissance (`/v1/recognize`)

> Photographie au 2026-10-31 du moteur de reconnaissance tel que déployé sur o2switch. Ce document ne propose pas de correctif : il fixe la ligne de base à laquelle comparer les prochains. La suite est cadrée par l'ADR [Banc continu de reconnaissance](../decisions/2026-10-31-banc-continu-reconnaissance.md).

## Fonctionnement actuel

Une requête est une suite de pHash 64 bits, un par frame, extraite par le client (VLC) sur une fenêtre de quelques secondes.

1. **Recherche par frame.** Chaque hash cherche ses voisins à distance de Hamming ≤ 6 bits (`DEFAULT_RECOGNITION_SEARCH_DISTANCE`) :
   * Tier 1 : catalogue `.dbphash` en RAM, index MIH, plus le delta en RAM ;
   * Tier 2 : fichiers `.phash` non encore packés ;
   * Tier 3 : PostgreSQL, s'il est configuré (pas sur o2switch).
2. **Cohérence temporelle.** Pour chaque média candidat, on compte les frames cibles **distinctes** alignées sur un même décalage (`t_idx - q_idx`, tolérance ±2 frames). Les frames unies (noires, blanches) sont ignorées. La confiance est la part de frames cohérentes ; un média n'est retenu qu'à partir de 65 % (`MIN_TEMPORAL_COHERENCE_CONFIDENCE`).
3. **Classement** (`rank_media_matches`) : confiance décroissante, puis distance de Hamming moyenne croissante, puis titre et (saison, épisode) pour un ordre déterministe.
4. **Épisode et sous-titres.** L'épisode reconnu filtre les couplages certifiés et le `.phash` utilisé pour calculer le décalage. Voir [Identification de l'Épisode & Classement](identification-episode-et-classement-des-correspondances.md).

## Mesures

| Mesure | Résultat | Source |
|---|---|---|
| Titre juste | 93,7 % | audit 2739 requêtes, fenêtre de 10 s au milieu de chaque `.phash` |
| Épisode juste | 75,8 % (735/970) avant correctif, ≈ 93,9 % attendu après repack | même audit, plus le rejeu des 177 réponses sans épisode |
| Fenêtres statiques (< 5 % de frames distinctes) | 17,5 % d'erreur | même audit |
| Médias avec couplage français, live | 36/40 bons médias, chacun avec son sous-titre certifié | échantillon aléatoire, release `b46ead268f446619` |
| Confiance des réponses fausses | presque toujours 100 | même audit |
| Latence | non mesurée de façon systématique | banc, phase P1 |

Ces chiffres viennent de requêtes **extraites du catalogue lui-même** : même encodage, aucun bruit. C'est le meilleur cas. Une capture VLC réelle (autre encodage, recadrage, logo, autre cadence) n'est pas encore mesurée.

## Avantages

* **Rapide et sobre.** Index MIH en RAM sur un fichier mmap : pas de base de données, tient dans les limites mémoire d'un hébergement mutualisé.
* **Robuste au réencodage léger.** Le pHash tolère compression et changement de résolution ; la distance de 6 bits absorbe le bruit courant.
* **Résistant aux frames isolées.** La cohérence temporelle exige une suite de frames alignées : une image commune à deux médias ne suffit pas.
* **Déterministe.** À entrée égale, même réponse : les égalités sont départagées par distance puis par titre et épisode.
* **Épisode exploitable.** Sous-titres et décalage sont limités à l'épisode reconnu.

## Inconvénients & limitations

* **Confiance non calibrée.** Une confiance de 100 ne garantit pas la bonne réponse. Le client ne peut donc pas distinguer une réponse sûre d'une réponse douteuse, ni demander une autre fenêtre.
* **Fenêtres statiques.** Plan fixe, générique, écran noir : peu de frames distinctes, beaucoup de médias compatibles. C'est la première cause d'échec.
* **Quasi-doublons.** Récaps, épisodes frères, génériques partagés, éditions multiples d'un même film : plusieurs médias contiennent réellement les mêmes images.
* **Pas de réponse « ambiguë ».** En cas d'égalité, le moteur choisit un média au lieu de signaler le doute.
* **Hors catalogue.** Un média absent peut produire un faux positif. Le taux n'est pas mesuré (requêtes négatives prévues au banc).
* **Fraîcheur du catalogue.** Un `.phash` ajouté n'entre dans le Tier 1 qu'au repack suivant (cron toutes les 15 minutes, rechargement à chaud). D'ici là il passe par le Tier 2, plus lent.
* **Métadonnées figées au pack.** Titre, saison et épisode sont parsés à la construction du catalogue : une correction du parseur exige un repack.

## Ce qui garantit la non-régression aujourd'hui

* Tests unitaires du classement, de la cohérence et de l'épisode (`crates/dbsrt-index`, `crates/dbsrt-server`).
* Recette E2E live (`task o2switch:e2e`) sur un **jeu de requêtes fixe** gardé sur o2switch (`etc/e2e_golden.json`) : 10 requêtes simples et 5 requêtes avec couplage français. Chaque requête doit retrouver son média, son SxxEyy et, si elle est couplée, uniquement les sous-titres certifiés de sa release. Le jeu est construit une seule fois, à partir de requêtes qui passent : il détecte une régression, il ne mesure pas la qualité globale. `task o2switch:e2e:reset-golden` le reconstruit, par exemple après une purge du catalogue.

La mesure de la qualité globale (précision par strate, calibration, faux positifs, latence) relève du banc décrit dans l'ADR.
