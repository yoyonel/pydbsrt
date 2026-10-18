//! Binary entry point for dbsrt-server daemon with auto-port discovery.

use anyhow::Result;
use clap::Parser;
use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};
use dbsrt_server::{
    bind_listener, create_router_with_state, remove_discovery_file, write_discovery_file, AppState,
    DbConnectionConfig, DISCOVERY_FILE,
};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "dbsrt-server",
    version,
    about = "dbsrt REST microservice daemon"
)]
pub struct ServerCli {
    /// Host to bind to (env: DBSRT_HOST)
    #[arg(short = 'H', long, env = "DBSRT_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Port to listen on (env: DBSRT_PORT, 0 for random free port)
    #[arg(short, long, env = "DBSRT_PORT", default_value_t = 8765)]
    pub port: u16,

    /// Default directory storing .phash files (env: DBSRT_PHASH_DIR)
    #[arg(long, env = "DBSRT_PHASH_DIR", default_value = "/tmp")]
    pub phash_dir: PathBuf,

    /// Additional directories storing .phash files
    #[arg(long = "extra-phash-dir")]
    pub extra_phash_dirs: Vec<PathBuf>,

    /// Default directory storing reference .srt subtitles (env: DBSRT_SUBTITLES_DIR)
    #[arg(long, env = "DBSRT_SUBTITLES_DIR", default_value = "data/SRT")]
    pub subtitles_dir: PathBuf,

    /// Optional compiled .dbphash catalog file (env: DBSRT_DBPHASH)
    #[arg(long, env = "DBSRT_DBPHASH")]
    pub dbphash: Option<PathBuf>,

    /// Optional PostgreSQL host (env: PSQL_IP). If omitted, runs in local Zero-DB mode.
    #[arg(long, env = "PSQL_IP")]
    pub db_host: Option<String>,

    /// PostgreSQL port (env: PSQL_PORT)
    #[arg(long, env = "PSQL_PORT", default_value_t = 5433)]
    pub db_port: u16,

    /// PostgreSQL user (env: PSQL_USER)
    #[arg(long, env = "PSQL_USER", default_value = "docker")]
    pub db_user: String,

    /// PostgreSQL password (env: PSQL_PASS)
    #[arg(long, env = "PSQL_PASS", default_value = "docker")]
    pub db_pass: String,

    /// PostgreSQL database name (env: PSQL_DBNAME)
    #[arg(long, env = "PSQL_DBNAME", default_value = "bktreedb")]
    pub db_name: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let cli = ServerCli::parse();
    let (listener, addr) = bind_listener(&cli.host, cli.port).await?;

    info!("dbsrt-server daemon listening on http://{}", addr);

    // Publish service discovery file
    write_discovery_file(addr)?;
    info!(
        "Published service discovery descriptor to {}",
        DISCOVERY_FILE
    );

    let db_config = cli.db_host.map(|host| DbConnectionConfig {
        host,
        port: cli.db_port,
        user: cli.db_user,
        pass: cli.db_pass,
        dbname: cli.db_name,
    });

    let mut state = AppState::new(
        cli.phash_dir,
        cli.subtitles_dir,
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    );
    for dir in cli.extra_phash_dirs {
        if !state.extra_phash_dirs.contains(&dir) {
            state.extra_phash_dirs.push(dir);
        }
    }
    state.dbphash_path = cli.dbphash;
    state.db_config = db_config;

    let app = create_router_with_state(state);

    let server = axum::serve(listener, app).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received, closing dbsrt-server...");
    });

    let res = server.await;
    remove_discovery_file();
    res?;

    Ok(())
}
