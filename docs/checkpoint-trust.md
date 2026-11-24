# Evaluation Checkpoint Verification Before Resumption

This design note specifies how the Evaluation harness verifies a saved pre-act checkpoint pair before forking counterfactual branches or running scoring rollouts, and documents how counterfactual rollout randomness streams are currently separated. No game rules or play-wire messages change. All verification designs follow the principles in an internal research report (`§1`–`§7`).

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

At Gate 1 (producer write time, internal research report `§5`), `EvalHost::to_save_and_seal_bytes` (`crates/league-core/src/eval/host.rs:911-918`) should additionally verify `turn > 1 => self.engine.state_digest()? == self.live_state_digests[usize::from(turn - 2)]` and run an in-memory `from_save_and_seal_bytes(&save_bytes, &seal_bytes)` round-trip before writing temporary files to disk, so any asymmetric serialization bug aborts immediately at checkpoint creation.

---

## 2. Gate 2: Study-Preflight Causal Verification and Verified Manifest (`§2`, `§5`)

Following an internal research report (`§2` "Full Deterministic Replay from the Start", "Merkle Structures and Incremental Hash Chains"; `§5` "Gate 2: Once at Study Preflight / First Load (Causal Gatekeeper)"), causal verification runs **once per distinct checkpoint** synchronously at study preflight inside the exact compiled engine revision used by scoring workers, with zero model calls.

### Where Gate 2 Runs

Add a preflight subcommand in `crates/league-eval/src/cli.rs` (`:213-224`):

```text
league-eval verify-checkpoints --checkpoints <dir> --out <verified_checkpoints.json>
```

It runs once after `league-eval build-blind-inputs --out-checkpoints <dir>` (`crates/league-eval/src/blind_builder.rs:368-631`) exports content-addressed `<64-hex>.save.json` and `<64-hex>.seal` pairs, and before `league-eval freeze` (`crates/league-eval/src/freeze_gate.rs:286`) or any `league-scorer score` worker runs. Every unique checkpoint pair found in `--checkpoints` (or referenced by `--blind-inputs`) is verified using `LEAGUE_EVAL_GIT_SHA` (`crates/league-eval/build.rs:65-70`) on a clean worktree (`resolve_crate_git_sha_from`, `crates/league-eval/src/serving.rs:560-573`).

### What Gate 2 Checks per Checkpoint

`EvalHost::verify_checkpoint_causal` (delegating to `crates/league-core/src/eval/checkpoint_verify.rs`) executes five checks on the `(save_bytes, seal_bytes)` pair in memory:

1. **Single-read parse, outer digests, and shape bounds:**
   - Compute `pre_act_save_sha256`, `pre_act_seal_sha256`, and `pre_act_pair_sha256` (`compute_pre_act_pair_sha256_hex`, `crates/league-core/src/eval/host.rs:1382-1391`).
   - Parse and validate `EvalSaveSnapshot` and `EvalSealPayload` under pre-act invariants (`ensure_pre_act_state`, `crates/league-core/src/eval/host.rs:1294-1317`, `TableMode::Eval`, `seat_count == 4`, `turn >= 1 && turn <= HARD_TURN_CAP`).
   - Verify `logged_turns.len() == usize::from(turn - 1)` and `live_state_digests.len() == usize::from(turn - 1)`, raising `CheckpointVerifyError::TipStateDigestMismatch` on mismatch.
2. **Header and provenance compatibility:**
   - Verify `save.format_version == EVAL_SAVE_FORMAT_VERSION`, `seal.format_version == EVAL_SEAL_FORMAT_VERSION`, and (once Section 4 binding is enabled) `seal.save_sha256`, `save.game_id == seal.game_id`, `seal.turn == save.turn`, and `seal.engine_commit == current_engine_commit`, raising `CheckpointVerifyError::IncompatibleEngineOrSchemaVersion` or `CheckpointVerifyError::PairBindingMismatch`.
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
       using compact serialized `LoggedTurn` bytes (`serde_json::to_vec(logged)`).
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
  save_format_version: u32,                // EVAL_SAVE_FORMAT_VERSION
  seal_format_version: u32,                // EVAL_SEAL_FORMAT_VERSION
  entries: Vec<VerifiedCheckpointEntry>,   // sorted ascending by pre_act_pair_sha256, unique
}

