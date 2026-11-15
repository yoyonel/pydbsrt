//! Bounded log tail scanning, timestamp parsing, and error signature deduplication.
//!
//! # Data Flow & Invariants
//! Reads at most `ALERTS_LOG_TAIL_BYTES` from the end of daemon/worker log files, discards any
//! truncated leading partial line, parses ISO-8601 and bracketed timestamps into Unix epoch seconds,
//! redacts credentials via `dbsrt_core`, and collapses variable numbers/hex into normalized error
//! signatures with `1h` / `24h` / `72h` window counts.

use crate::constants::{
    ALERTS_LOG_TAIL_BYTES, ALERT_WINDOW_1H_SECS, ALERT_WINDOW_24H_SECS, ALERT_WINDOW_72H_SECS,
    CIVIL_DAYS_PER_400_YEARS, CIVIL_EPOCH_DAYS_OFFSET, MAX_ALERT_SUMMARY_CHARS,
    MAX_DEDUP_LOG_ALERTS_PER_SOURCE, SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE,
};
use crate::dtos::{AlertSeverity, AlertWindowCounts, MonitoringAlertItem};
use crate::handlers::monitoring::format_unix_to_iso;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Minimum ASCII byte length of a `YYYY-MM-DD HH:MM:SS` timestamp prefix.
const TIMESTAMP_PREFIX_LEN: usize = 19;

/// Minimum plausible Gregorian year accepted by the log timestamp parser.
const MIN_VALID_YEAR: i64 = 1970;

/// Maximum plausible Gregorian year accepted by the log timestamp parser.
const MAX_VALID_YEAR: i64 = 2200;

/// Maximum valid calendar month (`1 ..= 12`).
const MAX_VALID_MONTH: u32 = 12;

/// Maximum valid calendar day of month (`1 ..= 31`).
const MAX_VALID_DAY: u32 = 31;

/// Maximum valid hour of day (`0 ..= 23`).
const MAX_VALID_HOUR: u32 = 23;

/// Maximum valid minute or second (`0 ..= 59`).
const MAX_VALID_MIN_SEC: u32 = 59;

/// Month pivot separating Jan/Feb from Mar..Dec in Howard Hinnant's civil calendar conversion.
const CIVIL_MARCH_PIVOT: u32 = 2;

/// Years per 400-year Gregorian leap cycle.
const CIVIL_YEARS_PER_ERA: i64 = 400;

/// Standard non-leap days per year.
const DAYS_PER_COMMON_YEAR: u32 = 365;

/// Years per quadrennial leap cycle.
const LEAP_YEARS_STEP: u32 = 4;

/// Years per century cycle.
const CENTURY_YEARS_STEP: u32 = 100;

/// Month shift for March..December (`month - 3`).
const MARCH_OFFSET_AFTER_FEB: u32 = 3;

/// Month shift for January..February (`month + 9`).
const MARCH_OFFSET_JAN_FEB: u32 = 9;

/// Linear day-of-year month numerator factor (`153`).
const DOY_MONTH_FACTOR: u32 = 153;

/// Linear day-of-year numerator bias (`2`).
const DOY_MONTH_BIAS: u32 = 2;

/// Linear day-of-year denominator (`5`).
const DOY_MONTH_DIVISOR: u32 = 5;

/// Reads at most `max_bytes` from the end of `path`, discarding any truncated leading line.
///
/// # Arguments
/// * `path` - Target log or NDJSON file path.
/// * `max_bytes` - Upper bound in bytes to read from the tail of `path`.
///
/// # Returns
/// `Some(String)` containing UTF-8 lossy decoded tail text, or `None` when `path` cannot be opened.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// When `file_len > max_bytes`, seeks to `file_len - max_bytes` and advances past the first `\n`
/// byte so callers never parse a torn UTF-8 sequence or half-line JSON/log prefix.
pub fn read_file_tail_utf8(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let truncated = len > max_bytes;
    if truncated {
        let start = len.saturating_sub(max_bytes);
        file.seek(SeekFrom::Start(start)).ok()?;
    }
    let mut buf = Vec::with_capacity(len.min(max_bytes) as usize);
    file.read_to_end(&mut buf).ok()?;
    let slice = if truncated {
        match buf.iter().position(|&b| b == b'\n') {
            Some(pos) => &buf[pos + 1..],
            None => &buf[..],
        }
    } else {
        &buf[..]
    };
    Some(String::from_utf8_lossy(slice).into_owned())
}

