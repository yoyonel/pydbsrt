//! Subtitle coupling certification subcommand for Zero-DB couplings.json and PostgreSQL.

use anyhow::Result;
use clap::Args;
use dbsrt_core::{
    build_and_validate_sub_frames_opts, compute_bytes_digest_32, compute_media_phash_digest,
    read_phash_file, CertifiedSubtitleRecord, CouplingRegistry, SubtitleFile,
};
use dbsrt_index::{parse_fps_from_phash_stem, DbClient};
use std::path::PathBuf;

use crate::commands::retarget::find_or_build_phash;
use crate::constants::{
    AUTO_SUBTITLE_LANG, DEFAULT_NTSC_FILM_FPS, DEFAULT_PAL_FPS, DEFAULT_POSTGRES_DB,
    DEFAULT_POSTGRES_HOST, DEFAULT_POSTGRES_PASS, DEFAULT_POSTGRES_PORT, DEFAULT_POSTGRES_USER,
    DEFAULT_SUBTITLES_DIR, DEFAULT_TMP_DIR, EPSILON_FPS,
};

#[derive(Args, Debug)]
pub struct CertifyCouplingArgs {
    /// Reference media file (.mp4, .mkv, .webm) to extract/locate .phash for
    #[arg(short, long, alias = "ref-media")]
    pub media: Option<PathBuf>,

    /// Existing .phash file (if certifying directly against a .phash without the raw video)
    #[arg(long, alias = "binary-img-hash-file")]
    pub phash_file: Option<PathBuf>,

    /// Reference subtitle file (.srt) certified as synchronized with the media
    #[arg(short, long, alias = "ref-subtitles")]
    pub subtitle: PathBuf,

    /// Subtitle language code (e.g. "fr", "en", or "auto" to detect from filename)
    #[arg(short, long, default_value = AUTO_SUBTITLE_LANG)]
    pub lang: String,

    /// Override framerate (e.g. 23.976, 24.0, 25.0)
    #[arg(long)]
    pub fps: Option<f64>,

    /// Directory storing certified .srt files and couplings.json
    #[arg(long, default_value = DEFAULT_SUBTITLES_DIR)]
    pub subtitles_dir: PathBuf,

    /// Directory storing .phash files
    #[arg(long, default_value = DEFAULT_TMP_DIR)]
    pub phash_dir: PathBuf,

    /// Allow subtitle cues that exceed the video frame count (disables strict out-of-bounds rejection)
    #[arg(long)]
    pub allow_oob: bool,

    /// Suppress verbose details and output only on error
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Also import the relational coupling (`medias`, `frames`, `subtitles`, `sub_frames`) into PostgreSQL
    #[arg(long)]
    pub use_db: bool,

    /// Import only into PostgreSQL without copying the .srt file or modifying couplings.json
    #[arg(long)]
    pub db_only: bool,

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
}

/// Helper detecting subtitle ISO 639-1 language code from filename tokens.
pub fn detect_cli_subtitle_lang(filename: &str, fallback: &str) -> String {
    if fallback != AUTO_SUBTITLE_LANG && !fallback.is_empty() {
        return fallback.to_lowercase();
    }
    let stem = filename
        .strip_suffix(".srt")
        .or_else(|| filename.strip_suffix(".SRT"))
        .unwrap_or(filename)
        .to_lowercase();
    for token in stem.rsplit(['.', '_', '-']) {
        if token.chars().all(|c| c.is_ascii_digit())
            || matches!(
                token,
                "forced" | "sdh" | "hi" | "shift" | "resync" | "synced" | "track" | "sub" | "subs"
            )
        {
            continue;
        }
        match token {
            "fr" | "fre" | "fra" | "french" | "vff" | "vf" | "vostfr" => return "fr".to_string(),
            "en" | "eng" | "english" => return "en".to_string(),
            "es" | "spa" | "spanish" => return "es".to_string(),
            "de" | "ger" | "deu" | "german" => return "de".to_string(),
            "it" | "ita" | "italian" => return "it".to_string(),
            "pt" | "por" | "pob" | "portuguese" => return "pt".to_string(),
            _ => break,
        }
    }
    "en".to_string()
}