VerifiedCheckpointEntry {
  pre_act_pair_sha256: String,             // 64-hex pair digest (lookup key)
  pre_act_save_sha256: String,             // 64-hex .save.json SHA-256
  pre_act_seal_sha256: String,             // 64-hex .seal SHA-256
  turn: u8,                                // 1..=32
  seat_count: u8,                          // 4
  expected_state_digest: String,           // 64-hex hex(TurnEngine::state_digest()) at tip turn
  turn_chain_digest: String,               // 64-hex C_{turn-1} cumulative chain digest
  post_probe_state_digest: String,         // 64-hex post-probe-step TurnEngine::state_digest()
}
```

**Read-only after write:** The manifest is created atomically with `OpenOptions::new().write(true).create_new(true)` (refusing to overwrite an existing manifest), flushed with `File::sync_all()`, and immediately locked read-only via `std::fs::Permissions::set_readonly(true)` (`0444` on Unix, read-only attribute on Windows; internal research report `§4`, `§5`). Exported `.save.json` and `.seal` blobs inside `--checkpoints` are likewise set read-only.

### Cost per Checkpoint

Gate 2 runs `turn - 1` historical sweeps (`0..=31`), one tip deserialization comparison, and two single-turn deterministic `ClubBot` probe sweeps (`<= 33` integer engine sweeps total, `O(turn)` time, zero GPU or model calls). This executes once per checkpoint (`24` checkpoints in the D5 pilot, `120` in the D5 main run; `docs/decision-scorer.md:904-906`), avoiding paying turn-zero replay costs across all `18,816` to `94,080` counterfactual rollouts.

---

## 3. Gate 3: Scoring Worker Verification, In-Memory Cloning, and Post-Batch Canary (`§1`, `§2`, `§5`)

Following an internal research report (`§1` ZFS RAM fault-injection findings; `§2` "Comparing the Loaded State to the Recorded Digest"; `§5` "Gate 3: On Scoring Worker Load (Amortized In-Memory Safety)"):

### Single-Read Worker Verification and TOCTOU Closure

Inside `execute_blind_request` (`crates/league-scorer/src/rollout.rs:263-369`):

1. **Manifest check:** Look up `request.pre_act_pair_sha256` in the loaded `VerifiedCheckpointsManifest`. Verify `manifest.engine_build_id == env!("LEAGUE_SCORER_GIT_SHA")` (provided by a new `crates/league-scorer/build.rs` mirroring `crates/league-eval/build.rs:1-71`), `manifest.save_format_version == EVAL_SAVE_FORMAT_VERSION`, `manifest.seal_format_version == EVAL_SEAL_FORMAT_VERSION`, and exact equality on `pre_act_save_sha256`, `pre_act_seal_sha256`, and `pre_act_pair_sha256` (`CheckpointVerifyError::IncompatibleEngineOrSchemaVersion` / `OuterFileHashMismatch`).
2. **Single disk read and outer hash check:** Read `save_bytes` and `seal_bytes` **once** from `--checkpoints/<sha256>.{save.json,seal}` and check `sha256_hex(&save_bytes) == request.pre_act_save_sha256`, `sha256_hex(&seal_bytes) == request.pre_act_seal_sha256`, and `compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes) == request.pre_act_pair_sha256` (`CheckpointVerifyError::OuterFileHashMismatch`).
3. **In-memory host construction:** Construct `base_host` directly from those verified `(save_bytes, seal_bytes)` slices via `EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)` (`crates/league-core/src/eval/host.rs`), closing the double-read TOCTOU window (`docs/decision-scorer.md:1236`) while keeping `from_save_and_seal_bytes` `pub(crate)` (preserving the `compile_fail` doctest at `crates/league-core/src/eval/host.rs:508-511`).
4. **Post-load canonical state digest check:** Compute `base_digest = base_host.canonical_state_digest_hex()?` (calling `TurnEngine::state_digest()`, `crates/league-core/src/engine.rs:511-519`) and assert `base_digest == entry.expected_state_digest`, `base_host.turn() == entry.turn`, `base_host.seat_count() == entry.seat_count`, and (when `entry.turn > 1`) `base_digest == base_host.last_live_state_digest_hex()?` (`CheckpointVerifyError::TipStateDigestMismatch`). No historical turn log replay occurs in workers.

### Load Once and Clone in Memory

Derive `Clone` on `EvalHost` (`crates/league-core/src/eval/host.rs:197-215`; every field of `EvalHost` already implements `Clone` and contains only owned value types and `BTreeMap`/`BTreeSet`/`Vec` collections). Replace the per-rollout disk read and JSON parse (`run_single_rollout_with_replay_log` at `crates/league-scorer/src/rollout.rs:181-182` inside the `K * M` loop at `:347-366`) with a shared immutable `base_host: EvalHost` cloned in RAM per rollout (`let mut host = base_host.clone();`). Existing `run_single_rollout` and `run_single_rollout_with_replay_log` remain available for standalone callers/tests, while `execute_blind_request` uses `run_single_rollout_from_host(&base_host, ...)` so disk reads and JSON parses per scored state drop from `1 + K * M` (`705`–`785` in D5) to `1`.

### Post-Batch Base-State Canary Check

Immediately after the `K * M` rollout loop completes (`crates/league-scorer/src/rollout.rs:366`) and before `score_outcomes` returns or `<out>/scores/<state_id>.json` is written (`crates/league-scorer/src/cli.rs:602-612`), recompute `base_host.canonical_state_digest_hex()?` and `base_host.public_save_snapshot()?` and assert exact equality with `entry.expected_state_digest` and the pre-batch snapshot. Any divergence fails the worker with `CheckpointVerifyError::InMemoryBaseStateCorrupted` (mapped onto a new `ScorerError` variant).

### Fit with Existing `league-scorer` Inputs (`BlindScoringRequest`)

`BlindScoringRequest` (`crates/league-scorer/src/types.rs:651-674`, `BLIND_SCORING_SCHEMA_VERSION = 1`) **requires zero changes**: it already carries `pre_act_save_sha256`, `pre_act_seal_sha256`, and `pre_act_pair_sha256` (`:656-661`), which identify the checkpoint entry in `verified_checkpoints.json`. `ScorerConfig` (`:227-313`) and its defaults remain completely untouched.

To wire Gate 3 into the CLI without changing `BlindScoringRequest`:

- Add `--verified-checkpoints <file>` to `league-scorer score` and `league-scorer candidates` (`crates/league-scorer/src/cli.rs:24-25`). When supplied (mandatory in frozen/production runs; optional or omitted only with an explicit test seam or unverified helper in unit tests unless default-required), `league-scorer` reads the manifest once at startup and passes it into `execute_blind_request` and `build_arm_independent_request`.
- If freeze-manifest binding of the verified checkpoints file itself is also desired (optional recorded-format extension outside `BlindScoringRequest`), `ScorerFreezeFile` (`crates/league-eval/src/freeze_gate.rs:55-70`) and `ScorerFreezeBlindView` (`crates/league-scorer/src/types.rs:936-943`) would add `verified_checkpoints_sha256: String`; without touching `scorer_freeze.json`, `--verified-checkpoints <file>` already enforces manifest and build-ID parity with zero change to any existing JSON schema.

---

## 4. Pair Binding Proposal (`§4`, `§7` — Recorded-Format Proposal Only)

Following an internal research report (`§4` "Hash vs. Keyed MAC vs. Signature for a Single-Producer File Pair" and `§7` "Are You Binding Model, Prompt, and Engine Provenance Inside Every Checkpoint?"), single-machine local workflows do not benefit from symmetric HMAC keys held by the same OS user that runs producer and worker binaries; domain-separated SHA-256 bound at write time plus a read-only preflight manifest suffices.

**Implementation status:** Because adding fields or bumping format versions modifies `.save.json` / `.seal`, **everything in this section is a proposal and will only be implemented if explicitly approved.**

### Proposed Format Bump and Binding Fields

Bump `EVAL_SAVE_FORMAT_VERSION` (`crates/league-core/src/eval/host.rs:45`) from `1` to `2` and `EVAL_SEAL_FORMAT_VERSION` (`:48`) from `1` to `2`.

1. **Additions to `EvalSaveSnapshot` (`.save.json`, `crates/league-core/src/eval/host.rs:54-121`):**
   - Already present: `format_version: u32`, `table_id: TableId`, `turn: u8`.
   - Proposed addition: `pub game_id: String` — 32 lowercase ASCII hex characters matching `EvalHost::game_id`, validated on load with `is_lower_ascii_hex_len(&save.game_id, 32)`.
2. **Additions to `EvalSealPayload` (`.seal`, `crates/league-core/src/eval/host.rs:167-188`):**
   - Already present: `format_version: u32`, `game_id: String`.
   - Proposed additions:
     - `save_sha256: Option<String>` — 64 lowercase ASCII hex digest `SHA-256("league-core/eval-save/v2|" || save_bytes)`: **required (`Some`)** whenever `.save.json` and `.seal` are written or loaded as a pair (`write_save_and_seal`, `write_game_save_and_seal`, `write_pre_act_checkpoint`, `from_save_and_seal_bytes`), and `None` strictly on standalone `day-DD-act-recorded.seal` (which continues to require `pre_act_checkpoint_sha256: Some(...)` at `:1160, 1353`).
     - `save_format_version: u32` — equals `EVAL_SAVE_FORMAT_VERSION` (`2`) and must match `save.format_version`.
     - `turn: u8` — header copy of `engine.state().turn` (`1..=32`), verified against both `save.turn` and `seal.engine.state().turn`.
     - `engine_commit: Option<String>` — 40 lowercase ASCII hex git commit SHA stamped on `EvalHost` by `league-eval` via `LEAGUE_EVAL_GIT_SHA` (`crates/league-eval/build.rs:70`), keeping `league-core` free of a `build.rs` script. When present (mandatory in `league-eval` runs), Gate 2 and Gate 3 verify `seal.engine_commit == current_engine_commit`; `None` is permitted only in direct synthetic `EvalHost` unit-test constructors unless an explicit commit SHA is attached.
3. **Write sequence:** `to_save_and_seal_bytes` serializes `save_bytes` first, computes `save_sha256`, embeds it along with `turn`, `save_format_version`, and `engine_commit` in `EvalSealPayload`, serializes `seal_bytes`, executes Gate 1 round-trip decoding, and writes `.seal` then `.save.json` via `write_atomic_bytes`.
4. **Load validation:** `from_save_and_seal_bytes` verifies `save.game_id == seal.game_id`, `save.turn == seal.turn == seal.engine.state().turn`, `save.format_version == seal.save_format_version == EVAL_SAVE_FORMAT_VERSION`, `seal.save_sha256 == Some(SHA256("league-core/eval-save/v2|" || save_bytes))`, and `engine_commit` compatibility before reconstructing `EvalHost`.

### Treatment of Existing `v1` Checkpoints

- **Default policy (recommended): Fail-closed refusal.** `from_save_and_seal_bytes`, Gate 2 (`league-eval verify-checkpoints`), and Gate 3 (`league-scorer`) accept `format_version == 2` only and reject `v1` checkpoints with `CheckpointVerifyError::IncompatibleEngineOrSchemaVersion` (wrapping `EngineError::UnsupportedReplayVersion { found: 1, newest_known: 2 }`). This prevents silent downgrade attacks where a `v1` header skips pair binding on Turn-1 D2 duplicate swaps (`M03`) or `engine_commit` skew (`M09`).
- **Optional pre-migration path (only if archived `v1` checkpoints must be scored):** Keep `EvalHost` runtime loaders strict `v2`-only, and add an explicit opt-in `--allow-legacy-v1` flag exclusively on `league-eval verify-checkpoints` so Gate 2 can replay historical `v1` checkpoints from Turn 0 and seal their hashes into `verified_checkpoints.json`, with `v1` clearly recorded in `VerifiedCheckpointsManifest`.

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

### Proposed Strict Verification and Recorded-Format Impact

Because adding `#[serde(deny_unknown_fields)]` directly onto structs in `engine.rs`, `types.rs`, or `league-seat-types` would touch files outside `eval/` (and could break `ReplayRecord` flatten semantics documented at `crates/league-core/src/replay.rs:973-974`), `crates/league-core/src/eval/checkpoint_verify.rs` can enforce complete strictness locally at the Eval checkpoint boundary in three steps:

