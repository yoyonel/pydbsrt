//! CLI handler for `dbsrt ahash:sidecar` (`ahash-sidecar` alias): idempotent `.ahash` JSON sidecar
//! backfill and multi-track Hamming verification against the source container.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{
    ahash_logical_stem, backfill_ahash_sidecar, BackfillAction, BackfillSidecarOutcome,
    BackfillSidecarRequest, TrackVerificationProbe,
};
use serde::Serialize;
use std::path::PathBuf;

use crate::commands::shared::{parse_moviehash, resolve_media_path_filename};

/// Command-line arguments for `dbsrt ahash:sidecar`.
#[derive(Args, Debug, Clone)]
pub struct AhashSidecarArgs {
    /// Path to an existing `.ahash` binary file.
    #[arg(long)]
    pub ahash: PathBuf,

    /// Source media file path or remote stream URI (`ftp://`, `http(s)://`).
    #[arg(long)]
    pub media: PathBuf,

    /// Optional 16-hex OpenSubtitles MovieHash (inferred automatically from `<hash>.<fps>fps.ahash`).
    #[arg(long, value_parser = parse_moviehash)]
    pub moviehash: Option<String>,

    /// Canonical media file name stored in `<stem>.json` (defaults to filename of `--media`).
    #[arg(long)]
    pub media_name: Option<String>,

    /// Optional logical media source identifier stored in `<stem>.json` (never credentials).
    #[arg(long)]
    pub source_id: Option<String>,

    /// Optional relative path within the source repository stored in `<stem>.json`.
    #[arg(long)]
    pub source_rel_path: Option<String>,

    /// Skip 5s midpoint Hamming verification on multi-track containers (`selection = inferred_backfill`).
    #[arg(long)]
    pub no_verify: bool,

    /// Force regeneration even when existing sidecar is already certain or `verified_backfill`.
    #[arg(long)]
    pub force: bool,

