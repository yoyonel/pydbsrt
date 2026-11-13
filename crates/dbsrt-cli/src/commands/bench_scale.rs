//! Massive multimodal multi-window scale benchmark runner.
//!
//! Evaluates both pHash (Coarse + MIH-16) and aHash (Coarse-to-Fine K=16 + MIH-32)
//! across thousands of videos in local dev / NAS / appliance libraries.

use anyhow::{Context, Result};
use clap::Args;
use dbsrt_core::ahash::{DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE};
use dbsrt_core::metadata::MediaMetadata;
use dbsrt_core::multi_window::{
    aggregate_multi_window_matches, MultiWindowConsensus, MultiWindowPlan, PerWindowMatches,
    RawCandidateMatch, MAX_SHIFT_DRIFT_TOLERANCE_MS,
};
use dbsrt_core::{
    extract_ahash, extract_frames_phash, inspect_audio_streams, probe_video,
    AudioExtractionOptions, VideoMetadata,
};
use dbsrt_index::coarse_to_fine::{CoarseToFineAHashIndex, FineSearchCriteria};
use dbsrt_index::{AHashCatalog, MmapDbphash};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Default path to unified .dbphash index file.
pub const DEFAULT_SCALE_DBPHASH_PATH: &str = "data/catalog.dbphash";

/// Directory holding `<moviehash>.json` pHash sidecars used to resolve MovieHash aliases.
pub const DEFAULT_PHASH_SIDECAR_DIR: &str = "data/__PHASH__";

/// Length in hexadecimal characters of an OpenSubtitles 64-bit MovieHash.
const MOVIEHASH_HEX_LEN: usize = 16;

/// Diagnostic cause for containers physically lacking any audio stream.
pub const CAUSE_NO_AUDIO_STREAM: &str = "NO_AUDIO_STREAM";

/// Diagnostic cause for multi-track containers whose dub conflict is not measured.
pub const CAUSE_MULTI_TRACK_UNVERIFIED: &str = "MULTI_TRACK_CANDIDATE_UNVERIFIED";

/// Default directory containing acoustic .ahash fingerprints.
pub const DEFAULT_SCALE_AHASH_DIR: &str = "data/__AHASH__";

/// Default directory where streaming JSONL and status reports are placed.
pub const DEFAULT_REPORTS_DIR: &str = "data/reports";

/// Symlink target path pointing to latest JSONL benchmark report.
pub const DEFAULT_SCALE_OUTPUT_LATEST: &str =
    "data/reports/eval_scale_multimodal_multiwindow_latest.jsonl";

/// Symlink target path pointing to latest status JSON file.
pub const DEFAULT_SCALE_STATUS_LATEST: &str = "data/reports/eval_scale_status_latest.json";

/// Max Hamming distance tolerated by pHash recognition in the scale benchmark.
pub const DEFAULT_PHASH_SCALE_MAX_DISTANCE: u32 = 6;

/// Default coarse downsampling factor K for hierarchical aHash index.
pub const DEFAULT_SCALE_FACTOR_K: usize = 16;

/// Default temporal offset window ratios along media duration.
pub const DEFAULT_SCALE_OFFSETS: &str = "0.2,0.5,0.8";

/// Default duration in seconds for each audio query window.
pub const DEFAULT_AUDIO_WINDOW_SEC: f64 = 5.0;

/// Default video frame count for 1.0s pHash window evaluation.
pub const DEFAULT_VIDEO_WINDOW_SEC: f64 = 1.0;

/// Maximum allowable shift discrepancy in ms to declare dual multimodal agreement.
pub const MAX_MULTIMODAL_AGREEMENT_DRIFT_MS: i64 = 500;

/// Interval in processed items between status telemetry file updates.
pub const STATUS_FLUSH_INTERVAL: usize = 10;

/// Minutes per hour conversion factor.
pub const MINUTES_PER_HOUR: u64 = 60;

/// Seconds per minute conversion factor.
pub const SECONDS_PER_MINUTE: u64 = 60;

/// Hours per day conversion factor.
pub const HOURS_PER_DAY: u64 = 24;

/// Milliseconds per second conversion factor.
pub const MILLIS_PER_SEC: f64 = 1000.0;

/// Low energy lower ratio threshold for aHash spectral frames.
pub const LOW_ENERGY_MIN_RATIO: f64 = 0.20;

/// Low energy upper ratio threshold for aHash spectral frames.
pub const LOW_ENERGY_MAX_RATIO: f64 = 0.80;

/// Minimum valid video duration in seconds.
pub const MIN_VIDEO_DURATION_SEC: f64 = 10.0;

/// Haitsma-Kalker sub-fingerprint frame bit width.
pub const AHASH_FRAME_BITS: usize = 64;

/// Maximum mean Hamming distance per frame for fine acoustic verification.
pub const FINE_SEARCH_MAX_DISTANCE: f64 = 14.0;

/// Minimum visual confidence threshold for raw candidate detection.
pub const PHASH_MIN_CONFIDENCE: f64 = 20.0;

/// CLI arguments for the multimodal scale benchmark.
#[derive(Args, Debug, Clone)]
pub struct BenchScaleMultimodalArgs {
    /// Directories containing media files to discover and benchmark.
    #[arg(short, long, value_delimiter = ',')]
    pub dirs: Vec<PathBuf>,

    /// Path to unified .dbphash catalog file.
    #[arg(long, default_value = DEFAULT_SCALE_DBPHASH_PATH)]
    pub dbphash: PathBuf,

    /// Directory containing .ahash acoustic fingerprints.
    #[arg(long, default_value = DEFAULT_SCALE_AHASH_DIR)]
    pub ahash_dir: PathBuf,

    /// Output path for streaming JSONL report (defaults to timestamped file in data/reports/).
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Path to write periodically flushed status telemetry (defaults to timestamped file in data/reports/).
    #[arg(long)]
    pub status_json: Option<PathBuf>,

    /// Downsampling factor K for hierarchical coarse-to-fine aHash index.
    #[arg(long, default_value_t = DEFAULT_SCALE_FACTOR_K)]
    pub factor_k: usize,

    /// Comma-separated temporal offset ratios (e.g. "0.2,0.5,0.8").
    #[arg(long, default_value = DEFAULT_SCALE_OFFSETS)]
    pub offsets: String,

    /// Maximum number of media files to benchmark (0 for unlimited).
    #[arg(long, default_value_t = 0)]
    pub limit: usize,

