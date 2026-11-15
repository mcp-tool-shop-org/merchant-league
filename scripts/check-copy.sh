#!/usr/bin/env bash
# A tripwire: fails when player-facing text (the crates, the README and the
# docs) frames the game as real money.
#
# Game words are fine, including ordinary trading language: "buy in Varn",
# "sell at a profit", "a profitable route" are about goods and marks (the
# Director ruled on 2026-11-14 that trading words are game language). What is
# banned is language that frames the game as earning, paying out, wagering
# or withdrawing money, "real profit", or a price in a real currency. Saying
# there is no real money is allowed.
#
# This is a tripwire, not a proof: a word list can't catch every phrasing.
# Review is the rest of the check. There is no allow marker: a line that
# trips this is reworded. scripts/test-guards.sh plants every pattern below,
# and near misses that must pass.
#
# A phrase split across two lines is caught: each pair of adjacent lines is
# checked joined. "pay out" is caught even in "pay out of your purse"; game
# copy says "pay from".
#
# Third-party licence files under crates/ are skipped, because their text
# must stay verbatim (ISC asks that its notice appear unchanged in every
# copy) and they are not player-facing. Only these exact names are skipped:
# LICENSE, LICENSE.md, LICENSE.txt, LICENSE-MIT, LICENSE-APACHE,
# LICENSE-xrpl.js (the xrpl.js notice beside its test vectors) and
# LICENSE-rippled (rippled's notice, for ported code). Any other
# name, such as LICENSE-notes.md, is checked. A new licence file's exact
# name is added here.
#
# Recorded AI play is skipped too (the Publisher, 2026-11-15): the paper
# test's referee logs and metrics, docs/paper-test/game-NN/log.md and
# metrics.md. They quote what AI seats wrote, verbatim; that text is quoted,
# not authored, and rewording it would falsify the record. Only those two
# exact names in a two-digit game folder are skipped. The game's setup,
# score, cards and every other file under docs/ are checked, as is the
# README.
set -euo pipefail
cd "$(dirname "$0")/.."

sep='[[:space:]_-]'
money='(money|cash|prizes?|rewards?|xrp|dollars?|euros?)'

banned=(
  "\bbuy[_-]ins?\b"                                                    # buy-in, buy_in
  "\b(xrp|[0-9]+)${sep}+buy${sep}+in\b"                                # a 50 XRP buy in
  "\bbuy${sep}+in${sep}+(fee|to${sep}+join|price|cost)"                # buy in to join, buy in fee
  "\bcash(es|ed|ing)?${sep}*outs?\b"                                   # cash out, cashed  out, cashout
  "\bcash${sep}+prizes?\b"
  "\bfor${sep}+cash\b"
  "\b(pay|pays|paying|paid)${sep}*outs?\b"                             # payout, pays out, paid_out
  "\binvest(s|ed|ing|ment|ments|or|ors)?\b"                            # invest, investor
  "\bwallet${sep}*values?\b"                                           # wallet value, walletValue
  "\bwithdr(aw|aws|awing|awn|ew)${sep}+([a-z]+${sep}+)?(funds|money|xrp|winnings)\b"  # withdraw your funds
  "\bdeposit(s|ed|ing)?${sep}+([a-z]+${sep}+)?(funds|money|xrp)\b"     # deposit XRP
  "\bwager(s|ed|ing)?\b"
  "\bjackpots?\b"
  "\breal${sep}*money\b"                                               # real money (disclaimers excepted)
  "\breal${sep}+profits?\b"                                           # real profit: banned under the ruling
  "\bprize${sep}+money\b"
  "\b(earn|earns|earned|earning|make|makes|made|making|win|wins|won|winning)${sep}+(real${sep}+)?money\b"
  "\b(earn|earns|earned|earning|make|makes|made|making|win|wins|won|winning)${sep}+real${sep}+${money}\b"
  "\\\$${sep}*[0-9]"                                                   # \$5
  # A currency code only next to an amount: XRPL test data uses USD as an
  # ordinary ledger currency code ("currency": "USD"), which isn't a price.
  "\b[0-9]+${sep}*(usd|eur|gbp)\b"                                     # 5 USD
  "\b(usd|eur|gbp)${sep}*[0-9]"                                        # USD 5
  "[0-9]${sep}*(dollars?|euros?)\b"                                    # 5 dollars
  "\b(for|into|to)${sep}+(dollars|euros|usd|eur|gbp)\b"               # sell marks for dollars, for USD
  # Alternation, not a bracket: where grep reads bytes, a bracket of
  # multi-byte symbols would match any of their bytes, such as the first
  # byte of an ellipsis.
  "(€|£)"
)
pattern=$(IFS='|'; echo "${banned[*]}")

# Saying there is no real money: no, not, never, without, not for.
disclaimer="\b(no|not|never|without)(${sep}+for)?${sep}+real${sep}*money\b"

# The files: the crates, the docs and the README, less the licence files
# and the recorded play.
licence='^crates/(.*/)?(LICENSE|LICENSE\.md|LICENSE\.txt|LICENSE-MIT|LICENSE-APACHE|LICENSE-xrpl\.js|LICENSE-rippled)$'
recorded='^docs/paper-test/game-[0-9]{2}/(log|metrics)\.md$'
all=$( { find crates docs -type f 2>/dev/null; [ -f README.md ] && echo README.md; } | sort)
skip=$( {
  printf '%s\n' "$all" | grep -E "$licence" | grep -viE '\.rs$' || true
  printf '%s\n' "$all" | grep -E "$recorded" || true
} | grep -v '^$' || true)
files=$(printf '%s\n' "$all" | grep -vxF -e "$skip" -e '' || true)
[ -z "$skip" ] && files=$all
# Text files only.
text=$(printf '%s\n' "$files" | xargs -d '\n' grep -lI '' 2>/dev/null || true)

candidates=$(
  {
    # Each line, then each pair of adjacent lines joined with a space.
    printf '%s\n' "$text" | xargs -d '\n' grep -nHiE "$pattern" 2>/dev/null || true
    printf '%s\n' "$text" | xargs -d '\n' awk 'FNR > 1 { print FILENAME ":" FNR - 1 ":" prev " " $0 } { prev = $0 }' 2>/dev/null \
      | grep -iE "$pattern" || true
  }
)

hits=$(printf '%s\n' "$candidates" | while IFS= read -r line; do
  [ -z "$line" ] && continue
  # Drop the disclaimers, then see whether anything banned is left.
  rest=$(printf '%s' "$line" | sed -E "s/$disclaimer//Ig")
  if printf '%s' "$rest" | grep -qiE "$pattern"; then printf '%s\n' "$line"; fi
done | sort -u)

if [ -n "$hits" ]; then
  echo "[check-copy] FAIL: real-money language in player-facing text. Use game words instead."
  echo "$hits"
  exit 1
fi

echo "[check-copy] OK: no real-money language in crates/, README.md or docs/."
