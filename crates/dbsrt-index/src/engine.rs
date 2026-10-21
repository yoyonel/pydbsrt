//! Unified PhashSearchEngine facade.
//! Transparently indexes and searches perceptual hashes with Multi-Index Hashing (MIH).
//! Unifies compiled `.dbphash` catalog files and directories of individual `.phash` files.

use crate::dbphash::MmapDbphash;
use crate::mmap::MmapLibrary;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Minimum confidence percentage threshold for matching.
pub const DEFAULT_MIN_RECOGNITION_CONFIDENCE: f64 = 80.0;

/// Unified result from a perceptual hash search/recognition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PhashEngineMatch {
    pub media_name: String,
    pub confidence_percent: f64,
    pub engine_label: &'static str,
    pub offset_delta: i64,
}

/// Unified perceptual hash search engine.
/// Always searches using Multi-Index Hashing (MIH) in RAM (< 1 ms).
#[derive(Clone, Default)]
pub struct PhashEngine {
    dbphash: Option<Arc<MmapDbphash>>,
    library: Option<Arc<MmapLibrary>>,
}

impl PhashEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder initializing with optional .dbphash catalog and loose .phash directories.
    pub fn open(dbphash_path: Option<&Path>, phash_dirs: &[PathBuf]) -> Result<Self> {
        let mut engine = Self::default();

        if let Some(p) = dbphash_path {
            if p.exists() {
                let reader = MmapDbphash::open(p)?;
                // Prewarm MIH tables
                let _ = reader.sparse_mih();
                let _ = reader.cuts_mih();
                engine.dbphash = Some(Arc::new(reader));
            }
        }

        if !phash_dirs.is_empty() {
            if let Ok(lib) = MmapLibrary::from_directories(phash_dirs) {
                lib.prewarm_mih();
                engine.library = Some(Arc::new(lib));
            }
        }

        Ok(engine)
    }

    /// Sets or updates the .dbphash catalog reader.
    pub fn set_dbphash(&mut self, reader: Option<Arc<MmapDbphash>>) {
        self.dbphash = reader;
    }

    /// Sets or updates the MmapLibrary reader.
    pub fn set_library(&mut self, library: Option<Arc<MmapLibrary>>) {
        self.library = library;
    }

    /// Returns reference to .dbphash reader if loaded.
    pub fn dbphash(&self) -> Option<&MmapDbphash> {
        self.dbphash.as_deref()
    }

    /// Returns cloned Arc reference to .dbphash reader if loaded.
    pub fn dbphash_arc(&self) -> Option<Arc<MmapDbphash>> {
        self.dbphash.clone()
    }

    /// Returns reference to MmapLibrary if loaded.
    pub fn library(&self) -> Option<&MmapLibrary> {
        self.library.as_deref()
    }

    /// Returns the number of distinct indexed media items.
    pub fn indexed_media_count(&self) -> usize {
        let mut count = 0;
        if let Some(ref dbphash) = self.dbphash {
            count += dbphash.media_entries().len();
        }
        if let Some(ref lib) = self.library {
            count += lib.file_count();
        }
        count
    }

    /// Total number of indexed frame hashes across both engines.
    pub fn total_frames(&self) -> usize {
        let mut count = 0;
        if let Some(ref dbphash) = self.dbphash {
            count += dbphash.cuts_entries().len() + dbphash.sparse_entries().len();
        }
        if let Some(ref lib) = self.library {
            count += lib.total_frames();
        }
        count
    }

    /// Recognizes a query sample against the indexed catalog using MIH.
    /// Priority 1: High-density compiled .dbphash (Tier 1 MIH, < 1ms)
    /// Priority 2: Loose .phash files via MmapLibrary (Tier 2 MIH, < 5ms)
    pub fn recognize(&self, query_sample: &[u64], max_distance: u32) -> Option<PhashEngineMatch> {
        let max_dist = max_distance.min(8);

        // 1. Check compiled .dbphash with MIH
        if let Some(ref dbphash) = self.dbphash {
            let matches = dbphash.recognize(query_sample, max_dist);
            if let Some(m) = matches.into_iter().next() {
                if m.confidence >= DEFAULT_MIN_RECOGNITION_CONFIDENCE {
                    return Some(PhashEngineMatch {
                        media_name: m.clean_title,
                        confidence_percent: m.confidence,
                        engine_label: "Tier 1 (RAM .dbphash MIH under 10ms)",
                        offset_delta: m.estimated_shift_ms,
                    });
                }
            }
        }

        // 2. Check loose .phash files with accelerated MIH
        if let Some(ref lib) = self.library {
            let matches = lib.recognize(query_sample, max_dist);
            if let Some(m) = matches.into_iter().next() {
                let conf_pct = m.confidence * 100.0;
                if conf_pct >= DEFAULT_MIN_RECOGNITION_CONFIDENCE {
                    return Some(PhashEngineMatch {
                        media_name: m.media_name,
                        confidence_percent: conf_pct,
                        engine_label: "Tier 2 (MmapLibrary MIH Zero-DB)",
                        offset_delta: m.best_offset_delta,
                    });
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_phash_engine_with_loose_files() -> Result<()> {
        let temp_dir = std::env::temp_dir().join(format!("test_engine_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join("Sample.Movie.2024.phash");

        let hashes: Vec<u64> = (0..150)
            .map(|i| 0xABCD_EF01_2345_6789 ^ (i * 0x0102_0304))
            .collect();
        {
            let mut f = File::create(&test_file)?;
            for &h in &hashes {
                f.write_all(&h.to_be_bytes())?;
            }
        }

        let engine = PhashEngine::open(None, std::slice::from_ref(&temp_dir))?;
        assert_eq!(engine.indexed_media_count(), 1);
        assert_eq!(engine.total_frames(), 150);

        let query_sample: Vec<u64> = hashes[20..60].to_vec();
        let matched = engine.recognize(&query_sample, 2);
        assert!(matched.is_some(), "Engine must recognize sample via MIH");
        let m = matched.unwrap();
        assert_eq!(m.media_name, "Sample.Movie.2024.phash");
        assert!(m.confidence_percent >= 90.0);
        assert_eq!(m.engine_label, "Tier 2 (MmapLibrary MIH Zero-DB)");

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(())
    }
}
