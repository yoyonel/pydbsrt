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
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::commands::top::constants::{
    BYTES_PER_GIB_F64, DEFAULT_BATCH_EVENTS_PATH, DEFAULT_COUPLINGS_PATH, DEFAULT_MEDIA_CACHE_PATH,
    DEFAULT_MEDIA_SOURCES_PATH, FEEDBACK_DISPLAY_DURATION_SECS, KEY_EVENT_POLL_TIMEOUT_MS,
    PAGE_SCROLL_STEP,
};
use crate::commands::top::model::{
    calculate_overall_stats, filter_active_jobs_with_search, filter_and_sort_completed_with_search,
    filter_and_sort_media_with_source_and_search, ActiveJobTelemetry, CompletedFilter,
    CompletedSort, DiscoveredMediaItem, GpuTelemetry, HelpTab, HistoricalRecord, MediaFilter,
    MediaSort, MediaSourceConfig, MediaSourceFilter, SourceInputModalState, SourceModalField,
    SourceModalMode, SystemTelemetry, ViewPage,
};
use crate::commands::top::process_ctl::{kill_job, stop_entire_batch, stop_job, toggle_pause_job};
use crate::commands::top::sampler::{
    add_media_source, compute_session_throughput, delete_media_source, launch_selected_batch,
    load_historical_records, load_media_sources, preload_stem_size_cache,
    run_telemetry_sampler_loop, save_media_cache, start_gpu_sampler, start_media_sources_scanner,
    toggle_media_source, trigger_media_sources_rescan, update_media_source,
};
use crate::commands::top::ui::{render_ui, UiContext};

/// State of the TUI interactive session and navigational cursor offsets.
///
/// # Invariants
/// Maintains active view page, table row selection indices, filter enums, sort orders,
/// search input buffers, modal popups, and transient feedback messages.
#[derive(Default)]
pub struct AppState {
    /// Active full-frame view or diagnostic drill-down page.
    pub view: ViewPage,
    /// Selected row index in Active Jobs table (`[1]`).
    pub selected_index: usize,
    /// Scroll offset index in Completed Media table (`[3/r]`).
    pub completed_scroll_index: usize,
    /// Scroll offset index in Media Browser table (`[4/m]`).
    pub media_scroll_index: usize,
    /// Scroll offset index in Media Sources table (`[5/s]`).
    pub source_scroll_index: usize,
    /// Modal state when adding or editing a media storage source.
    pub source_modal: Option<SourceInputModalState>,
    /// Set of multi-selected candidate media paths tagged with Spacebar (`[Space]`).
    pub selected_media_paths: HashSet<PathBuf>,
    /// Active filter criterion in Media Browser (`[f]`).
    pub media_filter: MediaFilter,
    /// Active sort criterion in Media Browser (`[o]`).
    pub media_sort: MediaSort,
    /// Active source filter in Media Browser (`[s]`).
    pub source_filter: MediaSourceFilter,
    /// Active filter criterion in Completed Media (`[f]`).
    pub completed_filter: CompletedFilter,
    /// Active sort criterion in Completed Media (`[o]`).
    pub completed_sort: CompletedSort,
    /// Interactive fuzzy search query string for Active Jobs view.
    pub jobs_search: String,
    /// Interactive fuzzy search query string for Completed Media view.
    pub completed_search: String,
    /// Interactive fuzzy search query string for Media Browser view.
    pub media_search: String,
    /// Flag indicating whether the interactive search input bar is currently focused (`/`).
    pub search_active: bool,
    /// Active tab in fullframe contextual Help view (`[?]`).
    pub help_tab: HelpTab,
    /// Scroll line offset in Help view.
    pub help_scroll_index: usize,
    /// Previous view page that requested help, for seamless restoration on Esc.
    pub help_caller_view: ViewPage,
    /// Ephemeral feedback toast message with timestamp: `(message, instant)`.
    pub feedback_message: Option<(String, Instant)>,
    /// Termination sentinel flag to break the main event loop.
    pub should_quit: bool,
}

impl AppState {
    /// Instantiates default initial application state.
    ///
    /// # Returns
    /// Initialized [`AppState`] with default zeroed cursors and active filters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieves the active search query string for the currently focused view page.
    ///
    /// # Arguments
    /// None (reads `&self`).
    ///
    /// # Returns
    /// String slice of the active view's search filter, or `""` if view does not support search.
    pub fn current_search_query(&self) -> &str {
        match self.view {
            ViewPage::ActiveJobs | ViewPage::JobInspection => &self.jobs_search,
            ViewPage::CompletedMedia | ViewPage::CompletedInspection => &self.completed_search,
            ViewPage::MediaBrowser | ViewPage::MediaInspection => &self.media_search,
            _ => "",
        }
    }

    /// Retrieves a mutable reference to the active view page's search query string.
    ///
    /// # Arguments
    /// None (operates on `&mut self`).
    ///
    /// # Returns
    /// `Some(&mut String)` pointing to `jobs_search`, `completed_search`, or `media_search`.
    /// `None` if the active view is not searchable.
    pub fn current_search_query_mut(&mut self) -> Option<&mut String> {
        match self.view {
            ViewPage::ActiveJobs | ViewPage::JobInspection => Some(&mut self.jobs_search),
            ViewPage::CompletedMedia | ViewPage::CompletedInspection => {
                Some(&mut self.completed_search)
            }
            ViewPage::MediaBrowser | ViewPage::MediaInspection => Some(&mut self.media_search),
            _ => None,
        }
    }

    /// Resets the table row cursor offset to 0 for the active view.
    ///
    /// # Data Flow & Invariants
    /// Invoked whenever the search query, filter predicate, or sort criteria change
    /// to avoid leaving the cursor pointing beyond the newly filtered item count.
    pub fn reset_current_view_scroll(&mut self) {
        match self.view {
            ViewPage::ActiveJobs => self.selected_index = 0,
            ViewPage::CompletedMedia => self.completed_scroll_index = 0,
            ViewPage::MediaBrowser => self.media_scroll_index = 0,
            _ => {}
        }
    }

