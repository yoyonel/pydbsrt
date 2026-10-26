//! SubRip (.srt) and SubStation Alpha (.ass/.ssa) subtitle parser, time-shifter, and serializer.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;

pub const MILLIS_PER_HOUR: i64 = 3_600_000;
pub const MILLIS_PER_MINUTE: i64 = 60_000;
pub const MILLIS_PER_SECOND: i64 = 1_000;
pub const MILLIS_PER_CENTISECOND: i64 = 10;
pub const SECONDS_PER_MINUTE: i64 = 60;
pub const SECONDS_PER_HOUR: i64 = 3_600;
pub const ASS_DIALOGUE_FIELD_COUNT: usize = 10;
pub const ASS_START_TIME_FIELD_INDEX: usize = 1;
pub const ASS_END_TIME_FIELD_INDEX: usize = 2;
pub const ASS_TEXT_FIELD_INDEX: usize = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCue {
    pub index: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Parses "HH:MM:SS,mmm" or "HH:MM:SS.mmm" into milliseconds with strict format validation.
pub fn parse_timecode(tc: &str) -> Result<i64> {
    let tc = tc.trim();
    let (hms_part, ms_part) = if let Some(idx) = tc.rfind([',', '.']) {
        (&tc[..idx], &tc[idx + 1..])
    } else {
        bail!(
            "Invalid timecode format: '{}' (missing millisecond separator ',' or '.')",
            tc
        );
    };

    let hms_parts: Vec<&str> = hms_part.split(':').collect();
    if hms_parts.len() != 3 {
        bail!(
            "Invalid timecode format: '{}' (expected HH:MM:SS[,.]mmm)",
            tc
        );
    }

    let h: i64 = hms_parts[0].parse().context("Invalid hours in timecode")?;
    let m: i64 = hms_parts[1]
        .parse()
        .context("Invalid minutes in timecode")?;
    let s: i64 = hms_parts[2]
        .parse()
        .context("Invalid seconds in timecode")?;

    if m >= 60 || s >= 60 || h < 0 || m < 0 || s < 0 {
        bail!(
            "Timecode '{}' has out-of-range minutes ({}) or seconds ({})",
            tc,
            m,
            s
        );
    }

    if ms_part.is_empty() || ms_part.len() > 3 || !ms_part.chars().all(|c| c.is_ascii_digit()) {
        bail!("Invalid milliseconds in timecode: '{}'", tc);
    }

    let ms_raw: i64 = ms_part.parse().context("Invalid milliseconds")?;
    let ms = match ms_part.len() {
        1 => ms_raw * 100,
        2 => ms_raw * 10,
        3 => ms_raw,
        _ => unreachable!(),
    };

    Ok((h * SECONDS_PER_HOUR + m * SECONDS_PER_MINUTE + s) * MILLIS_PER_SECOND + ms)
}

/// Parses ASS/SSA timecode "H:MM:SS.CC" or "H:MM:SS.mmm" into milliseconds.
pub fn parse_ass_timecode(tc: &str) -> Result<i64> {
    let tc = tc.trim();
    let parts: Vec<&str> = tc.split(':').collect();
    if parts.len() != 3 {
        return Err(anyhow!("Invalid ASS timecode format: '{}'", tc));
    }
    let h: i64 = parts[0].parse().context("Invalid hours in ASS timecode")?;
    let m: i64 = parts[1]
        .parse()
        .context("Invalid minutes in ASS timecode")?;
    let sec_cs: Vec<&str> = parts[2].split('.').collect();
    if sec_cs.len() != 2 {
        return Err(anyhow!("Invalid ASS seconds.centiseconds: '{}'", parts[2]));
    }
    let s: i64 = sec_cs[0]
        .parse()
        .context("Invalid seconds in ASS timecode")?;
    let cs_str = sec_cs[1].trim();
    let ms: i64 = match cs_str.len() {
        1 => cs_str.parse::<i64>().context("Invalid single-digit cs")? * 100,
        2 => cs_str.parse::<i64>().context("Invalid two-digit cs")? * MILLIS_PER_CENTISECOND,
        3 => cs_str.parse::<i64>().context("Invalid three-digit ms")?,
        _ => cs_str
            .get(..3)
            .unwrap_or("0")
            .parse::<i64>()
            .context("Invalid long ms")?,
    };
    Ok(h * MILLIS_PER_HOUR + m * MILLIS_PER_MINUTE + s * MILLIS_PER_SECOND + ms)
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

        if cues.is_empty()
            && (normalized.contains("[Script Info]") || normalized.contains("Dialogue:"))
        {
            return Self::parse_ass(&normalized);
        }

        Ok(Self { cues })
    }

    /// Parses ASS/SSA subtitle dialogue events into standard SubtitleCues.
    pub fn parse_ass(content: &str) -> Result<Self> {
        let mut cues = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("Dialogue:") {
                let parts: Vec<&str> = rest.splitn(ASS_DIALOGUE_FIELD_COUNT, ',').collect();
                if parts.len() < ASS_DIALOGUE_FIELD_COUNT {
                    continue;
                }
                let start_str = parts[ASS_START_TIME_FIELD_INDEX].trim();
                let end_str = parts[ASS_END_TIME_FIELD_INDEX].trim();
                let raw_text = parts[ASS_TEXT_FIELD_INDEX].trim();

                let start_ms = parse_ass_timecode(start_str)?;
                let end_ms = parse_ass_timecode(end_str)?;

                // Strip ASS style overrides (e.g. {\an8}, {\b1})
                let mut text = String::new();
                let mut in_tag = false;
                for ch in raw_text.chars() {
                    if ch == '{' {
                        in_tag = true;
                    } else if ch == '}' {
                        in_tag = false;
                    } else if !in_tag {
                        text.push(ch);
                    }
                }
                let cleaned_text = text
                    .replace("\\N", "\n")
                    .replace("\\n", "\n")
                    .trim()
                    .to_string();
                if !cleaned_text.is_empty() {
                    cues.push(SubtitleCue {
                        index: (cues.len() + 1) as u32,
                        start_ms,
                        end_ms,
                        text: cleaned_text,
                    });
                }
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
    fn test_parse_ass_subtitles_auto_fallback() {
        let raw_ass = r#"[Script Info]
Title: Test ASS
ScriptType: v4.00+

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:02.54,0:00:04.17,Default,,0000,0000,0000,,Amenez le prisonnier !
Dialogue: 0,0:00:06.00,0:00:10.46,Default,,0000,0000,0000,,Cet homme est accusé de conspiration\Navec les Valdrogiens.
Dialogue: 0,0:00:10.80,0:00:13.13,Default,,0000,0000,0000,,{\an8}Mérite-t-il notre indulgence ?
"#;
        let srt = SubtitleFile::parse(raw_ass).expect("Failed to parse ASS content");
        assert_eq!(srt.cues.len(), 3);
        assert_eq!(srt.cues[0].start_ms, 2540);
        assert_eq!(srt.cues[0].end_ms, 4170);
        assert_eq!(srt.cues[0].text, "Amenez le prisonnier !");

        assert_eq!(srt.cues[1].start_ms, 6000);
        assert_eq!(srt.cues[1].end_ms, 10460);
        assert_eq!(
            srt.cues[1].text,
            "Cet homme est accusé de conspiration\navec les Valdrogiens."
        );

        assert_eq!(srt.cues[2].start_ms, 10800);
        assert_eq!(srt.cues[2].end_ms, 13130);
        assert_eq!(srt.cues[2].text, "Mérite-t-il notre indulgence ?");
    }

    #[test]
    fn test_parse_timecode_strict_format() {
        assert_eq!(parse_timecode("00:00:01,000").unwrap(), 1000);
        assert_eq!(parse_timecode("00:00:01.500").unwrap(), 1500);
        assert_eq!(
            parse_timecode("01:02:03,456").unwrap(),
            (SECONDS_PER_HOUR + 2 * SECONDS_PER_MINUTE + 3) * MILLIS_PER_SECOND + 456
        );
        assert_eq!(parse_timecode("00:00:01,5").unwrap(), 1500);
        assert_eq!(parse_timecode("00:00:01,50").unwrap(), 1500);
    }

    #[test]
    fn test_parse_timecode_rejects_invalid_formats() {
        assert!(parse_timecode("00:00.01,500").is_err());
        assert!(parse_timecode("00,00.01:500").is_err());
        assert!(parse_timecode("00:00:01").is_err());
        assert!(parse_timecode("00:00:01:500").is_err());
        assert!(parse_timecode("00:60:01,500").is_err());
        assert!(parse_timecode("00:00:60,500").is_err());
        assert!(parse_timecode("00:00:01,abcd").is_err());
    }
}
