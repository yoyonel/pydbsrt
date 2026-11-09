---
title: "Comment Diagnostiquer les Sources Médias & Accès Réseau Multi-Environnements"
tags:
  - howto
  - portal
  - media-sources
  - troubleshooting
  - o2switch
---

# 🛠️ Comment Diagnostiquer les Sources Médias & Accès Réseau Multi-Environnements

Ce guide pratique explique comment démarrer le portail web `pydbsrt`, contrôler la connectivité physique des conteneurs vidéo (`.mkv`, `.mp4`) et basculer entre les profils d'exécution (`local`, `rpi4`, `o2switch`).

---

## 🎯 Objectif

- Vérifier en moins d'une seconde l'accessibilité réelle des gisements multimédias (NAS Synology, Disques Freebox, Dépôt local).
- Identifier instantanément quel canal est opérationnel depuis la machine courante (Montage CIFS, Flux WAN FTP, Passerelle HTTP).
- Consulter le diagnostic depuis un navigateur local ou à distance sur le serveur cloud `o2switch`.

---

## 📋 Prérequis

- **Environnement local** : `task` installé, Python 3.11+, et virtualenv initialisé (`task sync`).
- **Environnement distant o2switch** : Pile synchronisée (`task o2switch:sync`) et token d'authentification valide provisionné dans `dbsrt/etc/api_key.secret`.

---

## 🚀 Procédure

### Étape 1 : Démarrer le Serveur Portail en Local

Pour lancer le serveur de portail léger sur le poste de travail :

```bash
task db:portal
```

Le serveur démarre sur le port `8780` et sert immédiatement l'interface :

- URL locale : `http://127.0.0.1:8780/sources.html`
- Profil auto-détecté : `💻 Poste Local (debian-latty)`

### Étape 2 : Accéder au Portail Distant Sécurisé sur o2switch

Sur le serveur cloud `o2switch`, le portail est automatiquement protégé par la passerelle de reverse-proxy PHP (`index.php`) :

1. Récupérer la clé API provisionnée sur le serveur :
   ```bash
   ssh o2switch "cat dbsrt/etc/api_key.secret"
   ```
2. Ouvrir l'URL dans le navigateur avec le paramètre token :
   ```text
   https://2026-pydbsrt.fr/dbsrt/sources?token=<VOTRE_CLE_API>
   ```
3. La passerelle vérifie la clé, dépose un cookie de session chiffré `dbsrt_session` et affiche le portail.
4. Profil auto-détecté : `🌐 Cloud o2switch` (seuls les 3 flux WAN distants sont affichés et sondés).

> [!CAUTION]
> Sans clé API valide, toute requête non authentifiée est rejetée par une erreur `HTTP/2 401 Unauthorized` ou redirigée vers le formulaire de connexion.

### Étape 3 : Commuter de Profil d'Environnement

Dans le bandeau supérieur de l'interface :

1. Repérez le sélecteur **`🎯 Profil :`**.
2. Par défaut, le profil correspond à la machine hôte (`o2switch` sur le cloud, `local` sur votre poste).
3. Cliquez sur la liste déroulante pour commuter :
   - Choisissez **`🌐 Cloud o2switch`** pour masquer les montages locaux et n'inspecter que les flux distants WAN.
   - Choisissez **`💻 Poste Local`** pour afficher les montages CIFS directs et le dépôt de snippets.
   - Choisissez **`🔍 Diagnostic Brut (Tous)`** pour inspecter et tester l'exhaustivité des 12 canaux déclarés.
4. Le compteur d'en-tête et les cartes s'ajustent instantanément sans recharger la page.

### Étape 4 : Interpréter les Résultats et l'Accordéon Réactif

- **Badge Vert (`Accessible 🟢`)** : Le protocole répond nominalement, la latence est mesurée et le nombre réel de conteneurs vidéo détectés est affiché (ex: `541 fichiers réels`). Le bloc reste grand ouvert.
- **Badge Rouge (`Inaccessible 🔴`)** : Le point de montage est absent, le serveur FTP est injoignable ou la passerelle HTTP est hors ligne.
- **Auto-Collapse** : Si tous les canaux d'un gisement sont inaccessibles, le bloc se replie automatiquement avec le badge `Tous canaux inaccessibles 🔴`. Cliquez sur l'en-tête pour le déplier à la demande.

---

## 🔍 Vérification en Ligne de Commande (Mode Headless)

Il est possible de tester unitairement une sonde sans ouvrir de navigateur :

```bash
# Tester le flux FTP WAN du NAS Synology
python3 scripts/portal_server.py --probe synology-video wan_ftp

# Tester la passerelle HTTP Freebox
python3 scripts/portal_server.py --probe freebox-disks wan_http

# Exporter la structure des métadonnées et profils déclarés
python3 scripts/portal_server.py --metadata
```
