//! PostgreSQL .phash import subcommand.

use anyhow::Result;
use clap::Args;
use dbsrt_index::DbClient;
use std::path::PathBuf;
use std::time::Instant;

use crate::constants::{
    DEFAULT_POSTGRES_DB, DEFAULT_POSTGRES_HOST, DEFAULT_POSTGRES_PASS, DEFAULT_POSTGRES_PORT,
    DEFAULT_POSTGRES_USER,
};

#[derive(Args, Debug)]
pub struct ImportPhashArgs {
    /// Single .phash file to import
    #[arg(short = 'r', long, alias = "binary-img-hash-file")]
    pub file: Option<PathBuf>,

    /// Directory of .phash files to batch import
    #[arg(short = 'p', long)]
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
}

/// Run batch or single .phash import into PostgreSQL BK-Tree table.
pub fn run_import_phash(args: &ImportPhashArgs) -> Result<()> {
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
