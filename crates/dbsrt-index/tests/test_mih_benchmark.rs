use dbsrt_core::metadata::MediaMetadata;
use dbsrt_index::dbphash::{DbphashBuilder, MmapDbphash};
use dbsrt_index::mih::MultiIndexHash64;
use std::path::Path;
use std::time::Instant;

const MAX_DEBUG_SAMPLE_MEDIAS: usize = 5;

#[allow(unexpected_cfgs)]
fn open_benchmark_catalog(catalog_path: &Path) -> (Option<tempfile::TempDir>, MmapDbphash) {
    let full_mmap = MmapDbphash::open(catalog_path).expect("Failed to open catalog.dbphash");
    let use_bounded_sample =
        cfg!(coverage) || cfg!(debug_assertions) || std::env::var_os("LLVM_PROFILE_FILE").is_some();

    if !use_bounded_sample || full_mmap.media_entries().len() <= MAX_DEBUG_SAMPLE_MEDIAS {
        return (None, full_mmap);
    }

    let tmp = tempfile::tempdir().expect("tempdir for bounded catalog sample");
    let sample_path = tmp.path().join("sample_catalog.dbphash");
    let mut builder = DbphashBuilder::new();
    let all_cuts = full_mmap.cuts_entries();
    let all_sparse = full_mmap.sparse_entries();

    for entry in full_mmap
        .media_entries()
        .iter()
        .take(MAX_DEBUG_SAMPLE_MEDIAS)
    {
        let title = full_mmap.get_title(entry);
        let meta = MediaMetadata::parse(title);
        let c_start = entry.cuts_start_idx as usize;
        let c_end = (c_start + entry.cuts_count as usize).min(all_cuts.len());
        let cuts: Vec<(u64, u32)> = all_cuts[c_start..c_end]
            .iter()
            .map(|c| (c.phash, c.timestamp_ms))
            .collect();

        let s_start = entry.sparse_start_idx as usize;
        let s_end = (s_start + entry.sparse_count as usize).min(all_sparse.len());
        let sparse: Vec<(u64, u32)> = all_sparse[s_start..s_end]
            .iter()
            .map(|s| (s.phash, s.timestamp_ms))
            .collect();

        let fps = (entry.fps_x1000 as f64 / 1000.0).max(1.0);
        let id = builder.add_media(&meta, entry.duration_ms, fps, &cuts, &sparse);
        if let Some(fc) = entry.known_frame_count() {
            builder.set_frame_count(id, fc);
        }
    }

    builder
        .write_to_file(&sample_path)
        .expect("write bounded sample_catalog.dbphash");
    let sample_mmap = MmapDbphash::open(&sample_path).expect("open sample_catalog.dbphash");
    (Some(tmp), sample_mmap)
}

