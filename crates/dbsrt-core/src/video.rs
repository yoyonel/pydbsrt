//! Video probing and frame extraction via FFmpeg/FFprobe with hardware acceleration.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Execution timeout in seconds for invoking ffprobe.
pub const FFPROBE_TIMEOUT_SECS: u64 = 5;

/// Polling interval for checking child process completion.
pub const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Default baseline frame rate when media FPS cannot be parsed or probed.
pub const DEFAULT_BASELINE_FPS: f64 = 24.0;

/// Standard PAL broadcast frame rate.
pub const PAL_FPS: f64 = 25.0;

/// Standard NTSC film frame rate.
pub const NTSC_FILM_FPS: f64 = 23.976;

#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub duration: f64,
    pub fps: f64,
}

/// Sampling strategy for frame extraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamplingMode {
    /// Decode all frames sequentially at native video framerate.
    Full,
    /// Sample video at a fixed target framerate (e.g., 1.0 fps).
    Fps(f64),
    /// Decode only keyframes (I-frames) for zero B/P-frame decoding overhead.
    KeyFrames,
}

/// Hardware acceleration backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwAccel {
    /// Auto-detect available hardware acceleration (VA-API, CUDA, QSV) with automatic CPU fallback.
    Auto,
    /// Pure software CPU decoding (lavc).
    Disabled,
    /// VA-API (Intel / AMD GPU on Linux).
    Vaapi,
    /// NVIDIA CUDA / NVDEC.
    Cuda,
    /// Intel Quick Sync Video (QSV).
    Qsv,
}

/// Options for configuring video extraction.
#[derive(Debug, Clone)]
pub struct VideoExtractionOptions {
    /// Start position in seconds (corresponds to FFmpeg `-ss`).
    pub start_sec: f64,
    /// Duration in seconds to extract (corresponds to FFmpeg `-t`).
    pub duration_sec: Option<f64>,
    /// Maximum number of frames to extract.
    pub max_frames: Option<usize>,
    /// Sampling strategy (Full, Fps, KeyFrames).
    pub sampling: SamplingMode,
    /// Hardware acceleration backend.
    pub hwaccel: HwAccel,
    /// Optional explicit path to FFmpeg binary.
    pub custom_ffmpeg: Option<PathBuf>,
}

impl Default for VideoExtractionOptions {
    fn default() -> Self {
        Self {
            start_sec: 0.0,
            duration_sec: None,
            max_frames: None,
            sampling: SamplingMode::Full,
            hwaccel: HwAccel::Auto,
            custom_ffmpeg: None,
        }
    }
}

/// A frame hash along with its exact presentation timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSample {
    /// 64-bit perceptual hash (2D DCT-II SIMD AVX2).
    pub phash: u64,
    /// Presentation timestamp in milliseconds from video start.
    pub timestamp_ms: u64,
    /// Sequential index of the extracted frame (0, 1, 2, ...).
    pub frame_idx: usize,
}

