//! Core primitives for pydbsrt / dbsrt-rs:
//! - Exact DCT-II perceptual hash computation (`phash_32x32`)
//! - Hardware-accelerated Hamming distance (`hamming_distance`)
//! - Binary file format definitions

pub mod container;
pub mod coupling;
pub mod cuts;
pub mod ffmpeg_input;
pub mod job_progress;
pub mod lang_status;
pub mod media_pairing;
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
    build_and_validate_sub_frames, build_and_validate_sub_frames_opts, compute_bytes_digest_32,
    compute_media_phash_digest, CertifiedMediaCoupling, CertifiedSubtitleRecord, CouplingRegistry,
    RegistryLock, RelabelResult, SubFrameAnchor,
};
pub use cuts::{
    detect_cuts, detect_cuts_timestamped, is_blank_phash, is_blank_sequence, is_static_sequence,
    read_phash_file, write_phash_file, CutItem, TimestampedCutItem, FLAT_BLACK_DITHERED_1,
    FLAT_BLACK_DITHERED_2, FLAT_BLACK_DITHERED_3, FLAT_BLACK_PHASH, FLAT_WHITE_PHASH,
    MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT, MIN_DYNAMIC_SEQUENCE_WINDOW_LEN,
    STATIC_FRAME_MAX_DISTANCE, STATIC_SEQUENCE_MAX_JITTER_DISTANCE,
};
pub use ffmpeg_input::{
    apply_ftp_credentials, display_input, ffmpeg_decode_threads, redact_command_line,
    redact_url_userinfo, DEFAULT_FFMPEG_DECODE_THREADS, FFMPEG_THREADS_ENV, FTP_PASSWORD_ENV,
    FTP_USER_ENV,
};
pub use lang_status::LangStatus;
pub use media_pairing::{
    find_subtitle_pairs, parse_extended_language_token, parse_language_code, SUBTITLE_EXTENSION,
    SUBTITLE_SUBDIRS, SUPPORTED_VIDEO_EXTENSIONS,
};
pub use metadata::{MediaMetadata, MediaType};
pub use phash::{hamming_distance, phash_32x32};
pub use providers::{
    cache::SubtitleCache, mock::MockSubtitleProvider, opensubtitles::OpenSubtitlesProvider,
    score_candidate, RemoteSubtitleCandidate, SubtitleProvider,
};
pub use srt::{SubtitleCue, SubtitleFile};
pub use video::{
    detect_media_fps, extract_frames_advanced, extract_frames_phash,
    extract_frames_phash_with_hwaccel, extract_keyframes_phash, extract_sparse_phash,
    parse_fps_from_stem, probe_keyframe_timestamps, probe_video, probe_video_with_cmd,
    probe_video_with_timeout, FrameSample, HwAccel, ResolvedFfmpeg, SamplingMode,
    VideoExtractionOptions, VideoMetadata, DEFAULT_BASELINE_FPS, FFPROBE_TIMEOUT_SECS,
    NTSC_FILM_FPS, PAL_FPS,
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
