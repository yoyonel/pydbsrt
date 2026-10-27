//! Shared test harness and telemetry fixtures for dbsrt top scenario tests.

use std::sync::{Arc, RwLock};

use crossterm::event::KeyCode;
use dbsrt_cli::commands::top::app::{handle_key_event_with_state, AppState};
use dbsrt_cli::commands::top::model::{
    ActiveJobTelemetry, DiscoveredMediaItem, HistoricalRecord, IngestionStatus, MediaSourceConfig,
    MediaSubtrackInfo, PhashFingerprintInfo, SystemTelemetry,
};
use dbsrt_cli::commands::top::ui::{render_ui, UiContext};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::path::PathBuf;

/// Test harness encapsulating app state, telemetry, and headless terminal rendering.
pub struct TestHarness {
    pub app: AppState,
    pub shared_state: Arc<RwLock<SystemTelemetry>>,
    pub terminal: Terminal<TestBackend>,
}

impl TestHarness {
    pub fn new(width: u16, height: u16, initial_telemetry: SystemTelemetry) -> Self {
        Self {
            app: AppState::default(),
            shared_state: Arc::new(RwLock::new(initial_telemetry)),
            terminal: Terminal::new(TestBackend::new(width, height)).unwrap(),
        }
    }

    pub fn send_key(&mut self, key: KeyCode) {
        let tel = self.shared_state.read().unwrap().clone();
        handle_key_event_with_state(key, &mut self.app, &tel, Some(&self.shared_state));
    }

    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.send_key(KeyCode::Char(ch));
        }
    }

    pub fn draw(&mut self) {
        let tel = self.shared_state.read().unwrap().clone();
        let ctx = UiContext {
            telemetry: &tel,
            view: self.app.view,
            selected_index: self.app.selected_index,
            completed_scroll_index: self.app.completed_scroll_index,
            media_scroll_index: self.app.media_scroll_index,
            source_scroll_index: self.app.source_scroll_index,
            source_modal: self.app.source_modal.as_ref(),
            selected_media_paths: &self.app.selected_media_paths,
            media_filter: self.app.media_filter,
            media_sort: self.app.media_sort,
            source_filter: &self.app.source_filter,
            completed_filter: self.app.completed_filter,
            completed_sort: self.app.completed_sort,
            search_query: self.app.current_search_query(),
            search_active: self.app.search_active,
            help_tab: self.app.help_tab,
            help_scroll_index: self.app.help_scroll_index,
            feedback_msg: self
                .app
                .feedback_message
                .as_ref()
                .map(|(msg, _)| msg.as_str()),
        };
        self.terminal.draw(|f| render_ui(f, &ctx)).unwrap();
    }

    /// Renders UI and extracts 2D character text grid line by line with deterministic clock redaction.
    pub fn render_to_text(&mut self) -> String {
        self.draw();
        let buf = self.terminal.backend().buffer();
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                let cell = buf.cell((x, y)).unwrap();
                line.push_str(cell.symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        Self::sanitize_clock(&out)
    }

    /// Sanitizes non-deterministic wall-clock timestamps in header (e.g. "08:52:10" -> "HH:MM:SS").
    fn sanitize_clock(s: &str) -> String {
        let mut lines = Vec::new();
        for line in s.lines() {
            if line.contains('⏱') {
                let mut chars: Vec<char> = line.chars().collect();
                let mut i = 0;
                while i + 7 < chars.len() {
                    if chars[i].is_ascii_digit()
                        && chars[i + 1].is_ascii_digit()
                        && chars[i + 2] == ':'
                        && chars[i + 3].is_ascii_digit()
                        && chars[i + 4].is_ascii_digit()
                        && chars[i + 5] == ':'
                        && chars[i + 6].is_ascii_digit()
                        && chars[i + 7].is_ascii_digit()
                    {
                        chars[i] = 'H';
                        chars[i + 1] = 'H';
                        chars[i + 3] = 'M';
                        chars[i + 4] = 'M';
                        chars[i + 6] = 'S';
                        chars[i + 7] = 'S';
                        break;
                    }
                    i += 1;
                }
                lines.push(chars.into_iter().collect::<String>());
                continue;
            }
            lines.push(line.to_string());
        }
        lines.join("\n") + "\n"
    }

    pub fn assert_buffer_contains(&mut self, needle: &str) {
        let text = self.render_to_text();
        assert!(
            text.contains(needle),
            "Expected screen buffer to contain '{needle}', but was not found.\nCaptured buffer:\n{text}"
        );
    }

    pub fn assert_buffer_does_not_contain(&mut self, needle: &str) {
        let text = self.render_to_text();
        assert!(
            !text.contains(needle),
            "Expected screen buffer NOT to contain '{needle}', but it was found!\nCaptured buffer:\n{text}"
        );
    }

    pub fn feedback_text(&self) -> Option<String> {
        self.app
            .feedback_message
            .as_ref()
            .map(|(msg, _)| msg.clone())
    }
}