    /// Emit machine-readable JSON summary to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Machine-readable JSON summary printed when `--json` is supplied.
#[derive(Debug, Serialize)]
struct AhashSidecarCliReport {
    action: BackfillAction,
    ahash: PathBuf,
    sidecar: PathBuf,
    media_name: String,
    moviehash: Option<String>,
    selection: dbsrt_core::AudioTrackSelection,
    audio_index: Option<usize>,
    language: Option<String>,
    track_count: usize,
    verified_distance: Option<f64>,
    probes: Vec<TrackVerificationProbe>,
}

/// Infers a lowercase 16-hex MovieHash from `<moviehash>.<fps>fps.ahash` when `--moviehash` is omitted.
fn infer_moviehash_from_ahash_path(ahash_path: &std::path::Path) -> Option<String> {
    let file_name = ahash_path.file_name()?.to_str()?;
    let stem = ahash_logical_stem(file_name);
    parse_moviehash(stem).ok()
}

/// Prints a human-readable or `--json` summary of [`BackfillSidecarOutcome`].
fn emit_outcome(args: &AhashSidecarArgs, outcome: &BackfillSidecarOutcome) -> Result<()> {
    let selected = outcome.sidecar.selected_audio_stream.as_ref();
    if args.json {
        let report = AhashSidecarCliReport {
            action: outcome.action,
            ahash: args.ahash.clone(),
            sidecar: outcome.sidecar_path.clone(),
            media_name: outcome.sidecar.media_name.clone(),
            moviehash: outcome.sidecar.moviehash.clone(),
            selection: outcome.sidecar.selection,
            audio_index: selected.map(|s| s.audio_index),
            language: selected.and_then(|s| s.language.clone()),
            track_count: outcome.sidecar.container_audio_streams.len(),
            verified_distance: outcome.sidecar.verified_distance,
            probes: outcome.probes.clone(),
        };
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    let status = match outcome.action {
        BackfillAction::Skipped => "skipped (already certain/verified)",
        BackfillAction::Written => "written",
    };
    let track_label = selected.map_or_else(
        || "none".to_string(),
        |s| {
            format!(
                "0:a:{} ({})",
                s.audio_index,
                s.language.as_deref().unwrap_or("und")
            )
        },
    );
    println!(
        "✓ ahash:sidecar {}: {:?} [selection={:?}, track={}, streams={}]",
        status,
        outcome.sidecar_path,
        outcome.sidecar.selection,
        track_label,
        outcome.sidecar.container_audio_streams.len(),
    );
    Ok(())
}

/// Executes `dbsrt ahash:sidecar`: builds/upgrades `<stem>.json` next to `--ahash`.
///
/// # Arguments
/// * `args` - Parsed CLI options ([`AhashSidecarArgs`]).
///
/// # Returns
/// `Ok(())` on skip or atomic sidecar write.
///
/// # Errors
/// Returns `Err` if `.ahash` cannot be read, `ffprobe` fails on `--media`, or writing JSON fails.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Resolves fallback `media_name` and 16-hex `moviehash`, delegates probing and optional
/// multi-track Hamming verification to [`backfill_ahash_sidecar`], and prints a redacted report.
pub fn run_ahash_sidecar(args: &AhashSidecarArgs) -> Result<()> {
    let media_name = match args.media_name.clone() {
        Some(name) if !name.trim().is_empty() => name,
        _ => resolve_media_path_filename(&args.media)?,
    };
    let moviehash = args
        .moviehash
        .clone()
        .or_else(|| infer_moviehash_from_ahash_path(&args.ahash));

    let outcome = backfill_ahash_sidecar(&BackfillSidecarRequest {
        ahash_path: &args.ahash,
        media: &args.media,
        media_name: &media_name,
        moviehash: moviehash.as_deref(),
        source_id: args.source_id.as_deref(),
        source_rel_path: args.source_rel_path.as_deref(),
        no_verify: args.no_verify,
        force: args.force,
    })?;
    emit_outcome(args, &outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{
        extract_ahash, AhashFile, AhashSidecar, AudioExtractionOptions, AudioTrackSelection,
        DEFAULT_MAX_AUDIO_MATCH_DISTANCE,
    };
    use std::path::Path;
    use std::process::Command;

    const HEX: &str = "05b6126884056c42";
    const SYNTH_DURATION_SEC: &str = "3";

    fn make_two_track_mkv(dir: &Path) -> Option<PathBuf> {
        let out = dir.join("dual_cli.mkv");
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

    #[test]
    fn test_infer_moviehash_from_ahash_path() {
        assert_eq!(
            infer_moviehash_from_ahash_path(Path::new(&format!(
                "data/__AHASH__/{HEX}.86.1fps.ahash"
            )))
            .as_deref(),
            Some(HEX)
        );
        assert_eq!(
            infer_moviehash_from_ahash_path(Path::new("Episode.1.mkv.86.1fps.ahash")),
            None
        );
        assert_eq!(
            infer_moviehash_from_ahash_path(Path::new("0000000000000000.86.1fps.ahash")),
            None
        );
    }

    #[test]
    fn test_run_ahash_sidecar_cli_dual_track_verified_backfill() {
        let dir = tempfile::tempdir().unwrap();
        let Some(media) = make_two_track_mkv(dir.path()) else {
            eprintln!("Skipping: ffmpeg lavfi unavailable");
            return;
        };

        let opts_t0 = AudioExtractionOptions {
            audio_stream: Some(0),
            ..Default::default()
        };
        let ahash = extract_ahash(&media, &opts_t0).unwrap();
        let ahash_path = dir.path().join(format!("{HEX}.86.1fps.ahash"));
        AhashFile::write_to_path(&ahash_path, &ahash.header, &ahash.hashes).unwrap();

        let args = AhashSidecarArgs {
            ahash: ahash_path.clone(),
            media,
            moviehash: None,
            media_name: None,
            source_id: Some("synology_tvshow".to_string()),
            source_rel_path: Some("tvshow/Anime/dual_cli.mkv".to_string()),
            no_verify: false,
            force: false,
            json: true,
        };
        run_ahash_sidecar(&args).unwrap();

        let sidecar = AhashSidecar::read(&dir.path().join(format!("{HEX}.json"))).unwrap();
        assert_eq!(sidecar.moviehash.as_deref(), Some(HEX));
        assert_eq!(sidecar.media_name, "dual_cli.mkv");
        assert_eq!(sidecar.selection, AudioTrackSelection::VerifiedBackfill);
        assert_eq!(sidecar.source_id.as_deref(), Some("synology_tvshow"));
        assert_eq!(
            sidecar.source_rel_path.as_deref(),
            Some("tvshow/Anime/dual_cli.mkv")
        );
        assert!(sidecar.verified_distance.unwrap() <= DEFAULT_MAX_AUDIO_MATCH_DISTANCE);
        let chosen = sidecar.selected_audio_stream.unwrap();
        assert_eq!(chosen.audio_index, 0);
        assert_eq!(chosen.language.as_deref(), Some("fre"));
    }
}
