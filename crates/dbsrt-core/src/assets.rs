//! Domain models and integrity predicates for media sources and assets.
//!
//! Provides the primary input data model decoupling physical media containers
//! from downstream perceptual hash (pHash) fingerprints and subtitle couplings.

use serde::{Deserialize, Serialize};

/// Maximum percentage margin between real and expected frames for Constant Frame Rate (2%).
pub const DEFAULT_TOLERANCE_RATIO_CFR: f64 = 0.02;

/// Maximum percentage margin between real and expected frames for Variable Frame Rate (5%).
pub const DEFAULT_TOLERANCE_RATIO_VFR: f64 = 0.05;

/// Floor threshold on minimum acceptable frame difference before triggering truncation.
pub const MIN_COMPLETENESS_MARGIN_FRAMES: usize = 12;

/// Physical or network storage protocol backing a [`MediaSource`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    /// Direct local filesystem path.
    #[default]
    Local,
    /// CIFS/SMB remote network share mount.
    CifsMount,
    /// Streaming network URL (HTTP, HLS, RTMP).
    RemoteStream,
}

/// A configured storage root or network share where candidate video containers are discovered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSource {
    /// Persistent machine identifier (e.g. `"synology_tvshow"`).
    pub id: String,
    /// Human-friendly display label.
    pub name: String,
    /// Filesystem directory root path or network URI.
    pub root_uri: String,
    /// Underlying storage protocol.
    pub storage_kind: StorageKind,
    /// Whether this source is actively scanned and queried.
    pub enabled: bool,
}

/// Parameterized categories for synthetic benchmark test queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyntheticQueryKind {
    /// Temporal timestamp offset shift applied to cues or query window.
    TimeShift { offset_ms: i64 },
    /// Additive pixel or DCT noise injection.
    Noise { snr_db: u32 },
    /// Lossy transcoded re-encoding with target codec and CRF factor.
    Recode { codec: String, crf: u8 },
}

/// Nature and lineage of the media container within the asset library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AssetKind {
    /// Full-length, untruncated master release (movie, TV episode, documentary).
    #[default]
    Master,
    /// Temporal video excerpt cut from an identified master release.
    DerivedCut {
        /// Foreign key identifying the parent master release.
        parent_asset_id: u32,
        /// Presentation start offset in milliseconds from parent timeline origin.
        start_ms: u64,
        /// Total cut duration in milliseconds.
        duration_ms: u64,
    },
    /// Synthetic benchmark query generated to evaluate trajectory tracking algorithms.
    SyntheticQuery {
        /// Foreign key identifying the ground-truth reference asset.
        parent_asset_id: u32,
        /// Specific alteration kind applied to the synthetic sample.
        kind: SyntheticQueryKind,
    },
}

/// Immutable container fingerprint computed in $O(1)$ time over boundary chunks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContainerIdentity {
    /// 16-character hexadecimal OpenSubtitles 64-bit uint checksum.
    pub moviehash: String,
    /// Exact physical file size in bytes.
    pub file_size_bytes: u64,
}

/// Technical geometry and stream specifications extracted without decoding frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeSpecs {
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Nominal framerate in frames per second (decimal display).
    pub fps: f64,
    /// Rational framerate preserving exact NTSC ratios (e.g. `(24000, 1001)`).
    pub fps_rational: (u32, u32),
    /// Whether the video container exhibits variable frame rate timing.
    pub is_vfr: bool,
    /// Theoretical frame count computed by strict rounding.
    pub expected_frames: usize,
    /// Horizontal pixel resolution.
    pub width: u32,
    /// Vertical pixel resolution.
    pub height: u32,
    /// Video codec identifier (`"hevc"`, `"h264"`, `"av1"`).
    pub video_codec: String,
}

