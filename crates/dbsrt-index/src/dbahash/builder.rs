//! Deterministic `.dbahash` v1 archive builders (in-memory & low-RAM streaming directory packer).
//!
//! Provides two complementary compilation entrypoints:
//! 1. [`DbahashBuilder`]: programmatic builder for unit tests and in-memory [`AHashCatalog`] conversion.
//! 2. [`pack_ahash_directories`]: two-pass streaming compiler over `.ahash` directories that keeps peak
//!    dense heap usage at `64 KiB` per worker while collapsing binary twins and preserving secondary
//!    multi-audio tracks (`.a1`, `.a2`).

use crate::ahash::{
    is_moviehash_hex, resolve_ahash_file_metadata, sibling_phash_dir_for, AHashCatalog,
    AHashMih32Index, Mih32Entry, ResolvedAhashFileMeta, AHASH_FILE_EXTENSION,
};
use crate::dbahash::format::{
    align_up_64, compute_dbahash_checksum, encode_inline_language, DbahashHeader,
    DbahashMediaEntry, DbahashPackStats, DBAHASH_FLAG_HAS_COARSE_MIH32, DBAHASH_HEADER_BYTES,
    DBAHASH_HEADER_RESERVED_BYTES, DBAHASH_MAGIC, DBAHASH_MEDIA_ENTRY_BYTES,
    DBAHASH_TWIN_TRAILER_MAGIC, DBAHASH_VERSION, DEFAULT_DBAHASH_FACTOR_K,
    MEDIA_FLAG_HAS_AUDIO_INDEX, MEDIA_FLAG_HAS_MOVIEHASH,
};
use anyhow::{ensure, Context, Result};
use dbsrt_core::ahash::{AhashHeader, HEADER_SIZE_BYTES};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Intra-process monotonic counter disambiguating concurrent temporary `.dbahash` files.
static DBAHASH_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Size in bytes of one 64-bit perceptual/acoustic hash (`size_of::<u64>()`).
const U64_BYTES: usize = std::mem::size_of::<u64>();

/// Size in bytes of one [`Mih32Entry`] record (`size_of::<Mih32Entry>()`).
const MIH32_ENTRY_BYTES: usize = std::mem::size_of::<Mih32Entry>();

/// Streaming I/O chunk size in `u64` frames (`8_192 * 8 = 64 KiB`) during Pass 1 & Pass 2.
const STREAM_CHUNK_FRAMES: usize = 8_192;

/// Streaming I/O buffer size in bytes (`64 KiB`).
const STREAM_BUFFER_BYTES: usize = STREAM_CHUNK_FRAMES * U64_BYTES;

/// BufWriter capacity (`1 MiB`) used when serializing `.dbahash` archives to disk.
const ARCHIVE_WRITER_CAPACITY_BYTES: usize = 1_024 * 1_024;

/// Borrowed descriptor passed to [`DbahashBuilder::add_track`].
#[derive(Debug, Clone, Copy)]
pub struct DbahashTrackInput<'a> {
    /// Canonical media title or stem.
    pub media_name: &'a str,
    /// Source `.ahash` relative path or filename (distinguishes `.a1`, `.a2` tracks).
    pub file_path: &'a str,
    /// Effective sub-fingerprint rate in Hz (e.g. `86.125`).
    pub audio_fps: f64,
    /// Optional 64-bit OpenSubtitles MovieHash.
    pub moviehash: Option<u64>,
    /// Optional 0-based relative audio stream index (`0:a:<k>`).
    pub audio_index: Option<usize>,
    /// Optional ISO 639-2 / BCP-47 audio language tag (e.g. `"fre"`, `"eng"`).
    pub audio_language: Option<&'a str>,
    /// Contiguous dense `u64` sub-fingerprints for this track.
    pub hashes: &'a [u64],
}

/// Configuration options for [`pack_ahash_directories`].
#[derive(Debug, Clone)]
pub struct DbahashPackOptions {
    /// Ordered list of directories scanned for `*.ahash` files.
    pub ahash_dirs: Vec<PathBuf>,
    /// Destination path for the compiled `.dbahash` archive (written atomically).
    pub output_path: PathBuf,
    /// Coarse decimation stride $K$ (`>= 1`, default [`DEFAULT_DBAHASH_FACTOR_K`] = `32`).
    pub factor_k: usize,
    /// Embeds pre-sorted `MIH-32` coarse tables when `true` (`0` sort latency on `open()`).
    pub embed_coarse_mih32: bool,
}