    /// Filters raw active job snapshots using the current search query.
    ///
    /// # Arguments
    /// - `jobs`: Slice of active job telemetry metrics.
    ///
    /// # Returns
    /// Filtered vector of active job telemetry items.
    pub fn visible_jobs(&self, jobs: &[ActiveJobTelemetry]) -> Vec<ActiveJobTelemetry> {
        filter_active_jobs_with_search(
            jobs,
            if self.jobs_search.is_empty() {
                None
            } else {
                Some(&self.jobs_search)
            },
        )
    }

    /// Filters and sorts candidate media items in the library according to active view settings.
    ///
    /// # Arguments
    /// - `discovered`: Slice of candidate media items from library catalog.
    /// - `sources`: Configured media sources slice.
    ///
    /// # Returns
    /// Filtered and sorted vector of media items.
    pub fn visible_media(
        &self,
        discovered: &[DiscoveredMediaItem],
        sources: &[MediaSourceConfig],
    ) -> Vec<DiscoveredMediaItem> {
        filter_and_sort_media_with_source_and_search(
            discovered,
            self.media_filter,
            self.media_sort,
            &self.source_filter,
            sources,
            if self.media_search.is_empty() {
                None
            } else {
                Some(&self.media_search)
            },
        )
    }

    /// Filters and sorts historical batch records according to active view settings.
    ///
    /// # Arguments
    /// - `records`: Historical execution records from batch execution logs.
    ///
    /// # Returns
    /// Filtered and sorted vector of historical execution records.
    pub fn visible_completed(&self, records: &[HistoricalRecord]) -> Vec<HistoricalRecord> {
        filter_and_sort_completed_with_search(
            records,
            self.completed_filter,
            self.completed_sort,
            if self.completed_search.is_empty() {
                None
            } else {
                Some(&self.completed_search)
            },
        )
    }

    /// Sets an ephemeral status bar feedback toast message.
    ///
    /// # Arguments
    /// - `msg`: Formatted message string to display in header.
    ///
    /// # Data Flow & Invariants
    /// Stores the message paired with `Instant::now()`. Automatically expires
    /// after [`FEEDBACK_DISPLAY_DURATION_SECS`].
    pub fn set_feedback(&mut self, msg: String) {
        self.feedback_message = Some((msg, Instant::now()));
    }

    /// Returns the active feedback message if it has not expired.
    ///
    /// # Returns
    /// `Some(&str)` if within the TTL window, `None` if absent or expired.
    pub fn current_feedback(&self) -> Option<&str> {
        if let Some((ref msg, instant)) = self.feedback_message {
            if instant.elapsed().as_secs_f64() < FEEDBACK_DISPLAY_DURATION_SECS {
                return Some(msg.as_str());
            }
        }
        None
    }
}

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
    let events_path = std::path::PathBuf::from(DEFAULT_BATCH_EVENTS_PATH);
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

    let initial_telemetry = SystemTelemetry {
        host_cpu_pct: host_cpu,
        ram_used_gib: ram_used,
        ram_total_gib: ram_total,
        history_records: initial_history,
        stats: initial_stats,
        session: initial_session,
        discovered_media: Vec::new(),
        media_sources: initial_sources,
        is_scanning: true,
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

    let visible_jobs = app.visible_jobs(&telemetry.active_jobs);
    if visible_jobs.is_empty() {
        app.selected_index = 0;
    } else if app.selected_index >= visible_jobs.len() {
        app.selected_index = visible_jobs.len().saturating_sub(1);
    }

    let visible_completed = app.visible_completed(&telemetry.history_records);
    if visible_completed.is_empty() {
        app.completed_scroll_index = 0;
    } else if app.completed_scroll_index >= visible_completed.len() {
        app.completed_scroll_index = visible_completed.len().saturating_sub(1);
    }

    let visible_media = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
    if visible_media.is_empty() {
        app.media_scroll_index = 0;
    } else if app.media_scroll_index >= visible_media.len() {
        app.media_scroll_index = visible_media.len().saturating_sub(1);
    }

    if telemetry.media_sources.is_empty() {
        app.source_scroll_index = 0;
    } else if app.source_scroll_index >= telemetry.media_sources.len() {
        app.source_scroll_index = telemetry.media_sources.len().saturating_sub(1);
    }

    let feedback = app.current_feedback();
    let search_query = app.current_search_query();
    let search_active = app.search_active;

    terminal.draw(|f| {
        let ctx = UiContext {
            telemetry: &telemetry,
            view: app.view,
            selected_index: app.selected_index,
            completed_scroll_index: app.completed_scroll_index,
            media_scroll_index: app.media_scroll_index,
            source_scroll_index: app.source_scroll_index,
            source_modal: app.source_modal.as_ref(),
            selected_media_paths: &app.selected_media_paths,
            media_filter: app.media_filter,
            media_sort: app.media_sort,
            source_filter: &app.source_filter,
            completed_filter: app.completed_filter,
            completed_sort: app.completed_sort,
            search_query,
            search_active,
            help_tab: app.help_tab,
            help_scroll_index: app.help_scroll_index,
            feedback_msg: feedback,
        };
        render_ui(f, &ctx);
    })?;

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
/// # Data Flow & Invariants
/// Loops until `app.should_quit` is set. In each iteration, executes [`event_loop_step`],
/// then polls for keyboard events with timeout [`KEY_EVENT_POLL_TIMEOUT_MS`] (20ms).
fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
) -> Result<()> {
    while !app.should_quit {
        let telemetry = event_loop_step(terminal, app, shared_state)?;

        // Non-blocking poll for keyboard events (< 20ms response time)
        if event::poll(Duration::from_millis(KEY_EVENT_POLL_TIMEOUT_MS))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key_event_with_state(key.code, app, &telemetry, Some(shared_state));
                }
            }
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

