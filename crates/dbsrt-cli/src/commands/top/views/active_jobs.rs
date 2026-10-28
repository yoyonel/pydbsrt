//! Active decoding jobs monitor and deep job inspection view for `dbsrt top`.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::commands::top::constants::{BYTES_PER_MIB_F64, MAX_MEDIA_NAME_DISPLAY_CHARS};
use crate::commands::top::model::{
    default_progress_bar, filter_active_jobs_with_search, format_duration, ActiveJobTelemetry,
};
use crate::commands::top::ui::UiContext;

/// Renders the fullframe table of active decoding/pHash jobs.
///
/// # Arguments
/// * `frame` - Current Ratatui terminal frame buffer.
/// * `area` - Target rectangle area allocated for the body.
/// * `ctx` - Active UI rendering context.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// 1. Filters active jobs according to search query.
/// 2. If no jobs match, renders empty placeholder.
/// 3. Otherwise renders search bar followed by stateful active jobs table.
pub fn render_active_jobs_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let jobs = filter_active_jobs_with_search(
        &ctx.telemetry.active_jobs,
        if ctx.search_query.is_empty() {
            None
        } else {
            Some(ctx.search_query)
        },
    );

    let total_count = ctx.telemetry.active_jobs.len();
    let visible_count = jobs.len();
    let title =
        format!(" TÂCHES EN COURS (DÉCODAGE & PHASH TEMPS RÉEL) [{visible_count}/{total_count}] ");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sub_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    let bar_p = Paragraph::new(build_search_and_actions_bar(ctx));
    frame.render_widget(bar_p, sub_chunks[0]);

    if jobs.is_empty() {
        render_empty_jobs_placeholder(frame, sub_chunks[1], ctx.search_query);
        return;
    }

    let rows: Vec<Row> = jobs
        .iter()
        .enumerate()
        .map(|(idx, job)| build_job_row(job, idx, ctx.selected_index))
        .collect();

    let widths = [
        Constraint::Length(5),      // Cursor + Status
        Constraint::Length(8),      // PID
        Constraint::Percentage(32), // Media name
        Constraint::Length(18),     // Engine
        Constraint::Length(34),     // Progress bar + MB
        Constraint::Length(13),     // CIFS Rate
        Constraint::Length(10),     // ETA
        Constraint::Length(8),      // CPU
        Constraint::Length(11),     // RAM
    ];

    let header_cells = [
        " ",
        "PID",
        "MÉDIA",
        "MOTEUR",
        "PROGRESSION",
        "DÉBIT VFS",
        "ETA",
        "CPU",
        "RAM",
    ]
    .iter()
    .map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let table = Table::new(rows, widths).header(header);

    let mut state = TableState::default();
    state.select(Some(ctx.selected_index));
    frame.render_stateful_widget(table, sub_chunks[1], &mut state);
}

/// Builds the search bar and shortcut hint Line atop the active jobs list.
fn build_search_and_actions_bar(ctx: &UiContext) -> Line<'static> {
    let mut bar_spans = Vec::new();
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
            " [/] Chercher",
            Style::default().fg(Color::DarkGray),
        ));
    }

    bar_spans.push(Span::raw("   "));
    bar_spans.push(Span::styled(
        "[Enter/i] Inspecter   [p] Pause   [c] Stop   [x/X] Arrêter Batch",
        Style::default().fg(Color::DarkGray),
    ));

    Line::from(bar_spans)
}

/// Renders a centered explanatory placeholder when zero jobs are running or match search query.
fn render_empty_jobs_placeholder(frame: &mut Frame, area: Rect, search_query: &str) {
    let empty_msg =
        if !search_query.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "Aucune tâche ne correspond à la recherche \"{}\".",
                        search_query
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Appuyez sur [Échap] pour effacer la recherche.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        } else {
            vec![
            Line::from(""),
            Line::from(Span::styled(
                "Aucune tâche de décodage ou pHash active détectée.",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Pour lancer un traitement de masse avec monitoring en direct :",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  task media:batch:tvshow -- --workers 4 --limit 4 --fresh-only",
                Style::default().fg(Color::Green),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Le moniteur détecte automatiquement ffmpeg (GPU VA-API/CPU) et dbsrt media:add.",
                Style::default().fg(Color::DarkGray),
            )),
        ]
        };

    let p = Paragraph::new(empty_msg).alignment(Alignment::Center);
    frame.render_widget(p, area);
}

/// Builds a single table row representing an active job with colored metrics.
fn build_job_row(job: &ActiveJobTelemetry, idx: usize, selected_index: usize) -> Row<'static> {
    let is_selected = idx == selected_index;
    let status_icon = if job.status == "PAUSED" {
        Span::styled("⏸ ", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("▶ ", Style::default().fg(Color::Green))
    };

    let cursor = if is_selected { "► " } else { "  " };
    let cursor_span = Span::styled(
        cursor,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let mut media_display = job.media_name.clone();
    if media_display.chars().count() > MAX_MEDIA_NAME_DISPLAY_CHARS {
        media_display = media_display
            .chars()
            .take(MAX_MEDIA_NAME_DISPLAY_CHARS - 3)
            .collect::<String>()
            + "...";
    }

    let engine_style = if job.engine.contains("GPU") {
        Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD)
    } else if job.engine.contains("Sub") {
        Style::default().fg(Color::Magenta)
    } else if job.engine.contains("Hi10P") {
        Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::LightYellow)
    };

    let prog_bar = default_progress_bar(job.progress_ratio);
    let mb_read = job.read_bytes as f64 / BYTES_PER_MIB_F64;
    let mb_total = job.total_bytes as f64 / BYTES_PER_MIB_F64;
    let prog_text = if mb_read < 10.0 && mb_read > 0.0 {
        format!("{prog_bar} ({mb_read:.1}/{mb_total:.0} MB)")
    } else {
        format!("{prog_bar} ({mb_read:.0}/{mb_total:.0} MB)")
    };

    let eta_text = job
        .eta_secs
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());

    let row_cells = vec![
        Cell::from(Line::from(vec![cursor_span, status_icon])),
        Cell::from(job.pid.to_string()),
        Cell::from(media_display),
        Cell::from(Span::styled(job.engine.clone(), engine_style)),
        Cell::from(prog_text),
        Cell::from(format!("{:>5.1} MB/s", job.current_rate_mib_s)),
        Cell::from(format!("{eta_text:>7}")),
        Cell::from(format!("{:>4.1}%", job.cpu_percent)),
        Cell::from(format!("{:>5.1} MB", job.ram_rss_mib)),
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

/// Renders the fullframe deep inspection view for the selected active job.
///
/// # Arguments
/// * `frame` - Current Ratatui terminal frame buffer.
/// * `area` - Target rectangle area allocated for inspection.
/// * `ctx` - Active UI rendering context.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// Renders full inspection metadata lines or fallback if no job is selected.
pub fn render_inspection_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let jobs = filter_active_jobs_with_search(
        &ctx.telemetry.active_jobs,
        if ctx.search_query.is_empty() {
            None
        } else {
            Some(ctx.search_query)
        },
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(Span::styled(
            " INSPECTION DÉTAILLÉE DU PROCESSUS (FULL-FRAME) ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));

    if jobs.is_empty() || ctx.selected_index >= jobs.len() {
        let p = Paragraph::new("Aucun processus sélectionné pour l'inspection.")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
        return;
    }

    let job = &jobs[ctx.selected_index];
    let details = crate::commands::top::process_ctl::inspect_job_details(job);

    let lines: Vec<Line> = details.into_iter().map(Line::from).collect();
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(p, area);
}