/// Lifecycle state of a perceptual hash fingerprint stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PHashLifecycleStatus {
    /// Media discovered and probed; pHash extraction not yet scheduled.
    #[default]
    Pending,
    /// Frame decoding and perceptual hashing currently running in background.
    InProgress,
    /// Successfully extracted and verified against the completeness gate.
    Complete,
    /// Incomplete or interrupted extraction with fewer frames than expected.
    Truncated {
        frames_found: usize,
        expected: usize,
    },
    /// Unreadable or damaged file whose size is not a multiple of 8 bytes.
    Corrupted,
    /// Outdated fingerprint due to source file replacement on disk.
    Obsolete,
}

impl PHashLifecycleStatus {
    /// Returns a concise human-readable status badge label.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// Static string slice representing the state moniker.
    ///
    /// # Errors
    /// Never returns `Err`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Maps each variant deterministically to its uppercase operational tag.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::InProgress => "IN_PROGRESS",
            Self::Complete => "COMPLETE",
            Self::Truncated { .. } => "TRUNCATED",
            Self::Corrupted => "CORRUPTED",
            Self::Obsolete => "OBSOLETE",
        }
    }
}

/// Unified media container asset representing certified input truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    /// Unique identifier matching database primary key and `.dbphash` index offset.
    pub id: u32,
    /// Foreign key referencing the parent [`MediaSource`].
    pub source_id: String,
    /// Lineage classification (Master, DerivedCut, SyntheticQuery).
    pub kind: AssetKind,
    /// Immutable physical container identity.
    pub container: ContainerIdentity,
    /// Source file modification timestamp (UNIX seconds) for cache invalidation.
    pub source_mtime_unix: u64,
    /// Technical geometry and stream probe specifications.
    pub specs: ProbeSpecs,
    /// Canonical source filename including container extension.
    pub filename: String,
    /// Relative storage path from the [`MediaSource`] root directory.
    pub path_rel: String,
    /// Current lifecycle state of the associated pHash fingerprint.
    pub phash_status: PHashLifecycleStatus,
    /// Optional soft-delete timestamp in UNIX seconds.
    pub deleted_at: Option<u64>,
}

