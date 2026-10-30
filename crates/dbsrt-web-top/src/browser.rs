//! Browser glue of the web-top dashboard (wasm32 only): DOM backend, keyboard, Fetch polling.

use std::cell::RefCell;
use std::rc::Rc;

use dbsrt_tui::app::handle_key_event;
use dbsrt_tui::model::SystemTelemetry;
use dbsrt_tui::ui::render_ui;
use dbsrt_web_top::{
    http_error_message, next_poll_delay_ms, to_tui_key, ReadOnlyActions, WebAppState,
    TELEMETRY_ENDPOINT_PATH, TELEMETRY_POLL_INTERVAL_MS,
};
use gloo_net::http::Request;
use gloo_timers::future::TimeoutFuture;
use ratzilla::event::KeyEvent;
use ratzilla::ratatui::backend::{Backend, WindowSize};
use ratzilla::ratatui::buffer::Cell;
use ratzilla::ratatui::layout::{Position, Size};
use ratzilla::ratatui::text::Span;
use ratzilla::ratatui::Terminal;
use ratzilla::{DomBackend, WebRenderer};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

/// Ratzilla [`DomBackend`] that behaves like a terminal for wide (2-column) glyphs.
///
/// # Data Flow & Invariants
/// Ratatui's diff never sends the cell hidden behind a wide glyph. A terminal clears it when the
/// glyph is written; the DOM keeps one span per cell, so the stale symbol from a previous view
/// stayed visible (e.g. `📋Â`). Each wide cell is now followed by a blank cell update.
struct WideGlyphDom(DomBackend);

