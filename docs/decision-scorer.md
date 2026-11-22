# Evaluation Mode Decision Scorer Design

This document specifies the offline, CPU-only, deterministic decision scorer referenced as `D4` in [`docs/eval-harness.md`](eval-harness.md) for the first Evaluation Mode comparison (`D5`: three arms `off`, `control`, `on` branched at saved pre-act states).

**Scope boundary:** This is the design specification for the decision scorer. Stage S1 of the blind scoring kernel is implemented in [`crates/league-scorer`](../crates/league-scorer/README.md) (schema v1 validation, §5.2 `ScorerConfig`, Standard-posture undealt-mandate rollouts, §7.1/§3.2 scoring outputs, and the content-addressed CLI; S2 candidate generators/non-standard postures/talk compliance, S3 unblinded `league-eval` builder/`arm_map/`/freeze join gate, and S4 optional parallelism remain future stages). It requires **no** changes to `league-core`, to what any model or `ClubBot` seat is shown, or to any persisted Evaluation Mode file (`setup.json`, `checkpoints/`, `to-seat/`, `decisions/`, `outcome.json`, `receipt.json`, `relay/`). Every input named below is already written by `league-eval` and `EvalHost`.

---

## 1. Fixed Requirements

1. **Frozen candidate set including all committed arm decisions:** Each pre-act state is graded against a frozen set of legal act-phase submissions whose normalization span (`[v_min, v_max]`) contains passive, synthetic baselines, and every evaluated arm's actually committed decision at that state, guaranteeing non-negative regret for committed decisions and never penalizing a strong decision outside an enumerated baseline list. First drafts are evaluated on the same scenarios for revision gain, outside `[v_min, v_max]`.
2. **Horizon to season close:** Values reach terminal season scoring (`total_equity = project_equity + net_asset_equity + mandate_bonus`), avoiding circularity with the `on` arm's single-turn `preview_turn_orders` output.
3. **Counterfactual marginalization:** Scores do not depend on the single realized opponent move vector recorded in the base game's `day-DD-act-recorded.seal`. They average across multiple plausible other-seat decisions (respecting explicit talk-phase trade commitments probabilistically) and uniform undealt-mandate assignments.
4. **Deterministic CPU execution:** Pure CPU execution with every pseudo-random choice keyed deterministically from explicit recorded seeds, index-ordered reductions, integer sums before final division, and no thread-count dependence, timestamps, or paths inside score records or hashed blind requests. Identical inputs reproduce bit-identical score records.
5. **Arm-blind scoring kernel & `arm_map/` governance:** The scorer reads only `blind_inputs/` (the pre-act checkpoint hashes, focal seat index, extracted neutral talk-phase trade commitments, anonymized canonical candidates with span/independence flags, and frozen scorer seed/config hash) — zero arm labels, replicate counts, wording identifiers, tool call traces, raw reply strings, or reasoning traces. `arm_map/` is held strictly by the run operator's reporting step, separate from `league-scorer` and from anyone tuning parameters.
6. **Pre-main-run freeze, arm-blind pilot tuning, and single control arm:** Calibrated and verified on the D5 pilot (`4` seeds × `6` pre-act states = `24` states × `3` arms × `3` replicates = `216` arm decisions) using **arm-blind diagnostics only** (`paired_se_equity`, measured mid/late rollout runtime, flat-set rate, arm-independent candidate coverage, and bit-identical repeat passes). Neither any person nor any tool joins `arm_map/` to any score until `scorer_freeze.json` is written and recorded, and `scorer_freeze.json`'s SHA-256 is recorded in the first run's pre-registration before any main-run decision is scored (later implementation must include an automated test proving any `arm_map/` join without a recorded freeze file is refused). Both pilot and main runs MUST use the same `control` wording arm: the `r2` control arm that runs the same legality check as `on` (being built separately), recorded as `control_wording_arm` in `scorer_freeze.json` and checked solely by the unblinded input builder and reporting step outside `league-scorer`.
7. **Phase-invariant primary metric plus sensitivity score:** Reports a normalized `[0, 1]` score within `[v_min, v_max]` (`NA` / `null` when `v_max == v_min`, excluded upfront by the arm-blind pre-screen) alongside raw expected `total_equity`, regret, Winner's-curse disclosure, a pre-registered arm-independent rank/percentile sensitivity score, and descriptive-only post-unblinding mechanism measures.

---

## 2. Existing Code and Recorded Inputs

All path citations below are verified at `main` (`55fd92a2`):

