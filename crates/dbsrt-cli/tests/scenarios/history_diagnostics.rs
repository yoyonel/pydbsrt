//! User scenario tests: Completed records, status severities, diagnostics & error inspection.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::{CompletedFilter, CompletedSort, CompletedStatus, ViewPage};

use super::common::{mock_complete_telemetry, TestHarness};

#[test]
fn test_scenario_completed_status_severities_and_filtering() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // Switch to CompletedMedia ('3')
    harness.send_key(KeyCode::Char('3'));
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);

    // Verify 5 statuses are mapped correctly
    {
        let tel = harness.shared_state.read().unwrap();
        assert_eq!(tel.history_records[0].status(), CompletedStatus::Ok);
        assert_eq!(tel.history_records[1].status(), CompletedStatus::Ok);
        assert_eq!(tel.history_records[2].status(), CompletedStatus::NoSrt);
        assert_eq!(tel.history_records[3].status(), CompletedStatus::Cancelled);
        assert_eq!(tel.history_records[4].status(), CompletedStatus::Fail);
    }

    // Initial view shows all 5 records
    harness.assert_buffer_contains("Carnivàle");
    harness.assert_buffer_contains("The.Expanse");
    harness.assert_buffer_contains("The Leftovers S01E01");
    harness.assert_buffer_contains("The Leftovers S01E05");

    // 1. Filter: SuccessOnly
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.completed_filter, CompletedFilter::SuccessOnly);
    harness.assert_buffer_contains("Carnivàle");
    harness.assert_buffer_does_not_contain("The Leftovers S01E01");
    harness.assert_buffer_does_not_contain("The Leftovers S01E05");

    // 2. Filter: FailedOnly
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.completed_filter, CompletedFilter::FailedOnly);
    harness.assert_buffer_contains("The.Expanse.S04E08.HDR.2160p");
    harness.assert_buffer_does_not_contain("Carnivàle");

    // 3. Filter: NoSrtOnly
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.completed_filter, CompletedFilter::NoSrtOnly);
    harness.assert_buffer_contains("The Leftovers S01E01");
    harness.assert_buffer_does_not_contain("The.Expanse.S04E08.HDR.2160p");

    // 4. Filter: CancelledOnly
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.completed_filter, CompletedFilter::CancelledOnly);
    harness.assert_buffer_contains("The Leftovers S01E05");
    harness.assert_buffer_does_not_contain("The Leftovers S01E01");

    // 5. Back to All
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.completed_filter, CompletedFilter::All);
}

#[test]
fn test_scenario_completed_sorting_cycle() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('3'));

    assert_eq!(harness.app.completed_sort, CompletedSort::DateDesc);

    // Cycle sort with 'o'
    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::Status);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::Name);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::Duration);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::Size);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::Rate);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::Couplings);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::DateAsc);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.completed_sort, CompletedSort::DateDesc);
}

#[test]
fn test_scenario_completed_inspection_sheet_diagnostics() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('3'));

    // Select the NoSrt failure record (under DateDesc, index 2)
    harness.app.completed_scroll_index = 2;
    harness.send_key(KeyCode::Enter);
    assert_eq!(harness.app.view, ViewPage::CompletedInspection);

    harness.assert_buffer_contains("FICHE DÉTAILLÉE DU TRAITEMENT");
    harness.assert_buffer_contains("The Leftovers S01E01");
    harness.assert_buffer_contains("SOUS-TITRES MANQUANTS");
    harness.assert_buffer_contains("Aucun fichier .srt");

    // Exit with Esc
    harness.send_key(KeyCode::Esc);
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);

    // Select the Cancelled record (index 1 under DateDesc) and open with 'i'
    harness.app.completed_scroll_index = 1;
    harness.send_key(KeyCode::Char('i'));
    assert_eq!(harness.app.view, ViewPage::CompletedInspection);

    harness.assert_buffer_contains("INTERROMPU PAR L'UTILISATEUR");
    harness.assert_buffer_contains("Interruption manuelle ou système");

    // Exit with Backspace
    harness.send_key(KeyCode::Backspace);
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);
}

#[test]
fn test_scenario_completed_empty_state() {
    let mut tel = mock_complete_telemetry();
    tel.history_records.clear();
    let mut harness = TestHarness::new(240, 45, tel);

    harness.send_key(KeyCode::Char('3'));
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);

    // Buffer indicates 0 records
    harness.assert_buffer_contains("0 RÉFÉRENCÉS");

    // Keys do not panic
    harness.send_key(KeyCode::Down);
    harness.send_key(KeyCode::Up);
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);
}
