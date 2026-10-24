//! Chaos engineering resilience tests for dbsrt-server.
//! Validates ERR-02 (network relay timeout bounding), ERR-03 (special characters & UTF-8 paths),
//! and ERR-04 (automatic cache directory fallback when media directory is read-only).

use dbsrt_server::{
    create_router, DownloadSubtitleRequest, DownloadSubtitleResponse, SearchSubtitlesRequest,
    SearchSubtitlesResponse,
};
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const READONLY_DIR_PERMS_MODE: u32 = 0o555;
const RESTORE_DIR_PERMS_MODE: u32 = 0o755;
const MAX_SLA_WATCHDOG_LIMIT_SECS: u64 = 4;
const MOCK_HANGING_DELAY_SECS: u64 = 6;

/// ERR-04: Video directory is read-only (e.g. read-only SMB/NFS share).
/// Server must detect primary write failure and transparently fallback to ~/.cache/dbsrt/subtitles/.
#[tokio::test]
async fn test_chaos_err04_readonly_directory_fallback() {
    let tmp_root = std::env::temp_dir();
    let unique_id = std::process::id();
    let readonly_dir = tmp_root.join(format!("dbsrt_chaos_ro_{}", unique_id));
    let phash_dir = tmp_root.join(format!("dbsrt_chaos_phash_{}", unique_id));
    let srt_dir = PathBuf::from("../../data/SRT");

    let _ = std::fs::create_dir_all(&readonly_dir);
    let _ = std::fs::create_dir_all(&phash_dir);

    let test_video_src = Path::new("../../tests/data/big_buck_bunny_trailer_480p.mp4");
    let test_video_dst = readonly_dir.join("sample_movie.mp4");
    if test_video_src.exists() {
        let _ = std::fs::copy(test_video_src, &test_video_dst);
    } else {
        std::fs::write(&test_video_dst, b"dummy video content").unwrap();
    }

    // Set directory to read-only (r-xr-xr-x)
    std::fs::set_permissions(
        &readonly_dir,
        std::fs::Permissions::from_mode(READONLY_DIR_PERMS_MODE),
    )
    .expect("Failed to set directory to read-only");

    let app = create_router(phash_dir.clone(), srt_dir.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral test port");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    let sample_srt_fixture = Path::new("../../data/SRT/big_buck_bunny_trailer_480p.en.srt");
    let candidate_id = if sample_srt_fixture.exists() {
        sample_srt_fixture.to_str().unwrap().to_string()
    } else {
        "tests/data/sample.en.srt".to_string()
    };

    let dl_req = DownloadSubtitleRequest {
        media_path: test_video_dst.to_str().unwrap().to_string(),
        source: "catalog".to_string(),
        candidate_id: Some(candidate_id),
        language: "en".to_string(),
        resync: false,
        shift_ms: None,
        subtitles_dir: Some(srt_dir.to_str().unwrap().to_string()),
        phash_dir: Some(phash_dir.to_str().unwrap().to_string()),
    };

    let res = client
        .post(format!("{}/v1/download", base_url))
        .json(&dl_req)
        .send()
        .await
        .expect("POST /v1/download failed");

    // Restore write permissions for cleanup
    let _ = std::fs::set_permissions(
        &readonly_dir,
        std::fs::Permissions::from_mode(RESTORE_DIR_PERMS_MODE),
    );

    assert!(res.status().is_success());
    let dl_res: DownloadSubtitleResponse = res.json().await.expect("Failed to parse download JSON");

    assert!(
        dl_res.success,
        "Download must succeed by falling back to cache directory: {:?}",
        dl_res.error
    );
    assert!(
        !dl_res.output_srt_path.is_empty(),
        "Output SRT path must not be empty"
    );

    // Verify output path fell back to cache directory instead of readonly_dir
    let final_path = PathBuf::from(&dl_res.output_srt_path);
    assert!(
        final_path.exists(),
        "Fallback subtitle file must exist on disk: {:?}",
        final_path
    );
    assert!(
        !final_path.starts_with(&readonly_dir),
        "Final path must NOT be inside the read-only directory: {:?}",
        final_path
    );
    assert!(
        final_path.to_string_lossy().contains("subtitles"),
        "Final path must point to cache subtitles directory: {:?}",
        final_path
    );

    // Cleanup
    let _ = std::fs::remove_file(&final_path);
    let _ = std::fs::remove_file(&test_video_dst);
    let _ = std::fs::remove_dir_all(&readonly_dir);
    let _ = std::fs::remove_dir_all(&phash_dir);
}

/// ERR-02: Remote relay latency > 5s must be bounded by daemon SLA (< 4s) without hanging VLC.
#[tokio::test]
async fn test_chaos_err02_relay_timeout_bounded() {
    use axum::routing::post;
    use axum::Router;

    // 1. Spawn a deliberately slow mock relay server that delays beyond the SLA
    let hanging_relay = Router::new().route(
        "/v1/recognize",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(MOCK_HANGING_DELAY_SECS)).await;
            axum::Json(serde_json::json!({
                "found": false,
                "media_match": null,
                "confidence": 0.0
            }))
        }),
    );

    let relay_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind mock relay listener");
    let relay_addr = relay_listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(relay_listener, hanging_relay).await.unwrap();
    });

    // 2. Set DBSRT_REMOTE_URL to point to hanging relay
    std::env::set_var("DBSRT_REMOTE_URL", format!("http://{}", relay_addr));

    let tmp_root = std::env::temp_dir();
    let phash_dir = tmp_root.join(format!("dbsrt_chaos_timeout_{}", std::process::id()));
    let srt_dir = PathBuf::from("../../data/SRT");
    let _ = std::fs::create_dir_all(&phash_dir);

    let app = create_router(phash_dir.clone(), srt_dir.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind daemon listener");
    let daemon_addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", daemon_addr);

    let search_req = SearchSubtitlesRequest {
        media_path: "tests/data/big_buck_bunny_trailer_480p.mp4".to_string(),
        language: "en".to_string(),
        query_title: Some("Big Buck Bunny".to_string()),
        season: None,
        episode: None,
        subtitles_dir: Some(srt_dir.to_str().unwrap().to_string()),
        phash_dir: Some(phash_dir.to_str().unwrap().to_string()),
        search_mode: Some("phash".to_string()),
        index_engine: None,
    };

    let start_t = Instant::now();
    let res = client
        .post(format!("{}/v1/search", base_url))
        .json(&search_req)
        .send()
        .await
        .expect("POST /v1/search must complete");

    let elapsed = start_t.elapsed();

    // Reset env var
    std::env::remove_var("DBSRT_REMOTE_URL");

    assert!(res.status().is_success());
    let search_res: SearchSubtitlesResponse =
        res.json().await.expect("Failed to parse search response");

    // CRITICAL ASSERTION: The daemon must cut off the hanging relay and return before the VLC 5-second watchdog!
    assert!(
        elapsed < Duration::from_secs(MAX_SLA_WATCHDOG_LIMIT_SECS),
        "Daemon took {:?}, which violates the max SLA limit of {}s (risk of VLC watchdog trigger)",
        elapsed,
        MAX_SLA_WATCHDOG_LIMIT_SECS
    );

    // Should gracefully return local/fallback results without crashing
    assert_eq!(search_res.language, "en");

    let _ = std::fs::remove_dir_all(&phash_dir);
}

