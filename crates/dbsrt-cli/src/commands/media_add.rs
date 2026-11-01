//! Unified CLI facade subcommand orchestrating pHash extraction, coupling certification, and catalog ingestion.

use anyhow::{anyhow, bail, Result};
use clap::Args;
use dbsrt_core::{
    build_and_validate_sub_frames_opts, compute_bytes_digest_32, compute_media_phash_digest,
    detect_media_fps, extract_embedded_subtitle, find_subtitle_pairs, inspect_container,
    language_matches, parse_language_code, CertifiedSubtitleRecord, CouplingRegistry, LangStatus,
    SubtitleFile,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::cli::PostgresArgs;
use crate::commands::certify::detect_cli_subtitle_lang_status;
use crate::commands::shared::{find_or_build_phash_with_hwaccel, phash_file_name, PhashSidecar};
use crate::constants::{
    DEFAULT_HTTP_TIMEOUT_SECS, DEFAULT_SERVER_URL, DEFAULT_SUBTITLES_DIR, DEFAULT_TMP_DIR,
};

/// Maximum file size in bytes considered a partial/corrupted pHash file on interrupted write.
pub const PARTIAL_PHASH_THRESHOLD_BYTES: u64 = 1_000;

/// Standard UNIX process exit code on SIGINT signal (128 + 2).
pub const SIGINT_EXIT_CODE: i32 = 130;

/// Codecs considered bitmap subtitles that cannot be converted directly to SRT text.
pub const BITMAP_SUB_CODECS: &[&str] = &[
    "hdmv_pgs_subtitle",
    "dvd_subtitle",
    "pgssub",
    "vobsub",
    "dvdsub",
];

pub(crate) enum SubtitleJob {
    AlreadyResolved(Vec<(PathBuf, LangStatus)>),
    NeedsExtraction {
        candidates: Vec<(usize, Option<String>, PathBuf)>,
        discovered: Vec<(PathBuf, Option<String>)>,
    },
}

fn select_and_validate_subtitle_pairs(
    discovered: &[(PathBuf, Option<String>)],
    video_path: &Path,
    target_lang: Option<&str>,
    is_json: bool,
) -> Result<Vec<(PathBuf, LangStatus)>> {
    let selected: Vec<_> = if let Some(target) = target_lang {
        let matching: Vec<_> = discovered
            .iter()
            .filter(|(_, lang)| lang.as_deref() == Some(target))
            .cloned()
            .collect();
        if !matching.is_empty() {
            matching
        } else {
            let untagged: Vec<_> = discovered
                .iter()
                .filter(|(_, lang)| lang.is_none())
                .cloned()
                .collect();
            if untagged.is_empty() {
                bail!(
                    "❌ Discovered subtitle files did not match requested language '{}'",
                    target
                );
            }
            let in_parent: Vec<_> = untagged
                .iter()
                .filter(|(p, _)| p.parent() == video_path.parent())
                .cloned()
                .collect();
            if !in_parent.is_empty() {
                in_parent
            } else {
                untagged
            }
        }
    } else {
        discovered.to_vec()
    };

    if !is_json {
        if let Some(target) = target_lang {
            eprintln!(
                "🔍 Filtering subtitle candidates for requested language '{}':",
                target
            );
            for (p, _) in &selected {
                eprintln!("   ✓ Selected: {:?}", p.file_name().unwrap_or_default());
            }
            for (p, l) in discovered {
                if !selected.iter().any(|(sp, _)| sp == p) {
                    let reason = match l {
                        Some(other) => format!("detected language '{}' != '{}'", other, target),
                        None => "secondary track bypassed in favor of root sidecar".to_string(),
                    };
                    eprintln!(
                        "   ⊘ Ignored: {:?} ({})",
                        p.file_name().unwrap_or_default(),
                        reason
                    );
                }
            }
        }
    }

    let mut final_pairs = Vec::new();
    for (path, lang_opt) in selected {
        let srt_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default();
        let lang_status = if let Some(target) = target_lang {
            LangStatus::Confirmed(target.to_lowercase())
        } else if let Some(ref l) = lang_opt {
            LangStatus::Detected(l.clone())
        } else {
            let status = detect_cli_subtitle_lang_status(srt_name, "auto");
            if !is_json {
                eprintln!(
                    "   Lang inferred from filename via detect_cli_subtitle_lang: {}",
                    status.to_lang_code()
                );
            }
            match status {
                LangStatus::Detected(code) => LangStatus::Detected(code),
                _ => {
                    if !is_json {
                        eprintln!("⚠️  Language could not be determined from filename.");
                        eprintln!(
                            "   Coupling will be certified with lang=\"und\" (undetermined)."
                        );
                        eprintln!("   To fix later: dbsrt coupling:relabel --media <hash> --sub <hash> --lang <code>");
                        eprintln!("   Or specify --lang <code> now to set it explicitly.");
                    }
                    LangStatus::Unknown
                }
            }
        };
        let lang_code = lang_status.to_lang_code();
        if lang_code == "auto" || lang_code.is_empty() {
            bail!(
                "❌ Could not determine subtitle language for '{}'. Please specify it explicitly via --lang <fr|en|es|...>",
                srt_name
            );
        }
        final_pairs.push((path, lang_status));
    }
    Ok(final_pairs)
}

/// Executes batch subtitle stream extraction with sequential fallback.
///
/// # Arguments
/// * `video_path` - Path to the container file or stream.
/// * `candidates` - List of (subtitle stream index, language tag, output path).
/// * `cleanup` - Cleanup registry for temporary extracted files.
/// * `discovered` - Output accumulator for successfully extracted subtitle tracks.
///
/// # Returns
/// Nothing. Extracted files are pushed to `discovered`.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Tries single-pass multi-stream extraction via FFmpeg (`DBSRT_FFMPEG_THREADS`, default 0 = auto).
/// 2. If single-pass fails, falls back sequentially to `extract_embedded_subtitle`.
/// 3. Registers verified output files in `cleanup.srts` and `discovered`.
fn extract_subtitle_tracks_batch(
    video_path: &Path,
    candidates: &[(usize, Option<String>, PathBuf)],
    cleanup: &CleanupState,
    discovered: &mut Vec<(PathBuf, Option<String>)>,
) {
    let mut cmd = std::process::Command::new(dbsrt_core::video::resolve_ffmpeg_cpu());
    cmd.args(["-v", "error", "-y", "-threads"])
        .arg(dbsrt_core::ffmpeg_decode_threads());
    dbsrt_core::apply_ftp_credentials(&mut cmd, video_path);
    cmd.arg("-i").arg(video_path);
    for (idx, _lang, dest) in candidates {
        let map_arg = format!("0:s:{}", idx);
        cmd.args(["-map", &map_arg, "-f", "srt"]).arg(dest);
    }

    let batch_ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if batch_ok {
        for (_idx, lang_opt, srt_path) in candidates {
            let is_valid =
                srt_path.exists() && srt_path.metadata().map(|m| m.len() > 0).unwrap_or(false);
            if is_valid {
                cleanup.srts.lock().unwrap().push(srt_path.clone());
                discovered.push((srt_path.clone(), lang_opt.clone()));
            }
        }
        return;
    }

    for (idx, lang_opt, srt_path) in candidates {
        let Ok(srt_content) = extract_embedded_subtitle(video_path, *idx) else {
            continue;
        };
        if fs::write(srt_path, &srt_content).is_ok() {
            cleanup.srts.lock().unwrap().push(srt_path.clone());
            discovered.push((srt_path.clone(), lang_opt.clone()));
        }
    }
}

fn extract_embedded_subtitles_and_select(
    video_path: &Path,
    candidates: Vec<(usize, Option<String>, PathBuf)>,
    mut discovered: Vec<(PathBuf, Option<String>)>,
    target_lang: Option<&str>,
    is_json: bool,
) -> Result<Vec<(PathBuf, LangStatus)>> {
    let cleanup = CleanupState::global();
    if !candidates.is_empty() {
        let lang_tags: Vec<String> = candidates
            .iter()
            .map(|(_, l, _)| l.as_deref().unwrap_or("und").to_string())
            .collect();

        if !is_json {
            eprintln!(
                "📦 Extracting {} embedded subtitle track(s) in single pass: {:?}",
                candidates.len(),
                lang_tags
            );
        }

        extract_subtitle_tracks_batch(video_path, &candidates, &cleanup, &mut discovered);
    }

    if discovered.is_empty() {
        bail!(
            "❌ No matching .srt file found for '{:?}'. Specify a subtitle file with '--srt <path>' or download one using 'dbsrt fetch-subtitles -q \"<title>\" -l fr'.",
            video_path
        );
    }

    select_and_validate_subtitle_pairs(&discovered, video_path, target_lang, is_json)
}

static CLEANUP_STATE: OnceLock<Arc<CleanupState>> = OnceLock::new();

pub(crate) struct CleanupState {
    srts: Mutex<Vec<PathBuf>>,
    phash: Mutex<Option<PathBuf>>,
    phash_done: AtomicBool,
}

impl CleanupState {
    pub(crate) fn global() -> Arc<Self> {
        CLEANUP_STATE
            .get_or_init(|| {
                let state = Arc::new(Self {
                    srts: Mutex::new(Vec::new()),
                    phash: Mutex::new(None),
                    phash_done: AtomicBool::new(false),
                });
                let h_state = Arc::clone(&state);
                let _ = ctrlc::set_handler(move || {
                    eprintln!("\n⚠️  Received SIGINT (Ctrl+C). Cleaning up partial files...");
                    if let Ok(guard) = h_state.phash.lock() {
                        if let Some(ref path) = *guard {
                            if path.exists() {
                                let should_remove = match fs::metadata(path) {
                                    Ok(meta) => {
                                        meta.len() < PARTIAL_PHASH_THRESHOLD_BYTES
                                            || !h_state.phash_done.load(Ordering::SeqCst)
                                    }
                                    Err(_) => false,
                                };
                                if should_remove {
                                    let _ = fs::remove_file(path);
                                    eprintln!("   Removed partial pHash: {:?}", path);
                                }
                            }
                        }
                    }
                    if let Ok(guard) = h_state.srts.lock() {
                        for srt in guard.iter() {
                            if srt.exists() {
                                let _ = fs::remove_file(srt);
                                eprintln!("   Removed extracted SRT: {:?}", srt);
                            }
                        }
                    }
                    eprintln!("Cleanup done. Exiting.");
                    std::process::exit(SIGINT_EXIT_CODE);
                });
                state
            })
            .clone()
    }
}

/// CLI arguments for unified `media:add` command.
#[derive(Args, Debug, Clone)]
pub struct MediaAddArgs {
    /// Reference media video file (.mkv, .mp4, .webm, etc.)
    #[arg(short, long, value_name = "VIDEO")]
    pub video: Option<PathBuf>,

    /// Positional video path if not passed with --video
    #[arg(value_name = "VIDEO_PATH")]
    pub video_pos: Option<PathBuf>,

    /// Explicit media name stem override (useful for remote streaming URLs or pipes)
    #[arg(long, value_name = "NAME")]
    pub media_name: Option<String>,

    /// OpenSubtitles moviehash naming the .phash (16 hex chars); computed on a local file when absent
    #[arg(long, value_name = "HASH", value_parser = parse_moviehash)]
    pub moviehash: Option<String>,

    /// Reference subtitle file (.srt)
    #[arg(short, long, value_name = "SRT")]
    pub srt: Option<PathBuf>,

    /// Positional subtitle path if not passed with --srt
    #[arg(value_name = "SRT_PATH")]
    pub srt_pos: Option<PathBuf>,

    /// Subtitle ISO language code (e.g. "fr", "en")
    #[arg(short, long)]
    pub lang: Option<String>,

    /// Explicit framerate override (e.g. 23.976, 24.0, 25.0)
    #[arg(long)]
    pub fps: Option<f64>,

    /// Output directory storing .phash binary fingerprints
    #[arg(long, default_value = DEFAULT_TMP_DIR)]
    pub phash_dir: PathBuf,

    /// Directory storing certified .srt files and couplings.json
    #[arg(long, default_value = DEFAULT_SUBTITLES_DIR)]
    pub subtitles_dir: PathBuf,

    /// Extract all embedded subtitle tracks from container even if sidecar subtitles exist
    #[arg(long)]
    pub all_embedded: bool,

    /// Dry-run mode: extract pHash fingerprint but skip coupling certification and ingest
    #[arg(long, alias = "dry-run")]
    pub skip_certify: bool,

    /// Output execution summary in JSON format
    #[arg(long)]
    pub json: bool,

    /// Hardware acceleration backend (auto, vaapi, cuda, qsv, disabled)
    #[arg(long, default_value = "auto")]
    pub hwaccel: String,

    /// HTTP URL of dbsrt-server for live catalog ingestion
    #[arg(long, default_value = DEFAULT_SERVER_URL)]
    pub server_url: String,

    /// Database connection arguments (if syncing to PostgreSQL)
    #[command(flatten)]
    pub db: PostgresArgs,
}

/// Structured summary of media:add operations.
#[derive(Debug, Serialize, Deserialize)]
pub struct MediaAddResult {
    pub success: bool,
    pub video_path: String,
    pub phash_path: String,
    pub fps: f64,
    pub media_hash: String,
    pub certified_couplings: usize,
    pub skipped_idempotent: usize,
    pub server_ingested: bool,
    pub message: String,
}

/// Resolves and validates the existence of the source video file from CLI arguments.
///
/// # Arguments
/// * `args` - Reference to parsed CLI arguments containing `--video` or positional video path.
///
/// # Returns
/// * `Result<&PathBuf>` - Reference to validated existing file path.
///
/// # Errors
/// * Returns `Err` if no video path was supplied or if target file does not exist on disk.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Prioritizes `--video` flag over positional argument.
/// 2. Asserts existence with `video_path.exists()` before downstream processing.
pub(crate) fn resolve_media_video_path(args: &MediaAddArgs) -> Result<&PathBuf> {
    let video_path = args
        .video
        .as_ref()
        .or(args.video_pos.as_ref())
        .ok_or_else(|| {
            anyhow!("Error: No media video file specified. Usage: dbsrt media:add <video_file> [srt_file] [--lang <code>] [--fps <val>]")
        })?;

    if !dbsrt_core::container::is_remote_stream_or_pipe(video_path) && !video_path.exists() {
        bail!("Media video file not found: {:?}", video_path);
    }
    Ok(video_path)
}

/// Validates `--moviehash`: it becomes a file name, so 16 hexadecimal characters only.
pub(crate) fn parse_moviehash(raw: &str) -> Result<String, String> {
    if raw.len() == 16 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(raw.to_ascii_lowercase())
    } else {
        Err(format!("expected 16 hexadecimal characters, got {raw:?}"))
    }
}

