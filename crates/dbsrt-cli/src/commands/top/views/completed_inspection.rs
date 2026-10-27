//! View page rendering for the detailed historical treatment inspection and error diagnosis.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::commands::top::{
    constants::BYTES_PER_MIB_F64,
    model::{
        filter_and_sort_completed_with_search, format_duration, CompletedStatus, HistoricalRecord,
    },
    ui::UiContext,
};

/// Candidate filesystem directories searched for worker execution logs.
const CANDIDATE_LOG_DIRS: [&str; 3] = ["data/logs", "/tmp", "data/reports"];

/// Maximum number of log tail lines displayed in the inspection panel.
const MAX_LOG_TAIL_LINES: usize = 10;

/// Number of initial stem characters used to match associated log files.
const LOG_SEARCH_STEM_CHARS: usize = 25;

/// Conversion factor from Megabytes to Gibibytes.
const MEGABYTES_PER_GIBIBYTE: f64 = 1024.0;

/// Renders the fullframe detailed inspection and error diagnosis for a selected historical record.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `ctx` - Active UI rendering context containing selection index and records.
///
/// # Data Flow & Invariants
/// Filters historical records using the active filter/sort/search criteria, retrieves the selected
/// record at `completed_scroll_index`, and renders structured diagnostics.
pub fn render_completed_inspection_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
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

    lines.extend(build_inspection_identity_lines(r));
    lines.push(Line::from(""));
    lines.extend(build_inspection_diagnosis_banner(r));
    lines.extend(build_inspection_log_search_lines(r));

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

/// Formats the identity, media path, status, and telemetry execution metrics lines.
///
/// # Arguments
/// * `r` - Historical record reference.
///
/// # Returns
/// Vector of formatted Ratatui `Line` elements detailing media identity and timing.
pub fn build_inspection_identity_lines(r: &HistoricalRecord) -> Vec<Line<'static>> {
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

    let mut lines = vec![Line::from(vec![
        Span::styled(
            "  Média / Release  : ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            r.video_stem.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    if let Some(ref path) = r.video_rel_path {
        lines.push(Line::from(vec![
            Span::styled(
                "  Chemin Complet   : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(path.clone(), Style::default().fg(Color::Cyan)),
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
    let start_str = r.start_iso.clone().unwrap_or_else(|| "-".to_string());
    let end_str = r.end_iso.clone().unwrap_or_else(|| "-".to_string());

    lines.push(Line::from(vec![
        Span::styled(
            "  Taille Fichier   : ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!(
                "{size_mb:.0} MB ({:.2} GiB)",
                size_mb / MEGABYTES_PER_GIBIBYTE
            ),
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

    lines
}

/// Formats the error summary, cancellation alert, or success certification banner.
///
/// # Arguments
/// * `r` - Historical record reference.
///
/// # Returns
/// Vector of formatted Ratatui `Line` elements detailing errors or confirmation banners.
pub fn build_inspection_diagnosis_banner(r: &HistoricalRecord) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

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

    lines
}

/// Discovers and formats log search results from standard log paths.
///
/// # Arguments
/// * `r` - Historical record reference.
///
/// # Returns
/// Vector of formatted Ratatui `Line` elements displaying tail lines of discovered log files.
pub fn build_inspection_log_search_lines(r: &HistoricalRecord) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "  📁 RECHERCHE DE LOGS D'EXÉCUTION ASSOCIÉS :",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];

    if let Some((log_path, tail_lines)) = search_associated_logs(&r.video_stem) {
        lines.push(Line::from(Span::styled(
            format!("     Log trouvé : {log_path}"),
            Style::default().fg(Color::LightGreen),
        )));
        for l in tail_lines {
            lines.push(Line::from(Span::styled(
                format!("       {l}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "     Les logs consolidés de chaque batch sont archivés dans data/reports/batch_tvshow_nightly.jsonl.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

/// Reads and returns up to `MAX_LOG_TAIL_LINES` trailing lines from a file path.
///
/// # Arguments
/// * `path` - Filesystem path to the log file.
///
/// # Returns
/// Vector of line strings in chronological order. Empty vector if reading fails.
pub fn read_log_tail_lines(path: &std::path::Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .rev()
        .take(MAX_LOG_TAIL_LINES)
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Searches candidate filesystem directories for a log file matching the video stem.
///
/// # Arguments
/// * `video_stem` - Cleaned media title stem.
///
/// # Returns
/// `Some((path_str, tail_lines))` if a matching log file was found and read, `None` otherwise.
pub fn search_associated_logs(video_stem: &str) -> Option<(String, Vec<String>)> {
    let stem_clean = video_stem
        .chars()
        .take(LOG_SEARCH_STEM_CHARS)
        .collect::<String>();

    for dir in CANDIDATE_LOG_DIRS {
        let p = std::path::Path::new(dir);
        if !p.exists() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(p) else {
            continue;
        };

        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.contains(&stem_clean) && fname.ends_with(".log") {
                let tail_lines = read_log_tail_lines(&entry.path());
                return Some((entry.path().display().to_string(), tail_lines));
            }
        }
    }

    None
}
