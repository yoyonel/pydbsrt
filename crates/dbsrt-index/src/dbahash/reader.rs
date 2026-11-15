//! Zero-copy `memmap2::Mmap` reader and Coarse-to-Fine search engine for `.dbahash` v1 archives.
//!
//! Opens compiled `.dbahash` files in `< 1 ms` with zero dense heap allocation (`0 MiB` `Vec<u64>`),
//! probing pre-sorted `MIH-32` coarse tables directly inside the memory-mapped region and paging in
//! only the ~5 OS `4 KiB` pages (`~36 KiB`) covering verified candidate dense windows.

use crate::ahash::{
    dedup_and_truncate_matches, is_valid_ahash_search_criteria, AHashCatalogMatch, AHashMih32Index,
    AHashSearchCriteria, Mih32Entry, MAX_LINEAR_FALLBACK_FRAMES, MIN_QUERY_FRAMES,
};
use crate::coarse_to_fine::{
    evaluate_fine_slice, into_catalog_match, prepare_fallback_clusters, primary_vote_threshold,
    probe_coarse_mih32_votes, rank_hits_in_scratchpad, with_tls_scratchpad, CoarseCluster,
    CoarseProbeTables, CoarseSearchScratchpad, FineSearchCriteria, FineTrackView, FineWindowWin,
    DEFAULT_FINE_WINDOW_RADIUS_FRAMES, MIN_COARSE_ANCHOR_SPAN_MULTIPLIER, MIN_COARSE_CLUSTER_VOTES,
};
use crate::dbahash::format::{
    compute_dbahash_checksum, DbahashHeader, DbahashMediaEntry, DBAHASH_HEADER_BYTES,
    DBAHASH_MAGIC, DBAHASH_MEDIA_ENTRY_BYTES, DBAHASH_SECTION_ALIGN, DBAHASH_TWIN_TRAILER_MAGIC,
    DBAHASH_VERSION, TWIN_TRAILER_COUNT_BYTES, TWIN_TRAILER_RECORD_HEADER_BYTES,
};
use anyhow::{anyhow, ensure, Context, Result};
use dbsrt_core::ahash::is_silent_or_flat_audio_query;
use memmap2::Mmap;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Size in bytes of one 64-bit hash (`8`).
const U64_BYTES: u64 = std::mem::size_of::<u64>() as u64;

/// Size in bytes of one [`Mih32Entry`] (`8`).
const MIH32_BYTES: u64 = std::mem::size_of::<Mih32Entry>() as u64;

/// Lazily initialized coarse `MIH-32` index used only when opening `--no-mih32` archives.
struct FallbackCoarseTables {
    coarse_offsets: Vec<u64>,
    mih32: AHashMih32Index,
}

/// Zero-copy memory-mapped reader for `.dbahash` v1 acoustic fingerprint archives.
pub struct MmapDbahash {
    /// Filesystem path of the mapped `.dbahash` archive.
    pub path: PathBuf,
    _file: File,
    mmap: Mmap,
    header: DbahashHeader,
    /// Prefix sums of `coarse_frame_count` (`len == media_count + 1`) for $\mathcal{O}(\log M)$ resolution.
    coarse_offsets: Vec<u64>,
    /// Fast lookup `canonical_filename -> dense_frame_counts` skipping packed loose files in $\mathcal{O}(1)$.
    packed_by_filename: HashMap<String, Vec<u32>>,
    /// Lookup `dense_frame_count -> entry_indices` detecting renamed binary twins without heap bloat.
    packed_by_frames: HashMap<u32, Vec<usize>>,
    /// Fine dense refinement radius in frames (default [`DEFAULT_FINE_WINDOW_RADIUS_FRAMES`] = `16`).
    fine_window_radius_frames: usize,
    /// Lazy coarse index constructed only if the archive was compiled with `--no-mih32`.
    fallback_coarse: OnceLock<FallbackCoarseTables>,
}

impl MmapDbahash {
    /// Opens, validates, and memory-maps a `.dbahash` v1 archive in `< 1 ms`.
    ///
    /// # Arguments
    /// * `path` - Filesystem path to the `.dbahash` v1 binary archive.
    ///
    /// # Returns
    /// Validated [`MmapDbahash`] instance ready for zero-allocation search.
    ///
    /// # Errors
    /// Returns `Err` if the file cannot be opened/mapped, has invalid magic/version, violates
    /// 64-byte section alignment, exceeds file bounds, or fails checksum/UTF-8 verification.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Maps the file via `memmap2::Mmap` without faulting cold dense pages.
    /// 2. Validates header magic, version, `factor_k >= 1`, 64-byte alignments, and section bounds.
    /// 3. Verifies FNV-1a checksum over header, media table, and UTF-8 string pool.
    /// 4. Pre-indexes `coarse_offsets` and per-filename frame counts for $\mathcal{O}(1)$ delta deduplication.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let p = path.as_ref().to_path_buf();
        let file = File::open(&p).with_context(|| format!("Failed to open .dbahash at {:?}", p))?;
        // SAFETY: Archive is written atomically via `<tmp>` + `fsync` + `rename` and never mutated in place.
        let mmap = unsafe { Mmap::map(&file) }
            .with_context(|| format!("Failed to mmap .dbahash at {:?}", p))?;

