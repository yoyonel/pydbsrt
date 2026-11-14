//! `.ahash` sidecar backfill & multi-track Hamming verification.
//!
//! Populates missing `<stem>.json` sidecars for existing `.ahash` binaries and upgrades
//! uncertain multi-track sidecars (`ffmpeg_default`, `inferred_backfill`, `unknown`) to
//! [`AudioTrackSelection::VerifiedBackfill`] via 5s midpoint FFmpeg probing (`+-2s` search).

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::ahash::{
    is_silent_or_flat_audio_query, match_audio_sequence, AhashFile, AhashHeader,
    DEFAULT_MAX_AUDIO_MATCH_DISTANCE, HASH_TOTAL_BITS, MIN_AUDIO_QUERY_FRAMES,
};
use crate::ahash_sidecar::{
    ahash_sidecar_path, read_ahash_header, AhashSidecar, AhashSidecarRequest, AudioTrackSelection,
};
use crate::audio::{extract_ahash, AudioExtractionOptions};
use crate::container::{inspect_audio_streams, AudioContainerInspection, AudioStreamInfo};

/// Probe duration sampled from each track on multi-track containers (`5.0s`).
pub const BACKFILL_VERIFY_WINDOW_SEC: f64 = 5.0;

/// Local sliding cross-correlation radius around the nominal probe offset (`+-2.0s`).
pub const BACKFILL_VERIFY_RADIUS_SEC: f64 = 2.0;

/// Minimum `.ahash` duration above which full midpoint `5.0s` probing applies unscaled.
pub const BACKFILL_MIN_FULL_CLIP_SEC: f64 = 12.0;

/// Midpoint ratio of the `.ahash` timeline where track verification samples first (`t = 50%`).
pub const BACKFILL_MIDPOINT_RATIO: f64 = 0.5;

/// Early fallback timeline ratio probed when `t = 50%` is silent or M&E-ambiguous (`t = 25%`).
pub const BACKFILL_FALLBACK_EARLY_RATIO: f64 = 0.25;

/// Late fallback timeline ratio probed when `t = 50%` and `t = 25%` are silent or M&E-ambiguous (`t = 75%`).
pub const BACKFILL_FALLBACK_LATE_RATIO: f64 = 0.75;

/// Ordered timeline anchor ratios probed during multi-track verification (`50%`, then `25%`, `75%`).
pub const BACKFILL_VERIFY_ANCHOR_RATIOS: [f64; 3] = [
    BACKFILL_MIDPOINT_RATIO,
    BACKFILL_FALLBACK_EARLY_RATIO,
    BACKFILL_FALLBACK_LATE_RATIO,
];

/// Minimum Hamming distance margin (in bits) required between the best and second-best audio
/// tracks to certify [`AudioTrackSelection::VerifiedBackfill`] (prevents false track election
/// on shared M&E music/SFX passages).
pub const MIN_TRACK_DISCRIMINATION_MARGIN_BITS: f64 = 4.0;

/// Fraction of total duration sampled on short clips below [`BACKFILL_MIN_FULL_CLIP_SEC`].
pub const BACKFILL_SHORT_CLIP_WINDOW_RATIO: f64 = 0.6;

/// Lower clamp on probe duration (seconds) so tiny clips still yield enough FFT frames.
pub const BACKFILL_MIN_QUERY_SEC: f64 = 0.8;

/// Minimum start-time difference (seconds) between two anchor windows to avoid re-probing an identical slice.
const MIN_ANCHOR_START_DELTA_SEC: f64 = 0.1;

/// Permissive Hamming bound (`> 64.0`) passed to capture raw best distance for diagnostics.
const DIAGNOSTIC_UNBOUNDED_DISTANCE: f64 = (HASH_TOTAL_BITS as f64) + 1.0;

/// Computed temporal probe and local reference window for multi-track verification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerifyWindow {
    /// Start seek position (`-ss`) in seconds on the source container.
    pub start_sec: f64,
    /// Duration (`-t`) in seconds to decode from each candidate audio track.
    pub duration_sec: f64,
    /// Inclusive start frame index in the reference `.ahash` payload.
    pub ref_start_frame: usize,
    /// Exclusive end frame index in the reference `.ahash` payload.
    pub ref_end_frame: usize,
}

