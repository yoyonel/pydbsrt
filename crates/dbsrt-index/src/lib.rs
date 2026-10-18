//! In-memory and memory-mapped search index for perceptual hashes.

pub mod bktree;
pub mod db;
pub mod dbphash;
pub mod mmap;

pub use bktree::{BkItem, BkTree};
pub use db::{parse_fps_from_phash_stem, DbClient, DbMatch, DbMediaRecord, DbSubtitleCoupling};
pub use dbphash::{
    DbphashBuilder, DbphashCutEntry, DbphashCutMatch, DbphashHeader, DbphashMediaEntry,
    DbphashMediaMatch, DbphashPackStats, MmapDbphash, DBPHASH_MAGIC, DBPHASH_VERSION,
};
use dbsrt_core::{hamming_distance, FrameHashEntry};
pub use mmap::{LibraryMediaMatch, MmapLibrary, MmapPhashFile};

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
