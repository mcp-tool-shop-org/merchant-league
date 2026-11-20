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
- **Docker**, if you want to build and run the local Club Mode container image or run the stand-alone `xrpld` harness.

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
9. the site: `npm ci`, `npm audit` and the build of the landing page and this handbook, when `npm` is installed
10. the held Club Mode container build and offline smoke gate (`scripts/smoke-container.sh`), on Linux when `docker` and POSIX `python3` (`pty`/`fcntl`/`termios`) are available

CI also runs two checks that `verify.sh` leaves out: the stand-alone `xrpld` harness and the Atlas map check (`npx @dogfood-lab/atlas check`), which confirms the committed map in `atlas/` matches the code.

## Run Club Mode in Docker (local build)

There is **no published container image** yet. You can build the held multi-stage image from the repository's `Dockerfile` and run Club Mode offline (`--network=none`, tested in CI without any registry push):

```bash
docker build -t merchant-league:local .
mkdir -p saves
docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$(pwd)/saves:/saves" merchant-league:local
```

- **Saving & mounts:** On Linux Docker Engine, the default `65532:65532` user cannot write to a host bind mount owned by your user (or `root`-created when missing), so pre-creating `./saves` and passing `--user "$(id -u):$(id -g)"` keeps files owned by your non-root account without `chmod 777` or root. Alternatively, a Docker named volume (`docker run --rm -it --network=none -v merchant-league-saves:/saves merchant-league:local`, without `--user`) initializes `/saves` from the image's `65532:65532` ownership. On Docker Desktop (macOS and Windows), bind-mount permissions are virtualized across the Linux VM, so in PowerShell (`mkdir saves; docker run --rm -it --network=none -v "${PWD}/saves:/saves" merchant-league:local`) or macOS the bind mount saves under `65532:65532` without `--user`, while WSL 2 Linux shells can run the Linux command directly.
- Pass `-it` so `league-tui` has an interactive TTY (without a TTY, it exits `1` with `TTY_REQUIRED` and prints the `docker run --rm -it <image>` hint; `--help` and `--version` exit `0`).
- Built from digest-pinned `rust:1.100.0-bookworm` onto digest-pinned `gcr.io/distroless/cc-debian12:nonroot`, defaulting to `65532:65532` in `/saves`. On top of that pinned distroless base (which carries no shell and no package manager), the build adds only `/usr/local/bin/league-tui`, `/usr/share/doc/merchant-league/LICENSE`, and an empty `/saves` directory owned by `65532:65532` — no toolchain, no source files, and zero network calls.

## The stand-alone `xrpld` harness

Training mode runs games against a private `xrpld` node in Docker. Its live tests are skipped unless you ask for them:

```bash
# one container: start it, check server_info, tear it down
LEAGUE_XRPLD_LIVE=smoke cargo test -p league-standalone --test standalone_live --locked

# the full conformance suite, as CI runs it
LEAGUE_XRPLD_LIVE=full cargo test -p league-standalone --test standalone_live --locked -- --test-threads=1
```

The harness pulls one pinned image (`xrplf/xrpld:3.4.1`, by its manifest digest), runs at most one container at a time with 2 CPUs and 2 GB of memory, binds its port to `127.0.0.1` only, and removes the container after each run; a container left behind by a killed run is swept away when the next one starts. The [Security](../security/) page has the full list of limits.

## Try the paper-test relay offline

`league-relay` ran the paper playtests, sending each AI seat its turn. Its `prebook-check` command runs a whole turn against canned responses in a temporary folder, with no network call. Its commands and options:

```bash
cargo run -p league-relay -- --help
```

See the [Reference](../reference/) page for its commands.
