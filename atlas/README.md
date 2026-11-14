# merchant-league: how it works

Mapped at 2026-11-14 from commit c0d6c62 by Atlas 1.26.0.

## What this is

Merchant League: a slow-turn trading game for 2 to 6 players, played only on the XRPL Testnet. (written by a person)

9 parts, mostly Rust (5 files) and shell (2). Work enters through 1 door; the busiest is CI, which reaches 6 parts.

## What changed since 2026-11-14 (4c4f139)

- crates/league-tui/ is now read by Cargo.toml.
- league-tui is a new part, drawn from `crates/league-tui/**`.
- 2 files added and 7 changed content, across 4 parts.

## What comes in

1. **CI.** On a pull request; on a push to main touching 8 paths; or by hand. Runs scripts/check-copy.sh, scripts/check-testnet-only.sh and crates/.

## What happens through CI

1. The workflow runs scripts/check-copy.sh and scripts/check-testnet-only.sh in scripts and crates/ (5 parts).

## Who reads the results

CI writes nothing this map can see.

## What breaks what

No part is imported by another part, and no part sits on the path of two doors.

scripts holds only shell files, which this map does not read, so what uses it cannot be seen.

## What tends to change together

No two source files changed together often enough to name.

Window: 180 days; a pair counts from 3 shared commits, since the window holds fewer than 30 qualifying commits.

## What no test touches

Every code part this map reads is touched by at least one test.

league-core is tested only by the unit tests in its own files.

league-dice is tested only by the unit tests in its own files.

league-ledger is tested only by the unit tests in its own files.

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

.github/workflows/ci.yml → crates/league-core/src/lib.rs

Read those in order to follow one pull request end to end.

## What this map cannot see

- Statistics confidence is low: fewer than 30 qualifying commits in the window, and fewer than 25 source files reach 10 revisions.

Regenerate with `npx --yes @dogfood-lab/atlas map`.
