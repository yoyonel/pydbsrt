//! Recognition subcommand matching media perceptual hashes against PostgreSQL, .dbphash, or memory-mapped library.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{extract_frames_phash, probe_video};
use dbsrt_index::{DbClient, MmapDbphash, MmapLibrary};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use crate::cli::OutputFormat;
use crate::constants::{
    DEFAULT_EXTRACTION_DURATION_SEC, DEFAULT_POSTGRES_DB, DEFAULT_POSTGRES_HOST,
    DEFAULT_POSTGRES_PASS, DEFAULT_POSTGRES_PORT, DEFAULT_POSTGRES_USER, DEFAULT_RECOGNIZE_LIMIT,
    DEFAULT_SEARCH_DISTANCE,
};

#[derive(Args, Debug)]
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

    /// PostgreSQL host (defaults to PSQL_IP env or 127.0.0.1)
    #[arg(long)]
    pub db_host: Option<String>,

    /// PostgreSQL port
    #[arg(long, env = "PSQL_PORT", default_value_t = DEFAULT_POSTGRES_PORT)]
    pub db_port: u16,

    /// PostgreSQL user
    #[arg(long, default_value = DEFAULT_POSTGRES_USER)]
    pub db_user: String,

    /// PostgreSQL password
    #[arg(long, default_value = DEFAULT_POSTGRES_PASS)]
    pub db_pass: String,

    /// PostgreSQL database name
    #[arg(long, default_value = DEFAULT_POSTGRES_DB)]
    pub db_name: String,

    /// Maximum number of matched media results to output (0 for all)
    #[arg(long, default_value_t = DEFAULT_RECOGNIZE_LIMIT)]
    pub limit: usize,
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

/// Run media recognition pipeline against .dbphash, Zero-DB mmap library, or PostgreSQL.
pub fn run_recognize_media(args: &RecognizeArgs) -> Result<()> {
    // 1. Probe video
    let meta = probe_video(&args.media)?;
    let half_divisor = 2.0;
    let start_sec = (meta.duration / half_divisor).floor();
    let nb_frames = (args.nb_seconds_to_extract * meta.fps).round() as usize;

    // 2. Extract frames & compute pHashes in streaming
    let hashes = extract_frames_phash(&args.media, start_sec, nb_frames)?;

    let media_name_search = args
        .media
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Unified .dbphash mode: search directly over single compiled index
    if let Some(ref dbphash_path) = args.dbphash {
        let dbphash = MmapDbphash::open(dbphash_path)?;
        let match_start = Instant::now();
        let matches = dbphash.recognize(&hashes, args.search_distance);
        let timer_in_seconds = match_start.elapsed().as_secs_f64();

        if matches.is_empty() {
            if args.output_format == OutputFormat::DataFrame {
                println!("Empty DataFrame\nColumns: [media_found, media_name_search, media_id_match, media_name_match, nb_offsets_match, timer_in_seconds, confidence]\nIndex: []");
            }
            return Ok(());
        }

        let display_matches = if args.limit > 0 && args.limit < matches.len() {
            &matches[..args.limit]
        } else {
            &matches[..]
        };

        for m in display_matches.iter() {
            let media_found = m.clean_title.contains(&media_name_search)
                || media_name_search.contains(&m.clean_title);

            match args.output_format {
                OutputFormat::Csv => {
                    println!(
                        "{},{},{},{},{},{:.16},{:.1}",
                        if media_found { "True" } else { "False" },
                        media_name_search,
                        m.clean_title,
                        m.media_id,
                        m.matching_cuts,
                        timer_in_seconds,
                        m.confidence
                    );
                }
                OutputFormat::DataFrame => {
                    println!(
                        "   media_found            media_name_search  media_id_match             media_name_match  nb_offsets_match  ... timer_in_seconds  confidence\n\
                         0         {}  {}               {}  {}               {}  ...         {:.6}         {:.1}\n\n\
                         [1 rows x 7 columns]",
                        if media_found { "True" } else { "False" },
                        media_name_search,
                        m.media_id,
                        m.clean_title,
                        m.matching_cuts,
                        timer_in_seconds,
                        m.confidence
                    );
                }
            }
        }
        return Ok(());
    }

    // Zero-DB mode: search directly over memory-mapped .phash files
    if args.no_db {
        let phash_dir = args.phash_dir.clone().unwrap_or_else(std::env::temp_dir);
        let library = MmapLibrary::from_directory(&phash_dir)?;
        let match_start = Instant::now();
        let matches = library.recognize(&hashes, args.search_distance);
        let timer_in_seconds = match_start.elapsed().as_secs_f64();

        if matches.is_empty() {
            if args.output_format == OutputFormat::DataFrame {
                println!("Empty DataFrame\nColumns: [media_found, media_name_search, media_id_match, media_name_match, nb_offsets_match, search_offsets_match, match_frames_offsets, timer_in_seconds, confidence]\nIndex: []");
            }
            return Ok(());
        }

        let display_matches = if args.limit > 0 && args.limit < matches.len() {
            &matches[..args.limit]
        } else {
            &matches[..]
        };

        for (idx, m) in display_matches.iter().enumerate() {
            let media_found = m.media_name.contains(&media_name_search)
                || media_name_search.contains(&m.media_name);
            let search_set: BTreeSet<usize> = m.search_offsets_match.iter().copied().collect();
            let match_set: BTreeSet<usize> = m.match_frames_offsets.iter().copied().collect();

            match args.output_format {
                OutputFormat::Csv => {
                    println!(
                        "{},{},{},{},{},\"{}\",\"{}\",{:.16},{:.1}",
                        if media_found { "True" } else { "False" },
                        media_name_search,
                        m.media_name,
                        idx + 1,
                        m.nb_offsets_match,
                        format_set(&search_set),
                        format_set(&match_set),
                        timer_in_seconds,
                        m.confidence
                    );
                }
                OutputFormat::DataFrame => {
                    println!(
                        "   media_found            media_name_search  media_id_match             media_name_match  nb_offsets_match  ... timer_in_seconds  confidence\n\
                         0         {}  {}               {}  {}               {}  ...         {:.6}         {:.1}\n\n\
                         [1 rows x 9 columns]",
                        if media_found { "True" } else { "False" },
                        media_name_search,
                        idx + 1,
                        m.media_name,
                        m.nb_offsets_match,
                        timer_in_seconds,
                        m.confidence
                    );
                }
            }
        }
        return Ok(());
    }

    // 3. Connect to DB
    let host = args
        .db_host
        .clone()
        .or_else(|| std::env::var("PSQL_IP").ok())
        .unwrap_or_else(|| DEFAULT_POSTGRES_HOST.to_string());

    let mut db = DbClient::connect(
        &host,
        args.db_port,
        &args.db_user,
        &args.db_pass,
        &args.db_name,
    )?;

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

    // 5. Build results
    let media_name_search = args
        .media
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if matches_by_media.is_empty() {
        if args.output_format == OutputFormat::DataFrame {
            println!("Empty DataFrame\nColumns: [media_found, media_name_search, media_id_match, media_name_match, nb_offsets_match, search_offsets_match, match_frames_offsets, timer_in_seconds, confidence]\nIndex: []");
        }
        return Ok(());
    }

    // Sort by confidence / nb_offsets descending
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
}
