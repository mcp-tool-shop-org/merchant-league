# Evaluation Checkpoint Verification Before Resumption

This design and implementation specification documents how the Evaluation harness verifies a saved pre-act checkpoint pair before forking counterfactual branches or running scoring rollouts, and states the counterfactual rollout randomness scope across scenarios. No game rules or play-wire messages change. All verification designs follow the principles in an internal research report (`§1`–`§7`).

---

## 1. Current State and Verification Gaps (`§1`, `§2`, `§4`, `§6`)

At `885929c4`, an Evaluation Mode pre-act checkpoint consists of two JSON files written atomically by `EvalHost::write_pre_act_checkpoint` (`crates/league-core/src/eval/host.rs:1094-1108`) via `to_save_and_seal_bytes` (`:911-918`) and `write_atomic_bytes` (`:1567-1613`):

- `checkpoints/day-DD-pre-act.save.json`: the spectator-safe `EvalSaveSnapshot` (`crates/league-core/src/eval/host.rs:54-121`), carrying `format_version`, `layout_seed`, `table_id`, `turn`, `phase`, public holdings/contracts, `eval_vrf_pubkey`, and `public_receipts`.
- `checkpoints/day-DD-pre-act.seal`: the private `EvalSealPayload` (`crates/league-core/src/eval/host.rs:167-188`), carrying `format_version`, `game_id`, optional `pre_act_checkpoint_sha256` (populated only on `day-DD-act-recorded.seal`, `None` on pre-act seals via `to_seal_bytes` at `:920-922`), raw `layout_seed`, all 32 `vrf_proofs`, the full `TurnEngine`, `sealed_mandate_deal`, `mandate_commits`, `mandate_reveals`, `logged_turns`, `bot_tie_seed`, `live_state_digests`, and pre-derived `turn_salts`.

Three gaps leave resumed rollouts trusting unverified state:

1. **Reconstruction reads only the seal:** `EvalHost::fork_from_checkpoint` (`crates/league-core/src/eval/host.rs:1195-1213`) invokes `EvalHost::from_save_and_seal_bytes` (`:955-1018`), checks `ensure_pre_act_state` (`:1294-1317`), and records `compute_pre_act_pair_sha256_hex` (`:1382-1391`). Inside `from_save_and_seal_bytes`, the entire `EvalHost` is constructed from `EvalSealPayload` (`:992-1010`); `.save.json` is verified only for `format_version == EVAL_SAVE_FORMAT_VERSION` (`:963-981`), `layout_seed` parity with the seal (`:984-990`), and equality against `host.public_save_snapshot()?` (`:1011-1016`). Because `EvalSaveSnapshot` omits `game_id`, `bot_tie_seed`, `sealed_mandate_deal`, `vrf_proofs`, `turn_salts`, `logged_turns`, and `live_state_digests`, a `.save.json` from one game paired with a `.seal` from another run sharing `layout_seed` and matching public snapshot state (such as a Turn-1 D2 duplicate from `fork_duplicate_from_turn_0` at `:1217-1238`) loads cleanly.
2. **Shape validation without causal replay or tip-digest checks:** `validate_eval_seal_bytes` (`crates/league-core/src/eval/host.rs:1393-1528`) parses the seal, validates `TableSpec`, checks all 32 `vrf_proofs`, verifies `sealed_mandate_deal` and seed independence, and checks vector lengths (`seal.live_state_digests.len() == seal.logged_turns.len()` at `:1495-1501`). It never verifies `seal.logged_turns.len() == usize::from(seal.engine.state().turn - 1)`, never compares `seal.engine.state_digest()` (`crates/league-core/src/engine.rs:511-519`) against `seal.live_state_digests`, and never replays `logged_turns` from Turn 1. Full replay currently happens only at season end inside `EvalHost::finalize_eval_season` (`crates/league-core/src/eval/host.rs:783-840`, calling `replay_eval_season` and `replay_season_with_state_predicates` at `:811-812`).
3. **Permissive JSON decoding and repeated worker re-reads:** Neither `EvalSaveSnapshot` nor `EvalSealPayload` checks canonical byte encoding or rejects unknown or duplicate JSON keys. Meanwhile, `league-scorer` re-reads and re-parses the checkpoint pair from disk before every single `(candidate, scenario)` rollout (`crates/league-scorer/src/rollout.rs:181-182, 347-366`), exposing a verification-to-fork window (`docs/decision-scorer.md:1236`) and multiplying disk and parse cost across `18,816` to `94,080` rollouts (`docs/decision-scorer.md:904-906`).

### Module Boundary in `league-core`

To keep `crates/league-core/src/eval/mod.rs` and `crates/league-core/src/lib.rs` unchanged while separating verification code cleanly from host orchestration, implementation should live in a new host module `crates/league-core/src/eval/checkpoint_verify.rs` mounted directly from `crates/league-core/src/eval/host.rs` (`#[path = "checkpoint_verify.rs"] pub mod checkpoint_verify;`). `EvalHost` exposes host-boundary verification constructors and digest checks in `eval/host.rs`, while `crates/league-core/tests/raw_api_backdoor_guard.rs` is updated to treat `eval/checkpoint_verify.rs` as a host verification file alongside `eval/host.rs` (`:7678-7682`) and list new `EvalHost` methods in `ALLOWED_EVAL_HOST_PUB_FNS` (`:1386-1430`). Where `league-eval` invokes preflight verification (subject to `ALLOWED_LEAGUE_EVAL_CORE_ITEMS` at `raw_api_backdoor_guard.rs:148-172` and `check_league_core_use_suffix_for` at `:309-319`), `EvalHost` methods return either `CheckpointVerifyError` for `league-scorer` and `league-core` callers or `EngineError` carrying the uppercase `CheckpointVerifyError::code()` prefix for `league-eval`, avoiding any modification to `eval/mod.rs` or `lib.rs`.

At Gate 1 (producer write time, internal research report `§5`), `EvalHost::to_save_and_seal_bytes` enforces a 40-lowercase-hex `engine_commit` on the host, asserts `turn > 1 => self.engine.state_digest()? == self.live_state_digests[usize::from(turn - 2)]`, serializes v2 save and seal bytes, and executes `checkpoint_verify::verify_written_pair_round_trip(&save_bytes, &seal_bytes)` (canonical round-trip byte equality, duplicate-key rejection, unknown-field rejection, and re-load equality) before `write_atomic_bytes` persists temporary files to disk.

---

## 2. Gate 2: Study-Preflight Causal Verification and Verified Manifest (`§2`, `§5`)

