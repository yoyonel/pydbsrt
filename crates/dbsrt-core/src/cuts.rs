//! Scene cuts detection and .phash binary serialization.

use crate::hamming_distance;
use anyhow::{Context, Result};
use std::fs;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_THRESHOLD_DISTANCE_FOR_CUT: u32 = 16;
pub const DEFAULT_SEARCH_DISTANCE_FOR_BKTREE: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutItem {
    pub phash: u64,
    pub offset: usize,
}

/// A scene cut with its exact presentation timestamp in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampedCutItem {
    pub phash: u64,
    pub offset: usize,
    pub timestamp_ms: u64,
}

/// Standard 2D DCT-II hashes on uniform flat white or black frames.
pub const FLAT_WHITE_PHASH: u64 = 0x8669_3900_3986_c639;
pub const FLAT_BLACK_PHASH: u64 = 0x0000_0000_0000_0000;
pub const FLAT_BLACK_DITHERED_1: u64 = 0x8f4c_7233_588e_457e;
pub const FLAT_BLACK_DITHERED_2: u64 = 0x8fcc_7233_588e_453e;
pub const FLAT_BLACK_DITHERED_3: u64 = 0x86c6_9696_86c6_d6c6;

/// Maximum Hamming distance between consecutive frames considered static compression jitter.
pub const STATIC_FRAME_MAX_DISTANCE: u32 = 1;

/// Maximum Hamming distance between any pair of distinct frames in a low-motion window considered compression noise.
pub const STATIC_SEQUENCE_MAX_JITTER_DISTANCE: u32 = 2;

/// Minimum window length to evaluate low-entropy multi-frame compression jitter.
pub const MIN_DYNAMIC_SEQUENCE_WINDOW_LEN: usize = 10;

/// Minimum count of distinct non-blank visual hashes required in a query sequence to certify temporal dynamics.
pub const MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT: usize = 2;

/// Returns true if a perceptual hash has abnormally low or high bit density,
/// or matches a known uniform/flat screen fingerprint (black screen, fade-to-black, white frame, static intro).
/// Standard 2D DCT-II hashes on natural scenes have ~32 active bits (median thresholding).
///
/// # Arguments
/// * `hash` - 64-bit perceptual hash value to evaluate.
///
/// # Returns
/// * `bool` - `true` if the hash is identified as a uniform, blank, or invariant non-discriminant frame; `false` otherwise.
///
/// # Errors
/// * Guarantee: Never returns Err.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Computes active bit population count via `count_ones()`.
/// 2. Rejects hashes with bit densities outside nominal range `[10, 54]`.
/// 3. Checks against known static uniform/dithered white and black signatures (`FLAT_WHITE_PHASH`, `FLAT_BLACK_PHASH`, `FLAT_BLACK_DITHERED_1`, `FLAT_BLACK_DITHERED_2`, `FLAT_BLACK_DITHERED_3`).
#[inline]
pub fn is_blank_phash(hash: u64) -> bool {
    let ones = hash.count_ones();
    !(10..=54).contains(&ones)
        || hash == FLAT_WHITE_PHASH
        || hash == FLAT_BLACK_PHASH
        || hash == FLAT_BLACK_DITHERED_1
        || hash == FLAT_BLACK_DITHERED_2
        || hash == FLAT_BLACK_DITHERED_3
}

/// Returns `true` when a query carries no visual information: empty or every frame blank.
///
/// Recognition gates use this instead of [`is_static_sequence`]: a static but textured
/// scene (dialogue shot, title card) is a legitimate query and must reach the index.
pub fn is_blank_sequence(hashes: &[u64]) -> bool {
    hashes.iter().all(|&h| is_blank_phash(h))
}

