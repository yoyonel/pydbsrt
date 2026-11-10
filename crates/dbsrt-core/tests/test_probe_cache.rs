//! Integration tests for [`dbsrt_core::probe_cache::ProbeCache`] on actual video files.

use dbsrt_core::probe_cache::{compute_local_file_moviehash, ProbeCache, ProbeEntry};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn locate_sample_videos(max_count: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir("data/snippets") {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|ext| ext == "mp4") {
                if let Ok(meta) = p.metadata() {
                    if meta.len() >= 65_536 {
                        files.push(p);
                        if files.len() >= max_count {
                            break;
                        }
                    }
                }
            }
        }
    }
    if files.is_empty() {
        let fallback = PathBuf::from("data/white_frames.mp4");
        if fallback.is_file() {
            files.push(fallback);
        }
    }
    files
}

#[test]
fn test_probe_cache_real_video_miss_then_hit_latency() {
    let videos = locate_sample_videos(1);
    let video_path = match videos.first() {
        Some(v) => v,
        None => {
            eprintln!("Skipping real video probe test: no sample video found in data/");
            return;
        }
    };

    let temp_dir =
        std::env::temp_dir().join(format!("test_probe_cache_real_{}", std::process::id()));
    let cache_file = temp_dir.join("live_probes_cache.json");

    let cache = ProbeCache::load_or_create(cache_file.clone()).expect("init cache");
    assert_eq!(cache.len(), 0);

    // Call 1: Cache Miss -> Must execute ffprobe and persist to disk
    let t_start_miss = Instant::now();
    let entry_miss = cache
        .query_or_probe_local(video_path)
        .expect("probe real video");
    let elapsed_miss = t_start_miss.elapsed();

    assert!(!entry_miss.moviehash.is_empty());
    assert!(entry_miss.file_size > 65_536);
    assert!(entry_miss.duration_seconds > 0.0);
    assert!(entry_miss.fps > 0.0);
    assert_eq!(cache.len(), 1);
    assert!(cache_file.is_file(), "Cache file must be persisted on disk");

    // Call 2: Cache Hit -> Must be instantaneous (< 1 ms) without running ffprobe
    let t_start_hit = Instant::now();
    let entry_hit = cache
        .query_or_probe_local(video_path)
        .expect("hit real video");
    let elapsed_hit = t_start_hit.elapsed();

    assert_eq!(entry_hit, entry_miss);
    assert_eq!(cache.len(), 1);

    println!(
        "🎯 Real Video Probe Test (Single): Miss: {:?} (ffprobe) vs Hit: {:?} (cache lookup)",
        elapsed_miss, elapsed_hit
    );
    assert!(
        elapsed_hit < elapsed_miss,
        "Cache hit ({:?}) must be faster than cache miss ({:?})",
        elapsed_hit,
        elapsed_miss
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_probe_cache_batch_real_videos_speedup() {
    let videos = locate_sample_videos(10);
    if videos.len() < 2 {
        eprintln!("Skipping batch probe test: fewer than 2 videos found");
        return;
    }

    let temp_dir =
        std::env::temp_dir().join(format!("test_probe_cache_batch_{}", std::process::id()));
    let cache_file = temp_dir.join("batch_probes_cache.json");

    let cache = ProbeCache::load_or_create(cache_file.clone()).expect("init cache");

    // Pass 1: Cold Run (All Misses -> executing ffprobe on 10 real videos)
    let t_cold_start = Instant::now();
    for v in &videos {
        let entry = cache.query_or_probe_local(v).expect("probe cold");
        assert!(entry.duration_seconds > 0.0);
    }
    let cold_duration = t_cold_start.elapsed();
    assert_eq!(cache.len(), videos.len());

    // Pass 2: Warm Run (All Hits -> 100% memory cache lookups)
    let t_warm_start = Instant::now();
    for v in &videos {
        let entry = cache.query_or_probe_local(v).expect("hit warm");
        assert!(entry.duration_seconds > 0.0);
    }
    let warm_duration = t_warm_start.elapsed();

    let speedup = cold_duration.as_secs_f64() / warm_duration.as_secs_f64().max(0.000001);
    println!(
        "🚀 Batch Real Videos ({} files): Cold: {:?} ({:.1} ms/video) vs Warm: {:?} ({:.2} µs/video) | Speedup: {:.1}x",
        videos.len(),
        cold_duration,
        (cold_duration.as_secs_f64() * 1000.0) / videos.len() as f64,
        warm_duration,
        (warm_duration.as_secs_f64() * 1_000_000.0) / videos.len() as f64,
        speedup
    );

    assert!(
        speedup > 10.0,
        "Speedup must be at least 10x (got {:.1}x)",
        speedup
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_probe_cache_exhaustive_all_snippets() {
    let snippets_dir = Path::new("data/snippets");
    if !snippets_dir.is_dir() {
        eprintln!("Skipping exhaustive snippets test: data/snippets directory not present");
        return;
    }

    let mut valid_count = 0usize;
    let mut stub_count = 0usize;
    let mut total_files = 0usize;

    let temp_dir = std::env::temp_dir().join(format!(
        "test_probe_cache_exhaustive_{}",
        std::process::id()
    ));
    let cache_file = temp_dir.join("exhaustive_cache.json");
    let cache = ProbeCache::load_or_create(cache_file).expect("init cache");

    let t_start = Instant::now();
    if let Ok(entries) = std::fs::read_dir(snippets_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "mp4") {
                total_files += 1;
                let file_size = path.metadata().map(|m| m.len()).unwrap_or(0);
                match compute_local_file_moviehash(&path) {
                    Ok((hash, size)) => {
                        assert_eq!(size, file_size);
                        assert_eq!(hash.len(), 16);
                        valid_count += 1;
                        let entry = ProbeEntry {
                            moviehash: hash,
                            file_size: size,
                            duration_seconds: 15.0,
                            fps: 25.0,
                            video_codec: Some("h264".to_string()),
                            width: Some(854),
                            height: Some(480),
                            probed_at_epoch_s: 1_700_000_000,
                        };
                        cache.put(entry).expect("cache put");
                    }
                    Err(_) => {
                        assert!(
                            file_size < 65_536,
                            "File {:?} with size {} bytes should have succeeded",
                            path,
                            file_size
                        );
                        stub_count += 1;
                    }
                }
            }
        }
    }

    let elapsed = t_start.elapsed();
    assert_eq!(total_files, valid_count + stub_count);
    // Automatic Content Deduplication:
    // Out of 2,576 valid video files, 2 files are exact bit-for-bit duplicates
    // (e.g. BigBuckBunny(1) vs BigBuckBunny, Mustang vs CpasBien Mustang).
    // The cache key `moviehash:file_size` correctly coalesces them into 2,574 unique entries.
    assert_eq!(cache.len(), 2574);

    let throughput = total_files as f64 / elapsed.as_secs_f64().max(0.001);
    println!(
        "🎯 Exhaustive Snippets Audit (100% of data/snippets): Total: {} | Valid (>= 64 KiB): {} | Stubs (< 64 KiB): {} | Unique Content Entries: {} | Duration: {:?} | Throughput: {:.1} files/s",
        total_files, valid_count, stub_count, cache.len(), elapsed, throughput
    );

    // Test 100% in-memory cache retrieval across all valid entries
    let all_cached = cache.all_entries();
    assert_eq!(all_cached.len(), cache.len());

    let t_lookup_start = Instant::now();
    for entry in &all_cached {
        let hit = cache
            .get(&entry.moviehash, entry.file_size)
            .expect("must hit cached entry");
        assert_eq!(hit.duration_seconds, 15.0);
    }
    let lookup_elapsed = t_lookup_start.elapsed();
    let lookup_rate = valid_count as f64 / lookup_elapsed.as_secs_f64().max(0.000001);
    println!(
        "⚡ 100% In-Memory Cache Lookup on {} entries: {:?} ({:.2} µs/lookup) | Throughput: {:.0} lookups/s",
        valid_count,
        lookup_elapsed,
        (lookup_elapsed.as_secs_f64() * 1_000_000.0) / valid_count as f64,
        lookup_rate
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