impl Default for DbahashPackOptions {
    fn default() -> Self {
        Self {
            ahash_dirs: vec![PathBuf::from("data/__AHASH__")],
            output_path: PathBuf::from("data/catalog.dbahash"),
            factor_k: DEFAULT_DBAHASH_FACTOR_K,
            embed_coarse_mih32: true,
        }
    }
}

/// Track metadata common to both in-memory [`DbahashBuilder`] and streaming directory packing.
#[derive(Debug, Clone)]
struct TrackMetaRecord {
    media_name: String,
    file_path: String,
    audio_fps: f64,
    moviehash: Option<u64>,
    audio_index: Option<usize>,
    audio_language: Option<String>,
    dense_frame_count: u32,
    coarse_hashes: Vec<u64>,
}

/// Programmatic builder for constructing `.dbahash` v1 archives from in-memory slices.
#[derive(Debug, Clone)]
pub struct DbahashBuilder {
    factor_k: usize,
    embed_coarse_mih32: bool,
    tracks: Vec<TrackMetaRecord>,
    dense_hashes: Vec<u64>,
}

impl DbahashBuilder {
    /// Creates an empty `.dbahash` v1 builder configured with decimation stride `factor_k`.
    ///
    /// # Arguments
    /// * `factor_k` - Coarse sub-sampling stride (`1 <= factor_k <= u32::MAX`).
    ///
    /// # Returns
    /// Initialized [`DbahashBuilder`] with `embed_coarse_mih32 = true`.
    ///
    /// # Errors
    /// Returns `Err` if `factor_k == 0` or exceeds `u32::MAX`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Enforces `factor_k >= 1` upfront before any track insertion.
    pub fn new(factor_k: usize) -> Result<Self> {
        ensure!(
            factor_k >= 1 && factor_k <= u32::MAX as usize,
            "factor_k must be in 1..=u32::MAX, got {factor_k}"
        );
        Ok(Self {
            factor_k,
            embed_coarse_mih32: true,
            tracks: Vec::new(),
            dense_hashes: Vec::new(),
        })
    }

    /// Configures whether pre-sorted `MIH-32` coarse tables are embedded into the output file.
    ///
    /// # Arguments
    /// * `embed` - `true` to embed coarse hashes and `MIH-32` tables; `false` for dense-only archive.
    ///
    /// # Returns
    /// Updated [`DbahashBuilder`].
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Toggles [`DBAHASH_FLAG_HAS_COARSE_MIH32`] during serialization.
    pub fn with_coarse_mih32(mut self, embed: bool) -> Self {
        self.embed_coarse_mih32 = embed;
        self
    }

    /// Populates a [`DbahashBuilder`] from an existing in-memory [`AHashCatalog`].
    ///
    /// # Arguments
    /// * `catalog` - Source [`AHashCatalog`] whose tracks will be packed in order.
    /// * `factor_k` - Coarse sub-sampling stride (`>= 1`).
    ///
    /// # Returns
    /// Populated [`DbahashBuilder`] ready for [`Self::write_to_file`].
    ///
    /// # Errors
    /// Returns `Err` if `factor_k == 0` or any entry exceeds format frame/path limits.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Extracts stem MovieHashes when present and preserves `audio_index`, `audio_language`, and `path`.
    pub fn from_catalog(catalog: &AHashCatalog, factor_k: usize) -> Result<Self> {
        let mut builder = Self::new(factor_k)?;
        for entry in &catalog.entries {
            let file_path = entry.path.to_string_lossy();
            let moviehash = crate::ahash::extract_stem_moviehash(&entry.path);
            builder.add_track(DbahashTrackInput {
                media_name: &entry.media_name,
                file_path: &file_path,
                audio_fps: entry.audio_fps,
                moviehash,
                audio_index: entry.audio_index,
                audio_language: entry.audio_language.as_deref(),
                hashes: &entry.hashes,
            })?;
        }
        Ok(builder)
    }

    /// Appends a single audio track to the builder.
    ///
    /// # Arguments
    /// * `track` - [`DbahashTrackInput`] containing track metadata and dense `u64` hashes.
    ///
    /// # Returns
    /// 0-based track index inside the media table.
    ///
    /// # Errors
    /// Returns `Err` if `hashes` is empty, `audio_fps` is non-positive, or lengths overflow `u32`/`u16`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Subsamples `hashes` at stride `self.factor_k` (`0, K, 2K, ...`) and appends dense hashes contiguously.
    pub fn add_track(&mut self, track: DbahashTrackInput<'_>) -> Result<usize> {
        validate_track_bounds(
            track.media_name,
            track.file_path,
            track.audio_fps,
            track.hashes.len(),
        )?;
        let coarse_hashes: Vec<u64> = track
            .hashes
            .iter()
            .step_by(self.factor_k)
            .copied()
            .collect();
        let idx = self.tracks.len();
        self.dense_hashes.extend_from_slice(track.hashes);
        self.tracks.push(TrackMetaRecord {
            media_name: track.media_name.to_string(),
            file_path: track.file_path.to_string(),
            audio_fps: track.audio_fps,
            moviehash: track.moviehash,
            audio_index: track.audio_index,
            audio_language: track.audio_language.map(str::to_string),
            dense_frame_count: track.hashes.len() as u32,
            coarse_hashes,
        });
        Ok(idx)
    }

