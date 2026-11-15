//! Cross-node SHA-256 script parity evaluator for `dbsrt doctor`.
//!
//! # Data Flow & Invariants
//! Compares SHA-256 digests of the 5 critical orchestration and worker scripts across `local`,
//! `rpi4`, and `o2switch`. Flags `ParityStatus::Sha256Drift` immediately if `local` is missing a
//! tracked script, if a mandatory remote target lacks the script, or if *any* reachable node holds
//! a SHA-256 digest that diverges from `local` (including stale leftover copies on optional nodes).

use crate::commands::doctor::constants::{TrackedScriptSpec, TRACKED_SCRIPTS};
use crate::commands::doctor::models::{NodeProbeReport, ParityStatus, ScriptParityEntry};

/// Evaluates cross-node SHA-256 parity across all [`TRACKED_SCRIPTS`].
///
/// # Arguments
/// * `local` - Probed [`NodeProbeReport`] for the `local` workspace.
/// * `rpi4` - Probed [`NodeProbeReport`] for the `rpi4` appliance.
/// * `o2switch` - Probed [`NodeProbeReport`] for the `o2switch` server.
///
/// # Returns
/// Vector of [`ScriptParityEntry`] rows in canonical [`TRACKED_SCRIPTS`] order.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// * When both `rpi4` and `o2switch` are skipped or unreachable and `local` has the script,
///   status is [`ParityStatus::LocalOnly`].
/// * When any reachable remote node has a present hash `!= local_sha`, or misses a script marked
///   `required_on_<node>`, status is [`ParityStatus::Sha256Drift`].
pub fn evaluate_script_parity(
    local: &NodeProbeReport,
    rpi4: &NodeProbeReport,
    o2switch: &NodeProbeReport,
) -> Vec<ScriptParityEntry> {
    TRACKED_SCRIPTS
        .iter()
        .map(|spec| evaluate_single_script(spec, local, rpi4, o2switch))
        .collect()
}

/// Evaluates SHA-256 parity for a single [`TrackedScriptSpec`].
fn evaluate_single_script(
    spec: &TrackedScriptSpec,
    local: &NodeProbeReport,
    rpi4: &NodeProbeReport,
    o2switch: &NodeProbeReport,
) -> ScriptParityEntry {
    let local_sha = local.script_hashes.get(spec.canonical_name).cloned();
    let rpi4_sha = rpi4.script_hashes.get(spec.canonical_name).cloned();
    let o2_sha = o2switch.script_hashes.get(spec.canonical_name).cloned();

    let Some(ref expected) = local_sha else {
        let is_standalone_optional = !rpi4.reachable
            && !o2switch.reachable
            && !local.script_hashes.is_empty()
            && !(spec.required_on_rpi4 && spec.required_on_o2switch);
        if is_standalone_optional {
            return ScriptParityEntry {
                script: spec.canonical_name.to_string(),
                local_sha256: None,
                rpi4_sha256: None,
                o2switch_sha256: None,
                status: ParityStatus::LocalOnly,
                detail: Some("optional script not deployed on standalone tier".to_string()),
            };
        }
        return ScriptParityEntry {
            script: spec.canonical_name.to_string(),
            local_sha256: None,
            rpi4_sha256: rpi4_sha,
            o2switch_sha256: o2_sha,
            status: ParityStatus::Sha256Drift,
            detail: Some("missing on local".to_string()),
        };
    };

    if !rpi4.reachable && !o2switch.reachable {
        return ScriptParityEntry {
            script: spec.canonical_name.to_string(),
            local_sha256: local_sha,
            rpi4_sha256: None,
            o2switch_sha256: None,
            status: ParityStatus::LocalOnly,
            detail: Some("remote nodes skipped or unreachable".to_string()),
        };
    }

    let mut reasons = Vec::new();
    check_remote_node_hash(
        expected,
        rpi4_sha.as_deref(),
        (rpi4.reachable, spec.required_on_rpi4),
        ("rpi4", &mut reasons),
    );
    check_remote_node_hash(
        expected,
        o2_sha.as_deref(),
        (o2switch.reachable, spec.required_on_o2switch),
        ("o2switch", &mut reasons),
    );

    let (status, detail) = if reasons.is_empty() {
        (ParityStatus::Ok, None)
    } else {
        (ParityStatus::Sha256Drift, Some(reasons.join("; ")))
    };

    ScriptParityEntry {
        script: spec.canonical_name.to_string(),
        local_sha256: local_sha,
        rpi4_sha256: rpi4_sha,
        o2switch_sha256: o2_sha,
        status,
        detail,
    }
}

/// Appends a drift reason if a reachable remote node diverges from `expected` or lacks a required file.
fn check_remote_node_hash(
    expected: &str,
    actual: Option<&str>,
    flags: (bool, bool),
    ctx: (&str, &mut Vec<String>),
) {
    let (reachable, required) = flags;
    let (node_name, reasons) = ctx;
    if !reachable {
        return;
    }
    match actual {
        Some(hash) if hash != expected => {
            reasons.push(format!("hash mismatch on {node_name}"));
        }
        None if required => {
            reasons.push(format!("missing on {node_name}"));
        }
        _ => {}
    }
}
