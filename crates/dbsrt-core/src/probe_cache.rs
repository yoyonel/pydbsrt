//! Persistent and in-memory cache for media container probes and 64-bit moviehashes.
//!
//! Provides fast, allocation-minimized caching of video durations, framerates,
//! and metadata indexed by 64-bit OpenSubtitles-compatible MovieHash and file size.
//! Eliminates redundant `ffprobe` executions across batch jobs and WAN streaming.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Number of bytes read from head and tail for OpenSubtitles 64-bit MovieHash calculation (64 KiB).
pub const CHUNK_SIZE_BYTES: usize = 65_536;

/// Minimum file size required to compute an OpenSubtitles 64-bit MovieHash without overlap.
pub const MIN_FILE_SIZE_BYTES: u64 = CHUNK_SIZE_BYTES as u64;

/// Length in hexadecimal characters of a 64-bit MovieHash string.
pub const MOVIEHASH_HEX_LENGTH: usize = 16;

/// Default filename for the on-disk probe cache database.
pub const DEFAULT_PROBE_CACHE_FILE: &str = "probes_cache.json";

/// Current schema version of the serialized probe cache file.
pub const PROBE_CACHE_SCHEMA_VERSION: u32 = 1;

/// Default cut duration in seconds for benchmark video slices.
pub const DEFAULT_CUT_DURATION_SECONDS: f64 = 15.0;

/// Number of bytes in an 8-byte little-endian word for moviehash accumulation.
const WORD_SIZE_BYTES: usize = 8;

/// Minimum safe duration in seconds to consider a probed media valid.
const MIN_VALID_DURATION_SECONDS: f64 = 0.001;

/// Minimum safe framerate in frames per second to consider a probed media valid.
const MIN_VALID_FPS: f64 = 1.0;

/// Default fallback framerate when parsing fails or probe metadata is absent.
const DEFAULT_FALLBACK_FPS: f64 = 25.0;

/// Computes a standard 64-bit OpenSubtitles MovieHash from file size and head/tail 64 KiB buffers.
///
/// # Arguments
/// * `file_size` - Total file size in bytes as an unsigned 64-bit integer.
/// * `head` - Byte slice containing the first 64 KiB of the file.
/// * `tail` - Byte slice containing the last 64 KiB of the file.
///
/// # Returns
/// * `Result<String>` - Lowercase 16-character hexadecimal string representing the 64-bit hash.
///
/// # Errors
/// * Returns `Err` if `head` or `tail` length is smaller than [`CHUNK_SIZE_BYTES`].
/// * Returns `Err` if `file_size` is smaller than [`MIN_FILE_SIZE_BYTES`].
///
/// # Panics
/// * Guaranteed never to panic.
///
/// # Data Flow & Invariants
/// * Accumulates little-endian 64-bit words via wrapping addition onto the initial `file_size`.
/// * Preserves bit-for-bit exact parity with the OpenSubtitles specification.
pub fn compute_moviehash_from_head_tail(
    file_size: u64,
    head: &[u8],
    tail: &[u8],
) -> Result<String> {
    if file_size < MIN_FILE_SIZE_BYTES {
        return Err(anyhow!(
            "File size ({} bytes) is below minimum {} bytes for moviehash",
            file_size,
            MIN_FILE_SIZE_BYTES
        ));
    }
    if head.len() < CHUNK_SIZE_BYTES || tail.len() < CHUNK_SIZE_BYTES {
        return Err(anyhow!(
            "Head/tail slice length must be at least {} bytes (got head: {}, tail: {})",
            CHUNK_SIZE_BYTES,
            head.len(),
            tail.len()
        ));
    }

    let mut hash: u64 = file_size;

    // Accumulate head chunk (first 64 KiB)
    for word_chunk in head[..CHUNK_SIZE_BYTES].as_chunks::<WORD_SIZE_BYTES>().0 {
        let word = u64::from_le_bytes(*word_chunk);
        hash = hash.wrapping_add(word);
    }

    // Accumulate tail chunk (last 64 KiB)
    for word_chunk in tail[..CHUNK_SIZE_BYTES].as_chunks::<WORD_SIZE_BYTES>().0 {
        let word = u64::from_le_bytes(*word_chunk);
        hash = hash.wrapping_add(word);
    }

    Ok(format!("{:016x}", hash))
}

