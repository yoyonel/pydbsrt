//! Human-readable aligned table and JSON formatters for `dbsrt doctor`.
//!
//! # Data Flow & Invariants
//! Formats [`ClusterDoctorReport`] into either deterministic pretty-printed JSON (`--json`) or
//! a compact, aligned operator table summarizing per-node error windows, cross-node SHA-256
//! script parity, dataset/queue freshness, and active alerts.

use crate::commands::doctor::constants::{
    SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE, SHA256_SHORT_HEX_LEN,
};
use crate::commands::doctor::models::{
    ClusterDoctorReport, NodeProbeReport, ParityStatus, ScriptParityEntry,
};
use std::fmt::Write;

/// Formats a [`ClusterDoctorReport`] as a human-readable multi-section CLI summary.
///
/// # Arguments
/// * `report` - Evaluated [`ClusterDoctorReport`].
///
/// # Returns
/// Multi-line formatted report string ready for stdout.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Renders 4 deterministic sections: Cluster Banner, Node Health Matrix, Cross-Node SHA-256
/// Script Parity Table, and Active Alerts / Freshness Details.
pub fn render_human_report(report: &ClusterDoctorReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        &mut out,
        "dbsrt doctor ── Multi-Node Cluster Observability ({})",
        report.checked_at_iso
    );
    let _ = writeln!(
        &mut out,
        "STATUS: {:<4} │ EXIT: {} │ ELAPSED: {}ms │ CRIT: {} │ WARN: {} │ SHA256_DRIFT: {}",
        report.status.badge(),
        report.exit_code,
        report.elapsed_ms,
        report.active_critical_count,
        report.active_warning_count,
        report.sha256_drift_count
    );
    let _ = writeln!(&mut out);

    render_nodes_table(&mut out, &report.nodes);
    let _ = writeln!(&mut out);
    render_parity_table(&mut out, &report.script_parity);
    let _ = writeln!(&mut out);
    render_freshness_section(&mut out, &report.nodes);
    render_alerts_section(&mut out, &report.nodes, &report.script_parity);
    out
}

/// Appends the per-node health and rolling error window table.
fn render_nodes_table(out: &mut String, nodes: &[NodeProbeReport]) {
    let _ = writeln!(
        out,
        "{:<10} {:<8} {:<11} {:<6} {:<6} {:<6} {:<15} {:>7}",
        "NODE", "STATE", "SUPERVISOR", "ABORT", "Q_FAIL", "BLKLST", "ERR(1h/24h/72h)", "PROBE"
    );
    let _ = writeln!(out, "{}", "─".repeat(76));
    for n in nodes {
        if n.skipped {
            let _ = writeln!(
                out,
                "{:<10} {:<8} {:<11} {:<6} {:<6} {:<6} {:<15} {:>7}",
                n.node, "SKIP", "-", "-", "-", "-", "-", "-"
            );
            continue;
        }
        if !n.reachable {
            let _ = writeln!(
                out,
                "{:<10} {:<8} {:<11} {:<6} {:<6} {:<6} {:<15} {:>5}ms",
                n.node, "UNREACH", "-", "-", "-", "-", "-", n.probe_ms
            );
            continue;
        }
        let (h1, h24, h72) = sum_node_errors(n);
        let sup = n.supervisor_state.as_deref().unwrap_or("-");
        let abort = if n.batch_aborted { "YES!" } else { "no" };
        let err_col = format!("{h1}/{h24}/{h72}");
        let _ = writeln!(
            out,
            "{:<10} {:<8} {:<11} {:<6} {:<6} {:<6} {:<15} {:>5}ms",
            n.node,
            n.status.badge(),
            sup,
            abort,
            n.failed_queue_targets,
            n.blacklisted_stems,
            err_col,
            n.probe_ms
        );
    }
}

/// Sums `(h1, h24, h72)` error counts across all components on `node`.
fn sum_node_errors(node: &NodeProbeReport) -> (usize, usize, usize) {
    node.components.iter().fold((0, 0, 0), |(a, b, c), comp| {
        (a + comp.counts.h1, b + comp.counts.h24, c + comp.counts.h72)
    })
}