/// Request inputs for [`backfill_ahash_sidecar`].
#[derive(Debug, Clone, Copy)]
pub struct BackfillSidecarRequest<'a> {
    /// Existing `.ahash` binary path.
    pub ahash_path: &'a Path,
    /// Source media path or stream URI (`ftp://`, `http(s)://`).
    pub media: &'a Path,
    /// Canonical media file name recorded in the sidecar.
    pub media_name: &'a str,
    /// Optional 16-hex OpenSubtitles MovieHash.
    pub moviehash: Option<&'a str>,
    /// Optional logical media source identifier (e.g. `synology_tvshow`).
    pub source_id: Option<&'a str>,
    /// Optional relative path within the source repository.
    pub source_rel_path: Option<&'a str>,
    /// Skip acoustic verification on multi-track containers (`inferred_backfill`).
    pub no_verify: bool,
    /// Re-run even when existing sidecar is already `explicit`, `single_track`, or `verified_backfill`.
    pub force: bool,
}

/// Outcome disposition of [`backfill_ahash_sidecar`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillAction {
    /// Existing sidecar is already certain/verified and `--force` was not requested.
    Skipped,
    /// Sidecar was created or upgraded atomically on disk.
    Written,
}

/// Per-track verification measurement recorded during multi-track probing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackVerificationProbe {
    /// Relative audio stream index (`0:a:<k>`).
    pub audio_index: usize,
    /// Stream language tag if present.
    pub language: Option<String>,
    /// Best local sliding Hamming distance vs `.ahash`, `None` if probe/decode failed.
    pub mean_distance: Option<f64>,
    /// Non-fatal error description if probing this candidate failed.
    pub error: Option<String>,
}

/// Structured result of running [`backfill_ahash_sidecar`].
#[derive(Debug, Clone, PartialEq)]
pub struct BackfillSidecarOutcome {
    /// Whether the sidecar was skipped or written.
    pub action: BackfillAction,
    /// Path to `<stem>.json` beside the `.ahash`.
    pub sidecar_path: PathBuf,
    /// Existing (when skipped) or newly written [`AhashSidecar`].
    pub sidecar: AhashSidecar,
    /// Per-track verification probes (empty for single-track / `--no-verify` / skipped).
    pub probes: Vec<TrackVerificationProbe>,
}

/// Computes the midpoint probe window and `+-2.0s` reference bounds with short-clip clamping.
///
/// # Arguments
/// * `header` - Parsed `.ahash` header (`sample_rate`, `hop_size`).
/// * `frame_count` - Number of 64-bit hashes available in the `.ahash` payload.
///
/// # Returns
/// [`VerifyWindow`] bounded inside `[0, frame_count]` at [`BACKFILL_MIDPOINT_RATIO`].
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Delegates to [`compute_verify_window_at_ratio`] with [`BACKFILL_MIDPOINT_RATIO`] (`0.50`).
pub fn compute_verify_window(header: &AhashHeader, frame_count: usize) -> VerifyWindow {
    compute_verify_window_at_ratio(header, frame_count, BACKFILL_MIDPOINT_RATIO)
}

/// Computes a probe window at `anchor_ratio` along the `.ahash` timeline and `+-2.0s` reference bounds.
///
/// # Arguments
/// * `header` - Parsed `.ahash` header (`sample_rate`, `hop_size`).
/// * `frame_count` - Number of 64-bit hashes available in the `.ahash` payload.
/// * `anchor_ratio` - Fractional timeline anchor (e.g. `0.50`, `0.25`, `0.75`), clamped to `[0.0, 1.0]`.
///
/// # Returns
/// [`VerifyWindow`] bounded inside `[0, frame_count]`.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// * For durations `>= 12.0s`, extracts `5.0s` starting at `t = anchor_ratio * total_sec` (clamped).
/// * For shorter clips (`< 12.0s`), scales duration to `60%` of the clip clamped to `[0.8s, 5.0s]`
///   and positions it at `anchor_ratio * max_start`.
/// * Reference bounds expand around `[start_sec, start_sec + duration_sec]` by `+-2.0s`.
pub fn compute_verify_window_at_ratio(
    header: &AhashHeader,
    frame_count: usize,
    anchor_ratio: f64,
) -> VerifyWindow {
    let fps = header.audio_fps();
    if fps <= 0.0 || frame_count == 0 {
        return VerifyWindow {
            start_sec: 0.0,
            duration_sec: 0.0,
            ref_start_frame: 0,
            ref_end_frame: frame_count,
        };
    }

    let ratio = anchor_ratio.clamp(0.0, 1.0);
    let total_sec = (frame_count as f64) / fps;
    let duration_sec = if total_sec >= BACKFILL_MIN_FULL_CLIP_SEC {
        BACKFILL_VERIFY_WINDOW_SEC
    } else {
        let min_q = BACKFILL_MIN_QUERY_SEC.min(total_sec);
        (total_sec * BACKFILL_SHORT_CLIP_WINDOW_RATIO).clamp(min_q, BACKFILL_VERIFY_WINDOW_SEC)
    };

    let max_start = (total_sec - duration_sec).max(0.0);
    let nominal_start = if total_sec >= BACKFILL_MIN_FULL_CLIP_SEC {
        total_sec * ratio
    } else {
        max_start * ratio
    };
    let start_sec = nominal_start.clamp(0.0, max_start);

    let ref_start_sec = (start_sec - BACKFILL_VERIFY_RADIUS_SEC).max(0.0);
    let ref_end_sec = (start_sec + duration_sec + BACKFILL_VERIFY_RADIUS_SEC).min(total_sec);
    let ref_start_frame = ((ref_start_sec * fps).floor() as usize).min(frame_count);
    let ref_end_frame = ((ref_end_sec * fps).ceil() as usize).clamp(ref_start_frame, frame_count);

    VerifyWindow {
        start_sec,
        duration_sec,
        ref_start_frame,
        ref_end_frame,
    }
}

