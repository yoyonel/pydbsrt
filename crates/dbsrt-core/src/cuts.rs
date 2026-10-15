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

/// Detects scene cut frames from consecutive frame hashes.
/// Whenever hamming_distance(h[i], h[i+1]) > threshold, both frames are recorded.
pub fn detect_cuts(hashes: &[u64], threshold: u32) -> Vec<CutItem> {
    let mut cuts = Vec::new();
    if hashes.len() < 2 {
        return cuts;
    }

    for (offset, window) in hashes.windows(2).enumerate() {
        let h0 = window[0];
        let h1 = window[1];
        if hamming_distance(h0, h1) > threshold {
            cuts.push(CutItem { phash: h0, offset });
            cuts.push(CutItem {
                phash: h1,
                offset: offset + 1,
            });
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
