use std::path::Path;
use std::process::Command;

#[test]
fn test_retarget_subtitles_e2e_iso() {
    let ref_media = Path::new("../../data/big_buck_bunny_trailer_480p.webm");
    let ref_srt = Path::new("../../data/SRT/big_buck_bunny_trailer_480p.en.srt");
    let target_media = Path::new("../../tests/data/big_buck_bunny_trailer_480p.mp4");

    if !ref_media.exists() || !ref_srt.exists() || !target_media.exists() {
        eprintln!("Test assets missing, skipping e2e retarget test");
        return;
    }

    let tmp_dir =
        std::env::temp_dir().join(format!("dbsrt_retarget_iso_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "retarget-subtitles",
            "--ref-media",
            ref_media.to_str().unwrap(),
            "--ref-subtitles",
            ref_srt.to_str().unwrap(),
            "--target-media",
            target_media.to_str().unwrap(),
            "--phash-dir",
            tmp_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute dbsrt binary");

    assert!(
        output.status.success(),
        "dbsrt retarget failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Shift subtitles seconds = 0 frames"));
    assert!(stdout.contains("Exported subtitles for target in :"));

    // Check output file (matches target_media: big_buck_bunny_trailer_480p.mp4)
    let out_srt = tmp_dir.join("big_buck_bunny_trailer_480p.mp4.25.0fps.srt");
    assert!(out_srt.exists(), "Output SRT file must exist");

    let content = std::fs::read_to_string(&out_srt).expect("Failed to read output SRT");
    assert!(content.contains("The peach open movie project presents"));
    assert!(content.contains("00:00:00,005 --> 00:00:03,800"));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
