//! Deterministic counter-mode SHA-256 PRNG stream for seeded table generation.

use sha2::{Digest, Sha256};

/// Deterministic pseudorandom byte and number generator keyed by a `u64` seed
/// and an explicit domain-separation label.
#[derive(Debug, Clone)]
pub struct SeedStream {
    base_hasher: Sha256,
    counter: u64,
}

impl SeedStream {
    /// Creates a new deterministic stream from `seed` and domain tag `domain`.
    #[must_use]
    pub fn new(seed: u64, domain: &[u8]) -> Self {
        let mut base_hasher = Sha256::new();
        base_hasher.update(b"league-gen-v2\0");
        base_hasher.update(seed.to_be_bytes());
        base_hasher.update((domain.len() as u64).to_be_bytes());
        base_hasher.update(domain);
        Self {
            base_hasher,
            counter: 0,
        }
    }

    /// Generates the next 32 deterministic bytes by advancing the block counter.
    #[must_use]
    pub fn next_block(&mut self) -> [u8; 32] {
        let mut hasher = self.base_hasher.clone();
        hasher.update(self.counter.to_be_bytes());
        self.counter = self.counter.wrapping_add(1);
        hasher.finalize().into()
    }

    /// Returns the next pseudorandom `u64`.
    #[must_use]
    pub fn next_u64(&mut self) -> u64 {
        let block = self.next_block();
        u64::from_be_bytes([
            block[0], block[1], block[2], block[3], block[4], block[5], block[6], block[7],
        ])
    }

    /// Returns an unbiased integer uniformly distributed in `0..bound`.
    ///
    /// Returns `0` when `bound == 0`.
    #[must_use]
    pub fn next_u32_bounded(&mut self, bound: u32) -> u32 {
        if bound <= 1 {
            return 0;
        }
        let span = u64::from(bound);
        let limit = u64::MAX - (u64::MAX % span);
        loop {
            let candidate = self.next_u64();
            if candidate < limit {
                return (candidate % span) as u32;
            }
        }
    }

    /// Returns an `f64` uniformly distributed in `[0.0, 1.0)`.
    #[must_use]
    pub fn next_f64_unit(&mut self) -> f64 {
        let top53 = self.next_u64() >> 11;
        (top53 as f64) * (1.0 / ((1u64 << 53) as f64))
    }

    /// Returns an `f64` uniformly distributed in `[low, high)` (or `low` if `low >= high`).
    #[must_use]
    pub fn range_f64(&mut self, low: f64, high: f64) -> f64 {
        if high <= low {
            return low;
        }
        low + (high - low) * self.next_f64_unit()
    }

    /// Performs an unbiased Fisher-Yates shuffle on `slice`.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        if slice.len() <= 1 {
            return;
        }
        for end in (1..slice.len()).rev() {
            let idx = self.next_u32_bounded((end + 1) as u32) as usize;
            slice.swap(idx, end);
        }
    }

    /// Samples `k` distinct indices uniformly from `0..n`.
    #[must_use]
    pub fn sample_distinct_indices(&mut self, n: usize, k: usize) -> Vec<usize> {
        let take = k.min(n);
        let mut pool: Vec<usize> = (0..n).collect();
        for i in 0..take {
            let remaining = (n - i) as u32;
            let offset = self.next_u32_bounded(remaining) as usize;
            pool.swap(i, i + offset);
        }
        pool.truncate(take);
        pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_is_deterministic_and_domain_separated() {
        let mut a = SeedStream::new(42, b"alpha");
        let mut b = SeedStream::new(42, b"alpha");
        let mut c = SeedStream::new(42, b"beta");

        for _ in 0..8 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut a2 = SeedStream::new(42, b"alpha");
        assert_ne!(a2.next_u64(), c.next_u64());
    }

    #[test]
    fn bounded_and_unit_ranges_stay_in_bounds() {
        let mut s = SeedStream::new(999, b"bounds");
        assert_eq!(s.next_u32_bounded(0), 0);
        assert_eq!(s.next_u32_bounded(1), 0);
        assert!((s.range_f64(2.0, 2.0) - 2.0).abs() < 1e-12);
        assert!((s.range_f64(3.0, 1.0) - 3.0).abs() < 1e-12);

        for _ in 0..100 {
            let v = s.next_u32_bounded(7);
            assert!(v < 7);
            let u = s.next_f64_unit();
            assert!((0.0..1.0).contains(&u));
            let r = s.range_f64(-0.5, 1.5);
            assert!((-0.5..1.5).contains(&r));
        }

        let mut single = [7u8];
        s.shuffle(&mut single);
        assert_eq!(single, [7]);

        let picked = s.sample_distinct_indices(6, 4);
        assert_eq!(picked.len(), 4);
        let mut unique = picked.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 4);
        assert!(unique.iter().all(|&x| x < 6));
    }
}
