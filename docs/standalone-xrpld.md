# Stand-Alone `xrpld` Harness (`league-standalone`)

**Status:** Authorized by the v2 orchestrator card Part 4 and the Publisher's container limits (2026-11-15). Conflict 11 in `docs/design-v2.md` section 9 (line 327) remains an open decision for the Director.

This document specifies the containerized local stand-alone `xrpld` harness for headless evaluation and ledger-conformance testing (`crates/league-standalone`), keeping `league-ledger` strictly bound to public XRPL Testnet (`network_id == 1`).

---

## 1. Pinned Official Image and Resource Limits

- **Image:** `docker.io/xrplf/xrpld:3.4.1`
- **Pinned manifest digest:** `sha256:457f095d1948a354119187df962b60504a4791521f8225a28ba2345c34a6b0ce`
- **Pull reference:** `docker.io/xrplf/xrpld:3.4.1@sha256:457f095d1948a354119187df962b60504a4791521f8225a28ba2345c34a6b0ce`

Every `DockerSpec` is validated before any container starts (`DockerSpec::validate`), enforcing all Publisher resource, pin, and network bounds for both the default spec and any custom spec:

| Limit | Flag / Setting | Enforced Bound |
|---|---|---|
| Exact image pin | `pinned_image == OFFICIAL_XRPLD_PINNED_REF` | Exact tag + `sha256` manifest digest match (no prefix match) |
| Node count | `ACTIVE_CONTAINER_LOCK` + cross-process `docker ps -a --filter label=merchant-league.harness=standalone` | At most **1** active container per host (`1` node, `1` table per test); refuses concurrent runs |
| CPU | `--cpus=2` | Strict `0 < cpus <= 2.0` |
| RAM & swap | `--memory=2g --memory-swap=2g` | Strict `0 < memory <= 2048m` and `memory_swap == memory` (zero swap expansion, `memory: 0` rejected) |
| Ephemeral disk | `--tmpfs /tmp/xrpld:rw,size=512m,mode=1777` | `512 MiB` default, validated `1..=4768 MiB` (`<= 5,000,000,000` bytes / decimal `5 GB`, zero host volumes). Container filesystem stays writable so `/tmp/xrpld/xrpld.cfg` writes cleanly inside tmpfs without `--read-only`. |
| Network binding | `-p 127.0.0.1::5005` | Loopback `127.0.0.1` ephemeral host port only (internal `0.0.0.0:5005` admin RPC reachable only from the container namespace; `server_info` guard still applies) |
| Config injection | `-e XRPLD_CFG` written to `/tmp/xrpld/xrpld.cfg` at start | Isolated standalone config inside tmpfs (no validator lists or external peers) |
| Lifecycle & stale sweep | Start-of-run sweep (`docker rm -f` on stopped/expired labelled containers), `--stop-timeout=5`, in-container `max_lifetime` (`300s` default, `<= 600s`) self-expiry watchdog, `teardown()`, `Drop`, and cleanup `docker rm -f` on **every** `run`/startup/timeout failure path | Stopped and removed after every run; never left running |
| Subprocess bounds | `stdin(Stdio::null())`, bounded pipe capture (`1 MiB`), and bounded post-kill drain (`250 ms`) | CLI timeouts and closed helper pipes cannot hang waiting on stdin or child threads |
| Rig exclusivity | Local smoke only when no other heavy rig job runs | Full multi-scenario suite runs in CI (`ubuntu-latest`) |

No `-a` container shares state with Testnet or Mainnet: `[node_db] type=Memory`, `[database_path] /tmp/xrpld/db`, `[ips]` / `[validators]` omitted, and `[rpc_startup]` / `[port_rpc_admin]` map only to `127.0.0.1` on the host.

---

## 2. Conflict 11 Network & Stand-Alone Isolation

`league-ledger` stays completely unmodified and refuses every `network_id != Some(1)`. `league-standalone` lives in a separate workspace crate and implements a parallel `VerifiedStandAlone` capability token that cannot be constructed without first passing both URL checks and a live `server_info` check:

