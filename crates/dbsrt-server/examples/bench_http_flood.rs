//! End-to-end HTTP load test and flooding benchmark against local `dbsrt-server`.
//!
//! Evaluates server saturation, queueing, throughput (QPS), and latency percentiles (P50, P90, P95, P99)
//! under massive concurrent client requests, comparing Dense queries (15 frames) vs Anchor queries (4 frames).

use anyhow::{Context, Result};
use dbsrt_index::MmapDbphash;
use reqwest::Client;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;

pub const DEFAULT_TARGET_URL: &str = "http://127.0.0.1:8769/v1/recognize";
pub const DEFAULT_CATALOG_PATH: &str = "data/catalog.dbphash";
pub const DEFAULT_TOTAL_REQUESTS: usize = 1_000;
pub const DEFAULT_CONCURRENCY: usize = 32;
pub const DENSE_QUERY_FRAME_COUNT: usize = 15;
pub const ANCHOR_QUERY_FRAME_COUNT: usize = 4;
pub const WARMUP_REQUESTS: usize = 20;
pub const DEFAULT_SEARCH_DISTANCE: u32 = 4;
pub const MILLIS_PER_SEC: f64 = 1000.0;
pub const PERCENTILE_P50: f64 = 0.50;
pub const PERCENTILE_P90: f64 = 0.90;
pub const PERCENTILE_P95: f64 = 0.95;
pub const PERCENTILE_P99: f64 = 0.99;
pub const QUERY_POOL_CAPACITY: usize = 50;

/// Statistical summary of an HTTP benchmark run.
#[derive(Debug, Clone)]
pub struct BenchmarkStats {
    pub label: &'static str,
    pub total_requests: usize,
    pub success_count: usize,
    pub error_count: usize,
    pub elapsed_seconds: f64,
    pub qps: f64,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub avg_ms: f64,
}

/// Prepares realistic query payload strings sampled from real catalog entries.
///
/// # Arguments
/// * `reader` - Open mmap reader for the binary catalog.
/// * `frame_count` - Number of frames per query payload.
/// * `pool_size` - Number of unique payloads to generate.
///
/// # Returns
/// * `Vec<serde_json::Value>` - Pre-formatted JSON request bodies.
///
/// # Errors
/// * Guarantee: Never returns Err.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Reads cuts entries from the loaded catalog.
/// 2. Chunks cuts into consecutive windows of `frame_count`.
/// 3. Builds `RecognizeRequest` JSON payloads formatted with hex strings.
pub fn generate_query_pool(
    reader: &MmapDbphash,
    frame_count: usize,
    pool_size: usize,
) -> Vec<serde_json::Value> {
    let cuts = reader.cuts_entries();
    let mut pool = Vec::with_capacity(pool_size);

    let step = (cuts.len() / (pool_size + 1)).max(1);
    for i in 0..pool_size {
        let start = i * step;
        if start + frame_count <= cuts.len() {
            let hashes: Vec<String> = cuts[start..start + frame_count]
                .iter()
                .map(|c| format!("0x{:016x}", c.phash))
                .collect();

            let payload = json!({
                "query_fingerprint": hashes,
                "search_distance": DEFAULT_SEARCH_DISTANCE,
                "nb_seconds": 15.0,
            });
            pool.push(payload);
        }
    }

    pool
}

