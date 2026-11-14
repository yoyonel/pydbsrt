//! Subtitle auto-synchronization, retargeting, and certification endpoints.

use crate::constants::{
    DEFAULT_AHASH_QUERY_DURATION_SECONDS, DEFAULT_AHASH_START_OFFSET_SECONDS, DEFAULT_BASELINE_FPS,
    DEFAULT_CUT_THRESHOLD, MILLIS_PER_SEC,
};
use crate::dtos::{
    AutoSyncRequest, AutoSyncResponse, CertifySyncRequest, CertifySyncResponse, RetargetRequest,
    RetargetResponse, SubtitleSource,
};
use crate::services::catalog::{
    collect_subtitle_search_dirs, detect_subtitle_file_language, fetch_or_cache_remote_subtitle,
    find_subtitle_in_catalog, is_safe_language_token, normalize_lang_code,
};
use crate::services::phash::{
    calculate_shift_consensus, compute_media_phash_shift_multi, resolve_or_extract_phash,
    ExistingPhashData,
};
use crate::services::recognition::identify_media_by_phash_multi_with_reader;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use dbsrt_core::ahash::{AhashFile, DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE};
use dbsrt_core::{
    build_and_validate_sub_frames, compute_bytes_digest_32, compute_media_phash_digest,
    detect_cuts, extract_ahash, extract_embedded_subtitle, inspect_container,
    AudioExtractionOptions, CertifiedSubtitleRecord, CouplingRegistry, LangStatus, MediaMetadata,
    SubtitleFile,
};
use dbsrt_index::AHashSearchCriteria;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Relays autosync requests to the configured remote endpoint when operating in agent mode;
/// the downloaded subtitle is re-synchronized against the local media.
async fn handle_remote_agent_autosync(
    state: Arc<AppState>,
    payload: AutoSyncRequest,
    start: Instant,
) -> AutoSyncResponse {
    let search_req = crate::dtos::SearchSubtitlesRequest {
        media_path: payload.media_path.clone(),
        language: payload.language.clone(),
        subtitles_dir: payload.subtitles_dir.clone(),
        phash_dir: payload.phash_dir.clone(),
        search_mode: payload
            .search_mode
            .clone()
            .or_else(|| Some("auto".to_string())),
        index_engine: None,
        query_title: None,
        season: None,
        episode: None,
    };
    let search_res =
        crate::handlers::search::search_handler(State(state.clone()), Json(search_req))
            .await
            .0;

    if let Some(cand) = search_res.candidates.into_iter().next() {
        let download_req = crate::dtos::DownloadSubtitleRequest {
            media_path: payload.media_path.clone(),
            candidate_id: cand.candidate_id,
            source: cand.source,
            language: cand.language,
            resync: true,
            shift_ms: cand.shift_ms,
            subtitles_dir: payload.subtitles_dir.clone(),
            phash_dir: payload.phash_dir.clone(),
        };
        let download_res =
            crate::handlers::search::download_handler(State(state), Json(download_req))
                .await
                .0;

        AutoSyncResponse {
            success: download_res.success,
            matched_media: search_res.matched_media,
            language: payload.language,
            shift_frames: download_res.shift_frames,
            shift_ms: download_res.shift_ms,
            output_srt_path: download_res.output_srt_path,
            confidence_percent: search_res.timing.confidence_percent,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            source: Some(download_res.source),
            error: download_res.error,
        }
    } else {
        AutoSyncResponse {
            success: false,
            matched_media: search_res.matched_media,
            language: payload.language,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: search_res.timing.confidence_percent,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            source: None,
            error: Some("No matching subtitle candidates found via remote relay".to_string()),
        }
    }
}

/// Criteria parameters for resolving subtitles across local, container, or remote sources.
struct AutoSyncResolveCriteria<'a> {
    media_path: &'a Path,
    subtitles_dir: &'a Path,
    search_stem: &'a str,
    language: &'a str,
    include_sidecars: bool,
}

/// Synchronously checks local disk catalogs and embedded container subtitle streams.
fn resolve_local_or_container_subtitle(
    media_path: &Path,
    subtitles_dir: &Path,
    search_stem: &str,
    language_and_sidecars: (&str, bool),
) -> Result<Option<(SubtitleFile, String)>, String> {
    let (language, include_sidecars) = language_and_sidecars;
    let search_dirs = collect_subtitle_search_dirs(subtitles_dir, media_path, include_sidecars);
    if let Some(p) = search_dirs
        .iter()
        .find_map(|d| find_subtitle_in_catalog(d, search_stem, language))
    {
        return SubtitleFile::open(&p)
            .map(|s| Some((s, SubtitleSource::Catalog.as_str().to_string())))
            .map_err(|e| format!("Failed to parse catalog subtitle {:?}: {:#}", p, e));
    }

    if let Ok(insp) = inspect_container(media_path) {
        if let Some(stream) = insp.find_by_language(language) {
            let raw_srt = extract_embedded_subtitle(media_path, stream.subtitle_index)
                .map_err(|e| format!("Failed to extract embedded subtitle track: {:#}", e))?;
            let srt = SubtitleFile::parse(&raw_srt)
                .map_err(|e| format!("Failed to parse embedded subtitle track: {:#}", e))?;
            return Ok(Some((srt, SubtitleSource::Container.as_str().to_string())));
        }
    }

    Ok(None)
}

