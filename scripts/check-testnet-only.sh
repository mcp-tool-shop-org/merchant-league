#!/usr/bin/env bash
# A tripwire: fails when the repository's text names an XRPL Mainnet or
# Devnet endpoint. Merchant League plays on the Testnet only.
#
# This is not the guarantee. The guarantee is in the code and its tests:
# league-ledger builds a Testnet client only after the server reports
# network id 1 (`require_testnet`), and `cargo test` proves that every other
# id, and a missing one, is refused. This script only catches a Mainnet or
# Devnet host written out in plain text. It cannot see a host assembled at
# compile time (`concat!`, `\x` escapes) or a provider it doesn't list.
# Review catches those.
#
# There is no allow marker. A line that trips this is reworded or removed.
# scripts/test-guards.sh plants every pattern below, and near misses that
# must pass.
set -euo pipefail
cd "$(dirname "$0")/.."

# The whole repository, except what Git and Cargo generate and the vendored
# crate. The vendored crate is third-party source; its Mainnet defaults sit
# in a module the build leaves out (vendor/xrpl-rust/VENDORED.md). This
# script and its tests name the patterns, so they are skipped too.
skip_dirs=(--exclude-dir=.git --exclude-dir=target --exclude-dir=vendor --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=.astro)
self='^\./scripts/(check-testnet-only|test-guards)\.sh:'

hosts=(
  "(^|[^a-z0-9-])([a-z0-9-]+\.)*ripple\.com\b"            # any ripple.com host; the Testnet is rippletest.net
  "\bxrplcluster\.com\b"                                    # Mainnet clusters and providers
  "\bxrpl\.ws\b"
  "\bxrpl\.link\b"
  "\bxrpl\.to\b"
  "\bmainnet\.xrpl-labs\.com\b"
  "\bxrpscan\.com\b"                                        # Mainnet explorers
  "(^|[^a-z0-9.-])((www|dev)\.)?bithomp\.com\b"             # bare, www. and dev. (test.bithomp.com is the Testnet's)
  "\blivenet\.xrpl\.org\b"
  "\bxrpl\.org/mainnet\b"
  "(https?|wss?)://[^[:space:]\"'<>)]*(mainnet|livenet)"    # any URL that says mainnet or livenet
  "\bdevnet\.rippletest\.net\b"                             # Devnet
  "\bdevnet\.xrpl\.org\b"                                    # the Devnet explorer
)
pattern=$(IFS='|'; echo "${hosts[*]}")

hits=$(grep -rnIiE "${skip_dirs[@]}" "$pattern" . 2>/dev/null | grep -vE "$self" || true)
if [ -n "$hits" ]; then
  echo "[check-testnet-only] FAIL: a Mainnet or Devnet endpoint is named. Merchant League is Testnet only."
  echo "$hits"
  exit 1
fi

echo "[check-testnet-only] OK: no Mainnet or Devnet endpoint is named. (The guarantee is require_testnet and its tests.)"
