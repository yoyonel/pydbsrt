//! View page rendering for the Completed & Certified Media Releases table.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::{
    constants::{BYTES_PER_MIB_F64, MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS},
    model::{
        filter_and_sort_completed_with_search, format_duration, CompletedFilter, CompletedStatus,
        HistoricalRecord,
    },
    ui::UiContext,
    views::widgets::completed_status_badge,
};

/// Truncation ellipsis suffix length.
const TRUNCATION_SUFFIX_LEN: usize = 3;

/// Table column layout widths for the completed jobs grid.
const COMPLETED_COLUMN_WIDTHS: [Constraint; 8] = [
    Constraint::Length(3),
    Constraint::Percentage(36),
    Constraint::Length(10),
    Constraint::Length(11),
    Constraint::Length(11),
    Constraint::Length(12),
    Constraint::Length(14),
    Constraint::Length(22),
];

/// Renders the fullframe scrollable list of completed and certified media releases.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `ctx` - Active UI rendering context containing telemetry and search/filter state.
///
/// # Data Flow & Invariants
/// Filters historical batch records by current filter, sort, and search query.
/// Splits inner rect into a top action toolbar and a main table area.
pub fn render_completed_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let records = filter_and_sort_completed_with_search(
        &ctx.telemetry.history_records,
        ctx.completed_filter,
        ctx.completed_sort,
        if ctx.search_query.is_empty() {
            None
        } else {
            Some(ctx.search_query)
        },
    );

    let total_count = ctx.telemetry.history_records.len();
    let visible_count = records.len();

    let title = format!(
        " 📋 DERNIERS TRAITEMENTS COMPLÉTÉS & CERTIFIÉS [{visible_count}/{total_count} RÉFÉRENCÉS] "
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightBlue))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sub_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    render_completed_toolbar(frame, sub_chunks[0], ctx);

    if records.is_empty() {
        render_empty_completed_placeholder(frame, sub_chunks[1], ctx.search_query);
    } else {
        render_completed_table(frame, sub_chunks[1], &records, ctx.completed_scroll_index);
    }
}