1. **Lexical/streaming duplicate-key and truncation check:** Parse raw `save_bytes` / `seal_bytes` with a custom `serde_json::de::Deserializer` or `serde::de::DeserializeSeed` value walker (using no new third-party crates) that rejects syntax/EOF truncation (`CheckpointVerifyError::UnexpectedEofOrMalformedEncoding`, `M02`) and **duplicate JSON object keys at any nesting depth** (`CheckpointVerifyError::NonCanonicalEncoding`, `M07`).
2. **AST structural parity check (unknown-field rejection, `M08`):** After deserializing into `EvalSaveSnapshot` / `EvalSealPayload`, re-serialize the parsed struct to `serde_json::Value` (`canonical_val`) and walk `(raw_val, canonical_val)` recursively:
   - Any object key present in `raw_val` and absent from `canonical_val` raises `CheckpointVerifyError::UnknownFieldRejected { file, field_path }` (`M08`), **except** the explicit allowlist of known `skip_serializing_if` optional fields (`EvalSealPayload::pre_act_checkpoint_sha256`, `EvalActSubmission::pass_b_actions`, `TurnEngine::public_receipts`, `LoggedTurn::cancel_lock_at`, and `TableSpec::shocks` if ever optional), where an explicitly emitted empty/null default raises `CheckpointVerifyError::NonCanonicalEncoding` (`M07`) instead of `UnknownFieldRejected`.
