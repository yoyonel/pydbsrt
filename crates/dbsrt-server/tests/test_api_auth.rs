use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};
use dbsrt_server::{create_router_with_state, AppState};
use reqwest::StatusCode;
use std::path::PathBuf;
use std::sync::Arc;

async fn spawn_server(api_key_file: Option<PathBuf>) -> String {
    let phash_dir = std::env::temp_dir().join(format!("test_phash_auth_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&phash_dir);
    let mut state = AppState::new(
        phash_dir,
        PathBuf::from("../../data/SRT"),
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    );
    state.api_key_file = api_key_file;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, create_router_with_state(state))
            .await
            .unwrap();
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn test_api_key_file_guards_all_routes_but_health() {
    let key_file = std::env::temp_dir().join(format!("test_api_key_{}.secret", std::process::id()));
    std::fs::write(&key_file, "# pool\nmaster-key\n").unwrap();
    let base = spawn_server(Some(key_file.clone())).await;
    let client = reqwest::Client::new();
    let status_url = format!("{}/v1/status", base);

    let res = client.get(&status_url).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = client
        .get(&status_url)
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = client
        .get(&status_url)
        .bearer_auth("master-key")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = client
        .get(&status_url)
        .header("X-API-Key", "master-key")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = client
        .post(format!("{}/v1/catalog/reload", base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    for health in ["/health", "/v1/health"] {
        let res = client
            .get(format!("{}{}", base, health))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{health} must stay public");
    }

    // Rotation applies without restart: the file is re-read per request
    std::fs::write(&key_file, "rotated-key\n").unwrap();
    let res = client
        .get(&status_url)
        .bearer_auth("master-key")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = client
        .get(&status_url)
        .bearer_auth("rotated-key")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // A missing key file fails closed
    std::fs::remove_file(&key_file).unwrap();
    let res = client
        .get(&status_url)
        .bearer_auth("rotated-key")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_no_api_key_file_keeps_api_open() {
    let base = spawn_server(None).await;
    let res = reqwest::get(format!("{}/v1/status", base)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
