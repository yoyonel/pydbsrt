//! In-memory dynamic perceptual hash delta index.
//!
//! Provides O(1) dynamic ingestion of newly arrived media frames in RAM without
//! requiring a full rewrite or recompilation of the primary immutable `.dbphash` catalog.
//! Searches are merged across Base (.dbphash) and Delta (RAM) using Multi-Index Hashing (MIH).

use crate::dbphash::{
    DbphashBuilder, DbphashCutEntry, DbphashCutMatch, DbphashMediaMatch, DbphashSparseEntry,
};
use crate::mih::MultiIndexHash64;
use dbsrt_core::cuts::detect_cuts;
use dbsrt_core::MediaMetadata;
use rayon::prelude::*;
use std::collections::HashMap;

/// Default Hamming distance threshold for scene cut transition detection.
pub const DEFAULT_CUT_THRESHOLD: u32 = 16;

/// Media entry stored dynamically in RAM within the Delta Index.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeltaMediaEntry {
    pub media_id: u32,
    pub clean_title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub duration_ms: u64,
    pub fps: f64,
    pub cuts: Vec<DbphashCutEntry>,
    pub sparse: Vec<DbphashSparseEntry>,
}

/// Dynamic in-memory index for perceptual hashes.
#[derive(Debug, Clone, Default)]
pub struct DeltaIndex {
    medias: Vec<DeltaMediaEntry>,
    cuts_entries: Vec<DbphashCutEntry>,
    sparse_entries: Vec<DbphashSparseEntry>,
    cuts_mih: Option<MultiIndexHash64>,
    sparse_mih: Option<MultiIndexHash64>,
}

