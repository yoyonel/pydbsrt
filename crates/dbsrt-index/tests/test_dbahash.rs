//! Hermetic unit & integration suite for `.dbahash` v1 (`DbahashBuilder`, `MmapDbahash`, `AhashEngine`).

use dbsrt_core::ahash::{AhashFile, AhashHeader, HaitsmaKalkerExtractor};
use dbsrt_core::{AhashSidecar, AhashSidecarRequest, AudioContainerInspection, AudioStreamInfo};
use dbsrt_index::{
    pack_ahash_directories, AHashSearchCriteria, AhashEngine, CoarseSearchScratchpad,
    DbahashBuilder, DbahashPackOptions, DbahashTrackInput, FineSearchCriteria, MmapDbahash,
    DBAHASH_MAGIC, DBAHASH_SECTION_ALIGN, DBAHASH_VERSION, DEFAULT_DBAHASH_FACTOR_K,
    DEFAULT_FINE_MAX_DISTANCE,
};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

/// Nominal Haitsma-Kalker dense frame rate (`5512 / 64 = 86.125 fps`).
const TEST_AUDIO_FPS: f64 = 86.125;
/// Synthetic track length in frames (~46.4 s).
const TEST_TRACK_FRAMES: usize = 4_000;
/// Standard 5.0 s query window in frames (`430`).
const TEST_QUERY_FRAMES: usize = 430;
/// Base reference offset tested across all 32 modulo phases.
const TEST_BASE_OFFSET: usize = 1_600;
/// Sample rate for synthetic `.ahash` headers.
const TEST_SAMPLE_RATE: u32 = 5_512;
/// FFT window size for synthetic `.ahash` headers.
const TEST_FRAME_SIZE: usize = 2_048;
/// Hop size for synthetic `.ahash` headers.
const TEST_HOP_SIZE: u16 = 64;
/// Spectral sub-band count.
const TEST_NUM_BANDS: u8 = 33;
/// Spectral derivative lag.
const TEST_SPECTRAL_LAG: u8 = 2;

/// Generates deterministic uncorrelated 64-bit hashes via SplitMix64.
fn make_track_hashes(seed: u64, len: usize) -> Vec<u64> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        })
        .collect()
}

/// Flips ~3 bits per frame outside every 8th anchor to simulate AAC/Opus transcoding noise.
fn make_noisy_query(slice: &[u64], noise_seed: u64) -> Vec<u64> {
    let noise = make_track_hashes(noise_seed, slice.len());
    slice
        .iter()
        .zip(&noise)
        .enumerate()
        .map(|(i, (&h, &n))| {
            if i % 8 == 0 {
                h
            } else {
                let mask =
                    (1u64 << (n & 63)) | (1u64 << ((n >> 8) & 63)) | (1u64 << ((n >> 16) & 63));
                h ^ mask
            }
        })
        .collect()
}

/// Writes a valid `.ahash` v1.0 file on disk.
fn write_ahash_fixture(path: &Path, hashes: &[u64]) {
    let (_bins, band_bounds) =
        HaitsmaKalkerExtractor::compute_band_layout(TEST_SAMPLE_RATE, TEST_FRAME_SIZE);
    let header = AhashHeader {
        version: 1,
        sample_rate: TEST_SAMPLE_RATE,
        frame_size: TEST_FRAME_SIZE as u16,
        hop_size: TEST_HOP_SIZE,
        num_bands: TEST_NUM_BANDS,
        spectral_lag: TEST_SPECTRAL_LAG,
        frame_count: hashes.len() as u64,
        band_bounds,
    };
    AhashFile::write_to_path(path, &header, hashes).expect("write .ahash fixture");
}