    /// Automatically resume by skipping files already present in output JSONL.
    #[arg(long, default_value_t = true)]
    pub resume: bool,
}

/// Container audio metadata captured during benchmark probing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContainerAudioMetadata {
    pub track_count: usize,
    pub track0_lang: Option<String>,
    pub track0_title: Option<String>,
    pub track0_codec: Option<String>,
    pub track0_channels: Option<usize>,
    pub all_langs: Vec<String>,
    pub is_multi_track: bool,
}

/// Detailed telemetry for a single temporal window in visual pHash matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhashWindowDetail {
    pub window_idx: usize,
    pub ratio: f64,
    pub start_sec: f64,
    pub matched_media: Option<String>,
    pub matched_media_id: Option<u32>,
    pub matching_cuts: usize,
    pub matching_sparse: usize,
    pub estimated_shift_ms: i64,
    pub confidence: f64,
}

/// Detailed telemetry for a single temporal window in acoustic aHash matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AhashWindowDetail {
    pub window_idx: usize,
    pub ratio: f64,
    pub start_sec: f64,
    pub hashes_count: usize,
    pub ones_ratio: f64,
    pub is_low_energy: bool,
    pub coarse_candidate: Option<String>,
    pub fine_distance: Option<f64>,
    pub fine_snr_bits: Option<f64>,
    pub ref_offset_sec: Option<f64>,
    pub estimated_shift_ms: Option<i64>,
    pub confidence: f64,
}

/// Evaluation record for visual pHash across multiple temporal windows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhashMatchSummary {
    pub media_match: Option<String>,
    pub media_id: Option<u32>,
    /// Season of the matched reference entry (`None` when unknown / 0).
    #[serde(default)]
    pub season: Option<u32>,
    /// Episode of the matched reference entry (`None` when unknown / 0).
    #[serde(default)]
    pub episode: Option<u32>,
    pub confidence: f64,
    pub consensus_shift_ms: i64,
    pub matching_windows: usize,
    pub total_windows: usize,
    pub is_coherent: bool,
    pub timer_sec: f64,
    pub windows: Vec<PhashWindowDetail>,
}

/// Evaluation record for acoustic aHash across multiple temporal windows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AhashMatchSummary {
    pub media_match: Option<String>,
    pub media_id: Option<u32>,
    pub confidence: f64,
    pub consensus_shift_ms: i64,
    pub matching_windows: usize,
    pub total_windows: usize,
    pub is_coherent: bool,
    pub timer_sec: f64,
    pub windows: Vec<AhashWindowDetail>,
}

/// Comparison and agreement classification between visual and acoustic recognition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalAgreement {
    pub match_agreed: bool,
    pub shift_diff_ms: Option<i64>,
    pub status: String,
}

/// Complete benchmark record for one evaluated media file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaScaleBenchmarkRecord {
    pub timestamp: String,
    pub media_path: String,
    pub media_stem: String,
    pub duration_sec: f64,
    pub fps: f64,
    pub audio: ContainerAudioMetadata,
    pub phash: PhashMatchSummary,
    pub ahash: AhashMatchSummary,
    pub agreement: MultimodalAgreement,
    pub probable_cause: String,
    pub diagnostic_note: String,
    pub total_timer_sec: f64,
}

/// Live telemetry tracking progress and accuracy metrics across benchmark execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScaleBenchmarkTelemetry {
    pub total_discovered: usize,
    pub already_completed: usize,
    pub processed_this_run: usize,
    pub dual_match_agreed: usize,
    pub dual_match_disagreed: usize,
    pub phash_only: usize,
    pub ahash_only: usize,
    pub no_match: usize,
    pub errors: usize,
    pub avg_phash_sec: f64,
    pub avg_ahash_sec: f64,
    pub avg_total_sec: f64,
    pub elapsed_sec: f64,
    pub eta_sec: f64,
}

/// Context state bundling loaded indexes and parsed configuration.
pub struct BenchContext<'a> {
    pub dbphash: &'a MmapDbphash,
    pub dense_catalog: &'a AHashCatalog,
    pub coarse_index: &'a CoarseToFineAHashIndex,
    pub plan: &'a MultiWindowPlan,
    pub indexed_ahash_titles: &'a HashSet<String>,
}

/// Context bundling inputs for root cause diagnostic classification.
struct DiagnosticInput<'a> {
    pub phash: &'a PhashMatchSummary,
    pub ahash: &'a AhashMatchSummary,
    pub audio: &'a ContainerAudioMetadata,
    pub agreement: &'a MultimodalAgreement,
    /// True only when the container probe succeeded and reported zero audio tracks.
    pub no_audio_stream: bool,
}

/// Computes UTC civil date and time components from Unix epoch seconds using Hinnant algorithm.
fn civil_from_epoch(epoch_secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let sec = (epoch_secs % SECONDS_PER_MINUTE) as u32;
    let total_min = epoch_secs / SECONDS_PER_MINUTE;
    let min = (total_min % MINUTES_PER_HOUR) as u32;
    let total_hour = total_min / MINUTES_PER_HOUR;
    let hour = (total_hour % HOURS_PER_DAY) as u32;
    let mut days = (total_hour / HOURS_PER_DAY) as i64;

    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let final_y = if m <= 2 { y + 1 } else { y };
    (final_y, m, d, hour, min, sec)
}

fn current_utc_slug() -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, m, d, hour, min, sec) = civil_from_epoch(epoch);
    format!("{y:04}{m:02}{d:02}_{hour:02}{min:02}{sec:02}")
}

