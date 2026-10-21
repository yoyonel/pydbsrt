//! Remote subtitle search and download subcommand (OpenSubtitles REST / Mock).

use anyhow::{Context, Result};
use clap::Args;
use dbsrt_core::{
    MediaMetadata, MockSubtitleProvider, OpenSubtitlesProvider, SubtitleCache, SubtitleProvider,
};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::constants::{
    DEFAULT_SUBTITLE_LANG, DEFAULT_SUBTITLE_PROVIDER, FETCH_MAX_CANDIDATES_DISPLAY,
    FETCH_MAX_DOWNLOAD_ATTEMPTS,
};

#[derive(Args, Debug)]
pub struct FetchSubtitlesArgs {
    /// Media file path or search title query (e.g. "Silicon Valley S02E01" or path/to/video.mkv)
    #[arg(short, long)]
    pub query: String,

    /// Target subtitle language code (e.g. "fr", "en", "es")
    #[arg(short, long, default_value = DEFAULT_SUBTITLE_LANG)]
    pub language: String,

    /// Output SRT file path (default: stdout or <media>.<lang>.srt if file exists)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Subtitle provider ("opensubtitles", "mock")
    #[arg(long, default_value = DEFAULT_SUBTITLE_PROVIDER)]
    pub provider: String,

    /// OpenSubtitles API key (or set OPEN_SUBTITLES_API_KEY env)
    #[arg(long)]
    pub api_key: Option<String>,

    /// Local cache directory (defaults to ~/.cache/dbsrt/subtitles)
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Bypass local disk cache
    #[arg(long)]
    pub no_cache: bool,

    /// Dry run: search and display ranked candidates without downloading
    #[arg(long)]
    pub dry_run: bool,
}

/// Run remote subtitle query, score ranking, caching, and download.
pub async fn run_fetch_subtitles(args: &FetchSubtitlesArgs) -> Result<()> {
    let start = Instant::now();
    let query_path = Path::new(&args.query);
    let meta = if query_path.exists() {
        MediaMetadata::parse_from_path(query_path)
    } else {
        MediaMetadata::parse(&args.query)
    };

    println!("=== DBS-RT FETCH SUBTITLES ===");
    println!("Target Query   : {}", args.query);
    println!("Clean Title    : {}", meta.clean_title);
    if let (Some(s), Some(e)) = (meta.season, meta.episode) {
        println!("Series Episode : S{:02}E{:02}", s, e);
    } else if let Some(year) = meta.year {
        println!("Release Year   : {}", year);
    }
    if let Some(ref grp) = meta.release_group {
        println!("Release Group  : {}", grp);
    }
    println!("Language       : {}", args.language);
    println!("Provider       : {}", args.provider);

    let provider: Box<dyn SubtitleProvider> = if args.provider.to_lowercase() == "mock" {
        Box::new(MockSubtitleProvider::new())
    } else {
        Box::new(OpenSubtitlesProvider::new(args.api_key.clone()))
    };

    let cache = SubtitleCache::new(args.cache_dir.clone());
    println!("Cache Base Dir : {:?}", cache.base_dir());

    println!("\nSearching remote provider '{}'...", provider.name());
    let candidates = match provider.search(&meta, &args.language).await {
        Ok(cands) => cands,
        Err(e) => {
            eprintln!("Search error from provider: {:#}", e);
            anyhow::bail!("Provider search failed: {:#}", e);
        }
    };

    if candidates.is_empty() {
        println!(
            "No subtitle candidates found for '{}' ({}).",
            meta.clean_title, args.language
        );
        return Ok(());
    }

    println!(
        "Found {} candidates (ranked by heuristic match score):",
        candidates.len()
    );
    for (idx, cand) in candidates
        .iter()
        .enumerate()
        .take(FETCH_MAX_CANDIDATES_DISPLAY)
    {
        let cached_mark = if cache.exists(&meta, &args.language, &cand.subtitle_id) {
            "[CACHED]"
        } else {
            "        "
        };
        println!(
            "  [{:>2}] Score: {:.2} {} ID: {:<12} Release: {}",
            idx + 1,
            cand.match_score,
            cached_mark,
            cand.subtitle_id,
            cand.release_name.as_deref().unwrap_or("<unknown>")
        );
    }

    if args.dry_run {
        println!("\nDry run mode enabled. Subtitle was not downloaded.");
        return Ok(());
    }

    let mut chosen_content = None;
    let mut chosen_cand = None;

    for cand in candidates.iter().take(FETCH_MAX_DOWNLOAD_ATTEMPTS) {
        if !args.no_cache {
            if let Some(cached) = cache.get(&meta, &args.language, &cand.subtitle_id) {
                println!(
                    "\nUsing cached subtitle ID '{}' (bypassing network).",
                    cand.subtitle_id
                );
                chosen_content = Some(cached);
                chosen_cand = Some(cand);
                break;
            }
        }

        println!("\nDownloading candidate ID '{}'...", cand.subtitle_id);
        match provider.download(cand).await {
            Ok(content) => {
                let _ = cache.put(&meta, &args.language, &cand.subtitle_id, &content);
                chosen_content = Some(content);
                chosen_cand = Some(cand);
                break;
            }
            Err(e) => {
                eprintln!(
                    "Download failed for ID '{}': {:#}. Trying next candidate...",
                    cand.subtitle_id, e
                );
            }
        }
    }

    let (content, cand) = match (chosen_content, chosen_cand) {
        (Some(c), Some(cd)) => (c, cd),
        _ => anyhow::bail!("Failed to download any candidate subtitles."),
    };

    let output_path = if let Some(ref out) = args.output {
        Some(out.clone())
    } else if query_path.exists() {
        let parent = query_path.parent().unwrap_or_else(|| Path::new("."));
        let stem = query_path.file_stem().unwrap_or_default().to_string_lossy();
        Some(parent.join(format!("{}.{}.srt", stem, args.language)))
    } else {
        None
    };

    if let Some(ref path) = output_path {
        std::fs::write(path, &content)
            .with_context(|| format!("Failed to write subtitle to {:?}", path))?;
        println!("Saved subtitle to: {:?}", path);
    } else {
        println!("\n--- SUBTITLE CONTENT START ---");
        println!("{}", content);
        println!("--- SUBTITLE CONTENT END ---");
    }

    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "Completed in {:.3} s (Subtitle ID: {})",
        elapsed, cand.subtitle_id
    );

    Ok(())
}