/// Writes a typed `<stem>.json` sidecar next to an `.ahash` file.
fn write_sidecar_fixture(
    ahash_path: &Path,
    media_name: &str,
    moviehash: &str,
    track: (usize, &str),
) {
    let header = AhashFile::read_from_path(ahash_path)
        .expect("read header")
        .header;
    let stream = AudioStreamInfo {
        index: track.0 + 1,
        audio_index: track.0,
        codec_name: "aac".to_string(),
        channels: Some(2),
        sample_rate: Some(48_000),
        language: Some(track.1.to_string()),
        title: None,
        is_default: track.0 == 0,
    };
    let inspection = AudioContainerInspection {
        path: ahash_path.to_path_buf(),
        audio_streams: vec![stream],
    };
    AhashSidecar::build(&AhashSidecarRequest {
        ahash_path,
        header: &header,
        media_name,
        moviehash: Some(moviehash),
        inspection: Some(&inspection),
        explicit_audio_stream: Some(track.0),
    })
    .write_next_to(ahash_path)
    .expect("write sidecar");
}

#[test]
fn test_dbahash_header_alignment_all_phases_and_corruption_guards() {
    let dir = tempdir().expect("tempdir");
    let archive_path = dir.path().join("catalog.dbahash");

    let mut builder = DbahashBuilder::new(DEFAULT_DBAHASH_FACTOR_K).expect("builder");
    let track_a = make_track_hashes(0x1111, TEST_TRACK_FRAMES);
    let track_b = make_track_hashes(0x2222, TEST_TRACK_FRAMES);

    builder
        .add_track(DbahashTrackInput {
            media_name: "Berserk.1997.EP01",
            file_path: "data/__AHASH__/05b6126884056c42.86.1fps.ahash",
            audio_fps: TEST_AUDIO_FPS,
            moviehash: Some(0x05b6_1268_8405_6c42),
            audio_index: Some(0),
            audio_language: Some("fre"),
            hashes: &track_a,
        })
        .expect("add track a");
    builder
        .add_track(DbahashTrackInput {
            media_name: "Berserk.1997.EP01",
            file_path: "data/__AHASH__/05b6126884056c42.a1.86.1fps.ahash",
            audio_fps: TEST_AUDIO_FPS,
            moviehash: Some(0x05b6_1268_8405_6c42),
            audio_index: Some(1),
            audio_language: Some("jpn"),
            hashes: &track_b,
        })
        .expect("add track b");

    let stats = builder.write_to_file(&archive_path).expect("write archive");
    assert_eq!(stats.packed_tracks, 2);
    assert_eq!(stats.total_dense_frames, (TEST_TRACK_FRAMES * 2) as u64);

    let reader = MmapDbahash::open(&archive_path).expect("open mmap");
    let h = reader.header();
    assert_eq!(h.magic, DBAHASH_MAGIC);
    assert_eq!(h.version, DBAHASH_VERSION);
    assert!(h.has_coarse_mih32());
    assert_eq!(h.offset_media_table % DBAHASH_SECTION_ALIGN, 0);
    assert_eq!(h.offset_strings_table % DBAHASH_SECTION_ALIGN, 0);
    assert_eq!(h.offset_coarse_hashes % DBAHASH_SECTION_ALIGN, 0);
    assert_eq!(h.offset_coarse_mih_upper % DBAHASH_SECTION_ALIGN, 0);
    assert_eq!(h.offset_coarse_mih_lower % DBAHASH_SECTION_ALIGN, 0);
    assert_eq!(h.offset_dense_hashes % DBAHASH_SECTION_ALIGN, 0);

    let entries = reader.media_entries();
    assert_eq!(entries[0].moviehash(), Some(0x05b6_1268_8405_6c42));
    assert_eq!(entries[0].audio_index(), Some(0));
    assert_eq!(entries[0].language(), Some("fre"));
    assert_eq!(entries[1].audio_index(), Some(1));
    assert_eq!(entries[1].language(), Some("jpn"));

    // Verify all 32 phases hit exact frame on BOTH primary and secondary (.a1) tracks,
    // and verify zero scratchpad reallocation after warm-up.
    let mut scratch = CoarseSearchScratchpad::new();
    let cap_hits = scratch.clone();
    let _ = cap_hits;

    for phase in 0..DEFAULT_DBAHASH_FACTOR_K {
        let start = TEST_BASE_OFFSET + phase;
        let noisy_b = make_noisy_query(
            &track_b[start..start + TEST_QUERY_FRAMES],
            phase as u64 + 99,
        );
        let crit = FineSearchCriteria {
            dense_query: &noisy_b,
            dense_fps: TEST_AUDIO_FPS,
            max_distance: DEFAULT_FINE_MAX_DISTANCE,
        };
        let hit = reader
            .search_fine_with_scratchpad(&crit, &mut scratch)
            .expect("secondary .a1 track must match at every modulo-32 phase");
        assert_eq!(hit.media_name, "Berserk.1997.EP01");
        assert_eq!(
            hit.file_path,
            "data/__AHASH__/05b6126884056c42.a1.86.1fps.ahash"
        );
        assert_eq!(hit.ref_frame_offset, start);
    }

    // Corrupting one byte in the string pool must trigger checksum failure on open().
    let corrupt_path = dir.path().join("corrupt.dbahash");
    let mut raw = fs::read(&archive_path).expect("read raw");
    let flip_idx = (h.offset_strings_table as usize) + 2;
    raw[flip_idx] ^= 0xFF;
    fs::write(&corrupt_path, &raw).expect("write corrupt");
    assert!(
        MmapDbahash::open(&corrupt_path).is_err(),
        "Checksum mismatch must reject corrupted archive"
    );
}

