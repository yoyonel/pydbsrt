//! Shared helpers reused across CLI subcommands.

use anyhow::{Context, Result};
use dbsrt_core::{
    ahash_sidecar_path, display_input, extract_frames_advanced, inspect_audio_streams, probe_video,
    read_ahash_header, read_phash_file, write_phash_file, AhashSidecar, AhashSidecarRequest,
    AudioContainerInspection, PhashCheckpoint, VideoExtractionOptions, MOVIEHASH_HEX_LENGTH,
};
use dbsrt_index::{parse_fps_from_phash_stem, DbClient};
use std::path::{Path, PathBuf};

use crate::cli::PostgresArgs;
use crate::constants::{
    AHASH_DOT_EXT, DEFAULT_AHASH_HOP_SIZE, DEFAULT_AHASH_SAMPLE_RATE, DEFAULT_POSTGRES_HOST,
    PHASH_DOT_EXT, PHASH_EXT,
};

pub use dbsrt_core::{ahash_file_name, phash_file_name, PhashSidecar};

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

/// Degenerate all-zero OpenSubtitles 64-bit MovieHash rejected across CLI arguments and inference.
pub(crate) const NULL_MOVIEHASH_HEX: &str = "0000000000000000";

/// Validates `--moviehash`: it becomes a file name, so 16 non-zero hexadecimal characters only.
///
/// # Arguments
/// * `raw` - Raw CLI `--moviehash` value.
///
/// # Returns
/// Lowercase 16-hex string when valid and non-zero.
///
/// # Errors
/// Returns `Err` if `raw` does not contain exactly 16 ASCII hexadecimal characters or equals
/// [`NULL_MOVIEHASH_HEX`].
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Rejects path separators, dots, arbitrary lengths, and the degenerate all-zero hash so the
/// validated hash is always safe and non-colliding as a filesystem stem.
pub(crate) fn parse_moviehash(raw: &str) -> Result<String, String> {
    let lower = raw.to_ascii_lowercase();
    if lower.len() == MOVIEHASH_HEX_LENGTH
        && lower.bytes().all(|b| b.is_ascii_hexdigit())
        && lower != NULL_MOVIEHASH_HEX
    {
        Ok(lower)
    } else {
        Err(format!(
            "expected {MOVIEHASH_HEX_LENGTH} non-zero hexadecimal characters, got {raw:?}"
        ))
    }
}

/// Resolves the OpenSubtitles 64-bit MovieHash for a media target.
///
/// # Arguments
/// * `explicit` - Optional pre-resolved 16-hex MovieHash (`--moviehash`).
/// * `media_file` - Local file path or remote stream URI.
///
/// # Returns
/// `Some(hash)` from `explicit`, else computed on a local file (`>= 64 KiB`); `None` for
/// remote streams without `--moviehash`, files smaller than 64 KiB, or degenerate all-zero hashes.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Normalizes any explicit hash to lowercase, rejects [`NULL_MOVIEHASH_HEX`], and never performs
/// network I/O for remote URIs.
pub(crate) fn resolve_moviehash(explicit: Option<&str>, media_file: &Path) -> Option<String> {
    if let Some(hash) = explicit {
        return parse_moviehash(hash).ok();
    }
    if dbsrt_core::container::is_remote_stream_or_pipe(media_file) {
        return None;
    }
    dbsrt_core::metadata::compute_opensubtitles_hash(media_file)
        .ok()
        .and_then(|h| parse_moviehash(&h).ok())
}