/// Appends the cross-node SHA-256 script parity table.
fn render_parity_table(out: &mut String, rows: &[ScriptParityEntry]) {
    let _ = writeln!(
        out,
        "{:<28} {:<14} {:<14} {:<14} {:<14}",
        "SCRIPT", "LOCAL", "RPI4", "O2SWITCH", "PARITY"
    );
    let _ = writeln!(out, "{}", "─".repeat(88));
    for r in rows {
        let loc = short_hash(r.local_sha256.as_deref());
        let rpi = short_hash(r.rpi4_sha256.as_deref());
        let o2 = short_hash(r.o2switch_sha256.as_deref());
        let status = match r.status {
            ParityStatus::Ok => "OK",
            ParityStatus::Sha256Drift => "SHA256_DRIFT",
            ParityStatus::LocalOnly => "LOCAL_ONLY",
        };
        let suffix = r
            .detail
            .as_deref()
            .filter(|_| r.status == ParityStatus::Sha256Drift)
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{:<28} {:<14} {:<14} {:<14} {}{}",
            r.script, loc, rpi, o2, status, suffix
        );
    }
}

/// Formats an optional 64-char SHA-256 digest into a 12-char short prefix or `"-"`.
fn short_hash(opt: Option<&str>) -> String {
    match opt {
        Some(h) if h.len() >= SHA256_SHORT_HEX_LEN => h[..SHA256_SHORT_HEX_LEN].to_string(),
        Some(h) => h.to_string(),
        None => "-".to_string(),
    }
}

/// Appends compact per-node artifact freshness lines.
fn render_freshness_section(out: &mut String, nodes: &[NodeProbeReport]) {
    let _ = writeln!(out, "ARTIFACT & QUEUE FRESHNESS:");
    for n in nodes.iter().filter(|n| n.reachable) {
        let items: Vec<String> = n.freshness.iter().map(format_freshness_badge).collect();
        let _ = writeln!(out, "  {:<9}: {}", n.node, items.join(" │ "));
    }
}

/// Formats one [`ArtifactFreshness`] entry as `<name>=absent` or `<name>=<age>(ok|STALE)`.
fn format_freshness_badge(f: &crate::commands::doctor::models::ArtifactFreshness) -> String {
    if !f.exists {
        return format!("{}=absent", f.name);
    }
    let age = format_compact_age(f.age_secs.unwrap_or(0));
    let tag = if f.stale { "STALE" } else { "ok" };
    format!("{}={}({})", f.name, age, tag)
}

/// Formats seconds into a compact human age (`45s`, `12m`, `6h`, `12d`).
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

/// Appends active node alerts and SHA-256 drift summaries when non-empty.
fn render_alerts_section(
    out: &mut String,
    nodes: &[NodeProbeReport],
    parity: &[ScriptParityEntry],
) {
    let has_node_alerts = nodes.iter().any(|n| !n.alerts.is_empty());
    let has_drift = parity.iter().any(|p| p.status == ParityStatus::Sha256Drift);
    if !has_node_alerts && !has_drift {
        let _ = writeln!(out, "\nACTIVE ALERTS: none (all probed nodes nominal)");
        return;
    }

    let _ = writeln!(out, "\nACTIVE ALERTS & DRIFT:");
    for p in parity
        .iter()
        .filter(|p| p.status == ParityStatus::Sha256Drift)
    {
        let detail = p.detail.as_deref().unwrap_or("diverging SHA-256");
        let _ = writeln!(
            out,
            "  [CRIT] [parity] SHA256_DRIFT on {}: {}",
            p.script, detail
        );
    }
    for n in nodes {
        for a in &n.alerts {
            let _ = writeln!(
                out,
                "  [{}] [{}:{}] {}",
                a.severity.badge(),
                n.node,
                a.component,
                a.summary
            );
        }
    }
}
