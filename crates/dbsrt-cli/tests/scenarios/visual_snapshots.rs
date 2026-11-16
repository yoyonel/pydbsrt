//! Golden snapshot tests for dbsrt top terminal grids using insta (ISO Ratatui strategy).

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::{
    ClusterAlertEntry, ClusterArtifactFreshness, ClusterComponentErrorSummary,
    ClusterErrorSignature, ClusterHealthTelemetry, ClusterNodeSnapshot, ClusterParityStatus,
    ClusterScriptParity, ClusterSeverity, ViewPage,
};

use super::common::{mock_complete_telemetry, TestHarness};

const SNAPSHOT_WIDTH: u16 = 140;
const SNAPSHOT_HEIGHT: u16 = 30;

#[test]
fn test_snapshot_active_jobs_view() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::ActiveJobs;
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_active_jobs", grid);
}

#[test]
fn test_snapshot_stats_history_view() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::StatsHistory;
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_stats_history", grid);
}

#[test]
fn test_snapshot_completed_media_view() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::CompletedMedia;
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_completed_media", grid);
}

#[test]
fn test_snapshot_media_browser_view() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::MediaBrowser;
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_media_browser", grid);
}

#[test]
fn test_snapshot_media_sources_view() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::MediaSources;
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_media_sources", grid);
}

#[test]
fn test_snapshot_source_modal_add() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::MediaSources;
    harness.send_key(KeyCode::Char('a'));
    harness.type_text("/mnt/nas/demo");
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_source_modal_add", grid);
}

#[test]
fn test_snapshot_completed_inspection() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::CompletedMedia;
    harness.app.completed_scroll_index = 2; // The Leftovers S01E01 (NoSrt failure)
    harness.send_key(KeyCode::Enter);
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_completed_inspection", grid);
}

#[test]
fn test_snapshot_job_inspection() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::ActiveJobs;
    harness.send_key(KeyCode::Enter);
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_job_inspection", grid);
}

#[test]
fn test_snapshot_help_view() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::MediaBrowser;
    harness.send_key(KeyCode::Char('?'));
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_help_media_browser", grid);
}

#[test]
fn test_snapshot_system_view() {
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, mock_complete_telemetry());
    harness.app.view = ViewPage::System;
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_system", grid);
}

#[test]
fn test_snapshot_cluster_view() {
    let mut tel = mock_complete_telemetry();
    tel.cluster = Some(ClusterHealthTelemetry {
        status: ClusterSeverity::Warning,
        exit_code: 0,
        checked_at_iso: "2026-11-15T21:30:00Z".to_string(),
        elapsed_ms: 485,
        active_critical_count: 0,
        active_warning_count: 2,
        sha256_drift_count: 0,
        local_only_probe: false,
        refreshing: false,
        nodes: vec![
            ClusterNodeSnapshot {
                node: "local".to_string(),
                target: ".".to_string(),
                reachable: true,
                skipped: false,
                probe_ms: 1,
                status: ClusterSeverity::Ok,
                probe_error: None,
                daemon_running: Some(false),
                supervisor_state: None,
                batch_aborted: false,
                failed_queue_targets: 0,
                blacklisted_stems: 0,
                lve_faults_24h: 0,
                err_h1: 0,
                err_h24: 0,
                err_h72: 0,
                err_tail_total: 0,
                components: vec![],
                freshness: vec![ClusterArtifactFreshness {
                    name: "nas_ftp_index.json".to_string(),
                    exists: true,
                    age_secs: Some(3600),
                    ttl_secs: 86400,
                    stale: false,
                }],
            },
            ClusterNodeSnapshot {
                node: "o2switch".to_string(),
                target: "o2switch".to_string(),
                reachable: true,
                skipped: false,
                probe_ms: 480,
                status: ClusterSeverity::Warning,
                probe_error: None,
                daemon_running: Some(true),
                supervisor_state: Some("healthy".to_string()),
                batch_aborted: false,
                failed_queue_targets: 1,
                blacklisted_stems: 0,
                lve_faults_24h: 0,
                err_h1: 0,
                err_h24: 1,
                err_h72: 1,
                err_tail_total: 1,
                components: vec![ClusterComponentErrorSummary {
                    component: "batch_worker".to_string(),
                    h1: 0,
                    h24: 1,
                    h72: 1,
                    tail_total: 1,
                    top_signatures: vec![ClusterErrorSignature {
                        summary: "[WATCHDOG] Killed process: exceeded max duration".to_string(),
                        count: 1,
                        last_seen: Some("2026-11-15T18:10:00Z".to_string()),
                        critical: false,
                    }],
                }],
                freshness: vec![ClusterArtifactFreshness {
                    name: "supervisor_status.json".to_string(),
                    exists: true,
                    age_secs: Some(28),
                    ttl_secs: 300,
                    stale: false,
                }],
            },
        ],
        script_parity: vec![ClusterScriptParity {
            script: "o2switch_batch_worker.py".to_string(),
            local_sha256: Some("caa755f460a6abcdef0123456789".to_string()),
            rpi4_sha256: Some("caa755f460a6abcdef0123456789".to_string()),
            o2switch_sha256: Some("caa755f460a6abcdef0123456789".to_string()),
            status: ClusterParityStatus::Ok,
            detail: None,
        }],
        alerts: vec![ClusterAlertEntry {
            node: "o2switch".to_string(),
            severity: ClusterSeverity::Warning,
            component: "batch_worker".to_string(),
            summary: "Batch queue has 1/466 failed target(s)".to_string(),
        }],
        data_topology: None,
    });
    let mut harness = TestHarness::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT, tel);
    harness.app.view = ViewPage::Cluster;
    let grid = harness.render_to_text();
    insta::assert_snapshot!("view_cluster", grid);
}
