//! CLI argument definitions and clap parser hierarchy.

use clap::{Args, Parser, Subcommand, ValueEnum};
use dbsrt_core::HwAccel;

use crate::commands::{
    BenchCutsArgs, BenchVideoArgs, CertifyCouplingArgs, ExportPhashArgs, FetchSubtitlesArgs,
    ImportPhashArgs, InspectMediaArgs, MediaAddArgs, MigrateArgs, PackArgs, RecognizeArgs,
    RelabelCouplingArgs, RetargetArgs, ScanLibraryArgs, TopArgs,
};
use crate::constants::{
    DEFAULT_BENCH_PHASH_COUNT, DEFAULT_POSTGRES_DB, DEFAULT_POSTGRES_PASS, DEFAULT_POSTGRES_PORT,
    DEFAULT_POSTGRES_USER,
};

/// Common PostgreSQL database connection CLI arguments.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PostgresArgs {
    /// PostgreSQL host (defaults to PSQL_IP env or 127.0.0.1).
    /// Note: explicitly passing --db-host also enables DB sync for certify-coupling.
    #[arg(long = "db-host")]
    pub host: Option<String>,

    /// PostgreSQL port
    #[arg(long = "db-port", env = "PSQL_PORT", default_value_t = DEFAULT_POSTGRES_PORT)]
    pub port: u16,

    /// PostgreSQL user
    #[arg(long = "db-user", default_value = DEFAULT_POSTGRES_USER)]
    pub user: String,

    /// PostgreSQL password
    #[arg(long = "db-pass", default_value = DEFAULT_POSTGRES_PASS)]
    pub pass: String,

    /// PostgreSQL database name
    #[arg(long = "db-name", default_value = DEFAULT_POSTGRES_DB)]
    pub name: String,
}

impl Default for PostgresArgs {
    fn default() -> Self {
        let port = std::env::var("PSQL_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_POSTGRES_PORT);
        Self {
            host: None,
            port,
            user: DEFAULT_POSTGRES_USER.to_string(),
            pass: DEFAULT_POSTGRES_PASS.to_string(),
            name: DEFAULT_POSTGRES_DB.to_string(),
        }
    }
}

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

    /// Unified media ingestion facade: extract pHash, certify coupling, and hot-ingest into server
    #[command(name = "media:add", visible_alias = "media-add")]
    MediaAdd(MediaAddArgs),

    /// Relabel a certified subtitle language attribution and confidence status in couplings.json
    #[command(name = "coupling:relabel", visible_alias = "coupling-relabel")]
    RelabelCoupling(RelabelCouplingArgs),

    /// Real-time full-frame TUI process monitor for decoding tasks, CIFS throughput, and pHash
    #[command(name = "top")]
    Top(TopArgs),
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

    #[test]
    fn test_postgres_args_defaults() {
        let pg = PostgresArgs::default();
        let expected_port = std::env::var("PSQL_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_POSTGRES_PORT);
        assert_eq!(pg.host, None);
        assert_eq!(pg.port, expected_port);
        assert_eq!(pg.user, DEFAULT_POSTGRES_USER);
        assert_eq!(pg.pass, DEFAULT_POSTGRES_PASS);
        assert_eq!(pg.name, DEFAULT_POSTGRES_DB);
    }

    #[test]
    fn test_postgres_args_default_matches_clap_defaults() {
        let cli = Cli::parse_from(["dbsrt", "import-phash", "--file", "x.phash"]);
        let Commands::ImportPhash(args) = cli.command else {
            panic!("Expected ImportPhash");
        };
        assert_eq!(args.db.host, None);
        assert_eq!(args.db, PostgresArgs::default());
    }

    #[test]
    fn test_cli_parse_coupling_relabel() {
        let cli = Cli::parse_from([
            "dbsrt",
            "coupling:relabel",
            "--media",
            "43d70728",
            "--lang",
            "fr",
        ]);
        match cli.command {
            Commands::RelabelCoupling(args) => {
                assert_eq!(args.media, "43d70728");
                assert_eq!(args.lang, "fr");
                assert_eq!(args.status, "confirmed");
            }
            _ => panic!("Expected RelabelCoupling command"),
        }
    }

    #[test]
    fn test_cli_parse_top_defaults() {
        let cli = Cli::parse_from(["dbsrt", "top"]);
        match cli.command {
            Commands::Top(args) => {
                assert_eq!(
                    args.refresh_ms,
                    crate::commands::top::DEFAULT_REFRESH_INTERVAL_MS
                );
            }
            _ => panic!("Expected Top command"),
        }
    }

    #[test]
    fn test_cli_parse_top_custom_refresh() {
        let cli = Cli::parse_from(["dbsrt", "top", "--refresh-ms", "500"]);
        match cli.command {
            Commands::Top(args) => {
                assert_eq!(args.refresh_ms, 500);
            }
            _ => panic!("Expected Top command"),
        }
    }
}