        ensure!(
            mmap.len() >= DBAHASH_HEADER_BYTES,
            "File {:?} too small for .dbahash header ({} bytes)",
            p,
            mmap.len()
        );
        let header: DbahashHeader = *bytemuck::from_bytes(&mmap[..DBAHASH_HEADER_BYTES]);
        validate_header_layout(&header, mmap.len() as u64)?;
        advise_mmap_thermal_regions(&mmap, &header);

        let media_slice = slice_media_entries(&mmap, &header);
        let strings_slice = slice_strings_bytes(&mmap, &header);
        let expected_crc = compute_dbahash_checksum(&header, media_slice, strings_slice);
        ensure!(
            header.checksum == expected_crc,
            "Corrupt .dbahash checksum in {:?}: expected {:#018x}, got {:#018x}",
            p,
            expected_crc,
            header.checksum
        );

        let (coarse_offsets, packed_by_filename, packed_by_frames) =
            validate_and_index_entries(&header, media_slice, strings_slice)?;

        Ok(Self {
            path: p,
            _file: file,
            mmap,
            header,
            coarse_offsets,
            packed_by_filename,
            packed_by_frames,
            fine_window_radius_frames: DEFAULT_FINE_WINDOW_RADIUS_FRAMES,
            fallback_coarse: OnceLock::new(),
        })
    }

    /// Overrides the fine dense correlation radius in frames (default `16`).
    ///
    /// # Arguments
    /// * `radius_frames` - Half-window search radius in dense frames (`>= 1`).
    ///
    /// # Returns
    /// Updated [`MmapDbahash`].
    ///
    /// # Panics
    /// Never panics (clamps `0` to `1`).
    ///
    /// # Data Flow & Invariants
    /// Adjusts the `[approx - radius ..= approx + radius]` dense slice window.
    pub fn with_fine_window_radius_frames(mut self, radius_frames: usize) -> Self {
        self.fine_window_radius_frames = radius_frames.max(1);
        self
    }

    /// Returns a reference to the validated 128-byte [`DbahashHeader`].
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `&DbahashHeader`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Zero-cost header borrow.
    #[inline]
    pub fn header(&self) -> &DbahashHeader {
        &self.header
    }

    /// Returns the coarse decimation stride $K$ (`>= 1`).
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `factor_k` as `usize`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Guaranteed `>= 1` by [`Self::open`].
    #[inline]
    pub fn factor_k(&self) -> usize {
        self.header.factor_k as usize
    }

    /// Returns the number of packed audio tracks in the archive.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// Track count (`usize`).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Equals `self.media_entries().len()`.
    #[inline]
    pub fn media_count(&self) -> usize {
        self.header.media_count as usize
    }

    /// Returns the cumulative count of dense `u64` frames across all tracks.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// Total dense frame count (`usize`).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Equals `self.dense_hashes().len()`.
    #[inline]
    pub fn total_dense_frames(&self) -> usize {
        self.header.total_dense_frames as usize
    }

    /// Returns the cumulative count of embedded coarse `u64` frames (`0` if `--no-mih32`).
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// Total coarse frame count (`usize`).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Equals `self.coarse_hashes().len()`.
    #[inline]
    pub fn total_coarse_frames(&self) -> usize {
        self.header.total_coarse_frames as usize
    }

    /// Returns the zero-copy slice of [`DbahashMediaEntry`] records from the `mmap`.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `&[DbahashMediaEntry]`.
    ///
    /// # Panics
    /// Never panics (bounds and 64-byte alignment verified in [`Self::open`]).
    ///
    /// # Data Flow & Invariants
    /// Direct `bytemuck::cast_slice` over the mapped media table.
    #[inline]
    pub fn media_entries(&self) -> &[DbahashMediaEntry] {
        slice_media_entries(&self.mmap, &self.header)
    }

    /// Returns the zero-copy flat slice of all dense `u64` hashes (`86.125 fps`).
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `&[u64]` backed by OS virtual memory pages (not faulted until accessed).
    ///
    /// # Panics
    /// Never panics (bounds and alignment verified in [`Self::open`]).
    ///
    /// # Data Flow & Invariants
    /// Zero heap allocation; OS kernel pages in `4 KiB` blocks on demand.
    #[inline]
    pub fn dense_hashes(&self) -> &[u64] {
        let start = self.header.offset_dense_hashes as usize;
        let byte_len = (self.header.total_dense_frames * U64_BYTES) as usize;
        bytemuck::cast_slice(&self.mmap[start..start + byte_len])
    }

    /// Returns the zero-copy dense `u64` slice belonging to track `entry_idx`.
    ///
    /// # Arguments
    /// * `entry_idx` - 0-based index into [`Self::media_entries`].
    ///
    /// # Returns
    /// `Some(&[u64])` when `entry_idx < self.media_count()`, `None` otherwise.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Slices `self.dense_hashes()[dense_start_idx .. dense_start_idx + dense_frame_count]`.
    #[inline]
    pub fn media_dense_hashes(&self, entry_idx: usize) -> Option<&[u64]> {
        let entry = self.media_entries().get(entry_idx)?;
        let start = entry.dense_start_idx as usize;
        let end = start + entry.dense_frame_count as usize;
        self.dense_hashes().get(start..end)
    }

    /// Returns the zero-copy flat slice of coarse `u64` hashes (`empty` if `--no-mih32`).
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `&[u64]`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Empty slice when `!self.header.has_coarse_mih32()`.
    #[inline]
    pub fn coarse_hashes(&self) -> &[u64] {
        if !self.header.has_coarse_mih32() || self.header.total_coarse_frames == 0 {
            return &[];
        }
        let start = self.header.offset_coarse_hashes as usize;
        let byte_len = (self.header.total_coarse_frames * U64_BYTES) as usize;
        bytemuck::cast_slice(&self.mmap[start..start + byte_len])
    }

    /// Returns the zero-copy coarse `u64` slice belonging to track `entry_idx` (`None` if `--no-mih32`).
    ///
    /// # Arguments
    /// * `entry_idx` - 0-based index into [`Self::media_entries`].
    ///
    /// # Returns
    /// `Some(&[u64])` when coarse tables are embedded and `entry_idx < self.media_count()`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Slices `self.coarse_hashes()[coarse_start_idx .. coarse_start_idx + coarse_frame_count]`.
    #[inline]
    pub fn media_coarse_hashes(&self, entry_idx: usize) -> Option<&[u64]> {
        if !self.header.has_coarse_mih32() {
            return None;
        }
        let entry = self.media_entries().get(entry_idx)?;
        let start = entry.coarse_start_idx as usize;
        let end = start + entry.coarse_frame_count as usize;
        self.coarse_hashes().get(start..end)
    }

    /// Returns the pre-sorted upper 32-bit `MIH-32` table directly from `mmap`.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `&[Mih32Entry]` sorted by `(chunk, frame_idx)`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Zero-copy slice; enables instant binary search without boot-time sorting.
    #[inline]
    pub fn coarse_mih_upper(&self) -> &[Mih32Entry] {
        if !self.header.has_coarse_mih32() || self.header.total_coarse_frames == 0 {
            return &[];
        }
        let start = self.header.offset_coarse_mih_upper as usize;
        let byte_len = (self.header.total_coarse_frames * MIH32_BYTES) as usize;
        bytemuck::cast_slice(&self.mmap[start..start + byte_len])
    }

    /// Returns the pre-sorted lower 32-bit `MIH-32` table directly from `mmap`.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `&[Mih32Entry]` sorted by `(chunk, frame_idx)`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Zero-copy slice; enables instant binary search without boot-time sorting.
    #[inline]
    pub fn coarse_mih_lower(&self) -> &[Mih32Entry] {
        if !self.header.has_coarse_mih32() || self.header.total_coarse_frames == 0 {
            return &[];
        }
        let start = self.header.offset_coarse_mih_lower as usize;
        let byte_len = (self.header.total_coarse_frames * MIH32_BYTES) as usize;
        bytemuck::cast_slice(&self.mmap[start..start + byte_len])
    }

    /// Returns the UTF-8 `media_name` for `entry` without allocation.
    ///
    /// # Arguments
    /// * `entry` - Reference to a [`DbahashMediaEntry`] from this archive.
    ///
    /// # Returns
    /// Borrowed `&str` from the mapped string pool.
    ///
    /// # Panics
    /// Never panics (UTF-8 and bounds verified in [`Self::open`]).
    ///
    /// # Data Flow & Invariants
    /// Slices `strings_table[name_offset .. name_offset + name_len]`.
    #[inline]
    pub fn media_name(&self, entry: &DbahashMediaEntry) -> &str {
        let pool = slice_strings_bytes(&self.mmap, &self.header);
        let start = entry.name_offset as usize;
        let end = start.saturating_add(entry.name_len as usize);
        pool.get(start..end)
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("")
    }

    /// Returns the UTF-8 `file_path` for `entry` without allocation.
    ///
    /// # Arguments
    /// * `entry` - Reference to a [`DbahashMediaEntry`] from this archive.
    ///
    /// # Returns
    /// Borrowed `&str` from the mapped string pool.
    ///
    /// # Panics
    /// Never panics (UTF-8 and bounds verified in [`Self::open`]).
    ///
    /// # Data Flow & Invariants
    /// Slices `strings_table[path_offset .. path_offset + path_len]`.
    #[inline]
    pub fn file_path(&self, entry: &DbahashMediaEntry) -> &str {
        let pool = slice_strings_bytes(&self.mmap, &self.header);
        let start = entry.path_offset as usize;
        let end = start.saturating_add(entry.path_len as usize);
        pool.get(start..end)
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("")
    }

    /// Checks in $\mathcal{O}(1)$ (zero file read) whether a loose `.ahash` file is already packed.
    ///
    /// # Arguments
    /// * `file_name` - Basename of the `.ahash` file (e.g. `"05b6126884056c42.a1.86.1fps.ahash"`).
    /// * `dense_frames` - Payload frame count derived from `(file_bytes - 176) / 8`.
    ///
    /// # Returns
    /// `true` if a packed track shares both `file_name` and `dense_frames`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Enables [`crate::dbahash::AhashEngine`] to skip packed files via `stat()` alone (`0` heap RAM).
    #[inline]
    pub fn contains_file_name_and_frames(&self, file_name: &str, dense_frames: u32) -> bool {
        self.packed_by_filename
            .get(file_name)
            .is_some_and(|counts| counts.contains(&dense_frames))
    }

    /// Checks whether `hashes` is byte-identical to any packed track of the same length.
    ///
    /// # Arguments
    /// * `hashes` - Candidate dense `u64` slice from a loose file whose filename differed.
    ///
    /// # Returns
    /// `true` if `hashes` is a binary twin of an already-packed track.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// - Consults `packed_by_frames` first so non-matching frame counts touch `0` mmap pages.
    /// - **Invariant (ADR-0007)**: When coarse tables are embedded, filters candidates by `1/K`
    ///   `media_coarse_hashes` equality before touching cold `media_dense_hashes` pages.
    pub fn has_identical_dense_payload(&self, hashes: &[u64]) -> bool {
        let Ok(len_u32) = u32::try_from(hashes.len()) else {
            return false;
        };
        let Some(indices) = self.packed_by_frames.get(&len_u32) else {
            return false;
        };
        let k = self.factor_k();
        indices.iter().any(|&idx| {
            if let Some(coarse) = self.media_coarse_hashes(idx) {
                if !hashes.iter().step_by(k).copied().eq(coarse.iter().copied()) {
                    return false;
                }
            }
            self.media_dense_hashes(idx) == Some(hashes)
        })
    }

    /// Performs zero-allocation Coarse-to-Fine search using the thread-local scratchpad.
    ///
    /// # Arguments
    /// * `criteria` - [`FineSearchCriteria`] (`dense_query`, `dense_fps`, `max_distance`).
    ///
    /// # Returns
    /// Best certified [`AHashCatalogMatch`], or `None`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Borrows `TLS_COARSE_SCRATCHPAD` and delegates to [`Self::search_fine_with_scratchpad`].
    pub fn search_fine(&self, criteria: &FineSearchCriteria<'_>) -> Option<AHashCatalogMatch> {
        with_tls_scratchpad(|scratch| self.search_fine_with_scratchpad(criteria, scratch))
    }

    /// Performs zero-allocation Coarse-to-Fine search using a caller-provided [`CoarseSearchScratchpad`].
    ///
    /// # Arguments
    /// * `criteria` - [`FineSearchCriteria`] (`dense_query`, `dense_fps`, `max_distance`).
    /// * `scratch` - Pre-allocated reusable arena (`Zero Dynamic Heap Allocation` in hot loop).
    ///
    /// # Returns
    /// Best certified [`AHashCatalogMatch`], or `None`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Validates query length, silence/flatness, FPS, and `max_distance`.
    /// 2. Short queries (`< 2 * K`) on compact archives (`<= 500k` frames) run bounded linear scan.
    /// 3. Standard queries probe `mmap` `MIH-32` slices into `scratch`, rank jitter clusters without
    ///    `BTreeMap`, and verify top candidates over `±16` dense `mmap` frames (~5 OS `4 KiB` pages).
    pub fn search_fine_with_scratchpad(
        &self,
        criteria: &FineSearchCriteria<'_>,
        scratch: &mut CoarseSearchScratchpad,
    ) -> Option<AHashCatalogMatch> {
        let full = AHashSearchCriteria {
            query: criteria.dense_query,
            audio_fps: criteria.dense_fps,
            max_distance: criteria.max_distance,
            limit: 1,
            target_media: None,
        };
        self.search_with_scratchpad(&full, scratch)
            .into_iter()
            .next()
    }

    /// Searches the memory-mapped archive using [`AHashSearchCriteria`] (`limit`, `target_media`, C2F).
    ///
    /// # Arguments
    /// * `criteria` - [`AHashSearchCriteria`] specifying query, FPS, threshold, `limit`, and `target_media`.
    ///
    /// # Returns
    /// Ranked vector of up to `criteria.limit` distinct [`AHashCatalogMatch`] results.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Uses the thread-local [`CoarseSearchScratchpad`] for zero-allocation candidate voting.
    pub fn search(&self, criteria: &AHashSearchCriteria<'_>) -> Vec<AHashCatalogMatch> {
        with_tls_scratchpad(|scratch| self.search_with_scratchpad(criteria, scratch))
    }

    /// Searches the memory-mapped archive using an explicit [`CoarseSearchScratchpad`].
    ///
    /// # Arguments
    /// * `criteria` - [`AHashSearchCriteria`] specifying query, FPS, threshold, `limit`, and `target_media`.
    /// * `scratch` - Reusable pre-allocated [`CoarseSearchScratchpad`].
    ///
    /// # Returns
    /// Ranked vector of up to `criteria.limit` distinct [`AHashCatalogMatch`] results.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Handles targeted media lookups, short-query bounded fallback on compact archives, and
    /// multi-candidate C2F `MIH-32` verification with deterministic tie-breaking.
    pub fn search_with_scratchpad(
        &self,
        criteria: &AHashSearchCriteria<'_>,
        scratch: &mut CoarseSearchScratchpad,
    ) -> Vec<AHashCatalogMatch> {
        if !is_valid_ahash_search_criteria(criteria)
            || self.media_count() == 0
            || criteria.query.len() < MIN_QUERY_FRAMES
            || is_silent_or_flat_audio_query(criteria.query)
        {
            return Vec::new();
        }

        if let Some(target) = criteria.target_media {
            return self.search_targeted_media(criteria, target, scratch);
        }

        let min_c2f_len = self
            .factor_k()
            .saturating_mul(MIN_COARSE_ANCHOR_SPAN_MULTIPLIER);
        if criteria.query.len() < min_c2f_len
            && self.total_dense_frames() <= MAX_LINEAR_FALLBACK_FRAMES
        {
            return self.search_bounded_linear(criteria, None);
        }

        self.search_c2f_untargeted(criteria, scratch)
    }

    /// Resolves a global coarse frame index to `(media_idx, local_coarse_idx)` via binary search.
    #[inline]
    fn resolve_coarse_index(offsets: &[u64], global_idx: usize) -> (usize, usize) {
        let g = global_idx as u64;
        let pos = offsets.partition_point(|&off| off <= g);
        let m_idx = pos.saturating_sub(1);
        let local = g.saturating_sub(offsets.get(m_idx).copied().unwrap_or(0)) as usize;
        (m_idx, local)
    }

    /// Returns borrowed `(upper, lower, coarse_offsets)` slices (embedded `mmap` or lazy fallback).
    fn active_coarse_tables(&self) -> (&[Mih32Entry], &[Mih32Entry], &[u64]) {
        if self.header.has_coarse_mih32() {
            (
                self.coarse_mih_upper(),
                self.coarse_mih_lower(),
                &self.coarse_offsets,
            )
        } else {
            let fb = self
                .fallback_coarse
                .get_or_init(|| build_fallback_coarse_tables(self));
            (
                &fb.mih32.upper_table,
                &fb.mih32.lower_table,
                &fb.coarse_offsets,
            )
        }
    }

    /// Executes zero-alloc C2F `MIH-32` search across all packed tracks.
    fn search_c2f_untargeted(
        &self,
        criteria: &AHashSearchCriteria<'_>,
        scratch: &mut CoarseSearchScratchpad,
    ) -> Vec<AHashCatalogMatch> {
        let (upper, lower, offsets) = self.active_coarse_tables();
        if upper.is_empty() && lower.is_empty() {
            return Vec::new();
        }

        scratch.clear();
        let probe_tables = CoarseProbeTables {
            upper,
            lower,
            factor_k: self.factor_k(),
        };
        probe_coarse_mih32_votes(
            probe_tables,
            criteria.query,
            |g| Self::resolve_coarse_index(offsets, g),
            &mut scratch.hits,
        );

        let min_votes = primary_vote_threshold(criteria.query.len(), self.factor_k());
        rank_hits_in_scratchpad(scratch, min_votes);

        let fine_crit = FineSearchCriteria {
            dense_query: criteria.query,
            dense_fps: criteria.audio_fps,
            max_distance: criteria.max_distance,
        };

        let mut results = self.collect_cluster_matches(&fine_crit, &scratch.kept, criteria.limit);
        if results.is_empty()
            && min_votes > MIN_COARSE_CLUSTER_VOTES
            && !scratch.fallback_clusters.is_empty()
        {
            prepare_fallback_clusters(scratch);
            results =
                self.collect_cluster_matches(&fine_crit, &scratch.kept_fallback, criteria.limit);
        }
        results
    }

    /// Evaluates C2F first, then bounded sliding fallback on tracks matching `target_media`.
    fn search_targeted_media(
        &self,
        criteria: &AHashSearchCriteria<'_>,
        target: &str,
        scratch: &mut CoarseSearchScratchpad,
    ) -> Vec<AHashCatalogMatch> {
        let min_c2f_len = self
            .factor_k()
            .saturating_mul(MIN_COARSE_ANCHOR_SPAN_MULTIPLIER);
        if criteria.query.len() >= min_c2f_len {
            let (upper, lower, offsets) = self.active_coarse_tables();
            scratch.clear();
            probe_coarse_mih32_votes(
                CoarseProbeTables {
                    upper,
                    lower,
                    factor_k: self.factor_k(),
                },
                criteria.query,
                |g| Self::resolve_coarse_index(offsets, g),
                &mut scratch.hits,
            );
            let entries = self.media_entries();
            scratch.hits.retain(|&packed| {
                let (m_idx, _) = crate::coarse_to_fine::unpack_media_offset(packed);
                entries
                    .get(m_idx)
                    .is_some_and(|e| self.media_name(e).eq_ignore_ascii_case(target))
            });
            rank_hits_in_scratchpad(scratch, MIN_COARSE_CLUSTER_VOTES);
            let fine_crit = FineSearchCriteria {
                dense_query: criteria.query,
                dense_fps: criteria.audio_fps,
                max_distance: criteria.max_distance,
            };
            let hits = self.collect_cluster_matches(&fine_crit, &scratch.kept, criteria.limit);
            if !hits.is_empty() {
                return hits;
            }
        }
        self.search_bounded_linear(criteria, Some(target))
    }

    /// Materializes `AHashCatalogMatch` results from ranked coarse clusters.
    fn collect_cluster_matches(
        &self,
        fine_crit: &FineSearchCriteria<'_>,
        clusters: &[CoarseCluster],
        limit: usize,
    ) -> Vec<AHashCatalogMatch> {
        let entries = self.media_entries();
        if limit == 1 {
            let mut best: Option<(usize, FineWindowWin)> = None;
            for cluster in clusters {
                let Some(win) =
                    self.eval_entry_window(cluster.m_idx, fine_crit, cluster.approx_dense_start)
                else {
                    continue;
                };
                if best
                    .as_ref()
                    .is_none_or(|(_, b)| win.mean_distance < b.mean_distance)
                {
                    best = Some((cluster.m_idx, win));
                }
            }
            return best
                .and_then(|(idx, win)| {
                    let e = entries.get(idx)?;
                    Some(vec![into_catalog_match(
                        self.media_name(e).to_string(),
                        self.file_path(e).to_string(),
                        win,
                    )])
                })
                .unwrap_or_default();
        }

        let mut matches = Vec::new();
        for cluster in clusters {
            if let (Some(e), Some(win)) = (
                entries.get(cluster.m_idx),
                self.eval_entry_window(cluster.m_idx, fine_crit, cluster.approx_dense_start),
            ) {
                matches.push(into_catalog_match(
                    self.media_name(e).to_string(),
                    self.file_path(e).to_string(),
                    win,
                ));
            }
        }
        dedup_and_truncate_matches(matches, limit)
    }

    /// Evaluates one candidate cluster's `±radius` fine window directly on its `mmap` slice.
    #[inline]
    fn eval_entry_window(
        &self,
        entry_idx: usize,
        fine_crit: &FineSearchCriteria<'_>,
        approx_start: usize,
    ) -> Option<FineWindowWin> {
        let entry = self.media_entries().get(entry_idx)?;
        let hashes = self.media_dense_hashes(entry_idx)?;
        evaluate_fine_slice(
            FineTrackView {
                hashes,
                audio_fps: entry.audio_fps(),
            },
            fine_crit,
            approx_start,
            self.fine_window_radius_frames,
        )
    }

    /// Parallel bounded early-exit sliding search for targeted media or short queries on small archives.
    fn search_bounded_linear(
        &self,
        criteria: &AHashSearchCriteria<'_>,
        target_media: Option<&str>,
    ) -> Vec<AHashCatalogMatch> {
        let fine_crit = FineSearchCriteria {
            dense_query: criteria.query,
            dense_fps: criteria.audio_fps,
            max_distance: criteria.max_distance,
        };
        let entries = self.media_entries();
        let matches: Vec<AHashCatalogMatch> = (0..entries.len())
            .into_par_iter()
            .filter_map(|idx| {
                let entry = &entries[idx];
                if let Some(target) = target_media {
                    if !self.media_name(entry).eq_ignore_ascii_case(target) {
                        return None;
                    }
                }
                let hashes = self.media_dense_hashes(idx)?;
                let win = evaluate_fine_slice(
                    FineTrackView {
                        hashes,
                        audio_fps: entry.audio_fps(),
                    },
                    &fine_crit,
                    0,
                    hashes.len(),
                )?;
                Some(into_catalog_match(
                    self.media_name(entry).to_string(),
                    self.file_path(entry).to_string(),
                    win,
                ))
            })
            .collect();
        dedup_and_truncate_matches(matches, criteria.limit)
    }
}

