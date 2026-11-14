//! Hierarchical Coarse-to-Fine index for acoustic fingerprints.
//!
//! Sub-samples 86.125 fps (5512.5 / 64) acoustic streams by factor $K$ to create a compact,
//! low-RAM primary filter, followed by localized fine dense correlation.
//!
//! The catalog side is decimated by $K$ (RAM footprint $1/K$) while the query side
//! is **never** decimated: every dense query frame probes the coarse MIH-32 table, so
//! that for any query phase $p = R_0 \bmod K$ about $|Q|/K$ probes land exactly on a
//! coarse anchor $0, K, 2K, \dots$ (zero temporal delta).
//!
//! Hot-loop candidate voting, bucket accumulation, and cluster ranking execute inside
//! a reusable pre-allocated [`CoarseSearchScratchpad`] (`Zero Dynamic Heap Allocation`).

use crate::ahash::{
    lookup_mih32_slice, AHashCatalog, AHashCatalogEntry, AHashCatalogMatch, AHashMih32Index,
    AHashSearchCriteria, Mih32Entry, FPS_COMPATIBILITY_TOLERANCE, MILLIS_PER_SEC, MIN_QUERY_FRAMES,
};
use dbsrt_core::ahash::is_silent_or_flat_audio_query;
use rayon::prelude::*;
use std::cell::RefCell;
use std::sync::Arc;

/// Default fine refinement radius in dense frames around `approx_dense_start` (`~185.8 ms` at `86.125 fps`).
///
/// Since every dense query phase `0..K` probes `MIH-32`, `approx_dense_start = ref_coarse_idx * K - q_idx`
/// already pinpoints the true dense frame within $\pm 4$ frames; $\pm 16$ frames provides a $4\times$ safety margin.
pub const DEFAULT_FINE_WINDOW_RADIUS_FRAMES: usize = 16;

/// Nominal Haitsma-Kalker dense sub-fingerprint rate (`5512.5 Hz / 64 = 86.1328125` or `5512 / 64 = 86.125 fps`).
pub const NOMINAL_AHASH_FPS: f64 = 86.125;

/// Default radius in seconds around coarse offset for localized dense search (`~0.1858 s`).
pub const DEFAULT_FINE_WINDOW_RADIUS_SEC: f64 =
    (DEFAULT_FINE_WINDOW_RADIUS_FRAMES as f64) / NOMINAL_AHASH_FPS;

/// Default downsampling factor $K$ for acoustic coarse-to-fine index.
pub const DEFAULT_COARSE_FACTOR_K: usize = 16;

/// Minimum multiplier of `factor_k` required for a query to span coarse anchors;
/// shorter queries (`< 2 * factor_k`) bypass the coarse stage and query the dense catalog directly.
pub const MIN_COARSE_ANCHOR_SPAN_MULTIPLIER: usize = 2;

/// Minimum multiplier of `factor_k` (`>= 4 * factor_k` frames) at which primary candidate
/// filtering requires at least [`PRIMARY_COARSE_CLUSTER_VOTES`] (`>= 2`) before controlled fallback.
pub const MULTI_VOTE_QUERY_SPAN_MULTIPLIER: usize = 4;

/// Default Hamming distance threshold for fine search validation.
pub const DEFAULT_FINE_MAX_DISTANCE: f64 = 14.0;

/// Maximum candidate bucket size for MIH-32 slice lookup before skipping low-frequency hum / silence collisions.
pub const MAX_MIH_SLICE_CANDIDATES: usize = 1_024;

/// Number of top-ranked coarse clusters verified by the fine dense stage.
pub const MAX_COARSE_CANDIDATES_TO_VERIFY: usize = 8;

/// Maximum number of single-vote clusters checked as controlled fallback when `>= 2`-vote clusters fail to certify.
pub const MAX_SINGLE_VOTE_FALLBACK_CANDIDATES: usize = 2;

/// Width (in dense frames) of one jitter bucket used to group offset votes.
///
/// Adjacent buckets `b - 1, b, b + 1` are summed so that $\pm 1$ frame of encoder
/// jitter cannot split a true cluster across a bucket boundary.
pub const COARSE_JITTER_BUCKET_FRAMES: usize = 4;

/// Primary minimum summed votes required when the query spans at least `4 * factor_k` frames.
pub const PRIMARY_COARSE_CLUSTER_VOTES: u32 = 2;

/// Absolute minimum summed votes for a coarse cluster to be kept as a candidate.
pub const MIN_COARSE_CLUSTER_VOTES: u32 = 1;

/// Two clusters of the same media closer than this (dense frames) are merged via non-maximum suppression.
pub const COARSE_SUPPRESSION_RADIUS_FRAMES: usize = 2 * COARSE_JITTER_BUCKET_FRAMES;

/// Initial pre-allocated capacity for raw packed `u64` hit votes in [`CoarseSearchScratchpad`].
pub const SCRATCH_HITS_CAPACITY: usize = 16_384;

/// Initial pre-allocated capacity for run-length jitter buckets in [`CoarseSearchScratchpad`].
pub const SCRATCH_BUCKETS_CAPACITY: usize = 4_096;

/// Initial pre-allocated capacity for candidate cluster pools in [`CoarseSearchScratchpad`].
pub const SCRATCH_CLUSTERS_CAPACITY: usize = 512;

/// Unroll block size (in `u64` frames = 64-byte cache line) for inner XOR+POPCNT loop with early exit.
const POPCNT_CHUNK_FRAMES: usize = 8;

/// Safety margin (in bits) added to `ceil(max_distance * q_len)` for lossless integer early-exit cutoff.
const DISTANCE_CUTOFF_SAFETY_BITS: u64 = 2;

