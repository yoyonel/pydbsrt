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

/// [EMPIRICALLY PROVEN] Verifies automatic extraction and certification of embedded MKV subtitles
/// when no external sidecar .srt file exists. Requires ffmpeg binary.
#[test]
fn test_cli_media_add_mkv_with_embedded_subtitles() {
    let ffmpeg_check = Command::new("ffmpeg").arg("-version").output();
    if ffmpeg_check.is_err() || !ffmpeg_check.unwrap().status.success() {
        eprintln!("ffmpeg not available, skipping embedded subtitles test");
        return;
    }

    let temp = TempTestDir::new("embedded_subs");
    let mkv_path = temp.path.join("movie_with_subs.mkv");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    // Generate 2s MKV with embedded SRT subtitle track
    let srt_content = "1\n00:00:00,500 --> 00:00:01,500\nHello World\n";
    let srt_tmp = temp.path.join("input.srt");
    fs::write(&srt_tmp, srt_content).unwrap();

    let gen_status = Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=160x120:rate=24",
            "-f",
            "srt",
            "-i",
            srt_tmp.to_str().unwrap(),
            "-c:v",
            "libx264",
            "-c:s",
            "srt",
            "-metadata:s:s:0",
            "language=fre",
            "-y",
            mkv_path.to_str().unwrap(),
        ])
        .output();

    if gen_status.is_err() || !gen_status.unwrap().status.success() {
        eprintln!("Failed to generate MKV with embedded subs, skipping");
        return;
    }

    // Run media:add WITHOUT --srt (should auto-extract from container)
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
        .expect("Failed to run media:add on MKV with embedded subs");

    assert!(
        out.status.success(),
        "Embedded subs test failed: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("JSON parse");
    assert_eq!(json["success"], true);
    assert_eq!(json["certified_couplings"], 1);
}

/// [EMPIRICALLY PROVEN] Verifies multi-language embedded track extraction (fr + en).
#[test]
fn test_cli_media_add_mkv_multi_lang_tracks() {
    let ffmpeg_check = Command::new("ffmpeg").arg("-version").output();
    if ffmpeg_check.is_err() || !ffmpeg_check.unwrap().status.success() {
        eprintln!("ffmpeg not available, skipping embedded multi-lang test");
        return;
    }

    let temp = TempTestDir::new("embedded_multi_lang");
    let mkv_path = temp.path.join("movie_multi_lang.mkv");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    let srt_fr = temp.path.join("input_fr.srt");
    let srt_en = temp.path.join("input_en.srt");
    fs::write(
        &srt_fr,
        "1\n00:00:00,500 --> 00:00:01,500\nBonjour le monde\n",
    )
    .unwrap();
    fs::write(&srt_en, "1\n00:00:00,500 --> 00:00:01,500\nHello World\n").unwrap();

    let gen_status = Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=160x120:rate=24",
            "-f",
            "srt",
            "-i",
            srt_fr.to_str().unwrap(),
            "-f",
            "srt",
            "-i",
            srt_en.to_str().unwrap(),
            "-c:v",
            "libx264",
            "-c:s",
            "srt",
            "-map",
            "0:v:0",
            "-map",
            "1:s:0",
            "-map",
            "2:s:0",
            "-metadata:s:s:0",
            "language=fre",
            "-metadata:s:s:1",
            "language=eng",
            "-y",
            mkv_path.to_str().unwrap(),
        ])
        .output();

    if gen_status.is_err() || !gen_status.unwrap().status.success() {
        eprintln!("Failed to generate multi-lang MKV, skipping");
        return;
    }

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
        .expect("Failed to run media:add on multi-lang MKV");

    assert!(
        out.status.success(),
        "Multi-lang MKV failed: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("JSON parse");
    assert_eq!(json["success"], true);
    assert_eq!(json["certified_couplings"], 2);
}

