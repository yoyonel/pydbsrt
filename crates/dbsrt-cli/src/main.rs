use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use dbsrt_core::{
    detect_cuts, extract_frames_advanced, extract_frames_phash, probe_video, read_phash_file,
    write_phash_file, HwAccel, SamplingMode, SubtitleFile, VideoExtractionOptions,
};
use dbsrt_index::{BkItem, BkTree, DbClient, MmapLibrary};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(
    name = "dbsrt",
    version,
    about = "High-performance video perceptual hash & subtitle retargeting"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliHwAccel {
    #[value(name = "auto")]
    Auto,
    #[value(name = "disabled", alias = "cpu")]
    Disabled,
    #[value(name = "vaapi")]
    Vaapi,
    #[value(name = "cuda")]
    Cuda,
    #[value(name = "qsv")]
    Qsv,
}

impl From<CliHwAccel> for HwAccel {
    fn from(c: CliHwAccel) -> Self {
        match c {
            CliHwAccel::Auto => HwAccel::Auto,
            CliHwAccel::Disabled => HwAccel::Disabled,
            CliHwAccel::Vaapi => HwAccel::Vaapi,
            CliHwAccel::Cuda => HwAccel::Cuda,
            CliHwAccel::Qsv => HwAccel::Qsv,
        }
    }
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

    /// Benchmark video decoding throughput across CPU, HWAccel, sparse and keyframe modes
    BenchVideo(BenchVideoArgs),

    /// Recognize media from database or local phash files by computing perceptual hash fingerprint
    RecognizeMedia(RecognizeArgs),

    /// Retarget and synchronize SRT subtitles from reference to target media
    RetargetSubtitles(RetargetArgs),

    /// Export binary .phash file for a media using fast SIMD extraction
    ExportPhash(ExportPhashArgs),

    /// Zero-DB fast search across directory of .phash files via memory mapping
    ScanLibrary(ScanLibraryArgs),

    /// Benchmark scene cuts detection throughput across .phash files
    BenchCuts(BenchCutsArgs),

    /// Import .phash binary file(s) into PostgreSQL 17 BK-Tree
    ImportPhash(ImportPhashArgs),
}

#[derive(clap::Args, Debug)]
pub struct ImportPhashArgs {
    /// Single .phash file to import
    #[arg(short = 'r', long, alias = "binary-img-hash-file")]
    file: Option<PathBuf>,

    /// Directory of .phash files to batch import
    #[arg(short = 'p', long)]
    phash_dir: Option<PathBuf>,

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

#[derive(clap::Args, Debug)]
pub struct BenchCutsArgs {
    /// Directory containing .phash files
    #[arg(short, long, default_value = "data/__PHASH__")]
    phash_dir: PathBuf,

    /// Cut threshold Hamming distance
    #[arg(short = 't', long, default_value_t = 16)]
    threshold: u32,

    /// Force single-threaded execution to benchmark single-core performance
    #[arg(long)]
    single_thread: bool,
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

    /// Use zero-DB memory-mapped engine directly on .phash files (no PostgreSQL needed)
    #[arg(long)]
    no_db: bool,

    /// Directory containing .phash files to search against when --no-db is active
    #[arg(long)]
    phash_dir: Option<PathBuf>,

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

    /// Maximum number of matched media results to output (0 for all)
    #[arg(long, default_value_t = 1)]
    limit: usize,
}

#[derive(clap::Args, Debug)]
pub struct ScanLibraryArgs {
    /// Directory containing .phash files
    #[arg(short, long, default_value = "/tmp")]
    pub phash_dir: PathBuf,

    /// Query media file to recognize
    #[arg(short, long)]
    pub query_media: Option<PathBuf>,

    /// Query .phash file to search
    #[arg(long)]
    pub query_phash: Option<PathBuf>,

    /// Hamming search distance
    #[arg(short = 'd', long, default_value_t = 0)]
    pub search_distance: u32,

    /// Nb seconds to extract if querying with media file
    #[arg(short = 's', long, default_value_t = 1.0)]
    pub nb_seconds_to_extract: f64,

    /// Maximum number of top matches to display
    #[arg(short = 'n', long, default_value_t = 5)]
    pub limit: usize,
}

#[derive(clap::Args, Debug)]
pub struct RetargetArgs {
    /// Path to reference subtitles file (.srt)
    #[arg(long)]
    pub ref_subtitles: PathBuf,

    /// Path to reference media
    #[arg(long)]
    pub ref_media: PathBuf,

    /// Path to target media
    #[arg(long)]
    pub target_media: PathBuf,

    /// Path to directory containing or storing .phash files
    #[arg(long, default_value = "/tmp")]
    pub phash_dir: PathBuf,

    /// Hamming search distance for BK-Tree
    #[arg(long, default_value_t = 0)]
    pub search_distance: u32,
}

#[derive(clap::Args, Debug)]
pub struct BenchVideoArgs {
    /// Media file to benchmark
    #[arg(short, long)]
    pub media: PathBuf,

    /// Duration in seconds to extract for benchmarking (defaults to 10.0s)
    #[arg(short, long, default_value_t = 10.0)]
    pub duration: f64,
}

#[derive(clap::Args, Debug)]
pub struct ExportPhashArgs {
    /// Media file to extract phash from
    #[arg(short, long)]
    pub media: PathBuf,

    /// Output .phash file (defaults to /tmp/<name>.<fps>fps.phash)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Optional duration in seconds to extract (defaults to full media)
    #[arg(short, long)]
    pub duration: Option<f64>,

    /// Optional sampling rate in FPS (e.g. 1.0 for sparse fast fingerprints)
    #[arg(long)]
    pub fps: Option<f64>,

    /// Extract keyframes (I-frames) only
    #[arg(long)]
    pub keyframes: bool,

    /// Hardware acceleration backend (auto, disabled, vaapi, cuda, qsv)
    #[arg(long, value_enum, default_value_t = CliHwAccel::Disabled)]
    pub hwaccel: CliHwAccel,
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

fn run_recognize_media(args: &RecognizeArgs) -> Result<()> {
    // 1. Probe video
    let meta = probe_video(&args.media)?;
    let start_sec = (meta.duration / 2.0).floor();
    let nb_frames = (args.nb_seconds_to_extract * meta.fps).round() as usize;

    // 2. Extract frames & compute pHashes in streaming
    let hashes = extract_frames_phash(&args.media, start_sec, nb_frames)?;

    let media_name_search = args
        .media
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

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
        .unwrap_or_else(|| "127.0.0.1".to_string());

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

fn find_or_build_phash(media_file: &Path, phash_dir: &Path) -> Result<(PathBuf, Vec<u64>, f64)> {
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Look for existing .phash file in phash_dir
    if phash_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(phash_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.contains(&stem) && name.ends_with(".phash") {
                        let hashes = read_phash_file(&p)?;
                        let meta = probe_video(media_file)?;
                        return Ok((p, hashes, meta.fps));
                    }
                }
            }
        }
    }

    // Otherwise probe and extract all frames
    let meta = probe_video(media_file)?;
    let total_frames = (meta.duration * meta.fps).floor() as usize;
    let hashes = extract_frames_phash(media_file, 0.0, total_frames)?;

    let out_name = format!(
        "{}.{:.1}fps.phash",
        media_file.file_name().unwrap().to_string_lossy(),
        meta.fps
    );
    let out_path = phash_dir.join(&out_name);
    write_phash_file(&out_path, &hashes)?;

    Ok((out_path, hashes, meta.fps))
}

fn run_retarget_subtitles(args: &RetargetArgs) -> Result<()> {
    let (ref_phash_file, ref_hashes, _ref_fps) =
        find_or_build_phash(&args.ref_media, &args.phash_dir).with_context(|| {
            format!("Failed to resolve reference phash for {:?}", args.ref_media)
        })?;

    let (target_phash_file, target_hashes, target_fps) =
        find_or_build_phash(&args.target_media, &args.phash_dir).with_context(|| {
            format!("Failed to resolve target phash for {:?}", args.target_media)
        })?;

    println!("ref_phash_file={:?}", ref_phash_file);
    println!(
        "search_tree_from_target=SearchTreeFromCuts(binary_img_hash_file={:?})",
        target_phash_file
    );

    // 1. Build BKTree for target cuts
    let t_bktree = Instant::now();
    let target_cuts = detect_cuts(&target_hashes, 16);
    let mut target_tree = BkTree::new();
    for cut in &target_cuts {
        target_tree.insert(BkItem {
            phash: cut.phash,
            offset: cut.offset,
        });
    }
    let bktree_elapsed = t_bktree.elapsed().as_secs_f64();
    println!(
        "Build BKTrees for target => took {:.4} seconds",
        bktree_elapsed
    );

    // 2. Search difference offsets
    let t_search = Instant::now();
    let ref_cuts = detect_cuts(&ref_hashes, 16);
    let mut diff_counter: HashMap<i64, usize> = HashMap::new();

    for ref_cut in &ref_cuts {
        let matches = target_tree.search(ref_cut.phash, args.search_distance);
        if let Some(first_match) = matches.first() {
            let diff = first_match.offset as i64 - ref_cut.offset as i64;
            *diff_counter.entry(diff).or_default() += 1;
        }
    }
    let search_elapsed = t_search.elapsed().as_secs_f64();

    let total_paired: usize = diff_counter.values().sum();
    let mut sorted_diffs: Vec<(i64, usize)> = diff_counter.into_iter().collect();
    sorted_diffs.sort_by_key(|a| std::cmp::Reverse(a.1));

    if let Some(&(most_common_diff, count)) = sorted_diffs.first() {
        let percent = (count as f64 / total_paired as f64) * 100.0;
        println!(
            "Nb paired frames used: {} frames ~ {:.16} seconds",
            total_paired,
            total_paired as f64 / 24.0
        );
        println!(
            "Percent of frames used for the most common: ~ {:.2} %",
            percent
        );
        println!(
            "Search differences offsets between reference and target => took {:.4} seconds",
            search_elapsed
        );
        println!(
            "Find differences offsets for synchronize subtitles (reference -> target) => took {:.4} seconds",
            bktree_elapsed + search_elapsed
        );

        let t_shift = Instant::now();
        let shift_frames = most_common_diff;
        let shift_ms = (shift_frames as f64 / target_fps) * 1000.0;
        println!("target_frame_rate={:.1} fps", target_fps);
        println!(
            "Shift subtitles seconds = {} frames ~ {:.4} seconds",
            shift_frames,
            shift_ms / 1000.0
        );

        let mut srt = SubtitleFile::open(&args.ref_subtitles)?;
        srt.shift(shift_ms);

        let out_name = target_phash_file
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.replace(".phash", ".srt"))
            .unwrap_or_else(|| "out.srt".to_string());
        let out_path = args.phash_dir.join(&out_name);
        srt.save(&out_path)?;

        println!("Exported subtitles for target in : \n'{:?}'", out_path);
        println!(
            "Shift subtitles (from reference) and save it (for target) => took {:.4} seconds",
            t_shift.elapsed().as_secs_f64()
        );
    } else {
        println!("Error: Can't find difference offsets !");
    }

    Ok(())
}