/// Creates a complete realistic telemetry state with active jobs, history, sources, and media.
pub fn mock_complete_telemetry() -> SystemTelemetry {
    let src_video = MediaSourceConfig {
        id: "nas_video".to_string(),
        name: "Synology NAS (video)".to_string(),
        path: PathBuf::from("/mnt/nas/video"),
        enabled: true,
        media_count: 3,
        last_scan_added: 1,
        last_scan_iso: Some("2026-10-27T08:00:00Z".to_string()),
    };
    let src_tvshow = MediaSourceConfig {
        id: "nas_tvshow".to_string(),
        name: "Synology NAS (tvshow)".to_string(),
        path: PathBuf::from("/mnt/nas/tvshow"),
        enabled: true,
        media_count: 2,
        last_scan_added: 0,
        last_scan_iso: Some("2026-10-27T08:00:00Z".to_string()),
    };

    SystemTelemetry {
        host_cpu_pct: 18.5,
        ram_used_gib: 8.4,
        ram_total_gib: 31.0,
        active_jobs: vec![
            ActiveJobTelemetry {
                pid: 50101,
                media_name: "The.Wire.S01E01.1080p.mkv".to_string(),
                engine: "GPU (VA-API)".to_string(),
                engine_reason: None,
                elapsed_secs: 42.0,
                read_bytes: 420 * 1024 * 1024,
                total_bytes: 1400 * 1024 * 1024,
                progress_ratio: 0.30,
                current_rate_mib_s: 24.5,
                eta_secs: Some(85.0),
                cpu_percent: 15.0,
                ram_rss_mib: 64.0,
                status: "RUNNING".to_string(),
                cmdline: vec![
                    "ffmpeg".to_string(),
                    "-vaapi_device".to_string(),
                    "/dev/dri/renderD128".to_string(),
                    "-i".to_string(),
                    "The.Wire.S01E01.1080p.mkv".to_string(),
                ],
            },
            ActiveJobTelemetry {
                pid: 50102,
                media_name: "The.Wire.S01E02.1080p.mkv".to_string(),
                engine: "CPU Multi".to_string(),
                engine_reason: Some("VA-API fallback".to_string()),
                elapsed_secs: 15.0,
                read_bytes: 150 * 1024 * 1024,
                total_bytes: 1500 * 1024 * 1024,
                progress_ratio: 0.10,
                current_rate_mib_s: 18.2,
                eta_secs: Some(135.0),
                cpu_percent: 92.0,
                ram_rss_mib: 88.0,
                status: "RUNNING".to_string(),
                cmdline: vec![
                    "ffmpeg".to_string(),
                    "-i".to_string(),
                    "The.Wire.S01E02.1080p.mkv".to_string(),
                ],
            },
        ],
        history_records: vec![
            HistoricalRecord {
                video_stem: "Carnivàle (2003) - S02E08".to_string(),
                video_rel_path: Some("Carnivale.S02E08.mkv".to_string()),
                success: true,
                wall_duration_secs: 447.0,
                cifs_read_rate_mib_s: 3.6,
                file_size_bytes: 1_689_255_936,
                certified_couplings_count: 2,
                error_summary: None,
                start_iso: Some("2026-10-27T02:01:00Z".to_string()),
                end_iso: Some("2026-10-27T02:08:27Z".to_string()),
            },
            HistoricalRecord {
                video_stem: "The.Expanse.S02E03.1080p".to_string(),
                video_rel_path: Some("The.Expanse.S02E03.1080p.mkv".to_string()),
                success: true,
                wall_duration_secs: 75.0,
                cifs_read_rate_mib_s: 44.6,
                file_size_bytes: 3_517_841_408,
                certified_couplings_count: 1,
                error_summary: None,
                start_iso: Some("2026-10-27T01:58:45Z".to_string()),
                end_iso: Some("2026-10-27T02:00:00Z".to_string()),
            },
            HistoricalRecord {
                video_stem: "The Leftovers S01E01".to_string(),
                video_rel_path: Some("The.Leftovers.S01E01.mkv".to_string()),
                success: false,
                wall_duration_secs: 0.8,
                cifs_read_rate_mib_s: 0.0,
                file_size_bytes: 732_000_000,
                certified_couplings_count: 0,
                error_summary: Some(
                    "Aucun fichier .srt correspondant trouvé sur le disque/NAS pour cette release"
                        .to_string(),
                ),
                start_iso: Some("2026-10-27T01:52:17Z".to_string()),
                end_iso: Some("2026-10-27T01:52:18Z".to_string()),
            },
            HistoricalRecord {
                video_stem: "The Leftovers S01E05".to_string(),
                video_rel_path: Some("The.Leftovers.S01E05.mkv".to_string()),
                success: false,
                wall_duration_secs: 0.5,
                cifs_read_rate_mib_s: 0.0,
                file_size_bytes: 574_000_000,
                certified_couplings_count: 0,
                error_summary: Some(
                    "Interruption manuelle ou système par signal SIGTERM / SIGKILL".to_string(),
                ),
                start_iso: Some("2026-10-27T01:40:35Z".to_string()),
                end_iso: Some("2026-10-27T01:40:35Z".to_string()),
            },
            HistoricalRecord {
                video_stem: "The.Expanse.S04E08.HDR.2160p".to_string(),
                video_rel_path: Some("The.Expanse.S04E08.HDR.2160p.mkv".to_string()),
                success: false,
                wall_duration_secs: 129.0,
                cifs_read_rate_mib_s: 44.4,
                file_size_bytes: 6_027_000_000,
                certified_couplings_count: 0,
                error_summary: Some(
                    "Échec décodage FFmpeg: Hardware device error during initialization"
                        .to_string(),
                ),
                start_iso: Some("2026-10-27T01:38:25Z".to_string()),
                end_iso: Some("2026-10-27T01:40:34Z".to_string()),
            },
        ],
        media_sources: vec![src_video, src_tvshow],
        discovered_media: vec![
            DiscoveredMediaItem {
                source_id: Some("nas_video".to_string()),
                path: PathBuf::from("/mnt/nas/video/Movie.A.2024.mkv"),
                stem: "Movie.A.2024".to_string(),
                size_bytes: 4_500_000_000,
                duration_secs: Some(7200.0),
                video_codec: Some("hevc".to_string()),
                subtitles: vec![],
                phash: None,
                status: IngestionStatus::Virgin,
            },
            DiscoveredMediaItem {
                source_id: Some("nas_video".to_string()),
                path: PathBuf::from("/mnt/nas/video/Movie.B.2023.mkv"),
                stem: "Movie.B.2023".to_string(),
                size_bytes: 2_100_000_000,
                duration_secs: Some(5400.0),
                video_codec: Some("h264".to_string()),
                subtitles: vec![MediaSubtrackInfo {
                    index: 0,
                    lang: "fre".to_string(),
                    title: "VF".to_string(),
                    codec: "subrip".to_string(),
                }],
                phash: Some(PhashFingerprintInfo {
                    exists: true,
                    frame_count: 5400,
                    size_bytes: 43200,
                }),
                status: IngestionStatus::Completed,
            },
            DiscoveredMediaItem {
                source_id: Some("nas_video".to_string()),
                path: PathBuf::from("/mnt/nas/video/Movie.C.2022.mkv"),
                stem: "Movie.C.2022".to_string(),
                size_bytes: 1_200_000_000,
                duration_secs: Some(3600.0),
                video_codec: Some("h264".to_string()),
                subtitles: vec![],
                phash: None,
                status: IngestionStatus::Virgin,
            },
            DiscoveredMediaItem {
                source_id: Some("nas_tvshow".to_string()),
                path: PathBuf::from("/mnt/nas/tvshow/Show.S01E01.mkv"),
                stem: "Show.S01E01".to_string(),
                size_bytes: 800_000_000,
                duration_secs: Some(2700.0),
                video_codec: Some("h264".to_string()),
                subtitles: vec![],
                phash: Some(PhashFingerprintInfo {
                    exists: true,
                    frame_count: 2700,
                    size_bytes: 21600,
                }),
                status: IngestionStatus::PartialFail,
            },
            DiscoveredMediaItem {
                source_id: Some("nas_tvshow".to_string()),
                path: PathBuf::from("/mnt/nas/tvshow/Show.S01E02.mkv"),
                stem: "Show.S01E02".to_string(),
                size_bytes: 850_000_000,
                duration_secs: Some(2800.0),
                video_codec: Some("h264".to_string()),
                subtitles: vec![],
                phash: None,
                status: IngestionStatus::Virgin,
            },
        ],
        ..Default::default()
    }
}
