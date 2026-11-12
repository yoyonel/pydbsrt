//! Benchmark runner for acoustic fingerprint (.ahash) cross-correlation matching.
//!
//! Evaluates temporal offset accuracy, discrimination against cross-media pairs,
//! and raw POPCNT scan throughput across directories of binary .ahash files.

use anyhow::{Context, Result};
use clap::Args;
use dbsrt_core::ahash::{match_audio_sequence, AhashFile};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::constants::{
    BENCH_AHASH_EXACT_TOLERANCE_SEC, DEFAULT_AHASH_DATA_DIR, DEFAULT_BENCH_AHASH_CROSS_TRIALS,
    DEFAULT_BENCH_AHASH_MAX_DISTANCE, DEFAULT_BENCH_AHASH_SAMPLES, DEFAULT_BENCH_AHASH_WINDOWS,
    MILLIS_PER_SEC, SECONDS_PER_HOUR,
};

/// CLI arguments for the acoustic matching benchmark subcommand.
#[derive(Args, Debug, Clone)]
pub struct BenchAhashArgs {
    /// Directory containing binary .ahash files.
    #[arg(short, long, default_value = DEFAULT_AHASH_DATA_DIR)]
    pub ahash_dir: PathBuf,

    /// Number of evaluation query trials per window size.
    #[arg(short, long, default_value_t = DEFAULT_BENCH_AHASH_SAMPLES)]
    pub samples: usize,

    /// Window sizes in seconds to evaluate (comma-separated, e.g. "3,5,10,15,30").
    #[arg(short, long, default_value = DEFAULT_BENCH_AHASH_WINDOWS)]
    pub windows: String,

    /// Number of negative cross-media trials.
    #[arg(long, default_value_t = DEFAULT_BENCH_AHASH_CROSS_TRIALS)]
    pub cross_media_trials: usize,

    /// Maximum Hamming distance threshold for matching.
    #[arg(long, default_value_t = DEFAULT_BENCH_AHASH_MAX_DISTANCE)]
    pub max_distance: f64,

    /// Optional path to save structured benchmark results as JSON.
    #[arg(long)]
    pub json_out: Option<PathBuf>,
}

/// Minimal deterministic 64-bit PRNG for reproducible benchmark sampling.
struct XorShift64(u64);

impl XorShift64 {
    const DEFAULT_SEED: u64 = 0x853c_49e6_748f_ea9b;

    fn new(seed: u64) -> Self {
        Self(if seed == 0 { Self::DEFAULT_SEED } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn gen_range(&mut self, max: usize) -> usize {
        if max == 0 {
            0
        } else {
            (self.next_u64() as usize) % max
        }
    }
}

/// High-level dataset summary metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AhashDatasetSummary {
    /// Total number of valid .ahash files loaded into memory.
    pub file_count: usize,
    /// Total cumulative frames (sub-fingerprints) across all loaded files.
    pub total_frames: usize,
    /// Cumulative media duration in hours represented by the dataset.
    pub total_duration_hours: f64,
    /// Total memory footprint occupied in RAM (bytes).
    pub memory_bytes: usize,
}

/// Evaluation metrics for a single query window duration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowAccuracyMetrics {
    /// Window duration in seconds.
    pub window_sec: f64,
    /// Number of evaluation trials evaluated.
    pub trial_count: usize,
    /// Percentage of queries successfully matching within threshold.
    pub match_rate_pct: f64,
    /// Percentage of queries having absolute offset error <= exact tolerance.
    pub exact_match_pct: f64,
    /// Mean absolute offset error in milliseconds.
    pub mean_error_ms: f64,
    /// Median absolute offset error in milliseconds.
    pub median_error_ms: f64,
    /// 95th percentile offset error in milliseconds.
    pub p95_error_ms: f64,
    /// Average Hamming distance across matching queries.
    pub mean_distance: f64,
    /// Average SNR bit margin over noise floor.
    pub mean_snr_bits: f64,
    /// Mean cross-correlation scan latency in microseconds.
    pub mean_latency_us: f64,
}

/// Metrics measuring discrimination against unrelated audio pairs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscriminationMetrics {
    /// Number of negative cross-media pairs evaluated.
    pub cross_trials: usize,
    /// Count of spurious matches violating the rejection threshold.
    pub false_positives: usize,
    /// False positive rate as a percentage.
    pub false_positive_rate_pct: f64,
    /// Mean Hamming distance observed between unrelated streams.
    pub mean_distance: f64,
    /// Minimum (closest) Hamming distance observed on unrelated streams.
    pub min_distance: f64,
}

