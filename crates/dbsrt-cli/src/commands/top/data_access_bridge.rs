//! Non-blocking bridge connecting `dbsrt top` to the Python SSOT `scripts/data_access` resolver.
//!
//! # Data Flow & Invariants
//! Executes `scripts/data_access/cli.py` inside bounded background threads (never on the TUI render
//! thread), keeping all network, FTP MLSD, HTTPS `o2switch` bounce, and CIFS safety rules in one
//! single source of truth.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use dbsrt_tui::model::{
    ClusterChannelBadge, ClusterCorpusEntry, ClusterDataTopology, DiscoveredMediaItem,
    IngestionStatus, MediaSourceConfig, MediaSubtrackInfo, PhashFingerprintInfo, SystemTelemetry,
};
use serde_json::Value;

/// Relative path from repository root to the SSOT data-access CLI entrypoint.
const DATA_ACCESS_CLI_REL_PATH: &str = "scripts/data_access/cli.py";

/// Relative cache file under `$HOME` holding the latest `dbsrt_data.py` status snapshot.
const DATA_ACCESS_CACHE_REL_PATH: &str = ".cache/dbsrt/data_access_cache.json";

/// Maximum wait time in seconds for bounded `scripts/data_access/cli.py` subprocess execution.
const DATA_ACCESS_CMD_TIMEOUT_SECS: u64 = 18;

/// Poll interval in milliseconds when waiting on bounded subprocess completion.
const SUBPROCESS_POLL_STEP_MS: u64 = 50;

/// GiB conversion factor for human-readable corpus size strings.
const BYTES_PER_GIB_F64: f64 = 1024.0 * 1024.0 * 1024.0;

/// MiB conversion factor for human-readable corpus size strings.
const BYTES_PER_MIB_F64: f64 = 1024.0 * 1024.0;

/// Reads `network.cifs_safe_to_stat` from the local SWR data-access cache if present.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Some(bool)` when the cache JSON contains `network.cifs_safe_to_stat`, else `None`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Zero network I/O and zero CIFS `stat()` calls; reads only local `$HOME/.cache/dbsrt/...`.
pub fn cached_cifs_safe_to_stat() -> Option<bool> {
    let home = std::env::var_os("HOME")?;
    let cache_path = PathBuf::from(home).join(DATA_ACCESS_CACHE_REL_PATH);
    let raw = std::fs::read_to_string(cache_path).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    val.get("network")?.get("cifs_safe_to_stat")?.as_bool()
}

/// Probes workstation network topology, channels, and corpus deltas via `scripts/data_access/cli.py`.
///
/// # Arguments
/// None.
///
/// # Returns
/// `Some(ClusterDataTopology)` on success, or `None` when the CLI script is unavailable or fails.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Runs `python3 scripts/data_access/cli.py status --compact` with a bounded timeout so remote
/// cluster probes never hang indefinitely.
pub fn probe_data_access_topology() -> Option<ClusterDataTopology> {
    let raw = run_data_access_cli(&["status", "--compact"]).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    parse_topology_from_status_json(&val)
}

/// Spawns a background thread querying remote SSOT media families (`freebox`, `video`, `tvshow`, `snippets`).
///
/// # Arguments
/// * `query` - User search string entered in `MediaBrowser` (`[/]`).
/// * `shared_state` - Thread-safe handle to live TUI telemetry state.
///
/// # Returns
/// Immediate toast feedback message for the UI footer.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Rejects empty queries without spawning a subprocess; merges hits into `state.discovered_media`
/// and registers virtual remote `MediaSourceConfig` rows in memory only (never persisted to disk).
pub fn spawn_remote_media_query(
    query: String,
    shared_state: Arc<RwLock<SystemTelemetry>>,
) -> String {
    let trimmed = query.trim().to_string();
    if trimmed.is_empty() {
        return "Saisissez d'abord un mot-clé avec [/] puis [W] pour rechercher à distance."
            .to_string();
    }
    let toast = format!("🔍 Recherche SSOT distante lancée pour '{trimmed}'...");
    std::thread::spawn(move || {
        let specs = [
            format!("freebox:{trimmed}"),
            format!("video:{trimmed}"),
            format!("tvshow:{trimmed}"),
            format!("snippets:{trimmed}"),
        ];
        let args = [
            "query",
            "--compact",
            &specs[0],
            &specs[1],
            &specs[2],
            &specs[3],
        ];
        if let Ok(raw) = run_data_access_cli(&args) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                let items = parse_query_items_from_json(&val);
                merge_remote_items_into_telemetry(&shared_state, items);
            }
        }
    });
    toast
}