fn run_scan_library(args: &ScanLibraryArgs) -> Result<()> {
    let query_hashes = if let Some(ref media_path) = args.query_media {
        let meta = probe_video(media_path)?;
        let start_sec = (meta.duration / 2.0).floor();
        let nb_frames = (args.nb_seconds_to_extract * meta.fps).round() as usize;
        extract_frames_phash(media_path, start_sec, nb_frames)?
    } else if let Some(ref phash_path) = args.query_phash {
        read_phash_file(phash_path)?
    } else {
        anyhow::bail!("Either --query-media or --query-phash must be specified");
    };

    let start = Instant::now();
    let library = MmapLibrary::from_directory(&args.phash_dir)?;
    println!(
        "Loaded library from {:?}: {} files, {} total frames (took {:.2} ms)",
        args.phash_dir,
        library.file_count(),
        library.total_frames(),
        start.elapsed().as_secs_f64() * 1000.0
    );

    let search_start = Instant::now();
    let matches = library.recognize(&query_hashes, args.search_distance);
    let search_elapsed = search_start.elapsed().as_secs_f64();

    println!(
        "Search completed in {:.4} seconds ({} query frames)",
        search_elapsed,
        query_hashes.len()
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
                m.confidence * 100.0
            );
        }
    }
    Ok(())
}