/// Computes standard 64-bit OpenSubtitles MovieHash directly from an open local filesystem file.
///
/// # Arguments
/// * `path` - Reference to the target filesystem path.
///
/// # Returns
/// * `Result<(String, u64)>` - Tuple containing the 16-hex moviehash and total file size in bytes.
///
/// # Errors
/// * Returns `Err` if file cannot be opened or read.
/// * Returns `Err` if file size is smaller than [`MIN_FILE_SIZE_BYTES`].
///
/// # Panics
/// * Guaranteed never to panic.
///
/// # Data Flow & Invariants
/// * Reads two contiguous 64 KiB chunks via non-allocating seeks and passes them to
///   [`compute_moviehash_from_head_tail`].
pub fn compute_local_file_moviehash<P: AsRef<Path>>(path: P) -> Result<(String, u64)> {
    let mut file = File::open(path.as_ref())
        .with_context(|| format!("Failed to open file for moviehash: {:?}", path.as_ref()))?;
    let file_size = file.metadata()?.len();
    if file_size < MIN_FILE_SIZE_BYTES {
        return Err(anyhow!(
            "File size ({} bytes) is below minimum {} bytes for moviehash: {:?}",
            file_size,
            MIN_FILE_SIZE_BYTES,
            path.as_ref()
        ));
    }

    let mut head = vec![0u8; CHUNK_SIZE_BYTES];
    let mut tail = vec![0u8; CHUNK_SIZE_BYTES];

    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut head)?;

    let tail_offset = file_size.saturating_sub(CHUNK_SIZE_BYTES as u64);
    file.seek(SeekFrom::Start(tail_offset))?;
    file.read_exact(&mut tail)?;

    let hash = compute_moviehash_from_head_tail(file_size, &head, &tail)?;
    Ok((hash, file_size))
}

/// Asynchronously fetches the first and last 64 KiB chunks over HTTP Range requests and computes moviehash.
///
/// # Arguments
/// * `client` - Shared [`reqwest::Client`] configured with timeout and TLS pools.
/// * `url` - Full HTTP/HTTPS target stream URL.
///
/// # Returns
/// * `Result<(String, u64)>` - Tuple containing the 16-hex moviehash and total remote file size in bytes.
///
/// # Errors
/// * Returns `Err` if HEAD or Range requests fail, return non-200/206 status codes, or file size is invalid.
///
/// # Panics
/// * Guaranteed never to panic.
///
/// # Data Flow & Invariants
/// * Executes a HEAD request to extract `Content-Length`.
/// * Issues two concurrent/sequential HTTP Range GET requests reading exactly 64 KiB each.
/// * Computes moviehash in RAM without downloading the entire media.
pub async fn fetch_http_range_moviehash(
    client: &reqwest::Client,
    url: &str,
) -> Result<(String, u64)> {
    let head_resp = client
        .head(url)
        .send()
        .await
        .with_context(|| format!("HEAD request failed for {}", url))?;
    if !head_resp.status().is_success() {
        return Err(anyhow!(
            "HEAD request returned status {} for {}",
            head_resp.status(),
            url
        ));
    }

    let file_size = head_resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Missing or invalid Content-Length header for {}", url))?;

    if file_size < MIN_FILE_SIZE_BYTES {
        return Err(anyhow!(
            "Remote file size ({} bytes) is too small for moviehash",
            file_size
        ));
    }

    // Range 1: bytes=0-65535
    let head_range = format!("bytes=0-{}", CHUNK_SIZE_BYTES - 1);
    let head_bytes = client
        .get(url)
        .header(reqwest::header::RANGE, head_range)
        .send()
        .await?
        .bytes()
        .await?;

    // Range 2: bytes={tail_offset}-{file_size-1}
    let tail_offset = file_size.saturating_sub(CHUNK_SIZE_BYTES as u64);
    let tail_range = format!("bytes={}-{}", tail_offset, file_size - 1);
    let tail_bytes = client
        .get(url)
        .header(reqwest::header::RANGE, tail_range)
        .send()
        .await?
        .bytes()
        .await?;

    let hash = compute_moviehash_from_head_tail(file_size, &head_bytes, &tail_bytes)?;
    Ok((hash, file_size))
}

