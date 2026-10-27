//! User scenario tests: Media Browser discovery, filter, sort, selection, batch & inspection.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::{IngestionStatus, MediaFilter, MediaSort, ViewPage};

use super::common::{mock_complete_telemetry, TestHarness};

#[test]
fn test_scenario_media_browser_multi_source_badges() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // Switch to MediaBrowser ('4')
    harness.send_key(KeyCode::Char('4'));
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    // Assert that source badges [video] and [tvshow] are visibly rendered
    harness.assert_buffer_contains("[video]");
    harness.assert_buffer_contains("[tvshow]");
    harness.assert_buffer_contains("5/5 DISPONIBLES");
}

#[test]
fn test_scenario_media_browser_filter_cycling() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('4'));

    // Initial: All (5 items)
    assert_eq!(harness.app.media_filter, MediaFilter::All);
    harness.assert_buffer_contains("5/5 DISPONIBLES");

    // 1. Virgin filter
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(
        harness.app.media_filter,
        MediaFilter::Status(IngestionStatus::Virgin)
    );
    harness.assert_buffer_contains("[VIERGES SEULS]");
    harness.assert_buffer_contains("Movie.A.2024");
    harness.assert_buffer_contains("Movie.C.2022");
    harness.assert_buffer_contains("Show.S01E02");
    harness.assert_buffer_does_not_contain("Movie.B.2023"); // Completed
    harness.assert_buffer_does_not_contain("Show.S01E01"); // PartialFail

    // 2. InProgress filter
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(
        harness.app.media_filter,
        MediaFilter::Status(IngestionStatus::InProgress)
    );
    harness.assert_buffer_contains("[EN COURS]");

    // 3. Completed filter
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(
        harness.app.media_filter,
        MediaFilter::Status(IngestionStatus::Completed)
    );
    harness.assert_buffer_contains("[COMPLÉTÉS]");
    harness.assert_buffer_contains("Movie.B.2023");
    harness.assert_buffer_does_not_contain("Movie.A.2024");

    // 4. PartialFail filter
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(
        harness.app.media_filter,
        MediaFilter::Status(IngestionStatus::PartialFail)
    );
    harness.assert_buffer_contains("[HACHÉS SEULS]");

    // 5. WithSubtitles filter
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.media_filter, MediaFilter::WithSubtitles);
    harness.assert_buffer_contains("[AVEC SOUS-TITRES]");

    // 6. WithoutSubtitles filter
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.media_filter, MediaFilter::WithoutSubtitles);
    harness.assert_buffer_contains("[SANS SOUS-TITRE]");

    // 7. Cycle back to All
    harness.send_key(KeyCode::Char('f'));
    assert_eq!(harness.app.media_filter, MediaFilter::All);
    harness.assert_buffer_contains("5/5 DISPONIBLES");
}

#[test]
fn test_scenario_media_browser_source_filter_cycling() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('4'));

    // Toggle source filter with 'S'
    // 1st press -> Filter by first active source: nas_video
    harness.send_key(KeyCode::Char('S'));
    assert_eq!(
        harness.app.selected_source_filter.as_deref(),
        Some("nas_video")
    );
    harness.assert_buffer_contains("3/5 DISPONIBLES");
    harness.assert_buffer_contains("Movie.A.2024");
    harness.assert_buffer_does_not_contain("Show.S01E01");

    // 2nd press -> Filter by second active source: nas_tvshow
    harness.send_key(KeyCode::Char('S'));
    assert_eq!(
        harness.app.selected_source_filter.as_deref(),
        Some("nas_tvshow")
    );
    harness.assert_buffer_contains("2/5 DISPONIBLES");
    harness.assert_buffer_contains("Show.S01E01");
    harness.assert_buffer_does_not_contain("Movie.A.2024");

    // 3rd press -> All sources (None)
    harness.send_key(KeyCode::Char('S'));
    assert_eq!(harness.app.selected_source_filter, None);
    harness.assert_buffer_contains("5/5 DISPONIBLES");
}

#[test]
fn test_scenario_media_browser_sorting() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('4'));

    assert_eq!(harness.app.media_sort, MediaSort::Name);

    // Cycle sort with 'o'
    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.media_sort, MediaSort::Size);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.media_sort, MediaSort::Duration);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.media_sort, MediaSort::Status);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.media_sort, MediaSort::Subtitles);

    harness.send_key(KeyCode::Char('o'));
    assert_eq!(harness.app.media_sort, MediaSort::Name);
}

#[test]
fn test_scenario_media_browser_selection_select_all_and_batch_launch() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('4'));

    // Toggle all items with 'a'
    harness.send_key(KeyCode::Char('a'));
    assert_eq!(harness.app.selected_media_paths.len(), 5);
    harness.assert_buffer_contains("5 média(s) coché(s)");

    // Toggle all again with 'a' -> clears selection
    harness.send_key(KeyCode::Char('a'));
    assert_eq!(harness.app.selected_media_paths.len(), 0);
    harness.assert_buffer_contains("0 média(s) coché(s)");

    // Select first 2 items manually with Space and j
    harness.app.media_scroll_index = 0;
    harness.send_key(KeyCode::Char(' '));
    harness.send_key(KeyCode::Char('j'));
    harness.send_key(KeyCode::Char(' '));
    assert_eq!(harness.app.selected_media_paths.len(), 2);

    // Launch batch with 'b'
    harness.send_key(KeyCode::Char('b'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);
    assert_eq!(harness.app.selected_media_paths.len(), 0);
    let fb = harness.feedback_text().unwrap();
    assert!(fb.contains("Batch"));
}

#[test]
fn test_scenario_media_browser_inspection_sheet() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('4'));

    // Move to second item (Movie.B.2023 with subtitle track & phash)
    harness.app.media_scroll_index = 1;

    // Open detailed sheet with 'Enter'
    harness.send_key(KeyCode::Enter);
    assert_eq!(harness.app.view, ViewPage::MediaInspection);

    harness.assert_buffer_contains("FICHE DÉTAILLÉE & DIAGNOSTIC D'INGESTION DU MÉDIA");
    harness.assert_buffer_contains("Movie.B.2023");
    harness.assert_buffer_contains("COMPLÉTÉ");
    harness.assert_buffer_contains("h264");
    harness.assert_buffer_contains("VF"); // French subtitle track title

    // Check item from inside inspection sheet with Space
    harness.send_key(KeyCode::Char(' '));
    assert_eq!(harness.app.selected_media_paths.len(), 1);

    // Exit inspection sheet with Backspace
    harness.send_key(KeyCode::Backspace);
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);
}

#[test]
fn test_scenario_media_browser_empty_state_handling() {
    let mut tel = mock_complete_telemetry();
    tel.discovered_media.clear();
    let mut harness = TestHarness::new(240, 45, tel);

    harness.send_key(KeyCode::Char('4'));
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    // Buffer indicates 0 items without panic
    harness.assert_buffer_contains("0/0 DISPONIBLES");

    // Navigation keys on empty list must be no-ops and not panic
    harness.send_key(KeyCode::Down);
    harness.send_key(KeyCode::Up);
    harness.send_key(KeyCode::PageDown);
    harness.send_key(KeyCode::PageUp);
    harness.send_key(KeyCode::Home);
    harness.send_key(KeyCode::End);
    assert_eq!(harness.app.media_scroll_index, 0);
}
