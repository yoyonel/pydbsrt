//! Visual pHash recognition pipeline and index engine routing.

use crate::constants::MIN_PHASH_RECOGNITION_CONFIDENCE_PERCENT;
use crate::services::phash::{extract_media_query_sample, ExistingPhashData};
use crate::state::DbConnectionConfig;
use dbsrt_index::{MmapDbphash, MmapLibrary};
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

    if dbsrt_core::is_blank_sequence(&query_sample) {
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
                            "Tier 1 (RAM .dbphash MIH)".to_string(),
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
                        "Tier 1 (RAM .dbphash MIH)".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_identify_media_wrappers_and_missing_path() {
        let dummy_path = PathBuf::from("non_existent_video_12345.mp4");
        let phash_dir = PathBuf::from("data/__PHASH__");

        let res = identify_media_by_phash(&dummy_path, &phash_dir, None, 1.0, 6);
        assert_eq!(res.0, None);

        let res_multi = identify_media_by_phash_multi(
            &dummy_path,
            std::slice::from_ref(&phash_dir),
            None,
            None,
            1.0,
            6,
        );
        assert_eq!(res_multi.0, None);

        let res_reader = identify_media_by_phash_multi_with_reader(
            &dummy_path,
            std::slice::from_ref(&phash_dir),
            None,
            None,
            1.0,
            6,
        );
        assert_eq!(res_reader.0, None);
    }

    fn build_synthetic_recognition_fixture(
        dir: &Path,
    ) -> (PathBuf, PathBuf, MmapDbphash, Vec<PathBuf>) {
        use dbsrt_core::metadata::MediaMetadata;
        use dbsrt_index::DbphashBuilder;

        let make_hashes = |seed: u64, n: usize| -> Vec<u64> {
            let mut x = seed;
            (0..n)
                .map(|_| {
                    x = x
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    x ^ (x >> 29)
                })
                .collect()
        };

        let hashes_bbb = make_hashes(0x1122_3344_5566_7788, 75);
        let hashes_other = make_hashes(0x9988_7766_5544_3322, 75);
        let hashes_blank = vec![0u64; 30];

        let target_phash = dir.join("big_buck_bunny_trailer_480p.25.000fps.phash");
        let other_phash = dir.join("other_movie_sample.24.000fps.phash");
        let blank_phash = dir.join("black_frames.25.000fps.phash");
        dbsrt_core::write_phash_file(&target_phash, &hashes_bbb).expect("write target phash");
        dbsrt_core::write_phash_file(&other_phash, &hashes_other).expect("write other phash");
        dbsrt_core::write_phash_file(&blank_phash, &hashes_blank).expect("write blank phash");

        let mut builder = DbphashBuilder::new();
        let to_sparse = |hashes: &[u64], fps: f64| -> Vec<(u64, u32)> {
            hashes
                .iter()
                .enumerate()
                .map(|(idx, &h)| (h, ((idx as f64 / fps) * 1000.0) as u32))
                .collect()
        };
        let meta_bbb = MediaMetadata::parse("Big.Buck.Bunny.Trailer.2008.480p.mp4");
        let sparse_bbb = to_sparse(&hashes_bbb, 25.0);
        let id_bbb = builder.add_media(&meta_bbb, 3_000, 25.0, &sparse_bbb[..10], &sparse_bbb);
        builder.set_frame_count(id_bbb, hashes_bbb.len() as u64);

        let meta_other = MediaMetadata::parse("Other.Movie.Sample.2021.1080p.mkv");
        let sparse_other = to_sparse(&hashes_other, 24.0);
        let id_other =
            builder.add_media(&meta_other, 3_125, 24.0, &sparse_other[..10], &sparse_other);
        builder.set_frame_count(id_other, hashes_other.len() as u64);

        let dbphash_path = dir.join("catalog.dbphash");
        builder
            .write_to_file(&dbphash_path)
            .expect("write synthetic catalog.dbphash");
        let reader = MmapDbphash::open(&dbphash_path).expect("open synthetic catalog.dbphash");
        (target_phash, blank_phash, reader, vec![dir.to_path_buf()])
    }

    #[test]
    fn test_identify_media_all_engine_modes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (target_phash, blank_phash, reader, phash_dirs) =
            build_synthetic_recognition_fixture(tmp.path());

        // Test blank frames filter
        let (matched_blank, _, _) = identify_media_by_phash_multi_routed(
            &blank_phash,
            &phash_dirs,
            Some(&reader),
            None,
            1.0,
            12,
            None,
        );
        assert_eq!(matched_blank, None);

        // 1. Force linear mode (Tier 1 Flat Linear O(N))
        let (matched_lin, _, _) = identify_media_by_phash_multi_routed(
            &target_phash,
            &phash_dirs,
            Some(&reader),
            None,
            1.0,
            12,
            Some("linear"),
        );
        assert!(matched_lin.is_some());

        // 2. Force mih mode (Tier 1 RAM .dbphash MIH)
        let (matched_mih, _, _) = identify_media_by_phash_multi_routed(
            &target_phash,
            &phash_dirs,
            Some(&reader),
            None,
            1.0,
            12,
            Some("mih"),
        );
        assert!(matched_mih.is_some());

        // 3. Force postgres mode without config
        let (matched_pg_none, _, _) = identify_media_by_phash_multi_routed(
            &target_phash,
            &phash_dirs,
            Some(&reader),
            None,
            1.0,
            12,
            Some("pg_bktree"),
        );
        assert_eq!(matched_pg_none, None);

        // 4. Force postgres mode with dummy/invalid config
        let dummy_cfg =
            DbConnectionConfig::new("127.0.0.1", 5433, "invalid", "invalid", "invalid_db");
        let (matched_pg_cfg, _, _) = identify_media_by_phash_multi_routed(
            &target_phash,
            &phash_dirs,
            Some(&reader),
            Some(&dummy_cfg),
            1.0,
            12,
            Some("postgres"),
        );
        assert_eq!(matched_pg_cfg, None);

        // 5. Auto mode without dbphash reader (forces Tier 2 MmapLibrary from synthetic loose files)
        let (matched_tier2, _, _) = identify_media_by_phash_multi_routed(
            &target_phash,
            &phash_dirs,
            None,
            None,
            1.0,
            12,
            Some("auto"),
        );
        assert!(matched_tier2.is_some());
    }
}