/// Validates [`DbahashHeader`] magic, version, 64-byte section alignments, and file bounds.
fn validate_header_layout(h: &DbahashHeader, file_len: u64) -> Result<()> {
    if h.magic != DBAHASH_MAGIC {
        return Err(anyhow!("Invalid .dbahash magic: {:?}", h.magic));
    }
    ensure!(
        h.version == DBAHASH_VERSION,
        "Unsupported .dbahash version {} (expected {})",
        h.version,
        DBAHASH_VERSION
    );
    ensure!(h.factor_k >= 1, "Invalid .dbahash factor_k: 0");

    let check_section = |name: &str, offset: u64, byte_len: u64| -> Result<()> {
        ensure!(
            offset.is_multiple_of(DBAHASH_SECTION_ALIGN),
            "Section '{name}' offset {offset} is not {}-byte aligned",
            DBAHASH_SECTION_ALIGN
        );
        let end = offset
            .checked_add(byte_len)
            .ok_or_else(|| anyhow!("Section '{name}' byte range overflow"))?;
        ensure!(
            end <= file_len,
            "Section '{name}' ({offset}..{end}) exceeds file length {file_len}"
        );
        Ok(())
    };

    let media_bytes = (h.media_count as u64)
        .checked_mul(DBAHASH_MEDIA_ENTRY_BYTES as u64)
        .ok_or_else(|| anyhow!("Media table size overflow"))?;
    check_section("media_table", h.offset_media_table, media_bytes)?;
    check_section("strings_table", h.offset_strings_table, h.strings_len)?;

    let dense_bytes = h
        .total_dense_frames
        .checked_mul(U64_BYTES)
        .ok_or_else(|| anyhow!("Dense hashes size overflow"))?;
    check_section("dense_hashes", h.offset_dense_hashes, dense_bytes)?;

    if h.has_coarse_mih32() {
        let coarse_bytes = h
            .total_coarse_frames
            .checked_mul(U64_BYTES)
            .ok_or_else(|| anyhow!("Coarse hashes size overflow"))?;
        let mih_bytes = h
            .total_coarse_frames
            .checked_mul(MIH32_BYTES)
            .ok_or_else(|| anyhow!("Coarse MIH-32 size overflow"))?;
        check_section("coarse_hashes", h.offset_coarse_hashes, coarse_bytes)?;
        check_section("coarse_mih_upper", h.offset_coarse_mih_upper, mih_bytes)?;
        check_section("coarse_mih_lower", h.offset_coarse_mih_lower, mih_bytes)?;
    }
    Ok(())
}

