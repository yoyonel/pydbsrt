//! Top real-time telemetry header banner for `dbsrt top`.
//!
//! Renders host CPU/RAM, active workers, dynamic CPU/GPU engines, real-time throughput, and session totals.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::constants::{
    BITS_PER_BYTE_F64, BYTES_PER_GIB_F64, GPU_HIGH_LOAD_THRESHOLD_PCT,
    GPU_MODERATE_LOAD_THRESHOLD_PCT, GPU_PROGRESS_BAR_CHAR_WIDTH,
    MIN_SESSION_DURATION_DISPLAY_SECS,
};
use crate::host_info::HostSystemInfo;
use crate::model::{
    format_duration, format_progress_bar, ActiveJobTelemetry, BatchQueueTelemetry,
    ClusterHealthTelemetry, ClusterSeverity, GpuTelemetry, StreamKind, SystemTelemetry,
};
use crate::views::widgets::get_current_time_str;

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
/// 1. Assembles four distinct telemetry lines: Host & Workers, Engines (CPU/GPU), Real-Time Flow, Session Cumul.
/// 2. Configures a bordered block with version banner on the left and live clock on the right.
/// 3. Renders the paragraph widget onto `area`.
pub fn render_header(frame: &mut Frame, area: Rect, tel: &SystemTelemetry) {
    let now = get_current_time_str();
    let line1 = build_host_telemetry_line(tel);
    let line2 = build_hardware_telemetry_line(&tel.host_info, &tel.gpu);
    let line3 = build_realtime_flow_line(tel);
    let line4 = build_session_cumul_line(tel);

    let left_title_text = format!(
        " pydbsrt top {} ── Moniteur de Décodage pHash & Ingestion Temps Réel ",
        tel.host_info.app_version
    );

    let left_title = Line::from(Span::styled(
        left_title_text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Left);

    let right_title = Line::from(Span::styled(
        format!(" ⏱️  {now} "),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Right);

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(left_title)
        .title(right_title);

    let p = Paragraph::new(vec![line1, line2, line3, line4]).block(header_block);
    frame.render_widget(p, area);
}

/// Formats the header cluster badge showing overall `dbsrt doctor` severity and active alert count.
fn format_cluster_header_badge(cluster: &ClusterHealthTelemetry) -> Span<'static> {
    let (label, fg) = match cluster.status {
        ClusterSeverity::Ok => ("Cluster: OK".to_string(), Color::LightGreen),
        ClusterSeverity::Warning => (
            format!("Cluster: WARN({})", cluster.active_warning_count),
            Color::Yellow,
        ),
        ClusterSeverity::Critical => (
            format!(
                "Cluster: CRIT({}/{})",
                cluster.active_critical_count, cluster.sha256_drift_count
            ),
            Color::Red,
        ),
    };
    Span::styled(label, Style::default().fg(fg).add_modifier(Modifier::BOLD))
}

/// Formats the header batch queue badge, surfacing `batch_type` when present (`Batch [freebox]: ...`).
fn format_batch_queue_badge(queue: &BatchQueueTelemetry) -> String {
    let prefix = match queue.batch_type.as_deref() {
        Some(kind) if !kind.is_empty() => format!("Batch [{kind}]"),
        _ => "Batch".to_string(),
    };
    format!(
        "{prefix}: {}/{} ({:.0}%) ",
        queue.completed_targets, queue.total_targets, queue.progress_pct
    )
}

/// Builds Line 1: Node hostname, OS/arch, active workers count, host CPU%, and RAM GiB.
///
/// # Arguments
/// * `tel` - System telemetry snapshot reference.
///
/// # Returns
/// Formatted [`Line`] containing host and worker metrics.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Renders hostname with dark-gray OS tag, active workers with status color, CPU load, and RAM usage percentage.
fn build_host_telemetry_line(tel: &SystemTelemetry) -> Line<'static> {
    let host = &tel.host_info;
    let workers_count = tel.active_jobs.len();
    let worker_style = if workers_count > 0 {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let ram_pct = if tel.ram_total_gib > 0.0 {
        (tel.ram_used_gib / tel.ram_total_gib) * 100.0
    } else {
        0.0
    };

    let mut spans = vec![
        Span::styled(
            " 🖥️  HÔTE      │ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ", host.hostname),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({} {}) ", host.os_name, host.arch),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("│ "),
        Span::styled(
            match (tel.batch_queue.desired_workers, tel.batch_queue.is_paused) {
                (Some(w), true) => format!("Workers: {workers_count}/{w} [PAUSE] "),
                (Some(w), false) => format!("Workers: {workers_count}/{w} actifs "),
                (None, true) => format!("Workers: {workers_count} [PAUSE] "),
                (None, false) => format!("Workers: {workers_count} actifs "),
            },
            if tel.batch_queue.is_paused {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                worker_style
            },
        ),
        Span::raw("│ "),
    ];

    if tel.batch_queue.total_targets > 0 {
        spans.push(Span::styled(
            format_batch_queue_badge(&tel.batch_queue),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("│ "));
    }

    spans.extend([
        match tel.account_cpu_pct {
            Some(pct) => Span::styled(
                format!("CPU compte: {pct:>2.0}% "),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            None => Span::styled(
                format!("Host CPU: {:>2.0}% ", tel.host_cpu_pct),
                Style::default().fg(Color::White),
            ),
        },
        Span::raw("│ "),
    ]);
    // Host RAM stays in the Système tab: on shared hosting it counts every tenant.
    spans.push(match tel.account_ram_label() {
        Some(label) => Span::styled(
            format!("RAM compte: {label}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        None => Span::styled(
            format!(
                "RAM: {:>4.1}/{:>4.1} GiB ({:>2.0}%)",
                tel.ram_used_gib, tel.ram_total_gib, ram_pct
            ),
            Style::default().fg(Color::White),
        ),
    });

    if tel.is_scanning {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            "⟳ SCAN DISQUE EN COURS...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(cluster) = &tel.cluster {
        spans.push(Span::raw(" │ "));
        spans.push(format_cluster_header_badge(cluster));
    }

    Line::from(spans)
}

/// Builds Line 2: Host CPU specification and GPU hardware decoding engine load.
///
/// # Arguments
/// * `host` - Host hardware metadata reference.
/// * `gpu` - GPU telemetry snapshot reference.
///
/// # Returns
/// Formatted [`Line`] containing CPU model and GPU acceleration status.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Displays CPU brand and thread count, followed by GPU status (decoding load if active, or CPU fallback mode).
fn build_hardware_telemetry_line(host: &HostSystemInfo, gpu: &GpuTelemetry) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            " ⚙️  MOTEURS   │ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("CPU: {} ({} c.) ", host.cpu_brand, host.cpu_cores_logical),
            Style::default().fg(Color::White),
        ),
        Span::raw("│ "),
        Span::styled("GPU: ", Style::default().fg(Color::Cyan)),
    ];

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

        spans.extend(vec![
            Span::styled(
                format!("{} ", host.gpu_name),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("(Décodage: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:>5.1}% {v_bar}", gpu.video_busy_pct), v_style),
            Span::styled(
                format!(" │ Conso: {:>4.1} W)", gpu.power_watts),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
    } else if host.gpu_name == "Non détecté" || host.gpu_name.is_empty() {
        spans.push(Span::styled(
            "Non disponible (Mode CPU Multithread)",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        spans.push(Span::styled(
            format!("{} (Mode CPU Multithread)", host.gpu_name),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(spans)
}

/// Checks whether any active job is streaming video over remote network protocols (FTP/HTTP/SFTP).
///
/// # Arguments
/// * `jobs` - Slice of active job telemetry descriptors.
///
/// # Returns
/// `true` if any job argument matches a remote URI scheme, `false` otherwise.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Inspects commandline arguments for protocol schemes.
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

/// Builds the right-hand pipe throughput spans (`Pipe pHash`, `Pipe aHash`, or `Pipe pHash+aHash`).
fn build_pipe_flow_spans(jobs: &[ActiveJobTelemetry], safe_total_rate: f64) -> Vec<Span<'static>> {
    let has_video = jobs
        .iter()
        .any(|j| j.stream_kind() == StreamKind::VideoPhash);
    let has_audio = jobs.iter().any(ActiveJobTelemetry::is_ahash_audio);
    let total_video_fps: f64 = jobs
        .iter()
        .filter(|j| j.stream_kind() == StreamKind::VideoPhash)
        .map(|j| j.hashing_fps)
        .sum();
    let total_audio_rt: f64 = jobs
        .iter()
        .filter(|j| j.is_ahash_audio())
        .map(ActiveJobTelemetry::ahash_realtime_factor)
        .sum();
    let rate_style = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD);

    if has_audio && !has_video {
        return vec![
            Span::styled("Pipe aHash: ", Style::default().fg(Color::White)),
            Span::styled(format!("{safe_total_rate:>5.2} MB/s "), rate_style),
            Span::styled(
                format!("({total_audio_rt:.1}x RT)"),
                Style::default().fg(Color::LightCyan),
            ),
        ];
    }
    if has_audio && has_video {
        return vec![
            Span::styled("Pipe pHash+aHash: ", Style::default().fg(Color::White)),
            Span::styled(format!("{safe_total_rate:>5.1} MB/s "), rate_style),
            Span::styled(
                format!("({total_video_fps:.0} fps + {total_audio_rt:.1}x RT)"),
                Style::default().fg(Color::Yellow),
            ),
        ];
    }
    if is_streaming_active(jobs) {
        return vec![
            Span::styled("Pipe pHash: ", Style::default().fg(Color::White)),
            Span::styled(format!("{safe_total_rate:>5.1} MB/s "), rate_style),
            Span::styled(
                format!("({total_video_fps:.0} fps)"),
                Style::default().fg(Color::Yellow),
            ),
        ];
    }
    vec![
        Span::styled("Pipe pHash: ", Style::default().fg(Color::White)),
        Span::styled(format!("{safe_total_rate:>5.1} MB/s"), rate_style),
    ]
}

/// Builds Line 3: Real-time network RX throughput and pHash/aHash pipe processing rate.
///
/// # Arguments
/// * `tel` - System telemetry snapshot reference.
///
/// # Returns
/// Formatted [`Line`] containing network RX rate and active pipe bandwidth.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// Differentiates acoustic `aHash` PCM pipes (`x RT`), mixed `pHash+aHash`, and video `pHash` pipes.
fn build_realtime_flow_line(tel: &SystemTelemetry) -> Line<'static> {
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

    let net_label = if tel.is_shared_lve_account() {
        "Réseau compte RX: "
    } else {
        "Réseau Net RX: "
    };

    let mut spans = vec![
        Span::styled(
            " ⚡ FLUX REEL  │ ",
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(net_label, Style::default().fg(Color::White)),
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
        Span::styled("  ──►  ", Style::default().fg(Color::DarkGray)),
    ];
    spans.extend(build_pipe_flow_spans(&tel.active_jobs, safe_total_rate));

    Line::from(spans)
}

/// Builds Line 4: Cumulative session totals (processed volume, duration, mean speed, streams).
///
/// # Arguments
/// * `tel` - System telemetry snapshot reference.
///
/// # Returns
/// Formatted [`Line`] containing cumulative session throughput statistics.
///
/// # Panics
/// Panic-free.
///
/// # Data Flow & Invariants
/// If session has processed data, formats total GiB, elapsed time, mean throughput, and stream count.
/// Otherwise renders idle awaiting status.
fn build_session_cumul_line(tel: &SystemTelemetry) -> Line<'static> {
    let mut spans = vec![Span::styled(
        " 📈 CUMUL SESS │ ",
        Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
    )];

    if tel.session.stream_count > 0
        && tel.session.session_duration_secs > MIN_SESSION_DURATION_DISPLAY_SECS
    {
        let session_gib = tel.session.session_bytes as f64 / BYTES_PER_GIB_F64;
        let dur_str = format_duration(tel.session.session_duration_secs);
        spans.push(Span::styled(
            format!(
                "{session_gib:.1} GiB traités en {dur_str} (Moyenne: {:>4.1} MB/s sur {} flux)",
                tel.session.session_rate_mib_s, tel.session.stream_count
            ),
            Style::default().fg(Color::White),
        ));
    } else {
        spans.push(Span::styled(
            "0.0 MB traités en 0s (En attente de flux...)",
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(spans)
}