Following an internal research report (`§2` and `§5`), causal verification runs **once per distinct checkpoint** synchronously at study preflight inside the exact compiled engine revision used by scoring workers, with zero model calls.

### Where Gate 2 Runs

A preflight subcommand in `crates/league-eval/src/cli.rs` (`verify_checkpoints.rs`):

```text
league-eval verify-checkpoints --checkpoints <dir> --out <verified_checkpoints.json> [--blind <dir>]
```

It runs once after `league-eval build-blind-inputs --out-checkpoints <dir>` exports content-addressed `<64-hex>.save.json` and `<64-hex>.seal` pairs (or clean `<stem>.{save.json,seal}` pairs), and before `league-eval freeze` or any frozen `league-scorer score` run. There is **no `--allow-legacy-v1` flag**; unknown flags fail with `INVALID_USAGE`. Every checkpoint pair is verified using `LEAGUE_EVAL_GIT_SHA` (`crates/league-eval/build.rs`) resolved via `resolve_crate_git_sha_from` (`crates/league-eval/src/serving.rs`), which fails closed with `INVALID_CRATE_GIT_SHA` when the build SHA is empty, `"unknown"`, or non-40-hex (as occurs when `git` is absent at compile time and `build.rs` falls back to empty). When `--blind <dir>` is passed, `verify-checkpoints` also verifies that every blind request's `(pre_act_pair_sha256, pre_act_save_sha256, pre_act_seal_sha256)` is covered by the manifest.

### What Gate 2 Checks per Checkpoint

`EvalHost::verify_checkpoint_causal_raw` (delegating to `crates/league-core/src/eval/checkpoint_verify.rs::verify_checkpoint_causal`) executes five checks on `(save_bytes, seal_bytes)` in memory:

1. **Single-read parse, strict canonical decode, outer digests, and shape bounds:**
   - Compute `pre_act_save_sha256`, `pre_act_seal_sha256`, and `pre_act_pair_sha256` (`compute_pre_act_pair_sha256_hex`).
   - Run verifier-side strict JSON checks (syntax/EOF, duplicate object keys at any depth, unknown fields with `skip_serializing_if` empty-default checking, and canonical round-trip byte equality) before accepting `EvalSaveSnapshot` and `EvalSealPayload`.
   - Enforce `TableMode::Eval`, pre-act phase (`ensure_pre_act_state`), `turn >= 1 && turn <= HARD_TURN_CAP`, `logged_turns.len() == usize::from(turn - 1)`, and `live_state_digests.len() == usize::from(turn - 1)` inside Gate 2 (`decode_strict_checkpoint_pair` / `verify_checkpoint_causal` in `crates/league-core/src/eval/checkpoint_verify.rs`). Separately, `seat_count == 4` (`EXPECTED_EVAL_SEAT_COUNT`) is checked in the `league-eval verify-checkpoints` caller (`crates/league-eval/src/verify_checkpoints.rs:413`) and at Gate 3 manifest/worker validation (`crates/league-scorer/src/types.rs:1080-1085`, `crates/league-scorer/src/rollout.rs:252-256, 441-445`).
2. **Header, pair binding, and provenance format validation (`engine_commit` NOT equality-gated):**
   - Refuse `v1` or any non-`v2` format version with `INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION` (`IncompatibleEngineOrSchemaVersion`). Verify `save.format_version == EVAL_SAVE_FORMAT_VERSION` (`2`), `seal.format_version == EVAL_SEAL_FORMAT_VERSION` (`2`), and `seal.save_format_version == EVAL_SAVE_FORMAT_VERSION` (`2`).
   - Verify pair binding (`save.game_id == seal.game_id`, `seal.turn == save.turn`, `seal.save_sha256 == Some(SHA-256("league-core/eval-save/v2|" || save_bytes))`).
   - Verify `seal.engine_commit` is `Some` and a valid 40-lowercase-hex commit SHA (`IncompatibleEngineOrSchemaVersion` if missing/empty/malformed). **Gate 2 does NOT compare `seal.engine_commit` for equality with the current `league-eval` build commit:** checkpoints are written during paid Evaluation runs and verified/scored later, so `engine_commit` is recorded provenance copied into `verified_checkpoints.json` and surfaced in `joined_decisions.json` / `per_arm_summary.json`, while compatibility is decided solely by full replay in the current engine plus the `v2` format versions.
3. **Deterministic replay from Turn 0 through `turn - 1`:**
   - Initialize a fresh `replayed_engine = TurnEngine::new(spec.clone())?` (`crates/league-core/src/engine.rs:348`) and apply all `n` Turn-1 `mandate_commits` matching `EvalHost::new` (`crates/league-core/src/eval/host.rs:300-306`).
   - Initialize the temporal cumulative chain digest (internal research report `§2`):
     `C_0 = SHA256("league-core/eval-turn-chain-init/v1|" || replayed_engine.state_digest()?)`.
   - For each historical turn `k` in `1..=(turn - 1)` (`idx = usize::from(k - 1)`):
     - Confirm `logged.turn == k`, `logged.drand_round.is_none()`, `logged.drand_signature.is_none()`, and `logged.eval_vrf_proof.as_deref() == Some(loaded_host.vrf_proofs[idx].as_slice())`.
     - Verify `verify_eval_vrf_beacon` (`crates/league-core/src/commit.rs:957-972`) and `draw_shocks_for_turn` (`:585-625`) against `logged.drawn_shocks` (matching `replay_season_inner` at `crates/league-core/src/replay.rs:1302-1377`).
     - Replay `logged.market_actions`, `logged.cancel_lock_at`, `logged.commitments`, and `logged.reveals` (`crates/league-core/src/replay.rs:1383-1430`), then call `replayed_engine.sweep_turn_with_eval_vrf(vrf_proof)` (`crates/league-core/src/engine.rs:1245-1250`).
     - Assert `receipt.post_state_digest == loaded_host.live_state_digests[idx]`; divergence raises `CheckpointVerifyError::ReplayDigestMismatch { turn: k, .. }`.
     - Advance the cumulative chain digest:
       `C_k = SHA256("league-core/eval-turn-chain/v1|" || C_{k - 1} || (turn_json.len() as u64).to_le_bytes() || turn_json || receipt.post_state_digest)`
       using compact serialized `LoggedTurn` bytes (`serde_json::to_vec(logged)`). In production, `verify_checkpoint_causal` computes and records `turn_chain_digest` (`C_{turn - 1}`) into `verified_checkpoints.json` without comparing it against an external reference; `verify_checkpoint_causal_against_chain` (`ChainHashMismatch`) is a library helper called only by tests.
