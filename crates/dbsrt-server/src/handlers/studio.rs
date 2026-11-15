//! Web Backoffice Studio endpoints for certified couplings, minimap visualization, and sub-frames inspection.

use crate::dtos::{
    CouplingsOverviewResponse, CouplingsQuery, PostgresTableCounts, StudioMediaCoupling,
    StudioPhashEntry, SubFramesPageResponse, SubFramesQuery,
};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::Json;
use dbsrt_core::{CertifiedMediaCoupling, CouplingRegistry, SubFrameAnchor};
use dbsrt_index::parse_fps_from_phash_stem;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Byte width of a single 64-bit DCT-II perceptual hash frame in `.phash` files.
const BYTES_PER_PHASH_FRAME: u64 = 8;

/// Fixed-point scaling factor used by `DbphashMediaEntry::fps_x1000`.
const FPS_X1000_SCALE: f64 = 1000.0;

/// Canonical sibling directory name storing raw `.phash` binaries alongside `catalog.dbphash`.
const RAW_PHASH_SUBDIR: &str = "__PHASH__";

/// File extension of raw perceptual hash fingerprint streams.
const PHASH_FILE_EXT: &str = "phash";

/// Serves the embedded single-page `dbsrt Studio` HTML backoffice.
///
/// # Arguments
/// None.
///
/// # Returns
/// Static UTF-8 HTML document wrapping `studio.html`.
///
/// # Errors
/// Infallible handler.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Returns the compile-time embedded `studio.html` payload without disk I/O.
pub async fn studio_ui_handler() -> Html<&'static str> {
    Html(include_str!("../studio.html"))
}

/// Resolves candidate `.phash` directories for Studio catalog listing without loading `MmapLibrary`.
///
/// # Arguments
/// * `state` - Shared `AppState` holding configured `phash_dir`, `dbphash_path`, and `subtitles_dir`.
///
/// # Returns
/// Deduplicated vector of existing `.phash` directories, including sibling `__PHASH__` folders
/// adjacent to `catalog.dbphash` or `subtitles_dir` when `--phash-dir` points to `empty_phash`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Starts from `state.effective_phash_dirs(None)` and appends sibling `__PHASH__` directories
/// strictly for lightweight `read_dir` metadata enumeration (zero RAM MIH allocation).
fn studio_phash_scan_dirs(state: &AppState) -> Vec<PathBuf> {
    let mut dirs = state.effective_phash_dirs(None);
    let db_sibling = state
        .dbphash_path()
        .and_then(|p| p.parent().map(|d| d.join(RAW_PHASH_SUBDIR)));
    let srt_sibling = state
        .subtitles_dir
        .parent()
        .map(|d| d.join(RAW_PHASH_SUBDIR));

    for cand in [db_sibling, srt_sibling].into_iter().flatten() {
        if cand.is_dir() && !dirs.contains(&cand) {
            dirs.push(cand);
        }
    }
    dirs
}

/// Builds a [`StudioPhashEntry`] from a single filesystem path if it is a `.phash` file.
///
/// # Arguments
/// * `path` - Filesystem entry path to inspect.
/// * `certified_names` - Lookup set of certified `media_name` and `phash_filename` identifiers.
///
/// # Returns
/// `Some(StudioPhashEntry)` when `path` ends with `.phash`, otherwise `None`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads inode size via `std::fs::metadata` without mapping file contents into memory.
fn build_phash_entry_from_path(
    path: &Path,
    certified_names: &HashSet<String>,
) -> Option<StudioPhashEntry> {
    if path.extension().and_then(|e| e.to_str()) != Some(PHASH_FILE_EXT) {
        return None;
    }
    let filename = path.file_name()?.to_string_lossy().to_string();
    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let frame_count = (size_bytes / BYTES_PER_PHASH_FRAME) as usize;
    let media_name = dbsrt_core::phash_media_name(path);
    let fps = parse_fps_from_phash_stem(&media_name);
    let is_certified = certified_names.contains(&media_name) || certified_names.contains(&filename);
    Some(StudioPhashEntry {
        filename,
        media_name,
        fps,
        frame_count,
        size_bytes,
        is_certified,
    })
}

