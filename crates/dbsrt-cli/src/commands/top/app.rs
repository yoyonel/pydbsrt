//! # Application Lifecycle, State Machine & TUI Event Loop for `dbsrt top`
//!
//! This module coordinates the top monitor execution loop, terminal setup/teardown,
//! cross-thread telemetry synchronization via `Arc<RwLock<SystemTelemetry>>`, non-blocking
//! keyboard event dispatching, and view-level modal management.
//!
//! ## Architectural Topology
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        crossterm                            │
//! │             Raw Mode Terminal Backend (Alternate Screen)     │
//! └──────────────────────────────┬──────────────────────────────┘
//!                                │ draw / poll
//!                                ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                          AppState                           │
//! │   (ViewPage, Selections, Filters, Search Queries, Modals)    │
//! └──────────────────────────────┬──────────────────────────────┘
//!                                │ read telemetry
//!                                ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                Arc<RwLock<SystemTelemetry>>                 │
//! │  ◄── GPU Telemetry Loop (sysfs/intel_gpu_top)               │
//! │  ◄── ProcFS Telemetry Loop (CPU, RAM, CIFS read bytes)      │
//! │  ◄── Multi-Source Scanner Thread (media disk discovery)     │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashSet;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

pub use dbsrt_tui::app::{AppState, TopActions};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, DEFAULT_COUPLINGS_PATH, DEFAULT_MEDIA_CACHE_PATH,
    DEFAULT_MEDIA_SOURCES_PATH, DEFAULT_REFRESH_INTERVAL_MS, DEFAULT_SELECTED_BATCH_TARGETS_PATH,
    DEFAULT_SOURCE_ID_TVSHOW, KEY_EVENT_POLL_TIMEOUT_MS,
};
use crate::commands::top::model::{
    calculate_overall_stats, GpuTelemetry, MediaSourceConfig, MediaSourceFilter,
    SourceInputModalState, SourceModalMode, SystemTelemetry,
};
use crate::commands::top::sampler::{
    add_media_source, compute_session_throughput, delete_media_source, load_historical_records,
    load_media_sources, preload_stem_size_cache, resolve_events_log_path,
    run_telemetry_sampler_loop, save_media_cache, start_gpu_sampler, start_media_sources_scanner,
    toggle_media_source, trigger_media_sources_rescan, update_media_source,
};
use crate::commands::top::ui::render_ui;
use crate::commands::top::{process_ctl, sampler};

