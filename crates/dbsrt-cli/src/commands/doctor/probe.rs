//! Local filesystem and bounded-timeout SSH node probes for `dbsrt doctor`.
//!
//! # Data Flow & Invariants
//! Collects [`RawNodeSnapshot`] payloads from either a local workspace directory (`probe_directory`)
//! or a remote SSH host (`probe_ssh_host`) in a single read-only SSH connection bounded by a hard
//! wall-clock deadline (`kill` + `wait` on timeout). Zero inline Python or multi-line shell scripts
//! are used; remote output sections are delimited by ASCII markers and parsed natively in Rust.

use crate::commands::doctor::analyzer::{analyze_node_snapshot, RawNodeSnapshot};
use crate::commands::doctor::constants::{
    DEFAULT_REMOTE_BASE_DIR, DOCTOR_EVENTS_TAIL_BYTES, DOCTOR_FAULTWATCH_TAIL_BYTES,
    DOCTOR_FAULTWATCH_TAIL_LINES, DOCTOR_LOG_TAIL_BYTES, MIN_SSH_CONNECT_TIMEOUT_SECS,
    SHA256_FULL_HEX_LEN, SSH_POLL_INTERVAL_MS, TAIL_TRUNCATION_MARGIN_BYTES,
    TRACKED_FRESHNESS_ARTIFACTS, TRACKED_SCRIPTS,
};
use crate::commands::doctor::models::{DoctorSeverity, NodeAlert, NodeProbeReport, NodeRole};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Prefix marker introducing a named section in the single-shot SSH probe stream.
const SSH_SECTION_PREFIX: &str = "___DBSRT_DOC_SEC:";

/// Suffix marker closing a named section header in the single-shot SSH probe stream.
const SSH_SECTION_SUFFIX: &str = "___";

/// Target specification for probing one cluster node.
///
/// # Data Flow & Invariants
/// Allows both production SSH probing (`SshHost`) and 100% hermetic directory-backed multi-node
/// tests (`LocalDir`), plus graceful skipping (`Skipped` for `--local-only`).
#[derive(Debug, Clone)]
pub enum NodeProbeTarget {
    /// Inspect files directly under a local directory path.
    LocalDir(PathBuf),
    /// Inspect `~/<remote_base>` over read-only non-interactive SSH.
    SshHost {
        /// SSH host alias or `user@host`.
        host: String,
        /// Relative directory beneath `$HOME` on the remote host (default `"dbsrt"`).
        remote_base: String,
    },
    /// Node explicitly skipped via `--local-only`.
    Skipped(String),
}

/// Executes a bounded probe against `target` and returns a structured [`NodeProbeReport`].
///
/// # Arguments
/// * `role` - Cluster tier (`Local`, `Rpi4`, `O2switch`).
/// * `target` - [`NodeProbeTarget`] configuration.
/// * `timeout_secs` - Hard wall-clock timeout in seconds for remote SSH execution.
/// * `fallback_now_secs` - Local Unix epoch timestamp used for local directories or clock fallback.
///
/// # Returns
/// [`NodeProbeReport`] (gracefully degraded with `reachable: false` on SSH failure/timeout).
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Remote transport failures emit a `Warning` (`node_unreachable`) rather than panicking or aborting
/// the parallel probe across the remaining nodes.
pub fn probe_node(
    role: NodeRole,
    target: &NodeProbeTarget,
    timeout_secs: u64,
    fallback_now_secs: u64,
) -> NodeProbeReport {
    let t0 = Instant::now();
    match target {
        NodeProbeTarget::Skipped(label) => build_skipped_report(role, label),
        NodeProbeTarget::LocalDir(dir) => {
            let snap = probe_directory(dir, fallback_now_secs);
            let ms = t0.elapsed().as_millis() as u64;
            analyze_node_snapshot(role, dir.display().to_string(), ms, snap)
        }
        NodeProbeTarget::SshHost { host, remote_base } => probe_ssh_target(
            role,
            (host, remote_base),
            (timeout_secs, fallback_now_secs),
            t0,
        ),
    }
}