/// Represents cached probe metadata for a single video file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeEntry {
    /// 16-character hexadecimal OpenSubtitles MovieHash.
    pub moviehash: String,
    /// Total binary size of the media in bytes.
    pub file_size: u64,
    /// Exact video duration in seconds.
    pub duration_seconds: f64,
    /// Video framerate in frames per second.
    pub fps: f64,
    /// Optional detected video codec identifier (e.g. "hevc", "h264", "av1").
    #[serde(default)]
    pub video_codec: Option<String>,
    /// Optional horizontal pixel resolution.
    #[serde(default)]
    pub width: Option<u32>,
    /// Optional vertical pixel resolution.
    #[serde(default)]
    pub height: Option<u32>,
    /// Epoch timestamp in seconds when the probe was recorded.
    pub probed_at_epoch_s: u64,
}

impl ProbeEntry {
    /// Calculates the starting offset timestamp for a relative cut ratio clamped to video duration.
    ///
    /// # Arguments
    /// * `ratio` - Fraction of total video duration (e.g. 0.20 for 20%, 0.50 for mid-point).
    ///
    /// # Returns
    /// * `f64` - Clamped starting point in seconds `[0.0, duration_seconds]`.
    ///
    /// # Errors
    /// * None.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Multiplies ratio with `duration_seconds` and clamps between 0.0 and `duration_seconds`.
    pub fn calculate_cut_offset(&self, ratio: f64) -> f64 {
        let raw = ratio * self.duration_seconds;
        if raw < 0.0 {
            0.0
        } else if raw > self.duration_seconds {
            self.duration_seconds
        } else {
            raw
        }
    }

    /// Formats the composite cache lookup key for this entry (`"<moviehash>:<file_size>"`).
    ///
    /// # Arguments
    /// * None.
    ///
    /// # Returns
    /// * `String` - Formatted lookup key.
    ///
    /// # Errors
    /// * None.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Joins lowercase moviehash and file size with a colon separator.
    pub fn cache_key(&self) -> String {
        format!("{}:{}", self.moviehash, self.file_size)
    }
}

/// Top-level serialized schema container for the on-disk probe cache file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedProbeCache {
    /// Schema format version number.
    pub version: u32,
    /// Key-value map of composite keys to probe entries.
    pub entries: HashMap<String, ProbeEntry>,
}

/// Thread-safe, disk-backed cache for media container probes and moviehashes.
#[derive(Debug)]
pub struct ProbeCache {
    /// Destination filesystem path for JSON persistence.
    storage_path: PathBuf,
    /// Thread-safe in-memory cache map.
    entries: RwLock<HashMap<String, ProbeEntry>>,
    /// Atomic flag indicating unpersisted in-memory mutations.
    dirty: AtomicBool,
}

impl ProbeCache {
    /// Instantiates a new empty [`ProbeCache`] attached to a specific storage path.
    ///
    /// # Arguments
    /// * `storage_path` - Target path where JSON cache state is persisted.
    ///
    /// # Returns
    /// * `Self` - Initialized cache with empty memory map.
    ///
    /// # Errors
    /// * None.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Initializes clean dirty flag and empty entries map.
    pub fn new(storage_path: PathBuf) -> Self {
        Self {
            storage_path,
            entries: RwLock::new(HashMap::new()),
            dirty: AtomicBool::new(false),
        }
    }

    /// Loads an existing probe cache from disk or creates a new empty one if not present.
    ///
    /// # Arguments
    /// * `storage_path` - Target filesystem path for the cache file.
    ///
    /// # Returns
    /// * `Result<Self>` - Initialized cache populated with loaded entries.
    ///
    /// # Errors
    /// * Returns `Err` if existing file cannot be parsed or read.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Inspects filesystem: if file exists, reads and deserializes [`SerializedProbeCache`].
    /// * If file is missing, creates empty cache and marks dirty as false.
    pub fn load_or_create(storage_path: PathBuf) -> Result<Self> {
        if storage_path.is_file() {
            let mut file = File::open(&storage_path)
                .with_context(|| format!("Failed to open cache at {:?}", storage_path))?;
            let mut content = String::new();
            file.read_to_string(&mut content)?;

            let parsed: SerializedProbeCache = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse JSON cache at {:?}", storage_path))?;

            Ok(Self {
                storage_path,
                entries: RwLock::new(parsed.entries),
                dirty: AtomicBool::new(false),
            })
        } else {
            Ok(Self::new(storage_path))
        }
    }

