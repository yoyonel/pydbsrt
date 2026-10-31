---
title: "Explication : Identification de l'Épisode & Classement des Correspondances"
tags:
  - explication
  - architecture
  - recognize
  - phash
---

# 💡 Explication : Identification de l'Épisode & Classement des Correspondances

## Contexte & Enjeux

`POST /v1/recognize` doit désigner **le média exact** d'une requête pHash, pas seulement son titre. Le catalogue compte de nombreuses séries : un même `clean_title` y est partagé par des dizaines d'épisodes (jusqu'à 120 médias pour un titre). Un sous-titre n'est utilisable que pour son épisode : celui d'un épisode voisin est désynchronisé de bout en bout.

Avant correction, le serveur reconnaissait le bon titre mais :

* la réponse ne portait que `media_match` (le titre), sans saison ni épisode ;
* les sous-titres certifiés proposés mélangeaient tous les épisodes du titre ;
* le décalage (`shift_ms`) pouvait être calculé sur le `.phash` d'un épisode voisin.

## Analyse du Problème & Compromis

**Égalités de confiance.** La confiance (cohérence temporelle des frames) sature à 100 % pour plusieurs médias quand la fenêtre est statique ou sombre : les mêmes hashes existent dans d'autres médias. Trier sur la seule confiance laisse alors l'ordre de la table de hachage décider, donc une réponse non déterministe.

**Frames répétées.** Une fenêtre statique produit N fois le même hash : compter les correspondances brutes gonfle la cohérence d'un média qui ne partage qu'une image. La cohérence compte donc des frames cibles **distinctes**.

**Noms des tiers.** Le Tier 1 (catalogue `.dbphash` et delta en RAM) porte des métadonnées parsées (`clean_title`, `season`, `episode`), figées au pack. Les Tiers 2/3 (fichiers `.phash`, PostgreSQL) ne renvoient que le nom de release complet : saison et épisode y sont reparsés depuis ce nom.

## L'Architecture Retenue

Ordre de classement des correspondances (`rank_media_matches`) :

1. confiance décroissante ;
2. distance de Hamming moyenne des correspondances cohérentes croissante : la source d'une requête est plus proche que les scènes similaires des autres médias ;
3. titre, puis (saison, épisode) : ordre déterministe pour les égalités exactes.

L'épisode reconnu est ensuite propagé partout :

```text
match (titre, saison, épisode)
  ├─ réponse      : media_match + season + episode
  ├─ couplages    : lookup_certified_couplings(titre, saison, épisode)
  └─ décalage     : seuls les .phash de cet épisode (les autres sont ignorés)
```

## Mesures (audit du 2026-10-31)

Fenêtre de 10 s au milieu de chaque `.phash` du catalogue (2739 requêtes) :

| Mesure | Résultat |
|---|---|
| Titre juste | 93,7 % |
| Épisode juste (requêtes d'épisode) | 735 / 970 (75,8 %) |
| Bon titre, épisode absent de la réponse | 177 |
| Bon titre, mauvais épisode | 31 |

Les 177 réponses sans épisode désignaient pourtant le bon SxxEyy : 176 venaient du Tier 1, dont le catalogue avait été packé avec des noms **percent-encodés** (`Show%20Name%20S01E02`, listings FTP/HTTP). Le parseur ne reconnaissait ni le titre ni l'épisode. `MediaMetadata::parse` décode désormais ces séquences (sauf séparateurs de chemin et octets de contrôle) ; le titre étant figé au pack, le catalogue doit être repacké avec le nouveau binaire `dbsrt`.

Après repack sur o2switch, les mêmes 177 requêtes rejouées donnent 176 réponses Tier 1 avec le bon SxxEyy, contre 0 avant. L'épisode juste attendu passe à environ (735 + 176) / 970 ≈ 93,9 %. L'audit complet n'a pas été relancé (charge LVE).

Limites connues :

* **Fenêtres statiques** : moins de 5 % de frames distinctes donne 17,5 % d'erreur ; c'est la première cause d'échec.
* **Confiance non calibrée** : la quasi-totalité des réponses fausses ont une confiance de 100. Elle ne signale pas l'erreur.
* Quasi-doublons (récaps, éditions multiples) et trous de cuts épars expliquent la plupart des autres échecs.
