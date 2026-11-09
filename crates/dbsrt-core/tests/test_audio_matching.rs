//! Empirical validation of Haitsma-Kalker audio fingerprinting for media recognition & offset detection.

use dbsrt_core::{
    extract_ahash, match_audio_sequence, AudioExtractionOptions, DEFAULT_MAX_AUDIO_MATCH_DISTANCE,
};
use std::path::PathBuf;

#[test]
fn test_audio_recognition_and_offset_detection_on_real_media() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let media_path = manifest_dir.join("../../tests/data/big_buck_bunny_trailer_480p.mp4");
    if !media_path.exists() {
        eprintln!("Skipping test: test fixture not found at {:?}", media_path);
        return;
    }

    // 1. Extract full reference audio stream (33s at 86.1 fps)
    let ref_opts = AudioExtractionOptions {
        sample_rate: 5512,
        hop_size: 64,
        start_sec: 0.0,
        duration_sec: None,
        ..Default::default()
    };
    let ref_file = extract_ahash(&media_path, &ref_opts).expect("Extract reference ahash");
    let fps = ref_file.header.audio_fps();
    assert!(
        ref_file.hashes.len() > 2000,
        "Should have >2000 audio hashes for 33s"
    );

    // 2. Extract an intentional 6.0s probe query starting at t = 14.500s
    let target_offset_sec = 14.5;
    let query_duration_sec = 6.0;
    let query_opts = AudioExtractionOptions {
        sample_rate: 5512,
        hop_size: 64,
        start_sec: target_offset_sec,
        duration_sec: Some(query_duration_sec),
        ..Default::default()
    };
    let query_file = extract_ahash(&media_path, &query_opts).expect("Extract query ahash");
    assert!(!query_file.hashes.is_empty());

    // 3. Match probe against full reference
    let match_opt = match_audio_sequence(
        &query_file.hashes,
        &ref_file.hashes,
        fps,
        DEFAULT_MAX_AUDIO_MATCH_DISTANCE,
    );

    assert!(
        match_opt.is_some(),
        "Audio matcher must identify the media!"
    );
    let m = match_opt.unwrap();

    println!(
        "Audio Match Result: ref_offset_sec={:.3}s (ground-truth={:.3}s), mean_dist={:.2} bits, score={:.4}, SNR={:.1} bits",
        m.offset_sec, target_offset_sec, m.mean_distance, m.score, m.snr_bits
    );

    // Delta between detected offset and ground truth
    let error_ms = (m.offset_sec - target_offset_sec).abs() * 1000.0;
    println!(
        "Offset detection error: {:.2} ms (hop resolution = {:.2} ms)",
        error_ms,
        1000.0 / fps
    );

    // Error must be within 1 audio hop window (~11.6 ms)
    assert!(
        error_ms <= 15.0,
        "Offset error must be <= 15ms, got {:.2}ms",
        error_ms
    );
    // Mean Hamming distance must be extremely low (< 3 bits)
    assert!(
        m.mean_distance <= 3.0,
        "Hamming distance must be <= 3.0 bits, got {}",
        m.mean_distance
    );
    // SNR margin over noise must be >= 28 bits
    assert!(
        m.snr_bits >= 28.0,
        "SNR must be >= 28 bits, got {}",
        m.snr_bits
    );

    // 4. Test false positive rejection on uncorrelated audio
    let synthetic_noise: Vec<u64> = (0..query_file.hashes.len())
        .map(|i| (i as u64).wrapping_mul(0x9e3779b97f4a7c15))
        .collect();
    let false_match = match_audio_sequence(
        &synthetic_noise,
        &ref_file.hashes,
        fps,
        DEFAULT_MAX_AUDIO_MATCH_DISTANCE,
    );
    assert!(false_match.is_none(), "Unrelated audio MUST be rejected!");
}