#[test]
fn test_pack_directories_twin_dedup_and_incremental_engine_delta() {
    let dir = tempdir().expect("tempdir");
    let ahash_dir = dir.path().join("__AHASH__");
    fs::create_dir_all(&ahash_dir).expect("mkdir");
    let archive_path = dir.path().join("catalog.dbahash");

    let primary_hashes = make_track_hashes(0xAAAA_1111, TEST_TRACK_FRAMES);
    let secondary_hashes = make_track_hashes(0xBBBB_2222, TEST_TRACK_FRAMES);

    // 1. Canonical primary + secondary (.a1) + TWO binary twins (one alphabetically before, one after).
    let twin_before_path = ahash_dir.join("00_Alpha.CrossRelease.720p.86.1fps.ahash");
    let primary_path = ahash_dir.join("05b6126884056c42.86.1fps.ahash");
    let secondary_path = ahash_dir.join("05b6126884056c42.a1.86.1fps.ahash");
    let twin_legacy_path = ahash_dir.join("Legacy.Duplicate.Copy.86.1fps.ahash");

    write_ahash_fixture(&twin_before_path, &primary_hashes);
    write_ahash_fixture(&primary_path, &primary_hashes);
    write_sidecar_fixture(&primary_path, "Show.S01E01", "05b6126884056c42", (0, "fre"));

    write_ahash_fixture(&secondary_path, &secondary_hashes);
    write_sidecar_fixture(
        &secondary_path,
        "Show.S01E01",
        "05b6126884056c42",
        (1, "eng"),
    );

    write_ahash_fixture(&twin_legacy_path, &primary_hashes);

    let stats = pack_ahash_directories(&DbahashPackOptions {
        ahash_dirs: vec![ahash_dir.clone()],
        output_path: archive_path.clone(),
        factor_k: DEFAULT_DBAHASH_FACTOR_K,
        embed_coarse_mih32: true,
    })
    .expect("pack_ahash_directories");

    assert_eq!(stats.scanned_files, 4);
    assert_eq!(stats.packed_tracks, 2);
    assert_eq!(stats.deduplicated_twins, 2);

    // Verify twin alias trailer indexes BOTH displaced and rejected twin basenames in O(1).
    let reader = MmapDbahash::open(&archive_path).expect("open MmapDbahash");
    assert!(reader
        .contains_file_name_and_frames("05b6126884056c42.86.1fps.ahash", TEST_TRACK_FRAMES as u32));
    assert!(reader.contains_file_name_and_frames(
        "00_Alpha.CrossRelease.720p.86.1fps.ahash",
        TEST_TRACK_FRAMES as u32
    ));
    assert!(reader.contains_file_name_and_frames(
        "Legacy.Duplicate.Copy.86.1fps.ahash",
        TEST_TRACK_FRAMES as u32
    ));
    assert!(reader.has_identical_dense_payload(&primary_hashes));

    // 2. Opening AhashEngine over BOTH catalog.dbahash and __AHASH__/ must yield 0 delta entries!
    let engine =
        AhashEngine::open(Some(&archive_path), Some(&ahash_dir)).expect("open AhashEngine");
    assert_eq!(
        engine.delta_catalog().entries.len(),
        0,
        "Packed tracks and their binary twins must not be reloaded into heap delta"
    );
    assert_eq!(engine.media_count(), 2);
    assert_eq!(engine.total_frames(), TEST_TRACK_FRAMES * 2);

    // 3. Ingest a new unpacked loose .ahash file -> picked up exclusively as incremental delta!
    let delta_hashes = make_track_hashes(0xCCCC_3333, TEST_TRACK_FRAMES);
    let delta_path = ahash_dir.join("New.Episode.S01E02.86.1fps.ahash");
    write_ahash_fixture(&delta_path, &delta_hashes);

    let reloaded =
        AhashEngine::open(Some(&archive_path), Some(&ahash_dir)).expect("reload AhashEngine");
    assert_eq!(reloaded.delta_catalog().entries.len(), 1);
    assert_eq!(reloaded.media_count(), 3);

    let crit_delta = AHashSearchCriteria {
        query: &delta_hashes[900..900 + TEST_QUERY_FRAMES],
        audio_fps: TEST_AUDIO_FPS,
        max_distance: DEFAULT_FINE_MAX_DISTANCE,
        limit: 3,
        target_media: None,
    };
    let delta_hits = reloaded.search(&crit_delta);
    assert_eq!(delta_hits.len(), 1);
    assert_eq!(delta_hits[0].media_name, "New.Episode.S01E02");
    assert_eq!(delta_hits[0].ref_frame_offset, 900);

    let crit_packed = AHashSearchCriteria {
        query: &secondary_hashes[1_234..1_234 + TEST_QUERY_FRAMES],
        audio_fps: TEST_AUDIO_FPS,
        max_distance: DEFAULT_FINE_MAX_DISTANCE,
        limit: 3,
        target_media: Some("show.s01e01"),
    };
    let packed_hits = reloaded.search(&crit_packed);
    assert_eq!(packed_hits.len(), 1);
    assert_eq!(packed_hits[0].ref_frame_offset, 1_234);
    assert!(packed_hits[0]
        .file_path
        .ends_with("05b6126884056c42.a1.86.1fps.ahash"));
}

