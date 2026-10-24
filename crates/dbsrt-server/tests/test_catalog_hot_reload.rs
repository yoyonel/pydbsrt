//! Integration test suite for Zero-Downtime Catalog Hot-Reload via ArcSwap.

use dbsrt_core::metadata::{MediaMetadata, MediaType};
use dbsrt_core::{CertifiedMediaCoupling, CertifiedSubtitleRecord, CouplingRegistry};
use dbsrt_index::dbphash::DbphashBuilder;
use dbsrt_server::{create_router_with_state, AppState, CatalogReloadResponse, RecognizeRequest};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

#[tokio::test]
async fn test_zero_downtime_hot_reload_and_concurrency() {
    let tmp_dir = std::env::temp_dir().join(format!("test_hot_reload_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let phash_dir = tmp_dir.join("phash");
    let subtitles_dir = tmp_dir.join("SRT");
    let catalog_path = tmp_dir.join("catalog.dbphash");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&subtitles_dir).unwrap();

    // 1. Initial State: Empty catalog
    let state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        Arc::new(dbsrt_core::OpenSubtitlesProvider::new(None)),
        dbsrt_core::SubtitleCache::default(),
    );
    state.set_dbphash(Some(catalog_path.clone()));
    assert_eq!(state.dbphash_path(), Some(catalog_path.clone()));
    let state_arc = Arc::new(state);

    let app = create_router_with_state((*state_arc).clone());

    // Bind ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // Initial check: status shows 0 catalog medias
    let res = client
        .get(format!("{}/v1/status", base_url))
        .send()
        .await
        .expect("Failed to call status");
    assert!(res.status().is_success());
    let status_json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        status_json["indexed_phash_count"].as_u64().unwrap_or(999),
        0
    );

    // Verify recognize returns not recognized before catalog exists
    let rec_payload = RecognizeRequest {
        media_path: "query.mp4".to_string(),
        query_fingerprint: Some(vec![
            "0x0123456789abcdef".to_string(),
            "0x1111222233334444".to_string(),
        ]),
        fps: Some(25.0),
        language: Some("fr".to_string()),
        client_version: Some("2.1.0".to_string()),
        search_distance: 4,
        nb_seconds: 1.0,
        phash_dir: None,
    };
    let rec_res = client
        .post(format!("{}/v1/recognize", base_url))
        .json(&rec_payload)
        .send()
        .await
        .unwrap();
    let rec_json: serde_json::Value = rec_res.json().await.unwrap();
    assert_eq!(rec_json["found"], false);

    // 2. Build a catalog with 2 media entries on disk
    let mut builder = DbphashBuilder::new();
    let meta1 = MediaMetadata {
        raw_title: "Silicon.Valley.S01E01.1080p.mkv".to_string(),
        clean_title: "Silicon Valley".to_string(),
        media_type: MediaType::TvEpisode,
        year: Some(2014),
        season: Some(1),
        episode: Some(1),
        resolution: Some("1080p".to_string()),
        source: Some("WEB-DL".to_string()),
        release_group: Some("KILLERS".to_string()),
        moviehash: None,
    };
    let cuts1 = [
        (0x0123_4567_89ab_cdef_u64, 1000u32),
        (0x1111_2222_3333_4444_u64, 5000u32),
    ];
    let sparse1 = [
        (0x0123_4567_89ab_cdef_u64, 0u32),
        (0x1111_2222_3333_4444_u64, 1000u32),
    ];
    builder.add_media(&meta1, 1_800_000, 25.0, &cuts1, &sparse1);

    let meta2 = MediaMetadata {
        raw_title: "Severance.S01E01.1080p.mkv".to_string(),
        clean_title: "Severance".to_string(),
        media_type: MediaType::TvEpisode,
        year: Some(2022),
        season: Some(1),
        episode: Some(1),
        resolution: Some("1080p".to_string()),
        source: Some("WEB-DL".to_string()),
        release_group: Some("NTb".to_string()),
        moviehash: None,
    };
    let cuts2 = [(0xaaaa_bbbb_cccc_dddd_u64, 2000u32)];
    let sparse2 = [(0xaaaa_bbbb_cccc_dddd_u64, 0u32)];
    builder.add_media(&meta2, 3_000_000, 23.976, &cuts2, &sparse2);

    let _stats = builder.write_to_file(&catalog_path).unwrap();

    // Also write a certified coupling to couplings.json
    let mut reg = CouplingRegistry::default();
    reg.couplings.push(CertifiedMediaCoupling {
        media_hash: "11112222333344445555666677778888".to_string(),
        media_name: "Silicon Valley".to_string(),
        phash_filename: "Silicon.Valley.S01E01.1080p.phash".to_string(),
        fps: 25.0,
        frame_count: 1000,
        subtitles: vec![CertifiedSubtitleRecord {
            subtitles_hash: "abcdef0123456789".to_string(),
            name: "Silicon.Valley.S01E01.fr.srt".to_string(),
            lang: "fr".to_string(),
            srt_path: "Silicon.Valley.S01E01.fr.srt".to_string(),
            certified: true,
            cue_count: 450,
            sub_frames: vec![],
        }],
    });
    let reg_path = CouplingRegistry::default_path(&subtitles_dir);
    reg.save(&reg_path).unwrap();

    // 3. Trigger POST /v1/catalog/reload
    let t_reload = Instant::now();
    let res = client
        .post(format!("{}/v1/catalog/reload", base_url))
        .send()
        .await
        .expect("Failed to call catalog reload");
    let reload_elapsed = t_reload.elapsed();
    assert!(res.status().is_success());

    let reload_resp: CatalogReloadResponse = res.json().await.unwrap();
    assert!(reload_resp.success);
    assert_eq!(reload_resp.catalog_medias, 2);
    assert_eq!(reload_resp.certified_couplings, 1);
    println!(
        "✓ Hot-Reload completed in {:.2}ms (server reported {:.2}ms)",
        reload_elapsed.as_secs_f64() * 1000.0,
        reload_resp.elapsed_ms
    );

    // 4. Verify /v1/status and /v1/recognize immediately reflect new catalog
    let res = client
        .get(format!("{}/v1/status", base_url))
        .send()
        .await
        .unwrap();
    let status_after: serde_json::Value = res.json().await.unwrap();
    assert_eq!(status_after["indexed_phash_count"].as_u64().unwrap_or(0), 2);

    let rec_res = client
        .post(format!("{}/v1/recognize", base_url))
        .json(&rec_payload)
        .send()
        .await
        .unwrap();
    let rec_json: serde_json::Value = rec_res.json().await.unwrap();
    assert_eq!(rec_json["found"], true);
    assert_eq!(rec_json["media_match"], "Silicon Valley");

    // 5. Test concurrent matching during hot-reload to prove Zero Downtime on real queries
    let mut handles = Vec::new();
    let n_workers = 8;
    let n_req_per_worker = 20;

    for _ in 0..n_workers {
        let client_c = client.clone();
        let url_c = format!("{}/v1/recognize", base_url);
        let payload_c = serde_json::to_value(&rec_payload).unwrap();
        handles.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            for _ in 0..n_req_per_worker {
                let t = Instant::now();
                let r = client_c.post(&url_c).json(&payload_c).send().await.unwrap();
                assert_eq!(r.status(), reqwest::StatusCode::OK);
                let val: serde_json::Value = r.json().await.unwrap();
                assert_eq!(val["found"], true);
                assert_eq!(val["media_match"], "Silicon Valley");
                latencies.push(t.elapsed().as_micros());
            }
            latencies
        }));
    }

    // Trigger another reload during concurrent workload
    let reload_res = client
        .post(format!("{}/v1/reload", base_url))
        .send()
        .await
        .expect("Failed to call /v1/reload during load");
    assert!(reload_res.status().is_success());

    let mut all_latencies = Vec::new();
    for h in handles {
        all_latencies.extend(h.await.unwrap());
    }

    assert_eq!(all_latencies.len(), n_workers * n_req_per_worker);
    all_latencies.sort();
    let p50 = all_latencies[all_latencies.len() / 2];
    let p95 = all_latencies[(all_latencies.len() as f64 * 0.95) as usize];
    println!(
        "✓ Zero-Downtime Concurrent Real Matching: {} requests, 100% OK. p50={}µs, p95={}µs",
        all_latencies.len(),
        p50,
        p95
    );

    // 6. Bug #2 Verification: Remove catalog, reload -> reader is cleared (0 medias)
    std::fs::remove_file(&catalog_path).unwrap();
    let reload_after_delete = client
        .post(format!("{}/v1/catalog/reload", base_url))
        .send()
        .await
        .unwrap();
    assert!(reload_after_delete.status().is_success());
    let st = client
        .get(format!("{}/v1/status", base_url))
        .send()
        .await
        .unwrap();
    let st_json: serde_json::Value = st.json().await.unwrap();
    assert_eq!(st_json["indexed_phash_count"].as_u64().unwrap_or(999), 0);

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}
