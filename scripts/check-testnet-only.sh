#!/usr/bin/env bash
# Fails when anything points the game at a network other than the XRPL
# Testnet. Merchant League plays on the Testnet only.
#
# 1. No Mainnet or Devnet endpoint appears anywhere in the repository's text:
#    source, scripts, docs, README and CI.
# 2. No code sets a network id of 0 (Mainnet), in any of the usual spellings.
# 3. league-ledger defines the Testnet's id as 1, on its own line, and the
#    guard accepts exactly that constant.
#
# A line that must name one of these to test the refusal carries the marker
# "testnet-check: allow". scripts/test-guards.sh plants every pattern below
# and proves this script fails on each.
set -euo pipefail
cd "$(dirname "$0")/.."

scope=(crates scripts docs README.md Cargo.toml .github)
self='scripts/(check-testnet-only|test-guards)\.sh:'

# Mainnet servers and clusters on any scheme, or bare; Devnet hosts; Mainnet
# explorers. Add any new one here, and a planted case in test-guards.sh.
hosts='(^|[^a-z0-9.-])s[12]?\.ripple\.com|xrplcluster\.com|xrpl\.ws\b|xrpl\.link\b|mainnet\.xrpl-labs\.com|xrpl\.to\b|livenet|devnet\.rippletest\.net|xrpl\.org/mainnet'

# A network id of 0 written as code: "network_id": 0, network_id = 0,
# NETWORK_ID: u32 = 0, NetworkId(0) and the like.
netzero='network_?id["'"'"'`]?[[:space:]]*[:=][[:space:]]*(u32[[:space:]]*=[[:space:]]*)?0\b|networkid\(0\)'

fail=0

hits=$(grep -rnIiE "$hosts" "${scope[@]}" 2>/dev/null | grep -vE "^$self" | grep -v 'testnet-check: allow' || true)
if [ -n "$hits" ]; then
  echo "[check-testnet-only] FAIL: a Mainnet or Devnet endpoint is named. Merchant League is Testnet only."
  echo "$hits"
  fail=1
fi

hits=$(grep -rnIiE "$netzero" crates scripts 2>/dev/null | grep -vE "^$self" | grep -v 'testnet-check: allow' || true)
if [ -n "$hits" ]; then
  echo "[check-testnet-only] FAIL: code sets a network id of 0 (Mainnet)."
  echo "$hits"
  fail=1
fi

lib=crates/league-ledger/src/lib.rs
if ! grep -qE '^pub const TESTNET_NETWORK_ID: u32 = 1;' "$lib"; then
  echo "[check-testnet-only] FAIL: $lib does not define TESTNET_NETWORK_ID as 1 on its own line."
  fail=1
fi
if [ "$(grep -cE 'NETWORK_ID: u32 =' "$lib")" != 1 ]; then
  echo "[check-testnet-only] FAIL: $lib must define exactly one network id."
  fail=1
fi
if ! grep -qE 'Some\(TESTNET_NETWORK_ID\) => Ok\(\(\)\)' "$lib"; then
  echo "[check-testnet-only] FAIL: require_testnet in $lib no longer accepts only TESTNET_NETWORK_ID."
  fail=1
fi

if [ "$fail" = 1 ]; then
  exit 1
fi
echo "[check-testnet-only] OK: no Mainnet or Devnet endpoints, no network id 0, and the guard accepts network 1 only."
