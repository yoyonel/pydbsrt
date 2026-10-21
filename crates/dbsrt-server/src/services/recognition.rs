//! Visual pHash recognition pipeline and index engine routing.

use crate::constants::MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT;
use crate::services::phash::{extract_media_query_sample, ExistingPhashData};
use crate::state::DbConnectionConfig;
use dbsrt_index::{MmapDbphash, MmapLibrary};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub type PhashIdentificationResult = (Option<(String, f64)>, f64, Option<ExistingPhashData>);

pub type RoutedPhashResult = (
    Option<(String, f64, String)>,
    f64,
    Option<ExistingPhashData>,
);

/// Identifies a target media file against `MmapLibrary` in `phash_dir`.
pub fn identify_media_by_phash(
    media_path: &Path,
    phash_dir: &Path,
    db_config: Option<&DbConnectionConfig>,
    nb_seconds: f64,
    search_distance: u32,
) -> PhashIdentificationResult {
    identify_media_by_phash_multi(
        media_path,
        &[phash_dir.to_path_buf()],
        None,
        db_config,
        nb_seconds,
        search_distance,
    )
}

/// Identifies a target media file across multiple `.phash` directories, an optional `.dbphash`
/// catalog, and an optional PostgreSQL BK-tree (`DbConnectionConfig`).
pub fn identify_media_by_phash_multi(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    dbphash_path: Option<&Path>,
    db_config: Option<&DbConnectionConfig>,
    nb_seconds: f64,
    search_distance: u32,
) -> PhashIdentificationResult {
    let reader = dbphash_path.and_then(|p| MmapDbphash::open(p).ok());
    identify_media_by_phash_multi_with_reader(
        media_path,
        phash_dirs,
        reader.as_ref(),
        db_config,
        nb_seconds,
        search_distance,
    )
}

pub fn identify_media_by_phash_multi_with_reader(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    dbphash_reader: Option<&MmapDbphash>,
    db_config: Option<&DbConnectionConfig>,
    nb_seconds: f64,
    search_distance: u32,
) -> PhashIdentificationResult {
    let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_routed(
        media_path,
        phash_dirs,
        dbphash_reader,
        db_config,
        nb_seconds,
        search_distance,
        None,
    );
    let matched_pair = matched.map(|(name, conf, _tier)| (name, conf));
    (matched_pair, target_fps, existing_phash)
}

/// Identifies a target media file with explicit index engine routing.
pub fn identify_media_by_phash_multi_routed(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    dbphash_reader: Option<&MmapDbphash>,
    db_config: Option<&DbConnectionConfig>,
    nb_seconds: f64,
    search_distance: u32,
    engine: Option<&str>,
) -> RoutedPhashResult {
    let (target_fps, query_sample, existing_phash) =
        extract_media_query_sample(media_path, phash_dirs, nb_seconds);

    let non_blank_unique: HashSet<u64> = query_sample
        .iter()
        .copied()
        .filter(|&h| !dbsrt_core::is_blank_phash(h))
        .collect();

    if non_blank_unique.is_empty() || (query_sample.len() >= 10 && non_blank_unique.len() < 2) {
        return (None, target_fps, existing_phash);
    }

    let rec_distance = search_distance.min(8);
    let chosen_engine = engine.unwrap_or("auto");

    // Mode 1: Force PostgreSQL 17 SP-GiST BK-Tree
    if chosen_engine == "pg_bktree" || chosen_engine == "postgres" {
        if let Some(cfg) = db_config {
            if let Ok(Some((rec, conf))) =
                cfg.with_client(|client| client.recognize_sample(&query_sample, rec_distance))
            {
                let conf_pct = conf * 100.0;
                if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    return (
                        Some((
                            rec.name,
                            conf_pct,
                            "Tier 3 (PostgreSQL 17 SP-GiST BK-Tree)".to_string(),
                        )),
                        target_fps,
                        existing_phash,
                    );
                }
            }
        }
        return (None, target_fps, existing_phash);
    }

    // Mode 2: Force Flat Linear Scan O(N)
    if chosen_engine == "linear" {
        if let Some(reader) = dbphash_reader {
            let matches = reader.recognize_linear(&query_sample, rec_distance);
            if let Some(m) = matches.into_iter().next() {
                if m.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    return (
                        Some((
                            m.clean_title,
                            m.confidence,
                            "Tier 1 (RAM .dbphash Flat Linear O(N))".to_string(),
                        )),
                        target_fps,
                        existing_phash,
                    );
                }
            }
        }
        return (None, target_fps, existing_phash);
    }

    // Mode 3: Force MIH (Tier 1 exclusive)
    if chosen_engine == "mih" {
        if let Some(reader) = dbphash_reader {
            let matches = reader.recognize(&query_sample, rec_distance);
            if let Some(m) = matches.into_iter().next() {
                if m.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                    return (
                        Some((
                            m.clean_title,
                            m.confidence,
                            "Tier 1 (RAM .dbphash MIH under 10ms)".to_string(),
                        )),
                        target_fps,
                        existing_phash,
                    );
                }
            }
        }
        return (None, target_fps, existing_phash);
    }

    // Mode 4: Auto Cascade (Tier 1 MIH -> Tier 2 MmapLibrary MIH -> Tier 3 PostgreSQL)
    if let Some(reader) = dbphash_reader {
        let matches = reader.recognize(&query_sample, rec_distance);
        if let Some(m) = matches.into_iter().next() {
            if m.confidence >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return (
                    Some((
                        m.clean_title,
                        m.confidence,
                        "Tier 1 (RAM .dbphash MIH under 10ms)".to_string(),
                    )),
                    target_fps,
                    existing_phash,
                );
            }
        }
    }

    // 2. Check MmapLibrary across all configured phash_dirs (Tier 2: Accelerated In-Memory Zero-DB via MIH)
    if let Ok(lib) = MmapLibrary::from_directories(phash_dirs) {
        if let Some(m) = lib
            .recognize(&query_sample, rec_distance)
            .into_iter()
            .next()
        {
            let conf_pct = m.confidence * 100.0;
            if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return (
                    Some((
                        m.media_name,
                        conf_pct,
                        "Tier 2 (MmapLibrary Zero-DB Files)".to_string(),
                    )),
                    target_fps,
                    existing_phash,
                );
            }
        }
    }

    // 3. Check PostgreSQL BK-Tree if configured (Tier 3: Persistent Store Fallback)
    if let Some(cfg) = db_config {
        if let Ok(Some((rec, conf))) =
            cfg.with_client(|client| client.recognize_sample(&query_sample, rec_distance))
        {
            let conf_pct = conf * 100.0;
            if conf_pct >= MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT {
                return (
                    Some((
                        rec.name,
                        conf_pct,
                        "Tier 3 (PostgreSQL 17 SP-GiST BK-Tree)".to_string(),
                    )),
                    target_fps,
                    existing_phash,
                );
            }
        }
    }

    (None, target_fps, existing_phash)
}
