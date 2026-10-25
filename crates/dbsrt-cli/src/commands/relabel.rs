//! Subtitle coupling relabeling subcommand for Zero-DB couplings.json and optional PostgreSQL sync.

use anyhow::{bail, Result};
use clap::Args;
use dbsrt_core::{CouplingRegistry, LangStatus};
use std::path::PathBuf;

use crate::cli::PostgresArgs;
use crate::commands::connect_db;
use crate::constants::DEFAULT_SUBTITLES_DIR;

#[derive(Args, Debug, Clone)]
pub struct RelabelCouplingArgs {
    /// Media selector: 32-char media hash, hash prefix, media name, or phash filename
    #[arg(short = 'm', long = "media", alias = "media-hash")]
    pub media: String,

    /// Subtitle selector: 32-char subtitles hash, hash prefix, or srt filename (optional if media has only 1 subtitle)
    #[arg(short = 's', long = "sub", alias = "sub-hash", alias = "subtitle")]
    pub sub: Option<String>,

    /// Target ISO-639-1 language code (e.g. "fr", "en", "es") or "und"
    #[arg(short = 'l', long = "lang")]
    pub lang: String,

    /// Language confidence status override ("confirmed", "detected", "unknown")
    #[arg(long = "status", default_value = "confirmed")]
    pub status: String,

    /// Directory storing certified .srt files and couplings.json
    #[arg(long, default_value = DEFAULT_SUBTITLES_DIR)]
    pub subtitles_dir: PathBuf,

    /// Output results as JSON
    #[arg(long)]
    pub json: bool,

    /// Also update PostgreSQL relational table `subtitles.lang` if connected
    #[arg(long)]
    pub use_db: bool,

    /// PostgreSQL connection arguments
    #[command(flatten)]
    pub db: PostgresArgs,
}

pub fn run_relabel_coupling(args: RelabelCouplingArgs) -> Result<()> {
    let status = match args.status.to_lowercase().as_str() {
        "confirmed" => LangStatus::Confirmed(args.lang.to_lowercase()),
        "detected" => LangStatus::Detected(args.lang.to_lowercase()),
        "unknown" => LangStatus::Unknown,
        other => bail!(
            "Invalid --status '{}'. Expected 'confirmed', 'detected', or 'unknown'",
            other
        ),
    };

    let registry_path = CouplingRegistry::default_path(&args.subtitles_dir);
    if !registry_path.exists() {
        bail!(
            "Coupling registry not found at {:?}. No couplings to relabel.",
            registry_path
        );
    }

    let mut reg = CouplingRegistry::load_or_default(&registry_path)?;
    let result = reg.relabel(&args.media, args.sub.as_deref(), &args.lang, status)?;

    reg.save(&registry_path)?;

    // Optional PostgreSQL sync
    if args.use_db || args.db.host.is_some() {
        if let Ok(mut client) = connect_db(&args.db) {
            let _ = client.client_mut().execute(
                "UPDATE subtitles SET lang = $1 WHERE subtitles_hash = $2",
                &[&result.new_lang, &result.subtitles_hash],
            );
        }
    }

    if args.json {
        let out = serde_json::json!({
            "success": true,
            "media_hash": result.media_hash,
            "media_name": result.media_name,
            "subtitles_hash": result.subtitles_hash,
            "subtitle_name": result.subtitle_name,
            "old_lang": result.old_lang,
            "old_status": result.old_status,
            "new_lang": result.new_lang,
            "new_status": result.new_status,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("✓ Relabeled coupling in {:?}:", registry_path);
        println!(
            "   Media        : {} ({})",
            result.media_name, result.media_hash
        );
        println!(
            "   Subtitle     : {} ({})",
            result.subtitle_name, result.subtitles_hash
        );
        println!(
            "   Language     : {} -> {}",
            result.old_lang, result.new_lang
        );
        println!(
            "   Status       : {:?} -> {:?}",
            result.old_status, result.new_status
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::{CertifiedSubtitleRecord, LangStatus};
    use std::fs;

    #[test]
    fn test_relabel_coupling_cli_unit() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_cli_relabel_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let registry_path = CouplingRegistry::default_path(&temp_dir);
        let mut reg = CouplingRegistry::default();
        let sub = CertifiedSubtitleRecord {
            subtitles_hash: "1234567890abcdef1234567890abcdef".to_string(),
            name: "ep01.srt".to_string(),
            lang: "und".to_string(),
            lang_status: LangStatus::Unknown,
            srt_path: "ep01.srt".to_string(),
            certified: true,
            cue_count: 100,
            sub_frames: vec![],
        };
        reg.upsert_coupling(
            "mediahash1234567890abcdef123456".to_string(),
            "Show.S01E01".to_string(),
            "Show.S01E01.phash".to_string(),
            23.976,
            500,
            sub,
        );
        reg.save(&registry_path).unwrap();

        let args = RelabelCouplingArgs {
            media: "Show.S01E01".to_string(),
            sub: None,
            lang: "fr".to_string(),
            status: "confirmed".to_string(),
            subtitles_dir: temp_dir.clone(),
            json: true,
            use_db: false,
            db: PostgresArgs::default(),
        };

        run_relabel_coupling(args).unwrap();

        let reloaded = CouplingRegistry::load_or_default(&registry_path).unwrap();
        assert_eq!(reloaded.couplings[0].subtitles[0].lang, "fr");
        assert_eq!(
            reloaded.couplings[0].subtitles[0].lang_status,
            LangStatus::Confirmed("fr".to_string())
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
