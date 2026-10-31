//! High-density binary unified perceptual hash format (.dbphash v2.0 - Hybrid).
//!
//! Provides 64-byte aligned binary index specification, zero-copy memory-mapped
//! reader, and high-throughput compilation builder for packing millions of cuts,
//! sparse 1 fps anchor frames, and media metadata entries into a single compact file.

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use dbsrt_core::metadata::{MediaMetadata, MediaType};
use dbsrt_core::phash::hamming_distance;
use memmap2::Mmap;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const DBPHASH_MAGIC: [u8; 8] = *b"DBSRT\x02\x00\x00";
pub const DBPHASH_VERSION: u16 = 2;

/// Fixed 128-byte header for .dbphash files (version 2.0).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct DbphashHeader {
    /// Magic identifier: b"DBSRT\x02\x00\x00"
    pub magic: [u8; 8],
    /// Format version (2)
    pub version: u16,
    /// Header flags (reserved, 0)
    pub flags: u16,
    /// Total number of media entries in media table
    pub media_count: u32,
    /// Total number of cuts in flat cuts table
    pub total_cuts_count: u64,
    /// Total number of sparse (1 fps) frames in flat sparse table
    pub total_sparse_count: u64,
    /// Byte offset from start of file to media table
    pub offset_media_table: u64,
    /// Byte offset from start of file to cuts table
    pub offset_cuts_table: u64,
    /// Byte offset from start of file to sparse table
    pub offset_sparse_table: u64,
    /// Byte offset from start of file to string table
    pub offset_strings_table: u64,
    /// Byte length of string table
    pub strings_len: u64,
    /// Simple 64-bit checksum
    pub checksum: u64,
    /// Reserved padding to align to 128 bytes
    pub _reserved: [u8; 48],
}

/// Fixed 64-byte media descriptor in the media table.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct DbphashMediaEntry {
    /// 1-based unique identifier for this media
    pub media_id: u32,
    /// Media classification (1=Movie, 2=TvEpisode, 3=Anime, 4=Other)
    pub media_type: u8,
    pub _reserved1: u8,
    /// Production or release year (or 0)
    pub year: u16,
    /// Season index (or 0)
    pub season: u32,
    /// Episode index (or 0)
    pub episode: u32,
    /// Approximate or probed duration in milliseconds
    pub duration_ms: u64,
    /// Framerate multiplied by 1000 (e.g. 25000 for 25.0 fps, 23976 for 23.976 fps)
    pub fps_x1000: u32,
    /// Byte offset in string table
    pub title_offset: u32,
    /// Byte length of clean title in string table
    pub title_len: u32,
    /// Total number of scene cuts for this media
    pub cuts_count: u32,
    /// Starting index in flat cuts table
    pub cuts_start_idx: u64,
    /// Total number of sparse 1 fps frames for this media
    pub sparse_count: u32,
    /// Starting index in flat sparse table
    pub sparse_start_idx: u32,
    pub _padding: [u8; 8],
}

impl DbphashMediaEntry {
    pub fn media_type_enum(&self) -> MediaType {
        match self.media_type {
            1 => MediaType::Movie,
            2 => MediaType::TvEpisode,
            3 => MediaType::Anime,
            _ => MediaType::Other,
        }
    }
}

/// Fixed 16-byte cut or sparse frame entry in flat tables.
#[repr(C)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable, serde::Serialize, serde::Deserialize,
)]
pub struct DbphashCutEntry {
    /// 64-bit perceptual hash (2D DCT-II SIMD AVX2)
    pub phash: u64,
    /// Presentation timestamp in milliseconds from start of media
    pub timestamp_ms: u32,
    /// Reference media ID
    pub media_id: u32,
}

pub type DbphashSparseEntry = DbphashCutEntry;

/// Search match against a single cut or sparse entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbphashCutMatch {
    pub query_idx: usize,
    pub cut_entry: DbphashCutEntry,
    pub distance: u32,
}