/// [EMPIRICALLY PROVEN] Verifies duplicate language embedded track deduplication.
#[test]
fn test_cli_media_add_mkv_duplicate_lang_tracks() {
    let ffmpeg_check = Command::new("ffmpeg").arg("-version").output();
    if ffmpeg_check.is_err() || !ffmpeg_check.unwrap().status.success() {
        eprintln!("ffmpeg not available, skipping embedded duplicate-lang test");
        return;
    }

    let temp = TempTestDir::new("embedded_dup_lang");
    let mkv_path = temp.path.join("movie_dup_lang.mkv");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    let srt_tmp = temp.path.join("input.srt");
    fs::write(
        &srt_tmp,
        "1\n00:00:00,500 --> 00:00:01,500\nBonjour le monde\n",
    )
    .unwrap();

    let gen_status = Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=160x120:rate=24",
            "-f",
            "srt",
            "-i",
            srt_tmp.to_str().unwrap(),
            "-f",
            "srt",
            "-i",
            srt_tmp.to_str().unwrap(),
            "-c:v",
            "libx264",
            "-c:s",
            "srt",
            "-map",
            "0:v:0",
            "-map",
            "1:s:0",
            "-map",
            "2:s:0",
            "-metadata:s:s:0",
            "language=fre",
            "-metadata:s:s:1",
            "language=fre",
            "-y",
            mkv_path.to_str().unwrap(),
        ])
        .output();

    if gen_status.is_err() || !gen_status.unwrap().status.success() {
        eprintln!("Failed to generate duplicate-lang MKV, skipping");
        return;
    }

    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            mkv_path.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run media:add on duplicate-lang MKV");

    assert!(
        out.status.success(),
        "Duplicate-lang MKV failed: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Couplings Added  : 1"),
        "Expected stdout to report 1 coupling added, got: {}",
        stdout
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Skipping duplicate"),
        "Expected stderr to contain 'Skipping duplicate', got: {}",
        stderr
    );
}

/// [EMPIRICALLY PROVEN] Verifies error behavior on MKV with no subtitle tracks.
#[test]
fn test_cli_media_add_mkv_no_subtitle_track() {
    let ffmpeg_check = Command::new("ffmpeg").arg("-version").output();
    if ffmpeg_check.is_err() || !ffmpeg_check.unwrap().status.success() {
        eprintln!("ffmpeg not available, skipping no-subtitles MKV test");
        return;
    }

    let temp = TempTestDir::new("embedded_no_subs");
    let mkv_path = temp.path.join("movie_no_subs.mkv");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    let gen_status = Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=160x120:rate=24",
            "-c:v",
            "libx264",
            "-y",
            mkv_path.to_str().unwrap(),
        ])
        .output();

    if gen_status.is_err() || !gen_status.unwrap().status.success() {
        eprintln!("Failed to generate video-only MKV, skipping");
        return;
    }

    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            mkv_path.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run media:add on video-only MKV");

    assert!(
        !out.status.success(),
        "Expected media:add to fail on video without subtitles"
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No matching .srt file found"),
        "Expected stderr to contain 'No matching .srt file found', got: {}",
        stderr
    );
}

/// [EMPIRICALLY PROVEN] Verifies sidecar subtitle discovery without language suffix (Euphoria scenario).
#[test]
fn test_cli_media_add_sidecar_no_lang_suffix() {
    let ffmpeg_check = Command::new("ffmpeg").arg("-version").output();
    if ffmpeg_check.is_err() || !ffmpeg_check.unwrap().status.success() {
        eprintln!("ffmpeg not available, skipping sidecar no lang suffix test");
        return;
    }

    let temp = TempTestDir::new("sidecar_no_lang");
    let stem = "Show.US.S01E01.1080p.WEBRip.x265-RARBG";
    let video_path = temp.path.join(format!("{}.mp4", stem));
    let srt_path = temp.path.join(format!("{}.srt", stem));
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    let gen_status = Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=160x120:rate=24",
            "-c:v",
            "libx264",
            "-y",
            video_path.to_str().unwrap(),
        ])
        .output();

    if gen_status.is_err() || !gen_status.unwrap().status.success() {
        eprintln!("Failed to generate synthetic video, skipping");
        return;
    }

    fs::write(
        &srt_path,
        "1\n00:00:00,100 --> 00:00:00,800\nEuphoria scene line\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            video_path.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run media:add on sidecar without lang suffix");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "Expected media:add to succeed, stderr: {}",
        stderr
    );
    assert!(
        stderr.contains("Found 1 sidecar subtitle(s)"),
        "Expected stderr to contain 'Found 1 sidecar subtitle(s)', got: {}",
        stderr
    );
    assert!(
        stderr.contains("Show.US.S01E01"),
        "Expected stderr to contain 'Show.US.S01E01', got: {}",
        stderr
    );
    assert!(
        stdout.contains("Couplings Added  : 1"),
        "Expected stdout to report 1 coupling added, got: {}",
        stdout
    );
}

