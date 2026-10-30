//! View page rendering for candidate media full-frame inspection and codec diagnostic sheet.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::{
    constants::{BYTES_PER_GIB_F64, BYTES_PER_MIB_F64},
    model::{
        filter_and_sort_media_with_source_and_search, format_duration, DiscoveredMediaItem,
        IngestionStatus, MediaSourceConfig, MediaSubtrackInfo,
    },
    ui::UiContext,
};

/// Conversion factor from bytes to Kilobytes.
const BYTES_PER_KIB_F64: f64 = 1024.0;

/// Renders the full-frame diagnostic and inspection sheet for a candidate media.
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal render frame.
/// * `area` - Target display rectangular boundary.
/// * `ctx` - Active UI rendering context containing selection index and discovered media.
///
/// # Data Flow & Invariants
/// Filters discovered media by current query/source, retrieves the item at `media_scroll_index`,
/// and displays detailed hardware codec compatibility, pHash metadata, and lifecycle recommendations.
pub fn render_media_inspection_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " 🔍 FICHE DÉTAILLÉE & DIAGNOSTIC D'INGESTION DU MÉDIA (FULL-FRAME) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let visible_media = filter_and_sort_media_with_source_and_search(
        &ctx.telemetry.discovered_media,
        ctx.media_filter,
        ctx.media_sort,
        ctx.source_filter,
        &ctx.telemetry.media_sources,
        if ctx.search_query.is_empty() {
            None
        } else {
            Some(ctx.search_query)
        },
    );

    if visible_media.is_empty() || ctx.media_scroll_index >= visible_media.len() {
        let p = Paragraph::new("Aucun média sélectionné pour l'inspection.")
            .block(block)
            .alignment(Alignment::Center);
        frame.render_widget(p, area);
        return;
    }

    let item = &visible_media[ctx.media_scroll_index];
    let is_selected = ctx.selected_media_paths.contains(&item.path);

    let mut lines = Vec::new();
    lines.extend(build_media_identity_lines(
        item,
        is_selected,
        &ctx.telemetry.media_sources,
    ));
    lines.extend(build_media_codec_compatibility_lines(
        item.video_codec.as_deref().unwrap_or("Inconnu"),
    ));
    lines.extend(build_media_phash_lines(item));
    lines.extend(build_media_subtitles_lines(&item.subtitles));
    lines.extend(build_media_lifecycle_diagnostic_lines(item));
    lines.extend(build_media_engine_rules_lines());

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

/// Builds identity lines detailing media release, source label, filesystem access, size, and duration.
///
/// # Arguments
/// * `item` - Discovered media item.
/// * `is_selected` - Whether item is checked for batch processing.
/// * `sources` - Configured media sources slice.
pub fn build_media_identity_lines(
    item: &DiscoveredMediaItem,
    is_selected: bool,
    sources: &[MediaSourceConfig],
) -> Vec<Line<'static>> {
    let size_gib = item.size_bytes as f64 / BYTES_PER_GIB_F64;
    let dur_str = item
        .duration_secs
        .map(format_duration)
        .unwrap_or_else(|| "Inconnue".to_string());
    let codec_str = item.video_codec.as_deref().unwrap_or("Inconnu");

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

    let source_label = if let Some(ref sid) = item.source_id {
        let name = sources
            .iter()
            .find(|s| &s.id == sid)
            .map(|s| s.name.as_str())
            .unwrap_or(sid.as_str());
        format!("{name} ({sid})")
    } else {
        "Source principale".to_string()
    };

    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Release : ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                item.stem.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Source  : ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                source_label,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Chemin  : ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                item.path.display().to_string(),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Accès Disque: ", Style::default().fg(Color::DarkGray)),
            access_span,
        ]),
        Line::from(vec![
            Span::styled("  Taille  : ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{size_gib:.2} GiB ({} octets)", item.size_bytes),
                Style::default().fg(Color::White),
            ),
            Span::raw("    "),
            Span::styled("Durée : ", Style::default().fg(Color::DarkGray)),
            Span::styled(dur_str, Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Codec Vidéo : ", Style::default().fg(Color::DarkGray)),
            Span::styled(codec_str.to_string(), Style::default().fg(Color::Yellow)),
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
        ]),
    ]
}

/// Builds lines assessing hardware GPU decoding acceleration vs software CPU multithreading fallback.
///
/// # Arguments
/// * `codec_str` - Video codec descriptor string (e.g. `hevc`, `h264`, `Hi10P`).
pub fn build_media_codec_compatibility_lines(codec_str: &str) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "─── PROPRIÉTÉS VIDÉO & COMPATIBILITÉ MOTEUR DÉCODAGE ─────────────────────────",
            Style::default().fg(Color::DarkGray),
        )),
    ];

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
            Span::styled(
                "CPU (Software Multithread)",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" — Repli automatique et transparent sur CPU multi-cœurs (charge CPU ~800% normale)."),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(
                "  Codec & Profil   : ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(codec_str.to_string(), Style::default().fg(Color::White)),
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

    lines
}

/// Builds lines describing DCT-II perceptual hash footprint and disk storage.
///
/// # Arguments
/// * `item` - Discovered media item reference.
pub fn build_media_phash_lines(item: &DiscoveredMediaItem) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "─── EMPREINTE VISUELLE PHASH (HASHING VIDÉO DCT-II) ─────────────────────────",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    if let Some(ref phi) = item.phash {
        if phi.exists && phi.frame_count > 0 {
            let k_bytes = phi.size_bytes as f64 / BYTES_PER_KIB_F64;
            let m_bytes = phi.size_bytes as f64 / BYTES_PER_MIB_F64;
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
                    format!("{k_bytes:.1} KiB en RAM / B-K-Tree (8 octets / frame)"),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("    "),
                Span::styled("Fichier Local : ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("data/__PHASH__/{}.phash", item.stem),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            return lines;
        }
    }

    lines.push(Line::from(Span::styled(
        "  - AUCUN HACHAGE EFFECTUÉ (0 frame échantillonnée, aucun fichier .phash).",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

/// Builds lines enumerating embedded and sidecar subtitle streams.
///
/// # Arguments
/// * `subtitles` - Slice of subtitle track descriptors.
pub fn build_media_subtitles_lines(subtitles: &[MediaSubtrackInfo]) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "─── PISTES DE SOUS-TITRES DÉTECTÉES ──────────────────────────────────────────",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    if subtitles.is_empty() {
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
        for (i, sub) in subtitles.iter().enumerate() {
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

    lines
}

/// Builds lines presenting the lifecycle status badge and contextual operational recommendations.
///
/// # Arguments
/// * `item` - Discovered media item reference.
pub fn build_media_lifecycle_diagnostic_lines(item: &DiscoveredMediaItem) -> Vec<Line<'static>> {
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

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "─── STATUT DU CYCLE DE VIE & DIAGNOSTIC TECHNIQUE ─────────────────────────────",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(vec![
            Span::styled("  Statut Actuel : ", Style::default().fg(Color::White)),
            Span::styled(format!("[{}]", item.status.label()), status_style),
        ]),
        Line::from(""),
    ];

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

    lines
}

/// Builds footer lines summarizing core pydbsrt ingestion rules.
pub fn build_media_engine_rules_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "─── RÈGLES DU MOTEUR D'INGESTION PYDBSRT ──────────────────────────────────────",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  • But Premier : Hachage perceptuel DCT-II des frames vidéo pour la recherche visuelle rapide.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  • Couplage Certifié : Associe l'empreinte vidéo à une piste SRT de référence lorsque disponible.",
            Style::default().fg(Color::DarkGray),
        )),
    ]
}
