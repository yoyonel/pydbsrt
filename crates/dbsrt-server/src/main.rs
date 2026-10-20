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

    /// Optional remote dbsrt-server URL for Cloud/o2switch relay (env: DBSRT_REMOTE_API)
    #[arg(long, env = "DBSRT_REMOTE_API")]
    pub remote_api: Option<String>,

    /// Run as lightweight client agent (env: DBSRT_AGENT)
    #[arg(long, env = "DBSRT_AGENT", default_value_t = false)]
    pub agent: bool,
}

impl ServerCli {
    pub fn into_app_state(self) -> AppState {
        let db_config = self.db_host.map(|host| {
            DbConnectionConfig::new(host, self.db_port, self.db_user, self.db_pass, self.db_name)
        });

        let mut state = AppState::new(
            self.phash_dir,
            self.subtitles_dir,
            Arc::new(OpenSubtitlesProvider::new(None)),
            SubtitleCache::default(),
        );
        for dir in self.extra_phash_dirs {
            if !state.extra_phash_dirs.contains(&dir) {
                state.extra_phash_dirs.push(dir);
            }
        }
        state.set_dbphash(self.dbphash);
        state.db_config = db_config;
        state.remote_api = self.remote_api;
        state.is_agent = self.agent;
        state
    }
}

pub async fn run_server_until<F>(cli: ServerCli, shutdown: F) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let (listener, addr) = bind_listener(&cli.host, cli.port).await?;
    info!("dbsrt-server daemon listening on http://{}", addr);

    write_discovery_file(addr)?;
    info!(
        "Published service discovery descriptor to {}",
        DISCOVERY_FILE
    );

    let state = cli.into_app_state();
    let app = create_router_with_state(state);

    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown);
    let res = server.await;
    remove_discovery_file();
    res?;
    Ok(())
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
    run_server_until(cli, async {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received, closing dbsrt-server...");
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_cli_parsing_state_and_lifecycle() {
        let cli = ServerCli::try_parse_from([
            "dbsrt-server",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--phash-dir",
            "/tmp/phash_primary",
            "--extra-phash-dir",
            "/tmp/phash_extra",
            "--subtitles-dir",
            "data/SRT",
            "--dbphash",
            "/tmp/catalog.dbphash",
            "--db-host",
            "127.0.0.1",
            "--db-port",
            "5433",
            "--db-user",
            "docker",
            "--db-pass",
            "docker",
            "--db-name",
            "bktreedb",
        ])
        .expect("CLI must parse cleanly");

        assert_eq!(cli.host, "127.0.0.1");
        assert_eq!(cli.port, 0);
        assert_eq!(cli.extra_phash_dirs.len(), 1);
        assert_eq!(cli.db_port, 5433);

        // Preserve any existing discovery file from a running local daemon
        let existing_disc = std::fs::read_to_string(DISCOVERY_FILE).ok();

        let res = run_server_until(cli, async {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        })
        .await;

        if let Some(prev) = existing_disc {
            let _ = std::fs::write(DISCOVERY_FILE, prev);
        }

        assert!(res.is_ok());
    }
}
