//! CLI subcommand `dbsrt pack-ahash` compiling loose `.ahash` files into `catalog.dbahash` v1.

use anyhow::{bail, Result};
use clap::Args;
use dbsrt_index::{
    pack_ahash_directories, DbahashPackOptions, DbahashPackStats, DEFAULT_DBAHASH_FACTOR_K,
};
use std::path::PathBuf;

use crate::constants::{BYTES_PER_MIB, DEFAULT_AHASH_DATA_DIR, DEFAULT_DBAHASH_OUTPUT_PATH};

/// Arguments for the `dbsrt pack-ahash` binary archive compiler.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PackAhashArgs {
    /// Primary directory containing `.ahash` files and `<stem>.json` sidecars.
    #[arg(short = 'a', long = "ahash-dir", default_value = DEFAULT_AHASH_DATA_DIR)]
    pub ahash_dir: PathBuf,

    /// Additional directories scanned for `.ahash` files.
    #[arg(long = "extra-dir")]
    pub extra_dirs: Vec<PathBuf>,

    /// Destination path for the compiled `.dbahash` v1 binary archive.
    #[arg(short = 'o', long = "output", default_value = DEFAULT_DBAHASH_OUTPUT_PATH)]
    pub output: PathBuf,

    /// Coarse decimation stride K (`86.125 / K` fps; default `32` = `2.69 fps`).
    #[arg(short = 'k', long = "factor-k", default_value_t = DEFAULT_DBAHASH_FACTOR_K)]
    pub factor_k: usize,

    /// Omit pre-sorted `MIH-32` coarse tables from the archive (dense-only `.dbahash`).
    #[arg(long = "no-mih32", default_value_t = false)]
    pub no_mih32: bool,
}

/// Compiles `.ahash` directories into a single memory-mapped `.dbahash` v1 binary archive.
///
/// # Arguments
/// * `args` - Parsed [`PackAhashArgs`] CLI flags.
///
/// # Returns
/// [`DbahashPackStats`] summarizing scanned tracks, deduplicated twins, frames, and file size.
///
/// # Errors
/// Returns `Err` if `factor_k == 0`, no valid `.ahash` tracks are found, or atomic output fails.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Invokes [`pack_ahash_directories`] in two streaming passes (`64 KiB` dense buffer per worker),
/// preserving secondary multi-audio tracks (`.a1`, `.a2`) while collapsing byte-identical twins.
pub fn run_pack_ahash(args: &PackAhashArgs) -> Result<DbahashPackStats> {
    let mut ahash_dirs = vec![args.ahash_dir.clone()];
    ahash_dirs.extend(args.extra_dirs.clone());

    println!("=== DBS-RT COMPILER: UNIFIED .DBAHASH v1 PACKER ===");
    println!("Input directories : {:?}", ahash_dirs);
    println!("Output archive    : {:?}", args.output);
    println!("Coarse stride K   : {}", args.factor_k);
    println!("Embed MIH-32      : {}", !args.no_mih32);

    let stats = pack_ahash_directories(&DbahashPackOptions {
        ahash_dirs: ahash_dirs.clone(),
        output_path: args.output.clone(),
        factor_k: args.factor_k,
        embed_coarse_mih32: !args.no_mih32,
    })?;

    if stats.packed_tracks == 0 {
        let _ = std::fs::remove_file(&args.output);
        bail!("No valid .ahash tracks found in {:?}", ahash_dirs);
    }

    println!("\n=== .DBAHASH v1 PACKAGING SUCCESSFUL ===");
    println!("Scanned .ahash files   : {:>12}", stats.scanned_files);
    println!("Unique packed tracks   : {:>12}", stats.packed_tracks);
    println!("Deduplicated twins     : {:>12}", stats.deduplicated_twins);
    println!("Skipped invalid/empty  : {:>12}", stats.skipped_files);
    println!("Total dense frames     : {:>12}", stats.total_dense_frames);
    println!("Total coarse frames    : {:>12}", stats.total_coarse_frames);
    println!(
        "Archive size on disk   : {:>9.2} MiB",
        stats.output_bytes as f64 / BYTES_PER_MIB
    );
    println!("Compilation wall time  : {:>9.2} s", stats.duration_seconds);

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::ahash::{AhashFile, AhashHeader, HaitsmaKalkerExtractor};
    use dbsrt_index::MmapDbahash;
    use tempfile::tempdir;

    #[test]
    fn test_run_pack_ahash_cli_roundtrip_and_empty_dir_error() {
        let tmp = tempdir().expect("tempdir");
        let empty_dir = tmp.path().join("empty");
        std::fs::create_dir_all(&empty_dir).unwrap();
        let err_out = tmp.path().join("none.dbahash");

        let empty_res = run_pack_ahash(&PackAhashArgs {
            ahash_dir: empty_dir,
            extra_dirs: Vec::new(),
            output: err_out.clone(),
            factor_k: DEFAULT_DBAHASH_FACTOR_K,
            no_mih32: false,
        });
        assert!(empty_res.is_err());
        assert!(!err_out.exists());

        let src_dir = tmp.path().join("ahash");
        std::fs::create_dir_all(&src_dir).unwrap();
        let (_, band_bounds) = HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
        let hashes: Vec<u64> = (0..256).map(|i| 0xCAFE_BABE_0000_0000 ^ i).collect();
        let header = AhashHeader {
            version: 1,
            sample_rate: 5512,
            frame_size: 2048,
            hop_size: 64,
            num_bands: 33,
            spectral_lag: 2,
            frame_count: hashes.len() as u64,
            band_bounds,
        };
        AhashFile::write_to_path(&src_dir.join("Demo.Movie.86.1fps.ahash"), &header, &hashes)
            .unwrap();

        let out_path = tmp.path().join("catalog.dbahash");
        let stats = run_pack_ahash(&PackAhashArgs {
            ahash_dir: src_dir,
            extra_dirs: Vec::new(),
            output: out_path.clone(),
            factor_k: 16,
            no_mih32: false,
        })
        .expect("pack succeeds");

        assert_eq!(stats.packed_tracks, 1);
        assert_eq!(stats.total_dense_frames, 256);
        let reader = MmapDbahash::open(&out_path).expect("open compiled .dbahash");
        assert_eq!(reader.media_count(), 1);
        assert_eq!(reader.factor_k(), 16);
    }
}
