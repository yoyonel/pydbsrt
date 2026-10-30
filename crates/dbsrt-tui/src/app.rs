//! Platform-agnostic `dbsrt top` interaction state machine and keyboard dispatch.
//!
//! Host side effects (signals, persistence, rescans) are delegated to a [`TopActions`]
//! implementation.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::constants::{FEEDBACK_DISPLAY_DURATION_SECS, PAGE_SCROLL_STEP};
use crate::model::{
    filter_active_jobs_with_search, filter_and_sort_completed_with_search,
    filter_and_sort_media_with_source_and_search, ActiveJobTelemetry, CompletedFilter,
    CompletedSort, DiscoveredMediaItem, HelpTab, HistoricalRecord, MediaFilter, MediaSort,
    MediaSourceConfig, MediaSourceFilter, SourceInputModalState, SourceModalField, SourceModalMode,
    SystemTelemetry, ViewPage,
};
use crate::ui::UiContext;

/// Frontend-neutral key code (subset of crossterm codes used by `dbsrt top`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    /// Any key `dbsrt top` does not bind.
    Null,
}

/// Seconds since Unix epoch as `f64`.
pub fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Host side effects triggered by keys. Each method returns the feedback toast to display.
///
/// The native monitor signals processes and persists files; the web client is read-only and
/// answers every call with an explanatory message.
pub trait TopActions {
    /// SIGSTOP/SIGCONT toggle on a job (`[p]`).
    fn toggle_pause_job(&mut self, pid: u32) -> String;
    /// SIGTERM on a job (`[c]`).
    fn stop_job(&mut self, pid: u32) -> String;
    /// SIGKILL on a job (`[K]` in inspection).
    fn kill_job(&mut self, pid: u32) -> String;
    /// Stops the whole batch (`[x]` graceful, `[X]` forced).
    fn stop_entire_batch(&mut self, force: bool, pids: &[u32]) -> String;
    /// Launches a batch (`[b]`). `Ok` clears the selection and returns to Active Jobs.
    fn launch_selected_batch(&mut self, paths: &[PathBuf]) -> Result<String, String>;
    /// Removes media from the persistent catalog cache (`[d]`).
    fn remove_discovered_media(&mut self, paths: &HashSet<PathBuf>) -> Result<(), String>;
    /// Current media sources, falling back to the host's persisted list.
    fn media_sources(&mut self, telemetry: &SystemTelemetry) -> Vec<MediaSourceConfig> {
        telemetry.media_sources.clone()
    }
    /// Toggles the enabled flag of a source (`[Space]`); `None` when the index is invalid.
    fn toggle_media_source(&mut self, telemetry: &SystemTelemetry, index: usize) -> Option<String>;
    /// Validates and persists the add/edit source modal; `(saved, feedback)`.
    fn save_source_modal(
        &mut self,
        modal: &mut SourceInputModalState,
        telemetry: &SystemTelemetry,
    ) -> (bool, Option<String>);
    /// Deletes a source and evicts its cached media (`[d]` confirmed).
    fn delete_media_source(
        &mut self,
        app: &mut AppState,
        telemetry: &SystemTelemetry,
        index: usize,
    );
    /// Triggers an asynchronous rescan of all sources (`[r]`).
    fn rescan_media_sources(&mut self) -> String;
    /// Reloads remote streaming targets (`[r]` in Zero-NVMe mode).
    fn reload_remote_targets(&mut self, telemetry: &SystemTelemetry) -> String;
}

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
    /// Scroll line offset in System diagnostics view (`[6/y]`).
    pub system_scroll_index: usize,
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
    /// Ephemeral feedback toast message with timestamp: `(message, epoch_secs)`.
    pub feedback_message: Option<(String, f64)>,
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
            ViewPage::System => self.system_scroll_index = 0,
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
    /// Stores the message paired with [`now_secs_f64`]. Automatically expires
    /// after [`FEEDBACK_DISPLAY_DURATION_SECS`].
    pub fn set_feedback(&mut self, msg: String) {
        self.feedback_message = Some((msg, now_secs_f64()));
    }

    /// Returns the active feedback message if it has not expired.
    ///
    /// # Returns
    /// `Some(&str)` if within the TTL window, `None` if absent or expired.
    pub fn current_feedback(&self) -> Option<&str> {
        if let Some((ref msg, set_at)) = self.feedback_message {
            if now_secs_f64() - set_at < FEEDBACK_DISPLAY_DURATION_SECS {
                return Some(msg.as_str());
            }
        }
        None
    }

    /// Clamps every cursor to the bounds of its filtered list (run once per frame).
    pub fn clamp_indices(&mut self, telemetry: &SystemTelemetry) {
        let jobs_len = self.visible_jobs(&telemetry.active_jobs).len();
        let completed_len = self.visible_completed(&telemetry.history_records).len();
        let media_len = self
            .visible_media(&telemetry.discovered_media, &telemetry.media_sources)
            .len();
        let sources_len = telemetry.media_sources.len();
        for (index, len) in [
            (&mut self.selected_index, jobs_len),
            (&mut self.completed_scroll_index, completed_len),
            (&mut self.media_scroll_index, media_len),
            (&mut self.source_scroll_index, sources_len),
        ] {
            *index = (*index).min(len.saturating_sub(1));
        }
    }

    /// Builds the render context for [`crate::ui::render_ui`].
    pub fn ui_context<'a>(
        &'a self,
        telemetry: &'a SystemTelemetry,
        feedback_msg: Option<&'a str>,
    ) -> UiContext<'a> {
        UiContext {
            telemetry,
            view: self.view,
            selected_index: self.selected_index,
            completed_scroll_index: self.completed_scroll_index,
            media_scroll_index: self.media_scroll_index,
            source_scroll_index: self.source_scroll_index,
            source_modal: self.source_modal.as_ref(),
            selected_media_paths: &self.selected_media_paths,
            media_filter: self.media_filter,
            media_sort: self.media_sort,
            source_filter: &self.source_filter,
            completed_filter: self.completed_filter,
            completed_sort: self.completed_sort,
            search_query: self.current_search_query(),
            search_active: self.search_active,
            help_tab: self.help_tab,
            help_scroll_index: self.help_scroll_index,
            feedback_msg,
        }
    }
}