/// Sets up raw terminal mode, panic recovery hooks, spawns telemetry background threads,
/// and runs the interactive TUI event loop until user exit (`[q]` or Ctrl-C).
///
/// # Arguments
/// - `refresh_interval_ms`: Sampling and render refresh cycle in milliseconds (typically 500ms to 1000ms).
///
/// # Returns
/// - `Ok(())` on clean exit.
/// - `Err(anyhow::Error)` if raw mode initialization, terminal alternate screen, or event loop fails.
///
/// # Errors
/// Propagates `std::io::Error` during terminal mode configuration or backend rendering.
///
/// # Panics
/// Panic hook is registered to intercept unhandled panics, cleanly restoring raw mode and
/// restoring standard terminal buffer before forwarding to original panic hook.
///
/// # Data Flow & Invariants
/// 1. Switches standard output to alternate screen and enables raw mode via `crossterm`.
/// 2. Pre-warms historical batch JSONL records and computes initial statistical moments.
/// 3. Instantiates `SystemTelemetry` wrapped inside thread-safe `Arc<RwLock<SystemTelemetry>>`.
/// 4. Spawns GPU sampler thread, filesystem media discovery scanner, and ProcFS polling loop.
/// 5. Executes [`event_loop`], waiting for terminal key inputs.
/// 6. Signals worker termination via atomic `stop_flag`, leaves alternate screen, and shows cursor.
pub fn run_top_app(refresh_interval_ms: u64) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Register panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Pre-warm static history, statistics, and system memory to prevent initial frame lag
    let events_path = resolve_events_log_path();
    let couplings_path = std::path::PathBuf::from(DEFAULT_COUPLINGS_PATH);
    let initial_history = load_historical_records(&events_path, &couplings_path);
    let initial_stats = calculate_overall_stats(&initial_history);
    preload_stem_size_cache();

    let mut initial_sys = sysinfo::System::new();
    initial_sys.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
    initial_sys.refresh_memory_specifics(sysinfo::MemoryRefreshKind::everything());
    let ram_used = initial_sys.used_memory() as f64 / BYTES_PER_GIB_F64;
    let ram_total = initial_sys.total_memory() as f64 / BYTES_PER_GIB_F64;
    let host_cpu = initial_sys.global_cpu_usage();

    let now_unix_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let initial_session = compute_session_throughput(&initial_history, &[], now_unix_secs);

    let media_sources_path = PathBuf::from(DEFAULT_MEDIA_SOURCES_PATH);
    let initial_sources = load_media_sources(&media_sources_path);
    let host_info = crate::commands::top::host_info::detect_host_system_info_with_sys(&initial_sys);

    let has_local_sources = initial_sources.iter().any(|s| s.enabled && s.path.exists());
    let initial_is_scanning = has_local_sources;
    let is_remote_streaming =
        !has_local_sources && std::path::Path::new(DEFAULT_SELECTED_BATCH_TARGETS_PATH).exists();
    let initial_telemetry = SystemTelemetry {
        host_info,
        host_cpu_pct: host_cpu,
        ram_used_gib: ram_used,
        ram_total_gib: ram_total,
        history_records: initial_history,
        stats: initial_stats,
        session: initial_session,
        discovered_media: Vec::new(),
        media_sources: initial_sources,
        is_scanning: initial_is_scanning,
        is_remote_streaming,
        ..Default::default()
    };

    let shared_state = Arc::new(RwLock::new(initial_telemetry));
    let gpu_state = Arc::new(RwLock::new(GpuTelemetry::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Spawn GPU monitor thread
    start_gpu_sampler(Arc::clone(&gpu_state), Arc::clone(&stop_flag));

    // Spawn non-blocking background multi-source media scanner thread
    start_media_sources_scanner(Arc::clone(&shared_state), Arc::clone(&stop_flag));

    // Spawn system and process sampler thread
    {
        let shared = Arc::clone(&shared_state);
        let gpu = Arc::clone(&gpu_state);
        let stop = Arc::clone(&stop_flag);
        std::thread::spawn(move || {
            run_telemetry_sampler_loop(shared, gpu, refresh_interval_ms, stop);
        });
    }

    let mut app = AppState::new();

    let loop_res = event_loop(&mut terminal, &mut app, &shared_state);

    // Stop background workers
    stop_flag.store(true, Ordering::Relaxed);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    loop_res
}

/// Executes a single discrete iteration of the UI render pipeline:
/// clamps navigational indices against filtered lengths and paints widgets to terminal.
///
/// # Arguments
/// - `terminal`: Mutable reference to ratatui terminal backend.
/// - `app`: Mutable reference to active application state.
/// - `shared_state`: Atomic reference to shared system telemetry snapshot.
///
/// # Returns
/// - `Ok(SystemTelemetry)`: Cloned copy of the telemetry state rendered during this frame.
/// - `Err(anyhow::Error)`: If ratatui backend drawing encounters an IO error.
///
/// # Data Flow & Invariants
/// Clones `SystemTelemetry` from `shared_state` under read lock.
/// Clamps `selected_index`, `completed_scroll_index`, `media_scroll_index`, and `source_scroll_index`
/// to the boundaries of the visible filtered lists to prevent rendering out-of-bound cursors.
pub fn event_loop_step<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
) -> Result<SystemTelemetry> {
    let telemetry = {
        let lock = shared_state.read().unwrap();
        lock.clone()
    };

    app.clamp_indices(&telemetry);
    terminal.draw(|f| render_ui(f, &app.ui_context(&telemetry, app.current_feedback())))?;

    Ok(telemetry)
}

/// Main non-blocking event loop handling user input polling and screen refresh.
///
/// # Arguments
/// - `terminal`: Mutable reference to ratatui terminal backend.
/// - `app`: Application state container.
/// - `shared_state`: Thread-safe shared telemetry snapshot.
///
/// # Returns
/// - `Ok(())` when `app.should_quit` becomes true.
/// - `Err(anyhow::Error)` on IO or drawing error.
///
/// Determines whether a key event represents a terminal redraw request (Ctrl+L or Ctrl+R).
fn is_redraw_key(key: &crossterm::event::KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(
            key.code,
            KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Char('r')
        )
}

/// Dispatches keyboard or window resize events to terminal and application state.
///
/// # Arguments
/// * `timeout` - Duration to wait for pending key events.
/// * `app` - Mutable reference to application state.
/// * `telemetry` - Reference to active system telemetry snapshot.
/// * `shared_state` - Thread-safe shared telemetry handle.
///
/// # Returns
/// `Ok(true)` if screen redraw is required, `Ok(false)` otherwise.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Polls crossterm event reader with bounded `timeout`.
/// 2. If a key press event is received, forwards to `handle_key_event_with_state`.
fn poll_and_dispatch_key(
    timeout: Duration,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
) -> Result<bool> {
    if !event::poll(timeout)? {
        return Ok(false);
    }
    match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if is_redraw_key(&key) {
                return Ok(true);
            }
            handle_key_event_with_state(key.code, app, telemetry, Some(shared_state));
        }
        Event::Resize(_, _) => return Ok(true),
        _ => {}
    }
    Ok(false)
}

