//! Hierarchical Coarse-to-Fine index for acoustic fingerprints.
//!
//! Sub-samples 31.25 fps acoustic streams by factor $K$ to create a compact,
//! low-RAM primary filter, followed by localized fine dense correlation.
//!
//! The catalog side is decimated by $K$ (RAM footprint $1/K$) while the query side
//! is **never** decimated: every dense query frame probes the coarse MIH table, so
//! that for any query phase $p = R_0 \bmod K$ about $|Q|/K$ probes land exactly on a
//! coarse anchor $0, K, 2K, \dots$ (zero temporal delta).

use crate::ahash::{
    AHashCatalog, AHashCatalogEntry, AHashCatalogMatch, AHashMih32Index, Mih32Entry,
    MILLIS_PER_SEC, MIN_QUERY_FRAMES,
};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Default radius in seconds around coarse offset for localized dense search.
pub const DEFAULT_FINE_WINDOW_RADIUS_SEC: f64 = 2.0;

/// Default downsampling factor $K$ for acoustic coarse-to-fine index.
pub const DEFAULT_COARSE_FACTOR_K: usize = 16;

/// Default Hamming distance threshold for fine search validation.
pub const DEFAULT_FINE_MAX_DISTANCE: f64 = 14.0;

/// Maximum candidate bucket size for MIH slice lookup before skipping dense collision.
const MAX_MIH_SLICE_CANDIDATES: usize = 20_000;

/// Number of top-ranked coarse clusters verified by the fine dense stage.
pub const MAX_COARSE_CANDIDATES_TO_VERIFY: usize = 8;

/// Width (in dense frames) of one jitter bucket used to group offset votes.
///
/// Adjacent buckets `b - 1, b, b + 1` are summed so that $\pm 1$ frame of encoder
/// jitter cannot split a true cluster across a bucket boundary.
pub const COARSE_JITTER_BUCKET_FRAMES: usize = 4;

/// Minimum summed votes for a coarse cluster to be kept as a candidate.
const MIN_COARSE_CLUSTER_VOTES: u32 = 1;

/// Two clusters of the same media closer than this (dense frames) are one candidate.
const COARSE_SUPPRESSION_RADIUS_FRAMES: usize = 2 * COARSE_JITTER_BUCKET_FRAMES;

/// Number of bits in one MIH half-chunk (upper / lower 32 bits of a 64-bit hash).
const MIH_CHUNK_BITS: u32 = 32;

/// Bit mask selecting the lower 32-bit chunk of a 64-bit hash.
const MIH_LOWER_CHUNK_MASK: u64 = 0xFFFF_FFFF;

/// Reference noise floor (random 64-bit hashes differ on 32 bits on average).
const RANDOM_HASH_DISTANCE_BITS: f64 = 32.0;

/// Number of bits in one acoustic sub-fingerprint (normalizes the match score).
const HASH_BITS: f64 = 64.0;

/// Criteria context for fine-stage dense search.
#[derive(Debug, Clone)]
pub struct FineSearchCriteria<'a> {
    /// Dense query fingerprint hashes.
    pub dense_query: &'a [u64],
    /// Sampling framerate of dense query.
    pub dense_fps: f64,
    /// Maximum allowed Hamming distance.
    pub max_distance: f64,
}

/// Hierarchical Coarse-to-Fine index for acoustic fingerprints.
pub struct CoarseToFineAHashIndex {
    /// Downsample factor K (e.g. 16 for 1.95 fps coarse tier).
    pub factor_k: usize,
    /// Sub-sampled coarse catalog holding 1/K-th of the frames.
    pub coarse_catalog: AHashCatalog,
    /// 32-bit Multi-Index Hash table built over the coarse flat hashes.
    pub coarse_mih32: AHashMih32Index,
}

/// Candidate reference location produced by the coarse stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CoarseCluster {
    /// Summed votes over the jitter bucket and its two neighbours.
    votes: u32,
    /// Index of the media in the coarse (and dense) catalog.
    m_idx: usize,
    /// Estimated dense-frame start of the query inside the reference media.
    approx_dense_start: usize,
}

/// Per-bucket vote accumulator (deterministic, no hashing).
#[derive(Debug, Clone, Copy, Default)]
struct BucketAcc {
    /// Total votes falling in this bucket.
    votes: u32,
    /// Exact dense start holding the most votes inside this bucket.
    peak_start: usize,
    /// Votes received by `peak_start`.
    peak_count: u32,
}

