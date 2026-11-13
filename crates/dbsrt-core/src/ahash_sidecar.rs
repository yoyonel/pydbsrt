//! `.ahash` JSON sidecar (`<stem>.json`): media identity and extracted audio track topology.
//!
//! Every `.ahash` binary carries only Haitsma-Kalker sub-fingerprints: it does not say which
//! media it belongs to (16-hex MovieHash-named files) nor which audio track was decoded.
//! The sidecar closes that gap, symmetric to the `__PHASH__/<moviehash>.json` sidecar
//! ([`crate::PhashSidecar`]) and enriched with the container audio topology.
//!
//! Naming: `<stem>.json` next to the `.ahash`, where `<stem>` is the `.ahash` file name
//! without `.ahash` and without the trailing `.<fps>fps` token
//! (`05b6126884056c42.86.1fps.ahash` -> `05b6126884056c42.json`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::ahash::{AhashHeader, HEADER_SIZE_BYTES};
use crate::container::{AudioContainerInspection, AudioStreamInfo};

/// Current sidecar schema version, bumped on any breaking field change.
pub const AHASH_SIDECAR_SCHEMA_VERSION: u32 = 1;

/// `.ahash` binary file suffix (with leading dot).
pub const AHASH_FILE_SUFFIX: &str = ".ahash";

/// Sidecar file extension (without leading dot).
pub const AHASH_SIDECAR_EXTENSION: &str = "json";

/// Suffix appended to the sidecar file name for the temporary file renamed atomically.
pub const SIDECAR_TMP_SUFFIX: &str = ".tmp";

/// Trailing token of the framerate component in `.ahash` file names (`.86.1fps`).
const FPS_TOKEN_SUFFIX: &str = "fps";

/// Relative audio index FFmpeg falls back to when neither the default disposition nor the
/// channel count discriminates between tracks (first audio stream, `0:a:0`).
pub const FFMPEG_FALLBACK_AUDIO_INDEX: usize = 0;

/// Stem tag marking an explicitly selected non-default audio track (`<stem>.a<k>`).
pub const AUDIO_TRACK_STEM_TAG: &str = "a";

/// Stem of an `.ahash` extracted from a non-default audio track: `<stem>.a<k>`.
///
/// # Arguments
/// * `stem` - Base stem (MovieHash or media file name).
/// * `audio_index` - Relative audio index `k` (`0:a:k`).
///
/// # Returns
/// `"<stem>.a<k>"`, so the `.ahash` and its `<stem>.a<k>.json` sidecar never collide with
/// the default-track pair of the same media.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Survives [`ahash_logical_stem`] round-trips (`a<k>` is not a float token).
pub fn audio_track_stem(stem: &str, audio_index: usize) -> String {
    format!("{stem}.{AUDIO_TRACK_STEM_TAG}{audio_index}")
}

/// How the audio track decoded into an `.ahash` was chosen, and how certain it is.
///
/// # Data Flow & Invariants
/// * `Explicit`: `-map 0:a:<k>` was passed (`--audio-track`): certain.
/// * `SingleTrack`: no `-map`, container has exactly one audio stream: certain.
/// * `VerifiedBackfill`: multi-track backfill verified empirically via Hamming sliding
///   matching (`mean_distance <= DEFAULT_MAX_AUDIO_MATCH_DISTANCE`): certain.
/// * `FfmpegDefault`: no `-map`, several audio streams: the selected stream is *inferred*
///   with [`ffmpeg_default_audio_index`] (not observed from FFmpeg itself).
/// * `InferredBackfill`: multi-track backfill with `--no-verify` using FFmpeg default
///   inference without acoustic matching.
/// * `Unknown`: audio topology unavailable, probe failed, or no multi-track candidate matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioTrackSelection {
    /// Track forced via `-map 0:a:<k>`.
    Explicit,
    /// Container holds exactly one audio stream.
    SingleTrack,
    /// Multi-track backfill confirmed via sliding Hamming verification against `.ahash`.
    VerifiedBackfill,
    /// FFmpeg automatic selection among several streams (inferred track).
    FfmpegDefault,
    /// Multi-track backfill skipped verification (`--no-verify`), using inferred FFmpeg default.
    InferredBackfill,
    /// Audio topology unavailable, probe failed, or no multi-track candidate passed verification.
    Unknown,
}

