//! Subtitle auto-synchronization, retargeting, and certification endpoints.

use crate::constants::{DEFAULT_CUT_THRESHOLD, MILLIS_PER_SEC};
use crate::dtos::{
    AutoSyncRequest, AutoSyncResponse, CertifySyncRequest, CertifySyncResponse, RetargetRequest,
    RetargetResponse, SubtitleSource,
};
use crate::services::catalog::{
    collect_subtitle_search_dirs, detect_subtitle_file_language, fetch_or_cache_remote_subtitle,
    find_subtitle_in_catalog, normalize_lang_code,
};
use crate::services::phash::{
    calculate_shift_consensus, compute_media_phash_shift_multi, resolve_or_extract_phash,
};
use crate::services::recognition::identify_media_by_phash_multi_with_reader;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use dbsrt_core::{
    build_and_validate_sub_frames, compute_bytes_digest_32, compute_media_phash_digest,
    detect_cuts, extract_embedded_subtitle, inspect_container, CertifiedSubtitleRecord,
    CouplingRegistry, MediaMetadata, SubtitleFile,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

pub async fn autosync_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AutoSyncRequest>,
) -> Json<AutoSyncResponse> {
    let start = Instant::now();
    let media_path = PathBuf::from(&payload.media_path);
    if !media_path.exists() {
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
            error: Some(format!("Target media file not found: {:?}", media_path)),
        });
    }

    let override_phash_dir = payload.phash_dir.map(PathBuf::from);
    let phash_dirs = state.effective_phash_dirs(override_phash_dir.as_deref());
    let include_sidecars = payload.subtitles_dir.is_none();
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());
    let media_stem = media_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let reader_guard = state.dbphash_reader.load();
    let (matched, target_fps, existing_phash) = identify_media_by_phash_multi_with_reader(
        &media_path,
        &phash_dirs,
        reader_guard.as_deref(),
        state.db_config.as_ref(),
        payload.nb_seconds,
        payload.search_distance,
    );

    let (is_recognized, matched_stem_opt, match_conf) = match matched {
        Some((m_name, conf)) => (true, Some(m_name), conf),
        None => (false, None, 0.0),
    };

    let search_stem = matched_stem_opt.as_deref().unwrap_or(&media_stem);
    let search_dirs = collect_subtitle_search_dirs(&subtitles_dir, &media_path, include_sidecars);
    let found_catalog = search_dirs
        .iter()
        .find_map(|d| find_subtitle_in_catalog(d, search_stem, &payload.language));
    let (mut srt, subtitle_source) = match found_catalog {
        Some(p) => match SubtitleFile::open(&p) {
            Ok(s) => (s, SubtitleSource::Catalog.as_str().to_string()),
            Err(e) => {
                return Json(AutoSyncResponse {
                    success: false,
                    matched_media: matched_stem_opt,
                    language: payload.language.clone(),
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: match_conf,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    source: Some(SubtitleSource::Catalog.as_str().to_string()),
                    error: Some(format!("Failed to parse catalog subtitle {:?}: {:#}", p, e)),
                });
            }
        },
        None => {
            let embedded = inspect_container(&media_path)
                .ok()
                .and_then(|insp| insp.find_by_language(&payload.language).cloned());

            match embedded {
                Some(stream) => {
                    match extract_embedded_subtitle(&media_path, stream.subtitle_index) {
                        Ok(raw_srt) => match SubtitleFile::parse(&raw_srt) {
                            Ok(s) => (s, SubtitleSource::Container.as_str().to_string()),
                            Err(e) => {
                                return Json(AutoSyncResponse {
                                    success: false,
                                    matched_media: matched_stem_opt,
                                    language: payload.language.clone(),
                                    shift_frames: 0,
                                    shift_ms: 0.0,
                                    output_srt_path: String::new(),
                                    confidence_percent: match_conf,
                                    elapsed_seconds: start.elapsed().as_secs_f64(),
                                    source: Some(SubtitleSource::Container.as_str().to_string()),
                                    error: Some(format!(
                                        "Failed to parse embedded subtitle track: {:#}",
                                        e
                                    )),
                                });
                            }
                        },
                        Err(e) => {
                            return Json(AutoSyncResponse {
                                success: false,
                                matched_media: matched_stem_opt,
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: Some(SubtitleSource::Container.as_str().to_string()),
                                error: Some(format!(
                                    "Failed to extract embedded subtitle track: {:#}",
                                    e
                                )),
                            });
                        }
                    }
                }
                None => {
                    let meta = MediaMetadata::parse_from_path(&media_path);
                    let candidates = match state.provider.search(&meta, &payload.language).await {
                        Ok(cands) => cands,
                        Err(e) => {
                            return Json(AutoSyncResponse {
                                success: false,
                                matched_media: matched_stem_opt,
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: Some(SubtitleSource::OpenSubtitles.as_str().to_string()),
                                error: Some(format!(
                                    "Remote provider search failed for '{}' ({}): {:#}",
                                    meta.clean_title, payload.language, e
                                )),
                            });
                        }
                    };

                    let mut downloaded_srt = None;
                    for cand in candidates.iter().take(3) {
                        if let Ok(parsed) =
                            fetch_or_cache_remote_subtitle(&state, &meta, &payload.language, cand)
                                .await
                        {
                            downloaded_srt = Some(parsed);
                            break;
                        }
                    }

                    match downloaded_srt {
                        Some(s) => (s, SubtitleSource::OpenSubtitles.as_str().to_string()),
                        None => {
                            let err_msg = if is_recognized {
                                format!(
                                    "Media recognized as '{}' but no '{}' subtitle found in local catalog {:?}, container tracks, nor remote providers",
                                    matched_stem_opt.as_deref().unwrap_or(""), payload.language, subtitles_dir
                                )
                            } else {
                                format!(
                                    "Media not recognized by perceptual hash fingerprint in indexed libraries ({:?}) and no '{}' subtitle found in local catalog, container tracks, nor remote providers",
                                    phash_dirs, payload.language
                                )
                            };
                            return Json(AutoSyncResponse {
                                success: false,
                                matched_media: matched_stem_opt,
                                language: payload.language.clone(),
                                shift_frames: 0,
                                shift_ms: 0.0,
                                output_srt_path: String::new(),
                                confidence_percent: match_conf,
                                elapsed_seconds: start.elapsed().as_secs_f64(),
                                source: None,
                                error: Some(err_msg),
                            });
                        }
                    }
                }
            }
        }
    };

    let (shift_frames, shift_ms, confidence_percent) = if let Some(ref ref_stem) = matched_stem_opt
    {
        compute_media_phash_shift_multi(
            &media_path,
            &phash_dirs,
            ref_stem,
            match_conf,
            target_fps,
            &existing_phash,
            payload.search_distance,
        )
    } else {
        (0, 0.0, 0.0)
    };

    if shift_ms != 0.0 {
        srt.shift(shift_ms);
    }

    let out_dir = media_path
        .parent()
        .filter(|p| p.is_dir() && !p.to_string_lossy().is_empty())
        .unwrap_or_else(|| Path::new("/tmp"));

    let out_name = format!(
        "{}.{}.synced.srt",
        media_path.file_name().unwrap().to_string_lossy(),
        payload.language
    );
    let out_path = out_dir.join(&out_name);
    let final_path = match srt.save(&out_path) {
        Ok(()) => out_path,
        Err(e) => {
            let fallback_dir = state.cache.base_dir();
            if let Err(mkdir_err) = std::fs::create_dir_all(fallback_dir) {
                return Json(AutoSyncResponse {
                    success: false,
                    matched_media: matched_stem_opt,
                    language: payload.language,
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    source: Some(subtitle_source),
                    error: Some(format!(
                        "Failed to save synced subtitle to {:?} ({:#}) and cache fallback failed ({:#})",
                        out_path, e, mkdir_err
                    )),
                });
            }
            let fallback_path = fallback_dir.join(&out_name);
            match srt.save(&fallback_path) {
                Ok(()) => fallback_path,
                Err(fallback_err) => {
                    return Json(AutoSyncResponse {
                        success: false,
                        matched_media: matched_stem_opt,
                        language: payload.language,
                        shift_frames: 0,
                        shift_ms: 0.0,
                        output_srt_path: String::new(),
                        confidence_percent,
                        elapsed_seconds: start.elapsed().as_secs_f64(),
                        source: Some(subtitle_source),
                        error: Some(format!(
                            "Failed to save synced subtitle to {:?} ({:#}) and fallback to {:?} failed ({:#})",
                            out_path, e, fallback_path, fallback_err
                        )),
                    });
                }
            }
        }
    };

    Json(AutoSyncResponse {
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
    })
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
    let media_name = phash_fname
        .strip_suffix(".phash")
        .unwrap_or(&phash_fname)
        .to_string();

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
