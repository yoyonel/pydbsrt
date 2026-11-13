//! Spatiotemporal Multi-Window Matching Plan and Consensus Engine.
//!
//! Evaluates candidate media across multiple temporal offset windows (e.g. 20%, 50%, 80%)
//! to eliminate false positive collisions caused by shared eyecatches, intros, or static jingles.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Milliseconds per second (target start conversion).
const MILLIS_PER_SECOND: f64 = 1000.0;

/// Scale factor converting ratios (`0.0..=1.0`) into percentages (`0.0..=100.0`).
pub const PERCENT_SCALE: f64 = 100.0;

/// Default multi-window temporal ratios (20%, 50%, 80% along the media timeline).
pub const DEFAULT_MULTI_WINDOW_RATIOS: [f64; 3] = [0.20, 0.50, 0.80];

/// Maximum allowable shift drift in milliseconds between windows to declare temporal alignment.
pub const MAX_SHIFT_DRIFT_TOLERANCE_MS: i64 = 2500;

/// Multi-window extraction specification defining temporal sampling points.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiWindowPlan {
    /// Ordered list of fractional offsets along media timeline (values in `(0.0, 1.0)`).
    pub ratios: Vec<f64>,
    /// Duration of each extraction window in seconds.
    pub window_duration_sec: f64,
}

impl Default for MultiWindowPlan {
    fn default() -> Self {
        Self {
            ratios: DEFAULT_MULTI_WINDOW_RATIOS.to_vec(),
            window_duration_sec: 1.0,
        }
    }
}

impl MultiWindowPlan {
    /// Creates a new multi-window plan with custom ratios and window duration.
    ///
    /// # Arguments
    /// * `ratios` - List of fractional timeline ratios (e.g. `[0.2, 0.5, 0.8]`).
    /// * `window_duration_sec` - Duration in seconds for each window.
    ///
    /// # Returns
    /// * `Result<Self>` - Validated `MultiWindowPlan`.
    ///
    /// # Errors
    /// * Returns `Err` if ratios is empty, duration <= 0, or any ratio is outside `(0.0, 1.0)`.
    ///
    /// # Panics
    /// * Never panics.
    ///
    /// # Data Flow & Invariants
    /// * Ensures each ratio is strictly bounded `0.0 < r < 1.0`.
    /// * Preserves specified ratio order.
    pub fn new(ratios: Vec<f64>, window_duration_sec: f64) -> Result<Self> {
        if ratios.is_empty() {
            return Err(anyhow!("MultiWindowPlan ratios cannot be empty"));
        }
        if window_duration_sec <= 0.0 {
            return Err(anyhow!(
                "Window duration must be > 0.0, got {window_duration_sec}"
            ));
        }
        for &r in &ratios {
            if !(0.0..1.0).contains(&r) {
                return Err(anyhow!("Timeline ratio {r} out of valid range (0.0, 1.0)"));
            }
        }
        Ok(Self {
            ratios,
            window_duration_sec,
        })
    }

    /// Computes start timestamps (in seconds) for each window given media total duration.
    ///
    /// # Arguments
    /// * `total_duration_sec` - Total media duration in seconds.
    ///
    /// # Returns
    /// * `Vec<f64>` - Start time in seconds for each configured window.
    ///
    /// # Errors
    /// * Never returns Err.
    ///
    /// # Panics
    /// * Never panics.
    ///
    /// # Data Flow & Invariants
    /// * Clamps window start so `start + window_duration <= total_duration` whenever possible.
    pub fn compute_start_seconds(&self, total_duration_sec: f64) -> Vec<f64> {
        self.ratios
            .iter()
            .map(|&r| {
                let raw_start = (total_duration_sec * r) - (self.window_duration_sec / 2.0);
                let max_start = (total_duration_sec - self.window_duration_sec).max(0.0);
                raw_start.clamp(0.0, max_start)
            })
            .collect()
    }
}