/// Evaluates whether a sequence of perceptual hashes represents a static or invariant scene
/// (e.g. frozen frame, black screen, title card, or encoder dither without visual motion).
///
/// # Arguments
/// * `hashes` - Slice of 64-bit perceptual hashes representing consecutive video frames.
///
/// # Returns
/// * `bool` - `true` if the sequence lacks temporal dynamics (empty, blank single frame,
///   all consecutive transitions with distance <= `STATIC_FRAME_MAX_DISTANCE`, fewer than
///   `MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT` distinct non-blank hashes, or low-motion jitter
///   where pairwise distance <= `STATIC_SEQUENCE_MAX_JITTER_DISTANCE`); `false` if genuine motion exists.
///
/// # Errors
/// * Guarantee: Never returns Err.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Empty slices are considered static.
/// 2. Single frames are evaluated via `is_blank_phash`.
/// 3. Filters out blank hashes via `is_blank_phash`; if remaining count < `MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT`, returns `true`.
/// 4. If all adjacent transitions `(h_i, h_{i+1})` have Hamming distance <= `STATIC_FRAME_MAX_DISTANCE`, returns `true`.
/// 5. Verifies unique hash cardinality >= `MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT`.
/// 6. For windows >= `MIN_DYNAMIC_SEQUENCE_WINDOW_LEN`, flags sequences with <= 2 distinct frames differing by <= `STATIC_SEQUENCE_MAX_JITTER_DISTANCE` as encoder jitter.
pub fn is_static_sequence(hashes: &[u64]) -> bool {
    if hashes.is_empty() {
        return true;
    }
    if hashes.len() == 1 {
        return is_blank_phash(hashes[0]);
    }
    let non_blank: Vec<u64> = hashes
        .iter()
        .copied()
        .filter(|&h| !is_blank_phash(h))
        .collect();

    if non_blank.len() < MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT {
        return true;
    }

    let all_consecutive_static = hashes
        .windows(2)
        .all(|w| hamming_distance(w[0], w[1]) <= STATIC_FRAME_MAX_DISTANCE);

    if all_consecutive_static {
        return true;
    }

    let mut distinct = non_blank;
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() < MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT {
        return true;
    }

    if hashes.len() >= MIN_DYNAMIC_SEQUENCE_WINDOW_LEN
        && distinct.len() <= MIN_DYNAMIC_SEQUENCE_DISTINCT_COUNT
        && hamming_distance(distinct[0], distinct[1]) <= STATIC_SEQUENCE_MAX_JITTER_DISTANCE
    {
        return true;
    }

    false
}

/// Detects scene cut frames from consecutive frame hashes.
/// Whenever hamming_distance(h[i], h[i+1]) > threshold, frames are recorded as cut items,
/// filtering out blank/uniform frames and deduplicating consecutive identical cut offsets.
pub fn detect_cuts(hashes: &[u64], threshold: u32) -> Vec<CutItem> {
    let mut cuts = Vec::new();
    if hashes.len() < 2 {
        return cuts;
    }

    for (offset, window) in hashes.windows(2).enumerate() {
        let h0 = window[0];
        let h1 = window[1];
        if hamming_distance(h0, h1) > threshold {
            if !is_blank_phash(h0) && cuts.last().map(|c| c.offset) != Some(offset) {
                cuts.push(CutItem { phash: h0, offset });
            }
            if !is_blank_phash(h1) {
                cuts.push(CutItem {
                    phash: h1,
                    offset: offset + 1,
                });
            }
        }
    }

    cuts
}

/// Detects scene cut frames from timestamped FrameSamples.
/// Retains exact frame offsets and presentation timestamps in milliseconds.
pub fn detect_cuts_timestamped(
    samples: &[crate::video::FrameSample],
    threshold: u32,
) -> Vec<TimestampedCutItem> {
    let mut cuts = Vec::new();
    if samples.len() < 2 {
        return cuts;
    }

    for window in samples.windows(2) {
        let s0 = &window[0];
        let s1 = &window[1];
        if hamming_distance(s0.phash, s1.phash) > threshold {
            if !is_blank_phash(s0.phash) && cuts.last().map(|c| c.offset) != Some(s0.frame_idx) {
                cuts.push(TimestampedCutItem {
                    phash: s0.phash,
                    offset: s0.frame_idx,
                    timestamp_ms: s0.timestamp_ms,
                });
            }
            if !is_blank_phash(s1.phash) {
                cuts.push(TimestampedCutItem {
                    phash: s1.phash,
                    offset: s1.frame_idx,
                    timestamp_ms: s1.timestamp_ms,
                });
            }
        }
    }

    cuts
}

