//! Performance benchmarking subcommands for pHash, video decoding, and scene cut detection.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{
    detect_cuts, extract_frames_advanced, probe_video, read_phash_file, HwAccel, SamplingMode,
    VideoExtractionOptions,
};
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

use crate::constants::{
    DEFAULT_BASELINE_FPS, DEFAULT_BENCH_VIDEO_DURATION_SEC, DEFAULT_CUT_THRESHOLD,
    DEFAULT_PHASH_DATA_DIR, DEFAULT_SPARSE_FPS, DIVISION_BY_ZERO_FALLBACK_SEC, DUMMY_FRAME_BYTE,
    DUMMY_FRAME_SIZE, MICROS_PER_SEC, MILLIS_PER_SEC, SECONDS_PER_HOUR,
};

#[derive(Args, Debug)]
pub struct BenchCutsArgs {
    /// Directory containing .phash files
    #[arg(short, long, default_value = DEFAULT_PHASH_DATA_DIR)]
    pub phash_dir: PathBuf,

    /// Cut threshold Hamming distance
    #[arg(short = 't', long, default_value_t = DEFAULT_CUT_THRESHOLD)]
    pub threshold: u32,

    /// Force single-threaded execution to benchmark single-core performance
    #[arg(long)]
    pub single_thread: bool,
}

#[derive(Args, Debug)]
pub struct BenchVideoArgs {
    /// Media file to benchmark
    #[arg(short, long)]
    pub media: PathBuf,

    /// Duration in seconds to extract for benchmarking (defaults to 10.0s)
    #[arg(short, long, default_value_t = DEFAULT_BENCH_VIDEO_DURATION_SEC)]
    pub duration: f64,
}

/// Run synthetic in-memory pHash 32x32 benchmark.
pub fn run_bench_phash(count: usize) -> Result<()> {
    println!("Benchmarking pHash on {} frames...", count);
    let dummy_frame = [DUMMY_FRAME_BYTE; DUMMY_FRAME_SIZE];
    let start = Instant::now();
    let mut last_hash = 0;
    for _ in 0..count {
        last_hash ^= dbsrt_core::phash_32x32(&dummy_frame);
    }
    let elapsed = start.elapsed();
    let fps = count as f64 / elapsed.as_secs_f64();
    let per_frame_us = elapsed.as_secs_f64() * MICROS_PER_SEC / count as f64;
    println!("Result hash: {:016x}", last_hash);
    println!("Elapsed: {:?}", elapsed);
    println!(
        "Throughput: {:.0} frames/sec ({:.2} µs / frame)",
        fps, per_frame_us
    );
    Ok(())
}

/// Run multi-mode video decoding benchmark comparing CPU, HWAccel, sparse, and keyframe modes.
pub fn run_bench_video(args: &BenchVideoArgs) -> Result<()> {
    println!("Benchmarking video decoding on {:?}", args.media);
    let meta = probe_video(&args.media)?;
    println!(
        "Duration: {:.2}s, FPS: {:.2} (Total ~{} frames)\n",
        meta.duration,
        meta.fps,
        (meta.duration * meta.fps).round() as usize
    );

    let duration = args.duration.min(meta.duration);
    println!(
        "{:<26} | {:<12} | {:<12} | {:<14} | {:<10}",
        "Extraction Mode", "Frames", "Time (ms)", "Throughput", "Real-Time"
    );
    println!(
        "{:-<26}-+-{:-<12}-+-{:-<12}-+-{:-<14}-+-{:-<10}",
        "", "", "", "", ""
    );

    let benchmarks = [
        ("CPU Full (swscale)", SamplingMode::Full, HwAccel::Disabled),
        ("HWAccel Full", SamplingMode::Full, HwAccel::Auto),
        (
            "Sparse 1.0 FPS",
            SamplingMode::Fps(DEFAULT_SPARSE_FPS),
            HwAccel::Auto,
        ),
        ("KeyFrames Only", SamplingMode::KeyFrames, HwAccel::Auto),
    ];

    for (name, sampling, hwaccel) in benchmarks {
        let opts = VideoExtractionOptions {
            start_sec: 0.0,
            duration_sec: Some(duration),
            max_frames: None,
            sampling,
            hwaccel,
            custom_ffmpeg: None,
        };

        let t0 = Instant::now();
        match extract_frames_advanced(&args.media, &opts) {
            Ok(samples) => {
                let elapsed_s = t0.elapsed().as_secs_f64();
                let elapsed_ms = elapsed_s * MILLIS_PER_SEC;
                let nb_frames = samples.len();
                let fps = nb_frames as f64 / elapsed_s.max(DIVISION_BY_ZERO_FALLBACK_SEC);
                let speedup = duration / elapsed_s.max(DIVISION_BY_ZERO_FALLBACK_SEC);

                println!(
                    "{:<26} | {:<12} | {:<12.1} | {:<10.1} fps | {:<8.1}x",
                    name, nb_frames, elapsed_ms, fps, speedup
                );
            }
            Err(e) => {
                println!("{:<26} | FAILED ({})", name, e);
            }
        }
    }

    Ok(())
}

/// Run scene cut detection benchmark on a directory of .phash files.
pub fn run_bench_cuts(args: &BenchCutsArgs) -> Result<()> {
    let mut files = Vec::new();
    if !args.phash_dir.exists() {
        anyhow::bail!("Directory {:?} does not exist", args.phash_dir);
    }
    for entry in std::fs::read_dir(&args.phash_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("phash") {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        anyhow::bail!("No .phash files found in {:?}", args.phash_dir);
    }

    let mode_str = if args.single_thread {
        "Single-threaded"
    } else {
        "Multi-threaded (Rayon)"
    };
    println!("=== RUST NATIVE BENCHMARK: SCENE CUTS DETECTION ===");
    println!("Mode                  : {}", mode_str);
    println!("Files                 : {}", files.len());
    println!("Cut threshold         : {}", args.threshold);

    let start = Instant::now();
    let (total_frames, total_cuts): (usize, usize) = if args.single_thread {
        let mut frames = 0;
        let mut cuts = 0;
        for f in &files {
            let hashes = read_phash_file(f)?;
            frames += hashes.len();
            let detected = detect_cuts(&hashes, args.threshold);
            cuts += detected.len();
        }
        (frames, cuts)
    } else {
        files
            .par_iter()
            .map(|f| -> Result<(usize, usize)> {
                let hashes = read_phash_file(f)?;
                let frames = hashes.len();
                let cuts = detect_cuts(&hashes, args.threshold).len();
                Ok((frames, cuts))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .fold((0, 0), |acc, (f, c)| (acc.0 + f, acc.1 + c))
    };

    let elapsed = start.elapsed().as_secs_f64();
    let fps = total_frames as f64 / elapsed;
    let video_hours = (total_frames as f64 / DEFAULT_BASELINE_FPS) / SECONDS_PER_HOUR;
    let speedup = (video_hours * SECONDS_PER_HOUR) / elapsed;

    println!("Total frames          : {:>12}", total_frames);
    println!("Total cuts detected   : {:>12}", total_cuts);
    println!("Elapsed time          : {:>12.4} s", elapsed);
    println!("Throughput            : {:>12.0} frames/s", fps);
    println!(
        "Average cuts / file   : {:>12.1}",
        total_cuts as f64 / files.len() as f64
    );
    println!(
        "Equivalent video time : {:>12.1} hours of 24fps media",
        video_hours
    );
    println!("Processing speedup    : {:>12.0}x real-time", speedup);
    println!("===================================================");

    Ok(())
}