/// Moviehash naming the .phash: `--moviehash`, else computed on a local file; None for a stream without one.
pub(crate) fn resolve_moviehash(explicit: Option<&str>, video_path: &Path) -> Option<String> {
    if let Some(hash) = explicit {
        return Some(hash.to_string());
    }
    if dbsrt_core::container::is_remote_stream_or_pipe(video_path) {
        return None;
    }
    dbsrt_core::metadata::compute_opensubtitles_hash(video_path).ok()
}

/// Derives logical media stem name, respecting explicit `--media-name` overrides and remote stream URLs.
pub(crate) fn resolve_media_stem(args: &MediaAddArgs, video_path: &Path) -> String {
    if let Some(ref name) = args.media_name {
        return name.clone();
    }
    let raw = video_path.to_string_lossy();
    if dbsrt_core::container::is_remote_stream_or_pipe(video_path) {
        let unquoted = raw.split('?').next().unwrap_or(&raw);
        let last_segment = unquoted.rsplit('/').next().unwrap_or(unquoted);
        if let Some((stem, _)) = last_segment.rsplit_once('.') {
            return stem.to_string();
        }
        return last_segment.to_string();
    }
    video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Derives logical media filename with extension, respecting explicit `--media-name` overrides and remote stream URLs.
pub(crate) fn resolve_media_filename(args: &MediaAddArgs, video_path: &Path) -> String {
    if let Some(ref name) = args.media_name {
        return name.clone();
    }
    let raw = video_path.to_string_lossy();
    if dbsrt_core::container::is_remote_stream_or_pipe(video_path) {
        let unquoted = raw.split('?').next().unwrap_or(&raw);
        let last_segment = unquoted.rsplit('/').next().unwrap_or(unquoted);
        return last_segment.to_string();
    }
    video_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Resolves and validates an explicitly provided subtitle file and its language status.
///
/// # Arguments
/// * `srt_file` - Path to explicit subtitle file.
/// * `lang_override` - Optional ISO language code from `--lang`.
/// * `is_json` - Suppresses terminal user warnings if true.
///
/// # Returns
/// * `Result<Vec<(PathBuf, LangStatus)>>` - Single-element vector of paired subtitle path and language status.
///
/// # Errors
/// * Returns `Err` if the subtitle file does not exist or language code cannot be inferred/resolved.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Verifies explicit file existence on disk.
/// 2. Resolves language in priority: CLI `--lang` -> filename tag -> heuristics -> Unknown ("und").
/// 3. Rejects empty or unresolved "auto" codes immediately.
pub(crate) fn resolve_explicit_subtitle_pair(
    srt_file: &Path,
    lang_override: Option<&str>,
    is_json: bool,
) -> Result<Vec<(PathBuf, LangStatus)>> {
    if !srt_file.exists() {
        bail!("Specified subtitle file not found: {:?}", srt_file);
    }
    let srt_name = srt_file
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or_default();

    let lang_status = if let Some(l) = lang_override {
        LangStatus::Confirmed(l.to_lowercase())
    } else if let Some(code) = parse_language_code(srt_name) {
        if !is_json {
            eprintln!("   Lang detected from filename: {}", code);
        }
        LangStatus::Detected(code)
    } else {
        let status = detect_cli_subtitle_lang_status(srt_name, "auto");
        match status {
            LangStatus::Detected(code) => {
                if !is_json {
                    eprintln!("   Lang detected from filename: {}", code);
                }
                LangStatus::Detected(code)
            }
            _ => {
                if !is_json {
                    eprintln!("⚠️  Language could not be determined from filename.");
                    eprintln!("   Coupling will be certified with lang=\"und\" (undetermined).");
                    eprintln!("   To fix later: dbsrt coupling:relabel --media <hash> --sub <hash> --lang <code>");
                    eprintln!("   Or specify --lang <code> now to set it explicitly.");
                }
                LangStatus::Unknown
            }
        }
    };

    let lang_code = lang_status.to_lang_code();
    if lang_code == "auto" || lang_code.is_empty() {
        bail!(
            "❌ Could not determine subtitle language for '{}'. Please specify it explicitly via --lang <fr|en|es|...>",
            srt_name
        );
    }
    Ok(vec![(srt_file.to_path_buf(), lang_status)])
}

/// Checks if any container stream explicitly matches the requested target language.
fn has_explicit_language_match(
    streams: &[dbsrt_core::container::SubtitleStreamInfo],
    target: &str,
) -> bool {
    for s in streams {
        let parsed = s.language.as_deref().and_then(parse_language_code);
        if language_matches(s.language.as_deref(), target) || parsed.as_deref() == Some(target) {
            return true;
        }
    }
    false
}

/// Evaluates if an individual subtitle stream satisfies language filter criteria.
fn stream_matches_target_lang(
    raw_lang: Option<&str>,
    target: &str,
    parsed_lang: Option<&str>,
    has_explicit_match: bool,
) -> bool {
    match raw_lang {
        Some(tag) => language_matches(Some(tag), target) || parsed_lang == Some(target),
        None => !has_explicit_match,
    }
}

/// Filters and prepares embedded subtitle stream extraction candidates from container streams.
fn collect_embedded_subtitle_candidates(
    args: &MediaAddArgs,
    streams: &[dbsrt_core::container::SubtitleStreamInfo],
    video_stem: &str,
) -> Vec<(usize, Option<String>, PathBuf)> {
    let mut candidates = Vec::new();
    let mut seen_extracted_langs = HashSet::new();
    let mut seen_no_lang = false;

    let has_explicit_match = args
        .lang
        .as_ref()
        .map(|t| has_explicit_language_match(streams, t))
        .unwrap_or(false);

    for stream in streams {
        if BITMAP_SUB_CODECS
            .iter()
            .any(|c| stream.codec_name.eq_ignore_ascii_case(c))
        {
            if !args.json {
                eprintln!(
                    "ℹ️  Skipping bitmap subtitle track #{} (codec: {})",
                    stream.subtitle_index, stream.codec_name
                );
            }
            continue;
        }
        let stream_lang_opt = stream.language.as_deref().and_then(parse_language_code);

        if let Some(ref target) = args.lang {
            if !stream_matches_target_lang(
                stream.language.as_deref(),
                target,
                stream_lang_opt.as_deref(),
                has_explicit_match,
            ) {
                continue;
            }
        }

        let lang_opt = stream_lang_opt.or_else(|| {
            if let Some(ref target) = args.lang {
                Some(target.clone())
            } else {
                stream.language.clone()
            }
        });

        let is_duplicate = match &lang_opt {
            Some(lang) => !seen_extracted_langs.insert(lang.clone()),
            None => {
                let dup = seen_no_lang;
                seen_no_lang = true;
                dup
            }
        };
        if is_duplicate {
            if !args.json {
                let label = lang_opt.as_deref().unwrap_or("untagged");
                eprintln!(
                    "ℹ️  Skipping duplicate {} track (stream #{})",
                    label, stream.subtitle_index
                );
            }
            continue;
        }

        let srt_fname = match &lang_opt {
            Some(lang) => format!("{}.{}.srt", video_stem, lang),
            None => format!("{}.srt", video_stem),
        };
        let srt_path = args.subtitles_dir.join(&srt_fname);
        candidates.push((stream.subtitle_index, lang_opt, srt_path));
    }
    candidates
}

/// Inspects media container for embedded subtitle tracks and prepares extraction candidates.
///
/// # Arguments
/// * `args` - CLI options (subtitles_dir, lang, skip_certify, json).
/// * `video_path` - Path to media video file.
/// * `video_stem` - Filename stem of video without extension.
/// * `discovered` - Previously discovered sidecar subtitle files.
///
/// # Returns
/// * `Result<SubtitleJob>` - Extraction job or already resolved fallback.
///
/// # Errors
/// * Returns `Err` if no subtitles found on disk/container and certification was not skipped.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Probes container metadata streams with `inspect_container`.
/// 2. Filters out unsupported bitmap codecs (`BITMAP_SUB_CODECS`).
/// 3. Discards duplicates and aligns language tags with `--lang`.
/// 4. Falls back to sidecars or raises error if neither sidecar nor embedded track exists.
pub(crate) fn resolve_container_embedded_subtitles(
    args: &MediaAddArgs,
    video_path: &Path,
    video_stem: &str,
    discovered: &[(PathBuf, Option<String>)],
) -> Result<SubtitleJob> {
    if args.skip_certify && !args.all_embedded {
        if !discovered.is_empty() {
            let pairs = select_and_validate_subtitle_pairs(
                discovered,
                video_path,
                args.lang.as_deref(),
                args.json,
            )?;
            return Ok(SubtitleJob::AlreadyResolved(pairs));
        }
        return Ok(SubtitleJob::AlreadyResolved(Vec::new()));
    }

    if !args.json {
        if let Some(ref target) = args.lang {
            eprintln!(
                "ℹ️  No sidecar matches requested lang '{}', trying embedded tracks",
                target
            );
        }
        eprintln!("🔍 Inspecting video container for embedded subtitle tracks...");
    }

    let candidates = match inspect_container(video_path) {
        Ok(insp) if !insp.subtitle_streams.is_empty() => {
            fs::create_dir_all(&args.subtitles_dir)?;
            collect_embedded_subtitle_candidates(args, &insp.subtitle_streams, video_stem)
        }
        Ok(_) => Vec::new(),
        Err(e) => {
            if !args.json {
                eprintln!("⚠️  Could not inspect video container: {}", e);
            }
            Vec::new()
        }
    };

    if candidates.is_empty() {
        if discovered.is_empty() {
            if !args.skip_certify {
                bail!(
                    "❌ No matching .srt file found for '{:?}'. Specify a subtitle file with '--srt <path>' or download one using 'dbsrt fetch-subtitles -q \"<title>\" -l fr'.",
                    video_path
                );
            }
            Ok(SubtitleJob::AlreadyResolved(Vec::new()))
        } else {
            let pairs = select_and_validate_subtitle_pairs(
                discovered,
                video_path,
                args.lang.as_deref(),
                args.json,
            )?;
            Ok(SubtitleJob::AlreadyResolved(pairs))
        }
    } else {
        Ok(SubtitleJob::NeedsExtraction {
            candidates,
            discovered: discovered.to_vec(),
        })
    }
}

/// Plans the subtitle discovery and pairing strategy for media addition.
///
/// # Arguments
/// * `args` - CLI arguments.
/// * `video_path` - Path to media video file.
///
/// # Returns
/// * `Result<SubtitleJob>` - Configured subtitle plan (already resolved or needs extraction).
///
/// # Errors
/// * Returns `Err` if explicit subtitle is missing or no subtitles match required constraints.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Checks explicit `--srt` option first via `resolve_explicit_subtitle_pair`.
/// 2. If no explicit subtitle, scans for filesystem sidecars matching video stem.
/// 3. If sidecars match target language and `!all_embedded`, resolves pairs immediately.
/// 4. Otherwise delegates to container inspection via `resolve_container_embedded_subtitles`.
pub(crate) fn plan_subtitle_job(args: &MediaAddArgs, video_path: &Path) -> Result<SubtitleJob> {
    let explicit_srt = args.srt.as_ref().or(args.srt_pos.as_ref());
    if let Some(srt_file) = explicit_srt {
        let pairs = resolve_explicit_subtitle_pair(srt_file, args.lang.as_deref(), args.json)?;
        return Ok(SubtitleJob::AlreadyResolved(pairs));
    }

    let video_stem = resolve_media_stem(args, video_path);

    if !args.json {
        eprintln!("🔍 Scanning for sidecar .srt files matching video stem...");
        eprintln!("   Video stem: {}", video_stem);
        eprintln!("   Search dirs: parent + Subs/subtitles subdirs");
    }

    let discovered = find_subtitle_pairs(video_path)?;

    if !args.json {
        if discovered.is_empty() {
            eprintln!(
                "ℹ️  No sidecar .srt found, will inspect video container for embedded tracks"
            );
        } else {
            eprintln!("✓ Found {} sidecar subtitle(s):", discovered.len());
            for (path, lang_opt) in &discovered {
                let lang_display = match (lang_opt, &args.lang) {
                    (Some(l), _) => format!("detected: {}", l),
                    (None, Some(target)) => {
                        format!("none in filename, will use --lang: {}", target)
                    }
                    (None, None) => "none in filename (will infer)".to_string(),
                };
                eprintln!(
                    "   • {:?} ({})",
                    path.file_name().unwrap_or_default(),
                    lang_display
                );
            }
        }
    }

    let sidecars_match_target = if args.all_embedded {
        false
    } else if let Some(ref target) = args.lang {
        discovered
            .iter()
            .any(|(_, l)| l.as_deref() == Some(target.as_str()) || l.is_none())
    } else {
        !discovered.is_empty()
    };

    if sidecars_match_target {
        let pairs = select_and_validate_subtitle_pairs(
            &discovered,
            video_path,
            args.lang.as_deref(),
            args.json,
        )?;
        Ok(SubtitleJob::AlreadyResolved(pairs))
    } else {
        resolve_container_embedded_subtitles(args, video_path, &video_stem, &discovered)
    }
}

/// Extracts video pHash fingerprint frames and embedded subtitles concurrently.
///
/// # Arguments
/// * `args` - CLI arguments.
/// * `video_path` - Path to media video file.
/// * `sub_job` - Subtitle plan from `plan_subtitle_job`.
/// * `fps` - Media framerate in frames per second.
/// * `cleanup` - Global cleanup state tracker for SIGINT protection.
///
/// # Returns
/// * `Result<(Vec<(PathBuf, LangStatus)>, PathBuf, Vec<u64>, f64)>` - Resolved subtitle pairs, generated phash file path, vector of 64-bit frame hashes, and detected fps.
///
/// # Errors
/// * Returns `Err` if ffmpeg extraction fails or background task panics.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Initializes estimated phash file path into cleanup guard.
/// 2. If subtitles are already resolved, extracts pHash synchronously.
/// 3. Extracts embedded container subtitles sequentially followed by visual pHash frames.
/// 4. Marks cleanup state `phash_done = true`.
async fn extract_embedded_and_phash(
    video_path: &Path,
    candidates: Vec<(usize, Option<String>, PathBuf)>,
    discovered: Vec<(PathBuf, Option<String>)>,
    args: &MediaAddArgs,
    moviehash: Option<String>,
) -> Result<(Vec<(PathBuf, LangStatus)>, PathBuf, Vec<u64>, f64)> {
    let video_path_sub = video_path.to_path_buf();
    let video_path_phash = video_path.to_path_buf();
    let phash_dir = args.phash_dir.clone();
    let target_lang = args.lang.clone();
    let is_json = args.json;
    let hw_backend = parse_hwaccel(&args.hwaccel);

    let sub_task = tokio::task::spawn_blocking(move || -> Result<Vec<(PathBuf, LangStatus)>> {
        extract_embedded_subtitles_and_select(
            &video_path_sub,
            candidates,
            discovered,
            target_lang.as_deref(),
            is_json,
        )
    });

    let pairs = sub_task
        .await
        .map_err(|e| anyhow!("Subtitle extraction task panicked: {}", e))??;

    let phash_task = tokio::task::spawn_blocking(move || -> Result<(PathBuf, Vec<u64>, f64)> {
        find_or_build_phash_with_hwaccel(
            &video_path_phash,
            &phash_dir,
            hw_backend,
            moviehash.as_deref(),
        )
    });

    let (phash_path, hashes, detected_fps) = phash_task
        .await
        .map_err(|e| anyhow!("pHash extraction task panicked: {}", e))??;

    Ok((pairs, phash_path, hashes, detected_fps))
}

pub(crate) async fn extract_phash_and_subtitles(
    args: &MediaAddArgs,
    video_path: &Path,
    sub_job: SubtitleJob,
    fps: f64,
    cleanup: &Arc<CleanupState>,
) -> Result<(Vec<(PathBuf, LangStatus)>, PathBuf, Vec<u64>, f64)> {
    fs::create_dir_all(&args.phash_dir)?;

    let video_name = resolve_media_filename(args, video_path);
    let moviehash = resolve_moviehash(args.moviehash.as_deref(), video_path);
    let estimated_phash =
        args.phash_dir
            .join(phash_file_name(&video_name, moviehash.as_deref(), fps));
    *cleanup.phash.lock().unwrap() = Some(estimated_phash);

    if !args.json {
        eprintln!(
            "🎬 Extracting pHash frames via ffmpeg (may take a few minutes for long videos)..."
        );
        eprintln!("   Video: {:?}", video_path);
        eprintln!("   FPS detected: {:.3}", fps);
    }

    let (pairs, phash_path, hashes, detected_fps) =
        run_phash_job(args, video_path, sub_job, moviehash.as_deref()).await?;
    if let Some(hash) = &moviehash {
        let sidecar = PhashSidecar {
            moviehash: hash,
            media_name: &video_name,
            fps: detected_fps,
            frames: hashes.len(),
        };
        sidecar.write(&args.phash_dir)?;
    }

    cleanup.phash_done.store(true, Ordering::SeqCst);
    *cleanup.phash.lock().unwrap() = Some(phash_path.clone());
    Ok((pairs, phash_path, hashes, detected_fps))
}

fn parse_hwaccel(name: &str) -> dbsrt_core::HwAccel {
    match name.to_lowercase().as_str() {
        "vaapi" => dbsrt_core::HwAccel::Vaapi,
        "cuda" => dbsrt_core::HwAccel::Cuda,
        "qsv" => dbsrt_core::HwAccel::Qsv,
        "disabled" | "none" | "cpu" => dbsrt_core::HwAccel::Disabled,
        _ => dbsrt_core::HwAccel::Auto,
    }
}

/// pHash of `video_path`, alongside the embedded subtitle extraction when the job needs it.
async fn run_phash_job(
    args: &MediaAddArgs,
    video_path: &Path,
    sub_job: SubtitleJob,
    moviehash: Option<&str>,
) -> Result<(Vec<(PathBuf, LangStatus)>, PathBuf, Vec<u64>, f64)> {
    match sub_job {
        SubtitleJob::AlreadyResolved(pairs) => {
            let (phash_path, hashes, detected_fps) = find_or_build_phash_with_hwaccel(
                video_path,
                &args.phash_dir,
                parse_hwaccel(&args.hwaccel),
                moviehash,
            )?;
            Ok((pairs, phash_path, hashes, detected_fps))
        }
        SubtitleJob::NeedsExtraction {
            candidates,
            discovered,
        } => {
            let moviehash = moviehash.map(str::to_string);
            extract_embedded_and_phash(video_path, candidates, discovered, args, moviehash).await
        }
    }
}

/// Certifies subtitle couplings and sub_frames in Zero-DB registry with CAS idempotence.
///
/// # Arguments
/// * `args` - CLI options (subtitles_dir, json).
/// * `pairs` - Subtitle file paths and resolved language statuses.
/// * `hashes` - Full video pHash frames sequence.
/// * `fps` - Media framerate.
/// * `phash_path` - Path to `.phash` file on disk.
/// * `media_hash` - 32-character SHA256 digest of video frame hashes.
///
/// # Returns
/// * `Result<(usize, usize)>` - Tuple of (certified_count, skipped_count).
///
/// # Errors
/// * Returns `Err` if registry lock cannot be acquired or file I/O fails.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Acquires cross-process lock on `couplings.json`.
/// 2. Iterates each subtitle candidate and computes content hash.
/// 3. Checks CAS condition `is_already_certified`: skips if already recorded.
/// 4. Copies subtitle into `subtitles_dir` if external.
/// 5. Validates cue alignment and generates visual `sub_frames`.
/// 6. Calls `upsert_coupling` and commits to disk atomically with `save()`.
pub(crate) fn certify_subtitles_in_registry(
    args: &MediaAddArgs,
    pairs: &[(PathBuf, LangStatus)],
    hashes: &[u64],
    fps: f64,
    phash_path: &Path,
    media_hash: &str,
) -> Result<(usize, usize)> {
    fs::create_dir_all(&args.subtitles_dir)?;
    let reg_path = CouplingRegistry::default_path(&args.subtitles_dir);
    let _reg_lock = dbsrt_core::RegistryLock::acquire(&reg_path)?;
    let mut reg = CouplingRegistry::load_or_default(&reg_path)?;

    let mut certified_count = 0;
    let mut skipped_count = 0;

    let phash_fname = phash_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let media_name = phash_fname
        .strip_suffix(".phash")
        .unwrap_or(&phash_fname)
        .to_string();

    if !args.json {
        eprintln!("📋 Certifying {} subtitle coupling(s)...", pairs.len());
    }

    for (srt_path, lang_status) in pairs {
        let raw_srt_bytes = fs::read(srt_path)?;
        let subtitles_hash = compute_bytes_digest_32(&raw_srt_bytes);

        let srt_fname = srt_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let lang = lang_status.to_lang_code();
        if lang == "auto" || lang.is_empty() {
            bail!(
                "❌ Could not determine subtitle language for '{}'. Please specify it explicitly via --lang <fr|en|es|...>",
                srt_fname
            );
        }

        if reg.is_already_certified(media_hash, &subtitles_hash) {
            skipped_count += 1;
            if !args.json {
                eprintln!(
                    "ℹ️  Already certified: '{}' (media_hash={}, sub_hash={}) -> skipping",
                    srt_fname, media_hash, subtitles_hash
                );
            }
            continue;
        }

        let dest_srt_path = if srt_path.starts_with(&args.subtitles_dir) {
            srt_path.clone()
        } else {
            let dest = args.subtitles_dir.join(&srt_fname);
            if dest != *srt_path {
                fs::copy(srt_path, &dest)?;
            }
            dest
        };

        let srt_file = SubtitleFile::open(&dest_srt_path)?;
        let sub_frames = build_and_validate_sub_frames_opts(&srt_file, hashes, fps, true)?;

        let rel_srt_name = dest_srt_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let source_kind = if srt_path.starts_with(&args.subtitles_dir)
            && pairs
                .iter()
                .any(|(p, _)| p.starts_with(&args.subtitles_dir))
        {
            "embedded-extracted"
        } else {
            "sidecar"
        };

        if !args.json {
            eprintln!("✓ Certified coupling #{}:", certified_count + 1);
            eprintln!("   Source: {}", source_kind);
            eprintln!("   SRT: {}", srt_fname);
            eprintln!(
                "   Final lang: {} (from: lang_status={:?})",
                lang, lang_status
            );
            match lang_status {
                LangStatus::Unknown => {
                    eprintln!("   Lang status: Unknown (certified as 'und' — needs relabel)");
                }
                other => {
                    eprintln!("   Lang status: {:?}", other);
                }
            }
            eprintln!("   Cues: {}", srt_file.cues.len());
        }

        reg.upsert_coupling(
            media_hash.to_string(),
            media_name.clone(),
            phash_fname.clone(),
            fps,
            hashes.len(),
            CertifiedSubtitleRecord {
                name: rel_srt_name.clone(),
                srt_path: rel_srt_name.clone(),
                lang: lang.clone(),
                lang_status: lang_status.clone(),
                subtitles_hash: subtitles_hash.clone(),
                certified: true,
                cue_count: srt_file.cues.len(),
                sub_frames,
            },
        );
        certified_count += 1;
    }

    if certified_count > 0 {
        if !args.json {
            eprintln!("💾 Persisting coupling registry to disk...");
        }
        reg.save(&reg_path)?;
    }

    Ok((certified_count, skipped_count))
}

/// Triggers dynamic RAM catalog ingestion on active `dbsrt-server` instance.
///
/// # Arguments
/// * `server_url` - Base HTTP URL of dbsrt-server daemon.
/// * `phash_path` - Path to `.phash` file on disk.
/// * `fps` - Media framerate.
/// * `is_json` - Suppresses terminal user warnings if true.
///
/// # Returns
/// * `Result<bool>` - True if HTTP ingestion succeeded, false if server is offline or returned warning.
///
/// # Errors
/// * Returns `Err` if HTTP client construction fails.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Constructs POST `/v1/catalog/ingest` payload with `phash_path` and `fps`.
/// 2. Bounded by `DEFAULT_HTTP_TIMEOUT_SECS` timeout.
/// 3. Tolerates offline daemon gracefully: coupling remains durable on disk.
pub(crate) async fn trigger_server_catalog_ingest(
    server_url: &str,
    phash_path: &Path,
    fps: f64,
    is_json: bool,
) -> Result<bool> {
    if !is_json {
        eprintln!("🚀 Triggering dynamic RAM ingestion on dbsrt-server...");
    }
    let ingest_url = format!("{}/v1/catalog/ingest", server_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS))
        .build()?;

    let payload = serde_json::json!({
        "phash_path": phash_path.to_string_lossy(),
        "fps": fps
    });

    match client.post(&ingest_url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            if !is_json {
                eprintln!(
                    "✓ Dynamic RAM ingestion succeeded on dbsrt-server ({})",
                    ingest_url
                );
            }
            Ok(true)
        }
        Ok(resp) => {
            if !is_json {
                eprintln!(
                    "⚠️  dbsrt-server catalog ingest returned HTTP {}. Coupling remains certified on disk.",
                    resp.status()
                );
            }
            Ok(false)
        }
        Err(_) => {
            if !is_json {
                eprintln!(
                    "⚠️  dbsrt-server offline ({}): .phash extracted and coupling certified to disk, will be loaded on next server startup.",
                    server_url
                );
            }
            Ok(false)
        }
    }
}

