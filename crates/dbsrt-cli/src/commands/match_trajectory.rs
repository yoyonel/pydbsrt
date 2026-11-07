//! CLI Subcommand: match-trajectory
//!
//! Evaluates candidate media through sequential trajectory accumulation, tracking
//! valid hypotheses across frames and pruning divergent candidates in real-time.

use anyhow::{bail, Result};
use clap::Args;
use dbsrt_core::trajectory::TrajectoryTrackerConfig;
use dbsrt_index::dbphash::MmapDbphash;
use dbsrt_index::trajectory_runner::execute_trajectory_matching;
use std::path::PathBuf;

/// Arguments for `match-trajectory` subcommand.
#[derive(Args, Debug, Clone)]
pub struct MatchTrajectoryArgs {
    /// Video media file to evaluate
    #[arg(short, long)]
    pub media: PathBuf,

    /// Unified .dbphash index catalog to search against
    #[arg(long, default_value = "data/catalog.dbphash")]
    pub dbphash: PathBuf,

    /// Start timestamp along target media timeline in seconds (default: 50.0% of duration if available, or 0.0)
    #[arg(short, long, default_value_t = 0.0)]
    pub start: f64,

    /// Number of consecutive frames to evaluate (e.g. 24, 48, 120)
    #[arg(short = 'n', long, default_value_t = 24)]
    pub frames: usize,

    /// Initial seed Hamming distance threshold (default: 10)
    #[arg(long, default_value_t = 10)]
    pub seed_threshold: u32,

    /// Step propagation Hamming distance threshold (default: 8)
    #[arg(long, default_value_t = 8)]
    pub step_threshold: u32,

    /// Maximum consecutive misses before pruning hypothesis (default: 3)
    #[arg(long, default_value_t = 3)]
    pub max_misses: u32,

    /// Minimum coverage ratio (0.0 to 1.0) of evaluated frames required to elect winner (default: 0.50)
    #[arg(long, default_value_t = 0.50)]
    pub min_coverage: f64,

    /// Temporal slack tolerance in milliseconds around expected timestamp (default: 1200 ms)
    #[arg(long, default_value_t = 1200)]
    pub time_slack_ms: i64,

    /// Maximum active hypotheses tracked in memory (default: 1000)
    #[arg(long, default_value_t = 1000)]
    pub max_active: usize,

    /// Allow establishing initial t0 hypothesis seed on uniform/blank/black frames
    #[arg(long, default_value_t = false)]
    pub allow_blank_seed: bool,

    /// Output format (DataFrame, CSV, JSON) [default: DataFrame]
    #[arg(long, value_enum, default_value_t = crate::cli::OutputFormat::DataFrame)]
    pub output_format: crate::cli::OutputFormat,
}

/// Executes trajectory matching command.
pub fn run_match_trajectory(args: MatchTrajectoryArgs) -> Result<()> {
    if !args.media.is_file() {
        bail!("Media file not found: {}", args.media.display());
    }
    if !args.dbphash.is_file() {
        bail!("Catalog file not found: {}", args.dbphash.display());
    }

    let dbphash = MmapDbphash::open(&args.dbphash)?;
    let config = TrajectoryTrackerConfig {
        seed_threshold: args.seed_threshold,
        step_threshold: args.step_threshold,
        max_consecutive_misses: args.max_misses,
        time_slack_ms: args.time_slack_ms,
        step_interval_ms: 1000,
        max_active_hypotheses: args.max_active,
        reject_blank_seed: !args.allow_blank_seed,
        skip_blank_steps: true,
        min_coverage_ratio: args.min_coverage,
        election_strategy: dbsrt_core::trajectory::ElectionStrategy::QualityScore,
    };

    if args.output_format == crate::cli::OutputFormat::DataFrame {
        println!(
            "🚀 Trajectory Tracking on '{}' (evaluating {} frames from t={:.1}s)...",
            args.media.file_name().unwrap_or_default().to_string_lossy(),
            args.frames,
            args.start
        );
    }

    let req = dbsrt_index::trajectory_runner::TrajectoryMatchRequest {
        dbphash: &dbphash,
        target_path: &args.media,
        start_sec: args.start,
        nb_frames: args.frames,
    };
    let res = execute_trajectory_matching(&req, config)?;

    if args.output_format == crate::cli::OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&res)?);
        return Ok(());
    }

    if args.output_format == crate::cli::OutputFormat::Csv {
        println!("target_media,evaluated_frames,initial_candidates,surviving,winner_id,winner_title,matched_frames,coverage_ratio,score,ref_start_ms,elapsed_sec");
        println!(
            "{},{},{},{},{},\"{}\",{},{:.4},{:.2},{},{:.3}",
            res.target_media,
            res.evaluated_frames_count,
            res.initial_candidates_count,
            res.surviving_trajectories_count,
            res.winner_media_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            res.winner_title,
            res.winner_matched_frames,
            res.winner_coverage_ratio,
            res.winner_score,
            res.winner_initial_timestamp_ms,
            res.elapsed_seconds
        );
        return Ok(());
    }

    println!("================================================================================");
    println!("🎯 TRAJECTORY TRACKING VERDICT:");
    println!("  Target Media:                {}", res.target_media);
    println!(
        "  Evaluated Target Frames:     {}",
        res.evaluated_frames_count
    );
    println!(
        "  Initial Seed Candidates:     {}",
        res.initial_candidates_count
    );
    println!(
        "  Surviving Trajectories:      {}",
        res.surviving_trajectories_count
    );
    if let Some(winner_id) = res.winner_media_id {
        let confidence_pct =
            (res.winner_matched_frames as f64 / res.evaluated_frames_count.max(1) as f64) * 100.0;
        println!("  Elected Winner Media ID:     {}", winner_id);
        println!("  Elected Winner Title:        {}", res.winner_title);
        println!(
            "  Matched Frames in Trajectory: {}/{} ({:.1}%)",
            res.winner_matched_frames, res.evaluated_frames_count, confidence_pct
        );
        println!(
            "  Reference Start Timestamp:   {:.3}s ({} ms)",
            res.winner_initial_timestamp_ms as f64 / 1000.0,
            res.winner_initial_timestamp_ms
        );
        println!("  Quality Accumulated Score:   {:.2}", res.winner_score);
    } else {
        println!("  Elected Winner:              NONE (All hypotheses pruned)");
    }
    println!("  Evaluation Time:             {:.3}s", res.elapsed_seconds);
    println!("================================================================================");

    Ok(())
}