impl AudioTrackSelection {
    /// Returns `true` when the selected audio track identity is empirically certain or verified.
    ///
    /// # Returns
    /// `true` for [`Self::Explicit`], [`Self::SingleTrack`], and [`Self::VerifiedBackfill`].
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Used by idempotent backfill (`ahash:sidecar`): only uncertain selections (`FfmpegDefault`,
    /// `InferredBackfill`, `Unknown`) may be upgraded automatically without `--force`.
    pub fn is_certain_or_verified(self) -> bool {
        matches!(
            self,
            Self::Explicit | Self::SingleTrack | Self::VerifiedBackfill
        )
    }
}

/// Selection mode plus the resolved stream metadata (when known).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAudioTrack {
    /// How the track was chosen.
    pub selection: AudioTrackSelection,
    /// Metadata of the decoded stream, `None` when unknown or not present in the probe.
    pub stream: Option<AudioStreamInfo>,
}

/// Snapshot of the `.ahash` binary header parameters stored in the sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AhashSidecarHeader {
    /// `.ahash` format version.
    pub version: u16,
    /// Decoding sample rate in Hz (e.g. 5512).
    pub sample_rate: u32,
    /// FFT frame size in samples (e.g. 2048).
    pub frame_size: u16,
    /// Hop size in samples (64 -> ~86.1 fps).
    pub hop_size: u16,
    /// Number of spectral sub-bands.
    pub num_bands: u8,
    /// Spectral derivative lag.
    pub spectral_lag: u8,
    /// Number of 64-bit hashes in the payload.
    pub frame_count: u64,
}

impl From<&AhashHeader> for AhashSidecarHeader {
    fn from(h: &AhashHeader) -> Self {
        Self {
            version: h.version,
            sample_rate: h.sample_rate,
            frame_size: h.frame_size,
            hop_size: h.hop_size,
            num_bands: h.num_bands,
            spectral_lag: h.spectral_lag,
            frame_count: h.frame_count,
        }
    }
}

/// Inputs needed to build an [`AhashSidecar`] right after (or independently of) extraction.
#[derive(Debug, Clone, Copy)]
pub struct AhashSidecarRequest<'a> {
    /// Path of the `.ahash` binary the sidecar describes.
    pub ahash_path: &'a Path,
    /// Header of that `.ahash` binary.
    pub header: &'a AhashHeader,
    /// Canonical media file name (e.g. `Film (2021).mkv`).
    pub media_name: &'a str,
    /// OpenSubtitles 16-hex MovieHash when known.
    pub moviehash: Option<&'a str>,
    /// ffprobe audio topology of the source container, `None` when unavailable.
    pub inspection: Option<&'a AudioContainerInspection>,
    /// Relative audio index passed as `-map 0:a:<k>`, `None` for FFmpeg automatic selection.
    pub explicit_audio_stream: Option<usize>,
}

/// JSON sidecar describing one `.ahash` file (see module docs for naming).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AhashSidecar {
    /// Schema version ([`AHASH_SIDECAR_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// OpenSubtitles 16-hex MovieHash when known.
    #[serde(default)]
    pub moviehash: Option<String>,
    /// Canonical media file name (key shared with the pHash sidecar).
    pub media_name: String,
    /// File name of the described `.ahash` binary.
    pub ahash_file: String,
    /// Audio sub-fingerprint rate in frames per second (`sample_rate / hop_size`).
    pub fps: f64,
    /// Number of sub-fingerprints.
    pub frames: usize,
    /// Fingerprinted duration in seconds (`frames / fps`, 0 when fps is 0).
    pub duration_sec: f64,
    /// `.ahash` header parameters.
    pub header: AhashSidecarHeader,
    /// How the decoded audio track was chosen.
    pub selection: AudioTrackSelection,
    /// Metadata of the decoded audio track, `None` when unknown.
    #[serde(default)]
    pub selected_audio_stream: Option<AudioStreamInfo>,
    /// Every audio stream present in the source container (empty when unknown).
    #[serde(default)]
    pub container_audio_streams: Vec<AudioStreamInfo>,
    /// Unix timestamp (seconds) of sidecar generation.
    pub extracted_at: u64,
    /// Optional logical media source identifier recorded by backfill orchestration (never credentials).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Optional relative path within the source repository recorded by backfill orchestration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_rel_path: Option<String>,
    /// Mean Hamming distance measured when `selection == VerifiedBackfill` (or best candidate distance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_distance: Option<f64>,
}