/// # Data Flow & Invariants
/// Loops until `app.should_quit` is set. In each iteration, executes [`event_loop_step`],
/// then polls for keyboard events with bounded timeout up to [`DEFAULT_REFRESH_INTERVAL_MS`].
/// Clears the terminal screen buffer if resize or Ctrl+L redraw request is triggered.
fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
) -> Result<()> {
    let tick_rate = Duration::from_millis(DEFAULT_REFRESH_INTERVAL_MS);
    let mut last_tick = Instant::now();

    while !app.should_quit {
        let telemetry = event_loop_step(terminal, app, shared_state)?;
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::from_millis(KEY_EVENT_POLL_TIMEOUT_MS));

        if poll_and_dispatch_key(timeout, app, &telemetry, shared_state)? {
            terminal.clear()?;
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    Ok(())
}

/// Dispatches key press events to application actions without shared state synchronization.
///
/// # Arguments
/// - `code`: The crossterm [`KeyCode`] received.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
pub fn handle_key_event(code: KeyCode, app: &mut AppState, telemetry: &SystemTelemetry) {
    handle_key_event_with_state(code, app, telemetry, None);
}

/// Dispatches key press events through the shared `dbsrt_tui` state machine, executing host
/// side effects (signals, persistence) and syncing `shared_state` when provided.
pub fn handle_key_event_with_state(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) {
    let mut actions = NativeActions { shared_state };
    dbsrt_tui::app::handle_key_event(to_tui_key(code), app, telemetry, &mut actions);
}

/// Maps a crossterm key code onto the frontend-neutral `dbsrt_tui` key code.
fn to_tui_key(code: KeyCode) -> dbsrt_tui::app::KeyCode {
    use dbsrt_tui::app::KeyCode as K;
    match code {
        KeyCode::Char(c) => K::Char(c),
        KeyCode::Enter => K::Enter,
        KeyCode::Esc => K::Esc,
        KeyCode::Backspace => K::Backspace,
        KeyCode::Delete => K::Delete,
        KeyCode::Tab => K::Tab,
        KeyCode::BackTab => K::BackTab,
        KeyCode::Up => K::Up,
        KeyCode::Down => K::Down,
        KeyCode::Left => K::Left,
        KeyCode::Right => K::Right,
        KeyCode::PageUp => K::PageUp,
        KeyCode::PageDown => K::PageDown,
        KeyCode::Home => K::Home,
        KeyCode::End => K::End,
        _ => K::Null,
    }
}

/// Native host side effects: process signals, JSON persistence and live telemetry sync.
struct NativeActions<'a> {
    shared_state: Option<&'a Arc<RwLock<SystemTelemetry>>>,
}

