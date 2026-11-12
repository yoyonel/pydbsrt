use dbsrt_core::ahash::{DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE};
use dbsrt_core::phash::hamming_distance;
use dbsrt_core::{extract_ahash, extract_frames_phash, AudioExtractionOptions};
use dbsrt_index::ahash::{
    AHashCatalog, AHashCatalogMatch, AHashSearchCriteria, DEFAULT_AHASH_MIH_JITTER_RADIUS,
    FPS_COMPATIBILITY_TOLERANCE, MILLIS_PER_SEC, MIN_QUERY_FRAMES,
};
use dbsrt_index::dbphash::{DbphashMediaMatch, MmapDbphash};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mih32Entry {
    pub chunk: u32,
    pub frame_idx: u32,
}

pub struct Mih32Table {
    pub upper_table: Vec<Mih32Entry>,
    pub lower_table: Vec<Mih32Entry>,
}

impl Mih32Table {
    pub fn build(flat_hashes: &[u64]) -> Self {
        let (upper, lower) = rayon::join(
            || {
                let mut v: Vec<Mih32Entry> = flat_hashes
                    .par_iter()
                    .enumerate()
                    .map(|(i, &h)| Mih32Entry {
                        chunk: (h >> 32) as u32,
                        frame_idx: i as u32,
                    })
                    .collect();
                v.par_sort_unstable_by_key(|e| e.chunk);
                v
            },
            || {
                let mut v: Vec<Mih32Entry> = flat_hashes
                    .par_iter()
                    .enumerate()
                    .map(|(i, &h)| Mih32Entry {
                        chunk: (h & 0xFFFF_FFFF) as u32,
                        frame_idx: i as u32,
                    })
                    .collect();
                v.par_sort_unstable_by_key(|e| e.chunk);
                v
            },
        );

        Self {
            upper_table: upper,
            lower_table: lower,
        }
    }

    #[inline(always)]
    pub fn lookup_upper(&self, chunk: u32) -> &[Mih32Entry] {
        Self::lookup_slice(&self.upper_table, chunk)
    }

    #[inline(always)]
    pub fn lookup_lower(&self, chunk: u32) -> &[Mih32Entry] {
        Self::lookup_slice(&self.lower_table, chunk)
    }

    #[inline(always)]
    fn lookup_slice(table: &[Mih32Entry], chunk: u32) -> &[Mih32Entry] {
        let start = table.partition_point(|e| e.chunk < chunk);
        if start < table.len() && table[start].chunk == chunk {
            let end = table[start..].partition_point(|e| e.chunk <= chunk) + start;
            &table[start..end]
        } else {
            &[]
        }
    }
}

// ----------------------------------------------------------------------------
// aHash Search Methods
// ----------------------------------------------------------------------------