/// Number of bits in one MIH half-chunk (upper / lower 32 bits of a 64-bit hash).
const MIH_CHUNK_BITS: u32 = 32;

/// Bit mask selecting the lower 32-bit chunk of a 64-bit hash.
const MIH_LOWER_CHUNK_MASK: u64 = 0xFFFF_FFFF;

/// Bit shift packing `m_idx` into the upper 32 bits of a packed `u64` key.
const PACKED_MEDIA_SHIFT: u32 = 32;

/// Bit mask extracting the lower 32-bit frame or bucket offset from a packed `u64` key.
const PACKED_OFFSET_MASK: u64 = 0xFFFF_FFFF;

/// Reference noise floor (random 64-bit hashes differ on 32 bits on average).
const RANDOM_HASH_DISTANCE_BITS: f64 = 32.0;

/// Number of bits in one acoustic sub-fingerprint (normalizes the match score).
const HASH_BITS: f64 = 64.0;

thread_local! {
    /// Per-thread pre-allocated scratchpad ensuring zero dynamic heap allocations across hot-loop searches.
    static TLS_COARSE_SCRATCHPAD: RefCell<CoarseSearchScratchpad> =
        RefCell::new(CoarseSearchScratchpad::new());
}

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

/// Candidate reference location produced by the coarse stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoarseCluster {
    /// Summed votes over the jitter bucket and its two neighbours.
    pub votes: u32,
    /// Index of the media in the coarse (and dense) catalog.
    pub m_idx: usize,
    /// Estimated dense-frame start of the query inside the reference media.
    pub approx_dense_start: usize,
}

/// Per-bucket vote accumulator stored contiguously in [`CoarseSearchScratchpad`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BucketAcc {
    /// Total votes falling in this bucket.
    pub votes: u32,
    /// Exact dense start holding the most votes inside this bucket.
    pub peak_start: u32,
    /// Votes received by `peak_start`.
    pub peak_count: u32,
}

/// Pre-allocated reusable arena eliminating heap allocations (`Vec::new`, `BTreeMap`) per search query.
///
/// # Data Flow & Invariants
/// All internal buffers are allocated once at construction and reset via [`Self::clear`] (`len = 0`,
/// capacity retained) before each Coarse-to-Fine search invocation.
#[derive(Debug, Clone)]
pub struct CoarseSearchScratchpad {
    /// Packed `(m_idx << 32) | approx_dense_start` raw MIH-32 votes.
    pub(crate) hits: Vec<u64>,
    /// Strictly ascending `(packed_media_bucket, BucketAcc)` run-length accumulators.
    pub(crate) buckets: Vec<(u64, BucketAcc)>,
    /// Candidate clusters meeting the primary vote threshold.
    pub(crate) primary_clusters: Vec<CoarseCluster>,
    /// Single-vote candidate clusters held in reserve for controlled fallback.
    pub(crate) fallback_clusters: Vec<CoarseCluster>,
    /// Post-NMS top candidate clusters to verify in the fine dense pass.
    pub(crate) kept: Vec<CoarseCluster>,
    /// Post-NMS fallback clusters when primary verification yields no match.
    pub(crate) kept_fallback: Vec<CoarseCluster>,
}

impl Default for CoarseSearchScratchpad {
    fn default() -> Self {
        Self::new()
    }
}

impl CoarseSearchScratchpad {
    /// Allocates a new reusable search scratchpad with standard capacities.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// Pre-allocated [`CoarseSearchScratchpad`].
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Reserves [`SCRATCH_HITS_CAPACITY`], [`SCRATCH_BUCKETS_CAPACITY`], and [`SCRATCH_CLUSTERS_CAPACITY`] upfront.
    pub fn new() -> Self {
        Self {
            hits: Vec::with_capacity(SCRATCH_HITS_CAPACITY),
            buckets: Vec::with_capacity(SCRATCH_BUCKETS_CAPACITY),
            primary_clusters: Vec::with_capacity(SCRATCH_CLUSTERS_CAPACITY),
            fallback_clusters: Vec::with_capacity(SCRATCH_CLUSTERS_CAPACITY),
            kept: Vec::with_capacity(MAX_COARSE_CANDIDATES_TO_VERIFY),
            kept_fallback: Vec::with_capacity(MAX_SINGLE_VOTE_FALLBACK_CANDIDATES),
        }
    }

    /// Resets all scratchpad vectors in $\mathcal{O}(1)$ while retaining allocated heap capacities.
    ///
    /// # Arguments
    /// None (mutates `&mut self`).
    ///
    /// # Returns
    /// Nothing (`()`).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Sets all vector lengths to `0` without freeing backing allocations.
    #[inline]
    pub fn clear(&mut self) {
        self.hits.clear();
        self.buckets.clear();
        self.primary_clusters.clear();
        self.fallback_clusters.clear();
        self.kept.clear();
        self.kept_fallback.clear();
    }
}

/// Borrowed view of one reference audio track's dense hashes and framerate (heap or `mmap`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FineTrackView<'a> {
    /// Contiguous 64-bit dense sub-fingerprint slice (`86.125 fps`).
    pub hashes: &'a [u64],
    /// Reference track sampling framerate in Hz.
    pub audio_fps: f64,
}

/// Stack-allocated fine-window correlation result (avoids `String` cloning during candidate tournament).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FineWindowWin {
    /// Winning shift index inside the reference track's dense hash slice.
    pub best_shift: usize,
    /// Mean Hamming distance in bits (`<= criteria.max_distance`).
    pub mean_distance: f64,
    /// Normalized similarity score in `[0.0, 1.0]`.
    pub score: f64,
    /// Signal-to-noise margin above `32.0` random bits.
    pub snr_bits: f64,
    /// Reference offset in seconds.
    pub offset_sec: f64,
    /// Rounded reference offset in milliseconds.
    pub offset_ms: i64,
    /// Reference track framerate in Hz.
    pub audio_fps: f64,
}

