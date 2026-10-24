//! Application state, database connection pooling, and telemetry metrics.

use crate::constants::{
    LOG_BUFFER_CAPACITY, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE,
};
use crate::dtos::CatalogReloadResponse;
use anyhow::Result;
use arc_swap::ArcSwapOption;
use dbsrt_core::{CouplingRegistry, SubtitleCache, SubtitleProvider};
use dbsrt_index::{DbClient, MmapDbphash, PhashEngine};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Persistent container for a reusable `DbClient`.
/// Implements `Drop` on an isolated OS thread so `postgres::Client::drop` never calls
/// Tokio's `block_on` inside an active async runtime worker thread (which would panic).
pub struct ClientPool {
    client: std::sync::Mutex<Option<DbClient>>,
}

impl Drop for ClientPool {
    fn drop(&mut self) {
        if let Ok(mut lock) = self.client.lock() {
            if let Some(client) = lock.take() {
                let _ = std::thread::spawn(move || {
                    drop(client);
                })
                .join();
            }
        }
    }
}

#[derive(Clone)]
pub struct DbConnectionConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub dbname: String,
    pub pool: Arc<ClientPool>,
}

impl std::fmt::Debug for DbConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("dbname", &self.dbname)
            .finish()
    }
}

impl DbConnectionConfig {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        pass: impl Into<String>,
        dbname: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            pass: pass.into(),
            dbname: dbname.into(),
            pool: Arc::new(ClientPool {
                client: std::sync::Mutex::new(None),
            }),
        }
    }

    pub fn connect(&self) -> Result<DbClient> {
        DbClient::connect(&self.host, self.port, &self.user, &self.pass, &self.dbname)
    }

    /// Executes a synchronous `DbClient` closure inside an isolated OS thread scope so
    /// `postgres::Client` never panics when called from inside an active Tokio runtime.
    /// Reuses persistent connection from `pool` to avoid 10-13ms TCP handshake + SCRAM auth latency.
    pub fn with_client<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut DbClient) -> Result<T> + Send,
        T: Send,
    {
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut client = {
                    let mut lock = self
                        .pool
                        .client
                        .lock()
                        .map_err(|e| anyhow::anyhow!("DB pool lock poisoned: {}", e))?;
                    if let Some(c) = lock.take() {
                        if !c.is_closed() {
                            c
                        } else {
                            self.connect()?
                        }
                    } else {
                        self.connect()?
                    }
                };

                let res = f(&mut client);

                if !client.is_closed() {
                    if let Ok(mut lock) = self.pool.client.lock() {
                        *lock = Some(client);
                    }
                }

                res
            })
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("PostgreSQL worker thread panicked")))
        })
    }
}

