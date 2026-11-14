# Design check: which expiry an escrow needs

**When:** 2026-11-14, about 6:30 AM EST (11:30 UTC).
**Where:** the public XRPL Testnet, network id 1, rippled 3.4.1.
**What ran:** `cargo run -p league-ledger --example testnet_escrow_expiry`, with two faucet accounts: an issuer with Allow Trust Line Locking, and a holder. Seeds were never printed.

## Why

**This corrects a claim I made earlier.**
- The M2c proof (`docs/receipts/m2c-patched-xrpl-rust-proof-2026-11-14/`) sent a token escrow that had a condition but no `CancelAfter` and no `FinishAfter`. It was refused with `temBAD_EXPIRATION`.
- I wrote that up as "a token escrow must have `CancelAfter`". That's wrong: one refusal was read as a rule.
- rippled 3.4.1's `EscrowCreate::preflight` (`src/libxrpl/tx/transactors/escrow/EscrowCreate.cpp`) has these rules, and they are the same for XRP and tokens:
  - **at least one of `FinishAfter` or `CancelAfter`**, or `temBAD_EXPIRATION`;
  - if both are given, `CancelAfter` must be after `FinishAfter` (`temBAD_EXPIRATION`);
  - **with no `FinishAfter`, a `Condition` is required** (`temMALFORMED`), so the escrow can't be finished at once.

This probe checks those rules on the ledger.

## Result

**Every step came out as expected, and the program exited 0.**
- A `tem` result never reaches a ledger, so those three rows are the server's preliminary result. Their hashes aren't in any validated ledger, which the independent lookup confirms.
- The other six hashes are in validated ledgers with the same results (`raw/02_hash_check.json`, no mismatches).

| Step | Result | Expected | Ledger | Hash |
|---|---|---|---|---|
| issuer_allow_locking | tesSUCCESS | tesSUCCESS | 22317110 | `434E656132827EC2…` |
| issuer_default_ripple | tesSUCCESS | tesSUCCESS | 22317112 | `5833A94884B08D8B…` |
| holder_trust | tesSUCCESS | tesSUCCESS | 22317114 | `7BFE18CA7984D799…` |
| issuer_pays_holder_50 | tesSUCCESS | tesSUCCESS | 22317116 | `17B41A64E90763EE…` |
| xrp_escrow_condition_only | temBAD_EXPIRATION (not validated) | temBAD_EXPIRATION | — | `0D5E8BBAFFF249B7…` |
| token_escrow_condition_only | temBAD_EXPIRATION (not validated) | temBAD_EXPIRATION | — | `4EFD824BD85F75FE…` |
| token_escrow_cancel_after_only | temMALFORMED (not validated) | temMALFORMED | — | `A3C9EC90D075D278…` |
| token_escrow_finish_after_only | tesSUCCESS | tesSUCCESS | 22317140 | `10C45308AE0D5CE8…` |
| token_escrow_finished_after_finish_after | tesSUCCESS | tesSUCCESS | 22317148 | `44BE30F874017D1D…` |

## What it shows

1. **The refusal M2c saw is general, not about tokens.** An XRP escrow with only a condition is refused with `temBAD_EXPIRATION`, exactly like the token escrow.
2. **A token escrow doesn't need `CancelAfter`.** One with only `FinishAfter` (no `CancelAfter`, no condition) was accepted, and finished after its time.
3. **No `FinishAfter` means a condition is required:** a token escrow with only `CancelAfter` was refused with `temMALFORMED`.
4. **For the design, the bonds keep `CancelAfter` by choice, not because the ledger demands it.** The bonds have a condition (the referee secret) and need a time after which each side can take its bond back.
5. **For the vendored model,** the validation mirrors rippled's preflight for every escrow, XRP or token. It does not say "tokens need `CancelAfter`". See `vendor/xrpl-rust/VENDORED.md`.

## Re-running it

Since merchant-league #7, the vendored model checks rippled's escrow rules before signing:
- the expiry rules;
- since the follow-up PR, the condition's encoding as well.

A re-run of this probe stops at its first refusal step with a model error, before the server sees it. Those steps were sent to get the server's answer, and the table above records it. No way around the local check exists in the library, by design.

## Files

- `raw/00_snapshot.txt`: time, branch, toolchain, and the rippled source read.
- `raw/01_probe.out`: one JSON line per step, with full hashes. Also `raw/01_probe.err` (empty) and `raw/01_probe.exit` (0).
- `raw/02_hash_check.json` and `raw/hash_check.py`: the independent lookup. `hash_check.py` was trimmed after the run, in the follow-up PR to #7: the M2c vector-writing code was removed, and `02_hash_check.json` is the record from the run. Re-running it gives the same verdicts. The only change is that the server no longer caches the three refused transactions, so they now read "not found" instead of "found, not validated". Either way they never reached a ledger.