fn current_iso_timestamp() -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, m, d, hour, min, sec) = civil_from_epoch(epoch);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn update_latest_symlink(target_path: &Path, latest_path: &Path) {
    let _ = std::fs::remove_file(latest_path);
    #[cfg(unix)]
    {
        if let Some(filename) = target_path.file_name() {
            let _ = std::os::unix::fs::symlink(filename, latest_path);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::copy(target_path, latest_path);
    }
}

/// Resolves output JSONL and status JSON paths with automatic timestamp slugs.
fn resolve_report_destinations(
    output_arg: Option<PathBuf>,
    status_arg: Option<PathBuf>,
) -> Result<(PathBuf, PathBuf)> {
    let timestamp_slug = current_utc_slug();
    let reports_dir = PathBuf::from(DEFAULT_REPORTS_DIR);
    std::fs::create_dir_all(&reports_dir)?;

    let out = output_arg.unwrap_or_else(|| {
        reports_dir.join(format!(
            "eval_scale_multimodal_multiwindow_{timestamp_slug}.jsonl"
        ))
    });
    let status = status_arg
        .unwrap_or_else(|| reports_dir.join(format!("eval_scale_status_{timestamp_slug}.json")));

    println!("Report JSONL destination: {:?}", out);
    println!("Status JSON destination: {:?}", status);
    Ok((out, status))
}

/// Resolves set of already completed paths from current or latest report.
fn load_resumed_completed_paths(output_path: &Path, resume: bool) -> HashSet<PathBuf> {
    if !resume {
        return HashSet::new();
    }
    if output_path.exists() {
        load_resumed_paths(output_path).unwrap_or_default()
    } else if Path::new(DEFAULT_SCALE_OUTPUT_LATEST).exists() {
        load_resumed_paths(Path::new(DEFAULT_SCALE_OUTPUT_LATEST)).unwrap_or_default()
    } else {
        HashSet::new()
    }
}

/// Loaded catalog bundle returned by catalog initialization helper.
struct LoadedCatalogs {
    pub dbphash: MmapDbphash,
    pub dense_catalog: AHashCatalog,
    pub coarse_index: CoarseToFineAHashIndex,
    pub indexed_ahash_titles: HashSet<String>,
}

/// Reads `<hex>.json` pHash sidecars and maps each MovieHash to its `media_name`.
///
/// Missing directory or malformed sidecars are skipped; returns an empty map if none readable.
fn load_moviehash_aliases(dir: &Path) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return aliases;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if path.extension().is_none_or(|e| e != "json") || !is_moviehash_hex(&stem) {
            continue;
        }
        let name = std::fs::read_to_string(&path)
            .ok()
            .and_then(|txt| serde_json::from_str::<serde_json::Value>(&txt).ok())
            .and_then(|v| v.get("media_name")?.as_str().map(str::to_string));
        if let Some(name) = name {
            aliases.insert(stem, name);
        }
    }
    aliases
}

/// Returns true when `s` is a 16-character hexadecimal MovieHash.
fn is_moviehash_hex(s: &str) -> bool {
    s.len() == MOVIEHASH_HEX_LEN && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Renames MovieHash-named aHash catalog entries to their sidecar media name.
///
/// Returns the number of entries resolved.
fn resolve_catalog_moviehash_names(
    catalog: &mut AHashCatalog,
    aliases: &HashMap<String, String>,
) -> usize {
    let mut resolved = 0;
    for e in catalog.entries.iter_mut() {
        if let Some(name) = aliases.get(&e.media_name.to_lowercase()) {
            e.media_name = name.clone();
            resolved += 1;
        }
    }
    resolved
}

/// Loads visual and acoustic catalogs and constructs the coarse-to-fine index.
fn load_all_catalogs(
    dbphash_path: &Path,
    ahash_dir: &Path,
    factor_k: usize,
) -> Result<LoadedCatalogs> {
    println!("Loading visual index from {:?}...", dbphash_path);
    let dbphash = MmapDbphash::open(dbphash_path).context("Open dbphash")?;
    println!(
        "✓ Memory-mapped .dbphash catalog ({} medias)",
        dbphash.media_entries().len()
    );

    println!("Loading acoustic catalog from {:?}...", ahash_dir);
    let t_ahash = Instant::now();
    let mut dense_catalog = AHashCatalog::load_from_dir(ahash_dir).context("Load ahash")?;
    let aliases = load_moviehash_aliases(Path::new(DEFAULT_PHASH_SIDECAR_DIR));
    let resolved = resolve_catalog_moviehash_names(&mut dense_catalog, &aliases);
    println!("✓ Resolved {resolved} MovieHash-named aHash entries via pHash sidecars");
    println!(
        "✓ Loaded {} aHash files ({} frames) in {:.2}s",
        dense_catalog.entries.len(),
        dense_catalog.total_frames,
        t_ahash.elapsed().as_secs_f64()
    );

    let indexed_ahash_titles = dense_catalog
        .entries
        .iter()
        .map(|e| e.media_name.to_lowercase().replace(['.', '_', '-'], " "))
        .collect();

    println!("Building Coarse-to-Fine index (K={})...", factor_k);
    let t_c2f = Instant::now();
    let coarse_index = CoarseToFineAHashIndex::build(&dense_catalog, factor_k);
    println!(
        "✓ Coarse-to-Fine index ready in {:.2}s",
        t_c2f.elapsed().as_secs_f64()
    );

    Ok(LoadedCatalogs {
        dbphash,
        dense_catalog,
        coarse_index,
        indexed_ahash_titles,
    })
}

/// Main entry point for the multimodal scale benchmark command.
///
/// # Arguments
/// * `args` - CLI options and catalog path configurations.
///
/// # Returns
/// `Result<()>` indicating successful benchmark completion.
///
/// # Errors
/// Fails if catalog files cannot be loaded or output directory cannot be written.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. Generates timestamped report filenames and maintains latest symlinks.
/// 2. Discovers target media files across specified NAS / appliance paths.
/// 3. Memory-maps `catalog.dbphash` and builds `CoarseToFineAHashIndex` in memory once.
/// 4. Reads existing JSONL records to support instant crash recovery and resume.
/// 5. Iterates through media files, streaming records with audio layouts and per-window diagnostics.
pub fn run_bench_scale_multimodal(args: BenchScaleMultimodalArgs) -> Result<()> {
    let t_init = Instant::now();
    println!("=== DBS-RT MULTIMODAL SCALE BENCHMARK (pHash + aHash) ===");

    let (output_path, status_path) = resolve_report_destinations(args.output, args.status_json)?;

    let ratios: Vec<f64> = args
        .offsets
        .split(',')
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .collect();
    let plan = MultiWindowPlan::new(ratios, DEFAULT_VIDEO_WINDOW_SEC)?;

    let target_dirs = resolve_target_directories(&args.dirs);
    println!("Scanning target directories: {:?}", target_dirs);
    let all_files = discover_video_files(&target_dirs);
    println!("✓ Discovered {} video files on mounts", all_files.len());

    let completed_paths = load_resumed_completed_paths(&output_path, args.resume);
    println!(
        "✓ Found {} already-completed records in JSONL",
        completed_paths.len()
    );

    let catalogs = load_all_catalogs(&args.dbphash, &args.ahash_dir, args.factor_k)?;

    let mut out_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_path)
        .context("Open output JSONL")?;

    update_latest_symlink(&output_path, Path::new(DEFAULT_SCALE_OUTPUT_LATEST));
    update_latest_symlink(&status_path, Path::new(DEFAULT_SCALE_STATUS_LATEST));

    let ctx = BenchContext {
        dbphash: &catalogs.dbphash,
        dense_catalog: &catalogs.dense_catalog,
        coarse_index: &catalogs.coarse_index,
        plan: &plan,
        indexed_ahash_titles: &catalogs.indexed_ahash_titles,
    };

    let mut telemetry = ScaleBenchmarkTelemetry {
        total_discovered: all_files.len(),
        already_completed: completed_paths.len(),
        ..Default::default()
    };

    let files_to_run: Vec<PathBuf> = all_files
        .into_iter()
        .filter(|p| !completed_paths.contains(p))
        .collect();

    let max_run = if args.limit > 0 {
        files_to_run.len().min(args.limit)
    } else {
        files_to_run.len()
    };

    println!(
        "\nStarting execution on {} files (init took {:.1}s)...\n",
        max_run,
        t_init.elapsed().as_secs_f64()
    );

    let mut sweep = SweepContext {
        files: &files_to_run[..max_run],
        out_file: &mut out_file,
        status_json: &status_path,
    };

    execute_benchmark_sweep(&ctx, &mut sweep, &mut telemetry)?;

    write_status_summary(&status_path, &telemetry)?;
    update_latest_symlink(&status_path, Path::new(DEFAULT_SCALE_STATUS_LATEST));
    update_latest_symlink(&output_path, Path::new(DEFAULT_SCALE_OUTPUT_LATEST));
    Ok(())
}

