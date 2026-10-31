//! Integration test suite for Zero-Downtime Catalog Hot-Reload via ArcSwap.

use dbsrt_core::metadata::{MediaMetadata, MediaType};
use dbsrt_core::{CertifiedMediaCoupling, CertifiedSubtitleRecord, CouplingRegistry, LangStatus};
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
            lang_status: LangStatus::Detected("fr".to_string()),
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

#[tokio::test]
async fn test_pillar2_delta_ingest_and_compaction() {
    let tmp_dir = std::env::temp_dir().join(format!("test_pillar2_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let phash_dir = tmp_dir.join("phash");
    let subtitles_dir = tmp_dir.join("SRT");
    let catalog_path = tmp_dir.join("catalog.dbphash");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&subtitles_dir).unwrap();

    // 1. Initial State: Catalog with 1 Base media
    let mut builder = DbphashBuilder::new();
    let meta_base = MediaMetadata {
        raw_title: "Interstellar.2014.1080p.mkv".to_string(),
        clean_title: "Interstellar".to_string(),
        media_type: MediaType::Movie,
        year: Some(2014),
        season: None,
        episode: None,
        resolution: Some("1080p".to_string()),
        source: Some("BluRay".to_string()),
        release_group: Some("SPARKS".to_string()),
        moviehash: None,
    };
    let cuts_base = [(0x1111_2222_3333_4444_u64, 1000u32)];
    let sparse_base = [(0x1111_2222_3333_4444_u64, 0u32)];
    builder.add_media(&meta_base, 10_000_000, 24.0, &cuts_base, &sparse_base);
    builder.write_to_file(&catalog_path).unwrap();

    let state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        Arc::new(dbsrt_core::OpenSubtitlesProvider::new(None)),
        dbsrt_core::SubtitleCache::default(),
    );
    state.set_dbphash(Some(catalog_path.clone()));
    let state_arc = Arc::new(state);

    let app = create_router_with_state((*state_arc).clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 2. Check initial catalog status
    let res_status = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res_status.status(), reqwest::StatusCode::OK);
    let st1: serde_json::Value = res_status.json().await.unwrap();
    assert_eq!(st1["base_medias"], 1);
    assert_eq!(st1["delta_medias"], 0);
    assert_eq!(st1["total_medias"], 1);

    // 3. Ingest a new media into Delta Index in RAM
    let ingest_hashes = vec![0x9999_8888_7777_6666_u64; 100];
    let ingest_req = serde_json::json!({
        "filename": "Dune.Part.Two.2024.1080p.mkv",
        "hashes": ingest_hashes,
        "fps": 24.0
    });
    let ingest_res = client
        .post(format!("{}/v1/catalog/ingest", base_url))
        .json(&ingest_req)
        .send()
        .await
        .unwrap();
    assert_eq!(ingest_res.status(), reqwest::StatusCode::OK);
    let ingest_json: serde_json::Value = ingest_res.json().await.unwrap();
    assert_eq!(ingest_json["success"], true);
    assert_eq!(ingest_json["clean_title"], "Dune Part Two");
    assert_eq!(ingest_json["delta_medias_count"], 1);

    // 4. Verify catalog status reflects Delta
    let res_status2 = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap();
    let st2: serde_json::Value = res_status2.json().await.unwrap();
    assert_eq!(st2["base_medias"], 1);
    assert_eq!(st2["delta_medias"], 1);
    assert_eq!(st2["total_medias"], 2);

    // 5. Query recognize for the delta-ingested media
    let rec_payload = RecognizeRequest {
        media_path: "dune_query.mp4".to_string(),
        query_fingerprint: Some(vec![
            "0x9999888877776666".to_string(),
            "0x9999888877776666".to_string(),
            "0x9999888877776666".to_string(),
        ]),
        fps: Some(24.0),
        language: Some("en".to_string()),
        client_version: Some("2.1.0".to_string()),
        search_distance: 2,
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
    assert_eq!(rec_json["found"], true);
    assert_eq!(rec_json["media_match"], "Dune Part Two");

    // 6. Trigger Compaction (Pillar 2: Base + Delta -> compiled catalog -> hot reload -> clear delta)
    let compact_res = client
        .post(format!("{}/v1/catalog/compact", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(compact_res.status(), reqwest::StatusCode::OK);
    let compact_json: serde_json::Value = compact_res.json().await.unwrap();
    assert_eq!(compact_json["success"], true);
    assert_eq!(compact_json["compacted"], true);
    assert_eq!(compact_json["total_medias"], 2);
    assert_eq!(compact_json["delta_medias_flushed"], 1);

    // 7. Verify status after compaction: base has 2, delta has 0
    let res_status3 = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap();
    let st3: serde_json::Value = res_status3.json().await.unwrap();
    assert_eq!(st3["base_medias"], 2);
    assert_eq!(st3["delta_medias"], 0);
    assert_eq!(st3["total_medias"], 2);

    // 8. Recognize both media 1 and media 2 now served directly from Base
    let rec_res_dune = client
        .post(format!("{}/v1/recognize", base_url))
        .json(&rec_payload)
        .send()
        .await
        .unwrap();
    let rec_dune_json: serde_json::Value = rec_res_dune.json().await.unwrap();
    assert_eq!(rec_dune_json["found"], true);
    assert_eq!(rec_dune_json["media_match"], "Dune Part Two");

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_pre_swap_validation_rejects_corrupted_file() {
    let tmp_dir = std::env::temp_dir().join(format!("test_corrupt_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let phash_dir = tmp_dir.join("phash");
    let subtitles_dir = tmp_dir.join("SRT");
    let catalog_path = tmp_dir.join("catalog.dbphash");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&subtitles_dir).unwrap();

    // 1. Initial valid catalog
    let mut builder = DbphashBuilder::new();
    let meta = MediaMetadata {
        raw_title: "Valid.2020.mkv".to_string(),
        clean_title: "Valid".to_string(),
        media_type: MediaType::Movie,
        year: Some(2020),
        season: None,
        episode: None,
        resolution: None,
        source: None,
        release_group: None,
        moviehash: None,
    };
    builder.add_media(&meta, 1_000_000, 24.0, &[(0x1234, 100)], &[(0x1234, 0)]);
    builder.write_to_file(&catalog_path).unwrap();

    let state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        Arc::new(dbsrt_core::OpenSubtitlesProvider::new(None)),
        dbsrt_core::SubtitleCache::default(),
    );
    state.set_dbphash(Some(catalog_path.clone()));
    let state_arc = Arc::new(state);

    let app = create_router_with_state((*state_arc).clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // Initial status: 1 media
    let st: serde_json::Value = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(st["base_medias"], 1);

    // 2. Corrupt the file on disk (simulate interrupted rsync or truncated transfer)
    std::fs::write(&catalog_path, b"CORRUPTED_TRUNCATED_BYTES").unwrap();

    // 3. Trigger reload: pre-swap validation must reject it!
    let reload_res = client
        .post(format!("{}/v1/catalog/reload", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        reload_res.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_concurrent_ingest_during_compaction_zero_data_loss() {
    let tmp_dir =
        std::env::temp_dir().join(format!("test_compact_concurrent_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let phash_dir = tmp_dir.join("phash");
    let subtitles_dir = tmp_dir.join("SRT");
    let catalog_path = tmp_dir.join("catalog.dbphash");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&subtitles_dir).unwrap();

    // 1. Initial State: Catalog with 1 Base media
    let mut builder = DbphashBuilder::new();
    let meta = MediaMetadata {
        raw_title: "Base.Movie.2020.mkv".to_string(),
        clean_title: "Base Movie".to_string(),
        media_type: MediaType::Movie,
        year: Some(2020),
        season: None,
        episode: None,
        resolution: None,
        source: None,
        release_group: None,
        moviehash: None,
    };
    builder.add_media(&meta, 1_000_000, 24.0, &[(0x1111, 100)], &[(0x1111, 0)]);
    builder.write_to_file(&catalog_path).unwrap();

    let state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        Arc::new(dbsrt_core::OpenSubtitlesProvider::new(None)),
        dbsrt_core::SubtitleCache::default(),
    );
    state.set_dbphash(Some(catalog_path.clone()));
    let state_arc = Arc::new(state);

    let app = create_router_with_state((*state_arc).clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // Initial 1 media ingested in Delta
    let init1 = serde_json::json!({
        "filename": "Delta.One.2021.mkv",
        "hashes": vec![0x2222_u64; 20],
        "fps": 24.0
    });
    let _ = client
        .post(format!("{}/v1/catalog/ingest", base_url))
        .json(&init1)
        .send()
        .await
        .unwrap();

    // 2. Launch concurrent workers ingesting medias while compaction runs
    let n_workers = 4;
    let n_ingest_per_worker = 5;
    let mut handles = Vec::new();

    for w in 0..n_workers {
        let client_c = client.clone();
        let url_c = base_url.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..n_ingest_per_worker {
                let req = serde_json::json!({
                    "filename": format!("Concurrent.Movie.{}.{}.mkv", w, i),
                    "hashes": vec![0x3333_4444_u64; 20],
                    "fps": 24.0
                });
                let r = client_c
                    .post(format!("{}/v1/catalog/ingest", url_c))
                    .json(&req)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), reqwest::StatusCode::OK);
            }
        }));
    }

    // Simultaneously trigger compaction
    let compact_res = client
        .post(format!("{}/v1/catalog/compact", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(compact_res.status(), reqwest::StatusCode::OK);

    // Wait for all workers to finish
    for h in handles {
        h.await.unwrap();
    }

    // Check catalog status: MUST equal 1 (base) + 1 (init) + n_workers * n_ingest_per_worker
    let st: serde_json::Value = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let expected_total = 1 + 1 + (n_workers * n_ingest_per_worker);
    assert_eq!(
        st["total_medias"].as_u64().unwrap(),
        expected_total as u64,
        "Total medias must match exactly: ZERO data loss allowed during compaction!"
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_corrupted_couplings_json_fails_reload_without_clearing_state() {
    let tmp_dir =
        std::env::temp_dir().join(format!("test_corrupt_couplings_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let phash_dir = tmp_dir.join("phash");
    let subtitles_dir = tmp_dir.join("SRT");
    let catalog_path = tmp_dir.join("catalog.dbphash");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&subtitles_dir).unwrap();

    // Valid couplings file initially
    let reg_path = CouplingRegistry::default_path(&subtitles_dir);
    let mut reg = CouplingRegistry::default();
    reg.couplings.push(CertifiedMediaCoupling {
        media_hash: "11112222333344445555666677778888".to_string(),
        media_name: "Certified Movie".to_string(),
        phash_filename: "Certified.Movie.phash".to_string(),
        fps: 24.0,
        frame_count: 500,
        subtitles: vec![CertifiedSubtitleRecord {
            subtitles_hash: "abcdef0123456789".to_string(),
            name: "Certified.Movie.fr.srt".to_string(),
            lang: "fr".to_string(),
            lang_status: LangStatus::Detected("fr".to_string()),
            srt_path: "Certified.Movie.fr.srt".to_string(),
            certified: true,
            cue_count: 100,
            sub_frames: vec![],
        }],
    });
    reg.save(&reg_path).unwrap();

    let mut builder = DbphashBuilder::new();
    let meta = MediaMetadata {
        raw_title: "Certified.Movie.mkv".to_string(),
        clean_title: "Certified Movie".to_string(),
        media_type: MediaType::Movie,
        year: Some(2021),
        season: None,
        episode: None,
        resolution: None,
        source: None,
        release_group: None,
        moviehash: None,
    };
    builder.add_media(&meta, 1_000_000, 24.0, &[(0x1234, 100)], &[(0x1234, 0)]);
    builder.write_to_file(&catalog_path).unwrap();

    let state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        Arc::new(dbsrt_core::OpenSubtitlesProvider::new(None)),
        dbsrt_core::SubtitleCache::default(),
    );
    state.set_dbphash(Some(catalog_path.clone()));
    let state_arc = Arc::new(state);

    let app = create_router_with_state((*state_arc).clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // Initial check: 1 certified coupling
    let st: serde_json::Value = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(st["certified_couplings"], 1);

    // Corrupt couplings.json with invalid JSON
    std::fs::write(&reg_path, b"INVALID_JSON_CORRUPTED").unwrap();

    // Trigger reload: must FAIL and preserve existing 1 certified coupling in RAM
    let reload_res = client
        .post(format!("{}/v1/catalog/reload", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        reload_res.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    let st_after: serde_json::Value = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        st_after["certified_couplings"], 1,
        "Couplings in RAM must NOT be cleared when JSON is corrupted!"
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_true_unified_merge_picks_highest_confidence() {
    let tmp_dir = std::env::temp_dir().join(format!("test_merge_conf_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let phash_dir = tmp_dir.join("phash");
    let subtitles_dir = tmp_dir.join("SRT");
    let catalog_path = tmp_dir.join("catalog.dbphash");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&subtitles_dir).unwrap();

    // 1. Base catalog with movie "BasePartial" having only 1 matching cut
    let mut builder = DbphashBuilder::new();
    let meta_base = MediaMetadata {
        raw_title: "BasePartial.2020.mkv".to_string(),
        clean_title: "BasePartial".to_string(),
        media_type: MediaType::Movie,
        year: Some(2020),
        season: None,
        episode: None,
        resolution: None,
        source: None,
        release_group: None,
        moviehash: None,
    };
    let cuts_base = [
        (0xAAAA_BBBB_CCCC_DDDD_u64, 1000u32),
        (0x9999_9999_9999_9999_u64, 2000u32),
        (0x8888_8888_8888_8888_u64, 3000u32),
    ];
    let sparse_base = [(0x7777_7777_7777_7777_u64, 0u32)];
    builder.add_media(&meta_base, 5_000_000, 24.0, &cuts_base, &sparse_base);
    builder.write_to_file(&catalog_path).unwrap();

    let state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        Arc::new(dbsrt_core::OpenSubtitlesProvider::new(None)),
        dbsrt_core::SubtitleCache::default(),
    );
    state.set_dbphash(Some(catalog_path.clone()));
    let state_arc = Arc::new(state);

    let app = create_router_with_state((*state_arc).clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 2. Ingest "DeltaWinner" into DeltaIndex: distinct frames, the first one shared with Base
    let delta_hashes: Vec<u64> = std::iter::once(0xAAAA_BBBB_CCCC_DDDD_u64)
        .chain((1..100u64).map(|k| k.wrapping_mul(0x9E37_79B9_7F4A_7C15)))
        .collect();
    let ingest_req = serde_json::json!({
        "filename": "DeltaWinner.2024.mkv",
        "hashes": delta_hashes,
        "fps": 24.0
    });
    let _ = client
        .post(format!("{}/v1/catalog/ingest", base_url))
        .json(&ingest_req)
        .send()
        .await
        .unwrap();

    // 3. Query with the first 4 Delta frames: Base matches 1 of them (1 cut), Delta all 4.
    // Unified merge must evaluate both tiers and pick DeltaWinner because of higher confidence.
    let rec_payload = RecognizeRequest {
        media_path: "query.mp4".to_string(),
        query_fingerprint: Some(
            delta_hashes[..4]
                .iter()
                .map(|h| format!("{h:#018x}"))
                .collect(),
        ),
        fps: Some(24.0),
        language: Some("en".to_string()),
        client_version: Some("2.1.0".to_string()),
        search_distance: 2,
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
    assert_eq!(rec_json["found"], true);
    assert_eq!(rec_json["media_match"], "DeltaWinner");

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_catalog_handlers_coverage_edge_cases() {
    let tmp_dir = std::env::temp_dir().join(format!("test_cat_handlers_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let phash_dir = tmp_dir.join("phash");
    let subtitles_dir = tmp_dir.join("SRT");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&subtitles_dir).unwrap();

    let state = AppState::new(
        phash_dir.clone(),
        subtitles_dir.clone(),
        Arc::new(dbsrt_core::OpenSubtitlesProvider::new(None)),
        dbsrt_core::SubtitleCache::default(),
    );
    let state_arc = Arc::new(state);

    let app = create_router_with_state((*state_arc).clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 1. GET /v1/catalog/status
    let status_res = client
        .get(format!("{}/v1/catalog/status", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(status_res.status(), reqwest::StatusCode::OK);

    // 2. Ingest with missing both phash_path and hashes -> 400 Bad Request
    let bad_req = serde_json::json!({
        "filename": "NoHashes.mkv"
    });
    let r_bad = client
        .post(format!("{}/v1/catalog/ingest", base_url))
        .json(&bad_req)
        .send()
        .await
        .unwrap();
    assert_eq!(r_bad.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3. Ingest with non-existent phash_path -> 400 Bad Request
    let bad_path_req = serde_json::json!({
        "phash_path": tmp_dir.join("non_existent.phash").to_str().unwrap()
    });
    let r_bad_path = client
        .post(format!("{}/v1/catalog/ingest", base_url))
        .json(&bad_path_req)
        .send()
        .await
        .unwrap();
    assert_eq!(r_bad_path.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4. Ingest with directory as phash_path (unreadable file) -> 400 Bad Request
    let dir_req = serde_json::json!({
        "phash_path": tmp_dir.to_str().unwrap()
    });
    let r_dir = client
        .post(format!("{}/v1/catalog/ingest", base_url))
        .json(&dir_req)
        .send()
        .await
        .unwrap();
    assert_eq!(r_dir.status(), reqwest::StatusCode::BAD_REQUEST);

    // 5. Ingest with valid phash_path file -> 200 OK
    let valid_phash_file = tmp_dir.join("ValidMovie.2024.phash");
    dbsrt_core::write_phash_file(&valid_phash_file, &[0x1111_2222_3333_4444_u64; 20]).unwrap();
    let valid_req = serde_json::json!({
        "phash_path": valid_phash_file.to_str().unwrap()
    });
    let r_valid = client
        .post(format!("{}/v1/catalog/ingest", base_url))
        .json(&valid_req)
        .send()
        .await
        .unwrap();
    assert_eq!(r_valid.status(), reqwest::StatusCode::OK);

    // 6. Compact catalog with empty delta or flushed delta -> 200 OK
    let compact_res = client
        .post(format!("{}/v1/catalog/compact", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(compact_res.status(), reqwest::StatusCode::OK);

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}
