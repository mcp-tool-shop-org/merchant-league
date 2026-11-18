# merchant-league: how it works

Mapped at 2026-11-18 from commit 9818e99 by Atlas 1.26.0.

## What this is

Merchant League: a slow-turn trading game for 2 to 6 players, played only on the XRPL Testnet. (written by a person)

10 parts, mostly Rust (354 files), JavaScript (4), shell (4), Python (3), CSS (2), TypeScript (2) and Astro (1). Work enters through 7 doors; the busiest is CI, which reaches 7 parts. It deploys a site to GitHub Pages. People run league-relay. xrpl is a command built from vendor/xrpl-rust (nothing ships it).

## What changed since 2026-11-18 (7fd3b48)

- CI now also runs crates/league-core/src/error.rs, crates/league-error/, crates/league-ledger/src/ and 1 more.
- CI now also checks crates/league-error/src/lib.rs.
- CI no longer runs crates/league-ledger/src/client.rs and crates/league-ledger/src/escrow.rs.
- crates/league-error/ is now read by Cargo.toml, crates/league-core/Cargo.toml, crates/league-ledger/Cargo.toml, crates/league-relay/Cargo.toml and crates/league-standalone/Cargo.toml.
- crates/league-error/Cargo.toml is new and belongs to no part, so atlas check fails on it against the previous map.
- crates/league-error/src/lib.rs is new and belongs to no part, so atlas check fails on it against the previous map.
- 2 files added, 1 removed and 19 changed content, across 5 parts.

## What comes in

1. **CI.** On a pull request; on a push to main touching 13 paths; or by hand. Runs scripts/check-audit-allowlist.mjs, scripts/check-audit-allowlist.test.mjs, scripts/check-copy.sh and 45 more; checks crates/league-gen/src/lib.rs, crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs.
2. **Deploy site to GitHub Pages.** On a push to main touching 2 paths; or by hand. Runs site/astro.config.mjs and site/src/.
3. **testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`). Runs crates/league-ledger/examples/testnet_escrow_expiry.rs.
4. **testnet_proof** (a command people run with `cargo run --example testnet_proof`). Runs crates/league-ledger/examples/testnet_proof.rs.
5. **testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`). Runs crates/league-ledger/examples/testnet_requireauth.rs.
6. **league-relay** (a command people run). Runs crates/league-relay/src/main.rs.
7. **xrpl** (a command built from vendor/xrpl-rust, which nothing ships). Runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What happens through CI

1. The workflow runs 5 files in scripts, 12 files in league-core, 9 files in league-gen, crates/league-ledger/examples/, crates/league-ledger/src/ and crates/league-ledger/tests/vectors.rs in league-ledger, crates/league-season/ in league-season, and 14 files in 8 more places; it checks crates/league-gen/src/lib.rs in league-gen, crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs.
2. It writes to site/audit-report.json, which is not tracked.

## Who reads the results

CI writes only to site/audit-report.json, which is not tracked.

## The other doors

**Deploy site to GitHub Pages** runs site/astro.config.mjs and site/src/, and deploys the site on main.

**testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`) runs crates/league-ledger/examples/testnet_escrow_expiry.rs.

**testnet_proof** (a command people run with `cargo run --example testnet_proof`) runs crates/league-ledger/examples/testnet_proof.rs.

**testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`) runs crates/league-ledger/examples/testnet_requireauth.rs.

**league-relay** (a command people run) runs crates/league-relay/src/main.rs.

**xrpl** (a command built from vendor/xrpl-rust, which nothing ships) runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What breaks what

- **league-core** is imported by 2 parts (league-gen, league-standalone) and sits on the path of 1 door.
- **league-ledger** is imported by 1 part (league-standalone) and sits on the path of 4 doors.

## What tends to change together

- **crates/league-relay/src/secret.rs** and **crates/league-relay/src/transport.rs** changed together in 5 of 5 commits.
- **crates/league-gen/src/empirical.rs** and **crates/league-gen/tests/properties.rs** changed together in 5 of 6 commits, inside the league-gen part.
- **crates/league-relay/src/relay.rs** and **crates/league-relay/tests/relay_integration.rs** changed together in 18 of 22 commits.
- **crates/league-core/src/lib.rs** and **crates/league-core/src/replay.rs** changed together in 9 of 11 commits, inside the league-core part.
- **crates/league-core/src/replay.rs** and **crates/league-core/tests/replay_invariants.rs** changed together in 9 of 11 commits, inside the league-core part.

Confidence is low: fewer than 25 source files reach 10 revisions in the window.

Window: 180 days; a pair counts from 3 shared commits, since 6 source files reach 10 revisions; the floor rises to 10 when 25 do.

## What no test touches

Every code part is touched by at least one test.

league-season is tested only by the unit tests in its own files.

league-tui is tested only by the unit tests in its own files.

## Written but never read

No place is written by the files this map could read, so none goes unread; 1 file could not be.

## Helpers that look duplicated

No two parts export a helper that looks alike in the files this map could read; 1 file could not be.

## Generated, never hand-edited

Nothing in the files this map could read writes to a tracked place; 1 file could not be.

## Hand-authored

People write .github/, docs/ and the repository root; 3 writes with paths built at run time may land here.

## Where to start

crates/league-relay/src/main.rs → crates/league-relay/src/lib.rs → crates/league-relay/src/cli.rs → crates/league-relay/src/secret.rs → crates/league-relay/src/error.rs → crates/league-error/src/lib.rs

Read those in order to follow one run of league-relay end to end. This path follows league-relay (a command people run) from its entry, since CI runs only tests, scripts that import no code here and checks.

Beside the path, crates/league-relay/src/cli.rs also calls crates/league-relay/src/relay.rs and crates/league-relay/src/seats.rs.

## What this map cannot see

- 1 file uses syntax the parser cannot read (crates/league-ledger/tests/vectors/LICENSE-xrpl.js), so what it imports is not known.
- 3 writes and 4 reads use paths built at run time and are not named here.
- 1 write goes to places this repository does not track, so it is not listed as generated.
- 5 writes and 5 reads go to a path their caller passes, not to this repository.
- 1 write goes to a temporary directory, not to this repository.
- 391 files belong to no part: crates/league-error/Cargo.toml, crates/league-error/src/lib.rs, crates/league-relay/Cargo.toml and 388 more.
- Statistics confidence is low: fewer than 25 source files reach 10 revisions in the window.

Regenerate with `npx --yes @dogfood-lab/atlas map`.