/// Resolves candidate subtitle files across local catalog, embedded container streams, and remote providers.
///
/// # Arguments
/// - `state`: Application state containing providers and caches.
/// - `criteria`: Resolution parameters (media path, stem, language, directory).
///
/// # Returns
/// Parsed `SubtitleFile` and source name string, or formatted error message.
///
/// # Errors
/// Returns `Err` if parsing fails or no suitable subtitle can be found.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// First queries local filesystem sidecars and directory catalogs and embedded tracks inside `spawn_blocking`.
/// Finally attempts remote download via `state.provider.search`.
async fn resolve_autosync_subtitle(
    state: &AppState,
    criteria: &AutoSyncResolveCriteria<'_>,
) -> Result<(SubtitleFile, String), String> {
    let media_path = criteria.media_path.to_path_buf();
    let subtitles_dir = criteria.subtitles_dir.to_path_buf();
    let search_stem = criteria.search_stem.to_string();
    let language = criteria.language.to_string();
    let include_sidecars = criteria.include_sidecars;

    let local_or_container = tokio::task::spawn_blocking(move || {
        resolve_local_or_container_subtitle(
            &media_path,
            &subtitles_dir,
            &search_stem,
            (&language, include_sidecars),
        )
    })
    .await
    .map_err(|e| format!("Subtitle resolution task join error: {e}"))??;

    if let Some(found) = local_or_container {
        return Ok(found);
    }

    let meta = MediaMetadata::parse_from_path(criteria.media_path);
    let candidates = state
        .provider
        .search(&meta, criteria.language)
        .await
        .map_err(|e| {
            format!(
                "Remote provider search failed for '{}' ({}): {:#}",
                meta.clean_title, criteria.language, e
            )
        })?;

    for cand in candidates.iter().take(3) {
        if let Ok(parsed) =
            fetch_or_cache_remote_subtitle(state, &meta, criteria.language, cand).await
        {
            return Ok((parsed, SubtitleSource::OpenSubtitles.as_str().to_string()));
        }
    }

    Err(format!(
        "no '{}' subtitle found in local catalog {:?}, container tracks, nor remote providers",
        criteria.language, criteria.subtitles_dir
    ))
}

/// Saves the synchronized subtitle to disk next to the media or into the cache fallback directory.
///
/// # Arguments
/// - `srt`: The subtitle file with time shifts applied.
/// - `media_path`: Original media path for co-locating the sidecar file.
/// - `language`: Target language code for filename tagging.
/// - `cache_base_dir`: Fallback directory if co-location fails due to read-only paths.
///
/// # Returns
/// Result containing the saved subtitle `PathBuf` or an error description.
///
/// # Errors
/// Returns `Err` if both primary write and cache fallback write fail.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Attempts saving `<media>.<lang>.synced.srt` in the parent directory of the media.
/// If writing fails (e.g. read-only filesystem), creates and writes to `cache_base_dir`.
fn save_synced_subtitle(
    srt: &SubtitleFile,
    media_path: &Path,
    language: &str,
    cache_base_dir: &Path,
) -> Result<PathBuf, String> {
    let out_dir = media_path
        .parent()
        .filter(|p| p.is_dir() && !p.to_string_lossy().is_empty())
        .unwrap_or_else(|| Path::new("/tmp"));

    let out_name = format!(
        "{}.{}.synced.srt",
        media_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("subtitle"),
        language
    );
    let out_path = out_dir.join(&out_name);
    if srt.save(&out_path).is_ok() {
        return Ok(out_path);
    }

    std::fs::create_dir_all(cache_base_dir)
        .map_err(|e| format!("Failed to create cache dir {:?}: {:#}", cache_base_dir, e))?;

    let fallback_path = cache_base_dir.join(&out_name);
    srt.save(&fallback_path)
        .map(|()| fallback_path.clone())
        .map_err(|e| {
            format!(
                "Failed to save subtitle to fallback {:?}: {:#}",
                fallback_path, e
            )
        })
}

/// Validates client inputs before any work: the media must exist and the language must be
/// safe to embed in the output file name.
fn autosync_precheck_error(media_path: &Path, language: &str) -> Option<String> {
    if !media_path.exists() {
        return Some(format!("Target media file not found: {:?}", media_path));
    }
    (!is_safe_language_token(language)).then(|| format!("Invalid language code: {:?}", language))
}

pub async fn autosync_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AutoSyncRequest>,
) -> Json<AutoSyncResponse> {
    let start = Instant::now();
    let media_path = PathBuf::from(&payload.media_path);
    if let Some(error) = autosync_precheck_error(&media_path, &payload.language) {
        return Json(AutoSyncResponse {
            success: false,
            matched_media: None,
            language: payload.language.clone(),
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            source: None,
            error: Some(error),
        });
    }

    // Local identification and resolution first; the remote relay is only a fallback.
    let local = run_local_autosync(&state, payload.clone(), &media_path, start).await;
    if local.success || (state.remote_api.is_none() && !state.is_agent) {
        return Json(local);
    }
    Json(handle_remote_agent_autosync(state, payload, start).await)
}

/// Query start (seconds) used when the primary aHash probe window cannot be extracted
/// (e.g. a short cut shorter than [`DEFAULT_AHASH_START_OFFSET_SECONDS`]).
pub(crate) const AHASH_FALLBACK_QUERY_START_SECONDS: f64 = 0.0;

/// Position of a probed audio window in the target media versus its match in the reference.
///
/// # Data Flow & Invariants
/// Sign convention (same as `calculate_shift_consensus` and `SubtitleFile::shift`):
/// `shift = target_ts - ref_ts`. A reference cue at `ref_ts` is moved to
/// `ref_ts + shift` on the target timeline. A target window extracted at
/// `query_start_sec` that matches the reference at `ref_offset_sec` therefore yields
/// `shift_ms = (query_start_sec - ref_offset_sec) * 1000`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AcousticAlignment {
    /// Start (seconds, target timeline) of the window that was actually extracted.
    pub query_start_sec: f64,
    /// Offset (seconds, reference timeline) where that window matched in the catalog.
    pub ref_offset_sec: f64,
}