pub(crate) fn resolve_media_path_filename(media_file: &Path) -> Result<String> {
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
    let prefix = full_name.strip_suffix('.').unwrap_or(full_name);
    if prefix.is_empty() {
        return None;
    }
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !name.ends_with(PHASH_DOT_EXT) {
            continue;
        }
        let Some(_) = name.strip_prefix(prefix).and_then(|s| s.strip_prefix('.')) else {
            continue;
        };
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
    let mut entries: Vec<_> = dir_entries.flatten().map(|e| e.path()).collect();
    entries.sort();

    // 1. MovieHash exact prefix lookup is the primary identity: <moviehash>.<fps>fps.phash
    if let Some(res) = moviehash.and_then(|h| match_prefix_phash(&entries, h)) {
        return Ok(Some(res));
    }

    // 2. Strict exact filename match (e.g. Media.mkv.24.0fps.phash)
    let full_name = media_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if !full_name.is_empty() {
        if let Some(res) = match_prefix_phash(&entries, &full_name) {
            return Ok(Some(res));
        }
    }

    // NOTE: match_stem_phash fuzzy fallback was strictly REMOVED to prevent anti-shadowing bugs
    // (e.g. short snippets like "Movie_mid15s.phash" masking full 2-hour master movies).
    Ok(None)
}

/// Returns `true` when `after_prefix` starts with an explicit secondary audio track tag (`a<digits>.`).
///
/// # Arguments
/// * `after_prefix` - Suffix of an `.ahash` file name immediately following `<stem>.`.
///
/// # Returns
/// `true` when the leading dot-delimited token is `a<digits>` (e.g. `a1.86.1fps.ahash`).
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Prevents default-track reuse (`scan_existing_ahash`) from picking up non-default secondary
/// audio tracks (`<stem>.a1.<fps>fps.ahash`).
fn has_secondary_audio_track_tag(after_prefix: &str) -> bool {
    let Some((first_seg, _)) = after_prefix.split_once('.') else {
        return false;
    };
    let Some(digits) = first_seg.strip_prefix(dbsrt_core::AUDIO_TRACK_STEM_TAG) else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn match_prefix_ahash(entries: &[PathBuf], full_name: &str) -> Option<(PathBuf, Vec<u64>, f64)> {
    let prefix = full_name.strip_suffix('.').unwrap_or(full_name);
    if prefix.is_empty() {
        return None;
    }
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !name.ends_with(AHASH_DOT_EXT) {
            continue;
        }
        let Some(after_prefix) = name.strip_prefix(prefix).and_then(|s| s.strip_prefix('.')) else {
            continue;
        };
        if has_secondary_audio_track_tag(after_prefix) {
            continue;
        }
        let Ok(ahash_file) = dbsrt_core::AhashFile::read_from_path(p) else {
            continue;
        };
        if ahash_file.hashes.is_empty() {
            continue;
        }
        let fps = ahash_file.header.audio_fps();
        return Some((p.clone(), ahash_file.hashes, fps));
    }
    None
}

fn scan_existing_ahash(
    ahash_dir: &Path,
    media_file: &Path,
    moviehash: Option<&str>,
) -> Result<Option<(PathBuf, Vec<u64>, f64)>> {
    if !ahash_dir.exists() {
        return Ok(None);
    }
    let dir_entries = match std::fs::read_dir(ahash_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(None),
    };
    let mut entries: Vec<_> = dir_entries.flatten().map(|e| e.path()).collect();
    entries.sort();

    // 1. MovieHash exact prefix lookup: <moviehash>.<fps>fps.ahash
    if let Some(res) = moviehash.and_then(|h| match_prefix_ahash(&entries, h)) {
        return Ok(Some(res));
    }

    // 2. Strict exact filename match (e.g. Media.mkv.86.1fps.ahash)
    let full_name = media_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if !full_name.is_empty() {
        if let Some(res) = match_prefix_ahash(&entries, &full_name) {
            return Ok(Some(res));
        }
    }

    Ok(None)
}

