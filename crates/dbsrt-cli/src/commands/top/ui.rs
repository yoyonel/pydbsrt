//! Full-frame UI rendering for the dbsrt top process monitor.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

use crate::commands::top::constants::{
    BYTES_PER_MIB_F64, GPU_HIGH_LOAD_THRESHOLD_PCT, GPU_MODERATE_LOAD_THRESHOLD_PCT,
    MAX_MEDIA_NAME_DISPLAY_CHARS,
};
use crate::commands::top::model::{
    default_progress_bar, format_duration, SystemTelemetry, ViewPage,
};

/// UI rendering application context.
pub struct UiContext<'a> {
    pub telemetry: &'a SystemTelemetry,
    pub view: ViewPage,
    pub selected_index: usize,
    pub feedback_msg: Option<&'a str>,
}

/// Main entry point rendering full-frame UI onto the ratatui Frame.
pub fn render_ui(frame: &mut Frame, ctx: &UiContext) {
    let size = frame.area();

    // Divide screen: 1 line header, dynamic full-frame body, 2 lines footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Top banner
            Constraint::Min(5),    // Full-frame content
            Constraint::Length(2), // Bottom keybindings & feedback
        ])
        .split(size);

    render_header(frame, chunks[0], ctx.telemetry);

    match ctx.view {
        ViewPage::ActiveJobs => render_active_jobs_page(frame, chunks[1], ctx),
        ViewPage::JobInspection => render_inspection_page(frame, chunks[1], ctx),
        ViewPage::Help => render_help_page(frame, chunks[1]),
    }

    render_footer(frame, chunks[2], ctx);
}

