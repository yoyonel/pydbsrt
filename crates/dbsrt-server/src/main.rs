//! Binary entry point for dbsrt-server daemon with auto-port discovery.

use anyhow::Result;
use clap::Parser;
use dbsrt_server::{
    bind_listener, create_router, remove_discovery_file, write_discovery_file, DISCOVERY_FILE,
};
use std::path::PathBuf;
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

    /// Default directory storing reference .srt subtitles (env: DBSRT_SUBTITLES_DIR)
    #[arg(long, env = "DBSRT_SUBTITLES_DIR", default_value = "data/SRT")]
    pub subtitles_dir: PathBuf,
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

    let app = create_router(cli.phash_dir, cli.subtitles_dir);

    let server = axum::serve(listener, app).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received, closing dbsrt-server...");
    });

    let res = server.await;
    remove_discovery_file();
    res?;

    Ok(())
}
