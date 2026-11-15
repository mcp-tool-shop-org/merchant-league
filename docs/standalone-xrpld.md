# Stand-Alone `xrpld` Harness (`league-standalone`)

**Status:** Approved by the Director in `docs/design-v2.md` section 9 (Conflict 11) and scoped under Publisher container limits (2026-11-15).

This document specifies the containerized local stand-alone `xrpld` harness for headless evaluation and ledger-conformance testing (`crates/league-standalone`), keeping `league-ledger` strictly bound to public XRPL Testnet (`network_id == 1`).

---

## 1. Pinned Official Image and Resource Limits

- **Image:** `docker.io/xrplf/xrpld:3.4.1`
- **Pinned manifest digest:** `sha256:457f095d1948a354119187df962b60504a4791521f8225a28ba2345c34a6b0ce`
- **Pull reference:** `docker.io/xrplf/xrpld:3.4.1@sha256:457f095d1948a354119187df962b60504a4791521f8225a28ba2345c34a6b0ce`

Every container launch passes all Publisher resource and network limits:

| Limit | Flag / Setting | Enforced Bound |
|---|---|---|
| Node count | Process-wide mutex `ACTIVE_CONTAINER_LOCK` | At most **1** active container (`1` node, `1` table per test) |
| CPU | `--cpus=2` | Max `2` vCPUs |
| RAM & swap | `--memory=2g --memory-swap=2g` | Hard `2 GiB` RAM, zero swap expansion |
| Ephemeral disk | `--tmpfs /tmp/xrpld:rw,size=512m,mode=1777` | `512 MiB` RAM-backed tmpfs (strictly below `5 GiB` cap) |
| Network binding | `-p 127.0.0.1::5005` | Loopback `127.0.0.1` ephemeral host port only |
| Config injection | `-e XRPLD_CFG` written to `/tmp/xrpld/xrpld.cfg` at start | Isolated standalone config inside tmpfs (no validator lists or external peers) |
| Lifecycle | Explicit `teardown()` and `Drop` (`docker rm -f`) | Stopped and removed after every run; never left running |
| Test timeouts | Per-command timeout + CI `timeout 300` | Every CLI call and container wait is bounded |
| Rig exclusivity | Local smoke only when no other heavy rig job runs | Full multi-scenario suite runs in CI (`ubuntu-latest`) |

No `-a` container shares state with Testnet or Mainnet: `[node_db] type=Memory`, `[database_path] /tmp/xrpld/db`, `[ips]` / `[validators]` omitted, and `[rpc_startup]` / `[port_rpc_admin]` bind only inside the container and are mapped to `127.0.0.1` on the host.

---

## 2. Conflict 11 Network & Stand-Alone Isolation

`league-ledger` stays completely unmodified and refuses every `network_id != Some(1)`. `league-standalone` lives in a separate workspace crate and implements a parallel `VerifiedStandAlone` capability token that cannot be constructed without first passing both URL checks and a live `server_info` check:

1. **Loopback URL check (`require_loopback_http_url`):** accepts only `http://127.0.0.1:<port>[/]` or `http://localhost:<port>[/]` (with explicit valid port `1..=65535`). Rejects any remote hostname, TLS endpoint, Mainnet hostname, or URL without an explicit loopback host.
2. **`server_info` stand-alone guard (`require_standalone_info`):**
   - `info.network_id == Some(21337)` (`STANDALONE_NETWORK_ID = 21337`, set by `[network_id] 21337` in generated `xrpld.cfg`). Refuses `network_id == Some(0)` (Mainnet), `Some(1)` (Testnet), `None` (missing id / legacy Mainnet), and any other id after **at most one RPC request**.
   - `info.validation_quorum == Some(0)` (stand-alone mode has no UNL validators).
   - `info.peers.unwrap_or(0) == 0` (isolated single node with no P2P peers).
   - `info.server_state` is present and non-empty (observed as `"full"` once `xrpld -a --start` reaches stand-alone operating mode).

Every public `StandAlone` constructor (`StandAlone::connect_loopback` and `StandAlone::with_transport`) passes through this single checked path and has negative tests covering `Some(0)`, `None`, `Some(1)`, non-zero `validation_quorum`, non-zero `peers`, and non-loopback URLs.

---

## 3. Amendment Parity with Mainnet (`xrpld 3.4.1`)

When `xrpld -a --start` generates a new genesis ledger, it enables **only** amendments in its desired set (`DefaultYes` plus `[amendments]`) — it does **not** automatically enable every `Supported::Yes` amendment. In `xrpld 3.4.1`, core v2 features such as `BatchV1_1`, `TokenEscrow`, `fixTokenEscrowV1`, `PermissionedDomains`, `PermissionedDEX`, `Credentials`, `PermissionDelegationV1_1`, `PriceOracle`, `fixPriceOracleOrder`, `DeepFreeze`, `AMMClawback`, and `fixAMMClawbackRounding` default to `DefaultNo`.

