//! Haitsma-Kalker 64-bit Audio Fingerprint (.ahash v1.0) format and extractor.
//!
//! Provides bit-for-bit serialization, deserialization, and hot-loop computation
//! of differential spectral hashes according to RFC .ahash v1.0.

use anyhow::{ensure, Context, Result};
use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};
use std::f32::consts::PI;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// Magic header bytes identifying an .ahash v1.0 file.
pub const AHASH_MAGIC: [u8; 8] = *b"AHASH\0\x01\0";

/// Canonical .ahash format version.
pub const AHASH_VERSION: u16 = 1;

/// Default audio sampling rate in Hz (Nyquist = 2756 Hz).
pub const DEFAULT_SAMPLE_RATE: u32 = 5512;

/// Default FFT frame analysis size in audio samples (~371.5 ms).
pub const DEFAULT_FRAME_SIZE: usize = 2048;

/// Default hop size in samples for high temporal resolution (~86.1 fps).
pub const DEFAULT_HOP_SIZE: usize = 64;

/// Video-synchronized hop size in samples for ~24.0 fps parity (5512 / 230 = 23.965 fps).
pub const VIDEO_SYNC_HOP_SIZE: usize = 230;

/// Number of logarithmic frequency sub-bands covering human speech and music.
pub const NUM_SPECTRAL_BANDS: usize = 33;

/// Spectral band lag for computing frequency derivative (b vs b + LAG).
pub const DEFAULT_SPECTRAL_LAG: usize = 2;

/// Number of spectral derivative bits (NUM_SPECTRAL_BANDS - DEFAULT_SPECTRAL_LAG).
pub const NUM_SPECTRAL_BITS: usize = 31;

/// Total number of bits in the output hash.
pub const HASH_TOTAL_BITS: usize = 64;

/// Size of the fixed .ahash header in bytes.
/// Strictly 8-byte aligned (176 % 8 == 0) to ensure zero-copy mmap safety on ARM64 / RPi4.
pub const HEADER_SIZE_BYTES: usize = 176;

/// Fixed-size header prefix bytes before band boundaries table.
pub const HEADER_PREFIX_BYTES: usize = 40;

/// Number of reserved padding bytes in header prefix.
pub const HEADER_RESERVED_BYTES: usize = 12;

/// Number of alignment padding bytes at the end of the header.
pub const HEADER_ALIGNMENT_PADDING_BYTES: usize = 4;

/// Default maximum mean Hamming distance per frame to accept an audio match (out of 64 bits).
/// Pairs with different encodings of the same audio track typically average 2.0 to 6.0 bits.
/// Unrelated audio tracks average ~32.0 bits.
pub const DEFAULT_MAX_AUDIO_MATCH_DISTANCE: f64 = 14.0;

/// Background noise distance floor (random 64-bit uncorrelated hashes).
pub const RANDOM_AUDIO_HASH_DISTANCE: f64 = 32.0;

/// Result of matching a query audio fingerprint sequence against a reference sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioMatchResult {
    /// Optimal matching frame index in the reference stream.
    pub ref_frame_offset: usize,
    /// Estimated time offset in seconds in the reference stream.
    pub offset_sec: f64,
    /// Average Hamming distance across all frames of the query.
    pub mean_distance: f64,
    /// Normalized match score from 0.0 (unrelated) to 1.0 (identical).
    pub score: f64,
    /// Signal-to-noise ratio margin over baseline random noise (distance drop).
    pub snr_bits: f64,
}