/// Context bundling output destinations and target file slice for benchmark sweep.
struct SweepContext<'a> {
    pub files: &'a [PathBuf],
    pub out_file: &'a mut File,
    pub status_json: &'a Path,
}

/// Iterates sequentially through files, streaming results and telemetry.
fn execute_benchmark_sweep(
    ctx: &BenchContext<'_>,
    sweep: &mut SweepContext<'_>,
    telemetry: &mut ScaleBenchmarkTelemetry,
) -> Result<()> {
    let total = sweep.files.len();
    let run_start = Instant::now();

    for (idx, media_path) in sweep.files.iter().enumerate() {
        let record = evaluate_single_media(ctx, media_path);
        let elapsed = run_start.elapsed().as_secs_f64();
        update_telemetry(telemetry, &record, elapsed, total - idx - 1);

        let json_line = serde_json::to_string(&record)?;
        writeln!(sweep.out_file, "{}", json_line)?;
        sweep.out_file.flush()?;

        print_live_progress(idx + 1, total, &record, telemetry);

        if (idx + 1) % STATUS_FLUSH_INTERVAL == 0 || idx + 1 == total {
            write_status_summary(sweep.status_json, telemetry)?;
        }
    }

    println!(
        "\n=== BENCHMARK COMPLETED IN {:.1}s ===",
        run_start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Resolves search directories with automatic fallback to standard mount points.
fn resolve_target_directories(cli_dirs: &[PathBuf]) -> Vec<PathBuf> {
    if !cli_dirs.is_empty() {
        return cli_dirs.to_vec();
    }
    let user = std::env::var("USER").unwrap_or_default();
    let media_base = PathBuf::from("/media").join(user);
    let candidates = [
        media_base.join("Synology").join("tvshow"),
        media_base.join("Synology").join("video"),
        media_base.join("Freebox").join("Disque 1"),
        PathBuf::from("data/snippets"),
    ];
    let existing: Vec<PathBuf> = candidates.into_iter().filter(|p| p.is_dir()).collect();
    if existing.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        existing
    }
}

/// Checks whether a file path has an allowed video extension.
fn has_valid_video_ext(path: &Path, valid_exts: &[&str]) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|ext| valid_exts.iter().any(|v| v.eq_ignore_ascii_case(ext)))
        .unwrap_or(false)
}

/// Recursively scans directories to collect video media file paths.
fn discover_video_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let valid_exts = ["mkv", "mp4", "avi", "webm", "ts", "m4v"];

    for d in dirs {
        collect_videos_recursive(d, &valid_exts, &mut files);
    }
    files.sort();
    files
}

/// Helper function traversing directory hierarchy.
fn collect_videos_recursive(dir: &Path, valid_exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_videos_recursive(&path, valid_exts, out);
        } else if path.is_file() && has_valid_video_ext(&path, valid_exts) {
            out.push(path);
        }
    }
}

/// Reads previously written JSONL report to populate resume skip set.
fn load_resumed_paths(output_path: &Path) -> Result<HashSet<PathBuf>> {
    let file = File::open(output_path)?;
    let reader = BufReader::new(file);
    let mut completed = HashSet::new();

    for line in reader.lines().map_while(Result::ok) {
        if let Ok(record) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(path_str) = record.get("media_path").and_then(|v| v.as_str()) {
                completed.insert(PathBuf::from(path_str));
            }
        }
    }
    Ok(completed)
}

/// Checks whether reference title exists in the acoustic catalog.
fn is_media_indexed_in_ahash(ctx: &BenchContext<'_>, title: &str) -> bool {
    let clean_target = title.to_lowercase().replace(['.', '_', '-'], " ");
    let target_words: Vec<&str> = clean_target.split_whitespace().collect();
    if target_words.is_empty() {
        return false;
    }
    ctx.indexed_ahash_titles
        .iter()
        .any(|t| target_words.iter().all(|w| t.contains(w)))
}

/// Builds the diagnostic for containers with no physical audio stream.
fn no_audio_diagnostic(input: &DiagnosticInput<'_>) -> (String, String) {
    let note = if input.agreement.status == "PHASH_ONLY" {
        "Container has 0 audio streams: acoustic matching impossible (not an acoustic false negative); visual match stands alone"
    } else {
        "Container has 0 audio streams: acoustic matching skipped; visual match also missed"
    };
    (CAUSE_NO_AUDIO_STREAM.to_string(), note.to_string())
}

