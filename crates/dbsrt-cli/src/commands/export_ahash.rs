//! Haitsma-Kalker audio spectral hash (.ahash) extraction CLI command.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{
    extract_ahash, AhashFile, AudioExtractionOptions, DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE,
};
use std::path::PathBuf;
use std::time::Instant;

use crate::constants::{
    DEFAULT_AHASH_HOP_SIZE, DEFAULT_AHASH_SAMPLE_RATE, DIVISION_BY_ZERO_FALLBACK_SEC,
};

/// Command-line arguments for extracting Haitsma-Kalker audio hashes.
#[derive(Args, Debug, Clone)]
pub struct ExportAhashArgs {
    /// Media file path or stream URI to extract audio hash from.
    #[arg(short, long)]
    pub media: PathBuf,

    /// Output .ahash file path (defaults to /tmp/<name>.<fps>fps.ahash).
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Audio sampling rate in Hz (default: 5512).
    #[arg(long, default_value_t = DEFAULT_AHASH_SAMPLE_RATE)]
    pub sample_rate: u32,

    /// Hop size in samples (64 for ~86.1 fps, 230 for ~24.0 fps video sync).
    #[arg(long, default_value_t = DEFAULT_AHASH_HOP_SIZE)]
    pub hop_size: usize,

    /// Optional start seek offset in seconds.
    #[arg(long, default_value_t = 0.0)]
    pub start_sec: f64,

    /// Optional maximum duration in seconds to process.
    #[arg(short, long)]
    pub duration: Option<f64>,
}

/// Runs Haitsma-Kalker audio hash extraction and writes an .ahash v1.0 binary file.
///
/// # Arguments
/// * `args` - Parsed CLI options for audio extraction.
///
/// # Returns
/// `Ok(())` on successful extraction and serialization.
///
/// # Errors
/// Returns `Err` if FFmpeg fails, audio stream cannot be decoded, or output file write fails.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Configures `AudioExtractionOptions` from CLI parameters.
/// 2. Spawns FFmpeg and streams mono f32le audio into `extract_ahash`.
/// 3. Computes 64-bit Haitsma-Kalker hashes via sliding Hann FFT analysis.
/// 4. Determines default target path `<stem>.<fps>fps.ahash` if none provided.
/// 5. Writes header (176 bytes, 8-byte aligned) and little-endian payload atomically.
/// 6. Prints execution throughput and real-time speed factor.
pub fn run_export_ahash(args: &ExportAhashArgs) -> Result<()> {
    let start_time = Instant::now();
    println!(
        "Extracting Haitsma-Kalker audio hash from {:?}...",
        args.media
    );

    let opts = AudioExtractionOptions {
        sample_rate: args.sample_rate,
        frame_size: DEFAULT_FRAME_SIZE,
        hop_size: if args.hop_size == 0 {
            DEFAULT_HOP_SIZE
        } else {
            args.hop_size
        },
        start_sec: args.start_sec,
        duration_sec: args.duration,
        custom_ffmpeg: None,
    };

    let ahash_file = extract_ahash(&args.media, &opts)?;
    let elapsed = start_time.elapsed().as_secs_f64();

    let fps = ahash_file.header.audio_fps();
    let frame_count = ahash_file.hashes.len();
    let effective_duration = if fps > 0.0 {
        frame_count as f64 / fps
    } else {
        0.0
    };
    let speed_factor = effective_duration / elapsed.max(DIVISION_BY_ZERO_FALLBACK_SEC);

    println!(
        "Extracted {} audio hashes in {:.3}s ({:.1} fps, {:.1}x real-time, duration: {:.2}s)",
        frame_count, elapsed, fps, speed_factor, effective_duration
    );

    let target_path = resolve_output_path(args, fps);
    AhashFile::write_to_path(&target_path, &ahash_file.header, &ahash_file.hashes)?;

    println!(
        "✓ Successfully wrote .ahash v1.0 binary to {:?}",
        target_path
    );
    Ok(())
}

