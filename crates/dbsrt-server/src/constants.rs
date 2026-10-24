//! Operational constants, algorithmic thresholds, and unit factors for dbsrt-server.

pub const DISCOVERY_FILE: &str = "/tmp/dbsrt-server.json";

/// Default maximum Hamming distance (bits out of 64) used for visual pHash media identification.
pub const DEFAULT_RECOGNITION_SEARCH_DISTANCE: u32 = 6;

/// Minimum temporally coherent confidence percentage (`0.0 ..= 100.0`) required to accept a pHash media match.
pub const MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT: f64 =
    dbsrt_index::MIN_TEMPORAL_COHERENCE_CONFIDENCE * 100.0;

/// Default fallback frame rate when media container metadata cannot be probed.
pub const DEFAULT_FALLBACK_FPS: f64 = 23.976;

/// Default maximum number of log entries retained in the in-memory circular buffer.
pub const LOG_BUFFER_CAPACITY: usize = 300;

/// Default sample extraction duration in seconds for media fingerprinting.
pub const DEFAULT_EXTRACT_SAMPLE_SECONDS: f64 = 1.0;

/// Default network timeout in milliseconds for remote relay requests (bounded under VLC watchdog 5s threshold).
pub const DEFAULT_RELAY_TIMEOUT_MS: u64 = 3_500;

/// Default connect timeout in milliseconds for remote relay TCP socket.
pub const DEFAULT_RELAY_CONNECT_TIMEOUT_MS: u64 = 2_000;

/// Default network timeout in milliseconds for remote relay download requests.
pub const DEFAULT_DOWNLOAD_RELAY_TIMEOUT_MS: u64 = 3_500;

/// Subdirectory in user cache directory used as fallback when media storage is read-only.
pub const DEFAULT_CACHE_SUBTITLES_SUBDIR: &str = "dbsrt/subtitles";

/// Number of perceptual hash samples included in JSON search response preview.
pub const FINGERPRINT_PREVIEW_SAMPLE_COUNT: usize = 5;

/// Number of seconds in a standard 24-hour day.
pub const SECONDS_PER_DAY: u64 = 86_400;

/// Number of seconds in an hour.
pub const SECONDS_PER_HOUR: u64 = 3600;

/// Number of seconds in a minute.
pub const SECONDS_PER_MINUTE: u64 = 60;

/// Total bit depth of a 64-bit DCT perceptual hash.
pub const PHASH_BIT_DEPTH: f64 = 64.0;

/// Scale factor to convert ratios (0.0 ..= 1.0) into percentages (0.0 ..= 100.0).
pub const PERCENT_SCALE: f64 = 100.0;

/// Default baseline FPS when target FPS is unspecified or invalid.
pub const DEFAULT_BASELINE_FPS: f64 = 24.0;

/// Default Hamming threshold between consecutive frames to detect a scene cut.
pub const DEFAULT_CUT_THRESHOLD: u32 = 16;

/// Default Hamming search distance for BK-tree and sequence cut voting.
pub const DEFAULT_SEARCH_DISTANCE: u32 = 12;

/// Maximum number of frames (~5 seconds at 24fps) used for continuous cross-correlation sequence alignment.
pub const MAX_CORRELATION_SAMPLE_FRAMES: usize = 120;

/// Minimum number of scene cuts in both target and reference to trigger cut-consensus voting.
pub const MIN_CUTS_FOR_CONSENSUS: usize = 2;

/// Minimum confidence percentage (0.0 ..= 100.0) from cut consensus required before falling back to sliding window.
pub const MIN_CUT_CONSENSUS_CONFIDENCE: f64 = 70.0;

/// Minimum visual shift in milliseconds to consider an offset significant enough to offer a synced candidate.
pub const SIGNIFICANT_SHIFT_THRESHOLD_MS: f64 = 500.0;

/// Interval in milliseconds between file watcher mtime polls.
pub const DEFAULT_FILE_WATCH_INTERVAL_MS: u64 = 1_000;

/// Debounce duration in milliseconds before triggering reload after detecting file modification.
pub const DEFAULT_FILE_WATCH_DEBOUNCE_MS: u64 = 500;

/// Interval in seconds before retrying PostgreSQL event listener connection on failure.
pub const DEFAULT_PG_RECONNECT_INTERVAL_SECS: u64 = 10;

/// Interval in seconds before retrying PostgreSQL LISTEN subscription on error.
pub const DEFAULT_PG_ERROR_RETRY_SECS: u64 = 5;

/// Minimum significant shift in milliseconds to apply subtitle timeline modification.
pub const MIN_SIGNIFICANT_SHIFT_MS: f64 = 0.001;

/// Milliseconds per second conversion factor.
pub const MILLIS_PER_SEC: f64 = 1000.0;

/// Refinement window radius in step units (+/- 2 seconds).
pub const REFINEMENT_STEP_RADIUS: usize = 2;
