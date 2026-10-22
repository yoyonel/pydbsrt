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

#[test]
fn test_cli_pack_and_scan_dbphash() {
    let temp_dir = std::env::temp_dir().join(format!("test_cli_pack_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    // Create 2 synthetic phash files
    let phash1_path = temp_dir.join("Silicon.Valley.S01E01.1080p.24.0fps.phash");
    let phash2_path = temp_dir.join("Chernobyl.2019.Part.1.1080p.24.0fps.phash");

    let mut hashes1 = Vec::new();
    for i in 0..100 {
        hashes1.push(if i % 10 == 0 {
            0xAAAA_BBBB_CCCC_DDDD
        } else {
            0x1111_2222_3333_4444
        });
    }
    dbsrt_core::write_phash_file(&phash1_path, &hashes1).expect("Failed to write phash1");

    let mut hashes2 = Vec::new();
    for i in 0..100 {
        hashes2.push(if i % 15 == 0 {
            0x1234_5678_9ABC_DEF0
        } else {
            0x5555_6666_7777_8888
        });
    }
    dbsrt_core::write_phash_file(&phash2_path, &hashes2).expect("Failed to write phash2");

    let dbphash_out = temp_dir.join("catalog.dbphash");

    // 1. Run dbsrt pack
    let pack_out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "pack",
            "--phash-dir",
            temp_dir.to_str().unwrap(),
            "--output",
            dbphash_out.to_str().unwrap(),
            "--threshold",
            "16",
        ])
        .output()
        .expect("Failed to run dbsrt pack");

    assert!(pack_out.status.success());
    let pack_stdout = String::from_utf8_lossy(&pack_out.stdout);
    assert!(pack_stdout.contains("PACKAGING SUCCESSFUL"));
    assert!(pack_stdout.contains("Media items packed    :          2"));
    assert!(dbphash_out.exists());

    // 2. Run dbsrt scan-library --dbphash
    let scan_out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "scan-library",
            "--dbphash",
            dbphash_out.to_str().unwrap(),
            "--query-phash",
            phash1_path.to_str().unwrap(),
            "-d",
            "0",
        ])
        .output()
        .expect("Failed to run dbsrt scan-library --dbphash");

    assert!(scan_out.status.success());
    let scan_stdout = String::from_utf8_lossy(&scan_out.stdout);
    assert!(scan_stdout.contains("Loaded .dbphash index"));
    assert!(scan_stdout.contains("Top matches from .dbphash index"));
    assert!(scan_stdout.contains("Silicon Valley"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_fetch_subtitles_help() {
    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args(["fetch-subtitles", "--help"])
        .output()
        .expect("Failed to run dbsrt fetch-subtitles --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Fetch subtitles from remote providers"));
    assert!(stdout.contains("--query"));
    assert!(stdout.contains("--language"));
    assert!(stdout.contains("--dry-run"));
}

#[test]
fn test_cli_fetch_subtitles_mock_dry_run() {
    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "fetch-subtitles",
            "--query",
            "Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA.mkv",
            "--language",
            "fr",
            "--provider",
            "mock",
            "--dry-run",
        ])
        .output()
        .expect("Failed to run dbsrt fetch-subtitles");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("DBS-RT FETCH SUBTITLES"));
    assert!(stdout.contains("Silicon Valley"));
    assert!(stdout.contains("S02E01"));
    assert!(stdout.contains("SiGMA"));
    assert!(stdout.contains("mock"));
}

#[test]
fn test_cli_import_phash_missing_args() {
    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args(["import-phash"])
        .output()
        .expect("Failed to run dbsrt import-phash");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Either --file or --phash-dir must be specified"));
}