    /// Atomically serializes the `.dbahash` v1 archive to `path`.
    ///
    /// # Arguments
    /// * `path` - Target `.dbahash` file path.
    ///
    /// # Returns
    /// [`DbahashPackStats`] summarizing counts, byte size, and elapsed wall time.
    ///
    /// # Errors
    /// Returns `Err` on filesystem I/O failure or section overflow.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Writes to `<path>.<pid>.<seq>.tmp`, flushes + `sync_all()`, and atomically renames to `path`.
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> Result<DbahashPackStats> {
        let start = Instant::now();
        let compiled =
            compile_archive_prefix(&self.tracks, &[], self.factor_k, self.embed_coarse_mih32)?;
        let output_bytes = write_archive_atomic(path.as_ref(), &compiled, |writer| {
            writer.write_all(bytemuck::cast_slice(&self.dense_hashes))?;
            Ok(())
        })?;

        Ok(DbahashPackStats {
            scanned_files: self.tracks.len(),
            packed_tracks: self.tracks.len(),
            deduplicated_twins: 0,
            skipped_files: 0,
            factor_k: self.factor_k,
            has_coarse_mih32: self.embed_coarse_mih32,
            total_dense_frames: compiled.header.total_dense_frames,
            total_coarse_frames: compiled.header.total_coarse_frames,
            output_bytes,
            duration_seconds: start.elapsed().as_secs_f64(),
        })
    }
}

/// Validates per-track field bounds before packing into [`DbahashMediaEntry`].
fn validate_track_bounds(
    media_name: &str,
    file_path: &str,
    audio_fps: f64,
    dense_len: usize,
) -> Result<()> {
    ensure!(dense_len > 0, "Track '{file_path}' has 0 dense frames");
    ensure!(
        dense_len <= u32::MAX as usize,
        "Track '{file_path}' exceeds u32::MAX dense frames ({dense_len})"
    );
    ensure!(
        audio_fps.is_finite() && audio_fps > 0.0,
        "Track '{file_path}' has invalid audio_fps {audio_fps}"
    );
    ensure!(
        media_name.len() <= u32::MAX as usize,
        "media_name exceeds u32::MAX bytes"
    );
    ensure!(
        file_path.len() <= u16::MAX as usize,
        "file_path '{file_path}' exceeds u16::MAX bytes"
    );
    Ok(())
}

/// Pre-compiled prefix sections (everything preceding `dense_hashes`) ready for sequential disk output.
struct CompiledArchivePrefix {
    header: DbahashHeader,
    media_entries: Vec<DbahashMediaEntry>,
    string_table: Vec<u8>,
    coarse_hashes: Vec<u64>,
    mih_upper: Vec<Mih32Entry>,
    mih_lower: Vec<Mih32Entry>,
}

/// Appends the optional `\0TWINS1\0` alias trailer to `string_table` for $\mathcal{O}(1)$ twin skipping.
fn append_twin_alias_trailer(
    string_table: &mut Vec<u8>,
    twin_aliases: &[(String, u32)],
) -> Result<()> {
    if twin_aliases.is_empty() {
        return Ok(());
    }
    let count = u32::try_from(twin_aliases.len()).context("Twin alias count exceeds u32::MAX")?;
    string_table.extend_from_slice(&DBAHASH_TWIN_TRAILER_MAGIC);
    string_table.extend_from_slice(&count.to_le_bytes());
    for (file_name, frames) in twin_aliases {
        let len = u16::try_from(file_name.len()).context("Twin filename exceeds u16::MAX bytes")?;
        string_table.extend_from_slice(&frames.to_le_bytes());
        string_table.extend_from_slice(&len.to_le_bytes());
        string_table.extend_from_slice(file_name.as_bytes());
    }
    Ok(())
}

