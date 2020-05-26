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
Que rechercher, quel type de requêtes ?
Requêtes temps réel, batch, par bloc ?

## Construction de l'arbre de recherche
À partir d'un fichier de couplage entre un fichier de sous-titres et les hashs de frames correspondantes.
On peut récupérer l'ensemble des frames (fingerprints) pour ce sous-titre.
On peut alors associer pour chaque fingerprint l'id du sous-titre (relatif au média)
L'arbre n'associe que pour un noeud hash [64bits] un noeud id [64bits].
Les valeurs ne sont au plus que des indexes.
Il faut utiliser une structure de donnée jointe (une map, dictionnaire) pour enrichir le lien et lier le code/les données métier

# UseCase

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

### Réponses

a. On n'a pas assez d'informations avec la requête effectuée (de 5 secondes).
On peut catégoriser l'échec:
1. pas (ou très peu) de matchs: on ne connait pas (encore? :p) ce média
2. du "bruits": beaucoup de matchs, début de piste de matchs, etc ... implique des frames pas (assez) significatives, très présentes dans beaucoup de médias => générique, intro, outro, séquences de fondues, etc ...
3. Double, triple, ... reconnaissances significatives de fingerprints dans différents sous-titres
4. un début/une fin de reconnaissance, quelques frames/fingerprints matchés au début ou la fin: on est tombé (par malchance) sur une intersection fine d'une séquence matchable.
Soit au début d'une séquence avec quelques 1ères frames reconnues d'un sous-titre.
Soit à la fin d'une séquence avec quelques dernières frames reconnues d'un sous-titre.

1. On incite l'utilisateur à effectuer une nouvelle requête (d'un autre passage temporel du média).
Si on échoue plusieurs fois (à déterminer), on peut conclure qu'on ne connait pas ce média.
On pourrait être amener à exploiter plus d'informations que les images. Peut être analyser les méta-données associées à la requête: nom du fichier vidéo dont est issu la requête => regarder si on ne peut pas reconnaitre (par recherche sémantique) des informations audiovisuelles.
Peut être demander ou inciter l'utilisateur à rentrer/retourner plus d'informations.

2. Ces pistes de matchs ne sont assez significatives pour retourner un match précis, mais ces matchs peuvent former (si cohérents) un regroupement "médiatique". Si par exemple, c'est une séquence de générique d'une série, d'un diffuseur, d'une chaine, etc ... On devrait voir apparaitre des patterns dans les méta-données des (débuts) de sous-titres reconnues.

3. C'est une variante plus légère que 2.
Ce cas peut décrire les requêtes se trouvant par exemple en début ou fin de média, avec des passages récapitulatifs ou de prévisions sur les anciens/prochains épisodes (dans une série).
On se retrouve alors à matcher l'épisode courant possède le replay et matcher l'épisode représenter par le replay.
Ça peut aussi arriver sur des "passages souvenirs" (les épisodes "filler" dans les mangas :p)
TODO: Passage replay ne possèdant pas de sous-titres dans le "vrai" timing mais reconnu avec sous-titres (synchronisés) dans le passage originel de la séquence.
Dans ce cas, il ne faudrait pas reconnaitre les sous-tires parent du bloc de sous-titre matché,
mais (potentiellement) proposer une génération de sous-titres dans média requête (ne possèdant pas de bloc de sous-titres dans ce passage).
