//! In-memory and memory-mapped search index for perceptual hashes.

pub mod bktree;
pub mod db;
pub mod dbphash;
pub mod delta;
pub mod engine;
pub mod mih;
pub mod mmap;

pub use bktree::{BkItem, BkTree};
pub use db::{
    parse_fps_from_phash_stem, validate_lang_code, DbClient, DbMatch, DbMediaRecord,
    DbSubtitleCoupling, COPY_BUFFER_FLUSH_THRESHOLD, COPY_BUFFER_INITIAL_CAPACITY,
    DB_CONNECT_TIMEOUT_SECONDS, DEFAULT_BASELINE_FPS, DEFAULT_LANG_CODE,
    DEFAULT_POSTGRES_PORT_CUSTOM, DEFAULT_POSTGRES_PORT_STANDARD, MAX_LANG_CODE_LEN,
    MIN_LANG_CODE_LEN, NTSC_FILM_FPS, PAL_FPS,
};
pub use dbphash::{
    rank_media_matches, DbphashBuilder, DbphashCutEntry, DbphashCutMatch, DbphashHeader,
    DbphashMediaEntry, DbphashMediaMatch, DbphashPackStats, DbphashSparseEntry, MmapDbphash,
    DBPHASH_MAGIC, DBPHASH_VERSION,
};
use dbsrt_core::{hamming_distance, FrameHashEntry};
pub use delta::{DeltaIndex, DeltaMediaEntry};
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