/// Raw cross-correlation throughput metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchingThroughputMetrics {
    /// Total media hours scanned during the throughput benchmark.
    pub reference_hours_scanned: f64,
    /// Total elapsed wall time in seconds.
    pub elapsed_seconds: f64,
    /// Scan speed expressed in hours of audio scanned per second per core.
    pub hours_per_sec: f64,
    /// Throughput in millions of Hamming POPCNT operations per second.
    pub megahashes_per_sec: f64,
}

/// Complete benchmark audit payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AhashBenchmarkReport {
    /// ISO 8601 execution timestamp.
    pub timestamp: String,
    /// Dataset catalog summary.
    pub dataset: AhashDatasetSummary,
    /// Accuracy metrics across window sizes.
    pub windows: Vec<WindowAccuracyMetrics>,
    /// Discrimination and false-positive metrics.
    pub discrimination: DiscriminationMetrics,
    /// Scan throughput metrics.
    pub throughput: MatchingThroughputMetrics,
}

/// Loads all valid .ahash files from a given directory.
///
/// # Arguments
/// * `dir` - Directory path to scan for .ahash files.
///
/// # Returns
/// Vector of loaded tuples (PathBuf, AhashFile).
///
/// # Errors
/// Returns Err if the directory cannot be read.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Reads all directory entries, filters on `.ahash` extension, and parses each file in parallel.
pub fn load_ahash_files(dir: &Path) -> Result<Vec<(PathBuf, AhashFile)>> {
    let entries = fs::read_dir(dir).with_context(|| format!("Reading {:?}", dir))?;
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "ahash") {
            paths.push(path);
        }
    }
    paths.sort();

    let files: Vec<(PathBuf, AhashFile)> = paths
        .into_par_iter()
        .filter_map(|p| match AhashFile::read_from_path(&p) {
            Ok(file) if !file.hashes.is_empty() => Some((p, file)),
            _ => None,
        })
        .collect();

    Ok(files)
}

/// Computes summary metrics for loaded .ahash files.
///
/// # Arguments
/// * `files` - Slice of loaded .ahash file tuples.
///
/// # Returns
/// `AhashDatasetSummary` containing counts, duration and memory bytes.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Aggregates frame counts, durations in hours, and memory allocation size.
pub fn summarize_dataset(files: &[(PathBuf, AhashFile)]) -> AhashDatasetSummary {
    let mut total_frames = 0usize;
    let mut total_duration_secs = 0.0f64;
    let mut memory_bytes = 0usize;

    for (_, f) in files {
        let count = f.hashes.len();
        let fps = f.header.audio_fps();
        total_frames += count;
        if fps > 0.0 {
            total_duration_secs += count as f64 / fps;
        }
        memory_bytes += count * std::mem::size_of::<u64>();
    }

    AhashDatasetSummary {
        file_count: files.len(),
        total_frames,
        total_duration_hours: total_duration_secs / SECONDS_PER_HOUR,
        memory_bytes,
    }
}