/// Strips the trailing `.<rate>fps` token (`.86.1fps`, `.24fps`) from a file stem.
fn strip_fps_token(stem: &str) -> &str {
    let Some(before_fps) = stem.strip_suffix(FPS_TOKEN_SUFFIX) else {
        return stem;
    };
    let Some((base, candidate)) = before_fps.rsplit_once('.') else {
        return stem;
    };
    if candidate.parse::<f64>().is_err() {
        return stem;
    }
    match base.rsplit_once('.') {
        Some((sub_base, int_part)) if int_part.parse::<f64>().is_ok() => sub_base,
        _ => base,
    }
}

/// Logical stem of an `.ahash` file name: without `.ahash` and without `.<fps>fps`.
///
/// # Arguments
/// * `file_name` - `.ahash` file name (or already-stripped stem).
///
/// # Returns
/// The stem (`05b6126884056c42.86.1fps.ahash` -> `05b6126884056c42`,
/// `Show.mkv.86.1fps.ahash` -> `Show.mkv`). Inputs without the tokens are returned unchanged.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Removes [`AHASH_FILE_SUFFIX`] when present, then a trailing `.<float>fps` token whose
/// rate may itself contain one dot (`86.1`).
pub fn ahash_logical_stem(file_name: &str) -> &str {
    let without_ext = file_name
        .strip_suffix(AHASH_FILE_SUFFIX)
        .unwrap_or(file_name);
    strip_fps_token(without_ext)
}

/// Sidecar path `<dir>/<stem>.json` of an `.ahash` path.
///
/// # Arguments
/// * `ahash_path` - Path of the `.ahash` binary (need not exist).
///
/// # Returns
/// Sibling sidecar path; a path without parent resolves relative to the current directory.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Pure path computation via [`ahash_logical_stem`]; no filesystem access.
pub fn ahash_sidecar_path(ahash_path: &Path) -> PathBuf {
    let file_name = ahash_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = ahash_logical_stem(&file_name);
    let dir = ahash_path.parent().unwrap_or_else(|| Path::new(""));
    dir.join(format!("{stem}.{AHASH_SIDECAR_EXTENSION}"))
}

/// Relative audio index FFmpeg is expected to decode when no `-map` is given.
///
/// # Arguments
/// * `streams` - Container audio streams in container order.
///
/// # Returns
/// `audio_index` of the first stream flagged default; else of the first stream with the
/// highest known channel count; else [`FFMPEG_FALLBACK_AUDIO_INDEX`].
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Approximation of FFmpeg's automatic audio stream selection, decided for this project
/// (default disposition, then channels, then first). Not verified against FFmpeg for every
/// container: sidecars built from it carry [`AudioTrackSelection::FfmpegDefault`].
/// Channel ties resolve to the earliest stream (reverse iteration + `max_by_key` keeps
/// the last maximum, i.e. the first one in container order).
pub fn ffmpeg_default_audio_index(streams: &[AudioStreamInfo]) -> usize {
    if let Some(default) = streams.iter().find(|s| s.is_default) {
        return default.audio_index;
    }
    streams
        .iter()
        .rev()
        .filter_map(|s| s.channels.map(|c| (c, s.audio_index)))
        .max_by_key(|&(channels, _)| channels)
        .map_or(FFMPEG_FALLBACK_AUDIO_INDEX, |(_, idx)| idx)
}

