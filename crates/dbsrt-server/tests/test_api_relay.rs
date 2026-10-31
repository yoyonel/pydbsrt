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
    let couplings_file = srt_dir.join("couplings.json");
    let has_andor_coupling = if couplings_file.exists() {
        std::fs::read_to_string(&couplings_file)
            .map(|s| s.contains("Andor"))
            .unwrap_or(false)
    } else {
        false
    };

    if !phash_dir.exists() || !snippet_video.exists() || !has_andor_coupling {
        eprintln!("Skipping test: test fixtures or Andor coupling not found");
        return;
    }

    // SAFETY: Test setup before any async tasks or server instances spawn.
    unsafe {
        std::env::set_var("DBSRT_RELAY_TIMEOUT_MS", "15000");
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

    // 4. Test CAS Subtitle Service: GET /v1/subtitles/{hash} on Remote Server
    let top_cand = &res_data.candidates[0];
    let cand_id = top_cand
        .candidate_id
        .as_ref()
        .expect("candidate_id present");
    assert!(
        cand_id.starts_with("sub:sha256:"),
        "Expected CAS format 'sub:sha256:<hash>', got '{}'",
        cand_id
    );
    let sub_hash = cand_id.strip_prefix("sub:sha256:").unwrap();

    let cas_res = client
        .get(format!("{}/v1/subtitles/{}", remote_url, sub_hash))
        .send()
        .await
        .expect("GET /v1/subtitles/{hash}");
    assert!(cas_res.status().is_success());
    assert_eq!(
        cas_res
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "text/plain; charset=utf-8"
    );
    let srt_body = cas_res.text().await.expect("read CAS srt text");
    assert!(srt_body.contains("-->"), "SRT body must contain cue arrows");

    // 5. Test Remote Subtitle Transfer in Download Handler: Local agent downloads via remote CAS
    let dl_req = dbsrt_server::DownloadSubtitleRequest {
        media_path: snippet_video.to_string_lossy().to_string(),
        source: top_cand.source.clone(),
        candidate_id: top_cand.candidate_id.clone(),
        language: top_cand.language.clone(),
        resync: false,
        shift_ms: top_cand.shift_ms,
        subtitles_dir: None,
        phash_dir: None,
    };
    let dl_res = client
        .post(format!("{}/v1/download", local_url))
        .json(&dl_req)
        .send()
        .await
        .expect("POST /v1/download to local agent");
    assert!(dl_res.status().is_success());
    let dl_data: dbsrt_server::DownloadSubtitleResponse = dl_res.json().await.unwrap();
    assert!(
        dl_data.success,
        "Download must succeed via remote CAS: {:?}",
        dl_data.error
    );
    assert!(!dl_data.output_srt_path.is_empty());
    assert!(std::path::Path::new(&dl_data.output_srt_path).exists());
    let _ = std::fs::remove_file(&dl_data.output_srt_path);

    // 6. Verify Observability: Local agent metrics
    let local_metrics_res = client
        .get(format!("{}/v1/metrics", local_url))
        .send()
        .await
        .expect("local /v1/metrics");
    let local_m: MetricsResponse = local_metrics_res.json().await.unwrap();
    assert_eq!(local_m.total_requests, 2);
    assert_eq!(local_m.search_requests, 1);
    assert_eq!(local_m.remote_errors, 0);

    // 7. Verify Observability: Remote server metrics
    let remote_metrics_res = client
        .get(format!("{}/v1/metrics", remote_url))
        .send()
        .await
        .expect("remote /v1/metrics");
    let remote_m: MetricsResponse = remote_metrics_res.json().await.unwrap();
    assert_eq!(remote_m.total_requests, 3); // 1 recognize + 1 test CAS fetch + 1 agent CAS fetch during download
    assert_eq!(remote_m.recognize_requests, 1);
}

#[tokio::test]
async fn test_remote_relay_api_key_header_forwarding() {
    use axum::extract::Request;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use std::sync::atomic::{AtomicBool, Ordering};

    let api_key_received = Arc::new(AtomicBool::new(false));
    let flag_clone = api_key_received.clone();

    // Mock remote server asserting presence of X-API-Key
    let mock_remote = axum::Router::new().route(
        "/v1/recognize",
        post(move |req: Request| async move {
            if let Some(val) = req.headers().get("X-API-Key") {
                if val == "secret-test-key-256" {
                    flag_clone.store(true, Ordering::SeqCst);
                }
            }
            axum::Json(dbsrt_server::RecognizeResponse {
                found: false,
                media_name_search: "mock".to_string(),
                media_match: None,
                season: None,
                episode: None,
                nb_offsets_match: 0,
                confidence: 0.0,
                tier: Some("Mock Tier".to_string()),
                shift_frames: Some(0),
                shift_ms: Some(0.0),
                candidates: Some(vec![]),
                timer_seconds: 0.001,
            })
            .into_response()
        }),
    );

    let remote_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(remote_listener, mock_remote).await.unwrap();
    });

    // Local agent configured with remote_api and remote_api_key
    let mut local_state = AppState::new(
        std::env::temp_dir(),
        std::env::temp_dir(),
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    );
    local_state.remote_api = Some(format!("http://{}", remote_addr));
    local_state.remote_api_key = Some("secret-test-key-256".to_string());
    local_state.is_agent = true;

    let local_app = create_router_with_state(local_state);
    let local_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(local_listener, local_app).await.unwrap();
    });

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let snippet_video = repo_root.join("data/snippets/Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4");
    if !snippet_video.exists() {
        return;
    }

    let client = reqwest::Client::new();
    let req = SearchSubtitlesRequest {
        media_path: snippet_video.to_string_lossy().to_string(),
        language: "fr".to_string(),
        search_mode: Some("phash".to_string()),
        query_title: Some("Test".to_string()),
        season: None,
        episode: None,
        subtitles_dir: None,
        phash_dir: None,
        index_engine: None,
    };

    let _ = client
        .post(format!("http://{}/v1/search", local_addr))
        .json(&req)
        .send()
        .await;

    // Verify mock server received the header
    assert!(
        api_key_received.load(Ordering::SeqCst),
        "Expected remote server to receive 'X-API-Key: secret-test-key-256'"
    );
}