/// Converts a Gregorian UTC date/time tuple into Unix epoch seconds.
///
/// # Arguments
/// * `year` - Four-digit Gregorian year (`1970 ..= 2200`).
/// * `month` - One-based month (`1 ..= 12`).
/// * `day` - One-based day of month (`1 ..= 31`).
/// * `hms` - Tuple `(hour, minute, second)` in 24-hour clock.
///
/// # Returns
/// `Some(unix_secs)` when date/time components are within valid bounds, else `None`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Implements Howard Hinnant's `days_from_civil` inverse matching [`format_unix_to_iso`].
pub fn civil_to_unix_secs(year: i64, month: u32, day: u32, hms: (u32, u32, u32)) -> Option<u64> {
    let (hour, min, sec) = hms;
    let valid_date = (MIN_VALID_YEAR..=MAX_VALID_YEAR).contains(&year)
        && (1..=MAX_VALID_MONTH).contains(&month)
        && (1..=MAX_VALID_DAY).contains(&day);
    let valid_time = hour <= MAX_VALID_HOUR && min <= MAX_VALID_MIN_SEC && sec <= MAX_VALID_MIN_SEC;
    if !valid_date || !valid_time {
        return None;
    }

    let y_adj = if month <= CIVIL_MARCH_PIVOT {
        year - 1
    } else {
        year
    };
    let era = if y_adj >= 0 {
        y_adj
    } else {
        y_adj - (CIVIL_YEARS_PER_ERA - 1)
    } / CIVIL_YEARS_PER_ERA;
    let yoe = (y_adj - era * CIVIL_YEARS_PER_ERA) as u32;
    let shifted_month = if month > CIVIL_MARCH_PIVOT {
        month - MARCH_OFFSET_AFTER_FEB
    } else {
        month + MARCH_OFFSET_JAN_FEB
    };
    let doy = (DOY_MONTH_FACTOR * shifted_month + DOY_MONTH_BIAS) / DOY_MONTH_DIVISOR + day - 1;
    let doe = yoe * DAYS_PER_COMMON_YEAR + yoe / LEAP_YEARS_STEP - yoe / CENTURY_YEARS_STEP + doy;
    let days = era * CIVIL_DAYS_PER_400_YEARS + i64::from(doe) - CIVIL_EPOCH_DAYS_OFFSET;
    if days < 0 {
        return None;
    }
    let day_secs =
        u64::from(hour) * SECONDS_PER_HOUR + u64::from(min) * SECONDS_PER_MINUTE + u64::from(sec);
    Some((days as u64) * SECONDS_PER_DAY + day_secs)
}

/// Parses a leading `[YYYY-MM-DD HH:MM:SS]` or ISO-8601 `YYYY-MM-DDTHH:MM:SS...` timestamp.
///
/// # Arguments
/// * `line` - Single log line.
///
/// # Returns
/// `(Option<unix_secs>, rest_of_line)` with any leading timestamp stripped from `rest_of_line`.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Operates on ASCII byte offsets only after verifying ASCII character boundaries.
pub fn split_leading_timestamp(line: &str) -> (Option<u64>, &str) {
    let trimmed = line.trim_start();
    let (candidate, bracketed) = match trimmed.strip_prefix('[') {
        Some(inner) => (inner, true),
        None => (trimmed, false),
    };
    if candidate.len() < TIMESTAMP_PREFIX_LEN || !candidate.is_char_boundary(TIMESTAMP_PREFIX_LEN) {
        return (None, trimmed);
    }
    let bytes = candidate.as_bytes();
    let sep_ok = bytes[4] == b'-'
        && bytes[7] == b'-'
        && (bytes[10] == b' ' || bytes[10] == b'T')
        && bytes[13] == b':'
        && bytes[16] == b':';
    if !sep_ok {
        return (None, trimmed);
    }
    let Some(ts) = parse_fixed_datetime_prefix(&candidate[..TIMESTAMP_PREFIX_LEN]) else {
        return (None, trimmed);
    };
    let rest = skip_timestamp_suffix(&candidate[TIMESTAMP_PREFIX_LEN..], bracketed);
    (Some(ts), rest.trim_start())
}

