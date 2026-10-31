//! Shared helpers reused across CLI subcommands.

use anyhow::{Context, Result};
use dbsrt_core::{
    extract_frames_advanced, probe_video, read_phash_file, write_phash_file, PhashCheckpoint,
    VideoExtractionOptions,
};
use dbsrt_index::{parse_fps_from_phash_stem, DbClient};
use std::path::{Path, PathBuf};

use crate::cli::PostgresArgs;
use crate::constants::{DEFAULT_POSTGRES_HOST, KNOWN_VIDEO_EXTS, PHASH_DOT_EXT, PHASH_EXT};

/// Resolve target PostgreSQL host falling back to PSQL_IP env or DEFAULT_POSTGRES_HOST.
pub fn resolve_db_host(db: &PostgresArgs) -> String {
    db.host
        .clone()
        .or_else(|| std::env::var("PSQL_IP").ok())
        .unwrap_or_else(|| DEFAULT_POSTGRES_HOST.to_string())
}

/// Connect to PostgreSQL using configuration from `PostgresArgs`.
pub fn connect_db(db: &PostgresArgs) -> Result<DbClient> {
    let host = resolve_db_host(db);
    DbClient::connect(&host, db.port, &db.user, &db.pass, &db.name).with_context(|| {
        format!(
            "Failed to connect to PostgreSQL at {}:{}/{}",
            host, db.port, db.name
        )
    })
}

/// Helper resolving an existing .phash file or extracting a new one into `phash_dir`.
///
/// # Concurrency
/// This function is CPU-bound (FFmpeg decoding + pHash SIMD). When called
/// from a Tokio runtime (e.g. in `dbsrt-server`), wrap in `tokio::task::spawn_blocking`
/// to avoid starving async executors.
pub fn find_or_build_phash(
    media_file: &Path,
    phash_dir: &Path,
) -> Result<(PathBuf, Vec<u64>, f64)> {
    find_or_build_phash_with_hwaccel(media_file, phash_dir, dbsrt_core::HwAccel::Auto)
}

fn resolve_media_path_filename(media_file: &Path) -> Result<String> {
    if dbsrt_core::container::is_remote_stream_or_pipe(media_file) {
        let raw = media_file.to_string_lossy();
        let unquoted = raw.split('?').next().unwrap_or(&raw);
        let last_segment = unquoted.rsplit('/').next().unwrap_or(unquoted);
        return Ok(last_segment.to_string());
    }
    media_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| anyhow::anyhow!("Cannot derive filename from media path {:?}", media_file))
}

fn match_prefix_phash(entries: &[PathBuf], full_name: &str) -> Option<(PathBuf, Vec<u64>, f64)> {
    if full_name.is_empty() {
        return None;
    }
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !name.starts_with(full_name) || !name.ends_with(PHASH_DOT_EXT) {
            continue;
        }
        let Ok(hashes) = read_phash_file(p) else {
            continue;
        };
        if hashes.is_empty() {
            continue;
        }
        let fps = parse_fps_from_phash_stem(name);
        return Some((p.clone(), hashes, fps));
    }
    None
}

fn match_stem_phash(
    entries: &[PathBuf],
    stem: &str,
    my_ext: Option<&str>,
) -> Option<(PathBuf, Vec<u64>, f64)> {
    if stem.is_empty() {
        return None;
    }
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !name.contains(stem) || !name.ends_with(PHASH_DOT_EXT) {
            continue;
        }
        let name_lower = name.to_lowercase();
        let has_conflicting = KNOWN_VIDEO_EXTS
            .iter()
            .any(|ext| name_lower.contains(ext) && my_ext != Some(*ext));
        if has_conflicting {
            continue;
        }
        let Ok(hashes) = read_phash_file(p) else {
            continue;
        };
        if hashes.is_empty() {
            continue;
        }
        let fps = parse_fps_from_phash_stem(name);
        return Some((p.clone(), hashes, fps));
    }
    None
}

