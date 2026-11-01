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

pub use dbsrt_core::{phash_file_name, PhashSidecar};

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
    find_or_build_phash_with_hwaccel(media_file, phash_dir, dbsrt_core::HwAccel::Auto, None)
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
        // The parser reads the fps at the end of the stem: the .phash suffix would hide it
        let fps = parse_fps_from_phash_stem(name.trim_end_matches(PHASH_DOT_EXT));
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
        let fps = parse_fps_from_phash_stem(name.trim_end_matches(PHASH_DOT_EXT));
        return Some((p.clone(), hashes, fps));
    }
    None
}

fn scan_existing_phash(
    phash_dir: &Path,
    media_file: &Path,
    moviehash: Option<&str>,
) -> Result<Option<(PathBuf, Vec<u64>, f64)>> {
    if !phash_dir.exists() {
        return Ok(None);
    }
    let dir_entries = match std::fs::read_dir(phash_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(None),
    };
    let entries: Vec<_> = dir_entries.flatten().map(|e| e.path()).collect();

    // The moviehash names the content whatever the media name; files named before it fall through
    if let Some(res) = moviehash.and_then(|h| match_prefix_phash(&entries, &format!("{h}."))) {
        return Ok(Some(res));
    }
    let name_of = |s: Option<&std::ffi::OsStr>| s.map(|s| s.to_string_lossy().to_string());
    let full_name = name_of(media_file.file_name()).unwrap_or_default();
    let stem = name_of(media_file.file_stem()).unwrap_or_default();
    if let Some(res) = match_prefix_phash(&entries, &full_name) {
        return Ok(Some(res));
    }

    let my_ext = media_file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}.", e.to_lowercase()));

    if let Some(res) = match_stem_phash(&entries, &stem, my_ext.as_deref()) {
        return Ok(Some(res));
    }

    Ok(None)
}