/// Dispatches key press events to application actions with optional live shared_state synchronization.
///
/// # Arguments
/// - `code`: The crossterm [`KeyCode`] received.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
/// - `shared_state`: Optional atomic reference to update shared state on source changes.
///
/// # Returns
/// None.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Priority dispatch hierarchy:
/// 1. **Modal Interception**: If `app.source_modal` is open, intercepts typing, Tab, Backspace, Esc, and Enter.
/// 2. **Interactive Search Input**: If `app.search_active` is true, routes keystrokes to active query buffer.
/// 3. **Global Navigation**: Number keys (`1`..`5`), view hotkeys (`g`, `h`, `r`, `m`, `s`), help (`?`), quit (`q`).
/// 4. **Viewport Cursor & Scroll**: Up/Down/'j'/'k', PageUp/PageDown, Home/End.
/// 5. **View-Specific Actions**: Dedicated handlers per active page.
pub fn handle_key_event_with_state(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) {
    if handle_modal_key_event(code, app, telemetry, shared_state) {
        return;
    }
    if handle_search_active_key_event(code, app) {
        return;
    }
    if handle_global_navigation_key_event(code, app) {
        return;
    }
    if handle_scroll_navigation_key_event(code, app, telemetry) {
        return;
    }
    handle_view_specific_key_event(code, app, telemetry, shared_state);
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

/// Mutates the active text input buffer in the source modal on typing or deletion.
///
/// # Arguments
/// * `modal` - Mutable modal dialog state holding input buffers.
/// * `is_backspace` - True if pop operation, false if push.
/// * `ch` - Character to append if typing.
fn mutate_modal_input_field(
    modal: &mut SourceInputModalState,
    is_backspace: bool,
    ch: Option<char>,
) {
    let target = match modal.active_field {
        SourceModalField::Name => &mut modal.name_input,
        SourceModalField::Path => &mut modal.path_input,
    };
    if is_backspace {
        target.pop();
    } else if let Some(c) = ch {
        target.push(c);
    }
}

/// Handles keystrokes when the source addition or edition input form is focused.
///
/// # Arguments
/// * `code` - Input keyboard event code.
/// * `app` - Mutable reference to application state.
/// * `telemetry` - Active system telemetry snapshot.
/// * `shared_state` - Optional shared telemetry reference for thread-safe mutations.
///
/// # Returns
/// Always returns `true` to signal key event consumption.
fn handle_source_input_modal_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) -> bool {
    if code == KeyCode::Enter {
        let (success, feedback) = {
            let Some(ref mut modal) = app.source_modal else {
                return false;
            };
            validate_and_save_source_modal(modal, telemetry, shared_state)
        };
        if success {
            app.source_modal = None;
            if let Some(msg) = feedback {
                app.set_feedback(msg);
            }
        }
        return true;
    }

    let Some(ref mut modal) = app.source_modal else {
        return false;
    };

    match code {
        KeyCode::Esc => {
            app.source_modal = None;
        }
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
            modal.active_field = match modal.active_field {
                SourceModalField::Name => SourceModalField::Path,
                SourceModalField::Path => SourceModalField::Name,
            };
        }
        KeyCode::Backspace => {
            modal.error_msg = None;
            mutate_modal_input_field(modal, true, None);
        }
        KeyCode::Char(c) => {
            modal.error_msg = None;
            mutate_modal_input_field(modal, false, Some(c));
        }
        _ => {}
    }
    true
}

/// Handles keyboard inputs when the deletion confirmation popup is active.
///
/// # Arguments
/// * `code` - Input keyboard event code.
/// * `app` - Mutable reference to application state.
/// * `telemetry` - Active system telemetry snapshot.
/// * `shared_state` - Optional shared telemetry reference for thread-safe mutations.
///
/// # Returns
/// Always returns `true` to indicate modal event consumption.
fn handle_delete_confirm_modal_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) -> bool {
    let Some(ref modal) = app.source_modal else {
        return false;
    };
    let SourceModalMode::DeleteConfirm(idx) = modal.mode else {
        return false;
    };

    match code {
        KeyCode::Char('y')
        | KeyCode::Char('Y')
        | KeyCode::Char('o')
        | KeyCode::Char('O')
        | KeyCode::Enter => {
            app.source_modal = None;
            execute_media_source_deletion(app, telemetry, shared_state, idx);
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.source_modal = None;
            app.set_feedback("Suppression de la source annulée.".to_string());
        }
        _ => {}
    }
    true
}

/// Intercepts and dispatches keystrokes targeted at open modal dialogs.
///
/// # Arguments
/// * `code` - Input keyboard event code.
/// * `app` - Mutable reference to application state.
/// * `telemetry` - Active system telemetry snapshot.
/// * `shared_state` - Optional shared telemetry reference for thread-safe mutations.
///
/// # Returns
/// `true` if an active modal intercepted the event, `false` otherwise.
///
/// # Errors
/// None. Persistence failures surface via `app.set_feedback`.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Dispatches directly to [`handle_delete_confirm_modal_key_event`] if mode is DeleteConfirm,
/// or [`handle_source_input_modal_key_event`] for Add/Edit configuration forms.
fn handle_modal_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) -> bool {
    let Some(ref modal) = app.source_modal else {
        return false;
    };

    if matches!(modal.mode, SourceModalMode::DeleteConfirm(_)) {
        handle_delete_confirm_modal_key_event(code, app, telemetry, shared_state)
    } else {
        handle_source_input_modal_key_event(code, app, telemetry, shared_state)
    }
}

/// Handles keystrokes when interactive search mode is active.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state holding search query buffers.
///
/// # Returns
/// `true` if search mode was active and handled the key event; `false` otherwise.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Intercepts Esc (cancel query & deactivate), Enter (validate query & deactivate),
/// Backspace (pop character), and Char (append character). Viewport scroll index
/// is reset to 0 on any query buffer mutation.
fn handle_search_active_key_event(code: KeyCode, app: &mut AppState) -> bool {
    if !app.search_active {
        return false;
    }

    match code {
        KeyCode::Esc => {
            app.search_active = false;
            if let Some(query) = app.current_search_query_mut() {
                query.clear();
            }
            app.reset_current_view_scroll();
            app.set_feedback("Recherche annulée.".to_string());
        }
        KeyCode::Enter => {
            app.search_active = false;
            app.reset_current_view_scroll();
            let q = app.current_search_query().to_string();
            if q.trim().is_empty() {
                if let Some(query) = app.current_search_query_mut() {
                    query.clear();
                }
                app.set_feedback("Filtre de recherche désactivé.".to_string());
            } else {
                app.set_feedback(format!("Recherche active : \"{q}\""));
            }
        }
        KeyCode::Backspace => {
            if let Some(query) = app.current_search_query_mut() {
                query.pop();
            }
            app.reset_current_view_scroll();
        }
        KeyCode::Char(c) => {
            if let Some(query) = app.current_search_query_mut() {
                query.push(c);
            }
            app.reset_current_view_scroll();
        }
        _ => {}
    }
    true
}

