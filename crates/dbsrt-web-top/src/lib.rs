//! Host-testable core of the web-top dashboard: read-only actions, key, tap and swipe mapping,
//! polling.
//!
//! The browser glue (DOM backend, Fetch API, `sessionStorage`) lives in the `browser` module of
//! the wasm32 binary; everything here compiles and runs on the host so `cargo test` covers it.

use std::collections::HashSet;
use std::path::PathBuf;

use dbsrt_tui::app::{handle_key_event, AppState, KeyCode, TopActions};
use dbsrt_tui::model::{SourceInputModalState, SystemTelemetry};
use dbsrt_tui::ui::hit_test;
use ratatui::layout::Rect;
use ratzilla::event::{KeyCode as WebKeyCode, KeyEvent};

/// Polling frequency for live system telemetry in milliseconds.
pub const TELEMETRY_POLL_INTERVAL_MS: u32 = 2000;

/// Upper bound of the exponential back-off applied after failed telemetry polls.
pub const TELEMETRY_MAX_BACKOFF_MS: u32 = 60_000;

/// API path serving the full native `dbsrt top --json` snapshot (ISO with the terminal monitor).
pub const TELEMETRY_ENDPOINT_PATH: &str = "/v1/monitoring/top";

/// Feedback returned for every host side effect: the browser client never mutates the server.
pub const READ_ONLY_MSG: &str =
    "Lecture seule (web) : action réservée au `dbsrt top` natif sur l'hôte.";

/// Browser client state: the shared native state machine plus the latest snapshot.
#[derive(Default)]
pub struct WebAppState {
    pub app: AppState,
    pub telemetry: SystemTelemetry,
    /// Sticky connection error (the native monitor has no equivalent, so it only shows on failure).
    pub connection_error: Option<String>,
    /// Screen area of the last drawn frame, so taps resolve against what the user sees.
    pub frame_area: Rect,
    /// Vertical swipe in progress, if any.
    pub drag: Option<Drag>,
}

/// Vertical pointer drag, in CSS pixels, and the grid rows it already scrolled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    start_y: f64,
    row_px: f64,
    rows: i32,
}

impl WebAppState {
    /// Feeds one key to the shared native state machine.
    pub fn press(&mut self, code: KeyCode) {
        handle_key_event(code, &mut self.app, &self.telemetry, &mut ReadOnlyActions);
        // There is no process to quit in a browser tab.
        self.app.should_quit = false;
    }

    /// Feeds a tap on grid cell (`column`, `row`) as the key drawn there; `false` when none.
    pub fn tap(&mut self, column: u16, row: u16) -> bool {
        let ctx = self.app.ui_context(&self.telemetry, None);
        match hit_test(self.frame_area, &ctx, column, row) {
            Some(code) => {
                self.press(code);
                true
            }
            None => false,
        }
    }

    /// Starts a swipe at `y` over grid rows `row_px` tall.
    pub fn drag_start(&mut self, y: f64, row_px: f64) {
        self.drag = (row_px > 0.0).then_some(Drag {
            start_y: y,
            row_px,
            rows: 0,
        });
    }

    /// Scrolls one line per grid row travelled since the last move: the content follows the
    /// finger, so moving up presses `Down` like a phone list.
    pub fn drag_move(&mut self, y: f64) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let rows = ((drag.start_y - y) / drag.row_px).trunc() as i32;
        let code = if rows > drag.rows {
            KeyCode::Down
        } else {
            KeyCode::Up
        };
        let steps = rows.abs_diff(drag.rows);
        drag.rows = rows;
        for _ in 0..steps {
            self.press(code);
        }
    }

    /// Ends the swipe; `true` when it scrolled, so the release is not also a tap.
    pub fn drag_end(&mut self) -> bool {
        self.drag.take().is_some_and(|drag| drag.rows != 0)
    }
}

/// Read-only host: every side effect answers with [`READ_ONLY_MSG`].
pub struct ReadOnlyActions;

impl TopActions for ReadOnlyActions {
    fn toggle_pause_job(&mut self, _pid: u32) -> String {
        READ_ONLY_MSG.to_string()
    }

