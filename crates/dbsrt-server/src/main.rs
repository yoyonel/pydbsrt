//! Binary entry point for dbsrt-server daemon with auto-port discovery.

use anyhow::Result;
use clap::Parser;
use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};
use dbsrt_server::constants::{
    DEFAULT_FILE_WATCH_DEBOUNCE_MS, DEFAULT_FILE_WATCH_INTERVAL_MS, DEFAULT_PG_ERROR_RETRY_SECS,
    DEFAULT_PG_RECONNECT_INTERVAL_SECS,
};
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

    /// Optional API key for remote dbsrt-server relay authentication (env: DBSRT_REMOTE_API_KEY)
    #[arg(long, env = "DBSRT_REMOTE_API_KEY")]
    pub remote_api_key: Option<String>,

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
        state.remote_api_key = self.remote_api_key;
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
    let state_arc = Arc::new(state);

    #[cfg(unix)]
    {
        let state_sighup = state_arc.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut sighup_stream) = signal(SignalKind::hangup()) {
                while sighup_stream.recv().await.is_some() {
                    info!("Received SIGHUP, triggering atomic catalog reload...");
                    let st = state_sighup.clone();
                    let _ = tokio::task::spawn_blocking(move || st.reload_catalog()).await;
                }
            }
        });
    }

    // Pillar 3: File Watcher with Debounce & Stable Size for catalog.dbphash
    let state_file_watch = state_arc.clone();
    tokio::spawn(async move {
        let watch_path = state_file_watch
            .dbphash_path()
            .unwrap_or_else(|| PathBuf::from("data/catalog.dbphash"));
        let mut last_mtime = None;
        let mut last_size = None;
        let mut check_interval = tokio::time::interval(std::time::Duration::from_millis(
            DEFAULT_FILE_WATCH_INTERVAL_MS,
        ));

        loop {
            check_interval.tick().await;
            if watch_path.exists() {
                if let Ok(meta) = std::fs::metadata(&watch_path) {
                    let size = meta.len();
                    if size >= std::mem::size_of::<dbsrt_index::DbphashHeader>() as u64 {
                        if let Ok(mtime) = meta.modified() {
                            if let (Some(prev_mtime), Some(_)) = (last_mtime, last_size) {
                                if mtime > prev_mtime {
                                    // Debounce to ensure rsync or external packer has finished writing
                                    tokio::time::sleep(std::time::Duration::from_millis(
                                        DEFAULT_FILE_WATCH_DEBOUNCE_MS,
                                    ))
                                    .await;

                                    // Verify file size is stable before triggering reload
                                    if let Ok(meta2) = std::fs::metadata(&watch_path) {
                                        if meta2.len() == size {
                                            info!(
                                                "Detected stable modified catalog file on disk ({}, {} bytes), triggering hot-reload...",
                                                watch_path.display(),
                                                size
                                            );
                                            let st = state_file_watch.clone();
                                            let _ = tokio::task::spawn_blocking(move || {
                                                st.reload_catalog()
                                            })
                                            .await;
                                        }
                                    }
                                }
                            }
                            last_mtime = Some(mtime);
                            last_size = Some(size);
                        }
                    }
                }
            }
        }
    });

    // Pillar 3: PostgreSQL LISTEN / NOTIFY event listener (dbsrt_events)
    if let Some(ref db_config) = state_arc.db_config {
        let db_cfg_clone = db_config.clone();
        let state_pg = state_arc.clone();
        std::thread::Builder::new()
            .name("pg-event-listener".to_string())
            .spawn(move || {
                loop {
                    match db_cfg_clone.connect() {
                        Ok(mut client) => {
                            if let Err(e) = client.listen("dbsrt_events") {
                                tracing::warn!("Failed executing LISTEN dbsrt_events: {}", e);
                                std::thread::sleep(std::time::Duration::from_secs(
                                    DEFAULT_PG_ERROR_RETRY_SECS,
                                ));
                                continue;
                            }
                            info!("Subscribed to PostgreSQL event bus: LISTEN dbsrt_events");

                            loop {
                                match client.next_notification() {
                                    Ok(Some(notification)) => {
                                        info!(
                                            "Received PostgreSQL notification: channel='{}' payload='{}'",
                                            notification.channel(),
                                            notification.payload()
                                        );
                                        match notification.payload() {
                                            "catalog_reload" | "reload" => {
                                                let st = state_pg.clone();
                                                std::thread::spawn(move || {
                                                    let _ = st.reload_catalog();
                                                });
                                            }
                                            "catalog_compact" | "compact" => {
                                                let st = state_pg.clone();
                                                std::thread::spawn(move || {
                                                    let _ = st.compact_catalog();
                                                });
                                            }
                                            _ => {}
                                        }
                                    }
                                    Ok(None) => break,
                                    Err(e) => {
                                        tracing::warn!(
                                            "PostgreSQL notification stream error: {}",
                                            e
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!(
                                "PostgreSQL event listener cannot connect (retry in {}s): {}",
                                DEFAULT_PG_RECONNECT_INTERVAL_SECS,
                                e
                            );
                            std::thread::sleep(std::time::Duration::from_secs(
                                DEFAULT_PG_RECONNECT_INTERVAL_SECS,
                            ));
                        }
                    }
                }
            })
            .ok();
    }

    let app = create_router_with_state((*state_arc).clone());

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
