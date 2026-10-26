---
title: "Guide Pratique : Surveiller la Stack avec Prometheus & Grafana"
tags:

  - howto
  - monitoring
  - prometheus
  - grafana
  - métriques
---

# 🛠️ Guide Pratique : Surveiller la Stack avec Prometheus & Grafana

Ce guide décrit comment accéder aux tableaux de bord de télémétrie, superviser la santé de la base de données PostgreSQL 17 et analyser les métriques de latence du moteur pHash en temps réel.

---

## 1. Accès aux Interfaces de Monitoring

Lorsque la stack est démarrée (`task dev` ou `task studio`), les services de surveillance sont accessibles sans authentification (mode développeur local) :

| Interface | URL Locale | Rôle Principal |
| :--- | :--- | :--- |
| **Grafana** | `http://127.0.0.1:3000` | Tableaux de bord visuels et alertes |
| **Prometheus** | `http://127.0.0.1:9090` | Explorateur de requêtes PromQL et statut des cibles |
| **Postgres Exporter** | `http://127.0.0.1:9187/metrics` | Métriques brutes de PostgreSQL 17 |
| **API Server Metrics** | `http://127.0.0.1:8765/metrics` | Métriques de trafic et temps de réponse Axum |

---

## 2. Tableaux de Bord Disponibles dans Grafana

Ouvrez `http://127.0.0.1:3000` dans votre navigateur. Les dashboards pré-provisionnés apparaissent dans le menu **Dashboards** :

1. **`dbsrt — Moteur pHash & Recherche`** :
   - Débit de requêtes par seconde (RPS).
   - Distribution des latences au quantile \( p95 \) et \( p99 \) (typiquement \( < 5\text{ ms} \)).
   - Taux de correspondances trouvées vs échecs.
   - Empreinte mémoire résidente (RSS) du serveur Axum.
2. **`PostgreSQL 17 — Performance & SP-GiST`** :
   - Nombre de connexions actives.
   - Efficacité du cache de blocs (Buffer Cache Hit Ratio \( > 99\% \)).
   - Utilisation des index SP-GiST Hamming.
   - Débit des opérations transactionnelles `COPY`.

---

## 3. Requêtes PromQL Clés pour le Diagnostic

Dans l'interface Prometheus (`http://127.0.0.1:9090/graph`), vous pouvez exécuter directement les requêtes suivantes :

### Temps de Réponse Moyen de la Recherche par pHash
```promql
rate(dbsrt_search_duration_seconds_sum[1m]) / rate(dbsrt_search_duration_seconds_count[1m])
```

### Nombre de Requêtes Reçues par Endpoint HTTP
```promql
sum by (path, status) (rate(http_requests_total[5m]))
```

### Mémoire Vive Consommée par le Serveur
```promql
process_resident_memory_bytes{job="dbsrt_server"} / 1024 / 1024
```
