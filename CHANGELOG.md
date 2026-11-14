# Changelog

## [Unreleased]

- Repository created (design stage).
- Scaffold: a Cargo workspace (Rust 1.98.1, edition 2024) with four crates.
  - `league-core`: identifier types only. The game rules wait for the outside design review.
  - `league-dice`: secret and beacon-round types; a secret never prints its bytes.
  - `league-ledger`: the Testnet-only guard. A server must report network id 1, and any other id, or none, is refused. Only the Testnet endpoints exist.
  - `league-season`: the announced 2026-11-25 Testnet reset, its window across every timezone, and the ledger-went-backwards check (1,000-ledger tolerance), ported from xrpl-lab.
- CI (`ci.yml`): format, clippy with warnings denied, tests, coverage, `cargo audit --deny warnings` with no ignores, a Testnet-only check, a no-real-money-language check, and `atlas check`.
- `CLAUDE.md`, `SECURITY.md` and `docs/design.md`.
- Coverage 100% (117 of 117 lines), floor 90.