fn scan_existing_phash(
    phash_dir: &Path,
    full_name: &str,
    stem: &str,
    media_file: &Path,
) -> Result<Option<(PathBuf, Vec<u64>, f64)>> {
    if !phash_dir.exists() {
        return Ok(None);
    }
    let dir_entries = match std::fs::read_dir(phash_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(None),
    };
    let entries: Vec<_> = dir_entries.flatten().map(|e| e.path()).collect();

    if let Some(res) = match_prefix_phash(&entries, full_name) {
        return Ok(Some(res));
    }

    let my_ext = media_file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}.", e.to_lowercase()));

    if let Some(res) = match_stem_phash(&entries, stem, my_ext.as_deref()) {
        return Ok(Some(res));
    }

    Ok(None)
}

/// Discovers existing .phash file or extracts full perceptual hash fingerprint with hardware acceleration.
pub fn find_or_build_phash_with_hwaccel(
    media_file: &Path,
    phash_dir: &Path,
    hwaccel: dbsrt_core::HwAccel,
) -> Result<(PathBuf, Vec<u64>, f64)> {
    let full_name = media_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if media_file.extension().and_then(|e| e.to_str()) == Some(PHASH_EXT) && media_file.exists() {
        let hashes = read_phash_file(media_file)?;
        let fps = parse_fps_from_phash_stem(&stem);
        return Ok((media_file.to_path_buf(), hashes, fps));
    }

    if let Some(res) = scan_existing_phash(phash_dir, &full_name, &stem, media_file)? {
        return Ok(res);
    }

    // Otherwise probe and extract all frames
    let is_remote = dbsrt_core::container::is_remote_stream_or_pipe(media_file);
    let probed = (!dbsrt_core::container::is_pipe_input(media_file))
        .then(|| probe_video(media_file).ok())
        .flatten()
        .filter(|meta| meta.fps > 0.0);
    let meta_fps = probed.as_ref().map_or_else(
        || {
            media_file
                .file_stem()
                .and_then(|s| s.to_str())
                .map(dbsrt_core::parse_fps_from_stem)
                .unwrap_or(dbsrt_core::DEFAULT_BASELINE_FPS)
        },
        |meta| meta.fps,
    );
    // Remote streams stay uncapped: their probed fps only names the .phash file.
    let max_frames = match &probed {
        Some(meta) if !is_remote => (meta.duration * meta.fps).floor() as usize,
        _ => 0,
    };
    let file_name = resolve_media_path_filename(media_file)?;
    let out_name = format!("{}.{:.1}fps.phash", file_name, meta_fps);
    let out_path = phash_dir.join(&out_name);
    let base = VideoExtractionOptions {
        max_frames: (max_frames > 0).then_some(max_frames),
        hwaccel,
        ..Default::default()
    };
    let hashes = extract_resumable(media_file, &out_path, meta_fps, base)?;
    if hashes.is_empty() {
        anyhow::bail!("No video frames could be extracted from {:?}", media_file);
    }

    Ok((out_path, hashes, meta_fps))
}

/// Input seek lead before a resume point: covers the keyframe distance, since an MPEG-TS input
/// seek lands on the keyframe after its target.
const RESUME_PREROLL_SEC: f64 = 10.0;

/// `base` full-extraction options that continue after `done` frames, frame for frame. The input
/// seek lands on whole seconds well before frame `done`; the output seek then drops exactly the
/// frames before it. Seeking straight to frame `done` (or half a frame early) shifts the CFR output.
fn resume_options(done: usize, fps: f64, base: VideoExtractionOptions) -> VideoExtractionOptions {
    let target = done as f64 / fps;
    let start_sec = (target - RESUME_PREROLL_SEC).max(0.0).floor();
    VideoExtractionOptions {
        start_sec,
        output_seek_sec: target - start_sec,
        ..base
    }
}

