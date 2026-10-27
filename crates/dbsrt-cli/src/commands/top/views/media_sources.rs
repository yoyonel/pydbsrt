//! View page rendering for media library source management and interactive modal editor.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::commands::top::{
    constants::{
        DELETE_CONFIRM_MODAL_HEIGHT_LINES, MAX_SOURCE_NAME_CHARS, MAX_SOURCE_PATH_CHARS,
        SOURCE_MODAL_HEIGHT_LINES, SOURCE_MODAL_WIDTH_PERCENT,
    },
    model::{MediaSourceConfig, SourceInputModalState, SourceModalField, SourceModalMode},
    ui::UiContext,
    views::widgets::centered_rect,
};

/// Truncation ellipsis character length.
const TRUNCATION_SUFFIX_LEN: usize = 3;

/// Table column widths for media sources grid.
const MEDIA_SOURCES_COLUMN_WIDTHS: [Constraint; 7] = [
    Constraint::Length(3),
    Constraint::Length(16),
    Constraint::Length(26),
    Constraint::Length(20),
    Constraint::Length(16),
    Constraint::Length(22),
    Constraint::Min(25),
];

/// Renders the full-frame manager for configured media library sources.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `ctx` - Active UI rendering context.
///
/// # Data Flow & Invariants
/// Displays active and configured media sources (local drives, CIFS/NFS mounts),
/// rendering operational action shortcuts and disk availability statuses.
pub fn render_media_sources_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
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

    render_media_sources_toolbar(frame, sub_chunks[0], ctx);

    if ctx.telemetry.media_sources.is_empty() {
        render_empty_media_sources_placeholder(frame, sub_chunks[1]);
    } else {
        render_media_sources_table(frame, sub_chunks[1], ctx);
    }
}

/// Renders the action toolbar for media sources (add, edit, toggle, delete, rescan).
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target bounding rectangle for toolbar.
/// * `ctx` - Active UI rendering context.
pub fn render_media_sources_toolbar(frame: &mut Frame, area: Rect, ctx: &UiContext) {
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
    frame.render_widget(bar_p, area);
}

/// Renders a centered placeholder when zero media sources are configured.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
pub fn render_empty_media_sources_placeholder(frame: &mut Frame, area: Rect) {
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
    frame.render_widget(p, area);
}

/// Renders the multi-column table of configured sources and disk access states.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `ctx` - Active UI rendering context.
pub fn render_media_sources_table(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let header_cells = [
        " ",
        "STATUT",
        "NOM DE LA SOURCE",
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
            build_media_source_row(src, is_cursor, ctx)
        })
        .collect();

    let mut table_state = TableState::default();
    table_state.select(Some(ctx.source_scroll_index));

    let table = Table::new(rows, MEDIA_SOURCES_COLUMN_WIDTHS)
        .header(header)
        .row_highlight_style(Style::default());

    frame.render_stateful_widget(table, area, &mut table_state);
}

/// Formats a single configured media source into a styled Ratatui table row.
///
/// # Arguments
/// * `src` - Media source configuration item.
/// * `is_cursor` - Whether this row holds cursor focus.
/// * `ctx` - Active UI rendering context.
///
/// # Returns
/// Ratatui `Row` detailing source status, accessibility, discovered items count, and path.
pub fn build_media_source_row(
    src: &MediaSourceConfig,
    is_cursor: bool,
    ctx: &UiContext,
) -> Row<'static> {
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
            .take(MAX_SOURCE_NAME_CHARS - TRUNCATION_SUFFIX_LEN)
            .collect::<String>()
            + "...";
    }

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
                .skip(
                    path_display.chars().count() - (MAX_SOURCE_PATH_CHARS - TRUNCATION_SUFFIX_LEN),
                )
                .collect::<String>();
    }

    let row_cells = vec![
        Cell::from(cursor_span),
        Cell::from(status_span),
        Cell::from(name_display),
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
}

/// Dispatches rendering of source configuration or deletion confirmation modal overlay.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `modal` - Modal state descriptor containing active field and user text inputs.
///
/// # Data Flow & Invariants
/// Dispatches to [`render_delete_confirm_modal`] if modal mode is [`SourceModalMode::DeleteConfirm`],
/// otherwise renders the interactive configuration form via [`render_source_input_form`].
pub fn render_source_modal(frame: &mut Frame, area: Rect, modal: &SourceInputModalState) {
    match modal.mode {
        SourceModalMode::DeleteConfirm(_) => render_delete_confirm_modal(frame, area, modal),
        SourceModalMode::Add | SourceModalMode::Edit(_) => {
            render_source_input_form(frame, area, modal)
        }
    }
}