/// Parses an exact 19-byte `YYYY-MM-DD?HH:MM:SS` slice into Unix epoch seconds.
fn parse_fixed_datetime_prefix(prefix: &str) -> Option<u64> {
    let year: i64 = prefix.get(0..4)?.parse().ok()?;
    let month: u32 = prefix.get(5..7)?.parse().ok()?;
    let day: u32 = prefix.get(8..10)?.parse().ok()?;
    let hour: u32 = prefix.get(11..13)?.parse().ok()?;
    let min: u32 = prefix.get(14..16)?.parse().ok()?;
    let sec: u32 = prefix.get(17..19)?.parse().ok()?;
    civil_to_unix_secs(year, month, day, (hour, min, sec))
}

/// Strips fractional seconds, `Z` / `+HH:MM` timezone suffix, or closing `]` after `HH:MM:SS`.
fn skip_timestamp_suffix(after_sec: &str, bracketed: bool) -> &str {
    if bracketed {
        return after_sec.strip_prefix(']').unwrap_or(after_sec);
    }
    let rest = match after_sec.strip_prefix('.') {
        Some(frac) => frac.trim_start_matches(|c: char| c.is_ascii_digit()),
        None => after_sec,
    };
    if let Some(after_z) = rest.strip_prefix('Z') {
        return after_z;
    }
    if rest.starts_with('+') || rest.starts_with('-') {
        return rest
            .split_once(char::is_whitespace)
            .map_or("", |(_, tail)| tail);
    }
    rest
}

/// Determines whether a log line carries an error, abort, or fatal marker.
///
/// # Arguments
/// * `body` - Log line body (with or without leading timestamp).
///
/// # Returns
/// `(is_error, is_fatal)` flags.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Flags `❌`, non-graceful `🛑`, `FATAL`, and standalone `ERROR` tokens while ignoring benign
/// zero-error status counters (`errors=0`, `error: null`) and graceful `SIGTERM`/`SIGINT` stops
/// (`Received signal ... Stopping`).
pub fn classify_error_line(body: &str) -> (bool, bool) {
    if body.contains("Received signal") && body.contains("Stopping") {
        return (false, false);
    }
    if body.contains("🛑") || has_ascii_word(body, "FATAL") {
        return (true, true);
    }
    if body.contains('❌') || has_ascii_word(body, "ERROR") {
        return (true, false);
    }
    (false, false)
}

/// Checks whether `haystack` contains `word` bounded by non-alphanumeric/underscore ASCII boundaries.
fn has_ascii_word(haystack: &str, word: &str) -> bool {
    let hb = haystack.as_bytes();
    let wb = word.as_bytes();
    if wb.is_empty() || hb.len() < wb.len() {
        return false;
    }
    for idx in 0..=(hb.len() - wb.len()) {
        if &hb[idx..idx + wb.len()] != wb {
            continue;
        }
        let left_ok = idx == 0 || !is_ident_byte(hb[idx - 1]);
        let right_ok = idx + wb.len() == hb.len() || !is_ident_byte(hb[idx + wb.len()]);
        if left_ok && right_ok {
            return true;
        }
    }
    false
}

