//! Shared helpers reused across CLI subcommands.

use anyhow::{Context, Result};
use dbsrt_core::{
    extract_frames_phash_with_hwaccel, probe_video, read_phash_file, write_phash_file,
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
    let (meta_fps, max_frames) = if dbsrt_core::container::is_remote_stream_or_pipe(media_file) {
        let fps = media_file
            .file_stem()
            .and_then(|s| s.to_str())
            .map(dbsrt_core::parse_fps_from_stem)
            .unwrap_or(dbsrt_core::DEFAULT_BASELINE_FPS);
        (fps, 0)
    } else if let Ok(meta) = probe_video(media_file) {
        let frames = (meta.duration * meta.fps).floor() as usize;
        (meta.fps, if frames > 0 { frames } else { 0 })
    } else {
        let fps = media_file
            .file_stem()
            .and_then(|s| s.to_str())
            .map(dbsrt_core::parse_fps_from_stem)
            .unwrap_or(dbsrt_core::DEFAULT_BASELINE_FPS);
        (fps, 0)
    };
    let hashes = extract_frames_phash_with_hwaccel(media_file, 0.0, max_frames, hwaccel)?;
    if hashes.is_empty() {
        anyhow::bail!("No video frames could be extracted from {:?}", media_file);
    }

    let file_name = resolve_media_path_filename(media_file)?;
    let out_name = format!("{}.{:.1}fps.phash", file_name, meta_fps);
    let out_path = phash_dir.join(&out_name);
    write_phash_file(&out_path, &hashes)?;

    Ok((out_path, hashes, meta_fps))
}

#[cfg(test)]
mod tests {
    use super::*;

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
