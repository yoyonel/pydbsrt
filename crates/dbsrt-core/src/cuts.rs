//! Scene cuts detection and .phash binary serialization.

use crate::hamming_distance;
use anyhow::{Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

pub const DEFAULT_THRESHOLD_DISTANCE_FOR_CUT: u32 = 16;
pub const DEFAULT_SEARCH_DISTANCE_FOR_BKTREE: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutItem {
    pub phash: u64,
    pub offset: usize,
}

/// Returns true if a perceptual hash has abnormally low or high bit density,
/// indicating an uninformative uniform/blank frame (black screen, fade-to-black, white frame).
/// Standard 2D DCT-II hashes on natural scenes have ~32 active bits (median thresholding).
#[inline]
pub fn is_blank_phash(hash: u64) -> bool {
    let ones = hash.count_ones();
    !(10..=54).contains(&ones)
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
        assert!(!is_blank_phash(0xAAAA_AAAA_5555_5555)); // 32 bits set (natural image)
        assert!(!is_blank_phash(0xF72C_F0EA_5EC4_130A)); // 32 bits set
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
}
