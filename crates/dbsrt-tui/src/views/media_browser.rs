//! View page rendering for the interactive media browser catalog.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::{
    constants::{
        BYTES_PER_GIB, BYTES_PER_GIB_F64, BYTES_PER_MIB_F64, MEDIA_BROWSER_NON_STEM_COLUMNS_WIDTH,
    },
    model::{
        filter_and_sort_media_with_source_and_search, format_duration, DiscoveredMediaItem,
        MediaSourceConfig, MediaSourceFilter,
    },
    ui::UiContext,
    views::widgets::ingestion_status_badge,
};

/// Minimum allocated column width for media filename stem rendering.
const MIN_STEM_COLUMN_WIDTH: usize = 15;

/// Truncation ellipsis character length.
const TRUNCATION_SUFFIX_LEN: usize = 3;

/// Conversion factor from frames count to thousand frames (kf).
const THOUSAND_FRAMES_F64: f64 = 1000.0;

/// Conversion factor from bytes to Kilobytes.
const BYTES_PER_KIB_F64: f64 = 1024.0;

/// Maximum number of language subtitle tags displayed in the summary cell.
const MAX_SUMMARY_LANG_TAGS: usize = 3;

/// Table column widths for the media browser grid.
const MEDIA_BROWSER_COLUMN_WIDTHS: [Constraint; 9] = [
    Constraint::Length(3),
    Constraint::Length(5),
    Constraint::Fill(1),
    Constraint::Length(10),
    Constraint::Length(9),
    Constraint::Length(18),
    Constraint::Length(15),
    Constraint::Length(10),
    Constraint::Length(14),
];

/// Renders the interactive media browser allowing inspection, filtering, sorting and batch execution.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `ctx` - Active UI rendering context containing telemetry and selection state.
///
/// # Data Flow & Invariants
/// Filters discovered media files across active sources and query strings, rendering the title
/// banner, toolbar, and the stateful multi-column browser table.
pub fn render_media_browser_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let items = filter_and_sort_media_with_source_and_search(
        &ctx.telemetry.discovered_media,
        ctx.media_filter,
        ctx.media_sort,
        ctx.source_filter,
        &ctx.telemetry.media_sources,
        if ctx.search_query.is_empty() {
            None
        } else {
            Some(ctx.search_query)
        },
    );

    let total_count = ctx.telemetry.discovered_media.len();
    let visible_count = items.len();

    let title_line = build_browser_title_line(ctx, visible_count, total_count);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title_line);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sub_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    render_media_browser_toolbar(frame, sub_chunks[0], ctx);

    if items.is_empty() {
        render_empty_media_browser_placeholder(frame, sub_chunks[1], ctx.search_query);
    } else {
        render_media_browser_table(frame, sub_chunks[1], &items, ctx);
    }
}

