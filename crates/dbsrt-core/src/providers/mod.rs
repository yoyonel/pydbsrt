//! Subtitle providers and remote fetching abstractions (Axe 5).
//!
//! Provides a unified async trait [`SubtitleProvider`], metadata-driven candidate scoring,
//! and remote subtitle representations.

pub mod cache;
pub mod mock;
pub mod opensubtitles;

pub use cache::SubtitleCache;
pub use mock::MockSubtitleProvider;
pub use opensubtitles::OpenSubtitlesProvider;

use crate::metadata::MediaMetadata;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A subtitle candidate returned by a remote provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSubtitleCandidate {
    /// Identifier of the provider origin (e.g. "opensubtitles", "mock")
    pub provider: String,
    /// Unique identifier of the subtitle on the remote platform
    pub subtitle_id: String,
    /// Release name associated with the subtitle (e.g. "Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA")
    pub release_name: Option<String>,
    /// Normalized language code (ISO 639-1 / 639-2, e.g. "fr", "en")
    pub language: String,
    /// Direct download URL or file endpoint
    pub download_url: String,
    /// Subtitle for the deaf and hard of hearing (SDH)
    pub hearing_impaired: bool,
    /// Target nominal framerate if specified by provider (e.g. 23.976, 25.0)
    pub fps: Option<f64>,
    /// Heuristic match score against local media release (0.0 to 1.0)
    pub match_score: f32,
}

/// Core asynchronous abstraction for remote subtitle providers.
#[async_trait]
pub trait SubtitleProvider: Send + Sync {
    /// Human-readable provider name.
    fn name(&self) -> &'static str;

    /// Searches for matching subtitle candidates using parsed media metadata.
    async fn search(
        &self,
        meta: &MediaMetadata,
        language: &str,
    ) -> Result<Vec<RemoteSubtitleCandidate>>;

    /// Downloads the raw SRT subtitle content in-memory.
    async fn download(&self, candidate: &RemoteSubtitleCandidate) -> Result<String>;
}

/// Heuristic scoring of a remote candidate against local parsed media metadata.
///
/// Weights:
/// - Rejects (score 0.0) if candidate movie title does not match target clean title
/// - +0.4 if release group matches (e.g. "SiGMA")
/// - +0.3 if release source matches (e.g. "WEB-DL", "BluRay")
/// - +0.2 if resolution matches (e.g. "1080p")
/// - +0.1 baseline if language matches and title contains keywords
pub fn score_candidate(
    meta: &MediaMetadata,
    release_name: Option<&str>,
    candidate_title: Option<&str>,
    candidate_fps: Option<f64>,
) -> f32 {
    // 0. Verify feature/movie title match if candidate title is known
    if let Some(cand_t) = candidate_title {
        let cand_clean: String = cand_t
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        let query_clean: String = meta
            .clean_title
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();

        if !cand_clean.is_empty()
            && !query_clean.is_empty()
            && !cand_clean.contains(&query_clean)
            && !query_clean.contains(&cand_clean)
        {
            return 0.0;
        }
    }

    let mut score = 0.1f32; // Baseline match
    let rel_lower = release_name.map(|s| s.to_lowercase()).unwrap_or_default();

    if rel_lower.is_empty() {
        return score;
    }

    // 1. Release group (+0.4)
    if let Some(ref grp) = meta.release_group {
        let grp_lower = grp.to_lowercase();
        if rel_lower.contains(&grp_lower) {
            score += 0.4;
        }
    }

    // 2. Source (+0.3)
    if let Some(ref src) = meta.source {
        let src_lower = src.to_lowercase();
        if rel_lower.contains(&src_lower) {
            score += 0.3;
        }
    }

    // 3. Resolution (+0.2)
    if let Some(ref res) = meta.resolution {
        let res_lower = res.to_lowercase();
        if rel_lower.contains(&res_lower) {
            score += 0.2;
        }
    }

    // 4. Framerate hint bonus (if specified and close to 23.976/24/25)
    if let Some(fps) = candidate_fps {
        if fps > 23.0 && fps < 60.0 {
            score = (score + 0.05).min(1.0);
        }
    }

    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::MediaMetadata;

    #[test]
    fn test_score_candidate_exact_release_group() {
        let meta = MediaMetadata::parse("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA.mkv");
        let release = "Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA";

        let score = score_candidate(&meta, Some(release), Some("Silicon Valley"), Some(23.976));
        // 0.1 + 0.4 (SiGMA) + 0.3 (WEB-DL) + 0.2 (1080p) + 0.05 (fps) = 1.0 (clamped)
        assert!(score >= 0.95);
    }

    #[test]
    fn test_score_candidate_partial_match() {
        let meta = MediaMetadata::parse("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA.mkv");
        let release = "Silicon.Valley.S02E01.720p.HDTV.x264-KILLERS";

        let score = score_candidate(&meta, Some(release), Some("Silicon Valley"), None);
        // Does not match SiGMA, WEB-DL or 1080p -> only baseline 0.1
        assert_eq!(score, 0.1);
    }

    #[test]
    fn test_score_candidate_mismatched_title_rejected() {
        let meta = MediaMetadata::parse("The.Hyperions.2022.1080p.WEBRip.x264-RARBG.mp4");
        let release = "WEBRip.x264-RARBG";

        // Even though release group matches RARBG and source WEBRip, title is different
        let score = score_candidate(
            &meta,
            Some(release),
            Some("La revanche des Crevettes Pailletées"),
            None,
        );
        assert_eq!(score, 0.0);
    }
}
