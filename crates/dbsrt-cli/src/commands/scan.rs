//! Library scanning subcommand for fast zero-DB mmap and .dbphash searches.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{extract_frames_phash, probe_video, read_phash_file};
use dbsrt_index::{MmapDbphash, MmapLibrary};
use std::path::PathBuf;
use std::time::Instant;

use crate::constants::{
    DEFAULT_EXACT_SEARCH_DISTANCE, DEFAULT_EXTRACTION_DURATION_SEC, DEFAULT_SCAN_LIMIT,
    DEFAULT_TMP_DIR, DEFAULT_WINDOW_FRAME_GAP, MILLIS_PER_SEC, PERCENT_FACTOR,
};

#[derive(Args, Debug)]
pub struct ScanLibraryArgs {
    /// Directory containing .phash files
    #[arg(short, long, default_value = DEFAULT_TMP_DIR)]
    pub phash_dir: PathBuf,

    /// Optional unified .dbphash index file to search
    #[arg(long)]
    pub dbphash: Option<PathBuf>,

    /// Query media file to recognize
    #[arg(short, long)]
    pub query_media: Option<PathBuf>,

    /// Query .phash file to search
    #[arg(long)]
    pub query_phash: Option<PathBuf>,

    /// Optional second query .phash file for Two-Window Consensus recognition
    #[arg(long)]
    pub query_phash_b: Option<PathBuf>,

    /// Expected frame gap between query_phash and query_phash_b
    #[arg(long, default_value_t = DEFAULT_WINDOW_FRAME_GAP)]
    pub frame_gap: usize,

    /// Hamming search distance
    #[arg(short = 'd', long, default_value_t = DEFAULT_EXACT_SEARCH_DISTANCE)]
    pub search_distance: u32,

    /// Nb seconds to extract if querying with media file
    #[arg(short = 's', long, default_value_t = DEFAULT_EXTRACTION_DURATION_SEC)]
    pub nb_seconds_to_extract: f64,

    /// Maximum number of top matches to display
    #[arg(short = 'n', long, default_value_t = DEFAULT_SCAN_LIMIT)]
    pub limit: usize,
}

/// Run zero-DB memory-mapped or compiled .dbphash library search.
pub fn run_scan_library(args: &ScanLibraryArgs) -> Result<()> {
    let half_divisor = 2.0;
    let query_hashes = if let Some(ref media_path) = args.query_media {
        let meta = probe_video(media_path)?;
        let start_sec = (meta.duration / half_divisor).floor();
        let nb_frames = (args.nb_seconds_to_extract * meta.fps).round() as usize;
        extract_frames_phash(media_path, start_sec, nb_frames)?
    } else if let Some(ref phash_path) = args.query_phash {
        read_phash_file(phash_path)?
    } else {
        anyhow::bail!("Either --query-media or --query-phash must be specified");
    };

    if let Some(ref dbphash_path) = args.dbphash {
        let start = Instant::now();
        let dbphash = MmapDbphash::open(dbphash_path)?;
        println!(
            "Loaded .dbphash index {:?}: {} media entries, {} cuts, {} sparse frames (took {:.2} ms)",
            dbphash_path,
            dbphash.header().media_count,
            dbphash.header().total_cuts_count,
            dbphash.header().total_sparse_count,
            start.elapsed().as_secs_f64() * MILLIS_PER_SEC
        );

        let search_start = Instant::now();
        let matches = dbphash.recognize(&query_hashes, args.search_distance);
        let search_elapsed = search_start.elapsed().as_secs_f64();

        println!(
            "Search completed in {:.4} seconds ({} query frames)",
            search_elapsed,
            query_hashes.len()
        );

        if matches.is_empty() {
            println!("No matching media found in .dbphash index.");
        } else {
            println!("\nTop matches from .dbphash index:");
            let display_matches = if args.limit > 0 && args.limit < matches.len() {
                &matches[..args.limit]
            } else {
                &matches[..]
            };
            for (i, m) in display_matches.iter().enumerate() {
                let season_ep = match (m.season, m.episode) {
                    (Some(s), Some(e)) if s > 0 || e > 0 => format!(" S{:02}E{:02}", s, e),
                    _ => String::new(),
                };
                println!(
                    "  {}. ID #{}: {}{} -> cuts: {}, 1fps: {} ({:.1}% confidence, shift: {} ms)",
                    i + 1,
                    m.media_id,
                    m.clean_title,
                    season_ep,
                    m.matching_cuts,
                    m.matching_sparse,
                    m.confidence,
                    m.estimated_shift_ms
                );
            }
        }
        return Ok(());
    }

    let start = Instant::now();
    let library = MmapLibrary::from_directory(&args.phash_dir)?;
    println!(
        "Loaded library from {:?}: {} files, {} total frames (took {:.2} ms)",
        args.phash_dir,
        library.file_count(),
        library.total_frames(),
        start.elapsed().as_secs_f64() * MILLIS_PER_SEC
    );

    let search_start = Instant::now();
    let (matches, total_q_frames) = if let Some(ref phash_b_path) = args.query_phash_b {
        let hashes_b = read_phash_file(phash_b_path)?;
        let n_frames = query_hashes.len() + hashes_b.len();
        (
            library.recognize_two_windows(
                &query_hashes,
                &hashes_b,
                args.frame_gap,
                args.search_distance,
            ),
            n_frames,
        )
    } else {
        let n_frames = query_hashes.len();
        (
            library.recognize(&query_hashes, args.search_distance),
            n_frames,
        )
    };
    let search_elapsed = search_start.elapsed().as_secs_f64();

    println!(
        "Search completed in {:.4} seconds ({} query frames)",
        search_elapsed, total_q_frames
    );

    if matches.is_empty() {
        println!("No matching media found.");
    } else {
        println!("\nTop matches:");
        let display_matches = if args.limit > 0 && args.limit < matches.len() {
            &matches[..args.limit]
        } else {
            &matches[..]
        };
        for (i, m) in display_matches.iter().enumerate() {
            println!(
                "  {}. {} -> {} matching frames ({:.1}% confidence)",
                i + 1,
                m.media_name,
                m.nb_offsets_match,
                m.confidence * PERCENT_FACTOR
            );
        }
    }
    Ok(())
}
