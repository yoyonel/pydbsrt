//! In-memory acoustic fingerprint catalog and parallel search engine.
//!
//! Provides multi-media indexing, parallel cross-correlation search via Rayon,
//! and candidate ranking for 64-bit Haitsma-Kalker audio fingerprints.

use anyhow::{Context, Result};
use dbsrt_core::ahash::{match_audio_sequence, AhashFile, DEFAULT_MAX_AUDIO_MATCH_DISTANCE};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Default maximum number of match candidates to return.
pub const DEFAULT_AHASH_SEARCH_LIMIT: usize = 5;

/// Milliseconds per second conversion factor.
pub const MILLIS_PER_SEC: f64 = 1000.0;

/// Standard file extension for binary acoustic fingerprints.
pub const AHASH_FILE_EXTENSION: &str = "ahash";

/// Minimum query frame length in sub-fingerprints for valid matching.
pub const MIN_QUERY_FRAMES: usize = 8;

/// Floating-point framerate compatibility tolerance in FPS.
pub const FPS_COMPATIBILITY_TOLERANCE: f64 = 1.0;

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
/// Strips trailing `.ahash` if present, then checks for `.<num>fps` suffix and removes it.
/// Strips trailing framerate token (e.g. `.<rate>fps`) from a filename stem.
fn strip_fps_token(stem: &str) -> &str {
    let Some(before_fps) = stem.strip_suffix("fps") else {
        return stem;
    };
    let Some((base, candidate)) = before_fps.rsplit_once('.') else {
        return stem;
    };
    if candidate.parse::<f64>().is_err() {
        return stem;
    }
    if let Some((sub_base, int_part)) = base.rsplit_once('.') {
        if int_part.parse::<f64>().is_ok() {
            return sub_base;
        }
    }
    base
}

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
/// Strips trailing `.ahash` if present, then invokes [`strip_fps_token`].
pub fn parse_ahash_media_name(filename: &str) -> String {
    let without_ext = filename.strip_suffix(".ahash").unwrap_or(filename);
    strip_fps_token(without_ext).to_string()
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

/// In-memory catalog of acoustic fingerprints supporting multi-threaded cross-correlation search.
#[derive(Debug, Clone, Default)]
pub struct AHashCatalog {
    /// Indexed reference media entries.
    pub entries: Vec<AHashCatalogEntry>,
    /// Cumulative sub-fingerprint frame count across all entries.
    pub total_frames: usize,
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
    /// Evaluates cumulative total frame count across all provided entries.
    pub fn new(entries: Vec<AHashCatalogEntry>) -> Self {
        let total_frames = entries.iter().map(|e| e.frame_count).sum();
        Self {
            entries,
            total_frames,
        }
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
    /// in parallel using Rayon before aggregating into the in-memory catalog.
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

        let entries: Vec<AHashCatalogEntry> = paths
            .into_par_iter()
            .filter_map(|p| match AhashFile::read_from_path(&p) {
                Ok(file) if !file.hashes.is_empty() => {
                    let filename = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let media_name = parse_ahash_media_name(&filename);
                    let audio_fps = file.header.audio_fps();
                    let frame_count = file.hashes.len();
                    Some(AHashCatalogEntry {
                        media_name,
                        path: p,
                        audio_fps,
                        frame_count,
                        hashes: Arc::new(file.hashes),
                    })
                }
                _ => None,
            })
            .collect();

        Ok(Self::new(entries))
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
    /// Appends entry and increments `total_frames` counter.
    pub fn add_entry(&mut self, entry: AHashCatalogEntry) {
        self.total_frames += entry.frame_count;
        self.entries.push(entry);
    }

    /// Searches the catalog in parallel for matching audio sequences.
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
    /// 1. Rejects queries shorter than [`MIN_QUERY_FRAMES`].
    /// 2. Iterates over catalog entries in parallel via Rayon.
    /// 3. Filters candidates by target media name if specified.
    /// 4. Verifies framerate compatibility within [`FPS_COMPATIBILITY_TOLERANCE`].
    /// 5. Executes zero-allocation cross-correlation via `match_audio_sequence`.
    /// 6. Collects, sorts ascending by mean Hamming distance, and limits results.
    pub fn search(&self, criteria: &AHashSearchCriteria) -> Vec<AHashCatalogMatch> {
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

        if criteria.limit > 0 && matches.len() > criteria.limit {
            matches.truncate(criteria.limit);
        }

        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let catalog = AHashCatalog::new(vec![entry]);

        let query = vec![0x1234; 4]; // < MIN_QUERY_FRAMES (8)
        let criteria = AHashSearchCriteria::new(&query, 86.125);
        let results = catalog.search(&criteria);
        assert!(results.is_empty());
    }
}
