//! Integration and unit tests for the native Rust TUI top process monitor.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::app::{handle_key_event, AppState};
use dbsrt_cli::commands::top::constants::*;
use dbsrt_cli::commands::top::model::{
    default_progress_bar, format_duration, format_progress_bar, ActiveJobTelemetry, GpuTelemetry,
    SystemTelemetry, ViewPage,
};
use dbsrt_cli::commands::top::process_ctl::{
    check_if_stopped, inspect_job_details, kill_job, stop_job, toggle_pause_job,
};
use dbsrt_cli::commands::top::sampler::{
    is_thread_task, preload_stem_size_cache, read_proc_io_bytes,
};
use dbsrt_cli::commands::top::ui::{render_ui, UiContext};
use dbsrt_cli::commands::top::{probe_telemetry_snapshot, run_top, TopArgs};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[test]
fn test_format_duration_boundaries() {
    assert_eq!(format_duration(0.0), "0s");
    assert_eq!(format_duration(45.0), "45s");
    assert_eq!(format_duration(60.0), "1m00s");
    assert_eq!(format_duration(75.0), "1m15s");
    assert_eq!(format_duration(3600.0), "1h00m");
    assert_eq!(format_duration(3665.0), "1h01m");
    assert_eq!(format_duration(7200.0), "2h00m");
    assert_eq!(format_duration(-5.0), "N/A");
    assert_eq!(format_duration(10_000_000.0), "N/A");
}

#[test]
fn test_format_progress_bar_clamping() {
    let empty_bar = format_progress_bar(0.0, 10);
    assert_eq!(empty_bar, "[░░░░░░░░░░]   0.0%");

    let half_bar = format_progress_bar(0.5, 10);
    assert_eq!(half_bar, "[█████░░░░░]  50.0%");

    let full_bar = format_progress_bar(1.0, 10);
    assert_eq!(full_bar, "[██████████] 100.0%");

    let over_bar = format_progress_bar(1.5, 10);
    assert_eq!(over_bar, "[██████████] 100.0%");

    let under_bar = format_progress_bar(-0.5, 10);
    assert_eq!(under_bar, "[░░░░░░░░░░]   0.0%");
}

#[test]
fn test_default_progress_bar_length() {
    let bar = default_progress_bar(0.75);
    assert!(bar.starts_with('['));
    assert!(bar.ends_with('%'));
    assert!(bar.contains("75.0%"));
}

#[test]
fn test_gpu_telemetry_defaults() {
    let gpu = GpuTelemetry::default();
    assert!(!gpu.available);
    assert_eq!(gpu.video_busy_pct, 0.0);
    assert_eq!(gpu.render_busy_pct, 0.0);
    assert_eq!(gpu.freq_mhz, 0.0);
    assert_eq!(gpu.power_watts, 0.0);
}

#[test]
fn test_active_job_telemetry_instantiation() {
    let job = ActiveJobTelemetry {
        pid: 12345,
        media_name: "Silicon.Valley.S01E01.mkv".to_string(),
        engine: "GPU (VA-API)".to_string(),
        elapsed_secs: 42.5,
        read_bytes: 500 * BYTES_PER_MIB,
        total_bytes: 1000 * BYTES_PER_MIB,
        progress_ratio: 0.5,
        current_rate_mib_s: 25.4,
        eta_secs: Some(19.6),
        cpu_percent: 18.5,
        ram_rss_mib: 64.0,
        status: "RUNNING".to_string(),
        cmdline: vec![
            "ffmpeg".to_string(),
            "-hwaccel".to_string(),
            "vaapi".to_string(),
        ],
    };

    assert_eq!(job.pid, 12345);
    assert_eq!(job.media_name, "Silicon.Valley.S01E01.mkv");
    assert_eq!(job.engine, "GPU (VA-API)");
    assert_eq!(job.progress_ratio, 0.5);
    assert_eq!(job.status, "RUNNING");
    assert_eq!(job.eta_secs, Some(19.6));
}

#[test]
fn test_view_page_transitions() {
    assert_eq!(ViewPage::default(), ViewPage::ActiveJobs);
    let mut page = ViewPage::ActiveJobs;
    assert_eq!(page, ViewPage::ActiveJobs);
    page = ViewPage::JobInspection;
    assert_eq!(page, ViewPage::JobInspection);
    page = ViewPage::Help;
    assert_eq!(page, ViewPage::Help);
}

