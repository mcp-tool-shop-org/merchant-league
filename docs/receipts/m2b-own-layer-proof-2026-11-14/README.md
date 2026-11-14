# M2b proof: the league's own XRPL layer on Testnet, 2026-11-14

The same steps as the M2 xrpl-rust spike (`../m2-xrpl-rust-spike-2026-11-14/`), now on Merchant League's own layer: our ed25519 keys, our serializer, our signing and our JSON-RPC client. **The verdict is PASS**, with the same outcome as the spike.

- **Code:** `crates/league-ledger/examples/testnet_proof.rs` at commit `fdeeb19`, run from a clean tree (`raw/00_snapshot.txt`). It ran 12:49 to 12:51 AM EST on 14 Nov.
- **Network:** the public Testnet, through `Testnet::connect`, which refuses any server that does not report network id 1 (`network_id` 1, rippled 3.4.1).
- **Accounts:** two faucet calls, an issuer and a merchant. For each, the client checked that the seed the faucet issued derives, by our ed25519 path, to the address the faucet reported, so both accounts are live seed-to-account vectors. Seeds stayed in memory and were never printed or saved.
- **Redaction:** none needed; the addresses are public Testnet addresses.

## Results

Every hash below was computed locally, before submission. The client then found each result on the ledger by that hash.

| Step | Transaction | Result | Ledger |
| --- | --- | --- | --- |
| Issuer allows clawback | `AccountSet` | `tesSUCCESS` | 22,311,628 |
| Issuer default ripple | `AccountSet` | `tesSUCCESS` | 22,311,630 |
| Merchant trusts the issuer | `TrustSet` MRK, limit 1,000 | `tesSUCCESS` | 22,311,632 |
| Issuer pays 100 MRK | `Payment` | `tesSUCCESS` | 22,311,634 |
| An offer on the order book | `OfferCreate` 10 MRK for 1 XRP | `tesSUCCESS` | 22,311,636 |
| Its cancel | `OfferCancel` | `tesSUCCESS` | 22,311,638 |
| A contract | `CheckCreate` 5 MRK | `tesSUCCESS` | 22,311,640 |
| Delivery | `CheckCash` 5 MRK | `tesSUCCESS` | 22,311,643 |
| Dice commitment | `EscrowCreate`, locked to a secret's condition | `tesSUCCESS` | 22,311,645 |
| Reveal with the wrong secret | `EscrowFinish` | **`tecCRYPTOCONDITION_ERROR`**, validated, **with its hash** (`3169CDE4…`) | 22,311,647 |
| Reveal with the right secret | `EscrowFinish` | `tesSUCCESS` | 22,311,649 |
| Clawback | `Clawback` 3 MRK | `tesSUCCESS` | 22,311,651 |

**Final state, from the validated ledger** (`raw/01_proof.out`, last line):

- Merchant's MRK balance: **102**, as predicted and as in the spike.
- Escrows, checks and offers left: **0** each.

**Hash check** (`raw/02_hash_check.json`): all 12 locally computed hashes are in the two accounts' validated history with the same result, and the history holds no transaction of theirs that is not ours.

## What this closes from the spike

- **A failed transaction has its hash.** xrpl-rust returned the failed reveal as an error with no hash. Here it has one, and its validated result.
- **No vulnerable dependency.** The lock has no crypto-bigint, atomic-polyfill or secp256k1, and `cargo audit --deny warnings` passes.

## Offline evidence (in the repository's tests)

`crates/league-ledger/tests/testnet_vectors.rs` checks the twelve transactions the xrpl-rust spike signed and validated, captured before the 2026-11-25 reset:

- each re-encodes to the ledger's own bytes;
- each hash equals the one the Testnet reported;
- each signing key derives to its account;
- each signature verifies over our signing data.

The official binary-codec fixtures from xrpl.js are to be added once their licence is confirmed, which R&D has been asked to do.

## Run 1

The first run of this proof, at commit `ffac208`, stopped after `EscrowCreate`. It read the escrow's sequence back in the newer API's shape, and the server answers in the older one. Every transaction up to that point validated. The fix (`fdeeb19`) has the client return the sequence it used in each receipt. Run 1's output is kept as `raw/90_run1_*`.

## Layout

- `raw/00_snapshot.txt`: the commit, start and end of run 2.
- `raw/01_proof.out`, `raw/01_proof.exit`: one JSON line per step; exit code 0.
- `raw/02_hash_check.json`: our hashes against both accounts' validated history.
- `raw/90_run1_*`: the run that stopped, and its snapshot.
