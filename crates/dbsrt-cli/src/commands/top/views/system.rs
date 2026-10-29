//! Fastfetch-inspired system and hardware host diagnostics view for `dbsrt top`.
//!
//! Presents structured node, distribution, CPU, GPU, memory, and application telemetry
//! organized into contextual, cleanly formatted, symmetrically aligned tree blocks.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::commands::top::constants::{
    BITS_PER_BYTE_F64, BYTES_PER_GIB_F64, DEFAULT_MEDIA_CACHE_PATH, DEFAULT_PHASH_DIR,
};
use crate::commands::top::host_info::HostSystemInfo;
use crate::commands::top::model::{
    format_duration, format_progress_bar, GpuTelemetry, SystemTelemetry,
};
use crate::commands::top::ui::UiContext;

/// Minimum terminal column width required to split system diagnostics into a dual-column layout.
pub const MIN_SYSTEM_TWO_COLUMN_WIDTH: u16 = 90;

/// Default path to zero-db certified ground-truth couplings registry.
pub const COUPLINGS_REGISTRY_PATH: &str = "data/SRT/couplings.json";

/// Renders the Fastfetch-inspired system and hardware diagnostics page.
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
/// 1. Constructs left-column blocks: Applicatif, Nœud & Distribution, Processeur.
/// 2. Constructs right-column blocks: Accélération GPU, Mémoire & Stockage, Débits & Session.
/// 3. If area width >= [`MIN_SYSTEM_TWO_COLUMN_WIDTH`], splits horizontally into dual columns;
///    otherwise stacks all sections into a single scrollable vertical paragraph.
pub fn render_system_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " 💻 INFORMATIONS SYSTÈME & MATÉRIEL (VUE FASTFETCH) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    if inner_area.width >= MIN_SYSTEM_TWO_COLUMN_WIDTH {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner_area);

        let left_lines = build_left_column_lines(ctx.telemetry);
        let right_lines = build_right_column_lines(ctx.telemetry);

        let left_para = Paragraph::new(left_lines);
        let right_para = Paragraph::new(right_lines);

        frame.render_widget(left_para, cols[0]);
        frame.render_widget(right_para, cols[1]);
    } else {
        let mut all_lines = build_left_column_lines(ctx.telemetry);
        all_lines.push(Line::from(""));
        all_lines.extend(build_right_column_lines(ctx.telemetry));

        let para = Paragraph::new(all_lines);
        frame.render_widget(para, inner_area);
    }
}

/// Builds left column lines: Applicatif, Nœud & Distribution, and CPU processor telemetry.
///
/// # Arguments
/// * `tel` - Latest system telemetry snapshot.
///
/// # Returns
/// Vector of styled [`Line`] instances for the left panel.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Sequential assembly of application provenance, OS distribution, and CPU hardware specs.
fn build_left_column_lines(tel: &SystemTelemetry) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let host = &tel.host_info;

    // 1. APPLICATIF (4 items)
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "📦 APPLICATIF (dbsrt)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(build_tree_line("Version binaire", &host.app_version, false));
    lines.push(build_tree_line("Provenance Git", &host.git_hash, false));
    lines.push(build_tree_line("Date de build", &host.build_date, false));
    lines.push(build_tree_line(
        "Couplages cert.",
        COUPLINGS_REGISTRY_PATH,
        true,
    ));

    lines.push(Line::from(""));

    // 2. NŒUD & DISTRIBUTION (4 items)
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "🌐 NŒUD & SYSTÈME D'EXPLOITATION",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(build_tree_line("Hôte (Hostname)", &host.hostname, false));
    lines.push(build_tree_line("Distribution", &host.os_name, false));
    lines.push(build_tree_line("Noyau (Kernel)", &host.kernel, false));
    lines.push(build_tree_line("Architecture", &host.arch, true));

    lines.push(Line::from(""));

    // 3. PROCESSEUR (CPU) (4 items)
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "🧠 PROCESSEUR (CPU)",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(build_tree_line("Modèle", &host.cpu_brand, false));
    let threads_str = format!("{} threads", host.cpu_cores_logical);
    lines.push(build_tree_line("Cœurs logiques", &threads_str, false));
    let phys_str = host
        .cpu_cores_physical
        .map(|c| format!("{c} cœurs physiques"))
        .unwrap_or_else(|| "N/A".to_string());
    lines.push(build_tree_line("Cœurs physiques", &phys_str, false));
    let cpu_load_str = format!("{:>4.1}%", tel.host_cpu_pct);
    lines.push(build_tree_line("Charge globale", &cpu_load_str, true));

    lines
}

