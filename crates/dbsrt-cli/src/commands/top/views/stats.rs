//! Statistical and historical decoding analytics view for `dbsrt top`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::commands::top::constants::MIN_FULL_DECODE_DURATION_SECS;
use crate::commands::top::model::SystemTelemetry;
use crate::commands::top::ui::UiContext;

/// Renders the fullframe descriptive statistical analysis of historical decoding performance.
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
/// 1. Assembles performance metrics table (throughput, duration, size, certified couplings).
/// 2. Displays explanatory legend and metadata sampling box.
pub fn render_stats_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let stats = &ctx.telemetry.stats;
    let title = format!(
        " 📊 HISTORIQUE & STATISTIQUES RÉELLES (DÉCODAGE INTÉGRAL PHASH - {} MÉDIAS) ",
        stats.sample_count
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // Stats table
            Constraint::Min(4),    // Information & legend box
        ])
        .split(area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));

    let header_cells = [
        "Dimension Analysée",
        "Min",
        "Médiane",
        "Moyenne",
        "P90 (90e pct)",
        "Max",
        "Écart-type (σ)",
    ]
    .iter()
    .map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = build_stats_rows(ctx.telemetry);

    let widths = [
        Constraint::Length(32),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(15),
        Constraint::Length(12),
        Constraint::Length(15),
    ];

    let table = Table::new(rows, widths).header(header).block(block);
    frame.render_widget(table, chunks[0]);

    let info_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Paramètres de Métrologie & Échantillonnage ",
            Style::default().fg(Color::Cyan),
        ));

    let lines = build_stats_info_lines(stats.sample_count);
    let p = Paragraph::new(lines).block(info_block);
    frame.render_widget(p, chunks[1]);
}

/// Builds statistical dimension rows for rate, duration, size, and couplings.
fn build_stats_rows(tel: &SystemTelemetry) -> Vec<Row<'static>> {
    let stats = &tel.stats;
    vec![
        Row::new(vec![
            Cell::from("Débit Décodage pHash (MB/s)").style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(format!("{:>7.2}", stats.rate.min)),
            Cell::from(format!("{:>7.2}", stats.rate.median)),
            Cell::from(format!("{:>7.2}", stats.rate.mean)),
            Cell::from(format!("{:>7.2}", stats.rate.p90)),
            Cell::from(format!("{:>7.2}", stats.rate.max)),
            Cell::from(format!("{:>7.2}", stats.rate.stdev)),
        ]),
        Row::new(vec![
            Cell::from("Durée Décodage (minutes)").style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(format!("{:>7.1}", stats.duration_mins.min)),
            Cell::from(format!("{:>7.1}", stats.duration_mins.median)),
            Cell::from(format!("{:>7.1}", stats.duration_mins.mean)),
            Cell::from(format!("{:>7.1}", stats.duration_mins.p90)),
            Cell::from(format!("{:>7.1}", stats.duration_mins.max)),
            Cell::from(format!("{:>7.1}", stats.duration_mins.stdev)),
        ]),
        Row::new(vec![
            Cell::from("Taille Vidéo Décodée (GiB)").style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(format!("{:>7.2}", stats.size_gib.min)),
            Cell::from(format!("{:>7.2}", stats.size_gib.median)),
            Cell::from(format!("{:>7.2}", stats.size_gib.mean)),
            Cell::from(format!("{:>7.2}", stats.size_gib.p90)),
            Cell::from(format!("{:>7.2}", stats.size_gib.max)),
            Cell::from(format!("{:>7.2}", stats.size_gib.stdev)),
        ]),
        Row::new(vec![
            Cell::from("Couplings Certifiés").style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(format!("{:>7.0}", stats.couplings.min)),
            Cell::from(format!("{:>7.0}", stats.couplings.median)),
            Cell::from(format!("{:>7.1}", stats.couplings.mean)),
            Cell::from(format!("{:>7.0}", stats.couplings.p90)),
            Cell::from(format!("{:>7.0}", stats.couplings.max)),
            Cell::from(format!("{:>7.1}", stats.couplings.stdev)),
        ]),
    ]
}

/// Builds explanatory legend lines displaying data sources and sample counts.
fn build_stats_info_lines(sample_count: usize) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  • Population Échantillonnée : ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} médias décodés intégralement (durée > {:.0}s)",
                sample_count, MIN_FULL_DECODE_DURATION_SECS
            )),
        ]),
        Line::from(vec![
            Span::styled(
                "  • Sources Données           : ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("data/reports/batch_tvshow_nightly.jsonl & data/SRT/couplings.json"),
        ]),
        Line::from(vec![
            Span::styled(
                "  • Raccourcis Disponibles    : ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "[1] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Tâches en cours  "),
            Span::styled(
                "[2/h] ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Statistiques  "),
            Span::styled(
                "[3/r] ",
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Récents certifiés  "),
            Span::styled(
                "[Esc] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Retour aux Tâches"),
        ]),
    ]
}