/// Validates whether an extracted pHash frame count satisfies the completeness gate.
///
/// # Arguments
/// * `frame_count` - Actual number of 64-bit pHash frames extracted from decoding.
/// * `expected_frames` - Theoretical frame count determined by container probe metadata.
/// * `is_vfr` - Whether the media uses variable frame rate timing.
///
/// # Returns
/// `true` if the frame count matches expected duration within allowed tolerance; `false` otherwise.
///
/// # Errors
/// Never returns `Err`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. If `expected_frames == 0`, gate is bypassed returning `true` (unprobed asset).
/// 2. Selects tolerance factor (2% for CFR, 5% for VFR).
/// 3. Computes absolute difference `|frame_count - expected_frames|`.
/// 4. Compares difference against `max(12, round(expected_frames * tolerance))`.
pub fn check_completeness(frame_count: usize, expected_frames: usize, is_vfr: bool) -> bool {
    if expected_frames == 0 {
        return true;
    }
    let tolerance_ratio = if is_vfr {
        DEFAULT_TOLERANCE_RATIO_VFR
    } else {
        DEFAULT_TOLERANCE_RATIO_CFR
    };
    let delta = frame_count.abs_diff(expected_frames);
    let allowed_margin = (expected_frames as f64 * tolerance_ratio).round() as usize;
    delta <= allowed_margin.max(MIN_COMPLETENESS_MARGIN_FRAMES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_completeness_cfr_nominal() {
        // 2 hours at 24 fps = 172,800 frames. 2% tolerance = 3,456 frames.
        let expected = 172_800;
        assert!(check_completeness(172_800, expected, false));
        assert!(check_completeness(172_000, expected, false)); // -800 frames (-0.46%) -> Valid
        assert!(check_completeness(174_000, expected, false)); // +1200 frames (+0.69%) -> Valid
        assert!(!check_completeness(165_000, expected, false)); // -7800 frames (-4.5%) -> Truncated
        assert!(!check_completeness(50, expected, false)); // 50 frames crash -> Truncated
    }

    #[test]
    fn test_check_completeness_vfr_tolerance() {
        // 10 minutes at 30 fps = 18,000 frames. 5% tolerance = 900 frames.
        let expected = 18_000;
        assert!(check_completeness(17_200, expected, true)); // -800 frames (-4.4%) -> Valid for VFR
        assert!(!check_completeness(17_000, expected, false)); // -1000 frames (-5.5%) -> Invalid CFR
        assert!(!check_completeness(16_500, expected, true)); // -1500 frames (-8.3%) -> Invalid VFR
    }

    #[test]
    fn test_check_completeness_short_media_floor_margin() {
        // 10 seconds at 24 fps = 240 frames. 2% of 240 is 5 frames, so floor 12 applies.
        let expected = 240;
        assert!(check_completeness(230, expected, false)); // delta 10 <= 12 -> Valid
        assert!(check_completeness(250, expected, false)); // delta 10 <= 12 -> Valid
        assert!(!check_completeness(220, expected, false)); // delta 20 > 12 -> Truncated
    }

    #[test]
    fn test_check_completeness_zero_expected_bypasses() {
        assert!(check_completeness(100, 0, false));
        assert!(check_completeness(0, 0, false));
    }

    #[test]
    fn test_asset_lineage_serialization_roundtrip() {
        let master = MediaAsset {
            id: 1,
            source_id: "synology_tvshow".to_string(),
            kind: AssetKind::Master,
            container: ContainerIdentity {
                moviehash: "a1b2c3d4e5f60718".to_string(),
                file_size_bytes: 1_000_000_000,
            },
            source_mtime_unix: 1794154800,
            specs: ProbeSpecs {
                duration_ms: 3_600_000,
                fps: 24.0,
                fps_rational: (24, 1),
                is_vfr: false,
                expected_frames: 86_400,
                width: 1920,
                height: 1080,
                video_codec: "hevc".to_string(),
            },
            filename: "Example.S01E01.mkv".to_string(),
            path_rel: "Example/S01/Example.S01E01.mkv".to_string(),
            phash_status: PHashLifecycleStatus::Complete,
            deleted_at: None,
        };

        let json = serde_json::to_string(&master).unwrap();
        let decoded: MediaAsset = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.kind, AssetKind::Master);
        assert_eq!(decoded.container.moviehash, "a1b2c3d4e5f60718");
        assert_eq!(decoded.specs.expected_frames, 86_400);

        let cut = MediaAsset {
            id: 2,
            source_id: "local_snippets".to_string(),
            kind: AssetKind::DerivedCut {
                parent_asset_id: 1,
                start_ms: 10_000,
                duration_ms: 15_000,
            },
            container: ContainerIdentity {
                moviehash: "e5f60718a1b2c3d4".to_string(),
                file_size_bytes: 2_000_000,
            },
            source_mtime_unix: 1794154900,
            specs: ProbeSpecs {
                duration_ms: 15_000,
                fps: 24.0,
                fps_rational: (24, 1),
                is_vfr: false,
                expected_frames: 360,
                width: 1920,
                height: 1080,
                video_codec: "h264".to_string(),
            },
            filename: "Example.S01E01_cut.mp4".to_string(),
            path_rel: "cuts/Example.S01E01_cut.mp4".to_string(),
            phash_status: PHashLifecycleStatus::Complete,
            deleted_at: None,
        };

        let json_cut = serde_json::to_string(&cut).unwrap();
        let decoded_cut: MediaAsset = serde_json::from_str(&json_cut).unwrap();
        assert!(matches!(
            decoded_cut.kind,
            AssetKind::DerivedCut {
                parent_asset_id: 1,
                start_ms: 10_000,
                duration_ms: 15_000
            }
        ));
    }
}