/// Appends `.phash` entries from a single directory into `out` when not yet seen in `seen_files`.
///
/// # Arguments
/// * `dir` - Directory path to scan via `std::fs::read_dir`.
/// * `certified_names` - Lookup set of certified media names and filenames.
/// * `seen_files` - Mutable deduplication set of `.phash` filenames.
/// * `out` - Output vector accumulating [`StudioPhashEntry`] records.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Skips unreadable directories and non-`.phash` entries without mapping files into memory.
fn append_dir_phash_entries(
    dir: &Path,
    certified_names: &HashSet<String>,
    seen_files: &mut HashSet<String>,
    out: &mut Vec<StudioPhashEntry>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for item in entries
        .flatten()
        .filter_map(|e| build_phash_entry_from_path(&e.path(), certified_names))
    {
        if seen_files.insert(item.filename.clone()) {
            out.push(item);
        }
    }
}

/// Synthesizes [`StudioPhashEntry`] items from `MmapDbphash` when no loose `.phash` files exist.
///
/// # Arguments
/// * `reader` - Memory-mapped `catalog.dbphash` reader.
/// * `certified_names` - Lookup set of certified media names and filenames.
/// * `out` - Output vector accumulating synthesized [`StudioPhashEntry`] records.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads zero-copy media table descriptors and string table titles from `MmapDbphash`.
fn append_dbphash_fallback_entries(
    reader: &dbsrt_index::MmapDbphash,
    certified_names: &HashSet<String>,
    out: &mut Vec<StudioPhashEntry>,
) {
    for entry in reader.media_entries() {
        let media_name = reader.get_title(entry).to_string();
        let filename = format!("{media_name}.{PHASH_FILE_EXT}");
        let frame_u64 = entry.known_frame_count().unwrap_or(0);
        let is_certified =
            certified_names.contains(&media_name) || certified_names.contains(&filename);
        out.push(StudioPhashEntry {
            filename,
            media_name,
            fps: f64::from(entry.fps_x1000) / FPS_X1000_SCALE,
            frame_count: frame_u64 as usize,
            size_bytes: frame_u64 * BYTES_PER_PHASH_FRAME,
            is_certified,
        });
    }
}

/// Populates Studio `.phash` catalog entries from disk metadata or falls back to `MmapDbphash`.
///
/// # Arguments
/// * `state` - Shared `AppState` providing `dbphash_reader` and directory paths.
/// * `certified_names` - Set of certified media names and `.phash` filenames.
///
/// # Returns
/// Tuple `(indexed_phash_files, catalog_phashes)` sorted certified-first then lexicographically.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. Enumerates `.phash` files across [`studio_phash_scan_dirs`] deduplicated by filename.
/// 2. If no loose `.phash` files exist on disk (e.g. appliance mode with only `catalog.dbphash`),
///    synthesizes entries directly from `MmapDbphash::media_entries()`.
fn collect_studio_catalog_phashes(
    state: &AppState,
    certified_names: &HashSet<String>,
) -> (usize, Vec<StudioPhashEntry>) {
    let mut catalog_phashes = Vec::new();
    let mut seen_files = HashSet::new();

    for dir in studio_phash_scan_dirs(state) {
        append_dir_phash_entries(&dir, certified_names, &mut seen_files, &mut catalog_phashes);
    }

    let reader_guard = state.dbphash_reader.load();
    let base_dbphash_count = reader_guard
        .as_ref()
        .map(|r| r.media_entries().len())
        .unwrap_or(0);

    if catalog_phashes.is_empty() {
        if let Some(ref reader) = *reader_guard {
            append_dbphash_fallback_entries(reader, certified_names, &mut catalog_phashes);
        }
    }

    catalog_phashes.sort_by(|a, b| {
        b.is_certified
            .cmp(&a.is_certified)
            .then_with(|| a.media_name.cmp(&b.media_name))
    });

    let indexed_phash_files = catalog_phashes.len().max(base_dbphash_count);
    (indexed_phash_files, catalog_phashes)
}

