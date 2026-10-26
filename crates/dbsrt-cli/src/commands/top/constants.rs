//! Operational, layout, and temporal constants for the top process monitor.

/// Default sampling and UI refresh interval in milliseconds.
pub const DEFAULT_REFRESH_INTERVAL_MS: u64 = 1000;

/// Non-blocking event poll timeout in milliseconds for ultra-responsive key handling.
pub const KEY_EVENT_POLL_TIMEOUT_MS: u64 = 20;

/// Number of bytes in a Kibibyte (KiB).
pub const BYTES_PER_KIB: u64 = 1024;

/// Number of bytes in a Mebibyte (MiB).
pub const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Number of bytes in a Gibibyte (GiB).
pub const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Floating-point number of bytes per MiB for rate conversions.
pub const BYTES_PER_MIB_F64: f64 = (1024 * 1024) as f64;

/// Floating-point number of bytes per GiB for memory conversions.
pub const BYTES_PER_GIB_F64: f64 = (1024 * 1024 * 1024) as f64;

/// Milliseconds in a second as float for duration calculations.
pub const MILLIS_PER_SEC: f64 = 1000.0;

/// Seconds per minute for duration formatting.
pub const SECONDS_PER_MINUTE: u64 = 60;

/// Seconds per hour for duration formatting.
pub const SECONDS_PER_HOUR: u64 = 3600;

/// Seconds per day for sanity checking elapsed durations.
pub const SECONDS_PER_DAY: u64 = 86400;

/// Maximum duration in seconds tolerated for elapsed process time (7 days).
pub const MAX_SANITY_DURATION_SECS: u64 = SECONDS_PER_DAY * 7;

/// Minimum progress ratio threshold to compute valid ETA predictions.
pub const MIN_PROGRESS_THRESHOLD: f64 = 0.001;

/// Maximum progress ratio cap.
pub const MAX_PROGRESS_CAP: f64 = 1.0;

/// Minimum elapsed time floor in seconds to avoid divide-by-zero during rate computations.
pub const MIN_ELAPSED_FLOOR_SECS: f64 = 0.1;

/// Minimum delta time floor in seconds for instantaneous throughput calculations.
pub const MIN_DELTA_TIME_FLOOR_SECS: f64 = 0.05;

/// Number of characters for text progress bar formatting.
pub const PROGRESS_BAR_CHAR_WIDTH: usize = 14;

/// Maximum string length of truncated media name in fullframe table.
pub const MAX_MEDIA_NAME_DISPLAY_CHARS: usize = 36;

/// High GPU load threshold percentage for red coloring.
pub const GPU_HIGH_LOAD_THRESHOLD_PCT: f64 = 80.0;

/// Moderate GPU load threshold percentage for green coloring.
pub const GPU_MODERATE_LOAD_THRESHOLD_PCT: f64 = 15.0;

/// Duration in seconds to display feedback banner messages.
pub const FEEDBACK_DISPLAY_DURATION_SECS: f64 = 3.5;

/// Maximum preview count of open file descriptors in inspection frame.
pub const MAX_OPEN_FILES_PREVIEW: usize = 12;

/// Maximum preview count of log tail lines in inspection frame.
pub const MAX_LOG_LINES_PREVIEW: usize = 30;

/// Default relative path to batch ingestion telemetry NDJSON log.
pub const DEFAULT_BATCH_EVENTS_PATH: &str = "data/reports/batch_tvshow_nightly.jsonl";

/// Default relative path to certified subtitle couplings JSON registry.
pub const DEFAULT_COUPLINGS_PATH: &str = "data/SRT/couplings.json";

/// Height in lines of the top multi-line telemetry header block.
pub const HEADER_BLOCK_HEIGHT: u16 = 5;

/// Height in lines of the bottom keybinding and status footer block.
pub const FOOTER_BLOCK_HEIGHT: u16 = 2;

/// Minimum height in lines of the body content block.
pub const MIN_BODY_HEIGHT: u16 = 5;

/// Number of characters for GPU video engine progress bar rendering.
pub const GPU_PROGRESS_BAR_CHAR_WIDTH: usize = 12;

/// Rolling time window in seconds (2 hours) for session throughput aggregation.
pub const SESSION_WINDOW_SECS: f64 = 7200.0;

/// Minimum session duration in seconds required before displaying session throughput.
pub const MIN_SESSION_DURATION_DISPLAY_SECS: f64 = 5.0;

/// Minimum decode duration in seconds to qualify as full pHash processing for statistics.
pub const MIN_FULL_DECODE_DURATION_SECS: f64 = 60.0;

/// Percentile factor for p90 statistical calculations.
pub const P90_PERCENTILE_FACTOR: f64 = 0.90;

/// Upper rate threshold in MB/s beyond which a completed job is treated as cache hit.
pub const MAX_SENSIBLE_RATE_MIB_S: f64 = 500.0;

/// Minimum wall clock duration in seconds for non-cached completed decoding tasks.
pub const MIN_WALL_DURATION_NON_CACHE_SECS: f64 = 1.0;

/// Maximum display character width for media stems in completed media fullframe table.
pub const MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS: usize = 42;

/// Number of days per 400-year cycle in the Gregorian calendar for ISO timestamp parsing.
pub const CIVIL_DAYS_PER_400_YEARS: i64 = 146097;

/// Epoch day offset between civil year 0 and Unix epoch 1970-01-01.
pub const CIVIL_EPOCH_DAYS_OFFSET: i64 = 719468;

/// Default baseline FPS fallback for duration calculations from couplings.
pub const DEFAULT_COUPLING_BASELINE_FPS: f64 = 25.0;

/// Number of items to skip during PageUp / PageDown navigation.
pub const PAGE_SCROLL_STEP: usize = 10;
