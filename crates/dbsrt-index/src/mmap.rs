//! Zero-copy memory-mapped search engine for raw .phash files.
//!
//! Enables lightning-fast perceptual hash searching directly across disk files
//! without needing a PostgreSQL database or external daemon.

use anyhow::{Context, Result};
use dbsrt_core::hamming_distance;
use memmap2::Mmap;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};

/// A memory-mapped .phash binary file (contiguous big-endian u64 frames).
pub struct MmapPhashFile {
    pub path: PathBuf,
    pub name: String,
    _file: File,
    mmap: Mmap,
    len: usize,
}

impl MmapPhashFile {
    /// Opens and memory-maps a .phash file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let name = path_buf
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let file = File::open(&path_buf)
            .with_context(|| format!("Failed to open phash file {:?}", path_buf))?;
        let mmap = unsafe { Mmap::map(&file)? };
        let len = mmap.len() / 8;

        Ok(Self {
            path: path_buf,
            name,
            _file: file,
            mmap,
            len,
        })
    }

    /// Number of frames in this phash file.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Gets perceptual hash at frame index.
    #[inline]
    pub fn get_hash(&self, index: usize) -> Option<u64> {
        if index >= self.len {
            return None;
        }
        let offset = index * 8;
        let bytes: [u8; 8] = self.mmap[offset..offset + 8].try_into().ok()?;
        Some(u64::from_be_bytes(bytes))
    }

    /// Scans the file against a list of query hashes.
    ///
    /// Returns a list of matches: (query_offset, target_frame_offset, distance).
    pub fn search_query_hashes(
        &self,
        query_hashes: &[u64],
        max_distance: u32,
    ) -> Vec<(usize, usize, u32)> {
        let (chunks, _) = self.mmap.as_chunks::<8>();

        query_hashes
            .par_iter()
            .enumerate()
            .flat_map(|(q_idx, &q_hash)| {
                chunks
                    .iter()
                    .enumerate()
                    .filter_map(move |(t_idx, chunk)| {
                        let t_hash = u64::from_be_bytes(*chunk);
                        let dist = hamming_distance(q_hash, t_hash);
                        if dist <= max_distance {
                            Some((q_idx, t_idx, dist))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// Aggregated match result for a single file in the library.
#[derive(Debug, Clone)]
pub struct LibraryMediaMatch {
    pub file_path: PathBuf,
    pub media_name: String,
    pub nb_offsets_match: usize,
    pub search_offsets_match: HashSet<usize>,
    pub match_frames_offsets: HashSet<usize>,
    pub confidence: f64,
}

/// A collection of memory-mapped .phash files forming a local zero-DB library.
pub struct MmapLibrary {
    files: Vec<MmapPhashFile>,
}

impl MmapLibrary {
    /// Loads all .phash files found recursively in `dir`.
    pub fn from_directory<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let mut files = Vec::new();
        Self::collect_phash_files(dir.as_ref(), &mut files)?;
        Ok(Self { files })
    }

    fn collect_phash_files(dir: &Path, files: &mut Vec<MmapPhashFile>) -> Result<()> {
        Self::collect_phash_files_bounded(dir, files, 0)
    }

    fn collect_phash_files_bounded(
        dir: &Path,
        files: &mut Vec<MmapPhashFile>,
        depth: usize,
    ) -> Result<()> {
        if depth > 5 || !dir.exists() {
            return Ok(());
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return Ok(()),
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                let _ = Self::collect_phash_files_bounded(&path, files, depth + 1);
            } else if path.extension().and_then(|e| e.to_str()) == Some("phash") {
                if let Ok(mmap_file) = MmapPhashFile::open(&path) {
                    files.push(mmap_file);
                }
            }
        }
        Ok(())
    }

    /// Number of media files in this library.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total number of frame hashes across the whole library.
    pub fn total_frames(&self) -> usize {
        self.files.iter().map(|f| f.len()).sum()
    }

    /// Searches query hashes across all media files in parallel using rayon.
    pub fn recognize(&self, query_hashes: &[u64], max_distance: u32) -> Vec<LibraryMediaMatch> {
        if query_hashes.is_empty() || self.files.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<LibraryMediaMatch> = self
            .files
            .par_iter()
            .filter_map(|file| {
                let raw_matches = file.search_query_hashes(query_hashes, max_distance);
                if raw_matches.is_empty() {
                    return None;
                }

                let mut search_offsets = HashSet::new();
                let mut match_offsets = HashSet::new();
                let nb_matches = raw_matches.len();

                for (q_idx, t_idx, _dist) in raw_matches {
                    search_offsets.insert(q_idx);
                    match_offsets.insert(t_idx);
                }

                let confidence = search_offsets.len() as f64 / query_hashes.len() as f64;

                Some(LibraryMediaMatch {
                    file_path: file.path.clone(),
                    media_name: file.name.clone(),
                    nb_offsets_match: nb_matches,
                    search_offsets_match: search_offsets,
                    match_frames_offsets: match_offsets,
                    confidence,
                })
            })
            .collect();

        // Sort descending by number of matched offsets
        results.sort_by_key(|r| std::cmp::Reverse(r.nb_offsets_match));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_mmap_phash_file() -> Result<()> {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_mmap_sample.phash");

        let hashes = vec![
            0x1122334455667788u64,
            0xAABBCCDDEEFF0011u64,
            0x123456789ABCDEF0u64,
        ];
        {
            let mut f = File::create(&test_file)?;
            for &h in &hashes {
                f.write_all(&h.to_be_bytes())?;
            }
        }

        let mmap_phash = MmapPhashFile::open(&test_file)?;
        assert_eq!(mmap_phash.len(), 3);
        assert_eq!(mmap_phash.get_hash(0), Some(0x1122334455667788u64));
        assert_eq!(mmap_phash.get_hash(1), Some(0xAABBCCDDEEFF0011u64));
        assert_eq!(mmap_phash.get_hash(2), Some(0x123456789ABCDEF0u64));
        assert_eq!(mmap_phash.get_hash(3), None);

        // Test search
        let query = vec![0x1122334455667788u64];
        let matches = mmap_phash.search_query_hashes(&query, 0);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], (0, 0, 0));

        let _ = std::fs::remove_file(&test_file);
        Ok(())
    }
}