/// Returns the Studio relational overview (`GET /v1/couplings`), combining PostgreSQL counts,
/// certified couplings, and the `.phash` catalog inventory.
///
/// # Arguments
/// * `state` - Shared daemon `AppState`.
/// * `query` - Optional `summary=true` flag omitting bulky `sub_frames` vectors from initial payload.
///
/// # Returns
/// JSON [`CouplingsOverviewResponse`] with PostgreSQL status, certified couplings, and `.phash` list.
///
/// # Errors
/// Degrades gracefully to Zero-DB JSON mode if PostgreSQL is offline or unconfigured.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Reads `CouplingRegistry`, optionally enriches with PostgreSQL table counts, and resolves the
/// `.phash` catalog via [`collect_studio_catalog_phashes`] even when `--phash-dir` uses `empty_phash`.
pub async fn couplings_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CouplingsQuery>,
) -> Json<CouplingsOverviewResponse> {
    let summary_only = query.summary.unwrap_or(false);
    let reg_path = CouplingRegistry::default_path(&state.subtitles_dir);
    let reg = state
        .coupling_registry
        .read()
        .map(|r| r.clone())
        .unwrap_or_default();

    let mut postgres_connected = false;
    let mut postgres_counts = None;
    let mut pg_media_names = HashSet::new();

    if let Some(ref cfg) = state.db_config {
        if let Ok((counts_opt, subs)) = cfg.with_client(|client| {
            let counts = client.get_table_counts().ok();
            let subs = client
                .get_subtitles_for_media(None, None, None)
                .unwrap_or_default();
            Ok((counts, subs))
        }) {
            postgres_connected = true;
            if let Some((m, f, s, sf)) = counts_opt {
                postgres_counts = Some(PostgresTableCounts {
                    medias_count: m,
                    frames_count: f,
                    subtitles_count: s,
                    sub_frames_count: sf,
                });
            }
            for sub in subs {
                pg_media_names.insert(sub.media_name);
            }
        }
    }

    let couplings: Vec<StudioMediaCoupling> = reg
        .couplings
        .into_iter()
        .map(
            |CertifiedMediaCoupling {
                 media_hash,
                 media_name,
                 phash_filename,
                 fps,
                 frame_count,
                 mut subtitles,
             }| {
                let in_postgres = pg_media_names.contains(&media_name)
                    || pg_media_names.contains(&phash_filename);
                if summary_only {
                    for s in &mut subtitles {
                        s.sub_frames.clear();
                    }
                }
                StudioMediaCoupling {
                    media_hash,
                    media_name,
                    phash_filename,
                    fps,
                    frame_count,
                    in_postgres,
                    subtitles,
                }
            },
        )
        .collect();

    let certified_names: HashSet<String> = couplings
        .iter()
        .flat_map(|c| [c.media_name.clone(), c.phash_filename.clone()])
        .collect();

    let (indexed_phash_files, catalog_phashes) =
        collect_studio_catalog_phashes(&state, &certified_names);

    Json(CouplingsOverviewResponse {
        postgres_connected,
        postgres_counts,
        local_registry_path: reg_path.to_string_lossy().to_string(),
        indexed_phash_files,
        couplings,
        catalog_phashes,
    })
}

