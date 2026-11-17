#!/usr/bin/env bash
# Negative and positive controls for the two tripwire scripts.
#
# The repository's files (tracked and new, less vendor/ and what Git
# ignores) are copied once into a template. Each case copies the template,
# plants a change, runs one guard, and checks it fails (a bad case) or passes
# (a near miss, or a clean tree). A change is planted two ways: by appending
# a line to a file, or by replacing text already in one. A guard that stops
# catching a pattern, or starts catching a near miss, makes this script fail.
#
# The Testnet guarantee itself is not here: it's `cargo test` in
# league-ledger (require_testnet and its tests). These are the tripwires.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
failures=0
declare -A cases=()

template=$(mktemp -d)
trap 'rm -rf "$template"' EXIT
git ls-files --cached --others --exclude-standard \
  | grep -v '^vendor/' \
  | while IFS= read -r f; do [ -f "$f" ] && printf '%s\n' "$f"; done \
  | tar -cf - -T - | tar -xf - -C "$template"

verdict() { # verdict <guard> <expect> <tmp> <what>
  local guard=$1 expect=$2 tmp=$3 what=$4 got=pass out
  cases["$guard $expect"]=$(( ${cases["$guard $expect"]:-0} + 1 ))
  out=$(cd "$tmp" && bash "scripts/$guard" 2>&1) || got=fail
  rm -rf "$tmp"
  # A failure counts only if the guard reported a finding, not if it crashed.
  if [ "$got" = fail ] && ! printf '%s' "$out" | grep -q '] FAIL: '; then
    got="error ($(printf '%s' "$out" | tail -1))"
  fi
  if [ "$got" != "$expect" ]; then
    echo "[test-guards] WRONG: $guard should $expect on $what"
    failures=$((failures + 1))
  fi
}

run_case() { # run_case <guard> <expect: fail|pass> <file> <line>: append the line
  local tmp
  tmp=$(mktemp -d)
  cp -r "$template/." "$tmp/"
  mkdir -p "$(dirname "$tmp/$3")"
  printf '%s\n' "$4" >> "$tmp/$3"
  verdict "$1" "$2" "$tmp" "$3 (appended): $4"
}

run_replace() { # run_replace <guard> <expect> <file> <old> <new>: replace text in place
  local tmp body
  tmp=$(mktemp -d)
  cp -r "$template/." "$tmp/"
  body=$(cat "$tmp/$3")
  if [[ "$body" != *"$4"* ]]; then
    echo "[test-guards] WRONG: $3 no longer contains the text to replace: $4"
    failures=$((failures + 1))
    rm -rf "$tmp"
    return
  fi
  printf '%s\n' "${body/"$4"/"$5"}" > "$tmp/$3"
  verdict "$1" "$2" "$tmp" "$3 (replaced): $4 -> $5"
}

clean() { # clean <guard>: the unchanged tree passes
  local tmp
  tmp=$(mktemp -d)
  cp -r "$template/." "$tmp/"
  verdict "$1" pass "$tmp" "the unchanged tree"
}

# --- The Testnet tripwire -------------------------------------------------
t=check-testnet-only.sh
src=crates/league-ledger/src/planted.rs
clean $t
# Every host pattern: each scheme, bare, ports, mixed case, subdomains.
for url in "https://s.ripple.com:51234" "http://s.ripple.com:51234" "ws://s.ripple.com:51233" \
  "wss://s.ripple.com" "s.ripple.com" "wss://s1.ripple.com" "https://s2.ripple.com" \
  "WSS://S1.Ripple.COM:51233" "wss://clio.s1.ripple.com:51233" "wss://fh-s1.ripple.com:51233" \
  "https://clio.ripple.com:51234" "ripple.com" \
  "https://xrplcluster.com" "wss://xrpl.ws" "https://xrpl.link" "wss://xrpl.to" \
  "https://mainnet.xrpl-labs.com" "https://xrpscan.com/account/r1" "https://bithomp.com/explorer" \
  "https://livenet.xrpl.org" "https://xrpl.org/mainnet" "https://rpc.ankr.com/xrpl_mainnet" \
  "wss://example.net/livenet" "https://s.devnet.rippletest.net:51234" \
  "mainnet.xrpl-labs.com" "livenet.xrpl.org" "xrpl.org/mainnet" \
  "https://www.bithomp.com" "https://dev.bithomp.com/explorer" "https://devnet.xrpl.org"; do
  run_case $t fail "$src" "const U: &str = \"$url\";"