/// Casts the validated media table byte range to `&[DbahashMediaEntry]`.
#[inline]
fn slice_media_entries<'a>(mmap: &'a Mmap, header: &DbahashHeader) -> &'a [DbahashMediaEntry] {
    let start = header.offset_media_table as usize;
    let byte_len = (header.media_count as usize) * DBAHASH_MEDIA_ENTRY_BYTES;
    bytemuck::cast_slice(&mmap[start..start + byte_len])
}

/// Slices the validated unpadded UTF-8 string pool bytes.
#[inline]
fn slice_strings_bytes<'a>(mmap: &'a Mmap, header: &DbahashHeader) -> &'a [u8] {
    let start = header.offset_strings_table as usize;
    let end = start + (header.strings_len as usize);
    &mmap[start..end]
}

/// Applies VMA-scoped `MADV_RANDOM` exclusively to cold `dense_hashes`, keeping `64 KiB` fault-around
/// active on hot metadata and `MIH-32` tables (**Invariant ADR-0007**).
#[cfg(unix)]
fn advise_mmap_thermal_regions(mmap: &Mmap, header: &DbahashHeader) {
    let dense_off = header.offset_dense_hashes as usize;
    if mmap.len() > dense_off {
        let _ = mmap.advise_range(memmap2::Advice::Random, dense_off, mmap.len() - dense_off);
    }
}