/// Groups raw `(m_idx, approx_dense_start)` hits into per-bucket accumulators.
///
/// # Arguments
/// * `hits` - Hits sorted ascending by `(m_idx, approx_dense_start)`.
///
/// # Returns
/// Ordered map `(m_idx, bucket) -> BucketAcc`; empty if `hits` is empty.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. Run-length encodes identical consecutive hits into `(m_idx, start, count)`.
/// 2. Adds `count` to the bucket `start / COARSE_JITTER_BUCKET_FRAMES`.
/// 3. Because iteration is ascending, ties on `peak_count` keep the smallest start.
fn accumulate_buckets(hits: &[(usize, usize)]) -> BTreeMap<(usize, usize), BucketAcc> {
    let mut buckets: BTreeMap<(usize, usize), BucketAcc> = BTreeMap::new();
    let mut idx = 0;
    while idx < hits.len() {
        let (m_idx, start) = hits[idx];
        let run_end = idx + hits[idx..].iter().take_while(|&&h| h == hits[idx]).count();
        let count = (run_end - idx) as u32;
        let acc = buckets
            .entry((m_idx, start / COARSE_JITTER_BUCKET_FRAMES))
            .or_default();
        acc.votes += count;
        if count > acc.peak_count {
            acc.peak_count = count;
            acc.peak_start = start;
        }
        idx = run_end;
    }
    buckets
}

/// Sums votes over buckets `b - 1, b, b + 1` of one media and elects a peak start.
///
/// # Arguments
/// * `buckets` - Accumulators from [`accumulate_buckets`].
/// * `m_idx` - Media index.
/// * `bucket` - Central jitter bucket.
///
/// # Returns
/// The aggregated [`CoarseCluster`].
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Ascending bucket iteration + strict `>` keeps the smallest start on peak ties.
fn window_cluster(
    buckets: &BTreeMap<(usize, usize), BucketAcc>,
    m_idx: usize,
    bucket: usize,
) -> CoarseCluster {
    let lo = bucket.saturating_sub(1);
    let hi = bucket.saturating_add(1);
    let mut votes = 0u32;
    let mut peak_count = 0u32;
    let mut peak_start = 0usize;
    for acc in buckets.range((m_idx, lo)..=(m_idx, hi)).map(|(_, a)| a) {
        votes += acc.votes;
        if acc.peak_count > peak_count {
            peak_count = acc.peak_count;
            peak_start = acc.peak_start;
        }
    }
    CoarseCluster {
        votes,
        m_idx,
        approx_dense_start: peak_start,
    }
}

/// Drops near-duplicate clusters of the same media and keeps the top candidates.
///
/// # Arguments
/// * `sorted` - Clusters already in deterministic ranking order.
///
/// # Returns
/// At most [`MAX_COARSE_CANDIDATES_TO_VERIFY`] clusters, rank order preserved.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Greedy non-maximum suppression: a cluster is kept only if no already kept
/// cluster of the same media lies within [`COARSE_SUPPRESSION_RADIUS_FRAMES`].
fn suppress_and_truncate(sorted: Vec<CoarseCluster>) -> Vec<CoarseCluster> {
    let mut kept: Vec<CoarseCluster> = Vec::with_capacity(MAX_COARSE_CANDIDATES_TO_VERIFY);
    for cluster in sorted {
        if kept.len() >= MAX_COARSE_CANDIDATES_TO_VERIFY {
            break;
        }
        let duplicate = kept.iter().any(|k| {
            k.m_idx == cluster.m_idx
                && k.approx_dense_start.abs_diff(cluster.approx_dense_start)
                    <= COARSE_SUPPRESSION_RADIUS_FRAMES
        });
        if !duplicate {
            kept.push(cluster);
        }
    }
    kept
}