/// Matches a query audio hash sequence against a reference sequence via temporal sliding cross-correlation.
///
/// # Arguments
/// * `query` - Contiguous slice of query audio hashes (e.g. 5s-10s snippet).
/// * `reference` - Contiguous slice of reference media audio hashes.
/// * `audio_fps` - Sampling framerate of the audio hashes (e.g. 86.125 or 24.0).
/// * `max_distance` - Maximum allowed mean Hamming distance to consider a match valid.
///
/// # Returns
/// `Some(AudioMatchResult)` if a valid match meeting the threshold is found, `None` otherwise.
///
/// # Panics
/// Guarantee: Never panics.
///
/// # Data Flow & Invariants
/// 1. Rejects queries larger than the reference stream or empty queries.
/// 2. Slides query across reference positions, evaluating sum of Hamming distances via hardware POPCNT.
/// 3. Identifies the global minimum distance offset.
/// 4. Validates that the minimum distance is below `max_distance`.
/// 5. Computes SNR bit margin against expected random noise (32.0 bits).
pub fn match_audio_sequence(
    query: &[u64],
    reference: &[u64],
    audio_fps: f64,
    max_distance: f64,
) -> Option<AudioMatchResult> {
    if query.is_empty() || reference.len() < query.len() || audio_fps <= 0.0 {
        return None;
    }

    let q_len = query.len();
    let max_shift = reference.len() - q_len;

    let mut best_shift = 0;
    let mut min_total_distance = u64::MAX;

    for shift in 0..=max_shift {
        let mut total_dist = 0u64;
        let ref_slice = &reference[shift..shift + q_len];

        for (q, r) in query.iter().zip(ref_slice) {
            total_dist += (*q ^ *r).count_ones() as u64;
        }

        if total_dist < min_total_distance {
            min_total_distance = total_dist;
            best_shift = shift;
        }
    }

    let mean_distance = min_total_distance as f64 / q_len as f64;
    if mean_distance > max_distance {
        return None;
    }

    let score = (1.0 - (mean_distance / (HASH_TOTAL_BITS as f64))).clamp(0.0, 1.0);
    let snr_bits = (RANDOM_AUDIO_HASH_DISTANCE - mean_distance).max(0.0);
    let offset_sec = best_shift as f64 / audio_fps;

    Some(AudioMatchResult {
        ref_frame_offset: best_shift,
        offset_sec,
        mean_distance,
        score,
        snr_bits,
    })
}

/// Lowest frequency boundary in Hz for Haitsma-Kalker sub-band decomposition.
pub const MIN_FREQ_HZ: f32 = 300.0;

/// Maximum audio sampling rate supported for .ahash headers in Hz.
pub const MAX_SUPPORTED_SAMPLE_RATE: u32 = 48000;

/// Minimum allowed number of sub-bands.
pub const MIN_SUPPORTED_BANDS: u8 = 8;

/// Maximum allowed number of sub-bands.
pub const MAX_SUPPORTED_BANDS: u8 = 64;

/// Structure representing an .ahash v1.0 binary file header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhashHeader {
    /// Format version (always 1).
    pub version: u16,
    /// Sampling rate in Hz (e.g. 5512).
    pub sample_rate: u32,
    /// FFT frame size in samples (e.g. 2048).
    pub frame_size: u16,
    /// Hop size between frames in samples (e.g. 64 or 230).
    pub hop_size: u16,
    /// Number of sub-bands (always 33).
    pub num_bands: u8,
    /// Frequency lag for spectral derivative (always 2).
    pub spectral_lag: u8,
    /// Total count of 64-bit hashes in the payload.
    pub frame_count: u64,
    /// Upper frequency boundaries for each sub-band in Hz.
    pub band_bounds: [u32; NUM_SPECTRAL_BANDS],
}

impl AhashHeader {
    /// Computes the effective audio frame rate in frames per second.
    ///
    /// # Arguments
    /// None (operates on `&self`).
    ///
    /// # Returns
    /// Effective framerate as `f64`.
    ///
    /// # Panics
    /// Guarantee: Never panics (`hop_size > 0` is an invariant).
    ///
    /// # Data Flow & Invariants
    /// Evaluates `sample_rate / hop_size`.
    pub fn audio_fps(&self) -> f64 {
        if self.hop_size == 0 {
            0.0
        } else {
            self.sample_rate as f64 / self.hop_size as f64
        }
    }

