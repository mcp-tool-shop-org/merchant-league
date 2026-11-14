# merchant-league: how it works

Mapped at 2026-11-14 from commit 6364f25 by Atlas 1.26.0.

## What this is

Merchant League: a slow-turn trading game for 2 to 6 players, played only on the XRPL Testnet. (written by a person)

9 parts, mostly Rust (10 files) and shell (2). Work enters through 2 doors; the busiest is CI, which reaches 6 parts.

## What changed since 2026-11-14 (c0d6c62)

- league-ledger now imports league-dice.
- CI now also runs crates/league-core/, crates/league-dice/, crates/league-ledger/src/ and 3 more.
- CI now also checks crates/league-ledger/examples/testnet_proof.rs.
- CI no longer runs crates/.
- And 1 more change to a door.
- crates/league-dice/ is now also read by crates/league-ledger/Cargo.toml.
- crates/league-ledger/tests/vectors/testnet-spike-2026-11-14.json is now read by crates/league-ledger/tests/testnet_vectors.rs.
- 22 files added and 8 changed content, across 4 parts.

## What comes in

1. **CI.** On a pull request; on a push to main touching 8 paths; or by hand. Runs scripts/check-copy.sh, scripts/check-testnet-only.sh, crates/league-core/ and 8 more; checks crates/league-ledger/examples/testnet_proof.rs.
2. **testnet_proof** (a command people run with `cargo run --example testnet_proof`). Runs crates/league-ledger/examples/testnet_proof.rs.

## What happens through CI

1. The workflow runs crates/league-core/ in league-core, crates/league-dice/ in league-dice, crates/league-ledger/src/ and crates/league-ledger/tests/ in league-ledger, crates/league-season/ in league-season, crates/league-tui/ in league-tui, and scripts/check-copy.sh and scripts/check-testnet-only.sh in scripts; it checks crates/league-ledger/examples/testnet_proof.rs in league-ledger.

## Who reads the results

CI writes nothing this map can see.

## The other doors

**testnet_proof** (a command people run with `cargo run --example testnet_proof`) runs crates/league-ledger/examples/testnet_proof.rs and reaches league-dice.

## What breaks what

- **league-dice** is imported by 1 part (league-ledger) and sits on the path of 2 doors.
- **league-ledger** is imported by no other part and sits on the path of 2 doors.

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

No place this map can see is written, so none goes unread.

## Helpers that look duplicated

No two parts export a helper that looks alike.

## Generated, never hand-edited

Nothing in this repository writes to a tracked place this map can see.

## Hand-authored

People write .github/, docs/ and the repository root. Nothing in this repository writes to them.

## Where to start

.github/workflows/ci.yml → crates/league-ledger/tests/testnet_vectors.rs → crates/league-ledger/src/codec.rs → crates/league-ledger/src/keys.rs

Read those in order to follow one pull request end to end. This path starts at crates/league-ledger/tests/testnet_vectors.rs, the test CI runs that reaches the most parts, since CI runs only tests.

## What this map cannot see

- Statistics confidence is low: fewer than 30 qualifying commits in the window, and fewer than 25 source files reach 10 revisions.

Regenerate with `npx --yes @dogfood-lab/atlas map`.