/// Assembles header, media table, string pool, and optional `MIH-32` tables from track metadata.
fn compile_archive_prefix(
    tracks: &[TrackMetaRecord],
    twin_aliases: &[(String, u32)],
    factor_k: usize,
    embed_coarse_mih32: bool,
) -> Result<CompiledArchivePrefix> {
    ensure!(
        tracks.len() <= u32::MAX as usize,
        "Track count {} exceeds u32::MAX",
        tracks.len()
    );
    let mut string_table = Vec::new();
    let mut media_entries = Vec::with_capacity(tracks.len());
    let mut coarse_hashes = Vec::new();
    let mut dense_cursor = 0u64;
    let mut coarse_cursor = 0u64;

    for track in tracks {
        let name_offset = u32::try_from(string_table.len()).context("String table > 4 GiB")?;
        let name_len = u32::try_from(track.media_name.len()).context("media_name > 4 GiB")?;
        string_table.extend_from_slice(track.media_name.as_bytes());

        let path_offset = u32::try_from(string_table.len()).context("String table > 4 GiB")?;
        let path_len = u16::try_from(track.file_path.len()).context("file_path > 64 KiB")?;
        string_table.extend_from_slice(track.file_path.as_bytes());

        let coarse_count = if embed_coarse_mih32 {
            u32::try_from(track.coarse_hashes.len()).context("coarse_frame_count > u32::MAX")?
        } else {
            0
        };
        if embed_coarse_mih32 {
            coarse_hashes.extend_from_slice(&track.coarse_hashes);
        }

        media_entries.push(build_media_entry(
            track,
            (dense_cursor, coarse_cursor, coarse_count),
            (name_offset, name_len, path_offset, path_len),
        ));
        dense_cursor = dense_cursor.saturating_add(u64::from(track.dense_frame_count));
        coarse_cursor = coarse_cursor.saturating_add(u64::from(coarse_count));
    }
    append_twin_alias_trailer(&mut string_table, twin_aliases)?;

    ensure!(
        coarse_cursor <= u32::MAX as u64,
        "Total coarse frames ({coarse_cursor}) exceeds u32::MAX for Mih32Entry"
    );

    let (mih_upper, mih_lower) = if embed_coarse_mih32 && !coarse_hashes.is_empty() {
        let mih = AHashMih32Index::build(&coarse_hashes);
        (mih.upper_table, mih.lower_table)
    } else {
        (Vec::new(), Vec::new())
    };

    let header = build_header_with_offsets(
        (
            factor_k as u32,
            media_entries.len() as u32,
            embed_coarse_mih32,
        ),
        (dense_cursor, coarse_cursor),
        &media_entries,
        &string_table,
    );

    Ok(CompiledArchivePrefix {
        header,
        media_entries,
        string_table,
        coarse_hashes,
        mih_upper,
        mih_lower,
    })
}

/// Constructs a single 64-byte [`DbahashMediaEntry`] from resolved offsets and track metadata.
fn build_media_entry(
    track: &TrackMetaRecord,
    cursors: (u64, u64, u32),
    str_bounds: (u32, u32, u32, u16),
) -> DbahashMediaEntry {
    let (dense_start_idx, coarse_start_idx, coarse_frame_count) = cursors;
    let (name_offset, name_len, path_offset, path_len) = str_bounds;
    let (lang_flag, language) = encode_inline_language(track.audio_language.as_deref());
    let mut flags = lang_flag;
    let moviehash = match track.moviehash {
        Some(h) => {
            flags |= MEDIA_FLAG_HAS_MOVIEHASH;
            h
        }
        None => 0,
    };
    let audio_index = match track.audio_index.and_then(|idx| u16::try_from(idx).ok()) {
        Some(idx) => {
            flags |= MEDIA_FLAG_HAS_AUDIO_INDEX;
            idx
        }
        None => 0,
    };

    DbahashMediaEntry {
        moviehash,
        audio_fps_bits: track.audio_fps.to_bits(),
        dense_start_idx,
        coarse_start_idx,
        dense_frame_count: track.dense_frame_count,
        coarse_frame_count,
        name_offset,
        name_len,
        path_offset,
        path_len,
        audio_index,
        flags,
        language,
    }
}

