//! Application state, database connection pooling, and telemetry metrics.

use crate::constants::{
    LOG_BUFFER_CAPACITY, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE,
};
use crate::dtos::{CatalogCompactResponse, CatalogReloadResponse, CatalogStatusResponse};
use anyhow::Result;
use arc_swap::ArcSwapOption;
use dbsrt_core::{CouplingRegistry, SubtitleCache, SubtitleProvider};
use dbsrt_index::{DbClient, DeltaIndex, MmapDbphash, PhashEngine};
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
    pub remote_api_key: Option<String>,
    /// Key pool file guarding every non-health route; `None` keeps the API open.
    pub api_key_file: Option<PathBuf>,
    pub is_agent: bool,
    pub log_buffer: Arc<std::sync::RwLock<VecDeque<ServerLogEntry>>>,
    pub reload_in_progress: Arc<std::sync::atomic::AtomicBool>,
    pub compact_in_progress: Arc<std::sync::atomic::AtomicBool>,
    pub delta_index: Arc<std::sync::RwLock<DeltaIndex>>,
}

/// Optional configuration parameters for initializing [`AppState`].
#[derive(Debug, Clone, Default)]
pub struct AppStateInitOptions {
    /// Explicit target `.dbphash` catalog file path.
    pub dbphash_path: Option<PathBuf>,
    /// Additional search directories for loose `.phash` files.
    pub extra_phash_dirs: Vec<PathBuf>,
}

impl AppState {
    pub fn new(
        phash_dir: PathBuf,
        subtitles_dir: PathBuf,
        provider: Arc<dyn SubtitleProvider>,
        cache: SubtitleCache,
    ) -> Self {
        let mut st = Self::with_options(
            phash_dir,
            subtitles_dir,
            provider,
            AppStateInitOptions::default(),
        );
        st.cache = cache;
        st
    }