pub async fn sub_frames_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SubFramesQuery>,
) -> Result<Json<SubFramesPageResponse>, (StatusCode, String)> {
    let reg = state
        .coupling_registry
        .read()
        .map(|r| r.clone())
        .unwrap_or_default();

    let media = reg
        .couplings
        .into_iter()
        .find(|c| c.media_hash == query.media_hash || c.media_name == query.media_hash)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Media '{}' not found", query.media_hash),
            )
        })?;

    let sub = if let Some(ref sh) = query.subtitles_hash {
        media
            .subtitles
            .into_iter()
            .find(|s| s.subtitles_hash == *sh || s.lang.eq_ignore_ascii_case(sh))
    } else {
        media.subtitles.into_iter().next()
    }
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "Subtitle track not found for media".to_string(),
        )
    })?;

    let total_cues = sub.sub_frames.len();
    let step = (total_cues / 180).max(1);
    let minimap_ticks: Vec<i32> = sub
        .sub_frames
        .iter()
        .step_by(step)
        .map(|sf| sf.start_frame_offset)
        .collect();

    let q_norm = query
        .q
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());

    let filtered: Vec<SubFrameAnchor> = match q_norm {
        Some(ref q) => sub
            .sub_frames
            .iter()
            .filter(|sf| {
                let hex_str = sf
                    .anchor_phash
                    .map(|h| format!("0x{:016x}", h))
                    .unwrap_or_default();
                format!("#{}", sf.index).contains(q)
                    || sf.start_frame_offset.to_string().contains(q)
                    || sf.end_frame_offset.to_string().contains(q)
                    || hex_str.contains(q)
                    || sf.text.to_lowercase().contains(q)
            })
            .cloned()
            .collect(),
        None => sub.sub_frames.clone(),
    };

    let filtered_cues = filtered.len();
    let page_size = query.page_size.unwrap_or(50).clamp(1, 500);
    let total_pages = filtered_cues.div_ceil(page_size).max(1);

    let mut resolved_cue_index = query.cue_index;
    if let Some(tf) = query.target_frame {
        if let Some(closest) = filtered
            .iter()
            .min_by_key(|sf| (sf.start_frame_offset - tf).abs())
        {
            resolved_cue_index = Some(closest.index);
        }
    }

    let page = if let Some(ci) = resolved_cue_index {
        if let Some(pos) = filtered.iter().position(|sf| sf.index == ci) {
            (pos / page_size) + 1
        } else {
            query.page.unwrap_or(1).clamp(1, total_pages)
        }
    } else {
        query.page.unwrap_or(1).clamp(1, total_pages)
    };

    let start_idx = (page - 1) * page_size;
    let items: Vec<SubFrameAnchor> = filtered
        .iter()
        .skip(start_idx)
        .take(page_size)
        .cloned()
        .collect();

    let window_start_frame = items.first().map(|sf| sf.start_frame_offset).unwrap_or(0);
    let window_end_frame = items
        .last()
        .map(|sf| sf.end_frame_offset)
        .unwrap_or(media.frame_count as i32);

    let selected_cue = resolved_cue_index
        .and_then(|ci| filtered.iter().find(|sf| sf.index == ci).cloned())
        .or_else(|| items.first().cloned());

    Ok(Json(SubFramesPageResponse {
        media_hash: media.media_hash,
        media_name: media.media_name,
        fps: media.fps,
        frame_count: media.frame_count,
        subtitles_hash: sub.subtitles_hash,
        subtitle_name: sub.name,
        lang: sub.lang,
        srt_path: sub.srt_path,
        total_cues,
        filtered_cues,
        page,
        page_size,
        total_pages,
        window_start_frame,
        window_end_frame,
        minimap_ticks,
        selected_cue,
        items,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};

    #[tokio::test]
    async fn test_couplings_handler_discovers_sibling_phash_dir_when_empty_phash_used() {
        let tmp_root =
            std::env::temp_dir().join(format!("dbsrt_studio_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_root);
        let empty_phash = tmp_root.join("run").join("empty_phash");
        let raw_phash = tmp_root.join("data").join("__PHASH__");
        let srt_dir = tmp_root.join("data").join("SRT");
        std::fs::create_dir_all(&empty_phash).expect("mkdir empty_phash");
        std::fs::create_dir_all(&raw_phash).expect("mkdir raw_phash");
        std::fs::create_dir_all(&srt_dir).expect("mkdir srt_dir");

        let sample = raw_phash.join("Andor.S01E01.1080p.phash");
        std::fs::write(&sample, vec![0_u8; 64]).expect("write phash");

        let state = Arc::new(AppState::new(
            empty_phash,
            srt_dir,
            Arc::new(OpenSubtitlesProvider::new(None)),
            SubtitleCache::default(),
        ));
        let Json(resp) = couplings_handler(
            State(state),
            Query(CouplingsQuery {
                summary: Some(true),
            }),
        )
        .await;
        let _ = std::fs::remove_dir_all(&tmp_root);

        assert!(resp.indexed_phash_files >= 1);
        assert!(resp
            .catalog_phashes
            .iter()
            .any(|it| it.filename == "Andor.S01E01.1080p.phash" && it.frame_count == 8));
    }
}
