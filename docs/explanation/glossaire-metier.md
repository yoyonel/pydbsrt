# Glossaire métier

Les notions qui reviennent dans les PR et les comptes rendus, expliquées sans lire le code.
Chaque entrée dit **ce que c'est**, **pourquoi ça compte pour le produit** et, s'il y en a une, **la règle**.
La section « Objectif métier » d'une PR renvoie ici au lieu de réexpliquer.

## Le produit en une phrase

L'agent VLC reconnaît le film ou l'épisode en cours de lecture à partir de ses images, puis récupère les
sous-titres qui vont avec. La reconnaissance tourne sur un serveur hébergé chez o2switch.

## Agent VLC

- **Ce que c'est.** Le plugin VLC et son démon local, chez l'utilisateur. Le démon relaie la demande de
  reconnaissance vers le serveur o2switch (`/v1/recognize`).
- **Pourquoi ça compte.** C'est le seul point de contact avec l'utilisateur : si l'agent ne joint plus le
  serveur, plus aucun sous-titre.

## WAF o2switch

- **Ce que c'est.** Le pare-feu applicatif d'o2switch (PowerBoost), placé devant l'URL publique du serveur.
  Chaque requête de l'agent VLC passe d'abord par lui, et c'est lui qui décide si elle passe.
- **Pourquoi ça compte.** Il filtre le trafic qu'il juge anormal : des requêtes refusées (HTTP 429) ont déjà
  été observées. Il peut blacklister l'IP de la maison, qui est aussi l'IP de sortie de l'agent VLC. IP
  bannie = reconnaissance coupée = plus de sous-titres.
- **Règle.** Aucun appel de test, de sonde ou de rejeu vers l'URL publique de prod sans un go explicite pour
  cet appel précis. Le trafic de test consomme la tolérance du WAF sans rien apporter au produit.
- **Comment mesurer quand même.** `task o2switch:status` passe par SSH puis par le socket interne du serveur,
  jamais par l'URL publique : santé, catalogue et frames sont lisibles sans risque de ban.
- **Inconnu.** Le seuil exact qui déclenche un ban n'a pas été mesuré.

## Empreinte `.phash`

- **Ce que c'est.** L'empreinte d'un média : une valeur de 64 bits (8 octets) par image décodée. Taille du
  fichier / 8 = nombre d'images (frames).
- **Pourquoi ça compte.** Un média sans `.phash` ne peut pas être reconnu.

## Worker batch (o2switch)

- **Ce que c'est.** Le processus qui, sur o2switch, lit les médias sur le NAS, les décode et écrit leur
  `.phash`. Il traite une file de médias à extraire.
- **Pourquoi ça compte.** Son débit fixe la vitesse à laquelle le catalogue grossit. Un média qui échoue
  tard (après des minutes de décodage) fait perdre ce temps à toute la file.

## Checkpoint et reprise

- **Ce que c'est.** Pendant l'extraction, les frames déjà calculées sont sauvegardées. Si l'extraction est
  tuée, la suivante repart de là au lieu de tout redécoder.
- **Pourquoi ça compte.** Un arrêt ne coûte plus une extraction complète. Le log de chaque extraction dit
  s'il y a eu reprise (N frames), décodage complet ou checkpoint illisible : ce sont des faits, pas des
  déductions.

## Catalogue (`catalog.dbphash`)

- **Ce que c'est.** Tous les `.phash` regroupés (« packés ») en un seul fichier que le serveur charge pour
  reconnaître. Un repack automatique a lieu toutes les 15 minutes quand un nouveau `.phash` est arrivé.
- **Pourquoi ça compte.** Seul ce qui est dans le catalogue est reconnaissable.
- **KPI.** `/v1/catalog/status` donne `total_medias` (médias reconnaissables) et `total_frames` (quantité
  d'images indexées). `medias_without_frame_count` > 0 signifie un catalogue packé avant le comptage :
  `total_frames` est alors un minimum.

## Couplage certifié

- **Ce que c'est.** L'association validée entre un média et un fichier de sous-titres synchronisé.
- **Pourquoi ça compte.** Reconnaître le média ne suffit pas : c'est le couplage qui fournit les bons
  sous-titres. Détails : [Modèle relationnel & couplages certifiés](modele-relationnel-couplages-certifies.md).