/// Automatically classifies probable root-cause for diagnostic post-bench audits.
fn determine_diagnostic_cause(
    ctx: &BenchContext<'_>,
    input: &DiagnosticInput<'_>,
) -> (String, String) {
    if input.no_audio_stream && matches!(input.agreement.status.as_str(), "PHASH_ONLY" | "NO_MATCH")
    {
        return no_audio_diagnostic(input);
    }
    match input.agreement.status.as_str() {
        "DUAL_MATCH_AGREED" => (
            "NOMINAL_MULTIMODAL_MATCH".to_string(),
            "pHash and aHash match identical reference with coherent shift".to_string(),
        ),
        "DUAL_MATCH_SHIFT_DIVERGENCE" => {
            let diff = input.agreement.shift_diff_ms.unwrap_or(0);
            (
                "TIMELINE_SHIFT_DIVERGENCE".to_string(),
                format!("pHash and aHash agree on media title but shift diverges by {diff}ms"),
            )
        }
        "DUAL_MATCH_DISAGREED" => (
            "CROSS_MODAL_TITLE_DISCORD".to_string(),
            format!(
                "pHash matched '{}' while aHash matched '{}'",
                input.phash.media_match.as_deref().unwrap_or("?"),
                input.ahash.media_match.as_deref().unwrap_or("?")
            ),
        ),
        "AHASH_ONLY" => (
            "VISUAL_MISS_ACOUSTIC_MATCH".to_string(),
            "Acoustic match confirmed but visual cuts/frames were below confidence threshold"
                .to_string(),
        ),
        "PHASH_ONLY" => {
            let p_match = input.phash.media_match.as_deref().unwrap_or_default();
            if !is_media_indexed_in_ahash(ctx, p_match) {
                (
                    "UNINDEXED_IN_AHASH".to_string(),
                    format!(
                        "Visual match '{p_match}' has no acoustic fingerprint in aHash catalog"
                    ),
                )
            } else if input.ahash.windows.iter().any(|w| w.is_low_energy) {
                (
                    "LOW_ENERGY_SILENCE".to_string(),
                    "Sampled query windows contain low acoustic variance or silence".to_string(),
                )
            } else if input.audio.is_multi_track {
                (
                    CAUSE_MULTI_TRACK_UNVERIFIED.to_string(),
                    format!(
                        "Container has {} audio tracks ({:?}); dub conflict on track 0 NOT measured",
                        input.audio.track_count, input.audio.all_langs
                    ),
                )
            } else {
                (
                    "VOTE_STARVATION_SHORT_WINDOW".to_string(),
                    "Acoustic fingerprint present in catalog but 5s window starved coarse votes under K=16".to_string(),
                )
            }
        }
        "NO_MATCH" => (
            "UNINDEXED_MEDIA".to_string(),
            "Neither visual nor acoustic fingerprints matched any indexed catalog entry"
                .to_string(),
        ),
        _ => (
            "PROBE_OR_EXTRACTION_ERROR".to_string(),
            "Media container probing or stream decoding encountered an error".to_string(),
        ),
    }
}

/// Evaluates a single media against both pHash and aHash multi-window consensus.
fn evaluate_single_media(ctx: &BenchContext<'_>, media_path: &Path) -> MediaScaleBenchmarkRecord {
    let t_start = Instant::now();
    let media_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let meta = match probe_video(media_path) {
        Ok(m) if m.duration >= MIN_VIDEO_DURATION_SEC => m,
        _ => return build_error_record(media_path, &media_stem, t_start.elapsed().as_secs_f64()),
    };

    let audio_probe = inspect_audio_streams(media_path);
    let no_audio_stream = matches!(&audio_probe, Ok(a) if a.track_count() == 0);
    let audio_meta = match audio_probe {
        Ok(a) => ContainerAudioMetadata {
            track_count: a.track_count(),
            track0_lang: a.track0_lang(),
            track0_title: a.track0_title(),
            track0_codec: a.track0_codec(),
            track0_channels: a.track0_channels(),
            all_langs: a.all_languages(),
            is_multi_track: a.is_multi_track(),
        },
        Err(_) => ContainerAudioMetadata::default(),
    };

    let phash_summary = evaluate_phash_multi(ctx, media_path, &meta);
    let ahash_summary = if no_audio_stream {
        build_empty_ahash_summary(ctx.plan.ratios.len())
    } else {
        evaluate_ahash_multi(ctx, media_path, &meta)
    };
    let agreement = classify_agreement(&phash_summary, &ahash_summary);

    let diag_input = DiagnosticInput {
        phash: &phash_summary,
        ahash: &ahash_summary,
        audio: &audio_meta,
        agreement: &agreement,
        no_audio_stream,
    };
    let (probable_cause, diagnostic_note) = determine_diagnostic_cause(ctx, &diag_input);

    MediaScaleBenchmarkRecord {
        timestamp: current_iso_timestamp(),
        media_path: media_path.to_string_lossy().to_string(),
        media_stem,
        duration_sec: meta.duration,
        fps: meta.fps,
        audio: audio_meta,
        phash: phash_summary,
        ahash: ahash_summary,
        agreement,
        probable_cause,
        diagnostic_note,
        total_timer_sec: t_start.elapsed().as_secs_f64(),
    }
}

/// Builds an empty pHash window detail.
fn build_empty_phash_window(window_idx: usize, ratio: f64, start_sec: f64) -> PhashWindowDetail {
    PhashWindowDetail {
        window_idx,
        ratio,
        start_sec,
        matched_media: None,
        matched_media_id: None,
        matching_cuts: 0,
        matching_sparse: 0,
        estimated_shift_ms: 0,
        confidence: 0.0,
    }
}

/// Builds an empty aHash window detail.
fn build_empty_ahash_window(window_idx: usize, ratio: f64, start_sec: f64) -> AhashWindowDetail {
    AhashWindowDetail {
        window_idx,
        ratio,
        start_sec,
        hashes_count: 0,
        ones_ratio: 0.0,
        is_low_energy: true,
        coarse_candidate: None,
        fine_distance: None,
        fine_snr_bits: None,
        ref_offset_sec: None,
        estimated_shift_ms: None,
        confidence: 0.0,
    }
}

