//! Full-frame inspection and detailed diagnostic view of multi-cuts spatiotemporal alignment.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::{
    model::{compute_trajectory_indices, format_duration, TrajectoryAuditItem, TrajectoryCutItem},
    ui::UiContext,
};

/// Column layout constraints for the cut-by-cut inspection table.
const CUT_INSPECTION_COLUMN_WIDTHS: [Constraint; 8] = [
    Constraint::Length(5),
    Constraint::Length(14),
    Constraint::Length(16),
    Constraint::Length(14),
    Constraint::Length(14),
    Constraint::Length(14),
    Constraint::Length(10),
    Constraint::Length(16),
];

/// Renders the fullframe drill-down inspection view for the selected media trajectory.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target bounding rectangle.
/// * `ctx` - Active UI rendering context.
///
/// # Data Flow & Invariants
/// Extracts selected trajectory item using active sort/filter and scroll index.
/// Splits display into summary diagnostics header and scrollable cut detail table.
pub fn render_trajectory_inspection_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
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

    if indices.is_empty() {
        return;
    }

    let selected_idx = ctx
        .trajectory_scroll_index
        .min(indices.len().saturating_sub(1));
    let item_idx = indices[selected_idx];
    let item = &ctx.telemetry.trajectory_records[item_idx];

    let title = format!(
        " 🔍 INSPECTION TRAJECTOIRE SPATIOTEMPORELLE : {} ── [{}/{}] ",
        item.target_media,
        selected_idx + 1,
        indices.len()
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
        .constraints([Constraint::Length(5), Constraint::Min(4)])
        .split(inner);

    render_inspection_summary_banner(frame, sub_chunks[0], item);
    render_inspection_cuts_table(frame, sub_chunks[1], item, ctx.trajectory_inspection_scroll);
}

/// Renders the top metadata banner with consensus, alignment metrics, and duration.
fn render_inspection_summary_banner(frame: &mut Frame, area: Rect, item: &TrajectoryAuditItem) {
    let consensus_title = if let Some(id) = item.consensus_media_id {
        format!("ID #{id} : {}", item.consensus_title)
    } else {
        "AUCUN (Consensus non atteint)".to_string()
    };

    let (coherence_tag, coherence_color) = if item.consensus_media_id.is_none() {
        ("⚪ ÉLAGUÉ", Color::DarkGray)
    } else if item.is_temporally_coherent {
        ("✅ ALIGNEMENT MONOTONE COHÉRENT", Color::Green)
    } else {
        ("⚠️ DÉVIATION TEMPORELLE / DRIFT", Color::Rgb(255, 165, 0))
    };

    let duration_str = format_duration(item.duration_sec);
    let cuts_ratio = format!(
        "{}/{} cuts concordants",
        item.matched_cuts_count, item.cuts_count
    );

    let line1 = Line::from(vec![
        Span::styled("  Média Cible     : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &item.target_media,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("    "),
        Span::styled("Durée: ", Style::default().fg(Color::DarkGray)),
        Span::styled(duration_str, Style::default().fg(Color::Cyan)),
        Span::raw("    "),
        Span::styled("Temps Inférence: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:.2}s", item.total_elapsed_seconds),
            Style::default().fg(Color::Yellow),
        ),
    ]);

    let line2 = Line::from(vec![
        Span::styled("  Consensus Élu   : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            consensus_title,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("    "),
        Span::styled("Ratio: ", Style::default().fg(Color::DarkGray)),
        Span::styled(cuts_ratio, Style::default().fg(Color::LightBlue)),
    ]);

    let line3 = Line::from(vec![
        Span::styled("  Statut Global   : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            coherence_tag,
            Style::default()
                .fg(coherence_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let p = Paragraph::new(vec![line1, line2, line3]).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::Rgb(60, 60, 80))),
    );
    frame.render_widget(p, area);
}

/// Renders the cut-by-cut inspection table.
fn render_inspection_cuts_table(
    frame: &mut Frame,
    area: Rect,
    item: &TrajectoryAuditItem,
    scroll_idx: usize,
) {
    let header_cells = [
        Cell::from("CUT").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("OFFSET CIBLE").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("REF t₀ (ms)").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("DÉRIVE").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("CANDIDATS t₀").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("FRAMES MATCH").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("SCORE").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("STATUT").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows: Vec<Row> = item
        .cuts
        .iter()
        .enumerate()
        .map(|(idx, cut)| build_cut_inspection_row(idx + 1, cut, item))
        .collect();

    let table = Table::new(rows, CUT_INSPECTION_COLUMN_WIDTHS)
        .header(header)
        .block(Block::default());

    let mut state = TableState::default();
    state.select(Some(scroll_idx.min(item.cuts.len().saturating_sub(1))));
    frame.render_stateful_widget(table, area, &mut state);
}

/// Builds an individual table row for a single cut window inspection.
fn build_cut_inspection_row(
    num: usize,
    cut: &TrajectoryCutItem,
    parent: &TrajectoryAuditItem,
) -> Row<'static> {
    let offset_target_str = format_duration(cut.start_sec);
    let ref_t0_str = if cut.winner_initial_timestamp_ms > 0 {
        format!("{} ms", cut.winner_initial_timestamp_ms)
    } else {
        "0 ms".to_string()
    };

    let delta_ms = if cut.winner_initial_timestamp_ms > 0 {
        let expected_ref_ms = (cut.start_sec * 1000.0) as i64;
        let diff = cut.winner_initial_timestamp_ms - expected_ref_ms;
        if diff >= 0 {
            format!("+{diff} ms")
        } else {
            format!("{diff} ms")
        }
    } else {
        "—".to_string()
    };

    let (status_text, status_color) = if let Some(winner_id) = cut.winner_media_id {
        if Some(winner_id) == parent.consensus_media_id {
            ("✅ ALIGNÉ", Color::Green)
        } else {
            ("❌ CONFLIT", Color::Red)
        }
    } else {
        ("⚪ SILENCE", Color::DarkGray)
    };

    let match_ratio_str = format!(
        "{}/{} f ({:.0}%)",
        cut.winner_matched_frames,
        cut.evaluated_frames_count,
        cut.winner_coverage_ratio * 100.0
    );

    let cells = vec![
        Cell::from(format!("#{num:02}")).style(Style::default().fg(Color::DarkGray)),
        Cell::from(offset_target_str).style(Style::default().fg(Color::Cyan)),
        Cell::from(ref_t0_str).style(Style::default().fg(Color::LightBlue)),
        Cell::from(delta_ms).style(Style::default().fg(Color::Yellow)),
        Cell::from(format!("{}", cut.initial_candidates_count))
            .style(Style::default().fg(Color::White)),
        Cell::from(match_ratio_str).style(Style::default().fg(Color::LightGreen)),
        Cell::from(format!("{:.2}", cut.winner_score)).style(Style::default().fg(Color::Yellow)),
        Cell::from(status_text).style(
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    Row::new(cells).height(1)
}