impl Backend for WideGlyphDom {
    fn draw<'a, I>(&mut self, content: I) -> std::io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let blank = Cell::default();
        let mut cells: Vec<(u16, u16, &Cell)> = Vec::new();
        for (x, y, cell) in content {
            cells.push((x, y, cell));
            if Span::raw(cell.symbol()).width() > 1 {
                cells.push((x.saturating_add(1), y, &blank));
            }
        }
        self.0.draw(cells.into_iter())
    }

    fn hide_cursor(&mut self) -> std::io::Result<()> {
        self.0.hide_cursor()
    }

    fn show_cursor(&mut self) -> std::io::Result<()> {
        self.0.show_cursor()
    }

    fn get_cursor_position(&mut self) -> std::io::Result<Position> {
        self.0.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> std::io::Result<()> {
        self.0.set_cursor_position(position)
    }

    fn clear(&mut self) -> std::io::Result<()> {
        self.0.clear()
    }

    fn size(&self) -> std::io::Result<Size> {
        self.0.size()
    }

    fn window_size(&mut self) -> std::io::Result<WindowSize> {
        self.0.window_size()
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Registers a `keydown` listener that feeds the shared state machine.
///
/// # Data Flow & Invariants
/// Replaces Ratzilla's `on_key_event`, which never calls `preventDefault`: without it Tab moves
/// browser focus, arrows/Space/PageDown scroll the page and `/` opens Firefox quick-find.
fn register_key_listener(model: Rc<RefCell<WebAppState>>) {
    let closure = Closure::<dyn FnMut(_)>::new(move |event: web_sys::KeyboardEvent| {
        let Some(code) = to_tui_key(&KeyEvent::from(event.clone())) else {
            return;
        };
        event.prevent_default();
        let mut guard = model.borrow_mut();
        let state = &mut *guard;
        handle_key_event(code, &mut state.app, &state.telemetry, &mut ReadOnlyActions);
        // There is no process to quit in a browser tab.
        state.app.should_quit = false;
    });
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        let _ =
            document.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
    }
    closure.forget();
}

/// Resolves the absolute API endpoint path based on browser URL context.
///
/// # Returns
/// [`TELEMETRY_ENDPOINT_PATH`], prefixed with `/dbsrt` behind the o2switch PHP proxy.
///
/// # Data Flow & Invariants
/// Checks window pathname to support both nested path deployments and local ports.
pub fn resolve_telemetry_endpoint() -> String {
    if let Some(window) = web_sys::window() {
        if let Ok(pathname) = window.location().pathname() {
            if pathname.contains("/dbsrt") {
                return format!("/dbsrt{TELEMETRY_ENDPOINT_PATH}");
            }
        }
    }
    TELEMETRY_ENDPOINT_PATH.to_string()
}

/// Extracts API token from URL query string if present.
fn get_token_from_url(window: &web_sys::Window) -> Option<String> {
    let search = window.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    let token = params.get("token")?;
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(Some(storage)) = window.session_storage() {
        let _ = storage.set_item("dbsrt_api_token", trimmed);
    }
    Some(trimmed.to_string())
}

/// Retrieves cached API token from browser session storage.
fn get_token_from_storage(window: &web_sys::Window) -> Option<String> {
    let storage = window.session_storage().ok()??;
    let token = storage.get_item("dbsrt_api_token").ok()??;
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// Extracts API authentication token from URL query string or session storage.
///
/// # Returns
/// `Some(String)` with validated token, or `None`.
///
/// # Data Flow & Invariants
/// Prioritizes `?token=...` parameter in address bar, caching it in `sessionStorage`.
pub fn get_api_token() -> Option<String> {
    let window = web_sys::window()?;
    get_token_from_url(&window).or_else(|| get_token_from_storage(&window))
}

/// Processes and applies successfully retrieved JSON telemetry DTO to shared state.
///
/// # Arguments
/// * `app_model` - Shared reference-counted model container.
/// * `resp` - Successful HTTP response payload to deserialize.
///
/// # Data Flow & Invariants
/// Deserializes the native `SystemTelemetry` snapshot and clears any connection error.
async fn handle_telemetry_success(
    app_model: &Rc<RefCell<WebAppState>>,
    resp: gloo_net::http::Response,
) {
    match resp.json::<SystemTelemetry>().await {
        Ok(telemetry) => {
            let mut state = app_model.borrow_mut();
            state.telemetry = telemetry;
            state.connection_error = None;
        }
        Err(e) => {
            let msg = format!("⚠️ Erreur JSON: {e}");
            web_sys::console::error_1(&msg.clone().into());
            let mut state = app_model.borrow_mut();
            state.connection_error = Some(msg);
        }
    }
}

/// Formats and records telemetry HTTP failure messages for operator feedback.
///
/// # Arguments
/// * `app_model` - Shared reference-counted model container.
/// * `status` - HTTP status code returned by remote server.
/// * `url` - Full endpoint URL queried.
///
/// # Data Flow & Invariants
/// Updates feedback message banner with contextual diagnostic explanation.
fn handle_telemetry_http_error(app_model: &Rc<RefCell<WebAppState>>, status: u16, url: &str) {
    let msg = http_error_message(status, url);
    web_sys::console::warn_1(&msg.clone().into());
    let mut state = app_model.borrow_mut();
    state.connection_error = Some(msg);
}

/// Asynchronously fetches telemetry snapshot from server and updates shared model.
///
/// # Arguments
/// * `app_model` - Shared reference-counted model container.
///
/// # Data Flow & Invariants
/// Emits non-blocking HTTP GET with optional `X-API-Key` header (kept out of the URL so the
/// token never lands in access logs); mutates state on success.
///
/// # Returns
/// `true` when a 2xx response was received.
async fn fetch_telemetry_once(app_model: &Rc<RefCell<WebAppState>>) -> bool {
    let url = resolve_telemetry_endpoint();
    let mut req = Request::get(&url);
    if let Some(token) = get_api_token() {
        req = req.header("X-API-Key", &token);
    }
    match req.send().await {
        Ok(resp) if resp.ok() => {
            handle_telemetry_success(app_model, resp).await;
            true
        }
        Ok(resp) => {
            handle_telemetry_http_error(app_model, resp.status(), &url);
            false
        }
        Err(e) => {
            let msg = format!("⚠️ Échec réseau : {e}");
            web_sys::console::error_1(&msg.clone().into());
            let mut state = app_model.borrow_mut();
            state.connection_error = Some(msg);
            false
        }
    }
}

/// Spawns background telemetry polling loop fetching JSON metrics via browser Fetch API.
///
/// # Arguments
/// * `app_model` - Shared reference-counted model container.
///
/// # Data Flow & Invariants
/// Runs indefinitely on the browser microtask queue using `spawn_local`, backing off on errors.
fn spawn_telemetry_poller(app_model: Rc<RefCell<WebAppState>>) {
    spawn_local(async move {
        let mut delay_ms = TELEMETRY_POLL_INTERVAL_MS;
        loop {
            let success = fetch_telemetry_once(&app_model).await;
            delay_ms = next_poll_delay_ms(delay_ms, success);
            TimeoutFuture::new(delay_ms).await;
        }
    });
}

/// Mounts the Ratzilla TUI in the browser: DOM backend, key listener, poller and draw loop.
///
/// # Returns
/// Result indicating successful setup or DOM backend initialization error.
///
/// # Errors
/// Returns `std::io::Error` if DOM mounting points cannot be allocated.
///
/// # Panics
/// Does not panic under normal runtime execution.
///
/// # Data Flow & Invariants
/// Initializes DOM terminal backend, registers keyboard event listeners,
/// spawns asynchronous HTTP polling worker, and attaches Ratatui draw loop to `requestAnimationFrame`.
pub fn run() -> std::io::Result<()> {
    console_error_panic_hook::set_once();
    let app_model = Rc::new(RefCell::new(WebAppState::default()));

    let backend = DomBackend::new()?;
    let terminal = Terminal::new(WideGlyphDom(backend))?;

    register_key_listener(app_model.clone());

    // Start background asynchronous telemetry polling
    spawn_telemetry_poller(app_model.clone());

    // Render loop bound to browser requestAnimationFrame (same per-frame clamp as native)
    terminal.draw_web({
        let model = app_model.clone();
        move |frame| {
            let mut guard = model.borrow_mut();
            let state = &mut *guard;
            state.app.clamp_indices(&state.telemetry);
            let feedback = state
                .app
                .current_feedback()
                .or(state.connection_error.as_deref());
            render_ui(frame, &state.app.ui_context(&state.telemetry, feedback));
        }
    });

    Ok(())
}