    /// Queries the cache for a given moviehash and file size.
    ///
    /// # Arguments
    /// * `moviehash` - 16-character hexadecimal moviehash.
    /// * `file_size` - File size in bytes.
    ///
    /// # Returns
    /// * `Option<ProbeEntry>` - Cloned probe entry if present in memory, or `None`.
    ///
    /// # Errors
    /// * None.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Acquires read lock on internal `HashMap` and returns cloned value on hit.
    pub fn get(&self, moviehash: &str, file_size: u64) -> Option<ProbeEntry> {
        let key = format!("{}:{}", moviehash, file_size);
        let guard = self.entries.read().ok()?;
        guard.get(&key).cloned()
    }

    /// Inserts or updates a probe entry in the cache and marks state as dirty.
    ///
    /// # Arguments
    /// * `entry` - Fully populated [`ProbeEntry`].
    ///
    /// # Returns
    /// * `Result<()>` - Success confirmation.
    ///
    /// # Errors
    /// * Returns `Err` if internal lock is poisoned.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Acquires write lock on internal map, stores entry by composite key, and sets dirty flag.
    pub fn put(&self, entry: ProbeEntry) -> Result<()> {
        let key = entry.cache_key();
        let mut guard = self
            .entries
            .write()
            .map_err(|e| anyhow!("Cache lock poisoned: {}", e))?;
        guard.insert(key, entry);
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }

    /// Persists all in-memory cache entries to the configured disk file atomically.
    ///
    /// # Arguments
    /// * None.
    ///
    /// # Returns
    /// * `Result<()>` - Confirmation of successful disk sync.
    ///
    /// # Errors
    /// * Returns `Err` if parent directories cannot be created or file write fails.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Serializes `SerializedProbeCache` to temporary string.
    /// * Creates parent directory if missing.
    /// * Writes payload atomically and clears dirty flag on success.
    pub fn save(&self) -> Result<()> {
        let guard = self
            .entries
            .read()
            .map_err(|e| anyhow!("Cache lock poisoned: {}", e))?;
        let data = SerializedProbeCache {
            version: PROBE_CACHE_SCHEMA_VERSION,
            entries: guard.clone(),
        };
        let serialized = serde_json::to_string_pretty(&data)?;
        drop(guard);

        if let Some(parent) = self.storage_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.storage_path)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;

