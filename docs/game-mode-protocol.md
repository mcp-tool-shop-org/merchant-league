# Game Mode Protocol Specification (`TableMode::Game`)

**Status:** Normative protocol specification for Merchant League `TableMode::Game` on XRPL Testnet (`network_id = 1`, `TESTNET_JSON_RPC = "https://s.altnet.rippletest.net:51234"`, `TESTNET_WEBSOCKET = "wss://s.altnet.rippletest.net:51233"`).
**Scope:** Docs-only design specification separating live Game Mode ledger clearing, ledger-validated cutoffs, `MemoData` firewalling, authenticated off-ledger whispers, Steward XRP reserve management, and Two-Track benchmark rules from the pure-state `TableMode::Eval` and local `TableMode::Club` harnesses.

`TableMode::Eval` (fixed-scaffold silent/structured benchmark) and `TableMode::Club` (`docs/club-mode-design.md`) remain isolated from live XRPL submission and from Game Mode whisper extensions. On-chain settlement checks run against `require_testnet` (`TESTNET_NETWORK_ID = 1`) rather than standalone `xrpld` (`STANDALONE_NETWORK_ID = 21_337`, `docs/standalone-xrpld.md`).

---

## 1. Steward-Settled Clearing (`BatchV1_1` and `Clawback`, Resolving `B3.1`–`B3.3`)

In standalone conformance testing (`docs/standalone-xrpld.md`), native XRPL `OfferCreate` + issuer `TransferRate` exhibits three protocol divergences (`B3.1` spend-behind-offer on IOUs, `B3.2` seller-side vs. buyer-side surcharge on primary issuance, and `B3.3` XRPL IOU mantissa rounding vs. integer milli-units). Rather than emulating native `OfferCreate` crossing on ledger, `TableMode::Game` resolves `B3.1`–`B3.3` by making **Steward custody + integer Sweep accounting** the sole clearing mechanism:

| Standalone Divergence | Protocol Resolution in `TableMode::Game` |
| --- | --- |
| **`B3.1` (spend-behind-offer)** | Phase-1 `MarketAction::PostOffer` immediately transfers the locked IOU amount (`UNIT_SCALE = 1_000`) into Steward custody via issuer `Clawback` (`asfAllowTrustLineClawback` enabled on Steward). Seated accounts cannot spend locked IOUs behind an open offer because their spendable trust-line balance decreases immediately. Pre-lock `CancelOffer` refunds the exact integer amount via Steward `Payment`. Crossing never submits raw seat-to-seat `OfferCreate` transactions. |
| **`B3.2` (`TransferRate` surcharge)** | `AccountSet.TransferRate` stays unset (`0`) on all Steward-issued currencies (`AssetId::{Cred, Raw(u8), Refined(u8)}`). Clearinghouse and domain tariffs (`0..=MAX_TARIFF_BPS`, `MAX_TARIFF_BPS = 3_500`, `BPS_SCALE = 10_000`) are computed strictly in deterministic integer milli-units during Sweep Steps 0–8 and settled as exact Steward `Clawback` / `Payment` transfers. |
| **`B3.3` (IOU mantissa rounding)** | All clearing deltas, tariff cuts, bond deliveries, and Check/Escrow settlements use `TurnEngine` integer arithmetic (`i64` milli-units). XRPL IOU amounts transmitted by Steward are formatted to exact `.3` decimal strings derived from integer milli-units, keeping ledger balances bit-identical to engine state. |

### `BatchV1_1` (`XLS-56`) Submission and Mandatory Inner Metadata Verification

