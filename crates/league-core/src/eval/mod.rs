//! Evaluation Mode host ownership boundary (`EvalHost`).
//!
//! Only [`host`] inside this module owns [`crate::TurnEngine`] for Evaluation Mode, calls
//! `TurnEngine::state()`, executes sweeps, serializes/deserializes `TurnEngine` inside `.seal`,
//! and performs atomic `.save.json` / `.seal` and checkpoint persistence. All seat-side modules
//! consume solely [`crate::SeatObservation`], [`crate::PublicSweepReceipt`], and seat-safe
//! previews.

pub mod host;

use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};

pub use host::{
    EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, EvalActApplyOutcome, EvalActSubmission,
    EvalHost, EvalSaveSnapshot, EvalSeatApplyReport,
};

/// Deterministic seeded PRNG wrapper (`ChaCha20`) for Evaluation Mode scripted baseline policies.
#[derive(Debug, Clone)]
pub struct PolicyChaChaRng(ChaCha20Rng);

impl PolicyChaChaRng {
    /// Constructs a deterministic [`PolicyChaChaRng`] from a 32-byte domain-separated seed.
    #[must_use]
    pub fn from_seed_bytes(seed: [u8; 32]) -> Self {
        Self(ChaCha20Rng::from_seed(seed))
    }

    /// Generates the next deterministic pseudo-random `u64`.
    #[must_use]
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    /// Draws an unbiased uniform index in `0..upper_exclusive` (`0` when `upper_exclusive <= 1`).
    #[must_use]
    pub fn gen_index(&mut self, upper_exclusive: usize) -> usize {
        if upper_exclusive <= 1 {
            return 0;
        }
        let Ok(bound) = u64::try_from(upper_exclusive) else {
            return 0;
        };
        let zone = u64::MAX - (u64::MAX % bound);
        loop {
            let x = self.0.next_u64();
            if x < zone {
                let rem = x % bound;
                if let Ok(idx) = usize::try_from(rem) {
                    return idx;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PolicyChaChaRng;

    #[test]
    fn policy_chacha_rng_is_deterministic_and_bounded() {
        let mut a = PolicyChaChaRng::from_seed_bytes([0x5a; 32]);
        let mut b = PolicyChaChaRng::from_seed_bytes([0x5a; 32]);
        assert_eq!(a.gen_index(0), 0);
        assert_eq!(b.gen_index(0), 0);
        assert_eq!(a.gen_index(1), 0);
        assert_eq!(b.gen_index(1), 0);
        for _ in 0..32 {
            assert_eq!(a.next_u64(), b.next_u64());
            let idx_a = a.gen_index(17);
            let idx_b = b.gen_index(17);
            assert!(idx_a < 17);
            assert_eq!(idx_a, idx_b);
        }
    }
}
