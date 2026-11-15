use clap::Parser;
use dbsrt_cli::cli::{Cli, Commands};
use dbsrt_cli::commands::doctor::constants::TRACKED_SCRIPTS;
use dbsrt_cli::commands::doctor::probe::parse_ssh_probe_sections;
use dbsrt_cli::commands::{
    evaluate_cluster_doctor, ClusterDoctorReport, DoctorArgs, DoctorSeverity, ParityStatus,
    DOCTOR_EXIT_ALERT, DOCTOR_EXIT_OK,
};
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