3. **Canonical byte round-trip check (`M07`):** Assert `serde_json::to_vec_pretty(&save)? == save_bytes` and `serde_json::to_vec(&seal)? == seal_bytes`. Any whitespace variation, trailing newline/byte, key permutation, or non-minimal string escape fails with `CheckpointVerifyError::NonCanonicalEncoding`, guaranteeing `CanonicalSerialize(Deserialize(buf)) == buf` (internal research report `§3`).

**Recorded-format impact (proposal status):** Clean checkpoints produced by `EvalHost::to_save_and_seal_bytes` already satisfy all three checks byte-for-byte, so canonical serialization does not alter emitted bytes. However, turning on strict unknown-field and canonical-byte rejection narrows the set of accepted `.save.json` / `.seal` inputs (rejecting hand-formatted files or forward-schema extras) and is therefore **proposed alongside Section 4 as a strictness change** for explicit approval.

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
| `IncompatibleEngineOrSchemaVersion` | `INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION` | `{ field, found, expected }` | `format_version`, `engine_commit`, or `engine_build_id` mismatch |
| `InMemoryBaseStateCorrupted` | `IN_MEMORY_BASE_STATE_CORRUPTED` | `{ expected, actual }` | Gate 3 post-batch base-host canary digest or snapshot changed in RAM |

