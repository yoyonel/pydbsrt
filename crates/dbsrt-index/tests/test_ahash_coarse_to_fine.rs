use dbsrt_core::ahash::{DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE};
use dbsrt_core::{extract_ahash, AudioExtractionOptions};
use dbsrt_index::ahash::{
    AHashCatalog, AHashCatalogEntry, AHashCatalogMatch, AHashMih32Index, AHashSearchCriteria,
    MILLIS_PER_SEC, MIN_QUERY_FRAMES,
};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

const DEFAULT_FINE_WINDOW_SECONDS: f64 = 2.0;

/// Hierarchical Coarse-to-Fine index for acoustic fingerprints.
pub struct CoarseToFineAHashIndex {
    pub factor_k: usize,
    pub coarse_catalog: AHashCatalog,
    pub coarse_mih32: AHashMih32Index,
}

impl CoarseToFineAHashIndex {
    /// Builds a coarse-to-fine index from a dense catalog by sub-sampling every K-th frame.
    pub fn build(dense_catalog: &AHashCatalog, factor_k: usize) -> Self {
        assert!(factor_k >= 1, "Downsample factor must be >= 1");

        let coarse_entries: Vec<AHashCatalogEntry> = dense_catalog
            .entries
            .par_iter()
            .map(|entry| {
                let coarse_hashes: Vec<u64> =
                    entry.hashes.iter().step_by(factor_k).copied().collect();
                let coarse_fps = entry.audio_fps / factor_k as f64;
                let frame_count = coarse_hashes.len();
                AHashCatalogEntry {
                    media_name: entry.media_name.clone(),
                    path: entry.path.clone(),
                    audio_fps: coarse_fps,
                    frame_count,
                    hashes: Arc::new(coarse_hashes),
                }
            })
            .collect();

        let coarse_catalog = AHashCatalog::new(coarse_entries);
        let coarse_mih32 = AHashMih32Index::build(&coarse_catalog.flat_hashes);

        Self {
            factor_k,
            coarse_catalog,
            coarse_mih32,
        }
    }

    /// Performs two-tier coarse-to-fine search:
    /// 1. Coarse Stage: Identifies candidate media and rough temporal offset using sub-sampled frames.
    /// 2. Fine Stage: Evaluates dense 31.25 fps cross-correlation strictly within a narrow window around coarse offset.
    pub fn search(
        &self,
        dense_catalog: &AHashCatalog,
        dense_query: &[u64],
        dense_fps: f64,
        use_mih: bool,
    ) -> Option<AHashCatalogMatch> {
        let q_len = dense_query.len();
        if q_len < MIN_QUERY_FRAMES {
            return None;
        }

        // --- Tier 1: Coarse Stage ---
        let coarse_query: Vec<u64> = dense_query.iter().step_by(self.factor_k).copied().collect();
        let coarse_fps = dense_fps / self.factor_k as f64;
        let coarse_criteria = AHashSearchCriteria::new(&coarse_query, coarse_fps);

        let coarse_match = if use_mih {
            let res = self.search_coarse_mih32(&coarse_criteria);
            res.into_iter().next()
        } else {
            let res = self.coarse_catalog.search_linear(&coarse_criteria);
            res.into_iter().next()
        }?;

        // --- Tier 2: Fine Stage ---
        let dense_entry = dense_catalog
            .entries
            .iter()
            .find(|e| e.media_name == coarse_match.media_name)?;

        let approx_dense_offset = coarse_match.ref_frame_offset * self.factor_k;
        let window_radius =
            ((DEFAULT_FINE_WINDOW_SECONDS * dense_fps) as usize).max(self.factor_k * 2);

        let min_s = approx_dense_offset.saturating_sub(window_radius);
        let max_s = (approx_dense_offset + window_radius)
            .min(dense_entry.hashes.len().saturating_sub(q_len));

        if min_s > max_s || dense_entry.hashes.len() < q_len {
            return None;
        }

        let mut best_shift = min_s;
        let mut best_dist = u64::MAX;

        for s in min_s..=max_s {
            let ref_slice = &dense_entry.hashes[s..s + q_len];
            let mut dist = 0u64;
            for (&q, &r) in dense_query.iter().zip(ref_slice) {
                dist += (q ^ r).count_ones() as u64;
            }
            if dist < best_dist {
                best_dist = dist;
                best_shift = s;
            }
        }

        let mean_dist = best_dist as f64 / q_len as f64;
        let score = (1.0 - (mean_dist / 64.0)).clamp(0.0, 1.0);
        let snr_bits = (32.0 - mean_dist).max(0.0);
        let offset_sec = best_shift as f64 / dense_fps;

        Some(AHashCatalogMatch {
            media_name: dense_entry.media_name.clone(),
            file_path: dense_entry.path.to_string_lossy().to_string(),
            ref_frame_offset: best_shift,
            offset_sec,
            offset_ms: (offset_sec * MILLIS_PER_SEC).round() as i64,
            mean_distance: mean_dist,
            score,
            snr_bits,
            audio_fps: dense_fps,
        })
    }

