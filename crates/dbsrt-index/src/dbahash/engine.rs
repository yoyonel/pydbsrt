//! Unified [`AhashEngine`] combining `MmapDbahash` (`0 MiB` dense heap) with incremental loose deltas.
//!
//! Mirroring [`crate::engine::PhashEngine`], [`AhashEngine`] prioritizes `data/catalog.dbahash` via
//! [`MmapDbahash`] and loads ONLY newly ingested, unpacked `.ahash` files from `data/__AHASH__/` into
//! a small delta [`AHashCatalog`], enabling atomic hot-reload via `ArcSwap` in `dbsrt-server`.

use crate::ahash::{
    dedup_and_truncate_matches, dedup_twin_entries, load_catalog_entry, sibling_phash_dir_for,
    AHashCatalog, AHashCatalogEntry, AHashCatalogMatch, AHashSearchCriteria, AHASH_FILE_EXTENSION,
};
use crate::coarse_to_fine::{
    CoarseSearchScratchpad, CoarseToFineAHashIndex, FineSearchCriteria, DEFAULT_COARSE_FACTOR_K,
};
use crate::dbahash::reader::MmapDbahash;
use anyhow::{Context, Result};
use dbsrt_core::ahash::HEADER_SIZE_BYTES;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Size in bytes of one `u64` hash word in `.ahash` files.
const U64_BYTES: u64 = std::mem::size_of::<u64>() as u64;

/// Minimum delta catalog frame count before building a secondary `CoarseToFineAHashIndex` for deltas.
const DELTA_C2F_MIN_FRAMES: usize = 200_000;

/// Unified acoustic fingerprint search engine combining `MmapDbahash` and unpacked loose `.ahash` deltas.
#[derive(Clone, Default)]
pub struct AhashEngine {
    dbahash: Option<Arc<MmapDbahash>>,
    delta_catalog: Arc<AHashCatalog>,
    delta_c2f: Arc<OnceLock<CoarseToFineAHashIndex>>,
}

impl AhashEngine {
    /// Creates an empty [`AhashEngine`] with zero indexed tracks.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// Empty [`AhashEngine`].
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// `self.is_empty() == true` and `self.media_count() == 0`.
    pub fn empty() -> Self {
        Self {
            dbahash: None,
            delta_catalog: Arc::new(AHashCatalog::empty()),
            delta_c2f: Arc::new(OnceLock::new()),
        }
    }

    /// Opens an [`AhashEngine`] from an optional `catalog.dbahash` archive and an optional `.ahash` directory.
    ///
    /// # Arguments
    /// * `dbahash_path` - Optional path to `catalog.dbahash` (loaded via `memmap2::Mmap` when existing).
    /// * `ahash_dir` - Optional directory of loose `.ahash` files; files already packed in `dbahash` are skipped!
    ///
    /// # Returns
    /// Initialized [`AhashEngine`].
    ///
    /// # Errors
    /// Returns `Err` if `dbahash_path` exists and is corrupt, or if `ahash_dir` cannot be listed.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Opens `MmapDbahash` first (`< 1 ms`, `0 MiB` dense `Vec<u64>` heap).
    /// 2. Scans `ahash_dir` via `stat()` only, skipping every file whose `(filename, frame_count)`
    ///    is already present in `MmapDbahash` (`0` bytes read for packed files).
    /// 3. Drops any renamed binary twin matching `MmapDbahash::has_identical_dense_payload`, retaining
    ///    exclusively unpacked new delta tracks in `delta_catalog`.
    pub fn open(dbahash_path: Option<&Path>, ahash_dir: Option<&Path>) -> Result<Self> {
        let dbahash = match dbahash_path.filter(|p| p.exists()) {
            Some(path) => Some(Arc::new(MmapDbahash::open(path)?)),
            None => None,
        };

        let delta_catalog = match ahash_dir.filter(|d| d.exists()) {
            Some(dir) => load_delta_from_dir(dir, dbahash.as_deref())?,
            None => AHashCatalog::empty(),
        };

        Ok(Self::from_parts(dbahash, Arc::new(delta_catalog)))
    }