/// Discovers existing .phash file or extracts full perceptual hash fingerprint with hardware acceleration.
pub fn find_or_build_phash_with_hwaccel(
    media_file: &Path,
    phash_dir: &Path,
    hwaccel: dbsrt_core::HwAccel,
    moviehash: Option<&str>,
) -> Result<(PathBuf, Vec<u64>, f64)> {
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if media_file.extension().and_then(|e| e.to_str()) == Some(PHASH_EXT) && media_file.exists() {
        let hashes = read_phash_file(media_file)?;
        let fps = parse_fps_from_phash_stem(&stem);
        return Ok((media_file.to_path_buf(), hashes, fps));
    }

    if let Some(res) = scan_existing_phash(phash_dir, media_file, moviehash)? {
        // The batch summary and the worker rate read this note: no decode, no NAS read rate
        eprintln!(
            "Extraction: reused existing .phash ({} frames), no decode",
            res.1.len()
        );
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
    let out_name = phash_file_name(&file_name, moviehash, meta_fps);
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
    let tmp = out_path.with_extension("phash.tmp");
    ensure_creatable(&tmp)?;
    let mut hashes = if resumable {
        ensure_creatable(&partial)?;
        let (hashes, note) = load_checkpoint(&partial);
        eprintln!("{note}");
        hashes
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
        write_phash_file(&tmp, &hashes)?;
        std::fs::rename(&tmp, out_path)?;
        let _ = std::fs::remove_file(&partial);
    }
    Ok(hashes)
}

/// Fails before the decode when an output file cannot be created (name over NAME_MAX, missing or
/// read-only dir), instead of after minutes of extraction. An existing file is left untouched.
fn ensure_creatable(path: &Path) -> Result<()> {
    let existed = path.exists();
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Cannot create output file {:?}", path))?;
    if !existed {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// Loads the `.phash.partial` checkpoint of a remote extraction, with a note for the worker log
/// stating what the run starts from, so a resumed run is provable after the fact.
fn load_checkpoint(partial: &Path) -> (Vec<u64>, String) {
    if !partial.exists() {
        return (
            Vec::new(),
            "Extraction: no checkpoint, full decode".to_string(),
        );
    }
    match read_phash_file(partial) {
        Ok(hashes) => {
            let note = format!(
                "Extraction: resumed {} frames from checkpoint",
                hashes.len()
            );
            (hashes, note)
        }
        Err(e) => (
            Vec::new(),
            format!("Extraction: checkpoint unreadable ({e:#}), full decode"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dbsrt_shared_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_find_or_build_reuses_the_phash_named_by_moviehash() {
        let dir = scratch_dir("reuse");
        write_phash_file(&dir.join("e19d5212c9812cd6.25.0fps.phash"), &[1, 2, 3]).unwrap();
        // Media name unrelated to the file name: only the moviehash links them, no decode happens
        let media = Path::new("/nonexistent/Renamed Media.mkv");
        let (path, hashes, fps) = find_or_build_phash_with_hwaccel(
            media,
            &dir,
            dbsrt_core::HwAccel::Disabled,
            Some("e19d5212c9812cd6"),
        )
        .unwrap();
        assert_eq!(path, dir.join("e19d5212c9812cd6.25.0fps.phash"));
        assert_eq!(hashes, vec![1, 2, 3]);
        assert_eq!(fps, 25.0);
        // Another moviehash never picks this file
        let other = find_or_build_phash_with_hwaccel(
            media,
            &dir,
            dbsrt_core::HwAccel::Disabled,
            Some("0000000000000001"),
        );
        assert!(other.is_err());
    }

    #[test]
    fn test_find_or_build_reuse_by_media_name_reads_the_fps_of_the_file() {
        let dir = scratch_dir("reuse_name");
        write_phash_file(&dir.join("Old Name.mkv.23.976fps.phash"), &[7]).unwrap();
        let media = Path::new("/nonexistent/Old Name.mkv");
        let (_, hashes, fps) =
            find_or_build_phash_with_hwaccel(media, &dir, dbsrt_core::HwAccel::Disabled, None)
                .unwrap();
        assert_eq!(hashes, vec![7]);
        assert_eq!(fps, 23.976);
    }

    #[test]
    fn test_phash_file_name_uses_moviehash_whatever_the_media_name() {
        let long = format!("{}.mkv", "x".repeat(300));
        assert_eq!(
            phash_file_name(&long, Some("e19d5212c9812cd6"), 23.976),
            "e19d5212c9812cd6.24.0fps.phash"
        );
        assert_eq!(phash_file_name("a.mkv", None, 25.0), "a.mkv.25.0fps.phash");
    }

    #[test]
    fn test_write_phash_sidecar_keeps_the_media_name() {
        let dir = scratch_dir("sidecar");
        // 300 bytes: longer than NAME_MAX, only the sidecar content carries it
        let long = format!("{}.mkv", "\u{e9}".repeat(148));
        let sidecar = PhashSidecar {
            moviehash: "e19d5212c9812cd6",
            media_name: &long,
            fps: 25.0,
            frames: 1200,
        };
        let path = sidecar.write(&dir).unwrap();
        assert_eq!(path, dir.join("e19d5212c9812cd6.json"));
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["moviehash"], "e19d5212c9812cd6");
        assert_eq!(v["media_name"], long);
        assert_eq!(v["fps"], 25.0);
        assert_eq!(v["frames"], 1200);
        assert!(v["extracted_at"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_extract_resumable_fails_before_decode_when_output_name_too_long() {
        let dir = scratch_dir("long_name");
        // 252 bytes fits NAME_MAX (255); the `.phash.tmp` sibling (256) does not.
        let out_path = dir.join(format!("{}.phash", "x".repeat(246)));
        let missing_media = dir.join("missing.mkv");

        let err = extract_resumable(&missing_media, &out_path, 25.0, cpu()).unwrap_err();

        // The decode never starts: the error names the output, not the missing input.
        assert!(
            format!("{err:#}").contains("Cannot create output file"),
            "{err:#}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_creatable_keeps_existing_and_leaves_no_new_file() {
        let dir = scratch_dir("creatable");
        let existing = dir.join("a.phash.partial");
        std::fs::write(&existing, [1u8; 16]).unwrap();
        let fresh = dir.join("b.phash.tmp");

        ensure_creatable(&existing).unwrap();
        ensure_creatable(&fresh).unwrap();

        assert_eq!(std::fs::read(&existing).unwrap(), [1u8; 16]);
        assert!(!fresh.exists());
        assert!(ensure_creatable(&dir.join("no_such_dir").join("c.phash.tmp")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_checkpoint_states_what_the_run_starts_from() {
        let dir = scratch_dir("checkpoint");
        let partial = dir.join("m.phash.partial");

        let (hashes, note) = load_checkpoint(&partial);
        assert!(hashes.is_empty());
        assert!(note.contains("no checkpoint"), "{note}");

        write_phash_file(&partial, &[7, 8, 9]).unwrap();
        let (hashes, note) = load_checkpoint(&partial);
        assert_eq!(hashes, vec![7, 8, 9]);
        assert!(note.contains("resumed 3 frames from checkpoint"), "{note}");

        let unreadable = dir.join("dir.phash.partial");
        std::fs::create_dir_all(&unreadable).unwrap();
        let (hashes, note) = load_checkpoint(&unreadable);
        assert!(hashes.is_empty());
        assert!(note.contains("unreadable"), "{note}");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
