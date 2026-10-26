//! Integration and unit tests for the native Rust TUI top process monitor.

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::app::{handle_key_event, AppState};
use dbsrt_cli::commands::top::constants::*;
use dbsrt_cli::commands::top::model::{
    calculate_dimension_stats, calculate_overall_stats, default_progress_bar, format_duration,
    format_progress_bar, parse_iso_to_unix_secs, ActiveJobTelemetry, GpuTelemetry,
    HistoricalRecord, SessionThroughput, SystemTelemetry, ViewPage,
};
use dbsrt_cli::commands::top::process_ctl::{
    check_if_stopped, inspect_job_details, kill_job, stop_job, toggle_pause_job,
};
use dbsrt_cli::commands::top::sampler::{
    compute_session_throughput, is_thread_task, load_historical_records, preload_stem_size_cache,
    read_proc_io_bytes,
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

    let history = vec![
        HistoricalRecord {
            video_stem: "Series.S01E01.1080p".to_string(),
            success: true,
            wall_duration_secs: 120.0,
            cifs_read_rate_mib_s: 24.5,
            file_size_bytes: 1024 * BYTES_PER_MIB,
            certified_couplings_count: 2,
            start_iso: Some("2026-10-26T14:00:00".to_string()),
            end_iso: Some("2026-10-26T14:02:00".to_string()),
        },
        HistoricalRecord {
            video_stem: "Series.S01E02.1080p".to_string(),
            success: true,
            wall_duration_secs: 0.5,
            cifs_read_rate_mib_s: 600.0,
            file_size_bytes: 500 * BYTES_PER_MIB,
            certified_couplings_count: 1,
            start_iso: Some("2026-10-26T14:03:00".to_string()),
            end_iso: Some("2026-10-26T14:03:01".to_string()),
        },
        HistoricalRecord {
            video_stem: "Series.S01E03.1080p".to_string(),
            success: false,
            wall_duration_secs: 45.0,
            cifs_read_rate_mib_s: 10.0,
            file_size_bytes: 800 * BYTES_PER_MIB,
            certified_couplings_count: 0,
            start_iso: Some("2026-10-26T14:05:00".to_string()),
            end_iso: Some("2026-10-26T14:05:45".to_string()),
        },
    ];
    let stats = dbsrt_cli::commands::top::model::calculate_overall_stats(&history);

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
        session: SessionThroughput {
            session_bytes: 2 * BYTES_PER_GIB,
            session_duration_secs: 120.0,
            session_rate_mib_s: 17.5,
            stream_count: 3,
        },
        history_records: history,
        stats,
    };

    // 1. Render ActiveJobs view
    let ctx = UiContext {
        telemetry: &tel,
        view: ViewPage::ActiveJobs,
        selected_index: 0,
        completed_scroll_index: 0,
        feedback_msg: Some("Test Feedback Message"),
    };
    terminal.draw(|f| render_ui(f, &ctx)).unwrap();

    // 2. Render ActiveJobs view with moderate GPU and unavailable GPU
    let mut tel_mod = tel.clone();
    tel_mod.gpu.video_busy_pct = 40.0;
    let ctx_mod = UiContext {
        telemetry: &tel_mod,
        view: ViewPage::ActiveJobs,
        selected_index: 0,
        completed_scroll_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_mod)).unwrap();

    tel_mod.gpu.available = false;
    let ctx_no_gpu = UiContext {
        telemetry: &tel_mod,
        view: ViewPage::ActiveJobs,
        selected_index: 0,
        completed_scroll_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_no_gpu)).unwrap();

    // 3. Render ActiveJobs view empty
    let empty_tel = SystemTelemetry::default();
    let ctx_empty = UiContext {
        telemetry: &empty_tel,
        view: ViewPage::ActiveJobs,
        selected_index: 0,
        completed_scroll_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_empty)).unwrap();

    // 4. Render StatsHistory view
    let ctx_stats = UiContext {
        telemetry: &tel,
        view: ViewPage::StatsHistory,
        selected_index: 0,
        completed_scroll_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_stats)).unwrap();

    // 5. Render CompletedMedia view
    let ctx_completed = UiContext {
        telemetry: &tel,
        view: ViewPage::CompletedMedia,
        selected_index: 0,
        completed_scroll_index: 1,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_completed)).unwrap();

    // 6. Render CompletedMedia view empty
    let ctx_completed_empty = UiContext {
        telemetry: &empty_tel,
        view: ViewPage::CompletedMedia,
        selected_index: 0,
        completed_scroll_index: 0,
        feedback_msg: None,
    };
    terminal
        .draw(|f| render_ui(f, &ctx_completed_empty))
        .unwrap();

    // 7. Render JobInspection view
    let ctx_inspect = UiContext {
        telemetry: &tel,
        view: ViewPage::JobInspection,
        selected_index: 0,
        completed_scroll_index: 0,
        feedback_msg: None,
    };
    terminal.draw(|f| render_ui(f, &ctx_inspect)).unwrap();

    // 8. Render Help view
    let ctx_help = UiContext {
        telemetry: &tel,
        view: ViewPage::Help,
        selected_index: 0,
        completed_scroll_index: 0,
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
    handle_key_event(KeyCode::Char('?'), &mut app, &tel);
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

#[test]
fn test_parse_iso_to_unix_secs() {
    // Exact Unix Epoch
    assert_eq!(parse_iso_to_unix_secs("1970-01-01T00:00:00"), Some(0.0));

    // Year 2000 milestone
    assert_eq!(
        parse_iso_to_unix_secs("2000-01-01T00:00:00"),
        Some(946684800.0)
    );

    // Space delimiter instead of 'T'
    assert_eq!(
        parse_iso_to_unix_secs("2000-01-01 00:00:00"),
        Some(946684800.0)
    );

    // Valid ISO with microseconds
    let res = parse_iso_to_unix_secs("2026-10-25T22:36:56.415607");
    assert!(res.is_some());
    assert!(res.unwrap() > 1_700_000_000.0);

    // Invalid formatting boundaries
    assert_eq!(parse_iso_to_unix_secs("too_short"), None);
    assert_eq!(parse_iso_to_unix_secs("2026/10/25T22:36:56"), None);
    assert_eq!(parse_iso_to_unix_secs("2026-13-25T22:36:56"), None); // Invalid month 13
    assert_eq!(parse_iso_to_unix_secs("2026-00-25T22:36:56"), None); // Invalid month 0
    assert_eq!(parse_iso_to_unix_secs("2026-10-32T22:36:56"), None); // Invalid day 32
    assert_eq!(parse_iso_to_unix_secs("2026-10-25T25:36:56"), None); // Invalid hour 25
    assert_eq!(parse_iso_to_unix_secs("2026-10-25T22:65:56"), None); // Invalid minute 65
    assert_eq!(parse_iso_to_unix_secs("2026-10-25T22:36:70"), None); // Invalid second 70
}

#[test]
fn test_calculate_dimension_stats() {
    // Empty series
    let empty = calculate_dimension_stats(&[]);
    assert_eq!(empty.min, 0.0);
    assert_eq!(empty.median, 0.0);
    assert_eq!(empty.mean, 0.0);
    assert_eq!(empty.p90, 0.0);
    assert_eq!(empty.max, 0.0);
    assert_eq!(empty.stdev, 0.0);

    // Single item
    let single = calculate_dimension_stats(&[42.0]);
    assert_eq!(single.min, 42.0);
    assert_eq!(single.median, 42.0);
    assert_eq!(single.mean, 42.0);
    assert_eq!(single.p90, 42.0);
    assert_eq!(single.max, 42.0);
    assert_eq!(single.stdev, 0.0);

    // Odd count series [10, 20, 30]
    let odd = calculate_dimension_stats(&[10.0, 30.0, 20.0]);
    assert_eq!(odd.min, 10.0);
    assert_eq!(odd.median, 20.0);
    assert_eq!(odd.mean, 20.0);
    assert_eq!(odd.max, 30.0);
    assert_eq!(odd.p90, 30.0);
    assert_eq!(odd.stdev, 10.0);

    // Even count series [10, 20, 30, 40]
    let even = calculate_dimension_stats(&[40.0, 10.0, 30.0, 20.0]);
    assert_eq!(even.min, 10.0);
    assert_eq!(even.median, 25.0);
    assert_eq!(even.mean, 25.0);
    assert_eq!(even.max, 40.0);
    assert_eq!(even.p90, 40.0);
    assert!(even.stdev > 0.0);
}

#[test]
fn test_calculate_overall_stats() {
    // Empty records
    let empty = calculate_overall_stats(&[]);
    assert_eq!(empty.sample_count, 0);

    // Filter out short duration (< 60s) or failed jobs
    let records = vec![
        HistoricalRecord {
            video_stem: "Failed".to_string(),
            success: false,
            wall_duration_secs: 150.0,
            cifs_read_rate_mib_s: 20.0,
            file_size_bytes: 100 * BYTES_PER_MIB,
            certified_couplings_count: 1,
            start_iso: None,
            end_iso: None,
        },
        HistoricalRecord {
            video_stem: "CachedOrSkipped".to_string(),
            success: true,
            wall_duration_secs: 5.0, // < 60.0s
            cifs_read_rate_mib_s: 50.0,
            file_size_bytes: 100 * BYTES_PER_MIB,
            certified_couplings_count: 1,
            start_iso: None,
            end_iso: None,
        },
        HistoricalRecord {
            video_stem: "SuccessFull1".to_string(),
            success: true,
            wall_duration_secs: 120.0, // 2 mins
            cifs_read_rate_mib_s: 20.0,
            file_size_bytes: 1024 * BYTES_PER_MIB,
            certified_couplings_count: 2,
            start_iso: None,
            end_iso: None,
        },
        HistoricalRecord {
            video_stem: "SuccessFull2".to_string(),
            success: true,
            wall_duration_secs: 240.0, // 4 mins
            cifs_read_rate_mib_s: 30.0,
            file_size_bytes: 2048 * BYTES_PER_MIB,
            certified_couplings_count: 4,
            start_iso: None,
            end_iso: None,
        },
    ];

    let overall = calculate_overall_stats(&records);
    assert_eq!(overall.sample_count, 2);
    assert_eq!(overall.rate.min, 20.0);
    assert_eq!(overall.rate.max, 30.0);
    assert_eq!(overall.duration_mins.min, 2.0);
    assert_eq!(overall.duration_mins.max, 4.0);
    assert_eq!(overall.couplings.min, 2.0);
    assert_eq!(overall.couplings.max, 4.0);
}

#[test]
fn test_compute_session_throughput() {
    let recent_iso_end = "2023-11-14T22:00:00"; // within 2h if now is aligned
    let old_iso_end = "2020-01-01T00:00:00";

    let history = vec![
        HistoricalRecord {
            video_stem: "RecentJob".to_string(),
            success: true,
            wall_duration_secs: 60.0,
            cifs_read_rate_mib_s: 20.0,
            file_size_bytes: 50 * BYTES_PER_MIB,
            certified_couplings_count: 1,
            start_iso: Some("2023-11-14T21:50:00".to_string()),
            end_iso: Some(recent_iso_end.to_string()),
        },
        HistoricalRecord {
            video_stem: "AncientJob".to_string(),
            success: true,
            wall_duration_secs: 60.0,
            cifs_read_rate_mib_s: 20.0,
            file_size_bytes: 100 * BYTES_PER_MIB,
            certified_couplings_count: 1,
            start_iso: Some("2020-01-01T00:00:00".to_string()),
            end_iso: Some(old_iso_end.to_string()),
        },
    ];

    let active = vec![ActiveJobTelemetry {
        pid: 9999,
        media_name: "ActiveStream".to_string(),
        engine: "GPU (VA-API)".to_string(),
        elapsed_secs: 50.0,
        read_bytes: 25 * BYTES_PER_MIB,
        total_bytes: 100 * BYTES_PER_MIB,
        progress_ratio: 0.25,
        current_rate_mib_s: 15.0,
        eta_secs: Some(30.0),
        cpu_percent: 10.0,
        ram_rss_mib: 45.0,
        status: "RUNNING".to_string(),
        cmdline: vec![],
    }];

    // Compute session throughput with aligned timestamp
    let recent_unix = parse_iso_to_unix_secs(recent_iso_end).unwrap();
    let throughput = compute_session_throughput(&history, &active, recent_unix + 100.0);

    assert_eq!(throughput.stream_count, 2); // 1 recent + 1 active
    assert_eq!(throughput.session_bytes, 75 * BYTES_PER_MIB);
    assert!(throughput.session_duration_secs > 0.0);
    assert!(throughput.session_rate_mib_s > 0.0);
}

#[test]
fn test_load_historical_records_and_deduplication() {
    use std::io::Write;

    let dir = std::env::temp_dir().join(format!("pydbsrt_test_load_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);

    let events_path = dir.join("events.jsonl");
    let couplings_path = dir.join("couplings.json");

    // Write events with duplicates (stem 'Duplicate' has shorter and longer duration)
    {
        let mut f = std::fs::File::create(&events_path).unwrap();
        writeln!(
            f,
            r#"{{"video_stem": "Duplicate", "success": true, "wall_duration_secs": 10.0, "cifs_read_rate_mib_s": 5.0, "file_size_bytes": 1000, "certified_couplings_count": 1, "start_iso": "2026-10-25T10:00:00", "end_iso": "2026-10-25T10:00:10"}}"#
        ).unwrap();
        writeln!(
            f,
            r#"{{"video_stem": "Duplicate", "success": true, "wall_duration_secs": 120.0, "cifs_read_rate_mib_s": 25.0, "file_size_bytes": 2000, "certified_couplings_count": 2, "start_iso": "2026-10-25T10:00:00", "end_iso": "2026-10-25T10:02:00"}}"#
        ).unwrap();
        writeln!(
            f,
            r#"{{"video_stem": "Unique", "success": false, "wall_duration_secs": 5.0, "io_read_rate_mib_s": 12.0, "file_size_bytes": 500, "certified_couplings_count": 0, "start_iso": "2026-10-25T11:00:00", "end_iso": "2026-10-25T11:00:05"}}"#
        ).unwrap();
    }

    let loaded = load_historical_records(&events_path, &couplings_path);
    assert_eq!(loaded.len(), 2);

    let dup = loaded.iter().find(|r| r.video_stem == "Duplicate").unwrap();
    assert_eq!(dup.wall_duration_secs, 120.0); // kept the maximum wall duration
    assert_eq!(dup.cifs_read_rate_mib_s, 25.0);

    let unq = loaded.iter().find(|r| r.video_stem == "Unique").unwrap();
    assert_eq!(unq.cifs_read_rate_mib_s, 12.0); // fell back to io_read_rate_mib_s

    // Test fallback to couplings.json when events file does not exist
    let non_existent_events = dir.join("non_existent_events.jsonl");
    {
        let mut f = std::fs::File::create(&couplings_path).unwrap();
        writeln!(
            f,
            r#"{{"couplings": [{{"media_name": "CouplingRef", "frame_count": 2500, "fps": 25.0, "subtitles": ["track1", "track2"]}}]}}"#
        ).unwrap();
    }

    let fallback_loaded = load_historical_records(&non_existent_events, &couplings_path);
    assert_eq!(fallback_loaded.len(), 1);
    let c_item = &fallback_loaded[0];
    assert_eq!(c_item.video_stem, "CouplingRef");
    assert_eq!(c_item.wall_duration_secs, 100.0); // 2500 / 25.0
    assert_eq!(c_item.certified_couplings_count, 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_app_state_new_tabs_and_scrolling() {
    let mut app = AppState::new();

    let tel = SystemTelemetry {
        active_jobs: vec![
            ActiveJobTelemetry {
                pid: 1234,
                media_name: "Job1".to_string(),
                engine: "GPU".to_string(),
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
                pid: 1235,
                media_name: "Job2".to_string(),
                engine: "GPU".to_string(),
                elapsed_secs: 12.0,
                read_bytes: 20,
                total_bytes: 100,
                progress_ratio: 0.2,
                current_rate_mib_s: 2.0,
                eta_secs: None,
                cpu_percent: 2.0,
                ram_rss_mib: 12.0,
                status: "RUNNING".to_string(),
                cmdline: vec![],
            },
        ],
        history_records: vec![
            HistoricalRecord {
                video_stem: "Rec1".to_string(),
                success: true,
                wall_duration_secs: 10.0,
                cifs_read_rate_mib_s: 1.0,
                file_size_bytes: 100,
                certified_couplings_count: 1,
                start_iso: None,
                end_iso: None,
            },
            HistoricalRecord {
                video_stem: "Rec2".to_string(),
                success: true,
                wall_duration_secs: 20.0,
                cifs_read_rate_mib_s: 2.0,
                file_size_bytes: 200,
                certified_couplings_count: 2,
                start_iso: None,
                end_iso: None,
            },
            HistoricalRecord {
                video_stem: "Rec3".to_string(),
                success: true,
                wall_duration_secs: 30.0,
                cifs_read_rate_mib_s: 3.0,
                file_size_bytes: 300,
                certified_couplings_count: 3,
                start_iso: None,
                end_iso: None,
            },
        ],
        ..Default::default()
    };

    // Tab key cycling: ActiveJobs -> StatsHistory -> CompletedMedia -> ActiveJobs
    handle_key_event(KeyCode::Tab, &mut app, &tel);
    assert_eq!(app.view, ViewPage::StatsHistory);

    handle_key_event(KeyCode::Tab, &mut app, &tel);
    assert_eq!(app.view, ViewPage::CompletedMedia);

    handle_key_event(KeyCode::Tab, &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // BackTab cycling: ActiveJobs -> CompletedMedia -> StatsHistory -> ActiveJobs
    handle_key_event(KeyCode::BackTab, &mut app, &tel);
    assert_eq!(app.view, ViewPage::CompletedMedia);

    handle_key_event(KeyCode::BackTab, &mut app, &tel);
    assert_eq!(app.view, ViewPage::StatsHistory);

    handle_key_event(KeyCode::BackTab, &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // Direct key shortcuts '1', '2', '3'
    handle_key_event(KeyCode::Char('2'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::StatsHistory);

    handle_key_event(KeyCode::Char('3'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::CompletedMedia);

    handle_key_event(KeyCode::Char('1'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // 'h' toggles StatsHistory, 'r' toggles CompletedMedia
    handle_key_event(KeyCode::Char('h'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::StatsHistory);
    handle_key_event(KeyCode::Char('h'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    handle_key_event(KeyCode::Char('r'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::CompletedMedia);
    handle_key_event(KeyCode::Char('r'), &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // Scrolling in CompletedMedia
    app.view = ViewPage::CompletedMedia;
    assert_eq!(app.completed_scroll_index, 0);

    handle_key_event(KeyCode::Down, &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 1);

    handle_key_event(KeyCode::Char('j'), &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 2);

    // Down beyond limit clamps to max index (2)
    handle_key_event(KeyCode::Down, &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 2);

    handle_key_event(KeyCode::Up, &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 1);

    handle_key_event(KeyCode::Char('k'), &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 0);

    // Home, End, PageUp, PageDown in CompletedMedia
    handle_key_event(KeyCode::End, &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 2);

    handle_key_event(KeyCode::Home, &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 0);

    handle_key_event(KeyCode::PageDown, &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 2);

    handle_key_event(KeyCode::PageUp, &mut app, &tel);
    assert_eq!(app.completed_scroll_index, 0);

    // Esc from CompletedMedia returns to ActiveJobs
    handle_key_event(KeyCode::Esc, &mut app, &tel);
    assert_eq!(app.view, ViewPage::ActiveJobs);

    // Home, End, PageUp, PageDown in ActiveJobs
    handle_key_event(KeyCode::End, &mut app, &tel);
    assert_eq!(app.selected_index, 1);

    handle_key_event(KeyCode::Home, &mut app, &tel);
    assert_eq!(app.selected_index, 0);

    handle_key_event(KeyCode::PageDown, &mut app, &tel);
    assert_eq!(app.selected_index, 1);

    handle_key_event(KeyCode::PageUp, &mut app, &tel);
    assert_eq!(app.selected_index, 0);
}
