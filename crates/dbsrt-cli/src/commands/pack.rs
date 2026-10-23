//! Packing subcommand compiling loose .phash files into unified .dbphash v2.0 index.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{detect_cuts, read_phash_file, MediaMetadata};
use dbsrt_index::DbphashBuilder;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::constants::{
    BYTES_PER_MIB, DEFAULT_BASELINE_FPS, DEFAULT_CUT_THRESHOLD, DEFAULT_PHASH_DATA_DIR,
    MAX_VALID_FPS, MILLIS_PER_SEC, MIN_VALID_FPS, PACK_PROGRESS_INTERVAL, PERCENT_FACTOR,
    PHASH_EXT,
};

#[derive(Args, Debug)]
pub struct PackArgs {
    /// Directory containing .phash files
    #[arg(short, long, default_value = DEFAULT_PHASH_DATA_DIR)]
    pub phash_dir: PathBuf,

    /// Extra directory containing .phash files
    #[arg(long = "extra-dir")]
    pub extra_dirs: Vec<PathBuf>,

    /// Output .dbphash compiled binary index file
    #[arg(short, long, default_value = "catalog.dbphash")]
    pub output: PathBuf,

    /// Scene cut Hamming threshold
    #[arg(short = 't', long, default_value_t = DEFAULT_CUT_THRESHOLD)]
    pub threshold: u32,

    /// Default framerate if not detectable from filename
    #[arg(long, default_value_t = DEFAULT_BASELINE_FPS)]
    pub default_fps: f64,
}

/// Helper parsing framerate from dot-separated filename stem (e.g. `foo.25fps.phash` or `foo.25.0fps.phash`).
///
/// Unlike `dbsrt_index::parse_fps_from_phash_stem`, it deliberately skips release-tag heuristics
/// (PAL/NTSC fallbacks) and returns `None` instead, so `--default-fps` keeps priority.
pub fn extract_fps_from_path(path: &Path) -> Option<f64> {
    let s = path.file_name()?.to_str()?;
    let stem = path.file_stem()?.to_str()?;
    if let Some(fps_idx) = stem.rfind("fps") {
        let before_fps = &stem[..fps_idx];
        if let Some(dot_idx) = before_fps.rfind('.') {
            let candidate = if let Some(prev_dot) = before_fps[..dot_idx].rfind('.') {
                if before_fps[prev_dot + 1..dot_idx]
                    .chars()
                    .all(|c| c.is_ascii_digit())
                {
                    &before_fps[prev_dot + 1..]
                } else {
                    &before_fps[dot_idx + 1..]
                }
            } else {
                &before_fps[dot_idx + 1..]
            };
            if let Ok(fps) = candidate.parse::<f64>() {
                if fps > MIN_VALID_FPS && fps < MAX_VALID_FPS {
                    return Some(fps);
                }
            }
        }
    }
    for part in s.split('.') {
        if let Some(fps_str) = part.strip_suffix("fps") {
            if let Ok(fps) = fps_str.parse::<f64>() {
                if fps > MIN_VALID_FPS && fps < MAX_VALID_FPS {
                    return Some(fps);
                }
            }
        }
    }
    None
}

