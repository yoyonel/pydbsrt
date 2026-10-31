---
title: "Guide Pratique : Synchroniser des Sous-Titres dans VLC"
tags:

  - howto
  - vlc
  - vlsub
  - synchronisation
---

# 🛠️ Guide Pratique : Synchroniser des Sous-Titres dans VLC

Ce guide montre comment synchroniser automatiquement un sous-titre sur une vidéo en cours de lecture dans VLC, diagnostiquer un décalage temporel ou résoudre un échec de reconnaissance.

---

## Problème

Vous lisez une vidéo (épisode ou film) dans VLC dont les sous-titres sont désynchronisés (décalage constant, dérive temporelle PAL/NTSC 24 fps \( \leftrightarrow \) 25 fps, ou absence totale de sous-titres).

---

## Solution en 4 Étapes

### 1. Vérifier que la Stack Locale ou l'Appliance est Démarrée

Avant de solliciter le plugin, assurez-vous que le démon HTTP est joignable :

```bash
# Vérifier l'état du serveur local
task server:status
```

Si le serveur est éteint, démarrez-le :

```bash
task server:daemon
```

### 2. Déclencher la Recherche dans VLC

1. Dans VLC, lancez la lecture de la vidéo cible.
2. Ouvrez le menu **Vue** > **VLSub (dbsrt)**.
3. Deux modes de recherche sont disponibles :
   - **Recherche par pHash (dbsrt)** (*Recommandé*) : extrait 15 secondes de trames visuelles pour trouver le film exact sans dépendre de son nom de fichier.
   - **Recherche par nom / hash OpenSubtitles** : solution de repli si le média n'est pas encore indexé visuellement dans la base dbsrt.
4. Cliquez sur **Recherche par pHash (dbsrt)**.

### 3. Appliquer le Sous-Titre

1. La liste des résultats certifiés s'affiche avec la langue et le score de confiance.
2. Cliquez sur **Télécharger & Recaler**.
3. Le plugin télécharge la piste SRT, calcule le décalage temporel (\( \Delta t \) en millisecondes) par rapport aux trames pHash de référence, et injecte le sous-titre directement dans VLC.
4. Un message OSD confirme à l'écran :

```text
[dbsrt] Alignement appliqué : +1250ms (confiance 99.4%)
```

<figure markdown="1">
  <video autoplay loop muted playsinline width="100%">
    <source src="../../assets/vlc-v2/theboys_vlc_subtitles_1080p.mp4" type="video/mp4">
  </video>
  <figcaption>Démonstration vidéo 1080p : Recalage et rendu en direct de sous-titres synchronisés dans VLC.</figcaption>
</figure>

<figure markdown="1">
  <img src="../../assets/vlc-v2/02_search_candidates_result.png" alt="Interface VLSub dbsrt" width="100%">
  <figcaption>Interface graphique VLSub : Liste des candidats pHash et sélection du sous-titre certifié.</figcaption>
</figure>

### 4. Diagnostiquer en Cas d'Échec de Reconnaissance

Si le message OSD affiche `[dbsrt] Aucun couplage trouvé` :

1. **Le média n'est pas encore indexé** : vous devez ingérer le média de référence dans la base via la façade :
   ```bash
   task media:add -- /chemin/vers/video-reference.mkv /chemin/vers/sous-titre.srt
   ```
2. **Le port du serveur est personnalisé** : si votre serveur s'exécute sur un port non-standard (ex: `8766` ou distant sur RPi4), modifiez le paramètre d'hôte dans la configuration du plugin :
   - Fichier de configuration : `~/.config/vlc/dbsrt.conf`
   - Définir : `server_url=http://127.0.0.1:8765` ou `http://rpi4.local:8765`.
   - Pour basculer facilement entre Local, Cloud o2switch et RPi4 sans rien modifier dans VLC, consultez le guide dédié : [Comment basculer les cibles de backend VLC](basculer-cibles-vlc-local-cloud-rpi4.md).