/// Probes an SSH target and wraps either the analyzed snapshot or a graceful unreachable report.
fn probe_ssh_target(
    role: NodeRole,
    ssh: (&str, &str),
    clocks: (u64, u64),
    started_at: Instant,
) -> NodeProbeReport {
    let (host, remote_base) = ssh;
    let (timeout_secs, fallback_now_secs) = clocks;
    match probe_ssh_host(host, remote_base, timeout_secs, fallback_now_secs) {
        Ok(snap) => {
            let ms = started_at.elapsed().as_millis() as u64;
            analyze_node_snapshot(role, host.to_string(), ms, snap)
        }
        Err(err) => {
            let ms = started_at.elapsed().as_millis() as u64;
            build_unreachable_report(role, host, ms, err)
        }
    }
}

/// Builds a placeholder [`NodeProbeReport`] for a `--local-only` skipped remote node.
fn build_skipped_report(role: NodeRole, label: &str) -> NodeProbeReport {
    NodeProbeReport {
        node: role.as_str().to_string(),
        target: label.to_string(),
        reachable: false,
        skipped: true,
        probe_ms: 0,
        status: DoctorSeverity::Ok,
        probe_error: None,
        daemon_running: None,
        supervisor_state: None,
        batch_aborted: false,
        failed_queue_targets: 0,
        blacklisted_stems: 0,
        lve_faults_24h: 0,
        components: Vec::new(),
        freshness: Vec::new(),
        alerts: Vec::new(),
        script_hashes: BTreeMap::new(),
    }
}

/// Builds a gracefully degraded [`NodeProbeReport`] when an SSH node is unreachable or times out.
fn build_unreachable_report(
    role: NodeRole,
    host: &str,
    probe_ms: u64,
    err: String,
) -> NodeProbeReport {
    let redacted_err = dbsrt_core::redact_secrets(&err);
    NodeProbeReport {
        node: role.as_str().to_string(),
        target: host.to_string(),
        reachable: false,
        skipped: false,
        probe_ms,
        status: DoctorSeverity::Warning,
        probe_error: Some(redacted_err.clone()),
        daemon_running: None,
        supervisor_state: None,
        batch_aborted: false,
        failed_queue_targets: 0,
        blacklisted_stems: 0,
        lve_faults_24h: 0,
        components: Vec::new(),
        freshness: Vec::new(),
        alerts: vec![NodeAlert {
            id: "node_unreachable".to_string(),
            severity: DoctorSeverity::Warning,
            component: "ssh".to_string(),
            summary: format!("Node '{host}' unreachable: {redacted_err}"),
        }],
        script_hashes: BTreeMap::new(),
    }
}

