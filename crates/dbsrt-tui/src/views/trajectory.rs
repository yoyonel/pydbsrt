//! Full-frame view page for the Spatiotemporal Trajectory Tracking Audit dashboard (`[7/t]`).

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::{
    constants::{TRAJECTORY_FILE_MODAL_HEIGHT_LINES, TRAJECTORY_FILE_MODAL_WIDTH_PERCENT},
    model::{
        compute_trajectory_indices, format_duration, TrajectoryAuditItem, TrajectoryCutItem,
        TrajectoryFileModalState, TrajectoryFilter,
    },
    ui::UiContext,
    views::widgets::centered_rect,
};

/// Truncation suffix length for names.
const TRUNCATION_SUFFIX_LEN: usize = 3;
/// Target fixed width for the visual timeline sparkline ribbon.
const TIMELINE_SPARKLINE_WIDTH: usize = 20;

/// Column layout constraints for the trajectory audit table.
const TRAJECTORY_COLUMN_WIDTHS: [Constraint; 8] = [
    Constraint::Length(6),
    Constraint::Percentage(34),
    Constraint::Length(9),
    Constraint::Length(7),
    Constraint::Length(22),
    Constraint::Length(8),
    Constraint::Length(14),
    Constraint::Length(8),
];

/// Renders the fullframe interactive spatiotemporal trajectory tracking dashboard.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target bounding rectangle for the trajectory view.
/// * `ctx` - Active UI rendering context.
///
/// # Data Flow & Invariants
/// Filters and sorts trajectory audit records by active filter, sort criteria, and fuzzy query.
/// Renders top KPI toolbar, active file selector badge, and interactive table.
pub fn render_trajectory_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let fallback_indices;
    let indices: &[usize] = if !ctx.trajectory_indices.is_empty() {
        ctx.trajectory_indices
    } else if !ctx.telemetry.trajectory_records.is_empty() {
        fallback_indices = compute_trajectory_indices(
            &ctx.telemetry.trajectory_records,
            ctx.trajectory_filter,
            ctx.trajectory_sort,
            if ctx.search_query.is_empty() {
                None
            } else {
                Some(ctx.search_query)
            },
        );
        &fallback_indices
    } else {
        &[]
    };

    let total_count = ctx.telemetry.trajectory_records.len();
    let _visible_count = indices.len();
    let matched_count = ctx
        .telemetry
        .trajectory_records
        .iter()
        .filter(|r| r.consensus_media_id.is_some())
        .count();
    let match_pct = if total_count > 0 {
        (matched_count as f64 / total_count as f64) * 100.0
    } else {
        0.0
    };

    let active_file = ctx
        .telemetry
        .active_trajectory_file
        .as_deref()
        .unwrap_or("trajectory_overnight.jsonl");

    let title = format!(
        " 🎯 AUDIT DES TRAJECTOIRES SPATIOTEMPORELLES [{matched_count}/{total_count} RECONNUS ({:.1}%)] ── FICHIER: {active_file} ",
        match_pct
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(255, 140, 0)))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(255, 140, 0))
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sub_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    render_trajectory_toolbar(frame, sub_chunks[0], ctx);

    if indices.is_empty() {
        render_empty_trajectory_placeholder(frame, sub_chunks[1], ctx.search_query);
    } else {
        render_trajectory_table(frame, sub_chunks[1], ctx, indices);
    }
}

