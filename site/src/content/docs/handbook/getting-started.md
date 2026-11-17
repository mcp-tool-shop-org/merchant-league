---
title: Getting started
description: Build the Merchant League workspace from source, run its tests and run the same checks CI runs.
sidebar:
  order: 1
---

There is no release to install yet. Everything here builds the workspace from source.

## What you need

- **Rust.** Install it with [rustup](https://rustup.rs/). The repository pins its toolchain (1.100.0, with rustfmt, clippy and the LLVM tools) in `rust-toolchain.toml`, and rustup installs it on the first build.
- **Git.**
- **Docker**, only if you want to run the stand-alone `xrpld` harness.

CI builds and tests on Linux, and the workspace is developed on Windows. Other platforms are untested.

## Build and test

```bash
git clone https://github.com/mcp-tool-shop-org/merchant-league.git
cd merchant-league
cargo test --workspace --locked
```

`--locked` builds against the committed `Cargo.lock`, the same versions CI uses.

## Run the checks CI runs

```bash
bash scripts/verify.sh
```

It runs, in order:

1. `cargo fmt --all --check`
2. `cargo clippy --all-targets --workspace --locked -- -D warnings`
3. `cargo test --workspace --locked`
4. the coverage floor of 90% of lines, when `cargo-llvm-cov` is installed
5. `cargo audit --deny warnings`, when `cargo-audit` is installed
6. the Testnet-only tripwire (`scripts/check-testnet-only.sh`)
7. the copy tripwire, which fails on text that frames the game as a source of income (`scripts/check-copy.sh`)
8. the guards' own controls (`scripts/test-guards.sh`), which plant every banned pattern and check each guard still catches it

CI also runs two checks that `verify.sh` leaves out: the stand-alone `xrpld` harness and the Atlas map check (`npx @dogfood-lab/atlas check`), which confirms the committed map in `atlas/` matches the code.

## The stand-alone `xrpld` harness

Training mode runs games against a private `xrpld` node in Docker. Its live tests are skipped unless you ask for them:

```bash
# one container: start it, check server_info, tear it down
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked

# the full conformance suite, as CI runs it
LEAGUE_XRPLD_LIVE=full cargo test -p league-standalone --test standalone_live --locked -- --test-threads=1
```

The harness pulls one pinned image (`xrplf/xrpld:3.4.1`, by its manifest digest), runs at most one container at a time with 2 CPUs and 2 GB of memory, binds its port to `127.0.0.1` only, and removes the container after every run. The [Security](../security/) page has the full list of limits.

## Try the paper-test relay offline

`league-relay` ran the paper playtests, sending each AI seat its turn. Its `prebook-check` command runs a whole turn against canned responses in a temporary folder, with no network call:

```bash
cargo run -p league-relay -- --help
```

See the [Reference](../reference/) page for its commands.