/// Computes 64-byte aligned section offsets and checksum for [`DbahashHeader`].
fn build_header_with_offsets(
    meta: (u32, u32, bool),
    frame_totals: (u64, u64),
    media_entries: &[DbahashMediaEntry],
    string_table: &[u8],
) -> DbahashHeader {
    let (factor_k, media_count, embed_coarse_mih32) = meta;
    let (total_dense_frames, total_coarse_frames) = frame_totals;
    let strings_len = string_table.len() as u64;

    let offset_media_table = align_up_64(DBAHASH_HEADER_BYTES as u64);
    let media_bytes = (media_entries.len() * DBAHASH_MEDIA_ENTRY_BYTES) as u64;
    let offset_strings_table = align_up_64(offset_media_table + media_bytes);
    let mut cursor = align_up_64(offset_strings_table + strings_len);

    let (offset_coarse_hashes, offset_coarse_mih_upper, offset_coarse_mih_lower, flags) =
        if embed_coarse_mih32 {
            let off_coarse = cursor;
            cursor = align_up_64(off_coarse + total_coarse_frames * (U64_BYTES as u64));
            let off_upper = cursor;
            cursor = align_up_64(off_upper + total_coarse_frames * (MIH32_ENTRY_BYTES as u64));
            let off_lower = cursor;
            cursor = align_up_64(off_lower + total_coarse_frames * (MIH32_ENTRY_BYTES as u64));
            (
                off_coarse,
                off_upper,
                off_lower,
                DBAHASH_FLAG_HAS_COARSE_MIH32,
            )
        } else {
            (0, 0, 0, 0)
        };

    let offset_dense_hashes = cursor;
    let mut header = DbahashHeader {
        magic: DBAHASH_MAGIC,
        version: DBAHASH_VERSION,
        flags,
        factor_k,
        media_count,
        reserved_u32: 0,
        total_dense_frames,
        total_coarse_frames,
        offset_media_table,
        offset_strings_table,
        strings_len,
        offset_dense_hashes,
        offset_coarse_hashes,
        offset_coarse_mih_upper,
        offset_coarse_mih_lower,
        checksum: 0,
        _reserved: [0u8; DBAHASH_HEADER_RESERVED_BYTES],
    };
    header.checksum = compute_dbahash_checksum(&header, media_entries, string_table);
    header
}

/// Writes `count` zero padding bytes to align the stream position to 64 bytes.
fn write_zero_padding<W: Write>(writer: &mut W, current: u64, target: u64) -> Result<()> {
    ensure!(
        target >= current,
        "Section alignment cursor regressed ({target} < {current})"
    );
    let pad = (target - current) as usize;
    if pad > 0 {
        let zeros = [0u8; DBAHASH_HEADER_BYTES];
        let mut rem = pad;
        while rem > 0 {
            let step = rem.min(zeros.len());
            writer.write_all(&zeros[..step])?;
            rem -= step;
        }
    }
    Ok(())
}

/// Writes prefix sections + dense payload into `tmp_path` and flushes via `sync_all`.
fn write_archive_temp_file<F>(
    tmp_path: &Path,
    prefix: &CompiledArchivePrefix,
    write_dense: F,
) -> Result<u64>
where
    F: FnOnce(&mut BufWriter<File>) -> Result<()>,
{
    let file = File::create(tmp_path)
        .with_context(|| format!("Creating temporary archive {:?}", tmp_path))?;
    let mut writer = BufWriter::with_capacity(ARCHIVE_WRITER_CAPACITY_BYTES, file);

    write_prefix_sections(&mut writer, prefix)?;
    write_dense(&mut writer)?;
    writer.flush()?;

    let file = writer
        .into_inner()
        .map_err(|e| anyhow::anyhow!("Flushing archive writer failed: {e}"))?;
    file.sync_all()?;
    let total_bytes = file.metadata()?.len();
    crate::dbahash::format::advise_cold_archive_sections(&file, &prefix.header, total_bytes);
    Ok(total_bytes)
}

/// Writes prefix sections + caller-supplied dense payload into an atomic `.tmp` file and renames it.
fn write_archive_atomic<F>(
    output_path: &Path,
    prefix: &CompiledArchivePrefix,
    write_dense: F,
) -> Result<u64>
where
    F: FnOnce(&mut BufWriter<File>) -> Result<()>,
{
    if let Some(parent) = output_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("Creating parent directory {:?}", parent))?;
    }
    let seq = DBAHASH_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_name = output_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("catalog.dbahash");
    let tmp_path =
        output_path.with_file_name(format!(".{file_name}.{}.{seq}.tmp", std::process::id()));

    let bytes = match write_archive_temp_file(&tmp_path, prefix, write_dense) {
        Ok(b) => b,
        Err(err) => {
            let _ = fs::remove_file(&tmp_path);
            return Err(err);
        }
    };

    if let Err(err) = fs::rename(&tmp_path, output_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err).with_context(|| format!("Renaming {:?} -> {:?}", tmp_path, output_path));
    }
    Ok(bytes)
}

