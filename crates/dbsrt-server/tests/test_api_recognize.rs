use dbsrt_server::{create_router, MetricsResponse, RecognizeRequest, RecognizeResponse};
use std::net::SocketAddr;
use std::path::PathBuf;

#[tokio::test]
async fn test_api_recognize_fingerprint_and_metrics() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let bbb_phash = repo_root.join("tests/data/big_buck_bunny_trailer_480p.phash");
    let test_phash_dir = repo_root.join("tests/data");
    let srt_dir = repo_root.join("data/SRT");

    let app = create_router(test_phash_dir.clone(), srt_dir.clone());

    // Bind on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().unwrap();

    // Spawn server in background
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 1. Test GET /v1/metrics
    let m_res = client
        .get(format!("{}/v1/metrics", base_url))
        .send()
        .await
        .expect("Failed to call /v1/metrics");
    assert!(m_res.status().is_success());
    let metrics: MetricsResponse = m_res.json().await.expect("Failed to parse metrics json");
    assert_eq!(metrics.status, "ok");

    // 2. Test POST /v1/recognize with empty/blank hashes
    let blank_payload = RecognizeRequest {
        media_path: String::new(),
        query_fingerprint: Some(vec![
            "0x0000000000000000".to_string(),
            "0x0000000000000000".to_string(),
        ]),
        fps: Some(25.0),
        language: Some("fr".to_string()),
        client_version: Some("2.1.0".to_string()),
        search_distance: 6,
        nb_seconds: 1.0,
        phash_dir: None,
    };

    let blank_res = client
        .post(format!("{}/v1/recognize", base_url))
        .json(&blank_payload)
        .send()
        .await
        .expect("Failed to send blank recognize");
    assert!(blank_res.status().is_success());
    let blank_data: RecognizeResponse = blank_res.json().await.expect("Failed to parse json");
    assert!(!blank_data.found);
    assert_eq!(blank_data.confidence, 0.0);

    // 2b. Test POST /v1/recognize with 5 identical hashes (zero-entropy / uniform black frames rejection)
    let identical_payload = RecognizeRequest {
        media_path: String::new(),
        query_fingerprint: Some(vec![
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
            "0x8f4c7233588e457e".to_string(),
        ]),
        fps: Some(25.0),
        language: Some("fr".to_string()),
        client_version: Some("2.1.0".to_string()),
        search_distance: 6,
        nb_seconds: 1.0,
        phash_dir: None,
    };

    let id_res = client
        .post(format!("{}/v1/recognize", base_url))
        .json(&identical_payload)
        .send()
        .await
        .expect("Failed to send identical recognize");
    assert!(id_res.status().is_success());
    let id_data: RecognizeResponse = id_res.json().await.expect("Failed to parse json");
    assert!(
        !id_data.found,
        "Identical static hashes must be rejected with found=false"
    );
    assert_eq!(id_data.confidence, 0.0);
    assert!(id_data.media_match.is_none());

    // 3. Test POST /v1/recognize with 25 real hashes from versioned Big Buck Bunny .phash
    assert!(
        bbb_phash.exists(),
        "BBB phash file must exist at {:?}",
        bbb_phash
    );
    let hashes = dbsrt_core::read_phash_file(&bbb_phash).expect("read phash file");
    assert!(hashes.len() >= 100);

    // Take 25 consecutive hashes from middle
    let sample = &hashes[100..125];
    let fp_strings: Vec<String> = sample.iter().map(|h| format!("{:#018x}", h)).collect();

    let req_payload = RecognizeRequest {
        media_path: String::new(),
        query_fingerprint: Some(fp_strings),
        fps: Some(25.0),
        language: Some("fr".to_string()),
        client_version: Some("2.1.0".to_string()),
        search_distance: 12,
        nb_seconds: 1.0,
        phash_dir: None,
    };

    let res = client
        .post(format!("{}/v1/recognize", base_url))
        .json(&req_payload)
        .send()
        .await
        .expect("Failed to call recognize with fingerprint");
    assert!(res.status().is_success());
    let rec_data: RecognizeResponse = res
        .json()
        .await
        .expect("Failed to parse recognize response");

    assert!(rec_data.found);
    assert!(rec_data.media_match.is_some());
    let matched = rec_data.media_match.unwrap();
    assert!(matched.contains("big_buck_bunny"));
    assert!(rec_data.confidence >= 50.0);
    assert!(rec_data.tier.is_some());
    assert!(rec_data.shift_ms.is_some());
    println!(
        "✓ Recognized via pure 200-byte fingerprint in {:.2}ms: matched='{}' (conf={:.1}%, tier={:?}, shift={:?}ms)",
        rec_data.timer_seconds * 1000.0,
        matched,
        rec_data.confidence,
        rec_data.tier,
        rec_data.shift_ms
    );

    // 4. Verify metrics were updated
    let m_res2 = client
        .get(format!("{}/v1/metrics", base_url))
        .send()
        .await
        .expect("Failed to call /v1/metrics");
    let metrics2: MetricsResponse = m_res2.json().await.expect("Failed to parse metrics json");
    assert!(metrics2.total_requests >= 2);
    assert!(metrics2.recognize_requests >= 2);
}