4. **Tip state equivalence at `turn`:**
   - When `turn > 1`, assert `loaded_host.engine.state_digest()? == loaded_host.live_state_digests[usize::from(turn - 2)]`.
   - Assert `replayed_engine.state_digest()? == loaded_host.engine.state_digest()?` and `serde_json::to_vec(&replayed_engine)? == serde_json::to_vec(&loaded_host.engine)?` (verifying both `EngineState` and `TurnEngine::public_receipts`, `crates/league-core/src/engine.rs:339-344`).
   - Assert `loaded_host.public_save_snapshot()? == loaded_save`. Any tip divergence raises `CheckpointVerifyError::TipStateDigestMismatch`.
5. **Deterministic 1-turn probe step on both instances:**
   - Construct `replayed_host` from `replayed_engine` paired with the seal's validated `game_id`, `layout_seed`, `bot_tie_seed`, `vrf_proofs`, `sealed_mandate_deal`, `mandate_commits`, `mandate_reveals`, `logged_turns`, `live_state_digests`, and `turn_salts`.
   - Clone both `replayed_host` and `loaded_host`. On both copies, compute identical deterministic `ClubBot` submissions (`ClubBot::new(bot_tie_seed.wrapping_add(u64::from(s))).decide_full_turn(...)`, `crates/league-core/src/bot.rs:102-114`), call `apply_act_submissions` (`crates/league-core/src/eval/host.rs:631-734`), and call `sweep_turn` (`:739-778`).
   - Assert identical `PublicSweepReceipt`, identical post-probe `engine.state_digest()?`, and identical serialized `TurnEngine` bytes. Any mismatch raises `CheckpointVerifyError::ProbeStepDivergence { turn, .. }`.

### Verified Manifest Written by Gate 2

`league-eval verify-checkpoints` writes a single manifest file at `<study_dir>/verified_checkpoints.json`, kept **outside** `--checkpoints <dir>` so `league-scorer`'s directory validator (`validate_checkpoints_layout` at `crates/league-scorer/src/cli.rs:383-419`) continues to see strictly `<64-hex>.save.json` and `<64-hex>.seal` files inside `--checkpoints`.

Schema (`VerifiedCheckpointsManifest`, `#[serde(deny_unknown_fields)]`, canonical 2-space sorted-key JSON via `to_sorted_pretty_json`; zero `game_id`, zero arm labels, zero filesystem paths, and zero wall-clock timestamps so `league-scorer` remains arm-blind):

```text
VerifiedCheckpointsManifest {
  schema_version: u32,                     // 1
  engine_build_id: String,                 // 40-hex clean git SHA (LEAGUE_EVAL_GIT_SHA)
  save_format_version: u32,                // EVAL_SAVE_FORMAT_VERSION (2)
  seal_format_version: u32,                // EVAL_SEAL_FORMAT_VERSION (2)
  entries: Vec<VerifiedCheckpointEntry>,   // sorted ascending by pre_act_pair_sha256, unique
}

VerifiedCheckpointEntry {
  pre_act_pair_sha256: String,             // 64-hex pair digest (lookup key)
  pre_act_save_sha256: String,             // 64-hex .save.json SHA-256
  pre_act_seal_sha256: String,             // 64-hex .seal SHA-256
  engine_commit: String,                   // 40-hex provenance stamped in EvalSealPayload::engine_commit
  turn: u8,                                // 1..=32
  seat_count: u8,                          // 4
  expected_state_digest: String,           // 64-hex hex(TurnEngine::state_digest()) at tip turn
  turn_chain_digest: String,               // 64-hex C_{turn-1} cumulative chain digest
  post_probe_state_digest: String,         // 64-hex post-probe-step TurnEngine::state_digest()
}
```

**Read-only after write and mandatory freeze/join binding:** The manifest is created atomically with `OpenOptions::new().write(true).create_new(true)` (refusing overwrite with `VERIFIED_CHECKPOINTS_EXISTS`), flushed with `File::sync_all()`, and immediately locked read-only via `std::fs::Permissions::set_readonly(true)` (`0444` on Unix, read-only attribute on Windows; internal research report `§4`, `§5`). Verified `.save.json` and `.seal` blobs inside `--checkpoints` are likewise set read-only.

`league-eval freeze` requires `--verified-checkpoints <file>` (`MISSING_VERIFIED_CHECKPOINTS` if omitted/absent), verifies `engine_build_id == scorer_git_sha` and complete coverage of `blind_inputs/*.json` (`VERIFIED_CHECKPOINTS_MISMATCH`), and records `verified_checkpoints_sha256` inside `scorer_freeze.json` (`ScorerFreezeFile` and `ScorerFreezeBlindView`). `league-eval join` refuses if `scorer_freeze.json` omits `verified_checkpoints_sha256` or if `--verified-checkpoints <file>` is omitted/absent (`MISSING_VERIFIED_CHECKPOINTS`), verifies exact SHA-256 and per-state checkpoint coverage (`VERIFIED_CHECKPOINTS_MISMATCH`), and records `verified_checkpoints_sha256`, `scorer_engine_build_id`, `checkpoint_engine_commits`, `checkpoint_engine_commit` (per row), and `scenario_randomness_scope` in `per_arm_summary.json` and `joined_decisions.json`.

### Cost per Checkpoint

Gate 2 runs `turn - 1` historical sweeps (`0..=31`), one tip deserialization comparison, and two single-turn deterministic `ClubBot` probe sweeps (`<= 33` integer engine sweeps total, `O(turn)` time, zero GPU or model calls). This executes once per checkpoint (`24` checkpoints in the D5 pilot, `120` in the D5 main run; `docs/decision-scorer.md:904-906`), avoiding paying turn-zero replay costs across all `18,816` to `94,080` counterfactual rollouts.

---

## 3. Gate 3: Scoring Worker Verification, In-Memory Cloning, and Post-Batch Canary (`§1`, `§2`, `§5`)

Following an internal research report (`§1` ZFS RAM fault-injection findings; `§2` and `§5`):

### Single-Read Worker Verification and TOCTOU Closure

Inside `execute_blind_request_verified` (`crates/league-scorer/src/rollout.rs`):

