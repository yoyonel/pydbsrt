//! Audio probing, decoding, and Haitsma-Kalker perceptual hashing via FFmpeg.
//!
//! Streams decoded raw mono f32le audio samples directly from an FFmpeg stdout pipe
//! into a sliding-window Haitsma-Kalker differential spectral hash engine.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::ahash::{
    AhashFile, AhashHeader, HaitsmaKalkerExtractor, HaitsmaScratchpad, DEFAULT_FRAME_SIZE,
    DEFAULT_HOP_SIZE, DEFAULT_SAMPLE_RATE, DEFAULT_SPECTRAL_LAG, NUM_SPECTRAL_BANDS,
};
use crate::ffmpeg_input::{
    apply_ftp_credentials, display_input, ffmpeg_decode_threads, http_reconnect_args,
    redact_secrets,
};
use crate::video::resolve_ffmpeg_cpu;

/// Default execution timeout in seconds for spawning FFmpeg audio stream.
pub const DEFAULT_AUDIO_FFMPEG_TIMEOUT_SECS: u64 = 30;

/// Default sample read buffer size in f32 elements (1024 samples = 4096 bytes).
pub const AUDIO_SAMPLE_CHUNK_SIZE: usize = 1024;

/// Polling interval for child process status checks.
pub const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Options governing audio extraction and perceptual hashing.
#[derive(Debug, Clone)]
pub struct AudioExtractionOptions {
    /// Audio sampling rate in Hz (default: 5512).
    pub sample_rate: u32,
    /// FFT frame analysis size in samples (default: 2048).
    pub frame_size: usize,
    /// Hop size in samples between consecutive frames (default: 64 for ~86.1 fps, 230 for ~24.0 fps).
    pub hop_size: usize,
    /// Optional seek offset in seconds from start of media.
    pub start_sec: f64,
    /// Optional duration ceiling in seconds to process.
    pub duration_sec: Option<f64>,
    /// Optional path to custom FFmpeg binary.
    pub custom_ffmpeg: Option<PathBuf>,
    /// Optional relative audio stream index (`k` in `0:a:k`) to decode.
    ///
    /// `None` keeps FFmpeg automatic stream selection (no `-map`), which preserves
    /// bit-for-bit identity with every `.ahash` extracted before this option existed.
    /// `Some(k)` injects `-map 0:a:<k>`.
    pub audio_stream: Option<usize>,
}

impl Default for AudioExtractionOptions {
    fn default() -> Self {
        Self {
            sample_rate: DEFAULT_SAMPLE_RATE,
            frame_size: DEFAULT_FRAME_SIZE,
            hop_size: DEFAULT_HOP_SIZE,
            start_sec: 0.0,
            duration_sec: None,
            custom_ffmpeg: None,
            audio_stream: None,
        }
    }
}

/// Builds the FFmpeg Command configured to stream mono raw f32le PCM to stdout.
///
/// # Arguments
/// * `path` - Path or URI to input media.
/// * `options` - Configured extraction options.
///
/// # Returns
/// Configured `Command` ready to spawn.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Resolves FFmpeg binary (custom or fallback CPU).
/// 2. Injects quiet non-interactive flags (`-hide_banner`, `-nostats`, `-nostdin`).
/// 3. Injects seeking flags (`-ss`) and duration limits (`-t`) if configured.
/// 4. Injects `-map 0:a:<k>` only when `options.audio_stream == Some(k)`; `None` keeps
///    FFmpeg automatic audio selection (bit-identical with legacy `.ahash` files).
/// 5. Configures audio downmix (`-vn`, `-ac 1`, `-ar <sample_rate>`, `-f f32le`, `-`).
pub fn build_ffmpeg_audio_command(path: &Path, options: &AudioExtractionOptions) -> Command {
    let binary = options
        .custom_ffmpeg
        .clone()
        .unwrap_or_else(resolve_ffmpeg_cpu);

    let mut cmd = Command::new(binary);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostats", "-nostdin"]);

    let input_str = path.to_string_lossy();
    if input_str.starts_with("http://") || input_str.starts_with("https://") {
        cmd.args(http_reconnect_args(&input_str));
    }

    apply_ftp_credentials(&mut cmd, path);

    if options.start_sec > 0.0 {
        cmd.args(["-ss", &format!("{:.3}", options.start_sec)]);
    }

    cmd.args(["-i", &input_str]);

    if let Some(audio_index) = options.audio_stream {
        cmd.args(["-map", &format!("0:a:{audio_index}")]);
    }

    if let Some(duration) = options.duration_sec {
        if duration > 0.0 {
            cmd.args(["-t", &format!("{:.3}", duration)]);
        }
    }

    cmd.args([
        "-threads",
        &ffmpeg_decode_threads(),
        "-vn",
        "-ac",
        "1",
        "-ar",
        &options.sample_rate.to_string(),
        "-f",
        "f32le",
        "-",
    ]);

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd
}

