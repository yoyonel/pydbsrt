use crate::dbphash::MmapDbphash;
use anyhow::Result;
use dbsrt_core::trajectory::{TrajectoryTracker, TrajectoryTrackerConfig};
use std::path::Path;
use std::time::Instant;

/// Result summary of an end-to-end trajectory tracking run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrajectoryMatchResult {
    pub target_media: String,
    pub evaluated_frames_count: u32,
    pub initial_candidates_count: usize,
    pub surviving_trajectories_count: usize,
    pub winner_media_id: Option<u32>,
    pub winner_title: String,
    pub winner_matched_frames: u32,
    pub winner_coverage_ratio: f64,
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

/// Extracts query perceptual hashes from video via FFmpeg or directly from .phash binary.
///
/// # Arguments
/// * `req` - Trajectory match request containing target path, start timestamp and requested frames count.
///
/// # Returns
/// * `Result<Vec<u64>>` - Sequence of extracted perceptual hashes sampled at 1.0 fps.
///
/// # Errors
/// * Returns Err if target path does not exist or decoding fails.
///
/// # Panics
/// * Never panics.
///
/// # Data Flow & Invariants
/// 1. If file extension is `.phash`, reads binary fingerprint directly via `read_phash_file` and samples at 1.0 fps.
/// 2. If file is a video, invokes FFmpeg frame extraction at 1.0 fps.
pub fn extract_query_hashes(req: &TrajectoryMatchRequest<'_>) -> Result<Vec<u64>> {
    if req.target_path.extension().and_then(|e| e.to_str()) == Some("phash") {
        extract_from_phash_file(req)
    } else {
        extract_from_video_file(req)
    }
}

fn extract_from_phash_file(req: &TrajectoryMatchRequest<'_>) -> Result<Vec<u64>> {
    let all_hashes = dbsrt_core::read_phash_file(req.target_path)?;
    let stem = req
        .target_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let fps = dbsrt_core::parse_fps_from_stem(stem);
    let effective_fps = if fps > 0.0 {
        fps
    } else {
        dbsrt_core::DEFAULT_BASELINE_FPS
    };
    let total_frames = all_hashes.len();
    let total_duration_sec = total_frames as f64 / effective_fps;
    let start_sec = if req.start_sec >= total_duration_sec {
        (total_duration_sec * 0.2).max(0.0)
    } else {
        req.start_sec
    };
    let nb_samples = if req.nb_frames > 0 { req.nb_frames } else { 24 };
    let mut sampled = Vec::with_capacity(nb_samples);
    for i in 0..nb_samples {
        let t = start_sec + i as f64;
        let f_idx = (t * effective_fps).round() as usize;
        if f_idx < all_hashes.len() {
            sampled.push(all_hashes[f_idx]);
        } else {
            break;
        }
    }
    Ok(sampled)
}

fn extract_from_video_file(req: &TrajectoryMatchRequest<'_>) -> Result<Vec<u64>> {
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
    Ok(samples.into_iter().map(|s| s.phash).collect())
}