/// Resolves the selection mode and stream metadata of the decoded audio track.
///
/// # Arguments
/// * `streams` - Probed container audio streams (empty when unknown).
/// * `explicit` - Relative index passed as `-map 0:a:<k>`, if any.
///
/// # Returns
/// [`ResolvedAudioTrack`]: `Explicit` (stream looked up by `audio_index`), `Unknown` (no
/// streams), `SingleTrack` (one stream) or `FfmpegDefault` (inferred among several).
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// `Explicit` wins even when `streams` is empty (stream then `None`): the `-map` was applied.
pub fn resolve_audio_track(
    streams: &[AudioStreamInfo],
    explicit: Option<usize>,
) -> ResolvedAudioTrack {
    let find = |k: usize| streams.iter().find(|s| s.audio_index == k).cloned();
    let (selection, stream) = match (explicit, streams.len()) {
        (Some(k), _) => (AudioTrackSelection::Explicit, find(k)),
        (None, 0) => (AudioTrackSelection::Unknown, None),
        (None, 1) => (AudioTrackSelection::SingleTrack, streams.first().cloned()),
        (None, _) => (
            AudioTrackSelection::FfmpegDefault,
            find(ffmpeg_default_audio_index(streams)),
        ),
    };
    ResolvedAudioTrack { selection, stream }
}

/// Reads and validates only the fixed-size header of an `.ahash` file (no payload read).
///
/// # Arguments
/// * `path` - `.ahash` binary path.
///
/// # Returns
/// Parsed [`AhashHeader`].
///
/// # Errors
/// Returns `Err` if the file cannot be opened/read, is shorter than the header, or the
/// header is invalid (magic, version, bounds, length mismatch with the file size).
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Reads [`HEADER_SIZE_BYTES`] bytes and validates them against the on-disk file length,
/// so a truncated payload is still rejected without loading it.
pub fn read_ahash_header(path: &Path) -> Result<AhashHeader> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Cannot open .ahash {:?}", path))?;
    let total_len = usize::try_from(file.metadata()?.len())
        .with_context(|| format!("File too large: {:?}", path))?;
    let mut buf = [0u8; HEADER_SIZE_BYTES];
    file.read_exact(&mut buf)
        .with_context(|| format!("Cannot read .ahash header {:?}", path))?;
    AhashHeader::from_bytes(&buf, total_len)
}

impl AhashSidecar {
    /// Builds a sidecar from extraction inputs.
    ///
    /// # Arguments
    /// * `req` - Extraction context ([`AhashSidecarRequest`]).
    ///
    /// # Returns
    /// Fully populated sidecar stamped with the current Unix time.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// `frames = header.frame_count`, `fps = header.audio_fps()`, track resolved through
    /// [`resolve_audio_track`] over `req.inspection` streams (empty when `None`).
    pub fn build(req: &AhashSidecarRequest<'_>) -> Self {
        let streams: &[AudioStreamInfo] = req.inspection.map_or(&[], |i| &i.audio_streams);
        let track = resolve_audio_track(streams, req.explicit_audio_stream);
        let fps = req.header.audio_fps();
        let frames = usize::try_from(req.header.frame_count).unwrap_or(usize::MAX);
        let duration_sec = if fps > 0.0 { frames as f64 / fps } else { 0.0 };
        let extracted_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self {
            schema_version: AHASH_SIDECAR_SCHEMA_VERSION,
            moviehash: req.moviehash.map(str::to_string),
            media_name: req.media_name.to_string(),
            ahash_file: req
                .ahash_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            fps,
            frames,
            duration_sec,
            header: AhashSidecarHeader::from(req.header),
            selection: track.selection,
            selected_audio_stream: track.stream,
            container_audio_streams: streams.to_vec(),
            extracted_at,
            source_id: None,
            source_rel_path: None,
            verified_distance: None,
        }
    }