/// Single window match result for a candidate media entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowMatchItem {
    /// Window index in the evaluation plan (0-based).
    pub window_idx: usize,
    /// Timeline fractional ratio of the window.
    pub ratio: f64,
    /// Window start offset in target media in seconds.
    pub target_start_sec: f64,
    /// Number of matching scene cuts detected.
    pub matching_cuts: usize,
    /// Number of matching sparse 1 fps frames.
    pub matching_sparse: usize,
    /// Estimated time shift in milliseconds relative to candidate media timeline.
    pub estimated_shift_ms: i64,
    /// Computed matching confidence percentage (0.0 - 100.0).
    pub confidence: f64,
}

/// Aggregated multi-window consensus result for a candidate media entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiWindowConsensus {
    /// Candidate media identifier.
    pub media_id: u32,
    /// Clean candidate media title.
    pub clean_title: String,
    /// Total number of evaluated windows.
    pub total_windows: usize,
    /// Number of windows where candidate achieved positive match.
    pub matching_windows: usize,
    /// Details for each window where candidate was detected.
    pub window_matches: Vec<WindowMatchItem>,
    /// Whether time shifts across matching windows are coherent within tolerance.
    pub is_temporally_coherent: bool,
    /// Final consensus confidence score (0.0 - 100.0).
    pub consensus_confidence: f64,
    /// Weighted or median estimated time shift in milliseconds.
    pub consensus_shift_ms: i64,
}

/// Context criteria for multi-window consensus evaluation.
#[derive(Debug, Clone)]
pub struct ConsensusEvaluationContext<'a> {
    pub media_id: u32,
    pub clean_title: &'a str,
    pub total_windows: usize,
    pub tolerance_ms: i64,
}

/// Evaluates multi-window observations and scores consensus.
///
/// # Arguments
/// * `ctx` - Evaluation context criteria ([`ConsensusEvaluationContext`]).
/// * `matches` - Individual window match items observed for this media.
///
/// # Returns
/// * `MultiWindowConsensus` containing final confidence and temporal coherence verdict.
///
/// # Errors
/// * Never returns Err.
///
/// # Panics
/// * Never panics.
///
/// # Data Flow & Invariants
/// 1. If matching_windows == 0: returns confidence 0.0, shift 0.
/// 2. If matching_windows == 1: flags as isolated window (potential eyecatch/jingle). Confidence penalized by 0.35x.
/// 3. If matching_windows >= 2: checks cross-window shift stability:
///    `|shift_i - shift_j| <= tolerance_ms`.
///    - If coherent: confidence boosted (average * 1.15 up to 100.0).
///    - If incoherent: confidence slashed to max 25.0 (unrelated scenes).
pub fn evaluate_multi_window_consensus(
    ctx: &ConsensusEvaluationContext<'_>,
    mut matches: Vec<WindowMatchItem>,
) -> MultiWindowConsensus {
    if matches.is_empty() {
        return MultiWindowConsensus {
            media_id: ctx.media_id,
            clean_title: ctx.clean_title.to_string(),
            total_windows: ctx.total_windows,
            matching_windows: 0,
            window_matches: Vec::new(),
            is_temporally_coherent: false,
            consensus_confidence: 0.0,
            consensus_shift_ms: 0,
        };
    }

    matches.sort_by_key(|m| m.window_idx);
    let matching_windows = matches.len();

    let avg_confidence: f64 =
        matches.iter().map(|m| m.confidence).sum::<f64>() / matching_windows as f64;
    let primary_shift = matches[0].estimated_shift_ms
        - (matches[0].target_start_sec * MILLIS_PER_SECOND).round() as i64;

    if matching_windows == 1 {
        // Single window match: cannot rule out shared intro/eyecatch/jingle
        let penalty_ratio = if ctx.total_windows > 1 { 0.35 } else { 1.0 };
        let final_conf = (avg_confidence * penalty_ratio).min(35.0);
        return MultiWindowConsensus {
            media_id: ctx.media_id,
            clean_title: ctx.clean_title.to_string(),
            total_windows: ctx.total_windows,
            matching_windows: 1,
            window_matches: matches,
            is_temporally_coherent: false,
            consensus_confidence: final_conf,
            consensus_shift_ms: primary_shift,
        };
    }

    // Check shift coherence across all pairs of matching windows
    // Note: each window match carries its estimated_shift_ms (reference ts) and target_start_sec.
    // The relative alignment shift is: relative_shift_ms = estimated_shift_ms - (target_start_sec * MILLIS_PER_SECOND).
    let relative_shifts: Vec<i64> = matches
        .iter()
        .map(|m| m.estimated_shift_ms - (m.target_start_sec * MILLIS_PER_SECOND).round() as i64)
        .collect();

    let min_shift = relative_shifts.iter().copied().min().unwrap_or(0);
    let max_shift = relative_shifts.iter().copied().max().unwrap_or(0);
    let drift = (max_shift - min_shift).abs();
    let is_coherent = drift <= ctx.tolerance_ms;

    let (consensus_conf, consensus_shift) = if is_coherent {
        // Multi-window verified consensus
        let window_coverage = matching_windows as f64 / ctx.total_windows.max(1) as f64;
        let boost = if window_coverage >= 0.99 { 1.15 } else { 1.05 };
        let conf = (avg_confidence * boost).clamp(0.0, PERCENT_SCALE);
        let median_shift = relative_shifts[matching_windows / 2];
        (conf, median_shift)
    } else {
        // Incoherent shifts across windows: scenes match different points in media
        let penalized = (avg_confidence * 0.25).min(25.0);
        let first_rel_shift = relative_shifts.first().copied().unwrap_or(0);
        (penalized, first_rel_shift)
    };

    MultiWindowConsensus {
        media_id: ctx.media_id,
        clean_title: ctx.clean_title.to_string(),
        total_windows: ctx.total_windows,
        matching_windows,
        window_matches: matches,
        is_temporally_coherent: is_coherent,
        consensus_confidence: consensus_conf,
        consensus_shift_ms: consensus_shift,
    }
}

