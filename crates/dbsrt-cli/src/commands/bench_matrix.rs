//! Exhaustive multi-technique benchmark matrix evaluator for real multi-window media probes.
//!
//! Benchmarks visual (`pHash`: Linear, MIH-16 Mono/Rayon, MIH-32 Mono/Rayon) and acoustic
//! (`aHash`: Coarse-to-Fine $K \in [1, 32]$, MIH-16 Mono/Rayon, MIH-32 Mono/Rayon, Linear)
//! recognition performance and matching quality across thousands of real video multi-window cuts.

use anyhow::{Context, Result};
use dbsrt_core::ahash::{DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE};
use dbsrt_core::multi_window::{
    aggregate_multi_window_matches, MultiWindowConsensus, PerWindowMatches, RawCandidateMatch,
    MAX_SHIFT_DRIFT_TOLERANCE_MS,
};
use dbsrt_core::phash::hamming_distance;
use dbsrt_core::VideoMetadata;
use dbsrt_index::ahash::{
    AHashCatalog, AHashCatalogMatch, AHashSearchCriteria, DEFAULT_AHASH_MIH_JITTER_RADIUS,
    FPS_COMPATIBILITY_TOLERANCE, MIN_QUERY_FRAMES,
};
use dbsrt_index::coarse_to_fine::{CoarseToFineAHashIndex, FineSearchCriteria};
use dbsrt_index::dbphash::{DbphashCutMatch, DbphashMediaMatch, MmapDbphash};
use dbsrt_index::AHashMih32Index;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Instant;

use crate::commands::bench_scale::{
    are_titles_matching, filter_coherent_majority_windows, format_catalog_membership_keys,
    is_catalog_member, is_degenerate_static_phash_window, is_solid_dc_phash_window,
    normalize_phash_shift_ms, resolve_ahash_candidate_id, select_best_consensus,
    MediaScaleBenchmarkRecord, DEFAULT_PHASH_SCALE_MAX_DISTANCE, FINE_SEARCH_MAX_DISTANCE,
    MILLIS_PER_SECOND, PERCENT_SCALE, PHASH_MIN_CONFIDENCE,
};

/// Bytes per mebibyte conversion factor for RAM footprint telemetry.
pub const BYTES_PER_MIB: f64 = 1_048_576.0;

/// Byte width of a single 64-bit fingerprint hash word.
pub const HASH_WORD_BYTES: usize = 8;

/// Byte width of a single `DbphashCutEntry` record (`u64` + `u32` + `u32`).
pub const DBPHASH_ENTRY_BYTES: usize = 16;

/// Byte width of a single `Mih32Entry` record (`u32` + `u32`).
pub const MIH32_ENTRY_BYTES: usize = 8;

/// Number of 32-bit tables in `AHashMih32Index` (upper + lower).
pub const MIH32_TABLE_COUNT: usize = 2;

/// Number of 16-bit tables in `MultiIndexHash64`.
pub const MIH16_TABLE_COUNT: usize = 4;

/// Number of buckets in a 16-bit MIH slot table ($2^{16}$).
pub const MIH16_BUCKET_COUNT: usize = 65_536;

/// Byte overhead of a single 16-bit MIH bucket header (`u32` offset + `u32` len).
pub const MIH16_BUCKET_HEADER_BYTES: usize = 8;

/// Byte width of a single `u32` candidate index in MIH-16 CSR payload.
pub const MIH16_INDEX_BYTES: usize = 4;

/// Coarse-to-Fine downsampling factors $K \in [1, 32]$ evaluated in the acoustic sweep.
pub const COARSE_TO_FINE_SWEEP_FACTORS: [usize; 12] = [1, 2, 3, 4, 6, 8, 12, 16, 20, 24, 28, 32];

/// Maximum number of window candidates retained for multi-window consensus aggregation.
pub const MAX_CONSENSUS_CANDIDATES_PER_WINDOW: usize = 10;

/// Maximum bucket cardinality before skipping generic background hum collisions in MIH lookups.
pub const MAX_MIH_BUCKET_SIZE: usize = 20_000;

/// Query frame stride used by direct MIH-16 / MIH-32 benchmark probes.
pub const DIRECT_MIH_PROBE_STRIDE: usize = 4;

/// Minimum vote cluster threshold for direct MIH-16 / MIH-32 benchmark probes.
pub const DIRECT_MIH_MIN_VOTES: u32 = 3;

/// Maximum number of coarse vote clusters verified in direct MIH-16 / MIH-32 benchmark probes.
pub const DIRECT_MIH_TOP_CLUSTERS: usize = 5;

/// Bit shift to extract upper 32-bit chunk from a 64-bit hash.
pub const UPPER_32_SHIFT: u32 = 32;

/// Bit mask to extract lower 32-bit chunk from a 64-bit hash.
pub const LOWER_32_MASK: u64 = 0xFFFF_FFFF;

/// Total bits in a 64-bit hash.
pub const HASH_BITS_F64: f64 = 64.0;

/// Expected random Hamming distance (in bits) for 64-bit hashes.
pub const RANDOM_DISTANCE_BITS: f64 = 32.0;

/// Sparse score multiplier for aHash SNR bits in multi-window consensus.
pub const AHASH_SPARSE_SNR_SCALE: f64 = 10.0;

/// Subsample limit (in videos) for heavy O(N) linear pHash ground-truth verification.
pub const PHASH_LINEAR_SAMPLE_VIDEOS: usize = 300;

/// Subsample limit (in videos) for heavy Dense (95.3M frames) MIH-16 / MIH-32 direct comparison.
pub const AHASH_DENSE_MIH_SAMPLE_VIDEOS: usize = 60;

/// Subsample limit (in videos) for heavy Dense (95.3M frames) O(N) linear scan comparison.
pub const AHASH_DENSE_LINEAR_SAMPLE_VIDEOS: usize = 12;

/// Tight temporal shift error threshold in milliseconds (~1 audio frame).
pub const SHIFT_TOLERANCE_TIGHT_MS: i64 = 15;

/// Nominal temporal shift error threshold in milliseconds (~2 audio frames).
pub const SHIFT_TOLERANCE_NOMINAL_MS: i64 = 25;

/// Default audio framerate in frames per second (`DEFAULT_SAMPLE_RATE / DEFAULT_HOP_SIZE`).
pub const DEFAULT_AUDIO_FPS_VAL: f64 = (DEFAULT_SAMPLE_RATE as f64) / (DEFAULT_HOP_SIZE as f64);

/// Percentile p50 ratio.
pub const PERCENTILE_P50: f64 = 0.50;

/// Percentile p95 ratio.
pub const PERCENTILE_P95: f64 = 0.95;

/// Percentile p99 ratio.
pub const PERCENTILE_P99: f64 = 0.99;

/// Center window index in a 3-window (`20%, 50%, 80%`) plan.
pub const CENTER_WINDOW_IDX: usize = 1;

/// One-second frame cap used when comparing 1.0s vs full-window pHash slices.
pub const ONE_SECOND_FRAME_CAP: usize = 24;

/// Extra frames required beyond `ONE_SECOND_FRAME_CAP` to trigger the 1.0s vs full-window slice comparison.
pub const EXTENDED_WINDOW_MARGIN_FRAMES: usize = 6;

/// Minimum clean title length before falling back to parent directory tokens.
pub const MIN_CLEAN_TITLE_LEN: usize = 3;

/// Number of windows per video in standard 3-window plan for latency scaling.
pub const WINDOWS_PER_VIDEO_FACTOR: f64 = 3.0;

/// Visual (`pHash`) technique evaluation metrics across real multi-window cuts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhashTechniqueMetrics {
    pub technique: String,
    pub evaluated_videos: usize,
    pub evaluated_windows: usize,
    pub indexed_videos_in_sample: usize,
    pub index_frames: usize,
    pub index_ram_mb: f64,
    pub index_build_ms: f64,
    pub win_latency_mean_ms: f64,
    pub win_latency_p50_ms: f64,
    pub win_latency_p95_ms: f64,
    pub win_latency_p99_ms: f64,
    pub video_3win_latency_mean_ms: f64,
    pub win1_matched: usize,
    pub win1_tp: usize,
    pub win1_fp: usize,
    pub win1_fn: usize,
    pub win1_precision_pct: f64,
    pub win1_recall_pct: f64,
    pub win1_f1_pct: f64,
    pub win3_matched: usize,
    pub win3_coherent: usize,
    pub win3_tp: usize,
    pub win3_fp: usize,
    pub win3_fn: usize,
    pub win3_precision_pct: f64,
    pub win3_recall_pct: f64,
    pub win3_f1_pct: f64,
    pub median_intra_video_shift_spread_ms: i64,
}

/// Acoustic (`aHash`) technique evaluation metrics across real multi-window cuts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AhashTechniqueMetrics {
    pub technique: String,
    pub factor_k: usize,
    pub coarse_fps: f64,
    pub evaluated_videos: usize,
    pub evaluated_windows: usize,
    pub indexed_videos_in_sample: usize,
    pub index_frames: usize,
    pub index_ram_mb: f64,
    pub index_build_ms: f64,
    pub win_latency_mean_ms: f64,
    pub win_latency_p50_ms: f64,
    pub win_latency_p95_ms: f64,
    pub win_latency_p99_ms: f64,
    pub video_3win_latency_mean_ms: f64,
    pub win1_matched: usize,
    pub win1_tp: usize,
    pub win1_fp: usize,
    pub win1_fn: usize,
    pub win1_precision_pct: f64,
    pub win1_recall_indexed_pct: f64,
    pub win1_f1_pct: f64,
    pub win3_matched: usize,
    pub win3_coherent: usize,
    pub win3_tp: usize,
    pub win3_fp: usize,
    pub win3_fn: usize,
    pub win3_precision_pct: f64,
    pub win3_recall_indexed_pct: f64,
    pub win3_f1_pct: f64,
    pub mean_fine_distance_bits: f64,
    pub mean_snr_bits: f64,
    pub shift_drift_vs_k1_mean_ms: f64,
    pub shift_drift_vs_k1_p95_ms: i64,
    pub shift_drift_vs_k1_max_ms: i64,
    pub shift_within_15ms_pct: f64,
    pub shift_within_25ms_pct: f64,
    pub median_intra_video_shift_spread_ms: i64,
}

