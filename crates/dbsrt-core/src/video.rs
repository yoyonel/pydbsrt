//! Video probing and frame extraction via FFmpeg/FFprobe.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub duration: f64,
    pub fps: f64,
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

/// Probes a video file using ffprobe.
pub fn probe_video(path: &Path) -> Result<VideoMetadata> {
    let output = Command::new("ffprobe")
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
        .output()
        .with_context(|| format!("Failed to execute ffprobe on {:?}", path))?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout)
        .with_context(|| "Failed to parse ffprobe json output")?;

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

/// Extracts a stream of 32x32 grayscale frames using ffmpeg and computes pHash on each.
pub fn extract_frames_phash(path: &Path, start_sec: f64, nb_frames: usize) -> Result<Vec<u64>> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostats",
            "-nostdin",
            "-ss",
            &format!("{:.3}", start_sec),
            "-i",
        ])
        .arg(path)
        .args([
            "-frames:v",
            &nb_frames.to_string(),
            "-vf",
            "scale=width=32:height=32",
            "-pix_fmt",
            "gray",
            "-f",
            "rawvideo",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to spawn ffmpeg on {:?}", path))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Cannot capture ffmpeg stdout"))?;
    let mut buf = [0u8; 1024];
    let mut hashes = Vec::with_capacity(nb_frames);

    loop {
        match stdout.read_exact(&mut buf) {
            Ok(()) => {
                let h = crate::phash::phash_32x32(&buf);
                hashes.push(h);
                if hashes.len() >= nb_frames {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }

    let _ = child.wait();
    Ok(hashes)
}
