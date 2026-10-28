//! User scenario tests: Media Sources CRUD, modal interactions & invalidation.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::{
    MediaSourceConfig, SourceModalField, SourceModalMode, ViewPage,
};
use dbsrt_cli::commands::top::sampler::{
    add_media_source, delete_media_source, merge_scanned_source_metrics, save_media_sources,
    scan_all_media_sources_with_progress, update_media_source, MEDIA_RESCAN_REQUESTED,
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

    // Verify non-destructive toggle: persistent catalog in memory preserves all 5 items
    {
        let tel = harness.shared_state.read().unwrap();
        assert!(!tel.media_sources[0].enabled);
        assert_eq!(tel.discovered_media.len(), 5);
    }

    // 4. Return to MediaBrowser: default ActiveOnly filter shows only active tvshow items (2/5)
    harness.send_key(KeyCode::Char('4'));
    harness.assert_buffer_contains("2/5 DISPONIBLES sur 1/2 source active");
    harness.assert_buffer_contains("Show.S01E01");
    harness.assert_buffer_does_not_contain("Movie.A.2024");
    // Origin source badge is always preserved so operator knows where the media comes from
    harness.assert_buffer_contains("[tvshow] Show.S01E01");

    // 5. Press 'S' to switch filter to [TOUTES]: all 5 items in persistent catalog are visible!
    harness.send_key(KeyCode::Char('S'));
    harness.assert_buffer_contains("5/5 DISPONIBLES (Toutes sources)");
    harness.assert_buffer_contains("Movie.A.2024");
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

#[test]
fn test_scenario_multi_source_synchronization_and_table_expansion() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());

    // 1. Initial nominal multi-source state: 2 active sources, ActiveOnly default
    harness.send_key(KeyCode::Char('4'));
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);
    harness.assert_buffer_contains("5/5 DISPONIBLES sur 2 sources");
    harness.assert_buffer_contains("[video] Movie.A.2024");
    harness.assert_buffer_contains("[tvshow] Show.S01E01");
    harness.assert_buffer_contains("[S] Source: [ACTIVES]");

    // 2. Switch to Sources view: displays [2]
    harness.send_key(KeyCode::Char('5'));
    assert_eq!(harness.app.view, ViewPage::MediaSources);
    harness.assert_buffer_contains("GESTION DES SOURCES DE MÉDIAS (RÉPERTOIRES VIDÉO) [2]");

    // 3. Disable first source ([Space])
    harness.send_key(KeyCode::Char(' '));
    harness
        .assert_buffer_contains("GESTION DES SOURCES DE MÉDIAS (RÉPERTOIRES VIDÉO) [1/2 ACTIVES]");
    harness.assert_buffer_contains("0 (inactif)");
    harness.assert_buffer_contains("2 dispo");

    // 4. Return to MediaBrowser: title and rows reflect solitary active source under ActiveOnly default
    harness.send_key(KeyCode::Char('4'));
    harness.assert_buffer_contains("2/5 DISPONIBLES sur 1/2 source active");
    harness.assert_buffer_contains("[S] Source: [ACTIVES]");
    harness.assert_buffer_contains("[tvshow] Show.S01E01");

    // 5. Press [S] to cycle to TOUTES: shows all 5 items from persistent catalog
    harness.send_key(KeyCode::Char('S'));
    harness.assert_buffer_contains("5/5 DISPONIBLES (Toutes sources)");
    harness.assert_buffer_contains("[S] Source: [TOUTES]");

    // 6. Disable second source as well via Tab 5
    harness.send_key(KeyCode::Char('5'));
    harness.send_key(KeyCode::Char('j')); // move to second source
    harness.send_key(KeyCode::Char(' ')); // toggle off
    harness
        .assert_buffer_contains("GESTION DES SOURCES DE MÉDIAS (RÉPERTOIRES VIDÉO) [0/2 ACTIVES]");

    // 7. MediaBrowser reflects 0 active sources when under ActiveOnly
    harness.send_key(KeyCode::Char('4'));
    // Press 'S' three times to cycle back to ActiveOnly
    harness.send_key(KeyCode::Char('S'));
    harness.send_key(KeyCode::Char('S'));
    harness.send_key(KeyCode::Char('S'));
    harness.assert_buffer_contains("0/5 DISPONIBLES (toutes sources inactives)");
}

