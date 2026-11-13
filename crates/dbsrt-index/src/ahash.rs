//! In-memory acoustic fingerprint catalog and parallel search engine.
//!
//! Provides multi-media indexing, parallel cross-correlation search via Rayon,
//! and candidate ranking for 64-bit Haitsma-Kalker audio fingerprints.

use anyhow::{Context, Result};
use dbsrt_core::ahash::{match_audio_sequence, AhashFile, DEFAULT_MAX_AUDIO_MATCH_DISTANCE};
use dbsrt_core::{ahash_logical_stem, ahash_sidecar_path, AhashSidecar};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::mih::{extract_chunk, HashProvider, MihTable, MultiIndexHash64, NUM_TABLES};

/// Default maximum number of match candidates to return.
pub const DEFAULT_AHASH_SEARCH_LIMIT: usize = 5;

/// Maximum catalog size (in cumulative sub-fingerprint frames) for which [`AHashCatalog::search`]
/// still allows an exhaustive linear fallback after an empty 32-bit MIH result.
/// 500 000 frames at ~86.1 fps is about 96 minutes of cumulated audio; above this threshold a
/// non-match returns immediately instead of scanning (or building the 16-bit MIH over) the catalog.
pub const MAX_LINEAR_FALLBACK_FRAMES: usize = 500_000;

/// Milliseconds per second conversion factor.
pub const MILLIS_PER_SEC: f64 = 1000.0;

/// Standard file extension for binary acoustic fingerprints.
pub const AHASH_FILE_EXTENSION: &str = "ahash";

/// Minimum query frame length in sub-fingerprints for valid matching.
pub const MIN_QUERY_FRAMES: usize = 8;

/// Floating-point framerate compatibility tolerance in FPS.
pub const FPS_COMPATIBILITY_TOLERANCE: f64 = 1.0;

/// Default probe stride in frames for MIH acoustic candidate generation (sample every 4 frames ~21.5 Hz).
pub const DEFAULT_AHASH_MIH_PROBE_STRIDE: usize = 4;

/// Default maximum Hamming distance for candidate generation in MIH acoustic search (radius 0 on 16-bit blocks).
pub const DEFAULT_AHASH_MIH_SEARCH_DISTANCE: u32 = 3;

/// Minimum coherent votes required to consider an acoustic candidate cluster valid.
pub const DEFAULT_AHASH_MIH_MIN_VOTES: u32 = 3;

/// Maximum number of candidate clusters evaluated during fine verification.
pub const DEFAULT_AHASH_MIH_MAX_CANDIDATES: usize = 5;

/// Sub-frame jitter search radius in frames (+/- 1 frame) during fine verification.
pub const DEFAULT_AHASH_MIH_JITTER_RADIUS: usize = 1;

/// Default probe stride in frames for 32-bit MIH acoustic candidate generation (sample every 4 frames ~128 ms).
pub const DEFAULT_AHASH_MIH32_PROBE_STRIDE: usize = 4;

/// Minimum coherent votes required for 32-bit MIH candidate clusters.
pub const DEFAULT_AHASH_MIH32_MIN_VOTES: u32 = 3;

/// Maximum bucket size to consider in 32-bit MIH to filter background hum.
pub const DEFAULT_AHASH_MIH32_MAX_BUCKET_SIZE: usize = 20_000;

/// Parses the canonical media name from an .ahash filename.
///
/// Strips `.ahash` and optional trailing `.<rate>fps` tokens.
///
/// # Arguments
/// * `filename` - Raw filename or file stem string.
///
/// # Returns
/// Clean media stem or movie/episode name.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Delegates to [`dbsrt_core::ahash_logical_stem`], the single naming rule shared with the
/// `<stem>.json` sidecar writer.
pub fn parse_ahash_media_name(filename: &str) -> String {
    ahash_logical_stem(filename).to_string()
}

/// Length in hexadecimal characters of an OpenSubtitles 64-bit MovieHash stem.
const MOVIEHASH_HEX_LEN: usize = 16;

/// Name of the sibling pHash directory holding `<moviehash>.json` sidecars.
const PHASH_SIDECAR_DIRNAME: &str = "__PHASH__";

/// JSON sidecar keys probed (in priority order) for the canonical media name.
const SIDECAR_NAME_KEYS: [&str; 3] = ["media_name", "clean_title", "filename"];

/// Returns true when `s` is a raw 16-character hexadecimal MovieHash.
pub fn is_moviehash_hex(s: &str) -> bool {
    s.len() == MOVIEHASH_HEX_LEN && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Reads the first non-empty name key from a JSON sidecar file.
fn read_sidecar_name(path: &Path) -> Option<String> {
    let txt = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&txt).ok()?;
    SIDECAR_NAME_KEYS.iter().find_map(|key| {
        let name = value.get(key)?.as_str()?.trim();
        (!name.is_empty()).then(|| name.to_string())
    })
}

/// Resolves a canonical media name from `<stem>.json` for MovieHash-named files.
///
/// Searches `dir` first (aHash sidecars), then `fallback_dir` (`../__PHASH__`).
/// Returns `None` when `stem` is not a 16-hex hash or no sidecar yields a name.
fn resolve_sidecar_media_name(
    stem: &str,
    dir: &Path,
    fallback_dir: Option<&Path>,
) -> Option<String> {
    if !is_moviehash_hex(stem) {
        return None;
    }
    let file = format!("{stem}.json");
    std::iter::once(dir)
        .chain(fallback_dir)
        .find_map(|d| read_sidecar_name(&d.join(&file)))
}