1. **Manifest check:** Look up `request.pre_act_pair_sha256` in the loaded `VerifiedCheckpointsManifest`. Verify `manifest.engine_build_id == env!("LEAGUE_SCORER_GIT_SHA")` (provided by `crates/league-scorer/build.rs`, which compiles with an empty fallback when git is unavailable and causes Gate 3 to fail closed at runtime whenever `LEAGUE_SCORER_GIT_SHA` is empty or non-40-hex), `manifest.save_format_version == EVAL_SAVE_FORMAT_VERSION` (`2`), `manifest.seal_format_version == EVAL_SEAL_FORMAT_VERSION` (`2`), valid 40-hex `entry.engine_commit` provenance, and exact equality on `pre_act_save_sha256`, `pre_act_seal_sha256`, and `pre_act_pair_sha256` (`CheckpointVerifyError::IncompatibleEngineOrSchemaVersion` / `OuterFileHashMismatch`).
2. **Single disk read and outer hash check:** Read `save_bytes` and `seal_bytes` **once** from `--checkpoints/<sha256>.{save.json,seal}` and check `sha256_hex(&save_bytes) == request.pre_act_save_sha256`, `sha256_hex(&seal_bytes) == request.pre_act_seal_sha256`, and `compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes) == request.pre_act_pair_sha256` (`CheckpointVerifyError::OuterFileHashMismatch`).
3. **In-memory host construction:** Construct `base_host` directly from those verified `(save_bytes, seal_bytes)` slices via `EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)` (`crates/league-core/src/eval/host.rs`), closing the double-read TOCTOU window (`docs/decision-scorer.md:1236`) while keeping `from_save_and_seal_bytes` `pub(crate)`.
4. **Post-load canonical state digest check:** Compute `base_digest = base_host.canonical_state_digest_hex()?` (calling `TurnEngine::state_digest()`) and assert `base_digest == entry.expected_state_digest`, `base_host.turn() == entry.turn`, `base_host.seat_count() == entry.seat_count`, and (when `entry.turn > 1`) `base_digest == base_host.last_live_state_digest_hex()?` (`CheckpointVerifyError::TipStateDigestMismatch`). No historical turn log replay occurs in workers.

### Load Once and Clone in Memory

`EvalHost` derives `Clone` (`crates/league-core/src/eval/host.rs`; every field of `EvalHost` contains owned value types and `BTreeMap`/`BTreeSet`/`Vec` collections). Inside the `K * M` rollout loop, `execute_blind_request_verified` clones the single verified `base_host: EvalHost` in RAM per rollout via `run_single_rollout_from_host(&base_host, ...)` (`crates/league-scorer/src/rollout.rs`), reducing disk reads and JSON parses per scored state from `1 + K * M` to `1`.

### Post-Batch Base-State Canary and On-Disk Checkpoint Check

Immediately after the `K * M` rollout loop completes and before returning `StateScoreOutput`, Gate 3 recomputes `base_host.canonical_state_digest_hex()?` and `base_host.public_save_snapshot()?` and asserts exact equality with `entry.expected_state_digest` and the pre-batch snapshot (`CheckpointVerifyError::InMemoryBaseStateCorrupted`). Both `execute_blind_request` and `execute_blind_request_verified` also re-read `.save.json` and `.seal` once after the batch and verify byte and SHA-256 equality against the kept pre-batch buffers (`ScorerError::CheckpointChangedDuringBatch` / `CHECKPOINT_CHANGED_DURING_BATCH`), while `league-scorer score` stages `<out>/scores/*.json` in memory and re-verifies prior pairs in multi-state runs so a mid-batch file change refuses the entire batch rather than writing partial files.

### Fit with `league-scorer` Inputs and Freeze Binding

`BlindScoringRequest` (`BLIND_SCORING_SCHEMA_VERSION = 1`) **requires zero changes**: it already carries `pre_act_save_sha256`, `pre_act_seal_sha256`, and `pre_act_pair_sha256`. `ScorerConfig` and its defaults remain completely untouched.

- `league-scorer score` and `league-scorer candidates` accept `--verified-checkpoints <file>` (`crates/league-scorer/src/cli.rs`), and `--verified-checkpoints <file>` is **mandatory** whenever `--freeze <file>` is supplied (`MISSING_VERIFIED_CHECKPOINTS` if omitted or missing on disk).
- `ScorerFreezeFile` (`crates/league-eval/src/freeze_gate.rs`) and `ScorerFreezeBlindView` (`crates/league-scorer/src/types.rs`) carry `verified_checkpoints_sha256: String` (`#[serde(default)]`, refused when empty or mismatched with `MISSING_VERIFIED_CHECKPOINTS` / `VERIFIED_CHECKPOINTS_MISMATCH`).

---

## 4. Pair Binding Specification (`§4`, `§7` — Recorded Format v2 Active)

Following an internal research report (`§4` and `§7`), single-machine local workflows do not benefit from symmetric HMAC keys held by the same OS user that runs producer and worker binaries; domain-separated SHA-256 bound at write time plus a read-only preflight manifest suffices.

**Implementation status:** Recorded format v2 is active for Evaluation Mode checkpoints (`EVAL_SAVE_FORMAT_VERSION = 2`, `EVAL_SEAL_FORMAT_VERSION = 2`). Club Mode persistence uses `CLUB_SAVE_FORMAT_VERSION = 1` and `CLUB_SEAL_FORMAT_VERSION = 2` (`crates/league-core/src/club/host.rs:46` and `:51`) and remains isolated from Evaluation Mode.

### Format Bump and Binding Fields

Bump `EVAL_SAVE_FORMAT_VERSION` and `EVAL_SEAL_FORMAT_VERSION` (`crates/league-core/src/eval/host.rs`) from `1` to `2`.

1. **Addition to `EvalSaveSnapshot` (`.save.json`):**
   - Already present: `format_version: u32`, `table_id: TableId`, `turn: u8`.
   - Added: `pub game_id: String` — 32 lowercase ASCII hex characters matching `EvalHost::game_id`, validated on load.
2. **Additions to `EvalSealPayload` (`.seal`):**
   - Already present: `format_version: u32`, `game_id: String`.
   - Added:
     - `save_sha256: Option<String>` — 64 lowercase ASCII hex digest `SHA-256("league-core/eval-save/v2|" || save_bytes)`: **required (`Some`)** whenever `.save.json` and `.seal` are written or loaded as a pair (`write_save_and_seal`, `write_game_save_and_seal`, `write_pre_act_checkpoint`, `from_save_and_seal_bytes`), and `None` strictly on standalone `day-DD-act-recorded.seal` (which continues to require `pre_act_checkpoint_sha256: Some(...)`).
     - `save_format_version: u32` — equals `EVAL_SAVE_FORMAT_VERSION` (`2`) and must match `save.format_version`.
     - `turn: u8` — header copy of `engine.state().turn` (`1..=32`), verified against both `save.turn` and `seal.engine.state().turn`.
     - `engine_commit: Option<String>` — 40 lowercase ASCII hex git commit SHA stamped on `EvalHost` via `set_engine_commit` / `with_engine_commit` (from `LEAGUE_EVAL_GIT_SHA` in `league-eval` or an explicit 40-hex commit in unit tests). Persisting an Evaluation save/seal pair fails closed unless `engine_commit` is present and valid 40-lowercase-hex; `v2` loaders, Gate 2, and Gate 3 all refuse a missing, empty, `"unknown"`, or non-40-hex `engine_commit` / `engine_build_id` without exception.
