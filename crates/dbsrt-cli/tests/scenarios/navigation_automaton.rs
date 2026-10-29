//! Automated State Machine & Combinatorial Exploration Test Suite for TUI Navigation.
//!
//! Exhaustively explores all permutations, direct shortcuts, double-tap toggles,
//! cyclic tab advances/retreats, and sub-view escape sequences across all views.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::{HelpTab, ViewPage};

use super::common::{mock_complete_telemetry, TestHarness};

const PRIMARY_VIEWS: [ViewPage; 6] = [
    ViewPage::ActiveJobs,
    ViewPage::StatsHistory,
    ViewPage::CompletedMedia,
    ViewPage::MediaBrowser,
    ViewPage::MediaSources,
    ViewPage::System,
];

/// Verifies that pressing 's' from [1] Tâches (ActiveJobs) directly transitions to [5/s] Sources,
/// and pressing 's' again toggles back to [1] Tâches.
#[test]
fn test_regression_active_jobs_s_key_navigates_to_sources_and_toggles_back() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // Initial state is ActiveJobs [1]
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // 1. Press 's' on ActiveJobs -> MUST transition to MediaSources [5/s]
    harness.send_key(KeyCode::Char('s'));
    assert_eq!(
        harness.app.view,
        ViewPage::MediaSources,
        "Pressing 's' from ActiveJobs must navigate to MediaSources"
    );

    // 2. Press 's' on MediaSources -> MUST toggle back to ActiveJobs [1]
    harness.send_key(KeyCode::Char('s'));
    assert_eq!(
        harness.app.view,
        ViewPage::ActiveJobs,
        "Pressing 's' from MediaSources must toggle back to ActiveJobs"
    );

    // 3. Press '5' on ActiveJobs -> MUST transition to MediaSources [5/s]
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(
        harness.app.view,
        ViewPage::MediaSources,
        "Pressing '5' from ActiveJobs must navigate to MediaSources"
    );

    // 4. Press '5' on MediaSources -> MUST toggle back to ActiveJobs [1]
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(
        harness.app.view,
        ViewPage::ActiveJobs,
        "Pressing '5' from MediaSources must toggle back to ActiveJobs"
    );

    // 5. Verify 'c' triggers stop task feedback without changing view
    harness.send_key(KeyCode::Char('c'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);
    assert!(harness.feedback_text().is_some());
}

/// Exhaustive 5x5 permutation matrix test: Verifies that from every primary view,
/// pressing numeric keys '1'..'5' lands on the exact expected view.
#[test]
fn test_automaton_numeric_direct_jumps_5x5_matrix() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    for (from_idx, &from_view) in PRIMARY_VIEWS.iter().enumerate() {
        for (to_idx, &to_view) in PRIMARY_VIEWS.iter().enumerate() {
            // Set initial state
            harness.app.view = from_view;
            assert_eq!(harness.app.view, from_view);

            let key_char = char::from_digit((to_idx + 1) as u32, 10).unwrap();
            harness.send_key(KeyCode::Char(key_char));

            if from_idx == to_idx && to_idx > 0 {
                // Double-tapping current view toggles back to ActiveJobs
                assert_eq!(
                    harness.app.view,
                    ViewPage::ActiveJobs,
                    "Double tapping '{}' from {:?} must toggle to ActiveJobs",
                    key_char,
                    from_view
                );
            } else {
                assert_eq!(
                    harness.app.view, to_view,
                    "Jumping with '{}' from {:?} must land on {:?}",
                    key_char, from_view, to_view
                );
            }
        }
    }
}

/// Exhaustive mnemonic jump test: Verifies that mnemonic letters 'h', 'r', 'm', 's'
/// correctly transition from any starting view to their target views.
#[test]
fn test_automaton_mnemonic_letter_jumps_and_toggles() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    let mnemonic_targets = [
        ('h', ViewPage::StatsHistory),
        ('r', ViewPage::CompletedMedia),
        ('m', ViewPage::MediaBrowser),
        ('s', ViewPage::MediaSources),
        ('y', ViewPage::System),
    ];

    for &from_view in &PRIMARY_VIEWS {
        for &(letter, target_view) in &mnemonic_targets {
            // Documented exception: 'r' on MediaSources triggers a rescan rather than view switch
            if from_view == ViewPage::MediaSources && letter == 'r' {
                continue;
            }

            harness.app.view = from_view;
            harness.send_key(KeyCode::Char(letter));

            if from_view == target_view {
                // Toggling current view returns to ActiveJobs
                assert_eq!(
                    harness.app.view,
                    ViewPage::ActiveJobs,
                    "Pressing '{}' when already on {:?} must toggle back to ActiveJobs",
                    letter,
                    from_view
                );
            } else {
                assert_eq!(
                    harness.app.view, target_view,
                    "Pressing '{}' from {:?} must transition to {:?}",
                    letter, from_view, target_view
                );
            }
        }
    }
}

