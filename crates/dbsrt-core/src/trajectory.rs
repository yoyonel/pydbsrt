//! Multi-Hypothesis Trajectory Tracking Matching Engine.
//!
//! Accumulates perceptual hash frame observations along media timeline, maintaining
//! concurrent candidate hypotheses and pruning divergent trajectories in real-time.

use serde::{Deserialize, Serialize};

/// Default maximum Hamming distance to seed a new trajectory hypothesis at t0.
pub const DEFAULT_SEED_HAMMING_THRESHOLD: u32 = 10;

/// Default maximum Hamming distance to advance an active trajectory hypothesis.
pub const DEFAULT_STEP_HAMMING_THRESHOLD: u32 = 8;

/// Default maximum consecutive mismatches allowed before pruning an hypothesis.
pub const DEFAULT_MAX_CONSECUTIVE_MISSES: u32 = 3;

/// Default frame duration in milliseconds for sparse 1 fps frames (1000 ms).
pub const DEFAULT_STEP_INTERVAL_MS: u32 = 1000;

/// Default temporal tolerance in milliseconds around expected timestamp (+/- 1200 ms).
pub const DEFAULT_TIME_SLACK_MS: i64 = 1200;

/// Strategy for electing the winning trajectory from survivors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ElectionStrategy {
    /// Ranks by accumulated similarity quality score first (default).
    #[default]
    QualityScore,
    /// Ranks strictly by the longest streak / highest count of matched frames.
    MaxMatchedFrames,
    /// Ranks by minimum average Hamming distance per matched frame.
    LowestHammingCost,
}

/// Default maximum number of concurrent active hypotheses allowed.
pub const DEFAULT_MAX_ACTIVE_HYPOTHESES: usize = 1000;

/// Default initial pre-allocated scratchpad capacity for candidates buffer.
pub const DEFAULT_SCRATCHPAD_INITIAL_CAPACITY: usize = 2048;

/// Configuration criteria for trajectory tracking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryTrackerConfig {
    /// Maximum Hamming distance allowed to seed a hypothesis.
    pub seed_threshold: u32,
    /// Maximum Hamming distance allowed to sustain a step.
    pub step_threshold: u32,
    /// Maximum consecutive misses before pruning.
    pub max_consecutive_misses: u32,
    /// Temporal slack in milliseconds around expected reference timestamp (+/- ms).
    pub time_slack_ms: i64,
    /// Expected interval in milliseconds between consecutive query frames.
    pub step_interval_ms: u32,
    /// Maximum number of active hypotheses tracked concurrently in memory.
    pub max_active_hypotheses: usize,
    /// Whether to prohibit establishing t0 seed on uniform, blank or black frames (low visual energy).
    pub reject_blank_seed: bool,
    /// Whether to skip sparse catalog candidate lookups on blank or black frames during tracking propagation.
    pub skip_blank_steps: bool,
    /// Minimum percentage of evaluated frames matched (0.0 to 1.0) required to elect a winner.
    pub min_coverage_ratio: f64,
    /// Selection strategy used to rank and elect survivor hypotheses.
    pub election_strategy: ElectionStrategy,
}

impl Default for TrajectoryTrackerConfig {
    fn default() -> Self {
        Self {
            seed_threshold: DEFAULT_SEED_HAMMING_THRESHOLD,
            step_threshold: DEFAULT_STEP_HAMMING_THRESHOLD,
            max_consecutive_misses: DEFAULT_MAX_CONSECUTIVE_MISSES,
            time_slack_ms: DEFAULT_TIME_SLACK_MS,
            step_interval_ms: DEFAULT_STEP_INTERVAL_MS,
            max_active_hypotheses: DEFAULT_MAX_ACTIVE_HYPOTHESES,
            reject_blank_seed: true,
            skip_blank_steps: true,
            min_coverage_ratio: 0.50,
            election_strategy: ElectionStrategy::QualityScore,
        }
    }
}