/// Evaluates visual pHash across multiple temporal offset windows.
fn evaluate_phash_multi(
    ctx: &BenchContext<'_>,
    media_path: &Path,
    meta: &VideoMetadata,
) -> PhashMatchSummary {
    let t0 = Instant::now();
    let starts = ctx.plan.compute_start_seconds(meta.duration);
    let nb_frames = (ctx.plan.window_duration_sec * meta.fps).round() as usize;

    if nb_frames == 0 {
        return build_empty_phash_summary(t0.elapsed().as_secs_f64(), ctx.plan.ratios.len());
    }

    let mut per_window: Vec<PerWindowMatches> = Vec::with_capacity(ctx.plan.ratios.len());
    let mut window_details: Vec<PhashWindowDetail> = Vec::with_capacity(ctx.plan.ratios.len());

    for (w_idx, (&ratio, &start_sec)) in ctx.plan.ratios.iter().zip(starts.iter()).enumerate() {
        let hashes = match extract_frames_phash(media_path, start_sec, nb_frames) {
            Ok(h) => h,
            Err(_) => {
                window_details.push(build_empty_phash_window(w_idx, ratio, start_sec));
                continue;
            }
        };
        let matches = ctx
            .dbphash
            .recognize(&hashes, DEFAULT_PHASH_SCALE_MAX_DISTANCE);
        let top_match = matches
            .into_iter()
            .find(|m| m.confidence >= PHASH_MIN_CONFIDENCE);

        let win_matches: Vec<RawCandidateMatch> = match &top_match {
            Some(m) => vec![(
                m.media_id,
                m.clean_title.clone(),
                m.matching_cuts,
                m.matching_sparse,
                m.estimated_shift_ms,
                m.confidence,
            )],
            None => Vec::new(),
        };

        window_details.push(PhashWindowDetail {
            window_idx: w_idx,
            ratio,
            start_sec,
            matched_media: top_match.as_ref().map(|m| m.clean_title.clone()),
            matched_media_id: top_match.as_ref().map(|m| m.media_id),
            matching_cuts: top_match.as_ref().map(|m| m.matching_cuts).unwrap_or(0),
            matching_sparse: top_match.as_ref().map(|m| m.matching_sparse).unwrap_or(0),
            estimated_shift_ms: top_match
                .as_ref()
                .map(|m| m.estimated_shift_ms)
                .unwrap_or(0),
            confidence: top_match.as_ref().map(|m| m.confidence).unwrap_or(0.0),
        });

        per_window.push((w_idx, ratio, start_sec, win_matches));
    }

    let consensus = aggregate_multi_window_matches(
        ctx.plan.ratios.len(),
        per_window,
        MAX_SHIFT_DRIFT_TOLERANCE_MS,
    );

    let mut summary = summarize_phash_consensus(
        consensus.first(),
        window_details,
        t0.elapsed().as_secs_f64(),
        ctx.plan.ratios.len(),
    );
    if let Some(id) = summary.media_id {
        if let Some(e) = ctx
            .dbphash
            .media_entries()
            .iter()
            .find(|e| e.media_id == id)
        {
            summary.season = Some(e.season).filter(|&v| v > 0);
            summary.episode = Some(e.episode).filter(|&v| v > 0);
        }
    }
    summary
}

/// Evaluates acoustic aHash across multiple temporal offset windows using C2F K=16.
fn evaluate_ahash_multi(
    ctx: &BenchContext<'_>,
    media_path: &Path,
    meta: &VideoMetadata,
) -> AhashMatchSummary {
    let t0 = Instant::now();
    let starts = ctx.plan.compute_start_seconds(meta.duration);
    let mut per_window: Vec<PerWindowMatches> = Vec::with_capacity(ctx.plan.ratios.len());
    let mut window_details: Vec<AhashWindowDetail> = Vec::with_capacity(ctx.plan.ratios.len());

    for (w_idx, (&ratio, &start_sec)) in ctx.plan.ratios.iter().zip(starts.iter()).enumerate() {
        let opts = AudioExtractionOptions {
            sample_rate: DEFAULT_SAMPLE_RATE,
            frame_size: DEFAULT_FRAME_SIZE,
            hop_size: DEFAULT_HOP_SIZE,
            start_sec,
            duration_sec: Some(DEFAULT_AUDIO_WINDOW_SEC),
            custom_ffmpeg: None,
        };
        let ahash_file = match extract_ahash(media_path, &opts) {
            Ok(f) if !f.hashes.is_empty() => f,
            _ => {
                window_details.push(build_empty_ahash_window(w_idx, ratio, start_sec));
                continue;
            }
        };

        let hashes_count = ahash_file.hashes.len();
        let total_ones: usize = ahash_file
            .hashes
            .iter()
            .map(|h| h.count_ones() as usize)
            .sum();
        let total_bits = hashes_count * AHASH_FRAME_BITS;
        let ones_ratio = if total_bits > 0 {
            total_ones as f64 / total_bits as f64
        } else {
            0.0
        };
        let is_low_energy = ones_ratio < LOW_ENERGY_MIN_RATIO || ones_ratio > LOW_ENERGY_MAX_RATIO;

        let fps = ahash_file.header.audio_fps();
        let criteria = FineSearchCriteria {
            dense_query: &ahash_file.hashes,
            dense_fps: fps,
            max_distance: FINE_SEARCH_MAX_DISTANCE,
        };

        let mut win_matches: Vec<RawCandidateMatch> = Vec::new();
        let search_match = ctx.coarse_index.search(ctx.dense_catalog, &criteria);

        if let Some(ref m) = search_match {
            let ref_time_ms = (m.offset_sec * MILLIS_PER_SEC).round() as i64;
            let conf = m.score * 100.0;
            let sparse = (m.snr_bits * 10.0) as usize;
            win_matches.push((0, m.media_name.clone(), 0, sparse, ref_time_ms, conf));
        }

        let estimated_shift = search_match
            .as_ref()
            .map(|m| ((m.offset_sec - start_sec) * MILLIS_PER_SEC).round() as i64);

        window_details.push(AhashWindowDetail {
            window_idx: w_idx,
            ratio,
            start_sec,
            hashes_count,
            ones_ratio,
            is_low_energy,
            coarse_candidate: search_match.as_ref().map(|m| m.media_name.clone()),
            fine_distance: search_match.as_ref().map(|m| m.mean_distance),
            fine_snr_bits: search_match.as_ref().map(|m| m.snr_bits),
            ref_offset_sec: search_match.as_ref().map(|m| m.offset_sec),
            estimated_shift_ms: estimated_shift,
            confidence: search_match
                .as_ref()
                .map(|m| m.score * 100.0)
                .unwrap_or(0.0),
        });

        per_window.push((w_idx, ratio, start_sec, win_matches));
    }

    let consensus = aggregate_multi_window_matches(
        ctx.plan.ratios.len(),
        per_window,
        MAX_SHIFT_DRIFT_TOLERANCE_MS,
    );

    summarize_ahash_consensus(
        consensus.first(),
        window_details,
        t0.elapsed().as_secs_f64(),
        ctx.plan.ratios.len(),
    )
}