/// Extracts a full fingerprint into `out_path`. A remote stream checkpoints into
/// `<out_path>.partial` and resumes from it after a kill (watchdog timeout, worker reload).
/// The final file is renamed into place, so a reader never sees it half written.
// shortcut: resume seeks by frame count / fps, exact only for a constant framerate and an
// unchanged source; delete the .partial to force a full extraction of a VFR or replaced file.
fn extract_resumable(
    media_file: &Path,
    out_path: &Path,
    meta_fps: f64,
    base: VideoExtractionOptions,
) -> Result<Vec<u64>> {
    let resumable = dbsrt_core::container::is_remote_stream_or_pipe(media_file)
        && !dbsrt_core::container::is_pipe_input(media_file);
    let partial = out_path.with_extension("phash.partial");
    let mut hashes = if resumable {
        read_phash_file(&partial).unwrap_or_default()
    } else {
        Vec::new()
    };
    let checkpoint = resumable.then(|| PhashCheckpoint {
        path: partial.clone(),
        done: hashes.len(),
    });
    let opts = resume_options(
        hashes.len(),
        meta_fps,
        VideoExtractionOptions { checkpoint, ..base },
    );
    hashes.extend(
        extract_frames_advanced(media_file, &opts)?
            .into_iter()
            .map(|s| s.phash),
    );
    if !hashes.is_empty() {
        let tmp = out_path.with_extension("phash.tmp");
        write_phash_file(&tmp, &hashes)?;
        std::fs::rename(&tmp, out_path)?;
        let _ = std::fs::remove_file(&partial);
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu() -> VideoExtractionOptions {
        VideoExtractionOptions {
            hwaccel: dbsrt_core::HwAccel::Disabled,
            ..Default::default()
        }
    }

    #[test]
    fn test_resume_options_coarse_input_seek_then_exact_output_seek() {
        let opts = resume_options(1000, 25.0, cpu());
        assert_eq!((opts.start_sec, opts.output_seek_sec), (30.0, 10.0));
        assert_eq!(opts.hwaccel, dbsrt_core::HwAccel::Disabled);
        let opts = resume_options(100, 25.0, cpu());
        assert_eq!((opts.start_sec, opts.output_seek_sec), (0.0, 4.0));
        let opts = resume_options(0, 25.0, cpu());
        assert_eq!((opts.start_sec, opts.output_seek_sec), (0.0, 0.0));
    }

    fn assert_resume_matches_full(video: &Path) {
        let extract = |opts: &VideoExtractionOptions| -> Vec<u64> {
            extract_frames_advanced(video, opts)
                .unwrap()
                .into_iter()
                .map(|s| s.phash)
                .collect()
        };
        let full = extract(&cpu());
        let fps = probe_video(video).unwrap().fps;
        // Keyframe, mid-GOP and late resume points.
        for done in [1, 137, 500, full.len() - 3] {
            let resumed = extract(&resume_options(done, fps, cpu()));
            assert_eq!(resumed, full[done..], "resume after {done} frames");
        }
    }

    #[test]
    fn test_resume_seek_continues_the_full_extraction_exactly() {
        assert_resume_matches_full(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/data/big_buck_bunny_trailer_480p.mp4"),
        );
    }

    #[test]
    fn test_resume_seek_is_exact_on_offset_mpegts() {
        // MPEG-TS: a bare input seek lands on the next keyframe and drops frames.
        let video =
            std::env::temp_dir().join(format!("test_resume_offset_{}.ts", std::process::id()));
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x120:rate=25",
            ])
            .args([
                "-t",
                "40",
                "-g",
                "50",
                "-bf",
                "2",
                "-output_ts_offset",
                "1.4",
            ])
            .args(["-f", "mpegts"])
            .arg(&video)
            .status()
            .unwrap();
        assert!(status.success());
        assert_resume_matches_full(&video);
        let _ = std::fs::remove_file(&video);
    }

    #[test]
    fn test_resolve_db_host_explicit() {
        let args = PostgresArgs {
            host: Some("rpi4".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_db_host(&args), "rpi4");
    }

    #[test]
    fn test_resolve_db_host_fallback() {
        let args = PostgresArgs {
            host: None,
            ..Default::default()
        };
        // If PSQL_IP is set, returns that; otherwise DEFAULT_POSTGRES_HOST
        let expected =
            std::env::var("PSQL_IP").unwrap_or_else(|_| DEFAULT_POSTGRES_HOST.to_string());
        assert_eq!(resolve_db_host(&args), expected);
    }
}
