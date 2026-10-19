//! Multi-Index Hashing (MIH) for 64-bit perceptual hashes.
//!
//! Based on Norouzi, Punjani, Fleet (IEEE TPAMI 2014):
//! "Fast Exact Search in Hamming Space with Multi-Index Hashing".
//!
//! Splits 64-bit pHash into 4 blocks of 16-bit (2^16 = 65,536 buckets).
//! By the Pigeonhole Principle, for any two 64-bit hashes with Hamming distance <= K,
//! at least one 16-bit block has Hamming distance <= floor(K / 4).
//!
//! For K <= 7, floor(K / 4) <= 1:
//! Only 1 (exact) + 16 (1-bit flip) = 17 buckets need to be inspected per table.

use dbsrt_core::hamming_distance;
use rayon::prelude::*;

pub const NUM_TABLES: usize = 4;
pub const BUCKETS_PER_TABLE: usize = 65536;

/// Extracted 16-bit chunk from 64-bit hash for table `t` (0..4).
#[inline(always)]
pub fn extract_chunk(hash: u64, table_idx: usize) -> u16 {
    match table_idx {
        0 => (hash >> 48) as u16,
        1 => ((hash >> 32) & 0xFFFF) as u16,
        2 => ((hash >> 16) & 0xFFFF) as u16,
        _ => (hash & 0xFFFF) as u16,
    }
}

/// Inverted table for one 16-bit chunk using Compressed Sparse Row (CSR) layout.
#[derive(Debug, Clone)]
pub struct MihTable {
    /// Offsets into `indices` array. Length = 65537.
    pub offsets: Vec<u32>,
    /// Contiguous array of item indices. Length = N.
    pub indices: Vec<u32>,
}

impl MihTable {
    /// Builds CSR table from an iterator of 16-bit chunk values.
    pub fn build(chunks: &[u16]) -> Self {
        let n = chunks.len();
        let mut counts = vec![0u32; BUCKETS_PER_TABLE];
        for &c in chunks {
            counts[c as usize] += 1;
        }

        let mut offsets = Vec::with_capacity(BUCKETS_PER_TABLE + 1);
        let mut sum = 0u32;
        offsets.push(0);
        for &cnt in &counts {
            sum += cnt;
            offsets.push(sum);
        }

        let mut write_pos = offsets.clone();
        let mut indices = vec![0u32; n];
        for (idx, &c) in chunks.iter().enumerate() {
            let bucket = c as usize;
            let pos = write_pos[bucket];
            indices[pos as usize] = idx as u32;
            write_pos[bucket] += 1;
        }

        Self { offsets, indices }
    }

    /// Slice of candidate indices for a given bucket.
    #[inline(always)]
    pub fn bucket_indices(&self, bucket: u16) -> &[u32] {
        let b = bucket as usize;
        let start = self.offsets[b] as usize;
        let end = self.offsets[b + 1] as usize;
        &self.indices[start..end]
    }
}

/// Multi-Index Hash table over 64-bit hashes.
#[derive(Debug, Clone)]
pub struct MultiIndexHash64 {
    pub len: usize,
    pub tables: [MihTable; NUM_TABLES],
}

/// A candidate match found by MIH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MihCandidate {
    pub index: usize,
    pub distance: u32,
}

