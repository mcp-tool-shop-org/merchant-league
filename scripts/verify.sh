#!/usr/bin/env bash
# One command for the checks CI runs: format, lints, tests, coverage floor,
# advisories, the two tripwires with their controls, and the site build.
#
# Not here: the stand-alone xrpld harness (it needs Docker; see the README)
# and the Atlas map check (CI pins its version). CI runs both.
#
# Coverage, the advisory audit and the site build run when cargo-llvm-cov,
# cargo-audit and npm are installed, and are skipped with a note otherwise.
set -euo pipefail
cd "$(dirname "$0")/.."

skipped=()
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
  skipped+=("coverage")
fi

step "Security audit"
if command -v cargo-audit >/dev/null; then
  cargo audit --deny warnings
else
  echo "skipped: cargo-audit is not installed (CI runs it)"
  skipped+=("cargo audit")
fi

step "Testnet only"
bash scripts/check-testnet-only.sh

step "No real-money language"
bash scripts/check-copy.sh

step "Guard controls"
bash scripts/test-guards.sh

step "Site (npm ci, npm audit, build)"
if command -v npm >/dev/null; then
  (cd site && npm ci --no-fund --no-audit && npm audit --audit-level=high && npm run build)
else
  echo "skipped: npm is not installed (CI runs it)"
  skipped+=("site build")
fi

if [ ${#skipped[@]} -eq 0 ]; then
  printf '\nverify: all checks passed\n'
else
  printf '\nverify: every check that ran passed; NOT run here: %s (install the tool, or rely on CI)\n' "${skipped[*]}"
fi
