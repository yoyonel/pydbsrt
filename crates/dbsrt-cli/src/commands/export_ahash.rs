//! Haitsma-Kalker audio spectral hash (.ahash) extraction CLI command.
//!
//! Writes the `.ahash` binary and its `<stem>.json` sidecar (media identity + audio topology,
//! see [`dbsrt_core::AhashSidecar`]).

use anyhow::Result;
use clap::Args;
use dbsrt_core::{
    audio_track_stem, extract_ahash, ffmpeg_default_audio_index, AhashFile, AhashSidecar,
    AhashSidecarRequest, AudioContainerInspection, AudioExtractionOptions, DEFAULT_FRAME_SIZE,
    DEFAULT_HOP_SIZE,
};
use std::path::PathBuf;
use std::time::Instant;

use crate::commands::shared::{
    parse_moviehash, probe_audio_topology, resolve_media_path_filename, resolve_moviehash,
};
use crate::constants::{
    DEFAULT_AHASH_HOP_SIZE, DEFAULT_AHASH_SAMPLE_RATE, DIVISION_BY_ZERO_FALLBACK_SEC,
};

/// Default output directory when `--output` is omitted.
const DEFAULT_AHASH_OUTPUT_DIR: &str = "/tmp";

/// Command-line arguments for extracting Haitsma-Kalker audio hashes.
#[derive(Args, Debug, Clone)]
pub struct ExportAhashArgs {
    /// Media file path or stream URI to extract audio hash from.
    #[arg(short, long)]
    pub media: PathBuf,

    /// Output .ahash file path (defaults to `/tmp/<moviehash>[.a<k>].<fps>fps.ahash`
    /// when MovieHash is known or computed locally, else `/tmp/<name>[.a<k>].<fps>fps.ahash`).
    /// The `<stem>.json` sidecar is written next to it.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// OpenSubtitles 16-hex MovieHash naming the default output and stored in the sidecar;
    /// computed automatically on local files (`>= 64 KiB`) when omitted.
    #[arg(long, value_name = "HASH", value_parser = parse_moviehash)]
    pub moviehash: Option<String>,

    /// Audio sampling rate in Hz (default: 5512).
    #[arg(long, default_value_t = DEFAULT_AHASH_SAMPLE_RATE)]
    pub sample_rate: u32,

    /// Hop size in samples (64 for ~86.1 fps, 230 for ~24.0 fps video sync).
    #[arg(long, default_value_t = DEFAULT_AHASH_HOP_SIZE)]
    pub hop_size: usize,

    /// Optional start seek offset in seconds.
    #[arg(long, default_value_t = 0.0)]
    pub start_sec: f64,

    /// Optional maximum duration in seconds to process.
    #[arg(short, long)]
    pub duration: Option<f64>,

    /// Relative audio track to decode (`-map 0:a:<k>`). Omitted: FFmpeg automatic selection
    /// (bit-identical with legacy `.ahash` files).
    #[arg(long)]
    pub audio_track: Option<usize>,
}

