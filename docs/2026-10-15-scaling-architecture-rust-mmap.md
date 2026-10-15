# Réflexion d'Architecture : Passage à l'Échelle (Milliards de Frames)

**Date** : 2026-10-15
**Auteur** : Antigravity CLI
**Statut** : Étude d'ingénierie & R&D

---

## 1. Retour d'Expérience : Pourquoi l'Approche Mémoire en Python a Échoué

L'historique du projet montrait une tentative d'arbre métrique (BK-Tree / VP-Tree) directement en mémoire Python pour éviter la dépendance à une base de données. Cette approche s'est heurtée au mur de l'échelle dès quelques centaines de milliers de frames :

1. **Overhead structurel des objets Python (`PyObject`)** :
   * En Python, un entier n'est pas un scalaire 64-bit brut, c'est un `PyLongObject` alloué sur le tas (28 octets minimum).
   * Un nœud d'arbre (objet de classe ou dictionnaire d'arêtes + listes d'enfants + pointeurs de références) consomme entre **80 et 120 octets de RAM par frame**.
   * **Conséquence pour 1 milliard de frames** :
     $$\text{RAM requise} \approx 10^9 \times 100 \text{ octets} = 100 \text{ Go de RAM}$$
2. **Le verrou du Garbage Collector (GC)** :
   * Le GC de CPython doit traverser périodiquement des millions de pointeurs en graphe.
   * Les cycles de collection provoquent des gels complets du processus (*stop-the-world*) de plusieurs dizaines de secondes.
3. **Le verrou du GIL (Global Interpreter Lock)** :
   * Impossible d'interroger ou de construire l'arbre en multi-threading CPU réel sans sérialisation ou duplication inter-processus via `multiprocessing` (qui multiplie encore la consommation mémoire par le nombre de workers).

---

## 2. Pistes de Passage à l'Échelle (Milliards de Frames)

Pour traiter des millions de vidéos (plusieurs milliards de pHash 64-bit), trois paradigmes viables existent :

### Option A — Base Relationnelle PostgreSQL 17 + BK-Tree + Sharding / Partitioning
* **Mécanisme** :
  * Table `frames` partitionnée nativement par hash range ou par `media_id` (tranches de 10 à 50 millions de lignes).
  * Chaque partition dispose de son index `SP-GiST` BK-Tree indépendant.
  * Stockage disque géré par PostgreSQL avec buffer cache mutualisé.
* **Avantages** :
  * Requêtes SQL complexes (jointures avec sous-titres, métadonnées vidéos, timestamps).
  * Concurrence transactionnelle ACID native, outillage d'administration mature.
* **Limites** :
  * Nécessite un cluster PostgreSQL lourd et bien dimensionné (disques NVMe rapides, RAM conséquente pour le `shared_buffers`).

---

### Option B — Moteur Rust Bare-Metal avec Memory-Mapped Files (`mmap`) & SIMD (Recommandé)
Cette approche élimine tout SGBD lourd pour les calculs de matching tout en contournant les limites mémoire de Python.

#### 1. Densité Binaire Brute
* En Rust, un pHash 64-bit est un type primitif `u64` (ou `[u8; 8]`).
* Il est stocké **de manière contiguë, sans pointeur, sans indirection, sans GC**.
* **Pour 1 milliard de frames** :
  $$\text{Taille sur disque/RAM} = 10^9 \times 8 \text{ octets} = \mathbf{8 \text{ Go}}$$
  *(12 fois plus compact qu'en Python).*

#### 2. Mémoire Virtuelle Zero-Copy (`mmap`)
* Les hashes et l'index métrique sont stockés dans un format binaire optimisé (structure compacte type flat array ou VP-tree sérialisé en tableau plat).
* Le fichier est projeté en mémoire virtuelle via l'appel système `mmap(2)` (`memmap2` en Rust).
* **Consommation RAM physique** :
  * Le noyau Linux ne charge en mémoire que les pages (4 Ko ou HugePages 2 Mo) effectivement lues pendant la recherche arborescente.
  * L'application peut démarrer instantanément (zéro temps de désérialisation au boot) et tourner sur une machine avec seulement 4 ou 8 Go de RAM physique même pour un fichier de 50 Go.

#### 3. Accélération Matérielle SIMD & Parallélisme
* **Calcul Hamming en 1 cycle d'horloge** :
  * La distance de Hamming entre deux `u64` se calcule par l'instruction CPU matérielle :
    ```rust
    let dist = (hash_a ^ hash_b).count_ones(); // Compile en POPCNT
    ```
* **SIMD AVX-512 / AVX2** :
  * Comparaison vectorielle de 8 hashes simultanément par registre 512-bit (`_mm512_popcnt_epi64`).
* **Multi-threading sans verrou** :
  * Parallélisation naturelle de la recherche par `rayon` sur l'ensemble des cœurs CPU disponibles, sans GIL.

---

### Option C — Base Vectorielle Dédiée Distribuée (Milvus / Qdrant)
* Adaptée si le système est déployé sous forme de cluster distribué multi-nœuds avec sharding automatique.
* Gère nativement les métriques de Hamming sur vecteurs binaires (`BIN_FLAT`, `BIN_IVF_FLAT`).
* Plus complexe à opérer qu'un moteur embarqué Rust ou qu'une base Postgres unique.

---

## 3. Synthèse Stratégique & Feuille de Route

| Niveau d'Usage | Solution Technique | Pourquoi |
| :--- | :--- | :--- |
| **Niveau 1 : CLI & Desktop (90% des cas)** | **Moteur Rust Embedded (`dbsrt-rs`)** | Zéro base de données à installer pour l'utilisateur final. Single-binary portable, démarrage instantané, consommation RAM infime. |
| **Niveau 2 : Serveur / Cluster de sous-titrage** | **PostgreSQL 17 + extension `bktree`** | Métadonnées riches, API REST/gRPC, transactions, synchronisation multi-utilisateurs. |
| **Niveau 3 : Échelle Web (>10 milliards de frames)** | **Rust Daemon mmap distribué ou Milvus** | Partitionnement distribué haute densité. |