#[derive(Deserialize)]
struct FfprobeOutput {
    streams: Option<Vec<FfprobeStream>>,
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    r_frame_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

/// Heuristic stem parser returning framerate if detected in filename tokens.
pub fn parse_fps_from_stem(stem: &str) -> f64 {
    let lower = stem.to_lowercase();
    if let Some(before_fps) = lower.strip_suffix("fps") {
        if let Some(dec_dot) = before_fps.rfind('.') {
            let start = before_fps[..dec_dot]
                .rfind(|c: char| !c.is_ascii_digit())
                .map(|i| i + 1)
                .unwrap_or(0);
            if let Ok(fps) = before_fps[start..].parse::<f64>() {
                if fps > 0.0 {
                    return fps;
                }
            }
        } else {
            let start = before_fps
                .rfind(|c: char| !c.is_ascii_digit())
                .map(|i| i + 1)
                .unwrap_or(0);
            if let Ok(fps) = before_fps[start..].parse::<f64>() {
                if fps > 0.0 {
                    return fps;
                }
            }
        }
    }

    if lower.contains("pal")
        || lower.contains("576i")
        || lower.contains("576p")
        || lower.contains("25fps")
    {
        return PAL_FPS;
    }
    if lower.contains("1080p")
        || lower.contains("720p")
        || lower.contains("2160p")
        || lower.contains("480p")
        || lower.contains("bluray")
        || lower.contains("web-dl")
        || lower.contains("webrip")
        || lower.contains("web")
        || lower.contains("h264")
        || lower.contains("x264")
        || lower.contains("h265")
        || lower.contains("x265")
        || lower.contains("hevc")
        || lower.contains("ntsc")
        || lower.contains("[ember]")
        || lower.contains("[erai-raws]")
        || lower.contains("[subsplease]")
        || lower.contains("kimetsu")
        || lower.contains("haikyuu")
        || lower.contains("anime")
    {
        return NTSC_FILM_FPS;
    }

    DEFAULT_BASELINE_FPS
}

/// Probes a video file using ffprobe with a bounded execution timeout.
pub fn probe_video(path: &Path) -> Result<VideoMetadata> {
    probe_video_with_timeout(path, FFPROBE_TIMEOUT_SECS)
}

/// Probes a video file using ffprobe with a custom timeout.
pub fn probe_video_with_timeout(path: &Path, timeout_secs: u64) -> Result<VideoMetadata> {
    probe_video_with_cmd(path, "ffprobe", timeout_secs)
}

/// Probes a video file with a specific command/binary and timeout.
pub fn probe_video_with_cmd(
    path: &Path,
    ffprobe_bin: &str,
    timeout_secs: u64,
) -> Result<VideoMetadata> {
    let mut child = Command::new(ffprobe_bin)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate,duration:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to execute '{}' on {:?}", ffprobe_bin, path))?;

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        match child.try_wait()? {
            Some(status) => {
                let mut stdout = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_end(&mut stdout)?;
                }
                let mut stderr = String::new();
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_string(&mut stderr)?;
                }

                if !status.success() {
                    anyhow::bail!("{} failed: {}", ffprobe_bin, stderr);
                }
                return parse_ffprobe_json_output(&stdout);
            }
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    anyhow::bail!(
                        "Execution of '{}' on {:?} timed out after {} seconds",
                        ffprobe_bin,
                        path,
                        timeout_secs
                    );
                }
                std::thread::sleep(CHILD_POLL_INTERVAL);
            }
        }
    }
}

/// Parses raw JSON output from ffprobe into VideoMetadata.
pub fn parse_ffprobe_json_output(bytes: &[u8]) -> Result<VideoMetadata> {
    let parsed: FfprobeOutput =
        serde_json::from_slice(bytes).with_context(|| "Failed to parse ffprobe json output")?;

    // Parse FPS
    let fps_str = parsed
        .streams
        .as_ref()
        .and_then(|s| s.first())
        .and_then(|st| st.r_frame_rate.as_deref())
        .unwrap_or("25/1");

    let fps = if let Some((num, den)) = fps_str.split_once('/') {
        let n: f64 = num.parse().unwrap_or(25.0);
        let d: f64 = den.parse().unwrap_or(1.0);
        if d > 0.0 {
            n / d
        } else {
            25.0
        }
    } else {
        fps_str.parse().unwrap_or(25.0)
    };

    // Parse duration
    let duration: f64 = parsed
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(|d| d.parse().ok())
        .or_else(|| {
            parsed
                .streams
                .as_ref()
                .and_then(|s| s.first())
                .and_then(|st| st.duration.as_deref())
                .and_then(|d| d.parse().ok())
        })
        .unwrap_or(0.0);

    Ok(VideoMetadata { duration, fps })
}

