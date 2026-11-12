# Analyse Comparative : MIH-16 vs MIH-32 sur Empreintes aHash et pHash

Ce document d'explication analyse les fondements algorithmiques, spectraux et empiriques qui expliquent la divergence radicale de comportement entre le hachage perceptuel visuel (**pHash**) et acoustique (**aHash**) lorsqu'on varie la granularité du Multi-Index Hashing (**MIH-16** vs **MIH-32**) et la stratégie de parallélisation (**Rayon**).

---

## 1. Contexte et Problématique de Scalabilité

Lors de l'implémentation de l'Issue #79 (*Recherche de sous-titres par empreinte acoustique aHash*), le passage d'une batterie de tests synthétiques à un catalogue réel de production a mis en évidence un contraste de performance saisissant :

- **Catalogue visuel pHash** : 2 708 720 frames (2 758 médias, 1 fps sparse + scene cuts), recherché en **3.92 ms** via MIH-16.
- **Catalogue audio aHash** : 106 096 618 frames (584 médias, 31.25 fps continu), dont la recherche linéaire exhaustive $\mathcal{O}(N)$ nécessitait **7 272 ms** (7.27 s) par extrait de 15 secondes (soit 57 milliards d'opérations XOR/popcount).

L'objectif était de tester si la technique de partitionnement métrique MIH (Multi-Index Hashing) pouvait ramener aHash sous le seuil contractuel de **50 ms**, et de comparer rigoureusement l'impact de blocs de 16 bits (**MIH-16**) vs 32 bits (**MIH-32**) sur les deux types d'empreintes.

---

## 2. Résultats Empiriques sur 10 Extraits Réels de 15 Secondes

L'évaluation a été exécutée de manière hermétique sur les 10 vidéos de test réelles (`data/snippets_ahash/*.mp4`), sous profil compilé `release` avec 12 threads Rayon :

### Tableau A : Espace Acoustique aHash (106 096 618 frames / 584 médias)

| Stratégie aHash | Latence Moyenne | Facteur d'Accélération | Taux de Détection (10 cuts) | SNR Moyen |
|---|---|---|---|---|
| **Linear Scan $\mathcal{O}(N)$** | 7 272 ms | 1.0x (référence) | 10/10 (100%) | 30.1 bits |
| **MIH-16 Mono** | 141.1 ms | **$\times 51.5$** | 10/10 (100%) | 30.1 bits |
| **MIH-16 Rayon** | 113.9 ms | **$\times 63.8$** | 10/10 (100%) | 30.1 bits |
| **MIH-32 Mono** | 3.0 ms | **$\times 2\,424$** | 10/10 (100%) | 30.1 bits |
| **MIH-32 Rayon** | **2.1 ms** | **$\times 3\,462$** | **10/10 (100%)** | **30.1 bits** |

### Tableau B : Espace Visuel pHash (2 708 720 frames / 2 758 médias)

| Stratégie pHash | Latence Moyenne | Facteur d'Accélération | Taux de Détection (10 cuts) | Fiabilité Métrique |
|---|---|---|---|---|
| **Linear Scan $\mathcal{O}(N)$** | 128.72 ms | 1.0x (référence) | 10/10 (100%) | Vérité terrain exhaustive |
| **MIH-16 Mono** | 10.28 ms | **$\times 12.5$** | 10/10 (100%) | 100% conformité offset |
| **MIH-16 Rayon** | **3.92 ms** | **$\times 32.8$** | **10/10 (100%)** | **100% conformité offset** |
| **MIH-32 Mono** | 0.11 ms | $\times 1\,170$ | 7/10 (70%) | ❌ 30% d'échec / divergence |
| **MIH-32 Rayon** | 0.12 ms | $\times 1\,072$ | 7/10 (70%) | ❌ 30% d'échec / divergence |

---

## 3. Pourquoi MIH-32 Triomphe sur aHash ($\times 3\,462$) mais Échoue sur pHash (30% d'Échec) ?

Ce résultat, très contre-intuitif à première vue, s'explique par la combinaison de trois propriétés fondamentales : l'espace combinatoire des seaux, la redondance temporelle de l'échantillonnage, et la nature spectrale de la dispersion des erreurs de réencodage.

### A. Capacité des Seaux et Saturation des Collisions (MIH-16 vs MIH-32)

Dans une table de hachage de blocs de $b$ bits, l'espace d'adressage total est de $2^b$ seaux.

- **En MIH-16 ($b = 16$)** :
  L'espace ne comporte que $2^{16} = 65\,536$ seaux possibles.
  Pour un catalogue audio de $N = 106\,096\,618$ frames :

$$\text{Profondeur moyenne} = \frac{106\,096\,618}{65\,536} \approx 1\,618\text{ éléments par seau}$$

  De surcroît, les signaux audio réels comportent des passages silencieux, des bruits de fond génériques ou des accords prolongés qui concentrent des dizaines de milliers de frames dans un petit nombre de seaux (certains dépassant 50 000 éléments).
  Pour une requête de 15 secondes comportant 107 frames de sonde interrogées sur 4 tables de 16 bits, MIH-16 génère plus de **600 000 collisions candidates**. L'accumulation des votes dans l'histogramme devient le goulot d'étranglement majeur (113 ms).

- **En MIH-32 ($b = 32$)** :
  L'espace d'adressage s'élargit à $2^{32} \approx 4.29 \times 10^9$ seaux.

$$\text{Profondeur moyenne} = \frac{106\,096\,618}{4\,294\,967\,296} \approx 0.0247\text{ élément par seau non vide (soit } 1.52 \text{ à } 1.71 \text{ dans les seaux actifs)}$$

  Le bruit et les collisions fortuites s'effondrent : pour l'ensemble des 107 frames de sonde, le nombre total de candidats collectés tombe de 600 000 à **environ 350 candidats**. L'évaluation temporelle ne traite que des corrélations réelles, amenant la latence à **2.1 ms**.

### B. Nombre de Frames de Sonde (430 frames vs 15 frames)

- **aHash (Audio)** : Échantillonné à 31.25 fps (hop de 32 ms), une fenêtre de 15 secondes contient **430 frames**. Même si le réencodage ou la compression audio (AAC, MP3, AC3) altère une partie des bits, il suffit qu'une fraction minime des frames préserve un demi-mot de 32 bits intact pour que le media accumule 10 à 30 votes concordants et soit identifié avec certitude.
- **pHash (Vidéo)** : Échantillonné à 1 fps sparse, la fenêtre de 15 secondes ne contient que **15 frames**. Si les collisions de 32 bits manquent ne serait-ce que sur 5 ou 6 frames, le nombre de votes résiduels tombe à 0 ou 1, ce qui est insuffisant pour émerger au-dessus du seuil de décision.

### C. Dispersion des Bits et Principe des Tiroirs (Pigeonhole Principle)

Le hachage visuel pHash repose sur une Transformée en Cosinus Discrète (DCT) 2D $8 \times 8$. Lorsqu'une vidéo est transcodée ou redimensionnée, les perturbations affectent uniformément l'ensemble des coefficients fréquentiels bas et moyens. La distance de Hamming typique observée entre une vidéo de référence et un réencodage est de **3 à 6 bits sur 64**.

Considérons une frame ayant $d = 6$ bits inversés :

1. **Partitionnement en 2 blocs de 32 bits (MIH-32)** :
   La probabilité qu'un bloc de 32 bits ne subisse **aucun** bit inversé parmi les 6 s'exprime par le ratio combinatoire :

$$P(\text{bloc intact}) = \frac{\binom{32}{0} \binom{32}{6}}{\binom{64}{6}} = \frac{906\,192}{74\,974\,368} \approx 1.2\%$$

   Pour les deux blocs, la probabilité d'avoir au moins un bloc indemne est inférieure à **3%**.
   Autrement dit, **97% des frames visuelles réencodées ne génèrent aucune collision exacte dans MIH-32**.
   Sur un extrait de 15 frames, le nombre attendu de collisions est de $15 \times 0.03 \approx 0.45$. C'est pourquoi MIH-32 échoue sur 30% des extraits vidéo (cut 5 : 0 match, cuts 4 et 6 : offsets erronés).

2. **Partitionnement en 4 blocs de 16 bits avec rayon 1 (MIH-16)** :
   Par le **principe des tiroirs (Pigeonhole Principle)**, si l'on distribue $d = 6$ bits inversés dans $m = 4$ blocs de 16 bits :

$$\min_{i \in \{1..4\}} (k_i) \le \left\lfloor \frac{d}{m} \right\rfloor = \left\lfloor \frac{6}{4} \right\rfloor = 1$$

   Il est **mathématiquement garanti** qu'au moins un des 4 blocs de 16 bits contiendra **au plus 1 bit inversé**.
   Comme l'implémentation MIH-16 interroge le voisinage à distance de Hamming $r \le 1$ autour de chaque sous-bloc (17 clés testées par table), **100% des frames ayant jusqu'à 6 bits d'erreur sont garanties d'être retrouvées dans les index**.

---

## 4. Comportement du Multi-Threading Rayon (Mono-Thread vs Multi-Threads)

Le benchmark montre que le gain apporté par Rayon dépend fortement du goulot d'étranglement de l'algorithme :

| Configuration | Latence Mono | Latence Rayon | Gain Rayon | Explication du Comportement |
|---|---|---|---|---|
| **aHash MIH-16** | 141.1 ms | 113.9 ms | $\times 1.24$ | Goulot d'étranglement sur l'agrégation concurrente des 600k votes (contention mémoire / fusion de maps). |
| **aHash MIH-32** | 3.0 ms | 2.1 ms | $\times 1.43$ | La recherche binaire dans les tables triées est déjà quasi instantanée en mono-thread ; le surcoût de spawn des tâches Rayon amortit le gain. |
| **pHash MIH-16** | 10.28 ms | 3.92 ms | $\times 2.62$ | Parallélisation idéale sur les scènes et vérification fine des candidats. |
| **pHash MIH-32** | 0.11 ms | 0.12 ms | $\times 0.92$ | Le temps de calcul ($\approx 100\,\mu\text{s}$) est inférieur au temps de réveil d'un thread pool Rayon. |

---

## 5. Décisions d'Architecture Logique (SoC)

Sur la base de ces preuves empiriques :

1. **Espace Visuel (pHash)** :
   - Maintien strict de **MIH-16** ($m=4$, rayon 1).
   - MIH-32 est formellement exclu en vidéo car il sacrifie le recall (30% d'échecs inacceptables en production).
2. **Espace Acoustique (aHash)** :
   - Adoption de **MIH-32** comme moteur primaire (`search_mih32`), assurant une recherche instantanée en **2.1 ms** (gain $\times 3\,462$) et un respect total du KPI (< 50 ms).
   - Maintien d'une cascade de résilience : si MIH-32 ne recueille aucun vote sur un signal très bruité, l'algorithme bascule automatiquement sur MIH-16 puis sur le balayage linéaire exhaustif.
