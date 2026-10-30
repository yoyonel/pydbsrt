use dbsrt_server::{
    create_router, format_unix_to_iso, MonitoringTelemetryResponse, SupervisorHeartbeatPayload,
};
use std::net::SocketAddr;
use std::path::PathBuf;

#[tokio::test]
async fn test_server_monitoring_endpoints() {
    let tmp_dir = std::env::temp_dir();
    let test_phash_dir = tmp_dir.join(format!("test_phash_mon_{}", std::process::id()));
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

    // 1. Test /monitoring HTML UI endpoint
    let res = client
        .get(format!("{}/monitoring", base_url))
        .send()
        .await
        .expect("Failed to GET /monitoring");
    assert!(res.status().is_success());
    let html = res.text().await.expect("Failed reading HTML body");
    assert!(html.contains("dbsrt Monitor"));
    assert!(html.contains("Progression File Batch"));

    // 2. Test /top alias
    let res_top = client
        .get(format!("{}/top", base_url))
        .send()
        .await
        .expect("Failed to GET /top");
    assert!(res_top.status().is_success());

    // 3. Test /v1/monitoring/telemetry JSON endpoint
    let res_tel = client
        .get(format!("{}/v1/monitoring/telemetry", base_url))
        .send()
        .await
        .expect("Failed to GET telemetry");
    assert!(res_tel.status().is_success());
    let tel: MonitoringTelemetryResponse =
        res_tel.json().await.expect("Failed parsing telemetry JSON");
    assert!(!tel.server_time_iso.is_empty());
    assert!(!tel.system.hostname.is_empty());

    // 4. Test /v1/supervisor/heartbeat POST endpoint
    let heartbeat = SupervisorHeartbeatPayload {
        supervisor_host: "rpi4-test".to_string(),
        supervisor_pid: 99999,
        state: "healthy".to_string(),
        lve_cpu_pct: Some(65.4),
        lve_cpu_avg_pct: Some(72.1),
        desired_workers: Some(4),
        running_workers: Some(4),
        recent_logs: vec!["[2026-10-30 10:00:00] ● Test log message".to_string()],
    };

    let res_hb = client
        .post(format!("{}/v1/supervisor/heartbeat", base_url))
        .json(&heartbeat)
        .send()
        .await
        .expect("Failed to POST heartbeat");
    assert_eq!(res_hb.status(), reqwest::StatusCode::OK);

    // 5. Check updated supervisor telemetry
    let res_tel2 = client
        .get(format!("{}/v1/monitoring/telemetry", base_url))
        .send()
        .await
        .expect("Failed to GET telemetry after heartbeat");
    let tel2: MonitoringTelemetryResponse = res_tel2
        .json()
        .await
        .expect("Failed parsing updated telemetry");
    assert_eq!(tel2.supervisor.supervisor_host, "rpi4-test");
    assert_eq!(tel2.supervisor.supervisor_pid, Some(99999));
    assert!(tel2.supervisor.is_online);
    assert_eq!(tel2.supervisor.lve_cpu_pct, Some(65.4));
    assert_eq!(tel2.supervisor.recent_logs.len(), 1);

    // Clean up temporary run file
    let _ = std::fs::remove_file("run/supervisor_status.json");
    let _ = std::fs::remove_dir_all(&test_phash_dir);
}

#[test]
fn test_format_unix_to_iso_algorithm() {
    assert_eq!(format_unix_to_iso(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_unix_to_iso(1793352226), "2026-10-30T09:23:46Z");
}