/// Evaluates temporal offset accuracy for a specific query window duration.
///
/// # Arguments
/// * `files` - Loaded reference .ahash files.
/// * `window_sec` - Duration of the query snippet in seconds.
/// * `samples` - Number of evaluation trials to run.
/// * `max_distance` - Maximum allowed Hamming distance.
///
/// # Returns
/// `WindowAccuracyMetrics` with error distribution and match rates.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Extracts pseudo-random sub-slices with known ground-truth offsets, executes
/// `match_audio_sequence`, and records error distributions.
pub fn evaluate_window(
    files: &[(PathBuf, AhashFile)],
    window_sec: f64,
    samples: usize,
    max_distance: f64,
) -> WindowAccuracyMetrics {
    if files.is_empty() || samples == 0 {
        return WindowAccuracyMetrics {
            window_sec,
            trial_count: 0,
            match_rate_pct: 0.0,
            exact_match_pct: 0.0,
            mean_error_ms: 0.0,
            median_error_ms: 0.0,
            p95_error_ms: 0.0,
            mean_distance: 0.0,
            mean_snr_bits: 0.0,
            mean_latency_us: 0.0,
        };
    }

    let mut rng = XorShift64::new(0x42_1337 + (window_sec * 1000.0) as u64);
    let mut errors_ms = Vec::with_capacity(samples);
    let mut distances = Vec::with_capacity(samples);
    let mut snr_bits = Vec::with_capacity(samples);
    let mut latencies_us = Vec::with_capacity(samples);
    let mut exact_matches = 0usize;

    for _ in 0..samples {
        let file_idx = rng.gen_range(files.len());
        let (_, ref file) = &files[file_idx];
        let fps = file.header.audio_fps();
        let frame_count = (window_sec * fps).round() as usize;

        if frame_count == 0 || frame_count >= file.hashes.len() {
            continue;
        }

        let max_offset = file.hashes.len() - frame_count;
        let ground_truth_idx = rng.gen_range(max_offset);
        let query = &file.hashes[ground_truth_idx..ground_truth_idx + frame_count];

        let start = Instant::now();
        let res = match_audio_sequence(query, &file.hashes, fps, max_distance);
        let latency_us = start.elapsed().as_micros() as f64;
        latencies_us.push(latency_us);

        if let Some(m) = res {
            let error_sec = (m.ref_frame_offset as f64 / fps) - (ground_truth_idx as f64 / fps);
            let error_ms = error_sec.abs() * MILLIS_PER_SEC;
            errors_ms.push(error_ms);
            distances.push(m.mean_distance);
            snr_bits.push(m.snr_bits);

            if error_sec.abs() <= BENCH_AHASH_EXACT_TOLERANCE_SEC {
                exact_matches += 1;
            }
        }
    }

    let mut buffers = WindowObservationBuffers {
        errors_ms: &mut errors_ms,
        distances: &distances,
        snr_bits: &snr_bits,
        latencies_us: &latencies_us,
    };

    compute_window_summary(window_sec, samples, exact_matches, &mut buffers)
}

/// Internal observation buffers holding metric series for window statistical aggregation.
struct WindowObservationBuffers<'a> {
    errors_ms: &'a mut [f64],
    distances: &'a [f64],
    snr_bits: &'a [f64],
    latencies_us: &'a [f64],
}

/// Helper calculating statistical percentiles and averages for window evaluation.
fn compute_window_summary(
    window_sec: f64,
    samples: usize,
    exact_matches: usize,
    buffers: &mut WindowObservationBuffers<'_>,
) -> WindowAccuracyMetrics {
    let match_count = buffers.errors_ms.len();
    if match_count == 0 {
        return WindowAccuracyMetrics {
            window_sec,
            trial_count: samples,
            match_rate_pct: 0.0,
            exact_match_pct: 0.0,
            mean_error_ms: 0.0,
            median_error_ms: 0.0,
            p95_error_ms: 0.0,
            mean_distance: 0.0,
            mean_snr_bits: 0.0,
            mean_latency_us: 0.0,
        };
    }

    buffers
        .errors_ms
        .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_idx = match_count / 2;
    let p95_idx = ((match_count as f64 * 0.95).round() as usize).min(match_count - 1);

    let sum_err: f64 = buffers.errors_ms.iter().sum();
    let sum_dist: f64 = buffers.distances.iter().sum();
    let sum_snr: f64 = buffers.snr_bits.iter().sum();
    let sum_lat: f64 = buffers.latencies_us.iter().sum();

    WindowAccuracyMetrics {
        window_sec,
        trial_count: samples,
        match_rate_pct: (match_count as f64 / samples as f64) * 100.0,
        exact_match_pct: (exact_matches as f64 / samples as f64) * 100.0,
        mean_error_ms: sum_err / match_count as f64,
        median_error_ms: buffers.errors_ms[median_idx],
        p95_error_ms: buffers.errors_ms[p95_idx],
        mean_distance: sum_dist / match_count as f64,
        mean_snr_bits: sum_snr / match_count as f64,
        mean_latency_us: sum_lat / buffers.latencies_us.len() as f64,
    }
}

