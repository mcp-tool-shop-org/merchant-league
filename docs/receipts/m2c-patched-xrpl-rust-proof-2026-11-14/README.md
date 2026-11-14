# M2c receipt: patched xrpl-rust, live on the XRPL Testnet

**When:** 2026-11-14, from about 2:15 AM EST (07:15 UTC).
**Where:** the public XRPL Testnet, network id 1, rippled 3.4.1. The client refuses any other network before it sends anything.
**What ran:** `cargo run -p league-ledger --example testnet_proof` on branch `m2c/xrpl-rust-patched`. The transactions were built with xrpl-rust's typed models and signed with `xrpl::signing::sign`. They were encoded with xrpl-rust's binary codec, hashed locally and submitted through the league's own JSON-RPC client.
**The build:** vendored xrpl-rust 1.3.0 with upstream's RUSTSEC-2026-0387 fix, TokenEscrow and DomainID (`vendor/xrpl-rust/VENDORED.md`); features `std core utils models wallet`, with `json-rpc` off. `cargo tree -i crypto-bigint` finds no such package (`raw/00_snapshot.txt`).

## Result

**Every step came out as expected. Exit 0, and the summary line lists no unexpected step.**

The run used four faucet accounts, one faucet call each. Each faucet seed was ed25519 and derived to the address the faucet reported. Seeds were never printed.

An independent check (`raw/hash_check.py`, Python, no league code) looked up all 29 hashes on the Testnet. Every validated step is in a validated ledger with the same result and ledger index (`raw/02_hash_check.json`, mismatches: none).

## Steps

| Step | Result | Expected | Ledger | Hash |
|---|---|---|---|---|
| issuer_allow_clawback | tesSUCCESS | tesSUCCESS | 22313205 | `40D45E1EE03B8EA1…` |
| issuer_default_ripple | tesSUCCESS | tesSUCCESS | 22313207 | `D0739EB7733622DF…` |
| merchant_trust_set | tesSUCCESS | tesSUCCESS | 22313209 | `E7A7BBF32DF3F5B5…` |
| trader_trust_set | tesSUCCESS | tesSUCCESS | 22313211 | `A51D76A1A636148E…` |
| outsider_trust_set | tesSUCCESS | tesSUCCESS | 22313213 | `1E433AB43677AC03…` |
| issuer_pays_merchant_100 | tesSUCCESS | tesSUCCESS | 22313215 | `D82F6FB14222752B…` |
| merchant_offer_create | tesSUCCESS | tesSUCCESS | 22313217 | `78CA6D60B43C85E0…` |
| merchant_offer_cancel | tesSUCCESS | tesSUCCESS | 22313219 | `306BF85B3BDACE7B…` |
| issuer_check_create | tesSUCCESS | tesSUCCESS | 22313221 | `615C3BF66647CEAD…` |
| merchant_check_cash | tesSUCCESS | tesSUCCESS | 22313223 | `85C6ACD4AD920F60…` |
| merchant_escrow_create | tesSUCCESS | tesSUCCESS | 22313225 | `D9366D4A64AD1A79…` |
| merchant_escrow_finish_wrong_secret | tecCRYPTOCONDITION_ERROR | tecCRYPTOCONDITION_ERROR | 22313227 | `44E62D1FC64D4D64…` |
| merchant_escrow_finish | tesSUCCESS | tesSUCCESS | 22313229 | `0ACA9DAE00320739…` |
| issuer_clawback_3 | tesSUCCESS | tesSUCCESS | 22313231 | `E7A58A3FD65BE7C3…` |
| token_escrow_without_locking_flag | tecNO_PERMISSION | tecNO_PERMISSION | 22313233 | `3A27453E69DA1BF5…` |
| issuer_allow_trustline_locking | tesSUCCESS | tesSUCCESS | 22313235 | `80E73BC54155303B…` |
| token_escrow_probe_no_cancel_after | temBAD_EXPIRATION (not validated) | recorded | — | `C1340FC25A25C9D5…` |
| token_escrow_create | tesSUCCESS | tesSUCCESS | 22313259 | `ED5D309D29AE7716…` |
| token_escrow_finish_by_trader | tesSUCCESS | tesSUCCESS | 22313261 | `EE2D0EC9813C4727…` |
| issuer_credential_for_merchant | tesSUCCESS | tesSUCCESS | 22313263 | `76430FF1FF5E6041…` |
| issuer_credential_for_trader | tesSUCCESS | tesSUCCESS | 22313265 | `DD479BE5603D81E4…` |
| merchant_credential_accept | tesSUCCESS | tesSUCCESS | 22313267 | `E1490D3E0749CB48…` |
| trader_credential_accept | tesSUCCESS | tesSUCCESS | 22313269 | `92B1D85BABA44716…` |
| issuer_domain_set | tesSUCCESS | tesSUCCESS | 22313271 | `5B4C323604ED777A…` |
| merchant_domain_offer_sell_1 | tesSUCCESS | tesSUCCESS | 22313273 | `73276A1AD97311C1…` |
| outsider_open_offer_buy_1 | tesSUCCESS | tesSUCCESS | 22313275 | `F29195AD40107D33…` |
| outsider_domain_offer_refused | tecNO_PERMISSION | tecNO_PERMISSION | 22313277 | `9B05722D1841D0E7…` |
| trader_domain_offer_buy_1 | tesSUCCESS | tesSUCCESS | 22313279 | `7741BA8662AFDB83…` |
| merchant_domain_payment_to_trader | tesSUCCESS | recorded | 22313281 | `1C2BE6FF7719D31F…` |