### 10-Mutant Matrix (`§6`)

Each test generates a fresh deterministic 10-completed-turn synthetic golden checkpoint (`day-11-pre-act.save.json`, `day-11-pre-act.seal`, `logged_turns.len() == 10`, `live_state_digests.len() == 10`) in memory, applies one programmatic mutation, recomputes outer digests where specified so inner gates are tested, and asserts the exact `CheckpointVerifyError` variant:

| Mutant ID | Programmatic Mutation | Target Gate(s) | Asserted `CheckpointVerifyError` Variant | Applicability & Format Dependency |
|---|---|---|---|---|
| `M01_disk_bitflip` | Seal manifest, then flip 1 bit in `.save.json` or `.seal` bytes | Gate 2 (manifest verify) & Gate 3 | `OuterFileHashMismatch` | **Applies as-is** |
| `M02_torn_tail` | Truncate `.save.json` or `.seal` by 19 bytes | Gate 1, Gate 2, Gate 3 | `UnexpectedEofOrMalformedEncoding` | **Applies as-is** |
| `M03_cross_game_swap` | **(a)** Pair Game A's `day-11-pre-act.save.json` with Game B's `day-11-pre-act.seal` (equal `logged_turns.len() == 10`), recomputing outer hashes; **(b)** Pair Turn-1 D2 duplicate save and seal | Gate 2 & Gate 3 | **(a)** `TipStateDigestMismatch` (via `public_save_snapshot` / state divergence) or `PairBindingMismatch` (with binding); **(b)** `PairBindingMismatch` | **(a)** Applies as-is whenever games differ publicly; **(b)** Turn-1 D2 duplicate swap requires **Section 4 binding** (`save.game_id` + `seal.save_sha256`) |
| `M04_turn_offset_pad` | Pair `day-10-pre-act.save.json` with `day-11-pre-act.seal`, or truncate/pad `live_state_digests` / `logged_turns` relative to `engine.state().turn`, recomputing outer hashes | Gate 2 & Gate 3 | `TipStateDigestMismatch` | **Applies as-is** |
| `M05_co_written_bug` | Mutate a seat's `cred` or inventory in `seal.engine`, regenerate `.save.json` and `live_state_digests[9]` from the corrupted state, and recompute outer hashes | **Gate 2** | `ReplayDigestMismatch { turn: 10, .. }` (if `live_state_digests[9]` left valid from Turn 10, then `TipStateDigestMismatch`) | **Applies as-is** |
| `M06_mid_log_edit` | **(a)** Flip a byte in `live_state_digests[3]` or mutate a state-affecting Turn-4 action in `logged_turns[3]`; **(b)** Alter non-state-changing field in `logged_turns[3]` while keeping Turn-11 state & `live_state_digests[9]` intact | **Gate 2** | **(a)** `ReplayDigestMismatch { turn: 4, .. }`; **(b)** `ChainHashMismatch { turn: 4, .. }` (against sealed `turn_chain_digest`) | **(a)** Applies as-is; **(b)** Pre-preflight intra-seal `ChainHashMismatch` does not apply unless `turn_chain_digests` is added to `.seal` (not proposed); post-preflight edits hit `OuterFileHashMismatch` / `ChainHashMismatch` against `verified_checkpoints.json` |
| `M07_non_canonical` | Re-serialize `.seal` with pretty-print whitespace, reorder object keys in `.save.json`, append 1 trailing whitespace byte, or inject a duplicate JSON object key | Gate 1, Gate 2, Gate 3 | `NonCanonicalEncoding` | Requires **Section 5 strict decoding** approval |
| `M08_unknown_field` | Insert `"extra_probe_key": 1` at top level of `.save.json`/`.seal` and nested inside `seal.engine.state` | Gate 1, Gate 2, Gate 3 | `UnknownFieldRejected` | Requires **Section 5 unknown-field rejection** approval |
| `M09_engine_skew` | **(a)** Bump `format_version` in `.save.json` or `.seal`, or skew `manifest.engine_build_id`; **(b)** Skew `seal.engine_commit` in `.seal` | Gate 2 & Gate 3 | `IncompatibleEngineOrSchemaVersion` | **(a)** Applies as-is; **(b)** In-seal `engine_commit` requires **Section 4 binding** approval |
| `M10_shared_ref_bleed` | Mutate `base_host` in memory via a `#[cfg(test)]` fault-injection helper during a `league-scorer` batch before post-batch canary check | **Gate 3 Canary** | `InMemoryBaseStateCorrupted` | **Applies via `#[cfg(test)]` fault injection** (spontaneous safe-Rust aliasing cannot occur because `EvalHost` holds only owned collections under `#![forbid(unsafe_code)]`) |