/// Renders the action toolbar with active filter, sort order, search query, and campaign selector.
fn render_trajectory_toolbar(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    if ctx.search_active {
        let spans = vec![
            Span::styled(
                " Recherche: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}_", ctx.search_query),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                "[Entrée] Valider   [Échap] Annuler",
                Style::default().fg(Color::DarkGray),
            ),
        ];
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let filter_color = match ctx.trajectory_filter {
        TrajectoryFilter::All => Color::Yellow,
        TrajectoryFilter::Matched => Color::Green,
        TrajectoryFilter::Coherent => Color::Cyan,
        TrajectoryFilter::Drift => Color::Rgb(255, 165, 0),
        TrajectoryFilter::Pruned => Color::DarkGray,
    };

    let mut bar_spans = vec![
        Span::styled(
            " [↑/↓] Défiler  [Enter] Détail",
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("   "),
        Span::styled("[f] Filtre: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", ctx.trajectory_filter.label()),
            Style::default()
                .fg(filter_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("[o] Tri: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", ctx.trajectory_sort.label()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("[F] Fichiers: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", ctx.telemetry.trajectory_files.len()),
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
    ];

    if !ctx.search_query.is_empty() {
        bar_spans.push(Span::styled(
            "Recherche: ",
            Style::default().fg(Color::DarkGray),
        ));
        bar_spans.push(Span::styled(
            format!("\"{}\"", ctx.search_query),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        bar_spans.push(Span::raw(" "));
        bar_spans.push(Span::styled(
            "[Échap] Effacer",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        bar_spans.push(Span::styled(
            "[/] Chercher",
            Style::default().fg(Color::DarkGray),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(bar_spans)), area);
}

/// Renders the scrollable interactive table of evaluated media trajectories.
fn render_trajectory_table(frame: &mut Frame, area: Rect, ctx: &UiContext, indices: &[usize]) {
    let header_cells = [
        Cell::from("#").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("MÉDIA CIBLE / TITRE ÉLU").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("DURÉE").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("SCORE").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("TIMELINE (SPARKLINE)").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("CUTS").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("ALIGNEMENT").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("TEMPS").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let total = indices.len();
    let viewport = (area.height as usize).saturating_sub(2).max(1);
    let start = ctx.trajectory_table_offset.min(total.saturating_sub(1));
    let end = (start + viewport).min(total);

    let rows: Vec<Row> = (start..end)
        .map(|row_idx| {
            let record_idx = indices[row_idx];
            let is_selected = row_idx == ctx.trajectory_scroll_index;
            build_trajectory_row(
                row_idx + 1,
                &ctx.telemetry.trajectory_records[record_idx],
                is_selected,
            )
        })
        .collect();

    let table = Table::new(rows, TRAJECTORY_COLUMN_WIDTHS)
        .header(header)
        .block(Block::default());

    let mut state = TableState::default();
    state.select(Some(ctx.trajectory_scroll_index.saturating_sub(start)));
    frame.render_stateful_widget(table, area, &mut state);
}

/// Builds an individual table row for a trajectory item with dynamic sparkline.
fn build_trajectory_row(num: usize, item: &TrajectoryAuditItem, is_selected: bool) -> Row<'static> {
    let title_display = if item.consensus_media_id.is_some() {
        if item.consensus_title != "NONE" {
            item.consensus_title.clone()
        } else {
            item.target_media.clone()
        }
    } else {
        item.target_media.clone()
    };

    let sparkline_line = build_cuts_sparkline(item);

    let (status_text, status_style) = if item.consensus_media_id.is_none() {
        ("⚪ Élagué", Style::default().fg(Color::DarkGray))
    } else if item.is_temporally_coherent {
        (
            "✅ Cohérent",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "⚠️ Drift",
            Style::default()
                .fg(Color::Rgb(255, 165, 0))
                .add_modifier(Modifier::BOLD),
        )
    };

    let avg_score: f64 = if item.matched_cuts_count > 0 {
        let sum: f64 = item.cuts.iter().map(|c| c.winner_score).sum();
        sum / item.matched_cuts_count as f64
    } else {
        0.0
    };

    let cuts_ratio = format!("{}/{}", item.matched_cuts_count, item.cuts_count);
    let duration_str = format_duration(item.duration_sec);
    let time_str = format!("{:.2}s", item.total_elapsed_seconds);

    let row_style = if is_selected {
        Style::default().bg(Color::Rgb(40, 40, 60))
    } else {
        Style::default()
    };

    let cursor = if is_selected { "► " } else { "  " };

    let cells = vec![
        Cell::from(format!("{cursor}{num:03}")).style(if is_selected {
            Style::default()
                .fg(Color::Rgb(255, 140, 0))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        }),
        Cell::from(truncate_str(&title_display, 38)).style(if item.consensus_media_id.is_some() {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        }),
        Cell::from(duration_str).style(Style::default().fg(Color::Cyan)),
        Cell::from(format!("{avg_score:>5.1}")).style(Style::default().fg(Color::Yellow)),
        Cell::from(sparkline_line),
        Cell::from(cuts_ratio).style(Style::default().fg(Color::LightBlue)),
        Cell::from(status_text).style(status_style),
        Cell::from(time_str).style(Style::default().fg(Color::DarkGray)),
    ];

    Row::new(cells).style(row_style).height(1)
}

/// Maps cut outcome to sparkline character and style.
fn cut_sparkline_symbol(
    cut: &TrajectoryCutItem,
    consensus_id: Option<i64>,
    coherent: bool,
) -> (&'static str, Style) {
    if let Some(winner_id) = cut.winner_media_id {
        if Some(winner_id) == consensus_id {
            if coherent {
                ("■", Style::default().fg(Color::Green))
            } else {
                ("▲", Style::default().fg(Color::Rgb(255, 165, 0)))
            }
        } else {
            ("x", Style::default().fg(Color::Red))
        }
    } else {
        ("·", Style::default().fg(Color::DarkGray))
    }
}

/// Builds a 20-character ASCII sparkline ribbon visualising cut statuses across the timeline.
fn build_cuts_sparkline(item: &TrajectoryAuditItem) -> Line<'static> {
    if item.cuts.is_empty() {
        return Line::from(Span::styled(
            "[....................]",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let mut spans = Vec::with_capacity(item.cuts.len() + 2);
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));

    for cut in &item.cuts {
        let (ch, style) =
            cut_sparkline_symbol(cut, item.consensus_media_id, item.is_temporally_coherent);
        spans.push(Span::styled(ch, style));
    }

    // Pad if fewer than target width
    if item.cuts.len() < TIMELINE_SPARKLINE_WIDTH {
        let pad = TIMELINE_SPARKLINE_WIDTH - item.cuts.len();
        spans.push(Span::styled(
            "─".repeat(pad),
            Style::default().fg(Color::Rgb(60, 60, 60)),
        ));
    }

    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    Line::from(spans)
}

/// Truncates string with ellipsis if exceeding max characters.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let kept = max_len.saturating_sub(TRUNCATION_SUFFIX_LEN);
        let prefix: String = s.chars().take(kept).collect();
        format!("{prefix}...")
    }
}

/// Placeholder displayed when no records match filter or search query.
fn render_empty_trajectory_placeholder(frame: &mut Frame, area: Rect, query: &str) {
    let msg = if query.is_empty() {
        "Aucun enregistrement d'audit de trajectoire trouvé. Lancez une campagne avec 'task o2switch:trajectory:start'."
    } else {
        "Aucun média ne correspond à la recherche floue active."
    };
    let p = Paragraph::new(Span::styled(msg, Style::default().fg(Color::DarkGray)));
    frame.render_widget(p, area);
}

/// Column layout constraints for the audit file selection dialog table.
const TRAJECTORY_FILE_MODAL_COLUMN_WIDTHS: [Constraint; 5] = [
    Constraint::Length(4),
    Constraint::Length(8),
    Constraint::Min(26),
    Constraint::Length(9),
    Constraint::Length(14),
];

/// Formats byte length into human-readable KiB or MiB string.
fn format_file_size(len: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if len >= MIB {
        format!("{:.1} Mo", len as f64 / MIB as f64)
    } else if len >= KIB {
        format!("{:.1} Ko", len as f64 / KIB as f64)
    } else {
        format!("{len} B")
    }
}

/// Formats file modification system time into a compact date-time string.
fn format_file_mtime(mtime: Option<std::time::SystemTime>) -> String {
    let Some(time) = mtime else {
        return "-".to_string();
    };
    let Ok(dur) = time.duration_since(std::time::UNIX_EPOCH) else {
        return "-".to_string();
    };
    let total_secs = dur.as_secs();
    let days = total_secs / 86400;
    let rem = total_secs % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let secs = rem % 60;
    format!("D+{days} {hours:02}:{mins:02}:{secs:02}")
}

/// Builds an individual table row for an audit file entry in the modal selection dialog.
fn build_trajectory_file_row(
    num: usize,
    file_path: &str,
    is_active: bool,
    is_selected: bool,
) -> Row<'static> {
    let cursor = if is_selected { "► " } else { "  " };
    let status_cell = if is_active {
        Cell::from(Span::styled(
            "[ACTIF]",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Cell::from("      ")
    };

    let meta = std::fs::metadata(file_path).ok();
    let size_str = meta
        .as_ref()
        .map(|m| format_file_size(m.len()))
        .unwrap_or_else(|| "-".to_string());
    let mtime_str = format_file_mtime(meta.and_then(|m| m.modified().ok()));

    let mut row_style = Style::default().fg(Color::White);
    if is_selected {
        row_style = row_style
            .bg(Color::Rgb(40, 40, 50))
            .add_modifier(Modifier::BOLD);
    }

    let display_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(file_path);

    Row::new(vec![
        Cell::from(format!("{cursor}{num}")),
        status_cell,
        Cell::from(display_name.to_string()),
        Cell::from(size_str),
        Cell::from(mtime_str),
    ])
    .style(row_style)
}

/// Renders the interactive modal dialog for selecting and switching trajectory audit files.
pub fn render_trajectory_file_modal(
    frame: &mut Frame,
    area: Rect,
    ctx: &UiContext,
    modal: &TrajectoryFileModalState,
) {
    let popup_area = centered_rect(
        TRAJECTORY_FILE_MODAL_WIDTH_PERCENT,
        TRAJECTORY_FILE_MODAL_HEIGHT_LINES,
        area,
    );
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(255, 140, 0)))
        .title(Span::styled(
            " 📁 SÉLECTION DU FICHIER D'AUDIT TRAJECTOIRE ",
            Style::default()
                .fg(Color::Rgb(255, 140, 0))
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let search_line = Line::from(vec![
        Span::styled(
            " Recherche : ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(&modal.search_query, Style::default().fg(Color::White)),
        Span::styled("█", Style::default().fg(Color::Yellow)),
    ]);
    frame.render_widget(Paragraph::new(search_line), chunks[0]);

    let filtered = modal.filter_files(&ctx.telemetry.trajectory_files);
    let total = filtered.len();

    let header_cells = ["#", "STATUT", "FICHIER / CHEMIN", "TAILLE", "MODIFIÉ LE"]
        .iter()
        .map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let viewport = (chunks[1].height as usize).saturating_sub(2).max(1);
    let start = modal.scroll_offset.min(total.saturating_sub(1));
    let end = (start + viewport).min(total);

    let rows: Vec<Row> = (start..end)
        .map(|row_idx| {
            let f = filtered[row_idx];
            let is_active = ctx.telemetry.active_trajectory_file.as_deref() == Some(f.as_str());
            let is_selected = row_idx == modal.selected_index;
            build_trajectory_file_row(row_idx + 1, f, is_active, is_selected)
        })
        .collect();

    let table = Table::new(rows, TRAJECTORY_FILE_MODAL_COLUMN_WIDTHS)
        .header(header)
        .block(Block::default());
    let mut state = TableState::default();
    state.select(Some(modal.selected_index.saturating_sub(start)));
    frame.render_stateful_widget(table, chunks[1], &mut state);

    let footer_line = Line::from(vec![Span::styled(
        " [↑/↓] Défiler • [Enter] Charger • [Esc] Annuler • [Taper texte] Filtrer",
        Style::default().fg(Color::DarkGray),
    )]);
    frame.render_widget(Paragraph::new(footer_line), chunks[2]);
}