        self.dirty.store(false, Ordering::Release);
        Ok(())
    }

    /// Returns the number of cached probe entries.
    ///
    /// # Arguments
    /// * None.
    ///
    /// # Returns
    /// * `usize` - Entry count.
    ///
    /// # Errors
    /// * None.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Acquires read lock and returns map length.
    pub fn len(&self) -> usize {
        self.entries.read().map(|g| g.len()).unwrap_or(0)
    }

    /// Returns `true` if the cache contains zero entries.
    ///
    /// # Arguments
    /// * None.
    ///
    /// # Returns
    /// * `bool` - True when cache is empty.
    ///
    /// # Errors
    /// * None.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Returns `len() == 0`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a cloned snapshot of all cached probe entries.
    ///
    /// # Arguments
    /// * None.
    ///
    /// # Returns
    /// * `Vec<ProbeEntry>` - Cloned list of all current entries.
    ///
    /// # Errors
    /// * None.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Acquires read lock on internal entries map and clones values into a vector.
    pub fn all_entries(&self) -> Vec<ProbeEntry> {
        self.entries
            .read()
            .map(|g| g.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Queries the cache for a local video file, falling back to `probe_video` and updating cache on miss.
    ///
    /// # Arguments
    /// * `video_path` - Path to the local media file.
    ///
    /// # Returns
    /// * `Result<ProbeEntry>` - Retrieved or probed entry.
    ///
    /// # Errors
    /// * Returns `Err` if moviehash cannot be computed or probing fails.
    ///
    /// # Panics
    /// * Guaranteed never to panic.
    ///
    /// # Data Flow & Invariants
    /// * Computes moviehash via [`compute_local_file_moviehash`].
    /// * On cache hit: returns entry immediately with zero child processes spawned.
    /// * On cache miss: runs `probe_video`, constructs entry, stores in cache, calls `save()`, and returns.
    pub fn query_or_probe_local(&self, video_path: &Path) -> Result<ProbeEntry> {
        let (moviehash, file_size) = compute_local_file_moviehash(video_path)?;

        if let Some(hit) = self.get(&moviehash, file_size) {
            return Ok(hit);
        }

        // Cache miss: execute probe_video
        let meta = crate::video::probe_video(video_path)?;
        let duration = if meta.duration < MIN_VALID_DURATION_SECONDS {
            0.0
        } else {
            meta.duration
        };
        let fps = if meta.fps < MIN_VALID_FPS {
            DEFAULT_FALLBACK_FPS
        } else {
            meta.fps
        };

        let now_epoch_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let entry = ProbeEntry {
            moviehash,
            file_size,
            duration_seconds: duration,
            fps,
            video_codec: None,
            width: None,
            height: None,
            probed_at_epoch_s: now_epoch_s,
        };

        self.put(entry.clone())?;
        let _ = self.save();
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_compute_moviehash_from_head_tail_valid() {
        let file_size: u64 = 1_000_000;
        let head = vec![1u8; CHUNK_SIZE_BYTES];
        let tail = vec![2u8; CHUNK_SIZE_BYTES];

        let hash = compute_moviehash_from_head_tail(file_size, &head, &tail).unwrap();
        assert_eq!(hash.len(), MOVIEHASH_HEX_LENGTH);
        assert!(!hash.is_empty());
    }

    #[test]
    fn test_compute_moviehash_too_small_file() {
        let file_size: u64 = 100;
        let head = vec![0u8; CHUNK_SIZE_BYTES];
        let tail = vec![0u8; CHUNK_SIZE_BYTES];

        let res = compute_moviehash_from_head_tail(file_size, &head, &tail);
        assert!(res.is_err());
    }

    #[test]
    fn test_probe_entry_cut_offset() {
        let entry = ProbeEntry {
            moviehash: "0123456789abcdef".to_string(),
            file_size: 10_000_000,
            duration_seconds: 100.0,
            fps: 25.0,
            video_codec: Some("h264".to_string()),
            width: Some(1920),
            height: Some(1080),
            probed_at_epoch_s: 1_700_000_000,
        };

        assert_eq!(entry.calculate_cut_offset(0.20), 20.0);
        assert_eq!(entry.calculate_cut_offset(0.50), 50.0);
        assert_eq!(entry.calculate_cut_offset(0.80), 80.0);
        assert_eq!(entry.calculate_cut_offset(-0.10), 0.0);
        assert_eq!(entry.calculate_cut_offset(1.50), 100.0);
    }

    #[test]
    fn test_probe_cache_lifecycle_and_persistence() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_probe_cache_{}", std::process::id()));
        let cache_file = temp_dir.join("test_cache.json");

        let cache = ProbeCache::load_or_create(cache_file.clone()).unwrap();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());

        let entry = ProbeEntry {
            moviehash: "abcdef0123456789".to_string(),
            file_size: 5_000_000,
            duration_seconds: 120.0,
            fps: 24.0,
            video_codec: Some("hevc".to_string()),
            width: Some(3840),
            height: Some(2160),
            probed_at_epoch_s: 1_700_000_100,
        };

        cache.put(entry.clone()).unwrap();
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());

        let retrieved = cache.get("abcdef0123456789", 5_000_000).unwrap();
        assert_eq!(retrieved, entry);

        // Save to disk
        cache.save().unwrap();
        assert!(cache_file.is_file());

        // Reload from disk in a fresh cache instance
        let reloaded = ProbeCache::load_or_create(cache_file.clone()).unwrap();
        assert_eq!(reloaded.len(), 1);
        let reloaded_entry = reloaded.get("abcdef0123456789", 5_000_000).unwrap();
        assert_eq!(reloaded_entry, entry);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compute_local_file_moviehash_roundtrip() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_local_moviehash_{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let test_file = temp_dir.join("dummy_video.mkv");

        let total_size = CHUNK_SIZE_BYTES * 2 + 1024;
        let mut f = File::create(&test_file).unwrap();
        let payload = vec![0x42u8; total_size];
        f.write_all(&payload).unwrap();
        f.flush().unwrap();
        drop(f);

        let (hash, size) = compute_local_file_moviehash(&test_file).unwrap();
        assert_eq!(size, total_size as u64);
        assert_eq!(hash.len(), MOVIEHASH_HEX_LENGTH);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
