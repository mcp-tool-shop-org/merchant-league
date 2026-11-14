# Design check: RequireAuth, clawback and token escrow on one trust line

**When:** 2026-11-14, about 6:20 AM EST (11:20 UTC).
**Where:** the public XRPL Testnet, network id 1, rippled 3.4.1.
**Why:** `docs/design.md` (PR #6), "Check before building": the steward would issue each table's goods on authorized trust lines (RequireAuth), keep clawback (spoiling) and allow trust-line locking (bonds), all on one issuer. This proves they work together before the Director decides.
**What ran:** `cargo run -p league-ledger --example testnet_requireauth`, with four faucet accounts:
- the issuer;
- two seated guilds, alice and bob;
- an outsider whose trust line is never authorized.

Seeds were never printed.

## Result

**Every expected step came out as expected, and the program exited 0.** An independent lookup (`raw/hash_check.py`) found all 19 hashes in validated ledgers with the same results (`raw/02_hash_check.json`, no mismatches).

| Step | Result | Expected | Ledger | Hash |
|---|---|---|---|---|
| issuer_require_auth | tesSUCCESS | tesSUCCESS | 22317006 | `2E5CDF55F40BD0D4…` |
| issuer_allow_clawback | tesSUCCESS | tesSUCCESS | 22317008 | `A129AFA324332DCC…` |
| issuer_allow_locking | tesSUCCESS | tesSUCCESS | 22317010 | `B9E1638BD44697BC…` |
| issuer_default_ripple | tesSUCCESS | tesSUCCESS | 22317012 | `F48B7499640FA7CE…` |
| alice_trust | tesSUCCESS | tesSUCCESS | 22317014 | `534F181C02F3110D…` |
| bob_trust | tesSUCCESS | tesSUCCESS | 22317016 | `5CABB56D5C213035…` |
| outsider_trust | tesSUCCESS | tesSUCCESS | 22317018 | `1BB80C21206C6CF6…` |
| issuer_pays_unauthorized_outsider | tecPATH_DRY | recorded | 22317020 | `ED72C7DA70628941…` |
| issuer_authorizes_alice | tesSUCCESS | tesSUCCESS | 22317022 | `57A4A977FF9C09F8…` |
| issuer_authorizes_bob | tesSUCCESS | tesSUCCESS | 22317024 | `A8C336BED8E0810B…` |
| issuer_pays_alice_100 | tesSUCCESS | tesSUCCESS | 22317026 | `8D04CEAFB06FAA9A…` |
| alice_pays_unauthorized_outsider | tecPATH_DRY | recorded | 22317028 | `CD6099FC93CEC3A8…` |
| alice_escrows_10_to_bob | tesSUCCESS | tesSUCCESS | 22317030 | `3740A06E905129FA…` |
| bob_finishes_10 | tesSUCCESS | tesSUCCESS | 22317032 | `0F2CEA2C7D37BCB1…` |
| alice_escrows_5_to_unauthorized_outsider | tecNO_AUTH | recorded | 22317034 | `B4FAFD379DDB0121…` |
| issuer_claws_3_from_alice | tesSUCCESS | tesSUCCESS | 22317036 | `F2B7AE66BE3E63AF…` |
| alice_escrows_20_to_bob | tesSUCCESS | tesSUCCESS | 22317039 | `201D16DB26313C2F…` |
| issuer_claws_87_from_alice | tesSUCCESS | recorded | 22317041 | `80AC46CECD50FB07…` |
| bob_finishes_20_after_clawback | tesSUCCESS | recorded | 22317043 | `6E5A5EC4FBC69163…` |

**Final balances:** alice 0, bob 30 and the outsider 0. Alice and bob's lines show authorized; the outsider's doesn't.

## What it shows

1. **All four issuer flags work together.** RequireAuth, Allow Trust Line Clawback, Allow Trust Line Locking and DefaultRipple were set in that order before any trust line existed. `account_info` then showed all four on the issuer.
2. **An unauthorized line holds nothing.** The issuer can't pay the outsider, and a guild can't either: both fail with `tecPATH_DRY`. A token escrow to the outsider is refused with `tecNO_AUTH`. So per-table goods on authorized lines isolate a table, with no domain needed for that.
3. **Token escrow works on authorized lines.** It was created and finished between two authorized guilds.
4. **Clawback doesn't reach tokens in escrow.**
   - With 20 of alice's tokens in escrow, her line showed 67.
   - Asked to claw back 87, the issuer took the 67 on the line. The result was `tesSUCCESS`, and her balance went to 0.
   - The 20 in escrow then finished to bob.

   Two things follow for the design:
   - **bonds can't be spoiled or clawed back while locked**, so a bond settles only by its escrow rules;
   - **perishable goods must never be locked in an escrow.** Bonds are in table coin, which doesn't spoil.

   Also, a clawback for more than the line holds takes what is there and still reports `tesSUCCESS`. The steward must read the balance first, not trust the amount it asked for.

## Files

- `raw/00_snapshot.txt`: time, branch, toolchain.
- `raw/01_probe.out`: one JSON line per step, with full hashes. Also `raw/01_probe.err` (empty) and `raw/01_probe.exit` (0).
- `raw/02_hash_check.json` and `raw/hash_check.py`: the independent lookup.