/// Active trajectory hypothesis for a candidate media.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryHypothesis {
    /// Unique candidate media identifier.
    pub media_id: u32,
    /// Reference media timestamp (ms) corresponding to target frame 0.
    pub initial_ref_timestamp_ms: u32,
    /// Last matched reference media timestamp (ms).
    pub last_ref_timestamp_ms: u32,
    /// Total number of target frames where this trajectory matched.
    pub matched_frames: u32,
    /// Current consecutive misses counter.
    pub consecutive_misses: u32,
    /// Total accumulated matching score (higher is better).
    pub accumulated_score: f64,
    /// Total Hamming distance cost over matched frames.
    pub total_hamming_cost: u32,
}

impl TrajectoryHypothesis {
    /// Creates a newly seeded hypothesis at target frame 0.
    pub fn new(
        media_id: u32,
        ref_timestamp_ms: u32,
        hamming_dist: u32,
        max_threshold: u32,
    ) -> Self {
        let quality = 1.0 - (hamming_dist as f64 / (max_threshold.max(1) as f64 + 1.0));
        Self {
            media_id,
            initial_ref_timestamp_ms: ref_timestamp_ms,
            last_ref_timestamp_ms: ref_timestamp_ms,
            matched_frames: 1,
            consecutive_misses: 0,
            accumulated_score: quality.max(0.0),
            total_hamming_cost: hamming_dist,
        }
    }
}

/// Candidate match observation with distance and timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateObservation {
    pub media_id: u32,
    pub ref_timestamp_ms: u32,
    pub distance: u32,
}

/// Pre-allocated reusable workspace scratchpad for zero-allocation trajectory updates.
#[derive(Debug, Clone)]
pub struct TrajectoryScratchpad {
    /// Reusable flat vector of candidate matches for the current frame.
    pub candidates: Vec<CandidateObservation>,
    /// Range map of offsets: (media_id, start_idx, count) inside `candidates` sorted by media_id.
    pub media_ranges: std::collections::HashMap<u32, (usize, usize)>,
}

impl Default for TrajectoryScratchpad {
    fn default() -> Self {
        Self {
            candidates: Vec::with_capacity(DEFAULT_SCRATCHPAD_INITIAL_CAPACITY),
            media_ranges: std::collections::HashMap::with_capacity(DEFAULT_MAX_ACTIVE_HYPOTHESES),
        }
    }
}

impl TrajectoryScratchpad {
    /// Clears and loads candidate observations without reallocating underlying memory.
    pub fn reset_and_index<I>(&mut self, candidates: I, step_threshold: u32)
    where
        I: IntoIterator<Item = (u32, u32, u32)>,
    {
        self.candidates.clear();
        self.media_ranges.clear();

        for (m_id, ts, dist) in candidates {
            if dist <= step_threshold {
                self.candidates.push(CandidateObservation {
                    media_id: m_id,
                    ref_timestamp_ms: ts,
                    distance: dist,
                });
            }
        }

        // Sort by media_id to allow O(1) indexed lookup per hypothesis
        self.candidates.sort_unstable_by_key(|c| c.media_id);

        let mut idx = 0;
        while idx < self.candidates.len() {
            let m_id = self.candidates[idx].media_id;
            let start = idx;
            while idx < self.candidates.len() && self.candidates[idx].media_id == m_id {
                idx += 1;
            }
            let count = idx - start;
            self.media_ranges.insert(m_id, (start, count));
        }
    }
}

/// Trajectory tracker maintaining concurrent candidate paths across consecutive target frames.
#[derive(Debug, Clone)]
pub struct TrajectoryTracker {
    config: TrajectoryTrackerConfig,
    active_hypotheses: Vec<TrajectoryHypothesis>,
    scratchpad: TrajectoryScratchpad,
    processed_target_frames: u32,
}