#[test]
fn test_system_telemetry_aggregation() {
    let mut sys = SystemTelemetry::default();
    assert!(sys.active_jobs.is_empty());
    assert_eq!(sys.total_rate_mib_s, 0.0);

    sys.active_jobs.push(ActiveJobTelemetry {
        pid: 101,
        media_name: "Test1".to_string(),
        engine: "GPU (VA-API)".to_string(),
        elapsed_secs: 10.0,
        read_bytes: 100,
        total_bytes: 200,
        progress_ratio: 0.5,
        current_rate_mib_s: 15.0,
        eta_secs: None,
        cpu_percent: 10.0,
        ram_rss_mib: 30.0,
        status: "RUNNING".to_string(),
        cmdline: vec![],
    });

    sys.active_jobs.push(ActiveJobTelemetry {
        pid: 102,
        media_name: "Test2".to_string(),
        engine: "CPU (Software)".to_string(),
        elapsed_secs: 5.0,
        read_bytes: 50,
        total_bytes: 200,
        progress_ratio: 0.25,
        current_rate_mib_s: 20.0,
        eta_secs: None,
        cpu_percent: 45.0,
        ram_rss_mib: 40.0,
        status: "RUNNING".to_string(),
        cmdline: vec![],
    });

    sys.total_rate_mib_s = sys.active_jobs.iter().map(|j| j.current_rate_mib_s).sum();
    assert_eq!(sys.active_jobs.len(), 2);
    assert_eq!(sys.total_rate_mib_s, 35.0);
}

#[test]
fn test_ui_rendering_all_pages() {
    let backend = TestBackend::new(140, 45);
    let mut terminal = Terminal::new(backend).unwrap();

    let tel = SystemTelemetry {
        host_cpu_pct: 25.0,
        ram_used_gib: 8.5,
        ram_total_gib: 32.0,
        gpu: GpuTelemetry {
            video_busy_pct: 85.0,
            render_busy_pct: 12.0,
            freq_mhz: 1100.0,
            power_watts: 15.5,
            available: true,
        },
        active_jobs: vec![
            ActiveJobTelemetry {
                pid: 1111,
                media_name: "Series.S01E01.1080p.mkv".to_string(),
                engine: "GPU (VA-API)".to_string(),
                elapsed_secs: 65.0,
                read_bytes: 5 * BYTES_PER_MIB,
                total_bytes: 500 * BYTES_PER_MIB,
                progress_ratio: 0.01,
                current_rate_mib_s: 18.5,
                eta_secs: Some(45.0),
                cpu_percent: 15.0,
                ram_rss_mib: 120.0,
                status: "RUNNING".to_string(),
                cmdline: vec![
                    "ffmpeg".to_string(),
                    "-i".to_string(),
                    "video.mkv".to_string(),
                ],
            },
            ActiveJobTelemetry {
                pid: 2222,
                media_name: "Series.S01E02.1080p.mkv".to_string(),
                engine: "Sub Extraction".to_string(),
                elapsed_secs: 10.0,
                read_bytes: 50 * BYTES_PER_MIB,
                total_bytes: 100 * BYTES_PER_MIB,
                progress_ratio: 0.5,
                current_rate_mib_s: 25.0,
                eta_secs: Some(5.0),
                cpu_percent: 4.0,
                ram_rss_mib: 25.0,
                status: "PAUSED".to_string(),
                cmdline: vec![
                    "ffmpeg".to_string(),
                    "-map".to_string(),
                    "0:s:0".to_string(),
                ],
            },
        ],
        total_rate_mib_s: 43.5,
        net_rx_mib_s: 38.2,
    };

    // 1. Render ActiveJobs view
    let ctx = UiContext {
        telemetry: &tel,
        view: ViewPage::ActiveJobs,
        selected_index: 0,
        feedback_msg: Some("Test Feedback Message"),
    };
    terminal.draw(|f| render_ui(f, &ctx)).unwrap();

    // 2. Render ActiveJobs view with moderate GPU
    let mut tel_mod = tel.clone();
    tel_mod.gpu.video_busy_pct = 40.0;
    let ctx_mod = UiContext {
        telemetry: &tel_mod,
        view: ViewPage::ActiveJobs,
        selected_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_mod)).unwrap();

    // 3. Render ActiveJobs view empty
    let empty_tel = SystemTelemetry::default();
    let ctx_empty = UiContext {
        telemetry: &empty_tel,
        view: ViewPage::ActiveJobs,
        selected_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_empty)).unwrap();

    // 4. Render JobInspection view
    let ctx_inspect = UiContext {
        telemetry: &tel,
        view: ViewPage::JobInspection,
        selected_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_inspect)).unwrap();

    // 5. Render Help view
    let ctx_help = UiContext {
        telemetry: &tel,
        view: ViewPage::Help,
        selected_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_help)).unwrap();
}

