use base64::prelude::*;
use dbsrt_core::ahash::{AhashFile, AhashHeader, HaitsmaKalkerExtractor};
use dbsrt_core::OpenSubtitlesProvider;
use dbsrt_server::{
    create_router_with_state, AHashSearchRequest, AHashSearchResponse, AppState,
    AppStateInitOptions, CatalogReloadResponse, CatalogStatusResponse, MetricsResponse,
};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

fn write_mock_ahash(path: &Path, name: &str, frame_count: usize, seed_salt: u64) -> Vec<u64> {
    let (_band_bins, band_bounds) = HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
    let mut hashes = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
        // Construct distinct reproducible pseudo-random hashes
        let h = seed_salt
            .wrapping_mul(6364136223846793005)
            .wrapping_add((i as u64).wrapping_mul(1442695040888963407))
            ^ 0xA5A5_5A5A_A5A5_5A5A;
        hashes.push(h);
    }

    let header = AhashHeader {
        version: 1,
        sample_rate: 5512,
        frame_size: 2048,
        hop_size: 64,
        num_bands: 33,
        spectral_lag: 2,
        frame_count: hashes.len() as u64,
        band_bounds,
    };

    let file_path = path.join(format!("{}.86.1fps.ahash", name));
    AhashFile::write_to_path(&file_path, &header, &hashes).expect("write mock .ahash");
    hashes
}

