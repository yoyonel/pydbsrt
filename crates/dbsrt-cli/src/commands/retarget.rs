//! Subtitle retargeting subcommand aligning reference subtitles with target video release via visual consensus.

use anyhow::{Context, Result};
use clap::Args;
use dbsrt_core::{
    detect_cuts, extract_frames_phash, probe_video, read_phash_file, write_phash_file, SubtitleFile,
};
use dbsrt_index::{parse_fps_from_phash_stem, BkItem, BkTree};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::constants::{
    DEFAULT_BASELINE_FPS, DEFAULT_CUT_THRESHOLD, DEFAULT_EXACT_SEARCH_DISTANCE, DEFAULT_TMP_DIR,
    MILLIS_PER_SEC, PERCENT_FACTOR,
};

#[derive(Args, Debug)]
pub struct RetargetArgs {
    /// Path to reference subtitles file (.srt)
    #[arg(long)]
    pub ref_subtitles: PathBuf,

    /// Path to reference media
    #[arg(long)]
    pub ref_media: PathBuf,

    /// Path to target media
    #[arg(long)]
    pub target_media: PathBuf,

    /// Path to directory containing or storing .phash files
    #[arg(long, default_value = DEFAULT_TMP_DIR)]
    pub phash_dir: PathBuf,

    /// Hamming search distance for BK-Tree
    #[arg(long, default_value_t = DEFAULT_EXACT_SEARCH_DISTANCE)]
    pub search_distance: u32,
}

/// Helper resolving an existing .phash file or extracting a new one into `phash_dir`.
pub fn find_or_build_phash(
    media_file: &Path,
    phash_dir: &Path,
) -> Result<(PathBuf, Vec<u64>, f64)> {
    let full_name = media_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if media_file.extension().and_then(|e| e.to_str()) == Some("phash") && media_file.exists() {
        let hashes = read_phash_file(media_file)?;
        let fps = parse_fps_from_phash_stem(&stem);
        return Ok((media_file.to_path_buf(), hashes, fps));
    }

    if phash_dir.exists() {
        if let Ok(dir_entries) = std::fs::read_dir(phash_dir) {
            let entries: Vec<_> = dir_entries.flatten().map(|e| e.path()).collect();

            // Pass 1: Prefer exact full filename prefix (e.g. `foo.mp4.25.0fps.phash` over `foo.webm.25.0fps.phash`)
            if !full_name.is_empty() {
                for p in &entries {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with(&full_name) && name.ends_with(".phash") {
                            let hashes = read_phash_file(p)?;
                            let meta = probe_video(media_file)?;
                            return Ok((p.clone(), hashes, meta.fps));
                        }
                    }
                }
            }

            // Pass 2: Stem fallback without conflicting container extension
            let known_video_exts = [".mp4.", ".webm.", ".mkv.", ".avi.", ".mov."];
            let my_ext = media_file
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{}.", e.to_lowercase()));
            if !stem.is_empty() {
                for p in &entries {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        if name.contains(&stem) && name.ends_with(".phash") {
                            let name_lower = name.to_lowercase();
                            let has_conflicting_ext = known_video_exts.iter().any(|ext| {
                                name_lower.contains(ext) && my_ext.as_deref() != Some(*ext)
                            });
                            if has_conflicting_ext {
                                continue;
                            }
                            let hashes = read_phash_file(p)?;
                            let meta = probe_video(media_file)?;
                            return Ok((p.clone(), hashes, meta.fps));
                        }
                    }
                }
            }
        }
    }

    // Otherwise probe and extract all frames
    let meta = probe_video(media_file)?;
    let total_frames = (meta.duration * meta.fps).floor() as usize;
    let hashes = extract_frames_phash(media_file, 0.0, total_frames)?;

    let out_name = format!(
        "{}.{:.1}fps.phash",
        media_file.file_name().unwrap().to_string_lossy(),
        meta.fps
    );
    let out_path = phash_dir.join(&out_name);
    write_phash_file(&out_path, &hashes)?;

    Ok((out_path, hashes, meta.fps))
}