impl AcousticAlignment {
    /// Subtitle shift (ms) to apply to the reference subtitle to fit the target media.
    ///
    /// # Returns
    /// `(query_start_sec - ref_offset_sec) * 1000.0`: `0.0` for an already aligned media
    /// probed at the offset where it matches, negative for a cut taken later in the reference.
    ///
    /// # Panics
    /// Guaranteed panic-free.
    pub(crate) fn shift_ms(&self) -> f64 {
        (self.query_start_sec - self.ref_offset_sec) * MILLIS_PER_SEC
    }
}

/// Extracts the aHash probe window, reporting the start offset that was really used.
///
/// # Arguments
/// - `media_path`: Media file (or stream URL) to fingerprint.
///
/// # Returns
/// `Some((ahash, query_start_sec))` where `query_start_sec` is
/// [`DEFAULT_AHASH_START_OFFSET_SECONDS`] when the primary window yielded at least
/// [`dbsrt_index::MIN_QUERY_FRAMES`] hashes, or [`AHASH_FALLBACK_QUERY_START_SECONDS`] when
/// the fallback window was used; `None` if both extractions fail or produce too few frames.
///
/// # Errors
/// None. Extraction errors are mapped to `None`.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Blocking (spawns FFmpeg). The returned start is always the one passed to FFmpeg, so a
/// caller can compute an exact `target_ts - ref_ts` shift.
pub(crate) fn extract_ahash_query_window(media_path: &Path) -> Option<(AhashFile, f64)> {
    let primary = AudioExtractionOptions {
        sample_rate: DEFAULT_SAMPLE_RATE,
        frame_size: DEFAULT_FRAME_SIZE,
        hop_size: DEFAULT_HOP_SIZE,
        start_sec: DEFAULT_AHASH_START_OFFSET_SECONDS,
        duration_sec: Some(DEFAULT_AHASH_QUERY_DURATION_SECONDS),
        custom_ffmpeg: None,
        audio_stream: None,
    };
    if let Ok(file) = extract_ahash(media_path, &primary) {
        if file.hashes.len() >= dbsrt_index::MIN_QUERY_FRAMES {
            return Some((file, primary.start_sec));
        }
    }
    let fallback = AudioExtractionOptions {
        start_sec: AHASH_FALLBACK_QUERY_START_SECONDS,
        ..primary
    };
    extract_ahash(media_path, &fallback)
        .ok()
        .filter(|file| file.hashes.len() >= dbsrt_index::MIN_QUERY_FRAMES)
        .map(|file| (file, fallback.start_sec))
}

/// Relays extracted audio hashes to `POST {remote_api}/v1/search/ahash` for autosync.
async fn relay_ahash_autosync(
    state: &AppState,
    hashes: &[u64],
    audio_fps: f64,
    query_start_sec: f64,
) -> Option<(String, f64, AcousticAlignment)> {
    let remote_url = state.remote_api.as_deref()?;
    let (resp, _) = crate::handlers::search::ahash::relay_remote_ahash_search(
        state, remote_url, hashes, audio_fps,
    )
    .await?;
    let best = resp
        .best_match
        .or_else(|| resp.candidates.into_iter().next())?;
    Some((
        best.media_name,
        best.score * 100.0,
        AcousticAlignment {
            query_start_sec,
            ref_offset_sec: best.offset_sec,
        },
    ))
}

async fn identify_media_by_ahash_autosync(
    state: &Arc<AppState>,
    media_path: &Path,
) -> Option<(String, f64, AcousticAlignment)> {
    if state.ahash_catalog.load().entries.is_empty() && state.remote_api.is_none() {
        return None;
    }
    let _permit = state.heavy_task_semaphore.acquire().await.ok();
    let p = media_path.to_path_buf();
    let (extracted, query_start_sec) =
        tokio::task::spawn_blocking(move || extract_ahash_query_window(&p))
            .await
            .ok()
            .flatten()?;

    if extracted.hashes.len() < dbsrt_index::MIN_QUERY_FRAMES {
        return None;
    }

    let audio_fps = extracted.header.audio_fps();
    let catalog = state.ahash_catalog.load_full();
    if !catalog.entries.is_empty() && (!state.is_agent || state.remote_api.is_none()) {
        let hashes = extracted.hashes.clone();
        let matches = tokio::task::spawn_blocking(move || {
            catalog.search(&AHashSearchCriteria::new(&hashes, audio_fps))
        })
        .await
        .unwrap_or_default();

        if let Some(best) = matches.first() {
            return Some((
                best.media_name.clone(),
                best.score * 100.0,
                AcousticAlignment {
                    query_start_sec,
                    ref_offset_sec: best.offset_sec,
                },
            ));
        }
    }

    relay_ahash_autosync(state, &extracted.hashes, audio_fps, query_start_sec).await
}

struct AutoSyncIdentificationContext<'a> {
    state: &'a Arc<AppState>,
    payload: &'a AutoSyncRequest,
    media_path: &'a Path,
    phash_dirs: &'a [PathBuf],
    eff_search_distance: u32,
}

