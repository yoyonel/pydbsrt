# Analyse des données

## Les entrées
- video
- sous titres

## Extractions d'informations
de la vidéo, on peut extraire:
- les frames -> fingerprints/hashes
- meta data de la vidéo -> framerate
```shell script
2020-05-21 18:01:19,565 - __main__ - INFO - Video reader meta data:
{'codec': 'vp8,',
 'duration': 33.0,
 'ffmpeg_version': '4.1-static https://johnvansickle.com/ffmpeg/ built with '
                   'gcc 6.3.0 (Debian 6.3.0-18+deb9u1) 20170516',
 'fps': 25.0,
 'nframes': inf,
 'pix_fmt': 'yuv420p(progressive)',
 'plugin': 'ffmpeg',
 'size': (854, 480),
 'source_size': (854, 480)}
```
des sous titres, on est extrait:
- des subripitems => séquences de sous titres dans un interval de temps

# Arbre de recherche
Comment construire l'arbre de recherche ?
Que rechercher, quel type de requetes ?


# Usecase

## Un utilisateur souhaite trouver des sous-titres pour la version du média qu'il possède.

Plusieurs cas pour ce usecase sont possibles:
	a. on ne possède pas du tout d'informations dans la base par rapport à ce média -> aucun résultat possible ... ?
	b. on a déjà rippé ce média avec cette version de vidéo (encodage) -> on renvoit directement tout le sous-titre
	c. on a un rip qui semble compatible avec ce média (fingerprints équivalentes) -> on peut (doit) ré-aligner les sous-titres sur la version du média de l'utilisateur (et enregistrer cette nouvelle version dans le système).
	Par rapport à une présence du média dans la DB, on peut soit avoir:
		- la version de sous-titres demandée (la bonne langue)
		- d'autres versions (langues)
	Dans le cas des autres versions, on peut tenter une traduction automatique

### Requêtes
Pour la requête initiale, on peut partir sur des requêtes de 5 secondes pour effectuer des recherches.
Il faudra établir une stratégie pour sélectionner (choisir) ces requêtes afin qu'elles soient aussi pertinentes que possibles.
Le cas idéal serait une passage de 5 secondes avec des dialogues (devant générer des sous-titres) et une variété suffisantes de frames significatives (pas d'écran noir, blanc ... uniformes, etc ...)

