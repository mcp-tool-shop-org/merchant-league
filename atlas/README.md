# merchant-league: how it works

Mapped at 2026-11-19 from commit 40c7933 by Atlas 1.26.0.

## What this is

Merchant League: a slow-turn trading game for 2 to 6 players, played only on the XRPL Testnet. (written by a person)

14 parts, mostly Rust (368 files), JavaScript (4), shell (4), Python (3), CSS (2), TypeScript (2) and Astro (1). Work enters through 8 doors; the busiest is CI, which reaches 11 parts. It deploys a site to GitHub Pages. People run league-relay and league-tui. xrpl is a command built from vendor/xrpl-rust (nothing ships it).

## What changed since 2026-11-18 (f088743)

- CI now also runs crates/league-gen/src/glossary.rs.
- 14 files changed content, across 8 parts.

## What comes in

1. **CI.** On a pull request; on a push to main touching 13 paths; or by hand. Runs scripts/check-audit-allowlist.mjs, scripts/check-audit-allowlist.test.mjs, scripts/check-copy.sh and 53 more; checks crates/league-gen/src/lib.rs, crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs.
2. **Deploy site to GitHub Pages.** On a push to main touching 2 paths; or by hand. Runs site/astro.config.mjs and site/src/.
3. **league-tui** (a command people run). Runs crates/league-tui/src/main.rs.
4. **league-relay** (a command people run). Runs crates/league-relay/src/main.rs.
5. **testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`). Runs crates/league-ledger/examples/testnet_escrow_expiry.rs.
6. **testnet_proof** (a command people run with `cargo run --example testnet_proof`). Runs crates/league-ledger/examples/testnet_proof.rs.
7. **testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`). Runs crates/league-ledger/examples/testnet_requireauth.rs.
8. **xrpl** (a command built from vendor/xrpl-rust, which nothing ships). Runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What happens through CI

1. The workflow runs 5 files in scripts, 15 files in league-core, crates/league-error/ in league-error, 10 files in league-gen, crates/league-ledger/examples/, crates/league-ledger/src/ and crates/league-ledger/tests/vectors.rs in league-ledger, and 17 files in 8 more places; it checks crates/league-gen/src/lib.rs in league-gen and crates/league-relay/src/lib.rs and crates/league-relay/src/main.rs in league-relay.
2. It writes to site/audit-report.json, which is not tracked.

## Who reads the results

CI writes only to site/audit-report.json, which is not tracked.

## The other doors

**Deploy site to GitHub Pages** runs site/astro.config.mjs and site/src/, and deploys the site on main.

**league-tui** (a command people run) runs crates/league-tui/src/main.rs and reaches league-core, league-error, league-gen, league-seat-types and league-seat-ui.

**league-relay** (a command people run) runs crates/league-relay/src/main.rs and reaches league-error.

**testnet_escrow_expiry** (a command people run with `cargo run --example testnet_escrow_expiry`) runs crates/league-ledger/examples/testnet_escrow_expiry.rs and reaches league-error.

**testnet_proof** (a command people run with `cargo run --example testnet_proof`) runs crates/league-ledger/examples/testnet_proof.rs and reaches league-error.

**testnet_requireauth** (a command people run with `cargo run --example testnet_requireauth`) runs crates/league-ledger/examples/testnet_requireauth.rs and reaches league-error.

**xrpl** (a command built from vendor/xrpl-rust, which nothing ships) runs vendor/xrpl-rust/src/bin/xrpl.rs.

## What breaks what

- **league-error** is imported by 5 parts (league-core, league-ledger, league-relay, league-seat-types, league-standalone) and sits on the path of 6 doors.
- **league-seat-types** is imported by 4 parts (league-core, league-gen, league-seat-ui, league-tui) and sits on the path of 2 doors.
- **league-core** is imported by 3 parts (league-gen, league-standalone, league-tui) and sits on the path of 2 doors.
- **league-ledger** is imported by 1 part (league-standalone) and sits on the path of 4 doors.
- **league-gen** is imported by 1 part (league-tui) and sits on the path of 2 doors.
- **league-seat-ui** is imported by 1 part (league-tui) and sits on the path of 2 doors.
- **league-relay** is imported by no other part and sits on the path of 2 doors.
- **league-tui** is imported by no other part and sits on the path of 2 doors.

## What tends to change together

- **crates/league-gen/src/empirical.rs** and **crates/league-gen/tests/properties.rs** changed together in 5 of 6 commits, inside the league-gen part.
- **crates/league-relay/src/secret.rs** and **crates/league-relay/src/transport.rs** changed together in 5 of 6 commits, inside the league-relay part.
- **crates/league-relay/src/relay.rs** and **crates/league-relay/tests/relay_integration.rs** changed together in 18 of 22 commits, inside the league-relay part.
- **crates/league-core/src/lib.rs** and **crates/league-core/src/replay.rs** changed together in 9 of 11 commits, inside the league-core part.
- **crates/league-core/src/replay.rs** and **crates/league-core/tests/replay_invariants.rs** changed together in 9 of 11 commits, inside the league-core part.

Confidence is low: fewer than 25 source files reach 10 revisions in the window.

Window: 180 days; a pair counts from 3 shared commits, since 7 source files reach 10 revisions; the floor rises to 10 when 25 do.

## What no test touches

Every code part is touched by at least one test.

league-season is tested only by the unit tests in its own files.

league-seat-ui is tested only by the unit tests in its own files.

league-tui is tested only by the unit tests in its own files.

## Written but never read

No place is written by the files this map could read, so none goes unread; 1 file could not be.

## Helpers that look duplicated

No two parts export a helper that looks alike in the files this map could read; 1 file could not be.

## Generated, never hand-edited

Nothing in the files this map could read writes to a tracked place; 1 file could not be.

## Hand-authored

People write .github/, docs/ and the repository root; 23 writes with paths built at run time may land here.

## Where to start

crates/league-tui/src/main.rs → crates/league-seat-ui/src/lib.rs → crates/league-seat-types/src/lib.rs → crates/league-error/src/lib.rs

Read those in order to follow one run of league-tui end to end. This path follows league-tui (a command people run) from its entry, since CI runs only tests, scripts that import no code here and checks.

## What this map cannot see

- 1 file uses syntax the parser cannot read (crates/league-ledger/tests/vectors/LICENSE-xrpl.js), so what it imports is not known.
- 23 writes and 13 reads use paths built at run time and are not named here.
- 1 write goes to places this repository does not track, so it is not listed as generated.
- 13 writes and 17 reads go to a path their caller passes, not to this repository.
- 2 writes go to a temporary directory, not to this repository.
- 377 files belong to no part: site/astro.config.mjs, site/package-lock.json, site/package.json and 374 more.
- Statistics confidence is low: fewer than 25 source files reach 10 revisions in the window.

Regenerate with `npx --yes @dogfood-lab/atlas map`.