/// Non-Unix stub for [`advise_mmap_thermal_regions`].
#[cfg(not(unix))]
fn advise_mmap_thermal_regions(_mmap: &Mmap, _header: &DbahashHeader) {}

type EntryIndexMaps = (
    Vec<u64>,
    HashMap<String, Vec<u32>>,
    HashMap<u32, Vec<usize>>,
);

/// Parses the optional `\0TWINS1\0` suffix in `strings` so absorbed binary twins skip `stat()` in `O(1)`.
fn parse_twin_alias_trailer(
    trailer: &[u8],
    packed_by_filename: &mut HashMap<String, Vec<u32>>,
) -> Result<()> {
    let Some(mut rest) = trailer.strip_prefix(&DBAHASH_TWIN_TRAILER_MAGIC) else {
        return Ok(());
    };
    let count_bytes: [u8; TWIN_TRAILER_COUNT_BYTES] = rest
        .get(..TWIN_TRAILER_COUNT_BYTES)
        .ok_or_else(|| anyhow!("Truncated twin trailer count"))?
        .try_into()?;
    let count = u32::from_le_bytes(count_bytes) as usize;
    rest = &rest[TWIN_TRAILER_COUNT_BYTES..];

    for idx in 0..count {
        let hdr = rest
            .get(..TWIN_TRAILER_RECORD_HEADER_BYTES)
            .ok_or_else(|| anyhow!("Truncated twin trailer record {idx}"))?;
        let frames = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let name_len = u16::from_le_bytes([hdr[4], hdr[5]]) as usize;
        rest = &rest[TWIN_TRAILER_RECORD_HEADER_BYTES..];

        let name_bytes = rest
            .get(..name_len)
            .ok_or_else(|| anyhow!("Truncated twin trailer filename {idx}"))?;
        let fname = std::str::from_utf8(name_bytes)
            .with_context(|| format!("Invalid UTF-8 in twin trailer record {idx}"))?;
        packed_by_filename
            .entry(fname.to_string())
            .or_default()
            .push(frames);
        rest = &rest[name_len..];
    }
    Ok(())
}

