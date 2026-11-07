use crate::dbphash::MmapDbphash;
use anyhow::Result;
use dbsrt_core::trajectory::{TrajectoryTracker, TrajectoryTrackerConfig};
use std::path::Path;
use std::time::Instant;

/// Result summary of an end-to-end trajectory tracking run.
#[derive(Debug, Clone)]
pub struct TrajectoryMatchResult {
    pub target_media: String,
    pub evaluated_frames_count: u32,
    pub initial_candidates_count: usize,
    pub surviving_trajectories_count: usize,
    pub winner_media_id: Option<u32>,
    pub winner_title: String,
    pub winner_matched_frames: u32,
    pub winner_score: f64,
    pub winner_initial_timestamp_ms: u32,
    pub elapsed_seconds: f64,
}

/// Request criteria for executing trajectory matching.
#[derive(Clone)]
pub struct TrajectoryMatchRequest<'a> {
    pub dbphash: &'a MmapDbphash,
    pub target_path: &'a Path,
    pub start_sec: f64,
    pub nb_frames: usize,
}

/// Executes trajectory tracking against an MmapDbphash catalog on an extracted frame sequence.
pub fn execute_trajectory_matching(
    req: &TrajectoryMatchRequest<'_>,
    config: TrajectoryTrackerConfig,
) -> Result<TrajectoryMatchResult> {
    let t0 = Instant::now();
    let opts = dbsrt_core::video::VideoExtractionOptions {
        start_sec: req.start_sec,
        max_frames: if req.nb_frames > 0 {
            Some(req.nb_frames)
        } else {
            None
        },
        sampling: dbsrt_core::video::SamplingMode::Fps(1.0),
        hwaccel: dbsrt_core::video::HwAccel::Disabled,
        ..Default::default()
    };
    let samples = dbsrt_core::video::extract_frames_advanced(req.target_path, &opts)?;
    let hashes: Vec<u64> = samples.into_iter().map(|s| s.phash).collect();
    if hashes.is_empty() {
        anyhow::bail!("No frames extracted from target media");
    }

    let target_name = req
        .target_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Find first frame suitable for t0 seeding (reject blank/black frames if configured)
    let seed_idx_opt = if config.reject_blank_seed {
        hashes.iter().position(|&h| !dbsrt_core::is_blank_phash(h))
    } else {
        Some(0)
    };

    let seed_idx = match seed_idx_opt {
        Some(idx) => idx,
        None => {
            return Ok(TrajectoryMatchResult {
                target_media: target_name,
                evaluated_frames_count: hashes.len() as u32,
                initial_candidates_count: 0,
                surviving_trajectories_count: 0,
                winner_media_id: None,
                winner_title: "NONE".to_string(),
                winner_matched_frames: 0,
                winner_score: 0.0,
                winner_initial_timestamp_ms: 0,
                elapsed_seconds: t0.elapsed().as_secs_f64(),
            });
        }
    };

    let mut tracker = TrajectoryTracker::new(config.clone());

    // Frame seed_idx: Seed hypotheses from sparse index matches
    let f0_matches = req
        .dbphash
        .search_sparse(&hashes[seed_idx..=seed_idx], config.seed_threshold);
    let initial_candidates: Vec<(u32, u32, u32)> = f0_matches
        .into_iter()
        .map(|m| (m.cut_entry.media_id, m.cut_entry.timestamp_ms, m.distance))
        .collect();
    let initial_count = initial_candidates.len();
    tracker.seed(initial_candidates);

    // Frames (seed_idx + 1)..N: Advance and filter trajectories
    for &hash in &hashes[(seed_idx + 1)..] {
        if tracker.active_count() == 0 {
            break; // All hypotheses pruned early
        }
        if config.skip_blank_steps && dbsrt_core::is_blank_phash(hash) {
            tracker.advance(std::iter::empty());
        } else {
            let step_matches = req.dbphash.search_sparse(&[hash], config.step_threshold);
            let step_candidates: Vec<(u32, u32, u32)> = step_matches
                .into_iter()
                .map(|m| (m.cut_entry.media_id, m.cut_entry.timestamp_ms, m.distance))
                .collect();
            tracker.advance(step_candidates);
        }
    }

    let elapsed = t0.elapsed().as_secs_f64();
    let surviving_count = tracker.active_count();
    let best = tracker.elect_best();

    let (winner_id, winner_title, winner_matched, winner_score, winner_ts) = if let Some(h) = best {
        let title = req
            .dbphash
            .find_media_by_id(h.media_id)
            .map(|m| req.dbphash.get_title(m).to_string())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        let initial_ts = h
            .initial_ref_timestamp_ms
            .saturating_sub(seed_idx as u32 * config.step_interval_ms);
        (
            Some(h.media_id),
            title,
            h.matched_frames,
            h.accumulated_score,
            initial_ts,
        )
    } else {
        (None, "NONE".to_string(), 0, 0.0, 0)
    };

    Ok(TrajectoryMatchResult {
        target_media: target_name,
        evaluated_frames_count: hashes.len() as u32,
        initial_candidates_count: initial_count,
        surviving_trajectories_count: surviving_count,
        winner_media_id: winner_id,
        winner_title,
        winner_matched_frames: winner_matched,
        winner_score,
        winner_initial_timestamp_ms: winner_ts,
        elapsed_seconds: elapsed,
    })
}