done
# Everywhere in the repository, not just the crates.
for where in README.md docs/planted.md .github/workflows/planted.yml Cargo.toml scripts/planted.sh \
  CLAUDE.md CHANGELOG.md SECURITY.md tables/planted.toml; do
  run_case $t fail "$where" "# see wss://s1.ripple.com:51233"
done
# The Testnet endpoint replaced by a Mainnet one.
run_replace $t fail crates/league-ledger/src/lib.rs \
  '"https://s.altnet.rippletest.net:51234"' '"https://s1.ripple.com:51234"'
run_replace $t fail crates/league-ledger/src/lib.rs \
  '"wss://s.altnet.rippletest.net:51233"' '"wss://xrplcluster.com"'
# Near misses that must pass: the Testnet's own hosts and explorers, other
# names that only look close, prose, and the vendored crate (skipped).
for ok in "https://s.altnet.rippletest.net:51234" "wss://s.altnet.rippletest.net:51233" \
  "https://faucet.altnet.rippletest.net/accounts" "https://testnet.xrpl-labs.com" \
  "https://test.bithomp.com/explorer" "https://testnet.xrpl.org" "https://ripple.community" \
  "https://notripple.com" "https://xrpl.org/docs/references" "the file xrpl.toml is not a host"; do
  run_case $t pass "$src" "// $ok"
done
run_case $t pass docs/planted.md "Mainnet is out of bounds; the client refuses it."
# Network ids are not grepped: require_testnet's tests own that property.
run_case $t pass "$src" 'let network_id: u32 = 0;'
run_case $t pass vendor/xrpl-rust/src/planted.rs 'const U: &str = "wss://xrplcluster.com";'

# --- The copy tripwire ----------------------------------------------------
c=check-copy.sh
tui=crates/league-tui/src/planted.rs
clean $c
for phrase in "Pay the buy-in" "buy_in" "buy-ins" "Pay a 50 XRP buy in to join the table." \
  "Pay a 50 XRP buy in." "Sell marks for USD." \
  "a buy in to join" "the buy in fee" \
  "cash out" "cash  out" "cashes out" "cashed out" "cashing out" "cashout" "cash_out" \
  "win cash prizes" "Redeem marks for cash." \
  "payout" "payouts" "pays out" "paid out" "paying out" "pay_out" \
  "invest" "investing" "investment" "investors" \
  "wallet value" "walletValue" "wallet_values" \
  "withdraw funds" "withdrew funds" "withdrawing money" "withdraw XRP" "withdraw your funds" \
  "Deposit XRP to play." "deposit your funds" \
  "wager" "wagering" "the daily jackpot" \
  "real money prizes" "play for real money" "real-money prizes" "real_money" \
  "win prize money" "earn money" "earns money" "made money" "making money" "make real money" \
  "win money" "won money" "win real money" "make_money" "earn real rewards" "Win real prizes!" \
  "real profit" "real profits" "real_profit" "The real profit on the north road is salt." \
  "Each mark is worth \$5." "worth 5 USD" "sell marks for dollars" "10 euros a table" "10 €" "£5 a seat" "USD 5 a table"; do
  run_case $c fail "$tui" "// $phrase"
