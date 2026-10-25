//! Certified Ground-Truth Couplings `(Video Frames / pHash <-> Subtitle File)`
//! and relational data model shared between PostgreSQL (`DbClient`) and the
//! local Zero-DB JSON registry (`couplings.json`).

use crate::lang_status::LangStatus;
use crate::srt::SubtitleFile;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn default_lang_status_for_legacy() -> LangStatus {
    LangStatus::Inferred("und".to_string())
}

/// A single cue's frame-offset anchor on its reference media stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubFrameAnchor {
    /// 1-based SRT sequence index
    pub index: i32,
    /// Inclusive start frame offset on the reference media (`0..frame_count`)
    pub start_frame_offset: i32,
    /// Inclusive end frame offset on the reference media (`start_frame_offset..frame_count`)
    pub end_frame_offset: i32,
    /// Subtitle cue text
    pub text: String,
    /// Optional 64-bit pHash at `start_frame_offset` on the reference media
    pub anchor_phash: Option<u64>,
}

/// A certified subtitle track coupled to a specific reference media (`medias.id` / `media_hash`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CertifiedSubtitleRecord {
    /// 32-char hex digest of the `.srt` content
    pub subtitles_hash: String,
    /// Canonical stem or filename of the subtitle track
    pub name: String,
    /// Normalized ISO-639-1 language code (`"fr"`, `"en"`, `"es"`, etc.)
    pub lang: String,
    /// Confidence level of the subtitle language attribution
    #[serde(default = "default_lang_status_for_legacy")]
    pub lang_status: LangStatus,
    /// Relative or canonical path to the `.srt` file in `--subtitles-dir`
    pub srt_path: String,
    /// Whether this coupling has been certified as ground-truth
    pub certified: bool,
    /// Number of cues in the subtitle file
    pub cue_count: usize,
    /// Frame-offset anchors for each cue (optional in compact JSON, populated in PostgreSQL `sub_frames`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_frames: Vec<SubFrameAnchor>,
}

/// A reference media entry with its certified subtitle tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CertifiedMediaCoupling {
    /// 32-char hex digest of the reference `.phash` stream
    pub media_hash: String,
    /// Canonical name of the reference media or `.phash` stem
    pub media_name: String,
    /// Filename of the `.phash` file in `--phash-dir`
    pub phash_filename: String,
    /// Reference framerate (frames per second)
    pub fps: f64,
    /// Total number of frames in the `.phash` stream
    pub frame_count: usize,
    /// Certified subtitle tracks coupled to this reference media
    pub subtitles: Vec<CertifiedSubtitleRecord>,
}

/// Persistent Zero-DB JSON registry (`couplings.json`) mirroring the PostgreSQL
/// relational schema (`medias` <-> `subtitles` <-> `sub_frames`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CouplingRegistry {
    pub couplings: Vec<CertifiedMediaCoupling>,
}