/// Context borrowing sorted `MIH-32` coarse tables and prefix offsets for zero-alloc vote probing.
#[derive(Clone, Copy)]
pub(crate) struct CoarseProbeTables<'a> {
    /// Sorted upper 32-bit chunk table.
    pub upper: &'a [Mih32Entry],
    /// Sorted lower 32-bit chunk table.
    pub lower: &'a [Mih32Entry],
    /// Downsampling stride $K$.
    pub factor_k: usize,
}

/// Packs `(m_idx, offset)` into a single 64-bit integer preserving lexicographic sort order.
#[inline(always)]
pub(crate) fn pack_media_offset(m_idx: u32, offset: u32) -> u64 {
    ((m_idx as u64) << PACKED_MEDIA_SHIFT) | (offset as u64)
}

/// Unpacks `(m_idx, offset)` from a 64-bit key created by [`pack_media_offset`].
#[inline(always)]
pub(crate) fn unpack_media_offset(packed: u64) -> (usize, usize) {
    (
        (packed >> PACKED_MEDIA_SHIFT) as usize,
        (packed & PACKED_OFFSET_MASK) as usize,
    )
}

/// Executes `f` with a mutable borrow of the calling thread's reusable [`CoarseSearchScratchpad`].
pub(crate) fn with_tls_scratchpad<R, F>(f: F) -> R
where
    F: FnOnce(&mut CoarseSearchScratchpad) -> R,
{
    TLS_COARSE_SCRATCHPAD.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            guard.clear();
            f(&mut guard)
        } else {
            let mut fallback = CoarseSearchScratchpad::new();
            f(&mut fallback)
        }
    })
}

/// Probes both 32-bit `MIH-32` tables for every dense query frame and appends packed votes into `hits`.
///
/// # Arguments
/// * `tables` - Borrowed upper/lower `MIH-32` slices and `factor_k`.
/// * `dense_query` - Undecimated 64-bit query hashes.
/// * `resolve_coarse` - Closure mapping a global coarse `frame_idx` to `(media_idx, local_coarse_offset)`.
/// * `hits` - Pre-allocated output buffer of packed `(m_idx << 32) | approx_dense_start` keys.
///
/// # Returns
/// Nothing (`()`; writes directly into `hits` with zero heap allocation).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Skips buckets exceeding [`MAX_MIH_SLICE_CANDIDATES`] and negative starts (`coarse_idx * K < q_idx`).
pub(crate) fn probe_coarse_mih32_votes<F>(
    tables: CoarseProbeTables<'_>,
    dense_query: &[u64],
    resolve_coarse: F,
    hits: &mut Vec<u64>,
) where
    F: Fn(usize) -> (usize, usize),
{
    for (q_idx, &q_hash) in dense_query.iter().enumerate() {
        let upper = (q_hash >> MIH_CHUNK_BITS) as u32;
        let lower = (q_hash & MIH_LOWER_CHUNK_MASK) as u32;
        append_mih32_slice_votes(
            lookup_mih32_slice(tables.upper, upper),
            (q_idx, tables.factor_k),
            &resolve_coarse,
            hits,
        );
        append_mih32_slice_votes(
            lookup_mih32_slice(tables.lower, lower),
            (q_idx, tables.factor_k),
            &resolve_coarse,
            hits,
        );
    }
}

/// Appends packed `(m_idx, approx_dense_start)` votes from one `MIH-32` bucket slice.
#[inline]
fn append_mih32_slice_votes<F>(
    slice: &[Mih32Entry],
    query_pos: (usize, usize),
    resolve_coarse: &F,
    hits: &mut Vec<u64>,
) where
    F: Fn(usize) -> (usize, usize),
{
    if slice.is_empty() || slice.len() > MAX_MIH_SLICE_CANDIDATES {
        return;
    }
    let (q_idx, factor_k) = query_pos;
    for entry in slice {
        let (m_idx, coarse_idx) = resolve_coarse(entry.frame_idx as usize);
        let Some(start) = coarse_idx
            .saturating_mul(factor_k)
            .checked_sub(q_idx)
            .filter(|&s| s <= u32::MAX as usize && m_idx <= u32::MAX as usize)
        else {
            continue;
        };
        hits.push(pack_media_offset(m_idx as u32, start as u32));
    }
}

/// Merges a contiguous run of `(bucket_key, start_u32, count)` into `buckets` in $\mathcal{O}(1)$.
#[inline]
fn push_or_merge_bucket_run(
    buckets: &mut Vec<(u64, BucketAcc)>,
    bucket_key: u64,
    start_u32: u32,
    count: u32,
) {
    if let Some((_, last_acc)) = buckets.last_mut().filter(|(k, _)| *k == bucket_key) {
        last_acc.votes += count;
        if count > last_acc.peak_count {
            last_acc.peak_count = count;
            last_acc.peak_start = start_u32;
        }
        return;
    }
    buckets.push((
        bucket_key,
        BucketAcc {
            votes: count,
            peak_start: start_u32,
            peak_count: count,
        },
    ));
}

