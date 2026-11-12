//! Empirical validation test of the mathematical window scaling theory.
//! Evaluates both cross-media extreme distance distribution and noisy signal recovery.

use dbsrt_core::ahash::AhashFile;
use dbsrt_core::match_audio_sequence;
use rayon::prelude::*;
use std::path::PathBuf;

struct XorShift64(u64);
impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0xdead_beef_1234 } else { seed })
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn gen_range(&mut self, max: usize) -> usize {
        if max == 0 {
            0
        } else {
            (self.next_u64() as usize) % max
        }
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() as f64) / (u64::MAX as f64)
    }
}

/// Applies realistic bit error rate noise (e.g. 10% BER) simulating lossy compression.
fn apply_ber_noise(hashes: &[u64], ber: f64, rng: &mut XorShift64) -> Vec<u64> {
    let mut noisy = hashes.to_vec();
    for hash in &mut noisy {
        let mut noise_mask = 0u64;
        for bit in 0..64 {
            if rng.next_f64() < ber {
                noise_mask |= 1u64 << bit;
            }
        }
        *hash ^= noise_mask;
    }
    noisy
}

#[test]
#[ignore = "Long-running empirical validation of scaling theory (run with cargo test --test test_window_scaling_theory -- --ignored)"]
fn test_empirical_window_scaling_validation() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ahash_dir = manifest_dir.join("../../data/__AHASH__");

    let mut test_files = Vec::new();
    if ahash_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&ahash_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("ahash") {
                    test_files.push(p);
                }
            }
        }
    }

    if test_files.len() < 10 {
        eprintln!("Not enough .ahash files found, skipping.");
        return;
    }

    test_files.sort();

    println!("Loading all .ahash files in parallel with Rayon...");
    let loaded: Vec<AhashFile> = test_files
        .par_iter()
        .filter_map(|path| AhashFile::read_from_path(path).ok())
        .filter(|file| file.hashes.len() > 10_000)
        .collect();

    assert!(loaded.len() >= 50, "Need at least 50 valid files");
    let total_frames: usize = loaded.iter().map(|f| f.hashes.len()).sum();
    let total_hours = total_frames as f64 / loaded[0].header.audio_fps() / 3600.0;
    println!(
        "Loaded {} reference files ({} frames, {:.2} hours) into RAM",
        loaded.len(),
        total_frames,
        total_hours
    );

    let windows = [3.00, 1.00, 0.50, 0.30, 0.20, 0.10, 0.05, 0.02, 0.01];
    let trials_per_window = 10;
    let mut rng = XorShift64::new(0x2026_1112_abcd);

    println!("\n=================================================================================================================================");
    println!("  RÉSULTATS DE L'ÉVALUATION EMPIRIQUE 1:N SUR CATALOGUE COMPLET (583 MÉDIAS, 106M FRAMES, BRUIT COMPRESSION BER=10%)");
    println!("=================================================================================================================================");
    println!(" Fenêtre (W) | Frames | Top-1 Catalogue | Match Exact | Erreur Méd | Erreur p95 | Dist Vraie | Bruit Extrême |  Marge SNR ");
    println!("-------------+--------+-----------------+-------------+------------+------------+------------+---------------+------------");

    for &w in &windows {
        let mut top1_matches = 0;
        let mut exact_matches = 0;
        let mut errors_ms = Vec::with_capacity(trials_per_window);
        let mut true_distances = Vec::with_capacity(trials_per_window);
        let mut min_cross_distances = Vec::with_capacity(trials_per_window);

        for _ in 0..trials_per_window {
            let true_idx = rng.gen_range(loaded.len());
            let file_a = &loaded[true_idx];
            let fps = file_a.header.audio_fps();
            let frame_count = (w * fps).round().max(1.0) as usize;

            if frame_count >= file_a.hashes.len() {
                continue;
            }

            let max_start = file_a.hashes.len() - frame_count;
            let start = rng.gen_range(max_start);
            let ground_truth_ms = (start as f64 / fps) * 1000.0;
            let clean_query = &file_a.hashes[start..start + frame_count];
            let noisy_query = apply_ber_noise(clean_query, 0.10, &mut rng);

            // Recherche 1:N parallèle avec Rayon à travers TOUT le catalogue (583 fichiers)
            let match_results: Vec<(usize, Option<dbsrt_core::AudioMatchResult>)> = loaded
                .par_iter()
                .enumerate()
                .map(|(c_idx, cand_file)| {
                    let m = match_audio_sequence(
                        &noisy_query,
                        &cand_file.hashes,
                        cand_file.header.audio_fps(),
                        32.0,
                    );
                    (c_idx, m)
                })
                .collect();

            let mut best_file_idx = None;
            let mut best_distance = f64::INFINITY;
            let mut best_offset_ms = 0.0;
            let mut true_dist_sample = 0.0;
            let mut min_cross_noise = f64::INFINITY;

            for (c_idx, m_opt) in match_results {
                if let Some(m) = m_opt {
                    if c_idx == true_idx {
                        true_dist_sample = m.mean_distance;
                    } else if m.mean_distance < min_cross_noise {
                        min_cross_noise = m.mean_distance;
                    }
                    if m.mean_distance < best_distance {
                        best_distance = m.mean_distance;
                        best_file_idx = Some(c_idx);
                        best_offset_ms = m.offset_sec * 1000.0;
                    }
                }
            }

            true_distances.push(true_dist_sample);
            min_cross_distances.push(min_cross_noise);

            if best_file_idx == Some(true_idx) {
                top1_matches += 1;
                let err_ms = (best_offset_ms - ground_truth_ms).abs();
                errors_ms.push(err_ms);
                if err_ms <= 25.0 {
                    exact_matches += 1;
                }
            } else {
                errors_ms.push(9999.0);
            }
        }

        errors_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_err = if !errors_ms.is_empty() {
            errors_ms[errors_ms.len() / 2]
        } else {
            0.0
        };
        let p95_err = if !errors_ms.is_empty() {
            errors_ms[(errors_ms.len() as f64 * 0.95) as usize]
        } else {
            0.0
        };
        let top1_rate = (top1_matches as f64 / trials_per_window as f64) * 100.0;
        let exact_rate = (exact_matches as f64 / trials_per_window as f64) * 100.0;
        let avg_true_dist = if !true_distances.is_empty() {
            true_distances.iter().sum::<f64>() / true_distances.len() as f64
        } else {
            0.0
        };
        let min_cross_dist = if !min_cross_distances.is_empty() {
            min_cross_distances
                .iter()
                .cloned()
                .fold(f64::INFINITY, f64::min)
        } else {
            0.0
        };
        let margin_snr = min_cross_dist - avg_true_dist;

        let fps = loaded[0].header.audio_fps();
        let frame_count = (w * fps).round().max(1.0) as usize;

        println!(
            " {:>10.2}s | {:>6} | {:>14.1}% | {:>10.1}% | {:>9.2}ms | {:>9.2}ms | {:>9.2}b | {:>12.2}b | {:>+10.2}b",
            w, frame_count, top1_rate, exact_rate, median_err, p95_err, avg_true_dist, min_cross_dist, margin_snr
        );
    }
    println!("=================================================================================================================================\n");
}
