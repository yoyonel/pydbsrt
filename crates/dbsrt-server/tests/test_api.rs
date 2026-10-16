use dbsrt_server::{
    create_router, AutoSyncRequest, AutoSyncResponse, HealthResponse, RetargetRequest,
    RetargetResponse,
};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[tokio::test]
async fn test_server_health_retarget_and_autosync() {
    let tmp_dir = std::env::temp_dir();
    let srt_dir = PathBuf::from("../../data/SRT");
    let app = create_router(tmp_dir.clone(), srt_dir.clone());

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