/// Run subtitle coupling certification, registering ground-truth frames in couplings.json / PostgreSQL.
pub fn run_certify_coupling(args: &CertifyCouplingArgs) -> Result<()> {
    if !args.subtitle.exists() {
        anyhow::bail!("Subtitle file not found: {:?}", args.subtitle);
    }

    let (phash_path, hashes, detected_fps) = if let Some(ref pf) = args.phash_file {
        if !pf.exists() {
            anyhow::bail!("Specified .phash file not found: {:?}", pf);
        }
        let hashes = read_phash_file(pf)?;
        let stem = pf
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .trim_end_matches(".phash")
            .to_string();
        let fps = parse_fps_from_phash_stem(&stem);
        (pf.clone(), hashes, fps)
    } else if let Some(ref media) = args.media {
        if !media.exists() {
            anyhow::bail!("Media file not found: {:?}", media);
        }
        find_or_build_phash(media, &args.phash_dir)?
    } else {
        anyhow::bail!("Either --media or --phash-file must be provided");
    };

    let initial_fps = args.fps.unwrap_or(detected_fps);
    let srt_file = SubtitleFile::open(&args.subtitle)?;
    let strict = !args.allow_oob;
    let (sub_frames, fps) =
        match build_and_validate_sub_frames_opts(&srt_file, &hashes, initial_fps, args.allow_oob) {
            Ok(anchors) => (anchors, initial_fps),
            Err(e) if args.fps.is_none() && (initial_fps - DEFAULT_PAL_FPS).abs() < EPSILON_FPS => {
                if let Ok(anchors) = build_and_validate_sub_frames_opts(
                    &srt_file,
                    &hashes,
                    DEFAULT_NTSC_FILM_FPS,
                    args.allow_oob,
                ) {
                    (anchors, DEFAULT_NTSC_FILM_FPS)
                } else {
                    return Err(e);
                }
            }
            Err(e) => return Err(e),
        };

    let srt_fname = args
        .subtitle
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let lang = detect_cli_subtitle_lang(&srt_fname, &args.lang);

    let phash_fname = phash_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let media_name = phash_fname
        .strip_suffix(".phash")
        .unwrap_or(&phash_fname)
        .to_string();

    let dest_srt_path = if args.db_only || args.subtitle.starts_with(&args.subtitles_dir) {
        args.subtitle.clone()
    } else {
        std::fs::create_dir_all(&args.subtitles_dir)?;
        let dest = args.subtitles_dir.join(&srt_fname);
        if dest != args.subtitle {
            std::fs::copy(&args.subtitle, &dest)?;
        }
        dest
    };

    let raw_srt_bytes = std::fs::read(&dest_srt_path)?;
    let media_hash = compute_media_phash_digest(&hashes);
    let subtitles_hash = compute_bytes_digest_32(&raw_srt_bytes);
    let rel_srt_name = dest_srt_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let reg_path = CouplingRegistry::default_path(&args.subtitles_dir);
    if !args.db_only {
        std::fs::create_dir_all(&args.subtitles_dir)?;
        let mut reg = CouplingRegistry::load_or_default(&reg_path)?;
        reg.upsert_coupling(
            media_hash.clone(),
            media_name.clone(),
            phash_fname.clone(),
            fps,
            hashes.len(),
            CertifiedSubtitleRecord {
                name: rel_srt_name.clone(),
                srt_path: rel_srt_name.clone(),
                lang: lang.clone(),
                subtitles_hash: subtitles_hash.clone(),
                certified: true,
                cue_count: srt_file.cues.len(),
                sub_frames: sub_frames.clone(),
            },
        );
        reg.save(&reg_path)?;
    }

    if !args.quiet {
        println!("=== CERTIFIED COUPLING REGISTERED ===");
        println!("Media Name       : {}", media_name);
        println!(
            "pHash File       : {:?} ({} frames @ {:.2} fps)",
            phash_path,
            hashes.len(),
            fps
        );
        println!("Media Digest     : {}", media_hash);
        println!("Subtitle File    : {:?} (lang: {})", dest_srt_path, lang);
        println!("Subtitle Digest  : {}", subtitles_hash);
        println!(
            "Validated Cues   : {} cues -> {} frame anchors (start/end offsets)",
            srt_file.cues.len(),
            sub_frames.len()
        );
        if !args.db_only {
            println!("Local Registry   : {:?}", reg_path);
        }
    }

    if args.db_only || args.use_db || args.db_host.is_some() {
        let host = args
            .db_host
            .clone()
            .or_else(|| std::env::var("PSQL_IP").ok())
            .unwrap_or_else(|| DEFAULT_POSTGRES_HOST.to_string());
        match DbClient::connect(
            &host,
            args.db_port,
            &args.db_user,
            &args.db_pass,
            &args.db_name,
        ) {
            Ok(mut client) => {
                client.init_schema()?;
                let (media_id, sub_id, inserted_anchors) =
                    client.import_subtitle_coupling(&phash_path, &dest_srt_path, &lang, strict)?;
                if !args.quiet {
                    println!(
                        "PostgreSQL Sync  : Imported media_id={}, sub_id={}, sub_frames={}",
                        media_id, sub_id, inserted_anchors
                    );
                }
            }
            Err(e) => {
                if args.use_db || args.db_only {
                    return Err(e.context("Failed to connect to PostgreSQL"));
                }
                if !args.quiet {
                    eprintln!("PostgreSQL Sync  : Skipped (DB unreachable: {})", e);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_cli_subtitle_lang_tokens() {
        assert_eq!(detect_cli_subtitle_lang("movie.en.srt", "auto"), "en");
        assert_eq!(detect_cli_subtitle_lang("movie.fre.srt", "auto"), "fr");
        assert_eq!(detect_cli_subtitle_lang("movie.vostfr.srt", "auto"), "fr");
        assert_eq!(detect_cli_subtitle_lang("movie.es.srt", "auto"), "es");
        assert_eq!(detect_cli_subtitle_lang("movie.ger.srt", "auto"), "de");
        assert_eq!(detect_cli_subtitle_lang("movie.ita.srt", "auto"), "it");
        assert_eq!(detect_cli_subtitle_lang("movie.por.srt", "auto"), "pt");
        assert_eq!(
            detect_cli_subtitle_lang("movie.forced.fr.srt", "auto"),
            "fr"
        );
        assert_eq!(detect_cli_subtitle_lang("movie.unknown.srt", "auto"), "en");
        assert_eq!(detect_cli_subtitle_lang("movie.srt", "auto"), "en");
        assert_eq!(detect_cli_subtitle_lang("movie.srt", "fr"), "fr");
    }
}
