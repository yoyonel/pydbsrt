//! Binary entry point for dbsrt-server daemon with auto-port discovery.

use anyhow::{Context, Result};
use clap::Parser;
use dbsrt_server::create_router;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "dbsrt-server", about = "dbsrt REST microservice daemon")]
struct ServerCli {
    /// Host to bind to (env: DBSRT_HOST)
    #[arg(short = 'H', long, env = "DBSRT_HOST", default_value = "127.0.0.1")]
    host: String,

    /// Port to listen on (env: DBSRT_PORT, 0 for random free port)
    #[arg(short, long, env = "DBSRT_PORT", default_value_t = 8765)]
    port: u16,

    /// Default directory storing .phash files (env: DBSRT_PHASH_DIR)
    #[arg(long, env = "DBSRT_PHASH_DIR", default_value = "/tmp")]
    phash_dir: PathBuf,

    /// Default directory storing reference .srt subtitles (env: DBSRT_SUBTITLES_DIR)
    #[arg(long, env = "DBSRT_SUBTITLES_DIR", default_value = "data/SRT")]
    subtitles_dir: PathBuf,
}

const DISCOVERY_FILE: &str = "/tmp/dbsrt-server.json";

fn write_discovery_file(addr: SocketAddr) -> Result<()> {
    let payload = serde_json::json!({
        "host": addr.ip().to_string(),
        "port": addr.port(),
        "url": format!("http://{}", addr),
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION")
    });
    std::fs::write(DISCOVERY_FILE, serde_json::to_string_pretty(&payload)?)
        .with_context(|| format!("Failed to write discovery file {:?}", DISCOVERY_FILE))?;
    Ok(())
}

fn remove_discovery_file() {
    let _ = std::fs::remove_file(DISCOVERY_FILE);
}

async fn bind_listener(
    host: &str,
    start_port: u16,
) -> Result<(tokio::net::TcpListener, SocketAddr)> {
    if start_port == 0 {
        let addr: SocketAddr = format!("{}:0", host).parse()?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        return Ok((listener, local_addr));
    }

    // Try starting from start_port up to start_port + 20
    let mut current_port = start_port;
    loop {
        let addr: SocketAddr = format!("{}:{}", host, current_port).parse()?;
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                let local_addr = listener.local_addr()?;
                if current_port != start_port {
                    warn!(
                        "Port {} was already in use, successfully auto-selected port {}",
                        start_port, current_port
                    );
                }
                return Ok((listener, local_addr));
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::AddrInUse && current_port < start_port + 20 =>
            {
                current_port += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
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