/// Builds right column lines: GPU acceleration, memory/storage, and network/session flow rates.
///
/// # Arguments
/// * `tel` - Latest system telemetry snapshot.
///
/// # Returns
/// Vector of styled [`Line`] instances for the right panel.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Formats GPU hardware decoding stats, RAM allocation, and active streaming bandwidth.
fn build_right_column_lines(tel: &SystemTelemetry) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let host = &tel.host_info;
    let gpu = &tel.gpu;

    // 4. ACCÉLÉRATION MATÉRIELLE (GPU) (4 items)
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "🎮 ACCÉLÉRATION MATÉRIELLE (GPU)",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(build_tree_line("Contrôleur GPU", &host.gpu_name, false));
    let (engine_desc, is_hw) = format_gpu_engine_desc(host, gpu);
    lines.push(build_tree_line("Moteur décodage", &engine_desc, false));
    if is_hw {
        let load_str = format!("{:>5.1}%", gpu.video_busy_pct);
        lines.push(build_tree_line("Charge vidéo", &load_str, false));
        let pwr_str = format!("{:>4.1} W @ {:>4.0} MHz", gpu.power_watts, gpu.freq_mhz);
        lines.push(build_tree_line("Conso & Fréq.", &pwr_str, true));
    } else {
        lines.push(build_tree_line(
            "Mode d'ingestion",
            "Multi-threads CPU logiciel",
            false,
        ));
        lines.push(build_tree_line("Conso & Fréq.", "Inactif (Mode CPU)", true));
    }

    lines.push(Line::from(""));

    // 5. MÉMOIRE & STOCKAGE (4 items)
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "💾 MÉMOIRE & STOCKAGE",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let ram_pct = if tel.ram_total_gib > 0.0 {
        (tel.ram_used_gib / tel.ram_total_gib) * 100.0
    } else {
        0.0
    };
    let ram_str = format!(
        "{:>4.1} / {:>4.1} GiB ({:>2.0}%)",
        tel.ram_used_gib, tel.ram_total_gib, ram_pct
    );
    lines.push(build_tree_line("RAM système", &ram_str, false));
    lines.push(build_tree_line("Cache pHash", DEFAULT_PHASH_DIR, false));
    lines.push(build_tree_line(
        "Catalogue média",
        DEFAULT_MEDIA_CACHE_PATH,
        false,
    ));
    let sources_count = tel.media_sources.iter().filter(|s| s.enabled).count();
    let sources_str = format!("{sources_count} active(s)");
    lines.push(build_tree_line("Sources déclarées", &sources_str, true));

    lines.push(Line::from(""));

    // 6. DÉBITS & TÉLÉMÉTRIE EN DIRECT (4 items)
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "⚡ DÉBITS & TÉLÉMÉTRIE EN DIRECT",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let net_str = format!(
        "{:>5.1} MB/s ({:>5.1} Mbps)",
        tel.net_rx_mib_s,
        tel.net_rx_mib_s * BITS_PER_BYTE_F64
    );
    lines.push(build_tree_line("Réseau Net RX", &net_str, false));
    let pipe_str = format!("{:>5.1} MB/s", tel.total_rate_mib_s);
    lines.push(build_tree_line("Débit Pipe pHash", &pipe_str, false));
    let workers_str = format!("{} actifs", tel.active_jobs.len());
    lines.push(build_tree_line("Workers en cours", &workers_str, false));
    let session_str = if tel.session.stream_count > 0 {
        let gib = tel.session.session_bytes as f64 / BYTES_PER_GIB_F64;
        let dur = format_duration(tel.session.session_duration_secs);
        format!("{gib:.1} GiB ({dur}, {} flux)", tel.session.stream_count)
    } else {
        "0 flux (Session idle)".to_string()
    };
    lines.push(build_tree_line("Session globale", &session_str, true));

    lines
}

/// Resolves human-readable GPU engine description and boolean hardware acceleration flag.
///
/// # Arguments
/// * `host` - Host system metadata reference.
/// * `gpu` - GPU telemetry snapshot reference.
///
/// # Returns
/// A tuple `(description, is_hardware_active)`.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Inspects `gpu.available` and hardware descriptors to return informative status.
fn format_gpu_engine_desc(host: &HostSystemInfo, gpu: &GpuTelemetry) -> (String, bool) {
    if gpu.available {
        let bar = format_progress_bar(gpu.video_busy_pct / 100.0, 10);
        (format!("VA-API Actif [{bar}]"), true)
    } else if host.gpu_name == "Non détecté" || host.gpu_name.is_empty() {
        ("Non disponible (CPU multithread)".to_string(), false)
    } else {
        (
            format!("{} (Pilote non chargé / CPU)", host.gpu_name),
            false,
        )
    }
}

/// Helper formatting a single Fastfetch-style tree connector row.
///
/// # Arguments
/// * `key` - Property descriptor label.
/// * `value` - Formatted property value.
/// * `is_last` - Whether this is the terminating branch of the current section tree.
///
/// # Returns
/// Styled Ratatui [`Line`].
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Formats prefix connector (`│ ├─` or `│ └─`), padded label, and high-visibility value.
fn build_tree_line(key: &str, value: &str, is_last: bool) -> Line<'static> {
    let connector = if is_last {
        "  │ └─ "
    } else {
        "  │ ├─ "
    };

    Line::from(vec![
        Span::styled(connector, Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{key:<18}: "),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}