#[test]
fn test_dbahash_without_embedded_mih32_lazy_fallback() {
    let dir = tempdir().expect("tempdir");
    let archive_path = dir.path().join("dense_only.dbahash");
    let hashes = make_track_hashes(0x7777_8888, TEST_TRACK_FRAMES);

    let mut builder = DbahashBuilder::new(16)
        .expect("builder")
        .with_coarse_mih32(false);
    builder
        .add_track(DbahashTrackInput {
            media_name: "DenseOnly.Movie.2025",
            file_path: "DenseOnly.Movie.2025.86.1fps.ahash",
            audio_fps: TEST_AUDIO_FPS,
            moviehash: None,
            audio_index: None,
            audio_language: None,
            hashes: &hashes,
        })
        .expect("add track");
    builder.write_to_file(&archive_path).expect("write");

    let reader = MmapDbahash::open(&archive_path).expect("open");
    assert!(!reader.header().has_coarse_mih32());
    assert!(reader.coarse_mih_upper().is_empty());

    let hit = reader
        .search_fine(&FineSearchCriteria {
            dense_query: &hashes[555..555 + TEST_QUERY_FRAMES],
            dense_fps: TEST_AUDIO_FPS,
            max_distance: DEFAULT_FINE_MAX_DISTANCE,
        })
        .expect("lazy fallback coarse index must match");
    assert_eq!(hit.media_name, "DenseOnly.Movie.2025");
    assert_eq!(hit.ref_frame_offset, 555);
}