### Save-Load-Step Commutativity Test (`§6`)

Play a deterministic 4-seat `ClubBot` season on `live_host` through close (`>= 12` turns). At every pre-act turn `t`:

1. Serialize `(save_bytes, seal_bytes)` via `live_host.to_save_and_seal_bytes()?`.
2. Verify canonical round-trip bytes and reload `reloaded_host` via `EvalHost::fork_from_verified_checkpoint_bytes(&save_bytes, &seal_bytes)`.
3. Compute identical `ClubBot` submissions (`bot_tie_seed`) and advance **at least one full turn** (`apply_act_submissions` + `sweep_turn`), plus continue to season close on a sampled mid-season turn.
4. Assert exact equality of `reloaded_host.to_save_and_seal_bytes()? == live_host.to_save_and_seal_bytes()?` (valid because `turn_salts` are pre-derived at Turn 0 in `crates/league-core/src/eval/host.rs:299` and `sweep_turn` resets `active_pre_act_checkpoint_sha256 = None` at `:776`), `reloaded_host.engine.state_digest()? == live_host.engine.state_digest()?`, and identical `PublicSweepReceipt`.
5. Include a planted negative control where a synthetic `#[cfg(test)]` transient discrepancy diverges only after one `sweep_turn`, asserting Gate 2 catches it with `CheckpointVerifyError::ProbeStepDivergence`.

