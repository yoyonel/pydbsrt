use clap::Parser;
use dbsrt_cli::cli::{Cli, Commands};
use dbsrt_cli::commands::doctor::analyzer::{
    analyze_node_snapshot, format_unix_iso, parse_leading_timestamp, RawNodeSnapshot,
};
use dbsrt_cli::commands::doctor::constants::{DOCTOR_LOG_TAIL_BYTES, TRACKED_SCRIPTS};
use dbsrt_cli::commands::doctor::models::NodeRole;
use dbsrt_cli::commands::doctor::probe::{
    parse_ssh_probe_sections, probe_directory, probe_node, NodeProbeTarget,
};
use dbsrt_cli::commands::{
    evaluate_cluster_doctor, ClusterDoctorReport, DoctorArgs, DoctorSeverity, ParityStatus,
    DOCTOR_EXIT_ALERT, DOCTOR_EXIT_OK,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn seed_tracked_scripts(root: &Path, use_repo_layout: bool, include_rpi_only: bool) {
    for spec in TRACKED_SCRIPTS {
        if !include_rpi_only && !spec.required_on_o2switch {
            continue;
        }
        let rel = if use_repo_layout {
            spec.local_repo_path
        } else {
            spec.deployed_rel_path
        };
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(
            &full,
            format!("# canonical content for {}\n", spec.canonical_name),
        )
        .unwrap();
    }
}

#[test]
fn test_cli_parse_doctor_flags_and_alias() {
    let cli = Cli::parse_from([
        "dbsrt",
        "doctor",
        "--json",
        "--local-only",
        "--rpi4-host",
        "pi-box",
        "--o2switch-host",
        "cloud-box",
        "--timeout-secs",
        "3",
    ]);
    match cli.command {
        Commands::Doctor(args) => {
            assert!(args.json);
            assert!(args.local_only);
            assert_eq!(args.rpi4_host, "pi-box");
            assert_eq!(args.o2switch_host, "cloud-box");
            assert_eq!(args.timeout_secs, 3);
        }
        other => panic!("expected Doctor, got {other:?}"),
    }

    let alias_cli = Cli::parse_from(["dbsrt", "status:cluster", "--local-only"]);
    assert!(matches!(alias_cli.command, Commands::Doctor(_)));
}

#[test]
fn test_doctor_subprocess_local_only_healthy_exits_0() {
    let local = TempDir::new().unwrap();
    seed_tracked_scripts(local.path(), true, true);

    let bin = env!("CARGO_BIN_EXE_dbsrt");
    let out = Command::new(bin)
        .args([
            "doctor",
            "--local-only",
            "--json",
            "--local-root",
            local.path().to_str().unwrap(),
        ])
        .output()
        .expect("run dbsrt doctor --local-only --json");

    assert_eq!(
        out.status.code(),
        Some(DOCTOR_EXIT_OK),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rep: ClusterDoctorReport = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rep.status, DoctorSeverity::Ok);
    assert_eq!(rep.exit_code, DOCTOR_EXIT_OK);
    assert_eq!(rep.sha256_drift_count, 0);
    assert_eq!(rep.active_critical_count, 0);
    assert_eq!(rep.nodes.len(), 3);
    assert!(rep.nodes[0].reachable);
    assert!(rep.nodes[1].skipped);
    assert!(rep.nodes[2].skipped);
    assert!(rep
        .script_parity
        .iter()
        .all(|p| p.status == ParityStatus::LocalOnly));
}

#[test]
fn test_doctor_subprocess_sha256_parity_and_drift_exits_2() {
    let local = TempDir::new().unwrap();
    let rpi4 = TempDir::new().unwrap();
    let o2 = TempDir::new().unwrap();

    seed_tracked_scripts(local.path(), true, true);
    seed_tracked_scripts(rpi4.path(), false, true);
    seed_tracked_scripts(o2.path(), false, false);

    // 1. All 3 nodes in sync (o2switch only carries its required worker scripts) -> exit 0
    let args_ok = DoctorArgs {
        json: true,
        local_only: false,
        local_root: Some(local.path().to_path_buf()),
        rpi4_local_root: Some(rpi4.path().to_path_buf()),
        o2switch_local_root: Some(o2.path().to_path_buf()),
        ..DoctorArgs::default()
    };
    let rep_ok = evaluate_cluster_doctor(&args_ok);
    assert_eq!(rep_ok.exit_code, DOCTOR_EXIT_OK);
    assert_eq!(rep_ok.sha256_drift_count, 0);
    assert!(rep_ok
        .script_parity
        .iter()
        .all(|p| p.status == ParityStatus::Ok));

    // 2. Introduce SHA-256 drift on o2switch (diverging o2switch_batch_worker.py + stale o2switch_ctl.sh)
    std::fs::write(
        o2.path().join("scripts/o2switch_batch_worker.py"),
        "# drifted worker\n",
    )
    .unwrap();
    std::fs::write(o2.path().join("scripts/o2switch_ctl.sh"), "# stale ctl\n").unwrap();

    let bin = env!("CARGO_BIN_EXE_dbsrt");
    let out_drift = Command::new(bin)
        .args([
            "doctor",
            "--json",
            "--local-root",
            local.path().to_str().unwrap(),
            "--rpi4-local-root",
            rpi4.path().to_str().unwrap(),
            "--o2switch-local-root",
            o2.path().to_str().unwrap(),
        ])
        .output()
        .expect("run dbsrt doctor with drift");

    assert_eq!(out_drift.status.code(), Some(DOCTOR_EXIT_ALERT));
    let rep_drift: ClusterDoctorReport = serde_json::from_slice(&out_drift.stdout).unwrap();
    assert_eq!(rep_drift.status, DoctorSeverity::Critical);
    assert_eq!(rep_drift.sha256_drift_count, 2);
    let drifted_names: Vec<&str> = rep_drift
        .script_parity
        .iter()
        .filter(|p| p.status == ParityStatus::Sha256Drift)
        .map(|p| p.script.as_str())
        .collect();
    assert!(drifted_names.contains(&"o2switch_batch_worker.py"));
    assert!(drifted_names.contains(&"o2switch_ctl.sh"));
}

#[test]
fn test_doctor_subprocess_critical_alerts_redaction_and_human_table() {
    let local = TempDir::new().unwrap();
    seed_tracked_scripts(local.path(), true, true);

    let run_dir = local.path().join("run");
    let logs_dir = local.path().join("logs");
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::create_dir_all(&logs_dir).unwrap();

    std::fs::write(
        run_dir.join("batch_aborted"),
        "FTP abort ftp://user:secretpass@nas/video.mkv\n",
    )
    .unwrap();
    std::fs::write(
        logs_dir.join("batch_worker.log"),
        "❌ decode error pid 101 -ftp-password secretpass\n❌ decode error pid 202 -ftp-password secretpass\n",
    )
    .unwrap();
    std::fs::write(
        logs_dir.join("batch_events.jsonl"),
        "{\"video_stem\":\"ep01\",\"success\":false,\"status\":\"FAILED\"}\n\
         {\"video_stem\":\"ep01\",\"success\":false,\"status\":\"FAILED\"}\n\
         {\"video_stem\":\"ep01\",\"success\":false,\"status\":\"FAILED\"}\n",
    )
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_dbsrt");
    let out = Command::new(bin)
        .args([
            "doctor",
            "--local-only",
            "--local-root",
            local.path().to_str().unwrap(),
        ])
        .output()
        .expect("run dbsrt doctor human mode");

    assert_eq!(out.status.code(), Some(DOCTOR_EXIT_ALERT));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("STATUS: CRIT"));
    assert!(stdout.contains("EXIT: 2"));
    assert!(stdout.contains("YES!"));
    assert!(!stdout.contains("secretpass"));
}

