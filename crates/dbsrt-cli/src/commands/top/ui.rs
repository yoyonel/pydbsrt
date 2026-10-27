//! Full-frame UI rendering for the dbsrt top process monitor.

use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

use crate::commands::top::constants::{
    BYTES_PER_GIB, BYTES_PER_GIB_F64, BYTES_PER_MIB_F64, FOOTER_BLOCK_HEIGHT,
    GPU_HIGH_LOAD_THRESHOLD_PCT, GPU_MODERATE_LOAD_THRESHOLD_PCT, GPU_PROGRESS_BAR_CHAR_WIDTH,
    HEADER_BLOCK_HEIGHT, HELP_TAB_HEADER_HEIGHT, MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS,
    MAX_MEDIA_NAME_DISPLAY_CHARS, MAX_SOURCE_NAME_CHARS, MAX_SOURCE_PATH_CHARS,
    MEDIA_BROWSER_NON_STEM_COLUMNS_WIDTH, MIN_BODY_HEIGHT, MIN_SESSION_DURATION_DISPLAY_SECS,
    SOURCE_MODAL_HEIGHT_LINES, SOURCE_MODAL_WIDTH_PERCENT,
};
use crate::commands::top::model::{
    default_progress_bar, filter_and_sort_completed, filter_and_sort_media_with_source,
    format_duration, format_progress_bar, CompletedFilter, CompletedSort, CompletedStatus, HelpTab,
    IngestionStatus, MediaFilter, MediaSort, MediaSourceFilter, SourceInputModalState,
    SourceModalField, SourceModalMode, SystemTelemetry, ViewPage,
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
    pub help_tab: HelpTab,
    pub help_scroll_index: usize,
    pub feedback_msg: Option<&'a str>,
}

/// Main entry point rendering full-frame UI onto the ratatui Frame.
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