/// Collects a [`RawNodeSnapshot`] directly from a local workspace or test directory.
///
/// # Arguments
/// * `root` - Base directory containing `scripts/`, `run/`, `logs/`, and `data/reports/`.
/// * `now_unix_secs` - Reference Unix timestamp in seconds.
///
/// # Returns
/// Populated [`RawNodeSnapshot`].
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Resolves each tracked script via `local_repo_path` first (`scripts/workers/...`, `scripts/infra/...`)
/// and falls back to flat `deployed_rel_path` (`scripts/...`) so both Git workspaces and flat
/// appliance/test directories work seamlessly.
pub fn probe_directory(root: &Path, now_unix_secs: u64) -> RawNodeSnapshot {
    let mut script_hashes = BTreeMap::new();
    for spec in TRACKED_SCRIPTS {
        let primary = root.join(spec.local_repo_path);
        let fallback = root.join(spec.deployed_rel_path);
        let chosen = if primary.is_file() { primary } else { fallback };
        if let Some(hex) = compute_file_sha256(&chosen) {
            script_hashes.insert(spec.canonical_name.to_string(), hex);
        }
    }

    let mut artifact_mtimes = BTreeMap::new();
    for (_, rel, _) in TRACKED_FRESHNESS_ARTIFACTS {
        if let Some(mt) = read_mtime_secs(&root.join(rel)) {
            artifact_mtimes.insert(rel.to_string(), mt);
        }
    }

    let mut events_tails = Vec::new();
    for rel in [
        "logs/batch_events.jsonl",
        "data/reports/batch_tvshow_nightly.jsonl",
    ] {
        if let Some(t) = read_local_tail(&root.join(rel), DOCTOR_EVENTS_TAIL_BYTES) {
            events_tails.push(t);
        }
    }

    RawNodeSnapshot {
        now_unix_secs,
        daemon_running: None,
        script_hashes,
        artifact_mtimes,
        supervisor_json: fs::read_to_string(root.join("run/supervisor_status.json")).ok(),
        batch_queue_json: fs::read_to_string(root.join("run/batch_queue.json")).ok(),
        batch_aborted_text: fs::read_to_string(root.join("run/batch_aborted")).ok(),
        server_log_tail: read_local_tail(
            &root.join("logs/dbsrt-server.log"),
            DOCTOR_LOG_TAIL_BYTES,
        ),
        worker_log_tail: read_local_tail(
            &root.join("logs/batch_worker.log"),
            DOCTOR_LOG_TAIL_BYTES,
        ),
        supervisor_log_tail: read_local_tail(
            &root.join("logs/rpi4_supervisor.log"),
            DOCTOR_LOG_TAIL_BYTES,
        ),
        batch_events_tails: events_tails,
        faultwatch_tail: read_local_faultwatch_dir(&root.join("logs/faultwatch")),
    }
}

/// Computes the lowercase 64-char SHA-256 hex digest of a local file.
pub fn compute_file_sha256(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(SHA256_FULL_HEX_LEN);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Some(hex)
}

/// Reads modification timestamp in Unix epoch seconds for `path`.
fn read_mtime_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Reads at most `max_bytes` from the end of `path`, dropping any torn leading partial line.
fn read_local_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let truncated = len > max_bytes;
    if truncated {
        f.seek(SeekFrom::Start(len - max_bytes)).ok()?;
    }
    let mut buf = Vec::with_capacity(len.min(max_bytes) as usize);
    f.read_to_end(&mut buf).ok()?;
    let slice = if truncated {
        buf.iter()
            .position(|&b| b == b'\n')
            .map_or(&buf[..], |idx| &buf[idx + 1..])
    } else {
        &buf[..]
    };
    Some(String::from_utf8_lossy(slice).into_owned())
}

/// Concatenates bounded tails from files inside `logs/faultwatch/`.
fn read_local_faultwatch_dir(dir: &Path) -> Option<String> {
    let entries = fs::read_dir(dir).ok()?;
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let mut combined = String::new();
    for p in paths {
        if let Some(tail) = read_local_tail(&p, DOCTOR_FAULTWATCH_TAIL_BYTES) {
            combined.push_str(&tail);
            combined.push('\n');
        }
    }
    (!combined.is_empty()).then_some(combined)
}

/// Probes a remote node over a single bounded-timeout read-only SSH invocation.
fn probe_ssh_host(
    host: &str,
    remote_base: &str,
    timeout_secs: u64,
    fallback_now_secs: u64,
) -> Result<RawNodeSnapshot, String> {
    let connect_timeout = timeout_secs.max(MIN_SSH_CONNECT_TIMEOUT_SECS);
    let base = if remote_base.is_empty() {
        DEFAULT_REMOTE_BASE_DIR
    } else {
        remote_base
    };
    let remote_cmd = build_remote_posix_probe_command(base);

    let child = Command::new("ssh")
        .args([
            "-C",
            "-o",
            "BatchMode=yes",
            "-o",
            &format!("ConnectTimeout={connect_timeout}"),
            host,
            &remote_cmd,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn ssh: {e}"))?;

    let output = wait_child_bounded(child, Duration::from_secs(connect_timeout))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("ssh exited with {}", output.status)
        } else {
            stderr
        };
        return Err(detail);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_ssh_probe_sections(&stdout, fallback_now_secs))
}

