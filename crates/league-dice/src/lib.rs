//! Merchant League dice.
//!
//! A day's roll comes from every merchant's secret plus a public randomness
//! beacon (drand) round published after the reveal deadline. Each secret is
//! committed as an XRPL escrow locked to its SHA-256 hash (a PREIMAGE-SHA-256
//! crypto-condition) and revealed by finishing that escrow, so the ledger
//! checks every reveal and enforces the deadline.
//!
//! **Stubs only.** The condition and fulfillment encoding, the roll, and the
//! drand round rule and signature check arrive with milestone M3, proved on
//! Testnet first.

use std::fmt;

use sha2::{Digest, Sha256};

/// Maximum consecutive entropy draws attempted when rejecting all-zero secrets.
pub const MAX_NONZERO_RETRIES: usize = 8;

/// Errors returned when validating or generating a 32-byte merchant [`Secret`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiceError {
    /// An all-zero (`[0u8; 32]`) secret was supplied or produced by the entropy source.
    DegenerateSecret,
    /// The underlying OS randomness source failed to supply entropy.
    EntropyUnavailable,
}

impl fmt::Display for DiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DegenerateSecret => {
                f.write_str("all-zero 32-byte secret is degenerate and forbidden")
            }
            Self::EntropyUnavailable => f.write_str("system entropy source failed for league-dice"),
        }
    }
}

impl std::error::Error for DiceError {}

/// A merchant's 32-byte secret for one day's roll. Never logged or shown.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub [u8; 32]);

impl Secret {
    /// Returns `true` when the secret is the degenerate all-zero byte array.
    #[must_use]
    pub fn is_degenerate(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Wraps `bytes` into a [`Secret`] after rejecting `[0u8; 32]`.
    pub fn try_from_bytes(bytes: [u8; 32]) -> Result<Self, DiceError> {
        if bytes == [0u8; 32] {
            return Err(DiceError::DegenerateSecret);
        }
        Ok(Self(bytes))
    }

    /// Generates a non-zero [`Secret`] using `fill`, retrying degenerate all-zero draws up to
    /// [`MAX_NONZERO_RETRIES`] times.
    pub fn generate_with<F>(mut fill: F) -> Result<Self, DiceError>
    where
        F: FnMut(&mut [u8; 32]) -> Result<(), DiceError>,
    {
        for _ in 0..MAX_NONZERO_RETRIES {
            let mut buf = [0u8; 32];
            fill(&mut buf)?;
            if buf != [0u8; 32] {
                return Ok(Self(buf));
            }
        }
        Err(DiceError::DegenerateSecret)
    }

    /// Generates a fresh cryptographic non-zero [`Secret`] from OS entropy (`getrandom`).
    pub fn generate() -> Result<Self, DiceError> {
        Self::generate_with(|buf| getrandom::fill(buf).map_err(|_| DiceError::EntropyUnavailable))
    }

    /// Deterministically derives a non-zero [`Secret`] from `seed` and domain `context`.
    #[must_use]
    pub fn derive_from_seed(seed: u64, context: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"league-dice/secret/v2");
        hasher.update(seed.to_le_bytes());
        hasher.update(context);
        let mut bytes: [u8; 32] = hasher.finalize().into();
        if bytes == [0u8; 32] {
            bytes[0] = 1;
        }
        Self(bytes)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<hidden>)")
    }
}

/// A drand round number, named by the rules before reveals open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BeaconRound(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints_its_bytes() {
        let secret = Secret([0xAB; 32]);
        let shown = format!("{secret:?}");
        assert_eq!(shown, "Secret(<hidden>)");
        assert!(!shown.to_lowercase().contains("ab"));
    }

    #[test]
    fn beacon_rounds_order_by_number() {
        assert!(BeaconRound(10) < BeaconRound(11));
    }

    #[test]
    fn secret_rejects_all_zero_and_retries_degenerate_entropy() {
        assert_eq!(
            Secret::try_from_bytes([0u8; 32]),
            Err(DiceError::DegenerateSecret)
        );
        let Ok(ok_secret) = Secret::try_from_bytes([0x11; 32]) else {
            panic!("non-zero secret must succeed");
        };
        assert!(!ok_secret.is_degenerate());

        // Retries initial all-zero draws until non-zero arrives.
        let mut draws = 0_usize;
        let Ok(retried) = Secret::generate_with(|buf| {
            draws += 1;
            if draws < 3 {
                *buf = [0u8; 32];
            } else {
                *buf = [0x42; 32];
            }
            Ok(())
        }) else {
            panic!("retry should succeed once non-zero is drawn");
        };
        assert_eq!(draws, 3);
        assert_eq!(retried.0, [0x42; 32]);
        assert!(!retried.is_degenerate());

        // Persistent all-zero RNG is rejected after MAX_NONZERO_RETRIES attempts.
        let mut zero_draws = 0_usize;
        let err = Secret::generate_with(|buf| {
            zero_draws += 1;
            *buf = [0u8; 32];
            Ok(())
        });
        assert_eq!(err, Err(DiceError::DegenerateSecret));
        assert_eq!(zero_draws, MAX_NONZERO_RETRIES);

        // Entropy failure propagates immediately.
        let io_err = Secret::generate_with(|_| Err(DiceError::EntropyUnavailable));
        assert_eq!(io_err, Err(DiceError::EntropyUnavailable));

        // OS entropy generator and deterministic derivation always return non-zero secrets.
        let Ok(live) = Secret::generate() else {
            panic!("OS entropy generation should succeed");
        };
        assert!(!live.is_degenerate());

        let d1 = Secret::derive_from_seed(42, b"seat-0");
        let d2 = Secret::derive_from_seed(42, b"seat-1");
        assert!(!d1.is_degenerate());
        assert!(!d2.is_degenerate());
        assert_ne!(d1.0, d2.0);
        assert!(format!("{}", DiceError::DegenerateSecret).contains("all-zero"));
        assert!(format!("{}", DiceError::EntropyUnavailable).contains("entropy"));
    }
}