/// Renders the top real-time telemetry header banner with system, GPU, and network metrics.
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

    // Line 1: Clock, Workers, CPU, RAM, Scan indicator
    let mut line1_spans = vec![
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
                "RAM: {:>4.1}/{:>4.1} GiB",
                tel.ram_used_gib, tel.ram_total_gib
            ),
            Style::default().fg(Color::White),
        ),
    ];

    if tel.is_scanning {
        line1_spans.push(Span::raw(" │ "));
        line1_spans.push(Span::styled(
            "⟳ SCAN DISQUE EN COURS...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let line1 = Line::from(line1_spans);

    // Line 2: Intel Iris Xe Hardware GPU Telemetry
    let line2 = if tel.gpu.available {
        let v_bar =
            format_progress_bar(tel.gpu.video_busy_pct / 100.0, GPU_PROGRESS_BAR_CHAR_WIDTH);
        let v_style = if tel.gpu.video_busy_pct > GPU_HIGH_LOAD_THRESHOLD_PCT {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if tel.gpu.video_busy_pct > GPU_MODERATE_LOAD_THRESHOLD_PCT {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        Line::from(vec![
            Span::styled(
                " 🎮 GPU INTEL IRIS XE : ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Décodage (Video Engine) : ",
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{:>5.1}% {v_bar} ", tel.gpu.video_busy_pct),
                v_style,
            ),
            Span::raw("│ "),
            Span::styled(
                format!("Render/3D: {:>4.1}% ", tel.gpu.render_busy_pct),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("│ "),
            Span::styled(
                format!("Fréq: {:>4.0} MHz ", tel.gpu.freq_mhz),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("│ "),
            Span::styled(
                format!("Conso: {:>4.1} W", tel.gpu.power_watts),
                Style::default().fg(Color::Magenta),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " 🎮 GPU INTEL IRIS XE : ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Non disponible (Mode CPU Multithread)",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };

    // Line 3: CIFS / Network Bandwidth & Session Throughput
    let mut line3_spans = vec![
        Span::styled(
            " ⚡ DÉBITS RÉSEAU CIFS : ",
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Instantané: ", Style::default().fg(Color::White)),
        Span::styled(
            format!("{:>5.1} MB/s ", tel.total_rate_mib_s),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "(Net RX: {:>5.1} MB/s, VFS: {:>5.1} MB/s)",
                tel.net_rx_mib_s, tel.total_rate_mib_s
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    if tel.session.stream_count > 0
        && tel.session.session_duration_secs > MIN_SESSION_DURATION_DISPLAY_SECS
    {
        let session_gib = tel.session.session_bytes as f64 / BYTES_PER_GIB_F64;
        let dur_str = format_duration(tel.session.session_duration_secs);
        line3_spans.push(Span::raw(" │ "));
        line3_spans.push(Span::styled(
            format!(
                "Débit Cumulé Global Session: {:>4.1} MB/s ({session_gib:.1} GiB en {dur_str} sur {} flux)",
                tel.session.session_rate_mib_s, tel.session.stream_count
            ),
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let line3 = Line::from(line3_spans);

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " pydbsrt top ── Moniteur de Décodage pHash & Ingestion Temps Réel ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));

    let p = Paragraph::new(vec![line1, line2, line3]).block(header_block);
    frame.render_widget(p, area);
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

/// Renders the fullframe descriptive statistical analysis of historical decoding performance.
fn render_stats_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
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

    let rows = vec![
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
    ];

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

    // Summary & explanatory legend block
    let info_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Paramètres de Métrologie & Échantillonnage ",
            Style::default().fg(Color::Cyan),
        ));

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  • Population Échantillonnée : ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} médias décodés intégralement (durée > 60s)",
                stats.sample_count
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
    ];

    let p = Paragraph::new(lines).block(info_block);
    frame.render_widget(p, chunks[1]);
}

/// Renders the fullframe scrollable list of completed and certified media releases.
fn render_completed_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let records = filter_and_sort_completed(
        &ctx.telemetry.history_records,
        ctx.completed_filter,
        ctx.completed_sort,
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

    let filter_color = match ctx.completed_filter {
        CompletedFilter::All => Color::Yellow,
        CompletedFilter::FailedOnly => Color::Red,
        CompletedFilter::SuccessOnly => Color::Green,
        CompletedFilter::NoSrtOnly => Color::Yellow,
        CompletedFilter::CancelledOnly => Color::Magenta,
    };

    let bar_spans = vec![
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
        Span::styled(
            "[Enter/i] Fiche Diagnostic & Erreur",
            Style::default().fg(Color::DarkGray),
        ),
    ];
    let bar_p = Paragraph::new(Line::from(bar_spans));
    frame.render_widget(bar_p, sub_chunks[0]);

    if records.is_empty() {
        let empty_msg = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Aucun enregistrement ne correspond au filtre actif.",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Appuyez sur [f] pour modifier le filtre ou vérifiez l'indexation de data/reports/batch_tvshow_nightly.jsonl.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let p = Paragraph::new(empty_msg).alignment(Alignment::Center);
        frame.render_widget(p, sub_chunks[1]);
        return;
    }

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
        .map(|(idx, r)| {
            let is_selected = idx == ctx.completed_scroll_index;
            let cursor = if is_selected { "► " } else { "  " };
            let cursor_span = Span::styled(
                cursor,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

            let status_cell = match r.status() {
                CompletedStatus::Ok => Span::styled(
                    "✓ OK",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                CompletedStatus::Cache => {
                    Span::styled("✓ CACHE", Style::default().fg(Color::Yellow))
                }
                CompletedStatus::NoSrt => Span::styled(
                    "⚠ NO_SRT",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                CompletedStatus::Cancelled => Span::styled(
                    "⏹ CANCEL",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                CompletedStatus::Fail => Span::styled(
                    "✗ FAIL",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
            };

            let mut media_display = r.video_stem.clone();
            if media_display.chars().count() > MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS {
                media_display = media_display
                    .chars()
                    .take(MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS - 3)
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
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Percentage(36),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(12),
        Constraint::Length(14),
        Constraint::Length(22),
    ];

    let table = Table::new(rows, widths).header(header);

    let mut state = TableState::default();
    state.select(Some(ctx.completed_scroll_index));
    frame.render_stateful_widget(table, sub_chunks[1], &mut state);
}

/// Renders the fullframe detailed inspection and error diagnosis for a selected historical record.
fn render_completed_inspection_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let records = filter_and_sort_completed(
        &ctx.telemetry.history_records,
        ctx.completed_filter,
        ctx.completed_sort,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightBlue))
        .title(Span::styled(
            " 🔍 FICHE DÉTAILLÉE DU TRAITEMENT & DIAGNOSTIC D'EXÉCUTION (FULL-FRAME) ",
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ));

    if records.is_empty() || ctx.completed_scroll_index >= records.len() {
        let p = Paragraph::new("Aucun enregistrement sélectionné pour l'inspection.")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
        return;
    }

    let r = &records[ctx.completed_scroll_index];
    let mut lines = Vec::new();

    // 1. Identité et statut
    let (status_text, status_color) = match r.status() {
        CompletedStatus::Ok => ("✓ SUCCÈS (OK - Synchronisation certifiée)", Color::Green),
        CompletedStatus::Cache => ("✓ EN CACHE (Pré-calculé ou existant)", Color::Yellow),
        CompletedStatus::NoSrt => (
            "⚠ SOUS-TITRES MANQUANTS (Aucun .srt apparié)",
            Color::Yellow,
        ),
        CompletedStatus::Cancelled => (
            "⏹ INTERROMPU PAR L'UTILISATEUR (SIGTERM/SIGKILL)",
            Color::Magenta,
        ),
        CompletedStatus::Fail => ("✗ ÉCHEC CRITIQUE (Erreur décodeur / ffmpeg)", Color::Red),
    };

    let status_span = Span::styled(
        status_text,
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    );

    lines.push(Line::from(vec![
        Span::styled(
            "  Média / Release  : ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            &r.video_stem,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    if let Some(ref path) = r.video_rel_path {
        lines.push(Line::from(vec![
            Span::styled(
                "  Chemin Complet   : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(path, Style::default().fg(Color::Cyan)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(
            "  Statut Traitement: ",
            Style::default().fg(Color::DarkGray),
        ),
        status_span,
    ]));

    let size_mb = r.file_size_bytes as f64 / BYTES_PER_MIB_F64;
    let dur_str = format_duration(r.wall_duration_secs);
    let start_str = r.start_iso.as_deref().unwrap_or("-");
    let end_str = r.end_iso.as_deref().unwrap_or("-");

    lines.push(Line::from(vec![
        Span::styled(
            "  Taille Fichier   : ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{size_mb:.0} MB ({:.2} GiB)", size_mb / 1024.0),
            Style::default().fg(Color::White),
        ),
        Span::raw(" │ "),
        Span::styled("Durée Exécution: ", Style::default().fg(Color::DarkGray)),
        Span::styled(dur_str, Style::default().fg(Color::White)),
        Span::raw(" │ "),
        Span::styled("Débit CIFS: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:>5.1} MB/s", r.cifs_read_rate_mib_s),
            Style::default().fg(Color::White),
        ),
    ]));

    lines.push(Line::from(vec![
        Span::styled(
            "  Horodatage Début : ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(start_str, Style::default().fg(Color::White)),
        Span::raw(" │ "),
        Span::styled("Horodatage Fin: ", Style::default().fg(Color::DarkGray)),
        Span::styled(end_str, Style::default().fg(Color::White)),
    ]));

    lines.push(Line::from(vec![
        Span::styled(
            "  Couplings Sous-T.: ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("+{} certifié(s)", r.certified_couplings_count),
            Style::default().fg(Color::Green),
        ),
    ]));

    lines.push(Line::from(""));

    // 2. Erreur, Annulation ou Avertissement
    if let Some(ref err) = r.error_summary {
        let (banner_title, banner_color) = match r.status() {
            CompletedStatus::NoSrt => (
                "  ⚠️ ABSENCE DE SOUS-TITRES CONCORDANTS (NON BLOQUANT) :",
                Color::Yellow,
            ),
            CompletedStatus::Cancelled => (
                "  ⏹ ANNULATION OPÉRATEUR / ARRÊT BATCH (SIGTERM/SIGKILL) :",
                Color::Magenta,
            ),
            CompletedStatus::Fail => (
                "  🚨 CAUSE DÉTAILLÉE DE L'ERREUR / CRASH SYSTÈME :",
                Color::Red,
            ),
            _ => ("  ℹ️ RAPPORT D'EXÉCUTION DU WORKER :", Color::Cyan),
        };
        lines.push(Line::from(Span::styled(
            banner_title,
            Style::default()
                .fg(banner_color)
                .add_modifier(Modifier::BOLD),
        )));
        for err_line in err.lines() {
            lines.push(Line::from(Span::styled(
                format!("     {err_line}"),
                Style::default().fg(Color::Yellow),
            )));
        }
        lines.push(Line::from(""));
    } else if !r.success {
        lines.push(Line::from(Span::styled(
            "  ⚠️ Échec sans message d'erreur explicite enregistré dans le rapport.",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    } else {
        lines.push(Line::from(Span::styled(
            "  ✨ Traitement accompli avec succès et synchronisation pHash / SRT certifiée.",
            Style::default().fg(Color::Green),
        )));
        lines.push(Line::from(""));
    }

    // 3. Recherche des logs physiques associés
    lines.push(Line::from(Span::styled(
        "  📁 RECHERCHE DE LOGS D'EXÉCUTION ASSOCIÉS :",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    let candidate_dirs = ["data/logs", "/tmp", "data/reports"];
    let stem_clean = r.video_stem.chars().take(25).collect::<String>();
    let mut found_log = false;

    for dir in &candidate_dirs {
        let p = std::path::Path::new(dir);
        if p.exists() {
            if let Ok(entries) = std::fs::read_dir(p) {
                for entry in entries.flatten() {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if fname.contains(&stem_clean) && fname.ends_with(".log") {
                        lines.push(Line::from(Span::styled(
                            format!("     Log trouvé : {}", entry.path().display()),
                            Style::default().fg(Color::LightGreen),
                        )));
                        if let Ok(content) = std::fs::read_to_string(entry.path()) {
                            let tail: Vec<&str> = content.lines().rev().take(10).collect();
                            for l in tail.into_iter().rev() {
                                lines.push(Line::from(Span::styled(
                                    format!("       {l}"),
                                    Style::default().fg(Color::DarkGray),
                                )));
                            }
                        }
                        found_log = true;
                        break;
                    }
                }
            }
        }
        if found_log {
            break;
        }
    }

    if !found_log {
        lines.push(Line::from(Span::styled(
            "     Les logs consolidés de chaque batch sont archivés dans data/reports/batch_tvshow_nightly.jsonl.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Esc / Backspace / Enter] Revenir au tableau des récents",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )));

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

/// Renders the interactive media browser allowing inspection, filtering, sorting and batch execution.
fn render_media_browser_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let items = filter_and_sort_media_with_source(
        &ctx.telemetry.discovered_media,
        ctx.media_filter,
        ctx.media_sort,
        ctx.source_filter,
        &ctx.telemetry.media_sources,
    );

    let total_count = ctx.telemetry.discovered_media.len();
    let visible_count = items.len();
    let selected_count = ctx.selected_media_paths.len();
    let active_sources = ctx
        .telemetry
        .media_sources
        .iter()
        .filter(|s| s.enabled)
        .count();
    let total_sources = ctx.telemetry.media_sources.len();

    let title = match ctx.source_filter {
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

    let mut browser_title_spans = vec![Span::styled(
        title,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    if ctx.telemetry.is_scanning {
        browser_title_spans.push(Span::styled(
            "[⟳ ACTUALISATION DU CATALOGUE EN COURS...] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Line::from(browser_title_spans));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sub_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

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
    frame.render_widget(bar_p, sub_chunks[0]);

    if items.is_empty() {
        let empty_msg = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Aucun média disponible ne correspond au filtre actif.",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Appuyez sur [f] pour modifier le filtre ou vérifiez la liaison NAS Synology.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let p = Paragraph::new(empty_msg).alignment(Alignment::Center);
        frame.render_widget(p, sub_chunks[1]);
        return;
    }

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

    let available_media_width = sub_chunks[1]
        .width
        .saturating_sub(MEDIA_BROWSER_NON_STEM_COLUMNS_WIDTH)
        .max(15) as usize;

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_cursor = idx == ctx.media_scroll_index;
            let cursor = if is_cursor { "► " } else { "  " };
            let cursor_span = Span::styled(
                cursor,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

            let is_checked = ctx.selected_media_paths.contains(&item.path);
            let sel_span = if is_checked {
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
                    .telemetry
                    .media_sources
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

            let max_chars = available_media_width.saturating_sub(source_badge.chars().count());
            let mut media_display = item.stem.clone();
            if media_display.chars().count() > max_chars && max_chars > 3 {
                media_display = media_display
                    .chars()
                    .take(max_chars - 3)
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
                    let k_frames = phi.frame_count as f64 / 1000.0;
                    let k_bytes = phi.size_bytes as f64 / 1024.0;
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
                    .take(3)
                    .collect();
                format!("{} ({})", item.subtitles.len(), langs.join(","))
            };

            let codec_str = item.video_codec.as_deref().unwrap_or("-");

            let status_span = match item.status {
                IngestionStatus::Virgin => {
                    Span::styled("VIERGE", Style::default().fg(Color::Yellow))
                }
                IngestionStatus::InProgress => Span::styled(
                    "EN COURS",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                IngestionStatus::Completed => Span::styled(
                    "COMPLÉTÉ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                IngestionStatus::PartialFail => Span::styled(
                    "HACHÉ SEUL",
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                ),
            };

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

            let row_style = if is_cursor {
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

    let mut table_state = TableState::default();
    table_state.select(Some(ctx.media_scroll_index));

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default());

    frame.render_stateful_widget(table, sub_chunks[1], &mut table_state);
}

/// Renders the full-frame diagnostic and inspection sheet for a candidate media.
fn render_media_inspection_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " 🔍 FICHE DÉTAILLÉE & DIAGNOSTIC D'INGESTION DU MÉDIA (FULL-FRAME) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let visible_media = filter_and_sort_media_with_source(
        &ctx.telemetry.discovered_media,
        ctx.media_filter,
        ctx.media_sort,
        ctx.source_filter,
        &ctx.telemetry.media_sources,
    );

    if visible_media.is_empty() || ctx.media_scroll_index >= visible_media.len() {
        let p = Paragraph::new("Aucun média sélectionné pour l'inspection.")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
        return;
    }

    let item = &visible_media[ctx.media_scroll_index];
    let size_gib = item.size_bytes as f64 / BYTES_PER_GIB_F64;
    let dur_str = item
        .duration_secs
        .map(format_duration)
        .unwrap_or_else(|| "Inconnue".to_string());
    let codec_str = item.video_codec.as_deref().unwrap_or("Inconnu");

    let is_selected = ctx.selected_media_paths.contains(&item.path);
    let sel_status = if is_selected {
        "☑ COCHÉ pour batch"
    } else {
        "☐ NON COCHÉ"
    };

    let is_accessible = item.path.exists();
    let access_span = if is_accessible {
        Span::styled(
            "✓ ACCESSIBLE (Fichier présent sur le disque)",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "✗ INACCESSIBLE (Fichier introuvable sur le disque / montage déconnecté)",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    };

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Release : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &item.stem,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let source_label = if let Some(ref sid) = item.source_id {
        let name = ctx
            .telemetry
            .media_sources
            .iter()
            .find(|s| &s.id == sid)
            .map(|s| s.name.as_str())
            .unwrap_or(sid.as_str());
        format!("{name} ({sid})")
    } else {
        "Source principale".to_string()
    };
    lines.push(Line::from(vec![
        Span::styled("  Source  : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            source_label,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    lines.push(Line::from(vec![
        Span::styled("  Chemin  : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            item.path.display().to_string(),
            Style::default().fg(Color::Cyan),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Accès Disque: ", Style::default().fg(Color::DarkGray)),
        access_span,
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Taille  : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{:.2} GiB ({} octets)", size_gib, item.size_bytes),
            Style::default().fg(Color::White),
        ),
        Span::raw("    "),
        Span::styled("Durée : ", Style::default().fg(Color::DarkGray)),
        Span::styled(dur_str, Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled("Codec Vidéo : ", Style::default().fg(Color::DarkGray)),
        Span::styled(codec_str, Style::default().fg(Color::Yellow)),
        Span::raw("    "),
        Span::styled("Sélection : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            sel_status,
            if is_selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─── PROPRIÉTÉS VIDÉO & COMPATIBILITÉ MOTEUR DÉCODAGE ─────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    if codec_str.contains("Hi10P") || codec_str.contains("High 10") {
        lines.push(Line::from(vec![
            Span::styled(
                "  Codec & Profil   : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{codec_str} (yuv420p10le / 10-bit color)"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                "  Accélération GPU : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "✗ NON COMPATIBLE (Incompatibilité ASIC Silicium)",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(
                " — Les puces GPU (Intel Iris/UHD, Nvidia, AMD) ne décodent pas le H.264 10-bit.",
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Moteur Décodage  : ", Style::default().fg(Color::DarkGray)),
            Span::styled("CPU (Software Multithread)", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" — Repli automatique et transparent sur CPU multi-cœurs (charge CPU ~800% normale)."),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(
                "  Codec & Profil   : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(codec_str, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                "  Accélération GPU : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "✓ COMPATIBLE GPU VA-API (Intel Iris Xe)",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                "  Moteur Décodage  : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("GPU (VA-API / QSV)", Style::default().fg(Color::Green)),
            Span::raw(" — Décodage matériel ASIC ultra-rapide à faible consommation CPU."),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─── EMPREINTE VISUELLE PHASH (HASHING VIDÉO DCT-II) ─────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    if let Some(ref phi) = item.phash {
        if phi.exists && phi.frame_count > 0 {
            let k_bytes = phi.size_bytes as f64 / 1024.0;
            let m_bytes = phi.size_bytes as f64 / (1024.0 * 1024.0);
            lines.push(Line::from(vec![
                Span::styled("  Statut pHash : ", Style::default().fg(Color::White)),
                Span::styled(
                    "✓ HACHAGE EFFECTUÉ (Empreinte calculée & enregistrée)",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Volume pHash : ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} frames échantillonnées", phi.frame_count),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("    "),
                Span::styled(
                    "Taille Fichier/Disque : ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!(
                        "{:.1} KiB ({:.2} MiB / {} octets)",
                        k_bytes, m_bytes, phi.size_bytes
                    ),
                    Style::default().fg(Color::White),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Stockage DB  : ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:.1} KiB en RAM / B-K-Tree (8 octets / frame)", k_bytes),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("    "),
                Span::styled("Fichier Local : ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("data/__PHASH__/{}.phash", item.stem),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                "  - AUCUN HACHAGE EFFECTUÉ (0 frame échantillonnée, aucun fichier .phash).",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  - AUCUN HACHAGE EFFECTUÉ (0 frame échantillonnée, aucun fichier .phash).",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─── PISTES DE SOUS-TITRES DÉTECTÉES ──────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    if item.subtitles.is_empty() {
        lines.push(Line::from(Span::styled(
            "  ❌ AUCUN SOUS-TITRE DÉTECTÉ (0 flux incorporé, 0 fichier .srt sidecar).",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "     ↳ Ce conteneur vidéo ne possède aucun sous-titre textuel pour établir un couplage.",
            Style::default().fg(Color::Red),
        )));
    } else {
        for (i, sub) in item.subtitles.iter().enumerate() {
            let title_info = if !sub.title.is_empty() {
                format!(" ({})", sub.title)
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  • Piste #{}: ", i + 1),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("Langue '{}' ", sub.lang),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("| Codec: {} ", sub.codec),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(title_info, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─── STATUT DU CYCLE DE VIE & DIAGNOSTIC TECHNIQUE ─────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    let status_style = match item.status {
        IngestionStatus::Virgin => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        IngestionStatus::InProgress => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        IngestionStatus::Completed => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        IngestionStatus::PartialFail => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
    };

    lines.push(Line::from(vec![
        Span::styled("  Statut Actuel : ", Style::default().fg(Color::White)),
        Span::styled(format!("[{}]", item.status.label()), status_style),
    ]));
    lines.push(Line::from(""));

    match item.status {
        IngestionStatus::Virgin => {
            lines.push(Line::from(Span::styled(
                "  ℹ️  MÉDIA VIERGE : Ce média n'a jamais été analysé ni indexé.",
                Style::default().fg(Color::White),
            )));
            if item.subtitles.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  ⚠️  ATTENTION : Média sans sous-titre. Le batch ignorera ce fichier sans sous-titres.",
                    Style::default().fg(Color::LightYellow),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "  👉 ACTION DISPONIBLE : Cochez avec [Space] et pressez [b] pour lancer l'ingestion par lot.",
                    Style::default().fg(Color::Green),
                )));
            }
        }
        IngestionStatus::InProgress => {
            lines.push(Line::from(Span::styled(
                "  ⏳ EN COURS DE TRAITEMENT : Un worker actif traite ce média (décodage pHash / extraction).",
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(Span::styled(
                "  👉 Consultez l'onglet [1/Tâches] pour suivre la progression, le débit VFS et l'ETA en direct.",
                Style::default().fg(Color::White),
            )));
        }
        IngestionStatus::Completed => {
            lines.push(Line::from(Span::styled(
                "  ✅ CERTIFICATION RÉUSSIE : L'empreinte visuelle pHash et les sous-titres sont synchronisés.",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(Span::styled(
                "  👉 Enregistrement disponible dans 'data/SRT/couplings.json' et l'onglet [3/Récents].",
                Style::default().fg(Color::White),
            )));
        }
        IngestionStatus::PartialFail => {
            lines.push(Line::from(Span::styled(
                "  ℹ️  DIAGNOSTIC DU STATUT 'HACHÉ SEUL' :",
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "     1. Empreinte pHash entièrement calculée et enregistrée dans data/__PHASH__/<stem>.phash.",
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(Span::styled(
                "     2. Le média est 100% opérationnel pour la reconnaissance visuelle directe.",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(Span::styled(
                "     3. Aucun sous-titre n'est couplé dans data/SRT/couplings.json (0 piste textuelle).",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  💡 POUR CERTIFIER UN COUPLAGE ULTÉRIEUR (OPTIONNEL) :",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "     Déposez un fichier sous-titre .srt dans le même dossier sur le NAS, ex :",
                Style::default().fg(Color::White),
            )));
            let parent_dir = item
                .path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            lines.push(Line::from(Span::styled(
                format!("     {parent_dir}/{}.fr.srt", item.stem),
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(Span::styled(
                "     Puis relancez le batch [b] : le sous-titre sera certifié instantanément (pHash déjà précalculé).",
                Style::default().fg(Color::White),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─── RÈGLES DU MOTEUR D'INGESTION PYDBSRT ──────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  • But Premier : Hachage perceptuel DCT-II des frames vidéo pour la recherche visuelle rapide.",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  • Couplage Certifié : Associe l'empreinte vidéo à une piste SRT de référence lorsque disponible.",
        Style::default().fg(Color::DarkGray),
    )));

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

/// Renders the full-frame manager for configured media library sources.
fn render_media_sources_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let active_sources = ctx
        .telemetry
        .media_sources
        .iter()
        .filter(|s| s.enabled)
        .count();
    let total_sources = ctx.telemetry.media_sources.len();
    let sources_title = if active_sources == total_sources {
        format!(" 🗄️ GESTION DES SOURCES DE MÉDIAS (RÉPERTOIRES VIDÉO) [{total_sources}] ")
    } else {
        format!(" 🗄️ GESTION DES SOURCES DE MÉDIAS (RÉPERTOIRES VIDÉO) [{active_sources}/{total_sources} ACTIVES] ")
    };

    let mut sources_title_spans = vec![Span::styled(
        sources_title,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    if ctx.telemetry.is_scanning {
        sources_title_spans.push(Span::styled(
            "[⟳ SCAN DISQUE EN COURS...] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Line::from(sources_title_spans));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sub_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    let rescan_label = if ctx.telemetry.is_scanning {
        Span::styled(
            "Scan en cours...   ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("Rescanner Tout   ", Style::default().fg(Color::White))
    };

    let bar_spans = vec![
        Span::styled(
            " [a] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Ajouter Source   ", Style::default().fg(Color::White)),
        Span::styled(
            " [e] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Éditer   ", Style::default().fg(Color::White)),
        Span::styled(
            " [Space] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Activer/Désactiver   ", Style::default().fg(Color::White)),
        Span::styled(
            " [d] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("Supprimer   ", Style::default().fg(Color::White)),
        Span::styled(
            " [r] ",
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ),
        rescan_label,
        Span::styled(
            " [4/m] ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Voir Médias Détectés", Style::default().fg(Color::White)),
    ];
    let bar_p = Paragraph::new(Line::from(bar_spans));
    frame.render_widget(bar_p, sub_chunks[0]);

    if ctx.telemetry.media_sources.is_empty() {
        let empty_msg = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Aucune source de média configurée.",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Appuyez sur [a] pour ajouter un répertoire source (local ou point de montage CIFS/NFS).",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let p = Paragraph::new(empty_msg).alignment(Alignment::Center);
        frame.render_widget(p, sub_chunks[1]);
        return;
    }

    let header_cells = [
        " ",
        "STATUT",
        "NOM DE LA SOURCE",
        "ACCÈS DISQUE",
        "SCAN DISQUE",
        "DISPONIBLES",
        "DERNIER SCAN",
        "CHEMIN DU RÉPERTOIRE",
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

    let rows: Vec<Row> = ctx
        .telemetry
        .media_sources
        .iter()
        .enumerate()
        .map(|(idx, src)| {
            let is_cursor = idx == ctx.source_scroll_index;
            let cursor = if is_cursor { "► " } else { "  " };
            let cursor_span = Span::styled(
                cursor,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

            let status_span = if src.enabled {
                Span::styled(
                    "[✓] ACTIF",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("[ ] DÉSACTIVÉ", Style::default().fg(Color::DarkGray))
            };

            let mut name_display = src.name.clone();
            if name_display.chars().count() > MAX_SOURCE_NAME_CHARS {
                name_display = name_display
                    .chars()
                    .take(MAX_SOURCE_NAME_CHARS - 3)
                    .collect::<String>()
                    + "...";
            }

            let is_accessible = src.path.exists() && src.path.is_dir();
            let access_span = if is_accessible {
                Span::styled(
                    "✓ DISPONIBLE",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    "✗ INACCESSIBLE",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )
            };

            let scan_result_span = if src.last_scan_iso.is_some() {
                if src.last_scan_added > 0 {
                    Span::styled(
                        format!("{} (+{} nouv.)", src.media_count, src.last_scan_added),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(
                        format!("{} fichier(s)", src.media_count),
                        Style::default().fg(Color::White),
                    )
                }
            } else {
                Span::styled("- (en attente)", Style::default().fg(Color::DarkGray))
            };

            let in_mem_count = ctx
                .telemetry
                .discovered_media
                .iter()
                .filter(|m| m.source_id.as_deref() == Some(&src.id))
                .count();

            let dispo_span = if !src.enabled {
                Span::styled("0 (inactif)", Style::default().fg(Color::DarkGray))
            } else if in_mem_count > 0 {
                Span::styled(
                    format!("{in_mem_count} dispo"),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("0 dispo", Style::default().fg(Color::Yellow))
            };

            let scan_str = src.last_scan_iso.as_deref().unwrap_or("En attente...");

            let mut path_display = src.path.display().to_string();
            if path_display.chars().count() > MAX_SOURCE_PATH_CHARS {
                path_display = "...".to_string()
                    + &path_display
                        .chars()
                        .skip(path_display.chars().count() - (MAX_SOURCE_PATH_CHARS - 3))
                        .collect::<String>();
            }

            let row_cells = vec![
                Cell::from(cursor_span),
                Cell::from(status_span),
                Cell::from(name_display),
                Cell::from(access_span),
                Cell::from(scan_result_span),
                Cell::from(dispo_span),
                Cell::from(scan_str.to_string()),
                Cell::from(path_display),
            ];

            let row_style = if is_cursor {
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
        Constraint::Length(3),
        Constraint::Length(15),
        Constraint::Length(26),
        Constraint::Length(16),
        Constraint::Length(20),
        Constraint::Length(16),
        Constraint::Length(22),
        Constraint::Min(25),
    ];

    let mut table_state = TableState::default();
    table_state.select(Some(ctx.source_scroll_index));

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default());

    frame.render_stateful_widget(table, sub_chunks[1], &mut table_state);
}

/// Helper to center a rect inside another rect.
fn centered_rect(percent_x: u16, height_lines: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(r.height.saturating_sub(height_lines) / 2),
            Constraint::Length(height_lines),
            Constraint::Min(0),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Renders the interactive source addition / edition popup modal.
fn render_source_modal(frame: &mut Frame, area: Rect, modal: &SourceInputModalState) {
    let popup_area = centered_rect(SOURCE_MODAL_WIDTH_PERCENT, SOURCE_MODAL_HEIGHT_LINES, area);
    frame.render_widget(Clear, popup_area);

    let title = match modal.mode {
        SourceModalMode::Add => " ➕ AJOUTER UNE NOUVELLE SOURCE DE MÉDIAS ",
        SourceModalMode::Edit(_) => " ✏️ MODIFIER LA SOURCE DE MÉDIAS ",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Help / tip
            Constraint::Length(3), // Field: Path
            Constraint::Length(3), // Field: Name
            Constraint::Min(1),    // Action shortcuts
        ])
        .split(inner);

    let tip = Paragraph::new(Line::from(vec![
        Span::styled(
            " Astuce : ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(
            "Indiquez un chemin local ou un point de montage CIFS/NFS (/media/..., /mnt/...).",
        ),
    ]));
    frame.render_widget(tip, chunks[0]);

    // Path Field
    let path_border_style = if modal.active_field == SourceModalField::Path {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let path_cursor = if modal.active_field == SourceModalField::Path {
        "█"
    } else {
        ""
    };
    let path_block = Block::default()
        .borders(Borders::ALL)
        .border_style(path_border_style)
        .title(" Chemin du Répertoire (obligatoire) ");
    let path_p = Paragraph::new(format!("{}{}", modal.path_input, path_cursor))
        .block(path_block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(path_p, chunks[1]);

    // Name Field
    let name_border_style = if modal.active_field == SourceModalField::Name {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let name_cursor = if modal.active_field == SourceModalField::Name {
        "█"
    } else {
        ""
    };
    let name_block = Block::default()
        .borders(Borders::ALL)
        .border_style(name_border_style)
        .title(" Nom d'affichage (optionnel, auto-déduit si vide) ");
    let name_p = Paragraph::new(format!("{}{}", modal.name_input, name_cursor))
        .block(name_block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(name_p, chunks[2]);

    // Shortcuts footer
    let shortcuts = Line::from(vec![
        Span::styled(
            " [Tab/↑/↓] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Changer champ   "),
        Span::styled(
            " [Enter] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Valider & Scanner   "),
        Span::styled(
            " [Esc] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw("Annuler"),
    ]);
    let footer_p = Paragraph::new(shortcuts).alignment(Alignment::Center);
    frame.render_widget(footer_p, chunks[3]);
}

/// Renders the fullframe contextual help, technical glossary, and keybinding cheat sheet.
fn render_help_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightGreen))
        .title(Span::styled(
            " 📚 AIDE CONTEXTUELLE, GLOSSAIRE TECHNIQUE & COMMANDES D'EXPLOITATION (FULL-FRAME) ",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HELP_TAB_HEADER_HEIGHT),
            Constraint::Min(5),
        ])
        .split(inner);

    // 1. Contextual Help Tab Navigation Bar
    let mut tab_spans = Vec::new();
    tab_spans.push(Span::raw(" "));
    for t in &HelpTab::ALL {
        let is_active = *t == ctx.help_tab;
        let style = if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        tab_spans.push(Span::styled(format!(" {} ", t.label()), style));
        tab_spans.push(Span::raw(" "));
    }

    let nav_hint = Line::from(vec![
        Span::styled(
            "  [0-5] ou [Tab/Shift+Tab / ←/→] Naviguer entre onglets d'aide  │  [↑/↓/k/j/PgUp/PgDn] Défiler  │  [Esc/?] Revenir à la vue précédente",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let tab_bar = Paragraph::new(vec![Line::from(tab_spans), nav_hint]);
    frame.render_widget(tab_bar, chunks[0]);

    // 2. Tab Content & Detailed Glossary
    let lines = match ctx.help_tab {
        HelpTab::General => build_help_general(),
        HelpTab::ActiveJobs => build_help_active_jobs(),
        HelpTab::StatsHistory => build_help_stats_history(),
        HelpTab::Completed => build_help_completed(),
        HelpTab::MediaBrowser => build_help_media_browser(),
        HelpTab::MediaSources => build_help_media_sources(),
    };

    let p = Paragraph::new(lines)
        .scroll((ctx.help_scroll_index as u16, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, chunks[1]);
}

fn build_help_general() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  🌐 PRÉSENTATION DE L'OUTIL & ARCHITECTURE SYSTÈME",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from("  dbsrt top est le moniteur interactif temps réel d'ingestion vidéo, de calcul d'empreintes pHash et"),
        Line::from("  de certification de synchronisation de sous-titres développé pour pydbsrt."),
        Line::from(""),
        Line::from(Span::styled("  Navigation Globale :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("    [1]            : Onglet TÂCHES EN COURS (workers actifs, décodage, pHash en direct)"),
        Line::from("    [2] ou [h]     : Onglet HISTORIQUE & STATISTIQUES RÉELLES (distributions des débits/durées)"),
        Line::from("    [3] ou [r]     : Onglet DERNIERS TRAITEMENTS COMPLÉTÉS & CERTIFIÉS (journal, statuts, diagnostics FAIL)"),
        Line::from("    [4] ou [m]     : Onglet EXPLORATEUR DE MÉDIAS MULTI-SOURCES (inventaire, sélection batch)"),
        Line::from("    [5] ou [s]     : Onglet GESTION DES SOURCES DE MÉDIAS (CRUD répertoires vidéo, rescan)"),
        Line::from("    [Tab/BackTab]  : Naviguer circulairement entre les onglets"),
        Line::from("    [?]            : Ouvrir cette aide contextuelle (positionnée automatiquement sur la vue courante)"),
        Line::from("    [q]            : Quitter le moniteur TUI"),
        Line::from(""),
        Line::from(Span::styled("  Principes Architecturaux Majeurs :", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD))),
        Line::from("    • Pas de magie sur sous-titre seul : le recalage pHash compare DEUX flux vidéo (référence vs cible)."),
        Line::from("    • Couplings certifiés : stockés dans PostgreSQL 17 et dans data/SRT/couplings.json pour établir la vérité terrain."),
        Line::from("    • Mesure CIFS intègre : lecture rchar via /proc/[pid]/io évitant les biais de buffer Linux/SMB."),
        Line::from("    • Télémétrie GPU VA-API : surveillance matérielle sur /dev/dri/renderD128 avec bascule CPU automatique."),
    ]
}

fn build_help_active_jobs() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  ⚙️ ONGLET [1] TÂCHES EN COURS (WORKERS ACTIFS)",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Affiche en direct l'ensemble des processus workers décodant des flux vidéo ou extrayant les empreintes pHash."),
        Line::from(""),
        Line::from(Span::styled("  Glossaire des Colonnes :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("    • PID          : Identifiant système Unix du processus worker ffmpeg ou dbsrt."),
        Line::from("    • MÉDIA        : Nom du fichier vidéo en cours de décodage."),
        Line::from("    • MOTEUR       : Mode de décodage actif :"),
        Line::from("                     - GPU (VA-API)    : Accélération matérielle Intel QuickSync/Iris Xe via /dev/dri/renderD128."),
        Line::from("                     - CPU Multithread : Décodage logiciel multithread (fallback automatique si échec VA-API)."),
        Line::from("    • PROGRESSION  : Jauge graphique et volumétrie d'octets lus par rapport à la taille totale du fichier."),
        Line::from("    • DÉBIT VFS    : Débit instantané de lecture sur le montage réseau ou disque local (MB/s)."),
        Line::from("    • ETA          : Estimation du temps restant avant achèvement du traitement complet."),
        Line::from("    • CPU / RAM    : Pourcentage d'utilisation processeur instantané et mémoire vive RSS consommée."),
        Line::from(""),
        Line::from(Span::styled("  Commandes & Actions :", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD))),
        Line::from("    [Enter] / [i]  : Ouvrir la fiche d'inspection détaillée du processus (I/O, threads, mémoire, logs)."),
        Line::from("    [p]            : Mettre en pause ou reprendre la tâche pointée (signaux SIGSTOP / SIGCONT)."),
        Line::from("    [s]            : Arrêter proprement la tâche pointée (signal SIGTERM)."),
        Line::from("    [K] (en inspect): Forcer l'arrêt immédiat de la tâche (signal SIGKILL)."),
        Line::from("    [x]            : Arrêter proprement TOUT le batch (coordinateur batch_media_add + tous les workers)."),
        Line::from("    [X]            : Tuer immédiatement (SIGKILL) TOUT le batch et tous les processus enfants."),
    ]
}

fn build_help_stats_history() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  📊 ONGLET [2/h] STATISTIQUES HISTORIQUES RÉELLES",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Tableau de distribution statistique descriptive calculé sur l'ensemble des décodages complets (> 60s)."),
        Line::from(""),
        Line::from(Span::styled("  Glossaire des Métriques :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("    • Débit CIFS   : Vitesse réelle de transfert réseau observée (MB/s) via /proc/[pid]/io."),
        Line::from("    • Durée        : Temps d'horloge effectif (minutes) requis pour traiter un média de bout en bout."),
        Line::from("    • Taille       : Poids physique des fichiers traités (GiB)."),
        Line::from("    • Couplings    : Nombre de pistes sous-titres certifiées et synchronisées par fichier."),
        Line::from(""),
        Line::from(Span::styled("  Colonnes Statistiques :", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD))),
        Line::from("    • Min / Max    : Valeurs extrêmes observées sur l'échantillon."),
        Line::from("    • Médiane      : Valeur centrale séparant l'échantillon en deux moitiés égales (insensible aux outliers)."),
        Line::from("    • Moyenne      : Moyenne arithmétique globale."),
        Line::from("    • P90          : 90e percentile (90% des traitements ont atteint une performance inférieure ou égale)."),
        Line::from("    • Écart-type σ : Dispersion des mesures autour de la moyenne."),
    ]
}

fn build_help_completed() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  📋 ONGLET [3/r] DERNIERS TRAITEMENTS COMPLÉTÉS & CERTIFIÉS",
            Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Journal historique et chronologique des médias traités, certifiés ou en échec."),
        Line::from(""),
        Line::from(Span::styled("  Glossaire des Colonnes :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("    • MÉDIA / ÉPISODE TRAITÉ : Nom de release ou titre du fichier vidéo indexé."),
        Line::from("    • TAILLE                 : Taille du fichier source sur disque ou partage CIFS/NFS (en MB ou GiB)."),
        Line::from("    • DURÉE                  : Temps total d'horloge (wall-clock time) mesuré pour décoder et hacher le fichier."),
        Line::from("    • DÉBIT CIFS             : Vitesse moyenne de lecture brute (MB/s) capturée au niveau de /proc/[pid]/io."),
        Line::from("    • DATE FIN               : Horodatage ISO 8601 marquant l'achèvement de la tâche."),
        Line::from(""),
        Line::from(Span::styled("    • COUPLINGS (Définition Approfondie) :", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from("      Nombre d'associations certifiées 'Flux Vidéo (pHash) ↔ Fichier Sous-Titres (.srt) synchronisé'."),
        Line::from("      - Un sous-titre seul ne contient AUCUNE empreinte visuelle. Pour synchroniser une release cible,"),
        Line::from("        pydbsrt a besoin d'un couplage de vérité terrain certifié liant les frames pHash de référence"),
        Line::from("        aux repères temporels (cues) du sous-titre."),
        Line::from("      - '+1', '+2'... indique le nombre de pistes de sous-titres synchronisées et validées en base."),
        Line::from("      - '+0' signifie qu'aucun sous-titre n'a pu être couplé (ex: absence de .srt concordant ou échec)."),
        Line::from(""),
        Line::from(Span::styled("    • STATUT (Hiérarchie Rigoureuse des Sévérités) :", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from("      ✓ OK     : [NOMINAL / SUCCÈS] Ingestion et décodage pHash achevés avec succès."),
        Line::from("                 Les empreintes visuelles sont stockées et au moins un coupling vidéo/sous-titres est certifié."),
        Line::from("      ✓ CACHE  : [NOMINAL / EN CACHE] Média déjà traité lors d'une session antérieure (durée < 1s)."),
        Line::from("                 Présent en cache local ou PostgreSQL. Aucun réencodage ni recalcul pHash redondant."),
        Line::from("      ⚠ NO_SRT : [AVERTISSEMENT FONCTIONNEL / NON-BLOQUANT] Média parfaitement sain et décodable,"),
        Line::from("                 mais aucun fichier .srt concordant n'a été trouvé pour cette release. Aucun crash système."),
        Line::from("      ⏹ CANCEL : [ANNULATION OPÉRATEUR] Tâche délibérément interrompue par l'utilisateur via [x]/[X]"),
        Line::from("                 (signaux SIGTERM ou SIGKILL) ou par le gestionnaire de processus."),
        Line::from("      ✗ FAIL   : [ERREUR FATALE SYSTÈME] Vrai crash décodeur/ffmpeg, code retour non-nul du binaire,"),
        Line::from("                 corruption de conteneur ou erreur de lecture disque/réseau."),
        Line::from(""),
        Line::from(Span::styled("  Diagnostics & Commandes :", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD))),
        Line::from("    [Enter] ou [i] : Ouvre la FICHE DIAGNOSTIC & ERREUR plein écran affichant l'explication textuelle"),
        Line::from("                     complète de l'erreur (error_summary), le chemin absolu et la fin des logs d'exécution."),
        Line::from("    [f]            : Filtrage cyclique : [TOUS] -> [SUCCÈS] -> [ÉCHECS (FAIL)] -> [SANS SOUS-TITRES (NO_SRT)] -> [ANNULÉS (CANCEL)]."),
        Line::from("    [o]            : Tri cyclique : DATE (RÉCENT), DATE (ANCIEN), STATUT (FAIL / CANCEL / NO_SRT), NOM, DURÉE, TAILLE, DÉBIT, COUPLINGS."),
    ]
}

fn build_help_media_browser() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  📁 ONGLET [4/m] EXPLORATEUR DE MÉDIAS MULTI-SOURCES",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Catalogue interactif unifié des vidéos disponibles sur l'ensemble des sources configurées et actives."),
        Line::from(""),
        Line::from(Span::styled("  Glossaire des Colonnes :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("    • SEL                    : Case à cocher pour traitement par lot ([x] coché, [ ] vierge)."),
        Line::from("    • MÉDIA / RELEASE NAS    : Nom du fichier vidéo précédé du badge source en Cyan ([video], [tvshow]) si multi-sources."),
        Line::from("    • TAILLE                 : Taille du fichier (en GiB ou MB)."),
        Line::from("    • DURÉE                  : Durée temporelle estimée du flux vidéo via métadonnées conteneur."),
        Line::from("    • EMPREINTE PHASH        : Statut du hachage perceptuel :"),
        Line::from("                               - '✓ 65.5kf (512K)' : 65 500 frames analysées et index binaire généré."),
        Line::from("                               - '- Non calculé'   : Vidéo non encore décodée pour pHash."),
        Line::from("    • SOUS-TITRES            : Nombre de flux internes intégrés au fichier conteneur et langues détectées (eng, fre...)."),
        Line::from("    • ENCODAGE               : Codec vidéo détecté (hevc, h264, mpeg4, vp9...)."),
        Line::from(""),
        Line::from(Span::styled("    • STATUT (Toutes les Valeurs Possibles) :", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from("      VIERGE     : Fichier découvert lors du scan mais JAMAIS traité. Aucune empreinte pHash ni sous-titre."),
        Line::from("                   Prêt à être coché ([Space]) pour ingestion par lot ([b])."),
        Line::from("      EN COURS   : Fichier actuellement pris en charge par un worker actif visible dans l'onglet [1] Tâches."),
        Line::from("      COMPLÉTÉ   : Média entièrement ingéré : empreinte visuelle pHash calculée et sous-titres couplés/certifiés."),
        Line::from("      HACHÉ SEUL : Empreinte pHash de la vidéo calculée et stockée, mais aucun sous-titre n'a pu être couplé."),
        Line::from(""),
        Line::from(Span::styled("  Commandes & Actions :", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD))),
        Line::from("    [Space]        : Cocher / décocher le média pointé pour le lot."),
        Line::from("    [a]            : Cocher / décocher TOUS les médias visibles selon le filtre actif."),
        Line::from("    [b]            : Lancer l'ingestion batch ciblée exclusivement sur les médias cochés."),
        Line::from("    [f]            : Filtrer par statut : [TOUS], [VIERGES SEULS], [EN COURS], [COMPLÉTÉS], [HACHÉS SEULS], [AVEC SOUS-TITRES], [SANS SOUS-TITRES]."),
        Line::from("    [S]            : Filtrer par source d'origine : cycle entre [ACTIVES], [TOUTES] et chaque source spécifique."),
        Line::from("    [o]            : Trier par critère : NOM (A-Z), TAILLE, DURÉE, STATUT, SOUS-TITRES."),
        Line::from("    [d] / [Suppr]  : Supprimer le(s) média(s) sélectionné(s) du catalogue persistant."),
        Line::from("    [Enter] / [i]  : Ouvrir la fiche d'inspection détaillée du média candidat."),
        Line::from("    [x] / [X]      : Arrêt propre (SIGTERM) ou forcé (SIGKILL) de tout le batch."),
    ]
}

fn build_help_media_sources() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  🗄️ ONGLET [5/s] GESTION DES SOURCES DE MÉDIAS",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Gestionnaire interactif CRUD des répertoires vidéo scannés en parallèle (stocké dans data/reports/media_sources.json)."),
        Line::from(""),
        Line::from(Span::styled("  Glossaire des Colonnes :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("    • STATUT                 : [✓] ACTIF (inclus dans l'indexation) ou [ ] DÉSACTIVÉ (désactivé temporairement)."),
        Line::from("    • NOM DE LA SOURCE       : Libellé convivial (auto-déduit depuis le dossier ou personnalisé)."),
        Line::from("    • ACCÈS DISQUE           : '✓ DISPONIBLE' (chemin accessible en lecture) ou '✗ INACCESSIBLE' (point de montage démonté)."),
        Line::from("    • SCAN DISQUE            : Nombre total de fichiers vidéo recensés lors du dernier scan sur disque, avec mention des nouveaux ajouts (+X nouv.)."),
        Line::from("    • DISPONIBLES            : Nombre effectif de médias de cette source chargés en mémoire vive dans le catalogue actif (onglet [4/m] Médias). Si la source est désactivée, cette valeur passe immédiatement à 0 (inactif)."),
        Line::from("    • DERNIER SCAN           : Horodatage ISO de la dernière exploration du dossier."),
        Line::from("    • CHEMIN DU RÉPERTOIRE   : Chemin absolu ou relatif sur le système de fichiers ou partage réseau CIFS/NFS."),
        Line::from(""),
        Line::from(Span::styled("  Actions CRUD & Raccourcis :", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD))),
        Line::from("    [a]            : Ajouter une nouvelle source via le formulaire modal interactif."),
        Line::from("    [e]            : Éditer le chemin ou le libellé de la source sélectionnée."),
        Line::from("    [Space]        : Activer / Désactiver la source sélectionnée pour les scans disque (catalogue persistant préservé)."),
        Line::from("    [d]            : Supprimer la source sélectionnée (sécurité : interdit la suppression de la dernière source restante)."),
        Line::from("    [r]            : Déclencher un rescan complet immédiat de l'ensemble des sources actives en tâche de fond."),
    ]
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

    let tab_jobs_style = if ctx.view == ViewPage::ActiveJobs {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_stats_style = if ctx.view == ViewPage::StatsHistory {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_recents_style = if ctx.view == ViewPage::CompletedMedia {
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_media_style =
        if ctx.view == ViewPage::MediaBrowser || ctx.view == ViewPage::MediaInspection {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

    let tab_sources_style = if ctx.view == ViewPage::MediaSources {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut footer_spans = vec![
        Span::raw(" "),
        Span::styled(" [1] Tâches ", tab_jobs_style),
        Span::raw(" "),
        Span::styled(" [2/h] Stats ", tab_stats_style),
        Span::raw(" "),
        Span::styled(" [3/r] Récents ", tab_recents_style),
        Span::raw(" "),
        Span::styled(" [4/m] Médias ", tab_media_style),
        Span::raw(" "),
        Span::styled(" [5/s] Sources ", tab_sources_style),
        Span::raw(" │ "),
    ];

    let shortcuts = match ctx.view {
        ViewPage::ActiveJobs => {
            "[↑/k] Défiler  [Enter/i] Inspecter  [p] Pause  [s] Stop  [x/X] Arrêter Batch  [?] Aide  [q] Quitter"
        }
        ViewPage::StatsHistory => "[↑/k] Défiler  [?] Aide  [q] Quitter",
        ViewPage::CompletedMedia => {
            "[↑/k] Défiler  [Enter/i] Fiche/Erreur  [f] Filtre  [o] Tri  [?] Aide  [q] Quitter"
        }
        ViewPage::CompletedInspection => {
            "[Esc/Backspace/Enter] Retour Liste  [?] Aide  [q] Quitter"
        }
        ViewPage::MediaBrowser => {
            let active_sources = ctx
                .telemetry
                .media_sources
                .iter()
                .filter(|s| s.enabled)
                .count();
            if active_sources > 1 {
                "[↑/k] Défiler  [Enter/i] Fiche  [Space] Cocher  [a] Tout  [f] Filtre  [S] Source  [o] Tri  [b] Batch  [x/X] Arrêter Batch  [?] Aide  [q] Quitter"
            } else {
                "[↑/k] Défiler  [Enter/i] Fiche  [Space] Cocher  [a] Tout  [f] Filtre  [o] Tri  [b] Batch  [x/X] Arrêter Batch  [?] Aide  [q] Quitter"
            }
        }
        ViewPage::MediaSources => {
            "[↑/k] Défiler  [a] Ajouter  [e] Éditer  [Space] On/Off  [d] Suppr  [r] Rescan  [?] Aide  [q] Quitter"
        }
        ViewPage::MediaInspection => {
            "[Esc/Backspace/Enter] Retour Catalogue  [Space] Cocher  [b] Batch  [q] Quitter"
        }
        ViewPage::JobInspection => {
            "[Esc/Backspace] Retour Liste  [p] Pause  [s] Arrêter  [K] Kill  [x/X] Arrêter Batch  [q] Quitter"
        }
        ViewPage::Help => {
            "[0-5/Tab] Onglet Aide  [↑/↓/k/j] Défiler  [Esc/Backspace/?] Revenir  [q] Quitter"
        }
    };

    footer_spans.push(Span::styled(
        shortcuts,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ));

    lines.push(Line::from(footer_spans));

    let p = Paragraph::new(lines);
    frame.render_widget(p, area);
}
