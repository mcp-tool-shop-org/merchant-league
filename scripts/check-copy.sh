#!/usr/bin/env bash
# Fails when real-money language appears in player-facing text: the crates,
# the README and the docs.
#
# Game words are fine, including ordinary trading language: "buy in Varn",
# "sell at a profit", "a profitable route" are about goods and marks. What is
# banned is language that frames the game as earning, paying out or
# withdrawing money. The disclaimer "no real money" is allowed.
#
# A line that must name a banned phrase in order to forbid it carries the
# marker "copy-check: allow". scripts/test-guards.sh plants every pattern
# below and proves this script fails on each.
#
# "pay out" is caught even in "pay out of your purse": game copy says "pay
# from". Known limit: grep reads line by line, so a phrase split across two lines is
# not caught.
set -euo pipefail
cd "$(dirname "$0")/.."

scope=(crates README.md docs)
sep='[ _-]'

banned=(
  "\bbuy[_-]ins?\b"                                                   # buy-in, buy_in, buy-ins
  "\bcash(es|ed|ing)?${sep}?outs?\b"                                  # cash out, cashed out, cashout, cash_out
  "\b(pay|pays|paying|paid)${sep}?outs?\b"                            # payout, pays out, paid out, pay_out
  "\binvest(s|ed|ing|ment|ments|or|ors)?\b"                           # invest, investing, investor
  "\bwallet${sep}+value\b"                                            # wallet value
  "\bwithdr(aw|aws|awing|awn|ew)${sep}+(funds|money|xrp)\b"           # withdraw funds, withdrew money
  "\breal${sep}?money\b"                                              # real money, real-money (disclaimers excepted)
  "\breal${sep}+profits?\b"                                           # real profit
  "\b(earn|earns|earned|earning|make|makes|made|making|win|wins|won|winning)${sep}+(real${sep}+)?money\b"
  "\b(earn|earns|earned|earning)${sep}+real\b"                        # earn real ...
)
pattern=$(IFS='|'; echo "${banned[*]}")

# The one disclaimer that may name real money: saying there is none.
disclaimer='\b(no|not|never)[ -]real[ -]money\b'

hits=$(grep -rnIiE "$pattern" "${scope[@]}" 2>/dev/null \
  | grep -v 'copy-check: allow' \
  | while IFS= read -r line; do
      # Drop the disclaimer, then see whether anything banned is left.
      rest=$(printf '%s' "$line" | sed -E "s/$disclaimer//Ig")
      if printf '%s' "$rest" | grep -qiE "$pattern"; then printf '%s\n' "$line"; fi
    done || true)

if [ -n "$hits" ]; then
  echo "[check-copy] FAIL: real-money language in player-facing text. Use game words instead."
  echo "$hits"
  exit 1
fi

echo "[check-copy] OK: no real-money language in crates/, README.md or docs/."
