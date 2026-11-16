//! Full-frame multi-node cluster observability view (`[8/u] Cluster`) for `dbsrt top`.
//!
//! Renders the live `dbsrt doctor` multi-node matrix (`local`, `rpi4`, `o2switch`),
//! cross-node SHA-256 script parity, dataset/queue freshness, active `[CRIT]`/`[WARN]`
//! alerts, and deduplicated component log error signatures.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::{
    constants::{CLUSTER_SHA256_SHORT_LEN, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE},
    model::{
        ClusterAlertEntry, ClusterArtifactFreshness, ClusterChannelBadge, ClusterCorpusEntry,
        ClusterDataTopology, ClusterHealthTelemetry, ClusterNodeSnapshot, ClusterParityStatus,
        ClusterScriptParity, ClusterSeverity,
    },
    ui::UiContext,
};

/// Horizontal rule width used beneath the node health table header.
const NODE_TABLE_RULE_WIDTH: usize = 94;

/// Horizontal rule width used beneath the SHA-256 parity table header.
const PARITY_TABLE_RULE_WIDTH: usize = 92;

/// Renders the full-frame multi-node cluster health and parity diagnostics page (`[8/u]`).
///
/// # Arguments
/// * `frame` - Mutable Ratatui terminal frame buffer.
/// * `area` - Target rectangular display region allocated by root layout.
/// * `ctx` - Immutable UI rendering context carrying `telemetry.cluster` and `cluster_scroll_index`.
///
/// # Returns
/// `()` — draws bordered frame and scrollable telemetry paragraph into `frame`.
///
/// # Errors
/// Infallible rendering path.
///
/// # Panics
/// Guaranteed panic-free across empty telemetry, narrow terminals, and large scroll offsets.
///
/// # Data Flow & Invariants
/// Colors the outer frame dynamically from overall [`ClusterSeverity`], builds structured lines
/// across all five diagnostic sections, and applies vertical `Paragraph::scroll`.
pub fn render_cluster_page(frame: &mut Frame, area: Rect, ctx: &UiContext) {
    let (border_color, title_badge) = match &ctx.telemetry.cluster {
        Some(c) => (severity_color(c.status), format!(" [{}]", c.status.badge())),
        None => (Color::Cyan, " [PROBE EN COURS]".to_string()),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" 🩺 OBSERVABILITÉ CLUSTER MULTI-NŒUDS (dbsrt doctor){title_badge} "),
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        ));

    let lines = match &ctx.telemetry.cluster {
        Some(cluster) => build_cluster_lines(cluster),
        None => build_initializing_lines(),
    };

    let p = Paragraph::new(lines)
        .block(block)
        .scroll((ctx.cluster_scroll_index as u16, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

/// Maps cluster/node severity to its canonical foreground color.
///
/// # Arguments
/// * `sev` - [`ClusterSeverity`] enum value.
///
/// # Returns
/// `Green` (`Ok`), `Yellow` (`Warning`), or `Red` (`Critical`).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Pure constant match shared across frame borders, badges, and alert lines.
fn severity_color(sev: ClusterSeverity) -> Color {
    match sev {
        ClusterSeverity::Ok => Color::LightGreen,
        ClusterSeverity::Warning => Color::Yellow,
        ClusterSeverity::Critical => Color::Red,
    }
}

/// Builds placeholder lines displayed while the initial non-blocking cluster probe completes.
///
/// # Returns
/// Informative lines instructing the user how to force or wait for completion.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Zero blocking I/O on the UI thread; background worker populates `telemetry.cluster`.
fn build_initializing_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "  ⏳ Sonde cluster multi-nœuds (local, rpi4, o2switch) en cours d'exécution en arrière-plan...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  • Aucune opération bloquante sur l'interface TUI : les résultats apparaîtront automatiquement."),
        Line::from("  • Appuyez sur [a] pour déclencher une nouvelle sonde immédiate à tout moment."),
    ]
}