impl DeltaIndex {
    pub fn new() -> Self {
        Self::default()
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
        for media in &self.medias {
            let meta = MediaMetadata {
                raw_title: media.clean_title.clone(),
                clean_title: media.clean_title.clone(),
                media_type: if media.season.is_some() || media.episode.is_some() {
                    dbsrt_core::MediaType::TvEpisode
                } else {
                    dbsrt_core::MediaType::Movie
                },
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
        self.medias.clear();
        self.cuts_entries.clear();
        self.sparse_entries.clear();
        self.cuts_mih = None;
        self.sparse_mih = None;
    }

    /// Adds a media with explicit cuts and sparse hashes to the delta index.
    pub fn add_media(
        &mut self,
        meta: &MediaMetadata,
        duration_ms: u64,
        fps: f64,
        cuts: &[(u64, u32)],
        sparse: &[(u64, u32)],
    ) -> u32 {
        let media_id = 1_000_000 + self.medias.len() as u32 + 1;

        let mut delta_cuts = Vec::with_capacity(cuts.len());
        for &(phash, timestamp_ms) in cuts {
            let entry = DbphashCutEntry {
                phash,
                timestamp_ms,
                media_id,
            };
            self.cuts_entries.push(entry);
            delta_cuts.push(entry);
        }

        let mut delta_sparse = Vec::with_capacity(sparse.len());
        for &(phash, timestamp_ms) in sparse {
            let entry = DbphashSparseEntry {
                phash,
                timestamp_ms,
                media_id,
            };
            self.sparse_entries.push(entry);
            delta_sparse.push(entry);
        }

        // Rebuild MIH tables for delta entries in RAM (sub-millisecond for thousands of items)
        if !self.cuts_entries.is_empty() {
            self.cuts_mih = Some(MultiIndexHash64::build_from_provider(
                &self.cuts_entries[..],
            ));
        }
        if !self.sparse_entries.is_empty() {
            self.sparse_mih = Some(MultiIndexHash64::build_from_provider(
                &self.sparse_entries[..],
            ));
        }

        self.medias.push(DeltaMediaEntry {
            media_id,
            clean_title: meta.clean_title.clone(),
            year: meta.year,
            season: meta.season,
            episode: meta.episode,
            duration_ms,
            fps,
            cuts: delta_cuts,
            sparse: delta_sparse,
        });

        media_id
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

        let mut ranked = Vec::new();

        for media_id in all_media_ids {
            if let Some(media) = self.medias.iter().find(|m| m.media_id == media_id) {
                let s_matches = sparse_grouped.get(&media_id);
                let c_matches = cuts_grouped.get(&media_id);

                let matching_sparse = s_matches.map(|v| v.len()).unwrap_or(0);
                let matching_cuts = c_matches.map(|v| v.len()).unwrap_or(0);

                let fps = media.fps.max(1.0);
                let expected_sparse = ((query_hashes.len() as f64 / fps).round() as usize).max(1);

                // Analyze coherent cut matches along timeline shift
                let mut best_coherent_cuts = 0usize;
                let mut best_cut_shift_ms = 0i64;
                if let Some(cm) = c_matches {
                    if !cm.is_empty() {
                        let mut diffs: HashMap<i64, usize> = HashMap::new();
                        for m in cm {
                            let query_ts_ms = (m.query_idx as f64 / fps * 1000.0) as i64;
                            let diff = m.cut_entry.timestamp_ms as i64 - query_ts_ms;
                            let bucket = (diff / 50) * 50;
                            *diffs.entry(bucket).or_default() += 1;
                        }
                        if let Some((&shift, &count)) =
                            diffs.iter().max_by_key(|&(_, count)| *count)
                        {
                            best_cut_shift_ms = shift;
                            best_coherent_cuts = count;
                        }
                    }
                }

                // Analyze coherent sparse matches along timeline shift
                let mut best_coherent_sparse = 0usize;
                let mut best_sparse_shift_ms = 0i64;
                if let Some(sm) = s_matches {
                    if !sm.is_empty() {
                        let mut diffs: HashMap<i64, usize> = HashMap::new();
                        for m in sm {
                            let query_ts_ms = (m.query_idx as f64 / fps * 1000.0) as i64;
                            let diff = m.cut_entry.timestamp_ms as i64 - query_ts_ms;
                            let bucket = (diff / 500) * 500;
                            *diffs.entry(bucket).or_default() += 1;
                        }
                        if let Some((&shift, &count)) =
                            diffs.iter().max_by_key(|&(_, count)| *count)
                        {
                            best_sparse_shift_ms = shift;
                            best_coherent_sparse = count;
                        }
                    }
                }

                let query_len = query_hashes.len().max(1);
                let query_match_ratio =
                    (matching_sparse.max(matching_cuts) as f64 / query_len as f64).min(1.0);

                let (confidence, estimated_shift_ms) = if best_coherent_sparse >= 3 {
                    let sparse_ratio =
                        (best_coherent_sparse as f64 / expected_sparse as f64).min(1.0);
                    let base_conf = sparse_ratio * 100.0;
                    let conf = if best_coherent_cuts >= 2 {
                        (base_conf * 0.7 + (best_coherent_cuts as f64 * 15.0).min(30.0)).min(100.0)
                    } else {
                        base_conf
                    };
                    let shift = if best_coherent_cuts >= 2 {
                        best_cut_shift_ms
                    } else {
                        best_sparse_shift_ms
                    };
                    (conf, shift)
                } else if best_coherent_cuts >= 3 {
                    let conf = if expected_sparse <= 1 && query_len <= 30 {
                        95.0
                    } else {
                        (best_coherent_cuts as f64 * 25.0).min(90.0)
                    };
                    (conf, best_cut_shift_ms)
                } else if query_len <= 5 && query_match_ratio >= 0.9 {
                    (query_match_ratio * 100.0, 0i64)
                } else {
                    ((query_match_ratio * 100.0).min(30.0), 0i64)
                };

                ranked.push(DbphashMediaMatch {
                    media_id,
                    clean_title: media.clean_title.clone(),
                    year: media.year,
                    season: media.season,
                    episode: media.episode,
                    matching_cuts,
                    matching_sparse,
                    confidence,
                    estimated_shift_ms,
                });
            }
        }

        ranked.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
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
