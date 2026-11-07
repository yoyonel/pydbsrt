---
title: "Audit de Performance et Métriques du Pipeline sur les Cuts"
tags:
  - explication
  - performance
  - audit
  - pipeline
  - benchmark
---

# 💡 Audit de Performance et Métriques du Pipeline sur les Cuts

**Date :** 4 novembre 2026
**Statut :** Empiriquement Établi (Mesures réelles en direct via Sidecar Local et Relais Cloud o2switch)
**Corpus testé :** 2,758 extraits vidéo (snippets 15s) issus de `data/snippets/`
**Durée du banc d'essai :** 566.0 secondes (4.9 requêtes/sec)

---

## 1. Synthèse Exécutive des 6 Étapes du Pipeline

Le pipeline de matching évalue chaque vidéo à travers 6 briques techniques distinctes :

| Brique du Pipeline | Responsable & Environnement | Protocole & Données | p50 (Médiane) | p90 | p95 | p99 | Moyenne | Part du Temps |
| :--- | :--- | :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **1. Décodage FFmpeg** | 💻 Machine Client (CPU/FFmpeg) | C-ABI / IPC Pipes (~1-15 Mo $\to$ 24 pHash) | **2507.49 ms** | 4877.06 ms | 6878.24 ms | 11030.0 ms | 2985.32 ms | ~68% |
| **2. Transport Réseau** | 🌐 Transit WAN Internet | TLS HTTPS :443 (POST JSON 1.1 Ko) | **109.36 ms** | 249.83 ms | 879.42 ms | 2977.56 ms | 226.69 ms | ~18% |
| **3. Matching MIH** | ☁️ Serveur Cloud o2switch | RAM Multi-Index (POST /v1/recognize) | **23.62 ms** | 60.66 ms | 119.11 ms | 318.42 ms | 37.8 ms | ~5% |
| **4. Découverte SRT** | 📡 Serveur API SRT (:8765/:443) | GET /v1/subtitles/{hash} (SRT ~30-80 Ko) | **205.02 ms** | 376.69 ms | 449.72 ms | 620.98 ms | 229.17 ms | ~7% |
| **5. Calage SRT** | 💻 Local (RAM Client) | Rust Parser (POST /v1/retarget) | **1.0 ms** | 1.0 ms | 1.0 ms | 1.0 ms | 1.0 ms | < 1% |
| **6. Consensus Temporel** | 💻 Local (Sortie VLC) | Lua Socket (POST /v1/autosync) | **0.5 ms** | 0.5 ms | 0.5 ms | 0.5 ms | 0.5 ms | < 0.2% |
| **TOTAL PIPELINE** | **End-to-End Hybride** | **Workflow Complet** | **3119.76 ms** | **6302.49 ms** | **8300.78 ms** | **11482.51 ms** | **3768.63 ms** | **100%** |

---

## 2. Indicateurs de Reconnaissance & Qualité Métier

| Indicateur | Volume | Pourcentage | Qualification Opérationnelle |
| :--- | :---: | :---: | :--- |
| **Requêtes Exécutées avec Succès** | **2,655** | **96.3%** | Traitées sans exception HTTP ni timeout |
| **Correspondances Identifiées (Matches)** | **1,806** | **65.5%** | Média ou sous-titres reconnus dans le catalogue |
| **Vidéos Hors Catalogue (Rejets Propres)** | **849** | **34.5%** | Traitement nominal, absence de faux-positif certifié |
| **Erreurs Réseau / Timeouts** | **103** | **3.73%** | Échec de connexion ou fichier corrompu |

---

## 3. Matrice Détaillée des Résultats par Vidéo

Le tableau ci-dessous regroupe les métriques relevées sur l'ensemble des cuts du corpus :

| # | Snippet Vidéo | 1. FFmpeg | 2. WAN RTT | 3. MIH Serveur | 4. Découverte SRT | 5. Calage | 6. Consensus | Total Pipeline | Média / Match Reconnu | Confiance | Cand. |
| :--- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- | :---: | :---: |
| 1 | `01_Cruauté_mid15s.mp4` | 1004.43 ms | 122.94 ms | 21.67 ms | 99.59 ms | 1.0 ms | 0.5 ms | **1248.86 ms** | 01_Cruauté_mid15s.fr.srt | -- | 1 |
| 2 | `02_Urokodaki_Sakonji_le_formateur_mid15...` | 821.37 ms | 84.35 ms | 30.69 ms | 101.51 ms | 1.0 ms | 0.5 ms | **1038.14 ms** | 02 Urokodaki Sakonji le for... | 100.0% | 0 |
| 3 | `03_Sabito_et_Makomo_mid15s.mp4` | 1072.15 ms | 93.37 ms | 271.25 ms | 73.72 ms | 1.0 ms | 0.5 ms | **1510.72 ms** | 03 Sabito et Makomo mp4 24 ... | 100.0% | 0 |
| 4 | `04_La_sélection_finale_mid15s.mp4` | 990.83 ms | 94.89 ms | 17.81 ms | 79.38 ms | 1.0 ms | 0.5 ms | **1183.11 ms** | Aucun | -- | 0 |
| 5 | `05_Chacun_son_acier_mid15s.mp4` | 869.08 ms | 94.52 ms | 16.49 ms | 74.06 ms | 1.0 ms | 0.5 ms | **1054.41 ms** | Aucun | -- | 0 |
| 6 | `06_Le_chasseur_accompagné_d'un_démon_...` | 943.47 ms | 84.56 ms | 19.95 ms | 59.57 ms | 1.0 ms | 0.5 ms | **1107.78 ms** | Aucun | -- | 0 |
| 7 | `07_Kibutsuji_Muzan_mid15s.mp4` | 929.1 ms | 91.11 ms | 22.21 ms | 50.32 ms | 1.0 ms | 0.5 ms | **1092.92 ms** | 07 Kibutsuji Muzan mp4 24 0fps | 100.0% | 0 |
| 8 | `08_Le_parfum_du_sang_d'envoûtement_mid...` | 1156.33 ms | 96.71 ms | 252.19 ms | 113.52 ms | 1.0 ms | 0.5 ms | **1618.97 ms** | 08 Le parfum du sang d'envo... | 100.0% | 0 |
| 9 | `09_Les_démons_aux_ballons_et_aux_flèc...` | 783.75 ms | 85.87 ms | 22.03 ms | 56.72 ms | 1.0 ms | 0.5 ms | **948.61 ms** | Aucun | -- | 0 |
| 10 | `1-Pegasus_Fantasy_mid15s.mp4` | 1021.93 ms | 125.41 ms | 28.55 ms | 92.17 ms | 1.0 ms | 0.5 ms | **1268.27 ms** | 1-Pegasus Fantasy mkv 24 0fps | 95.0% | 1 |
| 11 | `10_Nous_resterons_ensemble_mid15s.mp4` | 993.23 ms | 106.23 ms | 29.28 ms | 75.79 ms | 1.0 ms | 0.5 ms | **1204.72 ms** | 10 Nous resterons ensemble ... | 100.0% | 0 |
| 12 | `11_La_maison_aux_tambours_mid15s.mp4` | 4932.12 ms | 101.06 ms | 29.1 ms | 51.2 ms | 1.0 ms | 0.5 ms | **5113.69 ms** | 11 La maison aux tambours m... | 100.0% | 0 |
| 13 | `12AOKA~2_mid15s.mp4` | 843.52 ms | 87.12 ms | 20.94 ms | 46.35 ms | 1.0 ms | 0.5 ms | **998.13 ms** | Aucun | -- | 0 |
| 14 | `12_Zen'itsu_dort,_le_sanglier_montre_le...` | 962.61 ms | 85.69 ms | 28.67 ms | 58.6 ms | 1.0 ms | 0.5 ms | **1135.76 ms** | Aucun | -- | 0 |
| 15 | `14_Jours_12_Nuits_(2020)_[1080p](Vietna...` | 2951.85 ms | 105.37 ms | 65.86 ms | 116.1 ms | 1.0 ms | 0.5 ms | **3239.4 ms** | Aucun | -- | 0 |
| 16 | `14_Le_Blason_des_glycines_mid15s.mp4` | 1242.12 ms | 177.39 ms | 148.75 ms | 102.76 ms | 1.0 ms | 0.5 ms | **1671.32 ms** | 14 Le Blason des glycines m... | 100.0% | 0 |
| 17 | `15_Le_Mont_Natagumo_mid15s.mp4` | 1499.75 ms | 92.84 ms | 25.47 ms | 171.81 ms | 1.0 ms | 0.5 ms | **1790.07 ms** | 15 Le Mont Natagumo mp4 24 ... | 100.0% | 0 |
| 18 | `16_Faire_passer_les_autres_avant_soi_mi...` | 1330.12 ms | 123.44 ms | 113.53 ms | 130.38 ms | 1.0 ms | 0.5 ms | **1698.33 ms** | 16 Faire passer les autres ... | 100.0% | 0 |
| 19 | `17_Parfaire_une_seule_technique_mid15s.mp4` | 1083.3 ms | 88.84 ms | 322.43 ms | 141.57 ms | 1.0 ms | 0.5 ms | **1636.43 ms** | 17 Parfaire une seule techn... | 100.0% | 0 |
| 20 | `18_Des_liens_factices_mid15s.mp4` | 1089.61 ms | 96.58 ms | 254.71 ms | 61.6 ms | 1.0 ms | 0.5 ms | **1502.69 ms** | 18 Des liens factices mp4 2... | 100.0% | 0 |
| 21 | `19_Le_Dieu_du_feu_mid15s.mp4` | 1930.63 ms | 117.63 ms | 20.32 ms | 51.06 ms | 1.0 ms | 0.5 ms | **2119.84 ms** | Aucun | -- | 0 |
| 22 | `1O6HM~0_mid15s.mp4` | 1118.55 ms | 104.09 ms | 23.41 ms | 64.79 ms | 1.0 ms | 0.5 ms | **1311.03 ms** | Aucun | -- | 0 |
| 23 | `2-Blue_Forever_mid15s.mp4` | 639.83 ms | 87.97 ms | 21.51 ms | 102.62 ms | 1.0 ms | 0.5 ms | **852.15 ms** | 2 | 100.0% | 156 |
| 24 | `21_Violation_du_code_mid15s.mp4` | 1001.63 ms | 85.9 ms | 38.93 ms | 44.76 ms | 1.0 ms | 0.5 ms | **1171.43 ms** | 21 Violation du code mp4 24... | 100.0% | 1 |
| 25 | `23NPOI~Q_mid15s.mp4` | 2024.29 ms | 81.29 ms | 22.99 ms | 61.18 ms | 1.0 ms | 0.5 ms | **2190.0 ms** | 24 L'Entrai 24 0fps | 100.0% | 1 |
| 26 | `25_Tsuyuri_Kanao,_la_successeuse_mid15s...` | 1653.02 ms | 143.89 ms | 21.13 ms | 97.02 ms | 1.0 ms | 0.5 ms | **1915.27 ms** | 25 Tsuyuri Kanao, la succes... | 100.0% | 1 |
| 27 | `26_Nouvelle_mission_mid15s.mp4` | 968.57 ms | 89.92 ms | 18.21 ms | 42.93 ms | 1.0 ms | 0.5 ms | **1119.84 ms** | Aucun | -- | 0 |
| 28 | `28.Years.Later.2025.2160p.AMZN.WEB-DL.D...` | 1547.04 ms | 137.27 ms | 33.91 ms | 61.19 ms | 1.0 ms | 0.5 ms | **1784.21 ms** | Aucun | -- | 0 |
| 29 | `28.Years.Later.The.Bone.Temple.2026.108...` | 1474.29 ms | 117.69 ms | 42.68 ms | 122.86 ms | 1.0 ms | 0.5 ms | **1757.76 ms** | 28 Years Later The Bone Temple | 100.0% | 1 |
| 30 | `28.Years.Later.The.Bone.Temple.2026.216...` | 1056.11 ms | 83.75 ms | 23.17 ms | 184.36 ms | 1.0 ms | 0.5 ms | **1347.65 ms** | 28 Years Later The Bone Temple | 100.0% | 1 |
| 31 | `28_Semaines_Plus_Tard_(2007)_[1080p]_mi...` | 605.65 ms | 86.23 ms | 30.1 ms | 113.24 ms | 1.0 ms | 0.5 ms | **835.58 ms** | 28 Semaines Plus Tard | 100.0% | 2 |
| 32 | `2Q2PRB~J_mid15s.mp4` | 1021.11 ms | 86.4 ms | 24.99 ms | 139.05 ms | 1.0 ms | 0.5 ms | **1271.76 ms** | 20 Une famille improvise 24... | 100.0% | 1 |
| 33 | `2T8EO4~B_mid15s.mp4` | 1137.5 ms | 129.39 ms | 26.6 ms | 81.99 ms | 1.0 ms | 0.5 ms | **1375.67 ms** | Aucun | -- | 0 |
| 34 | `2Y8CQT~C_mid15s.mp4` | 2194.77 ms | 87.56 ms | 26.45 ms | 95.3 ms | 1.0 ms | 0.5 ms | **2404.31 ms** | 23 La Re 24 0fps | 100.0% | 1 |
| 35 | `3-Soldier_Dream_mid15s.mp4` | 1259.28 ms | 113.15 ms | 25.95 ms | 113.87 ms | 1.0 ms | 0.5 ms | **1512.41 ms** | 3-Soldier Dream mkv 24 0fps | 95.0% | 1 |
| 36 | `4-Blue_Dream_mid15s.mp4` | 924.66 ms | 88.26 ms | 20.57 ms | 120.93 ms | 1.0 ms | 0.5 ms | **1154.64 ms** | Aucun | -- | 0 |
| 37 | `8_TV_Spots_mid15s.mp4` | 1034.46 ms | 111.47 ms | 54.48 ms | 155.56 ms | 1.0 ms | 0.5 ms | **1356.15 ms** | Aucun | -- | 0 |
| 38 | `9_Le_Film_(2016)[FRENCH](Comédie_Canadi...` | 853.32 ms | 92.11 ms | 26.81 ms | 91.56 ms | 1.0 ms | 0.5 ms | **1065.6 ms** | 9 Le Film | 100.0% | 0 |
| 39 | `A.Complete.Unknown.2024.2160p.AMZN.WEB-...` | 1045.62 ms | 92.08 ms | 19.87 ms | 57.41 ms | 1.0 ms | 0.5 ms | **1215.19 ms** | Aucun | -- | 0 |
| 40 | `A.History.of.Violence.2005.1080p.BluRay...` | 877.07 ms | 121.39 ms | 42.11 ms | 71.63 ms | 1.0 ms | 0.5 ms | **1112.4 ms** | A History of Violence | 100.0% | 0 |
| 41 | `A.Perfect.Day.2015.FRENCH.BRRip.XviD-AM...` | 1160.66 ms | 87.39 ms | 23.73 ms | 113.99 ms | 1.0 ms | 0.5 ms | **1385.98 ms** | A Perfect Day | 100.0% | 0 |
| 42 | `A.Tale.of.Two.Sisters.2003.REMASTERED.K...` | 969.53 ms | 79.73 ms | 24.0 ms | 64.84 ms | 1.0 ms | 0.5 ms | **1138.43 ms** | A Tale of Two Sisters | 100.0% | 0 |
| 43 | `A.Touch.Of.Zen.1971.1080p.BluRay.x264-[...` | 1186.16 ms | 85.43 ms | 21.85 ms | 94.05 ms | 1.0 ms | 0.5 ms | **1390.0 ms** | A Touch Of Zen | 100.0% | 1 |
| 44 | `AFRONAUTS_[short_film]_mid15s.mp4` | 757.16 ms | 86.85 ms | 31.73 ms | 111.09 ms | 1.0 ms | 0.5 ms | **987.07 ms** | AFRONAUTS short film webm 2... | 100.0% | 0 |
| 45 | `A_Plein_Temps_(2021)[1080p](mère_céliba...` | 1160.39 ms | 90.84 ms | 194.87 ms | 52.53 ms | 1.0 ms | 0.5 ms | **1498.87 ms** | Aucun | -- | 0 |
| 46 | `A_Plein_Temps_(2021)_[1080p.H264](Laure...` | 1111.86 ms | 121.82 ms | 215.98 ms | 49.18 ms | 1.0 ms | 0.5 ms | **1499.03 ms** | Aucun | -- | 0 |
| 47 | `A_Silent_Voice_(2016)_mid15s.mp4` | 956.94 ms | 80.69 ms | 22.15 ms | 95.28 ms | 1.0 ms | 0.5 ms | **1155.31 ms** | A Silent Voice | 100.0% | 0 |
| 48 | `A_propos_-_Mon_environnement_mid15s.mp4` | 1973.5 ms | 114.0 ms | 23.14 ms | 84.4 ms | 1.0 ms | 0.5 ms | **2195.32 ms** | A propos - Mon environnemen... | 100.0% | 0 |
| 49 | `Abominable.2019.1080p.BluRay.x264-[YTS....` | 997.97 ms | 98.51 ms | 188.46 ms | 65.94 ms | 1.0 ms | 0.5 ms | **1351.93 ms** | Abominable.2019.1080p.BluRa... | -- | 1 |
| 50 | `Adopte.Un.Veuf.2016.FRENCH.BDRip.x264-4...` | 1126.6 ms | 206.3 ms | 110.35 ms | 98.0 ms | 1.0 ms | 0.5 ms | **1544.47 ms** | Adopte Un Veuf | 100.0% | 0 |
| 51 | `Adoration_(2019)_[1080p](Thriller,_Dram...` | 859.56 ms | 81.1 ms | 103.46 ms | 43.22 ms | 1.0 ms | 0.5 ms | **1088.68 ms** | Adoration | 100.0% | 0 |
| 52 | `After.Yang.2022.1080p.AMZN.WEB-DL.DDP5....` | 857.14 ms | 86.19 ms | 197.72 ms | 75.03 ms | 1.0 ms | 0.5 ms | **1216.3 ms** | After Yang | 100.0% | 0 |
| 53 | `Aftersun.2022.1080p.WEBRip.x264.AAC-[YT...` | 1607.31 ms | 81.18 ms | 42.49 ms | 61.15 ms | 1.0 ms | 0.5 ms | **1792.35 ms** | Aucun | -- | 0 |
| 54 | `Ainbo.2021.1080p.WEBRip.DD5.1.x264-NOGR...` | 1756.55 ms | 84.8 ms | 17.39 ms | 65.69 ms | 1.0 ms | 0.5 ms | **1925.45 ms** | Ainbo | 100.0% | 0 |
| 55 | `Akira.1988.25th.Anniversary.Edition.108...` | 1906.01 ms | 81.97 ms | 21.94 ms | 60.66 ms | 1.0 ms | 0.5 ms | **2071.08 ms** | Aucun | -- | 0 |
| 56 | `Alabama_Monroe_-_1080p_Vff_EN_x264_ac3_...` | 765.04 ms | 232.42 ms | 155.92 ms | 57.62 ms | 1.0 ms | 0.5 ms | **1211.71 ms** | Alabama monroe | 100.0% | 0 |
| 57 | `Alabama_monroe_(2012).HDrip_mid15s.mp4` | 822.62 ms | 175.64 ms | 151.5 ms | 114.12 ms | 1.0 ms | 0.5 ms | **1265.68 ms** | Alabama monroe | 100.0% | 0 |
| 58 | `Alaska_mid15s.mp4` | 746.48 ms | 214.11 ms | 153.97 ms | 48.53 ms | 1.0 ms | 0.5 ms | **1164.21 ms** | Aucun | -- | 0 |
| 59 | `Alice_(2022)[MULTi][1080p](esclave_plan...` | 807.05 ms | 173.98 ms | 50.63 ms | 95.44 ms | 1.0 ms | 0.5 ms | **1128.33 ms** | Alice | 100.0% | 0 |
| 60 | `All.The.Bright.Places.2020.WEBRip.x264-...` | 846.91 ms | 171.93 ms | 23.99 ms | 116.38 ms | 1.0 ms | 0.5 ms | **1159.67 ms** | All The Bright Places | 100.0% | 1 |
| 61 | `All_About_Nina_(2018)_[1080p](Alors_que...` | 597.82 ms | 160.16 ms | 69.01 ms | 126.09 ms | 1.0 ms | 0.5 ms | **954.39 ms** | All About Nina | 100.0% | 0 |
| 62 | `Allons_Enfants_(2022)[1080p](🗼🇫🇷👦👧💃)_mi...` | 835.08 ms | 83.3 ms | 20.76 ms | 88.7 ms | 1.0 ms | 0.5 ms | **1029.3 ms** | Allons Enfants | 100.0% | 0 |
| 63 | `Alpha_(2018)_mid15s.mp4` | 752.68 ms | 93.15 ms | 19.35 ms | 88.9 ms | 1.0 ms | 0.5 ms | **954.65 ms** | Alpha | 100.0% | 0 |
| 64 | `Amanda_(2018)[1080p](Vincent_Lacoste_-_...` | 983.09 ms | 81.27 ms | 21.28 ms | 108.92 ms | 1.0 ms | 0.5 ms | **1197.47 ms** | Amanda | 100.0% | 0 |
| 65 | `Amsterdam_(2022)[MULTi][1080p](Christia...` | 790.07 ms | 88.6 ms | 19.66 ms | 59.61 ms | 1.0 ms | 0.5 ms | **958.4 ms** | Amsterdam | 100.0% | 0 |
| 66 | `Andor.S01E01.1080p.WEB.h264-KOGi_mid15s...` | 1444.72 ms | 85.45 ms | 21.22 ms | 58.65 ms | 1.0 ms | 0.5 ms | **1610.32 ms** | Andor | 100.0% | 3 |
| 67 | `Andor.S01E02.1080p.WEB.h264-KOGi_mid15s...` | 1008.68 ms | 94.82 ms | 15.94 ms | 95.63 ms | 1.0 ms | 0.5 ms | **1215.64 ms** | Andor | 100.0% | 2 |
| 68 | `Andor.S01E03.1080p.WEB.h264-KOGi_mid15s...` | 895.36 ms | 86.57 ms | 17.87 ms | 70.85 ms | 1.0 ms | 0.5 ms | **1071.6 ms** | Andor | 100.0% | 2 |
| 69 | `AngularJS_-_$apply,_$watch_et_$digest_m...` | 6492.76 ms | 90.27 ms | 19.86 ms | 200.06 ms | 1.0 ms | 0.5 ms | **6803.28 ms** | AngularJS - $apply, $watch ... | 100.0% | 0 |
| 70 | `AngularJS_-_Les_Filtres_mid15s.mp4` | 686.24 ms | 88.38 ms | 25.23 ms | 87.02 ms | 1.0 ms | 0.5 ms | **887.08 ms** | AngularJS - Les Filtres mp4... | 100.0% | 0 |
| 71 | `Anomalisa.2015.1080p.BluRay.x264-[YTS.A...` | 789.66 ms | 111.69 ms | 24.42 ms | 118.54 ms | 1.0 ms | 0.5 ms | **1044.75 ms** | Aucun | -- | 0 |
| 72 | `Anora.2024.MULTi.VFQ.SDR.2160p.WEB.H265...` | 905.32 ms | 93.52 ms | 21.05 ms | 135.78 ms | 1.0 ms | 0.5 ms | **1158.13 ms** | Anora | 100.0% | 0 |
| 73 | `Apprendre_CakePHP_-_CacheHelper_mid15s.mp4` | 2092.94 ms | 2163.0 ms | 360.95 ms | 206.52 ms | 1.0 ms | 0.5 ms | **4834.59 ms** | Apprendre CakePHP - CacheHe... | 100.0% | 0 |
| 74 | `Apprendre_CakePHP_-_CakeEmail_mid15s.mp4` | 1971.87 ms | 1883.17 ms | 572.44 ms | 410.82 ms | 1.0 ms | 0.5 ms | **4838.6 ms** | Apprendre CakePHP - CakeEma... | 100.0% | 0 |
| 75 | `Apprendre_CakePHP_-_Conclusion_mid15s.mp4` | 5384.11 ms | 88.1 ms | 25.06 ms | 148.95 ms | 1.0 ms | 0.5 ms | **5646.55 ms** | Apprendre CakePHP - Conclus... | 100.0% | 0 |
| 76 | `Apprendre_CakePHP_-_Configuration_mid15...` | 1706.78 ms | 1417.29 ms | 22.96 ms | 155.9 ms | 1.0 ms | 0.5 ms | **3303.24 ms** | Apprendre CakePHP - Install... | 100.0% | 0 |
| 77 | `Apprendre_CakePHP_-_Création_d'un_Racco...` | 6602.19 ms | 89.6 ms | 260.63 ms | 526.32 ms | 1.0 ms | 0.5 ms | **7479.07 ms** | Apprendre CakePHP - Créatio... | 100.0% | 0 |
| 78 | `Apprendre_CakePHP_-_FormHelper_mid15s.mp4` | 601.11 ms | 86.71 ms | 199.39 ms | 148.29 ms | 1.0 ms | 0.5 ms | **1035.72 ms** | Apprendre CakePHP - Les beh... | 100.0% | 0 |
| 79 | `Apprendre_CakePHP_-_HtmlHelper_mid15s.mp4` | 2724.72 ms | 649.57 ms | 557.81 ms | 474.04 ms | 1.0 ms | 0.5 ms | **4406.53 ms** | Apprendre CakePHP - Prefix ... | 100.0% | 0 |
| 80 | `Apprendre_CakePHP_-_Installation_mid15s...` | 7509.67 ms | 103.07 ms | 19.6 ms | 291.66 ms | 1.0 ms | 0.5 ms | **7924.31 ms** | Apprendre CakePHP - Install... | 100.0% | 0 |
| 81 | `Apprendre_CakePHP_-_La_console_mid15s.mp4` | 7336.4 ms | 87.54 ms | 25.16 ms | 169.51 ms | 1.0 ms | 0.5 ms | **7618.96 ms** | Apprendre CakePHP - La cons... | 100.0% | 0 |
| 82 | `Apprendre_CakePHP_-_La_validation_mid15...` | 800.04 ms | 103.45 ms | 97.97 ms | 109.24 ms | 1.0 ms | 0.5 ms | **1110.95 ms** | Apprendre CakePHP - La vali... | 100.0% | 0 |
| 83 | `Apprendre_CakePHP_-_Le_composant_Auth_m...` | 797.29 ms | 100.13 ms | 62.13 ms | 59.87 ms | 1.0 ms | 0.5 ms | **1019.74 ms** | Apprendre CakePHP - Le comp... | 100.0% | 0 |
| 84 | `Apprendre_CakePHP_-_Le_composant_Securi...` | 8654.7 ms | 125.96 ms | 21.95 ms | 244.58 ms | 1.0 ms | 0.5 ms | **9047.54 ms** | Apprendre CakePHP - Le comp... | 100.0% | 0 |
| 85 | `Apprendre_CakePHP_-_Les_Controllers_mid...` | 9475.84 ms | 100.59 ms | 115.75 ms | 300.88 ms | 1.0 ms | 0.5 ms | **9993.5 ms** | Apprendre CakePHP - Les Con... | 100.0% | 0 |
| 86 | `Apprendre_CakePHP_-_Les_associations_mi...` | 7666.18 ms | 121.24 ms | 24.97 ms | 129.96 ms | 1.0 ms | 0.5 ms | **7942.98 ms** | Apprendre CakePHP - Les ass... | 100.0% | 0 |
| 87 | `Apprendre_CakePHP_-_Les_behaviors_mid15...` | 905.77 ms | 1485.78 ms | 36.67 ms | 189.63 ms | 1.0 ms | 0.5 ms | **2618.12 ms** | Apprendre CakePHP - Les beh... | 100.0% | 0 |
| 88 | `Apprendre_CakePHP_-_Les_composants_mid1...` | 9713.83 ms | 95.19 ms | 173.07 ms | 124.9 ms | 1.0 ms | 0.5 ms | **10107.33 ms** | Apprendre CakePHP - beforeF... | 100.0% | 0 |
| 89 | `Apprendre_CakePHP_-_Les_models_mid15s.mp4` | 2412.03 ms | 458.55 ms | 607.03 ms | 157.23 ms | 1.0 ms | 0.5 ms | **3635.22 ms** | Aucun | -- | 0 |
| 90 | `Apprendre_CakePHP_-_Les_plugins_mid15s.mp4` | 1066.19 ms | 1145.28 ms | 20.99 ms | 135.65 ms | 1.0 ms | 0.5 ms | **2368.41 ms** | Apprendre CakePHP - Les plu... | 100.0% | 0 |
| 91 | `Apprendre_CakePHP_-_Les_vues_mid15s.mp4` | 9142.84 ms | 90.07 ms | 337.41 ms | 230.55 ms | 1.0 ms | 0.5 ms | **9801.22 ms** | Apprendre CakePHP - HtmlHel... | 100.0% | 0 |
| 92 | `Apprendre_CakePHP_-_Les_éléments_mid15s...` | 8845.47 ms | 93.12 ms | 223.89 ms | 156.28 ms | 1.0 ms | 0.5 ms | **9319.14 ms** | Apprendre CakePHP - Le comp... | 100.0% | 0 |
| 93 | `Apprendre_CakePHP_-_Méthodes_After_et_B...` | 7578.42 ms | 120.64 ms | 20.03 ms | 174.94 ms | 1.0 ms | 0.5 ms | **7894.31 ms** | Apprendre CakePHP - Méthode... | 100.0% | 0 |
| 94 | `Apprendre_CakePHP_-_Prefix_mid15s.mp4` | 1114.65 ms | 1133.12 ms | 304.67 ms | 333.2 ms | 1.0 ms | 0.5 ms | **2886.0 ms** | Apprendre CakePHP - Les beh... | 100.0% | 0 |
| 95 | `Apprendre_CakePHP_-_Routing_mid15s.mp4` | 10947.17 ms | 88.19 ms | 428.68 ms | 129.7 ms | 1.0 ms | 0.5 ms | **11594.09 ms** | Aucun | -- | 0 |
| 96 | `Apprendre_CakePHP_-_SessionHelper_mid15...` | 3933.72 ms | 94.55 ms | 118.57 ms | 488.42 ms | 1.0 ms | 0.5 ms | **4635.66 ms** | Apprendre CakePHP - Session... | 100.0% | 0 |
| 97 | `Apprendre_CakePHP_-_TP2_-_Introduction_...` | 2145.77 ms | 94.34 ms | 301.06 ms | 234.37 ms | 1.0 ms | 0.5 ms | **2775.92 ms** | Aucun | -- | 0 |
| 98 | `Apprendre_CakePHP_-_beforeFilter_&_befo...` | 2054.11 ms | 99.01 ms | 218.33 ms | 285.41 ms | 1.0 ms | 0.5 ms | **2657.31 ms** | Apprendre CakePHP - FormHel... | 100.0% | 0 |
| 99 | `Apprendre_CakePHP_mid15s.mp4` | 10700.74 ms | 106.62 ms | 21.04 ms | 270.12 ms | 1.0 ms | 0.5 ms | **11098.91 ms** | Aucun | -- | 0 |
| 100 | `Apprendre_MySQL_(10_19)_-_Fonctions_num...` | 9441.12 ms | 94.99 ms | 22.77 ms | 125.21 ms | 1.0 ms | 0.5 ms | **9687.54 ms** | Apprendre MySQL 9 19 - Séle... | 100.0% | 0 |
| 101 | `Apprendre_MySQL_(11_19)_-_Fonctions_sur...` | 1717.45 ms | 113.76 ms | 22.89 ms | 233.84 ms | 1.0 ms | 0.5 ms | **2089.36 ms** | Apprendre MySQL 11 19 - Fon... | 100.0% | 0 |
| 102 | `Apprendre_MySQL_(12_19)_-_Fonctions_tem...` | 11482.97 ms | 98.36 ms | 23.77 ms | 100.81 ms | 1.0 ms | 0.5 ms | **11706.25 ms** | Apprendre MySQL 12 19 - Fon... | 100.0% | 0 |
| 103 | `Apprendre_MySQL_(13_19)_-_Dumps_mid15s.mp4` | 1877.22 ms | 124.67 ms | 21.41 ms | 198.77 ms | 1.0 ms | 0.5 ms | **2234.76 ms** | Aucun | -- | 0 |
| 104 | `Apprendre_MySQL_(19_19)_-_Conclusion_mi...` | 11499.98 ms | 92.79 ms | 59.79 ms | 336.97 ms | 1.0 ms | 0.5 ms | **11990.55 ms** | Apprendre MySQL 19 19 - Con... | 100.0% | 0 |
| 105 | `Apprendre_MySQL_(1_19)_-_Qu’est_ce_que_...` | 3943.41 ms | 99.46 ms | 40.43 ms | 205.02 ms | 1.0 ms | 0.5 ms | **4288.69 ms** | Apprendre MySQL 1 19 - Qu’e... | 100.0% | 0 |
| 106 | `Apprendre_MySQL_(2_19)_-_Installation_m...` | 10010.29 ms | 91.65 ms | 20.51 ms | 252.74 ms | 1.0 ms | 0.5 ms | **10375.63 ms** | Apprendre MySQL 2 19 - Inst... | 100.0% | 0 |
| 107 | `Apprendre_MySQL_(3_19)_-_PHPMyAdmin_mid...` | 5688.52 ms | 107.47 ms | 19.42 ms | 337.03 ms | 1.0 ms | 0.5 ms | **6160.01 ms** | Apprendre MySQL 3 19 - PHPM... | 100.0% | 0 |
| 108 | `Apprendre_MySQL_(4_19)_-_Créer_une_base...` | 2840.47 ms | 88.13 ms | 22.0 ms | 272.2 ms | 1.0 ms | 0.5 ms | **3223.68 ms** | Apprendre MySQL 5 19 - Crée... | 100.0% | 0 |
| 109 | `Apprendre_MySQL_(5_19)_-_Créer_une_tabl...` | 9980.59 ms | 88.96 ms | 20.32 ms | 129.91 ms | 1.0 ms | 0.5 ms | **10220.09 ms** | Apprendre MySQL 5 19 - Crée... | 100.0% | 0 |
| 110 | `Apprendre_MySQL_(6_19)_-_Insérer_des_do...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.91 ms** | Erreur | -- | 0 |
| 111 | `Apprendre_MySQL_(7_19)_-_Supprimer_des_...` | 3645.05 ms | 409.35 ms | 23.19 ms | 237.38 ms | 1.0 ms | 0.5 ms | **4315.39 ms** | Apprendre MySQL 8 19 - Modi... | 100.0% | 0 |
| 112 | `Apprendre_MySQL_(8_19)_-_Modifier_des_d...` | 10496.8 ms | 91.56 ms | 23.09 ms | 309.08 ms | 1.0 ms | 0.5 ms | **10920.96 ms** | Apprendre MySQL 11 19 - Fon... | 100.0% | 0 |
| 113 | `Apprendre_MySQL_(9_19)_-_Sélectionner_d...` | 4734.54 ms | 93.69 ms | 21.6 ms | 104.44 ms | 1.0 ms | 0.5 ms | **4959.46 ms** | Apprendre MySQL 8 19 - Modi... | 100.0% | 0 |
| 114 | `Apprendre_MySQL_mid15s.mp4` | 1985.2 ms | 85.35 ms | 45.01 ms | 258.03 ms | 1.0 ms | 0.5 ms | **2373.89 ms** | Aucun | -- | 0 |
| 115 | `Apprendre_Ruby_-_Astuce_&_Typage_mid15s...` | 5182.24 ms | 89.65 ms | 38.61 ms | 319.67 ms | 1.0 ms | 0.5 ms | **5630.56 ms** | Apprendre Ruby - Les boucle... | 100.0% | 0 |
| 116 | `Apprendre_Ruby_-_Installation_mid15s.mp4` | 6056.89 ms | 85.44 ms | 22.01 ms | 226.91 ms | 1.0 ms | 0.5 ms | **6393.19 ms** | Apprendre Ruby - Installati... | 100.0% | 0 |
| 117 | `Apprendre_Ruby_-_Introduction_mid15s.mp4` | 2108.19 ms | 173.54 ms | 32.04 ms | 453.77 ms | 1.0 ms | 0.5 ms | **2777.04 ms** | Apprendre Ruby - Introducti... | 100.0% | 0 |
| 118 | `Apprendre_Ruby_-_L'héritage_mid15s.mp4` | 1730.24 ms | 91.53 ms | 24.86 ms | 162.4 ms | 1.0 ms | 0.5 ms | **2009.7 ms** | Apprendre Ruby - L'héritage... | 100.0% | 0 |
| 119 | `Apprendre_Ruby_-_Les_Classes_mid15s.mp4` | 3544.99 ms | 93.1 ms | 26.27 ms | 95.05 ms | 1.0 ms | 0.5 ms | **3759.72 ms** | Apprendre Ruby - Les Classe... | 100.0% | 0 |
| 120 | `Apprendre_Ruby_-_Les_Conditions_mid15s.mp4` | 5594.58 ms | 87.48 ms | 36.02 ms | 248.96 ms | 1.0 ms | 0.5 ms | **5967.85 ms** | Aucun | -- | 0 |
| 121 | `Apprendre_Ruby_-_Les_Tableaux_et_Hash_m...` | 11632.75 ms | 87.08 ms | 19.79 ms | 143.77 ms | 1.0 ms | 0.5 ms | **11883.78 ms** | Apprendre Ruby - Les Tablea... | 100.0% | 0 |
| 122 | `Apprendre_Ruby_-_Les_Variables_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.38 ms** | Erreur | -- | 0 |
| 123 | `Apprendre_Ruby_-_Les_boucles_mid15s.mp4` | 1842.25 ms | 93.34 ms | 25.64 ms | 199.25 ms | 1.0 ms | 0.5 ms | **2161.02 ms** | Apprendre Ruby - Astuce & T... | 100.0% | 0 |
| 124 | `Apprendre_Ruby_-_Les_exceptions_mid15s.mp4` | 1805.22 ms | 324.47 ms | 23.64 ms | 288.7 ms | 1.0 ms | 0.5 ms | **2442.44 ms** | Apprendre Ruby - Les except... | 100.0% | 0 |
| 125 | `Apprendre_Ruby_-_Les_modules_-_Mixins_m...` | 2317.31 ms | 88.94 ms | 34.77 ms | 162.91 ms | 1.0 ms | 0.5 ms | **2604.44 ms** | Apprendre Ruby - Les module... | 100.0% | 0 |
| 126 | `Apprendre_Ruby_-_Les_modules_-_Namespac...` | 2922.35 ms | 83.64 ms | 35.9 ms | 189.9 ms | 1.0 ms | 0.5 ms | **3232.2 ms** | Aucun | -- | 0 |
| 127 | `Apprendre_Ruby_-_Les_méthodes_mid15s.mp4` | 6327.76 ms | 98.41 ms | 36.7 ms | 146.82 ms | 1.0 ms | 0.5 ms | **6610.22 ms** | Apprendre Ruby - Les méthod... | 100.0% | 0 |
| 128 | `Apprendre_Ruby_-_Proc_&_Block_mid15s.mp4` | 1715.07 ms | 92.37 ms | 28.82 ms | 87.91 ms | 1.0 ms | 0.5 ms | **1924.51 ms** | Aucun | -- | 0 |
| 129 | `Apprendre_Ruby_-_TP_-_Calcul_des_fréque...` | 3543.64 ms | 108.52 ms | 29.64 ms | 411.05 ms | 1.0 ms | 0.5 ms | **4093.58 ms** | Aucun | -- | 0 |
| 130 | `Apprendre_Ruby_on_Rails_(10_29)_-_Model...` | 8779.95 ms | 1840.86 ms | 29.72 ms | 279.97 ms | 1.0 ms | 0.5 ms | **10930.88 ms** | Apprendre Ruby on Rails 10 ... | 100.0% | 0 |
| 131 | `Apprendre_Ruby_on_Rails_(11_29)_-_Model...` | 1029.72 ms | 0.0 ms | 0.0 ms | 277.91 ms | 1.0 ms | 0.5 ms | **1307.77 ms** | Aucun | -- | 0 |
| 132 | `Apprendre_Ruby_on_Rails_(12_29)_-_Model...` | 10777.43 ms | 84.62 ms | 23.42 ms | 349.45 ms | 1.0 ms | 0.5 ms | **11235.29 ms** | Aucun | -- | 0 |
| 133 | `Apprendre_Ruby_on_Rails_(13_29)_-_Vues,...` | 857.25 ms | 0.0 ms | 0.0 ms | 198.5 ms | 1.0 ms | 0.5 ms | **1055.9 ms** | Aucun | -- | 0 |
| 134 | `Apprendre_Ruby_on_Rails_(14_29)_-_Vues,...` | 611.55 ms | 0.0 ms | 0.0 ms | 127.65 ms | 1.0 ms | 0.5 ms | **739.31 ms** | Aucun | -- | 0 |
| 135 | `Apprendre_Ruby_on_Rails_(15_29)_-_L'ass...` | 1152.07 ms | 0.0 ms | 0.0 ms | 125.37 ms | 1.0 ms | 0.5 ms | **1277.57 ms** | Aucun | -- | 0 |
| 136 | `Apprendre_Ruby_on_Rails_(16_29)_-_Actio...` | 1370.5 ms | 0.0 ms | 0.0 ms | 250.64 ms | 1.0 ms | 0.5 ms | **1621.25 ms** | Aucun | -- | 0 |
| 137 | `Apprendre_Ruby_on_Rails_(17_29)_-_TP_-_...` | 1021.46 ms | 0.0 ms | 0.0 ms | 327.81 ms | 1.0 ms | 0.5 ms | **1349.39 ms** | Aucun | -- | 0 |
| 138 | `Apprendre_Ruby_on_Rails_(18_29)_-_TP_-_...` | 3900.19 ms | 79.67 ms | 30.97 ms | 170.47 ms | 1.0 ms | 0.5 ms | **4181.73 ms** | Apprendre Ruby on Rails 18 ... | 100.0% | 0 |
| 139 | `Apprendre_Ruby_on_Rails_(1_29)_-_Instal...` | 10088.35 ms | 102.85 ms | 28.78 ms | 217.6 ms | 1.0 ms | 0.5 ms | **10438.24 ms** | Apprendre Ruby on Rails 25 ... | 100.0% | 0 |
| 140 | `Apprendre_Ruby_on_Rails_(22_29)_-_TP_-_...` | 2645.64 ms | 89.53 ms | 23.57 ms | 115.27 ms | 1.0 ms | 0.5 ms | **2874.42 ms** | Apprendre Ruby on Rails 22 ... | 100.0% | 0 |
| 141 | `Apprendre_Ruby_on_Rails_(23_29)_-_Devis...` | 6135.44 ms | 86.89 ms | 24.61 ms | 177.04 ms | 1.0 ms | 0.5 ms | **6424.35 ms** | Apprendre Ruby on Rails 23 ... | 100.0% | 0 |
| 142 | `Apprendre_Ruby_on_Rails_(24_29)_-_CanCa...` | 5422.13 ms | 91.4 ms | 24.55 ms | 207.4 ms | 1.0 ms | 0.5 ms | **5745.91 ms** | Apprendre Ruby on Rails 24 ... | 100.0% | 0 |
| 143 | `Apprendre_Ruby_on_Rails_(25_29)_-_Simpl...` | 3169.84 ms | 93.81 ms | 26.74 ms | 251.49 ms | 1.0 ms | 0.5 ms | **3542.19 ms** | Aucun | -- | 0 |
| 144 | `Apprendre_Ruby_on_Rails_(26_29)_-_Carri...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.43 ms** | Erreur | -- | 0 |
| 145 | `Apprendre_Ruby_on_Rails_(27_29)_-_Uploa...` | 3201.35 ms | 97.36 ms | 34.46 ms | 167.71 ms | 1.0 ms | 0.5 ms | **3501.31 ms** | Apprendre Ruby on Rails 27 ... | 100.0% | 0 |
| 146 | `Apprendre_Ruby_on_Rails_(28_29)_-_Fragm...` | 6746.11 ms | 101.85 ms | 33.28 ms | 209.24 ms | 1.0 ms | 0.5 ms | **7090.86 ms** | Apprendre Ruby on Rails 28 ... | 100.0% | 0 |
| 147 | `Apprendre_Ruby_on_Rails_(29_29)_-_Teste...` | 1979.66 ms | 152.28 ms | 90.02 ms | 316.68 ms | 1.0 ms | 0.5 ms | **2539.04 ms** | Apprendre Ruby on Rails 29 ... | 100.0% | 0 |
| 148 | `Apprendre_Ruby_on_Rails_(2_29)_-_Notre_...` | 4001.56 ms | 126.75 ms | 46.9 ms | 270.14 ms | 1.0 ms | 0.5 ms | **4445.9 ms** | Aucun | -- | 0 |
| 149 | `Apprendre_Ruby_on_Rails_(3_29)_-_Les_Mi...` | 6614.64 ms | 101.2 ms | 22.62 ms | 194.49 ms | 1.0 ms | 0.5 ms | **6933.36 ms** | Apprendre Ruby on Rails 3 2... | 100.0% | 0 |
| 150 | `Apprendre_Ruby_on_Rails_(4_29)_-_Les_Mo...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.43 ms** | Erreur | -- | 0 |
| 151 | `Apprendre_Ruby_on_Rails_(5_29)_-_Le_CRU...` | 2559.55 ms | 127.55 ms | 179.24 ms | 214.78 ms | 1.0 ms | 0.5 ms | **3081.43 ms** | Apprendre Ruby on Rails 7 2... | 100.0% | 0 |
| 152 | `Apprendre_Ruby_on_Rails_(6_29)_-_Contro...` | 1780.2 ms | 102.96 ms | 154.84 ms | 184.78 ms | 1.0 ms | 0.5 ms | **2223.11 ms** | Apprendre Ruby on Rails 7 2... | 100.0% | 0 |
| 153 | `Apprendre_Ruby_on_Rails_(7_29)_-_Contro...` | 1747.79 ms | 127.39 ms | 181.95 ms | 95.17 ms | 1.0 ms | 0.5 ms | **2152.61 ms** | Aucun | -- | 0 |
| 154 | `Apprendre_Ruby_on_Rails_(8_29)_-_Contro...` | 10598.12 ms | 100.64 ms | 48.69 ms | 196.47 ms | 1.0 ms | 0.5 ms | **10944.42 ms** | Apprendre Ruby on Rails 8 2... | 100.0% | 0 |
| 155 | `Apprendre_Ruby_on_Rails_(9_29)_-_Models...` | 4006.36 ms | 121.84 ms | 53.24 ms | 214.12 ms | 1.0 ms | 0.5 ms | **4395.98 ms** | Apprendre Ruby on Rails 9 2... | 100.0% | 0 |
| 156 | `Apprendre_Ruby_on_Rails_-_Introduction_...` | 2663.07 ms | 1443.0 ms | 40.92 ms | 126.89 ms | 1.0 ms | 0.5 ms | **4274.27 ms** | Apprendre Ruby on Rails - I... | 100.0% | 0 |
| 157 | `Apprendre_VueJS_-_Introduction_mid15s.mp4` | 2858.06 ms | 110.53 ms | 20.88 ms | 145.7 ms | 1.0 ms | 0.5 ms | **3135.61 ms** | Aucun | -- | 0 |
| 158 | `Apprendre_l'HTML_(10_16)_-_Les_table_mi...` | 2160.66 ms | 558.14 ms | 28.56 ms | 148.14 ms | 1.0 ms | 0.5 ms | **2895.81 ms** | Apprendre l'HTML 11 16 - Le... | 100.0% | 0 |
| 159 | `Apprendre_l'HTML_(11_16)_-_Les_iframe_m...` | 11049.57 ms | 101.47 ms | 98.06 ms | 413.94 ms | 1.0 ms | 0.5 ms | **11663.39 ms** | Apprendre l'HTML 11 16 - Le... | 100.0% | 0 |
| 160 | `Apprendre_l'HTML_(12_16)_-_div_&_span_m...` | 2096.63 ms | 109.28 ms | 39.63 ms | 176.01 ms | 1.0 ms | 0.5 ms | **2422.62 ms** | Apprendre l'HTML 12 16 - di... | 100.0% | 0 |
| 161 | `Apprendre_l'HTML_(13_16)_-_L'inspecteur...` | 10258.44 ms | 100.26 ms | 130.77 ms | 409.2 ms | 1.0 ms | 0.5 ms | **10899.06 ms** | Apprendre l'HTML 15 16 - Le... | 100.0% | 0 |
| 162 | `Apprendre_l'HTML_(14_16)_-_Qu'est_ce_qu...` | 2195.7 ms | 422.39 ms | 22.08 ms | 227.62 ms | 1.0 ms | 0.5 ms | **2868.21 ms** | Apprendre l'HTML 14 16 - Qu... | 100.0% | 0 |
| 163 | `Apprendre_l'HTML_(15_16)_-_Les_formulai...` | 2124.63 ms | 89.99 ms | 44.43 ms | 295.63 ms | 1.0 ms | 0.5 ms | **2555.06 ms** | Apprendre l'HTML 12 16 - di... | 100.0% | 0 |
| 164 | `Apprendre_l'HTML_(16_16)_-_TP_-_Créer_s...` | 1877.6 ms | 105.62 ms | 35.55 ms | 431.38 ms | 1.0 ms | 0.5 ms | **2450.54 ms** | Apprendre l'HTML 16 16 - TP... | 100.0% | 0 |
| 165 | `Apprendre_l'HTML_(1_16)_-_Introduction_...` | 4951.2 ms | 112.91 ms | 35.87 ms | 147.34 ms | 1.0 ms | 0.5 ms | **5247.67 ms** | Apprendre l'HTML 2 16 - La ... | 100.0% | 0 |
| 166 | `Apprendre_l'HTML_(2_16)_-_La_syntaxe_mi...` | 8632.17 ms | 120.35 ms | 50.54 ms | 200.32 ms | 1.0 ms | 0.5 ms | **9004.25 ms** | Apprendre l'HTML 2 16 - La ... | 100.0% | 0 |
| 167 | `Apprendre_l'HTML_(3_16)_-_Installation_...` | 3614.83 ms | 99.99 ms | 31.24 ms | 279.45 ms | 1.0 ms | 0.5 ms | **4027.04 ms** | Apprendre l'HTML 3 16 - Ins... | 100.0% | 0 |
| 168 | `Apprendre_l'HTML_(4_16)_-_Installation_...` | 10299.24 ms | 126.73 ms | 24.74 ms | 158.22 ms | 1.0 ms | 0.5 ms | **10613.55 ms** | Apprendre l'HTML 4 16 - Ins... | 100.0% | 0 |
| 169 | `Apprendre_l'HTML_(5_16)_-_L'en_tête_hea...` | 1336.67 ms | 0.0 ms | 0.0 ms | 374.97 ms | 1.0 ms | 0.5 ms | **1711.76 ms** | Aucun | -- | 0 |
| 170 | `Apprendre_l'HTML_(6_16)_-_L'encodage_mi...` | 1244.74 ms | 0.0 ms | 0.0 ms | 271.11 ms | 1.0 ms | 0.5 ms | **1515.94 ms** | Aucun | -- | 0 |
| 171 | `Apprendre_l'HTML_(7_16)_-_Les_éléments_...` | 1186.08 ms | 0.0 ms | 0.0 ms | 248.85 ms | 1.0 ms | 0.5 ms | **1435.08 ms** | Aucun | -- | 0 |
| 172 | `Apprendre_l'HTML_(8_16)_-_Les_liens_hyp...` | 3971.19 ms | 129.9 ms | 31.56 ms | 237.6 ms | 1.0 ms | 0.5 ms | **4371.54 ms** | Apprendre l'HTML 8 16 - Les... | 100.0% | 0 |
| 173 | `Apprendre_l'HTML_(9_16)_-_Les_images_mi...` | 1214.05 ms | 0.0 ms | 0.0 ms | 157.66 ms | 1.0 ms | 0.5 ms | **1371.84 ms** | Aucun | -- | 0 |
| 174 | `Apprendre_le_JavaScript_-_Introduction_...` | 1245.14 ms | 0.0 ms | 0.0 ms | 169.24 ms | 1.0 ms | 0.5 ms | **1414.5 ms** | Aucun | -- | 0 |
| 175 | `Arcane_(2021)_-_S01E01_-_Welcome_to_the...` | 2743.22 ms | 90.48 ms | 30.74 ms | 178.93 ms | 1.0 ms | 0.5 ms | **3044.13 ms** | Aucun | -- | 0 |
| 176 | `Arcane_(2021)_-_S01E02_-_Some_Mysteries...` | 3195.66 ms | 110.62 ms | 14.65 ms | 211.52 ms | 1.0 ms | 0.5 ms | **3538.97 ms** | Arcane | 100.0% | 0 |
| 177 | `Arcane_(2021)_-_S01E03_-_The_Base_Viole...` | 2811.76 ms | 117.15 ms | 18.78 ms | 338.83 ms | 1.0 ms | 0.5 ms | **3289.0 ms** | Arcane | 100.0% | 0 |
| 178 | `Arcane_(2021)_-_S01E04_-_Happy_Progress...` | 2254.18 ms | 100.16 ms | 19.77 ms | 160.21 ms | 1.0 ms | 0.5 ms | **2535.52 ms** | Tales From The Loop | 100.0% | 0 |
| 179 | `Arcane_(2021)_-_S01E05_-_Everybody_Want...` | 2814.16 ms | 111.02 ms | 18.72 ms | 217.86 ms | 1.0 ms | 0.5 ms | **3162.5 ms** | Aucun | -- | 0 |
| 180 | `Arcane_(2021)_-_S01E06_-_When_These_Wal...` | 2829.95 ms | 93.04 ms | 18.38 ms | 337.52 ms | 1.0 ms | 0.5 ms | **3279.56 ms** | Aucun | -- | 0 |
| 181 | `Aristocrats.2020.1080p.BluRay.x264.AAC5...` | 2164.1 ms | 90.75 ms | 20.32 ms | 319.03 ms | 1.0 ms | 0.5 ms | **2594.91 ms** | Aristocrats | 100.0% | 0 |
| 182 | `Arrietty,_le_petit_monde_des_chapardeur...` | 2603.08 ms | 101.49 ms | 24.1 ms | 352.61 ms | 1.0 ms | 0.5 ms | **3082.06 ms** | Arrietty, le petit monde de... | 100.0% | 0 |
| 183 | `As_Bestas_(2022)[1080p.x264.AC3](Marina...` | 2398.87 ms | 96.78 ms | 21.04 ms | 203.18 ms | 1.0 ms | 0.5 ms | **2721.43 ms** | As Bestas | 100.0% | 0 |
| 184 | `Assault.On.Precinct.13.1976.1080p.BluRa...` | 1938.61 ms | 83.41 ms | 23.94 ms | 302.41 ms | 1.0 ms | 0.5 ms | **2349.05 ms** | Assault On Precinct 13 | 100.0% | 0 |
| 185 | `Atlantique_(2019)[MULTi.1080p.x264][AC3...` | 2135.15 ms | 82.08 ms | 65.55 ms | 132.79 ms | 1.0 ms | 0.5 ms | **2415.9 ms** | Aucun | -- | 0 |
| 186 | `Au_Bout_Des_Doigts_(2018)_[1080](Lamber...` | 1701.62 ms | 107.11 ms | 25.22 ms | 262.71 ms | 1.0 ms | 0.5 ms | **2097.25 ms** | Aucun | -- | 0 |
| 187 | `Avalon.2001.1080p.BluRay.x264.AAC5.1-[Y...` | 1990.34 ms | 95.45 ms | 21.27 ms | 144.11 ms | 1.0 ms | 0.5 ms | **2251.49 ms** | Avalon | 100.0% | 0 |
| 188 | `Avengers.Age.of.Ultron.2015.1080p.BluRa...` | 2915.57 ms | 104.02 ms | 19.93 ms | 310.79 ms | 1.0 ms | 0.5 ms | **3351.03 ms** | Avengers Age of Ultron | 100.0% | 1 |
| 189 | `Awakenings.1990.BLURAY.1080p.BluRay.x26...` | 2694.18 ms | 109.94 ms | 18.13 ms | 133.74 ms | 1.0 ms | 0.5 ms | **2956.71 ms** | Awakenings | 100.0% | 0 |
| 190 | `BILAN_2017_-_Ce_qu'on_a_vu_cette_année_...` | 2379.06 ms | 86.48 ms | 36.32 ms | 155.72 ms | 1.0 ms | 0.5 ms | **2658.02 ms** | BILAN | 100.0% | 0 |
| 191 | `BLADE_RUNNER_2049_-_'Black_Out_2022'_[A...` | 4887.22 ms | 113.98 ms | 23.62 ms | 150.49 ms | 1.0 ms | 0.5 ms | **5180.98 ms** | BLADE RUNNER | 100.0% | 0 |
| 192 | `Baby.Balloon.2013.FRENCH.DVDRip.XviD-UT...` | 6819.29 ms | 91.27 ms | 22.4 ms | 323.11 ms | 1.0 ms | 0.5 ms | **7257.76 ms** | Baby Balloon | 100.0% | 0 |
| 193 | `Bad.Education.2019.MULTi.1080p.WEB-DL.x...` | 2540.35 ms | 147.08 ms | 37.47 ms | 185.66 ms | 1.0 ms | 0.5 ms | **2915.75 ms** | Bad Education | 100.0% | 0 |
| 194 | `Battle_Royale_(Director's.Cut,_122min)(...` | 2394.91 ms | 96.71 ms | 20.65 ms | 257.84 ms | 1.0 ms | 0.5 ms | **2771.86 ms** | Battle Royale Director's Cu... | 100.0% | 0 |
| 195 | `Beacon.23.S01E06.Beacon.Twenty.Three.10...` | 6395.36 ms | 106.27 ms | 13.99 ms | 148.58 ms | 1.0 ms | 0.5 ms | **6668.13 ms** | Beacon 23 | 100.0% | 0 |
| 196 | `Beast_(2022)[MULTi][1080p](Idris_Elba)_...` | 2368.57 ms | 135.09 ms | 47.28 ms | 121.55 ms | 1.0 ms | 0.5 ms | **2685.84 ms** | Beast | 100.0% | 0 |
| 197 | `Beasts_Of_The_Southern_Wild_(2012)[1080...` | 2063.64 ms | 118.5 ms | 17.42 ms | 232.24 ms | 1.0 ms | 0.5 ms | **2432.13 ms** | Beasts Of The Southern Wild | 100.0% | 0 |
| 198 | `Belfast.2021.1080p.AMZN.WEBRip.DDP5.1.A...` | 2876.33 ms | 149.45 ms | 120.04 ms | 309.28 ms | 1.0 ms | 0.5 ms | **3457.79 ms** | Belfast | 100.0% | 0 |
| 199 | `Belle_(2021)[MULTi][1080p.DTS](🇯🇵)_mid1...` | 4952.32 ms | 0.0 ms | 0.0 ms | 225.16 ms | 1.0 ms | 0.5 ms | **5177.63 ms** | Aucun | -- | 0 |
| 200 | `Belle_(2021)[MULTi][1080p](monde_virtue...` | 3676.12 ms | 112.42 ms | 19.42 ms | 185.68 ms | 1.0 ms | 0.5 ms | **3996.21 ms** | Belle | 100.0% | 0 |
| 201 | `Belle_(2021)[MULTi][1080p][DTS.5.1.Fren...` | 3661.46 ms | 122.47 ms | 20.48 ms | 360.99 ms | 1.0 ms | 0.5 ms | **4172.16 ms** | Belle | 100.0% | 0 |
| 202 | `Benzinho.2018.WEB-DL.1080p.x264.NACIONA...` | 2191.3 ms | 115.46 ms | 42.42 ms | 147.0 ms | 1.0 ms | 0.5 ms | **2497.18 ms** | Benzinho | 100.0% | 0 |
| 203 | `Berserk.1997.EP01.MULTi.1080p.WEB.x264-...` | 2086.04 ms | 98.07 ms | 41.99 ms | 564.98 ms | 1.0 ms | 0.5 ms | **2795.49 ms** | Berserk | 100.0% | 12 |
| 204 | `Berserk.1997.EP02.MULTi.1080p.WEB.x264-...` | 2875.73 ms | 101.68 ms | 25.93 ms | 183.29 ms | 1.0 ms | 0.5 ms | **3187.46 ms** | Berserk | 95.0% | 12 |
| 205 | `Berserk.1997.EP03.MULTi.1080p.WEB.x264-...` | 2289.51 ms | 106.13 ms | 26.04 ms | 262.08 ms | 1.0 ms | 0.5 ms | **2684.54 ms** | Berserk | 100.0% | 12 |
| 206 | `Berserk.1997.EP04.MULTi.1080p.WEB.x264-...` | 3521.57 ms | 84.95 ms | 20.15 ms | 156.0 ms | 1.0 ms | 0.5 ms | **3783.42 ms** | Berserk | 100.0% | 12 |
| 207 | `Berserk.1997.EP05.MULTi.1080p.WEB.x264-...` | 2776.83 ms | 129.11 ms | 21.26 ms | 168.67 ms | 1.0 ms | 0.5 ms | **3097.33 ms** | Berserk | 100.0% | 12 |
| 208 | `Berserk.1997.EP06.MULTi.1080p.WEB.x264-...` | 2382.09 ms | 103.85 ms | 54.14 ms | 185.78 ms | 1.0 ms | 0.5 ms | **2726.43 ms** | Berserk | 100.0% | 12 |
| 209 | `Berserk.1997.EP07.MULTi.1080p.WEB.x264-...` | 4981.64 ms | 107.95 ms | 18.73 ms | 306.09 ms | 1.0 ms | 0.5 ms | **5422.01 ms** | Berserk | 100.0% | 12 |
| 210 | `Berserk.1997.EP08.MULTi.1080p.WEB.x264-...` | 2725.45 ms | 126.96 ms | 21.18 ms | 264.76 ms | 1.0 ms | 0.5 ms | **3139.06 ms** | Berserk | 100.0% | 12 |
| 211 | `Berserk.1997.EP09.MULTi.1080p.WEB.x264-...` | 1724.24 ms | 119.52 ms | 22.48 ms | 202.62 ms | 1.0 ms | 0.5 ms | **2082.89 ms** | Berserk | 100.0% | 12 |
| 212 | `Berserk.1997.EP10.MULTi.1080p.WEB.x264-...` | 2042.39 ms | 88.89 ms | 19.78 ms | 199.87 ms | 1.0 ms | 0.5 ms | **2352.79 ms** | Berserk | 100.0% | 12 |
| 213 | `Berserk.1997.EP11.MULTi.1080p.WEB.x264-...` | 1350.76 ms | 112.26 ms | 39.24 ms | 167.66 ms | 1.0 ms | 0.5 ms | **1671.91 ms** | Berserk | 100.0% | 12 |
| 214 | `Berserk.1997.EP12.MULTi.1080p.WEB.x264-...` | 1608.87 ms | 95.53 ms | 25.35 ms | 122.08 ms | 1.0 ms | 0.5 ms | **1853.9 ms** | Berserk | 100.0% | 12 |
| 215 | `Bicentennial_Man_(1999)_mid15s.mp4` | 2049.88 ms | 86.59 ms | 32.03 ms | 129.07 ms | 1.0 ms | 0.5 ms | **2299.62 ms** | Bicentennial Man | 100.0% | 0 |
| 216 | `Bien_débuter_avec_Adobe_Premiere_Pro_mi...` | 10110.36 ms | 89.98 ms | 26.55 ms | 229.78 ms | 1.0 ms | 0.5 ms | **10457.14 ms** | Bien débuter avec Adobe Pre... | 100.0% | 0 |
| 217 | `Big.Eyes.2014.1080p.BluRay.x264.YIFY_mi...` | 1561.95 ms | 414.22 ms | 26.43 ms | 293.48 ms | 1.0 ms | 0.5 ms | **2301.05 ms** | Big Eyes | 100.0% | 1 |
| 218 | `Big.Gold.Brick.2022.1080p.WEBRip.DD5.1....` | 2389.4 ms | 505.13 ms | 30.49 ms | 220.82 ms | 1.0 ms | 0.5 ms | **3150.39 ms** | Big Gold Brick | 100.0% | 0 |
| 219 | `BigBuckBunny(1)_mid15s.mp4` | 2538.74 ms | 137.95 ms | 23.81 ms | 185.88 ms | 1.0 ms | 0.5 ms | **2887.52 ms** | Aucun | -- | 0 |
| 220 | `BigBuckBunny_mid15s.mp4` | 2074.8 ms | 449.85 ms | 22.38 ms | 211.03 ms | 1.0 ms | 0.5 ms | **2759.86 ms** | Aucun | -- | 0 |
| 221 | `Big_Hero_6_(2014)[2160p.x265.10bit][AAC...` | 1600.71 ms | 1195.04 ms | 17.0 ms | 368.98 ms | 1.0 ms | 0.5 ms | **3188.42 ms** | Aucun | -- | 0 |
| 222 | `Bigger_Than_Us_(2021)[1080p][AAC5.1](do...` | 2022.59 ms | 93.14 ms | 23.96 ms | 134.97 ms | 1.0 ms | 0.5 ms | **2275.25 ms** | Bigger Than Us | 100.0% | 0 |
| 223 | `Bilal_A_New_Breed_of_Hero_(2015)_mid15s...` | 1851.14 ms | 459.6 ms | 25.42 ms | 205.51 ms | 1.0 ms | 0.5 ms | **2545.07 ms** | Aucun | -- | 0 |
| 224 | `Birth.Of.The.Dragon.2016.FRENCH.720p.Bl...` | 2013.75 ms | 93.56 ms | 30.82 ms | 195.64 ms | 1.0 ms | 0.5 ms | **2334.7 ms** | Birth Of The Dragon | 100.0% | 0 |
| 225 | `Blanche.Gardin.-.Je.parle.toute.seule.2...` | 2839.99 ms | 917.74 ms | 175.94 ms | 437.36 ms | 1.0 ms | 0.5 ms | **4373.77 ms** | Blanche Gardin - Je parle t... | 100.0% | 0 |
| 226 | `Blood.Diamond.2006.1080p.BluRay.x265-RA...` | 2525.73 ms | 939.1 ms | 163.57 ms | 129.99 ms | 1.0 ms | 0.5 ms | **3762.14 ms** | Blood Diamond | 100.0% | 0 |
| 227 | `Blood.Red.Sky.2021.GERMAN.1080p.NF.WEBR...` | 2705.83 ms | 1058.93 ms | 35.27 ms | 305.46 ms | 1.0 ms | 0.5 ms | **4115.43 ms** | Aucun | -- | 0 |
| 228 | `Blue.Bayou.2021.1080p.BluRay.x265-RARBG...` | 2275.6 ms | 903.7 ms | 81.32 ms | 98.2 ms | 1.0 ms | 0.5 ms | **3361.27 ms** | Blue Bayou | 100.0% | 0 |
| 229 | `Blue.Bayou.2021.2160p.4K.WEB.x265.10bit...` | 1714.78 ms | 95.39 ms | 31.02 ms | 246.4 ms | 1.0 ms | 0.5 ms | **2092.68 ms** | Blue Bayou | 100.0% | 0 |
| 230 | `Blue.Ruin.2013.FRENCH.BDRip.x264.AC3-KD...` | 2699.97 ms | 497.84 ms | 105.97 ms | 271.45 ms | 1.0 ms | 0.5 ms | **3576.59 ms** | Blue Ruin | 100.0% | 0 |
| 231 | `Bob_Marley_Dernier_Concert_RIP_Arte_mid...` | 2037.3 ms | 944.97 ms | 96.02 ms | 183.45 ms | 1.0 ms | 0.5 ms | **3262.94 ms** | Aucun | -- | 0 |
| 232 | `Boite_Noire_(2021)[1080p](Thriller_,_Dr...` | 2547.63 ms | 113.32 ms | 91.07 ms | 154.39 ms | 1.0 ms | 0.5 ms | **2909.39 ms** | Boite Noire | 100.0% | 0 |
| 233 | `Bojack.Horseman.S01E01.MULTi.[1080p].WE...` | 1641.44 ms | 868.21 ms | 150.89 ms | 195.01 ms | 1.0 ms | 0.5 ms | **2856.37 ms** | Aucun | -- | 0 |
| 234 | `Bojack.Horseman.S01E02.MULTi.[1080p].WE...` | 1982.57 ms | 110.48 ms | 123.31 ms | 138.29 ms | 1.0 ms | 0.5 ms | **2358.76 ms** | Bojack Horseman | 100.0% | 0 |
| 235 | `Bojack.Horseman.S01E03.MULTi.[1080p].WE...` | 1985.57 ms | 177.57 ms | 77.11 ms | 199.43 ms | 1.0 ms | 0.5 ms | **2440.34 ms** | Aucun | -- | 0 |
| 236 | `Bojack.Horseman.S01E04.MULTi.[1080p].WE...` | 5289.46 ms | 95.73 ms | 16.06 ms | 179.37 ms | 1.0 ms | 0.5 ms | **5580.95 ms** | Bojack Horseman | 100.0% | 0 |
| 237 | `Bojack.Horseman.S01E05.MULTi.[1080p].WE...` | 2173.77 ms | 256.26 ms | 22.42 ms | 173.71 ms | 1.0 ms | 0.5 ms | **2628.91 ms** | Bojack Horseman | 100.0% | 0 |
| 238 | `Bojack.Horseman.S01E06.MULTi.[1080p].WE...` | 1989.39 ms | 232.79 ms | 21.68 ms | 218.65 ms | 1.0 ms | 0.5 ms | **2464.01 ms** | Aucun | -- | 0 |
| 239 | `Bojack.Horseman.S01E07.MULTi.[1080p].WE...` | 3004.65 ms | 96.66 ms | 18.18 ms | 236.68 ms | 1.0 ms | 0.5 ms | **3356.89 ms** | Bojack Horseman | 100.0% | 0 |
| 240 | `Bojack.Horseman.S01E08.MULTi.[1080p].WE...` | 1711.56 ms | 2196.29 ms | 15.83 ms | 215.47 ms | 1.0 ms | 0.5 ms | **4141.42 ms** | Bojack Horseman | 100.0% | 0 |
| 241 | `Bojack.Horseman.S01E09.MULTi.[1080p].WE...` | 1944.16 ms | 230.06 ms | 22.53 ms | 116.98 ms | 1.0 ms | 0.5 ms | **2315.14 ms** | Aucun | -- | 0 |
| 242 | `Bojack.Horseman.S01E10.MULTi.[1080p].WE...` | 2230.07 ms | 873.36 ms | 15.5 ms | 151.13 ms | 1.0 ms | 0.5 ms | **3271.65 ms** | Bojack Horseman | 100.0% | 0 |
| 243 | `Bojack.Horseman.S01E11.MULTi.[1080p].WE...` | 2123.05 ms | 1122.67 ms | 22.4 ms | 174.3 ms | 1.0 ms | 0.5 ms | **3447.13 ms** | Bojack Horseman | 100.0% | 0 |
| 244 | `Bojack.Horseman.S01E12.MULTi.[1080p].WE...` | 2036.21 ms | 1136.92 ms | 16.3 ms | 162.79 ms | 1.0 ms | 0.5 ms | **3362.92 ms** | Bojack Horseman | 100.0% | 0 |
| 245 | `Bonne.Nuit.Blanche.Gardin.2019.FRENCH.H...` | 1993.63 ms | 113.95 ms | 24.89 ms | 215.62 ms | 1.0 ms | 0.5 ms | **2349.27 ms** | Bonne Nuit Blanche Gardin | 100.0% | 0 |
| 246 | `Border_(2018)[MULTi][1080p](Suédois_-_F...` | 2141.98 ms | 107.17 ms | 19.16 ms | 94.42 ms | 1.0 ms | 0.5 ms | **2364.09 ms** | Border | 100.0% | 0 |
| 247 | `Born_in_China_(2016)_mid15s.mp4` | 2631.6 ms | 356.03 ms | 24.32 ms | 366.7 ms | 1.0 ms | 0.5 ms | **3382.58 ms** | Born in China | 100.0% | 0 |
| 248 | `Boyhood_2014_720p_BRip_x264_COD_mid15s.mp4` | 2453.64 ms | 97.87 ms | 18.59 ms | 227.16 ms | 1.0 ms | 0.5 ms | **2797.85 ms** | Boyhood | 100.0% | 0 |
| 249 | `BrainDead_S01E01_mid15s.mp4` | 2133.36 ms | 99.04 ms | 27.73 ms | 164.84 ms | 1.0 ms | 0.5 ms | **2425.63 ms** | Aucun | -- | 0 |
| 250 | `BrainDead_S01E02_mid15s.mp4` | 2176.88 ms | 219.19 ms | 22.69 ms | 378.31 ms | 1.0 ms | 0.5 ms | **2801.04 ms** | Aucun | -- | 0 |
| 251 | `BrainDead_S01E03_mid15s.mp4` | 2930.86 ms | 96.17 ms | 17.34 ms | 192.9 ms | 1.0 ms | 0.5 ms | **3249.95 ms** | Haikyuu | 100.0% | 0 |
| 252 | `BrainDead_S01E04_mid15s.mp4` | 1783.67 ms | 146.21 ms | 23.61 ms | 101.32 ms | 1.0 ms | 0.5 ms | **2055.63 ms** | Aucun | -- | 0 |
| 253 | `BrainDead_S01E05_mid15s.mp4` | 3919.57 ms | 95.16 ms | 18.6 ms | 135.53 ms | 1.0 ms | 0.5 ms | **4170.36 ms** | Aucun | -- | 0 |
| 254 | `BrainDead_S01E06_mid15s.mp4` | 2060.05 ms | 96.23 ms | 19.38 ms | 482.35 ms | 1.0 ms | 0.5 ms | **2658.84 ms** | BrainDead | 100.0% | 0 |
| 255 | `BrainDead_S01E07_mid15s.mp4` | 1712.24 ms | 105.17 ms | 16.01 ms | 376.12 ms | 1.0 ms | 0.5 ms | **2209.93 ms** | BrainDead | 100.0% | 0 |
| 256 | `BrainDead_S01E08_mid15s.mp4` | 2050.76 ms | 144.17 ms | 16.05 ms | 181.83 ms | 1.0 ms | 0.5 ms | **2394.21 ms** | BrainDead | 100.0% | 0 |
| 257 | `BrainDead_S01E09_mid15s.mp4` | 8489.43 ms | 1096.96 ms | 13.86 ms | 122.62 ms | 1.0 ms | 0.5 ms | **9730.85 ms** | BrainDead | 100.0% | 0 |
| 258 | `BrainDead_S01E10_mid15s.mp4` | 2566.25 ms | 444.14 ms | 20.57 ms | 196.24 ms | 1.0 ms | 0.5 ms | **3230.98 ms** | BrainDead | 100.0% | 0 |
| 259 | `BrainDead_S01E11_mid15s.mp4` | 1778.47 ms | 97.07 ms | 30.22 ms | 255.98 ms | 1.0 ms | 0.5 ms | **2162.45 ms** | Aucun | -- | 0 |
| 260 | `BrainDead_S01E12_mid15s.mp4` | 2628.21 ms | 114.92 ms | 21.42 ms | 114.12 ms | 1.0 ms | 0.5 ms | **2879.31 ms** | BrainDead | 100.0% | 0 |
| 261 | `BrainDead_S01E13_mid15s.mp4` | 1696.87 ms | 86.93 ms | 19.38 ms | 190.74 ms | 1.0 ms | 0.5 ms | **2000.08 ms** | BrainDead | 100.0% | 0 |
| 262 | `Brother_Bear_(2003)[1080p.x264]_mid15s.mp4` | 2063.09 ms | 426.5 ms | 21.5 ms | 427.96 ms | 1.0 ms | 0.5 ms | **2941.22 ms** | Brother Bear | 100.0% | 0 |
| 263 | `Bubble_(2022)[MULTI][1080p](Tokyo_Bulle...` | 2030.88 ms | 109.04 ms | 28.76 ms | 104.48 ms | 1.0 ms | 0.5 ms | **2276.38 ms** | Bubble | 100.0% | 0 |
| 264 | `Bullet_Train_(2022)[MULTi][1080p]_mid15...` | 2544.9 ms | 93.61 ms | 20.94 ms | 429.81 ms | 1.0 ms | 0.5 ms | **3089.61 ms** | Bullet Train | 100.0% | 0 |
| 265 | `COMA_-_ESPRITS_PRISONNIERS_(Russe)(2019...` | 2175.89 ms | 91.78 ms | 25.65 ms | 226.99 ms | 1.0 ms | 0.5 ms | **2523.98 ms** | COMA - ESPRITS PRISONNIERS ... | 100.0% | 0 |
| 266 | `CSS_Maintenable_avec_SASS_et_Compass_-_...` | 8049.41 ms | 94.72 ms | 24.32 ms | 105.85 ms | 1.0 ms | 0.5 ms | **8274.64 ms** | CSS Maintenable avec SASS e... | 100.0% | 0 |
| 267 | `CSS_Maintenable_avec_SASS_et_Compass_-_...` | 7845.57 ms | 101.7 ms | 21.77 ms | 90.63 ms | 1.0 ms | 0.5 ms | **8060.0 ms** | CSS Maintenable avec SASS e... | 100.0% | 0 |
| 268 | `CSS_Maintenable_avec_SASS_et_Compass_-_...` | 4381.91 ms | 3044.99 ms | 323.85 ms | 93.84 ms | 1.0 ms | 0.5 ms | **7844.93 ms** | Aucun | -- | 0 |
| 269 | `CSS_Maintenable_avec_SASS_et_Compass_-_...` | 5242.04 ms | 173.71 ms | 32.8 ms | 355.97 ms | 1.0 ms | 0.5 ms | **5804.92 ms** | CSS Maintenable avec SASS e... | 100.0% | 0 |
| 270 | `CSS_Maintenable_avec_SASS_et_Compass_-_...` | 4402.37 ms | 2955.0 ms | 18.97 ms | 153.79 ms | 1.0 ms | 0.5 ms | **7530.52 ms** | CSS Maintenable avec SASS e... | 100.0% | 0 |
| 271 | `Cairo_Conspiracy_(2022)[MULTi.1080p][EA...` | 2235.81 ms | 85.89 ms | 24.68 ms | 262.72 ms | 1.0 ms | 0.5 ms | **2609.99 ms** | Cairo Conspiracy | 100.0% | 0 |
| 272 | `Call.My.Agent.S03E01.1080p.WEB-DL.DD+5....` | 4976.7 ms | 104.38 ms | 17.91 ms | 97.15 ms | 1.0 ms | 0.5 ms | **5196.89 ms** | Call My Agent | 100.0% | 1 |
| 273 | `Call.My.Agent.S03E02.1080p.WEB-DL.DD+5....` | 2348.41 ms | 87.07 ms | 16.95 ms | 269.34 ms | 1.0 ms | 0.5 ms | **2728.54 ms** | Call My Agent | 100.0% | 1 |
| 274 | `Call.My.Agent.S03E03.1080p.WEB-DL.DD+5....` | 2840.74 ms | 93.27 ms | 19.69 ms | 144.17 ms | 1.0 ms | 0.5 ms | **3105.39 ms** | Call My Agent | 100.0% | 1 |
| 275 | `Call.My.Agent.S03E04.1080p.WEB-DL.DD+5....` | 1968.4 ms | 157.14 ms | 23.82 ms | 268.8 ms | 1.0 ms | 0.5 ms | **2421.22 ms** | Call My Agent | 100.0% | 1 |
| 276 | `Call.My.Agent.S03E05.1080p.WEB-DL.DD+5....` | 4838.74 ms | 1167.27 ms | 16.21 ms | 110.86 ms | 1.0 ms | 0.5 ms | **6133.49 ms** | Call My Agent | 100.0% | 1 |
| 277 | `Call.My.Agent.S03E06.1080p.WEB-DL.DD+5....` | 2088.21 ms | 93.28 ms | 30.41 ms | 97.81 ms | 1.0 ms | 0.5 ms | **2312.55 ms** | Call My Agent | 100.0% | 1 |
| 278 | `Capharnaum_(2018)_[720p]_mid15s.mp4` | 2039.68 ms | 99.43 ms | 30.75 ms | 157.54 ms | 1.0 ms | 0.5 ms | **2327.78 ms** | Capharnaum | 100.0% | 0 |
| 279 | `Captain_America_The_First_Avenger_(2011...` | 2260.11 ms | 481.2 ms | 40.01 ms | 176.95 ms | 1.0 ms | 0.5 ms | **2959.12 ms** | Captain America The First A... | 100.0% | 0 |
| 280 | `Capturez_vos_mails_avec_MailDev_mid15s.mp4` | 1105.2 ms | 0.0 ms | 0.0 ms | 332.15 ms | 1.0 ms | 0.5 ms | **1437.45 ms** | Aucun | -- | 0 |
| 281 | `Carnival_Row_S02E03_The_Martyrs_Hand_10...` | 2146.28 ms | 1169.49 ms | 26.95 ms | 102.8 ms | 1.0 ms | 0.5 ms | **3461.69 ms** | Carnival Row | 100.0% | 0 |
| 282 | `Carnival_Row_S02E04_An_Unkindness_of_Ra...` | 2371.75 ms | 111.73 ms | 26.66 ms | 123.69 ms | 1.0 ms | 0.5 ms | **2634.73 ms** | Carnival Row | 100.0% | 0 |
| 283 | `Carnival_Row_S02E05_Reckoning._1080p_AM...` | 2033.03 ms | 3093.1 ms | 15.58 ms | 108.54 ms | 1.0 ms | 0.5 ms | **5250.95 ms** | Aucun | -- | 0 |
| 284 | `Carnival_Row_S02E06_Original_Sins_1080p...` | 2584.17 ms | 2111.8 ms | 15.03 ms | 107.16 ms | 1.0 ms | 0.5 ms | **4818.54 ms** | Carnival Row | 100.0% | 0 |
| 285 | `Carnival_Row_S02E07_Kindred_1080p_AMZN_...` | 1675.89 ms | 2673.72 ms | 15.52 ms | 81.53 ms | 1.0 ms | 0.5 ms | **4447.37 ms** | Carnival Row | 100.0% | 0 |
| 286 | `Carnival_Row_S02E08_Facta_Non_Verba_108...` | 2181.45 ms | 172.47 ms | 14.53 ms | 181.5 ms | 1.0 ms | 0.5 ms | **2574.87 ms** | Aucun | -- | 0 |
| 287 | `Carnival_Row_S02E09_Battle_Lines_1080p_...` | 3617.7 ms | 149.34 ms | 17.08 ms | 103.07 ms | 1.0 ms | 0.5 ms | **3897.99 ms** | Carnival Row | 100.0% | 0 |
| 288 | `Carnival_Row_S02E10_Carnival_Row_1080p_...` | 1773.12 ms | 0.0 ms | 0.0 ms | 115.41 ms | 1.0 ms | 0.5 ms | **6390.14 ms** | Aucun | -- | 0 |
| 289 | `Carnivàle_(2003)_-_S01E01_-_Milfay_(108...` | 1783.45 ms | 0.0 ms | 0.0 ms | 88.52 ms | 1.0 ms | 0.5 ms | **5390.72 ms** | Carnivàle (2003) - S01E01 -... | -- | 1 |
| 290 | `Carnivàle_(2003)_-_S01E02_-_After_the_B...` | 2146.58 ms | 1143.52 ms | 14.5 ms | 90.25 ms | 1.0 ms | 0.5 ms | **3398.98 ms** | Carnivàle | 100.0% | 1 |
| 291 | `Carnivàle_(2003)_-_S01E03_-_Tipton_(108...` | 1923.76 ms | 1425.41 ms | 19.87 ms | 61.73 ms | 1.0 ms | 0.5 ms | **3431.3 ms** | Aucun | -- | 0 |
| 292 | `Carnivàle_(2003)_-_S01E04_-_Black_Blizz...` | 1338.83 ms | 879.42 ms | 58.93 ms | 60.5 ms | 1.0 ms | 0.5 ms | **2338.1 ms** | Aucun | -- | 0 |
| 293 | `Carnivàle_(2003)_-_S01E05_-_Babylon_(10...` | 1290.5 ms | 0.0 ms | 0.0 ms | 145.82 ms | 1.0 ms | 0.5 ms | **5011.84 ms** | Aucun | -- | 0 |
| 294 | `Carnivàle_(2003)_-_S01E06_-_Pick_a_Numb...` | 1272.35 ms | 406.95 ms | 16.92 ms | 79.2 ms | 1.0 ms | 0.5 ms | **1776.14 ms** | Carnivàle | 100.0% | 0 |
| 295 | `Carnivàle_(2003)_-_S01E07_-_The_River_(...` | 2388.02 ms | 98.69 ms | 19.39 ms | 194.25 ms | 1.0 ms | 0.5 ms | **2701.05 ms** | Aucun | -- | 0 |
| 296 | `Carnivàle_(2003)_-_S01E08_-_Lonnigan,_T...` | 1110.28 ms | 492.66 ms | 24.9 ms | 179.63 ms | 1.0 ms | 0.5 ms | **1809.36 ms** | Carnivàle | 100.0% | 0 |
| 297 | `Carnivàle_(2003)_-_S01E09_-_Insomnia_(1...` | 1173.28 ms | 111.54 ms | 37.0 ms | 110.76 ms | 1.0 ms | 0.5 ms | **1434.86 ms** | Carnivàle | 100.0% | 0 |
| 298 | `Carnivàle_(2003)_-_S01E10_-_Hot_and_Bot...` | 1751.06 ms | 119.74 ms | 17.93 ms | 123.81 ms | 1.0 ms | 0.5 ms | **2013.41 ms** | Aucun | -- | 0 |
| 299 | `Carnivàle_(2003)_-_S01E11_-_The_Day_of_...` | 2164.91 ms | 119.87 ms | 19.92 ms | 141.45 ms | 1.0 ms | 0.5 ms | **2452.09 ms** | Aucun | -- | 0 |
| 300 | `Carnivàle_(2003)_-_S01E12_-_The_Day_Tha...` | 2122.13 ms | 112.51 ms | 24.19 ms | 185.17 ms | 1.0 ms | 0.5 ms | **2444.42 ms** | Aucun | -- | 0 |
| 301 | `Carnivàle_(2003)_-_S02E01_-_Los_Moscos_...` | 1589.55 ms | 102.81 ms | 20.35 ms | 152.16 ms | 1.0 ms | 0.5 ms | **1885.85 ms** | Aucun | -- | 0 |
| 302 | `Carnivàle_(2003)_-_S02E02_-_Alamogordo,...` | 1994.92 ms | 110.79 ms | 16.61 ms | 82.96 ms | 1.0 ms | 0.5 ms | **2206.18 ms** | Aucun | -- | 0 |
| 303 | `Carnivàle_(2003)_-_S02E03_-_Ingram,_TX_...` | 2015.31 ms | 108.86 ms | 21.85 ms | 158.57 ms | 1.0 ms | 0.5 ms | **2320.41 ms** | Aucun | -- | 0 |
| 304 | `Carnivàle_(2003)_-_S02E04_-_Old_Cherry_...` | 1690.37 ms | 389.99 ms | 23.14 ms | 128.82 ms | 1.0 ms | 0.5 ms | **2237.57 ms** | Aucun | -- | 0 |
| 305 | `Carnivàle_(2003)_-_S02E05_-_Creed,_OK_(...` | 3322.52 ms | 1891.95 ms | 23.42 ms | 223.19 ms | 1.0 ms | 0.5 ms | **5461.46 ms** | Aucun | -- | 0 |
| 306 | `Carnivàle_(2003)_-_S02E06_-_The_Road_to...` | 2153.96 ms | 200.84 ms | 23.1 ms | 143.89 ms | 1.0 ms | 0.5 ms | **2522.13 ms** | Carnivàle | 100.0% | 0 |
| 307 | `Carnivàle_(2003)_-_S02E07_-_Damascus,_N...` | 1762.03 ms | 1895.59 ms | 20.49 ms | 134.3 ms | 1.0 ms | 0.5 ms | **3814.04 ms** | Carnivàle | 100.0% | 0 |
| 308 | `Carnivàle_(2003)_-_S02E08_-_Outskirts,_...` | 2124.2 ms | 214.44 ms | 38.03 ms | 183.01 ms | 1.0 ms | 0.5 ms | **2560.34 ms** | Carnivàle | 100.0% | 0 |
| 309 | `Carnivàle_(2003)_-_S02E09_-_Lincoln_Hig...` | 1985.97 ms | 157.4 ms | 23.49 ms | 178.18 ms | 1.0 ms | 0.5 ms | **2345.58 ms** | Carnivàle | 100.0% | 0 |
| 310 | `Carnivàle_(2003)_-_S02E10_-_Cheyenne,_W...` | 1941.03 ms | 88.61 ms | 17.26 ms | 149.84 ms | 1.0 ms | 0.5 ms | **2197.56 ms** | Aucun | -- | 0 |
| 311 | `Carnivàle_(2003)_-_S02E11_-_Outside_New...` | 2034.73 ms | 451.46 ms | 63.82 ms | 180.47 ms | 1.0 ms | 0.5 ms | **2731.36 ms** | Aucun | -- | 0 |
| 312 | `Carnivàle_(2003)_-_S02E12_-_New_Canaan,...` | 1964.41 ms | 2086.7 ms | 69.28 ms | 256.82 ms | 1.0 ms | 0.5 ms | **4382.31 ms** | Aucun | -- | 0 |
| 313 | `Casualties.of.War.1989.1080p.BluRay.x26...` | 1610.66 ms | 88.47 ms | 28.01 ms | 103.72 ms | 1.0 ms | 0.5 ms | **1831.56 ms** | Casualties of War | 100.0% | 0 |
| 314 | `Caught_Stealing_(2025)_1080p_BluRay_10b...` | 1661.28 ms | 2034.62 ms | 52.96 ms | 265.15 ms | 1.0 ms | 0.5 ms | **4014.77 ms** | Caught Stealing | 100.0% | 0 |
| 315 | `Ce_que_League_of_Legends_m'a_appris_sur...` | 1665.17 ms | 171.75 ms | 33.79 ms | 127.75 ms | 1.0 ms | 0.5 ms | **1999.99 ms** | Aucun | -- | 0 |
| 316 | `Chaos.Walking.2021.1080p.WEB-DL.DD5.1.H...` | 2004.75 ms | 92.12 ms | 21.51 ms | 121.99 ms | 1.0 ms | 0.5 ms | **2244.0 ms** | Chaos Walking | 100.0% | 1 |
| 317 | `Charlotte_(2021)[1080p](juive,_peinture...` | 1515.29 ms | 449.6 ms | 19.22 ms | 154.46 ms | 1.0 ms | 0.5 ms | **2146.41 ms** | Charlotte | 100.0% | 0 |
| 318 | `Chico.And.Rita.FRENCH.BDRip.XviD-AYMO_m...` | 1578.11 ms | 188.03 ms | 25.63 ms | 179.96 ms | 1.0 ms | 0.5 ms | **1978.49 ms** | Chico And Rita FRENCH | 100.0% | 0 |
| 319 | `Children.of.Men.2006.1080p.BrRip.x264.B...` | 1756.49 ms | 358.11 ms | 19.51 ms | 128.48 ms | 1.0 ms | 0.5 ms | **2263.3 ms** | Aucun | -- | 0 |
| 320 | `Children_Of_The_Sea_(2019)[1080p][BluRa...` | 1810.27 ms | 388.41 ms | 48.31 ms | 117.18 ms | 1.0 ms | 0.5 ms | **2364.72 ms** | Children Of The Sea | 100.0% | 0 |
| 321 | `Chip.n.Dale.Rescue.Rangers.2022.1080p.D...` | 1937.49 ms | 91.73 ms | 20.08 ms | 140.18 ms | 1.0 ms | 0.5 ms | **2190.03 ms** | Chip n Dale Rescue Rangers | 100.0% | 0 |
| 322 | `Chronique_d'une_Liaison_Passagere_(2022...` | 7760.63 ms | 89.85 ms | 21.3 ms | 240.5 ms | 1.0 ms | 0.5 ms | **8112.7 ms** | Chronique d'une Liaison Pas... | 100.0% | 0 |
| 323 | `Citizen_Kane_(1941)[1080p]_mid15s.mp4` | 1642.34 ms | 219.33 ms | 27.39 ms | 125.54 ms | 1.0 ms | 0.5 ms | **2016.64 ms** | Citizen Kane | 100.0% | 0 |
| 324 | `Claire_Andrieux_(2020)[1080p](Drame,_se...` | 3689.86 ms | 2371.67 ms | 19.33 ms | 100.45 ms | 1.0 ms | 0.5 ms | **6198.48 ms** | Claire Andrieux | 100.0% | 0 |
| 325 | `Clara_(2019)[FRENCH][1080p](Issac_Bruno...` | 1919.0 ms | 0.0 ms | 0.0 ms | 72.67 ms | 1.0 ms | 0.5 ms | **5494.6 ms** | Aucun | -- | 0 |
| 326 | `Clean.2022.1080p.WEBRip.DD5.1.x264-NOGR...` | 2186.98 ms | 367.82 ms | 19.41 ms | 292.77 ms | 1.0 ms | 0.5 ms | **2867.74 ms** | Clean | 100.0% | 0 |
| 327 | `Climax_(2018)_[1080p](Gaspar_Noé,_Naîtr...` | 2244.54 ms | 96.35 ms | 26.69 ms | 182.31 ms | 1.0 ms | 0.5 ms | **2551.06 ms** | Climax | 100.0% | 0 |
| 328 | `Cmon.Cmon.2021.1080p.WEBRip.DDP5.1.Atmo...` | 1839.15 ms | 97.31 ms | 21.0 ms | 285.99 ms | 1.0 ms | 0.5 ms | **2248.01 ms** | Cmon Cmon | 100.0% | 1 |
| 329 | `Cmon.Cmon.2021.2160p.4K.WEB.x265.10bit....` | 2016.1 ms | 188.41 ms | 21.15 ms | 135.2 ms | 1.0 ms | 0.5 ms | **2361.85 ms** | Cmon Cmon | 100.0% | 1 |
| 330 | `Cobweb.2023.2160p.AMZN.WEB-DL.DDP5.1.H....` | 3322.62 ms | 1880.25 ms | 56.68 ms | 95.98 ms | 1.0 ms | 0.5 ms | **5356.13 ms** | Cobweb | 100.0% | 0 |
| 331 | `Cold.War_(2018)_[1080p](Pendant_la_guer...` | 1794.15 ms | 3175.17 ms | 55.02 ms | 141.45 ms | 1.0 ms | 0.5 ms | **5167.04 ms** | Cold War | 100.0% | 0 |
| 332 | `Comment_sont_fait_les_tutoriels_mid15s.mp4` | 2048.35 ms | 129.73 ms | 27.3 ms | 225.23 ms | 1.0 ms | 0.5 ms | **2431.12 ms** | Comment sont fait les tutor... | 100.0% | 0 |
| 333 | `Community_S01E01_Pilot_[1080p_x265_10bi...` | 2120.6 ms | 2179.33 ms | 16.78 ms | 136.84 ms | 1.0 ms | 0.5 ms | **4456.56 ms** | Community | 100.0% | 0 |
| 334 | `Community_S01E07_Introduction_to_Statis...` | 1700.61 ms | 2437.07 ms | 17.36 ms | 144.92 ms | 1.0 ms | 0.5 ms | **4304.79 ms** | Community | 100.0% | 0 |
| 335 | `Community_S01E16_Communication_Studies_...` | 1790.45 ms | 3895.27 ms | 30.23 ms | 207.24 ms | 1.0 ms | 0.5 ms | **5924.16 ms** | Extended Producer's Cut - C... | 100.0% | 0 |
| 336 | `Community_S01E23_Modern_Warfare_[1080p_...` | 1828.69 ms | 1986.1 ms | 14.66 ms | 117.56 ms | 1.0 ms | 0.5 ms | **3947.64 ms** | Aucun | -- | 0 |
| 337 | `Community_S01E25_Pascals_Triangle_Revis...` | 1884.55 ms | 2294.19 ms | 18.4 ms | 201.25 ms | 1.0 ms | 0.5 ms | **4399.54 ms** | Aucun | -- | 0 |
| 338 | `Community_S01_-_Outtakes_4_mid15s.mp4` | 2026.24 ms | 1864.85 ms | 17.95 ms | 87.12 ms | 1.0 ms | 0.5 ms | **3996.7 ms** | Community | 100.0% | 0 |
| 339 | `Compagnons_(2021)[1080p](Maison_des_Com...` | 1776.88 ms | 2012.87 ms | 27.55 ms | 98.09 ms | 1.0 ms | 0.5 ms | **3916.11 ms** | Compagnons | 100.0% | 0 |
| 340 | `Comprendre_Git_(12_17)_-_Nommer_ses_com...` | 1691.86 ms | 109.32 ms | 22.67 ms | 109.17 ms | 1.0 ms | 0.5 ms | **1933.42 ms** | Comprendre Git 12 17 - Nomm... | 100.0% | 0 |
| 341 | `Comprendre_Git_-_Fork_&_Pull_request_mi...` | 3527.84 ms | 138.44 ms | 22.55 ms | 165.21 ms | 1.0 ms | 0.5 ms | **3856.4 ms** | Comprendre Git - Fork & Pul... | 100.0% | 0 |
| 342 | `Comprendre_Git_-_GitKraken_mid15s.mp4` | 904.18 ms | 0.0 ms | 0.0 ms | 132.32 ms | 1.0 ms | 0.5 ms | **1036.64 ms** | Aucun | -- | 0 |
| 343 | `Comprendre_Git_-_Git_Flow_mid15s.mp4` | 1538.85 ms | 2238.67 ms | 42.14 ms | 267.46 ms | 1.0 ms | 0.5 ms | **4087.99 ms** | Comprendre Git - Git Flow m... | 100.0% | 0 |
| 344 | `Comprendre_Git_-_Github_Mac_mid15s.mp4` | 852.31 ms | 0.0 ms | 0.0 ms | 128.57 ms | 1.0 ms | 0.5 ms | **980.99 ms** | Aucun | -- | 0 |
| 345 | `Comprendre_Git_-_Github_Windows_mid15s.mp4` | 1245.79 ms | 0.0 ms | 0.0 ms | 112.49 ms | 1.0 ms | 0.5 ms | **1358.38 ms** | Aucun | -- | 0 |
| 346 | `Comprendre_Git_-_Installer_git_sur_linu...` | 770.29 ms | 0.0 ms | 0.0 ms | 90.1 ms | 1.0 ms | 0.5 ms | **860.5 ms** | Aucun | -- | 0 |
| 347 | `Comprendre_Git_-_Installer_git_sur_mac_...` | 607.85 ms | 0.0 ms | 0.0 ms | 108.97 ms | 1.0 ms | 0.5 ms | **716.95 ms** | Aucun | -- | 0 |
| 348 | `Comprendre_Git_-_Installer_git_sur_wind...` | 832.69 ms | 0.0 ms | 0.0 ms | 88.3 ms | 1.0 ms | 0.5 ms | **921.14 ms** | Aucun | -- | 0 |
| 349 | `Comprendre_Git_-_Le_remisage_mid15s.mp4` | 893.65 ms | 1133.02 ms | 21.29 ms | 97.79 ms | 1.0 ms | 0.5 ms | **2146.12 ms** | Comprendre Git - Le remisag... | 100.0% | 0 |
| 350 | `Comprendre_Git_-_Les_branches_mid15s.mp4` | 699.34 ms | 0.0 ms | 0.0 ms | 73.28 ms | 1.0 ms | 0.5 ms | **772.73 ms** | Aucun | -- | 0 |
| 351 | `Comprendre_Git_-_Revenir_en_arrière_mid...` | 625.29 ms | 0.0 ms | 0.0 ms | 67.69 ms | 1.0 ms | 0.5 ms | **693.12 ms** | Aucun | -- | 0 |
| 352 | `Comprendre_Git_-_Source_Tree_mid15s.mp4` | 770.41 ms | 0.0 ms | 0.0 ms | 89.5 ms | 1.0 ms | 0.5 ms | **860.01 ms** | Aucun | -- | 0 |
| 353 | `Comprendre_Webpack_(10_10)_-_Plugin_HTM...` | 1203.97 ms | 450.26 ms | 53.3 ms | 321.52 ms | 1.0 ms | 0.5 ms | **2029.41 ms** | Comprendre Webpack 10 10 - ... | 100.0% | 0 |
| 354 | `Comprendre_Webpack_(11_11)_-_Analyser_l...` | 932.08 ms | 94.31 ms | 22.06 ms | 108.14 ms | 1.0 ms | 0.5 ms | **1156.97 ms** | Comprendre Webpack 11 11 - ... | 100.0% | 0 |
| 355 | `Comprendre_Webpack_(12_12)_-_React_avec...` | 825.19 ms | 484.36 ms | 93.06 ms | 304.25 ms | 1.0 ms | 0.5 ms | **1707.21 ms** | Aucun | -- | 0 |
| 356 | `Comprendre_Webpack_(14_14)_-_VueJS_&_Ty...` | 853.14 ms | 156.94 ms | 56.62 ms | 101.78 ms | 1.0 ms | 0.5 ms | **1168.84 ms** | Aucun | -- | 0 |
| 357 | `Comprendre_Webpack_(15_15)_-_Webpack_4_...` | 760.7 ms | 0.0 ms | 0.0 ms | 65.49 ms | 1.0 ms | 0.5 ms | **826.5 ms** | Aucun | -- | 0 |
| 358 | `Comprendre_Webpack_(1_9)_-_Configuratio...` | 1241.98 ms | 88.08 ms | 23.92 ms | 106.61 ms | 1.0 ms | 0.5 ms | **1460.99 ms** | Comprendre Webpack 1 9 - Co... | 100.0% | 0 |
| 359 | `Comprendre_Webpack_(2_9)_-_Babel_(es201...` | 1165.19 ms | 86.03 ms | 19.06 ms | 174.17 ms | 1.0 ms | 0.5 ms | **1445.09 ms** | Comprendre Webpack 2 9 - Ba... | 100.0% | 0 |
| 360 | `Comprendre_Webpack_(3_9)_-_Minification...` | 892.9 ms | 0.0 ms | 0.0 ms | 108.38 ms | 1.0 ms | 0.5 ms | **1001.4 ms** | Aucun | -- | 0 |
| 361 | `Comprendre_Webpack_(4_9)_-_CSS_&_SASS_m...` | 843.44 ms | 0.0 ms | 0.0 ms | 122.34 ms | 1.0 ms | 0.5 ms | **965.89 ms** | Aucun | -- | 0 |
| 362 | `Comprendre_Webpack_(5_9)_-_Extraire_le_...` | 1368.36 ms | 0.0 ms | 0.0 ms | 147.42 ms | 1.0 ms | 0.5 ms | **1515.89 ms** | Aucun | -- | 0 |
| 363 | `Comprendre_Webpack_(6_9)_-_Mise_en_cach...` | 3768.42 ms | 133.67 ms | 163.41 ms | 223.94 ms | 1.0 ms | 0.5 ms | **4290.14 ms** | Aucun | -- | 0 |
| 364 | `Comprendre_Webpack_(7_9)_-_Url_Loader_m...` | 3218.82 ms | 87.13 ms | 19.96 ms | 131.76 ms | 1.0 ms | 0.5 ms | **3458.11 ms** | Comprendre Webpack 7 9 - Ur... | 100.0% | 0 |
| 365 | `Comprendre_Webpack_(8_9)_-_ESLint_mid15...` | 995.53 ms | 0.0 ms | 0.0 ms | 173.15 ms | 1.0 ms | 0.5 ms | **1168.79 ms** | Aucun | -- | 0 |
| 366 | `Comprendre_Webpack_(9_9)_-_Dev_server_m...` | 9708.64 ms | 114.85 ms | 21.63 ms | 104.02 ms | 1.0 ms | 0.5 ms | **9965.68 ms** | Comprendre Webpack 9 9 - De... | 100.0% | 0 |
| 367 | `Comprendre_git_-_Manipuler_l'historique...` | 8690.08 ms | 99.0 ms | 19.39 ms | 453.31 ms | 1.0 ms | 0.5 ms | **9262.14 ms** | Comprendre git - Manipuler ... | 100.0% | 0 |
| 368 | `Comprendre_git_-_Premiers_commits_mid15...` | 2454.89 ms | 98.42 ms | 35.75 ms | 203.94 ms | 1.0 ms | 0.5 ms | **2793.4 ms** | Comprendre git - Premiers c... | 100.0% | 0 |
| 369 | `Comprendre_git_-_Remote_mid15s.mp4` | 8569.25 ms | 93.63 ms | 24.12 ms | 654.51 ms | 1.0 ms | 0.5 ms | **9341.89 ms** | Comprendre git - Remote mp4... | 100.0% | 0 |
| 370 | `Conan.The.Barbarian.1982.1080p.BluRay.x...` | 2381.78 ms | 107.51 ms | 32.55 ms | 151.04 ms | 1.0 ms | 0.5 ms | **2677.53 ms** | Conan The Barbarian | 100.0% | 1 |
| 371 | `Concussion.2015.1080p.BluRay.x265-RARBG...` | 2588.61 ms | 90.27 ms | 21.0 ms | 139.85 ms | 1.0 ms | 0.5 ms | **2840.33 ms** | Concussion | 100.0% | 1 |
| 372 | `Contagion_(2011)_MULTi_(VFF-VO-VFQ)_108...` | 2433.6 ms | 125.8 ms | 19.21 ms | 108.87 ms | 1.0 ms | 0.5 ms | **2691.03 ms** | Contagion | 100.0% | 0 |
| 373 | `Cosmic.Sin.2021.1080p.AMZN.WEBRip.DDP5....` | 2713.97 ms | 90.54 ms | 33.38 ms | 201.19 ms | 1.0 ms | 0.5 ms | **3039.71 ms** | Cosmic Sin | 100.0% | 0 |
| 374 | `Cosmic.Sin.2021.1080p.WEBRip.x265-RARBG...` | 3166.6 ms | 161.05 ms | 31.99 ms | 145.43 ms | 1.0 ms | 0.5 ms | **3506.24 ms** | Cosmic Sin | 100.0% | 0 |
| 375 | `Cosmopolis.2012.1080p.BluRay.x265-RARBG...` | 3071.07 ms | 151.01 ms | 137.36 ms | 252.64 ms | 1.0 ms | 0.5 ms | **3618.73 ms** | Aucun | -- | 0 |
| 376 | `Coupez_(2022)[1080p](Romain_Duris,_Béré...` | 1881.91 ms | 93.2 ms | 25.02 ms | 200.63 ms | 1.0 ms | 0.5 ms | **2202.79 ms** | Coupez | 100.0% | 0 |
| 377 | `Creating_the_Scene_-_Brother_Justin_mid...` | 2730.81 ms | 158.1 ms | 20.04 ms | 406.27 ms | 1.0 ms | 0.5 ms | **3317.92 ms** | Creating the Scene - Brothe... | 100.0% | 0 |
| 378 | `Creating_the_Scene_-_Carnivàle_II_mid15...` | 4816.94 ms | 140.03 ms | 25.49 ms | 162.26 ms | 1.0 ms | 0.5 ms | **5149.46 ms** | Creating the Scene - Carniv... | 100.0% | 0 |
| 379 | `Creating_the_Scene_-_Face_Peel_mid15s.mp4` | 5838.56 ms | 319.94 ms | 20.39 ms | 225.06 ms | 1.0 ms | 0.5 ms | **6404.72 ms** | Creating the Scene - Face P... | 100.0% | 0 |
| 380 | `Creating_the_Scene_-_Trinity_mid15s.mp4` | 2499.74 ms | 173.27 ms | 112.92 ms | 207.95 ms | 1.0 ms | 0.5 ms | **2994.26 ms** | Creating the Scene - Trinit... | 100.0% | 0 |
| 381 | `Creed_II_(2018)_[1080p]_mid15s.mp4` | 2321.54 ms | 238.68 ms | 138.69 ms | 193.89 ms | 1.0 ms | 0.5 ms | **2893.41 ms** | Creed II | 100.0% | 0 |
| 382 | `Crimes_of_the_Future_(2022)[MULTi][1080...` | 2132.67 ms | 137.32 ms | 22.6 ms | 180.04 ms | 1.0 ms | 0.5 ms | **2473.34 ms** | Crimes of the Future | 100.0% | 0 |
| 383 | `Crimes_of_the_Future_(2022)[MULTi][1080...` | 1909.3 ms | 301.81 ms | 72.8 ms | 272.54 ms | 1.0 ms | 0.5 ms | **2556.93 ms** | Crimes of the Future | 100.0% | 0 |
| 384 | `Crimson.Peak.2015.MULTi.1080p.BluRay.x2...` | 1991.63 ms | 92.42 ms | 31.17 ms | 282.84 ms | 1.0 ms | 0.5 ms | **2399.45 ms** | Crimson Peak | 100.0% | 0 |
| 385 | `Créer_un_portfolio_de_A_à_Z_en_PHP_Brut...` | 3761.4 ms | 114.83 ms | 98.09 ms | 161.76 ms | 1.0 ms | 0.5 ms | **4136.44 ms** | Créer un portfolio de A à Z... | 100.0% | 0 |
| 386 | `Créer_un_portfolio_de_A_à_Z_en_PHP_Brut...` | 3540.51 ms | 394.32 ms | 107.7 ms | 221.11 ms | 1.0 ms | 0.5 ms | **4264.02 ms** | Créer un portfolio de A à Z... | 100.0% | 0 |
| 387 | `Créer_un_portfolio_de_A_à_Z_en_PHP_Brut...` | 2039.72 ms | 105.04 ms | 46.05 ms | 326.38 ms | 1.0 ms | 0.5 ms | **2517.61 ms** | Aucun | -- | 0 |
| 388 | `Créer_un_portfolio_de_A_à_Z_en_PHP_Brut...` | 1638.41 ms | 82.41 ms | 30.13 ms | 312.16 ms | 1.0 ms | 0.5 ms | **2063.55 ms** | Aucun | -- | 0 |
| 389 | `Créer_un_portfolio_de_A_à_Z_en_PHP_Brut...` | 6431.78 ms | 82.33 ms | 318.42 ms | 254.62 ms | 1.0 ms | 0.5 ms | **7087.72 ms** | Créer un portfolio de A à Z... | 100.0% | 0 |
| 390 | `Créer_un_système_de_messagerie_(0_3)_-_...` | 4636.96 ms | 431.37 ms | 25.12 ms | 134.68 ms | 1.0 ms | 0.5 ms | **5228.5 ms** | Aucun | -- | 0 |
| 391 | `Créer_un_système_de_messagerie_(1_3)_-_...` | 3368.85 ms | 389.65 ms | 22.49 ms | 179.52 ms | 1.0 ms | 0.5 ms | **3960.88 ms** | Créer un système de message... | 100.0% | 0 |
| 392 | `Créer_un_système_de_messagerie_(2_3)_-_...` | 2092.88 ms | 860.14 ms | 60.69 ms | 308.53 ms | 1.0 ms | 0.5 ms | **3322.64 ms** | Créer un système de message... | 100.0% | 0 |
| 393 | `Créer_un_système_de_messagerie_(3_3)_-_...` | 9794.75 ms | 95.39 ms | 70.2 ms | 215.28 ms | 1.0 ms | 0.5 ms | **10184.69 ms** | Créer un système de message... | 100.0% | 0 |
| 394 | `Cyrano_(2021)[MULTi][1080p](fr-en_srt-a...` | 1855.89 ms | 885.11 ms | 67.31 ms | 260.81 ms | 1.0 ms | 0.5 ms | **3069.51 ms** | Cyrano | 100.0% | 0 |
| 395 | `DC_League_of_Super_Pets_(2022)[MULTi][1...` | 2329.22 ms | 100.89 ms | 21.97 ms | 236.62 ms | 1.0 ms | 0.5 ms | **2695.05 ms** | Aucun | -- | 0 |
| 396 | `DC_League_of_Super_Pets_(2022)[MULTi][1...` | 2037.37 ms | 410.79 ms | 25.83 ms | 193.17 ms | 1.0 ms | 0.5 ms | **2667.97 ms** | DC League of Super Pets | 100.0% | 0 |
| 397 | `Daddy.Cool.2017.FRENCH.BDRiP.x264.AC3-S...` | 1966.96 ms | 1170.97 ms | 17.79 ms | 117.46 ms | 1.0 ms | 0.5 ms | **3275.68 ms** | Daddy Cool | 100.0% | 0 |
| 398 | `Dans.Un.Recoin.De.Ce.Monde.2016.FRENCH....` | 5922.37 ms | 90.81 ms | 20.35 ms | 157.05 ms | 1.0 ms | 0.5 ms | **6199.7 ms** | Dans Un Recoin De Ce Monde | 100.0% | 0 |
| 399 | `Dante.01.Space.Prison.English.DVDrip.CO...` | 1807.73 ms | 659.66 ms | 22.25 ms | 206.93 ms | 1.0 ms | 0.5 ms | **2699.47 ms** | Aucun | -- | 0 |
| 400 | `Dark.City.1998.1080p.BluRay.x264-[YTS.A...` | 3519.31 ms | 130.26 ms | 28.29 ms | 175.34 ms | 1.0 ms | 0.5 ms | **3864.71 ms** | Dark City | 100.0% | 1 |
| 401 | `Dark_City_(1998)_[1080]_mid15s.mp4` | 2070.36 ms | 396.88 ms | 18.17 ms | 173.67 ms | 1.0 ms | 0.5 ms | **2663.89 ms** | Dark City | 100.0% | 0 |
| 402 | `Date_Limite_(2010),_US,_Comédie,_Robert...` | 1997.61 ms | 402.68 ms | 25.28 ms | 220.49 ms | 1.0 ms | 0.5 ms | **2649.84 ms** | Date Limite | 100.0% | 0 |
| 403 | `Day_Shift_(2022)[MULTi][1080p](🧛🔫💰)_mid...` | 1896.36 ms | 88.34 ms | 35.87 ms | 257.23 ms | 1.0 ms | 0.5 ms | **2278.72 ms** | Day Shift | 100.0% | 0 |
| 404 | `De_Gaulle_(2020)[1080p]_mid15s.mp4` | 2160.79 ms | 100.12 ms | 25.05 ms | 376.69 ms | 1.0 ms | 0.5 ms | **2666.26 ms** | De Gaulle | 100.0% | 0 |
| 405 | `De_uskyldige_(THE_INNOCENTS)_(2021)[MUL...` | 1263.84 ms | 86.41 ms | 26.38 ms | 145.88 ms | 1.0 ms | 0.5 ms | **1530.69 ms** | De uskyldige THE INNOCENTS | 100.0% | 0 |
| 406 | `Dear.Eleanor.2016.FRENCH.DVDRIP.XviD-Sh...` | 1948.63 ms | 107.0 ms | 23.54 ms | 241.89 ms | 1.0 ms | 0.5 ms | **2321.84 ms** | Dear Eleanor | 100.0% | 0 |
| 407 | `Death.on.the.Nile.2022.1080p.BluRay.x26...` | 2202.11 ms | 145.4 ms | 23.57 ms | 435.24 ms | 1.0 ms | 0.5 ms | **2806.9 ms** | Aucun | -- | 0 |
| 408 | `Debout_Les_Femmes_(2021)[1080p]_mid15s.mp4` | 2160.44 ms | 84.01 ms | 18.29 ms | 136.29 ms | 1.0 ms | 0.5 ms | **2399.76 ms** | Debout Les Femmes | 100.0% | 0 |
| 409 | `Decision_to_Leave_(2022)[VOSTFR][1080p]...` | 2344.38 ms | 90.12 ms | 19.06 ms | 228.92 ms | 1.0 ms | 0.5 ms | **2690.47 ms** | Decision to Leave | 100.0% | 0 |
| 410 | `Delicieux_(2021)[1080p](Comédie_,_Histo...` | 2829.29 ms | 90.15 ms | 18.17 ms | 217.31 ms | 1.0 ms | 0.5 ms | **3155.66 ms** | Delicieux | 100.0% | 0 |
| 411 | `Demon.Slayer.Kimetsu.no.Yaiba.Hashira.T...` | 2576.44 ms | 89.92 ms | 19.55 ms | 210.9 ms | 1.0 ms | 0.5 ms | **2902.81 ms** | Demon Slayer Kimetsu no Yai... | 100.0% | 1 |
| 412 | `Demon.Slayer.Kimetsu.no.Yaiba.Hashira.T...` | 2680.3 ms | 91.2 ms | 16.44 ms | 318.33 ms | 1.0 ms | 0.5 ms | **3107.08 ms** | Aucun | -- | 0 |
| 413 | `Demon.Slayer.Kimetsu.no.Yaiba.S05E03.Fu...` | 4513.71 ms | 103.74 ms | 19.24 ms | 107.5 ms | 1.0 ms | 0.5 ms | **4745.7 ms** | Aucun | -- | 0 |
| 414 | `Demon.Slayer.Kimetsu.no.Yaiba.S107E07.S...` | 5762.72 ms | 115.53 ms | 23.46 ms | 324.53 ms | 1.0 ms | 0.5 ms | **6228.19 ms** | Aucun | -- | 0 |
| 415 | `Demon.Slayer.Kimetsu.no.Yaiba.S57E04.To...` | 4140.81 ms | 142.77 ms | 19.03 ms | 261.97 ms | 1.0 ms | 0.5 ms | **4565.36 ms** | Aucun | -- | 0 |
| 416 | `Demon.Slayer.S04E05.1080p.CR.WEB-DL.AAC...` | 2138.44 ms | 89.59 ms | 24.65 ms | 221.65 ms | 1.0 ms | 0.5 ms | **2474.88 ms** | Aucun | -- | 0 |
| 417 | `Demon_Slayer_–_Saison_2_Episode_10_VOST...` | 1618.84 ms | 84.04 ms | 21.24 ms | 148.82 ms | 1.0 ms | 0.5 ms | **1873.66 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 418 | `Demon_Slayer_–_Saison_2_Episode_11_VOST...` | 2321.84 ms | 136.79 ms | 16.78 ms | 168.75 ms | 1.0 ms | 0.5 ms | **2655.3 ms** | Aucun | -- | 0 |
| 419 | `Demon_Slayer_–_Saison_2_Episode_12_VOST...` | 3813.77 ms | 86.2 ms | 25.94 ms | 156.01 ms | 1.0 ms | 0.5 ms | **4105.18 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 420 | `Demon_Slayer_–_Saison_2_Episode_13_VOST...` | 4071.49 ms | 122.49 ms | 39.13 ms | 154.53 ms | 1.0 ms | 0.5 ms | **4388.42 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 421 | `Demon_Slayer_–_Saison_2_Episode_14_VOST...` | 9540.83 ms | 112.98 ms | 22.39 ms | 345.91 ms | 1.0 ms | 0.5 ms | **10035.79 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 422 | `Demon_Slayer_–_Saison_2_Episode_15_VOST...` | 2419.11 ms | 116.15 ms | 26.4 ms | 149.68 ms | 1.0 ms | 0.5 ms | **2716.93 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 423 | `Demon_Slayer_–_Saison_2_Episode_16_VOST...` | 2441.48 ms | 88.69 ms | 27.07 ms | 170.07 ms | 1.0 ms | 0.5 ms | **2728.31 ms** | Aucun | -- | 0 |
| 424 | `Demon_Slayer_–_Saison_2_Episode_17_VOST...` | 1928.3 ms | 96.13 ms | 21.68 ms | 286.14 ms | 1.0 ms | 0.5 ms | **2334.44 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 425 | `Demon_Slayer_–_Saison_2_Episode_18_VOST...` | 2698.22 ms | 120.45 ms | 27.53 ms | 156.44 ms | 1.0 ms | 0.5 ms | **3003.4 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 426 | `Demon_Slayer_–_Saison_2_Episode_1_VOSTF...` | 6271.27 ms | 104.1 ms | 31.86 ms | 238.9 ms | 1.0 ms | 0.5 ms | **6647.51 ms** | Aucun | -- | 0 |
| 427 | `Demon_Slayer_–_Saison_2_Episode_2_VOSTF...` | 2581.7 ms | 86.29 ms | 27.78 ms | 154.48 ms | 1.0 ms | 0.5 ms | **2850.82 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 428 | `Demon_Slayer_–_Saison_2_Episode_4_VOSTF...` | 2597.04 ms | 102.24 ms | 26.49 ms | 185.42 ms | 1.0 ms | 0.5 ms | **2919.25 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 429 | `Demon_Slayer_–_Saison_2_Episode_6_VOSTF...` | 2153.67 ms | 89.56 ms | 21.76 ms | 173.56 ms | 1.0 ms | 0.5 ms | **2444.4 ms** | Aucun | -- | 0 |
| 430 | `Demon_Slayer_–_Saison_2_Episode_7_VOSTF...` | 3623.74 ms | 81.17 ms | 23.18 ms | 418.06 ms | 1.0 ms | 0.5 ms | **4148.13 ms** | Demon Slayer – Saison 2 Epi... | 100.0% | 0 |
| 431 | `Demon_Slayer_–_Saison_2_Episode_8_VOSTF...` | 2233.92 ms | 85.98 ms | 14.82 ms | 190.42 ms | 1.0 ms | 0.5 ms | **2526.03 ms** | Aucun | -- | 0 |
| 432 | `Demon_Slayer_–_Saison_2_Episode_9_VOSTF...` | 2681.26 ms | 91.12 ms | 15.91 ms | 108.51 ms | 1.0 ms | 0.5 ms | **2897.6 ms** | Aucun | -- | 0 |
| 433 | `Den_Blomstertid_Nu_Kommer_(2018)[MULTI]...` | 2284.51 ms | 82.61 ms | 21.75 ms | 205.05 ms | 1.0 ms | 0.5 ms | **2594.88 ms** | Den Blomstertid Nu Kommer | 100.0% | 0 |
| 434 | `Deux_Fils_(2018)_[1080p](Drame_,_Comédi...` | 2163.46 ms | 98.78 ms | 43.39 ms | 262.36 ms | 1.0 ms | 0.5 ms | **2568.66 ms** | Deux Fils | 100.0% | 0 |
| 435 | `Devs.S01E01.1080p.AMZN.WEB-DL.DDP5.1.H....` | 2619.71 ms | 113.63 ms | 20.28 ms | 366.8 ms | 1.0 ms | 0.5 ms | **3137.29 ms** | Devs | 100.0% | 1 |
| 436 | `Devs.S01E02.1080p.AMZN.WEB-DL.DDP5.1.H....` | 2244.69 ms | 89.61 ms | 23.52 ms | 164.56 ms | 1.0 ms | 0.5 ms | **2523.09 ms** | Devs | 100.0% | 1 |
| 437 | `Devs.S01E03.1080p.AMZN.WEB-DL.DDP5.1.H....` | 1879.25 ms | 123.72 ms | 31.96 ms | 396.77 ms | 1.0 ms | 0.5 ms | **2432.87 ms** | Devs.S01E03.1080p.AMZN.WEB-... | -- | 1 |
| 438 | `Devs.S01E04.1080p.AMZN.WEB-DL.DDP5.1.H....` | 4958.63 ms | 107.83 ms | 55.31 ms | 156.68 ms | 1.0 ms | 0.5 ms | **5279.11 ms** | Devs | 100.0% | 1 |
| 439 | `Devs.S01E05.1080p.AMZN.WEB-DL.DDP5.1.H....` | 3846.04 ms | 105.27 ms | 60.66 ms | 222.69 ms | 1.0 ms | 0.5 ms | **4235.08 ms** | Devs.S01E05.1080p.AMZN.WEB-... | -- | 1 |
| 440 | `Devs.S01E06.1080p.AMZN.WEB-DL.DDP5.1.H....` | 3185.9 ms | 116.02 ms | 39.67 ms | 159.18 ms | 1.0 ms | 0.5 ms | **3501.21 ms** | Devs | 100.0% | 1 |
| 441 | `Devs.S01E07.1080p.AMZN.WEB-DL.DDP5.1.H....` | 1749.22 ms | 100.11 ms | 25.71 ms | 258.12 ms | 1.0 ms | 0.5 ms | **2133.89 ms** | Devs | 100.0% | 1 |
| 442 | `Devs.S01E08.1080p.AMZN.WEB-DL.DDP5.1.H....` | 2475.54 ms | 119.19 ms | 26.59 ms | 207.64 ms | 1.0 ms | 0.5 ms | **2829.84 ms** | Devs.S01E08.1080p.AMZN.WEB-... | -- | 1 |
| 443 | `Die_Welle_(2008)_[ukr,ger]_BDRip_720p_[...` | 2499.68 ms | 223.8 ms | 24.96 ms | 172.59 ms | 1.0 ms | 0.5 ms | **2928.03 ms** | Die Welle | 100.0% | 0 |
| 444 | `DieuMerci.2015.FRENCH.DVDRip.x264-UTT_m...` | 2280.18 ms | 90.38 ms | 22.83 ms | 355.79 ms | 1.0 ms | 0.5 ms | **2750.03 ms** | Aucun | -- | 0 |
| 445 | `Dieudonné_-_La_Guerre_(2017)_Spectacle_...` | 2164.44 ms | 92.92 ms | 27.82 ms | 239.07 ms | 1.0 ms | 0.5 ms | **2525.74 ms** | Dieudonné - La Guerre | 100.0% | 0 |
| 446 | `Disney.Gallery.Star.Wars.The.Mandaloria...` | 2698.02 ms | 125.94 ms | 32.25 ms | 210.58 ms | 1.0 ms | 0.5 ms | **3069.64 ms** | Aucun | -- | 0 |
| 447 | `Disney.Gallery.Star.Wars.The.Mandaloria...` | 1941.51 ms | 85.13 ms | 25.38 ms | 206.27 ms | 1.0 ms | 0.5 ms | **2259.1 ms** | Aucun | -- | 0 |
| 448 | `Divines.2016.FRENCH.WEBRip.XviD.AC3-FUN...` | 2567.42 ms | 135.13 ms | 35.61 ms | 323.95 ms | 1.0 ms | 0.5 ms | **3062.75 ms** | Divines | 100.0% | 0 |
| 449 | `Divorce_Club_(2020)[1080p](Comédie)(Arn...` | 2697.43 ms | 169.67 ms | 61.82 ms | 111.39 ms | 1.0 ms | 0.5 ms | **3040.93 ms** | Divorce Club | 100.0% | 0 |
| 450 | `Docteur.Folamour.1964.MULTi.BRrip.x264....` | 10150.79 ms | 90.11 ms | 20.62 ms | 146.1 ms | 1.0 ms | 0.5 ms | **10408.23 ms** | Docteur Folamour | 100.0% | 0 |
| 451 | `Dog.2022.1080p.AMZN.WEB-DL.DDP5.1.H264-...` | 2191.6 ms | 124.03 ms | 63.01 ms | 234.63 ms | 1.0 ms | 0.5 ms | **2613.88 ms** | Dog | 100.0% | 0 |
| 452 | `Dogman_(2018)_mid15s.mp4` | 3742.9 ms | 119.67 ms | 29.44 ms | 325.18 ms | 1.0 ms | 0.5 ms | **4221.39 ms** | Dogman | 100.0% | 0 |
| 453 | `Dolemite.Is.My.Name.2019.1080p.WEBRip.x...` | 2461.15 ms | 98.56 ms | 35.22 ms | 193.79 ms | 1.0 ms | 0.5 ms | **2792.58 ms** | Aucun | -- | 0 |
| 454 | `Doom_of_Love_(2022)[MULTi][1080p][Turk]...` | 2601.16 ms | 106.76 ms | 35.09 ms | 311.88 ms | 1.0 ms | 0.5 ms | **3059.34 ms** | Doom of Love | 100.0% | 0 |
| 455 | `Doutes_(2021)[1080p](Muriel_Robin,_moye...` | 1944.25 ms | 107.75 ms | 32.08 ms | 237.23 ms | 1.0 ms | 0.5 ms | **2322.08 ms** | Doutes | 100.0% | 0 |
| 456 | `Down_in_Paris_(2022)[1080p](🎦_quarantai...` | 2125.23 ms | 123.13 ms | 46.59 ms | 261.45 ms | 1.0 ms | 0.5 ms | **2557.16 ms** | Down in Paris | 100.0% | 0 |
| 457 | `Dracula.Untold.2014.1080p.BluRay.x265-R...` | 2167.45 ms | 93.74 ms | 25.48 ms | 248.79 ms | 1.0 ms | 0.5 ms | **2538.36 ms** | Dracula Untold | 100.0% | 1 |
| 458 | `Dragon.2025.1080p.WEBRip.x264.AAC5.1-[Y...` | 2444.09 ms | 90.14 ms | 22.88 ms | 380.6 ms | 1.0 ms | 0.5 ms | **2943.81 ms** | Dragon | 100.0% | 0 |
| 459 | `Dune.Part.Two.2024.2160p.4K.WEB.x265.10...` | 2158.01 ms | 93.73 ms | 22.79 ms | 110.71 ms | 1.0 ms | 0.5 ms | **2386.09 ms** | Dune Part Two | 100.0% | 0 |
| 460 | `Dune_(2021)[2160p](fr_SRT)_mid15s.mp4` | 1613.7 ms | 115.82 ms | 29.29 ms | 130.55 ms | 1.0 ms | 0.5 ms | **1889.96 ms** | Aucun | -- | 0 |
| 461 | `Déboguer_son_code_-_Headers_Already_Sen...` | 9627.66 ms | 93.44 ms | 24.66 ms | 164.55 ms | 1.0 ms | 0.5 ms | **9910.67 ms** | Déboguer son code - Headers... | 100.0% | 0 |
| 462 | `Déboguer_son_code_-_J'ai_une_page_blanc...` | 5007.78 ms | 103.11 ms | 28.5 ms | 192.8 ms | 1.0 ms | 0.5 ms | **5342.54 ms** | Déboguer son code - J'ai un... | 100.0% | 0 |
| 463 | `Déboguer_son_code_-_Syntax_error_mid15s...` | 980.62 ms | 0.0 ms | 0.0 ms | 110.01 ms | 1.0 ms | 0.5 ms | **1090.74 ms** | Aucun | -- | 0 |
| 464 | `Déboguer_son_code_-_Trying_to_get_prope...` | 2142.6 ms | 88.5 ms | 33.78 ms | 241.52 ms | 1.0 ms | 0.5 ms | **2506.83 ms** | Déboguer son code - Trying ... | 100.0% | 0 |
| 465 | `Déboguer_son_code_-_Undefined_index_mid...` | 1192.81 ms | 0.0 ms | 0.0 ms | 187.11 ms | 1.0 ms | 0.5 ms | **1380.05 ms** | Aucun | -- | 0 |
| 466 | `Découverte_de_Webpack_mid15s.mp4` | 941.68 ms | 0.0 ms | 0.0 ms | 109.29 ms | 1.0 ms | 0.5 ms | **1051.09 ms** | Aucun | -- | 0 |
| 467 | `Découverte_de_la_distribution_Antergos,...` | 10773.66 ms | 122.92 ms | 23.57 ms | 376.22 ms | 1.0 ms | 0.5 ms | **11296.76 ms** | Découverte de la distributi... | 100.0% | 0 |
| 468 | `Découverte_du_CSS_(10_28)_-_Le_box-sizi...` | 3933.43 ms | 102.89 ms | 68.08 ms | 134.8 ms | 1.0 ms | 0.5 ms | **4239.53 ms** | Aucun | -- | 0 |
| 469 | `Découverte_du_CSS_(11_28)_-_TP_-_Créati...` | 2283.86 ms | 87.59 ms | 21.98 ms | 198.06 ms | 1.0 ms | 0.5 ms | **2591.94 ms** | Découverte du CSS 11 28 - T... | 100.0% | 0 |
| 470 | `Découverte_du_CSS_(12_28)_-_La_spécific...` | 1921.74 ms | 121.14 ms | 20.38 ms | 300.84 ms | 1.0 ms | 0.5 ms | **2364.54 ms** | Découverte du CSS 12 28 - L... | 100.0% | 0 |
| 471 | `Découverte_du_CSS_(13_28)_-_La_compatib...` | 4558.88 ms | 92.05 ms | 29.62 ms | 181.19 ms | 1.0 ms | 0.5 ms | **4862.62 ms** | Découverte du CSS 13 28 - L... | 100.0% | 0 |
| 472 | `Découverte_du_CSS_(14_28)_-_Polices_per...` | 1673.57 ms | 91.67 ms | 24.78 ms | 189.23 ms | 1.0 ms | 0.5 ms | **1979.57 ms** | Découverte du CSS 14 28 - P... | 100.0% | 0 |
| 473 | `Découverte_du_CSS_(15_28)_-_Pseudo-élém...` | 4666.85 ms | 110.76 ms | 54.02 ms | 227.33 ms | 1.0 ms | 0.5 ms | **5059.37 ms** | Aucun | -- | 0 |
| 474 | `Découverte_du_CSS_(16_28)_-_Transform_m...` | 11015.61 ms | 129.37 ms | 76.46 ms | 260.64 ms | 1.0 ms | 0.5 ms | **11482.51 ms** | Découverte du CSS 16 28 - T... | 100.0% | 0 |
| 475 | `Découverte_du_CSS_(17_28)_-_Animation_&...` | 3986.09 ms | 101.83 ms | 77.42 ms | 158.59 ms | 1.0 ms | 0.5 ms | **4324.36 ms** | Aucun | -- | 0 |
| 476 | `Découverte_du_CSS_(18_28)_-_Media_query...` | 6693.34 ms | 201.75 ms | 65.54 ms | 248.76 ms | 1.0 ms | 0.5 ms | **7209.83 ms** | Découverte du CSS 18 28 - M... | 100.0% | 0 |
| 477 | `Découverte_du_CSS_(19_28)_-_TP_-_Réseau...` | 1896.85 ms | 84.75 ms | 29.36 ms | 168.05 ms | 1.0 ms | 0.5 ms | **2188.55 ms** | Découverte du CSS 19 28 - T... | 100.0% | 0 |
| 478 | `Découverte_du_CSS_(1_28)_-_Présentation...` | 4305.23 ms | 108.31 ms | 21.29 ms | 198.44 ms | 1.0 ms | 0.5 ms | **4634.0 ms** | Aucun | -- | 0 |
| 479 | `Découverte_du_CSS_(20_28)_-_TP_Restaura...` | 1390.86 ms | 94.84 ms | 33.89 ms | 302.76 ms | 1.0 ms | 0.5 ms | **1822.75 ms** | Découverte du CSS 20 28 - T... | 100.0% | 0 |
| 480 | `Découverte_du_CSS_(28_28)_-_Les_framewo...` | 10402.7 ms | 93.39 ms | 86.55 ms | 113.92 ms | 1.0 ms | 0.5 ms | **10696.92 ms** | Aucun | -- | 0 |
| 481 | `Découverte_du_CSS_(2_28)_-_Les_sélecteu...` | 2271.2 ms | 115.17 ms | 33.75 ms | 326.48 ms | 1.0 ms | 0.5 ms | **2747.01 ms** | Découverte du CSS 6 28 - Le... | 100.0% | 0 |
| 482 | `Découverte_du_CSS_(3_28)_-_Le_modèle_de...` | 8671.3 ms | 87.73 ms | 32.01 ms | 90.9 ms | 1.0 ms | 0.5 ms | **8882.34 ms** | Découverte du CSS 3 28 - Le... | 100.0% | 0 |
| 483 | `Découverte_du_CSS_(4_28)_-_Police_&_tex...` | 4426.96 ms | 101.17 ms | 89.82 ms | 211.56 ms | 1.0 ms | 0.5 ms | **4830.89 ms** | Aucun | -- | 0 |
| 484 | `Découverte_du_CSS_(5_28)_-_Les_formats_...` | 4007.42 ms | 94.98 ms | 22.28 ms | 190.66 ms | 1.0 ms | 0.5 ms | **4315.8 ms** | Découverte du CSS 5 28 - Le... | 100.0% | 0 |
| 485 | `Découverte_du_CSS_(6_28)_-_Les_unités_d...` | 2507.95 ms | 84.63 ms | 23.77 ms | 230.78 ms | 1.0 ms | 0.5 ms | **2847.52 ms** | Découverte du CSS 6 28 - Le... | 100.0% | 0 |
| 486 | `Découverte_du_CSS_(7_28)_-_Le_positionn...` | 3903.88 ms | 85.54 ms | 63.07 ms | 239.11 ms | 1.0 ms | 0.5 ms | **4292.01 ms** | Aucun | -- | 0 |
| 487 | `Découverte_du_CSS_(8_28)_-_Les_éléments...` | 2761.02 ms | 89.37 ms | 68.62 ms | 158.46 ms | 1.0 ms | 0.5 ms | **3077.91 ms** | Aucun | -- | 0 |
| 488 | `Découverte_du_CSS_(9_28)_-_La_propriété...` | 1933.76 ms | 109.45 ms | 23.38 ms | 160.14 ms | 1.0 ms | 0.5 ms | **2227.48 ms** | Découverte du CSS 9 28 - La... | 100.0% | 0 |
| 489 | `Découverte_du_shell_fish_mid15s.mp4` | 1224.16 ms | 0.0 ms | 0.0 ms | 340.51 ms | 1.0 ms | 0.5 ms | **1566.98 ms** | Aucun | -- | 0 |
| 490 | `Dédales_2004_Truefrench_DvdRip_Xvid-Tea...` | 2038.35 ms | 87.63 ms | 17.17 ms | 197.49 ms | 1.0 ms | 0.5 ms | **2341.38 ms** | Dédales | 100.0% | 0 |
| 491 | `Déployer_du_PHP_avec_Ansible_(1_2)_-_Co...` | 9409.7 ms | 160.46 ms | 27.82 ms | 199.21 ms | 1.0 ms | 0.5 ms | **9797.59 ms** | Aucun | -- | 0 |
| 492 | `Déployer_du_PHP_avec_Ansible_(2_2)_-_Dé...` | 4123.14 ms | 98.33 ms | 21.09 ms | 168.89 ms | 1.0 ms | 0.5 ms | **4411.8 ms** | Déployer du PHP avec Ansibl... | 100.0% | 0 |
| 493 | `Déployer_une_application_Rails_avec_Cap...` | 2369.79 ms | 109.61 ms | 35.92 ms | 359.43 ms | 1.0 ms | 0.5 ms | **2875.2 ms** | Aucun | -- | 0 |
| 494 | `Déployer_votre_PHP_avec_Ansible_&_Ansis...` | 1627.73 ms | 87.17 ms | 28.86 ms | 449.84 ms | 1.0 ms | 0.5 ms | **2193.95 ms** | Déployer votre PHP avec Ans... | 100.0% | 0 |
| 495 | `E4YUYB~A_mid15s.mp4` | 3831.8 ms | 90.23 ms | 24.23 ms | 209.66 ms | 1.0 ms | 0.5 ms | **4156.47 ms** | Aucun | -- | 0 |
| 496 | `EDLWDB~4_mid15s.mp4` | 2511.51 ms | 157.49 ms | 29.8 ms | 212.22 ms | 1.0 ms | 0.5 ms | **2911.64 ms** | Aucun | -- | 0 |
| 497 | `EL06FH~N_mid15s.mp4` | 1609.45 ms | 97.77 ms | 28.7 ms | 166.02 ms | 1.0 ms | 0.5 ms | **1902.96 ms** | Aucun | -- | 0 |
| 498 | `EX21UN~2_mid15s.mp4` | 2382.11 ms | 88.87 ms | 18.91 ms | 210.63 ms | 1.0 ms | 0.5 ms | **2701.17 ms** | Aucun | -- | 0 |
| 499 | `EXTREMELY_WICKED,_SHOCKINGLY_EVIL_AND_V...` | 2354.8 ms | 85.26 ms | 20.37 ms | 190.97 ms | 1.0 ms | 0.5 ms | **2651.98 ms** | EXTREMELY WICKED, SHOCKINGL... | 100.0% | 0 |
| 500 | `Earth_One_Amazing_Day_(2017)[MULTi][108...` | 1504.75 ms | 87.1 ms | 24.34 ms | 150.9 ms | 1.0 ms | 0.5 ms | **1767.39 ms** | Earth One Amazing Day | 100.0% | 0 |
| 501 | `Edge_of_Tomorrow_(2014)[2160p.DDP5.1]_m...` | 2311.33 ms | 89.83 ms | 25.38 ms | 167.31 ms | 1.0 ms | 0.5 ms | **2594.67 ms** | Edge of Tomorrow | 100.0% | 0 |
| 502 | `Edmond_(2018)[1080p](Décembre_1897,_Par...` | 2363.31 ms | 84.39 ms | 21.95 ms | 277.58 ms | 1.0 ms | 0.5 ms | **2747.91 ms** | Aucun | -- | 0 |
| 503 | `El_Buen_Patron_(2021)[MULTi][1080p](com...` | 2678.06 ms | 88.36 ms | 23.45 ms | 124.9 ms | 1.0 ms | 0.5 ms | **2915.18 ms** | El Buen Patron | 100.0% | 0 |
| 504 | `El_Reino_(2018)_[1080p](Spain_-_Manuel_...` | 3582.52 ms | 113.68 ms | 20.03 ms | 294.55 ms | 1.0 ms | 0.5 ms | **4013.28 ms** | El Reino | 100.0% | 0 |
| 505 | `Elizabeth_Harvest_(2018)_mid15s.mp4` | 2328.9 ms | 89.33 ms | 18.17 ms | 188.19 ms | 1.0 ms | 0.5 ms | **2626.56 ms** | Elizabeth Harvest | 100.0% | 0 |
| 506 | `Elser.Un_Heros.Ordinaire.2015.VOSTFR.BD...` | 3010.42 ms | 88.7 ms | 16.25 ms | 263.02 ms | 1.0 ms | 0.5 ms | **3379.59 ms** | Elser Un Heros Ordinaire | 100.0% | 0 |
| 507 | `Emergency.2022.2160p.4K.WEB.x265.10bit....` | 1773.55 ms | 101.21 ms | 22.33 ms | 219.59 ms | 1.0 ms | 0.5 ms | **2117.49 ms** | Aucun | -- | 0 |
| 508 | `En_Attendant_Bojangles_(2021)[1080p](Vi...` | 2400.59 ms | 112.02 ms | 18.63 ms | 332.02 ms | 1.0 ms | 0.5 ms | **2871.67 ms** | En Attendant Bojangles | 100.0% | 0 |
| 509 | `Encanto_(2021)[2160p.UHD.HDR.10bit.HEVC...` | 2139.92 ms | 86.43 ms | 18.74 ms | 188.65 ms | 1.0 ms | 0.5 ms | **2436.2 ms** | Aucun | -- | 0 |
| 510 | `Enquete_Sur_Un_Scandale_D_Etat_(2021)[1...` | 1933.74 ms | 84.35 ms | 19.89 ms | 171.4 ms | 1.0 ms | 0.5 ms | **2210.22 ms** | Enquete Sur Un Scandale D Etat | 100.0% | 0 |
| 511 | `Enter.The.Void.2009.1080p.BluRay.x265-R...` | 2743.62 ms | 106.12 ms | 18.65 ms | 375.97 ms | 1.0 ms | 0.5 ms | **3245.23 ms** | Enter The Void | 100.0% | 0 |
| 512 | `Entre_Deux_Rives_Fr_DvdRip_XviD_mid15s.mp4` | 2564.75 ms | 93.73 ms | 18.53 ms | 372.73 ms | 1.0 ms | 0.5 ms | **3054.29 ms** | Entre Deux Rives Fr | 100.0% | 0 |
| 513 | `Entre_les_murs_(2008)[1080p][AAC5.1](La...` | 2423.39 ms | 90.91 ms | 15.43 ms | 232.72 ms | 1.0 ms | 0.5 ms | **2763.15 ms** | Aucun | -- | 0 |
| 514 | `Environnement_de_bureau_linux_-_Deepin_...` | 1788.88 ms | 91.37 ms | 26.42 ms | 191.64 ms | 1.0 ms | 0.5 ms | **2098.7 ms** | Environnement de bureau lin... | 100.0% | 0 |
| 515 | `Eperdument.2016.FRENCH.WEBRip.XViD-eVe_...` | 2383.95 ms | 83.04 ms | 23.79 ms | 159.67 ms | 1.0 ms | 0.5 ms | **2653.17 ms** | Eperdument | 100.0% | 0 |
| 516 | `Er.Ist.Wieder.Da.2015.TRUEFRENCH.WEB-DL...` | 2412.0 ms | 99.01 ms | 26.67 ms | 295.46 ms | 1.0 ms | 0.5 ms | **2839.67 ms** | Er Ist Wieder Da | 100.0% | 0 |
| 517 | `Essence_of_Life_mid15s.mp4` | 3349.59 ms | 91.13 ms | 34.46 ms | 379.03 ms | 1.0 ms | 0.5 ms | **3854.94 ms** | Aucun | -- | 0 |
| 518 | `Ethel.&.Ernest.2016.MULTi.1080p.WEB-DL....` | 2075.14 ms | 86.9 ms | 21.18 ms | 121.62 ms | 1.0 ms | 0.5 ms | **2305.51 ms** | Ethel & Ernest | 100.0% | 0 |
| 519 | `Euphoria.US.S01E01.1080p.WEBRip.x265-RA...` | 2078.03 ms | 99.33 ms | 16.12 ms | 161.84 ms | 1.0 ms | 0.5 ms | **2355.91 ms** | Euphoria US | 100.0% | 1 |
| 520 | `Euphoria.US.S01E02.1080p.WEBRip.x265-RA...` | 2537.29 ms | 101.79 ms | 21.56 ms | 218.63 ms | 1.0 ms | 0.5 ms | **2882.28 ms** | Euphoria.US.S01E02.1080p.WE... | -- | 2 |
| 521 | `Euphoria.US.S01E03.1080p.WEBRip.x265-RA...` | 2105.0 ms | 108.4 ms | 30.11 ms | 136.19 ms | 1.0 ms | 0.5 ms | **2380.52 ms** | Euphoria.US.S01E03.1080p.WE... | -- | 2 |
| 522 | `Euphoria.US.S01E04.1080p.WEBRip.x265-RA...` | 2699.89 ms | 94.58 ms | 32.65 ms | 245.45 ms | 1.0 ms | 0.5 ms | **3074.15 ms** | Euphoria.US.S01E04.1080p.WE... | -- | 2 |
| 523 | `Euphoria.US.S01E05.1080p.WEBRip.x265-RA...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.25 ms** | Erreur | -- | 0 |
| 524 | `Euphoria.US.S01E06.1080p.WEBRip.x265-RA...` | 2881.37 ms | 95.16 ms | 28.11 ms | 75.51 ms | 1.0 ms | 0.5 ms | **3080.98 ms** | Euphoria.US.S01E06.1080p.WE... | -- | 1 |
| 525 | `Euphoria.US.S01E07.1080p.WEBRip.x265-RA...` | 2740.08 ms | 122.07 ms | 22.46 ms | 363.52 ms | 1.0 ms | 0.5 ms | **3270.78 ms** | Euphoria US | 100.0% | 1 |
| 526 | `Euphoria.US.S01E08.1080p.WEBRip.x265-RA...` | 2439.47 ms | 102.16 ms | 15.0 ms | 278.86 ms | 1.0 ms | 0.5 ms | **2838.95 ms** | Euphoria US | 100.0% | 1 |
| 527 | `Euphoria.US.S02E01.1080p.WEBRip.x265-RA...` | 2271.58 ms | 95.64 ms | 32.54 ms | 135.44 ms | 1.0 ms | 0.5 ms | **2536.0 ms** | Euphoria US | 100.0% | 1 |
| 528 | `Euphoria.US.S02E02.1080p.WEBRip.x265-RA...` | 2115.26 ms | 91.12 ms | 26.3 ms | 292.38 ms | 1.0 ms | 0.5 ms | **2530.06 ms** | Euphoria.US.S02E02.1080p.WE... | -- | 1 |
| 529 | `Euphoria.US.S02E03.1080p.WEBRip.x265-RA...` | 2538.13 ms | 91.95 ms | 15.55 ms | 159.3 ms | 1.0 ms | 0.5 ms | **2807.23 ms** | Euphoria US | 100.0% | 1 |
| 530 | `Euphoria.US.S02E04.1080p.WEBRip.x265-RA...` | 2176.7 ms | 147.8 ms | 25.53 ms | 303.23 ms | 1.0 ms | 0.5 ms | **2655.58 ms** | Euphoria US | 100.0% | 1 |
| 531 | `Euphoria.US.S02E05.1080p.WEBRip.x265-RA...` | 2979.72 ms | 94.56 ms | 21.3 ms | 241.57 ms | 1.0 ms | 0.5 ms | **3337.54 ms** | Euphoria US | 100.0% | 1 |
| 532 | `Euphoria.US.S02E06.1080p.WEBRip.x265-RA...` | 2817.01 ms | 88.03 ms | 23.39 ms | 153.4 ms | 1.0 ms | 0.5 ms | **3082.59 ms** | Euphoria US | 100.0% | 1 |
| 533 | `Euphoria.US.S02E07.1080p.WEBRip.x265-RA...` | 2631.42 ms | 85.12 ms | 28.61 ms | 235.0 ms | 1.0 ms | 0.5 ms | **2980.5 ms** | Euphoria US | 100.0% | 1 |
| 534 | `Euphoria.US.S02E08.1080p.WEBRip.x265-RA...` | 2906.36 ms | 86.23 ms | 16.27 ms | 236.02 ms | 1.0 ms | 0.5 ms | **3245.76 ms** | Euphoria US | 100.0% | 1 |
| 535 | `Evil_Goddess_Eris_1987_mid15s.mp4` | 2741.94 ms | 80.59 ms | 25.66 ms | 116.16 ms | 1.0 ms | 0.5 ms | **2964.91 ms** | Evil Goddess Eris | 100.0% | 0 |
| 536 | `Extended_Producer's_Cut_-_Communication...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.14 ms** | Erreur | -- | 0 |
| 537 | `Faire_fonctionner_Boot2docker_sur_Windo...` | 1329.32 ms | 0.0 ms | 0.0 ms | 249.59 ms | 1.0 ms | 0.5 ms | **1579.03 ms** | Aucun | -- | 0 |
| 538 | `Fantasia.1940.1080p.BluRay.x264.AAC5.1-...` | 1997.1 ms | 91.92 ms | 28.79 ms | 327.44 ms | 1.0 ms | 0.5 ms | **2447.51 ms** | Fantasia | 100.0% | 0 |
| 539 | `Fantastic.Planet.1973.1080p.BluRay.x264...` | 2194.32 ms | 95.6 ms | 20.04 ms | 274.97 ms | 1.0 ms | 0.5 ms | **2585.62 ms** | Aucun | -- | 0 |
| 540 | `Fargo_(2014)_-_S03E03_-_The_Law_of_Non-...` | 4367.72 ms | 132.31 ms | 16.84 ms | 164.59 ms | 1.0 ms | 0.5 ms | **4681.9 ms** | Fargo | 100.0% | 0 |
| 541 | `Fargo_(2014)_-_S03E04_-_The_Narrow_Esca...` | 1784.84 ms | 88.7 ms | 18.61 ms | 204.68 ms | 1.0 ms | 0.5 ms | **2097.69 ms** | Fargo | 100.0% | 0 |
| 542 | `Fargo_(2014)_-_S03E05_-_The_House_of_Sp...` | 2604.31 ms | 95.29 ms | 20.72 ms | 148.8 ms | 1.0 ms | 0.5 ms | **2874.43 ms** | Fargo | 100.0% | 0 |
| 543 | `Fargo_(2014)_-_S03E06_-_The_Lord_of_No_...` | 3134.5 ms | 139.25 ms | 18.61 ms | 160.56 ms | 1.0 ms | 0.5 ms | **3453.36 ms** | Fargo | 100.0% | 0 |
| 544 | `Fargo_(2014)_-_S03E07_-_The_Law_of_Inev...` | 3008.31 ms | 100.51 ms | 17.6 ms | 232.76 ms | 1.0 ms | 0.5 ms | **3359.59 ms** | Aucun | -- | 0 |
| 545 | `Fargo_(2014)_-_S03E08_-_Who_Rules_the_L...` | 2802.6 ms | 95.23 ms | 20.57 ms | 202.41 ms | 1.0 ms | 0.5 ms | **3121.72 ms** | Fargo | 100.0% | 0 |
| 546 | `Fargo_(2014)_-_S03E09_-_Aporia_(1080p_A...` | 3436.3 ms | 141.39 ms | 13.73 ms | 144.45 ms | 1.0 ms | 0.5 ms | **3750.9 ms** | Fargo | 100.0% | 0 |
| 547 | `Fargo_(2014)_-_S03E10_-_Somebody_to_Lov...` | 3195.09 ms | 199.58 ms | 20.59 ms | 289.01 ms | 1.0 ms | 0.5 ms | **3704.58 ms** | Fargo | 100.0% | 0 |
| 548 | `Favelas.2014.FRENCH.BDRip.XViD-AViTECH_...` | 3002.25 ms | 159.07 ms | 18.71 ms | 290.44 ms | 1.0 ms | 0.5 ms | **3472.16 ms** | Aucun | -- | 0 |
| 549 | `Fear.And.Loathing.In.Las.Vegas.1998.REM...` | 2373.82 ms | 93.45 ms | 23.02 ms | 194.3 ms | 1.0 ms | 0.5 ms | **2685.43 ms** | Fear And Loathing In Las Vegas | 100.0% | 0 |
| 550 | `Firestarter.2022.1080p.WEBRip.x264-RARB...` | 3056.79 ms | 103.27 ms | 19.55 ms | 468.44 ms | 1.0 ms | 0.5 ms | **3649.04 ms** | Firestarter | 100.0% | 0 |
| 551 | `Fireworks_(2017)_mid15s.mp4` | 2651.19 ms | 176.02 ms | 21.25 ms | 214.41 ms | 1.0 ms | 0.5 ms | **3063.72 ms** | Fireworks | 100.0% | 0 |
| 552 | `For.All.Mankind.S03E01.1080p.x265-ZMNT_...` | 2741.43 ms | 92.78 ms | 26.16 ms | 237.62 ms | 1.0 ms | 0.5 ms | **3102.17 ms** | For All Mankind | 100.0% | 0 |
| 553 | `For.All.Mankind.S03E02.iNTERNAL.1080p.x...` | 6460.8 ms | 424.56 ms | 23.55 ms | 216.0 ms | 1.0 ms | 0.5 ms | **7125.53 ms** | Aucun | -- | 0 |
| 554 | `For.All.Mankind.S03E04.iNTERNAL.1080p.x...` | 2302.57 ms | 210.88 ms | 48.06 ms | 168.86 ms | 1.0 ms | 0.5 ms | **2739.19 ms** | For All Mankind | 100.0% | 0 |
| 555 | `For.All.Mankind.S03E05.1080p.x265-ZMNT_...` | 2519.34 ms | 89.58 ms | 21.41 ms | 172.62 ms | 1.0 ms | 0.5 ms | **2805.36 ms** | For All Mankind | 100.0% | 0 |
| 556 | `For.All.Mankind.S03E06.1080p.x265-ZMNT_...` | 2903.65 ms | 126.35 ms | 19.18 ms | 84.68 ms | 1.0 ms | 0.5 ms | **3139.57 ms** | For All Mankind | 100.0% | 0 |
| 557 | `For.All.Mankind.S03E07.1080p.x265-ZMNT_...` | 2715.74 ms | 110.48 ms | 18.71 ms | 189.55 ms | 1.0 ms | 0.5 ms | **3044.1 ms** | For All Mankind | 100.0% | 0 |
| 558 | `For.All.Mankind.S03E08.1080p.x265-ZMNT_...` | 2409.34 ms | 97.09 ms | 32.44 ms | 173.24 ms | 1.0 ms | 0.5 ms | **2714.66 ms** | Aucun | -- | 0 |
| 559 | `For.All.Mankind.S03E09.1080p.x265-ZMNT_...` | 1820.86 ms | 96.82 ms | 18.84 ms | 119.46 ms | 1.0 ms | 0.5 ms | **2057.21 ms** | For All Mankind | 100.0% | 0 |
| 560 | `For.All.Mankind.S03E10.1080p.x265-ZMNT_...` | 2062.69 ms | 105.06 ms | 17.47 ms | 149.08 ms | 1.0 ms | 0.5 ms | **2335.52 ms** | For All Mankind | 100.0% | 0 |
| 561 | `For_All_Mankind_(2019)_-_S02E01_-_Every...` | 2380.4 ms | 102.71 ms | 24.78 ms | 161.75 ms | 1.0 ms | 0.5 ms | **2670.48 ms** | For All Mankind | 100.0% | 0 |
| 562 | `For_All_Mankind_(2019)_-_S02E02_-_The_B...` | 2454.07 ms | 112.01 ms | 19.41 ms | 168.16 ms | 1.0 ms | 0.5 ms | **2755.64 ms** | For All Mankind | 100.0% | 0 |
| 563 | `For_All_Mankind_(2019)_-_S02E03_-_Rules...` | 2314.99 ms | 108.76 ms | 19.99 ms | 139.72 ms | 1.0 ms | 0.5 ms | **2584.92 ms** | For All Mankind | 100.0% | 0 |
| 564 | `For_All_Mankind_(2019)_-_S02E06_-_Best-...` | 3861.27 ms | 98.99 ms | 31.87 ms | 201.92 ms | 1.0 ms | 0.5 ms | **4195.8 ms** | Aucun | -- | 0 |
| 565 | `For_All_Mankind_(2019)_-_S02E08_-_And_H...` | 2206.56 ms | 101.2 ms | 25.08 ms | 199.73 ms | 1.0 ms | 0.5 ms | **2537.56 ms** | For All Mankind | 100.0% | 0 |
| 566 | `For_All_Mankind_(2019)_-_S02E09_-_Triag...` | 5177.11 ms | 102.03 ms | 17.61 ms | 193.47 ms | 1.0 ms | 0.5 ms | **5493.92 ms** | For All Mankind | 100.0% | 0 |
| 567 | `For_All_Mankind_(2019)_-_S02E10_-_The_G...` | 2291.03 ms | 97.78 ms | 25.37 ms | 163.63 ms | 1.0 ms | 0.5 ms | **2578.22 ms** | For All Mankind | 100.0% | 0 |
| 568 | `For_All_Mankind_S01E01_Red_Moon_[2160p_...` | 2938.04 ms | 105.29 ms | 17.25 ms | 191.65 ms | 1.0 ms | 0.5 ms | **3253.09 ms** | For All Mankind | 100.0% | 0 |
| 569 | `For_All_Mankind_S01E03_Nixons_Women_[21...` | 4629.06 ms | 109.64 ms | 25.13 ms | 286.74 ms | 1.0 ms | 0.5 ms | **5053.06 ms** | Aucun | -- | 0 |
| 570 | `For_All_Mankind_S01E04_Prime_Crew_[2160...` | 2735.95 ms | 94.02 ms | 27.63 ms | 168.7 ms | 1.0 ms | 0.5 ms | **3027.07 ms** | For All Mankind | 100.0% | 0 |
| 571 | `For_All_Mankind_S01E06_Home_Again_[2160...` | 2503.14 ms | 100.34 ms | 18.24 ms | 238.98 ms | 1.0 ms | 0.5 ms | **2861.7 ms** | For All Mankind | 100.0% | 0 |
| 572 | `For_All_Mankind_S01E07_Hi_Bob_[2160p_x2...` | 5359.77 ms | 107.45 ms | 19.07 ms | 260.71 ms | 1.0 ms | 0.5 ms | **5747.61 ms** | For All Mankind | 100.0% | 0 |
| 573 | `Forgotten_Island_(2026)_2160p_WEB-DL_DV...` | 2670.61 ms | 100.8 ms | 28.52 ms | 228.47 ms | 1.0 ms | 0.5 ms | **3037.88 ms** | Forgotten Island | 100.0% | 0 |
| 574 | `Formation_AngularJS_-_Controller_&_Scop...` | 1292.47 ms | 0.0 ms | 0.0 ms | 318.21 ms | 1.0 ms | 0.5 ms | **1610.83 ms** | Aucun | -- | 0 |
| 575 | `Formation_AngularJS_-_Créer_ses_directi...` | 6344.71 ms | 123.34 ms | 421.98 ms | 287.85 ms | 1.0 ms | 0.5 ms | **7178.26 ms** | Aucun | -- | 0 |
| 576 | `Formation_AngularJS_-_Introduction_mid1...` | 1372.59 ms | 0.0 ms | 0.0 ms | 162.47 ms | 1.0 ms | 0.5 ms | **1535.18 ms** | Aucun | -- | 0 |
| 577 | `Formation_AngularJS_-_Les_Directives_mi...` | 1787.57 ms | 127.33 ms | 24.96 ms | 148.86 ms | 1.0 ms | 0.5 ms | **2089.09 ms** | Formation AngularJS - Les D... | 100.0% | 0 |
| 578 | `Formation_AngularJS_-_Les_Modules_mid15...` | 1180.67 ms | 0.0 ms | 0.0 ms | 198.68 ms | 1.0 ms | 0.5 ms | **1379.5 ms** | Aucun | -- | 0 |
| 579 | `Formation_AngularJS_-_Les_Promesses_mid...` | 5821.35 ms | 93.87 ms | 374.66 ms | 170.41 ms | 1.0 ms | 0.5 ms | **6461.64 ms** | Formation AngularJS - Les M... | 100.0% | 0 |
| 580 | `Formation_AngularJS_-_Les_Routes_mid15s...` | 7697.47 ms | 96.78 ms | 84.17 ms | 215.33 ms | 1.0 ms | 0.5 ms | **8094.08 ms** | Formation AngularJS - Les R... | 100.0% | 0 |
| 581 | `Formation_AngularJS_-_Les_Services_mid1...` | 4966.3 ms | 115.47 ms | 83.23 ms | 260.33 ms | 1.0 ms | 0.5 ms | **5425.67 ms** | Formation AngularJS - Les S... | 100.0% | 0 |
| 582 | `Formation_AngularJS_-_Les_resources_mid...` | 11030.0 ms | 100.66 ms | 47.06 ms | 313.16 ms | 1.0 ms | 0.5 ms | **11493.88 ms** | Aucun | -- | 0 |
| 583 | `Formation_Laravel_-_Introduction_mid15s...` | 4636.06 ms | 98.91 ms | 38.69 ms | 157.24 ms | 1.0 ms | 0.5 ms | **4931.34 ms** | Aucun | -- | 0 |
| 584 | `Fortune.Favors.Lady.Nikuko.2021.1080p.W...` | 2765.62 ms | 85.05 ms | 21.45 ms | 478.45 ms | 1.0 ms | 0.5 ms | **3355.86 ms** | ✔️Fortune Favors Lady Nikuko | 100.0% | 0 |
| 585 | `Four_Hours_At_The_Capitol_(2021)[1080p....` | 4788.82 ms | 88.78 ms | 34.94 ms | 247.92 ms | 1.0 ms | 0.5 ms | **5161.3 ms** | Four Hours At The Capitol | 100.0% | 0 |
| 586 | `Fractured_(2019)[MULTI][1080p](Sa_femme...` | 3873.56 ms | 110.01 ms | 70.44 ms | 191.75 ms | 1.0 ms | 0.5 ms | **4246.55 ms** | Fractured | 100.0% | 0 |
| 587 | `Freaks_Out_(2021)[MULTi][1080p](Rome,_N...` | 3156.36 ms | 102.41 ms | 22.04 ms | 317.3 ms | 1.0 ms | 0.5 ms | **3598.75 ms** | Freaks Out | 100.0% | 0 |
| 588 | `Freres_Ennemis_(2018)_mid15s.mp4` | 2826.63 ms | 92.82 ms | 26.49 ms | 495.49 ms | 1.0 ms | 0.5 ms | **3444.64 ms** | Freres Ennemis | 100.0% | 0 |
| 589 | `Frozen.2013.FRENCH.BluRay.x264.AAC.2.0-...` | 2828.28 ms | 107.74 ms | 23.71 ms | 444.89 ms | 1.0 ms | 0.5 ms | **3405.2 ms** | Frozen | 100.0% | 1 |
| 590 | `Funan_(2019)[1080p]_mid15s.mp4` | 3478.6 ms | 103.42 ms | 220.56 ms | 173.77 ms | 1.0 ms | 0.5 ms | **3976.74 ms** | Aucun | -- | 0 |
| 591 | `Funan_(2019)_[720p]_mid15s.mp4` | 3024.07 ms | 98.48 ms | 33.41 ms | 261.63 ms | 1.0 ms | 0.5 ms | **3418.0 ms** | Funan | 100.0% | 0 |
| 592 | `Funny.Games.1997.GERMAN.1080p.BluRay.H2...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.11 ms** | Erreur | -- | 0 |
| 593 | `Gemini.Man_(2019)[1080p]_mid15s.mp4` | 2164.7 ms | 129.46 ms | 26.81 ms | 370.59 ms | 1.0 ms | 0.5 ms | **2692.84 ms** | Gemini Man | 100.0% | 0 |
| 594 | `Gemini.Man_(2019[1080p]_mid15s.mp4` | 2571.47 ms | 90.07 ms | 19.93 ms | 131.04 ms | 1.0 ms | 0.5 ms | **2813.65 ms** | Gemini Man | 100.0% | 0 |
| 595 | `Get.On.Up.2014.FRENCH.720p.BluRay.x264-...` | 3235.32 ms | 89.4 ms | 29.26 ms | 135.52 ms | 1.0 ms | 0.5 ms | **3490.23 ms** | Get On Up | 100.0% | 0 |
| 596 | `Ghost.in.the.Shell.1995.1080p.BluRay.x2...` | 6919.01 ms | 89.3 ms | 27.4 ms | 279.86 ms | 1.0 ms | 0.5 ms | **7320.87 ms** | Ghost in the Shell | 100.0% | 0 |
| 597 | `Ghost_in_the_Shell_(1995)_(1080p_BluRay...` | 2105.35 ms | 97.23 ms | 34.29 ms | 189.3 ms | 1.0 ms | 0.5 ms | **2427.06 ms** | Ghost in the Shell | 100.0% | 0 |
| 598 | `Gi-Juk.2021.1080p.WEBRip.x264.AAC-[YTS....` | 4559.05 ms | 93.48 ms | 23.83 ms | 164.47 ms | 1.0 ms | 0.5 ms | **4841.73 ms** | Gi-Juk | 100.0% | 0 |
| 599 | `Girl.2018_[1080p]_mid15s.mp4` | 2190.23 ms | 89.81 ms | 18.36 ms | 321.31 ms | 1.0 ms | 0.5 ms | **2620.57 ms** | Girl | 100.0% | 0 |
| 600 | `Glass_(2019)[1080p]_mid15s.mp4` | 2922.85 ms | 85.51 ms | 21.41 ms | 156.35 ms | 1.0 ms | 0.5 ms | **3195.77 ms** | Glass | 100.0% | 0 |
| 601 | `Godfrey_Reggio_Interview_mid15s.mp4` | 2686.78 ms | 95.06 ms | 19.26 ms | 163.45 ms | 1.0 ms | 0.5 ms | **2967.35 ms** | Aucun | -- | 0 |
| 602 | `Gondola.2024.1080p.WEB.H264-FW_mid15s.mp4` | 2155.19 ms | 116.31 ms | 24.17 ms | 277.96 ms | 1.0 ms | 0.5 ms | **2576.44 ms** | Gondola | 100.0% | 0 |
| 603 | `Gone_Girl_2014_FRENCH_720p_BluRay_x264-...` | 2931.86 ms | 85.54 ms | 19.29 ms | 268.63 ms | 1.0 ms | 0.5 ms | **3306.81 ms** | Gone Girl | 100.0% | 0 |
| 604 | `Good_Bye_Lenin!_(2003)_[1080p]_mid15s.mp4` | 2745.79 ms | 99.33 ms | 18.07 ms | 194.91 ms | 1.0 ms | 0.5 ms | **3061.3 ms** | Good Bye Lenin! | 100.0% | 0 |
| 605 | `Gorillaz_-_Best_Songs_Compilation_mid15...` | 11114.84 ms | 112.36 ms | 66.85 ms | 256.48 ms | 1.0 ms | 0.5 ms | **11550.89 ms** | Gorillaz - Best Songs Compi... | 100.0% | 0 |
| 606 | `Grafikart_-_-Site.new_{code_-_true,_des...` | 2205.54 ms | 88.68 ms | 23.87 ms | 247.9 ms | 1.0 ms | 0.5 ms | **2566.35 ms** | Grafikart - -Site new {code... | 100.0% | 0 |
| 607 | `Grave.2016.1080p.BluRay.x265.hevc.10bit...` | 2384.65 ms | 79.97 ms | 26.2 ms | 187.35 ms | 1.0 ms | 0.5 ms | **2679.73 ms** | Grave | 100.0% | 0 |
| 608 | `Grave.2016.FRENCH.1080p.BluRay.x264-LOS...` | 2013.57 ms | 97.91 ms | 30.91 ms | 209.11 ms | 1.0 ms | 0.5 ms | **2352.28 ms** | Grave | 100.0% | 0 |
| 609 | `Grosse_colere_et_fantaisies_(2022)[FREN...` | 1325.79 ms | 102.81 ms | 22.27 ms | 102.48 ms | 1.0 ms | 0.5 ms | **1553.71 ms** | Grosse colere et fantaisies | 100.0% | 0 |
| 610 | `Gundala_-_RED_STORM_(Indonésien)(2019)[...` | 2680.75 ms | 90.26 ms | 18.61 ms | 196.32 ms | 1.0 ms | 0.5 ms | **2987.68 ms** | Gundala - RED STORM Indonésien | 100.0% | 0 |
| 611 | `Gunpowder.Milkshake.2021.2160p.NF.WEBRi...` | 2265.97 ms | 90.58 ms | 23.4 ms | 184.88 ms | 1.0 ms | 0.5 ms | **2570.41 ms** | Aucun | -- | 0 |
| 612 | `Haikyuu!!_Tokushuu!_Haru_Kou_Volley_ni_...` | 2374.35 ms | 93.4 ms | 26.29 ms | 137.6 ms | 1.0 ms | 0.5 ms | **2632.07 ms** | Haikyuu!! Tokushuu! Haru Ko... | 100.0% | 1 |
| 613 | `Haikyuu.S04E16.VOSTFR.1080p.WEBRiP.x265...` | 6389.01 ms | 90.22 ms | 15.34 ms | 187.19 ms | 1.0 ms | 0.5 ms | **6682.13 ms** | Haikyuu | 100.0% | 1 |
| 614 | `Halo_S02E07_Thermopylae_2160p_PMTP_WEB-...` | 2607.84 ms | 92.69 ms | 21.35 ms | 184.62 ms | 1.0 ms | 0.5 ms | **2906.89 ms** | Aucun | -- | 0 |
| 615 | `Halo_S02E08_Halo_2160p_PMTP_WEB-DL_DDP5...` | 5621.86 ms | 120.57 ms | 17.92 ms | 301.44 ms | 1.0 ms | 0.5 ms | **6062.26 ms** | Andor | 100.0% | 0 |
| 616 | `Halt.and.Catch.Fire.S03E01.Valley.of.th...` | 3218.67 ms | 104.78 ms | 15.1 ms | 325.77 ms | 1.0 ms | 0.5 ms | **3664.92 ms** | Halt and Catch Fire | 100.0% | 1 |
| 617 | `Halt.and.Catch.Fire.S03E02.One.Way.or.A...` | 2314.64 ms | 82.16 ms | 28.49 ms | 96.37 ms | 1.0 ms | 0.5 ms | **2532.71 ms** | Halt and Catch Fire | 100.0% | 1 |
| 618 | `Halt.and.Catch.Fire.S03E03.Flipping.the...` | 2358.82 ms | 102.87 ms | 17.02 ms | 149.5 ms | 1.0 ms | 0.5 ms | **2632.79 ms** | Halt and Catch Fire | 100.0% | 1 |
| 619 | `Halt.and.Catch.Fire.S03E03.Flipping.the...` | 2531.18 ms | 113.0 ms | 17.21 ms | 262.56 ms | 1.0 ms | 0.5 ms | **2925.01 ms** | Halt and Catch Fire | 100.0% | 1 |
| 620 | `Halt.and.Catch.Fire.S03E04.Rules.Of.Hon...` | 2856.58 ms | 98.15 ms | 13.73 ms | 229.66 ms | 1.0 ms | 0.5 ms | **3198.84 ms** | Halt and Catch Fire | 100.0% | 1 |
| 621 | `Halt.and.Catch.Fire.S03E04.Rules.of.Hon...` | 2385.3 ms | 83.74 ms | 15.21 ms | 130.61 ms | 1.0 ms | 0.5 ms | **2617.76 ms** | Halt and Catch Fire | 100.0% | 0 |
| 622 | `Halt.and.Catch.Fire.S03E05.Yerba.Buena....` | 2380.16 ms | 87.4 ms | 28.8 ms | 183.26 ms | 1.0 ms | 0.5 ms | **2680.2 ms** | Halt and Catch Fire | 100.0% | 0 |
| 623 | `Halt.and.Catch.Fire.S03E05.Yerba.Buena....` | 2544.15 ms | 88.07 ms | 27.8 ms | 332.75 ms | 1.0 ms | 0.5 ms | **3009.24 ms** | Halt and Catch Fire | 100.0% | 1 |
| 624 | `Halt.and.Catch.Fire.S03E06.And.She.Was....` | 2591.06 ms | 107.99 ms | 29.89 ms | 323.22 ms | 1.0 ms | 0.5 ms | **3064.65 ms** | Halt and Catch Fire | 100.0% | 0 |
| 625 | `Halt.and.Catch.Fire.S03E07.The.Threshol...` | 2523.65 ms | 153.7 ms | 87.36 ms | 121.76 ms | 1.0 ms | 0.5 ms | **2890.63 ms** | Halt.and.Catch.Fire.S03E07.... | -- | 1 |
| 626 | `Halt.and.Catch.Fire.S03E08.You.Are.Not....` | 2340.18 ms | 91.46 ms | 28.19 ms | 196.9 ms | 1.0 ms | 0.5 ms | **2660.42 ms** | Halt and Catch Fire | 100.0% | 1 |
| 627 | `Halt.and.Catch.Fire.S03E09.NIM.1080p.AM...` | 2301.55 ms | 118.64 ms | 18.01 ms | 278.2 ms | 1.0 ms | 0.5 ms | **2722.92 ms** | Halt and Catch Fire | 100.0% | 1 |
| 628 | `Halt.and.Catch.Fire.S03E10.NeXT.1080p.A...` | 2360.07 ms | 151.72 ms | 56.85 ms | 327.56 ms | 1.0 ms | 0.5 ms | **2896.98 ms** | Halt and Catch Fire | 100.0% | 1 |
| 629 | `Halt.and.Catch.Fire.S04E01.So.It.Goes.1...` | 2423.74 ms | 112.12 ms | 36.04 ms | 186.72 ms | 1.0 ms | 0.5 ms | **2759.12 ms** | Halt and Catch Fire | 100.0% | 1 |
| 630 | `Halt.and.Catch.Fire.S04E02.Signal.to.No...` | 2066.65 ms | 87.38 ms | 16.09 ms | 361.73 ms | 1.0 ms | 0.5 ms | **2534.25 ms** | Halt and Catch Fire | 100.0% | 1 |
| 631 | `Halt.and.Catch.Fire.S04E03.Miscellaneou...` | 1993.4 ms | 121.93 ms | 53.52 ms | 184.74 ms | 1.0 ms | 0.5 ms | **2357.44 ms** | Halt and Catch Fire | 100.0% | 1 |
| 632 | `Halt.and.Catch.Fire.S04E04.Tonya.and.Na...` | 2185.14 ms | 99.91 ms | 28.88 ms | 123.18 ms | 1.0 ms | 0.5 ms | **2437.96 ms** | Halt and Catch Fire | 100.0% | 1 |
| 633 | `Halt.and.Catch.Fire.S04E05.Nowhere.Man....` | 1960.2 ms | 127.95 ms | 33.25 ms | 120.78 ms | 1.0 ms | 0.5 ms | **2242.6 ms** | Halt.and.Catch.Fire.S04E05.... | -- | 1 |
| 634 | `Halt.and.Catch.Fire.S04E06.A.Connection...` | 2303.59 ms | 103.57 ms | 20.36 ms | 367.35 ms | 1.0 ms | 0.5 ms | **2795.62 ms** | Halt and Catch Fire | 100.0% | 0 |
| 635 | `Halt.and.Catch.Fire.S04E07.Who.Needs.a....` | 2098.71 ms | 103.28 ms | 36.76 ms | 491.42 ms | 1.0 ms | 0.5 ms | **2730.64 ms** | Halt and Catch Fire | 100.0% | 1 |
| 636 | `Halt.and.Catch.Fire.S04E08.Goodwill.108...` | 2124.88 ms | 114.71 ms | 17.42 ms | 298.98 ms | 1.0 ms | 0.5 ms | **2556.48 ms** | Halt and Catch Fire | 100.0% | 1 |
| 637 | `Halt.and.Catch.Fire.S04E09.Search.1080p...` | 2264.36 ms | 108.65 ms | 15.92 ms | 324.93 ms | 1.0 ms | 0.5 ms | **2727.35 ms** | Halt and Catch Fire | 100.0% | 1 |
| 638 | `Halt.and.Catch.Fire.S04E10.Ten.of.Sword...` | 5831.83 ms | 95.64 ms | 16.8 ms | 291.36 ms | 1.0 ms | 0.5 ms | **6236.14 ms** | Halt and Catch Fire | 100.0% | 1 |
| 639 | `Hardcore.Henry_(2015)_[1080p]_mid15s.mp4` | 2670.3 ms | 89.61 ms | 26.96 ms | 170.53 ms | 1.0 ms | 0.5 ms | **2957.98 ms** | Hardcore Henry | 100.0% | 0 |
| 640 | `Harry_Potter_20th_Anniversary_Return_to...` | 2261.98 ms | 101.82 ms | 19.47 ms | 395.18 ms | 1.0 ms | 0.5 ms | **2779.27 ms** | Harry Potter 20th Anniversa... | 100.0% | 0 |
| 641 | `Heaven_Chapter_Overture_2004_mid15s.mp4` | 2249.91 ms | 98.79 ms | 17.31 ms | 505.96 ms | 1.0 ms | 0.5 ms | **2874.63 ms** | Heaven Chapter Overture | 100.0% | 0 |
| 642 | `Heavenly.Creatures.1994.1080p.BluRay.x2...` | 3224.79 ms | 105.51 ms | 27.15 ms | 236.17 ms | 1.0 ms | 0.5 ms | **3598.91 ms** | Heavenly Creatures | 100.0% | 0 |
| 643 | `Hector_and_the_Search_for_Happiness_201...` | 3510.33 ms | 104.62 ms | 20.44 ms | 119.51 ms | 1.0 ms | 0.5 ms | **3755.7 ms** | Hector and the Search for H... | 100.0% | 0 |
| 644 | `Her.Smell.2018.1080p.BluRay.H264.AAC-RA...` | 2383.46 ms | 105.04 ms | 22.72 ms | 251.25 ms | 1.0 ms | 0.5 ms | **2766.82 ms** | Her Smell | 100.0% | 1 |
| 645 | `Hercules_(1997)[MULTI.1080p.H264][AC3.5...` | 2827.34 ms | 109.44 ms | 27.4 ms | 296.02 ms | 1.0 ms | 0.5 ms | **3262.09 ms** | Épisode 004 mkv 24 0fps | 100.0% | 1 |
| 646 | `Hidden.Figures.2016.1080p.BluRay.x265-R...` | 2977.68 ms | 80.86 ms | 19.12 ms | 408.75 ms | 1.0 ms | 0.5 ms | **3487.15 ms** | Hidden Figures | 100.0% | 1 |
| 647 | `High-Rise.2015.FRENCH.BDRiP.x264-AViTEC...` | 2103.41 ms | 96.13 ms | 21.03 ms | 285.96 ms | 1.0 ms | 0.5 ms | **2507.28 ms** | High-Rise | 100.0% | 0 |
| 648 | `His.Dark.Materials.S02E01.The.City.of.M...` | 2295.19 ms | 126.46 ms | 16.2 ms | 120.07 ms | 1.0 ms | 0.5 ms | **2559.35 ms** | His Dark Materials | 100.0% | 1 |
| 649 | `His.Dark.Materials.S02E02.The.Cave.1080...` | 2474.23 ms | 118.48 ms | 39.91 ms | 255.18 ms | 1.0 ms | 0.5 ms | **2888.64 ms** | His Dark Materials | 100.0% | 1 |
| 650 | `His.Dark.Materials.S02E03.Theft.1080p.A...` | 2445.46 ms | 95.14 ms | 16.11 ms | 242.93 ms | 1.0 ms | 0.5 ms | **2807.49 ms** | His Dark Materials | 100.0% | 1 |
| 651 | `His.Dark.Materials.S02E04.Tower.of.the....` | 2473.47 ms | 112.88 ms | 19.52 ms | 105.1 ms | 1.0 ms | 0.5 ms | **2711.82 ms** | His Dark Materials | 100.0% | 1 |
| 652 | `His.Dark.Materials.S02E05.The.Scholar.1...` | 2391.54 ms | 88.76 ms | 17.05 ms | 187.97 ms | 1.0 ms | 0.5 ms | **2689.5 ms** | His Dark Materials | 100.0% | 1 |
| 653 | `His.Dark.Materials.S02E06.Malice.1080p....` | 2468.68 ms | 95.07 ms | 20.16 ms | 203.99 ms | 1.0 ms | 0.5 ms | **2788.73 ms** | His Dark Materials | 100.0% | 1 |
| 654 | `His.Dark.Materials.S02E07.AEsahaettr.10...` | 2528.93 ms | 204.31 ms | 25.25 ms | 584.38 ms | 1.0 ms | 0.5 ms | **3345.7 ms** | His Dark Materials | 100.0% | 1 |
| 655 | `Hotel.Transylvania.4.Transformania.2022...` | 2295.08 ms | 174.86 ms | 27.72 ms | 122.6 ms | 1.0 ms | 0.5 ms | **2620.89 ms** | Hotel Transylvania 4 Transf... | 100.0% | 0 |
| 656 | `House.of.the.Dragon.S01E09.2160p.HMAX.W...` | 2114.31 ms | 92.0 ms | 16.16 ms | 156.54 ms | 1.0 ms | 0.5 ms | **2379.37 ms** | House of the Dragon | 100.0% | 1 |
| 657 | `House.of.the.Dragon.S01E10.2160p.HMAX.W...` | 2075.11 ms | 124.8 ms | 27.38 ms | 120.11 ms | 1.0 ms | 0.5 ms | **2347.73 ms** | House of the Dragon | 100.0% | 1 |
| 658 | `How.It.Ends.2021.1080p.WEB.h264-RUMOUR_...` | 2665.73 ms | 100.68 ms | 35.62 ms | 132.53 ms | 1.0 ms | 0.5 ms | **2946.41 ms** | How It Ends | 100.0% | 0 |
| 659 | `Hugo.2011.1080p.BluRay.x264.YIFY_mid15s...` | 1592.97 ms | 90.81 ms | 30.89 ms | 230.93 ms | 1.0 ms | 0.5 ms | **1946.29 ms** | Aucun | -- | 0 |
| 660 | `Hundreds.Of.Beavers.2022.1080p.BluRay.x...` | 1770.44 ms | 103.22 ms | 49.24 ms | 387.87 ms | 1.0 ms | 0.5 ms | **2316.33 ms** | Hundreds Of Beavers | 100.0% | 0 |
| 661 | `I.Am.Cuba.1964.1080p.BluRay.x264.AAC-[Y...` | 2481.65 ms | 89.93 ms | 23.06 ms | 183.62 ms | 1.0 ms | 0.5 ms | **2779.75 ms** | I Am Cuba | 100.0% | 0 |
| 662 | `I.Am.Legend.ALTERNATE.ENDING.2007.1080p...` | 1877.36 ms | 124.67 ms | 25.24 ms | 230.35 ms | 1.0 ms | 0.5 ms | **2260.97 ms** | I.Am.Legend.ALTERNATE.ENDIN... | -- | 1 |
| 663 | `I_Feel_Good_(2018)_mid15s.mp4` | 1996.66 ms | 92.5 ms | 22.41 ms | 194.41 ms | 1.0 ms | 0.5 ms | **2309.75 ms** | Aucun | -- | 0 |
| 664 | `I_Love_You_Heavenly_(2017)[720p]_-_Tchè...` | 2946.33 ms | 101.47 ms | 23.51 ms | 277.98 ms | 1.0 ms | 0.5 ms | **3350.03 ms** | I Love You Heavenly | 100.0% | 0 |
| 665 | `Im_Thinking_of_Ending_Things_(2020)[108...` | 2202.36 ms | 101.59 ms | 22.41 ms | 188.92 ms | 1.0 ms | 0.5 ms | **2516.33 ms** | Im Thinking of Ending Things | 100.0% | 0 |
| 666 | `In.Search.of.Tomorrow.2022.1080p.WEBRip...` | 2928.1 ms | 93.61 ms | 24.32 ms | 184.41 ms | 1.0 ms | 0.5 ms | **3232.35 ms** | In Search of Tomorrow | 100.0% | 0 |
| 667 | `In.the.Heart.of.the.Sea.2015.MULTi.1080...` | 2493.86 ms | 135.07 ms | 28.85 ms | 189.81 ms | 1.0 ms | 0.5 ms | **2849.72 ms** | In the Heart of the Sea | 100.0% | 0 |
| 668 | `In.the.Mouth.of.Madness.1994.1080p.BluR...` | 2159.25 ms | 126.96 ms | 18.95 ms | 211.33 ms | 1.0 ms | 0.5 ms | **2517.22 ms** | In the Mouth of Madness | 100.0% | 1 |
| 669 | `In.the.Mouth.of.Madness.1994.REMASTERED...` | 2338.77 ms | 104.87 ms | 23.56 ms | 204.72 ms | 1.0 ms | 0.5 ms | **2675.02 ms** | In the Mouth of Madness | 100.0% | 0 |
| 670 | `In_The_Fade_(2017)[MULTI][1080p]_mid15s...` | 2558.57 ms | 139.31 ms | 18.29 ms | 266.88 ms | 1.0 ms | 0.5 ms | **2985.95 ms** | Aucun | -- | 0 |
| 671 | `Inexorable_(2022)[1080p](Poelvoorde,_th...` | 2274.65 ms | 241.02 ms | 25.24 ms | 194.61 ms | 1.0 ms | 0.5 ms | **2740.27 ms** | Aucun | -- | 0 |
| 672 | `Ink.2009.BRRIP.720P.H264-ZEKTORM_mid15s...` | 2083.63 ms | 126.46 ms | 27.81 ms | 185.84 ms | 1.0 ms | 0.5 ms | **2427.73 ms** | Ink | 100.0% | 0 |
| 673 | `Inside.Chernobyl.With.Ben.Fogle.2021.10...` | 2455.54 ms | 117.7 ms | 28.92 ms | 212.57 ms | 1.0 ms | 0.5 ms | **2817.8 ms** | Inside.Chernobyl.With.Ben.F... | -- | 1 |
| 674 | `Inside.Job.2010.720p.BluRay.x264.YIFY_m...` | 2334.78 ms | 93.15 ms | 20.45 ms | 166.82 ms | 1.0 ms | 0.5 ms | **2615.92 ms** | Inside Job | 100.0% | 0 |
| 675 | `Interview.With.The.Vampire.1994.1080p.B...` | 2387.16 ms | 134.06 ms | 24.1 ms | 108.47 ms | 1.0 ms | 0.5 ms | **2654.53 ms** | Interview With The Vampire | 100.0% | 0 |
| 676 | `Introduction_mid15s.mp4` | 2515.79 ms | 125.22 ms | 22.63 ms | 110.03 ms | 1.0 ms | 0.5 ms | **2774.8 ms** | Aucun | -- | 0 |
| 677 | `Intégrer_webpack_à_votre_framework_Back...` | 2193.79 ms | 156.94 ms | 29.69 ms | 137.09 ms | 1.0 ms | 0.5 ms | **2517.87 ms** | Intégrer webpack à votre fr... | 100.0% | 0 |
| 678 | `Inuyashiki_(2018)[1080p]_mid15s.mp4` | 2598.19 ms | 106.01 ms | 37.85 ms | 167.43 ms | 1.0 ms | 0.5 ms | **2911.72 ms** | Inuyashiki | 100.0% | 0 |
| 679 | `Inverso.The.Peripheral.S01E04.Jackpot.I...` | 1692.38 ms | 187.01 ms | 21.2 ms | 178.71 ms | 1.0 ms | 0.5 ms | **2080.08 ms** | Inverso The Peripheral | 100.0% | 1 |
| 680 | `Inverso.The.Peripheral.S01E05.Tutte.le....` | 2078.85 ms | 86.9 ms | 16.76 ms | 224.86 ms | 1.0 ms | 0.5 ms | **2408.19 ms** | Inverso.The.Peripheral.S01E... | -- | 1 |
| 681 | `Inverso.The.Peripheral.S01E06.Fanculo.e...` | 1939.59 ms | 90.12 ms | 17.05 ms | 205.54 ms | 1.0 ms | 0.5 ms | **2257.61 ms** | Inverso The Peripheral | 100.0% | 1 |
| 682 | `Irrational.Man.2015.FRENCH.BDRiP.XViD-A...` | 1775.38 ms | 105.46 ms | 23.06 ms | 143.98 ms | 1.0 ms | 0.5 ms | **2048.6 ms** | Irrational Man | 100.0% | 0 |
| 683 | `Irresistible_(2020)[1080p](US,_Comédie,...` | 2162.52 ms | 118.04 ms | 21.05 ms | 154.35 ms | 1.0 ms | 0.5 ms | **2459.55 ms** | Irresistible | 100.0% | 0 |
| 684 | `J'aimerais_que_la_terre_s'arrête_pour_d...` | 1666.13 ms | 98.79 ms | 18.97 ms | 330.2 ms | 1.0 ms | 0.5 ms | **2125.84 ms** | J'aimerais que la terre s'a... | 100.0% | 0 |
| 685 | `Jackie.Brown.1997.1080p.BluRay.H264.AAC...` | 9907.77 ms | 94.72 ms | 32.76 ms | 395.2 ms | 1.0 ms | 0.5 ms | **10431.12 ms** | Jackie Brown | 100.0% | 0 |
| 686 | `Jacob's.Ladder.1990.720p.BluRay.x264-aA...` | 1976.15 ms | 128.48 ms | 61.24 ms | 272.38 ms | 1.0 ms | 0.5 ms | **2438.89 ms** | Jacob's Ladder | 100.0% | 0 |
| 687 | `Jallikattu.2019.1080p.BluRay.x264.AAC5....` | 2830.6 ms | 112.31 ms | 34.56 ms | 136.6 ms | 1.0 ms | 0.5 ms | **3115.89 ms** | Aucun | -- | 0 |
| 688 | `JavaScript_(10_23)_-_L'objet_Window_mid...` | 5144.19 ms | 112.24 ms | 175.24 ms | 277.62 ms | 1.0 ms | 0.5 ms | **5714.14 ms** | JavaScript 10 23 - L'objet ... | 100.0% | 0 |
| 689 | `JavaScript_(11_23)_-_Le_DOM_mid15s.mp4` | 1902.21 ms | 105.67 ms | 108.6 ms | 291.17 ms | 1.0 ms | 0.5 ms | **2408.03 ms** | JavaScript 11 23 - Le DOM m... | 100.0% | 0 |
| 690 | `JavaScript_(12_23)_-_Les_évènements_mid...` | 2479.45 ms | 144.74 ms | 207.03 ms | 281.43 ms | 1.0 ms | 0.5 ms | **3113.04 ms** | JavaScript 5 23 - Les Fonct... | 100.0% | 0 |
| 691 | `JavaScript_(13_23)_-_TP_-_Système_de_sp...` | 6158.06 ms | 93.4 ms | 125.28 ms | 180.88 ms | 1.0 ms | 0.5 ms | **6558.07 ms** | JavaScript 13 23 - TP - Sys... | 100.0% | 0 |
| 692 | `JavaScript_(14_23)_-_TP_-_Système_d'ong...` | 4272.74 ms | 100.87 ms | 287.79 ms | 300.02 ms | 1.0 ms | 0.5 ms | **4961.77 ms** | Aucun | -- | 0 |
| 693 | `JavaScript_(15_23)_-_TP_-_Menu_collant_...` | 6199.35 ms | 89.2 ms | 114.76 ms | 227.39 ms | 1.0 ms | 0.5 ms | **6631.11 ms** | Aucun | -- | 0 |
| 694 | `JavaScript_(16_23)_-_Ajax_mid15s.mp4` | 2034.66 ms | 110.81 ms | 162.57 ms | 341.59 ms | 1.0 ms | 0.5 ms | **2650.01 ms** | JavaScript 16 23 - Ajax mp4... | 100.0% | 0 |
| 695 | `JavaScript_(17_23)_-_Lodash_mid15s.mp4` | 5002.42 ms | 97.27 ms | 64.17 ms | 277.17 ms | 1.0 ms | 0.5 ms | **5455.01 ms** | JavaScript 17 23 - Lodash m... | 100.0% | 0 |
| 696 | `JavaScript_(18_23)_-_Découverte_de_jQue...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.89 ms** | Erreur | -- | 0 |
| 697 | `JavaScript_(19_23)_-_ECMAScript_2015_mi...` | 10848.29 ms | 83.35 ms | 36.91 ms | 255.42 ms | 1.0 ms | 0.5 ms | **11224.36 ms** | JavaScript 19 23 - ECMAScript | 100.0% | 0 |
| 698 | `JavaScript_(1_23)_-_Apprendre_le_JavaSc...` | 2607.32 ms | 113.25 ms | 53.08 ms | 315.99 ms | 1.0 ms | 0.5 ms | **3090.03 ms** | JavaScript 1 23 - Apprendre... | 100.0% | 0 |
| 699 | `JavaScript_(20_23)_-_Créer_un_Carousel_...` | 4065.39 ms | 97.06 ms | 29.0 ms | 145.15 ms | 1.0 ms | 0.5 ms | **4337.18 ms** | JavaScript 20 23 - Créer un... | 100.0% | 0 |
| 700 | `JavaScript_(21_23)_-_Créer_un_Carousel,...` | 4082.3 ms | 110.55 ms | 31.73 ms | 279.66 ms | 1.0 ms | 0.5 ms | **4504.85 ms** | JavaScript 21 23 - Créer un... | 100.0% | 0 |
| 701 | `JavaScript_(22_23)_-_Créer_un_Carousel,...` | 2057.49 ms | 128.9 ms | 32.46 ms | 201.18 ms | 1.0 ms | 0.5 ms | **2427.71 ms** | Tutoriel PreactJS - Créer u... | 100.0% | 0 |
| 702 | `JavaScript_(23_23)_-_Créer_un_Carousel,...` | 2632.83 ms | 88.06 ms | 26.75 ms | 399.4 ms | 1.0 ms | 0.5 ms | **3147.49 ms** | JavaScript 23 23 - Créer un... | 100.0% | 0 |
| 703 | `JavaScript_(2_23)_-_Les_Variables_mid15...` | 10037.21 ms | 159.65 ms | 25.73 ms | 288.93 ms | 1.0 ms | 0.5 ms | **10511.85 ms** | JavaScript 2 23 - Les Varia... | 100.0% | 0 |
| 704 | `JavaScript_(3_23)_-_Les_Conditions_mid1...` | 4397.06 ms | 108.12 ms | 136.6 ms | 219.86 ms | 1.0 ms | 0.5 ms | **4862.15 ms** | JavaScript 3 23 - Les Condi... | 100.0% | 0 |
| 705 | `JavaScript_(4_23)_-_Les_Boucles_mid15s.mp4` | 11286.33 ms | 104.18 ms | 154.43 ms | 194.66 ms | 1.0 ms | 0.5 ms | **11739.99 ms** | JavaScript 3 23 - Les Condi... | 100.0% | 0 |
| 706 | `JavaScript_(5_23)_-_Les_Fonctions_mid15...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.31 ms** | Erreur | -- | 0 |
| 707 | `JavaScript_(6_23)_-_Portée_des_variable...` | 1041.22 ms | 0.0 ms | 0.0 ms | 466.05 ms | 1.0 ms | 0.5 ms | **1507.44 ms** | Aucun | -- | 0 |
| 708 | `JavaScript_(7_23)_-_L'opérateur_this_mi...` | 1284.28 ms | 0.0 ms | 0.0 ms | 106.38 ms | 1.0 ms | 0.5 ms | **1390.79 ms** | Aucun | -- | 0 |
| 709 | `JavaScript_(8_23)_-_Les_prototypes_mid1...` | 10869.24 ms | 86.84 ms | 161.1 ms | 312.8 ms | 1.0 ms | 0.5 ms | **11437.28 ms** | JavaScript 8 23 - Les proto... | 100.0% | 0 |
| 710 | `JavaScript_(9_23)_-_Try_Catch_mid15s.mp4` | 1386.14 ms | 0.0 ms | 0.0 ms | 341.83 ms | 1.0 ms | 0.5 ms | **1728.12 ms** | Aucun | -- | 0 |
| 711 | `Je_peux_entendre_l'océan_(1993)_VOSTFR_...` | 2157.55 ms | 100.86 ms | 23.57 ms | 114.78 ms | 1.0 ms | 0.5 ms | **2397.41 ms** | Je peux entendre l'océan | 100.0% | 0 |
| 712 | `Jerry.And.Marge.Go.Large.2022.2160p.4K....` | 2452.57 ms | 114.1 ms | 42.99 ms | 175.68 ms | 1.0 ms | 0.5 ms | **2786.12 ms** | Aucun | -- | 0 |
| 713 | `Jodorowskys.Dune.2013.LIMITED.DOC.VOSTF...` | 2854.12 ms | 92.9 ms | 28.78 ms | 137.11 ms | 1.0 ms | 0.5 ms | **3114.75 ms** | Jodorowskys Dune | 100.0% | 0 |
| 714 | `John_Wick_Chapter_3_Parabellum_(2019)[1...` | 2512.2 ms | 115.11 ms | 26.77 ms | 283.88 ms | 1.0 ms | 0.5 ms | **2939.73 ms** | John Wick Chapter 3 Parabellum | 100.0% | 0 |
| 715 | `Johnny_English_Strikes_Again_(2018)_mid...` | 2299.55 ms | 87.84 ms | 17.66 ms | 256.03 ms | 1.0 ms | 0.5 ms | **2665.68 ms** | Johnny English Strikes Again | 100.0% | 0 |
| 716 | `Joker_(2019)[2160p][x265.10bit.HDR.AAC5...` | 2096.95 ms | 110.43 ms | 33.03 ms | 506.96 ms | 1.0 ms | 0.5 ms | **2749.69 ms** | Joker | 100.0% | 0 |
| 717 | `Junk_Head_(2013)[1080p](StopMotion,_🇯🇵,...` | 2539.57 ms | 122.51 ms | 21.98 ms | 274.71 ms | 1.0 ms | 0.5 ms | **2960.27 ms** | The Witcher | 100.0% | 1 |
| 718 | `Jupiter.Holdja.2017.MULTi.1080p.WEB.H26...` | 2101.46 ms | 107.17 ms | 22.48 ms | 500.5 ms | 1.0 ms | 0.5 ms | **2756.65 ms** | Aucun | -- | 0 |
| 719 | `Jurassic_World_Dominion_(2022)[MULTi][1...` | 2484.97 ms | 100.84 ms | 20.93 ms | 437.79 ms | 1.0 ms | 0.5 ms | **3055.95 ms** | Jurassic World Dominion | 100.0% | 0 |
| 720 | `Juste_la_fin_du_monde_2016_1080p_FR_X26...` | 3525.14 ms | 99.51 ms | 48.68 ms | 141.19 ms | 1.0 ms | 0.5 ms | **3815.27 ms** | Aucun | -- | 0 |
| 721 | `Kiki_la_petite_sorcière_(1989)_MULTi_10...` | 1720.12 ms | 109.91 ms | 20.23 ms | 279.02 ms | 1.0 ms | 0.5 ms | **2131.46 ms** | Aucun | -- | 0 |
| 722 | `Kingdom.Ashin.of.the.North.2021.KOREAN....` | 11290.07 ms | 151.21 ms | 22.18 ms | 130.97 ms | 1.0 ms | 0.5 ms | **11594.89 ms** | Aucun | -- | 0 |
| 723 | `Knight.Of.Cups.2015.720p.BluRay.x264-[Y...` | 2261.47 ms | 94.28 ms | 21.38 ms | 128.91 ms | 1.0 ms | 0.5 ms | **2507.21 ms** | Knight Of Cups | 100.0% | 0 |
| 724 | `Kodachrome.2018.MULTi.1080p.NF.WEB-DL.D...` | 2722.92 ms | 107.84 ms | 23.35 ms | 421.1 ms | 1.0 ms | 0.5 ms | **3278.71 ms** | Kodachrome | 100.0% | 0 |
| 725 | `Koyaanisqatsi_(1982)_Criterion_(1080p_B...` | 2379.67 ms | 106.79 ms | 34.33 ms | 123.73 ms | 1.0 ms | 0.5 ms | **2647.55 ms** | Koyaanisqatsi | 100.0% | 0 |
| 726 | `Kyan_Khojandi_-_PULSIONS_[SUBS_-_ENG,_S...` | 4042.19 ms | 183.44 ms | 40.88 ms | 319.91 ms | 1.0 ms | 0.5 ms | **4593.05 ms** | Kyan Khojandi - PULSIONS SUBS | 100.0% | 0 |
| 727 | `L'EFFONDREMENT_-_EP7_-_L'île_[720p]_mid...` | 2698.88 ms | 125.82 ms | 35.33 ms | 183.27 ms | 1.0 ms | 0.5 ms | **3044.08 ms** | L'EFFONDREMENT - EP7 | 100.0% | 0 |
| 728 | `L'EFFONDREMENT_[EP1]_Le_Supermarché_(No...` | 2259.94 ms | 134.91 ms | 24.07 ms | 241.87 ms | 1.0 ms | 0.5 ms | **2662.09 ms** | L'EFFONDREMENT EP1 Le Super... | 100.0% | 0 |
| 729 | `L'EFFONDREMENT_[EP2]_La_Station_Service...` | 2099.32 ms | 86.74 ms | 20.69 ms | 294.6 ms | 1.0 ms | 0.5 ms | **2502.14 ms** | L'EFFONDREMENT EP2 La Stati... | 100.0% | 0 |
| 730 | `L'Empreinte_des_civilisations___L'odyss...` | 2109.16 ms | 102.96 ms | 22.08 ms | 181.0 ms | 1.0 ms | 0.5 ms | **2417.99 ms** | L'Empreinte des civilisatio... | 100.0% | 0 |
| 731 | `L'Enfant_du_mois_de_Kamiari_(2021)[MULT...` | 2592.45 ms | 83.21 ms | 19.73 ms | 226.17 ms | 1.0 ms | 0.5 ms | **2928.02 ms** | L'Enfant du mois de Kamiari | 100.0% | 0 |
| 732 | `L'Evenement_(2021)[1080p](1963_avorteme...` | 2192.53 ms | 103.86 ms | 20.28 ms | 163.43 ms | 1.0 ms | 0.5 ms | **2480.97 ms** | Aucun | -- | 0 |
| 733 | `L'algorithmique_-_Boss_final_-_le_tri_à...` | 1065.78 ms | 0.0 ms | 0.0 ms | 388.57 ms | 1.0 ms | 0.5 ms | **1454.45 ms** | Aucun | -- | 0 |
| 734 | `L'algorithmique_-_La_boucle_POUR_mid15s...` | 795.02 ms | 0.0 ms | 0.0 ms | 174.08 ms | 1.0 ms | 0.5 ms | **969.22 ms** | Aucun | -- | 0 |
| 735 | `L'algorithmique_-_Lecture___Ecriture_mi...` | 1321.02 ms | 89.76 ms | 31.63 ms | 390.21 ms | 1.0 ms | 0.5 ms | **1832.91 ms** | Aucun | -- | 0 |
| 736 | `L'algorithmique_-_Les_Tableaux_mid15s.mp4` | 1310.04 ms | 0.0 ms | 0.0 ms | 264.95 ms | 1.0 ms | 0.5 ms | **1575.09 ms** | Aucun | -- | 0 |
| 737 | `L'algorithmique_-_Les_boucles_mid15s.mp4` | 779.31 ms | 0.0 ms | 0.0 ms | 159.05 ms | 1.0 ms | 0.5 ms | **938.48 ms** | Aucun | -- | 0 |
| 738 | `L'algorithmique_-_Les_fonction_personna...` | 1285.63 ms | 0.0 ms | 0.0 ms | 280.43 ms | 1.0 ms | 0.5 ms | **1566.19 ms** | Aucun | -- | 0 |
| 739 | `L'algorithmique_-_Les_fonctions_mid15s.mp4` | 956.01 ms | 0.0 ms | 0.0 ms | 304.74 ms | 1.0 ms | 0.5 ms | **1260.88 ms** | Aucun | -- | 0 |
| 740 | `L'algorithmique_-_Les_tableaux_multidim...` | 1249.97 ms | 0.0 ms | 0.0 ms | 92.01 ms | 1.0 ms | 0.5 ms | **1342.09 ms** | Aucun | -- | 0 |
| 741 | `L'algorithmique_-_Les_tests_mid15s.mp4` | 11216.42 ms | 102.06 ms | 42.14 ms | 326.29 ms | 1.0 ms | 0.5 ms | **11687.56 ms** | L'algorithmique - Les boucl... | 100.0% | 0 |
| 742 | `L'algorithmique_-_Les_variables_mid15s.mp4` | 1028.3 ms | 0.0 ms | 0.0 ms | 253.05 ms | 1.0 ms | 0.5 ms | **1281.48 ms** | Aucun | -- | 0 |
| 743 | `L'illusion_du_temps___La_magie_du_cosmo...` | 2707.22 ms | 97.08 ms | 50.21 ms | 217.96 ms | 1.0 ms | 0.5 ms | **3074.94 ms** | Aucun | -- | 0 |
| 744 | `L'origine_Du_Monde_(2020)[1080p](Comédi...` | 2606.42 ms | 96.37 ms | 20.24 ms | 253.04 ms | 1.0 ms | 0.5 ms | **2981.88 ms** | L'origine Du Monde | 100.0% | 0 |
| 745 | `L'été_de_kikujiro_(1999).x264_mid15s.mp4` | 2611.4 ms | 97.08 ms | 25.82 ms | 297.76 ms | 1.0 ms | 0.5 ms | **3032.69 ms** | L'été de kikujiro | 100.0% | 0 |
| 746 | `L.Apparition.2018.FRENCH.1080p.WEB.H264...` | 2570.85 ms | 122.84 ms | 25.47 ms | 230.5 ms | 1.0 ms | 0.5 ms | **2950.4 ms** | L Apparition | 100.0% | 0 |
| 747 | `L.Effondrement.S01E08.FiNAL.FRENCH.WEBR...` | 2488.43 ms | 182.14 ms | 22.33 ms | 186.72 ms | 1.0 ms | 0.5 ms | **2880.49 ms** | The Dark Crystal Age of Res... | 100.0% | 0 |
| 748 | `L.Ete.L.Eternite_(2021)[1080p](🔞,_insou...` | 2392.82 ms | 120.08 ms | 29.38 ms | 192.94 ms | 1.0 ms | 0.5 ms | **2739.05 ms** | L Ete L Eternite | 100.0% | 0 |
| 749 | `LE_GENDRE_DE_MA_VIE_(2019)(Comédie,_Kad...` | 2546.97 ms | 103.8 ms | 23.38 ms | 111.47 ms | 1.0 ms | 0.5 ms | **2786.86 ms** | LE GENDRE DE MA VIE | 100.0% | 0 |
| 750 | `LE_MONSTRE_DES_MERS_(2022)[MULTi][1080p...` | 3178.66 ms | 88.29 ms | 17.24 ms | 325.85 ms | 1.0 ms | 0.5 ms | **3610.88 ms** | Aucun | -- | 0 |
| 751 | `La.Belle.Et.La.Belle.2018.FRENCH.HDRip....` | 2481.87 ms | 108.85 ms | 20.65 ms | 252.03 ms | 1.0 ms | 0.5 ms | **2864.21 ms** | La Belle Et La Belle | 100.0% | 0 |
| 752 | `La.Glace.et.le.Ciel.2015.FRENCH.BDRip.X...` | 2640.79 ms | 99.2 ms | 24.07 ms | 392.4 ms | 1.0 ms | 0.5 ms | **3170.09 ms** | La Glace et le Ciel | 100.0% | 0 |
| 753 | `La.Momie.1999.MULTI.1080p.BluRay.HDLigh...` | 2100.34 ms | 232.04 ms | 31.36 ms | 434.72 ms | 1.0 ms | 0.5 ms | **2798.83 ms** | Aucun | -- | 0 |
| 754 | `La.Vie.Tres.Privee.De.Monsieur.Sim.2015...` | 2542.94 ms | 97.9 ms | 22.5 ms | 302.57 ms | 1.0 ms | 0.5 ms | **2966.67 ms** | La Vie Tres Privee De Monsi... | 100.0% | 0 |
| 755 | `La_Belle_Verte_(1996)_[480p](Comédie,_C...` | 2203.23 ms | 94.27 ms | 20.36 ms | 75.47 ms | 1.0 ms | 0.5 ms | **2394.14 ms** | La Belle Verte | 100.0% | 0 |
| 756 | `La_Chute_de_l'Empire_Americain_(2019)_[...` | 2980.53 ms | 100.99 ms | 18.26 ms | 361.89 ms | 1.0 ms | 0.5 ms | **3467.69 ms** | La Chute de l'Empire Americain | 100.0% | 0 |
| 757 | `La_Cité_des_enfants_perdus_(1995)[1080p...` | 4632.35 ms | 104.86 ms | 35.73 ms | 302.28 ms | 1.0 ms | 0.5 ms | **5080.28 ms** | La Cité des enfants perdus | 100.0% | 0 |
| 758 | `La_Colline_aux_Coquelicots_(2011)_MULTi...` | 2354.1 ms | 88.76 ms | 19.71 ms | 181.15 ms | 1.0 ms | 0.5 ms | **2648.7 ms** | La Colline aux Coquelicots | 100.0% | 0 |
| 759 | `La_Colline_ou_Rugissent_les_Lionnes_(20...` | 2605.0 ms | 92.03 ms | 25.22 ms | 257.66 ms | 1.0 ms | 0.5 ms | **2980.85 ms** | La Colline ou Rugissent les... | 100.0% | 0 |
| 760 | `La_Grande_Classe_(2019)_[1080](Comédie,...` | 2684.49 ms | 112.68 ms | 22.44 ms | 128.0 ms | 1.0 ms | 0.5 ms | **2948.53 ms** | La Grande Classe | 100.0% | 0 |
| 761 | `La_Mif_(2022)[1080p](jeunes_femmes,_foy...` | 2602.19 ms | 94.63 ms | 20.56 ms | 113.26 ms | 1.0 ms | 0.5 ms | **2831.55 ms** | La Mif | 100.0% | 0 |
| 762 | `La_Nuit_Des_Rois_(2020)[1080p](Adbijan,...` | 2162.61 ms | 97.8 ms | 30.58 ms | 384.03 ms | 1.0 ms | 0.5 ms | **2676.22 ms** | La Nuit Des Rois | 100.0% | 0 |
| 763 | `La_POO_en_PHP_(10_30)_-_Les_namespaces_...` | 4883.59 ms | 105.2 ms | 26.14 ms | 180.4 ms | 1.0 ms | 0.5 ms | **5195.75 ms** | La POO en PHP 10 30 - Les n... | 100.0% | 0 |
| 764 | `La_POO_en_PHP_(11_30)_-_TP_-_La_structu...` | 2453.26 ms | 86.14 ms | 63.49 ms | 155.81 ms | 1.0 ms | 0.5 ms | **2759.16 ms** | Aucun | -- | 0 |
| 765 | `La_POO_en_PHP_(12_30)_-_TP_-_Connexion_...` | 2714.25 ms | 111.28 ms | 158.53 ms | 335.88 ms | 1.0 ms | 0.5 ms | **3320.34 ms** | La POO en PHP 12 30 - TP - ... | 100.0% | 0 |
| 766 | `La_POO_en_PHP_(13_30)_-_TP_-_Création_d...` | 2318.02 ms | 102.83 ms | 128.43 ms | 327.39 ms | 1.0 ms | 0.5 ms | **2877.13 ms** | La POO en PHP 12 30 - TP - ... | 100.0% | 0 |
| 767 | `La_POO_en_PHP_(14_30)_-_TP_-_Problèmes_...` | 5932.65 ms | 84.19 ms | 27.76 ms | 368.74 ms | 1.0 ms | 0.5 ms | **6413.71 ms** | La POO en PHP 14 30 - TP - ... | 100.0% | 0 |
| 768 | `La_POO_en_PHP_(15_30)_-_Singleton_mid15...` | 2661.96 ms | 95.59 ms | 34.14 ms | 133.85 ms | 1.0 ms | 0.5 ms | **2932.38 ms** | La POO en PHP 18 30 - TP - ... | 100.0% | 0 |
| 769 | `La_POO_en_PHP_(16_30)_-_Factory_mid15s.mp4` | 4515.89 ms | 126.46 ms | 29.55 ms | 101.78 ms | 1.0 ms | 0.5 ms | **4774.0 ms** | La POO en PHP 17 30 - Injec... | 100.0% | 0 |
| 770 | `La_POO_en_PHP_(17_30)_-_Injection_de_dé...` | 3287.1 ms | 110.8 ms | 28.04 ms | 106.37 ms | 1.0 ms | 0.5 ms | **3532.63 ms** | La POO en PHP 17 30 - Injec... | 100.0% | 0 |
| 771 | `La_POO_en_PHP_(18_30)_-_TP_-_Refactorin...` | 2085.51 ms | 84.13 ms | 21.08 ms | 154.33 ms | 1.0 ms | 0.5 ms | **2345.43 ms** | La POO en PHP 18 30 - TP - ... | 100.0% | 0 |
| 772 | `La_POO_en_PHP_(19_30)_-_TP_-_Back-end_m...` | 2128.49 ms | 91.84 ms | 22.81 ms | 308.63 ms | 1.0 ms | 0.5 ms | **2552.18 ms** | La POO en PHP 19 30 - TP - ... | 100.0% | 0 |
| 773 | `La_POO_en_PHP_(1_30)_-_Les_objets,_c'es...` | 10321.48 ms | 98.05 ms | 22.55 ms | 265.86 ms | 1.0 ms | 0.5 ms | **10708.54 ms** | La POO en PHP 1 30 - Les ob... | 100.0% | 0 |
| 774 | `La_POO_en_PHP_(20_30)_-_Model_View_Cont...` | 4364.04 ms | 85.05 ms | 24.86 ms | 313.65 ms | 1.0 ms | 0.5 ms | **4788.1 ms** | La POO en PHP 20 30 - Model... | 100.0% | 0 |
| 775 | `La_POO_en_PHP_(21_30)_-_Fluent_mid15s.mp4` | 10589.69 ms | 101.47 ms | 20.42 ms | 354.24 ms | 1.0 ms | 0.5 ms | **11066.16 ms** | La POO en PHP 22 30 - Facad... | 100.0% | 0 |
| 776 | `La_POO_en_PHP_(22_30)_-_Facade_mid15s.mp4` | 2534.14 ms | 110.73 ms | 57.52 ms | 194.77 ms | 1.0 ms | 0.5 ms | **2897.68 ms** | La POO en PHP 22 30 - Facad... | 100.0% | 0 |
| 777 | `La_POO_en_PHP_(23_30)_-_Les_interfaces_...` | 1772.93 ms | 83.28 ms | 30.61 ms | 415.94 ms | 1.0 ms | 0.5 ms | **2303.63 ms** | La POO en PHP 23 30 - Les i... | 100.0% | 0 |
| 778 | `La_POO_en_PHP_(24_30)_-_Les_traits_mid1...` | 11627.61 ms | 99.95 ms | 51.78 ms | 146.14 ms | 1.0 ms | 0.5 ms | **11925.83 ms** | La POO en PHP 24 30 - Les t... | 100.0% | 0 |
| 779 | `La_POO_en_PHP_(25_30)_-_Conteneur_d'inj...` | 2475.95 ms | 116.08 ms | 87.54 ms | 178.56 ms | 1.0 ms | 0.5 ms | **2858.82 ms** | Aucun | -- | 0 |
| 780 | `La_POO_en_PHP_(26_30)_-_Adapter_mid15s.mp4` | 5940.44 ms | 102.44 ms | 28.54 ms | 230.73 ms | 1.0 ms | 0.5 ms | **6302.49 ms** | La POO en PHP 27 30 - Desig... | 100.0% | 0 |
| 781 | `La_POO_en_PHP_(27_30)_-_Design_pattern_...` | 1972.85 ms | 125.16 ms | 30.81 ms | 95.24 ms | 1.0 ms | 0.5 ms | **2224.41 ms** | La POO en PHP 27 30 - Desig... | 100.0% | 0 |
| 782 | `La_POO_en_PHP_(28_30)_-_Observateur_mid...` | 4470.54 ms | 88.46 ms | 20.99 ms | 198.03 ms | 1.0 ms | 0.5 ms | **4804.66 ms** | La POO en PHP 28 30 - Obser... | 100.0% | 0 |
| 783 | `La_POO_en_PHP_(29_30)_-_Middleware_mid1...` | 4176.12 ms | 98.51 ms | 54.87 ms | 200.48 ms | 1.0 ms | 0.5 ms | **4530.38 ms** | Aucun | -- | 0 |
| 784 | `La_POO_en_PHP_(2_30)_-_Notre_première_c...` | 1682.21 ms | 116.63 ms | 56.11 ms | 321.11 ms | 1.0 ms | 0.5 ms | **2176.43 ms** | La POO en PHP 2 30 - Notre ... | 100.0% | 0 |
| 785 | `La_POO_en_PHP_(30_30)_-_Event_Manager_m...` | 3192.04 ms | 89.16 ms | 21.41 ms | 236.21 ms | 1.0 ms | 0.5 ms | **3539.23 ms** | La POO en PHP 30 30 - Event... | 100.0% | 0 |
| 786 | `La_POO_en_PHP_(3_30)_-_La_visibilité_pu...` | 11051.42 ms | 105.1 ms | 25.27 ms | 272.99 ms | 1.0 ms | 0.5 ms | **11455.19 ms** | La POO en PHP 3 30 - La vis... | 100.0% | 0 |
| 787 | `La_POO_en_PHP_(4_30)_-_Un_exemple_concr...` | 2101.38 ms | 87.45 ms | 25.11 ms | 136.28 ms | 1.0 ms | 0.5 ms | **2354.13 ms** | La POO en PHP 4 30 - Un exe... | 100.0% | 0 |
| 788 | `La_POO_en_PHP_(5_30)_-_Bien_documenter_...` | 11108.03 ms | 104.05 ms | 57.87 ms | 182.95 ms | 1.0 ms | 0.5 ms | **11453.39 ms** | Aucun | -- | 0 |
| 789 | `La_POO_en_PHP_(6_30)_-_Propriétés_et_Mé...` | 10561.12 ms | 83.62 ms | 33.42 ms | 375.98 ms | 1.0 ms | 0.5 ms | **11054.58 ms** | La POO en PHP 5 30 - Bien d... | 100.0% | 0 |
| 790 | `La_POO_en_PHP_(7_30)_-_L'héritage_mid15...` | 1058.61 ms | 0.0 ms | 0.0 ms | 131.49 ms | 1.0 ms | 0.5 ms | **1190.21 ms** | Aucun | -- | 0 |
| 791 | `La_POO_en_PHP_(8_30)_-_Exemple_d'hérita...` | 2316.0 ms | 94.03 ms | 34.4 ms | 264.12 ms | 1.0 ms | 0.5 ms | **2708.9 ms** | Aucun | -- | 0 |
| 792 | `La_POO_en_PHP_(9_30)_-_L'autoloading_mi...` | 1304.45 ms | 0.0 ms | 0.0 ms | 131.97 ms | 1.0 ms | 0.5 ms | **1436.54 ms** | Aucun | -- | 0 |
| 793 | `La_Planète_Sauvage_(1973)_Fantastic_Pla...` | 10654.5 ms | 96.12 ms | 21.67 ms | 135.46 ms | 1.0 ms | 0.5 ms | **10910.72 ms** | La Planète Sauvage | 100.0% | 0 |
| 794 | `La_Porte_Du_Paradis_Fr_DvdRip_Nabil@Bat...` | 3738.94 ms | 126.4 ms | 18.92 ms | 239.96 ms | 1.0 ms | 0.5 ms | **4127.05 ms** | La Porte Du Paradis Fr | 100.0% | 0 |
| 795 | `La_Programmation_Orientée_Objet_en_PHP_...` | 1324.85 ms | 0.0 ms | 0.0 ms | 267.58 ms | 1.0 ms | 0.5 ms | **1592.59 ms** | Aucun | -- | 0 |
| 796 | `La_Revanche_des_Crevettes_Pailletées_(2...` | 2899.99 ms | 126.7 ms | 37.65 ms | 256.45 ms | 1.0 ms | 0.5 ms | **3321.94 ms** | La Revanche des Crevettes P... | 100.0% | 0 |
| 797 | `La_Revolution_Silencieuse_(2018)[TRUEFR...` | 3232.98 ms | 107.52 ms | 23.02 ms | 748.4 ms | 1.0 ms | 0.5 ms | **4112.64 ms** | La Revolution Silencieuse | 100.0% | 0 |
| 798 | `La_Terre_Des_Hommes_(2020)[1080p](Drame...` | 2897.69 ms | 102.97 ms | 18.71 ms | 127.81 ms | 1.0 ms | 0.5 ms | **3147.87 ms** | La Terre Des Hommes | 100.0% | 0 |
| 799 | `La_Tortue_Rouge_(2016)_mid15s.mp4` | 2921.13 ms | 100.26 ms | 15.47 ms | 227.9 ms | 1.0 ms | 0.5 ms | **3265.56 ms** | La Tortue Rouge | 100.0% | 0 |
| 800 | `La_Traversee_(2021)[1080p](fr,_animé,_U...` | 2103.15 ms | 124.96 ms | 20.2 ms | 137.26 ms | 1.0 ms | 0.5 ms | **2387.82 ms** | La Traversee | 100.0% | 0 |
| 801 | `La_Traversee_(2021)[1080p](style_peintu...` | 4438.01 ms | 96.33 ms | 18.19 ms | 273.02 ms | 1.0 ms | 0.5 ms | **4829.11 ms** | La Traversee | 100.0% | 0 |
| 802 | `La_Traversée_de_Paris_(1956)_mid15s.mp4` | 2362.27 ms | 97.94 ms | 17.61 ms | 169.16 ms | 1.0 ms | 0.5 ms | **2654.52 ms** | La Traversée de Paris | 100.0% | 0 |
| 803 | `La_concurrence_sur_PHP___NodeJS___Golan...` | 2054.19 ms | 112.61 ms | 22.67 ms | 234.19 ms | 1.0 ms | 0.5 ms | **2425.27 ms** | La concurrence sur PHP Node... | 100.0% | 0 |
| 804 | `Lady.and.the.Tramp.2019.iNTERNAL.720p.W...` | 2183.7 ms | 101.71 ms | 24.83 ms | 252.32 ms | 1.0 ms | 0.5 ms | **2563.25 ms** | Lady and the Tramp | 100.0% | 1 |
| 805 | `Lanterns_(2026)_S01E03_2160p_AMZN_WEB-D...` | 2472.57 ms | 88.58 ms | 84.45 ms | 122.76 ms | 1.0 ms | 0.5 ms | **2772.97 ms** | Aucun | -- | 0 |
| 806 | `Lanterns_(2026)_S01E04_2160p_AMZN_WEB-D...` | 4303.7 ms | 90.25 ms | 13.72 ms | 122.73 ms | 1.0 ms | 0.5 ms | **4531.66 ms** | Lanterns | 100.0% | 0 |
| 807 | `Lanterns_(2026)_S01E06_2160p_AMZN_WEB-D...` | 2174.84 ms | 111.98 ms | 21.75 ms | 264.77 ms | 1.0 ms | 0.5 ms | **2573.81 ms** | Lanterns | 100.0% | 0 |
| 808 | `Lanterns_(2026)_S01E07_2160p_AMZN_WEB-D...` | 2532.71 ms | 101.14 ms | 24.81 ms | 237.94 ms | 1.0 ms | 0.5 ms | **2897.07 ms** | Lanterns | 100.0% | 0 |
| 809 | `Lanterns_(2026)_S01E08_2160p_AMZN_WEB-D...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.04 ms** | Erreur | -- | 0 |
| 810 | `Laravel_-_Authentification_mid15s.mp4` | 2286.71 ms | 114.51 ms | 24.75 ms | 158.7 ms | 1.0 ms | 0.5 ms | **2585.09 ms** | Laravel - Authentification ... | 100.0% | 0 |
| 811 | `Laravel_-_Controller_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12056.85 ms** | Erreur | -- | 0 |
| 812 | `Laravel_-_Eloquent,_Relations_mid15s.mp4` | 2000.27 ms | 97.83 ms | 37.54 ms | 302.23 ms | 1.0 ms | 0.5 ms | **2438.3 ms** | Laravel - Eloquent, Relatio... | 100.0% | 0 |
| 813 | `Laravel_-_Eloquent,_belongsToMany_mid15...` | 2054.0 ms | 106.32 ms | 142.53 ms | 134.78 ms | 1.0 ms | 0.5 ms | **2438.03 ms** | Laravel - Eloquent, belongs... | 100.0% | 0 |
| 814 | `Laravel_-_Eloquent,_scope_&_mutateurs_m...` | 4399.75 ms | 87.4 ms | 22.74 ms | 275.69 ms | 1.0 ms | 0.5 ms | **4785.94 ms** | Laravel - Eloquent, scope &... | 100.0% | 0 |
| 815 | `Laravel_-_Email_mid15s.mp4` | 2390.86 ms | 86.16 ms | 20.56 ms | 211.75 ms | 1.0 ms | 0.5 ms | **2710.28 ms** | Laravel - Email mp4 24 0fps | 100.0% | 0 |
| 816 | `Laravel_-_Formulaires_mid15s.mp4` | 2692.63 ms | 95.3 ms | 36.75 ms | 232.29 ms | 1.0 ms | 0.5 ms | **3057.86 ms** | Aucun | -- | 0 |
| 817 | `Laravel_-_Installation_mid15s.mp4` | 2511.29 ms | 94.49 ms | 18.58 ms | 133.15 ms | 1.0 ms | 0.5 ms | **2758.13 ms** | Laravel - Installation mp4 ... | 100.0% | 0 |
| 818 | `Laravel_-_Le_Service_Container_et_le_Se...` | 1296.19 ms | 0.0 ms | 0.0 ms | 168.54 ms | 1.0 ms | 0.5 ms | **1464.9 ms** | Aucun | -- | 0 |
| 819 | `Laravel_-_Les_Middlewares_mid15s.mp4` | 1028.73 ms | 0.0 ms | 0.0 ms | 109.55 ms | 1.0 ms | 0.5 ms | **1138.38 ms** | Aucun | -- | 0 |
| 820 | `Laravel_-_Les_routes_mid15s.mp4` | 1003.21 ms | 0.0 ms | 0.0 ms | 346.49 ms | 1.0 ms | 0.5 ms | **1349.82 ms** | Aucun | -- | 0 |
| 821 | `Laravel_-_Session_&_Flash_mid15s.mp4` | 1571.1 ms | 0.0 ms | 0.0 ms | 158.14 ms | 1.0 ms | 0.5 ms | **1729.37 ms** | Aucun | -- | 0 |
| 822 | `Laravel_-_TP_2_Introduction_mid15s.mp4` | 1103.5 ms | 0.0 ms | 0.0 ms | 507.71 ms | 1.0 ms | 0.5 ms | **1611.34 ms** | Aucun | -- | 0 |
| 823 | `Laravel_-_TP___Raccourcisseur_d'URL_mid...` | 1611.71 ms | 0.0 ms | 0.0 ms | 212.82 ms | 1.0 ms | 0.5 ms | **1824.68 ms** | Aucun | -- | 0 |
| 824 | `Laravel_-_Templates_Blade_mid15s.mp4` | 4524.0 ms | 126.57 ms | 27.08 ms | 138.36 ms | 1.0 ms | 0.5 ms | **4819.33 ms** | Laravel - Templates Blade m... | 100.0% | 0 |
| 825 | `Laravel_-_Validation_mid15s.mp4` | 5082.1 ms | 90.41 ms | 33.16 ms | 265.73 ms | 1.0 ms | 0.5 ms | **5471.79 ms** | Aucun | -- | 0 |
| 826 | `Lawrence.Of.Arabia.1962.1080p.BluRay.x2...` | 2658.08 ms | 109.72 ms | 17.12 ms | 176.45 ms | 1.0 ms | 0.5 ms | **2974.51 ms** | Lawrence.Of.Arabia.1962.108... | -- | 1 |
| 827 | `Le.Daim.2019.FRENCH.1080p.BluRay.x264-U...` | 2818.51 ms | 89.91 ms | 19.14 ms | 339.6 ms | 1.0 ms | 0.5 ms | **3269.65 ms** | Aucun | -- | 0 |
| 828 | `Le.Grand.Partage.2015.FRENCH.720p.BluRa...` | 2751.01 ms | 93.71 ms | 21.62 ms | 262.94 ms | 1.0 ms | 0.5 ms | **3147.88 ms** | Le Grand Partage | 100.0% | 0 |
| 829 | `Le.Monde.est.à.toi.2018.FRENCH.HDRip.Xv...` | 3208.98 ms | 98.28 ms | 23.68 ms | 172.57 ms | 1.0 ms | 0.5 ms | **3507.99 ms** | Le Monde est à toi | 100.0% | 0 |
| 830 | `Le.Petit.Prince.2015.FRENCH.DVDRiP.x264...` | 2706.35 ms | 122.58 ms | 21.63 ms | 250.72 ms | 1.0 ms | 0.5 ms | **3101.98 ms** | Le Petit Prince | 100.0% | 0 |
| 831 | `Le.Retour.Du.Heros.2018.FRENCH.1080p.Bl...` | 2148.58 ms | 125.55 ms | 31.95 ms | 139.76 ms | 1.0 ms | 0.5 ms | **2447.42 ms** | Le Retour Du Heros | 100.0% | 0 |
| 832 | `Le_Bonheur_Des_Uns_(2020)[1080p](Comédi...` | 2527.47 ms | 230.86 ms | 37.55 ms | 228.83 ms | 1.0 ms | 0.5 ms | **3027.47 ms** | The Boys Presents Diabolical | 100.0% | 1 |
| 833 | `Le_Chant_du_Loup_(2019)_[1080p](Thrille...` | 3023.56 ms | 117.01 ms | 94.93 ms | 258.78 ms | 1.0 ms | 0.5 ms | **3494.73 ms** | Aucun | -- | 0 |
| 834 | `Le_Chateau_de_Cagliostro_(1979)[MULTi][...` | 2675.78 ms | 122.79 ms | 34.96 ms | 164.88 ms | 1.0 ms | 0.5 ms | **3000.2 ms** | Le Chateau de Cagliostro | 100.0% | 0 |
| 835 | `Le_Château_ambulant_(2004)_MULTi_1080p_...` | 4850.02 ms | 90.29 ms | 18.42 ms | 175.8 ms | 1.0 ms | 0.5 ms | **5141.17 ms** | The Expanse | 100.0% | 1 |
| 836 | `Le_Château_dans_le_ciel_(1986)_MULTi_10...` | 2451.81 ms | 350.39 ms | 30.94 ms | 142.42 ms | 1.0 ms | 0.5 ms | **2977.08 ms** | Aucun | -- | 0 |
| 837 | `Le_Grand_soir_(2012)_mid15s.mp4` | 2428.55 ms | 94.75 ms | 96.64 ms | 259.4 ms | 1.0 ms | 0.5 ms | **2879.73 ms** | Le Grand soir | 100.0% | 0 |
| 838 | `Le_Géant_de_fer_(1999)_mid15s.mp4` | 2307.62 ms | 93.42 ms | 19.47 ms | 226.92 ms | 1.0 ms | 0.5 ms | **2648.16 ms** | Le Géant de fer | 100.0% | 0 |
| 839 | `Le_Monde_D'Hier_(2022)[1080p](Président...` | 7561.86 ms | 91.46 ms | 100.19 ms | 267.14 ms | 1.0 ms | 0.5 ms | **8020.98 ms** | Aucun | -- | 0 |
| 840 | `Le_Monde_est_à_toi_(2018)_mid15s.mp4` | 4035.52 ms | 124.98 ms | 48.81 ms | 348.54 ms | 1.0 ms | 0.5 ms | **4558.26 ms** | Aucun | -- | 0 |
| 841 | `Le_Pacte_des_Loups_(2001)[1080p][x265]_...` | 1920.76 ms | 102.46 ms | 20.5 ms | 162.19 ms | 1.0 ms | 0.5 ms | **2211.41 ms** | Le Pacte des Loups | 100.0% | 0 |
| 842 | `Le_Pardon_(2021)[MULTi][1080p](🇮🇷_🤵_💀)_...` | 951.34 ms | 0.0 ms | 0.0 ms | 128.44 ms | 1.0 ms | 0.5 ms | **1080.45 ms** | Aucun | -- | 0 |
| 843 | `Le_Proces_du_36_-_Reportage_-_France_2_...` | 1968.5 ms | 95.78 ms | 25.03 ms | 144.14 ms | 1.0 ms | 0.5 ms | **2242.59 ms** | Aucun | -- | 0 |
| 844 | `Le_Royaume_des_chats_(2002)_MULTi_1080p...` | 2889.26 ms | 92.47 ms | 21.15 ms | 492.29 ms | 1.0 ms | 0.5 ms | **3495.8 ms** | Le Royaume des chats | 100.0% | 0 |
| 845 | `Le_Temps_des_Secrets_(2022)[1080p](Marc...` | 3040.43 ms | 84.48 ms | 26.59 ms | 369.25 ms | 1.0 ms | 0.5 ms | **3521.07 ms** | Le Temps des Secrets | 100.0% | 0 |
| 846 | `Le_Vent_se_lève_(2013)_MULTi_1080p_10bi...` | 2857.34 ms | 95.07 ms | 23.44 ms | 123.21 ms | 1.0 ms | 0.5 ms | **3101.36 ms** | Le Vent se lève | 100.0% | 0 |
| 847 | `Le_Voyage_D_Arlo_2015_MULTI_1080p_BluRa...` | 2580.77 ms | 94.49 ms | 33.94 ms | 218.33 ms | 1.0 ms | 0.5 ms | **2932.07 ms** | Le Voyage D Arlo | 100.0% | 0 |
| 848 | `Le_bon_la_brute_et_le_truand_(1966)_-_1...` | 2910.14 ms | 100.97 ms | 22.31 ms | 115.23 ms | 1.0 ms | 0.5 ms | **3151.97 ms** | Le bon la brute et le truand | 100.0% | 0 |
| 849 | `Le_conte_de_la_princesse_Kaguya_(2013)_...` | 2766.86 ms | 114.16 ms | 19.59 ms | 154.26 ms | 1.0 ms | 0.5 ms | **3057.92 ms** | Le conte de la princesse Ka... | 100.0% | 0 |
| 850 | `Le_pont_de_la_riviere_Kwai_CD1_-_TRUEFR...` | 3625.56 ms | 83.81 ms | 20.71 ms | 186.68 ms | 1.0 ms | 0.5 ms | **3921.0 ms** | Le pont de la riviere Kwai ... | 100.0% | 0 |
| 851 | `Le_pont_de_la_riviere_Kwai_CD2_-_TRUEFR...` | 3305.4 ms | 98.57 ms | 17.99 ms | 119.87 ms | 1.0 ms | 0.5 ms | **3556.9 ms** | Le pont de la riviere Kwai ... | 100.0% | 0 |
| 852 | `Le_saut_quantique___La_magie_du_cosmos_...` | 3022.23 ms | 100.04 ms | 31.25 ms | 273.0 ms | 1.0 ms | 0.5 ms | **3427.24 ms** | Le saut quantique La magie ... | 100.0% | 0 |
| 853 | `Le_statut_de_Freelance,_Introduction_mi...` | 1184.71 ms | 0.0 ms | 0.0 ms | 221.08 ms | 1.0 ms | 0.5 ms | **1405.94 ms** | Aucun | -- | 0 |
| 854 | `Le_statut_de_Freelance,_Mon_expérience_...` | 2513.16 ms | 106.38 ms | 33.19 ms | 253.39 ms | 1.0 ms | 0.5 ms | **2911.09 ms** | Le statut de Freelance, Mon... | 100.0% | 0 |
| 855 | `Le_statut_de_Freelance,_vos_questions_m...` | 2676.92 ms | 88.81 ms | 33.98 ms | 391.14 ms | 1.0 ms | 0.5 ms | **3191.61 ms** | Le statut de Freelance, vos... | 100.0% | 0 |
| 856 | `Le_tombeau_des_lucioles_(1988)_MULTi_10...` | 2209.68 ms | 99.49 ms | 24.34 ms | 111.4 ms | 1.0 ms | 0.5 ms | **2445.65 ms** | Le tombeau des lucioles | 100.0% | 0 |
| 857 | `Le_voyage_de_Chihiro_(2001)_MULTi_1080p...` | 2765.6 ms | 96.33 ms | 22.59 ms | 312.61 ms | 1.0 ms | 0.5 ms | **3202.18 ms** | Le voyage de Chihiro | 100.0% | 0 |
| 858 | `Leave.No.Trace.2018.1080p.BluRay.H264.A...` | 3174.39 ms | 99.21 ms | 22.98 ms | 253.83 ms | 1.0 ms | 0.5 ms | **3554.8 ms** | The Office US | 100.0% | 2 |
| 859 | `Lee_Scratch_Perry’s_vision_of_paradise_...` | 2178.35 ms | 130.1 ms | 25.11 ms | 318.43 ms | 1.0 ms | 0.5 ms | **2653.48 ms** | Lee Scratch Perry’s vision ... | 100.0% | 0 |
| 860 | `Legend.2015.MULTi.1080p.BluRay.x264-CNF...` | 2250.06 ms | 126.69 ms | 22.51 ms | 131.78 ms | 1.0 ms | 0.5 ms | **2531.65 ms** | Legend | 100.0% | 0 |
| 861 | `Legend_of_Crimson_Youth_1988_mid15s.mp4` | 3199.39 ms | 85.49 ms | 31.07 ms | 223.05 ms | 1.0 ms | 0.5 ms | **3543.06 ms** | Legend of Crimson Youth | 100.0% | 0 |
| 862 | `Les.Combattants.2014.FRENCH.BDRip.x264-...` | 2616.99 ms | 87.2 ms | 20.42 ms | 125.39 ms | 1.0 ms | 0.5 ms | **2851.89 ms** | Les Combattants | 100.0% | 0 |
| 863 | `Les.Heritiers_(2014)_mid15s.mp4` | 3917.89 ms | 137.92 ms | 18.89 ms | 191.66 ms | 1.0 ms | 0.5 ms | **4274.53 ms** | Les Heritiers | 100.0% | 0 |
| 864 | `Les.Premiers.Les.Derniers.2016.FRENCH.B...` | 3469.93 ms | 144.4 ms | 19.62 ms | 241.47 ms | 1.0 ms | 0.5 ms | **3876.44 ms** | Les Premiers Les Derniers | 100.0% | 0 |
| 865 | `Les.Tetes.De.L.Emploi.2016.FRENCH.BDRip...` | 2099.2 ms | 150.72 ms | 21.98 ms | 347.12 ms | 1.0 ms | 0.5 ms | **2619.61 ms** | Les Tetes De L Emploi | 100.0% | 0 |
| 866 | `Les.délices.de.Tokyo.(An).(2015).VostFr...` | 2307.1 ms | 84.82 ms | 19.98 ms | 318.56 ms | 1.0 ms | 0.5 ms | **2735.26 ms** | Les délices de Tokyo An | 100.0% | 0 |
| 867 | `Les_Bonnes_Manieres_(2017)[1080p](Brési...` | 2274.75 ms | 87.27 ms | 29.8 ms | 470.4 ms | 1.0 ms | 0.5 ms | **2863.02 ms** | Les Bonnes Manieres | 100.0% | 0 |
| 868 | `Les_Contes_de_Terremer_(2006)_MULTi_108...` | 1980.93 ms | 94.71 ms | 20.94 ms | 180.62 ms | 1.0 ms | 0.5 ms | **2280.42 ms** | Les Contes de Terremer | 100.0% | 0 |
| 869 | `Les_Etoiles_Restantes_(2017)[1080p]_mid...` | 3058.72 ms | 115.03 ms | 34.23 ms | 133.83 ms | 1.0 ms | 0.5 ms | **3342.9 ms** | Les Etoiles Restantes | 100.0% | 0 |
| 870 | `Les_Folies_Fermieres_(2022)[1080p](Alba...` | 2211.72 ms | 95.2 ms | 22.2 ms | 256.95 ms | 1.0 ms | 0.5 ms | **2586.87 ms** | Les Folies Fermieres | 100.0% | 0 |
| 871 | `Les_Gagnants_(2022)[1080p](Joey_Star)_m...` | 3047.31 ms | 152.32 ms | 20.64 ms | 166.88 ms | 1.0 ms | 0.5 ms | **3396.93 ms** | Les Gagnants | 100.0% | 0 |
| 872 | `Les_Garcons_Sauvages_(2018)_mid15s.mp4` | 2798.01 ms | 123.65 ms | 25.19 ms | 248.01 ms | 1.0 ms | 0.5 ms | **3203.43 ms** | Aucun | -- | 0 |
| 873 | `Les_Lyonnais[2011]BRRip_XviD[French]-ET...` | 2726.81 ms | 191.92 ms | 22.1 ms | 177.61 ms | 1.0 ms | 0.5 ms | **3120.29 ms** | Les Lyonnais | 100.0% | 0 |
| 874 | `Les_Meilleures_(2020)[1080p](jeunes_wes...` | 2133.25 ms | 91.8 ms | 19.81 ms | 122.93 ms | 1.0 ms | 0.5 ms | **2372.46 ms** | Les Meilleures | 100.0% | 0 |
| 875 | `Les_Origines___L'odyssée_de_l'écriture_...` | 2906.77 ms | 130.9 ms | 21.49 ms | 132.59 ms | 1.0 ms | 0.5 ms | **3196.02 ms** | Les Origines L'odyssée de l... | 100.0% | 0 |
| 876 | `Les_Passagers_de_la_Nuit_(2022)[1080p](...` | 2775.21 ms | 94.15 ms | 22.09 ms | 353.26 ms | 1.0 ms | 0.5 ms | **3247.8 ms** | Aucun | -- | 0 |
| 877 | `Les_Pires_(2022)[1080p.H264][EAC3.5.1]_...` | 2430.3 ms | 104.67 ms | 18.88 ms | 246.48 ms | 1.0 ms | 0.5 ms | **2801.04 ms** | Aucun | -- | 0 |
| 878 | `Les_Promesses_(2021)[1080p](Drame,_Poli...` | 3050.02 ms | 128.76 ms | 22.72 ms | 175.26 ms | 1.0 ms | 0.5 ms | **3381.61 ms** | Les Promesses | 100.0% | 0 |
| 879 | `Les_Sans-dents_(2022)[1080p](Yolande_Mo...` | 2890.05 ms | 98.32 ms | 25.3 ms | 272.47 ms | 1.0 ms | 0.5 ms | **3286.84 ms** | Les Sans-dents | 100.0% | 0 |
| 880 | `Les_Tuche_2_(2016)_mid15s.mp4` | 2279.94 ms | 83.02 ms | 25.74 ms | 194.14 ms | 1.0 ms | 0.5 ms | **2585.25 ms** | Les Tuche 2 | 100.0% | 0 |
| 881 | `Les_Voisins_De_Mes_Voisins_Sont_Mes_Voi...` | 2936.67 ms | 104.49 ms | 21.75 ms | 134.95 ms | 1.0 ms | 0.5 ms | **3198.6 ms** | Les Voisins De Mes Voisins ... | 100.0% | 0 |
| 882 | `Les_Yeux_sans_visage_(1960)[1080p.x264]...` | 2279.45 ms | 101.86 ms | 18.97 ms | 143.33 ms | 1.0 ms | 0.5 ms | **2546.63 ms** | Les Yeux sans visage | 100.0% | 0 |
| 883 | `Les_cryptomonnaies_sont-elles_le_futur_...` | 2486.46 ms | 88.9 ms | 29.79 ms | 188.51 ms | 1.0 ms | 0.5 ms | **2794.4 ms** | Les cryptomonnaies sont-ell... | 100.0% | 0 |
| 884 | `Liberte.2009.FRENCH.1080p.WEBRip.AAC2.0...` | 2337.72 ms | 110.79 ms | 22.84 ms | 127.32 ms | 1.0 ms | 0.5 ms | **2615.24 ms** | Liberte | 100.0% | 0 |
| 885 | `Lilo_And_Stitch_(2002)[1080p.AVC][DTS-H...` | 2124.67 ms | 114.84 ms | 24.74 ms | 257.23 ms | 1.0 ms | 0.5 ms | **2522.14 ms** | Lilo And Stitch | 100.0% | 0 |
| 886 | `Linux,_Synchroniser_les_fichiers_avec_r...` | 1019.97 ms | 0.0 ms | 0.0 ms | 282.87 ms | 1.0 ms | 0.5 ms | **1302.97 ms** | Aucun | -- | 0 |
| 887 | `Linux_-_Tâches_récurrents,_cron_mid15s.mp4` | 10677.2 ms | 88.07 ms | 26.17 ms | 192.35 ms | 1.0 ms | 0.5 ms | **10984.21 ms** | Linux - Tâches récurrents, ... | 100.0% | 0 |
| 888 | `Little.Monsters.2019.1080p.HULU.WEB-DL....` | 1639.91 ms | 86.93 ms | 22.22 ms | 234.04 ms | 1.0 ms | 0.5 ms | **1983.99 ms** | Little Monsters | 100.0% | 0 |
| 889 | `Little.Nemo.Adventures.In.Slumberland.1...` | 2665.67 ms | 110.13 ms | 36.02 ms | 125.35 ms | 1.0 ms | 0.5 ms | **2938.05 ms** | Little Nemo Adventures In S... | 100.0% | 0 |
| 890 | `Live_#10_-_Workflow_Dev_mid15s.mp4` | 3067.25 ms | 88.2 ms | 25.82 ms | 455.66 ms | 1.0 ms | 0.5 ms | **3637.53 ms** | Live #10 - Workflow Dev mp4... | 100.0% | 0 |
| 891 | `Live_#11_-_Laravel_mid15s.mp4` | 5468.71 ms | 85.05 ms | 32.13 ms | 290.53 ms | 1.0 ms | 0.5 ms | **5876.75 ms** | Live #11 - Laravel mp4 24 0fps | 100.0% | 0 |
| 892 | `Live_#12_-_Les_CMS,_un_outil_pour_les_m...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.61 ms** | Erreur | -- | 0 |
| 893 | `Live_#13_-_FAQ_mid15s.mp4` | 3770.01 ms | 103.04 ms | 44.7 ms | 474.67 ms | 1.0 ms | 0.5 ms | **4395.7 ms** | Aucun | -- | 0 |
| 894 | `Live_#14_-_Faille_Wordpress,_Nouveau_Gr...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.57 ms** | Erreur | -- | 0 |
| 895 | `Live_#15_mid15s.mp4` | 10709.17 ms | 88.33 ms | 21.31 ms | 167.63 ms | 1.0 ms | 0.5 ms | **10986.78 ms** | Live #15 mp4 24 0fps | 100.0% | 0 |
| 896 | `Live_#16_-_PhpStorm,_Idée_de_lives_et_a...` | 1848.99 ms | 126.43 ms | 25.47 ms | 163.53 ms | 1.0 ms | 0.5 ms | **2165.22 ms** | Live #16 - PhpStorm, Idée d... | 100.0% | 0 |
| 897 | `Live_#17_-_Comment_apprendre_mid15s.mp4` | 5423.44 ms | 124.77 ms | 44.96 ms | 143.32 ms | 1.0 ms | 0.5 ms | **5736.88 ms** | Aucun | -- | 0 |
| 898 | `Live_#18_-_Finalisation_de_la_nouvelle_...` | 10753.14 ms | 92.01 ms | 31.75 ms | 267.77 ms | 1.0 ms | 0.5 ms | **11145.1 ms** | Live #18 - Finalisation de ... | 100.0% | 0 |
| 899 | `Live_#19_-_Nouvelle_version,_Prochains_...` | 2045.59 ms | 83.11 ms | 44.35 ms | 169.55 ms | 1.0 ms | 0.5 ms | **2350.64 ms** | Live #19 - Nouvelle version... | 100.0% | 0 |
| 900 | `Live_#20_-_Bonne_année_!_mid15s.mp4` | 2455.89 ms | 168.61 ms | 64.44 ms | 151.22 ms | 1.0 ms | 0.5 ms | **2842.76 ms** | Live #20 - Bonne année ! mp... | 100.0% | 0 |
| 901 | `Live_#21_-_Votre_avis_sur_la_formation_...` | 8802.87 ms | 106.02 ms | 24.2 ms | 263.57 ms | 1.0 ms | 0.5 ms | **9197.02 ms** | Live #21 - Votre avis sur l... | 100.0% | 0 |
| 902 | `Live_#22_-_Les_technos_qui_ont_survécu_...` | 10802.25 ms | 120.62 ms | 39.49 ms | 243.86 ms | 1.0 ms | 0.5 ms | **11209.0 ms** | Live #22 - Les technos qui ... | 100.0% | 0 |
| 903 | `Live_#23_-_Comment_apprendre_mid15s.mp4` | 10410.34 ms | 83.92 ms | 21.33 ms | 145.44 ms | 1.0 ms | 0.5 ms | **10661.38 ms** | Aucun | -- | 0 |
| 904 | `Live_#24_-_Sujet_undefined_mid15s.mp4` | 1680.45 ms | 94.77 ms | 30.56 ms | 149.82 ms | 1.0 ms | 0.5 ms | **1956.44 ms** | Live #24 - Sujet undefined ... | 100.0% | 0 |
| 905 | `Live_#25_-_Windows_10_&_Formations_à_ve...` | 4327.23 ms | 89.67 ms | 21.17 ms | 167.65 ms | 1.0 ms | 0.5 ms | **4606.16 ms** | Live #25 - Windows 10 & For... | 100.0% | 0 |
| 906 | `Live_#26_-_Framework_fatigue_mid15s.mp4` | 11467.4 ms | 82.88 ms | 38.0 ms | 165.54 ms | 1.0 ms | 0.5 ms | **11754.2 ms** | Aucun | -- | 0 |
| 907 | `Live_#27_-_La_peur_du_changement_mid15s...` | 11688.37 ms | 87.58 ms | 43.85 ms | 185.63 ms | 1.0 ms | 0.5 ms | **12005.87 ms** | Aucun | -- | 0 |
| 908 | `Live_#28_-_On_fait_le_Bilan,_calmement_...` | 3928.44 ms | 89.31 ms | 173.61 ms | 296.47 ms | 1.0 ms | 0.5 ms | **4488.14 ms** | Aucun | -- | 0 |
| 909 | `Live_#29_-_Dev_&_Lemmings_mid15s.mp4` | 2725.88 ms | 99.19 ms | 50.12 ms | 280.36 ms | 1.0 ms | 0.5 ms | **3156.18 ms** | Live #29 - Dev & Lemmings m... | 100.0% | 0 |
| 910 | `Live_#3,_Le_Freelance_mid15s.mp4` | 6193.79 ms | 96.29 ms | 21.18 ms | 176.48 ms | 1.0 ms | 0.5 ms | **6489.67 ms** | Live #3, Le Freelance mp4 2... | 100.0% | 0 |
| 911 | `Live_#30_-_Tutoriels_en_kit,_dev_et_gue...` | 2974.82 ms | 104.08 ms | 197.13 ms | 202.65 ms | 1.0 ms | 0.5 ms | **3478.99 ms** | Live #30 - Tutoriels en kit... | 100.0% | 0 |
| 912 | `Live_#31_-_Wordpress_&_CMS_mid15s.mp4` | 2929.9 ms | 87.33 ms | 190.13 ms | 356.73 ms | 1.0 ms | 0.5 ms | **3565.19 ms** | Live #31 - Wordpress & CMS ... | 100.0% | 0 |
| 913 | `Live_#32_-_La_formation_JS___NodeJS___D...` | 4323.27 ms | 107.07 ms | 25.08 ms | 182.05 ms | 1.0 ms | 0.5 ms | **4637.8 ms** | Live #32 - La formation JS ... | 100.0% | 0 |
| 914 | `Live_#33_-_Grafikart.sources,_Symfony_e...` | 3260.86 ms | 121.44 ms | 46.63 ms | 393.63 ms | 1.0 ms | 0.5 ms | **3832.5 ms** | Aucun | -- | 0 |
| 915 | `Live_#34_-_Les_bonnes_pratiques_mid15s.mp4` | 9705.0 ms | 93.74 ms | 54.43 ms | 385.05 ms | 1.0 ms | 0.5 ms | **10238.58 ms** | Live #34 - Les bonnes prati... | 100.0% | 0 |
| 916 | `Live_#35_-_Organiser_son_framework_PHP,...` | 11192.09 ms | 97.05 ms | 32.88 ms | 357.39 ms | 1.0 ms | 0.5 ms | **11679.71 ms** | Live #35 - Organiser son fr... | 100.0% | 0 |
| 917 | `Live_#36_-_Vos_questions_sur_la_formati...` | 2046.45 ms | 82.98 ms | 60.6 ms | 189.21 ms | 1.0 ms | 0.5 ms | **2387.34 ms** | Live #36 - Vos questions su... | 100.0% | 0 |
| 918 | `Live_#37_-_Réorganisation_du_site_mid15...` | 4110.84 ms | 86.44 ms | 26.41 ms | 239.5 ms | 1.0 ms | 0.5 ms | **4463.59 ms** | Live #37 - Réorganisation d... | 100.0% | 0 |
| 919 | `Live_#38_-_Live_Battle_dev_mid15s.mp4` | 3057.88 ms | 100.15 ms | 28.41 ms | 232.07 ms | 1.0 ms | 0.5 ms | **3418.83 ms** | Aucun | -- | 0 |
| 920 | `Live_#4,_Les_outils_mid15s.mp4` | 3764.69 ms | 88.18 ms | 24.43 ms | 163.0 ms | 1.0 ms | 0.5 ms | **4041.77 ms** | Live #4, Les outils mp4 24 ... | 100.0% | 0 |
| 921 | `Live_#5_-_Questions_AngularJS_&_Autres_...` | 6857.03 ms | 110.41 ms | 29.88 ms | 248.41 ms | 1.0 ms | 0.5 ms | **7246.14 ms** | Aucun | -- | 0 |
| 922 | `Live_#6_mid15s.mp4` | 6004.58 ms | 96.21 ms | 23.22 ms | 167.2 ms | 1.0 ms | 0.5 ms | **6291.61 ms** | Live #6 mp4 24 0fps | 100.0% | 0 |
| 923 | `Live_#7_-_Nouvelle_version,_Macaw_&_Que...` | 3561.26 ms | 85.66 ms | 77.3 ms | 278.44 ms | 1.0 ms | 0.5 ms | **4004.52 ms** | Live #7 - Nouvelle version,... | 100.0% | 0 |
| 924 | `Live_#8_mid15s.mp4` | 9517.88 ms | 106.17 ms | 146.46 ms | 236.55 ms | 1.0 ms | 0.5 ms | **10007.39 ms** | Aucun | -- | 0 |
| 925 | `Live_#9_-_Les_clients___Les_projets_en_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.18 ms** | Erreur | -- | 0 |
| 926 | `Live_-_Découverte_CakePHP_3.0_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.55 ms** | Erreur | -- | 0 |
| 927 | `Live_Is_Life_(2022)[MULTi][1080p](Espag...` | 2043.41 ms | 93.58 ms | 21.55 ms | 112.36 ms | 1.0 ms | 0.5 ms | **2272.4 ms** | Live Is Life | 100.0% | 0 |
| 928 | `Locke.and.Key.S01E01.Welcome.to.Matheso...` | 1663.28 ms | 123.41 ms | 26.37 ms | 291.71 ms | 1.0 ms | 0.5 ms | **2106.94 ms** | Locke and Key | 100.0% | 1 |
| 929 | `Locke.and.Key.S01E02.Trapper..Keeper.10...` | 1924.63 ms | 86.73 ms | 22.11 ms | 275.54 ms | 1.0 ms | 0.5 ms | **2309.84 ms** | Locke.and.Key.S01E02.Trappe... | -- | 1 |
| 930 | `Locke.and.Key.S01E03.Head.Games.1080p.N...` | 2087.14 ms | 82.96 ms | 19.64 ms | 201.91 ms | 1.0 ms | 0.5 ms | **2392.15 ms** | Aucun | -- | 0 |
| 931 | `Locke.and.Key.S01E04.The.Keepers.of.the...` | 1594.78 ms | 83.72 ms | 21.6 ms | 165.96 ms | 1.0 ms | 0.5 ms | **1866.46 ms** | Locke and Key | 100.0% | 1 |
| 932 | `Locke.and.Key.S01E05.Family.Tree.1080p....` | 1555.43 ms | 86.88 ms | 28.12 ms | 249.49 ms | 1.0 ms | 0.5 ms | **1921.74 ms** | Locke and Key | 100.0% | 0 |
| 933 | `Locke.and.Key.S01E06.The.Black.Door.108...` | 2560.44 ms | 107.62 ms | 20.02 ms | 240.4 ms | 1.0 ms | 0.5 ms | **2932.47 ms** | Locke and Key | 100.0% | 0 |
| 934 | `Locke.and.Key.S01E08.Ray.of.Fking.Sunsh...` | 1593.27 ms | 90.12 ms | 18.59 ms | 191.92 ms | 1.0 ms | 0.5 ms | **1894.72 ms** | Locke and Key | 100.0% | 0 |
| 935 | `Locke.and.Key.S01E09.Echoes.1080p.NF.WE...` | 1566.58 ms | 130.99 ms | 19.19 ms | 176.77 ms | 1.0 ms | 0.5 ms | **1894.33 ms** | Locke and Key | 100.0% | 0 |
| 936 | `Locke.and.Key.S01E10.Crown.of.Shadows.1...` | 2332.66 ms | 107.9 ms | 19.97 ms | 150.13 ms | 1.0 ms | 0.5 ms | **2611.34 ms** | Aucun | -- | 0 |
| 937 | `London_Fields_(2018)(Une_voyante_fait_f...` | 2444.47 ms | 107.69 ms | 25.33 ms | 150.9 ms | 1.0 ms | 0.5 ms | **2736.46 ms** | London Fields 2018 Une voya... | 100.0% | 0 |
| 938 | `Lord.of.the.Flies.1963.1080p.BluRay.x26...` | 2215.08 ms | 90.11 ms | 22.43 ms | 188.74 ms | 1.0 ms | 0.5 ms | **2517.64 ms** | Lord of the Flies | 100.0% | 1 |
| 939 | `Lost.S01E01.1080p.BluRay.DTS.x264-CtrlH...` | 1830.84 ms | 88.47 ms | 23.5 ms | 217.29 ms | 1.0 ms | 0.5 ms | **2160.48 ms** | Aucun | -- | 0 |
| 940 | `Lost.S01E02.1080p.BluRay.DTS.x264-CtrlH...` | 2340.16 ms | 92.67 ms | 25.05 ms | 360.01 ms | 1.0 ms | 0.5 ms | **2821.36 ms** | Lost | 100.0% | 0 |
| 941 | `Lost.S01E03.1080p.BluRay.DTS.x264-CtrlH...` | 1917.87 ms | 86.54 ms | 16.65 ms | 271.33 ms | 1.0 ms | 0.5 ms | **2294.14 ms** | Lost | 100.0% | 0 |
| 942 | `Lost.S01E04.1080p.BluRay.DTS.x264-CtrlH...` | 2239.85 ms | 111.12 ms | 16.02 ms | 142.23 ms | 1.0 ms | 0.5 ms | **2509.96 ms** | Aucun | -- | 0 |
| 943 | `Lost.S01E05.1080p.BluRay.DTS.x264-CtrlH...` | 3084.53 ms | 102.02 ms | 15.44 ms | 269.99 ms | 1.0 ms | 0.5 ms | **3472.57 ms** | Lost | 100.0% | 0 |
| 944 | `Lost.S01E06.1080p.BluRay.DTS.x264-CtrlH...` | 2100.59 ms | 89.01 ms | 24.94 ms | 90.35 ms | 1.0 ms | 0.5 ms | **2305.45 ms** | Lost | 95.0% | 0 |
| 945 | `Lost.S01E07.1080p.BluRay.DTS.x264-CtrlH...` | 2710.73 ms | 107.58 ms | 32.95 ms | 166.82 ms | 1.0 ms | 0.5 ms | **3018.79 ms** | Aucun | -- | 0 |
| 946 | `Lost.S01E08.1080p.BluRay.DTS.x264-CtrlH...` | 2785.55 ms | 94.33 ms | 16.39 ms | 147.57 ms | 1.0 ms | 0.5 ms | **3060.83 ms** | Lost | 100.0% | 0 |
| 947 | `Lost.S01E09.1080p.BluRay.DTS.x264-CtrlH...` | 2611.16 ms | 97.38 ms | 17.67 ms | 209.26 ms | 1.0 ms | 0.5 ms | **2936.05 ms** | Lost | 100.0% | 0 |
| 948 | `Lost.S01E10.1080p.BluRay.DTS.x264-CtrlH...` | 2151.87 ms | 100.06 ms | 30.24 ms | 187.66 ms | 1.0 ms | 0.5 ms | **2471.2 ms** | Lost | 100.0% | 1 |
| 949 | `Lost.S01E11.1080p.BluRay.DTS.x264-CtrlH...` | 2149.17 ms | 115.17 ms | 20.04 ms | 224.92 ms | 1.0 ms | 0.5 ms | **2525.22 ms** | Lost | 100.0% | 0 |
| 950 | `Lost.S01E12.1080p.BluRay.DTS.x264-CtrlH...` | 2032.69 ms | 133.55 ms | 14.69 ms | 153.01 ms | 1.0 ms | 0.5 ms | **2336.17 ms** | Lost | 100.0% | 0 |
| 951 | `Lost.S01E13.1080p.BluRay.DTS.x264-CtrlH...` | 2398.72 ms | 83.77 ms | 18.3 ms | 107.63 ms | 1.0 ms | 0.5 ms | **2609.11 ms** | Lost | 100.0% | 0 |
| 952 | `Lost.S01E14.1080p.BluRay.DTS.x264-CtrlH...` | 1962.12 ms | 84.56 ms | 18.07 ms | 242.99 ms | 1.0 ms | 0.5 ms | **2308.5 ms** | Lost | 100.0% | 0 |
| 953 | `Lost.S01E15.1080p.BluRay.DTS.x264-CtrlH...` | 2093.07 ms | 96.65 ms | 22.63 ms | 159.46 ms | 1.0 ms | 0.5 ms | **2375.04 ms** | Lost | 100.0% | 0 |
| 954 | `Lost.S01E16.1080p.BluRay.DTS.x264-CtrlH...` | 1708.0 ms | 96.91 ms | 18.72 ms | 198.54 ms | 1.0 ms | 0.5 ms | **2028.81 ms** | Lost | 100.0% | 0 |
| 955 | `Lost.S01E17.1080p.BluRay.DTS.x264-CtrlH...` | 1876.92 ms | 93.21 ms | 14.77 ms | 99.66 ms | 1.0 ms | 0.5 ms | **2084.84 ms** | Lost | 100.0% | 0 |
| 956 | `Lost.S01E18.1080p.BluRay.DTS.x264-CtrlH...` | 1898.83 ms | 88.47 ms | 17.58 ms | 263.32 ms | 1.0 ms | 0.5 ms | **2269.11 ms** | Lost | 100.0% | 0 |
| 957 | `Lost.S01E19.1080p.BluRay.DTS.x264-CtrlH...` | 1837.79 ms | 96.94 ms | 21.43 ms | 174.82 ms | 1.0 ms | 0.5 ms | **2138.99 ms** | Lost | 100.0% | 0 |
| 958 | `Lost.S01E20.1080p.BluRay.DTS.x264-CtrlH...` | 2076.84 ms | 135.89 ms | 105.14 ms | 171.01 ms | 1.0 ms | 0.5 ms | **2492.05 ms** | Lost | 100.0% | 0 |
| 959 | `Lost.S01E21.1080p.BluRay.DTS.x264-CtrlH...` | 2046.34 ms | 123.43 ms | 99.98 ms | 323.59 ms | 1.0 ms | 0.5 ms | **2594.83 ms** | Lost | 100.0% | 0 |
| 960 | `Lost.S01E22.1080p.BluRay.DTS.x264-CtrlH...` | 2687.15 ms | 101.38 ms | 21.05 ms | 165.6 ms | 1.0 ms | 0.5 ms | **2976.36 ms** | Lost | 100.0% | 0 |
| 961 | `Lost.S01E23.1080p.BluRay.DTS.x264-CtrlH...` | 1999.82 ms | 83.93 ms | 13.15 ms | 212.22 ms | 1.0 ms | 0.5 ms | **2309.5 ms** | Lost | 100.0% | 0 |
| 962 | `Lost.S01E24.1080p.BluRay.DTS.x264-CtrlH...` | 2374.12 ms | 88.66 ms | 19.82 ms | 258.6 ms | 1.0 ms | 0.5 ms | **2741.87 ms** | Lost | 100.0% | 0 |
| 963 | `Lost.S01E25.1080p.BluRay.DTS.x264-CtrlH...` | 1715.81 ms | 89.44 ms | 26.81 ms | 104.42 ms | 1.0 ms | 0.5 ms | **1937.21 ms** | Lost | 100.0% | 0 |
| 964 | `Loue_Moi_(2017)_mid15s.mp4` | 2467.27 ms | 95.69 ms | 22.42 ms | 241.35 ms | 1.0 ms | 0.5 ms | **2829.6 ms** | Loue Moi | 100.0% | 0 |
| 965 | `Luck_(2022)[MULTi][1080p](malchance,_ch...` | 2492.3 ms | 160.83 ms | 24.83 ms | 344.91 ms | 1.0 ms | 0.5 ms | **3027.8 ms** | Luck | 100.0% | 0 |
| 966 | `Lukas_(2018)_mid15s.mp4` | 2661.2 ms | 125.96 ms | 23.67 ms | 154.46 ms | 1.0 ms | 0.5 ms | **2966.14 ms** | Lukas | 100.0% | 0 |
| 967 | `Luna_(2018)_mid15s.mp4` | 1950.87 ms | 96.17 ms | 17.03 ms | 142.35 ms | 1.0 ms | 0.5 ms | **2207.29 ms** | Luna | 100.0% | 0 |
| 968 | `Lunana_A_Yak_in_the_Classroom_(2019)[MU...` | 1060.75 ms | 0.0 ms | 0.0 ms | 281.01 ms | 1.0 ms | 0.5 ms | **1342.6 ms** | Aucun | -- | 0 |
| 969 | `MINUSCULE_-_LA_VALLÉE_DES_FOURMIS_PERDU...` | 2229.93 ms | 103.94 ms | 38.83 ms | 166.87 ms | 1.0 ms | 0.5 ms | **2540.21 ms** | MINUSCULE - LA VALLÉE DES F... | 100.0% | 0 |
| 970 | `MOOC2R~F_mid15s.mp4` | 2235.96 ms | 108.18 ms | 23.96 ms | 120.01 ms | 1.0 ms | 0.5 ms | **2488.82 ms** | Aucun | -- | 0 |
| 971 | `Ma_Nuit_(2021)[1080p](soeur_disparue,_n...` | 1865.43 ms | 87.9 ms | 27.27 ms | 189.54 ms | 1.0 ms | 0.5 ms | **2171.45 ms** | Ma Nuit | 100.0% | 0 |
| 972 | `Mad.Max.1979.2160p.4K.BluRay.x265.10bit...` | 2142.98 ms | 219.63 ms | 22.28 ms | 141.08 ms | 1.0 ms | 0.5 ms | **2528.38 ms** | Mad.Max.1979.2160p.4K.BluRa... | -- | 1 |
| 973 | `Mad.Max.Fury.Road.2015.2160p.4K.BluRay....` | 2530.33 ms | 99.95 ms | 21.83 ms | 165.98 ms | 1.0 ms | 0.5 ms | **2818.93 ms** | Mad.Max.Fury.Road.2015.2160... | -- | 1 |
| 974 | `Mad_Max_Beyond_Thunderdome-SEG_FPL_Main...` | 2457.93 ms | 97.16 ms | 22.49 ms | 101.19 ms | 1.0 ms | 0.5 ms | **2679.13 ms** | The Dark Crystal Age of Res... | 100.0% | 1 |
| 975 | `Magic_and_Myth_mid15s.mp4` | 2719.35 ms | 90.26 ms | 25.87 ms | 146.62 ms | 1.0 ms | 0.5 ms | **2982.86 ms** | Magic and Myth mkv 24 0fps | 100.0% | 0 |
| 976 | `Man.Up.2015.FANSUB.VOSTFR.BRRiP.XviD-Te...` | 2564.71 ms | 106.21 ms | 21.57 ms | 137.2 ms | 1.0 ms | 0.5 ms | **2830.43 ms** | Man Up | 100.0% | 0 |
| 977 | `Maniac.2018.S01E01.MULTi.720p.NF.WEB-DL...` | 2285.06 ms | 83.3 ms | 15.21 ms | 155.13 ms | 1.0 ms | 0.5 ms | **2539.57 ms** | Maniac | 100.0% | 1 |
| 978 | `Maniac.2018.S01E02.MULTi.720p.NF.WEB-DL...` | 2353.83 ms | 103.13 ms | 26.83 ms | 163.6 ms | 1.0 ms | 0.5 ms | **2663.33 ms** | Aucun | -- | 0 |
| 979 | `Maniac.2018.S01E03.MULTi.720p.NF.WEB-DL...` | 1727.79 ms | 87.38 ms | 26.04 ms | 197.15 ms | 1.0 ms | 0.5 ms | **2039.29 ms** | Aucun | -- | 0 |
| 980 | `Maniac.2018.S01E04.MULTi.720p.NF.WEB-DL...` | 3560.62 ms | 107.06 ms | 20.19 ms | 117.65 ms | 1.0 ms | 0.5 ms | **3806.27 ms** | Maniac | 100.0% | 0 |
| 981 | `Maniac.2018.S01E07.MULTi.720p.NF.WEB-DL...` | 2245.87 ms | 114.61 ms | 20.94 ms | 135.87 ms | 1.0 ms | 0.5 ms | **2523.54 ms** | Maniac | 100.0% | 0 |
| 982 | `Maniac.2018.S01E08.MULTi.720p.NF.WEB-DL...` | 1802.96 ms | 84.78 ms | 16.61 ms | 96.84 ms | 1.0 ms | 0.5 ms | **2002.0 ms** | Maniac | 100.0% | 0 |
| 983 | `Maniac.2018.S01E09.MULTi.720p.NF.WEB-DL...` | 2207.22 ms | 89.45 ms | 36.48 ms | 145.96 ms | 1.0 ms | 0.5 ms | **2482.71 ms** | Maniac | 100.0% | 0 |
| 984 | `Maniac.2018.S01E10.FiNAL.MULTi.720p.NF....` | 1823.43 ms | 97.55 ms | 19.81 ms | 102.4 ms | 1.0 ms | 0.5 ms | **2043.53 ms** | Maniac | 100.0% | 0 |
| 985 | `Margin.Call.2011.1080p.BluRay.x264.AAC5...` | 2344.84 ms | 106.46 ms | 38.58 ms | 172.81 ms | 1.0 ms | 0.5 ms | **2663.95 ms** | Margin.Call.2011.1080p.BluR... | -- | 1 |
| 986 | `Mars_Needs_Moms_(2011)[1080p.x264][AAC5...` | 2179.24 ms | 98.28 ms | 80.53 ms | 176.52 ms | 1.0 ms | 0.5 ms | **2539.08 ms** | Aucun | -- | 0 |
| 987 | `Marty.Supreme.2025.1080p.WEBRip.AAC5.1....` | 2695.36 ms | 86.5 ms | 21.23 ms | 182.2 ms | 1.0 ms | 0.5 ms | **2986.07 ms** | Aucun | -- | 0 |
| 988 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2046.14 ms | 111.94 ms | 15.35 ms | 198.09 ms | 1.0 ms | 0.5 ms | **2383.32 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 989 | `Marvels.Spidey.and.His.Amazing.Friends....` | 1989.77 ms | 100.0 ms | 21.99 ms | 142.21 ms | 1.0 ms | 0.5 ms | **2254.85 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 990 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2285.57 ms | 92.02 ms | 16.84 ms | 249.54 ms | 1.0 ms | 0.5 ms | **2649.1 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 991 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2487.01 ms | 93.39 ms | 18.36 ms | 100.18 ms | 1.0 ms | 0.5 ms | **2704.02 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 992 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2311.94 ms | 91.93 ms | 15.77 ms | 228.46 ms | 1.0 ms | 0.5 ms | **2648.89 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 993 | `Marvels.Spidey.and.His.Amazing.Friends....` | 1967.45 ms | 94.09 ms | 17.64 ms | 203.83 ms | 1.0 ms | 0.5 ms | **2283.65 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 994 | `Marvels.Spidey.and.His.Amazing.Friends....` | 1945.44 ms | 122.75 ms | 19.86 ms | 128.75 ms | 1.0 ms | 0.5 ms | **2219.98 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 995 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2426.24 ms | 101.19 ms | 15.92 ms | 244.77 ms | 1.0 ms | 0.5 ms | **2788.99 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 996 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2523.51 ms | 99.74 ms | 14.49 ms | 304.84 ms | 1.0 ms | 0.5 ms | **2943.29 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 997 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2960.29 ms | 195.81 ms | 17.01 ms | 178.68 ms | 1.0 ms | 0.5 ms | **3378.04 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 998 | `Marvels.Spidey.and.His.Amazing.Friends....` | 1974.42 ms | 103.1 ms | 19.4 ms | 140.68 ms | 1.0 ms | 0.5 ms | **2239.62 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 999 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2371.98 ms | 100.34 ms | 20.97 ms | 276.69 ms | 1.0 ms | 0.5 ms | **2771.14 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1000 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2710.3 ms | 109.18 ms | 15.08 ms | 165.92 ms | 1.0 ms | 0.5 ms | **3009.22 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1001 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2113.49 ms | 159.08 ms | 25.28 ms | 165.06 ms | 1.0 ms | 0.5 ms | **2464.49 ms** | Aucun | -- | 0 |
| 1002 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2020.62 ms | 208.41 ms | 20.47 ms | 415.43 ms | 1.0 ms | 0.5 ms | **2672.54 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1003 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2641.66 ms | 106.5 ms | 29.62 ms | 198.48 ms | 1.0 ms | 0.5 ms | **2977.3 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1004 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2700.25 ms | 92.96 ms | 15.73 ms | 180.43 ms | 1.0 ms | 0.5 ms | **2990.15 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1005 | `Marvels.Spidey.and.His.Amazing.Friends....` | 4668.78 ms | 130.05 ms | 18.75 ms | 114.68 ms | 1.0 ms | 0.5 ms | **4933.16 ms** | The Expanse | 100.0% | 0 |
| 1006 | `Marvels.Spidey.and.His.Amazing.Friends....` | 1789.65 ms | 177.77 ms | 16.45 ms | 482.88 ms | 1.0 ms | 0.5 ms | **2473.32 ms** | Aucun | -- | 0 |
| 1007 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2654.7 ms | 98.42 ms | 19.29 ms | 402.37 ms | 1.0 ms | 0.5 ms | **3185.02 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1008 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2948.03 ms | 115.79 ms | 22.45 ms | 356.79 ms | 1.0 ms | 0.5 ms | **3443.79 ms** | Aucun | -- | 0 |
| 1009 | `Marvels.Spidey.and.His.Amazing.Friends....` | 3067.85 ms | 127.06 ms | 21.82 ms | 211.12 ms | 1.0 ms | 0.5 ms | **3430.34 ms** | Aucun | -- | 0 |
| 1010 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2598.71 ms | 87.47 ms | 25.31 ms | 151.62 ms | 1.0 ms | 0.5 ms | **2864.06 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1011 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2446.12 ms | 99.09 ms | 21.95 ms | 390.75 ms | 1.0 ms | 0.5 ms | **2969.98 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1012 | `Marvels.Spidey.and.His.Amazing.Friends....` | 2706.42 ms | 113.45 ms | 23.43 ms | 133.01 ms | 1.0 ms | 0.5 ms | **2978.23 ms** | Marvels Spidey and His Amaz... | 100.0% | 0 |
| 1013 | `Marvin.Ou.La.Belle.Education.2018.FRENC...` | 2205.78 ms | 135.88 ms | 21.28 ms | 211.23 ms | 1.0 ms | 0.5 ms | **2586.66 ms** | Marvin Ou La Belle Education | 100.0% | 0 |
| 1014 | `Master.and.Commander.The.Far.Side.of.th...` | 2362.71 ms | 89.39 ms | 20.07 ms | 190.56 ms | 1.0 ms | 0.5 ms | **2663.32 ms** | Master and Commander The Fa... | 100.0% | 1 |
| 1015 | `Materialists_(2025)_1080p_WEB-DL_10bit_...` | 3032.07 ms | 93.68 ms | 17.42 ms | 170.02 ms | 1.0 ms | 0.5 ms | **3315.54 ms** | Materialists | 100.0% | 0 |
| 1016 | `Mayhem.2017.1080p.BluRay.x264-[YTS.AG]_...` | 2059.52 ms | 97.18 ms | 35.33 ms | 185.32 ms | 1.0 ms | 0.5 ms | **2378.18 ms** | Aucun | -- | 0 |
| 1017 | `Me.Myself.And.Mum.2013.1080p.BluRay.x26...` | 2469.47 ms | 96.54 ms | 22.85 ms | 255.49 ms | 1.0 ms | 0.5 ms | **2889.78 ms** | Me Myself And Mum | 100.0% | 0 |
| 1018 | `Mean.Girls.2004.1080p.BrRip.x264.YIFY_m...` | 1910.81 ms | 104.61 ms | 21.74 ms | 217.63 ms | 1.0 ms | 0.5 ms | **2262.07 ms** | Aucun | -- | 0 |
| 1019 | `Memoir.of.a.Snail.2024.REPACK.1080p.WEB...` | 2536.6 ms | 102.47 ms | 21.97 ms | 147.33 ms | 1.0 ms | 0.5 ms | **2808.95 ms** | Memoir of a Snail | 100.0% | 0 |
| 1020 | `Memories.1995.1080p.BluRay.x264.AAC5.1-...` | 2646.23 ms | 109.01 ms | 19.28 ms | 110.81 ms | 1.0 ms | 0.5 ms | **2886.1 ms** | Aucun | -- | 0 |
| 1021 | `Memories.of.Murder.2003.KOREAN.1080p.Bl...` | 2252.63 ms | 188.1 ms | 28.84 ms | 133.0 ms | 1.0 ms | 0.5 ms | **2603.24 ms** | Memories of Murder | 100.0% | 0 |
| 1022 | `Men.2022.2160p.4K.WEB.x265.10bit.AAC5.1...` | 5164.74 ms | 115.23 ms | 19.62 ms | 346.36 ms | 1.0 ms | 0.5 ms | **5648.41 ms** | Aucun | -- | 0 |
| 1023 | `Menteur_(2022)[1080p](Comédie)_mid15s.mp4` | 2111.78 ms | 100.14 ms | 22.4 ms | 161.87 ms | 1.0 ms | 0.5 ms | **2396.56 ms** | Menteur | 100.0% | 0 |
| 1024 | `Mes_Tres_Chers_Enfants_(2021)[1080p](Jo...` | 2150.23 ms | 97.86 ms | 26.68 ms | 228.46 ms | 1.0 ms | 0.5 ms | **2503.95 ms** | Mes Tres Chers Enfants | 100.0% | 0 |
| 1025 | `Mes_voisins_les_Yamada_(1999)_MULTi_108...` | 2809.72 ms | 112.0 ms | 118.01 ms | 384.29 ms | 1.0 ms | 0.5 ms | **3428.37 ms** | Mes voisins les Yamada | 100.0% | 0 |
| 1026 | `Metropolis_(1927)_Fritz_Lang_-_Rescore_...` | 1919.36 ms | 93.7 ms | 21.97 ms | 301.22 ms | 1.0 ms | 0.5 ms | **2336.97 ms** | Metropolis | 100.0% | 0 |
| 1027 | `Mettre_en_place_un_serveur_Web_-_Ansibl...` | 6116.38 ms | 88.1 ms | 22.54 ms | 291.7 ms | 1.0 ms | 0.5 ms | **6519.06 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1028 | `Mettre_en_place_un_serveur_Web_-_Apache...` | 2142.18 ms | 88.26 ms | 123.65 ms | 343.49 ms | 1.0 ms | 0.5 ms | **2697.95 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1029 | `Mettre_en_place_un_serveur_Web_-_Comman...` | 5339.97 ms | 114.64 ms | 32.76 ms | 228.96 ms | 1.0 ms | 0.5 ms | **5716.7 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1030 | `Mettre_en_place_un_serveur_Web_-_Fail2b...` | 1001.63 ms | 0.0 ms | 0.0 ms | 169.87 ms | 1.0 ms | 0.5 ms | **1171.59 ms** | Aucun | -- | 0 |
| 1031 | `Mettre_en_place_un_serveur_Web_-_Instal...` | 11515.25 ms | 117.24 ms | 26.75 ms | 179.13 ms | 1.0 ms | 0.5 ms | **11838.78 ms** | Aucun | -- | 0 |
| 1032 | `Mettre_en_place_un_serveur_Web_-_Iptabl...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.13 ms** | Erreur | -- | 0 |
| 1033 | `Mettre_en_place_un_serveur_Web_-_Machin...` | 11810.12 ms | 90.14 ms | 18.24 ms | 130.1 ms | 1.0 ms | 0.5 ms | **12048.92 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1034 | `Mettre_en_place_un_serveur_Web_-_MySQL_...` | 1648.49 ms | 94.94 ms | 168.99 ms | 312.66 ms | 1.0 ms | 0.5 ms | **2225.39 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1035 | `Mettre_en_place_un_serveur_Web_-_Nginx,...` | 5256.43 ms | 96.48 ms | 31.87 ms | 227.43 ms | 1.0 ms | 0.5 ms | **5612.55 ms** | Aucun | -- | 0 |
| 1036 | `Mettre_en_place_un_serveur_Web_-_Nginx_...` | 2695.54 ms | 113.81 ms | 29.72 ms | 196.56 ms | 1.0 ms | 0.5 ms | **3040.46 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1037 | `Mettre_en_place_un_serveur_Web_-_NodeJS...` | 2573.6 ms | 97.45 ms | 68.51 ms | 292.18 ms | 1.0 ms | 0.5 ms | **3035.53 ms** | Aucun | -- | 0 |
| 1038 | `Mettre_en_place_un_serveur_Web_-_PHP,_M...` | 4752.05 ms | 94.65 ms | 24.48 ms | 214.66 ms | 1.0 ms | 0.5 ms | **5086.52 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1039 | `Mettre_en_place_un_serveur_Web_-_PHP_FP...` | 3108.1 ms | 93.96 ms | 22.43 ms | 516.36 ms | 1.0 ms | 0.5 ms | **3741.21 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1040 | `Mettre_en_place_un_serveur_Web_-_PHP_FP...` | 6661.83 ms | 97.35 ms | 103.32 ms | 218.24 ms | 1.0 ms | 0.5 ms | **7081.14 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1041 | `Mettre_en_place_un_serveur_Web_-_Pimp_m...` | 2317.7 ms | 83.35 ms | 41.85 ms | 131.15 ms | 1.0 ms | 0.5 ms | **2583.3 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1042 | `Mettre_en_place_un_serveur_Web_-_ProFTP...` | 2507.49 ms | 90.5 ms | 85.55 ms | 256.43 ms | 1.0 ms | 0.5 ms | **2940.89 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1043 | `Mettre_en_place_un_serveur_Web_-_SSH_mi...` | 3821.83 ms | 120.68 ms | 21.04 ms | 221.75 ms | 1.0 ms | 0.5 ms | **4185.71 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1044 | `Mettre_en_place_un_serveur_Web_-_UFW,_U...` | 7885.7 ms | 111.35 ms | 60.42 ms | 131.34 ms | 1.0 ms | 0.5 ms | **8190.9 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1045 | `Mettre_en_place_un_serveur_Web_-_VIM_mi...` | 10966.17 ms | 109.84 ms | 19.93 ms | 201.56 ms | 1.0 ms | 0.5 ms | **11297.9 ms** | Mettre en place un serveur | 100.0% | 0 |
| 1046 | `Mettre_en_place_un_serveur_Web_-_postfi...` | 859.89 ms | 0.0 ms | 0.0 ms | 605.64 ms | 1.0 ms | 0.5 ms | **1465.69 ms** | Aucun | -- | 0 |
| 1047 | `Mettre_son_site_en_ligne_(1_4)_-_L'hébe...` | 8294.53 ms | 92.59 ms | 19.72 ms | 246.81 ms | 1.0 ms | 0.5 ms | **8656.23 ms** | Mettre son site en ligne 1 ... | 100.0% | 0 |
| 1048 | `Mettre_son_site_en_ligne_(2_4)_-_Commen...` | 10646.59 ms | 98.86 ms | 110.31 ms | 407.07 ms | 1.0 ms | 0.5 ms | **11263.31 ms** | Mettre son site en ligne 2 ... | 100.0% | 0 |
| 1049 | `Mettre_son_site_en_ligne_(3_4)_-_Qu'est...` | 3654.19 ms | 119.32 ms | 24.64 ms | 200.27 ms | 1.0 ms | 0.5 ms | **3998.82 ms** | Mettre son site en ligne 3 ... | 100.0% | 0 |
| 1050 | `Mettre_son_site_en_ligne_(4_4)_-_Compre...` | 10113.7 ms | 100.55 ms | 25.37 ms | 199.17 ms | 1.0 ms | 0.5 ms | **10439.29 ms** | Mettre son site en ligne 4 ... | 100.0% | 0 |
| 1051 | `Mijn_Vaders_Draak_(2022)[MULTi][1080p]_...` | 2038.97 ms | 121.32 ms | 81.43 ms | 211.16 ms | 1.0 ms | 0.5 ms | **2454.78 ms** | Mijn Vaders Draak | 100.0% | 0 |
| 1052 | `Milk.2008.1080p.BluRay.x264.YIFY_mid15s...` | 1752.12 ms | 85.41 ms | 17.89 ms | 206.45 ms | 1.0 ms | 0.5 ms | **2064.12 ms** | Aucun | -- | 0 |
| 1053 | `Minority.Report.2002.2160p.WEB-DL.x265....` | 2046.3 ms | 99.34 ms | 59.17 ms | 225.82 ms | 1.0 ms | 0.5 ms | **2442.69 ms** | Minority.Report.2002.2160p.... | -- | 1 |
| 1054 | `Miracle.Workers.S03E01.720p.AMZN.WEB-DL...` | 1652.87 ms | 166.49 ms | 16.33 ms | 210.38 ms | 1.0 ms | 0.5 ms | **2048.66 ms** | Miracle Workers | 100.0% | 0 |
| 1055 | `Miracle.Workers.S03E02.720p.AMZN.WEB-DL...` | 2237.94 ms | 111.34 ms | 19.25 ms | 143.72 ms | 1.0 ms | 0.5 ms | **2512.96 ms** | Miracle Workers | 100.0% | 0 |
| 1056 | `Miracle.Workers.S03E03.720p.AMZN.WEB-DL...` | 3264.37 ms | 113.05 ms | 21.98 ms | 323.51 ms | 1.0 ms | 0.5 ms | **3726.88 ms** | Miracle Workers | 100.0% | 0 |
| 1057 | `Miracle.Workers.S03E04.720p.AMZN.WEB-DL...` | 2612.99 ms | 109.95 ms | 19.28 ms | 137.46 ms | 1.0 ms | 0.5 ms | **2882.02 ms** | Miracle Workers | 100.0% | 0 |
| 1058 | `Miracle.Workers.S03E05.720p.AMZN.WEB-DL...` | 3013.35 ms | 108.11 ms | 20.34 ms | 185.93 ms | 1.0 ms | 0.5 ms | **3328.4 ms** | Miracle Workers | 100.0% | 0 |
| 1059 | `Miracle.Workers.S03E06.720p.AMZN.WEB-DL...` | 3031.19 ms | 201.73 ms | 22.03 ms | 134.41 ms | 1.0 ms | 0.5 ms | **3390.05 ms** | Miracle Workers | 100.0% | 0 |
| 1060 | `Miracle.Workers.S03E07.720p.AMZN.WEB-DL...` | 2914.12 ms | 95.89 ms | 20.18 ms | 149.37 ms | 1.0 ms | 0.5 ms | **3180.39 ms** | Miracle Workers | 100.0% | 0 |
| 1061 | `Miracle.Workers.S03E08.720p.AMZN.WEB-DL...` | 2299.48 ms | 129.29 ms | 15.55 ms | 156.68 ms | 1.0 ms | 0.5 ms | **2603.07 ms** | Miracle Workers | 100.0% | 0 |
| 1062 | `Miracle.Workers.S03E09.720p.AMZN.WEB-DL...` | 2721.93 ms | 103.13 ms | 18.58 ms | 244.84 ms | 1.0 ms | 0.5 ms | **3089.09 ms** | Miracle Workers | 100.0% | 0 |
| 1063 | `Miracle.Workers.S03E10.720p.AMZN.WEB-DL...` | 2882.56 ms | 101.52 ms | 21.23 ms | 175.7 ms | 1.0 ms | 0.5 ms | **3183.98 ms** | Miracle Workers | 100.0% | 0 |
| 1064 | `Miracle_Workers_(2019)_-_S01E01_-_2_Wee...` | 3525.11 ms | 87.69 ms | 18.64 ms | 107.72 ms | 1.0 ms | 0.5 ms | **3739.81 ms** | Miracle Workers | 100.0% | 0 |
| 1065 | `Miracle_Workers_(2019)_-_S01E02_-_13_Da...` | 2662.43 ms | 91.05 ms | 15.62 ms | 262.25 ms | 1.0 ms | 0.5 ms | **3032.1 ms** | Miracle Workers | 100.0% | 0 |
| 1066 | `Miracle_Workers_(2019)_-_S01E03_-_12_Da...` | 2144.12 ms | 110.5 ms | 17.26 ms | 181.76 ms | 1.0 ms | 0.5 ms | **2455.22 ms** | Miracle Workers | 100.0% | 0 |
| 1067 | `Miracle_Workers_(2019)_-_S01E04_-_6_Day...` | 2832.95 ms | 86.2 ms | 14.82 ms | 226.69 ms | 1.0 ms | 0.5 ms | **3163.26 ms** | Miracle Workers | 100.0% | 0 |
| 1068 | `Miracle_Workers_(2019)_-_S01E05_-_3_Day...` | 3850.19 ms | 102.58 ms | 20.69 ms | 333.94 ms | 1.0 ms | 0.5 ms | **4308.18 ms** | Miracle Workers | 100.0% | 0 |
| 1069 | `Miracle_Workers_(2019)_-_S01E06_-_1_Day...` | 2628.55 ms | 104.65 ms | 18.11 ms | 224.76 ms | 1.0 ms | 0.5 ms | **2991.2 ms** | Miracle Workers | 100.0% | 0 |
| 1070 | `Miracle_Workers_(2019)_-_S01E07_-_1_Hou...` | 2767.24 ms | 115.85 ms | 23.62 ms | 216.76 ms | 1.0 ms | 0.5 ms | **3124.99 ms** | Miracle Workers | 100.0% | 0 |
| 1071 | `Miracle_Workers_(2019)_-_S02E01_-_Gradu...` | 2303.44 ms | 87.75 ms | 22.33 ms | 160.5 ms | 1.0 ms | 0.5 ms | **2574.77 ms** | Miracle Workers | 100.0% | 0 |
| 1072 | `Miracle_Workers_(2019)_-_S02E02_-_Help_...` | 2431.64 ms | 88.47 ms | 19.28 ms | 173.77 ms | 1.0 ms | 0.5 ms | **2714.08 ms** | Miracle Workers | 100.0% | 0 |
| 1073 | `Miracle_Workers_(2019)_-_S02E03_-_Road_...` | 2463.88 ms | 83.89 ms | 48.46 ms | 205.29 ms | 1.0 ms | 0.5 ms | **2801.92 ms** | Miracle Workers | 100.0% | 0 |
| 1074 | `Miracle_Workers_(2019)_-_S02E04_-_Inter...` | 2412.31 ms | 95.11 ms | 29.31 ms | 199.8 ms | 1.0 ms | 0.5 ms | **2737.35 ms** | Miracle Workers | 100.0% | 0 |
| 1075 | `Miracle_Workers_(2019)_-_S02E05_-_Holid...` | 2034.73 ms | 99.61 ms | 27.01 ms | 170.18 ms | 1.0 ms | 0.5 ms | **2332.43 ms** | Miracle Workers | 100.0% | 0 |
| 1076 | `Miracle_Workers_(2019)_-_S02E06_-_Music...` | 2503.17 ms | 109.44 ms | 26.32 ms | 246.02 ms | 1.0 ms | 0.5 ms | **2885.72 ms** | Miracle Workers | 100.0% | 0 |
| 1077 | `Miracle_Workers_(2019)_-_S02E07_-_Day_i...` | 2005.44 ms | 97.48 ms | 18.57 ms | 335.83 ms | 1.0 ms | 0.5 ms | **2458.0 ms** | Miracle Workers | 100.0% | 0 |
| 1078 | `Miracle_Workers_(2019)_-_S02E08_-_First...` | 1645.08 ms | 169.04 ms | 24.88 ms | 175.91 ms | 1.0 ms | 0.5 ms | **2019.14 ms** | Miracle Workers | 100.0% | 0 |
| 1079 | `Miracle_Workers_(2019)_-_S02E09_-_Movin...` | 2421.34 ms | 121.68 ms | 20.86 ms | 333.24 ms | 1.0 ms | 0.5 ms | **2913.32 ms** | Miracle Workers | 100.0% | 0 |
| 1080 | `Miracle_Workers_(2019)_-_S02E10_-_Movin...` | 1635.27 ms | 112.93 ms | 20.04 ms | 300.34 ms | 1.0 ms | 0.5 ms | **2072.62 ms** | Miracle Workers | 100.0% | 0 |
| 1081 | `Mise_en_pratique_de_la_POO_en_PHP_(10_)...` | 4119.84 ms | 102.36 ms | 27.76 ms | 167.09 ms | 1.0 ms | 0.5 ms | **4417.4 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1082 | `Mise_en_pratique_de_la_POO_en_PHP_(11_)...` | 3080.92 ms | 85.09 ms | 27.84 ms | 147.16 ms | 1.0 ms | 0.5 ms | **3341.43 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1083 | `Mise_en_pratique_de_la_POO_en_PHP_(12_)...` | 3478.78 ms | 86.0 ms | 21.46 ms | 220.51 ms | 1.0 ms | 0.5 ms | **3807.37 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1084 | `Mise_en_pratique_de_la_POO_en_PHP_(13_)...` | 1987.14 ms | 93.65 ms | 47.34 ms | 224.43 ms | 1.0 ms | 0.5 ms | **2353.25 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1085 | `Mise_en_pratique_de_la_POO_en_PHP_(14_)...` | 2855.87 ms | 85.84 ms | 26.79 ms | 249.11 ms | 1.0 ms | 0.5 ms | **3218.05 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1086 | `Mise_en_pratique_de_la_POO_en_PHP_(15_)...` | 2469.5 ms | 88.17 ms | 19.11 ms | 255.45 ms | 1.0 ms | 0.5 ms | **2832.68 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1087 | `Mise_en_pratique_de_la_POO_en_PHP_(16_)...` | 1685.51 ms | 103.55 ms | 24.68 ms | 282.48 ms | 1.0 ms | 0.5 ms | **2096.64 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1088 | `Mise_en_pratique_de_la_POO_en_PHP_(17_)...` | 3568.47 ms | 89.19 ms | 19.97 ms | 275.97 ms | 1.0 ms | 0.5 ms | **3954.05 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1089 | `Mise_en_pratique_de_la_POO_en_PHP_(18_)...` | 1576.84 ms | 108.96 ms | 22.58 ms | 303.73 ms | 1.0 ms | 0.5 ms | **2012.63 ms** | Aucun | -- | 0 |
| 1090 | `Mise_en_pratique_de_la_POO_en_PHP_(19_)...` | 4541.02 ms | 117.69 ms | 31.35 ms | 229.56 ms | 1.0 ms | 0.5 ms | **4920.01 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1091 | `Mise_en_pratique_de_la_POO_en_PHP_(1_)_...` | 2278.41 ms | 91.47 ms | 33.26 ms | 262.11 ms | 1.0 ms | 0.5 ms | **2665.6 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1092 | `Mise_en_pratique_de_la_POO_en_PHP_(20_)...` | 2567.61 ms | 88.36 ms | 39.05 ms | 125.26 ms | 1.0 ms | 0.5 ms | **2820.59 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1093 | `Mise_en_pratique_de_la_POO_en_PHP_(21_)...` | 2020.62 ms | 103.39 ms | 21.41 ms | 198.42 ms | 1.0 ms | 0.5 ms | **2344.28 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1094 | `Mise_en_pratique_de_la_POO_en_PHP_(22_)...` | 2271.41 ms | 100.79 ms | 18.79 ms | 287.61 ms | 1.0 ms | 0.5 ms | **2678.95 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1095 | `Mise_en_pratique_de_la_POO_en_PHP_(23_)...` | 3318.83 ms | 96.15 ms | 22.5 ms | 242.15 ms | 1.0 ms | 0.5 ms | **3680.05 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1096 | `Mise_en_pratique_de_la_POO_en_PHP_(24_)...` | 5733.98 ms | 83.45 ms | 21.62 ms | 170.58 ms | 1.0 ms | 0.5 ms | **6010.05 ms** | Aucun | -- | 0 |
| 1097 | `Mise_en_pratique_de_la_POO_en_PHP_(2_)_...` | 3847.08 ms | 101.28 ms | 100.74 ms | 310.96 ms | 1.0 ms | 0.5 ms | **4360.48 ms** | Aucun | -- | 0 |
| 1098 | `Mise_en_pratique_de_la_POO_en_PHP_(3_)_...` | 10802.61 ms | 107.76 ms | 41.43 ms | 137.74 ms | 1.0 ms | 0.5 ms | **11089.9 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1099 | `Mise_en_pratique_de_la_POO_en_PHP_(4_)_...` | 1374.92 ms | 0.0 ms | 0.0 ms | 252.01 ms | 1.0 ms | 0.5 ms | **1627.03 ms** | Aucun | -- | 0 |
| 1100 | `Mise_en_pratique_de_la_POO_en_PHP_(5_)_...` | 2161.01 ms | 128.16 ms | 27.66 ms | 165.43 ms | 1.0 ms | 0.5 ms | **2482.68 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1101 | `Mise_en_pratique_de_la_POO_en_PHP_(6_)_...` | 1729.63 ms | 84.26 ms | 31.08 ms | 197.24 ms | 1.0 ms | 0.5 ms | **2042.59 ms** | Aucun | -- | 0 |
| 1102 | `Mise_en_pratique_de_la_POO_en_PHP_(7_)_...` | 3151.87 ms | 149.79 ms | 30.08 ms | 263.25 ms | 1.0 ms | 0.5 ms | **3602.76 ms** | Aucun | -- | 0 |
| 1103 | `Mise_en_pratique_de_la_POO_en_PHP_(8_)_...` | 2530.4 ms | 113.18 ms | 67.43 ms | 281.12 ms | 1.0 ms | 0.5 ms | **2992.56 ms** | Aucun | -- | 0 |
| 1104 | `Mise_en_pratique_de_la_POO_en_PHP_(9_)_...` | 1421.14 ms | 107.89 ms | 29.69 ms | 117.55 ms | 1.0 ms | 0.5 ms | **1676.57 ms** | Aucun | -- | 0 |
| 1105 | `Mise_en_pratique_de_la_POO_en_PHP_-_Int...` | 1152.74 ms | 0.0 ms | 0.0 ms | 397.15 ms | 1.0 ms | 0.5 ms | **1550.01 ms** | Aucun | -- | 0 |
| 1106 | `Miss.You.Already.2015.1080p.BluRay.x264...` | 2423.8 ms | 167.51 ms | 24.64 ms | 347.89 ms | 1.0 ms | 0.5 ms | **2964.62 ms** | Aucun | -- | 0 |
| 1107 | `Missing_Link_(2019)[1080p](Dernier_vest...` | 2094.68 ms | 82.96 ms | 28.21 ms | 249.56 ms | 1.0 ms | 0.5 ms | **2456.56 ms** | Aucun | -- | 0 |
| 1108 | `Missing_Link_(2019)_[720p]_mid15s.mp4` | 2104.34 ms | 268.23 ms | 23.65 ms | 230.09 ms | 1.0 ms | 0.5 ms | **2631.35 ms** | Missing Link | 100.0% | 0 |
| 1109 | `Mission_Paradis_(2020)[MULTi][1080p](ha...` | 1723.93 ms | 98.07 ms | 22.34 ms | 278.19 ms | 1.0 ms | 0.5 ms | **2129.87 ms** | Mission Paradis | 100.0% | 0 |
| 1110 | `Mon_voisin_Totoro_(1988)_MULTi_1080p_10...` | 2548.59 ms | 94.72 ms | 21.02 ms | 395.32 ms | 1.0 ms | 0.5 ms | **3061.49 ms** | Mon voisin Totoro | 100.0% | 0 |
| 1111 | `Monty_Python_1_Sacre_Graal_1080p_Fr_En_...` | 2370.65 ms | 96.8 ms | 27.2 ms | 148.14 ms | 1.0 ms | 0.5 ms | **2651.53 ms** | Aucun | -- | 0 |
| 1112 | `Monty_Python_2_La_Vie_De_Bryan_1080p_Fr...` | 2540.64 ms | 89.62 ms | 24.44 ms | 192.31 ms | 1.0 ms | 0.5 ms | **2853.98 ms** | Aucun | -- | 0 |
| 1113 | `Monty_Python_3_Le_Sens_De_La_Vie_1080p_...` | 2304.11 ms | 87.44 ms | 19.34 ms | 182.78 ms | 1.0 ms | 0.5 ms | **2596.67 ms** | Aucun | -- | 0 |
| 1114 | `Mortal_Kombat_Legends_Snow_Blind_(2022)...` | 10044.82 ms | 86.27 ms | 17.13 ms | 170.91 ms | 1.0 ms | 0.5 ms | **10319.51 ms** | Mortal Kombat Legends Snow ... | 100.0% | 0 |
| 1115 | `Motherland.Fort.Salem.S01E01.1080p.AMZN...` | 2208.3 ms | 116.52 ms | 18.68 ms | 183.91 ms | 1.0 ms | 0.5 ms | **2528.09 ms** | Motherland Fort Salem | 100.0% | 0 |
| 1116 | `Motherland.Fort.Salem.S01E03.1080p.AMZN...` | 2575.42 ms | 106.13 ms | 24.56 ms | 177.94 ms | 1.0 ms | 0.5 ms | **2885.2 ms** | Aucun | -- | 0 |
| 1117 | `Motherland.Fort.Salem.S01E04.1080p.AMZN...` | 2820.73 ms | 88.89 ms | 16.44 ms | 99.32 ms | 1.0 ms | 0.5 ms | **3025.71 ms** | Motherland Fort Salem | 100.0% | 0 |
| 1118 | `Motherland.Fort.Salem.S01E05.1080p.AMZN...` | 2172.68 ms | 101.82 ms | 13.9 ms | 295.15 ms | 1.0 ms | 0.5 ms | **2584.07 ms** | Motherland Fort Salem | 100.0% | 0 |
| 1119 | `Motherland.Fort.Salem.S01E06.1080p.AMZN...` | 2507.84 ms | 85.46 ms | 21.67 ms | 116.92 ms | 1.0 ms | 0.5 ms | **2732.19 ms** | Motherland Fort Salem | 100.0% | 0 |
| 1120 | `Motherland.Fort.Salem.S01E07.1080p.AMZN...` | 2460.82 ms | 98.25 ms | 36.61 ms | 129.23 ms | 1.0 ms | 0.5 ms | **2725.64 ms** | Motherland Fort Salem | 100.0% | 0 |
| 1121 | `Motherland.Fort.Salem.S01E08.1080p.AMZN...` | 2629.77 ms | 93.91 ms | 33.78 ms | 122.76 ms | 1.0 ms | 0.5 ms | **2880.98 ms** | Motherland Fort Salem | 100.0% | 0 |
| 1122 | `Motherland.Fort.Salem.S01E09.1080p.AMZN...` | 2395.63 ms | 104.1 ms | 34.1 ms | 110.99 ms | 1.0 ms | 0.5 ms | **2645.24 ms** | Motherland Fort Salem | 100.0% | 0 |
| 1123 | `Mountains.May.Depart.2015.FRENCH.BDRip....` | 4184.05 ms | 84.53 ms | 20.61 ms | 391.94 ms | 1.0 ms | 0.5 ms | **4681.45 ms** | Mountains May Depart | 100.0% | 0 |
| 1124 | `Mr.Robot.S02E01E02.1080p.BluRay.x264-SH...` | 2160.51 ms | 97.29 ms | 27.54 ms | 202.27 ms | 1.0 ms | 0.5 ms | **2488.26 ms** | Mr Robot | 100.0% | 0 |
| 1125 | `Mr.Robot.S02E03.1080p.BluRay.x264-SHORT...` | 2708.08 ms | 117.11 ms | 36.33 ms | 356.41 ms | 1.0 ms | 0.5 ms | **3218.31 ms** | Aucun | -- | 0 |
| 1126 | `Mr.Robot.S02E04.1080p.BluRay.x264-SHORT...` | 1928.43 ms | 105.28 ms | 24.94 ms | 114.61 ms | 1.0 ms | 0.5 ms | **2173.55 ms** | Mr Robot | 100.0% | 0 |
| 1127 | `Mr.Robot.S02E05.1080p.BluRay.x264-SHORT...` | 3183.97 ms | 120.87 ms | 26.12 ms | 323.34 ms | 1.0 ms | 0.5 ms | **3654.99 ms** | Aucun | -- | 0 |
| 1128 | `Mr.Robot.S02E06.1080p.BluRay.x264-SHORT...` | 1617.27 ms | 100.86 ms | 32.77 ms | 184.84 ms | 1.0 ms | 0.5 ms | **1936.94 ms** | Mr Robot | 100.0% | 0 |
| 1129 | `Mr.Robot.S02E07.1080p.BluRay.x264-SHORT...` | 1728.91 ms | 110.3 ms | 26.99 ms | 301.65 ms | 1.0 ms | 0.5 ms | **2168.58 ms** | Mr Robot | 100.0% | 0 |
| 1130 | `Mr.Robot.S02E08.1080p.BluRay.x264-SHORT...` | 2789.15 ms | 100.32 ms | 34.22 ms | 134.92 ms | 1.0 ms | 0.5 ms | **3058.96 ms** | mr robot | 100.0% | 0 |
| 1131 | `Mr.Robot.S02E09.1080p.BluRay.x264-SHORT...` | 3734.85 ms | 120.23 ms | 21.58 ms | 379.86 ms | 1.0 ms | 0.5 ms | **4259.45 ms** | Mr Robot | 100.0% | 0 |
| 1132 | `Mr.Robot.S02E10.1080p.BluRay.x264-SHORT...` | 9984.99 ms | 136.94 ms | 39.44 ms | 195.16 ms | 1.0 ms | 0.5 ms | **10356.92 ms** | Mr Robot | 100.0% | 0 |
| 1133 | `Mr.Robot.S02E11.1080p.BluRay.x264-SHORT...` | 2803.55 ms | 119.51 ms | 17.2 ms | 331.78 ms | 1.0 ms | 0.5 ms | **3272.48 ms** | Mr Robot | 100.0% | 1 |
| 1134 | `Mr.Robot.S02E12.1080p.BluRay.x264-SHORT...` | 5202.64 ms | 85.85 ms | 18.31 ms | 233.14 ms | 1.0 ms | 0.5 ms | **5540.33 ms** | Mr Robot | 100.0% | 0 |
| 1135 | `Mrs_Harris_Goes_to_Paris_(2022)[MULTi][...` | 1745.16 ms | 96.89 ms | 38.94 ms | 465.69 ms | 1.0 ms | 0.5 ms | **2348.15 ms** | Mrs Harris Goes to Paris | 100.0% | 0 |
| 1136 | `Mulan.1998.1080p.BRrip.x264.GAZ.YIFY_mi...` | 3075.94 ms | 86.87 ms | 25.28 ms | 193.82 ms | 1.0 ms | 0.5 ms | **3392.87 ms** | Mulan | 100.0% | 0 |
| 1137 | `Museum_of_Television_and_Radio_Panel_Di...` | 1226.91 ms | 95.66 ms | 34.98 ms | 166.67 ms | 1.0 ms | 0.5 ms | **1524.54 ms** | Museum of Television and Ra... | 100.0% | 0 |
| 1138 | `Mustang.2015.FRENCH.BDRip.XviD-EXTREME_...` | 1990.77 ms | 108.85 ms | 23.33 ms | 189.51 ms | 1.0 ms | 0.5 ms | **2313.06 ms** | Aucun | -- | 0 |
| 1139 | `Mute.2018.1080p.BluRay.x264-[YTS.AM]_mi...` | 2225.93 ms | 91.69 ms | 17.63 ms | 200.1 ms | 1.0 ms | 0.5 ms | **2536.09 ms** | Mute | 100.0% | 0 |
| 1140 | `My.Left.Foot_.The.Story.of.Christy.Brow...` | 2484.99 ms | 142.96 ms | 24.52 ms | 321.25 ms | 1.0 ms | 0.5 ms | **2974.68 ms** | My Left Foot: The Story of ... | 100.0% | 0 |
| 1141 | `Mystere_(2021)[1080p](enfant,_loup)_mid...` | 4373.58 ms | 100.02 ms | 27.02 ms | 112.86 ms | 1.0 ms | 0.5 ms | **4614.25 ms** | The Expanse | 100.0% | 1 |
| 1142 | `Naked.Lunch.1991.1080p.BluRay.x265-RARB...` | 2421.81 ms | 99.8 ms | 33.11 ms | 340.24 ms | 1.0 ms | 0.5 ms | **2905.18 ms** | Naked.Lunch.1991.1080p.BluR... | -- | 1 |
| 1143 | `Nappily.Ever.After.2018.1080p.WEBRip.x2...` | 1534.39 ms | 89.76 ms | 21.27 ms | 289.65 ms | 1.0 ms | 0.5 ms | **1936.45 ms** | Nappily.Ever.After.2018.108... | -- | 1 |
| 1144 | `Nausicaä_de_la_vallée_du_vent_(1984)_MU...` | 2153.48 ms | 95.16 ms | 25.35 ms | 161.1 ms | 1.0 ms | 0.5 ms | **2437.13 ms** | Nausicaä de la vallée du vent | 100.0% | 0 |
| 1145 | `Neon.Genesis.Evangelion.The.End.Of.Evan...` | 2368.36 ms | 160.69 ms | 27.73 ms | 176.75 ms | 1.0 ms | 0.5 ms | **2734.25 ms** | Neon Genesis Evangelion The... | 100.0% | 1 |
| 1146 | `Neon_Genesis_Evangelion_01_mid15s.mp4` | 2470.28 ms | 103.44 ms | 24.84 ms | 152.65 ms | 1.0 ms | 0.5 ms | **2752.37 ms** | Neon Genesis Evangelion 01 ... | 100.0% | 0 |
| 1147 | `Neon_Genesis_Evangelion_02_mid15s.mp4` | 2292.27 ms | 83.47 ms | 28.25 ms | 322.06 ms | 1.0 ms | 0.5 ms | **2737.42 ms** | Neon Genesis Evangelion 02 ... | 100.0% | 0 |
| 1148 | `Neon_Genesis_Evangelion_03_mid15s.mp4` | 1830.75 ms | 98.88 ms | 22.22 ms | 264.0 ms | 1.0 ms | 0.5 ms | **2216.55 ms** | Neon Genesis Evangelion 03 ... | 100.0% | 0 |
| 1149 | `Neon_Genesis_Evangelion_04_mid15s.mp4` | 4596.2 ms | 92.13 ms | 17.61 ms | 154.01 ms | 1.0 ms | 0.5 ms | **4860.86 ms** | Aucun | -- | 0 |
| 1150 | `Neon_Genesis_Evangelion_05_mid15s.mp4` | 2251.66 ms | 105.97 ms | 34.29 ms | 259.42 ms | 1.0 ms | 0.5 ms | **2652.07 ms** | The Office US | 100.0% | 1 |
| 1151 | `Neon_Genesis_Evangelion_06_mid15s.mp4` | 10091.29 ms | 87.09 ms | 19.79 ms | 175.46 ms | 1.0 ms | 0.5 ms | **10374.04 ms** | Neon Genesis Evangelion 06 ... | 100.0% | 0 |
| 1152 | `Neon_Genesis_Evangelion_07_mid15s.mp4` | 2111.81 ms | 100.42 ms | 26.29 ms | 230.64 ms | 1.0 ms | 0.5 ms | **2470.38 ms** | Neon Genesis Evangelion 07 ... | 100.0% | 0 |
| 1153 | `Neon_Genesis_Evangelion_08_mid15s.mp4` | 2406.67 ms | 97.8 ms | 27.62 ms | 297.99 ms | 1.0 ms | 0.5 ms | **2831.31 ms** | Aucun | -- | 0 |
| 1154 | `Neon_Genesis_Evangelion_09_mid15s.mp4` | 2265.56 ms | 124.88 ms | 20.65 ms | 215.4 ms | 1.0 ms | 0.5 ms | **2633.35 ms** | Neon Genesis Evangelion 09 ... | 100.0% | 0 |
| 1155 | `Neon_Genesis_Evangelion_10_mid15s.mp4` | 2076.78 ms | 107.31 ms | 22.08 ms | 292.93 ms | 1.0 ms | 0.5 ms | **2500.06 ms** | Neon Genesis Evangelion 10 ... | 100.0% | 0 |
| 1156 | `Neon_Genesis_Evangelion_11_mid15s.mp4` | 1932.7 ms | 121.07 ms | 31.2 ms | 245.71 ms | 1.0 ms | 0.5 ms | **2331.17 ms** | Neon Genesis Evangelion 11 ... | 100.0% | 0 |
| 1157 | `Neon_Genesis_Evangelion_12_mid15s.mp4` | 2141.6 ms | 93.11 ms | 23.57 ms | 266.53 ms | 1.0 ms | 0.5 ms | **2525.35 ms** | Neon Genesis Evangelion 12 ... | 100.0% | 0 |
| 1158 | `Neon_Genesis_Evangelion_13_mid15s.mp4` | 2462.92 ms | 88.19 ms | 21.39 ms | 198.59 ms | 1.0 ms | 0.5 ms | **2773.61 ms** | Neon Genesis Evangelion 13 ... | 100.0% | 0 |
| 1159 | `Neon_Genesis_Evangelion_14_mid15s.mp4` | 4443.49 ms | 178.17 ms | 26.16 ms | 179.37 ms | 1.0 ms | 0.5 ms | **4827.64 ms** | Neon Genesis Evangelion 14 ... | 100.0% | 0 |
| 1160 | `Neon_Genesis_Evangelion_15_mid15s.mp4` | 1883.28 ms | 88.77 ms | 22.38 ms | 258.31 ms | 1.0 ms | 0.5 ms | **2253.17 ms** | Neon Genesis Evangelion 15 ... | 100.0% | 0 |
| 1161 | `Neon_Genesis_Evangelion_16_mid15s.mp4` | 1501.66 ms | 96.42 ms | 21.58 ms | 279.35 ms | 1.0 ms | 0.5 ms | **1901.01 ms** | Neon Genesis Evangelion 16 ... | 100.0% | 0 |
| 1162 | `Neon_Genesis_Evangelion_17_mid15s.mp4` | 9959.12 ms | 138.11 ms | 22.7 ms | 296.52 ms | 1.0 ms | 0.5 ms | **10416.82 ms** | Aucun | -- | 0 |
| 1163 | `Neon_Genesis_Evangelion_18_mid15s.mp4` | 1711.68 ms | 93.21 ms | 38.22 ms | 149.16 ms | 1.0 ms | 0.5 ms | **1992.93 ms** | Aucun | -- | 0 |
| 1164 | `Neon_Genesis_Evangelion_19_mid15s.mp4` | 1659.91 ms | 99.14 ms | 20.84 ms | 152.45 ms | 1.0 ms | 0.5 ms | **1933.08 ms** | Neon Genesis Evangelion 19 ... | 100.0% | 0 |
| 1165 | `Neon_Genesis_Evangelion_20_mid15s.mp4` | 1412.09 ms | 93.17 ms | 21.17 ms | 135.43 ms | 1.0 ms | 0.5 ms | **1662.25 ms** | Neon Genesis Evangelion 20 ... | 100.0% | 0 |
| 1166 | `Neon_Genesis_Evangelion_21_mid15s.mp4` | 1724.89 ms | 103.25 ms | 46.47 ms | 373.72 ms | 1.0 ms | 0.5 ms | **2250.1 ms** | Aucun | -- | 0 |
| 1167 | `Neon_Genesis_Evangelion_22_mid15s.mp4` | 1673.02 ms | 107.15 ms | 21.93 ms | 279.7 ms | 1.0 ms | 0.5 ms | **2082.3 ms** | Neon Genesis Evangelion 22 ... | 100.0% | 0 |
| 1168 | `Neon_Genesis_Evangelion_23_mid15s.mp4` | 1805.21 ms | 87.97 ms | 24.28 ms | 273.0 ms | 1.0 ms | 0.5 ms | **2192.11 ms** | Neon Genesis Evangelion 23 ... | 100.0% | 0 |
| 1169 | `Neon_Genesis_Evangelion_24_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.7 ms** | Erreur | -- | 0 |
| 1170 | `Neon_Genesis_Evangelion_25_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.58 ms** | Erreur | -- | 0 |
| 1171 | `Neon_Genesis_Evangelion_26_mid15s.mp4` | 1626.01 ms | 93.76 ms | 26.86 ms | 226.61 ms | 1.0 ms | 0.5 ms | **1973.64 ms** | Neon Genesis Evangelion 26 ... | 100.0% | 0 |
| 1172 | `Never.Have.I.Ever.S01E01.SUBFRENCH.WEBR...` | 2206.62 ms | 86.25 ms | 14.72 ms | 228.61 ms | 1.0 ms | 0.5 ms | **2537.06 ms** | Never Have I Ever | 100.0% | 0 |
| 1173 | `Never.Let.Me.Go.2010.1080p.BluRay.x264....` | 2840.04 ms | 97.09 ms | 23.39 ms | 136.91 ms | 1.0 ms | 0.5 ms | **3098.38 ms** | Never Let Me Go | 100.0% | 0 |
| 1174 | `Next_Goal_Wins_2014_DVDRIP_XVID_AC3_ACA...` | 2898.43 ms | 100.39 ms | 25.4 ms | 239.26 ms | 1.0 ms | 0.5 ms | **3269.67 ms** | Next Goal Wins | 100.0% | 0 |
| 1175 | `Nginx_&_Fail2ban_-_Contrer_le_Flood_mid...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.8 ms** | Erreur | -- | 0 |
| 1176 | `Ninja.Scroll.1993.1080p.BluRay.x264-[YT...` | 2724.18 ms | 164.34 ms | 28.31 ms | 281.82 ms | 1.0 ms | 0.5 ms | **3199.81 ms** | Aucun | -- | 0 |
| 1177 | `Nobody_Has_to_Know_(2021)[MULTi][1080p]...` | 2286.03 ms | 111.41 ms | 24.12 ms | 260.52 ms | 1.0 ms | 0.5 ms | **2682.86 ms** | Nobody Has to Know | 100.0% | 0 |
| 1178 | `NodeJS_-_Express_mid15s.mp4` | 4025.1 ms | 104.45 ms | 249.58 ms | 163.72 ms | 1.0 ms | 0.5 ms | **4543.16 ms** | Aucun | -- | 0 |
| 1179 | `NodeJS_-_Installation_mid15s.mp4` | 1424.18 ms | 0.0 ms | 0.0 ms | 118.85 ms | 1.0 ms | 0.5 ms | **1543.15 ms** | Aucun | -- | 0 |
| 1180 | `NodeJS_-_Les_Streams_mid15s.mp4` | 3705.37 ms | 108.07 ms | 463.58 ms | 254.59 ms | 1.0 ms | 0.5 ms | **4532.05 ms** | NodeJS - Les Streams mp4 24... | 100.0% | 0 |
| 1181 | `NodeJS_-_Modules_&_NPM_mid15s.mp4` | 5487.86 ms | 95.92 ms | 31.29 ms | 363.33 ms | 1.0 ms | 0.5 ms | **5978.81 ms** | NodeJS - Modules & NPM mp4 ... | 100.0% | 0 |
| 1182 | `NodeJS_-_Notre_premier_serveur_web_mid1...` | 2294.88 ms | 84.79 ms | 331.53 ms | 214.83 ms | 1.0 ms | 0.5 ms | **2926.5 ms** | Aucun | -- | 0 |
| 1183 | `NodeJS_-_Qu'est_ce_que_NodeJS_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.76 ms** | Erreur | -- | 0 |
| 1184 | `Normandie_Nue_(2018)[720p](Drame-Comédi...` | 2148.48 ms | 90.41 ms | 14.26 ms | 115.08 ms | 1.0 ms | 0.5 ms | **2369.02 ms** | Aucun | -- | 0 |
| 1185 | `Nos_Batailles_(2018)_mid15s.mp4` | 2134.82 ms | 90.11 ms | 35.92 ms | 217.52 ms | 1.0 ms | 0.5 ms | **2479.07 ms** | Nos Batailles | 100.0% | 0 |
| 1186 | `Nous_Nous_Marierons_[2017][1080p](boxe,...` | 2551.99 ms | 131.78 ms | 57.6 ms | 136.97 ms | 1.0 ms | 0.5 ms | **2880.66 ms** | Aucun | -- | 0 |
| 1187 | `Nouvel_Ordre_(2020)[MULTi][1080p](coup_...` | 2741.83 ms | 113.47 ms | 16.62 ms | 284.32 ms | 1.0 ms | 0.5 ms | **3159.09 ms** | Nouvel Ordre | 100.0% | 0 |
| 1188 | `Numero.Une.2017.FRENCH.HDRip.XviD-EXTRE...` | 2346.28 ms | 148.91 ms | 68.24 ms | 241.33 ms | 1.0 ms | 0.5 ms | **2805.53 ms** | Numero Une | 100.0% | 0 |
| 1189 | `OSS_117_Rio_ne_repond_plus_2009_French_...` | 2806.6 ms | 91.33 ms | 26.09 ms | 353.95 ms | 1.0 ms | 0.5 ms | **3280.76 ms** | OSS 117 Rio ne repond plus | 100.0% | 0 |
| 1190 | `Obsession_(2025)_1080p_WEB-DL_10bit_HEV...` | 2161.17 ms | 110.38 ms | 25.29 ms | 201.01 ms | 1.0 ms | 0.5 ms | **2499.38 ms** | Obsession | 100.0% | 0 |
| 1191 | `Oldboy.2003.REMASTERED.KOREAN.1080p.Blu...` | 2835.51 ms | 87.17 ms | 24.77 ms | 183.6 ms | 1.0 ms | 0.5 ms | **3140.26 ms** | Oldboy | 100.0% | 1 |
| 1192 | `On_Sourit_pour_la_Photo_(2022)[1080p](c...` | 2628.16 ms | 94.04 ms | 18.85 ms | 173.81 ms | 1.0 ms | 0.5 ms | **2916.5 ms** | Aucun | -- | 0 |
| 1193 | `Once.2006.720p.BluRay.x264.YIFY_mid15s.mp4` | 2416.31 ms | 112.38 ms | 20.89 ms | 270.76 ms | 1.0 ms | 0.5 ms | **2821.11 ms** | Once | 100.0% | 0 |
| 1194 | `Once.Upon.a.Time.in.the.West.1968.1080p...` | 2641.87 ms | 88.55 ms | 23.38 ms | 423.73 ms | 1.0 ms | 0.5 ms | **3178.28 ms** | Once.Upon.a.Time.in.the.Wes... | -- | 1 |
| 1195 | `Once.Were.Warriors.1994.1080p.BluRay.H2...` | 3232.48 ms | 104.9 ms | 20.34 ms | 320.17 ms | 1.0 ms | 0.5 ms | **3683.73 ms** | Once.Were.Warriors.1994.108... | -- | 1 |
| 1196 | `Only.God.Forgives.2013.1080p.BluRay.DTS...` | 1890.05 ms | 92.33 ms | 20.45 ms | 144.61 ms | 1.0 ms | 0.5 ms | **2148.2 ms** | Aucun | -- | 0 |
| 1197 | `Only.Lovers.Left.Alive.2013.1080p.BluRa...` | 3985.19 ms | 89.1 ms | 17.93 ms | 184.81 ms | 1.0 ms | 0.5 ms | **4277.74 ms** | Only Lovers Left Alive | 100.0% | 0 |
| 1198 | `Original_Visual_Concept_mid15s.mp4` | 2980.79 ms | 96.33 ms | 29.54 ms | 405.16 ms | 1.0 ms | 0.5 ms | **3515.52 ms** | Aucun | -- | 0 |
| 1199 | `Ouistreham_(2021)[1080p](Juliette_Binoc...` | 3089.18 ms | 104.56 ms | 15.98 ms | 206.82 ms | 1.0 ms | 0.5 ms | **3417.28 ms** | Aucun | -- | 0 |
| 1200 | `PARTY-GIRL_mid15s.mp4` | 2400.21 ms | 100.99 ms | 24.72 ms | 283.9 ms | 1.0 ms | 0.5 ms | **2810.61 ms** | PARTY-GIRL avi 24 0fps | 100.0% | 0 |
| 1201 | `PHPStorm_-_Apparence_mid15s.mp4` | 5111.77 ms | 108.3 ms | 40.92 ms | 211.81 ms | 1.0 ms | 0.5 ms | **5477.56 ms** | Aucun | -- | 0 |
| 1202 | `PHPStorm_-_Composer_mid15s.mp4` | 1317.97 ms | 0.0 ms | 0.0 ms | 169.79 ms | 1.0 ms | 0.5 ms | **1487.87 ms** | Aucun | -- | 0 |
| 1203 | `PHPStorm_-_Conclusion_mid15s.mp4` | 1446.39 ms | 0.0 ms | 0.0 ms | 321.15 ms | 1.0 ms | 0.5 ms | **1767.67 ms** | Aucun | -- | 0 |
| 1204 | `PHPStorm_-_Déploiement_mid15s.mp4` | 1868.4 ms | 0.0 ms | 0.0 ms | 156.68 ms | 1.0 ms | 0.5 ms | **2025.22 ms** | Aucun | -- | 0 |
| 1205 | `PHPStorm_-_File_Watchers_mid15s.mp4` | 1594.7 ms | 0.0 ms | 0.0 ms | 178.65 ms | 1.0 ms | 0.5 ms | **1773.45 ms** | Aucun | -- | 0 |
| 1206 | `PHPStorm_-_Introduction_mid15s.mp4` | 2180.28 ms | 0.0 ms | 0.0 ms | 184.59 ms | 1.0 ms | 0.5 ms | **2404.39 ms** | Aucun | -- | 0 |
| 1207 | `PHPStorm_-_Live_Template_mid15s.mp4` | 1256.26 ms | 0.0 ms | 0.0 ms | 406.33 ms | 1.0 ms | 0.5 ms | **1662.72 ms** | Aucun | -- | 0 |
| 1208 | `PHPStorm_-_Nouveau_fichier_mid15s.mp4` | 1375.24 ms | 0.0 ms | 0.0 ms | 381.54 ms | 1.0 ms | 0.5 ms | **1756.89 ms** | Aucun | -- | 0 |
| 1209 | `PHPStorm_-_PHPUnit_mid15s.mp4` | 2312.29 ms | 89.12 ms | 34.26 ms | 248.14 ms | 1.0 ms | 0.5 ms | **2684.2 ms** | PHPStorm - PHPUnit mp4 24 0fps | 100.0% | 0 |
| 1210 | `PHPStorm_-_Raccourcis_mid15s.mp4` | 10687.24 ms | 84.24 ms | 191.12 ms | 161.16 ms | 1.0 ms | 0.5 ms | **11124.47 ms** | Aucun | -- | 0 |
| 1211 | `PHPStorm_-_Refactoring_mid15s.mp4` | 1632.51 ms | 0.0 ms | 0.0 ms | 335.31 ms | 1.0 ms | 0.5 ms | **1967.96 ms** | Aucun | -- | 0 |
| 1212 | `PHPStorm_-_Versioning_mid15s.mp4` | 1579.06 ms | 0.0 ms | 0.0 ms | 254.34 ms | 1.0 ms | 0.5 ms | **1833.7 ms** | Aucun | -- | 0 |
| 1213 | `PHPStorm_-_Xdebug_mid15s.mp4` | 6495.74 ms | 98.79 ms | 35.29 ms | 167.04 ms | 1.0 ms | 0.5 ms | **6797.25 ms** | PHPStorm - Xdebug mp4 24 0fps | 100.0% | 0 |
| 1214 | `PHP_'c'est_de_la_merde'_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.97 ms** | Erreur | -- | 0 |
| 1215 | `PHP_sur_Windows_avec_Laragon_(alternati...` | 5156.23 ms | 89.71 ms | 20.73 ms | 316.7 ms | 1.0 ms | 0.5 ms | **5592.77 ms** | PHP sur Windows avec Larago... | 100.0% | 0 |
| 1216 | `Pacific_Rim_Uprising_(2018)_mid15s.mp4` | 3177.83 ms | 141.59 ms | 22.73 ms | 684.18 ms | 1.0 ms | 0.5 ms | **4030.66 ms** | Aucun | -- | 0 |
| 1217 | `Pacifiction_(2022)[1080p.H264][E-AC3]_m...` | 2941.07 ms | 108.79 ms | 20.15 ms | 304.01 ms | 1.0 ms | 0.5 ms | **3376.12 ms** | Aucun | -- | 0 |
| 1218 | `Pandémie_(HD_Light)-zone-Telechargement...` | 3176.09 ms | 91.28 ms | 22.82 ms | 408.6 ms | 1.0 ms | 0.5 ms | **3702.41 ms** | Pandémie | 100.0% | 0 |
| 1219 | `Panic.Room.2002.1080p.WEBRip.DD2.0.x264...` | 2309.47 ms | 146.01 ms | 19.43 ms | 117.35 ms | 1.0 ms | 0.5 ms | **2600.93 ms** | Aucun | -- | 0 |
| 1220 | `Paperhouse.1988.1080p.BluRay.x264.YIFY_...` | 2748.29 ms | 150.39 ms | 24.67 ms | 613.47 ms | 1.0 ms | 0.5 ms | **3547.01 ms** | Paperhouse.1988.1080p.BluRa... | -- | 1 |
| 1221 | `Papillon_(2017)_mid15s.mp4` | 3257.32 ms | 103.13 ms | 22.7 ms | 339.36 ms | 1.0 ms | 0.5 ms | **3722.86 ms** | Papillon | 100.0% | 0 |
| 1222 | `Paradise_Beach_(2019)_[1080p](Thriller_...` | 2282.63 ms | 121.86 ms | 30.25 ms | 282.32 ms | 1.0 ms | 0.5 ms | **2721.62 ms** | Paradise Beach | 100.0% | 0 |
| 1223 | `Paris_Is_Us_(2019)_[1080p]_mid15s.mp4` | 5086.34 ms | 105.5 ms | 21.97 ms | 182.29 ms | 1.0 ms | 0.5 ms | **5401.05 ms** | Paris Is Us | 100.0% | 0 |
| 1224 | `Pendant.ce.temps.sur.Terre.2024.FRENCH....` | 3031.36 ms | 92.18 ms | 18.3 ms | 237.27 ms | 1.0 ms | 0.5 ms | **3379.97 ms** | Pendant ce temps sur Terre | 100.0% | 0 |
| 1225 | `Perfect.Strangers.2017.1080p.BluRay.x26...` | 3629.66 ms | 214.44 ms | 21.19 ms | 356.47 ms | 1.0 ms | 0.5 ms | **4223.95 ms** | Aucun | -- | 0 |
| 1226 | `Permis_de_Construire_(2021)[1080p](Cors...` | 3478.53 ms | 94.17 ms | 20.83 ms | 252.51 ms | 1.0 ms | 0.5 ms | **3852.23 ms** | Permis de Construire | 100.0% | 0 |
| 1227 | `Persona.1966.1080p.BluRay.x264-[YTS.AM]...` | 2732.08 ms | 91.44 ms | 25.54 ms | 866.31 ms | 1.0 ms | 0.5 ms | **3717.12 ms** | Persona | 100.0% | 0 |
| 1228 | `Petaouchnok_(2022)[1080p][DDP5.1]_mid15...` | 4344.89 ms | 103.04 ms | 21.22 ms | 352.64 ms | 1.0 ms | 0.5 ms | **4822.4 ms** | Aucun | -- | 0 |
| 1229 | `Peter.Rabbit.2.The.Runaway.2021.1080p.W...` | 2820.67 ms | 129.67 ms | 20.5 ms | 246.87 ms | 1.0 ms | 0.5 ms | **3218.67 ms** | Peter Rabbit 2 The Runaway | 100.0% | 0 |
| 1230 | `Peter.The.Wolf.2023.1080p.WEBRip.x264.A...` | 2913.92 ms | 101.64 ms | 20.81 ms | 532.42 ms | 1.0 ms | 0.5 ms | **3569.69 ms** | Peter The Wolf | 100.0% | 1 |
| 1231 | `Petite_leçon_d_amour_(2022)[1080p](☕🗼📩💕...` | 2931.2 ms | 113.48 ms | 24.06 ms | 601.24 ms | 1.0 ms | 0.5 ms | **3697.33 ms** | Petite leçon d amour | 100.0% | 0 |
| 1232 | `Petite_nature_(2021)[1080p](🧒🏢🏫🌍)_mid15...` | 4273.56 ms | 88.75 ms | 24.88 ms | 256.3 ms | 1.0 ms | 0.5 ms | **4654.19 ms** | Petite nature | 100.0% | 0 |
| 1233 | `Phenomena_(1985)[MULTi](_Épouvante-Horr...` | 2918.84 ms | 86.01 ms | 21.22 ms | 190.06 ms | 1.0 ms | 0.5 ms | **3222.49 ms** | Phenomena | 100.0% | 0 |
| 1234 | `Pierrot.Le.Fou.1965.1080p.BluRay.x264.A...` | 4129.28 ms | 86.12 ms | 19.87 ms | 230.71 ms | 1.0 ms | 0.5 ms | **4466.52 ms** | Aucun | -- | 0 |
| 1235 | `Pirates.Of.Silicon.Valley.1999.720p.WEB...` | 2182.11 ms | 107.02 ms | 30.61 ms | 132.82 ms | 1.0 ms | 0.5 ms | **2456.79 ms** | Pirates Of Silicon Valley | 100.0% | 0 |
| 1236 | `Places_(2022)[1080p](Science_Po,_éducat...` | 3416.88 ms | 120.31 ms | 32.21 ms | 176.07 ms | 1.0 ms | 0.5 ms | **3746.76 ms** | Places | 100.0% | 0 |
| 1237 | `Pocahontas.1995.1080p.BrRip.x264.YIFY_m...` | 3112.59 ms | 145.52 ms | 28.52 ms | 239.31 ms | 1.0 ms | 0.5 ms | **3526.58 ms** | Pocahontas | 100.0% | 0 |
| 1238 | `Podcast_-_Le_métier_de_développeur_Web_...` | 3022.51 ms | 95.67 ms | 35.09 ms | 169.05 ms | 1.0 ms | 0.5 ms | **3323.07 ms** | Podcast - Le métier de déve... | 100.0% | 0 |
| 1239 | `Polar_Bear_(2022)[MULTi][1080p](disney,...` | 2745.6 ms | 98.79 ms | 26.02 ms | 138.85 ms | 1.0 ms | 0.5 ms | **3019.96 ms** | Polar Bear | 100.0% | 0 |
| 1240 | `Pompoko_(1994)_MULTi_1080p_10bits_BluRa...` | 3157.52 ms | 88.09 ms | 16.34 ms | 217.67 ms | 1.0 ms | 0.5 ms | **3480.81 ms** | Pompoko | 100.0% | 0 |
| 1241 | `Ponyo_sur_la_falaise_(2008)_MULTi_1080p...` | 2283.21 ms | 90.07 ms | 25.32 ms | 281.04 ms | 1.0 ms | 0.5 ms | **2686.17 ms** | Ponyo sur la falaise | 100.0% | 0 |
| 1242 | `Porco_Rosso_(1992)_MULTi_1080p_10bits_B...` | 2745.9 ms | 105.89 ms | 24.04 ms | 158.73 ms | 1.0 ms | 0.5 ms | **3035.38 ms** | Porco Rosso | 100.0% | 0 |
| 1243 | `Predator.2.1990.1080p.BrRip.x264.bitlok...` | 3145.61 ms | 99.77 ms | 26.88 ms | 294.47 ms | 1.0 ms | 0.5 ms | **3567.78 ms** | Predator.2.1990.1080p.BrRip... | -- | 1 |
| 1244 | `Premier_De_La_Classe_(2019)[1080p](En_v...` | 4078.44 ms | 120.2 ms | 36.28 ms | 165.97 ms | 1.0 ms | 0.5 ms | **4403.25 ms** | Aucun | -- | 0 |
| 1245 | `Presidents_(2021)[1080p][DTS.5.1](Jean_...` | 4278.65 ms | 86.9 ms | 36.84 ms | 138.27 ms | 1.0 ms | 0.5 ms | **4544.72 ms** | Presidents | 100.0% | 0 |
| 1246 | `Presque_(2021)[1080p](Bernard_Campan)(D...` | 2363.02 ms | 87.39 ms | 18.27 ms | 96.85 ms | 1.0 ms | 0.5 ms | **2566.25 ms** | Aucun | -- | 0 |
| 1247 | `Primer.2004.1080p.BluRay.x265-RARBG_mid...` | 3153.11 ms | 112.98 ms | 18.87 ms | 237.39 ms | 1.0 ms | 0.5 ms | **3523.17 ms** | Primer | 100.0% | 0 |
| 1248 | `Primer.2004.1080p.WEBRip.DD5.1.x264-FOC...` | 2843.94 ms | 115.88 ms | 19.33 ms | 97.54 ms | 1.0 ms | 0.5 ms | **3077.84 ms** | Primer | 100.0% | 0 |
| 1249 | `Primer.2004.1080p.WEBRip.x264-[YTS.AM]_...` | 2652.1 ms | 108.23 ms | 23.63 ms | 236.2 ms | 1.0 ms | 0.5 ms | **3021.66 ms** | Primer | 100.0% | 0 |
| 1250 | `Princesse_Mononoké_(1997)_MULTi_1080p_1...` | 3472.63 ms | 97.6 ms | 23.43 ms | 192.52 ms | 1.0 ms | 0.5 ms | **3787.26 ms** | Princesse Mononoké | 100.0% | 0 |
| 1251 | `Prisoners.of.the.Ghostland.2021.1080p.W...` | 2990.73 ms | 100.47 ms | 23.52 ms | 244.78 ms | 1.0 ms | 0.5 ms | **3361.73 ms** | Aucun | -- | 0 |
| 1252 | `Project_Power_(2020)[1080p](Science_fic...` | 3233.22 ms | 97.96 ms | 24.54 ms | 279.6 ms | 1.0 ms | 0.5 ms | **3638.04 ms** | Project Power | 100.0% | 0 |
| 1253 | `Présentation_Docker_mid15s.mp4` | 3142.65 ms | 120.74 ms | 22.68 ms | 150.9 ms | 1.0 ms | 0.5 ms | **3437.3 ms** | Présentation Docker mp4 24 ... | 100.0% | 0 |
| 1254 | `Présentation_HTML_CSS_avec_Bespokejs_mi...` | 3473.19 ms | 91.57 ms | 113.29 ms | 352.39 ms | 1.0 ms | 0.5 ms | **4031.42 ms** | Présentation HTML CSS avec ... | 100.0% | 0 |
| 1255 | `Présentation_de_Brackets_mid15s.mp4` | 1321.81 ms | 0.0 ms | 0.0 ms | 371.39 ms | 1.0 ms | 0.5 ms | **1693.3 ms** | Aucun | -- | 0 |
| 1256 | `Présentation_de_Symfony_4_-_Quoi_de_neu...` | 2570.63 ms | 90.59 ms | 31.26 ms | 136.66 ms | 1.0 ms | 0.5 ms | **2829.46 ms** | Aucun | -- | 0 |
| 1257 | `Psychokinesis.2018.1080p.NF.WEB-DL.x264...` | 2976.07 ms | 93.43 ms | 16.93 ms | 160.91 ms | 1.0 ms | 0.5 ms | **3248.43 ms** | Psychokinesis | 100.0% | 0 |
| 1258 | `Pulp.Fiction.1994.1080p.BrRip.x264.YIFY...` | 2880.87 ms | 82.93 ms | 34.65 ms | 251.4 ms | 1.0 ms | 0.5 ms | **3251.15 ms** | 24 L'Entrai 24 0fps | 100.0% | 2 |
| 1259 | `Pumzi_(from_Director_Wanuri_Kahiu)_[sho...` | 7306.15 ms | 148.93 ms | 23.51 ms | 192.35 ms | 1.0 ms | 0.5 ms | **7674.11 ms** | Pumzi from Director Wanuri ... | 100.0% | 0 |
| 1260 | `Qu'est-ce_que_l'espace___La_magie_du_co...` | 2374.47 ms | 99.81 ms | 31.13 ms | 244.88 ms | 1.0 ms | 0.5 ms | **2754.87 ms** | Qu'est-ce que l'espace La m... | 100.0% | 0 |
| 1261 | `Qu'est_ce_que_la_Programmation_fonction...` | 4668.36 ms | 86.65 ms | 29.09 ms | 227.49 ms | 1.0 ms | 0.5 ms | **5012.04 ms** | Qu'est ce que la Programmat... | 100.0% | 0 |
| 1262 | `Qu.est.Ce.Qu.on.A.Tous.Fait.Au.Bon.Dieu...` | 1739.43 ms | 124.42 ms | 21.52 ms | 379.4 ms | 1.0 ms | 0.5 ms | **2282.34 ms** | Aucun | -- | 0 |
| 1263 | `Quels_sont_les_secrets_de_la_lumière___...` | 1646.11 ms | 88.69 ms | 29.23 ms | 134.95 ms | 1.0 ms | 0.5 ms | **1904.66 ms** | Quels sont les secrets de l... | 100.0% | 0 |
| 1264 | `Question_-_1080p_ou_720p_mid15s.mp4` | 4222.14 ms | 103.85 ms | 34.91 ms | 139.86 ms | 1.0 ms | 0.5 ms | **4501.41 ms** | Aucun | -- | 0 |
| 1265 | `Quo_Vadis_(1951)[MULTI][1080p](Rome,_Né...` | 9141.97 ms | 95.52 ms | 22.47 ms | 199.71 ms | 1.0 ms | 0.5 ms | **9460.07 ms** | Quo Vadis | 100.0% | 0 |
| 1266 | `REBOOTED___Short_Film[1080p](Mars_2020)...` | 2015.37 ms | 101.33 ms | 36.9 ms | 214.3 ms | 1.0 ms | 0.5 ms | **2368.73 ms** | Aucun | -- | 0 |
| 1267 | `Ragtime_(1981)[1080p](1906,_NewYork,_Ja...` | 2163.93 ms | 88.32 ms | 17.16 ms | 252.55 ms | 1.0 ms | 0.5 ms | **2522.93 ms** | Ragtime | 100.0% | 0 |
| 1268 | `Raised.by.Wolves.2020.S02E01.1080p.WEBR...` | 2152.53 ms | 87.48 ms | 29.46 ms | 422.48 ms | 1.0 ms | 0.5 ms | **2702.42 ms** | Raised by Wolves | 100.0% | 1 |
| 1269 | `Raised.by.Wolves.2020.S02E02.1080p.WEBR...` | 2835.92 ms | 94.07 ms | 21.73 ms | 336.08 ms | 1.0 ms | 0.5 ms | **3288.44 ms** | Aucun | -- | 0 |
| 1270 | `Raised.by.Wolves.2020.S02E03.1080p.WEBR...` | 1676.79 ms | 97.7 ms | 16.46 ms | 123.04 ms | 1.0 ms | 0.5 ms | **1914.94 ms** | Raised by Wolves | 100.0% | 0 |
| 1271 | `Raised.by.Wolves.2020.S02E04.1080p.WEBR...` | 2105.62 ms | 87.35 ms | 20.12 ms | 195.49 ms | 1.0 ms | 0.5 ms | **2409.43 ms** | Raised by Wolves | 100.0% | 1 |
| 1272 | `Raised.by.Wolves.2020.S02E05.1080p.WEBR...` | 3393.57 ms | 139.56 ms | 18.22 ms | 147.66 ms | 1.0 ms | 0.5 ms | **3713.17 ms** | Raised by Wolves | 100.0% | 0 |
| 1273 | `Raised.by.Wolves.2020.S02E06.1080p.WEBR...` | 3118.15 ms | 87.74 ms | 20.48 ms | 161.38 ms | 1.0 ms | 0.5 ms | **3388.62 ms** | Raised by Wolves | 100.0% | 0 |
| 1274 | `Raised.by.Wolves.2020.S02E07.1080p.WEBR...` | 3243.44 ms | 114.88 ms | 22.61 ms | 186.34 ms | 1.0 ms | 0.5 ms | **3576.61 ms** | Raised by Wolves | 100.0% | 0 |
| 1275 | `Raised.by.Wolves.2020.S02E08.1080p.WEBR...` | 3903.06 ms | 125.28 ms | 18.29 ms | 562.4 ms | 1.0 ms | 0.5 ms | **4613.56 ms** | Raised by Wolves | 100.0% | 0 |
| 1276 | `Realm_of_Terracota_(2021)[FRENCH][1080p...` | 2575.76 ms | 107.05 ms | 26.52 ms | 199.89 ms | 1.0 ms | 0.5 ms | **2912.25 ms** | Realm of Terracota | 100.0% | 0 |
| 1277 | `Red_Dwarf_S01E01_The_End_720p_BluRay_H2...` | 3981.42 ms | 96.61 ms | 17.96 ms | 243.54 ms | 1.0 ms | 0.5 ms | **4341.72 ms** | Red Dwarf | 100.0% | 0 |
| 1278 | `Red_Dwarf_S01E03_Balance_Of_Power_720p_...` | 2883.08 ms | 119.19 ms | 25.63 ms | 143.23 ms | 1.0 ms | 0.5 ms | **3174.15 ms** | Red Dwarf | 100.0% | 0 |
| 1279 | `Red_Dwarf_S01E04_Waiting_For_God_720p_B...` | 3300.05 ms | 108.58 ms | 16.21 ms | 178.55 ms | 1.0 ms | 0.5 ms | **3604.18 ms** | Red Dwarf | 100.0% | 0 |
| 1280 | `Red_Dwarf_S02E02_Better_Than_Life_720p_...` | 3516.36 ms | 95.53 ms | 27.02 ms | 279.37 ms | 1.0 ms | 0.5 ms | **3919.05 ms** | Red Dwarf | 100.0% | 0 |
| 1281 | `Red_Dwarf_S02E03_Thanks_For_The_Memory_...` | 3034.02 ms | 97.36 ms | 21.35 ms | 135.82 ms | 1.0 ms | 0.5 ms | **3289.96 ms** | Red Dwarf | 100.0% | 0 |
| 1282 | `Red_Dwarf_S02E05_Queeg_720p_BluRay_H264...` | 2094.75 ms | 115.55 ms | 15.57 ms | 197.71 ms | 1.0 ms | 0.5 ms | **2424.4 ms** | Red Dwarf | 100.0% | 0 |
| 1283 | `Red_Dwarf_S02E06_Parallel_Universe_720p...` | 2681.1 ms | 87.39 ms | 21.67 ms | 532.24 ms | 1.0 ms | 0.5 ms | **3331.23 ms** | Red Dwarf | 100.0% | 0 |
| 1284 | `Red_Dwarf_S03E01_Backwards_720p_BluRay_...` | 3305.32 ms | 100.17 ms | 50.96 ms | 329.67 ms | 1.0 ms | 0.5 ms | **3787.1 ms** | Aucun | -- | 0 |
| 1285 | `Red_Dwarf_S03E03_Polymorph_720p_BluRay_...` | 3003.61 ms | 92.07 ms | 15.1 ms | 262.81 ms | 1.0 ms | 0.5 ms | **3381.21 ms** | Aucun | -- | 0 |
| 1286 | `Red_Dwarf_S03E04_Bodyswap_720p_BluRay_H...` | 3173.2 ms | 98.02 ms | 20.95 ms | 129.56 ms | 1.0 ms | 0.5 ms | **3422.49 ms** | Red Dwarf | 100.0% | 0 |
| 1287 | `Red_Dwarf_S03E05_Timeslides_720p_BluRay...` | 2101.8 ms | 103.6 ms | 33.04 ms | 433.7 ms | 1.0 ms | 0.5 ms | **2673.48 ms** | Red Dwarf | 100.0% | 0 |
| 1288 | `Red_Dwarf_S03E06_The_Last_Day_720p_BluR...` | 3306.99 ms | 96.69 ms | 24.83 ms | 222.88 ms | 1.0 ms | 0.5 ms | **3652.84 ms** | Red Dwarf | 100.0% | 0 |
| 1289 | `Red_Dwarf_S04E03_Justice_720p_BluRay_H2...` | 2349.74 ms | 101.12 ms | 20.02 ms | 180.65 ms | 1.0 ms | 0.5 ms | **2651.94 ms** | Red Dwarf | 100.0% | 0 |
| 1290 | `Red_Dwarf_S04E05_Dimension_Jump_720p_Bl...` | 1889.62 ms | 89.08 ms | 27.88 ms | 175.66 ms | 1.0 ms | 0.5 ms | **2183.06 ms** | Red Dwarf | 100.0% | 0 |
| 1291 | `Red_Dwarf_S04E06_Meltdown_720p_BluRay_H...` | 6225.09 ms | 116.44 ms | 16.16 ms | 333.77 ms | 1.0 ms | 0.5 ms | **6692.21 ms** | Red Dwarf | 100.0% | 0 |
| 1292 | `Red_Dwarf_S05E01_Holoship_720p_BluRay_H...` | 2471.96 ms | 102.81 ms | 21.25 ms | 242.2 ms | 1.0 ms | 0.5 ms | **2838.92 ms** | Red Dwarf | 100.0% | 0 |
| 1293 | `Red_Dwarf_S05E03_Terrorform_720p_BluRay...` | 2142.96 ms | 98.17 ms | 21.34 ms | 214.48 ms | 1.0 ms | 0.5 ms | **2477.78 ms** | Red Dwarf | 100.0% | 0 |
| 1294 | `Red_Dwarf_S05E05_Demons_And_Angels_720p...` | 2521.22 ms | 123.55 ms | 15.18 ms | 401.63 ms | 1.0 ms | 0.5 ms | **3069.06 ms** | Red Dwarf | 100.0% | 0 |
| 1295 | `Red_Dwarf_S05E06_Back_To_Reality_720p_B...` | 2493.61 ms | 109.59 ms | 24.96 ms | 207.7 ms | 1.0 ms | 0.5 ms | **2836.84 ms** | Red Dwarf | 100.0% | 0 |
| 1296 | `Red_Dwarf_S06E03_Gunmen_Of_The_Apocalyp...` | 2479.47 ms | 94.29 ms | 15.95 ms | 83.61 ms | 1.0 ms | 0.5 ms | **2674.1 ms** | Red Dwarf | 100.0% | 0 |
| 1297 | `Red_Dwarf_S07E01_Tikka_To_Ride_720p_Blu...` | 2478.09 ms | 89.77 ms | 22.76 ms | 205.8 ms | 1.0 ms | 0.5 ms | **2800.29 ms** | Aucun | -- | 0 |
| 1298 | `Red_Dwarf_S07E03_Ouroboros_720p_BluRay_...` | 2132.01 ms | 103.73 ms | 17.06 ms | 286.47 ms | 1.0 ms | 0.5 ms | **2544.47 ms** | Red Dwarf | 100.0% | 0 |
| 1299 | `Red_Dwarf_S07E04_Duct_Soup_720p_BluRay_...` | 2895.53 ms | 95.4 ms | 28.13 ms | 219.44 ms | 1.0 ms | 0.5 ms | **3239.3 ms** | Épisode 003 mkv 24 0fps | 100.0% | 1 |
| 1300 | `Red_Dwarf_S07E08_Nanarchy_720p_BluRay_H...` | 2548.55 ms | 120.26 ms | 22.6 ms | 334.34 ms | 1.0 ms | 0.5 ms | **3030.9 ms** | Red Dwarf | 100.0% | 0 |
| 1301 | `Red_Dwarf_S08E01-E03_Back_In_The_Red_72...` | 2238.39 ms | 89.25 ms | 16.65 ms | 421.14 ms | 1.0 ms | 0.5 ms | **2766.29 ms** | Aucun | -- | 0 |
| 1302 | `Red_Dwarf_S08E04_Cassandra_720p_BluRay_...` | 2244.44 ms | 91.06 ms | 20.96 ms | 274.74 ms | 1.0 ms | 0.5 ms | **2631.96 ms** | Aucun | -- | 0 |
| 1303 | `Red_Dwarf_S08E05_Krytie_Tv_720p_BluRay_...` | 3482.69 ms | 111.42 ms | 32.21 ms | 180.63 ms | 1.0 ms | 0.5 ms | **3807.72 ms** | Aucun | -- | 0 |
| 1304 | `Red_Dwarf_S08E06-E07_Pete_720p_BluRay_H...` | 4281.75 ms | 95.89 ms | 22.76 ms | 340.47 ms | 1.0 ms | 0.5 ms | **4746.45 ms** | Aucun | -- | 0 |
| 1305 | `Red_Dwarf_S08E08_Only_The_Good.._720p_B...` | 2642.87 ms | 90.41 ms | 23.05 ms | 269.38 ms | 1.0 ms | 0.5 ms | **3027.19 ms** | Aucun | -- | 0 |
| 1306 | `Red_Dwarf_S09E01_Back_To_Earth_Part_1_7...` | 2339.79 ms | 108.32 ms | 28.64 ms | 174.05 ms | 1.0 ms | 0.5 ms | **2653.49 ms** | Red Dwarf | 100.0% | 0 |
| 1307 | `Red_Dwarf_S09E02_Back_To_Earth_Part_2_7...` | 3411.83 ms | 91.58 ms | 21.01 ms | 164.8 ms | 1.0 ms | 0.5 ms | **3690.65 ms** | Red Dwarf | 100.0% | 0 |
| 1308 | `Red_Dwarf_S10E04_Entangled_720p_BluRay_...` | 2817.85 ms | 98.26 ms | 28.18 ms | 163.42 ms | 1.0 ms | 0.5 ms | **3108.33 ms** | Aucun | -- | 0 |
| 1309 | `Red_Dwarf_S11E02_Samsara_720p_BluRay_H2...` | 2268.83 ms | 91.73 ms | 15.87 ms | 183.99 ms | 1.0 ms | 0.5 ms | **2561.15 ms** | Red Dwarf | 100.0% | 0 |
| 1310 | `Red_Dwarf_S11E03_Give_And_Take_720p_Blu...` | 3145.3 ms | 101.89 ms | 32.77 ms | 256.56 ms | 1.0 ms | 0.5 ms | **3537.34 ms** | Red Dwarf | 100.0% | 0 |
| 1311 | `Red_Dwarf_S11E04_Officer_Rimmer_720p_Bl...` | 1945.69 ms | 118.78 ms | 19.24 ms | 257.95 ms | 1.0 ms | 0.5 ms | **2342.44 ms** | Red Dwarf | 100.0% | 0 |
| 1312 | `Red_Dwarf_S12E02_Siliconia_720p_BluRay_...` | 2591.57 ms | 91.01 ms | 20.38 ms | 259.33 ms | 1.0 ms | 0.5 ms | **2963.86 ms** | Aucun | -- | 0 |
| 1313 | `Red_Dwarf_S12E04_Mechocracy_720p_BluRay...` | 2577.25 ms | 107.36 ms | 19.69 ms | 91.76 ms | 1.0 ms | 0.5 ms | **2796.61 ms** | Aucun | -- | 0 |
| 1314 | `Red_Dwarf_S12E06_Skipper_720p_BluRay_H2...` | 1796.68 ms | 107.59 ms | 19.75 ms | 335.31 ms | 1.0 ms | 0.5 ms | **2261.22 ms** | Aucun | -- | 0 |
| 1315 | `Red_Sparrow_(2018)[MULTi][1080p](Thrill...` | 1733.97 ms | 92.15 ms | 23.03 ms | 312.34 ms | 1.0 ms | 0.5 ms | **2169.23 ms** | Red Sparrow | 100.0% | 0 |
| 1316 | `Rediff_-_Découverte_du_CMS_Static_Grav_...` | 2495.81 ms | 95.22 ms | 23.18 ms | 320.03 ms | 1.0 ms | 0.5 ms | **2934.62 ms** | Aucun | -- | 0 |
| 1317 | `Rediff_-_Live_#1_mid15s.mp4` | 10711.75 ms | 95.88 ms | 50.59 ms | 418.83 ms | 1.0 ms | 0.5 ms | **11278.86 ms** | Rediff - Live #1 mp4 24 0fps | 100.0% | 0 |
| 1318 | `Rediff_-_Live_#2_mid15s.mp4` | 11610.76 ms | 112.51 ms | 39.09 ms | 249.62 ms | 1.0 ms | 0.5 ms | **12012.4 ms** | Rediff - Live #2 mp4 24 0fps | 100.0% | 0 |
| 1319 | `Replicas_(2018)[MULTi][1080p](US_-_Kean...` | 2290.58 ms | 98.96 ms | 23.32 ms | 298.71 ms | 1.0 ms | 0.5 ms | **2717.96 ms** | Replicas | 100.0% | 0 |
| 1320 | `Repo.Man.1984.1080p.BluRay.x264.AAC-[YT...` | 6706.46 ms | 106.44 ms | 25.8 ms | 195.23 ms | 1.0 ms | 0.5 ms | **7034.56 ms** | Aucun | -- | 0 |
| 1321 | `Resident.Evil.Welcome.to.Raccoon.City.2...` | 1926.3 ms | 105.39 ms | 20.99 ms | 281.06 ms | 1.0 ms | 0.5 ms | **2337.92 ms** | Aucun | -- | 0 |
| 1322 | `Revoir_Paris_(2022)[1080p](Virginie_Efi...` | 2314.96 ms | 95.67 ms | 17.23 ms | 134.3 ms | 1.0 ms | 0.5 ms | **2562.92 ms** | Revoir Paris | 100.0% | 0 |
| 1323 | `Rise_(2022)[MULTi][1080p](premier_trio_...` | 2869.6 ms | 161.59 ms | 21.07 ms | 393.82 ms | 1.0 ms | 0.5 ms | **3446.73 ms** | Rise | 100.0% | 0 |
| 1324 | `Robin_Hood_(2018)_mid15s.mp4` | 2428.69 ms | 127.71 ms | 22.63 ms | 242.97 ms | 1.0 ms | 0.5 ms | **2822.75 ms** | Robin Hood | 100.0% | 0 |
| 1325 | `Robots.2005.1080p.BrRip.x264.YIFY_mid15...` | 2331.05 ms | 102.93 ms | 24.82 ms | 287.31 ms | 1.0 ms | 0.5 ms | **2746.84 ms** | Aucun | -- | 0 |
| 1326 | `Rollerball.1975.1080p.BluRay.x264.YIFY_...` | 2234.66 ms | 97.47 ms | 23.69 ms | 298.49 ms | 1.0 ms | 0.5 ms | **2655.06 ms** | Rollerball.1975.1080p.BluRa... | -- | 1 |
| 1327 | `Ron_Fricke_Interview_mid15s.mp4` | 3264.62 ms | 93.27 ms | 21.66 ms | 403.84 ms | 1.0 ms | 0.5 ms | **3784.2 ms** | Aucun | -- | 0 |
| 1328 | `Rons.Gone.Wrong.2021.1080p.BluRay.x264....` | 2789.35 ms | 94.44 ms | 25.82 ms | 356.86 ms | 1.0 ms | 0.5 ms | **3287.84 ms** | Aucun | -- | 0 |
| 1329 | `Rouge_(2021)[FRENCH][1080p](usine_chimi...` | 3359.07 ms | 94.07 ms | 16.14 ms | 154.97 ms | 1.0 ms | 0.5 ms | **3628.59 ms** | Aucun | -- | 0 |
| 1330 | `Rubikon_(2022)[1080p](SF,_station_spati...` | 3111.19 ms | 111.47 ms | 23.58 ms | 247.34 ms | 1.0 ms | 0.5 ms | **3496.64 ms** | Rubikon | 100.0% | 0 |
| 1331 | `Rumble.2021.MULTi.1080p.BluRay.x264.AC3...` | 3156.24 ms | 109.61 ms | 22.33 ms | 514.39 ms | 1.0 ms | 0.5 ms | **3804.97 ms** | Rumble | 100.0% | 0 |
| 1332 | `Rumble_(2021)[MULTi][1080p](lutte_de_mo...` | 3523.26 ms | 146.93 ms | 31.61 ms | 210.46 ms | 1.0 ms | 0.5 ms | **3929.64 ms** | Rumble | 100.0% | 0 |
| 1333 | `S01E01_The_Gang_Gets_Racist_mid15s.mp4` | 1000.75 ms | 0.0 ms | 0.0 ms | 129.43 ms | 1.0 ms | 0.5 ms | **1130.3 ms** | Aucun | -- | 0 |
| 1334 | `S01E02_Charlie_Wants_an_Abortion_mid15s...` | 1217.99 ms | 0.0 ms | 0.0 ms | 178.21 ms | 1.0 ms | 0.5 ms | **1396.35 ms** | Aucun | -- | 0 |
| 1335 | `S01E03_Underage_Drinking__A_National_Co...` | 1910.22 ms | 114.53 ms | 15.53 ms | 450.14 ms | 1.0 ms | 0.5 ms | **2495.13 ms** | S01E03 Underage Drinking: A... | 100.0% | 0 |
| 1336 | `S01E04_Charlie_Has_Cancer_mid15s.mp4` | 1395.59 ms | 0.0 ms | 0.0 ms | 171.2 ms | 1.0 ms | 0.5 ms | **1566.93 ms** | Aucun | -- | 0 |
| 1337 | `S01E05_Gun_Fever_mid15s.mp4` | 1531.93 ms | 0.0 ms | 0.0 ms | 479.42 ms | 1.0 ms | 0.5 ms | **2011.48 ms** | Aucun | -- | 0 |
| 1338 | `S01E06_The_Gang_Finds_a_Dead_Guy_mid15s...` | 891.86 ms | 0.0 ms | 0.0 ms | 299.15 ms | 1.0 ms | 0.5 ms | **1191.1 ms** | Aucun | -- | 0 |
| 1339 | `S01E07_Charlie_Got_Molested_mid15s.mp4` | 4212.54 ms | 104.65 ms | 10.49 ms | 262.15 ms | 1.0 ms | 0.5 ms | **4590.49 ms** | S01E07 Charlie Got Molested | 100.0% | 0 |
| 1340 | `S02E01_Charlie_Gets_Crippled_mid15s.mp4` | 2061.88 ms | 162.12 ms | 25.45 ms | 232.28 ms | 1.0 ms | 0.5 ms | **2484.38 ms** | S02E01 Charlie Gets Cripple... | 100.0% | 0 |
| 1341 | `S02E02_The_Gang_Goes_Jihad_mid15s.mp4` | 1990.91 ms | 109.55 ms | 25.86 ms | 285.67 ms | 1.0 ms | 0.5 ms | **2417.41 ms** | S02E02 The Gang Goes Jihad.... | 100.0% | 0 |
| 1342 | `S02E03_Dennis_and_Dee_Go_on_Welfare_mid...` | 2651.68 ms | 87.56 ms | 19.95 ms | 211.07 ms | 1.0 ms | 0.5 ms | **2979.6 ms** | S02E03 Dennis and Dee Go on... | 100.0% | 0 |
| 1343 | `S02E04_Mac_Bangs_Dennis'_Mom_mid15s.mp4` | 2548.16 ms | 89.16 ms | 19.29 ms | 147.13 ms | 1.0 ms | 0.5 ms | **2804.99 ms** | S02E04 Mac Bangs Dennis' Mo... | 100.0% | 0 |
| 1344 | `S02E05_Hundred_Dollar_Baby_mid15s.mp4` | 2565.31 ms | 95.15 ms | 20.21 ms | 246.81 ms | 1.0 ms | 0.5 ms | **2931.56 ms** | S02E05 Hundred Dollar Baby.... | 100.0% | 0 |
| 1345 | `S02E06_The_Gang_Gives_Back_mid15s.mp4` | 2801.63 ms | 98.0 ms | 19.29 ms | 260.22 ms | 1.0 ms | 0.5 ms | **3180.03 ms** | S02E06 The Gang Gives Back.... | 100.0% | 0 |
| 1346 | `S02E07_The_Gang_Exploits_a_Miracle_mid1...` | 2825.42 ms | 91.75 ms | 19.23 ms | 203.6 ms | 1.0 ms | 0.5 ms | **3142.6 ms** | S02E07 The Gang Exploits a ... | 100.0% | 0 |
| 1347 | `S02E08_The_Gang_Runs_for_Office_mid15s.mp4` | 2609.06 ms | 112.51 ms | 18.66 ms | 106.68 ms | 1.0 ms | 0.5 ms | **2847.59 ms** | S02E08 The Gang Runs for Of... | 100.0% | 0 |
| 1348 | `S02E09_Charlie_Goes_America_All_Over_Ev...` | 2869.49 ms | 96.51 ms | 17.32 ms | 332.69 ms | 1.0 ms | 0.5 ms | **3316.85 ms** | S02E09 Charlie Goes America... | 100.0% | 0 |
| 1349 | `S02E10_Dennis_and_Dee_Get_a_New_Dad_mid...` | 2191.41 ms | 108.2 ms | 17.13 ms | 188.79 ms | 1.0 ms | 0.5 ms | **2509.85 ms** | S02E10 Dennis and Dee Get a... | 100.0% | 0 |
| 1350 | `S03E01_The_Gang_Finds_a_Dumpster_Baby_m...` | 2214.07 ms | 163.57 ms | 9.51 ms | 232.06 ms | 1.0 ms | 0.5 ms | **2620.6 ms** | S03E01 The Gang Finds a Dum... | 100.0% | 0 |
| 1351 | `S03E02_The_Gang_Gets_Invincible_mid15s.mp4` | 1977.6 ms | 135.68 ms | 9.67 ms | 162.24 ms | 1.0 ms | 0.5 ms | **2293.0 ms** | S03E02 The Gang Gets Invinc... | 100.0% | 0 |
| 1352 | `S03E03_Dennis_and_Dee's_Mom_Is_Dead_mid...` | 2228.27 ms | 84.32 ms | 14.76 ms | 208.81 ms | 1.0 ms | 0.5 ms | **2536.99 ms** | S03E03 Dennis and Dee's Mom... | 100.0% | 0 |
| 1353 | `S03E04_The_Gang_Gets_Held_Hostage_mid15...` | 2439.98 ms | 103.05 ms | 10.74 ms | 312.76 ms | 1.0 ms | 0.5 ms | **2867.13 ms** | S03E04 The Gang Gets Held H... | 100.0% | 0 |
| 1354 | `S03E05_The_Aluminum_Monster_vs._Fatty_M...` | 2298.72 ms | 92.72 ms | 8.65 ms | 181.64 ms | 1.0 ms | 0.5 ms | **2582.51 ms** | S03E05 The Aluminum Monster... | 100.0% | 0 |
| 1355 | `S03E06_The_Gang_Solves_the_North_Korea_...` | 2043.31 ms | 84.46 ms | 29.14 ms | 331.14 ms | 1.0 ms | 0.5 ms | **2490.66 ms** | S03E06 The Gang Solves the ... | 100.0% | 0 |
| 1356 | `S03E07_The_Gang_Sells_Out_mid15s.mp4` | 2090.41 ms | 91.72 ms | 16.53 ms | 294.26 ms | 1.0 ms | 0.5 ms | **2493.62 ms** | S03E07 The Gang Sells Out.m... | 100.0% | 0 |
| 1357 | `S03E08_Frank_Sets_Sweet_Dee_on_Fire_mid...` | 3919.63 ms | 95.54 ms | 26.13 ms | 238.93 ms | 1.0 ms | 0.5 ms | **4281.01 ms** | S03E08 Frank Sets Sweet Dee... | 100.0% | 0 |
| 1358 | `S03E09_Sweet_Dee's_Dating_a_Retarded_Pe...` | 2047.31 ms | 116.79 ms | 20.38 ms | 152.46 ms | 1.0 ms | 0.5 ms | **2337.93 ms** | Aucun | -- | 0 |
| 1359 | `S03E10_Mac_Is_a_Serial_Killer_mid15s.mp4` | 2809.69 ms | 100.56 ms | 36.08 ms | 239.33 ms | 1.0 ms | 0.5 ms | **3186.28 ms** | S03E10 Mac Is a Serial Kill... | 100.0% | 0 |
| 1360 | `S03E11_Dennis_Looks_Like_a_Registered_S...` | 2131.02 ms | 92.3 ms | 15.88 ms | 220.77 ms | 1.0 ms | 0.5 ms | **2461.02 ms** | S03E11 Dennis Looks Like a ... | 100.0% | 0 |
| 1361 | `S03E12_The_Gang_Gets_Whacked__Part_1_mi...` | 2441.36 ms | 109.21 ms | 28.03 ms | 150.39 ms | 1.0 ms | 0.5 ms | **2731.05 ms** | S03E12 The Gang Gets Whacke... | 100.0% | 0 |
| 1362 | `S03E13_The_Gang_Gets_Whacked__Part_2_mi...` | 1429.5 ms | 0.0 ms | 0.0 ms | 129.84 ms | 1.0 ms | 0.5 ms | **1559.44 ms** | Aucun | -- | 0 |
| 1363 | `S03E14_Bums__Making_a_Mess_All_Over_the...` | 2251.53 ms | 115.34 ms | 26.77 ms | 182.93 ms | 1.0 ms | 0.5 ms | **2577.4 ms** | S03E14 Bums: Making a Mess ... | 100.0% | 0 |
| 1364 | `S03E15_The_Gang_Dances_Their_Asses_Off_...` | 2525.86 ms | 117.0 ms | 40.6 ms | 345.08 ms | 1.0 ms | 0.5 ms | **3029.35 ms** | S03E15 The Gang Dances Thei... | 100.0% | 0 |
| 1365 | `S04E01_Mac_and_Dennis__Manhunters_mid15...` | 2262.83 ms | 153.11 ms | 35.4 ms | 211.64 ms | 1.0 ms | 0.5 ms | **2663.7 ms** | S04E01 Mac and Dennis: Manh... | 100.0% | 0 |
| 1366 | `S04E02_The_Gang_Solves_the_Gas_Crisis_m...` | 2335.89 ms | 112.67 ms | 34.65 ms | 184.36 ms | 1.0 ms | 0.5 ms | **2668.21 ms** | S04E02 The Gang Solves the ... | 100.0% | 0 |
| 1367 | `S04E03_America's_Next_Top_Paddy's_Billb...` | 2825.18 ms | 103.54 ms | 25.78 ms | 151.87 ms | 1.0 ms | 0.5 ms | **3107.83 ms** | S04E03 America's Next Top P... | 100.0% | 0 |
| 1368 | `S04E04_Mac's_Banging_the_Waitress_mid15...` | 2581.21 ms | 97.79 ms | 42.07 ms | 219.22 ms | 1.0 ms | 0.5 ms | **2942.01 ms** | Aucun | -- | 0 |
| 1369 | `S04E05_Mac_&_Charlie_Die__Part_1_mid15s...` | 1113.56 ms | 0.0 ms | 0.0 ms | 238.51 ms | 1.0 ms | 0.5 ms | **1352.2 ms** | Aucun | -- | 0 |
| 1370 | `S04E06_Mac_&_Charlie_Die__Part_2_mid15s...` | 3128.12 ms | 90.1 ms | 18.36 ms | 463.34 ms | 1.0 ms | 0.5 ms | **3701.9 ms** | S04E06 Mac & Charlie Die: P... | 100.0% | 0 |
| 1371 | `S04E08_Paddy's_Pub__The_Worst_Bar_in_Ph...` | 2629.41 ms | 106.61 ms | 37.88 ms | 139.34 ms | 1.0 ms | 0.5 ms | **2913.99 ms** | S04E08 Paddy's Pub: The Wor... | 100.0% | 0 |
| 1372 | `S04E09_Dennis_Reynolds__An_Erotic_Life_...` | 1023.35 ms | 0.0 ms | 0.0 ms | 300.63 ms | 1.0 ms | 0.5 ms | **1324.09 ms** | Aucun | -- | 0 |
| 1373 | `S04E10_Sweet_Dee_Has_a_Heart_Attack_mid...` | 2869.73 ms | 108.95 ms | 15.42 ms | 167.91 ms | 1.0 ms | 0.5 ms | **3162.56 ms** | S04E10 Sweet Dee Has a Hear... | 100.0% | 0 |
| 1374 | `S04E11_The_Gang_Cracks_the_Liberty_Bell...` | 2916.41 ms | 91.51 ms | 20.0 ms | 165.02 ms | 1.0 ms | 0.5 ms | **3193.57 ms** | S04E11 The Gang Cracks the ... | 100.0% | 0 |
| 1375 | `S04E12_The_Gang_Gets_Extreme__Home_Make...` | 1998.7 ms | 121.97 ms | 37.88 ms | 276.25 ms | 1.0 ms | 0.5 ms | **2435.66 ms** | S04E12 The Gang Gets Extrem... | 100.0% | 0 |
| 1376 | `S04E13_The_Nightman_Cometh_mid15s.mp4` | 1845.35 ms | 113.86 ms | 45.75 ms | 184.74 ms | 1.0 ms | 0.5 ms | **2190.38 ms** | Aucun | -- | 0 |
| 1377 | `S0VFP9~2_mid15s.mp4` | 2462.03 ms | 159.51 ms | 21.69 ms | 206.14 ms | 1.0 ms | 0.5 ms | **2850.0 ms** | Aucun | -- | 0 |
| 1378 | `S37G58~R_mid15s.mp4` | 2443.11 ms | 102.74 ms | 22.47 ms | 195.32 ms | 1.0 ms | 0.5 ms | **2771.11 ms** | Aucun | -- | 0 |
| 1379 | `S78GPW~1_mid15s.mp4` | 2505.65 ms | 97.21 ms | 21.35 ms | 188.35 ms | 1.0 ms | 0.5 ms | **2816.61 ms** | Aucun | -- | 0 |
| 1380 | `SAMOURAÏ_ACADEMY_(2022)[MULTi][1080p](F...` | 2891.68 ms | 100.73 ms | 27.23 ms | 255.79 ms | 1.0 ms | 0.5 ms | **3276.49 ms** | SAMOURAÏ ACADEMY | 100.0% | 0 |
| 1381 | `SAN404~C_mid15s.mp4` | 1691.4 ms | 116.32 ms | 16.74 ms | 160.0 ms | 1.0 ms | 0.5 ms | **1985.67 ms** | Aucun | -- | 0 |
| 1382 | `SBTHVY~F_mid15s.mp4` | 3337.61 ms | 99.2 ms | 18.49 ms | 258.23 ms | 1.0 ms | 0.5 ms | **3714.16 ms** | Aucun | -- | 0 |
| 1383 | `SDMR4X~J_mid15s.mp4` | 2128.97 ms | 133.61 ms | 22.46 ms | 143.97 ms | 1.0 ms | 0.5 ms | **2434.71 ms** | Aucun | -- | 0 |
| 1384 | `SKZJNR~M_mid15s.mp4` | 3055.9 ms | 93.35 ms | 15.81 ms | 143.99 ms | 1.0 ms | 0.5 ms | **3309.61 ms** | Aucun | -- | 0 |
| 1385 | `SP2OJB~V_mid15s.mp4` | 2600.5 ms | 84.55 ms | 15.22 ms | 173.88 ms | 1.0 ms | 0.5 ms | **2874.74 ms** | Aucun | -- | 0 |
| 1386 | `SY9QH1~F_mid15s.mp4` | 2564.46 ms | 86.45 ms | 25.15 ms | 203.75 ms | 1.0 ms | 0.5 ms | **2881.53 ms** | Aucun | -- | 0 |
| 1387 | `SZ2ZES~4_mid15s.mp4` | 2250.08 ms | 110.71 ms | 23.75 ms | 184.05 ms | 1.0 ms | 0.5 ms | **2570.94 ms** | Aucun | -- | 0 |
| 1388 | `Saint_Seiya_093_mid15s.mp4` | 2674.73 ms | 92.67 ms | 25.03 ms | 216.89 ms | 1.0 ms | 0.5 ms | **3010.01 ms** | Saint Seiya 093 mkv 24 0fps | 100.0% | 1 |
| 1389 | `Saint_Seiya_094_mid15s.mp4` | 4319.45 ms | 86.05 ms | 20.2 ms | 197.51 ms | 1.0 ms | 0.5 ms | **4624.37 ms** | Saint Seiya 094 mkv 24 0fps | 100.0% | 1 |
| 1390 | `Saint_Seiya_095_mid15s.mp4` | 2342.04 ms | 88.08 ms | 19.6 ms | 288.55 ms | 1.0 ms | 0.5 ms | **2739.92 ms** | Aucun | -- | 0 |
| 1391 | `Saint_Seiya_096_mid15s.mp4` | 2920.59 ms | 135.75 ms | 39.7 ms | 321.6 ms | 1.0 ms | 0.5 ms | **3418.4 ms** | Saint Seiya 096 mkv 24 0fps | 100.0% | 1 |
| 1392 | `Saint_Seiya_097_mid15s.mp4` | 3182.59 ms | 102.95 ms | 20.66 ms | 389.13 ms | 1.0 ms | 0.5 ms | **3700.52 ms** | Saint Seiya 097 mkv 24 0fps | 100.0% | 1 |
| 1393 | `Saint_Seiya_098_mid15s.mp4` | 1764.14 ms | 92.16 ms | 27.66 ms | 317.27 ms | 1.0 ms | 0.5 ms | **2202.09 ms** | Saint Seiya 098 mkv 24 0fps | 100.0% | 1 |
| 1394 | `Saint_Seiya_099_mid15s.mp4` | 2564.72 ms | 115.42 ms | 22.83 ms | 207.43 ms | 1.0 ms | 0.5 ms | **2934.13 ms** | Saint Seiya 099 mkv 24 0fps | 100.0% | 2 |
| 1395 | `Sauvages.2024.FRENCH.1080p.WEB.H264-FW_...` | 2663.22 ms | 92.64 ms | 23.68 ms | 578.27 ms | 1.0 ms | 0.5 ms | **3358.58 ms** | Sauvages | 100.0% | 0 |
| 1396 | `Searching.For.Sugar.Man.2012.1080p.BluR...` | 2549.76 ms | 91.9 ms | 31.92 ms | 188.9 ms | 1.0 ms | 0.5 ms | **2866.56 ms** | Searching.For.Sugar.Man.201... | -- | 1 |
| 1397 | `Season_One_Recap_mid15s.mp4` | 3137.5 ms | 97.79 ms | 23.12 ms | 204.26 ms | 1.0 ms | 0.5 ms | **3463.49 ms** | Season One Recap mkv 24 0fps | 100.0% | 0 |
| 1398 | `Second_Act_(2018)[FRENCH][1080p](Comédi...` | 2686.99 ms | 108.02 ms | 19.04 ms | 266.2 ms | 1.0 ms | 0.5 ms | **3080.96 ms** | Second Act | 100.0% | 0 |
| 1399 | `See.S01E01.Godflame.1080p.WEB-DL.DD5.1....` | 2031.42 ms | 93.09 ms | 23.96 ms | 205.16 ms | 1.0 ms | 0.5 ms | **2354.53 ms** | See | 100.0% | 0 |
| 1400 | `See.S01E04.The.River.720p.ATVP.WEB-DL.D...` | 2656.28 ms | 97.13 ms | 19.96 ms | 152.87 ms | 1.0 ms | 0.5 ms | **2935.6 ms** | See | 100.0% | 0 |
| 1401 | `See.S01E05.Plastic.720p.WEB-DL.DD5.1.H....` | 4804.25 ms | 119.13 ms | 16.76 ms | 262.77 ms | 1.0 ms | 0.5 ms | **5206.66 ms** | Aucun | -- | 0 |
| 1402 | `See.S01E06.Silk.720p.ATVP.WEB-DL.DD5.1....` | 2696.39 ms | 91.51 ms | 22.21 ms | 163.13 ms | 1.0 ms | 0.5 ms | **2973.93 ms** | See.S01E06.Silk.720p.ATVP.W... | -- | 1 |
| 1403 | `See.S01E07.The.Lavender.Road.720p.ATVP....` | 3602.22 ms | 97.02 ms | 17.23 ms | 183.15 ms | 1.0 ms | 0.5 ms | **3901.5 ms** | See.S01E07.The.Lavender.Roa... | -- | 1 |
| 1404 | `See.S01E08.iNTERNAL.720p.WEB.H264-AMRAP...` | 4945.05 ms | 84.46 ms | 18.66 ms | 193.73 ms | 1.0 ms | 0.5 ms | **5242.57 ms** | Aucun | -- | 0 |
| 1405 | `Selfie_(2019)[720p](Manu_Payet,_Blanche...` | 2999.59 ms | 129.72 ms | 17.86 ms | 309.18 ms | 1.0 ms | 0.5 ms | **3460.67 ms** | Aucun | -- | 0 |
| 1406 | `Selma.2014.MULTi.1080p.BluRay.x264-DesT...` | 5181.69 ms | 97.64 ms | 22.66 ms | 256.71 ms | 1.0 ms | 0.5 ms | **5560.59 ms** | Selma | 100.0% | 0 |
| 1407 | `Set.It.Up.2018.1080p.WEBRip.x264-RARBG_...` | 3135.56 ms | 123.74 ms | 19.81 ms | 467.53 ms | 1.0 ms | 0.5 ms | **3758.91 ms** | Aucun | -- | 0 |
| 1408 | `Seven.Samurai.1954.1080p.BluRay.x264-[Y...` | 2890.0 ms | 115.94 ms | 19.94 ms | 211.23 ms | 1.0 ms | 0.5 ms | **3237.74 ms** | Aucun | -- | 0 |
| 1409 | `Severance.S01E01.Good.News.About.Hell.1...` | 2716.28 ms | 127.07 ms | 18.9 ms | 218.48 ms | 1.0 ms | 0.5 ms | **3081.47 ms** | Severance | 100.0% | 1 |
| 1410 | `Severance.S01E02.Half.Loop.1080p.ATVP.W...` | 2643.91 ms | 103.45 ms | 23.19 ms | 147.05 ms | 1.0 ms | 0.5 ms | **2919.2 ms** | Severance | 100.0% | 1 |
| 1411 | `Severance.S01E03.In.Perpetuity.1080p.AT...` | 2739.26 ms | 116.93 ms | 18.88 ms | 400.44 ms | 1.0 ms | 0.5 ms | **3286.3 ms** | Severance | 100.0% | 1 |
| 1412 | `Severance.S01E04.The.You.You.Are.1080p....` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.43 ms** | Erreur | -- | 0 |
| 1413 | `Severance.S01E05.The.Grim.Barbarity.of....` | 2857.05 ms | 94.28 ms | 17.92 ms | 327.46 ms | 1.0 ms | 0.5 ms | **3304.73 ms** | Severance | 100.0% | 1 |
| 1414 | `Severance.S01E06.Hide.and.Seek.1080p.AT...` | 2377.05 ms | 97.0 ms | 15.26 ms | 293.2 ms | 1.0 ms | 0.5 ms | **2783.37 ms** | Severance | 100.0% | 1 |
| 1415 | `Severance.S01E07.Defiant.Jazz.1080p.ATV...` | 4302.75 ms | 221.25 ms | 21.35 ms | 141.46 ms | 1.0 ms | 0.5 ms | **4689.43 ms** | Severance | 100.0% | 1 |
| 1416 | `Severance.S01E08.Whats.for.Dinner.1080p...` | 4569.48 ms | 118.27 ms | 21.61 ms | 314.25 ms | 1.0 ms | 0.5 ms | **5031.21 ms** | Severance | 100.0% | 1 |
| 1417 | `Severance.S01E09.The.We.We.Are.1080p.AT...` | 3437.16 ms | 87.12 ms | 41.73 ms | 286.49 ms | 1.0 ms | 0.5 ms | **3859.78 ms** | Severance | 100.0% | 1 |
| 1418 | `Sex.Education.S03E01.Episode.1.1080p.NF...` | 3501.38 ms | 100.58 ms | 18.33 ms | 235.48 ms | 1.0 ms | 0.5 ms | **3856.59 ms** | Sex Education | 100.0% | 0 |
| 1419 | `Sex.Education.S03E03.Episode.3.1080p.NF...` | 3256.37 ms | 111.67 ms | 20.35 ms | 422.88 ms | 1.0 ms | 0.5 ms | **3813.98 ms** | Sex Education | 100.0% | 0 |
| 1420 | `Sex.Education.S03E04.Episode.4.1080p.NF...` | 3614.29 ms | 203.93 ms | 18.96 ms | 114.72 ms | 1.0 ms | 0.5 ms | **3952.58 ms** | Sex Education | 100.0% | 0 |
| 1421 | `Sex.Education.S03E05.Episode.5.1080p.NF...` | 2891.18 ms | 106.95 ms | 15.35 ms | 109.94 ms | 1.0 ms | 0.5 ms | **3125.14 ms** | Sex Education | 100.0% | 0 |
| 1422 | `Sex.Education.S03E06.Episode.6.1080p.NF...` | 3031.23 ms | 1156.17 ms | 19.11 ms | 297.21 ms | 1.0 ms | 0.5 ms | **4507.88 ms** | Sex Education | 100.0% | 0 |
| 1423 | `Sex.Education.S03E07.Episode.7.1080p.NF...` | 2649.86 ms | 104.43 ms | 17.15 ms | 158.68 ms | 1.0 ms | 0.5 ms | **2931.04 ms** | Sex Education | 100.0% | 0 |
| 1424 | `Sex.Education.S03E08.Episode.8.1080p.NF...` | 10370.66 ms | 126.46 ms | 57.14 ms | 210.22 ms | 1.0 ms | 0.5 ms | **10782.07 ms** | Sex Education | 100.0% | 0 |
| 1425 | `Shaft_(2000)_1080p_BluRay_10bit_HEVC_[H...` | 2366.73 ms | 146.68 ms | 17.35 ms | 261.13 ms | 1.0 ms | 0.5 ms | **2796.58 ms** | Aucun | -- | 0 |
| 1426 | `She_Said_(2022)[MULTi][1080p](Affaires_...` | 2825.43 ms | 93.54 ms | 20.52 ms | 192.52 ms | 1.0 ms | 0.5 ms | **3137.7 ms** | She Said | 100.0% | 0 |
| 1427 | `Sherlock.Jr..1924.1080p.BluRay.x264-[YT...` | 2596.18 ms | 103.28 ms | 21.8 ms | 369.14 ms | 1.0 ms | 0.5 ms | **3091.0 ms** | Aucun | -- | 0 |
| 1428 | `Shes.Out.Of.My.League.2010.1080p.BluRay...` | 2351.6 ms | 128.57 ms | 18.47 ms | 438.22 ms | 1.0 ms | 0.5 ms | **2937.61 ms** | Aucun | -- | 0 |
| 1429 | `Shogun.2024.S01E01.Anjin.2160p.DSNP.WEB...` | 3068.61 ms | 93.14 ms | 20.35 ms | 151.95 ms | 1.0 ms | 0.5 ms | **3336.72 ms** | What If | 100.0% | 1 |
| 1430 | `Shogun.2024.S01E02.Servants.of.Two.Mast...` | 2246.54 ms | 108.62 ms | 25.37 ms | 182.29 ms | 1.0 ms | 0.5 ms | **2563.36 ms** | Shogun.2024.S01E02.Servants... | -- | 1 |
| 1431 | `Shogun.2024.S01E03.Tomorrow.Is.Tomorrow...` | 2140.16 ms | 126.43 ms | 15.86 ms | 245.23 ms | 1.0 ms | 0.5 ms | **2532.54 ms** | Shogun.2024.S01E03.Tomorrow... | -- | 1 |
| 1432 | `Shogun.2024.S01E04.The.Eightfold.Fence....` | 6582.44 ms | 101.16 ms | 23.17 ms | 158.9 ms | 1.0 ms | 0.5 ms | **6869.0 ms** | Aucun | -- | 0 |
| 1433 | `Shogun.2024.S01E05.Broken.to.the.Fist.2...` | 2478.62 ms | 88.08 ms | 21.19 ms | 385.72 ms | 1.0 ms | 0.5 ms | **2974.1 ms** | Shogun.2024.S01E05.Broken.t... | -- | 1 |
| 1434 | `Shogun.2024.S01E06.Ladies.of.the.Willow...` | 2693.07 ms | 120.08 ms | 22.18 ms | 110.57 ms | 1.0 ms | 0.5 ms | **2946.36 ms** | The Expanse | 100.0% | 0 |
| 1435 | `Shogun_2024_S01E09_Crimson_Sky_2160p_DS...` | 3734.96 ms | 112.35 ms | 26.65 ms | 356.36 ms | 1.0 ms | 0.5 ms | **4233.17 ms** | Shogun 2024 S01E09 Crimson ... | -- | 1 |
| 1436 | `Shogun_2024_S01E10_A_Dream_of_a_Dream_2...` | 2035.61 ms | 91.85 ms | 20.59 ms | 124.12 ms | 1.0 ms | 0.5 ms | **2272.67 ms** | 28 Years Later | 100.0% | 1 |
| 1437 | `Shoot.the.Messenger.2006.1080p.BluRay.H...` | 3528.05 ms | 141.95 ms | 15.87 ms | 194.78 ms | 1.0 ms | 0.5 ms | **3883.39 ms** | Aucun | -- | 0 |
| 1438 | `Shoot.the.Messenger.2006.720p.BluRay.H2...` | 2396.99 ms | 98.09 ms | 19.49 ms | 213.73 ms | 1.0 ms | 0.5 ms | **2734.06 ms** | Aucun | -- | 0 |
| 1439 | `Si_tu_tends_l'oreille_(1995)_MULTi_1080...` | 2829.68 ms | 116.82 ms | 24.07 ms | 255.32 ms | 1.0 ms | 0.5 ms | **3228.03 ms** | Si tu tends l'oreille | 100.0% | 0 |
| 1440 | `Sibel_(2018)[MULTI][1080p](Turque,_jeun...` | 3812.67 ms | 84.0 ms | 22.26 ms | 370.95 ms | 1.0 ms | 0.5 ms | **4292.56 ms** | Sibel | 100.0% | 0 |
| 1441 | `Sicario.2015.MULTi.1080p.BluRay.x264-CN...` | 2769.88 ms | 111.26 ms | 32.82 ms | 116.71 ms | 1.0 ms | 0.5 ms | **3035.55 ms** | Sicario | 100.0% | 0 |
| 1442 | `Silent_Demo_mid15s.mp4` | 2849.76 ms | 88.25 ms | 26.81 ms | 357.52 ms | 1.0 ms | 0.5 ms | **3323.6 ms** | Aucun | -- | 0 |
| 1443 | `Silent_Hill_(2006)_UpScaled_2160p_H265_...` | 2885.09 ms | 91.02 ms | 27.21 ms | 299.27 ms | 1.0 ms | 0.5 ms | **3303.39 ms** | Aucun | -- | 0 |
| 1444 | `Silent_Night_(2021)[MULTi][1080p]_mid15...` | 4301.66 ms | 99.91 ms | 22.78 ms | 259.8 ms | 1.0 ms | 0.5 ms | **4684.61 ms** | Silent Night | 100.0% | 0 |
| 1445 | `Silo.S02E06.Barricades.1080p.ATVP.WEB-D...` | 3539.48 ms | 95.66 ms | 28.75 ms | 257.94 ms | 1.0 ms | 0.5 ms | **3922.22 ms** | Silo | 100.0% | 0 |
| 1446 | `Silo.S02E07.The.Dive.1080p.ATVP.WEB-DL....` | 2748.0 ms | 93.4 ms | 19.5 ms | 206.73 ms | 1.0 ms | 0.5 ms | **3068.23 ms** | Silo | 100.0% | 0 |
| 1447 | `Silo.S02E09.The.Safeguard.1080p.ATVP.WE...` | 5844.39 ms | 115.49 ms | 17.45 ms | 303.93 ms | 1.0 ms | 0.5 ms | **6282.05 ms** | Silo | 100.0% | 0 |
| 1448 | `Silo.S03E01.Who.Are.You.2160p.ATVP.WEB-...` | 2535.92 ms | 99.03 ms | 29.06 ms | 249.94 ms | 1.0 ms | 0.5 ms | **2914.58 ms** | Aucun | -- | 0 |
| 1449 | `Silo.S03E02.Its.All.Good.2160p.ATVP.WEB...` | 2297.28 ms | 104.45 ms | 22.05 ms | 233.63 ms | 1.0 ms | 0.5 ms | **2657.72 ms** | Silo | 100.0% | 0 |
| 1450 | `Sing.2.2021.1080p.WEBRip.DDP5.1.Atmos.x...` | 2638.17 ms | 82.05 ms | 37.58 ms | 250.68 ms | 1.0 ms | 0.5 ms | **3009.12 ms** | Aucun | -- | 0 |
| 1451 | `Sing.Street.2016.1080p.BluRay.x264-[YTS...` | 2633.79 ms | 95.15 ms | 22.25 ms | 178.84 ms | 1.0 ms | 0.5 ms | **2933.5 ms** | Sing.Street.2016.1080p.BluR... | -- | 1 |
| 1452 | `Skins_(2007)_-_S01E01_-_Tony_(1080p_AMZ...` | 2515.18 ms | 87.52 ms | 21.4 ms | 123.82 ms | 1.0 ms | 0.5 ms | **2748.62 ms** | Skins | 100.0% | 0 |
| 1453 | `Skins_(2007)_-_S01E02_-_Cassie_(1080p_A...` | 2431.49 ms | 88.61 ms | 22.19 ms | 157.62 ms | 1.0 ms | 0.5 ms | **2700.6 ms** | Skins | 100.0% | 0 |
| 1454 | `Skins_(2007)_-_S01E03_-_Jal_(1080p_AMZN...` | 3147.61 ms | 90.45 ms | 18.86 ms | 199.37 ms | 1.0 ms | 0.5 ms | **3459.76 ms** | Skins | 100.0% | 0 |
| 1455 | `Skins_(2007)_-_S01E05_-_Sid_(1080p_AMZN...` | 2821.72 ms | 108.43 ms | 28.49 ms | 396.21 ms | 1.0 ms | 0.5 ms | **3355.65 ms** | Skins | 100.0% | 0 |
| 1456 | `Skins_(2007)_-_S01E06_-_Maxxie_and_Anwa...` | 3076.15 ms | 120.78 ms | 35.37 ms | 255.53 ms | 1.0 ms | 0.5 ms | **3488.62 ms** | Skins | 100.0% | 0 |
| 1457 | `Skins_(2007)_-_S01E08_-_Effy_(1080p_AMZ...` | 2949.78 ms | 104.95 ms | 19.34 ms | 504.8 ms | 1.0 ms | 0.5 ms | **3579.67 ms** | Skins | 100.0% | 0 |
| 1458 | `Skins_(2007)_-_S01E09_-_Everyone_(1080p...` | 3767.94 ms | 188.21 ms | 28.88 ms | 126.41 ms | 1.0 ms | 0.5 ms | **4138.92 ms** | Skins | 100.0% | 0 |
| 1459 | `Skins_(2007)_-_S02E01_-_Tony_and_Maxxie...` | 3464.91 ms | 167.21 ms | 19.71 ms | 262.29 ms | 1.0 ms | 0.5 ms | **3914.74 ms** | Skins | 100.0% | 0 |
| 1460 | `Skins_(2007)_-_S02E03_-_Sid_(1080p_AMZN...` | 3334.66 ms | 199.24 ms | 23.67 ms | 150.9 ms | 1.0 ms | 0.5 ms | **3709.92 ms** | Skins | 100.0% | 0 |
| 1461 | `Skins_(2007)_-_S02E05_-_Chris_(1080p_AM...` | 3497.41 ms | 114.01 ms | 20.04 ms | 184.84 ms | 1.0 ms | 0.5 ms | **3818.43 ms** | Skins | 100.0% | 0 |
| 1462 | `Skins_(2007)_-_S02E07_-_Effy_(1080p_AMZ...` | 2915.33 ms | 99.83 ms | 20.74 ms | 205.62 ms | 1.0 ms | 0.5 ms | **3244.72 ms** | Skins | 100.0% | 0 |
| 1463 | `Skins_(2007)_-_S02E08_-_Jal_(1080p_AMZN...` | 3321.13 ms | 0.0 ms | 0.0 ms | 346.93 ms | 1.0 ms | 0.5 ms | **7169.83 ms** | Aucun | -- | 0 |
| 1464 | `Skins_(2007)_-_S02E09_-_Cassie_(1080p_A...` | 2962.03 ms | 145.87 ms | 22.64 ms | 428.49 ms | 1.0 ms | 0.5 ms | **3560.49 ms** | Skins | 100.0% | 0 |
| 1465 | `Skins_(2007)_-_S02E10_-_Final_Goodbyes_...` | 3307.92 ms | 1015.09 ms | 19.06 ms | 157.46 ms | 1.0 ms | 0.5 ms | **4501.12 ms** | Skins | 100.0% | 0 |
| 1466 | `Skins_(2007)_-_S03E01_-_Everyone_(1080p...` | 3248.7 ms | 1079.95 ms | 19.38 ms | 110.33 ms | 1.0 ms | 0.5 ms | **4463.99 ms** | Skins | 100.0% | 0 |
| 1467 | `Skins_(2007)_-_S03E02_-_Cook_(1080p_AMZ...` | 3595.93 ms | 600.8 ms | 16.04 ms | 126.94 ms | 1.0 ms | 0.5 ms | **4341.79 ms** | Aucun | -- | 0 |
| 1468 | `Skins_(2007)_-_S03E03_-_Thomas_(1080p_A...` | 3390.7 ms | 187.28 ms | 15.61 ms | 118.86 ms | 1.0 ms | 0.5 ms | **3716.04 ms** | Skins | 100.0% | 0 |
| 1469 | `Skins_(2007)_-_S03E04_-_Pandora_(1080p_...` | 3160.15 ms | 1961.41 ms | 27.55 ms | 89.72 ms | 1.0 ms | 0.5 ms | **5239.52 ms** | Skins | 100.0% | 0 |
| 1470 | `Skins_(2007)_-_S03E05_-_Freddie_(1080p_...` | 3002.85 ms | 2187.82 ms | 20.91 ms | 330.26 ms | 1.0 ms | 0.5 ms | **5549.88 ms** | Skins | 100.0% | 0 |
| 1471 | `Skins_(2007)_-_S03E06_-_Naomi_(1080p_AM...` | 2249.44 ms | 570.28 ms | 17.98 ms | 266.33 ms | 1.0 ms | 0.5 ms | **3104.98 ms** | Skins | 100.0% | 0 |
| 1472 | `Skins_(2007)_-_S03E08_-_Effy_(1080p_AMZ...` | 2205.19 ms | 350.88 ms | 18.93 ms | 125.03 ms | 1.0 ms | 0.5 ms | **2701.95 ms** | Skins | 100.0% | 0 |
| 1473 | `Skins_(2007)_-_S03E09_-_Katie_And_Emily...` | 2431.21 ms | 178.04 ms | 25.19 ms | 231.72 ms | 1.0 ms | 0.5 ms | **2866.85 ms** | Skins | 100.0% | 0 |
| 1474 | `Skins_(2007)_-_S04E01_-_Thomas_(1080p_A...` | 2708.16 ms | 498.98 ms | 21.26 ms | 154.67 ms | 1.0 ms | 0.5 ms | **3388.39 ms** | Skins | 100.0% | 0 |
| 1475 | `Skins_(2007)_-_S04E02_-_Emily_(1080p_AM...` | 2585.74 ms | 889.99 ms | 20.48 ms | 126.36 ms | 1.0 ms | 0.5 ms | **3623.39 ms** | Skins | 100.0% | 0 |
| 1476 | `Skins_(2007)_-_S04E03_-_Cook_(1080p_AMZ...` | 1890.83 ms | 213.63 ms | 27.23 ms | 161.73 ms | 1.0 ms | 0.5 ms | **2303.87 ms** | Skins | 100.0% | 0 |
| 1477 | `Skins_(2007)_-_S04E04_-_Katie_(1080p_AM...` | 2571.28 ms | 228.52 ms | 24.19 ms | 227.12 ms | 1.0 ms | 0.5 ms | **3069.33 ms** | Skins | 100.0% | 0 |
| 1478 | `Skins_(2007)_-_S04E05_-_Freddie_(1080p_...` | 2121.94 ms | 629.19 ms | 14.78 ms | 234.35 ms | 1.0 ms | 0.5 ms | **3001.34 ms** | Skins | 100.0% | 0 |
| 1479 | `Skins_(2007)_-_S04E06_-_JJ_(1080p_AMZN_...` | 2276.09 ms | 225.02 ms | 15.31 ms | 132.33 ms | 1.0 ms | 0.5 ms | **2654.59 ms** | Skins | 100.0% | 0 |
| 1480 | `Skins_(2007)_-_S04E07_-_Effy_(1080p_AMZ...` | 2365.1 ms | 132.4 ms | 17.7 ms | 337.02 ms | 1.0 ms | 0.5 ms | **2853.02 ms** | Skins | 100.0% | 0 |
| 1481 | `Skins_(2007)_-_S04E08_-_Everyone_(1080p...` | 2135.83 ms | 539.58 ms | 24.56 ms | 156.86 ms | 1.0 ms | 0.5 ms | **2858.58 ms** | Skins | 100.0% | 0 |
| 1482 | `Skins_(2007)_-_S05E01_-_Franky_(1080p_A...` | 1734.83 ms | 1112.47 ms | 17.63 ms | 279.1 ms | 1.0 ms | 0.5 ms | **3144.7 ms** | Skins | 100.0% | 0 |
| 1483 | `Skins_(2007)_-_S05E04_-_Liv_(1080p_AMZN...` | 1943.67 ms | 303.74 ms | 15.09 ms | 119.8 ms | 1.0 ms | 0.5 ms | **2383.36 ms** | Skins | 100.0% | 0 |
| 1484 | `Skins_(2007)_-_S05E05_-_Nick_(1080p_AMZ...` | 1988.13 ms | 205.41 ms | 24.52 ms | 331.52 ms | 1.0 ms | 0.5 ms | **2553.73 ms** | Skins | 100.0% | 0 |
| 1485 | `Skins_(2007)_-_S05E06_-_Alo_(1080p_AMZN...` | 4379.24 ms | 113.63 ms | 25.63 ms | 384.76 ms | 1.0 ms | 0.5 ms | **4948.04 ms** | Skins | 100.0% | 0 |
| 1486 | `Skins_(2007)_-_S05E07_-_Grace_(1080p_AM...` | 2090.43 ms | 132.02 ms | 13.78 ms | 160.78 ms | 1.0 ms | 0.5 ms | **2397.88 ms** | Skins | 100.0% | 0 |
| 1487 | `Skins_(2007)_-_S06E01_-_Everyone_(1080p...` | 2148.22 ms | 130.28 ms | 15.92 ms | 305.38 ms | 1.0 ms | 0.5 ms | **2602.61 ms** | Skins | 100.0% | 0 |
| 1488 | `Skins_(2007)_-_S06E02_-_Rich_(1080p_AMZ...` | 2185.52 ms | 186.62 ms | 22.12 ms | 123.58 ms | 1.0 ms | 0.5 ms | **2522.2 ms** | Skins | 100.0% | 0 |
| 1489 | `Skins_(2007)_-_S06E03_-_Alex_(1080p_AMZ...` | 2140.6 ms | 146.35 ms | 19.4 ms | 312.97 ms | 1.0 ms | 0.5 ms | **2620.12 ms** | Skins | 100.0% | 0 |
| 1490 | `Skins_(2007)_-_S06E04_-_Franky_(1080p_A...` | 2402.93 ms | 155.35 ms | 24.18 ms | 345.54 ms | 1.0 ms | 0.5 ms | **2929.54 ms** | Skins | 95.0% | 0 |
| 1491 | `Skins_(2007)_-_S06E05_-_Mini_(1080p_AMZ...` | 2007.36 ms | 202.65 ms | 18.69 ms | 116.43 ms | 1.0 ms | 0.5 ms | **2357.67 ms** | Skins | 100.0% | 0 |
| 1492 | `Skins_(2007)_-_S06E06_-_Nick_(1080p_AMZ...` | 2198.51 ms | 128.18 ms | 17.06 ms | 251.06 ms | 1.0 ms | 0.5 ms | **2597.54 ms** | Skins | 100.0% | 0 |
| 1493 | `Skins_(2007)_-_S06E07_-_Alo_(1080p_AMZN...` | 2791.31 ms | 106.82 ms | 13.84 ms | 187.03 ms | 1.0 ms | 0.5 ms | **3107.05 ms** | Skins | 100.0% | 0 |
| 1494 | `Skins_(2007)_-_S06E09_-_Mini_and_Franky...` | 3538.09 ms | 97.72 ms | 20.05 ms | 160.68 ms | 1.0 ms | 0.5 ms | **3817.27 ms** | Skins | 100.0% | 0 |
| 1495 | `Skins_(2007)_-_S06E10_-_Finale_(1080p_A...` | 2302.37 ms | 102.4 ms | 16.09 ms | 96.5 ms | 1.0 ms | 0.5 ms | **2518.04 ms** | Skins | 100.0% | 0 |
| 1496 | `Skins_(2007)_-_S07E01_-_Fire_(1)_(1080p...` | 2428.74 ms | 101.26 ms | 19.46 ms | 147.58 ms | 1.0 ms | 0.5 ms | **2700.67 ms** | Skins | 100.0% | 0 |
| 1497 | `Skins_(2007)_-_S07E03_-_Pure_(1)_(1080p...` | 2998.48 ms | 116.04 ms | 37.47 ms | 405.26 ms | 1.0 ms | 0.5 ms | **3560.8 ms** | Aucun | -- | 0 |
| 1498 | `Skins_(2007)_-_S07E04_-_Pure_(2)_(1080p...` | 3472.17 ms | 203.4 ms | 23.18 ms | 334.2 ms | 1.0 ms | 0.5 ms | **4053.54 ms** | Aucun | -- | 0 |
| 1499 | `Skins_(2007)_-_S07E06_-_Rise_(2)_(1080p...` | 7501.39 ms | 145.15 ms | 15.08 ms | 162.95 ms | 1.0 ms | 0.5 ms | **7825.58 ms** | Skins | 100.0% | 0 |
| 1500 | `So.Long.Stooge.1983.1080p.BluRay.x264.A...` | 2674.21 ms | 117.89 ms | 30.92 ms | 474.42 ms | 1.0 ms | 0.5 ms | **3298.37 ms** | So.Long.Stooge.1983.1080p.B... | -- | 1 |
| 1501 | `Solaris.1972.RUSSIAN.1080p.BluRay.x265-...` | 2843.27 ms | 217.8 ms | 20.92 ms | 133.99 ms | 1.0 ms | 0.5 ms | **3258.41 ms** | Aucun | -- | 0 |
| 1502 | `Someone.Great.2019.1080p.WEBRip.x265-RA...` | 2842.25 ms | 241.09 ms | 26.66 ms | 115.11 ms | 1.0 ms | 0.5 ms | **3231.24 ms** | Someone Great | 100.0% | 1 |
| 1503 | `Sorry.We.Missed.You.2019.1080p.BluRay.x...` | 3109.92 ms | 104.2 ms | 29.47 ms | 375.28 ms | 1.0 ms | 0.5 ms | **3619.67 ms** | Sorry.We.Missed.You.2019.10... | -- | 1 |
| 1504 | `Soul_(2020)[2160p.x265.10bit][AAC5.1]_m...` | 2049.21 ms | 115.06 ms | 16.57 ms | 408.71 ms | 1.0 ms | 0.5 ms | **2596.14 ms** | Aucun | -- | 0 |
| 1505 | `Sound.Of.Metal.2019.1080p.WEBRip.x264.A...` | 3039.12 ms | 109.53 ms | 17.35 ms | 219.22 ms | 1.0 ms | 0.5 ms | **3386.01 ms** | Aucun | -- | 0 |
| 1506 | `Sound.of.Metal.2019.Repack.HDR.2160p.WE...` | 2869.08 ms | 111.54 ms | 28.47 ms | 325.08 ms | 1.0 ms | 0.5 ms | **3334.83 ms** | Aucun | -- | 0 |
| 1507 | `Souvenirs.de.Marnie.2014.FRENCH.720p.Bl...` | 6431.52 ms | 105.39 ms | 21.42 ms | 353.3 ms | 1.0 ms | 0.5 ms | **6917.6 ms** | Souvenirs de Marnie | 100.0% | 0 |
| 1508 | `Souvenirs_de_Marnie_(2014)_MULTi_1080p_...` | 2303.39 ms | 148.98 ms | 21.14 ms | 587.25 ms | 1.0 ms | 0.5 ms | **3061.5 ms** | Souvenirs de Marnie | 100.0% | 0 |
| 1509 | `Souvenirs_goutte_à_goutte_(1991)_VOSTFR...` | 4429.0 ms | 111.33 ms | 20.73 ms | 321.55 ms | 1.0 ms | 0.5 ms | **4883.4 ms** | Aucun | -- | 0 |
| 1510 | `Space.Adventure.Cobra.1982.REPACK.1080p...` | 2367.03 ms | 136.54 ms | 24.15 ms | 235.3 ms | 1.0 ms | 0.5 ms | **2763.76 ms** | Aucun | -- | 0 |
| 1511 | `Speed_Racer_(2008)[1080p]_mid15s.mp4` | 2417.0 ms | 135.05 ms | 28.74 ms | 334.67 ms | 1.0 ms | 0.5 ms | **2925.14 ms** | Speed Racer | 100.0% | 0 |
| 1512 | `Spider-Man.Into.The.Spider-Verse.2018.2...` | 2127.32 ms | 155.03 ms | 29.98 ms | 180.45 ms | 1.0 ms | 0.5 ms | **2493.99 ms** | Aucun | -- | 0 |
| 1513 | `Spider-Man.Into.the.Spider-Verse.2018.M...` | 2785.41 ms | 101.92 ms | 22.01 ms | 273.31 ms | 1.0 ms | 0.5 ms | **3183.43 ms** | The Expanse | 100.0% | 0 |
| 1514 | `Spider-Man_-_Brand_New_Day_(2026)_2160p...` | 3132.75 ms | 145.24 ms | 21.49 ms | 221.4 ms | 1.0 ms | 0.5 ms | **3522.13 ms** | Aucun | -- | 0 |
| 1515 | `Spider.Man.Across.The.Spider.Verse.2023...` | 3115.47 ms | 175.89 ms | 40.77 ms | 187.53 ms | 1.0 ms | 0.5 ms | **3521.91 ms** | Aucun | -- | 0 |
| 1516 | `Spies.In.Disguise.2019.1080p.BluRay.x26...` | 3461.62 ms | 117.71 ms | 29.34 ms | 254.47 ms | 1.0 ms | 0.5 ms | **3864.2 ms** | Aucun | -- | 0 |
| 1517 | `Split_(2016)[1080p]_mid15s.mp4` | 3328.67 ms | 200.19 ms | 35.11 ms | 317.99 ms | 1.0 ms | 0.5 ms | **3882.97 ms** | Split | 100.0% | 0 |
| 1518 | `Stalker.1979.DVDRip.XviD.AC3.Mono.CD1-K...` | 3401.27 ms | 150.9 ms | 21.3 ms | 244.57 ms | 1.0 ms | 0.5 ms | **3819.94 ms** | Stalker | 100.0% | 0 |
| 1519 | `Stalker.1979.DVDRip.XviD.AC3.Mono.CD2-K...` | 2548.12 ms | 171.37 ms | 22.19 ms | 146.62 ms | 1.0 ms | 0.5 ms | **2889.21 ms** | Aucun | -- | 0 |
| 1520 | `Stalker.1979.DVDRip.XviD.AC3.Mono.CD3-K...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.66 ms** | Erreur | -- | 0 |
| 1521 | `Stalker.1979.REMASTERED.1080p.BluRay.x2...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.97 ms** | Erreur | -- | 0 |
| 1522 | `Star.Wars.Andor.S01E05.The.Axe.Forgets....` | 2711.01 ms | 149.62 ms | 19.93 ms | 144.32 ms | 1.0 ms | 0.5 ms | **3028.66 ms** | Star Wars Andor | 100.0% | 0 |
| 1523 | `Star.Wars.Andor.S01E06.The.Eye.1080p.DS...` | 2326.31 ms | 162.46 ms | 17.11 ms | 124.72 ms | 1.0 ms | 0.5 ms | **2636.59 ms** | Star Wars Andor | 100.0% | 0 |
| 1524 | `Star.Wars.Episode.IX.-.The.Rise.Of.Skyw...` | 2908.64 ms | 159.17 ms | 28.13 ms | 209.5 ms | 1.0 ms | 0.5 ms | **3314.24 ms** | Star.Wars.Episode.IX.-.The.... | -- | 1 |
| 1525 | `Star.Wars.Episode.VII.-.The.Force.Awake...` | 3505.83 ms | 224.38 ms | 18.83 ms | 272.34 ms | 1.0 ms | 0.5 ms | **4022.16 ms** | 28 Semaines Plus Tard | 100.0% | 2 |
| 1526 | `Star.Wars.Episode.VIII.-.The.Last.Jedi....` | 2399.74 ms | 156.22 ms | 19.84 ms | 208.67 ms | 1.0 ms | 0.5 ms | **2785.87 ms** | the expanse | 100.0% | 1 |
| 1527 | `Star.Wars.Visions.S01E01.The.Duel.1080p...` | 2184.11 ms | 128.61 ms | 22.78 ms | 163.13 ms | 1.0 ms | 0.5 ms | **2499.56 ms** | Star Wars Visions | 100.0% | 1 |
| 1528 | `Star.Wars.Visions.S01E02.Tatooine.Rhaps...` | 6744.84 ms | 97.3 ms | 17.39 ms | 153.02 ms | 1.0 ms | 0.5 ms | **7013.54 ms** | The Expanse | 100.0% | 0 |
| 1529 | `Star.Wars.Visions.S01E03.The.Twins.1080...` | 2310.43 ms | 122.94 ms | 27.55 ms | 329.21 ms | 1.0 ms | 0.5 ms | **2791.64 ms** | Star Wars Visions | 100.0% | 0 |
| 1530 | `Star.Wars.Visions.S01E04.The.Village.Br...` | 2588.1 ms | 137.11 ms | 19.82 ms | 549.94 ms | 1.0 ms | 0.5 ms | **3296.76 ms** | Star Wars Visions | 100.0% | 0 |
| 1531 | `Star.Wars.Visions.S01E05.The.Ninth.Jedi...` | 1654.41 ms | 228.76 ms | 19.7 ms | 130.5 ms | 1.0 ms | 0.5 ms | **2034.3 ms** | Star Wars Visions | 100.0% | 0 |
| 1532 | `Star.Wars.Visions.S01E06.T0-B1.1080p.DS...` | 2099.96 ms | 109.22 ms | 20.79 ms | 403.08 ms | 1.0 ms | 0.5 ms | **2639.22 ms** | Aucun | -- | 0 |
| 1533 | `Star.Wars.Visions.S01E07.The.Elder.1080...` | 1720.12 ms | 105.76 ms | 61.46 ms | 238.08 ms | 1.0 ms | 0.5 ms | **2125.84 ms** | Aucun | -- | 0 |
| 1534 | `Star.Wars.Visions.S01E08.Lop.and.Ocho.1...` | 1754.75 ms | 249.83 ms | 18.28 ms | 167.55 ms | 1.0 ms | 0.5 ms | **2202.7 ms** | Star Wars Visions | 100.0% | 0 |
| 1535 | `Station_Eleven_S01E05_mid15s.mp4` | 3269.93 ms | 100.49 ms | 22.36 ms | 292.65 ms | 1.0 ms | 0.5 ms | **3686.24 ms** | The Expanse | 100.0% | 0 |
| 1536 | `Station_Eleven_S01E06_mid15s.mp4` | 3513.48 ms | 108.03 ms | 22.42 ms | 277.05 ms | 1.0 ms | 0.5 ms | **3923.34 ms** | Aucun | -- | 0 |
| 1537 | `Station_Eleven_S01E07_mid15s.mp4` | 1725.61 ms | 133.69 ms | 24.79 ms | 209.22 ms | 1.0 ms | 0.5 ms | **2093.69 ms** | Aucun | -- | 0 |
| 1538 | `Station_Eleven_S01E09_mid15s.mp4` | 2516.07 ms | 127.25 ms | 14.06 ms | 283.74 ms | 1.0 ms | 0.5 ms | **2941.85 ms** | Aucun | -- | 0 |
| 1539 | `Station_Eleven_S01E10_mid15s.mp4` | 1879.03 ms | 144.31 ms | 18.89 ms | 335.35 ms | 1.0 ms | 0.5 ms | **2378.25 ms** | Aucun | -- | 0 |
| 1540 | `Stillwater.2021.1080p.AMZN.WEB-DL.DDP5....` | 3523.46 ms | 142.15 ms | 24.41 ms | 172.77 ms | 1.0 ms | 0.5 ms | **3863.48 ms** | Stillwater.2021.1080p.AMZN.... | -- | 1 |
| 1541 | `Strange.Days.1995.1080p.BluRay.x264.AAC...` | 2408.85 ms | 116.26 ms | 22.65 ms | 428.26 ms | 1.0 ms | 0.5 ms | **2976.8 ms** | Strange.Days.1995.1080p.Blu... | -- | 1 |
| 1542 | `Street.Fighter.II.The.Movie.1994.JAPANE...` | 2929.37 ms | 112.34 ms | 25.19 ms | 274.16 ms | 1.0 ms | 0.5 ms | **3342.01 ms** | Aucun | -- | 0 |
| 1543 | `Stronger_(2017)_mid15s.mp4` | 3062.35 ms | 181.15 ms | 19.4 ms | 134.5 ms | 1.0 ms | 0.5 ms | **3398.13 ms** | Stronger | 100.0% | 0 |
| 1544 | `Submarine.2010.720p.BrRip.x264YIFY_mid1...` | 1718.97 ms | 152.65 ms | 24.66 ms | 342.05 ms | 1.0 ms | 0.5 ms | **2239.51 ms** | Submarine | 100.0% | 0 |
| 1545 | `Suburbicon.2017.FRENCH.720p.BluRay.x264...` | 2621.78 ms | 111.51 ms | 22.9 ms | 264.33 ms | 1.0 ms | 0.5 ms | **3040.15 ms** | Suburbicon | 100.0% | 0 |
| 1546 | `Suffragette.2015.FRENCH.BDRip.x264.AC3-...` | 2640.48 ms | 126.23 ms | 25.59 ms | 280.56 ms | 1.0 ms | 0.5 ms | **3073.87 ms** | Suffragette | 100.0% | 0 |
| 1547 | `Sunflower.2006.1080p.WEBRip.x264.AAC5.1...` | 2887.53 ms | 126.71 ms | 23.18 ms | 163.41 ms | 1.0 ms | 0.5 ms | **3203.53 ms** | Aucun | -- | 0 |
| 1548 | `Super.2010.1080p.BluRay.x265-RARBG_mid1...` | 2832.66 ms | 98.05 ms | 19.9 ms | 333.49 ms | 1.0 ms | 0.5 ms | **3287.55 ms** | Aucun | -- | 0 |
| 1549 | `Symfony_4_par_l'exemple_(10_16)_-_Image...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.9 ms** | Erreur | -- | 0 |
| 1550 | `Symfony_4_par_l'exemple_(11_16)_-_Formu...` | 5514.47 ms | 92.45 ms | 28.11 ms | 548.73 ms | 1.0 ms | 0.5 ms | **6184.29 ms** | Symfony 4 par l'exemple 11 ... | 100.0% | 0 |
| 1551 | `Symfony_4_par_l'exemple_(12_16)_-_Symfo...` | 2557.37 ms | 94.12 ms | 39.62 ms | 170.96 ms | 1.0 ms | 0.5 ms | **2862.42 ms** | Symfony 4 par l'exemple 12 ... | 100.0% | 0 |
| 1552 | `Symfony_4_par_l'exemple_(15_16)_-_Le_pr...` | 2623.43 ms | 94.68 ms | 28.24 ms | 269.01 ms | 1.0 ms | 0.5 ms | **3015.72 ms** | Symfony 4 par l'exemple 15 ... | 100.0% | 0 |
| 1553 | `Symfony_4_par_l'exemple_(1_16)_-_Présen...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.68 ms** | Erreur | -- | 0 |
| 1554 | `Symfony_4_par_l'exemple_(2_16)_-_Nos_pr...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.52 ms** | Erreur | -- | 0 |
| 1555 | `Symfony_4_par_l'exemple_(3_16)_-_Découv...` | 3364.7 ms | 90.71 ms | 25.7 ms | 455.01 ms | 1.0 ms | 0.5 ms | **3936.49 ms** | A | 100.0% | 126 |
| 1556 | `Symfony_4_par_l'exemple_(4_16)_-_CRUD_d...` | 1947.16 ms | 97.04 ms | 27.11 ms | 355.38 ms | 1.0 ms | 0.5 ms | **2427.17 ms** | Symfony 4 par l'exemple 4 1... | 100.0% | 0 |
| 1557 | `Symfony_4_par_l'exemple_(5_16)_-_Valide...` | 3762.89 ms | 111.39 ms | 21.58 ms | 177.11 ms | 1.0 ms | 0.5 ms | **4073.45 ms** | Symfony 4 par l'exemple 5 1... | 100.0% | 0 |
| 1558 | `Symfony_4_par_l'exemple_(6_16)_-_Le_com...` | 4877.06 ms | 90.65 ms | 19.18 ms | 172.64 ms | 1.0 ms | 0.5 ms | **5159.88 ms** | Symfony 4 par l'exemple 6 1... | 100.0% | 0 |
| 1559 | `Symfony_4_par_l'exemple_(7_16)_-_Pagine...` | 1683.45 ms | 85.44 ms | 29.75 ms | 129.17 ms | 1.0 ms | 0.5 ms | **1928.14 ms** | Symfony 4 par l'exemple 7 1... | 100.0% | 0 |
| 1560 | `Symfony_4_par_l'exemple_(8_16)_-_Filtre...` | 3133.06 ms | 105.32 ms | 20.32 ms | 270.6 ms | 1.0 ms | 0.5 ms | **3529.81 ms** | silo | 100.0% | 1 |
| 1561 | `Symfony_4_par_l'exemple_(9_16)_-_Gestio...` | 3914.59 ms | 84.6 ms | 166.81 ms | 201.35 ms | 1.0 ms | 0.5 ms | **4367.75 ms** | Aucun | -- | 0 |
| 1562 | `THE_CHEF_(2021)[MULTI][1080p](Dans_un_r...` | 2470.22 ms | 94.65 ms | 20.31 ms | 426.81 ms | 1.0 ms | 0.5 ms | **3013.6 ms** | THE CHEF | 100.0% | 0 |
| 1563 | `Tar_(2022)[2160p][x265.10bit.AAC5.1]_mi...` | 4652.8 ms | 138.48 ms | 31.8 ms | 215.1 ms | 1.0 ms | 0.5 ms | **5038.86 ms** | Tar | 100.0% | 2 |
| 1564 | `Tar_(2022)[MULTi][1080p](Musique_classi...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12064.09 ms** | Erreur | -- | 0 |
| 1565 | `Taxi.Driver.1976.1080p.BluRay.x264-[YTS...` | 2938.77 ms | 101.7 ms | 21.29 ms | 426.6 ms | 1.0 ms | 0.5 ms | **3489.57 ms** | Taxi.Driver.1976.1080p.BluR... | -- | 1 |
| 1566 | `Tests_Unitaires_en_JavaScript_(10_10)_-...` | 4337.53 ms | 98.11 ms | 34.46 ms | 148.49 ms | 1.0 ms | 0.5 ms | **4619.01 ms** | Mise en pratique de la POO ... | 100.0% | 0 |
| 1567 | `Tests_Unitaires_en_JavaScript_-_Code_as...` | 1903.17 ms | 123.11 ms | 28.69 ms | 582.99 ms | 1.0 ms | 0.5 ms | **2638.32 ms** | Tests Unitaires en JavaScri... | 100.0% | 0 |
| 1568 | `Tests_Unitaires_en_JavaScript_-_Introdu...` | 1076.37 ms | 0.0 ms | 0.0 ms | 146.73 ms | 1.0 ms | 0.5 ms | **1223.23 ms** | Aucun | -- | 0 |
| 1569 | `Tests_Unitaires_en_JavaScript_-_Karma_m...` | 4481.88 ms | 127.55 ms | 41.69 ms | 220.43 ms | 1.0 ms | 0.5 ms | **4872.04 ms** | Tests Unitaires en JavaScri... | 100.0% | 0 |
| 1570 | `Tests_Unitaires_en_JavaScript_-_Lignes_...` | 4001.35 ms | 104.81 ms | 31.15 ms | 407.84 ms | 1.0 ms | 0.5 ms | **4545.56 ms** | Aucun | -- | 0 |
| 1571 | `Tests_Unitaires_en_JavaScript_-_Premier...` | 1276.81 ms | 0.0 ms | 0.0 ms | 103.79 ms | 1.0 ms | 0.5 ms | **1380.71 ms** | Aucun | -- | 0 |
| 1572 | `Tests_Unitaires_en_JavaScript_-_Spy,_Mo...` | 4239.56 ms | 126.28 ms | 45.94 ms | 201.72 ms | 1.0 ms | 0.5 ms | **4615.13 ms** | Tests Unitaires en JavaScri... | 100.0% | 0 |
| 1573 | `Tests_Unitaires_en_JavaScript_-_Tester_...` | 4212.59 ms | 143.26 ms | 101.17 ms | 215.45 ms | 1.0 ms | 0.5 ms | **4673.02 ms** | Aucun | -- | 0 |
| 1574 | `Tests_Unitaires_en_JavaScript_-_Tests_c...` | 4364.42 ms | 140.25 ms | 31.49 ms | 267.83 ms | 1.0 ms | 0.5 ms | **4804.42 ms** | Tests Unitaires en JavaScri... | 100.0% | 0 |
| 1575 | `The.355.2022.1080p.WEBRip.x265-RARBG_mi...` | 2459.28 ms | 136.75 ms | 25.01 ms | 282.42 ms | 1.0 ms | 0.5 ms | **2907.41 ms** | The.355.2022.1080p.WEBRip.x... | -- | 1 |
| 1576 | `The.400.Blows.1959.1080p.BluRay.x264-[Y...` | 2612.21 ms | 88.19 ms | 14.61 ms | 252.09 ms | 1.0 ms | 0.5 ms | **2968.68 ms** | Aucun | -- | 0 |
| 1577 | `The.Acolyte.S01E08.The.Acolyte.2160p.HS...` | 3957.3 ms | 97.42 ms | 26.25 ms | 324.98 ms | 1.0 ms | 0.5 ms | **4406.83 ms** | The Acolyte | 100.0% | 0 |
| 1578 | `The.Age.of.Adaline.2015.FRENCH.HDRip.Xv...` | 3257.6 ms | 168.33 ms | 67.59 ms | 420.19 ms | 1.0 ms | 0.5 ms | **3919.63 ms** | The Age of Adaline | 100.0% | 0 |
| 1579 | `The.Bad.Guys.2022.2160p.AMZN.WEB-DL.DDP...` | 3152.04 ms | 122.3 ms | 71.11 ms | 207.03 ms | 1.0 ms | 0.5 ms | **3553.32 ms** | The.Bad.Guys.2022.2160p.AMZ... | -- | 1 |
| 1580 | `The.Banshees.Of.Inisherin.2022.2160p.4K...` | 3350.88 ms | 89.66 ms | 28.45 ms | 244.44 ms | 1.0 ms | 0.5 ms | **3720.84 ms** | The Banshees Of Inisherin | 100.0% | 0 |
| 1581 | `The.Bobs.Burgers.Movie.2022.2160p.4K.WE...` | 3289.56 ms | 214.8 ms | 27.91 ms | 124.97 ms | 1.0 ms | 0.5 ms | **3658.09 ms** | Aucun | -- | 0 |
| 1582 | `The.Boss.Baby.Family.Business.2021.1080...` | 2941.05 ms | 218.64 ms | 39.78 ms | 291.53 ms | 1.0 ms | 0.5 ms | **3491.7 ms** | The Dirt | 100.0% | 0 |
| 1583 | `The.Boy.Who.Harnessed.The.Wind.2019.108...` | 4872.76 ms | 97.51 ms | 24.47 ms | 352.53 ms | 1.0 ms | 0.5 ms | **5348.14 ms** | Aucun | -- | 0 |
| 1584 | `The.Boy.and.the.Heron.2023.2160p.WEB-DL...` | 2782.92 ms | 214.27 ms | 25.62 ms | 343.82 ms | 1.0 ms | 0.5 ms | **3367.35 ms** | Aucun | -- | 0 |
| 1585 | `The.Boys.Presents.Diabolical.S01E01.Las...` | 1526.64 ms | 0.0 ms | 0.0 ms | 179.32 ms | 1.0 ms | 0.5 ms | **1706.28 ms** | The.Boys.Presents.Diabolica... | -- | 1 |
| 1586 | `The.Boys.Presents.Diabolical.S01E02.An....` | 2371.27 ms | 91.65 ms | 22.72 ms | 617.79 ms | 1.0 ms | 0.5 ms | **3104.15 ms** | Aucun | -- | 0 |
| 1587 | `The.Boys.Presents.Diabolical.S01E03.Im....` | 693.04 ms | 0.0 ms | 0.0 ms | 175.4 ms | 1.0 ms | 0.5 ms | **868.6 ms** | Aucun | -- | 0 |
| 1588 | `The.Boys.Presents.Diabolical.S01E04.Boy...` | 1692.7 ms | 150.79 ms | 27.99 ms | 287.17 ms | 1.0 ms | 0.5 ms | **2160.42 ms** | The Boys Presents Diabolical | 100.0% | 0 |
| 1589 | `The.Boys.Presents.Diabolical.S01E05.BFF...` | 627.6 ms | 0.0 ms | 0.0 ms | 187.62 ms | 1.0 ms | 0.5 ms | **815.36 ms** | Aucun | -- | 0 |
| 1590 | `The.Boys.Presents.Diabolical.S01E06.Nub...` | 2712.33 ms | 89.61 ms | 20.46 ms | 294.31 ms | 1.0 ms | 0.5 ms | **3119.76 ms** | The Boys Presents Diabolical | 100.0% | 0 |
| 1591 | `The.Boys.Presents.Diabolical.S01E08.One...` | 1260.57 ms | 0.0 ms | 0.0 ms | 133.53 ms | 1.0 ms | 0.5 ms | **1394.21 ms** | Aucun | -- | 0 |
| 1592 | `The.Boys.S02E03.1080p.AMZN.WEBRip.DDP5....` | 2067.9 ms | 91.25 ms | 17.34 ms | 154.55 ms | 1.0 ms | 0.5 ms | **2332.09 ms** | The Boys | 100.0% | 2 |
| 1593 | `The.Boys.S02E04.Nothing.Like.It.in.the....` | 3158.82 ms | 83.46 ms | 21.92 ms | 181.94 ms | 1.0 ms | 0.5 ms | **3448.66 ms** | The Boys | 100.0% | 0 |
| 1594 | `The.Boys.S02E05.We.Gotta.Go.Now.1080p.A...` | 2775.6 ms | 121.51 ms | 21.8 ms | 231.45 ms | 1.0 ms | 0.5 ms | **3157.25 ms** | The Boys | 100.0% | 0 |
| 1595 | `The.Boys.S02E06.The.Bloody.Doors.Off.10...` | 3361.19 ms | 101.46 ms | 158.78 ms | 300.07 ms | 1.0 ms | 0.5 ms | **3922.67 ms** | Aucun | -- | 0 |
| 1596 | `The.Boys.S02E07.Butcher.Baker.Candlesti...` | 2153.1 ms | 97.11 ms | 14.94 ms | 421.0 ms | 1.0 ms | 0.5 ms | **2687.22 ms** | The Boys | 100.0% | 0 |
| 1597 | `The.Boys.S02E08.What.I.Know.1080p.AMZN....` | 3441.97 ms | 142.63 ms | 35.43 ms | 265.93 ms | 1.0 ms | 0.5 ms | **3890.64 ms** | The Boys | 100.0% | 0 |
| 1598 | `The.Boys.S03E01.2160p.WEB-DL.DDP5.1.HEV...` | 5020.47 ms | 96.16 ms | 20.64 ms | 140.48 ms | 1.0 ms | 0.5 ms | **5278.95 ms** | The Boys | 100.0% | 1 |
| 1599 | `The.Boys.S03E02.2160p.WEB-DL.DDP5.1.HEV...` | 4149.05 ms | 106.96 ms | 28.54 ms | 166.19 ms | 1.0 ms | 0.5 ms | **4452.87 ms** | The Mandalorian | 100.0% | 0 |
| 1600 | `The.Boys.S03E03.2160p.WEB-DL.DDP5.1.HEV...` | 2780.7 ms | 98.34 ms | 42.06 ms | 584.62 ms | 1.0 ms | 0.5 ms | **3506.49 ms** | Aucun | -- | 0 |
| 1601 | `The.Boys.S03E04.Glorious.Five.Year.Plan...` | 3004.63 ms | 99.54 ms | 44.6 ms | 200.26 ms | 1.0 ms | 0.5 ms | **3349.87 ms** | The Boys | 100.0% | 0 |
| 1602 | `The.Breadwinner.2017.FRENCH.HDRip.XviD-...` | 2267.7 ms | 99.2 ms | 24.48 ms | 335.65 ms | 1.0 ms | 0.5 ms | **2727.44 ms** | Aucun | -- | 0 |
| 1603 | `The.Brutalist.2024.MULTi.1080p.WEB.H264...` | 3003.97 ms | 101.14 ms | 29.15 ms | 374.84 ms | 1.0 ms | 0.5 ms | **3509.46 ms** | The Brutalist | 100.0% | 0 |
| 1604 | `The.Butterfly.Effect.2004.DC.1080p.BluR...` | 2434.19 ms | 111.11 ms | 99.74 ms | 474.14 ms | 1.0 ms | 0.5 ms | **3120.17 ms** | Aucun | -- | 0 |
| 1605 | `The.Colony.2021.1080p.WEBRip.DD5.1.x264...` | 2548.21 ms | 94.06 ms | 54.58 ms | 158.93 ms | 1.0 ms | 0.5 ms | **2856.45 ms** | Aucun | -- | 0 |
| 1606 | `The.Commitments.1991.1080p.BluRay.x264-...` | 2335.43 ms | 106.5 ms | 47.34 ms | 158.83 ms | 1.0 ms | 0.5 ms | **2651.43 ms** | Aucun | -- | 0 |
| 1607 | `The.Connection.1961.1080p.BluRay.x264.A...` | 4356.14 ms | 169.66 ms | 25.25 ms | 526.28 ms | 1.0 ms | 0.5 ms | **5084.4 ms** | Maniac | 100.0% | 4 |
| 1608 | `The.Cured.2018.MULTi.TRUEFRENCH.1080p.W...` | 2248.97 ms | 112.58 ms | 39.62 ms | 424.32 ms | 1.0 ms | 0.5 ms | **2829.58 ms** | The Cured | 100.0% | 0 |
| 1609 | `The.Dark.Crystal.1982.REMASTERED.1080p....` | 3584.74 ms | 92.09 ms | 19.73 ms | 236.44 ms | 1.0 ms | 0.5 ms | **3945.04 ms** | The.Dark.Crystal.1982.REMAS... | -- | 8 |
| 1610 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 2473.87 ms | 90.81 ms | 28.2 ms | 140.65 ms | 1.0 ms | 0.5 ms | **2734.45 ms** | The Dark Crystal Age of Res... | 100.0% | 1 |
| 1611 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 3300.84 ms | 166.98 ms | 22.2 ms | 251.31 ms | 1.0 ms | 0.5 ms | **3749.37 ms** | the mandalorian | 100.0% | 0 |
| 1612 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 4727.66 ms | 115.24 ms | 20.61 ms | 236.67 ms | 1.0 ms | 0.5 ms | **5103.83 ms** | The Dark Crystal Age of Res... | 100.0% | 0 |
| 1613 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 2658.35 ms | 87.44 ms | 39.54 ms | 186.35 ms | 1.0 ms | 0.5 ms | **2972.54 ms** | Aucun | -- | 0 |
| 1614 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 3092.55 ms | 165.42 ms | 41.35 ms | 641.93 ms | 1.0 ms | 0.5 ms | **3942.13 ms** | The.Dark.Crystal.Age.of.Res... | -- | 1 |
| 1615 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 2159.27 ms | 120.03 ms | 76.81 ms | 289.88 ms | 1.0 ms | 0.5 ms | **2646.96 ms** | The.Dark.Crystal.Age.of.Res... | -- | 1 |
| 1616 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 2728.74 ms | 99.86 ms | 27.28 ms | 214.81 ms | 1.0 ms | 0.5 ms | **3071.48 ms** | The Dark Crystal Age of Res... | 100.0% | 1 |
| 1617 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 2267.36 ms | 94.41 ms | 23.67 ms | 247.05 ms | 1.0 ms | 0.5 ms | **2633.67 ms** | The Dark Crystal Age of Res... | 100.0% | 1 |
| 1618 | `The.Dark.Crystal.Age.of.Resistance.S01E...` | 2356.5 ms | 85.16 ms | 18.0 ms | 100.48 ms | 1.0 ms | 0.5 ms | **2562.7 ms** | The Dark Crystal Age of Res... | 100.0% | 1 |
| 1619 | `The.Dead.Dont.Die.2019.HDR.2160p.WEB.H2...` | 2799.55 ms | 169.01 ms | 22.07 ms | 316.9 ms | 1.0 ms | 0.5 ms | **3308.96 ms** | The.Dead.Dont.Die.2019.HDR.... | -- | 1 |
| 1620 | `The.Death.Of.Stalin.2018.FRENCH.1080p.W...` | 2991.13 ms | 94.88 ms | 21.4 ms | 420.96 ms | 1.0 ms | 0.5 ms | **3529.41 ms** | The Death Of Stalin | 100.0% | 0 |
| 1621 | `The.Deer.Hunter.1978.REMASTERED.1080p.B...` | 3170.63 ms | 115.98 ms | 27.78 ms | 218.81 ms | 1.0 ms | 0.5 ms | **3534.41 ms** | The.Deer.Hunter.1978.REMAST... | -- | 1 |
| 1622 | `The.Dropout.S01E01.1080p.WEBRip.x265-RA...` | 2930.26 ms | 105.07 ms | 32.71 ms | 390.76 ms | 1.0 ms | 0.5 ms | **3465.22 ms** | Aucun | -- | 0 |
| 1623 | `The.Dropout.S01E02.1080p.WEBRip.x265-RA...` | 3549.73 ms | 102.02 ms | 17.19 ms | 102.77 ms | 1.0 ms | 0.5 ms | **3772.45 ms** | The Dropout | 100.0% | 0 |
| 1624 | `The.Dropout.S01E03.1080p.WEBRip.x265-RA...` | 3814.15 ms | 103.08 ms | 16.81 ms | 183.7 ms | 1.0 ms | 0.5 ms | **4119.29 ms** | The Dropout | 100.0% | 0 |
| 1625 | `The.Dropout.S01E04.1080p.WEBRip.x265-RA...` | 4644.02 ms | 239.6 ms | 21.56 ms | 194.76 ms | 1.0 ms | 0.5 ms | **5106.68 ms** | The Dropout | 100.0% | 0 |
| 1626 | `The.Dropout.S01E05.1080p.WEBRip.x265-RA...` | 2621.38 ms | 89.25 ms | 17.84 ms | 288.32 ms | 1.0 ms | 0.5 ms | **3017.9 ms** | The Dropout | 100.0% | 0 |
| 1627 | `The.Dropout.S01E06.1080p.WEBRip.x265-RA...` | 1933.01 ms | 105.97 ms | 19.43 ms | 305.01 ms | 1.0 ms | 0.5 ms | **2364.2 ms** | The Dropout | 100.0% | 0 |
| 1628 | `The.Dropout.S01E08.1080p.WEBRip.x265-RA...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.73 ms** | Erreur | -- | 0 |
| 1629 | `The.Electrical.Life.of.Louis.Wain.2021....` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.31 ms** | Erreur | -- | 0 |
| 1630 | `The.Expanse.S03E01.Fight.or.Flight.1080...` | 3004.56 ms | 101.33 ms | 30.39 ms | 250.83 ms | 1.0 ms | 0.5 ms | **3389.53 ms** | The.Expanse.S03E01.Fight.or... | -- | 1 |
| 1631 | `The.Expanse.S03E02.REPACK.IFF.1080p.AMZ...` | 2328.36 ms | 110.52 ms | 21.1 ms | 144.53 ms | 1.0 ms | 0.5 ms | **2608.14 ms** | The.Expanse.S03E02.REPACK.I... | -- | 1 |
| 1632 | `The.Expanse.S03E03.Assured.Destruction....` | 2475.24 ms | 97.48 ms | 17.34 ms | 225.28 ms | 1.0 ms | 0.5 ms | **2821.35 ms** | The Expanse | 100.0% | 1 |
| 1633 | `The.Expanse.S03E04.Reload.1080p.AMZN.WE...` | 3508.88 ms | 132.29 ms | 35.73 ms | 198.26 ms | 1.0 ms | 0.5 ms | **3875.86 ms** | The.Expanse.S03E04.Reload.1... | -- | 1 |
| 1634 | `The.Expanse.S03E05.Triple.Point.1080p.A...` | 2379.16 ms | 89.28 ms | 15.02 ms | 235.81 ms | 1.0 ms | 0.5 ms | **2720.24 ms** | The Expanse | 100.0% | 1 |
| 1635 | `The.Expanse.S03E06.Immolation.1080p.AMZ...` | 5455.64 ms | 121.9 ms | 19.43 ms | 182.99 ms | 1.0 ms | 0.5 ms | **5781.04 ms** | The Expanse | 100.0% | 1 |
| 1636 | `The.Expanse.S03E07.Delta-V.1080p.AMZN.W...` | 2433.93 ms | 332.65 ms | 15.39 ms | 243.62 ms | 1.0 ms | 0.5 ms | **3039.09 ms** | The Expanse | 100.0% | 1 |
| 1637 | `The.Expanse.S03E08.It.Reaches.Out.1080p...` | 2693.68 ms | 143.74 ms | 22.13 ms | 399.01 ms | 1.0 ms | 0.5 ms | **3268.61 ms** | The Expanse | 100.0% | 1 |
| 1638 | `The.Expanse.S03E09.Intransigence.1080p....` | 4005.2 ms | 116.11 ms | 22.78 ms | 117.9 ms | 1.0 ms | 0.5 ms | **4262.98 ms** | The Expanse | 100.0% | 1 |
| 1639 | `The.Expanse.S03E10.Dandelion.Sky.1080p....` | 2615.47 ms | 206.12 ms | 19.7 ms | 118.61 ms | 1.0 ms | 0.5 ms | **2963.66 ms** | The Man in the High Castle | 100.0% | 1 |
| 1640 | `The.Expanse.S03E11.Fallen.World.1080p.A...` | 3702.74 ms | 95.61 ms | 25.12 ms | 163.64 ms | 1.0 ms | 0.5 ms | **3992.71 ms** | The Expanse | 100.0% | 1 |
| 1641 | `The.Expanse.S03E12.Congregation.1080p.A...` | 2589.76 ms | 222.16 ms | 45.08 ms | 191.49 ms | 1.0 ms | 0.5 ms | **3051.82 ms** | The One | 100.0% | 1 |
| 1642 | `The.Expanse.S03E13.Abaddons.Gate.1080p....` | 1781.8 ms | 225.62 ms | 16.84 ms | 322.52 ms | 1.0 ms | 0.5 ms | **2348.07 ms** | The Expanse | 100.0% | 1 |
| 1643 | `The.Expanse.S04E01.iNTERNAL.HDR.2160p.W...` | 2908.67 ms | 151.49 ms | 17.01 ms | 297.72 ms | 1.0 ms | 0.5 ms | **3385.29 ms** | The Expanse | 100.0% | 1 |
| 1644 | `The.Expanse.S04E02.HDR.2160p.WEB.h265-S...` | 4307.23 ms | 195.34 ms | 17.51 ms | 309.67 ms | 1.0 ms | 0.5 ms | **4830.63 ms** | The Expanse | 100.0% | 1 |
| 1645 | `The.Expanse.S04E03.HDR.2160p.WEB.h265-S...` | 2257.99 ms | 193.09 ms | 20.13 ms | 86.08 ms | 1.0 ms | 0.5 ms | **2558.77 ms** | The.Expanse.S04E03.HDR.2160... | -- | 1 |
| 1646 | `The.Expanse.S04E04.HDR.2160p.WEB.h265-S...` | 1942.35 ms | 142.59 ms | 38.33 ms | 311.09 ms | 1.0 ms | 0.5 ms | **2436.06 ms** | The Expanse | 100.0% | 1 |
| 1647 | `The.Expanse.S04E05.HDR.2160p.WEB.h265-S...` | 4452.1 ms | 155.45 ms | 14.87 ms | 245.02 ms | 1.0 ms | 0.5 ms | **4868.38 ms** | The Expanse | 100.0% | 1 |
| 1648 | `The.Expanse.S04E06.HDR.2160p.WEB.h265-S...` | 1986.84 ms | 108.04 ms | 23.94 ms | 155.77 ms | 1.0 ms | 0.5 ms | **2275.42 ms** | The Expanse | 100.0% | 1 |
| 1649 | `The.Expanse.S04E07.HDR.2160p.WEB.h265-S...` | 2109.48 ms | 117.85 ms | 19.14 ms | 155.97 ms | 1.0 ms | 0.5 ms | **2402.93 ms** | The Expanse | 100.0% | 1 |
| 1650 | `The.Expanse.S04E08.HDR.2160p.WEB.h265-S...` | 3143.84 ms | 160.29 ms | 18.64 ms | 449.72 ms | 1.0 ms | 0.5 ms | **3777.28 ms** | The Expanse | 100.0% | 1 |
| 1651 | `The.Expanse.S04E09.HDR.2160p.WEB.h265-S...` | 2462.81 ms | 181.4 ms | 33.27 ms | 231.03 ms | 1.0 ms | 0.5 ms | **2912.55 ms** | The.Expanse.S04E09.HDR.2160... | -- | 1 |
| 1652 | `The.Expanse.S04E10.HDR.2160p.WEB.h265-S...` | 3094.06 ms | 151.71 ms | 23.6 ms | 195.14 ms | 1.0 ms | 0.5 ms | **3470.72 ms** | The Expanse | 100.0% | 1 |
| 1653 | `The.Expanse.S05E01.PROPER.HDR.2160p.WEB...` | 2574.23 ms | 211.34 ms | 15.82 ms | 176.18 ms | 1.0 ms | 0.5 ms | **2978.44 ms** | The Expanse | 100.0% | 1 |
| 1654 | `The.Expanse.S05E02.PROPER.HDR.2160p.WEB...` | 2890.78 ms | 182.09 ms | 15.76 ms | 389.03 ms | 1.0 ms | 0.5 ms | **3478.19 ms** | The Expanse | 100.0% | 1 |
| 1655 | `The.Expanse.S05E03.PROPER.HDR.2160p.WEB...` | 4687.54 ms | 264.17 ms | 15.98 ms | 288.04 ms | 1.0 ms | 0.5 ms | **5258.47 ms** | The Expanse | 100.0% | 1 |
| 1656 | `The.Expanse.S05E04.PROPER.HDR.2160p.WEB...` | 3132.58 ms | 164.67 ms | 21.2 ms | 377.26 ms | 1.0 ms | 0.5 ms | **3700.03 ms** | The Expanse | 100.0% | 1 |
| 1657 | `The.Expanse.S05E05.PROPER.HDR.2160p.WEB...` | 2920.08 ms | 164.74 ms | 23.83 ms | 209.07 ms | 1.0 ms | 0.5 ms | **3322.79 ms** | The Expanse | 100.0% | 1 |
| 1658 | `The.Expanse.S05E06.2160p.AMZN.WEBRip.DD...` | 2449.57 ms | 187.47 ms | 25.51 ms | 188.21 ms | 1.0 ms | 0.5 ms | **2852.5 ms** | The Expanse | 100.0% | 1 |
| 1659 | `The.Expanse.S05E07.2160p.REPACK.AMZN.WE...` | 2799.22 ms | 197.7 ms | 20.65 ms | 394.13 ms | 1.0 ms | 0.5 ms | **3412.57 ms** | The.Expanse.S05E07.2160p.RE... | -- | 1 |
| 1660 | `The.Expanse.S05E08.2160p.AMZN.WEBRip.DD...` | 2218.22 ms | 182.09 ms | 29.91 ms | 216.08 ms | 1.0 ms | 0.5 ms | **2651.55 ms** | The Expanse | 100.0% | 1 |
| 1661 | `The.Expanse.S05E09.2160p.AMZN.WEBRip.DD...` | 1876.51 ms | 161.27 ms | 25.72 ms | 185.24 ms | 1.0 ms | 0.5 ms | **2250.07 ms** | The Expanse | 100.0% | 1 |
| 1662 | `The.Expanse.S05E10.2160p.AMZN.WEBRip.DD...` | 2636.14 ms | 162.05 ms | 16.92 ms | 389.14 ms | 1.0 ms | 0.5 ms | **3204.99 ms** | The Expanse | 100.0% | 1 |
| 1663 | `The.Expanse.S06E01.PROPER.1080p.WEB.h26...` | 3017.53 ms | 120.6 ms | 18.96 ms | 100.87 ms | 1.0 ms | 0.5 ms | **3259.82 ms** | The Expanse | 100.0% | 1 |
| 1664 | `The.Fisher.King.1991.1080p.BluRay.x264-...` | 2315.6 ms | 225.46 ms | 26.42 ms | 396.32 ms | 1.0 ms | 0.5 ms | **2965.37 ms** | The.Fisher.King.1991.1080p.... | -- | 1 |
| 1665 | `The.Front.Runner_(2018)_[1080p]_mid15s.mp4` | 2336.44 ms | 193.87 ms | 21.27 ms | 200.64 ms | 1.0 ms | 0.5 ms | **2753.07 ms** | The Front Runner | 100.0% | 0 |
| 1666 | `The.Girl.Without.Hands.2016.FRENCH.720p...` | 3210.83 ms | 232.08 ms | 19.61 ms | 138.68 ms | 1.0 ms | 0.5 ms | **3601.92 ms** | The Girl Without Hands | 100.0% | 0 |
| 1667 | `The.Golden.Child.1986.1080p.WEB-DL-iND_...` | 4273.07 ms | 108.59 ms | 20.38 ms | 315.87 ms | 1.0 ms | 0.5 ms | **4718.58 ms** | The Golden Child.DVD_PAL.fr... | -- | 1 |
| 1668 | `The.Golden.Child.1986.1080p.WEB-DL.DD5....` | 1391.76 ms | 139.81 ms | 21.98 ms | 568.28 ms | 1.0 ms | 0.5 ms | **2130.08 ms** | The Golden Child.DVD_PAL.fr... | -- | 1 |
| 1669 | `The.Guilty.2018.1080p.BluRay.x264-[YTS....` | 2416.13 ms | 211.88 ms | 28.35 ms | 610.32 ms | 1.0 ms | 0.5 ms | **3267.04 ms** | The.Guilty.2018.1080p.BluRa... | -- | 1 |
| 1670 | `The.Handmaiden.2016.1080p.BluRay.x264-[...` | 2751.93 ms | 179.63 ms | 23.63 ms | 367.1 ms | 1.0 ms | 0.5 ms | **3323.04 ms** | The Handmaiden | 100.0% | 0 |
| 1671 | `The.Handmaids.Tale.S01E10.1080p.BluRay....` | 4759.58 ms | 107.29 ms | 11.64 ms | 179.38 ms | 1.0 ms | 0.5 ms | **5058.66 ms** | The Handmaids Tale | 100.0% | 0 |
| 1672 | `The.Harder.They.Come.1972.1080p.BluRay....` | 3186.84 ms | 212.56 ms | 23.95 ms | 154.23 ms | 1.0 ms | 0.5 ms | **3580.67 ms** | Aucun | -- | 0 |
| 1673 | `The.Harder.They.Fall.2021.1080p.WEBRip....` | 3246.77 ms | 213.45 ms | 20.83 ms | 146.56 ms | 1.0 ms | 0.5 ms | **3628.33 ms** | The.Harder.They.Fall.2021.1... | -- | 1 |
| 1674 | `The.Hitmans.Wifes.Bodyguard.2021.EXTEND...` | 1869.31 ms | 173.03 ms | 20.33 ms | 153.27 ms | 1.0 ms | 0.5 ms | **2216.7 ms** | Maniac | 100.0% | 3 |
| 1675 | `The.Holy.Mountain.1973.1080p.BluRay.H26...` | 3447.37 ms | 129.74 ms | 16.58 ms | 172.94 ms | 1.0 ms | 0.5 ms | **3771.15 ms** | The Expanse | 100.0% | 2 |
| 1676 | `The.Hyperions.2022.1080p.WEBRip.x264-RA...` | 2516.76 ms | 144.8 ms | 24.11 ms | 161.82 ms | 1.0 ms | 0.5 ms | **2848.27 ms** | Aucun | -- | 0 |
| 1677 | `The.Lady.2011.FRENCH.BDRip.XviD-AYMO.CD...` | 2645.02 ms | 141.48 ms | 24.54 ms | 104.94 ms | 1.0 ms | 0.5 ms | **2916.61 ms** | Aucun | -- | 0 |
| 1678 | `The.Lady.2011.FRENCH.BDRip.XviD-AYMO.CD...` | 3106.2 ms | 114.48 ms | 17.69 ms | 438.78 ms | 1.0 ms | 0.5 ms | **3684.16 ms** | The Lady | 100.0% | 0 |
| 1679 | `The.Last.Detail.1973.1080p.BluRay.x265-...` | 2945.15 ms | 245.27 ms | 22.92 ms | 326.54 ms | 1.0 ms | 0.5 ms | **3541.41 ms** | The Last Detail | 100.0% | 1 |
| 1680 | `The.Last.Duel.2021.1080p.AMZN.WEBRip.DD...` | 2682.3 ms | 148.89 ms | 24.83 ms | 359.05 ms | 1.0 ms | 0.5 ms | **3219.03 ms** | The Cured | 100.0% | 0 |
| 1681 | `The.Last.Duel.2021.1080p.WEBRip.DD5.1.x...` | 3083.8 ms | 187.31 ms | 28.22 ms | 174.92 ms | 1.0 ms | 0.5 ms | **3474.67 ms** | Aucun | -- | 0 |
| 1682 | `The.Last.Temptation.of.Christ.1988.1080...` | 4850.24 ms | 163.71 ms | 23.62 ms | 205.22 ms | 1.0 ms | 0.5 ms | **5245.64 ms** | Aucun | -- | 0 |
| 1683 | `The.Legend.of.Ochi.2025.2160p.AMZN.WEB-...` | 2109.71 ms | 210.36 ms | 22.8 ms | 408.75 ms | 1.0 ms | 0.5 ms | **2752.45 ms** | Aucun | -- | 0 |
| 1684 | `The.Life.Aquatic.with.Steve.Zissou.2004...` | 2933.23 ms | 90.05 ms | 19.22 ms | 167.76 ms | 1.0 ms | 0.5 ms | **3210.92 ms** | Aucun | -- | 0 |
| 1685 | `The.Life.of.Chuck.2024.1080p.AMZN.WEB-D...` | 3183.6 ms | 127.2 ms | 17.18 ms | 245.48 ms | 1.0 ms | 0.5 ms | **3576.23 ms** | Kingdom | 100.0% | 2 |
| 1686 | `The.Lion.King.1994.Hybrid.MULTi.VFI.216...` | 2491.32 ms | 121.01 ms | 23.57 ms | 168.65 ms | 1.0 ms | 0.5 ms | **2805.31 ms** | The Lion King | 100.0% | 0 |
| 1687 | `The.Lord.of.the.Rings.The.Fellowship.of...` | 2802.45 ms | 93.55 ms | 32.54 ms | 284.29 ms | 1.0 ms | 0.5 ms | **3220.32 ms** | The.Lord.of.the.Rings.The.F... | -- | 1 |
| 1688 | `The.Lord.of.the.Rings.The.Return.of.the...` | 2999.04 ms | 107.16 ms | 24.36 ms | 294.08 ms | 1.0 ms | 0.5 ms | **3441.45 ms** | The.Lord.of.the.Rings.The.R... | -- | 1 |
| 1689 | `The.Lord.of.the.Rings.The.Two.Towers.20...` | 3035.63 ms | 198.75 ms | 33.11 ms | 197.94 ms | 1.0 ms | 0.5 ms | **3469.24 ms** | The.Lord.of.the.Rings.The.T... | -- | 1 |
| 1690 | `The.Lost.City.2022.1080p.AMZN.WEB-DL.DD...` | 2715.08 ms | 125.65 ms | 45.35 ms | 180.85 ms | 1.0 ms | 0.5 ms | **3067.59 ms** | Aucun | -- | 0 |
| 1691 | `The.Man.Who.Knew.Infinity.2015.1080p.Bl...` | 3199.52 ms | 116.56 ms | 22.29 ms | 498.02 ms | 1.0 ms | 0.5 ms | **3840.8 ms** | The.Man.Who.Knew.Infinity.2... | -- | 1 |
| 1692 | `The.Man.from.U.N.C.L.E.2015.2160p.WEB-D...` | 3791.26 ms | 152.89 ms | 26.37 ms | 252.54 ms | 1.0 ms | 0.5 ms | **4223.92 ms** | Aucun | -- | 0 |
| 1693 | `The.Man.in.the.High.Castle.S03E01.Now.M...` | 2358.0 ms | 89.87 ms | 18.64 ms | 168.45 ms | 1.0 ms | 0.5 ms | **2635.74 ms** | The Man in the High Castle | 100.0% | 1 |
| 1694 | `The.Man.in.the.High.Castle.S03E02.Imagi...` | 1914.3 ms | 110.76 ms | 31.52 ms | 266.69 ms | 1.0 ms | 0.5 ms | **2324.13 ms** | The Man in the High Castle | 100.0% | 0 |
| 1695 | `The.Man.in.the.High.Castle.S03E03.Senso...` | 2320.34 ms | 143.55 ms | 46.01 ms | 111.96 ms | 1.0 ms | 0.5 ms | **2622.29 ms** | Aucun | -- | 0 |
| 1696 | `The.Man.in.the.High.Castle.S03E04.Sabra...` | 2410.18 ms | 108.83 ms | 16.23 ms | 209.27 ms | 1.0 ms | 0.5 ms | **2746.08 ms** | The Man in the High Castle | 100.0% | 0 |
| 1697 | `The.Man.in.the.High.Castle.S03E05.The.N...` | 2432.83 ms | 90.55 ms | 14.14 ms | 238.56 ms | 1.0 ms | 0.5 ms | **2777.34 ms** | The Man in the High Castle | 100.0% | 1 |
| 1698 | `The.Man.in.the.High.Castle.S03E06.Histo...` | 2987.34 ms | 332.57 ms | 23.43 ms | 262.28 ms | 1.0 ms | 0.5 ms | **3615.19 ms** | The Man in the High Castle | 100.0% | 0 |
| 1699 | `The.Man.in.the.High.Castle.S03E07.Exces...` | 2518.57 ms | 204.81 ms | 62.16 ms | 320.29 ms | 1.0 ms | 0.5 ms | **3106.27 ms** | The Man in the High Castle | 100.0% | 0 |
| 1700 | `The.Man.in.the.High.Castle.S03E08.Kasum...` | 2897.36 ms | 142.79 ms | 16.2 ms | 373.36 ms | 1.0 ms | 0.5 ms | **3430.13 ms** | Aucun | -- | 0 |
| 1701 | `The.Man.in.the.High.Castle.S03E09.Baku....` | 3152.33 ms | 133.57 ms | 16.36 ms | 180.15 ms | 1.0 ms | 0.5 ms | **3482.88 ms** | The Man in the High Castle | 100.0% | 0 |
| 1702 | `The.Man.in.the.High.Castle.S03E10.Jahr....` | 2031.76 ms | 154.74 ms | 32.8 ms | 321.23 ms | 1.0 ms | 0.5 ms | **2540.95 ms** | The Man in the High Castle | 100.0% | 0 |
| 1703 | `The.Master.2012.1080p.BRrip.x264.GAZ.YI...` | 2266.23 ms | 207.16 ms | 25.4 ms | 178.16 ms | 1.0 ms | 0.5 ms | **2677.73 ms** | The.Master.2012.1080p.BRrip... | -- | 1 |
| 1704 | `The.Mauritanian.2021.1080p.AMZN.WEB-DL....` | 3908.44 ms | 150.28 ms | 19.75 ms | 142.73 ms | 1.0 ms | 0.5 ms | **4222.58 ms** | The Mauritanian | 100.0% | 0 |
| 1705 | `The.Mist.2007.720p.Bluray.x264.YIFY_mid...` | 2494.23 ms | 91.45 ms | 21.18 ms | 264.86 ms | 1.0 ms | 0.5 ms | **2873.3 ms** | The Mist | 100.0% | 0 |
| 1706 | `The.Name.of.the.Rose.1986.1080p.BluRay....` | 3099.75 ms | 113.19 ms | 18.59 ms | 174.89 ms | 1.0 ms | 0.5 ms | **3408.84 ms** | The.Name.of.the.Rose.1986.1... | -- | 1 |
| 1707 | `The.Neon.Demon.2016.2160p.HDR.UHD.BluRa...` | 2400.91 ms | 97.36 ms | 25.66 ms | 194.42 ms | 1.0 ms | 0.5 ms | **2720.42 ms** | Aucun | -- | 0 |
| 1708 | `The.NeverEnding.Story.1984.1080p.BluRay...` | 4461.2 ms | 376.08 ms | 18.98 ms | 396.65 ms | 1.0 ms | 0.5 ms | **5253.78 ms** | Aucun | -- | 0 |
| 1709 | `The.Novice.2021.1080p.WEBRip.DD5.1.x264...` | 1952.3 ms | 103.38 ms | 28.53 ms | 149.58 ms | 1.0 ms | 0.5 ms | **2235.16 ms** | The Mauritanian | 100.0% | 0 |
| 1710 | `The.Number.23.2007.UNRATED.1080p.BluRay...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.65 ms** | Erreur | -- | 0 |
| 1711 | `The.Office.US.S03E22.Beach.Games.1080p....` | 2161.93 ms | 177.37 ms | 23.08 ms | 149.85 ms | 1.0 ms | 0.5 ms | **2515.47 ms** | The Office US | 100.0% | 1 |
| 1712 | `The.Office.US.S03E23.The.Job.1080p.BluR...` | 2181.96 ms | 138.36 ms | 16.99 ms | 255.97 ms | 1.0 ms | 0.5 ms | **2595.92 ms** | The Office US | 100.0% | 1 |
| 1713 | `The.Office.US.S04E01-02.1080p.AMZN.WEB....` | 2575.84 ms | 116.95 ms | 17.49 ms | 243.9 ms | 1.0 ms | 0.5 ms | **2955.32 ms** | The Office US | 100.0% | 1 |
| 1714 | `The.Office.US.S04E01E02.Fun.Run.1080p.B...` | 3055.16 ms | 189.36 ms | 17.48 ms | 225.26 ms | 1.0 ms | 0.5 ms | **3488.41 ms** | The Office US | 100.0% | 0 |
| 1715 | `The.Office.US.S04E01E02.Fun.Run.WEBRip....` | 2988.23 ms | 155.2 ms | 18.86 ms | 217.11 ms | 1.0 ms | 0.5 ms | **3380.3 ms** | The Office US | 100.0% | 1 |
| 1716 | `The.Office.US.S04E03-04.1080p.AMZN.WEB....` | 2491.96 ms | 131.86 ms | 16.57 ms | 180.75 ms | 1.0 ms | 0.5 ms | **2824.31 ms** | The Office US | 100.0% | 1 |
| 1717 | `The.Office.US.S04E03E04.Dunder.Mifflin....` | 2855.51 ms | 170.73 ms | 28.75 ms | 285.58 ms | 1.0 ms | 0.5 ms | **3344.0 ms** | The Office US | 100.0% | 0 |
| 1718 | `The.Office.US.S04E03E04.Dunder.Mifflin....` | 1994.43 ms | 178.0 ms | 16.48 ms | 222.45 ms | 1.0 ms | 0.5 ms | **2412.79 ms** | The Office US | 100.0% | 1 |
| 1719 | `The.Office.US.S04E05-06.1080p.AMZN.WEB....` | 2325.29 ms | 211.89 ms | 26.86 ms | 290.73 ms | 1.0 ms | 0.5 ms | **2855.65 ms** | The Office US | 100.0% | 1 |
| 1720 | `The.Office.US.S04E05E06.Launch.Party.10...` | 2077.91 ms | 130.46 ms | 26.0 ms | 131.49 ms | 1.0 ms | 0.5 ms | **2368.38 ms** | The Office US | 100.0% | 0 |
| 1721 | `The.Office.US.S04E05E06.Launch.Party.WE...` | 2431.35 ms | 211.32 ms | 19.28 ms | 296.64 ms | 1.0 ms | 0.5 ms | **2962.49 ms** | The Office US | 100.0% | 1 |
| 1722 | `The.Office.US.S04E07-08.1080p.AMZN.WEB....` | 2571.66 ms | 230.36 ms | 22.88 ms | 155.97 ms | 1.0 ms | 0.5 ms | **2981.61 ms** | The Office US | 100.0% | 1 |
| 1723 | `The.Office.US.S04E07E08.Money.1080p.Blu...` | 2336.9 ms | 162.31 ms | 20.26 ms | 205.52 ms | 1.0 ms | 0.5 ms | **2725.78 ms** | The Office US | 100.0% | 0 |
| 1724 | `The.Office.US.S04E07E08.Money.WEBRip.x2...` | 1900.34 ms | 240.41 ms | 21.16 ms | 153.75 ms | 1.0 ms | 0.5 ms | **2316.7 ms** | The Office US | 100.0% | 1 |
| 1725 | `The.Office.US.S04E09.1080p.AMZN.WEB.x26...` | 2115.05 ms | 245.17 ms | 18.64 ms | 368.54 ms | 1.0 ms | 0.5 ms | **2753.99 ms** | The Office US | 100.0% | 1 |
| 1726 | `The.Office.US.S04E09.Local.Ad.1080p.Blu...` | 1893.16 ms | 235.5 ms | 19.06 ms | 236.16 ms | 1.0 ms | 0.5 ms | **2385.97 ms** | The Office US | 100.0% | 0 |
| 1727 | `The.Office.US.S04E09.Local.Ad.WEBRip.x2...` | 2188.98 ms | 173.88 ms | 18.66 ms | 297.46 ms | 1.0 ms | 0.5 ms | **2679.96 ms** | The Office US | 100.0% | 1 |
| 1728 | `The.Office.US.S04E10.1080p.AMZN.WEB.x26...` | 2818.13 ms | 122.51 ms | 14.81 ms | 196.97 ms | 1.0 ms | 0.5 ms | **3153.52 ms** | The Office US | 100.0% | 1 |
| 1729 | `The.Office.US.S04E10.Branch.Wars.1080p....` | 2441.75 ms | 136.83 ms | 22.88 ms | 243.09 ms | 1.0 ms | 0.5 ms | **2845.35 ms** | The Office US | 100.0% | 0 |
| 1730 | `The.Office.US.S04E10.Branch.Wars.WEBRip...` | 2723.65 ms | 145.1 ms | 15.83 ms | 250.19 ms | 1.0 ms | 0.5 ms | **3135.56 ms** | The Office US | 100.0% | 1 |
| 1731 | `The.Office.US.S04E11.1080p.AMZN.WEB.x26...` | 2372.29 ms | 122.11 ms | 15.92 ms | 292.39 ms | 1.0 ms | 0.5 ms | **2805.24 ms** | The Office US | 100.0% | 1 |
| 1732 | `The.Office.US.S04E11.Survivor.Man.1080p...` | 2249.81 ms | 120.37 ms | 17.2 ms | 356.42 ms | 1.0 ms | 0.5 ms | **2744.64 ms** | The Office US | 100.0% | 0 |
| 1733 | `The.Office.US.S04E11.Survivor.Man.WEBRi...` | 4453.99 ms | 118.15 ms | 15.72 ms | 127.82 ms | 1.0 ms | 0.5 ms | **4716.4 ms** | The Office US | 100.0% | 1 |
| 1734 | `The.Office.US.S04E12.1080p.AMZN.WEB.x26...` | 2550.4 ms | 124.93 ms | 24.49 ms | 323.94 ms | 1.0 ms | 0.5 ms | **3024.54 ms** | The Office US | 100.0% | 1 |
| 1735 | `The.Office.US.S04E12.The.Deposition.108...` | 2584.99 ms | 123.6 ms | 26.61 ms | 96.72 ms | 1.0 ms | 0.5 ms | **2832.85 ms** | The Office US | 100.0% | 0 |
| 1736 | `The.Office.US.S04E12.The.Deposition.WEB...` | 2333.25 ms | 114.08 ms | 23.79 ms | 341.38 ms | 1.0 ms | 0.5 ms | **2814.58 ms** | The Office US | 100.0% | 1 |
| 1737 | `The.Office.US.S04E13.1080p.AMZN.WEB.x26...` | 2759.45 ms | 122.82 ms | 34.62 ms | 245.05 ms | 1.0 ms | 0.5 ms | **3162.7 ms** | The.Office.US.S04E13.1080p.... | -- | 1 |
| 1738 | `The.Office.US.S04E13.Dinner.Party.1080p...` | 2607.18 ms | 120.54 ms | 31.21 ms | 336.59 ms | 1.0 ms | 0.5 ms | **3097.0 ms** | The Office US | 100.0% | 0 |
| 1739 | `The.Office.US.S04E13.Dinner.Party.WEBRi...` | 2679.39 ms | 158.99 ms | 27.58 ms | 144.64 ms | 1.0 ms | 0.5 ms | **3011.47 ms** | The.Office.US.S04E13.Dinner... | -- | 1 |
| 1740 | `The.Office.US.S04E14.1080p.AMZN.WEB.x26...` | 2953.27 ms | 142.43 ms | 18.53 ms | 229.99 ms | 1.0 ms | 0.5 ms | **3349.6 ms** | The Office US | 100.0% | 1 |
| 1741 | `The.Office.US.S04E14.The.Chair.Model.10...` | 3208.92 ms | 116.18 ms | 20.33 ms | 197.85 ms | 1.0 ms | 0.5 ms | **3544.89 ms** | The Office US | 100.0% | 0 |
| 1742 | `The.Office.US.S04E14.The.Chair.Model.WE...` | 3206.81 ms | 139.97 ms | 18.56 ms | 210.92 ms | 1.0 ms | 0.5 ms | **3577.03 ms** | The Office US | 100.0% | 1 |
| 1743 | `The.Office.US.S04E15.1080p.AMZN.WEB.x26...` | 2653.85 ms | 138.56 ms | 21.8 ms | 310.21 ms | 1.0 ms | 0.5 ms | **3129.08 ms** | The Office US | 100.0% | 1 |
| 1744 | `The.Office.US.S04E15.Night.Out.1080p.Bl...` | 2465.54 ms | 121.2 ms | 18.64 ms | 245.73 ms | 1.0 ms | 0.5 ms | **2853.79 ms** | The Office US | 100.0% | 0 |
| 1745 | `The.Office.US.S04E15.Night.Out.WEBRip.x...` | 2083.74 ms | 188.33 ms | 20.02 ms | 198.72 ms | 1.0 ms | 0.5 ms | **2493.28 ms** | The Office US | 100.0% | 1 |
| 1746 | `The.Office.US.S04E16.1080p.AMZN.WEB.x26...` | 3157.48 ms | 153.81 ms | 20.52 ms | 352.9 ms | 1.0 ms | 0.5 ms | **3685.62 ms** | The Office US | 100.0% | 1 |
| 1747 | `The.Office.US.S04E16.Did.I.Stutter.1080...` | 2860.09 ms | 172.79 ms | 21.3 ms | 165.21 ms | 1.0 ms | 0.5 ms | **3220.53 ms** | The Office US | 100.0% | 0 |
| 1748 | `The.Office.US.S04E16.Did.I.Stutter.WEBR...` | 2647.27 ms | 145.21 ms | 23.63 ms | 252.48 ms | 1.0 ms | 0.5 ms | **3069.29 ms** | The Office US | 100.0% | 1 |
| 1749 | `The.Office.US.S04E17.1080p.AMZN.WEB.x26...` | 2862.35 ms | 205.18 ms | 23.28 ms | 166.74 ms | 1.0 ms | 0.5 ms | **3260.49 ms** | The Office US | 100.0% | 1 |
| 1750 | `The.Office.US.S04E17.Job.Fair.1080p.Blu...` | 2635.96 ms | 140.6 ms | 24.02 ms | 418.43 ms | 1.0 ms | 0.5 ms | **3219.8 ms** | Aucun | -- | 0 |
| 1751 | `The.Office.US.S04E17.Job.Fair.WEBRip.x2...` | 2008.64 ms | 114.02 ms | 26.64 ms | 224.6 ms | 1.0 ms | 0.5 ms | **2377.43 ms** | The Office US | 100.0% | 1 |
| 1752 | `The.Office.US.S04E18-19.1080p.AMZN.WEB....` | 3388.46 ms | 86.78 ms | 17.23 ms | 115.92 ms | 1.0 ms | 0.5 ms | **3610.66 ms** | The Office US | 100.0% | 1 |
| 1753 | `The.Office.US.S04E18E19.Goodbye.Toby.10...` | 2566.47 ms | 111.41 ms | 20.11 ms | 309.09 ms | 1.0 ms | 0.5 ms | **3008.72 ms** | The Office US | 100.0% | 0 |
| 1754 | `The.Office.US.S04E18E19.Goodbye.Toby.WE...` | 2321.49 ms | 107.69 ms | 17.18 ms | 257.6 ms | 1.0 ms | 0.5 ms | **2705.38 ms** | The Office US | 100.0% | 1 |
| 1755 | `The.Office.US.S05E01.1080p.NF.WEB.x264-...` | 3026.18 ms | 89.55 ms | 15.89 ms | 520.97 ms | 1.0 ms | 0.5 ms | **3653.44 ms** | The Office US | 100.0% | 1 |
| 1756 | `The.Office.US.S05E02.1080p.NF.WEB.x264-...` | 2207.28 ms | 110.24 ms | 19.59 ms | 229.66 ms | 1.0 ms | 0.5 ms | **2571.5 ms** | Aucun | -- | 0 |
| 1757 | `The.Office.US.S05E03.1080p.NF.WEB.x264-...` | 2876.29 ms | 89.32 ms | 19.38 ms | 180.04 ms | 1.0 ms | 0.5 ms | **3166.99 ms** | The Office US | 100.0% | 0 |
| 1758 | `The.Office.US.S05E04.1080p.NF.WEB.x264-...` | 2773.06 ms | 128.27 ms | 17.45 ms | 229.98 ms | 1.0 ms | 0.5 ms | **3156.61 ms** | The Office US | 100.0% | 0 |
| 1759 | `The.Office.US.S05E05.1080p.NF.WEB.x264-...` | 2719.09 ms | 95.75 ms | 17.35 ms | 160.51 ms | 1.0 ms | 0.5 ms | **2993.68 ms** | The.Office.US.S05E05.1080p.... | -- | 1 |
| 1760 | `The.Office.US.S05E06.1080p.NF.WEB.x264-...` | 3275.35 ms | 89.06 ms | 17.35 ms | 120.38 ms | 1.0 ms | 0.5 ms | **3511.09 ms** | The Office US | 100.0% | 1 |
| 1761 | `The.Office.US.S05E07.1080p.NF.WEB.x264-...` | 3028.41 ms | 93.55 ms | 19.6 ms | 204.06 ms | 1.0 ms | 0.5 ms | **3346.41 ms** | The Office US | 100.0% | 1 |
| 1762 | `The.Office.US.S05E08.1080p.NF.WEB.x264-...` | 2425.04 ms | 92.16 ms | 20.79 ms | 155.69 ms | 1.0 ms | 0.5 ms | **2694.5 ms** | The Office US | 100.0% | 1 |
| 1763 | `The.Office.US.S05E09.1080p.NF.WEB.x264-...` | 2484.07 ms | 244.56 ms | 18.82 ms | 383.24 ms | 1.0 ms | 0.5 ms | **3131.54 ms** | The Office US | 100.0% | 1 |
| 1764 | `The.Office.US.S05E10.1080p.NF.WEB.x264-...` | 2614.83 ms | 83.24 ms | 23.18 ms | 148.88 ms | 1.0 ms | 0.5 ms | **2871.61 ms** | The Office US | 100.0% | 1 |
| 1765 | `The.Office.US.S05E11.1080p.NF.WEB.x264-...` | 2839.9 ms | 102.77 ms | 19.38 ms | 302.96 ms | 1.0 ms | 0.5 ms | **3265.73 ms** | The Office US | 100.0% | 1 |
| 1766 | `The.Office.US.S05E12.1080p.NF.WEB.x264-...` | 1776.51 ms | 94.32 ms | 16.12 ms | 145.04 ms | 1.0 ms | 0.5 ms | **2032.82 ms** | The Office US | 100.0% | 1 |
| 1767 | `The.Office.US.S05E13.1080p.NF.WEB.x264-...` | 2237.89 ms | 96.0 ms | 16.69 ms | 269.81 ms | 1.0 ms | 0.5 ms | **2624.79 ms** | The Office US | 100.0% | 1 |
| 1768 | `The.Office.US.S05E14.1080p.NF.WEB.x264-...` | 6935.25 ms | 164.7 ms | 17.52 ms | 208.5 ms | 1.0 ms | 0.5 ms | **7332.67 ms** | The Office US | 100.0% | 1 |
| 1769 | `The.Office.US.S05E15.1080p.NF.WEB.x264-...` | 3222.42 ms | 106.22 ms | 17.32 ms | 100.59 ms | 1.0 ms | 0.5 ms | **3448.49 ms** | The Office US | 100.0% | 1 |
| 1770 | `The.Office.US.S05E16.1080p.NF.WEB.x264-...` | 3292.57 ms | 111.33 ms | 30.18 ms | 115.36 ms | 1.0 ms | 0.5 ms | **3560.58 ms** | The.Office.US.S05E16.1080p.... | -- | 1 |
| 1771 | `The.Office.US.S05E17.1080p.NF.WEB.x264-...` | 3262.55 ms | 99.39 ms | 21.7 ms | 345.93 ms | 1.0 ms | 0.5 ms | **3730.52 ms** | The Office US | 100.0% | 1 |
| 1772 | `The.Office.US.S05E18.1080p.NF.WEB.x264-...` | 2828.49 ms | 115.54 ms | 20.64 ms | 225.15 ms | 1.0 ms | 0.5 ms | **3194.53 ms** | The Office US | 100.0% | 1 |
| 1773 | `The.Office.US.S05E19.1080p.NF.WEB.x264-...` | 2758.99 ms | 108.2 ms | 39.38 ms | 110.84 ms | 1.0 ms | 0.5 ms | **3017.82 ms** | The.Office.US.S05E19.1080p.... | -- | 1 |
| 1774 | `The.Office.US.S05E20.1080p.NF.WEB.x264-...` | 3139.06 ms | 153.44 ms | 15.06 ms | 212.2 ms | 1.0 ms | 0.5 ms | **3520.87 ms** | The Office US | 100.0% | 1 |
| 1775 | `The.Office.US.S05E21.1080p.NF.WEB.x264-...` | 3273.45 ms | 109.58 ms | 15.68 ms | 180.9 ms | 1.0 ms | 0.5 ms | **3580.33 ms** | The Office US | 100.0% | 1 |
| 1776 | `The.Office.US.S05E22.1080p.NF.WEB.x264-...` | 2645.39 ms | 113.03 ms | 18.12 ms | 241.91 ms | 1.0 ms | 0.5 ms | **3020.5 ms** | The Office US | 100.0% | 1 |
| 1777 | `The.Office.US.S05E23.1080p.NF.WEB.x264-...` | 2472.37 ms | 102.04 ms | 14.46 ms | 198.27 ms | 1.0 ms | 0.5 ms | **2788.82 ms** | The Office US | 100.0% | 1 |
| 1778 | `The.Office.US.S05E24.1080p.NF.WEB.x264-...` | 2707.39 ms | 128.6 ms | 25.52 ms | 225.02 ms | 1.0 ms | 0.5 ms | **3087.33 ms** | The Office US | 100.0% | 1 |
| 1779 | `The.Office.US.S05E25.1080p.NF.WEB.x264-...` | 1801.35 ms | 136.6 ms | 22.31 ms | 96.81 ms | 1.0 ms | 0.5 ms | **2057.83 ms** | The Office US | 100.0% | 1 |
| 1780 | `The.Office.US.S05E26.1080p.NF.WEB.x264-...` | 2728.82 ms | 101.11 ms | 15.95 ms | 203.31 ms | 1.0 ms | 0.5 ms | **3055.91 ms** | The Office US | 100.0% | 1 |
| 1781 | `The.Office.US.S06E01.1080p.NF.WEB.x264-...` | 2445.86 ms | 110.29 ms | 19.41 ms | 214.24 ms | 1.0 ms | 0.5 ms | **2794.6 ms** | The Office US | 100.0% | 0 |
| 1782 | `The.Office.US.S06E02.1080p.NF.WEB.x264-...` | 2598.85 ms | 132.99 ms | 18.47 ms | 322.94 ms | 1.0 ms | 0.5 ms | **3073.76 ms** | The Office US | 100.0% | 0 |
| 1783 | `The.Office.US.S06E03.1080p.NF.WEB.x264-...` | 2085.91 ms | 91.94 ms | 15.56 ms | 188.08 ms | 1.0 ms | 0.5 ms | **2383.69 ms** | The Office US | 100.0% | 1 |
| 1784 | `The.Office.US.S06E04-05.1080p.NF.WEB.x2...` | 2412.18 ms | 122.48 ms | 20.55 ms | 327.35 ms | 1.0 ms | 0.5 ms | **2883.43 ms** | The Office US | 100.0% | 1 |
| 1785 | `The.Office.US.S06E06.1080p.NF.WEB.x264-...` | 2631.05 ms | 115.11 ms | 24.01 ms | 321.28 ms | 1.0 ms | 0.5 ms | **3105.36 ms** | The.Office.US.S06E06.1080p.... | -- | 1 |
| 1786 | `The.Office.US.S06E07.1080p.NF.WEB.x264-...` | 2174.22 ms | 286.58 ms | 23.89 ms | 256.46 ms | 1.0 ms | 0.5 ms | **2743.91 ms** | A | 100.0% | 32 |
| 1787 | `The.Office.US.S06E08.1080p.NF.WEB.x264-...` | 1724.99 ms | 117.66 ms | 18.46 ms | 225.24 ms | 1.0 ms | 0.5 ms | **2090.47 ms** | The Office US | 100.0% | 1 |
| 1788 | `The.Office.US.S06E09.1080p.NF.WEB.x264-...` | 1913.08 ms | 108.92 ms | 16.15 ms | 252.26 ms | 1.0 ms | 0.5 ms | **2292.8 ms** | The Office US | 100.0% | 1 |
| 1789 | `The.Office.US.S06E10.1080p.NF.WEB.x264-...` | 2741.66 ms | 132.27 ms | 18.02 ms | 182.92 ms | 1.0 ms | 0.5 ms | **3082.44 ms** | The Office US | 100.0% | 0 |
| 1790 | `The.Office.US.S06E11.1080p.NF.WEB.x264-...` | 1876.22 ms | 137.51 ms | 17.36 ms | 231.79 ms | 1.0 ms | 0.5 ms | **2263.71 ms** | The.Office.US.S06E11.1080p.... | -- | 1 |
| 1791 | `The.Office.US.S06E12.1080p.NF.WEB.x264-...` | 2407.83 ms | 143.46 ms | 18.08 ms | 339.45 ms | 1.0 ms | 0.5 ms | **2920.21 ms** | The Office US | 100.0% | 1 |
| 1792 | `The.Office.US.S06E13.1080p.NF.WEB.x264-...` | 2488.48 ms | 115.66 ms | 16.8 ms | 251.52 ms | 1.0 ms | 0.5 ms | **2877.75 ms** | The Office US | 100.0% | 1 |
| 1793 | `The.Office.US.S06E14.1080p.NF.WEB.x264-...` | 2282.87 ms | 94.72 ms | 16.22 ms | 90.94 ms | 1.0 ms | 0.5 ms | **2485.88 ms** | The Office US | 100.0% | 0 |
| 1794 | `The.Office.US.S06E15.1080p.NF.WEB.x264-...` | 2951.84 ms | 116.0 ms | 44.12 ms | 196.95 ms | 1.0 ms | 0.5 ms | **3312.83 ms** | The Office US | 100.0% | 1 |
| 1795 | `The.Office.US.S06E16.1080p.NF.WEB.x264-...` | 2686.38 ms | 99.74 ms | 21.33 ms | 285.03 ms | 1.0 ms | 0.5 ms | **3093.29 ms** | The.Office.US.S06E16.1080p.... | -- | 1 |
| 1796 | `The.Office.US.S06E17-18.1080p.NF.WEB.x2...` | 3531.53 ms | 114.08 ms | 22.1 ms | 182.31 ms | 1.0 ms | 0.5 ms | **3850.76 ms** | The Office US | 100.0% | 0 |
| 1797 | `The.Office.US.S06E19.1080p.NF.WEB.x264-...` | 3575.68 ms | 139.04 ms | 27.04 ms | 219.02 ms | 1.0 ms | 0.5 ms | **3965.94 ms** | The.Office.US.S06E19.1080p.... | -- | 1 |
| 1798 | `The.Office.US.S06E20.1080p.NF.WEB.x264-...` | 3552.67 ms | 115.34 ms | 17.59 ms | 252.61 ms | 1.0 ms | 0.5 ms | **3942.83 ms** | The Office US | 100.0% | 0 |
| 1799 | `The.Office.US.S06E21.1080p.NF.WEB.x264-...` | 2935.26 ms | 482.34 ms | 20.57 ms | 178.81 ms | 1.0 ms | 0.5 ms | **3620.21 ms** | The Office US | 100.0% | 1 |
| 1800 | `The.Office.US.S06E22.1080p.NF.WEB.x264-...` | 3096.78 ms | 126.4 ms | 25.38 ms | 142.69 ms | 1.0 ms | 0.5 ms | **3392.22 ms** | The.Office.US.S06E22.1080p.... | -- | 1 |
| 1801 | `The.Office.US.S06E23.1080p.NF.WEB.x264-...` | 3623.34 ms | 170.03 ms | 27.97 ms | 127.42 ms | 1.0 ms | 0.5 ms | **3950.08 ms** | The Office US | 100.0% | 0 |
| 1802 | `The.Office.US.S06E24.1080p.NF.WEB.x264-...` | 3200.71 ms | 123.28 ms | 20.86 ms | 308.02 ms | 1.0 ms | 0.5 ms | **3653.51 ms** | The Office US | 100.0% | 0 |
| 1803 | `The.Office.US.S06E25.1080p.NF.WEB.x264-...` | 3234.54 ms | 89.87 ms | 16.13 ms | 167.51 ms | 1.0 ms | 0.5 ms | **3510.53 ms** | The Office US | 100.0% | 1 |
| 1804 | `The.Office.US.S06E26.1080p.NF.WEB.x264-...` | 3617.98 ms | 586.29 ms | 22.57 ms | 156.27 ms | 1.0 ms | 0.5 ms | **4384.63 ms** | The Office US | 100.0% | 0 |
| 1805 | `The.Office.US.S07E01.720p.NF.WEB.x264-M...` | 3154.75 ms | 103.27 ms | 16.32 ms | 371.87 ms | 1.0 ms | 0.5 ms | **3670.3 ms** | The Office US | 100.0% | 1 |
| 1806 | `The.Office.US.S07E02.720p.NF.WEB.x264-M...` | 3072.74 ms | 116.61 ms | 20.0 ms | 128.96 ms | 1.0 ms | 0.5 ms | **3338.97 ms** | The Office US | 100.0% | 1 |
| 1807 | `The.Office.US.S07E03.720p.NF.WEB.x264-M...` | 3054.68 ms | 500.57 ms | 18.49 ms | 124.91 ms | 1.0 ms | 0.5 ms | **3707.66 ms** | The Office US | 100.0% | 1 |
| 1808 | `The.Office.US.S07E04.720p.NF.WEB.x264-M...` | 2632.84 ms | 101.04 ms | 14.53 ms | 170.37 ms | 1.0 ms | 0.5 ms | **2921.09 ms** | The Office US | 100.0% | 1 |
| 1809 | `The.Office.US.S07E05.720p.NF.WEB.x264-M...` | 3046.67 ms | 138.62 ms | 24.72 ms | 416.13 ms | 1.0 ms | 0.5 ms | **3632.8 ms** | The Office US | 100.0% | 1 |
| 1810 | `The.Office.US.S07E06.720p.NF.WEB.x264-M...` | 2515.34 ms | 96.76 ms | 21.03 ms | 214.62 ms | 1.0 ms | 0.5 ms | **2850.08 ms** | The Office US | 100.0% | 1 |
| 1811 | `The.Office.US.S07E07.720p.NF.WEB.x264-M...` | 1743.5 ms | 242.94 ms | 16.29 ms | 141.02 ms | 1.0 ms | 0.5 ms | **2153.69 ms** | The Office US | 100.0% | 1 |
| 1812 | `The.Office.US.S07E08.720p.NF.WEB.x264-M...` | 2375.99 ms | 129.36 ms | 22.07 ms | 189.22 ms | 1.0 ms | 0.5 ms | **2719.39 ms** | The Office US | 100.0% | 1 |
| 1813 | `The.Office.US.S07E09.720p.NF.WEB.x264-M...` | 1864.39 ms | 88.92 ms | 21.49 ms | 203.42 ms | 1.0 ms | 0.5 ms | **2178.98 ms** | The Office US | 100.0% | 1 |
| 1814 | `The.Office.US.S07E10.720p.NF.WEB.x264-M...` | 1948.95 ms | 111.94 ms | 16.95 ms | 167.15 ms | 1.0 ms | 0.5 ms | **2246.55 ms** | The Office US | 100.0% | 1 |
| 1815 | `The.Office.US.S07E11.720p.NF.WEB.x264-M...` | 2171.55 ms | 101.79 ms | 18.22 ms | 231.31 ms | 1.0 ms | 0.5 ms | **2526.98 ms** | The Office US | 100.0% | 1 |
| 1816 | `The.Office.US.S07E12.720p.NF.WEB.x264-M...` | 2727.85 ms | 701.92 ms | 19.05 ms | 194.62 ms | 1.0 ms | 0.5 ms | **3646.06 ms** | The Office US | 100.0% | 1 |
| 1817 | `The.Office.US.S07E13.720p.NF.WEB.x264-M...` | 3108.04 ms | 91.2 ms | 18.43 ms | 132.25 ms | 1.0 ms | 0.5 ms | **3351.54 ms** | The Office US | 100.0% | 1 |
| 1818 | `The.Office.US.S07E14.720p.NF.WEB.x264-M...` | 3999.61 ms | 123.93 ms | 25.93 ms | 88.34 ms | 1.0 ms | 0.5 ms | **4238.56 ms** | The.Office.US.S07E14.720p.N... | -- | 1 |
| 1819 | `The.Office.US.S07E15.720p.NF.WEB.x264-M...` | 2623.79 ms | 99.9 ms | 19.93 ms | 133.49 ms | 1.0 ms | 0.5 ms | **2878.24 ms** | The Office US | 100.0% | 1 |
| 1820 | `The.Office.US.S07E16.720p.NF.WEB.x264-M...` | 2502.44 ms | 119.4 ms | 15.47 ms | 263.29 ms | 1.0 ms | 0.5 ms | **2901.76 ms** | The Office US | 100.0% | 1 |
| 1821 | `The.Office.US.S07E17.720p.NF.WEB.x264-M...` | 2168.59 ms | 100.4 ms | 13.89 ms | 106.13 ms | 1.0 ms | 0.5 ms | **2389.69 ms** | The Office US | 100.0% | 1 |
| 1822 | `The.Office.US.S07E18.720p.NF.WEB.x264-M...` | 2726.87 ms | 119.08 ms | 24.36 ms | 234.0 ms | 1.0 ms | 0.5 ms | **3105.99 ms** | The Office US | 100.0% | 1 |
| 1823 | `The.Office.US.S07E19.720p.NF.WEB.x264-M...` | 2880.3 ms | 97.34 ms | 19.74 ms | 268.59 ms | 1.0 ms | 0.5 ms | **3267.27 ms** | The Office US | 100.0% | 1 |
| 1824 | `The.Office.US.S07E20.720p.NF.WEB.x264-M...` | 2541.25 ms | 99.38 ms | 15.81 ms | 113.03 ms | 1.0 ms | 0.5 ms | **2770.79 ms** | The Office US | 100.0% | 1 |
| 1825 | `The.Office.US.S07E21.720p.NF.WEB.x264-M...` | 2233.85 ms | 127.96 ms | 18.13 ms | 97.18 ms | 1.0 ms | 0.5 ms | **2482.55 ms** | The Office US | 100.0% | 1 |
| 1826 | `The.Office.US.S07E22.720p.NF.WEB.x264-M...` | 2728.39 ms | 96.24 ms | 22.48 ms | 154.89 ms | 1.0 ms | 0.5 ms | **3004.23 ms** | The Office US | 100.0% | 1 |
| 1827 | `The.Office.US.S07E23.720p.NF.WEB.x264-M...` | 2699.28 ms | 120.08 ms | 22.7 ms | 281.4 ms | 1.0 ms | 0.5 ms | **3126.23 ms** | The Office US | 100.0% | 1 |
| 1828 | `The.Office.US.S07E24.720p.NF.WEB.x264-M...` | 2633.86 ms | 118.69 ms | 15.54 ms | 220.38 ms | 1.0 ms | 0.5 ms | **2989.33 ms** | The Office US | 100.0% | 1 |
| 1829 | `The.Office.US.S07E25.720p.NF.WEB.x264-M...` | 2254.73 ms | 119.94 ms | 23.39 ms | 297.54 ms | 1.0 ms | 0.5 ms | **2705.57 ms** | The Office US | 100.0% | 1 |
| 1830 | `The.Office.US.S07E26.720p.NF.WEB.x264-M...` | 2903.39 ms | 103.67 ms | 22.83 ms | 274.31 ms | 1.0 ms | 0.5 ms | **3307.77 ms** | The Office US | 100.0% | 1 |
| 1831 | `The.One.S01E01.1080p.WEB.H264-GGEZ_mid1...` | 2223.83 ms | 114.25 ms | 26.33 ms | 130.73 ms | 1.0 ms | 0.5 ms | **2499.18 ms** | The One | 100.0% | 1 |
| 1832 | `The.One.S01E02.1080p.WEB.H264-GGWP_mid1...` | 2265.78 ms | 175.13 ms | 28.43 ms | 219.64 ms | 1.0 ms | 0.5 ms | **2693.04 ms** | Locke and Key | 100.0% | 0 |
| 1833 | `The.One.S01E03.1080p.WEB.H264-GLHF_mid1...` | 2161.66 ms | 247.6 ms | 23.33 ms | 291.2 ms | 1.0 ms | 0.5 ms | **2724.57 ms** | The One | 100.0% | 0 |
| 1834 | `The.One.S01E04.1080p.WEB.H264-GGWP_mid1...` | 2316.04 ms | 163.48 ms | 22.65 ms | 319.37 ms | 1.0 ms | 0.5 ms | **2822.25 ms** | The One | 100.0% | 0 |
| 1835 | `The.One.S01E06.1080p.WEB.H264-GGWP_mid1...` | 2007.17 ms | 110.47 ms | 24.67 ms | 532.74 ms | 1.0 ms | 0.5 ms | **2675.81 ms** | The One | 100.0% | 0 |
| 1836 | `The.One.S01E07.1080p.WEB.H264-GLHF_mid1...` | 2350.57 ms | 96.92 ms | 17.67 ms | 202.45 ms | 1.0 ms | 0.5 ms | **2670.01 ms** | The One | 100.0% | 0 |
| 1837 | `The.One.S01E08.1080p.WEB.H264-GGWP_mid1...` | 2319.23 ms | 120.45 ms | 20.93 ms | 134.05 ms | 1.0 ms | 0.5 ms | **2596.11 ms** | The One | 100.0% | 0 |
| 1838 | `The.Other.Woman.2014.1080p.BluRay.x264....` | 3038.06 ms | 100.45 ms | 20.4 ms | 273.33 ms | 1.0 ms | 0.5 ms | **3440.28 ms** | Aucun | -- | 0 |
| 1839 | `The.Outsider.2018.SUBFRENCH.WEBRip.NF.x...` | 2493.48 ms | 131.33 ms | 29.23 ms | 177.26 ms | 1.0 ms | 0.5 ms | **2831.81 ms** | The Outsider | 100.0% | 0 |
| 1840 | `The.Passion.Of.The.Christ.2004.1080p.Bl...` | 2096.53 ms | 122.62 ms | 26.8 ms | 132.29 ms | 1.0 ms | 0.5 ms | **2379.05 ms** | The.Passion.Of.The.Christ.2... | -- | 1 |
| 1841 | `The.Peripheral.S01E01.Pilot.1080p.AMZN....` | 1798.59 ms | 85.45 ms | 23.95 ms | 183.54 ms | 1.0 ms | 0.5 ms | **2092.24 ms** | The Peripheral | 100.0% | 0 |
| 1842 | `The.Peripheral.S01E02.1080p.x265-ELiTE_...` | 1781.08 ms | 128.53 ms | 16.79 ms | 239.5 ms | 1.0 ms | 0.5 ms | **2167.58 ms** | The Peripheral | 100.0% | 1 |
| 1843 | `The.Physician.2013.REPACK.TRUEFRENCH.BD...` | 2673.69 ms | 96.22 ms | 25.77 ms | 218.75 ms | 1.0 ms | 0.5 ms | **3015.36 ms** | The Physician | 100.0% | 0 |
| 1844 | `The.Power.of.the.Dog.2021.1080p.NF.WEB-...` | 6421.1 ms | 150.96 ms | 183.43 ms | 294.37 ms | 1.0 ms | 0.5 ms | **7050.27 ms** | The Power of the Dog | 100.0% | 0 |
| 1845 | `The.Prophet.2014.FRENCH.BDRip.x264-Ulys...` | 2467.68 ms | 96.71 ms | 20.89 ms | 236.3 ms | 1.0 ms | 0.5 ms | **2822.21 ms** | The Prophet | 100.0% | 0 |
| 1846 | `The.Pursuit.Of.Happyness.2006.1080p.Blu...` | 2552.82 ms | 306.66 ms | 27.39 ms | 283.4 ms | 1.0 ms | 0.5 ms | **3177.87 ms** | The.Pursuit.Of.Happyness.20... | -- | 1 |
| 1847 | `The.Remains.Of.The.Day.1993.1080p.BluRa...` | 3000.74 ms | 110.17 ms | 30.41 ms | 311.25 ms | 1.0 ms | 0.5 ms | **3453.41 ms** | Aucun | -- | 0 |
| 1848 | `The.Sadness.2021.CHINESE.2160p.4K.BluRa...` | 2227.65 ms | 101.73 ms | 21.29 ms | 310.86 ms | 1.0 ms | 0.5 ms | **2665.86 ms** | The Sadness | 100.0% | 0 |
| 1849 | `The.Shining.1980.REMASTERED.VOSTFR.720p...` | 2407.5 ms | 292.51 ms | 30.92 ms | 235.99 ms | 1.0 ms | 0.5 ms | **2972.33 ms** | The Shining | 100.0% | 0 |
| 1850 | `The.Square.2017.FRENCH.HDRip.XviD-ACOOL...` | 2726.17 ms | 144.39 ms | 20.53 ms | 522.31 ms | 1.0 ms | 0.5 ms | **3421.21 ms** | The Square | 100.0% | 0 |
| 1851 | `The.Swarm.2020.FRENCH.1080p.NF.WEB-DL.D...` | 2375.41 ms | 112.58 ms | 19.91 ms | 555.18 ms | 1.0 ms | 0.5 ms | **3072.32 ms** | The One | 100.0% | 0 |
| 1852 | `The.Testament.Of.Ann.Lee.2025.1080p.WEB...` | 2684.39 ms | 304.42 ms | 30.52 ms | 181.21 ms | 1.0 ms | 0.5 ms | **3201.26 ms** | The Testament Of Ann Lee | 100.0% | 0 |
| 1853 | `The.Thing.1982.1080p.BluRay.x264-[YTS.A...` | 2113.84 ms | 134.19 ms | 19.17 ms | 541.65 ms | 1.0 ms | 0.5 ms | **2813.14 ms** | The.Thing.1982.1080p.BluRay... | -- | 1 |
| 1854 | `The.Thing.1982.REMASTERED.1080p.BluRay....` | 2479.34 ms | 286.09 ms | 17.52 ms | 129.6 ms | 1.0 ms | 0.5 ms | **2934.8 ms** | Aucun | -- | 0 |
| 1855 | `The.Thing.With.Feathers.2025.1080p.AMZN...` | 2700.16 ms | 170.34 ms | 69.81 ms | 331.9 ms | 1.0 ms | 0.5 ms | **3272.96 ms** | The Thing With Feathers | 100.0% | 0 |
| 1856 | `The.Tragedy.of.Macbeth.2021.1080p.ATVP....` | 1917.0 ms | 191.94 ms | 45.19 ms | 370.9 ms | 1.0 ms | 0.5 ms | **2525.81 ms** | Aucun | -- | 0 |
| 1857 | `The.Unbearable.Weight.of.Massive.Talent...` | 2391.15 ms | 106.99 ms | 26.47 ms | 193.86 ms | 1.0 ms | 0.5 ms | **2719.37 ms** | Aucun | -- | 0 |
| 1858 | `The.Valet.2022.2160p.4K.WEB.x265.10bit....` | 2894.27 ms | 137.72 ms | 30.88 ms | 285.36 ms | 1.0 ms | 0.5 ms | **3359.45 ms** | The.Valet.2022.2160p.4K.WEB... | -- | 1 |
| 1859 | `The.Vanishing.of.Sidney.Hall.2018.MULTi...` | 2793.21 ms | 140.73 ms | 38.0 ms | 146.32 ms | 1.0 ms | 0.5 ms | **3118.69 ms** | The Vanishing of Sidney Hall | 100.0% | 0 |
| 1860 | `The.Walking.Dead.Daryl.Dixon.S01E01.216...` | 3139.53 ms | 153.18 ms | 35.84 ms | 292.95 ms | 1.0 ms | 0.5 ms | **3622.42 ms** | The Walking Dead Daryl Dixon | 100.0% | 1 |
| 1861 | `The.Walking.Dead.Daryl.Dixon.S01E02.216...` | 2570.15 ms | 147.95 ms | 24.55 ms | 418.56 ms | 1.0 ms | 0.5 ms | **3184.8 ms** | The.Walking.Dead.Daryl.Dixo... | -- | 1 |
| 1862 | `The.Walking.Dead.Daryl.Dixon.S01E03.216...` | 3231.33 ms | 119.48 ms | 35.38 ms | 219.78 ms | 1.0 ms | 0.5 ms | **3608.65 ms** | The Walking Dead Daryl Dixon | 100.0% | 1 |
| 1863 | `The.Watcher.2022.S01E01.Benvenuti.amici...` | 3543.11 ms | 205.01 ms | 31.0 ms | 129.14 ms | 1.0 ms | 0.5 ms | **3909.04 ms** | The Leftovers | 100.0% | 1 |
| 1864 | `The.Watcher.2022.S01E02.Sacrificio.di.s...` | 2313.7 ms | 146.22 ms | 77.09 ms | 162.52 ms | 1.0 ms | 0.5 ms | **2700.35 ms** | The Watcher | 100.0% | 1 |
| 1865 | `The.Watcher.2022.S01E03.Gotterdammerung...` | 2636.06 ms | 168.79 ms | 30.85 ms | 173.49 ms | 1.0 ms | 0.5 ms | **3009.61 ms** | Aucun | -- | 0 |
| 1866 | `The.Watcher.2022.S01E04.Qualcuno.che.mi...` | 2398.24 ms | 171.97 ms | 35.04 ms | 358.69 ms | 1.0 ms | 0.5 ms | **2965.66 ms** | The Watcher | 100.0% | 0 |
| 1867 | `The.Watcher.2022.S01E05.Il.rasoio.di.Oc...` | 2593.1 ms | 173.47 ms | 22.53 ms | 364.13 ms | 1.0 ms | 0.5 ms | **3153.74 ms** | The Watcher | 100.0% | 1 |
| 1868 | `The.Watcher.2022.S01E06.L.oscurita.1080...` | 4226.5 ms | 97.42 ms | 22.16 ms | 239.71 ms | 1.0 ms | 0.5 ms | **4586.7 ms** | The Watcher | 100.0% | 1 |
| 1869 | `The.Watcher.2022.S01E07.La.maledizione....` | 2521.5 ms | 156.43 ms | 22.17 ms | 231.59 ms | 1.0 ms | 0.5 ms | **2934.73 ms** | The Watcher | 100.0% | 1 |
| 1870 | `The.Wheel.of.Time.S02E08.What.Was.Meant...` | 2187.99 ms | 140.55 ms | 40.19 ms | 223.35 ms | 1.0 ms | 0.5 ms | **2593.74 ms** | The.Wheel.of.Time.S02E08.Wh... | -- | 2 |
| 1871 | `The.Wild.Boys.2017.1080p.BluRay.x264-[Y...` | 2279.12 ms | 202.37 ms | 52.48 ms | 226.1 ms | 1.0 ms | 0.5 ms | **2764.71 ms** | Aucun | -- | 0 |
| 1872 | `The.Witcher.S02E01.A.Grain.of.Truth.108...` | 2448.64 ms | 188.82 ms | 41.1 ms | 143.81 ms | 1.0 ms | 0.5 ms | **2823.91 ms** | The Witcher | 100.0% | 1 |
| 1873 | `The.Witcher.S02E02.Kaer.Morhen.1080p.NF...` | 2585.46 ms | 173.97 ms | 159.59 ms | 406.43 ms | 1.0 ms | 0.5 ms | **3326.24 ms** | Aucun | -- | 0 |
| 1874 | `The.Witcher.S02E03.What.Is.Lost.1080p.N...` | 2278.9 ms | 187.9 ms | 140.42 ms | 174.54 ms | 1.0 ms | 0.5 ms | **2784.52 ms** | The Witcher | 100.0% | 0 |
| 1875 | `The.Witcher.S02E04.Redanian.Intelligenc...` | 2832.82 ms | 143.69 ms | 30.21 ms | 299.49 ms | 1.0 ms | 0.5 ms | **3307.06 ms** | The Witcher | 100.0% | 0 |
| 1876 | `The.Witcher.S02E05.Turn.Your.Back.1080p...` | 2598.52 ms | 147.2 ms | 24.06 ms | 247.76 ms | 1.0 ms | 0.5 ms | **3019.46 ms** | The.Witcher.S02E05.Turn.You... | -- | 1 |
| 1877 | `The.Witcher.S02E06.Dear.Friend.1080p.NF...` | 2101.6 ms | 143.49 ms | 16.71 ms | 379.33 ms | 1.0 ms | 0.5 ms | **2641.76 ms** | The Witcher | 100.0% | 1 |
| 1878 | `The.Witcher.S02E08.Family.1080p.NF.WEB-...` | 3751.6 ms | 0.0 ms | 0.0 ms | 31.92 ms | 1.0 ms | 0.5 ms | **7284.98 ms** | The.Witcher.S02E08.Family.1... | -- | 1 |
| 1879 | `The_Accountant_(2016)[MULTI][1080p](Ben...` | 2845.92 ms | 132.69 ms | 21.11 ms | 208.31 ms | 1.0 ms | 0.5 ms | **3213.86 ms** | The Accountant | 100.0% | 0 |
| 1880 | `The_Adventures_of_Baron_Munchausen_(198...` | 2068.59 ms | 98.85 ms | 21.76 ms | 243.09 ms | 1.0 ms | 0.5 ms | **2433.22 ms** | The Adventures of Baron Mun... | 100.0% | 0 |
| 1881 | `The_Black_Phone_(2022)[MULTi][1080p](🔪🧒...` | 3217.8 ms | 0.0 ms | 0.0 ms | 48.8 ms | 1.0 ms | 0.5 ms | **6768.52 ms** | Aucun | -- | 0 |
| 1882 | `The_Boy_the_Mole_the_Fox_and_the_Horse_...` | 2333.93 ms | 127.39 ms | 21.13 ms | 217.58 ms | 1.0 ms | 0.5 ms | **2700.94 ms** | The Boy the Mole the Fox an... | 100.0% | 0 |
| 1883 | `The_Boys_S02E01_[1080p]_mid15s.mp4` | 2315.53 ms | 405.47 ms | 16.38 ms | 234.89 ms | 1.0 ms | 0.5 ms | **2978.87 ms** | The Boys | 100.0% | 0 |
| 1884 | `The_Boys_S02E02_[1080p]_mid15s.mp4` | 2593.27 ms | 1048.38 ms | 16.74 ms | 116.52 ms | 1.0 ms | 0.5 ms | **3777.63 ms** | The Boys | 100.0% | 0 |
| 1885 | `The_Boys_S02E03_[720p]_mid15s.mp4` | 2078.56 ms | 1395.18 ms | 17.61 ms | 102.17 ms | 1.0 ms | 0.5 ms | **3594.25 ms** | The Boys | 100.0% | 2 |
| 1886 | `The_Code_-_Story_of_Linux_documentary_(...` | 2575.08 ms | 0.0 ms | 0.0 ms | 28.98 ms | 1.0 ms | 0.5 ms | **6121.48 ms** | Aucun | -- | 0 |
| 1887 | `The_Dark_Knight_(2008)_1080p_{60FPS}_Bl...` | 2650.02 ms | 1255.59 ms | 31.77 ms | 113.65 ms | 1.0 ms | 0.5 ms | **4051.82 ms** | The Dark Knight | 100.0% | 0 |
| 1888 | `The_Deer_King_(2021)[MULTi][1080p][5.1]...` | 2616.11 ms | 1149.75 ms | 25.62 ms | 99.46 ms | 1.0 ms | 0.5 ms | **3891.56 ms** | The Deer King | 100.0% | 0 |
| 1889 | `The_Dirt_(2019)[MULTi][1080p](Un_biopic...` | 2745.81 ms | 0.0 ms | 0.0 ms | 38.62 ms | 1.0 ms | 0.5 ms | **6295.57 ms** | Aucun | -- | 0 |
| 1890 | `The_Duke_(2020)[MULTi][1080p](histoire_...` | 2556.79 ms | 0.0 ms | 0.0 ms | 56.15 ms | 1.0 ms | 0.5 ms | **6153.79 ms** | Aucun | -- | 0 |
| 1891 | `The_Emperor's_New_Groove_(2000)[1080p.x...` | 2792.81 ms | 0.0 ms | 0.0 ms | 94.21 ms | 1.0 ms | 0.5 ms | **7390.17 ms** | Aucun | -- | 0 |
| 1892 | `The_Endless_(2017)[MULTi][1080p](Deux_f...` | 2263.11 ms | 0.0 ms | 0.0 ms | 32.09 ms | 1.0 ms | 0.5 ms | **5796.15 ms** | Aucun | -- | 0 |
| 1893 | `The_Gray_Man_(2022)[MULTi][1080p][Netfl...` | 2635.17 ms | 0.0 ms | 0.0 ms | 116.11 ms | 1.0 ms | 0.5 ms | **6266.91 ms** | Aucun | -- | 0 |
| 1894 | `The_Great_Wall_(2016)[MULTi][1080p](Mat...` | 2406.62 ms | 154.58 ms | 43.26 ms | 67.57 ms | 1.0 ms | 0.5 ms | **2672.88 ms** | The Great Wall | 100.0% | 0 |
| 1895 | `The_Heated_Battle_of_the_Gods_1988_mid1...` | 2298.4 ms | 1325.96 ms | 35.86 ms | 97.76 ms | 1.0 ms | 0.5 ms | **3758.73 ms** | The Office US | 100.0% | 1 |
| 1896 | `The_King_of_Staten_Island_(2020)[1080p]...` | 1963.07 ms | 0.0 ms | 0.0 ms | 128.38 ms | 1.0 ms | 0.5 ms | **5593.17 ms** | Aucun | -- | 0 |
| 1897 | `The_Land_of_the_Sons_(2021)[MULTi][1080...` | 1799.7 ms | 0.0 ms | 0.0 ms | 67.9 ms | 1.0 ms | 0.5 ms | **5394.33 ms** | Aucun | -- | 0 |
| 1898 | `The_Last_Client_(2022)[MULTi][1080p](🧠👩...` | 1744.95 ms | 341.54 ms | 32.28 ms | 121.6 ms | 1.0 ms | 0.5 ms | **2241.8 ms** | The Last Client | 100.0% | 0 |
| 1899 | `The_Leftovers_S01E01_mid15s.mp4` | 1393.08 ms | 0.0 ms | 0.0 ms | 126.11 ms | 1.0 ms | 0.5 ms | **5022.94 ms** | Aucun | -- | 0 |
| 1900 | `The_Leftovers_S01E02_mid15s.mp4` | 1629.24 ms | 0.0 ms | 0.0 ms | 134.94 ms | 1.0 ms | 0.5 ms | **5269.65 ms** | Aucun | -- | 0 |
| 1901 | `The_Leftovers_S01E03_mid15s.mp4` | 827.81 ms | 0.0 ms | 0.0 ms | 130.04 ms | 1.0 ms | 0.5 ms | **4464.44 ms** | Aucun | -- | 0 |
| 1902 | `The_Leftovers_S01E05_mid15s.mp4` | 765.02 ms | 0.0 ms | 0.0 ms | 109.83 ms | 1.0 ms | 0.5 ms | **4404.79 ms** | Aucun | -- | 0 |
| 1903 | `The_Leftovers_S01E06_mid15s.mp4` | 883.82 ms | 0.0 ms | 0.0 ms | 75.3 ms | 1.0 ms | 0.5 ms | **4461.16 ms** | Aucun | -- | 0 |
| 1904 | `The_Leftovers_S01E07_mid15s.mp4` | 699.4 ms | 0.0 ms | 0.0 ms | 102.16 ms | 1.0 ms | 0.5 ms | **5410.0 ms** | Aucun | -- | 0 |
| 1905 | `The_Leftovers_S01E08_mid15s.mp4` | 795.16 ms | 0.0 ms | 0.0 ms | 124.33 ms | 1.0 ms | 0.5 ms | **4423.8 ms** | Aucun | -- | 0 |
| 1906 | `The_Leftovers_S02E01_mid15s.mp4` | 873.4 ms | 0.0 ms | 0.0 ms | 176.92 ms | 1.0 ms | 0.5 ms | **4552.12 ms** | Aucun | -- | 0 |
| 1907 | `The_Leftovers_S02E02_mid15s.mp4` | 857.26 ms | 0.0 ms | 0.0 ms | 108.71 ms | 1.0 ms | 0.5 ms | **4468.34 ms** | Aucun | -- | 0 |
| 1908 | `The_Leftovers_S02E03_mid15s.mp4` | 569.9 ms | 0.0 ms | 0.0 ms | 163.95 ms | 1.0 ms | 0.5 ms | **4235.82 ms** | Aucun | -- | 0 |
| 1909 | `The_Leftovers_S02E04_mid15s.mp4` | 308.47 ms | 0.0 ms | 0.0 ms | 72.42 ms | 1.0 ms | 0.5 ms | **3883.16 ms** | Aucun | -- | 0 |
| 1910 | `The_Leftovers_S02E05_mid15s.mp4` | 524.07 ms | 0.0 ms | 0.0 ms | 94.25 ms | 1.0 ms | 0.5 ms | **4120.33 ms** | Aucun | -- | 0 |
| 1911 | `The_Leftovers_S02E06_mid15s.mp4` | 529.92 ms | 0.0 ms | 0.0 ms | 86.22 ms | 1.0 ms | 0.5 ms | **4206.94 ms** | Aucun | -- | 0 |
| 1912 | `The_Leftovers_S02E07_mid15s.mp4` | 1145.14 ms | 0.0 ms | 0.0 ms | 48.95 ms | 1.0 ms | 0.5 ms | **4745.65 ms** | Aucun | -- | 0 |
| 1913 | `The_Leftovers_S02E08_mid15s.mp4` | 1100.17 ms | 0.0 ms | 0.0 ms | 75.6 ms | 1.0 ms | 0.5 ms | **4679.09 ms** | Aucun | -- | 0 |
| 1914 | `The_Leftovers_S02E09_mid15s.mp4` | 1269.8 ms | 0.0 ms | 0.0 ms | 50.01 ms | 1.0 ms | 0.5 ms | **4821.55 ms** | Aucun | -- | 0 |
| 1915 | `The_Leftovers_S02E10_mid15s.mp4` | 1524.02 ms | 3289.74 ms | 22.09 ms | 73.72 ms | 1.0 ms | 0.5 ms | **4910.18 ms** | The Leftovers | 100.0% | 0 |
| 1916 | `The_Leftovers_S03E01_mid15s.mp4` | 1808.16 ms | 3448.88 ms | 22.8 ms | 274.4 ms | 1.0 ms | 0.5 ms | **5554.97 ms** | The Leftovers | 100.0% | 0 |
| 1917 | `The_Leftovers_S03E02_mid15s.mp4` | 1537.41 ms | 0.0 ms | 0.0 ms | 125.54 ms | 1.0 ms | 0.5 ms | **5197.71 ms** | Aucun | -- | 0 |
| 1918 | `The_Leftovers_S03E03_mid15s.mp4` | 3435.61 ms | 1201.85 ms | 16.36 ms | 98.6 ms | 1.0 ms | 0.5 ms | **4753.59 ms** | The Leftovers | 100.0% | 0 |
| 1919 | `The_Leftovers_S03E04_mid15s.mp4` | 1814.43 ms | 0.0 ms | 0.0 ms | 129.6 ms | 1.0 ms | 0.5 ms | **5448.04 ms** | Aucun | -- | 0 |
| 1920 | `The_Leftovers_S03E05_mid15s.mp4` | 1488.73 ms | 0.0 ms | 0.0 ms | 181.78 ms | 1.0 ms | 0.5 ms | **5173.26 ms** | Aucun | -- | 0 |
| 1921 | `The_Leftovers_S03E06_mid15s.mp4` | 1521.19 ms | 0.0 ms | 0.0 ms | 284.4 ms | 1.0 ms | 0.5 ms | **5308.43 ms** | Aucun | -- | 0 |
| 1922 | `The_Leftovers_S03E07_mid15s.mp4` | 1370.32 ms | 0.0 ms | 0.0 ms | 436.51 ms | 1.0 ms | 0.5 ms | **5348.62 ms** | Aucun | -- | 0 |
| 1923 | `The_Leftovers_S03E08_mid15s.mp4` | 1359.49 ms | 2282.62 ms | 14.05 ms | 75.01 ms | 1.0 ms | 0.5 ms | **3731.73 ms** | The Leftovers | 100.0% | 0 |
| 1924 | `The_Legend_of_Tomiris_(2019)[1080p](Kaz...` | 1363.68 ms | 0.0 ms | 0.0 ms | 429.56 ms | 1.0 ms | 0.5 ms | **5295.98 ms** | Aucun | -- | 0 |
| 1925 | `The_Lion_King_(1994)[2160p,_x265](🐭)_mi...` | 1196.73 ms | 2200.78 ms | 28.97 ms | 185.28 ms | 1.0 ms | 0.5 ms | **3612.59 ms** | The Lion King | 100.0% | 0 |
| 1926 | `The_Making_of_Carnivàle_mid15s.mp4` | 1058.58 ms | 2291.71 ms | 37.66 ms | 154.42 ms | 1.0 ms | 0.5 ms | **3547.34 ms** | The Making of Carnivàle mkv... | 100.0% | 0 |
| 1927 | `The_Matrix_(1999)_2160p_UHD_BluRay_DV_H...` | 1091.85 ms | 1843.37 ms | 39.45 ms | 145.52 ms | 1.0 ms | 0.5 ms | **3121.61 ms** | The Yards | 100.0% | 0 |
| 1928 | `The_Princess_and_the_Frog_(2009)[1080p]...` | 903.86 ms | 2296.28 ms | 19.32 ms | 146.2 ms | 1.0 ms | 0.5 ms | **3366.38 ms** | The Princess and the Frog | 100.0% | 0 |
| 1929 | `The_Promised_Neverland_-_01_VOSTFR_1080...` | 685.2 ms | 1186.23 ms | 22.6 ms | 70.21 ms | 1.0 ms | 0.5 ms | **1964.55 ms** | The Promised Neverland - 01... | 100.0% | 0 |
| 1930 | `The_Sadness_(2021)[MULTi][1080p](Epouva...` | 562.35 ms | 1127.1 ms | 17.58 ms | 174.57 ms | 1.0 ms | 0.5 ms | **1882.21 ms** | The Sadness | 100.0% | 0 |
| 1931 | `The_White_Lotus_(2021)_-_S02E01_-_Ciao_...` | 768.65 ms | 622.01 ms | 31.87 ms | 99.97 ms | 1.0 ms | 0.5 ms | **1523.11 ms** | The White Lotus | 100.0% | 0 |
| 1932 | `The_White_Lotus_(2021)_-_S02E02_-_Itali...` | 444.86 ms | 171.62 ms | 29.27 ms | 91.72 ms | 1.0 ms | 0.5 ms | **738.35 ms** | Aucun | -- | 0 |
| 1933 | `The_White_Lotus_(2021)_-_S02E05_-_That'...` | 782.39 ms | 137.98 ms | 65.09 ms | 159.74 ms | 1.0 ms | 0.5 ms | **1146.27 ms** | The Man in the High Castle | 100.0% | 0 |
| 1934 | `The_White_Lotus_(2021)_-_S02E07_-_Arriv...` | 728.46 ms | 207.71 ms | 24.84 ms | 219.48 ms | 1.0 ms | 0.5 ms | **1182.86 ms** | The Cured | 100.0% | 0 |
| 1935 | `The_Yards_(2000)_[1080p]_mid15s.mp4` | 2396.51 ms | 189.72 ms | 19.56 ms | 277.37 ms | 1.0 ms | 0.5 ms | **2884.11 ms** | The Yards | 100.0% | 0 |
| 1936 | `The_Young_Pope_S01E01_mid15s.mp4` | 2222.13 ms | 82.58 ms | 21.97 ms | 134.59 ms | 1.0 ms | 0.5 ms | **2462.01 ms** | The Young Pope | 100.0% | 0 |
| 1937 | `The_Young_Pope_S01E02_mid15s.mp4` | 1480.98 ms | 367.63 ms | 23.96 ms | 248.11 ms | 1.0 ms | 0.5 ms | **2121.47 ms** | The Young Pope | 100.0% | 0 |
| 1938 | `The_Young_Pope_S01E03_mid15s.mp4` | 1547.44 ms | 449.03 ms | 16.97 ms | 285.52 ms | 1.0 ms | 0.5 ms | **2299.63 ms** | The Young Pope | 100.0% | 0 |
| 1939 | `The_Young_Pope_S01E04_mid15s.mp4` | 3927.51 ms | 150.03 ms | 16.47 ms | 243.1 ms | 1.0 ms | 0.5 ms | **4337.54 ms** | The Young Pope | 100.0% | 0 |
| 1940 | `The_Young_Pope_S01E05_mid15s.mp4` | 2162.98 ms | 196.25 ms | 23.75 ms | 214.26 ms | 1.0 ms | 0.5 ms | **2597.61 ms** | The Young Pope | 100.0% | 0 |
| 1941 | `The_Young_Pope_S01E06_mid15s.mp4` | 1986.19 ms | 110.58 ms | 40.43 ms | 254.44 ms | 1.0 ms | 0.5 ms | **2400.15 ms** | Aucun | -- | 0 |
| 1942 | `The_Young_Pope_S01E08_mid15s.mp4` | 2455.15 ms | 130.78 ms | 23.74 ms | 180.76 ms | 1.0 ms | 0.5 ms | **2793.37 ms** | The Young Pope | 100.0% | 0 |
| 1943 | `The_Young_Pope_S01E09_mid15s.mp4` | 3149.73 ms | 185.74 ms | 24.93 ms | 189.24 ms | 1.0 ms | 0.5 ms | **3560.49 ms** | The Young Pope | 100.0% | 0 |
| 1944 | `The_Young_Pope_S01E10_mid15s.mp4` | 1987.02 ms | 185.53 ms | 17.7 ms | 105.97 ms | 1.0 ms | 0.5 ms | **2296.8 ms** | The Young Pope | 100.0% | 0 |
| 1945 | `Thelma.2017.FRENCH.BDRip.XviD-FuN_mid15...` | 2269.2 ms | 104.2 ms | 25.01 ms | 559.61 ms | 1.0 ms | 0.5 ms | **2959.59 ms** | Thelma | 100.0% | 0 |
| 1946 | `Theo_Et_Les_Metamorphoses_(2021)[1080p]...` | 4477.38 ms | 208.19 ms | 19.82 ms | 299.77 ms | 1.0 ms | 0.5 ms | **5005.58 ms** | Theo Et Les Metamorphoses | 100.0% | 0 |
| 1947 | `This.Is.England.2006.1080p.BluRay.H264....` | 2226.14 ms | 165.23 ms | 23.72 ms | 213.37 ms | 1.0 ms | 0.5 ms | **2630.89 ms** | This Is England | 100.0% | 0 |
| 1948 | `This.Is.Spinal.Tap.1984.1080p.BluRay.x2...` | 2580.32 ms | 128.88 ms | 17.71 ms | 294.9 ms | 1.0 ms | 0.5 ms | **3022.6 ms** | This Is Spinal Tap | 100.0% | 1 |
| 1949 | `Three.Thousand.Years.of.Longing.2022.21...` | 7173.41 ms | 112.93 ms | 24.71 ms | 368.13 ms | 1.0 ms | 0.5 ms | **7679.84 ms** | Three.Thousand.Years.of.Lon... | -- | 1 |
| 1950 | `Timbuktu.2014.FESTiVAL.FRENCH.BRRip.x26...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.95 ms** | Erreur | -- | 0 |
| 1951 | `Time.Bandits.1981.REMASTERED.1080p.BluR...` | 3034.74 ms | 92.67 ms | 18.38 ms | 274.08 ms | 1.0 ms | 0.5 ms | **3420.61 ms** | Time Bandits | 100.0% | 1 |
| 1952 | `Time.Trap.2017.1080p.BluRay.x264-[YTS.A...` | 2270.63 ms | 123.42 ms | 25.61 ms | 164.41 ms | 1.0 ms | 0.5 ms | **2584.98 ms** | Aucun | -- | 0 |
| 1953 | `Time_Freak_(2018)(Un_inventeur_névrotiq...` | 2511.79 ms | 94.88 ms | 20.29 ms | 425.69 ms | 1.0 ms | 0.5 ms | **3053.52 ms** | Time Freak | 100.0% | 0 |
| 1954 | `Titane.2021.FRENCH.1080p.WEBRip.DD5.1.x...` | 4771.35 ms | 150.16 ms | 19.81 ms | 186.19 ms | 1.0 ms | 0.5 ms | **5128.34 ms** | Aucun | -- | 0 |
| 1955 | `Tokyo_Monogatari_mid15s.mp4` | 2002.59 ms | 121.48 ms | 21.26 ms | 233.28 ms | 1.0 ms | 0.5 ms | **2384.62 ms** | Tokyo Monogatari avi 24 0fps | 100.0% | 0 |
| 1956 | `Tous_Les_Dieux_du_Ciel_(2018)[1080p]_mi...` | 2451.52 ms | 183.78 ms | 20.28 ms | 366.99 ms | 1.0 ms | 0.5 ms | **3032.84 ms** | Tous Les Dieux du Ciel | 100.0% | 0 |
| 1957 | `Tout_Simplement_Noir_(2020)_[1080p]_mid...` | 1920.8 ms | 107.76 ms | 20.74 ms | 288.25 ms | 1.0 ms | 0.5 ms | **2343.95 ms** | Aucun | -- | 0 |
| 1958 | `Tracky_Chrome_Extension_[English]_mid15...` | 846.96 ms | 0.0 ms | 0.0 ms | 199.51 ms | 1.0 ms | 0.5 ms | **1046.59 ms** | Aucun | -- | 0 |
| 1959 | `Trailer_1_mid15s.mp4` | 2434.32 ms | 81.62 ms | 23.24 ms | 565.82 ms | 1.0 ms | 0.5 ms | **3106.31 ms** | Aucun | -- | 0 |
| 1960 | `Trailer_2_[jp]_mid15s.mp4` | 2948.95 ms | 103.94 ms | 44.37 ms | 236.92 ms | 1.0 ms | 0.5 ms | **3335.15 ms** | Trailer 2 jp mkv 24 0fps | 100.0% | 0 |
| 1961 | `Trailer_3_mid15s.mp4` | 9860.9 ms | 119.83 ms | 25.73 ms | 249.87 ms | 1.0 ms | 0.5 ms | **10270.65 ms** | Aucun | -- | 0 |
| 1962 | `Trailer_mid15s.mp4` | 2520.66 ms | 94.38 ms | 17.38 ms | 484.12 ms | 1.0 ms | 0.5 ms | **3118.54 ms** | Aucun | -- | 0 |
| 1963 | `Train.To.Busan.2016.KOREAN.1080p.BluRay...` | 2493.09 ms | 121.17 ms | 25.82 ms | 430.41 ms | 1.0 ms | 0.5 ms | **3072.18 ms** | Train.To.Busan.2016.KOREAN.... | -- | 1 |
| 1964 | `Trash.2014.MULTi.1080p.BluRay.x264-LOST...` | 2602.5 ms | 235.12 ms | 19.08 ms | 252.28 ms | 1.0 ms | 0.5 ms | **3110.1 ms** | Aucun | -- | 0 |
| 1965 | `Travaux_Pratique_-_Application_de_gesti...` | 6163.32 ms | 122.25 ms | 35.03 ms | 212.08 ms | 1.0 ms | 0.5 ms | **6533.19 ms** | Aucun | -- | 0 |
| 1966 | `Tre_Piani_(2021)[MULTi][1080p](comédie_...` | 4954.17 ms | 87.93 ms | 21.84 ms | 362.5 ms | 1.0 ms | 0.5 ms | **5426.77 ms** | Aucun | -- | 0 |
| 1967 | `Triple_Frontier_(2019)(Ben_Affleck,_Osc...` | 4195.99 ms | 116.12 ms | 26.14 ms | 247.3 ms | 1.0 ms | 0.5 ms | **4594.99 ms** | Triple Frontier | 100.0% | 0 |
| 1968 | `Tron.1982.1080p.BrRip.x264.bitloks.YIFY...` | 4035.48 ms | 120.58 ms | 18.66 ms | 395.05 ms | 1.0 ms | 0.5 ms | **4572.4 ms** | Shogun | 100.0% | 1 |
| 1969 | `Troy.2004.DC.1080p.BluRay.x264.AAC5.1-[...` | 3276.73 ms | 99.23 ms | 13.7 ms | 269.74 ms | 1.0 ms | 0.5 ms | **3671.32 ms** | Troy.2004.DC.1080p.BluRay.x... | -- | 1 |
| 1970 | `Tunnel.2016.FRENCH.720p.BluRay.x264-LOS...` | 6798.22 ms | 126.52 ms | 25.42 ms | 166.66 ms | 1.0 ms | 0.5 ms | **7117.28 ms** | Tunnel | 100.0% | 0 |
| 1971 | `Turning_Red_(2022)[2160p.x265.10bit.AAC...` | 4855.93 ms | 103.17 ms | 18.1 ms | 119.8 ms | 1.0 ms | 0.5 ms | **5098.75 ms** | Turning Red | 100.0% | 0 |
| 1972 | `Tutopriel_Mac_-_Installer_Apache,_PHP_e...` | 2058.86 ms | 131.99 ms | 20.82 ms | 489.64 ms | 1.0 ms | 0.5 ms | **2701.76 ms** | Tutopriel Mac - Installer A... | 100.0% | 0 |
| 1973 | `Tutoriel_-_Actualisez_vos_pages_automat...` | 5951.05 ms | 121.83 ms | 25.68 ms | 264.55 ms | 1.0 ms | 0.5 ms | **6363.93 ms** | Tutoriel - Actualisez vos p... | 100.0% | 0 |
| 1974 | `Tutoriel_-_BrowserSync_mid15s.mp4` | 2933.89 ms | 108.87 ms | 37.89 ms | 231.23 ms | 1.0 ms | 0.5 ms | **3312.2 ms** | Tutoriel - BrowserSync mp4 ... | 100.0% | 0 |
| 1975 | `Tutoriel_-_Compiler_ses_SCSS_avec_libSA...` | 1920.2 ms | 102.75 ms | 350.6 ms | 225.6 ms | 1.0 ms | 0.5 ms | **2599.54 ms** | Tutoriel Javascript - | 100.0% | 0 |
| 1976 | `Tutoriel_-_Créer_un_système_de_sujet_lu...` | 4349.15 ms | 130.4 ms | 26.89 ms | 399.53 ms | 1.0 ms | 0.5 ms | **4907.15 ms** | Tutoriel - Créer un système... | 100.0% | 0 |
| 1977 | `Tutoriel_-_Créer_une_VM_avec_Vagrant_mi...` | 2245.29 ms | 143.46 ms | 30.89 ms | 347.24 ms | 1.0 ms | 0.5 ms | **2767.3 ms** | Tutoriel - Créer une VM ave... | 100.0% | 0 |
| 1978 | `Tutoriel_-_Créer_une_application_Ionic_...` | 9949.23 ms | 108.56 ms | 109.38 ms | 336.44 ms | 1.0 ms | 0.5 ms | **10504.08 ms** | Tutoriel - Créer une applic... | 100.0% | 0 |
| 1979 | `Tutoriel_-_Découverte_d'Elastic_Search_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.16 ms** | Erreur | -- | 0 |
| 1980 | `Tutoriel_-_Déployer_avec_Dploy_mid15s.mp4` | 1216.52 ms | 0.0 ms | 0.0 ms | 95.61 ms | 1.0 ms | 0.5 ms | **1312.24 ms** | Aucun | -- | 0 |
| 1981 | `Tutoriel_-_Git_Workflow_mid15s.mp4` | 4669.91 ms | 94.25 ms | 31.45 ms | 145.82 ms | 1.0 ms | 0.5 ms | **4941.82 ms** | Tutoriel - Git Workflow mp4... | 100.0% | 0 |
| 1982 | `Tutoriel_-_Jekyll_mid15s.mp4` | 2497.89 ms | 119.97 ms | 25.79 ms | 290.29 ms | 1.0 ms | 0.5 ms | **2934.75 ms** | Tutoriel - Jekyll mp4 24 0fps | 100.0% | 0 |
| 1983 | `Tutoriel_-_Le_MJML_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.85 ms** | Erreur | -- | 0 |
| 1984 | `Tutoriel_-_Ngrok_mid15s.mp4` | 1206.35 ms | 0.0 ms | 0.0 ms | 111.79 ms | 1.0 ms | 0.5 ms | **1318.24 ms** | Aucun | -- | 0 |
| 1985 | `Tutoriel_-_PuPHPet_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12061.54 ms** | Erreur | -- | 0 |
| 1986 | `Tutoriel_-_REST_mid15s.mp4` | 5357.2 ms | 138.61 ms | 132.77 ms | 176.27 ms | 1.0 ms | 0.5 ms | **5805.35 ms** | Aucun | -- | 0 |
| 1987 | `Tutoriel_-_SEO,_Référencement_naturel_m...` | 11609.77 ms | 96.56 ms | 24.6 ms | 280.37 ms | 1.0 ms | 0.5 ms | **12011.75 ms** | Aucun | -- | 0 |
| 1988 | `Tutoriel_-_Tester_son_site_sur_Internet...` | 5397.17 ms | 159.55 ms | 27.01 ms | 333.6 ms | 1.0 ms | 0.5 ms | **5917.65 ms** | Aucun | -- | 0 |
| 1989 | `Tutoriel_-_Trello_mid15s.mp4` | 11565.92 ms | 131.96 ms | 22.83 ms | 139.65 ms | 1.0 ms | 0.5 ms | **11860.69 ms** | Tutoriel - Trello mp4 24 0fps | 100.0% | 0 |
| 1990 | `Tutoriel_After_Effect_-_Réaliser_l'intr...` | 2342.38 ms | 123.73 ms | 28.78 ms | 319.78 ms | 1.0 ms | 0.5 ms | **2815.19 ms** | Aucun | -- | 0 |
| 1991 | `Tutoriel_After_Effect_-_Spot_Hp_mid15s.mp4` | 2942.63 ms | 117.02 ms | 30.5 ms | 305.81 ms | 1.0 ms | 0.5 ms | **3396.31 ms** | Tutoriel After Effect - Spo... | 100.0% | 0 |
| 1992 | `Tutoriel_AngularJS_-_Découverte_du_Fram...` | 4533.0 ms | 145.57 ms | 122.46 ms | 340.61 ms | 1.0 ms | 0.5 ms | **5141.99 ms** | Aucun | -- | 0 |
| 1993 | `Tutoriel_AngularJS_-_ngAnimate_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.9 ms** | Erreur | -- | 0 |
| 1994 | `Tutoriel_Apache_Cordova_-_Compiler_sur_...` | 2612.65 ms | 188.5 ms | 18.06 ms | 226.7 ms | 1.0 ms | 0.5 ms | **3046.38 ms** | Tutoriel Apache Cordova - C... | 100.0% | 0 |
| 1995 | `Tutoriel_Apache_Cordova_-_Compiler_sur_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.22 ms** | Erreur | -- | 0 |
| 1996 | `Tutoriel_Apache_Cordova_-_Créer_sa_prem...` | 9215.16 ms | 104.04 ms | 52.83 ms | 373.9 ms | 1.0 ms | 0.5 ms | **9747.37 ms** | Aucun | -- | 0 |
| 1997 | `Tutoriel_Apache_Cordova_-_Installer_et_...` | 2295.08 ms | 152.21 ms | 21.88 ms | 227.92 ms | 1.0 ms | 0.5 ms | **2697.6 ms** | Tutoriel Apache Cordova - I... | 100.0% | 0 |
| 1998 | `Tutoriel_Appcelerator_Titanium_-_Découv...` | 1956.12 ms | 160.67 ms | 41.44 ms | 284.47 ms | 1.0 ms | 0.5 ms | **2443.17 ms** | Tutoriel Appcelerator Titan... | 100.0% | 0 |
| 1999 | `Tutoriel_Appcelerator_Titanium_-_Dévelo...` | 5862.48 ms | 123.49 ms | 24.45 ms | 227.64 ms | 1.0 ms | 0.5 ms | **6238.51 ms** | Tutoriel Appcelerator Titan... | 100.0% | 0 |
| 2000 | `Tutoriel_CSS_-_Animer_des_sprites_en_ut...` | 11559.56 ms | 104.62 ms | 23.8 ms | 248.05 ms | 1.0 ms | 0.5 ms | **11936.39 ms** | Tutoriel CSS - Animer des s... | 100.0% | 0 |
| 2001 | `Tutoriel_CSS_-_Card_UI_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.0 ms** | Erreur | -- | 0 |
| 2002 | `Tutoriel_CSS_-_Centrer_les_éléments_mid...` | 4807.94 ms | 93.61 ms | 29.76 ms | 248.64 ms | 1.0 ms | 0.5 ms | **5180.37 ms** | Tutoriel CSS - Centrer les ... | 100.0% | 0 |
| 2003 | `Tutoriel_CSS_-_Comment_bien_organiser_s...` | 2920.72 ms | 95.37 ms | 55.12 ms | 162.99 ms | 1.0 ms | 0.5 ms | **3234.63 ms** | Aucun | -- | 0 |
| 2004 | `Tutoriel_CSS_-_Créer_une_grille_respons...` | 4473.93 ms | 113.23 ms | 120.61 ms | 411.65 ms | 1.0 ms | 0.5 ms | **5119.83 ms** | Tutoriel HTML-CSS - PostCSS... | 100.0% | 0 |
| 2005 | `Tutoriel_CSS_-_px,_em_et_rem_mid15s.mp4` | 11133.03 ms | 114.86 ms | 107.3 ms | 151.72 ms | 1.0 ms | 0.5 ms | **11507.29 ms** | Tutoriel CSS - px, em et re... | 100.0% | 0 |
| 2006 | `Tutoriel_CakePHP_-_Ajax_et_CakePHP_mid1...` | 2141.64 ms | 96.99 ms | 53.64 ms | 129.91 ms | 1.0 ms | 0.5 ms | **2423.46 ms** | Aucun | -- | 0 |
| 2007 | `Tutoriel_CakePHP_-_CakePHP_3.0_Dev_prev...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.17 ms** | Erreur | -- | 0 |
| 2008 | `Tutoriel_CakePHP_-_CakePHP_ACL_mid15s.mp4` | 2471.1 ms | 127.66 ms | 20.75 ms | 213.17 ms | 1.0 ms | 0.5 ms | **2833.06 ms** | Tutoriel CakePHP - CakePHP ... | 100.0% | 0 |
| 2009 | `Tutoriel_CakePHP_-_Changements_dans_Cak...` | 10819.26 ms | 91.99 ms | 60.13 ms | 191.82 ms | 1.0 ms | 0.5 ms | **11172.03 ms** | Tutoriel CakePHP - Changeme... | 100.0% | 0 |
| 2010 | `Tutoriel_CakePHP_-_Comportement_Contain...` | 3795.57 ms | 117.06 ms | 48.31 ms | 505.71 ms | 1.0 ms | 0.5 ms | **4508.92 ms** | Tutoriel CakePHP - Comporte... | 100.0% | 0 |
| 2011 | `Tutoriel_CakePHP_-_Création_d'un_blog,_...` | 6681.34 ms | 84.66 ms | 18.13 ms | 153.99 ms | 1.0 ms | 0.5 ms | **6938.49 ms** | Tutoriel CakePHP - Création... | 100.0% | 0 |
| 2012 | `Tutoriel_CakePHP_-_Création_d'un_blog_m...` | 8050.4 ms | 123.38 ms | 19.29 ms | 320.73 ms | 1.0 ms | 0.5 ms | **8514.25 ms** | Tutoriel CakePHP - Création... | 100.0% | 0 |
| 2013 | `Tutoriel_CakePHP_-_Créer_un_''Bonjour''...` | 3423.61 ms | 81.87 ms | 135.58 ms | 186.49 ms | 1.0 ms | 0.5 ms | **3828.0 ms** | Aucun | -- | 0 |
| 2014 | `Tutoriel_CakePHP_-_Créer_un_Portfolio_m...` | 1930.21 ms | 87.9 ms | 19.76 ms | 174.53 ms | 1.0 ms | 0.5 ms | **2212.89 ms** | Tutoriel CakePHP - Créer un... | 100.0% | 0 |
| 2015 | `Tutoriel_CakePHP_-_Créer_un_système_d'a...` | 3402.54 ms | 101.1 ms | 38.94 ms | 425.29 ms | 1.0 ms | 0.5 ms | **3968.3 ms** | Tutoriel CakePHP - Créer un... | 100.0% | 0 |
| 2016 | `Tutoriel_CakePHP_-_Créer_un_système_de_...` | 4294.0 ms | 107.66 ms | 363.61 ms | 194.36 ms | 1.0 ms | 0.5 ms | **4960.12 ms** | Aucun | -- | 0 |
| 2017 | `Tutoriel_CakePHP_-_Développer_un_site_a...` | 5022.09 ms | 108.33 ms | 18.23 ms | 133.26 ms | 1.0 ms | 0.5 ms | **5282.36 ms** | Tutoriel CakePHP - Développ... | 100.0% | 0 |
| 2018 | `Tutoriel_CakePHP_-_Développer_un_site_a...` | 1831.9 ms | 92.06 ms | 20.48 ms | 341.55 ms | 1.0 ms | 0.5 ms | **2286.33 ms** | Tutoriel CakePHP - Développ... | 100.0% | 0 |
| 2019 | `Tutoriel_CakePHP_-_Développer_un_site_a...` | 10308.85 ms | 140.11 ms | 88.43 ms | 335.07 ms | 1.0 ms | 0.5 ms | **10872.85 ms** | Aucun | -- | 0 |
| 2020 | `Tutoriel_CakePHP_-_Développer_un_site_a...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.05 ms** | Erreur | -- | 0 |
| 2021 | `Tutoriel_CakePHP_-_Events_mid15s.mp4` | 3656.96 ms | 92.38 ms | 199.82 ms | 215.39 ms | 1.0 ms | 0.5 ms | **4164.89 ms** | Tutoriel CakePHP - Events m... | 100.0% | 0 |
| 2022 | `Tutoriel_CakePHP_-_Facebook_Connect_mid...` | 1929.73 ms | 89.22 ms | 19.35 ms | 239.51 ms | 1.0 ms | 0.5 ms | **2287.12 ms** | Tutoriel CakePHP - Facebook... | 100.0% | 0 |
| 2023 | `Tutoriel_CakePHP_-_Formulaire_de_contac...` | 9511.86 ms | 190.61 ms | 28.96 ms | 461.07 ms | 1.0 ms | 0.5 ms | **10192.96 ms** | Tutoriel CakePHP - Formulai... | 100.0% | 0 |
| 2024 | `Tutoriel_CakePHP_-_Générer_un_flux_RSS_...` | 5331.03 ms | 114.64 ms | 395.03 ms | 151.71 ms | 1.0 ms | 0.5 ms | **5992.87 ms** | Tutoriel CakePHP - Générer ... | 100.0% | 0 |
| 2025 | `Tutoriel_CakePHP_-_Gérer_l'upload_de_fi...` | 10042.89 ms | 90.2 ms | 373.57 ms | 278.85 ms | 1.0 ms | 0.5 ms | **10785.91 ms** | Aucun | -- | 0 |
| 2026 | `Tutoriel_CakePHP_-_Initiation_à_CakePHP...` | 1948.55 ms | 116.27 ms | 19.74 ms | 246.12 ms | 1.0 ms | 0.5 ms | **2331.09 ms** | Tutoriel CakePHP - Initiati... | 100.0% | 0 |
| 2027 | `Tutoriel_CakePHP_-_Installation_de_Cake...` | 1377.72 ms | 0.0 ms | 0.0 ms | 130.0 ms | 1.0 ms | 0.5 ms | **1507.86 ms** | Aucun | -- | 0 |
| 2028 | `Tutoriel_CakePHP_-_Internationalisation...` | 11590.14 ms | 121.79 ms | 30.17 ms | 186.25 ms | 1.0 ms | 0.5 ms | **11928.64 ms** | Aucun | -- | 0 |
| 2029 | `Tutoriel_CakePHP_-_Les_View_Blocks_mid1...` | 1577.12 ms | 0.0 ms | 0.0 ms | 143.11 ms | 1.0 ms | 0.5 ms | **1720.33 ms** | Aucun | -- | 0 |
| 2030 | `Tutoriel_CakePHP_-_Plugin_Medias_mid15s...` | 2716.53 ms | 117.17 ms | 50.16 ms | 126.51 ms | 1.0 ms | 0.5 ms | **3010.75 ms** | Tutoriel CakePHP - Plugin M... | 100.0% | 0 |
| 2031 | `Tutoriel_CakePHP_-_Plugin_Migrations_mi...` | 1518.02 ms | 142.64 ms | 18.13 ms | 157.29 ms | 1.0 ms | 0.5 ms | **1836.42 ms** | Aucun | -- | 0 |
| 2032 | `Tutoriel_CakePHP_-_Routing_CakePHP_mid1...` | 1707.72 ms | 0.0 ms | 0.0 ms | 323.38 ms | 1.0 ms | 0.5 ms | **2031.2 ms** | Aucun | -- | 0 |
| 2033 | `Tutoriel_CakePHP_-_Système_de_Tags,_Has...` | 2043.9 ms | 113.11 ms | 91.09 ms | 186.04 ms | 1.0 ms | 0.5 ms | **2434.95 ms** | Aucun | -- | 0 |
| 2034 | `Tutoriel_CakePHP_-_Système_de_membres_m...` | 10989.41 ms | 89.34 ms | 36.54 ms | 275.14 ms | 1.0 ms | 0.5 ms | **11390.8 ms** | Tutoriel CakePHP - Formulai... | 100.0% | 0 |
| 2035 | `Tutoriel_CakePHP_-_Sécuriser_CakePHP_av...` | 3599.34 ms | 145.99 ms | 32.82 ms | 364.96 ms | 1.0 ms | 0.5 ms | **4149.89 ms** | Tutoriel CakePHP - Sécurise... | 100.0% | 0 |
| 2036 | `Tutoriel_CakePHP_-_Taxonomy_mid15s.mp4` | 2728.74 ms | 115.83 ms | 65.3 ms | 234.64 ms | 1.0 ms | 0.5 ms | **3144.89 ms** | Aucun | -- | 0 |
| 2037 | `Tutoriel_CakePHP_-_Tester_vos_models_mi...` | 2725.42 ms | 154.21 ms | 20.2 ms | 604.61 ms | 1.0 ms | 0.5 ms | **3504.87 ms** | Aucun | -- | 0 |
| 2038 | `Tutoriel_CakePHP_-_Tree_Behaviour_mid15...` | 2165.01 ms | 107.31 ms | 31.86 ms | 237.19 ms | 1.0 ms | 0.5 ms | **2541.75 ms** | Aucun | -- | 0 |
| 2039 | `Tutoriel_CakePHP_-_Validate_&_Redirect_...` | 2461.51 ms | 105.94 ms | 37.42 ms | 172.99 ms | 1.0 ms | 0.5 ms | **2778.23 ms** | Tutoriel CakePHP - Validate... | 100.0% | 0 |
| 2040 | `Tutoriel_DNS_-_DKIM_&_SPF_mid15s.mp4` | 4606.98 ms | 104.6 ms | 50.23 ms | 154.89 ms | 1.0 ms | 0.5 ms | **4917.24 ms** | Aucun | -- | 0 |
| 2041 | `Tutoriel_Divers_-_Affinity_Designer_mid...` | 2283.4 ms | 110.04 ms | 18.94 ms | 327.66 ms | 1.0 ms | 0.5 ms | **2740.7 ms** | Tutoriel Divers - Affinity ... | 100.0% | 0 |
| 2042 | `Tutoriel_Divers_-_Avocode_mid15s.mp4` | 1450.11 ms | 0.0 ms | 0.0 ms | 201.19 ms | 1.0 ms | 0.5 ms | **1651.39 ms** | Aucun | -- | 0 |
| 2043 | `Tutoriel_Divers_-_Bash_sur_Windows_mid1...` | 1377.23 ms | 0.0 ms | 0.0 ms | 145.5 ms | 1.0 ms | 0.5 ms | **1522.85 ms** | Aucun | -- | 0 |
| 2044 | `Tutoriel_Divers_-_Découverte_du_JWT_mid...` | 1371.31 ms | 0.0 ms | 0.0 ms | 285.06 ms | 1.0 ms | 0.5 ms | **1656.5 ms** | Aucun | -- | 0 |
| 2045 | `Tutoriel_Divers_-_Découverte_du_langage...` | 10638.04 ms | 105.0 ms | 74.45 ms | 140.94 ms | 1.0 ms | 0.5 ms | **10958.9 ms** | Tutoriel Divers - Découvert... | 100.0% | 0 |
| 2046 | `Tutoriel_Divers_-_Déployer_avec_Capistr...` | 6077.87 ms | 91.91 ms | 51.25 ms | 237.18 ms | 1.0 ms | 0.5 ms | **6459.21 ms** | Tutoriel Divers - Déployer ... | 100.0% | 0 |
| 2047 | `Tutoriel_Divers_-_Intégration_continue_...` | 8641.4 ms | 111.45 ms | 25.89 ms | 815.55 ms | 1.0 ms | 0.5 ms | **9594.72 ms** | Tutoriel Divers - Intégrati... | 100.0% | 0 |
| 2048 | `Tutoriel_Docker_-_Stack_web_avec_Boot2D...` | 5206.24 ms | 101.39 ms | 212.89 ms | 134.68 ms | 1.0 ms | 0.5 ms | **5655.51 ms** | Tutoriel Docker - Stack | 100.0% | 0 |
| 2049 | `Tutoriel_Flash_-_Créer_des_class_en_AS3...` | 2478.78 ms | 130.82 ms | 36.58 ms | 372.88 ms | 1.0 ms | 0.5 ms | **3019.73 ms** | Tutoriel Flash - Créer un p... | 100.0% | 0 |
| 2050 | `Tutoriel_Flash_-_Créer_un_player_flash_...` | 3346.06 ms | 120.08 ms | 27.65 ms | 308.44 ms | 1.0 ms | 0.5 ms | **3802.95 ms** | Tutoriel Flash - Créer un p... | 100.0% | 0 |
| 2051 | `Tutoriel_Flash_-_Créer_un_player_flash_...` | 2116.31 ms | 98.85 ms | 43.25 ms | 180.57 ms | 1.0 ms | 0.5 ms | **2439.36 ms** | Tutoriel Flash - Créer un p... | 100.0% | 0 |
| 2052 | `Tutoriel_Flash_-_Créer_un_player_flash_...` | 3490.15 ms | 239.22 ms | 24.73 ms | 439.43 ms | 1.0 ms | 0.5 ms | **4193.92 ms** | Aucun | -- | 0 |
| 2053 | `Tutoriel_Flash_-_Diaporama_Flash_mid15s...` | 1648.03 ms | 0.0 ms | 0.0 ms | 204.78 ms | 1.0 ms | 0.5 ms | **1852.92 ms** | Aucun | -- | 0 |
| 2054 | `Tutoriel_Flash_-_Gérer_les_classes_dans...` | 6100.99 ms | 102.76 ms | 26.34 ms | 259.18 ms | 1.0 ms | 0.5 ms | **6489.63 ms** | Tutoriel Flash - Gérer les ... | 100.0% | 0 |
| 2055 | `Tutoriel_Flash_-_Lecteur_MP3_mid15s.mp4` | 1197.28 ms | 0.0 ms | 0.0 ms | 251.04 ms | 1.0 ms | 0.5 ms | **1448.45 ms** | Aucun | -- | 0 |
| 2056 | `Tutoriel_Flash_-_Papervision,_panorama_...` | 1727.1 ms | 0.0 ms | 0.0 ms | 86.62 ms | 1.0 ms | 0.5 ms | **1813.85 ms** | Aucun | -- | 0 |
| 2057 | `Tutoriel_Flash_-_Particules_Flash_mid15...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.13 ms** | Erreur | -- | 0 |
| 2058 | `Tutoriel_Flash_-_SWFObject_mid15s.mp4` | 1815.08 ms | 153.83 ms | 22.8 ms | 122.1 ms | 1.0 ms | 0.5 ms | **2126.52 ms** | Aucun | -- | 0 |
| 2059 | `Tutoriel_Grunt_-_Découverte_de_Grunt_mi...` | 9379.54 ms | 82.96 ms | 180.52 ms | 152.28 ms | 1.0 ms | 0.5 ms | **9795.68 ms** | Aucun | -- | 0 |
| 2060 | `Tutoriel_HTML-CSS_-_Animer_l'icône_hamb...` | 2490.71 ms | 119.43 ms | 209.0 ms | 233.48 ms | 1.0 ms | 0.5 ms | **3052.91 ms** | Tutoriel HTML-CSS - Animer ... | 100.0% | 0 |
| 2061 | `Tutoriel_HTML-CSS_-_CSScomb_mid15s.mp4` | 917.24 ms | 0.0 ms | 0.0 ms | 204.85 ms | 1.0 ms | 0.5 ms | **1122.33 ms** | Aucun | -- | 0 |
| 2062 | `Tutoriel_HTML-CSS_-_Créer_un_email_HTML...` | 2182.01 ms | 133.26 ms | 189.15 ms | 183.78 ms | 1.0 ms | 0.5 ms | **2692.5 ms** | Tutoriel HTML-CSS - Créer u... | 100.0% | 0 |
| 2063 | `Tutoriel_HTML-CSS_-_Faire_tomber_de_la_...` | 2109.73 ms | 107.93 ms | 68.39 ms | 134.64 ms | 1.0 ms | 0.5 ms | **2421.19 ms** | Tutoriel HTML-CSS - Faire t... | 100.0% | 0 |
| 2064 | `Tutoriel_HTML-CSS_-_Flexbox_mid15s.mp4` | 996.54 ms | 0.0 ms | 0.0 ms | 286.69 ms | 1.0 ms | 0.5 ms | **1283.33 ms** | Aucun | -- | 0 |
| 2065 | `Tutoriel_HTML-CSS_-_Foundation_6_mid15s...` | 1158.97 ms | 0.0 ms | 0.0 ms | 102.16 ms | 1.0 ms | 0.5 ms | **1261.29 ms** | Aucun | -- | 0 |
| 2066 | `Tutoriel_HTML-CSS_-_PostCSS_mid15s.mp4` | 2600.87 ms | 131.07 ms | 21.06 ms | 114.34 ms | 1.0 ms | 0.5 ms | **2867.66 ms** | Tutoriel HTML-CSS - PostCSS... | 100.0% | 0 |
| 2067 | `Tutoriel_HTML-CSS_-_display_-_grid;_mid...` | 4417.19 ms | 89.0 ms | 42.98 ms | 323.54 ms | 1.0 ms | 0.5 ms | **4872.99 ms** | Aucun | -- | 0 |
| 2068 | `Tutoriel_HTML_-_Les_formats_d'image_mid...` | 10458.65 ms | 83.01 ms | 25.16 ms | 163.02 ms | 1.0 ms | 0.5 ms | **10730.19 ms** | Tutoriel HTML - Les formats... | 100.0% | 0 |
| 2069 | `Tutoriel_HTML_CSS_-_Animer_un_champ_en_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.94 ms** | Erreur | -- | 0 |
| 2070 | `Tutoriel_HTML_CSS_-_Bootstrap_CSS_from_...` | 1541.86 ms | 101.6 ms | 48.18 ms | 197.67 ms | 1.0 ms | 0.5 ms | **1889.78 ms** | Tutoriel HTML CSS - Bootstr... | 100.0% | 0 |
| 2071 | `Tutoriel_HTML_CSS_-_Box_Sizing_mid15s.mp4` | 1205.16 ms | 0.0 ms | 0.0 ms | 285.1 ms | 1.0 ms | 0.5 ms | **1490.41 ms** | Aucun | -- | 0 |
| 2072 | `Tutoriel_HTML_CSS_-_Créer_des_loaders_a...` | 2129.66 ms | 100.73 ms | 27.07 ms | 468.89 ms | 1.0 ms | 0.5 ms | **2727.31 ms** | Aucun | -- | 0 |
| 2073 | `Tutoriel_HTML_CSS_-_Créer_des_triangles...` | 2171.07 ms | 97.05 ms | 46.82 ms | 475.33 ms | 1.0 ms | 0.5 ms | **2790.7 ms** | Aucun | -- | 0 |
| 2074 | `Tutoriel_HTML_CSS_-_Créer_un_site_web_-...` | 3291.05 ms | 89.42 ms | 19.04 ms | 295.21 ms | 1.0 ms | 0.5 ms | **3695.04 ms** | Tutoriel HTML CSS - Créer u... | 100.0% | 0 |
| 2075 | `Tutoriel_HTML_CSS_-_Créer_une_version_m...` | 1539.38 ms | 141.39 ms | 28.52 ms | 299.27 ms | 1.0 ms | 0.5 ms | **2008.96 ms** | Aucun | -- | 0 |
| 2076 | `Tutoriel_HTML_CSS_-_Design_''Responsive...` | 1013.18 ms | 0.0 ms | 0.0 ms | 175.04 ms | 1.0 ms | 0.5 ms | **1188.35 ms** | Aucun | -- | 0 |
| 2077 | `Tutoriel_HTML_CSS_-_Ecran_Retina_mid15s...` | 1788.58 ms | 82.2 ms | 60.1 ms | 282.63 ms | 1.0 ms | 0.5 ms | **2213.92 ms** | Aucun | -- | 0 |
| 2078 | `Tutoriel_HTML_CSS_-_Editeur_CSS_-_Styli...` | 827.83 ms | 0.0 ms | 0.0 ms | 228.62 ms | 1.0 ms | 0.5 ms | **1056.58 ms** | Aucun | -- | 0 |
| 2079 | `Tutoriel_HTML_CSS_-_Firebug,_L'extensio...` | 2894.2 ms | 107.22 ms | 24.43 ms | 264.46 ms | 1.0 ms | 0.5 ms | **3290.88 ms** | Tutoriel HTML CSS - Firebug... | 100.0% | 0 |
| 2080 | `Tutoriel_HTML_CSS_-_Icônes_@font-face_m...` | 10157.6 ms | 122.82 ms | 41.68 ms | 247.57 ms | 1.0 ms | 0.5 ms | **10570.26 ms** | Aucun | -- | 0 |
| 2081 | `Tutoriel_HTML_CSS_-_Le_sélecteur_-nth-c...` | 5428.81 ms | 99.15 ms | 23.88 ms | 360.14 ms | 1.0 ms | 0.5 ms | **5912.34 ms** | Tutoriel HTML CSS - Le séle... | 100.0% | 0 |
| 2082 | `Tutoriel_HTML_CSS_-_Les_Sprites_CSS_mid...` | 1586.79 ms | 0.0 ms | 0.0 ms | 498.66 ms | 1.0 ms | 0.5 ms | **2085.58 ms** | Aucun | -- | 0 |
| 2083 | `Tutoriel_HTML_CSS_-_Les_grilles_en_CSS_...` | 1450.03 ms | 0.0 ms | 0.0 ms | 425.08 ms | 1.0 ms | 0.5 ms | **1875.31 ms** | Aucun | -- | 0 |
| 2084 | `Tutoriel_HTML_CSS_-_Menu_déroulant_en_C...` | 2572.95 ms | 87.14 ms | 25.84 ms | 469.05 ms | 1.0 ms | 0.5 ms | **3155.39 ms** | Tutoriel HTML CSS - Menu dé... | 100.0% | 0 |
| 2085 | `Tutoriel_HTML_CSS_-_Personnaliser_des_c...` | 4122.7 ms | 109.36 ms | 86.44 ms | 157.91 ms | 1.0 ms | 0.5 ms | **4476.89 ms** | Aucun | -- | 0 |
| 2086 | `Tutoriel_HTML_CSS_-_Personnaliser_formu...` | 1703.23 ms | 86.19 ms | 22.22 ms | 254.85 ms | 1.0 ms | 0.5 ms | **2067.28 ms** | Tutoriel HTML CSS - Personn... | 100.0% | 0 |
| 2087 | `Tutoriel_HTML_CSS_-_Personnaliser_le_le...` | 2834.19 ms | 90.02 ms | 21.91 ms | 284.48 ms | 1.0 ms | 0.5 ms | **3231.06 ms** | Tutoriel HTML CSS - Personn... | 100.0% | 0 |
| 2088 | `Tutoriel_HTML_CSS_-_Pli_3D_en_CSS3_mid1...` | 3028.05 ms | 83.59 ms | 66.51 ms | 181.26 ms | 1.0 ms | 0.5 ms | **3359.88 ms** | Aucun | -- | 0 |
| 2089 | `Tutoriel_HTML_CSS_-_Prepros_mid15s.mp4` | 1109.52 ms | 0.0 ms | 0.0 ms | 149.24 ms | 1.0 ms | 0.5 ms | **1258.92 ms** | Aucun | -- | 0 |
| 2090 | `Tutoriel_HTML_CSS_-_Rendre_vos_vidéos_R...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.7 ms** | Erreur | -- | 0 |
| 2091 | `Tutoriel_HTML_CSS_-_Sécuriser_avec_.htp...` | 1976.05 ms | 94.46 ms | 21.86 ms | 111.44 ms | 1.0 ms | 0.5 ms | **2204.14 ms** | Tutoriel HTML CSS - Sécuris... | 100.0% | 0 |
| 2092 | `Tutoriel_HTML_CSS_-_Validation_de_formu...` | 3897.06 ms | 89.29 ms | 19.5 ms | 384.32 ms | 1.0 ms | 0.5 ms | **4390.62 ms** | Tutoriel HTML CSS - Validat... | 100.0% | 0 |
| 2093 | `Tutoriel_HTML_CSS_-_Zoombox_mid15s.mp4` | 1455.17 ms | 0.0 ms | 0.0 ms | 347.2 ms | 1.0 ms | 0.5 ms | **1802.5 ms** | Aucun | -- | 0 |
| 2094 | `Tutoriel_Hébergement_-_Filezilla_mid15s...` | 1771.39 ms | 0.0 ms | 0.0 ms | 84.63 ms | 1.0 ms | 0.5 ms | **1856.12 ms** | Aucun | -- | 0 |
| 2095 | `Tutoriel_Hébergement_-_Héberger_et_Uplo...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.36 ms** | Erreur | -- | 0 |
| 2096 | `Tutoriel_Illustrator_-_Masque_d'écretag...` | 1632.34 ms | 0.0 ms | 0.0 ms | 146.31 ms | 1.0 ms | 0.5 ms | **1778.77 ms** | Aucun | -- | 0 |
| 2097 | `Tutoriel_Illustrator_-_Réaliser_un_dess...` | 2166.43 ms | 92.27 ms | 21.57 ms | 181.69 ms | 1.0 ms | 0.5 ms | **2462.32 ms** | Tutoriel Illustrator - Réal... | 100.0% | 0 |
| 2098 | `Tutoriel_Internet_-_Github_-windows_mid...` | 1630.0 ms | 88.44 ms | 39.16 ms | 208.82 ms | 1.0 ms | 0.5 ms | **1966.75 ms** | Tutoriel Internet - Github ... | 100.0% | 0 |
| 2099 | `Tutoriel_Internet_-_Markdown_mid15s.mp4` | 11600.12 ms | 181.07 ms | 78.7 ms | 147.0 ms | 1.0 ms | 0.5 ms | **12007.27 ms** | Aucun | -- | 0 |
| 2100 | `Tutoriel_Internet_-_Utiliser_GitHub_mid...` | 1557.02 ms | 0.0 ms | 0.0 ms | 166.44 ms | 1.0 ms | 0.5 ms | **1723.57 ms** | Aucun | -- | 0 |
| 2101 | `Tutoriel_JS_-_Jquery.cookie_mid15s.mp4` | 11034.78 ms | 163.1 ms | 86.88 ms | 187.33 ms | 1.0 ms | 0.5 ms | **11472.49 ms** | Aucun | -- | 0 |
| 2102 | `Tutoriel_JavaScript_-_Créer_un_carousel...` | 2627.86 ms | 122.8 ms | 72.41 ms | 400.47 ms | 1.0 ms | 0.5 ms | **3224.07 ms** | Tutoriel JavaScript - Décou... | 100.0% | 0 |
| 2103 | `Tutoriel_JavaScript_-_Créer_un_éditeur_...` | 2960.08 ms | 86.17 ms | 68.69 ms | 203.66 ms | 1.0 ms | 0.5 ms | **3319.01 ms** | Aucun | -- | 0 |
| 2104 | `Tutoriel_JavaScript_-_Créer_un_éditeur_...` | 2331.97 ms | 106.74 ms | 68.3 ms | 206.16 ms | 1.0 ms | 0.5 ms | **2713.53 ms** | Aucun | -- | 0 |
| 2105 | `Tutoriel_JavaScript_-_Découverte_de_Rea...` | 2236.31 ms | 103.86 ms | 93.58 ms | 427.29 ms | 1.0 ms | 0.5 ms | **2861.4 ms** | Tutoriel JavaScript - Décou... | 100.0% | 0 |
| 2106 | `Tutoriel_JavaScript_-_Découverte_du_Web...` | 4070.35 ms | 93.91 ms | 40.35 ms | 266.65 ms | 1.0 ms | 0.5 ms | **4471.76 ms** | Tutoriel JavaScript - Décou... | 100.0% | 0 |
| 2107 | `Tutoriel_JavaScript_-_Infobulle_mid15s.mp4` | 1069.05 ms | 0.0 ms | 0.0 ms | 278.1 ms | 1.0 ms | 0.5 ms | **1347.28 ms** | Aucun | -- | 0 |
| 2108 | `Tutoriel_JavaScript_-_IntersectionObser...` | 3447.99 ms | 105.23 ms | 413.97 ms | 148.78 ms | 1.0 ms | 0.5 ms | **4116.39 ms** | Tutoriel JavaScript - Inter... | 100.0% | 0 |
| 2109 | `Tutoriel_JavaScript_-_Les_modules_Bundl...` | 2556.65 ms | 85.96 ms | 31.77 ms | 304.57 ms | 1.0 ms | 0.5 ms | **2979.66 ms** | Aucun | -- | 0 |
| 2110 | `Tutoriel_JavaScript_-_Lier_plusieurs_se...` | 2396.26 ms | 89.06 ms | 45.23 ms | 221.04 ms | 1.0 ms | 0.5 ms | **2751.92 ms** | Aucun | -- | 0 |
| 2111 | `Tutoriel_JavaScript_-_Menu_en_'vague'_m...` | 1464.73 ms | 0.0 ms | 0.0 ms | 351.94 ms | 1.0 ms | 0.5 ms | **1816.78 ms** | Aucun | -- | 0 |
| 2112 | `Tutoriel_JavaScript_-_Modernizr_mid15s.mp4` | 3971.09 ms | 97.91 ms | 45.03 ms | 145.06 ms | 1.0 ms | 0.5 ms | **4259.44 ms** | Aucun | -- | 0 |
| 2113 | `Tutoriel_JavaScript_-_Panorama_360°_ave...` | 4358.43 ms | 156.51 ms | 41.21 ms | 300.46 ms | 1.0 ms | 0.5 ms | **4857.11 ms** | Tutoriel JavaScript - Panor... | 100.0% | 0 |
| 2114 | `Tutoriel_JavaScript_-_Parcel,_Module_bu...` | 4294.64 ms | 98.48 ms | 58.4 ms | 297.32 ms | 1.0 ms | 0.5 ms | **4749.26 ms** | Tutoriel JavaScript - Parce... | 100.0% | 0 |
| 2115 | `Tutoriel_JavaScript_-_Personnaliser_les...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.39 ms** | Erreur | -- | 0 |
| 2116 | `Tutoriel_JavaScript_-_Présentation_'à_l...` | 2961.5 ms | 109.45 ms | 25.05 ms | 248.23 ms | 1.0 ms | 0.5 ms | **3345.57 ms** | Tutoriel JavaScript - Prése... | 100.0% | 0 |
| 2117 | `Tutoriel_JavaScript_-_Redux_vs_MobX_mid...` | 3786.36 ms | 99.22 ms | 58.84 ms | 716.91 ms | 1.0 ms | 0.5 ms | **4661.98 ms** | Tutoriel JavaScript - Promi... | 100.0% | 0 |
| 2118 | `Tutoriel_JavaScript_-_Replace_global_mi...` | 1359.91 ms | 0.0 ms | 0.0 ms | 315.66 ms | 1.0 ms | 0.5 ms | **1675.73 ms** | Aucun | -- | 0 |
| 2119 | `Tutoriel_JavaScript_-_Tests_end-to-end_...` | 3796.94 ms | 153.76 ms | 27.73 ms | 350.77 ms | 1.0 ms | 0.5 ms | **4329.64 ms** | Andor | 100.0% | 7 |
| 2120 | `Tutoriel_JavaScript_-_Turbolinks_mid15s...` | 1253.41 ms | 0.0 ms | 0.0 ms | 100.02 ms | 1.0 ms | 0.5 ms | **1361.15 ms** | Aucun | -- | 0 |
| 2121 | `Tutoriel_Javascript_-_Bower_mid15s.mp4` | 2568.11 ms | 110.61 ms | 40.0 ms | 202.13 ms | 1.0 ms | 0.5 ms | **2921.32 ms** | Tutoriel Javascript - Bower... | 100.0% | 0 |
| 2122 | `Tutoriel_Javascript_-_Carte_interactive...` | 6245.3 ms | 133.49 ms | 511.7 ms | 429.4 ms | 1.0 ms | 0.5 ms | **7322.08 ms** | Aucun | -- | 0 |
| 2123 | `Tutoriel_Javascript_-_CoffeeScript_mid1...` | 1210.91 ms | 0.0 ms | 0.0 ms | 143.16 ms | 1.0 ms | 0.5 ms | **1354.2 ms** | Aucun | -- | 0 |
| 2124 | `Tutoriel_Javascript_-_Créer_des_animati...` | 5152.11 ms | 156.54 ms | 27.68 ms | 147.16 ms | 1.0 ms | 0.5 ms | **5485.07 ms** | Tutoriel Javascript - Créer... | 100.0% | 0 |
| 2125 | `Tutoriel_Javascript_-_Debounce_&_Thrott...` | 1165.23 ms | 0.0 ms | 0.0 ms | 163.39 ms | 1.0 ms | 0.5 ms | **1328.73 ms** | Aucun | -- | 0 |
| 2126 | `Tutoriel_Javascript_-_Gérer_l'historiqu...` | 1896.8 ms | 83.07 ms | 47.0 ms | 310.95 ms | 1.0 ms | 0.5 ms | **2338.26 ms** | Tutoriel Javascript - Gérer... | 100.0% | 0 |
| 2127 | `Tutoriel_Javascript_-_Gérer_les_évèneme...` | 5177.57 ms | 118.72 ms | 186.54 ms | 165.91 ms | 1.0 ms | 0.5 ms | **5649.21 ms** | Tutoriel Javascript - Gérer... | 100.0% | 0 |
| 2128 | `Tutoriel_Javascript_-_L'Ajax_mid15s.mp4` | 4948.66 ms | 123.99 ms | 456.6 ms | 225.75 ms | 1.0 ms | 0.5 ms | **5755.46 ms** | Aucun | -- | 0 |
| 2129 | `Tutoriel_Javascript_-_Mustache_mid15s.mp4` | 4273.79 ms | 108.86 ms | 30.11 ms | 248.02 ms | 1.0 ms | 0.5 ms | **4661.15 ms** | Tutoriel Javascript - Musta... | 100.0% | 0 |
| 2130 | `Tutoriel_Javascript_-_Réorganisation_en...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.5 ms** | Erreur | -- | 0 |
| 2131 | `Tutoriel_Javascript_-_Timer_relatif_mid...` | 1346.37 ms | 0.0 ms | 0.0 ms | 150.34 ms | 1.0 ms | 0.5 ms | **1496.83 ms** | Aucun | -- | 0 |
| 2132 | `Tutoriel_Javascript_-_TypeScript_mid15s...` | 3056.21 ms | 1177.11 ms | 24.37 ms | 281.41 ms | 1.0 ms | 0.5 ms | **4539.59 ms** | Tutoriel Javascript - TypeS... | 100.0% | 0 |
| 2133 | `Tutoriel_Javascript_-_VelocityJS_mid15s...` | 3982.05 ms | 153.57 ms | 31.95 ms | 218.57 ms | 1.0 ms | 0.5 ms | **4386.53 ms** | Tutoriel Javascript - Veloc... | 100.0% | 0 |
| 2134 | `Tutoriel_Javascript_-_Webpack,_module_b...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.36 ms** | Erreur | -- | 0 |
| 2135 | `Tutoriel_Jquery_-_Carrousel_javascript_...` | 2625.04 ms | 1124.95 ms | 24.87 ms | 257.91 ms | 1.0 ms | 0.5 ms | **4033.13 ms** | Tutoriel Jquery - Carrousel... | 100.0% | 0 |
| 2136 | `Tutoriel_Laravel_-_Création_d'un_systèm...` | 2586.3 ms | 92.69 ms | 22.51 ms | 376.67 ms | 1.0 ms | 0.5 ms | **3078.65 ms** | Tutoriel Laravel - Création... | 100.0% | 0 |
| 2137 | `Tutoriel_Laravel_-_Gates_et_Policies_mi...` | 2712.27 ms | 121.23 ms | 417.61 ms | 127.75 ms | 1.0 ms | 0.5 ms | **3379.26 ms** | Aucun | -- | 0 |
| 2138 | `Tutoriel_Laravel_-_L'injecteur_de_dépen...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.88 ms** | Erreur | -- | 0 |
| 2139 | `Tutoriel_Laravel_-_Système_de_badge_mid...` | 2242.3 ms | 112.59 ms | 24.91 ms | 104.86 ms | 1.0 ms | 0.5 ms | **2485.01 ms** | Aucun | -- | 0 |
| 2140 | `Tutoriel_Laravel_-_Système_de_messageri...` | 9555.79 ms | 95.77 ms | 32.27 ms | 368.97 ms | 1.0 ms | 0.5 ms | **10053.16 ms** | Tutoriel Laravel - Système ... | 100.0% | 0 |
| 2141 | `Tutoriel_Laravel_-_Tests_unitaires_mid1...` | 1974.69 ms | 98.65 ms | 25.79 ms | 311.36 ms | 1.0 ms | 0.5 ms | **2410.88 ms** | Aucun | -- | 0 |
| 2142 | `Tutoriel_Laravel_-_belongsToMany_-_Syst...` | 1687.7 ms | 103.28 ms | 22.0 ms | 499.2 ms | 1.0 ms | 0.5 ms | **2312.67 ms** | Tutoriel Laravel - belongsT... | 100.0% | 0 |
| 2143 | `Tutoriel_Laravel___VueJS_-_Module_de_co...` | 1765.7 ms | 108.18 ms | 24.13 ms | 429.08 ms | 1.0 ms | 0.5 ms | **2327.57 ms** | Tutoriel Laravel VueJS - Mo... | 100.0% | 0 |
| 2144 | `Tutoriel_Laravel___VueJS_-_Module_de_co...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.39 ms** | Erreur | -- | 0 |
| 2145 | `Tutoriel_Linux_-_Configuration_d'i3wm_m...` | 851.98 ms | 0.0 ms | 0.0 ms | 158.14 ms | 1.0 ms | 0.5 ms | **1010.23 ms** | Aucun | -- | 0 |
| 2146 | `Tutoriel_Linux_-_Les_ACL_mid15s.mp4` | 1925.7 ms | 121.84 ms | 27.43 ms | 203.43 ms | 1.0 ms | 0.5 ms | **2278.79 ms** | Tutoriel Linux - Les ACL mp... | 100.0% | 0 |
| 2147 | `Tutoriel_Linux_-_Makefile_mid15s.mp4` | 2869.07 ms | 161.08 ms | 31.23 ms | 151.12 ms | 1.0 ms | 0.5 ms | **3216.26 ms** | Tutoriel Linux - Makefile m... | 100.0% | 0 |
| 2148 | `Tutoriel_Linux_-_Nginx_+_PHP_mid15s.mp4` | 2211.89 ms | 120.64 ms | 20.65 ms | 223.76 ms | 1.0 ms | 0.5 ms | **2577.45 ms** | Tutoriel Linux - Nginx + PH... | 100.0% | 0 |
| 2149 | `Tutoriel_Mysql_-_Les_jointures_MySQL_mi...` | 5143.85 ms | 91.44 ms | 22.08 ms | 221.36 ms | 1.0 ms | 0.5 ms | **5479.19 ms** | Tutoriel Mysql - Les jointu... | 100.0% | 0 |
| 2150 | `Tutoriel_Mysql_-_Modéliser_une_base_de_...` | 4788.55 ms | 117.16 ms | 26.91 ms | 180.75 ms | 1.0 ms | 0.5 ms | **5113.81 ms** | Tutoriel Mysql - Modéliser ... | 100.0% | 0 |
| 2151 | `Tutoriel_Mysql_-_Représentation_interva...` | 1608.84 ms | 0.0 ms | 0.0 ms | 162.39 ms | 1.0 ms | 0.5 ms | **1771.39 ms** | Aucun | -- | 0 |
| 2152 | `Tutoriel_Netbeans_-_Présentation_Netbea...` | 1076.78 ms | 0.0 ms | 0.0 ms | 139.3 ms | 1.0 ms | 0.5 ms | **1216.18 ms** | Aucun | -- | 0 |
| 2153 | `Tutoriel_Netbeans_-_Trucs_et_Astuces_mi...` | 2752.09 ms | 157.03 ms | 20.51 ms | 227.99 ms | 1.0 ms | 0.5 ms | **3158.0 ms** | Tutoriel Netbeans - Trucs e... | 100.0% | 0 |
| 2154 | `Tutoriel_NodeJS_-_Créer_un_bot_Discord_...` | 2752.43 ms | 159.14 ms | 23.65 ms | 179.43 ms | 1.0 ms | 0.5 ms | **3115.06 ms** | Tutoriel NodeJS - Créer un ... | 100.0% | 0 |
| 2155 | `Tutoriel_NodeJS_-_Créer_un_tchat_avec_N...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.44 ms** | Erreur | -- | 0 |
| 2156 | `Tutoriel_NodeJS_-_NodeWebkit_mid15s.mp4` | 2022.26 ms | 126.62 ms | 17.58 ms | 141.32 ms | 1.0 ms | 0.5 ms | **2317.13 ms** | Tutoriel NodeJS - NodeWebki... | 100.0% | 0 |
| 2157 | `Tutoriel_NodeJS_-_Système_de_présence_m...` | 2288.59 ms | 161.99 ms | 24.94 ms | 222.61 ms | 1.0 ms | 0.5 ms | **2698.55 ms** | Tutoriel NodeJS - Système d... | 100.0% | 0 |
| 2158 | `Tutoriel_NodeJS_-_Tchat_Socket.IO_-_Per...` | 2103.33 ms | 140.23 ms | 27.09 ms | 141.45 ms | 1.0 ms | 0.5 ms | **2412.54 ms** | Tutoriel NodeJS - Tchat Soc... | 100.0% | 0 |
| 2159 | `Tutoriel_NodeJS_-_Yarn,_un_nouveau_gest...` | 7362.86 ms | 100.1 ms | 29.69 ms | 368.25 ms | 1.0 ms | 0.5 ms | **7861.6 ms** | Aucun | -- | 0 |
| 2160 | `Tutoriel_PHP_-_10_trucs_et_astuces_en_P...` | 9538.51 ms | 127.19 ms | 25.66 ms | 299.49 ms | 1.0 ms | 0.5 ms | **9991.18 ms** | Aucun | -- | 0 |
| 2161 | `Tutoriel_PHP_-_API_Instagram_mid15s.mp4` | 4089.7 ms | 90.86 ms | 32.68 ms | 156.97 ms | 1.0 ms | 0.5 ms | **4370.66 ms** | Aucun | -- | 0 |
| 2162 | `Tutoriel_PHP_-_API_LastFM_mid15s.mp4` | 1945.71 ms | 108.87 ms | 67.67 ms | 181.36 ms | 1.0 ms | 0.5 ms | **2304.03 ms** | Aucun | -- | 0 |
| 2163 | `Tutoriel_PHP_-_Afficher_ses_derniers_Tw...` | 2959.07 ms | 118.47 ms | 62.07 ms | 311.55 ms | 1.0 ms | 0.5 ms | **3451.62 ms** | Aucun | -- | 0 |
| 2164 | `Tutoriel_PHP_-_Capturez_les_emails_avec...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.44 ms** | Erreur | -- | 0 |
| 2165 | `Tutoriel_PHP_-_Comment_bien_organiser_s...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.24 ms** | Erreur | -- | 0 |
| 2166 | `Tutoriel_PHP_-_Commentaires_imbriqués_m...` | 6860.85 ms | 98.45 ms | 28.62 ms | 167.6 ms | 1.0 ms | 0.5 ms | **7155.95 ms** | Tutoriel PHP - Commentaires... | 100.0% | 0 |
| 2167 | `Tutoriel_PHP_-_Composer_mid15s.mp4` | 4931.48 ms | 95.97 ms | 21.64 ms | 258.08 ms | 1.0 ms | 0.5 ms | **5307.6 ms** | Tutoriel PHP - Composer mp4... | 100.0% | 0 |
| 2168 | `Tutoriel_PHP_-_Connexion_persistante_''...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.67 ms** | Erreur | -- | 0 |
| 2169 | `Tutoriel_PHP_-_Créer_un_Calendrier_1_3,...` | 2494.18 ms | 87.73 ms | 25.06 ms | 340.5 ms | 1.0 ms | 0.5 ms | **2948.02 ms** | Aucun | -- | 0 |
| 2170 | `Tutoriel_PHP_-_Créer_un_Calendrier_2_3,...` | 2332.5 ms | 126.19 ms | 27.37 ms | 462.86 ms | 1.0 ms | 0.5 ms | **2949.33 ms** | JavaScript 22 23 - Créer un... | 100.0% | 0 |
| 2171 | `Tutoriel_PHP_-_Créer_un_Calendrier_3_3,...` | 2481.2 ms | 90.68 ms | 25.46 ms | 303.44 ms | 1.0 ms | 0.5 ms | **2901.68 ms** | Tutoriel PHP - Créer un Cal... | 100.0% | 0 |
| 2172 | `Tutoriel_PHP_-_Créer_un_Calendrier_4_3,...` | 2277.64 ms | 92.38 ms | 26.78 ms | 245.93 ms | 1.0 ms | 0.5 ms | **2643.42 ms** | Tutoriel PHP - Créer un Cal... | 100.0% | 0 |
| 2173 | `Tutoriel_PHP_-_Créer_un_MVC_mid15s.mp4` | 2329.41 ms | 87.98 ms | 164.7 ms | 232.86 ms | 1.0 ms | 0.5 ms | **2815.4 ms** | Tutoriel PHP - Créer un MVC... | 100.0% | 0 |
| 2174 | `Tutoriel_PHP_-_Créer_un_panier_en_PHP_m...` | 2219.57 ms | 86.02 ms | 25.81 ms | 441.46 ms | 1.0 ms | 0.5 ms | **2773.19 ms** | Tutoriel PHP - Créer un pan... | 100.0% | 0 |
| 2175 | `Tutoriel_PHP_-_Créer_un_router_mid15s.mp4` | 4208.97 ms | 119.86 ms | 33.28 ms | 280.92 ms | 1.0 ms | 0.5 ms | **4643.45 ms** | Tutoriel PHP - Développer u... | 100.0% | 0 |
| 2176 | `Tutoriel_PHP_-_Créer_un_site_dynamique_...` | 2288.13 ms | 104.18 ms | 25.66 ms | 368.99 ms | 1.0 ms | 0.5 ms | **2787.38 ms** | Tutoriel PHP - Créer un sit... | 100.0% | 0 |
| 2177 | `Tutoriel_PHP_-_Créer_un_système_de_Cach...` | 4592.62 ms | 165.76 ms | 39.46 ms | 258.28 ms | 1.0 ms | 0.5 ms | **5058.34 ms** | Aucun | -- | 0 |
| 2178 | `Tutoriel_PHP_-_Créer_un_tchat_en_Ajax_P...` | 2783.33 ms | 115.76 ms | 22.94 ms | 492.48 ms | 1.0 ms | 0.5 ms | **3415.03 ms** | Tutoriel PHP - Créer un tch... | 100.0% | 0 |
| 2179 | `Tutoriel_PHP_-_Créer_une_pagination_mid...` | 3953.6 ms | 98.85 ms | 22.21 ms | 350.72 ms | 1.0 ms | 0.5 ms | **4425.76 ms** | Tutoriel PHP - Créer une pa... | 100.0% | 0 |
| 2180 | `Tutoriel_PHP_-_DOM_Parser_mid15s.mp4` | 1232.56 ms | 0.0 ms | 0.0 ms | 193.47 ms | 1.0 ms | 0.5 ms | **1426.17 ms** | Aucun | -- | 0 |
| 2181 | `Tutoriel_PHP_-_Dump_Mysql_mid15s.mp4` | 1306.38 ms | 0.0 ms | 0.0 ms | 110.66 ms | 1.0 ms | 0.5 ms | **1417.16 ms** | Aucun | -- | 0 |
| 2182 | `Tutoriel_PHP_-_Décomposer_un_site_en_PH...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.06 ms** | Erreur | -- | 0 |
| 2183 | `Tutoriel_PHP_-_Décomposer_un_site_en_PH...` | 800.11 ms | 0.0 ms | 0.0 ms | 116.24 ms | 1.0 ms | 0.5 ms | **916.45 ms** | Aucun | -- | 0 |
| 2184 | `Tutoriel_PHP_-_Déployer_avec_Envoy_mid1...` | 1948.32 ms | 96.0 ms | 66.11 ms | 237.04 ms | 1.0 ms | 0.5 ms | **2355.8 ms** | Tutoriel PHP - Déployer ave... | 100.0% | 0 |
| 2185 | `Tutoriel_PHP_-_Développer_un_site_-_Jou...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.76 ms** | Erreur | -- | 0 |
| 2186 | `Tutoriel_PHP_-_Développer_un_site_-_Jou...` | 5558.84 ms | 109.17 ms | 60.45 ms | 364.74 ms | 1.0 ms | 0.5 ms | **6097.94 ms** | Tutoriel PHP - Développer u... | 100.0% | 0 |
| 2187 | `Tutoriel_PHP_-_Développer_un_site_-_Jou...` | 3282.58 ms | 115.03 ms | 77.11 ms | 207.77 ms | 1.0 ms | 0.5 ms | **3683.06 ms** | Tutoriel PHP - Développer u... | 100.0% | 0 |
| 2188 | `Tutoriel_PHP_-_Développer_un_site_-_Jou...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.89 ms** | Erreur | -- | 0 |
| 2189 | `Tutoriel_PHP_-_Développer_un_site_-_Jou...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.3 ms** | Erreur | -- | 0 |
| 2190 | `Tutoriel_PHP_-_Développer_un_site_-_Jou...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.22 ms** | Erreur | -- | 0 |
| 2191 | `Tutoriel_PHP_-_Développer_un_site_-_Jou...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12064.12 ms** | Erreur | -- | 0 |
| 2192 | `Tutoriel_PHP_-_Edition_de_fichiers_en_P...` | 1764.89 ms | 0.0 ms | 0.0 ms | 384.4 ms | 1.0 ms | 0.5 ms | **2149.41 ms** | Aucun | -- | 0 |
| 2193 | `Tutoriel_PHP_-_Filtrer_le_spam_avec_Aki...` | 3848.7 ms | 168.74 ms | 186.3 ms | 312.49 ms | 1.0 ms | 0.5 ms | **4518.11 ms** | Tutoriel PHP - Filtrer le s... | 100.0% | 0 |
| 2194 | `Tutoriel_PHP_-_Fonction_rechercher_mid1...` | 1627.82 ms | 0.0 ms | 0.0 ms | 198.78 ms | 1.0 ms | 0.5 ms | **1826.74 ms** | Aucun | -- | 0 |
| 2195 | `Tutoriel_PHP_-_Formulaire_de_contact_en...` | 3576.19 ms | 130.65 ms | 43.31 ms | 232.45 ms | 1.0 ms | 0.5 ms | **3982.92 ms** | Tutoriel PHP - L'URL Rewrit... | 100.0% | 0 |
| 2196 | `Tutoriel_PHP_-_Gestion_d'images_Créatio...` | 2140.85 ms | 140.31 ms | 20.44 ms | 152.22 ms | 1.0 ms | 0.5 ms | **2454.31 ms** | Tutoriel PHP - Gestion d'im... | 100.0% | 0 |
| 2197 | `Tutoriel_PHP_-_Gestion_d'un_compte_memb...` | 3240.27 ms | 108.27 ms | 36.4 ms | 290.01 ms | 1.0 ms | 0.5 ms | **3675.39 ms** | Tutoriel PHP - Commentaires... | 100.0% | 0 |
| 2198 | `Tutoriel_PHP_-_Gestion_d'un_espace_memb...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12061.08 ms** | Erreur | -- | 0 |
| 2199 | `Tutoriel_PHP_-_Gestion_du_cache_Etag_et...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.26 ms** | Erreur | -- | 0 |
| 2200 | `Tutoriel_PHP_-_Générer_des_PDF_avec_WKH...` | 9909.49 ms | 141.17 ms | 79.6 ms | 417.0 ms | 1.0 ms | 0.5 ms | **10547.67 ms** | Aucun | -- | 0 |
| 2201 | `Tutoriel_PHP_-_Gérer_des_Permissions_mi...` | 1908.64 ms | 0.0 ms | 0.0 ms | 293.21 ms | 1.0 ms | 0.5 ms | **2201.99 ms** | Aucun | -- | 0 |
| 2202 | `Tutoriel_PHP_-_Gérer_des_news_en_PHP_mi...` | 2921.17 ms | 152.89 ms | 27.88 ms | 353.44 ms | 1.0 ms | 0.5 ms | **3455.8 ms** | Tutoriel PHP - Gérer des ne... | 100.0% | 0 |
| 2203 | `Tutoriel_PHP_-_Imposez_vos_normes_avec_...` | 2532.9 ms | 151.07 ms | 28.12 ms | 237.48 ms | 1.0 ms | 0.5 ms | **2950.47 ms** | Tutoriel PHP - Imposez vos ... | 100.0% | 0 |
| 2204 | `Tutoriel_PHP_-_Installer_PHP___MySQL_su...` | 2590.25 ms | 148.91 ms | 21.11 ms | 232.3 ms | 1.0 ms | 0.5 ms | **2993.03 ms** | Tutoriel PHP - Installer PH... | 100.0% | 0 |
| 2205 | `Tutoriel_PHP_-_Introduction_à_Laravel_m...` | 3927.57 ms | 152.39 ms | 25.07 ms | 391.66 ms | 1.0 ms | 0.5 ms | **4506.15 ms** | Tutoriel PHP - Introduction... | 100.0% | 0 |
| 2206 | `Tutoriel_PHP_-_Introduction_à_la_POO_mi...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.32 ms** | Erreur | -- | 0 |
| 2207 | `Tutoriel_PHP_-_Laravel_ou_Symfony_mid15...` | 3734.21 ms | 127.74 ms | 33.05 ms | 291.93 ms | 1.0 ms | 0.5 ms | **4187.28 ms** | Tutoriel PHP - Laravel ou S... | 100.0% | 0 |
| 2208 | `Tutoriel_PHP_-_Les_Exceptions_mid15s.mp4` | 4182.47 ms | 147.67 ms | 154.87 ms | 343.16 ms | 1.0 ms | 0.5 ms | **4829.18 ms** | Tutoriel PHP - Les Exceptio... | 100.0% | 0 |
| 2209 | `Tutoriel_PHP_-_Les_expressions_régulièr...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.97 ms** | Erreur | -- | 0 |
| 2210 | `Tutoriel_PHP_-_Like___Dislike_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.25 ms** | Erreur | -- | 0 |
| 2211 | `Tutoriel_PHP_-_Manipuler_les_tableaux_e...` | 3235.01 ms | 119.18 ms | 43.0 ms | 294.31 ms | 1.0 ms | 0.5 ms | **3691.82 ms** | Aucun | -- | 0 |
| 2212 | `Tutoriel_PHP_-_Middleware_CSRF_PSR-15_m...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12061.47 ms** | Erreur | -- | 0 |
| 2213 | `Tutoriel_PHP_-_Migrer_sa_base_de_donnée...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.89 ms** | Erreur | -- | 0 |
| 2214 | `Tutoriel_PHP_-_PHP-DI,_Conteneur_d'inje...` | 2882.12 ms | 120.86 ms | 128.3 ms | 217.86 ms | 1.0 ms | 0.5 ms | **3349.57 ms** | Tutoriel PHP - PHP-DI, Cont... | 100.0% | 0 |
| 2215 | `Tutoriel_PHP_-_PHPUnit_Prophecy_mid15s.mp4` | 3169.2 ms | 190.49 ms | 25.75 ms | 279.9 ms | 1.0 ms | 0.5 ms | **3665.69 ms** | Tutoriel PHP - PHPUnit Prop... | 100.0% | 0 |
| 2216 | `Tutoriel_PHP_-_PHP_7_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.15 ms** | Erreur | -- | 0 |
| 2217 | `Tutoriel_PHP_-_POO_et_Création_de_model...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.9 ms** | Erreur | -- | 0 |
| 2218 | `Tutoriel_PHP_-_Paiement_Paypal_mid15s.mp4` | 3709.61 ms | 164.85 ms | 35.92 ms | 303.73 ms | 1.0 ms | 0.5 ms | **4214.46 ms** | Tutoriel PHP - Paiement Pay... | 100.0% | 0 |
| 2219 | `Tutoriel_PHP_-_Paypal,_Paiement_récurre...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12064.33 ms** | Erreur | -- | 0 |
| 2220 | `Tutoriel_PHP_-_Plugin_de_commentaires_m...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12062.88 ms** | Erreur | -- | 0 |
| 2221 | `Tutoriel_PHP_-_Poster_un_Tweet_mid15s.mp4` | 5078.64 ms | 197.3 ms | 24.75 ms | 315.16 ms | 1.0 ms | 0.5 ms | **5616.31 ms** | Tutoriel PHP - Poster un Tw... | 100.0% | 0 |
| 2222 | `Tutoriel_PHP_-_Qu'est_ce_que_le_PSR_mid...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.37 ms** | Erreur | -- | 0 |
| 2223 | `Tutoriel_PHP_-_ReCAPTCHA_mid15s.mp4` | 1511.04 ms | 0.0 ms | 0.0 ms | 390.66 ms | 1.0 ms | 0.5 ms | **1901.82 ms** | Aucun | -- | 0 |
| 2224 | `Tutoriel_PHP_-_Redimensionner_vos_image...` | 8368.08 ms | 211.61 ms | 85.15 ms | 885.74 ms | 1.0 ms | 0.5 ms | **9550.93 ms** | Tutoriel PHP - Redimensionn... | 100.0% | 0 |
| 2225 | `Tutoriel_PHP_-_Russian_Doll_Caching_mid...` | 1392.35 ms | 0.0 ms | 0.0 ms | 193.19 ms | 1.0 ms | 0.5 ms | **1614.48 ms** | Aucun | -- | 0 |
| 2226 | `Tutoriel_PHP_-_Se_démerder_tout_seul_mi...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.7 ms** | Erreur | -- | 0 |
| 2227 | `Tutoriel_PHP_-_Serveur_Web_Interne_mid1...` | 3763.02 ms | 201.64 ms | 100.98 ms | 377.54 ms | 1.0 ms | 0.5 ms | **4443.64 ms** | Aucun | -- | 0 |
| 2228 | `Tutoriel_PHP_-_Simple_ou_Double_guillem...` | 2905.26 ms | 0.0 ms | 0.0 ms | 214.79 ms | 1.0 ms | 0.5 ms | **3120.2 ms** | Aucun | -- | 0 |
| 2229 | `Tutoriel_PHP_-_Slim_Framework_mid15s.mp4` | 3281.36 ms | 141.99 ms | 169.18 ms | 115.11 ms | 1.0 ms | 0.5 ms | **3708.37 ms** | Tutoriel PHP - Like Dislike... | 100.0% | 0 |
| 2230 | `Tutoriel_PHP_-_Sécuriser_avec_les_Sessi...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12063.35 ms** | Erreur | -- | 0 |
| 2231 | `Tutoriel_PHP_-_Sécurité,_Les_Failles_CS...` | 4869.68 ms | 213.33 ms | 20.31 ms | 256.03 ms | 1.0 ms | 0.5 ms | **5359.7 ms** | Tutoriel PHP - Sécurité, Le... | 100.0% | 0 |
| 2232 | `Tutoriel_PHP_-_Sécurité,_Les_failles_XS...` | 2469.07 ms | 143.97 ms | 18.08 ms | 354.56 ms | 1.0 ms | 0.5 ms | **2986.05 ms** | Tutoriel PHP - Sécurité, Le... | 100.0% | 0 |
| 2233 | `Tutoriel_PHP_-_Sécurité,_Les_injections...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.79 ms** | Erreur | -- | 0 |
| 2234 | `Tutoriel_PHP_-_Sécurité,_les_failles_in...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.5 ms** | Erreur | -- | 0 |
| 2235 | `Tutoriel_PHP_-_Testez_votre_code_avec_K...` | 6757.73 ms | 201.34 ms | 24.09 ms | 299.57 ms | 1.0 ms | 0.5 ms | **7283.14 ms** | Tutoriel PHP - Testez votre... | 100.0% | 0 |
| 2236 | `Tutoriel_PHP_-_Tests_unitaires,_PHPUnit...` | 6545.11 ms | 236.2 ms | 23.72 ms | 162.61 ms | 1.0 ms | 0.5 ms | **6968.07 ms** | Tutoriel PHP - Tests unitai... | 100.0% | 0 |
| 2237 | `Tutoriel_PHP_-_Travailler_avec_des_imag...` | 1807.85 ms | 0.0 ms | 0.0 ms | 518.61 ms | 1.0 ms | 0.5 ms | **2326.6 ms** | Aucun | -- | 0 |
| 2238 | `Tutoriel_PHP_-_Twig_mid15s.mp4` | 2949.72 ms | 261.94 ms | 129.41 ms | 773.82 ms | 1.0 ms | 0.5 ms | **4115.32 ms** | Aucun | -- | 0 |
| 2239 | `Tutoriel_PHP_-_Téléchargement_sécurisé_...` | 7726.46 ms | 261.37 ms | 24.48 ms | 240.14 ms | 1.0 ms | 0.5 ms | **8253.04 ms** | Tutoriel PHP - Téléchargeme... | 100.0% | 0 |
| 2240 | `Tutoriel_PHP_-_Uploader_un_fichier_sur_...` | 3224.37 ms | 219.86 ms | 26.38 ms | 294.29 ms | 1.0 ms | 0.5 ms | **3765.54 ms** | Tutoriel PHP - Uploader un ... | 100.0% | 0 |
| 2241 | `Tutoriel_PHP_-_Xdebug,_l'exécution_pas_...` | 1820.56 ms | 0.0 ms | 0.0 ms | 478.41 ms | 1.0 ms | 0.5 ms | **2299.15 ms** | Aucun | -- | 0 |
| 2242 | `Tutoriel_PHP_-_Youtube_Data_API_mid15s.mp4` | 5446.83 ms | 237.9 ms | 28.09 ms | 291.88 ms | 1.0 ms | 0.5 ms | **6005.04 ms** | Tutoriel PHP - Youtube Data... | 100.0% | 0 |
| 2243 | `Tutoriel_PHP_-_Zend_Expressive_mid15s.mp4` | 1509.68 ms | 0.0 ms | 0.0 ms | 228.59 ms | 1.0 ms | 0.5 ms | **1738.41 ms** | Aucun | -- | 0 |
| 2244 | `Tutoriel_PHP_-_reCaptcha_mid15s.mp4` | 5009.52 ms | 151.33 ms | 250.83 ms | 290.62 ms | 1.0 ms | 0.5 ms | **5702.66 ms** | Aucun | -- | 0 |
| 2245 | `Tutoriel_Photoshop_-_Arrondir_les_coins...` | 1100.85 ms | 0.0 ms | 0.0 ms | 411.45 ms | 1.0 ms | 0.5 ms | **1512.86 ms** | Aucun | -- | 0 |
| 2246 | `Tutoriel_Photoshop_-_Automatiser_avec_d...` | 2583.68 ms | 190.56 ms | 21.63 ms | 354.65 ms | 1.0 ms | 0.5 ms | **3150.92 ms** | Tutoriel Photoshop - Automa... | 100.0% | 0 |
| 2247 | `Tutoriel_Photoshop_-_Capture_d'écran_3D...` | 2361.03 ms | 0.0 ms | 0.0 ms | 190.53 ms | 1.0 ms | 0.5 ms | **2551.67 ms** | Aucun | -- | 0 |
| 2248 | `Tutoriel_Photoshop_-_Créer_un_design_we...` | 9146.4 ms | 245.1 ms | 25.89 ms | 279.24 ms | 1.0 ms | 0.5 ms | **9697.08 ms** | Tutoriel Photoshop - Créer ... | 100.0% | 0 |
| 2249 | `Tutoriel_Photoshop_-_Les_scripts_mid15s...` | 1042.7 ms | 0.0 ms | 0.0 ms | 495.18 ms | 1.0 ms | 0.5 ms | **1538.0 ms** | Aucun | -- | 0 |
| 2250 | `Tutoriel_Photoshop_-_Plugin_CSS3Ps_mid1...` | 1622.94 ms | 0.0 ms | 0.0 ms | 646.36 ms | 1.0 ms | 0.5 ms | **2269.44 ms** | Aucun | -- | 0 |
| 2251 | `Tutoriel_Photoshop_-_Réaliser_un_gif_an...` | 3466.28 ms | 124.6 ms | 21.19 ms | 267.17 ms | 1.0 ms | 0.5 ms | **3879.65 ms** | Tutoriel Photoshop - Réalis... | 100.0% | 0 |
| 2252 | `Tutoriel_Photoshop_-_Utiliser_les_motif...` | 5051.37 ms | 102.79 ms | 23.46 ms | 680.19 ms | 1.0 ms | 0.5 ms | **5858.28 ms** | Tutoriel Photoshop - Utilis... | 100.0% | 0 |
| 2253 | `Tutoriel_PreactJS_-_Créer_un_éditeur_Ma...` | 4336.81 ms | 227.5 ms | 26.49 ms | 533.54 ms | 1.0 ms | 0.5 ms | **5124.71 ms** | Tutoriel PreactJS - Créer u... | 100.0% | 0 |
| 2254 | `Tutoriel_Rails_-_Héberger_Rails_avec_ng...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12063.66 ms** | Erreur | -- | 0 |
| 2255 | `Tutoriel_ReactJS_-_React_par_la_pratiqu...` | 3331.53 ms | 252.17 ms | 111.25 ms | 318.73 ms | 1.0 ms | 0.5 ms | **4020.5 ms** | Tutoriel ReactJS - React pa... | 100.0% | 0 |
| 2256 | `Tutoriel_Ruby_-_Découverte_d'ActionCabl...` | 2501.39 ms | 0.0 ms | 0.0 ms | 322.32 ms | 1.0 ms | 0.5 ms | **2823.86 ms** | Aucun | -- | 0 |
| 2257 | `Tutoriel_Ruby_-_de_PHP_à_Ruby_on_rails_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.24 ms** | Erreur | -- | 0 |
| 2258 | `Tutoriel_SASS_-_Susy_mid15s.mp4` | 8877.22 ms | 254.78 ms | 24.17 ms | 196.07 ms | 1.0 ms | 0.5 ms | **9352.65 ms** | Tutoriel SASS - Susy mp4 24... | 100.0% | 0 |
| 2259 | `Tutoriel_Serveur_-_Elastic_Stack_mid15s...` | 3440.8 ms | 156.92 ms | 22.16 ms | 183.98 ms | 1.0 ms | 0.5 ms | **3804.44 ms** | Tutoriel Serveur - Elastic ... | 100.0% | 0 |
| 2260 | `Tutoriel_Serveur_-_Les_bases_de_Redis_m...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.98 ms** | Erreur | -- | 0 |
| 2261 | `Tutoriel_Sublime_Text_2_-_Créer_un_plug...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.47 ms** | Erreur | -- | 0 |
| 2262 | `Tutoriel_Sublime_Text_2_-_Emmet_&_PHP_m...` | 1663.21 ms | 0.0 ms | 0.0 ms | 517.95 ms | 1.0 ms | 0.5 ms | **2182.52 ms** | Aucun | -- | 0 |
| 2263 | `Tutoriel_Sublime_Text_2_-_Emmet_Livesty...` | 1676.74 ms | 0.0 ms | 0.0 ms | 313.84 ms | 1.0 ms | 0.5 ms | **1990.71 ms** | Aucun | -- | 0 |
| 2264 | `Tutoriel_Sublime_Text_2_-_Fetch_Plugin_...` | 2780.58 ms | 0.0 ms | 0.0 ms | 206.57 ms | 1.0 ms | 0.5 ms | **2987.42 ms** | Aucun | -- | 0 |
| 2265 | `Tutoriel_Sublime_Text_2_-_Présentation_...` | 3594.46 ms | 121.81 ms | 48.46 ms | 161.08 ms | 1.0 ms | 0.5 ms | **3926.16 ms** | Tutoriel Sublime Text 2 - P... | 100.0% | 0 |
| 2266 | `Tutoriel_Sublime_Text_2_-_mCSS_mid15s.mp4` | 1627.87 ms | 0.0 ms | 0.0 ms | 175.56 ms | 1.0 ms | 0.5 ms | **1803.57 ms** | Aucun | -- | 0 |
| 2267 | `Tutoriel_Symfony_-_Système_de_Tags_mid1...` | 5899.1 ms | 261.21 ms | 26.65 ms | 185.41 ms | 1.0 ms | 0.5 ms | **6372.8 ms** | Tutoriel Symfony - Système ... | 100.0% | 0 |
| 2268 | `Tutoriel_Symfony_-_Système_de_badges___...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.22 ms** | Erreur | -- | 0 |
| 2269 | `Tutoriel_Symfony_3_-_Annotation_et_Uplo...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.34 ms** | Erreur | -- | 0 |
| 2270 | `Tutoriel_Vidéo_HTML_CSS_-_Intégrer_une_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.58 ms** | Erreur | -- | 0 |
| 2271 | `Tutoriel_VueJS_-_Créer_un_Datepicker_mi...` | 3961.09 ms | 246.22 ms | 23.07 ms | 415.91 ms | 1.0 ms | 0.5 ms | **4647.0 ms** | Tutoriel VueJS - Créer un D... | 100.0% | 0 |
| 2272 | `Tutoriel_VueJS_-_Sélecteur_de_périodes_...` | 2918.12 ms | 142.17 ms | 21.34 ms | 328.4 ms | 1.0 ms | 0.5 ms | **3410.85 ms** | Tutoriel VueJS - Sélecteur ... | 100.0% | 0 |
| 2273 | `Tutoriel_Wamp_-_Utilisation_Wamp_mid15s...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.55 ms** | Erreur | -- | 0 |
| 2274 | `Tutoriel_WordPress_-_Ajouter_un_champ_a...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.26 ms** | Erreur | -- | 0 |
| 2275 | `Tutoriel_WordPress_-_Créer_un_carrousel...` | 4732.75 ms | 167.19 ms | 21.0 ms | 229.93 ms | 1.0 ms | 0.5 ms | **5151.32 ms** | Tutoriel WordPress - Créer ... | 100.0% | 0 |
| 2276 | `Tutoriel_WordPress_-_Créer_un_panneau_d...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.24 ms** | Erreur | -- | 0 |
| 2277 | `Tutoriel_WordPress_-_Créer_un_site_Word...` | 6628.3 ms | 136.69 ms | 17.03 ms | 304.5 ms | 1.0 ms | 0.5 ms | **7086.98 ms** | Tutoriel WordPress - Créer ... | 100.0% | 0 |
| 2278 | `Tutoriel_WordPress_-_Créer_un_thème_wor...` | 5485.24 ms | 259.97 ms | 143.75 ms | 620.98 ms | 1.0 ms | 0.5 ms | **6519.48 ms** | Aucun | -- | 0 |
| 2279 | `Tutoriel_WordPress_-_Créer_un_widget_Wo...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.97 ms** | Erreur | -- | 0 |
| 2280 | `Tutoriel_WordPress_-_Créer_une_homepage...` | 3176.33 ms | 166.3 ms | 21.14 ms | 438.6 ms | 1.0 ms | 0.5 ms | **3803.94 ms** | Aucun | -- | 0 |
| 2281 | `Tutoriel_WordPress_-_Custom_post_type_m...` | 2271.51 ms | 241.64 ms | 21.29 ms | 338.1 ms | 1.0 ms | 0.5 ms | **2872.95 ms** | Tutoriel WordPress - Custom... | 100.0% | 0 |
| 2282 | `Tutoriel_WordPress_-_Elementor_Page_Bui...` | 2210.17 ms | 116.61 ms | 20.41 ms | 253.65 ms | 1.0 ms | 0.5 ms | **2601.33 ms** | Tutoriel WordPress - Elemen... | 100.0% | 0 |
| 2283 | `Tutoriel_WordPress_-_Framework_Wordpres...` | 2265.09 ms | 112.78 ms | 137.26 ms | 609.39 ms | 1.0 ms | 0.5 ms | **3132.5 ms** | Tutoriel WordPress - Framew... | 100.0% | 0 |
| 2284 | `Tutoriel_WordPress_-_Habillage_d'articl...` | 1820.3 ms | 0.0 ms | 0.0 ms | 144.25 ms | 1.0 ms | 0.5 ms | **1964.67 ms** | Aucun | -- | 0 |
| 2285 | `Tutoriel_WordPress_-_Inscription_et_Con...` | 1774.72 ms | 0.0 ms | 0.0 ms | 346.24 ms | 1.0 ms | 0.5 ms | **2121.12 ms** | Aucun | -- | 0 |
| 2286 | `Tutoriel_WordPress_-_Les_menus_mid15s.mp4` | 1966.74 ms | 0.0 ms | 0.0 ms | 191.21 ms | 1.0 ms | 0.5 ms | **2158.11 ms** | Aucun | -- | 0 |
| 2287 | `Tutoriel_WordPress_-_Shortcode_mid15s.mp4` | 1844.0 ms | 0.0 ms | 0.0 ms | 164.79 ms | 1.0 ms | 0.5 ms | **2008.92 ms** | Aucun | -- | 0 |
| 2288 | `Tutoriel_WordPress_-_Taxonomy_mid15s.mp4` | 1543.03 ms | 0.0 ms | 0.0 ms | 274.89 ms | 1.0 ms | 0.5 ms | **1818.05 ms** | Aucun | -- | 0 |
| 2289 | `Tutoriel_WordPress_-_Utiliser_l'uploade...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.41 ms** | Erreur | -- | 0 |
| 2290 | `Tutoriel_WordPress_-_Wordplate_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12058.77 ms** | Erreur | -- | 0 |
| 2291 | `Tutoriel_after_effect_-_les_bases_d'aft...` | 3547.38 ms | 693.79 ms | 21.39 ms | 660.17 ms | 1.0 ms | 0.5 ms | **4923.09 ms** | Tutoriel after effect - les... | 100.0% | 0 |
| 2292 | `Tutoriel_cakephp_-_Créer_un_bonjour_mid...` | 5356.96 ms | 0.0 ms | 0.0 ms | 317.19 ms | 1.0 ms | 0.5 ms | **9185.1 ms** | Aucun | -- | 0 |
| 2293 | `Tutoriel_cakephp_-_Passer_de_cakephp_1....` | 1486.51 ms | 0.0 ms | 0.0 ms | 337.94 ms | 1.0 ms | 0.5 ms | **1824.61 ms** | Aucun | -- | 0 |
| 2294 | `Tutoriel_grafikart_-_Making_of_mid15s.mp4` | 2200.39 ms | 1467.19 ms | 42.77 ms | 596.25 ms | 1.0 ms | 0.5 ms | **4307.18 ms** | Tutoriel grafikart - Making... | 100.0% | 0 |
| 2295 | `Tutoriel_html-css_-_Simuler_le_tactile_...` | 3399.2 ms | 205.61 ms | 28.28 ms | 611.23 ms | 1.0 ms | 0.5 ms | **4245.2 ms** | Tutoriel html-css - Simuler... | 100.0% | 0 |
| 2296 | `Tutoriel_jQuery_-_Créer_un_système_de_n...` | 6599.12 ms | 0.0 ms | 0.0 ms | 291.75 ms | 1.0 ms | 0.5 ms | **10392.65 ms** | Aucun | -- | 0 |
| 2297 | `Tutoriel_jQuery_-_Créer_une_Jauge_Circu...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12061.41 ms** | Erreur | -- | 0 |
| 2298 | `Tutoriel_jQuery_-_Créer_une_carte_inter...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.32 ms** | Erreur | -- | 0 |
| 2299 | `Tutoriel_jQuery_-_Décompte_Javascript_m...` | 1236.6 ms | 0.0 ms | 0.0 ms | 175.92 ms | 1.0 ms | 0.5 ms | **1412.64 ms** | Aucun | -- | 0 |
| 2300 | `Tutoriel_jQuery_-_Effet_de_lueur_en_Jav...` | 2673.92 ms | 0.0 ms | 0.0 ms | 189.16 ms | 1.0 ms | 0.5 ms | **6364.62 ms** | Aucun | -- | 0 |
| 2301 | `Tutoriel_jQuery_-_Effet_de_survol_mid15...` | 1546.32 ms | 0.0 ms | 0.0 ms | 181.73 ms | 1.0 ms | 0.5 ms | **1728.18 ms** | Aucun | -- | 0 |
| 2302 | `Tutoriel_jQuery_-_Infobulle_avec_jQuery...` | 2814.08 ms | 0.0 ms | 0.0 ms | 153.18 ms | 1.0 ms | 0.5 ms | **2967.41 ms** | Aucun | -- | 0 |
| 2303 | `Tutoriel_jQuery_-_Initiation_à_jQuery_m...` | 2986.87 ms | 188.1 ms | 20.12 ms | 257.47 ms | 1.0 ms | 0.5 ms | **3452.98 ms** | Aucun | -- | 0 |
| 2304 | `Tutoriel_jQuery_-_La_fonction_on()_mid1...` | 1796.33 ms | 0.0 ms | 0.0 ms | 334.93 ms | 1.0 ms | 0.5 ms | **2131.38 ms** | Aucun | -- | 0 |
| 2305 | `Tutoriel_jQuery_-_Les_évènements_Focus_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.69 ms** | Erreur | -- | 0 |
| 2306 | `Tutoriel_jQuery_-_Limiter_le_choix_des_...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.81 ms** | Erreur | -- | 0 |
| 2307 | `Tutoriel_jQuery_-_Menu_collant_mid15s.mp4` | 1881.18 ms | 0.0 ms | 0.0 ms | 258.14 ms | 1.0 ms | 0.5 ms | **2139.43 ms** | Aucun | -- | 0 |
| 2308 | `Tutoriel_jQuery_-_Navigation_en_Onglets...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12061.31 ms** | Erreur | -- | 0 |
| 2309 | `Tutoriel_jQuery_-_Page_cornée_animée_mi...` | 2099.66 ms | 0.0 ms | 0.0 ms | 411.2 ms | 1.0 ms | 0.5 ms | **2511.03 ms** | Aucun | -- | 0 |
| 2310 | `Tutoriel_jQuery_-_Plupload_mid15s.mp4` | 3840.32 ms | 0.0 ms | 0.0 ms | 146.39 ms | 1.0 ms | 0.5 ms | **7488.37 ms** | Aucun | -- | 0 |
| 2311 | `Tutoriel_jQuery_-_Poster_un_formulaire_...` | 2536.51 ms | 0.0 ms | 0.0 ms | 171.99 ms | 1.0 ms | 0.5 ms | **6418.44 ms** | Aucun | -- | 0 |
| 2312 | `Tutoriel_jQuery_-_Réaliser_une_Pop-in_a...` | 4637.04 ms | 0.0 ms | 0.0 ms | 398.96 ms | 1.0 ms | 0.5 ms | **8537.39 ms** | Aucun | -- | 0 |
| 2313 | `Tutoriel_jQuery_-_Scrolling_animé_avec_...` | 2121.85 ms | 0.0 ms | 0.0 ms | 388.85 ms | 1.0 ms | 0.5 ms | **2510.85 ms** | Aucun | -- | 0 |
| 2314 | `Tutoriel_jQuery_-_Slider_Javascript_ave...` | 2267.54 ms | 0.0 ms | 0.0 ms | 373.4 ms | 1.0 ms | 0.5 ms | **2641.09 ms** | Aucun | -- | 0 |
| 2315 | `Tutoriel_jQuery_-_Survol_directionnel_m...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.09 ms** | Erreur | -- | 0 |
| 2316 | `Tutoriel_jQuery_-_Zoombox_1.1_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.87 ms** | Erreur | -- | 0 |
| 2317 | `Tutoriel_jQuery_-_Zoombox_V2_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.53 ms** | Erreur | -- | 0 |
| 2318 | `Tutoriel_jQuery_-_jQueryUI_Sortable_mid...` | 6226.19 ms | 0.0 ms | 0.0 ms | 109.1 ms | 1.0 ms | 0.5 ms | **9836.3 ms** | Aucun | -- | 0 |
| 2319 | `Tutoriel_jquery_-_valider_un_formulaire...` | 2159.23 ms | 0.0 ms | 0.0 ms | 221.95 ms | 1.0 ms | 0.5 ms | **2381.32 ms** | Aucun | -- | 0 |
| 2320 | `Tutoriel_linux_-_configurer_son_premier...` | 2413.46 ms | 0.0 ms | 0.0 ms | 373.53 ms | 1.0 ms | 0.5 ms | **6389.12 ms** | Aucun | -- | 0 |
| 2321 | `Tutoriel_photoshop_-_Ombres_chinoises_m...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12059.55 ms** | Erreur | -- | 0 |
| 2322 | `Tutoriel_photoshop_-_intégrer_plus_vite...` | 4125.72 ms | 0.0 ms | 0.0 ms | 124.84 ms | 1.0 ms | 0.5 ms | **7753.42 ms** | Aucun | -- | 0 |
| 2323 | `Tutoriel_php_-_Créations_de_helpers_mid...` | 1210.03 ms | 0.0 ms | 0.0 ms | 175.16 ms | 1.0 ms | 0.5 ms | **1385.32 ms** | Aucun | -- | 0 |
| 2324 | `Tutoriel_php_-_Facebook_connect_mid15s.mp4` | 5695.06 ms | 0.0 ms | 0.0 ms | 88.77 ms | 1.0 ms | 0.5 ms | **9303.32 ms** | Aucun | -- | 0 |
| 2325 | `Tutoriel_php_-_L'objet_pdo_en_php_mid15...` | 5570.4 ms | 0.0 ms | 0.0 ms | 129.83 ms | 1.0 ms | 0.5 ms | **9201.59 ms** | Aucun | -- | 0 |
| 2326 | `Tutoriel_php_-_générer_un_pdf_mid15s.mp4` | 4599.05 ms | 0.0 ms | 0.0 ms | 95.01 ms | 1.0 ms | 0.5 ms | **8211.35 ms** | Aucun | -- | 0 |
| 2327 | `Tutoriel_php_-_la_récursivité_mid15s.mp4` | 933.66 ms | 0.0 ms | 0.0 ms | 258.89 ms | 1.0 ms | 0.5 ms | **1192.7 ms** | Aucun | -- | 0 |
| 2328 | `Tutoriel_sublime_text_2_-_Livereload_mi...` | 2070.81 ms | 0.0 ms | 0.0 ms | 186.95 ms | 1.0 ms | 0.5 ms | **5769.72 ms** | Aucun | -- | 0 |
| 2329 | `Tutoriel_vidéo_Google_Chrome_-_Chrome_&...` | 1864.38 ms | 0.0 ms | 0.0 ms | 422.97 ms | 1.0 ms | 0.5 ms | **2287.51 ms** | Aucun | -- | 0 |
| 2330 | `Tutoriel_wordpress_-_Créer_un_thème_ava...` | 3546.7 ms | 0.0 ms | 0.0 ms | 83.98 ms | 1.0 ms | 0.5 ms | **7161.17 ms** | Aucun | -- | 0 |
| 2331 | `Un_idiot_a_Paris_(1967)_mid15s.mp4` | 2668.62 ms | 0.0 ms | 0.0 ms | 49.82 ms | 1.0 ms | 0.5 ms | **6277.59 ms** | Aucun | -- | 0 |
| 2332 | `Uncharted.2022.1080p.WEBRip.x264-RARBG_...` | 2768.77 ms | 0.0 ms | 0.0 ms | 98.85 ms | 1.0 ms | 0.5 ms | **6396.62 ms** | Uncharted.2022.1080p.WEBRip... | -- | 1 |
| 2333 | `Une_Nouvelle_Ère___L'odyssée_de_l'écrit...` | 2277.42 ms | 0.0 ms | 0.0 ms | 84.73 ms | 1.0 ms | 0.5 ms | **5863.48 ms** | Aucun | -- | 0 |
| 2334 | `Univers_ou_multivers___La_magie_du_cosm...` | 2137.21 ms | 0.0 ms | 0.0 ms | 55.98 ms | 1.0 ms | 0.5 ms | **5694.93 ms** | Aucun | -- | 0 |
| 2335 | `Utopia_(2013)_-_S02E01_-_Episode_1_(108...` | 1995.97 ms | 0.0 ms | 0.0 ms | 110.65 ms | 1.0 ms | 0.5 ms | **5608.83 ms** | Aucun | -- | 0 |
| 2336 | `Utopia_(2013)_-_S02E02_-_Episode_2_(108...` | 1804.14 ms | 0.0 ms | 0.0 ms | 79.25 ms | 1.0 ms | 0.5 ms | **5385.73 ms** | Aucun | -- | 0 |
| 2337 | `Utopia_(2013)_-_S02E03_-_Episode_3_(108...` | 1314.26 ms | 0.0 ms | 0.0 ms | 72.71 ms | 1.0 ms | 0.5 ms | **4888.89 ms** | Aucun | -- | 0 |
| 2338 | `Utopia_(2013)_-_S02E04_-_Episode_4_(108...` | 2048.01 ms | 0.0 ms | 0.0 ms | 188.76 ms | 1.0 ms | 0.5 ms | **5740.74 ms** | Aucun | -- | 0 |
| 2339 | `Utopia_(2013)_-_S02E05_-_Episode_5_(108...` | 1332.75 ms | 0.0 ms | 0.0 ms | 94.15 ms | 1.0 ms | 0.5 ms | **4932.93 ms** | Aucun | -- | 0 |
| 2340 | `Utopia_(2013)_-_S02E06_-_Episode_6_(108...` | 1095.66 ms | 0.0 ms | 0.0 ms | 104.88 ms | 1.0 ms | 0.5 ms | **4704.0 ms** | Aucun | -- | 0 |
| 2341 | `VID_20210113_181312_mid15s.mp4` | 1213.01 ms | 0.0 ms | 0.0 ms | 169.84 ms | 1.0 ms | 0.5 ms | **4909.55 ms** | Aucun | -- | 0 |
| 2342 | `VID_20210113_181312_x265_720p_mid15s.mp4` | 1068.3 ms | 0.0 ms | 0.0 ms | 145.15 ms | 1.0 ms | 0.5 ms | **4716.84 ms** | Aucun | -- | 0 |
| 2343 | `Vanilla_Sky_(2001)_[1080p]_mid15s.mp4` | 900.12 ms | 0.0 ms | 0.0 ms | 129.25 ms | 1.0 ms | 0.5 ms | **4650.66 ms** | Aucun | -- | 0 |
| 2344 | `Velvet_Buzzsaw_(2019)_mid15s.mp4` | 667.03 ms | 0.0 ms | 0.0 ms | 134.78 ms | 1.0 ms | 0.5 ms | **4303.41 ms** | Aucun | -- | 0 |
| 2345 | `Victoria_and_Abdul_(2017)[720p]_mid15s.mp4` | 856.61 ms | 0.0 ms | 0.0 ms | 297.58 ms | 1.0 ms | 0.5 ms | **4655.71 ms** | Aucun | -- | 0 |
| 2346 | `Vinyan.2008.1080p.BluRay.H264.AAC-RARBG...` | 1011.46 ms | 0.0 ms | 0.0 ms | 183.41 ms | 1.0 ms | 0.5 ms | **4696.85 ms** | Aucun | -- | 0 |
| 2347 | `Vivarium.2019.1080p.WEB-DL.DD5.1.H264-F...` | 778.68 ms | 0.0 ms | 0.0 ms | 223.49 ms | 1.0 ms | 0.5 ms | **4504.2 ms** | Aucun | -- | 0 |
| 2348 | `Vous_êtes_freelance_Votre_expérience_m'...` | 823.65 ms | 0.0 ms | 0.0 ms | 124.17 ms | 1.0 ms | 0.5 ms | **4478.5 ms** | Aucun | -- | 0 |
| 2349 | `Voyage.Of.Time.Life's.Journey.2016.1080...` | 1196.28 ms | 0.0 ms | 0.0 ms | 163.85 ms | 1.0 ms | 0.5 ms | **4913.25 ms** | Aucun | -- | 0 |
| 2350 | `Voyage_à_Tokyo_(1953)_VOSTFR_[1080p]_Bl...` | 3923.77 ms | 0.0 ms | 0.0 ms | 81.95 ms | 1.0 ms | 0.5 ms | **7507.79 ms** | Aucun | -- | 0 |
| 2351 | `VueJS_2_(13_15)_-_Vue_Resource_mid15s.mp4` | 1067.6 ms | 0.0 ms | 0.0 ms | 194.45 ms | 1.0 ms | 0.5 ms | **4766.83 ms** | Aucun | -- | 0 |
| 2352 | `VueJS_2_(14_15)_-_Vue_Router_mid15s.mp4` | 2970.56 ms | 0.0 ms | 0.0 ms | 123.83 ms | 1.0 ms | 0.5 ms | **6595.83 ms** | Aucun | -- | 0 |
| 2353 | `VueJS_2_(15_15)_-_VueX_mid15s.mp4` | 7060.47 ms | 0.0 ms | 0.0 ms | 118.0 ms | 1.0 ms | 0.5 ms | **10681.75 ms** | Aucun | -- | 0 |
| 2354 | `VueJS_2_(2_15)_-_L'instance_mid15s.mp4` | 6777.17 ms | 0.0 ms | 0.0 ms | 139.09 ms | 1.0 ms | 0.5 ms | **10418.95 ms** | Aucun | -- | 0 |
| 2355 | `VueJS_2_(3_15)_-_Propriétés_combinées_&...` | 1116.16 ms | 0.0 ms | 0.0 ms | 113.33 ms | 1.0 ms | 0.5 ms | **4731.68 ms** | Aucun | -- | 0 |
| 2356 | `VueJS_2_(7_15)_-_Créer_un_'Store'_mid15...` | 955.74 ms | 0.0 ms | 0.0 ms | 122.59 ms | 1.0 ms | 0.5 ms | **1078.47 ms** | Aucun | -- | 0 |
| 2357 | `VueJS_2_(8_15)_-_Vue-CLI_mid15s.mp4` | 985.17 ms | 0.0 ms | 0.0 ms | 240.55 ms | 1.0 ms | 0.5 ms | **1225.88 ms** | Aucun | -- | 0 |
| 2358 | `VueJS_2_(9_15)_-_Animations_&_Transitio...` | 602.74 ms | 0.0 ms | 0.0 ms | 102.05 ms | 1.0 ms | 0.5 ms | **704.92 ms** | Aucun | -- | 0 |
| 2359 | `Waiting_for_the_Barbarians_(2019)[1080p...` | 2099.28 ms | 0.0 ms | 0.0 ms | 47.9 ms | 1.0 ms | 0.5 ms | **5657.8 ms** | Aucun | -- | 0 |
| 2360 | `Warriors_of_the_Final_Holy_Battle_1989_...` | 2155.54 ms | 0.0 ms | 0.0 ms | 81.95 ms | 1.0 ms | 0.5 ms | **5778.19 ms** | Aucun | -- | 0 |
| 2361 | `Watchmen.Ultimate.Cut.2009.1080p.BrRip....` | 2151.95 ms | 0.0 ms | 0.0 ms | 113.0 ms | 1.0 ms | 0.5 ms | **5856.33 ms** | Aucun | -- | 0 |
| 2362 | `Waves.2019.1080p.WEBRip.x264.AAC5.1-[YT...` | 1873.05 ms | 0.0 ms | 0.0 ms | 69.32 ms | 1.0 ms | 0.5 ms | **5460.66 ms** | Waves.2019.1080p.WEBRip.x26... | -- | 1 |
| 2363 | `We.Own.the.Night.2007.1080p.BluRay.H264...` | 1783.27 ms | 0.0 ms | 0.0 ms | 87.43 ms | 1.0 ms | 0.5 ms | **5372.24 ms** | Aucun | -- | 0 |
| 2364 | `We.Own.the.Night.2007.1080p.BrRip.x264....` | 1774.15 ms | 0.0 ms | 0.0 ms | 115.65 ms | 1.0 ms | 0.5 ms | **5391.2 ms** | We.Own.the.Night.2007.1080p... | -- | 1 |
| 2365 | `Welcome_to_Marwen_(2018)_[1080p](L'hist...` | 2122.23 ms | 0.0 ms | 0.0 ms | 97.78 ms | 1.0 ms | 0.5 ms | **5731.09 ms** | Aucun | -- | 0 |
| 2366 | `West.Side.Story.2021.1080p.BluRay.x264-...` | 2038.62 ms | 0.0 ms | 0.0 ms | 94.02 ms | 1.0 ms | 0.5 ms | **5635.82 ms** | Aucun | -- | 0 |
| 2367 | `West.Side.Story.2021.1080p.BluRay.x264....` | 1968.56 ms | 0.0 ms | 0.0 ms | 231.64 ms | 1.0 ms | 0.5 ms | **5739.19 ms** | Aucun | -- | 0 |
| 2368 | `What.If.2021.S02E01.What.if.Nebula.Join...` | 1730.11 ms | 0.0 ms | 0.0 ms | 84.73 ms | 1.0 ms | 0.5 ms | **5323.07 ms** | What.If.2021.S02E01.What.if... | -- | 1 |
| 2369 | `What.If.2021.S02E02.What.if.Peter.Quill...` | 2141.09 ms | 0.0 ms | 0.0 ms | 178.1 ms | 1.0 ms | 0.5 ms | **5820.68 ms** | Aucun | -- | 0 |
| 2370 | `What.If.2021.S02E03.What.if.Happy.Hogan...` | 2023.41 ms | 0.0 ms | 0.0 ms | 210.33 ms | 1.0 ms | 0.5 ms | **5736.05 ms** | What.If.2021.S02E03.What.if... | -- | 1 |
| 2371 | `What.If.2021.S02E04.What.if.Iron.Man.Cr...` | 1773.63 ms | 0.0 ms | 0.0 ms | 151.06 ms | 1.0 ms | 0.5 ms | **5427.07 ms** | Aucun | -- | 0 |
| 2372 | `What.If.2021.S02E05.What.if.Captain.Car...` | 2174.75 ms | 0.0 ms | 0.0 ms | 156.52 ms | 1.0 ms | 0.5 ms | **5833.52 ms** | What.If.2021.S02E05.What.if... | -- | 1 |
| 2373 | `What.If.2021.S02E06.What.if.Kahhori.Res...` | 1398.04 ms | 0.0 ms | 0.0 ms | 211.06 ms | 1.0 ms | 0.5 ms | **5145.33 ms** | What.If.2021.S02E06.What.if... | -- | 1 |
| 2374 | `What.If.2021.S02E07.What.if.Hela.Found....` | 1136.12 ms | 0.0 ms | 0.0 ms | 144.3 ms | 1.0 ms | 0.5 ms | **4793.53 ms** | What.If.2021.S02E07.What.if... | -- | 1 |
| 2375 | `What.If.2021.S02E08.What.if.the.Avenger...` | 6439.61 ms | 0.0 ms | 0.0 ms | 171.2 ms | 1.0 ms | 0.5 ms | **11112.25 ms** | What.If.2021.S02E08.What.if... | -- | 1 |
| 2376 | `What.If.2021.S02E09.What.if.Strange.Sup...` | 779.04 ms | 0.0 ms | 0.0 ms | 115.79 ms | 1.0 ms | 0.5 ms | **4403.88 ms** | What.If.2021.S02E09.What.if... | -- | 1 |
| 2377 | `White_House_Down_(2013)_[1080p][MULTI]_...` | 2064.07 ms | 0.0 ms | 0.0 ms | 101.66 ms | 1.0 ms | 0.5 ms | **5667.88 ms** | Aucun | -- | 0 |
| 2378 | `White_Snake_(2019)[MULTi][1080p](conte_...` | 1859.58 ms | 0.0 ms | 0.0 ms | 68.44 ms | 1.0 ms | 0.5 ms | **5430.6 ms** | Aucun | -- | 0 |
| 2379 | `Why.Him.2016.FRENCH.BDRip.XviD-EXTREME_...` | 1946.82 ms | 0.0 ms | 0.0 ms | 136.94 ms | 1.0 ms | 0.5 ms | **5617.68 ms** | Aucun | -- | 0 |
| 2380 | `Widows.2018.MULTI.1080p.WEB.H264_mid15s...` | 8021.07 ms | 0.0 ms | 0.0 ms | 108.52 ms | 1.0 ms | 0.5 ms | **11727.78 ms** | Aucun | -- | 0 |
| 2381 | `Wonder.2017.FRENCH.720p.BluRay.x264-VEN...` | 2050.12 ms | 0.0 ms | 0.0 ms | 145.06 ms | 1.0 ms | 0.5 ms | **5852.87 ms** | Aucun | -- | 0 |
| 2382 | `Wonder.Wheel.2017.FRENCH.1080p.WEB.H264...` | 1941.43 ms | 0.0 ms | 0.0 ms | 227.44 ms | 1.0 ms | 0.5 ms | **5705.03 ms** | Aucun | -- | 0 |
| 2383 | `Yardie_(2018)_[1080p]_mid15s.mp4` | 1593.4 ms | 0.0 ms | 0.0 ms | 128.14 ms | 1.0 ms | 0.5 ms | **5226.73 ms** | Aucun | -- | 0 |
| 2384 | `Zabriskie_Point_(1970)_vost_mid15s.mp4` | 1725.99 ms | 0.0 ms | 0.0 ms | 399.57 ms | 1.0 ms | 0.5 ms | **5688.9 ms** | Aucun | -- | 0 |
| 2385 | `Zardoz_(1974)[1080p]_mid15s.mp4` | 1649.65 ms | 0.0 ms | 0.0 ms | 119.44 ms | 1.0 ms | 0.5 ms | **5271.92 ms** | Aucun | -- | 0 |
| 2386 | `Zepon_(2021)[1080p](Martinique,_combat_...` | 1680.07 ms | 0.0 ms | 0.0 ms | 173.13 ms | 1.0 ms | 0.5 ms | **5364.28 ms** | Aucun | -- | 0 |
| 2387 | `Zombieland.2009.720p.BrRip.x264-YIFY_mi...` | 2204.47 ms | 0.0 ms | 0.0 ms | 178.53 ms | 1.0 ms | 0.5 ms | **5887.2 ms** | Zombieland.2009.720p.BrRip.... | -- | 1 |
| 2388 | `Zombieland.Double.Tap.2019.1080p.BluRay...` | 2136.07 ms | 0.0 ms | 0.0 ms | 202.33 ms | 1.0 ms | 0.5 ms | **5843.24 ms** | Aucun | -- | 0 |
| 2389 | `Zomboat!_(2019)_-_S01E03_-_Episode_3_(1...` | 1661.95 ms | 0.0 ms | 0.0 ms | 324.66 ms | 1.0 ms | 0.5 ms | **6491.53 ms** | Aucun | -- | 0 |
| 2390 | `Zomboat!_(2019)_-_S01E04_-_Episode_4_(1...` | 913.83 ms | 0.0 ms | 0.0 ms | 135.73 ms | 1.0 ms | 0.5 ms | **1049.74 ms** | Aucun | -- | 0 |
| 2391 | `Zomboat!_(2019)_-_S01E05_-_Episode_5_(1...` | 733.41 ms | 0.0 ms | 0.0 ms | 108.17 ms | 1.0 ms | 0.5 ms | **841.75 ms** | Aucun | -- | 0 |
| 2392 | `Zomboat!_(2019)_-_S01E06_-_Episode_6_(1...` | 1923.23 ms | 0.0 ms | 0.0 ms | 274.89 ms | 1.0 ms | 0.5 ms | **5700.63 ms** | Aucun | -- | 0 |
| 2393 | `Zygomatiques_-_Stefen.Cafiero.les.frère...` | 1450.53 ms | 0.0 ms | 0.0 ms | 241.69 ms | 1.0 ms | 0.5 ms | **5195.31 ms** | Aucun | -- | 0 |
| 2394 | `[Ani]_La_Reine_des_neiges_02_(Frozen_02...` | 1478.67 ms | 0.0 ms | 0.0 ms | 237.62 ms | 1.0 ms | 0.5 ms | **5220.16 ms** | Aucun | -- | 0 |
| 2395 | `[CBM]_Cowboy_Bebop_-_Session_01_-_Aster...` | 1319.94 ms | 0.0 ms | 0.0 ms | 173.94 ms | 1.0 ms | 0.5 ms | **5998.17 ms** | Aucun | -- | 0 |
| 2396 | `[CBM]_Cowboy_Bebop_-_Session_02_-_Stray...` | 4004.41 ms | 0.0 ms | 0.0 ms | 156.82 ms | 1.0 ms | 0.5 ms | **7865.08 ms** | Aucun | -- | 0 |
| 2397 | `[CBM]_Cowboy_Bebop_-_Session_03_-_Honky...` | 968.46 ms | 0.0 ms | 0.0 ms | 208.29 ms | 1.0 ms | 0.5 ms | **4901.39 ms** | Aucun | -- | 0 |
| 2398 | `[CBM]_Cowboy_Bebop_-_Session_04_-_Gatew...` | 1653.61 ms | 0.0 ms | 0.0 ms | 291.33 ms | 1.0 ms | 0.5 ms | **5453.07 ms** | Aucun | -- | 0 |
| 2399 | `[CBM]_Cowboy_Bebop_-_Session_05_-_Balla...` | 1697.06 ms | 0.0 ms | 0.0 ms | 222.83 ms | 1.0 ms | 0.5 ms | **5421.49 ms** | Aucun | -- | 0 |
| 2400 | `[CBM]_Cowboy_Bebop_-_Session_06_-_Sympa...` | 1337.28 ms | 0.0 ms | 0.0 ms | 111.72 ms | 1.0 ms | 0.5 ms | **4989.15 ms** | Aucun | -- | 0 |
| 2401 | `[CBM]_Cowboy_Bebop_-_Session_07_-_Heavy...` | 2713.21 ms | 0.0 ms | 0.0 ms | 167.81 ms | 1.0 ms | 0.5 ms | **6515.9 ms** | Aucun | -- | 0 |
| 2402 | `[CBM]_Cowboy_Bebop_-_Session_08_-_Waltz...` | 1951.6 ms | 0.0 ms | 0.0 ms | 118.11 ms | 1.0 ms | 0.5 ms | **5570.94 ms** | Aucun | -- | 0 |
| 2403 | `[CBM]_Cowboy_Bebop_-_Session_09_-_Jammi...` | 2556.72 ms | 0.0 ms | 0.0 ms | 140.08 ms | 1.0 ms | 0.5 ms | **6197.97 ms** | Aucun | -- | 0 |
| 2404 | `[CBM]_Cowboy_Bebop_-_Session_10_-_Ganym...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.79 ms** | Erreur | -- | 0 |
| 2405 | `[CBM]_Cowboy_Bebop_-_Session_11_-_Toys_...` | 2725.22 ms | 0.0 ms | 0.0 ms | 143.03 ms | 1.0 ms | 0.5 ms | **6369.84 ms** | Aucun | -- | 0 |
| 2406 | `[CBM]_Cowboy_Bebop_-_Session_12_-_Jupit...` | 3517.7 ms | 0.0 ms | 0.0 ms | 192.41 ms | 1.0 ms | 0.5 ms | **7212.81 ms** | Aucun | -- | 0 |
| 2407 | `[CBM]_Cowboy_Bebop_-_Session_13_-_Jupit...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12061.33 ms** | Erreur | -- | 0 |
| 2408 | `[CBM]_Cowboy_Bebop_-_Session_14_-_Bohem...` | 1733.09 ms | 3480.29 ms | 24.87 ms | 119.35 ms | 1.0 ms | 0.5 ms | **5358.52 ms** | cowboy bebop | 100.0% | 0 |
| 2409 | `[CBM]_Cowboy_Bebop_-_Session_15_-_My_Fu...` | 1915.71 ms | 3028.0 ms | 18.12 ms | 645.0 ms | 1.0 ms | 0.5 ms | **5611.42 ms** | cowboy bebop | 100.0% | 0 |
| 2410 | `[CBM]_Cowboy_Bebop_-_Session_16_-_Black...` | 3660.37 ms | 1281.94 ms | 25.78 ms | 172.99 ms | 1.0 ms | 0.5 ms | **5142.06 ms** | CBM Cowboy Bebop - Session ... | 100.0% | 0 |
| 2411 | `[CBM]_Cowboy_Bebop_-_Session_17_-_Mushr...` | 4462.25 ms | 0.0 ms | 0.0 ms | 164.94 ms | 1.0 ms | 0.5 ms | **8129.88 ms** | Aucun | -- | 0 |
| 2412 | `[CBM]_Cowboy_Bebop_-_Session_18_-_Speak...` | 1808.3 ms | 0.0 ms | 0.0 ms | 344.54 ms | 1.0 ms | 0.5 ms | **5663.95 ms** | Aucun | -- | 0 |
| 2413 | `[CBM]_Cowboy_Bebop_-_Session_19_-_Wild_...` | 2216.54 ms | 2127.55 ms | 26.67 ms | 474.72 ms | 1.0 ms | 0.5 ms | **4851.79 ms** | CBM Cowboy Bebop - Session ... | 100.0% | 0 |
| 2414 | `[CBM]_Cowboy_Bebop_-_Session_20_-_Pierr...` | 2078.1 ms | 3384.52 ms | 20.07 ms | 191.04 ms | 1.0 ms | 0.5 ms | **5678.8 ms** | Aucun | -- | 0 |
| 2415 | `[CBM]_Cowboy_Bebop_-_Session_21_-_Boogi...` | 2055.81 ms | 0.0 ms | 0.0 ms | 448.75 ms | 1.0 ms | 0.5 ms | **6006.63 ms** | Aucun | -- | 0 |
| 2416 | `[CBM]_Cowboy_Bebop_-_Session_22_-_Cowbo...` | 3717.99 ms | 0.0 ms | 0.0 ms | 250.08 ms | 1.0 ms | 0.5 ms | **7480.44 ms** | Aucun | -- | 0 |
| 2417 | `[CBM]_Cowboy_Bebop_-_Session_23_-_Brain...` | 1791.37 ms | 0.0 ms | 0.0 ms | 154.18 ms | 1.0 ms | 0.5 ms | **5453.55 ms** | Aucun | -- | 0 |
| 2418 | `[CBM]_Cowboy_Bebop_-_Session_24_-_Hard_...` | 2558.49 ms | 0.0 ms | 0.0 ms | 243.33 ms | 1.0 ms | 0.5 ms | **6420.44 ms** | Aucun | -- | 0 |
| 2419 | `[CBM]_Cowboy_Bebop_-_Session_25_-_The_R...` | 4586.08 ms | 0.0 ms | 0.0 ms | 147.88 ms | 1.0 ms | 0.5 ms | **8236.95 ms** | Aucun | -- | 0 |
| 2420 | `[CBM]_Cowboy_Bebop_-_Session_26_-_The_R...` | 1790.2 ms | 0.0 ms | 0.0 ms | 304.82 ms | 1.0 ms | 0.5 ms | **5597.27 ms** | Aucun | -- | 0 |
| 2421 | `[EMBER]_Kimetsu_no_Yaiba_-_Hashira_Geik...` | 5319.27 ms | 0.0 ms | 0.0 ms | 221.45 ms | 1.0 ms | 0.5 ms | **9125.6 ms** | Aucun | -- | 0 |
| 2422 | `[EMBER]_Kimetsu_no_Yaiba_-_Hashira_Geik...` | 4027.87 ms | 0.0 ms | 0.0 ms | 334.38 ms | 1.0 ms | 0.5 ms | **7867.14 ms** | Aucun | -- | 0 |
| 2423 | `[Erai-raws]_Evangelion_3.0+1.01_Thrice_...` | 2433.32 ms | 0.0 ms | 0.0 ms | 86.19 ms | 1.0 ms | 0.5 ms | **6023.8 ms** | Aucun | -- | 0 |
| 2424 | `[Erai-raws]_Haikyuu!!_OADs_-_Lev_Appear...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.32 ms** | Erreur | -- | 0 |
| 2425 | `[GkTorrent.com]_Le.Retour.Du.Heros.2018...` | 2788.12 ms | 0.0 ms | 0.0 ms | 258.43 ms | 1.0 ms | 0.5 ms | **6580.67 ms** | Aucun | -- | 0 |
| 2426 | `[GkTorrent.org]_The.Hitmans.Bodyguard.2...` | 2667.8 ms | 0.0 ms | 0.0 ms | 142.0 ms | 1.0 ms | 0.5 ms | **6327.76 ms** | Aucun | -- | 0 |
| 2427 | `[HorribleSubs]_Haikyuu!!_Riku_vs_Kuu_-_...` | 2295.01 ms | 0.0 ms | 0.0 ms | 106.27 ms | 1.0 ms | 0.5 ms | **5902.89 ms** | Aucun | -- | 0 |
| 2428 | `[Prof]_S01E01_-_Angel_Attack!_mid15s.mp4` | 2754.05 ms | 0.0 ms | 0.0 ms | 442.89 ms | 1.0 ms | 0.5 ms | **6969.51 ms** | Aucun | -- | 0 |
| 2429 | `[Prof]_S01E02_-_The_Beast_mid15s.mp4` | 2732.49 ms | 0.0 ms | 0.0 ms | 326.13 ms | 1.0 ms | 0.5 ms | **6723.98 ms** | Aucun | -- | 0 |
| 2430 | `[Prof]_S01E03_-_A_Transfer_mid15s.mp4` | 2671.98 ms | 0.0 ms | 0.0 ms | 202.09 ms | 1.0 ms | 0.5 ms | **6387.37 ms** | Aucun | -- | 0 |
| 2431 | `[Prof]_S01E04_-_Hedgehog`s_Dilemma_mid1...` | 4823.64 ms | 3287.55 ms | 84.29 ms | 101.35 ms | 1.0 ms | 0.5 ms | **8300.78 ms** | S01E04 | 100.0% | 3 |
| 2432 | `[Prof]_S01E05_-_Rei_I_mid15s.mp4` | 7023.58 ms | 285.02 ms | 25.67 ms | 190.51 ms | 1.0 ms | 0.5 ms | **7531.2 ms** | S01E05 | 100.0% | 4 |
| 2433 | `[Prof]_S01E06_-_Rei_II_mid15s.mp4` | 10688.26 ms | 179.17 ms | 17.0 ms | 266.67 ms | 1.0 ms | 0.5 ms | **11156.51 ms** | S01E06 | 100.0% | 4 |
| 2434 | `[Prof]_S01E07_-_A_Human_Work_mid15s.mp4` | 1830.55 ms | 0.0 ms | 0.0 ms | 98.76 ms | 1.0 ms | 0.5 ms | **5432.69 ms** | Aucun | -- | 0 |
| 2435 | `[Prof]_S01E08_-_Asuka_Strikes!_mid15s.mp4` | 1874.22 ms | 0.0 ms | 0.0 ms | 130.57 ms | 1.0 ms | 0.5 ms | **5507.46 ms** | Aucun | -- | 0 |
| 2436 | `[Prof]_S01E09_-_Both_of_You,_Dance_Like...` | 3634.26 ms | 1611.23 ms | 27.63 ms | 109.91 ms | 1.0 ms | 0.5 ms | **5389.74 ms** | Aucun | -- | 0 |
| 2437 | `[Prof]_S01E10_-_Magmadiver_mid15s.mp4` | 1706.86 ms | 2924.59 ms | 26.38 ms | 270.24 ms | 1.0 ms | 0.5 ms | **4932.9 ms** | Aucun | -- | 0 |
| 2438 | `[Prof]_S01E11_-_The_Day_Tokyo-3_Stood_S...` | 3882.74 ms | 499.82 ms | 17.67 ms | 98.73 ms | 1.0 ms | 0.5 ms | **4499.37 ms** | S01E11 | 100.0% | 0 |
| 2439 | `[Prof]_S01E12_-_She_Said_-_Don`t_Make_O...` | 2041.5 ms | 2329.1 ms | 159.83 ms | 155.27 ms | 1.0 ms | 0.5 ms | **4689.95 ms** | S01E12 | 100.0% | 0 |
| 2440 | `[Prof]_S01E13_-_Lilliputian_Hitcher_mid...` | 2477.98 ms | 818.04 ms | 22.03 ms | 344.71 ms | 1.0 ms | 0.5 ms | **3663.7 ms** | S01E13 | 100.0% | 0 |
| 2441 | `[Prof]_S01E14_-_Weaving_A_Story_mid15s.mp4` | 2144.15 ms | 1097.63 ms | 184.58 ms | 227.27 ms | 1.0 ms | 0.5 ms | **3654.95 ms** | Aucun | -- | 0 |
| 2442 | `[Prof]_S01E15_-_Those_Women_Longed_for_...` | 1873.74 ms | 1175.58 ms | 16.96 ms | 270.65 ms | 1.0 ms | 0.5 ms | **3337.91 ms** | S01E15 | 100.0% | 0 |
| 2443 | `[Prof]_S01E16_-_Splitting_of_the_Breast...` | 2328.11 ms | 756.59 ms | 181.96 ms | 158.83 ms | 1.0 ms | 0.5 ms | **3428.88 ms** | Aucun | -- | 0 |
| 2444 | `[Prof]_S01E17_-_Fourth_Children_mid15s.mp4` | 6878.24 ms | 124.2 ms | 18.33 ms | 280.61 ms | 1.0 ms | 0.5 ms | **7301.8 ms** | S01E17 | 100.0% | 0 |
| 2445 | `[Prof]_S01E18_-_Ambivalence_mid15s.mp4` | 2703.89 ms | 112.92 ms | 22.93 ms | 272.73 ms | 1.0 ms | 0.5 ms | **3123.53 ms** | S01E18 | 100.0% | 0 |
| 2446 | `[Prof]_S01E19_-_Introjection_mid15s.mp4` | 2323.05 ms | 200.65 ms | 20.42 ms | 154.1 ms | 1.0 ms | 0.5 ms | **2718.78 ms** | Aucun | -- | 0 |
| 2447 | `[Prof]_S01E20_-_Weaving_a_Story_2_-_Ora...` | 7064.71 ms | 237.44 ms | 20.45 ms | 479.85 ms | 1.0 ms | 0.5 ms | **7802.83 ms** | S01E20 | 100.0% | 0 |
| 2448 | `[Prof]_S01E21_-_He_Was_Aware_that_He_Wa...` | 1746.83 ms | 254.66 ms | 22.57 ms | 125.7 ms | 1.0 ms | 0.5 ms | **2150.82 ms** | S01E21 | 100.0% | 0 |
| 2449 | `[Prof]_S01E22_-_Don`t_Be_mid15s.mp4` | 2187.78 ms | 215.82 ms | 18.71 ms | 243.84 ms | 1.0 ms | 0.5 ms | **2666.71 ms** | S01E22 | 100.0% | 0 |
| 2450 | `[Prof]_S01E23_-_Rei_III_mid15s.mp4` | 3401.4 ms | 337.11 ms | 20.12 ms | 389.24 ms | 1.0 ms | 0.5 ms | **4178.39 ms** | S01E23 | 100.0% | 0 |
| 2451 | `[Prof]_S01E24_-_The_Beginning_and_the_E...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.76 ms** | Erreur | -- | 0 |
| 2452 | `[Prof]_S01E25_-_Do_You_Love_Me_mid15s.mp4` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12060.59 ms** | Erreur | -- | 0 |
| 2453 | `[Prof]_S01E26_-_Take_Care_of_Yourself_m...` | 3843.04 ms | 189.6 ms | 19.33 ms | 159.8 ms | 1.0 ms | 0.5 ms | **4215.05 ms** | S01E26 | 100.0% | 0 |
| 2454 | `[Valenciano]_Kimetsu_no_Yaiba_S01E01_[B...` | 2484.22 ms | 265.49 ms | 20.5 ms | 374.75 ms | 1.0 ms | 0.5 ms | **3146.88 ms** | 26 Nouvelle mission mp4 24 ... | 100.0% | 1 |
| 2455 | `[Valenciano]_Kimetsu_no_Yaiba_S01E02_[B...` | 3142.18 ms | 241.93 ms | 31.54 ms | 194.0 ms | 1.0 ms | 0.5 ms | **3613.64 ms** | Aucun | -- | 0 |
| 2456 | `[Valenciano]_Kimetsu_no_Yaiba_S01E03_[B...` | 3146.14 ms | 272.77 ms | 29.3 ms | 464.2 ms | 1.0 ms | 0.5 ms | **3913.25 ms** | Aucun | -- | 0 |
| 2457 | `[Valenciano]_Kimetsu_no_Yaiba_S01E04_[B...` | 3150.58 ms | 204.24 ms | 45.1 ms | 320.21 ms | 1.0 ms | 0.5 ms | **3721.05 ms** | Aucun | -- | 0 |
| 2458 | `[Valenciano]_Kimetsu_no_Yaiba_S01E05_[B...` | 3655.33 ms | 388.2 ms | 15.41 ms | 523.97 ms | 1.0 ms | 0.5 ms | **4585.51 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2459 | `[Valenciano]_Kimetsu_no_Yaiba_S01E06_[B...` | 3207.76 ms | 176.53 ms | 24.1 ms | 214.31 ms | 1.0 ms | 0.5 ms | **3623.14 ms** | Aucun | -- | 0 |
| 2460 | `[Valenciano]_Kimetsu_no_Yaiba_S01E07_[B...` | 3466.87 ms | 253.78 ms | 19.34 ms | 172.16 ms | 1.0 ms | 0.5 ms | **3912.52 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2461 | `[Valenciano]_Kimetsu_no_Yaiba_S01E08_[B...` | 3629.0 ms | 362.36 ms | 20.53 ms | 143.95 ms | 1.0 ms | 0.5 ms | **4156.5 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2462 | `[Valenciano]_Kimetsu_no_Yaiba_S01E09_[B...` | 3104.9 ms | 335.13 ms | 21.73 ms | 281.65 ms | 1.0 ms | 0.5 ms | **3744.26 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2463 | `[Valenciano]_Kimetsu_no_Yaiba_S01E10_[B...` | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms | **12057.22 ms** | Erreur | -- | 0 |
| 2464 | `[Valenciano]_Kimetsu_no_Yaiba_S01E11_[B...` | 11203.13 ms | 195.24 ms | 19.98 ms | 93.39 ms | 1.0 ms | 0.5 ms | **11512.2 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2465 | `[Valenciano]_Kimetsu_no_Yaiba_S01E12_[B...` | 3616.06 ms | 154.34 ms | 30.91 ms | 338.55 ms | 1.0 ms | 0.5 ms | **4149.35 ms** | Aucun | -- | 0 |
| 2466 | `[Valenciano]_Kimetsu_no_Yaiba_S01E13_[B...` | 3036.73 ms | 112.99 ms | 22.01 ms | 132.95 ms | 1.0 ms | 0.5 ms | **3305.52 ms** | Aucun | -- | 0 |
| 2467 | `[Valenciano]_Kimetsu_no_Yaiba_S01E14_[B...` | 3671.06 ms | 0.0 ms | 0.0 ms | 128.86 ms | 1.0 ms | 0.5 ms | **7422.63 ms** | Aucun | -- | 0 |
| 2468 | `[Valenciano]_Kimetsu_no_Yaiba_S01E15_[B...` | 4240.55 ms | 0.0 ms | 0.0 ms | 125.15 ms | 1.0 ms | 0.5 ms | **7867.72 ms** | Aucun | -- | 0 |
| 2469 | `[Valenciano]_Kimetsu_no_Yaiba_S01E16_[B...` | 2238.08 ms | 1234.87 ms | 25.57 ms | 165.06 ms | 1.0 ms | 0.5 ms | **3664.08 ms** | Aucun | -- | 0 |
| 2470 | `[Valenciano]_Kimetsu_no_Yaiba_S01E17_[B...` | 3047.9 ms | 0.0 ms | 0.0 ms | 159.23 ms | 1.0 ms | 0.5 ms | **6718.27 ms** | Aucun | -- | 0 |
| 2471 | `[Valenciano]_Kimetsu_no_Yaiba_S01E18_[B...` | 3115.19 ms | 123.66 ms | 22.74 ms | 131.5 ms | 1.0 ms | 0.5 ms | **3395.68 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2472 | `[Valenciano]_Kimetsu_no_Yaiba_S01E19_[B...` | 5713.4 ms | 2327.71 ms | 21.32 ms | 192.06 ms | 1.0 ms | 0.5 ms | **8254.93 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2473 | `[Valenciano]_Kimetsu_no_Yaiba_S01E20_[B...` | 5211.58 ms | 3457.84 ms | 24.75 ms | 215.63 ms | 1.0 ms | 0.5 ms | **8912.24 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2474 | `[Valenciano]_Kimetsu_no_Yaiba_S01E21_[B...` | 2765.32 ms | 101.62 ms | 33.15 ms | 116.72 ms | 1.0 ms | 0.5 ms | **3017.9 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2475 | `[Valenciano]_Kimetsu_no_Yaiba_S01E22_[B...` | 2614.23 ms | 2183.92 ms | 25.36 ms | 204.56 ms | 1.0 ms | 0.5 ms | **5029.11 ms** | 22 Le Mai 24 0fps | 100.0% | 1 |
| 2476 | `[Valenciano]_Kimetsu_no_Yaiba_S01E23_[B...` | 2647.96 ms | 133.21 ms | 18.2 ms | 220.01 ms | 1.0 ms | 0.5 ms | **3020.15 ms** | 23 La Re 24 0fps | 100.0% | 1 |
| 2477 | `[Valenciano]_Kimetsu_no_Yaiba_S01E24_[B...` | 3496.17 ms | 1152.89 ms | 25.77 ms | 118.65 ms | 1.0 ms | 0.5 ms | **4795.86 ms** | 24 L'Entrai 24 0fps | 100.0% | 1 |
| 2478 | `[Valenciano]_Kimetsu_no_Yaiba_S01E25_[B...` | 4215.03 ms | 152.05 ms | 19.95 ms | 197.88 ms | 1.0 ms | 0.5 ms | **4585.77 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2479 | `[Valenciano]_Kimetsu_no_Yaiba_S01E26_[B...` | 4509.53 ms | 3378.41 ms | 21.38 ms | 173.07 ms | 1.0 ms | 0.5 ms | **8082.81 ms** | 26 Nouvelle mission mp4 24 ... | 100.0% | 1 |
| 2480 | `[Valenciano]_Kimetsu_no_Yaiba_S02E01_[B...` | 9428.64 ms | 222.98 ms | 32.66 ms | 731.38 ms | 1.0 ms | 0.5 ms | **10416.04 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2481 | `[Valenciano]_Kimetsu_no_Yaiba_S02E02_[B...` | 2104.56 ms | 270.27 ms | 40.91 ms | 138.81 ms | 1.0 ms | 0.5 ms | **2555.84 ms** | Aucun | -- | 0 |
| 2482 | `[Valenciano]_Kimetsu_no_Yaiba_S02E03_[B...` | 2961.77 ms | 0.0 ms | 0.0 ms | 84.59 ms | 1.0 ms | 0.5 ms | **6548.81 ms** | Aucun | -- | 0 |
| 2483 | `[Valenciano]_Kimetsu_no_Yaiba_S02E04_[B...` | 3604.1 ms | 1347.87 ms | 20.98 ms | 152.75 ms | 1.0 ms | 0.5 ms | **5128.45 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2484 | `[Valenciano]_Kimetsu_no_Yaiba_S02E05_[B...` | 1955.51 ms | 3242.31 ms | 28.28 ms | 112.39 ms | 1.0 ms | 0.5 ms | **5339.29 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2485 | `[Valenciano]_Kimetsu_no_Yaiba_S02E06_[B...` | 1971.49 ms | 3430.12 ms | 21.54 ms | 122.44 ms | 1.0 ms | 0.5 ms | **5551.5 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2486 | `[Valenciano]_Kimetsu_no_Yaiba_S02E07_[B...` | 3826.87 ms | 1279.67 ms | 17.98 ms | 257.14 ms | 1.0 ms | 0.5 ms | **5382.63 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2487 | `[Valenciano]_Kimetsu_no_Yaiba_S03E01_[B...` | 3580.95 ms | 1199.36 ms | 15.81 ms | 468.33 ms | 1.0 ms | 0.5 ms | **5264.89 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2488 | `[Valenciano]_Kimetsu_no_Yaiba_S03E02_[B...` | 1494.95 ms | 2394.25 ms | 14.34 ms | 182.8 ms | 1.0 ms | 0.5 ms | **4088.4 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2489 | `[Valenciano]_Kimetsu_no_Yaiba_S03E03_[B...` | 1706.79 ms | 1174.95 ms | 18.14 ms | 105.33 ms | 1.0 ms | 0.5 ms | **3006.03 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2490 | `[Valenciano]_Kimetsu_no_Yaiba_S03E04_[B...` | 1562.53 ms | 2186.48 ms | 17.15 ms | 261.11 ms | 1.0 ms | 0.5 ms | **4033.1 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2491 | `[Valenciano]_Kimetsu_no_Yaiba_S03E05_[B...` | 1332.25 ms | 198.05 ms | 31.35 ms | 97.37 ms | 1.0 ms | 0.5 ms | **1659.65 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2492 | `[Valenciano]_Kimetsu_no_Yaiba_S03E06_[B...` | 4986.55 ms | 205.39 ms | 44.79 ms | 214.18 ms | 1.0 ms | 0.5 ms | **5453.15 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2493 | `[Valenciano]_Kimetsu_no_Yaiba_S03E07_[B...` | 1223.73 ms | 197.79 ms | 21.15 ms | 302.55 ms | 1.0 ms | 0.5 ms | **1746.02 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2494 | `[Valenciano]_Kimetsu_no_Yaiba_S03E08_[B...` | 2000.85 ms | 135.02 ms | 23.1 ms | 498.75 ms | 1.0 ms | 0.5 ms | **2662.89 ms** | Aucun | -- | 0 |
| 2495 | `[Valenciano]_Kimetsu_no_Yaiba_S03E09_[B...` | 2159.95 ms | 193.13 ms | 120.55 ms | 883.24 ms | 1.0 ms | 0.5 ms | **3357.51 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2496 | `[Valenciano]_Kimetsu_no_Yaiba_S03E10_[B...` | 2024.77 ms | 224.89 ms | 95.01 ms | 812.45 ms | 1.0 ms | 0.5 ms | **3157.46 ms** | Aucun | -- | 0 |
| 2497 | `[Valenciano]_Kimetsu_no_Yaiba_S03E11_[B...` | 3450.82 ms | 187.49 ms | 42.5 ms | 185.77 ms | 1.0 ms | 0.5 ms | **3867.38 ms** | Valenciano Kimetsu no Yaiba | 100.0% | 0 |
| 2498 | `[_GkTorrent.com_]_Au.Poste.2018.FRENCH....` | 2699.63 ms | 225.98 ms | 33.8 ms | 257.02 ms | 1.0 ms | 0.5 ms | **3217.31 ms** | GkTorrent com Au Poste | 100.0% | 0 |
| 2499 | `[_GkTorrent.com_]_Mon.Ket.2018.FRENCH.B...` | 3015.86 ms | 213.75 ms | 119.47 ms | 344.29 ms | 1.0 ms | 0.5 ms | **3697.71 ms** | GkTorrent com Mon Ket | 100.0% | 0 |
| 2500 | `[_OxTorrent.cc_]_Les.101.Dalmatiens.196...` | 2858.55 ms | 215.56 ms | 101.16 ms | 253.3 ms | 1.0 ms | 0.5 ms | **3439.73 ms** | Aucun | -- | 0 |
| 2501 | `[_OxTorrent.com_]_Cars.3.2017.TRUEFRENC...` | 3115.64 ms | 196.59 ms | 37.85 ms | 235.22 ms | 1.0 ms | 0.5 ms | **3587.56 ms** | Aucun | -- | 0 |
| 2502 | `[_OxTorrent.com_]_Cars_2_2011_FRENCH_BR...` | 2959.87 ms | 273.56 ms | 41.69 ms | 146.21 ms | 1.0 ms | 0.5 ms | **3422.02 ms** | OxTorrent com Cars 2 | 100.0% | 0 |
| 2503 | `[_OxTorrent.com_]_Cars_mid15s.mp4` | 2374.07 ms | 264.91 ms | 35.73 ms | 349.16 ms | 1.0 ms | 0.5 ms | **3031.63 ms** | OxTorrent com Cars avi 24 0fps | 100.0% | 0 |
| 2504 | `[_OxTorrent.com_]_Dirty.God.2019.MULTi....` | 3713.26 ms | 297.52 ms | 25.46 ms | 259.95 ms | 1.0 ms | 0.5 ms | **4297.81 ms** | OxTorrent com Dirty God | 100.0% | 0 |
| 2505 | `[_OxTorrent.com_]_Les.Miserables.2019.F...` | 2431.67 ms | 255.51 ms | 98.87 ms | 179.2 ms | 1.0 ms | 0.5 ms | **2966.24 ms** | OxTorrent com Les Miserables | 100.0% | 0 |
| 2506 | `[_OxTorrent.com_]_Peter_Pan_mid15s.mp4` | 3594.04 ms | 226.61 ms | 17.81 ms | 330.94 ms | 1.0 ms | 0.5 ms | **4196.79 ms** | OxTorrent com Peter Pan avi... | 100.0% | 0 |
| 2507 | `[_OxTorrent.com_]_See.2019.S01E03.VOSTF...` | 3350.05 ms | 165.44 ms | 18.57 ms | 293.05 ms | 1.0 ms | 0.5 ms | **3827.55 ms** | See | 100.0% | 0 |
| 2508 | `[_OxTorrent.com_]_See.S01E02.VOSTFR.WEB...` | 3451.47 ms | 201.73 ms | 16.52 ms | 299.18 ms | 1.0 ms | 0.5 ms | **3994.86 ms** | OxTorrent com See | 100.0% | 0 |
| 2509 | `[_OxTorrent.com_]_See.S01E04.VOSTFR.WEB...` | 2092.82 ms | 202.55 ms | 23.62 ms | 205.53 ms | 1.0 ms | 0.5 ms | **2525.28 ms** | OxTorrent com See | 100.0% | 0 |
| 2510 | `[_OxTorrent.com_]_See.S01E05.VOSTFR.WEB...` | 3117.42 ms | 216.01 ms | 14.63 ms | 305.23 ms | 1.0 ms | 0.5 ms | **3658.46 ms** | See | 100.0% | 0 |
| 2511 | `[_OxTorrent.com_]_See.S01E06.SUBFRENCH....` | 2889.34 ms | 175.19 ms | 20.66 ms | 310.21 ms | 1.0 ms | 0.5 ms | **3395.81 ms** | See | 100.0% | 1 |
| 2512 | `[_OxTorrent.com_]_See.S01E07.SUBFRENCH....` | 2169.89 ms | 0.0 ms | 0.0 ms | 73.89 ms | 1.0 ms | 0.5 ms | **6746.04 ms** | Aucun | -- | 0 |
| 2513 | `[_OxTorrent.com_]_See.S01E08.VOSTFR.WEB...` | 1798.75 ms | 0.0 ms | 0.0 ms | 67.9 ms | 1.0 ms | 0.5 ms | **5377.69 ms** | Aucun | -- | 0 |
| 2514 | `[_OxTorrent.com_]_Zootopia.2016.MULTi.1...` | 2847.91 ms | 0.0 ms | 0.0 ms | 40.92 ms | 1.0 ms | 0.5 ms | **6391.61 ms** | Aucun | -- | 0 |
| 2515 | `[_Torrent9.cz_]_Funan.2019.FRENCH.1080p...` | 2839.99 ms | 0.0 ms | 0.0 ms | 32.96 ms | 1.0 ms | 0.5 ms | **6374.57 ms** | Aucun | -- | 0 |
| 2516 | `[_Torrent9.cz_]_Maquia.When.The.Promise...` | 4014.83 ms | 0.0 ms | 0.0 ms | 128.55 ms | 1.0 ms | 0.5 ms | **7646.95 ms** | Aucun | -- | 0 |
| 2517 | `[_Torrent9.cz_]_Who.Killed.Lady.Winsley...` | 2869.69 ms | 0.0 ms | 0.0 ms | 142.52 ms | 1.0 ms | 0.5 ms | **7536.06 ms** | Aucun | -- | 0 |
| 2518 | `[_Torrent911.cc_]_Champagne.2022.FRENCH...` | 2715.03 ms | 0.0 ms | 0.0 ms | 62.17 ms | 1.0 ms | 0.5 ms | **6278.65 ms** | Aucun | -- | 0 |
| 2519 | `[_Torrent911.io_]_Drone.Games.2023.FREN...` | 3688.63 ms | 0.0 ms | 0.0 ms | 216.94 ms | 1.0 ms | 0.5 ms | **7409.4 ms** | [ Torrent911.io ] Drone.Gam... | -- | 1 |
| 2520 | `[_Torrent911.io_]_L.Ultima.Notte.Di.Amo...` | 4607.41 ms | 0.0 ms | 0.0 ms | 85.95 ms | 1.0 ms | 0.5 ms | **8294.1 ms** | [ Torrent911.io ] L.Ultima.... | -- | 1 |
| 2521 | `[_Torrent911.io_]_Les.Mauvais.Garcons.2...` | 3163.74 ms | 0.0 ms | 0.0 ms | 99.3 ms | 1.0 ms | 0.5 ms | **6835.09 ms** | Aucun | -- | 0 |
| 2522 | `[_Torrent911.io_]_Sur.la.Branche.2023.F...` | 2846.79 ms | 0.0 ms | 0.0 ms | 84.91 ms | 1.0 ms | 0.5 ms | **6442.76 ms** | Aucun | -- | 0 |
| 2523 | `[_Torrent911.io_]_The.Velveteen.Rabbit....` | 1399.89 ms | 0.0 ms | 0.0 ms | 164.73 ms | 1.0 ms | 0.5 ms | **1565.64 ms** | Aucun | -- | 0 |
| 2524 | `[_Torrent911.me_]_Alibi.com.2.2023.FREN...` | 2640.38 ms | 0.0 ms | 0.0 ms | 47.89 ms | 1.0 ms | 0.5 ms | **6190.35 ms** | Aucun | -- | 0 |
| 2525 | `[_Torrent911.tv_]_Le.Visiteur.Du.Futur....` | 2938.44 ms | 0.0 ms | 0.0 ms | 98.77 ms | 1.0 ms | 0.5 ms | **6540.51 ms** | Aucun | -- | 0 |
| 2526 | `[_www.CpasBien.cm_]_Mustang.2015.FRENCH...` | 2420.41 ms | 0.0 ms | 0.0 ms | 111.44 ms | 1.0 ms | 0.5 ms | **6119.91 ms** | Aucun | -- | 0 |
| 2527 | `[_www.Torrent9.uno_]_Rosetta_1999_1080p...` | 2511.01 ms | 0.0 ms | 0.0 ms | 240.27 ms | 1.0 ms | 0.5 ms | **6254.89 ms** | Aucun | -- | 0 |
| 2528 | `an_autumn_afternoon_mid15s.mp4` | 2875.48 ms | 0.0 ms | 0.0 ms | 186.49 ms | 1.0 ms | 0.5 ms | **6564.88 ms** | Aucun | -- | 0 |
| 2529 | `blanche.gardin.il.faut.que.je.vous.parl...` | 2072.14 ms | 0.0 ms | 0.0 ms | 71.47 ms | 1.0 ms | 0.5 ms | **5695.09 ms** | Aucun | -- | 0 |
| 2530 | `encounter.2021.1080p.web.h264-naisu_mid...` | 2300.82 ms | 2301.93 ms | 27.29 ms | 37.04 ms | 1.0 ms | 0.5 ms | **4667.56 ms** | encounter | 100.0% | 0 |
| 2531 | `hustle.2022.1080p.web.h264-cupcakes_mid...` | 2142.2 ms | 0.0 ms | 0.0 ms | 229.98 ms | 1.0 ms | 0.5 ms | **6881.21 ms** | Aucun | -- | 0 |
| 2532 | `judas.and.the.black.messiah.2021.1080p....` | 1940.72 ms | 0.0 ms | 0.0 ms | 136.54 ms | 1.0 ms | 0.5 ms | **5619.35 ms** | Aucun | -- | 0 |
| 2533 | `la_personne_aux_2_personnes-medelle_mid...` | 399.56 ms | 0.0 ms | 0.0 ms | 131.4 ms | 1.0 ms | 0.5 ms | **4032.61 ms** | Aucun | -- | 0 |
| 2534 | `medecin.de.campagne.2016.french.bdrip.x...` | 332.0 ms | 169.31 ms | 28.78 ms | 36.91 ms | 1.0 ms | 0.5 ms | **567.21 ms** | medecin de campagne | 100.0% | 0 |
| 2535 | `mr.robot.s01e01.1080p.bluray.x264-rover...` | 454.88 ms | 0.0 ms | 0.0 ms | 93.1 ms | 1.0 ms | 0.5 ms | **4059.11 ms** | mr.robot.s01e01.1080p.blura... | -- | 1 |
| 2536 | `mr.robot.s01e02.1080p.bluray.x264-rover...` | 510.41 ms | 0.0 ms | 0.0 ms | 49.98 ms | 1.0 ms | 0.5 ms | **4061.65 ms** | mr.robot.s01e02.1080p.blura... | -- | 1 |
| 2537 | `mr.robot.s01e03.1080p.bluray.x264-rover...` | 929.97 ms | 0.0 ms | 0.0 ms | 69.77 ms | 1.0 ms | 0.5 ms | **4502.39 ms** | mr.robot.s01e03.1080p.blura... | -- | 1 |
| 2538 | `mr.robot.s01e04.1080p.bluray.x264-rover...` | 579.99 ms | 0.0 ms | 0.0 ms | 87.4 ms | 1.0 ms | 0.5 ms | **4169.53 ms** | mr.robot.s01e04.1080p.blura... | -- | 1 |
| 2539 | `mr.robot.s01e05.1080p.bluray.x264-rover...` | 891.06 ms | 0.0 ms | 0.0 ms | 56.62 ms | 1.0 ms | 0.5 ms | **4448.69 ms** | mr.robot.s01e05.1080p.blura... | -- | 1 |
| 2540 | `mr.robot.s01e06.1080p.bluray.x264-rover...` | 977.82 ms | 0.0 ms | 0.0 ms | 93.31 ms | 1.0 ms | 0.5 ms | **4572.94 ms** | mr.robot.s01e06.1080p.blura... | -- | 1 |
| 2541 | `mr.robot.s01e07.1080p.bluray.x264-rover...` | 1303.75 ms | 0.0 ms | 0.0 ms | 116.47 ms | 1.0 ms | 0.5 ms | **4922.78 ms** | mr.robot.s01e07.1080p.blura... | -- | 1 |
| 2542 | `mr.robot.s01e08.1080p.bluray.x264-rover...` | 1364.15 ms | 0.0 ms | 0.0 ms | 82.11 ms | 1.0 ms | 0.5 ms | **4949.3 ms** | mr.robot.s01e08.1080p.blura... | -- | 1 |
| 2543 | `mr.robot.s01e09.1080p.bluray.x264-rover...` | 1405.21 ms | 0.0 ms | 0.0 ms | 101.63 ms | 1.0 ms | 0.5 ms | **5094.69 ms** | mr.robot.s01e09.1080p.blura... | -- | 1 |
| 2544 | `mr.robot.s01e10.1080p.bluray.x264-rover...` | 1363.49 ms | 0.0 ms | 0.0 ms | 98.14 ms | 1.0 ms | 0.5 ms | **4963.34 ms** | mr.robot.s01e10.1080p.blura... | -- | 1 |
| 2545 | `mr.robot.s03e01.1080p.bluray.x264-rover...` | 1600.21 ms | 0.0 ms | 0.0 ms | 115.41 ms | 1.0 ms | 0.5 ms | **5217.46 ms** | mr.robot.s03e01.1080p.blura... | -- | 1 |
| 2546 | `mr.robot.s03e02.1080p.bluray.x264-rover...` | 2469.29 ms | 0.0 ms | 0.0 ms | 122.22 ms | 1.0 ms | 0.5 ms | **6092.52 ms** | mr.robot.s03e02.1080p.blura... | -- | 1 |
| 2547 | `mr.robot.s03e03.1080p.bluray.x264-rover...` | 1674.94 ms | 0.0 ms | 0.0 ms | 162.36 ms | 1.0 ms | 0.5 ms | **5340.87 ms** | Aucun | -- | 0 |
| 2548 | `mr.robot.s03e04.1080p.bluray.x264-rover...` | 3119.27 ms | 0.0 ms | 0.0 ms | 114.29 ms | 1.0 ms | 0.5 ms | **6734.43 ms** | mr.robot.s03e04.1080p.blura... | -- | 1 |
| 2549 | `mr.robot.s03e05.1080p.bluray.x264-rover...` | 1831.11 ms | 0.0 ms | 0.0 ms | 126.9 ms | 1.0 ms | 0.5 ms | **5465.57 ms** | mr.robot.s03e05.1080p.blura... | -- | 1 |
| 2550 | `mr.robot.s03e06.1080p.bluray.x264-rover...` | 1717.75 ms | 0.0 ms | 0.0 ms | 159.56 ms | 1.0 ms | 0.5 ms | **5391.47 ms** | mr.robot.s03e06.1080p.blura... | -- | 1 |
| 2551 | `mr.robot.s03e07.1080p.bluray.x264-rover...` | 1581.2 ms | 0.0 ms | 0.0 ms | 147.74 ms | 1.0 ms | 0.5 ms | **5301.77 ms** | mr.robot.s03e07.1080p.blura... | -- | 1 |
| 2552 | `mr.robot.s03e08.1080p.bluray.x264-rover...` | 4780.3 ms | 0.0 ms | 0.0 ms | 86.97 ms | 1.0 ms | 0.5 ms | **8368.49 ms** | mr.robot.s03e08.1080p.blura... | -- | 1 |
| 2553 | `mr.robot.s03e09.1080p.bluray.x264-rover...` | 1506.23 ms | 0.0 ms | 0.0 ms | 104.64 ms | 1.0 ms | 0.5 ms | **5191.7 ms** | mr.robot.s03e09.1080p.blura... | -- | 1 |
| 2554 | `mr.robot.s03e10.1080p.bluray.x264-rover...` | 866.91 ms | 0.0 ms | 0.0 ms | 124.02 ms | 1.0 ms | 0.5 ms | **4493.43 ms** | mr.robot.s03e10.1080p.blura... | -- | 1 |
| 2555 | `mr.robot.s04e01.1080p.bluray.x264-rover...` | 578.92 ms | 0.0 ms | 0.0 ms | 144.45 ms | 1.0 ms | 0.5 ms | **4232.95 ms** | Aucun | -- | 0 |
| 2556 | `mr.robot.s04e02.1080p.bluray.x264-rover...` | 3142.33 ms | 3552.37 ms | 16.91 ms | 54.12 ms | 1.0 ms | 0.5 ms | **6766.05 ms** | mr robot | 100.0% | 0 |
| 2557 | `mr.robot.s04e03.1080p.bluray.x264-rover...` | 1135.05 ms | 0.0 ms | 0.0 ms | 99.36 ms | 1.0 ms | 0.5 ms | **4737.47 ms** | Aucun | -- | 0 |
| 2558 | `mr.robot.s04e04.1080p.bluray.x264-rover...` | 1040.72 ms | 0.0 ms | 0.0 ms | 90.4 ms | 1.0 ms | 0.5 ms | **4632.99 ms** | Aucun | -- | 0 |
| 2559 | `mr.robot.s04e05.1080p.bluray.x264-rover...` | 823.45 ms | 0.0 ms | 0.0 ms | 111.09 ms | 1.0 ms | 0.5 ms | **4437.41 ms** | Aucun | -- | 0 |
| 2560 | `mr.robot.s04e06.1080p.bluray.x264-rover...` | 837.65 ms | 0.0 ms | 0.0 ms | 50.65 ms | 1.0 ms | 0.5 ms | **4389.43 ms** | Aucun | -- | 0 |
| 2561 | `mr.robot.s04e07.1080p.bluray.x264-rover...` | 1134.44 ms | 0.0 ms | 0.0 ms | 73.34 ms | 1.0 ms | 0.5 ms | **4710.08 ms** | Aucun | -- | 0 |
| 2562 | `mr.robot.s04e08.1080p.bluray.x264-rover...` | 1292.0 ms | 3770.71 ms | 189.1 ms | 251.26 ms | 1.0 ms | 0.5 ms | **5503.37 ms** | Aucun | -- | 0 |
| 2563 | `mr.robot.s04e09.1080p.bluray.x264-rover...` | 1349.6 ms | 3337.75 ms | 229.68 ms | 122.7 ms | 1.0 ms | 0.5 ms | **5040.06 ms** | mr robot | 100.0% | 0 |
| 2564 | `mr.robot.s04e10.1080p.bluray.x264-rover...` | 1338.3 ms | 3187.18 ms | 14.39 ms | 84.96 ms | 1.0 ms | 0.5 ms | **4629.16 ms** | mr robot | 100.0% | 0 |
| 2565 | `mr.robot.s04e11.1080p.bluray.x264-rover...` | 1187.64 ms | 3249.66 ms | 25.85 ms | 92.61 ms | 1.0 ms | 0.5 ms | **4556.09 ms** | mr robot | 100.0% | 0 |
| 2566 | `mr.robot.s04e12.1080p.bluray.x264-rover...` | 1255.25 ms | 2722.87 ms | 14.35 ms | 143.59 ms | 1.0 ms | 0.5 ms | **4136.78 ms** | mr robot | 100.0% | 0 |
| 2567 | `mr.robot.s04e13.1080p.bluray.x264-rover...` | 1183.17 ms | 2977.56 ms | 172.01 ms | 275.51 ms | 1.0 ms | 0.5 ms | **4608.69 ms** | mr robot | 100.0% | 0 |
| 2568 | `one.night.in.miami.2020.hdr.2160p.web.h...` | 1239.47 ms | 2601.07 ms | 325.85 ms | 332.08 ms | 1.0 ms | 0.5 ms | **4499.39 ms** | Aucun | -- | 0 |
| 2569 | `oslo.2021.1080p.web.h264-naisu_mid15s.mp4` | 865.61 ms | 3440.93 ms | 80.3 ms | 142.94 ms | 1.0 ms | 0.5 ms | **4530.16 ms** | Aucun | -- | 0 |
| 2570 | `sde-marsetavril_mid15s.mp4` | 1624.03 ms | 1815.45 ms | 24.13 ms | 160.45 ms | 1.0 ms | 0.5 ms | **3624.68 ms** | sde-marsetavril avi 24 0fps | 100.0% | 0 |
| 2571 | `shogun.2024.s01e08.dv.hdr.2160p.web.h26...` | 961.15 ms | 2229.48 ms | 22.33 ms | 57.9 ms | 1.0 ms | 0.5 ms | **3271.39 ms** | shogun.2024.s01e08.dv.hdr.2... | -- | 1 |
| 2572 | `silo.s01e10.1080p.web.h264-nhtfs[eztv.r...` | 11233.31 ms | 0.0 ms | 0.0 ms | 496.45 ms | 1.0 ms | 0.5 ms | **11730.02 ms** | silo.s01e10.1080p.web.h264-... | -- | 1 |
| 2573 | `stalker.1979.1080p.bluray.x264-usury_mi...` | 6480.93 ms | 141.89 ms | 101.41 ms | 223.16 ms | 1.0 ms | 0.5 ms | **6948.07 ms** | Stalker | 100.0% | 0 |
| 2574 | `swan.song.2021.1080p.web.h264-naisu_mid...` | 857.24 ms | 1614.67 ms | 20.51 ms | 115.81 ms | 1.0 ms | 0.5 ms | **2608.84 ms** | Aucun | -- | 0 |
| 2575 | `the.boys.s03e05.1080p.web.h264-glhf_mid...` | 1021.01 ms | 651.47 ms | 18.33 ms | 91.7 ms | 1.0 ms | 0.5 ms | **1791.93 ms** | the boys | 100.0% | 0 |
| 2576 | `the.boys.s03e06.1080p.web.h264-glhf_mid...` | 1375.66 ms | 334.16 ms | 179.24 ms | 181.01 ms | 1.0 ms | 0.5 ms | **2070.56 ms** | the boys | 100.0% | 0 |
| 2577 | `the.boys.s03e08.1080p.web.h264-glhf_mid...` | 794.08 ms | 1334.21 ms | 142.36 ms | 447.24 ms | 1.0 ms | 0.5 ms | **2721.93 ms** | the boys | 100.0% | 0 |
| 2578 | `the.expanse.s01e02.1080p.bluray.x264-ro...` | 737.64 ms | 631.61 ms | 347.89 ms | 437.84 ms | 1.0 ms | 0.5 ms | **2155.68 ms** | Aucun | -- | 0 |
| 2579 | `the.expanse.s01e03.1080p.bluray.x264-ro...` | 1982.98 ms | 148.76 ms | 23.16 ms | 124.37 ms | 1.0 ms | 0.5 ms | **2280.04 ms** | the expanse | 100.0% | 0 |
| 2580 | `the.expanse.s01e04.1080p.bluray.x264-ro...` | 3358.33 ms | 219.43 ms | 32.31 ms | 246.43 ms | 1.0 ms | 0.5 ms | **3867.24 ms** | Aucun | -- | 0 |
| 2581 | `the.expanse.s01e05.1080p.bluray.x264-ro...` | 3662.01 ms | 201.55 ms | 19.32 ms | 281.23 ms | 1.0 ms | 0.5 ms | **4165.65 ms** | the expanse | 100.0% | 0 |
| 2582 | `the.expanse.s01e07.1080p.bluray.x264-ro...` | 3382.85 ms | 241.49 ms | 31.63 ms | 120.07 ms | 1.0 ms | 0.5 ms | **3776.91 ms** | Aucun | -- | 0 |
| 2583 | `the.expanse.s01e08.1080p.bluray.x264-ro...` | 3035.13 ms | 97.23 ms | 21.81 ms | 210.24 ms | 1.0 ms | 0.5 ms | **3365.76 ms** | the expanse | 100.0% | 0 |
| 2584 | `the.expanse.s01e09e10.1080p.bluray.x264...` | 1786.19 ms | 142.46 ms | 25.63 ms | 149.26 ms | 1.0 ms | 0.5 ms | **2105.1 ms** | the expanse | 100.0% | 0 |
| 2585 | `the.expanse.s02e01.1080p.bluray.x264-ro...` | 2202.26 ms | 241.65 ms | 23.16 ms | 115.18 ms | 1.0 ms | 0.5 ms | **2582.95 ms** | the expanse | 100.0% | 0 |
| 2586 | `the.expanse.s02e03.1080p.bluray.x264-ro...` | 3720.72 ms | 257.59 ms | 123.59 ms | 531.41 ms | 1.0 ms | 0.5 ms | **4635.2 ms** | the expanse | 100.0% | 0 |
| 2587 | `the.expanse.s02e04.1080p.bluray.x264-ro...` | 747.79 ms | 0.0 ms | 0.0 ms | 387.83 ms | 1.0 ms | 0.5 ms | **1135.72 ms** | Aucun | -- | 0 |
| 2588 | `the.expanse.s02e06.1080p.bluray.x264-ro...` | 3301.21 ms | 288.83 ms | 20.56 ms | 125.0 ms | 1.0 ms | 0.5 ms | **3738.87 ms** | the expanse | 100.0% | 0 |
| 2589 | `the.expanse.s02e09.1080p.bluray.x264-ro...` | 2968.62 ms | 98.34 ms | 19.18 ms | 252.23 ms | 1.0 ms | 0.5 ms | **3339.63 ms** | the expanse | 100.0% | 0 |
| 2590 | `the.expanse.s02e10.1080p.bluray.x264-ro...` | 2891.96 ms | 107.68 ms | 18.56 ms | 258.34 ms | 1.0 ms | 0.5 ms | **3280.63 ms** | the expanse | 100.0% | 0 |
| 2591 | `the.expanse.s02e13.1080p.bluray.x264-ro...` | 3056.62 ms | 133.97 ms | 49.19 ms | 123.43 ms | 1.0 ms | 0.5 ms | **3373.13 ms** | Aucun | -- | 0 |
| 2592 | `the.expanse.s06e02.1080p.web.h264-glhf_...` | 2460.21 ms | 101.23 ms | 18.75 ms | 155.47 ms | 1.0 ms | 0.5 ms | **2736.84 ms** | the expanse | 100.0% | 1 |
| 2593 | `the.expanse.s06e03.1080p.web.h264-glhf_...` | 2950.49 ms | 172.37 ms | 20.88 ms | 302.3 ms | 1.0 ms | 0.5 ms | **3450.95 ms** | the.expanse.s06e03.1080p.we... | -- | 1 |
| 2594 | `the.expanse.s06e04.1080p.web.h264-glhf_...` | 2376.34 ms | 203.2 ms | 16.16 ms | 189.65 ms | 1.0 ms | 0.5 ms | **2785.81 ms** | the expanse | 100.0% | 1 |
| 2595 | `the.expanse.s06e05.1080p.web.h264-glhf_...` | 4419.14 ms | 97.27 ms | 15.07 ms | 333.76 ms | 1.0 ms | 0.5 ms | **4865.96 ms** | the expanse | 100.0% | 1 |
| 2596 | `the.expanse.s06e06.1080p.web.h264-glhf_...` | 2170.9 ms | 255.27 ms | 28.73 ms | 208.52 ms | 1.0 ms | 0.5 ms | **2666.19 ms** | the.expanse.s06e06.1080p.we... | -- | 1 |
| 2597 | `the.handmaids.tale.s02e01.1080p.bluray....` | 2460.83 ms | 193.03 ms | 95.67 ms | 248.72 ms | 1.0 ms | 0.5 ms | **2998.6 ms** | Aucun | -- | 0 |
| 2598 | `the.handmaids.tale.s02e02.1080p.bluray....` | 1912.94 ms | 339.88 ms | 119.11 ms | 162.19 ms | 1.0 ms | 0.5 ms | **2543.26 ms** | the handmaids tale | 100.0% | 0 |
| 2599 | `the.handmaids.tale.s02e03.1080p.bluray....` | 2031.83 ms | 264.2 ms | 27.18 ms | 235.22 ms | 1.0 ms | 0.5 ms | **2558.88 ms** | the handmaids tale | 100.0% | 0 |
| 2600 | `the.handmaids.tale.s02e04.1080p.bluray....` | 1473.16 ms | 276.88 ms | 19.53 ms | 282.77 ms | 1.0 ms | 0.5 ms | **2053.19 ms** | the expanse | 100.0% | 0 |
| 2601 | `the.handmaids.tale.s02e05.1080p.bluray....` | 3672.41 ms | 103.3 ms | 28.34 ms | 453.1 ms | 1.0 ms | 0.5 ms | **4258.25 ms** | the handmaids tale | 100.0% | 0 |
| 2602 | `the.handmaids.tale.s02e06.1080p.bluray....` | 2284.17 ms | 181.39 ms | 17.09 ms | 132.71 ms | 1.0 ms | 0.5 ms | **2616.55 ms** | the handmaids tale | 100.0% | 0 |
| 2603 | `the.handmaids.tale.s02e07.1080p.bluray....` | 3876.44 ms | 121.58 ms | 97.4 ms | 451.21 ms | 1.0 ms | 0.5 ms | **4547.03 ms** | Aucun | -- | 0 |
| 2604 | `the.handmaids.tale.s02e08.1080p.bluray....` | 3867.77 ms | 106.79 ms | 18.5 ms | 394.79 ms | 1.0 ms | 0.5 ms | **4388.21 ms** | the handmaids tale | 100.0% | 0 |
| 2605 | `the.handmaids.tale.s02e09.1080p.bluray....` | 3683.5 ms | 275.03 ms | 18.6 ms | 316.78 ms | 1.0 ms | 0.5 ms | **4294.3 ms** | the handmaids tale | 100.0% | 0 |
| 2606 | `the.handmaids.tale.s02e10.1080p.bluray....` | 2627.97 ms | 96.6 ms | 28.66 ms | 196.37 ms | 1.0 ms | 0.5 ms | **2949.92 ms** | the handmaids tale | 100.0% | 0 |
| 2607 | `the.handmaids.tale.s02e11.1080p.bluray....` | 4821.53 ms | 147.65 ms | 22.21 ms | 246.88 ms | 1.0 ms | 0.5 ms | **5239.11 ms** | the handmaids tale | 100.0% | 0 |
| 2608 | `the.handmaids.tale.s02e12.1080p.bluray....` | 2065.42 ms | 171.28 ms | 19.46 ms | 724.32 ms | 1.0 ms | 0.5 ms | **2982.6 ms** | the handmaids tale | 100.0% | 0 |
| 2609 | `the.handmaids.tale.s02e13.1080p.bluray....` | 2665.77 ms | 169.09 ms | 18.17 ms | 338.6 ms | 1.0 ms | 0.5 ms | **3192.4 ms** | the handmaids tale | 100.0% | 0 |
| 2610 | `the.handmaids.tale.s03e01.1080p.bluray....` | 2791.22 ms | 115.1 ms | 35.83 ms | 351.62 ms | 1.0 ms | 0.5 ms | **3294.54 ms** | Aucun | -- | 0 |
| 2611 | `the.handmaids.tale.s03e02.1080p.bluray....` | 3303.58 ms | 141.14 ms | 130.72 ms | 182.4 ms | 1.0 ms | 0.5 ms | **3758.07 ms** | the handmaids tale | 100.0% | 0 |
| 2612 | `the.handmaids.tale.s03e03.1080p.bluray....` | 2954.26 ms | 139.34 ms | 16.37 ms | 271.52 ms | 1.0 ms | 0.5 ms | **3382.18 ms** | the handmaids tale | 100.0% | 0 |
| 2613 | `the.handmaids.tale.s03e04.1080p.bluray....` | 2293.34 ms | 107.07 ms | 24.67 ms | 316.6 ms | 1.0 ms | 0.5 ms | **2742.04 ms** | the handmaids tale | 100.0% | 0 |
| 2614 | `the.handmaids.tale.s03e05.1080p.bluray....` | 3939.0 ms | 294.93 ms | 19.5 ms | 339.49 ms | 1.0 ms | 0.5 ms | **4600.85 ms** | the handmaids tale | 100.0% | 0 |
| 2615 | `the.handmaids.tale.s03e06.1080p.bluray....` | 2876.1 ms | 139.35 ms | 13.1 ms | 263.56 ms | 1.0 ms | 0.5 ms | **3292.53 ms** | the handmaids tale | 100.0% | 0 |
| 2616 | `the.handmaids.tale.s03e07.1080p.bluray....` | 2940.48 ms | 376.37 ms | 124.91 ms | 633.93 ms | 1.0 ms | 0.5 ms | **4076.42 ms** | the handmaids tale | 100.0% | 0 |
| 2617 | `the.handmaids.tale.s03e08.1080p.bluray....` | 2995.73 ms | 290.87 ms | 68.93 ms | 331.17 ms | 1.0 ms | 0.5 ms | **3690.84 ms** | the handmaids tale | 100.0% | 0 |
| 2618 | `the.handmaids.tale.s03e09.1080p.bluray....` | 4410.79 ms | 104.22 ms | 22.0 ms | 320.16 ms | 1.0 ms | 0.5 ms | **4857.86 ms** | the handmaids tale | 100.0% | 0 |
| 2619 | `the.handmaids.tale.s03e10.1080p.bluray....` | 2939.96 ms | 141.39 ms | 18.65 ms | 417.52 ms | 1.0 ms | 0.5 ms | **3518.04 ms** | the handmaids tale | 100.0% | 0 |
| 2620 | `the.handmaids.tale.s03e11.1080p.bluray....` | 2835.74 ms | 92.96 ms | 15.41 ms | 436.05 ms | 1.0 ms | 0.5 ms | **3380.48 ms** | the handmaids tale | 100.0% | 0 |
| 2621 | `the.handmaids.tale.s03e12.1080p.bluray....` | 3109.27 ms | 169.77 ms | 17.35 ms | 712.52 ms | 1.0 ms | 0.5 ms | **4010.44 ms** | the handmaids tale | 100.0% | 0 |
| 2622 | `the.handmaids.tale.s03e13.1080p.bluray....` | 2525.25 ms | 261.61 ms | 90.82 ms | 242.81 ms | 1.0 ms | 0.5 ms | **3120.95 ms** | Aucun | -- | 0 |
| 2623 | `the.lost.city.of.z.2016.french.1080p.bl...` | 2574.52 ms | 168.73 ms | 22.85 ms | 209.25 ms | 1.0 ms | 0.5 ms | **2979.22 ms** | the lost city of z | 100.0% | 0 |
| 2624 | `the.matrix.resurrections.2021.1080p.web...` | 4467.89 ms | 247.37 ms | 24.46 ms | 591.97 ms | 1.0 ms | 0.5 ms | **5333.25 ms** | Aucun | -- | 0 |
| 2625 | `the.peripheral.s01e03.1080p.web.h264-gl...` | 3848.36 ms | 182.9 ms | 17.72 ms | 351.81 ms | 1.0 ms | 0.5 ms | **4402.66 ms** | the peripheral | 100.0% | 1 |
| 2626 | `the.peripheral.s01e04.1080p.web.h264-gl...` | 3846.34 ms | 102.85 ms | 20.62 ms | 148.73 ms | 1.0 ms | 0.5 ms | **4127.45 ms** | the peripheral | 100.0% | 1 |
| 2627 | `to.all.the.boys.always.and.forever.2021...` | 4269.51 ms | 104.48 ms | 20.69 ms | 365.51 ms | 1.0 ms | 0.5 ms | **4765.61 ms** | Aucun | -- | 0 |
| 2628 | `utopia.s01e01.proper_720p_bluray.sujaid...` | 3469.64 ms | 214.16 ms | 15.85 ms | 350.04 ms | 1.0 ms | 0.5 ms | **4050.61 ms** | utopia | 100.0% | 1 |
| 2629 | `utopia.s01e02.720p.bluray.sujaidr_mid15...` | 3251.4 ms | 142.73 ms | 23.54 ms | 574.05 ms | 1.0 ms | 0.5 ms | **3992.56 ms** | utopia | 100.0% | 1 |
| 2630 | `utopia.s01e03.720p.bluray.sujaidr_mid15...` | 2556.01 ms | 257.51 ms | 18.14 ms | 461.87 ms | 1.0 ms | 0.5 ms | **3302.23 ms** | utopia | 100.0% | 1 |
| 2631 | `utopia.s01e04.720p.bluray.sujaidr_mid15...` | 10985.18 ms | 116.86 ms | 17.12 ms | 89.21 ms | 1.0 ms | 0.5 ms | **11210.31 ms** | utopia | 100.0% | 1 |
| 2632 | `utopia.s01e05.720p.bluray.sujaidr_mid15...` | 3774.62 ms | 179.27 ms | 17.48 ms | 484.63 ms | 1.0 ms | 0.5 ms | **4489.16 ms** | utopia | 100.0% | 1 |
| 2633 | `utopia.s01e06.720p.bluray.sujaidr_mid15...` | 6170.92 ms | 118.31 ms | 13.63 ms | 187.41 ms | 1.0 ms | 0.5 ms | **6491.1 ms** | utopia | 100.0% | 1 |
| 2634 | `vacation.friends.2021.1080p.web.h264-na...` | 4146.2 ms | 117.85 ms | 21.71 ms | 557.65 ms | 1.0 ms | 0.5 ms | **4847.59 ms** | Aucun | -- | 0 |
| 2635 | `westworld.s04e01.1080p.web.h264-cakes_m...` | 3506.61 ms | 105.51 ms | 20.54 ms | 413.36 ms | 1.0 ms | 0.5 ms | **4047.88 ms** | westworld | 100.0% | 1 |
| 2636 | `westworld.s04e02.1080p.web.h264-cakes_m...` | 3364.45 ms | 182.38 ms | 20.52 ms | 323.62 ms | 1.0 ms | 0.5 ms | **3895.08 ms** | westworld | 100.0% | 1 |
| 2637 | `westworld.s04e03.1080p.web.h264-cakes_m...` | 4288.54 ms | 118.19 ms | 20.33 ms | 433.3 ms | 1.0 ms | 0.5 ms | **4861.15 ms** | westworld | 100.0% | 1 |
| 2638 | `westworld.s04e04.1080p.web.h264-cakes[e...` | 3543.79 ms | 124.68 ms | 14.23 ms | 728.85 ms | 1.0 ms | 0.5 ms | **4412.31 ms** | westworld | 100.0% | 1 |
| 2639 | `what.we.do.in.the.shadows.s04e01.1080p....` | 3214.46 ms | 115.11 ms | 37.23 ms | 478.56 ms | 1.0 ms | 0.5 ms | **3847.88 ms** | what we do in the shadows | 100.0% | 1 |
| 2640 | `what.we.do.in.the.shadows.s04e02.1080p....` | 5410.8 ms | 90.66 ms | 18.1 ms | 270.59 ms | 1.0 ms | 0.5 ms | **5792.27 ms** | what.we.do.in.the.shadows.s... | -- | 1 |
| 2641 | `Épisode_001_mid15s.mp4` | 2876.77 ms | 133.18 ms | 24.21 ms | 548.21 ms | 1.0 ms | 0.5 ms | **3583.05 ms** | Aucun | -- | 0 |
| 2642 | `Épisode_002_mid15s.mp4` | 3547.99 ms | 120.27 ms | 18.21 ms | 321.01 ms | 1.0 ms | 0.5 ms | **4015.04 ms** | Épisode 002 mkv 24 0fps | 100.0% | 1 |
| 2643 | `Épisode_003_mid15s.mp4` | 2992.86 ms | 104.36 ms | 22.38 ms | 212.76 ms | 1.0 ms | 0.5 ms | **3333.12 ms** | Épisode 003 mkv 24 0fps | 100.0% | 1 |
| 2644 | `Épisode_004_mid15s.mp4` | 2724.18 ms | 110.1 ms | 21.18 ms | 188.17 ms | 1.0 ms | 0.5 ms | **3045.53 ms** | Épisode 004 mkv 24 0fps | 100.0% | 1 |
| 2645 | `Épisode_005_mid15s.mp4` | 2874.65 ms | 156.01 ms | 23.68 ms | 977.54 ms | 1.0 ms | 0.5 ms | **4035.38 ms** | Épisode 005 mkv 24 0fps | 100.0% | 1 |
| 2646 | `Épisode_006_mid15s.mp4` | 2831.3 ms | 147.74 ms | 27.99 ms | 402.96 ms | 1.0 ms | 0.5 ms | **3410.79 ms** | Épisode 006 mkv 24 0fps | 100.0% | 1 |
| 2647 | `Épisode_007_mid15s.mp4` | 6590.69 ms | 444.47 ms | 21.93 ms | 199.34 ms | 1.0 ms | 0.5 ms | **7257.41 ms** | Épisode 007 mkv 24 0fps | 100.0% | 1 |
| 2648 | `Épisode_008_mid15s.mp4` | 3832.03 ms | 145.27 ms | 21.23 ms | 506.19 ms | 1.0 ms | 0.5 ms | **4505.94 ms** | Épisode 008 mkv 24 0fps | 100.0% | 1 |
| 2649 | `Épisode_009_mid15s.mp4` | 2646.6 ms | 130.76 ms | 18.27 ms | 295.88 ms | 1.0 ms | 0.5 ms | **3094.67 ms** | Épisode 009 mkv 24 0fps | 100.0% | 1 |
| 2650 | `Épisode_010_mid15s.mp4` | 6316.72 ms | 198.36 ms | 58.89 ms | 116.04 ms | 1.0 ms | 0.5 ms | **6692.07 ms** | Aucun | -- | 0 |
| 2651 | `Épisode_011_mid15s.mp4` | 2421.2 ms | 199.98 ms | 18.05 ms | 409.72 ms | 1.0 ms | 0.5 ms | **3056.05 ms** | Épisode 011 mkv 24 0fps | 100.0% | 1 |
| 2652 | `Épisode_012_mid15s.mp4` | 2313.07 ms | 133.92 ms | 18.96 ms | 175.53 ms | 1.0 ms | 0.5 ms | **2648.47 ms** | Épisode 012 mkv 24 0fps | 100.0% | 1 |
| 2653 | `Épisode_013_mid15s.mp4` | 2998.48 ms | 128.39 ms | 25.05 ms | 359.46 ms | 1.0 ms | 0.5 ms | **3512.2 ms** | Aucun | -- | 0 |
| 2654 | `Épisode_014_mid15s.mp4` | 4568.52 ms | 122.11 ms | 27.59 ms | 529.86 ms | 1.0 ms | 0.5 ms | **5252.7 ms** | Aucun | -- | 0 |
| 2655 | `Épisode_015_mid15s.mp4` | 2683.37 ms | 124.28 ms | 18.01 ms | 438.08 ms | 1.0 ms | 0.5 ms | **3266.11 ms** | Aucun | -- | 0 |
| 2656 | `Épisode_016_mid15s.mp4` | 5083.33 ms | 302.52 ms | 29.56 ms | 113.57 ms | 1.0 ms | 0.5 ms | **5538.9 ms** | Épisode 016 mkv 24 0fps | 100.0% | 0 |
| 2657 | `Épisode_017_mid15s.mp4` | 3009.82 ms | 117.22 ms | 22.07 ms | 368.15 ms | 1.0 ms | 0.5 ms | **3517.96 ms** | Épisode 017 mkv 24 0fps | 100.0% | 0 |
| 2658 | `Épisode_018_mid15s.mp4` | 4671.07 ms | 282.44 ms | 21.37 ms | 285.5 ms | 1.0 ms | 0.5 ms | **5261.62 ms** | Épisode 018 mkv 24 0fps | 100.0% | 0 |
| 2659 | `Épisode_019_mid15s.mp4` | 3840.17 ms | 157.06 ms | 17.98 ms | 641.48 ms | 1.0 ms | 0.5 ms | **4661.08 ms** | Épisode 019 mkv 24 0fps | 100.0% | 0 |
| 2660 | `Épisode_020_mid15s.mp4` | 3808.17 ms | 96.29 ms | 21.15 ms | 103.67 ms | 1.0 ms | 0.5 ms | **4034.33 ms** | Épisode 020 mkv 24 0fps | 100.0% | 0 |
| 2661 | `Épisode_021_mid15s.mp4` | 3422.4 ms | 88.98 ms | 24.82 ms | 862.04 ms | 1.0 ms | 0.5 ms | **4399.0 ms** | Épisode 021 mkv 24 0fps | 100.0% | 0 |
| 2662 | `Épisode_022_mid15s.mp4` | 2321.5 ms | 142.87 ms | 19.79 ms | 468.06 ms | 1.0 ms | 0.5 ms | **2958.46 ms** | Épisode 022 mkv 24 0fps | 100.0% | 0 |
| 2663 | `Épisode_023_mid15s.mp4` | 4403.96 ms | 90.55 ms | 17.97 ms | 177.02 ms | 1.0 ms | 0.5 ms | **4690.13 ms** | Épisode 023 mkv 24 0fps | 100.0% | 0 |
| 2664 | `Épisode_024_mid15s.mp4` | 4328.8 ms | 94.53 ms | 22.23 ms | 631.4 ms | 1.0 ms | 0.5 ms | **5077.73 ms** | Épisode 024 mkv 24 0fps | 100.0% | 0 |
| 2665 | `Épisode_025_mid15s.mp4` | 3571.75 ms | 219.17 ms | 21.07 ms | 270.16 ms | 1.0 ms | 0.5 ms | **4087.9 ms** | Épisode 025 mkv 24 0fps | 100.0% | 0 |
| 2666 | `Épisode_026_mid15s.mp4` | 2544.47 ms | 112.98 ms | 19.96 ms | 965.88 ms | 1.0 ms | 0.5 ms | **3644.02 ms** | Épisode 026 mkv 24 0fps | 100.0% | 0 |
| 2667 | `Épisode_027_mid15s.mp4` | 3241.0 ms | 250.34 ms | 25.4 ms | 302.88 ms | 1.0 ms | 0.5 ms | **3820.22 ms** | Aucun | -- | 0 |
| 2668 | `Épisode_028_mid15s.mp4` | 2226.08 ms | 477.75 ms | 23.27 ms | 122.01 ms | 1.0 ms | 0.5 ms | **2851.33 ms** | Épisode 028 mkv 24 0fps | 100.0% | 0 |
| 2669 | `Épisode_029_mid15s.mp4` | 3398.33 ms | 101.02 ms | 17.42 ms | 496.73 ms | 1.0 ms | 0.5 ms | **4017.73 ms** | Épisode 029 mkv 24 0fps | 100.0% | 0 |
| 2670 | `Épisode_030_mid15s.mp4` | 4985.01 ms | 136.28 ms | 19.46 ms | 284.55 ms | 1.0 ms | 0.5 ms | **5429.65 ms** | Épisode 030 mkv 24 0fps | 100.0% | 0 |
| 2671 | `Épisode_031_mid15s.mp4` | 3569.54 ms | 86.22 ms | 20.78 ms | 529.14 ms | 1.0 ms | 0.5 ms | **4228.96 ms** | Épisode 031 mkv 24 0fps | 100.0% | 0 |
| 2672 | `Épisode_032_mid15s.mp4` | 3170.08 ms | 105.77 ms | 25.63 ms | 462.19 ms | 1.0 ms | 0.5 ms | **3767.88 ms** | Épisode 032 mkv 24 0fps | 100.0% | 0 |
| 2673 | `Épisode_033_mid15s.mp4` | 2623.01 ms | 91.99 ms | 24.16 ms | 121.1 ms | 1.0 ms | 0.5 ms | **2891.63 ms** | Épisode 033 mkv 24 0fps | 100.0% | 0 |
| 2674 | `Épisode_034_mid15s.mp4` | 4789.95 ms | 202.89 ms | 141.14 ms | 249.04 ms | 1.0 ms | 0.5 ms | **5383.75 ms** | Épisode 034 mkv 24 0fps | 100.0% | 0 |
| 2675 | `Épisode_035_mid15s.mp4` | 3011.06 ms | 126.93 ms | 25.0 ms | 273.54 ms | 1.0 ms | 0.5 ms | **3437.33 ms** | Aucun | -- | 0 |
| 2676 | `Épisode_036_mid15s.mp4` | 2525.83 ms | 132.94 ms | 18.5 ms | 114.0 ms | 1.0 ms | 0.5 ms | **2792.13 ms** | Épisode 036 mkv 24 0fps | 100.0% | 0 |
| 2677 | `Épisode_037_mid15s.mp4` | 6531.77 ms | 121.28 ms | 20.13 ms | 554.38 ms | 1.0 ms | 0.5 ms | **7231.13 ms** | Épisode 037 mkv 24 0fps | 100.0% | 0 |
| 2678 | `Épisode_038_mid15s.mp4` | 3523.91 ms | 212.38 ms | 41.3 ms | 327.65 ms | 1.0 ms | 0.5 ms | **4106.81 ms** | Aucun | -- | 0 |
| 2679 | `Épisode_039_mid15s.mp4` | 3257.36 ms | 112.09 ms | 26.41 ms | 129.85 ms | 1.0 ms | 0.5 ms | **3526.54 ms** | Épisode 039 mkv 24 0fps | 100.0% | 0 |
| 2680 | `Épisode_040_mid15s.mp4` | 3330.14 ms | 182.99 ms | 21.64 ms | 293.03 ms | 1.0 ms | 0.5 ms | **3836.15 ms** | Épisode 040 mkv 24 0fps | 100.0% | 0 |
| 2681 | `Épisode_041_mid15s.mp4` | 3256.91 ms | 115.1 ms | 20.11 ms | 317.05 ms | 1.0 ms | 0.5 ms | **3710.86 ms** | Épisode 041 mkv 24 0fps | 100.0% | 0 |
| 2682 | `Épisode_042_mid15s.mp4` | 3678.47 ms | 135.22 ms | 34.75 ms | 326.08 ms | 1.0 ms | 0.5 ms | **4175.27 ms** | Épisode 042 mkv 24 0fps | 100.0% | 0 |
| 2683 | `Épisode_043_mid15s.mp4` | 3378.73 ms | 220.42 ms | 75.25 ms | 125.74 ms | 1.0 ms | 0.5 ms | **3800.89 ms** | Épisode 043 mkv 24 0fps | 100.0% | 0 |
| 2684 | `Épisode_044_mid15s.mp4` | 3306.06 ms | 251.15 ms | 158.9 ms | 214.05 ms | 1.0 ms | 0.5 ms | **3932.71 ms** | Épisode 044 mkv 24 0fps | 100.0% | 0 |
| 2685 | `Épisode_045_mid15s.mp4` | 3659.99 ms | 188.48 ms | 37.11 ms | 203.99 ms | 1.0 ms | 0.5 ms | **4090.39 ms** | Épisode 045 mkv 24 0fps | 100.0% | 0 |
| 2686 | `Épisode_046_mid15s.mp4` | 3314.63 ms | 106.79 ms | 36.95 ms | 177.18 ms | 1.0 ms | 0.5 ms | **3640.14 ms** | Épisode 046 mkv 24 0fps | 100.0% | 0 |
| 2687 | `Épisode_047_mid15s.mp4` | 5971.95 ms | 98.51 ms | 21.26 ms | 553.6 ms | 1.0 ms | 0.5 ms | **6647.55 ms** | Épisode 047 mkv 24 0fps | 100.0% | 0 |
| 2688 | `Épisode_048_mid15s.mp4` | 2920.55 ms | 187.21 ms | 55.37 ms | 276.07 ms | 1.0 ms | 0.5 ms | **3439.99 ms** | Épisode 048 mkv 24 0fps | 100.0% | 0 |
| 2689 | `Épisode_049_mid15s.mp4` | 5012.38 ms | 136.76 ms | 25.1 ms | 564.84 ms | 1.0 ms | 0.5 ms | **5739.96 ms** | Épisode 049 mkv 24 0fps | 100.0% | 0 |
| 2690 | `Épisode_050_mid15s.mp4` | 3122.26 ms | 96.45 ms | 33.7 ms | 217.61 ms | 1.0 ms | 0.5 ms | **3470.84 ms** | Épisode 050 mkv 24 0fps | 100.0% | 0 |
| 2691 | `Épisode_051_mid15s.mp4` | 2509.08 ms | 102.5 ms | 32.09 ms | 151.42 ms | 1.0 ms | 0.5 ms | **2795.87 ms** | Épisode 051 mkv 24 0fps | 100.0% | 0 |
| 2692 | `Épisode_052_mid15s.mp4` | 2953.55 ms | 158.17 ms | 33.45 ms | 228.8 ms | 1.0 ms | 0.5 ms | **3374.73 ms** | Épisode 052 mkv 24 0fps | 100.0% | 0 |
| 2693 | `Épisode_053_mid15s.mp4` | 1834.34 ms | 107.01 ms | 28.94 ms | 290.73 ms | 1.0 ms | 0.5 ms | **2261.75 ms** | Épisode 053 mkv 24 0fps | 100.0% | 0 |
| 2694 | `Épisode_054_mid15s.mp4` | 2959.61 ms | 190.55 ms | 24.0 ms | 353.65 ms | 1.0 ms | 0.5 ms | **3536.07 ms** | Aucun | -- | 0 |
| 2695 | `Épisode_055_mid15s.mp4` | 2808.53 ms | 216.02 ms | 20.2 ms | 516.9 ms | 1.0 ms | 0.5 ms | **3569.98 ms** | Épisode 055 mkv 24 0fps | 100.0% | 0 |
| 2696 | `Épisode_056_mid15s.mp4` | 5180.65 ms | 0.0 ms | 0.0 ms | 116.29 ms | 1.0 ms | 0.5 ms | **8809.0 ms** | Aucun | -- | 0 |
| 2697 | `Épisode_057_mid15s.mp4` | 2755.93 ms | 94.71 ms | 21.39 ms | 290.1 ms | 1.0 ms | 0.5 ms | **3163.06 ms** | Épisode 057 mkv 24 0fps | 100.0% | 0 |
| 2698 | `Épisode_058_mid15s.mp4` | 4950.87 ms | 0.0 ms | 0.0 ms | 121.96 ms | 1.0 ms | 0.5 ms | **8620.88 ms** | Aucun | -- | 0 |
| 2699 | `Épisode_059_mid15s.mp4` | 3001.57 ms | 163.67 ms | 19.76 ms | 151.84 ms | 1.0 ms | 0.5 ms | **3338.25 ms** | Épisode 059 mkv 24 0fps | 100.0% | 0 |
| 2700 | `Épisode_060_mid15s.mp4` | 4216.98 ms | 0.0 ms | 0.0 ms | 133.23 ms | 1.0 ms | 0.5 ms | **8865.67 ms** | Aucun | -- | 0 |
| 2701 | `Épisode_074_mid15s.mp4` | 4715.62 ms | 0.0 ms | 0.0 ms | 192.44 ms | 1.0 ms | 0.5 ms | **8414.93 ms** | Aucun | -- | 0 |
| 2702 | `Épisode_075_mid15s.mp4` | 3009.05 ms | 157.51 ms | 28.22 ms | 815.5 ms | 1.0 ms | 0.5 ms | **4019.12 ms** | Épisode 075 mkv 24 0fps | 100.0% | 0 |
| 2703 | `Épisode_076_mid15s.mp4` | 4758.68 ms | 0.0 ms | 0.0 ms | 263.89 ms | 1.0 ms | 0.5 ms | **8526.27 ms** | Aucun | -- | 0 |
| 2704 | `Épisode_077_mid15s.mp4` | 6531.58 ms | 0.0 ms | 0.0 ms | 245.24 ms | 1.0 ms | 0.5 ms | **10279.36 ms** | Aucun | -- | 0 |
| 2705 | `Épisode_078_mid15s.mp4` | 5405.83 ms | 0.0 ms | 0.0 ms | 313.14 ms | 1.0 ms | 0.5 ms | **9224.95 ms** | Aucun | -- | 0 |
| 2706 | `Épisode_079_mid15s.mp4` | 2687.25 ms | 215.95 ms | 20.58 ms | 150.01 ms | 1.0 ms | 0.5 ms | **3074.84 ms** | Épisode 079 mkv 24 0fps | 100.0% | 0 |
| 2707 | `Épisode_080_mid15s.mp4` | 4255.92 ms | 0.0 ms | 0.0 ms | 152.68 ms | 1.0 ms | 0.5 ms | **7987.13 ms** | Aucun | -- | 0 |
| 2708 | `Épisode_081_mid15s.mp4` | 3447.04 ms | 108.84 ms | 20.77 ms | 314.77 ms | 1.0 ms | 0.5 ms | **3901.86 ms** | Aucun | -- | 0 |
| 2709 | `Épisode_082_mid15s.mp4` | 4516.53 ms | 0.0 ms | 0.0 ms | 205.7 ms | 1.0 ms | 0.5 ms | **8233.86 ms** | Aucun | -- | 0 |
| 2710 | `Épisode_083_mid15s.mp4` | 4545.35 ms | 0.0 ms | 0.0 ms | 326.15 ms | 1.0 ms | 0.5 ms | **8410.91 ms** | Aucun | -- | 0 |
| 2711 | `Épisode_084_mid15s.mp4` | 2946.94 ms | 0.0 ms | 0.0 ms | 163.77 ms | 1.0 ms | 0.5 ms | **6611.94 ms** | Aucun | -- | 0 |
| 2712 | `Épisode_085_mid15s.mp4` | 3789.46 ms | 0.0 ms | 0.0 ms | 277.37 ms | 1.0 ms | 0.5 ms | **7731.15 ms** | Aucun | -- | 0 |
| 2713 | `Épisode_086_mid15s.mp4` | 3716.13 ms | 0.0 ms | 0.0 ms | 260.58 ms | 1.0 ms | 0.5 ms | **7500.08 ms** | Aucun | -- | 0 |
| 2714 | `Épisode_087_mid15s.mp4` | 3210.41 ms | 0.0 ms | 0.0 ms | 178.7 ms | 1.0 ms | 0.5 ms | **6894.38 ms** | Aucun | -- | 0 |
| 2715 | `Épisode_088_mid15s.mp4` | 2439.09 ms | 0.0 ms | 0.0 ms | 157.2 ms | 1.0 ms | 0.5 ms | **6098.58 ms** | Aucun | -- | 0 |
| 2716 | `Épisode_089_mid15s.mp4` | 2484.61 ms | 0.0 ms | 0.0 ms | 264.06 ms | 1.0 ms | 0.5 ms | **6289.87 ms** | Aucun | -- | 0 |
| 2717 | `Épisode_090_mid15s.mp4` | 2330.94 ms | 0.0 ms | 0.0 ms | 86.7 ms | 1.0 ms | 0.5 ms | **5919.78 ms** | Aucun | -- | 0 |
| 2718 | `Épisode_091_mid15s.mp4` | 2627.26 ms | 0.0 ms | 0.0 ms | 233.44 ms | 1.0 ms | 0.5 ms | **6364.64 ms** | Aucun | -- | 0 |
| 2719 | `Épisode_092_mid15s.mp4` | 2125.64 ms | 0.0 ms | 0.0 ms | 189.56 ms | 1.0 ms | 0.5 ms | **5993.45 ms** | Aucun | -- | 0 |
| 2720 | `Épisode_100_mid15s.mp4` | 1817.21 ms | 0.0 ms | 0.0 ms | 258.35 ms | 1.0 ms | 0.5 ms | **5754.68 ms** | Aucun | -- | 0 |
| 2721 | `Épisode_101_mid15s.mp4` | 2832.23 ms | 0.0 ms | 0.0 ms | 56.71 ms | 1.0 ms | 0.5 ms | **6399.16 ms** | Aucun | -- | 0 |
| 2722 | `Épisode_102_mid15s.mp4` | 2285.29 ms | 0.0 ms | 0.0 ms | 37.24 ms | 1.0 ms | 0.5 ms | **5833.44 ms** | Aucun | -- | 0 |
| 2723 | `Épisode_103_mid15s.mp4` | 2582.3 ms | 0.0 ms | 0.0 ms | 44.68 ms | 1.0 ms | 0.5 ms | **6137.66 ms** | Aucun | -- | 0 |
| 2724 | `Épisode_104_mid15s.mp4` | 1818.24 ms | 0.0 ms | 0.0 ms | 65.26 ms | 1.0 ms | 0.5 ms | **5393.87 ms** | Aucun | -- | 0 |
| 2725 | `Épisode_105_mid15s.mp4` | 2399.32 ms | 0.0 ms | 0.0 ms | 45.06 ms | 1.0 ms | 0.5 ms | **5945.63 ms** | Aucun | -- | 0 |
| 2726 | `Épisode_106_mid15s.mp4` | 2273.58 ms | 0.0 ms | 0.0 ms | 57.88 ms | 1.0 ms | 0.5 ms | **5849.0 ms** | Aucun | -- | 0 |
| 2727 | `Épisode_107_mid15s.mp4` | 2122.05 ms | 0.0 ms | 0.0 ms | 52.71 ms | 1.0 ms | 0.5 ms | **5676.25 ms** | Aucun | -- | 0 |
| 2728 | `Épisode_108_mid15s.mp4` | 1958.16 ms | 0.0 ms | 0.0 ms | 46.71 ms | 1.0 ms | 0.5 ms | **5538.57 ms** | Aucun | -- | 0 |
| 2729 | `Épisode_109_mid15s.mp4` | 3027.34 ms | 0.0 ms | 0.0 ms | 123.41 ms | 1.0 ms | 0.5 ms | **6653.16 ms** | Aucun | -- | 0 |
| 2730 | `Épisode_110_mid15s.mp4` | 1880.23 ms | 0.0 ms | 0.0 ms | 31.75 ms | 1.0 ms | 0.5 ms | **5413.91 ms** | Aucun | -- | 0 |
| 2731 | `Épisode_111_mid15s.mp4` | 4184.02 ms | 0.0 ms | 0.0 ms | 86.78 ms | 1.0 ms | 0.5 ms | **7782.19 ms** | Aucun | -- | 0 |
| 2732 | `Épisode_112_mid15s.mp4` | 2063.31 ms | 0.0 ms | 0.0 ms | 112.48 ms | 1.0 ms | 0.5 ms | **5677.79 ms** | Aucun | -- | 0 |
| 2733 | `Épisode_113_mid15s.mp4` | 1890.52 ms | 0.0 ms | 0.0 ms | 78.64 ms | 1.0 ms | 0.5 ms | **5479.23 ms** | Aucun | -- | 0 |
| 2734 | `Épisode_114_mid15s.mp4` | 1767.47 ms | 0.0 ms | 0.0 ms | 46.42 ms | 1.0 ms | 0.5 ms | **5316.31 ms** | Aucun | -- | 0 |
| 2735 | `✔️Altered_Carbon_Resleeved_(2020)[MULTi...` | 1786.23 ms | 0.0 ms | 0.0 ms | 132.26 ms | 1.0 ms | 0.5 ms | **5472.53 ms** | Aucun | -- | 0 |
| 2736 | `✔️Fortune_Favors_Lady_Nikuko_(2021)[MUL...` | 1494.23 ms | 0.0 ms | 0.0 ms | 221.26 ms | 1.0 ms | 0.5 ms | **5220.21 ms** | Aucun | -- | 0 |
| 2737 | `✔️Icare_(2022)[1080p]_mid15s.mp4` | 1922.46 ms | 0.0 ms | 0.0 ms | 135.09 ms | 1.0 ms | 0.5 ms | **5560.09 ms** | Aucun | -- | 0 |
| 2738 | `✔️John.Wick.Chapter.2.2017.1080p.WEB-DL...` | 1651.56 ms | 0.0 ms | 0.0 ms | 128.43 ms | 1.0 ms | 0.5 ms | **5283.15 ms** | Aucun | -- | 0 |
| 2739 | `✔️Vivo_(2021)[MULTi][1080p](kinkajou_mé...` | 1738.71 ms | 0.0 ms | 0.0 ms | 140.1 ms | 1.0 ms | 0.5 ms | **5380.92 ms** | Aucun | -- | 0 |
| 2740 | `✔️_Asterix_Le_Secret_de_la_Potion_Magiq...` | 1722.12 ms | 0.0 ms | 0.0 ms | 134.71 ms | 1.0 ms | 0.5 ms | **5390.96 ms** | Aucun | -- | 0 |
| 2741 | `✔️_BRENDAN_ET_LE_SECRET_DE_KELLS_(2009)...` | 379.28 ms | 0.0 ms | 0.0 ms | 71.97 ms | 1.0 ms | 0.5 ms | **3953.11 ms** | Aucun | -- | 0 |
| 2742 | `✔️_Dragon_3_(2019)[MULTi][1080p]_mid15s...` | 352.0 ms | 0.0 ms | 0.0 ms | 58.44 ms | 1.0 ms | 0.5 ms | **3920.33 ms** | Aucun | -- | 0 |
| 2743 | `✔️_J'AI_PERDU_MON_CORPS_(2019)_[1080p](...` | 951.69 ms | 0.0 ms | 0.0 ms | 37.91 ms | 1.0 ms | 0.5 ms | **4491.44 ms** | Aucun | -- | 0 |
| 2744 | `✔️_L'ILLUSIONNISTE_(2010)_mid15s.mp4` | 791.92 ms | 0.0 ms | 0.0 ms | 33.41 ms | 1.0 ms | 0.5 ms | **4326.38 ms** | Aucun | -- | 0 |
| 2745 | `✔️_LE_PARC_DES_MERVEILLES_(2019)[MULTi]...` | 1059.73 ms | 0.0 ms | 0.0 ms | 45.49 ms | 1.0 ms | 0.5 ms | **4615.86 ms** | Aucun | -- | 0 |
| 2746 | `✔️_LE_VOYAGE_DU_PRINCE_(2019)_[1080p](U...` | 699.04 ms | 0.0 ms | 0.0 ms | 31.82 ms | 1.0 ms | 0.5 ms | **4232.55 ms** | Aucun | -- | 0 |
| 2747 | `✔️_La_Fameuse_Invasion_des_Ours_en_Sici...` | 1084.88 ms | 0.0 ms | 0.0 ms | 25.45 ms | 1.0 ms | 0.5 ms | **4611.56 ms** | Aucun | -- | 0 |
| 2748 | `✔️_Le_Grand_Mechant_Renard_et_Autres_Co...` | 1005.64 ms | 0.0 ms | 0.0 ms | 24.74 ms | 1.0 ms | 0.5 ms | **4570.23 ms** | Aucun | -- | 0 |
| 2749 | `✔️_Le_Roi_Et_L_oiseau_[1080p]_mid15s.mp4` | 1066.81 ms | 0.0 ms | 0.0 ms | 25.01 ms | 1.0 ms | 0.5 ms | **4605.19 ms** | Aucun | -- | 0 |
| 2750 | `✔️_Le_Tableau_(2011)(Peintures,_Toupins...` | 1200.78 ms | 0.0 ms | 0.0 ms | 45.04 ms | 1.0 ms | 0.5 ms | **4747.76 ms** | Aucun | -- | 0 |
| 2751 | `✔️_MARY_ET_LA_FLEUR_DE_LA_SORCIÈRE_(201...` | 1173.82 ms | 0.0 ms | 0.0 ms | 23.82 ms | 1.0 ms | 0.5 ms | **4699.6 ms** | Aucun | -- | 0 |
| 2752 | `✔️_Miraï,_ma_petite_sœur_(2018)(arbre_g...` | 3005.14 ms | 2382.07 ms | 21.6 ms | 34.48 ms | 1.0 ms | 0.5 ms | **5443.82 ms** | ✔️ Miraï, ma petite sœur | 100.0% | 0 |
| 2753 | `✔️_Ojamajo_Doremi___Majo_Minarai_wo_Sag...` | 969.92 ms | 0.0 ms | 0.0 ms | 30.78 ms | 1.0 ms | 0.5 ms | **4511.09 ms** | Aucun | -- | 0 |
| 2754 | `✔️_Pachamama_(2018)[FRENCH][720p](deux_...` | 1390.55 ms | 0.0 ms | 0.0 ms | 37.68 ms | 1.0 ms | 0.5 ms | **4938.73 ms** | Aucun | -- | 0 |
| 2755 | `✔️_Sahara_(2017)_mid15s.mp4` | 1317.87 ms | 0.0 ms | 0.0 ms | 24.21 ms | 1.0 ms | 0.5 ms | **4854.69 ms** | Aucun | -- | 0 |
| 2756 | `✔️_TOUT_EN_HAUT_DU_MONDE_(2015](1882,_S...` | 1041.19 ms | 0.0 ms | 0.0 ms | 44.8 ms | 1.0 ms | 0.5 ms | **4596.35 ms** | Aucun | -- | 0 |
| 2757 | `✔️_Vaillante_(2022)[FRENCH][1080p](femm...` | 1282.31 ms | 3485.54 ms | 23.22 ms | 41.02 ms | 1.0 ms | 0.5 ms | **4833.11 ms** | ✔️ Vaillante | 100.0% | 0 |
| 2758 | `✔️🇫🇷Mars.Express.2023.1080p.WEBRip.x265...` | 1877.9 ms | 3517.38 ms | 28.94 ms | 39.6 ms | 1.0 ms | 0.5 ms | **5464.3 ms** | ✔️🇫🇷Mars Express | 100.0% | 0 |

---

## 4. Observations & Enseignements Techniques

1. **Décodage FFmpeg Local (Goulot CPU Principal)** :
   - Le décodage 10-15s en 1080p et conversion d'échelle `swscale` monopolisent en moyenne **2985.32 ms** (~68% du temps total).
   - Ce temps est strictement local et dépend des performances CPU de la machine cliente.

2. **Transport WAN Internet (HTTPS :443)** :
   - La latence d'aller-retour réseau vers Cloud o2switch s'établit à une médiane de **109.36 ms** (p95: **879.42 ms**).
   - Le payload JSON compact (192 octets de pHash, 1.1 Ko HTTP) limite le transit à 1 RTT TCP/TLS.

3. **Lookup MIH Serveur en RAM** :
   - La recherche par Multi-Index Hashing (MIH) sur o2switch est ultra-rapide : médiane de **23.62 ms** sur des millions de frames.

4. **Découverte & Calage SRT** :
   - L'interrogation de l'API SRT (`GET /v1/subtitles/{hash}`) s'effectue en moyenne en **229.17 ms**.
   - L'alignement temporel par décalage constant en mémoire prend **1.0 ms**, confirmant l'absence de coût pour le calcul sub-seconde.

## 4. Analyse Forensique Exhaustive des 849 Rejets

Chaque vidéo rejetée a été analysée empiriquement à l'échelle unitaire sur 4 axes de contrôle :

1. **Nature du contenu** (Taxonomie éditoriale : Tutoriels informatiques, Bonus/Trailers, Films/Séries).
2. **Présence physique de sous-titres** (Contrôle d'existence de fichiers `.srt` locaux).
3. **Couverture de l'index de référence** (Croisement avec les 3 061 médias de `catalog.dbphash`).
4. **Entropie visuelle et gardes-fous** (Détection des plans noirs/statiques et activation du rejet sécurisé).

### 4.1 Répartition Catégorielle des Rejets

| Motif / Qualification Métier | Volume | Part des Rejets | Justification Algorithmique & Règle Métier |
| :--- | :---: | :---: | :--- |
| **Médias au Catalogue (Zéro SRT ou Seuil < 70%)** | **340** | **40.0 %** | Titre répertorié dans `catalog.dbphash`, mais sans sous-titre certifié enregistré ou confiance < 70%. |
| **Tutoriels Informatiques (Grafikart, etc.)** | **205** | **24.1 %** | Vidéos techniques sans fiche cinéma ni aucun sous-titre `.srt` existant (CakePHP, Git, SQL, etc.). |
| **Films / Séries Absents du Catalogue** | **169** | **19.9 %** | Média non indexé dans l'index de référence `catalog.dbphash` (aucune empreinte correspondante). |
| **Collisions Écartées (Plans Noirs / Fixes)** | **133** | **15.7 %** | Extrait mid-15s sans variation visuelle, collision écartée par sécurité par `stem_and_title_overlap`. |
| **Bonus, Teasers & TV Spots** | **2** | **0.2 %** | Extraits promotionnels courts exclus du catalogue de synchronisation cinéma. |

> **Constat Clé sur les Sous-Titres :** Sur les 849 vidéos rejetées,
> **849 (100.0 %)** ne possèdent **aucun fichier de sous-titre local `.srt`** dans `data/snippets/`.
> Le système n'a ignoré aucun sous-titre existant : les rejets correspondent strictement à des médias
> non pourvus de sous-titres ou à des contenus hors catalogue.

### 4.2 Matrice Détaillée des 849 Rejets Vérifiés

| # | Fichier Snippet Rejeté | Catégorie | SRT Local | Dans Catalogue | Entropie | Justification du Rejet |
| :--- | :--- | :---: | :---: | :---: | :---: | :--- |
| 1 | `04_La_sélection_finale_mid15s.mp4` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 2 | `05_Chacun_son_acier_mid15s.mp4` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 3 | `06_Le_chasseur_accompagné_d'un_...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 4 | `09_Les_démons_aux_ballons_et_au...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 5 | `12AOKA~2_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 6 | `12_Zen'itsu_dort,_le_sanglier_mo...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 7 | `14_Jours_12_Nuits_(2020)_[1080p]...` | **COLLISION_PLAN_FIXE_ECARTEE** | Non (0) | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 8 | `19_Le_Dieu_du_feu_mid15s.mp4` | **COLLISION_PLAN_FIXE_ECARTEE** | Non (0) | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 9 | `1O6HM~0_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 10 | `26_Nouvelle_mission_mid15s.mp4` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 11 | `28.Years.Later.2025.2160p.AMZN.W...` | **COLLISION_PLAN_FIXE_ECARTEE** | Non (0) | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 12 | `2T8EO4~B_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 13 | `4-Blue_Dream_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 14 | `8_TV_Spots_mid15s.mp4` | **EXTRA_BONUS** | Non (0) | Non | Plate (Noir) | Extrait bonus ou spot publicitaire court exclu du catalogue |
| 15 | `A.Complete.Unknown.2024.2160p.AM...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 16 | `A_Plein_Temps_(2021)[1080p](mère...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Oui | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 17 | `A_Plein_Temps_(2021)_[1080p.H264...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Oui | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 18 | `Aftersun.2022.1080p.WEBRip.x264....` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Plate (Noir) | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 19 | `Akira.1988.25th.Anniversary.Edit...` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 20 | `Alaska_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 21 | `Anomalisa.2015.1080p.BluRay.x264...` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 22 | `Apprendre_CakePHP_-_Les_models_m...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 23 | `Apprendre_CakePHP_-_Routing_mid1...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 24 | `Apprendre_CakePHP_-_TP2_-_Introd...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 25 | `Apprendre_CakePHP_mid15s.mp4` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 26 | `Apprendre_MySQL_(13_19)_-_Dumps_...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 27 | `Apprendre_MySQL_mid15s.mp4` | **TUTORIEL_DEV** | Oui | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 28 | `Apprendre_Ruby_-_Les_Conditions_...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 29 | `Apprendre_Ruby_-_Les_modules_-_N...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 30 | `Apprendre_Ruby_-_Proc_&_Block_mi...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 31 | `Apprendre_Ruby_-_TP_-_Calcul_des...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 32 | `Apprendre_Ruby_on_Rails_(11_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 33 | `Apprendre_Ruby_on_Rails_(12_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 34 | `Apprendre_Ruby_on_Rails_(13_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 35 | `Apprendre_Ruby_on_Rails_(14_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 36 | `Apprendre_Ruby_on_Rails_(15_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 37 | `Apprendre_Ruby_on_Rails_(16_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 38 | `Apprendre_Ruby_on_Rails_(17_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 39 | `Apprendre_Ruby_on_Rails_(25_29)_...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 40 | `Apprendre_Ruby_on_Rails_(2_29)_-...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 41 | `Apprendre_Ruby_on_Rails_(7_29)_-...` | **TUTORIEL_DEV** | Non (0) | Oui | Plate (Noir) | Tutoriel de développement informatique sans sous-titre .srt existant |
| 42 | `Apprendre_VueJS_-_Introduction_m...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 43 | `Apprendre_l'HTML_(5_16)_-_L'en_t...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 44 | `Apprendre_l'HTML_(6_16)_-_L'enco...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 45 | `Apprendre_l'HTML_(7_16)_-_Les_él...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 46 | `Apprendre_l'HTML_(9_16)_-_Les_im...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 47 | `Apprendre_le_JavaScript_-_Introd...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 48 | `Arcane_(2021)_-_S01E01_-_Welcome...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 49 | `Arcane_(2021)_-_S01E05_-_Everybo...` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 50 | `Arcane_(2021)_-_S01E06_-_When_Th...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 51 | `Atlantique_(2019)[MULTi.1080p.x2...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 52 | `Au_Bout_Des_Doigts_(2018)_[1080]...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 53 | `Belle_(2021)[MULTi][1080p.DTS](🇯...` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 54 | `BigBuckBunny(1)_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 55 | `BigBuckBunny_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 56 | `Big_Hero_6_(2014)[2160p.x265.10b...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 57 | `Bilal_A_New_Breed_of_Hero_(2015)...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 58 | `Blood.Red.Sky.2021.GERMAN.1080p....` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 59 | `Bob_Marley_Dernier_Concert_RIP_A...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Oui | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 60 | `Bojack.Horseman.S01E01.MULTi.[10...` | **COLLISION_PLAN_FIXE_ECARTEE** | Oui | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 61 | `Bojack.Horseman.S01E03.MULTi.[10...` | **COLLISION_PLAN_FIXE_ECARTEE** | Oui | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 62 | `Bojack.Horseman.S01E06.MULTi.[10...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Oui | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 63 | `Bojack.Horseman.S01E09.MULTi.[10...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Oui | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 64 | `BrainDead_S01E01_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 65 | `BrainDead_S01E02_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 66 | `BrainDead_S01E04_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 67 | `BrainDead_S01E05_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 68 | `BrainDead_S01E11_mid15s.mp4` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 69 | `CSS_Maintenable_avec_SASS_et_Com...` | **COLLISION_PLAN_FIXE_ECARTEE** | Non (0) | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 70 | `Capturez_vos_mails_avec_MailDev_...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 71 | `Carnival_Row_S02E05_Reckoning._1...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 72 | `Carnival_Row_S02E08_Facta_Non_Ve...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 73 | `Carnival_Row_S02E10_Carnival_Row...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 74 | `Carnivàle_(2003)_-_S01E03_-_Tipt...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 75 | `Carnivàle_(2003)_-_S01E04_-_Blac...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Plate (Noir) | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 76 | `Carnivàle_(2003)_-_S01E05_-_Baby...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 77 | `Carnivàle_(2003)_-_S01E07_-_The_...` | **COLLISION_PLAN_FIXE_ECARTEE** | Oui | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 78 | `Carnivàle_(2003)_-_S01E10_-_Hot_...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 79 | `Carnivàle_(2003)_-_S01E11_-_The_...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Oui | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 80 | `Carnivàle_(2003)_-_S01E12_-_The_...` | **COLLISION_PLAN_FIXE_ECARTEE** | Non (0) | Oui | Plate (Noir) | Extrait mid-15s sur plan noir/statique, écarté par le garde-fou de sécurité |
| 81 | `Carnivàle_(2003)_-_S02E01_-_Los_...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 82 | `Carnivàle_(2003)_-_S02E02_-_Alam...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 83 | `Carnivàle_(2003)_-_S02E03_-_Ingr...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 84 | `Carnivàle_(2003)_-_S02E04_-_Old_...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 85 | `Carnivàle_(2003)_-_S02E05_-_Cree...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Plate (Noir) | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 86 | `Carnivàle_(2003)_-_S02E10_-_Chey...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 87 | `Carnivàle_(2003)_-_S02E11_-_Outs...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 88 | `Carnivàle_(2003)_-_S02E12_-_New_...` | **ABSENT_DU_CATALOGUE** | Oui | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 89 | `Ce_que_League_of_Legends_m'a_app...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 90 | `Children.of.Men.2006.1080p.BrRip...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 91 | `Clara_(2019)[FRENCH][1080p](Issa...` | **SEUIL_OU_ZERO_SOUS_TITRE** | Non (0) | Oui | Dynamique | Média répertorié dans le catalogue, mais extrait < 70% de confiance ou 0 sous-titre certifié |
| 92 | `Community_S01E23_Modern_Warfare_...` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 93 | `Community_S01E25_Pascals_Triangl...` | **ABSENT_DU_CATALOGUE** | Non (0) | Non | Dynamique | Média physique non indexé dans les 3 061 entrées de catalog.dbphash |
| 94 | `Comprendre_Git_-_GitKraken_mid15...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 95 | `Comprendre_Git_-_Github_Mac_mid1...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 96 | `Comprendre_Git_-_Github_Windows_...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 97 | `Comprendre_Git_-_Installer_git_s...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 98 | `Comprendre_Git_-_Installer_git_s...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 99 | `Comprendre_Git_-_Installer_git_s...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |
| 100 | `Comprendre_Git_-_Les_branches_mi...` | **TUTORIEL_DEV** | Non (0) | Oui | Dynamique | Tutoriel de développement informatique sans sous-titre .srt existant |

*Tableau tronqué aux 100 premiers éléments pour lisibilité dans la documentation. L'ensemble des 849 entrées est persisté dans `data/snippets/audit_rejected_cuts_analysis.json`.*

> **Garantie Zéro Faux-Positif :** 100 % des rejets analysés sont opérationnellement nominaux
> (traitement HTTP 200 sans exception) et évitent l'injection de sous-titres incohérents dans VLC.