#[tokio::test]
async fn test_server_ahash_validation_and_errors() {
    let dir = tempdir().expect("tempdir");
    let state = AppState::with_options(
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
        Arc::new(OpenSubtitlesProvider::new(None)),
        AppStateInitOptions {
            dbphash_path: None,
            extra_phash_dirs: Vec::new(),
            ahash_dir: Some(dir.path().to_path_buf()),
        },
    );

    let app = create_router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let url = format!("http://{}/v1/search/ahash", addr);

    // 1. Missing/empty query hashes
    let empty_req = AHashSearchRequest::default();
    let resp = client.post(&url).json(&empty_req).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. Query shorter than MIN_QUERY_FRAMES (8)
    let short_req = AHashSearchRequest {
        hashes: Some(vec![0x1234_5678_9ABC_DEF0; 4]),
        ..Default::default()
    };
    let resp = client.post(&url).json(&short_req).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_server_ahash_end_to_end_search_and_encodings() {
    let dir = tempdir().expect("tempdir");
    let hashes_one = write_mock_ahash(dir.path(), "media_one", 500, 0x1111_2222_3333_4444);
    let _hashes_two = write_mock_ahash(dir.path(), "media_two", 500, 0x9999_8888_7777_6666);

    let state = AppState::with_options(
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
        Arc::new(OpenSubtitlesProvider::new(None)),
        AppStateInitOptions {
            dbphash_path: None,
            extra_phash_dirs: Vec::new(),
            ahash_dir: Some(dir.path().to_path_buf()),
        },
    );

    let app = create_router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let search_url = format!("http://{}/v1/search/ahash", addr);

    // 1. Query with raw u64 slice (430 frames = optimal 5.0s window, offset 60)
    let query_slice = &hashes_one[60..490];
    let req_u64 = AHashSearchRequest {
        hashes: Some(query_slice.to_vec()),
        ..Default::default()
    };
    let resp = client
        .post(&search_url)
        .json(&req_u64)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let data: AHashSearchResponse = resp.json().await.unwrap();

    assert!(data.success);
    assert_eq!(data.total_catalog_files, 2);
    assert_eq!(data.total_catalog_frames, 1000);
    let best = data.best_match.expect("best match present");
    assert_eq!(best.media_name, "media_one");
    assert_eq!(best.ref_frame_offset, 60);
    assert_eq!(best.mean_distance, 0.0);
    assert_eq!(best.score, 1.0);
    assert!(best.snr_bits >= 31.0);

    // 2. Query with hex strings format
    let hex_fingerprint: Vec<String> = query_slice
        .iter()
        .map(|h| format!("0x{:016x}", h))
        .collect();
    let req_hex = AHashSearchRequest {
        fingerprint: Some(hex_fingerprint),
        ..Default::default()
    };
    let resp_hex = client
        .post(&search_url)
        .json(&req_hex)
        .send()
        .await
        .unwrap();
    assert!(resp_hex.status().is_success());
    let data_hex: AHashSearchResponse = resp_hex.json().await.unwrap();
    assert!(data_hex.success);
    assert_eq!(data_hex.best_match.unwrap().ref_frame_offset, 60);

    // 3. Query with base64 encoded bytes
    let mut raw_bytes = Vec::with_capacity(query_slice.len() * 8);
    for h in query_slice {
        raw_bytes.extend_from_slice(&h.to_le_bytes());
    }
    let b64_str = BASE64_STANDARD.encode(&raw_bytes);
    let req_b64 = AHashSearchRequest {
        ahash_bytes: Some(b64_str),
        ..Default::default()
    };
    let resp_b64 = client
        .post(&search_url)
        .json(&req_b64)
        .send()
        .await
        .unwrap();
    assert!(resp_b64.status().is_success());
    let data_b64: AHashSearchResponse = resp_b64.json().await.unwrap();
    assert!(data_b64.success);
    assert_eq!(data_b64.best_match.unwrap().ref_frame_offset, 60);

    // 4. Query with uncorrelated random hashes -> rejected
    let uncorrelated_hashes: Vec<u64> = (0..50)
        .map(|i| 0xDEAD_BEEF_0000_0000 ^ (i as u64))
        .collect();
    let req_unrelated = AHashSearchRequest {
        hashes: Some(uncorrelated_hashes),
        ..Default::default()
    };
    let resp_unrelated = client
        .post(&search_url)
        .json(&req_unrelated)
        .send()
        .await
        .unwrap();
    assert!(resp_unrelated.status().is_success());
    let data_unrelated: AHashSearchResponse = resp_unrelated.json().await.unwrap();
    assert!(!data_unrelated.success);
    assert!(data_unrelated.best_match.is_none());
    assert!(data_unrelated.candidates.is_empty());
}

#[tokio::test]
async fn test_server_ahash_catalog_status_and_reload() {
    let dir = tempdir().expect("tempdir");
    write_mock_ahash(dir.path(), "stream_a", 200, 0x1234);

    let state = AppState::with_options(
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
        Arc::new(OpenSubtitlesProvider::new(None)),
        AppStateInitOptions {
            dbphash_path: None,
            extra_phash_dirs: Vec::new(),
            ahash_dir: Some(dir.path().to_path_buf()),
        },
    );

    let app = create_router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 1. Initial status
    let status_resp = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap();
    assert!(status_resp.status().is_success());
    let status: CatalogStatusResponse = status_resp.json().await.unwrap();
    assert_eq!(status.ahash_medias, 1);
    assert_eq!(status.ahash_frames, 200);

    // 2. Add second .ahash file and reload
    write_mock_ahash(dir.path(), "stream_b", 300, 0x5678);

    let reload_resp = client
        .post(format!("{}/v1/catalog/reload", base_url))
        .send()
        .await
        .unwrap();
    assert!(reload_resp.status().is_success());
    let reload: CatalogReloadResponse = reload_resp.json().await.unwrap();
    assert_eq!(reload.ahash_medias, 2);
    assert_eq!(reload.ahash_frames, 500);

    // 3. Verify status after reload
    let status2_resp = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap();
    let status2: CatalogStatusResponse = status2_resp.json().await.unwrap();
    assert_eq!(status2.ahash_medias, 2);
    assert_eq!(status2.ahash_frames, 500);

    // 4. Verify metrics endpoint
    let metrics_resp = client
        .get(format!("{}/v1/metrics", base_url))
        .send()
        .await
        .unwrap();
    let metrics: MetricsResponse = metrics_resp.json().await.unwrap();
    assert_eq!(metrics.status, "ok");
}
