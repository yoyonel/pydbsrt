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
use crate::commands::shared::find_or_build_phash_with_hwaccel;
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

enum SubtitleJob {
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

        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.args(["-v", "error", "-y", "-i"]).arg(video_path);
        for (idx, _lang, dest) in &candidates {
            let map_arg = format!("0:s:{}", idx);
            cmd.args(["-map", &map_arg, "-f", "srt"]).arg(dest);
        }

        let batch_ok = cmd.status().map(|s| s.success()).unwrap_or(false);
        if batch_ok {
            for (_idx, lang_opt, srt_path) in candidates {
                if srt_path.exists() && srt_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                    cleanup.srts.lock().unwrap().push(srt_path.clone());
                    discovered.push((srt_path, lang_opt));
                }
            }
        } else {
            // Fallback to sequential extraction
            for (idx, lang_opt, srt_path) in candidates {
                if let Ok(srt_content) = extract_embedded_subtitle(video_path, idx) {
                    if fs::write(&srt_path, &srt_content).is_ok() {
                        cleanup.srts.lock().unwrap().push(srt_path.clone());
                        discovered.push((srt_path, lang_opt));
                    }
                }
            }
        }
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

struct CleanupState {
    srts: Mutex<Vec<PathBuf>>,
    phash: Mutex<Option<PathBuf>>,
    phash_done: AtomicBool,
}

