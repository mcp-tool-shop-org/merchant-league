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
- Guard scripts tightened after Argon's review of the scaffold (PASS WITH FOLLOW-UPS, 2026-11-14).
  - `check-testnet-only.sh` catches the Ripple Mainnet servers on every scheme and bare, and more Mainnet and Devnet hosts. It scans the docs, README and CI as well as the crates.
  - `check-copy.sh` scans the docs, catches inflections, snake_case and "real money", and lets ordinary trading language and the "no real money" disclaimer through.
  - `test-guards.sh`, run in CI, plants every pattern and checks each verdict.
- Guards reworked after Carbon's review of #4 (FAIL, 2026-11-14), as the Publisher ruled the same day: the tests are the Testnet guarantee, and the greps are tripwires with no allow markers.
  - `league-ledger` tests prove `TESTNET_NETWORK_ID` is 1 and that `require_testnet` accepts only `Some(1)`, across a sweep of ids and the extremes. They replace the grep checks on the constant and the match arm, which a second `Ok` arm could bypass. `CLAUDE.md` adds that every public constructor of a ledger client needs a test showing it refuses network id 0 and a missing id.
  - `check-testnet-only.sh` is a host tripwire over the whole repository except `vendor/`. It catches any host on Ripple's own domain (including subdomains like a Clio server), more providers and explorers, and any URL that says mainnet or livenet. The network-id-0 grep is gone: grep can't tell a test or a ledger index from a network id, and the tests own that property.
  - `check-copy.sh` catches more separators (repeated spaces, tabs, camelCase), phrases split across two lines, wagering, jackpots, deposits of funds, cash prizes and real currencies. It accepts more disclaimers (without real money, not for real money). "Real profit" stays banned under the Director's ruling. The licence skip is narrowed to exact names under `crates/`, never a `.rs` file and never in docs.
  - No allow markers anywhere. `docs/design.md`'s rules and risks rows are reworded to need none.
  - `test-guards.sh` replaces lines as well as appending them, has a failing case for every pattern and a near miss for each, and copies the whole repository so the Testnet tripwire's scope is tested as it runs.
- Ledger on xrpl-rust (M2c).
  - xrpl-rust 1.3.0 is vendored with upstream's own fix for RUSTSEC-2026-0387 (2a5191b), plus TokenEscrow and DomainID, and wired with `[patch.crates-io]`. `json-rpc` is off, crypto-bigint is gone from the lock, and `cargo audit` is clean with no ignores. After Carbon's review of #5 (PASS WITH FOLLOW-UPS, 2026-11-14), the crate's own `Cargo.lock`, which pinned the vulnerable versions, is removed.
  - `league-ledger::client` runs every submission through one path. It checks the network id first. It fills in the sequence, fee and expiry, signs with xrpl-rust and hashes locally before submitting, then waits for a validated result. Accounts are ed25519 only, with keys derived once, and a seed that fails to decode gets a fixed error message that carries none of its bytes.
  - A client can't exist without a checked network id: it holds a `verified::Verified` token that only the network check can make, so a constructor that skips the check doesn't compile. Each constructor (`with_transport`, and `connect`'s path) has a test that network id 0, other ids and a missing id are refused after one request. This follows option (a), which the Publisher approved on 2026-11-14 (the Director didn't rule it, whatever an earlier commit message says), and the Publisher's addition that every way to reach the ledger is tested.
  - Test vectors: 12 validated Testnet transactions from M2b and 9 from M2c re-encode to their blobs and hashes. All 39 of xrpl.js's codec fixtures and its ed25519 keypair fixture also pass (ISC notice in `tests/vectors/LICENSE-xrpl.js`, which the copy check skips by exact name).
  - Live Testnet proof: the M2 escrow suite, TokenEscrow and DomainID, all as expected (`docs/receipts/m2c-patched-xrpl-rust-proof-2026-11-14/`).
  - Coverage 92.95% of lines, floor 90.
