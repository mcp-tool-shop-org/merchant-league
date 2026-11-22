# Evaluation Mode Decision Scorer Design

This document specifies the offline, CPU-only, deterministic decision scorer referenced as `D4` in [`docs/eval-harness.md`](eval-harness.md) for the first Evaluation Mode comparison (`D5`: three arms `off`, `control`, `on` branched at saved pre-act states).

**Scope boundary:** This is a design specification only. It requires **no** changes to `league-core`, to what any model or `ClubBot` seat is shown, or to any persisted Evaluation Mode file (`setup.json`, `checkpoints/`, `to-seat/`, `decisions/`, `outcome.json`, `receipt.json`, `relay/`). Every input named below is already written by `league-eval` and `EvalHost`.

---

## 1. Fixed Requirements

1. **Frozen candidate set including all arms:** Each pre-act state is graded against a frozen set of legal act-phase submissions that includes every evaluated arm's actually committed decision at that state, guaranteeing non-negative regret and never penalizing a strong decision outside an enumerated baseline list.
2. **Horizon to season close:** Values reach terminal season scoring (`total_equity = project_equity + net_asset_equity + mandate_bonus`), avoiding circularity with the `on` arm's single-turn `preview_turn_orders` output.
3. **Counterfactual marginalization:** Scores do not depend on the single realized opponent move vector recorded in the base game's `day-DD-act-recorded.seal`. They average across multiple plausible other-seat decisions (respecting explicit talk-phase trade commitments probabilistically) and consistent hidden-objective assignments.
4. **Deterministic CPU execution:** Pure CPU execution with every pseudo-random choice keyed deterministically from explicit recorded seeds. Identical inputs reproduce bit-identical score records.
5. **Arm-blind scoring kernel:** The scorer input contains only the pre-act checkpoint, the focal seat index, extracted neutral talk-phase trade commitments, anonymized canonical candidates, and the frozen scorer seed/config hash — zero arm labels, wording identifiers, tool call traces, raw reply strings, or reasoning traces.
6. **Pre-main-run freeze:** Calibrated and verified on the D5 pilot (`4` seeds × `6` pre-act states × `3` replicates per arm), then code and parameters are hashed and recorded before any main-run decision is scored.
7. **Phase-invariant primary metric:** Reports a normalized `[0, 1]` score within the state's candidate set alongside raw expected `total_equity` and regret, plus post-scoring per-arm D5 mechanism measures.

---

## 2. Existing Code and Recorded Inputs

All path citations below are verified at `main` (`55fd92a2`):

