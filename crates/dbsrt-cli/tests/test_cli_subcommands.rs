use std::path::{Path, PathBuf};
use std::process::Command;

fn get_canonical_demo_media() -> Option<PathBuf> {
    let candidates = [
        Path::new("data/big_buck_bunny_trailer_480p.webm"),
        Path::new("../../data/big_buck_bunny_trailer_480p.webm"),
    ];
    for c in &candidates {
        if c.exists() {
            if let Ok(canon) = c.canonicalize() {
                return Some(canon);
            }
        }
    }
    None
}

#[test]
fn test_cli_help_and_version() {
    let help_out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .arg("--help")
        .output()
        .expect("Failed to run dbsrt --help");
    assert!(help_out.status.success());
    let stdout = String::from_utf8_lossy(&help_out.stdout);
    assert!(stdout.contains("High-performance video perceptual hash"));
    assert!(stdout.contains("bench-video"));
    assert!(stdout.contains("recognize-media"));

    let ver_out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .arg("--version")
        .output()
        .expect("Failed to run dbsrt --version");
    assert!(ver_out.status.success());
}

#[test]
fn test_cli_bench_phash() {
    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args(["bench-phash", "--count", "5000"])
        .output()
        .expect("Failed to run bench-phash");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("pHash throughput") || stdout.contains("Throughput"));
}

#[test]
fn test_cli_bench_video_all_modes() {
    let media = match get_canonical_demo_media() {
        Some(m) => m,
        None => {
            eprintln!("Media file not found, skipping test");
            return;
        }
    };

    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "bench-video",
            "--media",
            media.to_str().unwrap(),
            "--duration",
            "2",
        ])
        .output()
        .expect("Failed to run bench-video");
    assert!(
        out.status.success(),
        "bench-video failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Benchmarking video decoding"));
    assert!(stdout.contains("CPU Full"));
    assert!(stdout.contains("KeyFrames Only"));
}

#[test]
fn test_cli_export_bench_cuts_and_scan_library() {
    let media = match get_canonical_demo_media() {
        Some(m) => m,
        None => {
            eprintln!("Media file not found, skipping test");
            return;
        }
    };

    let temp_dir = std::env::temp_dir().join(format!("test_cli_cuts_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let phash_path = temp_dir.join("demo_trailer.phash");

    // 1. export-phash
    let out_export = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "export-phash",
            "--media",
            media.to_str().unwrap(),
            "-o",
            phash_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run export-phash");
    assert!(
        out_export.status.success(),
        "export-phash failed: {}",
        String::from_utf8_lossy(&out_export.stderr)
    );
    assert!(phash_path.exists());

    // 2. bench-cuts
    let out_cuts = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args(["bench-cuts", "--phash-dir", temp_dir.to_str().unwrap()])
        .output()
        .expect("Failed to run bench-cuts");
    assert!(
        out_cuts.status.success(),
        "bench-cuts failed: {}",
        String::from_utf8_lossy(&out_cuts.stderr)
    );

    // 3. scan-library
    let out_scan = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "scan-library",
            "--phash-dir",
            temp_dir.to_str().unwrap(),
            "--query-phash",
            phash_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run scan-library");
    assert!(
        out_scan.status.success(),
        "scan-library failed: {}",
        String::from_utf8_lossy(&out_scan.stderr)
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_missing_args_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args(["bench-video"])
        .output()
        .expect("Failed to run bench-video without args");
    assert!(!out.status.success());
}

#[test]
fn test_cli_inspect_media() {
    let media = match get_canonical_demo_media() {
        Some(m) => m,
        None => {
            eprintln!("Media file not found, skipping test");
            return;
        }
    };

    // 1. Text mode
    let out_text = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args(["inspect-media", "--media", media.to_str().unwrap()])
        .output()
        .expect("Failed to run inspect-media text");
    assert!(out_text.status.success());
    let stdout_text = String::from_utf8_lossy(&out_text.stdout);
    assert!(stdout_text.contains("Media Release Metadata"));
    assert!(stdout_text.contains("Clean Title:"));
    assert!(stdout_text.contains("Container Subtitle Tracks"));

    // 2. JSON mode
    let out_json = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "inspect-media",
            "--media",
            media.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("Failed to run inspect-media json");
    assert!(out_json.status.success());
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    assert!(stdout_json.contains("\"clean_title\""));
    assert!(stdout_json.contains("\"subtitle_streams\""));
}