/// Extracts Haitsma-Kalker .ahash fingerprints from an input media stream.
///
/// # Arguments
/// * `path` - Filesystem path or network stream URL.
/// * `options` - Configured audio extraction options.
///
/// # Returns
/// Complete `AhashFile` containing header and generated hashes on success.
///
/// # Errors
/// Returns `Err` if FFmpeg fails to launch, exits abnormally, or I/O error occurs.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Initializes `HaitsmaKalkerExtractor` and pre-allocated `HaitsmaScratchpad`.
/// 2. Spawns FFmpeg child process streaming f32le samples over stdout.
/// 3. Streams samples into a sliding ring buffer, advancing by `options.hop_size` samples.
/// 4. Computes 64-bit hash per hop with zero dynamic heap allocations in hot loop.
/// 5. Validates FFmpeg exit code and wraps payload into an `AhashFile`.
pub fn extract_ahash(path: &Path, options: &AudioExtractionOptions) -> Result<AhashFile> {
    let extractor = HaitsmaKalkerExtractor::new(options.sample_rate, options.frame_size)
        .with_context(|| "Failed to initialize Haitsma-Kalker extractor")?;

    let mut scratch = HaitsmaScratchpad::new(options.frame_size);

    let mut cmd = build_ffmpeg_audio_command(path, options);
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn FFmpeg for audio input {:?}",
            display_input(path)
        )
    })?;

    let stdout = child
        .stdout
        .take()
        .context("Failed to capture FFmpeg stdout pipe for audio stream")?;
    let stderr = child.stderr.take();

    let timeout = Duration::from_secs(DEFAULT_AUDIO_FFMPEG_TIMEOUT_SECS);
    let hashes = std::thread::scope(|s| -> Result<Vec<u64>> {
        let stderr_handle = s.spawn(move || {
            let mut msg = String::new();
            if let Some(mut err_pipe) = stderr {
                let _ = err_pipe.read_to_string(&mut msg);
            }
            msg
        });

        let mut stdout_pipe = stdout;
        let window_res =
            run_audio_sliding_window(&mut stdout_pipe, &extractor, &mut scratch, options);
        drop(stdout_pipe);

        if let Err(err) = window_res {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_handle.join();
            return Err(err);
        }

        let status = poll_child_with_timeout(&mut child, path, timeout)?;
        let stderr_msg = stderr_handle.join().unwrap_or_default();
        validate_child_exit_status(status, path, &stderr_msg)?;
        window_res
    })?;

    let header = AhashHeader {
        version: 1,
        sample_rate: options.sample_rate,
        frame_size: options.frame_size as u16,
        hop_size: options.hop_size as u16,
        num_bands: NUM_SPECTRAL_BANDS as u8,
        spectral_lag: DEFAULT_SPECTRAL_LAG as u8,
        frame_count: hashes.len() as u64,
        band_bounds: extractor.band_bounds,
    };

    Ok(AhashFile { header, hashes })
}

/// Executes sliding window extraction over a raw f32le audio sample reader.
///
/// # Arguments
/// * `reader` - Stream providing raw 32-bit float audio samples in little-endian.
/// * `extractor` - Initialized Haitsma-Kalker extractor.
/// * `scratch` - Mutable reusable scratchpad.
/// * `options` - Audio extraction parameters (frame size, hop size).
///
/// # Returns
/// Vector of extracted 64-bit hashes.
///
/// # Errors
/// Returns `Err` on unrecoverable stream read failure.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Reads initial `frame_size` samples into sliding window.
/// 2. Iteratively processes current window, shifts by `hop_size`, and reads `hop_size` new samples.
fn run_audio_sliding_window<R: Read>(
    reader: &mut R,
    extractor: &HaitsmaKalkerExtractor,
    scratch: &mut HaitsmaScratchpad,
    options: &AudioExtractionOptions,
) -> Result<Vec<u64>> {
    let frame_size = options.frame_size;
    let hop_size = options.hop_size;
    let mut window = vec![0.0f32; frame_size];
    let mut hashes = Vec::new();

    // 1. Fill initial window
    let initial_samples_read = read_f32_exact_or_eof(reader, &mut window)?;
    if initial_samples_read < frame_size {
        if initial_samples_read > 0 {
            // Pad remainder with zeros if audio ended prematurely
            window[initial_samples_read..].fill(0.0);
            let h = extractor.process_frame(&window, scratch);
            hashes.push(h);
        }
        return Ok(hashes);
    }

    // Process frame 0
    let h0 = extractor.process_frame(&window, scratch);
    hashes.push(h0);

    // 2. Sliding window loop
    let shift_len = frame_size - hop_size;
    let mut hop_buf = vec![0.0f32; hop_size];

    loop {
        let read_count = read_f32_exact_or_eof(reader, &mut hop_buf)?;
        if read_count == 0 {
            break;
        }

        // Shift window left by hop_size
        window.copy_within(hop_size..frame_size, 0);

        if read_count < hop_size {
            // Partial hop: copy what we have and zero-pad remainder
            window[shift_len..shift_len + read_count].copy_from_slice(&hop_buf[..read_count]);
            window[shift_len + read_count..].fill(0.0);
            let h = extractor.process_frame(&window, scratch);
            hashes.push(h);
            break;
        }

        window[shift_len..].copy_from_slice(&hop_buf);
        let h = extractor.process_frame(&window, scratch);
        hashes.push(h);
    }

    Ok(hashes)
}

