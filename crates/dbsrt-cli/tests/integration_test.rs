use std::path::Path;
use std::process::Command;

#[test]
fn test_recognize_media_iso_parity() {
    let host = std::env::var("PSQL_IP").unwrap_or_else(|_| "127.0.0.1".to_string());
    let media = Path::new("../../data/big_buck_bunny_trailer_480p.webm");
    if !media.exists() {
        eprintln!("Media file not found at {:?}, skipping test", media);
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "recognize-media",
            "--media",
            media.to_str().unwrap(),
            "--search-distance",
            "0",
            "--output-format",
            "csv",
            "--db-host",
            &host,
        ])
        .output()
        .expect("Failed to execute dbsrt binary");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Connection refused")
            || stderr.contains("does not exist")
            || stderr.contains("connection error")
        {
            eprintln!("Database not ready/seeded, skipping test: {}", stderr);
            return;
        }
        panic!("dbsrt failed: {}", stderr);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .next()
        .expect("Expected at least one CSV row");
    let fields: Vec<&str> = line.split(',').collect();

    assert_eq!(fields[0], "True", "media_found must be True");
    assert_eq!(
        fields[1], "big_buck_bunny_trailer_480p",
        "media_name_search mismatch"
    );
    assert_eq!(
        fields[2], "big_buck_bunny_trailer_480p.webm.25.0fps",
        "media_name_match mismatch"
    );
    assert_eq!(fields[3], "1", "media_id must be 1");
    assert_eq!(fields[4], "224", "nb_offsets_match must be 224");
    assert!(line.contains("1.0"), "confidence must be 1.0");
}
