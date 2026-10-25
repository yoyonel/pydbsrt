use dbsrt_server::{
    create_router, AutoSyncRequest, AutoSyncResponse, HealthResponse, RetargetRequest,
    RetargetResponse, SearchSubtitlesRequest, SearchSubtitlesResponse, StatusResponse,
};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[tokio::test]
async fn test_server_health_retarget_and_autosync() {
    let tmp_dir = std::env::temp_dir();
    let test_phash_dir = tmp_dir.join(format!("test_phash_health_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&test_phash_dir);
    let srt_dir = PathBuf::from("../../data/SRT");
    let app = create_router(test_phash_dir.clone(), srt_dir.clone());

    // Bind on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().unwrap();

    // Spawn server in background task
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 1. Test /v1/health
    let res = client
        .get(format!("{}/v1/health", base_url))
        .send()
        .await
        .expect("Failed to call health");
    assert!(res.status().is_success());
    let health: HealthResponse = res.json().await.expect("Failed to parse health json");
    assert_eq!(health.status, "ok");
    assert_eq!(health.service, "dbsrt-server");

    // 2. Test /v1/retarget with Big Buck Bunny
    let ref_media = Path::new("../../data/big_buck_bunny_trailer_480p.webm");
    let ref_srt = Path::new("../../data/SRT/big_buck_bunny_trailer_480p.en.srt");
    let target_media = Path::new("../../tests/data/big_buck_bunny_trailer_480p.mp4");

    if ref_media.exists() && ref_srt.exists() && target_media.exists() {
        let payload = RetargetRequest {
            target_media: target_media.to_str().unwrap().to_string(),
            ref_subtitles: ref_srt.to_str().unwrap().to_string(),
            ref_media: Some(ref_media.to_str().unwrap().to_string()),
            phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
            search_distance: 0,
        };

        let res = client
            .post(format!("{}/v1/retarget", base_url))
            .json(&payload)
            .send()
            .await
            .expect("Failed to call retarget");

        assert!(res.status().is_success());
        let retarget_res: RetargetResponse =
            res.json().await.expect("Failed to parse retarget json");
        assert!(retarget_res.success);
        assert_eq!(retarget_res.shift_frames, 0);
        assert_eq!(retarget_res.shift_ms, 0.0);
        assert!(!retarget_res.output_srt_path.is_empty());
        assert!(Path::new(&retarget_res.output_srt_path).exists());

        // 3. Test /v1/autosync (Cas 1: Pure video input, zero client SRT provided!)
        let autosync_payload = AutoSyncRequest {
            media_path: target_media.to_str().unwrap().to_string(),
            language: "en".to_string(),
            phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
            subtitles_dir: Some(srt_dir.to_str().unwrap().to_string()),
            search_distance: 0,
            nb_seconds: 1.0,
        };

        let auto_res = client
            .post(format!("{}/v1/autosync", base_url))
            .json(&autosync_payload)
            .send()
            .await
            .expect("Failed to call autosync");

        assert!(auto_res.status().is_success());
        let auto_data: AutoSyncResponse = auto_res
            .json()
            .await
            .expect("Failed to parse autosync json");
        assert!(
            auto_data.success,
            "AutoSync should succeed: {:?}",
            auto_data.error
        );
        assert_eq!(auto_data.language, "en");
        assert_eq!(auto_data.shift_frames, 0);
        assert_eq!(auto_data.shift_ms, 0.0);
        // 4. Test /v1/recognize
        let rec_payload = dbsrt_server::RecognizeRequest {
            media_path: target_media.to_str().unwrap().to_string(),
            search_distance: 0,
            nb_seconds: 1.0,
            phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
            ..Default::default()
        };
        let rec_res = client
            .post(format!("{}/v1/recognize", base_url))
            .json(&rec_payload)
            .send()
            .await
            .expect("Failed to call recognize");
        assert!(rec_res.status().is_success());
        let rec_data: dbsrt_server::RecognizeResponse = rec_res.json().await.expect("parse rec");
        assert_eq!(rec_data.media_name_search, "big_buck_bunny_trailer_480p");

        // 5. Test /v1/scan
        let scan_payload = dbsrt_server::ScanRequest {
            query_media: target_media.to_str().unwrap().to_string(),
            phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
            search_distance: 0,
            limit: 5,
        };
        let scan_res = client
            .post(format!("{}/v1/scan", base_url))
            .json(&scan_payload)
            .send()
            .await
            .expect("Failed to call scan");
        assert!(scan_res.status().is_success());
        let scan_data: dbsrt_server::ScanResponse = scan_res.json().await.expect("parse scan");
        assert!(scan_data.elapsed_seconds >= 0.0);

        // 6. Test Error Handling (HTTP 400 Bad Request on missing media)
        let bad_rec = dbsrt_server::RecognizeRequest {
            media_path: "/tmp/non_existent_file_path_12345.mp4".to_string(),
            search_distance: 0,
            nb_seconds: 1.0,
            phash_dir: None,
            ..Default::default()
        };
        let bad_rec_res = client
            .post(format!("{}/v1/recognize", base_url))
            .json(&bad_rec)
            .send()
            .await
            .expect("call bad recognize");
        assert_eq!(bad_rec_res.status(), reqwest::StatusCode::BAD_REQUEST);

        let bad_scan = dbsrt_server::ScanRequest {
            query_media: "/tmp/non_existent_file_path_12345.mp4".to_string(),
            phash_dir: None,
            search_distance: 0,
            limit: 5,
        };
        let bad_scan_res = client
            .post(format!("{}/v1/scan", base_url))
            .json(&bad_scan)
            .send()
            .await
            .expect("call bad scan");
        assert_eq!(bad_scan_res.status(), reqwest::StatusCode::BAD_REQUEST);

        let bad_auto = AutoSyncRequest {
            media_path: "/tmp/non_existent_file_path_12345.mp4".to_string(),
            language: "en".to_string(),
            phash_dir: None,
            subtitles_dir: None,
            search_distance: 0,
            nb_seconds: 1.0,
        };
        let bad_auto_res = client
            .post(format!("{}/v1/autosync", base_url))
            .json(&bad_auto)
            .send()
            .await
            .expect("call bad autosync");
        assert!(bad_auto_res.status().is_success());
        let bad_auto_data: AutoSyncResponse = bad_auto_res.json().await.expect("parse auto");
        assert!(!bad_auto_data.success);
        assert!(bad_auto_data.error.is_some());

        // Test missing subtitle catalog match (graceful success: false)
        let empty_srt_dir = std::env::temp_dir().join("empty_srt_dir_test_404");
        let _ = std::fs::create_dir_all(&empty_srt_dir);
        let unk_lang_auto = AutoSyncRequest {
            media_path: target_media.to_str().unwrap().to_string(),
            language: "non_existent_lang_xyz".to_string(),
            phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
            subtitles_dir: Some(empty_srt_dir.to_str().unwrap().to_string()),
            search_distance: 0,
            nb_seconds: 1.0,
        };
        let unk_lang_res = client
            .post(format!("{}/v1/autosync", base_url))
            .json(&unk_lang_auto)
            .send()
            .await
            .expect("call unk lang autosync");
        assert!(unk_lang_res.status().is_success());
        let unk_lang_data: AutoSyncResponse = unk_lang_res.json().await.expect("parse unk");
        assert!(!unk_lang_data.success);
        assert!(unk_lang_data.error.is_some());
        let _ = std::fs::remove_dir_all(&empty_srt_dir);

        // Test retarget missing files error responses (graceful success: false)
        let bad_retarget = RetargetRequest {
            target_media: "/tmp/non_existent_file_path_12345.mp4".to_string(),
            ref_subtitles: ref_srt.to_str().unwrap().to_string(),
            ref_media: None,
            phash_dir: None,
            search_distance: 0,
        };
        let bad_ret_res = client
            .post(format!("{}/v1/retarget", base_url))
            .json(&bad_retarget)
            .send()
            .await
            .expect("call bad retarget");
        assert!(bad_ret_res.status().is_success());
        let bad_ret_data: RetargetResponse = bad_ret_res.json().await.expect("parse ret");
        assert!(!bad_ret_data.success);
        assert!(bad_ret_data.error.is_some());
    }
}

#[tokio::test]
async fn test_autosync_embedded_container_subtitle() {
    let tmp_dir = std::env::temp_dir();
    let empty_subtitles_dir = tmp_dir.join(format!("empty_sub_dir_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&empty_subtitles_dir);

    let test_mkv = tmp_dir.join(format!("test_autosync_embedded_{}.mkv", std::process::id()));
    let srt_path = PathBuf::from("../../tests/data/big_buck_bunny_trailer_480p.en.srt");

    if !srt_path.exists() {
        let _ = std::fs::remove_dir_all(&empty_subtitles_dir);
        return;
    }

    // Mux black video + embedded french subtitle
    let status = std::process::Command::new("ffmpeg")
        .stdin(std::process::Stdio::null())
        .args([
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:d=2",
            "-f",
            "srt",
            "-i",
        ])
        .arg(&srt_path)
        .args([
            "-map",
            "0:v",
            "-map",
            "1:s",
            "-c:v",
            "mjpeg",
            "-c:s",
            "srt",
            "-metadata:s:s:0",
            "language=fra",
            "-metadata:s:s:0",
            "title=French",
        ])
        .arg(&test_mkv)
        .status();

    if !matches!(status, Ok(s) if s.success()) {
        let _ = std::fs::remove_dir_all(&empty_subtitles_dir);
        let _ = std::fs::remove_file(&test_mkv);
        return;
    }

    let test_phash_dir = tmp_dir.join(format!("test_phash_auto_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&test_phash_dir);
    let app = create_router(test_phash_dir.clone(), empty_subtitles_dir.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    let payload = AutoSyncRequest {
        media_path: test_mkv.to_str().unwrap().to_string(),
        language: "fr".to_string(),
        phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
        subtitles_dir: Some(empty_subtitles_dir.to_str().unwrap().to_string()),
        search_distance: 0,
        nb_seconds: 1.0,
    };

    let res = client
        .post(format!("{}/v1/autosync", base_url))
        .json(&payload)
        .send()
        .await
        .expect("Call autosync with embedded subtitle");

    assert!(res.status().is_success());
    let auto_res: AutoSyncResponse = res.json().await.expect("Parse autosync response");

    assert!(auto_res.success, "Autosync failed: {:?}", auto_res.error);
    assert_eq!(auto_res.language, "fr");
    assert!(!auto_res.output_srt_path.is_empty());
    assert!(Path::new(&auto_res.output_srt_path).exists());

    // Verify content
    let saved_content = std::fs::read_to_string(&auto_res.output_srt_path).unwrap();
    assert!(saved_content.contains("peach open movie project"));

    // Cleanup
    let _ = std::fs::remove_file(&auto_res.output_srt_path);
    let _ = std::fs::remove_file(&test_mkv);
    let _ = std::fs::remove_dir_all(&empty_subtitles_dir);
}

#[tokio::test]
async fn test_server_status_and_search_endpoints() {
    let tmp_dir = std::env::temp_dir();
    let test_phash_dir = tmp_dir.join(format!("test_phash_status_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&test_phash_dir);
    let srt_dir = PathBuf::from("../../data/SRT");
    let app = create_router(test_phash_dir.clone(), srt_dir.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 1. Test /health (alias)
    let res = client
        .get(format!("{}/health", base_url))
        .send()
        .await
        .expect("Failed to call /health");
    assert!(res.status().is_success());

    // 2. Test /v1/status
    let res = client
        .get(format!("{}/v1/status", base_url))
        .send()
        .await
        .expect("Failed to call /v1/status");
    assert!(res.status().is_success());
    let status: StatusResponse = res.json().await.expect("Failed to parse status JSON");
    assert_eq!(status.status, "ok");
    assert_eq!(status.service, "dbsrt-server");
    assert!(!status.version.is_empty());

    // 3. Test /v1/search for Big Buck Bunny
    let original_media = "../../tests/data/big_buck_bunny_trailer_480p.mp4";
    let tmp_media = tmp_dir.join("big_buck_bunny_trailer_480p.mp4");
    let _ = std::fs::copy(original_media, &tmp_media);
    let test_media = tmp_media.to_str().unwrap();
    let search_req = SearchSubtitlesRequest {
        media_path: test_media.to_string(),
        language: "en".to_string(),
        subtitles_dir: Some(srt_dir.to_str().unwrap().to_string()),
        phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
        search_mode: Some("phash".to_string()),
        query_title: None,
        season: None,
        episode: None,
        index_engine: None,
    };

    let res = client
        .post(format!("{}/v1/search", base_url))
        .json(&search_req)
        .send()
        .await
        .expect("Failed to call /v1/search");
    assert!(res.status().is_success());
    let search_res: SearchSubtitlesResponse =
        res.json().await.expect("Failed to parse search JSON");
    assert_eq!(search_res.language, "en");
    assert!(search_res.moviehash.is_some());
    assert!(search_res.total_candidates > 0);
    assert!(search_res
        .candidates
        .iter()
        .any(|c| c.source == "certified" || c.source == "catalog"));
    // Verify strict language filtering: no "unknown" or non-matching language candidates
    assert!(search_res
        .candidates
        .iter()
        .filter(|c| c.source == "certified" || c.source == "catalog")
        .all(|c| c.language == "en"));

    // 4. Test /v1/download for selected candidate
    let local_cand = search_res
        .candidates
        .iter()
        .find(|c| c.source == "certified" || c.source == "catalog")
        .expect("Must have certified or catalog candidate");
    let dl_req = dbsrt_server::DownloadSubtitleRequest {
        media_path: test_media.to_string(),
        source: local_cand.source.clone(),
        candidate_id: local_cand.candidate_id.clone(),
        language: "en".to_string(),
        resync: true,
        shift_ms: None,
        subtitles_dir: Some(srt_dir.to_str().unwrap().to_string()),
        phash_dir: Some(tmp_dir.to_str().unwrap().to_string()),
    };

    let res = client
        .post(format!("{}/v1/download", base_url))
        .json(&dl_req)
        .send()
        .await
        .expect("Failed to call /v1/download");
    assert!(res.status().is_success());
    let dl_res: dbsrt_server::DownloadSubtitleResponse =
        res.json().await.expect("Failed to parse download JSON");
    assert!(dl_res.success);
    assert!(!dl_res.output_srt_path.is_empty());
    assert!(std::path::Path::new(&dl_res.output_srt_path).exists());

    // Cleanup
    let _ = std::fs::remove_file(&tmp_media);
    let _ = std::fs::remove_file(&dl_res.output_srt_path);
    let _ = std::fs::remove_dir_all(&test_phash_dir);
}

#[tokio::test]
async fn test_certify_and_certified_search_download_e2e() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let orig_media = workspace_root.join("tests/data/big_buck_bunny_trailer_480p.mp4");
    let orig_srt = workspace_root.join("data/SRT/big_buck_bunny_trailer_480p.fr.srt");
    if !orig_media.exists() || !orig_srt.exists() {
        return;
    }

    let tmp_base = std::env::temp_dir().join(format!("dbsrt_certify_e2e_{}", std::process::id()));
    let phash_dir = tmp_base.join("phash");
    let srt_dir = tmp_base.join("srt");
    let _ = std::fs::create_dir_all(&phash_dir);
    let _ = std::fs::create_dir_all(&srt_dir);

    let test_media = tmp_base.join("big_buck_bunny_trailer_480p.mp4");
    std::fs::copy(&orig_media, &test_media).unwrap();
    let test_srt = srt_dir.join("big_buck_bunny_trailer_480p.fr.srt");
    std::fs::copy(&orig_srt, &test_srt).unwrap();

    let (listener, addr) = dbsrt_server::bind_listener("127.0.0.1", 0).await.unwrap();
    let app = create_router(phash_dir.clone(), srt_dir.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{}", addr);
    let client = reqwest::Client::new();

    // 1. Call POST /v1/certify with valid media + SRT
    let cert_req = dbsrt_server::CertifySyncRequest {
        media_path: test_media.to_string_lossy().to_string(),
        subtitle_path: test_srt.to_string_lossy().to_string(),
        language: "fr".to_string(),
        strict: true,
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
        subtitles_dir: Some(srt_dir.to_string_lossy().to_string()),
    };

    let res = client
        .post(format!("{}/v1/certify", base_url))
        .json(&cert_req)
        .send()
        .await
        .expect("Failed to call /v1/certify");
    assert!(res.status().is_success());
    let cert_res: dbsrt_server::CertifySyncResponse =
        res.json().await.expect("Failed to parse /v1/certify JSON");
    assert!(cert_res.success, "Certify error: {:?}", cert_res.error);
    assert_eq!(cert_res.cue_count, 8);
    assert_eq!(cert_res.sub_frames_count, 8);
    assert_eq!(cert_res.media_hash.len(), 32);
    assert_eq!(cert_res.subtitles_hash.len(), 32);
    assert!(std::path::Path::new(&cert_res.registry_path).exists());

    // 2. Call POST /v1/certify with an out-of-bounds cue -> must be rejected
    let bad_srt = tmp_base.join("oob.srt");
    std::fs::write(
        &bad_srt,
        "1\n02:30:00,000 --> 02:30:05,000\nCue way past 33s video duration\n\n",
    )
    .unwrap();
    let bad_req = dbsrt_server::CertifySyncRequest {
        media_path: test_media.to_string_lossy().to_string(),
        subtitle_path: bad_srt.to_string_lossy().to_string(),
        language: "fr".to_string(),
        strict: true,
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
        subtitles_dir: Some(srt_dir.to_string_lossy().to_string()),
    };
    let bad_res: dbsrt_server::CertifySyncResponse = client
        .post(format!("{}/v1/certify", base_url))
        .json(&bad_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !bad_res.success,
        "Out-of-bounds subtitle must fail certification"
    );
    assert!(bad_res
        .error
        .unwrap_or_default()
        .contains("exceeds media duration"));

    // 3. Call POST /v1/search (search_mode="phash", language="fr") -> first candidate must be source="certified"
    let search_req = dbsrt_server::SearchSubtitlesRequest {
        media_path: test_media.to_string_lossy().to_string(),
        language: "fr".to_string(),
        subtitles_dir: Some(srt_dir.to_string_lossy().to_string()),
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
        search_mode: Some("phash".to_string()),
        query_title: None,
        season: None,
        episode: None,
        index_engine: None,
    };
    let search_res: dbsrt_server::SearchSubtitlesResponse = client
        .post(format!("{}/v1/search", base_url))
        .json(&search_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(search_res.matched_media.is_some());
    assert!(!search_res.candidates.is_empty());
    let top_cand = &search_res.candidates[0];
    assert_eq!(top_cand.source, "certified");
    assert_eq!(top_cand.language, "fr");

    // 4. Call POST /v1/download for the certified candidate
    let dl_req = dbsrt_server::DownloadSubtitleRequest {
        media_path: test_media.to_string_lossy().to_string(),
        source: "certified".to_string(),
        candidate_id: top_cand.candidate_id.clone(),
        language: "fr".to_string(),
        resync: true,
        shift_ms: None,
        subtitles_dir: Some(srt_dir.to_string_lossy().to_string()),
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
    };
    let dl_res: dbsrt_server::DownloadSubtitleResponse = client
        .post(format!("{}/v1/download", base_url))
        .json(&dl_req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(dl_res.success);
    assert_eq!(dl_res.source, "certified");
    assert!(std::path::Path::new(&dl_res.output_srt_path).exists());

    // 5. Call GET /v1/couplings (full) and GET /v1/couplings?summary=true
    let couplings_res: dbsrt_server::CouplingsOverviewResponse = client
        .get(format!("{}/v1/couplings", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!couplings_res.couplings.is_empty());
    assert_eq!(couplings_res.couplings[0].subtitles.len(), 1);
    assert_eq!(couplings_res.couplings[0].subtitles[0].sub_frames.len(), 8);

    let summary_res: dbsrt_server::CouplingsOverviewResponse = client
        .get(format!("{}/v1/couplings?summary=true", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!summary_res.couplings.is_empty());
    assert!(summary_res.couplings[0].subtitles[0].sub_frames.is_empty());

    // 6. Call GET /v1/couplings/sub_frames (pagination, search q, cue_index, target_frame, 404s)
    let m_hash = &couplings_res.couplings[0].media_hash;
    let s_hash = &couplings_res.couplings[0].subtitles[0].subtitles_hash;

    let sf_page: dbsrt_server::SubFramesPageResponse = client
        .get(format!(
            "{}/v1/couplings/sub_frames?media_hash={}&subtitles_hash={}&page=1&page_size=3",
            base_url, m_hash, s_hash
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sf_page.total_cues, 8);
    assert_eq!(sf_page.items.len(), 3);
    assert_eq!(sf_page.total_pages, 3);
    assert!(!sf_page.minimap_ticks.is_empty());

    // Filter with q
    let sf_q: dbsrt_server::SubFramesPageResponse = client
        .get(format!(
            "{}/v1/couplings/sub_frames?media_hash={}&q=Big&page_size=10",
            base_url, m_hash
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sf_q.filtered_cues >= 1);

    // Jump by cue_index and target_frame
    let sf_cue: dbsrt_server::SubFramesPageResponse = client
        .get(format!(
            "{}/v1/couplings/sub_frames?media_hash={}&cue_index=2&page_size=2",
            base_url, m_hash
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sf_cue.selected_cue.is_some());

    let sf_frame: dbsrt_server::SubFramesPageResponse = client
        .get(format!(
            "{}/v1/couplings/sub_frames?media_hash={}&target_frame=100&page_size=2",
            base_url, m_hash
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sf_frame.selected_cue.is_some());

    // 404 cases on /v1/couplings/sub_frames
    let nf_media = client
        .get(format!(
            "{}/v1/couplings/sub_frames?media_hash=non_existent_hash",
            base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(nf_media.status(), reqwest::StatusCode::NOT_FOUND);

    let nf_sub = client
        .get(format!(
            "{}/v1/couplings/sub_frames?media_hash={}&subtitles_hash=unknown_sub",
            base_url, m_hash
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(nf_sub.status(), reqwest::StatusCode::NOT_FOUND);

    // 7. Test search_mode="hash", search_mode="name", search_mode="auto", and download resync=false
    for mode in ["hash", "name", "auto"] {
        let s_req = dbsrt_server::SearchSubtitlesRequest {
            media_path: test_media.to_string_lossy().to_string(),
            language: "fr".to_string(),
            subtitles_dir: Some(srt_dir.to_string_lossy().to_string()),
            phash_dir: Some(phash_dir.to_string_lossy().to_string()),
            search_mode: Some(mode.to_string()),
            query_title: Some("big buck bunny".to_string()),
            season: Some(1),
            episode: Some(1),
            index_engine: None,
        };
        let s_res = client
            .post(format!("{}/v1/search", base_url))
            .json(&s_req)
            .send()
            .await
            .unwrap();
        assert!(s_res.status().is_success());
    }

    let dl_no_resync = dbsrt_server::DownloadSubtitleRequest {
        media_path: test_media.to_string_lossy().to_string(),
        source: "catalog".to_string(),
        candidate_id: Some(test_srt.to_string_lossy().to_string()),
        language: "fr".to_string(),
        resync: false,
        shift_ms: None,
        subtitles_dir: Some(srt_dir.to_string_lossy().to_string()),
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
    };
    let dl_nr_res: dbsrt_server::DownloadSubtitleResponse = client
        .post(format!("{}/v1/download", base_url))
        .json(&dl_no_resync)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(dl_nr_res.success);

    let dl_invalid = dbsrt_server::DownloadSubtitleRequest {
        media_path: test_media.to_string_lossy().to_string(),
        source: "unknown_source_xyz".to_string(),
        candidate_id: None,
        language: "fr".to_string(),
        resync: false,
        shift_ms: None,
        subtitles_dir: None,
        phash_dir: None,
    };
    let dl_inv_res: dbsrt_server::DownloadSubtitleResponse = client
        .post(format!("{}/v1/download", base_url))
        .json(&dl_invalid)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!dl_inv_res.success);

    let studio_html = client
        .get(format!("{}/studio", base_url))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(studio_html.contains("dbsrt Studio"));

    // 8. Test PostgreSQL-backed AppState when PostgreSQL is reachable
    let db_host = std::env::var("PSQL_IP").unwrap_or_else(|_| "127.0.0.1".to_string());
    let env_port = std::env::var("PSQL_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok());
    let ports: Vec<u16> = match env_port {
        Some(p) => vec![p, 5433, 5432],
        None => vec![5433, 5432],
    };
    let mut reachable_port = None;
    for p in ports {
        let probe_cfg = dbsrt_server::DbConnectionConfig::new(
            db_host.clone(),
            p,
            "docker",
            "docker",
            "bktreedb",
        );
        if probe_cfg.with_client(|_| Ok(())).is_ok() {
            reachable_port = Some(p);
            break;
        }
    }
    if let Some(pg_port) = reachable_port {
        let db_cfg =
            dbsrt_server::DbConnectionConfig::new(db_host, pg_port, "docker", "docker", "bktreedb");
        let mut pg_state = dbsrt_server::AppState::new(
            phash_dir.clone(),
            srt_dir.clone(),
            std::sync::Arc::new(
                dbsrt_core::providers::opensubtitles::OpenSubtitlesProvider::new(None),
            ),
            dbsrt_core::providers::cache::SubtitleCache::default(),
        );
        pg_state.db_config = Some(db_cfg);
        let (pg_listener, pg_addr) = dbsrt_server::bind_listener("127.0.0.1", 0).await.unwrap();
        let pg_app = dbsrt_server::create_router_with_state(pg_state);
        tokio::spawn(async move {
            axum::serve(pg_listener, pg_app).await.unwrap();
        });
        let pg_base = format!("http://{}", pg_addr);

        let pg_cert: dbsrt_server::CertifySyncResponse = client
            .post(format!("{}/v1/certify", pg_base))
            .json(&cert_req)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(pg_cert.success);
        assert!(pg_cert.postgres_imported);

        let pg_status: dbsrt_server::StatusResponse = client
            .get(format!("{}/v1/status", pg_base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(pg_status.postgres_connected);

        let pg_couplings: dbsrt_server::CouplingsOverviewResponse = client
            .get(format!("{}/v1/couplings?summary=true", pg_base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(pg_couplings.postgres_connected);
        assert!(pg_couplings.postgres_counts.is_some());

        let pg_search: dbsrt_server::SearchSubtitlesResponse = client
            .post(format!("{}/v1/search", pg_base))
            .json(&search_req)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!pg_search.candidates.is_empty());
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_base);
}

#[tokio::test]
async fn test_download_latin1_subtitle_preserves_french_accents_utf8() {
    let tmp_base = std::env::temp_dir().join(format!("dbsrt_latin1_e2e_{}", std::process::id()));
    let phash_dir = tmp_base.join("phash");
    let srt_dir = tmp_base.join("srt");
    let _ = std::fs::create_dir_all(&phash_dir);
    let _ = std::fs::create_dir_all(&srt_dir);

    // Create a Latin-1 / Windows-1252 SRT file containing French accents: "Désolée, y a rien qui me vient..."
    // 0xE9 is 'é' in ISO-8859-1 / Windows-1252
    let latin1_bytes =
        b"1\r\n00:00:01,000 --> 00:00:04,000\r\nD\xe9sol\xe9e, y a rien qui me vient...\r\n\r\n";
    let latin1_srt_path = srt_dir.join("test_media.fr.srt");
    std::fs::write(&latin1_srt_path, latin1_bytes).expect("Failed to write Latin-1 SRT");

    let fake_video = tmp_base.join("test_media.mp4");
    std::fs::write(&fake_video, b"fake video content").unwrap();

    let (listener, addr) = dbsrt_server::bind_listener("127.0.0.1", 0).await.unwrap();
    let app = create_router(phash_dir.clone(), srt_dir.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{}", addr);
    let client = reqwest::Client::new();

    let dl_req = dbsrt_server::DownloadSubtitleRequest {
        media_path: fake_video.to_string_lossy().to_string(),
        source: "catalog".to_string(),
        candidate_id: Some(latin1_srt_path.to_string_lossy().to_string()),
        language: "fr".to_string(),
        resync: false,
        shift_ms: None,
        subtitles_dir: Some(srt_dir.to_string_lossy().to_string()),
        phash_dir: Some(phash_dir.to_string_lossy().to_string()),
    };

    let dl_res: dbsrt_server::DownloadSubtitleResponse = client
        .post(format!("{}/v1/download", base_url))
        .json(&dl_req)
        .send()
        .await
        .expect("Download request failed")
        .json()
        .await
        .expect("Parse download response");

    assert!(dl_res.success, "Download failed: {:?}", dl_res.error);
    let out_path = std::path::PathBuf::from(&dl_res.output_srt_path);
    assert!(out_path.exists(), "Output SRT file does not exist");

    let downloaded_bytes = std::fs::read(&out_path).expect("Read downloaded SRT");
    // Verify valid UTF-8
    let utf8_text =
        std::str::from_utf8(&downloaded_bytes).expect("Downloaded subtitle must be valid UTF-8");
    assert!(
        utf8_text.contains("Désolée, y a rien qui me vient..."),
        "Accents missing or mangled in: {}",
        utf8_text
    );
    assert!(
        !utf8_text.contains('\u{FFFD}'),
        "Found Unicode replacement character (U+FFFD)!"
    );

    let _ = std::fs::remove_dir_all(&tmp_base);
}