    /// Constructs an [`AhashEngine`] from pre-loaded components.
    ///
    /// # Arguments
    /// * `dbahash` - Optional shared [`MmapDbahash`] reader.
    /// * `delta_catalog` - Shared delta [`AHashCatalog`] holding unpacked tracks.
    ///
    /// # Returns
    /// Assembled [`AhashEngine`].
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Pre-builds a secondary `CoarseToFineAHashIndex` only when `delta_catalog` exceeds [`DELTA_C2F_MIN_FRAMES`].
    pub fn from_parts(dbahash: Option<Arc<MmapDbahash>>, delta_catalog: Arc<AHashCatalog>) -> Self {
        let delta_c2f = OnceLock::new();
        if delta_catalog.total_frames >= DELTA_C2F_MIN_FRAMES {
            let _ = delta_c2f.set(CoarseToFineAHashIndex::build(
                &delta_catalog,
                DEFAULT_COARSE_FACTOR_K,
            ));
        }
        Self {
            dbahash,
            delta_catalog,
            delta_c2f: Arc::new(delta_c2f),
        }
    }

    /// Returns a reference to the underlying [`MmapDbahash`] reader if loaded.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `Option<&MmapDbahash>`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Borrows the `Arc<MmapDbahash>` without cloning.
    #[inline]
    pub fn dbahash(&self) -> Option<&MmapDbahash> {
        self.dbahash.as_deref()
    }

    /// Returns a cloned `Arc<MmapDbahash>` handle if loaded.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `Option<Arc<MmapDbahash>>`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Cheap atomic reference-count increment.
    #[inline]
    pub fn dbahash_arc(&self) -> Option<Arc<MmapDbahash>> {
        self.dbahash.clone()
    }

    /// Returns a reference to the incremental loose [`AHashCatalog`] delta.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `&AHashCatalog` containing only loose `.ahash` files not already packed in `dbahash`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Empty (`0` entries, `0` bytes) whenever all `.ahash` files on disk are packed in `catalog.dbahash`.
    #[inline]
    pub fn delta_catalog(&self) -> &AHashCatalog {
        &self.delta_catalog
    }

    /// Returns the shared `Arc<AHashCatalog>` delta handle.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `Arc<AHashCatalog>`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Cheap `Arc` clone for lock-free state snapshots.
    #[inline]
    pub fn delta_catalog_arc(&self) -> Arc<AHashCatalog> {
        self.delta_catalog.clone()
    }

    /// Returns the total number of indexed audio tracks (`MmapDbahash` + unpacked delta).
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// Combined track count (`usize`).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Disjoint sum because packed files and binary twins are excluded from `delta_catalog`.
    #[inline]
    pub fn media_count(&self) -> usize {
        self.dbahash.as_ref().map_or(0, |d| d.media_count()) + self.delta_catalog.entries.len()
    }

    /// Returns the cumulative number of dense sub-fingerprint frames across `MmapDbahash` and delta.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// Total dense frames (`usize`).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Disjoint sum of `dbahash.total_dense_frames()` and `delta_catalog.total_frames`.
    #[inline]
    pub fn total_frames(&self) -> usize {
        self.dbahash.as_ref().map_or(0, |d| d.total_dense_frames())
            + self.delta_catalog.total_frames
    }