#[test]
fn test_mih_vs_linear_on_real_catalog() {
    let catalog_path = if Path::new("data/catalog.dbphash").exists() {
        Path::new("data/catalog.dbphash")
    } else if Path::new("../../data/catalog.dbphash").exists() {
        Path::new("../../data/catalog.dbphash")
    } else {
        println!("catalog.dbphash not found in ./ or ../../, skipping");
        return;
    };

    let (_tmp_guard, mmap) = open_benchmark_catalog(catalog_path);
    let sparse_entries = mmap.sparse_entries();
    let cuts_entries = mmap.cuts_entries();

    println!(
        "\n=== CATALOG STATS ===\nSparse entries: {}\nCuts entries: {}\nTotal targets: {}",
        sparse_entries.len(),
        cuts_entries.len(),
        sparse_entries.len() + cuts_entries.len()
    );

    // 1. Measure direct MIH build time on zero-copy slices
    let t0 = Instant::now();
    let sparse_mih = MultiIndexHash64::build_from_provider(sparse_entries);
    let sparse_build_time = t0.elapsed();

    let t1 = Instant::now();
    let cuts_mih = MultiIndexHash64::build_from_provider(cuts_entries);
    let cuts_build_time = t1.elapsed();

    println!(
        "MIH Build Time (zero-copy slices): Sparse: {:.2?} | Cuts: {:.2?}",
        sparse_build_time, cuts_build_time
    );

    // Pick 25 query frames from a known media
    let q_start = 1000.min(sparse_entries.len().saturating_sub(25));
    let q_end = (q_start + 25).min(sparse_entries.len());
    let query_frames: Vec<u64> = sparse_entries[q_start..q_end]
        .iter()
        .map(|e| e.phash)
        .collect();
    let max_dist = 6;

    // 2. Measure Sparse: Linear Scan vs MIH
    let _ = mmap.sparse_mih(); // Warm up OnceLock cache
    let t_lin = Instant::now();
    let linear_sparse_matches = mmap.search_sparse_linear(&query_frames, max_dist);
    let elapsed_linear = t_lin.elapsed();

    let t_mih = Instant::now();
    let mih_sparse_matches = mmap.search_sparse(&query_frames, max_dist);
    let elapsed_mih = t_mih.elapsed();

    println!(
        "\n=== SPARSE SEARCH COMPARISON (25 frames, max_dist = {}) ===",
        max_dist
    );
    println!(
        "Linear Scan (Par) : {:?} total ({:.2?} / query) | Matches: {}",
        elapsed_linear,
        elapsed_linear / query_frames.len() as u32,
        linear_sparse_matches.len()
    );
    println!(
        "MIH Search (Par)  : {:?} total ({:.2?} / query) | Matches: {}",
        elapsed_mih,
        elapsed_mih / query_frames.len() as u32,
        mih_sparse_matches.len()
    );

    assert_eq!(
        linear_sparse_matches.len(),
        mih_sparse_matches.len(),
        "Sparse MIH must find exact same number of matches as linear scan!"
    );

    let speedup_sparse = elapsed_linear.as_secs_f64() / elapsed_mih.as_secs_f64();
    println!("SPARSE SPEEDUP: {:.1}x FASTER WITH MIH!", speedup_sparse);

    // 3. Measure Cuts: Linear Scan vs MIH
    let _ = mmap.cuts_mih(); // Warm up OnceLock cache
    let t_lin_cuts = Instant::now();
    let linear_cuts_matches = mmap.search_cuts_linear(&query_frames, max_dist);
    let elapsed_linear_cuts = t_lin_cuts.elapsed();

    let t_mih_cuts = Instant::now();
    let mih_cuts_matches = mmap.search_cuts(&query_frames, max_dist);
    let elapsed_mih_cuts = t_mih_cuts.elapsed();

    println!(
        "\n=== CUTS SEARCH COMPARISON (25 frames, max_dist = {}) ===",
        max_dist
    );
    println!(
        "Linear Scan (Par) : {:?} total ({:.2?} / query) | Matches: {}",
        elapsed_linear_cuts,
        elapsed_linear_cuts / query_frames.len() as u32,
        linear_cuts_matches.len()
    );
    println!(
        "MIH Search (Par)  : {:?} total ({:.2?} / query) | Matches: {}",
        elapsed_mih_cuts,
        elapsed_mih_cuts / query_frames.len() as u32,
        mih_cuts_matches.len()
    );

    assert_eq!(
        linear_cuts_matches.len(),
        mih_cuts_matches.len(),
        "Cuts MIH must find exact same number of matches as linear scan!"
    );

    let speedup_cuts = elapsed_linear_cuts.as_secs_f64() / elapsed_mih_cuts.as_secs_f64();
    println!("CUTS SPEEDUP: {:.1}x FASTER WITH MIH!\n", speedup_cuts);

    // Verify cuts_mih directly
    let direct_matches = cuts_mih.search(query_frames[0], max_dist, cuts_entries);
    let sparse_direct = sparse_mih.search(query_frames[0], max_dist, sparse_entries);
    assert!(!sparse_direct.is_empty());
    let _ = direct_matches;
}