/// Run-length encodes sorted `hits` into strictly ascending `(packed_media_bucket, BucketAcc)` entries.
///
/// # Data Flow & Invariants
/// Because `hits` is sorted ascending by `(m_idx, start)`, `(m_idx, start / COARSE_JITTER_BUCKET_FRAMES)`
/// is monotonically non-decreasing, allowing $\mathcal{O}(H)$ single-pass accumulation without `BTreeMap`.
fn accumulate_sorted_hits(hits: &[u64], buckets: &mut Vec<(u64, BucketAcc)>) {
    buckets.clear();
    let mut idx = 0;
    while idx < hits.len() {
        let packed = hits[idx];
        let mut run_end = idx + 1;
        while run_end < hits.len() && hits[run_end] == packed {
            run_end += 1;
        }
        let count = (run_end - idx) as u32;
        let (m_idx, start) = unpack_media_offset(packed);
        let bucket_idx = (start / COARSE_JITTER_BUCKET_FRAMES) as u32;
        let bucket_key = pack_media_offset(m_idx as u32, bucket_idx);
        push_or_merge_bucket_run(buckets, bucket_key, start as u32, count);
        idx = run_end;
    }
}

/// Sums votes over adjacent jitter buckets `b - 1, b, b + 1` of the same media in $\mathcal{O}(1)$.
///
/// # Data Flow & Invariants
/// - Since `buckets` is strictly increasing by `(m_idx << 32) | b`, `(m_idx, b - 1)` can only reside
///   at `idx - 1` (when `b > 0`) and `(m_idx, b + 1)` can only reside at `idx + 1` (when `b < u32::MAX`).
/// - **Invariant (ADR-0007)**: `(b as u32) < u32::MAX` guards against cross-media wrap-around where
///   `((m << 32) | u32::MAX) + 1 == ((m + 1) << 32) | 0`.
#[inline]
fn eval_adjacent_bucket_cluster(buckets: &[(u64, BucketAcc)], idx: usize) -> CoarseCluster {
    let (key, center) = buckets[idx];
    let (m_idx, b) = unpack_media_offset(key);
    let mut votes = 0u32;
    let mut peak_count = 0u32;
    let mut peak_start = 0u32;

    if b > 0 && idx > 0 && buckets[idx - 1].0 == key - 1 {
        let left = buckets[idx - 1].1;
        votes += left.votes;
        peak_count = left.peak_count;
        peak_start = left.peak_start;
    }

    votes += center.votes;
    if center.peak_count > peak_count {
        peak_count = center.peak_count;
        peak_start = center.peak_start;
    }

    if (b as u32) < u32::MAX && idx + 1 < buckets.len() && buckets[idx + 1].0 == key + 1 {
        let right = buckets[idx + 1].1;
        votes += right.votes;
        if right.peak_count > peak_count {
            peak_start = right.peak_start;
        }
    }

    CoarseCluster {
        votes,
        m_idx,
        approx_dense_start: peak_start as usize,
    }
}

/// Applies greedy non-maximum suppression on deterministically sorted clusters up to `budget`.
fn suppress_into_budget(sorted: &[CoarseCluster], budget: usize, out: &mut Vec<CoarseCluster>) {
    out.clear();
    for &cluster in sorted {
        if out.len() >= budget {
            break;
        }
        let duplicate = out.iter().any(|k| {
            k.m_idx == cluster.m_idx
                && k.approx_dense_start.abs_diff(cluster.approx_dense_start)
                    <= COARSE_SUPPRESSION_RADIUS_FRAMES
        });
        if !duplicate {
            out.push(cluster);
        }
    }
}

/// Deterministically sorts raw hits, accumulates jitter buckets, and populates primary & fallback clusters.
///
/// # Arguments
/// * `scratch` - Pre-allocated [`CoarseSearchScratchpad`] holding `hits`.
/// * `primary_min_votes` - Minimum votes required for the primary verification pass (`1` or `2`).
///
/// # Returns
/// Nothing (`()`; populates `scratch.kept` and `scratch.fallback_clusters`).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. Sorts `scratch.hits` ascending (eliminates thread/bucket ordering variance).
/// 2. Accumulates adjacent jitter buckets in $\mathcal{O}(H)$ without heap allocation.
/// 3. Sorts primary clusters by `(Reverse(votes), m_idx, approx_dense_start)` and applies NMS.
pub(crate) fn rank_hits_in_scratchpad(
    scratch: &mut CoarseSearchScratchpad,
    primary_min_votes: u32,
) {
    scratch.primary_clusters.clear();
    scratch.fallback_clusters.clear();
    scratch.kept.clear();
    scratch.kept_fallback.clear();

    if scratch.hits.is_empty() {
        return;
    }

    scratch.hits.sort_unstable();
    accumulate_sorted_hits(&scratch.hits, &mut scratch.buckets);

    for idx in 0..scratch.buckets.len() {
        let cluster = eval_adjacent_bucket_cluster(&scratch.buckets, idx);
        if cluster.votes >= primary_min_votes {
            scratch.primary_clusters.push(cluster);
        } else if cluster.votes >= MIN_COARSE_CLUSTER_VOTES {
            scratch.fallback_clusters.push(cluster);
        }
    }

    scratch
        .primary_clusters
        .sort_unstable_by_key(|c| (std::cmp::Reverse(c.votes), c.m_idx, c.approx_dense_start));
    suppress_into_budget(
        &scratch.primary_clusters,
        MAX_COARSE_CANDIDATES_TO_VERIFY,
        &mut scratch.kept,
    );
}

/// Lazily sorts and suppresses single-vote fallback clusters when the primary pass yields no match.
pub(crate) fn prepare_fallback_clusters(scratch: &mut CoarseSearchScratchpad) {
    scratch.kept_fallback.clear();
    if scratch.fallback_clusters.is_empty() {
        return;
    }
    scratch
        .fallback_clusters
        .sort_unstable_by_key(|c| (std::cmp::Reverse(c.votes), c.m_idx, c.approx_dense_start));
    suppress_into_budget(
        &scratch.fallback_clusters,
        MAX_SINGLE_VOTE_FALLBACK_CANDIDATES,
        &mut scratch.kept_fallback,
    );
}

