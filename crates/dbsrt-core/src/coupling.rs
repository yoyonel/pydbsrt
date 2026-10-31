//! Certified Ground-Truth Couplings `(Video Frames / pHash <-> Subtitle File)`
//! and relational data model shared between PostgreSQL (`DbClient`) and the
//! local Zero-DB JSON registry (`couplings.json`).

use crate::lang_status::LangStatus;
use crate::metadata::MediaMetadata;
use crate::srt::SubtitleFile;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

    /// Resolves the canonical path to `couplings.json` inside a given directory.
    ///
    /// # Arguments
    /// - `subtitles_dir`: Base directory path hosting subtitle catalogs and coupling definitions.
    ///
    /// # Returns
    /// PathBuf pointing to `<subtitles_dir>/couplings.json`.
    pub fn default_path(subtitles_dir: &Path) -> PathBuf {
        subtitles_dir.join(Self::REGISTRY_FILENAME)
    }

    /// Loads `CouplingRegistry` from disk, or instantiates a blank registry if missing.
    ///
    /// # Arguments
    /// - `path`: Filesystem path to `couplings.json`.
    ///
    /// # Returns
    /// - `Ok(Self)`: Parsed registry or default empty registry if the file does not exist.
    /// - `Err(anyhow::Error)`: If the file exists but cannot be read or contains invalid JSON.
    ///
    /// # Errors
    /// Returns error with context if filesystem permissions fail or JSON syntax is invalid.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let file = fs::File::open(path)
            .with_context(|| format!("Failed to open coupling registry at {:?}", path))?;
        let reader = std::io::BufReader::new(file);
        let reg: Self = serde_json::from_reader(reader)
            .with_context(|| format!("Failed to parse coupling registry JSON at {:?}", path))?;
        Ok(reg)
    }

    /// Saves `CouplingRegistry` atomically using a temporary file and atomic rename.
    ///
    /// # Arguments
    /// - `path`: Destination filesystem path.
    ///
    /// # Returns
    /// - `Ok(())` on successful atomic write.
    /// - `Err(anyhow::Error)` on serialization, write, or rename error.
    ///
    /// # Errors
    /// Propagates filesystem and serde serialization errors.
    ///
    /// # Data Flow & Invariants
    /// 1. Creates parent directories if they do not exist.
    /// 2. Pretty-prints registry to JSON.
    /// 3. Writes payload to a process-isolated temporary file (`<path>.tmp.<pid>`).
    /// 4. Atomically replaces target file using `fs::rename` to prevent tearing or corrupted state.
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

    /// Acquires a safe file lock on the registry path to synchronize concurrent batch writers.
    ///
    /// # Arguments
    /// - `path`: Target file to lock.
    ///
    /// # Returns
    /// - `Ok(RegistryLock)` guard maintaining the exclusive lock until dropped.
    /// - `Err(anyhow::Error)` on lock acquisition failure.
    pub fn lock(path: &Path) -> Result<RegistryLock> {
        RegistryLock::acquire(path)
    }

    /// Upserts a certified `(media <-> subtitle)` coupling record into the registry.
    ///
    /// # Arguments
    /// - `media_hash`: 32-character hex SHA-256 fingerprint of the reference pHash stream.
    /// - `media_name`: Canonical release name or stem of reference media.
    /// - `phash_filename`: Name of `.phash` file in catalog.
    /// - `fps`: Reference video stream framerate.
    /// - `frame_count`: Total count of sampled video frames.
    /// - `sub_record`: Certified subtitle track metadata.
    ///
    /// # Data Flow & Invariants
    /// If an entry matches `media_hash`, `phash_filename`, or `media_name`, updates metadata in place.
    /// Otherwise appends a new [`CertifiedMediaCoupling`]. Then deduplicates or inserts `sub_record`.
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
    ///
    /// # Arguments
    /// - `matched_media`: Matched identifier string from pHash lookup or media release recognition.
    ///
    /// # Returns
    /// `Some(&CertifiedMediaCoupling)` if a matching reference media is found, `None` otherwise.
    pub fn find_by_matched_media(&self, matched_media: &str) -> Option<&CertifiedMediaCoupling> {
        self.find_by_matched_media_opts(matched_media, None, None)
    }

    /// Finds a media coupling matching a recognized `.phash` filename or title, with strict season and episode isolation.
    ///
    /// # Arguments
    /// - `matched_media`: Media filename, stem, or clean title.
    /// - `season`: Expected season number (e.g. `Some(2)`).
    /// - `episode`: Expected episode number (e.g. `Some(1)`).
    ///
    /// # Returns
    /// `Some(&CertifiedMediaCoupling)` matching both media identity and episodic constraints, `None` otherwise.
    pub fn find_by_matched_media_opts(
        &self,
        matched_media: &str,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> Option<&CertifiedMediaCoupling> {
        self.find_all_by_matched_media_opts(matched_media, season, episode)
            .into_iter()
            .next()
    }

    /// Finds all media couplings matching a recognized media identifier, with strict season and episode compatibility.
    ///
    /// # Arguments
    /// - `matched_media`: Target identifier string.
    /// - `season`: Optional season filter.
    /// - `episode`: Optional episode filter.
    ///
    /// # Returns
    /// Vector of matching [`CertifiedMediaCoupling`] references.
    ///
    /// # Data Flow & Invariants
    /// Prevents cross-season and cross-episode subtitle contamination:
    /// - Extracts season and episode from both reference coupling and search query via [`MediaMetadata::parse`].
    /// - First pass: evaluates exact matches on filename, media name, clean title, or media hash.
    /// - Second pass: evaluates normalized fuzzy prefix matches only if season and episode match strictly.
    pub fn find_all_by_matched_media_opts(
        &self,
        matched_media: &str,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> Vec<&CertifiedMediaCoupling> {
        let clean = matched_media
            .strip_suffix(".phash")
            .unwrap_or(matched_media);
        let norm_matched = matched_media.replace(['_', '.', '-'], " ").to_lowercase();
        let norm_clean = norm_matched.trim();

        let matches_se = |c: &CertifiedMediaCoupling| -> bool {
            let c_meta = MediaMetadata::parse(&c.media_name);
            let p_meta = MediaMetadata::parse(&c.phash_filename);
            let c_season = c_meta.season.or(p_meta.season);
            let c_episode = c_meta.episode.or(p_meta.episode);

            if let Some(req_s) = season {
                match c_season {
                    Some(cs) if cs != req_s => return false,
                    None => return false,
                    _ => {}
                }
            }
            if let Some(req_e) = episode {
                match c_episode {
                    Some(ce) if ce != req_e => {
                        let ep_tag = format!("e{:02}", req_e);
                        let name_lower = c.media_name.to_lowercase();
                        let phash_lower = c.phash_filename.to_lowercase();
                        if !name_lower.contains(&ep_tag) && !phash_lower.contains(&ep_tag) {
                            return false;
                        }
                    }
                    None => {
                        let ep_tag = format!("e{:02}", req_e);
                        let name_lower = c.media_name.to_lowercase();
                        let phash_lower = c.phash_filename.to_lowercase();
                        if !name_lower.contains(&ep_tag) && !phash_lower.contains(&ep_tag) {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            true
        };

        let mut results = Vec::new();

        // Pass 1: Exact matches on filename, media_name, or media_hash
        for c in &self.couplings {
            if (c.phash_filename == matched_media
                || c.media_name == matched_media
                || c.media_name == clean
                || c.phash_filename.strip_suffix(".phash") == Some(clean)
                || c.media_hash == matched_media)
                && matches_se(c)
            {
                results.push(c);
            }
        }

        if !results.is_empty() {
            return results;
        }

        // Pass 2: Fuzzy / prefix match requiring strict season & episode compatibility
        if !norm_clean.is_empty() {
            for c in &self.couplings {
                if !matches_se(c) {
                    continue;
                }
                let c_meta = MediaMetadata::parse(&c.media_name);
                let norm_c_name = c.media_name.replace(['_', '.', '-'], " ").to_lowercase();
                let norm_c_phash = c
                    .phash_filename
                    .replace(['_', '.', '-'], " ")
                    .to_lowercase();
                let norm_c_clean = c_meta
                    .clean_title
                    .replace(['_', '.', '-'], " ")
                    .to_lowercase();

                if norm_c_clean == norm_clean
                    || norm_c_name.starts_with(norm_clean)
                    || norm_c_phash.starts_with(norm_clean)
                    || norm_clean.starts_with(&norm_c_name)
                    || norm_clean.starts_with(&norm_c_phash)
                    || norm_clean.starts_with(&norm_c_clean)
                {
                    results.push(c);
                }
            }
        }

        results
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

    /// Relabels a subtitle's language code and confidence status within a media coupling.
    ///
    /// - `media_selector`: matches `media_hash` (exact or prefix), `media_name`, or `phash_filename`.
    /// - `sub_selector`: optional. If provided, matches `subtitles_hash` (exact or prefix), `name`, or `srt_path`.
    ///   If None and exactly one subtitle is coupled to the media, that subtitle is relabeled.
    /// - `new_lang`: validated ISO-639 language code or "und".
    /// - `new_status`: target `LangStatus` (typically `Confirmed(new_lang)`).
    pub fn relabel(
        &mut self,
        media_selector: &str,
        sub_selector: Option<&str>,
        new_lang: &str,
        new_status: LangStatus,
    ) -> Result<RelabelResult> {
        let norm_lang = new_lang.trim().to_lowercase();
        if norm_lang.is_empty() {
            bail!("Language code cannot be empty");
        }
        if norm_lang != "und" && crate::media_pairing::parse_language_code(&norm_lang).is_none() {
            bail!(
                "Invalid language code '{}'. Expected ISO 639-1 (2 letters), ISO 639-2 (3 letters), or 'und'",
                new_lang
            );
        }

        let media_sel = media_selector.trim();
        if media_sel.is_empty() {
            bail!("Media selector cannot be empty");
        }

        // 1. Locate matching media
        let matching_media_indices: Vec<usize> = self
            .couplings
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.media_hash.eq_ignore_ascii_case(media_sel)
                    || c.media_hash.starts_with(media_sel)
                    || c.media_name == media_sel
                    || c.phash_filename == media_sel
                    || c.media_name.contains(media_sel)
            })
            .map(|(idx, _)| idx)
            .collect();

        let media_idx = match matching_media_indices.len() {
            0 => bail!("No media coupling found matching '{}'", media_sel),
            1 => matching_media_indices[0],
            _ => {
                // Try exact matches first
                if let Some(&exact) = matching_media_indices.iter().find(|&&i| {
                    self.couplings[i].media_hash.eq_ignore_ascii_case(media_sel)
                        || self.couplings[i].media_name == media_sel
                        || self.couplings[i].phash_filename == media_sel
                }) {
                    exact
                } else {
                    let candidates: Vec<String> = matching_media_indices
                        .iter()
                        .take(5)
                        .map(|&i| {
                            format!(
                                "{} ({})",
                                self.couplings[i].media_name,
                                &self.couplings[i].media_hash
                                    [..8.min(self.couplings[i].media_hash.len())]
                            )
                        })
                        .collect();
                    bail!(
                        "Ambiguous media selector '{}' matches {} medias: {}. Please provide a longer hash prefix.",
                        media_sel,
                        matching_media_indices.len(),
                        candidates.join(", ")
                    );
                }
            }
        };

        let media = &mut self.couplings[media_idx];

        // 2. Locate matching subtitle
        let sub_sel_opt = sub_selector.map(str::trim).filter(|s| !s.is_empty());
        let sub_idx = match sub_sel_opt {
            None => {
                if media.subtitles.is_empty() {
                    bail!(
                        "Media '{}' has no coupled subtitles to relabel",
                        media.media_name
                    );
                }
                if media.subtitles.len() > 1 {
                    let subs: Vec<String> = media
                        .subtitles
                        .iter()
                        .map(|s| {
                            format!(
                                "{} (lang={}, hash={})",
                                s.name,
                                s.lang,
                                &s.subtitles_hash[..8.min(s.subtitles_hash.len())]
                            )
                        })
                        .collect();
                    bail!(
                        "Media '{}' has {} coupled subtitles. Please specify which subtitle to relabel with --sub <hash_or_name>: {}",
                        media.media_name,
                        media.subtitles.len(),
                        subs.join(", ")
                    );
                }
                0
            }
            Some(sub_sel) => {
                let clean_sub_sel = sub_sel
                    .trim_start_matches("sub:sha256:")
                    .trim_start_matches("sha256:");
                let matching_sub_indices: Vec<usize> = media
                    .subtitles
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        s.subtitles_hash.eq_ignore_ascii_case(clean_sub_sel)
                            || s.subtitles_hash.starts_with(clean_sub_sel)
                            || s.name == sub_sel
                            || s.srt_path == sub_sel
                            || s.name.contains(sub_sel)
                            || s.srt_path.contains(sub_sel)
                    })
                    .map(|(idx, _)| idx)
                    .collect();

                match matching_sub_indices.len() {
                    0 => bail!(
                        "No subtitle found in media '{}' matching '{}'",
                        media.media_name,
                        sub_sel
                    ),
                    1 => matching_sub_indices[0],
                    _ => {
                        if let Some(&exact) = matching_sub_indices.iter().find(|&&i| {
                            media.subtitles[i]
                                .subtitles_hash
                                .eq_ignore_ascii_case(clean_sub_sel)
                                || media.subtitles[i].name == sub_sel
                        }) {
                            exact
                        } else {
                            bail!(
                                "Ambiguous subtitle selector '{}' in media '{}'. Please specify full hash.",
                                sub_sel,
                                media.media_name
                            );
                        }
                    }
                }
            }
        };

        let sub = &mut media.subtitles[sub_idx];
        let old_lang = sub.lang.clone();
        let old_status = sub.lang_status.clone();

        sub.lang = norm_lang.clone();
        sub.lang_status = new_status.clone();

        Ok(RelabelResult {
            media_hash: media.media_hash.clone(),
            media_name: media.media_name.clone(),
            subtitles_hash: sub.subtitles_hash.clone(),
            subtitle_name: sub.name.clone(),
            old_lang,
            old_status,
            new_lang: norm_lang,
            new_status,
        })
    }
}

/// Result metadata from relabeling a coupled subtitle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelabelResult {
    pub media_hash: String,
    pub media_name: String,
    pub subtitles_hash: String,
    pub subtitle_name: String,
    pub old_lang: String,
    pub old_status: LangStatus,
    pub new_lang: String,
    pub new_status: LangStatus,
}

/// RAII advisory kernel file lock protecting concurrent read-modify-write on coupling registry.
/// Uses POSIX flock; automatically released by the kernel when the file descriptor is closed (drop/exit/SIGKILL).
pub struct RegistryLock {
    _file: fs::File,
    pub path: PathBuf,
}

impl RegistryLock {
    pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
    pub const RETRY_INTERVAL: Duration = Duration::from_millis(50);

    pub fn acquire(registry_path: &Path) -> Result<Self> {
        Self::acquire_timeout(registry_path, Self::DEFAULT_LOCK_TIMEOUT)
    }

    pub fn acquire_timeout(registry_path: &Path, timeout: Duration) -> Result<Self> {
        let lock_path = registry_path.with_extension("lock");
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("Failed to open registry lock file {:?}", lock_path))?;

        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => {
                    return Ok(Self {
                        _file: file,
                        path: lock_path,
                    })
                }
                Err(rustix::io::Errno::WOULDBLOCK) => {
                    std::thread::sleep(Self::RETRY_INTERVAL);
                }
                Err(e) => return Err(anyhow::anyhow!("flock failed on {:?}: {}", lock_path, e)),
            }
        }
        bail!("Timed out waiting for registry lock on {:?}", lock_path);
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

    #[test]
    fn test_coupling_relabel_lifecycle_and_edge_cases() {
        let mut reg = CouplingRegistry::default();
        let sub1 = CertifiedSubtitleRecord {
            subtitles_hash: "11112222333344445555666677778888".to_string(),
            name: "movie.srt".to_string(),
            lang: "und".to_string(),
            lang_status: LangStatus::Unknown,
            srt_path: "movie.srt".to_string(),
            certified: true,
            cue_count: 50,
            sub_frames: vec![],
        };
        reg.upsert_coupling(
            "aaaa1111bbbb2222cccc3333dddd4444".to_string(),
            "movie".to_string(),
            "movie.phash".to_string(),
            24.0,
            1000,
            sub1,
        );

        // 1. Relabel single subtitle without specifying sub selector
        let res = reg
            .relabel(
                "aaaa1111",
                None,
                "fr",
                LangStatus::Confirmed("fr".to_string()),
            )
            .unwrap();
        assert_eq!(res.old_lang, "und");
        assert_eq!(res.old_status, LangStatus::Unknown);
        assert_eq!(res.new_lang, "fr");
        assert_eq!(res.new_status, LangStatus::Confirmed("fr".to_string()));

        let target_sub = &reg.couplings[0].subtitles[0];
        assert_eq!(target_sub.lang, "fr");
        assert_eq!(
            target_sub.lang_status,
            LangStatus::Confirmed("fr".to_string())
        );

        // 2. Add second subtitle to create ambiguity if sub_selector is None
        let sub2 = CertifiedSubtitleRecord {
            subtitles_hash: "99998888777766665555444433332222".to_string(),
            name: "movie.commentary.srt".to_string(),
            lang: "en".to_string(),
            lang_status: LangStatus::Detected("en".to_string()),
            srt_path: "movie.commentary.srt".to_string(),
            certified: true,
            cue_count: 30,
            sub_frames: vec![],
        };
        reg.upsert_coupling(
            "aaaa1111bbbb2222cccc3333dddd4444".to_string(),
            "movie".to_string(),
            "movie.phash".to_string(),
            24.0,
            1000,
            sub2,
        );

        // Now without sub selector, should fail because 2 subtitles exist
        let err_ambig = reg
            .relabel(
                "aaaa1111",
                None,
                "es",
                LangStatus::Confirmed("es".to_string()),
            )
            .unwrap_err();
        assert!(err_ambig.to_string().contains("has 2 coupled subtitles"));

        // With explicit sub selector (prefix of sub2 hash)
        let res2 = reg
            .relabel(
                "aaaa1111",
                Some("99998888"),
                "es",
                LangStatus::Confirmed("es".to_string()),
            )
            .unwrap();
        assert_eq!(res2.new_lang, "es");

        // 3. Error case: invalid language code
        let err_lang = reg
            .relabel(
                "aaaa1111",
                Some("99998888"),
                "french_dub_invalid",
                LangStatus::Confirmed("fr".to_string()),
            )
            .unwrap_err();
        assert!(err_lang.to_string().contains("Invalid language code"));

        // 4. Error case: unknown media selector
        let err_media = reg
            .relabel(
                "nonexistent_media",
                None,
                "fr",
                LangStatus::Confirmed("fr".to_string()),
            )
            .unwrap_err();
        assert!(err_media.to_string().contains("No media coupling found"));

        // 5. Error case: unknown subtitle selector
        let err_sub = reg
            .relabel(
                "aaaa1111",
                Some("00000000"),
                "fr",
                LangStatus::Confirmed("fr".to_string()),
            )
            .unwrap_err();
        assert!(err_sub.to_string().contains("No subtitle found in media"));
    }

    #[test]
    fn test_registry_lock_concurrency() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;

        let temp_dir = std::env::temp_dir().join(format!("test_reg_lock_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let reg_path = temp_dir.join("couplings.json");

        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        for _ in 0..10 {
            let path = reg_path.clone();
            let c = Arc::clone(&counter);
            handles.push(thread::spawn(move || {
                let lock = RegistryLock::acquire_timeout(&path, Duration::from_secs(5));
                assert!(lock.is_ok(), "Thread must acquire lock");
                let prev = c.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(10));
                assert_eq!(
                    c.load(Ordering::SeqCst),
                    prev + 1,
                    "Lock must guarantee mutual exclusion"
                );
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 10);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_by_matched_media_opts_strict_season_episode() {
        let mut reg = CouplingRegistry::default();

        let make_coupling = |name: &str, lang: &str| CertifiedMediaCoupling {
            media_hash: format!("hash_{}", name),
            media_name: name.to_string(),
            phash_filename: format!("{}.phash", name),
            fps: 24.0,
            frame_count: 100,
            subtitles: vec![CertifiedSubtitleRecord {
                subtitles_hash: format!("sub_hash_{}", name),
                name: format!("{}.{}.srt", name, lang),
                lang: lang.to_string(),
                lang_status: LangStatus::Confirmed(lang.to_string()),
                srt_path: format!("{}.{}.srt", name, lang),
                certified: true,
                cue_count: 50,
                sub_frames: vec![],
            }],
        };

        // Seed with What If S2E3 and S2E1, and The Office S5E1
        reg.couplings.push(make_coupling(
            "What.If.2021.S02E03.What.if.Happy.Hogan.Saved.Christmas.2160p",
            "fr",
        ));
        reg.couplings.push(make_coupling(
            "What.If.2021.S02E01.What.if.Nebula.Joined.the.Nova.Corps.2160p",
            "fr",
        ));
        reg.couplings.push(make_coupling(
            "The.Office.US.S05E01.1080p.NF.WEB.x264-MRSK",
            "fr",
        ));

        // 1. Query What If with S2E1 -> MUST match S2E1, NOT S2E3 (even though S2E3 is first in array!)
        let match_s2e1 = reg.find_by_matched_media_opts("What If", Some(2), Some(1));
        assert!(match_s2e1.is_some());
        assert_eq!(
            match_s2e1.unwrap().media_name,
            "What.If.2021.S02E01.What.if.Nebula.Joined.the.Nova.Corps.2160p"
        );

        // 2. Query What If with S2E3 -> MUST match S2E3
        let match_s2e3 = reg.find_by_matched_media_opts("What If", Some(2), Some(3));
        assert!(match_s2e3.is_some());
        assert_eq!(
            match_s2e3.unwrap().media_name,
            "What.If.2021.S02E03.What.if.Happy.Hogan.Saved.Christmas.2160p"
        );

        // 3. Query What If with S1E1 -> MUST NOT match any (no S1 in registry)
        let match_s1e1 = reg.find_by_matched_media_opts("What If", Some(1), Some(1));
        assert!(match_s1e1.is_none());

        // 4. Query The Office with S4E1 -> MUST return None (MUST NOT match S5E1!)
        let match_off_s4e1 = reg.find_by_matched_media_opts("The Office US", Some(4), Some(1));
        assert!(match_off_s4e1.is_none());

        // 5. Query The Office with S5E1 -> MUST match S5E1
        let match_off_s5e1 = reg.find_by_matched_media_opts("The Office US", Some(5), Some(1));
        assert!(match_off_s5e1.is_some());
        assert_eq!(
            match_off_s5e1.unwrap().media_name,
            "The.Office.US.S05E01.1080p.NF.WEB.x264-MRSK"
        );
    }
}