/// Returns true when `b` is an ASCII alphanumeric or underscore identifier byte.
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Redacts credentials and normalizes variable numeric/hex tokens to form a stable dedup key.
///
/// # Arguments
/// * `body` - Log message body with leading timestamp already stripped.
///
/// # Returns
/// `(signature_key, redacted_summary)` pair.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Applies both `redact_secrets` and `redact_url_userinfo`, truncates `redacted_summary` to
/// `MAX_ALERT_SUMMARY_CHARS` UTF-8 scalar values, and collapses digit/hex runs in `signature_key`
/// into `<N>` so PIDs, ports, and timings deduplicate cleanly.
pub fn normalize_error_signature(body: &str) -> (String, String) {
    let redacted = dbsrt_core::redact_url_userinfo(&dbsrt_core::redact_secrets(body.trim()));
    let summary = truncate_chars(&redacted, MAX_ALERT_SUMMARY_CHARS);
    let mut key = String::with_capacity(summary.len());
    let mut in_num = false;
    for ch in summary.chars() {
        if ch.is_ascii_digit() {
            if !in_num {
                key.push_str("<N>");
                in_num = true;
            }
        } else {
            in_num = false;
            key.push(ch);
        }
    }
    (key, summary)
}

/// Truncates `input` to at most `max_chars` Unicode scalar values without splitting UTF-8 boundaries.
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// Aggregated statistics for one normalized log signature within a single log source.
#[derive(Debug, Clone)]
struct SignatureBucket {
    summary: String,
    is_fatal: bool,
    total_tail: usize,
    has_timestamp: bool,
    last_ts: Option<u64>,
    windows: AlertWindowCounts,
}

/// Summary of scanning a single daemon/worker log file tail.
#[derive(Debug, Clone, Default)]
pub struct LogScanOutcome {
    /// Active deduplicated alert items derived from this log file.
    pub alerts: Vec<MonitoringAlertItem>,
    /// Total error occurrences across `1h` / `24h` / `72h` windows in this log file.
    pub totals: AlertWindowCounts,
}

/// Scans the tail of a log file and returns deduplicated active error alerts + window totals.
///
/// # Arguments
/// * `path` - Filesystem path to the log file.
/// * `component` - Component label (`"batch_worker"`, `"dbsrt-server"`, `"supervisor"`).
/// * `now_unix_secs` - Reference current Unix timestamp in seconds.
///
/// # Returns
/// [`LogScanOutcome`] containing up to `MAX_DEDUP_LOG_ALERTS_PER_SOURCE` active signature alerts.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Un-timestamped continuation lines inherit the most recent timestamp seen earlier in the tail.
/// Signatures whose latest timestamp is older than `72h` are treated as historical and excluded
/// from active alerts.
pub fn scan_log_file_alerts(path: &Path, component: &str, now_unix_secs: u64) -> LogScanOutcome {
    let Some(tail) = read_file_tail_utf8(path, ALERTS_LOG_TAIL_BYTES) else {
        return LogScanOutcome::default();
    };
    let mut buckets: BTreeMap<String, SignatureBucket> = BTreeMap::new();
    let mut totals = AlertWindowCounts::default();
    let mut inherited_ts: Option<u64> = None;

    for raw_line in tail.lines() {
        let (parsed_ts, body) = split_leading_timestamp(raw_line);
        if let Some(ts) = parsed_ts {
            inherited_ts = Some(ts);
        }
        let (is_err, is_fatal) = classify_error_line(body);
        if !is_err || body.is_empty() {
            continue;
        }
        let effective_ts = parsed_ts.or(inherited_ts);
        record_signature_hit(
            &mut buckets,
            &mut totals,
            (body, is_fatal),
            (effective_ts, now_unix_secs),
        );
    }

    let alerts = build_alerts_from_buckets(buckets, component);
    LogScanOutcome { alerts, totals }
}