#[test]
fn test_parse_ssh_probe_sections_roundtrip() {
    let fake_hash = "a".repeat(64);
    let raw = format!(
        "\n___DBSRT_DOC_SEC:NOW___\n1793352226\n\
         ___DBSRT_DOC_SEC:DAEMON___\n1\n\
         ___DBSRT_DOC_SEC:SHA256___\n{fake_hash}  scripts/o2switch_batch_worker.py\n\
         ___DBSRT_DOC_SEC:STAT___\n1793350000|data/reports/nas_ftp_index.json\n\
         ___DBSRT_DOC_SEC:ABORTED___\n2026-10-30T09:00:00Z test abort\n"
    );
    let snap = parse_ssh_probe_sections(&raw, 0);
    assert_eq!(snap.now_unix_secs, 1_793_352_226);
    assert_eq!(snap.daemon_running, Some(true));
    assert_eq!(
        snap.script_hashes
            .get("o2switch_batch_worker.py")
            .map(String::as_str),
        Some(fake_hash.as_str())
    );
    assert_eq!(
        snap.artifact_mtimes.get("data/reports/nas_ftp_index.json"),
        Some(&1_793_350_000)
    );
    assert!(snap.batch_aborted_text.is_some());
}

#[test]
fn test_parse_ssh_probe_sections_truncation_and_edge_cases() {
    let valid_h1 = "B".repeat(64);
    let oversized_server_log = format!(
        "PARTIAL_TORN_LINE\n{}",
        "2026-10-30T08:50:00Z INFO ok\n".repeat(2500)
    );
    assert!(oversized_server_log.len() as u64 > DOCTOR_LOG_TAIL_BYTES);

    let raw = format!(
        "noise before sections\n\
         ___DBSRT_DOC_SEC:NOW___\nnot_a_number\n\
         ___DBSRT_DOC_SEC:DAEMON___\n0\n\
         ___DBSRT_DOC_SEC:SHA256___\n\
         short_hash scripts/o2switch_batch_worker.py\n\
         only_one_col\n\
         {valid_h1}  *scripts/batch_worker/cli.py\n\
         {valid_h1}  scripts/untracked_helper.py\n\
         ___DBSRT_DOC_SEC:STAT___\n\
         malformed_stat_line\n\
         bad_num|run/batch_queue.json\n\
         1793350100|run/supervisor_status.json\n\
         ___DBSRT_DOC_SEC:SUPERVISOR___\n  {{\"state\":\"healthy\"}}  \n\
         ___DBSRT_DOC_SEC:QUEUE___\n   \n\
         ___DBSRT_DOC_SEC:LOG_SERVER___\n{oversized_server_log}\n\
         ___DBSRT_DOC_SEC:LOG_WORKER___\n   \n\
         ___DBSRT_DOC_SEC:EVENTS_ALT___\n{{\"video_stem\":\"s1\",\"status\":\"OK\"}}\n\
         ___DBSRT_DOC_SEC:FAULTWATCH___\n{{\"faults\":1}}\n"
    );

    let snap = parse_ssh_probe_sections(&raw, 1_793_352_000);
    assert_eq!(snap.now_unix_secs, 1_793_352_000);
    assert_eq!(snap.daemon_running, Some(false));
    assert_eq!(
        snap.script_hashes
            .get("batch_worker/cli.py")
            .map(String::as_str),
        Some(valid_h1.to_ascii_lowercase().as_str())
    );
    assert_eq!(snap.script_hashes.len(), 1);
    assert_eq!(
        snap.artifact_mtimes.get("run/supervisor_status.json"),
        Some(&1_793_350_100)
    );
    assert!(snap.supervisor_json.is_some());
    assert!(snap.batch_queue_json.is_none());
    assert!(snap.worker_log_tail.is_none());
    let srv_tail = snap.server_log_tail.expect("truncated server log");
    assert!(!srv_tail.contains("PARTIAL_TORN_LINE"));
    assert_eq!(snap.batch_events_tails.len(), 1);
    assert!(snap.faultwatch_tail.is_some());
}

