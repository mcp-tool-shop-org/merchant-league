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
  - `check-testnet-only.sh` catches `s.ripple.com` on every scheme and bare, more Mainnet and Devnet hosts, and a network id of 0 in any spelling. It now scans the docs, README and CI as well as the crates, and requires the guard's constant on its own line, used by `require_testnet`.
  - `check-copy.sh` scans the docs, catches inflections, snake_case and "real money", and lets ordinary trading language and the "no real money" disclaimer through.
  - `test-guards.sh`, run in CI, plants every pattern and checks each verdict.
