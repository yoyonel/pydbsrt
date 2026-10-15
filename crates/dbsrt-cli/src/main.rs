use clap::{Parser, Subcommand, ValueEnum};
use dbsrt_core::{extract_frames_phash, probe_video};
use dbsrt_index::DbClient;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "dbsrt", version, about = "High-performance video perceptual hash & subtitle retargeting")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    #[value(name = "DataFrame", alias = "dataframe")]
    DataFrame,
    #[value(name = "CSV", alias = "csv")]
    Csv,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Benchmark pHash computation throughput
    BenchPhash {
        #[arg(short, long, default_value_t = 100_000)]
        count: usize,
    },

    /// Recognize media from database by computing perceptual hash fingerprint
    RecognizeMedia(RecognizeArgs),
}

#[derive(clap::Args, Debug)]
pub struct RecognizeArgs {
    /// Media to recognize
    #[arg(short, long, alias = "ref_media")]
    media: PathBuf,

    /// Search distance to use (Hamming threshold)
    #[arg(short = 'd', long, default_value_t = 1)]
    search_distance: u32,

    /// Nb seconds to use for cutting the media and searching (around the middle)
    #[arg(short = 's', long, default_value_t = 1.0)]
    nb_seconds_to_extract: f64,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::DataFrame)]
    output_format: OutputFormat,

    /// PostgreSQL host (defaults to PSQL_IP env or 127.0.0.1)
    #[arg(long)]
    db_host: Option<String>,

    /// PostgreSQL port
    #[arg(long, default_value_t = 5432)]
    db_port: u16,

    /// PostgreSQL user
    #[arg(long, default_value = "docker")]
    db_user: String,

    /// PostgreSQL password
    #[arg(long, default_value = "docker")]
    db_pass: String,

    /// PostgreSQL database name
    #[arg(long, default_value = "bktreedb")]
    db_name: String,
}

#[derive(Default)]
struct AggregatedMatch {
    nb_offsets_match: usize,
    search_offsets_match: BTreeSet<usize>,
    match_frames_offsets: BTreeSet<i32>,
}

fn format_set<T: std::fmt::Display>(set: &BTreeSet<T>) -> String {
    let items: Vec<String> = set.iter().map(|x| x.to_string()).collect();
    format!("{{{}}}", items.join(", "))
}

fn run_recognize_media(args: &RecognizeArgs) -> anyhow::Result<()> {
    // 1. Probe video
    let meta = probe_video(&args.media)?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (args.nb_seconds_to_extract * meta.fps).round() as usize;

    // 2. Extract frames & compute pHashes in streaming
    let hashes = extract_frames_phash(&args.media, start_sec, nb_frames)?;

    // 3. Connect to DB
    let host = args
        .db_host
        .clone()
        .or_else(|| std::env::var("PSQL_IP").ok())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let mut db = DbClient::connect(&host, args.db_port, &args.db_user, &args.db_pass, &args.db_name)?;

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

    for (media_id, agg) in sorted_media {
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::BenchPhash { count } => {
            println!("Benchmarking pHash on {} frames...", count);
            let dummy_frame = [128u8; 1024];
            let start = Instant::now();
            let mut last_hash = 0;
            for _ in 0..count {
                last_hash ^= dbsrt_core::phash_32x32(&dummy_frame);
            }
            let elapsed = start.elapsed();
            let fps = count as f64 / elapsed.as_secs_f64();
            let per_frame_us = elapsed.as_secs_f64() * 1_000_000.0 / count as f64;
            println!("Result hash: {:016x}", last_hash);
            println!("Elapsed: {:?}", elapsed);
            println!("Throughput: {:.0} frames/sec ({:.2} µs / frame)", fps, per_frame_us);
        }
        Commands::RecognizeMedia(args) => {
            run_recognize_media(&args)?;
        }
    }
    Ok(())
}