/// Abstract provider for indexed 64-bit perceptual hashes.
pub trait HashProvider: Sync {
    fn get_hash(&self, index: usize) -> u64;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl HashProvider for [u64] {
    #[inline(always)]
    fn get_hash(&self, index: usize) -> u64 {
        self[index]
    }
    #[inline(always)]
    fn len(&self) -> usize {
        <[u64]>::len(self)
    }
}

impl HashProvider for Vec<u64> {
    #[inline(always)]
    fn get_hash(&self, index: usize) -> u64 {
        self[index]
    }
    #[inline(always)]
    fn len(&self) -> usize {
        self.len()
    }
}

impl HashProvider for [crate::dbphash::DbphashCutEntry] {
    #[inline(always)]
    fn get_hash(&self, index: usize) -> u64 {
        self[index].phash
    }
    #[inline(always)]
    fn len(&self) -> usize {
        <[crate::dbphash::DbphashCutEntry]>::len(self)
    }
}

/// Iterates over all 16-bit bucket indices within `max_chunk_dist` (0, 1, or 2 bit flips) of `base`.
///
/// - dist 0: 1 bucket (exact)
/// - dist 1: 16 buckets (single bit flips)
/// - dist 2: 120 buckets (two bit flips)
#[inline(always)]
pub fn for_each_chunk_neighbor<F: FnMut(u16)>(base: u16, max_chunk_dist: u32, mut f: F) {
    // 1. Distance 0: Exact bucket
    f(base);

    // 2. Distance 1: 16 single-bit flip buckets
    if max_chunk_dist >= 1 {
        for bit in 0..16 {
            f(base ^ (1 << bit));
        }
    }

    // 3. Distance 2: 120 two-bit flip buckets (for max_distance >= 8)
    if max_chunk_dist >= 2 {
        for bit1 in 0..15 {
            for bit2 in (bit1 + 1)..16 {
                f(base ^ (1 << bit1) ^ (1 << bit2));
            }
        }
    }
}

impl MultiIndexHash64 {
    /// Builds MIH index from a hash provider in parallel across the 4 tables.
    pub fn build_from_provider<P: HashProvider + ?Sized>(provider: &P) -> Self {
        let n = provider.len();

        let tables: Vec<MihTable> = (0..NUM_TABLES)
            .into_par_iter()
            .map(|t| {
                let chunks: Vec<u16> = (0..n)
                    .map(|i| extract_chunk(provider.get_hash(i), t))
                    .collect();
                MihTable::build(&chunks)
            })
            .collect();

        Self {
            len: n,
            tables: [
                tables[0].clone(),
                tables[1].clone(),
                tables[2].clone(),
                tables[3].clone(),
            ],
        }
    }

    /// Builds MIH index from a slice of 64-bit hashes.
    pub fn build(hashes: &[u64]) -> Self {
        Self::build_from_provider(hashes)
    }

    /// Search candidates within `max_distance` Hamming distance.
    ///
    /// Implements Asymmetric Substring Search Radii (Norouzi et al. 2014, Proposition 2 / Theorem 2.1):
    /// For radius $r = m \cdot r_0 + a$ with $0 \le a < m$, it is mathematically sufficient
    /// to search only the first $a + 1$ tables at radius $r_0$, and the remaining $m - (a + 1)$
    /// tables at radius $r_0 - 1$.
    ///
    /// For $K=6, m=4$: $6 = 4 \times 1 + 2 \implies r_0 = 1, a = 2$.
    /// - Tables 0, 1, 2 search at radius 1 (17 buckets each).
    /// - Table 3 searches at radius 0 (exact bucket only, zero 1-bit flips).
    ///
    /// Total buckets examined: $3 \times 17 + 1 = 52$ buckets instead of 68 (-23.5% lookups).
    ///
    /// Deduplication is performed using Disjoint Substrings (Norouzi et al. 2014 §3.2):
    /// Table `t` only evaluates candidates that had distance > table_radii[prev_t] in all
    /// preceding tables `prev_t < t`. This guarantees exact 1-to-1 candidate generation with
    /// zero deduplication array and zero DRAM allocation overhead.
    pub fn search<P: HashProvider + ?Sized>(
        &self,
        query_hash: u64,
        max_distance: u32,
        provider: &P,
    ) -> Vec<MihCandidate> {
        let mut results = Vec::new();

        // Asymmetric Radii (Theorem 2.1): r = m * r0 + a
        let m = NUM_TABLES as u32;
        let r0 = max_distance / m;
        let a = max_distance % m;

        // Radii per table: tables 0..=a get r0; tables (a+1)..m get r0 - 1
        let mut table_radii = [0i32; NUM_TABLES];
        for (i, r) in table_radii.iter_mut().enumerate() {
            *r = if (i as u32) <= a {
                r0 as i32
            } else {
                r0 as i32 - 1
            };
        }

        let query_chunks: [u16; NUM_TABLES] = [
            extract_chunk(query_hash, 0),
            extract_chunk(query_hash, 1),
            extract_chunk(query_hash, 2),
            extract_chunk(query_hash, 3),
        ];

        for (t, table) in self.tables.iter().enumerate() {
            let radius = table_radii[t];
            if radius < 0 {
                continue; // Table can be completely skipped (e.g. if max_distance < 3)
            }

            let base_chunk = query_chunks[t];

            for_each_chunk_neighbor(base_chunk, radius as u32, |bucket| {
                for &cand_idx in table.bucket_indices(bucket) {
                    let uidx = cand_idx as usize;
                    let cand_hash = provider.get_hash(uidx);

                    // Disjoint Substrings check: skip if matched in any previous active table
                    let mut already_visited = false;
                    for (prev_t, &q_prev) in query_chunks.iter().enumerate().take(t) {
                        let prev_rad = table_radii[prev_t];
                        if prev_rad >= 0 {
                            let h_prev = extract_chunk(cand_hash, prev_t);
                            if (q_prev ^ h_prev).count_ones() <= prev_rad as u32 {
                                already_visited = true;
                                break;
                            }
                        }
                    }

                    if !already_visited {
                        let dist = hamming_distance(query_hash, cand_hash);
                        if dist <= max_distance {
                            results.push(MihCandidate {
                                index: uidx,
                                distance: dist,
                            });
                        }
                    }
                }
            });
        }

        results
    }

