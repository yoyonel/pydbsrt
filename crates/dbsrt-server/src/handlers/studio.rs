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
use std::sync::Arc;

pub async fn studio_ui_handler() -> Html<&'static str> {
    Html(include_str!("../studio.html"))
}

pub async fn couplings_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CouplingsQuery>,
) -> Json<CouplingsOverviewResponse> {
    let summary_only = query.summary.unwrap_or(false);
    let dirs = state.effective_phash_dirs(None);
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

    let mut catalog_phashes = Vec::new();
    let mut seen_files = HashSet::new();
    for dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("phash") {
                    let fname = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if !seen_files.insert(fname.clone()) {
                        continue;
                    }
                    let size_bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    let frame_count = (size_bytes / 8) as usize;
                    let media_name = fname.strip_suffix(".phash").unwrap_or(&fname).to_string();
                    let fps = parse_fps_from_phash_stem(&media_name);
                    let is_certified =
                        certified_names.contains(&media_name) || certified_names.contains(&fname);
                    catalog_phashes.push(StudioPhashEntry {
                        filename: fname,
                        media_name,
                        fps,
                        frame_count,
                        size_bytes,
                        is_certified,
                    });
                }
            }
        }
    }
    catalog_phashes.sort_by(|a, b| {
        b.is_certified
            .cmp(&a.is_certified)
            .then_with(|| a.media_name.cmp(&b.media_name))
    });

    let indexed_phash_files = catalog_phashes.len();

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
