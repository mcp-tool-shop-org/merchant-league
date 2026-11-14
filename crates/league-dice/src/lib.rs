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

/// A merchant's 32-byte secret for one day's roll. Never logged or shown.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub [u8; 32]);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
}