#[test]
fn test_merge_scanned_source_metrics_preserves_operator_toggle_and_config() {
    let mut target = vec![
        MediaSourceConfig {
            id: "src_video".to_string(),
            name: "Video Source".to_string(),
            path: std::path::PathBuf::from("/media/video"),
            enabled: false, // Operator explicitly disabled this
            media_count: 50,
            last_scan_added: 0,
            last_scan_iso: None,
        },
        MediaSourceConfig {
            id: "src_tvshow".to_string(),
            name: "TV Show Source".to_string(),
            path: std::path::PathBuf::from("/media/tvshow"),
            enabled: true,
            media_count: 10,
            last_scan_added: 0,
            last_scan_iso: None,
        },
    ];

    let scanned = vec![
        MediaSourceConfig {
            id: "src_video".to_string(),
            name: "Video Source Old Name".to_string(),
            path: std::path::PathBuf::from("/media/video"),
            enabled: true, // Scanner had stale enabled flag
            media_count: 1549,
            last_scan_added: 1500,
            last_scan_iso: Some("2026-10-27T10:00:00Z".to_string()),
        },
        MediaSourceConfig {
            id: "src_tvshow".to_string(),
            name: "TV Show Source".to_string(),
            path: std::path::PathBuf::from("/media/tvshow"),
            enabled: true,
            media_count: 1039,
            last_scan_added: 200,
            last_scan_iso: Some("2026-10-27T10:05:00Z".to_string()),
        },
    ];

    merge_scanned_source_metrics(&mut target, &scanned);

    // Operator's disabled choice MUST NOT be overwritten by scanner
    assert!(
        !target[0].enabled,
        "Operator disabled status must be preserved"
    );
    assert_eq!(
        target[0].media_count, 1549,
        "Media count must be updated from scan"
    );
    assert_eq!(
        target[0].last_scan_added, 1500,
        "Last scan added must be updated"
    );
    assert_eq!(
        target[0].last_scan_iso.as_deref(),
        Some("2026-10-27T10:00:00Z")
    );
    assert_eq!(target[0].name, "Video Source", "Name must be preserved");

    // Second source verified
    assert!(target[1].enabled);
    assert_eq!(target[1].media_count, 1039);
    assert_eq!(target[1].last_scan_added, 200);
}

#[test]
fn test_scan_all_media_sources_preserves_persistent_catalog_when_all_disabled() {
    let tmp_dir = std::env::temp_dir().join(format!("test_all_disabled_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp_dir);
    let sources = vec![
        MediaSourceConfig {
            id: "src1".to_string(),
            name: "Source 1".to_string(),
            path: tmp_dir.join("dir1"),
            enabled: false,
            media_count: 100,
            last_scan_added: 0,
            last_scan_iso: None,
        },
        MediaSourceConfig {
            id: "src2".to_string(),
            name: "Source 2".to_string(),
            path: tmp_dir.join("dir2"),
            enabled: false,
            media_count: 200,
            last_scan_added: 0,
            last_scan_iso: None,
        },
    ];

    let couplings_path = tmp_dir.join("couplings.json");
    let events_path = tmp_dir.join("events.jsonl");
    let phash_dir = tmp_dir.join("phash");
    let cache_path = tmp_dir.join("media_cache.json");

    // Create a non-empty cache to verify persistent catalog is preserved
    let fake_cached_item = dbsrt_cli::commands::top::model::DiscoveredMediaItem {
        path: tmp_dir.join("dir1/sample.mkv"),
        stem: "sample".to_string(),
        size_bytes: 1024,
        duration_secs: Some(60.0),
        video_codec: Some("h264".to_string()),
        subtitles: Vec::new(),
        phash: None,
        source_id: Some("src1".to_string()),
        status: dbsrt_cli::commands::top::model::IngestionStatus::Virgin,
    };
    dbsrt_cli::commands::top::sampler::save_media_cache(&cache_path, &[fake_cached_item]);

    let mut callback_invocations = 0;
    let (discovered, updated_sources) = scan_all_media_sources_with_progress(
        &sources,
        &couplings_path,
        &events_path,
        &phash_dir,
        &[],
        &cache_path,
        |_items, _sources| {
            callback_invocations += 1;
        },
    );

    assert_eq!(
        discovered.len(),
        1,
        "Persistent catalog items must be preserved even when all sources are disabled"
    );
    assert_eq!(
        callback_invocations, 0,
        "Zero callbacks should occur when no sources are enabled"
    );
    assert_eq!(updated_sources.len(), 2);

    let _ = fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_scenario_media_browser_item_deletion_via_key_d() {
    let mut harness = TestHarness::new(240, 45, mock_complete_telemetry());
    harness.send_key(KeyCode::Char('4'));
    assert_eq!(harness.app.view, ViewPage::MediaBrowser);

    // Initial count: 5 items in catalog
    {
        let tel = harness.shared_state.read().unwrap();
        assert_eq!(tel.discovered_media.len(), 5);
    }
    harness.assert_buffer_contains("5/5 DISPONIBLES");

    // Press 'd' to delete current item
    harness.send_key(KeyCode::Char('d'));

    // Catalog length drops to 4 in shared state and on screen
    {
        let tel = harness.shared_state.read().unwrap();
        assert_eq!(tel.discovered_media.len(), 4);
    }
    harness.assert_buffer_contains("4/4 DISPONIBLES");
    harness.assert_buffer_contains("1 média(s) supprimé(s) du catalogue persistant.");
}
