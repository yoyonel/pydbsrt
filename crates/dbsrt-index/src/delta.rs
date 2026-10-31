//! In-memory dynamic perceptual hash delta index.
//!
//! Provides O(1) dynamic ingestion of newly arrived media frames in RAM without
//! requiring a full rewrite or recompilation of the primary immutable `.dbphash` catalog.
//! Searches are merged across Base (.dbphash) and Delta (RAM) using Multi-Index Hashing (MIH).

use crate::dbphash::{
    media_confidence, rank_media_matches, DbphashBuilder, DbphashCutEntry, DbphashCutMatch,
    DbphashMediaMatch, DbphashSparseEntry,
};
use crate::mih::MultiIndexHash64;
use dbsrt_core::cuts::detect_cuts;
use dbsrt_core::MediaMetadata;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

/// Default Hamming distance threshold for scene cut transition detection.
pub const DEFAULT_CUT_THRESHOLD: u32 = 16;

/// Media entry stored dynamically in RAM within the Delta Index.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeltaMediaEntry {
    pub media_id: u32,
    pub clean_title: String,
    pub media_type: dbsrt_core::MediaType,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub duration_ms: u64,
    pub fps: f64,
    pub cuts: Vec<DbphashCutEntry>,
    pub sparse: Vec<DbphashSparseEntry>,
}

/// Dynamic in-memory index for perceptual hashes.
/// Inner data structures are wrapped in `Arc` so readers can acquire a fast ~15 ns snapshot
/// and release the read lock immediately before performing Rayon searches lock-free.
#[derive(Debug, Clone, Default)]
pub struct DeltaIndex {
    medias: Arc<Vec<DeltaMediaEntry>>,
    cuts_entries: Arc<Vec<DbphashCutEntry>>,
    sparse_entries: Arc<Vec<DbphashSparseEntry>>,
    cuts_mih: Option<Arc<MultiIndexHash64>>,
    sparse_mih: Option<Arc<MultiIndexHash64>>,
}

/// Media stream item payload for batch insertion into `DeltaIndex`.
#[derive(Debug, Clone)]
pub struct DeltaBatchItem<'a> {
    pub meta: MediaMetadata,
    pub duration_ms: u64,
    pub fps: f64,
    pub cuts: &'a [(u64, u32)],
    pub sparse: &'a [(u64, u32)],
}

