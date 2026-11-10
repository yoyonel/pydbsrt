//! Spatiotemporal Multi-Window Recognition Pipeline.
//!
//! Evaluates media across multiple temporal offset windows (e.g. 20%, 50%, 80%)
//! to eliminate false positive collisions caused by shared eyecatches, intros, or static jingles.

use anyhow::Result;
use dbsrt_core::multi_window::{
    aggregate_multi_window_matches, MultiWindowConsensus, MultiWindowPlan,
    MAX_SHIFT_DRIFT_TOLERANCE_MS,
};
use dbsrt_core::{extract_frames_phash, VideoMetadata};
use dbsrt_index::MmapDbphash;
use std::path::Path;
use std::time::Instant;

use crate::cli::OutputFormat;
use crate::commands::recognize::RecognizeArgs;

/// Context criteria for multi-window execution.
pub struct MultiWindowContext<'a> {
    pub media_path: &'a Path,
    pub meta: &'a VideoMetadata,
    pub dbphash: &'a MmapDbphash,
    pub plan: &'a MultiWindowPlan,
}

/// Parameters for rendering a consensus result row.
struct ConsensusRenderRow<'a> {
    consensus: &'a MultiWindowConsensus,
    media_name_search: &'a str,
    timer_in_seconds: f64,
}

/// Executes multi-window recognition against a memory-mapped `.dbphash` catalog.
///
/// # Arguments
/// * `ctx` - Multi-window context criteria ([`MultiWindowContext`]).
/// * `args` - Original CLI recognition parameters.
///
/// # Returns
/// * `Result<()>`
///
/// # Errors
/// * Returns Err if frame extraction or probing fails.
///
/// # Panics
/// * Never panics.
///
/// # Data Flow & Invariants
/// 1. Computes window start offsets along timeline via [`MultiWindowPlan::compute_start_seconds`].
/// 2. For each window, extracts 1s of frames and queries `.dbphash` index.
/// 3. Feeds observations to [`aggregate_multi_window_matches`] to enforce spatiotemporal delta stability.
/// 4. Filters out false collisions (such as shared eyecatches) and outputs ranked consensus.
pub fn execute_multi_window_dbphash(
    ctx: &MultiWindowContext<'_>,
    args: &RecognizeArgs,
) -> Result<()> {
    let start_time = Instant::now();
    let starts = ctx.plan.compute_start_seconds(ctx.meta.duration);
    let nb_frames = (ctx.plan.window_duration_sec * ctx.meta.fps).round() as usize;
    if nb_frames == 0 {
        anyhow::bail!("Number of frames to extract per window must be > 0");
    }

    let mut per_window = Vec::with_capacity(ctx.plan.ratios.len());

    for (w_idx, (&ratio, &start_sec)) in ctx.plan.ratios.iter().zip(starts.iter()).enumerate() {
        let hashes = extract_frames_phash(ctx.media_path, start_sec, nb_frames)?;
        let matches = ctx.dbphash.recognize(&hashes, args.search_distance);
        let win_matches: Vec<(u32, String, usize, usize, i64, f64)> = matches
            .into_iter()
            .filter(|m| m.confidence >= 20.0)
            .map(|m| {
                (
                    m.media_id,
                    m.clean_title,
                    m.matching_cuts,
                    m.matching_sparse,
                    m.estimated_shift_ms,
                    m.confidence,
                )
            })
            .collect();
        per_window.push((w_idx, ratio, start_sec, win_matches));
    }

    let consensus = aggregate_multi_window_matches(
        ctx.plan.ratios.len(),
        per_window,
        MAX_SHIFT_DRIFT_TOLERANCE_MS,
    );
    let timer_in_seconds = start_time.elapsed().as_secs_f64();

    let media_name_search = ctx
        .media_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if consensus.is_empty() {
        if args.output_format == OutputFormat::DataFrame {
            println!("Empty DataFrame\nColumns: [media_found, media_name_search, media_id_match, media_name_match, nb_windows, coherent, timer_in_seconds, confidence]\nIndex: []");
        } else if args.output_format == OutputFormat::Json {
            println!("[]");
        }
        return Ok(());
    }

    let display_matches: &[MultiWindowConsensus] = if args.limit > 0 && args.limit < consensus.len()
    {
        &consensus[..args.limit]
    } else {
        &consensus[..]
    };

    if args.output_format == OutputFormat::Json {
        let rows: Vec<serde_json::Value> = display_matches
            .iter()
            .map(|m| {
                let media_found = m.clean_title.contains(&media_name_search)
                    || media_name_search.contains(&m.clean_title);
                serde_json::json!({
                    "media_found": media_found,
                    "media_name_search": media_name_search,
                    "media_id_match": m.media_id,
                    "media_name_match": m.clean_title,
                    "matching_windows": m.matching_windows,
                    "total_windows": m.total_windows,
                    "is_temporally_coherent": m.is_temporally_coherent,
                    "timer_in_seconds": timer_in_seconds,
                    "confidence": m.consensus_confidence,
                    "consensus_shift_ms": m.consensus_shift_ms,
                    "window_matches": m.window_matches,
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&rows)?);
        return Ok(());
    }

    for m in display_matches {
        let row = ConsensusRenderRow {
            consensus: m,
            media_name_search: &media_name_search,
            timer_in_seconds,
        };
        render_consensus_row(&row, args.output_format);
    }

    Ok(())
}

fn render_consensus_row(row: &ConsensusRenderRow<'_>, output_format: OutputFormat) {
    let m = row.consensus;
    let media_found = m.clean_title.contains(row.media_name_search)
        || row.media_name_search.contains(&m.clean_title);

    match output_format {
        OutputFormat::Csv => {
            println!(
                "{},{},{},{},{}/{},{},{:.16},{:.1}",
                if media_found { "True" } else { "False" },
                row.media_name_search,
                m.clean_title,
                m.media_id,
                m.matching_windows,
                m.total_windows,
                if m.is_temporally_coherent {
                    "True"
                } else {
                    "False"
                },
                row.timer_in_seconds,
                m.consensus_confidence
            );
        }
        OutputFormat::DataFrame => {
            println!(
                "   media_found            media_name_search  media_id_match             media_name_match  windows  coherent ... timer_in_seconds  confidence\n\
                 0         {}  {}               {}  {}               {}/{}     {} ...         {:.6}         {:.1}\n\n\
                 [1 rows x 8 columns]",
                if media_found { "True" } else { "False" },
                row.media_name_search,
                m.media_id,
                m.clean_title,
                m.matching_windows,
                m.total_windows,
                if m.is_temporally_coherent { "True" } else { "False" },
                row.timer_in_seconds,
                m.consensus_confidence
            );
        }
        OutputFormat::Json => {
            // Handled globally in execute_multi_window_dbphash before rendering individual rows
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_consensus_row_csv_and_dataframe() {
        let consensus = MultiWindowConsensus {
            media_id: 42,
            clean_title: "Test Movie".to_string(),
            matching_windows: 3,
            total_windows: 3,
            is_temporally_coherent: true,
            consensus_shift_ms: 100,
            consensus_confidence: 98.5,
            window_matches: vec![],
        };

        let row = ConsensusRenderRow {
            consensus: &consensus,
            media_name_search: "Test Movie 1080p",
            timer_in_seconds: 0.123,
        };

        render_consensus_row(&row, OutputFormat::Csv);
        render_consensus_row(&row, OutputFormat::DataFrame);
        render_consensus_row(&row, OutputFormat::Json);
    }
}