/// Ranks raw coarse hits into deterministic candidate clusters.
///
/// # Arguments
/// * `hits` - Unordered `(m_idx, approx_dense_start)` hits (one per vote).
///
/// # Returns
/// Top clusters sorted by `votes DESC`, then `m_idx ASC`, then
/// `approx_dense_start ASC` (100% deterministic, independent of thread scheduling).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. Sorts hits (removes any rayon / hash ordering effect).
/// 2. Buckets votes by jitter step and aggregates adjacent buckets.
/// 3. Filters by [`MIN_COARSE_CLUSTER_VOTES`], sorts, suppresses duplicates, truncates.
fn rank_clusters(mut hits: Vec<(usize, usize)>) -> Vec<CoarseCluster> {
    hits.sort_unstable();
    let buckets = accumulate_buckets(&hits);
    let mut clusters: Vec<CoarseCluster> = buckets
        .keys()
        .map(|&(m_idx, bucket)| window_cluster(&buckets, m_idx, bucket))
        .filter(|c| c.votes >= MIN_COARSE_CLUSTER_VOTES)
        .collect();
    clusters.sort_by_key(|c| (std::cmp::Reverse(c.votes), c.m_idx, c.approx_dense_start));
    suppress_and_truncate(clusters)
}

impl CoarseToFineAHashIndex {
    /// Builds a coarse-to-fine index from a dense catalog by sub-sampling every K-th frame.
    ///
    /// # Arguments
    /// * `dense_catalog` - Reference to loaded dense [`AHashCatalog`].
    /// * `factor_k` - Sub-sampling stride (must be $\ge 1$).
    ///
    /// # Returns
    /// Initialized [`CoarseToFineAHashIndex`].
    ///
    /// # Panics
    /// Panics if `factor_k` is 0.
    ///
    /// # Data Flow & Invariants
    /// 1. Par-iterates over catalog entries to downsample hashes by stride $K$.
    /// 2. Assembles compact coarse catalog ($1/K$ RAM footprint).
    /// 3. Coarse frame `c` of a media always corresponds to dense frame `c * K`.
    /// 4. Builds multi-index hash table on coarse flat hashes for sub-millisecond candidate lookup.
    pub fn build(dense_catalog: &AHashCatalog, factor_k: usize) -> Self {
        assert!(factor_k >= 1, "Downsample factor must be >= 1");

        let coarse_entries: Vec<AHashCatalogEntry> = dense_catalog
            .entries
            .par_iter()
            .map(|entry| {
                let coarse_hashes: Vec<u64> =
                    entry.hashes.iter().step_by(factor_k).copied().collect();
                let coarse_fps = entry.audio_fps / factor_k as f64;
                let frame_count = coarse_hashes.len();
                AHashCatalogEntry {
                    media_name: entry.media_name.clone(),
                    path: entry.path.clone(),
                    audio_fps: coarse_fps,
                    frame_count,
                    hashes: Arc::new(coarse_hashes),
                }
            })
            .collect();

        let coarse_catalog = AHashCatalog::new(coarse_entries);
        let coarse_mih32 = AHashMih32Index::build(&coarse_catalog.flat_hashes);

        Self {
            factor_k,
            coarse_catalog,
            coarse_mih32,
        }
    }

    /// Performs two-tier coarse-to-fine acoustic search.
    ///
    /// # Arguments
    /// * `dense_catalog` - Reference to the full-resolution dense [`AHashCatalog`].
    /// * `criteria` - Search criteria context ([`FineSearchCriteria`]).
    ///
    /// # Returns
    /// `Option<AHashCatalogMatch>` containing the certified fine match with the lowest
    /// `mean_distance <= criteria.max_distance`, or `None`.
    ///
    /// # Errors
    /// Infallible (never returns Err).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Probes the coarse MIH-32 table with **every dense** query frame (stride 1),
    ///    so ~`|Q|/K` probes align exactly with coarse anchors for any query phase.
    /// 2. Each hit `(m_idx, ref_coarse_idx)` of query frame `q` votes for the dense
    ///    start `ref_coarse_idx * K - q`.
    /// 3. Votes are bucketed (jitter tolerant) and ranked deterministically.
    /// 4. The top [`MAX_COARSE_CANDIDATES_TO_VERIFY`] clusters are verified by dense
    ///    correlation in a narrow window; the lowest mean distance wins (ties: rank order).
    pub fn search(
        &self,
        dense_catalog: &AHashCatalog,
        criteria: &FineSearchCriteria<'_>,
    ) -> Option<AHashCatalogMatch> {
        if criteria.dense_query.len() < MIN_QUERY_FRAMES {
            return None;
        }
        let hits = self.collect_dense_votes(criteria.dense_query);
        let clusters = rank_clusters(hits);
        self.best_verified_match(dense_catalog, criteria, &clusters)
    }