/// Executes trajectory tracking against an MmapDbphash catalog on an extracted frame sequence.
pub fn execute_trajectory_matching(
    req: &TrajectoryMatchRequest<'_>,
    config: TrajectoryTrackerConfig,
) -> Result<TrajectoryMatchResult> {
    let t0 = Instant::now();
    let hashes = extract_query_hashes(req)?;
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
                winner_coverage_ratio: 0.0,
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

    let total_frames = hashes.len() as u32;
    let coverage_ratio = if total_frames > 0 {
        winner_matched as f64 / total_frames as f64
    } else {
        0.0
    };

    Ok(TrajectoryMatchResult {
        target_media: target_name,
        evaluated_frames_count: total_frames,
        initial_candidates_count: initial_count,
        surviving_trajectories_count: surviving_count,
        winner_media_id: winner_id,
        winner_title,
        winner_matched_frames: winner_matched,
        winner_coverage_ratio: coverage_ratio,
        winner_score,
        winner_initial_timestamp_ms: winner_ts,
        elapsed_seconds: elapsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbphash::DbphashBuilder;
    use dbsrt_core::metadata::{MediaMetadata, MediaType};

    #[test]
    fn test_trajectory_result_serde() {
        let res = TrajectoryMatchResult {
            target_media: "sample.mp4".to_string(),
            evaluated_frames_count: 24,
            initial_candidates_count: 5,
            surviving_trajectories_count: 1,
            winner_media_id: Some(1),
            winner_title: "Test Film".to_string(),
            winner_matched_frames: 20,
            winner_coverage_ratio: 0.833,
            winner_score: 95.5,
            winner_initial_timestamp_ms: 1000,
            elapsed_seconds: 0.05,
        };
        let json = serde_json::to_string(&res).expect("serialize");
        let de: TrajectoryMatchResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.winner_media_id, Some(1));
        assert_eq!(de.winner_title, "Test Film");
    }

    #[test]
    fn test_extract_query_hashes_phash_direct() {
        let dir = std::env::temp_dir();
        let phash_path = dir.join(format!("test_query_{}.24.0fps.phash", std::process::id()));
        let hashes = vec![0x1111_2222_3333_4444_u64; 48];
        dbsrt_core::write_phash_file(&phash_path, &hashes).expect("write phash");

        let dummy_catalog = dir.join(format!("dummy_{}.dbphash", std::process::id()));
        let builder = DbphashBuilder::new();
        let _ = builder.write_to_file(&dummy_catalog);
        if let Ok(dbphash) = MmapDbphash::open(&dummy_catalog) {
            let req = TrajectoryMatchRequest {
                dbphash: &dbphash,
                target_path: &phash_path,
                start_sec: 0.0,
                nb_frames: 2,
            };
            let extracted = extract_query_hashes(&req).expect("extract hashes");
            assert_eq!(extracted.len(), 2);
            assert_eq!(extracted[0], 0x1111_2222_3333_4444_u64);
        }
        let _ = std::fs::remove_file(&phash_path);
        let _ = std::fs::remove_file(&dummy_catalog);
    }

    #[test]
    fn test_execute_trajectory_matching_synthetic() {
        let dir = std::env::temp_dir();
        let catalog_path = dir.join(format!("test_traj_cat_{}.dbphash", std::process::id()));
        let mut builder = DbphashBuilder::new();
        let meta = MediaMetadata {
            raw_title: "Test.Show.S01E01.mkv".to_string(),
            clean_title: "Test Show".to_string(),
            media_type: MediaType::TvEpisode,
            year: Some(2023),
            season: Some(1),
            episode: Some(1),
            resolution: None,
            source: None,
            release_group: None,
            moviehash: None,
        };
        let sparse = [
            (0x1234_5678_9abc_def0_u64, 0u32),
            (0x1234_5678_9abc_def1_u64, 1000u32),
            (0x1234_5678_9abc_def2_u64, 2000u32),
            (0x1234_5678_9abc_def3_u64, 3000u32),
        ];
        let cuts = sparse;
        builder.add_media(&meta, 10_000, 24.0, &cuts, &sparse);
        builder.write_to_file(&catalog_path).expect("build catalog");

        let dbphash = MmapDbphash::open(&catalog_path).expect("open mmap");
        let query_path = dir.join(format!(
            "test_query_traj_{}.1.0fps.phash",
            std::process::id()
        ));
        let query_hashes = vec![
            0x1234_5678_9abc_def0_u64,
            0x1234_5678_9abc_def1_u64,
            0x1234_5678_9abc_def2_u64,
            0x1234_5678_9abc_def3_u64,
        ];
        dbsrt_core::write_phash_file(&query_path, &query_hashes).expect("write query");

        let req = TrajectoryMatchRequest {
            dbphash: &dbphash,
            target_path: &query_path,
            start_sec: 0.0,
            nb_frames: 4,
        };
        let config = TrajectoryTrackerConfig {
            seed_threshold: 5,
            step_threshold: 5,
            max_consecutive_misses: 2,
            time_slack_ms: 1200,
            step_interval_ms: 1000,
            max_active_hypotheses: 100,
            reject_blank_seed: true,
            skip_blank_steps: true,
            min_coverage_ratio: 0.5,
            election_strategy: dbsrt_core::trajectory::ElectionStrategy::QualityScore,
        };
        let res = execute_trajectory_matching(&req, config).expect("run trajectory");
        assert_eq!(res.winner_media_id, Some(1));
        assert_eq!(res.winner_title, "Test Show");
        assert_eq!(res.evaluated_frames_count, 4);
        assert!(res.winner_matched_frames >= 3);

        let _ = std::fs::remove_file(&catalog_path);
        let _ = std::fs::remove_file(&query_path);
    }
}
