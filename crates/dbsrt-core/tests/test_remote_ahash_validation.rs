//! Test verifying empirical validity and recognition with .ahash files generated on o2switch.

use dbsrt_core::ahash::AhashFile;
use dbsrt_core::{match_audio_sequence, DEFAULT_MAX_AUDIO_MATCH_DISTANCE};
use std::path::PathBuf;

#[test]
fn test_remote_pulled_ahash_files_integrity_and_recognition() {
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

    if test_files.is_empty() {
        eprintln!("No .ahash files found in data/__AHASH__, skipping test.");
        return;
    }

    println!(
        "Testing {} .ahash file(s) from o2switch batch:",
        test_files.len()
    );

    let mut loaded_files = Vec::new();

    for path in &test_files {
        let file_name = path.file_name().unwrap().to_string_lossy();
        println!("--> Validating .ahash format for: {}", file_name);

        let ahash_file = AhashFile::read_from_path(path)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {}", file_name, e));

        assert_eq!(ahash_file.header.version, 1);
        assert_eq!(ahash_file.header.sample_rate, 5512);
        assert_eq!(ahash_file.header.hop_size, 64);
        assert_eq!(ahash_file.header.num_bands, 33);
        assert_eq!(
            ahash_file.header.frame_count as usize,
            ahash_file.hashes.len()
        );

        let fps = ahash_file.header.audio_fps();
        assert!((fps - 86.125).abs() < 0.1);
        println!(
            "    ✓ Header valid: {} frames ({:.1}s duration) at {:.2} fps",
            ahash_file.hashes.len(),
            ahash_file.hashes.len() as f64 / fps,
            fps
        );

        // Self-recognition test on a 10-second slice (861 frames) extracted from the middle
        let query_len = 861.min(ahash_file.hashes.len() / 2);
        let start_offset = ahash_file.hashes.len() / 4;
        let query_slice = &ahash_file.hashes[start_offset..start_offset + query_len];

        let match_res = match_audio_sequence(
            query_slice,
            &ahash_file.hashes,
            fps,
            DEFAULT_MAX_AUDIO_MATCH_DISTANCE,
        )
        .expect("Self-recognition must match the target stream");

        assert_eq!(match_res.ref_frame_offset, start_offset);
        assert_eq!(match_res.mean_distance, 0.0);
        assert_eq!(match_res.score, 1.0);
        assert!(match_res.snr_bits >= 31.0);

        println!(
            "    ✓ Self-recognition verified: exact offset detected at frame {} (distance=0.0 bits, score=1.0)",
            match_res.ref_frame_offset
        );

        loaded_files.push((file_name.to_string(), ahash_file));
    }

    // Cross-recognition test: verify distinct medias reject false positives
    if loaded_files.len() >= 2 {
        let (name1, f1) = &loaded_files[0];
        let (name2, f2) = &loaded_files[1];

        let query_len = 861.min(f1.hashes.len());
        let query_slice = &f1.hashes[0..query_len];

        let cross_match = match_audio_sequence(
            query_slice,
            &f2.hashes,
            f2.header.audio_fps(),
            DEFAULT_MAX_AUDIO_MATCH_DISTANCE,
        );

        assert!(
            cross_match.is_none(),
            "Audio sequences from distinct media ({} vs {}) must NOT cross-match!",
            name1,
            name2
        );
        println!(
            "    ✓ False positive rejection verified: {} does not match {}",
            name1, name2
        );
    }
}
