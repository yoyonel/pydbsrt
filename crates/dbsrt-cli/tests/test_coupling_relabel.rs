//! Integration tests for dbsrt coupling:relabel CLI subcommand.
//!
//! FACT-FIRST CLASSIFICATION:
//! - Hermetically Mocked: Isolated temporary coupling registry JSON without external network or DB.

use dbsrt_core::{CertifiedSubtitleRecord, CouplingRegistry, LangStatus};
use std::fs;
use std::process::Command;

fn get_dbsrt_bin() -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(env!("CARGO_BIN_EXE_dbsrt"));
    if !path.exists() {
        path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target/debug/dbsrt");
    }
    path
}

#[test]
fn test_cli_coupling_relabel_help() {
    let bin = get_dbsrt_bin();
    let output = Command::new(&bin)
        .args(["coupling:relabel", "--help"])
        .output()
        .expect("dbsrt coupling:relabel --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--media"));
    assert!(stdout.contains("--lang"));
    assert!(stdout.contains("--sub"));
    assert!(stdout.contains("--status"));
}

#[test]
fn test_cli_coupling_relabel_e2e_json() {
    let temp_dir =
        std::env::temp_dir().join(format!("test_cli_relabel_e2e_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let registry_path = CouplingRegistry::default_path(&temp_dir);
    let mut reg = CouplingRegistry::default();
    let sub = CertifiedSubtitleRecord {
        subtitles_hash: "abcdef1234567890abcdef1234567890".to_string(),
        name: "Euphoria.S01E02.srt".to_string(),
        lang: "und".to_string(),
        lang_status: LangStatus::Unknown,
        srt_path: "Euphoria.S01E02.srt".to_string(),
        certified: true,
        cue_count: 701,
        sub_frames: vec![],
    };
    reg.upsert_coupling(
        "43d7072825f81d077638a1342c416763".to_string(),
        "Euphoria.US.S01E02.1080p.WEBRip.x265-RARBG".to_string(),
        "Euphoria.US.S01E02.1080p.WEBRip.x265-RARBG.mp4.24.0fps.phash".to_string(),
        23.976,
        1500,
        sub,
    );
    reg.save(&registry_path).unwrap();

    let bin = get_dbsrt_bin();
    let output = Command::new(&bin)
        .args([
            "coupling:relabel",
            "--media",
            "43d70728",
            "--lang",
            "fr",
            "--subtitles-dir",
            temp_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("execute coupling:relabel");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_val: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json_val["success"], true);
    assert_eq!(json_val["old_lang"], "und");
    assert_eq!(json_val["new_lang"], "fr");
    assert_eq!(json_val["new_status"]["kind"], "Confirmed");
    assert_eq!(json_val["new_status"]["value"], "fr");

    // Verify persisted registry on disk
    let reloaded = CouplingRegistry::load_or_default(&registry_path).unwrap();
    assert_eq!(reloaded.couplings[0].subtitles[0].lang, "fr");
    assert_eq!(
        reloaded.couplings[0].subtitles[0].lang_status,
        LangStatus::Confirmed("fr".to_string())
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_coupling_relabel_rejects_invalid_language() {
    let temp_dir =
        std::env::temp_dir().join(format!("test_cli_relabel_err_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let registry_path = CouplingRegistry::default_path(&temp_dir);
    let mut reg = CouplingRegistry::default();
    let sub = CertifiedSubtitleRecord {
        subtitles_hash: "11112222333344445555666677778888".to_string(),
        name: "test.srt".to_string(),
        lang: "und".to_string(),
        lang_status: LangStatus::Unknown,
        srt_path: "test.srt".to_string(),
        certified: true,
        cue_count: 10,
        sub_frames: vec![],
    };
    reg.upsert_coupling(
        "aaaa1111bbbb2222cccc3333dddd4444".to_string(),
        "TestVideo".to_string(),
        "TestVideo.phash".to_string(),
        24.0,
        100,
        sub,
    );
    reg.save(&registry_path).unwrap();

    let bin = get_dbsrt_bin();
    let output = Command::new(&bin)
        .args([
            "coupling:relabel",
            "--media",
            "aaaa1111",
            "--lang",
            "invalid_not_iso",
            "--subtitles-dir",
            temp_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute coupling:relabel invalid lang");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid language code"));

    let _ = fs::remove_dir_all(&temp_dir);
}