/// Renders the modal configuration dialog for adding or editing a media source.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `modal` - Modal state descriptor containing active field, error banner, and user text inputs.
///
/// # Data Flow & Invariants
/// Centers an input dialog box, displays active field focus cursors, renders any validation
/// error banner if present, and lists navigational keyboard shortcuts.
fn render_source_input_form(frame: &mut Frame, area: Rect, modal: &SourceInputModalState) {
    let popup_area = centered_rect(SOURCE_MODAL_WIDTH_PERCENT, SOURCE_MODAL_HEIGHT_LINES, area);
    frame.render_widget(Clear, popup_area);

    let title = match modal.mode {
        SourceModalMode::Add => " ➕ AJOUTER UNE NOUVELLE SOURCE DE MÉDIAS ",
        SourceModalMode::Edit(_) => " ✏️ MODIFIER LA SOURCE DE MÉDIAS ",
        SourceModalMode::DeleteConfirm(_) => " 🗑️ CONFIRMER LA SUPPRESSION ",
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
            Constraint::Length(1), // Help / tip / error
            Constraint::Length(3), // Field: Path
            Constraint::Length(3), // Field: Name
            Constraint::Min(1),    // Action shortcuts
        ])
        .split(inner);

    let tip = if let Some(ref err) = modal.error_msg {
        Paragraph::new(Line::from(vec![
            Span::styled(
                " ❌ Erreur : ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(err, Style::default().fg(Color::LightRed)),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Astuce : ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(
                "Indiquez un chemin local ou un point de montage CIFS/NFS (/media/..., /mnt/...).",
            ),
        ]))
    };
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

/// Renders modal confirmation popup before deleting a media source.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `modal` - Modal state containing target source name, path, and active status.
///
/// # Data Flow & Invariants
/// Centers a confirmation box on screen, displays a highlighted red warning if the source
/// is currently active, and lists confirmed keybindings ([y/o/Enter] vs [n/Esc]).
fn render_delete_confirm_modal(frame: &mut Frame, area: Rect, modal: &SourceInputModalState) {
    let popup_area = centered_rect(
        SOURCE_MODAL_WIDTH_PERCENT,
        DELETE_CONFIRM_MODAL_HEIGHT_LINES,
        area,
    );
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            " 🗑️ CONFIRMER LA SUPPRESSION DE LA SOURCE ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Question prompt
            Constraint::Length(2), // Source details (name, path)
            Constraint::Length(2), // Active source warning
            Constraint::Length(1), // Action shortcuts
        ])
        .split(inner);

    let prompt = Paragraph::new(Span::styled(
        "Voulez-vous vraiment supprimer cette source de médias ?",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Center);
    frame.render_widget(prompt, chunks[0]);

    let details = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Source : ", Style::default().fg(Color::Cyan)),
            Span::styled(
                &modal.name_input,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Chemin : ", Style::default().fg(Color::Cyan)),
            Span::styled(&modal.path_input, Style::default().fg(Color::White)),
        ]),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(details, chunks[1]);

    let warning_spans = if modal.is_active_source {
        vec![
            Span::styled(
                "⚠️ ATTENTION : ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Cette source est ACTUELLEMENT ACTIVE. Ses médias ne seront plus surveillés.",
                Style::default().fg(Color::LightRed),
            ),
        ]
    } else {
        vec![Span::styled(
            "(Cette source est actuellement désactivée)",
            Style::default().fg(Color::DarkGray),
        )]
    };
    let warning = Paragraph::new(Line::from(warning_spans)).alignment(Alignment::Center);
    frame.render_widget(warning, chunks[2]);

    let shortcuts = Line::from(vec![
        Span::styled(
            " [Entrée / o / y] ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw("Confirmer la suppression      "),
        Span::styled(
            " [Esc / n] ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("Annuler"),
    ]);
    let footer_p = Paragraph::new(shortcuts).alignment(Alignment::Center);
    frame.render_widget(footer_p, chunks[3]);
}
