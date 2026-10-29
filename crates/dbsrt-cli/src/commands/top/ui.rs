//! Full-frame UI rendering router for the dbsrt top process monitor.

use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::commands::top::{
    constants::{FOOTER_BLOCK_HEIGHT, HEADER_BLOCK_HEIGHT, MIN_BODY_HEIGHT},
    model::{
        CompletedFilter, CompletedSort, HelpTab, MediaFilter, MediaSort, MediaSourceFilter,
        SourceInputModalState, SystemTelemetry, ViewPage,
    },
    views::{
        active_jobs::{render_active_jobs_page, render_inspection_page},
        completed::render_completed_page,
        completed_inspection::render_completed_inspection_page,
        footer::render_footer,
        header::render_header,
        help::render_help_page,
        media_browser::render_media_browser_page,
        media_inspection::render_media_inspection_page,
        media_sources::{render_media_sources_page, render_source_modal},
        stats::render_stats_page,
        system::render_system_page,
    },
};

/// UI rendering application context.
pub struct UiContext<'a> {
    pub telemetry: &'a SystemTelemetry,
    pub view: ViewPage,
    pub selected_index: usize,
    pub completed_scroll_index: usize,
    pub media_scroll_index: usize,
    pub source_scroll_index: usize,
    pub source_modal: Option<&'a SourceInputModalState>,
    pub selected_media_paths: &'a HashSet<PathBuf>,
    pub media_filter: MediaFilter,
    pub media_sort: MediaSort,
    pub source_filter: &'a MediaSourceFilter,
    pub completed_filter: CompletedFilter,
    pub completed_sort: CompletedSort,
    pub search_query: &'a str,
    pub search_active: bool,
    pub help_tab: HelpTab,
    pub help_scroll_index: usize,
    pub feedback_msg: Option<&'a str>,
}

/// Main entry point rendering full-frame UI onto the ratatui Frame.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `ctx` - Active UI rendering context containing telemetry and selection state.
///
/// # Data Flow & Invariants
/// Splits the root display into fixed header (5 rows), dynamic full-frame view body,
/// and fixed footer bar (2 rows). Displays source modal overlay if present.
pub fn render_ui(frame: &mut Frame, ctx: &UiContext) {
    let size = frame.area();

    // Divide screen: 5 lines header block, dynamic full-frame body, 2 lines footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_BLOCK_HEIGHT),
            Constraint::Min(MIN_BODY_HEIGHT),
            Constraint::Length(FOOTER_BLOCK_HEIGHT),
        ])
        .split(size);

    render_header(frame, chunks[0], ctx.telemetry);

    match ctx.view {
        ViewPage::ActiveJobs => render_active_jobs_page(frame, chunks[1], ctx),
        ViewPage::StatsHistory => render_stats_page(frame, chunks[1], ctx),
        ViewPage::CompletedMedia => render_completed_page(frame, chunks[1], ctx),
        ViewPage::MediaBrowser => render_media_browser_page(frame, chunks[1], ctx),
        ViewPage::MediaSources => render_media_sources_page(frame, chunks[1], ctx),
        ViewPage::System => render_system_page(frame, chunks[1], ctx),
        ViewPage::JobInspection => render_inspection_page(frame, chunks[1], ctx),
        ViewPage::MediaInspection => render_media_inspection_page(frame, chunks[1], ctx),
        ViewPage::CompletedInspection => render_completed_inspection_page(frame, chunks[1], ctx),
        ViewPage::Help => render_help_page(frame, chunks[1], ctx),
    }

    render_footer(frame, chunks[2], ctx);

    if let Some(modal) = ctx.source_modal {
        render_source_modal(frame, size, modal);
    }
}
