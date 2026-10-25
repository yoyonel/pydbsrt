use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dbsrt_test_e2e_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("Failed to create temp test directory");
        Self { path }
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn find_fixture(relative_path: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(relative_path),
        PathBuf::from("../..").join(relative_path),
        PathBuf::from("..").join(relative_path),
    ];
    for c in &candidates {
        if c.exists() {
            return c.canonicalize().ok();
        }
    }
    None
}

#[test]
fn test_cli_media_add_help() {
    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args(["media:add", "--help"])
        .output()
        .expect("Failed to run dbsrt media:add --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("media:add"));
    assert!(stdout.contains("--video"));
    assert!(stdout.contains("--skip-certify"));
}

#[test]
fn test_cli_media_add_dry_run_skip_certify() {
    let video = match find_fixture("tests/data/big_buck_bunny_trailer_480p.mp4") {
        Some(v) => v,
        None => return,
    };
    let srt = match find_fixture("tests/data/big_buck_bunny_trailer_480p.fr.srt") {
        Some(s) => s,
        None => return,
    };

    let temp = TempTestDir::new("dry_run");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            "--video",
            video.to_str().unwrap(),
            "--srt",
            srt.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
            "--skip-certify",
            "--json",
        ])
        .output()
        .expect("Failed to run dbsrt media:add dry run");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let val: serde_json::Value = serde_json::from_str(&stdout).expect("Must parse JSON summary");
    assert_eq!(val["success"], true);
    assert_eq!(val["certified_couplings"], 0);
    assert_eq!(val["skipped_idempotent"], 0);
    // Couplings.json should NOT exist
    assert!(!subtitles_dir.join("couplings.json").exists());
}

#[test]
fn test_cli_media_add_e2e_nominal_and_idempotent() {
    let video = match find_fixture("tests/data/big_buck_bunny_trailer_480p.mp4") {
        Some(v) => v,
        None => return,
    };
    let srt = match find_fixture("tests/data/big_buck_bunny_trailer_480p.fr.srt") {
        Some(s) => s,
        None => return,
    };

    let temp = TempTestDir::new("nominal_idempotent");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    // Pass 1: Initial ingestion
    let out1 = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            "--video",
            video.to_str().unwrap(),
            "--srt",
            srt.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to run pass 1 media:add");

    assert!(
        out1.status.success(),
        "Pass 1 failed: {:?}",
        String::from_utf8_lossy(&out1.stderr)
    );
    let json1: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out1.stdout)).expect("JSON parse");
    assert_eq!(json1["success"], true);
    assert_eq!(json1["certified_couplings"], 1);
    assert_eq!(json1["skipped_idempotent"], 0);

    // couplings.json must exist
    let reg_file = subtitles_dir.join("couplings.json");
    assert!(reg_file.exists());

    // Pass 2: Re-running on identical media/srt must be idempotent (skip)
    let out2 = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            "--video",
            video.to_str().unwrap(),
            "--srt",
            srt.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to run pass 2 media:add");

    assert!(
        out2.status.success(),
        "Pass 2 failed: {:?}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let json2: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out2.stdout)).expect("JSON parse");
    assert_eq!(json2["success"], true);
    assert_eq!(json2["certified_couplings"], 0);
    assert_eq!(json2["skipped_idempotent"], 1);
}

#[test]
fn test_cli_media_add_synthetic_mkv_with_auto_stem_pairing() {
    // Generate 1-second synthetic 24fps MKV using ffmpeg if available
    let ffmpeg_check = Command::new("ffmpeg").arg("-version").output();
    if ffmpeg_check.is_err() || !ffmpeg_check.unwrap().status.success() {
        eprintln!("ffmpeg not available on host, skipping synthetic MKV test");
        return;
    }

    let temp = TempTestDir::new("synthetic_mkv");
    let mkv_path = temp.path.join("synth_trailer.mkv");
    let srt_path = temp.path.join("synth_trailer.fr.srt");

    // Generate 1s test video via ffmpeg testsrc
    let gen_status = Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=160x120:rate=24",
            "-c:v",
            "libx264",
            "-y",
            mkv_path.to_str().unwrap(),
        ])
        .output();

    if gen_status.is_err() || !gen_status.unwrap().status.success() {
        eprintln!("Failed to generate synthetic test MKV, skipping");
        return;
    }

    // Write matching sidecar subtitle
    fs::write(&srt_path, "1\n00:00:00,100 --> 00:00:00,800\nHello World\n").unwrap();

    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    // Test media:add passing ONLY the video file without --srt
    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            mkv_path.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("Failed to run media:add on synthetic MKV");

    assert!(
        out.status.success(),
        "MKV test failed: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("JSON parse");
    assert_eq!(json["success"], true);
    assert_eq!(json["certified_couplings"], 1);
}
