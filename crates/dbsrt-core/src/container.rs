//! Media container inspection and embedded subtitle extraction.
//!
//! Provides inspection of MKV, MP4, and other multimedia containers via FFprobe
//! to detect internal subtitle tracks (SubRip/SRT, ASS/SSA, MOV_TEXT) with
//! ISO 639-1 / 639-2 language tags, and memory-only streaming extraction via FFmpeg.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ffmpeg_input::{
    apply_ftp_credentials, display_input, ffmpeg_decode_threads, redact_url_userinfo,
};
use crate::media_pairing::parse_language_code;

/// Metadata describing an embedded audio track within a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioStreamInfo {
    /// Absolute stream index within the container (e.g. 1 for 0:1).
    pub index: usize,
    /// Relative 0-based audio stream index (e.g. 0 for 0:a:0).
    pub audio_index: usize,
    /// Audio codec name (e.g. "aac", "ac3", "flac", "mp3").
    pub codec_name: String,
    /// Number of audio channels (e.g. 2 for stereo, 6 for 5.1).
    pub channels: Option<usize>,
    /// Audio sample rate in Hz (e.g. 48000).
    pub sample_rate: Option<u32>,
    /// ISO language tag if specified (e.g. "fra", "fre", "eng", "jpn").
    pub language: Option<String>,
    /// Stream title or description if present in metadata.
    pub title: Option<String>,
    /// True if the stream is marked as default in container disposition.
    pub is_default: bool,
}

/// Inspection summary for audio streams in a multimedia container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioContainerInspection {
    /// File path of the inspected media.
    pub path: PathBuf,
    /// List of detected audio streams.
    pub audio_streams: Vec<AudioStreamInfo>,
}

impl AudioContainerInspection {
    /// Returns total count of audio streams.
    #[inline]
    pub fn track_count(&self) -> usize {
        self.audio_streams.len()
    }

    /// Returns language code of the first audio stream if present.
    pub fn track0_lang(&self) -> Option<String> {
        self.audio_streams.first().and_then(|s| s.language.clone())
    }

    /// Returns title of the first audio stream if present.
    pub fn track0_title(&self) -> Option<String> {
        self.audio_streams.first().and_then(|s| s.title.clone())
    }

    /// Returns codec name of the first audio stream if present.
    pub fn track0_codec(&self) -> Option<String> {
        self.audio_streams.first().map(|s| s.codec_name.clone())
    }

    /// Returns channel count of the first audio stream if present.
    pub fn track0_channels(&self) -> Option<usize> {
        self.audio_streams.first().and_then(|s| s.channels)
    }

    /// Returns all declared stream language tags.
    pub fn all_languages(&self) -> Vec<String> {
        self.audio_streams
            .iter()
            .filter_map(|s| s.language.clone())
            .collect()
    }

    /// True if container contains more than 1 audio track.
    #[inline]
    pub fn is_multi_track(&self) -> bool {
        self.audio_streams.len() > 1
    }
}

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
struct FfprobeAudioOutput {
    streams: Option<Vec<FfprobeAudioStream>>,
}