/// Builds the single-line read-only POSIX command list executed over SSH.
fn build_remote_posix_probe_command(base: &str) -> String {
    let scripts_arg = TRACKED_SCRIPTS
        .iter()
        .map(|s| s.deployed_rel_path)
        .collect::<Vec<_>>()
        .join(" ");
    let artifacts_arg = TRACKED_FRESHNESS_ARTIFACTS
        .iter()
        .map(|(_, rel, _)| *rel)
        .collect::<Vec<_>>()
        .join(" ");
    let sec = |name: &str| format!("printf '\\n{SSH_SECTION_PREFIX}{name}{SSH_SECTION_SUFFIX}\\n'");

    [
        format!("cd \"$HOME/{base}\" 2>/dev/null || exit 0"),
        sec("NOW"),
        "date +%s 2>/dev/null || true".to_string(),
        sec("DAEMON"),
        "pgrep -cf '[d]bsrt-server' 2>/dev/null || true".to_string(),
        sec("SHA256"),
        format!("sha256sum {scripts_arg} 2>/dev/null || true"),
        sec("STAT"),
        format!("stat -c '%Y|%n' {artifacts_arg} 2>/dev/null || true"),
        sec("SUPERVISOR"),
        "cat run/supervisor_status.json 2>/dev/null || true".to_string(),
        sec("QUEUE"),
        "cat run/batch_queue.json 2>/dev/null || true".to_string(),
        sec("ABORTED"),
        "cat run/batch_aborted 2>/dev/null || true".to_string(),
        sec("LOG_SERVER"),
        format!("tail -c {DOCTOR_LOG_TAIL_BYTES} logs/dbsrt-server.log 2>/dev/null || true"),
        sec("LOG_WORKER"),
        format!("tail -c {DOCTOR_LOG_TAIL_BYTES} logs/batch_worker.log 2>/dev/null || true"),
        sec("LOG_SUPERVISOR"),
        format!("tail -c {DOCTOR_LOG_TAIL_BYTES} logs/rpi4_supervisor.log 2>/dev/null || true"),
        sec("EVENTS_ALT"),
        format!("tail -c {DOCTOR_EVENTS_TAIL_BYTES} logs/batch_events.jsonl 2>/dev/null || true"),
        sec("EVENTS_MAIN"),
        format!(
            "tail -c {DOCTOR_EVENTS_TAIL_BYTES} data/reports/batch_tvshow_nightly.jsonl 2>/dev/null || true"
        ),
        sec("FAULTWATCH"),
        format!(
            "tail -q -n {DOCTOR_FAULTWATCH_TAIL_LINES} logs/faultwatch/*.jsonl 2>/dev/null || true"
        ),
    ]
    .join("; ")
}

/// Waits for `child` up to `timeout`, draining stdout/stderr on helper threads to prevent pipe deadlocks.
fn wait_child_bounded(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let out_handle = spawn_pipe_drain(child.stdout.take());
    let err_handle = spawn_pipe_drain(child.stderr.take());
    let status = poll_child_until_deadline(&mut child, timeout)?;
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Spawns a background thread draining an optional child stdio pipe into a `Vec<u8>`.
fn spawn_pipe_drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut reader) = pipe {
            let _ = reader.read_to_end(&mut buf);
        }
        buf
    })
}

