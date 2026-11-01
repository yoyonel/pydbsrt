//! OpenSubtitles moviehash vectors, shared with the batch worker (tests/tools/test_o2switch_batch_worker_moviehash.py):
//! both sides must name the same .phash for the same bytes.

use dbsrt_core::metadata::compute_opensubtitles_hash;

/// Same deterministic content as the Python test: byte i is i % 251.
fn write_pattern_file(name: &str, size: usize) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "dbsrt_moviehash_{}_{}.mkv",
        std::process::id(),
        name
    ));
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, data).expect("write pattern file");
    path
}

#[test]
fn test_moviehash_vectors_match_the_worker() {
    // 100_000 bytes: head and tail chunks overlap, as in the reference algorithm
    for (name, size, expected) in [
        ("large", 200_000, "e19d5212c9812cd6"),
        ("overlap", 100_000, "e59c5111c783a536"),
        ("one_chunk", 65_536, "d48b4000b76d0d86"),
    ] {
        let path = write_pattern_file(name, size);
        let hash = compute_opensubtitles_hash(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(hash.expect("hash"), expected, "{name}");
    }
}

#[test]
fn test_moviehash_rejects_files_under_one_chunk() {
    let path = write_pattern_file("small", 65_535);
    let hash = compute_opensubtitles_hash(&path);
    std::fs::remove_file(&path).ok();
    assert!(hash.is_err());
}