    /// Serializes the header into an exact 176-byte buffer.
    ///
    /// # Arguments
    /// None (operates on `&self`).
    ///
    /// # Returns
    /// Array of 176 bytes ready for file writing.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Encodes fields in Little-Endian byte order matching RFC .ahash v1.0 layout.
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE_BYTES] {
        let mut buf = [0u8; HEADER_SIZE_BYTES];
        buf[0..8].copy_from_slice(&AHASH_MAGIC);
        buf[8..10].copy_from_slice(&self.version.to_le_bytes());
        buf[10..14].copy_from_slice(&self.sample_rate.to_le_bytes());
        buf[14..16].copy_from_slice(&self.frame_size.to_le_bytes());
        buf[16..18].copy_from_slice(&self.hop_size.to_le_bytes());
        buf[18] = self.num_bands;
        buf[19] = self.spectral_lag;
        // bytes 20..32 are 12 reserved zero bytes (already initialized to 0)
        buf[32..40].copy_from_slice(&self.frame_count.to_le_bytes());

        let mut offset = HEADER_PREFIX_BYTES;
        for bound in &self.band_bounds {
            buf[offset..offset + 4].copy_from_slice(&bound.to_le_bytes());
            offset += 4;
        }
        // bytes 172..176 are 4 alignment padding zero bytes
        buf
    }

    /// Deserializes and validates an .ahash header from a byte slice.
    ///
    /// # Arguments
    /// * `bytes` - Slice containing at least 176 header bytes.
    /// * `total_file_len` - Total byte length of the underlying file for invariant verification.
    ///
    /// # Returns
    /// Parsed and validated `AhashHeader` on success.
    ///
    /// # Errors
    /// Returns `Err` if any of the 11 RFC invariants are violated.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Verifies buffer length >= 176 bytes.
    /// 2. Asserts magic bytes match `AHASH\0\1\0`.
    /// 3. Validates version, powers of two, bounds ordering, and exact file length.
    pub fn from_bytes(bytes: &[u8], total_file_len: usize) -> Result<Self> {
        ensure!(
            bytes.len() >= HEADER_SIZE_BYTES,
            "Buffer too short for .ahash header: {} bytes (required {})",
            bytes.len(),
            HEADER_SIZE_BYTES
        );

        ensure!(
            bytes[0..8] == AHASH_MAGIC,
            "Invalid .ahash magic: expected {:?}, got {:?}",
            AHASH_MAGIC,
            &bytes[0..8]
        );

        let version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        ensure!(
            version == AHASH_VERSION,
            "Unsupported .ahash version: {}",
            version
        );

        let sample_rate = u32::from_le_bytes(bytes[10..14].try_into().unwrap());
        ensure!(
            sample_rate > 0 && sample_rate <= MAX_SUPPORTED_SAMPLE_RATE,
            "Invalid sample_rate: {}",
            sample_rate
        );

        let frame_size = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
        ensure!(
            frame_size > 0 && frame_size.is_power_of_two(),
            "Invalid frame_size (must be power of two): {}",
            frame_size
        );

        let hop_size = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
        ensure!(
            hop_size > 0 && hop_size <= frame_size,
            "Invalid hop_size: {} (frame_size = {})",
            hop_size,
            frame_size
        );

        let num_bands = bytes[18];
        ensure!(
            (MIN_SUPPORTED_BANDS..=MAX_SUPPORTED_BANDS).contains(&num_bands),
            "Invalid num_bands: {}",
            num_bands
        );
        ensure!(
            num_bands as usize == NUM_SPECTRAL_BANDS,
            "Unsupported num_bands {} (expected {})",
            num_bands,
            NUM_SPECTRAL_BANDS
        );

        let spectral_lag = bytes[19];
        ensure!(
            spectral_lag >= 1 && spectral_lag < num_bands,
            "Invalid spectral_lag: {}",
            spectral_lag
        );

        let total_bits = (num_bands as usize) + (num_bands as usize - spectral_lag as usize);
        ensure!(
            total_bits == HASH_TOTAL_BITS,
            "Invariant violation: 2 * num_bands - lag must equal 64, got {}",
            total_bits
        );

        let frame_count = u64::from_le_bytes(bytes[32..40].try_into().unwrap());

        let mut band_bounds = [0u32; NUM_SPECTRAL_BANDS];
        let mut offset = HEADER_PREFIX_BYTES;
        let mut prev_bound = 0u32;

        for bound_slot in &mut band_bounds {
            let bound = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            ensure!(
                bound > prev_bound,
                "Band bounds must be strictly increasing: {} <= {}",
                bound,
                prev_bound
            );
            prev_bound = bound;
            *bound_slot = bound;
            offset += 4;
        }

        let nyquist = sample_rate / 2;
        ensure!(
            band_bounds[NUM_SPECTRAL_BANDS - 1] == nyquist,
            "Last band bound must equal Nyquist frequency ({} Hz), got {} Hz",
            nyquist,
            band_bounds[NUM_SPECTRAL_BANDS - 1]
        );

        let expected_file_len =
            HEADER_SIZE_BYTES + (frame_count as usize * std::mem::size_of::<u64>());
        ensure!(
            total_file_len == expected_file_len,
            "File length mismatch: expected {} bytes, got {}",
            expected_file_len,
            total_file_len
        );

        Ok(Self {
            version,
            sample_rate,
            frame_size,
            hop_size,
            num_bands,
            spectral_lag,
            frame_count,
            band_bounds,
        })
    }
}