/// Runs Haitsma-Kalker audio hash extraction and writes an .ahash v1.0 binary + JSON sidecar.
///
/// # Arguments
/// * `args` - Parsed CLI options for audio extraction.
///
/// # Returns
/// `Ok(())` on successful extraction and serialization.
///
/// # Errors
/// Returns `Err` if FFmpeg fails, the audio stream cannot be decoded (including a
/// non-existent `--audio-track`), or the `.ahash`/sidecar write fails.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Probes the container audio topology once (ffprobe, headers only; `None` on failure).
/// 2. Resolves `--moviehash` or computes it from `args.media` when local (`>= 64 KiB`).
/// 3. Configures `AudioExtractionOptions` (`audio_stream = --audio-track`).
/// 4. Spawns FFmpeg and streams mono f32le audio into `extract_ahash`.
/// 5. Resolves the output path (`<moviehash>[.a<k>].<fps>fps.ahash` by default when known).
/// 6. Writes the `.ahash` (176-byte header + LE payload), then the sidecar atomically.
/// 7. Prints execution throughput and real-time speed factor.
pub fn run_export_ahash(args: &ExportAhashArgs) -> Result<()> {
    let start_time = Instant::now();
    println!(
        "Extracting Haitsma-Kalker audio hash from {:?}...",
        args.media
    );
    let inspection = probe_audio_topology(&args.media);
    let moviehash = resolve_moviehash(args.moviehash.as_deref(), &args.media);

    let opts = AudioExtractionOptions {
        sample_rate: args.sample_rate,
        frame_size: DEFAULT_FRAME_SIZE,
        hop_size: if args.hop_size == 0 {
            DEFAULT_HOP_SIZE
        } else {
            args.hop_size
        },
        start_sec: args.start_sec,
        duration_sec: args.duration,
        custom_ffmpeg: None,
        audio_stream: args.audio_track,
    };

    let ahash_file = extract_ahash(&args.media, &opts)?;
    let elapsed = start_time.elapsed().as_secs_f64();

    let fps = ahash_file.header.audio_fps();
    let frame_count = ahash_file.hashes.len();
    let effective_duration = if fps > 0.0 {
        frame_count as f64 / fps
    } else {
        0.0
    };
    let speed_factor = effective_duration / elapsed.max(DIVISION_BY_ZERO_FALLBACK_SEC);

    println!(
        "Extracted {} audio hashes in {:.3}s ({:.1} fps, {:.1}x real-time, duration: {:.2}s)",
        frame_count, elapsed, fps, speed_factor, effective_duration
    );

    let track_tag = non_default_track(args.audio_track, inspection.as_ref());
    let target_path = resolve_output_path(args, fps, track_tag, moviehash.as_deref());
    AhashFile::write_to_path(&target_path, &ahash_file.header, &ahash_file.hashes)?;
    let media_name = resolve_media_path_filename(&args.media)?;
    let sidecar_path = AhashSidecar::build(&AhashSidecarRequest {
        ahash_path: &target_path,
        header: &ahash_file.header,
        media_name: &media_name,
        moviehash: moviehash.as_deref(),
        inspection: inspection.as_ref(),
        explicit_audio_stream: args.audio_track,
    })
    .write_next_to(&target_path)?;

    println!(
        "✓ Successfully wrote .ahash v1.0 binary to {:?} (sidecar {:?})",
        target_path, sidecar_path
    );
    Ok(())
}

/// Returns the explicit track index when it differs from FFmpeg's automatic choice.
///
/// # Arguments
/// * `explicit` - `--audio-track` value.
/// * `inspection` - Probed audio topology, if available.
///
/// # Returns
/// `None` without `--audio-track` or when it equals the inferred FFmpeg default track;
/// `Some(k)` otherwise (including when the topology is unknown: safer to tag).
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Uses [`ffmpeg_default_audio_index`] (default disposition, then channels, then first).
fn non_default_track(
    explicit: Option<usize>,
    inspection: Option<&AudioContainerInspection>,
) -> Option<usize> {
    let k = explicit?;
    match inspection.map(|i| i.audio_streams.as_slice()) {
        Some(streams) if !streams.is_empty() => {
            (k != ffmpeg_default_audio_index(streams)).then_some(k)
        }
        _ => Some(k),
    }
}

