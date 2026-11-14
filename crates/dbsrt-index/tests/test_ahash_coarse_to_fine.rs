use dbsrt_core::ahash::{DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE};
use dbsrt_core::{extract_ahash, AudioExtractionOptions};
use dbsrt_index::ahash::AHashCatalog;
use dbsrt_index::coarse_to_fine::{CoarseToFineAHashIndex, FineSearchCriteria};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

// ----------------------------------------------------------------------------
// Comprehensive Coarse-to-Fine Benchmark Matrix
// ----------------------------------------------------------------------------

#[test]
#[ignore = "heavy 106M-frame benchmark; run via task bench:ahash:coarse-to-fine with --ignored --release"]
fn test_exhaustive_coarse_to_fine_benchmark() {
    let ahash_dir = if Path::new("data/__AHASH__").exists() {
        Path::new("data/__AHASH__")
    } else {
        Path::new("../../data/__AHASH__")
    };
    let snippets_dir = if Path::new("data/snippets_ahash").exists() {
        Path::new("data/snippets_ahash")
    } else {
        Path::new("../../data/snippets_ahash")
    };

    if !ahash_dir.exists() || !snippets_dir.exists() {
        println!("Skipping coarse-to-fine benchmark: missing data directories");
        return;
    }

    println!("\n====================================================================================================");
    println!(" 🚀 CHARGEMENT DU CATALOGUE DENSE aHash (86.125 fps, 106M frames)");
    println!("====================================================================================================");

    let t0 = Instant::now();
    let dense_catalog = AHashCatalog::load_from_dir(ahash_dir).expect("Load ahash");
    let dense_load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let dense_frames = dense_catalog.total_frames;
    let dense_ram_mb = (dense_frames * 8) as f64 / (1024.0 * 1024.0);

    println!(
        "✓ Catalogue dense chargé : {} médias, {} frames ({:.1} MB RAM) en {:.2} ms",
        dense_catalog.entries.len(),
        dense_frames,
        dense_ram_mb,
        dense_load_ms
    );

    // Collect query snippets
    let mut cut_paths: Vec<_> = std::fs::read_dir(snippets_dir)
        .expect("read snippets")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "mp4"))
        .collect();
    cut_paths.sort();

    println!("Extraction de {} extraits de test...", cut_paths.len());
    let mut queries = Vec::new();
    let opts = AudioExtractionOptions {
        sample_rate: DEFAULT_SAMPLE_RATE,
        frame_size: DEFAULT_FRAME_SIZE,
        hop_size: DEFAULT_HOP_SIZE,
        start_sec: 0.0,
        duration_sec: Some(15.0),
        custom_ffmpeg: None,
        audio_stream: None,
    };

    for path in &cut_paths {
        let ahash_file = match extract_ahash(path, &opts) {
            Ok(f) if !f.hashes.is_empty() => f,
            _ => continue,
        };
        let fps = ahash_file.header.audio_fps();
        let hashes = ahash_file.hashes;
        queries.push((path.clone(), hashes, fps));
    }

    println!("\n====================================================================================================");
    println!(" 📊 BENCHMARK COARSE-TO-FINE : DENSE vs K=4 vs K=8 vs K=16 vs K=31");
    println!("====================================================================================================");
    println!(
        "{:<8} | {:<12} | {:<10} | {:<10} | {:<12} | {:<12} | {:<8} | {:<9} | {:<10}",
        "Factor K",
        "FPS Coarse",
        "Frames",
        "RAM Coarse",
        "Build Time",
        "Search Time",
        "Recall",
        "Max Drift",
        "Mean SNR"
    );
    println!("----------------------------------------------------------------------------------------------------------------");

    let mut ground_truth: HashMap<std::path::PathBuf, (String, i64)> = HashMap::new();
    let factors = [1, 4, 8, 16, 31];

    for &k in &factors {
        let t_build = Instant::now();
        let c2f = CoarseToFineAHashIndex::build(&dense_catalog, k);
        let build_ms = t_build.elapsed().as_secs_f64() * 1000.0;

        let coarse_frames = c2f.coarse_catalog.total_frames;
        let coarse_ram_mb = (coarse_frames * 8) as f64 / (1024.0 * 1024.0);
        let coarse_fps = 86.125 / k as f64;

        let mut total_search_ms = 0.0;
        let mut hits = 0;
        let mut total_snr = 0.0;
        let mut max_drift_ms = 0i64;

        for (path, q_hashes, q_fps) in &queries {
            let t_search = Instant::now();
            let criteria = FineSearchCriteria {
                dense_query: q_hashes,
                dense_fps: *q_fps,
                max_distance: 14.0,
            };
            let res = c2f.search(&dense_catalog, &criteria);
            let search_ms = t_search.elapsed().as_secs_f64() * 1000.0;
            total_search_ms += search_ms;

            if let Some(m) = res {
                if m.mean_distance <= 10.0 {
                    hits += 1;
                    total_snr += m.snr_bits;

                    if k == 1 {
                        ground_truth.insert(path.clone(), (m.media_name.clone(), m.offset_ms));
                    } else if let Some((gt_media, gt_offset)) = ground_truth.get(path) {
                        if &m.media_name == gt_media {
                            let drift = (m.offset_ms - gt_offset).abs();
                            if drift > max_drift_ms {
                                max_drift_ms = drift;
                            }
                        }
                    }
                }
            }
        }

        let avg_search_ms = total_search_ms / queries.len() as f64;
        let avg_snr = if hits > 0 {
            total_snr / hits as f64
        } else {
            0.0
        };

        let drift_str = if k == 1 {
            "0 ms (ref)".to_string()
        } else if hits > 0 {
            format!("{} ms", max_drift_ms)
        } else {
            "N/A".to_string()
        };

        println!(
            "K={:<6} | {:<10.2} fps | {:<10} | {:<8.1} MB | {:<9.1} ms | {:<9.2} ms | {:>2}/10   | {:<9} | {:<7.1} b",
            k,
            coarse_fps,
            coarse_frames,
            coarse_ram_mb,
            build_ms,
            avg_search_ms,
            hits,
            drift_str,
            avg_snr
        );
    }
    println!("====================================================================================================\n");
}