impl TopActions for NativeActions<'_> {
    fn toggle_pause_job(&mut self, pid: u32) -> String {
        process_ctl::toggle_pause_job(pid).1
    }

    fn stop_job(&mut self, pid: u32) -> String {
        process_ctl::stop_job(pid).1
    }

    fn kill_job(&mut self, pid: u32) -> String {
        process_ctl::kill_job(pid).1
    }

    fn stop_entire_batch(&mut self, force: bool, pids: &[u32]) -> String {
        process_ctl::stop_entire_batch(force, pids).1
    }

    fn launch_selected_batch(&mut self, paths: &[PathBuf]) -> Result<String, String> {
        Ok(sampler::launch_selected_batch(paths).1)
    }

    fn remove_discovered_media(&mut self, paths: &HashSet<PathBuf>) -> Result<(), String> {
        if let Some(shared) = self.shared_state {
            if let Ok(mut state) = shared.write() {
                state.discovered_media.retain(|m| !paths.contains(&m.path));
                let cache_path = std::path::Path::new(DEFAULT_MEDIA_CACHE_PATH);
                save_media_cache(cache_path, &state.discovered_media);
            }
        }
        Ok(())
    }

    fn media_sources(&mut self, telemetry: &SystemTelemetry) -> Vec<MediaSourceConfig> {
        if !telemetry.media_sources.is_empty() {
            telemetry.media_sources.clone()
        } else {
            load_media_sources(std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH))
        }
    }

    fn toggle_media_source(&mut self, telemetry: &SystemTelemetry, index: usize) -> Option<String> {
        let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
        let mut sources = self.media_sources(telemetry);
        let new_state = toggle_media_source(sources_path, &mut sources, index)?;
        if let Some(shared) = self.shared_state {
            if let Ok(mut state) = shared.write() {
                state.media_sources = sources.clone();
            }
        }
        let name = sources
            .get(index)
            .map(|s| s.name.as_str())
            .unwrap_or("Source");
        let state_str = if new_state {
            "activée (incluse dans les futurs scans)"
        } else {
            "désactivée (ignorée lors des scans)"
        };
        Some(format!("Source '{name}' {state_str}."))
    }

    fn save_source_modal(
        &mut self,
        modal: &mut SourceInputModalState,
        telemetry: &SystemTelemetry,
    ) -> (bool, Option<String>) {
        validate_and_save_source_modal(modal, telemetry, self.shared_state)
    }

    fn delete_media_source(
        &mut self,
        app: &mut AppState,
        telemetry: &SystemTelemetry,
        index: usize,
    ) {
        execute_media_source_deletion(app, telemetry, self.shared_state, index);
    }

    fn rescan_media_sources(&mut self) -> String {
        sampler::IS_MEDIA_SCANNING.store(true, Ordering::SeqCst);
        if let Some(shared) = self.shared_state {
            if let Ok(mut state) = shared.write() {
                state.is_scanning = true;
            }
        }
        trigger_media_sources_rescan();
        "⟳ Rescan de toutes les sources lancé...".to_string()
    }

    fn reload_remote_targets(&mut self, telemetry: &SystemTelemetry) -> String {
        let count = reload_remote_streaming_targets(telemetry, self.shared_state);
        format!("⟳ Cibles streaming réseau actualisées ({count} médias).")
    }
}

/// Dispatches source modal addition or modification action to persistent storage.
fn apply_source_modal_action(
    mode: &SourceModalMode,
    sources: &mut Vec<MediaSourceConfig>,
    name: String,
    path: PathBuf,
) -> Result<String, String> {
    let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
    match mode {
        SourceModalMode::Add => add_media_source(sources_path, sources, name, path)
            .map(|s| format!("✓ Source ajoutée : {}", s.name)),
        SourceModalMode::Edit(idx) => update_media_source(sources_path, sources, *idx, name, path)
            .map(|s| format!("✓ Source modifiée : {}", s.name)),
        SourceModalMode::DeleteConfirm(_) => Err("Action invalide".to_string()),
    }
}

/// Handles keyboard interactions when the interactive source modal is active.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state holding the active modal buffer.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
/// - `shared_state`: Optional atomic reference to update shared state on source changes.
///
/// # Returns
/// `true` if a modal was active and processed the keystroke; `false` otherwise.
///
/// # Errors
/// None. Persistence failures surface via `app.set_feedback`.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Intercepts Esc (dismissal), Tab/BackTab/Up/Down (field toggling),
/// Backspace/Char (text mutation), and Enter (validation & file persistence).
/// Validates directory path and persists added or edited media source.
///
/// # Arguments
/// * `modal` - Mutable reference to source input dialog state.
/// * `telemetry` - Active system telemetry snapshot.
/// * `shared_state` - Optional shared telemetry reference for thread-safe mutations.
///
/// # Returns
/// Pair `(success, optional_feedback_message)`.
fn validate_and_save_source_modal(
    modal: &mut SourceInputModalState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) -> (bool, Option<String>) {
    let path_str = modal.path_input.trim();
    if path_str.is_empty() {
        modal.error_msg = Some("Le chemin du répertoire est obligatoire.".to_string());
        return (false, None);
    }
    let path = PathBuf::from(path_str);
    if !path.is_absolute() {
        modal.error_msg =
            Some("Le chemin doit être absolu (ex: /media/... ou /mnt/...).".to_string());
        return (false, None);
    }
    if !path.exists() || !path.is_dir() {
        modal.error_msg = Some(format!(
            "Répertoire introuvable ou inaccessible : '{}'",
            path.display()
        ));
        return (false, None);
    }

    let name = modal.name_input.trim().to_string();
    let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
    let mut sources = if !telemetry.media_sources.is_empty() {
        telemetry.media_sources.clone()
    } else {
        load_media_sources(sources_path)
    };

    let (success, feedback_msg) =
        match apply_source_modal_action(&modal.mode, &mut sources, name, path) {
            Ok(msg) => (true, Some(msg)),
            Err(err) => {
                modal.error_msg = Some(err);
                (false, None)
            }
        };

    if !success {
        return (false, None);
    }

    if let Some(shared) = shared_state {
        if let Ok(mut state) = shared.write() {
            state.media_sources = sources;
        }
    }
    (true, feedback_msg)
}

