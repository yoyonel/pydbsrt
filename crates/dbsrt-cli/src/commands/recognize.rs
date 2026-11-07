//! Recognition subcommand matching media perceptual hashes against PostgreSQL, .dbphash, or memory-mapped library.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{extract_frames_phash, probe_video};
use dbsrt_index::{DbphashMediaMatch, LibraryMediaMatch, MmapDbphash, MmapLibrary};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use crate::cli::{OutputFormat, PostgresArgs};
use crate::commands::connect_db;
use crate::commands::recognize_multi::{execute_multi_window_dbphash, MultiWindowContext};
use crate::constants::{
    DEFAULT_EXTRACTION_DURATION_SEC, DEFAULT_RECOGNIZE_LIMIT, DEFAULT_SEARCH_DISTANCE,
    MIDPOINT_DURATION_DIVISOR,
};

#[derive(Args, Debug, Clone)]
pub struct RecognizeArgs {
    /// Media to recognize
    #[arg(short, long, alias = "ref_media")]
    pub media: PathBuf,

    /// Search distance to use (Hamming threshold)
    #[arg(short = 'd', long, default_value_t = DEFAULT_SEARCH_DISTANCE)]
    pub search_distance: u32,

    /// Nb seconds to use for cutting the media and searching (around the middle)
    #[arg(short = 's', long, default_value_t = DEFAULT_EXTRACTION_DURATION_SEC)]
    pub nb_seconds_to_extract: f64,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::DataFrame)]
    pub output_format: OutputFormat,

    /// Optional unified .dbphash index file to search against
    #[arg(long)]
    pub dbphash: Option<PathBuf>,

    /// Use zero-DB memory-mapped engine directly on .phash files (no PostgreSQL needed)
    #[arg(long)]
    pub no_db: bool,

    /// Directory containing .phash files to search against when --no-db is active
    #[arg(long)]
    pub phash_dir: Option<PathBuf>,

    /// PostgreSQL connection arguments
    #[command(flatten)]
    pub db: PostgresArgs,

    /// Maximum number of matched media results to output (0 for all)
    #[arg(long, default_value_t = DEFAULT_RECOGNIZE_LIMIT)]
    pub limit: usize,

    /// Multi-window matching mode (evaluates multiple timeline windows to eliminate collisions)
    #[arg(long)]
    pub multi_window: bool,

    /// Custom temporal offset ratios for multi-window matching (comma-separated, e.g. "0.2,0.5,0.8")
    #[arg(long)]
    pub offsets: Option<String>,

    /// Path to persistent probe cache JSON file (bypasses ffprobe when moviehash matches)
    #[arg(long)]
    pub probe_cache: Option<PathBuf>,
}

#[derive(Default)]
pub struct AggregatedMatch {
    pub nb_offsets_match: usize,
    pub search_offsets_match: BTreeSet<usize>,
    pub match_frames_offsets: BTreeSet<i32>,
}

pub fn format_set<T: std::fmt::Display>(set: &BTreeSet<T>) -> String {
    let items: Vec<String> = set.iter().map(|x| x.to_string()).collect();
    format!("{{{}}}", items.join(", "))
}

fn maybe_run_multi_window(
    args: &RecognizeArgs,
    meta: &dbsrt_core::VideoMetadata,
) -> Result<Option<()>> {
    if (!args.multi_window && args.offsets.is_none()) || args.dbphash.is_none() {
        return Ok(None);
    }

    let ratios = if let Some(ref offset_str) = args.offsets {
        offset_str
            .split(',')
            .map(|s| s.trim().parse::<f64>())
            .collect::<Result<Vec<f64>, _>>()
            .map_err(|e| anyhow::anyhow!("Invalid offset ratio: {e}"))?
    } else {
        dbsrt_core::multi_window::DEFAULT_MULTI_WINDOW_RATIOS.to_vec()
    };

    let plan = dbsrt_core::multi_window::MultiWindowPlan::new(ratios, args.nb_seconds_to_extract)?;
    let dbphash = MmapDbphash::open(args.dbphash.as_ref().unwrap())?;
    let ctx = MultiWindowContext {
        media_path: &args.media,
        meta,
        dbphash: &dbphash,
        plan: &plan,
    };
    execute_multi_window_dbphash(&ctx, args)?;
    Ok(Some(()))
}

fn render_dbphash_match(m: &DbphashMediaMatch, media_name: &str, timer: f64, fmt: OutputFormat) {
    let media_found = m.clean_title.contains(media_name) || media_name.contains(&m.clean_title);
    match fmt {
        OutputFormat::Csv => {
            println!(
                "{},{},{},{},{},{:.16},{:.1}",
                if media_found { "True" } else { "False" },
                media_name,
                m.clean_title,
                m.media_id,
                m.matching_cuts,
                timer,
                m.confidence
            );
        }
        OutputFormat::DataFrame => {
            println!(
                "   media_found            media_name_search  media_id_match             media_name_match  nb_offsets_match  ... timer_in_seconds  confidence\n\
                 0         {}  {}               {}  {}               {}  ...         {:.6}         {:.1}\n\n\
                 [1 rows x 7 columns]",
                if media_found { "True" } else { "False" },
                media_name,
                m.media_id,
                m.clean_title,
                m.matching_cuts,
                timer,
                m.confidence
            );
        }
    }
}

