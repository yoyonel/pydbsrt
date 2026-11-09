---
title: "Comment Auditer les Actifs Médias & l'Intégrité pHash dans PostgreSQL"
tags:
  - howto
  - portal
  - media-assets
  - postgresql
  - phash
  - audit
---

# 🛠️ Comment Auditer les Actifs Médias & l'Intégrité pHash dans PostgreSQL

Ce guide pratique explique comment utiliser la vue **Actifs Médias (DB)** du portail web `pydbsrt` pour auditer la réconciliation entre les conteneurs physiques, les enregistrements relationnels dans PostgreSQL et les empreintes visuelles pHash sur disque.

---

## 🎯 Objectif

- Contrôler l'intégrité globale du catalogue multimédia stocké dans PostgreSQL 17 (`medias`, `frames`).
- Détecter immédiatement les médias sans empreinte pHash (`phash_status != 'complete'`) ou non localisés sur le stockage physique.
- Inspecter les métadonnées techniques détaillées (flux, moviehash, cadences, résolutions, durées) via le tiroir d'inspection.
- Diagnostiquer la disponibilité de la base relationnelle en local ou sur l'hébergement cloud distant `o2switch`.

---

## 📋 Prérequis

- **Serveur de portail actif** : `task db:portal` (en local sur `http://127.0.0.1:8780/`) ou passerelle o2switch accessible avec token d'authentification.
- **Base PostgreSQL active** :
  - En local : conteneur Docker démarré via `task db:up` (port `5433`).
  - Sur o2switch : instance PostgreSQL 9.6+ locale (`sapin.o2switch.net:5432`, base `atli6249_bktreedb`).

---

## 🚀 Procédure Pas à Pas

### Étape 1 : Accéder à la Vue « Actifs Médias (DB) »

1. Ouvrez le portail web dans votre navigateur :
   - En local : `http://127.0.0.1:8780/sources.html`
   - Sur o2switch : `https://2026-pydbsrt.fr/dbsrt/sources?token=<VOTRE_CLE_API>`
2. Dans la barre de navigation supérieure, cliquez sur le bouton à onglet **`Actifs Médias (DB)`** (badge affichant le nombre total d'enregistrements en base, ex: `2321`).
3. L'URL se synchronise automatiquement avec le fragment `#view=assets`.

> [!NOTE]
> Si la base relationnelle n'est pas joignable, une bannière ambrée explicite apparaît immédiatement, détaillant l'erreur de connexion, l'hôte testé et proposant un bouton **`🔄 Réessayer la connexion`**. En local, lancez `task db:up`. Sur o2switch, vérifiez les identifiants dans `dbsrt/etc/.pg_config`.

### Étape 2 : Analyser les 4 Cartes d'Indicateurs Clés (KPIs)

En haut de la vue, 4 cartes interactives résument la santé globale du catalogue :

| Carte KPI | Statut Métier | Signification Technique | Action Recommandée |
| :--- | :--- | :--- | :--- |
| **🎬 TOTAL EN BASE** | `PG 17` | Nombre total d'enregistrements dans la table `medias`. | Cliquez pour réinitialiser tous les filtres (`status=all`). |
| **🟢 EMPREINTÉ & LOCALISÉ** | `Nominal` | Média dont l'empreinte pHash est calculée (`phash_status = 'complete'`) et dont le conteneur physique est accessible. | État nominal de production prêt pour le retargeting. |
| **⚠️ SANS EMPREINTE** | `Chantier` | Média sans empreinte complète (`phash_status = 'pending'`, absent ou corrompu). | Lancer le worker d'extraction pHash (`task o2switch:sync` ou pipeline local). |
| **🛑 CHEMIN MANQUANT** | `Alerte` | Média sans chemin relatif vidéo (`video_rel_path IS NULL`). | Réconcilier le média avec son fichier physique ou archiver. |

Cliquez sur n'importe quelle carte pour filtrer instantanément le tableau sur ce statut.

### Étape 3 : Rechercher et Filtrer sans Perte de Contexte

La barre d'outils permet de filtrer dynamiquement le catalogue :

1. **Recherche Textuelle** : Saisissez un titre, une release ou un moviehash (ex: `Carnivàle`). La recherche s'exécute automatiquement après 300 ms de debounce.
2. **Remise à zéro automatique de la pagination** : Toute modification du texte de recherche, du gisement ou du tri réinitialise automatiquement la pagination à la **Page 1**, évitant de se retrouver bloqué sur une page vide.
3. **Clamping serveur de sécurité** : Si une URL est partagée avec une page hors limites (ex: `page=999`), l'API et le frontend corrigent automatiquement la requête vers la première page valide.

### Étape 4 : Inspecter la Fiche Technique d'un Actif (Tiroir Latéral)

Pour chaque ligne du tableau :

1. Cliquez sur le bouton **`Inspecter`** dans la colonne *Action*.
2. Un tiroir latéral rétractable s'ouvre à droite sans masquer la liste sous-jacente :
   - **Empreintes & Identifiants Uniques** : Moviehash 16 caractères hexadécimaux, MD5 de métadonnées, état de présence du fichier `.phash` binaire sur disque.
   - **Ancrage Physique & Transport Réseau** : Chemin absolu résolu sur disque local, ou canal de relais distant **Streaming WAN FTP** (Synology NAS) lorsque le montage local CIFS est éteint.
   - **Caractéristiques Multimédia** : Résolution exacte, profil d'encodage vidéo, cadence d'images (`FPS`), nombre total de frames décodées, durée formatée et horodatage de synchronisation.
3. Cliquez sur **`Fermer`** ou en dehors du tiroir pour le refermer.

### Étape 5 : Consulter le Glossaire Métier Intégré

Pour lever toute ambiguïté sur la sémantique des colonnes :

1. Cliquez sur le bouton **`💡 Aide & Définitions Métier`** en haut à droite.
2. La modale interactive affiche les définitions formelles du domaine :
   - Cycle de vie `phash_status` (`COMPLETE` vs `PENDING`).
   - Différence fondamentale entre `moviehash` (OpenSubtitles) et `media_hash`.
   - Rôle du couplage certifié Zero-DB (`couplings.json`).
3. Vous pouvez également cliquer sur l'icône **`ⓘ`** présente dans l'en-tête de la colonne *Empreinte pHash* pour ouvrir directement la rubrique dédiée.
