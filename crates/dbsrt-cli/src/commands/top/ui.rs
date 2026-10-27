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
    FOOTER_BLOCK_HEIGHT, HEADER_BLOCK_HEIGHT, HELP_TAB_HEADER_HEIGHT, MAX_SOURCE_NAME_CHARS,
    MAX_SOURCE_PATH_CHARS, MIN_BODY_HEIGHT, SOURCE_MODAL_HEIGHT_LINES, SOURCE_MODAL_WIDTH_PERCENT,
};
use crate::commands::top::model::{
    CompletedFilter, CompletedSort, HelpTab, MediaFilter, MediaSort, MediaSourceFilter,
    SourceInputModalState, SourceModalField, SourceModalMode, SystemTelemetry, ViewPage,
};
use crate::commands::top::views::active_jobs::{render_active_jobs_page, render_inspection_page};
use crate::commands::top::views::completed::render_completed_page;
use crate::commands::top::views::completed_inspection::render_completed_inspection_page;
use crate::commands::top::views::footer::render_footer;
use crate::commands::top::views::header::render_header;
use crate::commands::top::views::media_browser::render_media_browser_page;
use crate::commands::top::views::media_inspection::render_media_inspection_page;
use crate::commands::top::views::stats::render_stats_page;
use crate::commands::top::views::widgets::centered_rect;

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
    pub search_query: &'a str,
    pub search_active: bool,
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
        Line::from("    [/]            : Recherche interactive (fuzzy search) par nom de média, PID, moteur ou commande."),
        Line::from("                     En saisie : [Entrée] valide et filtre, [Échap] annule. Hors saisie : [Échap] efface le filtre."),
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
        Line::from("    [/]            : Recherche interactive (fuzzy search) par nom de vidéo, chemin, statut ou motif d'erreur."),
        Line::from("                     En saisie : [Entrée] valide et filtre, [Échap] annule. Hors saisie : [Échap] efface le filtre."),
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
        Line::from("    [/]            : Recherche interactive (fuzzy search) par nom de vidéo, chemin, codec ou source."),
        Line::from("                     En saisie : [Entrée] valide et filtre, [Échap] annule. Hors saisie : [Échap] efface le filtre."),
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
