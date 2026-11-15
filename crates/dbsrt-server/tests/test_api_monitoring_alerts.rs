use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};
use dbsrt_server::{
    collect_monitoring_alerts, create_router_with_state, AlertHealthStatus, AlertSeverity,
    AppState, MonitoringAlertsConfig, MonitoringAlertsResponse,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_monitoring_alerts_endpoint_healthy_and_degraded() {
    let mon_root = TempDir::new().expect("create temp monitoring root");
    let phash_dir = mon_root.path().join("phash");
    let srt_dir = mon_root.path().join("srt");
    std::fs::create_dir_all(&phash_dir).unwrap();
    std::fs::create_dir_all(&srt_dir).unwrap();

    let state = AppState::new(
        phash_dir,
        srt_dir,
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    )
    .with_monitoring_root(mon_root.path().to_path_buf());

    let app = create_router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let alerts_url = format!("http://{addr}/v1/monitoring/alerts");

    // 1. Nominal healthy state on empty isolated root
    let res_ok = client.get(&alerts_url).send().await.unwrap();
    assert!(res_ok.status().is_success());
    assert!(res_ok.headers().contains_key("x-server-time-ms"));
    let body_ok: MonitoringAlertsResponse = res_ok.json().await.unwrap();
    assert_eq!(body_ok.status, AlertHealthStatus::Ok);
    assert_eq!(body_ok.active_alerts_count, 0);
    assert!(body_ok.alerts.is_empty());
    assert!(!body_ok.metrics.batch_aborted);
    assert_eq!(body_ok.metrics.supervisor_state, "absent");

    // 2. Verify /monitoring HTML embeds persistent alerts banner
    let html = client
        .get(format!("http://{addr}/monitoring"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("id=\"alerts-banner\""));
    assert!(html.contains("v1/monitoring/alerts"));

    // 3. Inject degraded supervisor + batch_aborted + queue failures
    let run_dir = mon_root.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("supervisor_status.json"),
        serde_json::json!({
            "timestamp": "2026-11-15T16:00:00Z",
            "unix_ts": 100,
            "supervisor_host": "rpi4",
            "supervisor_pid": 4242,
            "state": "degraded",
            "launch_error_count": 3,
            "failed_batch_types": {
                "freebox": "exit 2: targets missing"
            }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        run_dir.join("batch_aborted"),
        "2026-11-15T16:05:00Z FTP auth failed ftp://admin:supersecret@nas.local/tv/ep1.mkv\n",
    )
    .unwrap();
    std::fs::write(
        run_dir.join("batch_queue.json"),
        serde_json::json!({
            "total_targets": 20,
            "completed_targets": 10,
            "failed_targets": 2,
            "remaining_targets": 8,
            "progress_pct": 50.0,
            "active_workers": 0,
            "is_active": false,
            "updated_at": "2026-11-15T16:05:00Z"
        })
        .to_string(),
    )
    .unwrap();

    let res_crit = client.get(&alerts_url).send().await.unwrap();
    let body_crit: MonitoringAlertsResponse = res_crit.json().await.unwrap();
    assert_eq!(body_crit.status, AlertHealthStatus::Critical);
    assert!(body_crit.active_alerts_count >= 3);
    assert!(body_crit.metrics.batch_aborted);
    assert_eq!(body_crit.metrics.supervisor_state, "degraded");
    assert_eq!(body_crit.metrics.supervisor_launch_errors, 3);
    assert_eq!(body_crit.metrics.failed_queue_targets, 2);

    let ids: Vec<&str> = body_crit.alerts.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"supervisor_degraded"));
    assert!(ids.contains(&"supervisor_stale_heartbeat"));
    assert!(ids.contains(&"batch_aborted"));
    assert!(ids.contains(&"batch_queue_failures"));

    let joined_summaries: String = body_crit
        .alerts
        .iter()
        .map(|a| a.summary.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined_summaries.contains("supersecret"));
    assert!(joined_summaries.contains("ftp://admin:***@nas.local/tv/ep1.mkv"));
}

#[test]
fn test_collect_alerts_log_dedup_blacklist_faultwatch_and_staleness() {
    let root = TempDir::new().unwrap();
    let logs_dir = root.path().join("logs");
    let fw_dir = logs_dir.join("faultwatch");
    let reports_dir = root.path().join("data/reports");
    std::fs::create_dir_all(&fw_dir).unwrap();
    std::fs::create_dir_all(&reports_dir).unwrap();

    // Reference timestamp: 2026-10-30T09:23:46Z -> 1_793_352_226
    let now_ts: u64 = 1_793_352_226;

    // 1. Log lines: two identical signatures within 1h (different PIDs), one 12h old, one >96h old (expired)
    let worker_log = "\
[2026-10-25 01:00:00] ❌ Old historical failure pid 111\n\
[2026-10-29 21:23:46] ERROR Worker timeout after 30s on job 10\n\
[2026-10-30 09:00:00] ❌ FFmpeg decode failed pid 501 -ftp-password topsec\n\
[2026-10-30 09:10:00] ❌ FFmpeg decode failed pid 992 -ftp-password topsec\n";
    std::fs::write(logs_dir.join("batch_worker.log"), worker_log).unwrap();

    // 2. Blacklisted media: stem_bad has 3 consecutive failures; stem_recovered has 3 failures then 1 success
    let events_ndjson = "\
{\"video_stem\":\"stem_bad\",\"success\":false,\"status\":\"FAILED\",\"timestamp_iso\":\"2026-10-30T08:00:00Z\"}\n\
{\"video_stem\":\"stem_bad\",\"success\":false,\"status\":\"FAILED\",\"timestamp_iso\":\"2026-10-30T08:10:00Z\"}\n\
{\"video_stem\":\"stem_bad\",\"success\":false,\"status\":\"ERROR\",\"timestamp_iso\":\"2026-10-30T08:20:00Z\"}\n\
{\"video_stem\":\"stem_recovered\",\"success\":false,\"status\":\"FAILED\",\"timestamp_iso\":\"2026-10-30T07:00:00Z\"}\n\
{\"video_stem\":\"stem_recovered\",\"success\":false,\"status\":\"FAILED\",\"timestamp_iso\":\"2026-10-30T07:10:00Z\"}\n\
{\"video_stem\":\"stem_recovered\",\"success\":false,\"status\":\"FAILED\",\"timestamp_iso\":\"2026-10-30T07:20:00Z\"}\n\
{\"video_stem\":\"stem_recovered\",\"success\":true,\"status\":\"COMPLETED\",\"timestamp_iso\":\"2026-10-30T07:30:00Z\"}\n";
    std::fs::write(
        reports_dir.join("batch_tvshow_nightly.jsonl"),
        events_ndjson,
    )
    .unwrap();

    // 3. CloudLinux LVE faultwatch saturation entry within 24h
    std::fs::write(
        fw_dir.join("2026-10-30.jsonl"),
        "{\"minute\":\"2026-10-30T09:15Z\",\"secs_over_100\":4,\"run_delay_ms_max\":650.0}\n",
    )
    .unwrap();

    // 4. Stale nas_ftp_index.json: set now_ts far ahead of file mtime if needed, or check via future now_ts
    let nas_index_path = reports_dir.join("nas_ftp_index.json");
    std::fs::write(&nas_index_path, "{}").unwrap();
    let mtime_secs = std::fs::metadata(&nas_index_path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // First check with deterministic `now_ts` for log dedup + blacklist + faultwatch
    let cfg = MonitoringAlertsConfig {
        root_dir: Some(root.path().to_path_buf()),
        now_unix_secs: now_ts,
    };
    let resp = collect_monitoring_alerts(&cfg);
    assert_eq!(resp.status, AlertHealthStatus::Critical);
    assert_eq!(resp.metrics.blacklisted_media_count, 1);
    assert_eq!(resp.metrics.log_error_signatures, 2); // expired 2026-10-25 excluded, two PIDs collapsed!
    assert_eq!(resp.metrics.log_errors_1h, 2);
    assert_eq!(resp.metrics.log_errors_24h, 3);
    assert_eq!(resp.metrics.log_errors_72h, 3);
    assert_eq!(resp.metrics.lve_fault_entries_24h, 1);

    let crit_log = resp
        .alerts
        .iter()
        .find(|a| a.component == "batch_worker" && a.id.starts_with("log_error_"))
        .expect("deduplicated critical log alert");
    assert_eq!(crit_log.severity, AlertSeverity::Critical);
    assert_eq!(crit_log.count, 2);
    assert!(!crit_log.summary.contains("topsec"));

    // Second check with `now_unix_secs = mtime_secs + 100_000` (> 86_400s) to assert NAS index staleness
    let cfg_stale = MonitoringAlertsConfig {
        root_dir: Some(root.path().to_path_buf()),
        now_unix_secs: mtime_secs + 100_000,
    };
    let resp_stale = collect_monitoring_alerts(&cfg_stale);
    assert!(resp_stale
        .alerts
        .iter()
        .any(|a| a.id == "nas_ftp_index_stale"));
}

#[test]
fn test_batch_worker_multisession_undated_scoping_debug_ignore_and_faultwatch_jsonl_only() {
    let root = TempDir::new().unwrap();
    let logs_dir = root.path().join("logs");
    let fw_dir = logs_dir.join("faultwatch");
    std::fs::create_dir_all(&fw_dir).unwrap();

    // Multi-session append-only batch_worker.log: historical session had undated ❌ + [DEBUG],
    // latest session after `=== [1/4]` is clean except for a benign `[DEBUG] dbsrt ERROR` trace.
    let worker_log = "\
=== [1/4] Scanning local catalogs and couplings ===\n\
❌ [WATCHDOG] historical timeout in previous batch run\n\
❌ [35 FAILED] old media target\n\
=== [1/4] Scanning local catalogs and couplings ===\n\
--> [DEBUG] dbsrt ERROR (returncode 1): benign diagnostic probe\n\
✅ [1/466] Freebox episode indexed\n";
    std::fs::write(logs_dir.join("batch_worker.log"), worker_log).unwrap();

    // Plain-text watch.log inside logs/faultwatch/ must NOT be counted as LVE JSONL faults
    std::fs::write(
        fw_dir.join("watch.log"),
        "plain text daemon startup banner line 1\nline 2\n",
    )
    .unwrap();

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let resp = collect_monitoring_alerts(&MonitoringAlertsConfig {
        root_dir: Some(root.path().to_path_buf()),
        now_unix_secs: now_ts,
    });
    assert_eq!(resp.status, AlertHealthStatus::Ok);
    assert_eq!(resp.active_alerts_count, 0);
    assert_eq!(resp.metrics.lve_fault_entries_24h, 0);
}