Planned test files (respecting crate isolation boundaries in `raw_api_backdoor_guard.rs`):
- `crates/league-core/tests/checkpoint_verify.rs` (`M01`–`M10` core matrix + save-load-step commutativity + `ProbeStepDivergence` negative control)
- `crates/league-scorer/tests/checkpoint_gate3.rs` (Gate 3 worker manifest/hash/digest checks, in-memory `EvalHost::clone` parity vs disk fork, and `M10` canary test)
- `crates/league-eval/tests/verify_checkpoints_cli.rs` (`league-eval verify-checkpoints` manifest creation, read-only permissions, and negative cases)
- `crates/league-core/tests/raw_api_backdoor_guard.rs` (allowlist updates for `eval/checkpoint_verify.rs` and new `EvalHost` pub methods)

---

## 7. Read-Only Finding: Randomness Streams in Counterfactual Rollouts (`§7`)

Inspection of `crates/league-core` and `crates/league-scorer` at `885929c4` establishes how randomness streams behave across counterfactual rollouts. **No code or format is changed for this item.**

### Finding Summary

1. **Within-scenario candidate comparisons (`(c, m)` vs `(c', m)`) already use completely separate, stateless, domain-separated streams.** Choosing a different focal action `c'` on turn `day` never advances a shared PRNG cursor and **cannot shift any later environment draw** (market shocks, closing bell rolls, DEX/election/standings tie-breaks), mandate draw, talk-honour draw, or peer `ClubBot` seed.
2. **Related caveat across scenarios (`m = 0..M`):** All `M` scenarios of a checkpoint currently share the **single base-game `vrf_proofs` sequence** stored in `.seal` (`docs/decision-scorer.md:1230`), meaning environmental outcomes (`t >= day` shocks and closing bell rolls) are pinned to the realized base-game trajectory rather than resampled across `m`. Resampling per-scenario environment trajectories is not required to compare focal candidates under matched draws, and would require a new `ScorerConfig`-gated rollout path (detailed below).

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
