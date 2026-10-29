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

/// Bits per byte factor for network throughput conversion to Mbps.
pub const BITS_PER_BYTE_F64: f64 = 8.0;

/// Procfs path to network interface statistics.
pub const PROC_NET_DEV_PATH: &str = "/proc/net/dev";

/// Number of header lines in `/proc/net/dev`.
pub const PROC_NET_DEV_HEADER_LINES: usize = 2;

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

/// Default relative path to history seed NDJSON log used when live batch report is absent.
pub const DEFAULT_HISTORY_SEED_PATH: &str = "data/seed/history_seed.jsonl";

/// Default relative path to certified subtitle couplings JSON registry.
pub const DEFAULT_COUPLINGS_PATH: &str = "data/SRT/couplings.json";

/// Height in lines of the top multi-line telemetry header block.
pub const HEADER_BLOCK_HEIGHT: u16 = 6;

/// Height in lines of the bottom keybinding and status footer block.
pub const FOOTER_BLOCK_HEIGHT: u16 = 2;

/// Minimum height in lines of the body content block.
pub const MIN_BODY_HEIGHT: u16 = 5;

/// Number of characters for GPU video engine progress bar rendering.
pub const GPU_PROGRESS_BAR_CHAR_WIDTH: usize = 12;

/// Rolling time window in seconds (2 hours) for session throughput aggregation.
pub const SESSION_WINDOW_SECS: f64 = 7200.0;

/// Maximum worker threads allocated for Rayon global thread pool to prevent NPROC exhaustion on shared hosting.
pub const DEFAULT_MAX_RAYON_WORKER_THREADS: usize = 2;

/// Minimum session duration in seconds required before displaying session throughput.
pub const MIN_SESSION_DURATION_DISPLAY_SECS: f64 = 5.0;

/// Minimum decode duration in seconds to qualify as full pHash processing for statistics.
pub const MIN_FULL_DECODE_DURATION_SECS: f64 = 1.0;

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

/// Default filesystem directory path fallback for Synology NAS TVShow library.
pub const DEFAULT_NAS_TVSHOW_PATH: &str = "/media/storage/tvshow";

/// Environment variable overriding the NAS TVShow library directory path.
pub const NAS_TVSHOW_DIR_ENV: &str = "NAS_TVSHOW_DIR";

/// Default path to discovered media metadata JSON cache.
pub const DEFAULT_MEDIA_CACHE_PATH: &str = "data/reports/media_browser_cache.json";

/// Default path to media sources configuration JSON file.
pub const DEFAULT_MEDIA_SOURCES_PATH: &str = "data/reports/media_sources.json";

/// Default display name for the default Synology NAS TVShow source.
pub const DEFAULT_SOURCE_NAME_TVSHOW: &str = "Synology NAS (tvshow)";

/// Default identifier for the primary NAS source.
pub const DEFAULT_SOURCE_ID_TVSHOW: &str = "synology_tvshow";

/// Default height in lines for the contextual help tab selector bar.
pub const HELP_TAB_HEADER_HEIGHT: u16 = 3;

/// Default directory storing precomputed pHash binary fingerprints.
pub const DEFAULT_PHASH_DIR: &str = "data/__PHASH__";

/// Cumulative width of all fixed non-stem columns and inter-column spacers in the media browser table.
/// 3 (cursor) + 5 (sel) + 10 (size) + 9 (duration) + 18 (phash) + 15 (subs) + 10 (codec) + 14 (status) + 8 (spacers) = 92.
pub const MEDIA_BROWSER_NON_STEM_COLUMNS_WIDTH: u16 = 92;

/// Maximum number of discovered video files collected per media source (safety ceiling against filesystem loops).
pub const MAX_MEDIA_SCAN_LIMIT: usize = 50_000;

/// Number of items processed per batch chunk for progressive streaming updates to the TUI.
pub const SCAN_STREAMING_CHUNK_SIZE: usize = 200;

/// Timeout in seconds for container probing with ffprobe.
pub const FAST_PROBE_TIMEOUT_SECS: u64 = 4;

/// Interval in seconds between periodic background scans of the NAS library.
pub const DEFAULT_MEDIA_SCAN_INTERVAL_SECS: u64 = 30;

/// Default path to selected batch targets JSON file.
pub const DEFAULT_SELECTED_BATCH_TARGETS_PATH: &str = "data/reports/selected_batch_targets.json";

/// Default path to output report for interactive selected batches.
pub const DEFAULT_SELECTED_BATCH_REPORT_PATH: &str = "data/reports/selected_batch_report.md";

/// Maximum concurrent workers spawned for interactive selected batch execution.
pub const MAX_SELECTED_BATCH_WORKERS: usize = 4;

/// Maximum display character width for source names in the sources table.
pub const MAX_SOURCE_NAME_CHARS: usize = 28;

/// Maximum display character width for source paths in the sources table.
pub const MAX_SOURCE_PATH_CHARS: usize = 50;

/// Default modal width percentage for source configuration dialog.
pub const SOURCE_MODAL_WIDTH_PERCENT: u16 = 65;

/// Default modal height lines for source configuration dialog.
pub const SOURCE_MODAL_HEIGHT_LINES: u16 = 13;

/// Default modal height lines for source deletion confirmation dialog.
pub const DELETE_CONFIRM_MODAL_HEIGHT_LINES: u16 = 11;

/// Process command line pattern identifying batch coordinator processes.
pub const BATCH_COORDINATOR_PATTERN: &str = "batch_media_add";

/// Process command line pattern identifying dbsrt media add worker processes.
pub const BATCH_WORKER_CMD_PATTERN_1: &str = "dbsrt media:add";

/// Process command line pattern identifying alternate dbsrt worker processes.
pub const BATCH_WORKER_CMD_PATTERN_2: &str = "dbsrt media add";

/// Brief pause in milliseconds allowing coordinators to terminate child process groups gracefully.
pub const BATCH_GRACEFUL_SHUTDOWN_PAUSE_MS: u64 = 50;

/// Number of bytes in a single 32x32 8-bit grayscale raw video frame (1 KiB).
pub const RAW_FRAME_BYTES: f64 = 1024.0;

/// Default baseline duration in seconds for streaming TV show episodes (45 minutes).
pub const DEFAULT_STREAM_EPISODE_DURATION_SECS: f64 = 2700.0;

/// Directory path storing volatile active job progress tracking JSON files.
pub const ACTIVE_JOBS_RUN_DIR: &str = "run/jobs";
