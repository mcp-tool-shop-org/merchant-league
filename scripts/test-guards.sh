#!/usr/bin/env bash
# Negative and positive controls for the two guard scripts.
#
# For each case, the script copies the repository's guard scripts, crates,
# README and docs into a scratch tree, plants one line, runs the guard, and
# checks it fails (a "bad" case) or passes (a "good" case). A guard that
# stops catching a pattern makes this script fail.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd)
failures=0
declare -A cases=()

run_case() { # run_case <guard> <expect: fail|pass> <file> <line>
  local guard=$1 expect=$2 file=$3 line=$4
  cases["$guard $expect"]=$(( ${cases["$guard $expect"]:-0} + 1 ))
  local tmp
  tmp=$(mktemp -d)
  cp -r "$root/scripts" "$root/crates" "$root/docs" "$root/README.md" "$root/Cargo.toml" "$tmp/"
  [ -d "$root/.github" ] && cp -r "$root/.github" "$tmp/"
  mkdir -p "$(dirname "$tmp/$file")"
  printf '%s\n' "$line" >> "$tmp/$file"
  local got=pass
  (cd "$tmp" && bash "scripts/$guard" >/dev/null 2>&1) || got=fail
  rm -rf "$tmp"
  if [ "$got" != "$expect" ]; then
    echo "[test-guards] WRONG: $guard should $expect on $file: $line"
    failures=$((failures + 1))
  fi
}

t=check-testnet-only.sh
src=crates/league-ledger/src/planted.rs
# Mainnet hosts on every scheme, and bare.
for url in "https://s.ripple.com:51234" "http://s.ripple.com:51234" "ws://s.ripple.com:51233" \
  "wss://s.ripple.com" "s.ripple.com" "wss://s1.ripple.com" "https://s2.ripple.com" \
  "https://xrplcluster.com" "wss://xrpl.ws" "https://xrpl.link" "https://mainnet.xrpl-labs.com" \
  "wss://xrpl.to" "https://s.devnet.rippletest.net:51234" "https://livenet.xrpl.org"; do
  run_case $t fail "$src" "const U: &str = \"$url\";"
done
# Outside the crates too.
run_case $t fail README.md "Connect to https://s.ripple.com:51234."
run_case $t fail docs/planted.md "Use wss://xrplcluster.com."
run_case $t fail .github/workflows/planted.yml "url: https://s.ripple.com"
# A network id of 0, however it is spelled.
for code in 'pub const MAINNET_NETWORK_ID: u32 = 0;' 'let network_id: u32 = 0;' \
  'let v = json!({"network_id": 0});' 'network_id = 0' 'let n = NetworkId(0);'; do
  run_case $t fail "$src" "$code"
done
# The constant commented out and another set in its place.
run_case $t fail crates/league-ledger/src/lib.rs 'pub const OTHER_NETWORK_ID: u32 = 2;'
# Allowed: the Testnet itself, ids that are not 0, an explicit test marker, prose.
run_case $t pass "$src" 'const U: &str = "https://s.altnet.rippletest.net:51234";'
run_case $t pass "$src" 'let network_id: u32 = 1;'
run_case $t pass "$src" 'let network_id: u32 = 10;'
run_case $t pass "$src" 'let reply = json!({"network_id": 0}); // testnet-check: allow'
run_case $t pass docs/planted.md 'The guard refuses network id 0 and every other id.'
run_case $t pass "$src" '// the file xrpl.toml is not a host'

c=check-copy.sh
for phrase in "Pay the buy-in" "buy_in" "buy-ins" "cash out" "cashes out" "cashed out" \
  "cashing out" "cashout" "cash_out" "payout" "payouts" "pays out" "paid out" "paying out" \
  "pay_out" "invest" "investing" "investment" "investors" "wallet value" "withdraw funds" \
  "withdrew funds" "withdrawing money" "withdraw XRP" "real money prizes" "play for real money" \
  "real-money prizes" "real_money" "real profit" "earn money" "earns money" "made money" \
  "making money" "make real money" "win money" "won money" "win real money" "make_money" \
  "earn real rewards"; do
  run_case $c fail crates/league-tui/src/planted.rs "// $phrase"
done
run_case $c fail docs/planted.md "Players cash out at the end."
run_case $c fail README.md "Win real money!"
# "pay out" is caught even as "pay out of": write "pay from" in game copy.
run_case $c fail crates/league-tui/src/planted.rs "// Pay out of your purse."
# Ordinary trading language and the disclaimer pass.
for phrase in "Choose which goods to buy in town." "You cannot buy in Varn today." \
  "Sell salt at a profit." "A profitable trade route." "Investigate the order book." \
  "No real money." "There is no real money in this game." "This is not real money." \
  "The caravan pays its toll." "Pay the toll from your purse."; do
  run_case $c pass crates/league-tui/src/planted.rs "// $phrase"
done
run_case $c pass docs/planted.md "A copy check bans cash out and payout. <!-- copy-check: allow -->"

if [ "$failures" -gt 0 ]; then
  echo "[test-guards] FAIL: $failures case(s) wrong."
  exit 1
fi
for key in "check-testnet-only.sh fail" "check-testnet-only.sh pass" "check-copy.sh fail" "check-copy.sh pass"; do
  echo "[test-guards] $key: ${cases[$key]:-0} cases"
done
echo "[test-guards] OK: every planted case got the expected verdict."
