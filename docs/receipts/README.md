# Receipts

Records of live checks on the XRPL Testnet. Each folder holds a README with what was run and what it showed, and a `raw/` folder with the server's own replies.

| Receipt | What it checked |
|---|---|
| [`m2-xrpl-rust-spike-2026-11-14/`](m2-xrpl-rust-spike-2026-11-14/) | xrpl-rust 1.3.0 builds, signs and submits every transaction the league needs |
| [`m2b-own-layer-proof-2026-11-14/`](m2b-own-layer-proof-2026-11-14/) | the same steps on the league's own XRPL layer |
| [`m2c-patched-xrpl-rust-proof-2026-11-14/`](m2c-patched-xrpl-rust-proof-2026-11-14/) | the patched xrpl-rust: TokenEscrow, DomainID, a permissioned domain |
| [`design-check-requireauth-2026-11-14/`](design-check-requireauth-2026-11-14/) | RequireAuth, clawback and token escrow on one trust line |
| [`design-check-escrow-expiry-2026-11-14/`](design-check-escrow-expiry-2026-11-14/) | which expiry an escrow needs |

## These receipts outlive the ledger they describe

The XRPL Testnet is wiped from time to time. The next reset is on 2026-11-25, announced on xrpl.org (["Upcoming Testnet and Devnet Resets"](https://xrpl.org/blog/2026/testnet-devnet-reset), 2026-11-09). A reset deletes every account, balance, offer and escrow; the network's URLs and id stay the same.

All five receipts here were taken on 2026-11-14, before that reset. After it:
- **The transaction hashes and accounts they name no longer resolve.** Looking one up on the Testnet, or in an explorer, finds nothing. That is the reset, not a sign the receipt is wrong.
- **The evidence is what was recorded at the time:** the server replies in each `raw/` folder, and, for the M2 spike and the M2c proof, the signed transactions kept as test vectors in [`crates/league-ledger/tests/vectors/`](../../crates/league-ledger/tests/vectors/) (`testnet-spike-2026-11-14.json`, `testnet-m2c-2026-11-14.json`). The vectors are checked offline by `cargo test`, so they keep working after a reset.
- **To see it live again, re-run the check.** The live examples in `crates/league-ledger/examples/` fund fresh faucet wallets each run, so they work on the reset network. A re-run is a new receipt, with its own date.