/// Assembles all five diagnostic sections for an active [`ClusterHealthTelemetry`] snapshot.
///
/// # Arguments
/// * `cluster` - Immutable reference to the latest evaluated cluster snapshot.
///
/// # Returns
/// Ordered [`Vec<Line<'static>>`] consumed directly by [`Paragraph`].
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Emits Banner -> Node Health Matrix -> Script Parity Table -> Freshness -> Alerts & Error Signatures.
fn build_cluster_lines(cluster: &ClusterHealthTelemetry) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    append_banner_lines(&mut lines, cluster);
    append_nodes_table(&mut lines, &cluster.nodes);
    append_parity_table(&mut lines, &cluster.script_parity);
    append_freshness_section(&mut lines, &cluster.nodes);
    if let Some(topo) = &cluster.data_topology {
        append_data_topology_section(&mut lines, topo);
    }
    append_alerts_and_signatures(&mut lines, cluster);
    lines
}

/// Appends the top cluster KPI banner summarizing exit code, elapsed probe time, and alert totals.
///
/// # Arguments
/// * `lines` - Mutable output line buffer.
/// * `cluster` - Cluster telemetry snapshot.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Surfaces parity drift count and active refresh/local-only flags inline.
fn append_banner_lines(lines: &mut Vec<Line<'static>>, cluster: &ClusterHealthTelemetry) {
    let sev_color = severity_color(cluster.status);
    let mode_tag = if cluster.local_only_probe {
        "local-only"
    } else {
        "multi-nœuds SSH"
    };
    let refresh_tag = if cluster.refreshing {
        " │ ⏳ ACTUALISATION..."
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  STATUS: ",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<4}", cluster.status.badge()),
            Style::default().fg(sev_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " │ EXIT: {} │ ELAPSED: {}ms │ CRIT: {} │ WARN: {} │ SHA256_DRIFT: {} │ MODE: {}{}",
            cluster.exit_code,
            cluster.elapsed_ms,
            cluster.active_critical_count,
            cluster.active_warning_count,
            cluster.sha256_drift_count,
            mode_tag,
            refresh_tag
        )),
    ]));
    lines.push(Line::from(Span::styled(
        format!(
            "  Dernière sonde : {}   ([a] actualiser maintenant  •  [↑/↓/PgUp/PgDn] défiler)",
            cluster.checked_at_iso
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
}

/// Appends the aligned per-node health matrix covering supervisor, aborts, queue fails, and errors.
///
/// # Arguments
/// * `lines` - Mutable output line buffer.
/// * `nodes` - Ordered slice of node snapshots (`local`, `rpi4`, `o2switch`).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Renders unreachable or skipped nodes explicitly with optional SSH error context beneath the row.
fn append_nodes_table(lines: &mut Vec<Line<'static>>, nodes: &[ClusterNodeSnapshot]) {
    lines.push(Line::from(Span::styled(
        "  SANTÉ DES NŒUDS",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "  {:<9} {:<7} {:<7} {:<10} {:<5} {:<6} {:<6} {:<5} {:<14} {:>7}",
            "NODE",
            "STATE",
            "DAEMON",
            "SUPERVISOR",
            "ABORT",
            "Q_FAIL",
            "BLKLST",
            "LVE24",
            "ERR(1h/24/72)",
            "PROBE"
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!("  {}", "─".repeat(NODE_TABLE_RULE_WIDTH)),
        Style::default().fg(Color::DarkGray),
    )));

    for node in nodes {
        append_single_node_row(lines, node);
    }
    lines.push(Line::from(""));
}

/// Formats one row of the node health table and optional unreachable diagnostic detail.
///
/// # Arguments
/// * `lines` - Mutable output line buffer.
/// * `node` - Single node snapshot.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Distinguishes `SKIP` (local-only), `UNREACH` (transport failure), and live probed metrics.
fn append_single_node_row(lines: &mut Vec<Line<'static>>, node: &ClusterNodeSnapshot) {
    if node.skipped {
        lines.push(Line::from(Span::styled(
            format!(
                "  {:<9} {:<7} {:<7} {:<10} {:<5} {:<6} {:<6} {:<5} {:<14} {:>7}",
                node.node, "SKIP", "-", "-", "-", "-", "-", "-", "-", "-"
            ),
            Style::default().fg(Color::DarkGray),
        )));
        return;
    }

    if !node.reachable {
        lines.push(Line::from(Span::styled(
            format!(
                "  {:<9} {:<7} {:<7} {:<10} {:<5} {:<6} {:<6} {:<5} {:<14} {:>5}ms",
                node.node, "UNREACH", "-", "-", "-", "-", "-", "-", "-", node.probe_ms
            ),
            Style::default().fg(Color::Yellow),
        )));
        if let Some(err) = &node.probe_error {
            lines.push(Line::from(Span::styled(
                format!("    └─ transport: {err}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
        return;
    }

    let daemon = match node.daemon_running {
        Some(true) => "up",
        Some(false) => "down",
        None => "-",
    };
    let supervisor = node.supervisor_state.as_deref().unwrap_or("-");
    let abort = if node.batch_aborted { "YES!" } else { "no" };
    let err_col = format!("{}/{}/{}", node.err_h1, node.err_h24, node.err_h72);
    let row_text = format!(
        "  {:<9} {:<7} {:<7} {:<10} {:<5} {:<6} {:<6} {:<5} {:<14} {:>5}ms",
        node.node,
        node.status.badge(),
        daemon,
        supervisor,
        abort,
        node.failed_queue_targets,
        node.blacklisted_stems,
        node.lve_faults_24h,
        err_col,
        node.probe_ms
    );
    lines.push(Line::from(Span::styled(
        row_text,
        Style::default().fg(severity_color(node.status)),
    )));
}

/// Appends the cross-node SHA-256 parity verification matrix for tracked scripts.
///
/// # Arguments
/// * `lines` - Mutable output line buffer.
/// * `rows` - Slice of script parity entries.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Highlights `SHA256_DRIFT` in bold red with explanatory mismatch text and `OK` in light green.
fn append_parity_table(lines: &mut Vec<Line<'static>>, rows: &[ClusterScriptParity]) {
    lines.push(Line::from(Span::styled(
        "  PARITÉ SHA-256 DES SCRIPTS CRITIQUES",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "  {:<28} {:<14} {:<14} {:<14} {:<14}",
            "SCRIPT", "LOCAL", "RPI4", "O2SWITCH", "PARITY"
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!("  {}", "─".repeat(PARITY_TABLE_RULE_WIDTH)),
        Style::default().fg(Color::DarkGray),
    )));

    for row in rows {
        let loc = short_hash(row.local_sha256.as_deref());
        let rpi = short_hash(row.rpi4_sha256.as_deref());
        let o2 = short_hash(row.o2switch_sha256.as_deref());
        let detail_suffix = row
            .detail
            .as_deref()
            .filter(|_| row.status == ClusterParityStatus::Sha256Drift)
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        let (color, modifier) = match row.status {
            ClusterParityStatus::Ok => (Color::LightGreen, Modifier::empty()),
            ClusterParityStatus::Sha256Drift => (Color::Red, Modifier::BOLD),
            ClusterParityStatus::LocalOnly => (Color::DarkGray, Modifier::empty()),
        };
        lines.push(Line::from(Span::styled(
            format!(
                "  {:<28} {:<14} {:<14} {:<14} {}{}",
                row.script,
                loc,
                rpi,
                o2,
                row.status.badge(),
                detail_suffix
            ),
            Style::default().fg(color).add_modifier(modifier),
        )));
    }
    lines.push(Line::from(""));
}

/// Formats an optional 64-char SHA-256 digest into a [`CLUSTER_SHA256_SHORT_LEN`]-char prefix.
///
/// # Arguments
/// * `opt` - Optional SHA-256 hex digest string.
///
/// # Returns
/// Short hex prefix string or `"-"`.
///
/// # Panics
/// Never panics (guards slice length against [`CLUSTER_SHA256_SHORT_LEN`]).
///
/// # Data Flow & Invariants
/// Safe on arbitrary UTF-8 strings by capping only when byte len meets threshold for hex ASCII.
fn short_hash(opt: Option<&str>) -> &str {
    match opt {
        Some(h) if h.len() >= CLUSTER_SHA256_SHORT_LEN && h.is_ascii() => {
            &h[..CLUSTER_SHA256_SHORT_LEN]
        }
        Some(h) => h,
        None => "-",
    }
}

/// Appends per-node artifact and queue freshness lines (`ok`, `STALE`, `absent`).
///
/// # Arguments
/// * `lines` - Mutable output line buffer.
/// * `nodes` - Slice of probed node snapshots.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Renders one line per reachable node with color-coded `Span` badges for each tracked artifact.
fn append_freshness_section(lines: &mut Vec<Line<'static>>, nodes: &[ClusterNodeSnapshot]) {
    lines.push(Line::from(Span::styled(
        "  FRAÎCHEUR DES ARTEFACTS & FILES D'ATTENTE",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    for node in nodes.iter().filter(|n| n.reachable) {
        let mut spans = vec![Span::styled(
            format!("  {:<9}: ", node.node),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )];
        for (idx, item) in node.freshness.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
            }
            spans.push(format_freshness_span(item));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
}

/// Formats one artifact freshness badge with color indicating `absent`, `STALE`, or `ok`.
///
/// # Arguments
/// * `item` - Tracked artifact freshness metadata.
///
/// # Returns
/// Styled [`Span<'static>`].
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Pure threshold color mapping.
fn format_freshness_span(item: &ClusterArtifactFreshness) -> Span<'static> {
    if !item.exists {
        return Span::styled(
            format!("{}=absent", item.name),
            Style::default().fg(Color::DarkGray),
        );
    }
    let age = format_compact_age(item.age_secs.unwrap_or(0));
    if item.stale {
        Span::styled(
            format!("{}={}(STALE)", item.name, age),
            Style::default().fg(Color::Yellow),
        )
    } else {
        Span::styled(
            format!("{}={}(ok)", item.name, age),
            Style::default().fg(Color::LightGreen),
        )
    }
}

/// Formats elapsed seconds into compact `Nd`, `Nh`, `Nm`, or `Ns` age notation.
///
/// # Arguments
/// * `secs` - Non-negative elapsed duration in seconds.
///
/// # Returns
/// Human-readable compact age string.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Uses named conversion factors [`SECONDS_PER_DAY`], [`SECONDS_PER_HOUR`], [`SECONDS_PER_MINUTE`].
fn format_compact_age(secs: u64) -> String {
    if secs >= SECONDS_PER_DAY {
        format!("{}d", secs / SECONDS_PER_DAY)
    } else if secs >= SECONDS_PER_HOUR {
        format!("{}h", secs / SECONDS_PER_HOUR)
    } else if secs >= SECONDS_PER_MINUTE {
        format!("{}m", secs / SECONDS_PER_MINUTE)
    } else {
        format!("{secs}s")
    }
}

/// Appends active alerts (`[CRIT]`/`[WARN]`) and deduplicated log error signatures across nodes.
///
/// # Arguments
/// * `lines` - Mutable output line buffer.
/// * `cluster` - Complete cluster health telemetry snapshot.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Displays active alerts first (critical before warning) followed by non-empty component
/// error signatures so operators see both cluster roll-up alerts and underlying log motifs.
fn append_alerts_and_signatures(lines: &mut Vec<Line<'static>>, cluster: &ClusterHealthTelemetry) {
    lines.push(Line::from(Span::styled(
        "  ALERTES ACTIVES & DRIFT",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    if cluster.alerts.is_empty() {
        lines.push(Line::from(Span::styled(
            "  ✓ Aucune alerte active (tous les nœuds sondés et scripts critiques sont nominaux).",
            Style::default().fg(Color::LightGreen),
        )));
    } else {
        let mut sorted: Vec<&ClusterAlertEntry> = cluster.alerts.iter().collect();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.severity));
        for alert in sorted {
            let color = severity_color(alert.severity);
            lines.push(Line::from(Span::styled(
                format!(
                    "  [{}] [{}:{}] {}",
                    alert.severity.badge(),
                    alert.node,
                    alert.component,
                    alert.summary
                ),
                Style::default().fg(color),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  SIGNATURES D'ERREURS LOGS DÉDUPLIQUÉES (1h / 24h / 72h)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    let mut sig_count = 0usize;
    for node in cluster.nodes.iter().filter(|n| n.reachable) {
        for comp in &node.components {
            sig_count += append_component_signatures(lines, &node.node, comp);
        }
    }
    if sig_count == 0 {
        lines.push(Line::from(Span::styled(
            "  ✓ Zéro signature d'erreur récente dans les journaux sondés.",
            Style::default().fg(Color::LightGreen),
        )));
    }
}

/// Appends deduplicated error signatures for one node component and returns appended count.
fn append_component_signatures(
    lines: &mut Vec<Line<'static>>,
    node_name: &str,
    comp: &crate::model::ClusterComponentErrorSummary,
) -> usize {
    for sig in &comp.top_signatures {
        let color = if sig.critical {
            Color::Red
        } else {
            Color::Yellow
        };
        let last_seen = sig
            .last_seen
            .as_deref()
            .map(|ts| format!(" [{ts}]"))
            .unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!(
                "  [{}:{}] (x{}){} {}",
                node_name, comp.component, sig.count, last_seen, sig.summary
            ),
            Style::default().fg(color),
        )));
    }
    comp.top_signatures.len()
}

/// Appends workstation network topology, SSOT channel states, corpus deltas, and backup summary.
///
/// # Arguments
/// * `lines` - Mutable output line buffer.
/// * `topo` - Evaluated [`ClusterDataTopology`] snapshot from `dbsrt_data.py status`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Surfaces `cifs_safe_to_stat`, non-skipped channel latencies, corpus deltas, and `backup_state`.
fn append_data_topology_section(lines: &mut Vec<Line<'static>>, topo: &ClusterDataTopology) {
    lines.push(Line::from(Span::styled(
        "  TOPOLOGIE POSTE, CANAUX & CORPUS (SSOT)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    let cifs_span = if topo.cifs_safe_to_stat {
        Span::styled(
            "SAFE (Home LAN)",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "BLOQUÉ (Garde-fou VPN/WAN)",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    };
    let backup_txt = format_backup_summary(topo);

    lines.push(Line::from(vec![
        Span::raw(format!(
            "  Réseau : {} ({}) │ CIFS : ",
            topo.network_profile, topo.host_env
        )),
        cifs_span,
        Span::raw(format!(" │ Backups : {backup_txt}")),
    ]));

    append_channel_badges_line(lines, &topo.channels);
    for entry in &topo.corpus {
        lines.push(format_corpus_line(entry));
    }
    lines.push(Line::from(""));
}

/// Formats multi-tier backup state and latest snapshot suffix.
fn format_backup_summary(topo: &ClusterDataTopology) -> String {
    let status = topo.backup_status.as_deref().unwrap_or("UNKNOWN");
    let snap = topo.backup_snapshot.as_deref().unwrap_or("-");
    let reps = if topo.backup_replicas.is_empty() {
        "-".to_string()
    } else {
        topo.backup_replicas.join(",")
    };
    format!("{status} ({snap} • {reps} • evict:{})", topo.evicted_count)
}

/// Appends compact channel availability spans grouped on a single readable line.
fn append_channel_badges_line(lines: &mut Vec<Line<'static>>, channels: &[ClusterChannelBadge]) {
    if channels.is_empty() {
        return;
    }
    let mut spans = vec![Span::styled(
        "  Canaux : ",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];
    let mut shown = 0usize;
    for ch in channels.iter().filter(|c| c.state_tag != "SKIP") {
        if shown > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        }
        let color = if ch.accessible {
            Color::LightGreen
        } else if ch.state_tag == "GUARD" {
            Color::Yellow
        } else {
            Color::Red
        };
        let lat = if ch.accessible {
            format!("{:.0}ms", ch.latency_ms)
        } else {
            ch.state_tag.clone()
        };
        spans.push(Span::styled(
            format!("{}({})", ch.id, lat),
            Style::default().fg(color),
        ));
        shown += 1;
    }
    if shown > 0 {
        lines.push(Line::from(spans));
    }
}

/// Formats one corpus comparison row (`local` vs `remote` + status badge).
fn format_corpus_line(entry: &ClusterCorpusEntry) -> Line<'static> {
    let color = match entry.status.as_str() {
        "UP_TO_DATE" => Color::LightGreen,
        "STALE_LOCAL" | "PARTIAL_LOCAL" => Color::Yellow,
        _ => Color::Red,
    };
    let delta = if entry.delta_summary.is_empty() {
        String::new()
    } else {
        format!(" • {}", entry.delta_summary)
    };
    Line::from(vec![
        Span::styled(
            format!("  {:<15}: ", entry.label),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "local={} vs distant={} ",
            entry.local_summary, entry.remote_summary
        )),
        Span::styled(
            format!("[{}{delta}]", entry.status),
            Style::default().fg(color),
        ),
    ])
}