impl DeltaIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a zero-heap-allocation snapshot of the delta index (5 Arc clones, ~15 ns).
    /// Readers can query the snapshot without holding any lock, eliminating lock contention.
    #[inline(always)]
    pub fn snapshot(&self) -> Self {
        self.clone()
    }

    /// Returns true if no media is currently indexed in the delta tier.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.medias.is_empty()
    }

    /// Number of distinct media items stored in the delta tier.
    #[inline(always)]
    pub fn media_count(&self) -> usize {
        self.medias.len()
    }

    /// Total number of scene cut hashes indexed in RAM.
    #[inline(always)]
    pub fn cuts_count(&self) -> usize {
        self.cuts_entries.len()
    }

    /// Total number of sparse (1 fps) hashes indexed in RAM.
    #[inline(always)]
    pub fn sparse_count(&self) -> usize {
        self.sparse_entries.len()
    }

    /// Slice of all media entries in the delta tier.
    pub fn medias(&self) -> &[DeltaMediaEntry] {
        &self.medias
    }

    /// Appends all media, cuts, and sparse entries in this delta index into a `DbphashBuilder`.
    pub fn export_to_builder(&self, builder: &mut DbphashBuilder) {
        for media in self.medias.iter() {
            let meta = MediaMetadata {
                raw_title: media.clean_title.clone(),
                clean_title: media.clean_title.clone(),
                media_type: media.media_type,
                year: media.year,
                season: media.season,
                episode: media.episode,
                resolution: None,
                source: None,
                release_group: None,
                moviehash: None,
            };

            let cuts: Vec<(u64, u32)> = media
                .cuts
                .iter()
                .map(|c| (c.phash, c.timestamp_ms))
                .collect();
            let sparse: Vec<(u64, u32)> = media
                .sparse
                .iter()
                .map(|s| (s.phash, s.timestamp_ms))
                .collect();

            builder.add_media(&meta, media.duration_ms, media.fps, &cuts, &sparse);
        }
    }

    /// Clears all entries from the delta index.
    pub fn clear(&mut self) {
        self.medias = Arc::new(Vec::new());
        self.cuts_entries = Arc::new(Vec::new());
        self.sparse_entries = Arc::new(Vec::new());
        self.cuts_mih = None;
        self.sparse_mih = None;
    }

    /// Consumes and returns all media entries from the delta index.
    pub fn drain_medias(&mut self) -> Vec<DeltaMediaEntry> {
        let old_arc = std::mem::take(&mut self.medias);
        self.cuts_entries = Arc::new(Vec::new());
        self.sparse_entries = Arc::new(Vec::new());
        self.cuts_mih = None;
        self.sparse_mih = None;
        Arc::try_unwrap(old_arc).unwrap_or_else(|arc| (*arc).clone())
    }

    /// Restores multiple delta entries in a single batch with a single MIH rebuild.
    pub fn add_delta_entries_batch(&mut self, entries: Vec<DeltaMediaEntry>) {
        if entries.is_empty() {
            return;
        }

        let new_medias = Arc::make_mut(&mut self.medias);
        let new_cuts = Arc::make_mut(&mut self.cuts_entries);
        let new_sparse = Arc::make_mut(&mut self.sparse_entries);

        for mut entry in entries {
            let media_id = 1_000_000 + new_medias.len() as u32 + 1;

            for c in &entry.cuts {
                new_cuts.push(DbphashCutEntry {
                    phash: c.phash,
                    timestamp_ms: c.timestamp_ms,
                    media_id,
                });
            }
            for s in &entry.sparse {
                new_sparse.push(DbphashSparseEntry {
                    phash: s.phash,
                    timestamp_ms: s.timestamp_ms,
                    media_id,
                });
            }

            entry.media_id = media_id;
            new_medias.push(entry);
        }

        self.cuts_mih = if !new_cuts.is_empty() {
            Some(Arc::new(MultiIndexHash64::build_from_provider(
                &new_cuts[..],
            )))
        } else {
            None
        };
        self.sparse_mih = if !new_sparse.is_empty() {
            Some(Arc::new(MultiIndexHash64::build_from_provider(
                &new_sparse[..],
            )))
        } else {
            None
        };
    }

    /// Restores a previously extracted delta media entry (used for rollback).
    pub fn add_delta_entry(&mut self, entry: DeltaMediaEntry) {
        self.add_delta_entries_batch(vec![entry]);
    }

    /// Adds multiple media streams in batch, rebuilding MIH tables once in O(N).
    pub fn add_media_batch(&mut self, items: &[DeltaBatchItem<'_>]) -> Vec<u32> {
        let new_medias = Arc::make_mut(&mut self.medias);
        let new_cuts = Arc::make_mut(&mut self.cuts_entries);
        let new_sparse = Arc::make_mut(&mut self.sparse_entries);
        let mut ids = Vec::with_capacity(items.len());

        for item in items {
            let media_id = 1_000_000 + new_medias.len() as u32 + 1;
            let mut delta_cuts = Vec::with_capacity(item.cuts.len());
            for &(phash, timestamp_ms) in item.cuts {
                let entry = DbphashCutEntry {
                    phash,
                    timestamp_ms,
                    media_id,
                };
                new_cuts.push(entry);
                delta_cuts.push(entry);
            }

            let mut delta_sparse = Vec::with_capacity(item.sparse.len());
            for &(phash, timestamp_ms) in item.sparse {
                let entry = DbphashSparseEntry {
                    phash,
                    timestamp_ms,
                    media_id,
                };
                new_sparse.push(entry);
                delta_sparse.push(entry);
            }

            new_medias.push(DeltaMediaEntry {
                media_id,
                clean_title: item.meta.clean_title.clone(),
                media_type: item.meta.media_type,
                year: item.meta.year,
                season: item.meta.season,
                episode: item.meta.episode,
                duration_ms: item.duration_ms,
                fps: item.fps,
                cuts: delta_cuts,
                sparse: delta_sparse,
            });
            ids.push(media_id);
        }

        // Rebuild MIH once across entire batch
        self.cuts_mih = if !new_cuts.is_empty() {
            Some(Arc::new(MultiIndexHash64::build_from_provider(
                &new_cuts[..],
            )))
        } else {
            None
        };

        self.sparse_mih = if !new_sparse.is_empty() {
            Some(Arc::new(MultiIndexHash64::build_from_provider(
                &new_sparse[..],
            )))
        } else {
            None
        };

        ids
    }

    /// Adds a single media with explicit cuts and sparse hashes to the delta index.
    pub fn add_media(
        &mut self,
        meta: &MediaMetadata,
        duration_ms: u64,
        fps: f64,
        cuts: &[(u64, u32)],
        sparse: &[(u64, u32)],
    ) -> u32 {
        self.add_media_batch(&[DeltaBatchItem {
            meta: meta.clone(),
            duration_ms,
            fps,
            cuts,
            sparse,
        }])[0]
    }

    /// Ingests a raw frame hash sequence into the delta index.
    ///
    /// Automatically performs:
    /// - Filename metadata parsing (clean title, year, season, episode)
    /// - Cut detection
    /// - 1 fps sparse sampling
    /// - MIH RAM table update
    pub fn ingest_raw_frames(&mut self, filename: &str, hashes: &[u64], fps: f64) -> u32 {
        let meta = MediaMetadata::parse(filename);
        let safe_fps = if fps > 0.0 { fps } else { 24.0 };
        let duration_ms = (hashes.len() as f64 / safe_fps * 1000.0).round() as u64;

        // 1. Detect scene cuts
        let cut_items = detect_cuts(hashes, DEFAULT_CUT_THRESHOLD);
        let cuts: Vec<(u64, u32)> = cut_items
            .into_iter()
            .map(|c| {
                let ts_ms = (c.offset as f64 / safe_fps * 1000.0).round() as u32;
                (c.phash, ts_ms)
            })
            .collect();

        // 2. Sample sparse 1 fps frames
        let stride = (safe_fps.round() as usize).max(1);
        let sparse: Vec<(u64, u32)> = hashes
            .iter()
            .enumerate()
            .step_by(stride)
            .map(|(idx, &h)| {
                let ts_ms = (idx as f64 / safe_fps * 1000.0).round() as u32;
                (h, ts_ms)
            })
            .collect();

        self.add_media(&meta, duration_ms, safe_fps, &cuts, &sparse)
    }

    /// Search the delta index using Multi-Index Hashing (MIH).
    pub fn recognize(&self, query_hashes: &[u64], max_distance: u32) -> Vec<DbphashMediaMatch> {
        if self.is_empty() || query_hashes.is_empty() {
            return Vec::new();
        }

        let sparse_matches: Vec<DbphashCutMatch> = if let Some(ref mih) = self.sparse_mih {
            let sparse = &self.sparse_entries[..];
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
        } else {
            Vec::new()
        };

        let cuts_matches: Vec<DbphashCutMatch> = if let Some(ref mih) = self.cuts_mih {
            let cuts = &self.cuts_entries[..];
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
        } else {
            Vec::new()
        };

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

        let mut sparse_grouped: HashMap<u32, Vec<DbphashCutMatch>> = HashMap::new();
        for m in sparse_matches {
            sparse_grouped
                .entry(m.cut_entry.media_id)
                .or_default()
                .push(m);
        }

        let mut cuts_grouped: HashMap<u32, Vec<DbphashCutMatch>> = HashMap::new();
        for m in cuts_matches {
            cuts_grouped
                .entry(m.cut_entry.media_id)
                .or_default()
                .push(m);
        }

        let mut all_media_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
        all_media_ids.extend(sparse_grouped.keys());
        all_media_ids.extend(cuts_grouped.keys());

        let media_map: HashMap<u32, &DeltaMediaEntry> =
            self.medias.iter().map(|m| (m.media_id, m)).collect();

        let mut ranked = Vec::new();

        for media_id in all_media_ids {
            if let Some(media) = media_map.get(&media_id) {
                let s_matches = sparse_grouped.get(&media_id);
                let c_matches = cuts_grouped.get(&media_id);

                let matching_sparse = s_matches.map(|v| v.len()).unwrap_or(0);
                let matching_cuts = c_matches.map(|v| v.len()).unwrap_or(0);

                let fps = media.fps.max(1.0);
                let (confidence, estimated_shift_ms, coherent_distance) = media_confidence(
                    query_hashes.len(),
                    fps,
                    s_matches.map_or(&[][..], Vec::as_slice),
                    c_matches.map_or(&[][..], Vec::as_slice),
                );

                ranked.push(DbphashMediaMatch {
                    media_id,
                    clean_title: media.clean_title.clone(),
                    year: media.year,
                    season: media.season,
                    episode: media.episode,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_index_empty() {
        let delta = DeltaIndex::new();
        assert!(delta.is_empty());
        assert_eq!(delta.media_count(), 0);
        let res = delta.recognize(&[0x1234], 4);
        assert!(res.is_empty());
    }

    #[test]
    fn test_delta_index_ingest_and_search() {
        let mut delta = DeltaIndex::new();
        let hashes = vec![0xAAAA_BBBB_CCCC_DDDD; 100];
        let id = delta.ingest_raw_frames("Inception.2010.1080p.mkv", &hashes, 24.0);
        assert!(id >= 1_000_000);
        assert_eq!(delta.media_count(), 1);
        assert!(!delta.is_empty());

        let query = vec![0xAAAA_BBBB_CCCC_DDDD; 10];
        let matches = delta.recognize(&query, 2);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].clean_title, "Inception");
        assert_eq!(matches[0].year, Some(2010));
    }

    /// Same as the `.dbphash` case: a static decoy scene must not outrank the source media.
    #[test]
    fn test_delta_recognize_prefers_coherent_media_over_static_decoy() {
        let frame = |k: u64| (k + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let sparse_source: Vec<(u64, u32)> = (0..10).map(|k| (frame(k), k as u32 * 1000)).collect();
        let sparse_decoy: Vec<(u64, u32)> = (0..60).map(|k| (frame(0), k * 1000)).collect();
        let mut delta = DeltaIndex::new();
        delta.add_media(
            &MediaMetadata::parse("Decoy.2001.mkv"),
            60_000,
            24.0,
            &[],
            &sparse_decoy,
        );
        delta.add_media(
            &MediaMetadata::parse("Source.2002.mkv"),
            10_000,
            24.0,
            &[],
            &sparse_source,
        );

        let query: Vec<u64> = (0..240).map(|i| frame(i / 24)).collect();
        let ranked = delta.recognize(&query, 0);

        assert_eq!(ranked[0].clean_title, "Source");
        assert_eq!(ranked[0].confidence, 100.0);
        let decoy = ranked.iter().find(|m| m.clean_title == "Decoy").unwrap();
        assert!(
            decoy.confidence <= 30.0,
            "decoy confidence {}",
            decoy.confidence
        );
    }

    /// Same as the `.dbphash` case: at equal confidence the closest media ranks first.
    #[test]
    fn test_delta_recognize_tie_prefers_closest_media() {
        let frame = |k: u64| (k + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut sparse_source: Vec<(u64, u32)> =
            (0..10).map(|k| (frame(k), k as u32 * 1000)).collect();
        sparse_source.extend((20..26).map(|k| (frame(0), k * 1000)));
        let sparse_rival: Vec<(u64, u32)> = (0..30)
            .map(|j| (frame(j / 3) ^ 0b111, (j / 3) as u32 * 1000))
            .collect();
        let mut delta = DeltaIndex::new();
        delta.add_media(
            &MediaMetadata::parse("Rival.2001.mkv"),
            10_000,
            24.0,
            &[],
            &sparse_rival,
        );
        delta.add_media(
            &MediaMetadata::parse("Source.2002.mkv"),
            26_000,
            24.0,
            &[],
            &sparse_source,
        );

        let query: Vec<u64> = (0..240).map(|i| frame(i / 24)).collect();
        let ranked = delta.recognize(&query, 4);

        assert_eq!(ranked[0].confidence, 100.0);
        assert_eq!(ranked[1].confidence, 100.0);
        assert_eq!(ranked[0].clean_title, "Source");
    }

    #[test]
    fn test_delta_index_export_to_builder() {
        let mut delta = DeltaIndex::new();
        let hashes = vec![0x1111_2222_3333_4444; 100];
        delta.ingest_raw_frames("Matrix.1999.1080p.mkv", &hashes, 24.0);

        let mut builder = DbphashBuilder::new();
        delta.export_to_builder(&mut builder);

        let temp_dir = tempfile::tempdir().unwrap();
        let catalog_path = temp_dir.path().join("catalog.dbphash");
        let stats = builder.write_to_file(&catalog_path).unwrap();
        assert_eq!(stats.media_count, 1);

        let reader = crate::dbphash::MmapDbphash::open(&catalog_path).unwrap();
        assert_eq!(reader.media_entries().len(), 1);
        assert_eq!(reader.get_title(&reader.media_entries()[0]), "Matrix");
    }
}