/// Handles global application navigation, view switching, and search activation.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
///
/// # Returns
/// `true` if a global key was processed; `false` if the event should fall through to view handlers.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Intercepts 'q'/'Q' (quit flag), '?' (help view toggle), Esc/Backspace (hierarchical back navigation),
/// tab cycling, direct view shortcuts, and '/' (search activation in searchable views).
fn handle_global_navigation_key_event(code: KeyCode, app: &mut AppState) -> bool {
    if handle_tab_cycling(code, app) || handle_direct_view_shortcuts(code, app) {
        return true;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.should_quit = true;
            true
        }
        KeyCode::Char('?') => {
            if app.view == ViewPage::Help {
                app.view = app.help_caller_view;
            } else {
                app.help_caller_view = app.view;
                app.help_tab = HelpTab::from_view(app.view);
                app.help_scroll_index = 0;
                app.view = ViewPage::Help;
            }
            true
        }
        KeyCode::Esc => {
            if !app.current_search_query().is_empty() {
                if let Some(query) = app.current_search_query_mut() {
                    query.clear();
                }
                app.reset_current_view_scroll();
                app.set_feedback("Recherche effacée.".to_string());
            } else {
                match app.view {
                    ViewPage::ActiveJobs => {
                        app.should_quit = true;
                    }
                    ViewPage::MediaInspection => {
                        app.view = ViewPage::MediaBrowser;
                    }
                    ViewPage::CompletedInspection => {
                        app.view = ViewPage::CompletedMedia;
                    }
                    ViewPage::Help => {
                        app.view = app.help_caller_view;
                    }
                    ViewPage::JobInspection
                    | ViewPage::StatsHistory
                    | ViewPage::CompletedMedia
                    | ViewPage::MediaBrowser
                    | ViewPage::MediaSources => {
                        app.view = ViewPage::ActiveJobs;
                    }
                }
            }
            true
        }
        KeyCode::Backspace if app.view != ViewPage::ActiveJobs => {
            if app.view == ViewPage::MediaInspection {
                app.view = ViewPage::MediaBrowser;
            } else if app.view == ViewPage::CompletedInspection {
                app.view = ViewPage::CompletedMedia;
            } else if app.view == ViewPage::Help {
                app.view = app.help_caller_view;
            } else {
                app.view = ViewPage::ActiveJobs;
            }
            true
        }
        KeyCode::Char('/')
            if matches!(
                app.view,
                ViewPage::ActiveJobs | ViewPage::CompletedMedia | ViewPage::MediaBrowser
            ) =>
        {
            app.search_active = true;
            app.set_feedback(
                "Recherche : tapez votre texte (Entrée pour valider, Échap pour annuler)..."
                    .to_string(),
            );
            true
        }
        _ => false,
    }
}

/// Handles linear tab navigation cycling between views.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
///
/// # Returns
/// `true` if a tab cycling key was processed; `false` otherwise.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Tab/Right advances to the next view/help tab, BackTab/Left retreats to the previous view/help tab.
fn handle_tab_cycling(code: KeyCode, app: &mut AppState) -> bool {
    match code {
        KeyCode::Tab | KeyCode::Right => {
            if app.view == ViewPage::Help {
                app.help_tab = app.help_tab.next();
                app.help_scroll_index = 0;
            } else {
                app.view = match app.view {
                    ViewPage::ActiveJobs => ViewPage::StatsHistory,
                    ViewPage::StatsHistory => ViewPage::CompletedMedia,
                    ViewPage::CompletedMedia => ViewPage::MediaBrowser,
                    ViewPage::MediaBrowser => ViewPage::MediaSources,
                    ViewPage::MediaSources
                    | ViewPage::JobInspection
                    | ViewPage::MediaInspection
                    | ViewPage::CompletedInspection
                    | ViewPage::Help => ViewPage::ActiveJobs,
                };
            }
            true
        }
        KeyCode::BackTab | KeyCode::Left => {
            if app.view == ViewPage::Help {
                app.help_tab = app.help_tab.prev();
                app.help_scroll_index = 0;
            } else {
                app.view = match app.view {
                    ViewPage::ActiveJobs => ViewPage::MediaSources,
                    ViewPage::MediaSources => ViewPage::MediaBrowser,
                    ViewPage::MediaBrowser => ViewPage::CompletedMedia,
                    ViewPage::CompletedMedia => ViewPage::StatsHistory,
                    ViewPage::StatsHistory => ViewPage::ActiveJobs,
                    ViewPage::JobInspection
                    | ViewPage::MediaInspection
                    | ViewPage::CompletedInspection
                    | ViewPage::Help => ViewPage::ActiveJobs,
                };
            }
            true
        }
        _ => false,
    }
}

/// Handles numeric and mnemonic direct view switching shortcuts.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
///
/// # Returns
/// `true` if a direct view shortcut key was processed; `false` otherwise.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Direct routing to views 1..5 and aliases ('h' for StatsHistory, 'm' for MediaBrowser).
fn handle_direct_view_shortcuts(code: KeyCode, app: &mut AppState) -> bool {
    match code {
        KeyCode::Char('1') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::ActiveJobs;
                app.help_scroll_index = 0;
            } else {
                app.view = ViewPage::ActiveJobs;
            }
            true
        }
        KeyCode::Char('2') | KeyCode::Char('h') | KeyCode::Char('H') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::StatsHistory;
                app.help_scroll_index = 0;
            } else if app.view == ViewPage::StatsHistory {
                app.view = ViewPage::ActiveJobs;
            } else {
                app.view = ViewPage::StatsHistory;
            }
            true
        }
        KeyCode::Char('3') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::Completed;
                app.help_scroll_index = 0;
            } else if app.view == ViewPage::CompletedMedia {
                app.view = ViewPage::ActiveJobs;
            } else {
                app.view = ViewPage::CompletedMedia;
            }
            true
        }
        KeyCode::Char('4') | KeyCode::Char('m') | KeyCode::Char('M') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::MediaBrowser;
                app.help_scroll_index = 0;
            } else if app.view == ViewPage::MediaBrowser {
                app.view = ViewPage::ActiveJobs;
            } else {
                app.view = ViewPage::MediaBrowser;
            }
            true
        }
        KeyCode::Char('5') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::MediaSources;
                app.help_scroll_index = 0;
            } else if app.view == ViewPage::MediaSources {
                app.view = ViewPage::ActiveJobs;
            } else {
                app.view = ViewPage::MediaSources;
            }
            true
        }
        _ => false,
    }
}