#[test]
fn test_probe_directory_large_tails_faultwatch_and_unreachable_ssh() {
    let dir = TempDir::new().unwrap();
    seed_tracked_scripts(dir.path(), false, true);

    let logs_dir = dir.path().join("logs");
    let fw_dir = logs_dir.join("faultwatch");
    let reports_dir = dir.path().join("data/reports");
    std::fs::create_dir_all(&fw_dir).unwrap();
    std::fs::create_dir_all(&reports_dir).unwrap();

    let big_log = format!(
        "TORN_HEADER_WITHOUT_NEWLINE_AT_START\n{}",
        "[2026-10-30 08:59:00] ERROR repeated disk glitch\n".repeat(2000)
    );
    assert!(big_log.len() as u64 > DOCTOR_LOG_TAIL_BYTES);
    std::fs::write(logs_dir.join("dbsrt-server.log"), big_log).unwrap();
    std::fs::write(
        fw_dir.join("2026-10-30.jsonl"),
        "{\"minute\":\"2026-10-30T08:55Z\",\"secs_over_100\":4}\n",
    )
    .unwrap();
    std::fs::write(
        reports_dir.join("batch_tvshow_nightly.jsonl"),
        "{\"video_stem\":\"ep99\",\"status\":\"OK\"}\n",
    )
    .unwrap();
    std::fs::write(reports_dir.join("nas_ftp_index.json"), "{}").unwrap();

    // Note:probe_directory filters faultwatch `.jsonl` by real filesystem mtime <= 48h, so pass current epoch
    let now_real = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(fw_dir.join("watch.log"), "plain text non-jsonl log\n").unwrap();
    let snap = probe_directory(dir.path(), now_real);
    assert_eq!(snap.script_hashes.len(), TRACKED_SCRIPTS.len());
    assert!(snap
        .artifact_mtimes
        .contains_key("data/reports/nas_ftp_index.json"));
    assert!(!snap
        .server_log_tail
        .as_deref()
        .unwrap_or("")
        .contains("TORN_HEADER_WITHOUT_NEWLINE_AT_START"));
    assert!(snap.faultwatch_tail.is_some());
    assert!(!snap
        .faultwatch_tail
        .as_deref()
        .unwrap_or("")
        .contains("plain text non-jsonl"));
    assert_eq!(snap.batch_events_tails.len(), 1);

    // Unreachable SSH target degrades gracefully to Warning without panicking
    let ssh_rep = probe_node(
        NodeRole::Rpi4,
        &NodeProbeTarget::SshHost {
            host: "ssh://127.0.0.1:1".to_string(),
            remote_base: String::new(),
        },
        1,
        1_793_352_000,
    );
    assert!(!ssh_rep.reachable);
    assert!(!ssh_rep.skipped);
    assert_eq!(ssh_rep.status, DoctorSeverity::Warning);
    assert!(ssh_rep.alerts.iter().any(|a| a.id == "node_unreachable"));
}

