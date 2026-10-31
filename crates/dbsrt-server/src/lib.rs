//! dbsrt-server: REST API microservice daemon for visual subtitle synchronization.

pub mod auth;
pub mod constants;
pub mod dtos;
pub mod handlers;
pub mod services;
pub mod state;

pub use constants::*;
pub use dtos::*;
pub use handlers::*;
pub use services::*;
pub use state::*;

use anyhow::{Context, Result};
use axum::routing::{get, post};
use axum::Router;
use dbsrt_core::{OpenSubtitlesProvider, SubtitleCache};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub fn write_discovery_file(addr: SocketAddr) -> Result<()> {
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

pub fn remove_discovery_file() {
    let _ = std::fs::remove_file(DISCOVERY_FILE);
}

pub async fn bind_listener(
    host: &str,
    start_port: u16,
) -> Result<(tokio::net::TcpListener, SocketAddr)> {
    if start_port == 0 {
        let addr: SocketAddr = format!("{}:0", host).parse()?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        return Ok((listener, local_addr));
    }

    let mut current_port = start_port;
    loop {
        let addr: SocketAddr = format!("{}:{}", host, current_port).parse()?;
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                let local_addr = listener.local_addr()?;
                if current_port != start_port {
                    tracing::warn!(
                        "Port {} was already in use, successfully auto-selected port {}",
                        start_port,
                        current_port
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

/// Binds a Unix socket listener (mode 0600), replacing a stale socket left by a killed daemon.
///
/// Refuses to steal a socket a live daemon still answers on, and never removes a path that is
/// not a socket. Check, removal and bind run under an exclusive `flock` on `<path>.lock`, so
/// concurrent starters (PHP gateway, cron watchdog, `o2switch_ctl.sh start`) cannot delete each
/// other's fresh socket.
#[cfg(unix)]
pub fn bind_unix_listener(path: &std::path::Path) -> Result<tokio::net::UnixListener> {
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};

    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);
    // The lock file is never removed: unlinking it would let two starters lock different inodes.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&lock_path)
        .with_context(|| format!("Failed to open spawn lock {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("Failed to lock {}", lock_path.display()))?;

    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            anyhow::bail!("Refusing to replace non-socket path {}", path.display());
        }
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            anyhow::bail!("A dbsrt-server already listens on {}", path.display());
        }
        std::fs::remove_file(path)
            .with_context(|| format!("Failed to remove stale socket {}", path.display()))?;
    }
    let listener = tokio::net::UnixListener::bind(path)
        .with_context(|| format!("Failed to bind Unix socket {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub fn create_router(phash_dir: PathBuf, subtitles_dir: PathBuf) -> Router {
    create_router_with_state(AppState::new(
        phash_dir,
        subtitles_dir,
        Arc::new(OpenSubtitlesProvider::new(None)),
        SubtitleCache::default(),
    ))
}

/// Builds the Axum router for dbsrt-server with custom AppState.
pub fn create_router_with_state(state: AppState) -> Router {
    let state = Arc::new(state);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/studio", get(studio_ui_handler))
        .route("/ui", get(studio_ui_handler))
        .route("/monitoring", get(monitoring_ui_handler))
        .route("/top", get(monitoring_ui_handler))
        .route("/health", get(health_handler))
        .route("/v1/health", get(health_handler))
        .route("/v1/status", get(status_handler))
        .route("/v1/metrics", get(metrics_handler))
        .route("/v1/logs", get(logs_handler))
        .route(
            "/v1/monitoring/telemetry",
            get(monitoring_telemetry_handler),
        )
        .route("/v1/monitoring/top", get(monitoring_top_handler))
        .route(
            "/v1/supervisor/heartbeat",
            post(supervisor_heartbeat_handler),
        )
        .route("/v1/couplings", get(couplings_handler))
        .route("/v1/couplings/sub_frames", get(sub_frames_handler))
        .route("/v1/search", post(search_handler))
        .route("/v1/download", post(download_handler))
        .route("/v1/subtitles/{hash}", get(get_subtitle_by_hash_handler))
        .route("/v1/certify", post(certify_handler))
        .route("/v1/recognize", post(recognize_handler))
        .route("/v1/retarget", post(retarget_handler))
        .route("/v1/autosync", post(autosync_handler))
        .route("/v1/scan", post(scan_handler))
        .route("/v1/catalog/status", get(catalog_status_handler))
        .route("/v1/catalog/reload", post(catalog_reload_handler))
        .route("/v1/reload", post(catalog_reload_handler))
        .route("/v1/catalog/ingest", post(catalog_ingest_handler))
        .route("/v1/catalog/compact", post(catalog_compact_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_core::CutItem;

    #[test]
    fn test_calculate_shift_consensus_empty() {
        let (shift, conf) = calculate_shift_consensus(&[], &[], 0, 42.0);
        assert_eq!(shift, 0);
        assert_eq!(conf, 42.0);
    }

    #[test]
    fn test_calculate_shift_consensus_perfect() {
        let cuts = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 25,
            },
        ];
        let (shift, conf) = calculate_shift_consensus(&cuts, &cuts, 0, 0.0);
        assert_eq!(shift, 0);
        assert!((conf - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_calculate_shift_consensus_with_shift() {
        let target = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 20,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 35,
            },
        ];
        let reference = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 25,
            },
        ];
        let (shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        assert_eq!(shift, 10);
        assert!((conf - 100.0).abs() < 1e-4);
    }

    #[test]
    fn test_calculate_shift_consensus_tie() {
        let target = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 20,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 50,
            },
        ];
        let reference = vec![
            CutItem {
                phash: 0x1111_2222_3333_4444,
                offset: 10,
            },
            CutItem {
                phash: 0x5555_6666_7777_8888,
                offset: 10,
            },
        ];
        let (_shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn test_calculate_shift_consensus_dominance() {
        let mut target = Vec::new();
        let mut reference = Vec::new();
        for i in 0..10 {
            let h = 0x1000_0000_0000_0000 + i;
            target.push(CutItem {
                phash: h,
                offset: 100 + i as usize,
            });
            reference.push(CutItem {
                phash: h,
                offset: 100 + i as usize,
            });
        }
        let h_noise = 0x9999_9999_9999_9999;
        target.push(CutItem {
            phash: h_noise,
            offset: 200,
        });
        reference.push(CutItem {
            phash: h_noise,
            offset: 250,
        });

        let (shift, conf) = calculate_shift_consensus(&target, &reference, 0, 0.0);
        assert_eq!(shift, 0);
        assert!(conf > 80.0 && conf < 85.0);
    }

    #[tokio::test]
    async fn test_bind_listener_ephemeral() {
        let (listener, addr) = bind_listener("127.0.0.1", 0).await.unwrap();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert!(addr.port() > 0);
        drop(listener);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_bind_unix_listener_cases() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("dbsrt_uds_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Fresh path: bound with owner-only permissions
        let fresh = dir.join("fresh.sock");
        let live = bind_unix_listener(&fresh).expect("bind fresh socket");
        let mode = std::fs::metadata(&fresh).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // Live socket: refused, file kept
        assert!(bind_unix_listener(&fresh).is_err());
        assert!(fresh.exists());

        // Stale socket (listener gone, file left behind): replaced
        drop(live);
        assert!(fresh.exists());
        let rebound = bind_unix_listener(&fresh).expect("rebind stale socket");
        drop(rebound);

        // Regular file at the path: refused and left untouched
        let regular = dir.join("not_a_socket");
        std::fs::write(&regular, b"keep").unwrap();
        assert!(bind_unix_listener(&regular).is_err());
        assert_eq!(std::fs::read(&regular).unwrap(), b"keep");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two starters (PHP gateway, cron watchdog) racing on a stale socket: the second one must
    /// wait for the first bind, otherwise it deletes the fresh socket and orphans the first daemon.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bind_unix_listener_waits_for_spawn_lock() {
        const HOLD_MS: u64 = 300;
        let dir = std::env::temp_dir().join(format!("dbsrt_uds_lock_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("d.sock");
        drop(std::os::unix::net::UnixListener::bind(&sock).unwrap()); // stale socket file

        let lock = std::fs::File::create(dir.join("d.sock.lock")).unwrap();
        lock.lock().unwrap(); // another starter is mid check/remove/bind

        let path = sock.clone();
        let starter = tokio::task::spawn_blocking(move || bind_unix_listener(&path));
        tokio::time::sleep(std::time::Duration::from_millis(HOLD_MS)).await;
        assert!(
            !starter.is_finished(),
            "bind must wait while the spawn lock is held"
        );

        lock.unlock().unwrap();
        let listener = starter.await.unwrap().expect("bind after lock release");
        drop(listener);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_discovery_file_lifecycle() {
        let addr = "127.0.0.1:9999".parse().unwrap();
        write_discovery_file(addr).expect("write discovery file");
        assert!(std::path::Path::new(DISCOVERY_FILE).exists());
        let content = std::fs::read_to_string(DISCOVERY_FILE).unwrap();
        assert!(content.contains("9999"));
        assert!(content.contains("version"));
        remove_discovery_file();
        assert!(!std::path::Path::new(DISCOVERY_FILE).exists());
    }

    #[test]
    fn test_find_subtitle_in_catalog_fallback() {
        let empty_dir = std::env::temp_dir().join("empty_srt_dir_test_nonexistent");
        assert!(find_subtitle_in_catalog(&empty_dir, "media", "en").is_none());
    }
}
