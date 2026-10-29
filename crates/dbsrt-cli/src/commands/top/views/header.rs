//! Top real-time telemetry header banner for `dbsrt top`.
//!
//! Renders host CPU/RAM, Intel Iris Xe hardware GPU load/VRAM, and CIFS network throughput.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::commands::top::constants::{
    BITS_PER_BYTE_F64, BYTES_PER_GIB_F64, GPU_HIGH_LOAD_THRESHOLD_PCT,
    GPU_MODERATE_LOAD_THRESHOLD_PCT, GPU_PROGRESS_BAR_CHAR_WIDTH,
    MIN_SESSION_DURATION_DISPLAY_SECS,
};
use crate::commands::top::model::{
    format_duration, format_progress_bar, ActiveJobTelemetry, GpuTelemetry, SystemTelemetry,
};
use crate::commands::top::views::widgets::get_current_time_str;

/// Renders the top real-time telemetry header banner with system, GPU, and network metrics.
///
/// # Arguments
/// * `frame` - Current Ratatui terminal frame buffer.
/// * `area` - Target rectangle area allocated for the header banner.
/// * `tel` - Latest system telemetry snapshot.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// 1. Assembles three distinct telemetry lines: Host CPU/RAM, Intel GPU, CIFS Network.
/// 2. Wraps them inside a bordered block with cyan title.
/// 3. Renders the paragraph widget onto `area`.
pub fn render_header(frame: &mut Frame, area: Rect, tel: &SystemTelemetry) {
    let line1 = build_host_telemetry_line(tel);
    let line2 = build_gpu_telemetry_line(&tel.gpu);
    let line3 = build_cifs_network_line(tel);

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

/// Builds Line 1: Clock, active workers count, host CPU%, and RAM GiB.
fn build_host_telemetry_line(tel: &SystemTelemetry) -> Line<'static> {
    let now = get_current_time_str();
    let workers_count = tel.active_jobs.len();
    let worker_style = if workers_count > 0 {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };

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

    Line::from(line1_spans)
}

/// Builds Line 2: Intel Iris Xe hardware GPU video decoding load, render 3D, and power consumption.
fn build_gpu_telemetry_line(gpu: &GpuTelemetry) -> Line<'static> {
    if gpu.available {
        let v_bar = format_progress_bar(gpu.video_busy_pct / 100.0, GPU_PROGRESS_BAR_CHAR_WIDTH);
        let v_style = if gpu.video_busy_pct > GPU_HIGH_LOAD_THRESHOLD_PCT {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if gpu.video_busy_pct > GPU_MODERATE_LOAD_THRESHOLD_PCT {
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
            Span::styled(format!("{:>5.1}% {v_bar} ", gpu.video_busy_pct), v_style),
            Span::raw("│ "),
            Span::styled(
                format!("Render/3D: {:>4.1}% ", gpu.render_busy_pct),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("│ "),
            Span::styled(
                format!("Fréq: {:>4.0} MHz ", gpu.freq_mhz),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("│ "),
            Span::styled(
                format!("Conso: {:>4.1} W", gpu.power_watts),
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
    }
}

/// Checks whether any active job is streaming video over remote network protocols (FTP/HTTP/SFTP).
fn is_streaming_active(jobs: &[ActiveJobTelemetry]) -> bool {
    !jobs.is_empty()
        && jobs.iter().any(|j| {
            j.cmdline.iter().any(|arg| {
                arg.starts_with("ftp://")
                    || arg.starts_with("http://")
                    || arg.starts_with("sftp://")
            })
        })
}

/// Builds Line 3: CIFS network throughput, instant RX rate, and cumulative session stats.
fn build_cifs_network_line(tel: &SystemTelemetry) -> Line<'static> {
    let safe_total_rate = if tel.total_rate_mib_s <= 0.0 {
        0.0
    } else {
        tel.total_rate_mib_s
    };
    let safe_net_rx = if tel.net_rx_mib_s <= 0.0 {
        0.0
    } else {
        tel.net_rx_mib_s
    };

    let mut line3_spans = if is_streaming_active(&tel.active_jobs) {
        let total_fps: f64 = tel.active_jobs.iter().map(|j| j.hashing_fps).sum();
        vec![
            Span::styled(
                " ⚡ DÉBITS RÉSEAU & PIPE : ",
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Net RX: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:>5.1} MB/s ", safe_net_rx),
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({:>5.1} Mbps)", safe_net_rx * BITS_PER_BYTE_F64),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" │ "),
            Span::styled("Pipe pHash: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:>5.1} MB/s ", safe_total_rate),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({:.0} fps)", total_fps),
                Style::default().fg(Color::Yellow),
            ),
        ]
    } else {
        vec![
            Span::styled(
                " ⚡ DÉBITS RÉSEAU CIFS : ",
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Instantané: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:>5.1} MB/s ", safe_total_rate),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "(Net RX: {:>5.1} MB/s, VFS: {:>5.1} MB/s)",
                    safe_net_rx, safe_total_rate
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]
    };

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

    Line::from(line3_spans)
}