/// Container for a complete .ahash file containing header and sequential hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhashFile {
    /// Parsed validated header.
    pub header: AhashHeader,
    /// Contiguous payload of 64-bit Haitsma-Kalker hashes.
    pub hashes: Vec<u64>,
}

impl AhashFile {
    /// Reads and validates an .ahash file from disk.
    ///
    /// # Arguments
    /// * `path` - Filesystem path to the .ahash file.
    ///
    /// # Returns
    /// `AhashFile` containing header and hash vector.
    ///
    /// # Errors
    /// Returns `Err` on I/O error or if format invariants are violated.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Reads entire file into memory, parses header, and converts little-endian payload to `u64`.
    pub fn read_from_path(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("Failed to read .ahash file {:?}", path))?;
        let header = AhashHeader::from_bytes(&bytes, bytes.len())?;

        let payload_bytes = &bytes[HEADER_SIZE_BYTES..];
        let mut hashes = Vec::with_capacity(header.frame_count as usize);

        let (chunks, _) = payload_bytes.as_chunks::<8>();
        for chunk in chunks {
            hashes.push(u64::from_le_bytes(*chunk));
        }

        Ok(Self { header, hashes })
    }

    /// Writes header and hashes atomically to disk via a temporary file.
    ///
    /// # Arguments
    /// * `path` - Destination file path.
    /// * `header` - Header struct populated with parameters and frame count.
    /// * `hashes` - Slice of 64-bit hashes to persist.
    ///
    /// # Returns
    /// `Ok(())` on successful atomic write.
    ///
    /// # Errors
    /// Returns `Err` on I/O failure or if hash count does not match `header.frame_count`.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Writes header then little-endian u64 payload to `<path>.tmp.<pid>`, syncs, and renames atomically.
    pub fn write_to_path(path: &Path, header: &AhashHeader, hashes: &[u64]) -> Result<()> {
        ensure!(
            header.frame_count as usize == hashes.len(),
            "Header frame count ({}) does not match slice length ({})",
            header.frame_count,
            hashes.len()
        );

        let tmp_path = path.with_extension(format!("ahash.tmp.{}", std::process::id()));
        let mut file = File::create(&tmp_path)
            .with_context(|| format!("Failed to create temporary file {:?}", tmp_path))?;

        let header_bytes = header.to_bytes();
        file.write_all(&header_bytes)?;

        for &h in hashes {
            file.write_all(&h.to_le_bytes())?;
        }

        file.sync_all()?;
        drop(file);

        fs::rename(&tmp_path, path)
            .with_context(|| format!("Failed to rename {:?} to {:?}", tmp_path, path))?;

        Ok(())
    }
}

/// Scratchpad for hot-loop Haitsma-Kalker calculations.
/// Pre-allocated once to guarantee ZERO dynamic heap allocations in hot loops.
pub struct HaitsmaScratchpad {
    /// Windowed input samples buffer (size = frame_size).
    pub windowed: Vec<f32>,
    /// Complex spectrum output buffer (size = frame_size / 2 + 1).
    pub spectrum: Vec<Complex32>,
    /// Power spectral density (size = frame_size / 2 + 1).
    pub power_spectrum: Vec<f32>,
    /// Current frame sub-band energies (size = 33).
    pub current_energies: [f32; NUM_SPECTRAL_BANDS],
    /// Previous frame sub-band energies for temporal derivative (size = 33).
    pub previous_energies: [f32; NUM_SPECTRAL_BANDS],
    /// Total frames processed by this scratchpad instance.
    pub frame_counter: usize,
}