fn search_ahash_mih32(
    catalog: &AHashCatalog,
    index: &Mih32Table,
    criteria: &AHashSearchCriteria,
    stride: usize,
    min_votes: u32,
    parallel: bool,
) -> Vec<AHashCatalogMatch> {
    if criteria.query.len() < MIN_QUERY_FRAMES || catalog.total_frames == 0 {
        return Vec::new();
    }

    let probe_frames: Vec<(usize, u64)> = criteria
        .query
        .iter()
        .enumerate()
        .step_by(stride)
        .map(|(k, &h)| (k, h))
        .collect();

    let votes: HashMap<(usize, usize), u32> = if parallel {
        let partial_votes: Vec<Vec<(usize, usize)>> = probe_frames
            .par_iter()
            .map(|&(k, q_hash)| {
                let mut hits = Vec::new();
                let u = (q_hash >> 32) as u32;
                let upper_slice = index.lookup_upper(u);
                // Cap oversized buckets to avoid generic background hums
                if upper_slice.len() <= 20_000 {
                    for entry in upper_slice {
                        let (m_idx, ref_off) =
                            catalog.resolve_global_index(entry.frame_idx as usize);
                        if ref_off >= k {
                            hits.push((m_idx, ref_off - k));
                        }
                    }
                }
                let l = (q_hash & 0xFFFF_FFFF) as u32;
                let lower_slice = index.lookup_lower(l);
                if lower_slice.len() <= 20_000 {
                    for entry in lower_slice {
                        let (m_idx, ref_off) =
                            catalog.resolve_global_index(entry.frame_idx as usize);
                        if ref_off >= k {
                            hits.push((m_idx, ref_off - k));
                        }
                    }
                }
                hits
            })
            .collect();

        let mut v_map = HashMap::new();
        for hits in partial_votes {
            for pair in hits {
                *v_map.entry(pair).or_default() += 1;
            }
        }
        v_map
    } else {
        let mut v_map = HashMap::new();
        for (k, q_hash) in probe_frames {
            let u = (q_hash >> 32) as u32;
            let upper_slice = index.lookup_upper(u);
            if upper_slice.len() <= 20_000 {
                for entry in upper_slice {
                    let (m_idx, ref_off) = catalog.resolve_global_index(entry.frame_idx as usize);
                    if ref_off >= k {
                        *v_map.entry((m_idx, ref_off - k)).or_default() += 1;
                    }
                }
            }
            let l = (q_hash & 0xFFFF_FFFF) as u32;
            let lower_slice = index.lookup_lower(l);
            if lower_slice.len() <= 20_000 {
                for entry in lower_slice {
                    let (m_idx, ref_off) = catalog.resolve_global_index(entry.frame_idx as usize);
                    if ref_off >= k {
                        *v_map.entry((m_idx, ref_off - k)).or_default() += 1;
                    }
                }
            }
        }
        v_map
    };

    let mut clusters: Vec<(usize, usize, u32)> = votes
        .into_iter()
        .filter(|&(_, v)| v >= min_votes)
        .map(|((m_idx, shift), v)| (m_idx, shift, v))
        .collect();

    clusters.sort_by_key(|a| std::cmp::Reverse(a.2));

    let q_len = criteria.query.len();
    let mut matches = Vec::new();

    for (media_idx, coarse_shift, _) in clusters.into_iter().take(5) {
        let entry = &catalog.entries[media_idx];
        if (entry.audio_fps - criteria.audio_fps).abs() > FPS_COMPATIBILITY_TOLERANCE {
            continue;
        }
        let ref_len = entry.hashes.len();
        if ref_len < q_len {
            continue;
        }

        let min_s = coarse_shift.saturating_sub(DEFAULT_AHASH_MIH_JITTER_RADIUS);
        let max_s = (coarse_shift + DEFAULT_AHASH_MIH_JITTER_RADIUS).min(ref_len - q_len);

        let mut best_shift = coarse_shift;
        let mut best_dist = u64::MAX;

        for s in min_s..=max_s {
            let ref_slice = &entry.hashes[s..s + q_len];
            let mut dist = 0u64;
            for (&q, &r) in criteria.query.iter().zip(ref_slice) {
                dist += (q ^ r).count_ones() as u64;
            }
            if dist < best_dist {
                best_dist = dist;
                best_shift = s;
            }
        }

        let mean_dist = best_dist as f64 / q_len as f64;
        if mean_dist <= criteria.max_distance {
            let score = (1.0 - (mean_dist / 64.0)).clamp(0.0, 1.0);
            let snr_bits = (32.0 - mean_dist).max(0.0);
            let offset_sec = best_shift as f64 / entry.audio_fps;
            matches.push(AHashCatalogMatch {
                media_name: entry.media_name.clone(),
                file_path: entry.path.to_string_lossy().to_string(),
                ref_frame_offset: best_shift,
                offset_sec,
                offset_ms: (offset_sec * MILLIS_PER_SEC).round() as i64,
                mean_distance: mean_dist,
                score,
                snr_bits,
                audio_fps: entry.audio_fps,
            });
        }
    }

    matches.sort_by(|a, b| {
        a.mean_distance
            .partial_cmp(&b.mean_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches
}

fn search_ahash_mih16(
    catalog: &AHashCatalog,
    criteria: &AHashSearchCriteria,
    stride: usize,
    min_votes: u32,
    parallel: bool,
) -> Vec<AHashCatalogMatch> {
    let mih = catalog.mih();
    let probe_frames: Vec<(usize, u64)> = criteria
        .query
        .iter()
        .enumerate()
        .step_by(stride)
        .map(|(k, &h)| (k, h))
        .collect();

    let votes: HashMap<(usize, usize), u32> = if parallel {
        let partial_votes: Vec<Vec<(usize, usize)>> = probe_frames
            .par_iter()
            .map(|&(k, q_hash)| {
                let mut hits = Vec::new();
                for (t, table) in mih.tables.iter().enumerate() {
                    let chunk = dbsrt_index::mih::extract_chunk(q_hash, t);
                    let bucket = table.bucket_indices(chunk);
                    if bucket.len() <= 20_000 {
                        for &cand_idx in bucket {
                            let (m_idx, ref_off) = catalog.resolve_global_index(cand_idx as usize);
                            if ref_off >= k {
                                hits.push((m_idx, ref_off - k));
                            }
                        }
                    }
                }
                hits
            })
            .collect();

        let mut v_map = HashMap::new();
        for hits in partial_votes {
            for pair in hits {
                *v_map.entry(pair).or_default() += 1;
            }
        }
        v_map
    } else {
        let mut v_map = HashMap::new();
        for (k, q_hash) in probe_frames {
            for (t, table) in mih.tables.iter().enumerate() {
                let chunk = dbsrt_index::mih::extract_chunk(q_hash, t);
                let bucket = table.bucket_indices(chunk);
                if bucket.len() <= 20_000 {
                    for &cand_idx in bucket {
                        let (m_idx, ref_off) = catalog.resolve_global_index(cand_idx as usize);
                        if ref_off >= k {
                            *v_map.entry((m_idx, ref_off - k)).or_default() += 1;
                        }
                    }
                }
            }
        }
        v_map
    };

    let mut clusters: Vec<(usize, usize, u32)> = votes
        .into_iter()
        .filter(|&(_, v)| v >= min_votes)
        .map(|((m_idx, shift), v)| (m_idx, shift, v))
        .collect();

    clusters.sort_by_key(|a| std::cmp::Reverse(a.2));

    let q_len = criteria.query.len();
    let mut matches = Vec::new();

    for (media_idx, coarse_shift, _) in clusters.into_iter().take(5) {
        let entry = &catalog.entries[media_idx];
        if (entry.audio_fps - criteria.audio_fps).abs() > FPS_COMPATIBILITY_TOLERANCE {
            continue;
        }
        let ref_len = entry.hashes.len();
        if ref_len < q_len {
            continue;
        }

        let min_s = coarse_shift.saturating_sub(DEFAULT_AHASH_MIH_JITTER_RADIUS);
        let max_s = (coarse_shift + DEFAULT_AHASH_MIH_JITTER_RADIUS).min(ref_len - q_len);

        let mut best_shift = coarse_shift;
        let mut best_dist = u64::MAX;

        for s in min_s..=max_s {
            let ref_slice = &entry.hashes[s..s + q_len];
            let mut dist = 0u64;
            for (&q, &r) in criteria.query.iter().zip(ref_slice) {
                dist += (q ^ r).count_ones() as u64;
            }
            if dist < best_dist {
                best_dist = dist;
                best_shift = s;
            }
        }

        let mean_dist = best_dist as f64 / q_len as f64;
        if mean_dist <= criteria.max_distance {
            let score = (1.0 - (mean_dist / 64.0)).clamp(0.0, 1.0);
            let snr_bits = (32.0 - mean_dist).max(0.0);
            let offset_sec = best_shift as f64 / entry.audio_fps;
            matches.push(AHashCatalogMatch {
                media_name: entry.media_name.clone(),
                file_path: entry.path.to_string_lossy().to_string(),
                ref_frame_offset: best_shift,
                offset_sec,
                offset_ms: (offset_sec * MILLIS_PER_SEC).round() as i64,
                mean_distance: mean_dist,
                score,
                snr_bits,
                audio_fps: entry.audio_fps,
            });
        }
    }

    matches.sort_by(|a, b| {
        a.mean_distance
            .partial_cmp(&b.mean_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches
}

// ----------------------------------------------------------------------------
// pHash Search Methods
// ----------------------------------------------------------------------------

fn search_phash_mih16_mono(
    mmap: &MmapDbphash,
    query_hashes: &[u64],
    max_dist: u32,
) -> Vec<DbphashMediaMatch> {
    let sparse = mmap.sparse_entries();
    let cuts = mmap.cuts_entries();
    let sparse_mih = mmap.sparse_mih();
    let cuts_mih = mmap.cuts_mih();

    let mut sparse_matches = Vec::new();
    for (q_idx, &q_hash) in query_hashes.iter().enumerate() {
        let candidates = sparse_mih.search(q_hash, max_dist, sparse);
        for c in candidates {
            sparse_matches.push(dbsrt_index::dbphash::DbphashCutMatch {
                query_idx: q_idx,
                cut_entry: sparse[c.index],
                distance: c.distance,
            });
        }
    }

    let mut cuts_matches = Vec::new();
    for (q_idx, &q_hash) in query_hashes.iter().enumerate() {
        let candidates = cuts_mih.search(q_hash, max_dist, cuts);
        for c in candidates {
            cuts_matches.push(dbsrt_index::dbphash::DbphashCutMatch {
                query_idx: q_idx,
                cut_entry: cuts[c.index],
                distance: c.distance,
            });
        }
    }

    mmap.aggregate_matches(query_hashes, sparse_matches, cuts_matches)
}

fn search_phash_mih32(
    mmap: &MmapDbphash,
    sparse_index32: &Mih32Table,
    cuts_index32: &Mih32Table,
    query_hashes: &[u64],
    parallel: bool,
) -> Vec<DbphashMediaMatch> {
    let sparse = mmap.sparse_entries();
    let cuts = mmap.cuts_entries();

    let (sparse_matches, cuts_matches) = if parallel {
        rayon::join(
            || {
                query_hashes
                    .par_iter()
                    .enumerate()
                    .flat_map_iter(|(q_idx, &q_hash)| {
                        let mut hits = Vec::new();
                        let u = (q_hash >> 32) as u32;
                        for entry in sparse_index32.lookup_upper(u) {
                            let cand = sparse[entry.frame_idx as usize];
                            let dist = hamming_distance(cand.phash, q_hash);
                            if dist <= 6 {
                                hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                                    query_idx: q_idx,
                                    cut_entry: cand,
                                    distance: dist,
                                });
                            }
                        }
                        let l = (q_hash & 0xFFFF_FFFF) as u32;
                        for entry in sparse_index32.lookup_lower(l) {
                            let cand = sparse[entry.frame_idx as usize];
                            let dist = hamming_distance(cand.phash, q_hash);
                            if dist <= 6 {
                                hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                                    query_idx: q_idx,
                                    cut_entry: cand,
                                    distance: dist,
                                });
                            }
                        }
                        hits
                    })
                    .collect()
            },
            || {
                query_hashes
                    .par_iter()
                    .enumerate()
                    .flat_map_iter(|(q_idx, &q_hash)| {
                        let mut hits = Vec::new();
                        let u = (q_hash >> 32) as u32;
                        for entry in cuts_index32.lookup_upper(u) {
                            let cand = cuts[entry.frame_idx as usize];
                            let dist = hamming_distance(cand.phash, q_hash);
                            if dist <= 6 {
                                hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                                    query_idx: q_idx,
                                    cut_entry: cand,
                                    distance: dist,
                                });
                            }
                        }
                        let l = (q_hash & 0xFFFF_FFFF) as u32;
                        for entry in cuts_index32.lookup_lower(l) {
                            let cand = cuts[entry.frame_idx as usize];
                            let dist = hamming_distance(cand.phash, q_hash);
                            if dist <= 6 {
                                hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                                    query_idx: q_idx,
                                    cut_entry: cand,
                                    distance: dist,
                                });
                            }
                        }
                        hits
                    })
                    .collect()
            },
        )
    } else {
        let mut s_hits = Vec::new();
        for (q_idx, &q_hash) in query_hashes.iter().enumerate() {
            let u = (q_hash >> 32) as u32;
            for entry in sparse_index32.lookup_upper(u) {
                let cand = sparse[entry.frame_idx as usize];
                let dist = hamming_distance(cand.phash, q_hash);
                if dist <= 6 {
                    s_hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                        query_idx: q_idx,
                        cut_entry: cand,
                        distance: dist,
                    });
                }
            }
            let l = (q_hash & 0xFFFF_FFFF) as u32;
            for entry in sparse_index32.lookup_lower(l) {
                let cand = sparse[entry.frame_idx as usize];
                let dist = hamming_distance(cand.phash, q_hash);
                if dist <= 6 {
                    s_hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                        query_idx: q_idx,
                        cut_entry: cand,
                        distance: dist,
                    });
                }
            }
        }

        let mut c_hits = Vec::new();
        for (q_idx, &q_hash) in query_hashes.iter().enumerate() {
            let u = (q_hash >> 32) as u32;
            for entry in cuts_index32.lookup_upper(u) {
                let cand = cuts[entry.frame_idx as usize];
                let dist = hamming_distance(cand.phash, q_hash);
                if dist <= 6 {
                    c_hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                        query_idx: q_idx,
                        cut_entry: cand,
                        distance: dist,
                    });
                }
            }
            let l = (q_hash & 0xFFFF_FFFF) as u32;
            for entry in cuts_index32.lookup_lower(l) {
                let cand = cuts[entry.frame_idx as usize];
                let dist = hamming_distance(cand.phash, q_hash);
                if dist <= 6 {
                    c_hits.push(dbsrt_index::dbphash::DbphashCutMatch {
                        query_idx: q_idx,
                        cut_entry: cand,
                        distance: dist,
                    });
                }
            }
        }
        (s_hits, c_hits)
    };

    mmap.aggregate_matches(query_hashes, sparse_matches, cuts_matches)
}