/// Executes a concurrent HTTP benchmark scenario.
///
/// # Arguments
/// * `client` - Shared HTTP client with connection pooling.
/// * `target_url` - Full URL for the target `/v1/recognize` endpoint.
/// * `payload_pool` - Slice of JSON request payloads to cycle through.
/// * `concurrency` - Number of concurrent worker tasks.
/// * `total_requests` - Total number of HTTP requests to execute.
/// * `label` - Human-readable scenario label.
///
/// # Returns
/// * `Result<BenchmarkStats>` - Calculated latency percentiles and throughput metrics.
///
/// # Errors
/// * Returns `Err` if HTTP client execution encounters fatal socket errors.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Uses a Tokio `Semaphore` to strictly enforce `concurrency` bounds.
/// 2. Dispatches `total_requests` across asynchronous worker tasks.
/// 3. Measures individual request latency with `Instant::now()`.
/// 4. Aggregates response latencies into a thread-safe synchronized buffer.
/// 5. Computes P50, P90, P95, P99 percentiles on the sorted latency distribution.
pub async fn run_flood_scenario(
    client: &Client,
    target_url: &str,
    payload_pool: &[serde_json::Value],
    concurrency: usize,
    total_requests: usize,
    label: &'static str,
) -> Result<BenchmarkStats> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let success_counter = Arc::new(AtomicUsize::new(0));
    let error_counter = Arc::new(AtomicUsize::new(0));

    let latencies = Arc::new(tokio::sync::Mutex::new(Vec::with_capacity(total_requests)));
    let pool_len = payload_pool.len().max(1);

    let t_start = Instant::now();
    let mut tasks = Vec::with_capacity(total_requests);

    for req_idx in 0..total_requests {
        let sem = semaphore.clone();
        let client_clone = client.clone();
        let url = target_url.to_string();
        let payload = payload_pool[req_idx % pool_len].clone();
        let succ = success_counter.clone();
        let err = error_counter.clone();
        let lats = latencies.clone();

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let req_t0 = Instant::now();
            let resp = client_clone.post(&url).json(&payload).send().await;

            let dur_ms = req_t0.elapsed().as_secs_f64() * MILLIS_PER_SEC;
            match resp {
                Ok(r) if r.status().is_success() => {
                    succ.fetch_add(1, Ordering::Relaxed);
                    let mut lock = lats.lock().await;
                    lock.push(dur_ms);
                }
                _ => {
                    err.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        tasks.push(handle);
    }

    for task in tasks {
        let _ = task.await;
    }

    let elapsed = t_start.elapsed().as_secs_f64();
    let mut lat_vec = latencies.lock().await.clone();
    lat_vec.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let succ_total = success_counter.load(Ordering::Relaxed);
    let err_total = error_counter.load(Ordering::Relaxed);
    let qps = if elapsed > 0.0 {
        succ_total as f64 / elapsed
    } else {
        0.0
    };

    let (min_ms, p50_ms, p90_ms, p95_ms, p99_ms, max_ms, avg_ms) = if !lat_vec.is_empty() {
        let n = lat_vec.len();
        let p50 = lat_vec[((n as f64 * PERCENTILE_P50) as usize).min(n - 1)];
        let p90 = lat_vec[((n as f64 * PERCENTILE_P90) as usize).min(n - 1)];
        let p95 = lat_vec[((n as f64 * PERCENTILE_P95) as usize).min(n - 1)];
        let p99 = lat_vec[((n as f64 * PERCENTILE_P99) as usize).min(n - 1)];
        let min = lat_vec[0];
        let max = lat_vec[n - 1];
        let avg = lat_vec.iter().sum::<f64>() / n as f64;
        (min, p50, p90, p95, p99, max, avg)
    } else {
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
    };

    Ok(BenchmarkStats {
        label,
        total_requests,
        success_count: succ_total,
        error_count: err_total,
        elapsed_seconds: elapsed,
        qps,
        min_ms,
        p50_ms,
        p90_ms,
        p95_ms,
        p99_ms,
        max_ms,
        avg_ms,
    })
}

/// Benchmark configuration options parsed from CLI or defaults.
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub url: String,
    pub requests: usize,
    pub concurrency: usize,
    pub dense_frames: usize,
    pub anchor_frames: usize,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            url: DEFAULT_TARGET_URL.to_string(),
            requests: DEFAULT_TOTAL_REQUESTS,
            concurrency: DEFAULT_CONCURRENCY,
            dense_frames: DENSE_QUERY_FRAME_COUNT,
            anchor_frames: ANCHOR_QUERY_FRAME_COUNT,
        }
    }
}

