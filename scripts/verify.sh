#!/usr/bin/env bash
# One command for the checks CI runs: format, lints, tests, coverage floor,
# advisories and the two tripwires with their controls.
#
# Not here: the stand-alone xrpld harness (it needs Docker; see the README)
# and the Atlas map check (it needs Node). CI runs both.
#
# Coverage and the advisory audit run when cargo-llvm-cov and cargo-audit
# are installed, and are skipped with a note otherwise.
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n== %s\n' "$1"; }

step "Format"
cargo fmt --all --check

step "Clippy"
cargo clippy --all-targets --workspace --locked -- -D warnings

step "Test"
cargo test --workspace --locked

step "Coverage (floor 90)"
if command -v cargo-llvm-cov >/dev/null; then
  cargo llvm-cov --workspace --locked --fail-under-lines 90
else
  echo "skipped: cargo-llvm-cov is not installed (CI runs it)"
fi

step "Security audit"
if command -v cargo-audit >/dev/null; then
  cargo audit --deny warnings
else
  echo "skipped: cargo-audit is not installed (CI runs it)"
fi

step "Testnet only"
bash scripts/check-testnet-only.sh

step "No real-money language"
bash scripts/check-copy.sh

step "Guard controls"
bash scripts/test-guards.sh

printf '\nverify: all checks passed\n'