/// Aggregated media match across matching cuts and sparse frames.
#[derive(Debug, Clone)]
pub struct DbphashMediaMatch {
    pub media_id: u32,
    pub clean_title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub matching_cuts: usize,
    pub matching_sparse: usize,
    /// Mean Hamming distance of the matches in the best time-shift buckets (sparse + cuts);
    /// infinite without any coherent match.
    pub coherent_distance: f64,
    pub confidence: f64,
    pub estimated_shift_ms: i64,
}

/// Ranking order: confidence descending, then mean coherent Hamming distance ascending, then
/// title, season and episode so that exact ties never fall back to hash-map order.
///
/// At equal confidence the closest media wins: dark or static scenes of other media match the
/// query frames too, but at a larger distance than the frames the query was taken from.
pub fn rank_media_matches(a: &DbphashMediaMatch, b: &DbphashMediaMatch) -> std::cmp::Ordering {
    b.confidence
        .total_cmp(&a.confidence)
        .then_with(|| a.coherent_distance.total_cmp(&b.coherent_distance))
        .then_with(|| a.clean_title.cmp(&b.clean_title))
        .then_with(|| (a.season, a.episode).cmp(&(b.season, b.episode)))
}

/// Summary statistics for a packaging run.
#[derive(Debug, Clone)]
pub struct DbphashPackStats {
    pub media_count: usize,
    pub total_cuts_count: usize,
    pub total_sparse_count: usize,
    pub output_bytes: u64,
    pub duration_seconds: f64,
}

/// In-memory builder for constructing and serializing `.dbphash` index files.
#[derive(Default)]
pub struct DbphashBuilder {
    media_records: Vec<(DbphashMediaEntry, String)>,
    cuts_records: Vec<DbphashCutEntry>,
    sparse_records: Vec<DbphashSparseEntry>,
}

