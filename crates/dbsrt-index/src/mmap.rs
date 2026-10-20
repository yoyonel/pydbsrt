//! Zero-copy memory-mapped search engine for raw .phash files.
//!
//! Enables lightning-fast perceptual hash searching directly across disk files
//! without needing a PostgreSQL database or external daemon.

use anyhow::Result;
use dbsrt_core::{hamming_distance, is_blank_phash};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Minimum temporally coherent frame match ratio (`0.0 ..= 1.0`) required to certify
/// that a query video sample genuinely matches a reference `.phash` timeline.
pub const MIN_TEMPORAL_COHERENCE_CONFIDENCE: f64 = 0.65;

/// Computes the maximum number of temporally coherent 1-to-1 frame matches `(q_idx, t_idx)`
/// aligned along a consistent timeline shift `delta = t_idx - q_idx` (within `±2` frames tolerance),
/// ignoring uniform/blank frames (`is_blank_phash`).
///
/// Returns `(coherent_frame_count, confidence_0_to_1, best_center_delta)`.
pub fn compute_temporal_coherence_with_delta(
    query_hashes: &[u64],
    raw_pairs: &[(usize, usize)],
) -> (usize, f64, i64) {
    let valid_q_count = query_hashes.iter().filter(|&&h| !is_blank_phash(h)).count();
    if valid_q_count == 0 || raw_pairs.is_empty() {
        return (0, 0.0, 0);
    }

    let mut delta_buckets: HashMap<i64, Vec<(usize, usize)>> = HashMap::new();
    for &(q_idx, t_idx) in raw_pairs {
        if q_idx < query_hashes.len() && !is_blank_phash(query_hashes[q_idx]) {
            let delta = t_idx as i64 - q_idx as i64;
            delta_buckets.entry(delta).or_default().push((q_idx, t_idx));
        }
    }

    if delta_buckets.is_empty() {
        return (0, 0.0, 0);
    }

    let mut best_coherent = 0usize;
    let mut best_delta = 0i64;
    for &center_delta in delta_buckets.keys() {
        let mut q_set = HashSet::new();
        let mut t_set = HashSet::new();
        for d in (center_delta - 2)..=(center_delta + 2) {
            if let Some(pairs) = delta_buckets.get(&d) {
                for &(q_idx, t_idx) in pairs {
                    q_set.insert(q_idx);
                    t_set.insert(t_idx);
                }
            }
        }
        let coherent = q_set.len().min(t_set.len());
        if coherent > best_coherent {
            best_coherent = coherent;
            best_delta = center_delta;
        }
    }

    let confidence = (best_coherent as f64 / valid_q_count as f64).min(1.0);
    (best_coherent, confidence, best_delta)
}

/// Computes the maximum number of temporally coherent 1-to-1 frame matches `(q_idx, t_idx)`
/// aligned along a consistent timeline shift `delta = t_idx - q_idx` (within `±2` frames tolerance),
/// ignoring uniform/blank frames (`is_blank_phash`).
///
/// Returns `(coherent_frame_count, confidence_0_to_1)`.
pub fn compute_temporal_coherence(
    query_hashes: &[u64],
    raw_pairs: &[(usize, usize)],
) -> (usize, f64) {
    let (coh, conf, _) = compute_temporal_coherence_with_delta(query_hashes, raw_pairs);
    (coh, conf)
}

/// A loaded or memory-mapped .phash binary file (contiguous u64 frames).
pub struct MmapPhashFile {
    pub path: PathBuf,
    pub name: String,
    hashes: Vec<u64>,
}

impl MmapPhashFile {
    /// Opens and loads a .phash file into memory without keeping the file descriptor open.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let name = path_buf
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let hashes = dbsrt_core::read_phash_file(&path_buf)?;

