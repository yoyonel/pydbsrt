use std::path::Path;
use std::process::Command;

#[test]
fn test_recognize_media_mmap_zero_db() {
    let mp4_media = Path::new("../../tests/data/big_buck_bunny_trailer_480p.mp4");
    let webm_media = Path::new("../../data/big_buck_bunny_trailer_480p.webm");
    let media = if mp4_media.exists() {
        mp4_media
    } else if webm_media.exists() {
        webm_media
    } else {
        eprintln!(
            "Media file not found at {:?} or {:?}, skipping test",
            mp4_media, webm_media
        );
        return;
    };

    let tmp_dir = std::env::temp_dir().join(format!("test_mmap_zero_db_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let stem = media.file_name().unwrap().to_str().unwrap();
    let expected_phash = tmp_dir.join(format!("{}.25.0fps.phash", stem));

    let export_output = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "export-phash",
            "--media",
            media.to_str().unwrap(),
            "-o",
            expected_phash.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute export-phash");
    assert!(
        export_output.status.success(),
        "export-phash failed: {}",
        String::from_utf8_lossy(&export_output.stderr)
    );

    let output = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "recognize-media",
            "--media",
            media.to_str().unwrap(),
            "--no-db",
            "--phash-dir",
            tmp_dir.to_str().unwrap(),
            "--search-distance",
            "0",
            "--output-format",
            "csv",
        ])
        .output()
        .expect("Failed to execute dbsrt binary");

    assert!(
        output.status.success(),
        "dbsrt zero-db failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(line) = stdout.lines().next() {
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(fields[0], "True", "media_found must be True");
        assert_eq!(
            fields[1], "big_buck_bunny_trailer_480p",
            "media_name_search mismatch"
        );
        assert!(
            fields[2].contains("big_buck_bunny_trailer_480p"),
            "media_name_match mismatch"
        );
        assert!(line.contains("1.0"), "confidence must be 1.0");
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
