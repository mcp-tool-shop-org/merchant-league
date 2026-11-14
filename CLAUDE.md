# Merchant League

A slow-turn multiplayer trading game for 2 to 6 players, played entirely on the XRPL **Testnet**. Every move is a ledger transaction, and every Testnet reset ends a season. The design is in `docs/design.md`.

**Status: scaffold.** No game rules yet. The rules wait for the outside design review (research seed 25) to be ruled on; see "Milestones".

## Hard rules (non-negotiable)

- **Testnet only. No Mainnet code path, ever.** The only endpoints in the code are the Testnet ones in `league-ledger`. Every connection passes `require_testnet` (network id 1) before anything else is sent; a server reporting any other id, or none, is refused. `scripts/check-testnet-only.sh` fails CI on any Mainnet or Devnet host (every scheme, and bare) in the crates, scripts, docs, README or CI, on any network id of 0 written as code, and if the guard's constant changes. A test that must name one carries `testnet-check: allow`.
- **No real-money language.** Game words only: marks, table coin, goods, purse, caravan, merchant. Ordinary trading language is fine ("buy in Varn", "sell at a profit"). Never buy-in, cash out, payout (or "pay out", so write "pay from"), invest, wallet value, withdraw funds, earn or win money. `scripts/check-copy.sh` fails CI on those in the crates, README and docs; the disclaimer "no real money" is allowed. A line that must name one to forbid it carries `copy-check: allow`.
  - The Director ruled on 2026-11-14 that ordinary trading words, such as profit, selling at a profit or buying in a town, are game language and stay allowed. Framing the game as earning, paying out or withdrawing money stays banned.
- **Guards are tested.** `scripts/test-guards.sh` plants every pattern and checks each verdict. A new pattern gets a planted case there.
- **One faucet call per player per season.** Keep the seed on the player's machine (`.league/`, git-ignored). On HTTP 429, wait and retry with backoff; never loop.
- **Secrets never print.** Seeds, dice secrets and keys are redacted in `Debug` output and never logged.
- **Resets are designed in.** The announced-reset list and the ledger-went-backwards check live in `league-season`, ported from xrpl-lab's `resets.py`. A new announced reset is added in xrpl-lab, xrpl-camp, sovereignty and here.
- **Only XRPL features enabled on Mainnet.** After a reset the Testnet's amendments are set to match Mainnet's, so a Testnet-only feature could vanish. Check the amendment status before using a new transaction type.

## Toolchain

- `rust-toolchain.toml` pins the current stable Rust by exact version (1.100.0, released 2026-11-09). Check the stable channel when bumping; never write a version from memory. A new stable lands by PR.
- Edition 2024 throughout.

## Tables and roll

- Game tables (road events, market shocks, town tables) are roll tables in `tables/`. CI validates them and computes their exact odds with roll 2.1.1, pinned, through roll's library API (its 2.1.1 command line reads only the older loot format). Node is CI-only, never in the game.
- `league-core` reads the same JSON in Rust. Every draw's value comes from `league-dice`, never from a local RNG.
- Do not write the roll conformance test, or depend on anything newer than roll 2.1.1, until the conformance fixture ships. Whether roll becomes a Rust crate (roll 3.0.0) is decided after M2.
- roll changes go to R&D as tickets (`rnd-local/projects/antigravity/tickets/`), never straight to the roll repo. Credit roll (MIT, same org) in the notes wherever its code or design is used.

## Workflow

- Branch per change; one PR per milestone; the Publisher merges. Once a PR's head is sent for merge, push nothing more to it.
- Stage files explicitly. Never `git add .` or `git add -A`.
- Before every push: `cargo fmt --all --check`, `cargo clippy --all-targets --workspace -- -D warnings`, `cargo test --workspace`, both check scripts and `scripts/test-guards.sh`, `npx --yes @dogfood-lab/atlas@1.26.0 check` (after committing; new files need `atlas map` and a commit), and the identity scan on `git archive HEAD`.
- Advisories are fixed by upgrading, never ignored (`cargo audit --deny warnings`, no `--ignore`).
- Coverage: the floor is the latest measurement, rounded down, ratcheting toward 90. Never lower it.

## Milestones (Publisher's order, 2026-11-14)

1. **Scaffold** (this PR): workspace, crates with types and stubs, CI, Atlas map.
2. **M2, Testnet spike:** prove `xrpl-rust` (pinned exactly) on Testnet: issuer, trust line, offer, check, escrow with a crypto-condition and its finish, clawback. A receipt. Decides the library.
3. **M3, dice live:** commit and reveal through escrows plus a verified drand round, including the "did not reveal" path. A receipt.
4. **M1, rules:** `league-core`, only after the design review has been ruled on.
5. **M4:** one table of three people on Testnet, with a receipt.

The client is a ratatui and crossterm terminal client, `league-tui` (decided 2026-11-14). A stub until the rules exist.

## Crates

| Crate | Holds |
| --- | --- |
| `league-core` | Game rules. Pure, no I/O, deterministic from a seed. Types only for now. |
| `league-dice` | Commit and reveal through hash-locked escrows, the roll, the drand round rule. Stubs for now. |
| `league-ledger` | Testnet access. The network guard and endpoints now; transactions in M2. |
| `league-season` | Announced resets, the ledger-went-backwards check; the season record later. |
| `league-tui` | The terminal client: title screen and key handling for now. |
