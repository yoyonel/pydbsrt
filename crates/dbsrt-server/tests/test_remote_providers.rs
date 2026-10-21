use axum::http::StatusCode;
use dbsrt_core::providers::{
    cache::SubtitleCache, mock::MockSubtitleProvider, RemoteSubtitleCandidate,
};
use dbsrt_core::write_phash_file;
use dbsrt_server::{create_router_with_state, AppState, AutoSyncRequest, AutoSyncResponse};
use reqwest::Client;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;

fn make_dummy_phash_file(dir: &PathBuf, stem: &str, count: usize) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let file_path = dir.join(format!("{}.phash", stem));
    let hashes: Vec<u64> = (0..count).map(|i| (i as u64) << 16 | 0xA5A5_5A5A).collect();
    write_phash_file(&file_path, &hashes).unwrap();
    file_path
}

#[tokio::test]
async fn test_autosync_fallback_to_remote_provider_and_cache() {
    let tmp = std::env::temp_dir().join(format!(
        "dbsrt_remote_autosync_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&tmp).unwrap();

    let phash_dir = tmp.join("phash");
    let subtitles_dir = tmp.join("subtitles");
    let cache_dir = tmp.join("cache");
    fs::create_dir_all(&phash_dir).unwrap();
    fs::create_dir_all(&subtitles_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();

    // Create target media dummy video & phash
    let media_stem = "Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA";
    let media_video_path = tmp.join(format!("{}.mkv", media_stem));
    fs::write(&media_video_path, b"dummy mkv header without embedded subs").unwrap();
    make_dummy_phash_file(&phash_dir, media_stem, 100);

    // Set up mock provider
    let mock_provider = Arc::new(MockSubtitleProvider::new());
    let candidate = RemoteSubtitleCandidate {
        provider: "mock".to_string(),
        subtitle_id: "os-7890".to_string(),
        release_name: Some("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA".to_string()),
        language: "fr".to_string(),
        download_url: "mock://os-7890".to_string(),
        hearing_impaired: false,
        fps: Some(25.0),
        match_score: 0.95,
    };
    let remote_srt = "1\n00:00:01,000 --> 00:00:03,000\nBienvenue chez Pied Piper!\n";
    mock_provider.add_candidate("silicon valley", candidate, remote_srt);

    let cache = SubtitleCache::new(Some(cache_dir.clone()));

    let app_state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        mock_provider.clone(),
        cache.clone(),
    );

    let router = create_router_with_state(app_state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = Client::new();

    // Request autosync for 'fr'
    let req = AutoSyncRequest {
        media_path: media_video_path.to_string_lossy().to_string(),
        language: "fr".to_string(),
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
        subtitles_dir: Some(subtitles_dir.to_string_lossy().to_string()),
        search_distance: 16,
        nb_seconds: 1.0,
    };

    let resp = client
        .post(format!("http://{}/v1/autosync", addr))
        .json(&req)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let sync_res: AutoSyncResponse = resp.json().await.unwrap();

    // Verification:
    // 1. Success true
    assert!(
        sync_res.success,
        "autosync should succeed via remote fallback: {:?}",
        sync_res.error
    );
    assert_eq!(sync_res.language, "fr");
    assert!(!sync_res.output_srt_path.is_empty());
    assert!(PathBuf::from(&sync_res.output_srt_path).exists());

    // 2. Cache verification: subtitle was persisted to local cache
    let cached_files: Vec<_> = fs::read_dir(&cache_dir)
        .unwrap()
        .flat_dir_entries()
        .collect();
    assert!(
        !cached_files.is_empty(),
        "cache directory should contain cached subtitle"
    );

    // Clean up
    let _ = fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn test_autosync_remote_fallback_not_found() {
    let tmp = std::env::temp_dir().join(format!(
        "dbsrt_remote_404_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&tmp).unwrap();

    let phash_dir = tmp.join("phash");
    let subtitles_dir = tmp.join("subtitles");
    let cache_dir = tmp.join("cache");
    fs::create_dir_all(&phash_dir).unwrap();
    fs::create_dir_all(&subtitles_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();

    let media_stem = "Unknown.Movie.2024.1080p";
    let media_video_path = tmp.join(format!("{}.mkv", media_stem));
    fs::write(&media_video_path, b"dummy mkv header").unwrap();
    make_dummy_phash_file(&phash_dir, media_stem, 50);

    let mock_provider = Arc::new(MockSubtitleProvider::new());
    let cache = SubtitleCache::new(Some(cache_dir.clone()));

    let app_state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        mock_provider,
        cache,
    );

    let router = create_router_with_state(app_state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = Client::new();
    let req = AutoSyncRequest {
        media_path: media_video_path.to_string_lossy().to_string(),
        language: "es".to_string(),
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
        subtitles_dir: Some(subtitles_dir.to_string_lossy().to_string()),
        search_distance: 16,
        nb_seconds: 1.0,
    };

    let resp = client
        .post(format!("http://{}/v1/autosync", addr))
        .json(&req)
        .send()
        .await
        .unwrap();

    let sync_res: AutoSyncResponse = resp.json().await.unwrap();
    assert!(!sync_res.success);
    assert!(sync_res.error.unwrap().contains("nor remote providers"));

    let _ = fs::remove_dir_all(&tmp);
}

trait ReadDirExt {
    fn flat_dir_entries(self) -> Box<dyn Iterator<Item = PathBuf>>;
}

impl ReadDirExt for fs::ReadDir {
    fn flat_dir_entries(self) -> Box<dyn Iterator<Item = PathBuf>> {
        Box::new(self.filter_map(Result::ok).flat_map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub) = fs::read_dir(&path) {
                    return sub.flat_dir_entries();
                }
            }
            Box::new(std::iter::once(path))
        }))
    }
}