`stand_alone_xrpld_cfg` writes deterministic 64-hex SHA-512Half amendment IDs into `[amendments]` for those 12 Mainnet-enabled `DefaultNo` features and omits forbidden non-Mainnet `DefaultNo` features (`ConfidentialTransfer`, `MPTokensV2`, `DynamicMPT`) so they stay `enabled == false`. (`fixBatchV1_2` is compiled `DefaultYes` in `xrpld 3.4.1` and Testnet `s.altnet.rippletest.net` while `enabled == false` on Mainnet per `docs/design-v2.md` section 8; `AmendmentReport` records it explicitly, and the harness avoids relying on `fixBatchV1_2`-only rules.) `StandAlone::verify_v2_amendments` queries `feature` RPC (`result.features`) after startup and verifies that all 12 Mainnet-matched features are `enabled == true` and forbidden non-Mainnet features remain `enabled == false`.

---

## 4. Genesis Funding and Turn Progression

- **Genesis root account:** Standard XRPL stand-alone genesis account derived locally via SHA-512Half passphrase derivation (`"masterpassphrase"` -> `secp256k1` root wallet), used **strictly** inside `league-standalone` for funding stand-alone test seats from genesis ledger `1` (`Payment` in drops).
- **League seats:** Initialized with `ed25519` keys validated through `league_ledger::client::Account::from_seed`, enforcing `lsfRequireAuth` / `DefaultRipple` / `lsfAllowTrustLineClawback` / `DomainID` setup as needed without printing seeds in `Debug` or errors.
- **Ledger progression:** Transactions run through `submit` (`submit_only`) and explicit admin `ledger_accept` RPC calls, closing the ledger on demand without waiting for consensus timers and advancing `parent_close_time` cleanly.

---

## 5. CI vs. Local Execution Policy

- **Default `cargo test --workspace --locked`:** `tests/standalone_live.rs` sees `LEAGUE_XRPLD_LIVE` unset and returns immediately with no Docker commands, keeping local unit runs and `cargo-llvm-cov` fast and deterministic.
- **Local smoke (`LEAGUE_XRPLD_LIVE=smoke`):** Starts 1 `xrplf/xrpld:3.4.1` container with the resource bounds in Section 1, verifies `network_id == 21337`, checks `verify_v2_amendments`, bootstraps a 4-seat chiral test table, executes `ledger_accept`, and tears down the container cleanly.
- **CI full harness (`LEAGUE_XRPLD_LIVE=full`):** Runs everything in smoke plus all Seed-27 Section B item B3 conformance tests (offer funding/spend-down, partial-fill `Quality` + `TransferRate` rounding, and 32-byte `PREIMAGE-SHA-256` escrow fees and close-time enforcement) in `.github/workflows/ci.yml`, followed by an explicit assertion that zero harness containers remain (`docker ps -aq --filter label=merchant-league.harness=standalone`).

---

## 6. Seed-27 Section B Item B3 Conformance Cases

`crates/league-standalone/src/conformance.rs` leaves `league-core` completely untouched and compares `league-core` (`ab06dd39`) rules directly against live `xrpld` ledger mechanics on:

1. **B3.1 — Offer funding vs. spend-down:** `league-core` locks `locked_cred` / commodity inventory upfront on `OfferCreate` and refuses spending behind the open offer; `xrpld` leaves funds liquid behind open offers until crossing, where an under-funded offer partially fills or auto-deletes as unfunded (`tecUNFUNDED_OFFER` / directory removal).
2. **B3.2 — Partial fills, `Quality`, and `TransferRate` rounding:** Compares `league-core` integer `mul_units_parity` + `ceil_bps_fee` (`net_commodity = fill - ceil(fill * tariff_bps / 10_000)`) against `xrpld` issuer `TransferRate` (`delivered * (1 + bps / 10_000)` charged to sender, `16`-significant-digit `IOUAmount` mantissa/exponent normalization, and rational `Quality` orderbook math), reporting exact per-case differences.
3. **B3.3 — 32-byte preimage escrows & close-time bounds:** Verifies 32-byte `PREIMAGE-SHA-256` condition/fulfillment DER encoding, `xrpld` fulfillment base-fee escalation (`350` drops floor vs `10` drops standard tx vs `0` in `league-core`), and discrete turn thresholds vs `xrpld` `parent_close_time` (`Ripple` epoch) enforcement of `FinishAfter` and `CancelAfter`.
