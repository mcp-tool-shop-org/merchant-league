#!/usr/bin/env bash
# Fails when any known XRPL Mainnet endpoint appears in the source.
# Merchant League plays on the Testnet only; the only endpoints the code may
# name are the Testnet ones in crates/league-ledger.
set -euo pipefail
cd "$(dirname "$0")/.."

# Public Mainnet servers and clusters. Add any new one here.
pattern='s1\.ripple\.com|s2\.ripple\.com|xrplcluster\.com|xrpl\.ws|xrpl\.link|xrpl\.org/mainnet|wss://s\.ripple|livenet'

hits=$(grep -rnIiE "$pattern" crates/ scripts/ Cargo.toml 2>/dev/null | grep -v '^scripts/check-testnet-only\.sh:' || true)
if [ -n "$hits" ]; then
  echo "[check-testnet-only] FAIL: Mainnet endpoints found. Merchant League is Testnet only."
  echo "$hits"
  exit 1
fi

# The network id the guard accepts must be the Testnet's, 1.
if ! grep -q 'pub const TESTNET_NETWORK_ID: u32 = 1;' crates/league-ledger/src/lib.rs; then
  echo "[check-testnet-only] FAIL: TESTNET_NETWORK_ID is not 1 in crates/league-ledger/src/lib.rs."
  exit 1
fi

echo "[check-testnet-only] OK: no Mainnet endpoints; the guard accepts network 1 only."
