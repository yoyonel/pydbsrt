//! Volatile per-process extraction progress files shared between the extractor and `dbsrt top`.
//!
//! Each running ffmpeg extraction owns `<jobs dir>/<pid>.json`. The file records the process
//! start time so readers can reject files left behind by a dead process whose PID was reused.

use std::fs;
use std::path::{Path, PathBuf};

/// Env var overriding the directory holding active job progress files.
pub const JOBS_RUN_DIR_ENV: &str = "DBSRT_JOBS_RUN_DIR";
/// Default directory (relative to the working directory) holding active job progress files.
pub const DEFAULT_JOBS_RUN_DIR: &str = "run/jobs";
/// Number of decoded frames between volatile progress status writes.
pub const PROGRESS_UPDATE_INTERVAL_FRAMES: usize = 120;

/// Zero-based index of `starttime` (field 22) among the fields after the `comm` field's `)`.
const PROC_STAT_STARTTIME_INDEX_AFTER_COMM: usize = 19;

/// Directory holding active job progress files: `DBSRT_JOBS_RUN_DIR` or `DEFAULT_JOBS_RUN_DIR`.
pub fn jobs_run_dir() -> PathBuf {
    std::env::var_os(JOBS_RUN_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_JOBS_RUN_DIR), PathBuf::from)
}

/// Progress file path for `pid` inside `dir`.
pub fn job_progress_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{pid}.json"))
}

/// Extracts the process start time (clock ticks since boot) from `/proc/<pid>/stat` content.
pub fn parse_proc_stat_start_ticks(stat: &str) -> Option<u64> {
    let (_, after_comm) = stat.rsplit_once(')')?;
    after_comm
        .split_whitespace()
        .nth(PROC_STAT_STARTTIME_INDEX_AFTER_COMM)?
        .parse()
        .ok()
}

/// Start time of a live process, or `None` when it is gone or `/proc` is unavailable.
pub fn process_start_ticks(pid: u32) -> Option<u64> {
    parse_proc_stat_start_ticks(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// True when a progress file written for `recorded_start` belongs to the live process with
/// `live_start`. A dead process never matches; files without a start time (older writers)
/// match any live process.
pub fn progress_belongs_to_process(recorded_start: Option<u64>, live_start: Option<u64>) -> bool {
    match (recorded_start, live_start) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(recorded), Some(live)) => recorded == live,
    }
}

/// Progress file guard for an active extraction process; removes the file on drop.
pub(crate) struct ExtractionJobGuard {
    file_path: PathBuf,
    start_ticks: Option<u64>,
}

impl Drop for ExtractionJobGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file_path);
    }
}

/// Creates the progress file guard for `pid` in [`jobs_run_dir`].
pub(crate) fn create_job_guard(pid: u32) -> Option<ExtractionJobGuard> {
    let dir = jobs_run_dir();
    fs::create_dir_all(&dir).ok()?;
    Some(ExtractionJobGuard {
        file_path: job_progress_path(&dir, pid),
        start_ticks: process_start_ticks(pid),
    })
}

/// Serializes one progress snapshot as a single JSON line.
fn progress_payload(
    frames: usize,
    total_frames: Option<usize>,
    fps: f64,
    start_ticks: Option<u64>,
) -> String {
    let opt = |v: Option<String>| v.unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"frames\":{frames},\"total_frames\":{},\"fps\":{fps:.3},\"start_ticks\":{}}}\n",
        opt(total_frames.map(|t| t.to_string())),
        opt(start_ticks.map(|t| t.to_string()))
    )
}

/// Writes the snapshot to a temp file then renames it, so readers never see a partial file.
fn write_job_progress(guard: &ExtractionJobGuard, frames: usize, total: Option<usize>, fps: f64) {
    let tmp_path = guard.file_path.with_extension("json.tmp");
    let payload = progress_payload(frames, total, fps, guard.start_ticks);
    if fs::write(&tmp_path, payload).is_ok() && fs::rename(&tmp_path, &guard.file_path).is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
}

/// Writes a progress snapshot every `PROGRESS_UPDATE_INTERVAL_FRAMES` frames.
pub(crate) fn maybe_update_progress(
    guard: Option<&ExtractionJobGuard>,
    frame_idx: usize,
    expected_total: Option<usize>,
    meta_fps: f64,
) {
    if let Some(g) = guard {
        if frame_idx.is_multiple_of(PROGRESS_UPDATE_INTERVAL_FRAMES) {
            write_job_progress(g, frame_idx, expected_total, meta_fps);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_proc_stat_start_ticks() {
        // `comm` may contain spaces and ')'; starttime is field 22.
        let stat = "4242 (ffmpeg (x) y) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 987654 19";
        assert_eq!(parse_proc_stat_start_ticks(stat), Some(987_654));
        assert_eq!(parse_proc_stat_start_ticks("4242 (ffmpeg) S 1"), None);
        assert_eq!(parse_proc_stat_start_ticks("garbage"), None);
    }

    #[test]
    fn test_progress_belongs_to_process() {
        assert!(progress_belongs_to_process(Some(10), Some(10)));
        assert!(!progress_belongs_to_process(Some(10), Some(11)));
        assert!(!progress_belongs_to_process(Some(10), None));
        assert!(!progress_belongs_to_process(None, None));
        assert!(progress_belongs_to_process(None, Some(11)));
    }

    #[test]
    fn test_write_job_progress_is_complete_json_and_removed_on_drop() {
        let dir = std::env::temp_dir().join(format!("dbsrt_jobs_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = job_progress_path(&dir, 7);
        let guard = ExtractionJobGuard {
            file_path: path.clone(),
            start_ticks: Some(55),
        };
        write_job_progress(&guard, 240, None, 23.976);
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "{\"frames\":240,\"total_frames\":null,\"fps\":23.976,\"start_ticks\":55}\n"
        );
        assert!(!path.with_extension("json.tmp").exists());
        drop(guard);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_process_start_ticks_self_is_stable() {
        let me = std::process::id();
        let first = process_start_ticks(me);
        assert!(first.is_some());
        assert_eq!(first, process_start_ticks(me));
    }
}