#[test]
fn test_analyzer_supervisor_queue_aborted_and_freshness_variants() {
    let now = 1_793_352_000_u64;

    // 1. Corrupted supervisor JSON + empty batch_aborted + corrupted queue + stale nas_ftp_index
    let mut mtimes = BTreeMap::new();
    mtimes.insert("data/reports/nas_ftp_index.json".to_string(), now - 180_000);
    let snap_corrupt = RawNodeSnapshot {
        now_unix_secs: now,
        supervisor_json: Some("{broken_json".to_string()),
        batch_aborted_text: Some("   ".to_string()),
        batch_queue_json: Some("{bad".to_string()),
        artifact_mtimes: mtimes,
        ..Default::default()
    };
    let rep1 = analyze_node_snapshot(NodeRole::Rpi4, "pi".to_string(), 12, snap_corrupt);
    assert_eq!(rep1.supervisor_state.as_deref(), Some("corrupted"));
    assert!(rep1.batch_aborted);
    assert_eq!(rep1.failed_queue_targets, 0);
    assert!(rep1.alerts.iter().any(|a| a.id == "supervisor_corrupted"));
    assert!(rep1.alerts.iter().any(|a| a.id == "stale_nas_ftp_index"));

    // 2. Degraded supervisor with failed_batch_types, stale heartbeat, and failed queue targets
    let sup_deg = format!(
        "{{\"state\":\"degraded\",\"launch_error_count\":2,\"unix_ts\":{},\"failed_batch_types\":{{\"freebox\":\"timeout\"}}}}",
        now - 600
    );
    let snap_deg = RawNodeSnapshot {
        now_unix_secs: now,
        supervisor_json: Some(sup_deg),
        batch_queue_json: Some("{\"failed_targets\":3,\"total_targets\":20}".to_string()),
        ..Default::default()
    };
    let rep2 = analyze_node_snapshot(NodeRole::Rpi4, "pi".to_string(), 8, snap_deg);
    assert_eq!(rep2.failed_queue_targets, 3);
    assert!(rep2.alerts.iter().any(|a| a.id == "supervisor_degraded"));
    assert!(rep2.alerts.iter().any(|a| a.id == "supervisor_stale"));
    assert!(rep2.alerts.iter().any(|a| a.id == "batch_queue_failed"));

    // 3. Supervisor with launch_error_count > 0 and empty failed_batch_types
    let snap_launch = RawNodeSnapshot {
        now_unix_secs: now,
        supervisor_json: Some("{\"state\":\"healthy\",\"launch_error_count\":1}".to_string()),
        ..Default::default()
    };
    let rep3 = analyze_node_snapshot(NodeRole::Rpi4, "pi".to_string(), 5, snap_launch);
    assert!(rep3.alerts.iter().any(|a| a.id == "supervisor_degraded"));
}