/// Raw match observation tuple: `(media_id, clean_title, matching_cuts, matching_sparse, estimated_shift_ms, confidence)`.
pub type RawCandidateMatch = (u32, String, usize, usize, i64, f64);

/// Per-window query observation tuple: `(window_idx, ratio, target_start_sec, matches)`.
pub type PerWindowMatches = (usize, f64, f64, Vec<RawCandidateMatch>);

/// Aggregates multiple window match sets across a collection of candidate media.
///
/// # Arguments
/// * `total_windows` - Number of distinct temporal windows evaluated.
/// * `per_window_matches` - List of (window_idx, ratio, target_start_sec, matches) for each window.
/// * `tolerance_ms` - Max shift drift allowed between windows.
///
/// # Returns
/// * `Vec<MultiWindowConsensus>` ranked by consensus confidence descending.
///
/// # Errors
/// * Never returns Err.
///
/// # Panics
/// * Never panics.
///
/// # Data Flow & Invariants
/// * Groups observed matches by `media_id`.
/// * Evaluates consensus for each distinct candidate.
/// * Sorts descending by `consensus_confidence`, breaking ties with `matching_windows`.
pub fn aggregate_multi_window_matches(
    total_windows: usize,
    per_window_matches: Vec<PerWindowMatches>,
    tolerance_ms: i64,
) -> Vec<MultiWindowConsensus> {
    let mut by_media: HashMap<u32, (String, Vec<WindowMatchItem>)> = HashMap::new();

    for (w_idx, ratio, start_sec, matches) in per_window_matches {
        for (m_id, title, cuts, sparse, shift_ms, conf) in matches {
            let entry = by_media
                .entry(m_id)
                .or_insert_with(|| (title.clone(), Vec::new()));
            entry.1.push(WindowMatchItem {
                window_idx: w_idx,
                ratio,
                target_start_sec: start_sec,
                matching_cuts: cuts,
                matching_sparse: sparse,
                estimated_shift_ms: shift_ms,
                confidence: conf,
            });
        }
    }

    let mut results: Vec<MultiWindowConsensus> = by_media
        .into_iter()
        .map(|(m_id, (title, items))| {
            let ctx = ConsensusEvaluationContext {
                media_id: m_id,
                clean_title: &title,
                total_windows,
                tolerance_ms,
            };
            evaluate_multi_window_consensus(&ctx, items)
        })
        .collect();

    results.sort_by(|a, b| {
        b.consensus_confidence
            .total_cmp(&a.consensus_confidence)
            .then_with(|| b.matching_windows.cmp(&a.matching_windows))
            .then_with(|| a.media_id.cmp(&b.media_id))
            .then_with(|| a.clean_title.cmp(&b.clean_title))
    });

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_window_plan_validation() {
        assert!(MultiWindowPlan::new(vec![], 1.0).is_err());
        assert!(MultiWindowPlan::new(vec![0.5], 0.0).is_err());
        assert!(MultiWindowPlan::new(vec![0.5], -1.0).is_err());
        assert!(MultiWindowPlan::new(vec![1.5], 1.0).is_err());
        assert!(MultiWindowPlan::new(vec![-0.1], 1.0).is_err());

        let plan = MultiWindowPlan::new(vec![0.2, 0.5, 0.8], 2.0).unwrap();
        assert_eq!(plan.ratios, vec![0.2, 0.5, 0.8]);
        assert_eq!(plan.window_duration_sec, 2.0);
    }

    #[test]
    fn test_multi_window_start_seconds_clamping() {
        let plan = MultiWindowPlan::new(vec![0.2, 0.5, 0.8], 1.0).unwrap();
        let starts = plan.compute_start_seconds(100.0);
        assert_eq!(starts.len(), 3);
        assert!((starts[0] - 19.5).abs() < 1e-4);
        assert!((starts[1] - 49.5).abs() < 1e-4);
        assert!((starts[2] - 79.5).abs() < 1e-4);

        // Short media clamping
        let short_starts = plan.compute_start_seconds(0.8);
        for s in short_starts {
            assert_eq!(s, 0.0);
        }
    }

    #[test]
    fn test_single_window_primary_shift_is_relative_to_window_start() {
        const START_SEC: f64 = 100.0;
        const POSITIVE_REF_MS: i64 = 102_500;
        const NEGATIVE_REF_MS: i64 = 97_500;
        const EXPECTED_SHIFT_MS: i64 = 2_500;
        let ctx = ConsensusEvaluationContext {
            media_id: 7,
            clean_title: "Single Window Shift",
            total_windows: 3,
            tolerance_ms: MAX_SHIFT_DRIFT_TOLERANCE_MS,
        };
        for (ref_ms, expected) in [
            (POSITIVE_REF_MS, EXPECTED_SHIFT_MS),
            (NEGATIVE_REF_MS, -EXPECTED_SHIFT_MS),
        ] {
            let m = vec![WindowMatchItem {
                window_idx: 0,
                ratio: 0.5,
                target_start_sec: START_SEC,
                matching_cuts: 10,
                matching_sparse: 10,
                estimated_shift_ms: ref_ms,
                confidence: 80.0,
            }];
            let c = evaluate_multi_window_consensus(&ctx, m);
            assert_eq!(c.consensus_shift_ms, expected);
            assert!(!c.is_temporally_coherent);
        }
    }

    #[test]
    fn test_single_window_match_penalized() {
        let single_match = vec![WindowMatchItem {
            window_idx: 1,
            ratio: 0.5,
            target_start_sec: 50.0,
            matching_cuts: 50,
            matching_sparse: 24,
            estimated_shift_ms: 1000,
            confidence: 100.0,
        }];

        let ctx = ConsensusEvaluationContext {
            media_id: 101,
            clean_title: "Shared Eyecatch Anime",
            total_windows: 3,
            tolerance_ms: MAX_SHIFT_DRIFT_TOLERANCE_MS,
        };
        let consensus = evaluate_multi_window_consensus(&ctx, single_match);
        assert_eq!(consensus.matching_windows, 1);
        assert!(!consensus.is_temporally_coherent);
        assert_eq!(consensus.consensus_confidence, 35.0);
    }

    #[test]
    fn test_multi_window_coherent_consensus_boosted() {
        let multi_matches = vec![
            WindowMatchItem {
                window_idx: 0,
                ratio: 0.2,
                target_start_sec: 20.0,
                matching_cuts: 30,
                matching_sparse: 24,
                estimated_shift_ms: 21000,
                confidence: 90.0,
            },
            WindowMatchItem {
                window_idx: 1,
                ratio: 0.5,
                target_start_sec: 50.0,
                matching_cuts: 40,
                matching_sparse: 24,
                estimated_shift_ms: 51050,
                confidence: 95.0,
            },
            WindowMatchItem {
                window_idx: 2,
                ratio: 0.8,
                target_start_sec: 80.0,
                matching_cuts: 35,
                matching_sparse: 24,
                estimated_shift_ms: 80980,
                confidence: 92.0,
            },
        ];

        let ctx = ConsensusEvaluationContext {
            media_id: 202,
            clean_title: "Genuine Movie",
            total_windows: 3,
            tolerance_ms: MAX_SHIFT_DRIFT_TOLERANCE_MS,
        };
        let consensus = evaluate_multi_window_consensus(&ctx, multi_matches);
        assert_eq!(consensus.matching_windows, 3);
        assert!(consensus.is_temporally_coherent);
        assert!(consensus.consensus_confidence > 95.0);
        assert_eq!(consensus.consensus_shift_ms, 1050);
    }

    #[test]
    fn test_multi_window_incoherent_shifts_slashed() {
        let incoherent_matches = vec![
            WindowMatchItem {
                window_idx: 0,
                ratio: 0.2,
                target_start_sec: 20.0,
                matching_cuts: 30,
                matching_sparse: 24,
                estimated_shift_ms: 1000,
                confidence: 90.0,
            },
            WindowMatchItem {
                window_idx: 1,
                ratio: 0.5,
                target_start_sec: 50.0,
                matching_cuts: 40,
                matching_sparse: 24,
                estimated_shift_ms: 50000, // 50s discrepancy!
                confidence: 85.0,
            },
        ];

        let ctx = ConsensusEvaluationContext {
            media_id: 303,
            clean_title: "Incoherent Media",
            total_windows: 3,
            tolerance_ms: MAX_SHIFT_DRIFT_TOLERANCE_MS,
        };
        let consensus = evaluate_multi_window_consensus(&ctx, incoherent_matches);
        assert_eq!(consensus.matching_windows, 2);
        assert!(!consensus.is_temporally_coherent);
        assert!(consensus.consensus_confidence <= 25.0);
    }

    #[test]
    fn test_aggregate_multi_window_matches_deterministic_tie_breaking() {
        let windows_a = vec![(
            0usize,
            0.5f64,
            50.0f64,
            vec![
                (
                    20u32,
                    "Beta".to_string(),
                    10usize,
                    10usize,
                    50_000i64,
                    80.0f64,
                ),
                (
                    10u32,
                    "Alpha".to_string(),
                    10usize,
                    10usize,
                    50_000i64,
                    80.0f64,
                ),
            ],
        )];
        let windows_b = vec![(
            0usize,
            0.5f64,
            50.0f64,
            vec![
                (
                    10u32,
                    "Alpha".to_string(),
                    10usize,
                    10usize,
                    50_000i64,
                    80.0f64,
                ),
                (
                    20u32,
                    "Beta".to_string(),
                    10usize,
                    10usize,
                    50_000i64,
                    80.0f64,
                ),
            ],
        )];
        let res_a = aggregate_multi_window_matches(3, windows_a, MAX_SHIFT_DRIFT_TOLERANCE_MS);
        let res_b = aggregate_multi_window_matches(3, windows_b, MAX_SHIFT_DRIFT_TOLERANCE_MS);
        assert_eq!(res_a, res_b);
        assert_eq!(res_a[0].media_id, 10);
        assert_eq!(res_a[1].media_id, 20);
    }
}