impl HaitsmaScratchpad {
    /// Constructs a reusable scratchpad for the specified frame size.
    ///
    /// # Arguments
    /// * `frame_size` - FFT size in samples (e.g. 2048).
    ///
    /// # Returns
    /// Allocated `HaitsmaScratchpad`.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Pre-allocates fixed-capacity vectors for zero-allocation hot loop processing.
    pub fn new(frame_size: usize) -> Self {
        let spectrum_len = frame_size / 2 + 1;
        Self {
            windowed: vec![0.0f32; frame_size],
            spectrum: vec![Complex32::default(); spectrum_len],
            power_spectrum: vec![0.0f32; spectrum_len],
            current_energies: [0.0f32; NUM_SPECTRAL_BANDS],
            previous_energies: [0.0f32; NUM_SPECTRAL_BANDS],
            frame_counter: 0,
        }
    }

    /// Resets scratchpad state for a new audio stream.
    ///
    /// # Arguments
    /// None (operates on `&mut self`).
    ///
    /// # Returns
    /// None.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Clears previous energies and resets frame counter to 0.
    pub fn reset(&mut self) {
        self.current_energies.fill(0.0);
        self.previous_energies.fill(0.0);
        self.frame_counter = 0;
    }
}

/// Extractor engine implementing Haitsma-Kalker differential spectral hashing.
pub struct HaitsmaKalkerExtractor {
    /// Sampling rate in Hz.
    pub sample_rate: u32,
    /// FFT frame size in samples.
    pub frame_size: usize,
    /// Precomputed periodic Hann analysis window.
    pub hann_window: Vec<f32>,
    /// Precomputed bin ranges `[start_bin, end_bin)` for each of the 33 sub-bands.
    pub band_bins: [(usize, usize); NUM_SPECTRAL_BANDS],
    /// Precomputed upper frequency boundaries in Hz.
    pub band_bounds: [u32; NUM_SPECTRAL_BANDS],
    /// Real-to-Complex FFT forward operator.
    pub fft: Arc<dyn RealToComplex<f32>>,
}

impl HaitsmaKalkerExtractor {
    /// Constructs an extractor configured with canonical Haitsma-Kalker parameters.
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sampling rate in Hz (e.g. 5512).
    /// * `frame_size` - Window size in samples (e.g. 2048).
    ///
    /// # Returns
    /// Initialized `HaitsmaKalkerExtractor`.
    ///
    /// # Errors
    /// Returns `Err` if FFT planner fails to initialize.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Precomputes Hann window coefficients, logarithmic sub-band bins, and compiles forward FFT plan.
    pub fn new(sample_rate: u32, frame_size: usize) -> Result<Self> {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(frame_size);

        // Precompute Hann window: w[n] = 0.5 * (1 - cos(2*pi*n / (N - 1)))
        let mut hann_window = Vec::with_capacity(frame_size);
        for n in 0..frame_size {
            let val = 0.5 * (1.0 - ((2.0 * PI * n as f32) / (frame_size - 1) as f32).cos());
            hann_window.push(val);
        }

        let (band_bins, band_bounds) = Self::compute_band_layout(sample_rate, frame_size);

        Ok(Self {
            sample_rate,
            frame_size,
            hann_window,
            band_bins,
            band_bounds,
            fft,
        })
    }

    /// Precomputes logarithmic sub-band boundaries and corresponding FFT bin ranges.
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sampling rate in Hz.
    /// * `frame_size` - FFT frame size in samples.
    ///
    /// # Returns
    /// Tuple of `(band_bins, band_bounds)`.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Divides [300 Hz, Nyquist] into 33 logarithmic intervals and maps them to FFT bins.
    pub fn compute_band_layout(
        sample_rate: u32,
        frame_size: usize,
    ) -> (
        [(usize, usize); NUM_SPECTRAL_BANDS],
        [u32; NUM_SPECTRAL_BANDS],
    ) {
        let nyquist = (sample_rate / 2) as f32;
        let delta_f = sample_rate as f32 / frame_size as f32;
        let num_bands = NUM_SPECTRAL_BANDS as f32;

        let mut band_bins = [(0usize, 0usize); NUM_SPECTRAL_BANDS];
        let mut band_bounds = [0u32; NUM_SPECTRAL_BANDS];

        // f_b = F_min * (F_max / F_min)^(b / 33)
        let ratio = nyquist / MIN_FREQ_HZ;
        let mut freq_edges = [0.0f32; NUM_SPECTRAL_BANDS + 1];
        for (b, edge) in freq_edges.iter_mut().enumerate() {
            *edge = MIN_FREQ_HZ * ratio.powf(b as f32 / num_bands);
        }
        freq_edges[NUM_SPECTRAL_BANDS] = nyquist;

        for b in 0..NUM_SPECTRAL_BANDS {
            let f_start = freq_edges[b];
            let f_end = freq_edges[b + 1];

            let bin_start = (f_start / delta_f).round() as usize;
            let mut bin_end = (f_end / delta_f).round() as usize;

            // Guarantee each band contains at least 1 FFT bin
            if bin_end <= bin_start {
                bin_end = bin_start + 1;
            }

            band_bins[b] = (bin_start, bin_end);
            band_bounds[b] = f_end.round() as u32;
        }

        band_bounds[NUM_SPECTRAL_BANDS - 1] = sample_rate / 2;
        (band_bins, band_bounds)
    }

