//! User scenario tests: Media Sources CRUD, modal interactions & invalidation.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::{
    MediaSourceConfig, SourceModalField, SourceModalMode, ViewPage,
};
use dbsrt_cli::commands::top::sampler::{
    add_media_source, delete_media_source, save_media_sources, update_media_source,
    MEDIA_RESCAN_REQUESTED,
};
use std::fs;
use std::sync::atomic::Ordering;

use super::common::{mock_complete_telemetry, TestHarness};

#[test]
fn test_scenario_source_instant_invalidation_on_space_toggle() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // 1. Check initial media count
    harness.send_key(KeyCode::Char('4'));
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);
    harness.assert_buffer_contains("5/5 DISPONIBLES");
    harness.assert_buffer_contains("Movie.A.2024");
    harness.assert_buffer_contains("Show.S01E01");

    // 2. Go to Sources view ('5')
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(harness.app.view, ViewPage::MediaSources);
    harness.assert_buffer_contains("Synology NAS (video)");
    harness.assert_buffer_contains("Synology NAS (tvshow)");
    harness.assert_buffer_contains("SCAN DISQUE");
    harness.assert_buffer_contains("DISPONIBLES");
    harness.assert_buffer_contains("3 dispo");
    harness.assert_buffer_contains("2 dispo");

    // 3. Toggle first source (video) to INACTIF
    harness.app.source_scroll_index = 0;
    MEDIA_RESCAN_REQUESTED.store(false, Ordering::Relaxed);
    harness.send_key(KeyCode::Char(' '));

    // In Sources view, scan result remains visible (3 (+1 nouv.)) while disponibles drops to 0 (inactif)
    harness.assert_buffer_contains("3 (+1 nouv.)");
    harness.assert_buffer_contains("0 (inactif)");
    harness.assert_buffer_contains("2 dispo");

    // Verify 0ms memory invalidation
    {
        let tel = harness.shared_state.read().unwrap();
        assert!(!tel.media_sources[0].enabled);
        // Video items purged, only tvshow items remain (2 items)
        assert_eq!(tel.discovered_media.len(), 2);
        assert!(tel
            .discovered_media
            .iter()
            .all(|m| m.source_id.as_deref() == Some("nas_tvshow")));
    }
    assert!(MEDIA_RESCAN_REQUESTED.load(Ordering::Relaxed));

    // 4. Return to MediaBrowser: buffer shows only tvshow items
    harness.send_key(KeyCode::Char('4'));
    harness.assert_buffer_contains("2/2 DISPONIBLES");
    harness.assert_buffer_contains("Show.S01E01");
    harness.assert_buffer_does_not_contain("Movie.A.2024");
}

#[test]
fn test_scenario_source_modal_add_and_submit() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // Navigate to Sources view
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(harness.app.view, ViewPage::MediaSources);

    // Press 'a' to open Add Source modal
    harness.send_key(KeyCode::Char('a'));
    assert!(harness.app.source_modal.is_some());
    let modal = harness.app.source_modal.as_ref().unwrap();
    assert_eq!(modal.mode, SourceModalMode::Add);
    assert_eq!(modal.active_field, SourceModalField::Path);

    // Modal is rendered on screen
    harness.assert_buffer_contains("AJOUTER UNE NOUVELLE SOURCE");
    harness.assert_buffer_contains("Chemin du Répertoire (obligatoire)");

    // Type directory path
    harness.type_text("/mnt/nas/anime");
    assert_eq!(
        harness.app.source_modal.as_ref().unwrap().path_input,
        "/mnt/nas/anime"
    );

    // Press Tab or Down to switch to Name field
    harness.send_key(KeyCode::Tab);
    assert_eq!(
        harness.app.source_modal.as_ref().unwrap().active_field,
        SourceModalField::Name
    );

    // Type source label
    harness.type_text("NAS Anime");
    assert_eq!(
        harness.app.source_modal.as_ref().unwrap().name_input,
        "NAS Anime"
    );

    // Backspace editing works
    harness.send_key(KeyCode::Backspace);
    assert_eq!(
        harness.app.source_modal.as_ref().unwrap().name_input,
        "NAS Anim"
    );
    harness.send_key(KeyCode::Char('e'));

    // Cancel modal with Esc
    harness.send_key(KeyCode::Esc);
    assert!(harness.app.source_modal.is_none());
    harness.assert_buffer_does_not_contain("AJOUTER UNE NOUVELLE SOURCE");
}

#[test]
fn test_scenario_source_modal_edit_and_cancel() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // Go to Sources view
    harness.send_key(KeyCode::Char('5'));
    harness.app.source_scroll_index = 1; // Synology NAS (tvshow)

    // Press 'e' to open Edit modal
    harness.send_key(KeyCode::Char('e'));
    assert!(harness.app.source_modal.is_some());
    let modal = harness.app.source_modal.as_ref().unwrap();
    assert_eq!(modal.mode, SourceModalMode::Edit(1));
    assert_eq!(modal.path_input, "/mnt/nas/tvshow");
    assert_eq!(modal.name_input, "Synology NAS (tvshow)");

    // Modal rendered with EDITER LA SOURCE
    harness.assert_buffer_contains("MODIFIER LA SOURCE");

    // Close modal with Esc
    harness.send_key(KeyCode::Esc);
    assert!(harness.app.source_modal.is_none());
}

#[test]
fn test_scenario_sources_crud_disk_operations_and_solitary_protection() {
    let tmp_dir = std::env::temp_dir().join(format!(
        "pydbsrt_sources_disk_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&tmp_dir).unwrap();
    let sources_file = tmp_dir.join("media_sources.json");

    let mut sources = vec![
        MediaSourceConfig {
            id: "src_1".to_string(),
            name: "Source 1".to_string(),
            path: tmp_dir.join("dir1"),
            enabled: true,
            media_count: 0,
            last_scan_added: 0,
            last_scan_iso: None,
        },
        MediaSourceConfig {
            id: "src_2".to_string(),
            name: "Source 2".to_string(),
            path: tmp_dir.join("dir2"),
            enabled: true,
            media_count: 0,
            last_scan_added: 0,
            last_scan_iso: None,
        },
    ];
    save_media_sources(&sources_file, &sources).unwrap();

    // 1. Add new source
    let added = add_media_source(
        &sources_file,
        &mut sources,
        "Source 3".to_string(),
        tmp_dir.join("dir3"),
    )
    .unwrap();
    assert_eq!(added.name, "Source 3");
    assert_eq!(sources.len(), 3);

    // 2. Update source
    let updated = update_media_source(
        &sources_file,
        &mut sources,
        0,
        "Source 1 Renamed".to_string(),
        tmp_dir.join("dir1_new"),
    )
    .unwrap();
    assert_eq!(updated.name, "Source 1 Renamed");
    assert_eq!(sources[0].path, tmp_dir.join("dir1_new"));

    // 3. Delete source 2
    let deleted = delete_media_source(&sources_file, &mut sources, 1).unwrap();
    assert_eq!(deleted.name, "Source 2");
    assert_eq!(sources.len(), 2);

    // 4. Delete source 0 -> 1 source left
    let _ = delete_media_source(&sources_file, &mut sources, 0).unwrap();
    assert_eq!(sources.len(), 1);

    // 5. Attempt to delete solitary source -> must be blocked with error
    let err = delete_media_source(&sources_file, &mut sources, 0).unwrap_err();
    assert!(
        err.contains("Impossible de supprimer la seule source restante"),
        "Expected solitary protection error, got: {err}"
    );
    assert_eq!(sources.len(), 1);

    let _ = fs::remove_dir_all(&tmp_dir);
}
