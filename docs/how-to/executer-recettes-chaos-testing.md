---
title: "Guide Pratique : Exécuter les Recettes de Chaos Testing"
tags:

  - howto
  - test
  - chaos
  - résilience
  - infra
---

# 🛠️ Guide Pratique : Exécuter les Recettes de Chaos Testing & Tests d'Infra

Ce guide montre comment lancer la suite d'épreuves de robustesse et de chaos testing pour valider que le serveur `dbsrt-server` et le plugin VLC résistent aux pannes réseau, aux coupures brutales et aux timeouts sans geler l'interface graphique.

---

## Problème

En environnement réel (réseau Wi-Fi domestique instable, mise en veille du PC hôte, saturation CPU), les appels réseau vers le serveur de synchronisation peuvent échouer brutalement. Le système doit garantir une dégradation gracieuse sans jamais faire planter ou geler VLC.

---

## Commandes Opérationnelles

### 1. Exécuter la Suite Complète des Tests d'Infrastructure

Lancez le harnais de tests d'infrastructure hermétiques :

```bash
task test:infra
```

Ce harnais teste :

- L'isolation des processus et des fichiers PID (`/tmp/dbsrt-server.pid`).
- Le comportement face à la corruption intentionnelle du fichier de couplages `couplings.json`.
- La reconnexion automatique du pool PostgreSQL après un redémarrage forcé du conteneur.

### 2. Simuler une Coupure Brutale du Démon Pendant une Requête VLC

Pour vérifier que VLC ne se fige pas lorsque le démon s'arrête en plein traitement :

```bash
# Lancer le scénario de rupture de socket
uv run pytest tests/test_vlc_chaos.py -k test_server_abrupt_kill
```

Assertion vérifiée : le client Lua reçoit un timeout franc sous 500 ms et libère immédiatement le thread UI de VLC avec notification d'avertissement.

### 3. Injecter des Payloads Corrompus (Fuzzing Léger)

Pour s'assurer que le parseur pHash rejette les données tronquées sans panic en mémoire :

```bash
uv run pytest tests/test_vlc_chaos.py -k test_malformed_phash_payload
```

Le serveur Axum renvoie un statut HTTP `400 Bad Request` sans fuite mémoire ni arrêt du processus.