/// Loads one `.ahash` file into a catalog entry, resolving its name and audio track.
///
/// # Arguments
/// * `path` - `.ahash` binary path.
/// * `dir` - Directory being scanned (holds `<stem>.json` aHash sidecars).
/// * `phash_dir` - Sibling `__PHASH__` directory for legacy `<moviehash>.json` fallback.
///
/// # Returns
/// `None` when the binary is unreadable or empty; otherwise the populated entry.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Typed [`AhashSidecar`] at [`ahash_sidecar_path`]: non-empty `media_name` and
///    `selected_audio_stream` (`audio_language`, `audio_index`) win.
/// 2. Otherwise legacy name-only lookup ([`resolve_sidecar_media_name`], 16-hex stems only).
/// 3. Otherwise the parsed file stem. Audio fields stay `None` without a typed sidecar.
fn load_catalog_entry(
    path: PathBuf,
    dir: &Path,
    phash_dir: Option<&Path>,
) -> Option<AHashCatalogEntry> {
    let file = AhashFile::read_from_path(&path).ok()?;
    if file.hashes.is_empty() {
        return None;
    }
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let parsed_name = parse_ahash_media_name(filename);
    let typed = AhashSidecar::read(&ahash_sidecar_path(&path)).ok();
    let typed_name = typed
        .as_ref()
        .map(|s| s.media_name.trim())
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    let media_name = typed_name
        .or_else(|| resolve_sidecar_media_name(&parsed_name, dir, phash_dir))
        .unwrap_or(parsed_name);
    let track = typed.and_then(|s| s.selected_audio_stream);
    Some(AHashCatalogEntry {
        media_name,
        audio_fps: file.header.audio_fps(),
        frame_count: file.hashes.len(),
        hashes: Arc::new(file.hashes),
        audio_language: track.as_ref().and_then(|t| t.language.clone()),
        audio_index: track.map(|t| t.audio_index),
        path,
    })
}

/// Collapses binary twins (identical `frame_count` + `hashes`) into one entry.
///
/// Preference: resolved (non raw-hex) name first, then lexicographic `path`.
/// Output order follows the first-seen position of each twin group, so a
/// path-sorted input yields a deterministic path-sorted output.
fn dedup_twin_entries(entries: Vec<AHashCatalogEntry>) -> Vec<AHashCatalogEntry> {
    use std::hash::{Hash, Hasher};
    let rank = |e: &AHashCatalogEntry| (is_moviehash_hex(&e.media_name), e.path.clone());
    let mut kept: Vec<AHashCatalogEntry> = Vec::with_capacity(entries.len());
    let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
    for entry in entries {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            entry.frame_count,
            entry.hashes.first(),
            entry.hashes.get(entry.frame_count / 2),
            entry.hashes.last(),
        )
            .hash(&mut hasher);
        let bucket = buckets.entry(hasher.finish()).or_default();
        let twin = bucket
            .iter()
            .copied()
            .find(|&i| kept[i].frame_count == entry.frame_count && kept[i].hashes == entry.hashes);
        match twin {
            Some(i) if rank(&entry) < rank(&kept[i]) => kept[i] = entry,
            Some(_) => {}
            None => {
                bucket.push(kept.len());
                kept.push(entry);
            }
        }
    }
    kept
}

/// Single indexed reference media entry in the acoustic catalog.
#[derive(Debug, Clone)]
pub struct AHashCatalogEntry {
    /// Stem or clean title of the reference media.
    pub media_name: String,
    /// Absolute or relative path to the .ahash file.
    pub path: PathBuf,
    /// Sampling framerate in frames per second (e.g. 86.125).
    pub audio_fps: f64,
    /// Total count of 64-bit sub-fingerprint frames.
    pub frame_count: usize,
    /// Complete sequence of 64-bit Haitsma-Kalker hashes.
    pub hashes: Arc<Vec<u64>>,
    /// ISO language tag of the decoded audio track (from the typed `<stem>.json` sidecar).
    pub audio_language: Option<String>,
    /// Relative audio index (`k` in `0:a:k`) of the decoded track, when known.
    pub audio_index: Option<usize>,
}

/// Search criteria parameters for querying an [`AHashCatalog`].
#[derive(Debug, Clone)]
pub struct AHashSearchCriteria<'a> {
    /// Query snippet hashes to match against reference streams.
    pub query: &'a [u64],
    /// Sampling framerate of the query audio hashes (default: ~86.125).
    pub audio_fps: f64,
    /// Maximum allowed mean Hamming distance per frame (default: 14.0).
    pub max_distance: f64,
    /// Maximum number of top candidates to return.
    pub limit: usize,
    /// Optional target media name filter (matches only this media if provided).
    pub target_media: Option<&'a str>,
}

impl<'a> AHashSearchCriteria<'a> {
    /// Constructs default search criteria for a given query hash slice.
    ///
    /// # Arguments
    /// * `query` - Contiguous slice of 64-bit sub-fingerprint hashes.
    /// * `audio_fps` - Framerate of the query snippet.
    ///
    /// # Returns
    /// Initialized [`AHashSearchCriteria`] with default thresholds.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Applies [`DEFAULT_MAX_AUDIO_MATCH_DISTANCE`] and [`DEFAULT_AHASH_SEARCH_LIMIT`].
    pub fn new(query: &'a [u64], audio_fps: f64) -> Self {
        Self {
            query,
            audio_fps,
            max_distance: DEFAULT_MAX_AUDIO_MATCH_DISTANCE,
            limit: DEFAULT_AHASH_SEARCH_LIMIT,
            target_media: None,
        }
    }
}

/// Match candidate resulting from acoustic cross-correlation search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AHashCatalogMatch {
    /// Stem or clean title of the reference media.
    pub media_name: String,
    /// Path to the matched reference .ahash file.
    pub file_path: String,
    /// Optimal matching frame index in the reference stream.
    pub ref_frame_offset: usize,
    /// Estimated time offset in seconds in the reference media.
    pub offset_sec: f64,
    /// Estimated time offset in milliseconds.
    pub offset_ms: i64,
    /// Average Hamming distance across the query window.
    pub mean_distance: f64,
    /// Normalized match score (0.0 to 1.0).
    pub score: f64,
    /// SNR bits margin above random noise (32.0 bits).
    pub snr_bits: f64,
    /// Audio framerate of the reference media.
    pub audio_fps: f64,
}