3. **Write sequence:** `to_save_and_seal_bytes` checks `engine_commit` is 40 lowercase hex, serializes `save_bytes` first, computes `save_sha256`, embeds it along with `turn`, `save_format_version`, and `engine_commit` in `EvalSealPayload`, serializes `seal_bytes`, executes Gate 1 round-trip decoding, and writes `.seal` then `.save.json` via `write_atomic_bytes`.
4. **Load validation:** `from_save_and_seal_bytes` verifies `save.game_id == seal.game_id`, `save.turn == seal.turn == seal.engine.state().turn`, `save.format_version == seal.save_format_version == EVAL_SAVE_FORMAT_VERSION` (`2`), `seal.save_sha256 == Some(SHA-256("league-core/eval-save/v2|" || save_bytes))`, and valid 40-lowercase-hex `seal.engine_commit` before reconstructing `EvalHost`.

### Treatment of Existing `v1` Checkpoints

**Fail-closed `v2`-only refusal (no legacy flag):** `from_save_and_seal_bytes`, `league-eval verify-checkpoints` (`Gate 2`), and `league-scorer` (`Gate 3`) accept `format_version == 2` only and refuse `v1` checkpoints everywhere with `INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION` (`CheckpointVerifyError::IncompatibleEngineOrSchemaVersion` / `EngineError::UnsupportedReplayVersion { found: 1, newest_known: 2 }`). There is **no `--allow-legacy-v1` flag**.

---

## 5. Canonical Bytes and Strict Decoding (`§3`, `§6`)

### Current Serializer Guarantees at `885929c4`

Inspection of `crates/league-core/src/{engine.rs,types.rs,contracts.rs,commit.rs,scoring.rs,observation.rs,eval/host.rs}` confirms four structural properties are already satisfied:

1. **Zero floating-point fields in serialized state:** `EngineState` (`crates/league-core/src/engine.rs:59-139`), `TurnEngine` (`:340-344`), `TableSpec` (`crates/league-core/src/types.rs:364-423`), `EvalSaveSnapshot` (`crates/league-core/src/eval/host.rs:54-121`), and `EvalSealPayload` (`:167-188`) contain no `f32` or `f64` fields; all quantities, parities, and basis-point rates are integers (`i64` `Units`, `u16` `Bps`).
2. **Ordered maps and sets only:** Every map and set uses `BTreeMap` or `BTreeSet` (zero `HashMap` or `HashSet` fields), including ordered tuple-key serialization via `delivery_ledger_serde` (`crates/league-core/src/engine.rs:51-55, 105-106`).
3. **Single JSON representation per struct:** `TurnEngine::state_digest()` (`crates/league-core/src/engine.rs:511-519`) hashes `b"league-core/state/v2" || serde_json::to_vec(&self.state)`; `.seal` uses `serde_json::to_vec(&seal)` (`eval/host.rs:948`); `.save.json` uses `serde_json::to_vec_pretty(&save)` (`eval/host.rs:913`).
4. **Validated UTF-8 strings:** `serde_json` accepts only valid UTF-8 and preserves exact string bytes without locale normalization.

### Current Strictness Gaps (`M07`, `M08`)