1. **Atomic Sweeps (`tfAllOrNothing`):** Steward wraps multi-leg Sweep transfers (asset delivery, tariff collection, wash-freeze bond redistribution) inside `Batch` (`XLS-56`, transaction type `71` in `vendor/xrpl-rust/definitions.json`) with `tfAllOrNothing` and `2..=8` inner `RawTransactions` per outer `Batch`. Because `vendor/xrpl-rust` exposes `Batch` (`ParentBatchID`, `RawTransactions`, `BatchSigners`) and `DelegateSet` (type `64`) in `definitions.json` without high-level model structs, `league-steward` encodes inner and outer transactions via `xrpl::core::binarycodec::encode`.
2. **Batch Ordering Beyond 8 Inner Legs:** If a deterministic Sweep substep requires more than 8 inner transfers, Steward splits legs into fixed Sweep-order `BatchV1_1` chunks (`<= 8` legs each) after verifying every seat's live `account_lines` balance can satisfy all planned `Clawback` debits before any chunk is signed.
3. **Mandatory Inner Transaction Checks (No Outer-Only Trust):** An outer `Batch` returns `meta.TransactionResult = "tesSUCCESS"` even when an inner transaction fails with `tec*` and rolls back the entire `tfAllOrNothing` group. Therefore, `league-steward` MUST:
   - Pre-compute every inner transaction hash before submitting the outer `Batch`.
   - Query validated `tx` on the outer `Batch` AND on **every inner transaction hash**.
   - Assert for every inner transaction: `validated == true`, `ParentBatchID == outer_batch_hash`, and `meta.TransactionResult == "tesSUCCESS"`.
   - Assert `account_lines` balance parity across Steward and all 6 seats against expected post-sweep `TurnEngine` integers before advancing the turn state or initiating any later `Clawback`.
4. **Escrows and Checks:** `CreateEscrow`, `CancelExpiredEscrow`, `CreateCheck`, and `CancelCheck` map directly to native XRPL `EscrowCreate` / `EscrowFinish` / `EscrowCancel` and `CheckCreate` / `CheckCash` / `CheckCancel`. Preimage fulfillment (`condition_hex`, `fulfillment_hex`) submits at least `escrow_finish_min_fee_drops(base_fee_drops)` (`base_fee_drops * 35`; `ESCROW_FINISH_DEFAULT_FEE_DROPS = "500"` on Testnet vs. `PREIMAGE_32_FINISH_FEE_DROPS = "350"` in `league-standalone`).

---

## 2. Validated `close_time` Cutoffs, VRF Table Binding (#50), and Post-Commit `drand` Check (#51)

### 2.1 Ledger-Validated Turn Windows and `cancel_lock`

Host wall-clock time MUST NOT trigger `TurnEngine::begin_cancel_lock` or close turn phases in `TableMode::Game`. Phase boundaries bind exclusively to **validated XRPL ledger headers** (`close_time` converted from Ripple epoch to UNIX seconds + monotonic `ledger_index`, guarded by `league_season::ledger_went_backwards`):

- **Open Window (`[ledger_open, ledger_lock)`):** All `MarketAction` submissions (`PostOffer`, `CancelOffer`, `CreateCheck`, `CancelCheck`, `CreateEscrow`, `CancelExpiredEscrow`, `RecordIntentPledge`, `FlagWashFreeze`, `StandForOffice`, `TownImport`) and `SealedOrderBundle` commits are eligible.
- **Cancel-Lock Cutoff (`DEFAULT_CANCEL_LOCK_BPS = 2_500`):** Once a validated ledger reaches `close_time >= T_lock`, Steward transitions to cancel-lock: Phase-1 `PostOffer` locks remain non-cancelable (`CancelOffer` rejected), while sealed order commitments continue until `T_commit_close`.
- **Commit Cutoff (`T_commit_close`):** Commits included in a validated ledger with `close_time <= T_commit_close` belong to turn `t`. Any commit whose validated `close_time > T_commit_close` is rejected for turn `t` (logged and dropped at ingress if already expired).
- **Liveness and Termination:** `IDLE_MISS_TAKEOVER_TURNS = 3`; closing hazard rolls begin at `CLOSING_ROLL_START_TURN = 24` with `DEFAULT_CLOSING_HAZARD_BPS = [3_500; 8]` up to `HARD_TURN_CAP = 32`.

### 2.2 Post-Commit `drand quicknet` Target and Replay Check (#51)

`league-seat-types` defines `DRAND_QUICKNET_GENESIS_UNIX = 1_692_803_367`, `DRAND_QUICKNET_PERIOD_SECS = 3`, and:

```text
drand_quicknet_round_for_close(ts) = floor((ts - 1_692_803_367) / 3) + 1
t_round(r) = 1_692_803_367 + 3 * (r - 1) <= ts
```

Because `floor` rounds **down**, pointing `DrandSchedule::close_unix_for_turn(t)` directly at `T_commit_close(t)` picks a beacon whose `t_round(r) <= T_commit_close(t)` is already published before the commit deadline closes.