    fn stop_job(&mut self, _pid: u32) -> String {
        READ_ONLY_MSG.to_string()
    }

    fn kill_job(&mut self, _pid: u32) -> String {
        READ_ONLY_MSG.to_string()
    }

    fn stop_entire_batch(&mut self, _force: bool, _pids: &[u32]) -> String {
        READ_ONLY_MSG.to_string()
    }

    fn launch_selected_batch(&mut self, _paths: &[PathBuf]) -> Result<String, String> {
        Err(READ_ONLY_MSG.to_string())
    }

    fn remove_discovered_media(&mut self, _paths: &HashSet<PathBuf>) -> Result<(), String> {
        Err(READ_ONLY_MSG.to_string())
    }

    fn toggle_media_source(
        &mut self,
        _telemetry: &SystemTelemetry,
        _index: usize,
    ) -> Option<String> {
        Some(READ_ONLY_MSG.to_string())
    }

    fn save_source_modal(
        &mut self,
        modal: &mut SourceInputModalState,
        _telemetry: &SystemTelemetry,
    ) -> (bool, Option<String>) {
        modal.error_msg = Some(READ_ONLY_MSG.to_string());
        (false, None)
    }

    fn delete_media_source(
        &mut self,
        app: &mut AppState,
        _telemetry: &SystemTelemetry,
        _index: usize,
    ) {
        app.set_feedback(READ_ONLY_MSG.to_string());
    }

    fn rescan_media_sources(&mut self) -> String {
        READ_ONLY_MSG.to_string()
    }

    fn reload_remote_targets(&mut self, _telemetry: &SystemTelemetry) -> String {
        READ_ONLY_MSG.to_string()
    }
}

/// Maps a browser key event onto the shared key code; `None` for keys left to the browser.
///
/// # Data Flow & Invariants
/// Ctrl/Alt chords stay with the browser (reload, copy...). Shift+Tab becomes `BackTab`
/// like crossterm reports it.
pub fn to_tui_key(event: &KeyEvent) -> Option<KeyCode> {
    if event.ctrl || event.alt {
        return None;
    }
    Some(match event.code {
        WebKeyCode::Char(c) => KeyCode::Char(c),
        WebKeyCode::Enter => KeyCode::Enter,
        WebKeyCode::Esc => KeyCode::Esc,
        WebKeyCode::Backspace => KeyCode::Backspace,
        WebKeyCode::Delete => KeyCode::Delete,
        WebKeyCode::Tab if event.shift => KeyCode::BackTab,
        WebKeyCode::Tab => KeyCode::Tab,
        WebKeyCode::Up => KeyCode::Up,
        WebKeyCode::Down => KeyCode::Down,
        WebKeyCode::Left => KeyCode::Left,
        WebKeyCode::Right => KeyCode::Right,
        WebKeyCode::PageUp => KeyCode::PageUp,
        WebKeyCode::PageDown => KeyCode::PageDown,
        WebKeyCode::Home => KeyCode::Home,
        WebKeyCode::End => KeyCode::End,
        WebKeyCode::F(_) | WebKeyCode::Unidentified => return None,
    })
}

/// Maps a pointer `offset` inside a box of `extent` pixels split into `cells` equal cells.
///
/// # Returns
/// The cell index, or `None` outside the box.
pub fn pixel_to_cell(offset: f64, extent: f64, cells: u16) -> Option<u16> {
    if cells == 0 || !(0.0..extent).contains(&offset) {
        return None;
    }
    Some(((offset / extent * f64::from(cells)) as u16).min(cells - 1))
}

/// Formats the operator banner for a non-2xx telemetry response.
///
/// # Data Flow & Invariants
/// Known o2switch gateway statuses (401 token, 421 SNI, 429 rate limit) get an actionable hint.
pub fn http_error_message(status: u16, url: &str) -> String {
    match status {
        401 => "⚠️ 401 Non autorisé : Token absent ou invalide".to_string(),
        421 => "⚠️ 421 SNI Mismatch : Rechargez (Ctrl+Shift+R)".to_string(),
        429 => "⚠️ 429 Trop de requêtes : Rate limit temporaire".to_string(),
        code => format!("⚠️ Erreur HTTP {code} ({url})"),
    }
}

