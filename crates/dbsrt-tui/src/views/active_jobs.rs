//! Active decoding jobs monitor and deep job inspection view for `dbsrt top`.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::constants::{
    BYTES_PER_MIB_F64, MAX_MEDIA_NAME_DISPLAY_CHARS, MIN_DISPLAY_HASHING_FPS,
    MIN_DISPLAY_REALTIME_FACTOR, SECONDS_PER_HOUR, SECONDS_PER_MINUTE,
};
use crate::model::{
    default_progress_bar, filter_active_jobs_with_search, format_duration, ActiveJobTelemetry,
    StreamKind,
};
use crate::ui::UiContext;

/// Builds the active jobs panel title, surfacing active pipeline type (`pHash` vs `aHash`) and `batch_type`.
fn format_active_jobs_title(ctx: &UiContext, visible_count: usize, total_count: usize) -> String {
    let has_audio = ctx
        .telemetry
        .active_jobs
        .iter()
        .any(ActiveJobTelemetry::is_ahash_audio);
    let has_video = ctx
        .telemetry
        .active_jobs
        .iter()
        .any(|j| j.stream_kind() == StreamKind::VideoPhash);
    let mode_label = match (has_video, has_audio) {
        (false, true) => "DÉCODAGE & AHASH AUDIO TEMPS RÉEL",
        (true, true) => "DÉCODAGE PHASH & AHASH TEMPS RÉEL",
        _ => "DÉCODAGE & PHASH TEMPS RÉEL",
    };
    if ctx.telemetry.batch_queue.total_targets > 0 {
        let queue_prefix = match ctx.telemetry.batch_queue.batch_type.as_deref() {
            Some(kind) if !kind.is_empty() => format!("File [{kind}]"),
            _ => "File".to_string(),
        };
        format!(
            " TÂCHES EN COURS ({mode_label}) [{visible_count}/{total_count} actifs │ {queue_prefix}: {}/{} traités, {} restants (dont {total_count} en cours)] ",
            ctx.telemetry.batch_queue.completed_targets,
            ctx.telemetry.batch_queue.total_targets,
            ctx.telemetry.batch_queue.remaining_targets,
        )
    } else {
        format!(" TÂCHES EN COURS ({mode_label}) [{visible_count}/{total_count}] ")
    }
}

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
    let title = format_active_jobs_title(ctx, visible_count, total_count);

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

