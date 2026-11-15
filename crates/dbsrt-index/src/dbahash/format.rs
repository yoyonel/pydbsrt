//! Binary layout specification for the memory-mapped acoustic archive (`.dbahash` v1).
//!
//! Defines the 64-byte cache-line aligned header ([`DbahashHeader`], 128 bytes),
//! per-track descriptor ([`DbahashMediaEntry`], 64 bytes), checksum routine, and
//! compilation statistics ([`DbahashPackStats`]). Every section offset is aligned
//! to [`DBAHASH_SECTION_ALIGN`] (64 bytes) so `memmap2::Mmap` slices cast cleanly
//! to `&[u64]` and `&[Mih32Entry]` on both `x86_64` and `aarch64` (Raspberry Pi 4).

use bytemuck::{Pod, Zeroable};

/// Magic 8-byte signature identifying a `.dbahash` v1 binary archive.
pub const DBAHASH_MAGIC: [u8; 8] = *b"DBAHASH\0";

/// Canonical `.dbahash` binary format version.
pub const DBAHASH_VERSION: u16 = 1;

/// Byte alignment enforced at the start of every binary section (64-byte CPU cache line).
pub const DBAHASH_SECTION_ALIGN: u64 = 64;

/// Header flag indicating embedded pre-sorted `MIH-32` coarse tables (`upper` + `lower`).
pub const DBAHASH_FLAG_HAS_COARSE_MIH32: u16 = 1 << 0;

/// Default coarse decimation stride $K$ (`86.125 / 32 = 2.69 fps`) for `.dbahash` archives.
pub const DEFAULT_DBAHASH_FACTOR_K: usize = 32;

/// Bit flag in [`DbahashMediaEntry::flags`] indicating `moviehash` is populated.
pub const MEDIA_FLAG_HAS_MOVIEHASH: u8 = 1 << 0;

/// Bit flag in [`DbahashMediaEntry::flags`] indicating `audio_index` is populated.
pub const MEDIA_FLAG_HAS_AUDIO_INDEX: u8 = 1 << 1;

/// Bit flag in [`DbahashMediaEntry::flags`] indicating `language` is populated.
pub const MEDIA_FLAG_HAS_LANGUAGE: u8 = 1 << 2;

/// Maximum UTF-8 byte capacity of the inline ISO 639-2 / BCP-47 `language` field in [`DbahashMediaEntry`].
pub const DBAHASH_LANG_CAPACITY: usize = 7;

/// Exact byte size of [`DbahashHeader`] (`2 * 64` bytes).
pub const DBAHASH_HEADER_BYTES: usize = 128;

/// Exact byte size of [`DbahashMediaEntry`] (`1 * 64` bytes).
pub const DBAHASH_MEDIA_ENTRY_BYTES: usize = 64;

/// Number of reserved trailing bytes in [`DbahashHeader`] padding it to 128 bytes.
pub const DBAHASH_HEADER_RESERVED_BYTES: usize = 24;

/// Optional 8-byte trailer magic appended at the end of `strings_table` listing absorbed twin filenames.
pub const DBAHASH_TWIN_TRAILER_MAGIC: [u8; 8] = *b"\0TWINS1\0";

/// Byte size of the little-endian `u32` twin record count following [`DBAHASH_TWIN_TRAILER_MAGIC`].
pub const TWIN_TRAILER_COUNT_BYTES: usize = 4;

/// Byte size of each twin record prefix (`dense_frame_count: u32` + `filename_len: u16`).
pub const TWIN_TRAILER_RECORD_HEADER_BYTES: usize = 6;

/// FNV-1a 64-bit offset basis used to compute deterministic archive header checksums.
const CHECKSUM_FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime multiplier used in [`compute_dbahash_checksum`].
const CHECKSUM_FNV_PRIME: u64 = 0x0100_0000_01b3;