#[derive(Deserialize)]
struct FfprobeAudioStream {
    index: Option<usize>,
    codec_name: Option<String>,
    channels: Option<usize>,
    sample_rate: Option<String>,
    tags: Option<FfprobeSubTags>,
    disposition: Option<FfprobeSubDisposition>,
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

/// Returns true if the path represents a remote streaming URL or Unix pipe rather than a local file.
///
/// # Arguments
/// * `path` - Input filesystem path or stream URL.
///
/// # Returns
/// * `bool` - True if path starts with a recognized network protocol (`ftp://`, `http://`, etc.) or pipe syntax.
///
/// # Invariants
/// * Never panics. Zero allocation on typical static strings.
pub fn is_remote_stream_or_pipe<P: AsRef<Path>>(path: P) -> bool {
    let s = path.as_ref().to_string_lossy();
    s.starts_with("ftp://")
        || s.starts_with("ftps://")
        || s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("sftp://")
        || s.starts_with("pipe:")
        || s == "-"
}

/// Returns true for a pipe input (`pipe:N` or `-`): ffprobe would consume it, so never probe it.
pub fn is_pipe_input<P: AsRef<Path>>(path: P) -> bool {
    let s = path.as_ref().to_string_lossy();
    s.starts_with("pipe:") || s == "-"
}

/// Inspects a video/media container file using ffprobe to detect embedded subtitle tracks.
pub fn inspect_container<P: AsRef<Path>>(path: P) -> Result<ContainerInspection> {
    let p = path.as_ref();
    if !is_remote_stream_or_pipe(p) && !p.exists() {
        return Err(anyhow!("Media container not found: {:?}", display_input(p)));
    }

    let ffprobe_bin = crate::video::resolve_ffprobe();
    let mut cmd = Command::new(ffprobe_bin);
    cmd.args(["-threads", &ffmpeg_decode_threads()]);
    apply_ftp_credentials(&mut cmd, p);
    let output = cmd
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
        .with_context(|| format!("Failed to execute ffprobe on {:?}", display_input(p)))?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffprobe inspection failed: {}",
            redact_url_userinfo(&String::from_utf8_lossy(&output.stderr))
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

/// Inspects a multimedia container file using ffprobe to detect embedded audio tracks.
///
/// # Arguments
/// * `path` - Input filesystem path or stream URL.
///
/// # Returns
/// An [`AudioContainerInspection`] containing detected audio streams.
///
/// # Errors
/// Returns error if ffprobe fails to execute or input file does not exist.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Queries ffprobe for `-select_streams a` with audio stream metadata.
/// 2. Parses audio codec, channels, sample rate, language tags, and disposition flags.
/// 3. Normalizes missing fields to defaults (0-based stream indexing).
pub fn inspect_audio_streams<P: AsRef<Path>>(path: P) -> Result<AudioContainerInspection> {
    let p = path.as_ref();
    if !is_remote_stream_or_pipe(p) && !p.exists() {
        return Err(anyhow!("Media container not found: {:?}", display_input(p)));
    }

    let ffprobe_bin = crate::video::resolve_ffprobe();
    let mut cmd = Command::new(ffprobe_bin);
    cmd.args(["-threads", &ffmpeg_decode_threads()]);
    apply_ftp_credentials(&mut cmd, p);
    let output = cmd
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index,codec_name,channels,sample_rate:stream_disposition=default:stream_tags=language,title",
            "-of",
            "json",
        ])
        .arg(p)
        .output()
        .with_context(|| format!("Failed to execute ffprobe on {:?}", display_input(p)))?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffprobe audio inspection failed: {}",
            redact_url_userinfo(&String::from_utf8_lossy(&output.stderr))
        ));
    }

    let parsed: FfprobeAudioOutput = serde_json::from_slice(&output.stdout)
        .with_context(|| "Failed to parse ffprobe json output")?;

    let mut audio_streams = Vec::new();
    if let Some(streams) = parsed.streams {
        for (a_idx, stream) in streams.into_iter().enumerate() {
            let codec_name = stream.codec_name.unwrap_or_else(|| "unknown".to_string());
            let (language, title) = if let Some(tags) = stream.tags {
                (tags.language, tags.title)
            } else {
                (None, None)
            };
            let is_default = stream
                .disposition
                .and_then(|d| d.default)
                .map(|val| val == 1)
                .unwrap_or(false);
            let sample_rate = stream.sample_rate.and_then(|s| s.parse::<u32>().ok());

            audio_streams.push(AudioStreamInfo {
                index: stream.index.unwrap_or(a_idx),
                audio_index: a_idx,
                codec_name,
                channels: stream.channels,
                sample_rate,
                language,
                title,
                is_default,
            });
        }
    }

    Ok(AudioContainerInspection {
        path: p.to_path_buf(),
        audio_streams,
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
    if !is_remote_stream_or_pipe(p) && !p.exists() {
        return Err(anyhow!("Media container not found: {:?}", display_input(p)));
    }

    let map_arg = format!("0:s:{}", subtitle_index);

    let ffmpeg_bin = crate::video::resolve_ffmpeg_cpu();
    let mut cmd = Command::new(ffmpeg_bin);
    cmd.args(["-v", "error", "-y", "-threads", &ffmpeg_decode_threads()]);
    apply_ftp_credentials(&mut cmd, p);
    let output = cmd
        .arg("-i")
        .arg(p)
        .args(["-map", &map_arg, "-f", "srt", "-"])
        .output()
        .with_context(|| {
            format!(
                "Failed to execute ffmpeg subtitle extraction on {:?}",
                display_input(p)
            )
        })?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffmpeg extraction failed for stream {}: {}",
            subtitle_index,
            redact_url_userinfo(&String::from_utf8_lossy(&output.stderr))
        ));
    }

    let srt_content = crate::srt::decode_subtitle_bytes(&output.stdout);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_remote_stream_or_pipe() {
        assert!(is_remote_stream_or_pipe(Path::new(
            "ftp://user:pass@example.com/video.mkv"
        )));
        assert!(is_remote_stream_or_pipe(Path::new(
            "ftps://example.com/video.mp4"
        )));
        assert!(is_remote_stream_or_pipe(Path::new(
            "http://127.0.0.1:8080/stream.webm"
        )));
        assert!(is_remote_stream_or_pipe(Path::new(
            "https://example.com/file.mkv"
        )));
        assert!(is_remote_stream_or_pipe(Path::new(
            "sftp://nas.local/share/media.mkv"
        )));
        assert!(is_remote_stream_or_pipe(Path::new("pipe:0")));
        assert!(is_remote_stream_or_pipe(Path::new("-")));

        assert!(!is_remote_stream_or_pipe(Path::new("/var/media/video.mkv")));
        assert!(!is_remote_stream_or_pipe(Path::new("relative/video.mp4")));
        assert!(!is_remote_stream_or_pipe(Path::new("sample.mkv")));
    }

    #[test]
    fn test_is_pipe_input() {
        assert!(is_pipe_input(Path::new("pipe:0")));
        assert!(is_pipe_input(Path::new("-")));
        assert!(!is_pipe_input(Path::new("http://host/movie.mkv")));
        assert!(!is_pipe_input(Path::new("/var/media/video.mkv")));
    }

    #[test]
    fn test_audio_container_inspection_helpers() {
        let inspection = AudioContainerInspection {
            path: PathBuf::from("test.mkv"),
            audio_streams: vec![
                AudioStreamInfo {
                    index: 1,
                    audio_index: 0,
                    codec_name: "aac".to_string(),
                    channels: Some(6),
                    sample_rate: Some(48000),
                    language: Some("fre".to_string()),
                    title: Some("VF".to_string()),
                    is_default: true,
                },
                AudioStreamInfo {
                    index: 2,
                    audio_index: 1,
                    codec_name: "ac3".to_string(),
                    channels: Some(2),
                    sample_rate: Some(48000),
                    language: Some("jpn".to_string()),
                    title: Some("VO".to_string()),
                    is_default: false,
                },
            ],
        };

        assert_eq!(inspection.track_count(), 2);
        assert!(inspection.is_multi_track());
        assert_eq!(inspection.track0_lang().as_deref(), Some("fre"));
        assert_eq!(inspection.track0_title().as_deref(), Some("VF"));
        assert_eq!(inspection.track0_codec().as_deref(), Some("aac"));
        assert_eq!(inspection.track0_channels(), Some(6));
        assert_eq!(
            inspection.all_languages(),
            vec!["fre".to_string(), "jpn".to_string()]
        );
    }
}
