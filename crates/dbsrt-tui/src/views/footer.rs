//! Bottom status bar, navigation tabs, and contextual keybindings for `dbsrt top`.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::KeyCode;
use crate::model::ViewPage;
use crate::ui::UiContext;

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
/// 3. Appends contextual shortcut keybindings based on current view and search state,
///    except on narrow screens where numbered tab cells take the whole row.
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

    let mut footer_spans = build_tab_bar_spans(ctx.view, area.width);
    if !is_compact(area.width) {
        footer_spans.push(Span::raw(" │ "));
        footer_spans.push(Span::styled(
            build_shortcuts_text(ctx),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
    }

    lines.push(Line::from(footer_spans));

    let p = Paragraph::new(lines);
    frame.render_widget(p, area);
}

/// Navigation tab: full label, the key it stands for, and its background when active.
struct Tab {
    label: &'static str,
    key: char,
    color: Color,
}

const TABS: [Tab; 6] = [
    Tab {
        label: " [1] Tâches ",
        key: '1',
        color: Color::Cyan,
    },
    Tab {
        label: " [2/h] Stats ",
        key: '2',
        color: Color::Green,
    },
    Tab {
        label: " [3/r] Récents ",
        key: '3',
        color: Color::LightBlue,
    },
    Tab {
        label: " [4/m] Médias ",
        key: '4',
        color: Color::Cyan,
    },
    Tab {
        label: " [5/s] Sources ",
        key: '5',
        color: Color::Yellow,
    },
    Tab {
        label: " [6/y] Système ",
        key: '6',
        color: Color::Magenta,
    },
];

/// Returns the tab key highlighted for a view (`None` on inspection and help pages).
fn active_tab_key(view: ViewPage) -> Option<char> {
    match view {
        ViewPage::ActiveJobs => Some('1'),
        ViewPage::StatsHistory => Some('2'),
        ViewPage::CompletedMedia => Some('3'),
        ViewPage::MediaBrowser | ViewPage::MediaInspection => Some('4'),
        ViewPage::MediaSources => Some('5'),
        ViewPage::System => Some('6'),
        ViewPage::JobInspection | ViewPage::CompletedInspection | ViewPage::Help => None,
    }
}

/// Width of the full tab bar (each label preceded by one space).
fn full_tab_bar_width() -> u16 {
    TABS.iter()
        .map(|tab| 1 + Span::raw(tab.label).width() as u16)
        .sum()
}

/// Narrow screens (phones) get one equal-width numbered cell per tab and no shortcuts.
fn is_compact(width: u16) -> bool {
    width < full_tab_bar_width()
}

/// Builds navigation tab badge spans with distinct colors for the active view.
fn build_tab_bar_spans(current_view: ViewPage, width: u16) -> Vec<Span<'static>> {
    let active = active_tab_key(current_view);
    let style = |tab: &Tab| {
        if active == Some(tab.key) {
            Style::default()
                .fg(Color::Black)
                .bg(tab.color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    };
    if is_compact(width) {
        let slot = usize::from(width) / TABS.len();
        return TABS
            .iter()
            .map(|tab| Span::styled(format!("{:^slot$}", tab.key), style(tab)))
            .collect();
    }
    TABS.iter()
        .flat_map(|tab| [Span::raw(" "), Span::styled(tab.label, style(tab))])
        .collect()
}

/// Tappable tab cells of the footer `area`, in the geometry [`render_footer`] draws them.
///
/// # Data Flow & Invariants
/// Tabs sit on the footer's second row; each cell maps to the key that switches to its tab.
pub fn footer_tab_hits(area: Rect) -> Vec<(Rect, KeyCode)> {
    if area.height < 2 {
        return Vec::new();
    }
    let row = area.y + 1;
    if is_compact(area.width) {
        let slot = area.width / TABS.len() as u16;
        return TABS
            .iter()
            .zip(0..)
            .map(|(tab, i)| {
                (
                    Rect::new(area.x + i * slot, row, slot, 1),
                    KeyCode::Char(tab.key),
                )
            })
            .collect();
    }
    let mut x = area.x;
    TABS.iter()
        .map(|tab| {
            let width = Span::raw(tab.label).width() as u16;
            x += 1;
            let zone = Rect::new(x, row, width, 1);
            x += width;
            (zone, KeyCode::Char(tab.key))
        })
        .collect()
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
            if ctx.telemetry.is_remote_streaming {
                "[4/m] Cibles Batch  [r] Actualiser  [?] Aide  [q] Quitter  (Stockage Local Verrouillé)"
            } else {
                "[↑/k] Défiler  [a] Ajouter  [e] Éditer  [Space] On/Off  [d] Suppr  [r] Rescan  [?] Aide  [q] Quitter"
            }
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