    /// Initializes `AppState` with custom catalog path and extra directory options.
    ///
    /// # Arguments
    /// - `phash_dir`: Primary directory for loose `.phash` files.
    /// - `subtitles_dir`: Root directory for reference `.srt` files and `couplings.json`.
    /// - `provider`: Remote subtitle download provider facade.
    /// - `options`: Extended initialization options including custom dbphash path and extra dirs.
    ///
    /// # Returns
    /// An initialized [`AppState`] ready to serve HTTP requests.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Prevents unbounded memory bloat: when a compiled `.dbphash` catalog is present,
    /// loose fallback directories (`data/__PHASH__`) are NOT scanned into RAM.
    pub fn with_options(
        phash_dir: PathBuf,
        subtitles_dir: PathBuf,
        provider: Arc<dyn SubtitleProvider>,
        options: AppStateInitOptions,
    ) -> Self {
        let default_catalog = PathBuf::from("data/catalog.dbphash");
        let initial_catalog_path = match options.dbphash_path {
            Some(p) => Some(p),
            None if default_catalog.exists() => Some(default_catalog),
            None => None,
        };

        let mut extra_phash_dirs = options.extra_phash_dirs;
        let default_repo_phash = PathBuf::from("data/__PHASH__");
        if initial_catalog_path.is_none()
            && default_repo_phash != phash_dir
            && !extra_phash_dirs.contains(&default_repo_phash)
            && default_repo_phash.exists()
        {
            extra_phash_dirs.push(default_repo_phash);
        }

        let reg_path = CouplingRegistry::default_path(&subtitles_dir);
        let coupling_registry = Arc::new(std::sync::RwLock::new(
            CouplingRegistry::load_or_default(&reg_path).unwrap_or_default(),
        ));

        let mut all_dirs = Vec::new();
        if phash_dir.exists() && phash_dir != Path::new("/tmp") {
            all_dirs.push(phash_dir.clone());
        }
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
            cache: SubtitleCache::default(),
            start_time: Instant::now(),
            metrics: Arc::new(ServerMetrics::default()),
            remote_api: None,
            remote_api_key: None,
            api_key_file: None,
            is_agent: false,
            log_buffer: Arc::new(std::sync::RwLock::new(VecDeque::with_capacity(
                LOG_BUFFER_CAPACITY,
            ))),
            reload_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            compact_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            delta_index: Arc::new(std::sync::RwLock::new(DeltaIndex::new())),
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
        let new_couplings = CouplingRegistry::load_or_default(&reg_path)?;

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
                // Pre-swap validation: check file size, magic, header integrity, and prewarm MIH
                let meta = std::fs::metadata(p)?;
                if meta.len() < std::mem::size_of::<dbsrt_index::DbphashHeader>() as u64 {
                    anyhow::bail!(
                        "Corrupted .dbphash catalog: file size {} bytes is smaller than header",
                        meta.len()
                    );
                }
                let reader = MmapDbphash::open(p)?;
                media_count = reader.media_entries().len();
                cuts_count = reader.cuts_entries().len();
                sparse_count = reader.sparse_entries().len();
                Some(Arc::new(reader))
            }
            _ => None,
        };

        let dirs = self.effective_phash_dirs(None);
        let mut loose_file_count = 0;
        let new_lib_opt = if !dirs.is_empty() {
            match dbsrt_index::MmapLibrary::from_directories(&dirs) {
                Ok(lib) => {
                    lib.prewarm_mih();
                    loose_file_count = lib.file_count();
                    Some(Arc::new(lib))
                }
                Err(_) => None,
            }
        } else {
            None
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

    /// Explicitly configures the target `.dbphash` catalog file path.
    pub fn set_dbphash_path(&self, path: Option<PathBuf>) {
        if let Ok(mut p_lock) = self.dbphash_path.write() {
            *p_lock = path;
        }
    }

    /// Sets or updates the `.dbphash` catalog and pre-warms the MIH indices in RAM.
    ///
    /// If `path` is `Some(p)`:
    /// - If `MmapDbphash::open(p)` succeeds, `dbphash_path` is updated and readers are swapped.
    /// - If `p` does not exist yet (pre-configuration before initial generation), `dbphash_path` is stored for subsequent reload.
    /// - If `p` exists but is invalid/corrupt, `dbphash_path` is NOT updated, readers are cleared, and an error is logged.
    ///
    /// If `path` is `None`:
    /// - `dbphash_path` is cleared to `None` and readers are cleared.
    pub fn set_dbphash(&self, path: Option<PathBuf>) {
        if let Some(ref p) = path {
            if let Ok(reader) = MmapDbphash::open(p) {
                let reader_arc = Arc::new(reader);
                if let Ok(mut p_lock) = self.dbphash_path.write() {
                    *p_lock = Some(p.clone());
                }
                self.dbphash_reader.store(Some(reader_arc.clone()));
                if let Ok(mut eng) = self.engine.write() {
                    eng.set_dbphash(Some(reader_arc));
                }
            } else {
                if !p.exists() {
                    // Pre-configured path before file creation
                    if let Ok(mut p_lock) = self.dbphash_path.write() {
                        *p_lock = Some(p.clone());
                    }
                } else {
                    self.push_log(
                        "CATALOG",
                        "ERROR",
                        format!(
                            "Failed to open existing .dbphash at {:?}; rejecting path update",
                            p
                        ),
                    );
                }
                self.dbphash_reader.store(None);
                if let Ok(mut eng) = self.engine.write() {
                    eng.set_dbphash(None);
                }
            }
        } else {
            if let Ok(mut p_lock) = self.dbphash_path.write() {
                *p_lock = None;
            }
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
        if primary.exists() && primary != Path::new("/tmp") {
            dirs.push(primary);
        }
        if override_dir.is_none() {
            for d in &self.extra_phash_dirs {
                if d.exists() && !dirs.contains(d) {
                    dirs.push(d.clone());
                }
            }
        }
        dirs
    }

    /// Total number of distinct indexed medias across Base, Delta, and loose files.
    pub fn indexed_media_count(&self) -> usize {
        let base_count = if let Some(ref r) = *self.dbphash_reader.load() {
            r.media_entries().len()
        } else {
            0
        };
        let delta_count = self
            .delta_index
            .read()
            .map(|d| d.media_count())
            .unwrap_or(0);
        let loose_count = self
            .engine
            .read()
            .map(|e| e.library().map(|l| l.file_count()).unwrap_or(0))
            .unwrap_or(0);
        base_count + delta_count + loose_count
    }

    /// Returns a comprehensive snapshot of the catalog state (Base + Delta + Couplings).
    pub fn catalog_status(&self) -> CatalogStatusResponse {
        let (base_medias, base_cuts, base_sparse) =
            if let Some(ref reader) = *self.dbphash_reader.load() {
                (
                    reader.media_entries().len(),
                    reader.cuts_entries().len(),
                    reader.sparse_entries().len(),
                )
            } else {
                (0, 0, 0)
            };

        let (delta_medias, delta_cuts, delta_sparse) = {
            if let Ok(delta) = self.delta_index.read() {
                (
                    delta.media_count(),
                    delta.cuts_count(),
                    delta.sparse_count(),
                )
            } else {
                (0, 0, 0)
            }
        };

        let certified_couplings = if let Ok(reg) = self.coupling_registry.read() {
            reg.couplings.iter().map(|c| c.subtitles.len()).sum()
        } else {
            0
        };

        CatalogStatusResponse {
            base_medias,
            base_cuts,
            base_sparse,
            delta_medias,
            delta_cuts,
            delta_sparse,
            certified_couplings,
            total_medias: base_medias + delta_medias,
        }
    }

    /// Ingests a new media stream into the RAM Delta Index without rewriting `catalog.dbphash`.
    pub fn ingest_delta_frames(&self, filename: &str, hashes: &[u64], fps: f64) -> Result<u32> {
        let mut delta = self
            .delta_index
            .write()
            .map_err(|e| anyhow::anyhow!("DeltaIndex RwLock error: {e}"))?;
        let media_id = delta.ingest_raw_frames(filename, hashes, fps);
        self.push_log(
            "DELTA",
            "INFO",
            format!(
                "Ingested '{}' into DeltaIndex (id={}, {} frames, {} total delta medias)",
                filename,
                media_id,
                hashes.len(),
                delta.media_count()
            ),
        );
        Ok(media_id)
    }

    /// Merges Base (.dbphash) and Delta (RAM) into a new catalog file and triggers hot-reload.
    pub fn compact_catalog(&self) -> Result<CatalogCompactResponse> {
        // Enforce anti-stampede for compaction
        if self
            .compact_in_progress
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            anyhow::bail!("Catalog compaction is already in progress");
        }

        struct CompactGuard(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for CompactGuard {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _guard = CompactGuard(self.compact_in_progress.clone());

        let t0 = Instant::now();

        // 1. Atomically swap out current delta with an empty DeltaIndex.
        // Incoming concurrent ingestions will write to the new empty DeltaIndex.
        // Zero media can ever be lost!
        let old_delta = {
            let mut delta_lock = self
                .delta_index
                .write()
                .map_err(|e| anyhow::anyhow!("DeltaIndex write lock failed: {e}"))?;
            if delta_lock.is_empty() {
                let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
                return Ok(CatalogCompactResponse {
                    success: true,
                    compacted: false,
                    elapsed_ms,
                    total_medias: self.indexed_media_count(),
                    delta_medias_flushed: 0,
                    message: "Delta index is empty; no compaction needed".to_string(),
                });
            }
            std::mem::take(&mut *delta_lock)
        };

        // 2. Build combined catalog
        let mut builder = dbsrt_index::DbphashBuilder::new();

        if let Some(ref reader) = *self.dbphash_reader.load() {
            reader.export_to_builder(&mut builder);
        }
        old_delta.export_to_builder(&mut builder);

        let target_path = self
            .dbphash_path()
            .unwrap_or_else(|| PathBuf::from("data/catalog.dbphash"));
        let tmp_path = target_path.with_extension("tmp");

        if let Some(parent) = target_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let restore_delta = |old: DeltaIndex| match self.delta_index.write() {
            Ok(mut delta_lock) => {
                let mut current = std::mem::take(&mut *delta_lock);
                let mut restored = old;
                restored.add_delta_entries_batch(current.drain_medias());
                *delta_lock = restored;
            }
            Err(lock_err) => {
                self.push_log(
                    "COMPACT",
                    "ERROR",
                    format!(
                        "CRITICAL: Delta rollback failed, lock poisoned: {}. {} medias may be lost.",
                        lock_err,
                        old.media_count()
                    ),
                );
            }
        };

        let pack_stats = match builder.write_to_file(&tmp_path) {
            Ok(s) => s,
            Err(e) => {
                restore_delta(old_delta);
                return Err(e);
            }
        };

        if let Err(e) = std::fs::rename(&tmp_path, &target_path) {
            restore_delta(old_delta);
            return Err(e.into());
        }

        // If dbphash_path was None, update it to target_path (Bug #7)
        if self.dbphash_path().is_none() {
            if let Ok(mut p_lock) = self.dbphash_path.write() {
                *p_lock = Some(target_path.clone());
            }
        }

        // 3. Hot-reload new catalog
        self.reload_catalog()?;

        let flushed_count = old_delta.media_count();
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.push_log(
            "COMPACT",
            "INFO",
            format!(
                "Compacted {} delta medias into {} total medias in {:.2}ms",
                flushed_count, pack_stats.media_count, elapsed_ms
            ),
        );

        Ok(CatalogCompactResponse {
            success: true,
            compacted: true,
            elapsed_ms,
            total_medias: pack_stats.media_count,
            delta_medias_flushed: flushed_count,
            message: format!("Successfully compacted into {}", target_path.display()),
        })
    }
}