impl DbphashBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a media file with its cuts and sparse 1 fps frames into the builder.
    pub fn add_media(
        &mut self,
        meta: &MediaMetadata,
        duration_ms: u64,
        fps: f64,
        cuts: &[(u64, u32)],   // (phash, timestamp_ms)
        sparse: &[(u64, u32)], // (phash, timestamp_ms)
    ) -> u32 {
        let media_id = (self.media_records.len() + 1) as u32;
        let cuts_start_idx = self.cuts_records.len() as u64;
        let cuts_count = cuts.len() as u32;
        let sparse_start_idx = self.sparse_records.len() as u32;
        let sparse_count = sparse.len() as u32;

        for &(phash, timestamp_ms) in cuts {
            self.cuts_records.push(DbphashCutEntry {
                phash,
                timestamp_ms,
                media_id,
            });
        }

        for &(phash, timestamp_ms) in sparse {
            self.sparse_records.push(DbphashCutEntry {
                phash,
                timestamp_ms,
                media_id,
            });
        }

        let media_type_byte = match meta.media_type {
            MediaType::Movie => 1,
            MediaType::TvEpisode => 2,
            MediaType::Anime => 3,
            MediaType::Other => 4,
        };

        let fps_x1000 = (fps * 1000.0).round() as u32;

        let entry = DbphashMediaEntry {
            media_id,
            media_type: media_type_byte,
            _reserved1: 0,
            year: meta.year.unwrap_or(0),
            season: meta.season.unwrap_or(0),
            episode: meta.episode.unwrap_or(0),
            duration_ms,
            fps_x1000,
            title_offset: 0, // Will be computed during final layout
            title_len: meta.clean_title.len() as u32,
            cuts_count,
            cuts_start_idx,
            sparse_count,
            sparse_start_idx,
            _padding: [0u8; 8],
        };

        self.media_records.push((entry, meta.clean_title.clone()));
        media_id
    }

    /// Compiles and serializes the complete binary database to a `.dbphash` file.
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> Result<DbphashPackStats> {
        let start = std::time::Instant::now();
        let path_ref = path.as_ref();

        // 1. Build string table
        let mut string_table = Vec::new();
        let mut final_media_entries = Vec::with_capacity(self.media_records.len());

        for (mut entry, title) in &self.media_records {
            entry.title_offset = string_table.len() as u32;
            entry.title_len = title.len() as u32;
            string_table.extend_from_slice(title.as_bytes());
            final_media_entries.push(entry);
        }

        // Align string table to 8 bytes
        while string_table.len() % 8 != 0 {
            string_table.push(0);
        }

        // 2. Compute table offsets (aligned to 64 bytes)
        let header_size = std::mem::size_of::<DbphashHeader>() as u64; // 128
        let media_table_size =
            (final_media_entries.len() * std::mem::size_of::<DbphashMediaEntry>()) as u64;
        let cuts_table_size =
            (self.cuts_records.len() * std::mem::size_of::<DbphashCutEntry>()) as u64;
        let sparse_table_size =
            (self.sparse_records.len() * std::mem::size_of::<DbphashSparseEntry>()) as u64;

        let offset_media_table = header_size;
        let offset_cuts_table = offset_media_table + media_table_size;
        let offset_sparse_table = offset_cuts_table + cuts_table_size;
        let offset_strings_table = offset_sparse_table + sparse_table_size;

        // Simple checksum over counts and lengths
        let checksum = (final_media_entries.len() as u64).wrapping_mul(31)
            ^ (self.cuts_records.len() as u64).wrapping_mul(17)
            ^ (self.sparse_records.len() as u64).wrapping_mul(13)
            ^ (string_table.len() as u64);

        let header = DbphashHeader {
            magic: DBPHASH_MAGIC,
            version: DBPHASH_VERSION,
            flags: 0,
            media_count: final_media_entries.len() as u32,
            total_cuts_count: self.cuts_records.len() as u64,
            total_sparse_count: self.sparse_records.len() as u64,
            offset_media_table,
            offset_cuts_table,
            offset_sparse_table,
            offset_strings_table,
            strings_len: string_table.len() as u64,
            checksum,
            _reserved: [0u8; 48],
        };

        // 3. Write contiguous bytes to disk
        let mut file = File::create(path_ref)
            .with_context(|| format!("Failed to create .dbphash output at {:?}", path_ref))?;

        file.write_all(bytemuck::bytes_of(&header))?;
        file.write_all(bytemuck::cast_slice(&final_media_entries))?;
        file.write_all(bytemuck::cast_slice(&self.cuts_records))?;
        file.write_all(bytemuck::cast_slice(&self.sparse_records))?;
        file.write_all(&string_table)?;
        file.flush()?;

        let output_bytes = file.metadata()?.len();

        Ok(DbphashPackStats {
            media_count: final_media_entries.len(),
            total_cuts_count: self.cuts_records.len(),
            total_sparse_count: self.sparse_records.len(),
            output_bytes,
            duration_seconds: start.elapsed().as_secs_f64(),
        })
    }
}

/// Most populated time-shift bucket of one media's matches:
/// `(shift_ms, matched pairs, distinct target frames, sum of Hamming distances)`.
fn best_shift_bucket(
    matches: &[DbphashCutMatch],
    fps: f64,
    bucket_ms: i64,
) -> (i64, usize, usize, u64) {
    let mut buckets: HashMap<i64, (usize, HashSet<u32>, u64)> = HashMap::new();
    for m in matches {
        let query_ts_ms = (m.query_idx as f64 / fps * 1000.0) as i64;
        let diff = m.cut_entry.timestamp_ms as i64 - query_ts_ms;
        let bucket = buckets.entry((diff / bucket_ms) * bucket_ms).or_default();
        bucket.0 += 1;
        bucket.1.insert(m.cut_entry.timestamp_ms);
        bucket.2 += u64::from(m.distance);
    }
    buckets
        .into_iter()
        .max_by_key(|(_, (pairs, _, _))| *pairs)
        .map(|(shift, (pairs, targets, dist))| (shift, pairs, targets.len(), dist))
        .unwrap_or((0, 0, 0, 0))
}