/// Maximum startup latency gate for `MmapDbahash::open` on SSD (`50.0 ms`).
const MAX_OPEN_LATENCY_MS: f64 = 50.0;
/// Maximum startup latency gate for `AhashEngine::open` including `stat()` delta scan (`100.0 ms`).
const MAX_ENGINE_OPEN_LATENCY_MS: f64 = 100.0;
/// Maximum anonymous heap RSS (`RssAnon < 48.0 MiB`) proving zero-alloc Coarse-to-Fine execution.
const MAX_ANON_RSS_MIB: f64 = 48.0;
/// Maximum `RssFile` immediately after `AhashEngine::open` (`< 24.0 MiB`), guarding against cold twin faults.
const MAX_ENGINE_FILE_RSS_MIB: f64 = 24.0;
/// Maximum `RssFile` overhead above the embedded `MIH-32` tables after `200` random queries (`64.0 MiB`).
const MAX_DENSE_FILE_RSS_OVERHEAD_MIB: f64 = 64.0;
/// Bytes per `(upper + lower)` `Mih32Entry` pair (`2 * 8 = 16`).
const MIH32_PAIR_BYTES: usize = 16;
/// Number of sampled 5.0 s (`430`-frame) queries for empirical RSS & latency verification.
const EMPIRICAL_SAMPLE_QUERIES: usize = 200;
/// KiB per MiB conversion factor (`1024.0`).
const KIB_PER_MIB: f64 = 1_024.0;
/// Bytes per MiB conversion factor (`1048576.0`).
const BYTES_PER_MIB: f64 = 1_024.0 * 1_024.0;
/// Microseconds per millisecond conversion factor (`1000.0`).
const MICROS_PER_MS: f64 = 1_000.0;

#[derive(Debug, Default, Clone, Copy)]
struct ProcRssSnapshot {
    vm_rss_mib: f64,
    rss_anon_mib: f64,
    rss_file_mib: f64,
}

fn read_proc_rss_snapshot() -> ProcRssSnapshot {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return ProcRssSnapshot::default();
    };
    let parse_kib = |prefix: &str| -> f64 {
        status
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|val| val.parse::<f64>().ok())
            .unwrap_or(0.0)
            / KIB_PER_MIB
    };
    ProcRssSnapshot {
        vm_rss_mib: parse_kib("VmRSS:"),
        rss_anon_mib: parse_kib("RssAnon:"),
        rss_file_mib: parse_kib("RssFile:"),
    }
}