/// Reads a binary .phash file (8-byte big-endian integers).
pub fn read_phash_file(path: &Path) -> Result<Vec<u64>> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open .phash file {:?}", path))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let count = buf.len() / 8;
    let (chunks, _) = buf.as_chunks::<8>();
    let mut hashes = Vec::with_capacity(count);
    for &chunk in chunks {
        let val = u64::from_be_bytes(chunk);
        hashes.push(val);
    }
    Ok(hashes)
}

/// A `.phash` file that already holds `done` hashes from an interrupted extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhashCheckpoint {
    pub path: PathBuf,
    pub done: usize,
}

impl PhashCheckpoint {
    /// Opens the file for appending after its first `done` hashes, dropping anything past them:
    /// a torn last write, or frames from a failed attempt that is now being retried.
    pub fn open_append(&self) -> Result<BufWriter<fs::File>> {
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("Failed to open checkpoint {:?}", self.path))?;
        file.set_len(self.done as u64 * 8)?;
        Ok(BufWriter::new(file))
    }
}

/// Writes a binary .phash file (8-byte big-endian integers).
pub fn write_phash_file(path: &Path, hashes: &[u64]) -> Result<()> {
    let mut file = fs::File::create(path)
        .with_context(|| format!("Failed to create .phash file {:?}", path))?;
    for &h in hashes {
        file.write_all(&h.to_be_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_blank_phash() {
        assert!(is_blank_phash(0x0000_0000_0000_0000));
        assert!(is_blank_phash(0x8080_0080_0080_0080)); // 5 bits set (uniform black)
        assert!(is_blank_phash(0xFFFF_FFFF_FFFF_FFFF)); // 64 bits set (uniform white)
        assert!(is_blank_phash(FLAT_WHITE_PHASH));
        assert!(is_blank_phash(FLAT_BLACK_PHASH));
        assert!(is_blank_phash(FLAT_BLACK_DITHERED_1));
        assert!(is_blank_phash(FLAT_BLACK_DITHERED_2));
        assert!(is_blank_phash(FLAT_BLACK_DITHERED_3));
        assert!(!is_blank_phash(0xAAAA_AAAA_5555_5555)); // 32 bits set (natural image)
        assert!(!is_blank_phash(0xF72C_F0EA_5EC4_130A)); // 32 bits set
    }

    #[test]
    fn test_is_blank_sequence() {
        assert!(is_blank_sequence(&[]));
        assert!(is_blank_sequence(&[
            FLAT_BLACK_PHASH,
            FLAT_BLACK_DITHERED_3
        ]));
        // A static textured scene is static but not blank: it stays queryable.
        let freeze_5 = vec![0xAAAA_AAAA_5555_5555; 5];
        assert!(is_static_sequence(&freeze_5));
        assert!(!is_blank_sequence(&freeze_5));
        assert!(!is_blank_sequence(&[
            FLAT_BLACK_PHASH,
            0xAAAA_AAAA_5555_5555
        ]));
    }

    #[test]
    fn test_is_static_sequence() {
        // Empty sequence is static
        assert!(is_static_sequence(&[]));

        // Single blank frame is static
        assert!(is_static_sequence(&[0x0000_0000_0000_0000]));
        assert!(is_static_sequence(&[FLAT_BLACK_DITHERED_1]));
        assert!(is_static_sequence(&[FLAT_BLACK_DITHERED_3]));

        // Single natural frame is not static
        assert!(!is_static_sequence(&[0xAAAA_AAAA_5555_5555]));

        // 5 identical frames of ANY natural hash (freeze frame / static card) -> static
        let freeze_5 = vec![0xAAAA_AAAA_5555_5555; 5];
        assert!(is_static_sequence(&freeze_5));

        // 5 frames with 1-bit compression jitter -> static
        let jitter_5 = vec![
            0xAAAA_AAAA_5555_5555,
            0xAAAA_AAAA_5555_5554, // 1 bit flip
            0xAAAA_AAAA_5555_5555,
            0xAAAA_AAAA_5555_5554,
            0xAAAA_AAAA_5555_5555,
        ];
        assert!(is_static_sequence(&jitter_5));

        // 5 frames with genuine natural motion (distance >= 2, distinct >= 2) -> dynamic
        let motion_5 = vec![
            0xAAAA_AAAA_5555_5555,
            0xAAAA_AAAA_5555_5550, // 2 bits flip
            0xAAAA_AAAA_5555_5500, // 4 bits flip
            0xAAAA_AAAA_5555_0000,
            0xAAAA_AAAA_5550_0000,
        ];
        assert!(!is_static_sequence(&motion_5));

        // 12 frames with 2-bit jitter (e.g. compression noise on dark/fade frames) -> static
        let mut jitter_12 = vec![0xAAAA_AAAA_5555_5555; 12];
        jitter_12[3] = 0xAAAA_AAAA_5555_5556; // 2 bits flip
        jitter_12[7] = 0xAAAA_AAAA_5555_5556;
        assert!(is_static_sequence(&jitter_12));
    }

    #[test]
    fn test_detect_cuts_filters_blank() {
        let natural1 = 0xAAAA_AAAA_5555_5555;
        let natural2 = 0x5555_5555_AAAA_AAAA;
        let black = 0x8080_0080_0080_0080;

        // Cut between natural1 and black: black should not be registered as a valid cut
        let seq = vec![natural1, black, natural2];
        let cuts = detect_cuts(&seq, 16);
        assert_eq!(cuts.len(), 2);
        assert_eq!(cuts[0].offset, 0);
        assert_eq!(cuts[0].phash, natural1);
        assert_eq!(cuts[1].offset, 2);
        assert_eq!(cuts[1].phash, natural2);
    }

    #[test]
    fn test_detect_cuts_edge_cases() {
        assert!(detect_cuts(&[], 16).is_empty());
        assert!(detect_cuts(&[0x1234], 16).is_empty());

        let h1 = 0xAAAA_AAAA_5555_5555;
        let cuts_identical = detect_cuts(&[h1, h1, h1], 16);
        assert!(cuts_identical.is_empty());
    }

    #[test]
    fn test_phash_file_roundtrip() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_roundtrip.phash");
        let sample = vec![0x1234_5678_9ABC_DEF0, 0xFEDC_BA98_7654_3210];

        write_phash_file(&test_file, &sample).expect("write failed");
        let loaded = read_phash_file(&test_file).expect("read failed");
        assert_eq!(sample, loaded);

        let _ = std::fs::remove_file(test_file);
    }

    #[test]
    fn test_phash_checkpoint_truncates_then_appends() {
        let path =
            std::env::temp_dir().join(format!("test_checkpoint_{}.phash", std::process::id()));
        // Two whole hashes plus a torn third write.
        let mut raw = [1u64, 2, 3].map(u64::to_be_bytes).concat();
        raw.truncate(20);
        std::fs::write(&path, raw).unwrap();

        let checkpoint = PhashCheckpoint {
            path: path.clone(),
            done: 2,
        };
        let mut writer = checkpoint.open_append().unwrap();
        writer.write_all(&9u64.to_be_bytes()).unwrap();
        drop(writer);
        assert_eq!(read_phash_file(&path).unwrap(), vec![1, 2, 9]);

        // A retry from the same checkpoint drops the previous attempt's frames.
        let mut writer = checkpoint.open_append().unwrap();
        writer.write_all(&7u64.to_be_bytes()).unwrap();
        drop(writer);
        assert_eq!(read_phash_file(&path).unwrap(), vec![1, 2, 7]);

        let _ = std::fs::remove_file(path);
    }
}