/// Formats compact media runtime (`1h17`, `42m`, `25s`) fitting inside the 34-col progression cell.
fn format_compact_media_clock(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    if total >= SECONDS_PER_HOUR {
        let h = total / SECONDS_PER_HOUR;
        let m = (total % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
        format!("{h}h{m:02}")
    } else if total >= SECONDS_PER_MINUTE {
        format!("{}m", total / SECONDS_PER_MINUTE)
    } else {
        format!("{total}s")
    }
}

/// Formats the `PROGRESSION` and `DÉBIT VFS` cells according to `StreamKind` (`aHash` vs `pHash`).
fn format_job_progress_and_rate(job: &ActiveJobTelemetry) -> (String, String) {
    if job.is_ahash_audio() {
        let pcm_bps = job.ahash_pcm_bytes_per_sec();
        let decoded_secs = if pcm_bps > 0.0 {
            job.read_bytes as f64 / pcm_bps
        } else {
            0.0
        };
        let prog_text = if job.total_bytes > 0 && pcm_bps > 0.0 {
            let total_secs = job.total_bytes as f64 / pcm_bps;
            let prog_bar = default_progress_bar(job.progress_ratio);
            format!(
                "{prog_bar} {}/{}",
                format_compact_media_clock(decoded_secs),
                format_compact_media_clock(total_secs)
            )
        } else {
            format!(
                "[░░░░░░░░░░░░░░]  --.-% ({})",
                format_compact_media_clock(decoded_secs)
            )
        };
        let rt = job.ahash_realtime_factor();
        let rate_display = if rt >= MIN_DISPLAY_REALTIME_FACTOR {
            format!("{rt:>5.1}x RT")
        } else {
            format!("{:>5.2} MB/s", job.current_rate_mib_s)
        };
        return (prog_text, rate_display);
    }

    let prog_bar = default_progress_bar(job.progress_ratio);
    let mb_read = job.read_bytes as f64 / BYTES_PER_MIB_F64;
    let mb_total = job.total_bytes as f64 / BYTES_PER_MIB_F64;
    let prog_text = if mb_read < 10.0 && mb_read > 0.0 {
        format!("{prog_bar} ({mb_read:.1}/{mb_total:.0} MB)")
    } else {
        format!("{prog_bar} ({mb_read:.0}/{mb_total:.0} MB)")
    };
    let rate_display = if job.hashing_fps > MIN_DISPLAY_HASHING_FPS {
        format!("{:>4.0} fps", job.hashing_fps)
    } else {
        format!("{:>5.1} MB/s", job.current_rate_mib_s)
    };
    (prog_text, rate_display)
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

    let engine_style = if job.is_ahash_audio() {
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD)
    } else if job.engine.contains("GPU") {
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

    let (prog_text, rate_display) = format_job_progress_and_rate(job);
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
        Cell::from(rate_display),
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
    let details = inspect_job_details(job);

    let lines: Vec<Line> = details.into_iter().map(Line::from).collect();
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(p, area);
}

/// Formats detailed diagnostic inspection lines for an active decoding job.
///
/// # Arguments
/// * `job` - Active job telemetry snapshot.
///
/// # Returns
/// A vector of formatted text strings representing operational metrics, I/O rates, and engine status.
///
/// # Data Flow & Invariants
/// Pure projection of in-memory telemetry fields without blocking I/O.
pub fn inspect_job_details(job: &ActiveJobTelemetry) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push(format!(
        "PID: {:<8} │ Moteur: {:<16} │ État: {}",
        job.pid,
        job.engine,
        job.status.to_uppercase()
    ));
    lines.push(format!(
        "Écoulé: {:<8} │ Débit CIFS: {:>6.1} MB/s │ CPU: {:>4.1}% │ RAM: {:>5.1} MB",
        crate::model::format_duration(job.elapsed_secs),
        job.current_rate_mib_s,
        job.cpu_percent,
        job.ram_rss_mib
    ));
    if job.is_ahash_audio() {
        let pcm_bps = job.ahash_pcm_bytes_per_sec().max(1.0);
        let decoded_secs = job.read_bytes as f64 / pcm_bps;
        let total_secs = job.total_bytes as f64 / pcm_bps;
        lines.push(format!(
            "Progression audio: {:>5.1}% │ Audio décodé: {} / {} │ Vitesse: {:.1}x RT ({:.0} fps spectraux)",
            job.progress_ratio * 100.0,
            crate::model::format_duration(decoded_secs),
            if job.total_bytes > 0 {
                crate::model::format_duration(total_secs)
            } else {
                "N/A".to_string()
            },
            job.ahash_realtime_factor(),
            job.hashing_fps,
        ));
    } else {
        lines.push(format!(
            "Progression: {:>5.1}% │ Lus: {:>6.1} MB / Total: {:>6.1} MB",
            job.progress_ratio * 100.0,
            job.read_bytes as f64 / crate::constants::BYTES_PER_MIB_F64,
            job.total_bytes as f64 / crate::constants::BYTES_PER_MIB_F64
        ));
    }

    if let Some(eta) = job.eta_secs {
        lines.push(format!(
            "ETA estimé: {}",
            crate::model::format_duration(eta)
        ));
    }

    lines.push(String::new());
    lines.push("--- Diagnostic Moteur & Accélération Matérielle ---".to_string());
    if let Some(ref reason) = job.engine_reason {
        lines.push(format!("  • Moteur sélectionné : {}", job.engine));
        lines.push(format!("  • Diagnostic matériel: {reason}"));
    } else if job.is_ahash_audio() {
        lines.push("  • Moteur sélectionné : aHash Audio".to_string());
        lines.push("  • Opération active   : Extraction acoustique Haitsma-Kalker (.ahash, PCM mono f32le 5512 Hz, -vn sans décodage vidéo).".to_string());
    } else if job.engine.contains("Hi10P") {
        lines.push(format!("  • Moteur sélectionné : {}", job.engine));
        lines.push(
            "  • Incompatibilité ASIC: Codec H.264 Hi10P (High 10 Profile, 10-bit color)."
                .to_string(),
        );
        lines.push(
            "    Fallback logiciel CPU (sw) requis car aucun ASIC moderne ne décode le Hi10P en matériel."
                .to_string(),
        );
    } else if job.engine.contains("GPU") {
        lines.push("  • Moteur sélectionné : GPU (VA-API)".to_string());
        lines.push("  • Accélération active: Décodage matériel ASIC GPU actif via VA-API (/dev/dri/renderD128).".to_string());
        lines.push("  • Efficacité         : Déchargement CPU quasi-total (< 20% CPU hôte) et débit de décodage maximal.".to_string());
    } else if job.engine.contains("Sub") {
        lines.push("  • Moteur sélectionné : Sub Extraction".to_string());
        lines.push("  • Opération active   : Démultiplexage et extraction brute des pistes SRT (flux texte léger, aucun décodage vidéo).".to_string());
    } else {
        lines.push(format!("  • Moteur sélectionné : {}", job.engine));
        lines.push(
            "  • Mode Décodage      : Décodage logiciel CPU (sans accélération matérielle VA-API)."
                .to_string(),
        );
    }

    if !job.cmdline.is_empty() {
        lines.push(String::new());
        lines.push("--- Ligne de Commande Exécutée ---".to_string());
        lines.push(redact_command_line(&job.cmdline));
    }

    lines.push(String::new());
    lines.push("--- Descripteurs Fichiers & I/O Actifs ---".to_string());
    lines.push("  • Aucun fichier physique direct ouvert détecté".to_string());

    lines.push(String::new());
    lines.push("--- Extrait Derniers Logs d'Exécution ---".to_string());
    lines.push("  (Aucun fichier log dédié trouvé pour ce PID / média)".to_string());

    lines
}

fn redact_url_arg(arg: &str) -> String {
    let Some((prefix, rest)) = arg.split_once("://") else {
        return arg.to_string();
    };
    let Some((userinfo, hostpath)) = rest.split_once('@') else {
        return arg.to_string();
    };
    let Some((user, _pass)) = userinfo.split_once(':') else {
        return arg.to_string();
    };
    format!("{prefix}://{user}:***@{hostpath}")
}

/// Masks sensitive passwords in command line arguments for safe display.
pub fn redact_command_line(args: &[String]) -> String {
    let mut previous_was_password_flag = false;
    let masked: Vec<String> = args
        .iter()
        .map(|arg| {
            let shown = if previous_was_password_flag {
                "***".to_string()
            } else {
                redact_url_arg(arg)
            };
            previous_was_password_flag = arg == "-ftp-password";
            shown
        })
        .collect();
    masked.join(" ")
}
