# Test vectors

| File | Source | What it checks |
|---|---|---|
| `testnet-spike-2026-11-14.json` | Twelve transactions the league signed and the XRPL Testnet validated on 2026-11-14, as the server returned them (`tx_json`, `tx_blob`, `hash`). Receipt: `docs/receipts/m2b-own-layer-proof-2026-11-14/`. | The binary codec re-encodes each one to the validated blob, and the local hash is the ledger's. |
| `testnet-m2c-2026-11-14.json` | Nine TokenEscrow and DomainID transactions the Testnet validated in the M2c proof, as the server returned them (API v2 `tx_json`, and `tx_blob`). Receipt: `docs/receipts/m2c-patched-xrpl-rust-proof-2026-11-14/`. | Each re-encodes to its validated blob and hash, and parses into the patched model with the token amount or DomainID intact. |
| `xrpljs-codec-fixtures.json` | xrpl.js, commit `0ce90e29186139036018bac23b7b3ef2596dd990`, `packages/ripple-binary-codec/test/fixtures/codec-fixtures.json`. Unchanged. | All 39 transaction fixtures encode to their published binary. |
| `xrpljs-keypairs-api.json` | xrpl.js, commit `0ce90e29186139036018bac23b7b3ef2596dd990`, `packages/ripple-keypairs/test/fixtures/api.json`. Unchanged. | The published ed25519 seed derives its published public key and address; the secp256k1 seed is refused. |

The two xrpl.js files are under the ISC licence, which asks that its notice appear in every copy. The notices are in `LICENSE-xrpl.js`: the repository's and the two packages' (Ripple Labs Inc. and the contributors to xrpl.js).

The seeds in `xrpljs-keypairs-api.json` are xrpl.js's published test seeds, not accounts the league uses.

No published vector covers a token escrow or a DomainID. Those are covered by the league's own validated transactions in `testnet-m2c-2026-11-14.json`, and the DomainID field header by `a_domain_id_encodes_as_hash256_field_34`.
