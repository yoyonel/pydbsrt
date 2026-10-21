//! In-memory and memory-mapped search index for perceptual hashes.

pub mod bktree;
pub mod db;
pub mod dbphash;
pub mod engine;
pub mod mih;
pub mod mmap;

pub use bktree::{BkItem, BkTree};
pub use db::{parse_fps_from_phash_stem, DbClient, DbMatch, DbMediaRecord, DbSubtitleCoupling};
pub use dbphash::{
    DbphashBuilder, DbphashCutEntry, DbphashCutMatch, DbphashHeader, DbphashMediaEntry,
    DbphashMediaMatch, DbphashPackStats, MmapDbphash, DBPHASH_MAGIC, DBPHASH_VERSION,
};
use dbsrt_core::{hamming_distance, FrameHashEntry};
pub use engine::{PhashEngine, PhashEngineMatch, DEFAULT_MIN_RECOGNITION_CONFIDENCE};
pub use mih::{MihCandidate, MihTable, MultiIndexHash64};
pub use mmap::{
    compute_temporal_coherence, LibraryMediaMatch, MmapLibrary, MmapPhashFile,
    MIN_TEMPORAL_COHERENCE_CONFIDENCE,
};

/// A memory-mapped or in-memory flat frame hash index.
pub struct FlatHashIndex<'a> {
    entries: &'a [FrameHashEntry],
}

impl<'a> FlatHashIndex<'a> {
    pub fn new(entries: &'a [FrameHashEntry]) -> Self {
        Self { entries }
    }

    /// Search all entries matching within `max_distance` Hamming distance.
    pub fn search(&self, target_hash: u64, max_distance: u32) -> Vec<&FrameHashEntry> {
        self.entries
            .iter()
            .filter(|entry| hamming_distance(entry.phash, target_hash) <= max_distance)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flat_hash_index_search() {
        let entries = vec![
            FrameHashEntry {
                media_id: 1,
                frame_offset: 10,
                phash: 0b1010,
            },
            FrameHashEntry {
                media_id: 1,
                frame_offset: 20,
                phash: 0b1111_0000,
            },
        ];
        let index = FlatHashIndex::new(&entries);
        let exact = index.search(0b1010, 0);
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].frame_offset, 10);

        let near = index.search(0b1011, 1);
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].frame_offset, 10);

        let none = index.search(0xFFFF_FFFF_FFFF_FFFF, 2);
        assert!(none.is_empty());
    }
}