/// Run compilation of .phash directories into a single unified .dbphash binary index.
pub fn run_pack(args: &PackArgs) -> Result<()> {
    let mut search_dirs = vec![args.phash_dir.clone()];
    search_dirs.extend(args.extra_dirs.clone());

    let mut files = Vec::new();
    let mut seen_stems = std::collections::HashSet::new();

    for dir in &search_dirs {
        if !dir.exists() {
            eprintln!("Warning: Scanned directory {:?} does not exist", dir);
            continue;
        }
        let mut found_in_dir = 0;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some(PHASH_EXT) {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if seen_stems.insert(stem.to_string()) {
                        files.push(path);
                    }
                    found_in_dir += 1;
                }
            }
        }
        if found_in_dir == 0 {
            eprintln!(
                "Warning: Scanned directory {:?} contains no .{} files",
                dir, PHASH_EXT
            );
        }
    }
    files.sort();

    if files.is_empty() {
        anyhow::bail!("No .{} files found in {:?}", PHASH_EXT, search_dirs);
    }

    println!("=== DBS-RT COMPILER: UNIFIED .DBPHASH PACKER ===");
    println!("Input directories: {:?}", search_dirs);
    println!("Output index     : {:?}", args.output);
    println!("Found .phash     : {} unique files", files.len());
    println!("Cut threshold    : {}", args.threshold);

    let start = Instant::now();
    let mut builder = DbphashBuilder::new();
    let mut total_raw_bytes = 0u64;

    for (idx, file_path) in files.iter().enumerate() {
        let meta = MediaMetadata::parse_from_path(file_path);
        let hashes = read_phash_file(file_path)?;
        let file_len = std::fs::metadata(file_path)?.len();
        total_raw_bytes += file_len;

        let fps = extract_fps_from_path(file_path).unwrap_or(args.default_fps);
        let duration_ms = (hashes.len() as f64 / fps * MILLIS_PER_SEC).round() as u64;

        let detected = detect_cuts(&hashes, args.threshold);
        let cuts: Vec<(u64, u32)> = detected
            .into_iter()
            .map(|c| {
                (
                    c.phash,
                    (c.offset as f64 / fps * MILLIS_PER_SEC).round() as u32,
                )
            })
            .collect();

        let fps_step = (fps.round() as usize).max(1);
        let sparse: Vec<(u64, u32)> = (0..hashes.len())
            .step_by(fps_step)
            .map(|idx| {
                (
                    hashes[idx],
                    (idx as f64 / fps * MILLIS_PER_SEC).round() as u32,
                )
            })
            .collect();

        let cuts_cnt = cuts.len();
        let sparse_cnt = sparse.len();
        builder.add_media(&meta, duration_ms, fps, &cuts, &sparse);

        if (idx + 1) % PACK_PROGRESS_INTERVAL == 0 || idx + 1 == files.len() {
            println!(
                "  [{:>3}/{:>3}] Packed {:<40} ({} cuts, {} 1fps)",
                idx + 1,
                files.len(),
                file_path.file_name().unwrap_or_default().to_string_lossy(),
                cuts_cnt,
                sparse_cnt
            );
        }
    }

    let stats = builder.write_to_file(&args.output)?;
    let elapsed = start.elapsed().as_secs_f64();
    let ratio = if stats.output_bytes > 0 {
        total_raw_bytes as f64 / stats.output_bytes as f64
    } else {
        1.0
    };

    println!("\n=== PACKAGING SUCCESSFUL (HYBRID v2.0) ===");
    println!("Media items packed    : {:>10}", stats.media_count);
    println!("Total scene cuts      : {:>10}", stats.total_cuts_count);
    println!("Total 1 fps frames    : {:>10}", stats.total_sparse_count);
    println!(
        "Raw .phash total size : {:>10.2} MB",
        total_raw_bytes as f64 / BYTES_PER_MIB
    );
    println!(
        "Compiled .dbphash size: {:>10.2} MB",
        stats.output_bytes as f64 / BYTES_PER_MIB
    );
    println!(
        "Space reduction       : {:>10.1}x ({:.1}% smaller)",
        ratio,
        (1.0 - 1.0 / ratio) * PERCENT_FACTOR
    );
    println!("Total elapsed time    : {:>10.3} s", elapsed);
    println!("Output index path     : {:?}", args.output);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_fps_from_path() {
        assert_eq!(
            extract_fps_from_path(Path::new("video.25.0fps.phash")),
            Some(25.0)
        );
        assert_eq!(
            extract_fps_from_path(Path::new("video.23.976fps.phash")),
            Some(23.976)
        );
        assert_eq!(extract_fps_from_path(Path::new("video.phash")), None);
        assert_eq!(
            extract_fps_from_path(Path::new("video.invalidfps.phash")),
            None
        );
        assert_eq!(extract_fps_from_path(Path::new("video.0.0fps.phash")), None);
        assert_eq!(
            extract_fps_from_path(Path::new("video.500.0fps.phash")),
            None
        );
    }

    #[test]
    fn test_pack_empty_directory_error() {
        let temp_dir = std::env::temp_dir().join(format!("test_pack_empty_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let out_file = temp_dir.join("empty.dbphash");

        let args = PackArgs {
            phash_dir: temp_dir.clone(),
            extra_dirs: vec![],
            output: out_file,
            threshold: DEFAULT_CUT_THRESHOLD,
            default_fps: DEFAULT_BASELINE_FPS,
        };

        let err = run_pack(&args).unwrap_err();
        assert!(err.to_string().contains("No .phash files found"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