impl TrajectoryTracker {
    /// Creates a new tracker with given configuration and pre-allocated capacity.
    pub fn new(config: TrajectoryTrackerConfig) -> Self {
        let max_hyp = config.max_active_hypotheses;
        Self {
            config,
            active_hypotheses: Vec::with_capacity(max_hyp),
            scratchpad: TrajectoryScratchpad::default(),
            processed_target_frames: 0,
        }
    }

    /// Number of currently active hypotheses.
    pub fn active_count(&self) -> usize {
        self.active_hypotheses.len()
    }

    /// Number of target frames processed so far.
    pub fn processed_frames(&self) -> u32 {
        self.processed_target_frames
    }

    /// Seeds initial hypotheses from frame 0 candidate matches.
    pub fn seed<I>(&mut self, candidates: I)
    where
        I: IntoIterator<Item = (u32, u32, u32)>,
    {
        self.active_hypotheses.clear();
        for (media_id, ref_timestamp_ms, dist) in candidates {
            if dist <= self.config.seed_threshold {
                self.active_hypotheses.push(TrajectoryHypothesis::new(
                    media_id,
                    ref_timestamp_ms,
                    dist,
                    self.config.seed_threshold,
                ));
                if self.active_hypotheses.len() >= self.config.max_active_hypotheses {
                    break;
                }
            }
        }
        self.processed_target_frames = 1;
    }

    /// Advances tracker with candidates observed at next target frame.
    /// Uses pre-allocated scratchpad and range index: O(H + C) total update complexity.
    pub fn advance<I>(&mut self, candidates: I)
    where
        I: IntoIterator<Item = (u32, u32, u32)>,
    {
        let target_idx = self.processed_target_frames;
        self.processed_target_frames += 1;

        // Zero-alloc indexed reset of frame candidate observations
        self.scratchpad
            .reset_and_index(candidates, self.config.step_threshold);

        let time_slack = self.config.time_slack_ms;
        let step_ms = self.config.step_interval_ms as i64;
        let step_thresh = self.config.step_threshold;

        for hyp in &mut self.active_hypotheses {
            let expected_ref = hyp.initial_ref_timestamp_ms as i64 + (target_idx as i64 * step_ms);

            // O(1) retrieval of candidates for this specific media_id via scratchpad range
            let best_match =
                if let Some(&(start, count)) = self.scratchpad.media_ranges.get(&hyp.media_id) {
                    self.scratchpad.candidates[start..start + count]
                        .iter()
                        .filter(|c| (c.ref_timestamp_ms as i64 - expected_ref).abs() <= time_slack)
                        .min_by_key(|c| c.distance)
                } else {
                    None
                };

            if let Some(cand) = best_match {
                let quality = 1.0 - (cand.distance as f64 / (step_thresh.max(1) as f64 + 1.0));
                hyp.matched_frames += 1;
                hyp.last_ref_timestamp_ms = cand.ref_timestamp_ms;
                hyp.consecutive_misses = 0;
                hyp.accumulated_score += quality.max(0.0);
                hyp.total_hamming_cost += cand.distance;
            } else {
                hyp.consecutive_misses += 1;
            }
        }

        let max_misses = self.config.max_consecutive_misses;
        self.active_hypotheses
            .retain(|h| h.consecutive_misses <= max_misses);
    }

    /// Returns ranked hypotheses by chosen election strategy.
    pub fn rank_hypotheses(&self) -> Vec<TrajectoryHypothesis> {
        let mut list = self.active_hypotheses.clone();
        match self.config.election_strategy {
            ElectionStrategy::QualityScore => Self::sort_by_quality(&mut list),
            ElectionStrategy::MaxMatchedFrames => Self::sort_by_matched(&mut list),
            ElectionStrategy::LowestHammingCost => Self::sort_by_hamming(&mut list),
        }
        list
    }