/// Resolves the destination output file path.
///
/// # Arguments
/// * `args` - CLI extraction arguments.
/// * `fps` - Effective audio framerate.
///
/// # Returns
/// Destination `PathBuf`.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Returns explicitly provided `--output` or defaults to `/tmp/<media_file_name>.<fps>fps.ahash`.
fn resolve_output_path(args: &ExportAhashArgs, fps: f64) -> PathBuf {
    if let Some(ref path) = args.output {
        path.clone()
    } else {
        let name = args.media.file_name().unwrap_or_default().to_string_lossy();
        PathBuf::from(format!("/tmp/{}.{:.1}fps.ahash", name, fps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_output_path_default() {
        let args = ExportAhashArgs {
            media: PathBuf::from("movie.mkv"),
            output: None,
            sample_rate: 5512,
            hop_size: 64,
            start_sec: 0.0,
            duration: None,
        };
        let path = resolve_output_path(&args, 86.125);
        assert_eq!(path, PathBuf::from("/tmp/movie.mkv.86.1fps.ahash"));
    }

    #[test]
    fn test_resolve_output_path_explicit() {
        let explicit = PathBuf::from("/data/custom.ahash");
        let args = ExportAhashArgs {
            media: PathBuf::from("movie.mkv"),
            output: Some(explicit.clone()),
            sample_rate: 5512,
            hop_size: 64,
            start_sec: 0.0,
            duration: None,
        };
        let path = resolve_output_path(&args, 86.125);
        assert_eq!(path, explicit);
    }

    #[test]
    fn test_run_export_ahash_real_media() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let media = manifest_dir.join("../../tests/data/big_buck_bunny_trailer_480p.mp4");
        if !media.exists() {
            eprintln!("Skipping: media fixture not found at {:?}", media);
            return;
        }

        let out_path =
            std::env::temp_dir().join(format!("test_ahash_export_{}.ahash", std::process::id()));
        let args = ExportAhashArgs {
            media,
            output: Some(out_path.clone()),
            sample_rate: 5512,
            hop_size: 64,
            start_sec: 0.0,
            duration: Some(1.0),
        };

        let res = run_export_ahash(&args);
        assert!(res.is_ok(), "run_export_ahash should succeed: {:?}", res);
        assert!(out_path.exists());

        let ahash = AhashFile::read_from_path(&out_path).expect("valid ahash binary");
        assert_eq!(ahash.header.version, 1);
        assert!(ahash.header.frame_count > 0);
        assert!(!ahash.hashes.is_empty());

        let _ = std::fs::remove_file(out_path);
    }

    #[test]
    fn test_run_export_ahash_default_hop_and_output() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let media = manifest_dir.join("../../tests/data/big_buck_bunny_trailer_480p.mp4");
        if !media.exists() {
            eprintln!("Skipping: media fixture not found at {:?}", media);
            return;
        }

        let args = ExportAhashArgs {
            media: media.clone(),
            output: None,
            sample_rate: 5512,
            hop_size: 0,
            start_sec: 0.0,
            duration: Some(0.5),
        };

        let res = run_export_ahash(&args);
        assert!(res.is_ok());

        let fps = 5512.0 / 64.0;
        let default_path = resolve_output_path(&args, fps);
        if default_path.exists() {
            let _ = std::fs::remove_file(default_path);
        }
    }

    #[test]
    fn test_run_export_ahash_non_existent_media_fails() {
        let args = ExportAhashArgs {
            media: PathBuf::from("this_file_does_not_exist_at_all.mp4"),
            output: None,
            sample_rate: 5512,
            hop_size: 64,
            start_sec: 0.0,
            duration: Some(1.0),
        };

        let res = run_export_ahash(&args);
        assert!(res.is_err(), "Non-existent media must return Err");
    }
}
