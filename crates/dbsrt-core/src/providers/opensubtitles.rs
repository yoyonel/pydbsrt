//! OpenSubtitles REST API v1 subtitle provider.
//!
//! Implements searching and downloading subtitles via the official OpenSubtitles v1 REST API:
//! - Search endpoint: `GET https://api.opensubtitles.com/api/v1/subtitles`
//! - Download endpoint: `POST https://api.opensubtitles.com/api/v1/download`
//! - Fast scoring against local release metadata (`MediaMetadata`).

use super::{score_candidate, RemoteSubtitleCandidate, SubtitleProvider};
use crate::metadata::MediaMetadata;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_OPENSUBTITLES_BASE_URL: &str = "https://api.opensubtitles.com/api/v1";
pub const DEFAULT_USER_AGENT: &str = "dbsrt v0.2.0";
pub const SEARCH_TIMEOUT_MS: u64 = 1500;
pub const DOWNLOAD_TIMEOUT_MS: u64 = 2000;

/// OpenSubtitles REST v1 client provider.
#[derive(Debug, Clone)]
pub struct OpenSubtitlesProvider {
    client: Client,
    base_url: String,
    api_key: Option<String>,
    user_agent: String,
}

#[derive(Debug, Deserialize)]
struct OsSearchResponse {
    #[serde(default)]
    data: Vec<OsSubtitleData>,
}

