#!/usr/bin/env bash
# Tests for check-copy.sh's licence exclusion. Each case builds a scratch
# tree with a copy of the check, runs it, and compares the exit code.
#
# The exclusion skips files named LICENSE* (verbatim legal text, which the
# ISC licence requires unchanged). It must not reach anything else: a money
# word in any other file under crates/ still fails.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
pass=0
fail=0

run_case() {
  local name="$1" want="$2" path="$3" text="$4"
  local tmp
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/scripts" "$tmp/crates/demo/src"
  cp "$here/check-copy.sh" "$tmp/scripts/"
  printf 'A merchant league.\n' > "$tmp/README.md"
  if [ -n "$path" ]; then
    mkdir -p "$(dirname "$tmp/$path")"
    printf '%s\n' "$text" > "$tmp/$path"
  fi
  local got=0
  bash "$tmp/scripts/check-copy.sh" > /dev/null 2>&1 || got=$?
  [ "$got" -ne 0 ] && got=1
  rm -rf "$tmp"
  if [ "$got" -eq "$want" ]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "[test-copy-check] FAIL: $name (wanted exit $want, got $got)"
  fi
}

warranty='WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN'

# The exclusion: licence files pass, wherever they sit under crates/.
run_case "a clean tree passes" 0 "" ""
run_case "the vendored xrpl.js licence passes" 0 "crates/demo/tests/vectors/LICENSE-xrpl.js" "$warranty"
run_case "a plain LICENSE passes" 0 "crates/demo/LICENSE" "$warranty"
run_case "a LICENSE.md passes" 0 "crates/demo/LICENSE.md" "$warranty"
run_case "a vendored crate's LICENSE passes" 0 "crates/vendor/xrpl-rust/LICENSE" "$warranty"

# It does not widen: the same words anywhere else still fail.
run_case "the warranty text in a source file fails" 1 "crates/demo/src/lib.rs" "// $warranty"
run_case "a money word in a test vector fails" 1 "crates/demo/tests/vectors/notes.md" "Keep the profit."
run_case "a file only named like a licence inside fails" 1 "crates/demo/src/licence_profit.rs" "// profit"
run_case "a lowercase license file is not excluded" 1 "crates/demo/license.txt" "$warranty"
run_case "a LICENSE directory does not exclude its files" 1 "crates/demo/LICENSES/notes.md" "Cash out now."
run_case "README.md still fails" 1 "README.md" "Cash out your marks."

echo "[test-copy-check] $pass passed, $fail failed."
[ "$fail" -eq 0 ]
