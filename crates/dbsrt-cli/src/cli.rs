//! CLI argument definitions and clap parser hierarchy.

use clap::{Parser, Subcommand, ValueEnum};
use dbsrt_core::HwAccel;

use crate::commands::{
    BenchCutsArgs, BenchVideoArgs, CertifyCouplingArgs, ExportPhashArgs, FetchSubtitlesArgs,
    ImportPhashArgs, InspectMediaArgs, MigrateArgs, PackArgs, RecognizeArgs, RetargetArgs,
    ScanLibraryArgs,
};
use crate::constants::DEFAULT_BENCH_PHASH_COUNT;

#[derive(Parser, Debug)]
#[command(
    name = "dbsrt",
    version,
    about = "High-performance video perceptual hash & subtitle retargeting"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
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
pub enum OutputFormat {
    #[value(name = "DataFrame", alias = "dataframe")]
    DataFrame,
    #[value(name = "CSV", alias = "csv")]
    Csv,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Benchmark pHash computation throughput
    BenchPhash {
        #[arg(short, long, default_value_t = DEFAULT_BENCH_PHASH_COUNT)]
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

    /// Certify and import a ground-truth (media/phash <-> subtitle) coupling into couplings.json and/or PostgreSQL
    #[command(visible_alias = "import-subtitles")]
    CertifyCoupling(CertifyCouplingArgs),

    /// Inspect media container tracks and release metadata
    InspectMedia(InspectMediaArgs),

    /// Pack a directory of .phash files into a unified high-density .dbphash index
    Pack(PackArgs),

    /// Fetch subtitles from remote providers (OpenSubtitles REST) with local cache fallback
    FetchSubtitles(FetchSubtitlesArgs),

    /// Apply or check versioned SQL database migrations (refinery) on PostgreSQL
    Migrate(MigrateArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_cli_parse_bench_phash_defaults() {
        let cli = Cli::parse_from(["dbsrt", "bench-phash"]);
        match cli.command {
            Commands::BenchPhash { count } => assert_eq!(count, DEFAULT_BENCH_PHASH_COUNT),
            _ => panic!("Expected BenchPhash command"),
        }
    }

    #[test]
    fn test_cli_parse_certify_alias() {
        let cli = Cli::parse_from([
            "dbsrt",
            "import-subtitles",
            "--media",
            "video.mp4",
            "--subtitle",
            "sub.srt",
        ]);
        match cli.command {
            Commands::CertifyCoupling(args) => {
                assert_eq!(args.media, Some(PathBuf::from("video.mp4")));
                assert_eq!(args.subtitle, PathBuf::from("sub.srt"));
            }
            _ => panic!("Expected CertifyCoupling command"),
        }
    }

    #[test]
    fn test_cli_parse_fetch_subtitles() {
        let cli = Cli::parse_from([
            "dbsrt",
            "fetch-subtitles",
            "-q",
            "Silicon Valley S01E01",
            "-l",
            "fr",
            "--dry-run",
        ]);
        match cli.command {
            Commands::FetchSubtitles(args) => {
                assert_eq!(args.query, "Silicon Valley S01E01");
                assert_eq!(args.language, "fr");
                assert!(args.dry_run);
            }
            _ => panic!("Expected FetchSubtitles command"),
        }
    }
}
