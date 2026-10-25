//! Media container inspection and embedded subtitle extraction.
//!
//! Provides inspection of MKV, MP4, and other multimedia containers via FFprobe
//! to detect internal subtitle tracks (SubRip/SRT, ASS/SSA, MOV_TEXT) with
//! ISO 639-1 / 639-2 language tags, and memory-only streaming extraction via FFmpeg.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::media_pairing::parse_language_code;

/// Metadata describing an embedded subtitle track within a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleStreamInfo {
    /// Absolute stream index within the container (e.g. 2 for 0:2)
    pub index: usize,
    /// Relative 0-based subtitle stream index (e.g. 0 for 0:s:0)
    pub subtitle_index: usize,
    /// Subtitle codec name (e.g. "subrip", "ass", "mov_text", "webvtt")
    pub codec_name: String,
    /// ISO language tag if specified (e.g. "fra", "fre", "eng", "fr")
    pub language: Option<String>,
    /// Stream title or description if present in metadata
    pub title: Option<String>,
    /// True if the stream is marked as default in container disposition
    pub is_default: bool,
    /// True if the stream is marked as forced in container disposition
    pub is_forced: bool,
}

/// Inspection summary for a multimedia container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerInspection {
    /// File path of the inspected media
    pub path: PathBuf,
    /// List of detected embedded subtitle streams
    pub subtitle_streams: Vec<SubtitleStreamInfo>,
}

impl ContainerInspection {
    /// Searches for the first subtitle stream matching the requested language tag
    /// using ISO 639-1 / 639-2 normalization (e.g. "fr" matches "fra" or "fre").
    pub fn find_by_language(&self, lang: &str) -> Option<&SubtitleStreamInfo> {
        self.subtitle_streams
            .iter()
            .find(|stream| language_matches(stream.language.as_deref(), lang))
    }

    /// Returns the default subtitle track if designated in the container metadata.
    pub fn default_subtitle(&self) -> Option<&SubtitleStreamInfo> {
        self.subtitle_streams
            .iter()
            .find(|stream| stream.is_default)
            .or_else(|| self.subtitle_streams.first())
    }
}

#[derive(Deserialize)]
struct FfprobeSubOutput {
    streams: Option<Vec<FfprobeSubStream>>,
}

#[derive(Deserialize)]
struct FfprobeSubStream {
    index: Option<usize>,
    codec_name: Option<String>,
    tags: Option<FfprobeSubTags>,
    disposition: Option<FfprobeSubDisposition>,
}

#[derive(Deserialize)]
struct FfprobeSubTags {
    language: Option<String>,
    title: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeSubDisposition {
    default: Option<u32>,
    forced: Option<u32>,
}

/// Inspects a video/media container file using ffprobe to detect embedded subtitle tracks.
pub fn inspect_container<P: AsRef<Path>>(path: P) -> Result<ContainerInspection> {
    let p = path.as_ref();
    if !p.exists() {
        return Err(anyhow!("Media container not found: {:?}", p));
    }

    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "s",
            "-show_entries",
            "stream=index,codec_name:stream_disposition=default,forced:stream_tags=language,title",
            "-of",
            "json",
        ])
        .arg(p)
        .output()
        .with_context(|| format!("Failed to execute ffprobe on {:?}", p))?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffprobe inspection failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let parsed: FfprobeSubOutput = serde_json::from_slice(&output.stdout)
        .with_context(|| "Failed to parse ffprobe json output")?;

    let mut subtitle_streams = Vec::new();

    if let Some(streams) = parsed.streams {
        for (sub_idx, stream) in streams.into_iter().enumerate() {
            let codec_name = stream.codec_name.unwrap_or_else(|| "unknown".to_string());
            let (language, title) = if let Some(tags) = stream.tags {
                (tags.language, tags.title)
            } else {
                (None, None)
            };

            let (is_default, is_forced) = if let Some(disp) = stream.disposition {
                (
                    disp.default.unwrap_or(0) == 1,
                    disp.forced.unwrap_or(0) == 1,
                )
            } else {
                (false, false)
            };

            subtitle_streams.push(SubtitleStreamInfo {
                index: stream.index.unwrap_or(sub_idx),
                subtitle_index: sub_idx,
                codec_name,
                language,
                title,
                is_default,
                is_forced,
            });
        }
    }

    Ok(ContainerInspection {
        path: p.to_path_buf(),
        subtitle_streams,
    })
}

/// Extracts an embedded subtitle track directly into memory as a SubRip (SRT) string.
///
/// Executes FFmpeg stream copy / demuxing to stdout without disk I/O and without
/// video/audio re-encoding overhead.
///
/// # Arguments
/// * `media_path` - Path to the video or container file
/// * `subtitle_index` - 0-based relative index of the subtitle stream (corresponds to FFmpeg `-map 0:s:<subtitle_index>`)
pub fn extract_embedded_subtitle<P: AsRef<Path>>(
    media_path: P,
    subtitle_index: usize,
) -> Result<String> {
    let p = media_path.as_ref();
    if !p.exists() {
        return Err(anyhow!("Media container not found: {:?}", p));
    }

    let map_arg = format!("0:s:{}", subtitle_index);

    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(p)
        .args(["-map", &map_arg, "-f", "srt", "-"])
        .output()
        .with_context(|| format!("Failed to execute ffmpeg subtitle extraction on {:?}", p))?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffmpeg extraction failed for stream {}: {}",
            subtitle_index,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let srt_content = String::from_utf8(output.stdout)
        .context("Extracted subtitle content is not valid UTF-8")?;

    Ok(srt_content)
}

/// Checks whether a stream language code matches the requested language.
///
/// Supports ISO 639-1 (2-letter) and ISO 639-2 (3-letter bibliographic & terminology) codes.
pub fn language_matches(stream_lang: Option<&str>, target_lang: &str) -> bool {
    let sl = match stream_lang {
        Some(s) => s.trim().to_lowercase(),
        None => return false,
    };
    let tl = target_lang.trim().to_lowercase();

    if sl == tl {
        return true;
    }

    if let (Some(n1), Some(n2)) = (parse_language_code(&sl), parse_language_code(&tl)) {
        if n1 == n2 {
            return true;
        }
    }

    // Common multilingual equivalents
    let synonyms: &[&[&str]] = &[
        &["fr", "fre", "fra", "french", "francais"],
        &["en", "eng", "english"],
        &["es", "spa", "spanish", "espanol"],
        &["de", "deu", "ger", "german", "deutsch"],
        &["it", "ita", "italian", "italiano"],
        &["pt", "por", "portuguese"],
        &["ja", "jpn", "japanese"],
        &["zh", "zho", "chi", "chinese"],
        &["ru", "rus", "russian"],
        &["ko", "kor", "korean"],
        &["nl", "dut", "nld", "dutch"],
        &["ar", "ara", "arabic"],
    ];

    for group in synonyms {
        let has_sl = group.contains(&sl.as_str());
        let has_tl = group.contains(&tl.as_str());
        if has_sl && has_tl {
            return true;
        }
    }

    false
}
