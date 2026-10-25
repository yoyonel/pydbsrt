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

/// Decodes raw subtitle bytes into a clean UTF-8 string, handling BOM and detecting
/// legacy encodings (e.g. Windows-1252, ISO-8859-1, ISO-8859-15, UTF-16LE/BE).
pub fn decode_subtitle_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    // 1. Check for byte-order mark (BOM) for UTF-8, UTF-16LE, UTF-16BE
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (cow, _) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        return cow.into_owned();
    }

    // 2. Strict UTF-8 validation (zero-copy fast-path)
    if let Ok(utf8_str) = std::str::from_utf8(bytes) {
        return utf8_str.to_string();
    }

    // 3. Fallback to chardetng statistical encoding detector
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);
    let (cow, _) = encoding.decode_without_bom_handling(bytes);
    cow.into_owned()
}

pub struct SubtitleFile {
    pub cues: Vec<SubtitleCue>,
}

impl SubtitleFile {
    /// Reads and parses an SRT file, supporting UTF-8 and legacy encodings gracefully.
    pub fn open(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("Failed to read subtitle file {:?}", path))?;
        let content = decode_subtitle_bytes(&bytes);
        Self::parse(&content)
    }

    /// Parses raw bytes into SubtitleFile.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let content = decode_subtitle_bytes(bytes);
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
    /// Cues that end before t=0s are pruned, and remaining cues are re-indexed.
    pub fn shift(&mut self, offset_ms: f64) {
        let delta = offset_ms.round() as i64;
        self.cues.retain_mut(|cue| {
            let new_end = cue.end_ms + delta;
            if new_end <= 0 {
                return false;
            }
            let new_start = (cue.start_ms + delta).max(0);
            cue.start_ms = new_start;
            cue.end_ms = new_end;
            true
        });
        for (i, cue) in self.cues.iter_mut().enumerate() {
            cue.index = (i + 1) as u32;
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

    #[test]
    fn test_decode_subtitle_bytes_windows1252_accents() {
        // "Désolée, y a rien qui me vient..." in Windows-1252 / ISO-8859-1
        // 'é' is 0xE9
        let latin1_bytes = b"1\r\n00:00:01,000 --> 00:00:04,000\r\nD\xe9sol\xe9e, y a rien qui me vient...\r\n\r\n";
        let srt = SubtitleFile::from_bytes(latin1_bytes).expect("Failed to parse Latin-1 bytes");
        assert_eq!(srt.cues.len(), 1);
        assert_eq!(srt.cues[0].text, "Désolée, y a rien qui me vient...");
        assert!(!srt.cues[0].text.contains('\u{FFFD}'));
    }

    #[test]
    fn test_decode_subtitle_bytes_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(
            "1\n00:00:01,000 --> 00:00:04,000\nÇa fonctionne parfaitement !\n\n".as_bytes(),
        );
        let srt = SubtitleFile::from_bytes(&bytes).expect("Failed to parse UTF-8 with BOM");
        assert_eq!(srt.cues.len(), 1);
        assert_eq!(srt.cues[0].text, "Ça fonctionne parfaitement !");
    }

    #[test]
    fn test_decode_subtitle_bytes_utf16le_bom() {
        let text = "1\n00:00:01,000 --> 00:00:04,000\nSubtitles in UTF-16LE\n\n";
        let mut bytes = vec![0xFF, 0xFE]; // UTF-16LE BOM
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let srt = SubtitleFile::from_bytes(&bytes).expect("Failed to parse UTF-16LE with BOM");
        assert_eq!(srt.cues.len(), 1);
        assert_eq!(srt.cues[0].text, "Subtitles in UTF-16LE");
    }

    #[test]
    fn test_open_euphoria_latin1_srt() {
        let srt_path = Path::new("../../data/SRT/Euphoria.US.S01E02.1080p.WEBRip.x265-RARBG.srt");
        if srt_path.exists() {
            let srt = SubtitleFile::open(srt_path).expect("Failed to open Euphoria SRT");
            let target_cue = srt
                .cues
                .iter()
                .find(|c| c.text.contains("rien qui me vient"));
            assert!(
                target_cue.is_some(),
                "Expected cue 'rien qui me vient' found"
            );
            let cue_text = &target_cue.unwrap().text;
            assert_eq!(cue_text, "Désolée, y a rien qui me vient...");
            assert!(!cue_text.contains('\u{FFFD}'));
        }
    }
}