/// Updates the signature bucket map and aggregate window counters for a single matched error line.
fn record_signature_hit(
    buckets: &mut BTreeMap<String, SignatureBucket>,
    totals: &mut AlertWindowCounts,
    line_info: (&str, bool),
    clocks: (Option<u64>, u64),
) {
    let (body, is_fatal) = line_info;
    let (effective_ts, now_unix_secs) = clocks;
    let (sig_key, summary) = normalize_error_signature(body);

    let entry = buckets.entry(sig_key).or_insert_with(|| SignatureBucket {
        summary: summary.clone(),
        is_fatal,
        total_tail: 0,
        has_timestamp: false,
        last_ts: None,
        windows: AlertWindowCounts::default(),
    });
    entry.summary = summary;
    entry.is_fatal |= is_fatal;
    entry.total_tail += 1;

    if let Some(ts) = effective_ts {
        if ts <= now_unix_secs.saturating_add(SECONDS_PER_DAY) {
            entry.has_timestamp = true;
            entry.last_ts = Some(entry.last_ts.map_or(ts, |prev| prev.max(ts)));
            let age = now_unix_secs.saturating_sub(ts);
            bump_window_counts(&mut entry.windows, age);
            bump_window_counts(totals, age);
        }
    }
}

/// Increments `1h`, `24h`, and `72h` counters according to event `age_secs`.
fn bump_window_counts(counts: &mut AlertWindowCounts, age_secs: u64) {
    if age_secs <= ALERT_WINDOW_1H_SECS {
        counts.h1 += 1;
    }
    if age_secs <= ALERT_WINDOW_24H_SECS {
        counts.h24 += 1;
    }
    if age_secs <= ALERT_WINDOW_72H_SECS {
        counts.h72 += 1;
    }
}

/// Converts active signature buckets into sorted [`MonitoringAlertItem`] entries.
fn build_alerts_from_buckets(
    buckets: BTreeMap<String, SignatureBucket>,
    component: &str,
) -> Vec<MonitoringAlertItem> {
    let mut active: Vec<SignatureBucket> = buckets
        .into_values()
        .filter(|b| !b.has_timestamp || b.windows.h72 > 0)
        .collect();

    active.sort_by(|a, b| {
        b.is_fatal
            .cmp(&a.is_fatal)
            .then_with(|| b.windows.h1.cmp(&a.windows.h1))
            .then_with(|| b.windows.h24.cmp(&a.windows.h24))
            .then_with(|| b.total_tail.cmp(&a.total_tail))
    });

    active
        .into_iter()
        .take(MAX_DEDUP_LOG_ALERTS_PER_SOURCE)
        .enumerate()
        .map(|(idx, b)| {
            let severity = if b.is_fatal || !b.has_timestamp || b.windows.h1 > 0 {
                AlertSeverity::Critical
            } else {
                AlertSeverity::Warning
            };
            let count = if b.has_timestamp {
                b.windows.h72
            } else {
                b.total_tail
            };
            MonitoringAlertItem {
                id: format!("log_error_{}_{}", component.replace('-', "_"), idx + 1),
                severity,
                component: component.to_string(),
                summary: b.summary,
                count,
                last_seen: b.last_ts.map(format_unix_to_iso),
                window_counts: b.has_timestamp.then_some(b.windows),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_civil_to_unix_secs_matches_inverse() {
        let ts = 1_793_352_226;
        let iso = format_unix_to_iso(ts);
        assert_eq!(iso, "2026-10-30T09:23:46Z");
        let (parsed, rest) = split_leading_timestamp("2026-10-30T09:23:46.123456Z ERROR boom");
        assert_eq!(parsed, Some(ts));
        assert_eq!(rest, "ERROR boom");
    }

    #[test]
    fn test_bracketed_timestamp_and_dedup_redaction() {
        let line =
            "[2026-10-30 09:23:46] ❌ ffmpeg -ftp-password sec -i ftp://u:pw@nas/a.mkv pid 42";
        let (parsed, rest) = split_leading_timestamp(line);
        assert_eq!(parsed, Some(1_793_352_226));
        let (sig1, sum1) = normalize_error_signature(rest);
        let (sig2, _) = normalize_error_signature(
            "❌ ffmpeg -ftp-password sec -i ftp://u:pw@nas/a.mkv pid 9999",
        );
        assert_eq!(sig1, sig2);
        assert!(!sum1.contains("sec") && !sum1.contains("pw"));
    }
}