#[derive(Debug, Default)]
pub struct ServerMetrics {
    pub total_requests: std::sync::atomic::AtomicU64,
    pub recognize_requests: std::sync::atomic::AtomicU64,
    pub search_requests: std::sync::atomic::AtomicU64,
    pub cache_hits: std::sync::atomic::AtomicU64,
    pub remote_errors: std::sync::atomic::AtomicU64,
    pub total_latency_us: std::sync::atomic::AtomicU64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServerLogEntry {
    pub timestamp: String,
    pub tag: String,
    pub level: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServerLogsResponse {
    pub count: usize,
    pub logs: Vec<ServerLogEntry>,
}

#[derive(Clone)]
pub struct AppState {
    pub phash_dir: PathBuf,
    pub extra_phash_dirs: Vec<PathBuf>,
    pub dbphash_path: Arc<std::sync::RwLock<Option<PathBuf>>>,
    pub dbphash_reader: Arc<ArcSwapOption<MmapDbphash>>,
    pub engine: Arc<std::sync::RwLock<PhashEngine>>,
    pub db_config: Option<DbConnectionConfig>,
    pub subtitles_dir: PathBuf,
    pub coupling_registry: Arc<std::sync::RwLock<CouplingRegistry>>,
    pub provider: Arc<dyn SubtitleProvider>,
    pub cache: SubtitleCache,
    pub start_time: Instant,
    pub metrics: Arc<ServerMetrics>,
    pub remote_api: Option<String>,
    pub is_agent: bool,
    pub log_buffer: Arc<std::sync::RwLock<VecDeque<ServerLogEntry>>>,
    pub reload_in_progress: Arc<std::sync::atomic::AtomicBool>,
}

impl AppState {
    pub fn new(
        phash_dir: PathBuf,
        subtitles_dir: PathBuf,
        provider: Arc<dyn SubtitleProvider>,
        cache: SubtitleCache,
    ) -> Self {
        let mut extra_phash_dirs = Vec::new();
        let default_repo_phash = PathBuf::from("data/__PHASH__");
        if default_repo_phash != phash_dir && default_repo_phash.exists() {
            extra_phash_dirs.push(default_repo_phash);
        }

        let reg_path = CouplingRegistry::default_path(&subtitles_dir);
        let coupling_registry = Arc::new(std::sync::RwLock::new(
            CouplingRegistry::load_or_default(&reg_path).unwrap_or_default(),
        ));

        // Auto-detect default catalog if exists
        let default_catalog = PathBuf::from("data/catalog.dbphash");
        let initial_catalog_path = if default_catalog.exists() {
            Some(default_catalog)
        } else {
            None
        };

        let mut all_dirs = vec![phash_dir.clone()];
        all_dirs.extend_from_slice(&extra_phash_dirs);

        let initial_engine =
            PhashEngine::open(initial_catalog_path.as_deref(), &all_dirs).unwrap_or_default();

        let dbphash_reader = Arc::new(ArcSwapOption::new(initial_engine.dbphash_arc()));

        Self {
            phash_dir,
            extra_phash_dirs,
            dbphash_path: Arc::new(std::sync::RwLock::new(initial_catalog_path)),
            dbphash_reader,
            engine: Arc::new(std::sync::RwLock::new(initial_engine)),
            db_config: None,
            subtitles_dir,
            coupling_registry,
            provider,
            cache,
            start_time: Instant::now(),
            metrics: Arc::new(ServerMetrics::default()),
            remote_api: None,
            is_agent: false,
            log_buffer: Arc::new(std::sync::RwLock::new(VecDeque::with_capacity(
                LOG_BUFFER_CAPACITY,
            ))),
            reload_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Returns the current active `MmapDbphash` handle (wait-free, zero-lock).
    pub fn dbphash_reader(&self) -> Option<Arc<MmapDbphash>> {
        self.dbphash_reader.load_full()
    }

    /// Returns the active `catalog.dbphash` path if configured.
    pub fn dbphash_path(&self) -> Option<PathBuf> {
        self.dbphash_path.read().ok().and_then(|p| p.clone())
    }

    pub fn push_log(&self, tag: &str, level: &str, message: impl Into<String>) {
        if let Ok(mut buf) = self.log_buffer.write() {
            if buf.len() >= LOG_BUFFER_CAPACITY {
                buf.pop_front();
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = now.as_secs() % SECONDS_PER_DAY;
            let h = secs / SECONDS_PER_HOUR;
            let m = (secs % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
            let s = secs % SECONDS_PER_MINUTE;
            let ms = now.subsec_millis();
            let ts = format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms);
            buf.push_back(ServerLogEntry {
                timestamp: ts,
                tag: tag.to_string(),
                level: level.to_string(),
                message: message.into(),
            });
        }
    }

    /// Reloads the in-memory `CouplingRegistry` from disk.
    pub fn reload_coupling_registry(&self) {
        let reg_path = CouplingRegistry::default_path(&self.subtitles_dir);
        if let Ok(reg) = CouplingRegistry::load_or_default(&reg_path) {
            if let Ok(mut lock) = self.coupling_registry.write() {
                *lock = reg;
            }
        }
    }

    /// Atomically reloads the `.dbphash` catalog, loose `.phash` directories, and `couplings.json`.
    /// Follows a strict 2-Phase Commit (Prepare-then-Commit) so running state is never left
    /// inconsistent if any load/validation step fails.
    pub fn reload_catalog(&self) -> Result<CatalogReloadResponse> {
        // Enforce single concurrent reload (anti-stampede rate limiting)
        if self
            .reload_in_progress
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            anyhow::bail!("Catalog reload is already in progress");
        }

        // RAII guard to reset reload_in_progress on scope exit
        struct ReloadGuard(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for ReloadGuard {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _guard = ReloadGuard(self.reload_in_progress.clone());

        let t0 = Instant::now();

        // --- Phase 1: Prepare & Validate all resources (can fail without corrupting runtime) ---
        let reg_path = CouplingRegistry::default_path(&self.subtitles_dir);
        let new_couplings = CouplingRegistry::load_or_default(&reg_path).unwrap_or_default();

        let catalog_path = self.dbphash_path().or_else(|| {
            let default_catalog = PathBuf::from("data/catalog.dbphash");
            if default_catalog.exists() {
                Some(default_catalog)
            } else {
                None
            }
        });

        let mut media_count = 0;
        let mut cuts_count = 0;
        let mut sparse_count = 0;

        let new_reader_opt = match catalog_path {
            Some(ref p) if p.exists() => {
                let reader = MmapDbphash::open(p)?;
                let _ = reader.sparse_mih();
                let _ = reader.cuts_mih();
                media_count = reader.media_entries().len();
                cuts_count = reader.cuts_entries().len();
                sparse_count = reader.sparse_entries().len();
                Some(Arc::new(reader))
            }
            _ => None,
        };

        let dirs = self.effective_phash_dirs(None);
        let mut loose_file_count = 0;
        let new_lib_opt = match dbsrt_index::MmapLibrary::from_directories(&dirs) {
            Ok(lib) => {
                lib.prewarm_mih();
                loose_file_count = lib.file_count();
                Some(Arc::new(lib))
            }
            Err(_) => None,
        };

        // --- Phase 2: Commit Atomically (cannot fail) ---
        let coupling_count = new_couplings
            .couplings
            .iter()
            .map(|c| c.subtitles.len())
            .sum();

        if let Ok(mut lock) = self.coupling_registry.write() {
            *lock = new_couplings;
        }

        self.dbphash_reader.store(new_reader_opt.clone());

        if let Ok(mut eng) = self.engine.write() {
            eng.set_dbphash(new_reader_opt);
            if let Some(new_lib) = new_lib_opt {
                eng.set_library(Some(new_lib));
            }
        }

        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.push_log(
            "CATALOG",
            "INFO",
            format!(
                "Reloaded in {:.2}ms: {} medias ({} cuts, {} sparse), {} loose files, {} certified couplings",
                elapsed_ms, media_count, cuts_count, sparse_count, loose_file_count, coupling_count
            ),
        );

        Ok(CatalogReloadResponse {
            success: true,
            elapsed_ms,
            indexed_medias: media_count + loose_file_count,
            catalog_medias: media_count,
            cuts_entries: cuts_count,
            sparse_entries: sparse_count,
            certified_couplings: coupling_count,
        })
    }

    /// Sets or updates the `.dbphash` catalog and pre-warms the MIH indices in RAM.
    pub fn set_dbphash(&self, path: Option<PathBuf>) {
        if let Ok(mut p_lock) = self.dbphash_path.write() {
            *p_lock = path.clone();
        }

        if let Some(ref p) = path {
            if let Ok(reader) = MmapDbphash::open(p) {
                let _ = reader.sparse_mih();
                let _ = reader.cuts_mih();
                let reader_arc = Arc::new(reader);
                self.dbphash_reader.store(Some(reader_arc.clone()));
                if let Ok(mut eng) = self.engine.write() {
                    eng.set_dbphash(Some(reader_arc));
                }
            } else {
                self.dbphash_reader.store(None);
                if let Ok(mut eng) = self.engine.write() {
                    eng.set_dbphash(None);
                }
            }
        } else {
            self.dbphash_reader.store(None);
            if let Ok(mut eng) = self.engine.write() {
                eng.set_dbphash(None);
            }
        }
    }

    /// Returns all active `.phash` directories with `override_dir` (or `self.phash_dir`) first.
    pub fn effective_phash_dirs(&self, override_dir: Option<&Path>) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let primary = override_dir.unwrap_or(&self.phash_dir).to_path_buf();
        dirs.push(primary);
        if override_dir.is_none() {
            for d in &self.extra_phash_dirs {
                if !dirs.contains(d) {
                    dirs.push(d.clone());
                }
            }
        }
        dirs
    }
}