done
run_case $c fail "$tui" "// real	money"
run_case $c fail docs/planted.md "Players cash out at the end."
run_case $c fail README.md "Win real money!"
run_case $c fail site/src/content/docs/handbook/planted.md "Players cash out at the end."
run_case $c fail SECURITY.md "Players cash out at the end."
# A disclaimer on the line doesn't hide another banned phrase.
run_case $c fail "$tui" "// No real money, but you can cash out."
# A phrase split across two lines.
run_case $c fail docs/planted.md "Players can cash
out their marks at the end."
# "pay out" is caught even as "pay out of": write "pay from" in game copy.
run_case $c fail "$tui" "// Pay out of your purse."
# The README's disclaimer turned into a claim.
run_replace $c fail README.md "no real money" "real money prizes"
run_replace $c fail crates/league-tui/src/lib.rs "No real money." "Win real money."
# Near misses that must pass: trading language, disclaimers, game nouns.
for phrase in "Choose which goods to buy in town." "You cannot buy in Varn today." \
  "Sell salt at a profit." "A profitable trade route." "Profit on the north road is in salt." \
  "The truly profitable road is the north one." \
  "Investigate the order book." "The seller cashes on delivery." "The cashier counts marks." \
  "The caravan pays its toll." "Pay the toll from your purse." \
  "The wallet holds marks." "Withdraw an offer from the book." "Make a deposit of salt." \
  "Earn real renown in the league." "Win the table." "Make your mark." \
  "No real money." "There is no real money in this game." "This is not real money." \
  "It is never real money." "Played without real money." "This game is not for real money." \
  "Pay 5 marks." '{ "currency": "USD", "value": "10" }' "An IOU in USD on the Testnet." \
  "A hash, cut short: 40D45E1EE03B8EA1…" "Salt — and only salt — sells in Varn." "ISO 4217 codes" \
  "The guild pays its carters' wages." "A jack of all trades." "The jackdaw nests in Kell." \
  "An IOU in USD, held for the table."; do
  run_case $c pass "$tui" "// $phrase"
done
# Licence files under crates/ are skipped by exact name, and nothing else is.
for lic in crates/league-ledger/tests/vectors/LICENSE-xrpl.js crates/league-tui/LICENSE \
  crates/league-tui/LICENSE.md crates/league-tui/LICENSE.txt crates/vendor/xrpl-rust/LICENSE \
  crates/league-tui/LICENSE-MIT crates/league-tui/LICENSE-APACHE crates/vendor/xrpl-rust/LICENSE-rippled; do
  run_case $c pass "$lic" "Cash out now."
done
for other in docs/LICENSE docs/LICENSE-notes.md crates/league-tui/src/LICENSE_screen.rs \
  crates/league-tui/LICENSE-notes.md crates/league-ledger/tests/vectors/LICENSE-other.js \
  crates/league-tui/src/LICENSE-screen.rs crates/league-tui/src/LICENSE-screen.RS \
  crates/league-tui/LICENCE crates/league-tui/license.txt \
  crates/league-tui/LICENSES/notes.md crates/league-ledger/tests/vectors/notes.md \
  crates/league-tui/src/licence_cash.rs; do
  run_case $c fail "$other" "Cash out now."
done

# Recorded AI play is skipped by exact path: a game folder's log.md and
# metrics.md (the Publisher, 2026-11-15). Everything else stays checked,
# including the same game's other files and look-alike paths.
for rec in docs/paper-test/game-01/log.md docs/paper-test/game-02/metrics.md \
  docs/paper-test/game-07/log.md; do
  run_case $c pass "$rec" "Cash out now."
done
for other in docs/paper-test/game-02/score.md docs/paper-test/game-02/setup.md \
  docs/paper-test/game-02/cards/Tanners.md docs/paper-test/game-02/log.txt \
  docs/paper-test/game-02/log.md.bak docs/paper-test/game-02/notes/log.md \
  docs/paper-test/game-2/log.md docs/paper-test/game-002/log.md \
  docs/paper-test/log.md docs/paper-test/results.md docs/paper-test.md \
  docs/game-02/log.md crates/league-tui/src/log.md; do
  run_case $c fail "$other" "Cash out now."
done

if [ "$failures" -gt 0 ]; then
  echo "[test-guards] FAIL: $failures case(s) wrong."
  exit 1
fi
for key in "check-testnet-only.sh fail" "check-testnet-only.sh pass" "check-copy.sh fail" "check-copy.sh pass"; do
  echo "[test-guards] $key: ${cases[$key]:-0} cases"
done
echo "[test-guards] OK: every planted case got the expected verdict."