/// Probes the audio stream topology of a media container for the `.ahash` sidecar.
///
/// # Arguments
/// * `media_file` - Local path or remote stream URI (FTP/HTTP).
///
/// # Returns
/// `Some(inspection)` on success; `None` for pipe inputs (probing would consume stdin) or
/// when ffprobe fails (warning printed on stderr, URL userinfo redacted).
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// One ffprobe call (`-select_streams a`, headers only): no audio decode.
pub fn probe_audio_topology(media_file: &Path) -> Option<AudioContainerInspection> {
    if dbsrt_core::container::is_pipe_input(media_file) {
        return None;
    }
    match inspect_audio_streams(media_file) {
        Ok(inspection) => Some(inspection),
        Err(err) => {
            eprintln!(
                "⚠ Audio topology unavailable for {}: {err:#}",
                display_input(media_file)
            );
            None
        }
    }
}

/// Media identity stored in an `.ahash` sidecar.
#[derive(Debug, Clone, Copy)]
pub struct AhashIdentity<'a> {
    /// Canonical media file name.
    pub media_name: &'a str,
    /// OpenSubtitles 16-hex MovieHash when known.
    pub moviehash: Option<&'a str>,
}

/// Creates the `<stem>.json` sidecar of an existing `.ahash` when it is missing.
///
/// # Arguments
/// * `ahash_path` - Existing `.ahash` binary.
/// * `media_file` - Source media probed for its audio topology.
/// * `identity` - Media name and MovieHash to record.
///
/// # Returns
/// `Ok(Some(path))` when a sidecar was written, `Ok(None)` when one already existed.
///
/// # Errors
/// Returns `Err` if the `.ahash` header is invalid or the sidecar cannot be written.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Reads only the 176-byte header (no payload, no FFT), probes the container once
/// (`selection = unknown` if the probe fails) and writes atomically. Never overwrites an
/// existing sidecar.
pub fn ensure_ahash_sidecar(
    ahash_path: &Path,
    media_file: &Path,
    identity: AhashIdentity<'_>,
) -> Result<Option<PathBuf>> {
    if ahash_sidecar_path(ahash_path).exists() {
        return Ok(None);
    }
    let header = read_ahash_header(ahash_path)?;
    let inspection = probe_audio_topology(media_file);
    let sidecar = AhashSidecar::build(&AhashSidecarRequest {
        ahash_path,
        header: &header,
        media_name: identity.media_name,
        moviehash: identity.moviehash,
        inspection: inspection.as_ref(),
        explicit_audio_stream: None,
    });
    sidecar.write_next_to(ahash_path).map(Some)
}