/// Evaluates discrimination and rejection against unrelated cross-media pairs.
///
/// # Arguments
/// * `files` - Loaded reference .ahash files.
/// * `trials` - Number of cross-media negative pairs to test.
/// * `max_distance` - Threshold distance used for matching.
///
/// # Returns
/// `DiscriminationMetrics` tracking false positive count and distance separation.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Selects pairs of distinct media files (A != B), matches query from A against reference B,
/// and asserts that spurious match count is zero.
pub fn evaluate_discrimination(
    files: &[(PathBuf, AhashFile)],
    trials: usize,
    max_distance: f64,
) -> DiscriminationMetrics {
    if files.len() < 2 || trials == 0 {
        return DiscriminationMetrics {
            cross_trials: trials,
            false_positives: 0,
            false_positive_rate_pct: 0.0,
            mean_distance: 0.0,
            min_distance: 0.0,
        };
    }

    const TEST_WINDOW_SEC: f64 = 15.0;
    let mut rng = XorShift64::new(0xdead_beef_1234);
    let mut false_positives = 0usize;
    let mut all_min_distances = Vec::with_capacity(trials);

    for _ in 0..trials {
        let idx_a = rng.gen_range(files.len());
        let mut idx_b = rng.gen_range(files.len());
        while idx_b == idx_a {
            idx_b = rng.gen_range(files.len());
        }

        let (_, file_a) = &files[idx_a];
        let (_, file_b) = &files[idx_b];
        let fps_a = file_a.header.audio_fps();
        let fps_b = file_b.header.audio_fps();
        let frame_count = (TEST_WINDOW_SEC * fps_a).round() as usize;

        if frame_count == 0
            || frame_count >= file_a.hashes.len()
            || frame_count >= file_b.hashes.len()
        {
            continue;
        }

        let max_offset_a = file_a.hashes.len() - frame_count;
        let start_a = rng.gen_range(max_offset_a);
        let query = &file_a.hashes[start_a..start_a + frame_count];

        if let Some(m) = match_audio_sequence(query, &file_b.hashes, fps_b, max_distance) {
            false_positives += 1;
            all_min_distances.push(m.mean_distance);
        } else {
            all_min_distances.push(calc_min_scan_distance(query, &file_b.hashes));
        }
    }

    let min_dist = all_min_distances
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let sum_dist: f64 = all_min_distances.iter().sum();
    let mean_dist = if all_min_distances.is_empty() {
        0.0
    } else {
        sum_dist / all_min_distances.len() as f64
    };

    DiscriminationMetrics {
        cross_trials: trials,
        false_positives,
        false_positive_rate_pct: (false_positives as f64 / trials as f64) * 100.0,
        mean_distance: mean_dist,
        min_distance: if min_dist.is_finite() { min_dist } else { 0.0 },
    }
}

/// Helper computing minimum brute-force Hamming distance for baseline noise tracking.
fn calc_min_scan_distance(query: &[u64], reference: &[u64]) -> f64 {
    let q_len = query.len();
    if reference.len() < q_len || q_len == 0 {
        return 32.0;
    }
    let mut min_total = u64::MAX;
    for shift in 0..=(reference.len() - q_len) {
        let mut dist = 0u64;
        for (q, r) in query.iter().zip(&reference[shift..shift + q_len]) {
            dist += (*q ^ *r).count_ones() as u64;
        }
        if dist < min_total {
            min_total = dist;
        }
    }
    min_total as f64 / q_len as f64
}

/// Measures raw cross-correlation scan throughput across the loaded dataset.
///
/// # Arguments
/// * `files` - Loaded reference .ahash files.
/// * `query_sec` - Duration of synthetic query in seconds.
///
/// # Returns
/// `MatchingThroughputMetrics` quantifying scan speed and operations per second.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Evaluates sliding POPCNT over multiple media files and computes operational rates.
pub fn evaluate_throughput(
    files: &[(PathBuf, AhashFile)],
    query_sec: f64,
) -> MatchingThroughputMetrics {
    if files.is_empty() {
        return MatchingThroughputMetrics {
            reference_hours_scanned: 0.0,
            elapsed_seconds: 0.0,
            hours_per_sec: 0.0,
            megahashes_per_sec: 0.0,
        };
    }

    let (_, ref sample_file) = files[0];
    let fps = sample_file.header.audio_fps();
    let q_len = (query_sec * fps).round() as usize;
    let query: Vec<u64> = sample_file.hashes.iter().take(q_len).copied().collect();

    let mut total_scanned_hashes = 0usize;
    let start = Instant::now();

    for (_, f) in files.iter().take(50) {
        if f.hashes.len() > q_len {
            let max_shift = f.hashes.len() - q_len;
            total_scanned_hashes += max_shift * q_len;
            let _ = match_audio_sequence(&query, &f.hashes, fps, 32.0);
        }
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.000_001);
    let total_hours = (total_scanned_hashes as f64 / fps) / SECONDS_PER_HOUR;

    MatchingThroughputMetrics {
        reference_hours_scanned: total_hours,
        elapsed_seconds: elapsed,
        hours_per_sec: total_hours / elapsed,
        megahashes_per_sec: (total_scanned_hashes as f64 / elapsed) / 1_000_000.0,
    }
}

