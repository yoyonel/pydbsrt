//! Benchmark and validation suite on real NAS video snippets pHash dataset.
//!
//! Loads all `.phash` files in `data/snippets/`, builds MultiIndexHash64 (MIH),
//! runs strict mathematical parity checks (MIH vs Linear Scan across CPU cores),
//! measures Recall@1 under noise (d=0, d=2, d=4), and benchmarks Anchor Temporal Tracking.

use dbsrt_core::{hamming_distance, read_phash_file};
use dbsrt_index::mih::MultiIndexHash64;
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

struct MediaEntry {
    name: String,
    start_frame: usize,
    frame_count: usize,
}

fn main() {
    let snippets_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/snippets"));

    println!("================================================================================");
    println!(" 🚀 EMPIRICAL BENCHMARK & PARITY TEST: NAS SNIPPETS DATASET");
    println!("================================================================================");
    println!("Snippets directory: {:?}", snippets_dir);

    if !snippets_dir.exists() {
        eprintln!("❌ Error: Snippets directory not found: {:?}", snippets_dir);
        std::process::exit(1);
    }

    // 1. Discover and load all .phash files
    let mut phash_files: Vec<PathBuf> = std::fs::read_dir(&snippets_dir)
        .expect("Failed to read snippets directory")
        .filter_map(|e| e.ok().map(|de| de.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "phash"))
        .collect();

    phash_files.sort();
    let total_medias = phash_files.len();
    if total_medias == 0 {
        eprintln!("❌ No .phash files found in {:?}", snippets_dir);
        std::process::exit(1);
    }

    println!("📂 Discovered {} .phash media files.", total_medias);
    println!("⏳ Loading all frame pHashes into contiguous memory...");

    let mut dataset: Vec<u64> = Vec::new();
    let mut medias: Vec<MediaEntry> = Vec::with_capacity(total_medias);

    for path in &phash_files {
        if let Ok(hashes) = read_phash_file(path) {
            let start = dataset.len();
            let count = hashes.len();
            if count >= 25 {
                dataset.extend(hashes);
                let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                medias.push(MediaEntry {
                    name: stem,
                    start_frame: start,
                    frame_count: count,
                });
            }
        }
    }

    let total_frames = dataset.len();
    let valid_medias = medias.len();
    let raw_mb = (total_frames * 8) as f64 / 1_048_576.0;

    println!(
        "✅ Loaded {} valid medias ({} frames total, {:.2} MB raw pHash in RAM).",
        valid_medias, total_frames, raw_mb
    );

    // 2. Build MIH Index
    println!("\n⚡ Building Multi-Index Hashing (CSR 4x16-bit)...");
    let t_build_start = Instant::now();
    let mih = MultiIndexHash64::build(&dataset);
    let build_ms = t_build_start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "✅ MIH Index built in {:.2} ms ({:.2} s) -> {:.1} Million frames/s",
        build_ms,
        build_ms / 1000.0,
        (total_frames as f64 / 1_000_000.0) / (build_ms / 1000.0)
    );

    // 3. Mathematical Parity Check: MIH vs Parallel Linear Scan
    println!("\n================================================================================");
    println!(" 🔍 SUITE 1 : STRICT MATHEMATICAL PARITY (0 FALSE NEGATIVES)");
    println!("================================================================================");

    let sample_count = valid_medias.min(100);
    let max_dist = 6u32;
    println!(
        "Testing {} sample media queries with max_distance = {} against {} frames...",
        sample_count, max_dist, total_frames
    );

    let mut total_mih_matches = 0usize;
    let mut total_lin_matches = 0usize;
    let mut parity_passed = true;

    for i in 0..sample_count {
        let entry = &medias[i * (valid_medias / sample_count)];
        let query_hash = dataset[entry.start_frame + entry.frame_count / 2];

        // Query MIH
        let mih_res = mih.search(query_hash, max_dist, &dataset);
        // Query Linear
        let lin_count = dataset
            .par_iter()
            .filter(|&&h| hamming_distance(query_hash, h) <= max_dist)
            .count();

        if mih_res.len() != lin_count {
            eprintln!(
                "❌ PARITY FAILURE on media '{}': MIH found {}, Linear found {}",
                entry.name,
                mih_res.len(),
                lin_count
            );
            parity_passed = false;
        }

        total_mih_matches += mih_res.len();
        total_lin_matches += lin_count;
    }

    assert!(parity_passed, "Parity failure detected!");
    assert_eq!(total_mih_matches, total_lin_matches);
    println!(
        "✅ 100% PARITY CONFIRMED! Exactly {} matches found across both methods (Zero False Negatives).",
        total_mih_matches
    );

    // 4. Recall@1 Benchmark under Noise
    println!("\n================================================================================");
    println!(" 🎯 SUITE 2 : RECALL@1 & ANCHOR TRACKING UNDER ENCODING NOISE");
    println!("================================================================================");

    for noise_bits in [0u32, 1, 2, 4] {
        let mut correct_matches = 0usize;
        let mut evaluated = 0usize;
        let t_recall_start = Instant::now();

        for entry in medias.iter().take(sample_count) {
            let offset = entry.start_frame + 5;
            let mut query_window: Vec<u64> = dataset[offset..(offset + 25)].to_vec();

            // Inject noise
            for (f_idx, q) in query_window.iter_mut().enumerate() {
                for b in 0..noise_bits {
                    let bit = ((f_idx as u32 * 7 + b * 13) % 64) as u8;
                    *q ^= 1u64 << bit;
                }
            }

            // Anchor Tracking: Search frame 0 in MIH, then verify 24 sequential frames O(1)
            let anchor_candidates = mih.search(query_window[0], max_dist, &dataset);
            let mut best_match_idx: Option<usize> = None;
            let mut min_cum_dist = u32::MAX;

            for cand in anchor_candidates {
                let cand_offset = cand.index;
                if cand_offset + 25 <= dataset.len() {
                    let mut cum_dist = 0u32;
                    let mut valid = true;
                    for f in 0..25 {
                        let d = hamming_distance(query_window[f], dataset[cand_offset + f]);
                        if d > max_dist {
                            valid = false;
                            break;
                        }
                        cum_dist += d;
                    }
                    if valid && cum_dist < min_cum_dist {
                        min_cum_dist = cum_dist;
                        best_match_idx = Some(cand_offset);
                    }
                }
            }

            if let Some(matched_offset) = best_match_idx {
                // Check if matched_offset belongs to the true media entry
                if matched_offset >= entry.start_frame
                    && matched_offset < entry.start_frame + entry.frame_count
                {
                    correct_matches += 1;
                }
            }
            evaluated += 1;
        }

        let recall_pct = (correct_matches as f64 / evaluated as f64) * 100.0;
        let elapsed_ms = t_recall_start.elapsed().as_secs_f64() * 1000.0;
        let latency_per_req_ms = elapsed_ms / evaluated as f64;

        println!(
            "  • Noise = {} bits: Recall@1 = {:5.1}% ({}/{} correct) | Avg Latency = {:.2} ms/query",
            noise_bits, recall_pct, correct_matches, evaluated, latency_per_req_ms
        );
    }

    // 5. Latency Comparison: 25-Frame Query vs Linear Scan Baseline
    println!("\n================================================================================");
    println!(" ⚡ SUITE 3 : LATENCY & THROUGHPUT BENCHMARK (REAL DATASET)");
    println!("================================================================================");

    let test_window: Vec<u64> = dataset[0..25].to_vec();

    // 1-frame latency
    let t_single = Instant::now();
    let _ = mih.search(test_window[0], max_dist, &dataset);
    let single_us = t_single.elapsed().as_secs_f64() * 1_000_000.0;

    // Anchor tracking latency (1s recognition window)
    let t_anchor = Instant::now();
    let cands = mih.search(test_window[0], max_dist, &dataset);
    let mut verified = 0;
    for c in cands {
        let off = c.index;
        if off + 25 <= dataset.len() {
            let mut ok = true;
            for f in 1..25 {
                if hamming_distance(test_window[f], dataset[off + f]) > max_dist {
                    ok = false;
                    break;
                }
            }
            if ok {
                verified += 1;
            }
        }
    }
    let anchor_ms = t_anchor.elapsed().as_secs_f64() * 1000.0;

    // Linear scan baseline
    let t_lin = Instant::now();
    let lin_matches: usize = test_window
        .par_iter()
        .map(|&q| {
            dataset
                .par_iter()
                .filter(|&&h| hamming_distance(q, h) <= max_dist)
                .count()
        })
        .sum();
    let lin_ms = t_lin.elapsed().as_secs_f64() * 1000.0;

    println!("  🎯 1-Frame Query Latency:        {:.2} µs", single_us);
    println!(
        "  🎬 Anchor Temporal Tracking:     {:.2} ms (verified sequences: {})",
        anchor_ms, verified
    );
    println!(
        "  🐢 Linear Scan 25-Frames:        {:.2} ms ({:.2} s, total matches: {})",
        lin_ms,
        lin_ms / 1000.0,
        lin_matches
    );
    println!(
        "  🏆 MIH Acceleration Factor:      {:.1}x FASTER than Linear Scan",
        lin_ms / anchor_ms.max(0.001)
    );

    println!("\n🎉 ALL SNIPPETS EMPIRICAL TESTS PASSED SUCCESSFULLY!");
    println!("================================================================================");
}