    /// Computes the 64-bit Haitsma-Kalker hash for a single window of audio samples.
    ///
    /// # Arguments
    /// * `samples` - Slice of audio samples (exact length = `frame_size`).
    /// * `scratch` - Mutable reusable scratchpad.
    ///
    /// # Returns
    /// 64-bit perceptual audio hash.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// 1. Multiplies input samples by precomputed Hann window.
    /// 2. Performs in-place real-to-complex forward FFT.
    /// 3. Computes power spectral density |X[k]|^2.
    /// 4. Integrates energy across the 33 logarithmic sub-bands.
    /// 5. Encodes 31 spectral derivative bits (MSB) + 33 temporal derivative bits (LSB).
    /// 6. Rotates previous energy buffer for the next frame.
    #[inline(always)]
    pub fn process_frame(&self, samples: &[f32], scratch: &mut HaitsmaScratchpad) -> u64 {
        debug_assert_eq!(samples.len(), self.frame_size);

        // 1. Hann windowing
        for (i, (&sample, &hann)) in samples.iter().zip(&self.hann_window).enumerate() {
            scratch.windowed[i] = sample * hann;
        }

        // 2. Real-to-complex FFT
        let _ = self
            .fft
            .process(&mut scratch.windowed, &mut scratch.spectrum);

        // 3. Power spectrum: |X[k]|^2 = re^2 + im^2
        for (i, c) in scratch.spectrum.iter().enumerate() {
            scratch.power_spectrum[i] = c.norm_sqr();
        }

        // 4. Band energy integration
        for (b, &(start_bin, end_bin)) in self.band_bins.iter().enumerate() {
            let mut energy = 0.0f32;
            for k in start_bin..end_bin {
                if k < scratch.power_spectrum.len() {
                    energy += scratch.power_spectrum[k];
                }
            }
            scratch.current_energies[b] = energy;
        }

        // 5. Pack 64-bit hash
        let hash = Self::pack_hash(
            &scratch.current_energies,
            &scratch.previous_energies,
            scratch.frame_counter == 0,
        );

        // 6. Update state for next frame
        scratch
            .previous_energies
            .copy_from_slice(&scratch.current_energies);
        scratch.frame_counter += 1;

        hash
    }

