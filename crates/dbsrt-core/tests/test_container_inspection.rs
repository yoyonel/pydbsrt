//! Unit & integration test suite for dbsrt-core::container.
//!
//! Tests container subtitle track probing, ISO language mapping, and in-memory SRT extraction.

use dbsrt_core::container::{extract_embedded_subtitle, inspect_container, language_matches};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn get_repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn test_language_matches() {
    // Exact
    assert!(language_matches(Some("fr"), "fr"));
    assert!(language_matches(Some("eng"), "eng"));

    // ISO 639-1 / ISO 639-2 French
    assert!(language_matches(Some("fre"), "fr"));
    assert!(language_matches(Some("fra"), "fr"));
    assert!(language_matches(Some("fr"), "fre"));
    assert!(language_matches(Some("fr"), "fra"));
    assert!(language_matches(Some("French"), "fra"));

    // English
    assert!(language_matches(Some("en"), "eng"));
    assert!(language_matches(Some("eng"), "en"));

    // Spanish / German / Japanese
    assert!(language_matches(Some("spa"), "es"));
    assert!(language_matches(Some("ger"), "de"));
    assert!(language_matches(Some("deu"), "de"));
    assert!(language_matches(Some("jpn"), "ja"));

    // Negative tests
    assert!(!language_matches(Some("eng"), "fr"));
    assert!(!language_matches(Some("fre"), "en"));
    assert!(!language_matches(None, "fr"));
}

#[test]
fn test_inspect_media_without_subtitles() {
    let root = get_repo_root();
    let video_path = root.join("tests/data/big_buck_bunny_trailer_480p.mp4");

    let inspection = inspect_container(&video_path).expect("Failed to inspect container");
    assert_eq!(inspection.path, video_path);
    assert_eq!(inspection.subtitle_streams.len(), 0);
    assert!(inspection.default_subtitle().is_none());
    assert!(inspection.find_by_language("fr").is_none());
}

#[test]
fn test_inspect_nonexistent_container() {
    let bad_path = PathBuf::from("/non/existent/path/movie.mkv");
    let res = inspect_container(&bad_path);
    assert!(res.is_err());
}

#[test]
fn test_embedded_subtitle_inspection_and_extraction() {
    let root = get_repo_root();
    let srt_path = root.join("tests/data/big_buck_bunny_trailer_480p.en.srt");
    let _original_srt = fs::read_to_string(&srt_path).expect("Failed to read test srt");

    let tmp_dir = std::env::temp_dir();
    let test_mkv = tmp_dir.join(format!("test_embedded_subs_{}.mkv", std::process::id()));

    // Generate a test MKV with 2 embedded subtitle tracks: English (default) and French
    let status = Command::new("ffmpeg")
        .stdin(std::process::Stdio::null())
        .args([
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:d=1",
            "-f",
            "srt",
            "-i",
        ])
        .arg(&srt_path)
        .args(["-f", "srt", "-i"])
        .arg(&srt_path)
        .args([
            "-map",
            "0:v",
            "-map",
            "1:s",
            "-map",
            "2:s",
            "-c:v",
            "mjpeg",
            "-c:s",
            "srt",
            "-metadata:s:1",
            "language=eng",
            "-metadata:s:1",
            "title=English Subtitles",
            "-disposition:1",
            "default",
            "-metadata:s:2",
            "language=fra",
            "-metadata:s:2",
            "title=French Subtitles",
        ])
        .arg(&test_mkv)
        .status();

    let ffmpeg_ok = matches!(status, Ok(s) if s.success());

    if !ffmpeg_ok {
        eprintln!("Warning: FFmpeg muxing not supported in this test run, skipping test");
        return;
    }

    // 1. Inspect container
    let inspection = inspect_container(&test_mkv).expect("Failed to inspect generated MKV");
    assert_eq!(inspection.subtitle_streams.len(), 2);

    let en_stream = &inspection.subtitle_streams[0];
    assert_eq!(en_stream.subtitle_index, 0);
    assert_eq!(en_stream.language.as_deref(), Some("eng"));
    assert_eq!(en_stream.title.as_deref(), Some("English Subtitles"));
    assert!(en_stream.is_default);

    let fr_stream = &inspection.subtitle_streams[1];
    assert_eq!(fr_stream.subtitle_index, 1);
    assert_eq!(fr_stream.language.as_deref(), Some("fra"));
    assert_eq!(fr_stream.title.as_deref(), Some("French Subtitles"));
    assert!(!fr_stream.is_default);

    // 2. Query methods
    let found_fr = inspection
        .find_by_language("fr")
        .expect("FR stream not found");
    assert_eq!(found_fr.subtitle_index, 1);

    let default_sub = inspection
        .default_subtitle()
        .expect("Default subtitle not found");
    assert_eq!(default_sub.subtitle_index, 0);

    // 3. Extract subtitle to memory
    let extracted_srt =
        extract_embedded_subtitle(&test_mkv, 0).expect("Failed to extract subtitle stream 0");
    assert!(!extracted_srt.is_empty());
    assert!(extracted_srt.contains("peach open movie project"));

    // Cleanup
    let _ = fs::remove_file(&test_mkv);
}