/// Dispatches key press events to application actions.
///
/// # Arguments
/// - `code`: The frontend-neutral [`KeyCode`] received.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
/// - `actions`: Host side-effect executor (process control, persistence).
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
pub fn handle_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    actions: &mut dyn TopActions,
) {
    if handle_modal_key_event(code, app, telemetry, actions) {
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
    handle_view_specific_key_event(code, app, telemetry, actions);
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
/// * `actions` - Host side-effect executor (process control, persistence).
///
/// # Returns
/// Always returns `true` to signal key event consumption.
fn handle_source_input_modal_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    actions: &mut dyn TopActions,
) -> bool {
    if code == KeyCode::Enter {
        let (success, feedback) = {
            let Some(ref mut modal) = app.source_modal else {
                return false;
            };
            actions.save_source_modal(modal, telemetry)
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
/// * `actions` - Host side-effect executor (process control, persistence).
///
/// # Returns
/// Always returns `true` to indicate modal event consumption.
fn handle_delete_confirm_modal_key_event(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    actions: &mut dyn TopActions,
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
            actions.delete_media_source(app, telemetry, idx);
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
/// * `actions` - Host side-effect executor (process control, persistence).
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
    actions: &mut dyn TopActions,
) -> bool {
    let Some(ref modal) = app.source_modal else {
        return false;
    };

    if matches!(modal.mode, SourceModalMode::DeleteConfirm(_)) {
        handle_delete_confirm_modal_key_event(code, app, telemetry, actions)
    } else {
        handle_source_input_modal_key_event(code, app, telemetry, actions)
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
        KeyCode::Char('?') => handle_help_key_toggle(app),
        KeyCode::Esc => handle_esc_key_navigation(app),
        KeyCode::Backspace if app.view != ViewPage::ActiveJobs => {
            handle_backspace_navigation(app);
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

/// Toggles between the Help screen and previous caller view.
fn handle_help_key_toggle(app: &mut AppState) -> bool {
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

/// Handles Escape key behavior: clearing search query or retreating through view hierarchy.
fn handle_esc_key_navigation(app: &mut AppState) -> bool {
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
            | ViewPage::MediaSources
            | ViewPage::System => {
                app.view = ViewPage::ActiveJobs;
            }
        }
    }
    true
}

/// Handles Backspace key retreat navigation from sub-views and modal screens.
fn handle_backspace_navigation(app: &mut AppState) {
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
            cycle_view_forward(app);
            true
        }
        KeyCode::BackTab | KeyCode::Left => {
            cycle_view_backward(app);
            true
        }
        _ => false,
    }
}

/// Advances to the next view or help tab.
fn cycle_view_forward(app: &mut AppState) {
    if app.view == ViewPage::Help {
        app.help_tab = app.help_tab.next();
        app.help_scroll_index = 0;
    } else {
        app.view = match app.view {
            ViewPage::ActiveJobs => ViewPage::StatsHistory,
            ViewPage::StatsHistory => ViewPage::CompletedMedia,
            ViewPage::CompletedMedia => ViewPage::MediaBrowser,
            ViewPage::MediaBrowser => ViewPage::MediaSources,
            ViewPage::MediaSources => ViewPage::System,
            ViewPage::System
            | ViewPage::JobInspection
            | ViewPage::MediaInspection
            | ViewPage::CompletedInspection
            | ViewPage::Help => ViewPage::ActiveJobs,
        };
    }
}

/// Retreats to the previous view or help tab.
fn cycle_view_backward(app: &mut AppState) {
    if app.view == ViewPage::Help {
        app.help_tab = app.help_tab.prev();
        app.help_scroll_index = 0;
    } else {
        app.view = match app.view {
            ViewPage::ActiveJobs => ViewPage::System,
            ViewPage::System => ViewPage::MediaSources,
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

/// Resolves destination ViewPage and HelpTab for numeric and mnemonic direct shortcuts.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `current_view`: Active [`ViewPage`] used for contextual discrimination.
///
/// # Returns
/// `Some((target_view, help_tab))` if the key matches a direct navigation shortcut, `None` otherwise.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Direct routing: '1' -> ActiveJobs, '2'/'h' -> StatsHistory, '3'/'r' -> CompletedMedia
/// (unless in MediaSources where 'r' rescans), '4'/'m' -> MediaBrowser, '5'/'s' -> MediaSources
/// (unless in MediaBrowser where uppercase 'S' cycles the source filter).
fn resolve_shortcut_target(code: KeyCode, current_view: ViewPage) -> Option<(ViewPage, HelpTab)> {
    match code {
        KeyCode::Char('1') => Some((ViewPage::ActiveJobs, HelpTab::ActiveJobs)),
        KeyCode::Char('2') | KeyCode::Char('h') | KeyCode::Char('H') => {
            Some((ViewPage::StatsHistory, HelpTab::StatsHistory))
        }
        KeyCode::Char('3') | KeyCode::Char('r') | KeyCode::Char('R')
            if current_view != ViewPage::MediaSources
                || (code != KeyCode::Char('r') && code != KeyCode::Char('R')) =>
        {
            Some((ViewPage::CompletedMedia, HelpTab::Completed))
        }
        KeyCode::Char('4') | KeyCode::Char('m') | KeyCode::Char('M') => {
            Some((ViewPage::MediaBrowser, HelpTab::MediaBrowser))
        }
        KeyCode::Char('5') | KeyCode::Char('s') | KeyCode::Char('S')
            if current_view != ViewPage::MediaBrowser || code != KeyCode::Char('S') =>
        {
            Some((ViewPage::MediaSources, HelpTab::MediaSources))
        }
        KeyCode::Char('6') | KeyCode::Char('y') | KeyCode::Char('Y') => {
            Some((ViewPage::System, HelpTab::System))
        }
        _ => None,
    }
}

/// Applies direct view switching or double-tap toggle logic.
///
/// # Arguments
/// - `target_view`: Desired destination [`ViewPage`].
/// - `help_tab`: Corresponding [`HelpTab`] when navigating inside the help overlay.
/// - `app`: Mutable reference to application state.
///
/// # Returns
/// None. Mutates application view or help tab in place.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// If current view is Help, switches the help tab. If current view already equals target view
/// (and is not ActiveJobs), toggles back to ActiveJobs. Otherwise switches to target view.
fn switch_or_toggle_view(target_view: ViewPage, help_tab: HelpTab, app: &mut AppState) {
    if app.view == ViewPage::Help {
        app.help_tab = help_tab;
        app.help_scroll_index = 0;
    } else if app.view == target_view && target_view != ViewPage::ActiveJobs {
        app.view = ViewPage::ActiveJobs;
    } else {
        app.view = target_view;
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
/// Delegates resolution to [`resolve_shortcut_target`] and state mutation to [`switch_or_toggle_view`].
fn handle_direct_view_shortcuts(code: KeyCode, app: &mut AppState) -> bool {
    if let Some((target_view, help_tab)) = resolve_shortcut_target(code, app.view) {
        switch_or_toggle_view(target_view, help_tab, app);
        true
    } else {
        false
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
        ViewPage::System if app.system_scroll_index > 0 => {
            app.system_scroll_index -= 1;
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
        ViewPage::System => {
            app.system_scroll_index += 1;
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
        ViewPage::System => {
            app.system_scroll_index = app.system_scroll_index.saturating_sub(PAGE_SCROLL_STEP);
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
        ViewPage::System => {
            app.system_scroll_index += PAGE_SCROLL_STEP;
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
        ViewPage::System => {
            app.system_scroll_index = 0;
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
        ViewPage::System => {
            app.system_scroll_index = 20;
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
/// - `actions`: Host side-effect executor (process control, persistence).
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
    actions: &mut dyn TopActions,
) {
    match app.view {
        ViewPage::Help => handle_help_view_keys(code, app),
        ViewPage::ActiveJobs | ViewPage::JobInspection => {
            handle_active_jobs_view_keys(code, app, telemetry, actions);
        }
        ViewPage::CompletedMedia | ViewPage::CompletedInspection => {
            handle_completed_media_view_keys(code, app);
        }
        ViewPage::MediaBrowser | ViewPage::MediaInspection => {
            handle_media_browser_view_keys(code, app, telemetry, actions);
        }
        ViewPage::MediaSources => {
            if telemetry.is_remote_streaming {
                handle_remote_streaming_view_keys(code, app, telemetry, actions);
            } else {
                handle_media_sources_view_keys(code, app, telemetry, actions);
            }
        }
        ViewPage::StatsHistory => {
            handle_stats_history_view_keys(code, app);
        }
        ViewPage::System => {}
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
fn handle_active_jobs_view_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    actions: &mut dyn TopActions,
) {
    match code {
        KeyCode::Char('p') | KeyCode::Char('P') => {
            if !telemetry.active_jobs.is_empty() && app.selected_index < telemetry.active_jobs.len()
            {
                let pid = telemetry.active_jobs[app.selected_index].pid;
                let msg = actions.toggle_pause_job(pid);
                app.set_feedback(msg);
            }
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if !telemetry.active_jobs.is_empty() && app.selected_index < telemetry.active_jobs.len()
            {
                let pid = telemetry.active_jobs[app.selected_index].pid;
                let msg = actions.stop_job(pid);
                app.set_feedback(msg);
            }
        }
        KeyCode::Char('K')
            if app.view == ViewPage::JobInspection
                && !telemetry.active_jobs.is_empty()
                && app.selected_index < telemetry.active_jobs.len() =>
        {
            let pid = telemetry.active_jobs[app.selected_index].pid;
            let msg = actions.kill_job(pid);
            app.set_feedback(msg);
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
            let active_pids: Vec<u32> = telemetry.active_jobs.iter().map(|j| j.pid).collect();
            let msg = actions.stop_entire_batch(code == KeyCode::Char('X'), &active_pids);
            app.set_feedback(msg);
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('i') | KeyCode::Char('I') => {
            if app.view == ViewPage::ActiveJobs && !telemetry.active_jobs.is_empty() {
                app.view = ViewPage::JobInspection;
            } else if app.view == ViewPage::JobInspection {
                app.view = ViewPage::ActiveJobs;
            }
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
        _ => false,
    }
}

/// Handles persistent deletion of media items from memory and disk cache.
fn handle_media_browser_deletion_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    actions: &mut dyn TopActions,
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
                if let Err(msg) = actions.remove_discovered_media(&to_delete) {
                    app.set_feedback(msg);
                    return true;
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
                if let Err(msg) =
                    actions.remove_discovered_media(&std::iter::once(target_path.clone()).collect())
                {
                    app.set_feedback(msg);
                    return true;
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
    actions: &mut dyn TopActions,
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
            match actions.launch_selected_batch(&paths) {
                Ok(msg) => {
                    app.set_feedback(msg);
                    app.selected_media_paths.clear();
                    app.view = ViewPage::ActiveJobs;
                }
                Err(msg) => app.set_feedback(msg),
            }
            true
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
            let active_pids: Vec<u32> = telemetry.active_jobs.iter().map(|j| j.pid).collect();
            let msg = actions.stop_entire_batch(code == KeyCode::Char('X'), &active_pids);
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
/// - `actions`: Host side-effect executor (process control, persistence).
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
    actions: &mut dyn TopActions,
) {
    if handle_media_browser_selection_keys(code, app, telemetry) {
        return;
    }
    if handle_media_browser_filter_sort_keys(code, app, telemetry) {
        return;
    }
    if handle_media_browser_deletion_keys(code, app, telemetry, actions) {
        return;
    }
    if handle_media_browser_batch_keys(code, app, telemetry, actions) {
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

/// Handles keypresses specific to the MediaSources configuration view.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Immutable reference to active telemetry snapshot.
/// - `actions`: Host side-effect executor (process control, persistence).
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
    actions: &mut dyn TopActions,
) {
    match code {
        KeyCode::Char(' ') => {
            if let Some(msg) = actions.toggle_media_source(telemetry, app.source_scroll_index) {
                app.set_feedback(msg);
            }
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.source_modal = Some(SourceInputModalState::new_add());
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            let sources = actions.media_sources(telemetry);
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
            let sources = actions.media_sources(telemetry);
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
            let msg = actions.rescan_media_sources();
            app.set_feedback(msg);
        }
        _ => {}
    }
}

/// Handles keypresses specific to the Remote Streaming / Zero-NVMe sources view.
///
/// # Arguments
/// - `code`: Crossterm [`KeyCode`] received from the terminal event loop.
/// - `app`: Mutable reference to application state.
/// - `telemetry`: Active system telemetry snapshot.
/// - `actions`: Host side-effect executor (process control, persistence).
///
/// # Returns
/// None. Mutates feedback message or reloads remote batch targets.
///
/// # Errors
/// None.
///
/// # Panics
/// Guaranteed panic-free.
///
/// # Data Flow & Invariants
/// Locks out physical disk actions (' ', 'a', 'e', 'd') on Zero-NVMe nodes,
/// routing 'r' to reload the remote targets registry (`selected_batch_targets.json`).
fn handle_remote_streaming_view_keys(
    code: KeyCode,
    app: &mut AppState,
    telemetry: &SystemTelemetry,
    actions: &mut dyn TopActions,
) {
    match code {
        KeyCode::Char(' ')
        | KeyCode::Char('a')
        | KeyCode::Char('A')
        | KeyCode::Char('e')
        | KeyCode::Char('E')
        | KeyCode::Char('d')
        | KeyCode::Char('D') => {
            app.set_feedback(
                "Mode Zéro-NVMe actif : la gestion des répertoires physiques s'effectue sur la station hôte locale.".to_string(),
            );
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            let msg = actions.reload_remote_targets(telemetry);
            app.set_feedback(msg);
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