/// Handles vertical scrolling and cursor movement across all scrollable views.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state holding scroll indexes.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
///
/// # Returns
/// `true` if a scroll navigation key was processed; `false` otherwise.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Modifies view-specific scroll cursor (ActiveJobs, CompletedMedia, MediaBrowser, MediaSources, Help)
/// with safe boundary clamping against collection length. Supports single-step navigation (Up/Down/'k'/'j'),
/// paging (PageUp/PageDown by [`PAGE_SCROLL_STEP`]), and jump-to-boundary (Home/End).
/// Scrolls one step upward across the active view.
fn scroll_up(app: &mut AppState) {
    match app.view {
        ViewPage::ActiveJobs if app.selected_index > 0 => {
            app.selected_index -= 1;
        }
        ViewPage::CompletedMedia if app.completed_scroll_index > 0 => {
            app.completed_scroll_index -= 1;
        }
        ViewPage::MediaBrowser if app.media_scroll_index > 0 => {
            app.media_scroll_index -= 1;
        }
        ViewPage::MediaSources if app.source_scroll_index > 0 => {
            app.source_scroll_index -= 1;
        }
        ViewPage::Help if app.help_scroll_index > 0 => {
            app.help_scroll_index -= 1;
        }
        _ => {}
    }
}

fn handle_scroll_navigation_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
) -> bool {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            scroll_up(app);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            scroll_down(app, telemetry);
            true
        }
        KeyCode::PageUp => {
            scroll_page_up(app);
            true
        }
        KeyCode::PageDown => {
            scroll_page_down(app, telemetry);
            true
        }
        KeyCode::Home => {
            scroll_home(app);
            true
        }
        KeyCode::End => {
            scroll_end(app, telemetry);
            true
        }
        _ => false,
    }
}

/// Scrolls one step downward across the active view.
fn scroll_down(app: &mut AppState, telemetry: &SystemTelemetry) {
    match app.view {
        ViewPage::ActiveJobs
            if !telemetry.active_jobs.is_empty()
                && app.selected_index + 1 < telemetry.active_jobs.len() =>
        {
            app.selected_index += 1;
        }
        ViewPage::CompletedMedia => {
            let visible = app.visible_completed(&telemetry.history_records);
            if !visible.is_empty() && app.completed_scroll_index + 1 < visible.len() {
                app.completed_scroll_index += 1;
            }
        }
        ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            if !visible.is_empty() && app.media_scroll_index + 1 < visible.len() {
                app.media_scroll_index += 1;
            }
        }
        ViewPage::MediaSources => {
            let count = if !telemetry.media_sources.is_empty() {
                telemetry.media_sources.len()
            } else {
                1
            };
            if app.source_scroll_index + 1 < count {
                app.source_scroll_index += 1;
            }
        }
        ViewPage::Help => {
            app.help_scroll_index += 1;
        }
        _ => {}
    }
}

/// Scrolls one page upward across the active view.
fn scroll_page_up(app: &mut AppState) {
    match app.view {
        ViewPage::ActiveJobs => {
            app.selected_index = app.selected_index.saturating_sub(PAGE_SCROLL_STEP);
        }
        ViewPage::CompletedMedia => {
            app.completed_scroll_index =
                app.completed_scroll_index.saturating_sub(PAGE_SCROLL_STEP);
        }
        ViewPage::MediaBrowser => {
            app.media_scroll_index = app.media_scroll_index.saturating_sub(PAGE_SCROLL_STEP);
        }
        ViewPage::MediaSources => {
            app.source_scroll_index = app.source_scroll_index.saturating_sub(PAGE_SCROLL_STEP);
        }
        ViewPage::Help => {
            app.help_scroll_index = app.help_scroll_index.saturating_sub(PAGE_SCROLL_STEP);
        }
        _ => {}
    }
}

/// Scrolls one page downward across the active view.
fn scroll_page_down(app: &mut AppState, telemetry: &SystemTelemetry) {
    match app.view {
        ViewPage::ActiveJobs => {
            let max_idx = telemetry.active_jobs.len().saturating_sub(1);
            app.selected_index = (app.selected_index + PAGE_SCROLL_STEP).min(max_idx);
        }
        ViewPage::CompletedMedia => {
            let visible = app.visible_completed(&telemetry.history_records);
            let max_idx = visible.len().saturating_sub(1);
            app.completed_scroll_index =
                (app.completed_scroll_index + PAGE_SCROLL_STEP).min(max_idx);
        }
        ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            let max_idx = visible.len().saturating_sub(1);
            app.media_scroll_index = (app.media_scroll_index + PAGE_SCROLL_STEP).min(max_idx);
        }
        ViewPage::MediaSources => {
            let max_idx = telemetry.media_sources.len().saturating_sub(1);
            app.source_scroll_index = (app.source_scroll_index + PAGE_SCROLL_STEP).min(max_idx);
        }
        ViewPage::Help => {
            app.help_scroll_index += PAGE_SCROLL_STEP;
        }
        _ => {}
    }
}

/// Scrolls to the beginning of the active view.
fn scroll_home(app: &mut AppState) {
    match app.view {
        ViewPage::ActiveJobs => {
            app.selected_index = 0;
        }
        ViewPage::CompletedMedia => {
            app.completed_scroll_index = 0;
        }
        ViewPage::MediaBrowser => {
            app.media_scroll_index = 0;
        }
        ViewPage::MediaSources => {
            app.source_scroll_index = 0;
        }
        ViewPage::Help => {
            app.help_scroll_index = 0;
        }
        _ => {}
    }
}