/// Serializes the 64-byte aligned header, media table, string pool, and optional `MIH-32` sections.
fn write_prefix_sections<W: Write>(writer: &mut W, p: &CompiledArchivePrefix) -> Result<()> {
    let h = &p.header;
    writer.write_all(bytemuck::bytes_of(h))?;
    let mut pos = DBAHASH_HEADER_BYTES as u64;

    write_zero_padding(writer, pos, h.offset_media_table)?;
    let media_bytes: &[u8] = bytemuck::cast_slice(&p.media_entries);
    writer.write_all(media_bytes)?;
    pos = h.offset_media_table + media_bytes.len() as u64;

    write_zero_padding(writer, pos, h.offset_strings_table)?;
    writer.write_all(&p.string_table)?;
    pos = h.offset_strings_table + p.string_table.len() as u64;

    if h.has_coarse_mih32() {
        write_zero_padding(writer, pos, h.offset_coarse_hashes)?;
        let coarse_bytes: &[u8] = bytemuck::cast_slice(&p.coarse_hashes);
        writer.write_all(coarse_bytes)?;
        pos = h.offset_coarse_hashes + coarse_bytes.len() as u64;

        write_zero_padding(writer, pos, h.offset_coarse_mih_upper)?;
        let upper_bytes: &[u8] = bytemuck::cast_slice(&p.mih_upper);
        writer.write_all(upper_bytes)?;
        pos = h.offset_coarse_mih_upper + upper_bytes.len() as u64;

        write_zero_padding(writer, pos, h.offset_coarse_mih_lower)?;
        let lower_bytes: &[u8] = bytemuck::cast_slice(&p.mih_lower);
        writer.write_all(lower_bytes)?;
        pos = h.offset_coarse_mih_lower + lower_bytes.len() as u64;
    }

    write_zero_padding(writer, pos, h.offset_dense_hashes)?;
    Ok(())
}

/// Intermediate Pass-1 descriptor produced without retaining dense `Vec<u64>` in heap memory.
struct ScannedAhashTrack {
    path: PathBuf,
    meta: ResolvedAhashFileMeta,
    audio_fps: f64,
    dense_frame_count: u32,
    sha256: [u8; 32],
    coarse_hashes: Vec<u64>,
}

/// Compiles one or more `.ahash` directories into a single `.dbahash` v1 archive in two streaming passes.
///
/// # Arguments
/// * `options` - [`DbahashPackOptions`] specifying input directories, output path, `factor_k`, and `MIH-32` flag.
///
/// # Returns
/// [`DbahashPackStats`] detailing scanned files, deduplicated twins, frame counts, byte size, and elapsed time.
///
/// # Errors
/// Returns `Err` if `factor_k == 0`, output creation fails, or a source file truncates mid-pack.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// 1. **Pass 1 (Parallel, `64 KiB` buffer/worker)**: validates `.ahash` headers, resolves `<stem>.json`
///    sidecar metadata, computes exact SHA-256 payload digests, and extracts decimated `1/K` coarse hashes.
/// 2. **Twin Deduplication**: collapses byte-identical `.ahash` twins using the canonical preference order
///    of [`AHashCatalog::load_from_dir`] while preserving distinct `.a1` / `.a2` multi-audio tracks.
/// 3. **Pass 2 (Sequential Streaming)**: writes 64-byte aligned metadata + pre-sorted `MIH-32` tables, then
///    streams dense `u64` payloads directly into `<output>.tmp` before `fsync` + atomic `rename`.
pub fn pack_ahash_directories(options: &DbahashPackOptions) -> Result<DbahashPackStats> {
    ensure!(
        options.factor_k >= 1 && options.factor_k <= u32::MAX as usize,
        "factor_k must be in 1..=u32::MAX, got {}",
        options.factor_k
    );
    let start = Instant::now();
    let discovered = collect_sorted_ahash_files(&options.ahash_dirs)?;
    let scanned_files = discovered.len();

    let scanned: Vec<ScannedAhashTrack> = discovered
        .into_par_iter()
        .filter_map(|(path, dir)| {
            let phash_dir = sibling_phash_dir_for(&dir);
            scan_single_ahash_file(&path, &dir, phash_dir.as_deref(), options.factor_k)
        })
        .collect();
    let skipped_files = scanned_files.saturating_sub(scanned.len());

    let (unique_tracks, deduplicated_twins, twin_aliases) = dedup_scanned_tracks(scanned)?;
    let source_paths: Vec<(PathBuf, u32)> = unique_tracks
        .iter()
        .map(|t| (t.path.clone(), t.dense_frame_count))
        .collect();
    let meta_records: Vec<TrackMetaRecord> = unique_tracks
        .into_iter()
        .map(|t| TrackMetaRecord {
            media_name: t.meta.media_name,
            file_path: t.path.to_string_lossy().to_string(),
            audio_fps: t.audio_fps,
            moviehash: t.meta.moviehash,
            audio_index: t.meta.audio_index,
            audio_language: t.meta.audio_language,
            dense_frame_count: t.dense_frame_count,
            coarse_hashes: t.coarse_hashes,
        })
        .collect();

    let compiled = compile_archive_prefix(
        &meta_records,
        &twin_aliases,
        options.factor_k,
        options.embed_coarse_mih32,
    )?;
    let output_bytes = write_archive_atomic(&options.output_path, &compiled, |writer| {
        for (src_path, expected_frames) in &source_paths {
            stream_track_dense_payload(src_path, *expected_frames, writer)?;
        }
        Ok(())
    })?;

    Ok(DbahashPackStats {
        scanned_files,
        packed_tracks: meta_records.len(),
        deduplicated_twins,
        skipped_files,
        factor_k: options.factor_k,
        has_coarse_mih32: options.embed_coarse_mih32,
        total_dense_frames: compiled.header.total_dense_frames,
        total_coarse_frames: compiled.header.total_coarse_frames,
        output_bytes,
        duration_seconds: start.elapsed().as_secs_f64(),
    })
}