/// Parses CLI arguments into structured configuration.
fn parse_args() -> BenchmarkConfig {
    let args: Vec<String> = std::env::args().collect();
    let mut config = BenchmarkConfig::default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--url" if i + 1 < args.len() => {
                config.url = args[i + 1].clone();
                i += 2;
            }
            "--requests" | "-n" if i + 1 < args.len() => {
                if let Ok(val) = args[i + 1].parse() {
                    config.requests = val;
                }
                i += 2;
            }
            "--concurrency" | "-c" if i + 1 < args.len() => {
                if let Ok(val) = args[i + 1].parse() {
                    config.concurrency = val;
                }
                i += 2;
            }
            "--dense-frames" if i + 1 < args.len() => {
                if let Ok(val) = args[i + 1].parse() {
                    config.dense_frames = val;
                }
                i += 2;
            }
            "--anchor-frames" if i + 1 < args.len() => {
                if let Ok(val) = args[i + 1].parse() {
                    config.anchor_frames = val;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    config
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = parse_args();

    println!("================================================================================");
    println!(" 🚀 DBSrt Server: End-to-End HTTP Matching Flood Benchmark");
    println!("================================================================================");
    println!("Target URL:       {}", cfg.url);
    println!("Total Requests:   {} per scenario", cfg.requests);
    println!("Concurrency:      {} concurrent clients", cfg.concurrency);
    println!("Dense Frames:     {} frames/query", cfg.dense_frames);
    println!("Anchor Frames:    {} frames/query", cfg.anchor_frames);

    let catalog_path = Path::new(DEFAULT_CATALOG_PATH);
    if !catalog_path.exists() {
        eprintln!("❌ Error: Catalog not found at {:?}", catalog_path);
        std::process::exit(1);
    }

    println!("📂 Loading catalog metadata from {:?}...", catalog_path);
    let reader = MmapDbphash::open(catalog_path).context("Failed to open catalog")?;
    println!(
        "✓ Loaded {} medias, {} cuts, {} sparse frames.",
        reader.media_entries().len(),
        reader.cuts_entries().len(),
        reader.sparse_entries().len()
    );

    let dense_pool = generate_query_pool(&reader, cfg.dense_frames, QUERY_POOL_CAPACITY);
    let anchor_pool = generate_query_pool(&reader, cfg.anchor_frames, QUERY_POOL_CAPACITY);

    let client = Client::builder()
        .pool_max_idle_per_host(cfg.concurrency + 10)
        .build()?;

    // 1. Healthcheck probe
    print!("⏳ Probing target service health... ");
    let health_url = cfg.url.replace("/v1/recognize", "/v1/health");
    let resp = client.get(&health_url).send().await;
    match resp {
        Ok(r) if r.status().is_success() => println!("✓ Service is UP (HTTP 200)"),
        _ => {
            println!("❌ FAILED. Is dbsrt-server running at {}?", cfg.url);
            std::process::exit(1);
        }
    }

    // 2. Warm-up
    print!(
        "🔥 Warming up connection pool ({} requests)... ",
        WARMUP_REQUESTS
    );
    let _ = run_flood_scenario(
        &client,
        &cfg.url,
        &dense_pool,
        cfg.concurrency.min(10),
        WARMUP_REQUESTS,
        "Warmup",
    )
    .await?;
    println!("✓ Done");

    // 3. Scenario A: Dense Flood
    println!(
        "\n⚡ Running Scenario A: DENSE MATCHING ({} frames / query)...",
        cfg.dense_frames
    );
    let stats_dense = run_flood_scenario(
        &client,
        &cfg.url,
        &dense_pool,
        cfg.concurrency,
        cfg.requests,
        "Dense",
    )
    .await?;

    // 4. Scenario B: Anchor Flood
    println!(
        "⚡ Running Scenario B: ANCHOR MATCHING ({} frames / query)...",
        cfg.anchor_frames
    );
    let stats_anchor = run_flood_scenario(
        &client,
        &cfg.url,
        &anchor_pool,
        cfg.concurrency,
        cfg.requests,
        "Anchor",
    )
    .await?;

    // 5. Comparison Results
    println!("\n================================================================================");
    println!(
        " 📊 BENCHMARK RESULTS : DENSE vs ANCHORS UNDER CONCURRENCY ({})",
        cfg.concurrency
    );
    println!("================================================================================");
    println!(
        "| {:<18} | {:>8} | {:>10} | {:>8} | {:>8} | {:>8} | {:>8} |",
        "Mode", "Throughput", "Success", "P50 Lat", "P90 Lat", "P95 Lat", "P99 Lat"
    );
    println!("|:-------------------|---------:|-----------:|---------:|---------:|---------:|---------:|");

    for s in [&stats_dense, &stats_anchor] {
        println!(
            "| {:<18} | {:>6.1} QPS | {:>6}/{} | {:>6.2} ms | {:>6.2} ms | {:>6.2} ms | {:>6.2} ms |",
            s.label,
            s.qps,
            s.success_count,
            s.total_requests,
            s.p50_ms,
            s.p90_ms,
            s.p95_ms,
            s.p99_ms
        );
    }
    println!("================================================================================");

    if stats_dense.qps > 0.0 {
        let speedup = stats_anchor.qps / stats_dense.qps;
        let p50_gain = (1.0 - (stats_anchor.p50_ms / stats_dense.p50_ms.max(0.001))) * 100.0;
        println!(
            "🚀 Summary: Anchor mode delivers {:.2}x higher throughput with {:.1}% lower P50 latency.",
            speedup, p50_gain
        );
    }

    Ok(())
}
