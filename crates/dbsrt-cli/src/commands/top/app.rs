//! Application lifecycle and event loop for dbsrt top.

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
    calculate_overall_stats, filter_and_sort_completed, filter_and_sort_media_with_source,
    CompletedFilter, CompletedSort, DiscoveredMediaItem, GpuTelemetry, HelpTab, HistoricalRecord,
    MediaFilter, MediaSort, SourceInputModalState, SourceModalField, SourceModalMode,
    SystemTelemetry, ViewPage,
};
use crate::commands::top::process_ctl::{kill_job, stop_entire_batch, stop_job, toggle_pause_job};
use crate::commands::top::sampler::{
    add_media_source, compute_session_throughput, delete_media_source, launch_selected_batch,
    load_historical_records, load_media_cache, load_media_sources, preload_stem_size_cache,
    run_telemetry_sampler_loop, start_gpu_sampler, start_media_sources_scanner,
    toggle_media_source, trigger_media_sources_rescan, update_media_source,
};
use crate::commands::top::ui::{render_ui, UiContext};

/// State of the TUI application.
#[derive(Default)]
pub struct AppState {
    pub view: ViewPage,
    pub selected_index: usize,
    pub completed_scroll_index: usize,
    pub media_scroll_index: usize,
    pub source_scroll_index: usize,
    pub source_modal: Option<SourceInputModalState>,
    pub selected_media_paths: HashSet<PathBuf>,
    pub media_filter: MediaFilter,
    pub media_sort: MediaSort,
    pub selected_source_filter: Option<String>,
    pub completed_filter: CompletedFilter,
    pub completed_sort: CompletedSort,
    pub help_tab: HelpTab,
    pub help_scroll_index: usize,
    pub help_caller_view: ViewPage,
    pub feedback_message: Option<(String, Instant)>,
    pub should_quit: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible_media(&self, discovered: &[DiscoveredMediaItem]) -> Vec<DiscoveredMediaItem> {
        filter_and_sort_media_with_source(
            discovered,
            self.media_filter,
            self.media_sort,
            self.selected_source_filter.as_deref(),
        )
    }

    pub fn visible_completed(&self, records: &[HistoricalRecord]) -> Vec<HistoricalRecord> {
        filter_and_sort_completed(records, self.completed_filter, self.completed_sort)
    }

    pub fn set_feedback(&mut self, msg: String) {
        self.feedback_message = Some((msg, Instant::now()));
    }

    pub fn current_feedback(&self) -> Option<&str> {
        if let Some((ref msg, instant)) = self.feedback_message {
            if instant.elapsed().as_secs_f64() < FEEDBACK_DISPLAY_DURATION_SECS {
                return Some(msg.as_str());
            }
        }
        None
    }
}

/// Sets up terminal, panic hooks, and launches the TUI event loop.
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

    let media_cache_path = PathBuf::from(DEFAULT_MEDIA_CACHE_PATH);
    let cached_map = load_media_cache(&media_cache_path);
    let mut initial_media: Vec<_> = cached_map.into_values().collect();
    initial_media.sort_by_key(|a| a.stem.to_lowercase());

    let media_sources_path = PathBuf::from(DEFAULT_MEDIA_SOURCES_PATH);
    let initial_sources = load_media_sources(&media_sources_path);

    let initial_telemetry = SystemTelemetry {
        host_cpu_pct: host_cpu,
        ram_used_gib: ram_used,
        ram_total_gib: ram_total,
        history_records: initial_history,
        stats: initial_stats,
        session: initial_session,
        discovered_media: initial_media,
        media_sources: initial_sources,
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

/// Single step of the UI render cycle: clamps selected index and draws full-frame UI.
pub fn event_loop_step<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    shared_state: &Arc<RwLock<SystemTelemetry>>,
) -> Result<SystemTelemetry> {
    let telemetry = {
        let lock = shared_state.read().unwrap();
        lock.clone()
    };

    if telemetry.active_jobs.is_empty() {
        app.selected_index = 0;
    } else if app.selected_index >= telemetry.active_jobs.len() {
        app.selected_index = telemetry.active_jobs.len().saturating_sub(1);
    }

    let visible_completed = app.visible_completed(&telemetry.history_records);
    if visible_completed.is_empty() {
        app.completed_scroll_index = 0;
    } else if app.completed_scroll_index >= visible_completed.len() {
        app.completed_scroll_index = visible_completed.len().saturating_sub(1);
    }

    let visible_media = app.visible_media(&telemetry.discovered_media);
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
            selected_source_filter: app.selected_source_filter.as_deref(),
            completed_filter: app.completed_filter,
            completed_sort: app.completed_sort,
            help_tab: app.help_tab,
            help_scroll_index: app.help_scroll_index,
            feedback_msg: feedback,
        };
        render_ui(f, &ctx);
    })?;

    Ok(telemetry)
}