/// Scrolls to the end of the active view.
fn scroll_end(app: &mut AppState, telemetry: &SystemTelemetry) {
    match app.view {
        ViewPage::ActiveJobs => {
            app.selected_index = telemetry.active_jobs.len().saturating_sub(1);
        }
        ViewPage::CompletedMedia => {
            let visible = app.visible_completed(&telemetry.history_records);
            app.completed_scroll_index = visible.len().saturating_sub(1);
        }
        ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            app.media_scroll_index = visible.len().saturating_sub(1);
        }
        ViewPage::MediaSources => {
            app.source_scroll_index = telemetry.media_sources.len().saturating_sub(1);
        }
        ViewPage::Help => {
            app.help_scroll_index = 100;
        }
        _ => {}
    }
}

/// Dispatches view-specific operational keys to dedicated page sub-handlers.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
/// - `shared_state`: Optional atomic reference to update shared state.
///
/// # Returns
/// None.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Routes keystroke events depending on `app.view` to the corresponding view handler.
fn handle_view_specific_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) {
    match app.view {
        ViewPage::Help => handle_help_view_keys(code, app),
        ViewPage::ActiveJobs | ViewPage::JobInspection => {
            handle_active_jobs_view_keys(code, app, telemetry);
        }
        ViewPage::CompletedMedia | ViewPage::CompletedInspection => {
            handle_completed_media_view_keys(code, app);
        }
        ViewPage::MediaBrowser | ViewPage::MediaInspection => {
            handle_media_browser_view_keys(code, app, telemetry, shared_state);
        }
        ViewPage::MediaSources => {
            handle_media_sources_view_keys(code, app, telemetry, shared_state);
        }
        ViewPage::StatsHistory => {
            handle_stats_history_view_keys(code, app);
        }
    }
}

/// Handles keypresses specific to the Help screen tabs.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
///
/// # Returns
/// None. Mutates `app.help_tab`.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Direct mnemonic shortcut routing to help tabs ('0'/'g' for General, 'r' for Completed, 's' for MediaSources).
fn handle_help_view_keys(code: KeyCode, app: &mut AppState) {
    match code {
        KeyCode::Char('0') | KeyCode::Char('g') | KeyCode::Char('G') => {
            app.help_tab = HelpTab::General;
            app.help_scroll_index = 0;
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.help_tab = HelpTab::Completed;
            app.help_scroll_index = 0;
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.help_tab = HelpTab::MediaSources;
            app.help_scroll_index = 0;
        }
        _ => {}
    }
}