/// Checks if an existing sidecar is already certain/verified and should be skipped.
fn check_skip_existing(sidecar_path: &Path, force: bool) -> Option<AhashSidecar> {
    if force {
        return None;
    }
    let existing = AhashSidecar::read(sidecar_path).ok()?;
    existing
        .selection
        .is_certain_or_verified()
        .then_some(existing)
}

/// Evaluates an extracted candidate hash sequence against a reference `.ahash` window,
/// rejecting short (`< MIN_AUDIO_QUERY_FRAMES`), silent (`0u64`), or flat-energy sequences.
pub(crate) fn match_candidate_hashes_against_window(
    query_hashes: &[u64],
    ahash_file: &AhashFile,
    window: VerifyWindow,
    stream: &AudioStreamInfo,
) -> TrackVerificationProbe {
    if query_hashes.len() < MIN_AUDIO_QUERY_FRAMES {
        return TrackVerificationProbe {
            audio_index: stream.audio_index,
            language: stream.language.clone(),
            mean_distance: None,
            error: Some(format!(
                "query window too short ({} frames < {MIN_AUDIO_QUERY_FRAMES})",
                query_hashes.len()
            )),
        };
    }
    if is_silent_or_flat_audio_query(query_hashes) {
        return TrackVerificationProbe {
            audio_index: stream.audio_index,
            language: stream.language.clone(),
            mean_distance: None,
            error: Some("silent or flat-energy query window".to_string()),
        };
    }

    let mut reference_slice = &ahash_file.hashes[window.ref_start_frame..window.ref_end_frame];
    if reference_slice.len() < query_hashes.len() {
        reference_slice = &ahash_file.hashes;
    }
    if is_silent_or_flat_audio_query(reference_slice) {
        return TrackVerificationProbe {
            audio_index: stream.audio_index,
            language: stream.language.clone(),
            mean_distance: None,
            error: Some("silent or flat-energy reference window".to_string()),
        };
    }

    let fps = ahash_file.header.audio_fps();
    let matched = match_audio_sequence(
        query_hashes,
        reference_slice,
        fps,
        DIAGNOSTIC_UNBOUNDED_DISTANCE,
    );
    TrackVerificationProbe {
        audio_index: stream.audio_index,
        language: stream.language.clone(),
        mean_distance: matched.map(|m| m.mean_distance),
        error: None,
    }
}

/// Probes one candidate audio stream and compares it locally against the reference `.ahash`.
fn probe_candidate_track(
    media: &Path,
    ahash_file: &AhashFile,
    window: VerifyWindow,
    stream: &AudioStreamInfo,
) -> TrackVerificationProbe {
    let opts = AudioExtractionOptions {
        sample_rate: ahash_file.header.sample_rate,
        frame_size: usize::from(ahash_file.header.frame_size),
        hop_size: usize::from(ahash_file.header.hop_size),
        start_sec: window.start_sec,
        duration_sec: Some(window.duration_sec),
        custom_ffmpeg: None,
        audio_stream: Some(stream.audio_index),
    };
    let extracted = match extract_ahash(media, &opts) {
        Ok(file) if !file.hashes.is_empty() => file,
        Ok(_) => {
            return TrackVerificationProbe {
                audio_index: stream.audio_index,
                language: stream.language.clone(),
                mean_distance: None,
                error: Some("empty query hashes extracted".to_string()),
            };
        }
        Err(err) => {
            return TrackVerificationProbe {
                audio_index: stream.audio_index,
                language: stream.language.clone(),
                mean_distance: None,
                error: Some(format!("{err:#}")),
            };
        }
    };

    match_candidate_hashes_against_window(&extracted.hashes, ahash_file, window, stream)
}

