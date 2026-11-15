# merchant-league: how it works

Mapped at 2026-11-15 from commit 5ee0322 by Atlas 1.26.0.

## What this is

Merchant League: a slow-turn trading game for 2 to 6 players, played only on the XRPL Testnet. (written by a person)

9 parts, mostly Rust (315 files), Python (3), shell (3) and JavaScript (1). Work enters through 6 doors; the busiest is CI, which reaches 6 parts. People run league-relay. xrpl is a command built from vendor/xrpl-rust (nothing ships it).

## What changed since 2026-11-15 (3354384)

Nothing structural changed since 2026-11-15; 1 file changed content.

## What comes in

1. **CI.** On a pull request; on a push to main touching 9 paths; or by hand. Runs scripts/check-copy.sh, scripts/check-testnet-only.sh, scripts/test-guards.sh and 11 more; checks crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs.
2. **testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`). Runs crates/league-ledger/examples/testnet_escrow_expiry.rs.
3. **testnet_proof** (a command people run with `cargo run --example testnet_proof`). Runs crates/league-ledger/examples/testnet_proof.rs.
4. **testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`). Runs crates/league-ledger/examples/testnet_requireauth.rs.
5. **league-relay** (a command people run). Runs crates/league-relay/src/main.rs.
6. **xrpl** (a command built from vendor/xrpl-rust, which nothing ships). Runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What happens through CI

1. The workflow runs scripts/check-copy.sh, scripts/check-testnet-only.sh and scripts/test-guards.sh in scripts, crates/league-core/ in league-core, crates/league-dice/ in league-dice, crates/league-ledger/examples/, crates/league-ledger/src/ and crates/league-ledger/tests/vectors.rs in league-ledger, crates/league-season/ in league-season, and 2 files in 2 more places; it checks crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs.

## Who reads the results

CI writes nothing in the files this map could read; 1 file could not be.

## The other doors

**testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`) runs crates/league-ledger/examples/testnet_escrow_expiry.rs.

**testnet_proof** (a command people run with `cargo run --example testnet_proof`) runs crates/league-ledger/examples/testnet_proof.rs.

**testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`) runs crates/league-ledger/examples/testnet_requireauth.rs.

**league-relay** (a command people run) runs crates/league-relay/src/main.rs.

**xrpl** (a command built from vendor/xrpl-rust, which nothing ships) runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What breaks what

- **league-ledger** is imported by no other part and sits on the path of 4 doors.

scripts holds only shell files, which this map does not read, so what uses it cannot be seen.

## What tends to change together

No two source files changed together often enough to name.

Window: 180 days; a pair counts from 3 shared commits, since the window holds fewer than 30 qualifying commits.

## What no test touches

Every code part this map reads is touched by at least one test.

league-core is tested only by the unit tests in its own files.

league-dice is tested only by the unit tests in its own files.

league-season is tested only by the unit tests in its own files.

league-tui is tested only by the unit tests in its own files.

scripts holds only shell files, which this map does not read, so whether a test touches it cannot be seen.

## Written but never read

No place is written by the files this map could read, so none goes unread; 1 file could not be.

## Helpers that look duplicated

No two parts export a helper that looks alike in the files this map could read; 1 file could not be.

## Generated, never hand-edited

Nothing in the files this map could read writes to a tracked place; 1 file could not be.

## Hand-authored

People write .github/, docs/ and the repository root. Nothing in the files this map could read writes to them; 1 file could not be.

## Where to start

crates/league-relay/src/main.rs → crates/league-relay/src/lib.rs → crates/league-relay/src/cli.rs → crates/league-relay/src/secret.rs → crates/league-relay/src/error.rs

Read those in order to follow one run of league-relay end to end. This path follows league-relay (a command people run) from its entry, since CI runs only tests and checks.

Beside the path, crates/league-relay/src/cli.rs also calls crates/league-relay/src/relay.rs, crates/league-relay/src/seats.rs and crates/league-relay/src/transport.rs.

## What this map cannot see

- 1 file uses syntax the parser cannot read (crates/league-ledger/tests/vectors/LICENSE-xrpl.js), so what it imports is not known.
- 4 writes and 3 reads go to a path their caller passes, not to this repository.
- 370 files belong to no part: crates/league-relay/Cargo.toml, crates/league-relay/src/cli.rs, crates/league-relay/src/error.rs and 367 more.
- Statistics confidence is low: fewer than 30 qualifying commits in the window, and fewer than 25 source files reach 10 revisions.

Regenerate with `npx --yes @dogfood-lab/atlas map`.