/// Handles keypresses specific to ActiveJobs and JobInspection views.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
///
/// # Returns
/// None. Mutates application view or signals external processes.
///
/// # Errors
/// None. Process signaling failures surface via `app.set_feedback`.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Intercepts 'p'/'P' (pause/resume SIGSTOP/CONT), 's'/'S' (stop SIGTERM), 'K' (kill SIGKILL),
/// 'x'/'X' (batch termination), Enter/'l'/'i'/'I' (inspection toggle), and 'r'/'R' (Completed view jump).
fn handle_active_jobs_view_keys(code: KeyCode, app: &mut AppState, telemetry: &SystemTelemetry) {
    match code {
        KeyCode::Char('p') | KeyCode::Char('P') => {
            if !telemetry.active_jobs.is_empty() && app.selected_index < telemetry.active_jobs.len()
            {
                let pid = telemetry.active_jobs[app.selected_index].pid;
                let (_ok, msg) = toggle_pause_job(pid);
                app.set_feedback(msg);
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            if !telemetry.active_jobs.is_empty() && app.selected_index < telemetry.active_jobs.len()
            {
                let pid = telemetry.active_jobs[app.selected_index].pid;
                let (_ok, msg) = stop_job(pid);
                app.set_feedback(msg);
            }
        }
        KeyCode::Char('K')
            if app.view == ViewPage::JobInspection
                && !telemetry.active_jobs.is_empty()
                && app.selected_index < telemetry.active_jobs.len() =>
        {
            let pid = telemetry.active_jobs[app.selected_index].pid;
            let (_ok, msg) = kill_job(pid);
            app.set_feedback(msg);
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
            let active_pids: Vec<u32> = telemetry.active_jobs.iter().map(|j| j.pid).collect();
            let (_ok, msg) = stop_entire_batch(code == KeyCode::Char('X'), &active_pids);
            app.set_feedback(msg);
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('i') | KeyCode::Char('I') => {
            if app.view == ViewPage::ActiveJobs && !telemetry.active_jobs.is_empty() {
                app.view = ViewPage::JobInspection;
            } else if app.view == ViewPage::JobInspection {
                app.view = ViewPage::ActiveJobs;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.view = ViewPage::CompletedMedia;
        }
        _ => {}
    }
}

/// Handles keypresses specific to CompletedMedia and CompletedInspection views.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
///
/// # Returns
/// None. Mutates filters, sorts, or inspection view state.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Intercepts 'f'/'F' (filter rotation), 'o'/'O' (sort rotation), Enter/'l'/'i'/'I' (inspection toggle),
/// 'r'/'R' (toggle back to ActiveJobs), and 's'/'S' (jump to MediaSources).
fn handle_completed_media_view_keys(code: KeyCode, app: &mut AppState) {
    match code {
        KeyCode::Char('f') | KeyCode::Char('F') if app.view == ViewPage::CompletedMedia => {
            app.completed_filter = app.completed_filter.next();
            app.completed_scroll_index = 0;
            app.set_feedback(format!("Filtre Récents : {}", app.completed_filter.label()));
        }
        KeyCode::Char('o') | KeyCode::Char('O') if app.view == ViewPage::CompletedMedia => {
            app.completed_sort = app.completed_sort.next();
            app.completed_scroll_index = 0;
            app.set_feedback(format!("Tri Récents : {}", app.completed_sort.label()));
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('i') | KeyCode::Char('I') => {
            if app.view == ViewPage::CompletedMedia {
                app.view = ViewPage::CompletedInspection;
            } else if app.view == ViewPage::CompletedInspection {
                app.view = ViewPage::CompletedMedia;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') if app.view == ViewPage::CompletedMedia => {
            app.view = ViewPage::ActiveJobs;
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.view = ViewPage::MediaSources;
        }
        _ => {}
    }
}

/// Handles item selection toggling in media browser and inspection views.
fn handle_media_browser_selection_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
) -> bool {
    match code {
        KeyCode::Char(' ') if app.view == ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            if let Some(item) = visible.get(app.media_scroll_index) {
                if app.selected_media_paths.contains(&item.path) {
                    app.selected_media_paths.remove(&item.path);
                } else {
                    app.selected_media_paths.insert(item.path.clone());
                }
            }
            true
        }
        KeyCode::Char(' ') if app.view == ViewPage::MediaInspection => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            if app.media_scroll_index < visible.len() {
                let p = &visible[app.media_scroll_index].path;
                if app.selected_media_paths.contains(p) {
                    app.selected_media_paths.remove(p);
                    app.set_feedback("Média décoché.".to_string());
                } else {
                    app.selected_media_paths.insert(p.clone());
                    app.set_feedback("Média coché pour batch.".to_string());
                }
            }
            true
        }
        KeyCode::Char('a') | KeyCode::Char('A') if app.view == ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            let all_selected = !visible.is_empty()
                && visible
                    .iter()
                    .all(|item| app.selected_media_paths.contains(&item.path));
            if all_selected {
                for item in &visible {
                    app.selected_media_paths.remove(&item.path);
                }
            } else {
                for item in &visible {
                    app.selected_media_paths.insert(item.path.clone());
                }
            }
            true
        }
        _ => false,
    }
}

/// Handles filter and sort toggling in media browser.
fn handle_media_browser_filter_sort_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
) -> bool {
    match code {
        KeyCode::Char('f') | KeyCode::Char('F') if app.view == ViewPage::MediaBrowser => {
            app.media_filter = app.media_filter.next();
            app.media_scroll_index = 0;
            app.set_feedback(format!("Filtre actif : {}", app.media_filter.label()));
            true
        }
        KeyCode::Char('o') | KeyCode::Char('O') if app.view == ViewPage::MediaBrowser => {
            app.media_sort = app.media_sort.next();
            app.media_scroll_index = 0;
            app.set_feedback(format!("Tri actif : {}", app.media_sort.label()));
            true
        }
        KeyCode::Char('S') if app.view == ViewPage::MediaBrowser => {
            let next_filter = match &app.source_filter {
                MediaSourceFilter::ActiveOnly => MediaSourceFilter::All,
                MediaSourceFilter::All => {
                    if let Some(first) = telemetry.media_sources.first() {
                        MediaSourceFilter::Specific(first.id.clone())
                    } else {
                        MediaSourceFilter::ActiveOnly
                    }
                }
                MediaSourceFilter::Specific(current_id) => {
                    if let Some(pos) = telemetry
                        .media_sources
                        .iter()
                        .position(|s| &s.id == current_id)
                    {
                        if pos + 1 < telemetry.media_sources.len() {
                            MediaSourceFilter::Specific(telemetry.media_sources[pos + 1].id.clone())
                        } else {
                            MediaSourceFilter::ActiveOnly
                        }
                    } else {
                        MediaSourceFilter::ActiveOnly
                    }
                }
            };
            app.source_filter = next_filter;
            app.media_scroll_index = 0;
            app.set_feedback(format!(
                "Filtre source : [{}]",
                app.source_filter.label(&telemetry.media_sources)
            ));
            true
        }
        KeyCode::Char('s') if app.view == ViewPage::MediaBrowser => {
            app.view = ViewPage::MediaSources;
            true
        }
        _ => false,
    }
}

/// Handles persistent deletion of media items from memory and disk cache.
fn handle_media_browser_deletion_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) -> bool {
    match code {
        KeyCode::Char('d') | KeyCode::Delete if app.view == ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            let to_delete: std::collections::HashSet<PathBuf> =
                if !app.selected_media_paths.is_empty() {
                    app.selected_media_paths.clone()
                } else if app.media_scroll_index < visible.len() {
                    std::iter::once(visible[app.media_scroll_index].path.clone()).collect()
                } else {
                    std::collections::HashSet::new()
                };

            if !to_delete.is_empty() {
                let count = to_delete.len();
                if let Some(shared) = shared_state {
                    if let Ok(mut state) = shared.write() {
                        state
                            .discovered_media
                            .retain(|m| !to_delete.contains(&m.path));
                        let cache_path = std::path::Path::new(DEFAULT_MEDIA_CACHE_PATH);
                        save_media_cache(cache_path, &state.discovered_media);
                    }
                }
                app.selected_media_paths.retain(|p| !to_delete.contains(p));
                let max_idx = visible.len().saturating_sub(count).saturating_sub(1);
                if app.media_scroll_index > max_idx {
                    app.media_scroll_index = max_idx;
                }
                app.set_feedback(format!(
                    "{count} média(s) supprimé(s) du catalogue persistant."
                ));
            }
            true
        }
        KeyCode::Char('d') | KeyCode::Delete if app.view == ViewPage::MediaInspection => {
            let visible = app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
            if app.media_scroll_index < visible.len() {
                let target_path = visible[app.media_scroll_index].path.clone();
                let stem = visible[app.media_scroll_index].stem.clone();
                if let Some(shared) = shared_state {
                    if let Ok(mut state) = shared.write() {
                        state.discovered_media.retain(|m| m.path != target_path);
                        let cache_path = std::path::Path::new(DEFAULT_MEDIA_CACHE_PATH);
                        save_media_cache(cache_path, &state.discovered_media);
                    }
                }
                app.selected_media_paths.remove(&target_path);
                app.view = ViewPage::MediaBrowser;
                let max_idx = visible.len().saturating_sub(2);
                if app.media_scroll_index > max_idx {
                    app.media_scroll_index = max_idx;
                }
                app.set_feedback(format!("Média '{stem}' supprimé du catalogue persistant."));
            }
            true
        }
        _ => false,
    }
}