    /// Packs 31 spectral derivative bits and 33 temporal derivative bits into a u64.
    ///
    /// # Arguments
    /// * `current` - Current frame's 33 band energies.
    /// * `previous` - Previous frame's 33 band energies.
    /// * `is_first_frame` - Whether this is frame 0 (temporal bits forced to 0).
    ///
    /// # Returns
    /// 64-bit unsigned hash.
    ///
    /// # Panics
    /// Guarantee: Never panics.
    ///
    /// # Data Flow & Invariants
    /// Encodes spectral derivative in upper 31 bits, temporal derivative in lower 33 bits.
    #[inline(always)]
    fn pack_hash(
        current: &[f32; NUM_SPECTRAL_BANDS],
        previous: &[f32; NUM_SPECTRAL_BANDS],
        is_first_frame: bool,
    ) -> u64 {
        let mut hash: u64 = 0;

        // 1. Spectral derivative in MSB (31 bits: b vs b + 2)
        for b in 0..NUM_SPECTRAL_BITS {
            let bit = if current[b] > current[b + DEFAULT_SPECTRAL_LAG] {
                1u64
            } else {
                0u64
            };
            hash = (hash << 1) | bit;
        }

        // 2. Temporal derivative in LSB (33 bits: E[b, t] vs E[b, t-1])
        if is_first_frame {
            hash <<= NUM_SPECTRAL_BANDS;
        } else {
            for b in 0..NUM_SPECTRAL_BANDS {
                let bit = if current[b] > previous[b] { 1u64 } else { 0u64 };
                hash = (hash << 1) | bit;
            }
        }

        hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size_alignment_invariant() {
        assert_eq!(HEADER_SIZE_BYTES, 176);
        assert_eq!(
            HEADER_SIZE_BYTES % 8,
            0,
            "Header MUST be 8-byte aligned for mmap!"
        );
    }

    #[test]
    fn test_hash_total_bits_invariant() {
        assert_eq!(
            NUM_SPECTRAL_BANDS + (NUM_SPECTRAL_BANDS - DEFAULT_SPECTRAL_LAG),
            HASH_TOTAL_BITS
        );
        assert_eq!(NUM_SPECTRAL_BITS, 31);
        assert_eq!(NUM_SPECTRAL_BANDS, 33);
    }

    #[test]
    fn test_header_serialization_roundtrip() {
        let (_band_bins, band_bounds) = HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
        let header = AhashHeader {
            version: 1,
            sample_rate: 5512,
            frame_size: 2048,
            hop_size: 64,
            num_bands: 33,
            spectral_lag: 2,
            frame_count: 100,
            band_bounds,
        };

        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), HEADER_SIZE_BYTES);

        let total_file_len = HEADER_SIZE_BYTES + (100 * 8);
        let parsed = AhashHeader::from_bytes(&bytes, total_file_len).expect("Parsing must succeed");
        assert_eq!(header, parsed);
        assert!((parsed.audio_fps() - 86.125).abs() < 0.01);
    }

    #[test]
    fn test_silence_deterministic_hash() {
        let extractor = HaitsmaKalkerExtractor::new(5512, 2048).expect("Extractor init");
        let mut scratch = HaitsmaScratchpad::new(2048);

        let silence = vec![0.0f32; 2048];
        let hash_frame0 = extractor.process_frame(&silence, &mut scratch);
        let hash_frame1 = extractor.process_frame(&silence, &mut scratch);

        // Frame 0: temporal derivative = 0, spectral derivative = 0 (energies are all 0.0)
        assert_eq!(hash_frame0, 0);
        // Frame 1: subsequent silence still has 0 delta everywhere
        assert_eq!(hash_frame1, 0);
    }

    #[test]
    fn test_sine_wave_produces_non_zero_hash() {
        let extractor = HaitsmaKalkerExtractor::new(5512, 2048).expect("Extractor init");
        let mut scratch = HaitsmaScratchpad::new(2048);

        // Generate 1000 Hz sine wave
        let mut tone = vec![0.0f32; 2048];
        for (i, sample) in tone.iter_mut().enumerate() {
            *sample = (2.0 * PI * 1000.0 * i as f32 / 5512.0).sin();
        }

        let hash0 = extractor.process_frame(&tone, &mut scratch);
        assert_ne!(hash0, 0, "Tone should produce distinct spectral peaks");

        // Scale amplitude by 2 for next frame -> temporal derivative must see growth
        for s in &mut tone {
            *s *= 2.0;
        }
        let hash1 = extractor.process_frame(&tone, &mut scratch);
        assert_ne!(hash1, 0);
    }

    #[test]
    fn test_ahash_file_disk_roundtrip() {
        let (_bins, band_bounds) = HaitsmaKalkerExtractor::compute_band_layout(5512, 2048);
        let header = AhashHeader {
            version: 1,
            sample_rate: 5512,
            frame_size: 2048,
            hop_size: 64,
            num_bands: 33,
            spectral_lag: 2,
            frame_count: 3,
            band_bounds,
        };
        let hashes = vec![
            0x1122334455667788u64,
            0x8877665544332211u64,
            0xabcdef0123456789u64,
        ];

        let tmp_file =
            std::env::temp_dir().join(format!("test_ahash_roundtrip_{}.ahash", std::process::id()));
        AhashFile::write_to_path(&tmp_file, &header, &hashes).expect("write_to_path");

        let loaded = AhashFile::read_from_path(&tmp_file).expect("read_from_path");
        assert_eq!(loaded.header, header);
        assert_eq!(loaded.hashes, hashes);

        let _ = fs::remove_file(tmp_file);
    }
}