/// Dispatches media identification across aHash and pHash pipelines based on search mode.
///
/// The fourth tuple element is the exact acoustic subtitle shift in ms
/// (`query_start_sec - ref_offset_sec`), `None` for the pHash pipeline.
async fn identify_media_for_autosync(
    ctx: AutoSyncIdentificationContext<'_>,
) -> (
    Option<(String, f64)>,
    f64,
    Option<ExistingPhashData>,
    Option<f64>,
) {
    let mode = ctx.payload.search_mode.as_deref().unwrap_or("auto");
    if mode == "ahash" {
        let ahash_match = identify_media_by_ahash_autosync(ctx.state, ctx.media_path).await;
        return match ahash_match {
            Some((name, conf, alignment)) => (
                Some((name, conf)),
                DEFAULT_BASELINE_FPS,
                None,
                Some(alignment.shift_ms()),
            ),
            None => (None, DEFAULT_BASELINE_FPS, None, None),
        };
    }

    let reader_guard = ctx.state.dbphash_reader.load_full();
    let media_path = ctx.media_path.to_path_buf();
    let phash_dirs = ctx.phash_dirs.to_vec();
    let db_config = ctx.state.db_config.clone();
    let nb_seconds = ctx.payload.nb_seconds;
    let eff_search_distance = ctx.eff_search_distance;
    let (m, fps, ex) = tokio::task::spawn_blocking(move || {
        let reader_ref = reader_guard.as_ref().map(|arc| arc.as_ref());
        identify_media_by_phash_multi_with_reader(
            &media_path,
            &phash_dirs,
            reader_ref,
            db_config.as_ref(),
            nb_seconds,
            eff_search_distance,
        )
    })
    .await
    .unwrap_or((None, DEFAULT_BASELINE_FPS, None));

    if m.is_none() && mode != "phash" {
        if let Some((name, conf, alignment)) =
            identify_media_by_ahash_autosync(ctx.state, ctx.media_path).await
        {
            return (Some((name, conf)), fps, ex, Some(alignment.shift_ms()));
        }
    }

    (m, fps, ex, None)
}

/// Enforces the architectural subtitle shift policy by source (`Catalog` vs `Container` / `OpenSubtitles`).
///
/// # Arguments
/// * `subtitle_source` - Resolved source identifier (`"catalog"`, `"container"`, or `"opensubtitles"`).
/// * `candidate_shift` - Computed `(shift_frames, shift_ms, confidence_percent)` from reference media alignment.
/// * `match_conf` - Recognition confidence percentage (`0.0..=100.0`).
///
/// # Returns
/// `(shift_frames, shift_ms, confidence_percent)` where `shift_frames == 0` and `shift_ms == 0.0`
/// whenever `subtitle_source != SubtitleSource::Catalog.as_str()`.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Per GEMINI.md Axe 5 and B-06:
/// - `SubtitleSource::Catalog`: Reference subtitle coupled to reference timeline -> apply `candidate_shift`.
/// - `SubtitleSource::Container`: Embedded track already synchronized to target container -> force `0.0 ms`.
/// - `SubtitleSource::OpenSubtitles`: Uncertified external subtitle -> force `0.0 ms`.
pub(crate) fn enforce_subtitle_source_shift_policy(
    subtitle_source: &str,
    candidate_shift: (i64, f64, f64),
    match_conf: f64,
) -> (i64, f64, f64) {
    if subtitle_source == SubtitleSource::Catalog.as_str() {
        candidate_shift
    } else {
        (0, 0.0, match_conf)
    }
}

/// Saves a synchronized subtitle file on the blocking thread pool.
async fn save_autosync_srt_off_runtime(
    srt: SubtitleFile,
    media_path: &Path,
    language: &str,
    cache_dir: &Path,
) -> Result<PathBuf, String> {
    let mp = media_path.to_path_buf();
    let lang = language.to_string();
    let cdir = cache_dir.to_path_buf();
    tokio::task::spawn_blocking(move || save_synced_subtitle(&srt, &mp, &lang, &cdir))
        .await
        .map_err(|e| format!("Save subtitle task join error: {e}"))
        .and_then(|r| r)
}

fn format_autosync_resolve_error(
    is_recognized: bool,
    matched_stem_opt: Option<&str>,
    phash_dirs: &[PathBuf],
    err_msg: &str,
) -> String {
    if is_recognized {
        format!(
            "Media recognized as '{}' but {}",
            matched_stem_opt.unwrap_or(""),
            err_msg
        )
    } else {
        format!(
            "Media not recognized by perceptual hash fingerprint in indexed libraries ({:?}) and {}",
            phash_dirs, err_msg
        )
    }
}