/// Discovers all `*.ahash` files across `dirs` in deterministic sorted order.
fn collect_sorted_ahash_files(dirs: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut out = Vec::new();
    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        let mut local = Vec::new();
        for entry in fs::read_dir(dir).with_context(|| format!("Reading {:?}", dir))? {
            let p = entry?.path();
            if p.is_file() && p.extension().is_some_and(|e| e == AHASH_FILE_EXTENSION) {
                local.push((p, dir.clone()));
            }
        }
        local.sort_by(|a, b| a.0.cmp(&b.0));
        out.extend(local);
    }
    Ok(out)
}

/// Streams a single `.ahash` file to extract header metadata, SHA-256 digest, and `1/K` coarse hashes.
fn scan_single_ahash_file(
    path: &Path,
    dir: &Path,
    phash_dir: Option<&Path>,
    factor_k: usize,
) -> Option<ScannedAhashTrack> {
    let mut file = File::open(path).ok()?;
    let file_len = usize::try_from(file.metadata().ok()?.len()).ok()?;
    let mut hdr_bytes = [0u8; HEADER_SIZE_BYTES];
    file.read_exact(&mut hdr_bytes).ok()?;
    let header = AhashHeader::from_bytes(&hdr_bytes, file_len).ok()?;
    if header.frame_count == 0 || header.frame_count > u32::MAX as u64 {
        return None;
    }
    let audio_fps = header.audio_fps();
    if !audio_fps.is_finite() || audio_fps <= 0.0 {
        return None;
    }

    let total_frames = header.frame_count as usize;
    let coarse_cap = total_frames.div_ceil(factor_k);
    let mut coarse_hashes = Vec::with_capacity(coarse_cap);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; STREAM_BUFFER_BYTES];
    let mut frame_idx = 0usize;

    while frame_idx < total_frames {
        let frames_to_read = (total_frames - frame_idx).min(STREAM_CHUNK_FRAMES);
        let byte_count = frames_to_read * U64_BYTES;
        file.read_exact(&mut buf[..byte_count]).ok()?;
        hasher.update(&buf[..byte_count]);

        let first_rem = frame_idx % factor_k;
        let mut local = if first_rem == 0 {
            0
        } else {
            factor_k - first_rem
        };
        while local < frames_to_read {
            let off = local * U64_BYTES;
            let word = u64::from_le_bytes(buf[off..off + U64_BYTES].try_into().ok()?);
            coarse_hashes.push(word);
            local += factor_k;
        }
        frame_idx += frames_to_read;
    }

    let meta = resolve_ahash_file_metadata(path, dir, phash_dir);
    if path.to_string_lossy().len() > u16::MAX as usize {
        return None;
    }

    Some(ScannedAhashTrack {
        path: path.to_path_buf(),
        meta,
        audio_fps,
        dense_frame_count: total_frames as u32,
        sha256: hasher.finalize().into(),
        coarse_hashes,
    })
}

