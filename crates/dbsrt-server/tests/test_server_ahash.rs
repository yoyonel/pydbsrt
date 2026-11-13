use base64::prelude::*;
use dbsrt_core::ahash::{AhashFile, AhashHeader, HaitsmaKalkerExtractor};
use dbsrt_core::{
    CertifiedSubtitleRecord, CouplingRegistry, LangStatus, MockSubtitleProvider,
    OpenSubtitlesProvider,
};
use dbsrt_server::{
    create_router_with_state, AHashSearchRequest, AHashSearchResponse, AppState,
    AppStateInitOptions, CatalogReloadResponse, CatalogStatusResponse, MetricsResponse,
    SearchSubtitlesRequest, SearchSubtitlesResponse,
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

/// Frames of the synthetic reference media (> offset + query window).
const SYNC_REF_FRAMES: usize = 6_000;
/// Dense frame where the query window matches in the reference (~60.006 s at 86.125 fps).
const SYNC_REF_OFFSET_FRAMES: usize = 5_168;
/// Query window length in frames (5.0 s).
const SYNC_QUERY_FRAMES: usize = 430;
/// Frame rate of the mock `.ahash` files (5512 Hz / hop 64).
const SYNC_AUDIO_FPS: f64 = 5_512.0 / 64.0;
/// Milliseconds per second.
const SYNC_MILLIS_PER_SEC: f64 = 1_000.0;
/// Absolute tolerance on the exact floating point shift, ms.
const SYNC_SHIFT_EPSILON_MS: f64 = 1e-6;
/// Number of cues / digest placeholders of the certified subtitle record.
const SYNC_CERT_CUE_COUNT: usize = 1;
const SYNC_CERT_SUBTITLES_HASH: &str = "00112233445566778899aabbccddeeff";

/// Writes an arbitrary hash sequence as a `.ahash` sidecar (same layout as the catalog files).
fn write_ahash_sidecar(path: &Path, hashes: &[u64]) {
    let (_band_bins, band_bounds) = HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
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
    AhashFile::write_to_path(path, &header, hashes).expect("write query .ahash");
}

/// `POST /v1/search` in `ahash` mode must expose the acoustic shift
/// (`query_start_sec - ref_offset_sec`) on the `certified_synced` candidate instead of
/// re-running the visual pHash consensus (which yields 0 ms here: no pHash data at all).
#[tokio::test]
async fn test_search_ahash_mode_propagates_acoustic_shift_to_certified_synced() {
    let catalog_dir = tempdir().expect("catalog dir");
    let query_dir = tempdir().expect("query dir");
    let request_subs_dir = tempdir().expect("request subtitles dir");
    let state_subs_dir = tempdir().expect("state subtitles dir");

    let reference = write_mock_ahash(
        catalog_dir.path(),
        "media_one",
        SYNC_REF_FRAMES,
        0x1357_9BDF,
    );
    let query_path = query_dir.path().join("query_cut.86.1fps.ahash");
    write_ahash_sidecar(
        &query_path,
        &reference[SYNC_REF_OFFSET_FRAMES..SYNC_REF_OFFSET_FRAMES + SYNC_QUERY_FRAMES],
    );

    let mut registry = CouplingRegistry::default();
    registry.upsert_coupling(
        "ffeeddccbbaa99887766554433221100".to_string(),
        "media_one".to_string(),
        "media_one.25.0fps.phash".to_string(),
        25.0,
        SYNC_REF_FRAMES,
        CertifiedSubtitleRecord {
            subtitles_hash: SYNC_CERT_SUBTITLES_HASH.to_string(),
            name: "media_one.en.srt".to_string(),
            lang: "en".to_string(),
            lang_status: LangStatus::Detected("en".to_string()),
            srt_path: "media_one.en.srt".to_string(),
            certified: true,
            cue_count: SYNC_CERT_CUE_COUNT,
            sub_frames: Vec::new(),
        },
    );
    registry
        .save(&CouplingRegistry::default_path(request_subs_dir.path()))
        .expect("save couplings.json");

    let state = AppState::with_options(
        catalog_dir.path().to_path_buf(),
        state_subs_dir.path().to_path_buf(),
        Arc::new(MockSubtitleProvider::new()),
        AppStateInitOptions {
            dbphash_path: None,
            extra_phash_dirs: Vec::new(),
            ahash_dir: Some(catalog_dir.path().to_path_buf()),
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

    let req = SearchSubtitlesRequest {
        media_path: query_path.to_string_lossy().to_string(),
        language: "auto".to_string(),
        subtitles_dir: Some(request_subs_dir.path().to_string_lossy().to_string()),
        phash_dir: None,
        search_mode: Some("ahash".to_string()),
        index_engine: None,
        query_title: None,
        season: None,
        episode: None,
    };
    let resp = reqwest::Client::new()
        .post(format!("http://{}/v1/search", addr))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let data: SearchSubtitlesResponse = resp.json().await.unwrap();

    assert_eq!(data.matched_media.as_deref(), Some("media_one"));
    let baseline = data
        .candidates
        .iter()
        .find(|c| c.source == "certified")
        .expect("baseline certified candidate");
    assert_eq!(baseline.shift_ms, Some(0.0));

    // `.ahash` sidecar => query_start_sec = 0.0; shift = 0.0 - ref_offset_sec.
    let expected_shift_ms = -(SYNC_REF_OFFSET_FRAMES as f64 / SYNC_AUDIO_FPS) * SYNC_MILLIS_PER_SEC;
    let synced = data
        .candidates
        .iter()
        .find(|c| c.source == "certified_synced")
        .expect("certified_synced candidate must carry the acoustic shift");
    let got = synced.shift_ms.expect("shift_ms present");
    println!("certified_synced shift_ms = {got} (expected {expected_shift_ms})");
    assert!(
        (got - expected_shift_ms).abs() < SYNC_SHIFT_EPSILON_MS,
        "acoustic shift lost: got {got}, expected {expected_shift_ms}"
    );
}