/// Polls `child.try_wait()` until completion or `timeout`, killing the child if the deadline expires.
fn poll_child_until_deadline(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(st) = child.try_wait().map_err(|e| format!("wait error: {e}"))? {
            return Ok(st);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("SSH probe timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(SSH_POLL_INTERVAL_MS));
    }
}

/// Parses delimited section output from the remote SSH probe into a [`RawNodeSnapshot`].
pub fn parse_ssh_probe_sections(raw: &str, fallback_now_secs: u64) -> RawNodeSnapshot {
    let mut sections: BTreeMap<String, String> = BTreeMap::new();
    let mut current_sec: Option<String> = None;

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix(SSH_SECTION_PREFIX) {
            if let Some(name) = rest.strip_suffix(SSH_SECTION_SUFFIX) {
                current_sec = Some(name.to_string());
                sections.entry(name.to_string()).or_default();
                continue;
            }
        }
        if let Some(ref sec) = current_sec {
            let buf = sections.entry(sec.clone()).or_default();
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
        }
    }

    let now_unix_secs = sections
        .get("NOW")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(fallback_now_secs);
    let daemon_running = sections
        .get("DAEMON")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|n| n > 0);

    let script_hashes = parse_sha256_section(sections.get("SHA256").map(String::as_str));
    let artifact_mtimes = parse_stat_section(sections.get("STAT").map(String::as_str));

    let mut events_tails = Vec::new();
    for key in ["EVENTS_ALT", "EVENTS_MAIN"] {
        if let Some(t) = non_empty_tail(sections.get(key), DOCTOR_EVENTS_TAIL_BYTES as usize) {
            events_tails.push(t);
        }
    }

    RawNodeSnapshot {
        now_unix_secs,
        daemon_running,
        script_hashes,
        artifact_mtimes,
        supervisor_json: non_empty_raw(sections.get("SUPERVISOR")),
        batch_queue_json: non_empty_raw(sections.get("QUEUE")),
        batch_aborted_text: non_empty_raw(sections.get("ABORTED")),
        server_log_tail: non_empty_tail(sections.get("LOG_SERVER"), DOCTOR_LOG_TAIL_BYTES as usize),
        worker_log_tail: non_empty_tail(sections.get("LOG_WORKER"), DOCTOR_LOG_TAIL_BYTES as usize),
        supervisor_log_tail: non_empty_tail(
            sections.get("LOG_SUPERVISOR"),
            DOCTOR_LOG_TAIL_BYTES as usize,
        ),
        batch_events_tails: events_tails,
        faultwatch_tail: non_empty_raw(sections.get("FAULTWATCH")),
    }
}

/// Parses `sha256sum` lines (`<64-hex>  scripts/<name>`) into canonical script hash entries.
fn parse_sha256_section(sec: Option<&str>) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(text) = sec else {
        return map;
    };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(rel_path)) = (parts.next(), parts.next()) else {
            continue;
        };
        if hash.len() != SHA256_FULL_HEX_LEN || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let clean_path = rel_path.trim_start_matches('*');
        if let Some(spec) = TRACKED_SCRIPTS
            .iter()
            .find(|s| s.deployed_rel_path == clean_path)
        {
            map.insert(spec.canonical_name.to_string(), hash.to_ascii_lowercase());
        }
    }
    map
}

/// Parses `stat -c '%Y|%n'` lines into relative path -> mtime Unix seconds.
fn parse_stat_section(sec: Option<&str>) -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    let Some(text) = sec else {
        return map;
    };
    for line in text.lines() {
        let Some((mtime_str, path_str)) = line.trim().split_once('|') else {
            continue;
        };
        if let Ok(mt) = mtime_str.parse::<u64>() {
            map.insert(path_str.trim().to_string(), mt);
        }
    }
    map
}

/// Returns `Some(trimmed)` when `opt` is non-empty.
fn non_empty_raw(opt: Option<&String>) -> Option<String> {
    let s = opt?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Strips a potentially torn leading partial line when `tail -c` hit `max_bytes`.
fn non_empty_tail(opt: Option<&String>, max_bytes: usize) -> Option<String> {
    let s = opt?;
    if s.trim().is_empty() {
        return None;
    }
    if s.len() >= max_bytes.saturating_sub(TAIL_TRUNCATION_MARGIN_BYTES) {
        if let Some((_, after_nl)) = s.split_once('\n') {
            return (!after_nl.trim().is_empty()).then(|| after_nl.to_string());
        }
    }
    Some(s.clone())
}
