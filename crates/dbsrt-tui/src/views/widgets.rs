//! Reusable UI widgets, layout utilities, and status badge builders for `dbsrt top`.
//!
//! Provides modular components for popup placement, time formatting, and unified
//! visual status badges across all top view tabs.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders},
};

use crate::model::{CompletedStatus, IngestionStatus};

/// Percentage factor for symmetric horizontal layout padding (50% each side).
pub const LAYOUT_CENTER_PERCENTAGE_DIVISOR: u16 = 2;

/// Standard base 100 percentage allocation.
pub const TOTAL_PERCENTAGE: u16 = 100;

/// Queries the current UTC timestamp in seconds since Unix epoch across platforms.
///
/// # Returns
/// `Some(u64)` with seconds elapsed since 1970-01-01, or `None` if clock retrieval fails.
///
/// # Data Flow & Invariants
/// Uses native `std::time::SystemTime` on non-wasm targets.
#[cfg(not(target_arch = "wasm32"))]
fn get_now_epoch_secs() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Queries the current UTC timestamp in seconds since Unix epoch across platforms.
///
/// # Returns
/// `Some(u64)` with seconds elapsed since 1970-01-01, or `None` if clock retrieval fails.
///
/// # Data Flow & Invariants
/// Uses JavaScript `Date.now()` on wasm32 targets to avoid unsupported OS syscall panics.
#[cfg(target_arch = "wasm32")]
fn get_now_epoch_secs() -> Option<u64> {
    let millis = js_sys::Date::now();
    if millis >= 0.0 {
        Some((millis / crate::constants::MILLIS_PER_SEC) as u64)
    } else {
        None
    }
}

/// Formats the current local wall-clock time as `HH:MM:SS` across native and WebAssembly platforms.
///
/// # Returns
/// A formatted string representing the current system time in 24-hour format.
///
/// # Panics
/// Guaranteed not to panic; falls back to `"--:--:--"` if clock retrieval fails.
///
/// # Data Flow & Invariants
/// 1. Queries platform epoch seconds via [`get_now_epoch_secs`].
/// 2. Converts timestamp to UTC time of day.
/// 3. Formats buffer into fixed 2-digit zero-padded segments.
pub fn get_current_time_str() -> String {
    if let Some(secs) = get_now_epoch_secs() {
        let secs_of_day = secs % crate::constants::SECONDS_PER_DAY;
        let hour = secs_of_day / crate::constants::SECONDS_PER_HOUR;
        let min = (secs_of_day % crate::constants::SECONDS_PER_HOUR)
            / crate::constants::SECONDS_PER_MINUTE;
        let sec = secs_of_day % crate::constants::SECONDS_PER_MINUTE;
        format!("{hour:02}:{min:02}:{sec:02}")
    } else {
        "--:--:--".to_string()
    }
}

/// Computes a centered sub-rectangle inside a bounding area with fixed height and relative width.
///
/// # Arguments
/// * `percent_x` - Horizontal width as percentage of parent area (1..100).
/// * `height_lines` - Fixed vertical height in terminal lines.
/// * `r` - Bounding rectangle area.
///
/// # Returns
/// A new `Rect` positioned in the visual center of `r`.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// 1. Calculates vertical margins by dividing remaining height by 2.
/// 2. Calculates horizontal margins symmetrically from `percent_x`.
/// 3. Returns the intersection sub-rectangle.
pub fn centered_rect(percent_x: u16, height_lines: u16, r: Rect) -> Rect {
    let vertical_margin = r.height.saturating_sub(height_lines) / LAYOUT_CENTER_PERCENTAGE_DIVISOR;
    let horizontal_margin =
        (TOTAL_PERCENTAGE.saturating_sub(percent_x)) / LAYOUT_CENTER_PERCENTAGE_DIVISOR;

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vertical_margin),
            Constraint::Length(height_lines),
            Constraint::Min(0),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(horizontal_margin),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(horizontal_margin),
        ])
        .split(popup_layout[1])[1]
}

/// Builds a standardized color badge `Span` for a `CompletedStatus`.
///
/// # Arguments
/// * `status` - The operational status of a completed media task.
///
/// # Returns
/// A styled `Span<'static>` with appropriate color and symbol.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// Maps nominal status to bold green, cached status to yellow, warnings to bold yellow,
/// cancellations to magenta, and fatal errors to bold red.
pub fn completed_status_badge(status: CompletedStatus) -> Span<'static> {
    match status {
        CompletedStatus::Ok => Span::styled(
            "✓ OK",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        CompletedStatus::Cache => Span::styled("✓ CACHE", Style::default().fg(Color::Yellow)),
        CompletedStatus::NoSrt => Span::styled(
            "⚠ NO_SRT",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        CompletedStatus::Cancelled => Span::styled(
            "⏹ CANCEL",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        CompletedStatus::Fail => Span::styled(
            "✗ FAIL",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    }
}

/// Builds a standardized color badge `Span` for an `IngestionStatus`.
///
/// # Arguments
/// * `status` - The catalog ingestion lifecycle state of a media file.
///
/// # Returns
/// A styled `Span<'static>` representing virgin, in-progress, completed, or failed state.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// Ensures visual 1:1 parity with the contextual glossary in `HelpTab::MediaBrowser`.
pub fn ingestion_status_badge(status: IngestionStatus) -> Span<'static> {
    match status {
        IngestionStatus::Virgin => Span::styled("VIERGE", Style::default().fg(Color::Yellow)),
        IngestionStatus::InProgress => Span::styled(
            "EN COURS",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        IngestionStatus::Completed => Span::styled(
            "COMPLÉTÉ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        IngestionStatus::PartialFail => Span::styled(
            "HACHÉ SEUL",
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        ),
    }
}

/// Builds a standardized panel container `Block` with consistent border styling.
///
/// # Arguments
/// * `title` - Title string displayed on top border.
/// * `is_focused` - Whether this panel currently has keyboard focus.
///
/// # Returns
/// A configured `Block<'static>` ready for rendering.
///
/// # Panics
/// Guaranteed not to panic.
///
/// # Data Flow & Invariants
/// Focus state applies bold cyan border highlight; unfocused panels use subtle dark gray borders.
pub fn standard_panel_block(title: &'static str, is_focused: bool) -> Block<'static> {
    let border_style = if is_focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
}