/// Converts an optional pHash consensus match into a standardized summary record.
fn summarize_phash_consensus(
    best: Option<&MultiWindowConsensus>,
    windows: Vec<PhashWindowDetail>,
    timer_sec: f64,
    total_windows: usize,
) -> PhashMatchSummary {
    if let Some(c) = best {
        PhashMatchSummary {
            season: None,
            episode: None,
            media_match: Some(c.clean_title.clone()),
            media_id: if c.media_id > 0 {
                Some(c.media_id)
            } else {
                None
            },
            confidence: c.consensus_confidence,
            consensus_shift_ms: c.consensus_shift_ms,
            matching_windows: c.matching_windows,
            total_windows,
            is_coherent: c.is_temporally_coherent,
            timer_sec,
            windows,
        }
    } else {
        PhashMatchSummary {
            season: None,
            episode: None,
            media_match: None,
            media_id: None,
            confidence: 0.0,
            consensus_shift_ms: 0,
            matching_windows: 0,
            total_windows,
            is_coherent: false,
            timer_sec,
            windows,
        }
    }
}

/// Converts an optional aHash consensus match into a standardized summary record.
fn summarize_ahash_consensus(
    best: Option<&MultiWindowConsensus>,
    windows: Vec<AhashWindowDetail>,
    timer_sec: f64,
    total_windows: usize,
) -> AhashMatchSummary {
    if let Some(c) = best {
        AhashMatchSummary {
            media_match: Some(c.clean_title.clone()),
            media_id: if c.media_id > 0 {
                Some(c.media_id)
            } else {
                None
            },
            confidence: c.consensus_confidence,
            consensus_shift_ms: c.consensus_shift_ms,
            matching_windows: c.matching_windows,
            total_windows,
            is_coherent: c.is_temporally_coherent,
            timer_sec,
            windows,
        }
    } else {
        AhashMatchSummary {
            media_match: None,
            media_id: None,
            confidence: 0.0,
            consensus_shift_ms: 0,
            matching_windows: 0,
            total_windows,
            is_coherent: false,
            timer_sec,
            windows,
        }
    }
}

/// Builds an empty aHash summary (no audio extraction performed).
fn build_empty_ahash_summary(total_windows: usize) -> AhashMatchSummary {
    AhashMatchSummary {
        media_match: None,
        media_id: None,
        confidence: 0.0,
        consensus_shift_ms: 0,
        matching_windows: 0,
        total_windows,
        is_coherent: false,
        timer_sec: 0.0,
        windows: Vec::new(),
    }
}

/// Builds an empty pHash summary when no frames were evaluated.
fn build_empty_phash_summary(timer_sec: f64, total_windows: usize) -> PhashMatchSummary {
    PhashMatchSummary {
        season: None,
        episode: None,
        media_match: None,
        media_id: None,
        confidence: 0.0,
        consensus_shift_ms: 0,
        matching_windows: 0,
        total_windows,
        is_coherent: false,
        timer_sec,
        windows: Vec::new(),
    }
}

/// Builds an error record when media probing or container reading fails.
fn build_error_record(
    media_path: &Path,
    media_stem: &str,
    timer_sec: f64,
) -> MediaScaleBenchmarkRecord {
    MediaScaleBenchmarkRecord {
        timestamp: current_iso_timestamp(),
        media_path: media_path.to_string_lossy().to_string(),
        media_stem: media_stem.to_string(),
        duration_sec: 0.0,
        fps: 0.0,
        audio: ContainerAudioMetadata::default(),
        phash: build_empty_phash_summary(0.0, 3),
        ahash: AhashMatchSummary {
            media_match: None,
            media_id: None,
            confidence: 0.0,
            consensus_shift_ms: 0,
            matching_windows: 0,
            total_windows: 3,
            is_coherent: false,
            timer_sec: 0.0,
            windows: Vec::new(),
        },
        agreement: MultimodalAgreement {
            match_agreed: false,
            shift_diff_ms: None,
            status: "ERROR".to_string(),
        },
        probable_cause: "PROBE_FAILURE".to_string(),
        diagnostic_note: "Container probe failed or duration below minimum threshold".to_string(),
        total_timer_sec: timer_sec,
    }
}

/// Extracts a detectable `(season, episode)` pair from a media name.
fn detect_episode(name: &str) -> Option<(u32, u32)> {
    let m = MediaMetadata::parse(name);
    match (m.season, m.episode) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
    }
}

/// Checks fuzzy title matching, rejecting differing `(season, episode)` pairs.
///
/// `p_episode` comes from the pHash catalog entry; the aHash episode is parsed from `a_title`.
fn are_titles_matching(p_title: &str, p_episode: Option<(u32, u32)>, a_title: &str) -> bool {
    if let (Some(p_ep), Some(a_ep)) = (p_episode, detect_episode(a_title)) {
        if p_ep != a_ep {
            return false;
        }
    }
    if p_title.eq_ignore_ascii_case(a_title) {
        return true;
    }
    let clean_p = p_title.to_lowercase().replace(['.', '_', '-'], " ");
    let clean_a = a_title.to_lowercase().replace(['.', '_', '-'], " ");
    let p_words: Vec<&str> = clean_p.split_whitespace().collect();
    let p_matches = !p_words.is_empty() && p_words.iter().all(|w| clean_a.contains(w));
    if p_matches {
        return true;
    }
    let a_words: Vec<&str> = clean_a.split_whitespace().collect();
    !a_words.is_empty() && a_words.iter().all(|w| clean_p.contains(w))
}

/// Evaluates agreement when both modalities yielded candidate matches.
fn classify_dual_match(
    phash: &PhashMatchSummary,
    p_title: &str,
    a_title: &str,
    a_shift: i64,
) -> MultimodalAgreement {
    let p_episode = phash.season.zip(phash.episode);
    let agreed = are_titles_matching(p_title, p_episode, a_title);
    let shift_diff = (phash.consensus_shift_ms - a_shift).abs();
    let status = match (agreed, shift_diff <= MAX_MULTIMODAL_AGREEMENT_DRIFT_MS) {
        (true, true) => "DUAL_MATCH_AGREED",
        (true, false) => "DUAL_MATCH_SHIFT_DIVERGENCE",
        (false, _) => "DUAL_MATCH_DISAGREED",
    };
    MultimodalAgreement {
        match_agreed: agreed,
        shift_diff_ms: Some(shift_diff),
        status: status.to_string(),
    }
}