**Normative Offset Requirement (#51):**

1. `DrandSchedule` MUST set `T_beacon(t) >= T_commit_close(t) + DRAND_QUICKNET_PERIOD_SECS` (minimum `+3` seconds, i.e., strictly after `T_commit_close(t)`), so the scheduled round satisfies:
   `t_round(drand_round) > T_commit_close(t)`.
2. `LoggedTurn` records `commit_deadline_unix: Option<u64>` alongside `cancel_lock_at`, `turn_beacon`, `drand_round`, `drand_signature`, and `eval_vrf_proof`.
3. Both live Sweep assembly and deterministic `replay_season` MUST fail closed whenever `t_round(drand_round) <= commit_deadline_unix` or BLS12-381 `drand` verification rejects.

### 2.3 Table-ID VRF and HKDF Binding Across Duplicate and Live Tables (#50)

To prevent mirrored duplicate tables (`EvalHost::fork_duplicate_from_turn_0`) or adjacent live Game tables from sharing VRF inputs or derived keys:

1. `eval_vrf_alpha(table_id, turn)` binds every turn VRF alpha input to the concrete `table_id` (`u64` / `EvalSaveSnapshot.game_id`), and `fork_duplicate_from_turn_0` assigns distinct domain-separated child table IDs rather than reusing the parent `table_id`.
2. Both `TableMode::Game` and `TableMode::Eval` HKDF key expansion mix the protocol tag `"merchant-league/v2"` with `(table_id, turn)` before deriving per-turn shock or tiebreak subkeys, isolating randomness streams across every live and duplicated table.

---

## 3. `MemoData` Ingress Firewall and Sender Allowlist

XRPL `Memos` on Testnet are publicly writable by any funded account (`<= 1 KB` total per transaction). Without strict filtering, external accounts or compromised peers could inject prompt text or malformed bundles into seat state.

**Mandatory Ingress Rules (Executed Before `TurnEngine` or `SeatObservation`):**

1. **Seated Account Allowlist:** The ledger listener tracks the exact set of 6 registered seated XRPL classic accounts for the active `table_id` (plus Steward's own address). Any transaction whose outer `Account` is not in the active allowlist is dropped immediately before parsing `Memos`.
2. **Strict 64-Hex SHA-256 `MemoData` Grammar:** For Phase-1 commits in `TableMode::Game`:
   - `MemoType` (decoded from hex) MUST equal `"ml.commit.v2"`.
   - `MemoData` (on-ledger hex string representing 32 raw bytes, or validated uppercase/lowercase 64-hex digest canonicalized to lowercase ASCII 64-hex) MUST match `^[0-9a-f]{64}$` after canonical validation.
   - Any free-text UTF-8 `MemoData`, wrong byte length (`!= 32` decoded bytes / `!= 64` ASCII hex chars), extra unsolicited `Memo` entries beyond the table commit schema, or invalid `MemoFormat` triggers immediate ingress drop (`IngressDropReason`).
3. **Zero Prompt Leakage:** Dropped memo payloads MUST NOT enter `TurnEngine` state, `SeatObservation`, error strings forwarded to seats, or archived negotiation transcripts (structured telemetry records only `(tx_hash, sender, drop_reason)`).

---

## 4. Authenticated Off-Ledger Whisper Envelope, Typed `IntentHeader`, and Tick-Barrier Release

Private negotiation text never touches the XRPL ledger (`docs/design-v2.md` §Privacy). In `TableMode::Game`, off-ledger whispers pass through the Steward/Relay firewall under three hard invariants:

### 4.1 Signed Whisper Envelope

Each off-ledger whisper from `sender_seat` to `recipient_seat` travels as a signed JSON envelope over per-seat Ed25519 session keys registered at table start:

- **Authenticated Fields:** `(table_id, turn, sub_round, sender_seat, recipient_seat, envelope_seq, header, untrusted_body, signature)`.
- **Anti-Replay:** Relay verifies the Ed25519 signature against `sender_seat`'s session public key, checks `table_id`/`turn`/`sub_round`, and enforces strictly monotonic `envelope_seq` per `(sender_seat, recipient_seat)`. Unauthenticated, duplicated, or out-of-window packets drop silently before queuing.

### 4.2 Closed Typed `IntentHeader` Enum (5 Variants) and Body Isolation

Every whisper pairs a strictly typed `IntentHeader` (nesting cap `MAX_INTENT_HEADER_DEPTH = 8`) with a bounded body (`MAX_WHISPER_WORDS = 120`, `MAX_WHISPER_CHARS = 1_800`) wrapped in `UntrustedWhisper` and escaped via `json_escape_untrusted_str` (mirroring `crates/league-relay/src/wording.rs`).

`IntentHeader` is a **closed 5-variant enum** aligned 1:1 with the UI deal composer (arbitrary free-form JSON maps are rejected, and `EscrowDeal` covers preimage oracle/deliverable terms cleanly without a separate oracle-hint variant):

1. `SpotOffer` — bilateral spot proposal (`OfferSide::{Buy, Sell}`, `AssetId`, whole-unit qty, Cred unit price).
2. `AtomicSwap` — linked multi-asset exchange legs (`AssetId` + integer quantities).
3. `EscrowDeal` — conditional escrow terms (`BondSettlementMode::{SellerPerformanceBond, BuyerDvpPayment}` or SHA-256 preimage `condition_hex`).
4. `CreditCheck` — deferred Cred check proposal (integer Cred limit and expiry turn).
5. `DiplomaticPledge` — alignment/voting/preimage intent matching `IntentTarget::{VoteForSeat, PreserveCartelMembership, RevealPreimage}`.

### 4.3 Discrete Sub-Round Tick-Barrier Release and Track-1 Freeze

- **Tick-Barrier Buffering:** Relay holds all validated whispers submitted during communication sub-round `k` until sub-round `k` closes on the discrete sub-round tick barrier, then delivers the batch simultaneously to recipient seats in canonical `sender_seat` order. This eliminates asynchronous race-to-interrupt timing side-channels and high-rate message flooding.
- **Track-1 Freeze:** `TableMode::Eval` remains locked on `ACTIVE_WORDING_ARM = "v2_neutral_2026_11_15_r6"` (`talk_rules_v2`) and its fixed harness. `IntentHeader` and off-ledger session envelopes apply only to `TableMode::Game` and opt-in `TableMode::Club` tables.

---

## 5. Steward XRP Reserve Pool and Two-Track Benchmark Policy

### 5.1 Steward XRP Reserve and Faucet Pool Sizing

- **Table Ledger Footprint:** Each `TableMode::Game` table provisions 1 Steward account, 6 seat accounts, up to 42 trust lines (7 assets across 6 seats), and transient Escrows/Checks (`~90` active ledger objects worst-case; modeled baseline `~36.8` XRP total base + owner reserves across the table, compared against `1,000` Testnet XRP per account from `TESTNET_FAUCET = "https://faucet.altnet.rippletest.net/accounts"`).
- **Custody Reserve Advantage:** Because Phase-1 `PostOffer` locks use Steward `Clawback` instead of per-seat `OfferCreate` objects, seats do not accumulate open-offer XRP owner reserves during heavy book quoting.
- **Steward Fee & Reserve Pool:** Steward funds a dedicated Testnet XRP pool sized to cover all table base/owner reserves, `BatchV1_1` outer+inner fees, and `ESCROW_FINISH_DEFAULT_FEE_DROPS = "500"` preimage finishes, automatically replenishing from `TESTNET_FAUCET` between tables when pool headroom falls below threshold (`table_reserve_estimate * safety_factor`).
- **Complete Seat Isolation:** Raw Testnet XRP balances, drop fees (`350`/`500` drops), owner reserves, and faucet top-ups never enter `SeatObservation` or LLM prompts; seats act solely in `Cred`, `Raw`, and `Refined` milli-units and receive verified `[Ledger #...]` links.

### 5.2 Two-Track Benchmark Policy

Evaluations stay strictly separated across two non-pooled tracks:

| Track | Mode | Protocol & Scaffold Policy |
| --- | --- | --- |
| **Track 1: Fixed-Scaffold Model Benchmark** | `TableMode::Eval` | Locked `setup.json`, silent-market baseline + `ACTIVE_WORDING_ARM` (`talk_rules_v2`), `4x4` Latin square `SeedBlockManifest`, uniform seat prompt/tool wrapper. Measures intrinsic model strategic/negotiation capability without custom external tooling. |
| **Track 2: Open Syndicate League** | `TableMode::Game` | Live XRPL Testnet clearing (Sections 1–4), verified `BatchV1_1` + `Clawback` settlement, authenticated whispers (`IntentHeader`), custom agent scaffolds/solvers permitted per entrant. Measures end-to-end autonomous agent competitiveness under identical on-ledger enforcement. |

Results, Elo/AIVAT skill estimates, and win rates from Track 1 and Track 2 MUST NOT be pooled on shared leaderboards.
