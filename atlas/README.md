# merchant-league: how it works

Mapped at 2026-11-14 from commit ddc80b8 by Atlas 1.26.0.

## What this is

Merchant League: a slow-turn trading game for 2 to 6 players, played only on the XRPL Testnet. (written by a person)

9 parts, mostly Rust (306 files), Python (3), shell (3) and JavaScript (1). Work enters through 5 doors; the busiest is CI, which reaches 6 parts. xrpl is a command built from vendor/xrpl-rust (nothing ships it).

## What changed since 2026-11-14 (ef971a4)

Nothing structural changed since 2026-11-14; 16 files added.

## What comes in

1. **CI.** On a pull request; on a push to main touching 9 paths; or by hand. Runs scripts/check-copy.sh, scripts/check-testnet-only.sh, scripts/test-guards.sh and 10 more.
2. **testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`). Runs crates/league-ledger/examples/testnet_escrow_expiry.rs.
3. **testnet_proof** (a command people run with `cargo run --example testnet_proof`). Runs crates/league-ledger/examples/testnet_proof.rs.
4. **testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`). Runs crates/league-ledger/examples/testnet_requireauth.rs.
5. **xrpl** (a command built from vendor/xrpl-rust, which nothing ships). Runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What happens through CI

1. The workflow runs crates/league-core/ in league-core, crates/league-dice/ in league-dice, crates/league-ledger/examples/, crates/league-ledger/src/ and crates/league-ledger/tests/vectors.rs in league-ledger, crates/league-season/ in league-season, crates/league-tui/ in league-tui, and scripts/check-copy.sh, scripts/check-testnet-only.sh and scripts/test-guards.sh in scripts.

## Who reads the results

CI writes nothing in the files this map could read; 1 file could not be.

## The other doors

**testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`) runs crates/league-ledger/examples/testnet_escrow_expiry.rs.

**testnet_proof** (a command people run with `cargo run --example testnet_proof`) runs crates/league-ledger/examples/testnet_proof.rs.

**testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`) runs crates/league-ledger/examples/testnet_requireauth.rs.

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

vendor/xrpl-rust/src/bin/xrpl.rs → vendor/xrpl-rust/src/lib.rs → vendor/xrpl-rust/src/account/mod.rs → vendor/xrpl-rust/src/asynch/account/mod.rs → vendor/xrpl-rust/src/core/addresscodec/mod.rs → vendor/xrpl-rust/src/core/addresscodec/utils.rs → vendor/xrpl-rust/src/core/addresscodec/exceptions.rs

Read those in order to follow one run of xrpl end to end. This path follows xrpl (a command built from vendor/xrpl-rust, which nothing ships) from its entry, since CI runs only tests.

## What this map cannot see

- 1 file uses syntax the parser cannot read (crates/league-ledger/tests/vectors/LICENSE-xrpl.js), so what it imports is not known.
- 4 writes and 3 reads go to a path their caller passes, not to this repository.
- 360 files belong to no part: vendor/xrpl-rust/Cargo.toml, vendor/xrpl-rust/Cargo.toml.orig, vendor/xrpl-rust/LICENSE and 357 more.
- Statistics confidence is low: fewer than 30 qualifying commits in the window, and fewer than 25 source files reach 10 revisions.

Regenerate with `npx --yes @dogfood-lab/atlas map`.
