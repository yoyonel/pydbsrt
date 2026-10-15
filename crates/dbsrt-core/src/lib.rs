//! Core primitives for pydbsrt / dbsrt-rs:
//! - Exact DCT-II perceptual hash computation (`phash_32x32`)
//! - Hardware-accelerated Hamming distance (`hamming_distance`)
//! - Binary file format definitions

pub mod cuts;
pub mod phash;
pub mod srt;
pub mod video;

pub use cuts::{detect_cuts, read_phash_file, write_phash_file, CutItem};
pub use phash::{hamming_distance, phash_32x32};
pub use srt::{SubtitleCue, SubtitleFile};
pub use video::{extract_frames_phash, probe_video, VideoMetadata};

/// A timestamped frame hash entry.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameHashEntry {
    /// Zero-based frame offset in the media stream
    pub frame_offset: u32,
    /// Identifier of the media (0 if standalone)
    pub media_id: u32,
    /// 64-bit perceptual hash
    pub phash: u64,
}