/// Computes the canonical deduplication preference tuple (smaller tuple wins).
fn track_dedup_rank(t: &ScannedAhashTrack) -> (bool, bool, bool, PathBuf) {
    let has_sidecar_audio = t.meta.audio_index.is_some() || t.meta.audio_language.is_some();
    (
        is_moviehash_hex(&t.meta.media_name),
        !has_sidecar_audio,
        !t.meta.has_moviehash_stem,
        t.path.clone(),
    )
}

/// Returns the index in `kept` whose payload matches `candidate` byte-for-byte, if any.
fn find_identical_kept_track(
    kept: &[ScannedAhashTrack],
    bucket: &[usize],
    candidate: &ScannedAhashTrack,
) -> Result<Option<usize>> {
    for &idx in bucket {
        if files_have_identical_payload(
            &kept[idx].path,
            &candidate.path,
            candidate.dense_frame_count,
        )? {
            return Ok(Some(idx));
        }
    }
    Ok(None)
}

type DedupScanResult = (Vec<ScannedAhashTrack>, usize, Vec<(String, u32)>);

/// Collapses binary twins with identical `(dense_frame_count, sha256, payload)` using canonical ranking.
fn dedup_scanned_tracks(tracks: Vec<ScannedAhashTrack>) -> Result<DedupScanResult> {
    let mut kept: Vec<ScannedAhashTrack> = Vec::with_capacity(tracks.len());
    let mut buckets: HashMap<(u32, [u8; 32]), Vec<usize>> = HashMap::new();
    let mut deduplicated = 0usize;
    let mut twin_aliases: Vec<(String, u32)> = Vec::new();

    for track in tracks {
        let key = (track.dense_frame_count, track.sha256);
        let bucket = buckets.entry(key).or_default();
        if let Some(idx) = find_identical_kept_track(&kept, bucket, &track)? {
            deduplicated += 1;
            let absorbed = if track_dedup_rank(&track) < track_dedup_rank(&kept[idx]) {
                std::mem::replace(&mut kept[idx], track)
            } else {
                track
            };
            if let Some(fname) = absorbed.path.file_name().and_then(|s| s.to_str()) {
                twin_aliases.push((fname.to_string(), absorbed.dense_frame_count));
            }
        } else {
            bucket.push(kept.len());
            kept.push(track);
        }
    }
    Ok((kept, deduplicated, twin_aliases))
}

/// Verifies byte-for-byte payload equality between two candidate twin `.ahash` files.
fn files_have_identical_payload(path_a: &Path, path_b: &Path, frames: u32) -> Result<bool> {
    let mut fa = File::open(path_a)?;
    let mut fb = File::open(path_b)?;
    fa.seek(SeekFrom::Start(HEADER_SIZE_BYTES as u64))?;
    fb.seek(SeekFrom::Start(HEADER_SIZE_BYTES as u64))?;

    let mut rem = (frames as usize) * U64_BYTES;
    let mut buf_a = vec![0u8; STREAM_BUFFER_BYTES];
    let mut buf_b = vec![0u8; STREAM_BUFFER_BYTES];
    while rem > 0 {
        let step = rem.min(STREAM_BUFFER_BYTES);
        fa.read_exact(&mut buf_a[..step])?;
        fb.read_exact(&mut buf_b[..step])?;
        if buf_a[..step] != buf_b[..step] {
            return Ok(false);
        }
        rem -= step;
    }
    Ok(true)
}

/// Streams exact `expected_frames * 8` little-endian payload bytes from `src_path` into `writer`.
fn stream_track_dense_payload<W: Write>(
    src_path: &Path,
    expected_frames: u32,
    writer: &mut W,
) -> Result<()> {
    let mut file = File::open(src_path)
        .with_context(|| format!("Opening {:?} during Pass 2 streaming", src_path))?;
    file.seek(SeekFrom::Start(HEADER_SIZE_BYTES as u64))?;

    let expected_bytes = (expected_frames as usize) * U64_BYTES;
    let mut remaining = expected_bytes;
    let mut buf = vec![0u8; STREAM_BUFFER_BYTES];

    while remaining > 0 {
        let step = remaining.min(STREAM_BUFFER_BYTES);
        file.read_exact(&mut buf[..step]).with_context(|| {
            format!(
                "Truncated .ahash payload while streaming {:?} ({} bytes remaining)",
                src_path, remaining
            )
        })?;
        #[cfg(target_endian = "big")]
        for chunk in buf[..step].chunks_exact_mut(U64_BYTES) {
            let v = u64::from_le_bytes(chunk.try_into().unwrap());
            chunk.copy_from_slice(&v.to_ne_bytes());
        }
        writer.write_all(&buf[..step])?;
        remaining -= step;
    }
    Ok(())
}