/// Validates per-entry slice/UTF-8 invariants and constructs fast $\mathcal{O}(1)$ delta lookup maps.
fn validate_and_index_entries(
    header: &DbahashHeader,
    entries: &[DbahashMediaEntry],
    strings: &[u8],
) -> Result<EntryIndexMaps> {
    let mut coarse_offsets = Vec::with_capacity(entries.len() + 1);
    coarse_offsets.push(0);
    let mut packed_by_filename: HashMap<String, Vec<u32>> = HashMap::with_capacity(entries.len());
    let mut packed_by_frames: HashMap<u32, Vec<usize>> = HashMap::with_capacity(entries.len());
    let mut running_coarse = 0u64;
    let mut max_str_end = 0usize;

    for (idx, e) in entries.iter().enumerate() {
        let dense_end = e
            .dense_start_idx
            .checked_add(u64::from(e.dense_frame_count))
            .ok_or_else(|| anyhow!("Entry {idx} dense range overflow"))?;
        ensure!(
            dense_end <= header.total_dense_frames,
            "Entry {idx} dense range ({dense_end}) > total_dense_frames ({})",
            header.total_dense_frames
        );

        if header.has_coarse_mih32() {
            ensure!(
                e.coarse_start_idx == running_coarse,
                "Entry {idx} non-contiguous coarse_start_idx ({} != {running_coarse})",
                e.coarse_start_idx
            );
            running_coarse = running_coarse.saturating_add(u64::from(e.coarse_frame_count));
            ensure!(
                running_coarse <= header.total_coarse_frames,
                "Entry {idx} coarse range exceeds total_coarse_frames"
            );
        }
        coarse_offsets.push(running_coarse);

        let name_end = (e.name_offset as usize)
            .checked_add(e.name_len as usize)
            .ok_or_else(|| anyhow!("Entry {idx} name range overflow"))?;
        let name_bytes = strings
            .get(e.name_offset as usize..name_end)
            .ok_or_else(|| anyhow!("Entry {idx} name out of string pool bounds"))?;
        std::str::from_utf8(name_bytes)
            .with_context(|| format!("Entry {idx} media_name is invalid UTF-8"))?;

        let path_end = (e.path_offset as usize)
            .checked_add(e.path_len as usize)
            .ok_or_else(|| anyhow!("Entry {idx} path range overflow"))?;
        let path_bytes = strings
            .get(e.path_offset as usize..path_end)
            .ok_or_else(|| anyhow!("Entry {idx} file_path out of string pool bounds"))?;
        let path_str = std::str::from_utf8(path_bytes)
            .with_context(|| format!("Entry {idx} file_path is invalid UTF-8"))?;
        max_str_end = max_str_end.max(name_end).max(path_end);

        if let Some(fname) = Path::new(path_str).file_name().and_then(|s| s.to_str()) {
            packed_by_filename
                .entry(fname.to_string())
                .or_default()
                .push(e.dense_frame_count);
        }
        packed_by_frames
            .entry(e.dense_frame_count)
            .or_default()
            .push(idx);
    }

    parse_twin_alias_trailer(
        strings.get(max_str_end..).unwrap_or(&[]),
        &mut packed_by_filename,
    )?;

    Ok((coarse_offsets, packed_by_filename, packed_by_frames))
}

/// Builds a lazy in-RAM coarse `MIH-32` table if an archive was packed with `--no-mih32`.
fn build_fallback_coarse_tables(reader: &MmapDbahash) -> FallbackCoarseTables {
    let k = reader.factor_k();
    let entries = reader.media_entries();
    let mut coarse_offsets = Vec::with_capacity(entries.len() + 1);
    coarse_offsets.push(0);
    let mut flat_coarse = Vec::new();

    for idx in 0..entries.len() {
        if let Some(dense) = reader.media_dense_hashes(idx) {
            flat_coarse.extend(dense.iter().step_by(k).copied());
        }
        coarse_offsets.push(flat_coarse.len() as u64);
    }
    let mih32 = AHashMih32Index::build(&flat_coarse);
    FallbackCoarseTables {
        coarse_offsets,
        mih32,
    }
}