/// Compact 32-bit chunk entry for sorted index lookups.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mih32Entry {
    /// 32-bit chunk prefix or suffix.
    pub chunk: u32,
    /// Flat global frame index in catalog.
    pub frame_idx: u32,
}

/// Dual-table 32-bit Multi-Index Hash index for acoustic fingerprints.
#[derive(Debug, Clone, Default)]
pub struct AHashMih32Index {
    /// Upper 32-bit sorted chunk table.
    pub upper_table: Vec<Mih32Entry>,
    /// Lower 32-bit sorted chunk table.
    pub lower_table: Vec<Mih32Entry>,
}

impl AHashMih32Index {
    /// Compiles 32-bit Multi-Index Hash tables across all flat hashes in parallel.
    ///
    /// # Arguments
    /// * `flat_hashes` - Slice of all 64-bit sub-fingerprint hashes.
    ///
    /// # Returns
    /// Compiled [`AHashMih32Index`] with upper and lower sorted tables.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Uses `rayon::join` to sort upper 32-bit and lower 32-bit halves in parallel.
    pub fn build(flat_hashes: &[u64]) -> Self {
        let (upper, lower) = rayon::join(
            || {
                let mut v: Vec<Mih32Entry> = flat_hashes
                    .par_iter()
                    .enumerate()
                    .map(|(i, &h)| Mih32Entry {
                        chunk: (h >> 32) as u32,
                        frame_idx: i as u32,
                    })
                    .collect();
                v.par_sort_unstable_by_key(|e| e.chunk);
                v
            },
            || {
                let mut v: Vec<Mih32Entry> = flat_hashes
                    .par_iter()
                    .enumerate()
                    .map(|(i, &h)| Mih32Entry {
                        chunk: (h & 0xFFFF_FFFF) as u32,
                        frame_idx: i as u32,
                    })
                    .collect();
                v.par_sort_unstable_by_key(|e| e.chunk);
                v
            },
        );

        Self {
            upper_table: upper,
            lower_table: lower,
        }
    }

    /// Performs binary search lookup on upper 32-bit chunk table.
    ///
    /// # Arguments
    /// * `chunk` - 32-bit chunk key to query.
    ///
    /// # Returns
    /// Contiguous slice of matching [`Mih32Entry`] items.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Uses `partition_point` for sub-microsecond binary search range determination.
    #[inline(always)]
    pub fn lookup_upper(&self, chunk: u32) -> &[Mih32Entry] {
        Self::lookup_slice(&self.upper_table, chunk)
    }

    /// Performs binary search lookup on lower 32-bit chunk table.
    ///
    /// # Arguments
    /// * `chunk` - 32-bit chunk key to query.
    ///
    /// # Returns
    /// Contiguous slice of matching [`Mih32Entry`] items.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Uses `partition_point` for sub-microsecond binary search range determination.
    #[inline(always)]
    pub fn lookup_lower(&self, chunk: u32) -> &[Mih32Entry] {
        Self::lookup_slice(&self.lower_table, chunk)
    }

    #[inline(always)]
    fn lookup_slice(table: &[Mih32Entry], chunk: u32) -> &[Mih32Entry] {
        let start = table.partition_point(|e| e.chunk < chunk);
        if start < table.len() && table[start].chunk == chunk {
            let end = table[start..].partition_point(|e| e.chunk <= chunk) + start;
            &table[start..end]
        } else {
            &[]
        }
    }
}

/// In-memory catalog of acoustic fingerprints supporting accelerated MIH and cross-correlation search.
#[derive(Debug, Clone, Default)]
pub struct AHashCatalog {
    /// Indexed reference media entries.
    pub entries: Vec<AHashCatalogEntry>,
    /// Cumulative sub-fingerprint frame count across all entries.
    pub total_frames: usize,
    /// Prefix sum offsets mapping global frame indices to individual media entries.
    pub media_offsets: Vec<usize>,
    /// Contiguous flat array of all 64-bit sub-fingerprint hashes for zero-overhead MIH lookup.
    pub flat_hashes: Arc<Vec<u64>>,
    /// Lazily compiled Multi-Index Hash table across all acoustic sub-fingerprints.
    pub mih: Arc<OnceLock<MultiIndexHash64>>,
    /// Lazily compiled 32-bit Multi-Index Hash table across all acoustic sub-fingerprints.
    pub mih32: Arc<OnceLock<AHashMih32Index>>,
}

impl HashProvider for AHashCatalog {
    #[inline(always)]
    fn get_hash(&self, index: usize) -> u64 {
        self.flat_hashes[index]
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.total_frames
    }
}

