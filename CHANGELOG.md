# Changelog

## [Unreleased]

- Repository created (design stage).
- Scaffold: a Cargo workspace (Rust 1.100.0, the current stable, edition 2024) with five crates.
  - `league-core`: identifier types only. The game rules wait for the outside design review.
  - `league-dice`: secret and beacon-round types; a secret never prints its bytes.
  - `league-ledger`: the Testnet-only guard. A server must report network id 1, and any other id, or none, is refused. Only the Testnet endpoints exist.
  - `league-season`: the announced 2026-11-25 Testnet reset, its window across every timezone, and the ledger-went-backwards check (1,000-ledger tolerance), ported from xrpl-lab.
  - `league-tui`: the terminal client (ratatui 0.30.2, crossterm 0.29.0), a stub: the title screen and key handling.
- CI (`ci.yml`): format, clippy with warnings denied, tests, coverage, `cargo audit --deny warnings` with no ignores, a Testnet-only check, a no-real-money-language check, and `atlas check`.
- `CLAUDE.md`, `SECURITY.md` and `docs/design.md`, which now covers writing game tables for roll (@mcptoolshop/roll 2.1.1).
- Coverage 100% (173 of 173 lines), floor 90.
- M2: the XRPL layer.
  - `league-dice` encodes a secret as the PREIMAGE-SHA-256 condition (for EscrowCreate) and fulfillment (for EscrowFinish), with the finish's fee.
  - An xrpl-rust 1.3.0 spike did every needed transaction on Testnet, but xrpl-rust is not adopted: its crypto-bigint 0.5 (RUSTSEC-2026-0387) has no fixed version it can take. Receipt in `docs/receipts/m2-xrpl-rust-spike-2026-11-14/`.
  - `league-ledger` has its own layer instead: ed25519 keys, a serializer for the ten transaction types, and a JSON-RPC client that computes each hash before submitting. It is checked against twelve transactions captured from Testnet, and proved live with the same outcome as the spike (`docs/receipts/m2b-own-layer-proof-2026-11-14/`).