/// Identifies the media against local indexes, resolves a subtitle from the local catalog,
/// container tracks or providers, shifts it, and saves it next to the media.
async fn run_local_autosync(
    state: &Arc<AppState>,
    payload: AutoSyncRequest,
    media_path: &Path,
    start: Instant,
) -> AutoSyncResponse {
    let media_path = media_path.to_path_buf();
    let override_phash_dir = payload.phash_dir.as_ref().map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());
    let include_sidecars = payload.subtitles_dir.is_none();
    let subtitles_dir = payload
        .subtitles_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let media_stem = media_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let eff_search_distance = if payload.search_distance > 0 {
        payload.search_distance
    } else {
        crate::constants::DEFAULT_SEARCH_DISTANCE
    };

    let (matched, target_fps, existing_phash, ahash_shift_ms) =
        identify_media_for_autosync(AutoSyncIdentificationContext {
            state,
            payload: &payload,
            media_path: &media_path,
            phash_dirs: &phash_dirs,
            eff_search_distance,
        })
        .await;

    let (is_recognized, matched_stem_opt, match_conf) = match matched {
        Some((m_name, conf)) => (true, Some(m_name), conf),
        None => (false, None, 0.0),
    };

    let search_stem = matched_stem_opt.as_deref().unwrap_or(media_stem.as_str());
    let resolve_criteria = AutoSyncResolveCriteria {
        media_path: &media_path,
        subtitles_dir: &subtitles_dir,
        search_stem,
        language: &payload.language,
        include_sidecars,
    };
    let (mut srt, subtitle_source) = match resolve_autosync_subtitle(state, &resolve_criteria).await
    {
        Ok(res) => res,
        Err(err_msg) => {
            let formatted_err = format_autosync_resolve_error(
                is_recognized,
                matched_stem_opt.as_deref(),
                &phash_dirs,
                &err_msg,
            );
            return AutoSyncResponse {
                success: false,
                matched_media: matched_stem_opt,
                language: payload.language,
                shift_frames: 0,
                shift_ms: 0.0,
                output_srt_path: String::new(),
                confidence_percent: match_conf,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                source: None,
                error: Some(formatted_err),
            };
        }
    };

    let raw_shift = if subtitle_source != SubtitleSource::Catalog.as_str() {
        (0, 0.0, match_conf)
    } else if let Some(shift_ms) = ahash_shift_ms {
        (
            (shift_ms / MILLIS_PER_SEC * target_fps).round() as i64,
            shift_ms,
            match_conf,
        )
    } else if let Some(ref ref_stem) = matched_stem_opt {
        let (mp, pdirs, rstem) = (media_path.clone(), phash_dirs.clone(), ref_stem.clone());
        tokio::task::spawn_blocking(move || {
            compute_media_phash_shift_multi(
                &mp,
                &pdirs,
                &rstem,
                match_conf,
                target_fps,
                &existing_phash,
                eff_search_distance,
            )
        })
        .await
        .unwrap_or((0, 0.0, match_conf))
    } else {
        (0, 0.0, 0.0)
    };
    let (shift_frames, shift_ms, confidence_percent) =
        enforce_subtitle_source_shift_policy(&subtitle_source, raw_shift, match_conf);

    if shift_ms != 0.0 {
        srt.shift(shift_ms);
    }

    let final_path = match save_autosync_srt_off_runtime(
        srt,
        &media_path,
        &payload.language,
        state.cache.base_dir(),
    )
    .await
    {
        Ok(p) => p,
        Err(err_msg) => {
            return AutoSyncResponse {
                success: false,
                matched_media: matched_stem_opt,
                language: payload.language,
                shift_frames: 0,
                shift_ms: 0.0,
                output_srt_path: String::new(),
                confidence_percent,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                source: Some(subtitle_source),
                error: Some(err_msg),
            };
        }
    };

    AutoSyncResponse {
        success: true,
        matched_media: matched_stem_opt,
        language: payload.language,
        shift_frames,
        shift_ms,
        output_srt_path: final_path.to_string_lossy().to_string(),
        confidence_percent,
        elapsed_seconds: start.elapsed().as_secs_f64(),
        source: Some(subtitle_source),
        error: None,
    }
}

pub async fn retarget_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RetargetRequest>,
) -> Json<RetargetResponse> {
    let target_media = PathBuf::from(&payload.target_media);
    let ref_subtitles = PathBuf::from(&payload.ref_subtitles);

    if !target_media.exists() {
        return Json(RetargetResponse {
            success: false,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: 0.0,
            error: Some(format!("Target media file not found: {:?}", target_media)),
        });
    }
    if !ref_subtitles.exists() {
        return Json(RetargetResponse {
            success: false,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: 0.0,
            error: Some(format!(
                "Subtitle file not found on disk: {:?}",
                ref_subtitles
            )),
        });
    }

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let start = Instant::now();

    let (target_phash_file, target_hashes, target_fps) =
        match resolve_or_extract_phash(&target_media, &phash_dir) {
            Ok(res) => res,
            Err(e) => {
                return Json(RetargetResponse {
                    success: false,
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: 0.0,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    error: Some(format!("Failed to extract target media phash: {:#}", e)),
                });
            }
        };

    let (_ref_phash_file, ref_hashes, _ref_fps) = if let Some(ref ref_media_str) = payload.ref_media
    {
        let ref_media = PathBuf::from(ref_media_str);
        match resolve_or_extract_phash(&ref_media, &phash_dir) {
            Ok(res) => res,
            Err(e) => {
                return Json(RetargetResponse {
                    success: false,
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: 0.0,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    error: Some(format!("Failed to extract reference media phash: {:#}", e)),
                });
            }
        }
    } else {
        (target_phash_file.clone(), target_hashes.clone(), target_fps)
    };

    let target_cuts = detect_cuts(&target_hashes, DEFAULT_CUT_THRESHOLD);
    let ref_cuts = detect_cuts(&ref_hashes, DEFAULT_CUT_THRESHOLD);

    let (shift_frames, confidence_percent) =
        calculate_shift_consensus(&target_cuts, &ref_cuts, payload.search_distance, 0.0);

    let shift_ms = (shift_frames as f64 / target_fps) * MILLIS_PER_SEC;

    let mut srt = match SubtitleFile::open(&ref_subtitles) {
        Ok(s) => s,
        Err(e) => {
            return Json(RetargetResponse {
                success: false,
                shift_frames: 0,
                shift_ms: 0.0,
                output_srt_path: String::new(),
                confidence_percent: 0.0,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                error: Some(format!("Failed to parse subtitle file: {:#}", e)),
            });
        }
    };
    srt.shift(shift_ms);

    let out_name = format!(
        "{}.synced.srt",
        target_media.file_name().unwrap().to_string_lossy()
    );
    let out_path = phash_dir.join(&out_name);
    if let Err(e) = srt.save(&out_path) {
        return Json(RetargetResponse {
            success: false,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent: 0.0,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            error: Some(format!("Failed to save shifted subtitle file: {:#}", e)),
        });
    }

    Json(RetargetResponse {
        success: true,
        shift_frames,
        shift_ms,
        output_srt_path: out_path.to_string_lossy().to_string(),
        confidence_percent,
        elapsed_seconds: start.elapsed().as_secs_f64(),
        error: None,
    })
}

