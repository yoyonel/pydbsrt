//! Memory-mapped acoustic fingerprint archive (`.dbahash` v1) and unified [`AhashEngine`].
//!
//! Eliminates the `Vec<u64>` dense heap footprint (`~727 MiB` at `537` medias, `~4.4 GiB` at `3,089` medias)
//! by packing all `.ahash` dense sequences, sidecar metadata, and pre-sorted `MIH-32` coarse tables
//! into a single 64-byte aligned binary archive projected via `memmap2::Mmap`.

pub mod builder;
pub mod engine;
pub mod format;
pub mod reader;

pub use builder::{pack_ahash_directories, DbahashBuilder, DbahashPackOptions, DbahashTrackInput};
pub use engine::AhashEngine;
pub use format::{
    align_up_64, compute_dbahash_checksum, DbahashHeader, DbahashMediaEntry, DbahashPackStats,
    DBAHASH_FLAG_HAS_COARSE_MIH32, DBAHASH_HEADER_BYTES, DBAHASH_LANG_CAPACITY, DBAHASH_MAGIC,
    DBAHASH_MEDIA_ENTRY_BYTES, DBAHASH_SECTION_ALIGN, DBAHASH_VERSION, DEFAULT_DBAHASH_FACTOR_K,
    MEDIA_FLAG_HAS_AUDIO_INDEX, MEDIA_FLAG_HAS_LANGUAGE, MEDIA_FLAG_HAS_MOVIEHASH,
};
pub use reader::MmapDbahash;