/// Detects framerate for a media file applying strict precedence:
/// 1. Explicit CLI argument (`explicit_fps`).
/// 2. Container probe via `ffprobe` with timeout (`FFPROBE_TIMEOUT_SECS`).
/// 3. Filename stem heuristic (`parse_fps_from_stem`).
/// 4. Baseline default (`DEFAULT_BASELINE_FPS` = 24.0).
pub fn detect_media_fps(video_path: &Path, explicit_fps: Option<f64>) -> f64 {
    if let Some(fps) = explicit_fps {
        if fps > 0.0 {
            return fps;
        }
    }

    if let Ok(meta) = probe_video(video_path) {
        if meta.fps > 0.0 {
            return meta.fps;
        }
    }

    if let Some(stem) = video_path.file_stem().and_then(|s| s.to_str()) {
        let stem_fps = parse_fps_from_stem(stem);
        if stem_fps > 0.0 {
            return stem_fps;
        }
    }

    DEFAULT_BASELINE_FPS
}

/// Discovered FFmpeg binary and supported hardware acceleration.
#[derive(Debug, Clone)]
pub struct ResolvedFfmpeg {
    pub binary: PathBuf,
    pub hwaccel: HwAccel,
}

static CACHED_AUTO_FFMPEG: OnceLock<ResolvedFfmpeg> = OnceLock::new();

/// Detects available FFmpeg binary and optimal hardware acceleration backend.
pub fn resolve_ffmpeg(requested: HwAccel, custom_bin: Option<&Path>) -> ResolvedFfmpeg {
    if let Some(custom) = custom_bin {
        return ResolvedFfmpeg {
            binary: custom.to_path_buf(),
            hwaccel: requested,
        };
    }

    if let Ok(env_bin) = std::env::var("DBSRT_FFMPEG").or_else(|_| std::env::var("FFMPEG_PATH")) {
        let p = PathBuf::from(env_bin);
        if p.exists() {
            return ResolvedFfmpeg {
                binary: p,
                hwaccel: requested,
            };
        }
    }

    if requested == HwAccel::Disabled {
        return ResolvedFfmpeg {
            binary: PathBuf::from("ffmpeg"),
            hwaccel: HwAccel::Disabled,
        };
    }

    if requested == HwAccel::Auto {
        if let Some(cached) = CACHED_AUTO_FFMPEG.get() {
            return cached.clone();
        }
        let detected = detect_best_hw_ffmpeg();
        let _ = CACHED_AUTO_FFMPEG.set(detected.clone());
        return detected;
    }

    // Specific backend requested (Vaapi, Cuda, Qsv)
    let candidate = find_ffmpeg_supporting(requested).unwrap_or_else(|| PathBuf::from("ffmpeg"));
    ResolvedFfmpeg {
        binary: candidate,
        hwaccel: requested,
    }
}

fn detect_best_hw_ffmpeg() -> ResolvedFfmpeg {
    // 1. Check VA-API (Intel/AMD render node)
    let has_dri = Path::new("/dev/dri/renderD128").exists() || Path::new("/dev/dri/card0").exists();
    if has_dri {
        if let Some(bin) = find_ffmpeg_supporting(HwAccel::Vaapi) {
            return ResolvedFfmpeg {
                binary: bin,
                hwaccel: HwAccel::Vaapi,
            };
        }
    }

    // 2. Check NVIDIA CUDA
    let has_nvidia = Path::new("/dev/nvidia0").exists() || Path::new("/dev/nvidiactl").exists();
    if has_nvidia {
        if let Some(bin) = find_ffmpeg_supporting(HwAccel::Cuda) {
            return ResolvedFfmpeg {
                binary: bin,
                hwaccel: HwAccel::Cuda,
            };
        }
    }

    // 3. Fallback: standard CPU ffmpeg
    ResolvedFfmpeg {
        binary: PathBuf::from("ffmpeg"),
        hwaccel: HwAccel::Disabled,
    }
}