/// Helper constructing single-modality or null agreement records.
fn build_single_agreement(status: &str) -> MultimodalAgreement {
    MultimodalAgreement {
        match_agreed: false,
        shift_diff_ms: None,
        status: status.to_string(),
    }
}

/// Compares pHash and aHash outcomes to classify multimodal agreement.
fn classify_agreement(phash: &PhashMatchSummary, ahash: &AhashMatchSummary) -> MultimodalAgreement {
    match (&phash.media_match, &ahash.media_match) {
        (Some(p_title), Some(a_title)) => {
            classify_dual_match(phash, p_title, a_title, ahash.consensus_shift_ms)
        }
        (Some(_), None) => build_single_agreement("PHASH_ONLY"),
        (None, Some(_)) => build_single_agreement("AHASH_ONLY"),
        (None, None) => build_single_agreement("NO_MATCH"),
    }
}

/// Updates running benchmark telemetry counters.
fn update_telemetry(
    t: &mut ScaleBenchmarkTelemetry,
    record: &MediaScaleBenchmarkRecord,
    elapsed_sec: f64,
    remaining: usize,
) {
    t.processed_this_run += 1;
    match record.agreement.status.as_str() {
        "DUAL_MATCH_AGREED" => t.dual_match_agreed += 1,
        "DUAL_MATCH_DISAGREED" | "DUAL_MATCH_SHIFT_DIVERGENCE" => t.dual_match_disagreed += 1,
        "PHASH_ONLY" => t.phash_only += 1,
        "AHASH_ONLY" => t.ahash_only += 1,
        "NO_MATCH" => t.no_match += 1,
        _ => t.errors += 1,
    }
    t.elapsed_sec = elapsed_sec;
    let n = t.processed_this_run as f64;
    t.avg_phash_sec = (t.avg_phash_sec * (n - 1.0) + record.phash.timer_sec) / n;
    t.avg_ahash_sec = (t.avg_ahash_sec * (n - 1.0) + record.ahash.timer_sec) / n;
    t.avg_total_sec = (t.avg_total_sec * (n - 1.0) + record.total_timer_sec) / n;
    t.eta_sec = t.avg_total_sec * remaining as f64;
}

/// Prints a concise live terminal update for the current video.
fn print_live_progress(
    done: usize,
    total: usize,
    record: &MediaScaleBenchmarkRecord,
    t: &ScaleBenchmarkTelemetry,
) {
    let p_desc = record
        .phash
        .media_match
        .as_deref()
        .map(|m| format!("'{}' ({:.0}%)", m, record.phash.confidence))
        .unwrap_or_else(|| "none".to_string());
    let a_desc = record
        .ahash
        .media_match
        .as_deref()
        .map(|m| format!("'{}' ({:.0}%)", m, record.ahash.confidence))
        .unwrap_or_else(|| "none".to_string());

    let eta_min = (t.eta_sec / 60.0).round() as u64;
    let pct = (done as f64 / total as f64) * 100.0;

    println!(
        "[{:>4}/{:<4}] ({:>5.1}%) | {:>4.1}s | pHash: {:<20} | aHash: {:<20} | {:<16} | ETA: {:>3}m",
        done, total, pct, record.total_timer_sec, p_desc, a_desc, record.agreement.status, eta_min
    );
}

/// Serializes and flushes live telemetry to status JSON file.
fn write_status_summary(path: &Path, telemetry: &ScaleBenchmarkTelemetry) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(telemetry)?;
    std::fs::write(path, data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phash_summary(title: &str, season: u32, episode: u32) -> PhashMatchSummary {
        let mut p = build_empty_phash_summary(0.0, 3);
        p.media_match = Some(title.to_string());
        p.season = Some(season);
        p.episode = Some(episode);
        p
    }

    #[test]
    fn test_episode_mismatch_rejected() {
        let p = phash_summary("Halt and Catch Fire", 4, 9);
        let a = "Halt.and.Catch.Fire.S04E04.1080p.WEB-DL";
        let agreement = classify_dual_match(&p, "Halt and Catch Fire", a, 0);
        assert!(!agreement.match_agreed);
        assert_eq!(agreement.status, "DUAL_MATCH_DISAGREED");
        assert!(!are_titles_matching("Halt and Catch Fire", Some((4, 9)), a));
    }

    #[test]
    fn test_same_episode_agrees() {
        let p = phash_summary("Halt and Catch Fire", 4, 4);
        let a = "Halt.and.Catch.Fire.S04E04.1080p.WEB-DL";
        let agreement = classify_dual_match(&p, "Halt and Catch Fire", a, 0);
        assert_eq!(agreement.status, "DUAL_MATCH_AGREED");
    }

    #[test]
    fn test_moviehash_alias_resolution() {
        const HASH: &str = "002835455e305309";
        const NAME: &str = "Tre Piani (2021)[MULTi][1080p].mkv";
        let dir = std::env::temp_dir().join(format!("dbsrt_bench_alias_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{HASH}.json")),
            format!("{{\"moviehash\": \"{HASH}\", \"media_name\": \"{NAME}\"}}"),
        )
        .unwrap();
        std::fs::write(dir.join("not_a_hash.json"), "{}").unwrap();
        let aliases = load_moviehash_aliases(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases.get(HASH).map(String::as_str), Some(NAME));
        assert!(is_moviehash_hex(HASH));
        assert!(!is_moviehash_hex("Tre Piani"));
    }

    #[test]
    fn test_no_audio_track_diagnostic() {
        let phash = phash_summary("Some Show", 1, 1);
        let ahash = build_empty_ahash_summary(3);
        let audio = ContainerAudioMetadata::default();
        let agreement = classify_agreement(&phash, &ahash);
        assert_eq!(agreement.status, "PHASH_ONLY");
        let input = DiagnosticInput {
            phash: &phash,
            ahash: &ahash,
            audio: &audio,
            agreement: &agreement,
            no_audio_stream: audio.track_count == 0,
        };
        let (cause, note) = no_audio_diagnostic(&input);
        assert_eq!(cause, CAUSE_NO_AUDIO_STREAM);
        assert!(note.contains("not an acoustic false negative"));
        assert!(ahash.windows.is_empty() && ahash.media_match.is_none());
    }
}
