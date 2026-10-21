//! pHash binary file extraction subcommand.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{
    extract_frames_advanced, probe_video, write_phash_file, SamplingMode, VideoExtractionOptions,
};
use std::path::PathBuf;
use std::time::Instant;

use crate::cli::CliHwAccel;
use crate::constants::DIVISION_BY_ZERO_FALLBACK_SEC;

#[derive(Args, Debug)]
pub struct ExportPhashArgs {
    /// Media file to extract phash from
    #[arg(short, long)]
    pub media: PathBuf,

    /// Output .phash file (defaults to /tmp/<name>.<fps>fps.phash)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Optional duration in seconds to extract (defaults to full media)
    #[arg(short, long)]
    pub duration: Option<f64>,

    /// Optional sampling rate in FPS (e.g. 1.0 for sparse fast fingerprints)
    #[arg(long)]
    pub fps: Option<f64>,

    /// Extract keyframes (I-frames) only
    #[arg(long)]
    pub keyframes: bool,

    /// Hardware acceleration backend (auto, disabled, vaapi, cuda, qsv)
    #[arg(long, value_enum, default_value_t = CliHwAccel::Disabled)]
    pub hwaccel: CliHwAccel,
}

/// Run frame extraction and write binary .phash file.
pub fn run_export_phash(args: &ExportPhashArgs) -> Result<()> {
    let start = Instant::now();
    println!("Probing media {:?}...", args.media);
    let meta = probe_video(&args.media)?;
    println!(
        "Media duration: {:.2}s, FPS: {:.3}",
        meta.duration, meta.fps
    );

    let duration = args.duration.unwrap_or(meta.duration);
    let sampling = if args.keyframes {
        SamplingMode::KeyFrames
    } else if let Some(target_fps) = args.fps {
        SamplingMode::Fps(target_fps)
    } else {
        SamplingMode::Full
    };

    let total_frames = (duration * meta.fps).round() as usize;

    let opts = VideoExtractionOptions {
        start_sec: 0.0,
        duration_sec: if sampling == SamplingMode::Full {
            None
        } else {
            Some(duration)
        },
        max_frames: if sampling == SamplingMode::Full {
            Some(total_frames)
        } else {
            None
        },
        sampling,
        hwaccel: args.hwaccel.into(),
        custom_ffmpeg: None,
    };

    let out_path = args.output.clone().unwrap_or_else(|| {
        let name = args.media.file_name().unwrap().to_string_lossy();
        let suffix = match sampling {
            SamplingMode::KeyFrames => "keyframes.phash".to_string(),
            SamplingMode::Fps(fps) => format!("{:.1}fps.phash", fps),
            SamplingMode::Full => format!("{:.3}fps.phash", meta.fps),
        };
        PathBuf::from(format!("/tmp/{}.{}", name, suffix))
    });

    println!(
        "Extracting pHash ({:?}, hwaccel={:?}) -> {:?}...",
        sampling, args.hwaccel, out_path
    );

    let samples = extract_frames_advanced(&args.media, &opts)?;
    let elapsed = start.elapsed().as_secs_f64();
    let hashes: Vec<u64> = samples.into_iter().map(|s| s.phash).collect();
    println!(
        "Computed {} frame hashes in {:.2}s ({:.0} fps, {:.1}x real-time)",
        hashes.len(),
        elapsed,
        hashes.len() as f64 / elapsed.max(DIVISION_BY_ZERO_FALLBACK_SEC),
        duration / elapsed.max(DIVISION_BY_ZERO_FALLBACK_SEC)
    );

    write_phash_file(&out_path, &hashes)?;
    println!("✓ Successfully wrote binary phash to {:?}", out_path);
    Ok(())
}