/// Renders the filter, sort, search, and inspection shortcut action bar.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target bounding rectangle for the toolbar.
/// * `ctx` - Active UI rendering context.
///
/// # Data Flow & Invariants
/// Produces a highlighted summary line reflecting active search queries, active filters,
/// and available shortcut keys.
pub fn render_completed_toolbar(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let filter_color = match ctx.completed_filter {
        CompletedFilter::All => Color::Yellow,
        CompletedFilter::FailedOnly => Color::Red,
        CompletedFilter::SuccessOnly => Color::Green,
        CompletedFilter::NoSrtOnly => Color::Yellow,
        CompletedFilter::CancelledOnly => Color::Magenta,
    };

    let mut bar_spans = vec![
        Span::styled(" [f] Filtre: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", ctx.completed_filter.label()),
            Style::default()
                .fg(filter_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(" [o] Tri: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", ctx.completed_sort.label()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
    ];

    if ctx.search_active {
        bar_spans.push(Span::styled(
            format!(
                " [/] RECHERCHE: \"{}█\" (Entrée: Valider, Échap: Annuler)",
                ctx.search_query
            ),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else if !ctx.search_query.is_empty() {
        bar_spans.push(Span::styled(
            " [/] Recherche: ",
            Style::default().fg(Color::DarkGray),
        ));
        bar_spans.push(Span::styled(
            format!("\"{}\" ", ctx.search_query),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        bar_spans.push(Span::styled(
            "[Échap: Effacer]",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        bar_spans.push(Span::styled(
            "[/] Chercher",
            Style::default().fg(Color::DarkGray),
        ));
    }

    bar_spans.push(Span::raw("   "));
    bar_spans.push(Span::styled(
        "[Enter/i] Fiche Diagnostic & Erreur",
        Style::default().fg(Color::DarkGray),
    ));

    let bar_p = Paragraph::new(Line::from(bar_spans));
    frame.render_widget(bar_p, area);
}

/// Renders a centered informative placeholder when no historical records match criteria.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `search_query` - Current active text search string.
///
/// # Data Flow & Invariants
/// Displays contextual remediation instructions (press Escape to clear search or change filter).
pub fn render_empty_completed_placeholder(frame: &mut Frame, area: Rect, search_query: &str) {
    let msg_detail = if !search_query.is_empty() {
        format!(
            "Aucun résultat pour la recherche \"{search_query}\". Appuyez sur [Échap] pour réinitialiser."
        )
    } else {
        "Appuyez sur [f] pour modifier le filtre ou vérifiez l'indexation de data/reports/batch_tvshow_nightly.jsonl.".to_string()
    };

    let empty_msg = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Aucun enregistrement ne correspond aux critères actifs.",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            msg_detail,
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(empty_msg).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(p, area);
}

/// Renders the multi-column historical completed jobs table.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `records` - Filtered slice of historical records.
/// * `selected_index` - Currently highlighted row cursor index.
///
/// # Data Flow & Invariants
/// Assembles header, applies cursor styles, and renders stateful table widget.
pub fn render_completed_table(
    frame: &mut Frame,
    area: Rect,
    records: &[HistoricalRecord],
    selected_index: usize,
) {
    let header_cells = [
        " ",
        "MÉDIA / ÉPISODE TRAITÉ",
        "STATUT",
        "TAILLE",
        "DURÉE",
        "COUPLINGS",
        "DÉBIT CIFS",
        "DATE FIN",
    ]
    .iter()
    .map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = records
        .iter()
        .enumerate()
        .map(|(idx, r)| build_completed_row(r, idx == selected_index))
        .collect();

    let table = Table::new(rows, COMPLETED_COLUMN_WIDTHS).header(header);

    let mut state = TableState::default();
    state.select(Some(selected_index));
    frame.render_stateful_widget(table, area, &mut state);
}

/// Formats a single historical record into a styled table row.
///
/// # Arguments
/// * `r` - Historical record reference.
/// * `is_selected` - Whether this row currently holds keyboard cursor focus.
///
/// # Returns
/// Ratatui `Row` populated with cursor, truncated title, badge, duration, CIFS throughput, and timestamp.
///
/// # Data Flow & Invariants
/// Cache records display `< 1s` duration and `N/A (Cache)` throughput.
pub fn build_completed_row(r: &HistoricalRecord, is_selected: bool) -> Row<'static> {
    let cursor = if is_selected { "► " } else { "  " };
    let cursor_span = Span::styled(
        cursor,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let status_cell = completed_status_badge(r.status());

    let mut media_display = r.video_stem.clone();
    if media_display.chars().count() > MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS {
        media_display = media_display
            .chars()
            .take(MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS - TRUNCATION_SUFFIX_LEN)
            .collect::<String>()
            + "...";
    }

    let size_str = if r.file_size_bytes > 0 {
        format!("{:.0} MB", r.file_size_bytes as f64 / BYTES_PER_MIB_F64)
    } else {
        "-".to_string()
    };

    let is_cache = r.status() == CompletedStatus::Cache;

    let dur_str = if is_cache {
        "< 1s".to_string()
    } else {
        format_duration(r.wall_duration_secs)
    };

    let rate_str = if is_cache {
        "N/A (Cache)".to_string()
    } else {
        format!("{:>5.1} MB/s", r.cifs_read_rate_mib_s)
    };

    let date_str = r
        .end_iso
        .as_deref()
        .or(r.start_iso.as_deref())
        .unwrap_or("-");

    let row_cells = vec![
        Cell::from(cursor_span),
        Cell::from(media_display),
        Cell::from(status_cell),
        Cell::from(size_str),
        Cell::from(dur_str),
        Cell::from(format!("+{}", r.certified_couplings_count)),
        Cell::from(rate_str),
        Cell::from(date_str.to_string()),
    ];

    let row_style = if is_selected {
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Row::new(row_cells).style(row_style)
}
