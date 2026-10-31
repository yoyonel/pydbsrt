//! User scenario tests: Active jobs monitoring, process control & inspection.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::ViewPage;

use super::common::{mock_complete_telemetry, TestHarness};

#[test]
fn test_scenario_active_jobs_table_metrics_and_navigation() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // View is ActiveJobs by default or press '1'
    harness.send_key(KeyCode::Char('1'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Verify 2 workers active
    harness.assert_buffer_contains("Workers: 2 actifs");
    harness.assert_buffer_contains("The.Wire.S01E01");
    harness.assert_buffer_contains("GPU (VA-API)");
    harness.assert_buffer_contains("The.Wire.S01E02");
    harness.assert_buffer_contains("CPU Multi");

    // Test selection navigation
    assert_eq!(harness.app.selected_index, 0);
    harness.send_key(KeyCode::Char('j'));
    assert_eq!(harness.app.selected_index, 1);

    // Cannot scroll past last job
    harness.send_key(KeyCode::Char('j'));
    assert_eq!(harness.app.selected_index, 1);

    // Scroll back up
    harness.send_key(KeyCode::Char('k'));
    assert_eq!(harness.app.selected_index, 0);

    // Home / End
    harness.send_key(KeyCode::End);
    assert_eq!(harness.app.selected_index, 1);
    harness.send_key(KeyCode::Home);
    assert_eq!(harness.app.selected_index, 0);
}

#[test]
fn test_scenario_process_signals_pause_stop_kill() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('1'));

    // 1. Toggle pause with 'p' on PID 50101
    harness.send_key(KeyCode::Char('p'));
    assert!(harness.feedback_text().is_some());

    // 2. Stop single task with 'c'
    harness.send_key(KeyCode::Char('c'));
    assert!(harness.feedback_text().is_some());

    // 3. Open fullframe inspection with 'Enter'
    harness.send_key(KeyCode::Enter);
    assert_eq!(harness.app.view, ViewPage::JobInspection);

    harness.assert_buffer_contains("INSPECTION DÉTAILLÉE DU PROCESSUS");
    harness.assert_buffer_contains("PID");
    harness.assert_buffer_contains("50101");
    harness.assert_buffer_contains("--- Ligne de Commande Exécutée ---");
    harness.assert_buffer_contains("/dev/dri/renderD128");

    // 4. Force kill job with 'K' from inspection sheet
    harness.send_key(KeyCode::Char('K'));
    assert!(harness.feedback_text().is_some());

    // 5. Exit inspection with Esc
    harness.send_key(KeyCode::Esc);
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);
}

#[test]
fn test_scenario_batch_stop_graceful_and_emergency() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('1'));

    // Graceful stop 'x'
    harness.send_key(KeyCode::Char('x'));
    let msg_graceful = harness.feedback_text().unwrap();
    assert!(
        msg_graceful.contains("Arrêt gracieux")
            || msg_graceful.contains("SIGTERM")
            || msg_graceful.contains("stoppé")
            || msg_graceful.contains("Aucun batch")
    );

    // Emergency kill 'X'
    harness.send_key(KeyCode::Char('X'));
    let msg_force = harness.feedback_text().unwrap();
    assert!(
        msg_force.contains("Arrêt forcé")
            || msg_force.contains("SIGKILL")
            || msg_force.contains("tué")
            || msg_force.contains("Aucun batch")
    );
}

#[test]
fn test_scenario_active_jobs_empty_state() {
    let mut tel = mock_complete_telemetry();
    tel.active_jobs.clear();
    let mut harness = TestHarness::new(240, 45, tel);

    harness.send_key(KeyCode::Char('1'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Header shows 0 active workers
    harness.assert_buffer_contains("Workers: 0 actifs");

    // Keys do not panic
    harness.send_key(KeyCode::Down);
    harness.send_key(KeyCode::Up);
    harness.send_key(KeyCode::Enter);
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);
}
