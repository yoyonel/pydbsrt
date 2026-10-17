//! Local disk cache for remote subtitles.
//!
//! Stores downloaded subtitles locally to conserve API quotas and avoid unnecessary network requests:
//! `~/.cache/dbsrt/subtitles/<clean_title>/<clean_title>.<sXXeXX>.<lang>.<subtitle_id>.srt`

use crate::metadata::MediaMetadata;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Disk cache manager for downloaded subtitles.
#[derive(Debug, Clone)]
pub struct SubtitleCache {
    base_dir: PathBuf,
}

impl Default for SubtitleCache {
    fn default() -> Self {
        Self::new(None)
    }
}

impl SubtitleCache {
    /// Creates a new cache instance pointing to `base_dir`, or default cache directory if `None`.
    pub fn new(base_dir: Option<PathBuf>) -> Self {
        let dir = base_dir.unwrap_or_else(|| {
            if let Some(custom) = std::env::var_os("DBSRT_CACHE_DIR") {
                PathBuf::from(custom).join("subtitles")
            } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
                PathBuf::from(xdg).join("dbsrt").join("subtitles")
            } else if let Some(home) = std::env::var_os("HOME") {
                PathBuf::from(home)
                    .join(".cache")
                    .join("dbsrt")
                    .join("subtitles")
            } else {
                PathBuf::from(".dbsrt_cache").join("subtitles")
            }
        });

        Self { base_dir: dir }
    }

    /// Returns the configured base cache directory.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Computes the disk path for a given metadata and subtitle ID.
    pub fn file_path(&self, meta: &MediaMetadata, lang: &str, subtitle_id: &str) -> PathBuf {
        let clean_title_slug = sanitize_filename(&meta.clean_title);
        let series_tag = match (meta.season, meta.episode) {
            (Some(s), Some(e)) => format!("s{:02}e{:02}", s, e),
            (Some(s), None) => format!("s{:02}", s),
            (None, Some(e)) => format!("e{:02}", e),
            (None, None) => meta
                .year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "release".to_string()),
        };

        let file_name = format!(
            "{}.{}.{}.{}.srt",
            clean_title_slug,
            series_tag,
            sanitize_filename(lang),
            sanitize_filename(subtitle_id)
        );

        self.base_dir.join(&clean_title_slug).join(file_name)
    }

    /// Checks if a cached subtitle exists on disk.
    pub fn exists(&self, meta: &MediaMetadata, lang: &str, subtitle_id: &str) -> bool {
        self.file_path(meta, lang, subtitle_id).exists()
    }

    /// Retrieves cached subtitle contents from disk if available.
    pub fn get(&self, meta: &MediaMetadata, lang: &str, subtitle_id: &str) -> Option<String> {
        let path = self.file_path(meta, lang, subtitle_id);
        if path.is_file() {
            fs::read_to_string(&path).ok()
        } else {
            None
        }
    }

    /// Writes raw subtitle contents into cache and returns the path on disk.
    pub fn put(
        &self,
        meta: &MediaMetadata,
        lang: &str,
        subtitle_id: &str,
        content: &str,
    ) -> Result<PathBuf> {
        let target_path = self.file_path(meta, lang, subtitle_id);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create cache directory {:?}", parent))?;
        }

        fs::write(&target_path, content)
            .with_context(|| format!("Failed to write cache file {:?}", target_path))?;

        Ok(target_path)
    }
}

fn sanitize_filename(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtitle_cache_lifecycle() {
        let unique_dir = std::env::temp_dir().join(format!(
            "dbsrt_cache_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = SubtitleCache::new(Some(unique_dir.clone()));

        let meta = MediaMetadata::parse("Silicon.Valley.S02E01.1080p.AMZN.WEB-DL-SiGMA.mkv");
        let lang = "fr";
        let sub_id = "987654";
        let sample_srt = "1\n00:00:01,000 --> 00:00:03,000\nHello world!\n";

        assert!(!cache.exists(&meta, lang, sub_id));
        assert!(cache.get(&meta, lang, sub_id).is_none());

        let saved = cache.put(&meta, lang, sub_id, sample_srt).unwrap();
        assert!(saved.exists());
        assert!(cache.exists(&meta, lang, sub_id));

        let retrieved = cache.get(&meta, lang, sub_id).unwrap();
        assert_eq!(retrieved, sample_srt);

        let _ = fs::remove_dir_all(&unique_dir);
    }
}
