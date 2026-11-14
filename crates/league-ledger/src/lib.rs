//! Merchant League ledger access. **XRPL Testnet only.**
//!
//! The game never talks to any network but the XRPL Testnet. The endpoints
//! below are the only ones the code knows, and every connection must pass
//! [`require_testnet`] before anything else is sent. A server that reports a
//! different network id, or none, is refused.
//!
//! Transactions are built and signed with xrpl-rust (vendored with upstream's
//! security fix; see `vendor/xrpl-rust/VENDORED.md`) and submitted through
//! [`client::Testnet`].

use std::fmt;

pub mod client;

/// The Testnet's network id, as its servers report it in `server_info`
/// (measured on 2026-11-14: rippled 3.4.1, `network_id` 1).
pub const TESTNET_NETWORK_ID: u32 = 1;

/// The public Testnet JSON-RPC endpoint.
pub const TESTNET_JSON_RPC: &str = "https://s.altnet.rippletest.net:51234";

/// The public Testnet WebSocket endpoint.
pub const TESTNET_WEBSOCKET: &str = "wss://s.altnet.rippletest.net:51233";

/// The public Testnet faucet. Called once per player per season.
pub const TESTNET_FAUCET: &str = "https://faucet.altnet.rippletest.net/accounts";

/// A server is not the Testnet, so the game will not use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotTestnet {
    /// The network id the server reported, if it reported one.
    pub reported: Option<u32>,
}

impl fmt::Display for NotTestnet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reported {
            Some(id) => write!(
                f,
                "this server is on network {id}, not the XRPL Testnet (network {TESTNET_NETWORK_ID}); Merchant League plays on Testnet only"
            ),
            None => write!(
                f,
                "this server reports no network id, so it cannot be confirmed as the XRPL Testnet; Merchant League plays on Testnet only"
            ),
        }
    }
}

impl std::error::Error for NotTestnet {}

/// Accept a server only if it reports the Testnet's network id.
///
/// `reported` is the `network_id` from the server's `server_info`. A server
/// that leaves it out is refused: the game must know where it is playing.
pub fn require_testnet(reported: Option<u32>) -> Result<(), NotTestnet> {
    match reported {
        Some(TESTNET_NETWORK_ID) => Ok(()),
        other => Err(NotTestnet { reported: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_testnet_is_accepted() {
        assert_eq!(require_testnet(Some(1)), Ok(()));
    }

    #[test]
    fn every_other_network_is_refused() {
        for id in [0, 2, 21337, u32::MAX] {
            assert_eq!(
                require_testnet(Some(id)),
                Err(NotTestnet { reported: Some(id) })
            );
        }
    }

    #[test]
    fn a_server_with_no_network_id_is_refused() {
        assert_eq!(require_testnet(None), Err(NotTestnet { reported: None }));
    }

    #[test]
    fn the_refusal_says_testnet_only() {
        let other = NotTestnet { reported: Some(0) }.to_string();
        assert!(other.contains("network 0"));
        assert!(other.contains("Testnet only"));
        let missing = NotTestnet { reported: None }.to_string();
        assert!(missing.contains("no network id"));
        assert!(missing.contains("Testnet only"));
    }

    #[test]
    fn every_known_endpoint_is_a_testnet_host() {
        for url in [TESTNET_JSON_RPC, TESTNET_WEBSOCKET, TESTNET_FAUCET] {
            assert!(url.contains(".altnet.rippletest.net"), "{url}");
        }
    }
}