/// Complete consolidated matrix report persisted to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExhaustiveMatrixReport {
    pub total_real_videos: usize,
    pub total_cut_windows: usize,
    pub synology_videos: usize,
    pub freebox_videos: usize,
    pub indexed_in_phash_videos: usize,
    pub indexed_in_ahash_videos: usize,
    pub phash_techniques: Vec<PhashTechniqueMetrics>,
    pub ahash_coarse_to_fine_sweep: Vec<AhashTechniqueMetrics>,
    pub ahash_mih_comparison: Vec<AhashTechniqueMetrics>,
}

/// Pre-resolved ground-truth descriptor for one real video record.
#[derive(Debug, Clone)]
struct PreparedVideoProbe {
    pub record: MediaScaleBenchmarkRecord,
    pub gt_title: String,
    pub in_phash_catalog: bool,
    pub in_ahash_catalog: bool,
}

/// Internal accumulator for classification and timing counters.
#[derive(Default)]
struct EvalAccumulator {
    pub win_latencies_ms: Vec<f64>,
    pub win1_matched: usize,
    pub win1_tp: usize,
    pub win1_fp: usize,
    pub win3_matched: usize,
    pub win3_coherent: usize,
    pub win3_tp: usize,
    pub win3_fp: usize,
    pub shift_spreads_ms: Vec<i64>,
    pub fine_distances: Vec<f64>,
    pub snr_values: Vec<f64>,
    pub k1_drifts_ms: Vec<i64>,
}

/// Specification context for evaluating a visual `pHash` technique.
struct PhashEvalSpec<'a> {
    pub name: &'a str,
    pub dbphash: &'a MmapDbphash,
    pub index_frames: usize,
    pub index_ram_mb: f64,
    pub index_build_ms: f64,
    pub multi_cand_and_fps_norm: bool,
    pub frame_cap: Option<usize>,
}

/// Specification context for evaluating an acoustic `aHash` technique.
struct AhashEvalSpec<'a> {
    pub name: &'a str,
    pub factor_k: usize,
    pub coarse_fps: f64,
    pub dense_catalog: &'a AHashCatalog,
    pub index_frames: usize,
    pub index_ram_mb: f64,
    pub index_build_ms: f64,
}

/// Context bundling 32-bit MIH tables for visual `pHash` searches.
struct PhashMih32Context<'a> {
    pub mmap: &'a MmapDbphash,
    pub sparse32: &'a AHashMih32Index,
    pub cuts32: &'a AHashMih32Index,
}

/// Context bundling 32-bit MIH index and query parameters for direct `aHash` search.
struct AhashDirectSearchReq<'a> {
    pub catalog: &'a AHashCatalog,
    pub query: &'a [u64],
    pub audio_fps: f64,
    pub parallel: bool,
}