/// Helper resolving an existing .ahash file or extracting a new one into `ahash_dir`.
///
/// Rationale: shared by `media:add` (hence the o2switch batch worker), so every `.ahash`
/// it produces or reuses ends up with its `<stem>.json` audio topology sidecar.
///
/// # Arguments
/// * `media_file` - Path or stream URI of the media file.
/// * `ahash_dir` - Directory where .ahash files are stored.
/// * `moviehash` - Optional 16-hex OpenSubtitles moviehash for canonical naming.
/// * `media_name` - Canonical media name for the sidecar (defaults to the media file name).
///
/// # Returns
/// `(PathBuf, Vec<u64>, f64)` containing output path, audio hashes, and audio fps.
///
/// # Errors
/// Returns `Err` if FFmpeg fails, audio cannot be decoded, or the `.ahash`/sidecar write fails.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Resolves MovieHash automatically when `moviehash` is `None` and `media_file` is local.
/// 2. Reuses an existing `.ahash` (MovieHash prefix, then file name) and creates its
///    sidecar only if missing ([`ensure_ahash_sidecar`]).
/// 3. Otherwise decodes with FFmpeg automatic track selection (`audio_stream: None`, no
///    `-map`, bit-identical to legacy files), writes `<moviehash_16hex>.<fps>fps.ahash`
///    (falling back to media filename only when MovieHash is unavailable), probes the
///    topology, and writes `<stem>.json` with `"moviehash"` populated.
pub fn find_or_build_ahash(
    media_file: &Path,
    ahash_dir: &Path,
    moviehash: Option<&str>,
    media_name: Option<&str>,
) -> Result<(PathBuf, Vec<u64>, f64)> {
    let resolved_hash = resolve_moviehash(moviehash, media_file);
    let effective_hash = resolved_hash.as_deref();
    let fallback_name = resolve_media_path_filename(media_file).ok();
    let identity = AhashIdentity {
        media_name: media_name.or(fallback_name.as_deref()).unwrap_or_default(),
        moviehash: effective_hash,
    };
    if let Some(res) = scan_existing_ahash(ahash_dir, media_file, effective_hash)? {
        eprintln!(
            "Extraction: reused existing .ahash ({} frames), no decode",
            res.1.len()
        );
        if let Some(sidecar) = ensure_ahash_sidecar(&res.0, media_file, identity)? {
            eprintln!("✓ Created missing aHash sidecar {:?}", sidecar);
        }
        return Ok(res);
    }

    std::fs::create_dir_all(ahash_dir)?;
    let options = dbsrt_core::AudioExtractionOptions {
        sample_rate: DEFAULT_AHASH_SAMPLE_RATE,
        hop_size: DEFAULT_AHASH_HOP_SIZE,
        ..Default::default()
    };

    let ahash_file = dbsrt_core::extract_ahash(media_file, &options)?;
    if ahash_file.hashes.is_empty() {
        anyhow::bail!(
            "No audio hashes could be extracted from {:?}",
            dbsrt_core::display_input(media_file)
        );
    }

    let fps = ahash_file.header.audio_fps();
    let file_name = resolve_media_path_filename(media_file)?;
    let out_name = dbsrt_core::ahash_file_name(&file_name, effective_hash, fps);
    let out_path = ahash_dir.join(&out_name);

    dbsrt_core::AhashFile::write_to_path(&out_path, &ahash_file.header, &ahash_file.hashes)?;
    let inspection = probe_audio_topology(media_file);
    AhashSidecar::build(&AhashSidecarRequest {
        ahash_path: &out_path,
        header: &ahash_file.header,
        media_name: identity.media_name,
        moviehash: effective_hash,
        inspection: inspection.as_ref(),
        explicit_audio_stream: None,
    })
    .write_next_to(&out_path)?;

    Ok((out_path, ahash_file.hashes, fps))
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

    const AHASH_FIXTURE: &str = "../../tests/data/big_buck_bunny_trailer_480p.mp4";
    const FIXTURE_MOVIEHASH: &str = "0123456789abcdef";

    #[test]
    fn test_find_or_build_ahash_writes_and_backfills_sidecar() {
        let media = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(AHASH_FIXTURE);
        if !media.exists() {
            eprintln!("Skipping: media fixture not found");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (path, hashes, _) =
            find_or_build_ahash(&media, dir.path(), Some(FIXTURE_MOVIEHASH), Some("BBB.mp4"))
                .unwrap();
        let sidecar_path = dir.path().join(format!("{FIXTURE_MOVIEHASH}.json"));
        assert_eq!(ahash_sidecar_path(&path), sidecar_path);
        let fresh = AhashSidecar::read(&sidecar_path).unwrap();
        assert_eq!(fresh.media_name, "BBB.mp4");
        assert_eq!(fresh.moviehash.as_deref(), Some(FIXTURE_MOVIEHASH));
        assert_eq!(fresh.frames, hashes.len());
        assert_eq!(
            fresh.selection,
            dbsrt_core::AudioTrackSelection::SingleTrack
        );

        // Reuse path (no decode) re-creates a missing sidecar from the header + ffprobe.
        std::fs::remove_file(&sidecar_path).unwrap();
        let (reused, _, _) =
            find_or_build_ahash(&media, dir.path(), Some(FIXTURE_MOVIEHASH), Some("BBB.mp4"))
                .unwrap();
        assert_eq!(reused, path);
        let backfilled = AhashSidecar::read(&sidecar_path).unwrap();
        assert_eq!(backfilled.header, fresh.header);
        assert_eq!(
            backfilled.selected_audio_stream,
            fresh.selected_audio_stream
        );
        assert_eq!(
            ensure_ahash_sidecar(
                &path,
                &media,
                AhashIdentity {
                    media_name: "x",
                    moviehash: None
                }
            )
            .unwrap(),
            None,
            "existing sidecar must never be overwritten"
        );
    }

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
    fn test_find_or_build_ahash_names_local_file_by_moviehash() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/data/big_buck_bunny_trailer_480p.mp4");
        if !fixture.exists() {
            return;
        }
        let dir = scratch_dir("find_or_build_ahash_mh");
        let media = dir.join("Accent é [Release] (2025).mp4");
        std::fs::copy(&fixture, &media).unwrap();
        let expected_hash = dbsrt_core::metadata::compute_opensubtitles_hash(&media).unwrap();
        let ahash_dir = dir.join("ahash");

        let (out_path, hashes, _fps) = find_or_build_ahash(&media, &ahash_dir, None, None).unwrap();
        assert!(!hashes.is_empty());
        let file_name = out_path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            file_name.starts_with(&format!("{expected_hash}.")),
            "expected moviehash prefix {expected_hash}, got {file_name}"
        );
        let sidecar_path = ahash_dir.join(format!("{expected_hash}.json"));
        let sidecar = AhashSidecar::read(&sidecar_path).unwrap();
        assert_eq!(sidecar.moviehash.as_deref(), Some(expected_hash.as_str()));
        assert_eq!(sidecar.media_name, "Accent é [Release] (2025).mp4");
        assert_eq!(sidecar.ahash_file, file_name);
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn test_scan_existing_ahash_sorts_deterministically_and_filters_secondary_tracks_and_prefix_collisions(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_, band_bounds) =
            dbsrt_core::ahash::HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
        let header = dbsrt_core::AhashHeader {
            version: 1,
            sample_rate: 5512,
            frame_size: 2048,
            hop_size: 64,
            num_bands: 33,
            spectral_lag: 2,
            frame_count: 16,
            band_bounds,
        };
        // 1. Secondary track `.a1.` must NOT be picked when default track is requested
        let secondary_path = dir.path().join("Movie.mkv.a1.86.1fps.ahash");
        dbsrt_core::AhashFile::write_to_path(&secondary_path, &header, &[0xAA_u64; 16]).unwrap();

        // 2. Prefix collision without `.` delimiter (`Movie.mkv2.86.1fps.ahash`) must NOT match `Movie.mkv`
        let collision_path = dir.path().join("Movie.mkv2.86.1fps.ahash");
        dbsrt_core::AhashFile::write_to_path(&collision_path, &header, &[0xBB_u64; 16]).unwrap();

        let media = Path::new("/nonexistent/Movie.mkv");
        assert!(
            scan_existing_ahash(dir.path(), media, None)
                .unwrap()
                .is_none(),
            "secondary .a1. track and non-dot prefix collision must be ignored"
        );

        // 3. When multiple default-track files exist, sorted order is deterministic
        let z_path = dir.path().join("Movie.mkv.90.0fps.ahash");
        let a_path = dir.path().join("Movie.mkv.86.1fps.ahash");
        dbsrt_core::AhashFile::write_to_path(&z_path, &header, &[0xCC_u64; 16]).unwrap();
        dbsrt_core::AhashFile::write_to_path(&a_path, &header, &[0xDD_u64; 16]).unwrap();

        let (matched_path, matched_hashes, _) = scan_existing_ahash(dir.path(), media, None)
            .unwrap()
            .expect("must match default-track ahash");
        assert_eq!(matched_path, a_path);
        assert_eq!(matched_hashes[0], 0xDD_u64);

        // 4. Degenerate all-zero moviehash is rejected by parse_moviehash
        assert!(parse_moviehash("0000000000000000").is_err());
        assert_eq!(
            parse_moviehash("0123456789ABCDEF").unwrap(),
            "0123456789abcdef"
        );
    }
}