Full hashes, sequences and preliminary results are in `raw/01_proof.out`.

## Final state

- **MRK balances:** merchant 96, trader 6, outsider 0.
  - Merchant: 100 paid in, +5 cashed from the check, −3 clawed back, −5 into the token escrow, −1 sold in the domain.
  - Trader: 5 from the token escrow, +1 bought in the domain.
- **Objects left:** no escrows and no checks. The merchant and trader have no offers. The outsider has one offer, its open-book one.
- **Credentials:** the issuer holds 2 credential objects.
- **Domain:** `B3C99893EE264C631C0062764C974F2AB9C4324BA76A554D96DC59E179369CB1`.

## What it shows

1. **The patched build does everything M2 proved.** Signing, a trust line, a payment, an offer and its cancel, a check and its cash, and clawback all work. The escrow with a condition refused the wrong secret (`tecCRYPTOCONDITION_ERROR`) and released for the right one.
2. **TokenEscrow works, and needs two things.**
   - **The issuer must set Allow Trust Line Locking** (flag 17). Before the flag, a token escrow fails with `tecNO_PERMISSION`, validated.
   - **A token escrow must have `CancelAfter`.** Without it, the server refuses the transaction with `temBAD_EXPIRATION`, and it never reaches a ledger. xrpl.org's EscrowCreate reference doesn't state this; the seed 25 harvest did. The league's escrow bonds will always set `CancelAfter`.
   - The model doesn't enforce `CancelAfter`, because the reference doesn't state the rule. It's a candidate for the upstream offer, as a validation with this receipt as evidence.
3. **DomainID works, and keeps a domain's market apart.**
   - A credentialed member's offer was accepted in the domain.
   - An account without the credential was refused (`tecNO_PERMISSION`).
   - The outsider's open-book offer, at a price that would have crossed the member's domain offer, **did not cross it**: the outsider still holds 0 MRK and its offer is still open.
   - A second member's domain offer did cross, and the trader received 1 MRK.
   - This is the behaviour the redesign relies on for separate markets per table.
   - A same-currency XRP payment carrying a DomainID was accepted. xrpl.org says the field only affects cross-currency payments; this run doesn't test a cross-currency domain payment.
4. **The local hash is the ledger's.** Every hash was computed before submission and found in a validated ledger, and the refused probe kept its hash too.

## Vectors kept

`crates/league-ledger/tests/vectors/testnet-m2c-2026-11-14.json` holds 9 of these transactions as the server returned them (API v2 `tx_json`, plus `tx_blob` from a binary lookup): the two token escrows, their finish, the locking flag, the domain, three domain offers and the domain payment. `the_validated_token_escrow_and_domain_transactions_round_trip` checks two things for each one:

- it re-encodes to the validated blob and hash;
- it parses into the patched model with the token amount or the DomainID intact.

API v2 returns a Payment's `Amount` as `DeliverMax`, so the test maps it back before encoding.

## Found along the way

- **xrpl-rust's `Credential` can't be deserialised from a `serde_json::Value`.** It deserialises only from borrowed text, so a `PermissionedDomainSet` can't be built from JSON. The proof uses its constructor. This is a candidate upstream fix, not patched here.

## Files

- `raw/00_snapshot.txt`: time, branch, toolchain, `cargo tree -i crypto-bigint`.
- `raw/01_proof.out`, `raw/01_proof.err` (empty), `raw/01_proof.exit` (0).
- `raw/02_hash_check.json` and `raw/hash_check.py`: the independent lookup.