1. **Loopback URL check (`require_loopback_http_url`):** accepts only `http://127.0.0.1:<port>[/]` or `http://localhost:<port>[/]` (with explicit valid port `1..=65535`; note OS hosts/DNS resolution for `localhost` is backed up by the `server_info` guard below). Rejects any remote hostname, TLS endpoint, Mainnet hostname, or URL without an explicit loopback host.
2. **`server_info` stand-alone guard (`require_standalone_info`):**
   - `info.network_id == Some(21337)` (`STANDALONE_NETWORK_ID = 21337`, set by `[network_id] 21337` in generated `xrpld.cfg`). Refuses `network_id == Some(0)` (Mainnet), `Some(1)` (Testnet), `None` (missing id / legacy Mainnet), and any other id after **at most one RPC request**.
   - `info.validation_quorum == Some(0)` (stand-alone mode has no UNL validators).
   - `info.peers.unwrap_or(0) == 0` (isolated single node with no P2P peers).
   - `info.server_state` is present and non-empty (observed as `"full"` once `xrpld -a --start` reaches stand-alone operating mode).

Every public `StandAlone` constructor (`StandAlone::connect_loopback` and `StandAlone::with_transport`) passes through this single checked path and has negative tests covering `Some(0)`, `None`, `Some(1)`, non-zero `validation_quorum`, non-zero `peers`, and non-loopback URLs.

---

## 3. Mainnet-Matched Checks for the v2 Feature Set (`xrpld 3.4.1`)

When `xrpld -a --start` generates a new genesis ledger, it enables amendments in its desired set (`DefaultYes` plus `[amendments]`). In `xrpld 3.4.1`, core v2 features such as `BatchV1_1`, `TokenEscrow`, `fixTokenEscrowV1`, `PermissionedDomains`, `PermissionedDEX`, `Credentials`, `PermissionDelegationV1_1`, `PriceOracle`, `fixPriceOracleOrder`, `DeepFreeze`, `AMMClawback`, and `fixAMMClawbackRounding` default to `DefaultNo`.

`stand_alone_xrpld_cfg` writes deterministic 64-hex SHA-512Half amendment IDs into `[amendments]` for those 12 Mainnet-enabled `DefaultNo` features and omits forbidden non-Mainnet `DefaultNo` features (`ConfidentialTransfer`, `MPTokensV2`, `DynamicMPT`) so they stay `enabled == false`. (`fixBatchV1_2` is compiled `DefaultYes` in `xrpld 3.4.1` and Testnet while `enabled == false` on Mainnet per `docs/design-v2.md` section 8; `AmendmentReport` records it explicitly, and the harness avoids relying on `fixBatchV1_2`-only rules.)

`StandAlone::verify_v2_amendments` queries `feature` RPC (`result.features`) after startup, verifies that all 12 Mainnet-matched v2 features are `enabled == true` and the 3 vetoed non-Mainnet features remain `enabled == false`, and captures the full `all_features` map (`FEATURE_MAP:` printed once per `full` CI run). Note: non-listed `3.4.1` `DefaultYes` amendments are enabled by genesis and are outside the 15-feature v2 check; they are logged in `FEATURE_MAP:` for auditability rather than asserted against Mainnet.

---

## 4. Genesis Funding and Turn Progression

- **Genesis root account:** Standard XRPL stand-alone genesis account derived locally via SHA-512Half passphrase derivation (`"masterpassphrase"` -> `secp256k1` root wallet), used **strictly** inside `league-standalone` for funding stand-alone test seats from genesis ledger `1` (`Payment` in drops).
- **League seats:** Initialized with `ed25519` keys validated through `league_ledger::client::Account::from_seed`, enforcing `lsfRequireAuth` / `DefaultRipple` / `lsfAllowTrustLineClawback` / `DomainID` setup as needed without printing seeds in `Debug` or errors.
- **Ledger progression:** Transactions run through `submit` (`submit_only`) and explicit admin `ledger_accept` RPC calls, closing the ledger on demand without waiting for consensus timers and advancing `parent_close_time` cleanly.

---

## 5. CI vs. Local Execution Policy