    /// Verifies ranked clusters with the fine stage and keeps the closest match.
    ///
    /// # Arguments
    /// * `dense_catalog` - Dense reference catalog.
    /// * `criteria` - Fine search criteria.
    /// * `clusters` - Ranked candidate clusters.
    ///
    /// # Returns
    /// The candidate with the smallest `mean_distance`, or `None` if none certifies.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Strict `<` comparison: on equal distance the better-ranked cluster is kept.
    fn best_verified_match(
        &self,
        dense_catalog: &AHashCatalog,
        criteria: &FineSearchCriteria<'_>,
        clusters: &[CoarseCluster],
    ) -> Option<AHashCatalogMatch> {
        let mut best: Option<AHashCatalogMatch> = None;
        for cluster in clusters {
            let Some(coarse_entry) = self.coarse_catalog.entries.get(cluster.m_idx) else {
                continue;
            };
            let Some(dense_entry) = dense_catalog
                .entries
                .iter()
                .find(|e| e.media_name == coarse_entry.media_name)
            else {
                continue;
            };
            let Some(candidate) =
                self.evaluate_fine_window(dense_entry, criteria, cluster.approx_dense_start)
            else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|b| candidate.mean_distance < b.mean_distance)
            {
                best = Some(candidate);
            }
        }
        best
    }

    /// Probes the coarse MIH-32 tables with every dense query frame.
    ///
    /// # Arguments
    /// * `dense_query` - Dense (undecimated) query hashes.
    ///
    /// # Returns
    /// Unordered `(m_idx, approx_dense_start)` hits, one per matching 32-bit half.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// One hit buffer per rayon split (fold/reduce); final order is irrelevant because
    /// [`rank_clusters`] sorts before aggregating.
    fn collect_dense_votes(&self, dense_query: &[u64]) -> Vec<(usize, usize)> {
        dense_query
            .par_iter()
            .enumerate()
            .fold(Vec::new, |mut acc, (q_idx, &q_hash)| {
                self.probe_frame(q_idx, q_hash, &mut acc);
                acc
            })
            .reduce(Vec::new, |mut left, mut right| {
                left.append(&mut right);
                left
            })
    }

    /// Looks up both 32-bit halves of one dense query frame.
    fn probe_frame(&self, q_idx: usize, q_hash: u64, hits: &mut Vec<(usize, usize)>) {
        let upper = (q_hash >> MIH_CHUNK_BITS) as u32;
        let lower = (q_hash & MIH_LOWER_CHUNK_MASK) as u32;
        self.append_slice_hits(self.coarse_mih32.lookup_upper(upper), q_idx, hits);
        self.append_slice_hits(self.coarse_mih32.lookup_lower(lower), q_idx, hits);
    }

    /// Appends offset votes from one MIH slice for dense query frame `q_idx`.
    ///
    /// # Arguments
    /// * `slice` - Coarse MIH entries sharing the probed 32-bit chunk.
    /// * `q_idx` - Index of the probing frame inside the dense query.
    /// * `hits` - Output buffer of `(m_idx, approx_dense_start)`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// `approx_dense_start = ref_coarse_idx * K - q_idx`; hits with a negative start
    /// (`checked_sub` fails) are discarded. Oversized slices are skipped.
    fn append_slice_hits(
        &self,
        slice: &[Mih32Entry],
        q_idx: usize,
        hits: &mut Vec<(usize, usize)>,
    ) {
        if slice.len() > MAX_MIH_SLICE_CANDIDATES {
            return;
        }
        for entry in slice {
            let (m_idx, coarse_idx) = self
                .coarse_catalog
                .resolve_global_index(entry.frame_idx as usize);
            if let Some(start) = (coarse_idx * self.factor_k).checked_sub(q_idx) {
                hits.push((m_idx, start));
            }
        }
    }

    /// Evaluates fine-grained cross correlation within a localized window.
    fn evaluate_fine_window(
        &self,
        dense_entry: &AHashCatalogEntry,
        criteria: &FineSearchCriteria<'_>,
        approx_offset: usize,
    ) -> Option<AHashCatalogMatch> {
        let q_len = criteria.dense_query.len();
        let window_radius =
            ((DEFAULT_FINE_WINDOW_RADIUS_SEC * criteria.dense_fps) as usize).max(self.factor_k * 2);

        let min_s = approx_offset.saturating_sub(window_radius);
        let max_s =
            (approx_offset + window_radius).min(dense_entry.hashes.len().saturating_sub(q_len));

        if min_s > max_s || dense_entry.hashes.len() < q_len {
            return None;
        }

        let mut best_shift = min_s;
        let mut best_dist = u64::MAX;

        for s in min_s..=max_s {
            let ref_slice = &dense_entry.hashes[s..s + q_len];
            let mut dist = 0u64;
            for (&q, &r) in criteria.dense_query.iter().zip(ref_slice) {
                dist += (q ^ r).count_ones() as u64;
            }
            if dist < best_dist {
                best_dist = dist;
                best_shift = s;
            }
        }

        let mean_dist = best_dist as f64 / q_len as f64;
        if mean_dist > criteria.max_distance {
            return None;
        }

        let score = (1.0 - (mean_dist / HASH_BITS)).clamp(0.0, 1.0);
        let snr_bits = (RANDOM_HASH_DISTANCE_BITS - mean_dist).max(0.0);
        let offset_sec = best_shift as f64 / criteria.dense_fps;

        Some(AHashCatalogMatch {
            media_name: dense_entry.media_name.clone(),
            file_path: dense_entry.path.to_string_lossy().to_string(),
            ref_frame_offset: best_shift,
            offset_sec,
            offset_ms: (offset_sec * MILLIS_PER_SEC).round() as i64,
            mean_distance: mean_dist,
            score,
            snr_bits,
            audio_fps: criteria.dense_fps,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic, hermetic tests (no filesystem, no network, no external media).

    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;

    /// Decimation factor under test.
    const TEST_FACTOR_K: usize = 16;
    /// Dense query window: 5 s at 86.125 fps.
    const TEST_QUERY_FRAMES: usize = 430;
    /// Dense frame rate used by the synthetic catalog.
    const TEST_DENSE_FPS: f64 = 31.25;
    /// Frames per synthetic reference media.
    const TEST_MEDIA_FRAMES: usize = 20_000;
    /// Number of synthetic reference media.
    const TEST_MEDIA_COUNT: usize = 3;
    /// Index of the media the queries are cut from.
    const TEST_TARGET_MEDIA: usize = 1;
    /// Grid-aligned base offset (multiple of K) of the query window.
    const TEST_BASE_OFFSET: usize = 4_096;
    /// Minimum number of flipped bits per query frame.
    const TEST_MIN_FLIPS: u64 = 2;
    /// Number of distinct flip counts (2, 3 or 4 bits).
    const TEST_FLIP_SPAN: u64 = 3;
    /// PRNG seeds (catalog content, query noise).
    const TEST_CATALOG_SEED: u64 = 0x5EED_CA7A;
    const TEST_NOISE_SEED: u64 = 0xA015_E5ED;
    /// SplitMix64 constants.
    const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
    const SPLITMIX_MUL_1: u64 = 0xBF58_476D_1CE4_E5B9;
    const SPLITMIX_MUL_2: u64 = 0x94D0_49BB_1331_11EB;
    const SPLITMIX_SHIFT_1: u32 = 30;
    const SPLITMIX_SHIFT_2: u32 = 27;
    const SPLITMIX_SHIFT_3: u32 = 31;
    /// Bits per hash (flip positions are drawn in `0..HASH_BITS_U64`).
    const HASH_BITS_U64: u64 = 64;

    /// Deterministic SplitMix64 step.
    fn next_u64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(SPLITMIX_GAMMA);
        let mut z = *state;
        z = (z ^ (z >> SPLITMIX_SHIFT_1)).wrapping_mul(SPLITMIX_MUL_1);
        z = (z ^ (z >> SPLITMIX_SHIFT_2)).wrapping_mul(SPLITMIX_MUL_2);
        z ^ (z >> SPLITMIX_SHIFT_3)
    }

    fn make_entry(name: &str, seed: u64) -> AHashCatalogEntry {
        let mut state = seed;
        let hashes: Vec<u64> = (0..TEST_MEDIA_FRAMES)
            .map(|_| next_u64(&mut state))
            .collect();
        AHashCatalogEntry {
            media_name: name.to_string(),
            path: PathBuf::from(format!("{name}.ahash")),
            audio_fps: TEST_DENSE_FPS,
            frame_count: hashes.len(),
            hashes: Arc::new(hashes),
        }
    }

    fn make_catalog() -> AHashCatalog {
        let entries = (0..TEST_MEDIA_COUNT)
            .map(|i| make_entry(&format!("media_{i}"), TEST_CATALOG_SEED + i as u64))
            .collect();
        AHashCatalog::new(entries)
    }

    /// Copies a reference window and flips 2..=4 deterministic random bits per frame.
    fn noisy_window(reference: &[u64], start: usize, noise_state: &mut u64) -> Vec<u64> {
        reference[start..start + TEST_QUERY_FRAMES]
            .iter()
            .map(|&h| {
                let flips = TEST_MIN_FLIPS + next_u64(noise_state) % TEST_FLIP_SPAN;
                (0..flips).fold(h, |acc, _| {
                    acc ^ (1u64 << (next_u64(noise_state) % HASH_BITS_U64))
                })
            })
            .collect()
    }

    /// All phases `p = 0..K` of a 430-frame noisy query must hit the exact frame.
    #[test]
    fn test_coarse_to_fine_all_phases_exact_frame_hits() {
        let dense = make_catalog();
        let index = CoarseToFineAHashIndex::build(&dense, TEST_FACTOR_K);
        let reference = dense.entries[TEST_TARGET_MEDIA].hashes.clone();
        let target_name = dense.entries[TEST_TARGET_MEDIA].media_name.clone();
        let mut noise_state = TEST_NOISE_SEED;

        let mut hits = 0usize;
        let mut total_search_ms = 0.0f64;
        for phase in 0..TEST_FACTOR_K {
            let start = TEST_BASE_OFFSET + phase;
            assert_eq!(start % TEST_FACTOR_K, phase);
            let query = noisy_window(&reference, start, &mut noise_state);
            let criteria = FineSearchCriteria {
                dense_query: &query,
                dense_fps: TEST_DENSE_FPS,
                max_distance: DEFAULT_FINE_MAX_DISTANCE,
            };
            let t0 = Instant::now();
            let found = index.search(&dense, &criteria);
            let ms = t0.elapsed().as_secs_f64() * MILLIS_PER_SEC;
            total_search_ms += ms;
            match found {
                Some(m) if m.media_name == target_name && m.ref_frame_offset == start => {
                    hits += 1;
                }
                other => println!("phase {phase:>2}: MISS (start={start}) -> {other:?}"),
            }
            println!("phase {phase:>2}: start={start} search={ms:.3} ms");
        }
        println!(
            "exact hits: {hits}/{TEST_FACTOR_K} | mean search: {:.3} ms",
            total_search_ms / TEST_FACTOR_K as f64
        );
        assert_eq!(hits, TEST_FACTOR_K, "every phase must hit the exact frame");
    }

    /// Ranking is `votes DESC`, `m_idx ASC`, `approx_dense_start ASC`, order-independent.
    #[test]
    fn test_rank_clusters_is_deterministic_and_ordered() {
        let far = COARSE_SUPPRESSION_RADIUS_FRAMES * 10;
        let mut hits = vec![(2, far), (1, far), (1, far), (0, far * 2), (0, far * 2)];
        let ranked_a = rank_clusters(hits.clone());
        hits.reverse();
        let ranked_b = rank_clusters(hits);
        assert_eq!(ranked_a, ranked_b);

        let order: Vec<(usize, u32)> = ranked_a.iter().map(|c| (c.m_idx, c.votes)).collect();
        assert_eq!(order, vec![(0, 2), (1, 2), (2, 1)]);
    }

    /// Candidate list never exceeds the verification budget.
    #[test]
    fn test_rank_clusters_truncates_to_budget() {
        let far = COARSE_SUPPRESSION_RADIUS_FRAMES * 10;
        let hits: Vec<(usize, usize)> = (0..MAX_COARSE_CANDIDATES_TO_VERIFY * 2)
            .map(|i| (i, far))
            .collect();
        assert_eq!(rank_clusters(hits).len(), MAX_COARSE_CANDIDATES_TO_VERIFY);
    }
}