/// Computes a percentile from a slice of `f64` values.
///
/// # Arguments
/// * `values` - Slice of latency observations.
/// * `pct` - Target percentile in `0.0..=1.0`.
///
/// # Returns
/// Percentile value as `f64` (`0.0` if empty).
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Clones and sorts `values` using `total_cmp` before indexing `(len - 1) * pct`.
fn percentile_f64(values: &[f64], pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let idx = ((sorted.len() - 1) as f64 * pct.clamp(0.0, 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Computes a percentile from a slice of `i64` values.
///
/// # Arguments
/// * `values` - Slice of integer drift observations.
/// * `pct` - Target percentile in `0.0..=1.0`.
///
/// # Returns
/// Percentile value as `i64` (`0` if empty).
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Clones and sorts `values` ascending before indexing `(len - 1) * pct`.
fn percentile_i64(values: &[i64], pct: f64) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() - 1) as f64 * pct.clamp(0.0, 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Computes precision, recall, and F1 percentages from TP, FP, and indexed count.
///
/// # Arguments
/// * `tp` - True positive matches.
/// * `fp` - False positive collisions.
/// * `indexed_total` - Number of evaluated videos that exist in the reference catalog.
///
/// # Returns
/// Tuple `(fn_count, precision_pct, recall_pct, f1_pct)`.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Avoids division by zero when `tp + fp == 0` or `indexed_total == 0`.
fn compute_prf1(tp: usize, fp: usize, indexed_total: usize) -> (usize, f64, f64, f64) {
    let fn_count = indexed_total.saturating_sub(tp);
    let precision = if tp + fp > 0 {
        (tp as f64 / (tp + fp) as f64) * PERCENT_SCALE
    } else {
        PERCENT_SCALE
    };
    let recall = if indexed_total > 0 {
        (tp as f64 / indexed_total as f64) * PERCENT_SCALE
    } else {
        0.0
    };
    let f1 = if precision + recall > 0.0 {
        (2.0 * precision * recall) / (precision + recall)
    } else {
        0.0
    };
    (fn_count, precision, recall, f1)
}

/// Extracts ground-truth title string from media path and stem.
///
/// # Arguments
/// * `media_path` - Full path to the video file.
/// * `media_stem` - File stem without extension.
///
/// # Returns
/// Combined ground-truth title string suitable for `are_titles_matching`.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// When episodes are stored as `Show Name/S01/S01E01.mkv`, combines parent directory name with stem.
fn derive_ground_truth_title(media_path: &str, media_stem: &str) -> String {
    let parsed = dbsrt_core::MediaMetadata::parse(media_stem);
    if parsed.clean_title.len() >= MIN_CLEAN_TITLE_LEN {
        return media_stem.to_string();
    }
    let path = Path::new(media_path);
    let mut parts = Vec::new();
    if let Some(parent) = path.parent() {
        if let Some(grandparent) = parent.parent().and_then(|p| p.file_name()) {
            parts.push(grandparent.to_string_lossy().to_string());
        }
        if let Some(pname) = parent.file_name() {
            parts.push(pname.to_string_lossy().to_string());
        }
    }
    parts.push(media_stem.to_string());
    parts.join(" ")
}

/// Builds normalized `(title|sX|eY)` membership sets for `dbphash` and `dense_catalog`.
fn build_catalog_membership_sets(
    dbphash: &MmapDbphash,
    dense_catalog: &AHashCatalog,
) -> (HashSet<String>, HashSet<String>) {
    let mut phash_keys = HashSet::new();
    for entry in dbphash.media_entries() {
        let se = (
            Some(entry.season).filter(|&v| v > 0),
            Some(entry.episode).filter(|&v| v > 0),
        );
        let (p, s) = format_catalog_membership_keys(dbphash.get_title(entry), se);
        if !p.is_empty() {
            phash_keys.insert(p);
        }
        if let Some(sec) = s {
            phash_keys.insert(sec);
        }
    }
    let mut ahash_keys = HashSet::new();
    for entry in &dense_catalog.entries {
        let (p, s) = format_catalog_membership_keys(&entry.media_name, (None, None));
        if !p.is_empty() {
            ahash_keys.insert(p);
        }
        if let Some(sec) = s {
            ahash_keys.insert(sec);
        }
    }
    (phash_keys, ahash_keys)
}

/// Loads and prepares real video multi-window probe records from JSONL.
///
/// # Arguments
/// * `jsonl_path` - Path to JSONL file produced by `bench-scale-multimodal`.
/// * `dbphash` - Loaded `.dbphash` visual catalog.
/// * `dense_catalog` - Loaded `.ahash` acoustic catalog.
///
/// # Returns
/// `Result<Vec<PreparedVideoProbe>>` containing valid records with extracted window hashes.
///
/// # Errors
/// Fails if `jsonl_path` cannot be opened.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Filters out `ERROR` records and resolves episode-exact catalog membership flags for each video.
fn load_prepared_probes(
    jsonl_path: &Path,
    dbphash: &MmapDbphash,
    dense_catalog: &AHashCatalog,
) -> Result<Vec<PreparedVideoProbe>> {
    let (phash_keys, ahash_keys) = build_catalog_membership_sets(dbphash, dense_catalog);
    let file = File::open(jsonl_path).context("Open JSONL for matrix evaluation")?;
    let reader = BufReader::new(file);
    let mut probes = Vec::new();

    for line in reader.lines().map_while(Result::ok) {
        let Ok(rec) = serde_json::from_str::<MediaScaleBenchmarkRecord>(&line) else {
            continue;
        };
        if rec.agreement.status == "ERROR" || rec.duration_sec <= 0.0 {
            continue;
        }
        if rec.phash.windows.iter().all(|w| w.query_hashes.is_empty()) {
            continue;
        }
        let gt_title = derive_ground_truth_title(&rec.media_path, &rec.media_stem);
        let in_phash = rec.phash.media_match.as_ref().is_some_and(|m| {
            are_titles_matching(m, (rec.phash.season, rec.phash.episode), &gt_title)
        }) || is_catalog_member(&phash_keys, &gt_title, (None, None));
        let in_ahash = rec.audio.track_count > 0
            && (rec
                .ahash
                .media_match
                .as_ref()
                .is_some_and(|m| are_titles_matching(&gt_title, (None, None), m))
                || is_catalog_member(&ahash_keys, &gt_title, (None, None)));

        probes.push(PreparedVideoProbe {
            record: rec,
            gt_title,
            in_phash_catalog: in_phash,
            in_ahash_catalog: in_ahash,
        });
    }
    Ok(probes)
}

/// Searches `MmapDbphash` using single-threaded MIH-16 lookups.
///
/// # Arguments
/// * `mmap` - Memory-mapped `.dbphash` catalog.
/// * `query_hashes` - Query perceptual hashes.
/// * `max_dist` - Maximum Hamming distance threshold.
///
/// # Returns
/// Ranked list of [`DbphashMediaMatch`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Probes sparse and cuts MIH-16 tables sequentially on a single thread, then aggregates matches.
fn search_phash_mih16_mono(
    mmap: &MmapDbphash,
    query_hashes: &[u64],
    max_dist: u32,
) -> Vec<DbphashMediaMatch> {
    let sparse = mmap.sparse_entries();
    let cuts = mmap.cuts_entries();
    let sparse_mih = mmap.sparse_mih();
    let cuts_mih_idx = mmap.cuts_mih();

    let mut sparse_matches = Vec::new();
    for (q_idx, &q_hash) in query_hashes.iter().enumerate() {
        for c in sparse_mih.search(q_hash, max_dist, sparse) {
            sparse_matches.push(DbphashCutMatch {
                query_idx: q_idx,
                cut_entry: sparse[c.index],
                distance: c.distance,
            });
        }
    }
    let mut cuts_matches = Vec::new();
    for (q_idx, &q_hash) in query_hashes.iter().enumerate() {
        for c in cuts_mih_idx.search(q_hash, max_dist, cuts) {
            cuts_matches.push(DbphashCutMatch {
                query_idx: q_idx,
                cut_entry: cuts[c.index],
                distance: c.distance,
            });
        }
    }
    mmap.aggregate_matches(query_hashes, sparse_matches, cuts_matches)
}

/// Probes one `AHashMih32Index` table against a `DbphashCutEntry` slice.
///
/// # Arguments
/// * `index32` - 32-bit half-hash MIH table.
/// * `entries` - Reference cut or sparse entries.
/// * `q_idx` - Query frame index.
/// * `q_hash` - 64-bit query perceptual hash.
///
/// # Returns
/// Matching [`DbphashCutMatch`] items within [`DEFAULT_PHASH_SCALE_MAX_DISTANCE`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Probes both upper and lower 32-bit slots and filters candidates by full 64-bit Hamming distance.
fn collect_phash_mih32_slice(
    index32: &AHashMih32Index,
    entries: &[dbsrt_index::DbphashCutEntry],
    q_idx: usize,
    q_hash: u64,
) -> Vec<DbphashCutMatch> {
    let mut hits = Vec::new();
    let u = (q_hash >> UPPER_32_SHIFT) as u32;
    for entry in index32.lookup_upper(u) {
        let cand = entries[entry.frame_idx as usize];
        let dist = hamming_distance(cand.phash, q_hash);
        if dist <= DEFAULT_PHASH_SCALE_MAX_DISTANCE {
            hits.push(DbphashCutMatch {
                query_idx: q_idx,
                cut_entry: cand,
                distance: dist,
            });
        }
    }
    let l = (q_hash & LOWER_32_MASK) as u32;
    for entry in index32.lookup_lower(l) {
        let cand = entries[entry.frame_idx as usize];
        let dist = hamming_distance(cand.phash, q_hash);
        if dist <= DEFAULT_PHASH_SCALE_MAX_DISTANCE {
            hits.push(DbphashCutMatch {
                query_idx: q_idx,
                cut_entry: cand,
                distance: dist,
            });
        }
    }
    hits
}

/// Searches `MmapDbphash` using 32-bit `AHashMih32Index` tables (Mono or Rayon).
///
/// # Arguments
/// * `ctx` - Bundled `.dbphash` and 32-bit MIH tables ([`PhashMih32Context`]).
/// * `query_hashes` - Query perceptual hashes.
/// * `parallel` - Whether to execute query frame lookups via Rayon.
///
/// # Returns
/// Ranked list of [`DbphashMediaMatch`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Collects sparse and cuts hits via MIH-32 and delegates scoring to `aggregate_matches`.
fn search_phash_mih32(
    ctx: &PhashMih32Context<'_>,
    query_hashes: &[u64],
    parallel: bool,
) -> Vec<DbphashMediaMatch> {
    let sparse = ctx.mmap.sparse_entries();
    let cuts = ctx.mmap.cuts_entries();
    let (sparse_matches, cuts_matches) = if parallel {
        rayon::join(
            || {
                query_hashes
                    .par_iter()
                    .enumerate()
                    .flat_map_iter(|(q_idx, &q_hash)| {
                        collect_phash_mih32_slice(ctx.sparse32, sparse, q_idx, q_hash)
                    })
                    .collect()
            },
            || {
                query_hashes
                    .par_iter()
                    .enumerate()
                    .flat_map_iter(|(q_idx, &q_hash)| {
                        collect_phash_mih32_slice(ctx.cuts32, cuts, q_idx, q_hash)
                    })
                    .collect()
            },
        )
    } else {
        let s = query_hashes
            .iter()
            .enumerate()
            .flat_map(|(q_idx, &q_hash)| {
                collect_phash_mih32_slice(ctx.sparse32, sparse, q_idx, q_hash)
            })
            .collect();
        let c = query_hashes
            .iter()
            .enumerate()
            .flat_map(|(q_idx, &q_hash)| collect_phash_mih32_slice(ctx.cuts32, cuts, q_idx, q_hash))
            .collect();
        (s, c)
    };
    ctx.mmap
        .aggregate_matches(query_hashes, sparse_matches, cuts_matches)
}

/// Records multi-window shift spread on a verified true positive consensus match.
fn record_shift_spread(best: &MultiWindowConsensus, out_spreads: &mut Vec<i64>) {
    if best.window_matches.len() >= 2 {
        let rels: Vec<i64> = best
            .window_matches
            .iter()
            .map(|w| w.estimated_shift_ms - (w.target_start_sec * MILLIS_PER_SECOND).round() as i64)
            .collect();
        let min_s = rels.iter().copied().min().unwrap_or(0);
        let max_s = rels.iter().copied().max().unwrap_or(0);
        out_spreads.push((max_s - min_s).abs());
    }
}

/// Converts filtered `DbphashMediaMatch` entries for one window into `RawCandidateMatch` tuples.
///
/// # Arguments
/// * `spec` - Visual evaluation specification.
/// * `filtered` - Ranked visual matches above confidence threshold.
/// * `meta` - Probed container duration and framerate.
///
/// # Returns
/// Vector of [`RawCandidateMatch`] items for multi-window consensus.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Applies [`normalize_phash_shift_ms`] when `spec.multi_cand_and_fps_norm` is enabled.
fn build_phash_window_cands(
    spec: &PhashEvalSpec<'_>,
    filtered: Vec<DbphashMediaMatch>,
    meta: &VideoMetadata,
) -> Vec<RawCandidateMatch> {
    filtered
        .into_iter()
        .map(|m| {
            let shift = if spec.multi_cand_and_fps_norm {
                normalize_phash_shift_ms(spec.dbphash, m.media_id, m.estimated_shift_ms, meta)
            } else {
                m.estimated_shift_ms
            };
            (
                m.media_id,
                m.clean_title,
                m.matching_cuts,
                m.matching_sparse,
                shift,
                m.confidence,
            )
        })
        .collect()
}

/// Evaluates one prepared video probe under a visual `pHash` technique and updates `acc`.
///
/// # Arguments
/// * `spec` - Visual technique specification.
/// * `probe` - Prepared real video multi-window probe.
/// * `acc` - Mutable evaluation accumulator.
/// * `search_fn` - Closure executing single-window `pHash` lookup.
///
/// # Returns
/// `()`.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Updates single-window (`t=50%`) and 3-window consensus (`20%, 50%, 80%`) TP/FP counters and latency vectors.
fn evaluate_phash_probe<F>(
    spec: &PhashEvalSpec<'_>,
    probe: &PreparedVideoProbe,
    acc: &mut EvalAccumulator,
    search_fn: &F,
) where
    F: Fn(&[u64]) -> Vec<DbphashMediaMatch>,
{
    let take_limit = if spec.multi_cand_and_fps_norm {
        MAX_CONSENSUS_CANDIDATES_PER_WINDOW
    } else {
        1
    };
    let meta = VideoMetadata {
        duration: probe.record.duration_sec,
        fps: probe.record.fps,
    };
    let mut per_window: Vec<PerWindowMatches> =
        Vec::with_capacity(probe.record.phash.windows.len());
    let mut win1_top: Option<DbphashMediaMatch> = None;

    for (w_idx, win) in probe.record.phash.windows.iter().enumerate() {
        if win.query_hashes.is_empty() {
            continue;
        }
        let slice_len = spec.frame_cap.map_or(win.query_hashes.len(), |cap| {
            cap.min(win.query_hashes.len())
        });
        let q_slice = &win.query_hashes[..slice_len];
        let is_solid_dc = spec.multi_cand_and_fps_norm && is_solid_dc_phash_window(q_slice);
        let is_degenerate =
            spec.multi_cand_and_fps_norm && is_degenerate_static_phash_window(q_slice);
        let t0 = Instant::now();
        let matches = search_fn(q_slice);
        acc.win_latencies_ms
            .push(t0.elapsed().as_secs_f64() * MILLIS_PER_SECOND);

        let filtered: Vec<DbphashMediaMatch> = matches
            .into_iter()
            .filter(|m| {
                !is_solid_dc
                    && m.confidence >= PHASH_MIN_CONFIDENCE
                    && (!is_degenerate || m.matching_cuts > 0)
            })
            .take(take_limit)
            .collect();
        if w_idx == CENTER_WINDOW_IDX || (win1_top.is_none() && w_idx == 0) {
            win1_top = filtered.first().cloned();
        }
        let raw_cands = build_phash_window_cands(spec, filtered, &meta);
        per_window.push((w_idx, win.ratio, win.start_sec, raw_cands));
    }

    if let Some(m1) = win1_top {
        acc.win1_matched += 1;
        if are_titles_matching(&m1.clean_title, (m1.season, m1.episode), &probe.gt_title) {
            acc.win1_tp += 1;
        } else {
            acc.win1_fp += 1;
        }
    }

    let cleaned = if spec.multi_cand_and_fps_norm {
        filter_coherent_majority_windows(per_window, MAX_SHIFT_DRIFT_TOLERANCE_MS)
    } else {
        per_window
    };
    let consensus = aggregate_multi_window_matches(
        probe.record.phash.windows.len().max(1),
        cleaned,
        MAX_SHIFT_DRIFT_TOLERANCE_MS,
    );
    let chosen = if spec.multi_cand_and_fps_norm {
        select_best_consensus(&consensus)
    } else {
        consensus.first().filter(|c| c.matching_windows > 0)
    };
    let Some(best) = chosen else {
        return;
    };
    acc.win3_matched += 1;
    if best.is_temporally_coherent {
        acc.win3_coherent += 1;
    }
    let se = spec
        .dbphash
        .find_media_by_id(best.media_id)
        .map(|e| {
            (
                Some(e.season).filter(|&v| v > 0),
                Some(e.episode).filter(|&v| v > 0),
            )
        })
        .unwrap_or((None, None));
    if are_titles_matching(&best.clean_title, se, &probe.gt_title) {
        acc.win3_tp += 1;
        record_shift_spread(best, &mut acc.shift_spreads_ms);
    } else {
        acc.win3_fp += 1;
    }
}

/// Evaluates one visual `pHash` technique across the prepared real video probes.
///
/// # Arguments
/// * `spec` - Visual technique specification ([`PhashEvalSpec`]).
/// * `probes` - Prepared real video multi-window probes.
/// * `search_fn` - Closure executing single-window `pHash` lookup.
///
/// # Returns
/// Populated [`PhashTechniqueMetrics`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Evaluates both single-window (`t=50%`) and 3-window consensus (`20%, 50%, 80%`) on identical hashes.
fn run_single_phash_technique<F>(
    spec: &PhashEvalSpec<'_>,
    probes: &[PreparedVideoProbe],
    search_fn: F,
) -> PhashTechniqueMetrics
where
    F: Fn(&[u64]) -> Vec<DbphashMediaMatch>,
{
    let mut acc = EvalAccumulator::default();
    let indexed_videos = probes.iter().filter(|p| p.in_phash_catalog).count();
    for probe in probes {
        evaluate_phash_probe(spec, probe, &mut acc, &search_fn);
    }
    finalize_phash_metrics(spec, (probes.len(), indexed_videos), &acc)
}

/// Converts accumulated counters into [`PhashTechniqueMetrics`].
fn finalize_phash_metrics(
    spec: &PhashEvalSpec<'_>,
    counts: (usize, usize),
    acc: &EvalAccumulator,
) -> PhashTechniqueMetrics {
    let (evaluated_videos, indexed_videos) = counts;
    let evaluated_windows = acc.win_latencies_ms.len();
    let mean_win_ms = if evaluated_windows > 0 {
        acc.win_latencies_ms.iter().sum::<f64>() / evaluated_windows as f64
    } else {
        0.0
    };
    let (win1_fn, win1_prec, win1_rec, win1_f1) =
        compute_prf1(acc.win1_tp, acc.win1_fp, indexed_videos);
    let (win3_fn, win3_prec, win3_rec, win3_f1) =
        compute_prf1(acc.win3_tp, acc.win3_fp, indexed_videos);

    PhashTechniqueMetrics {
        technique: spec.name.to_string(),
        evaluated_videos,
        evaluated_windows,
        indexed_videos_in_sample: indexed_videos,
        index_frames: spec.index_frames,
        index_ram_mb: spec.index_ram_mb,
        index_build_ms: spec.index_build_ms,
        win_latency_mean_ms: mean_win_ms,
        win_latency_p50_ms: percentile_f64(&acc.win_latencies_ms, PERCENTILE_P50),
        win_latency_p95_ms: percentile_f64(&acc.win_latencies_ms, PERCENTILE_P95),
        win_latency_p99_ms: percentile_f64(&acc.win_latencies_ms, PERCENTILE_P99),
        video_3win_latency_mean_ms: mean_win_ms * WINDOWS_PER_VIDEO_FACTOR,
        win1_matched: acc.win1_matched,
        win1_tp: acc.win1_tp,
        win1_fp: acc.win1_fp,
        win1_fn,
        win1_precision_pct: win1_prec,
        win1_recall_pct: win1_rec,
        win1_f1_pct: win1_f1,
        win3_matched: acc.win3_matched,
        win3_coherent: acc.win3_coherent,
        win3_tp: acc.win3_tp,
        win3_fp: acc.win3_fp,
        win3_fn,
        win3_precision_pct: win3_prec,
        win3_recall_pct: win3_rec,
        win3_f1_pct: win3_f1,
        median_intra_video_shift_spread_ms: percentile_i64(&acc.shift_spreads_ms, PERCENTILE_P50),
    }
}

/// Evaluates all visual `pHash` techniques across the real video probes.
fn evaluate_phash_matrix(
    probes: &[PreparedVideoProbe],
    dbphash: &MmapDbphash,
) -> Vec<PhashTechniqueMetrics> {
    let total_frames = dbphash.sparse_entries().len() + dbphash.cuts_entries().len();
    let t_m16 = Instant::now();
    let _ = dbphash.sparse_mih();
    let _ = dbphash.cuts_mih();
    let m16_build_ms = t_m16.elapsed().as_secs_f64() * MILLIS_PER_SECOND;
    let m16_ram_mb = (total_frames * DBPHASH_ENTRY_BYTES
        + 2 * MIH16_TABLE_COUNT * MIH16_BUCKET_COUNT * MIH16_BUCKET_HEADER_BYTES
        + total_frames * MIH16_TABLE_COUNT * MIH16_INDEX_BYTES) as f64
        / BYTES_PER_MIB;

    let t_m32 = Instant::now();
    let sparse_hashes: Vec<u64> = dbphash.sparse_entries().iter().map(|e| e.phash).collect();
    let cuts_hashes: Vec<u64> = dbphash.cuts_entries().iter().map(|e| e.phash).collect();
    let sparse32 = AHashMih32Index::build(&sparse_hashes);
    let cuts32 = AHashMih32Index::build(&cuts_hashes);
    let m32_build_ms = t_m32.elapsed().as_secs_f64() * MILLIS_PER_SECOND;
    let m32_ram_mb = (total_frames * DBPHASH_ENTRY_BYTES
        + total_frames * MIH32_TABLE_COUNT * MIH32_ENTRY_BYTES) as f64
        / BYTES_PER_MIB;
    let linear_ram_mb = (total_frames * DBPHASH_ENTRY_BYTES) as f64 / BYTES_PER_MIB;
    let m32_ctx = PhashMih32Context {
        mmap: dbphash,
        sparse32: &sparse32,
        cuts32: &cuts32,
    };

    let mut results = Vec::new();
    results.push(run_single_phash_technique(
        &PhashEvalSpec {
            name: "pHash MIH-16 (Rayon, Multi-Cand + FPS Norm)",
            dbphash,
            index_frames: total_frames,
            index_ram_mb: m16_ram_mb,
            index_build_ms: m16_build_ms,
            multi_cand_and_fps_norm: true,
            frame_cap: None,
        },
        probes,
        |q| dbphash.recognize(q, DEFAULT_PHASH_SCALE_MAX_DISTANCE),
    ));
    let has_extended_window =
        probes.iter().any(|p| {
            p.record.phash.windows.iter().any(|w| {
                w.query_hashes.len() > ONE_SECOND_FRAME_CAP + EXTENDED_WINDOW_MARGIN_FRAMES
            })
        });
    if has_extended_window {
        results.push(run_single_phash_technique(
            &PhashEvalSpec {
                name: "pHash MIH-16 (Rayon, 1.0s / 24-frame Slice)",
                dbphash,
                index_frames: total_frames,
                index_ram_mb: m16_ram_mb,
                index_build_ms: m16_build_ms,
                multi_cand_and_fps_norm: true,
                frame_cap: Some(ONE_SECOND_FRAME_CAP),
            },
            probes,
            |q| dbphash.recognize(q, DEFAULT_PHASH_SCALE_MAX_DISTANCE),
        ));
    }
    results.push(run_single_phash_technique(
        &PhashEvalSpec {
            name: "pHash MIH-16 (Rayon, Legacy Top-1 Raw)",
            dbphash,
            index_frames: total_frames,
            index_ram_mb: m16_ram_mb,
            index_build_ms: m16_build_ms,
            multi_cand_and_fps_norm: false,
            frame_cap: Some(ONE_SECOND_FRAME_CAP),
        },
        probes,
        |q| dbphash.recognize(q, DEFAULT_PHASH_SCALE_MAX_DISTANCE),
    ));
    results.push(run_single_phash_technique(
        &PhashEvalSpec {
            name: "pHash MIH-16 (Mono, Multi-Cand + FPS Norm)",
            dbphash,
            index_frames: total_frames,
            index_ram_mb: m16_ram_mb,
            index_build_ms: m16_build_ms,
            multi_cand_and_fps_norm: true,
            frame_cap: None,
        },
        probes,
        |q| search_phash_mih16_mono(dbphash, q, DEFAULT_PHASH_SCALE_MAX_DISTANCE),
    ));
    results.push(run_single_phash_technique(
        &PhashEvalSpec {
            name: "pHash MIH-32 (Rayon, Multi-Cand + FPS Norm)",
            dbphash,
            index_frames: total_frames,
            index_ram_mb: m32_ram_mb,
            index_build_ms: m32_build_ms,
            multi_cand_and_fps_norm: true,
            frame_cap: None,
        },
        probes,
        |q| search_phash_mih32(&m32_ctx, q, true),
    ));
    results.push(run_single_phash_technique(
        &PhashEvalSpec {
            name: "pHash MIH-32 (Mono, Multi-Cand + FPS Norm)",
            dbphash,
            index_frames: total_frames,
            index_ram_mb: m32_ram_mb,
            index_build_ms: m32_build_ms,
            multi_cand_and_fps_norm: true,
            frame_cap: None,
        },
        probes,
        |q| search_phash_mih32(&m32_ctx, q, false),
    ));
    let lin_slice = &probes[..probes.len().min(PHASH_LINEAR_SAMPLE_VIDEOS)];
    results.push(run_single_phash_technique(
        &PhashEvalSpec {
            name: "pHash Linear O(N) (Rayon, Ground Truth)",
            dbphash,
            index_frames: total_frames,
            index_ram_mb: linear_ram_mb,
            index_build_ms: 0.0,
            multi_cand_and_fps_norm: true,
            frame_cap: None,
        },
        lin_slice,
        |q| dbphash.recognize_linear(q, DEFAULT_PHASH_SCALE_MAX_DISTANCE),
    ));
    results
}

/// Ground-truth `(media_name, offset_ms)` map indexed by `(video_idx, window_idx)` from $K=1$.
type K1ShiftBaselineMap = HashMap<(usize, usize), (String, i64)>;

/// Mutable `(accumulator, k1_baseline_map)` state pair threaded through acoustic probe evaluation.
type AhashProbeState<'a> = (&'a mut EvalAccumulator, &'a mut K1ShiftBaselineMap);

/// Records an `AHashCatalogMatch` into `acc` and `k1_gt`, returning a `RawCandidateMatch` tuple.
///
/// # Arguments
/// * `spec` - Acoustic technique specification.
/// * `coord` - `(video_idx, window_idx)` coordinates for $K=1$ drift tracking.
/// * `m` - Matched acoustic candidate.
/// * `state` - Mutable `(accumulator, k1_ground_truth_map)` pair.
///
/// # Returns
/// Single [`RawCandidateMatch`] tuple for multi-window consensus.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Stores `(media_name, offset_ms)` when `factor_k == 1` and computes `|offset_ms - gt_off|` for $K > 1$.
fn record_ahash_window_hit(
    spec: &AhashEvalSpec<'_>,
    coord: (usize, usize),
    m: AHashCatalogMatch,
    state: AhashProbeState<'_>,
) -> RawCandidateMatch {
    let (acc, k1_gt) = state;
    acc.fine_distances.push(m.mean_distance);
    acc.snr_values.push(m.snr_bits);
    if spec.factor_k == 1 {
        k1_gt.insert(coord, (m.media_name.clone(), m.offset_ms));
    } else if let Some((gt_name, gt_off)) = k1_gt.get(&coord) {
        if &m.media_name == gt_name {
            acc.k1_drifts_ms.push((m.offset_ms - gt_off).abs());
        }
    }
    let cand_id = resolve_ahash_candidate_id(spec.dense_catalog, &m.file_path, &m.media_name);
    let ref_time_ms = (m.offset_sec * MILLIS_PER_SECOND).round() as i64;
    let conf = m.score * PERCENT_SCALE;
    let sparse = (m.snr_bits * AHASH_SPARSE_SNR_SCALE) as usize;
    (cand_id, m.media_name, 0, sparse, ref_time_ms, conf)
}

/// Evaluates one prepared video probe under an acoustic `aHash` technique.
///
/// # Arguments
/// * `spec` - Acoustic evaluation specification.
/// * `probe_indexed` - `(video_idx, probe)` pair.
/// * `state` - Mutable `(accumulator, k1_ground_truth_map)` pair.
/// * `search_fn` - Closure executing single-window `aHash` lookup.
///
/// # Returns
/// `()`.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Runs single-window and 3-window consensus evaluation across persisted acoustic `query_hashes`.
fn evaluate_ahash_probe<F>(
    spec: &AhashEvalSpec<'_>,
    probe_indexed: (usize, &PreparedVideoProbe),
    state: AhashProbeState<'_>,
    search_fn: &F,
) where
    F: Fn(&[u64], f64) -> Option<AHashCatalogMatch>,
{
    let (v_idx, probe) = probe_indexed;
    let (acc, k1_gt) = state;
    let mut per_window: Vec<PerWindowMatches> =
        Vec::with_capacity(probe.record.ahash.windows.len());
    let mut win1_match: Option<AHashCatalogMatch> = None;

    for (w_idx, win) in probe.record.ahash.windows.iter().enumerate() {
        if win.query_hashes.is_empty() {
            continue;
        }
        let fps = if win.audio_fps > 0.0 {
            win.audio_fps
        } else {
            DEFAULT_AUDIO_FPS_VAL
        };
        let t0 = Instant::now();
        let res = search_fn(&win.query_hashes, fps);
        acc.win_latencies_ms
            .push(t0.elapsed().as_secs_f64() * MILLIS_PER_SECOND);

        if w_idx == CENTER_WINDOW_IDX || (win1_match.is_none() && w_idx == 0) {
            win1_match = res.clone();
        }
        let mut win_matches = Vec::new();
        if let Some(m) = res {
            win_matches.push(record_ahash_window_hit(
                spec,
                (v_idx, w_idx),
                m,
                (acc, k1_gt),
            ));
        }
        per_window.push((w_idx, win.ratio, win.start_sec, win_matches));
    }

    if let Some(m1) = win1_match {
        acc.win1_matched += 1;
        if are_titles_matching(&probe.gt_title, (None, None), &m1.media_name) {
            acc.win1_tp += 1;
        } else {
            acc.win1_fp += 1;
        }
    }

    let consensus: Vec<MultiWindowConsensus> = aggregate_multi_window_matches(
        probe.record.ahash.windows.len().max(1),
        per_window,
        MAX_SHIFT_DRIFT_TOLERANCE_MS,
    );
    let Some(best) = select_best_consensus(&consensus) else {
        return;
    };
    acc.win3_matched += 1;
    if best.is_temporally_coherent {
        acc.win3_coherent += 1;
    }
    if are_titles_matching(&probe.gt_title, (None, None), &best.clean_title) {
        acc.win3_tp += 1;
        record_shift_spread(best, &mut acc.shift_spreads_ms);
    } else {
        acc.win3_fp += 1;
    }
}

/// Evaluates one acoustic `aHash` technique across the prepared real video probes.
///
/// # Arguments
/// * `spec` - Acoustic evaluation specification ([`AhashEvalSpec`]).
/// * `probes` - Prepared real video multi-window probes.
/// * `k1_gt` - Ground-truth offset registry populated during $K=1$ evaluation.
/// * `search_fn` - Closure executing single-window `aHash` lookup.
///
/// # Returns
/// Populated [`AhashTechniqueMetrics`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Accumulates single-window and 3-window accuracy, SNR, latency, and temporal drift vs $K=1$.
fn run_single_ahash_technique<F>(
    spec: &AhashEvalSpec<'_>,
    probes: &[PreparedVideoProbe],
    k1_gt: &mut K1ShiftBaselineMap,
    search_fn: F,
) -> AhashTechniqueMetrics
where
    F: Fn(&[u64], f64) -> Option<AHashCatalogMatch>,
{
    let mut acc = EvalAccumulator::default();
    let indexed_videos = probes.iter().filter(|p| p.in_ahash_catalog).count();
    for (v_idx, probe) in probes.iter().enumerate() {
        evaluate_ahash_probe(spec, (v_idx, probe), (&mut acc, k1_gt), &search_fn);
    }
    finalize_ahash_metrics(spec, (probes.len(), indexed_videos), &acc)
}

/// Converts accumulated counters into [`AhashTechniqueMetrics`].
fn finalize_ahash_metrics(
    spec: &AhashEvalSpec<'_>,
    counts: (usize, usize),
    acc: &EvalAccumulator,
) -> AhashTechniqueMetrics {
    let (evaluated_videos, indexed_videos) = counts;
    let evaluated_windows = acc.win_latencies_ms.len();
    let mean_win_ms = if evaluated_windows > 0 {
        acc.win_latencies_ms.iter().sum::<f64>() / evaluated_windows as f64
    } else {
        0.0
    };
    let (win1_fn, win1_prec, win1_rec, win1_f1) =
        compute_prf1(acc.win1_tp, acc.win1_fp, indexed_videos);
    let (win3_fn, win3_prec, win3_rec, win3_f1) =
        compute_prf1(acc.win3_tp, acc.win3_fp, indexed_videos);
    let mean_dist = if !acc.fine_distances.is_empty() {
        acc.fine_distances.iter().sum::<f64>() / acc.fine_distances.len() as f64
    } else {
        0.0
    };
    let mean_snr = if !acc.snr_values.is_empty() {
        acc.snr_values.iter().sum::<f64>() / acc.snr_values.len() as f64
    } else {
        0.0
    };
    let mean_drift = if !acc.k1_drifts_ms.is_empty() {
        acc.k1_drifts_ms.iter().sum::<i64>() as f64 / acc.k1_drifts_ms.len() as f64
    } else {
        0.0
    };
    let drift_pct = |tol: i64| -> f64 {
        if spec.factor_k == 1 {
            PERCENT_SCALE
        } else if !acc.k1_drifts_ms.is_empty() {
            let ok = acc.k1_drifts_ms.iter().filter(|&&d| d <= tol).count();
            (ok as f64 / acc.k1_drifts_ms.len() as f64) * PERCENT_SCALE
        } else {
            0.0
        }
    };

    AhashTechniqueMetrics {
        technique: spec.name.to_string(),
        factor_k: spec.factor_k,
        coarse_fps: spec.coarse_fps,
        evaluated_videos,
        evaluated_windows,
        indexed_videos_in_sample: indexed_videos,
        index_frames: spec.index_frames,
        index_ram_mb: spec.index_ram_mb,
        index_build_ms: spec.index_build_ms,
        win_latency_mean_ms: mean_win_ms,
        win_latency_p50_ms: percentile_f64(&acc.win_latencies_ms, PERCENTILE_P50),
        win_latency_p95_ms: percentile_f64(&acc.win_latencies_ms, PERCENTILE_P95),
        win_latency_p99_ms: percentile_f64(&acc.win_latencies_ms, PERCENTILE_P99),
        video_3win_latency_mean_ms: mean_win_ms * WINDOWS_PER_VIDEO_FACTOR,
        win1_matched: acc.win1_matched,
        win1_tp: acc.win1_tp,
        win1_fp: acc.win1_fp,
        win1_fn,
        win1_precision_pct: win1_prec,
        win1_recall_indexed_pct: win1_rec,
        win1_f1_pct: win1_f1,
        win3_matched: acc.win3_matched,
        win3_coherent: acc.win3_coherent,
        win3_tp: acc.win3_tp,
        win3_fp: acc.win3_fp,
        win3_fn,
        win3_precision_pct: win3_prec,
        win3_recall_indexed_pct: win3_rec,
        win3_f1_pct: win3_f1,
        mean_fine_distance_bits: mean_dist,
        mean_snr_bits: mean_snr,
        shift_drift_vs_k1_mean_ms: mean_drift,
        shift_drift_vs_k1_p95_ms: percentile_i64(&acc.k1_drifts_ms, PERCENTILE_P95),
        shift_drift_vs_k1_max_ms: acc.k1_drifts_ms.iter().copied().max().unwrap_or(0),
        shift_within_15ms_pct: drift_pct(SHIFT_TOLERANCE_TIGHT_MS),
        shift_within_25ms_pct: drift_pct(SHIFT_TOLERANCE_NOMINAL_MS),
        median_intra_video_shift_spread_ms: percentile_i64(&acc.shift_spreads_ms, PERCENTILE_P50),
    }
}

/// Evaluates fine correlation around top vote clusters for direct MIH-16 / MIH-32 benchmarks.
fn verify_top_vote_clusters(
    req: &AhashDirectSearchReq<'_>,
    votes: HashMap<(usize, usize), u32>,
) -> Option<AHashCatalogMatch> {
    let q_len = req.query.len();
    let mut clusters: Vec<(usize, usize, u32)> = votes
        .into_iter()
        .filter(|&(_, v)| v >= DIRECT_MIH_MIN_VOTES)
        .map(|((m_idx, shift), v)| (m_idx, shift, v))
        .collect();
    clusters.sort_by_key(|a| std::cmp::Reverse(a.2));

    let mut best_match: Option<AHashCatalogMatch> = None;
    for (media_idx, coarse_shift, _) in clusters.into_iter().take(DIRECT_MIH_TOP_CLUSTERS) {
        let entry = &req.catalog.entries[media_idx];
        if (entry.audio_fps - req.audio_fps).abs() > FPS_COMPATIBILITY_TOLERANCE
            || entry.hashes.len() < q_len
        {
            continue;
        }
        let min_s = coarse_shift.saturating_sub(DEFAULT_AHASH_MIH_JITTER_RADIUS);
        let max_s =
            (coarse_shift + DEFAULT_AHASH_MIH_JITTER_RADIUS).min(entry.hashes.len() - q_len);
        let mut best_shift = coarse_shift;
        let mut best_dist = u64::MAX;
        for s in min_s..=max_s {
            let dist: u64 = req
                .query
                .iter()
                .zip(&entry.hashes[s..s + q_len])
                .map(|(&q, &r)| (q ^ r).count_ones() as u64)
                .sum();
            if dist < best_dist {
                best_dist = dist;
                best_shift = s;
            }
        }
        let mean_dist = best_dist as f64 / q_len as f64;
        if mean_dist <= FINE_SEARCH_MAX_DISTANCE
            && best_match
                .as_ref()
                .is_none_or(|b| mean_dist < b.mean_distance)
        {
            let offset_sec = best_shift as f64 / entry.audio_fps;
            best_match = Some(AHashCatalogMatch {
                media_name: entry.media_name.clone(),
                file_path: entry.path.to_string_lossy().to_string(),
                ref_frame_offset: best_shift,
                offset_sec,
                offset_ms: (offset_sec * MILLIS_PER_SECOND).round() as i64,
                mean_distance: mean_dist,
                score: (1.0 - (mean_dist / HASH_BITS_F64)).clamp(0.0, 1.0),
                snr_bits: (RANDOM_DISTANCE_BITS - mean_dist).max(0.0),
                audio_fps: entry.audio_fps,
            });
        }
    }
    best_match
}

/// Collects `(media_idx, shift)` hits for one query frame against `AHashMih32Index`.
///
/// # Arguments
/// * `catalog` - Dense `AHashCatalog`.
/// * `index32` - 32-bit half-hash MIH index.
/// * `frame` - `(query_frame_offset, query_hash)` tuple.
///
/// # Returns
/// Vector of `(media_idx, shift_offset)` candidate hits.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Skips buckets larger than [`MAX_MIH_BUCKET_SIZE`] and resolves global frame offsets via `resolve_global_index`.
fn collect_mih32_frame_hits(
    catalog: &AHashCatalog,
    index32: &AHashMih32Index,
    frame: (usize, u64),
) -> Vec<(usize, usize)> {
    let (k, q_hash) = frame;
    let mut hits = Vec::new();
    for slice in [
        index32.lookup_upper((q_hash >> UPPER_32_SHIFT) as u32),
        index32.lookup_lower((q_hash & LOWER_32_MASK) as u32),
    ] {
        if slice.len() > MAX_MIH_BUCKET_SIZE {
            continue;
        }
        for e in slice {
            let (m_idx, off) = catalog.resolve_global_index(e.frame_idx as usize);
            if off >= k {
                hits.push((m_idx, off - k));
            }
        }
    }
    hits
}

/// Direct search over `AHashCatalog` using 32-bit `AHashMih32Index` (Mono or Rayon).
///
/// # Arguments
/// * `req` - Direct search request ([`AhashDirectSearchReq`]).
/// * `index32` - Pre-built 32-bit MIH index over the dense catalog.
///
/// # Returns
/// `Option<AHashCatalogMatch>` when a verified cluster satisfies [`FINE_SEARCH_MAX_DISTANCE`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Probes every [`DIRECT_MIH_PROBE_STRIDE`]-th frame (Mono or Rayon), tallies `(media, shift)` votes, and verifies top clusters.
fn search_ahash_mih32_direct(
    req: &AhashDirectSearchReq<'_>,
    index32: &AHashMih32Index,
) -> Option<AHashCatalogMatch> {
    if req.query.len() < MIN_QUERY_FRAMES || req.catalog.total_frames == 0 {
        return None;
    }
    let probes: Vec<(usize, u64)> = req
        .query
        .iter()
        .enumerate()
        .step_by(DIRECT_MIH_PROBE_STRIDE)
        .map(|(k, &h)| (k, h))
        .collect();

    let mut v_map: HashMap<(usize, usize), u32> = HashMap::new();
    if req.parallel {
        let partials: Vec<Vec<(usize, usize)>> = probes
            .into_par_iter()
            .map(|f| collect_mih32_frame_hits(req.catalog, index32, f))
            .collect();
        for pair in partials.into_iter().flatten() {
            *v_map.entry(pair).or_default() += 1;
        }
    } else {
        for pair in probes
            .into_iter()
            .flat_map(|f| collect_mih32_frame_hits(req.catalog, index32, f))
        {
            *v_map.entry(pair).or_default() += 1;
        }
    }
    verify_top_vote_clusters(req, v_map)
}

/// Collects `(media_idx, shift)` hits for one query frame against 16-bit `MultiIndexHash64`.
///
/// # Arguments
/// * `catalog` - Dense `AHashCatalog`.
/// * `mih` - 16-bit 4-slot MIH index.
/// * `frame` - `(query_frame_offset, query_hash)` tuple.
///
/// # Returns
/// Vector of `(media_idx, shift_offset)` candidate hits.
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Probes all 4 16-bit tables, skipping buckets exceeding [`MAX_MIH_BUCKET_SIZE`].
fn collect_mih16_frame_hits(
    catalog: &AHashCatalog,
    mih: &dbsrt_index::MultiIndexHash64,
    frame: (usize, u64),
) -> Vec<(usize, usize)> {
    let (k, q_hash) = frame;
    let mut hits = Vec::new();
    for (t, table) in mih.tables.iter().enumerate() {
        let chunk = dbsrt_index::mih::extract_chunk(q_hash, t);
        let bucket = table.bucket_indices(chunk);
        if bucket.len() > MAX_MIH_BUCKET_SIZE {
            continue;
        }
        for &cand_idx in bucket {
            let (m_idx, off) = catalog.resolve_global_index(cand_idx as usize);
            if off >= k {
                hits.push((m_idx, off - k));
            }
        }
    }
    hits
}

/// Direct search over `AHashCatalog` using 16-bit `MultiIndexHash64` (Mono or Rayon).
///
/// # Arguments
/// * `req` - Direct search request ([`AhashDirectSearchReq`]).
///
/// # Returns
/// `Option<AHashCatalogMatch>` when a verified cluster satisfies [`FINE_SEARCH_MAX_DISTANCE`].
///
/// # Errors
/// Infallible.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Probes 16-bit MIH tables (Mono or Rayon), aggregates `(media, shift)` votes, and verifies top clusters.
fn search_ahash_mih16_direct(req: &AhashDirectSearchReq<'_>) -> Option<AHashCatalogMatch> {
    if req.query.len() < MIN_QUERY_FRAMES || req.catalog.total_frames == 0 {
        return None;
    }
    let mih = req.catalog.mih();
    let probes: Vec<(usize, u64)> = req
        .query
        .iter()
        .enumerate()
        .step_by(DIRECT_MIH_PROBE_STRIDE)
        .map(|(k, &h)| (k, h))
        .collect();

    let mut v_map: HashMap<(usize, usize), u32> = HashMap::new();
    if req.parallel {
        let partials: Vec<Vec<(usize, usize)>> = probes
            .into_par_iter()
            .map(|f| collect_mih16_frame_hits(req.catalog, mih, f))
            .collect();
        for pair in partials.into_iter().flatten() {
            *v_map.entry(pair).or_default() += 1;
        }
    } else {
        for pair in probes
            .into_iter()
            .flat_map(|f| collect_mih16_frame_hits(req.catalog, mih, f))
        {
            *v_map.entry(pair).or_default() += 1;
        }
    }
    verify_top_vote_clusters(req, v_map)
}

/// Evaluates the Coarse-to-Fine $K \in [1, 32]$ sweep and MIH-16 vs MIH-32 comparison.
fn evaluate_ahash_matrices(
    probes: &[PreparedVideoProbe],
    dense_catalog: &AHashCatalog,
) -> (Vec<AhashTechniqueMetrics>, Vec<AhashTechniqueMetrics>) {
    let mut c2f_results = Vec::new();
    let mut k1_gt: HashMap<(usize, usize), (String, i64)> = HashMap::new();

    for &k in &COARSE_TO_FINE_SWEEP_FACTORS {
        println!("  -> Evaluating aHash Coarse-to-Fine K={}...", k);
        let t_build = Instant::now();
        let c2f = CoarseToFineAHashIndex::build(dense_catalog, k);
        let build_ms = t_build.elapsed().as_secs_f64() * MILLIS_PER_SECOND;
        let frames = c2f.coarse_catalog.total_frames;
        let ram_mb = (frames * HASH_WORD_BYTES + frames * MIH32_TABLE_COUNT * MIH32_ENTRY_BYTES)
            as f64
            / BYTES_PER_MIB;
        let coarse_fps = DEFAULT_AUDIO_FPS_VAL / k as f64;
        let label = format!("aHash C2F (MIH-32) K={k}");
        let spec = AhashEvalSpec {
            name: &label,
            factor_k: k,
            coarse_fps,
            dense_catalog,
            index_frames: frames,
            index_ram_mb: ram_mb,
            index_build_ms: build_ms,
        };
        let metrics = run_single_ahash_technique(&spec, probes, &mut k1_gt, |q, fps| {
            let criteria = FineSearchCriteria {
                dense_query: q,
                dense_fps: fps,
                max_distance: FINE_SEARCH_MAX_DISTANCE,
            };
            c2f.search(dense_catalog, &criteria)
        });
        c2f_results.push(metrics);
    }

    let mih_results = evaluate_ahash_mih16_vs_mih32(probes, dense_catalog, &mut k1_gt);
    (c2f_results, mih_results)
}

/// Evaluates direct MIH-16 vs MIH-32 (Mono & Rayon) on Coarse ($K=16$) and Dense ($K=1$).
fn evaluate_ahash_mih16_vs_mih32(
    probes: &[PreparedVideoProbe],
    dense_catalog: &AHashCatalog,
    k1_gt: &mut HashMap<(usize, usize), (String, i64)>,
) -> Vec<AhashTechniqueMetrics> {
    let mut mih_results = Vec::new();
    let dense_frames = dense_catalog.total_frames;

    let t_m32 = Instant::now();
    let dense_mih32 = dense_catalog.mih32();
    let m32_build_ms = t_m32.elapsed().as_secs_f64() * MILLIS_PER_SECOND;
    let m32_ram_mb = (dense_frames * HASH_WORD_BYTES
        + dense_frames * MIH32_TABLE_COUNT * MIH32_ENTRY_BYTES) as f64
        / BYTES_PER_MIB;

    let t_m16 = Instant::now();
    let _ = dense_catalog.mih();
    let m16_build_ms = t_m16.elapsed().as_secs_f64() * MILLIS_PER_SECOND;
    let m16_ram_mb = (dense_frames * HASH_WORD_BYTES
        + MIH16_TABLE_COUNT * MIH16_BUCKET_COUNT * MIH16_BUCKET_HEADER_BYTES
        + dense_frames * MIH16_TABLE_COUNT * MIH16_INDEX_BYTES) as f64
        / BYTES_PER_MIB;

    let mut sample_probes: Vec<PreparedVideoProbe> = probes
        .iter()
        .filter(|p| p.in_ahash_catalog)
        .take(AHASH_DENSE_MIH_SAMPLE_VIDEOS)
        .cloned()
        .collect();
    if sample_probes.len() < AHASH_DENSE_MIH_SAMPLE_VIDEOS {
        let extra = AHASH_DENSE_MIH_SAMPLE_VIDEOS - sample_probes.len();
        sample_probes.extend(
            probes
                .iter()
                .filter(|p| !p.in_ahash_catalog && p.record.audio.track_count > 0)
                .take(extra)
                .cloned(),
        );
    }

    for (label, ram_mb, build_ms, is_m32, parallel) in [
        (
            "aHash Dense MIH-32 (Rayon)",
            m32_ram_mb,
            m32_build_ms,
            true,
            true,
        ),
        (
            "aHash Dense MIH-32 (Mono)",
            m32_ram_mb,
            m32_build_ms,
            true,
            false,
        ),
        (
            "aHash Dense MIH-16 (Rayon)",
            m16_ram_mb,
            m16_build_ms,
            false,
            true,
        ),
        (
            "aHash Dense MIH-16 (Mono)",
            m16_ram_mb,
            m16_build_ms,
            false,
            false,
        ),
    ] {
        println!("  -> Evaluating {}...", label);
        let spec = AhashEvalSpec {
            name: label,
            factor_k: 1,
            coarse_fps: DEFAULT_AUDIO_FPS_VAL,
            dense_catalog,
            index_frames: dense_frames,
            index_ram_mb: ram_mb,
            index_build_ms: build_ms,
        };
        let m = run_single_ahash_technique(&spec, &sample_probes, k1_gt, |q, fps| {
            let req = AhashDirectSearchReq {
                catalog: dense_catalog,
                query: q,
                audio_fps: fps,
                parallel,
            };
            if is_m32 {
                search_ahash_mih32_direct(&req, dense_mih32)
            } else {
                search_ahash_mih16_direct(&req)
            }
        });
        mih_results.push(m);
    }

    let lin_probes = &sample_probes[..sample_probes.len().min(AHASH_DENSE_LINEAR_SAMPLE_VIDEOS)];
    let lin_ram_mb = (dense_frames * HASH_WORD_BYTES) as f64 / BYTES_PER_MIB;
    println!("  -> Evaluating aHash Dense Linear O(N) (Rayon)...");
    let lin_spec = AhashEvalSpec {
        name: "aHash Dense Linear O(N) (Rayon)",
        factor_k: 1,
        coarse_fps: DEFAULT_AUDIO_FPS_VAL,
        dense_catalog,
        index_frames: dense_frames,
        index_ram_mb: lin_ram_mb,
        index_build_ms: 0.0,
    };
    mih_results.push(run_single_ahash_technique(
        &lin_spec,
        lin_probes,
        k1_gt,
        |q, fps| {
            let crit = AHashSearchCriteria::new(q, fps);
            dense_catalog.search_linear(&crit).into_iter().next()
        },
    ));
    mih_results
}

/// Prints formatted summary tables for visual and acoustic benchmark matrices.
fn print_matrix_tables(report: &ExhaustiveMatrixReport) {
    println!("\n==================================================================================================================================");
    println!(
        " MATRICE VISUELLE pHash SUR {} VRAIES VIDEOS ({} CUT WINDOWS, {} INDEXEES)",
        report.total_real_videos, report.total_cut_windows, report.indexed_in_phash_videos
    );
    println!("==================================================================================================================================");
    println!(
        "{:<44} | {:>8} | {:>9} | {:>9} | {:>9} | {:>8} | {:>8} | {:>8}",
        "Technique", "RAM (MB)", "Win Mean", "Win p50", "Win p95", "1W Rec%", "3W Rec%", "3W Prec%"
    );
    println!("----------------------------------------------------------------------------------------------------------------------------------");
    for m in &report.phash_techniques {
        println!(
            "{:<44} | {:>8.1} | {:>6.2} ms | {:>6.2} ms | {:>6.2} ms | {:>7.1}% | {:>7.1}% | {:>7.1}%",
            m.technique,
            m.index_ram_mb,
            m.win_latency_mean_ms,
            m.win_latency_p50_ms,
            m.win_latency_p95_ms,
            m.win1_recall_pct,
            m.win3_recall_pct,
            m.win3_precision_pct
        );
    }

    println!("\n==========================================================================================================================================");
    println!(
        " MATRICE ACOUSTIQUE aHash COARSE-TO-FINE K in [1..32] SUR {} VRAIES VIDEOS ({} CUT WINDOWS, {} INDEXEES)",
        report.total_real_videos, report.total_cut_windows, report.indexed_in_ahash_videos
    );
    println!("==========================================================================================================================================");
    println!(
        "{:<22} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8} | {:>7} | {:>7} | {:>7} | {:>7} | {:>7}",
        "Technique",
        "FPS",
        "RAM(MB)",
        "Build",
        "WinMean",
        "Win p95",
        "1W Rec%",
        "3W Rec%",
        "3W Prc%",
        "SNR(b)",
        "<=25ms%"
    );
    println!("------------------------------------------------------------------------------------------------------------------------------------------");
    for m in &report.ahash_coarse_to_fine_sweep {
        println!(
            "{:<22} | {:>8.2} | {:>8.1} | {:>5.0} ms | {:>5.2} ms | {:>5.2} ms | {:>6.1}% | {:>6.1}% | {:>6.1}% | {:>6.1}b | {:>6.1}%",
            m.technique,
            m.coarse_fps,
            m.index_ram_mb,
            m.index_build_ms,
            m.win_latency_mean_ms,
            m.win_latency_p95_ms,
            m.win1_recall_indexed_pct,
            m.win3_recall_indexed_pct,
            m.win3_precision_pct,
            m.mean_snr_bits,
            m.shift_within_25ms_pct
        );
    }

    println!("\n==========================================================================================================================================");
    println!(" COMPARAISON DIRECTE aHash DENSE (95.3M FRAMES) : LINEAR O(N) vs MIH-16 vs MIH-32 (MONO & RAYON)");
    println!("==========================================================================================================================================");
    for m in &report.ahash_mih_comparison {
        println!(
            "{:<34} | RAM: {:>7.1} MB | Build: {:>6.0} ms | WinMean: {:>7.2} ms | p95: {:>7.2} ms | 3W Rec: {:>5.1}% | 3W Prec: {:>5.1}%",
            m.technique,
            m.index_ram_mb,
            m.index_build_ms,
            m.win_latency_mean_ms,
            m.win_latency_p95_ms,
            m.win3_recall_indexed_pct,
            m.win3_precision_pct
        );
    }
    println!("==========================================================================================================================================\n");
}

/// Runs the exhaustive multi-technique matrix evaluation on extracted JSONL probes and writes JSON report.
///
/// # Arguments
/// * `dbphash` - Loaded memory-mapped `.dbphash` catalog.
/// * `dense_catalog` - Loaded dense `.ahash` catalog.
/// * `jsonl_path` - Path to JSONL file containing extracted multi-window `query_hashes`.
/// * `matrix_json_path` - Destination path for the consolidated JSON matrix report.
///
/// # Returns
/// `Result<ExhaustiveMatrixReport>` containing all evaluated metrics.
///
/// # Errors
/// Fails if `jsonl_path` cannot be read or `matrix_json_path` cannot be written.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. Loads all valid `MediaScaleBenchmarkRecord` entries with persisted `query_hashes`.
/// 2. Evaluates visual `pHash` configurations (`MIH-16` Mono/Rayon, `MIH-32` Mono/Rayon, `Linear`).
/// 3. Evaluates 12 `CoarseToFineAHashIndex` configurations ($K \in [1, 32]$) + 5 direct `MIH-16`/`MIH-32`/`Linear` configurations.
/// 4. Prints comparative tables and serializes the full report to `matrix_json_path`.
pub fn run_matrix_benchmark(
    dbphash: &MmapDbphash,
    dense_catalog: &AHashCatalog,
    jsonl_path: &Path,
    matrix_json_path: &Path,
) -> Result<ExhaustiveMatrixReport> {
    println!("\n=== STARTING EXHAUSTIVE MULTI-TECHNIQUE MATRIX EVALUATION ===");
    let probes = load_prepared_probes(jsonl_path, dbphash, dense_catalog)?;
    let total_videos = probes.len();
    let total_windows: usize = probes.iter().map(|p| p.record.phash.windows.len()).sum();
    let synology_videos = probes
        .iter()
        .filter(|p| p.record.media_path.contains("/Synology/"))
        .count();
    let freebox_videos = probes
        .iter()
        .filter(|p| p.record.media_path.contains("/Freebox/"))
        .count();
    let indexed_phash = probes.iter().filter(|p| p.in_phash_catalog).count();
    let indexed_ahash = probes.iter().filter(|p| p.in_ahash_catalog).count();

    println!(
        "Loaded {} real videos ({} cut windows | Synology: {}, Freebox: {} | Indexed pHash: {}, Indexed aHash: {})",
        total_videos, total_windows, synology_videos, freebox_videos, indexed_phash, indexed_ahash
    );

    let phash_techniques = evaluate_phash_matrix(&probes, dbphash);
    let (ahash_c2f, ahash_mih) = evaluate_ahash_matrices(&probes, dense_catalog);

    let report = ExhaustiveMatrixReport {
        total_real_videos: total_videos,
        total_cut_windows: total_windows,
        synology_videos,
        freebox_videos,
        indexed_in_phash_videos: indexed_phash,
        indexed_in_ahash_videos: indexed_ahash,
        phash_techniques,
        ahash_coarse_to_fine_sweep: ahash_c2f,
        ahash_mih_comparison: ahash_mih,
    };

    print_matrix_tables(&report);

    if let Some(parent) = matrix_json_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json_bytes = serde_json::to_string_pretty(&report)?;
    std::fs::write(matrix_json_path, json_bytes)?;
    println!("Matrix report written to {:?}", matrix_json_path);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bench_scale::{
        AhashMatchSummary, AhashWindowDetail, ContainerAudioMetadata, MediaScaleBenchmarkRecord,
        MultimodalAgreement, PhashMatchSummary, PhashWindowDetail,
    };
    use dbsrt_core::MediaMetadata;

    /// Builds synthetic `.dbphash` and `.ahash` fixtures inside `tmp` for matrix unit tests.
    ///
    /// # Arguments
    /// * `tmp` - Temporary test directory.
    ///
    /// # Returns
    /// Tuple `(MmapDbphash, AHashCatalog, Vec<u64>)` of loaded synthetic catalogs and 128 audio hashes.
    ///
    /// # Panics
    /// Panics if synthetic fixture creation fails.
    ///
    /// # Data Flow & Invariants
    /// Writes one visual entry (`Matrix.Show.S01E01`) and one 128-frame `.ahash` file.
    fn build_synthetic_matrix_catalogs(tmp: &Path) -> (MmapDbphash, AHashCatalog, Vec<u64>) {
        let dbphash_path = tmp.join("catalog.dbphash");
        let ahash_dir = tmp.join("ahash");
        std::fs::create_dir_all(&ahash_dir).unwrap();

        let mut builder = dbsrt_index::DbphashBuilder::new();
        let meta = MediaMetadata::parse("Matrix.Show.S01E01");
        let sparse = vec![(0x5555_aaaa_1234_5678_u64, 5_000_u32)];
        let cuts = vec![(0x5555_aaaa_1234_5678_u64, 5_000_u32)];
        builder.add_media(&meta, 30_000, 24.0, &sparse, &cuts);
        builder.write_to_file(&dbphash_path).unwrap();

        let (_, band_bounds) =
            dbsrt_core::ahash::HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
        let header = dbsrt_core::AhashHeader {
            version: 1,
            sample_rate: 5512,
            frame_size: 2048,
            hop_size: 64,
            num_bands: 33,
            spectral_lag: 2,
            frame_count: 128,
            band_bounds,
        };
        let a_hashes: Vec<u64> = (0..128)
            .map(|i| 0x9e37_79b9_7f4a_7c15_u64.wrapping_mul(i + 1))
            .collect();
        dbsrt_core::AhashFile::write_to_path(
            &ahash_dir.join("Matrix.Show.S01E01.86.1fps.ahash"),
            &header,
            &a_hashes,
        )
        .unwrap();

        let dbphash = MmapDbphash::open(&dbphash_path).unwrap();
        let dense = AHashCatalog::load_from_dir(&ahash_dir).unwrap();
        (dbphash, dense, a_hashes)
    }

    #[test]
    fn test_percentiles_prf1_and_title_matching() {
        assert_eq!(percentile_f64(&[], 0.50), 0.0);
        assert_eq!(percentile_f64(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.50), 3.0);
        assert_eq!(percentile_i64(&[], 0.95), 0);
        assert_eq!(percentile_i64(&[10, 20, 30, 40, 100], 0.50), 30);

        let (fn_cnt, prec, rec, f1) = compute_prf1(80, 20, 100);
        assert_eq!(fn_cnt, 20);
        assert!((prec - 80.0).abs() < 1e-6);
        assert!((rec - 80.0).abs() < 1e-6);
        assert!((f1 - 80.0).abs() < 1e-6);

        let gt = "See.S01E04.1080p.WEB";
        assert!(are_titles_matching("See", (Some(1), Some(4)), gt));
        assert!(!are_titles_matching("See", (Some(1), Some(5)), gt));
        assert!(!are_titles_matching("Foundation", (Some(1), Some(4)), gt));
    }

    #[test]
    fn test_run_matrix_benchmark_end_to_end_synthetic() {
        let tmp = tempfile::tempdir().unwrap();
        let (dbphash, dense, a_hashes) = build_synthetic_matrix_catalogs(tmp.path());
        let rec = MediaScaleBenchmarkRecord {
            timestamp: "2026-11-14T00:00:00Z".to_string(),
            media_path: "/Synology/tvshow/Matrix.Show.S01E01.mkv".to_string(),
            media_stem: "Matrix.Show.S01E01".to_string(),
            duration_sec: 30.0,
            fps: 24.0,
            audio: ContainerAudioMetadata {
                track_count: 1,
                ..Default::default()
            },
            phash: PhashMatchSummary {
                media_match: Some("Matrix Show".to_string()),
                media_id: Some(1),
                season: Some(1),
                episode: Some(1),
                confidence: 100.0,
                consensus_shift_ms: 0,
                matching_windows: 1,
                total_windows: 1,
                is_coherent: true,
                timer_sec: 0.01,
                windows: vec![PhashWindowDetail {
                    window_idx: 0,
                    ratio: 0.5,
                    start_sec: 5.0,
                    matched_media: Some("Matrix Show".to_string()),
                    matched_media_id: Some(1),
                    matching_cuts: 1,
                    matching_sparse: 1,
                    estimated_shift_ms: 0,
                    confidence: 100.0,
                    query_hashes: vec![0x5555_aaaa_1234_5678_u64; 48],
                }],
            },
            ahash: AhashMatchSummary {
                media_match: Some("Matrix.Show.S01E01".to_string()),
                media_id: Some(1),
                confidence: 100.0,
                consensus_shift_ms: 0,
                matching_windows: 1,
                total_windows: 1,
                is_coherent: true,
                timer_sec: 0.01,
                windows: vec![AhashWindowDetail {
                    window_idx: 0,
                    ratio: 0.5,
                    start_sec: 0.0,
                    hashes_count: 64,
                    ones_ratio: 0.5,
                    is_low_energy: false,
                    coarse_candidate: Some("Matrix.Show.S01E01".to_string()),
                    fine_distance: Some(0.0),
                    fine_snr_bits: Some(32.0),
                    ref_offset_sec: Some(0.0),
                    estimated_shift_ms: Some(0),
                    confidence: 100.0,
                    audio_fps: 86.125,
                    query_hashes: a_hashes[..64].to_vec(),
                }],
            },
            agreement: MultimodalAgreement {
                match_agreed: true,
                shift_diff_ms: Some(0),
                status: "DUAL_MATCH_AGREED".to_string(),
            },
            probable_cause: "NOMINAL_MULTIMODAL_MATCH".to_string(),
            diagnostic_note: String::new(),
            total_timer_sec: 0.02,
        };
        let jsonl_path = tmp.path().join("probes.jsonl");
        std::fs::write(
            &jsonl_path,
            format!("{}\n", serde_json::to_string(&rec).unwrap()),
        )
        .unwrap();
        let matrix_path = tmp.path().join("matrix.json");
        let report = run_matrix_benchmark(&dbphash, &dense, &jsonl_path, &matrix_path).unwrap();
        assert_eq!(report.total_real_videos, 1);
        assert_eq!(report.phash_techniques.len(), 7);
        assert_eq!(report.ahash_coarse_to_fine_sweep.len(), 12);
        assert_eq!(report.ahash_mih_comparison.len(), 5);
        assert!(matrix_path.exists());
    }
}