/// Computes exact Hamming bit distance when `<= max_inclusive`, or `None` via lossless early exit.
#[inline]
fn bounded_hamming_sum(query: &[u64], reference: &[u64], max_inclusive: u64) -> Option<u64> {
    let mut dist = 0u64;
    let (q_chunks, q_rem) = query.as_chunks::<POPCNT_CHUNK_FRAMES>();
    let (r_chunks, r_rem) = reference.as_chunks::<POPCNT_CHUNK_FRAMES>();

    for (qc, rc) in q_chunks.iter().zip(r_chunks) {
        let mut block = 0u32;
        for i in 0..POPCNT_CHUNK_FRAMES {
            block += (qc[i] ^ rc[i]).count_ones();
        }
        dist += block as u64;
        if dist > max_inclusive {
            return None;
        }
    }

    for (&q, &r) in q_rem.iter().zip(r_rem) {
        dist += (q ^ r).count_ones() as u64;
    }
    (dist <= max_inclusive).then_some(dist)
}

/// Evaluates localized fine sliding cross-correlation over a dense `u64` slice (heap or `mmap`).
///
/// # Arguments
/// * `track` - Borrowed [`FineTrackView`] (`hashes` slice + `audio_fps`).
/// * `criteria` - Search parameters (`dense_query`, `dense_fps`, `max_distance`).
/// * `approx_offset` - Coarse-stage estimated start offset in dense frames.
/// * `radius_frames` - Half-window search radius in dense frames (default: `16`).
///
/// # Returns
/// `Some(FineWindowWin)` if the sliding minimum satisfies `mean_distance <= criteria.max_distance`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// - Evaluates shifts in `[approx_offset - radius_frames ..= approx_offset + radius_frames]` using
///   cache-line chunked `POPCNT` summation with lossless monotonic early exit.
/// - **Invariant (ADR-0007)**: [`bounded_hamming_sum`] returns `Option<u64>` (`None` on cutoff) and uses
///   asymmetric `best_dist` (`s < best_shift`) vs `best_dist - 1` (`s > best_shift`, guarded for `0`) so
///   truncated partial sums can never corrupt deterministic left tie-breaks.
pub(crate) fn evaluate_fine_slice(
    track: FineTrackView<'_>,
    criteria: &FineSearchCriteria<'_>,
    approx_offset: usize,
    radius_frames: usize,
) -> Option<FineWindowWin> {
    if criteria.max_distance.is_nan()
        || !track.audio_fps.is_finite()
        || track.audio_fps <= 0.0
        || (track.audio_fps - criteria.dense_fps).abs() > FPS_COMPATIBILITY_TOLERANCE
    {
        return None;
    }

    let q_len = criteria.dense_query.len();
    if q_len == 0 || track.hashes.len() < q_len {
        return None;
    }

    let min_s = approx_offset.saturating_sub(radius_frames);
    let max_s = approx_offset
        .saturating_add(radius_frames)
        .min(track.hashes.len() - q_len);
    if min_s > max_s {
        return None;
    }

    let max_valid_bits = ((criteria.max_distance.max(0.0) * q_len as f64).ceil() as u64)
        .saturating_add(DISTANCE_CUTOFF_SAFETY_BITS);
    let mut best_shift = min_s;
    let mut best_dist = u64::MAX;

    // Probe center estimate first so `best_dist` immediately tightens the early-exit cutoff.
    let center = approx_offset.clamp(min_s, max_s);
    if let Some(d) = bounded_hamming_sum(
        criteria.dense_query,
        &track.hashes[center..center + q_len],
        max_valid_bits,
    ) {
        best_dist = d;
        best_shift = center;
    }

    for s in min_s..=max_s {
        if s == center {
            continue;
        }
        let max_allowed = if best_dist == u64::MAX {
            max_valid_bits
        } else if s < best_shift {
            best_dist.min(max_valid_bits)
        } else if best_dist == 0 {
            continue;
        } else {
            (best_dist - 1).min(max_valid_bits)
        };
        if let Some(dist) = bounded_hamming_sum(
            criteria.dense_query,
            &track.hashes[s..s + q_len],
            max_allowed,
        ) {
            best_dist = dist;
            best_shift = s;
        }
    }

    if best_dist == u64::MAX {
        return None;
    }

    let mean_distance = best_dist as f64 / q_len as f64;
    if mean_distance > criteria.max_distance {
        return None;
    }

    let score = (1.0 - (mean_distance / HASH_BITS)).clamp(0.0, 1.0);
    let snr_bits = (RANDOM_HASH_DISTANCE_BITS - mean_distance).max(0.0);
    let offset_sec = best_shift as f64 / track.audio_fps;

    Some(FineWindowWin {
        best_shift,
        mean_distance,
        score,
        snr_bits,
        offset_sec,
        offset_ms: (offset_sec * MILLIS_PER_SEC).round() as i64,
        audio_fps: track.audio_fps,
    })
}

/// Materializes an owned [`AHashCatalogMatch`] once for a winning [`FineWindowWin`].
#[inline]
pub(crate) fn into_catalog_match(
    media_name: String,
    file_path: String,
    win: FineWindowWin,
) -> AHashCatalogMatch {
    AHashCatalogMatch {
        media_name,
        file_path,
        ref_frame_offset: win.best_shift,
        offset_sec: win.offset_sec,
        offset_ms: win.offset_ms,
        mean_distance: win.mean_distance,
        score: win.score,
        snr_bits: win.snr_bits,
        audio_fps: win.audio_fps,
    }
}

/// Determines the primary coarse vote threshold for a query of `query_len` frames at `factor_k`.
#[inline]
pub(crate) fn primary_vote_threshold(query_len: usize, factor_k: usize) -> u32 {
    if query_len >= factor_k.saturating_mul(MULTI_VOTE_QUERY_SPAN_MULTIPLIER) {
        PRIMARY_COARSE_CLUSTER_VOTES
    } else {
        MIN_COARSE_CLUSTER_VOTES
    }
}