#[derive(Debug, Deserialize)]
struct OsSubtitleData {
    id: Option<String>,
    attributes: Option<OsSubtitleAttributes>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OsSubtitleAttributes {
    subtitle_id: Option<String>,
    language: Option<String>,
    download_count: Option<u64>,
    hearing_impaired: Option<bool>,
    fps: Option<f64>,
    release: Option<String>,
    #[serde(default)]
    feature_details: Option<OsFeatureDetails>,
    #[serde(default)]
    files: Vec<OsFileItem>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OsFeatureDetails {
    title: Option<String>,
    year: Option<u32>,
    imdb_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OsFileItem {
    file_id: Option<u64>,
    file_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct OsDownloadRequest {
    file_id: u64,
}

#[derive(Debug, Deserialize)]
struct OsDownloadResponse {
    link: Option<String>,
    message: Option<String>,
}

impl OpenSubtitlesProvider {
    /// Creates a new OpenSubtitles client.
    ///
    /// Reads `OPEN_SUBTITLES_API_KEY` from environment if `api_key` is `None`.
    pub fn new(api_key: Option<String>) -> Self {
        let key = api_key.or_else(|| std::env::var("OPEN_SUBTITLES_API_KEY").ok());
        let client = Client::builder()
            .timeout(Duration::from_millis(DOWNLOAD_TIMEOUT_MS))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            base_url: DEFAULT_OPENSUBTITLES_BASE_URL.to_string(),
            api_key: key,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }

    /// Allows overriding base URL (e.g. for mock server or staging testing).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// Sets explicit API key.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Builds request headers including User-Agent and Api-Key.
    fn make_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&self.user_agent)
                .unwrap_or_else(|_| HeaderValue::from_static("dbsrt")),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        if let Some(ref key) = self.api_key {
            if let Ok(val) = HeaderValue::from_str(key) {
                headers.insert("Api-Key", val);
            }
        }
        headers
    }
}

#[async_trait]
impl SubtitleProvider for OpenSubtitlesProvider {
    fn name(&self) -> &'static str {
        "opensubtitles"
    }

    async fn search(
        &self,
        meta: &MediaMetadata,
        language: &str,
    ) -> Result<Vec<RemoteSubtitleCandidate>> {
        let url = format!("{}/subtitles", self.base_url);

        let mut req = self
            .client
            .get(&url)
            .headers(self.make_headers())
            .timeout(Duration::from_millis(SEARCH_TIMEOUT_MS))
            .query(&[
                ("query", meta.clean_title.as_str()),
                ("languages", language),
                ("order_by", "download_count"),
                ("order_direction", "desc"),
            ]);

        // OpenSubtitles 64-bit MovieHash if computed on local file
        if let Some(ref mh) = meta.moviehash {
            req = req.query(&[("moviehash", mh.as_str())]);
        }

        // TV Show specific parameters
        let season_str = meta.season.map(|s| s.to_string());
        let episode_str = meta.episode.map(|e| e.to_string());
        if let (Some(ref s), Some(ref e)) = (&season_str, &episode_str) {
            req = req.query(&[("season_number", s), ("episode_number", e)]);
        } else if let Some(year) = meta.year {
            let year_str = year.to_string();
            req = req.query(&[("year", &year_str)]);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to connect to OpenSubtitles at {}", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            bail!("OpenSubtitles API error (HTTP {}): {}", status, err_body);
        }

        let parsed: OsSearchResponse = resp
            .json()
            .await
            .context("Failed to deserialize OpenSubtitles JSON response")?;

        let mut candidates = Vec::new();
        for item in parsed.data {
            let sub_id = item
                .attributes
                .as_ref()
                .and_then(|a| a.subtitle_id.clone())
                .or_else(|| item.id.clone())
                .unwrap_or_default();

            if sub_id.is_empty() {
                continue;
            }

            let attrs = item.attributes.unwrap_or(OsSubtitleAttributes {
                subtitle_id: Some(sub_id.clone()),
                language: Some(language.to_string()),
                download_count: None,
                hearing_impaired: None,
                fps: None,
                release: None,
                feature_details: None,
                files: Vec::new(),
            });

            let release_name = attrs
                .release
                .clone()
                .or_else(|| attrs.files.first().and_then(|f| f.file_name.clone()));

            let download_url = if let Some(file_id) = attrs.files.first().and_then(|f| f.file_id) {
                format!("{}/download#{}", self.base_url, file_id)
            } else {
                format!("{}/subtitles/{}", self.base_url, sub_id)
            };

            let cand_title = attrs
                .feature_details
                .as_ref()
                .and_then(|f| f.title.as_deref());
            let match_score = score_candidate(meta, release_name.as_deref(), cand_title, attrs.fps);

            candidates.push(RemoteSubtitleCandidate {
                provider: "opensubtitles".to_string(),
                subtitle_id: sub_id,
                release_name,
                language: attrs.language.unwrap_or_else(|| language.to_string()),
                download_url,
                hearing_impaired: attrs.hearing_impaired.unwrap_or(false),
                fps: attrs.fps,
                match_score,
            });
        }

        // Rank candidates: highest match_score first
        candidates.sort_by(|a, b| {
            b.match_score
                .partial_cmp(&a.match_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(candidates)
    }

    async fn download(&self, candidate: &RemoteSubtitleCandidate) -> Result<String> {
        // 1. Direct URL download (if candidate already has external http direct download URL)
        if candidate.download_url.starts_with("http://")
            || candidate.download_url.starts_with("https://")
        {
            if let Some((_, file_id_str)) = candidate.download_url.split_once('#') {
                if let Ok(file_id) = file_id_str.parse::<u64>() {
                    // Call OpenSubtitles /download endpoint
                    let dl_endpoint = format!("{}/download", self.base_url);
                    let resp = self
                        .client
                        .post(&dl_endpoint)
                        .headers(self.make_headers())
                        .timeout(Duration::from_millis(DOWNLOAD_TIMEOUT_MS))
                        .json(&OsDownloadRequest { file_id })
                        .send()
                        .await
                        .with_context(|| {
                            format!("Failed to request download link from {}", dl_endpoint)
                        })?;

                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        bail!("OpenSubtitles download failed (HTTP {}): {}", status, text);
                    }

                    let dl_info: OsDownloadResponse = resp
                        .json()
                        .await
                        .context("Failed to parse OpenSubtitles download payload")?;

                    if let Some(link) = dl_info.link {
                        let content_resp = self
                            .client
                            .get(&link)
                            .timeout(Duration::from_millis(DOWNLOAD_TIMEOUT_MS))
                            .send()
                            .await
                            .with_context(|| {
                                format!("Failed to fetch subtitle content from {}", link)
                            })?;

                        return content_resp
                            .text()
                            .await
                            .context("Failed to read downloaded subtitle SRT text");
                    } else {
                        bail!(
                            "OpenSubtitles did not return download link: {:?}",
                            dl_info.message
                        );
                    }
                }
            }

            // Direct GET if no file_id anchor
            let resp = self
                .client
                .get(&candidate.download_url)
                .headers(self.make_headers())
                .timeout(Duration::from_millis(DOWNLOAD_TIMEOUT_MS))
                .send()
                .await
                .with_context(|| format!("Failed to download from {}", candidate.download_url))?;

            if !resp.status().is_success() {
                bail!("Direct download failed with HTTP {}", resp.status());
            }

            return resp
                .text()
                .await
                .context("Failed to read SRT response content");
        }

        bail!(
            "Unsupported download_url scheme for OpenSubtitles candidate: {}",
            candidate.download_url
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opensubtitles_client_initialization() {
        let provider = OpenSubtitlesProvider::new(Some("test-token-123".to_string()))
            .with_base_url("http://127.0.0.1:9999/v1");

        assert_eq!(provider.name(), "opensubtitles");
        assert_eq!(provider.base_url, "http://127.0.0.1:9999/v1");
        assert_eq!(provider.api_key.as_deref(), Some("test-token-123"));

        let headers = provider.make_headers();
        assert_eq!(
            headers.get("Api-Key").unwrap().to_str().unwrap(),
            "test-token-123"
        );
        assert_eq!(
            headers.get(USER_AGENT).unwrap().to_str().unwrap(),
            DEFAULT_USER_AGENT
        );
    }
}
