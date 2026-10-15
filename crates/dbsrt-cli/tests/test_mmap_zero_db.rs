use std::path::Path;
use std::process::Command;

#[test]
fn test_recognize_media_mmap_zero_db() {
    let media = Path::new("../../data/big_buck_bunny_trailer_480p.webm");
    if !media.exists() {
        eprintln!("Media file not found at {:?}, skipping test", media);
        return;
    }

    let tmp_dir = std::env::temp_dir();

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
}
