//! Library core for dbsrt-server HTTP API.

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use dbsrt_core::{
    detect_cuts, extract_frames_phash, probe_video, read_phash_file, write_phash_file, CutItem,
    SubtitleFile,
};
use dbsrt_index::{BkItem, BkTree, MmapLibrary};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub phash_dir: PathBuf,
    pub subtitles_dir: PathBuf,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct AutoSyncRequest {
    pub media_path: String,
    #[serde(default = "default_lang")]
    pub language: String,
    pub phash_dir: Option<String>,
    pub subtitles_dir: Option<String>,
    #[serde(default)]
    pub search_distance: u32,
    #[serde(default = "default_extract_duration")]
    pub nb_seconds: f64,
}

fn default_lang() -> String {
    "en".to_string()
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AutoSyncResponse {
    pub success: bool,
    pub matched_media: Option<String>,
    pub language: String,
    pub shift_frames: i64,
    pub shift_ms: f64,
    pub output_srt_path: String,
    pub confidence_percent: f64,
    pub elapsed_seconds: f64,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub service: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RecognizeRequest {
    pub media_path: String,
    #[serde(default)]
    pub search_distance: u32,
    #[serde(default = "default_extract_duration")]
    pub nb_seconds: f64,
    pub phash_dir: Option<String>,
}

fn default_extract_duration() -> f64 {
    1.0
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RecognizeResponse {
    pub found: bool,
    pub media_name_search: String,
    pub media_match: Option<String>,
    pub nb_offsets_match: usize,
    pub confidence: f64,
    pub timer_seconds: f64,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RetargetRequest {
    pub target_media: String,
    pub ref_subtitles: String,
    pub ref_media: Option<String>,
    pub phash_dir: Option<String>,
    #[serde(default)]
    pub search_distance: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RetargetResponse {
    pub success: bool,
    pub shift_frames: i64,
    pub shift_ms: f64,
    pub output_srt_path: String,
    pub confidence_percent: f64,
    pub elapsed_seconds: f64,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ScanRequest {
    pub query_media: String,
    pub phash_dir: Option<String>,
    #[serde(default)]
    pub search_distance: u32,
    #[serde(default = "default_scan_limit")]
    pub limit: usize,
}

fn default_scan_limit() -> usize {
    5
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ScanMatchItem {
    pub media_name: String,
    pub matching_frames: usize,
    pub confidence_percent: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ScanResponse {
    pub total_files: usize,
    pub total_frames: usize,
    pub elapsed_seconds: f64,
    pub top_matches: Vec<ScanMatchItem>,
}

/// Builds the Axum router for dbsrt-server.
pub fn create_router(phash_dir: PathBuf, subtitles_dir: PathBuf) -> Router {
    let state = Arc::new(AppState {
        phash_dir,
        subtitles_dir,
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/v1/health", get(health_handler))
        .route("/v1/recognize", post(recognize_handler))
        .route("/v1/retarget", post(retarget_handler))
        .route("/v1/autosync", post(autosync_handler))
        .route("/v1/scan", post(scan_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        service: "dbsrt-server".to_string(),
    })
}

pub async fn recognize_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RecognizeRequest>,
) -> Result<Json<RecognizeResponse>, (StatusCode, String)> {
    let media_path = PathBuf::from(&payload.media_path);
    if !media_path.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Media file not found: {:?}", media_path),
        ));
    }

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let start = Instant::now();
    let meta =
        probe_video(&media_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (payload.nb_seconds * meta.fps).round() as usize;

    let hashes = extract_frames_phash(&media_path, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let library = MmapLibrary::from_directory(&phash_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let matches = library.recognize(&hashes, payload.search_distance);
    let timer_seconds = start.elapsed().as_secs_f64();

    let media_name_search = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if let Some(top) = matches.first() {
        Ok(Json(RecognizeResponse {
            found: true,
            media_name_search,
            media_match: Some(top.media_name.clone()),
            nb_offsets_match: top.nb_offsets_match,
            confidence: top.confidence,
            timer_seconds,
        }))
    } else {
        Ok(Json(RecognizeResponse {
            found: false,
            media_name_search,
            media_match: None,
            nb_offsets_match: 0,
            confidence: 0.0,
            timer_seconds,
        }))
    }
}

pub fn resolve_or_extract_phash(
    media_file: &Path,
    phash_dir: &Path,
) -> Result<(PathBuf, Vec<u64>, f64)> {
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if phash_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(phash_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.contains(&stem) && name.ends_with(".phash") {
                        let hashes = read_phash_file(&p)?;
                        let mut fps = if let Some(fps_match) = name.strip_suffix("fps.phash") {
                            let parts: Vec<&str> = fps_match.split('.').collect();
                            if parts.len() >= 2 {
                                let last_two = format!(
                                    "{}.{}",
                                    parts[parts.len() - 2],
                                    parts[parts.len() - 1]
                                );
                                if let Ok(v) = last_two.parse::<f64>() {
                                    v
                                } else {
                                    parts
                                        .last()
                                        .and_then(|p| p.parse::<f64>().ok())
                                        .unwrap_or(25.0)
                                }
                            } else {
                                parts
                                    .last()
                                    .and_then(|p| p.parse::<f64>().ok())
                                    .unwrap_or(25.0)
                            }
                        } else {
                            probe_video(media_file).map(|m| m.fps).unwrap_or(25.0)
                        };
                        if fps <= 0.0 || fps.is_nan() {
                            fps = 25.0;
                        }
                        return Ok((p, hashes, fps));
                    }
                }
            }
        }
    }

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

    // 1. Resolve target phash
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

    // 2. Resolve reference phash (either from ref_media or find in phash_dir)
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

    // 3. Detect cuts and calculate shift consensus
    let target_cuts = detect_cuts(&target_hashes, 16);
    let ref_cuts = detect_cuts(&ref_hashes, 16);

    let (shift_frames, confidence_percent) =
        calculate_shift_consensus(&target_cuts, &ref_cuts, payload.search_distance, 0.0);

    let shift_ms = (shift_frames as f64 / target_fps) * 1000.0;

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

/// Calculates the optimal consensus frame shift and confidence percentage using
/// BK-Tree candidate voting, Lowe's dominance ratio, and cut consensus support.
pub fn calculate_shift_consensus(
    target_cuts: &[CutItem],
    ref_cuts: &[CutItem],
    search_distance: u32,
    fallback_confidence: f64,
) -> (i64, f64) {
    if ref_cuts.is_empty() || target_cuts.is_empty() {
        return (0, fallback_confidence);
    }

    let mut target_tree = BkTree::new();
    for cut in target_cuts {
        target_tree.insert(BkItem {
            phash: cut.phash,
            offset: cut.offset,
        });
    }

    let mut diff_counter: HashMap<i64, usize> = HashMap::new();
    for ref_cut in ref_cuts {
        let matches = target_tree.search(ref_cut.phash, search_distance);
        for m in matches {
            let diff = m.offset as i64 - ref_cut.offset as i64;
            *diff_counter.entry(diff).or_default() += 1;
        }
    }

    let mut sorted_diffs: Vec<(i64, usize)> = diff_counter.into_iter().collect();
    sorted_diffs.sort_by_key(|a| std::cmp::Reverse(a.1));

    if let Some(&(shift_frames, count)) = sorted_diffs.first() {
        let second_count = sorted_diffs.get(1).map(|(_, c)| *c).unwrap_or(0);
        let dominance = if count > 0 {
            1.0 - (second_count as f64 / count as f64)
        } else {
            0.0
        };
        let support = (count as f64 / ref_cuts.len() as f64).min(1.0);
        let confidence_percent = (dominance * support * 100.0).clamp(0.0, 100.0);
        (shift_frames, confidence_percent)
    } else {
        (0, fallback_confidence)
    }
}

pub fn find_subtitle_in_catalog(
    subtitles_dir: &Path,
    media_stem: &str,
    language: &str,
) -> Option<PathBuf> {
    if !subtitles_dir.exists() {
        return None;
    }
    let lang = language.to_lowercase();
    let entries = std::fs::read_dir(subtitles_dir).ok()?;
    let mut candidates: Vec<(i32, PathBuf)> = Vec::new();

    let clean_stem = media_stem
        .strip_suffix(".phash")
        .unwrap_or(media_stem)
        .strip_suffix(".mp4")
        .unwrap_or(media_stem)
        .strip_suffix(".webm")
        .unwrap_or(media_stem)
        .strip_suffix(".mkv")
        .unwrap_or(media_stem);
    let stem_lower = clean_stem.to_lowercase();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("srt") {
            continue;
        }
        let fname = path.file_name()?.to_string_lossy().to_lowercase();

        let has_stem = fname.contains(&stem_lower)
            || stem_lower.contains(&fname.trim_end_matches(".srt").to_string())
            || (stem_lower.len() > 10 && fname.contains(&stem_lower[..10]));
        if !has_stem {
            continue;
        }

        // Check language match
        let is_exact_lang = fname.ends_with(&format!(".{}.srt", lang))
            || fname.ends_with(&format!("_{}.srt", lang))
            || fname.contains(&format!(".{}.", lang));

        if is_exact_lang {
            return Some(path);
        }

        if lang == "auto" || lang == "all" || fname.contains(&lang) {
            candidates.push((10, path));
        } else {
            candidates.push((1, path));
        }
    }

    candidates.sort_by_key(|c| std::cmp::Reverse(c.0));
    candidates.first().map(|c| c.1.clone())
}

pub fn read_ref_phash_for_media(phash_dir: &Path, media_name: &str) -> Result<Vec<u64>> {
    let clean_name = media_name.strip_suffix(".phash").unwrap_or(media_name);
    let entries = std::fs::read_dir(phash_dir)?;
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if (name.contains(clean_name) || clean_name.contains(name.trim_end_matches(".phash")))
                && name.ends_with(".phash")
            {
                return read_phash_file(&p);
            }
        }
    }
    anyhow::bail!("Reference phash not found for media '{}'", media_name);
}

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
            error: Some(format!("Target media file not found: {:?}", media_path)),
        });
    }

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());
    let subtitles_dir = payload
        .subtitles_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.subtitles_dir.clone());

    // 1. Resolve or extract target media phash
    let (_target_phash_file, target_hashes, target_fps) =
        match resolve_or_extract_phash(&media_path, &phash_dir) {
            Ok(res) => res,
            Err(e) => {
                return Json(AutoSyncResponse {
                    success: false,
                    matched_media: None,
                    language: payload.language.clone(),
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: 0.0,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    error: Some(format!("Failed to extract target media phash: {:#}", e)),
                });
            }
        };

    // 2. Identify media in index library
    let library = match MmapLibrary::from_directory(&phash_dir) {
        Ok(lib) => lib,
        Err(e) => {
            return Json(AutoSyncResponse {
                success: false,
                matched_media: None,
                language: payload.language.clone(),
                shift_frames: 0,
                shift_ms: 0.0,
                output_srt_path: String::new(),
                confidence_percent: 0.0,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                error: Some(format!("Failed to load mmap library: {:#}", e)),
            });
        }
    };

    let sample_len = ((payload.nb_seconds * target_fps).round() as usize).min(target_hashes.len());
    let sample_start = if target_hashes.len() > sample_len * 2 {
        target_hashes.len() / 4
    } else {
        0
    };
    let query_sample = &target_hashes[sample_start..sample_start + sample_len];
    let matches = library.recognize(query_sample, payload.search_distance);

    let (matched_stem, match_conf) = if let Some(m) = matches.first() {
        (m.media_name.clone(), m.confidence * 100.0)
    } else {
        let stem = media_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        (stem, 80.0)
    };

    // 3. Find reference subtitle in catalog for requested language
    let ref_srt_path =
        match find_subtitle_in_catalog(&subtitles_dir, &matched_stem, &payload.language) {
            Some(p) => p,
            None => {
                return Json(AutoSyncResponse {
                    success: false,
                    matched_media: Some(matched_stem.clone()),
                    language: payload.language.clone(),
                    shift_frames: 0,
                    shift_ms: 0.0,
                    output_srt_path: String::new(),
                    confidence_percent: match_conf,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                    error: Some(format!(
                        "Media recognized as '{}' but no '{}' subtitle found in {:?}",
                        matched_stem, payload.language, subtitles_dir
                    )),
                });
            }
        };

    // 4. Calculate shift:
    let target_cuts = detect_cuts(&target_hashes, 16);
    let ref_hashes = if let Ok(ref_h) = read_ref_phash_for_media(&phash_dir, &matched_stem) {
        ref_h
    } else {
        target_hashes.clone()
    };
    let ref_cuts = detect_cuts(&ref_hashes, 16);

    let (shift_frames, confidence_percent) =
        calculate_shift_consensus(&target_cuts, &ref_cuts, payload.search_distance, match_conf);

    let shift_ms = (shift_frames as f64 / target_fps) * 1000.0;

    // 5. Shift & Save Subtitle
    let mut srt = match SubtitleFile::open(&ref_srt_path) {
        Ok(s) => s,
        Err(e) => {
            return Json(AutoSyncResponse {
                success: false,
                matched_media: Some(matched_stem.clone()),
                language: payload.language.clone(),
                shift_frames: 0,
                shift_ms: 0.0,
                output_srt_path: String::new(),
                confidence_percent,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                error: Some(format!(
                    "Failed to parse subtitle {:?}: {:#}",
                    ref_srt_path, e
                )),
            });
        }
    };
    srt.shift(shift_ms);

    let out_name = format!(
        "{}.{}.synced.srt",
        media_path.file_name().unwrap().to_string_lossy(),
        payload.language
    );
    let out_path = phash_dir.join(&out_name);
    if let Err(e) = srt.save(&out_path) {
        return Json(AutoSyncResponse {
            success: false,
            matched_media: Some(matched_stem),
            language: payload.language,
            shift_frames: 0,
            shift_ms: 0.0,
            output_srt_path: String::new(),
            confidence_percent,
            elapsed_seconds: start.elapsed().as_secs_f64(),
            error: Some(format!("Failed to save synced subtitle: {:#}", e)),
        });
    }

    Json(AutoSyncResponse {
        success: true,
        matched_media: Some(matched_stem),
        language: payload.language,
        shift_frames,
        shift_ms,
        output_srt_path: out_path.to_string_lossy().to_string(),
        confidence_percent,
        elapsed_seconds: start.elapsed().as_secs_f64(),
        error: None,
    })
}

