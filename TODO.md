# 2020-05-17

## Intégration d'un arbre BKTree de recherche

- Rajouter le BKTree
- Mettre en place une sauvegarde/chargement simple d'un BKTree
- Effectuer des recherches/correspondances via le BKTree
- Mettre en place un système de cache/indexage simple sur l'arbre et les méta-données associées [POC]
- Tester la mise à l'échelle (avec des données factices) de l'arbre en terme de taille et vitesse de réponse sur la recherche
- Mettre en place une DB (NoSQL ou SQL ... à voir) pour l'indexage des méta-données en lien avec l'arbre (64bits words -> index -> méta-data)

Dans un 1er temps, on place tout coté CPU et RAM => modèle de données Python/C++.
On voit par la suite (ou en même temps) pour intégrer un modèle full DB (PostGreSQL via un indexage "custom" supportant le BKTree, VPTree).
Il faudrait définir (assez rapidement) l'API minimaliste pour:
    - effectuer les recherches
    - charger les données
    - effectuer les correspondances
    - uploader de nouvelles données
    - versionning et check de validité des données, algorithmes, "processors"

Faudrait aussi mettre en place un framework de tests (fonctionnels) pour:
    - évaluer la qualité
    - la sensibilité
        + des algos
        + du stockage
    - évaluer les impacts
    - anticiper:
        + la mise à l'échelle
        + la distribution
            - des calculs
            - du stockages

# 2022-03

- [✔️] Ajouter une CI (Github Action)
- [🚧] Créer des images (docker) pour chacune des applications
- [ ] Automatiser le proocessus --> le faire faire par Github Action
  - [ ] targets multi-os en particulier pour le raspberry (armhf)
  - [ ] mettre en place un docker registry perso/private ?
  - [ ] autoriser github a pousser des images pour le projet
- [ ] Réduire les dépendances pour permettre d'avoir des images/containeurs rapides à construire/utiliser.
-