/// Spawns a background `ffprobe` inspection call (`dbsrt_data.py inspect`) for an incomplete item.
///
/// # Arguments
/// * `item` - Selected candidate media item missing duration, codec, or subtitle streams.
/// * `shared_state` - Thread-safe handle to live TUI telemetry state.
///
/// # Returns
/// `Some(toast)` when background inspection starts, or `None` when the item's source has no mapping.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Maps `source_id` (`"freebox"`, `"video"`, `"tvshow"`, `"snippets"`) and executes `inspect --compact`
/// asynchronously so `MediaInspection` refreshes smoothly upon completion.
pub fn spawn_remote_media_inspect(
    item: &DiscoveredMediaItem,
    shared_state: Arc<RwLock<SystemTelemetry>>,
) -> Option<String> {
    let family = map_source_id_to_inspect_family(item.source_id.as_deref())?;
    let target = item.path.to_string_lossy().to_string();
    let stem = item.stem.clone();
    std::thread::spawn(move || execute_remote_inspect_job(family, &target, &shared_state));
    Some(format!(
        "🔎 Inspection ffprobe distante lancée pour '{stem}'..."
    ))
}

/// Runs `dbsrt_data.py inspect --compact` and writes parsed streams back into `shared_state`.
fn execute_remote_inspect_job(
    family: &str,
    target: &str,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
) {
    let args = ["inspect", "--compact", family, target];
    let Ok(raw) = run_data_access_cli(&args) else {
        return;
    };
    let Ok(val) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    let Ok(mut state) = shared_state.write() else {
        return;
    };
    if let Some(found) = state
        .discovered_media
        .iter_mut()
        .find(|m| m.path.to_string_lossy() == target)
    {
        apply_inspect_json_to_item(found, &val);
    }
}

/// Maps a TUI `MediaSourceConfig.id` onto the canonical `dbsrt_data.py inspect` family alias.
fn map_source_id_to_inspect_family(source_id: Option<&str>) -> Option<&'static str> {
    match source_id? {
        "freebox" | "freebox-disks" => Some("freebox"),
        "video" | "nas_video" | "synology-video" => Some("video"),
        "tvshow" | "nas_tvshow" | "synology-tvshow" => Some("tvshow"),
        "snippets" | "local-snippets" => Some("snippets"),
        _ => None,
    }
}

/// Relative path to repository virtualenv Python interpreter (fallback: system `python3`).
const VENV_PYTHON_REL_PATH: &str = ".venv/bin/python";

/// Executes `python` on `scripts/data_access/cli.py <args...>` with a bounded wall-clock timeout.
fn run_data_access_cli(args: &[&str]) -> Result<String, String> {
    if !Path::new(DATA_ACCESS_CLI_REL_PATH).is_file() {
        return Err("data_access CLI absent".to_string());
    }
    let py_bin = if Path::new(VENV_PYTHON_REL_PATH).is_file() {
        VENV_PYTHON_REL_PATH
    } else {
        "python3"
    };
    let mut cmd = Command::new(py_bin);
    cmd.arg(DATA_ACCESS_CLI_REL_PATH)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(DATA_ACCESS_CMD_TIMEOUT_SECS);
    wait_bounded_child(child, deadline)
}

/// Polls a spawned child process until completion or `deadline`, killing it on timeout.
fn wait_bounded_child(mut child: std::process::Child, deadline: Instant) -> Result<String, String> {
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let out = child.wait_with_output().map_err(|e| e.to_string())?;
            if status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
            }
            return Err(format!("exit {:?}", status.code()));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timeout".to_string());
        }
        std::thread::sleep(Duration::from_millis(SUBPROCESS_POLL_STEP_MS));
    }
}