/// Fixed 128-byte header at offset `0` of every `.dbahash` v1 binary archive.
///
/// # Data Flow & Invariants
/// - `magic` must equal [`DBAHASH_MAGIC`] (`b"DBAHASH\0"`).
/// - `version` must equal [`DBAHASH_VERSION`] (`1`).
/// - Every non-zero `offset_*` field is a multiple of [`DBAHASH_SECTION_ALIGN`] (`64`).
/// - **Invariant (ADR-0007)**: Cold `dense_hashes` (`86.125 fps`) are stored last (`thermal layout`) so
///   OS kernel read-ahead on metadata and `MIH-32` tables never pages in cold dense audio blocks.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct DbahashHeader {
    /// Magic signature (`b"DBAHASH\0"`).
    pub magic: [u8; 8],
    /// Format version (`1`).
    pub version: u16,
    /// Bitwise flags (see [`DBAHASH_FLAG_HAS_COARSE_MIH32`]).
    pub flags: u16,
    /// Coarse decimation stride $K$ (`>= 1`, default `32`).
    pub factor_k: u32,
    /// Number of [`DbahashMediaEntry`] records in the media table.
    pub media_count: u32,
    /// Reserved 32-bit slot for 8-byte field alignment (`0`).
    pub reserved_u32: u32,
    /// Cumulative count of 64-bit dense sub-fingerprints across all tracks.
    pub total_dense_frames: u64,
    /// Cumulative count of 64-bit coarse sub-fingerprints (`1/K` decimated).
    pub total_coarse_frames: u64,
    /// 64-byte aligned byte offset to the [`DbahashMediaEntry`] array.
    pub offset_media_table: u64,
    /// 64-byte aligned byte offset to the UTF-8 string pool (`media_name` + `file_path`).
    pub offset_strings_table: u64,
    /// Exact unpadded byte length of valid UTF-8 data inside the string pool.
    pub strings_len: u64,
    /// 64-byte aligned byte offset to the contiguous dense `u64` array (`total_dense_frames * 8` bytes).
    pub offset_dense_hashes: u64,
    /// 64-byte aligned byte offset to the contiguous coarse `u64` array (`0` if omitted).
    pub offset_coarse_hashes: u64,
    /// 64-byte aligned byte offset to the sorted upper `Mih32Entry` table (`0` if omitted).
    pub offset_coarse_mih_upper: u64,
    /// 64-byte aligned byte offset to the sorted lower `Mih32Entry` table (`0` if omitted).
    pub offset_coarse_mih_lower: u64,
    /// Deterministic 64-bit FNV-1a structural checksum over header layout, media entries, and string pool.
    pub checksum: u64,
    /// Zero-filled reserved space padding the header to 128 bytes.
    pub _reserved: [u8; DBAHASH_HEADER_RESERVED_BYTES],
}

const _: () = assert!(std::mem::size_of::<DbahashHeader>() == DBAHASH_HEADER_BYTES);

impl DbahashHeader {
    /// Returns `true` when the archive embeds pre-sorted `MIH-32` coarse tables.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `true` if [`DBAHASH_FLAG_HAS_COARSE_MIH32`] is set in `self.flags`.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Pure bitwise check on `self.flags`.
    #[inline]
    pub fn has_coarse_mih32(&self) -> bool {
        (self.flags & DBAHASH_FLAG_HAS_COARSE_MIH32) != 0
    }
}

/// Fixed 64-byte descriptor for a single audio track inside `.dbahash` v1.
///
/// # Data Flow & Invariants
/// Multi-audio containers (`.ahash`, `.a1.ahash`, `.a2.ahash`) emit distinct
/// [`DbahashMediaEntry`] rows sharing `moviehash` and `media_name` while preserving
/// distinct `audio_index`, `language`, `file_path`, and `dense_start_idx` slices.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct DbahashMediaEntry {
    /// OpenSubtitles 64-bit MovieHash when [`MEDIA_FLAG_HAS_MOVIEHASH`] is set (`0` otherwise).
    pub moviehash: u64,
    /// IEEE-754 bit pattern (`f64::to_bits`) of the track's sub-fingerprint rate (`~86.125 fps`).
    pub audio_fps_bits: u64,
    /// Starting element index into the flat `dense_hashes` `u64` array.
    pub dense_start_idx: u64,
    /// Starting element index into the flat `coarse_hashes` `u64` array.
    pub coarse_start_idx: u64,
    /// Number of dense `u64` frames belonging to this track.
    pub dense_frame_count: u32,
    /// Number of coarse `u64` frames (`ceil(dense_frame_count / K)`) belonging to this track.
    pub coarse_frame_count: u32,
    /// Byte offset of `media_name` inside the UTF-8 string pool.
    pub name_offset: u32,
    /// Byte length of `media_name` inside the UTF-8 string pool.
    pub name_len: u32,
    /// Byte offset of `file_path` inside the UTF-8 string pool.
    pub path_offset: u32,
    /// Byte length of `file_path` inside the UTF-8 string pool.
    pub path_len: u16,
    /// 0-based relative audio stream index (`0:a:<k>`) when [`MEDIA_FLAG_HAS_AUDIO_INDEX`] is set.
    pub audio_index: u16,
    /// Presence bitmask (`MEDIA_FLAG_HAS_*`).
    pub flags: u8,
    /// Inline NUL-padded UTF-8 ISO 639-2 / BCP-47 language tag (up to 7 bytes, e.g. `"fre"`).
    pub language: [u8; DBAHASH_LANG_CAPACITY],
}

const _: () = assert!(std::mem::size_of::<DbahashMediaEntry>() == DBAHASH_MEDIA_ENTRY_BYTES);