1. **Lenient JSON parser:** `serde_json::from_slice` accepts reordered struct keys, extra whitespace or altered formatting (pretty `.seal` vs compact `.save.json`), trailing whitespace after the root object, non-minimal `\uXXXX` escapes, explicitly serialized empty optional fields marked `skip_serializing_if`, and **duplicate object keys** (where `BTreeMap`'s `Deserialize` impl silently keeps the last duplicate key).
2. **Silent unknown-field drop (`M08`):** `EvalSaveSnapshot`, `EvalSealPayload`, `TurnEngine`, `EngineState`, `TableSpec`, and `LoggedTurn` do not carry `#[serde(deny_unknown_fields)]`, so unknown fields injected at top level or nested inside `engine.state` are silently dropped during deserialization.

### Implemented Verifier-Side Strict Verification (No Engine Serde Change)

Adding `#[serde(deny_unknown_fields)]` directly onto structs in `engine.rs`, `types.rs`, or `league-seat-types` would touch files outside `eval/` (and could break `ReplayRecord` flatten semantics documented at `crates/league-core/src/replay.rs:973-974`). Instead, `crates/league-core/src/eval/checkpoint_verify.rs` enforces full strictness strictly on the verifier side:

1. **Streaming duplicate-key and truncation check (`StrictJsonValueVisitor`):** Parses raw `save_bytes` / `seal_bytes` with a custom `serde::de::DeserializeSeed` value walker (no new third-party crates) rejecting syntax/EOF truncation (`CheckpointVerifyError::UnexpectedEofOrMalformedEncoding`, `M02`) and **duplicate JSON object keys at any nesting depth** (`CheckpointVerifyError::NonCanonicalEncoding`, `M07`).
2. **Recursive AST unknown-field and `skip_serializing_if` check (`check_unknown_and_skipped_fields`):** Re-serializes the decoded `EvalSaveSnapshot` / `EvalSealPayload` to `serde_json::Value` (`canonical_val`) and walks `(raw_val, canonical_val)`:
   - Any object key present in `raw_val` and absent from `canonical_val` raises `CheckpointVerifyError::UnknownFieldRejected { file, field_path }` (`M08`), **except** the explicit allowlist of six `skip_serializing_if` branches in `is_known_skip_serializing_field` (`crates/league-core/src/eval/checkpoint_verify.rs:548-582`): `.seal` top-level `pre_act_checkpoint_sha256`, `save_sha256`, and `engine_commit` when `null`; `.seal` `engine.public_receipts` when `[]`; `.seal` `logged_turns[i].cancel_lock_at` when `null`; and `.seal` `recorded_act_submissions.<seat>.pass_b_actions` when `[]` — where an explicitly emitted empty/`null` default raises `CheckpointVerifyError::NonCanonicalEncoding` (`M07`) instead of `UnknownFieldRejected`.
3. **Canonical byte round-trip check (`verify_written_pair_round_trip`):** Asserts `serde_json::to_vec_pretty(&save)? == save_bytes` and `serde_json::to_vec(&seal)? == seal_bytes`, raising `CheckpointVerifyError::NonCanonicalEncoding` on whitespace, key reordering, trailing bytes, or non-minimal string escapes, and verified across two separate processes (`cross_process_round_trip_byte_equality` and `verify_checkpoints_cross_process_round_trip_and_blind_coverage_checks`).

---

## 6. Typed Error Enum, 10-Mutant Matrix, and Save-Load-Step Test (`§6`)

### Typed Error Enum (`CheckpointVerifyError`)

Define `CheckpointVerifyError` in `crates/league-core/src/eval/checkpoint_verify.rs` implementing `fmt::Display`, `std::error::Error`, and `ToStructuredError`, with stable `SCREAMING_SNAKE` codes:

| Variant | Code | Fields | Raised when |
|---|---|---|---|
| `OuterFileHashMismatch` | `OUTER_FILE_HASH_MISMATCH` | `{ file, expected, actual }` | File bytes disagree with manifest or `BlindScoringRequest` SHA-256 |
| `UnexpectedEofOrMalformedEncoding` | `UNEXPECTED_EOF_OR_MALFORMED_ENCODING` | `{ file, line, column, detail }` | Truncated JSON, syntax error, or trailing garbage |
| `PairBindingMismatch` | `PAIR_BINDING_MISMATCH` | `{ reason }` | Save/seal pair fails `game_id`, `layout_seed`, `turn`, or `save_sha256` binding |
| `TipStateDigestMismatch` | `TIP_STATE_DIGEST_MISMATCH` | `{ turn, expected, actual, detail }` | Loaded tip state disagrees with `live_state_digests`, `.save.json`, manifest, or replay tip |
| `ReplayDigestMismatch` | `REPLAY_DIGEST_MISMATCH` | `{ turn, expected, actual }` | Turn-0 replay diverges at intermediate turn `turn` |
| `ChainHashMismatch` | `CHAIN_HASH_MISMATCH` | `{ turn, expected, actual }` | Cumulative turn chain digest `C_k` diverges at `turn` |
| `ProbeStepDivergence` | `PROBE_STEP_DIVERGENCE` | `{ turn, replayed_digest, loaded_digest }` | Deterministic 1-turn `ClubBot` probe step diverges between replayed and loaded hosts |
| `NonCanonicalEncoding` | `NON_CANONICAL_ENCODING` | `{ file, detail }` | Key reordering, extra whitespace, duplicate keys, or non-canonical JSON |
| `UnknownFieldRejected` | `UNKNOWN_FIELD_REJECTED` | `{ file, field_path }` | Unrecognized JSON key in `.save.json` or `.seal` |
| `IncompatibleEngineOrSchemaVersion` | `INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION` | `{ field, found, expected }` | `v1`/unknown `format_version`, missing/malformed `engine_commit` or `engine_build_id`, or Gate 3 `engine_build_id` skew |
| `InMemoryBaseStateCorrupted` | `IN_MEMORY_BASE_STATE_CORRUPTED` | `{ expected, actual }` | Gate 3 post-batch base-host canary digest or snapshot changed in RAM |

### 10-Mutant Matrix (`§6`)

Each test generates a deterministic 10-completed-turn synthetic golden checkpoint (`day-11-pre-act.save.json`, `day-11-pre-act.seal`, `logged_turns.len() == 10`, `live_state_digests.len() == 10`) in memory, applies one programmatic mutation, recomputes outer digests where needed so inner gates execute, and asserts the exact `CheckpointVerifyError` variant:

| Mutant ID | Programmatic Mutation | Target Gate(s) | Asserted `CheckpointVerifyError` Variant | Status |
|---|---|---|---|---|
| `M01_disk_bitflip` | Seal manifest, then flip 1 bit in `.save.json` or `.seal` bytes | Gate 2 (outer check) & Gate 3 | `OuterFileHashMismatch` | Implemented |
| `M02_torn_tail` | Truncate `.save.json` or `.seal` by 19 bytes | Gate 1, Gate 2, Gate 3 | `UnexpectedEofOrMalformedEncoding` | Implemented |
| `M03_cross_game_swap` | **(a)** Pair Game A's `.save.json` with Game B's `.seal`; **(b)** Pair Turn-1 D2 duplicate save and seal | Gate 2 & Gate 3 | `PairBindingMismatch` (`v2` `save.game_id` / `seal.save_sha256`) & `TipStateDigestMismatch` | Implemented (`v2`) |
| `M04_turn_offset_pad` | Pair `day-10` `.save.json` with `day-11` `.seal`, or truncate/pad `live_state_digests` / `logged_turns` relative to `engine.state().turn` | Gate 2 & Gate 3 | `PairBindingMismatch` / `TipStateDigestMismatch` | Implemented |
| `M05_co_written_bug` | Mutate seat state in `seal.engine`, regenerate `.save.json`, `save_sha256`, and `live_state_digests[9]` | **Gate 2** | `ReplayDigestMismatch` / `TipStateDigestMismatch` | Implemented |
| `M06_mid_log_edit` | **(a)** Alter Turn-4 action or `live_state_digests[3]`; **(b)** Alter non-state field in `logged_turns[3]` while passing an external `turn_chain_digest` reference to `verify_checkpoint_causal_against_chain` | **Gate 2** (`verify_checkpoint_causal` for **(a)**; test-only `verify_checkpoint_causal_against_chain` helper for **(b)**) | **(a)** `ReplayDigestMismatch { turn: 4, .. }`; **(b)** `ChainHashMismatch { turn: 4, .. }` | **(a)** Enforced in production; **(b)** Not checked in production today (`turn_chain_digest` is computed and stored in `verified_checkpoints.json`, while `verify_checkpoint_causal_against_chain` is called only by tests) |
| `M07_non_canonical` | Re-serialize `.seal` pretty, reorder keys, append trailing whitespace, inject duplicate JSON key at depth, or explicitly emit skipped empty/null optional field | Gate 1, Gate 2, Gate 3 | `NonCanonicalEncoding` | Implemented |
| `M08_unknown_field` | Insert unknown key at top level of `.save.json`/`.seal` and nested inside `seal.engine.state` | Gate 1, Gate 2, Gate 3 | `UnknownFieldRejected` | Implemented |
| `M09_engine_skew` | Downgrade `format_version` to `1` (`v1`), omit/malform `seal.engine_commit`, or skew Gate 3 `manifest.engine_build_id` | Gate 2 & Gate 3 | `IncompatibleEngineOrSchemaVersion` | Implemented (Gate 2 does not equality-gate `engine_commit`; separate `engine_commit_is_provenance_not_equality_gate` test verifies cross-commit provenance `COMMIT_A` -> `COMMIT_B`) |
| `M10_shared_ref_bleed` | Mutate `base_host` in memory via `execute_blind_request_verified_with_hook` during a `league-scorer` batch before post-batch canary check | **Gate 3 Canary** | `InMemoryBaseStateCorrupted` | Implemented |

### Save-Load-Step Commutativity & Cross-Process Tests (`§6`)

Play a deterministic 4-seat `ClubBot` season through close (`save_load_step_commutativity_across_full_season`). At every pre-act turn `t`, serialize `(save_bytes, seal_bytes)` via `to_save_and_seal_bytes`, verify canonical round-trip bytes and reload `reloaded_host` via `EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)`, compute identical `ClubBot` submissions (`bot_tie_seed`), advance at least one full turn (and run to season close on a sampled mid-season turn), and assert identical `to_save_and_seal_bytes`, `canonical_state_digest_hex`, and `PublicSweepReceipt`. Negative probe controls in `crates/league-core/tests/checkpoint_verify.rs` exercise two paths through `verify_checkpoint_causal_with_probe_fault_for_test`: `probe_step_divergence_negative_control_caught_by_gate2` (`:848-871`) calls `begin_cancel_lock()` on the loaded probe copy before the probe step (causing `apply_act_submissions` to fail on the loaded copy and map to `CheckpointVerifyError::ProbeStepDivergence`), while `strict_error_variants_skip_serializing_fields_and_action_replay_paths` (`:1458-1481`) injects an extra `RecordIntentPledge` market action onto the loaded probe copy so both copies complete `sweep_turn()` cleanly with distinct post-probe digests and return `CheckpointVerifyError::ProbeStepDivergence`. Two-process tests (`cross_process_round_trip_byte_equality` and `verify_checkpoints_cross_process_round_trip_and_blind_coverage_checks`) write a checkpoint in a child process and verify canonical byte equality in the parent process.

Implemented test suites (respecting crate isolation boundaries in `raw_api_backdoor_guard.rs`):
- `crates/league-core/tests/checkpoint_verify.rs` (`M01`–`M09`, cross-commit provenance vs replay divergence, Save-Load-Step commutativity, probe-step negative control, cross-process round-trip)
- `crates/league-scorer/tests/checkpoint_gate3.rs` (Gate 3 manifest/hash/digest checks, `M10` shared-ref canary, freeze binding parity)
- `crates/league-eval/tests/verify_checkpoints_cli.rs` (`league-eval verify-checkpoints`, read-only output/locks, `--allow-legacy-v1` flag rejection and `v1` refusal, cross-commit provenance + replay divergence, cross-process verify, freeze/join binding & readout provenance/`scenario_randomness_scope`)
- `crates/league-core/tests/raw_api_backdoor_guard.rs` (allowlist updates for `eval/checkpoint_verify.rs` and new `EvalHost` pub methods)

### Known Limits

- **Unauthenticated `engine_commit` provenance:** `seal.engine_commit` is recorded provenance, not an authenticated build attestation (there is no cryptographic signature or MAC over `.save.json` or `.seal`). Gate 2 validates only `40`-lowercase-hex syntax and format-`v2` SHA-256 bindings without comparing `seal.engine_commit` against any expected commit, so a checkpoint whose `seal.engine_commit` is hand-edited to another 40-lowercase-hex SHA (with recomputed `save_sha256` / outer hashes when re-verified) passes `verify-checkpoints` and flows into `verified_checkpoints.json`, `joined_decisions.json`, and `per_arm_summary.json` (see also Known Limit 9 in [`docs/decision-scorer.md`](decision-scorer.md)).
- **`turn_chain_digest` not verified in production:** As noted in `M06`, `verify_checkpoint_causal` computes and stores `turn_chain_digest` (`C_{turn - 1}`) in `verified_checkpoints.json`, whereas comparing against an expected chain digest via `verify_checkpoint_causal_against_chain` (`ChainHashMismatch`) is exercised only in tests today.

---

## 7. Randomness Streams and Counterfactual Rollout Scope (`§7`)

Inspection of `crates/league-core` and `crates/league-scorer` establishes how randomness streams behave across counterfactual rollouts. **No stream change is made.**

### Randomness Scope Statement (Documented Here and in Readout JSON)

1. The verbatim `SCENARIO_RANDOMNESS_SCOPE_STATEMENT` recorded in `scenario_randomness_scope` inside both `per_arm_summary.json` and `joined_decisions.json` (`crates/league-eval/src/freeze_gate.rs:39`) is:

> Counterfactual scenarios m in 0..M share the base game's future environment RNG stream (market shocks and season-close rolls) and vary only sampled peer hidden mandates and talk-commitment honour draws; within a fixed scenario m, distinct focal actions can change tie-break outcomes (batch matching, council elections, and standings ties) but cannot change market shocks or season-close rolls.

2. **Within a fixed scenario `m`, candidate comparisons (`(c, m)` vs `(c', m)`) compare focal candidates under matched peer mandate and talk-honour draws without advancing a shared PRNG cursor** (`EngineState` carries no mutable PRNG state, and `draw_scenarios` / `draw_honour_commitment` are counter-indexed SHA-256 functions run outside the candidate loop). Market shocks and season-close rolls remain identical across focal candidates in `m` because both expand strictly from `(turn_beacon, table_id, Turn(t), domain)` via `HKDF-SHA256`; distinct focal actions can change tie-break outcomes (`RngDomain::BatchTie` keyed per `(good_kind, good_idx, side, maker)`, council elections, and standings ties under `RngDomain::Tiebreak` by changing which orders or seats tie), but cannot change market shocks or season-close rolls.
3. **Resampling future environment draws across `m` remains a future option only:** Nothing is built for per-`m` environment re-keying; it remains documented below as a possible future extension.

### Evidence at `885929c4` (`path:line`)

#### Engine Environment Draws (`crates/league-core`)

- `EvalHost::new` (`crates/league-core/src/eval/host.rs:243-327`, proof derivation at `:294`) pre-derives all 32 `EvalVrf` proofs via `derive_all_vrf_proofs` (`:1530-1540`) -> `SealedEvalSeed::prove_turn` (`crates/league-core/src/commit.rs:869-892`) over `eval_vrf_alpha(table_id, turn)` (`commit.rs:897-903`) and stores them in `EvalSealPayload::vrf_proofs` (`eval/host.rs:173, 933`).
- `EngineState` (`crates/league-core/src/engine.rs:59-139`) carries **no mutable PRNG state**. During `EvalHost::sweep_turn` (`eval/host.rs:739-778`), turn `t` selects `self.vrf_proofs[t - 1]` (`:744-750`) and passes it to `TurnEngine::sweep_turn_with_eval_vrf` (`engine.rs:1245-1279`), which extracts `turn_beacon` (`commit.rs:921-954`).
- Every intra-turn draw expands `(turn_beacon, table_id, Turn(t), RngDomain, subtag)` statelessly via RFC 5869 `HKDF-SHA256` (`expand_turn_stream`, `crates/league-core/src/commit.rs:548-576`; `RngDomain`, `:521-543`):
  - **Market shocks:** `step8_shocks_and_close` (`crates/league-core/src/engine.rs:3941-3949`) calls `draw_shocks_for_turn` (`crates/league-core/src/commit.rs:585-625`), expanding `RngDomain::Shocks` (`commit.rs:594`; `"shocks"` at `:537`) on `DEFAULT_SHOCK_DRAW_TURNS = [2, 7, 12, 17]` (`commit.rs:580`).
  - **Closing bell roll:** `step8_shocks_and_close` (`crates/league-core/src/engine.rs:3990-3993`) calls `close_roll_bps_from_beacon` (`crates/league-core/src/commit.rs:630-633`), expanding `RngDomain::CloseRoll` (`commit.rs:631`; `"close_roll"` at `:538`) for turns `24..32`.
  - **DEX batch, council, and standings tie-breaks:** Batch matching priority (`engine.rs:1874-1880`) expands `RngDomain::BatchTie` keyed per `(good_kind, good_idx, side, maker)`; election plurality ties call `break_tie_with_turn_beacon` (`commit.rs:638-654`, `RngDomain::Tiebreak`); final standings ties call `standings_tie_key` (`commit.rs:666-676`, `RngDomain::Tiebreak`, subtag `b"standings|<seat>"` at `:672-674`).
- **Turn-1 mandate deal and Phase-2 salts:** Deals are fixed once at Turn 0 via `deal_graded_mandates_with_entropy` (`commit.rs:314-368`, calling `deal_mandate_deck_indices` at `:288-310` with domain `b"league-core/mandate-deal/v2"` at `:301`), and Phase-2 commit salts are pre-derived for every `(turn, seat)` slot at init via `derive_all_turn_commit_salts` (`eval/host.rs:299, 1542-1565`) and indexed directly by `(turn - 1, seat)` (`:539-548`).

#### Scorer Scenario & Per-Seat Draws (`crates/league-scorer` and `ClubBot`)

- **Undealt peer mandate draws:** `draw_scenarios` (`crates/league-scorer/src/scenarios.rs:71-144`) runs once per `(state, m)` outside the candidate loop (`execute_blind_request`, `crates/league-scorer/src/rollout.rs:293-301`), using counter-indexed SHA-256 (`sha256_u64_le`, `scenarios.rs:21-33, 50-58`, domain `b"league-scorer/mandate-draw/v1|"`) keyed by `(scorer_seed, pre_act_pair_sha256, day, m, ctr)`.
- **Talk-commitment honour draws:** `draw_honour_commitment` (`crates/league-scorer/src/postures.rs:96-117, 137-169`, domain `b"league-scorer/talk-honour/v1|"`) runs once per `(state, m, peer_seat)` before the candidate loop (`rollout.rs:315-338`), keyed by `(scorer_seed, pre_act_pair_sha256, day, m, canonical_commitment, ctr)`.
- **Per-seat rollout bot seeds:** `scenarios.rs:130-137` derives `bot_base_seed` under domain `b"league-scorer/scenario-bot-seed/v1|"` keyed by `(scorer_seed, pre_act_pair_sha256, day, m)`. Inside `run_single_rollout_with_replay_log` (`rollout.rs:213-223`) and `execute_blind_request` (`:324-325`), each seat `s` receives `scenario.bot_base_seed.wrapping_add(u64::from(s))`. `ClubBot` (`crates/league-core/src/bot.rs:33-35`) holds no mutable RNG state; it uses `bot_tie_seed` only statelessly in Turn-`4n` `cartel_chair` ballot selection (`bot.rs:534-541`).

### What Optional Per-Scenario Environment Resampling Would Require

Because `EngineState`, `ClubBot`, `draw_scenarios`, and `draw_honour_commitment` are already stateless and separated across candidate comparisons `c` vs `c'`, **no stream-splitting change is needed for matched counterfactual draws**. If future work ever wants independent environmental trajectories across scenarios `m = 0..M` (replacing the base-game VRF proofs from turn `day` onward rather than pinning them to the base season):

1. **Seeds:** Derive a scenario environment `SealedEvalSeed` per `(scorer_seed, pre_act_pair_sha256, day, m)` via SHA-256 with a dedicated domain tag (for example `b"league-scorer/scenario-env-vrf/v1|"`), ensuring non-zero entropy.
2. **Call sites:** Add a host-level in-memory helper in `crates/league-core/src/eval/host.rs` (e.g., `rekey_future_vrf_for_rollout(scenario_eval_seed)`) invoked inside `crates/league-scorer/src/rollout.rs` on the cloned rollout host after turn `day` assignment, updating `spec.eval_vrf_pubkey` and recomputing `vrf_proofs[(day - 1)..]` for turns `day..=HARD_TURN_CAP` while preserving `drawn_shocks` and `active_shocks` from turns `< day` (note: any shock drawn on `turn < day` such as Turn 2 realizing at Turn 5 remains fixed in `EngineState::drawn_shocks`).
3. **Recorded-format and determinism impact:** Zero change to on-disk `.save.json` or `.seal` recorded formats (mutation happens strictly in RAM on the rollout clone, whose `finalize_eval_season` self-replays against the clone's rekeyed `spec.eval_vrf_pubkey`). Scores for `scenarios_m` would change relative to `v1` baselines because future shocks and closing turns would vary across `m`, so it would require gating behind a new `ScorerConfig` field (`crates/league-scorer/src/types.rs:227-276`) and re-baselining `ScorerConfig` snapshots.
