use dbsrt_core::{
    detect_cuts_timestamped, extract_frames_advanced, extract_keyframes_phash,
    extract_sparse_phash, probe_video, HwAccel, SamplingMode, VideoExtractionOptions,
};
use std::path::PathBuf;

fn get_test_video() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/data/big_buck_bunny_trailer_480p.mp4")
}

#[test]
fn test_probe_video() {
    let video = get_test_video();
    let meta = probe_video(&video).expect("probe_video failed");
    assert!(meta.duration > 30.0 && meta.duration < 35.0);
    assert!((meta.fps - 25.0).abs() < 0.1);
}

#[test]
fn test_sparse_fps_extraction() {
    let video = get_test_video();
    let samples = extract_sparse_phash(&video, 1.0).expect("sparse extraction failed");

    // Video is ~33 seconds -> 1 fps should produce ~32-34 samples
    assert!(
        samples.len() >= 30 && samples.len() <= 35,
        "Expected ~33 frames, got {}",
        samples.len()
    );

    // Timestamps should increase by roughly 1000ms
    for (i, sample) in samples.iter().enumerate() {
        assert_eq!(sample.frame_idx, i);
        let expected_ms = (i as u64) * 1000;
        assert!(
            (sample.timestamp_ms as i64 - expected_ms as i64).abs() <= 50,
            "Timestamp mismatch at index {}: got {}, expected {}",
            i,
            sample.timestamp_ms,
            expected_ms
        );
    }
}

#[test]
fn test_keyframes_extraction_and_cuts() {
    let video = get_test_video();
    let keyframes = extract_keyframes_phash(&video).expect("keyframe extraction failed");

    assert!(
        keyframes.len() >= 10,
        "Expected at least 10 keyframes, got {}",
        keyframes.len()
    );

    // Ensure timestamps are monotonic
    for window in keyframes.windows(2) {
        assert!(
            window[1].timestamp_ms >= window[0].timestamp_ms,
            "Timestamps must be non-decreasing: {} vs {}",
            window[0].timestamp_ms,
            window[1].timestamp_ms
        );
    }

    // Detect timestamped cuts
    let cuts = detect_cuts_timestamped(&keyframes, 10);
    assert!(
        !cuts.is_empty(),
        "Should detect scene cuts across keyframes"
    );
    for cut in &cuts {
        assert!(cut.phash != 0);
    }
}

#[test]
fn test_cpu_and_hwaccel_equivalence() {
    let video = get_test_video();

    // CPU decode first 10 frames from natural scene at 15.0s
    let opts_cpu = VideoExtractionOptions {
        start_sec: 15.0,
        max_frames: Some(10),
        sampling: SamplingMode::Full,
        hwaccel: HwAccel::Disabled,
        ..Default::default()
    };
    let samples_cpu = extract_frames_advanced(&video, &opts_cpu).expect("CPU extraction failed");
    assert_eq!(samples_cpu.len(), 10);

    // Auto / HWAccel decode first 10 frames
    let opts_hw = VideoExtractionOptions {
        start_sec: 15.0,
        max_frames: Some(10),
        sampling: SamplingMode::Full,
        hwaccel: HwAccel::Auto,
        ..Default::default()
    };
    let samples_hw = extract_frames_advanced(&video, &opts_hw).expect("HW extraction failed");
    assert_eq!(samples_hw.len(), 10);

    // Verify Hamming distance between corresponding frames is within perceptual similarity threshold (<= 10 bits)
    // Differences arise due to GPU hardware polyphase scaler vs CPU swscale bicubic scaler
    for (i, (c, h)) in samples_cpu.iter().zip(samples_hw.iter()).enumerate() {
        let dist = dbsrt_core::hamming_distance(c.phash, h.phash);
        println!(
            "Frame {}: dist={}, cpu={:016x}, hw={:016x}",
            i, dist, c.phash, h.phash
        );
        assert!(
            dist <= 10,
            "Frame {} mismatch: dist={}, cpu={:016x}, hw={:016x}",
            i,
            dist,
            c.phash,
            h.phash
        );
    }
}

#[test]
fn test_hwaccel_automatic_fallback_to_cpu() {
    let video = get_test_video();

    // Requesting CUDA on an Intel-only machine should automatically fallback to CPU
    let opts = VideoExtractionOptions {
        start_sec: 15.0,
        max_frames: Some(5),
        sampling: SamplingMode::Full,
        hwaccel: HwAccel::Cuda,
        ..Default::default()
    };

    let samples = extract_frames_advanced(&video, &opts).expect("Fallback to CPU should succeed");
    assert_eq!(samples.len(), 5);
}
