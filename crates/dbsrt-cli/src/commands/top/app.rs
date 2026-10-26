//! Application lifecycle and event loop for dbsrt top.

use std::io::{self, stdout};
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

use crate::commands::top::constants::{FEEDBACK_DISPLAY_DURATION_SECS, KEY_EVENT_POLL_TIMEOUT_MS};
use crate::commands::top::model::{GpuTelemetry, SystemTelemetry, ViewPage};
use crate::commands::top::process_ctl::{kill_job, stop_job, toggle_pause_job};
use crate::commands::top::sampler::{run_telemetry_sampler_loop, start_gpu_sampler};
use crate::commands::top::ui::{render_ui, UiContext};

/// State of the TUI application.
#[derive(Default)]
pub struct AppState {
    pub view: ViewPage,
    pub selected_index: usize,
    pub feedback_message: Option<(String, Instant)>,
    pub should_quit: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
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

    let shared_state = Arc::new(RwLock::new(SystemTelemetry::default()));
    let gpu_state = Arc::new(RwLock::new(GpuTelemetry::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Spawn GPU monitor thread
    start_gpu_sampler(Arc::clone(&gpu_state), Arc::clone(&stop_flag));

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

    let feedback = app.current_feedback();

    terminal.draw(|f| {
        let ctx = UiContext {
            telemetry: &telemetry,
            view: app.view,
            selected_index: app.selected_index,
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
                    handle_key_event(key.code, app, &telemetry);
                }
            }
        }
    }

    Ok(())
}

/// Dispatches key press events to application actions.
pub fn handle_key_event(code: KeyCode, app: &mut AppState, telemetry: &SystemTelemetry) {
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.should_quit = true;
        }
        KeyCode::Esc => match app.view {
            ViewPage::ActiveJobs => {
                app.should_quit = true;
            }
            ViewPage::JobInspection | ViewPage::Help => {
                app.view = ViewPage::ActiveJobs;
            }
        },
        KeyCode::Backspace if app.view != ViewPage::ActiveJobs => {
            app.view = ViewPage::ActiveJobs;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.view == ViewPage::ActiveJobs && app.selected_index > 0 {
                app.selected_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.view == ViewPage::ActiveJobs
                && !telemetry.active_jobs.is_empty()
                && app.selected_index + 1 < telemetry.active_jobs.len()
            {
                app.selected_index += 1;
            }
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            if app.view == ViewPage::ActiveJobs && !telemetry.active_jobs.is_empty() {
                app.view = ViewPage::JobInspection;
            } else if app.view == ViewPage::JobInspection {
                app.view = ViewPage::ActiveJobs;
            }
        }
        KeyCode::Char('?') | KeyCode::Char('h') => {
            if app.view == ViewPage::Help {
                app.view = ViewPage::ActiveJobs;
            } else {
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
        _ => {}
    }
}