/// Main non-blocking event loop handling user input and screen rendering.
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

/// Dispatches key press events to application actions.
pub fn handle_key_event(code: KeyCode, app: &mut AppState, telemetry: &SystemTelemetry) {
    handle_key_event_with_state(code, app, telemetry, None);
}

/// Dispatches key press events to application actions with optional live shared_state synchronization.
pub fn handle_key_event_with_state(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    shared_state: Option<&Arc<RwLock<SystemTelemetry>>>,
) {
    // 1. Interactive modal keyboard handling (when modal is active)
    if let Some(ref mut modal) = app.source_modal {
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
            KeyCode::Backspace => match modal.active_field {
                SourceModalField::Name => {
                    modal.name_input.pop();
                }
                SourceModalField::Path => {
                    modal.path_input.pop();
                }
            },
            KeyCode::Char(c) => match modal.active_field {
                SourceModalField::Name => {
                    modal.name_input.push(c);
                }
                SourceModalField::Path => {
                    modal.path_input.push(c);
                }
            },
            KeyCode::Enter => {
                let path_str = modal.path_input.trim();
                if path_str.is_empty() {
                    app.set_feedback(
                        "Erreur : Le chemin du répertoire est obligatoire.".to_string(),
                    );
                    return;
                }
                let path = PathBuf::from(path_str);
                let name = modal.name_input.trim().to_string();
                let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
                let mut sources = if !telemetry.media_sources.is_empty() {
                    telemetry.media_sources.clone()
                } else {
                    load_media_sources(sources_path)
                };

                match modal.mode {
                    SourceModalMode::Add => {
                        match add_media_source(sources_path, &mut sources, name, path) {
                            Ok(src) => {
                                if let Some(shared) = shared_state {
                                    if let Ok(mut state) = shared.write() {
                                        state.media_sources = sources.clone();
                                    }
                                }
                                app.set_feedback(format!("✓ Source ajoutée : {}", src.name));
                                app.source_modal = None;
                            }
                            Err(err) => {
                                app.set_feedback(format!("Erreur : {err}"));
                            }
                        }
                    }
                    SourceModalMode::Edit(idx) => {
                        match update_media_source(sources_path, &mut sources, idx, name, path) {
                            Ok(src) => {
                                if let Some(shared) = shared_state {
                                    if let Ok(mut state) = shared.write() {
                                        state.media_sources = sources.clone();
                                    }
                                }
                                app.set_feedback(format!("✓ Source modifiée : {}", src.name));
                                app.source_modal = None;
                            }
                            Err(err) => {
                                app.set_feedback(format!("Erreur : {err}"));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        return;
    }

    // 2. Main top application key dispatching
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.should_quit = true;
        }
        KeyCode::Esc => match app.view {
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
        },
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
        }
        KeyCode::Char('0') | KeyCode::Char('g') | KeyCode::Char('G')
            if app.view == ViewPage::Help =>
        {
            app.help_tab = HelpTab::General;
            app.help_scroll_index = 0;
        }
        KeyCode::Char('1') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::ActiveJobs;
                app.help_scroll_index = 0;
            } else {
                app.view = ViewPage::ActiveJobs;
            }
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
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::Completed;
                app.help_scroll_index = 0;
            } else if app.view == ViewPage::MediaSources {
                trigger_media_sources_rescan();
                app.set_feedback("⟳ Rescan de toutes les sources lancé...".to_string());
            } else if app.view == ViewPage::CompletedMedia {
                app.view = ViewPage::ActiveJobs;
            } else {
                app.view = ViewPage::CompletedMedia;
            }
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
        }
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
        }
        KeyCode::Up | KeyCode::Char('k') => match app.view {
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
        },
        KeyCode::Down | KeyCode::Char('j') => match app.view {
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
                let visible = app.visible_media(&telemetry.discovered_media);
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
        },
        KeyCode::PageUp => match app.view {
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
        },
        KeyCode::PageDown => match app.view {
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
                let visible = app.visible_media(&telemetry.discovered_media);
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
        },
        KeyCode::Home => match app.view {
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
        },
        KeyCode::End => match app.view {
            ViewPage::ActiveJobs => {
                app.selected_index = telemetry.active_jobs.len().saturating_sub(1);
            }
            ViewPage::CompletedMedia => {
                let visible = app.visible_completed(&telemetry.history_records);
                app.completed_scroll_index = visible.len().saturating_sub(1);
            }
            ViewPage::MediaBrowser => {
                let visible = app.visible_media(&telemetry.discovered_media);
                app.media_scroll_index = visible.len().saturating_sub(1);
            }
            ViewPage::MediaSources => {
                app.source_scroll_index = telemetry.media_sources.len().saturating_sub(1);
            }
            ViewPage::Help => {
                app.help_scroll_index = 100;
            }
            _ => {}
        },
        KeyCode::Char(' ') if app.view == ViewPage::MediaSources => {
            let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
            let mut sources = if !telemetry.media_sources.is_empty() {
                telemetry.media_sources.clone()
            } else {
                load_media_sources(sources_path)
            };
            if let Some(new_state) =
                toggle_media_source(sources_path, &mut sources, app.source_scroll_index)
            {
                let active_source_ids: std::collections::HashSet<String> = sources
                    .iter()
                    .filter(|s| s.enabled)
                    .map(|s| s.id.clone())
                    .collect();

                if let Some(shared) = shared_state {
                    if let Ok(mut state) = shared.write() {
                        state.media_sources = sources.clone();
                        state.discovered_media.retain(|item| {
                            item.source_id
                                .as_ref()
                                .is_none_or(|id| active_source_ids.contains(id))
                        });
                    }
                }
                let name = sources
                    .get(app.source_scroll_index)
                    .map(|s| s.name.as_str())
                    .unwrap_or("Source");
                let state_str = if new_state {
                    "activée"
                } else {
                    "désactivée"
                };
                app.set_feedback(format!("Source '{name}' {state_str}."));
            }
        }
        KeyCode::Char('a') | KeyCode::Char('A') if app.view == ViewPage::MediaSources => {
            app.source_modal = Some(SourceInputModalState {
                mode: SourceModalMode::Add,
                name_input: String::new(),
                path_input: String::new(),
                active_field: SourceModalField::Path,
            });
        }
        KeyCode::Char('e') | KeyCode::Char('E') if app.view == ViewPage::MediaSources => {
            let sources = if !telemetry.media_sources.is_empty() {
                telemetry.media_sources.clone()
            } else {
                load_media_sources(std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH))
            };
            if let Some(src) = sources.get(app.source_scroll_index) {
                app.source_modal = Some(SourceInputModalState {
                    mode: SourceModalMode::Edit(app.source_scroll_index),
                    name_input: src.name.clone(),
                    path_input: src.path.display().to_string(),
                    active_field: SourceModalField::Path,
                });
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') if app.view == ViewPage::MediaSources => {
            let sources_path = std::path::Path::new(DEFAULT_MEDIA_SOURCES_PATH);
            let mut sources = if !telemetry.media_sources.is_empty() {
                telemetry.media_sources.clone()
            } else {
                load_media_sources(sources_path)
            };
            match delete_media_source(sources_path, &mut sources, app.source_scroll_index) {
                Ok(src) => {
                    let active_source_ids: std::collections::HashSet<String> = sources
                        .iter()
                        .filter(|s| s.enabled)
                        .map(|s| s.id.clone())
                        .collect();

                    if let Some(shared) = shared_state {
                        if let Ok(mut state) = shared.write() {
                            state.media_sources = sources.clone();
                            state.discovered_media.retain(|item| {
                                item.source_id
                                    .as_ref()
                                    .is_none_or(|id| active_source_ids.contains(id))
                            });
                        }
                    }
                    app.set_feedback(format!("✓ Source supprimée : {}", src.name));
                    if app.source_scroll_index >= sources.len() {
                        app.source_scroll_index = sources.len().saturating_sub(1);
                    }
                }
                Err(err) => {
                    app.set_feedback(format!("Erreur : {err}"));
                }
            }
        }
        KeyCode::Char(' ') if app.view == ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media);
            if let Some(item) = visible.get(app.media_scroll_index) {
                if app.selected_media_paths.contains(&item.path) {
                    app.selected_media_paths.remove(&item.path);
                } else {
                    app.selected_media_paths.insert(item.path.clone());
                }
            }
        }
        KeyCode::Char('a') | KeyCode::Char('A') if app.view == ViewPage::MediaBrowser => {
            let visible = app.visible_media(&telemetry.discovered_media);
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
        }
        KeyCode::Char('f') | KeyCode::Char('F') if app.view == ViewPage::MediaBrowser => {
            app.media_filter = app.media_filter.next();
            app.media_scroll_index = 0;
            app.set_feedback(format!("Filtre actif : {}", app.media_filter.label()));
        }
        KeyCode::Char('f') | KeyCode::Char('F') if app.view == ViewPage::CompletedMedia => {
            app.completed_filter = app.completed_filter.next();
            app.completed_scroll_index = 0;
            app.set_feedback(format!("Filtre Récents : {}", app.completed_filter.label()));
        }
        KeyCode::Char('S') if app.view == ViewPage::MediaBrowser => {
            let active_sources: Vec<&crate::commands::top::model::MediaSourceConfig> = telemetry
                .media_sources
                .iter()
                .filter(|s| s.enabled)
                .collect();
            if active_sources.is_empty() {
                app.selected_source_filter = None;
            } else {
                let current_idx = app
                    .selected_source_filter
                    .as_ref()
                    .and_then(|id| active_sources.iter().position(|s| &s.id == id));
                match current_idx {
                    None => {
                        app.selected_source_filter = Some(active_sources[0].id.clone());
                        app.set_feedback(format!("Filtre Source : {}", active_sources[0].name));
                    }
                    Some(idx) if idx + 1 < active_sources.len() => {
                        app.selected_source_filter = Some(active_sources[idx + 1].id.clone());
                        app.set_feedback(format!(
                            "Filtre Source : {}",
                            active_sources[idx + 1].name
                        ));
                    }
                    _ => {
                        app.selected_source_filter = None;
                        app.set_feedback("Filtre Source : TOUTES".to_string());
                    }
                }
            }
            app.media_scroll_index = 0;
        }
        KeyCode::Char('o') | KeyCode::Char('O') if app.view == ViewPage::MediaBrowser => {
            app.media_sort = app.media_sort.next();
            app.media_scroll_index = 0;
            app.set_feedback(format!("Tri actif : {}", app.media_sort.label()));
        }
        KeyCode::Char('o') | KeyCode::Char('O') if app.view == ViewPage::CompletedMedia => {
            app.completed_sort = app.completed_sort.next();
            app.completed_scroll_index = 0;
            app.set_feedback(format!("Tri Récents : {}", app.completed_sort.label()));
        }
        KeyCode::Char('b') | KeyCode::Char('B')
            if app.view == ViewPage::MediaBrowser || app.view == ViewPage::MediaInspection =>
        {
            if app.selected_media_paths.is_empty() {
                let visible = app.visible_media(&telemetry.discovered_media);
                if app.media_scroll_index < visible.len() {
                    let item = &visible[app.media_scroll_index];
                    app.selected_media_paths.insert(item.path.clone());
                } else {
                    app.set_feedback(
                        "Aucun média sélectionné pour le traitement par lot. Cochez avec [Space]."
                            .to_string(),
                    );
                    return;
                }
            }

            let paths: Vec<PathBuf> = app.selected_media_paths.iter().cloned().collect();
            let (_ok, msg) = launch_selected_batch(&paths);
            app.set_feedback(msg);
            app.selected_media_paths.clear();
            app.view = ViewPage::ActiveJobs;
        }
        KeyCode::Char('x') => {
            if app.view == ViewPage::ActiveJobs
                || app.view == ViewPage::JobInspection
                || app.view == ViewPage::MediaBrowser
            {
                let (_ok, msg) = stop_entire_batch(false);
                app.set_feedback(msg);
            }
        }
        KeyCode::Char('X') => {
            if app.view == ViewPage::ActiveJobs
                || app.view == ViewPage::JobInspection
                || app.view == ViewPage::MediaBrowser
            {
                let (_ok, msg) = stop_entire_batch(true);
                app.set_feedback(msg);
            }
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            if app.view == ViewPage::ActiveJobs && !telemetry.active_jobs.is_empty() {
                app.view = ViewPage::JobInspection;
            } else if app.view == ViewPage::JobInspection {
                app.view = ViewPage::ActiveJobs;
            } else if app.view == ViewPage::MediaBrowser {
                app.view = ViewPage::MediaInspection;
            } else if app.view == ViewPage::MediaInspection {
                app.view = ViewPage::MediaBrowser;
            } else if app.view == ViewPage::CompletedMedia {
                app.view = ViewPage::CompletedInspection;
            } else if app.view == ViewPage::CompletedInspection {
                app.view = ViewPage::CompletedMedia;
            }
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            if app.view == ViewPage::ActiveJobs && !telemetry.active_jobs.is_empty() {
                app.view = ViewPage::JobInspection;
            } else if app.view == ViewPage::MediaBrowser {
                app.view = ViewPage::MediaInspection;
            } else if app.view == ViewPage::MediaInspection {
                app.view = ViewPage::MediaBrowser;
            } else if app.view == ViewPage::CompletedMedia {
                app.view = ViewPage::CompletedInspection;
            } else if app.view == ViewPage::CompletedInspection {
                app.view = ViewPage::CompletedMedia;
            }
        }
        KeyCode::Char(' ') if app.view == ViewPage::MediaInspection => {
            let visible = app.visible_media(&telemetry.discovered_media);
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
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            if !telemetry.active_jobs.is_empty() && app.selected_index < telemetry.active_jobs.len()
            {
                let pid = telemetry.active_jobs[app.selected_index].pid;
                let (_ok, msg) = toggle_pause_job(pid);
                app.set_feedback(msg);
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            if app.view == ViewPage::Help {
                app.help_tab = HelpTab::MediaSources;
                app.help_scroll_index = 0;
            } else if app.view == ViewPage::ActiveJobs || app.view == ViewPage::JobInspection {
                if !telemetry.active_jobs.is_empty()
                    && app.selected_index < telemetry.active_jobs.len()
                {
                    let pid = telemetry.active_jobs[app.selected_index].pid;
                    let (_ok, msg) = stop_job(pid);
                    app.set_feedback(msg);
                }
            } else if app.view == ViewPage::MediaSources {
                app.view = ViewPage::ActiveJobs;
            } else {
                app.view = ViewPage::MediaSources;
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
        _ => {}
    }
}