fn run_export_phash(args: &ExportPhashArgs) -> Result<()> {
    let start = Instant::now();
    println!("Probing media {:?}...", args.media);
    let meta = probe_video(&args.media)?;
    println!(
        "Media duration: {:.2}s, FPS: {:.3}",
        meta.duration, meta.fps
    );

    let duration = args.duration.unwrap_or(meta.duration);
    let sampling = if args.keyframes {
        SamplingMode::KeyFrames
    } else if let Some(target_fps) = args.fps {
        SamplingMode::Fps(target_fps)
    } else {
        SamplingMode::Full
    };

    let total_frames = (duration * meta.fps).round() as usize;

    let opts = VideoExtractionOptions {
        start_sec: 0.0,
        duration_sec: if sampling == SamplingMode::Full {
            None
        } else {
            Some(duration)
        },
        max_frames: if sampling == SamplingMode::Full {
            Some(total_frames)
        } else {
            None
        },
        sampling,
        hwaccel: args.hwaccel.into(),
        custom_ffmpeg: None,
    };

    let out_path = args.output.clone().unwrap_or_else(|| {
        let name = args.media.file_name().unwrap().to_string_lossy();
        let suffix = match sampling {
            SamplingMode::KeyFrames => "keyframes.phash".to_string(),
            SamplingMode::Fps(fps) => format!("{:.1}fps.phash", fps),
            SamplingMode::Full => format!("{:.3}fps.phash", meta.fps),
        };
        PathBuf::from(format!("/tmp/{}.{}", name, suffix))
    });

    println!(
        "Extracting pHash ({:?}, hwaccel={:?}) -> {:?}...",
        sampling, args.hwaccel, out_path
    );

    let samples = extract_frames_advanced(&args.media, &opts)?;
    let elapsed = start.elapsed().as_secs_f64();
    let hashes: Vec<u64> = samples.into_iter().map(|s| s.phash).collect();
    println!(
        "Computed {} frame hashes in {:.2}s ({:.0} fps, {:.1}x real-time)",
        hashes.len(),
        elapsed,
        hashes.len() as f64 / elapsed.max(0.0001),
        duration / elapsed.max(0.0001)
    );

    write_phash_file(&out_path, &hashes)?;
    println!("✓ Successfully wrote binary phash to {:?}", out_path);
    Ok(())
}

