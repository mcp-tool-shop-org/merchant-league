# Vendored: xrpl-rust 1.3.0, patched

This folder is **xrpl-rust 1.3.0 as published on crates.io**, with three changes on top. The workspace uses it through `[patch.crates-io]` in the root `Cargo.toml`.

## Where it comes from

- **Base:** the published crate `xrpl-rust` 1.3.0. Its SHA-256 is `c0c758f6900c150c4b4bd9ebef6f403717bdf23d047a2c3bf0da15dd8be4aae9`, matching crates.io's checksum.
- **Same as the tag:** its `src/` is identical (ignoring line endings) to the upstream tag `v1.3.0`, `2f2deb9f7daab682b6816458fcbece6c3bbe0fa6`, of https://github.com/XRPLF/xrpl-rust.
- **Licence:** ISC. `LICENSE` is the repository's, copied from the tag, because the published crate doesn't include it. Its notice must stay in every copy.
- **Files:** `Cargo.toml` (the published, normalised manifest), `Cargo.toml.orig`, `README.md` and `src/` are from the published crate. Its `Cargo.lock`, `.cargo-ok` and `.cargo_vcs_info.json` are left out. (The crate's `Cargo.lock` was committed by mistake at first and removed after Carbon's review on 2026-11-14. It pinned crypto-bigint 0.5.5 and atomic-polyfill 1.0.3, and Cargo never read it, since only the workspace's root lock counts.)

## Why

xrpl-rust 1.3.0 depends directly on crypto-bigint 0.5, which has an advisory with no fix in 0.5: RUSTSEC-2026-0387, timing variability in operations meant to be constant-time. Upstream fixed this after 1.3.0, but no release carries the fix yet. The repository's rule is that advisories are fixed, never ignored (Director, 2026-11-14), so the fix is applied here. Merchant League also needs two transaction features 1.3.0 doesn't have.

## The changes

1. **Upstream's own fix for RUSTSEC-2026-0387**, from XRPLF/xrpl-rust `2a5191b1442d2a77c9a89ae860f71b319ef693f2` (#376, 2026-10-26). Only two of its files are taken:
   - `Cargo.toml` (and `Cargo.toml.orig`): the `crypto-bigint` dependency is removed.
   - `src/core/keypairs/algorithms.rs`: secp256k1 secret-key validation goes through libsecp256k1 (`SecretKey::from_byte_array`), in constant time. This also fixes an off-by-one: the old check accepted a key equal to the curve order. Upstream's test for it is included.

   Not taken: the commit's `.cargo/audit.toml`, CI workflow and CHANGELOG edits. Both files were identical at the tag and at the fix's parent (`4d579ffe66`), so the change applied cleanly.
2. **TokenEscrow:** `EscrowCreate.amount` is a full `Amount`, XRP or a fungible token, where it was `XRPAmount` only. `EscrowCreate::new` takes `impl Into<Amount>`, so code passing an `XRPAmount` keeps compiling. Tests: a token escrow's JSON round trip, an MPT escrow's, and the constructor still taking drops.
   - **Expiry rules, as rippled checks them** (`EscrowCreate::preflight`, rippled 3.4.1), the same for XRP and tokens:
     - `FinishAfter` or `CancelAfter` is required;
     - without `FinishAfter`, a `Condition` is required;
     - `CancelAfter` must be after `FinishAfter` (already checked by upstream).

     Measured on the Testnet in `docs/receipts/design-check-escrow-expiry-2026-11-14/`.
   - **The condition, as rippled decodes it** (`Condition::deserialize`, rippled 3.4.1):
     - a DER PREIMAGE-SHA-256 condition, at most 128 bytes, with nothing after it;
     - a 32-byte fingerprint;
     - a cost of 1 to 5 bytes, at most 128;
     - the other condition types are refused, as rippled refuses them.

     An empty or malformed condition is refused before signing.
3. **DomainID (Permissioned DEX):** an optional `domain_id` on `OfferCreate` and `Payment`, serialised as `DomainID` and set with `with_domain_id`. It's validated by `PermissionedDomainSet`'s own `validate_domain_id`: 64 hex digits in either case, not all zero. The binary codec's definitions already had the field (Hash256, number 34). Tests: JSON round trip, validation, and omission when unset.

Upstream main (`b345290`, 2026-11-14) has neither the TokenEscrow nor the DomainID change. Offering them upstream is approved, after a Carbon review, and the Director is told before the PR opens.

## The build uses

`default-features = false` with `std`, `core`, `utils`, `models` and `wallet`. The `json-rpc` feature (reqwless, embedded-tls) is **off**: it would bring crypto-bigint 0.5 back through p256 0.13, plus atomic-polyfill (RUSTSEC-2023-0089). Submission goes through the league's own JSON-RPC client instead.

**Mainnet:** the library's only Mainnet hosts (`DEFAULT_MAINNET_URL` and `DEFAULT_WEBSOCKET_URL`, both xrplcluster.com) are in `src/cli/`. That module is compiled only with the `cli` feature (`src/lib.rs:40`), which the league leaves off, so they're not in the game's build. The league's guard still refuses any server that doesn't report the Testnet's network id.

**Formatting:** `escrow_create.rs`, `offer_create.rs` and `payment.rs` are formatted with rustfmt (edition 2021, no config), as upstream's own files are. The pass touched only layout, and it reformatted the TokenEscrow and DomainID lines added before it.

## Checks at vendoring

- `cargo tree -i crypto-bigint`: no such package in the lock.
- `cargo audit --deny warnings`: clean, with no ignores.
- The vendored crate's own tests: 1,283 pass with these features, including the new ones.

## Live proof

Both new features were proved on the XRPL Testnet on 2026-11-14 (`docs/receipts/m2c-patched-xrpl-rust-proof-2026-11-14/`):

- **Token escrow.** It was refused (`tecNO_PERMISSION`) until the issuer set Allow Trust Line Locking, then created and finished.
- **Expiry.** A token escrow with a condition and no expiry was refused with `temBAD_EXPIRATION`.
  - This was first written up as "token escrows need `CancelAfter`", which was wrong.
  - rippled's rule is general: `FinishAfter` or `CancelAfter`, and a `Condition` without `FinishAfter`.
  - A token escrow with only `FinishAfter` was accepted live.
  - The model now checks the general rule.
- **DomainID.** Domain offers were accepted for credentialed accounts and refused for an outsider. An open-book offer did not cross a domain offer.

The validated transactions are test vectors in `crates/league-ledger/tests/vectors/`.

## Known, not patched

- `Credential` (used by `PermissionedDomainSet`) deserialises only from borrowed text, not from a `serde_json::Value`. The league builds that transaction with its constructor. This is a candidate for the upstream offer.

## Updating

When an xrpl-rust release carries the fix and these two features, drop this folder and the `[patch.crates-io]` entry, and pin that release. Until then, any change here is reviewed like the league's own code.