fn execute_dbphash_recognize(
    args: &RecognizeArgs,
    hashes: &[u64],
    media_name: &str,
    dbphash_path: &std::path::Path,
) -> Result<()> {
    let dbphash = MmapDbphash::open(dbphash_path)?;
    let match_start = Instant::now();
    let matches = dbphash.recognize(hashes, args.search_distance);
    let timer_in_seconds = match_start.elapsed().as_secs_f64();

    if matches.is_empty() {
        if args.output_format == OutputFormat::DataFrame {
            println!("Empty DataFrame\nColumns: [media_found, media_name_search, media_id_match, media_name_match, nb_offsets_match, timer_in_seconds, confidence]\nIndex: []");
        }
        return Ok(());
    }

    let limit = if args.limit > 0 {
        args.limit
    } else {
        matches.len()
    };
    for m in matches.iter().take(limit) {
        render_dbphash_match(m, media_name, timer_in_seconds, args.output_format);
    }
    Ok(())
}

struct MmapRenderRow<'a> {
    idx: usize,
    m: &'a LibraryMediaMatch,
    media_name: &'a str,
    timer: f64,
}

fn render_mmap_match(row: &MmapRenderRow<'_>, fmt: OutputFormat) {
    let m = row.m;
    let media_found =
        m.media_name.contains(row.media_name) || row.media_name.contains(&m.media_name);
    let search_set: BTreeSet<usize> = m.search_offsets_match.iter().copied().collect();
    let match_set: BTreeSet<usize> = m.match_frames_offsets.iter().copied().collect();

    match fmt {
        OutputFormat::Csv => {
            println!(
                "{},{},{},{},{},\"{}\",\"{}\",{:.16},{:.1}",
                if media_found { "True" } else { "False" },
                row.media_name,
                m.media_name,
                row.idx + 1,
                m.nb_offsets_match,
                format_set(&search_set),
                format_set(&match_set),
                row.timer,
                m.confidence
            );
        }
        OutputFormat::DataFrame => {
            println!(
                "   media_found            media_name_search  media_id_match             media_name_match  nb_offsets_match  ... timer_in_seconds  confidence\n\
                 0         {}  {}               {}  {}               {}  ...         {:.6}         {:.1}\n\n\
                 [1 rows x 9 columns]",
                if media_found { "True" } else { "False" },
                row.media_name,
                row.idx + 1,
                m.media_name,
                m.nb_offsets_match,
                row.timer,
                m.confidence
            );
        }
    }
}

fn execute_mmap_recognize(args: &RecognizeArgs, hashes: &[u64], media_name: &str) -> Result<()> {
    let phash_dir = args.phash_dir.clone().unwrap_or_else(std::env::temp_dir);
    let library = MmapLibrary::from_directory(&phash_dir)?;
    let match_start = Instant::now();
    let matches = library.recognize(hashes, args.search_distance);
    let timer_in_seconds = match_start.elapsed().as_secs_f64();

    if matches.is_empty() {
        if args.output_format == OutputFormat::DataFrame {
            println!("Empty DataFrame\nColumns: [media_found, media_name_search, media_id_match, media_name_match, nb_offsets_match, search_offsets_match, match_frames_offsets, timer_in_seconds, confidence]\nIndex: []");
        }
        return Ok(());
    }

    let limit = if args.limit > 0 {
        args.limit
    } else {
        matches.len()
    };
    for (idx, m) in matches.iter().take(limit).enumerate() {
        let row = MmapRenderRow {
            idx,
            m,
            media_name,
            timer: timer_in_seconds,
        };
        render_mmap_match(&row, args.output_format);
    }
    Ok(())
}

/// Resolves video metadata either from the probe cache (if provided) or falling back to ffprobe.
fn resolve_video_metadata(args: &RecognizeArgs) -> Result<dbsrt_core::VideoMetadata> {
    if let Some(meta) = try_resolve_from_cache(args) {
        return Ok(meta);
    }
    probe_video(&args.media)
}

fn try_resolve_from_cache(args: &RecognizeArgs) -> Option<dbsrt_core::VideoMetadata> {
    let cache_path = args.probe_cache.as_ref()?;
    let cache = dbsrt_core::probe_cache::ProbeCache::load_or_create(cache_path.clone()).ok()?;
    let entry = cache.query_or_probe_local(&args.media).ok()?;
    Some(dbsrt_core::VideoMetadata {
        duration: entry.duration_seconds,
        fps: entry.fps,
    })
}