/// Builds the titled header Line describing source scoping and catalog scanning status.
///
/// # Arguments
/// * `ctx` - Active UI rendering context.
/// * `visible_count` - Number of items currently passing filters.
/// * `total_count` - Total number of discovered items across all sources.
///
/// # Returns
/// Ratatui `Line` containing cyan title badge and optional yellow scanner indicator.
pub fn build_browser_title_line(
    ctx: &UiContext,
    visible_count: usize,
    total_count: usize,
) -> Line<'static> {
    let active_sources = ctx
        .telemetry
        .media_sources
        .iter()
        .filter(|s| s.enabled)
        .count();
    let total_sources = ctx.telemetry.media_sources.len();

    let title_text = match ctx.source_filter {
        MediaSourceFilter::ActiveOnly => {
            if total_sources <= 1 {
                format!(" 📂 EXPLORATEUR DE MÉDIAS [{visible_count}/{total_count} DISPONIBLES] ")
            } else if active_sources == 0 {
                format!(" 📂 EXPLORATEUR DE MÉDIAS MULTI-SOURCES [0/{total_count} DISPONIBLES (toutes sources inactives)] ")
            } else if active_sources == 1 {
                format!(" 📂 EXPLORATEUR DE MÉDIAS MULTI-SOURCES [{visible_count}/{total_count} DISPONIBLES sur 1/{total_sources} source active] ")
            } else if active_sources < total_sources {
                format!(" 📂 EXPLORATEUR DE MÉDIAS MULTI-SOURCES [{visible_count}/{total_count} DISPONIBLES sur {active_sources}/{total_sources} sources actives] ")
            } else {
                format!(" 📂 EXPLORATEUR DE MÉDIAS MULTI-SOURCES [{visible_count}/{total_count} DISPONIBLES sur {total_sources} sources] ")
            }
        }
        MediaSourceFilter::All => {
            format!(" 📂 EXPLORATEUR DE MÉDIAS MULTI-SOURCES [{visible_count}/{total_count} DISPONIBLES (Toutes sources)] ")
        }
        MediaSourceFilter::Specific(id) => {
            let name = ctx
                .telemetry
                .media_sources
                .iter()
                .find(|s| &s.id == id)
                .map(|s| s.name.as_str())
                .unwrap_or(id.as_str());
            format!(" 📂 EXPLORATEUR DE MÉDIAS MULTI-SOURCES [{visible_count}/{total_count} DISPONIBLES (Source: {name})] ")
        }
    };

    let mut spans = vec![Span::styled(
        title_text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    if ctx.telemetry.is_scanning {
        spans.push(Span::styled(
            "[⟳ ACTUALISATION DU CATALOGUE EN COURS...] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    Line::from(spans)
}

/// Renders the action toolbar featuring filter, sort, source scope, search input, and selection badges.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target bounding rectangle for the toolbar.
/// * `ctx` - Active UI rendering context.
pub fn render_media_browser_toolbar(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let total_sources = ctx.telemetry.media_sources.len();
    let selected_count = ctx.selected_media_paths.len();

    let mut bar_spans = vec![
        Span::styled(" [f] Filtre: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", ctx.media_filter.label()),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(" [o] Tri: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", ctx.media_sort.label()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if total_sources > 1 {
        let source_filter_name = ctx.source_filter.label(&ctx.telemetry.media_sources);
        bar_spans.push(Span::raw("   "));
        bar_spans.push(Span::styled(
            " [S] Source: ",
            Style::default().fg(Color::DarkGray),
        ));
        bar_spans.push(Span::styled(
            format!("[{source_filter_name}]"),
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if ctx.search_active {
        bar_spans.push(Span::raw("   "));
        bar_spans.push(Span::styled(
            format!(
                "[/] RECHERCHE: \"{}█\" (Entrée: Valider, Échap: Annuler)",
                ctx.search_query
            ),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else if !ctx.search_query.is_empty() {
        bar_spans.push(Span::raw("   "));
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
        bar_spans.push(Span::raw("   "));
        bar_spans.push(Span::styled(
            "[/] Chercher",
            Style::default().fg(Color::DarkGray),
        ));
    }

    bar_spans.push(Span::raw("   "));
    bar_spans.push(Span::styled(
        " Sélection: ",
        Style::default().fg(Color::DarkGray),
    ));
    bar_spans.push(Span::styled(
        format!("{selected_count} média(s) coché(s)"),
        if selected_count > 0 {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        },
    ));
    bar_spans.push(Span::raw("   "));
    bar_spans.push(Span::styled(
        " [d] Supprimer",
        Style::default().fg(Color::Red),
    ));

    let bar_p = Paragraph::new(Line::from(bar_spans));
    frame.render_widget(bar_p, area);
}

/// Renders a centered informative placeholder when no catalog items match criteria.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `search_query` - Current active text search string.
pub fn render_empty_media_browser_placeholder(frame: &mut Frame, area: Rect, search_query: &str) {
    let msg_detail = if !search_query.is_empty() {
        format!(
            "Aucun média ne correspond à la recherche \"{search_query}\". Appuyez sur [Échap] pour réinitialiser."
        )
    } else {
        "Appuyez sur [f] pour modifier le filtre ou vérifiez la liaison NAS Synology.".to_string()
    };
    let empty_msg = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Aucun média disponible ne correspond aux critères actifs.",
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
    let p = Paragraph::new(empty_msg).alignment(Alignment::Center);
    frame.render_widget(p, area);
}

/// Rendering context for an individual media browser table row.
pub struct MediaRowContext<'a> {
    /// Whether the row currently holds keyboard cursor focus.
    pub is_cursor: bool,
    /// Whether the row is batch-checked.
    pub is_checked: bool,
    /// Configured media sources slice.
    pub sources: &'a [MediaSourceConfig],
    /// Computed available character width for the truncated filename stem.
    pub available_media_width: usize,
}

/// Renders the multi-column scrollable table of discovered media releases.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `items` - Filtered slice of discovered media items.
/// * `ctx` - Active UI rendering context.
pub fn render_media_browser_table(
    frame: &mut Frame,
    area: Rect,
    items: &[DiscoveredMediaItem],
    ctx: &UiContext,
) {
    let header_cells = [
        " ",
        "SEL",
        "MÉDIA / RELEASE NAS",
        "TAILLE",
        "DURÉE",
        "EMPREINTE PHASH",
        "SOUS-TITRES",
        "ENCODAGE",
        "STATUT",
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

    let available_media_width = area
        .width
        .saturating_sub(MEDIA_BROWSER_NON_STEM_COLUMNS_WIDTH)
        .max(MIN_STEM_COLUMN_WIDTH as u16) as usize;

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let row_ctx = MediaRowContext {
                is_cursor: idx == ctx.media_scroll_index,
                is_checked: ctx.selected_media_paths.contains(&item.path),
                sources: &ctx.telemetry.media_sources,
                available_media_width,
            };
            build_media_browser_row(item, &row_ctx)
        })
        .collect();

    let mut table_state = TableState::default();
    table_state.select(Some(ctx.media_scroll_index));

    let table = Table::new(rows, MEDIA_BROWSER_COLUMN_WIDTHS)
        .header(header)
        .row_highlight_style(Style::default());

    frame.render_stateful_widget(table, area, &mut table_state);
}

/// Formats a single discovered media item into a styled Ratatui table row.
///
/// # Arguments
/// * `item` - Media catalog item reference.
/// * `ctx` - Row rendering context (cursor, selection, sources, column width).
///
/// # Returns
/// Styled `Row` with selection checkbox, source badge, title, size, duration, pHash, subtitles, and codec.
pub fn build_media_browser_row(item: &DiscoveredMediaItem, ctx: &MediaRowContext) -> Row<'static> {
    let cursor = if ctx.is_cursor { "► " } else { "  " };
    let cursor_span = Span::styled(
        cursor,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let sel_span = if ctx.is_checked {
        Span::styled(
            "[✓]",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("[ ]", Style::default().fg(Color::DarkGray))
    };

    let source_badge = if let Some(ref sid) = item.source_id {
        let short_name = ctx
            .sources
            .iter()
            .find(|s| &s.id == sid)
            .map(|s| {
                let clean = s
                    .name
                    .trim_start_matches("Synology NAS (")
                    .trim_end_matches(')')
                    .trim();
                if clean.is_empty() {
                    s.id.as_str()
                } else {
                    clean
                }
            })
            .unwrap_or(sid.as_str());
        format!("[{short_name}] ")
    } else {
        String::new()
    };

    let max_chars = ctx
        .available_media_width
        .saturating_sub(source_badge.chars().count());
    let mut media_display = item.stem.clone();
    if media_display.chars().count() > max_chars && max_chars > TRUNCATION_SUFFIX_LEN {
        media_display = media_display
            .chars()
            .take(max_chars - TRUNCATION_SUFFIX_LEN)
            .collect::<String>()
            + "...";
    }

    let media_cell_content = if !source_badge.is_empty() {
        Line::from(vec![
            Span::styled(
                source_badge,
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(media_display),
        ])
    } else {
        Line::from(media_display)
    };

    let size_str = if item.size_bytes > 0 {
        if item.size_bytes >= BYTES_PER_GIB {
            format!("{:.1} GiB", item.size_bytes as f64 / BYTES_PER_GIB_F64)
        } else {
            format!("{:.0} MiB", item.size_bytes as f64 / BYTES_PER_MIB_F64)
        }
    } else {
        "-".to_string()
    };

    let dur_str = item
        .duration_secs
        .map(format_duration)
        .unwrap_or_else(|| "-".to_string());

    let phash_span = if let Some(ref phi) = item.phash {
        if phi.exists && phi.frame_count > 0 {
            let k_frames = phi.frame_count as f64 / THOUSAND_FRAMES_F64;
            let k_bytes = phi.size_bytes as f64 / BYTES_PER_KIB_F64;
            Span::styled(
                format!("✓ {:>4.1}kf ({:>3.0}K)", k_frames, k_bytes),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("- Non calculé", Style::default().fg(Color::DarkGray))
        }
    } else {
        Span::styled("- Non calculé", Style::default().fg(Color::DarkGray))
    };

    let subs_str = if item.subtitles.is_empty() {
        "0 (Aucun)".to_string()
    } else {
        let langs: Vec<&str> = item
            .subtitles
            .iter()
            .map(|s| s.lang.as_str())
            .take(MAX_SUMMARY_LANG_TAGS)
            .collect();
        format!("{} ({})", item.subtitles.len(), langs.join(","))
    };

    let codec_str = item.video_codec.as_deref().unwrap_or("-");
    let status_span = ingestion_status_badge(item.status);

    let row_cells = vec![
        Cell::from(cursor_span),
        Cell::from(sel_span),
        Cell::from(media_cell_content),
        Cell::from(size_str),
        Cell::from(dur_str),
        Cell::from(phash_span),
        Cell::from(subs_str),
        Cell::from(codec_str.to_string()),
        Cell::from(status_span),
    ];

    let row_style = if ctx.is_cursor {
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Row::new(row_cells).style(row_style)
}
