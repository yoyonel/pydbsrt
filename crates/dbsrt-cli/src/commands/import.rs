//! PostgreSQL .phash import subcommand.

use anyhow::Result;
use clap::Args;
use std::path::PathBuf;
use std::time::Instant;

use crate::cli::PostgresArgs;
use crate::commands::connect_db;
use crate::constants::PHASH_EXT;

#[derive(Args, Debug)]
pub struct ImportPhashArgs {
    /// Single .phash file to import
    #[arg(short = 'r', long, alias = "binary-img-hash-file")]
    pub file: Option<PathBuf>,

    /// Directory of .phash files to batch import
    #[arg(short = 'p', long)]
    pub phash_dir: Option<PathBuf>,

    /// PostgreSQL connection arguments
    #[command(flatten)]
    pub db: PostgresArgs,
}

/// Run batch or single .phash import into PostgreSQL BK-Tree table.
pub fn run_import_phash(args: &ImportPhashArgs) -> Result<()> {
    let mut target_files = Vec::new();
    if let Some(ref f) = args.file {
        if !f.exists() {
            anyhow::bail!("File {:?} does not exist", f);
        }
        target_files.push(f.clone());
    } else if let Some(ref dir) = args.phash_dir {
        if !dir.exists() {
            anyhow::bail!("Directory {:?} does not exist", dir);
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some(PHASH_EXT) {
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

    let mut db = connect_db(&args.db)?;

    println!("Ensuring BK-Tree schema & SP-GiST indexes in PostgreSQL...");
    db.init_schema()?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{
        DEFAULT_POSTGRES_DB, DEFAULT_POSTGRES_HOST, DEFAULT_POSTGRES_PASS, DEFAULT_POSTGRES_PORT,
        DEFAULT_POSTGRES_USER, STANDARD_POSTGRES_PORT, TEST_UNREACHABLE_PORT,
    };
    use dbsrt_core::write_phash_file;
    use dbsrt_index::DbClient;

    fn create_test_import_args(
        file: Option<PathBuf>,
        phash_dir: Option<PathBuf>,
    ) -> ImportPhashArgs {
        ImportPhashArgs {
            file,
            phash_dir,
            db: PostgresArgs::default(),
        }
    }

    #[test]
    fn test_import_phash_missing_args() {
        let args = create_test_import_args(None, None);
        let err = run_import_phash(&args).unwrap_err();
        assert!(err
            .to_string()
            .contains("Either --file or --phash-dir must be specified"));
    }

    #[test]
    fn test_import_phash_nonexistent_file() {
        let args = create_test_import_args(Some(PathBuf::from("/nonexistent/file.phash")), None);
        let err = run_import_phash(&args).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn test_import_phash_nonexistent_dir() {
        let args = create_test_import_args(None, Some(PathBuf::from("/nonexistent/dir")));
        let err = run_import_phash(&args).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn test_import_phash_empty_dir() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_import_empty_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let args = create_test_import_args(None, Some(temp_dir.clone()));
        let err = run_import_phash(&args).unwrap_err();
        assert!(err.to_string().contains("No .phash files found to import"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_import_phash_db_connection_failure() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_import_conn_fail_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let phash_path = temp_dir.join("sample.25.0fps.phash");
        write_phash_file(&phash_path, &[0x1111_2222_3333_4444_u64]).unwrap();

        let args = ImportPhashArgs {
            db: PostgresArgs {
                host: Some(DEFAULT_POSTGRES_HOST.to_string()),
                port: TEST_UNREACHABLE_PORT,
                ..Default::default()
            },
            ..create_test_import_args(Some(phash_path), None)
        };
        assert!(run_import_phash(&args).is_err());
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_import_phash_live_db() {
        let _guard = crate::commands::LIVE_DB_TEST_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let host = std::env::var("PSQL_IP").unwrap_or_else(|_| DEFAULT_POSTGRES_HOST.to_string());
        let env_port = std::env::var("PSQL_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok());
        let ports: Vec<u16> = match env_port {
            Some(p) => vec![p, DEFAULT_POSTGRES_PORT, STANDARD_POSTGRES_PORT],
            None => vec![DEFAULT_POSTGRES_PORT, STANDARD_POSTGRES_PORT],
        };
        let connect = |port: u16| {
            DbClient::connect(
                &host,
                port,
                DEFAULT_POSTGRES_USER,
                DEFAULT_POSTGRES_PASS,
                DEFAULT_POSTGRES_DB,
            )
        };
        let Some(port) = ports.into_iter().find(|&p| connect(p).is_ok()) else {
            eprintln!("PostgreSQL BK-Tree container not available, skipping live import test");
            return;
        };

        if let Ok(mut client) = connect(port) {
            let _ = client.init_schema();
        }

        let temp_dir =
            std::env::temp_dir().join(format!("test_import_live_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let synth_file = temp_dir.join(format!(
            "unit_test_cli_import_{}.25.0fps.phash",
            std::process::id()
        ));
        // Unique content per run: an orphan row from a crashed run would otherwise match by
        // media_hash and silently short-circuit the insert path.
        let hashes =
            vec![0x1234_5678_0000_0000_u64 | (u64::from(std::process::id()) & 0xFFFF_FFFF); 20];
        write_phash_file(&synth_file, &hashes).unwrap();

        // 1. Single file import
        let args_file = ImportPhashArgs {
            db: PostgresArgs {
                host: Some(host.clone()),
                port,
                ..Default::default()
            },
            ..create_test_import_args(Some(synth_file.clone()), None)
        };
        run_import_phash(&args_file).expect("run_import_phash file failed");

        // 2. Directory import (re-import triggers idempotent skip branch)
        let args_dir = ImportPhashArgs {
            db: PostgresArgs {
                host: Some(host.clone()),
                port,
                ..Default::default()
            },
            ..create_test_import_args(None, Some(temp_dir.clone()))
        };
        run_import_phash(&args_dir).expect("run_import_phash directory failed");

        // Cleanup DB row
        if let Ok(mut db) = connect(port) {
            let stem = synth_file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let _ = db.delete_media_by_name(stem);
        }
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