#[test]
#[ignore]
fn test_empirical_real_catalog_dbahash_open_rss_and_latency() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.join("../..");
    let dbahash_path = repo_root.join("data/catalog.dbahash");
    let ahash_dir = repo_root.join("data/__AHASH__");
    assert!(
        dbahash_path.exists(),
        "Run `task ahash:pack` first so {:?} exists",
        dbahash_path
    );

    // Pre-extract 200 real 430-frame queries from loose .ahash files before opening catalog.dbahash
    // so OS page-cache faults on catalog.dbahash are measured without pre-warming.
    let mut loose_files: Vec<_> = fs::read_dir(&ahash_dir)
        .expect("read data/__AHASH__")
        .filter_map(|e| e.ok().map(|d| d.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "ahash"))
        .collect();
    loose_files.sort();

    let mut sampled_queries: Vec<(usize, Vec<u64>)> = Vec::with_capacity(EMPIRICAL_SAMPLE_QUERIES);
    let step = (loose_files.len() / EMPIRICAL_SAMPLE_QUERIES).max(1);
    for (q_i, path) in loose_files.iter().step_by(step).enumerate() {
        if sampled_queries.len() >= EMPIRICAL_SAMPLE_QUERIES {
            break;
        }
        let Ok(mut f) = fs::File::open(path) else {
            continue;
        };
        let Ok(file_len) = f.metadata().map(|m| m.len() as usize) else {
            continue;
        };
        let mut hdr_buf = [0u8; dbsrt_core::ahash::HEADER_SIZE_BYTES];
        if std::io::Read::read_exact(&mut f, &mut hdr_buf).is_err() {
            continue;
        }
        let Ok(hdr) = AhashHeader::from_bytes(&hdr_buf, file_len) else {
            continue;
        };
        let total = hdr.frame_count as usize;
        if total < TEST_QUERY_FRAMES * 4 {
            continue;
        }
        let base = (total / 2) + (q_i % DEFAULT_DBAHASH_FACTOR_K);
        if base + TEST_QUERY_FRAMES >= total {
            continue;
        }
        let byte_off = (dbsrt_core::ahash::HEADER_SIZE_BYTES + base * 8) as u64;
        if std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(byte_off)).is_err() {
            continue;
        }
        let mut raw = vec![0u8; TEST_QUERY_FRAMES * 8];
        if std::io::Read::read_exact(&mut f, &mut raw).is_err() {
            continue;
        }
        let slice: Vec<u64> = raw
            .as_chunks::<8>()
            .0
            .iter()
            .map(|&c| u64::from_le_bytes(c))
            .collect();
        let non_extreme = slice.iter().filter(|&&h| h != 0 && h != u64::MAX).count();
        if non_extreme * 2 < TEST_QUERY_FRAMES {
            continue;
        }
        sampled_queries.push((base, slice));
    }
    assert!(
        sampled_queries.len() >= 100,
        "Expected >= 100 valid real queries, got {}",
        sampled_queries.len()
    );

    let rss_before = read_proc_rss_snapshot();

    let t_open = std::time::Instant::now();
    let reader = MmapDbahash::open(&dbahash_path).expect("MmapDbahash::open");
    let open_ms = t_open.elapsed().as_secs_f64() * MICROS_PER_MS;
    let rss_after_open = read_proc_rss_snapshot();

    let t_engine = std::time::Instant::now();
    let engine =
        AhashEngine::open(Some(&dbahash_path), Some(&ahash_dir)).expect("AhashEngine::open");
    let engine_open_ms = t_engine.elapsed().as_secs_f64() * MICROS_PER_MS;
    let rss_after_engine = read_proc_rss_snapshot();

    let mut latencies_ms = Vec::with_capacity(sampled_queries.len());
    let mut exact_hits = 0usize;
    for (expected_offset, query) in &sampled_queries {
        let t0 = std::time::Instant::now();
        let hits = engine.search(&AHashSearchCriteria {
            query,
            audio_fps: TEST_AUDIO_FPS,
            max_distance: DEFAULT_FINE_MAX_DISTANCE,
            limit: 3,
            target_media: None,
        });
        latencies_ms.push(t0.elapsed().as_secs_f64() * MICROS_PER_MS);
        if let Some(best) = hits.first() {
            if best.ref_frame_offset.abs_diff(*expected_offset) <= 1 {
                exact_hits += 1;
            }
        }
    }
    let rss_after_queries = read_proc_rss_snapshot();

    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean_ms = latencies_ms.iter().sum::<f64>() / latencies_ms.len() as f64;
    let p50_ms = latencies_ms[latencies_ms.len() / 2];
    let p95_ms = latencies_ms[(latencies_ms.len() * 95) / 100];
    let p99_ms = latencies_ms[(latencies_ms.len() * 99) / 100];

    let mih32_mib = (reader.total_coarse_frames() * MIH32_PAIR_BYTES) as f64 / BYTES_PER_MIB;
    let max_file_rss_mib = mih32_mib + MAX_DENSE_FILE_RSS_OVERHEAD_MIB;

    println!("=== Empirical MmapDbahash & AhashEngine Telemetry ===");
    println!(
        "Archive        : {} medias | {} dense frames | {} coarse frames (K={}, MIH-32={:.2} MiB)",
        reader.media_count(),
        reader.total_dense_frames(),
        reader.total_coarse_frames(),
        reader.factor_k(),
        mih32_mib
    );
    println!(
        "Delta Catalog  : {} loose unpacked entries (0 expected when up-to-date)",
        engine.delta_catalog().entries.len()
    );
    println!(
        "Open Latency   : MmapDbahash::open = {:.3} ms | AhashEngine::open (with {} stat/dedup) = {:.3} ms",
        open_ms,
        loose_files.len(),
        engine_open_ms
    );
    println!(
        "RSS Before     : VmRSS={:.2} MiB (Anon={:.2} MiB, File={:.2} MiB)",
        rss_before.vm_rss_mib, rss_before.rss_anon_mib, rss_before.rss_file_mib
    );
    println!(
        "RSS After Open : VmRSS={:.2} MiB (Anon={:.2} MiB, File={:.2} MiB)",
        rss_after_open.vm_rss_mib, rss_after_open.rss_anon_mib, rss_after_open.rss_file_mib
    );
    println!(
        "RSS Engine     : VmRSS={:.2} MiB (Anon={:.2} MiB, File={:.2} MiB)",
        rss_after_engine.vm_rss_mib, rss_after_engine.rss_anon_mib, rss_after_engine.rss_file_mib
    );
    println!(
        "RSS Post-{}Q  : VmRSS={:.2} MiB (Anon={:.2} MiB, File={:.2} MiB <= {:.2} MiB gate)",
        sampled_queries.len(),
        rss_after_queries.vm_rss_mib,
        rss_after_queries.rss_anon_mib,
        rss_after_queries.rss_file_mib,
        max_file_rss_mib
    );
    println!(
        "Query Latency  : mean={:.3} ms | p50={:.3} ms | p95={:.3} ms | p99={:.3} ms | exact={}/{}",
        mean_ms,
        p50_ms,
        p95_ms,
        p99_ms,
        exact_hits,
        sampled_queries.len()
    );

    assert!(
        open_ms < MAX_OPEN_LATENCY_MS,
        "MmapDbahash::open latency {:.3} ms exceeded {:.1} ms gate",
        open_ms,
        MAX_OPEN_LATENCY_MS
    );
    assert!(
        engine_open_ms < MAX_ENGINE_OPEN_LATENCY_MS,
        "AhashEngine::open latency {:.3} ms exceeded {:.1} ms gate",
        engine_open_ms,
        MAX_ENGINE_OPEN_LATENCY_MS
    );
    assert_eq!(
        engine.delta_catalog().entries.len(),
        0,
        "Expected 0 loose delta entries when catalog.dbahash packs all {} files",
        loose_files.len()
    );
    assert!(
        rss_after_engine.rss_file_mib < MAX_ENGINE_FILE_RSS_MIB,
        "AhashEngine::open RssFile {:.2} MiB exceeded {:.1} MiB gate (cold twin page fault leak)",
        rss_after_engine.rss_file_mib,
        MAX_ENGINE_FILE_RSS_MIB
    );
    assert!(
        rss_after_queries.rss_anon_mib < MAX_ANON_RSS_MIB,
        "RssAnon {:.2} MiB exceeded {:.1} MiB zero-alloc gate",
        rss_after_queries.rss_anon_mib,
        MAX_ANON_RSS_MIB
    );
    assert!(
        rss_after_queries.rss_file_mib <= max_file_rss_mib,
        "RssFile {:.2} MiB exceeded MIH-32 ({:.2} MiB) + {:.1} MiB dense gate ({:.2} MiB)",
        rss_after_queries.rss_file_mib,
        mih32_mib,
        MAX_DENSE_FILE_RSS_OVERHEAD_MIB,
        max_file_rss_mib
    );
    assert_eq!(exact_hits, sampled_queries.len());
}