    /// Parallel multi-query search.
    pub fn search_parallel<P: HashProvider + ?Sized>(
        &self,
        query_hashes: &[u64],
        max_distance: u32,
        provider: &P,
    ) -> Vec<(usize, Vec<MihCandidate>)> {
        query_hashes
            .par_iter()
            .enumerate()
            .map(|(q_idx, &q_hash)| {
                let matches = self.search(q_hash, max_distance, provider);
                (q_idx, matches)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_chunk() {
        let hash: u64 = 0x1234_5678_9ABC_DEF0;
        assert_eq!(extract_chunk(hash, 0), 0x1234);
        assert_eq!(extract_chunk(hash, 1), 0x5678);
        assert_eq!(extract_chunk(hash, 2), 0x9ABC);
        assert_eq!(extract_chunk(hash, 3), 0xDEF0);
    }

    #[test]
    fn test_mih_exact_and_near_match() {
        let base = 0xAAAA_BBBB_CCCC_DDDD;
        let hashes = vec![
            base,
            base ^ 0x0001,                // dist 1 (chunk 3 flip)
            base ^ 0x0001_0000,           // dist 1 (chunk 2 flip)
            base ^ 0x0000_0001_0000_0000, // dist 1 (chunk 1 flip)
            base ^ (1 << 63),             // dist 1 (chunk 0 flip)
            base ^ 0x0003,                // dist 2 (chunk 3 two flips)
            0x1111_2222_3333_4444,        // distant
        ];

        let index = MultiIndexHash64::build(&hashes);

        // Search with dist <= 1
        let matches = index.search(base, 1, &hashes);
        assert_eq!(matches.len(), 5); // base + 4 items at dist 1

        // Search with dist <= 2
        let matches_2 = index.search(base, 2, &hashes);
        assert_eq!(matches_2.len(), 6); // base + 4 dist 1 + 1 dist 2
    }

    #[test]
    fn test_mih_vs_linear_ground_truth() {
        // Simple deterministic Xorshift64 PRNG to avoid external rand dependency
        let mut state: u64 = 0x853c_49e6_748f_ea9b;
        let mut next_u64 = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let n = 10_000;
        let mut dataset = Vec::with_capacity(n);
        for _ in 0..n {
            dataset.push(next_u64());
        }

        let index = MultiIndexHash64::build(&dataset);

        // Run 50 pseudo-random queries with max_dist = 6
        for _ in 0..50 {
            let q = next_u64();
            let max_dist = 6;

            // Ground truth linear scan
            let mut expected: Vec<(usize, u32)> = dataset
                .iter()
                .enumerate()
                .filter_map(|(idx, &h)| {
                    let d = hamming_distance(q, h);
                    if d <= max_dist {
                        Some((idx, d))
                    } else {
                        None
                    }
                })
                .collect();
            expected.sort_by_key(|&(idx, _)| idx);

            // MIH search
            let mut actual: Vec<(usize, u32)> = index
                .search(q, max_dist, &dataset)
                .into_iter()
                .map(|c| (c.index, c.distance))
                .collect();
            actual.sort_by_key(|&(idx, _)| idx);

            assert_eq!(
                actual, expected,
                "MIH search must match exact linear ground truth for dist <= 6"
            );
        }
    }
}