/// Returns formatted current local time HH:MM:SS via POSIX localtime_r.
fn get_current_time_str() -> String {
    let mut tm = libc::tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// Renders the top real-time telemetry header banner.
fn render_header(frame: &mut Frame, area: Rect, tel: &SystemTelemetry) {
    let now = get_current_time_str();

    let workers_count = tel.active_jobs.len();
    let worker_style = if workers_count > 0 {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let mut spans = vec![
        Span::styled(
            format!(" ⏱️  {now} "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("│ "),
        Span::styled(format!("Workers Actifs: {workers_count} "), worker_style),
        Span::raw("│ "),
        Span::styled(
            format!("Host CPU: {:>2.0}% ", tel.host_cpu_pct),
            Style::default().fg(Color::White),
        ),
        Span::raw("│ "),
        Span::styled(
            format!(
                "RAM: {:>4.1}/{:>4.1} GiB ",
                tel.ram_used_gib, tel.ram_total_gib
            ),
            Style::default().fg(Color::White),
        ),
        Span::raw("│ "),
    ];

    if tel.gpu.available {
        let gpu_style = if tel.gpu.video_busy_pct > GPU_HIGH_LOAD_THRESHOLD_PCT {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if tel.gpu.video_busy_pct > GPU_MODERATE_LOAD_THRESHOLD_PCT {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(
            format!("GPU Video: {:>3.0}% ", tel.gpu.video_busy_pct),
            gpu_style,
        ));
        spans.push(Span::raw("│ "));
    }

    spans.push(Span::styled(
        format!("Net RX: {:>5.1} MB/s ", tel.net_rx_mib_s),
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw("│ "));
    spans.push(Span::styled(
        format!("VFS Total: {:>5.1} MB/s ", tel.total_rate_mib_s),
        Style::default()
            .fg(Color::LightBlue)
            .add_modifier(Modifier::BOLD),
    ));

    let paragraph = Paragraph::new(Line::from(spans));
    frame.render_widget(paragraph, area);
}

/// Renders the fullframe table of active decoding/pHash jobs.
fn render_active_jobs_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let title = format!(
        " TÂCHES EN COURS (DÉCODAGE & PHASH TEMPS RÉEL) [{}] ",
        ctx.telemetry.active_jobs.len()
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    if ctx.telemetry.active_jobs.is_empty() {
        let empty_msg =
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
        ];

        let p = Paragraph::new(empty_msg)
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
        return;
    }

    // Build Table Rows
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

    let rows: Vec<Row> = ctx
        .telemetry
        .active_jobs
        .iter()
        .enumerate()
        .map(|(idx, job)| {
            let is_selected = idx == ctx.selected_index;
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
        })
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

    let table = Table::new(rows, widths).header(header).block(block);

    let mut state = TableState::default();
    state.select(Some(ctx.selected_index));
    frame.render_stateful_widget(table, area, &mut state);
}

/// Renders the fullframe deep inspection view for the selected job.
fn render_inspection_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(Span::styled(
            " INSPECTION DÉTAILLÉE DU PROCESSUS (FULL-FRAME) ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));

    if ctx.telemetry.active_jobs.is_empty() || ctx.selected_index >= ctx.telemetry.active_jobs.len()
    {
        let p = Paragraph::new("Aucun processus sélectionné pour l'inspection.")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
        return;
    }

    let job = &ctx.telemetry.active_jobs[ctx.selected_index];
    let details = crate::commands::top::process_ctl::inspect_job_details(job);

    let lines: Vec<Line> = details.into_iter().map(Line::from).collect();
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(p, area);
}

/// Renders the fullframe help and keybinding cheat sheet.
fn render_help_page(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(Span::styled(
            " AIDE & COMMANDES D'EXPLOITATION (FULL-FRAME) ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));

    let content = vec![
        Line::from(""),
        Line::from(Span::styled("Navigation & Contrôle UI :", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from("  [↑] ou [k]       : Sélectionner la tâche précédente"),
        Line::from("  [↓] ou [j]       : Sélectionner la tâche suivante"),
        Line::from("  [Enter] ou [l]   : Basculer sur la page d'Inspection Complète (Full-Frame)"),
        Line::from("  [Esc] / [Back]   : Revenir à la page principale des Tâches"),
        Line::from("  [?] ou [h]       : Afficher / Masquer cette page d'Aide"),
        Line::from("  [q]              : Quitter le moniteur TUI"),
        Line::from(""),
        Line::from(Span::styled("Actions sur le Processus Sélectionné :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("  [p]              : Mettre en pause ou reprendre le processus (SIGSTOP / SIGCONT)"),
        Line::from("  [s]              : Arrêter proprement le processus (SIGTERM)"),
        Line::from("  [k] (en inspect) : Tuer immédiatement le processus (SIGKILL)"),
        Line::from(""),
        Line::from(Span::styled("Architecture de Mesure & Télémesures :", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD))),
        Line::from("  • Débit CIFS     : Échantillonnage /proc/[pid]/io (rchar) évitant le masquage SMB/pagecache"),
        Line::from("  • Accélération   : Détection matérielle Intel VA-API (/dev/dri/renderD128) vs CPU multithread"),
        Line::from("  • Réactivité     : Boucle événementielle non-bloquante < 20ms avec rendu instantané"),
    ];

    let p = Paragraph::new(content).block(block);
    frame.render_widget(p, area);
}

/// Renders the bottom status and keybinding bar.
fn render_footer(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let mut lines = Vec::new();

    if let Some(msg) = ctx.feedback_msg {
        lines.push(Line::from(vec![
            Span::styled(
                " 📢 ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                msg,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    } else {
        lines.push(Line::from(""));
    }

    let shortcuts = match ctx.view {
        ViewPage::ActiveJobs => {
            " [↑/k] Haut  [↓/j] Bas  [p] Pause/Reprendre  [s] Arrêter  [Enter/l] Inspecter  [?] Aide  [q] Quitter"
        }
        ViewPage::JobInspection => {
            " [Esc/Backspace] Retour Liste  [p] Pause/Reprendre  [s] Arrêter  [k] Kill  [q] Quitter"
        }
        ViewPage::Help => " [Esc/Backspace/?] Revenir aux Tâches  [q] Quitter",
    };

    lines.push(Line::from(Span::styled(
        shortcuts,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )));

    let p = Paragraph::new(lines);
    frame.render_widget(p, area);
}