/// Confidence (0-100), estimated shift and mean coherent Hamming distance of one media from its
/// sparse and cut matches.
///
/// Random cross-media collisions in large indices have scattered deltas (count <= 1 or 2)
/// and a tiny match ratio (< 5% of query frames). The sparse ratio counts distinct target
/// frames: a static scene matched by every query frame is one frame, not a full window.
pub(crate) fn media_confidence(
    query_frames: usize,
    fps: f64,
    s_matches: &[DbphashCutMatch],
    c_matches: &[DbphashCutMatch],
) -> (f64, i64, f64) {
    let expected_sparse = ((query_frames as f64 / fps).round() as usize).max(1);
    let (best_cut_shift_ms, best_coherent_cuts, _, cut_distance) =
        best_shift_bucket(c_matches, fps, 50);
    let (best_sparse_shift_ms, best_coherent_sparse, coherent_sparse_frames, sparse_distance) =
        best_shift_bucket(s_matches, fps, 500);

    let query_len = query_frames.max(1);
    let query_match_ratio =
        (s_matches.len().max(c_matches.len()) as f64 / query_len as f64).min(1.0);
    let coherent_matches = best_coherent_sparse + best_coherent_cuts;
    let coherent_distance = if coherent_matches == 0 {
        f64::INFINITY
    } else {
        (cut_distance + sparse_distance) as f64 / coherent_matches as f64
    };

    let (confidence, shift_ms) = if best_coherent_sparse >= 3 {
        let sparse_ratio = (coherent_sparse_frames as f64 / expected_sparse as f64).min(1.0);
        let base_conf = sparse_ratio * 100.0;
        if best_coherent_cuts >= 2 {
            let conf = (base_conf * 0.7 + (best_coherent_cuts as f64 * 15.0).min(30.0)).min(100.0);
            (conf, best_cut_shift_ms)
        } else {
            (base_conf, best_sparse_shift_ms)
        }
    } else if best_coherent_cuts >= 3 {
        // For short 1-second queries (query_len <= 30), 3 coherent cuts correspond to a verified match
        let conf = if expected_sparse <= 1 && query_len <= 30 {
            95.0
        } else {
            (best_coherent_cuts as f64 * 25.0).min(90.0)
        };
        (conf, best_cut_shift_ms)
    } else if query_len <= 5 && query_match_ratio >= 0.9 {
        // Micro-query where 100% of query frames matched both cuts and sparse
        (query_match_ratio * 100.0, 0i64)
    } else {
        // Incoherent noise / random collision across large index: reject
        ((query_match_ratio * 100.0).min(30.0), 0i64)
    };
    (confidence, shift_ms, coherent_distance)
}

/// Zero-copy memory-mapped reader for `.dbphash` binary index files.
pub struct MmapDbphash {
    pub path: PathBuf,
    _file: File,
    mmap: Mmap,
    header: DbphashHeader,
    sparse_mih: std::sync::OnceLock<crate::mih::MultiIndexHash64>,
    cuts_mih: std::sync::OnceLock<crate::mih::MultiIndexHash64>,
}

impl MmapDbphash {
    /// Opens and maps a `.dbphash` index file into memory.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let p = path.as_ref().to_path_buf();
        let file = File::open(&p).with_context(|| format!("Failed to open .dbphash at {:?}", p))?;
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < std::mem::size_of::<DbphashHeader>() {
            return Err(anyhow!("File is too small to be a valid .dbphash file"));
        }

        let header_bytes = &mmap[..std::mem::size_of::<DbphashHeader>()];
        let header: DbphashHeader = *bytemuck::from_bytes(header_bytes);

        if header.magic != DBPHASH_MAGIC {
            return Err(anyhow!("Invalid .dbphash magic bytes: {:?}", header.magic));
        }

        if header.version != DBPHASH_VERSION {
            return Err(anyhow!(
                "Unsupported .dbphash version: {} (expected {})",
                header.version,
                DBPHASH_VERSION
            ));
        }

