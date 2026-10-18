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

    // 5. Call GET /v1/couplings and GET /studio
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

    let studio_html = client
        .get(format!("{}/studio", base_url))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(studio_html.contains("dbsrt Studio"));

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_base);
}