/// Main entry point executing the comprehensive acoustic matching benchmark.
///
/// # Arguments
/// * `args` - Parsed CLI options controlling corpus directory, trials, and outputs.
///
/// # Returns
/// `Ok(())` on success.
///
/// # Errors
/// Returns Err if the .ahash files cannot be loaded or JSON output cannot be saved.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Loads files from disk.
/// 2. Executes temporal accuracy evaluation across window sizes.
/// 3. Executes discrimination tests.
/// 4. Executes raw throughput benchmark.
/// 5. Prints human-readable tables and serializes JSON payload if requested.
pub fn run_bench_ahash(args: &BenchAhashArgs) -> Result<()> {
    println!(
        "🔍 [1/4] Loading .ahash dataset from {:?}...",
        args.ahash_dir
    );
    let start_load = Instant::now();
    let files = load_ahash_files(&args.ahash_dir)?;
    if files.is_empty() {
        anyhow::bail!("No valid .ahash files found in {:?}", args.ahash_dir);
    }
    let summary = summarize_dataset(&files);
    println!(
        "   Loaded {} files ({} frames, {:.2} hours of media, {:.2} MB RAM) in {:.2}s",
        summary.file_count,
        summary.total_frames,
        summary.total_duration_hours,
        summary.memory_bytes as f64 / (1024.0 * 1024.0),
        start_load.elapsed().as_secs_f64()
    );

    println!("\n⏱️  [2/4] Evaluating temporal offset accuracy (Axe 1)...");
    let mut window_metrics = Vec::new();
    let window_sizes: Vec<f64> = args
        .windows
        .split(',')
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .collect();

    for &w in &window_sizes {
        let m = evaluate_window(&files, w, args.samples, args.max_distance);
        println!(
            "   • Window {:>4.1}s: Match Rate: {:>5.1}% | Median Error: {:>5.2} ms | p95 Error: {:>5.2} ms | SNR: {:>4.1} bits | Scan: {:>6.0} µs",
            m.window_sec, m.match_rate_pct, m.median_error_ms, m.p95_error_ms, m.mean_snr_bits, m.mean_latency_us
        );
        window_metrics.push(m);
    }

    println!("\n🛡️  [3/4] Evaluating discrimination & negative rejection (Axe 2)...");
    let disc = evaluate_discrimination(&files, args.cross_media_trials, args.max_distance);
    println!(
        "   • Evaluated {} cross-media pairs: False Positives = {} ({:.2}%) | Mean Dist = {:.2} bits | Min Dist = {:.2} bits",
        disc.cross_trials, disc.false_positives, disc.false_positive_rate_pct, disc.mean_distance, disc.min_distance
    );

    println!("\n⚡ [4/4] Evaluating POPCNT throughput & scan latency (Axe 3)...");
    const THROUGHPUT_WINDOW_SEC: f64 = 15.0;
    let tp = evaluate_throughput(&files, THROUGHPUT_WINDOW_SEC);
    println!(
        "   • Throughput: {:.1} hours/sec/core | {:.2} MHashes/sec (Scanned {:.2} hours in {:.3}s)",
        tp.hours_per_sec, tp.megahashes_per_sec, tp.reference_hours_scanned, tp.elapsed_seconds
    );

    let report = AhashBenchmarkReport {
        timestamp: chrono_iso_now(),
        dataset: summary,
        windows: window_metrics,
        discrimination: disc,
        throughput: tp,
    };

    if let Some(ref out_path) = args.json_out {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json_text = serde_json::to_string_pretty(&report)?;
        fs::write(out_path, json_text)?;
        println!(
            "\n✅ Structured benchmark metrics written to {:?}",
            out_path
        );
    }

    Ok(())
}

/// Helper producing an ISO 8601 UTC timestamp string without pulling chrono.
fn chrono_iso_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{}-unix", secs)
}
