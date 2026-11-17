//! XRPL `PREIMAGE-SHA-256` escrow condition and fulfillment encoding, plus the `EscrowFinish` fee rule.
//!
//! This module encodes the on-ledger XRPL crypto-condition (`A0258020` + `SHA-256(secret)` +
//! `810120`) and fulfillment (`A0228020` + `secret`) for 32-byte preimage secrets used by
//! `EscrowCreate` and `EscrowFinish`.
//!
//! **Not interchangeable with `league-core` commitments:** `league-core`'s in-engine
//! `PreimageSecret` commitment (`league_core::commit::compute_preimage_condition` in `commit.rs`)
//! hashes a domain-separated payload (`"league-core/preimage/v2" || secret`) rather than encoding
//! an XRPL DER crypto-condition, and the two are not interchangeable.

use sha2::{Digest, Sha256};

/// Byte length of the XRPL `PREIMAGE-SHA-256` fulfillment for a 32-byte secret
/// (`A0228020` 4-byte DER prefix plus 32-byte preimage).
pub const ESCROW_FINISH_FULFILLMENT_BYTES: u64 = 36;

/// Default `EscrowFinish` transaction fee in drops (`500`), leaving headroom above the `350`-drop
/// minimum for a 32-byte preimage at a `10`-drop base fee.
pub const ESCROW_FINISH_DEFAULT_FEE_DROPS: &str = "500";

/// Minimum `EscrowFinish` fee in drops required by XRPL for a 32-byte `PREIMAGE-SHA-256`
/// fulfillment at `base_fee_drops`: `base_fee_drops * (33 + 36 / 16) = 35 * base_fee_drops`.
#[must_use]
pub const fn escrow_finish_min_fee_drops(base_fee_drops: u64) -> u64 {
    base_fee_drops.saturating_mul(33 + ESCROW_FINISH_FULFILLMENT_BYTES / 16)
}

/// Encodes the uppercase hex XRPL `PREIMAGE-SHA-256` `Condition` (`A0258020` + `SHA-256(secret)` +
/// `810120`) for a 32-byte `secret`.
///
/// This is the XRPL ledger crypto-condition, **not** `league-core`'s in-engine
/// `PreimageSecret` commitment (`commit.rs`), and the two are not interchangeable.
#[must_use]
pub fn condition_hex(secret: &[u8; 32]) -> String {
    let digest = Sha256::digest(secret);
    format!("A0258020{}810120", crate::upper_hex(&digest))
}

/// Encodes the uppercase hex XRPL `PREIMAGE-SHA-256` `Fulfillment` (`A0228020` + `secret`) for a
/// 32-byte `secret`.
///
/// This is the XRPL ledger fulfillment, **not** `league-core`'s in-engine `PreimageSecret`
/// commitment (`commit.rs`), and the two are not interchangeable.
#[must_use]
pub fn fulfillment_hex(secret: &[u8; 32]) -> String {
    format!("A0228020{}", crate::upper_hex(secret))
}

/// Encodes the XRPL `PREIMAGE-SHA-256` `(Condition, Fulfillment)` uppercase hex pair for a 32-byte
/// `secret`.
///
/// This is the XRPL ledger crypto-condition, **not** `league-core`'s in-engine `PreimageSecret`
/// commitment (`commit.rs`), and the two are not interchangeable.
#[must_use]
pub fn escrow_condition(secret: &[u8; 32]) -> (String, String) {
    (condition_hex(secret), fulfillment_hex(secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_secret_matches_published_preimage_sha256_vector() {
        let (cond, fulfil) = escrow_condition(&[0u8; 32]);
        assert_eq!(
            cond,
            "A025802066687AADF862BD776C8FC18B8E9F8E20089714856EE233B3902A591D0D5F2925810120"
        );
        assert_eq!(fulfil, format!("A0228020{}", "00".repeat(32)));
        assert_eq!(condition_hex(&[0u8; 32]), cond);
        assert_eq!(fulfillment_hex(&[0u8; 32]), fulfil);
    }

    #[test]
    fn finish_fee_rule_computes_expected_drops() {
        assert_eq!(ESCROW_FINISH_FULFILLMENT_BYTES, 36);
        assert_eq!(escrow_finish_min_fee_drops(10), 350);
        assert_eq!(escrow_finish_min_fee_drops(12), 420);
        assert_eq!(ESCROW_FINISH_DEFAULT_FEE_DROPS, "500");
    }
}