fn run_bench_cuts(args: &BenchCutsArgs) -> Result<()> {
    use rayon::prelude::*;
    let mut files = Vec::new();
    if !args.phash_dir.exists() {
        anyhow::bail!("Directory {:?} does not exist", args.phash_dir);
    }
    for entry in std::fs::read_dir(&args.phash_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("phash") {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        anyhow::bail!("No .phash files found in {:?}", args.phash_dir);
    }

    let mode_str = if args.single_thread {
        "Single-threaded"
    } else {
        "Multi-threaded (Rayon)"
    };
    println!("=== RUST NATIVE BENCHMARK: SCENE CUTS DETECTION ===");
    println!("Mode                  : {}", mode_str);
    println!("Files                 : {}", files.len());
    println!("Cut threshold         : {}", args.threshold);

    let start = Instant::now();
    let (total_frames, total_cuts): (usize, usize) = if args.single_thread {
        let mut frames = 0;
        let mut cuts = 0;
        for f in &files {
            let hashes = read_phash_file(f)?;
            frames += hashes.len();
            let detected = detect_cuts(&hashes, args.threshold);
            cuts += detected.len();
        }
        (frames, cuts)
    } else {
        files
            .par_iter()
            .map(|f| -> Result<(usize, usize)> {
                let hashes = read_phash_file(f)?;
                let frames = hashes.len();
                let cuts = detect_cuts(&hashes, args.threshold).len();
                Ok((frames, cuts))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .fold((0, 0), |acc, (f, c)| (acc.0 + f, acc.1 + c))
    };

    let elapsed = start.elapsed().as_secs_f64();
    let fps = total_frames as f64 / elapsed;
    let video_hours = (total_frames as f64 / 24.0) / 3600.0;
    let speedup = (video_hours * 3600.0) / elapsed;

    println!("Total frames          : {:>12}", total_frames);
    println!("Total cuts detected   : {:>12}", total_cuts);
    println!("Elapsed time          : {:>12.4} s", elapsed);
    println!("Throughput            : {:>12.0} frames/s", fps);
    println!(
        "Average cuts / file   : {:>12.1}",
        total_cuts as f64 / files.len() as f64
    );
    println!(
        "Equivalent video time : {:>12.1} hours of 24fps media",
        video_hours
    );
    println!("Processing speedup    : {:>12.0}x real-time", speedup);
    println!("===================================================");

    Ok(())
}

fn run_import_phash(args: &ImportPhashArgs) -> Result<()> {
    let host = args
        .db_host
        .clone()
        .or_else(|| std::env::var("PSQL_IP").ok())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let mut db = DbClient::connect(
        &host,
        args.db_port,
        &args.db_user,
        &args.db_pass,
        &args.db_name,
    )?;

    println!("Ensuring BK-Tree schema & SP-GiST indexes in PostgreSQL...");
    db.init_schema()?;

    let mut target_files = Vec::new();
    if let Some(ref f) = args.file {
        target_files.push(f.clone());
    } else if let Some(ref dir) = args.phash_dir {
        if !dir.exists() {
            anyhow::bail!("Directory {:?} does not exist", dir);
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("phash") {
                target_files.push(path);
            }
        }
        target_files.sort();
    } else {
        anyhow::bail!("Either --file or --phash-dir must be specified");
    }

    if target_files.is_empty() {
        anyhow::bail!("No .phash files found to import");
    }

    println!(
        "Importing {} .phash file(s) into PostgreSQL BK-Tree...",
        target_files.len()
    );
    let start = Instant::now();
    let mut total_inserted = 0;
    let mut total_skipped = 0;

    for (idx, file_path) in target_files.iter().enumerate() {
        let t_file = Instant::now();
        let (media_id, nb_frames) = db.import_phash_file(file_path)?;
        let elapsed_file = t_file.elapsed().as_secs_f64();
        if nb_frames > 0 {
            total_inserted += nb_frames;
            let fps = nb_frames as f64 / elapsed_file;
            println!(
                "[{}/{}] Imported {:?} (media_id={}) -> {} frames in {:.3}s ({:.0} frames/s)",
                idx + 1,
                target_files.len(),
                file_path.file_name().unwrap_or_default(),
                media_id,
                nb_frames,
                elapsed_file,
                fps
            );
        } else {
            total_skipped += 1;
            println!(
                "[{}/{}] Skipped {:?} (already in DB, media_id={})",
                idx + 1,
                target_files.len(),
                file_path.file_name().unwrap_or_default(),
                media_id
            );
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let overall_fps = if elapsed > 0.0 {
        total_inserted as f64 / elapsed
    } else {
        0.0
    };
    println!("=== RUST POSTGRESQL IMPORT COMPLETE ===");
    println!("Total files processed : {}", target_files.len());
    println!(
        "Files newly inserted  : {}",
        target_files.len() - total_skipped
    );
    println!("Files skipped (exist) : {}", total_skipped);
    println!("Total frames inserted : {}", total_inserted);
    println!("Total elapsed time    : {:.3} s", elapsed);
    println!("Average throughput    : {:.0} frames/s", overall_fps);
    println!("=======================================");

    Ok(())
}

fn run_bench_video(args: &BenchVideoArgs) -> Result<()> {
    println!("Benchmarking video decoding on {:?}", args.media);
    let meta = probe_video(&args.media)?;
    println!(
        "Duration: {:.2}s, FPS: {:.2} (Total ~{} frames)\n",
        meta.duration,
        meta.fps,
        (meta.duration * meta.fps).round() as usize
    );

    let duration = args.duration.min(meta.duration);
    println!(
        "{:<26} | {:<12} | {:<12} | {:<14} | {:<10}",
        "Extraction Mode", "Frames", "Time (ms)", "Throughput", "Real-Time"
    );
    println!(
        "{:-<26}-+-{:-<12}-+-{:-<12}-+-{:-<14}-+-{:-<10}",
        "", "", "", "", ""
    );

    let benchmarks = [
        ("CPU Full (swscale)", SamplingMode::Full, HwAccel::Disabled),
        ("HWAccel Full", SamplingMode::Full, HwAccel::Auto),
        ("Sparse 1.0 FPS", SamplingMode::Fps(1.0), HwAccel::Auto),
        ("KeyFrames Only", SamplingMode::KeyFrames, HwAccel::Auto),
    ];

    for (name, sampling, hwaccel) in benchmarks {
        let opts = VideoExtractionOptions {
            start_sec: 0.0,
            duration_sec: Some(duration),
            max_frames: None,
            sampling,
            hwaccel,
            custom_ffmpeg: None,
        };

        let t0 = Instant::now();
        match extract_frames_advanced(&args.media, &opts) {
            Ok(samples) => {
                let elapsed_s = t0.elapsed().as_secs_f64();
                let elapsed_ms = elapsed_s * 1000.0;
                let nb_frames = samples.len();
                let fps = nb_frames as f64 / elapsed_s.max(0.0001);
                let speedup = duration / elapsed_s.max(0.0001);

                println!(
                    "{:<26} | {:<12} | {:<12.1} | {:<10.1} fps | {:<8.1}x",
                    name, nb_frames, elapsed_ms, fps, speedup
                );
            }
            Err(e) => {
                println!("{:<26} | FAILED ({})", name, e);
            }
        }
    }

    Ok(())
}

fn main() -> Result<()> {
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
            println!(
                "Throughput: {:.0} frames/sec ({:.2} µs / frame)",
                fps, per_frame_us
            );
        }
        Commands::BenchVideo(args) => {
            run_bench_video(&args)?;
        }
        Commands::RecognizeMedia(args) => {
            run_recognize_media(&args)?;
        }
        Commands::RetargetSubtitles(args) => {
            run_retarget_subtitles(&args)?;
        }
        Commands::ExportPhash(args) => {
            run_export_phash(&args)?;
        }
        Commands::ScanLibrary(args) => {
            run_scan_library(&args)?;
        }
        Commands::BenchCuts(args) => {
            run_bench_cuts(&args)?;
        }
        Commands::ImportPhash(args) => {
            run_import_phash(&args)?;
        }
    }
    Ok(())
}
