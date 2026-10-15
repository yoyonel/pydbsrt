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
        assert!(auto_data.output_srt_path.ends_with(".en.synced.srt"));
        assert!(Path::new(&auto_data.output_srt_path).exists());
    }
}