#[test]
fn test_analyzer_logs_windows_faultwatch_streaks_and_timestamps() {
    // 2026-10-30T12:00:00Z -> compute exact epoch via parse_leading_timestamp
    let (Some(now), rest) = parse_leading_timestamp("2026-10-30T12:00:00Z boot") else {
        panic!("expected valid timestamp");
    };
    assert_eq!(rest, "boot");
    assert_eq!(format_unix_iso(now), "2026-10-30T12:00:00Z");

    // Timestamp edge cases: bracketed, fractional+Z, tz offset, invalid dates
    assert!(parse_leading_timestamp("[2026-10-30 11:30:00] msg")
        .0
        .is_some());
    assert!(parse_leading_timestamp("2026-02-15T11:30:00.987Z msg")
        .0
        .is_some());
    assert!(parse_leading_timestamp("2026-10-30T11:30:00+01:00 msg")
        .0
        .is_some());
    assert!(parse_leading_timestamp("short").0.is_none());
    assert!(parse_leading_timestamp("2026/10/30 11:30:00 bad sep")
        .0
        .is_none());
    assert!(parse_leading_timestamp("2026-99-30 11:30:00 bad month")
        .0
        .is_none());
    assert!(parse_leading_timestamp("2026-10-30 99:00:00 bad hour")
        .0
        .is_none());

    let ts_30m = format_unix_iso(now - 1_800);
    let ts_10h = format_unix_iso(now - 36_000);
    let ts_48h = format_unix_iso(now - 172_800);
    let ts_96h = format_unix_iso(now - 345_600);
    let long_tail_msg = "x".repeat(220);

    let server_log = format!(
        "{ts_30m} ERROR Received signal SIGTERM, Stopping gracefully\n\
         {ts_96h} ERROR ancient expired error\n\
         {ts_48h} 🛑 fatal panic in worker 99\n\
         {ts_10h} ERROR timeout on request 42 {long_tail_msg}\n\
         continuation ERROR without own timestamp\n"
    );
    let faultwatch = "\n\
         not_json_faultwatch_line\n\
         {\"minute\":\"2026-10-25T08:00Z\",\"faults\":5}\n\
         {\"minute\":\"2026-10-30T11:55Z\",\"faults\":0,\"secs_over_100\":0,\"run_delay_ms_max\":10.0}\n\
         {\"minute\":\"2026-10-30T11:56Z\",\"run_delay_ms_max\":650.0}\n\
         {\"minute\":\"2026-10-30T11:57Z\",\"faults\":2}\n"
        .to_string();
    let events = "\n\
        not_json\n\
        {\"no_stem\":1}\n\
        {\"video_stem\":\"\",\"status\":\"FAILED\"}\n\
        {\"video_stem\":\"flaky\",\"status\":\"FAILED\"}\n\
        {\"video_stem\":\"flaky\",\"status\":\"OK\"}\n\
        {\"video_stem\":\"dead\",\"status\":\"FAILED\"}\n\
        {\"video_stem\":\"dead\",\"status\":\"ERROR\"}\n\
        {\"video_stem\":\"dead\",\"success\":false}\n"
        .to_string();

    let snap = RawNodeSnapshot {
        now_unix_secs: now,
        server_log_tail: Some(server_log),
        faultwatch_tail: Some(faultwatch),
        batch_events_tails: vec![events],
        ..Default::default()
    };
    let rep = analyze_node_snapshot(NodeRole::O2switch, "o2".to_string(), 15, snap);
    assert_eq!(rep.status, DoctorSeverity::Critical);
    assert_eq!(rep.blacklisted_stems, 1);
    assert_eq!(rep.lve_faults_24h, 2);
    let srv_comp = rep
        .components
        .iter()
        .find(|c| c.component == "dbsrt-server")
        .unwrap();
    assert_eq!(srv_comp.counts.h1, 0);
    assert_eq!(srv_comp.counts.h24, 2);
    assert_eq!(srv_comp.counts.h72, 3);
    assert!(srv_comp.top_signatures.iter().any(|s| s.critical));
    assert!(srv_comp
        .top_signatures
        .iter()
        .any(|s| s.summary.ends_with('…')));
}

#[test]
fn test_doctor_multisession_worker_log_and_standalone_o2switch_local_only() {
    let o2_dir = TempDir::new().unwrap();
    // Seed only worker scripts (include_rpi_only = false), mimicking ~/dbsrt on o2switch
    seed_tracked_scripts(o2_dir.path(), false, false);
    let logs_dir = o2_dir.path().join("logs");
    std::fs::create_dir_all(&logs_dir).unwrap();
    std::fs::write(
        logs_dir.join("batch_worker.log"),
        "=== [1/4] Scanning local catalogs ===\n\
         ❌ historical undated failure\n\
         === [1/4] Scanning local catalogs ===\n\
         --> [DEBUG] dbsrt ERROR (returncode 1): debug trace\n\
         ✅ nominal progress\n",
    )
    .unwrap();

    let rep = evaluate_cluster_doctor(&DoctorArgs {
        json: true,
        local_only: true,
        local_root: Some(o2_dir.path().to_path_buf()),
        ..DoctorArgs::default()
    });
    assert_eq!(rep.exit_code, DOCTOR_EXIT_OK);
    assert_eq!(rep.sha256_drift_count, 0);
    assert_eq!(rep.status, DoctorSeverity::Ok);
}