/// Run end-to-end subtitle retargeting using scene cuts BK-Tree search.
pub fn run_retarget_subtitles(args: &RetargetArgs) -> Result<()> {
    let (ref_phash_file, ref_hashes, _ref_fps) =
        find_or_build_phash(&args.ref_media, &args.phash_dir).with_context(|| {
            format!("Failed to resolve reference phash for {:?}", args.ref_media)
        })?;

    let (target_phash_file, target_hashes, target_fps) =
        find_or_build_phash(&args.target_media, &args.phash_dir).with_context(|| {
            format!("Failed to resolve target phash for {:?}", args.target_media)
        })?;

    println!("ref_phash_file={:?}", ref_phash_file);
    println!(
        "search_tree_from_target=SearchTreeFromCuts(binary_img_hash_file={:?})",
        target_phash_file
    );

    // 1. Build BKTree for target cuts
    let t_bktree = Instant::now();
    let target_cuts = detect_cuts(&target_hashes, DEFAULT_CUT_THRESHOLD);
    let mut target_tree = BkTree::new();
    for cut in &target_cuts {
        target_tree.insert(BkItem {
            phash: cut.phash,
            offset: cut.offset,
        });
    }
    let bktree_elapsed = t_bktree.elapsed().as_secs_f64();
    println!(
        "Build BKTrees for target => took {:.4} seconds",
        bktree_elapsed
    );

    // 2. Search difference offsets
    let t_search = Instant::now();
    let ref_cuts = detect_cuts(&ref_hashes, DEFAULT_CUT_THRESHOLD);
    let mut diff_counter: HashMap<i64, usize> = HashMap::new();

    for ref_cut in &ref_cuts {
        let matches = target_tree.search(ref_cut.phash, args.search_distance);
        if let Some(first_match) = matches.first() {
            let diff = first_match.offset as i64 - ref_cut.offset as i64;
            *diff_counter.entry(diff).or_default() += 1;
        }
    }
    let search_elapsed = t_search.elapsed().as_secs_f64();

    let total_paired: usize = diff_counter.values().sum();
    let mut sorted_diffs: Vec<(i64, usize)> = diff_counter.into_iter().collect();
    sorted_diffs.sort_by_key(|a| std::cmp::Reverse(a.1));

    if let Some(&(most_common_diff, count)) = sorted_diffs.first() {
        let percent = (count as f64 / total_paired as f64) * PERCENT_FACTOR;
        println!(
            "Nb paired frames used: {} frames ~ {:.16} seconds",
            total_paired,
            total_paired as f64 / DEFAULT_BASELINE_FPS
        );
        println!(
            "Percent of frames used for the most common: ~ {:.2} %",
            percent
        );
        println!(
            "Search differences offsets between reference and target => took {:.4} seconds",
            search_elapsed
        );
        println!(
            "Find differences offsets for synchronize subtitles (reference -> target) => took {:.4} seconds",
            bktree_elapsed + search_elapsed
        );

        let t_shift = Instant::now();
        let shift_frames = most_common_diff;
        let shift_ms = (shift_frames as f64 / target_fps) * MILLIS_PER_SEC;
        println!("target_frame_rate={:.1} fps", target_fps);
        println!(
            "Shift subtitles seconds = {} frames ~ {:.4} seconds",
            shift_frames,
            shift_ms / MILLIS_PER_SEC
        );

        let mut srt = SubtitleFile::open(&args.ref_subtitles)?;
        srt.shift(shift_ms);

        let out_name = target_phash_file
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.replace(".phash", ".srt"))
            .unwrap_or_else(|| "out.srt".to_string());
        let out_path = args.phash_dir.join(&out_name);
        srt.save(&out_path)?;

        println!("Exported subtitles for target in : \n'{:?}'", out_path);
        println!(
            "Shift subtitles (from reference) and save it (for target) => took {:.4} seconds",
            t_shift.elapsed().as_secs_f64()
        );
    } else {
        println!("Error: Can't find difference offsets !");
    }

    Ok(())
}