- **Saved `day-DD-pre-act` checkpoint pair:**
  - `EvalHost::write_pre_act_checkpoint` (`crates/league-core/src/eval/host.rs:1094-1108`) writes `checkpoints/day-DD-pre-act.save.json` and `checkpoints/day-DD-pre-act.seal` atomically inside `league-eval play` right after writing `to-seat/day-DD-act-<Guild>.negotiation.json` (`crates/league-eval/src/driver.rs:2043-2054`) and before rendering any `act` view or reading any `act` reply.
  - `day-DD-pre-act.save.json` holds `EvalSaveSnapshot` (`crates/league-core/src/eval/host.rs:54-121`): `table_id`, `turn`, `phase` (`TurnPhase::TalkAndMarket`), `cancel_locked`, `trade_minister`, `cartel_chair`, `PublicTableSpec`, `parities`, `seats`, resting `offers`, `checks`, `escrows`, `dual_escrows`, `pledges`, `audited_pledges`, `domains`, `council_platforms`, `charters`, `active_shocks`, `public_forecasts`, monotonic IDs, `delivery_ledger`, `last_potentials`, `fill_log`, `pending_wash_freezes`, `eval_vrf_pubkey`, and `public_receipts`.
  - `day-DD-pre-act.seal` holds `EvalSealPayload` (`crates/league-core/src/eval/host.rs:167-188`): the full live `TurnEngine` state, pre-derived `vrf_proofs` for all `1..=HARD_TURN_CAP` turns, `sealed_mandate_deal` (`SealedMandateDeal` with `deal_seed` and all seats' `MandateReveal`s, `crates/league-core/src/commit.rs:212-218`), `logged_turns`, `bot_tie_seed`, and pre-derived per-turn `turn_salts`.
  - `EvalHost::fork_from_checkpoint` (`crates/league-core/src/eval/host.rs:1195-1213`) reconstructs an independent mutable `EvalHost` from the checkpoint pair, while `EvalHost::observe_seat` (`:512-515`) and `EvalHost::mandate_for_seat` (`:521-527`) expose seat observations and own mandates without mutating state.
- **Turn resolution and season close in `league-core`:**
  - `EvalHost::apply_act_submissions(&mut self, submissions: &BTreeMap<u8, EvalActSubmission>)` (`crates/league-core/src/eval/host.rs:631-734`) applies each seat's Phase-1 Pass A `market_actions` (`EvalActSubmission` at `:126-134`), triggers `begin_cancel_lock()`, applies optional Pass B actions (empty for model seats), probes each seat's Phase-2 `SealedOrderBundle` reveal (falling back to `SealedOrderBundle::default()` and recording `refused_bundle_code` if reveal validation fails at `:716-727`), and commits/reveals all bundles simultaneously (`commit_and_reveal_all`, `:731`).
  - `EvalHost::sweep_turn(&mut self)` (`:739-778`) executes the deterministic 8-step `EvalVrf` sweep using the host's pre-derived VRF proof for that turn and returns `PublicSweepReceipt`.
  - Once `phase() == TurnPhase::AwaitingMandateReveal` (up to `HARD_TURN_CAP = 32`), `EvalHost::finalize_eval_season(&mut self)` (`:783-840`) reveals all Turn-1 mandates, calls `TurnEngine::finalize_season` (`crates/league-core/src/engine.rs:1577`) -> `compute_final_scoreboard` (`crates/league-core/src/scoring.rs:545-617`), and verifies replay digests. For each seat, terminal `SeatScoreEntry::total_equity` (`scoring.rs:582-591`) equals `project_equity + net_asset_equity + mandate_bonus`.
- **`ClubBot::decide_full_turn` and seeding:**
  - `ClubBot::new(bot_tie_seed)` (`crates/league-core/src/bot.rs:40-42`) stores a single `u64` `bot_tie_seed`. `ClubBot::decide_full_turn(&self, obs: &SeatObservation, own_mandate: &PrivateMandate)` (`:102-114`) deterministically plans `pass_a_actions`, `pass_b_actions`, and `bundle` (`ClubBotDecision`, `:22-29`), breaking vote tie-breaks via `(seat + 1 + ((self.bot_tie_seed ^ u64::from(obs.turn)) as u8 % (n_u8 - 1))) % n_u8` (`:536`). Because `own_mandate` is an explicit parameter (`:105`), rollouts can condition peer `ClubBot` instances on any counterfactual mandate without mutating `EvalHost`'s cryptographic commitments.
- **Pessimistic preview candidate set (`crates/league-core/src/observation.rs:532`):**
  - Inside `preview_what_if` (`observation.rs:503-537`), `PreviewTieMode::PessimisticForSeat` (`:513-535`) evaluates a **single** caller-supplied `HumanWhatIfRequest` across 8 derived tie-break beacons (`seed_idx in 0..8`) and picks the worst `(post_potential, elected_seat, seed_idx)` outcome (`"pessimistic preview candidate set cannot be empty"` at `:532`). It is a tie-break stress test for one draft, **not** an action-space generator. For legality checking and fast own-seat pruning when building action candidates, the scorer uses `preview_seat_bundle` (`observation.rs:542-566`).
- **Recorded `decisions/*.json` (`D4`), `branch` outputs, and relay sidecars:**
  - `EvalDecisionRecord` (`crates/league-eval/src/records.rs:328-432`), written to `decisions/day-DD-talk-<Guild>.json` (`driver.rs:2038`), `decisions/day-DD-act-<Guild>.json` (`driver.rs:2184`), and per-arm `<out_dir>/<arm>/decision.json` + `outcome.json` (`BranchArmOutcome`, `records.rs:601-618`, `driver.rs:2936-2939`), records: `arm`, `seat`, `guild`, `raw_reply`, `discarded_reply_path`, `discarded_reply_sha256`, `attempts`, `failed_attempts`, `prompt_tokens`, `completion_tokens`, `wall_secs`, `tool_calls_used`, `parse_failure`, `refused_market_actions`, `refused_bundle_legs`, `cleared_non_empty_actions`, `intermediate_bundle_sha256s`, `prompt_token_ids_sha256s`, `reply_token_ids`, `reply_top_logprobs`, `refusal_code`, `apply_report`, `refused_bundle`, and `club_bot_baseline`.
  - When `--preview-tool on` or `--preview-tool control` ran, `SavedToolRoundRecord` sidecars (`relay/seats/<Guild>.day-DD-act.tool-round-*.json`, `crates/league-relay/src/relay.rs:2685-2718`, consumed at `driver.rs:714-767`) persist every `SavedToolRoundCall::arguments` string (`relay.rs:2648-2677`) alongside `SavedHttpReplyRecord` (`relay.rs:2801-2844`, sidecar path `:2848-2859`) which retains raw HTTP JSON responses (including model reasoning fields/text). Negotiation inputs delivered to each seat on `act` live in `to-seat/day-DD-act-<Guild>.negotiation.json` (`NegotiationInputs { batna, whispers }`, `crates/league-relay/src/wording.rs:279-285`, written at `driver.rs:2048` and verified in `branch` at `driver.rs:2675`). Committed `act` submissions are recovered deterministically from `raw_reply` via `parse_eval_act_reply` (`crates/league-eval/src/reply.rs:518-566`, falling back to `EvalActSubmission::default()` on `no_reply`).
- **Full-season CPU rollout feasibility and measured timing:**
  - `eval_host_turn_0_fork_deals_identical_mandates_and_runs_independently_to_eval_replay` (`crates/league-core/tests/eval_host.rs:447-515`) and `item10_full_32_turn_clubbot_season_exports_verified_replay_log` (`crates/league-eval/tests/play_and_branch.rs:414-476`) already run 4-seat `ClubBot` rollouts through `HARD_TURN_CAP = 32` to `finalize_eval_season` on CPU.
  - Measured in `--release` on this codebase (`cargo test --release --locked --jobs 2 -p league-core --test eval_host eval_host_turn_0_fork_deals_identical_mandates_and_runs_independently_to_eval_replay -- --exact`): **`0.10s` (`100 ms`)** total wall time for turn-0 save+seal disk writes, two checkpoint forks, a single-turn fork step, a full 32-turn `ClubBot` rollout, `finalize_eval_season` (which internally runs both `replay_eval_season` and `replay_season_with_state_predicates`), JSON replay serialization, and an external replay verification. Therefore `0.10s` (`100 ms`) per full-season rollout is a conservative `--release` CPU upper bound.

---

## 3. Frozen Candidate Set

For each `(state_id, pre_act_checkpoint, focal_seat)`, an unblinded builder constructs a single deduplicated candidate set `C(state)` across all arms and replicates associated with that state before blinding:

1. **Mandatory passive baseline:** `EvalActSubmission::default()` (empty `market_actions`, empty `pass_b_actions`, empty `SealedOrderBundle`), representing `NO_REPLY` / inaction.
2. **All evaluated arms' committed decisions:** For every replicate and arm (`off`, `control`, `on`) at `state_id`, parse its `decision.json` (`raw_reply` -> `parse_eval_act_reply`, or `EvalActSubmission::default()` when `no_reply == true` / whole-reply refusal) using exact submitted `market_actions` and `bundle` (`pass_b_actions` empty for model seats), and include it unconditionally. Passing it through `EvalHost::apply_act_submissions` (`eval/host.rs:631-734`) replicates the engine's exact handling of partial Phase-1 refusals and whole-bundle reveal fallback.
3. **First accepted preview/draft decision per arm/replicate:** If any `SavedToolRoundRecord` call in an arm/replicate accepted a `preview_turn_orders` or `draft_turn_orders` bundle matching `intermediate_bundle_sha256s[0]`, parse `SavedToolRoundCall::arguments` into an `EvalActSubmission` and include it in `C(state)` so draft-to-commit revision gain uses the exact same shared rollout estimator.
4. **Deterministic synthetic baseline & search candidates (up to `K_synth_max = 32` slots):** Generated strictly from `SeatObservation` and `own_mandate` as `EvalActSubmission` values:
   - `ClubBot` baselines: `ClubBot::new(bot_tie_seed + k).decide_full_turn(&obs, &own_mandate)` for a fixed small bank of tie seeds (`k in 0..4`), plus variants with `pass_b_actions` cleared to match the model-seat single-pass interface.
   - Single-axis enumerations from `obs`: full/partial crossing of top-of-book resting `CredOffer`s (`MarketAction::PostOffer`), parity-anchored quote postings (`±5%`, `±10%`), maximum feasible `ConverterRunOrder` batches ranked by `parities`, maximum coverable `ProjectDeliveryOrder` batches (`obs.spec`), `TownImport` deficit coverage for upkeep or stalled converter inputs, voluntary debt repayment (`debt_repayment_cred`), maturing `CheckCash` / `EscrowFinish` claims, and election/charter bids on `turn % 4 == 0` / `turn % 6 == 0`.
   - Composite greedy combinations and local perturbations (scaling quantities `{50%, 100%}`, dropping highest-upkeep-risk legs), each screened with `preview_seat_bundle` (`crates/league-core/src/observation.rs:542-566`) to discard immediately illegal or self-bankrupting synthetic drafts before filling up to `K_synth_max = 32`.
5. **Canonical deduplication and freezing:** Each `EvalActSubmission` is hashed by canonical compact JSON bytes (`candidate_id = sha256_hex(serde_json::to_vec(&sub))`). Identical submissions across arms, replicates, or synthetic generators collapse to one entry. `C(state)` is sorted ascending by `candidate_id` with all origin metadata stripped, capped at at most `K_synth_max = 32` synthetic candidates plus all unique arm committed/draft submissions (so any finite number of outside-set model decisions is always retained). The complete sorted list `C(state)` is serialized into `BlindScoringRequest` and SHA-256 frozen before scoring begins.

---

## 4. Other Seats: Act-Phase Sampling and Talk-Phase Commitments

Instead of conditioning on the single realized peer action vector inside `day-DD-act-recorded.seal`, each state evaluates `M = 16` independent counterfactual scenarios `m in 0..M` (shared as Common Random Numbers across every candidate `c in C(state)`):

1. **Deterministic act-phase policy mix:** For each non-focal seat `s != focal_seat`, scenario `m` instantiates `ClubBot::new(peer_seed(scorer_seed, pre_act_pair_sha256, day, m, s))` conditioned on `(obs_s, mandate_{m,s})`. To capture strategic dispersion across scenarios without leaving `league-core`'s pure observation boundary, each `(m, s)` deterministically selects one of three rule-based postures parameterized over `ClubBot::decide_full_turn`:
   - *Standard `ClubBot`* (scenarios `m % 4 in {0, 1}`): unmodified `decide_full_turn(&obs_s, &mandate_{m,s})`.
   - *Liquidity/converter active* (`m % 4 == 2`): retains `decide_full_turn` bundle, adding deterministic crossing `PostOffer` actions for idle excess `Raw`/`Refined` inventory above upkeep reserves when resting book spreads exceed parity bounds.
   - *Conservative/upkeep-first* (`m % 4 == 3`): retains `decide_full_turn` while suppressing speculative resting offer posts that lock `CRED` or goods below `2 turns` of upkeep cover.
   Only Pass A (`market_actions`), Pass B (`pass_b_actions`), and `bundle` matching `s`'s posture are emitted.
2. **Talk-phase trade commitments and conditional compliance:**
   - **Offline extraction (before blinding):** A frozen deterministic parser scans each seat's `to-seat/day-DD-act-<Guild>.negotiation.json` (`NegotiationInputs::whispers`, checking `intent_header` first and structured keyword/number patterns in `body`: counterpart guild, `AssetId`, positive quantity, unit `CRED` price, direction buy/sell/swap) alongside outgoing messages parsed by `parse_eval_talk_reply` (`crates/league-eval/src/reply.rs:577`) from `decisions/day-DD-talk-<Guild>.json`. Bilateral commitments require a concrete unambiguous good, quantity, price/counter-asset, and recipient guild without conflicting retractions later in the same message.
   - **Compliance in rollouts:** Extracted commitments are stored neutrally in `BlindScoringRequest::talk_commitments` (`proposer_seat`, `counterparty_seat`, `offered_action_template`). For each scenario `m` and non-focal seat `s` with an active commitment involving `focal_seat` (or another peer), the scenario PRNG draws `honour_{m,s} ~ Bernoulli(0.75)` independently per commitment. When `honour_{m,s} == true`, the matching legal `MarketAction` (e.g., matching `PostOffer` / escrow / check step) is prepended ahead of `ClubBot`'s `pass_a_actions` (pruned via `preview_seat_bundle` if unaffordable). This preserves strategic counterparty commitment variance across scenarios while grading deal-keeping consistently.

---

## 5. Hidden-Objective Sampling

At Turn 1, `default_graded_mandate_deck(4)` (`crates/league-core/src/commit.rs:124-204`) constructs `8` distinct `GradedMandateCard` mandates covering all four `MandateKind` variants (`MaintainCleanCredit`, `MinProjectEquity`, `MinRefinedVariety`, `MinActivePartners`), and deals `4` distinct mandates without replacement (`deal_mandate_deck_indices`, `:281-310`).

Conditioned on what the focal seat can legally know at `day-DD-pre-act`:
- The focal seat knows its own dealt mandate (`focal_mandate = *host.mandate_for_seat(Seat(focal_seat))?`, `eval/host.rs:521-527`).
- In scenario `m in 0..M`, the scorer identifies the `7` mandates in `default_graded_mandate_deck(4)` not equal to `focal_mandate` (preserving deck-target parameters from `host.public_spec()`) and draws an ordered `3`-tuple without replacement (`210` equiprobable permutations) for the `3` peer seats using SHA-256 reverse Fisher-Yates seeded by `(scorer_seed, pre_act_pair_sha256, day, m)`. Notice `PublicSweepReceipt`s and resting actions remain uninformative-to-noisy clues, so uniform-over-7 without replacement (stratified across permutations when `M = 16`) avoids uncalibrated posterior overfitting while remaining consistent with the true deal space (`4` distinct mandates at the table).
- **Exact focal scoring interaction:** In `compute_final_scoreboard` (`crates/league-core/src/scoring.rs:545-617`), `total_equity` for `focal_seat` sums `project_equity + net_asset_equity + mandate_bonus(focal_seat)`, where `mandate_bonus` uses `pre_mandate_nws` median (`:557-563`) — meaning counterfactual peer mandates shape peer `ClubBot` market/converter/delivery play throughout `day..=close` (`m`'s counterparty environment), while `EvalHost::finalize_eval_season()` unseals the host's authentic `mandate_commits[focal_seat]` and awards the focal seat's true hidden objective bonus.

---

## 6. Horizon Valuation and CPU Cost Estimates

### 6.1 Season-Close Rollout Procedure

For each scenario `m in 0..M` and each candidate `c in C(state)`:

1. Clone a pre-forked `EvalHost` loaded from `day-DD-pre-act.{save.json,seal}` (`EvalHost::fork_from_checkpoint`, `eval/host.rs:1195-1213`).
2. Construct the turn-`day` `BTreeMap<u8, EvalActSubmission>` by inserting `c` at `focal_seat` and scenario `m`'s counterfactual peer submissions at `s != focal_seat`. Call `host.apply_act_submissions(&subs)` (`:631-734`) followed by `host.sweep_turn()` (`:739-778`). (Peer submissions for turn `day` are computed once per `(state, m)` before cloning across `c in C(state)`.)
3. While `host.phase() == TurnPhase::TalkAndMarket` (up to `HARD_TURN_CAP = 32`):
   - Observe every seat (`host.observe_seat(Seat(s))`, `:512-515`).
   - Run `ClubBot::new(rollout_bot_seed(scorer_seed, pre_act_pair_sha256, host.turn(), m, s)).decide_full_turn(&obs_s, &role_mandate)` (`bot.rs:102-114`), passing `focal_mandate` for `focal_seat` and `mandate_{m,s}` for peers.
   - Apply submissions and `sweep_turn()`.
4. Call `host.finalize_eval_season()` (`:783-840`) to reach `TurnPhase::Completed`, extract `scoreboard.standings` entry for `focal_seat`, and record `(total_equity, project_equity, net_asset_equity, mandate_bonus)`.

Each `(c, m)` rollout runs to the natural close turn (driven by the host's deterministic closing hazard roll embedded in `vrf_proofs` inside `.seal`, matching the exact same remaining-horizon dynamics for all candidates `c in C(state)` via CRN).

### 6.2 CPU Cost Estimates (Anchored to Empirical `--release` Measurement)

From the empirical measurement on `55fd92a2` (`eval_host_turn_0_fork_deals_identical_mandates_and_runs_independently_to_eval_replay`, `--release`), one complete 4-seat `ClubBot` rollout over `32` turns plus `finalize_eval_season`, disk I/O, and replay checks finishes in **`0.10s` (`100 ms`)** single-core. Using `100 ms` per `(c, m)` rollout as a conservative upper bound (mid- and late-stage checkpoints execute strictly fewer remaining turns):

- Candidates per state: conservatively `K = 35` (`32` synthetic/enumerated + up to `3` distinct arm/draft submissions deduplicated into `C(state)`; note that `3` replicates within an arm/state only add rollouts when replicates emit distinct submissions or first drafts not already in `C(state)`).
- Scenarios per state: `M = 16`.
- Upper-bound rollouts per state: `K * M = 35 * 16 = 560` rollouts.

| Scope | States (`C(state)` evaluations) | Max Rollouts (`K * M`) | Single-Core Time (`@ 100 ms/rollout`) | 8-Core Wall Time (embarrassingly parallel across states/candidates) |
|---|---|---|---|---|
| **Single State** | `1` | `560` | `56.0 s` (`0.93 CPU-min`) | `~7.0 s` |
| **D5 Pilot** (`4` seeds × `6` states × `3` reps/arm) | `24` (replicates share state rollout cache when `C(state)` includes all `72` arm decisions) | `13,440` (or `<= 18,432` even if all `72` committed decisions + unique drafts are distinct) | `22.4 CPU-min` (up to `30.7 CPU-min` unshared worst case) | `~2.8 – 3.8 min` |
| **D5 Main Run** (`20` seeds × `6` states, `D5`) | `120` | `67,200` | `112.0 CPU-min` (`1.87 CPU-hours`) | `~14.0 min` |

*Estimation basis:* `states × K × M × 0.10 s` (1 core), divided by `8` for 8 independent worker threads over rayon/threadpool with immutable shared read-only state setup. Pilot timing cleanly calibrates actual per-state latency and Monte Carlo standard error before freezing `K` and `M`.

---

## 7. Outputs and Pre-Registered Mechanism Measures

### 7.1 Blind Scorer Outputs (Per `state_id` and Per `candidate_id`)

For every candidate `c in C(state)` across scenarios `m in 0..M`:
- `ev_total_equity`: mean of terminal `total_equity(focal_seat)` across `m = 0..M` (plus mean `project_equity`, `net_asset_equity`, `mandate_bonus`).
- `v_min` and `v_max`: minimum and maximum `ev_total_equity` across all `c in C(state)`.
- **Primary normalized phase-invariant score (`score_norm`):**
  - If `v_max > v_min`: `(ev_total_equity(c) - v_min) / (v_max - v_min)` in `[0.0, 1.0]`.
  - If `v_max == v_min`: `1.0` with `degenerate_flat_set: true` (flagged explicitly for downstream diagnostics).
- **Regret (`regret_equity`):** `v_max - ev_total_equity(c) >= 0.0` (strictly non-negative because every committed arm decision belongs to `C(state)`).
- **CRN paired Monte Carlo standard error (`paired_se_equity`):** sample standard error across `m in 0..M` of the paired difference `(V_best(m) - V_c(m))`, plus pairwise arm contrasts `EV(on) - EV(off)`, `EV(on) - EV(control)`, and `EV(control) - EV(off)` with paired per-scenario SEs computed after unblinding.
- **Anchor reference:** `club_bot_baseline_score_norm` (score of the deterministic default `ClubBot::decide_full_turn` candidate included in `C(state)`).

### 7.2 Unblinded D5 Pre-Registration Mechanism Measures

Joined post-scoring by `(state_id, arm, replicate)` using `arm_map`, `decisions/`, `outcome.json`, and `relay/seats/` sidecars (no scorer re-run required):

1. **Tool-call rate:** Mean `tool_calls_used` per decision (`EvalDecisionRecord::tool_calls_used`, `records.rs:375`) and fraction of rounds with `tool_calls_used >= 1`.
2. **Draft-to-commit revision gain:** For each decision with at least one valid accepted preview/draft (`!intermediate_bundle_sha256s.is_empty()`), match `intermediate_bundle_sha256s[0]` (`records.rs:390`) against `SavedToolRoundRecord::calls` (`relay.rs:2685-2718`) to obtain `first_draft_candidate_id` (already evaluated inside `C(state)`), and report `score_norm(committed) - score_norm(first_draft)` (and raw `ev_total_equity(committed) - ev_total_equity(first_draft)`). Evaluated identically for both `control` (`draft_turn_orders`) and `on` (`preview_turn_orders`) since both emit `intermediate_bundle_sha256s` (`eval-harness.md:55`), reporting `n_with_draft` separately.
3. **Reasoning length:** Total character length and server token count (`completion_tokens` / sum of `reply_token_ids` lengths from `EvalDecisionRecord` and raw reasoning extracted from `SavedHttpReplyRecord::body`, `relay.rs:2830`) across HTTP rounds in the `act` decision.
4. **Illegal-order rate:** Fraction of decisions with reply-level refusal (`parse_failure` or `refusal_code.is_some()`, covering `ACT_REPLY_INVALID`, `ACT_REPLY_HAS_MESSAGES`, `NO_REPLY`), mean refused Phase-1 actions (`refused_market_actions`, `records.rs:381`), and fraction with whole-bundle reveal rejection (`refused_bundle.is_some()` / `refused_bundle_legs > 0`, `records.rs:384,428`).
5. **Talk-phase deal-keeping:** Across `decisions/day-DD-talk-<Guild>.json` (`driver.rs:2038`, `raw_reply` parsed via `parse_eval_talk_reply` at `reply.rs:577`) and `to-seat/day-DD-act-<Guild>.negotiation.json` (`view.rs:48`), the frozen deterministic rule-based extractor identifies explicit trade commitments made or accepted by `focal_seat` in `talk`. An `act` submission honours a commitment when the matching counterparty order (`PostOffer` at agreed price/side/qty, `CreateEscrow`, `DualEscrowProposal`, or `AtomicSwapProposal`) appears in its submitted `market_actions` or `bundle`. Reported as `honoured_commitments / applicable_commitments` with `applicable_commitments` stated separately (never imputing `0/0`).

---

## 8. Blinding and Freezing Protocol

### 8.1 Exact Blind Input Schema (`BlindScoringRequest`)

The offline unblinded adapter extracts inputs into a staging directory containing two disjoint trees: `blind_inputs/<state_id>.json` and `arm_map/<state_id>.json`. The scoring crate reads **only** `blind_inputs/`.

Each `blind_inputs/<state_id>.json` serializes strictly these fields:

```text
BlindScoringRequest {
  schema_version: u32,                    // fixed = 1
  state_id: String,                       // canonical "<source_game_id>:day-<DD>:seat-<idx>" (no arm)
  pre_act_save_path: PathBuf,             // copied or content-addressed checkpoints/day-DD-pre-act.save.json
  pre_act_seal_path: PathBuf,             // copied or content-addressed checkpoints/day-DD-pre-act.seal
  pre_act_pair_sha256: String,            // 64-hex digest matching compute_pre_act_pair_sha256_hex
  focal_seat: u8,                         // 0..4
  talk_commitments: Vec<ExtractedTalkCommitment>, // neutral counterfactual commitment specs (no raw text)
  candidates: Vec<BlindCandidate>,        // sorted ascending by candidate_id; duplicates merged
  scorer_seed: u64,                       // explicit recorded seed
  scorer_config_sha256: String,           // SHA-256 of canonical ScorerConfig JSON
}

BlindCandidate {
  candidate_id: String,                   // sha256_hex(serde_json::to_vec(&submission))
  submission: EvalActSubmission,          // market_actions, pass_b_actions, bundle only
}
```

Removed/excluded from `BlindScoringRequest` and `blind_inputs/`:
- `arm` (`off` / `control` / `on`), replicate index, `wording_arm`, branch output paths (`<out>/<arm>/...`), `raw_reply`, `discarded_reply_path`, `tool_calls_used`, `intermediate_bundle_sha256s` order labels, `<Guild>.jsonl` transcripts, `<Guild>.scratchpad.md`, `http-reply-round-*.json`, and `tool-round-*.json`.
- Notice all checkpoint paths in `BlindScoringRequest` come from the **source** `day-DD-pre-act` checkpoint (shared identically across all three arms at `state_id`), never per-arm branch output directories.

Separately, `arm_map/<state_id>.json` stores `Vec<{ arm, replicate, committed_candidate_id, first_draft_candidate_id, decision_path, outcome_path }>` for post-scoring unblinding in reporting.

### 8.2 Freeze Procedure Before Main Run

1. Run pilot (`4` seeds × `6` states × `3` replicates per arm): build `blind_inputs/` and `arm_map/`, execute `league-scorer` twice on separate CPU passes to assert bit-identical `scores/` JSON digests, inspect candidate-set coverage (`V_max > V_min` rate, `paired_se_equity` precision, runtime within budget), and finalize parameters (`K_synth_max`, `M`, posture weights, `Bernoulli(0.75)` honour parameter, talk-extractor patterns).
2. Commit final scorer code and default config; reject any dirty git worktree when freezing.
3. Generate and store `scorer_freeze.json` in the experiment ledger **before** scoring any main-run decision:
   - `scorer_git_sha` (40-hex commit SHA of the workspace)
   - `scorer_tree_sha256` (hash of `crates/league-scorer/**`)
   - `scorer_config_sha256` (canonical SHA-256 of frozen `ScorerConfig`)
   - `talk_extractor_version` & regex/rule table digest
   - `blind_inputs_manifest_sha256` (sorted SHA-256 manifest of main-run `blind_inputs/*.json`, committed before any `scores/*.json` write)
4. Verify during main-run scoring that `ScorerConfig` SHA-256 matches `scorer_freeze.json::scorer_config_sha256`.

---

## 9. Crate Placement (`crates/league-scorer`)

**Decision:** Place the blind scoring engine in a new dedicated workspace crate **`crates/league-scorer`** (`library` + `league-scorer` binary), accompanied by thin unblinded orchestration hooks in `crates/league-eval` (`prepare-scores` / `report` subcommand integration).

**Rationale over alternatives:**
- *Why not directly inside `league-eval`?* `league-eval` already links `EvalArm`, `EvalDecisionRecord`, `BranchArmOutcome`, `league-relay`, HTTP transcripts, and arm directories. Separating `league-scorer` enforces blinding **at compile time**: `crates/league-scorer/Cargo.toml` depends strictly on `league-core`, `league-seat-types`, `serde`, `serde_json`, `sha2` (and optional `rayon` for deterministic parallel reduce), with **zero** dependency on `league-eval` or `league-relay`. It cannot read arm labels or tool telemetry types.
- *Why not an external standalone tool (e.g., Python)?* Cross-language FFI or subprocess JSON RPC across `~67,200` 32-turn `EvalHost` rollouts would add serialization overhead and diverge from `league-core`'s `EngineState`/`EvalHost` invariants. A Rust workspace crate reuses `EvalHost::fork_from_checkpoint`, `apply_act_submissions`, `sweep_turn`, `finalize_eval_season`, `ClubBot::decide_full_turn`, and `preview_seat_bundle` directly in `--release`, inherits `[workspace.lints]` (`unsafe_code = "forbid"`, `missing_docs = "warn"`), and builds offline under `--locked`.

---

## 10. Known Limits

1. **Surrogate rollout policy (`ClubBot`) vs model continuation:** From turn `day` forward, both peer counterfactuals and post-branch continuation play `ClubBot::decide_full_turn` (`crates/league-core/src/bot.rs:102`) plus simple posture perturbations. If an action's real value hinges on complex multi-turn deception or linguistic persuasion not modeled by `ClubBot`, the rollout understates or mis-weights that option. Conversely, every candidate `c in C(state)` faces the same opponent model (`CRN`), keeping within-state comparison internally consistent.
2. **Fixed base-game VRF future beacons across counterfactuals:** `EvalHost::fork_from_checkpoint` restores `vrf_proofs` from the base game's `.seal`, so future turn closing rolls (`close_roll_bps_from_beacon`) and shocks along turn `t > day` are deterministic functions of the original game rather than freshly resampled per rollout. All candidates share identical hazard timing (`CRN`), which lowers variance at the cost of conditioning on one season hazard trajectory.
3. **Talk-deal parser recall on implicit diplomacy:** Deterministic pattern matching over `decisions/day-DD-talk-*.json` and `to-seat/day-DD-act-<Guild>.negotiation.json` captures explicit structured proposals (`intent_header`, explicit quantities/prices/assets) but will miss vague hints or indirect conditional promises. Because the extractor is blind and identical across all three arms, precision stays high while any under-capture is symmetric across arms (reported alongside raw `applicable_commitments`).
4. **Span sensitivity of `[0, 1]` normalization:** Normalization `(EV(c) - v_min) / (v_max - v_min)` can amplify small raw differences on low-variance states (`v_max - v_min` small) or compress differences when one extreme candidate performs very poorly (e.g., `NO_REPLY` causing upkeep default). For this reason the scorer always reports raw `ev_total_equity`, `regret_equity`, `degenerate_flat_set`, and `club_bot_baseline_score_norm` alongside normalized scores.
5. **Single-state branching vs whole-season compounding:** Branching evaluates isolated pre-act deviations on turns drawn by `league-eval select` (`early`/`mid`/`late`). Downstream strategy shifts on turns `day + 1..` revert to `ClubBot` rather than an interactive LLM, so cumulative multi-turn learning or memory effects across turns are outside single-branch regret and must be read alongside whole-season `play` metrics (`docs/eval-harness.md` `D5`).
6. **`EvalHost::finalize_eval_season` replay overhead:** Calling `finalize_eval_season` (`crates/league-core/src/eval/host.rs:783-840`) unmodified runs both `replay_eval_season` and `replay_season_with_state_predicates` on every rollout. Empirical `--release` timing shows total runtime remains under `2` CPU-hours (`~14` min on `8` cores) for the full `120`-state D5 run, so keeping `league-core` completely untouched outweighs optimizing this loop now.
