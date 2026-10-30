//! Full-frame UI rendering router for the dbsrt top process monitor.

use std::collections::HashSet;
use std::path::PathBuf;

use std::rc::Rc;

use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    Frame,
};

use crate::{
    app::KeyCode,
    constants::{FOOTER_BLOCK_HEIGHT, HEADER_BLOCK_HEIGHT, MIN_BODY_HEIGHT},
    model::{
        CompletedFilter, CompletedSort, HelpTab, MediaFilter, MediaSort, MediaSourceFilter,
        SourceInputModalState, SystemTelemetry, ViewPage,
    },
    views::{
        active_jobs::{render_active_jobs_page, render_inspection_page},
        completed::render_completed_page,
        completed_inspection::render_completed_inspection_page,
        footer::{footer_tab_hits, render_footer},
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
    let chunks = root_chunks(size);

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

/// Divides the screen: header block, dynamic full-frame body, footer.
fn root_chunks(size: Rect) -> Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_BLOCK_HEIGHT),
            Constraint::Min(MIN_BODY_HEIGHT),
            Constraint::Length(FOOTER_BLOCK_HEIGHT),
        ])
        .split(size)
}

/// Resolves a click or tap on cell (`column`, `row`) of a `size` screen into the key it stands for.
///
/// # Data Flow & Invariants
/// Uses the same layout as [`render_ui`], so a tap feeds `handle_key_event` exactly like the
/// key printed under the finger. Ignored while the source modal or search input owns the keys
/// (a tab digit would be typed into the field).
pub fn hit_test(size: Rect, ctx: &UiContext, column: u16, row: u16) -> Option<KeyCode> {
    if ctx.source_modal.is_some() || ctx.search_active {
        return None;
    }
    let position = Position::new(column, row);
    footer_tab_hits(root_chunks(size)[2])
        .into_iter()
        .find(|(zone, _)| zone.contains(position))
        .map(|(_, code)| code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, KeyCode};
    use ratatui::{backend::TestBackend, Terminal};

    /// Renders `app` on a `width` x `height` screen and returns the footer tab row text.
    fn tab_row(app: &AppState, telemetry: &SystemTelemetry, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render_ui(f, &app.ui_context(telemetry, None)))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, height - 1)].symbol())
            .collect()
    }

    #[test]
    fn test_narrow_screen_gets_tappable_numbered_tabs() {
        let telemetry = SystemTelemetry::default();
        let app = AppState::default();
        let size = Rect::new(0, 0, 40, 44);
        let row = tab_row(&app, &telemetry, size.width, size.height);
        assert_eq!(row.trim_end(), "  1     2     3     4     5     6");

        let ctx = app.ui_context(&telemetry, None);
        assert_eq!(hit_test(size, &ctx, 14, 43), Some(KeyCode::Char('3')));
        assert_eq!(hit_test(size, &ctx, 0, 43), Some(KeyCode::Char('1')));
        assert_eq!(hit_test(size, &ctx, 39, 43), None);
        assert_eq!(hit_test(size, &ctx, 14, 42), None);
    }

    #[test]
    fn test_wide_screen_tab_hits_cover_their_labels() {
        let telemetry = SystemTelemetry::default();
        let app = AppState::default();
        let size = Rect::new(0, 0, 140, 30);
        let row = tab_row(&app, &telemetry, size.width, size.height);
        assert!(row.contains("[6/y] Système"));
        assert!(row.contains("│ [↑/k] Défiler"));

        let ctx = app.ui_context(&telemetry, None);
        let column = row.chars().position(|c| c == '5').unwrap() as u16;
        assert_eq!(hit_test(size, &ctx, column, 29), Some(KeyCode::Char('5')));
        assert_eq!(hit_test(size, &ctx, 0, 29), None);
    }

    #[test]
    fn test_hit_test_ignored_while_typing() {
        let telemetry = SystemTelemetry::default();
        let app = AppState {
            search_active: true,
            ..Default::default()
        };
        let ctx = app.ui_context(&telemetry, None);
        assert_eq!(hit_test(Rect::new(0, 0, 40, 44), &ctx, 0, 43), None);
    }
}
