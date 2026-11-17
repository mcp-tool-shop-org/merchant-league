# merchant-league: how it works

Mapped at 2026-11-17 from commit d4fe153 by Atlas 1.26.0.

## What this is

Merchant League: a slow-turn trading game for 2 to 6 players, played only on the XRPL Testnet. (written by a person)

11 parts, mostly Rust (341 files), Python (3), shell (3) and JavaScript (1). Work enters through 6 doors; the busiest is CI, which reaches 8 parts. People run league-relay. xrpl is a command built from vendor/xrpl-rust (nothing ships it).

## What changed since 2026-11-16 (a46e51c)

Nothing structural changed since 2026-11-16; 9 files changed content.

## What comes in

1. **CI.** On a pull request; on a push to main touching 9 paths; or by hand. Runs scripts/check-copy.sh, scripts/check-testnet-only.sh, scripts/test-guards.sh and 30 more; checks crates/league-gen/src/lib.rs, crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs.
2. **testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`). Runs crates/league-ledger/examples/testnet_escrow_expiry.rs.
3. **testnet_proof** (a command people run with `cargo run --example testnet_proof`). Runs crates/league-ledger/examples/testnet_proof.rs.
4. **testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`). Runs crates/league-ledger/examples/testnet_requireauth.rs.
5. **league-relay** (a command people run). Runs crates/league-relay/src/main.rs.
6. **xrpl** (a command built from vendor/xrpl-rust, which nothing ships). Runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What happens through CI

1. The workflow runs scripts/check-copy.sh, scripts/check-testnet-only.sh and scripts/test-guards.sh in scripts, 6 files in league-core, crates/league-dice/ in league-dice, 8 files in league-gen, crates/league-ledger/examples/, crates/league-ledger/src/ and crates/league-ledger/tests/vectors.rs in league-ledger, and 8 files in 4 more places; it checks crates/league-gen/src/lib.rs in league-gen, crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs.

## Who reads the results

CI writes nothing in the files this map could read; 1 file could not be.

## The other doors

**testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`) runs crates/league-ledger/examples/testnet_escrow_expiry.rs.

**testnet_proof** (a command people run with `cargo run --example testnet_proof`) runs crates/league-ledger/examples/testnet_proof.rs.

**testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`) runs crates/league-ledger/examples/testnet_requireauth.rs.

**league-relay** (a command people run) runs crates/league-relay/src/main.rs.

**xrpl** (a command built from vendor/xrpl-rust, which nothing ships) runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What breaks what

- **league-core** is imported by 2 parts (league-gen, league-standalone) and sits on the path of 1 door.
- **league-ledger** is imported by 1 part (league-standalone) and sits on the path of 4 doors.

scripts holds only shell files, which this map does not read, so what uses it cannot be seen.

## What tends to change together

- **crates/league-core/src/replay.rs** and **crates/league-core/tests/replay_invariants.rs** changed together in 5 of 5 commits, inside the league-core part.
- **crates/league-relay/src/relay.rs** and **crates/league-relay/tests/relay_integration.rs** changed together in 17 of 19 commits.
- **crates/league-gen/src/generator.rs** and **crates/league-gen/src/lib.rs** changed together in 7 of 8 commits, inside the league-gen part.
- **crates/league-gen/src/empirical.rs** and **crates/league-gen/tests/properties.rs** changed together in 5 of 6 commits, inside the league-gen part.
- **crates/league-gen/src/generator.rs** and **crates/league-gen/src/schema.rs** changed together in 8 of 10 commits, inside the league-gen part.

Confidence is low: fewer than 25 source files reach 10 revisions in the window.

Window: 180 days; a pair counts from 3 shared commits, since 3 source files reach 10 revisions; the floor rises to 10 when 25 do.

## What no test touches

Every code part this map reads is touched by at least one test.

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

People write .github/, docs/ and the repository root; 1 write with a path built at run time may land here.

## Where to start

crates/league-relay/src/main.rs → crates/league-relay/src/lib.rs → crates/league-relay/src/cli.rs → crates/league-relay/src/secret.rs → crates/league-relay/src/seats.rs → crates/league-relay/src/wording.rs → crates/league-relay/src/error.rs

Read those in order to follow one run of league-relay end to end. This path follows league-relay (a command people run) from its entry, since CI runs only tests and checks.

Beside the path, crates/league-relay/src/cli.rs also calls crates/league-relay/src/relay.rs.

## What this map cannot see

- 1 file uses syntax the parser cannot read (crates/league-ledger/tests/vectors/LICENSE-xrpl.js), so what it imports is not known.
- 1 write uses a path built at run time and is not named here.
- 4 writes and 3 reads go to a path their caller passes, not to this repository.
- 1 write goes to a temporary directory, not to this repository.
- 371 files belong to no part: crates/league-relay/Cargo.toml, crates/league-relay/src/cli.rs, crates/league-relay/src/error.rs and 368 more.
- Statistics confidence is low: fewer than 25 source files reach 10 revisions in the window.

Regenerate with `npx --yes @dogfood-lab/atlas map`.