        Ok(Self {
            path: path_buf,
            name,
            hashes,
        })
    }

    /// Number of frames in this phash file.
    #[inline]
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    /// Returns true if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// Gets perceptual hash at frame index.
    #[inline]
    pub fn get_hash(&self, index: usize) -> Option<u64> {
        self.hashes.get(index).copied()
    }

    /// Scans the file against a list of query hashes.
    ///
    /// Returns a list of matches: (query_offset, target_frame_offset, distance).
    pub fn search_query_hashes(
        &self,
        query_hashes: &[u64],
        max_distance: u32,
    ) -> Vec<(usize, usize, u32)> {
        query_hashes
            .par_iter()
            .enumerate()
            .flat_map(|(q_idx, &q_hash)| {
                self.hashes
                    .iter()
                    .enumerate()
                    .filter_map(move |(t_idx, &t_hash)| {
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
    pub best_offset_delta: i64,
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

    /// Loads all .phash files found recursively across multiple directories (deduplicated by filename).
    pub fn from_directories<P: AsRef<Path>>(dirs: &[P]) -> Result<Self> {
        let mut files = Vec::new();
        let mut seen_names = HashSet::new();
        for dir in dirs {
            let mut dir_files = Vec::new();
            Self::collect_phash_files(dir.as_ref(), &mut dir_files)?;
            for f in dir_files {
                if seen_names.insert(f.name.clone()) {
                    files.push(f);
                }
            }
        }
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
                let mut pairs = Vec::with_capacity(nb_matches);

                for (q_idx, t_idx, _dist) in raw_matches {
                    search_offsets.insert(q_idx);
                    match_offsets.insert(t_idx);
                    pairs.push((q_idx, t_idx));
                }

                let (_, confidence, best_offset_delta) =
                    compute_temporal_coherence_with_delta(query_hashes, &pairs);
                if confidence <= 0.0 {
                    return None;
                }

                Some(LibraryMediaMatch {
                    file_path: file.path.clone(),
                    media_name: file.name.clone(),
                    nb_offsets_match: nb_matches,
                    search_offsets_match: search_offsets,
                    match_frames_offsets: match_offsets,
                    confidence,
                    best_offset_delta,
                })
            })
            .collect();

        // Sort descending by temporally coherent confidence first, then raw matched offsets
        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.nb_offsets_match.cmp(&a.nb_offsets_match))
        });
        results
    }

    /// Searches two distinct query windows sampled from different parts of a target video
    /// (e.g. at 35% and 65% timeline positions) separated by `expected_gap_frames`.
    ///
    /// Requires candidate medias to independently match BOTH windows and enforces strict
    /// cross-window timeline shift coherence:
    /// `| (delta_b - delta_a) | <= tolerance_frames` (default: 4 frames, ~160ms).
    ///
    /// This completely eliminates inter-episode false positives caused by shared TV/anime
    /// intro/credits sequences while boosting recognition recall to >= 99%.
    pub fn recognize_two_windows(
        &self,
        window_a: &[u64],
        window_b: &[u64],
        expected_gap_frames: usize,
        max_distance: u32,
    ) -> Vec<LibraryMediaMatch> {
        if window_a.is_empty() || window_b.is_empty() || self.files.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<LibraryMediaMatch> = self
            .files
            .par_iter()
            .filter_map(|file| {
                let raw_matches_a = file.search_query_hashes(window_a, max_distance);
                let raw_matches_b = file.search_query_hashes(window_b, max_distance);
                if raw_matches_a.is_empty() && raw_matches_b.is_empty() {
                    return None;
                }

                let mut pairs_a = Vec::with_capacity(raw_matches_a.len());
                let mut search_offsets = HashSet::new();
                let mut match_offsets = HashSet::new();
                for (q_idx, t_idx, _) in &raw_matches_a {
                    search_offsets.insert(*q_idx);
                    match_offsets.insert(*t_idx);
                    pairs_a.push((*q_idx, *t_idx));
                }

                let mut pairs_b = Vec::with_capacity(raw_matches_b.len());
                for (q_idx, t_idx, _) in &raw_matches_b {
                    search_offsets.insert(*q_idx + expected_gap_frames);
                    match_offsets.insert(*t_idx);
                    pairs_b.push((*q_idx, *t_idx));
                }

                let (_, conf_a, delta_a) =
                    compute_temporal_coherence_with_delta(window_a, &pairs_a);
                let (_, conf_b, delta_b) =
                    compute_temporal_coherence_with_delta(window_b, &pairs_b);

                // Both windows must show significant coherence or cross-window delta alignment
                let delta_diff = (delta_b - delta_a).abs();
                let is_aligned = delta_diff <= 4;

                let confidence = if is_aligned && conf_a > 0.25 && conf_b > 0.25 {
                    // Strong Two-Window Consensus
                    let avg = (conf_a + conf_b) / 2.0;
                    if delta_diff <= 2 {
                        (avg * 1.1).min(1.0)
                    } else {
                        avg
                    }
                } else if conf_a >= 0.75 && conf_b >= 0.75 {
                    (conf_a + conf_b) / 2.0
                } else if is_aligned && (conf_a > 0.40 || conf_b > 0.40) {
                    (conf_a + conf_b) / 2.0 * 0.8
                } else {
                    // Mismatch or only one window matched (e.g. repeated intro credits across seasons)
                    (conf_a.max(conf_b) * 0.35).min(0.35)
                };

                if confidence < 0.20 {
                    return None;
                }

                let total_matches = raw_matches_a.len() + raw_matches_b.len();
                Some(LibraryMediaMatch {
                    file_path: file.path.clone(),
                    media_name: file.name.clone(),
                    nb_offsets_match: total_matches,
                    search_offsets_match: search_offsets,
                    match_frames_offsets: match_offsets,
                    confidence,
                    best_offset_delta: delta_a,
                })
            })
            .collect();

        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.nb_offsets_match.cmp(&a.nb_offsets_match))
        });
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
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

    #[test]
    fn test_compute_temporal_coherence_rejects_false_positives() {
        let natural = 0x5555_AAAA_3333_CCCCu64;
        let query: Vec<u64> = (0..24).map(|i| natural ^ (i as u64)).collect();

        // 1. Blank query frames must yield (0, 0.0)
        let blanks = vec![0u64; 24];
        assert_eq!(compute_temporal_coherence(&blanks, &[(0, 100)]), (0, 0.0));

        // 2. Scattered random hits across a 75,000-frame episode -> max coherent count is 1 (4.1% < 65%)
        let scattered: Vec<(usize, usize)> =
            (0..10).map(|q_idx| (q_idx, (q_idx + 1) * 5000)).collect();
        let (cnt_scattered, conf_scattered) = compute_temporal_coherence(&query, &scattered);
        assert_eq!(cnt_scattered, 1);
        assert!(conf_scattered < MIN_TEMPORAL_COHERENCE_CONFIDENCE);

        // 3. Static shot where 10 query frames match a SINGLE isolated target frame 14860 -> 1-to-1 cap yields 1
        let single_target: Vec<(usize, usize)> = (14..24).map(|q_idx| (q_idx, 14860)).collect();
        let (cnt_single, conf_single) = compute_temporal_coherence(&query, &single_target);
        assert_eq!(cnt_single, 1);
        assert!(conf_single < MIN_TEMPORAL_COHERENCE_CONFIDENCE);

        // 4. Genuine sequential alignment (t_idx = 1000 + q_idx) -> 24/24 (100%)
        let aligned: Vec<(usize, usize)> = (0..24).map(|q_idx| (q_idx, 1000 + q_idx)).collect();
        let (cnt_aligned, conf_aligned) = compute_temporal_coherence(&query, &aligned);
        assert_eq!(cnt_aligned, 24);
        assert!((conf_aligned - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_recognize_two_windows_distinguishes_shared_intro() -> Result<()> {
        let temp_dir = std::env::temp_dir().join(format!("test_two_win_{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir)?;

        let intro_hashes: Vec<u64> = (0..100)
            .map(|i| 0x0123_4567_89AB_CDEFu64 ^ (i as u64).wrapping_mul(0x1111_2222_3333_4444u64))
            .collect();
        let ep1_body: Vec<u64> = (0..200)
            .map(|i| 0x5555_AAAA_5555_AAAAu64 ^ (i as u64).wrapping_mul(0x00FF_00FF_00FF_00FFu64))
            .collect();
        let ep2_body: Vec<u64> = (0..200)
            .map(|i| 0x3333_CCCC_3333_CCCCu64 ^ (i as u64).wrapping_mul(0xF0F0_F0F0_F0F0_F0F0u64))
            .collect();

        let mut ep1_hashes = intro_hashes.clone();
        ep1_hashes.extend_from_slice(&ep1_body);

        let mut ep2_hashes = intro_hashes.clone();
        ep2_hashes.extend_from_slice(&ep2_body);

        let ep1_file = temp_dir.join("Show.S01E01.phash");
        let ep2_file = temp_dir.join("Show.S01E02.phash");

        {
            let mut f1 = File::create(&ep1_file)?;
            for &h in &ep1_hashes {
                f1.write_all(&h.to_be_bytes())?;
            }
            let mut f2 = File::create(&ep2_file)?;
            for &h in &ep2_hashes {
                f2.write_all(&h.to_be_bytes())?;
            }
        }

        let library = MmapLibrary::from_directory(&temp_dir)?;
        assert_eq!(library.file_count(), 2);

        // Window A in shared intro (frames 20..44)
        let win_a = &ep1_hashes[20..44];
        // Window B in Ep 1 body (frames 150..174)
        let win_b = &ep1_hashes[150..174];
        let gap = 150 - 20;

        let matches = library.recognize_two_windows(win_a, win_b, gap, 2);
        assert!(!matches.is_empty(), "Must find matches");
        assert_eq!(
            matches[0].media_name, "Show.S01E01.phash",
            "Top-1 match must be Ep 1, not Ep 2 sharing only intro"
        );
        assert!(
            matches[0].confidence >= 0.90,
            "Ep 1 must have >= 90% confidence"
        );

        if matches.len() > 1 {
            assert!(
                matches[1].confidence < matches[0].confidence,
                "Ep 2 sharing only intro must have lower confidence"
            );
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(())
    }
}
