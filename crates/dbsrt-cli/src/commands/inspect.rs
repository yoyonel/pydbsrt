//! Media container inspection and embedded track extraction subcommand.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{extract_embedded_subtitle, inspect_container, MediaMetadata};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct InspectMediaArgs {
    /// Media file to inspect
    #[arg(short, long, alias = "ref_media")]
    pub media: PathBuf,

    /// Optional target language code to extract embedded subtitle track (e.g. "fr", "en")
    #[arg(short = 'e', long)]
    pub extract_subtitle: Option<String>,

    /// Output format (json or text)
    #[arg(long, default_value = "text")]
    pub format: String,
}

/// Run media metadata parsing and container track inspection.
pub fn run_inspect_media(args: &InspectMediaArgs) -> Result<()> {
    if !args.media.exists() {
        anyhow::bail!("Media file not found: {:?}", args.media);
    }

    let meta = MediaMetadata::parse_from_path(&args.media);
    let inspection = inspect_container(&args.media).ok();

    if args.format.to_lowercase() == "json" {
        let val = serde_json::json!({
            "metadata": meta,
            "container": inspection,
        });
        println!("{}", serde_json::to_string_pretty(&val)?);
        return Ok(());
    }

    println!("=== Media Release Metadata ===");
    println!("Raw Title:     {}", meta.raw_title);
    println!("Clean Title:   {}", meta.clean_title);
    println!("Media Type:    {:?}", meta.media_type);
    if let Some(y) = meta.year {
        println!("Year:          {}", y);
    }
    if let Some(s) = meta.season {
        println!("Season:        {}", s);
    }
    if let Some(e) = meta.episode {
        println!("Episode:       {}", e);
    }
    if let Some(res) = &meta.resolution {
        println!("Resolution:    {}", res);
    }
    if let Some(src) = &meta.source {
        println!("Source:        {}", src);
    }
    if let Some(grp) = &meta.release_group {
        println!("Release Group: {}", grp);
    }

    if let Some(insp) = &inspection {
        println!(
            "\n=== Container Subtitle Tracks ({}) ===",
            insp.subtitle_streams.len()
        );
        for s in &insp.subtitle_streams {
            let def_tag = if s.is_default { " [DEFAULT]" } else { "" };
            let frc_tag = if s.is_forced { " [FORCED]" } else { "" };
            println!(
                "  Stream #{:<2} (sub:{}): lang={:<4} codec={:<8} title='{}'{}{}",
                s.index,
                s.subtitle_index,
                s.language.as_deref().unwrap_or("und"),
                s.codec_name,
                s.title.as_deref().unwrap_or(""),
                def_tag,
                frc_tag
            );
        }
    }

    if let Some(target_lang) = &args.extract_subtitle {
        if let Some(insp) = &inspection {
            if let Some(stream) = insp.find_by_language(target_lang) {
                println!(
                    "\n=== Extracting Subtitle Track (lang: {}) ===",
                    target_lang
                );
                let srt = extract_embedded_subtitle(&args.media, stream.subtitle_index)?;
                println!("{}", srt);
            } else {
                println!(
                    "\nNo embedded subtitle track matching '{}' found in container.",
                    target_lang
                );
            }
        }
    }

    Ok(())
}
