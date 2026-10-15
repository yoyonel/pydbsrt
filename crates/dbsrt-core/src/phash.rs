//! Bit-for-bit compatible pHash computation (2D DCT-II on 32x32 grayscale).
//! Matches Python `imagehash.phash(image, hash_size=8, highfreq_factor=4)`.

use std::f64::consts::PI;

/// Compute precomputed C matrix.
fn generate_c_matrix() -> [[f64; 32]; 8] {
    let mut c = [[0.0f64; 32]; 8];
    for (k, row) in c.iter_mut().enumerate() {
        for (n, val) in row.iter_mut().enumerate() {
            *val = 2.0 * ((PI * (2.0 * n as f64 + 1.0) * k as f64) / 64.0).cos();
        }
    }
    c
}

/// Computes the 64-bit perceptual hash of a 32x32 grayscale image buffer (1024 bytes).
/// Result matches Python `imagehash.phash` and `imghash_to_signed_int64` (as i64).
pub fn phash_32x32(pixels: &[u8; 1024]) -> u64 {
    // 1. Precomputed DCT projection matrix
    // In production, we cache this statically.
    static C_MATRIX: std::sync::OnceLock<[[f64; 32]; 8]> = std::sync::OnceLock::new();
    let c = C_MATRIX.get_or_init(generate_c_matrix);

    // 2. Step 1: T = C @ X
    // C is (8, 32), X is (32, 32), T is (8, 32)
    let mut t = [[0.0f64; 32]; 8];
    for k in 0..8 {
        for col in 0..32 {
            let mut sum = 0.0f64;
            for row in 0..32 {
                let pixel = pixels[row * 32 + col] as f64;
                sum += c[k][row] * pixel;
            }
            t[k][col] = sum;
        }
    }

    // 3. Step 2: R = T @ C.T
    // T is (8, 32), C.T is (32, 8), R is (8, 8)
    let mut dct_low = [0.0f64; 64];
    for r in 0..8 {
        for c_idx in 0..8 {
            let mut sum = 0.0f64;
            for n in 0..32 {
                sum += t[r][n] * c[c_idx][n];
            }
            dct_low[r * 8 + c_idx] = sum;
        }
    }

    // 4. Compute median of the 64 values (matching numpy.median)

    // numpy.median sorts the array and takes the average of element 31 and 32 (0-indexed).
    let mut sorted = dct_low;
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = (sorted[31] + sorted[32]) / 2.0;

    // 5. Pack into 64-bit integer (MSB first, C-order / row-major)
    let mut hash: u64 = 0;
    for &val in &dct_low {
        hash = (hash << 1) | if val > median { 1 } else { 0 };
    }

    hash
}

/// Compute Hamming distance between two 64-bit perceptual hashes.
/// Uses native hardware POPCNT CPU instruction.
#[inline(always)]
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hamming_distance() {
        assert_eq!(hamming_distance(0, 0), 0);
        assert_eq!(hamming_distance(0, 1), 1);
        assert_eq!(hamming_distance(0b1010, 0b0101), 4);
    }

    #[test]
    fn test_phash_32x32_deterministic() {
        let bytes = include_bytes!("../tests/seed42.bin");
        let hash = phash_32x32(bytes);
        println!("Rust computed hash: {:016x}", hash);
        assert_eq!(hash, 0xf72cf0ea5ec4130a);
    }
}