    fn sort_by_quality(list: &mut [TrajectoryHypothesis]) {
        list.sort_by(|a, b| {
            b.accumulated_score
                .partial_cmp(&a.accumulated_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.matched_frames.cmp(&a.matched_frames))
                .then_with(|| a.total_hamming_cost.cmp(&b.total_hamming_cost))
        });
    }

    fn sort_by_matched(list: &mut [TrajectoryHypothesis]) {
        list.sort_by(|a, b| {
            b.matched_frames
                .cmp(&a.matched_frames)
                .then_with(|| {
                    b.accumulated_score
                        .partial_cmp(&a.accumulated_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.total_hamming_cost.cmp(&b.total_hamming_cost))
        });
    }

    fn sort_by_hamming(list: &mut [TrajectoryHypothesis]) {
        list.sort_by(|a, b| {
            let avg_a = a.total_hamming_cost as f64 / a.matched_frames.max(1) as f64;
            let avg_b = b.total_hamming_cost as f64 / b.matched_frames.max(1) as f64;
            avg_a
                .partial_cmp(&avg_b)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.matched_frames.cmp(&a.matched_frames))
        });
    }

    /// Elects best candidate if one exists and satisfies minimum coverage ratio.
    pub fn elect_best(&self) -> Option<TrajectoryHypothesis> {
        let candidate = self.rank_hypotheses().into_iter().next()?;
        let coverage = candidate.matched_frames as f64 / self.processed_target_frames.max(1) as f64;
        if coverage >= self.config.min_coverage_ratio {
            Some(candidate)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trajectory_lifecycle_and_pruning() {
        let config = TrajectoryTrackerConfig {
            seed_threshold: 8,
            step_threshold: 6,
            max_consecutive_misses: 2,
            time_slack_ms: 200,
            step_interval_ms: 1000,
            max_active_hypotheses: 100,
            reject_blank_seed: true,
            skip_blank_steps: true,
            min_coverage_ratio: 0.50,
            election_strategy: ElectionStrategy::QualityScore,
        };
        let mut tracker = TrajectoryTracker::new(config);

        // Frame 0: seed 3 hypotheses
        // Hyp A (media 1): starts at ts 100_000ms
        // Hyp B (media 2): starts at ts 500_000ms (will diverge)
        // Hyp C (media 3): starts at ts 900_000ms (will fail immediately)
        tracker.seed(vec![(1, 100_000, 2), (2, 500_000, 3), (3, 900_000, 4)]);
        assert_eq!(tracker.active_count(), 3);

        // Frame 1: Hyp A matches 101_000, Hyp B matches 501_000, Hyp C missing
        tracker.advance(vec![(1, 101_000, 1), (2, 501_000, 2)]);
        assert_eq!(tracker.active_count(), 3); // Hyp C has 1 miss, max is 2

        // Frame 2: Hyp A matches 102_000, Hyp B missing, Hyp C missing
        tracker.advance(vec![(1, 102_000, 0)]);
        assert_eq!(tracker.active_count(), 3);

        // Frame 3: Hyp A matches 103_000, Hyp B missing, Hyp C missing
        tracker.advance(vec![(1, 103_000, 1)]);
        // Hyp C has 3 misses -> pruned! Hyp B has 2 misses -> retained.
        assert_eq!(tracker.active_count(), 2);

        // Frame 4: Hyp A matches 104_000, Hyp B missing
        tracker.advance(vec![(1, 104_000, 0)]);
        // Hyp B has 3 misses -> pruned! Only Hyp A survives.
        assert_eq!(tracker.active_count(), 1);

        let best = tracker.elect_best().expect("Hypothesis A must win");
        assert_eq!(best.media_id, 1);
        assert_eq!(best.initial_ref_timestamp_ms, 100_000);
        assert_eq!(best.matched_frames, 5); // 0, 1, 2, 3, 4 all matched
        assert_eq!(best.consecutive_misses, 0);
    }
}