    /// Returns `true` when neither `MmapDbahash` nor `delta_catalog` holds any track.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `true` if `self.media_count() == 0`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Fast emptiness guard for HTTP search handlers.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.media_count() == 0
    }

    /// Searches both `MmapDbahash` (Tier 1 `mmap` C2F) and `delta_catalog` (Tier 2 delta), merging top matches.
    ///
    /// # Arguments
    /// * `criteria` - [`AHashSearchCriteria`] specifying query, FPS, threshold, `limit`, and `target_media`.
    ///
    /// # Returns
    /// Deterministically ranked vector of up to `criteria.limit` [`AHashCatalogMatch`] candidates.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Reuses thread-local [`CoarseSearchScratchpad`]; merges `MmapDbahash` and delta hits via
    /// [`dedup_and_truncate_matches`] (`mean_distance` ascending, deterministic tie-break).
    pub fn search(&self, criteria: &AHashSearchCriteria<'_>) -> Vec<AHashCatalogMatch> {
        crate::coarse_to_fine::with_tls_scratchpad(|scratch| {
            self.search_with_scratchpad(criteria, scratch)
        })
    }

    /// Searches both `MmapDbahash` and `delta_catalog` using an explicit [`CoarseSearchScratchpad`].
    ///
    /// # Arguments
    /// * `criteria` - [`AHashSearchCriteria`] specifying query, FPS, threshold, `limit`, and `target_media`.
    /// * `scratch` - Caller-managed [`CoarseSearchScratchpad`].
    ///
    /// # Returns
    /// Deterministically ranked vector of up to `criteria.limit` [`AHashCatalogMatch`] candidates.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Zero heap allocation when `delta_catalog` is empty and no candidate certifies.
    pub fn search_with_scratchpad(
        &self,
        criteria: &AHashSearchCriteria<'_>,
        scratch: &mut CoarseSearchScratchpad,
    ) -> Vec<AHashCatalogMatch> {
        if criteria.limit == 0 || self.is_empty() {
            return Vec::new();
        }

        let mut merged = match &self.dbahash {
            Some(db) => db.search_with_scratchpad(criteria, scratch),
            None => Vec::new(),
        };

        if !self.delta_catalog.entries.is_empty() {
            let delta_hits = self.search_delta(criteria, scratch);
            if merged.is_empty() {
                return delta_hits;
            }
            merged.extend(delta_hits);
            merged = dedup_and_truncate_matches(merged, criteria.limit);
        }

        merged
    }

    /// Searches the unpacked delta catalog using C2F (when large) or [`AHashCatalog::search`].
    fn search_delta(
        &self,
        criteria: &AHashSearchCriteria<'_>,
        scratch: &mut CoarseSearchScratchpad,
    ) -> Vec<AHashCatalogMatch> {
        if criteria.limit == 1 && criteria.target_media.is_none() {
            if let Some(c2f) = self.delta_c2f.get() {
                let fine = FineSearchCriteria {
                    dense_query: criteria.query,
                    dense_fps: criteria.audio_fps,
                    max_distance: criteria.max_distance,
                };
                return c2f
                    .search_with_scratchpad(&self.delta_catalog, &fine, scratch)
                    .into_iter()
                    .collect();
            }
        }
        self.delta_catalog.search(criteria)
    }
}

/// Scans `dir` and loads ONLY loose `.ahash` files not already packed inside `dbahash`.
///
/// # Data Flow & Invariants
/// **Invariant (ADR-0007)**: Two-stage incremental deduplication — Stage 1 skips known `(filename, frames)`
/// via `stat()` (`0` bytes read); Stage 2 rejects renamed binary twins via `has_identical_dense_payload`.
fn load_delta_from_dir(dir: &Path, dbahash: Option<&MmapDbahash>) -> Result<AHashCatalog> {
    let read_dir = fs::read_dir(dir).with_context(|| format!("Reading {:?}", dir))?;
    let mut candidate_paths: Vec<PathBuf> = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() || !path.extension().is_some_and(|e| e == AHASH_FILE_EXTENSION) {
            continue;
        }
        if is_file_already_packed(&path, dbahash) {
            continue;
        }
        candidate_paths.push(path);
    }

    if candidate_paths.is_empty() {
        return Ok(AHashCatalog::empty());
    }
    candidate_paths.sort();

    let phash_dir = sibling_phash_dir_for(dir);
    let loaded: Vec<AHashCatalogEntry> = candidate_paths
        .into_par_iter()
        .filter_map(|p| {
            let entry = load_catalog_entry(p, dir, phash_dir.as_deref())?;
            if let Some(db) = dbahash {
                if db.has_identical_dense_payload(&entry.hashes) {
                    return None;
                }
            }
            Some(entry)
        })
        .collect();

    Ok(AHashCatalog::new(dedup_twin_entries(loaded)))
}

/// Checks via `std::fs::metadata` (`stat` only, zero file read) whether `path` is already in `dbahash`.
fn is_file_already_packed(path: &Path, dbahash: Option<&MmapDbahash>) -> bool {
    let Some(db) = dbahash else {
        return false;
    };
    let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let len = meta.len();
    let header_len = HEADER_SIZE_BYTES as u64;
    if len <= header_len || !(len - header_len).is_multiple_of(U64_BYTES) {
        return false;
    }
    let Ok(frames) = u32::try_from((len - header_len) / U64_BYTES) else {
        return false;
    };
    db.contains_file_name_and_frames(fname, frames)
}
