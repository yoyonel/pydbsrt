//! Operational constants, conversion factors, and default configuration parameters for `dbsrt-cli`.

/// Milliseconds per second conversion factor.
pub const MILLIS_PER_SEC: f64 = 1000.0;

/// Microseconds per second conversion factor.
pub const MICROS_PER_SEC: f64 = 1_000_000.0;

/// Seconds per minute conversion factor.
pub const SECONDS_PER_MINUTE: f64 = 60.0;

/// Seconds per hour conversion factor.
pub const SECONDS_PER_HOUR: f64 = 3600.0;

/// Percentage scaling factor (100.0%).
pub const PERCENT_FACTOR: f64 = 100.0;

/// Bytes per Megabyte (1024 * 1024).
pub const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// Small epsilon value for floating-point framerate comparisons.
pub const EPSILON_FPS: f64 = 1e-3;

/// Division by zero safety floor in seconds.
pub const DIVISION_BY_ZERO_FALLBACK_SEC: f64 = 0.0001;

/// Default iteration count for pHash throughput benchmark.
pub const DEFAULT_BENCH_PHASH_COUNT: usize = 100_000;

/// Dummy frame luminance byte value for benchmark synthetic frames.
pub const DUMMY_FRAME_BYTE: u8 = 128;

/// Dummy frame side length in pixels (square grayscale frame).
pub const DUMMY_FRAME_DIM: usize = 32;

/// Dummy frame buffer size in bytes (DUMMY_FRAME_DIM x DUMMY_FRAME_DIM grayscale).
pub const DUMMY_FRAME_SIZE: usize = DUMMY_FRAME_DIM * DUMMY_FRAME_DIM;

/// Default duration in seconds to extract for video decoding benchmark.
pub const DEFAULT_BENCH_VIDEO_DURATION_SEC: f64 = 10.0;

/// Default scene cut Hamming distance threshold.
pub const DEFAULT_CUT_THRESHOLD: u32 = 16;

/// Default Hamming search distance for video recognition.
pub const DEFAULT_SEARCH_DISTANCE: u32 = 1;

/// Search distance of 0 (exact match).
pub const DEFAULT_EXACT_SEARCH_DISTANCE: u32 = 0;

/// Default number of seconds extracted around media middle for recognition.
pub const DEFAULT_EXTRACTION_DURATION_SEC: f64 = 1.0;

/// Default limit of matched media items to display.
pub const DEFAULT_RECOGNIZE_LIMIT: usize = 1;

/// Default limit of top matches to display in library scan.
pub const DEFAULT_SCAN_LIMIT: usize = 5;

/// Default frame gap between window A and window B for two-window consensus scan.
pub const DEFAULT_WINDOW_FRAME_GAP: usize = 0;

/// Standard baseline framerate for cinema / media files (24.0 fps).
pub const DEFAULT_BASELINE_FPS: f64 = 24.0;

/// Standard PAL framerate (25.0 fps).
pub const DEFAULT_PAL_FPS: f64 = 25.0;

/// Standard NTSC film framerate (23.976 fps).
pub const DEFAULT_NTSC_FILM_FPS: f64 = 23.976;

/// Default sparse sampling framerate (1.0 fps).
pub const DEFAULT_SPARSE_FPS: f64 = 1.0;

/// Minimum sane video framerate in FPS.
pub const MIN_VALID_FPS: f64 = 0.0;

/// Maximum sane video framerate in FPS.
pub const MAX_VALID_FPS: f64 = 240.0;

/// Interval of media items between packing progress logs.
pub const PACK_PROGRESS_INTERVAL: usize = 25;

/// Default PostgreSQL port for pydbsrt Docker / native service.
pub const DEFAULT_POSTGRES_PORT: u16 = 5433;

/// Standard PostgreSQL port (used by CI service containers).
pub const STANDARD_POSTGRES_PORT: u16 = 5432;

/// Port with no listener, used by tests to force a connection failure.
pub const TEST_UNREACHABLE_PORT: u16 = 59999;

/// Default directory storing certified .srt files and couplings registry.
pub const DEFAULT_SUBTITLES_DIR: &str = "data/SRT";

/// Default directory storing .phash binary fingerprints.
pub const DEFAULT_PHASH_DATA_DIR: &str = "data/__PHASH__";

/// File extension of binary pHash fingerprint files (without leading dot).
pub const PHASH_EXT: &str = "phash";

/// Default system temporary directory path fallback.
pub const DEFAULT_TMP_DIR: &str = "/tmp";

/// Default PostgreSQL host fallback.
pub const DEFAULT_POSTGRES_HOST: &str = "127.0.0.1";

/// Default PostgreSQL username.
pub const DEFAULT_POSTGRES_USER: &str = "docker";

/// Default PostgreSQL password.
pub const DEFAULT_POSTGRES_PASS: &str = "docker";

/// Default PostgreSQL database name.
pub const DEFAULT_POSTGRES_DB: &str = "bktreedb";

/// Default subtitle provider identifier.
pub const DEFAULT_SUBTITLE_PROVIDER: &str = "opensubtitles";

/// Default subtitle language code fallback.
pub const DEFAULT_SUBTITLE_LANG: &str = "en";

/// Auto-detection subtitle language token.
pub const AUTO_SUBTITLE_LANG: &str = "auto";

/// Maximum number of subtitle candidates to display in ranked search output.
pub const FETCH_MAX_CANDIDATES_DISPLAY: usize = 10;

/// Maximum number of candidate download attempts before giving up.
pub const FETCH_MAX_DOWNLOAD_ATTEMPTS: usize = 3;

/// Dot-prefixed .phash binary fingerprint extension.
pub const PHASH_DOT_EXT: &str = ".phash";

/// Divisor used to seek to the temporal midpoint of a media stream.
pub const MIDPOINT_DURATION_DIVISOR: f64 = 2.0;

/// Default HTTP URL of dbsrt-server for live catalog telemetry and ingest.
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8765";

/// Default HTTP request timeout in seconds for server catalog ingest.
pub const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 2;

/// Known video extensions used for fingerprint stem conflict filtering.
pub const KNOWN_VIDEO_EXTS: &[&str] = &[".mp4.", ".webm.", ".mkv.", ".avi.", ".mov."];