    /// Fast coarse probe using MIH-32 on downsampled tables.
    fn search_coarse_mih32(&self, criteria: &AHashSearchCriteria) -> Vec<AHashCatalogMatch> {
        let q_len = criteria.query.len();
        if q_len < 4 {
            return Vec::new();
        }

        let probe_stride = 1.max(4 / self.factor_k.min(4));
        let probe_frames: Vec<(usize, u64)> = criteria
            .query
            .iter()
            .enumerate()
            .step_by(probe_stride)
            .map(|(k, &h)| (k, h))
            .collect();

        let partial_votes: Vec<Vec<(usize, usize)>> = probe_frames
            .par_iter()
            .map(|&(k, q_hash)| {
                let mut hits = Vec::new();
                let u = (q_hash >> 32) as u32;
                let upper_slice = self.coarse_mih32.lookup_upper(u);
                if upper_slice.len() <= 20_000 {
                    for entry in upper_slice {
                        let (m_idx, ref_off) = self
                            .coarse_catalog
                            .resolve_global_index(entry.frame_idx as usize);
                        if ref_off >= k {
                            hits.push((m_idx, ref_off - k));
                        }
                    }
                }
                let l = (q_hash & 0xFFFF_FFFF) as u32;
                let lower_slice = self.coarse_mih32.lookup_lower(l);
                if lower_slice.len() <= 20_000 {
                    for entry in lower_slice {
                        let (m_idx, ref_off) = self
                            .coarse_catalog
                            .resolve_global_index(entry.frame_idx as usize);
                        if ref_off >= k {
                            hits.push((m_idx, ref_off - k));
                        }
                    }
                }
                hits
            })
            .collect();

        let mut votes = HashMap::new();
        for hits in partial_votes {
            for pair in hits {
                *votes.entry(pair).or_default() += 1;
            }
        }

        let min_votes = 1.max(3 / self.factor_k.min(3) as u32);
        let mut clusters: Vec<(usize, usize, u32)> = votes
            .into_iter()
            .filter(|&(_, v)| v >= min_votes)
            .map(|((m_idx, shift), v)| (m_idx, shift, v))
            .collect();

        clusters.sort_by_key(|a| std::cmp::Reverse(a.2));

        let mut matches = Vec::new();
        for (m_idx, coarse_shift, _v) in clusters.into_iter().take(3) {
            let entry = &self.coarse_catalog.entries[m_idx];
            matches.push(AHashCatalogMatch {
                media_name: entry.media_name.clone(),
                file_path: entry.path.to_string_lossy().to_string(),
                ref_frame_offset: coarse_shift,
                offset_sec: coarse_shift as f64 / entry.audio_fps,
                offset_ms: (coarse_shift as f64 / entry.audio_fps * MILLIS_PER_SEC).round() as i64,
                mean_distance: 0.0,
                score: 1.0,
                snr_bits: 32.0,
                audio_fps: entry.audio_fps,
            });
        }
        matches
    }
}

// ----------------------------------------------------------------------------
// Comprehensive Coarse-to-Fine Benchmark Matrix
// ----------------------------------------------------------------------------

#[test]
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
    println!(" 🚀 CHARGEMENT DU CATALOGUE DENSE aHash (31.25 fps, 106M frames)");
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
        let coarse_fps = 31.25 / k as f64;

        let mut total_search_ms = 0.0;
        let mut hits = 0;
        let mut total_snr = 0.0;
        let mut max_drift_ms = 0i64;

        for (path, q_hashes, q_fps) in &queries {
            let t_search = Instant::now();
            let res = c2f.search(&dense_catalog, q_hashes, *q_fps, true);
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