fn find_ffmpeg_supporting(hw: HwAccel) -> Option<PathBuf> {
    let method_str = match hw {
        HwAccel::Vaapi => "vaapi",
        HwAccel::Cuda => "cuda",
        HwAccel::Qsv => "qsv",
        HwAccel::Disabled | HwAccel::Auto => return None,
    };

    // Candidate locations in priority order:
    // Distro /usr/bin/ffmpeg usually compiles hwaccels while brew/custom might omit them
    let candidates = ["/usr/bin/ffmpeg", "/usr/local/bin/ffmpeg", "ffmpeg"];

    for candidate in candidates {
        if let Ok(output) = Command::new(candidate).arg("-hwaccels").output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.lines().any(|l| l.trim() == method_str) {
                    return Some(PathBuf::from(candidate));
                }
            }
        }
    }

    None
}

/// Probes presentation timestamps (in ms) of keyframes using ffprobe.
pub fn probe_keyframe_timestamps(
    path: &Path,
    start_sec: f64,
    duration_sec: Option<f64>,
) -> Result<Vec<u64>> {
    let mut cmd = Command::new("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-skip_frame",
        "nokey",
        "-select_streams",
        "v:0",
        "-show_entries",
        "frame=best_effort_timestamp_time",
        "-of",
        "csv=p=0",
    ]);

    if start_sec > 0.0 || duration_sec.is_some() {
        let start = start_sec.max(0.0);
        let interval = if let Some(d) = duration_sec {
            format!("{:.3}%+{:.3}", start, d)
        } else {
            format!("{:.3}%", start)
        };
        cmd.args(["-read_intervals", &interval]);
    }

    cmd.arg(path);

    let output = cmd
        .output()
        .with_context(|| format!("Failed to run ffprobe for keyframes on {:?}", path))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut timestamps = Vec::new();
    for line in text.lines() {
        let clean = line.trim().trim_end_matches(',');
        if let Ok(sec) = clean.parse::<f64>() {
            if sec.is_finite() && sec >= 0.0 {
                timestamps.push((sec * 1000.0).round() as u64);
            }
        }
    }

    Ok(timestamps)
}

/// Extracts perceptual hashes from video frames with hardware acceleration,
/// adaptive sampling (Full, 1 FPS, KeyFrames) and exact presentation timestamps.
pub fn extract_frames_advanced(
    path: &Path,
    options: &VideoExtractionOptions,
) -> Result<Vec<FrameSample>> {
    let resolved = resolve_ffmpeg(options.hwaccel, options.custom_ffmpeg.as_deref());

    // Try decoding with resolved configuration; if HWAccel fails, automatically fallback to CPU
    let res = run_extraction_process(path, options, &resolved);
    match res {
        Ok(samples) if !samples.is_empty() => Ok(samples),
        other => {
            if resolved.hwaccel != HwAccel::Disabled {
                // Hardware acceleration failed or produced 0 frames -> fallback to CPU software decoding
                let fallback = ResolvedFfmpeg {
                    binary: PathBuf::from("ffmpeg"),
                    hwaccel: HwAccel::Disabled,
                };
                let fallback_res = run_extraction_process(path, options, &fallback);
                if fallback_res.is_ok() {
                    return fallback_res;
                }
            }
            other
        }
    }
}