- **Saved `day-DD-pre-act` checkpoint pair:**
  - `EvalHost::write_pre_act_checkpoint` (`crates/league-core/src/eval/host.rs:1094-1108`) writes `checkpoints/day-DD-pre-act.save.json` and `checkpoints/day-DD-pre-act.seal` atomically inside `league-eval play` right after writing `to-seat/day-DD-act-<Guild>.negotiation.json` (`crates/league-eval/src/driver.rs:2043-2054`) and before rendering any `act` view or reading any `act` reply.
  - `day-DD-pre-act.save.json` holds `EvalSaveSnapshot` (`crates/league-core/src/eval/host.rs:54-121`): `format_version`, `layout_seed`, `table_id`, `turn`, `phase` (`TurnPhase::TalkAndMarket`), `cancel_locked`, `trade_minister`, `cartel_chair`, `PublicTableSpec` (`spec`), `parities`, `seats`, resting `offers`, `checks`, `escrows`, `dual_escrows`, `pledges`, `audited_pledges`, `domains`, `council_platforms`, `charters`, `active_shocks`, `public_forecasts`, monotonic IDs (`next_offer_id`, `next_check_id`, `next_escrow_id`), `delivery_ledger`, `last_potentials`, `pending_town_import_cred_spent`, `fill_log`, `pending_wash_freezes`, `eval_vrf_pubkey`, and `public_receipts`.
  - `day-DD-pre-act.seal` holds `EvalSealPayload` (`crates/league-core/src/eval/host.rs:167-188`): the full live `TurnEngine` state, pre-derived `vrf_proofs` for all `1..=HARD_TURN_CAP` turns, `sealed_mandate_deal` (`SealedMandateDeal` with `deal_seed` and all seats' `MandateReveal`s, `crates/league-core/src/commit.rs:212-218`), `logged_turns`, `bot_tie_seed`, and pre-derived per-turn `turn_salts`.
  - `EvalHost` (`crates/league-core/src/eval/host.rs:197`) does **not** implement `Clone` (manual `Debug` only at `:217`), and `from_save_and_seal_bytes` is `pub(crate)` (`:955`). Each rollout reconstructs an independent host from disk via `EvalHost::fork_from_checkpoint` (`:1195-1213`), while `EvalHost::observe_seat` (`:512-515`) and `EvalHost::mandate_for_seat` (`:521-527`) read observations and dealt mandates without mutating state.
- **Turn resolution and season close in `league-core`:**
  - `EvalHost::apply_act_submissions(&mut self, submissions: &BTreeMap<u8, EvalActSubmission>)` (`crates/league-core/src/eval/host.rs:631-734`) applies Phase-1 Pass A `market_actions` (`EvalActSubmission` at `:126-134`), triggers `begin_cancel_lock()`, applies optional Pass B actions (empty for model seats), probes each seat's Phase-2 `SealedOrderBundle` reveal (falling back to `SealedOrderBundle::default()` and recording `refused_bundle_code` if reveal validation fails at `:716-727`), and commits/reveals all bundles simultaneously (`commit_and_reveal_all`, `:731`).
  - `EvalHost::sweep_turn(&mut self)` (`:739-778`) executes the deterministic 8-step `EvalVrf` sweep using the host's pre-derived VRF proof for that turn and returns `PublicSweepReceipt`.
  - Once `phase() == TurnPhase::AwaitingMandateReveal` (up to `HARD_TURN_CAP = 32`), `EvalHost::finalize_eval_season(&mut self)` (`:783-840`) reveals all Turn-1 mandates, calls `TurnEngine::finalize_season` (`crates/league-core/src/engine.rs:1577`) -> `compute_final_scoreboard` (`crates/league-core/src/scoring.rs:545-617`), and replays the log from Turn 1 twice (`replay_eval_season` and `replay_season_with_state_predicates` at `host.rs:811-812`). For each seat, `SeatScoreEntry::total_equity` (`scoring.rs:582-591`) equals `project_equity + net_asset_equity + mandate_bonus`, and `SeatScoreEntry::broken_promises` (`scoring.rs:595`) records audited promise breaches.
- **`ClubBot` surface and `bot_tie_seed`:**
  - `ClubBot` (`crates/league-core/src/bot.rs:40-114`) exposes only `plan_pass_a` (`:52`), `plan_pass_b_and_bundle` (`:69`), and `decide_full_turn(&self, obs: &SeatObservation, own_mandate: &PrivateMandate)` (`:102-114`). Its `bot_tie_seed` (`u64`) is used **only** for the Turn-`4n` council-vote tie-break `(seat + 1 + ((self.bot_tie_seed ^ u64::from(obs.turn)) as u8 % (n_u8 - 1))) % n_u8` (`:536`), so varying `bot_tie_seed` produces almost no strategic diversity.
- **Legality checking vs pessimistic tie-break preview:**
  - `preview_seat_bundle` (`crates/league-core/src/observation.rs:542-566`) takes a `PublicPreviewState`, obtained with `obs.to_public_preview_state()` (`crates/league-core/src/bot.rs:657`). It is used to legality-check and prune synthetic candidates.
  - Inside `preview_what_if` (`observation.rs:503-537`), `PreviewTieMode::PessimisticForSeat` (`:513-535`) evaluates one caller-supplied `HumanWhatIfRequest` across 8 tie-break beacons (`"pessimistic preview candidate set cannot be empty"` at `:532`) and is a tie-break stress test, **not** an action generator.
- **Recorded `decisions/*.json` (`D4`), `branch` outputs, and relay sidecars:**
  - `EvalDecisionRecord` (`crates/league-eval/src/records.rs:328-432`), written to `decisions/day-DD-talk-<Guild>.json` (`driver.rs:2038`), `decisions/day-DD-act-<Guild>.json` (`driver.rs:2184`), and per-arm `<out_dir>/<arm>/decision.json` + `outcome.json` (`BranchArmOutcome`, `records.rs:601-618`, `driver.rs:2936-2939`), stores `arm`, `raw_reply`, `tool_calls_used`, `intermediate_bundle_sha256s`, `refused_market_actions`, `refused_bundle_legs`, `parse_failure`, `refusal_code`, `refused_bundle`, `completion_tokens`, `reply_token_ids`, `club_bot_baseline`, etc.
  - When `--arm control` or `--arm on` ran (`docs/eval-harness.md:54`), `SavedToolRoundRecord` (`crates/league-relay/src/relay.rs:2685-2718`, consumed by `league-eval` at `driver.rs:714-767`) persists every `SavedToolRoundCall::arguments` (`relay.rs:2660`) and `reasoning` (`:2710`), while `SavedHttpReplyRecord::body` (`relay.rs:2801-2844`, `:2830`) keeps the raw HTTP JSON responses. `driver.rs:747-766` sorts tool records by `(round_index, attempt)`, skips `!tool_result_sent`, `CALL_LIMIT_REACHED`, and `MALFORMED_TOOL_CALL`, parses `call.arguments` via `parse_preview_tool_arguments`, and pushes `sha256_hex` of `parsed_req.bundle` into `intermediate_bundle_sha256s` for **every** parsed sent call, including calls whose preview/legality check returned an engine rule refusal.
- **CPU timing anchor (`0.10 s` single sample, not an upper bound):**
  - `eval_host_turn_0_fork_deals_identical_mandates_and_runs_independently_to_eval_replay` (`crates/league-core/tests/eval_host.rs:447-515`) measured **`0.10 s` (`100 ms`)** in `--release` at the test runner's resolution. That test stops at the season's natural close (`fork_b.phase() != TurnPhase::TalkAndMarket` at `:490-491`), runs **three** full-season replays in total (two inside `finalize_eval_season` at `host.rs:811-812` plus a third external `replay_eval_season` at `eval_host.rs:510`), and does **not** isolate or time per-rollout `fork_from_checkpoint` disk reads, mid-season state sizes, candidate building, or peer posture logic. Accordingly, `0.10 s` is only a preliminary timing anchor — **not** an upper bound or a guarantee. The pilot explicitly times a mid-season and a late-season fork-to-close rollout inside the actual scorer loop, re-derives the cost table (§6.2), and re-checks the ~1-day 8-core cost stop before any main-run decision is scored.

---

## 3. Frozen Candidate Set and Arm-Blind Pre-Screen

### 3.1 Candidate Set Construction `C(state)`

For each `(state_id, pre_act_checkpoint, focal_seat)`, the unblinded builder constructs a deduplicated candidate set `C(state)` from four sources before passing `BlindScoringRequest` to `crates/league-scorer`:

1. **Mandatory passive baseline (`in_span: true`, `is_arm_independent: true`):** `EvalActSubmission::default()` (empty `market_actions`, empty `pass_b_actions`, empty `SealedOrderBundle`), representing `NO_REPLY` / inaction.
2. **All evaluated arms' committed decisions (`in_span: true`, `is_arm_independent: false`):** For every `(arm, replicate)` at `state_id` (`off`, `control`, `on`), parse `decision.json`'s `raw_reply` (`parse_eval_act_reply`, or `EvalActSubmission::default()` when `no_reply == true` / whole-reply refusal) using exact submitted `market_actions` and `bundle` (`pass_b_actions` empty for model seats), and include it unconditionally. Passing it through `EvalHost::apply_act_submissions` (`eval/host.rs:631-734`) reproduces the engine's exact Phase-1 partial refusal and Phase-2 bundle reveal fallback handling.
3. **First draft per tool arm/replicate (`in_span: false` unless equal to a committed/baseline candidate, `is_arm_independent: false`):** Because `intermediate_bundle_sha256s` (`crates/league-eval/src/driver.rs:749-766`) hashes only the `bundle` field (`:762-765`) and records every parsed sent call that is neither `CALL_LIMIT_REACHED` nor `MALFORMED_TOOL_CALL` (including calls whose preview/legality check returned a rule refusal), the builder reconstructs tool records from `SavedToolRoundRecord` in `(round_index, attempt, call_index)` order (`SavedToolRoundCall::call_index` at `crates/league-relay/src/relay.rs:2651`, set from enumeration at `:7237`, matching position in `rec.calls` iterated at `driver.rs:747-749`), selects the **first** sent call that parsed via `parse_preview_tool_arguments` without `CALL_LIMIT_REACHED` or `MALFORMED_TOOL_CALL` (even if its wire tool result was an engine refusal), and constructs its full `EvalActSubmission` with both `market_actions` and `bundle` from `SavedToolRoundCall::arguments` (`crates/league-relay/src/relay.rs:2660`). First drafts are scored with identical CRN rollouts so draft-to-commit revision gain uses one estimator, **but stay outside `[v_min, v_max]`** (`in_span: false`) unless that identical submission also appears as a committed, passive, or synthetic candidate — ensuring a bad draft cannot shift any arm's normalized score.
4. **Deterministic arm-independent `ClubBot` and synthetic candidates (`in_span: true`, `is_arm_independent: true`; `1` unmodified `ClubBot` + up to `K_synth_max = 32` slots):** Generated strictly from `SeatObservation` and `own_mandate`. None of the proposed `ScorerConfig` parameters (`§5.2`), proposed `crates/league-scorer` crate, or proposed fixed constants below exists in the repository today; when built, all fixed constants and rules are pinned by `scorer_tree_sha256`:
   - `1` unmodified `ClubBot::new(bot_tie_seed).decide_full_turn(&obs, &own_mandate)` baseline outside `k_synth_max` (`bot.rs:536` affects council tie-breaks only; no `bot_tie_seed` bank is used). When the proposed `include_pass_b_cleared_club_bot == true` (`§5.2`), a `pass_b_actions`-cleared copy is included as a candidate inside `k_synth_max` matching the single-pass model interface.
   - Single-axis enumerations from `obs`: partial and full crossing of top-of-book resting `CredOffer`s (proposed `partial_crossing_fractions_bps`), parity-anchored quote postings (`quote_offsets_bps` filtered inside `parity_spread_threshold_bps`), top `SYNTH_MAX_CONVERTER_BATCHES` feasible `ConverterRunOrder` batches ranked by `parities` (proposed fixed code constant/rule, pinned by `scorer_tree_sha256`), top `SYNTH_MAX_PROJECT_BATCHES` coverable `ProjectDeliveryOrder` batches ranked by `SYNTH_PROJECT_RANK_BY_EQUITY` (`obs.spec`: descending equity, ties broken by ascending project ID — a proposed fixed code constant/rule pinned by `scorer_tree_sha256`, not a `ScorerConfig` parameter), `TownImport` deficit coverage for `upkeep_cover_turns` and stalled converter inputs (`SYNTH_CONVERTER_INPUT_DEFICIT_COVER`, proposed fixed rule), voluntary debt repayment fractions (`debt_repayment_grid_bps`), maturing `CheckCash` / `EscrowFinish` exact claims (`SYNTH_CLAIM_MATURING_EXACT`, proposed fixed rule), and election/charter self-bids on `turn % 4 == 0` / `turn % 6 == 0` (`SYNTH_COUNCIL_SELF_NOMINATION`, proposed fixed rule).
   - Composite greedy combinations and local perturbations governed by proposed `ScorerConfig` fields `composite_greedy_rules` and `quantity_scales_bps` (with leg-drop tie-breaking governed by proposed fixed constant `DROP_TIE_BREAK_BY_CANDIDATE_ID`), each checked with `preview_seat_bundle(&obs.to_public_preview_state(), &req)` (`crates/league-core/src/bot.rs:657`, `observation.rs:542-566`) to discard illegal or immediately upkeep-bankrupting drafts. When more than `k_synth_max` unique candidates in this group (non-`ClubBot` synthetics plus the optional cleared-pass-B `ClubBot` copy) pass, slots are filled strictly in proposed `synth_slot_fill_order` (`§5.2`), breaking intra-bucket ties by ascending canonical `candidate_id` up to `K_synth_max = 32`.
5. **Canonical deduplication and freezing:** Each `EvalActSubmission` is hashed by canonical compact JSON bytes (`candidate_id = sha256_hex(serde_json::to_vec(&sub))`). Identical submissions merge with logical-OR flags (`in_span = a.in_span || b.in_span`, `is_arm_independent = a.is_arm_independent || b.is_arm_independent`). `C(state)` is sorted ascending by `candidate_id` with all arm/origin metadata stripped into `BlindScoringRequest` (`§8.1`).

### 3.2 Arm-Blind Pre-Screen

Before any evaluated arm's committed decision or first draft is joined into `C(state)`, an **arm-blind pre-screen** evaluates each state strictly over its arm-independent candidate set `C_indep(state) = { c in C(state) : c.is_arm_independent == true }` (passive baseline + `ClubBot` + `<= K_synth_max` synthetics) across the `M` scenarios:

- Computes `v_min_indep`, `v_max_indep`, `span_indep = v_max_indep - v_min_indep`, `is_flat_indep = (v_max_indep == v_min_indep)`, and distinct feasible candidate count `n_feasible_indep`.
- Any state failing pre-screen thresholds (`is_flat_indep == true`, `span_indep < min_arm_indep_span`, or `n_feasible_indep < min_arm_indep_candidates`) is excluded from primary `[0, 1]` D5 hypothesis evaluation (`excluded_by_prescreen: true`) and flagged **arm-blindly** before any arm identity touches the data.
- Pre-screen thresholds live in `ScorerConfig` (`§5.2`), are calibrated on the pilot strictly from arm-blind diagnostics, and are frozen in `scorer_freeze.json` prior to main-run scoring. Any post-join state where `v_max == v_min` after adding committed decisions still receives `score_norm = NA` (`null`, §7.1).

---

## 4. Other Seats: Diversity Sources, Postures, and Talk Commitments

Instead of conditioning on the single realized peer vector in `day-DD-act-recorded.seal`, each state evaluates `M` counterfactual scenarios `m in 0..M` (pilot initial `M = 16`, shared as Common Random Numbers across every `c in C(state)`).

1. **Sources of counterfactual scenario diversity (and honest limits):**
   - `bot_tie_seed` controls **only** the council-vote tie-break (`crates/league-core/src/bot.rs:536`) and creates almost no variation across rollouts; the scorer **never** uses `bot_tie_seed` variation as a diversity source and drops any tie-seed bank.
   - Scenario spread comes from three explicit mechanisms: **(a) peer policy postures**, **(b) the counterfactual mandate draw (§5)**, and **(c) the probabilistic talk-phase deal-keeping draw**.
   - Any individual source can yield small spread on its own on particular states — for example, when no bilateral deal was proposed in `talk`, when late-turn horizons leave `ClubBot` play nearly identical across mandates, or when tight `CRED`/inventory budgets constrain posture adjustments. For this reason `M` is not permanently fixed a priori: the pilot measures observed across-scenario CRN variance and sets final `M` in `ScorerConfig` before freezing.
2. **Postures implemented as new scorer code in `crates/league-scorer`:**
   - Because `league-core`'s `ClubBot` exposes only `plan_pass_a`, `plan_pass_b_and_bundle`, and `decide_full_turn` (`crates/league-core/src/bot.rs:52-114`), posture variants are implemented inside `crates/league-scorer` (leaving `league-core` untouched).
   - For each non-focal seat `s != focal_seat`, scenario `m` deterministically selects a posture from `ScorerConfig::posture_weights` (`m % 4` mapping in `§5.2`):
     - *Standard `ClubBot`* (`m % 4 in {0, 1}`): unmodified `decide_full_turn(&obs_s, &mandate_{m,s})`.
     - *Liquidity/converter active* (`m % 4 == 2`): retains `decide_full_turn` bundle and adds deterministic crossing `PostOffer` actions for idle excess inventory above `upkeep_cover_turns` when resting book spreads exceed `parity_spread_threshold_bps`.
     - *Conservative/upkeep-first* (`m % 4 == 3`): retains `decide_full_turn` while suppressing speculative resting offer posts that lock `CRED` or goods below `upkeep_cover_turns`.
   - All postures respect the `ClubBot` `pass_a_actions`, `pass_b_actions`, `bundle` interface cleanly and are parameterized strictly in `ScorerConfig` (`§5.2`).
3. **Talk-phase trade commitments and conditional compliance:**
   - **Offline extraction (before blinding):** A frozen deterministic extractor scans `to-seat/day-DD-act-<Guild>.negotiation.json` (`NegotiationInputs::whispers`) and outgoing `decisions/day-DD-talk-<Guild>.json` (`parse_eval_talk_reply`, `crates/league-eval/src/reply.rs:577`) using versioned rules (`talk_extractor_version`, `§5.2`), extracting unambiguous `(proposer_seat, counterparty_seat, offered_action_template)` commitments without later conflicting retractions.
   - **Probabilistic compliance in rollouts:** For each `(m, s)` with an extracted commitment, the deterministic scenario PRNG draws `honour_{m,s} ~ Bernoulli(p_honour)` (`p_honour = 0.75` pilot initial in `§5.2`) using wrapping SHA-256 keying. When `true`, the corresponding legal `MarketAction` is prepended to `pass_a_actions` (pruned via `preview_seat_bundle(&obs_s.to_public_preview_state(), ...)` if unaffordable).

---

## 5. Hidden-Objective Sampling and Free Parameters

### 5.1 Mandate Prior Read From Checkpoint `spec.mandate_deck`

Rather than calling `default_graded_mandate_deck`, the scorer reads the table's actual dealt mandate deck directly from the checkpoint's specification: **`obs.spec.mandate_deck`** (`TableSpec::mandate_deck` at `crates/league-core/src/types.rs:381`, exposed on every `SeatObservation` via `PublicTableSpec::mandate_deck` at `crates/league-core/src/observation.rs:86`).

At `day-DD-pre-act`:
- The focal seat's own dealt mandate is known (`focal_mandate = *host.mandate_for_seat(Seat(focal_seat))?`, `crates/league-core/src/eval/host.rs:521-527`).
- In each scenario `m in 0..M`, the scorer forms the remaining undealt pool by taking `spec.mandate_deck` minus one instance matching `focal_mandate` (`7` mandates when `mandate_deck.len() == 8`), and draws an ordered `(seat_count - 1)`-tuple (`3` distinct mandates without replacement) uniformly via SHA-256 reverse Fisher-Yates seeded from `(scorer_seed, pre_act_pair_sha256, day, m)` under `mandate_stratification_mode`.
- **Uniform prior, not behavioural posterior:** The draw is an explicit **uniform prior over the mandates the focal seat has not been dealt** in `spec.mandate_deck`, **not** a posterior inferred from earlier-turn peer actions (`§10`).
- **Exact focal scoring:** Peer counterfactual mandates drive peer `ClubBot` play across `day..=close`, while `EvalHost::finalize_eval_season()` unseals `focal_seat`'s authentic `mandate_commits[focal_seat]` and computes `pre_mandate_nws` median (`crates/league-core/src/scoring.rs:545-617`).

### 5.2 Complete Pre-Registered Free Parameter Table (Proposed `ScorerConfig`)

None of `ScorerConfig`, `crates/league-scorer`, or the fixed code constants below exists in the repository today; all names in this subsection are proposed names for the future scorer crate, to be hashed into `scorer_config_sha256` / `scorer_tree_sha256` when built, calibrated from arm-blind pilot diagnostics only, and frozen in `scorer_freeze.json` (`§8.2`):

| Proposed Parameter (`ScorerConfig` field) | Meaning | Pilot Initial Value |
|---|---|---|
| `posture_weights` & `posture_schedule` | Peer policy posture mix and deterministic `m % 4` scenario mapping | `[2 Standard, 1 Active, 1 Conservative]` (`m % 4 in {0,1} -> Std, 2 -> Act, 3 -> Cons`) |
| `p_honour` | Probability an extracted talk commitment is prepended to peer Turn-`day` actions | `0.75` (`Bernoulli(0.75)`) |
| `scenarios_m` (`M`) | Number of CRN scenarios per state (sized from pilot measured SE/diversity) | `16` |
| `k_synth_max` (`K_synth_max`) | Cap on non-`ClubBot` synthetic candidates plus the optional cleared-pass-B `ClubBot` copy (`include_pass_b_cleared_club_bot`, outside the unmodified `ClubBot` baseline) | `32` |
| `include_pass_b_cleared_club_bot` | Include `ClubBot` decision with `pass_b_actions` cleared (counted inside `k_synth_max`) | `true` |
| `partial_crossing_fractions_bps` | Resting top-of-book `CredOffer` crossing fractions (`PostOffer` legs) | `[5000, 10000]` bps (`50%`, `100%`) |
| `debt_repayment_grid_bps` | Fractions of spare `CRED` above upkeep reserve offered as `debt_repayment_cred` | `[5000, 10000]` bps (`50%`, `100%`) |
| `quote_offsets_bps` | Parity-relative limit-order price offsets for synthetic `PostOffer` generation | `[-1000, -500, 500, 1000]` bps (`±5%`, `±10%`) |
| `quantity_scales_bps` | Perturbation scalings applied to feasible composite drafts | `[5000, 10000]` bps (`50%`, `100%`) |
| `composite_greedy_rules` | Sole owner of composite greedy combination & leg-drop mode (`DROP_TIE_BREAK_BY_CANDIDATE_ID` breaks risk ties only) | `"top_pass_per_axis_plus_drop_max_upkeep"` |
| `synth_slot_fill_order` | Deterministic priority queue truncating excess feasible synthetics (incl. cleared-pass-B `ClubBot` copy) at `k_synth_max` | `["cleared_pass_b", "claims_council", "upkeep_inputs", "cross_quotes", "batches", "debt", "composite_drops", "scaled"]` |
| `upkeep_cover_turns` | Minimum upkeep turns reserved by synthetic upkeep rule and conservative posture | `2` turns |
| `parity_spread_threshold_bps` | Minimum spread vs shadow parity triggering liquidity posture `PostOffer` posts | `500` bps (`5%`) |
| `talk_extractor_version` & `talk_extractor_rules_digest` | Frozen deterministic regex/keyword/retraction table version and SHA-256 | `"talk_rules_v1"` + frozen rules digest |
| `mandate_stratification_mode` | Permutation stratification scheme over undealt `spec.mandate_deck` entries | `"uniform_undealt_fy_stratified"` |
| `prescreen_min_arm_indep_span` | Minimum arm-independent `v_max_indep - v_min_indep` span to pass `§3.2` | Calibrated on pilot (`arm-blind`) |
| `prescreen_exclude_flat_sets` | Exclude states with `v_max_indep == v_min_indep` in `§3.2` | `true` |
| `prescreen_min_arm_indep_candidates` | Minimum feasible candidates in `C_indep(state)` | Calibrated on pilot (`arm-blind`) |
| `strata_schedule` | Checkpoint stratification across early/mid/late season thirds (`league-eval select`) | `"2_early_2_mid_2_late_20_seeds"` (`docs/eval-harness.md` `D5`) |

*Proposed fixed code constants (to be pinned by `scorer_tree_sha256` when `crates/league-scorer` is built, `§3.1`):* `SYNTH_MAX_CONVERTER_BATCHES = 4`, `SYNTH_MAX_PROJECT_BATCHES = 4`, `SYNTH_PROJECT_RANK_BY_EQUITY` (fixed rule: rank coverable project batches by descending equity, then ascending project ID), `SYNTH_CONVERTER_INPUT_DEFICIT_COVER = true`, `SYNTH_CLAIM_MATURING_EXACT = true`, `SYNTH_COUNCIL_SELF_NOMINATION = true`, and `DROP_TIE_BREAK_BY_CANDIDATE_ID` (tie-break only when legs under `composite_greedy_rules` share equal upkeep risk). **Build requirement:** when `crates/league-scorer` is built, it must contain no unlisted free value outside the proposed `ScorerConfig` fields and these proposed fixed constants.

---

## 6. Horizon Valuation, Determinism, and CPU Cost Estimates

### 6.1 Season-Close Rollout Procedure and Determinism Rules

For each scenario `m in 0..M` and each candidate `c in C(state)`:

1. **Reconstruct an independent host from checkpoint files (no `Clone`):** Because `EvalHost` does not implement `Clone` (`crates/league-core/src/eval/host.rs:197`, manual `Debug` only at `:217`) and `from_save_and_seal_bytes` is `pub(crate)` (`:955`), each `(c, m)` rollout calls `EvalHost::fork_from_checkpoint(save_path, seal_path)` (`host.rs:1195-1213`) directly on the immutable checkpoint pair whose contents were verified against `BlindScoringRequest::{pre_act_save_sha256, pre_act_seal_sha256, pre_act_pair_sha256}`. The fork I/O and state construction cost per rollout is therefore part of §6.2's runtime budget.
2. **Apply turn-`day` submissions:** Insert `c` at `focal_seat` and scenario `m`'s counterfactual peer submissions (`s != focal_seat`, computed deterministically once per `(state, m)`) into `BTreeMap<u8, EvalActSubmission>`. Call `host.apply_act_submissions(&subs)` (`:631-734`) followed by `host.sweep_turn()` (`:739-778`).
3. **Play turns `(day + 1)..` to natural close (`HARD_TURN_CAP = 32`):** While `host.phase() == TurnPhase::TalkAndMarket`:
   - Observe every seat (`host.observe_seat(Seat(s))`, `:512-515`).
   - Run peer posture builders (`§4`) for peers (`mandate_{m,s}`) and unmodified `ClubBot` for `focal_seat` (`focal_mandate`, `§10`), with seeds computed via domain-separated SHA-256 and explicit `wrapping_add` (`bot_seed.wrapping_add(u64::from(offset))`).
   - Call `apply_act_submissions` and `sweep_turn()`.
4. **Finalize and collect integer outcomes:** Call `host.finalize_eval_season()` (`:783-840`), read `scoreboard.standings` entry for `focal_seat`, and store exact integer outputs `(total_equity, project_equity, net_asset_equity, mandate_bonus)` in an indexed slot `(state_idx, c_idx, m)` without any intermediate floating-point arithmetic.

**Strict determinism constraints:**
- **No parallel floating-point reduction:** Any parallel worker pool executes independent `(state, c, m)` rollouts into pre-allocated index slots (`results[c_idx][m]`). Reductions iterate strictly in ascending index order (`m = 0..M`, `c = 0..|C|`).
- **Integer sums, single final division:** Mean equity sums `total_equity` as `i128` across `m = 0..M` in fixed `0..M` order and performs **one** final division `sum as f64 / (M as f64)`.
- **No wall-clock fields or filesystem paths:** Score output records omit timestamps, execution durations, hostnames, and paths. `BlindScoringRequest` hashes carry content SHA-256 digests (`pre_act_save_sha256`, `pre_act_seal_sha256`, `pre_act_pair_sha256`), never filesystem paths.
- **Parallelism & `rayon` dependency note:** `rayon` is **not** in `Cargo.lock` today. Keeping parallel rollout execution in `crates/league-scorer` requires adding `rayon` as one new workspace dependency in `Cargo.toml`/`Cargo.lock` (only inside `crates/league-scorer`), and automated tests must verify that outputs are bit-identical across thread counts (`1` thread vs `8` threads).

### 6.2 Candidate Counts, Recomputed Cost Table, and Pilot Re-Derivation

For `K_synth_max` synthetic candidates, `arms = 3` (`off`, `control`, `on`), and `tool_arms = 2` (`control`, `on`), including the mandatory passive candidate (`1`) and the unmodified `ClubBot` baseline (`1`), the maximum unique candidate count `K` per state is:

$$K \le 1\text{ (passive)} + 1\text{ (ClubBot)} + K_{\text{synth\_max}} + (\text{arms} \times \text{replicates})\text{ committed} + (\text{tool\_arms} \times \text{replicates})\text{ first drafts}$$

With `K_synth_max = 32`, `arms = 3`, `tool_arms = 2`, and `M = 16`:
- `replicates = 2`: `K = 1 + 1 + 32 + (3 × 2) + (2 × 2) = 1 + 1 + 32 + 6 + 4 = 44` candidates/state; `K × M = 44 × 16 = 704` rollouts/state.
- `replicates = 3`: `K = 1 + 1 + 32 + (3 × 3) + (2 × 3) = 1 + 1 + 32 + 9 + 6 = 49` candidates/state; `K × M = 49 × 16 = 784` rollouts/state.
- The D5 pilot (`4` seeds × `6` states = `24` states, `3` arms × `3` replicates) produces `24 × 3 × 3 = 216` arm decisions (plus up to `24 × 2 × 3 = 144` first drafts).

Anchored at the preliminary **`0.10 s` (`100 ms`)** single-core sample from `eval_host_turn_0_...` (`crates/league-core/tests/eval_host.rs:447-515`), exact arithmetic gives:

| Scope | States | Formula (`states × K × M`) | Max Rollouts | 1-Core Time (`rollouts × 0.10 s`) | 8-Core Wall Time (`1-core / 8`) |
|---|---|---|---|---|---|
| **Single State (`rep = 2`)** | `1` | `1 × 44 × 16` (`44 = 1+1+32+6+4`) | `704` | `70.4 s` (`1.17 CPU-min`) | `8.8 s` |
| **Single State (`rep = 3`)** | `1` | `1 × 49 × 16` (`49 = 1+1+32+9+6`) | `784` | `78.4 s` (`1.31 CPU-min`) | `9.8 s` |
| **D5 Pilot (`rep = 3`, `216` arm decisions)** | `24` (`4×6`) | `24 × 49 × 16` | `18,816` | `1,881.6 s` (`31.36 CPU-min`) | `235.2 s` (`3.92 min`) |
| **D5 Main Run (`rep = 2`, `720` arm decisions)** | `120` (`20×6`) | `120 × 44 × 16` | `84,480` | `8,448.0 s` (`140.80 CPU-min` / `2.35 CPU-h`) | `1,056.0 s` (`17.60 min`) |
| **D5 Main Run (`rep = 3`, `1,080` arm decisions)** | `120` (`20×6`) | `120 × 49 × 16` | `94,080` | `9,408.0 s` (`156.80 CPU-min` / `2.61 CPU-h`) | `1,176.0 s` (`19.60 min`) |

*Anchor caveats, mandatory pilot re-derivation, and post-pilot cost stop:* As documented in `§2`, the `0.10 s` figure is **not** an upper bound: the timed test stops at natural close (`crates/league-core/tests/eval_host.rs:490-491`), runs three full-season replays in total (two inside `finalize_eval_season` at `host.rs:811-812` plus a third external `replay_eval_season` at `eval_host.rs:510`), omits per-rollout `EvalHost::fork_from_checkpoint` (`host.rs:1195`) disk/JSON cost (`18,816–94,080` forks, one per rollout), mid-season checkpoint state size, candidate building, and peer posture computation, and is one sample at the test runner's resolution. At `0.10 s`, main-run 8-core wall time (`17.60 min` for `rep = 2`; `19.60 min` for `rep = 3`) is under one day. **Before freezing `scorer_freeze.json`, the pilot times a mid-season checkpoint and a late-season checkpoint fork-to-close rollout in `--release` with candidate building and postures active, re-derives this cost table from those measured timings, and re-applies the one-day wall-time limit on 8 cores: if the re-derived main run crosses ~1 day on 8 cores, scoring stops for a decision before freezing or scoring any main-run decision.**

---

## 7. Outputs, Scores, Sensitivity Score, and Mechanism Diagnostics

### 7.1 Blind Scorer Outputs (Per `state_id` and Per `candidate_id`)

For every candidate `c in C(state)` across scenarios `m in 0..M` (stored without wall-clock fields or paths):

- `ev_total_equity`: integer-summed mean `total_equity(focal_seat)` over `m = 0..M` (computed as exact `i128` sum divided once by `M as f64`, plus mean `project_equity`, `net_asset_equity`, `mandate_bonus`).
- **Committed-plus-baseline span (`v_min`, `v_max`):** minimum and maximum `ev_total_equity` computed strictly across **span-eligible candidates** (`in_span == true`: passive + synthetic + every committed arm decision). First drafts (`in_span == false`) are evaluated with identical CRN rollouts so revision gain uses the same estimator, **without** ever entering `[v_min, v_max]`. Because every committed arm decision satisfies `in_span == true`, committed `regret_equity = v_max - ev_total_equity(c) >= 0.0` always holds.
- **Primary normalized phase-invariant score (`score_norm`):**
  - If `v_max > v_min`: `(ev_total_equity(c) - v_min) / (v_max - v_min)` (inside `[0.0, 1.0]` for `in_span == true` candidates).
  - If `v_max == v_min`: **`NA` (`null` in JSON, never `1.0`)**, with `degenerate_flat_set: true`. States whose arm-independent subset is already flat are excluded upfront by `§3.2`; any post-join flat state still emits `null` rather than `1.0`.
- **Upward bias disclosure & split-half measurement (Winner's curse):** Because `v_max` takes the maximum over up to `K` noisy `M`-scenario estimates, `v_max` is biased upward (`E[max_c EV_hat(c)] >= max_c E[EV_hat(c)]`), pulling `regret_equity` upward and compressing mid-pack `score_norm`. Always measure and report this upward bias via **split-half (held-out scenario) re-estimation** across disjoint scenario halves (`H_A = 0..(M/2)` and `H_B = (M/2)..M`): selection uses one half (`c*_A = argmax_{c in span} EV_{H_A}(c)`) and evaluation uses the other half (`EV_{H_B}(c*_A)`), defining the bias estimate as the mean over both directions of `EV_{H_A}(c*_A) - EV_{H_B}(c*_A)` and `EV_{H_B}(c*_B) - EV_{H_A}(c*_B)` (`c*_B = argmax_{c in span} EV_{H_B}(c)`). State that this estimates selection bias at `M/2` scenarios, which is an upper estimate for `M` scenarios, and report it alongside raw `ev_total_equity` and `sensitivity_indep_percentile`.
- **Pre-registered second sensitivity score (`sensitivity_indep_percentile`, `sensitivity_indep_rank`):** Rank and percentile of `ev_total_equity(c)` among the **arm-independent candidates only** (`is_arm_independent == true`: passive + `ClubBot` + synthetics), insensitive to whether another arm emitted an extreme candidate.
- **CRN paired Monte Carlo standard error (`paired_se_equity`):** sample standard error across `m in 0..M` of paired per-scenario differences against `c_best` (`sum_m` integer differences ordered `0..M`).
- **Descriptive-only pairwise contrasts:** Post-unblinding pairwise arm contrasts (`EV(on) - EV(off)`, `EV(on) - EV(control)`, `EV(control) - EV(off)`) and paired per-scenario SEs reported from unblinded tables are **descriptive only**. The pre-registered inferential analysis is the **mixed model** and the **cluster sign-flip test over seed means**, which are not defined in this document.

### 7.2 Unblinded D5 Pre-Registration Mechanism Measures

Joined post-scoring by `(state_id, arm, replicate)` **only after** `scorer_freeze.json` is written and recorded (`§8.2`), using the reporting step's `arm_map/`, `decisions/`, `outcome.json`, and `relay/seats/` sidecars (no scorer re-run required):

1. **Tool-call rate:** Mean `tool_calls_used` (`EvalDecisionRecord::tool_calls_used`, `records.rs:375`) and fraction of rounds with `tool_calls_used >= 1`.
2. **Draft-to-commit revision gain:** For each decision with `!intermediate_bundle_sha256s.is_empty()` (`driver.rs:749-766`), locate the **first** parsed sent tool call (non-`CALL_LIMIT_REACHED`, non-`MALFORMED_TOOL_CALL`, including rule-refused drafts) in `(round_index, attempt, call_index)` order (`SavedToolRoundCall::call_index` at `crates/league-relay/src/relay.rs:2651`, set from the enumeration at `:7237`, matching the position in `rec.calls` iterated at `driver.rs:747-749`), take `market_actions` and `bundle` from `SavedToolRoundCall::arguments` (`relay.rs:2660`) to resolve `first_draft_candidate_id` (already evaluated inside `C(state)` outside `[v_min, v_max]`), and report `score_norm(committed) - score_norm(first_draft)` (when `v_max > v_min`) and raw `ev_total_equity(committed) - ev_total_equity(first_draft)`, with `n_with_draft` reported separately.
3. **Reasoning length:** Total character length of `SavedToolRoundRecord::reasoning` (`crates/league-relay/src/relay.rs:2710`) and raw reasoning text stored inside `SavedHttpReplyRecord::body` (`relay.rs:2830`), alongside token counts (`completion_tokens` / `reply_token_ids` lengths).
4. **Illegal-order rate:** Fraction with reply-level refusal (`parse_failure` or `refusal_code.is_some()`), mean `refused_market_actions`, and fraction with whole-bundle reveal rejection (`refused_bundle.is_some()` / `refused_bundle_legs > 0`).
5. **Deal-keeping (two explicit lines):**
   - *(a) Engine-recorded pledges:* count structured Track-2 `pledges` / `audited_pledges` (`crates/league-core/src/eval/host.rs:85-88`) and terminal `broken_promises` (`crates/league-core/src/scoring.rs:595`) directly on their own reporting line wherever engine pledges exist.
   - *(b) Text-parsed talk commitments:* extracted deterministically from `decisions/day-DD-talk-<Guild>.json` (`driver.rs:2038`, `reply.rs:577`) and `to-seat/day-DD-act-<Guild>.negotiation.json` (`view.rs:48`) using `talk_extractor_version`, reported as `honoured_commitments / applicable_commitments` with `applicable_commitments` stated separately (never imputing `0/0`).

---

## 8. Blinding, `arm_map/` Governance, and Freezing Protocol

### 8.1 Exact Blind Input Schema (`BlindScoringRequest`) and Disjoint `arm_map/`

An unblinded builder prepares staging inputs into two disjoint locations:
- `blind_inputs/<state_id>.json` — the **only** tree readable by `crates/league-scorer`; contains strictly content hashes and anonymous candidates, with zero filesystem paths, arm labels, or tool-trace metadata. (Archive paths for `.save.json`/`.seal` blobs are resolved outside the hashed `BlindScoringRequest` via content-addressed CLI inputs and verified against `pre_act_save_sha256` / `pre_act_seal_sha256`.)
- `arm_map/<state_id>.json` — held strictly by the **run operator's reporting step**, separate from `crates/league-scorer` and from anyone tuning parameters.

```text
BlindScoringRequest {
  schema_version: u32,                            // fixed = 1
  state_id: String,                               // canonical "<source_game_id>:day-<DD>:seat-<idx>" (no arm)
  pre_act_save_sha256: String,                    // 64-hex SHA-256 of checkpoints/day-DD-pre-act.save.json bytes
  pre_act_seal_sha256: String,                    // 64-hex SHA-256 of checkpoints/day-DD-pre-act.seal bytes
  pre_act_pair_sha256: String,                    // 64-hex digest matching compute_pre_act_pair_sha256_hex
  focal_seat: u8,                                 // 0..4
  talk_commitments: Vec<ExtractedTalkCommitment>, // neutral counterfactual commitment specs (no raw text)
  candidates: Vec<BlindCandidate>,                // sorted ascending by candidate_id; duplicates merged
  scorer_seed: u64,                               // explicit recorded seed
  scorer_config_sha256: String,                   // SHA-256 of canonical ScorerConfig JSON (§5.2)
}

BlindCandidate {
  candidate_id: String,                           // sha256_hex(serde_json::to_vec(&submission))
  submission: EvalActSubmission,                  // market_actions, pass_b_actions, bundle only
  in_span: bool,                                  // true iff passive, synthetic, or any committed decision
  is_arm_independent: bool,                       // true iff passive, ClubBot, or synthetic (§3.2 / §7.1)
}
```

Removed/excluded from `BlindScoringRequest` and `blind_inputs/`: filesystem paths, wall-clock timestamps, `arm` (`off` / `control` / `on`), replicate index, `wording_arm`, branch output paths (`<out>/<arm>/...`), `raw_reply`, `discarded_reply_path`, `tool_calls_used`, `intermediate_bundle_sha256s` labels, `<Guild>.jsonl`, `<Guild>.scratchpad.md`, `http-reply-round-*.json`, and `tool-round-*.json`.

Separately, `arm_map/<state_id>.json` (held exclusively by the run operator's reporting step) stores `Vec<{ arm, replicate, committed_candidate_id, first_draft_candidate_id, decision_path, outcome_path }>` for post-freeze reporting.

### 8.2 Arm-Blind Tuning Rules and Freeze Procedure Before Main Run

1. **Arm-blind pilot tuning (`§5.2` parameters) & post-pilot cost check:** Run the D5 pilot (`4` seeds × `6` states × `3` arms × `3` replicates = `216` arm decisions) using the `r2` legality-check `control` wording arm being built separately (any pilot collected on the superseded inert control arm MUST be rerun on the `r2` legality-check control arm before freezing). Build `blind_inputs/` and `arm_map/` (retained inside the reporting step's separated store), run `league-scorer` twice to verify bit-identical `scores/` JSON across runs and thread counts, time mid-season and late-season fork-to-close rollouts to re-derive `§6.2` and re-apply the ~1-day 8-core cost stop (stopping for a decision if the re-derived main run crosses one day on 8 cores), and tune only parameters in `§5.2`'s Free Parameter Table strictly from **arm-blind diagnostics** (`paired_se_equity`, measured runtime, flat-set / pre-screen rate, arm-independent candidate coverage, and repeat-pass bit identity).
2. **Pre-freeze isolation rule:** **No person or tool joins `arm_map/` to any score before `scorer_freeze.json` exists.** Later implementation MUST include an automated test proving that attempting to join `arm_map/` to scores is refused while no recorded `scorer_freeze.json` exists.
3. **Commit clean tree and write `scorer_freeze.json`:** Refuse any dirty git worktree (`DIRTY_GIT_WORKTREE`), write `scorer_freeze.json`, and record **`scorer_freeze.json`'s SHA-256 in the first run's pre-registration before any main-run decision is scored**:
   - `scorer_git_sha` (40-hex workspace commit SHA)
   - `scorer_tree_sha256` (hash of `crates/league-scorer/**`)
   - `scorer_config_sha256` (canonical SHA-256 of frozen `ScorerConfig` covering every entry in `§5.2`)
   - `control_wording_arm` (exact `r2` legality-check control arm label used identically by pilot and main run, enforced strictly outside `league-scorer`)
   - `talk_extractor_version` & `talk_extractor_rules_digest`
   - `blind_inputs_manifest_sha256` (sorted SHA-256 manifest of main-run `blind_inputs/*.json` committed before any `scores/*.json` write)
4. **Main-run enforcement (with `league-scorer` isolated from arm/wording labels):**
   - `crates/league-scorer` reads only `blind_inputs/` and `scorer_freeze.json::{scorer_config_sha256, blind_inputs_manifest_sha256, scorer_tree_sha256}`, verifying `ScorerConfig` SHA-256 and never reading `setup.json` (`relay_wording_arm`) or any wording-arm or arm label.
   - Outside `league-scorer`, the unblinded input builder and the reporting step verify `setup.json`'s `relay_wording_arm` (`crates/league-eval/src/driver.rs:1715,1921`) matches `scorer_freeze.json::control_wording_arm` on `control` runs; the reporting step additionally refuses any `arm_map/` join lacking a valid pre-registered `scorer_freeze.json`.

---

## 9. Crate Placement (`crates/league-scorer`)

**Decision:** Place the blind scoring engine in a new dedicated workspace crate **`crates/league-scorer`** (`library` + `league-scorer` binary), accompanied by unblinded reporting hooks in `crates/league-eval` (held by the reporting step).

**Rationale over alternatives:**
- *Compile-time blinding vs placing inside `league-eval`:* `league-eval` links `EvalArm`, `EvalDecisionRecord`, `BranchArmOutcome`, `league-relay`, HTTP transcripts, and arm directories. Separating `crates/league-scorer` guarantees compile-time isolation: `crates/league-scorer/Cargo.toml` depends strictly on `league-core`, `league-seat-types`, `serde`, `serde_json`, `sha2` (and optional `rayon` — noting `rayon` is **not** in `Cargo.lock` today and would add one new dependency whose usage must collect indexed results so outputs never depend on thread count), with **zero** dependency on `league-eval` or `league-relay`.
- *In-process Rust vs external FFI/Python:* Across `84,480–94,080` full-horizon rollouts (one `EvalHost::fork_from_checkpoint` call per rollout, as in `§6.2`), external IPC or Python bindings would add overhead and risk divergence from `league-core` invariants. A Rust workspace crate calls `EvalHost::fork_from_checkpoint`, `apply_act_submissions`, `sweep_turn`, `finalize_eval_season`, `ClubBot::decide_full_turn`, and `preview_seat_bundle` directly in `--release`, inherits `[workspace.lints]` (`unsafe_code = "forbid"`, `missing_docs = "warn"`), and builds offline under `--locked`.

---

## 10. Known Limits

1. **Surrogate rollout policy (`ClubBot`) for peers and focal post-branch continuation:** After the branch day `day`, **the focal seat is played by `ClubBot` like every other seat in the rollout**, while D5's base-game peers (`play`) are interactive model seats (`docs/eval-harness.md:13`). If an action's payoff hinges on multi-turn LLM persuasion or adaptive play across subsequent turns rather than `ClubBot` continuation, single-branch rollouts will miss that strategy difference; CRN keeps within-state comparison internally consistent across `C(state)`.
2. **Fixed base-game VRF future beacons across counterfactuals:** `EvalHost::fork_from_checkpoint` restores `vrf_proofs` from the base game's `.seal`, so future close-hazard rolls (`close_roll_bps_from_beacon`) and shocks on `t > day` remain fixed to the base season trajectory across all candidates (`CRN`).
3. **Talk-deal extractor recall and separate engine pledges:** Deterministic rule matching misses vague hints or indirect conditional messages; under-capture is symmetric across arms and supplemented by the separate engine-recorded `pledges`/`audited_pledges`/`broken_promises` mechanism line (`§7.2`).
4. **Span sensitivity, `NA` flat sets, and Winner's curse:** Normalization `(EV(c) - v_min) / (v_max - v_min)` amplifies noise on narrow-span states and compresses scores near extreme failures. Flat sets (`v_max == v_min`) emit `NA` (`null`, excluded upfront by `§3.2`), Winner's-curse upward bias on `v_max` over `K` noisy estimates is measured via split-half re-estimation (an `M/2` selection-bias upper estimate for `M` scenarios) and explicitly reported, and the arm-independent percentile sensitivity score (`§7.1`) provides an outlier-resistant complement.
5. **Uniform prior over `spec.mandate_deck` vs behavioural posterior:** Peer mandates are drawn uniformly without replacement from the undealt entries of `obs.spec.mandate_deck` (`types.rs:381`, `observation.rs:86`), **not** inferred as a posterior from peer behaviour on turns `1..=day`.
6. **Untimed fork/posture costs in the `0.10 s` anchor and three-replay test overhead:** Calling `EvalHost::fork_from_checkpoint` per `(c, m)` rollout (`EvalHost` lacks `Clone`, `host.rs:197`) and `EvalHost::finalize_eval_season` (`host.rs:783-840`) adds per-rollout cost not captured in the `0.10 s` `turn-0` runner-resolution sample (`crates/league-core/tests/eval_host.rs:447-515`, which runs three replays in total: two at `host.rs:811-812` plus one external `replay_eval_season` at `eval_host.rs:510`). Pilot mid-season and late-season timed runs re-derive §6.2's budget and re-check the ~1-day 8-core cost stop before main-run freezing.
7. **Tool-draft candidate role visibility (`in_span: false`, `is_arm_independent: false`):** Inside `BlindScoringRequest`, any candidate with `in_span: false` and `is_arm_independent: false` can only be a tool-arm first draft (since `off` has no drafts, and matching passive/synthetic/committed candidates merge into `in_span: true`). Neither `control` nor `on` nor any arm label is identified, and `league-scorer` must use `in_span` strictly to exclude the candidate from `[v_min, v_max]` normalization and never to tell arms apart.