    /// Writes the sidecar atomically to `path` (`<path>.tmp` then `rename`).
    ///
    /// # Arguments
    /// * `path` - Final sidecar path.
    ///
    /// # Returns
    /// `Ok(())` once the final file is in place.
    ///
    /// # Errors
    /// Returns `Err` on serialization, write, fsync, or rename failure (the temporary file
    /// is removed best-effort).
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// The temporary file lives in the same directory, so `rename` is atomic on POSIX: a
    /// process killed mid-write (e.g. LVE limit on o2switch) leaves either the previous
    /// sidecar or a stray `.tmp`, never a truncated `<stem>.json`.
    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(SIDECAR_TMP_SUFFIX);
        let tmp_path = path.with_file_name(tmp_name);
        let body = serde_json::to_string_pretty(self)?;
        let written = (|| -> Result<()> {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp_path, path)?;
            Ok(())
        })();
        if written.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        written.with_context(|| format!("Cannot write sidecar {:?}", path))
    }

    /// Writes the sidecar next to its `.ahash` ([`ahash_sidecar_path`]).
    ///
    /// # Arguments
    /// * `ahash_path` - Path of the described `.ahash` binary.
    ///
    /// # Returns
    /// The sidecar path written.
    ///
    /// # Errors
    /// Same as [`Self::write_atomic`].
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Overwrites any existing sidecar atomically.
    pub fn write_next_to(&self, ahash_path: &Path) -> Result<PathBuf> {
        let path = ahash_sidecar_path(ahash_path);
        self.write_atomic(&path)?;
        Ok(path)
    }

    /// Parses a typed aHash sidecar.
    ///
    /// # Arguments
    /// * `path` - Sidecar JSON path.
    ///
    /// # Returns
    /// Parsed sidecar.
    ///
    /// # Errors
    /// Returns `Err` if the file is unreadable or is not a typed aHash sidecar (e.g. a
    /// legacy pHash sidecar lacking `header`/`selection`).
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Strict serde parse: callers fall back to name-only legacy parsing on `Err`.
    pub fn read(path: &Path) -> Result<Self> {
        let txt =
            fs::read_to_string(path).with_context(|| format!("Cannot read sidecar {:?}", path))?;
        serde_json::from_str(&txt).with_context(|| format!("Not an aHash sidecar {:?}", path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ahash::{
        AhashFile, HaitsmaKalkerExtractor, DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE,
        DEFAULT_SAMPLE_RATE, DEFAULT_SPECTRAL_LAG, NUM_SPECTRAL_BANDS,
    };

    const HEX: &str = "05b6126884056c42";

    fn stream(audio_index: usize, channels: Option<usize>, is_default: bool) -> AudioStreamInfo {
        AudioStreamInfo {
            index: audio_index + 1,
            audio_index,
            codec_name: "aac".into(),
            channels,
            sample_rate: Some(48_000),
            language: Some("fre".into()),
            title: None,
            is_default,
        }
    }

    fn header(frame_count: u64) -> AhashHeader {
        let (_bins, band_bounds) =
            HaitsmaKalkerExtractor::compute_band_layout(DEFAULT_SAMPLE_RATE, DEFAULT_FRAME_SIZE);
        AhashHeader {
            version: 1,
            sample_rate: DEFAULT_SAMPLE_RATE,
            frame_size: DEFAULT_FRAME_SIZE as u16,
            hop_size: DEFAULT_HOP_SIZE as u16,
            num_bands: NUM_SPECTRAL_BANDS as u8,
            spectral_lag: DEFAULT_SPECTRAL_LAG as u8,
            frame_count,
            band_bounds,
        }
    }

    #[test]
    fn test_ahash_logical_stem_and_sidecar_path() {
        assert_eq!(ahash_logical_stem(&format!("{HEX}.86.1fps.ahash")), HEX);
        assert_eq!(ahash_logical_stem("Show.mkv.86.1fps.ahash"), "Show.mkv");
        assert_eq!(
            ahash_logical_stem("Show.mkv.a1.86.1fps.ahash"),
            "Show.mkv.a1"
        );
        assert_eq!(audio_track_stem("Show.mkv", 1), "Show.mkv.a1");
        assert_eq!(ahash_logical_stem("x.24fps.ahash"), "x");
        assert_eq!(ahash_logical_stem("plain.ahash"), "plain");
        assert_eq!(ahash_logical_stem("no_ext"), "no_ext");
        assert_eq!(
            ahash_sidecar_path(Path::new(&format!("data/__AHASH__/{HEX}.86.1fps.ahash"))),
            PathBuf::from(format!("data/__AHASH__/{HEX}.json"))
        );
    }

    #[test]
    fn test_ffmpeg_default_audio_index_rule() {
        assert_eq!(ffmpeg_default_audio_index(&[]), FFMPEG_FALLBACK_AUDIO_INDEX);
        let by_default = [stream(0, Some(6), false), stream(1, Some(2), true)];
        assert_eq!(ffmpeg_default_audio_index(&by_default), 1);
        let by_channels = [stream(0, Some(2), false), stream(1, Some(6), false)];
        assert_eq!(ffmpeg_default_audio_index(&by_channels), 1);
        let tie = [stream(0, Some(6), false), stream(1, Some(6), false)];
        assert_eq!(ffmpeg_default_audio_index(&tie), 0);
        let unknown = [stream(0, None, false), stream(1, None, false)];
        assert_eq!(
            ffmpeg_default_audio_index(&unknown),
            FFMPEG_FALLBACK_AUDIO_INDEX
        );
    }

    #[test]
    fn test_resolve_audio_track_modes() {
        let two = [stream(0, Some(2), false), stream(1, Some(6), false)];
        let r = resolve_audio_track(&two, Some(0));
        assert_eq!(r.selection, AudioTrackSelection::Explicit);
        assert_eq!(r.stream.unwrap().audio_index, 0);
        let r = resolve_audio_track(&two, None);
        assert_eq!(r.selection, AudioTrackSelection::FfmpegDefault);
        assert_eq!(r.stream.unwrap().audio_index, 1);
        let r = resolve_audio_track(&two[..1], None);
        assert_eq!(r.selection, AudioTrackSelection::SingleTrack);
        assert!(r.stream.is_some());
        assert_eq!(
            resolve_audio_track(&[], None).selection,
            AudioTrackSelection::Unknown
        );
        assert!(AudioTrackSelection::Explicit.is_certain_or_verified());
        assert!(AudioTrackSelection::SingleTrack.is_certain_or_verified());
        assert!(AudioTrackSelection::VerifiedBackfill.is_certain_or_verified());
        assert!(!AudioTrackSelection::FfmpegDefault.is_certain_or_verified());
        assert!(!AudioTrackSelection::InferredBackfill.is_certain_or_verified());
        assert!(!AudioTrackSelection::Unknown.is_certain_or_verified());
        let r = resolve_audio_track(&[], Some(3));
        assert_eq!(r.selection, AudioTrackSelection::Explicit);
        assert!(r.stream.is_none());
    }

    #[test]
    fn test_sidecar_build_write_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let ahash_path = dir.path().join(format!("{HEX}.86.1fps.ahash"));
        let hdr = header(4);
        AhashFile::write_to_path(&ahash_path, &hdr, &[1, 2, 3, 4]).unwrap();
        let inspection = AudioContainerInspection {
            path: PathBuf::from("film.mkv"),
            audio_streams: vec![stream(0, Some(2), false), stream(1, Some(6), true)],
        };
        let sidecar = AhashSidecar::build(&AhashSidecarRequest {
            ahash_path: &ahash_path,
            header: &read_ahash_header(&ahash_path).unwrap(),
            media_name: "Film (2021).mkv",
            moviehash: Some(HEX),
            inspection: Some(&inspection),
            explicit_audio_stream: None,
        });
        assert_eq!(sidecar.frames, 4);
        assert_eq!(sidecar.selection, AudioTrackSelection::FfmpegDefault);
        assert_eq!(
            sidecar.selected_audio_stream.as_ref().unwrap().audio_index,
            1
        );
        assert_eq!(sidecar.container_audio_streams.len(), 2);

        let written = sidecar.write_next_to(&ahash_path).unwrap();
        assert_eq!(written, dir.path().join(format!("{HEX}.json")));
        assert!(!dir
            .path()
            .join(format!("{HEX}.json{SIDECAR_TMP_SUFFIX}"))
            .exists());
        let back = AhashSidecar::read(&written).unwrap();
        assert_eq!(back, sidecar);
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&written).unwrap()).unwrap();
        assert_eq!(raw["selection"], "ffmpeg_default");
        assert_eq!(raw["media_name"], "Film (2021).mkv");
    }

    #[test]
    fn test_legacy_phash_sidecar_is_not_typed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{HEX}.json"));
        fs::write(
            &path,
            r#"{"moviehash":"x","media_name":"M","fps":24.0,"frames":1}"#,
        )
        .unwrap();
        assert!(AhashSidecar::read(&path).is_err());
    }

    #[test]
    fn test_read_ahash_header_rejects_truncated_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.86.1fps.ahash");
        AhashFile::write_to_path(&path, &header(2), &[7, 8]).unwrap();
        assert_eq!(read_ahash_header(&path).unwrap().frame_count, 2);
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(read_ahash_header(&path).is_err());
    }
}