fn run_extraction_process(
    path: &Path,
    options: &VideoExtractionOptions,
    resolved: &ResolvedFfmpeg,
) -> Result<Vec<FrameSample>> {
    // Collect keyframe timestamps if KeyFrames mode is requested
    let keyframe_ts = if options.sampling == SamplingMode::KeyFrames {
        probe_keyframe_timestamps(path, options.start_sec, options.duration_sec).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut cmd = Command::new(&resolved.binary);
    cmd.args(["-hide_banner", "-nostats", "-nostdin"]);

    // HWAccel input arguments
    match resolved.hwaccel {
        HwAccel::Vaapi => {
            let device = if Path::new("/dev/dri/renderD128").exists() {
                "/dev/dri/renderD128"
            } else {
                "/dev/dri/card0"
            };
            cmd.args([
                "-hwaccel",
                "vaapi",
                "-vaapi_device",
                device,
                "-hwaccel_output_format",
                "vaapi",
            ]);
        }
        HwAccel::Cuda => {
            cmd.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
        }
        HwAccel::Qsv => {
            cmd.args(["-hwaccel", "qsv", "-hwaccel_output_format", "qsv"]);
        }
        HwAccel::Disabled | HwAccel::Auto => {}
    }

    if options.start_sec > 0.0 {
        cmd.args(["-ss", &format!("{:.3}", options.start_sec)]);
    }

    if options.sampling == SamplingMode::KeyFrames {
        cmd.args(["-skip_frame", "nokey"]);
    }

    cmd.arg("-i").arg(path);

    if let Some(dur) = options.duration_sec {
        cmd.args(["-t", &format!("{:.3}", dur)]);
    }

    if let Some(max_f) = options.max_frames {
        if options.sampling == SamplingMode::Full {
            cmd.args(["-frames:v", &max_f.to_string()]);
        }
    }

    if options.sampling == SamplingMode::KeyFrames {
        cmd.args(["-fps_mode", "passthrough"]);
    }

    // Build filter chain
    let filter_chain = match (resolved.hwaccel, options.sampling) {
        (HwAccel::Vaapi, SamplingMode::Fps(fps)) => {
            format!("fps={fps},scale_vaapi=w=32:h=32,hwdownload,format=nv12,format=gray")
        }
        (HwAccel::Vaapi, _) => {
            "scale_vaapi=w=32:h=32,hwdownload,format=nv12,format=gray".to_string()
        }
        (HwAccel::Cuda, SamplingMode::Fps(fps)) => {
            format!("fps={fps},scale_cuda=w=32:h=32,hwdownload,format=nv12,format=gray")
        }
        (HwAccel::Cuda, _) => "scale_cuda=w=32:h=32,hwdownload,format=nv12,format=gray".to_string(),
        (_, SamplingMode::Fps(fps)) => {
            format!("fps={fps},scale=32:32")
        }
        (_, _) => "scale=32:32".to_string(),
    };

    cmd.args(["-vf", &filter_chain]);

    if resolved.hwaccel == HwAccel::Disabled || resolved.hwaccel == HwAccel::Auto {
        cmd.args(["-pix_fmt", "gray"]);
    }

    cmd.args(["-f", "rawvideo", "-"]);

    cmd.stdout(Stdio::piped()).stderr(Stdio::null());

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn ffmpeg ({:?}) on {:?}",
            resolved.binary, path
        )
    })?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Cannot capture ffmpeg stdout"))?;

    let mut buf = [0u8; 1024];
    let capacity = options.max_frames.unwrap_or(256);
    let mut samples = Vec::with_capacity(capacity);

    // Frame timing parameters
    let meta_fps = if options.sampling == SamplingMode::Full {
        probe_video(path).map(|m| m.fps).unwrap_or(25.0)
    } else {
        25.0
    };

    let start_ms = (options.start_sec * 1000.0).round() as u64;

    while options.max_frames.is_none_or(|m| samples.len() < m) {
        match stdout.read_exact(&mut buf) {
            Ok(()) => {
                let frame_idx = samples.len();
                let phash = crate::phash::phash_32x32(&buf);

                let timestamp_ms = match options.sampling {
                    SamplingMode::Full => {
                        let offset_ms = ((frame_idx as f64 / meta_fps) * 1000.0).round() as u64;
                        start_ms + offset_ms
                    }
                    SamplingMode::Fps(target_fps) => {
                        let offset_ms = ((frame_idx as f64 / target_fps) * 1000.0).round() as u64;
                        start_ms + offset_ms
                    }
                    SamplingMode::KeyFrames => {
                        if frame_idx < keyframe_ts.len() {
                            keyframe_ts[frame_idx]
                        } else {
                            // Fallback approximation if probe returned fewer items
                            start_ms + ((frame_idx as f64) * 1000.0).round() as u64
                        }
                    }
                };

                samples.push(FrameSample {
                    phash,
                    timestamp_ms,
                    frame_idx,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }

    let _ = child.wait();
    Ok(samples)
}

/// Convenience function: extracts 1 FPS sparse perceptual hashes for ultra-fast media identification.
pub fn extract_sparse_phash(path: &Path, target_fps: f64) -> Result<Vec<FrameSample>> {
    let opts = VideoExtractionOptions {
        sampling: SamplingMode::Fps(target_fps),
        hwaccel: HwAccel::Auto,
        ..Default::default()
    };
    extract_frames_advanced(path, &opts)
}

/// Convenience function: extracts keyframe (I-frame) perceptual hashes for instant scene cut analysis.
pub fn extract_keyframes_phash(path: &Path) -> Result<Vec<FrameSample>> {
    let opts = VideoExtractionOptions {
        sampling: SamplingMode::KeyFrames,
        hwaccel: HwAccel::Auto,
        ..Default::default()
    };
    extract_frames_advanced(path, &opts)
}

/// Extracts a stream of 32x32 grayscale frames using ffmpeg and computes pHash on each.
/// Preserved for backward compatibility.
pub fn extract_frames_phash(path: &Path, start_sec: f64, nb_frames: usize) -> Result<Vec<u64>> {
    let opts = VideoExtractionOptions {
        start_sec,
        max_frames: Some(nb_frames),
        sampling: SamplingMode::Full,
        hwaccel: HwAccel::Disabled,
        ..Default::default()
    };
    let samples = extract_frames_advanced(path, &opts)?;
    Ok(samples.into_iter().map(|s| s.phash).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_media_fps_explicit_priority() {
        let dummy = Path::new("non_existent_movie.mkv");
        assert_eq!(detect_media_fps(dummy, Some(29.97)), 29.97);
        assert_eq!(detect_media_fps(dummy, Some(60.0)), 60.0);
    }

    #[test]
    fn test_detect_media_fps_stem_heuristics() {
        let nts_movie = Path::new("Show.S01E01.1080p.WEB-DL.mkv");
        assert_eq!(detect_media_fps(nts_movie, None), NTSC_FILM_FPS);

        let pal_movie = Path::new("BBC.Documentary.576i.PAL.mkv");
        assert_eq!(detect_media_fps(pal_movie, None), PAL_FPS);

        let explicit_stem = Path::new("custom_clip.29.97fps.mkv");
        assert!((detect_media_fps(explicit_stem, None) - 29.97).abs() < 1e-4);

        let direct_fps_stem = Path::new("23.976fps.mkv");
        assert!((detect_media_fps(direct_fps_stem, None) - 23.976).abs() < 1e-4);
    }

    #[test]
    fn test_detect_media_fps_default_baseline() {
        let unknown = Path::new("unnamed_clip.mkv");
        assert_eq!(detect_media_fps(unknown, None), DEFAULT_BASELINE_FPS);
    }

    #[test]
    fn test_parse_ffprobe_json_output_fractional() {
        let json = br#"{
            "streams": [{"r_frame_rate": "24000/1001", "duration": "120.5"}],
            "format": {"duration": "120.5"}
        }"#;
        let meta = parse_ffprobe_json_output(json).expect("Should parse json");
        assert!((meta.fps - 23.976023976).abs() < 1e-5);
        assert!((meta.duration - 120.5).abs() < 1e-5);
    }

    #[test]
    fn test_parse_ffprobe_json_output_invalid() {
        let json = b"not json";
        assert!(parse_ffprobe_json_output(json).is_err());
    }

    #[test]
    fn test_probe_video_invalid_command_returns_error() {
        let dummy = Path::new("dummy.mkv");
        let result = probe_video_with_cmd(dummy, "non_existent_binary_xyz", 1);
        assert!(result.is_err());
    }
}
