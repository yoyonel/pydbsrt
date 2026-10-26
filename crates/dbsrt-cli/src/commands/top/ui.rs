//! Full-frame UI rendering for the dbsrt top process monitor.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, BYTES_PER_MIB_F64, FOOTER_BLOCK_HEIGHT, GPU_HIGH_LOAD_THRESHOLD_PCT,
    GPU_MODERATE_LOAD_THRESHOLD_PCT, GPU_PROGRESS_BAR_CHAR_WIDTH, HEADER_BLOCK_HEIGHT,
    MAX_COMPLETED_MEDIA_NAME_DISPLAY_CHARS, MAX_MEDIA_NAME_DISPLAY_CHARS, MAX_SENSIBLE_RATE_MIB_S,
    MIN_BODY_HEIGHT, MIN_SESSION_DURATION_DISPLAY_SECS, MIN_WALL_DURATION_NON_CACHE_SECS,
};
use crate::commands::top::model::{
    default_progress_bar, format_duration, format_progress_bar, SystemTelemetry, ViewPage,
};

/// UI rendering application context.
pub struct UiContext<'a> {
    pub telemetry: &'a SystemTelemetry,
    pub view: ViewPage,
    pub selected_index: usize,
    pub completed_scroll_index: usize,
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

    // Line 1: Clock, Workers, CPU, RAM
    let line1 = Line::from(vec![
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
    ]);

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
    let records = &ctx.telemetry.history_records;
    let title = format!(
        " 📋 DERNIERS TRAITEMENTS COMPLÉTÉS & CERTIFIÉS [{} RÉFÉRENCÉS] ",
        records.len()
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

    if records.is_empty() {
        let empty_msg = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Aucun enregistrement d'ingestion historique ou coupling certifié trouvé.",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Les rapports sont automatiquement indexés depuis data/reports/batch_tvshow_nightly.jsonl.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let p = Paragraph::new(empty_msg)
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
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

    // Records displayed reversed: most recent completions first
    let reversed_records: Vec<&crate::commands::top::model::HistoricalRecord> =
        records.iter().rev().collect();

    let rows: Vec<Row> = reversed_records
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

            let is_cache_or_dry = r.wall_duration_secs < MIN_WALL_DURATION_NON_CACHE_SECS
                || r.cifs_read_rate_mib_s > MAX_SENSIBLE_RATE_MIB_S;

            let status_cell = if !r.success {
                Span::styled("✗ FAIL", Style::default().fg(Color::Red))
            } else if is_cache_or_dry {
                Span::styled("✓ CACHE", Style::default().fg(Color::Yellow))
            } else {
                Span::styled(
                    "✓ OK",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
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

            let dur_str = if is_cache_or_dry {
                "< 1s".to_string()
            } else {
                format_duration(r.wall_duration_secs)
            };

            let rate_str = if is_cache_or_dry {
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

    let table = Table::new(rows, widths).header(header).block(block);

    let mut state = TableState::default();
    state.select(Some(ctx.completed_scroll_index));
    frame.render_stateful_widget(table, area, &mut state);
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
        Line::from(Span::styled("Navigation & Vues Full-Frame :", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from("  [1]              : Basculer vers l'onglet TÂCHES EN COURS"),
        Line::from("  [2] ou [h]       : Basculer vers l'onglet HISTORIQUE & STATISTIQUES RÉELLES"),
        Line::from("  [3] ou [r]       : Basculer vers l'onglet DERNIERS TRAITEMENTS COMPLÉTÉS"),
        Line::from("  [Tab]            : Naviguer circulairement entre les onglets"),
        Line::from("  [↑] ou [k]       : Sélectionner / Défiler vers le haut"),
        Line::from("  [↓] ou [j]       : Sélectionner / Défiler vers le bas"),
        Line::from("  [PageUp/Down]    : Défiler de 10 lignes vers le haut / bas"),
        Line::from("  [Home] / [End]   : Aller au premier / dernier élément de la liste"),
        Line::from("  [Enter] ou [l]   : Inspection Détaillée de la tâche (dans l'onglet Tâches)"),
        Line::from("  [Esc] / [Back]   : Revenir à la vue Tâches"),
        Line::from("  [?]              : Afficher / Masquer cette page d'Aide"),
        Line::from("  [q]              : Quitter le moniteur TUI"),
        Line::from(""),
        Line::from(Span::styled("Actions sur le Processus Sélectionné (Onglet 1) :", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("  [p]              : Mettre en pause ou reprendre le processus (SIGSTOP / SIGCONT)"),
        Line::from("  [s]              : Arrêter proprement le processus (SIGTERM)"),
        Line::from("  [K] (en inspect) : Tuer immédiatement le processus (SIGKILL)"),
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

    let mut footer_spans = vec![
        Span::raw(" "),
        Span::styled(" [1] Tâches ", tab_jobs_style),
        Span::raw(" "),
        Span::styled(" [2/h] Stats ", tab_stats_style),
        Span::raw(" "),
        Span::styled(" [3/r] Récents ", tab_recents_style),
        Span::raw(" │ "),
    ];

    let shortcuts = match ctx.view {
        ViewPage::ActiveJobs => {
            "[↑/k] Haut  [↓/j] Bas  [PgUp/Dn] Saut  [Home/End] Début/Fin  [p] Pause  [s] Stop  [Enter/l] Inspecter  [?] Aide  [q] Quitter"
        }
        ViewPage::StatsHistory => {
            "[Esc/1] Tâches  [3/r] Récents  [?] Aide  [q] Quitter"
        }
        ViewPage::CompletedMedia => {
            "[↑/k] Défiler  [PgUp/Dn] Saut  [Home/End] Début/Fin  [Esc/1] Tâches  [2/h] Stats  [?] Aide  [q] Quitter"
        }
        ViewPage::JobInspection => {
            "[Esc/Backspace] Retour Liste  [p] Pause  [s] Arrêter  [K] Kill  [q] Quitter"
        }
        ViewPage::Help => "[Esc/Backspace/?] Revenir aux Tâches  [q] Quitter",
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
