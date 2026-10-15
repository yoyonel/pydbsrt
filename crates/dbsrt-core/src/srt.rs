//! SubRip (.srt) subtitle parser, time-shifter, and serializer.

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCue {
    pub index: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Parses "HH:MM:SS,mmm" or "HH:MM:SS.mmm" into milliseconds.
pub fn parse_timecode(tc: &str) -> Result<i64> {
    let tc = tc.trim();
    let parts: Vec<&str> = tc.split([':', ',', '.']).collect();
    if parts.len() != 4 {
        return Err(anyhow!("Invalid timecode format: '{}'", tc));
    }
    let h: i64 = parts[0].parse().context("Invalid hours")?;
    let m: i64 = parts[1].parse().context("Invalid minutes")?;
    let s: i64 = parts[2].parse().context("Invalid seconds")?;
    let ms: i64 = parts[3].parse().context("Invalid milliseconds")?;
    Ok((h * 3600 + m * 60 + s) * 1000 + ms)
}

/// Formats milliseconds into "HH:MM:SS,mmm".
pub fn format_timecode(ms: i64) -> String {
    let ms = ms.max(0);
    let total_s = ms / 1000;
    let millis = ms % 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, millis)
}

pub struct SubtitleFile {
    pub cues: Vec<SubtitleCue>,
}

impl SubtitleFile {
    /// Reads and parses an SRT file.
    pub fn open(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read subtitle file {:?}", path))?;
        Self::parse(&content)
    }

    /// Parses SRT content from string.
    pub fn parse(content: &str) -> Result<Self> {
        let mut cues = Vec::new();
        let normalized = content.replace("\r\n", "\n");
        let blocks = normalized.split("\n\n");

        for block in blocks {
            let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.len() < 2 {
                continue;
            }

            let index: u32 = lines[0].trim().parse().unwrap_or(cues.len() as u32 + 1);

            let time_line = lines[1];
            if let Some((start_str, end_str)) = time_line.split_once("-->") {
                let start_ms = parse_timecode(start_str)?;
                let end_ms = parse_timecode(end_str)?;
                let text = lines[2..].join("\n");
                cues.push(SubtitleCue {
                    index,
                    start_ms,
                    end_ms,
                    text,
                });
            }
        }

        Ok(Self { cues })
    }

    /// Shifts all cues by a given offset in milliseconds.
    pub fn shift(&mut self, offset_ms: f64) {
        let delta = offset_ms.round() as i64;
        for cue in &mut self.cues {
            cue.start_ms = (cue.start_ms + delta).max(0);
            cue.end_ms = (cue.end_ms + delta).max(0);
        }
    }

    /// Saves the shifted subtitles to file.
    pub fn save(&self, path: &Path) -> Result<()> {
        fs::write(path, self.to_string())
            .with_context(|| format!("Failed to save subtitle file to {:?}", path))?;
        Ok(())
    }
}

impl std::fmt::Display for SubtitleFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, cue) in self.cues.iter().enumerate() {
            writeln!(f, "{}", i + 1)?;
            writeln!(
                f,
                "{} --> {}",
                format_timecode(cue.start_ms),
                format_timecode(cue.end_ms)
            )?;
            writeln!(f, "{}\n", cue.text)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srt_roundtrip() {
        let raw = "1\n00:00:01,000 --> 00:00:04,000\nHello world\n\n2\n00:00:05,500 --> 00:00:08,200\nSecond line\n\n";
        let mut srt = SubtitleFile::parse(raw).unwrap();
        assert_eq!(srt.cues.len(), 2);
        assert_eq!(srt.cues[0].start_ms, 1000);
        assert_eq!(srt.cues[0].end_ms, 4000);

        srt.shift(1500.0);
        assert_eq!(srt.cues[0].start_ms, 2500);
        assert_eq!(srt.cues[0].end_ms, 5500);

        let out = srt.to_string();
        assert!(out.contains("00:00:02,500 --> 00:00:05,500"));
    }
}