/// Run media recognition pipeline against .dbphash, Zero-DB mmap library, or PostgreSQL.
pub fn run_recognize_media(args: &RecognizeArgs) -> Result<()> {
    // 1. Probe video (cached or direct)
    let meta = resolve_video_metadata(args)?;

    if let Some(()) = maybe_run_multi_window(args, &meta)? {
        return Ok(());
    }

    let start_sec = (meta.duration / MIDPOINT_DURATION_DIVISOR).floor();
    let nb_frames = (args.nb_seconds_to_extract * meta.fps).round() as usize;
    if nb_frames == 0 {
        anyhow::bail!("Number of frames to extract must be greater than 0");
    }

    // 2. Extract frames & compute pHashes in streaming
    let hashes = extract_frames_phash(&args.media, start_sec, nb_frames)?;

    let media_name_search = args
        .media
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Unified .dbphash mode
    if let Some(ref dbphash_path) = args.dbphash {
        return execute_dbphash_recognize(args, &hashes, &media_name_search, dbphash_path);
    }

    // Zero-DB mode
    if args.no_db {
        return execute_mmap_recognize(args, &hashes, &media_name_search);
    }

    // 3. Connect to DB
    let mut db = connect_db(&args.db)?;

    // 4. Query DB & measure matching time
    let match_start = Instant::now();
    let mut matches_by_media: HashMap<i32, AggregatedMatch> = HashMap::new();

    for (offset, &phash) in hashes.iter().enumerate() {
        let frame_matches = db.search_frame(phash, args.search_distance)?;
        for m in frame_matches {
            let entry = matches_by_media.entry(m.media_id).or_default();
            entry.nb_offsets_match += 1;
            entry.search_offsets_match.insert(offset);
            entry.match_frames_offsets.insert(m.frame_offset);
        }
    }

    let timer_in_seconds = match_start.elapsed().as_secs_f64();

    if matches_by_media.is_empty() {
        if args.output_format == OutputFormat::DataFrame {
            println!("Empty DataFrame\nColumns: [media_found, media_name_search, media_id_match, media_name_match, nb_offsets_match, search_offsets_match, match_frames_offsets, timer_in_seconds, confidence]\nIndex: []");
        }
        return Ok(());
    }

    let mut sorted_media: Vec<(i32, AggregatedMatch)> = matches_by_media.into_iter().collect();
    sorted_media.sort_by_key(|a| std::cmp::Reverse(a.1.nb_offsets_match));

    let display_media = if args.limit > 0 && args.limit < sorted_media.len() {
        &sorted_media[..args.limit]
    } else {
        &sorted_media[..]
    };

    for &(media_id, ref agg) in display_media {
        let media_name_match = db.get_media_name(media_id)?;
        let media_found = media_name_match.contains(&media_name_search)
            || media_name_search.contains(&media_name_match);
        let confidence = agg.search_offsets_match.len() as f64 / nb_frames as f64;

        match args.output_format {
            OutputFormat::Csv => {
                println!(
                    "{},{},{},{},{},\"{}\",\"{}\",{:.16},{:.1}",
                    if media_found { "True" } else { "False" },
                    media_name_search,
                    media_name_match,
                    media_id,
                    agg.nb_offsets_match,
                    format_set(&agg.search_offsets_match),
                    format_set(&agg.match_frames_offsets),
                    timer_in_seconds,
                    confidence
                );
            }
            OutputFormat::DataFrame => {
                println!(
                    "   media_found            media_name_search  media_id_match             media_name_match  nb_offsets_match  ... timer_in_seconds  confidence\n\
                     0         {}  {}               {}  {}               {}  ...         {:.6}         {:.1}\n\n\
                     [1 rows x 9 columns]",
                    if media_found { "True" } else { "False" },
                    media_name_search,
                    media_id,
                    media_name_match,
                    agg.nb_offsets_match,
                    timer_in_seconds,
                    confidence
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_set() {
        let mut set = BTreeSet::new();
        assert_eq!(format_set(&set), "{}");
        set.insert(42);
        assert_eq!(format_set(&set), "{42}");
        set.insert(10);
        assert_eq!(format_set(&set), "{10, 42}");
    }

    #[test]
    fn test_recognize_zero_frames_bail() {
        let video = if std::path::Path::new("tests/data/big_buck_bunny_trailer_480p.mp4").exists() {
            PathBuf::from("tests/data/big_buck_bunny_trailer_480p.mp4")
        } else {
            PathBuf::from("data/big_buck_bunny_trailer_480p.webm")
        };
        if !video.exists() {
            return;
        }
        let args = RecognizeArgs {
            media: video,
            search_distance: crate::constants::DEFAULT_SEARCH_DISTANCE,
            nb_seconds_to_extract: 0.0,
            output_format: OutputFormat::DataFrame,
            limit: crate::constants::DEFAULT_RECOGNIZE_LIMIT,
            dbphash: None,
            no_db: true,
            phash_dir: None,
            db: PostgresArgs::default(),
            multi_window: false,
            offsets: None,
            probe_cache: None,
        };
        let err = run_recognize_media(&args).unwrap_err();
        assert!(err
            .to_string()
            .contains("Number of frames to extract must be greater than 0"));
    }
}