impl CleanupState {
    fn global() -> Arc<Self> {
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

/// Executes unified media addition: pHash extraction -> certification -> dynamic ingest.
pub async fn run_media_add(args: &MediaAddArgs) -> Result<MediaAddResult> {
    let cleanup = CleanupState::global();
    cleanup.srts.lock().unwrap().clear();
    *cleanup.phash.lock().unwrap() = None;
    cleanup.phash_done.store(false, Ordering::SeqCst);

    // 1. Resolve video path
    let video_path = args
        .video
        .as_ref()
        .or(args.video_pos.as_ref())
        .ok_or_else(|| {
            anyhow!("Error: No media video file specified. Usage: dbsrt media:add <video_file> [srt_file] [--lang <code>] [--fps <val>]")
        })?;

    if !video_path.exists() {
        bail!("Media video file not found: {:?}", video_path);
    }

    // 2. Resolve subtitle pairs / pre-flight
    let explicit_srt = args.srt.as_ref().or(args.srt_pos.as_ref());
    let sub_job: SubtitleJob = if let Some(srt_file) = explicit_srt {
        if !srt_file.exists() {
            bail!("Specified subtitle file not found: {:?}", srt_file);
        }
        let srt_name = srt_file
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default();
        let lang_status = if let Some(ref l) = args.lang {
            LangStatus::Confirmed(l.to_lowercase())
        } else if let Some(code) = parse_language_code(srt_name) {
            if !args.json {
                eprintln!("   Lang detected from filename: {}", code);
            }
            LangStatus::Detected(code)
        } else {
            let status = detect_cli_subtitle_lang_status(srt_name, "auto");
            match status {
                LangStatus::Detected(code) => {
                    if !args.json {
                        eprintln!("   Lang detected from filename: {}", code);
                    }
                    LangStatus::Detected(code)
                }
                _ => {
                    if !args.json {
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
        SubtitleJob::AlreadyResolved(vec![(srt_file.clone(), lang_status)])
    } else {
        let video_stem = video_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

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
            SubtitleJob::AlreadyResolved(pairs)
        } else {
            if !args.json {
                if let Some(ref target) = args.lang {
                    eprintln!(
                        "ℹ️  No sidecar matches requested lang '{}', trying embedded tracks",
                        target
                    );
                }
            }

            if !args.json {
                eprintln!("🔍 Inspecting video container for embedded subtitle tracks...");
            }

            let mut candidates = Vec::new();
            match inspect_container(video_path) {
                Ok(insp) if !insp.subtitle_streams.is_empty() => {
                    fs::create_dir_all(&args.subtitles_dir)?;

                    let mut seen_extracted_langs = HashSet::new();
                    let mut seen_no_lang = false;

                    let has_explicit_lang_match = args.lang.as_ref().is_some_and(|target| {
                        insp.subtitle_streams.iter().any(|s| {
                            language_matches(s.language.as_deref(), target)
                                || s.language
                                    .as_deref()
                                    .and_then(parse_language_code)
                                    .as_deref()
                                    == Some(target.as_str())
                        })
                    });

                    for stream in &insp.subtitle_streams {
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
                        let stream_lang_opt =
                            stream.language.as_deref().and_then(parse_language_code);

                        if let Some(ref target) = args.lang {
                            let matches = match &stream.language {
                                Some(raw_tag) => {
                                    language_matches(Some(raw_tag), target)
                                        || stream_lang_opt.as_deref() == Some(target.as_str())
                                }
                                None => !has_explicit_lang_match,
                            };
                            if !matches {
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

                        if let Some(ref lang) = lang_opt {
                            if seen_extracted_langs.contains(lang) {
                                if !args.json {
                                    eprintln!(
                                        "ℹ️  Skipping duplicate {} track (stream #{})",
                                        lang, stream.subtitle_index
                                    );
                                }
                                continue;
                            }
                            seen_extracted_langs.insert(lang.clone());
                        } else {
                            if seen_no_lang {
                                if !args.json {
                                    eprintln!(
                                        "ℹ️  Skipping duplicate untagged track (stream #{})",
                                        stream.subtitle_index
                                    );
                                }
                                continue;
                            }
                            seen_no_lang = true;
                        }

                        let srt_fname = match &lang_opt {
                            Some(lang) => format!("{}.{}.srt", video_stem, lang),
                            None => format!("{}.srt", video_stem),
                        };
                        let srt_path = args.subtitles_dir.join(&srt_fname);
                        candidates.push((stream.subtitle_index, lang_opt, srt_path));
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    if !args.json {
                        eprintln!("⚠️  Could not inspect video container: {}", e);
                    }
                }
            }

            if candidates.is_empty() {
                if discovered.is_empty() {
                    if !args.skip_certify {
                        bail!(
                            "❌ No matching .srt file found for '{:?}'. Specify a subtitle file with '--srt <path>' or download one using 'dbsrt fetch-subtitles -q \"<title>\" -l fr'.",
                            video_path
                        );
                    }
                    SubtitleJob::AlreadyResolved(Vec::new())
                } else {
                    let pairs = select_and_validate_subtitle_pairs(
                        &discovered,
                        video_path,
                        args.lang.as_deref(),
                        args.json,
                    )?;
                    SubtitleJob::AlreadyResolved(pairs)
                }
            } else {
                SubtitleJob::NeedsExtraction {
                    candidates,
                    discovered,
                }
            }
        }
    };

    // 3. Detect FPS
    let fps = detect_media_fps(video_path, args.fps);

    // 4. Extract pHash fingerprint (concurrently with subtitle extraction if needed)
    fs::create_dir_all(&args.phash_dir)?;

    let video_name = video_path.file_name().unwrap_or_default().to_string_lossy();
    let estimated_phash = args
        .phash_dir
        .join(format!("{}.{:.1}fps.phash", video_name, fps));
    *cleanup.phash.lock().unwrap() = Some(estimated_phash);

    if !args.json {
        eprintln!(
            "🎬 Extracting pHash frames via ffmpeg (may take a few minutes for long videos)..."
        );
        eprintln!("   Video: {:?}", video_path);
        eprintln!("   FPS detected: {:.3}", fps);
    }

    let hw_backend = match args.hwaccel.to_lowercase().as_str() {
        "vaapi" => dbsrt_core::HwAccel::Vaapi,
        "cuda" => dbsrt_core::HwAccel::Cuda,
        "qsv" => dbsrt_core::HwAccel::Qsv,
        "disabled" | "none" | "cpu" => dbsrt_core::HwAccel::Disabled,
        _ => dbsrt_core::HwAccel::Auto,
    };

    let (pairs, phash_path, hashes, _detected_fps) = match sub_job {
        SubtitleJob::AlreadyResolved(pairs) => {
            let (phash_path, hashes, detected_fps) =
                find_or_build_phash_with_hwaccel(video_path, &args.phash_dir, hw_backend)?;
            (pairs, phash_path, hashes, detected_fps)
        }
        SubtitleJob::NeedsExtraction {
            candidates,
            discovered,
        } => {
            let video_path_sub = video_path.to_path_buf();
            let video_path_phash = video_path.to_path_buf();
            let phash_dir = args.phash_dir.clone();
            let target_lang = args.lang.clone();
            let is_json = args.json;

            let sub_task =
                tokio::task::spawn_blocking(move || -> Result<Vec<(PathBuf, LangStatus)>> {
                    extract_embedded_subtitles_and_select(
                        &video_path_sub,
                        candidates,
                        discovered,
                        target_lang.as_deref(),
                        is_json,
                    )
                });

            let phash_task =
                tokio::task::spawn_blocking(move || -> Result<(PathBuf, Vec<u64>, f64)> {
                    find_or_build_phash_with_hwaccel(&video_path_phash, &phash_dir, hw_backend)
                });

            let sub_future = async {
                sub_task
                    .await
                    .map_err(|e| anyhow!("Subtitle extraction task panicked: {}", e))?
            };
            let phash_future = async {
                phash_task
                    .await
                    .map_err(|e| anyhow!("pHash extraction task panicked: {}", e))?
            };
            let (pairs, (phash_path, hashes, detected_fps)) =
                tokio::try_join!(sub_future, phash_future)?;
            (pairs, phash_path, hashes, detected_fps)
        }
    };

    cleanup.phash_done.store(true, Ordering::SeqCst);
    *cleanup.phash.lock().unwrap() = Some(phash_path.clone());
    let media_hash = compute_media_phash_digest(&hashes);

    if !args.json {
        eprintln!(
            "✓ pHash extraction completed: {} frames computed",
            hashes.len()
        );
        eprintln!("   Media hash: {}", media_hash);
    }

    // 5. Early exit if skip_certify
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

    // 6. Certification & CAS Idempotence
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

    for (srt_path, lang_status) in &pairs {
        let raw_srt_bytes = fs::read(srt_path)?;
        let subtitles_hash = compute_bytes_digest_32(&raw_srt_bytes);

        let srt_fname = srt_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let lang = lang_status.to_lang_code();

        // Validate: reject "auto" or empty language codes
        if lang == "auto" || lang.is_empty() {
            bail!(
                "❌ Could not determine subtitle language for '{}'. Please specify it explicitly via --lang <fr|en|es|...>",
                srt_fname
            );
        }

        if reg.is_already_certified(&media_hash, &subtitles_hash) {
            skipped_count += 1;
            if !args.json {
                eprintln!(
                    "ℹ️  Already certified: '{}' (media_hash={}, sub_hash={}) -> skipping",
                    srt_fname, media_hash, subtitles_hash
                );
            }
            continue;
        }

        // Copy subtitle into subtitles_dir if needed
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
        let sub_frames = build_and_validate_sub_frames_opts(&srt_file, &hashes, fps, true)?;

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
            media_hash.clone(),
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

    // 7. Dynamic Catalog RAM Ingestion (only if new couplings were certified)
    let mut server_ingested = false;

    if certified_count > 0 {
        if !args.json {
            eprintln!("🚀 Triggering dynamic RAM ingestion on dbsrt-server...");
        }
        let ingest_url = format!(
            "{}/v1/catalog/ingest",
            args.server_url.trim_end_matches('/')
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS))
            .build()?;

        let payload = serde_json::json!({
            "phash_path": phash_path.to_string_lossy(),
            "fps": fps
        });

        match client.post(&ingest_url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                server_ingested = true;
                if !args.json {
                    eprintln!(
                        "✓ Dynamic RAM ingestion succeeded on dbsrt-server ({})",
                        ingest_url
                    );
                }
            }
            Ok(resp) => {
                if !args.json {
                    eprintln!(
                        "⚠️  dbsrt-server catalog ingest returned HTTP {}. Coupling remains certified on disk.",
                        resp.status()
                    );
                }
            }
            Err(_) => {
                if !args.json {
                    eprintln!(
                        "⚠️  dbsrt-server offline ({}): .phash extracted and coupling certified to disk, will be loaded on next server startup.",
                        args.server_url
                    );
                }
            }
        }
    } else if !args.json {
        eprintln!("ℹ️  All couplings already certified (idempotent skip). Server ingest skipped.");
    }

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

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("=== MEDIA:ADD INGESTION SUMMARY ===");
        println!("Media Video      : {:?}", video_path);
        println!("Framerate (FPS)  : {:.3}", fps);
        println!("pHash File       : {:?}", phash_path);
        println!("Couplings Added  : {}", certified_count);
        println!("Couplings Skipped: {}", skipped_count);
        println!("Catalog Ingested : {}", server_ingested);
    }

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
}