impl AHashCatalog {
    /// Creates an empty acoustic catalog.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// An empty [`AHashCatalog`].
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Allocates empty entry list with zero total frames.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            total_frames: 0,
            media_offsets: vec![0],
            flat_hashes: Arc::new(Vec::new()),
            mih: Arc::new(OnceLock::new()),
            mih32: Arc::new(OnceLock::new()),
        }
    }

    /// Constructs a catalog from an existing vector of entries.
    ///
    /// # Arguments
    /// * `entries` - Vector of [`AHashCatalogEntry`] items.
    ///
    /// # Returns
    /// Initialized [`AHashCatalog`].
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Evaluates cumulative total frame count and compiles flat hash slice for accelerated lookup.
    pub fn new(entries: Vec<AHashCatalogEntry>) -> Self {
        let mut total_frames = 0;
        let mut media_offsets = Vec::with_capacity(entries.len() + 1);
        for entry in &entries {
            media_offsets.push(total_frames);
            total_frames += entry.frame_count;
        }
        media_offsets.push(total_frames);

        let mut flat = Vec::with_capacity(total_frames);
        for entry in &entries {
            flat.extend_from_slice(&entry.hashes);
        }

        Self {
            entries,
            total_frames,
            media_offsets,
            flat_hashes: Arc::new(flat),
            mih: Arc::new(OnceLock::new()),
            mih32: Arc::new(OnceLock::new()),
        }
    }

    /// Resolves global flat frame index to specific `(media_index, local_frame_offset)`.
    ///
    /// # Arguments
    /// * `index` - Global frame index within `[0, total_frames)`.
    ///
    /// # Returns
    /// Tuple of `(media_index, local_frame_offset)`.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Uses binary search on prefix offsets for sub-microsecond resolution.
    #[inline(always)]
    pub fn resolve_global_index(&self, index: usize) -> (usize, usize) {
        if self.entries.is_empty() || self.media_offsets.is_empty() {
            return (0, 0);
        }
        let media_idx = match self.media_offsets.binary_search(&index) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let bounded_media_idx = media_idx.min(self.entries.len().saturating_sub(1));
        let base_offset = self
            .media_offsets
            .get(bounded_media_idx)
            .copied()
            .unwrap_or(0);
        let local_offset = index.saturating_sub(base_offset);
        (bounded_media_idx, local_offset)
    }

    /// Compiles 64-bit Multi-Index Hash (MIH) across all catalog sub-fingerprints in parallel.
    ///
    /// # Arguments
    /// None (operates on `&self`).
    ///
    /// # Returns
    /// Compiled [`MultiIndexHash64`] with 4 16-bit CSR tables.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Scans contiguous chunks in parallel across 4 tables via Rayon.
    pub fn build_mih(&self) -> MultiIndexHash64 {
        let n = self.total_frames;
        let flat = &self.flat_hashes;
        let tables: Vec<MihTable> = (0..NUM_TABLES)
            .into_par_iter()
            .map(|t| {
                let mut chunks = Vec::with_capacity(n);
                for &h in flat.iter() {
                    chunks.push(extract_chunk(h, t));
                }
                MihTable::build(&chunks)
            })
            .collect();

        MultiIndexHash64 {
            len: n,
            tables: [
                tables[0].clone(),
                tables[1].clone(),
                tables[2].clone(),
                tables[3].clone(),
            ],
        }
    }

    /// Accesses or lazily builds the shared Multi-Index Hash (MIH) index.
    ///
    /// # Arguments
    /// None (operates on `&self`).
    ///
    /// # Returns
    /// Reference to compiled [`MultiIndexHash64`].
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Lazily initialized via `OnceLock::get_or_init`.
    pub fn mih(&self) -> &MultiIndexHash64 {
        self.mih.get_or_init(|| self.build_mih())
    }

    /// Accesses or lazily builds the shared 32-bit Multi-Index Hash (MIH-32) index.
    ///
    /// # Arguments
    /// None (operates on `&self`).
    ///
    /// # Returns
    /// Reference to compiled [`AHashMih32Index`].
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Lazily initialized via `OnceLock::get_or_init`.
    pub fn mih32(&self) -> &AHashMih32Index {
        self.mih32
            .get_or_init(|| AHashMih32Index::build(&self.flat_hashes))
    }

    /// Loads and parses all `.ahash` files found in the specified directory.
    ///
    /// # Arguments
    /// * `dir` - Filesystem path to scan for `.ahash` files.
    ///
    /// # Returns
    /// Initialized [`AHashCatalog`] populated with parsed entries.
    ///
    /// # Errors
    /// Returns `Err` if the directory cannot be accessed.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Scans directory entries, filters by `.ahash` extension, and parses binary files
    /// in parallel using Rayon ([`load_catalog_entry`]: typed `<stem>.json` sidecar first,
    /// legacy name-only sidecars as fallback) before twin dedup and aggregation.
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        if !dir.exists() {
            return Ok(Self::empty());
        }

        let read_dir = fs::read_dir(dir).with_context(|| format!("Reading {:?}", dir))?;
        let mut paths = Vec::new();
        for entry in read_dir.flatten() {
            let p = entry.path();
            if p.is_file() && p.extension().is_some_and(|e| e == AHASH_FILE_EXTENSION) {
                paths.push(p);
            }
        }
        paths.sort();

        let sibling_phash_dir = dir.parent().map(|p| p.join(PHASH_SIDECAR_DIRNAME));
        let entries: Vec<AHashCatalogEntry> = paths
            .into_par_iter()
            .filter_map(|p| load_catalog_entry(p, dir, sibling_phash_dir.as_deref()))
            .collect();

        Ok(Self::new(dedup_twin_entries(entries)))
    }

    /// Adds a single entry to the catalog in place.
    ///
    /// # Arguments
    /// * `entry` - New [`AHashCatalogEntry`] to insert.
    ///
    /// # Returns
    /// Nothing (`()`).
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Appends entry, updates offsets, and resets MIH index.
    pub fn add_entry(&mut self, entry: AHashCatalogEntry) {
        if self.media_offsets.is_empty() {
            self.media_offsets.push(0);
        }
        self.total_frames += entry.frame_count;
        self.media_offsets.push(self.total_frames);
        Arc::make_mut(&mut self.flat_hashes).extend_from_slice(&entry.hashes);
        self.entries.push(entry);
        self.mih = Arc::new(OnceLock::new());
        self.mih32 = Arc::new(OnceLock::new());
    }

    /// Probes all MIH tables for a single query frame and returns valid shift candidates.
    ///
    /// # Arguments
    /// * `mih` - Multi-index hash structure containing bucket partitions.
    /// * `k` - Query frame index offset within the query sequence.
    /// * `q_hash` - 64-bit acoustic hash of the probe frame.
    ///
    /// # Returns
    /// Vector of candidate tuples `(media_index, candidate_shift)`.
    ///
    /// # Errors
    /// This function does not return `Result` and cannot error.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// For each of the 4 chunk tables, retrieves matching bucket entries and converts
    /// flat global indexes into `(media_idx, local_frame_offset)`.
    fn probe_frame_hits(
        &self,
        mih: &MultiIndexHash64,
        k: usize,
        q_hash: u64,
    ) -> Vec<(usize, usize)> {
        let mut hits = Vec::new();
        for (t, table) in mih.tables.iter().enumerate() {
            let chunk = extract_chunk(q_hash, t);
            for &cand_idx in table.bucket_indices(chunk) {
                let (media_idx, ref_offset) = self.resolve_global_index(cand_idx as usize);
                if ref_offset >= k {
                    hits.push((media_idx, ref_offset - k));
                }
            }
        }
        hits
    }

    /// Collects candidate shift votes in parallel via direct bucket index retrieval.
    fn collect_mih_candidate_votes(
        &self,
        query: &[u64],
        stride: usize,
    ) -> HashMap<(usize, usize), u32> {
        let mih = self.mih();
        let probe_frames: Vec<(usize, u64)> = query
            .iter()
            .enumerate()
            .step_by(stride)
            .map(|(k, &h)| (k, h))
            .collect();

        let partial_votes: Vec<Vec<(usize, usize)>> = probe_frames
            .par_iter()
            .map(|&(k, q_hash)| self.probe_frame_hits(mih, k, q_hash))
            .collect();

        let mut votes = HashMap::new();
        for hits in partial_votes {
            for pair in hits {
                *votes.entry(pair).or_default() += 1;
            }
        }
        votes
    }

    /// Filters and ranks candidate clusters by temporal vote density with jitter aggregation.
    fn rank_candidate_clusters(
        votes: &HashMap<(usize, usize), u32>,
        min_votes: u32,
        max_candidates: usize,
    ) -> Vec<(usize, usize, u32)> {
        let mut clusters: Vec<(usize, usize, u32)> = votes
            .iter()
            .filter_map(|(&(media_idx, shift), &v)| {
                let v_prev = shift
                    .checked_sub(1)
                    .and_then(|prev| votes.get(&(media_idx, prev)))
                    .copied()
                    .unwrap_or(0);
                let v_next = votes.get(&(media_idx, shift + 1)).copied().unwrap_or(0);
                let total_v = v + v_prev + v_next;
                if total_v >= min_votes {
                    Some((media_idx, shift, total_v))
                } else {
                    None
                }
            })
            .collect();

        clusters.sort_by_key(|a| std::cmp::Reverse(a.2));

        let mut deduplicated: Vec<(usize, usize, u32)> = Vec::new();
        for (m_idx, s, score) in clusters {
            let already_covered = deduplicated.iter().any(|&(dm, ds, _)| {
                dm == m_idx
                    && (s as isize - ds as isize).abs() <= DEFAULT_AHASH_MIH_JITTER_RADIUS as isize
            });
            if !already_covered {
                deduplicated.push((m_idx, s, score));
                if deduplicated.len() >= max_candidates {
                    break;
                }
            }
        }
        deduplicated
    }

    /// Performs sub-frame fine jitter verification across candidate clusters.
    fn verify_candidate_clusters(
        &self,
        candidates: &[(usize, usize, u32)],
        criteria: &AHashSearchCriteria,
    ) -> Vec<AHashCatalogMatch> {
        let q_len = criteria.query.len();
        let mut matches = Vec::new();

        for &(media_idx, coarse_shift, _score) in candidates {
            let entry = &self.entries[media_idx];
            if (entry.audio_fps - criteria.audio_fps).abs() > FPS_COMPATIBILITY_TOLERANCE {
                continue;
            }

            let ref_len = entry.hashes.len();
            if ref_len < q_len {
                continue;
            }

            let min_s = coarse_shift.saturating_sub(DEFAULT_AHASH_MIH_JITTER_RADIUS);
            let max_s = (coarse_shift + DEFAULT_AHASH_MIH_JITTER_RADIUS).min(ref_len - q_len);

            let mut best_shift = coarse_shift;
            let mut best_dist = u64::MAX;

            for s in min_s..=max_s {
                let ref_slice = &entry.hashes[s..s + q_len];
                let mut dist = 0u64;
                for (&q, &r) in criteria.query.iter().zip(ref_slice) {
                    dist += (q ^ r).count_ones() as u64;
                }
                if dist < best_dist {
                    best_dist = dist;
                    best_shift = s;
                }
            }

            let mean_dist = best_dist as f64 / q_len as f64;
            if mean_dist <= criteria.max_distance {
                let score = (1.0 - (mean_dist / 64.0)).clamp(0.0, 1.0);
                let snr_bits = (32.0 - mean_dist).max(0.0);
                let offset_sec = best_shift as f64 / entry.audio_fps;
                matches.push(AHashCatalogMatch {
                    media_name: entry.media_name.clone(),
                    file_path: entry.path.to_string_lossy().to_string(),
                    ref_frame_offset: best_shift,
                    offset_sec,
                    offset_ms: (offset_sec * MILLIS_PER_SEC).round() as i64,
                    mean_distance: mean_dist,
                    score,
                    snr_bits,
                    audio_fps: entry.audio_fps,
                });
            }
        }

        matches.sort_by(|a, b| {
            a.mean_distance
                .partial_cmp(&b.mean_distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut dedup_matches: Vec<AHashCatalogMatch> = Vec::new();
        for m in matches {
            if !dedup_matches
                .iter()
                .any(|existing| existing.media_name == m.media_name)
            {
                dedup_matches.push(m);
            }
        }

        if criteria.limit > 0 && dedup_matches.len() > criteria.limit {
            dedup_matches.truncate(criteria.limit);
        }

        dedup_matches
    }

    /// Searches the catalog using accelerated Multi-Index Hashing (MIH) with fallback.
    ///
    /// # Arguments
    /// * `criteria` - Search parameters, query slice, thresholds and limits.
    ///
    /// # Returns
    /// Vector of [`AHashCatalogMatch`] items ranked by Hamming distance.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Probes MIH tables at step intervals for candidate cluster voting.
    /// 2. Verifies top candidate clusters with local jitter refinement.
    /// 3. Falls back seamlessly to exhaustive linear scan if no cluster reaches vote threshold.
    pub fn search_mih(&self, criteria: &AHashSearchCriteria) -> Vec<AHashCatalogMatch> {
        if criteria.query.len() < MIN_QUERY_FRAMES || self.total_frames == 0 {
            return Vec::new();
        }

        if criteria.target_media.is_some() {
            return self.search_linear(criteria);
        }

        let votes =
            self.collect_mih_candidate_votes(criteria.query, DEFAULT_AHASH_MIH_PROBE_STRIDE);

        let candidates = Self::rank_candidate_clusters(
            &votes,
            DEFAULT_AHASH_MIH_MIN_VOTES,
            DEFAULT_AHASH_MIH_MAX_CANDIDATES,
        );

        if candidates.is_empty() {
            return self.search_linear(criteria);
        }

        let verified = self.verify_candidate_clusters(&candidates, criteria);
        if verified.is_empty() {
            self.search_linear(criteria)
        } else {
            verified
        }
    }

    /// Searches the catalog in parallel using exhaustive linear cross-correlation (ground truth).
    ///
    /// # Arguments
    /// * `criteria` - Search parameters, query slice, thresholds and limits.
    ///
    /// # Returns
    /// Vector of [`AHashCatalogMatch`] items ranked by Hamming distance.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Iterates over all catalog entries in parallel via Rayon using [`match_audio_sequence`].
    pub fn search_linear(&self, criteria: &AHashSearchCriteria) -> Vec<AHashCatalogMatch> {
        if criteria.query.len() < MIN_QUERY_FRAMES {
            return Vec::new();
        }

        let mut matches: Vec<AHashCatalogMatch> = self
            .entries
            .par_iter()
            .filter(|entry| {
                if let Some(target) = criteria.target_media {
                    if !entry.media_name.eq_ignore_ascii_case(target) {
                        return false;
                    }
                }
                (entry.audio_fps - criteria.audio_fps).abs() <= FPS_COMPATIBILITY_TOLERANCE
            })
            .filter_map(|entry| {
                let match_opt = match_audio_sequence(
                    criteria.query,
                    &entry.hashes,
                    entry.audio_fps,
                    criteria.max_distance,
                );
                match_opt.map(|res| AHashCatalogMatch {
                    media_name: entry.media_name.clone(),
                    file_path: entry.path.to_string_lossy().to_string(),
                    ref_frame_offset: res.ref_frame_offset,
                    offset_sec: res.offset_sec,
                    offset_ms: (res.offset_sec * MILLIS_PER_SEC).round() as i64,
                    mean_distance: res.mean_distance,
                    score: res.score,
                    snr_bits: res.snr_bits,
                    audio_fps: entry.audio_fps,
                })
            })
            .collect();

        matches.sort_by(|a, b| {
            a.mean_distance
                .partial_cmp(&b.mean_distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut dedup_matches: Vec<AHashCatalogMatch> = Vec::new();
        for m in matches {
            if !dedup_matches
                .iter()
                .any(|existing| existing.media_name == m.media_name)
            {
                dedup_matches.push(m);
            }
        }

        if criteria.limit > 0 && dedup_matches.len() > criteria.limit {
            dedup_matches.truncate(criteria.limit);
        }

        dedup_matches
    }

    /// Collects candidate shift votes for a single probe frame using 32-bit chunk tables.
    ///
    /// # Arguments
    /// * `index` - Compiled [`AHashMih32Index`].
    /// * `k` - Query frame index offset.
    /// * `q_hash` - 64-bit query acoustic hash.
    ///
    /// # Returns
    /// Vector of candidate tuples `(media_index, candidate_shift)`.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Queries upper and lower 32-bit slices, filtering oversized buckets.
    fn probe_frame_hits_mih32(
        &self,
        index: &AHashMih32Index,
        k: usize,
        q_hash: u64,
    ) -> Vec<(usize, usize)> {
        let mut hits = Vec::new();
        let u = (q_hash >> 32) as u32;
        let upper_slice = index.lookup_upper(u);
        if upper_slice.len() <= DEFAULT_AHASH_MIH32_MAX_BUCKET_SIZE {
            for entry in upper_slice {
                let (m_idx, ref_off) = self.resolve_global_index(entry.frame_idx as usize);
                if ref_off >= k {
                    hits.push((m_idx, ref_off - k));
                }
            }
        }
        let l = (q_hash & 0xFFFF_FFFF) as u32;
        let lower_slice = index.lookup_lower(l);
        if lower_slice.len() <= DEFAULT_AHASH_MIH32_MAX_BUCKET_SIZE {
            for entry in lower_slice {
                let (m_idx, ref_off) = self.resolve_global_index(entry.frame_idx as usize);
                if ref_off >= k {
                    hits.push((m_idx, ref_off - k));
                }
            }
        }
        hits
    }

    /// Collects candidate shift votes in parallel via 32-bit Multi-Index Hashing.
    fn collect_mih32_candidate_votes(
        &self,
        query: &[u64],
        stride: usize,
    ) -> HashMap<(usize, usize), u32> {
        let index = self.mih32();
        let probe_frames: Vec<(usize, u64)> = query
            .iter()
            .enumerate()
            .step_by(stride)
            .map(|(k, &h)| (k, h))
            .collect();

        let partial_votes: Vec<Vec<(usize, usize)>> = probe_frames
            .par_iter()
            .map(|&(k, q_hash)| self.probe_frame_hits_mih32(index, k, q_hash))
            .collect();

        let mut votes = HashMap::new();
        for hits in partial_votes {
            for pair in hits {
                *votes.entry(pair).or_default() += 1;
            }
        }
        votes
    }

    /// Searches the catalog using high-selectivity 32-bit Multi-Index Hashing (~2ms).
    ///
    /// # Arguments
    /// * `criteria` - Search parameters, query slice, thresholds and limits.
    ///
    /// # Returns
    /// Vector of [`AHashCatalogMatch`] items ranked by Hamming distance.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Uses 32-bit chunk tables to isolate candidate clusters with near-zero false collisions,
    /// then performs fine jitter verification on candidate windows.
    pub fn search_mih32(&self, criteria: &AHashSearchCriteria) -> Vec<AHashCatalogMatch> {
        if criteria.query.len() < MIN_QUERY_FRAMES || self.total_frames == 0 {
            return Vec::new();
        }

        if criteria.target_media.is_some() {
            return self.search_linear(criteria);
        }

        let votes =
            self.collect_mih32_candidate_votes(criteria.query, DEFAULT_AHASH_MIH32_PROBE_STRIDE);

        let candidates = Self::rank_candidate_clusters(
            &votes,
            DEFAULT_AHASH_MIH32_MIN_VOTES,
            DEFAULT_AHASH_MIH_MAX_CANDIDATES,
        );

        if candidates.is_empty() {
            return Vec::new();
        }

        self.verify_candidate_clusters(&candidates, criteria)
    }

    /// Primary search router: targeted media -> linear; otherwise 32-bit MIH (~2ms), with a
    /// linear fallback only for small catalogs (`total_frames <= MAX_LINEAR_FALLBACK_FRAMES`).
    ///
    /// # Arguments
    /// * `criteria` - Search parameters, query slice, thresholds and limits.
    ///
    /// # Returns
    /// Vector of [`AHashCatalogMatch`] items ranked by Hamming distance. Empty on non-match
    /// in large catalogs.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Never calls [`search_mih`] (16-bit MIH), so the large `mih` table is never allocated by
    /// this router. [`search_mih`] and [`search_linear`] remain public for explicit benchmarks.
    pub fn search(&self, criteria: &AHashSearchCriteria) -> Vec<AHashCatalogMatch> {
        if criteria.target_media.is_some() {
            return self.search_linear(criteria);
        }
        let results = self.search_mih32(criteria);
        if !results.is_empty() || self.total_frames > MAX_LINEAR_FALLBACK_FRAMES {
            return results;
        }
        self.search_linear(criteria)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::ahash::{AhashHeader, HaitsmaKalkerExtractor};

    fn write_ahash(path: &Path, hashes: &[u64]) {
        let (_bins, band_bounds) = HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
        let header = AhashHeader {
            version: 1,
            sample_rate: 5512,
            frame_size: 2048,
            hop_size: 64,
            num_bands: 33,
            spectral_lag: 2,
            frame_count: hashes.len() as u64,
            band_bounds,
        };
        AhashFile::write_to_path(path, &header, hashes).unwrap();
    }

    #[test]
    fn test_sidecar_resolution_and_twin_dedup() {
        const HEX_TWIN: &str = "05b6126884056c42";
        const HEX_LOCAL: &str = "0123456789abcdef";
        const HEX_FALLBACK: &str = "fedcba9876543210";
        const CLEAN: &str = "Titre.Clair";
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("__AHASH__");
        let phash = root.path().join("__PHASH__");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&phash).unwrap();

        let twin_hashes = [1u64, 2, 3, 4];
        write_ahash(&dir.join(format!("{HEX_TWIN}.86.1fps.ahash")), &twin_hashes);
        write_ahash(&dir.join(format!("{CLEAN}.86.1fps.ahash")), &twin_hashes);
        write_ahash(
            &dir.join(format!("{HEX_LOCAL}.86.1fps.ahash")),
            &[10, 11, 12],
        );
        write_ahash(
            &dir.join(format!("{HEX_FALLBACK}.86.1fps.ahash")),
            &[20, 21, 22],
        );
        fs::write(
            dir.join(format!("{HEX_LOCAL}.json")),
            r#"{"media_name": "Local.Name"}"#,
        )
        .unwrap();
        fs::write(
            phash.join(format!("{HEX_FALLBACK}.json")),
            r#"{"media_name": "", "clean_title": "Fallback.Name"}"#,
        )
        .unwrap();

        let catalog = AHashCatalog::load_from_dir(&dir).unwrap();
        let mut names: Vec<&str> = catalog
            .entries
            .iter()
            .map(|e| e.media_name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["Fallback.Name", "Local.Name", CLEAN]);
        assert_eq!(catalog.total_frames, 4 + 3 + 3);

        // Twin whose raw-hex file resolves via sidecar to the same name stays single.
        fs::write(
            phash.join(format!("{HEX_TWIN}.json")),
            r#"{"media_name": "Titre.Clair"}"#,
        )
        .unwrap();
        let catalog = AHashCatalog::load_from_dir(&dir).unwrap();
        assert_eq!(catalog.entries.len(), 3);
        let twin = catalog
            .entries
            .iter()
            .find(|e| e.media_name == CLEAN)
            .unwrap();
        // Equal resolved names: deterministic lexicographic path tiebreak ('0' < 'T').
        assert!(twin.path.to_string_lossy().contains(HEX_TWIN));
    }

    #[test]
    fn test_typed_sidecar_takes_priority_over_legacy() {
        use dbsrt_core::{AhashSidecarRequest, AudioContainerInspection, AudioStreamInfo};
        const HEX: &str = "00112233445566aa";
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("__AHASH__");
        let phash = root.path().join("__PHASH__");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&phash).unwrap();
        let ahash_path = dir.join(format!("{HEX}.86.1fps.ahash"));
        write_ahash(&ahash_path, &[5, 6, 7]);
        fs::write(
            phash.join(format!("{HEX}.json")),
            r#"{"media_name": "Legacy.Name"}"#,
        )
        .unwrap();

        // Legacy only: name from __PHASH__, no audio info.
        let catalog = AHashCatalog::load_from_dir(&dir).unwrap();
        assert_eq!(catalog.entries[0].media_name, "Legacy.Name");
        assert_eq!(catalog.entries[0].audio_language, None);

        let track = AudioStreamInfo {
            index: 2,
            audio_index: 1,
            codec_name: "ac3".into(),
            channels: Some(6),
            sample_rate: Some(48_000),
            language: Some("jpn".into()),
            title: Some("VO".into()),
            is_default: true,
        };
        let inspection = AudioContainerInspection {
            path: PathBuf::from("m.mkv"),
            audio_streams: vec![
                AudioStreamInfo {
                    index: 1,
                    audio_index: 0,
                    language: Some("fre".into()),
                    is_default: false,
                    ..track.clone()
                },
                track,
            ],
        };
        let header = AhashFile::read_from_path(&ahash_path).unwrap().header;
        AhashSidecar::build(&AhashSidecarRequest {
            ahash_path: &ahash_path,
            header: &header,
            media_name: "Typed.Name.mkv",
            moviehash: Some(HEX),
            inspection: Some(&inspection),
            explicit_audio_stream: None,
        })
        .write_next_to(&ahash_path)
        .unwrap();

        let catalog = AHashCatalog::load_from_dir(&dir).unwrap();
        let entry = &catalog.entries[0];
        assert_eq!(entry.media_name, "Typed.Name.mkv");
        assert_eq!(entry.audio_language.as_deref(), Some("jpn"));
        assert_eq!(entry.audio_index, Some(1));
    }

    #[test]
    fn test_parse_ahash_media_name() {
        assert_eq!(
            parse_ahash_media_name("04b46e246928e909.86.1fps.ahash"),
            "04b46e246928e909"
        );
        assert_eq!(
            parse_ahash_media_name("1-Pegasus Fantasy.mkv.86.1fps.ahash"),
            "1-Pegasus Fantasy.mkv"
        );
        assert_eq!(parse_ahash_media_name("simple_video.ahash"), "simple_video");
        assert_eq!(
            parse_ahash_media_name("no_extension_stem"),
            "no_extension_stem"
        );
    }

    #[test]
    fn test_empty_catalog_search() {
        let catalog = AHashCatalog::empty();
        let query = vec![0x1111_2222_3333_4444; 16];
        let criteria = AHashSearchCriteria::new(&query, 86.125);
        let results = catalog.search(&criteria);
        assert!(results.is_empty());
    }

    #[test]
    fn test_catalog_search_exact_match() {
        let mut hashes = vec![0xAAAA_BBBB_CCCC_DDDD; 100];
        // Inject distinct patterns
        for (i, h) in hashes.iter_mut().enumerate() {
            *h = 0x1000_0000_0000_0000 | (i as u64);
        }

        let entry = AHashCatalogEntry {
            media_name: "test_media".to_string(),
            path: PathBuf::from("test_media.ahash"),
            audio_fps: 86.125,
            frame_count: hashes.len(),
            hashes: Arc::new(hashes.clone()),
            audio_language: None,
            audio_index: None,
        };

        let mut catalog = AHashCatalog::empty();
        catalog.add_entry(entry);

        let query_slice = &hashes[20..36]; // 16 frames
        let criteria = AHashSearchCriteria::new(query_slice, 86.125);
        let results = catalog.search(&criteria);

        assert_eq!(results.len(), 1);
        let best = &results[0];
        assert_eq!(best.media_name, "test_media");
        assert_eq!(best.ref_frame_offset, 20);
        assert_eq!(best.mean_distance, 0.0);
        assert_eq!(best.score, 1.0);
    }

    #[test]
    fn test_catalog_query_too_short() {
        let entry = AHashCatalogEntry {
            media_name: "test".to_string(),
            path: PathBuf::from("test.ahash"),
            audio_fps: 86.125,
            frame_count: 50,
            hashes: Arc::new(vec![0x1234; 50]),
            audio_language: None,
            audio_index: None,
        };
        let catalog = AHashCatalog::new(vec![entry]);

        let query = vec![0x1234; 4]; // < MIN_QUERY_FRAMES (8)
        let criteria = AHashSearchCriteria::new(&query, 86.125);
        let results = catalog.search(&criteria);
        assert!(results.is_empty());
    }

    #[test]
    fn test_large_catalog_non_match_skips_fallbacks() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let n = MAX_LINEAR_FALLBACK_FRAMES + 1;
        let hashes: Vec<u64> = (0..n).map(|_| next()).collect();
        let entry = AHashCatalogEntry {
            media_name: "big".to_string(),
            path: PathBuf::from("big.ahash"),
            audio_fps: 86.125,
            frame_count: n,
            hashes: Arc::new(hashes),
            audio_language: None,
            audio_index: None,
        };
        let catalog = AHashCatalog::new(vec![entry]);
        let query: Vec<u64> = (0..64).map(|_| next()).collect();
        let criteria = AHashSearchCriteria::new(&query, 86.125);
        let results = catalog.search(&criteria);
        assert!(results.is_empty());
        assert!(catalog.mih.get().is_none());
    }
}
