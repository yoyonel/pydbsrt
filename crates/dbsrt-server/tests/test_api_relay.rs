use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};
use dbsrt_server::{
    create_router_with_state, AppState, MetricsResponse, SearchSubtitlesRequest,
    SearchSubtitlesResponse,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn test_dual_process_remote_relay_e2e() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let phash_dir = repo_root.join("data/__PHASH__");
    let srt_dir = repo_root.join("data/SRT");
    let snippet_video = repo_root.join("data/snippets/Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4");

    if !phash_dir.exists() || !snippet_video.exists() {
        eprintln!("Skipping test: test fixtures not found");
        return;
    }

    // 1. Start Server B (Simulated Remote / o2switch / NAS)
    let remote_state = AppState::new(
        phash_dir.clone(),
        srt_dir.clone(),
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    );
    let remote_app = create_router_with_state(remote_state);
    let remote_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind remote listener");
    let remote_addr: SocketAddr = remote_listener.local_addr().unwrap();
    let remote_url = format!("http://{}", remote_addr);

    tokio::spawn(async move {
        axum::serve(remote_listener, remote_app).await.unwrap();
    });

    // 2. Start Server A (Local Sidecar Agent) with remote_api set to Server B
    let mut local_state = AppState::new(
        std::env::temp_dir(), // Empty local phash dir!
        std::env::temp_dir(), // Empty local srt dir!
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    );
    local_state.remote_api = Some(remote_url.clone());
    local_state.is_agent = true;

    let local_app = create_router_with_state(local_state);
    let local_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local listener");
    let local_addr: SocketAddr = local_listener.local_addr().unwrap();
    let local_url = format!("http://{}", local_addr);

    tokio::spawn(async move {
        axum::serve(local_listener, local_app).await.unwrap();
    });

    let client = reqwest::Client::new();

    // 3. VLC Lua Simulator calls Local Agent: POST /v1/search
    let search_req = SearchSubtitlesRequest {
        media_path: snippet_video.to_string_lossy().to_string(),
        language: "fr".to_string(),
        search_mode: Some("phash".to_string()),
        query_title: Some("Andor".to_string()),
        season: Some(1),
        episode: Some(1),
        subtitles_dir: None,
        phash_dir: None,
        index_engine: None,
    };

    let t0 = std::time::Instant::now();
    let search_res = client
        .post(format!("{}/v1/search", local_url))
        .json(&search_req)
        .send()
        .await
        .expect("POST /v1/search to local agent");

    assert!(search_res.status().is_success());
    let res_data: SearchSubtitlesResponse = search_res.json().await.expect("parse search response");
    let elapsed = t0.elapsed().as_secs_f64() * 1000.0;

    println!(
        "✓ Dual-Process Relay E2E completed in {:.2}ms: matched={:?}, total_cands={}",
        elapsed, res_data.matched_media, res_data.total_candidates
    );

    // Assertions
    assert!(res_data.matched_media.is_some());
    let matched = res_data.matched_media.unwrap();
    assert!(matched.contains("Andor"));
    assert!(res_data.total_candidates > 0);
    assert!(!res_data.candidates.is_empty());

    // Check that at least one candidate has resync shift applied
    let synced_cand = res_data
        .candidates
        .iter()
        .find(|c| c.shift_ms.is_some() && c.shift_ms.unwrap().abs() > 1000.0);
    assert!(
        synced_cand.is_some(),
        "Expected at least one synced candidate with ~ -1167s shift, found: {:?}",
        res_data.candidates
    );

    // 4. Verify Observability: Local agent metrics
    let local_metrics_res = client
        .get(format!("{}/v1/metrics", local_url))
        .send()
        .await
        .expect("local /v1/metrics");
    let local_m: MetricsResponse = local_metrics_res.json().await.unwrap();
    assert_eq!(local_m.total_requests, 1);
    assert_eq!(local_m.search_requests, 1);
    assert_eq!(local_m.remote_errors, 0);

    // 5. Verify Observability: Remote server metrics
    let remote_metrics_res = client
        .get(format!("{}/v1/metrics", remote_url))
        .send()
        .await
        .expect("remote /v1/metrics");
    let remote_m: MetricsResponse = remote_metrics_res.json().await.unwrap();
    assert_eq!(remote_m.total_requests, 1);
    assert_eq!(remote_m.recognize_requests, 1);
}