impl DbahashMediaEntry {
    /// Decodes the IEEE-754 audio frame rate in Hz.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// Effective sub-fingerprint sampling rate (`f64`, e.g. `86.125`).
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Reconstructs `f64::from_bits(self.audio_fps_bits)`.
    #[inline]
    pub fn audio_fps(&self) -> f64 {
        f64::from_bits(self.audio_fps_bits)
    }

    /// Returns the OpenSubtitles 64-bit MovieHash if recorded.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `Some(moviehash)` when [`MEDIA_FLAG_HAS_MOVIEHASH`] is set, `None` otherwise.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Guards `0` ambiguity using explicit flag bit [`MEDIA_FLAG_HAS_MOVIEHASH`].
    #[inline]
    pub fn moviehash(&self) -> Option<u64> {
        ((self.flags & MEDIA_FLAG_HAS_MOVIEHASH) != 0).then_some(self.moviehash)
    }

    /// Returns the relative audio stream index (`0:a:<k>`) if recorded.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `Some(audio_index)` when [`MEDIA_FLAG_HAS_AUDIO_INDEX`] is set, `None` otherwise.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Distinguishes `Some(0)` (default primary stream) from unknown (`None`).
    #[inline]
    pub fn audio_index(&self) -> Option<usize> {
        ((self.flags & MEDIA_FLAG_HAS_AUDIO_INDEX) != 0).then_some(self.audio_index as usize)
    }

    /// Returns the decoded UTF-8 audio language code if recorded.
    ///
    /// # Arguments
    /// None (`&self`).
    ///
    /// # Returns
    /// `Some(&str)` (e.g. `"fre"`, `"eng"`, `"jpn"`) when [`MEDIA_FLAG_HAS_LANGUAGE`] is set.
    ///
    /// # Panics
    /// Never panics.
    ///
    /// # Data Flow & Invariants
    /// Strips trailing NUL padding bytes and validates UTF-8.
    #[inline]
    pub fn language(&self) -> Option<&str> {
        if (self.flags & MEDIA_FLAG_HAS_LANGUAGE) == 0 {
            return None;
        }
        let len = self
            .language
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(DBAHASH_LANG_CAPACITY);
        std::str::from_utf8(&self.language[..len])
            .ok()
            .filter(|s| !s.is_empty())
    }
}

/// Encodes an optional UTF-8 language code into the inline 7-byte field and flag bit.
///
/// # Arguments
/// * `lang` - Optional ISO 639-2 / BCP-47 language string.
///
/// # Returns
/// Tuple `(flag_bit, [u8; 7])` truncated cleanly on a UTF-8 char boundary if longer than 7 bytes.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// Trims ASCII whitespace; empty strings clear [`MEDIA_FLAG_HAS_LANGUAGE`].
pub(crate) fn encode_inline_language(lang: Option<&str>) -> (u8, [u8; DBAHASH_LANG_CAPACITY]) {
    let mut buf = [0u8; DBAHASH_LANG_CAPACITY];
    let Some(trimmed) = lang.map(str::trim).filter(|s| !s.is_empty()) else {
        return (0, buf);
    };
    let mut valid_len = trimmed.len().min(DBAHASH_LANG_CAPACITY);
    while valid_len > 0 && !trimmed.is_char_boundary(valid_len) {
        valid_len -= 1;
    }
    if valid_len == 0 {
        return (0, buf);
    }
    buf[..valid_len].copy_from_slice(&trimmed.as_bytes()[..valid_len]);
    (MEDIA_FLAG_HAS_LANGUAGE, buf)
}

/// Rounds `offset` up to the next multiple of [`DBAHASH_SECTION_ALIGN`] (`64` bytes).
///
/// # Arguments
/// * `offset` - Raw byte offset.
///
/// # Returns
/// Smallest multiple of `64` greater than or equal to `offset`.
///
/// # Panics
/// Never panics (uses wrapping/mask arithmetic within valid file sizes).
///
/// # Data Flow & Invariants
/// `(offset + 63) & !63`.
#[inline]
pub const fn align_up_64(offset: u64) -> u64 {
    let mask = DBAHASH_SECTION_ALIGN - 1;
    (offset + mask) & !mask
}