// ----------------------------------------------------------------------------
// Matrix Benchmark Test
// ----------------------------------------------------------------------------

#[test]
fn test_exhaustive_matrix_benchmark() {
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
    let catalog_path = if Path::new("data/catalog.dbphash").exists() {
        Path::new("data/catalog.dbphash")
    } else {
        Path::new("../../data/catalog.dbphash")
    };

    if !ahash_dir.exists() || !snippets_dir.exists() || !catalog_path.exists() {
        println!("Skipping matrix benchmark: missing data directories");
        return;
    }

    println!("\n====================================================================================================");
    println!(" 🚀 CHARGEMENT DES CATALOGUES ET PRÉPARATION DES INDEX");
    println!("====================================================================================================");

    // 1. aHash Catalog
    let t_a0 = Instant::now();
    let ahash_catalog = AHashCatalog::load_from_dir(ahash_dir).expect("Load ahash");
    let ahash_index32 = Mih32Table::build(&ahash_catalog.flat_hashes);
    let _ = ahash_catalog.mih(); // warm up MIH-16
    println!(
        "✓ aHash Catalog : 584 médias, 106 096 618 frames (chargé + indexé en {:.2?})",
        t_a0.elapsed()
    );

    // 2. pHash Catalog
    let t_p0 = Instant::now();
    let phash_mmap = MmapDbphash::open(catalog_path).expect("Open dbphash");
    let sparse_hashes: Vec<u64> = phash_mmap
        .sparse_entries()
        .iter()
        .map(|e| e.phash)
        .collect();
    let cuts_hashes: Vec<u64> = phash_mmap.cuts_entries().iter().map(|e| e.phash).collect();
    let phash_sparse32 = Mih32Table::build(&sparse_hashes);
    let phash_cuts32 = Mih32Table::build(&cuts_hashes);
    let _ = phash_mmap.sparse_mih(); // warm up MIH-16
    let _ = phash_mmap.cuts_mih();
    println!(
        "✓ pHash Catalog : 2 758 médias, 2 708 720 frames (chargé + indexé en {:.2?})",
        t_p0.elapsed()
    );

    // 3. Cut Files
    let mut cut_paths: Vec<_> = std::fs::read_dir(snippets_dir)
        .expect("Read snippets")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "mp4"))
        .map(|e| e.path())
        .collect();
    cut_paths.sort();

    println!("\n====================================================================================================");
    println!(" 📊 MATRICE D'ÉVALUATION ACOUSTIQUE (aHash - 106M frames) SUR LES 10 CUTS RÉELS");
    println!("====================================================================================================");
    println!(
        "{:<20} | {:<10} | {:<12} | {:<12} | {:<12} | {:<12}",
        "Cut File", "Linear O(N)", "MIH16-Mono", "MIH16-Rayon", "MIH32-Mono", "MIH32-Rayon"
    );
    println!("----------------------------------------------------------------------------------------------------");

    let mut a_tot_lin = 0.0;
    let mut a_tot_m16_mono = 0.0;
    let mut a_tot_m16_par = 0.0;
    let mut a_tot_m32_mono = 0.0;
    let mut a_tot_m32_par = 0.0;
    let mut a_count = 0;

    for cut_path in &cut_paths {
        let opts = AudioExtractionOptions {
            sample_rate: DEFAULT_SAMPLE_RATE,
            frame_size: DEFAULT_FRAME_SIZE,
            hop_size: DEFAULT_HOP_SIZE,
            start_sec: 0.0,
            duration_sec: Some(5.0),
            custom_ffmpeg: None,
        };

        let ahash_file = match extract_ahash(cut_path, &opts) {
            Ok(f) if !f.hashes.is_empty() => f,
            _ => continue,
        };

        let criteria = AHashSearchCriteria::new(&ahash_file.hashes, ahash_file.header.audio_fps());

        // 1. Linear Scan
        let t_lin = Instant::now();
        let _ = ahash_catalog.search_linear(&criteria);
        let ms_lin = t_lin.elapsed().as_secs_f64() * 1000.0;

        // 2. MIH-16 Mono
        let t_m16_m = Instant::now();
        let _ = search_ahash_mih16(&ahash_catalog, &criteria, 4, 3, false);
        let ms_m16_m = t_m16_m.elapsed().as_secs_f64() * 1000.0;

        // 3. MIH-16 Rayon
        let t_m16_p = Instant::now();
        let _ = search_ahash_mih16(&ahash_catalog, &criteria, 4, 3, true);
        let ms_m16_p = t_m16_p.elapsed().as_secs_f64() * 1000.0;

        // 4. MIH-32 Mono
        let t_m32_m = Instant::now();
        let _ = search_ahash_mih32(&ahash_catalog, &ahash_index32, &criteria, 4, 3, false);
        let ms_m32_m = t_m32_m.elapsed().as_secs_f64() * 1000.0;

        // 5. MIH-32 Rayon
        let t_m32_p = Instant::now();
        let _m32_res = search_ahash_mih32(&ahash_catalog, &ahash_index32, &criteria, 4, 3, true);
        let ms_m32_p = t_m32_p.elapsed().as_secs_f64() * 1000.0;

        a_tot_lin += ms_lin;
        a_tot_m16_mono += ms_m16_m;
        a_tot_m16_par += ms_m16_p;
        a_tot_m32_mono += ms_m32_m;
        a_tot_m32_par += ms_m32_p;
        a_count += 1;

        let name = cut_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let short = if name.len() > 19 { &name[..19] } else { name };
        println!(
            "{:<20} | {:>7.0} ms | {:>9.1} ms | {:>9.1} ms | {:>9.1} ms | {:>9.1} ms",
            short, ms_lin, ms_m16_m, ms_m16_p, ms_m32_m, ms_m32_p
        );
    }

    println!("----------------------------------------------------------------------------------------------------");
    println!(
        "{:<20} | {:>7.0} ms | {:>9.1} ms | {:>9.1} ms | {:>9.1} ms | {:>9.1} ms",
        "MOYENNE aHash",
        a_tot_lin / a_count as f64,
        a_tot_m16_mono / a_count as f64,
        a_tot_m16_par / a_count as f64,
        a_tot_m32_mono / a_count as f64,
        a_tot_m32_par / a_count as f64,
    );

    println!("\n====================================================================================================");
    println!(" 📊 MATRICE D'ÉVALUATION VISUELLE (pHash - 2.7M frames) SUR LES 10 CUTS RÉELS");
    println!("====================================================================================================");
    println!(
        "{:<20} | {:<10} | {:<12} | {:<12} | {:<12} | {:<12} | {:<12}",
        "Cut File",
        "Linear O(N)",
        "MIH16-Mono",
        "MIH16-Rayon",
        "MIH32-Mono",
        "MIH32-Rayon",
        "MIH32 Match?"
    );
    println!("----------------------------------------------------------------------------------------------------");

    let mut p_tot_lin = 0.0;
    let mut p_tot_m16_mono = 0.0;
    let mut p_tot_m16_par = 0.0;
    let mut p_tot_m32_mono = 0.0;
    let mut p_tot_m32_par = 0.0;
    let mut p_m32_matches = 0;
    let mut p_count = 0;

    for cut_path in &cut_paths {
        let q_hashes = match extract_frames_phash(cut_path, 0.0, 24) {
            Ok(h) if !h.is_empty() => h,
            _ => continue,
        };

        // 1. Linear Scan
        let t_lin = Instant::now();
        let _ = phash_mmap.recognize_linear(&q_hashes, 6);
        let ms_lin = t_lin.elapsed().as_secs_f64() * 1000.0;

        // 2. MIH-16 Mono
        let t_m16_m = Instant::now();
        let _ = search_phash_mih16_mono(&phash_mmap, &q_hashes, 6);
        let ms_m16_m = t_m16_m.elapsed().as_secs_f64() * 1000.0;

        // 3. MIH-16 Rayon
        let t_m16_p = Instant::now();
        let m16_res = phash_mmap.recognize(&q_hashes, 6);
        let ms_m16_p = t_m16_p.elapsed().as_secs_f64() * 1000.0;

        // 4. MIH-32 Mono
        let t_m32_m = Instant::now();
        let _ = search_phash_mih32(
            &phash_mmap,
            &phash_sparse32,
            &phash_cuts32,
            &q_hashes,
            false,
        );
        let ms_m32_m = t_m32_m.elapsed().as_secs_f64() * 1000.0;

        // 5. MIH-32 Rayon
        let t_m32_p = Instant::now();
        let m32_res =
            search_phash_mih32(&phash_mmap, &phash_sparse32, &phash_cuts32, &q_hashes, true);
        let ms_m32_p = t_m32_p.elapsed().as_secs_f64() * 1000.0;

        p_tot_lin += ms_lin;
        p_tot_m16_mono += ms_m16_m;
        p_tot_m16_par += ms_m16_p;
        p_tot_m32_mono += ms_m32_m;
        p_tot_m32_par += ms_m32_p;
        p_count += 1;

        let m32_success = !m32_res.is_empty()
            && !m16_res.is_empty()
            && m32_res[0].media_id == m16_res[0].media_id;
        if m32_success {
            p_m32_matches += 1;
        }

        let name = cut_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let short = if name.len() > 19 { &name[..19] } else { name };
        let m32_str = if m32_success {
            "✅ Match"
        } else if m32_res.is_empty() {
            "❌ 0 Match"
        } else {
            "⚠️ Diverge"
        };

        println!(
            "{:<20} | {:>7.2} ms | {:>9.2} ms | {:>9.2} ms | {:>9.2} ms | {:>9.2} ms | {:<12}",
            short, ms_lin, ms_m16_m, ms_m16_p, ms_m32_m, ms_m32_p, m32_str
        );
    }

    println!("----------------------------------------------------------------------------------------------------");
    println!(
        "{:<20} | {:>7.2} ms | {:>9.2} ms | {:>9.2} ms | {:>9.2} ms | {:>9.2} ms | Taux: {}/{}",
        "MOYENNE pHash",
        p_tot_lin / p_count as f64,
        p_tot_m16_mono / p_count as f64,
        p_tot_m16_par / p_count as f64,
        p_tot_m32_mono / p_count as f64,
        p_tot_m32_par / p_count as f64,
        p_m32_matches,
        p_count
    );
    println!("====================================================================================================");
}