/// Classification of one anchor probe pass across all candidate tracks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AnchorProbeVerdict {
    /// Unambiguous winning track `(audio_index, distance)` (`best_dist <= 14.0` and margin `>= 4.0` bits).
    Verified(usize, f64),
    /// Anchor was silent/flat or two tracks matched within `< 4.0` bits (e.g. shared M&E passage).
    AmbiguousOrSilent(Option<f64>),
    /// All tracks decoded non-silent audio above `DEFAULT_MAX_AUDIO_MATCH_DISTANCE`.
    Unmatched(Option<f64>),
}

/// Evaluates per-track probe distances at a single temporal anchor, enforcing both
/// [`DEFAULT_MAX_AUDIO_MATCH_DISTANCE`] and [`MIN_TRACK_DISCRIMINATION_MARGIN_BITS`].
pub(crate) fn classify_anchor_probes(probes: &[TrackVerificationProbe]) -> AnchorProbeVerdict {
    let mut ranked: Vec<(usize, f64)> = probes
        .iter()
        .filter_map(|p| p.mean_distance.map(|d| (p.audio_index, d)))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let Some(&(best_idx, best_dist)) = ranked.first() else {
        return AnchorProbeVerdict::AmbiguousOrSilent(None);
    };

    if best_dist > DEFAULT_MAX_AUDIO_MATCH_DISTANCE {
        return AnchorProbeVerdict::Unmatched(Some(best_dist));
    }

    if let Some(&(_, second_dist)) = ranked.get(1) {
        if (second_dist - best_dist) < MIN_TRACK_DISCRIMINATION_MARGIN_BITS {
            return AnchorProbeVerdict::AmbiguousOrSilent(Some(best_dist));
        }
        return AnchorProbeVerdict::Verified(best_idx, best_dist);
    }

    // Only one track produced a valid distance in a multi-track container: accept only if all other
    // tracks were rejected as silent/flat (not because of a transient FFmpeg I/O failure).
    let others_silent_only = probes
        .iter()
        .filter(|p| p.audio_index != best_idx)
        .all(|p| {
            p.error
                .as_deref()
                .is_some_and(|e| e.contains("silent or flat-energy"))
        });
    if others_silent_only {
        AnchorProbeVerdict::Verified(best_idx, best_dist)
    } else {
        AnchorProbeVerdict::AmbiguousOrSilent(Some(best_dist))
    }
}

/// Internal outcome of probing all candidate tracks in a multi-track container.
struct MultiTrackVerification {
    selection: AudioTrackSelection,
    stream: Option<AudioStreamInfo>,
    distance: Option<f64>,
    probes: Vec<TrackVerificationProbe>,
}

/// Updates `best_raw` with `candidate` when `candidate` is smaller than the current minimum.
#[inline]
fn update_min_distance(best_raw: &mut Option<f64>, candidate: Option<f64>) {
    let Some(d) = candidate else {
        return;
    };
    if best_raw.is_none_or(|prev| d < prev) {
        *best_raw = Some(d);
    }
}

/// Probes all candidate audio streams of a container at a single verification window.
fn probe_all_tracks_at_window(
    media: &Path,
    ahash_file: &AhashFile,
    window: VerifyWindow,
    streams: &[AudioStreamInfo],
) -> Vec<TrackVerificationProbe> {
    streams
        .iter()
        .map(|stream| probe_candidate_track(media, ahash_file, window, stream))
        .collect()
}

/// Builds a certified [`MultiTrackVerification`] result for a winning `(audio_index, distance)` pair.
fn build_verified_outcome(
    inspection: &AudioContainerInspection,
    winner: (usize, f64),
    probes: Vec<TrackVerificationProbe>,
) -> MultiTrackVerification {
    let (audio_index, distance) = winner;
    let chosen = inspection
        .audio_streams
        .iter()
        .find(|s| s.audio_index == audio_index)
        .cloned();
    MultiTrackVerification {
        selection: AudioTrackSelection::VerifiedBackfill,
        stream: chosen,
        distance: Some(distance),
        probes,
    }
}

