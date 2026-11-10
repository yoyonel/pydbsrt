# Trajectoires Spatio-temporelles : Modèle de Données, KPIs & Invariants

## 1. Contexte & Problématique Métier

La synchronisation de sous-titres basée sur les empreintes perceptuelles visuelles (`pHash`) nécessite une reconnaissance formelle entre deux flux vidéo (une référence certifiée et une cible inconnue ou candidate).

Une reconnaissance sur une seule fenêtre temporelle isolée (ex. 10 à 20 secondes) présente deux limites majeures :

* **Risque de faux positif ponctuel** : des scènes très sombres, des cartons d'ouverture ou des génériques peuvent présenter une proximité pHash artificielle avec d'autres œuvres.
* **Cécité face aux divergences de cadence (PAL speedup) et de montage** : un échantillon unique permet de calculer un décalage instantané $t_0$, mais ne peut pas déterminer si la vidéo tourne à 23.976 fps, 24.0 fps ou 25.0 fps, ni si une coupure publicitaire ou une scène coupée décale la suite du film.

L'analyse par **trajectoire spatio-temporelle** résout ce problème en échantillonnant la vidéo cible sur plusieurs coupures distinctes réparties le long de sa durée, puis en arbitrant un consensus mathématique.

---

## 2. Modèle des Données Manipulées

### 2.1. Données d'Entrée (Requête / Cible)

Pour chaque vidéo auditée :

* `target_video` : Chemin du conteneur vidéo cible évalué.
* `target_moviehash` : Empreinte OpenSubtitles 64-bit hexadécimale du fichier vidéo.
* `target_duration_s` : Durée totale mesurée ou estimée du conteneur (secondes).
* `window_s` : Durée temporelle de chaque tranche analysée (ex. 20s).
* `cuts` : Nombre total de tranches échantillonnées le long de la timeline (ex. $N = 3$ coupures réparties à 10%, 50%, 80%).

### 2.2. Observations Brutes par Tranche (Processing Outputs)

Pour chaque coupure $k \in [1, N]$ :

* `timestamp_s` : Position temporelle dans la cible ($t_{\text{target}}$).
* `slice_index` : Rang ordinal de la coupure ($1, 2, \dots, N$).
* `candidates` : Liste des médias du catalogue ayant matché, avec :
    * `media_id` : Identifiant unique de l'actif média de référence dans PostgreSQL / Zero-DB.
    * `media_title` : Titre et release du média de référence.
    * `matched_frames` : Nombre de trames pHash dont la distance de Hamming est inférieure au seuil critique ($H \le 12$).
    * `total_frames` : Nombre total de trames échantillonnées dans la fenêtre.
    * `score` : Ratio de confiance local $\frac{\text{matched\_frames}}{\text{total\_frames}} \in [0.0, 1.0]$.
    * `avg_distance` : Distance moyenne de Hamming sur les trames matchées.
    * `offset_ms` : Décalage d'alignement estimé sur cette tranche ($t_0^{(k)} = t_{\text{ref\_frame}} - t_{\text{target\_frame}}$).

### 2.3. Consensus Spatio-temporel Arbitré (Consolidated Arbitration)

L'arbitre consolide les $N$ tranches pour produire les métriques d'évaluation :

* `elected_media` : Média de référence ayant remporté le score cumulé majoritaire sur l'ensemble des coupures.
* `drift_ms` ($\Delta t_0$) : Écart maximal mesuré entre les estimations temporelles du média élu ($\Delta t_0 = \max_{k}(t_0^{(k)}) - \min_{k}(t_0^{(k)})$).
* `status` : Verdict qualitatif du consensus :
    * **`COHERENT`** : Consensus unanime sur $\ge 80\%$ des tranches avec alignement linéaire strict ($\Delta t_0 \le 100\text{ ms}$).
    * **`CONSENSUS_WON`** : Consensus majoritaire élu, mais avec une dérive temporelle modérée ($100\text{ ms} < \Delta t_0 \le 1000\text{ ms}$) ou des tranches manquantes (ex. générique ou plan noir).
    * **`DIVERGENT`** : Médias concurrents gagnant des tranches différentes sans majorité univoque.
    * **`NO_MATCH`** : Aucune tranche n'a atteint le seuil d'admissibilité de confiance.
* `total_eval_duration_s` : Temps d'exécution de l'algorithme (latence CPU).

---

## 3. Indicateurs de Performance Métier (KPIs)

### 3.1. KPIs au Niveau Global (Audit Batch)

1. **Volume de Médias Évalués** : Nombre total de vidéos cibles testées dans le rapport.
2. **Taux de Consensus Global** : Pourcentage des cibles atteignant le consensus (`COHERENT` ou `CONSENSUS_WON`).
3. **Cohérence Temporelle Stricte** : Pourcentage des cibles reconnues affichant $\Delta t_0 \le 100\text{ ms}$.
4. **Latence Moyenne par Évaluation** : Temps moyen de calcul par vidéo cible (secondes).

### 3.2. KPIs au Niveau Unitaire (Par Vidéo Cible)

1. **Ruban des Tranches** : Échantillonnage visuel des tranches (points d'ancrage temporels et scores respectifs).
2. **Dérive Temporelle $\Delta t_0$** : Mesure de la stabilité du framerate et absence de décalage progressif.
3. **Marge de Séparation** : Différence de score entre le 1er candidat (élu) et le 2e candidat (runner-up), mesurant l'ambiguïté du choix.