/// Reads exact number of f32 samples from byte stream or returns number of samples read on EOF.
///
/// # Arguments
/// * `reader` - Byte stream source.
/// * `dest` - Destination float buffer.
///
/// # Returns
/// Number of f32 samples successfully read.
///
/// # Errors
/// Returns `Err` on underlying I/O read failure.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Reads up to `dest.len() * 4` bytes and decodes f32 values in little-endian.
fn read_f32_exact_or_eof<R: Read>(reader: &mut R, dest: &mut [f32]) -> Result<usize> {
    let byte_target = std::mem::size_of_val(dest);
    let mut raw_bytes = vec![0u8; byte_target];
    let mut total_bytes = 0;

    while total_bytes < byte_target {
        match reader.read(&mut raw_bytes[total_bytes..]) {
            Ok(0) => break, // EOF
            Ok(n) => total_bytes += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }

    let samples_read = total_bytes / std::mem::size_of::<f32>();
    for i in 0..samples_read {
        let chunk = &raw_bytes[i * 4..(i + 1) * 4];
        dest[i] = f32::from_le_bytes(chunk.try_into().unwrap());
    }

    Ok(samples_read)
}

/// Formats a secret-redacted diagnostic message for a failed FFmpeg audio process.
///
/// # Arguments
/// * `code` - Process exit status code (or `-1` if terminated by signal).
/// * `path` - Input media path or URI.
/// * `stderr_msg` - Raw captured `stderr` payload from FFmpeg.
///
/// # Returns
/// Sanitized human-readable failure string with URL userinfo and `-ftp-password` secrets masked.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Applies [`redact_secrets`] to `stderr_msg.trim()` before inspecting or formatting.
/// 2. Preserves the `"Media has no audio stream to extract"` prefix when FFmpeg reports no output
///    stream so downstream silent-media handlers classify it non-fatally.
fn format_audio_child_error(code: i32, path: &Path, stderr_msg: &str) -> String {
    let redacted = redact_secrets(stderr_msg.trim());
    if redacted.contains("does not contain any stream") || redacted.contains("matches no streams") {
        return format!(
            "Media has no audio stream to extract: {:?}",
            display_input(path)
        );
    }
    let detail = if redacted.is_empty() {
        "unknown error"
    } else {
        redacted.as_str()
    };
    format!(
        "FFmpeg audio extraction exited with status {} for {:?}: {}",
        code,
        display_input(path),
        detail
    )
}

/// Polls an FFmpeg child process until completion or kills and reaps it on timeout.
///
/// # Arguments
/// * `child` - Mutable reference to the spawned child process.
/// * `path` - Media path or URI for redacted error reporting.
/// * `timeout` - Maximum allowed wall-clock duration before termination.
///
/// # Returns
/// `Ok(ExitStatus)` once the child exits within `timeout`.
///
/// # Errors
/// Returns `Err` if `try_wait` fails or if `timeout` elapses before process exit.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Polls `child.try_wait()` at [`CHILD_POLL_INTERVAL`] steps; on timeout issues `kill()` followed
/// by `wait()` so no zombie process or open pipe writer remains.
fn poll_child_with_timeout(
    child: &mut Child,
    path: &Path,
    timeout: Duration,
) -> Result<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait().with_context(|| {
            format!(
                "Failed waiting for FFmpeg child process on {:?}",
                display_input(path)
            )
        })? {
            Some(status) => return Ok(status),
            None if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "FFmpeg audio extraction timed out after {}s for {:?}",
                    timeout.as_secs(),
                    display_input(path)
                );
            }
            None => std::thread::sleep(CHILD_POLL_INTERVAL),
        }
    }
}

