//! Bottom status bar, navigation tabs, and contextual keybindings for `dbsrt top`.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::commands::top::model::ViewPage;
use crate::commands::top::ui::UiContext;

/// Renders the bottom status and keybinding bar.
///
/// # Arguments
/// * `frame` - Current Ratatui terminal frame buffer.
/// * `area` - Target rectangle area allocated for the footer banner.
/// * `ctx` - Active UI rendering context.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// 1. Emits feedback banner if a user notification message is pending.
/// 2. Assembles tab bar spans with colored background on active tab.
/// 3. Appends contextual shortcut keybindings based on current view and search state.
pub fn render_footer(frame: &mut Frame, area: Rect, ctx: &UiContext) {
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

    let mut footer_spans = build_tab_bar_spans(ctx.view);
    footer_spans.push(Span::raw(" │ "));

    let shortcuts = build_shortcuts_text(ctx);
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

/// Builds navigation tab badge spans with distinct colors for the active view.
fn build_tab_bar_spans(current_view: ViewPage) -> Vec<Span<'static>> {
    let tab_jobs_style = if current_view == ViewPage::ActiveJobs {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_stats_style = if current_view == ViewPage::StatsHistory {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_recents_style = if current_view == ViewPage::CompletedMedia {
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_media_style =
        if current_view == ViewPage::MediaBrowser || current_view == ViewPage::MediaInspection {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

    let tab_sources_style = if current_view == ViewPage::MediaSources {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let tab_system_style = if current_view == ViewPage::System {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    vec![
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
        Span::raw(" "),
        Span::styled(" [6/y] Système ", tab_system_style),
    ]
}

/// Returns contextual keyboard shortcuts string based on the active tab and search query state.
fn build_shortcuts_text(ctx: &UiContext) -> &'static str {
    match ctx.view {
        ViewPage::ActiveJobs => build_active_jobs_shortcuts(ctx),
        ViewPage::StatsHistory => "[↑/k] Défiler  [?] Aide  [q] Quitter",
        ViewPage::CompletedMedia => build_completed_media_shortcuts(ctx),
        ViewPage::CompletedInspection => {
            "[Esc/Backspace/Enter] Retour Liste  [?] Aide  [q] Quitter"
        }
        ViewPage::MediaBrowser => build_media_browser_shortcuts(ctx),
        ViewPage::MediaSources => {
            "[↑/k] Défiler  [a] Ajouter  [e] Éditer  [Space] On/Off  [d] Suppr  [r] Rescan  [?] Aide  [q] Quitter"
        }
        ViewPage::System => "[↑/k] Défiler  [?] Aide  [q] Quitter",
        ViewPage::MediaInspection => {
            "[Esc/Backspace/Enter] Retour Catalogue  [Space] Cocher  [b] Batch  [q] Quitter"
        }
        ViewPage::JobInspection => {
            "[Esc/Backspace] Retour Liste  [p] Pause  [c] Arrêter  [K] Kill  [x/X] Arrêter Batch  [q] Quitter"
        }
        ViewPage::Help => {
            "[0-6/Tab] Onglet Aide  [↑/↓/k/j] Défiler  [Esc/Backspace/?] Revenir  [q] Quitter"
        }
    }
}

/// Builds shortcuts line for ActiveJobs view depending on search state.
fn build_active_jobs_shortcuts(ctx: &UiContext) -> &'static str {
    if ctx.search_active {
        "[Entrée] Valider recherche  [Échap] Annuler  [Texte] Filtrer  [?] Aide"
    } else if !ctx.search_query.is_empty() {
        "[↑/k] Défiler  [/] Chercher  [Échap] Effacer recherche  [Enter/i] Inspecter  [p] Pause  [c] Stop  [x/X] Arrêter Batch  [?] Aide  [q] Quitter"
    } else {
        "[↑/k] Défiler  [/] Chercher  [Enter/i] Inspecter  [p] Pause  [c] Stop  [x/X] Arrêter Batch  [?] Aide  [q] Quitter"
    }
}

/// Builds shortcuts line for CompletedMedia view depending on search state.
fn build_completed_media_shortcuts(ctx: &UiContext) -> &'static str {
    if ctx.search_active {
        "[Entrée] Valider recherche  [Échap] Annuler  [Texte] Filtrer  [?] Aide"
    } else if !ctx.search_query.is_empty() {
        "[↑/k] Défiler  [/] Chercher  [Échap] Effacer recherche  [Enter/i] Fiche/Erreur  [f] Filtre  [o] Tri  [?] Aide  [q] Quitter"
    } else {
        "[↑/k] Défiler  [/] Chercher  [Enter/i] Fiche/Erreur  [f] Filtre  [o] Tri  [?] Aide  [q] Quitter"
    }
}

/// Builds shortcuts line for MediaBrowser view depending on search and sources state.
fn build_media_browser_shortcuts(ctx: &UiContext) -> &'static str {
    if ctx.search_active {
        "[Entrée] Valider recherche  [Échap] Annuler  [Texte] Filtrer  [?] Aide"
    } else if !ctx.search_query.is_empty() {
        "[↑/k] Défiler  [/] Chercher  [Échap] Effacer  [Enter/i] Fiche  [Space] Cocher  [a] Tout  [f] Filtre  [o] Tri  [b] Batch  [?] Aide  [q] Quitter"
    } else {
        let active_sources = ctx
            .telemetry
            .media_sources
            .iter()
            .filter(|s| s.enabled)
            .count();
        if active_sources > 1 {
            "[↑/k] Défiler  [/] Chercher  [Enter/i] Fiche  [Space] Cocher  [a] Tout  [f] Filtre  [S] Source  [o] Tri  [b] Batch  [x/X] Arrêter Batch  [?] Aide  [q] Quitter"
        } else {
            "[↑/k] Défiler  [/] Chercher  [Enter/i] Fiche  [Space] Cocher  [a] Tout  [f] Filtre  [o] Tri  [b] Batch  [x/X] Arrêter Batch  [?] Aide  [q] Quitter"
        }
    }
}
