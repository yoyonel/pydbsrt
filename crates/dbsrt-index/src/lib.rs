//! In-memory and memory-mapped search index for perceptual hashes.

pub mod bktree;
pub mod db;

pub use bktree::{BkItem, BkTree};
pub use db::{DbClient, DbMatch};
use dbsrt_core::{hamming_distance, FrameHashEntry};

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