pub async fn scan_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ScanRequest>,
) -> Result<Json<ScanResponse>, (StatusCode, String)> {
    let query_media = PathBuf::from(&payload.query_media);
    if !query_media.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Query media not found: {:?}", query_media),
        ));
    }

    let phash_dir = payload
        .phash_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.phash_dir.clone());

    let start = Instant::now();
    let meta = probe_video(&query_media)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (1.0 * meta.fps).round() as usize;

    let query_hashes = extract_frames_phash(&query_media, start_sec, nb_frames)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let library = MmapLibrary::from_directory(&phash_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total_files = library.file_count();
    let total_frames = library.total_frames();

    let matches = library.recognize(&query_hashes, payload.search_distance);
    let elapsed_seconds = start.elapsed().as_secs_f64();

    let top_matches = matches
        .into_iter()
        .take(payload.limit)
        .map(|m| ScanMatchItem {
            media_name: m.media_name,
            matching_frames: m.nb_offsets_match,
            confidence_percent: m.confidence * 100.0,
        })
        .collect();

    Ok(Json(ScanResponse {
        total_files,
        total_frames,
        elapsed_seconds,
        top_matches,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_shift_consensus_empty() {
        let (shift, conf) = calculate_shift_consensus(&[], &[], 0, 42.0);
        assert_eq!(shift, 0);
        assert_eq!(conf, 42.0);
    }

    #[test]
    fn test_calculate_shift_consensus_perfect() {
        let cuts = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 25,
            },
        ];
        let (shift, conf) = calculate_shift_consensus(&cuts, &cuts, 0, 0.0);
        assert_eq!(shift, 0);
        assert!((conf - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_calculate_shift_consensus_with_shift() {
        let target = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 20,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 35,
            },
        ];
        let reference = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 25,
            },
        ];
        // target offset - ref offset = 10
        let (shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        assert_eq!(shift, 10);
        assert!((conf - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_calculate_shift_consensus_tie() {
        // Two conflicting candidates with 1 vote each
        let target = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 20,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 50,
            },
        ];
        let reference = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            }, // diff = +10
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 10,
            }, // diff = +40
        ];
        let (_shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        // Tie between diff 10 and diff 40: dominance is 1.0 - (1/1) = 0.0
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn test_calculate_shift_consensus_dominance() {
        let mut target = Vec::new();
        let mut reference = Vec::new();
        for i in 0..10 {
            let h = 0x1000_0000_0000_0000 + i;
            target.push(CutItem {
                phash: h,
                offset: 100 + i as usize,
            });
            reference.push(CutItem {
                phash: h,
                offset: 100 + i as usize,
            });
        }
        // Add 1 noise cut cross-matching
        let h_noise = 0x9999_9999_9999_9999;
        target.push(CutItem {
            phash: h_noise,
            offset: 200,
        });
        reference.push(CutItem {
            phash: h_noise,
            offset: 250,
        }); // diff = -50 (1 vote)

        let (shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        assert_eq!(shift, 0);
        // 11 votes total, 10 votes for 0, 1 vote for -50
        // Dominance: 1 - 1/10 = 0.90
        // Support: 10 / 11 = 0.90909
        // Conf: 0.90 * (10/11) * 100 = 81.818%
        assert!(conf > 80.0 && conf < 85.0);
    }
}