/// Validates FFmpeg child exit status and formats secret-redacted diagnostics on failure.
///
/// # Arguments
/// * `status` - Completed child process [`ExitStatus`].
/// * `path` - Media path or URI for diagnostic context.
/// * `stderr_msg` - Captured `stderr` output from the child process.
///
/// # Returns
/// `Ok(())` when `status.success()` or exit code is `255` (early pipe termination).
///
/// # Errors
/// Returns `Err` formatted via [`format_audio_child_error`] on non-zero failure exit codes.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// Treats exit code `255` as acceptable early termination and delegates all other failures to
/// [`format_audio_child_error`].
fn validate_child_exit_status(status: ExitStatus, path: &Path, stderr_msg: &str) -> Result<()> {
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        // Exit code 255 typically indicates SIGINT / early pipe termination
        if code != 255 {
            bail!("{}", format_audio_child_error(code, path, stderr_msg));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn command_args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn test_build_ffmpeg_audio_command_without_audio_stream_has_no_map() {
        let opts = AudioExtractionOptions::default();
        let args = command_args(&build_ffmpeg_audio_command(Path::new("in.mkv"), &opts));
        assert!(!args.iter().any(|a| a == "-map"));
        let loglevel_pos = args
            .iter()
            .position(|a| a == "-loglevel")
            .expect("-loglevel present");
        assert_eq!(args[loglevel_pos + 1], "error");
        assert!(args.iter().any(|a| a == "-nostats"));
    }

    #[test]
    fn test_build_ffmpeg_audio_command_with_audio_stream_maps_track() {
        let opts = AudioExtractionOptions {
            audio_stream: Some(2),
            ..Default::default()
        };
        let args = command_args(&build_ffmpeg_audio_command(Path::new("in.mkv"), &opts));
        let pos = args.iter().position(|a| a == "-map").expect("-map present");
        assert_eq!(args[pos + 1], "0:a:2");
        let input_pos = args.iter().position(|a| a == "-i").expect("-i present");
        assert!(pos > input_pos, "-map must follow the input");
    }

    #[test]
    fn test_run_audio_sliding_window_mock_data() {
        let extractor = HaitsmaKalkerExtractor::new(5512, 2048).expect("Init extractor");
        let mut scratch = HaitsmaScratchpad::new(2048);

        // Generate 4096 samples of mock sine wave
        let mut raw_bytes = Vec::with_capacity(4096 * 4);
        for i in 0..4096 {
            let sample = (i as f32 * 0.1).sin();
            raw_bytes.extend_from_slice(&sample.to_le_bytes());
        }

        let mut cursor = Cursor::new(raw_bytes);
        let opts = AudioExtractionOptions {
            frame_size: 2048,
            hop_size: 64,
            ..Default::default()
        };
        let hashes = run_audio_sliding_window(&mut cursor, &extractor, &mut scratch, &opts)
            .expect("Sliding window processing");

        // Expected frames: (4096 - 2048) / 64 + 1 = 33 frames
        assert_eq!(hashes.len(), 33);
        for &h in &hashes {
            assert_ne!(h, 0);
        }
    }

    #[test]
    fn test_format_audio_child_error_redacts_secrets_and_userinfo() {
        let path = Path::new("ftp://nas_user:top_s3cret@nas.local:21/show/ep01.mkv");
        let stderr = "ffmpeg -ftp-password flag_s3cret -i ftp://nas_user:url_s3cret@nas.local:21/show/ep01.mkv: I/O error\n";
        let msg = format_audio_child_error(1, path, stderr);
        assert!(!msg.contains("top_s3cret"));
        assert!(!msg.contains("flag_s3cret"));
        assert!(!msg.contains("url_s3cret"));
        assert!(msg.contains("ftp://nas_user:***@nas.local:21/show/ep01.mkv"));
        assert!(msg.contains("-ftp-password ***"));

        let silent = format_audio_child_error(1, path, "Output file does not contain any stream\n");
        assert!(silent.contains("no audio stream"));
        assert!(!silent.contains("top_s3cret"));
    }

    #[test]
    fn test_poll_child_with_timeout_kills_stalled_process() {
        let mut child = Command::new("sleep")
            .arg("5")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let path = Path::new("ftp://nas_user:secret_pw@nas.local:21/show/stalled.mkv");
        let err = poll_child_with_timeout(&mut child, path, Duration::from_millis(120))
            .expect_err("stalled child must time out");
        let msg = err.to_string();
        assert!(msg.contains("timed out"));
        assert!(!msg.contains("secret_pw"));
        assert!(msg.contains("ftp://nas_user:***@nas.local:21/show/stalled.mkv"));
    }
}