/// Resolves the destination output file path.
///
/// # Arguments
/// * `args` - CLI extraction arguments.
/// * `fps` - Effective audio framerate.
/// * `track_tag` - Explicit non-default track index to tag in the default stem.
/// * `moviehash` - Resolved 16-hex OpenSubtitles MovieHash, if available.
///
/// # Returns
/// Destination `PathBuf`.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Returns explicitly provided `--output` unchanged; otherwise uses `<moviehash>` when
/// available (preventing accents/spaces/brackets in filenames), falling back to the media
/// filename only when MovieHash cannot be computed:
/// `/tmp/<stem>[.a<k>].<fps>fps.ahash`.
fn resolve_output_path(
    args: &ExportAhashArgs,
    fps: f64,
    track_tag: Option<usize>,
    moviehash: Option<&str>,
) -> PathBuf {
    if let Some(ref path) = args.output {
        return path.clone();
    }
    let base_stem = moviehash.map(str::to_string).unwrap_or_else(|| {
        resolve_media_path_filename(&args.media).unwrap_or_else(|_| {
            args.media
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
    });
    let stem = track_tag.map_or(base_stem.clone(), |k| audio_track_stem(&base_stem, k));
    PathBuf::from(DEFAULT_AHASH_OUTPUT_DIR).join(format!("{}.{:.1}fps.ahash", stem, fps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{
        ahash_sidecar_path, AudioStreamInfo, AudioTrackSelection, DEFAULT_SAMPLE_RATE,
    };
    use std::path::Path;
    use std::process::Command;

    const FIXTURE: &str = "../../tests/data/big_buck_bunny_trailer_480p.mp4";
    /// Duration of the synthetic two-track fixture, in seconds.
    const SYNTH_DURATION_SEC: &str = "3";

    fn base_args(media: PathBuf, output: Option<PathBuf>) -> ExportAhashArgs {
        ExportAhashArgs {
            media,
            output,
            moviehash: None,
            sample_rate: DEFAULT_SAMPLE_RATE,
            hop_size: DEFAULT_HOP_SIZE,
            start_sec: 0.0,
            duration: None,
            audio_track: None,
        }
    }

    fn fixture() -> Option<PathBuf> {
        let media = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        media.exists().then_some(media)
    }

    fn stream(audio_index: usize, channels: usize, is_default: bool) -> AudioStreamInfo {
        AudioStreamInfo {
            index: audio_index + 1,
            audio_index,
            codec_name: "aac".into(),
            channels: Some(channels),
            sample_rate: None,
            language: None,
            title: None,
            is_default,
        }
    }

    /// Two-track MKV: a:0 = 5.1 440 Hz `fre` non-default, a:1 = stereo 1000 Hz `jpn` default.
    fn make_two_track_mkv(dir: &Path) -> Option<PathBuf> {
        let out = dir.join("two_tracks.mkv");
        let status = Command::new(dbsrt_core::video::resolve_ffmpeg_cpu())
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=f=440:d={SYNTH_DURATION_SEC}"),
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=f=1000:d={SYNTH_DURATION_SEC}"),
            ])
            .args(["-map", "0:a", "-map", "1:a", "-c:a", "aac"])
            .args(["-ac:a:0", "6", "-ac:a:1", "2"])
            .args([
                "-metadata:s:a:0",
                "language=fre",
                "-metadata:s:a:1",
                "language=jpn",
            ])
            .args(["-disposition:a:0", "0", "-disposition:a:1", "default"])
            .arg(&out)
            .status()
            .ok()?;
        status.success().then_some(out)
    }

    #[test]
    fn test_resolve_output_path_default() {
        let args = base_args(PathBuf::from("movie.mkv"), None);
        let path = resolve_output_path(&args, 86.125, None, None);
        assert_eq!(path, PathBuf::from("/tmp/movie.mkv.86.1fps.ahash"));
        let hashed = resolve_output_path(&args, 86.125, None, Some("e19d5212c9812cd6"));
        assert_eq!(hashed, PathBuf::from("/tmp/e19d5212c9812cd6.86.1fps.ahash"));
        let tagged = resolve_output_path(&args, 86.125, Some(1), Some("e19d5212c9812cd6"));
        assert_eq!(
            tagged,
            PathBuf::from("/tmp/e19d5212c9812cd6.a1.86.1fps.ahash")
        );
    }

    #[test]
    fn test_resolve_output_path_explicit() {
        let explicit = PathBuf::from("/data/custom.ahash");
        let args = base_args(PathBuf::from("movie.mkv"), Some(explicit.clone()));
        assert_eq!(
            resolve_output_path(&args, 86.125, Some(1), Some("e19d5212c9812cd6")),
            explicit
        );
    }

    #[test]
    fn test_non_default_track() {
        let insp = AudioContainerInspection {
            path: PathBuf::from("m.mkv"),
            audio_streams: vec![stream(0, 6, false), stream(1, 2, true)],
        };
        assert_eq!(non_default_track(None, Some(&insp)), None);
        assert_eq!(non_default_track(Some(1), Some(&insp)), None);
        assert_eq!(non_default_track(Some(0), Some(&insp)), Some(0));
        assert_eq!(non_default_track(Some(0), None), Some(0));
    }

    #[test]
    fn test_run_export_ahash_real_media() {
        let Some(media) = fixture() else {
            eprintln!("Skipping: media fixture not found");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("bbb.86.1fps.ahash");
        let args = ExportAhashArgs {
            duration: Some(1.0),
            ..base_args(media, Some(out_path.clone()))
        };

        let res = run_export_ahash(&args);
        assert!(res.is_ok(), "run_export_ahash should succeed: {:?}", res);

        let ahash = AhashFile::read_from_path(&out_path).expect("valid ahash binary");
        assert_eq!(ahash.header.version, 1);
        assert!(!ahash.hashes.is_empty());

        let sidecar_path = ahash_sidecar_path(&out_path);
        assert_eq!(sidecar_path, dir.path().join("bbb.json"));
        let sidecar = AhashSidecar::read(&sidecar_path).expect("typed sidecar");
        assert_eq!(sidecar.media_name, "big_buck_bunny_trailer_480p.mp4");
        assert_eq!(sidecar.frames, ahash.hashes.len());
        assert_eq!(sidecar.selection, AudioTrackSelection::SingleTrack);
        assert_eq!(sidecar.container_audio_streams.len(), 1);
        assert_eq!(sidecar.selected_audio_stream.unwrap().audio_index, 0);
    }

    #[test]
    fn test_run_export_ahash_audio_track_selection_two_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let Some(media) = make_two_track_mkv(dir.path()) else {
            eprintln!("Skipping: ffmpeg lavfi unavailable");
            return;
        };
        let export = |track: Option<usize>, name: &str| {
            let out = dir.path().join(name);
            let args = ExportAhashArgs {
                audio_track: track,
                ..base_args(media.clone(), Some(out.clone()))
            };
            run_export_ahash(&args).expect("export");
            let hashes = AhashFile::read_from_path(&out).unwrap().hashes;
            (
                hashes,
                AhashSidecar::read(&ahash_sidecar_path(&out)).unwrap(),
            )
        };

        let (auto_h, auto_s) = export(None, "auto.86.1fps.ahash");
        let (t0_h, t0_s) = export(Some(0), "t0.86.1fps.ahash");
        let (t1_h, t1_s) = export(Some(1), "t1.86.1fps.ahash");

        assert_eq!(auto_s.selection, AudioTrackSelection::FfmpegDefault);
        assert_eq!(auto_s.container_audio_streams.len(), 2);
        let inferred = auto_s.selected_audio_stream.expect("inferred stream");
        assert_eq!(inferred.audio_index, 1);
        assert_eq!(inferred.language.as_deref(), Some("jpn"));
        assert_eq!(t0_s.selection, AudioTrackSelection::Explicit);
        assert_eq!(
            t0_s.selected_audio_stream.unwrap().language.as_deref(),
            Some("fre")
        );
        assert_eq!(t1_s.selection, AudioTrackSelection::Explicit);

        // Empirical check of the inference rule: FFmpeg's automatic pick == -map 0:a:1.
        assert_ne!(t0_h, t1_h, "-map must change the decoded track");
        assert_eq!(
            auto_h, t1_h,
            "FFmpeg default must be the default-disposition track"
        );
    }

    #[test]
    fn test_run_export_ahash_default_hop_and_output() {
        let Some(media) = fixture() else {
            eprintln!("Skipping: media fixture not found");
            return;
        };
        let expected_hash = dbsrt_core::metadata::compute_opensubtitles_hash(&media).unwrap();
        let args = ExportAhashArgs {
            hop_size: 0,
            duration: Some(0.5),
            ..base_args(media, None)
        };

        let res = run_export_ahash(&args);
        assert!(res.is_ok());

        let fps = f64::from(DEFAULT_SAMPLE_RATE) / DEFAULT_HOP_SIZE as f64;
        let default_path = resolve_output_path(&args, fps, None, Some(&expected_hash));
        assert_eq!(
            default_path.file_name().unwrap().to_string_lossy(),
            format!("{expected_hash}.86.1fps.ahash")
        );
        assert!(default_path.exists(), "default moviehash .ahash output");
        let sidecar = ahash_sidecar_path(&default_path);
        assert!(sidecar.exists(), "sidecar next to default output");
        let parsed = AhashSidecar::read(&sidecar).unwrap();
        assert_eq!(parsed.moviehash.as_deref(), Some(expected_hash.as_str()));
        let _ = std::fs::remove_file(default_path);
        let _ = std::fs::remove_file(sidecar);
    }

    #[test]
    fn test_run_export_ahash_non_existent_media_fails() {
        let args = ExportAhashArgs {
            duration: Some(1.0),
            ..base_args(PathBuf::from("this_file_does_not_exist_at_all.mp4"), None)
        };
        assert!(
            run_export_ahash(&args).is_err(),
            "Non-existent media must return Err"
        );
    }
}