pub async fn certify_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CertifySyncRequest>,
) -> Json<CertifySyncResponse> {
    let media_path = PathBuf::from(&payload.media_path);
    let subtitle_path = PathBuf::from(&payload.subtitle_path);
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let err_resp = |msg: String| {
        Json(CertifySyncResponse {
            success: false,
            media_name: String::new(),
            media_hash: String::new(),
            phash_filename: String::new(),
            subtitles_hash: String::new(),
            certified_srt_path: String::new(),
            cue_count: 0,
            sub_frames_count: 0,
            postgres_imported: false,
            registry_path: String::new(),
            error: Some(msg),
        })
    };

    if !media_path.exists() {
        return err_resp(format!("Media file not found: {:?}", media_path));
    }
    if !subtitle_path.exists() {
        return err_resp(format!("Subtitle file not found: {:?}", subtitle_path));
    }

    let (phash_file, hashes, fps) = match resolve_or_extract_phash(&media_path, &phash_dir) {
        Ok(res) => res,
        Err(e) => {
            return err_resp(format!(
                "Failed to resolve/extract media .phash in {:?}: {:#}",
                phash_dir, e
            ));
        }
    };

    let srt_file = match SubtitleFile::open(&subtitle_path) {
        Ok(s) => s,
        Err(e) => {
            return err_resp(format!("Failed to parse subtitle file: {:#}", e));
        }
    };

    let (sub_frames, fps) = match build_and_validate_sub_frames(&srt_file, &hashes, fps) {
        Ok(sf) => (sf, fps),
        Err(e) if (fps - 25.0).abs() < 1e-3 => {
            if let Ok(sf) = build_and_validate_sub_frames(&srt_file, &hashes, 23.976) {
                (sf, 23.976)
            } else {
                return err_resp(format!("Subtitle frame validation failed: {:#}", e));
            }
        }
        Err(e) => {
            return err_resp(format!("Subtitle frame validation failed: {:#}", e));
        }
    };

    let lang = normalize_lang_code(&payload.language);
    let lang = if lang == "auto" || lang == "all" || lang.is_empty() {
        subtitle_path
            .file_name()
            .and_then(|n| detect_subtitle_file_language(&n.to_string_lossy()))
            .unwrap_or_else(|| "und".to_string())
    } else {
        lang
    };

    let _ = std::fs::create_dir_all(&subtitles_dir);
    let phash_fname = phash_file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let media_name = dbsrt_core::phash_media_name(&phash_file);

    let dest_srt_path = if subtitle_path.starts_with(&subtitles_dir) {
        subtitle_path.clone()
    } else {
        let base_stem = media_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| media_name.clone());
        let cert_filename = format!("{}.{}.srt", base_stem, lang);
        let dest = subtitles_dir.join(&cert_filename);
        if dest != subtitle_path {
            if let Err(e) = std::fs::copy(&subtitle_path, &dest) {
                return err_resp(format!(
                    "Failed to copy certified subtitle to {:?}: {:#}",
                    dest, e
                ));
            }
        }
        dest
    };

    let raw_srt_bytes = match std::fs::read(&dest_srt_path) {
        Ok(b) => b,
        Err(e) => {
            return err_resp(format!("Failed to read certified subtitle bytes: {:#}", e));
        }
    };

    let media_hash = compute_media_phash_digest(&hashes);
    let subtitles_hash = compute_bytes_digest_32(&raw_srt_bytes);
    let rel_srt_name = dest_srt_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let reg_path = CouplingRegistry::default_path(&subtitles_dir);
    let mut reg = match CouplingRegistry::load_or_default(&reg_path) {
        Ok(r) => r,
        Err(e) => {
            return err_resp(format!("Failed to load coupling registry: {:#}", e));
        }
    };

    reg.upsert_coupling(
        media_hash.clone(),
        media_name.clone(),
        phash_fname.clone(),
        fps,
        hashes.len(),
        CertifiedSubtitleRecord {
            name: rel_srt_name.clone(),
            srt_path: rel_srt_name,
            lang: lang.clone(),
            lang_status: LangStatus::Detected(lang.clone()),
            subtitles_hash: subtitles_hash.clone(),
            certified: true,
            cue_count: srt_file.cues.len(),
            sub_frames: sub_frames.clone(),
        },
    );

    if let Err(e) = reg.save(&reg_path) {
        return err_resp(format!("Failed to save coupling registry: {:#}", e));
    }

    if let Ok(mut lock) = state.coupling_registry.write() {
        *lock = reg;
    }

    let mut postgres_imported = false;
    if let Some(ref cfg) = state.db_config {
        if cfg
            .with_client(|client| {
                client.init_schema()?;
                client.import_subtitle_coupling(&phash_file, &dest_srt_path, &lang, payload.strict)
            })
            .is_ok()
        {
            postgres_imported = true;
        }
    }

    Json(CertifySyncResponse {
        success: true,
        media_name,
        media_hash,
        phash_filename: phash_fname,
        subtitles_hash,
        certified_srt_path: dest_srt_path.to_string_lossy().to_string(),
        cue_count: srt_file.cues.len(),
        sub_frames_count: sub_frames.len(),
        postgres_imported,
        registry_path: reg_path.to_string_lossy().to_string(),
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};

    #[tokio::test]
    async fn test_agent_autosync_resolves_local_sidecar_before_relay() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let srt_fixture = repo_root.join("tests/data/big_buck_bunny_trailer_480p.en.srt");
        let work_dir =
            std::env::temp_dir().join(format!("test_agent_autosync_{}", std::process::id()));
        std::fs::create_dir_all(&work_dir).unwrap();
        let media = work_dir.join("clip.mkv");
        std::fs::write(&media, b"not a real video").unwrap();
        std::fs::copy(&srt_fixture, work_dir.join("clip.en.srt")).unwrap();

        let mut state = AppState::new(
            work_dir.clone(),
            work_dir.clone(),
            Arc::new(OpenSubtitlesProvider::new(None)),
            SubtitleCache::default(),
        );
        state.is_agent = true;
        let payload = AutoSyncRequest {
            media_path: media.to_string_lossy().to_string(),
            language: "en".to_string(),
            phash_dir: None,
            subtitles_dir: None,
            search_mode: None,
            search_distance: 0,
            nb_seconds: 1.0,
        };
        let res = autosync_handler(State(Arc::new(state)), Json(payload))
            .await
            .0;
        let _ = std::fs::remove_dir_all(&work_dir);

        // Agent mode must not bypass the local sidecar in favour of the remote relay.
        assert!(res.success, "local autosync failed: {:?}", res.error);
        assert_eq!(
            res.source.as_deref(),
            Some(SubtitleSource::Catalog.as_str())
        );
    }

    // ------------------------------------------------------------------
    // Acoustic shift sign convention: shift = target_ts - ref_ts
    // ------------------------------------------------------------------

    /// Tolerance of one acoustic frame (hop 64 @ 5512 Hz => ~11.6 ms), ms.
    const ONE_AUDIO_FRAME_MS: f64 = 1_000.0 * 64.0 / 5_512.0;
    /// Probe start of a full, already aligned film (= `DEFAULT_AHASH_START_OFFSET_SECONDS`).
    const ALIGNED_QUERY_START_SEC: f64 = 60.0;
    /// Reference offset where the cut / the aligned probe matches.
    const REF_OFFSET_SEC: f64 = 60.0;
    /// Expected shift of a cut probed from its beginning and matching 60 s into the reference.
    const EXPECTED_CUT_SHIFT_MS: f64 = -60_000.0;
    /// Cue straddling the 60 s mark of the reference subtitle.
    const REF_SRT: &str = "1\n00:01:02,000 --> 00:01:04,000\nHello\n\n";

    #[test]
    fn test_aligned_media_probed_at_60s_matching_at_60s_has_zero_shift() {
        let alignment = AcousticAlignment {
            query_start_sec: ALIGNED_QUERY_START_SEC,
            ref_offset_sec: REF_OFFSET_SEC,
        };
        assert_eq!(alignment.shift_ms(), 0.0);
    }

    #[test]
    fn test_cut_probed_at_fallback_0s_matching_at_60s_shifts_cue_back_by_60s() {
        let alignment = AcousticAlignment {
            query_start_sec: AHASH_FALLBACK_QUERY_START_SECONDS,
            ref_offset_sec: REF_OFFSET_SEC,
        };
        assert_eq!(alignment.shift_ms(), EXPECTED_CUT_SHIFT_MS);

        let mut srt = SubtitleFile::parse(REF_SRT).unwrap();
        assert_eq!(srt.cues[0].start_ms, 62_000);
        srt.shift(alignment.shift_ms());
        assert_eq!(srt.cues[0].start_ms, 2_000);
        assert_eq!(srt.cues[0].end_ms, 4_000);
        assert!(srt.to_string().contains("00:00:02,000 --> 00:00:04,000"));
    }

    // ------------------------------------------------------------------
    // End-to-end (real FFmpeg + real aHash extraction + real catalog search)
    // ------------------------------------------------------------------

    /// Mono 16-bit PCM WAV sample rate (equals the aHash rate: no resampling noise).
    const WAV_SAMPLE_RATE: u32 = 5_512;
    /// Duration of the synthetic full-length "film", seconds.
    const FULL_AUDIO_SECONDS: usize = 90;
    /// Start and length of the short cut extracted from the film, seconds.
    const CUT_START_SECONDS: &str = "60";
    const CUT_LENGTH_SECONDS: &str = "15";
    /// xorshift64 seed and shifts for the deterministic noise track.
    const NOISE_SEED: u64 = 0x2545_F491_4F6C_DD1D;
    const XORSHIFT_A: u32 = 13;
    const XORSHIFT_B: u32 = 7;
    const XORSHIFT_C: u32 = 17;
    /// Right shift mapping the top bits of the PRNG to a signed 16-bit sample.
    const SAMPLE_SHIFT: u32 = 48;
    /// Size of a canonical PCM WAV header, bytes.
    const WAV_HEADER_BYTES: u32 = 44;
    const WAV_BITS_PER_SAMPLE: u16 = 16;
    const WAV_BYTES_PER_SAMPLE: u32 = 2;

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Writes deterministic white noise as mono 16-bit PCM WAV.
    fn write_noise_wav(path: &Path, seconds: usize) {
        let mut state = NOISE_SEED;
        let sample_count = WAV_SAMPLE_RATE as usize * seconds;
        let mut pcm = Vec::with_capacity(sample_count * WAV_BYTES_PER_SAMPLE as usize);
        for _ in 0..sample_count {
            state ^= state << XORSHIFT_A;
            state ^= state >> XORSHIFT_B;
            state ^= state << XORSHIFT_C;
            pcm.extend_from_slice(&((state >> SAMPLE_SHIFT) as i16).to_le_bytes());
        }
        let data_len = pcm.len() as u32;
        let mut wav = Vec::with_capacity(pcm.len() + WAV_HEADER_BYTES as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(data_len + WAV_HEADER_BYTES - 8).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&WAV_SAMPLE_RATE.to_le_bytes());
        wav.extend_from_slice(&(WAV_SAMPLE_RATE * WAV_BYTES_PER_SAMPLE).to_le_bytes());
        wav.extend_from_slice(&(WAV_BYTES_PER_SAMPLE as u16).to_le_bytes());
        wav.extend_from_slice(&WAV_BITS_PER_SAMPLE.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&pcm);
        std::fs::write(path, wav).unwrap();
    }

    /// Builds a state whose aHash catalog holds the dense fingerprint of `full_wav`.
    fn state_with_reference_catalog(root: &Path, full_wav: &Path) -> Arc<AppState> {
        let cat_dir = root.join("ahash_catalog");
        std::fs::create_dir_all(&cat_dir).unwrap();
        let opts = AudioExtractionOptions {
            sample_rate: DEFAULT_SAMPLE_RATE,
            frame_size: DEFAULT_FRAME_SIZE,
            hop_size: DEFAULT_HOP_SIZE,
            start_sec: 0.0,
            duration_sec: None,
            custom_ffmpeg: None,
            audio_stream: None,
        };
        let reference = extract_ahash(full_wav, &opts).unwrap();
        AhashFile::write_to_path(
            &cat_dir.join("ref_full.86.1fps.ahash"),
            &reference.header,
            &reference.hashes,
        )
        .unwrap();
        Arc::new(AppState::with_options(
            root.to_path_buf(),
            root.to_path_buf(),
            Arc::new(OpenSubtitlesProvider::new(None)),
            crate::state::AppStateInitOptions {
                dbphash_path: None,
                extra_phash_dirs: Vec::new(),
                ahash_dir: Some(cat_dir),
            },
        ))
    }

    #[tokio::test]
    async fn test_e2e_identify_aligned_film_and_short_cut_propagate_real_query_start() {
        if !ffmpeg_available() {
            eprintln!("SKIPPED: ffmpeg not available (end-to-end aHash autosync test)");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let full_wav = dir.path().join("film.wav");
        let cut_wav = dir.path().join("cut.wav");
        write_noise_wav(&full_wav, FULL_AUDIO_SECONDS);
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-ss",
                CUT_START_SECONDS,
                "-t",
                CUT_LENGTH_SECONDS,
                "-i",
            ])
            .arg(&full_wav)
            .args(["-c", "copy"])
            .arg(&cut_wav)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg cut failed");
        let state = state_with_reference_catalog(dir.path(), &full_wav);

        // Full film: the 60 s probe window exists -> query_start 60.0, match at ~60 s -> ~0 ms.
        let (name, _, aligned) = identify_media_by_ahash_autosync(&state, &full_wav)
            .await
            .expect("aligned film must be recognized");
        println!("aligned film: {aligned:?} shift_ms={}", aligned.shift_ms());
        assert_eq!(name, "ref_full");
        assert_eq!(aligned.query_start_sec, ALIGNED_QUERY_START_SEC);
        assert!(
            aligned.shift_ms().abs() <= ONE_AUDIO_FRAME_MS,
            "aligned film must not be shifted by +60 s: {}",
            aligned.shift_ms()
        );

        // 15 s cut: the 60 s probe is past EOF -> fallback 0.0 s, match at ~60 s -> ~ -60 000 ms.
        let (name, _, cut) = identify_media_by_ahash_autosync(&state, &cut_wav)
            .await
            .expect("cut must be recognized");
        println!("cut: {cut:?} shift_ms={}", cut.shift_ms());
        assert_eq!(name, "ref_full");
        assert_eq!(cut.query_start_sec, AHASH_FALLBACK_QUERY_START_SECONDS);
        assert!(
            (cut.shift_ms() - EXPECTED_CUT_SHIFT_MS).abs() <= ONE_AUDIO_FRAME_MS,
            "cut shift must be ~ -60 000 ms: {}",
            cut.shift_ms()
        );

        let mut srt = SubtitleFile::parse(REF_SRT).unwrap();
        srt.shift(cut.shift_ms());
        assert!((srt.cues[0].start_ms - 2_000).abs() <= ONE_AUDIO_FRAME_MS.ceil() as i64);
    }

    #[test]
    fn test_enforce_subtitle_source_shift_policy_zero_for_container_and_opensubtitles() {
        let candidate = (-1440, -60_000.0, 96.5);

        let catalog_res =
            enforce_subtitle_source_shift_policy(SubtitleSource::Catalog.as_str(), candidate, 96.5);
        assert_eq!(catalog_res, (-1440, -60_000.0, 96.5));

        let container_res = enforce_subtitle_source_shift_policy(
            SubtitleSource::Container.as_str(),
            candidate,
            96.5,
        );
        assert_eq!(container_res, (0, 0.0, 96.5));

        let opensub_res = enforce_subtitle_source_shift_policy(
            SubtitleSource::OpenSubtitles.as_str(),
            candidate,
            96.5,
        );
        assert_eq!(opensub_res, (0, 0.0, 96.5));
    }

    #[test]
    fn test_extract_ahash_query_window_falls_back_when_60s_tail_shorter_than_min_frames() {
        if !ffmpeg_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let full_wav = dir.path().join("film.wav");
        let clip_60_05s = dir.path().join("clip_60_05s.wav");
        write_noise_wav(&full_wav, FULL_AUDIO_SECONDS);
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-ss", "0", "-t", "60.05", "-i"])
            .arg(&full_wav)
            .args(["-c", "copy"])
            .arg(&clip_60_05s)
            .status()
            .unwrap();
        assert!(status.success());

        let (extracted, query_start_sec) =
            extract_ahash_query_window(&clip_60_05s).expect("must fall back to 0.0s window");
        assert_eq!(query_start_sec, AHASH_FALLBACK_QUERY_START_SECONDS);
        assert!(extracted.hashes.len() >= dbsrt_index::MIN_QUERY_FRAMES);
    }
}