#[test]
fn test_cli_media_add_unknown_lang_certified_as_und() {
    let video = match find_fixture("tests/data/big_buck_bunny_trailer_480p.mp4") {
        Some(v) => v,
        None => return,
    };

    let temp = TempTestDir::new("unknown_lang");
    let srt_path = temp.path.join("random_name.srt");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    fs::write(
        &srt_path,
        "1\n00:00:01,000 --> 00:00:03,000\nUnknown language cue\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            "--video",
            video.to_str().unwrap(),
            "--srt",
            srt_path.to_str().unwrap(),
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run media:add with unknown language");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "Expected success exit code, stderr: {}",
        stderr
    );
    assert!(
        stderr.contains("Language could not be determined"),
        "Expected warning about language determination, got: {}",
        stderr
    );
    assert!(
        stderr.contains("certified with lang=\"und\""),
        "Expected note about certified with lang=\"und\", got: {}",
        stderr
    );
    assert!(
        stdout.contains("Couplings Added  : 1"),
        "Expected 1 coupling added, got: {}",
        stdout
    );

    let couplings_file = subtitles_dir.join("couplings.json");
    assert!(couplings_file.exists());
    let content = fs::read_to_string(&couplings_file).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    let sub = &json["couplings"][0]["subtitles"][0];
    assert_eq!(sub["lang"], "und");
    assert_eq!(sub["lang_status"]["kind"], "Unknown");
}

#[test]
fn test_cli_media_add_confirmed_lang_overrides_unknown() {
    let video = match find_fixture("tests/data/big_buck_bunny_trailer_480p.mp4") {
        Some(v) => v,
        None => return,
    };

    let temp = TempTestDir::new("confirmed_lang");
    let srt_path = temp.path.join("random_name.srt");
    let phash_dir = temp.path.join("phash");
    let subtitles_dir = temp.path.join("subtitles");

    fs::write(
        &srt_path,
        "1\n00:00:01,000 --> 00:00:03,000\nConfirmed language cue\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_dbsrt"))
        .args([
            "media:add",
            "--video",
            video.to_str().unwrap(),
            "--srt",
            srt_path.to_str().unwrap(),
            "--lang",
            "fr",
            "--phash-dir",
            phash_dir.to_str().unwrap(),
            "--subtitles-dir",
            subtitles_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run media:add with confirmed language override");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "Expected success exit code, stderr: {}",
        stderr
    );
    assert!(
        !stderr.contains("could not be determined"),
        "Expected no unknown language warning, got: {}",
        stderr
    );
    assert!(
        stdout.contains("Couplings Added  : 1"),
        "Expected 1 coupling added, got: {}",
        stdout
    );

    let couplings_file = subtitles_dir.join("couplings.json");
    assert!(couplings_file.exists());
    let content = fs::read_to_string(&couplings_file).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    let sub = &json["couplings"][0]["subtitles"][0];
    assert_eq!(sub["lang"], "fr");
    assert_eq!(sub["lang_status"]["kind"], "Confirmed");
    assert_eq!(sub["lang_status"]["value"], "fr");
}