/// ERR-03: Paths with double quotes, apostrophes, ampersands, brackets and UTF-8 characters.
#[tokio::test]
async fn test_chaos_err03_special_characters_and_quotes() {
    let tmp_root = std::env::temp_dir();
    let phash_dir = tmp_root.join(format!("dbsrt_chaos_quotes_{}", std::process::id()));
    let srt_dir = PathBuf::from("../../data/SRT");
    let _ = std::fs::create_dir_all(&phash_dir);

    let app = create_router(phash_dir.clone(), srt_dir.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind daemon listener");
    let daemon_addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", daemon_addr);

    // Chaotic path with quotes, apostrophes, ampersands, brackets, UTF-8 accents
    let chaotic_path = "/media/L'été \"Spécial\" & [Action] 1080p - 2026.mkv";

    let search_req = SearchSubtitlesRequest {
        media_path: chaotic_path.to_string(),
        language: "fr".to_string(),
        query_title: Some("L'été \"Spécial\" & [Action]".to_string()),
        season: None,
        episode: None,
        subtitles_dir: Some(srt_dir.to_str().unwrap().to_string()),
        phash_dir: Some(phash_dir.to_str().unwrap().to_string()),
        search_mode: Some("phash".to_string()),
        index_engine: None,
    };

    let res = client
        .post(format!("{}/v1/search", base_url))
        .json(&search_req)
        .send()
        .await
        .expect("POST /v1/search with chaotic path must succeed");

    assert!(res.status().is_success());
    let search_res: SearchSubtitlesResponse =
        res.json().await.expect("Failed to parse search response");
    assert_eq!(search_res.language, "fr");

    let _ = std::fs::remove_dir_all(&phash_dir);
}
