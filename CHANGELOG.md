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
- Ledger on xrpl-rust (M2c).
  - xrpl-rust 1.3.0 is vendored with upstream's own fix for RUSTSEC-2026-0387 (2a5191b), plus TokenEscrow and DomainID, and wired with `[patch.crates-io]`. `json-rpc` is off, crypto-bigint is gone from the lock, and `cargo audit` is clean with no ignores.
  - `league-ledger::client` runs every submission through one path. It checks the network id first. It fills in the sequence, fee and expiry, signs with xrpl-rust and hashes locally before submitting, then waits for a validated result. Accounts are ed25519 only, with keys derived once.
  - Test vectors: 12 validated Testnet transactions from M2b and 9 from M2c re-encode to their blobs and hashes. All 39 of xrpl.js's codec fixtures and its ed25519 keypair fixture also pass (ISC notice in `tests/vectors/LICENSE-xrpl.js`).
  - Live Testnet proof: the M2 escrow suite, TokenEscrow and DomainID, all as expected (`docs/receipts/m2c-patched-xrpl-rust-proof-2026-11-14/`).
  - Coverage 92.19% of lines, floor 90.