/// Computes the delay before the next telemetry poll.
///
/// # Data Flow & Invariants
/// Resets to `TELEMETRY_POLL_INTERVAL_MS` on success; doubles up to `TELEMETRY_MAX_BACKOFF_MS`
/// on failure so o2switch Tiger Protect / PowerBoost (429, 421) stops flagging the client.
pub fn next_poll_delay_ms(previous_ms: u32, success: bool) -> u32 {
    if success {
        TELEMETRY_POLL_INTERVAL_MS
    } else {
        previous_ms.saturating_mul(2).min(TELEMETRY_MAX_BACKOFF_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbsrt_tui::model::ViewPage;

    fn key(code: WebKeyCode, ctrl: bool, shift: bool) -> KeyEvent {
        KeyEvent {
            code,
            ctrl,
            alt: false,
            shift,
        }
    }

    #[test]
    fn test_to_tui_key_maps_backtab_and_leaves_chords_to_browser() {
        assert_eq!(
            to_tui_key(&key(WebKeyCode::Tab, false, true)),
            Some(KeyCode::BackTab)
        );
        assert_eq!(
            to_tui_key(&key(WebKeyCode::Char('3'), false, false)),
            Some(KeyCode::Char('3'))
        );
        assert_eq!(to_tui_key(&key(WebKeyCode::Char('r'), true, false)), None);
        assert_eq!(to_tui_key(&key(WebKeyCode::F(5), false, false)), None);
    }

    #[test]
    fn test_to_tui_key_maps_every_navigation_key() {
        let cases = [
            (WebKeyCode::Enter, KeyCode::Enter),
            (WebKeyCode::Esc, KeyCode::Esc),
            (WebKeyCode::Backspace, KeyCode::Backspace),
            (WebKeyCode::Delete, KeyCode::Delete),
            (WebKeyCode::Tab, KeyCode::Tab),
            (WebKeyCode::Up, KeyCode::Up),
            (WebKeyCode::Down, KeyCode::Down),
            (WebKeyCode::Left, KeyCode::Left),
            (WebKeyCode::Right, KeyCode::Right),
            (WebKeyCode::PageUp, KeyCode::PageUp),
            (WebKeyCode::PageDown, KeyCode::PageDown),
            (WebKeyCode::Home, KeyCode::Home),
            (WebKeyCode::End, KeyCode::End),
        ];
        for (web, tui) in cases {
            assert_eq!(to_tui_key(&key(web, false, false)), Some(tui));
        }
        assert_eq!(
            to_tui_key(&key(WebKeyCode::Unidentified, false, false)),
            None
        );
    }

    #[test]
    fn test_read_only_actions_keep_state_and_explain() {
        let mut state = WebAppState::default();
        state.app.view = dbsrt_tui::model::ViewPage::MediaSources;
        handle_key_event(
            KeyCode::Char('r'),
            &mut state.app,
            &state.telemetry,
            &mut ReadOnlyActions,
        );
        assert_eq!(state.app.current_feedback(), Some(READ_ONLY_MSG));
        handle_key_event(
            KeyCode::Char('3'),
            &mut state.app,
            &state.telemetry,
            &mut ReadOnlyActions,
        );
        assert_eq!(state.app.view, dbsrt_tui::model::ViewPage::CompletedMedia);
    }

    #[test]
    fn test_read_only_actions_refuse_every_side_effect() {
        let telemetry = SystemTelemetry::default();
        let mut actions = ReadOnlyActions;
        assert_eq!(actions.toggle_pause_job(1), READ_ONLY_MSG);
        assert_eq!(actions.stop_job(1), READ_ONLY_MSG);
        assert_eq!(actions.kill_job(1), READ_ONLY_MSG);
        assert_eq!(actions.stop_entire_batch(true, &[1]), READ_ONLY_MSG);
        assert_eq!(
            actions.launch_selected_batch(&[]),
            Err(READ_ONLY_MSG.to_string())
        );
        assert_eq!(
            actions.remove_discovered_media(&HashSet::new()),
            Err(READ_ONLY_MSG.to_string())
        );
        assert_eq!(
            actions.toggle_media_source(&telemetry, 0),
            Some(READ_ONLY_MSG.to_string())
        );
        assert_eq!(actions.rescan_media_sources(), READ_ONLY_MSG);
        assert_eq!(actions.reload_remote_targets(&telemetry), READ_ONLY_MSG);

        let mut modal = SourceInputModalState::new_add();
        assert_eq!(
            actions.save_source_modal(&mut modal, &telemetry),
            (false, None)
        );
        assert_eq!(modal.error_msg.as_deref(), Some(READ_ONLY_MSG));

        let mut app = AppState::default();
        actions.delete_media_source(&mut app, &telemetry, 0);
        assert_eq!(app.current_feedback(), Some(READ_ONLY_MSG));
    }

    #[test]
    fn test_pixel_to_cell_clamps_to_grid() {
        assert_eq!(pixel_to_cell(0.0, 400.0, 40), Some(0));
        assert_eq!(pixel_to_cell(145.0, 400.0, 40), Some(14));
        assert_eq!(pixel_to_cell(399.99, 400.0, 40), Some(39));
        assert_eq!(pixel_to_cell(400.0, 400.0, 40), None);
        assert_eq!(pixel_to_cell(-1.0, 400.0, 40), None);
        assert_eq!(pixel_to_cell(10.0, 0.0, 40), None);
        assert_eq!(pixel_to_cell(10.0, 400.0, 0), None);
    }

    #[test]
    fn test_tap_on_compact_tab_switches_view() {
        let mut state = WebAppState {
            frame_area: Rect::new(0, 0, 40, 44),
            ..Default::default()
        };
        assert!(state.tap(20, 43));
        assert_eq!(state.app.view, ViewPage::MediaBrowser);
        assert!(!state.tap(25, 10));
        assert_eq!(state.app.view, ViewPage::MediaBrowser);
    }

    #[test]
    fn test_swipe_scrolls_one_line_per_row_travelled() {
        let mut state = WebAppState::default();
        state.press(KeyCode::Char('6'));
        state.drag_start(500.0, 20.0);
        state.drag_move(490.0);
        assert_eq!(state.app.system_scroll_index, 0, "under one row: no scroll");
        state.drag_move(430.0);
        assert_eq!(
            state.app.system_scroll_index, 3,
            "finger up 3.5 rows scrolls down 3"
        );
        state.drag_move(470.0);
        assert_eq!(state.app.system_scroll_index, 1, "coming back scrolls up");
        assert!(state.drag_end(), "a swipe that scrolled is not a tap");
        assert!(state.drag.is_none());
    }

    #[test]
    fn test_drag_without_travel_or_row_height_stays_a_tap() {
        let mut state = WebAppState::default();
        state.drag_start(500.0, 0.0);
        state.drag_move(100.0);
        assert!(!state.drag_end());
        state.drag_start(500.0, 20.0);
        state.drag_move(505.0);
        assert!(!state.drag_end());
        assert!(!state.drag_end(), "no drag in progress");
    }

    #[test]
    fn test_http_error_message_hints_known_gateway_statuses() {
        assert!(http_error_message(401, "/x").contains("Token"));
        assert!(http_error_message(421, "/x").contains("SNI"));
        assert!(http_error_message(429, "/x").contains("Rate limit"));
        assert_eq!(
            http_error_message(502, "/dbsrt/v1/monitoring/top"),
            "⚠️ Erreur HTTP 502 (/dbsrt/v1/monitoring/top)"
        );
    }

    #[test]
    fn test_next_poll_delay_backs_off_and_resets() {
        let mut delay = TELEMETRY_POLL_INTERVAL_MS;
        for _ in 0..10 {
            delay = next_poll_delay_ms(delay, false);
        }
        assert_eq!(delay, TELEMETRY_MAX_BACKOFF_MS);
        assert_eq!(next_poll_delay_ms(TELEMETRY_POLL_INTERVAL_MS, false), 4000);
        assert_eq!(next_poll_delay_ms(delay, true), TELEMETRY_POLL_INTERVAL_MS);
    }
}