/// Computes a 32-char hex SHA-256 digest of a slice of 64-bit pHashes (big-endian bytes).
pub fn compute_media_phash_digest(hashes: &[u64]) -> String {
    let mut hasher = Sha256::new();
    for &h in hashes {
        hasher.update(h.to_be_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in &digest {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    hex[..32].to_string()
}

/// Computes a 32-char hex SHA-256 digest of raw bytes (e.g. `.srt` file content).
pub fn compute_bytes_digest_32(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in &digest {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    hex[..32].to_string()
}

/// Builds the list of `SubFrameAnchor` entries by projecting each `SubtitleCue` timestamp
/// Validates and converts subtitle cues into `SubFrameAnchor` structs mapped
/// onto the reference media's frame grid (`fps` and `hashes`) and validates temporal bounds.
pub fn build_and_validate_sub_frames(
    sub_file: &SubtitleFile,
    hashes: &[u64],
    fps: f64,
) -> Result<Vec<SubFrameAnchor>> {
    build_and_validate_sub_frames_opts(sub_file, hashes, fps, false)
}

/// Validates and converts subtitle cues into `SubFrameAnchor` structs with optional out-of-bounds relaxation.
pub fn build_and_validate_sub_frames_opts(
    sub_file: &SubtitleFile,
    hashes: &[u64],
    fps: f64,
    allow_oob: bool,
) -> Result<Vec<SubFrameAnchor>> {
    if hashes.is_empty() {
        bail!("Reference pHash stream is empty (0 frames)");
    }
    if fps <= 0.0 {
        bail!("Invalid framerate: {}", fps);
    }
    if sub_file.cues.is_empty() {
        bail!("Subtitle file contains 0 cues");
    }

    let max_frame_idx = (hashes.len() - 1) as i32;
    // Allow tolerance (e.g. +15 seconds past the last frame) for end credits / preview cues
    let tolerance_frames = (fps * 15.0).ceil() as i32;
    let mut anchors = Vec::with_capacity(sub_file.cues.len());

    for cue in &sub_file.cues {
        if cue.end_ms < cue.start_ms {
            bail!(
                "Cue #{} has end_ms ({}) < start_ms ({})",
                cue.index,
                cue.end_ms,
                cue.start_ms
            );
        }
        let start_frame = ((cue.start_ms as f64 / 1000.0) * fps).round() as i32;
        let end_frame = ((cue.end_ms as f64 / 1000.0) * fps).round() as i32;

        if start_frame > max_frame_idx + tolerance_frames && !allow_oob {
            bail!(
                "Cue #{} start_ms ({} ms -> frame {}) exceeds media duration ({} frames at {} fps)",
                cue.index,
                cue.start_ms,
                start_frame,
                hashes.len(),
                fps
            );
        }

        let clamped_start = start_frame.clamp(0, max_frame_idx);
        let clamped_end = end_frame.clamp(clamped_start, max_frame_idx);
        let anchor_phash = hashes.get(clamped_start as usize).copied();

        anchors.push(SubFrameAnchor {
            index: cue.index as i32,
            start_frame_offset: clamped_start,
            end_frame_offset: clamped_end,
            text: cue.text.clone(),
            anchor_phash,
        });
    }

    Ok(anchors)
}

impl CouplingRegistry {
    /// Default filename inside `--subtitles-dir`.
    pub const REGISTRY_FILENAME: &'static str = "couplings.json";

    /// Resolves the path to `couplings.json` inside `subtitles_dir`.
    pub fn default_path(subtitles_dir: &Path) -> PathBuf {
        subtitles_dir.join(Self::REGISTRY_FILENAME)
    }

    /// Loads `CouplingRegistry` from a JSON file.
    ///
    /// - Returns `Ok(Self::default())` if the file does not exist.
    /// - Returns `Err(...)` if the file exists but fails to read or parse (invalid JSON).
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read coupling registry at {:?}", path))?;
        let reg: Self = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse coupling registry JSON at {:?}", path))?;
        Ok(reg)
    }

    /// Saves `CouplingRegistry` atomically to `path` using a temporary file and rename.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
        fs::write(&tmp_path, json).with_context(|| {
            format!(
                "Failed to write temporary coupling registry to {:?}",
                tmp_path
            )
        })?;
        fs::rename(&tmp_path, path).with_context(|| {
            format!("Failed to rename temporary coupling registry to {:?}", path)
        })?;
        Ok(())
    }

    /// Upserts a certified `(media <-> subtitle)` coupling into the registry.
    pub fn upsert_coupling(
        &mut self,
        media_hash: String,
        media_name: String,
        phash_filename: String,
        fps: f64,
        frame_count: usize,
        sub_record: CertifiedSubtitleRecord,
    ) {
        let entry = if let Some(existing) = self.couplings.iter_mut().find(|c| {
            c.media_hash == media_hash
                || c.phash_filename == phash_filename
                || c.media_name == media_name
        }) {
            existing.media_hash = media_hash;
            existing.phash_filename = phash_filename;
            existing.fps = fps;
            existing.frame_count = frame_count;
            existing
        } else {
            self.couplings.push(CertifiedMediaCoupling {
                media_hash,
                media_name,
                phash_filename,
                fps,
                frame_count,
                subtitles: Vec::new(),
            });
            self.couplings.last_mut().unwrap()
        };

        if let Some(existing_sub) = entry.subtitles.iter_mut().find(|s| {
            s.subtitles_hash == sub_record.subtitles_hash
                || (s.lang == sub_record.lang && s.name == sub_record.name)
        }) {
            *existing_sub = sub_record;
        } else {
            entry.subtitles.push(sub_record);
        }
    }

    /// Finds a media coupling matching a recognized `.phash` filename, media name, clean title, or media hash.
    pub fn find_by_matched_media(&self, matched_media: &str) -> Option<&CertifiedMediaCoupling> {
        let clean = matched_media
            .strip_suffix(".phash")
            .unwrap_or(matched_media);
        let norm_matched = matched_media.replace(['_', '.', '-'], " ").to_lowercase();
        let norm_clean = norm_matched.trim();

        self.couplings.iter().find(|c| {
            if c.phash_filename == matched_media
                || c.media_name == matched_media
                || c.media_name == clean
                || c.phash_filename.strip_suffix(".phash") == Some(clean)
                || c.media_hash == matched_media
            {
                return true;
            }
            if !norm_clean.is_empty() {
                let norm_c_name = c.media_name.replace(['_', '.', '-'], " ").to_lowercase();
                let norm_c_phash = c
                    .phash_filename
                    .replace(['_', '.', '-'], " ")
                    .to_lowercase();
                if norm_c_name.starts_with(norm_clean)
                    || norm_c_phash.starts_with(norm_clean)
                    || norm_clean.starts_with(&norm_c_name)
                    || norm_clean.starts_with(&norm_c_phash)
                {
                    return true;
                }
            }
            false
        })
    }

    /// Finds a subtitle record by its full or partial SHA-256 hash.
    pub fn find_subtitle_by_hash(
        &self,
        hash: &str,
    ) -> Option<(&CertifiedMediaCoupling, &CertifiedSubtitleRecord)> {
        let clean = hash
            .trim_start_matches("sub:sha256:")
            .trim_start_matches("sha256:")
            .trim();
        for coupling in &self.couplings {
            for sub in &coupling.subtitles {
                if sub.subtitles_hash.eq_ignore_ascii_case(clean)
                    || sub.subtitles_hash.starts_with(clean)
                    || clean.starts_with(&sub.subtitles_hash)
                {
                    return Some((coupling, sub));
                }
            }
        }
        None
    }

    /// Returns true if a coupling matching `media_hash` and `subtitles_hash` is already certified.
    pub fn is_already_certified(&self, media_hash: &str, subtitles_hash: &str) -> bool {
        self.couplings.iter().any(|c| {
            c.media_hash == media_hash
                && c.subtitles
                    .iter()
                    .any(|s| s.subtitles_hash == subtitles_hash && s.certified)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coupling_registry_roundtrip_and_lookup() {
        let srt_raw = "1\n00:00:01,000 --> 00:00:03,000\nBonjour\n\n2\n00:00:04,000 --> 00:00:06,000\nMonde\n";
        let sub_file = SubtitleFile::parse(srt_raw).unwrap();
        let hashes = vec![0xAAAA_BBBB_CCCC_DDDDu64; 250]; // 10s at 25fps
        let anchors = build_and_validate_sub_frames(&sub_file, &hashes, 25.0).unwrap();
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].start_frame_offset, 25);
        assert_eq!(anchors[0].end_frame_offset, 75);

        let mut reg = CouplingRegistry::default();
        let media_hash = compute_media_phash_digest(&hashes);
        let sub_hash = compute_bytes_digest_32(srt_raw.as_bytes());

        reg.upsert_coupling(
            media_hash.clone(),
            "sample_movie.mp4.25.0fps".to_string(),
            "sample_movie.mp4.25.0fps.phash".to_string(),
            25.0,
            hashes.len(),
            CertifiedSubtitleRecord {
                subtitles_hash: sub_hash,
                name: "sample_movie.fr.srt".to_string(),
                lang: "fr".to_string(),
                lang_status: LangStatus::Detected("fr".to_string()),
                srt_path: "sample_movie.fr.srt".to_string(),
                certified: true,
                cue_count: 2,
                sub_frames: anchors,
            },
        );

        let found = reg
            .find_by_matched_media("sample_movie.mp4.25.0fps.phash")
            .expect("Must find coupled media");
        assert_eq!(found.subtitles.len(), 1);
        assert_eq!(found.subtitles[0].lang, "fr");
    }

    #[test]
    fn test_is_already_certified_behavior() {
        let mut reg = CouplingRegistry::default();
        let media_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sub_hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let different_sub_hash = "cccccccccccccccccccccccccccccccc";

        reg.upsert_coupling(
            media_hash.to_string(),
            "test_movie".to_string(),
            "test_movie.phash".to_string(),
            24.0,
            100,
            CertifiedSubtitleRecord {
                subtitles_hash: sub_hash.to_string(),
                name: "test_movie.fr.srt".to_string(),
                lang: "fr".to_string(),
                lang_status: LangStatus::Detected("fr".to_string()),
                srt_path: "test_movie.fr.srt".to_string(),
                certified: true,
                cue_count: 5,
                sub_frames: Vec::new(),
            },
        );

        // Identical media and subtitle hash -> already certified
        assert!(
            reg.is_already_certified(media_hash, sub_hash),
            "Expected true for identical media and subtitle hash"
        );

        // Same media, different subtitle hash -> not certified yet
        assert!(
            !reg.is_already_certified(media_hash, different_sub_hash),
            "Expected false for different subtitle hash"
        );

        // Unknown media -> not certified
        assert!(
            !reg.is_already_certified("unknown_media_hash", sub_hash),
            "Expected false for unknown media hash"
        );
    }

    #[test]
    fn test_legacy_coupling_without_lang_status_deserialization() {
        let json = r#"{
            "subtitles_hash": "abc",
            "name": "legacy.srt",
            "lang": "en",
            "srt_path": "legacy.srt",
            "certified": true,
            "cue_count": 10
        }"#;
        let record: CertifiedSubtitleRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.lang, "en");
        assert_eq!(record.lang_status, LangStatus::Inferred("und".to_string()));
    }

    #[test]
    fn test_coupling_registry_persistence_and_queries() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_coupling_reg_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let default_path = CouplingRegistry::default_path(&temp_dir);
        assert_eq!(default_path, temp_dir.join("couplings.json"));

        let loaded_default = CouplingRegistry::load_or_default(&default_path).unwrap();
        assert!(loaded_default.couplings.is_empty());

        let mut reg = CouplingRegistry::default();
        let sub = CertifiedSubtitleRecord {
            subtitles_hash: "1234567890abcdef1234567890abcdef".to_string(),
            name: "movie.fr.srt".to_string(),
            lang: "fr".to_string(),
            lang_status: LangStatus::Confirmed("fr".to_string()),
            srt_path: "movie.fr.srt".to_string(),
            certified: true,
            cue_count: 10,
            sub_frames: vec![],
        };

        reg.upsert_coupling(
            "hash1".to_string(),
            "movie".to_string(),
            "movie.phash".to_string(),
            24.0,
            100,
            sub.clone(),
        );

        // Upsert again to exercise existing coupling update branch
        reg.upsert_coupling(
            "hash1".to_string(),
            "movie".to_string(),
            "movie.phash".to_string(),
            24.0,
            100,
            sub.clone(),
        );

        reg.save(&default_path).unwrap();
        let loaded = CouplingRegistry::load_or_default(&default_path).unwrap();
        assert_eq!(loaded.couplings.len(), 1);

        let by_hash = loaded.find_subtitle_by_hash("sub:sha256:1234567890abcdef");
        assert!(by_hash.is_some());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
