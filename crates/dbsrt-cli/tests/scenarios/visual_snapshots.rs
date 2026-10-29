//! Golden snapshot tests for dbsrt top terminal grids using insta (ISO Ratatui strategy).

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::model::ViewPage;

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