/// Evaluates all container audio tracks against `ahash_path`, falling back from `50%` to `25%` / `75%`
/// when the midpoint anchor is silent or M&E-ambiguous.
fn verify_multitrack_selection(
    req: &BackfillSidecarRequest<'_>,
    inspection: &AudioContainerInspection,
) -> Result<MultiTrackVerification> {
    let ahash_file = AhashFile::read_from_path(req.ahash_path)?;
    let mut visited_starts: Vec<f64> = Vec::with_capacity(BACKFILL_VERIFY_ANCHOR_RATIOS.len());
    let mut best_raw: Option<f64> = None;
    let mut last_probes: Vec<TrackVerificationProbe> = Vec::new();

    for &ratio in &BACKFILL_VERIFY_ANCHOR_RATIOS {
        let window =
            compute_verify_window_at_ratio(&ahash_file.header, ahash_file.hashes.len(), ratio);
        let already_probed = visited_starts
            .iter()
            .any(|&prev| (prev - window.start_sec).abs() < MIN_ANCHOR_START_DELTA_SEC);
        if already_probed {
            continue;
        }
        visited_starts.push(window.start_sec);

        let probes =
            probe_all_tracks_at_window(req.media, &ahash_file, window, &inspection.audio_streams);

        match classify_anchor_probes(&probes) {
            AnchorProbeVerdict::Verified(audio_index, distance) => {
                return Ok(build_verified_outcome(
                    inspection,
                    (audio_index, distance),
                    probes,
                ));
            }
            AnchorProbeVerdict::AmbiguousOrSilent(best_distance) => {
                update_min_distance(&mut best_raw, best_distance);
                last_probes = probes;
            }
            AnchorProbeVerdict::Unmatched(best_distance) => {
                update_min_distance(&mut best_raw, best_distance);
                last_probes = probes;
                break;
            }
        }
    }

    Ok(MultiTrackVerification {
        selection: AudioTrackSelection::Unknown,
        stream: None,
        distance: best_raw,
        probes: last_probes,
    })
}

/// Attaches backfill provenance metadata onto a sidecar.
fn attach_source_metadata(sidecar: &mut AhashSidecar, req: &BackfillSidecarRequest<'_>) {
    sidecar.source_id = req.source_id.map(str::to_string);
    sidecar.source_rel_path = req.source_rel_path.map(str::to_string);
}