        // Validate table offsets
        let expected_min_len = header.offset_strings_table + header.strings_len;
        if (mmap.len() as u64) < expected_min_len {
            return Err(anyhow!(
                "Corrupt .dbphash file: length {} is less than expected layout {}",
                mmap.len(),
                expected_min_len
            ));
        }

        Ok(Self {
            path: p,
            _file: file,
            mmap,
            header,
            sparse_mih: std::sync::OnceLock::new(),
            cuts_mih: std::sync::OnceLock::new(),
        })
    }

    #[inline]
    pub fn header(&self) -> &DbphashHeader {
        &self.header
    }

    /// Slice of media descriptor entries.
    #[inline]
    pub fn media_entries(&self) -> &[DbphashMediaEntry] {
        let start = self.header.offset_media_table as usize;
        let count = self.header.media_count as usize;
        let size = std::mem::size_of::<DbphashMediaEntry>();
        let end = start + count * size;
        bytemuck::cast_slice(&self.mmap[start..end])
    }

    /// Slice of contiguous cut entries.
    #[inline]
    pub fn cuts_entries(&self) -> &[DbphashCutEntry] {
        let start = self.header.offset_cuts_table as usize;
        let count = self.header.total_cuts_count as usize;
        let size = std::mem::size_of::<DbphashCutEntry>();
        let end = start + count * size;
        bytemuck::cast_slice(&self.mmap[start..end])
    }

    /// Slice of contiguous sparse 1 fps entries.
    #[inline]
    pub fn sparse_entries(&self) -> &[DbphashSparseEntry] {
        let start = self.header.offset_sparse_table as usize;
        let count = self.header.total_sparse_count as usize;
        let size = std::mem::size_of::<DbphashSparseEntry>();
        let end = start + count * size;
        bytemuck::cast_slice(&self.mmap[start..end])
    }

    /// Returns the clean title string for a media entry.
    pub fn get_title(&self, entry: &DbphashMediaEntry) -> &str {
        let str_start = (self.header.offset_strings_table + entry.title_offset as u64) as usize;
        let str_end = str_start + entry.title_len as usize;
        if str_end <= self.mmap.len() {
            std::str::from_utf8(&self.mmap[str_start..str_end]).unwrap_or("")
        } else {
            ""
        }
    }

    /// Finds a media entry by its ID in O(1) via direct table index.
    pub fn find_media_by_id(&self, media_id: u32) -> Option<&DbphashMediaEntry> {
        if media_id == 0 {
            return None;
        }
        let entries = self.media_entries();
        let idx = (media_id - 1) as usize;
        entries.get(idx).filter(|e| e.media_id == media_id)
    }

    #[inline]
    pub fn sparse_mih(&self) -> &crate::mih::MultiIndexHash64 {
        self.sparse_mih.get_or_init(|| {
            crate::mih::MultiIndexHash64::build_from_provider(self.sparse_entries())
        })
    }

    #[inline]
    pub fn cuts_mih(&self) -> &crate::mih::MultiIndexHash64 {
        self.cuts_mih
            .get_or_init(|| crate::mih::MultiIndexHash64::build_from_provider(self.cuts_entries()))
    }

    /// High-throughput MIH-accelerated parallel search of query hashes against the cuts array.
    pub fn search_cuts(&self, query_hashes: &[u64], max_distance: u32) -> Vec<DbphashCutMatch> {
        let cuts = self.cuts_entries();
        if cuts.is_empty() || query_hashes.is_empty() {
            return Vec::new();
        }

        let mih = self.cuts_mih();
        query_hashes
            .par_iter()
            .enumerate()
            .flat_map_iter(|(q_idx, &q_hash)| {
                let candidates = mih.search(q_hash, max_distance, cuts);
                candidates.into_iter().map(move |c| DbphashCutMatch {
                    query_idx: q_idx,
                    cut_entry: cuts[c.index],
                    distance: c.distance,
                })
            })
            .collect()
    }

    /// High-throughput MIH-accelerated parallel search of query hashes against the sparse 1 fps array.
    pub fn search_sparse(&self, query_hashes: &[u64], max_distance: u32) -> Vec<DbphashCutMatch> {
        let sparse = self.sparse_entries();
        if sparse.is_empty() || query_hashes.is_empty() {
            return Vec::new();
        }

        let mih = self.sparse_mih();
        query_hashes
            .par_iter()
            .enumerate()
            .flat_map_iter(|(q_idx, &q_hash)| {
                let candidates = mih.search(q_hash, max_distance, sparse);
                candidates.into_iter().map(move |c| DbphashCutMatch {
                    query_idx: q_idx,
                    cut_entry: sparse[c.index],
                    distance: c.distance,
                })
            })
            .collect()
    }

    /// Linear ground-truth search of query hashes against cuts array (for verification/benchmarks).
    pub fn search_cuts_linear(
        &self,
        query_hashes: &[u64],
        max_distance: u32,
    ) -> Vec<DbphashCutMatch> {
        let cuts = self.cuts_entries();
        query_hashes
            .par_iter()
            .enumerate()
            .flat_map_iter(|(q_idx, &q_hash)| {
                cuts.iter().filter_map(move |&cut_entry| {
                    let dist = hamming_distance(q_hash, cut_entry.phash);
                    if dist <= max_distance {
                        Some(DbphashCutMatch {
                            query_idx: q_idx,
                            cut_entry,
                            distance: dist,
                        })
                    } else {
                        None
                    }
                })
            })
            .collect()
    }

    /// Linear ground-truth search of query hashes against sparse array (for verification/benchmarks).
    pub fn search_sparse_linear(
        &self,
        query_hashes: &[u64],
        max_distance: u32,
    ) -> Vec<DbphashCutMatch> {
        let sparse = self.sparse_entries();
        query_hashes
            .par_iter()
            .enumerate()
            .flat_map_iter(|(q_idx, &q_hash)| {
                sparse.iter().filter_map(move |&cut_entry| {
                    let dist = hamming_distance(q_hash, cut_entry.phash);
                    if dist <= max_distance {
                        Some(DbphashCutMatch {
                            query_idx: q_idx,
                            cut_entry,
                            distance: dist,
                        })
                    } else {
                        None
                    }
                })
            })
            .collect()
    }

    /// Two-tier recognition engine using accelerated MIH:
    /// - Tier 1: Identifies media candidates using sparse 1 fps frames (robust on short clips).
    /// - Tier 2: Refines time shift and confidence using scene cuts (sub-frame accuracy).
    pub fn recognize(&self, query_hashes: &[u64], max_distance: u32) -> Vec<DbphashMediaMatch> {
        let sparse_matches = self.search_sparse(query_hashes, max_distance);
        let cuts_matches = self.search_cuts(query_hashes, max_distance);
        self.aggregate_matches(query_hashes, sparse_matches, cuts_matches)
    }

    /// Two-tier recognition engine using linear scan (ground truth / benchmarks):
    pub fn recognize_linear(
        &self,
        query_hashes: &[u64],
        max_distance: u32,
    ) -> Vec<DbphashMediaMatch> {
        let sparse_matches = self.search_sparse_linear(query_hashes, max_distance);
        let cuts_matches = self.search_cuts_linear(query_hashes, max_distance);
        self.aggregate_matches(query_hashes, sparse_matches, cuts_matches)
    }

    fn aggregate_matches(
        &self,
        query_hashes: &[u64],
        sparse_matches: Vec<DbphashCutMatch>,
        cuts_matches: Vec<DbphashCutMatch>,
    ) -> Vec<DbphashMediaMatch> {
        if sparse_matches.is_empty() && cuts_matches.is_empty() {
            return Vec::new();
        }

        // Aggregate sparse matches by media_id
        let mut sparse_grouped: HashMap<u32, Vec<DbphashCutMatch>> = HashMap::new();
        for m in sparse_matches {
            sparse_grouped
                .entry(m.cut_entry.media_id)
                .or_default()
                .push(m);
        }

        // Aggregate cuts matches by media_id
        let mut cuts_grouped: HashMap<u32, Vec<DbphashCutMatch>> = HashMap::new();
        for m in cuts_matches {
            cuts_grouped
                .entry(m.cut_entry.media_id)
                .or_default()
                .push(m);
        }

        // Set of all candidate media_ids
        let mut all_media_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
        all_media_ids.extend(sparse_grouped.keys());
        all_media_ids.extend(cuts_grouped.keys());

        let mut ranked = Vec::new();

        for media_id in all_media_ids {
            if let Some(media) = self.find_media_by_id(media_id) {
                let s_matches = sparse_grouped.get(&media_id);
                let c_matches = cuts_grouped.get(&media_id);

                let matching_sparse = s_matches.map(|v| v.len()).unwrap_or(0);
                let matching_cuts = c_matches.map(|v| v.len()).unwrap_or(0);

                let fps = (media.fps_x1000 as f64 / 1000.0).max(1.0);
                let (confidence, estimated_shift_ms, coherent_distance) = media_confidence(
                    query_hashes.len(),
                    fps,
                    s_matches.map_or(&[][..], Vec::as_slice),
                    c_matches.map_or(&[][..], Vec::as_slice),
                );

                let clean_title = self.get_title(media).to_string();
                let year = if media.year > 0 {
                    Some(media.year)
                } else {
                    None
                };
                let season = if media.season > 0 {
                    Some(media.season)
                } else {
                    None
                };
                let episode = if media.episode > 0 {
                    Some(media.episode)
                } else {
                    None
                };

                ranked.push(DbphashMediaMatch {
                    media_id,
                    clean_title,
                    year,
                    season,
                    episode,
                    matching_cuts,
                    matching_sparse,
                    coherent_distance,
                    confidence,
                    estimated_shift_ms,
                });
            }
        }

        ranked.sort_by(rank_media_matches);
        ranked
    }

    /// Appends all media, cuts, and sparse entries in this memory-mapped catalog into a `DbphashBuilder`.
    /// Enables O(N) compaction of Base + Delta without requiring loose `.phash` source files.
    pub fn export_to_builder(&self, builder: &mut DbphashBuilder) {
        let cuts_all = self.cuts_entries();
        let sparse_all = self.sparse_entries();

        for media in self.media_entries() {
            let title = self.get_title(media);
            let meta = MediaMetadata {
                raw_title: title.to_string(),
                clean_title: title.to_string(),
                media_type: media.media_type_enum(),
                year: if media.year > 0 {
                    Some(media.year)
                } else {
                    None
                },
                season: if media.season > 0 {
                    Some(media.season)
                } else {
                    None
                },
                episode: if media.episode > 0 {
                    Some(media.episode)
                } else {
                    None
                },
                resolution: None,
                source: None,
                release_group: None,
                moviehash: None,
            };

            let fps = (media.fps_x1000 as f64 / 1000.0).max(1.0);
            let cuts_end = (media.cuts_start_idx + media.cuts_count as u64) as usize;
            let cuts_slice = if (media.cuts_start_idx as usize) <= cuts_all.len()
                && cuts_end <= cuts_all.len()
            {
                &cuts_all[media.cuts_start_idx as usize..cuts_end]
            } else {
                &[]
            };

            let sparse_end = (media.sparse_start_idx + media.sparse_count) as usize;
            let sparse_slice = if (media.sparse_start_idx as usize) <= sparse_all.len()
                && sparse_end <= sparse_all.len()
            {
                &sparse_all[media.sparse_start_idx as usize..sparse_end]
            } else {
                &[]
            };

            let cuts: Vec<(u64, u32)> = cuts_slice
                .iter()
                .map(|c| (c.phash, c.timestamp_ms))
                .collect();
            let sparse: Vec<(u64, u32)> = sparse_slice
                .iter()
                .map(|s| (s.phash, s.timestamp_ms))
                .collect();

            builder.add_media(&meta, media.duration_ms, fps, &cuts, &sparse);
        }
    }
}
