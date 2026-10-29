//! User scenario tests: Contextual help 1:1 view parity, exhaustive glossary & navigation.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::{HelpTab, ViewPage};

use super::common::{mock_complete_telemetry, TestHarness};

#[test]
fn test_scenario_contextual_help_calling_parity_and_glossary() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    let views_and_glossary = vec![
        (
            ViewPage::ActiveJobs,
            KeyCode::Char('1'),
            HelpTab::ActiveJobs,
            vec!["PID", "MOTEUR", "PROGRESSION", "DÉBIT VFS"],
        ),
        (
            ViewPage::StatsHistory,
            KeyCode::Char('2'),
            HelpTab::StatsHistory,
            vec!["Débit CIFS", "Durée", "Taille", "Médiane"],
        ),
        (
            ViewPage::CompletedMedia,
            KeyCode::Char('3'),
            HelpTab::Completed,
            vec![
                "COUPLINGS",
                "STATUT",
                "OK",
                "CACHE",
                "NO_SRT",
                "CANCEL",
                "FAIL",
            ],
        ),
        (
            ViewPage::MediaBrowser,
            KeyCode::Char('4'),
            HelpTab::MediaBrowser,
            vec![
                "STATUT",
                "VIERGE",
                "EN COURS",
                "COMPLÉTÉ",
                "HACHÉ SEUL",
                "SOUS-TITRES",
            ],
        ),
        (
            ViewPage::MediaSources,
            KeyCode::Char('5'),
            HelpTab::MediaSources,
            vec![
                "STATUT",
                "NOM DE LA SOURCE",
                "CHEMIN DU RÉPERTOIRE",
                "ACTIF",
                "SCAN DISQUE",
                "DISPONIBLES",
            ],
        ),
    ];

    for (calling_view, view_key, expected_tab, expected_terms) in views_and_glossary {
        harness.send_key(view_key);
        assert_eq!(harness.app.view, calling_view);

        // Open help with '?'
        harness.send_key(KeyCode::Char('?'));
        assert_eq!(harness.app.view, ViewPage::Help);
        assert_eq!(harness.app.help_tab, expected_tab);

        // Assert all technical glossary terms exist
        for term in expected_terms {
            harness.assert_buffer_contains(term);
        }

        // Close help with '?'
        harness.send_key(KeyCode::Char('?'));
        assert_eq!(harness.app.view, calling_view);
    }
}

#[test]
fn test_scenario_help_tab_cycling_and_scrolling() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // Open help directly
    harness.send_key(KeyCode::Char('?'));
    assert_eq!(harness.app.view, ViewPage::Help);
    assert_eq!(harness.app.help_tab, HelpTab::ActiveJobs);

    // Tab cycles forward: ActiveJobs -> StatsHistory -> Completed -> MediaBrowser -> MediaSources -> General
    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.help_tab, HelpTab::StatsHistory);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.help_tab, HelpTab::Completed);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.help_tab, HelpTab::MediaBrowser);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.help_tab, HelpTab::MediaSources);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.help_tab, HelpTab::System);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.help_tab, HelpTab::General);

    harness.send_key(KeyCode::Tab);
    assert_eq!(harness.app.help_tab, HelpTab::ActiveJobs);

    // BackTab cycles backward
    harness.send_key(KeyCode::BackTab);
    assert_eq!(harness.app.help_tab, HelpTab::General);

    // Direct jump keys: '0', '1', '2', '3', '4', '5', '6'
    harness.send_key(KeyCode::Char('6'));
    assert_eq!(harness.app.help_tab, HelpTab::System);

    harness.send_key(KeyCode::Char('3'));
    assert_eq!(harness.app.help_tab, HelpTab::Completed);

    harness.send_key(KeyCode::Char('0'));
    assert_eq!(harness.app.help_tab, HelpTab::General);

    // Help scrolling
    assert_eq!(harness.app.help_scroll_index, 0);
    harness.send_key(KeyCode::Char('j'));
    assert_eq!(harness.app.help_scroll_index, 1);
    harness.send_key(KeyCode::PageDown);
    assert!(harness.app.help_scroll_index > 1);
    harness.send_key(KeyCode::Home);
    assert_eq!(harness.app.help_scroll_index, 0);

    // Exit with Esc
    harness.send_key(KeyCode::Esc);
    assert_eq!(harness.app.view, ViewPage::ActiveJobs);
}