/// Creates or upgrades the `<stem>.json` sidecar of an existing `.ahash` file.
///
/// # Arguments
/// * `req` - Backfill parameters ([`BackfillSidecarRequest`]).
///
/// # Returns
/// [`BackfillSidecarOutcome`] reporting whether the sidecar was skipped or atomically written,
/// plus any multi-track verification distances.
///
/// # Errors
/// Returns `Err` if `.ahash` cannot be read/parsed, `ffprobe` fails on `req.media`, or atomic
/// JSON serialization fails.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Idempotence: skips when `<stem>.json` already exists with `explicit`, `single_track`, or
///    `verified_backfill` unless `req.force` is set. Allows automatic upgrade from
///    `ffmpeg_default`, `inferred_backfill`, or `unknown`.
/// 2. Reads `.ahash` 176-byte header and inspects container audio streams via `ffprobe`.
/// 3. Single-track containers write `single_track` with zero audio decode.
/// 4. Multi-track containers either infer (`inferred_backfill` when `no_verify`) or sample
///    `5.0s` at `t = 50%` per stream (`-map 0:a:k`) and accept `verified_backfill` only when
///    the lowest sliding Hamming distance in `+-2.0s` satisfies `<= 14.0` (otherwise `unknown`).
/// 5. Writes `<stem>.json.<pid>.tmp` -> `fsync` -> `rename` atomically.
pub fn backfill_ahash_sidecar(req: &BackfillSidecarRequest<'_>) -> Result<BackfillSidecarOutcome> {
    let sidecar_path = ahash_sidecar_path(req.ahash_path);
    if let Some(existing) = check_skip_existing(&sidecar_path, req.force) {
        return Ok(BackfillSidecarOutcome {
            action: BackfillAction::Skipped,
            sidecar_path,
            sidecar: existing,
            probes: Vec::new(),
        });
    }

    let header = read_ahash_header(req.ahash_path)?;
    let inspection = inspect_audio_streams(req.media).with_context(|| {
        format!(
            "Failed probing audio streams for backfill of {:?}",
            req.ahash_path
        )
    })?;

    let mut sidecar = AhashSidecar::build(&AhashSidecarRequest {
        ahash_path: req.ahash_path,
        header: &header,
        media_name: req.media_name,
        moviehash: req.moviehash,
        inspection: Some(&inspection),
        explicit_audio_stream: None,
    });
    attach_source_metadata(&mut sidecar, req);

    let mut probes = Vec::new();
    if inspection.audio_streams.len() > 1 {
        if req.no_verify {
            sidecar.selection = AudioTrackSelection::InferredBackfill;
        } else {
            let res = verify_multitrack_selection(req, &inspection)?;
            sidecar.selection = res.selection;
            sidecar.selected_audio_stream = res.stream;
            sidecar.verified_distance = res.distance;
            probes = res.probes;
        }
    }

    let written_path = sidecar.write_next_to(req.ahash_path)?;
    Ok(BackfillSidecarOutcome {
        action: BackfillAction::Written,
        sidecar_path: written_path,
        sidecar,
        probes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ahash::{
        HaitsmaKalkerExtractor, DEFAULT_FRAME_SIZE, DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE,
        DEFAULT_SPECTRAL_LAG, NUM_SPECTRAL_BANDS,
    };
    use std::process::Command;

    const HEX: &str = "05b6126884056c42";
    const SYNTH_DURATION_SEC: &str = "3";

    fn synth_header(frame_count: u64) -> AhashHeader {
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

    fn make_two_track_mkv(dir: &Path) -> Option<PathBuf> {
        let out = dir.join("dual_track.mkv");
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1318:duration=3",
                "-map",
                "0:a",
                "-map",
                "1:a",
                "-metadata:s:a:0",
                "language=fre",
                "-metadata:s:a:1",
                "language=jpn",
                "-disposition:a:0",
                "0",
                "-disposition:a:1",
                "default",
                "-c:a",
                "aac",
                "-t",
                SYNTH_DURATION_SEC,
                "-y",
            ])
            .arg(&out)
            .status()
            .ok()?;
        (status.success() && out.exists()).then_some(out)
    }

    fn make_single_track_mkv(dir: &Path) -> Option<PathBuf> {
        let out = dir.join("single_track.mkv");
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=523:duration=3",
                "-metadata:s:a:0",
                "language=eng",
                "-c:a",
                "aac",
                "-t",
                SYNTH_DURATION_SEC,
                "-y",
            ])
            .arg(&out)
            .status()
            .ok()?;
        (status.success() && out.exists()).then_some(out)
    }

    #[test]
    fn test_compute_verify_window_full_and_short_clips() {
        let hdr = synth_header(0);
        let fps = hdr.audio_fps();

        // 1200s clip (>= 12s): samples 5.0s at t = 600.0s, search [598.0s .. 607.0s]
        let long_frames = (1200.0 * fps).round() as usize;
        let long_win = compute_verify_window(&hdr, long_frames);
        assert!((long_win.start_sec - 600.0).abs() < 0.05);
        assert!((long_win.duration_sec - BACKFILL_VERIFY_WINDOW_SEC).abs() < f64::EPSILON);
        assert!(long_win.ref_start_frame < long_win.ref_end_frame);
        assert!(long_win.ref_end_frame <= long_frames);

        // 3s clip (< 12s): clamped duration = 1.8s centered at 0.6s, bounds within [0..frames]
        let short_frames = (3.0 * fps).round() as usize;
        let short_win = compute_verify_window(&hdr, short_frames);
        assert!((short_win.duration_sec - 1.8).abs() < 0.05);
        assert!((short_win.start_sec - 0.6).abs() < 0.05);
        assert_eq!(short_win.ref_start_frame, 0);
        assert_eq!(short_win.ref_end_frame, short_frames);
    }

    #[test]
    fn test_backfill_single_track_and_idempotence() {
        let dir = tempfile::tempdir().unwrap();
        let Some(media) = make_single_track_mkv(dir.path()) else {
            eprintln!("Skipping: ffmpeg lavfi unavailable");
            return;
        };
        let extracted = extract_ahash(&media, &AudioExtractionOptions::default()).unwrap();
        let ahash_path = dir.path().join(format!("{HEX}.86.1fps.ahash"));
        AhashFile::write_to_path(&ahash_path, &extracted.header, &extracted.hashes).unwrap();

        let req = BackfillSidecarRequest {
            ahash_path: &ahash_path,
            media: &media,
            media_name: "single_track.mkv",
            moviehash: Some(HEX),
            source_id: Some("test_local"),
            source_rel_path: Some("single_track.mkv"),
            no_verify: false,
            force: false,
        };
        let first = backfill_ahash_sidecar(&req).unwrap();
        assert_eq!(first.action, BackfillAction::Written);
        assert_eq!(first.sidecar.selection, AudioTrackSelection::SingleTrack);
        assert!(first.probes.is_empty());
        assert_eq!(first.sidecar.source_id.as_deref(), Some("test_local"));
        assert_eq!(
            first
                .sidecar
                .selected_audio_stream
                .as_ref()
                .and_then(|s| s.language.as_deref()),
            Some("eng")
        );

        let second = backfill_ahash_sidecar(&req).unwrap();
        assert_eq!(second.action, BackfillAction::Skipped);
    }

    #[test]
    fn test_backfill_dual_track_verifies_exact_stream_and_upgrades_inferred() {
        let dir = tempfile::tempdir().unwrap();
        let Some(media) = make_two_track_mkv(dir.path()) else {
            eprintln!("Skipping: ffmpeg lavfi unavailable");
            return;
        };

        // Extract specifically track 0 (fre, 440Hz), which is NOT the FFmpeg default track 1 (jpn, 1318Hz)!
        let opts_t0 = AudioExtractionOptions {
            audio_stream: Some(0),
            ..Default::default()
        };
        let extracted_t0 = extract_ahash(&media, &opts_t0).unwrap();
        let ahash_path = dir.path().join("dual_track.mkv.86.1fps.ahash");
        AhashFile::write_to_path(&ahash_path, &extracted_t0.header, &extracted_t0.hashes).unwrap();

        // First run with --no-verify: gets InferredBackfill (default track 1, jpn)
        let inferred_req = BackfillSidecarRequest {
            ahash_path: &ahash_path,
            media: &media,
            media_name: "dual_track.mkv",
            moviehash: None,
            source_id: None,
            source_rel_path: None,
            no_verify: true,
            force: false,
        };
        let inferred = backfill_ahash_sidecar(&inferred_req).unwrap();
        assert_eq!(inferred.action, BackfillAction::Written);
        assert_eq!(
            inferred.sidecar.selection,
            AudioTrackSelection::InferredBackfill
        );
        assert_eq!(
            inferred.sidecar.selected_audio_stream.unwrap().audio_index,
            1
        );

        // Next run with verification (without --force!): must UPGRADE inferred_backfill -> verified_backfill (track 0, fre)!
        let verify_req = BackfillSidecarRequest {
            no_verify: false,
            ..inferred_req
        };
        let verified = backfill_ahash_sidecar(&verify_req).unwrap();
        assert_eq!(verified.action, BackfillAction::Written);
        assert_eq!(
            verified.sidecar.selection,
            AudioTrackSelection::VerifiedBackfill
        );
        assert_eq!(verified.probes.len(), 2);
        let chosen = verified.sidecar.selected_audio_stream.unwrap();
        assert_eq!(chosen.audio_index, 0);
        assert_eq!(chosen.language.as_deref(), Some("fre"));
        assert!(verified.sidecar.verified_distance.unwrap() <= DEFAULT_MAX_AUDIO_MATCH_DISTANCE);

        // Subsequent call without --force must skip because VerifiedBackfill is certain
        let skipped = backfill_ahash_sidecar(&verify_req).unwrap();
        assert_eq!(skipped.action, BackfillAction::Skipped);
    }

    #[test]
    fn test_backfill_dual_track_unmatched_records_unknown_without_lying() {
        let dir = tempfile::tempdir().unwrap();
        let Some(dual) = make_two_track_mkv(dir.path()) else {
            eprintln!("Skipping: ffmpeg lavfi unavailable");
            return;
        };
        let Some(unrelated) = make_single_track_mkv(dir.path()) else {
            return;
        };
        let unrelated_ahash =
            extract_ahash(&unrelated, &AudioExtractionOptions::default()).unwrap();
        let ahash_path = dir.path().join("mismatch.mkv.86.1fps.ahash");
        AhashFile::write_to_path(
            &ahash_path,
            &unrelated_ahash.header,
            &unrelated_ahash.hashes,
        )
        .unwrap();

        let res = backfill_ahash_sidecar(&BackfillSidecarRequest {
            ahash_path: &ahash_path,
            media: &dual,
            media_name: "dual_track.mkv",
            moviehash: None,
            source_id: None,
            source_rel_path: None,
            no_verify: false,
            force: false,
        })
        .unwrap();
        assert_eq!(res.action, BackfillAction::Written);
        assert_eq!(res.sidecar.selection, AudioTrackSelection::Unknown);
        assert!(res.sidecar.selected_audio_stream.is_none());
    }

    fn make_identical_me_two_track_mkv(dir: &Path) -> Option<PathBuf> {
        let out = dir.join("identical_me.mkv");
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
                "-map",
                "0:a",
                "-map",
                "0:a",
                "-metadata:s:a:0",
                "language=fre",
                "-metadata:s:a:1",
                "language=eng",
                "-c:a",
                "aac",
                "-t",
                SYNTH_DURATION_SEC,
                "-y",
            ])
            .arg(&out)
            .status()
            .ok()?;
        (status.success() && out.exists()).then_some(out)
    }

    #[test]
    fn test_backfill_rejects_silence_short_windows_and_ambiguous_me_tracks() {
        let stream = AudioStreamInfo {
            index: 1,
            audio_index: 0,
            codec_name: "aac".to_string(),
            channels: Some(2),
            sample_rate: Some(48_000),
            language: Some("fre".to_string()),
            title: None,
            is_default: true,
        };
        let hdr = synth_header(64);
        let ref_hashes: Vec<u64> = (0..64)
            .map(|i| 0xA5A5_5A5A_0000_0000u64 ^ ((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)))
            .collect();
        let ahash_file = AhashFile {
            header: hdr,
            hashes: ref_hashes,
        };
        let win = VerifyWindow {
            start_sec: 0.0,
            duration_sec: 0.5,
            ref_start_frame: 0,
            ref_end_frame: 64,
        };

        // 1-frame query (< MIN_AUDIO_QUERY_FRAMES) rejected
        let short_probe =
            match_candidate_hashes_against_window(&[0x1234_5678], &ahash_file, win, &stream);
        assert!(short_probe.mean_distance.is_none());
        assert!(short_probe.error.as_deref().unwrap().contains("too short"));

        // Digital silence (0u64) rejected even if reference were also 0u64
        let silent_probe =
            match_candidate_hashes_against_window(&[0u64; 16], &ahash_file, win, &stream);
        assert!(silent_probe.mean_distance.is_none());
        assert!(silent_probe
            .error
            .as_deref()
            .unwrap()
            .contains("silent or flat-energy"));

        // Flat constant energy rejected
        let flat_probe = match_candidate_hashes_against_window(
            &[0xDEAD_BEEF_CAFE_BABEu64; 16],
            &ahash_file,
            win,
            &stream,
        );
        assert!(flat_probe.mean_distance.is_none());

        // M&E ambiguity: VF (1.2 bits) vs VO (2.5 bits) -> margin 1.3 < 4.0 -> AmbiguousOrSilent
        let me_probes = vec![
            TrackVerificationProbe {
                audio_index: 0,
                language: Some("fre".to_string()),
                mean_distance: Some(1.2),
                error: None,
            },
            TrackVerificationProbe {
                audio_index: 1,
                language: Some("eng".to_string()),
                mean_distance: Some(2.5),
                error: None,
            },
        ];
        assert_eq!(
            classify_anchor_probes(&me_probes),
            AnchorProbeVerdict::AmbiguousOrSilent(Some(1.2))
        );

        // Clear margin: VF (1.2 bits) vs VO (18.0 bits) -> margin 16.8 >= 4.0 -> Verified
        let distinct_probes = vec![
            me_probes[0].clone(),
            TrackVerificationProbe {
                audio_index: 1,
                language: Some("eng".to_string()),
                mean_distance: Some(18.0),
                error: None,
            },
        ];
        assert_eq!(
            classify_anchor_probes(&distinct_probes),
            AnchorProbeVerdict::Verified(0, 1.2)
        );

        // End-to-end M&E identical dual-track MKV must fall back to Unknown instead of certifying wrong track
        let dir = tempfile::tempdir().unwrap();
        let Some(me_media) = make_identical_me_two_track_mkv(dir.path()) else {
            return;
        };
        let extracted = extract_ahash(&me_media, &AudioExtractionOptions::default()).unwrap();
        let ahash_path = dir.path().join("identical_me.mkv.86.1fps.ahash");
        AhashFile::write_to_path(&ahash_path, &extracted.header, &extracted.hashes).unwrap();
        let outcome = backfill_ahash_sidecar(&BackfillSidecarRequest {
            ahash_path: &ahash_path,
            media: &me_media,
            media_name: "identical_me.mkv",
            moviehash: None,
            source_id: None,
            source_rel_path: None,
            no_verify: false,
            force: false,
        })
        .unwrap();
        assert_eq!(outcome.sidecar.selection, AudioTrackSelection::Unknown);
        assert!(outcome.sidecar.selected_audio_stream.is_none());
    }
}