/// Handles batch launching and terminating across media views.
fn handle_media_browser_batch_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
) -> bool {
    match code {
        KeyCode::Char('b') | KeyCode::Char('B') => {
            if app.selected_media_paths.is_empty() {
                let visible =
                    app.visible_media(&telemetry.discovered_media, &telemetry.media_sources);
                if app.media_scroll_index < visible.len() {
                    let item = &visible[app.media_scroll_index];
                    app.selected_media_paths.insert(item.path.clone());
                } else {
                    app.set_feedback(
                        "Aucun média sélectionné pour le traitement par lot. Cochez avec [Space]."
                            .to_string(),
                    );
                    return true;
                }
            }

            let paths: Vec<PathBuf> = app.selected_media_paths.iter().cloned().collect();
            let (_ok, msg) = launch_selected_batch(&paths);
            app.set_feedback(msg);
            app.selected_media_paths.clear();
            app.view = ViewPage::ActiveJobs;
            true
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
            let active_pids: Vec<u32> = telemetry.active_jobs.iter().map(|j| j.pid).collect();
            let (_ok, msg) = stop_entire_batch(code == KeyCode::Char('X'), &active_pids);
            app.set_feedback(msg);
            true
        }
        _ => false,
    }
}

/// Handles keypresses specific to MediaBrowser and MediaInspection views.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
/// - `shared_state`: Optional atomic reference to update shared state on deletions.
///
/// # Returns
/// None. Mutates batch selections, filters, sorts, cache persistence, or transitions view.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Supports space selection (single / inspected), 'a'/'A' select-all toggle, 'f'/'F' filter cycling,
/// 'o'/'O' sort cycling, 'S' (capital) source filter cycling, 's' (lowercase) jump to MediaSources,
/// 'b'/'B' batch launch, 'x'/'X' batch kill, 'd'/Delete (removal from memory and disk cache),
/// Enter/'l'/'i'/'I' (inspection toggle), and 'r'/'R' (jump to CompletedMedia).
fn handle_media_browser_view_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) {
    if handle_media_browser_selection_keys(code, app, telemetry) {
        return;
    }
    if handle_media_browser_filter_sort_keys(code, app, telemetry) {
        return;
    }
    if handle_media_browser_deletion_keys(code, app, telemetry, shared_state) {
        return;
    }
    if handle_media_browser_batch_keys(code, app, telemetry) {
        return;
    }

    match code {
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('i') | KeyCode::Char('I') => {
            if app.view == ViewPage::MediaBrowser {
                app.view = ViewPage::MediaInspection;
            } else if app.view == ViewPage::MediaInspection {
                app.view = ViewPage::MediaBrowser;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.view = ViewPage::CompletedMedia;
        }
        _ => {}
    }
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

/// Handles keypresses specific to the MediaSources configuration view.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
/// - `shared_state`: Optional atomic reference to update shared state on modifications.
///
/// # Returns
/// None. Mutates source activation, spawns modals, deletes sources, or initiates rescans.
///
/// # Errors
/// None. Deletion or persistence errors surface via `app.set_feedback`.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Intercepts Space (toggle source active flag), 'a'/'A' (spawn Add modal),
/// 'e'/'E' (spawn Edit modal), 'd'/'D' (delete source and evict cached media),
/// 'r'/'R' (trigger asynchronous directory rescanning), and 's'/'S' (jump back to ActiveJobs).
fn handle_media_sources_view_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) {
    match code {
        KeyCode::Char(' ') => {
            let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
            let mut sources = if !telemetry.media_sources.is_empty() {
                telemetry.media_sources.clone()
            } else {
                load_media_sources(sources_path)
            };
            if let Some(new_state) =
                toggle_media_source(sources_path, &mut sources, app.source_scroll_index)
            {
                if let Some(shared) = shared_state {
                    if let Ok(mut state) = shared.write() {
                        state.media_sources = sources.clone();
                    }
                }
                let name = sources
                    .get(app.source_scroll_index)
                    .map(|s| s.name.as_str())
                    .unwrap_or("Source");
                let state_str = if new_state {
                    "activée (incluse dans les futurs scans)"
                } else {
                    "désactivée (ignorée lors des scans)"
                };
                app.set_feedback(format!("Source '{name}' {state_str}."));
            }
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.source_modal = Some(SourceInputModalState::new_add());
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            let sources = if !telemetry.media_sources.is_empty() {
                telemetry.media_sources.clone()
            } else {
                load_media_sources(std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH))
            };
            if let Some(src) = sources.get(app.source_scroll_index) {
                app.source_modal = Some(SourceInputModalState::new_edit(
                    app.source_scroll_index,
                    src.name.clone(),
                    src.path.display().to_string(),
                    src.enabled,
                ));
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            let sources = if !telemetry.media_sources.is_empty() {
                telemetry.media_sources.clone()
            } else {
                load_media_sources(std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH))
            };
            if sources.len() <= 1 {
                app.set_feedback("Impossible de supprimer la seule source restante.".to_string());
                return;
            }
            if let Some(src) = sources.get(app.source_scroll_index) {
                app.source_modal = Some(SourceInputModalState::new_delete(
                    app.source_scroll_index,
                    src.name.clone(),
                    src.path.display().to_string(),
                    src.enabled,
                ));
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            crate::commands::top::sampler::IS_MEDIA_SCANNING
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(shared) = shared_state {
                if let Ok(mut state) = shared.write() {
                    state.is_scanning = true;
                }
            }
            trigger_media_sources_rescan();
            app.set_feedback("⟳ Rescan de toutes les sources lancé...".to_string());
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.view = ViewPage::ActiveJobs;
        }
        _ => {}
    }
}

/// Handles keypresses specific to the StatsHistory view.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
///
/// # Returns
/// None. Transitions active view.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Handles mnemonic jumps from stats history ('r'/'R' to CompletedMedia, 's'/'S' to MediaSources).
fn handle_stats_history_view_keys(code: KeyCode, app: &mut AppState) {
    match code {
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.view = ViewPage::CompletedMedia;
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.view = ViewPage::MediaSources;
        }
        _ => {}
    }
}
