//! Hermetic in-memory mock provider for unit testing and offline development.

use super::{RemoteSubtitleCandidate, SubtitleProvider};
use crate::metadata::MediaMetadata;
use anyhow::{bail, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type CandidateMap = HashMap<(String, String), Vec<RemoteSubtitleCandidate>>;
type CandidateStore = Arc<Mutex<CandidateMap>>;
type ContentStore = Arc<Mutex<HashMap<String, String>>>;

/// In-memory mock subtitle provider for hermetic testing.
#[derive(Debug, Clone, Default)]
pub struct MockSubtitleProvider {
    /// In-memory registered candidates indexed by `(query_title_lower, language)`
    candidates: CandidateStore,
    /// In-memory subtitle content indexed by `subtitle_id`
    contents: ContentStore,
    /// Optional error to return on search
    search_error: Arc<Mutex<Option<String>>>,
    /// Optional error to return on download
    download_error: Arc<Mutex<Option<String>>>,
}

impl MockSubtitleProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a mock provider pre-populated with standard testing and demonstration fixtures.
    pub fn with_default_fixtures() -> Self {
        let provider = Self::default();
        let sv_cand_fr = RemoteSubtitleCandidate {
            provider: "mock".to_string(),
            subtitle_id: "mock-sv-fr-1".to_string(),
            release_name: Some("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA".to_string()),
            language: "fr".to_string(),
            download_url: "mock://silicon-valley-s02e01-fr".to_string(),
            hearing_impaired: false,
            fps: Some(23.976),
            match_score: 0.95,
        };
        let sv_srt_fr = "1\n00:00:01,000 --> 00:00:04,000\nPied Piper en français\n";
        provider.add_candidate("silicon valley", sv_cand_fr, sv_srt_fr);

        let sv_cand_en = RemoteSubtitleCandidate {
            provider: "mock".to_string(),
            subtitle_id: "mock-sv-en-1".to_string(),
            release_name: Some("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA".to_string()),
            language: "en".to_string(),
            download_url: "mock://silicon-valley-s02e01-en".to_string(),
            hearing_impaired: false,
            fps: Some(23.976),
            match_score: 0.98,
        };
        let sv_srt_en = "1\n00:00:01,000 --> 00:00:04,000\nPied Piper in English\n";
        provider.add_candidate("silicon valley", sv_cand_en, sv_srt_en);

        let bbb_cand_en = RemoteSubtitleCandidate {
            provider: "mock".to_string(),
            subtitle_id: "mock-bbb-en-1".to_string(),
            release_name: Some("Big.Buck.Bunny.1080p".to_string()),
            language: "en".to_string(),
            download_url: "mock://big-buck-bunny-en".to_string(),
            hearing_impaired: false,
            fps: Some(24.0),
            match_score: 0.92,
        };
        let bbb_srt_en = "1\n00:00:01,000 --> 00:00:04,000\nBig Buck Bunny English\n";
        provider.add_candidate("big buck bunny", bbb_cand_en, bbb_srt_en);

        provider
    }

    /// Adds a mock candidate and its associated SRT content.
    pub fn add_candidate(
        &self,
        title_query: &str,
        candidate: RemoteSubtitleCandidate,
        srt_content: &str,
    ) {
        let key = (
            title_query.to_lowercase(),
            candidate.language.to_lowercase(),
        );
        let mut c_map = self.candidates.lock().unwrap();
        let sub_id = candidate.subtitle_id.clone();
        c_map.entry(key).or_default().push(candidate);
        drop(c_map);

        let mut cnt_map = self.contents.lock().unwrap();
        cnt_map.insert(sub_id, srt_content.to_string());
    }

    /// Injects an artificial error on `search` calls.
    pub fn set_search_error(&self, err_msg: Option<&str>) {
        let mut err = self.search_error.lock().unwrap();
        *err = err_msg.map(str::to_string);
    }

    /// Injects an artificial error on `download` calls.
    pub fn set_download_error(&self, err_msg: Option<&str>) {
        let mut err = self.download_error.lock().unwrap();
        *err = err_msg.map(str::to_string);
    }
}

#[async_trait]
impl SubtitleProvider for MockSubtitleProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn search(
        &self,
        meta: &MediaMetadata,
        language: &str,
    ) -> Result<Vec<RemoteSubtitleCandidate>> {
        if let Some(ref err) = *self.search_error.lock().unwrap() {
            bail!("Mock search error: {}", err);
        }

        let clean_title_lower = meta.clean_title.to_lowercase();
        let lang_lower = language.to_lowercase();

        let c_map = self.candidates.lock().unwrap();
        let candidates = c_map
            .get(&(clean_title_lower, lang_lower))
            .cloned()
            .unwrap_or_default();

        Ok(candidates)
    }

    async fn download(&self, candidate: &RemoteSubtitleCandidate) -> Result<String> {
        if let Some(ref err) = *self.download_error.lock().unwrap() {
            bail!("Mock download error: {}", err);
        }

        let cnt_map = self.contents.lock().unwrap();
        if let Some(content) = cnt_map.get(&candidate.subtitle_id) {
            Ok(content.clone())
        } else {
            bail!("Mock subtitle ID '{}' not found", candidate.subtitle_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_search_and_download() {
        let provider = MockSubtitleProvider::new();
        let meta = MediaMetadata::parse("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA.mkv");

        let candidate = RemoteSubtitleCandidate {
            provider: "mock".to_string(),
            subtitle_id: "mock-123".to_string(),
            release_name: Some("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA".to_string()),
            language: "fr".to_string(),
            download_url: "mock://123".to_string(),
            hearing_impaired: false,
            fps: Some(23.976),
            match_score: 0.95,
        };

        let srt_text = "1\n00:00:01,000 --> 00:00:04,000\nPied Piper\n";
        provider.add_candidate("silicon valley", candidate.clone(), srt_text);

        let results = provider.search(&meta, "fr").await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].subtitle_id, "mock-123");

        let downloaded = provider.download(&results[0]).await.unwrap();
        assert_eq!(downloaded, srt_text);
    }

    #[tokio::test]
    async fn test_mock_provider_errors() {
        let provider = MockSubtitleProvider::new();
        let meta = MediaMetadata::parse("Test.Movie.2024.mkv");

        provider.set_search_error(Some("Rate limit exceeded 429"));
        let search_res = provider.search(&meta, "en").await;
        assert!(search_res.is_err());
        assert!(search_res.unwrap_err().to_string().contains("429"));

        provider.set_search_error(None);
        let candidate = RemoteSubtitleCandidate {
            provider: "mock".to_string(),
            subtitle_id: "err-456".to_string(),
            release_name: None,
            language: "en".to_string(),
            download_url: "mock://456".to_string(),
            hearing_impaired: false,
            fps: None,
            match_score: 0.5,
        };

        provider.set_download_error(Some("Network timeout 504"));
        let dl_res = provider.download(&candidate).await;
        assert!(dl_res.is_err());
        assert!(dl_res.unwrap_err().to_string().contains("504"));
    }
}