/// Computes the deterministic 64-bit FNV-1a checksum over header layout, media table, and strings.
///
/// # Arguments
/// * `header` - Populated [`DbahashHeader`] (`checksum` field itself is ignored during hashing).
/// * `media_entries` - Slice of packed [`DbahashMediaEntry`] records.
/// * `strings` - Unpadded UTF-8 string pool bytes (`&mmap[offset_strings..offset_strings + strings_len]`).
///
/// # Returns
/// 64-bit FNV-1a digest verifying structural and metadata integrity in `< 0.2 ms` at `open()` without
/// paging cold dense hashes into RSS.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// - Zeroes `copy.checksum` before hashing `bytes_of(&copy)`, `cast_slice(media_entries)`, and `strings`.
/// - **Invariant (ADR-0007)**: Intentionally excludes `MIH-32` and `dense_hashes` so `MmapDbahash::open`
///   verifies structural integrity in `< 0.1 ms` without faulting `727+ MiB` of cold pages into `VmRSS`.
pub fn compute_dbahash_checksum(
    header: &DbahashHeader,
    media_entries: &[DbahashMediaEntry],
    strings: &[u8],
) -> u64 {
    let mut copy = *header;
    copy.checksum = 0;

    let mut hash = CHECKSUM_FNV_OFFSET_BASIS;
    for &b in bytemuck::bytes_of(&copy) {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(CHECKSUM_FNV_PRIME);
    }
    for &b in bytemuck::cast_slice::<DbahashMediaEntry, u8>(media_entries) {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(CHECKSUM_FNV_PRIME);
    }
    for &b in strings {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(CHECKSUM_FNV_PRIME);
    }
    hash
}

/// Telemetry summary returned after compiling a `.dbahash` v1 archive.
#[derive(Debug, Clone, PartialEq)]
pub struct DbahashPackStats {
    /// Total `.ahash` files discovered across scanned directories (`0` for programmatic builder).
    pub scanned_files: usize,
    /// Number of unique tracks written into the archive after binary twin deduplication.
    pub packed_tracks: usize,
    /// Number of duplicate binary twin `.ahash` files collapsed.
    pub deduplicated_twins: usize,
    /// Number of unreadable or empty `.ahash` files skipped.
    pub skipped_files: usize,
    /// Downsampling factor $K$ stored in the header.
    pub factor_k: usize,
    /// Whether pre-sorted `MIH-32` coarse tables were embedded.
    pub has_coarse_mih32: bool,
    /// Cumulative dense sub-fingerprint count (`86.125 fps`).
    pub total_dense_frames: u64,
    /// Cumulative coarse sub-fingerprint count (`1/K`).
    pub total_coarse_frames: u64,
    /// Final `.dbahash` file size on disk in bytes.
    pub output_bytes: u64,
    /// Wall-clock compilation time in seconds.
    pub duration_seconds: f64,
}

/// Evicts sequential writeback folios on cold `.dbahash` sections and marks dense hashes random-access.
///
/// # Arguments
/// * `file` - Open file descriptor backing the `.dbahash` archive.
/// * `header` - Validated [`DbahashHeader`] supplying section offsets.
/// * `file_len` - Total archive byte length.
///
/// # Returns
/// Nothing (`()`). No-op on non-Unix targets or empty sections.
///
/// # Panics
/// Never panics.
///
/// # Data Flow & Invariants
/// - Sequential buffered writes create multi-order (`256–512 KiB`) kernel page-cache folios on Linux 6.6+.
/// - **Invariant (ADR-0007)**: `POSIX_FADV_DONTNEED` MUST execute at write-time (`pack-ahash` completion),
///   never inside `MmapDbahash::open` (where synchronous `XArray` eviction across `727 MiB` adds `~64 ms`),
///   ensuring `3.7 KiB` fine-window faults map `order-0` (`4 KiB`) pages while keeping `open() < 1 ms`.
#[cfg(unix)]
pub(crate) fn advise_cold_archive_sections(
    file: &std::fs::File,
    header: &DbahashHeader,
    file_len: u64,
) {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    if header.has_coarse_mih32() && header.offset_coarse_mih_upper > header.offset_coarse_hashes {
        if let (Ok(off), Ok(len)) = (
            libc::off_t::try_from(header.offset_coarse_hashes),
            libc::off_t::try_from(header.offset_coarse_mih_upper - header.offset_coarse_hashes),
        ) {
            // SAFETY: `fd` is a valid open file descriptor and `off`/`len` are non-negative bounds.
            unsafe {
                let _ = libc::posix_fadvise(fd, off, len, libc::POSIX_FADV_DONTNEED);
            }
        }
    }
    if file_len > header.offset_dense_hashes {
        if let (Ok(off), Ok(len)) = (
            libc::off_t::try_from(header.offset_dense_hashes),
            libc::off_t::try_from(file_len - header.offset_dense_hashes),
        ) {
            // SAFETY: `fd` is a valid open file descriptor and `off`/`len` are non-negative bounds.
            unsafe {
                let _ = libc::posix_fadvise(fd, off, len, libc::POSIX_FADV_RANDOM);
                let _ = libc::posix_fadvise(fd, off, len, libc::POSIX_FADV_DONTNEED);
            }
        }
    }
}

/// Non-Unix stub for [`advise_cold_archive_sections`].
#[cfg(not(unix))]
pub(crate) fn advise_cold_archive_sections(
    _file: &std::fs::File,
    _header: &DbahashHeader,
    _file_len: u64,
) {
}
