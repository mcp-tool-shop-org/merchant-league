#!/usr/bin/env bash
# Fails when real-money language appears in player-facing text.
# Game words only: marks, table coin, goods, purse, caravan, merchant.
# A line that must name a banned word in order to forbid it carries the
# marker "copy-check: allow".
# Files named LICENSE* are skipped: they are third-party licence text, which
# must stay verbatim (ISC asks that its notice appear unchanged in every
# copy), and they are not player-facing. Only the file name is matched, so
# a money word in any other file still fails (scripts/test-copy-check.sh).
set -euo pipefail
cd "$(dirname "$0")/.."

pattern='buy-in|\bbuy in\b|\bcash[ -]?out|\bpayouts?\b|\bpay out\b|\bprofit|\binvest(s|ed|ing|ment|ments|or|ors)?\b|wallet value|withdraw funds|real-money value|\bearn real\b|\bmake money\b|\bwin money\b'

hits=$(grep -rnIiE --exclude='LICENSE*' "$pattern" crates/ README.md 2>/dev/null | grep -v 'copy-check: allow' || true)
if [ -n "$hits" ]; then
  echo "[check-copy] FAIL: real-money language in player-facing text. Use game words instead."
  echo "$hits"
  exit 1
fi

echo "[check-copy] OK: no real-money language in crates/ or README.md."