/// Evicts cached media records belonging to a deleted source from memory and persistent cache.
///
/// # Arguments
/// * `shared_state` - Thread-safe handle to shared telemetry state.
/// * `source_id` - Identifier of the media source being purged.
/// * `app` - Mutable reference to application navigation state.
///
/// # Data Flow & Invariants
/// Retains only items from other sources, persists updated cache to disk, and clamps selection cursors.
fn cleanup_shared_source_media(
    shared_state: &Arc<RwLock<SystemTelemetry>>,
    source_id: &str,
    app: &mut AppState,
) {
    let Ok(mut state) = shared_state.write() else {
        return;
    };
    state
        .discovered_media
        .retain(|item| item.source_id.as_deref() != Some(source_id));
    let cache_path = std::path::Path::new(DEFAULT_MEDIA_CACHE_PATH);
    save_media_cache(cache_path, &state.discovered_media);
    let active_paths: std::collections::HashSet<_> = state
        .discovered_media
        .iter()
        .map(|m| m.path.clone())
        .collect();
    app.selected_media_paths
        .retain(|p| active_paths.contains(p));
    let max_media_idx = state.discovered_media.len().saturating_sub(1);
    if app.media_scroll_index > max_media_idx {
        app.media_scroll_index = max_media_idx;
    }
}

/// Deletes a configured media source from disk, updates shared memory, and adjusts catalog indexes.
///
/// # Arguments
/// * `app` - Mutable application state containing source cursor and filters.
/// * `telemetry` - Active system telemetry snapshot.
/// * `shared_state` - Optional shared telemetry reference for thread-safe mutations.
/// * `index` - Index of the target source within the media sources list.
///
/// # Data Flow & Invariants
/// 1. Persists removal in `media_sources.json`.
/// 2. Evicts associated cached media items from `discovered_media` and `media_cache.json`.
/// 3. Resets `app.source_filter` to `ActiveOnly` if the currently filtered source was deleted.
/// 4. Adjusts `source_scroll_index` and `media_scroll_index` within safe valid bounds.
fn execute_media_source_deletion(
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
    index: usize,
) {
    let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
    let mut sources = if !telemetry.media_sources.is_empty() {
        telemetry.media_sources.clone()
    } else {
        load_media_sources(sources_path)
    };

    let src = match delete_media_source(sources_path, &mut sources, index) {
        Ok(s) => s,
        Err(err) => {
            app.set_feedback(format!("Erreur : {err}"));
            return;
        }
    };

    if let MediaSourceFilter::Specific(ref filter_id) = app.source_filter {
        if filter_id == &src.id {
            app.source_filter = MediaSourceFilter::ActiveOnly;
        }
    }

    if let Some(shared) = shared_state {
        if let Ok(mut state) = shared.write() {
            state.media_sources = sources.clone();
        }
        cleanup_shared_source_media(shared, &src.id, app);
    }

    app.set_feedback(format!("✓ Source supprimée : {}", src.name));
    if app.source_scroll_index >= sources.len() {
        app.source_scroll_index = sources.len().saturating_sub(1);
    }
}

/// Reloads remote streaming batch targets from disk and updates shared telemetry state.
///
/// # Arguments
/// - `telemetry`: Active system telemetry snapshot.
/// - `shared_state`: Optional thread-safe shared telemetry container.
///
/// # Returns
/// Total count of discovered remote streaming media targets.
fn reload_remote_streaming_targets(
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) -> usize {
    let batch_path = std::path::Path::new(DEFAULT_SELECTED_BATCH_TARGETS_PATH);
    let default_source_id = telemetry
        .media_sources
        .first()
        .map(|s| s.id.as_str())
        .unwrap_or(DEFAULT_SOURCE_ID_TVSHOW);
    let active_jobs = telemetry.active_jobs.clone();
    let certified_stems = std::collections::HashSet::new();
    let criteria = crate::commands::top::sampler::MediaDiscoveryCriteria {
        phash_dir: std::path::Path::new("data/__PHASH__"),
        active_jobs: &active_jobs,
        certified_stems: &certified_stems,
        source_id: default_source_id,
    };
    let remote_items =
        crate::commands::top::sampler::load_remote_batch_media_items(batch_path, &criteria);
    let count = remote_items.len();
    if let Some(shared) = shared_state {
        if let Ok(mut state) = shared.write() {
            state.discovered_media = remote_items;
            state.is_remote_streaming = true;
        }
    }
    count
}