#[test]
fn test_app_state_and_key_handling() {
    let mut app = AppState::new();
    assert_eq!(app.view, ViewPage::ActiveJobs);
    assert_eq!(app.selected_index, 0);
    assert!(!app.should_quit);

    app.set_feedback("Signal sent".to_string());
    assert_eq!(app.current_feedback(), Some("Signal sent"));

    let tel = SystemTelemetry {
        active_jobs: vec![
            ActiveJobTelemetry {
                pid: 5555,
                media_name: "Job1".to_string(),
                engine: "GPU (VA-API)".to_string(),
                elapsed_secs: 10.0,
                read_bytes: 10,
                total_bytes: 100,
                progress_ratio: 0.1,
                current_rate_mib_s: 1.0,
                eta_secs: None,
                cpu_percent: 1.0,
                ram_rss_mib: 10.0,
                status: "RUNNING".to_string(),
                cmdline: vec![],
            },
            ActiveJobTelemetry {
                pid: 6666,
                media_name: "Job2".to_string(),
                engine: "CPU (Software)".to_string(),
                elapsed_secs: 10.0,
                read_bytes: 10,
                total_bytes: 100,
                progress_ratio: 0.1,
                current_rate_mib_s: 1.0,
                eta_secs: None,
                cpu_percent: 1.0,
                ram_rss_mib: 10.0,
                status: "RUNNING".to_string(),
                cmdline: vec![],
            },
        ],
        ..Default::default()
    };

    // Navigation Down
    handle_key_event(KeyCode::Down, &mut app, &tel);
    assert_eq!(app.selected_index, 1);

    // Navigation Up
    handle_key_event(KeyCode::Char('k'), &mut app, &tel);
    assert_eq!(app.selected_index, 0);

    // Enter -> Inspection
    handle_key_event(KeyCode::Enter, &mut app, &tel);
    assert_eq!(app.view, ViewPage::JobInspection);

    // Backspace -> ActiveJobs
    handle_key_event(KeyCode::Backspace, &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // '?' -> Help
    handle_key_event(KeyCode::Char('?'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::Help);

    // '?' again -> ActiveJobs
    handle_key_event(KeyCode::Char('h'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // 'p' -> Pause feedback
    handle_key_event(KeyCode::Char('p'), &mut app, &tel);
    assert!(app.current_feedback().is_some());

    // 's' -> Stop feedback
    handle_key_event(KeyCode::Char('s'), &mut app, &tel);
    assert!(app.current_feedback().is_some());

    // Switch to inspection and press 'K'
    app.view = ViewPage::JobInspection;
    handle_key_event(KeyCode::Char('K'), &mut app, &tel);
    assert!(app.current_feedback().is_some());

    // Esc in inspection -> back to ActiveJobs
    handle_key_event(KeyCode::Esc, &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // 'q' -> quit
    handle_key_event(KeyCode::Char('q'), &mut app, &tel);
    assert!(app.should_quit);
}

#[test]
fn test_process_ctl_and_inspection() {
    let self_pid = std::process::id();
    let _stopped = check_if_stopped(self_pid);

    let dummy_job = ActiveJobTelemetry {
        pid: self_pid,
        media_name: "SelfTestProcess".to_string(),
        engine: "GPU (VA-API)".to_string(),
        elapsed_secs: 15.0,
        read_bytes: 50 * BYTES_PER_MIB,
        total_bytes: 100 * BYTES_PER_MIB,
        progress_ratio: 0.5,
        current_rate_mib_s: 12.0,
        eta_secs: Some(4.0),
        cpu_percent: 5.0,
        ram_rss_mib: 30.0,
        status: "RUNNING".to_string(),
        cmdline: vec!["cargo".to_string(), "test".to_string()],
    };

    let details = inspect_job_details(&dummy_job);
    assert!(!details.is_empty());
    assert!(details.iter().any(|l| l.contains("PID:")));

    // Test non-existent PID error branches
    let (ok, msg) = toggle_pause_job(99_999_999);
    assert!(!ok);
    assert!(msg.contains("Erreur") || msg.contains("introuvable"));

    let (ok2, _msg2) = stop_job(99_999_999);
    assert!(!ok2);

    let (ok3, _msg3) = kill_job(99_999_999);
    assert!(!ok3);
}

#[test]
fn test_sampler_functions_and_json_probe() {
    preload_stem_size_cache();
    let self_pid = std::process::id();
    let _ = is_thread_task(self_pid);
    let _ = read_proc_io_bytes(self_pid);

    // Test JSON probe with minimal interval
    let snap = probe_telemetry_snapshot(200).expect("probe_telemetry_snapshot failed");
    assert!(snap.ram_total_gib > 0.0);

    // Test run_top with json=true
    let args = TopArgs {
        refresh_ms: 200,
        json: true,
    };
    assert!(run_top(&args).is_ok());
}

#[test]
fn test_event_loop_step_execution() {
    let backend = TestBackend::new(140, 45);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut app = AppState::new();

    let shared = std::sync::Arc::new(std::sync::RwLock::new(SystemTelemetry {
        active_jobs: vec![ActiveJobTelemetry {
            pid: 1234,
            media_name: "TestMovie.mkv".to_string(),
            engine: "GPU (VA-API)".to_string(),
            elapsed_secs: 10.0,
            read_bytes: 100,
            total_bytes: 200,
            progress_ratio: 0.5,
            current_rate_mib_s: 10.0,
            eta_secs: Some(10.0),
            cpu_percent: 5.0,
            ram_rss_mib: 50.0,
            status: "RUNNING".to_string(),
            cmdline: vec![],
        }],
        ..Default::default()
    }));

    // Step 1: normal step
    let res = dbsrt_cli::commands::top::app::event_loop_step(&mut terminal, &mut app, &shared);
    assert!(res.is_ok());

    // Step 2: out of bounds index clamping
    app.selected_index = 99;
    let res2 = dbsrt_cli::commands::top::app::event_loop_step(&mut terminal, &mut app, &shared);
    assert!(res2.is_ok());
    assert_eq!(app.selected_index, 0);
}

#[test]
fn test_process_candidate_jobs_pipeline() {
    use dbsrt_cli::commands::top::sampler::process_candidate_jobs;
    use std::collections::HashMap;
    use std::time::Instant;

    let now = Instant::now();
    let mut prev_io = HashMap::new();
    prev_io.insert(
        1001,
        (
            now - std::time::Duration::from_millis(500),
            10 * 1024 * 1024,
        ),
    );

    let raw = vec![
        // 1. dbsrt coordinator with active child -> skipped
        (
            1000,
            0,
            vec!["dbsrt".to_string(), "media:add".to_string()],
            "dbsrt media:add".to_string(),
            true,
            false,
            5.0,
            10_000_000,
            10,
        ),
        // 2. ffmpeg vaapi decoding (seen before -> delta rate calculated)
        (
            1001,
            1000,
            vec![
                "ffmpeg".to_string(),
                "-hwaccel".to_string(),
                "vaapi".to_string(),
                "-i".to_string(),
                "test.mkv".to_string(),
            ],
            "ffmpeg -hwaccel vaapi -i test.mkv".to_string(),
            false,
            true,
            45.0,
            150_000_000,
            10,
        ),
        // 3. ffmpeg subtitle extraction (first tick -> rate 0.0)
        (
            1002,
            1000,
            vec![
                "ffmpeg".to_string(),
                "-i".to_string(),
                "test.mkv".to_string(),
                "-map".to_string(),
                "0:s:0".to_string(),
                "-f".to_string(),
                "srt".to_string(),
            ],
            "ffmpeg -i test.mkv -map 0:s:0 -f srt".to_string(),
            false,
            true,
            2.0,
            20_000_000,
            2,
        ),
        // 4. dbsrt standalone with cpu hwaccel
        (
            1003,
            0,
            vec![
                "dbsrt".to_string(),
                "media:add".to_string(),
                "--hwaccel".to_string(),
                "cpu".to_string(),
                "clip.mp4".to_string(),
            ],
            "dbsrt media:add --hwaccel cpu clip.mp4".to_string(),
            true,
            false,
            80.0,
            50_000_000,
            5,
        ),
    ];

    let ffmpeg_ppids = vec![1000];
    let (jobs, new_prev) = process_candidate_jobs(raw, &ffmpeg_ppids, prev_io, now);

    assert_eq!(jobs.len(), 3); // 1000 skipped
    assert!(new_prev.contains_key(&1001));
    assert!(new_prev.contains_key(&1002));
    assert!(new_prev.contains_key(&1003));

    let j1001 = jobs.iter().find(|j| j.pid == 1001).unwrap();
    assert_eq!(j1001.engine, "GPU (VA-API)");
    assert_eq!(j1001.media_name, "test");

    let j1002 = jobs.iter().find(|j| j.pid == 1002).unwrap();
    assert_eq!(j1002.engine, "Sub Extraction");

    let j1003 = jobs.iter().find(|j| j.pid == 1003).unwrap();
    assert_eq!(j1003.engine, "CPU (Software)");
    assert_eq!(j1003.media_name, "clip");
}

#[test]
fn test_get_file_size_cache_and_fallbacks() {
    use dbsrt_cli::commands::top::sampler::get_file_size;
    use std::path::Path;

    // Existing file
    let size_cargo = get_file_size("Cargo.toml", Some(Path::new("Cargo.toml")));
    assert!(size_cargo > 0);

    // Non-existent path
    let size_none = get_file_size(
        "unknown_file",
        Some(Path::new("/tmp/non_existent_12345.xyz")),
    );
    assert_eq!(size_none, 0);

    // Without target_path
    let size_empty = get_file_size("unknown_no_path", None);
    assert_eq!(size_empty, 0);
}
