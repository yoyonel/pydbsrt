//! User scenario tests: Navigation, circular tab cycling & terminal bounds resizing.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::ViewPage;

use super::common::{mock_complete_telemetry, TestHarness};

#[test]
fn test_scenario_circular_tab_cycling_forward_and_backward() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Forward cycling via Tab: 1 -> 2 -> 3 -> 4 -> 5 -> 1
    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.view, ViewPage::StatsHistory);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.view, ViewPage::MediaSources);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Backward cycling via BackTab: 1 -> 5 -> 4 -> 3 -> 2 -> 1
    harness.send_key(KeyCode::BackTab);
    assert_eq!(harness.app.view, ViewPage::MediaSources);

    harness.send_key(KeyCode::BackTab);
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    harness.send_key(KeyCode::BackTab);
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);

    harness.send_key(KeyCode::BackTab);
    assert_eq!(harness.app.view, ViewPage::StatsHistory);

    harness.send_key(KeyCode::BackTab);
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Arrow keys Left / Right as aliases
    harness.send_key(KeyCode::Right);
    assert_eq!(harness.app.view, ViewPage::StatsHistory);

    harness.send_key(KeyCode::Left);
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);
}

#[test]
fn test_scenario_direct_jump_shortcuts_and_double_tap_toggle() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // Jump to Stats with '2'
    harness.send_key(KeyCode::Char('2'));
    assert_eq!(harness.app.view, ViewPage::StatsHistory);

    // Double-tap '2' returns to ActiveJobs
    harness.send_key(KeyCode::Char('2'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Jump to Completed with 'r'
    harness.send_key(KeyCode::Char('r'));
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);

    // Double-tap 'r' returns to ActiveJobs
    harness.send_key(KeyCode::Char('r'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Jump to Media with 'm'
    harness.send_key(KeyCode::Char('m'));
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    // Double-tap 'm' returns to ActiveJobs
    harness.send_key(KeyCode::Char('m'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // Jump to Sources with '5'
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(harness.app.view, ViewPage::MediaSources);

    // Double-tap '5' returns to ActiveJobs
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);
}

#[test]
fn test_scenario_quit_and_escape_behavior() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    assert!(!harness.app.should_quit);

    // Esc on ActiveJobs quits
    harness.send_key(KeyCode::Esc);
    assert!(harness.app.should_quit);

    // 'q' quits from any view
    harness.app.should_quit = false;
    harness.app.view = ViewPage::MediaBrowser;
    harness.send_key(KeyCode::Char('q'));
    assert!(harness.app.should_quit);
}

#[test]
fn test_scenario_terminal_dimension_bounds_and_no_panic_resizing() {
    let tel = mock_complete_telemetry();

    // Test a variety of terminal dimensions from compact to ultra-wide
    let dimensions = [
        (80, 24),  // Standard legacy VT100
        (100, 30), // Compact terminal
        (140, 35), // Medium laptop window
        (200, 40), // Large display
        (240, 45), // Full HD 1080p terminal
    ];

    let views = [
        ViewPage::ActiveJobs,
        ViewPage::StatsHistory,
        ViewPage::CompletedMedia,
        ViewPage::MediaBrowser,
        ViewPage::MediaSources,
        ViewPage::JobInspection,
        ViewPage::MediaInspection,
        ViewPage::CompletedInspection,
        ViewPage::Help,
    ];

    for (w, h) in dimensions {
        for &view in &views {
            let mut harness = TestHarness::new(w, h, tel.clone());
            harness.app.view = view;
            // Drawing must never panic regardless of terminal bounds
            let text = harness.render_to_text();
            assert!(
                !text.is_empty(),
                "Rendered buffer for {w}x{h} in {view:?} was empty!"
            );
        }
    }
}
