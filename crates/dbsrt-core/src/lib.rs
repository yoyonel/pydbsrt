//! Core primitives for pydbsrt / dbsrt-rs:
//! - Exact DCT-II perceptual hash computation (`phash_32x32`)
//! - Hardware-accelerated Hamming distance (`hamming_distance`)
//! - Binary file format definitions

pub mod container;
pub mod coupling;
pub mod cuts;
pub mod metadata;
pub mod phash;
pub mod providers;
pub mod srt;
pub mod video;

pub use container::{
    extract_embedded_subtitle, inspect_container, language_matches, ContainerInspection,
    SubtitleStreamInfo,
};
pub use coupling::{
    build_and_validate_sub_frames, compute_bytes_digest_32, compute_media_phash_digest,
    CertifiedMediaCoupling, CertifiedSubtitleRecord, CouplingRegistry, SubFrameAnchor,
};
pub use cuts::{
    detect_cuts, detect_cuts_timestamped, is_blank_phash, read_phash_file, write_phash_file,
    CutItem, TimestampedCutItem,
};
pub use metadata::{MediaMetadata, MediaType};
pub use phash::{hamming_distance, phash_32x32};
pub use providers::{
    cache::SubtitleCache, mock::MockSubtitleProvider, opensubtitles::OpenSubtitlesProvider,
    score_candidate, RemoteSubtitleCandidate, SubtitleProvider,
};
pub use srt::{SubtitleCue, SubtitleFile};
pub use video::{
    extract_frames_advanced, extract_frames_phash, extract_keyframes_phash, extract_sparse_phash,
    probe_keyframe_timestamps, probe_video, FrameSample, HwAccel, ResolvedFfmpeg, SamplingMode,
    VideoExtractionOptions, VideoMetadata,
};

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