/// Forward and backward cyclic exploration: Verifies that every pair (from_view, to_view)
/// can be reached via Tab/Right and BackTab/Left with exact modulo 5 distance steps.
#[test]
fn test_automaton_cyclic_tab_distance_all_pairs() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    for (from_idx, &from_view) in PRIMARY_VIEWS.iter().enumerate() {
        for (to_idx, &to_view) in PRIMARY_VIEWS.iter().enumerate() {
            // Forward cycling with Tab
            harness.app.view = from_view;
            let forward_steps = (to_idx + 6 - from_idx) % 6;
            for _ in 0..forward_steps {
                harness.send_key(KeyCode::Tab);
            }
            assert_eq!(
                harness.app.view, to_view,
                "Tab cycling {forward_steps} times from {from_view:?} must reach {to_view:?}"
            );

            // Forward cycling with Right arrow
            harness.app.view = from_view;
            for _ in 0..forward_steps {
                harness.send_key(KeyCode::Right);
            }
            assert_eq!(
                harness.app.view, to_view,
                "Right arrow cycling {forward_steps} times from {from_view:?} must reach {to_view:?}"
            );

            // Backward cycling with BackTab
            harness.app.view = from_view;
            let backward_steps = (from_idx + 6 - to_idx) % 6;
            for _ in 0..backward_steps {
                harness.send_key(KeyCode::BackTab);
            }
            assert_eq!(
                harness.app.view, to_view,
                "BackTab cycling {backward_steps} times from {from_view:?} must reach {to_view:?}"
            );

            // Backward cycling with Left arrow
            harness.app.view = from_view;
            for _ in 0..backward_steps {
                harness.send_key(KeyCode::Left);
            }
            assert_eq!(
                harness.app.view, to_view,
                "Left arrow cycling {backward_steps} times from {from_view:?} must reach {to_view:?}"
            );
        }
    }
}

/// Sub-view navigation automaton: Tests transitions out of modal/inspection views
/// (JobInspection, MediaInspection, CompletedInspection) via direct shortcuts and Escape.
#[test]
fn test_automaton_inspection_subview_transitions() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // 1. From JobInspection
    harness.app.view = ViewPage::JobInspection;
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(harness.app.view, ViewPage::MediaSources);

    harness.app.view = ViewPage::JobInspection;
    harness.send_key(KeyCode::Char('s'));
    assert_eq!(harness.app.view, ViewPage::MediaSources);

    harness.app.view = ViewPage::JobInspection;
    harness.send_key(KeyCode::Esc);
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    // 2. From MediaInspection
    harness.app.view = ViewPage::MediaInspection;
    harness.send_key(KeyCode::Char('1'));
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);

    harness.app.view = ViewPage::MediaInspection;
    harness.send_key(KeyCode::Char('s'));
    assert_eq!(harness.app.view, ViewPage::MediaSources);

    harness.app.view = ViewPage::MediaInspection;
    harness.send_key(KeyCode::Esc);
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    // 3. From CompletedInspection
    harness.app.view = ViewPage::CompletedInspection;
    harness.send_key(KeyCode::Char('m'));
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    harness.app.view = ViewPage::CompletedInspection;
    harness.send_key(KeyCode::Esc);
    assert_eq!(harness.app.view, ViewPage::CompletedMedia);
}

/// Contextual Help view automaton: Tests opening help from every view, switching help tabs
/// via direct shortcuts 1..5 / mnemonics, and returning to caller view via '?' and Esc.
#[test]
fn test_automaton_help_view_tab_routing_and_return() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    for &origin_view in &PRIMARY_VIEWS {
        harness.app.view = origin_view;

        // Open help with '?'
        harness.send_key(KeyCode::Char('?'));
        assert_eq!(harness.app.view, ViewPage::Help);
        assert_eq!(harness.app.help_caller_view, origin_view);

        // Switch to Sources help tab via '5' and 's'
        harness.send_key(KeyCode::Char('5'));
        assert_eq!(harness.app.help_tab, HelpTab::MediaSources);

        harness.send_key(KeyCode::Char('1'));
        assert_eq!(harness.app.help_tab, HelpTab::ActiveJobs);

        harness.send_key(KeyCode::Char('s'));
        assert_eq!(harness.app.help_tab, HelpTab::MediaSources);

        harness.send_key(KeyCode::Char('h'));
        assert_eq!(harness.app.help_tab, HelpTab::StatsHistory);

        harness.send_key(KeyCode::Char('r'));
        assert_eq!(harness.app.help_tab, HelpTab::Completed);

        harness.send_key(KeyCode::Char('m'));
        assert_eq!(harness.app.help_tab, HelpTab::MediaBrowser);

        // Close help with '?' -> returns to origin_view
        harness.send_key(KeyCode::Char('?'));
        assert_eq!(harness.app.view, origin_view);
    }
}