/// Renders execution summary to terminal stdout in either JSON or human-readable format.
///
/// # Arguments
/// * `args` - CLI options (`json` flag).
/// * `result` - Structured operation metrics and outcomes.
///
/// # Returns
/// * `Result<()>` - Successful stdout write.
///
/// # Errors
/// * Returns `Err` if JSON serialization fails.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Outputs structured JSON when `--json` flag is provided.
/// 2. Prints formatted CLI banner and key/value table otherwise.
pub(crate) fn render_media_add_output(args: &MediaAddArgs, result: &MediaAddResult) -> Result<()> {
    if args.json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        println!("=== MEDIA:ADD INGESTION SUMMARY ===");
        println!("Media Video      : {}", result.video_path);
        println!("Framerate (FPS)  : {:.3}", result.fps);
        println!("pHash File       : {}", result.phash_path);
        println!("Couplings Added  : {}", result.certified_couplings);
        println!("Couplings Skipped: {}", result.skipped_idempotent);
        println!("Catalog Ingested : {}", result.server_ingested);
    }
    Ok(())
}

/// Executes unified media addition: pHash extraction -> certification -> dynamic ingest.
///
/// # Arguments
/// * `args` - Parsed CLI options and arguments.
///
/// # Returns
/// * `Result<MediaAddResult>` - Execution metrics and outcomes.
///
/// # Errors
/// * Returns `Err` if input validation, pHash extraction, or certification fails.
///
/// # Panics
/// * Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Cleans up any existing global partial state.
/// 2. Resolves media video path and plans subtitle pairing.
/// 3. Extracts pHash frames and subtitles (concurrently if needed).
/// 4. Handles early exit if `skip_certify` is active.
/// 5. Certifies couplings in `couplings.json` registry with CAS idempotence.
/// 6. Triggers live server ingest if new couplings were registered.
/// 7. Clears cleanup guards and renders output summary.
pub async fn run_media_add(args: &MediaAddArgs) -> Result<MediaAddResult> {
    let cleanup = CleanupState::global();
    cleanup.srts.lock().unwrap().clear();
    *cleanup.phash.lock().unwrap() = None;
    cleanup.phash_done.store(false, Ordering::SeqCst);

    let video_path = resolve_media_video_path(args)?;
    let sub_job = plan_subtitle_job(args, video_path)?;
    let fps = detect_media_fps(video_path, args.fps);

    let (pairs, phash_path, hashes, _detected_fps) =
        extract_phash_and_subtitles(args, video_path, sub_job, fps, &cleanup).await?;

    let media_hash = compute_media_phash_digest(&hashes);

    if !args.json {
        eprintln!(
            "✓ pHash extraction completed: {} frames computed",
            hashes.len()
        );
        eprintln!("   Media hash: {}", media_hash);
    }

    if args.skip_certify {
        let result = MediaAddResult {
            success: true,
            video_path: video_path.to_string_lossy().to_string(),
            phash_path: phash_path.to_string_lossy().to_string(),
            fps,
            media_hash,
            certified_couplings: 0,
            skipped_idempotent: 0,
            server_ingested: false,
            message: "pHash extracted. Skipped certification and ingest (--skip-certify)"
                .to_string(),
        };
        cleanup.srts.lock().unwrap().clear();
        *cleanup.phash.lock().unwrap() = None;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("✓ pHash extracted to {:?}", phash_path);
            println!("  [DRY-RUN] Certification and catalog ingest skipped (--skip-certify)");
        }
        return Ok(result);
    }

    let (certified_count, skipped_count) =
        certify_subtitles_in_registry(args, &pairs, &hashes, fps, &phash_path, &media_hash)?;

    let server_ingested = if certified_count > 0 {
        trigger_server_catalog_ingest(&args.server_url, &phash_path, fps, args.json).await?
    } else {
        if !args.json {
            eprintln!(
                "ℹ️  All couplings already certified (idempotent skip). Server ingest skipped."
            );
        }
        false
    };

    cleanup.srts.lock().unwrap().clear();
    *cleanup.phash.lock().unwrap() = None;

    let result = MediaAddResult {
        success: true,
        video_path: video_path.to_string_lossy().to_string(),
        phash_path: phash_path.to_string_lossy().to_string(),
        fps,
        media_hash,
        certified_couplings: certified_count,
        skipped_idempotent: skipped_count,
        server_ingested,
        message: format!(
            "Ingestion completed: {} certified, {} skipped (idempotent)",
            certified_count, skipped_count
        ),
    };

    render_media_add_output(args, &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    struct TempTestDir {
        path: PathBuf,
    }

    impl TempTestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dbsrt_test_media_add_{}_{}_{}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).expect("Failed to create temp test directory");
            Self { path }
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test]
    async fn test_media_add_missing_video_returns_error() {
        let args = MediaAddArgs {
            video: Some(PathBuf::from("non_existent_file.mkv")),
            video_pos: None,
            media_name: None,
            moviehash: None,
            srt: None,
            srt_pos: None,
            lang: None,
            fps: None,
            phash_dir: PathBuf::from("/tmp"),
            subtitles_dir: PathBuf::from("/tmp"),
            all_embedded: false,
            skip_certify: false,
            hwaccel: "auto".to_string(),
            json: false,
            server_url: DEFAULT_SERVER_URL.to_string(),
            db: PostgresArgs::default(),
        };

        let res = run_media_add(&args).await;
        assert!(res.is_err(), "Expected error for non-existent video");
    }

    #[tokio::test]
    async fn test_media_add_missing_srt_returns_clear_error() {
        let temp = TempTestDir::new("no_srt");
        let video = temp.path.join("sample.mp4");
        File::create(&video).unwrap();

        let args = MediaAddArgs {
            video: Some(video),
            video_pos: None,
            media_name: None,
            moviehash: None,
            srt: None,
            srt_pos: None,
            lang: None,
            fps: None,
            phash_dir: temp.path.join("phash"),
            subtitles_dir: temp.path.join("subtitles"),
            all_embedded: false,
            skip_certify: false,
            hwaccel: "auto".to_string(),
            json: false,
            server_url: DEFAULT_SERVER_URL.to_string(),
            db: PostgresArgs::default(),
        };

        let err = run_media_add(&args).await.unwrap_err();
        assert!(
            err.to_string().contains("No matching .srt file found"),
            "Expected clear error message about missing SRT: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_media_add_unknown_language_rejects_auto() {
        let temp = TempTestDir::new("auto_lang");
        let video = temp.path.join("sample.mp4");
        let srt = temp.path.join("random_unnamed_sub.srt");
        File::create(&video).unwrap();
        fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();

        let args = MediaAddArgs {
            video: Some(video),
            video_pos: None,
            media_name: None,
            moviehash: None,
            srt: Some(srt),
            srt_pos: None,
            lang: Some("auto".to_string()),
            fps: Some(24.0),
            phash_dir: temp.path.join("phash"),
            subtitles_dir: temp.path.join("subtitles"),
            all_embedded: false,
            skip_certify: false,
            hwaccel: "auto".to_string(),
            json: false,
            server_url: DEFAULT_SERVER_URL.to_string(),
            db: PostgresArgs::default(),
        };

        let err = run_media_add(&args).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("Could not determine subtitle language"),
            "Expected error rejecting 'auto' language: {}",
            err
        );
    }

    #[test]
    fn test_resolve_media_stem_and_filename_remote_and_local() {
        let default_args = MediaAddArgs {
            video: None,
            video_pos: None,
            media_name: None,
            moviehash: None,
            srt: None,
            srt_pos: None,
            lang: None,
            fps: None,
            phash_dir: PathBuf::from("/tmp"),
            subtitles_dir: PathBuf::from("/tmp"),
            all_embedded: false,
            skip_certify: false,
            hwaccel: "auto".to_string(),
            json: false,
            server_url: DEFAULT_SERVER_URL.to_string(),
            db: PostgresArgs::default(),
        };

        // 1. Local path
        let local = Path::new("/media/storage/tvshow/The.Wire.S01E01.1080p.mkv");
        assert_eq!(
            resolve_media_stem(&default_args, local),
            "The.Wire.S01E01.1080p"
        );
        assert_eq!(
            resolve_media_filename(&default_args, local),
            "The.Wire.S01E01.1080p.mkv"
        );

        // 2. Remote FTP URL
        let remote_ftp =
            Path::new("ftp://user:secret@nas.local:21/tvshow/WestWorld/WestWorld.S01E02.mkv");
        assert_eq!(
            resolve_media_stem(&default_args, remote_ftp),
            "WestWorld.S01E02"
        );
        assert_eq!(
            resolve_media_filename(&default_args, remote_ftp),
            "WestWorld.S01E02.mkv"
        );

        // 3. Remote HTTP stream with query parameters
        let remote_http = Path::new("http://127.0.0.1:8080/stream/Carnivale.S01E01.mkv?token=123");
        assert_eq!(
            resolve_media_stem(&default_args, remote_http),
            "Carnivale.S01E01"
        );
        assert_eq!(
            resolve_media_filename(&default_args, remote_http),
            "Carnivale.S01E01.mkv"
        );

        // 4. Pipe with explicit --media-name
        let pipe = Path::new("pipe:0");
        let with_override = MediaAddArgs {
            media_name: Some("Custom.Title.2024".to_string()),
            ..default_args
        };
        assert_eq!(
            resolve_media_stem(&with_override, pipe),
            "Custom.Title.2024"
        );
        assert_eq!(
            resolve_media_filename(&with_override, pipe),
            "Custom.Title.2024"
        );
    }

    #[test]
    fn test_parse_moviehash_accepts_16_hex_only() {
        assert_eq!(
            parse_moviehash("E19D5212C9812CD6").unwrap(),
            "e19d5212c9812cd6"
        );
        assert!(parse_moviehash("e19d").is_err());
        // It becomes a file name: no path component may get through
        assert!(parse_moviehash("../../etc/passw").is_err());
        assert!(parse_moviehash("e19d5212c9812cd6/").is_err());
    }

    #[test]
    fn test_resolve_moviehash_explicit_then_local_then_none() {
        let remote = Path::new("ftp://nas:21/video/a.mkv");
        assert_eq!(
            resolve_moviehash(Some("e19d5212c9812cd6"), remote).as_deref(),
            Some("e19d5212c9812cd6")
        );
        assert_eq!(resolve_moviehash(None, remote), None);

        let dir = TempTestDir::new("moviehash");
        let local = dir.path.join("a.mkv");
        let data: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
        fs::write(&local, data).unwrap();
        assert_eq!(
            resolve_moviehash(None, &local).as_deref(),
            Some("e19d5212c9812cd6")
        );
    }
}
