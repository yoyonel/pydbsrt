---
title: "Tutoriel : Premiers Pas & Synchronisation VLC"
tags:

  - tutoriel
  - débutant
  - vlc
  - vlsub
---

# 🎓 Tutoriel : Premiers Pas & Synchronisation de Sous-Titres dans VLC

Dans ce tutoriel pas à pas, vous allez apprendre à démarrer l'ensemble de la pile `pydbsrt`, installer le plugin VLC et effectuer votre première synchronisation automatique de sous-titres basée sur les empreintes visuelles pHash.

> [!NOTE]
> **Temps estimé** : 5 minutes.
> **Objectif** : Vérifier que le serveur local répond, que le plugin VLC est installé et qu'un sous-titre est synchronisé avec affichage OSD.

---

## 1. Prérequis

Assurez-vous de disposer sur votre machine des outils suivants :

- Git
- Docker et Docker Compose
- Le lecteur multimédia VLC (version 3.0 ou supérieure)
- Le gestionnaire de tâches [Task](https://taskfile.dev/) (`task --version`)

---

## 2. Étape 1 : Démarrer l'Environnement de Développement

Ouvrez un terminal dans le répertoire racine du dépôt cloné et lancez la commande unifiée :

```bash
task dev
```

Cette commande orchestre automatiquement :

1. Le démarrage du conteneur PostgreSQL 17 (`bktreedb`) avec l'extension SP-GiST Hamming.
2. La certification des médias de démonstration déjà présents dans le dépôt.
3. Le démarrage du démon dbsrt en arrière-plan (port `8765`).
4. L'installation automatique des scripts d'extension VLC dans votre dossier de configuration local (`~/.local/share/vlc/lua/`).
5. Le démarrage du serveur de documentation local (port `8000`).

Vérifiez que tout est opérationnel en interrogeant le statut :

```bash
task status
```

Le terminal affiche un rapport de santé vert indiquant la disponibilité du démon HTTP, de PostgreSQL et du portail de documentation.

<div class="asciinema-wrapper">
  <div class="asciinema-cast" data-cast="../../assets/casts/term_status.cast" data-rows="26"></div>
  <figcaption>Enregistrement terminal interactif : Vérification unifiée de la stack de services (Asciinema).</figcaption>
</div>

---

## 3. Étape 2 : Lancer VLC avec un Extrait Vidéo de Démonstration

Le projet fournit des extraits vidéo test sous le répertoire `data/snippets/`.
Lancez VLC sur l'un de ces extraits via la commande Task dédiée :

```bash
task vlc:open -- data/snippets/Apprendre_CakePHP_-_SessionHelper_mid15s.mp4
```

VLC s'ouvre et commence la lecture de l'extrait.

---

## 4. Étape 3 : Déclencher la Synchronisation via VLSub dbsrt

1. Dans le menu supérieur de VLC, cliquez sur **Vue** > **VLSub (dbsrt)**.
2. Une fenêtre de dialogue s'ouvre.
3. Cliquez sur le bouton **Recherche par pHash (dbsrt)**.
4. Le plugin extrait les empreintes de l'extrait vidéo, interroge le démon local sur `http://127.0.0.1:8765/v1/phash/search`, et identifie le média de référence associé.
5. Une notification OSD (On-Screen Display) apparaît brièvement en surimpression sur la vidéo pour confirmer l'injection et le décalage temporel appliqué :

```text
[dbsrt] Sous-titre synchronisé (offset: +0ms, confiance: 100%)
```

<figure markdown="1">
  <video autoplay loop muted playsinline width="100%">
    <source src="../../assets/vlc-v2/bbb_vlc_workflow_1080p.mp4" type="video/mp4">
  </video>
  <figcaption>Démonstration vidéo 1080p : Recherche par pHash, recalage automatique et notification OSD dans VLC.</figcaption>
</figure>

---

## 5. Étape 4 : Constater le Résultat dans le Backoffice Studio

Ouvrez votre navigateur web à l'adresse suivante :

```text
http://127.0.0.1:8765/studio
```

Vous visualisez l'interface de **dbsrt Studio** :

- La liste des médias certifiés dans la base locale.
- La piste bi-temporelle associant chaque trame clé vidéo à son segment de sous-titres correspondant.

<figure markdown="1">
  <img src="../../assets/screenshots/studio_backoffice_1080p.webp" alt="Backoffice dbsrt Studio" width="100%">
  <figcaption>Interface web dbsrt Studio : Timeline bi-piste et validation relationnelle.</figcaption>
</figure>

---

## 6. Félicitations !

Vous venez de franchir votre premier cycle complet avec `pydbsrt` :

- Démarrage automatisé de la pile de services.
- Intégration transparente dans VLC.
- Alignement temporel garanti par empreinte visuelle.

Pour aller plus loin, vous pouvez suivre le tutoriel suivant :
👉 **[Ingestion & Certification d'un Premier Média](02-ingestion-et-certification-premier-media.md)**.