/// Hierarchical Coarse-to-Fine index for acoustic fingerprints.
pub struct CoarseToFineAHashIndex {
    /// Downsample factor K (e.g. 16 for 5.38 fps or 32 for 2.69 fps coarse tier from 86.125 fps dense).
    pub factor_k: usize,
    /// Sub-sampled coarse catalog holding 1/K-th of the frames.
    pub coarse_catalog: AHashCatalog,
    /// 32-bit Multi-Index Hash table built over the coarse flat hashes.
    pub coarse_mih32: AHashMih32Index,
    /// Half-window radius in dense frames evaluated during the fine correlation pass.
    pub fine_window_radius_frames: usize,
}

impl CoarseToFineAHashIndex {
    /// Builds a coarse-to-fine index from a dense catalog by sub-sampling every K-th frame.
    ///
    /// # Arguments
    /// * `dense_catalog` - Reference to loaded dense [`AHashCatalog`].
    /// * `factor_k` - Sub-sampling stride (must be $\ge 1$).
    ///
    /// # Returns
    /// Initialized [`CoarseToFineAHashIndex`] with default fine window radius (`16` frames).
    ///
    /// # Panics
    /// Panics if `factor_k` is 0.
    ///
    /// # Data Flow & Invariants
    /// 1. Par-iterates over catalog entries to downsample hashes by stride $K$.
    /// 2. Assembles compact coarse catalog ($1/K$ RAM footprint).
    /// 3. Coarse frame `c` of a media always corresponds to dense frame `c * K`.
    /// 4. Builds 32-bit multi-index hash table on coarse flat hashes for sub-millisecond lookup.
    pub fn build(dense_catalog: &AHashCatalog, factor_k: usize) -> Self {
        assert!(factor_k >= 1, "Downsample factor must be >= 1");

        let coarse_catalog = if factor_k == 1 {
            dense_catalog.clone()
        } else {
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
                        audio_language: entry.audio_language.clone(),
                        audio_index: entry.audio_index,
                    }
                })
                .collect();
            AHashCatalog::new(coarse_entries)
        };
        let coarse_mih32 = AHashMih32Index::build(&coarse_catalog.flat_hashes);

        Self {
            factor_k,
            coarse_catalog,
            coarse_mih32,
            fine_window_radius_frames: DEFAULT_FINE_WINDOW_RADIUS_FRAMES,
        }
    }

    /// Overrides the fine dense verification radius in frames.
    ///
    /// # Arguments
    /// * `radius_frames` - Half-window search radius in dense frames (`>= 1`).
    ///
    /// # Returns
    /// Updated [`CoarseToFineAHashIndex`].
    ///
    /// # Panics
    /// Never panics (clamps `0` to `1`).
    ///
    /// # Data Flow & Invariants
    /// Controls the `[approx - radius ..= approx + radius]` dense correlation span.
    pub fn with_fine_window_radius_frames(mut self, radius_frames: usize) -> Self {
        self.fine_window_radius_frames = radius_frames.max(1);
        self
    }

    /// Performs two-tier coarse-to-fine acoustic search using the thread-local zero-alloc scratchpad.
    ///
    /// # Arguments
    /// * `dense_catalog` - Reference to the full-resolution dense [`AHashCatalog`].
    /// * `criteria` - Search criteria context ([`FineSearchCriteria`]).
    ///
    /// # Returns
    /// `Option<AHashCatalogMatch>` containing the certified fine match with the lowest
    /// `mean_distance <= criteria.max_distance`, or `None`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Delegates to [`Self::search_with_scratchpad`] using a thread-local reusable arena.
    pub fn search(
        &self,
        dense_catalog: &AHashCatalog,
        criteria: &FineSearchCriteria<'_>,
    ) -> Option<AHashCatalogMatch> {
        with_tls_scratchpad(|scratch| self.search_with_scratchpad(dense_catalog, criteria, scratch))
    }

    /// Performs two-tier coarse-to-fine acoustic search using an explicit reusable [`CoarseSearchScratchpad`].
    ///
    /// # Arguments
    /// * `dense_catalog` - Reference to the full-resolution dense [`AHashCatalog`].
    /// * `criteria` - Search criteria context ([`FineSearchCriteria`]).
    /// * `scratch` - Reusable pre-allocated scratchpad (`Zero Dynamic Heap Allocation` in hot loop).
    ///
    /// # Returns
    /// `Option<AHashCatalogMatch>` containing the certified fine match with the lowest
    /// `mean_distance <= criteria.max_distance`, or `None`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Rejects queries shorter than [`MIN_QUERY_FRAMES`], silent/flat queries, `NaN` thresholds, or invalid FPS.
    /// 2. Short queries (`< 2 * factor_k`) fall back to [`AHashCatalog::search`] to avoid modulo-$K$ blind spots.
    /// 3. Probes `MIH-32` sequentially into `scratch.hits`, sorts, and ranks jitter-tolerant clusters without `BTreeMap`.
    /// 4. Requires `>= 2` coarse votes when `query.len() >= 4 * factor_k`, falling back to top single-vote clusters
    ///    only if no multi-vote cluster certifies.
    pub fn search_with_scratchpad(
        &self,
        dense_catalog: &AHashCatalog,
        criteria: &FineSearchCriteria<'_>,
        scratch: &mut CoarseSearchScratchpad,
    ) -> Option<AHashCatalogMatch> {
        if criteria.dense_query.len() < MIN_QUERY_FRAMES
            || is_silent_or_flat_audio_query(criteria.dense_query)
            || criteria.max_distance.is_nan()
            || !criteria.dense_fps.is_finite()
            || criteria.dense_fps <= 0.0
        {
            return None;
        }
        if criteria.dense_query.len()
            < self
                .factor_k
                .saturating_mul(MIN_COARSE_ANCHOR_SPAN_MULTIPLIER)
        {
            let dense_criteria = AHashSearchCriteria {
                query: criteria.dense_query,
                audio_fps: criteria.dense_fps,
                max_distance: criteria.max_distance,
                limit: 1,
                target_media: None,
            };
            return dense_catalog.search(&dense_criteria).into_iter().next();
        }

        scratch.clear();
        let tables = CoarseProbeTables {
            upper: &self.coarse_mih32.upper_table,
            lower: &self.coarse_mih32.lower_table,
            factor_k: self.factor_k,
        };
        probe_coarse_mih32_votes(
            tables,
            criteria.dense_query,
            |global_idx| self.coarse_catalog.resolve_global_index(global_idx),
            &mut scratch.hits,
        );

        let min_votes = primary_vote_threshold(criteria.dense_query.len(), self.factor_k);
        rank_hits_in_scratchpad(scratch, min_votes);

        if let Some((entry_idx, win)) =
            self.verify_cluster_slice(dense_catalog, criteria, &scratch.kept)
        {
            let dense_entry = &dense_catalog.entries[entry_idx];
            return Some(into_catalog_match(
                dense_entry.media_name.clone(),
                dense_entry.path.to_string_lossy().to_string(),
                win,
            ));
        }

        if min_votes > MIN_COARSE_CLUSTER_VOTES && !scratch.fallback_clusters.is_empty() {
            prepare_fallback_clusters(scratch);
            if let Some((entry_idx, win)) =
                self.verify_cluster_slice(dense_catalog, criteria, &scratch.kept_fallback)
            {
                let dense_entry = &dense_catalog.entries[entry_idx];
                return Some(into_catalog_match(
                    dense_entry.media_name.clone(),
                    dense_entry.path.to_string_lossy().to_string(),
                    win,
                ));
            }
        }

        None
    }

    /// Verifies ranked clusters against their bijective dense entries without `String` allocations.
    fn verify_cluster_slice(
        &self,
        dense_catalog: &AHashCatalog,
        criteria: &FineSearchCriteria<'_>,
        clusters: &[CoarseCluster],
    ) -> Option<(usize, FineWindowWin)> {
        let mut best: Option<(usize, FineWindowWin)> = None;
        for cluster in clusters {
            let Some(coarse_entry) = self.coarse_catalog.entries.get(cluster.m_idx) else {
                continue;
            };
            let Some((dense_idx, dense_entry)) = dense_catalog
                .entries
                .get(cluster.m_idx)
                .filter(|e| e.path == coarse_entry.path)
                .map(|e| (cluster.m_idx, e))
                .or_else(|| {
                    dense_catalog
                        .entries
                        .iter()
                        .enumerate()
                        .find(|(_, e)| e.path == coarse_entry.path)
                })
            else {
                continue;
            };
            let track = FineTrackView {
                hashes: &dense_entry.hashes,
                audio_fps: dense_entry.audio_fps,
            };
            let Some(win) = evaluate_fine_slice(
                track,
                criteria,
                cluster.approx_dense_start,
                self.fine_window_radius_frames,
            ) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(_, b)| win.mean_distance < b.mean_distance)
            {
                best = Some((dense_idx, win));
            }
        }
        best
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
    /// Dense query window: 5.0 s at 86.125 fps (5512 / 64).
    const TEST_QUERY_FRAMES: usize = 430;
    /// Dense frame rate used by the synthetic catalog (5512 / 64 = 86.125 fps).
    const TEST_DENSE_FPS: f64 = 86.125;
    /// Incompatible frame rate used to verify FPS tolerance rejection.
    const TEST_INCOMPATIBLE_FPS: f64 = 48.0;
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
            audio_language: None,
            audio_index: None,
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

    fn rank_pairs(pairs: &[(usize, usize)], min_votes: u32) -> Vec<CoarseCluster> {
        let mut scratch = CoarseSearchScratchpad::new();
        for &(m_idx, start) in pairs {
            scratch
                .hits
                .push(pack_media_offset(m_idx as u32, start as u32));
        }
        rank_hits_in_scratchpad(&mut scratch, min_votes);
        scratch.kept.clone()
    }

    /// All phases `p = 0..K` of a 430-frame noisy query must hit the exact frame.
    #[test]
    fn test_coarse_to_fine_all_phases_exact_frame_hits() {
        let dense = make_catalog();
        let index = CoarseToFineAHashIndex::build(&dense, TEST_FACTOR_K);
        let reference = dense.entries[TEST_TARGET_MEDIA].hashes.clone();
        let target_name = dense.entries[TEST_TARGET_MEDIA].media_name.clone();
        let mut noise_state = TEST_NOISE_SEED;

        let mut scratch = CoarseSearchScratchpad::new();
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
            let found = index.search_with_scratchpad(&dense, &criteria, &mut scratch);
            let ms = t0.elapsed().as_secs_f64() * MILLIS_PER_SEC;
            total_search_ms += ms;
            match found {
                Some(m) if m.media_name == target_name && m.ref_frame_offset == start => {
                    hits += 1;
                }
                other => println!("phase {phase:>2}: MISS (start={start}) -> {other:?}"),
            }
        }
        println!(
            "exact hits: {hits}/{TEST_FACTOR_K} | mean search: {:.3} ms",
            total_search_ms / TEST_FACTOR_K as f64
        );
        assert_eq!(hits, TEST_FACTOR_K, "every phase must hit the exact frame");
        assert!(
            scratch.hits.capacity() <= SCRATCH_HITS_CAPACITY,
            "scratchpad must not reallocate during normal search"
        );
    }

    /// Ranking is `votes DESC`, `m_idx ASC`, `approx_dense_start ASC`, order-independent.
    #[test]
    fn test_rank_clusters_is_deterministic_and_ordered() {
        let far = COARSE_SUPPRESSION_RADIUS_FRAMES * 10;
        let mut hits = vec![(2, far), (1, far), (1, far), (0, far * 2), (0, far * 2)];
        let ranked_a = rank_pairs(&hits, MIN_COARSE_CLUSTER_VOTES);
        hits.reverse();
        let ranked_b = rank_pairs(&hits, MIN_COARSE_CLUSTER_VOTES);
        assert_eq!(ranked_a, ranked_b);

        let order: Vec<(usize, u32)> = ranked_a.iter().map(|c| (c.m_idx, c.votes)).collect();
        assert_eq!(order, vec![(0, 2), (1, 2), (2, 1)]);
    }

    /// Candidate list never exceeds the verification budget and separates primary vs fallback votes.
    #[test]
    fn test_rank_clusters_truncates_and_partitions_single_vote_fallback() {
        let far = COARSE_SUPPRESSION_RADIUS_FRAMES * 10;
        let hits: Vec<(usize, usize)> = (0..MAX_COARSE_CANDIDATES_TO_VERIFY * 2)
            .map(|i| (i, far))
            .collect();
        assert_eq!(
            rank_pairs(&hits, MIN_COARSE_CLUSTER_VOTES).len(),
            MAX_COARSE_CANDIDATES_TO_VERIFY
        );

        let mut scratch = CoarseSearchScratchpad::new();
        for &(m_idx, start) in &hits {
            scratch
                .hits
                .push(pack_media_offset(m_idx as u32, start as u32));
        }
        rank_hits_in_scratchpad(&mut scratch, PRIMARY_COARSE_CLUSTER_VOTES);
        assert!(scratch.kept.is_empty());
        prepare_fallback_clusters(&mut scratch);
        assert_eq!(
            scratch.kept_fallback.len(),
            MAX_SINGLE_VOTE_FALLBACK_CANDIDATES
        );
    }

    /// Secondary track `.a1.86.1fps.ahash` sharing `media_name` with primary `.86.1fps.ahash`
    /// must achieve 100% recall across all phases via bijective `m_idx` resolution (B-03),
    /// plus short-query dense fallback (m-03) and FPS / NaN guards (M-06).
    #[test]
    fn test_coarse_to_fine_secondary_track_same_media_name_recall_and_guards() {
        let mut primary = make_entry("Shared.Movie.mkv", TEST_CATALOG_SEED + 10);
        primary.path = PathBuf::from("05b6126884056c42.86.1fps.ahash");
        primary.audio_index = Some(0);
        primary.audio_language = Some("fre".to_string());

        let mut secondary = make_entry("Shared.Movie.mkv", TEST_CATALOG_SEED + 20);
        secondary.path = PathBuf::from("05b6126884056c42.a1.86.1fps.ahash");
        secondary.audio_index = Some(1);
        secondary.audio_language = Some("jpn".to_string());

        let dense = AHashCatalog::new(vec![primary, secondary]);
        let index = CoarseToFineAHashIndex::build(&dense, TEST_FACTOR_K);
        let sec_hashes = dense.entries[1].hashes.clone();
        let mut noise_state = TEST_NOISE_SEED ^ 0x1234;

        let mut hits = 0usize;
        for phase in 0..TEST_FACTOR_K {
            let start = TEST_BASE_OFFSET + phase;
            let query = noisy_window(&sec_hashes, start, &mut noise_state);
            let criteria = FineSearchCriteria {
                dense_query: &query,
                dense_fps: TEST_DENSE_FPS,
                max_distance: DEFAULT_FINE_MAX_DISTANCE,
            };
            let found = index
                .search(&dense, &criteria)
                .expect("secondary track must match");
            assert_eq!(found.media_name, "Shared.Movie.mkv");
            assert_eq!(found.file_path, "05b6126884056c42.a1.86.1fps.ahash");
            assert_eq!(found.ref_frame_offset, start);
            hits += 1;
        }
        assert_eq!(
            hits, TEST_FACTOR_K,
            "100% recall required on secondary track .a1"
        );

        // m-03: short query (< 2 * factor_k) falls back to dense catalog search without blind spot
        let short_start = TEST_BASE_OFFSET + 3;
        let short_query = &sec_hashes[short_start..short_start + TEST_FACTOR_K];
        let short_match = index
            .search(
                &dense,
                &FineSearchCriteria {
                    dense_query: short_query,
                    dense_fps: TEST_DENSE_FPS,
                    max_distance: DEFAULT_FINE_MAX_DISTANCE,
                },
            )
            .expect("short query must match via dense fallback");
        assert_eq!(short_match.file_path, "05b6126884056c42.a1.86.1fps.ahash");
        assert_eq!(short_match.ref_frame_offset, short_start);

        // M-06: NaN max_distance and incompatible FPS are rejected
        assert!(index
            .search(
                &dense,
                &FineSearchCriteria {
                    dense_query: short_query,
                    dense_fps: TEST_DENSE_FPS,
                    max_distance: f64::NAN,
                },
            )
            .is_none());
        assert!(index
            .search(
                &dense,
                &FineSearchCriteria {
                    dense_query: &sec_hashes
                        [TEST_BASE_OFFSET..TEST_BASE_OFFSET + TEST_QUERY_FRAMES],
                    dense_fps: TEST_INCOMPATIBLE_FPS,
                    max_distance: DEFAULT_FINE_MAX_DISTANCE,
                },
            )
            .is_none());
    }
}