/// Parses `dbsrt_data.py status --compact` JSON into [`ClusterDataTopology`].
///
/// # Arguments
/// * `val` - Parsed JSON root object emitted by `scripts/data_access/cli.py status`.
///
/// # Returns
/// `Some(ClusterDataTopology)` when `network` metadata is present, else `None`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Extracts network profile, CIFS safety guard, channel statuses, corpus deltas, and backup state.
pub fn parse_topology_from_status_json(val: &Value) -> Option<ClusterDataTopology> {
    let net = val.get("network")?;
    let channels = parse_channel_badges(val.get("channels"));
    let corpus = parse_corpus_entries(val.get("freshness"));
    let backup = val.get("backup_state");

    Some(ClusterDataTopology {
        network_profile: json_str(net, "profile").unwrap_or_else(|| "unknown".to_string()),
        host_env: json_str(net, "host_env").unwrap_or_else(|| "workstation".to_string()),
        cifs_safe_to_stat: net
            .get("cifs_safe_to_stat")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        channels,
        corpus,
        backup_status: backup.and_then(|b| json_str(b, "status")),
        backup_snapshot: backup.and_then(|b| json_str(b, "latest_snapshot_id")),
        backup_replicas: backup
            .and_then(|b| b.get("verified_replicas"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        evicted_count: backup
            .and_then(|b| b.get("evicted_datasets"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
    })
}

/// Converts JSON `channels` map into sorted [`ClusterChannelBadge`] items.
fn parse_channel_badges(channels_val: Option<&Value>) -> Vec<ClusterChannelBadge> {
    let Some(obj) = channels_val.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut badges: Vec<ClusterChannelBadge> = obj
        .iter()
        .map(|(id, info)| {
            let accessible = info
                .get("accessible")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let error = json_str(info, "error");
            let state_tag = if accessible {
                "OK"
            } else if error.as_deref().is_some_and(|e| e.starts_with("skipped_")) {
                "SKIP"
            } else if error.as_deref() == Some("blocked_by_stale_cifs_hang_guard") {
                "GUARD"
            } else {
                "OFF"
            };
            ClusterChannelBadge {
                id: id.clone(),
                label: json_str(info, "label").unwrap_or_else(|| id.clone()),
                accessible,
                latency_ms: info
                    .get("latency_ms")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                state_tag: state_tag.to_string(),
                error,
            }
        })
        .collect();
    badges.sort_by(|a, b| a.id.cmp(&b.id));
    badges
}

/// Paired JSON node snapshots for local workstation and authoritative `o2switch`.
struct CorpusNodeViews<'a> {
    local: Option<&'a Value>,
    o2: Option<&'a Value>,
}

/// Converts JSON `freshness` summary into compact [`ClusterCorpusEntry`] rows.
fn parse_corpus_entries(freshness: Option<&Value>) -> Vec<ClusterCorpusEntry> {
    let Some(fresh) = freshness else {
        return Vec::new();
    };
    let nodes = fresh.get("nodes");
    let views = CorpusNodeViews {
        local: nodes.and_then(|n| n.get("local")),
        o2: nodes.and_then(|n| n.get("o2switch")),
    };

    let mut rows = Vec::new();
    if let Some(cat) = fresh.get("packed_catalog") {
        rows.push(build_catalog_corpus_row(
            "catalog.dbphash",
            "catalog_dbphash",
            &views,
            cat,
        ));
    }
    if let Some(acat) = fresh.get("packed_ahash_catalog") {
        rows.push(build_catalog_corpus_row(
            "catalog.dbahash",
            "catalog_dbahash",
            &views,
            acat,
        ));
    }
    for (label, corp_key, field) in [
        ("Corpus .phash", "phash_corpus", "phash_files"),
        ("Corpus .ahash", "ahash_corpus", "ahash_files"),
    ] {
        if let Some(corp) = fresh.get(corp_key) {
            rows.push(build_files_corpus_row(label, field, &views, corp));
        }
    }
    rows
}

/// Formats one packed binary catalog comparison row (`catalog.dbphash` / `catalog.dbahash`).
fn build_catalog_corpus_row(
    label: &str,
    field: &str,
    views: &CorpusNodeViews<'_>,
    summary: &Value,
) -> ClusterCorpusEntry {
    let delta_m = summary
        .get("delta_medias")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let status = json_str(summary, "status").unwrap_or_else(|| "UNKNOWN".to_string());
    let delta_summary = if delta_m > 0 {
        format!("+{delta_m} médias")
    } else {
        "synchro".to_string()
    };
    ClusterCorpusEntry {
        label: label.to_string(),
        local_summary: format_node_catalog(views.local.and_then(|n| n.get(field))),
        remote_summary: format_node_catalog(views.o2.and_then(|n| n.get(field))),
        status,
        delta_summary,
    }
}

/// Formats one corpus directory file-count row (`.phash` / `.ahash`).
fn build_files_corpus_row(
    label: &str,
    field: &str,
    views: &CorpusNodeViews<'_>,
    corp: &Value,
) -> ClusterCorpusEntry {
    let miss = corp
        .get("missing_locally")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let status = json_str(corp, "status").unwrap_or_else(|| "UNKNOWN".to_string());
    let delta_summary = if miss > 0 {
        format!("-{miss} local")
    } else {
        "complet".to_string()
    };
    let loc_cnt = views
        .local
        .and_then(|n| n.get(field))
        .and_then(|f| f.get("count"))
        .and_then(Value::as_u64)
        .map_or_else(|| "-".to_string(), |c| format!("{c} fich."));
    let rem_cnt = views
        .o2
        .and_then(|n| n.get(field))
        .and_then(|f| f.get("count"))
        .and_then(Value::as_u64)
        .map_or_else(|| "-".to_string(), |c| format!("{c} fich."));

    ClusterCorpusEntry {
        label: label.to_string(),
        local_summary: loc_cnt,
        remote_summary: rem_cnt,
        status,
        delta_summary,
    }
}

/// Formats `media_count` and human byte size for a catalog node block.
fn format_node_catalog(entry: Option<&Value>) -> String {
    let Some(ent) = entry else {
        return "-".to_string();
    };
    if !ent.get("exists").and_then(Value::as_bool).unwrap_or(true) {
        return "absent".to_string();
    }
    let cnt = ent.get("media_count").and_then(Value::as_u64).unwrap_or(0);
    let bytes = ent.get("size_bytes").and_then(Value::as_u64).unwrap_or(0);
    let size_txt = if bytes >= BYTES_PER_GIB_F64 as u64 {
        format!("{:.2}GiB", bytes as f64 / BYTES_PER_GIB_F64)
    } else {
        format!("{:.0}MiB", bytes as f64 / BYTES_PER_MIB_F64)
    };
    format!("{cnt} ({size_txt})")
}

/// Parses `dbsrt_data.py query --compact` JSON hits into [`DiscoveredMediaItem`] records.
///
/// # Arguments
/// * `val` - Parsed JSON output emitted by `scripts/data_access/cli.py query`.
///
/// # Returns
/// Vector of [`DiscoveredMediaItem`] ready for `MediaBrowser` display.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Maps each `spec` prefix (`freebox:`, `video:`, `tvshow:`, `snippets:`) onto `source_id` and
/// assigns initial [`IngestionStatus`] from `coupled_srt` / `phash_file` presence.
pub fn parse_query_items_from_json(val: &Value) -> Vec<DiscoveredMediaItem> {
    let Some(results) = val.get("results").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for (spec, entry) in results {
        let source_id = spec.split(':').next().unwrap_or("remote").to_string();
        let Some(matches) = entry
            .get("query_result")
            .and_then(|qr| qr.get("matches"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for hit in matches {
            if let Some(item) = map_query_hit_to_item(&source_id, hit) {
                items.push(item);
            }
        }
    }
    items
}

/// Converts a single `query` match JSON object into a [`DiscoveredMediaItem`].
fn map_query_hit_to_item(source_id: &str, hit: &Value) -> Option<DiscoveredMediaItem> {
    let rel_path = json_str(hit, "path").or_else(|| json_str(hit, "filename"))?;
    let fname = json_str(hit, "filename").unwrap_or_else(|| rel_path.clone());
    let stem = Path::new(&fname)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(fname);
    let size_bytes = hit.get("size_bytes").and_then(Value::as_u64).unwrap_or(0);
    let has_phash = json_str(hit, "phash_file").is_some()
        || hit
            .get("sidecars_present")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(".phash")));
    let coupled = hit
        .get("coupled_srt")
        .or_else(|| hit.get("parent_media_coupled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let phash = if has_phash {
        Some(PhashFingerprintInfo {
            exists: true,
            frame_count: 0,
            size_bytes: 0,
        })
    } else {
        None
    };
    let status = match (has_phash, coupled) {
        (true, true) => IngestionStatus::Completed,
        (true, false) => IngestionStatus::PartialFail,
        _ => IngestionStatus::Virgin,
    };

    Some(DiscoveredMediaItem {
        path: PathBuf::from(rel_path),
        stem,
        size_bytes,
        duration_secs: None,
        video_codec: None,
        subtitles: Vec::new(),
        phash,
        source_id: Some(source_id.to_string()),
        status,
    })
}

/// Applies `dbsrt_data.py inspect --compact` JSON metadata onto an existing [`DiscoveredMediaItem`].
///
/// # Arguments
/// * `item` - Mutable media item to enrich in place.
/// * `val` - Parsed JSON output emitted by `scripts/data_access/cli.py inspect`.
///
/// # Returns
/// `true` when `val.ok == true` and metadata was applied, else `false`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Populates `duration_secs`, `video_codec`, and `subtitles` (`MediaSubtrackInfo`) without wiping
/// already-populated local fields when missing from JSON.
pub fn apply_inspect_json_to_item(item: &mut DiscoveredMediaItem, val: &Value) -> bool {
    if !val.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    if let Some(dur) = val.get("duration_sec").and_then(Value::as_f64) {
        item.duration_secs = Some(dur);
    }
    if let Some(codec) = val.get("video").and_then(|v| json_str(v, "codec")) {
        item.video_codec = Some(codec);
    }
    if let Some(size) = val.get("size_bytes").and_then(Value::as_u64) {
        if size > 0 {
            item.size_bytes = size;
        }
    }
    if let Some(subs) = val.get("subtitle_streams").and_then(Value::as_array) {
        item.subtitles = subs
            .iter()
            .enumerate()
            .map(|(idx, s)| MediaSubtrackInfo {
                index: s.get("index").and_then(Value::as_u64).unwrap_or(idx as u64) as usize,
                lang: json_str(s, "language").unwrap_or_else(|| "und".to_string()),
                codec: json_str(s, "codec").unwrap_or_else(|| "subrip".to_string()),
                title: json_str(s, "title").unwrap_or_default(),
            })
            .collect();
    }
    true
}

/// Merges remote SSOT items into `shared_state` and ensures virtual remote sources are enabled.
fn merge_remote_items_into_telemetry(
    shared_state: &Arc<RwLock<SystemTelemetry>>,
    new_items: Vec<DiscoveredMediaItem>,
) {
    if let Ok(mut state) = shared_state.write() {
        for item in new_items {
            if let Some(src_id) = item.source_id.as_deref() {
                ensure_virtual_remote_source(&mut state.media_sources, src_id);
            }
            if let Some(existing) = state
                .discovered_media
                .iter_mut()
                .find(|m| m.path == item.path)
            {
                *existing = item;
            } else {
                state.discovered_media.push(item);
            }
        }
        state.is_remote_streaming = true;
    }
}

/// Registers an in-memory enabled [`MediaSourceConfig`] for remote SSOT query results.
fn ensure_virtual_remote_source(sources: &mut Vec<MediaSourceConfig>, source_id: &str) {
    if sources.iter().any(|s| s.id == source_id) {
        return;
    }
    let display = match source_id {
        "freebox" | "video" | "tvshow" | "snippets" => source_id,
        _ => source_id,
    };
    sources.push(MediaSourceConfig {
        id: source_id.to_string(),
        name: display.to_string(),
        path: PathBuf::from(format!("ssot://{source_id}")),
        enabled: true,
        media_count: 0,
        last_scan_added: 0,
        last_scan_iso: None,
    });
}

/// Extracts a non-empty string field from a `serde_json::Value` object.
fn json_str(val: &Value, key: &str) -> Option<String> {
    val.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_topology_from_status_json_extracts_channels_and_corpus() {
        let raw = r#"{
            "network":{"profile":"work_vpn","host_env":"workstation","cifs_safe_to_stat":false},
            "channels":{
                "https_o2switch":{"label":"HTTPS o2switch API","accessible":true,"latency_ms":85.1},
                "local_cifs":{"label":"Local CIFS","accessible":false,"latency_ms":0.0,"error":"blocked_by_stale_cifs_hang_guard"},
                "ssh_rpi4":{"label":"SSH rpi4","accessible":false,"latency_ms":0.0,"error":"skipped_no_direct_home_lan_route"}
            },
            "freshness":{
                "packed_catalog":{"status":"STALE_LOCAL","delta_medias":8},
                "packed_ahash_catalog":{"status":"UP_TO_DATE","delta_medias":0},
                "phash_corpus":{"status":"PARTIAL_LOCAL","missing_locally":402},
                "ahash_corpus":{"status":"PARTIAL_LOCAL","missing_locally":536},
                "nodes":{
                    "local":{"catalog_dbphash":{"exists":true,"media_count":4974,"size_bytes":395529096},"catalog_dbahash":{"exists":true,"media_count":2490,"size_bytes":5503776992},"phash_files":{"count":4580},"ahash_files":{"count":2503}},
                    "o2switch":{"catalog_dbphash":{"exists":true,"media_count":4982,"size_bytes":397067144},"catalog_dbahash":{"exists":true,"media_count":2490,"size_bytes":5503776992},"phash_files":{"count":4982},"ahash_files":{"count":3039}}
                }
            },
            "backup_state":{"status":"BACKED_UP","latest_snapshot_id":"20261115T113737Z_fe26d89","verified_replicas":["o2switch","synology_nas"],"evicted_datasets":[]}
        }"#;
        let payload: Value = serde_json::from_str(raw).expect("valid json");
        let topo = parse_topology_from_status_json(&payload).expect("valid topology");
        assert_eq!(topo.network_profile, "work_vpn");
        assert!(!topo.cifs_safe_to_stat);
        assert_eq!(topo.backup_status.as_deref(), Some("BACKED_UP"));
        assert_eq!(topo.corpus.len(), 4);
        assert_eq!(topo.corpus[0].delta_summary, "+8 médias");
        let has_guard = topo
            .channels
            .iter()
            .any(|c| c.id == "local_cifs" && c.state_tag == "GUARD");
        assert!(has_guard);
    }

    #[test]
    fn test_parse_query_items_and_apply_inspect_json() {
        let q_raw = r#"{
            "results":{
                "tvshow:Andor S01E01":{
                    "query_result":{"matches":[{"filename":"Andor.S01E01.1080p.WEB.h264-KOGi.mkv","path":"Andor/Andor.S01E01.1080p.WEB.h264-KOGi.mkv","size_bytes":1985633052,"phash_file":"498032c4b01cbc0c.24.0fps.phash","coupled_srt":true}]}
                }
            }
        }"#;
        let q_json: Value = serde_json::from_str(q_raw).expect("valid query json");
        let mut items = parse_query_items_from_json(&q_json);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_id.as_deref(), Some("tvshow"));
        assert_eq!(items[0].status, IngestionStatus::Completed);

        let insp_raw = r#"{
            "ok":true,"duration_sec":2350.59,"size_bytes":1985633052,"video":{"codec":"h264"},
            "subtitle_streams":[{"index":2,"codec":"subrip","language":"eng","title":"SDH"}]
        }"#;
        let insp_json: Value = serde_json::from_str(insp_raw).expect("valid inspect json");
        assert!(apply_inspect_json_to_item(&mut items[0], &insp_json));
        assert_eq!(items[0].video_codec.as_deref(), Some("h264"));
        assert_eq!(items[0].duration_secs, Some(2350.59));
        assert_eq!(items[0].subtitles.len(), 1);
    }
}