- **Default `cargo test --workspace --locked`:** `tests/standalone_live.rs` sees `LEAGUE_XRPLD_LIVE` unset and returns immediately with no Docker commands, keeping local unit runs and `cargo-llvm-cov` fast and deterministic.
- **Local smoke (`LEAGUE_XRPLD_LIVE=smoke`):** Starts 1 `xrplf/xrpld:3.4.1` container with the resource bounds in Section 1, verifies `network_id == 21337`, checks `verify_v2_amendments`, bootstraps a 4-seat chiral test table, executes `ledger_accept`, and tears down the container cleanly.
- **CI full harness (`LEAGUE_XRPLD_LIVE=full`):** Runs everything in smoke, prints `FEATURE_MAP:` once, runs all Seed-27 Section B item B3 conformance tests (asserting `B3.1`, `B3.2`, and `B3.3` values on both live `xrpld` and `league_core::TurnEngine`) in `.github/workflows/ci.yml`, followed by an explicit assertion that zero harness containers remain (`docker ps -aq --filter label=merchant-league.harness=standalone`).

---

## 6. Seed-27 Section B Item B3 Conformance Cases & Recorded Divergences

`crates/league-standalone/src/conformance.rs` leaves `crates/league-core` completely untouched and drives the real `league_core::TurnEngine` (and `league_core` arithmetic/cryptocondition functions) directly alongside live `xrpld` ledger mechanics, pinning all `B3.1`, `B3.2`, and `B3.3` outcomes with explicit assertions in `tests/standalone_live.rs` and unit tests:

1. **B3.1 — Offer funding vs. spend-down (`D1`):** `TurnEngine` locks `cred` / commodity inventory upfront on `OfferCreate` and refuses spending behind the open offer (`PostOffer` and `AtomicSwapProposal`); `xrpld` leaves funds liquid behind open offers until crossing, where an under-funded offer fills the remaining funded balance (`40`), deletes the seller's unfunded remainder, and leaves the buyer's unfilled `40` remainder bid resting on the book (`buyer_open_offers_after_cross == 1`).
2. **B3.2 — Partial fills, `Quality`, and `TransferRate` rounding (`D2` & `D3`):** Compares real `TurnEngine` `PostOffer` crossing (`checked_mul_units_parity` + `ceil_bps_fee`, deducting `ceil(fill * tariff_bps / 10_000)` from buyer delivery) against live `xrpld` `account_lines` reads across three cases:
   - **D2 (issuer direct export, `R0A`, `800 bps`):** `xrpld` exempts issuer-to-holder primary issuance from `TransferRate` (`0` fee; observed issuer-to-buyer line `"0->-0.333"`), whereas `TurnEngine` charges `ceil_bps_fee(333, 800) = 27` milli-units (`306` net). Matters only if a design permits a commodity issuer to trade directly.
   - **D3 (third-party headroom & balance-capped seller, `R0S` `750 bps` & `R0T` `833 bps`):** `xrpld` charges `TransferRate` upward on the seller (`1.075x` / `1.0833x`) and delivers full commodity to the buyer while headroom exists, keeping exact rational `CRED` quality vs `TurnEngine` flooring `checked_mul_units_parity` to milli-`CRED`. When a seller holds exact `1` unit (`R0T` `833 bps`), `xrpld` delivers `1 / 1.0833 = 0.9231053263177329` vs `TurnEngine` `1000 - ceil(83.3) = 916` milli-units (`0.916`); roughly `6.4` milli of that gap comes from the same seller-surcharge vs buyer-deduction rule (`1 - 0.0833 = 0.9167`), roughly `0.7` milli comes from `ceil_bps_fee`, and 16-digit `IOUAmount` mantissa precision contributes only `~1e-16`.
3. **B3.3 — 32-byte preimage escrows & close-time bounds (`D4`):** Verifies 32-byte `PREIMAGE-SHA-256` condition/fulfillment matching (`compute_preimage_condition`), `xrpld` fulfillment base-fee escalation (`>= 350` drops required vs `10`/`349` refused with `telINSUF_FEE_P` vs `0` drops in `TurnEngine`), and discrete turn bounds (`turn >= finish_after_turn` and `turn >= cancel_after_turn`) in `TurnEngine` vs `xrpld` `close_time` bounds on `FinishAfter` and `CancelAfter` (noting explicitly that the exact `>` vs `>=` boundary on `CancelAfter` is not measured at the equality boundary in this run).

**Note on upcoming `C4` (simultaneous batch clearing):** `TurnEngine` currently uses arrival-order `OfferCreate` matching (`B3.1`/`B3.2`). When Director Decision `C4` replaces arrival-order crossing with simultaneous batch clearing in a later `league-core` PR, `B3.1`, `B3.2`, and `B3.3` must be re-checked against the updated clearing rules.
